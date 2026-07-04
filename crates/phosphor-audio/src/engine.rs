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
    /// File playback reached its true end on its own (never sent for
    /// an explicit stop — v3's on_stream_ended contract).
    PlaybackEnded,
    /// A track began decoding (first play or a gapless splice);
    /// metadata + cover art are ready to read.
    TrackStarted { path: std::path::PathBuf },
}

enum Command {
    ConfigureRate(u32),
    StartCapture(ConnectSpec),
    StopCapture,
    CreatePlayback {
        rate: u32,
        ring: Arc<crate::playback::AudibleRing>,
        volume: f32,
    },
    DestroyPlayback,
    SetPlaybackActive(bool),
    SetVolume(f32),
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
    events: mpsc::Sender<AudioEvent>,
    sample_rate: std::sync::atomic::AtomicU32,
    playback: Mutex<Option<crate::playback::PlayerSession>>,
    playback_paused: AtomicBool,
    volume: Mutex<f32>,
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
            let events = events.clone();
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
                events,
                sample_rate: std::sync::atomic::AtomicU32::new(sample_rate),
                playback: Mutex::new(None),
                playback_paused: AtomicBool::new(false),
                volume: Mutex::new(1.0),
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
        self.sample_rate
            .store(sample_rate, std::sync::atomic::Ordering::Relaxed);
        let _ = self.commands.send(Command::ConfigureRate(sample_rate));
    }

    // ---- file playback ---------------------------------------------------

    /// Play an audio file (or .phos postcard), feeding the scope the
    /// same resampled stream the ear gets. `vacuum` plays as light
    /// only. Any previous playback stops silently first (v3 law: an
    /// explicit stop never reports "ended").
    pub fn start_file(&self, path: &std::path::Path, seek_seconds: f64,
                      loop_forever: bool, vacuum: bool) {
        self.stop_playback();
        let pipe_rate = self.sample_rate.load(std::sync::atomic::Ordering::Relaxed);
        let audible = if vacuum {
            None
        } else {
            Some(crate::playback::AudibleRing::new(pipe_rate))
        };
        if let Some(ring) = &audible {
            let _ = self.commands.send(Command::CreatePlayback {
                rate: pipe_rate,
                ring: ring.clone(),
                volume: *self.volume.lock().unwrap(),
            });
        }
        let session = crate::playback::spawn_player(
            crate::playback::PlayerConfig {
                path: path.to_path_buf(),
                seek_seconds,
                loop_forever,
                vacuum,
                pipe_rate,
            },
            self.ring.clone(),
            audible,
            self.events.clone(),
        );
        *self.playback.lock().unwrap() = Some(session);
        self.playback_paused.store(false, Ordering::Relaxed);
    }

    /// Stop playback explicitly — no PlaybackEnded event (v3 contract).
    pub fn stop_playback(&self) {
        let session = self.playback.lock().unwrap().take();
        if let Some(mut session) = session {
            if let Some(ring) = &session.audible {
                ring.close();
            }
            let _ = session.control.send(crate::playback::PlayerCommand::Stop);
            let _ = self.commands.send(Command::DestroyPlayback);
            if let Some(thread) = session.thread.take() {
                let _ = thread.join();
            }
            // pending cleared, history kept (a clip right after stop
            // still works — v3 law)
            self.ring.lock().unwrap().clear_pending();
        }
        self.playback_paused.store(false, Ordering::Relaxed);
    }

    pub fn is_playing_file(&self) -> bool {
        self.playback.lock().unwrap().is_some()
    }

    /// Freeze/unfreeze file playback. Audible: the PW stream goes
    /// inactive and backpressure freezes the decoder (v3: SIGSTOP).
    /// Vacuum: the reader gate (v3: the vacuum gate event).
    pub fn set_playback_paused(&self, paused: bool) {
        if paused == self.playback_paused.load(Ordering::Relaxed) {
            return;
        }
        let guard = self.playback.lock().unwrap();
        let Some(session) = guard.as_ref() else { return };
        if session.vacuum {
            let command = if paused {
                crate::playback::PlayerCommand::Pause
            } else {
                crate::playback::PlayerCommand::Resume
            };
            let _ = session.control.send(command);
        } else {
            let _ = self.commands.send(Command::SetPlaybackActive(!paused));
        }
        drop(guard);
        self.playback_paused.store(paused, Ordering::Relaxed);
    }

    pub fn playback_paused(&self) -> bool {
        self.playback_paused.load(Ordering::Relaxed)
    }

    /// How far into the current file playback we are, in seconds.
    pub fn playback_position_seconds(&self) -> f64 {
        self.playback
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| {
                s.shared
                    .position_micros
                    .load(std::sync::atomic::Ordering::Relaxed) as f64
                    / 1e6
            })
            .unwrap_or(0.0)
    }

    /// Queue the next track for a gapless splice at EOF (None clears).
    pub fn set_next_track(&self, path: Option<std::path::PathBuf>) {
        if let Some(session) = self.playback.lock().unwrap().as_ref() {
            let _ = session
                .control
                .send(crate::playback::PlayerCommand::SetNext(path));
        }
    }

    /// Playback volume (0.0–1.0), applied to the PW stream.
    pub fn set_volume(&self, volume: f32) {
        *self.volume.lock().unwrap() = volume;
        let _ = self.commands.send(Command::SetVolume(volume));
    }

    pub fn current_track_metadata(&self) -> Option<crate::metadata::TrackMetadata> {
        self.playback
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| s.shared.current_metadata.lock().unwrap().clone())
    }

    pub fn current_cover_art(&self) -> Option<crate::metadata::CoverArt> {
        self.playback
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|s| s.shared.current_cover.lock().unwrap().clone())
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
    playback: Option<PlaybackHolder>,
    node_watches: HashMap<u32, NodeWatch>,
    metadata: Option<MetadataHold>,
    sample_rate: u32,
}

