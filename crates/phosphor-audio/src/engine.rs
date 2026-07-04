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
    /// Multi-app mixing: N app streams, each into its own member
    /// buffer; the facade folds them into the scope ring at drain
    /// time (new in v4 — V4PLAN step 8).
    StartMix(Vec<(ConnectSpec, Arc<Mutex<Vec<f32>>>)>),
    SweepVacuum(mpsc::Sender<usize>),
    /// Route one app's stream into the vacuum null sink. Replies Ok
    /// once the app→vacuum link is confirmed on the graph (the facade
    /// owns the timeout + rollback).
    RouteVacuum {
        app_global: u32,
        reply: mpsc::Sender<Result<(), String>>,
    },
    /// Put the world back: metadata restored exactly, sink destroyed.
    ReleaseVacuum(mpsc::Sender<()>),
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
    /// pactl module id of the live vacuum sink (the hatch — see the
    /// vacuum section below).
    vacuum_module: Mutex<Option<String>>,
    /// Live mix member buffers (empty = single-capture mode).
    mix_members: Mutex<Vec<Arc<Mutex<Vec<f32>>>>>,
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
                vacuum_module: Mutex::new(None),
                mix_members: Mutex::new(Vec::new()),
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
        self.mix_members.lock().unwrap().clear();
        let _ = self.commands.send(Command::StopCapture);
    }

    /// Start a multi-app mix: every resolvable app combo id gets its
    /// own capture stream; the scope sees their sum. Returns how many
    /// resolved (0 = nothing started).
    pub fn start_capture_mix(&self, combo_ids: &[String]) -> usize {
        let mut specs = Vec::new();
        {
            let mirror = self.mirror.lock().unwrap();
            for combo_id in combo_ids {
                if let Some(spec @ ConnectSpec::AppStream { .. }) =
                    targets::resolve_combo_id(&mirror, combo_id)
                {
                    specs.push(spec);
                }
            }
        }
        if specs.is_empty() {
            return 0;
        }
        let members: Vec<(ConnectSpec, Arc<Mutex<Vec<f32>>>)> = specs
            .into_iter()
            .map(|spec| (spec, Arc::new(Mutex::new(Vec::new()))))
            .collect();
        *self.mix_members.lock().unwrap() =
            members.iter().map(|(_, buffer)| buffer.clone()).collect();
        let count = members.len();
        let _ = self.commands.send(Command::StartMix(members));
        count
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
        self.fold_mix_into_ring();
        self.ring.lock().unwrap().take_stereo_samples()
    }

    /// Cheap peek for the shell's idle loop: is there audio waiting?
    pub fn pending_scope_samples(&self) -> usize {
        let mix_pending: usize = self
            .mix_members
            .lock()
            .unwrap()
            .iter()
            .map(|m| m.lock().unwrap().len())
            .sum();
        self.ring.lock().unwrap().pending_len() + mix_pending
    }

    /// Fold pending mix-member audio into the scope ring: sum with
    /// zero-padding to the longest member (a silent app contributes
    /// silence, a paused one just stops contributing). Inter-app skew
    /// is bounded by one drain period; each app's own L/R stays
    /// coherent, so each source's shape is exact.
    fn fold_mix_into_ring(&self) {
        let members = self.mix_members.lock().unwrap();
        if members.is_empty() {
            return;
        }
        let mut mixed: Vec<f32> = Vec::new();
        for member in members.iter() {
            let mut buffer = member.lock().unwrap();
            if buffer.len() > mixed.len() {
                mixed.resize(buffer.len(), 0.0);
            }
            for (slot, sample) in mixed.iter_mut().zip(buffer.iter()) {
                *slot += *sample;
            }
            buffer.clear();
        }
        if !mixed.is_empty() {
            let whole = mixed.len() - mixed.len() % 2;
            self.ring
                .lock()
                .unwrap()
                .push_interleaved(&mixed[..whole]);
        }
    }

    pub fn copy_history(&self, seconds: f32) -> Vec<f32> {
        self.ring.lock().unwrap().copy_history(seconds)
    }

    /// Unload stale vacuum sinks left behind by a crash (kill -9 never
    /// runs atexit — every launch sweeps; v3 law). Blocking, bounded.
    ///
    /// ORDER MATTERS (Gate A receipt): module unload FIRST — the
    /// server migrates streams gracefully on unload. Destroying the
    /// backing node natively kills pulse-shim streams playing into it
    /// ("Connection terminated"), so the native broom only runs for
    /// module-less leftovers that somehow remain after.
    pub fn sweep_stale_vacuum(&self) -> usize {
        let mut removed = sweep_stale_pulse_modules();
        let survivor = std::process::Command::new("pactl")
            .args(["list", "short", "sinks"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout).contains(crate::VACUUM_SINK_NAME)
            })
            .unwrap_or(true);
        if survivor {
            let (reply_sender, reply_receiver) = mpsc::channel();
            if self.commands.send(Command::SweepVacuum(reply_sender)).is_ok() {
                removed += reply_receiver
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap_or(0);
            }
        }
        removed
    }

    // ---- vacuum ------------------------------------------------------------
    //
    // THE HATCH, INVOKED (decision made once, Gate A receipt): sink
    // lifecycle goes through `pactl load-module/unload-module`.
    // Reason: destroying a null-sink NODE natively (registry destroy)
    // kills pulse-shim streams playing into it on PipeWire 1.0.5
    // ("Connection terminated" — the gate script caught paplay dying),
    // while module unload migrates them gracefully. A module also
    // survives kill -9 exactly like v3 (app keeps playing into the
    // void; the next launch's sweep unloads it and the server rescues
    // the stream). Routing, verification (link watch), and restore
    // stay fully native — pactl is module load/unload ONLY, per the
    // pre-authorized V4PLAN escape hatch.

    /// Route one app (by its stable key, e.g. "Google Chrome") into
    /// the vacuum. On success returns the combo id of the monitor to
    /// scope (v3 returned "phosphor_vacuum.monitor" the same way).
    /// On any failure the world is put back first (restore is sacred).
    pub fn vacuum_route_app(&self, stable_key: &str) -> Result<String, String> {
        self.vacuum_release();
        let spec = {
            let mirror = self.mirror.lock().unwrap();
            targets::resolve_combo_id(&mirror, &format!("app:{stable_key}"))
        };
        let Some(ConnectSpec::AppStream { global_id, .. }) = spec else {
            return Err(format!("no playing app matches \"{stable_key}\""));
        };
        let module_id = pactl_load_vacuum_sink()?;
        *self.vacuum_module.lock().unwrap() = Some(module_id);
        let (reply_sender, reply_receiver) = mpsc::channel();
        let _ = self.commands.send(Command::RouteVacuum {
            app_global: global_id,
            reply: reply_sender,
        });
        match reply_receiver.recv_timeout(Duration::from_millis(2500)) {
            Ok(Ok(())) => Ok(format!("device:{}.monitor", crate::VACUUM_SINK_NAME)),
            Ok(Err(message)) => {
                self.vacuum_release();
                Err(message)
            }
            Err(_) => {
                self.vacuum_release();
                Err("vacuum route: no link confirmation within 2.5 s".into())
            }
        }
    }

    /// Restore the routed app, then unload the sink module (that
    /// order — the stream is back home before its refuge vanishes).
    /// Safe to call twice.
    pub fn vacuum_release(&self) {
        let (reply_sender, reply_receiver) = mpsc::channel();
        if self.commands.send(Command::ReleaseVacuum(reply_sender)).is_ok() {
            let _ = reply_receiver.recv_timeout(Duration::from_secs(2));
        }
        if let Some(module_id) = self.vacuum_module.lock().unwrap().take() {
            let _ = std::process::Command::new("pactl")
                .args(["unload-module", &module_id])
                .status();
        }
    }

    pub fn vacuum_active(&self) -> bool {
        self.vacuum_module.lock().unwrap().is_some()
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        // Restore is sacred on quit: routing restored by the loop's
        // Shutdown handler, then the sink module unloaded here.
        let _ = self.commands.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        if let Some(module_id) = self.vacuum_module.lock().unwrap().take() {
            let _ = std::process::Command::new("pactl")
                .args(["unload-module", &module_id])
                .status();
        }
    }
}

