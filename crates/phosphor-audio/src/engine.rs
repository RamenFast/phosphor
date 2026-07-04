// SPDX-License-Identifier: GPL-3.0-or-later
//! The PipeWire engine thread. One dedicated thread owns the main
//! loop, the registry mirror, and every stream; everyone else talks to
//! it through a `pipewire::channel` command pipe and reads samples out
//! of a shared [`SampleRing`]. This is v3's parec-subprocess replaced
//! by a native client — same observable contract:
//!
//! - `take_stereo_samples` drains the scope feed (f32 interleaved).
//! - the stream-ended callback fires when a stream dies on its own
//!   (app stopped, device vanished) — never on an explicit stop.
//! - capture rate changes take effect on the next start.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pipewire as pw;
use pw::types::ObjectType;

use crate::mirror::{GraphMirror, NodeClass};
use crate::ring::SampleRing;
use crate::targets::{self, CaptureTarget, ConnectSpec};

/// Events the engine reports back to the shell (poll each frame).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioEvent {
    /// The capturable-target list changed (new app, device gone…).
    TargetsChanged,
    /// The running capture stream ended on its own.
    StreamEnded,
    /// The default sink changed (v3 followed it for the ⭐ entry).
    DefaultSinkChanged,
}

enum Command {
    ConfigureRate(u32),
    StartCapture(ConnectSpec),
    StopCapture,
    SweepVacuum(mpsc::Sender<usize>),
    Shutdown,
}

struct SharedFlags {
    capture_running: AtomicBool,
}

/// Handle owned by the shell; everything here is cheap and non-blocking
/// except the explicitly-blocking vacuum sweep.
pub struct AudioEngine {
    commands: pw::channel::Sender<Command>,
    ring: Arc<Mutex<SampleRing>>,
    mirror: Arc<Mutex<GraphMirror>>,
    flags: Arc<SharedFlags>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl AudioEngine {
    /// Start the engine thread and wait for the first registry
    /// round-trip, so the mirror is warm before this returns.
    pub fn spawn(
        sample_rate: u32,
        events: mpsc::Sender<AudioEvent>,
    ) -> Result<AudioEngine, String> {
        let (command_sender, command_receiver) = pw::channel::channel::<Command>();
        let ring = Arc::new(Mutex::new(SampleRing::new(sample_rate)));
        let mirror = Arc::new(Mutex::new(GraphMirror::default()));
        let flags = Arc::new(SharedFlags {
            capture_running: AtomicBool::new(false),
        });
        let (ready_sender, ready_receiver) = mpsc::channel::<Result<(), String>>();

        let thread = {
            let ring = ring.clone();
            let mirror = mirror.clone();
            let flags = flags.clone();
            std::thread::Builder::new()
                .name("phosphor-audio-pw".into())
                .spawn(move || {
                    run_loop(
                        command_receiver,
                        ring,
                        mirror,
                        flags,
                        events,
                        ready_sender,
                        sample_rate,
                    );
                })
                .map_err(|e| format!("audio thread: {e}"))?
        };

        match ready_receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(AudioEngine {
                commands: command_sender,
                ring,
                mirror,
                flags,
                thread: Some(thread),
            }),
            Ok(Err(message)) => Err(message),
            Err(_) => Err("audio engine: no answer from PipeWire within 5 s".into()),
        }
    }

    pub fn targets(&self) -> Vec<CaptureTarget> {
        targets::list_capture_targets(&self.mirror.lock().unwrap())
    }

    pub fn default_monitor_target_id(&self) -> Option<String> {
        targets::default_monitor_target_id(&self.mirror.lock().unwrap())
    }

    /// Resolve a persisted combo id and start capturing it. Returns
    /// false when the target does not exist right now (v3: the combo
    /// shows it unavailable; nothing starts).
    pub fn start_capture(&self, combo_id: &str) -> bool {
        let spec = {
            let mirror = self.mirror.lock().unwrap();
            targets::resolve_combo_id(&mirror, combo_id)
        };
        match spec {
            Some(spec) => {
                let _ = self.commands.send(Command::StartCapture(spec));
                true
            }
            None => false,
        }
    }

