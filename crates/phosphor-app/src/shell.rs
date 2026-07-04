// SPDX-License-Identifier: GPL-3.0-or-later
//! The live shell — v3's GTK window replaced by winit + wgpu + egui in
//! one surface. The scope is the app's background: the same render-gpu
//! passes composite straight into the surface view (per-frame readback
//! stays offline-only), egui chrome draws on top in the same encoder.
//!
//! The quiet law lives HERE, not in the engine (v3 law, constants
//! verbatim from phosphor.py):
//! - peak < 1e-4 counts a quiet frame; > 120 quiet frames stop
//!   advancing the phosphor (the picture freezes, the loop idles).
//! - capture off → 90 fade-out frames of empty advance, then the
//!   render loop truly stops (zero CPU, zero GPU).
//! - real signal resumes rendering on the very next poll — no wake
//!   event, the same loop simply starts drawing again.
//! - max_fps cap: skip the frame if it arrived more than 0.5 ms early
//!   (v3's `1/max_fps - 5e-4` slack, verbatim).

use std::sync::mpsc;
use std::time::{Duration, Instant};

use phosphor_audio::{AudioEngine, AudioEvent};
use phosphor_proto::settings::{default_path, Settings};
use phosphor_render_cpu::CpuRenderer;
use phosphor_render_gpu::GpuRenderer;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::render::{build_computer, build_theme};

const QUIET_PEAK_THRESHOLD: f32 = 1e-4;
const QUIET_FRAMES_BEFORE_SLEEP: u32 = 120;
const FADE_OUT_FRAMES: u32 = 90;

pub struct ShellArgs {
    pub fps_log: bool,
    pub exit_after: Option<f64>,
    pub visitor: bool,
    pub mini: bool,
    /// `phosphor song.flac` — open like a file manager would.
    pub play_path: Option<std::path::PathBuf>,
}

pub fn parse_args(arguments: &[String]) -> ShellArgs {
    let mut args = ShellArgs {
        fps_log: false,
        exit_after: None,
        visitor: false,
        mini: false,
        play_path: None,
    };
    let mut iterator = arguments.iter();
    while let Some(argument) = iterator.next() {
        match argument.as_str() {
            "--fps-log" => args.fps_log = true,
            "--exit-after" => {
                args.exit_after = iterator.next().and_then(|s| s.parse().ok());
            }
            "--visitor" => args.visitor = true,
            "--mini" => args.mini = true,
            other if !other.starts_with("--") => {
                let path = std::path::PathBuf::from(other);
                if crate::player::is_audio_path(&path) {
                    args.play_path = Some(path);
                }
            }
            _ => {}
        }
    }
    args
}

struct Graphics {
    window: std::sync::Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    scope_gpu: Option<GpuRenderer>,
    scope_cpu: Option<CpuScope>,
}

struct CpuScope {
    renderer: CpuRenderer,
    texture: Option<egui::TextureHandle>,
}

enum RendererChoice {
    Gpu,
    Cpu,
}

/// Deferred UI intents: chrome pushes, frame() drains with graphics
/// in hand (renderer rebuilds need the device).
pub(crate) enum UiAction {
    CaptureOn,
    CaptureOff,
    TargetPicked(String),
    RefreshTargets,
    ModeChanged,
    SignalTuning,
    RenderTuning,
    RendererChanged,
    ScopeRateChanged,
    SaveSettings,
    SaveSnapshot,
    SaveClip,
    OpenFile,
    PlayPath(std::path::PathBuf),
    PlayerPrevious,
    PlayerNext,
    PlayerTogglePause,
    PlayerVacuumToggled,
    GaplessRequeue,
    ComposeToggle,
    MiniToggle,
    PinToggle,
    FullscreenToggle,
    AlignMini(f32, f32),
    MiniSizePreset(i64),
    /// Route/release the CAPTURED app through the vacuum (the second,
    /// ephemeral ⌀ — never saved; distinct from the file-vacuum).
    VacuumApp(bool),
    KitChanged,
    Quit,
}

pub struct Shell {
    pub(crate) args: ShellArgs,
    pub(crate) settings: Settings,
    pub(crate) engine: AudioEngine,
    audio_events: mpsc::Receiver<AudioEvent>,
    pub(crate) computer: phosphor_dsp::Computer,
    renderer_choice: RendererChoice,

    graphics: Option<Graphics>,
    scope_rect: egui::Rect,
    pub(crate) actions: Vec<UiAction>,
    pub(crate) target_cache: Vec<phosphor_audio::CaptureTarget>,
    pub(crate) settings_panel_open: bool,
    pub(crate) player: crate::player::PlayerState,
    /// paths picked in the threaded native dialog
    file_dialog: Option<mpsc::Receiver<Option<std::path::PathBuf>>>,

    // pass iii state
    pub(crate) konami_progress: usize,
    pub(crate) camera_yaw: f64,
    pub(crate) camera_pitch: f64,
    pub(crate) camera_dolly: f64,
    pub(crate) orbit_last_interaction: Instant,
    pub(crate) composing: bool,
    pub(crate) is_mini: bool,
    pub(crate) is_fullscreen: bool,
    /// (size, position) to restore when leaving mini
    normal_geometry: Option<(winit::dpi::PhysicalSize<u32>,
                             Option<winit::dpi::PhysicalPosition<i32>>)>,
    /// mini magnetism: the 180 ms settle timer after the last move
    mini_settle: Option<Instant>,
    /// stable key of the app currently routed through the vacuum
    pub(crate) app_vacuum: Option<String>,
    ctrl_down: bool,
    quit_requested: bool,
    /// ids of genuinely text-capable widgets that held focus this
    /// frame — the ONLY focus that may eat keyboard shortcuts
    pub(crate) text_focus_ids: std::collections::HashSet<egui::Id>,
    /// pending square-enforcement side after a mini corner resize
    mini_resquare: Option<i64>,
    cursor_position: (f64, f64),
    mini_last_click: Option<Instant>,
    /// the Konami visitor swim (verbatim v3 turtle)
    visitor_started: Option<Instant>,
    exporting: bool,
    export_results: Option<mpsc::Receiver<Result<std::path::PathBuf, String>>>,
    mpris: Option<crate::mpris::MprisHandle>,

    // quiet law state
    quiet_frame_count: u32,
    fade_out_frames_remaining: u32,
    render_loop_active: bool,

    // pacing + receipts
    monitor_hz: f64,
    next_frame_due: Option<Instant>,
    next_frame_anchor: Option<Instant>,
    chrome_dirty: bool,
    last_frame_time: Option<Instant>,
    started: Instant,
    fps_frames: u32,
    fps_window_start: Instant,
    pub last_fps: f64,
    pub(crate) capture_on: bool,
    pub(crate) status_line: String,
}