/// Everything the loop thread owns, reachable from listener closures.
struct LoopState {
    capture: Option<CaptureHolder>,
    mix: Vec<CaptureHolder>,
    playback: Option<PlaybackHolder>,
    node_watches: HashMap<u32, NodeWatch>,
    metadata: Option<MetadataHold>,
    vacuum: Option<VacuumHold>,
    sample_rate: u32,
}

/// Where a capture stream's samples land.
enum CaptureDestination {
    /// Straight into the scope ring (single-target capture).
    ScopeRing(Arc<Mutex<SampleRing>>),
    /// Into a mix member buffer, folded by the facade at drain time.
    /// Capped at ~2 s so a stalled shell never balloons memory.
    MemberBuffer(Arc<Mutex<Vec<f32>>>, usize),
}

/// The live vacuum routing (the sink itself is a pactl-loaded module
/// owned by the facade — see the hatch note on [`AudioEngine::vacuum_route_app`]).
struct VacuumHold {
    sink_global: Option<u32>,
    sink_serial: Option<u64>,
    app_global: u32,
    /// The app's explicit target.object before us (None = follow
    /// default) — restore puts back exactly this. v3's previous_sink.
    prior_target: Option<String>,
    /// Present until the app→sink link is confirmed.
    pending_reply: Option<mpsc::Sender<Result<(), String>>>,
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
        mix: Vec::new(),
        playback: None,
        node_watches: HashMap::new(),
        metadata: None,
        vacuum: None,
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
                let destination = CaptureDestination::ScopeRing(ring.clone());
                match build_capture_stream(
                    &core, &state, destination, &flags, &events, &spec,
                ) {
                    Ok(holder) => state.borrow_mut().capture = Some(holder),
                    Err(error) => {
                        eprintln!("phosphor-audio: capture failed: {error}");
                        let _ = events.send(AudioEvent::StreamEnded);
                    }
                }
            }
            Command::StartMix(members) => {
                stop_capture(&state, &flags);
                let cap_samples =
                    state.borrow().sample_rate as usize * 2 * 2; // 2 s stereo
                for (spec, buffer) in members {
                    let destination =
                        CaptureDestination::MemberBuffer(buffer, cap_samples);
                    match build_capture_stream(
                        &core, &state, destination, &flags, &events, &spec,
                    ) {
                        Ok(holder) => state.borrow_mut().mix.push(holder),
                        Err(error) => {
                            eprintln!("phosphor-audio: mix member failed: {error}");
                        }
                    }
                }
                if state.borrow().mix.is_empty() {
                    let _ = events.send(AudioEvent::StreamEnded);
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
                let own_sink = state
                    .borrow()
                    .vacuum
                    .as_ref()
                    .and_then(|v| v.sink_global);
                let stale: Vec<u32> = {
                    let mirror = mirror.lock().unwrap();
                    mirror
                        .nodes_of_class(NodeClass::Sink)
                        .into_iter()
                        .filter(|n| {
                            n.node_name == crate::VACUUM_SINK_NAME
                                && Some(n.global_id) != own_sink
                        })
                        .map(|n| n.global_id)
                        .collect()
                };
                for id in &stale {
                    registry.destroy_global(*id);
                }
                let _ = reply.send(stale.len());
            }
            Command::RouteVacuum { app_global, reply } => {
                // Route is release-first (v3 law: route() calls release()).
                release_vacuum(&state, &mirror);
                let prior_target = {
                    let mirror = mirror.lock().unwrap();
                    let candidate = mirror.explicit_targets.get(&app_global).cloned();
                    // Never treat a vacuum as "where it lived": if the
                    // recorded target IS a phosphor_vacuum sink (ours,
                    // or a stale one), restore-to-default instead.
                    candidate.filter(|value| {
                        !mirror.nodes_of_class(NodeClass::Sink).iter().any(|n| {
                            n.node_name == crate::VACUUM_SINK_NAME
                                && (n.serial.map(|s| s.to_string()).as_deref()
                                    == Some(value.as_str())
                                    || n.node_name == value.as_str())
                        }) && value != crate::VACUUM_SINK_NAME
                    })
                };
                if state.borrow().metadata.is_none() {
                    let _ = reply.send(Err(
                        "no \"default\" metadata object on this server".into(),
                    ));
                    return;
                }
                // The sink module is already loading (facade, pactl).
                // Everything else is event-driven: the sink's global
                // announce writes the metadata move; the app→sink
                // link announce confirms + replies. If the sink is
                // ALREADY announced (fast pactl), do phase 2 now.
                let existing = mirror
                    .lock()
                    .unwrap()
                    .find_node_by_name(NodeClass::Sink, crate::VACUUM_SINK_NAME)
                    .map(|n| (n.global_id, n.serial));
                state.borrow_mut().vacuum = Some(VacuumHold {
                    sink_global: None,
                    sink_serial: None,
                    app_global,
                    prior_target,
                    pending_reply: Some(reply),
                });
                if let Some((sink_global, sink_serial)) = existing {
                    if let Some(hold) = state.borrow_mut().vacuum.as_mut() {
                        hold.sink_global = Some(sink_global);
                        hold.sink_serial = sink_serial;
                    }
                    if let (Some(metadata), Some(serial)) =
                        (state.borrow().metadata.as_ref(), sink_serial)
                    {
                        metadata.proxy.set_property(
                            app_global,
                            "target.object",
                            Some("Spa:Id"),
                            Some(&serial.to_string()),
                        );
                    }
                }
            }
            Command::ReleaseVacuum(reply) => {
                release_vacuum(&state, &mirror);
                let _ = reply.send(());
            }
            Command::Shutdown => {
                // Restore is sacred: put the routing back before quit
                // (the facade unloads the sink module after the join).
                release_vacuum(&state, &mirror);
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
            let node_name = props.get("node.name").unwrap_or("");
            {
                let mut mirror = mirror.lock().unwrap();
                mirror.upsert_node(global.id, class, crate::mirror::NodeAnnounce {
                    serial,
                    node_name,
                    description: props.get("node.description"),
                    app_name: props.get("application.name"),
                    media_name: props.get("media.name"),
                    prior_target: props.get("target.object").or(props.get("node.target")),
                });
            }
            let _ = events.send(AudioEvent::TargetsChanged);

            // Vacuum phase 2: our null sink just appeared — move the
            // app into it (metadata target.object = sink serial, the
            // same write pactl move-sink-input performs).
            if class == NodeClass::Sink && node_name == crate::VACUUM_SINK_NAME {
                let mut state_mut = state.borrow_mut();
                let (write, app_global) = match state_mut.vacuum.as_mut() {
                    Some(hold) if hold.sink_global.is_none() => {
                        hold.sink_global = Some(global.id);
                        hold.sink_serial = serial;
                        (serial, hold.app_global)
                    }
                    _ => (None, 0),
                };
                let metadata_ready = state_mut.metadata.is_some();
                drop(state_mut);
                if let Some(sink_serial) = write {
                    if metadata_ready {
                        if let Some(metadata) = state.borrow().metadata.as_ref() {
                            metadata.proxy.set_property(
                                app_global,
                                "target.object",
                                Some("Spa:Id"),
                                Some(&sink_serial.to_string()),
                            );
                        }
                    } else if let Some(hold) = state.borrow_mut().vacuum.as_mut()
                        && let Some(reply) = hold.pending_reply.take()
                    {
                        let _ = reply.send(Err("metadata object missing".into()));
                    }
                }
            }

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

                // Vacuum phase 3: the app→sink link exists — the move
                // is REAL (verified, not assumed). Confirm the route.
                let mut state_mut = state.borrow_mut();
                if let Some(hold) = state_mut.vacuum.as_mut()
                    && hold.pending_reply.is_some()
                    && output == hold.app_global
                    && Some(input) == hold.sink_global
                    && let Some(reply) = hold.pending_reply.take()
                {
                    let _ = reply.send(Ok(()));
                }
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
                    .property(move |subject, key, _type, value| {
                        if key == Some("default.audio.sink") {
                            let name = value.and_then(parse_metadata_name);
                            let mut mirror = mirror.lock().unwrap();
                            if mirror.default_sink != name {
                                mirror.default_sink = name;
                                drop(mirror);
                                let _ = events.send(AudioEvent::DefaultSinkChanged);
                            }
                        } else if subject != 0
                            && matches!(key, Some("target.object") | None)
                        {
                            // Track explicit routing so vacuum restore
                            // puts back exactly what was there.
                            let mut mirror = mirror.lock().unwrap();
                            match (key, value) {
                                (Some(_), Some(v)) => {
                                    mirror.explicit_targets.insert(subject, v.to_string());
                                }
                                _ => {
                                    mirror.explicit_targets.remove(&subject);
                                }
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
    // A mix member died: drop it, keep mixing; the LAST death ends
    // the stream like a single capture would.
    let had_mix = !state_mut.mix.is_empty();
    if had_mix {
        let mut dead = Vec::new();
        state_mut.mix.retain(|holder| {
            if holder.watched_global == Some(global_id) {
                dead.push(());
                false
            } else {
                true
            }
        });
        if !dead.is_empty() && state_mut.mix.is_empty() {
            flags.capture_running.store(false, Ordering::Relaxed);
            let _ = events.send(AudioEvent::StreamEnded);
        }
    }
    drop(state_mut);
    if removed_node.is_some() {
        let _ = events.send(AudioEvent::TargetsChanged);
    }
}

/// Load the vacuum null sink via the hatch. v3 law verbatim: the
/// module id must be digits (pactl prints it on success and nothing
/// on most failures).
fn pactl_load_vacuum_sink() -> Result<String, String> {
    let output = std::process::Command::new("pactl")
        .args([
            "load-module",
            "module-null-sink",
            &format!("sink_name={}", crate::VACUUM_SINK_NAME),
            "sink_properties=device.description=Phosphor\\ Vacuum",
        ])
        .output()
        .map_err(|e| format!("pactl: {e}"))?;
    let module_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !output.status.success() || module_id.is_empty()
        || !module_id.bytes().all(|b| b.is_ascii_digit())
    {
        return Err("could not create the vacuum sink".into());
    }
    Ok(module_id)
}

/// pactl leftovers from a crashed v3 OR v4: unload any
/// module-null-sink whose arguments name the vacuum. Return code is
/// the only truth (pactl is silent on success — v3 law).
fn sweep_stale_pulse_modules() -> usize {
    let Ok(output) = std::process::Command::new("pactl")
        .args(["list", "short", "modules"])
        .output()
    else {
        return 0;
    };
    if !output.status.success() {
        return 0;
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    let mut removed = 0;
    for line in listing.lines() {
        let mut parts = line.split('\t');
        let (Some(module_id), Some(module_name)) = (parts.next(), parts.next()) else {
            continue;
        };
        let arguments = parts.next().unwrap_or("");
        if module_name == "module-null-sink" && arguments.contains(crate::VACUUM_SINK_NAME) {
            let unloaded = std::process::Command::new("pactl")
                .args(["unload-module", module_id])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if unloaded {
                removed += 1;
            }
        }
    }
    removed
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
    let mut state_mut = state.borrow_mut();
    if let Some(holder) = state_mut.capture.take() {
        let _ = holder.stream.disconnect();
    }
    for holder in state_mut.mix.drain(..) {
        let _ = holder.stream.disconnect();
    }
    drop(state_mut);
    flags.capture_running.store(false, Ordering::Relaxed);
}

/// Put the routing back: the app's target.object restored to exactly
/// what it was (or cleared → follows default, v3's @DEFAULT_SINK@
/// fallback). The sink module itself is unloaded by the facade via
/// pactl AFTER this (restore first, then the sink goes). Safe twice.
fn release_vacuum(
    state: &Rc<std::cell::RefCell<LoopState>>,
    mirror: &Arc<Mutex<GraphMirror>>,
) {
    let Some(hold) = state.borrow_mut().vacuum.take() else { return };
    if let Some(reply) = hold.pending_reply {
        let _ = reply.send(Err("vacuum released before the move confirmed".into()));
    }
    let app_alive = mirror.lock().unwrap().node(hold.app_global).is_some();
    if app_alive
        && let Some(metadata) = state.borrow().metadata.as_ref()
    {
        match &hold.prior_target {
            Some(previous) => metadata.proxy.set_property(
                hold.app_global,
                "target.object",
                Some("Spa:Id"),
                Some(previous),
            ),
            None => metadata.proxy.set_property(
                hold.app_global,
                "target.object",
                None,
                None,
            ),
        }
    }
    // Fix the mirror NOW rather than waiting for the metadata event —
    // an immediate re-route must not capture our own write as "prior".
    {
        let mut mirror = mirror.lock().unwrap();
        match &hold.prior_target {
            Some(previous) => {
                mirror
                    .explicit_targets
                    .insert(hold.app_global, previous.clone());
            }
            None => {
                mirror.explicit_targets.remove(&hold.app_global);
            }
        }
    }
}

fn build_capture_stream(
    core: &pw::core::CoreRc,
    state: &Rc<std::cell::RefCell<LoopState>>,
    destination: CaptureDestination,
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
                            let bytes = &slice[offset..end];
                            match &destination {
                                CaptureDestination::ScopeRing(ring) => {
                                    ring.lock()
                                        .unwrap()
                                        .push_interleaved_le_bytes(bytes);
                                }
                                CaptureDestination::MemberBuffer(member, cap) => {
                                    let mut buffer = member.lock().unwrap();
                                    let whole = bytes.len() - bytes.len() % 4;
                                    buffer.reserve(whole / 4);
                                    for quad in bytes[..whole].chunks_exact(4) {
                                        buffer.push(f32::from_le_bytes([
                                            quad[0], quad[1], quad[2], quad[3],
                                        ]));
                                    }
                                    if buffer.len() > *cap {
                                        let overflow = buffer.len() - *cap;
                                        let aligned = overflow - overflow % 2;
                                        buffer.drain(..aligned);
                                    }
                                }
                            }
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