    pub fn stop_capture(&self) {
        let _ = self.commands.send(Command::StopCapture);
    }

    pub fn is_capture_running(&self) -> bool {
        self.flags.capture_running.load(Ordering::Relaxed)
    }

    /// Scope feed rate; takes effect on the next capture start (v3 law).
    pub fn configure_sample_rate(&self, sample_rate: u32) {
        self.ring.lock().unwrap().configure_sample_rate(sample_rate);
        let _ = self.commands.send(Command::ConfigureRate(sample_rate));
    }

    pub fn take_stereo_samples(&self) -> Vec<f32> {
        self.ring.lock().unwrap().take_stereo_samples()
    }

    pub fn copy_history(&self, seconds: f32) -> Vec<f32> {
        self.ring.lock().unwrap().copy_history(seconds)
    }

    /// Unload stale vacuum sinks left behind by a crash (kill -9 never
    /// runs atexit — every launch sweeps; v3 law). Blocking, bounded.
    pub fn sweep_stale_vacuum(&self) -> usize {
        let (reply_sender, reply_receiver) = mpsc::channel();
        if self.commands.send(Command::SweepVacuum(reply_sender)).is_err() {
            return 0;
        }
        reply_receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap_or(0)
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Everything the loop thread owns, reachable from listener closures.
struct LoopState {
    capture: Option<CaptureHolder>,
    node_watches: HashMap<u32, NodeWatch>,
    metadata: Option<MetadataHold>,
    sample_rate: u32,
}

struct CaptureHolder {
    stream: pw::stream::StreamRc,
    _listener: pw::stream::StreamListener<()>,
    /// Registry global of the captured node — when this global dies,
    /// the stream ended on its own.
    watched_global: Option<u32>,
}

struct NodeWatch {
    _proxy: pw::node::Node,
    _listener: pw::node::NodeListener,
}

struct MetadataHold {
    #[allow(dead_code)] // vacuum routing writes through this (A4)
    proxy: pw::metadata::Metadata,
    _listener: pw::metadata::MetadataListener,
    global_id: u32,
}

#[allow(clippy::too_many_arguments)]
fn run_loop(
    command_receiver: pw::channel::Receiver<Command>,
    ring: Arc<Mutex<SampleRing>>,
    mirror: Arc<Mutex<GraphMirror>>,
    flags: Arc<SharedFlags>,
    events: mpsc::Sender<AudioEvent>,
    ready_sender: mpsc::Sender<Result<(), String>>,
    sample_rate: u32,
) {
    pw::init();

    macro_rules! ready_or_return {
        ($expr:expr, $what:literal) => {
            match $expr {
                Ok(value) => value,
                Err(error) => {
                    let _ = ready_sender.send(Err(format!("{}: {error}", $what)));
                    return;
                }
            }
        };
    }

    let mainloop = ready_or_return!(pw::main_loop::MainLoopRc::new(None), "main loop");
    let context = ready_or_return!(pw::context::ContextRc::new(&mainloop, None), "context");
    let core = ready_or_return!(context.connect_rc(None), "connect");
    let registry = Rc::new(ready_or_return!(core.get_registry_rc(), "registry"));

    let state = Rc::new(std::cell::RefCell::new(LoopState {
        capture: None,
        node_watches: HashMap::new(),
        metadata: None,
        sample_rate,
    }));

    // --- registry mirror ---------------------------------------------------
    let _registry_listener = {
        let mirror = mirror.clone();
        let events = events.clone();
        let state = state.clone();
        let registry_for_bind = registry.clone();
        let events_remove = events.clone();
        let mirror_remove = mirror.clone();
        let state_remove = state.clone();
        let flags_remove = flags.clone();
        registry
            .add_listener_local()
            .global(move |global| {
                handle_global(
                    global,
                    &mirror,
                    &events,
                    &state,
                    &registry_for_bind,
                );
            })
            .global_remove(move |global_id| {
                handle_global_remove(
                    global_id,
                    &mirror_remove,
                    &events_remove,
                    &state_remove,
                    &flags_remove,
                );
            })
            .register()
    };

    // --- warm-mirror handshake: TWO round-trips then report ready.
    // Trip 1 delivers the existing globals (we bind the "default"
    // metadata while processing them); trip 2 delivers the initial
    // property events of those binds, so default_sink is known before
    // spawn() returns.
    let first = ready_or_return!(core.sync(0), "sync");
    let expected = std::cell::Cell::new(Some(first));
    let round = std::cell::Cell::new(0u8);
    let ready_once = std::cell::Cell::new(Some(ready_sender.clone()));
    let _core_listener = {
        let core = core.clone();
        core.clone()
            .add_listener_local()
            .done(move |id, seq| {
                if id != pw::core::PW_ID_CORE || Some(seq) != expected.get() {
                    return;
                }
                if round.get() == 0 {
                    round.set(1);
                    expected.set(core.sync(0).ok());
                } else if let Some(sender) = ready_once.take() {
                    let _ = sender.send(Ok(()));
                }
            })
            .register()
    };

    // --- commands from the shell -------------------------------------------
    let _attached_receiver = {
        let mainloop_quit = mainloop.clone();
        let core = core.clone();
        let registry = registry.clone();
        let ring = ring.clone();
        let mirror = mirror.clone();
        let flags = flags.clone();
        let events = events.clone();
        let state = state.clone();
        command_receiver.attach(mainloop.as_ref(), move |command| match command {
            Command::ConfigureRate(rate) => {
                state.borrow_mut().sample_rate = rate;
            }
            Command::StartCapture(spec) => {
                stop_capture(&state, &flags);
                match build_capture_stream(
                    &core, &state, &ring, &flags, &events, &spec,
                ) {
                    Ok(holder) => state.borrow_mut().capture = Some(holder),
                    Err(error) => {
                        eprintln!("phosphor-audio: capture failed: {error}");
                        let _ = events.send(AudioEvent::StreamEnded);
                    }
                }
            }
            Command::StopCapture => {
                stop_capture(&state, &flags);
                ring.lock().unwrap().clear_pending();
            }
            Command::SweepVacuum(reply) => {
                let stale: Vec<u32> = {
                    let mirror = mirror.lock().unwrap();
                    mirror
                        .nodes_of_class(NodeClass::Sink)
                        .into_iter()
                        .filter(|n| n.node_name == crate::VACUUM_SINK_NAME)
                        .map(|n| n.global_id)
                        .collect()
                };
                for id in &stale {
                    registry.destroy_global(*id);
                }
                let _ = reply.send(stale.len());
            }
            Command::Shutdown => {
                stop_capture(&state, &flags);
                mainloop_quit.quit();
            }
        })
    };

    mainloop.run();
}

fn handle_global(
    global: &pw::registry::GlobalObject<&pw::spa::utils::dict::DictRef>,
    mirror: &Arc<Mutex<GraphMirror>>,
    events: &mpsc::Sender<AudioEvent>,
    state: &Rc<std::cell::RefCell<LoopState>>,
    registry: &Rc<pw::registry::RegistryRc>,
) {
    match global.type_ {
        ObjectType::Node => {
            let Some(props) = global.props else { return };
            let class = match props.get("media.class") {
                Some("Stream/Output/Audio") => NodeClass::AppStream,
                Some("Audio/Sink") => NodeClass::Sink,
                Some("Audio/Source") => NodeClass::Source,
                _ => return,
            };
            let serial = props
                .get("object.serial")
                .and_then(|s| s.parse::<u64>().ok());
            {
                let mut mirror = mirror.lock().unwrap();
                mirror.upsert_node(global.id, class, crate::mirror::NodeAnnounce {
                    serial,
                    node_name: props.get("node.name").unwrap_or(""),
                    description: props.get("node.description"),
                    app_name: props.get("application.name"),
                    media_name: props.get("media.name"),
                    prior_target: props.get("target.object").or(props.get("node.target")),
                });
            }
            let _ = events.send(AudioEvent::TargetsChanged);

            // Song titles live in node *info* events, not the registry:
            // watch app streams so the combo label follows the music.
            if class == NodeClass::AppStream
                && let Ok(proxy) = registry.bind::<pw::node::Node, _>(global)
            {
                let mirror = mirror.clone();
                let events = events.clone();
                let global_id = global.id;
                let listener = proxy
                    .add_listener_local()
                    .info(move |info| {
                        let Some(props) = info.props() else { return };
                        let changed = mirror.lock().unwrap().update_node_labels(
                            global_id,
                            props.get("application.name"),
                            props.get("media.name"),
                        );
                        if changed {
                            let _ = events.send(AudioEvent::TargetsChanged);
                        }
                    })
                    .register();
                state.borrow_mut().node_watches.insert(
                    global_id,
                    NodeWatch {
                        _proxy: proxy,
                        _listener: listener,
                    },
                );
            }
        }
        ObjectType::Link => {
            let Some(props) = global.props else { return };
            let output = props
                .get("link.output.node")
                .and_then(|s| s.parse::<u32>().ok());
            let input = props
                .get("link.input.node")
                .and_then(|s| s.parse::<u32>().ok());
            if let (Some(output), Some(input)) = (output, input) {
                mirror.lock().unwrap().upsert_link(global.id, output, input);
            }
        }
        ObjectType::Metadata => {
            let Some(props) = global.props else { return };
            if props.get("metadata.name") != Some("default") {
                return;
            }
            if let Ok(proxy) = registry.bind::<pw::metadata::Metadata, _>(global) {
                let mirror = mirror.clone();
                let events = events.clone();
                let listener = proxy
                    .add_listener_local()
                    .property(move |_subject, key, _type, value| {
                        if key == Some("default.audio.sink") {
                            let name = value.and_then(parse_metadata_name);
                            let mut mirror = mirror.lock().unwrap();
                            if mirror.default_sink != name {
                                mirror.default_sink = name;
                                drop(mirror);
                                let _ = events.send(AudioEvent::DefaultSinkChanged);
                            }
                        }
                        0
                    })
                    .register();
                state.borrow_mut().metadata = Some(MetadataHold {
                    proxy,
                    _listener: listener,
                    global_id: global.id,
                });
            }
        }
        _ => {}
    }
}

fn handle_global_remove(
    global_id: u32,
    mirror: &Arc<Mutex<GraphMirror>>,
    events: &mpsc::Sender<AudioEvent>,
    state: &Rc<std::cell::RefCell<LoopState>>,
    flags: &Arc<SharedFlags>,
) {
    let removed_node = mirror.lock().unwrap().remove_global(global_id);
    let mut state_mut = state.borrow_mut();
    state_mut.node_watches.remove(&global_id);
    if state_mut
        .metadata
        .as_ref()
        .is_some_and(|m| m.global_id == global_id)
    {
        state_mut.metadata = None;
    }
    // The captured node vanished → the stream ended on its own (v3:
    // parec died and the reader loop reported it).
    let captured_died = state_mut
        .capture
        .as_ref()
        .and_then(|c| c.watched_global)
        == Some(global_id);
    if captured_died {
        if let Some(holder) = state_mut.capture.take() {
            let _ = holder.stream.disconnect();
        }
        flags.capture_running.store(false, Ordering::Relaxed);
        let _ = events.send(AudioEvent::StreamEnded);
    }
    drop(state_mut);
    if removed_node.is_some() {
        let _ = events.send(AudioEvent::TargetsChanged);
    }
}

/// `default.audio.sink` metadata value is JSON: `{"name":"sink-name"}`.
fn parse_metadata_name(value: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(value).ok()?;
    parsed
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

fn stop_capture(state: &Rc<std::cell::RefCell<LoopState>>, flags: &Arc<SharedFlags>) {
    if let Some(holder) = state.borrow_mut().capture.take() {
        let _ = holder.stream.disconnect();
    }
    flags.capture_running.store(false, Ordering::Relaxed);
}

fn build_capture_stream(
    core: &pw::core::CoreRc,
    state: &Rc<std::cell::RefCell<LoopState>>,
    ring: &Arc<Mutex<SampleRing>>,
    flags: &Arc<SharedFlags>,
    events: &mpsc::Sender<AudioEvent>,
    spec: &ConnectSpec,
) -> Result<CaptureHolder, pw::Error> {
    use pw::properties::properties;

    let sample_rate = state.borrow().sample_rate;
    // v3 asked parec for 20 ms of latency; ask the graph for the same.
    let latency_frames = (sample_rate / 50).max(1);

    let mut props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::APP_NAME => "Phosphor",
        *pw::keys::NODE_NAME => "phosphor-scope",
        *pw::keys::NODE_LATENCY => format!("{latency_frames}/{sample_rate}").as_str(),
    };
    let watched_global = match spec {
        ConnectSpec::SinkMonitor { node_name } => {
            props.insert(*pw::keys::TARGET_OBJECT, node_name.as_str());
            props.insert(*pw::keys::STREAM_CAPTURE_SINK, "true");
            None
        }
        ConnectSpec::SourceDevice { node_name } => {
            props.insert(*pw::keys::TARGET_OBJECT, node_name.as_str());
            None
        }
        ConnectSpec::AppStream { global_id, serial } => {
            props.insert(*pw::keys::TARGET_OBJECT, serial.to_string().as_str());
            Some(*global_id)
        }
    };

    let stream = pw::stream::StreamRc::new(core.clone(), "phosphor-capture", props)?;

    let listener = {
        let ring = ring.clone();
        let flags_state = flags.clone();
        let events = events.clone();
        stream
            .add_local_listener::<()>()
            .state_changed(move |_stream, _data, _old, new| {
                use pw::stream::StreamState;
                match new {
                    StreamState::Streaming => {
                        flags_state.capture_running.store(true, Ordering::Relaxed);
                    }
                    StreamState::Error(_) => {
                        flags_state.capture_running.store(false, Ordering::Relaxed);
                        let _ = events.send(AudioEvent::StreamEnded);
                    }
                    _ => {}
                }
            })
            .process(move |stream, _data| {
                while let Some(mut buffer) = stream.dequeue_buffer() {
                    let datas = buffer.datas_mut();
                    if datas.is_empty() {
                        continue;
                    }
                    let data = &mut datas[0];
                    let offset = data.chunk().offset() as usize;
                    let size = data.chunk().size() as usize;
                    if let Some(slice) = data.data() {
                        let end = (offset + size).min(slice.len());
                        if offset < end {
                            ring.lock()
                                .unwrap()
                                .push_interleaved_le_bytes(&slice[offset..end]);
                        }
                    }
                }
            })
            .register()?
    };

    // Ask for f32 stereo at the scope pipe rate; the stream's converter
    // resamples/remixes from whatever the target runs at (v3 let pulse
    // do exactly this inside parec).
    let mut audio_info = pw::spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(pw::spa::param::audio::AudioFormat::F32LE);
    audio_info.set_rate(sample_rate);
    audio_info.set_channels(2);
    let object = pw::spa::pod::Object {
        type_: pw::spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: pw::spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    let values: Vec<u8> = pw::spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(object),
    )
    .map_err(|_| pw::Error::CreationFailed)?
    .0
    .into_inner();
    let mut params = [pw::spa::pod::Pod::from_bytes(&values)
        .ok_or(pw::Error::CreationFailed)?];

    stream.connect(
        pw::spa::utils::Direction::Input,
        None,
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut params,
    )?;

    Ok(CaptureHolder {
        stream,
        _listener: listener,
        watched_global,
    })
}