impl Shell {
    pub fn new(args: ShellArgs) -> Result<Shell, String> {
        let settings = Settings::load(&default_path());

        let (event_sender, audio_events) = mpsc::channel();
        let engine = AudioEngine::spawn(settings.scope_sample_rate,
                                        event_sender)?;
        // Every launch sweeps stale vacuum artifacts — atexit does not
        // survive kill -9 (v3 law).
        let swept = engine.sweep_stale_vacuum();
        if swept > 0 {
            eprintln!("phosphor: swept {swept} stale vacuum sink(s)");
        }

        let computer = build_computer(&settings, settings.scope_sample_rate)
            .map_err(|(_, message)| message)?;
        let renderer_choice = if settings.renderer == "cairo" {
            RendererChoice::Cpu
        } else {
            RendererChoice::Gpu
        };

        Ok(Shell {
            args,
            settings,
            engine,
            audio_events,
            computer,
            renderer_choice,
            graphics: None,
            scope_rect: egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0), egui::vec2(980.0, 640.0)),
            actions: Vec::new(),
            target_cache: Vec::new(),
            settings_panel_open: false,
            player: Default::default(),
            file_dialog: None,
            konami_progress: 0,
            camera_yaw: 0.0,
            camera_pitch: 0.0,
            camera_dolly: 3.2,
            orbit_last_interaction: Instant::now(),
            composing: false,
            is_mini: false,
            is_fullscreen: false,
            normal_geometry: None,
            mini_settle: None,
            app_vacuum: None,
            ctrl_down: false,
            quit_requested: false,
            text_focus_ids: std::collections::HashSet::new(),
            mini_resquare: None,
            cursor_position: (0.0, 0.0),
            mini_last_click: None,
            visitor_started: None,
            exporting: false,
            export_results: None,
            mpris: crate::mpris::spawn(),
            quiet_frame_count: 0,
            fade_out_frames_remaining: FADE_OUT_FRAMES,
            render_loop_active: true,
            monitor_hz: 0.0,
            next_frame_due: None,
            next_frame_anchor: None,
            chrome_dirty: true,
            last_frame_time: None,
            started: Instant::now(),
            fps_frames: 0,
            fps_window_start: Instant::now(),
            last_fps: 0.0,
            capture_on: false,
            status_line: String::new(),
        })
    }

    fn init_graphics(&mut self, event_loop: &ActiveEventLoop) {
        #[allow(unused_imports)]
        use winit::platform::x11::WindowAttributesExtX11;
        let attributes = Window::default_attributes()
            .with_title("Phosphor")
            // WM_CLASS "phosphor" — the .desktop StartupWMClass match
            // (v3 did this via GLib.set_prgname)
            .with_name("phosphor", "phosphor")
            .with_transparent(true)
            .with_inner_size(winit::dpi::LogicalSize::new(
                self.settings.window_width.max(320) as f64,
                self.settings.window_height.max(240) as f64,
            ));
        let window = std::sync::Arc::new(
            event_loop.create_window(attributes).expect("window"));

        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.clone())
            .expect("surface");
        let adapter = pollster::block_on(instance.request_adapter(
            &wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                ..Default::default()
            })).expect("adapter");
        let (device, queue) = pollster::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("device");

        let capabilities = surface.get_capabilities(&adapter);
        // Mailbox-first (the spike's ~1000 fps receipt), then whatever
        // the surface offers. PreMultiplied-first for glass.
        let present_mode = if capabilities.present_modes
            .contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else if capabilities.present_modes
            .contains(&wgpu::PresentMode::Immediate) {
            wgpu::PresentMode::Immediate
        } else {
            wgpu::PresentMode::Fifo
        };
        let alpha_mode = [wgpu::CompositeAlphaMode::PreMultiplied,
                          wgpu::CompositeAlphaMode::PostMultiplied,
                          wgpu::CompositeAlphaMode::Inherit]
            .into_iter()
            .find(|mode| capabilities.alpha_modes.contains(mode))
            .unwrap_or(wgpu::CompositeAlphaMode::Opaque);

        let size = window.inner_size();
        // Prefer a NON-sRGB surface: the composite shader encodes its
        // own gamma; formats[0] on RADV is typically Bgra8UnormSrgb
        // and double-encoding washed the live beam (wave-2.5 root #3).
        let surface_format = capabilities
            .formats
            .iter()
            .copied()
            .find(|format| !format.is_srgb())
            .unwrap_or(capabilities.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let egui_ctx = egui::Context::default();
        let egui_state = egui_winit::State::new(
            egui_ctx.clone(), egui::ViewportId::ROOT, &window, None,
            None, None);
        let egui_renderer = egui_wgpu::Renderer::new(
            &device, config.format,
            egui_wgpu::RendererOptions::default());

        let scope_gpu = match self.renderer_choice {
            RendererChoice::Gpu => Some(
                GpuRenderer::new_for_surface(
                    &adapter, device.clone(), queue.clone(),
                    config.width, config.height,
                    self.settings.gl_supersample, config.format)
                    .expect("scope renderer")),
            RendererChoice::Cpu => None,
        };
        let scope_cpu = match self.renderer_choice {
            RendererChoice::Cpu => Some(CpuScope {
                renderer: CpuRenderer::new(config.width as usize,
                                           config.height as usize, 1),
                texture: None,
            }),
            RendererChoice::Gpu => None,
        };

        let mut graphics = Graphics {
            window, surface, device, queue, config, egui_ctx,
            egui_state, egui_renderer, scope_gpu, scope_cpu,
        };
        self.apply_render_settings(&mut graphics);
        self.graphics = Some(graphics);

        // v3 starts scoping on launch: saved target, else the default
        // monitor. A file argument opens like the file dialog would.
        self.refresh_target_cache();
        if let Some(path) = self.args.play_path.take() {
            self.play_file(&path, true);
        } else {
            self.start_capture_from_settings();
        }
        if self.args.visitor {
            self.begin_visitor(); // you know why
        }
        if self.args.mini || self.settings.start_in_mini {
            self.actions.push(UiAction::MiniToggle);
        }
    }

    /// Drain chrome intents (frame() calls this with graphics free).
    fn drain_actions(&mut self, graphics: &mut Graphics) {
        let actions = std::mem::take(&mut self.actions);
        for action in actions {
            match action {
                UiAction::CaptureOn => {
                    self.start_capture_from_settings();
                }
                UiAction::CaptureOff => {
                    self.engine.stop_capture();
                    self.capture_on = false;
                    self.status_line = "idle".into();
                }
                UiAction::TargetPicked(combo_id) => {
                    // switching away restores the sound (v3 law)
                    self.engine.vacuum_release();
                    if self.engine.is_playing_file() {
                        self.engine.stop_playback();
                    }
                    if self.capture_on || self.engine.is_playing_file() {
                        if self.engine.start_capture(&combo_id) {
                            self.capture_on = true;
                            self.wake_render_loop();
                            self.status_line =
                                format!("scoping {combo_id}");
                        } else {
                            self.capture_on = false;
                            self.status_line =
                                format!("capture failed: {combo_id}");
                        }
                    }
                }
                UiAction::RefreshTargets => {
                    self.refresh_target_cache();
                }
                UiAction::KitChanged => {
                    // rebuild the computer so the kit chain re-applies
                    // (state zeroed on configure — the kit parity law)
                    let rate = self.settings.scope_sample_rate;
                    if let Ok(computer) =
                        crate::render::build_computer(&self.settings, rate)
                    {
                        self.computer = computer;
                        self.push_camera();
                    }
                    self.settings.save(&default_path()).ok();
                }
                UiAction::ModeChanged => {
                    if let Ok(mode) = self.settings.display_mode
                        .parse::<phosphor_dsp::Mode>()
                    {
                        self.computer.mode = mode;
                    }
                    self.settings.save(&default_path()).ok();
                }
                UiAction::SignalTuning => {
                    self.computer.gain = self.settings.gain;
                    self.computer.beam_energy = self.settings.beam_energy;
                }
                UiAction::RenderTuning => {
                    self.apply_render_settings(graphics);
                }
                UiAction::RendererChanged => {
                    self.rebuild_scope_renderers(graphics);
                }
                UiAction::ScopeRateChanged => {
                    let rate = self.settings.scope_sample_rate;
                    self.engine.configure_sample_rate(rate);
                    self.computer.set_sample_rate(rate, 1);
                    if self.capture_on {
                        // v3: rate takes effect by restarting the stream
                        self.start_capture_from_settings();
                    }
                }
                UiAction::SaveSettings => {
                    self.settings.save(&default_path()).ok();
                }
                action @ (UiAction::SaveSnapshot | UiAction::SaveClip) => {
                    if self.exporting {
                        self.status_line = "export already running".into();
                        continue;
                    }
                    let snapshot = matches!(action, UiAction::SaveSnapshot);
                    let seconds = if snapshot { 1.5 } else { 10.0 };
                    let history = self.engine.copy_history(seconds);
                    let settings = self.settings.clone();
                    let rate = self.settings.scope_sample_rate;
                    let (sender, receiver) = mpsc::channel();
                    self.export_results = Some(receiver);
                    self.exporting = true;
                    self.status_line = if snapshot {
                        "rendering snapshot…".into()
                    } else {
                        "rendering clip…".into()
                    };
                    std::thread::spawn(move || {
                        let result = if snapshot {
                            crate::exports::save_snapshot(
                                history, settings, rate)
                        } else {
                            crate::exports::save_clip(
                                history, settings, rate)
                        };
                        let _ = sender.send(result);
                    });
                }
                UiAction::OpenFile => {
                    if self.file_dialog.is_none() {
                        let (sender, receiver) = mpsc::channel();
                        self.file_dialog = Some(receiver);
                        std::thread::spawn(move || {
                            let picked = rfd::FileDialog::new()
                                .set_title("Play audio file")
                                .add_filter(
                                    "Audio files",
                                    &["mp3", "flac", "ogg", "oga",
                                      "opus", "wav", "m4a", "aac",
                                      "wma", "aif", "aiff", "mka",
                                      "phos"])
                                .add_filter("All files", &["*"])
                                .pick_file();
                            let _ = sender.send(picked);
                        });
                    }
                }
                UiAction::PlayPath(path) => {
                    self.play_file(&path, false);
                }
                UiAction::PlayerPrevious => self.step_playlist(-1),
                UiAction::PlayerNext => self.step_playlist(1),
                UiAction::PlayerTogglePause => {
                    let paused = !self.player.paused;
                    self.engine.set_playback_paused(paused);
                    self.player.paused = paused;
                    self.set_mpris_status(
                        if paused { "Paused" } else { "Playing" });
                    if !paused {
                        self.wake_render_loop();
                    }
                }
                UiAction::PlayerVacuumToggled => {
                    // reopen the pipeline seek-style at the current
                    // position, with/without the audible leg (v3 ⌀)
                    if let Some(path) = self.player.playing.clone() {
                        let position =
                            self.engine.playback_position_seconds();
                        let was_paused = self.player.paused;
                        self.engine.start_file(
                            &path, position, false,
                            self.settings.vacuum_enabled);
                        if was_paused {
                            self.engine.set_playback_paused(true);
                        }
                        self.queue_gapless_next();
                    }
                }
                UiAction::GaplessRequeue => self.queue_gapless_next(),
                UiAction::ComposeToggle => {
                    self.status_line =
                        "compose mode lands in chrome pass iv".into();
                }
                UiAction::MiniToggle => {
                    let enable = !self.is_mini;
                    self.set_mini_mode(enable, graphics);
                }
                UiAction::PinToggle => {
                    self.settings.pinned = !self.settings.pinned;
                    self.apply_window_level(graphics);
                }
                UiAction::FullscreenToggle => {
                    self.is_fullscreen = !self.is_fullscreen;
                    graphics.window.set_fullscreen(
                        if self.is_fullscreen {
                            Some(winit::window::Fullscreen::Borderless(None))
                        } else {
                            None
                        });
                }
                UiAction::AlignMini(fraction_x, fraction_y) => {
                    self.align_mini(graphics, fraction_x, fraction_y);
                }
                UiAction::MiniSizePreset(size) => {
                    self.settings.mini_size = size.clamp(140, 1000);
                    if self.is_mini {
                        let _ = graphics.window.request_inner_size(
                            winit::dpi::PhysicalSize::new(
                                self.settings.mini_size as u32,
                                self.settings.mini_size as u32));
                    }
                }
                UiAction::Quit => {
                    self.quit_requested = true;
                }
                UiAction::VacuumApp(enable) => {
                    if enable {
                        let key = self.settings.target_id.clone()
                            .and_then(|id| id.strip_prefix("app:")
                                      .map(str::to_string));
                        if let Some(key) = key {
                            match self.engine.vacuum_route_app(&key) {
                                Ok(monitor_id) => {
                                    self.engine.start_capture(&monitor_id);
                                    self.capture_on = true;
                                    self.app_vacuum = Some(key);
                                    self.wake_render_loop();
                                    self.status_line =
                                        "⌀ scoping the void".into();
                                }
                                Err(error) => {
                                    self.status_line =
                                        format!("vacuum: {error}");
                                }
                            }
                        }
                    } else if self.app_vacuum.take().is_some() {
                        // restore is sacred: stream home, then back to
                        // scoping the app itself
                        self.engine.vacuum_release();
                        if let Some(id) = self.settings.target_id.clone()
                            && self.engine.start_capture(&id)
                        {
                            self.capture_on = true;
                        }
                        self.status_line = "vacuum released".into();
                    }
                }
            }
        }

        // threaded file-dialog result
        if let Some(receiver) = &self.file_dialog
            && let Ok(result) = receiver.try_recv()
        {
            self.file_dialog = None;
            if let Some(path) = result {
                self.play_file(&path, true);
            }
        }
        self.service_seek_debounce();
    }

    /// Drain MPRIS commands and keep the shared state fresh.
    fn service_mpris(&mut self) {
        let Some(mpris) = &self.mpris else { return };
        // live position + step capability
        let playing = self.player.playing.is_some();
        mpris.shared.position_micros.store(
            (self.engine.playback_position_seconds() * 1e6) as i64,
            std::sync::atomic::Ordering::Relaxed);
        mpris.shared.can_step.store(
            self.player.playlist.len() > 1,
            std::sync::atomic::Ordering::Relaxed);
        let mut commands = Vec::new();
        while let Ok(command) = mpris.commands.try_recv() {
            commands.push(command);
        }
        for command in commands {
            use crate::mpris::MprisCommand;
            match command {
                MprisCommand::Next => {
                    if playing { self.actions.push(UiAction::PlayerNext); }
                }
                MprisCommand::Previous => {
                    if playing {
                        self.actions.push(UiAction::PlayerPrevious);
                    }
                }
                MprisCommand::PlayPause => {
                    if playing {
                        self.actions.push(UiAction::PlayerTogglePause);
                    }
                }
                MprisCommand::Play => {
                    if playing && self.player.paused {
                        self.actions.push(UiAction::PlayerTogglePause);
                    }
                }
                MprisCommand::Pause => {
                    if playing && !self.player.paused {
                        self.actions.push(UiAction::PlayerTogglePause);
                    }
                }
                MprisCommand::Stop => {
                    // v4 fix: Stop actually stops (v3 aliased Pause)
                    self.engine.stop_playback();
                    self.player.playing = None;
                    self.player.duration = None;
                    self.set_mpris_status("Stopped");
                }
                MprisCommand::SeekRelative(offset_micros) => {
                    let target = (self.engine.playback_position_seconds()
                                  + offset_micros as f64 / 1e6).max(0.0);
                    self.player.seek_debounce =
                        Some((target, Instant::now()
                              - Duration::from_millis(250)));
                }
                MprisCommand::SetPosition(position_micros) => {
                    let target = (position_micros as f64 / 1e6).max(0.0);
                    self.player.seek_debounce =
                        Some((target, Instant::now()
                              - Duration::from_millis(250)));
                }
                MprisCommand::OpenUri(uri) => {
                    let path = uri.strip_prefix("file://")
                        .unwrap_or(&uri).to_string();
                    let path = std::path::PathBuf::from(path);
                    if crate::player::is_audio_path(&path) && path.exists() {
                        self.actions.push(UiAction::PlayPath(path));
                    }
                }
                MprisCommand::Raise => {
                    if let Some(graphics) = &self.graphics {
                        graphics.window.focus_window();
                    }
                }
                MprisCommand::SetVolume(volume) => {
                    self.settings.playback_volume = volume as f32;
                    self.engine.set_volume(
                        crate::player::cubic_volume(volume as f32));
                }
            }
        }
    }

    pub(crate) fn set_mpris_status(&self, status: &'static str) {
        if let Some(mpris) = &self.mpris {
            *mpris.shared.status.lock().unwrap() = status;
            let _ = mpris.notify.send(
                crate::mpris::MprisNotify::StatusChanged);
        }
    }

    pub(crate) fn mpris_track_changed(&self) {
        let Some(mpris) = &self.mpris else { return };
        let metadata = self.engine.current_track_metadata()
            .unwrap_or_default();
        *mpris.shared.track.lock().unwrap() = crate::mpris::MprisTrack {
            path: self.player.playing.clone(),
            title: metadata.title,
            artist: metadata.artist,
            album: metadata.album,
            duration_micros: metadata.duration.map(|d| (d * 1e6) as i64),
        };
        *mpris.shared.status.lock().unwrap() =
            if self.player.playing.is_some() { "Playing" } else { "Stopped" };
        let _ = mpris.notify.send(crate::mpris::MprisNotify::TrackChanged);
    }

    pub(crate) fn mpris_seeked(&self, seconds: f64) {
        if let Some(mpris) = &self.mpris {
            let _ = mpris.notify.send(
                crate::mpris::MprisNotify::Seeked((seconds * 1e6) as i64));
        }
    }

    pub(crate) fn begin_visitor(&mut self) {
        // you know the code
        self.visitor_started = Some(Instant::now());
        self.fade_out_frames_remaining = self.fade_out_frames_remaining
            .max((crate::exports::VISITOR_SWIM_SECONDS * 240.0) as u32);
        self.wake_render_loop();
    }

    pub(crate) fn push_camera(&mut self) {
        self.computer.set_camera(Some(self.camera_yaw),
                                 Some(self.camera_pitch),
                                 Some(self.camera_dolly));
    }

    pub(crate) fn mark_orbit_interaction(&mut self) {
        self.orbit_last_interaction = Instant::now();
    }

    fn apply_window_level(&self, graphics: &Graphics) {
        use winit::window::WindowLevel;
        let level = if self.settings.pinned || self.is_mini {
            WindowLevel::AlwaysOnTop
        } else {
            WindowLevel::Normal
        };
        graphics.window.set_window_level(level);
    }

    /// v3 set_mini_mode: square, undecorated, kept above; leaving
    /// restores geometry + decorations (§6.3).
    fn set_mini_mode(&mut self, enable: bool, graphics: &mut Graphics) {
        if enable == self.is_mini {
            return;
        }
        self.is_mini = enable;
        let window = &graphics.window;
        if enable {
            self.normal_geometry = Some((
                window.inner_size(),
                window.outer_position().ok(),
            ));
            window.set_decorations(false);
            let size = self.settings.mini_size.clamp(140, 1000) as u32;
            let _ = window.request_inner_size(
                winit::dpi::PhysicalSize::new(size, size));
            if let (Some(x), Some(y)) =
                (self.settings.mini_x, self.settings.mini_y)
            {
                window.set_outer_position(
                    winit::dpi::PhysicalPosition::new(x as i32, y as i32));
            }
        } else {
            // remember where the mini lived (v3 persists on leave)
            if let Ok(position) = window.outer_position() {
                self.settings.mini_x = Some(position.x as i64);
                self.settings.mini_y = Some(position.y as i64);
            }
            window.set_decorations(true);
            if let Some((size, position)) = self.normal_geometry.take() {
                let _ = window.request_inner_size(size);
                if let Some(position) = position {
                    window.set_outer_position(position);
                }
            }
        }
        self.apply_window_level(graphics);
        self.chrome_dirty = true;
    }

    /// The monitor's work area (panels excluded) via _NET_WORKAREA —
    /// v3 used GTK's; a one-shot xprop at snap time is the X11-native
    /// equivalent without a new dependency.
    fn workarea() -> Option<(i32, i32, i32, i32)> {
        let output = std::process::Command::new("xprop")
            .args(["-root", "_NET_WORKAREA"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        let numbers: Vec<i32> = text
            .split('=')
            .nth(1)?
            .split(',')
            .filter_map(|part| part.trim().parse().ok())
            .collect();
        if numbers.len() >= 4 {
            Some((numbers[0], numbers[1], numbers[2], numbers[3]))
        } else {
            None
        }
    }

    /// v3 _snap_mini_to_edges: within 32 px of a work-area edge →
    /// flush to it; position persisted.
    fn snap_mini_to_edges(&mut self, graphics: &Graphics) {
        const SNAP: i32 = 32;
        let Some((area_x, area_y, area_w, area_h)) = Self::workarea()
        else { return };
        let window = &graphics.window;
        let Ok(position) = window.outer_position() else { return };
        let size = window.outer_size();
        let (mut x, mut y) = (position.x, position.y);
        if (x - area_x).abs() <= SNAP {
            x = area_x;
        } else if ((area_x + area_w) - (x + size.width as i32)).abs() <= SNAP {
            x = area_x + area_w - size.width as i32;
        }
        if (y - area_y).abs() <= SNAP {
            y = area_y;
        } else if ((area_y + area_h) - (y + size.height as i32)).abs() <= SNAP {
            y = area_y + area_h - size.height as i32;
        }
        if (x, y) != (position.x, position.y) {
            window.set_outer_position(
                winit::dpi::PhysicalPosition::new(x, y));
        }
        self.settings.mini_x = Some(x as i64);
        self.settings.mini_y = Some(y as i64);
    }

    /// v3 _align_mini: fraction of the work area, position persisted.
    fn align_mini(&mut self, graphics: &Graphics, fraction_x: f32,
                  fraction_y: f32) {
        let Some((area_x, area_y, area_w, area_h)) = Self::workarea()
        else { return };
        let size = graphics.window.outer_size();
        let x = area_x
            + (fraction_x * (area_w - size.width as i32) as f32) as i32;
        let y = area_y
            + (fraction_y * (area_h - size.height as i32) as f32) as i32;
        graphics.window.set_outer_position(
            winit::dpi::PhysicalPosition::new(x, y));
        self.settings.mini_x = Some(x as i64);
        self.settings.mini_y = Some(y as i64);
    }

    /// Renderer or quality changed: rebuild the scope sinks in place.
    fn rebuild_scope_renderers(&mut self, graphics: &mut Graphics) {
        self.renderer_choice = if self.settings.renderer == "cairo" {
            RendererChoice::Cpu
        } else {
            RendererChoice::Gpu
        };
        let (width, height) = graphics
            .scope_gpu
            .as_ref()
            .map(|g| g.size())
            .or_else(|| graphics.scope_cpu.as_ref().map(|c| {
                (c.renderer.width() as u32, c.renderer.height() as u32)
            }))
            .unwrap_or((graphics.config.width, graphics.config.height));
        graphics.scope_gpu = None;
        graphics.scope_cpu = None;
        match self.renderer_choice {
            RendererChoice::Gpu => {
                let instance = wgpu::Instance::default();
                let surface_format = graphics.config.format;
                if let Ok(adapter) = pollster::block_on(
                    instance.request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference:
                            wgpu::PowerPreference::HighPerformance,
                        compatible_surface: None,
                        ..Default::default()
                    }))
                {
                    match GpuRenderer::new_for_surface(
                        &adapter, graphics.device.clone(),
                        graphics.queue.clone(), width, height,
                        self.settings.gl_supersample, surface_format)
                    {
                        Ok(renderer) => {
                            graphics.scope_gpu = Some(renderer)
                        }
                        Err(error) => eprintln!(
                            "phosphor: gpu renderer: {error}"),
                    }
                }
            }
            RendererChoice::Cpu => {
                graphics.scope_cpu = Some(CpuScope {
                    renderer: CpuRenderer::new(
                        width as usize, height as usize, 1),
                    texture: None,
                });
            }
        }
        self.apply_render_settings(graphics);
    }

    fn apply_render_settings(&self, graphics: &mut Graphics) {
        let theme = build_theme(&self.settings);
        let grid_fraction =
            phosphor_beam::grid_spacing_fraction(self.settings.gain);
        if let Some(gpu) = graphics.scope_gpu.as_mut() {
            gpu.beam_focus = self.settings.beam_focus;
            gpu.persistence = self.settings.persistence;
            gpu.theme = theme;
            gpu.grid_enabled = self.settings.grid_enabled;
            gpu.grid_spacing_fraction = grid_fraction;
        }
        if let Some(cpu) = graphics.scope_cpu.as_mut() {
            cpu.renderer.beam_focus = self.settings.beam_focus;
            cpu.renderer.persistence = self.settings.persistence;
            cpu.renderer.theme = theme;
            cpu.renderer.grid_enabled = self.settings.grid_enabled;
            cpu.renderer.grid_spacing_fraction = grid_fraction;
        }
    }

    fn start_capture_from_settings(&mut self) {
        let target = self
            .settings
            .target_id
            .clone()
            .or_else(|| self.engine.default_monitor_target_id());
        if let Some(combo_id) = target
            && self.engine.start_capture(&combo_id)
        {
            self.capture_on = true;
            self.wake_render_loop();
            self.status_line = format!("scoping {combo_id}");
        } else {
            self.status_line = "no capture target".into();
        }
    }

    pub(crate) fn wake_render_loop(&mut self) {
        self.quiet_frame_count = 0;
        self.fade_out_frames_remaining = FADE_OUT_FRAMES;
        self.next_frame_due = None;
        if !self.render_loop_active {
            self.render_loop_active = true;
        }
        if let Some(graphics) = &self.graphics {
            graphics.window.request_redraw();
        }
    }

    /// The frame cadence: -1 = genuinely uncapped (new in v4), 0 =
    /// the monitor's refresh (v3's meaning), else the user's cap.
    /// Returns 0.0 for "no deadline pacing".
    fn cap_hz(&self) -> f64 {
        if self.settings.max_fps < 0 {
            0.0
        } else if self.settings.max_fps > 0 {
            self.settings.max_fps as f64
        } else {
            self.monitor_hz
        }
    }

    /// One tick of the loop (the v3 tick callback's shape: drain,
    /// quiet-check, maybe render). Returns whether to keep ticking.
    fn redraw(&mut self) -> bool {
        let now = Instant::now();
        self.last_frame_time = Some(now);

        // v3's "0 = uncapped/monitor": pace at the panel's refresh.
        // current_monitor() answers only once the window is mapped,
        // so probe lazily here (Mailbox shows the newest frame; we
        // just don't cook 3,400 of them for a 165 Hz panel —
        // measured before this cap).
        if self.monitor_hz == 0.0
            && let Some(graphics) = &self.graphics
        {
            let refresh = graphics.window.current_monitor()
                .or_else(|| graphics.window.primary_monitor())
                .and_then(|monitor| monitor.refresh_rate_millihertz());
            if let Some(millihertz) = refresh {
                self.monitor_hz = millihertz as f64 / 1000.0;
                eprintln!("phosphor: pacing at {:.1} Hz (monitor)",
                          self.monitor_hz);
            }
        }

        // ---- audio events ----
        let mut targets_dirty = false;
        while let Ok(event) = self.audio_events.try_recv() {
            match event {
                AudioEvent::StreamEnded => {
                    self.capture_on = false;
                    self.status_line = "stream ended".into();
                }
                AudioEvent::TargetsChanged
                | AudioEvent::DefaultSinkChanged => {
                    targets_dirty = true;
                }
                AudioEvent::PlaybackEnded => {
                    self.handle_track_finished();
                }
                AudioEvent::TrackStarted { path } => {
                    // gapless splice moved us forward: sync the index
                    if let Some(index) = self.player.playlist
                        .iter().position(|p| p == &path)
                    {
                        self.player.playlist_index = index;
                    }
                    self.player.playing = Some(path.clone());
                    let metadata = self.engine.current_track_metadata()
                        .unwrap_or_default();
                    self.player.duration = metadata.duration;
                    let vacuum_mark = if self.settings.vacuum_enabled {
                        "⌀ "
                    } else {
                        ""
                    };
                    let basename = path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    self.status_line = format!("▶ {vacuum_mark}{basename}");
                    if self.settings.show_now_playing {
                        let title = metadata.title.clone()
                            .unwrap_or_else(|| basename.clone());
                        // .phos: subtitle is "trace by <credit>" —
                        // the postcard credit fade (v3 law). Certain
                        // artists get a nod (undocumented, v3 table).
                        let mut subtitle = metadata.artist.clone();
                        let nod = match metadata.artist.as_deref()
                            .map(|a| a.trim().to_lowercase())
                            .as_deref()
                        {
                            Some("jerobeam fenderson") =>
                                Some("🍄 the real deal"),
                            Some("brakence") =>
                                Some("🫧 there are hidden pictures in here"),
                            _ => None,
                        };
                        if let Some(nod) = nod {
                            subtitle = Some(match subtitle {
                                Some(s) => format!("{s}  ·  {nod}"),
                                None => nod.to_string(),
                            });
                        }
                        self.player.flash_now_playing(
                            &title, subtitle.as_deref());
                    }
                    self.queue_gapless_next();
                    self.mpris_track_changed();
                    self.chrome_dirty = true;
                }
            }
        }
        if targets_dirty {
            self.refresh_target_cache();
            self.chrome_dirty = true;
        }

        // ---- MPRIS: media keys arrive as Player method calls ----
        self.service_mpris();

        // ---- export results ----
        if let Some(receiver) = &self.export_results
            && let Ok(result) = receiver.try_recv()
        {
            self.export_results = None;
            self.exporting = false;
            self.status_line = match result {
                Ok(path) => format!("saved {}", path.display()),
                Err(error) => error,
            };
            self.chrome_dirty = true;
        }

        // ---- 3D idle drift (§8: 6 s hands-off, 0.05 rad/s yaw) ----
        let is_3d = matches!(self.settings.display_mode.as_str(),
                             "xyz_takens" | "helix");
        if is_3d
            && self.orbit_last_interaction.elapsed().as_secs_f64() > 6.0
        {
            self.camera_yaw += 0.05 / self.cap_hz().max(30.0);
            self.push_camera();
        }

        // ---- samples + quiet law (visitor overrides the sleep) ----
        let samples = self.engine.take_stereo_samples();
        let visitor_active = self
            .visitor_started
            .map(|t| t.elapsed().as_secs_f64()
                 <= crate::exports::VISITOR_SWIM_SECONDS)
            .unwrap_or(false);
        if !visitor_active {
            self.visitor_started = None;
        }
        let advancing = if self.capture_on || self.engine.is_playing_file() {
            let peak = samples
                .iter()
                .fold(0.0f32, |peak, s| peak.max(s.abs()));
            let is_quiet = peak < QUIET_PEAK_THRESHOLD;
            self.quiet_frame_count =
                if is_quiet { self.quiet_frame_count + 1 } else { 0 };
            self.quiet_frame_count <= QUIET_FRAMES_BEFORE_SLEEP
        } else if self.fade_out_frames_remaining > 0 {
            // capture off: fade the glow, then truly stop (zero CPU)
            self.fade_out_frames_remaining -= 1;
            true
        } else {
            self.render_loop_active = false;
            false
        } || visitor_active;

        let Some(mut graphics) = self.graphics.take() else {
            return false;
        };
        let keep_going = self.frame(&mut graphics, &samples, advancing);
        // actions drain at TICK level: while quiet-asleep the frame
        // early-outs, but MPRIS media keys must still act (found live:
        // Next while paused sat queued forever)
        self.drain_actions(&mut graphics);
        self.graphics = Some(graphics);

        // ---- fps receipt (per TICK, like v3's counter: it keeps
        // counting while quiet — §2.3 semantics) ----
        self.fps_frames += 1;
        let window_elapsed = self.fps_window_start.elapsed();
        if window_elapsed.as_secs_f64() >= 1.0 {
            self.last_fps =
                self.fps_frames as f64 / window_elapsed.as_secs_f64();
            if self.args.fps_log {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "t": (self.started.elapsed().as_secs_f64()
                              * 10.0).round() / 10.0,
                        "fps": (self.last_fps * 10.0).round() / 10.0,
                        "quiet": self.quiet_frame_count
                            > QUIET_FRAMES_BEFORE_SLEEP,
                        "active": self.render_loop_active,
                    })
                );
            }
            self.fps_frames = 0;
            self.fps_window_start = Instant::now();
        }
        keep_going
    }

    fn frame(&mut self, graphics: &mut Graphics, samples: &[f32],
             advancing: bool) -> bool {
        // Asleep and nothing changed on screen: zero GPU (the picture
        // stays frozen at its last-drawn state — v3's quiet law).
        if !advancing && !self.chrome_dirty {
            return true;
        }
        self.chrome_dirty = false;
        let frame = match graphics.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(_) => {
                graphics.surface.configure(&graphics.device,
                                           &graphics.config);
                return true;
            }
        };
        let view = frame.texture.create_view(&Default::default());
        let pixels_per_point = graphics.window.scale_factor() as f32;

        // ---- scope advance (physical pixels of last frame's rect) ----
        let scope_physical = egui::Rect::from_min_max(
            self.scope_rect.min * pixels_per_point,
            self.scope_rect.max * pixels_per_point,
        );
        let scope_width = scope_physical.width().max(1.0) as u32;
        let scope_height = scope_physical.height().max(1.0) as u32;

        // CPU path traces at its reduced resolution (v3 law); GPU at full.
        let (trace_w, trace_h) = if graphics.scope_cpu.is_some() {
            let fraction = self.settings.cairo_resolution.clamp(0.25, 1.0);
            (((scope_width as f32 * fraction) as u32).max(64),
             ((scope_height as f32 * fraction) as u32).max(64))
        } else {
            (scope_width, scope_height)
        };
        if advancing {
            let mut segments: Vec<[f32; 5]> = self.computer.compute(
                samples, trace_w as f32, trace_h as f32)
                .to_vec();
            // the visitor swims OVER whatever the audio draws
            if let Some(started) = self.visitor_started {
                segments.extend(crate::exports::visitor_segments(
                    started.elapsed().as_secs_f64(),
                    trace_w as f32, trace_h as f32));
            }
            let segments = &segments[..];
            if let Some(gpu) = graphics.scope_gpu.as_mut() {
                if let Err(error) = gpu.resize(scope_width, scope_height) {
                    eprintln!("phosphor: scope resize: {error}");
                }
                gpu.advance(segments);
            }
            if let Some(cpu) = graphics.scope_cpu.as_mut() {
                // v3's CPU resolution law: render at scope x fraction,
                // the egui image upscales (0.75/0.5 presets — this was
                // silently ignored in wave 2, all CPU frames full-res)
                let fraction =
                    self.settings.cairo_resolution.clamp(0.25, 1.0);
                let target_w = ((scope_width as f32 * fraction) as usize)
                    .max(64);
                let target_h = ((scope_height as f32 * fraction) as usize)
                    .max(64);
                let (w, h) = (cpu.renderer.width(), cpu.renderer.height());
                if w != target_w || h != target_h {
                    let mut renderer = CpuRenderer::new(
                        target_w, target_h, 1);
                    renderer.beam_focus = cpu.renderer.beam_focus;
                    renderer.persistence = cpu.renderer.persistence;
                    renderer.theme = cpu.renderer.theme;
                    renderer.grid_enabled = cpu.renderer.grid_enabled;
                    renderer.grid_spacing_fraction =
                        cpu.renderer.grid_spacing_fraction;
                    cpu.renderer = renderer;
                }
                cpu.renderer.advance(segments);
            }
        }

        // CPU path: upload the composite as an egui texture
        if let Some(cpu) = graphics.scope_cpu.as_mut() {
            let size = [cpu.renderer.width(), cpu.renderer.height()];
            let pixels = cpu.renderer.composite();
            let image = egui::ColorImage::from_rgba_unmultiplied(
                size, pixels);
            match cpu.texture.as_mut() {
                Some(texture) => texture.set(image, Default::default()),
                None => {
                    cpu.texture = Some(graphics.egui_ctx.load_texture(
                        "scope-cpu", image, Default::default()));
                }
            }
        }

        // ---- egui chrome ----
        let raw_input = graphics.egui_state
            .take_egui_input(&graphics.window);
        let cpu_texture_id =
            graphics.scope_cpu.as_ref()
                .and_then(|c| c.texture.as_ref().map(|t| t.id()));
        let mut scope_rect_out = self.scope_rect;
        let egui_ctx = graphics.egui_ctx.clone();
        self.player.tick_overlay();
        self.text_focus_ids.clear(); // chrome re-registers each frame
        let hide_chrome = self.is_mini || self.is_fullscreen;
        let full_output = egui_ctx.run(raw_input, |ctx| {
            self.apply_ui_style(ctx);
            if !hide_chrome {
                egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
                    self.ui_toolbar(ui);
                    self.ui_sliders(ui);
                    self.ui_transport(ui);
                });
                self.ui_settings_panel(ctx);
                self.ui_playlist_panel(ctx);
            }
            let fps = self.last_fps;
            let show_fps = self.settings.show_fps;
            if !hide_chrome {
                egui::TopBottomPanel::bottom("statusbar").show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(self.status_line.as_str());
                        if show_fps {
                            ui.with_layout(
                                egui::Layout::right_to_left(
                                    egui::Align::Center),
                                |ui| { ui.label(format!("{fps:.0} fps")); });
                        }
                    });
                });
            }
            let central = egui::CentralPanel::default()
                .frame(egui::Frame::NONE);
            central.show(ctx, |ui| {
                scope_rect_out = ui.max_rect();
                let scope_response = ui.interact(
                    scope_rect_out, ui.id().with("scope"),
                    egui::Sense::click_and_drag());
                self.ui_context_menu(&scope_response);
                if scope_response.double_clicked() && self.is_mini {
                    self.actions.push(UiAction::MiniToggle);
                }
                // drag-to-orbit (3D, desktop only — mini blocks drag,
                // the v3 asymmetry; wheel-dolly still works in mini)
                let is_3d = matches!(
                    self.settings.display_mode.as_str(),
                    "xyz_takens" | "helix");
                if is_3d && !self.is_mini && scope_response.dragged() {
                    let delta = scope_response.drag_delta();
                    self.camera_yaw += delta.x as f64 * 0.008;
                    self.camera_pitch = (self.camera_pitch
                        + delta.y as f64 * 0.008).clamp(-1.45, 1.45);
                    self.push_camera();
                    self.mark_orbit_interaction();
                }
                if let Some(texture_id) = cpu_texture_id {
                    ui.painter().image(
                        texture_id, scope_rect_out,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0),
                                                 egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE);
                }
                // now-playing overlay: top-left, 12 px margins (v3 §9)
                if let Some((title, subtitle, opacity)) =
                    self.player.overlay_visible()
                {
                    let alpha = (opacity * 255.0) as u8;
                    let position = scope_rect_out.min
                        + egui::vec2(12.0, 12.0);
                    let painter = ui.painter();
                    let title_id = painter.text(
                        position, egui::Align2::LEFT_TOP,
                        title,
                        egui::FontId::proportional(16.0),
                        egui::Color32::from_white_alpha(alpha));
                    if let Some(subtitle) = subtitle {
                        painter.text(
                            egui::pos2(position.x,
                                       title_id.max.y + 2.0),
                            egui::Align2::LEFT_TOP,
                            subtitle,
                            egui::FontId::proportional(12.0),
                            egui::Color32::from_white_alpha(
                                (alpha as f32 * 0.8) as u8));
                    }
                    ui.ctx().request_repaint();
                }
            });
        });
        self.scope_rect = scope_rect_out;

        // Focus hygiene: only registered text widgets may HOLD focus —
        // egui buttons keep focus after a click, and gating shortcuts
        // on that killed them all (the wave-2.5 root fix).
        egui_ctx.memory_mut(|memory| {
            if let Some(id) = memory.focused()
                && !self.text_focus_ids.contains(&id)
            {
                memory.surrender_focus(id);
            }
        });
        // Honor egui's requested follow-up paints (press/release
        // visuals, hover fades) — without this, chrome only repainted
        // on new input while the scope was quiet-asleep ("laggy").
        let repaint_delay = full_output
            .viewport_output
            .values()
            .map(|viewport| viewport.repaint_delay)
            .min()
            .unwrap_or(Duration::MAX);
        if repaint_delay < Duration::from_secs(1) {
            self.chrome_dirty = true;
            let due = Instant::now() + repaint_delay;
            self.next_frame_due =
                Some(self.next_frame_due.map_or(due, |d| d.min(due)));
        }

        let clipped = graphics.egui_ctx.tessellate(
            full_output.shapes, full_output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [graphics.config.width,
                             graphics.config.height],
            pixels_per_point: full_output.pixels_per_point,
        };

        let mut encoder = graphics.device.create_command_encoder(
            &Default::default());

        // Scope first: clears the whole surface with the theme's
        // background, deposits the glow inside the scope rect.
        // Glass: the pane's opacity is the per-style tint; the clear
        // is premultiplied so the compositor sees the desktop through.
        let theme = build_theme(&self.settings);
        let glass_alpha = if self.settings.scope_glass {
            let style = &self.settings.ui_style;
            (*self.settings.glass_tints.get(style)
                .unwrap_or(&self.settings.glass_tint)) as f64
        } else {
            1.0
        };
        let background = wgpu::Color {
            r: (theme.background_color[0] as f64).powf(2.2) * glass_alpha,
            g: (theme.background_color[1] as f64).powf(2.2) * glass_alpha,
            b: (theme.background_color[2] as f64).powf(2.2) * glass_alpha,
            a: glass_alpha,
        };
        if let Some(gpu) = graphics.scope_gpu.as_mut() {
            gpu.scope_alpha = glass_alpha as f32;
            // clamp to the ACQUIRED surface: on a shrink (mini) the
            // egui rect is one frame stale and a scissor outside the
            // render target is a validation error
            let surface_w = graphics.config.width as f32;
            let surface_h = graphics.config.height as f32;
            let x = scope_physical.min.x.clamp(0.0, surface_w - 1.0);
            let y = scope_physical.min.y.clamp(0.0, surface_h - 1.0);
            let w = scope_physical.width().max(1.0).min(surface_w - x);
            let h = scope_physical.height().max(1.0).min(surface_h - y);
            gpu.composite_into(&mut encoder, &view, (x, y, w, h),
                               Some(background));
        }

        // egui on top (Load — never clear over the scope)
        for (id, delta) in &full_output.textures_delta.set {
            graphics.egui_renderer.update_texture(
                &graphics.device, &graphics.queue, *id, delta);
        }
        graphics.egui_renderer.update_buffers(
            &graphics.device, &graphics.queue, &mut encoder,
            &clipped, &screen);
        {
            let load = if graphics.scope_gpu.is_some() {
                wgpu::LoadOp::Load
            } else {
                wgpu::LoadOp::Clear(background)
            };
            let pass = encoder.begin_render_pass(
                &wgpu::RenderPassDescriptor {
                    label: Some("chrome"),
                    color_attachments: &[Some(
                        wgpu::RenderPassColorAttachment {
                            view: &view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                    ..Default::default()
                });
            let mut pass = pass.forget_lifetime();
            graphics.egui_renderer.render(&mut pass, &clipped, &screen);
        }
        for id in &full_output.textures_delta.free {
            graphics.egui_renderer.free_texture(id);
        }
        graphics.queue.submit([encoder.finish()]);
        graphics.window.pre_present_notify();
        frame.present();
        true
    }
}

impl ApplicationHandler for Shell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_none() {
            self.init_graphics(event_loop);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop,
                    _id: WindowId, event: WindowEvent) {
        if let Some(graphics) = self.graphics.as_mut() {
            let response = graphics.egui_state
                .on_window_event(&graphics.window, &event);
            // RedrawRequested answers repaint=true by definition —
            // honoring it would busy-loop past the frame pacing.
            if response.repaint
                && !matches!(event, WindowEvent::RedrawRequested)
            {
                self.chrome_dirty = true;
                graphics.window.request_redraw();
            }
        }
        match event {
            WindowEvent::CloseRequested => {
                // clean shutdown: the catch-all save (v3 §18 law —
                // most keys only reach disk here)
                self.settings.save(&default_path()).ok();
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(graphics) = self.graphics.as_mut() {
                    graphics.config.width = size.width.max(1);
                    graphics.config.height = size.height.max(1);
                    graphics.surface.configure(&graphics.device,
                                               &graphics.config);
                    // chrome MUST repaint even while quiet-asleep, or
                    // freshly exposed regions present as black bands
                    self.chrome_dirty = true;
                    graphics.window.request_redraw();
                }
                // mini stays square: a WM corner-resize can skew it —
                // re-square once the drag settles (shares the
                // magnetism settle timer)
                if self.is_mini {
                    let side = size.width.max(size.height)
                        .clamp(140, 1000) as i64;
                    if size.width != size.height {
                        self.mini_resquare = Some(side);
                    }
                    self.settings.mini_size = side;
                    self.mini_settle = Some(
                        Instant::now() + Duration::from_millis(180));
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_position = (position.x, position.y);
            }
            WindowEvent::MouseInput {
                state: winit::event::ElementState::Pressed,
                button: winit::event::MouseButton::Left, ..
            } if self.is_mini => {
                // Mini owns left-press: corner 26 px = WM resize, a
                // quick second click = restore, anywhere else = WM
                // move (v3: drag moves the mini; drag never orbits).
                if let Some(graphics) = &self.graphics {
                    let now = Instant::now();
                    let double = self.mini_last_click
                        .is_some_and(|t| now.duration_since(t)
                                     < Duration::from_millis(400));
                    self.mini_last_click = Some(now);
                    if double {
                        self.actions.push(UiAction::MiniToggle);
                        graphics.window.request_redraw();
                    } else {
                        let size = graphics.window.inner_size();
                        let (x, y) = self.cursor_position;
                        let in_corner = x > (size.width as f64 - 26.0)
                            && y > (size.height as f64 - 26.0);
                        let _ = if in_corner {
                            graphics.window.drag_resize_window(
                                winit::window::ResizeDirection::SouthEast)
                        } else {
                            graphics.window.drag_window()
                        };
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                // v3 law: the window handles keys unless a text entry
                // is being edited. egui grabs widget focus liberally
                // (buttons included), so gate ONLY on an egui widget
                // holding focus AND expecting text — otherwise our
                // table wins first, like GTK's window-level handler.
                // The gate is OUR text registry: egui 0.33's
                // wants_keyboard_input() is literally
                // focused().is_some(), and clicked buttons KEEP focus
                // — gating on it killed every shortcut after the
                // first click (Ben's "Show FPS doesn't work at all").
                let editing = self.graphics.as_ref()
                    .and_then(|g| g.egui_ctx.memory(|m| m.focused()))
                    .is_some_and(|id| self.text_focus_ids.contains(&id));
                if !editing
                    && event.state == winit::event::ElementState::Pressed
                {
                    match self.handle_key(&event.logical_key) {
                        crate::keyboard::KeyOutcome::CloseRequested => {
                            self.settings.save(&default_path()).ok();
                            event_loop.exit();
                        }
                        _ => {
                            self.chrome_dirty = true;
                            if let Some(graphics) = &self.graphics {
                                graphics.window.request_redraw();
                            }
                        }
                    }
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let over_ui = self.graphics.as_ref()
                    .map(|g| g.egui_ctx.wants_pointer_input())
                    .unwrap_or(false);
                if !over_ui {
                    let notches = match delta {
                        winit::event::MouseScrollDelta::LineDelta(_, y) =>
                            y as f64,
                        winit::event::MouseScrollDelta::PixelDelta(p) =>
                            p.y / 40.0,
                    };
                    let is_3d = matches!(
                        self.settings.display_mode.as_str(),
                        "xyz_takens" | "helix");
                    if self.is_mini && self.ctrl_down {
                        // Ctrl+scroll resizes the mini view (v3 §6.4)
                        let size = (self.settings.mini_size as f64
                                    + notches * 20.0)
                            .clamp(140.0, 1000.0) as i64;
                        self.actions.push(UiAction::MiniSizePreset(size));
                    } else if is_3d && !self.composing {
                        // wheel-dolly (§8.2: 0.92 in / 1.09 out,
                        // clamp 1.6..8.0 — works in mini too)
                        let factor = if notches > 0.0 { 0.92 } else { 1.09 };
                        self.camera_dolly =
                            (self.camera_dolly * factor).clamp(1.6, 8.0);
                        self.push_camera();
                        self.mark_orbit_interaction();
                    } else {
                        // wheel on the scope = gain — MULTIPLICATIVE
                        // so steps feel even across 0.1..6.0 (linear
                        // +0.15 was mushy low and jumpy high — Ben's
                        // "inconsistent zoom" feedback)
                        let factor = 1.08f32.powf(notches as f32);
                        self.settings.gain =
                            (self.settings.gain * factor).clamp(0.1, 6.0);
                        self.actions.push(UiAction::SignalTuning);
                    }
                    self.chrome_dirty = true;
                    if let Some(graphics) = &self.graphics {
                        graphics.window.request_redraw();
                    }
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.ctrl_down = modifiers.state().control_key();
            }
            WindowEvent::Moved(_) => {
                if self.is_mini {
                    // magnetism: settle 180 ms after the LAST move
                    self.mini_settle = Some(
                        Instant::now() + Duration::from_millis(180));
                }
            }
            WindowEvent::DroppedFile(path) => {
                // whole-window drop target (v3 §1.5): .phoskit files
                // import (pass iv); audio files become the playlist
                // verbatim and the first one plays
                let lower = path.to_string_lossy().to_lowercase();
                if lower.ends_with(".phoskit") {
                    self.status_line =
                        "kit import lands in chrome pass iv".into();
                } else if crate::player::is_audio_path(&path)
                    && path.exists()
                {
                    self.player.playlist = vec![path.clone()];
                    self.player.playlist_index = 0;
                    self.play_file(&path, false);
                }
                self.chrome_dirty = true;
            }
            WindowEvent::RedrawRequested => {
                let keep_going = self.redraw();
                if keep_going && self.render_loop_active {
                    // paced by a ROLLING deadline (about_to_wait fires
                    // it): period anchors to the previous due, not to
                    // "now + period" — the absolute form loses the
                    // frame's own work time and lands ~152 on a 165 Hz
                    // panel (measured; ≥157 is the wave-2 law).
                    let cap = self.cap_hz();
                    if cap > 0.0 {
                        let period = Duration::from_secs_f64(1.0 / cap);
                        let now = Instant::now();
                        let due = match self.next_frame_anchor {
                            Some(anchor) if anchor + period > now =>
                                anchor + period,
                            _ => now, // fell behind: re-anchor
                        };
                        self.next_frame_anchor = Some(due);
                        self.next_frame_due = Some(due);
                    } else if let Some(graphics) = &self.graphics {
                        graphics.window.request_redraw();
                    }
                }
                if self.quit_requested {
                    self.settings.save(&default_path()).ok();
                    event_loop.exit();
                }
                if let Some(limit) = self.args.exit_after
                    && self.started.elapsed().as_secs_f64() >= limit
                {
                    event_loop.exit();
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // mini magnetism settle (fires even while the loop idles)
        if let Some(due) = self.mini_settle
            && Instant::now() >= due
        {
            self.mini_settle = None;
            if let Some(graphics) = self.graphics.take() {
                // square first (a corner drag may have skewed it),
                // then snap to the work-area edges
                if let Some(side) = self.mini_resquare.take() {
                    let _ = graphics.window.request_inner_size(
                        winit::dpi::PhysicalSize::new(
                            side as u32, side as u32));
                }
                self.snap_mini_to_edges(&graphics);
                self.graphics = Some(graphics);
            }
        }
        let mut wake_at = self.mini_settle;
        if (self.render_loop_active || self.chrome_dirty)
            && let Some(due) = self.next_frame_due
        {
            if Instant::now() >= due {
                self.next_frame_due = None;
                if let Some(graphics) = &self.graphics {
                    graphics.window.request_redraw();
                }
                event_loop.set_control_flow(ControlFlow::Poll);
                return;
            }
            wake_at = Some(wake_at.map_or(due, |w| w.min(due)));
        }
        match wake_at {
            Some(due) => event_loop
                .set_control_flow(ControlFlow::WaitUntil(due)),
            // faded out: pure event-driven idle (zero CPU) — capture
            // or playback start calls wake_render_loop
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }
}

pub fn run(arguments: &[String]) -> i32 {
    let args = parse_args(arguments);
    let mut shell = match Shell::new(args) {
        Ok(shell) => shell,
        Err(message) => {
            eprintln!("phosphor: {message}");
            return 4;
        }
    };
    let event_loop = match EventLoop::new() {
        Ok(event_loop) => event_loop,
        Err(error) => {
            eprintln!("phosphor: event loop: {error}");
            return 4;
        }
    };
    match event_loop.run_app(&mut shell) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("phosphor: {error}");
            4
        }
    }
}