struct PlaybackHolder {
    stream: pw::stream::StreamRc,
    _listener: pw::stream::StreamListener<()>,
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
        playback: None,
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
            Command::CreatePlayback { rate, ring, volume } => {
                if let Some(old) = state.borrow_mut().playback.take() {
                    let _ = old.stream.disconnect();
                }
                match build_playback_stream(&core, rate, ring, volume) {
                    Ok(holder) => state.borrow_mut().playback = Some(holder),
                    Err(error) => {
                        eprintln!("phosphor-audio: playback stream failed: {error}");
                    }
                }
            }
            Command::DestroyPlayback => {
                if let Some(holder) = state.borrow_mut().playback.take() {
                    let _ = holder.stream.disconnect();
                }
            }
            Command::SetPlaybackActive(active) => {
                if let Some(holder) = state.borrow().playback.as_ref() {
                    let _ = holder.stream.set_active(active);
                }
            }
            Command::SetVolume(volume) => {
                if let Some(holder) = state.borrow().playback.as_ref() {
                    let _ = holder.stream.set_control(
                        pw::spa::sys::SPA_PROP_volume,
                        &[volume],
                    );
                }
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

/// The audible output: a playback stream at the pipe rate pulling from
/// the audible ring (PW converts to the device rate). Underruns emit
/// silence — same as pacat starving.
fn build_playback_stream(
    core: &pw::core::CoreRc,
    sample_rate: u32,
    ring: Arc<crate::playback::AudibleRing>,
    volume: f32,
) -> Result<PlaybackHolder, pw::Error> {
    use pw::properties::properties;

    // node.latency pins our cycle to what one buffer holds — without
    // it the graph may run a bigger cycle than a single 1024-frame
    // buffer and consumption drops below realtime (measured: 0.35×,
    // pw-top showed 16 cycles/s). pacat forced --latency-msec for the
    // same reason (v3 law: 60 ms; we run tighter).
    let latency_frames = 1024u32;
    let props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Playback",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::APP_NAME => "Phosphor",
        *pw::keys::NODE_NAME => "phosphor-playback",
        *pw::keys::NODE_LATENCY => format!("{latency_frames}/{sample_rate}").as_str(),
    };
    let stream = pw::stream::StreamRc::new(core.clone(), "phosphor-playback", props)?;

    let listener = {
        let mut scratch: Vec<f32> = Vec::new();
        stream
            .add_local_listener::<()>()
            .process(move |stream, _data| {
                while let Some(mut buffer) = stream.dequeue_buffer() {
                    let requested_frames = buffer.requested() as usize;
                    let datas = buffer.datas_mut();
                    if datas.is_empty() {
                        continue;
                    }
                    let data = &mut datas[0];
                    let Some(slice) = data.data() else { continue };
                    let slice_frames = slice.len() / 8;
                    let frames = if requested_frames > 0 {
                        requested_frames.min(slice_frames)
                    } else {
                        slice_frames
                    };
                    scratch.resize(frames * 2, 0.0);
                    ring.pop_into(&mut scratch);
                    for (quad, value) in
                        slice.chunks_exact_mut(4).zip(scratch.iter())
                    {
                        quad.copy_from_slice(&value.to_le_bytes());
                    }
                    let chunk = data.chunk_mut();
                    *chunk.offset_mut() = 0;
                    *chunk.stride_mut() = 8;
                    *chunk.size_mut() = (frames * 8) as u32;
                }
            })
            .register()?
    };

    let mut audio_info = pw::spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(pw::spa::param::audio::AudioFormat::F32LE);
    audio_info.set_rate(sample_rate);
    audio_info.set_channels(2);
    let mut position = [0; pw::spa::param::audio::MAX_CHANNELS];
    position[0] = pw::spa::sys::SPA_AUDIO_CHANNEL_FL;
    position[1] = pw::spa::sys::SPA_AUDIO_CHANNEL_FR;
    audio_info.set_position(position);
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

    // RT_PROCESS: the callback runs on the graph's data thread — the
    // non-RT main-loop hop measurably missed ~2/3 of cycles (0.35×
    // consumption). The callback only memcpys out of the audible
    // ring's mutex (µs holds), which is RT-safe in practice.
    stream.connect(
        pw::spa::utils::Direction::Output,
        None,
        pw::stream::StreamFlags::AUTOCONNECT
            | pw::stream::StreamFlags::MAP_BUFFERS
            | pw::stream::StreamFlags::RT_PROCESS,
        &mut params,
    )?;
    let _ = stream.set_control(pw::spa::sys::SPA_PROP_volume, &[volume]);

    Ok(PlaybackHolder {
        stream,
        _listener: listener,
    })
}
