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
use phosphor_render_gpu::GpuRenderer;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

use crate::render::{build_computer, build_theme, build_theme_phase,
                    beam_cycle_animating, cycle_beam_color_phase};

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
    /// the raster thread (issue #5): chrome never blocks on a raster
    worker: crate::raster_worker::RasterWorker,
    texture: Option<egui::TextureHandle>,
    /// size of the newest published frame (0,0 until one lands)
    frame_size: (usize, usize),
    /// worker-measured advance+composite cost (HUD)
    raster_ms: Option<f32>,
}

impl CpuScope {
    fn fresh() -> CpuScope {
        CpuScope {
            worker: crate::raster_worker::RasterWorker::spawn(),
            texture: None,
            frame_size: (0, 0),
            raster_ms: None,
        }
    }
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
    /// fold several app streams into one beam (issue #6 — the light
    /// streams panel; the engine has been ready since wave 2)
    StartMix(Vec<String>),
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
    /// context menu while composing: the 10 s shareable WAV
    ExportDrawing,
    MiniToggle,
    PinToggle,
    FullscreenToggle,
    AlignMini(f32, f32),
    MiniSizePreset(i64),
    /// Route/release the CAPTURED app through the vacuum (the second,
    /// ephemeral ⌀ — never saved; distinct from the file-vacuum).
    VacuumApp(bool),
    KitChanged,
    OpenKitEditor,
    OpenPostcard,
    Quit,
}

/// The single source of truth for what feeds the beam. Transitions
/// re-derive it via `sync_beam_source`; the target combo renders from it
/// and probe reports it — so display state can never drift from the
/// engine again (the "Spotify still selected → black screen" root).
/// A `Player` source means the player session owns the beam, playing OR
/// paused — capture outranks it while on.
#[derive(Clone, PartialEq, Debug, Default)]
pub(crate) enum BeamSource {
    Capture { combo_id: String },
    /// several app streams folded (engine-ready; UI in the ensemble wave)
    #[allow(dead_code)]
    Mix { combo_ids: Vec<String> },
    Player { path: std::path::PathBuf },
    #[default]
    Silent,
}

impl BeamSource {
    /// What the collapsed target combo shows (chrome prepends the kind
    /// icon). `resolve` maps a combo id to its human label.
    pub(crate) fn combo_label(
        &self, resolve: impl Fn(&str) -> Option<String>) -> String
    {
        match self {
            BeamSource::Capture { combo_id } =>
                resolve(combo_id).unwrap_or_else(|| combo_id.clone()),
            BeamSource::Mix { combo_ids } =>
                format!("APP mix ({})", combo_ids.len()),
            BeamSource::Player { path } => path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "playing file".into()),
            BeamSource::Silent => "— pick a source —".into(),
        }
    }

    pub(crate) fn status(&self) -> crate::control::SourceStatus {
        let (kind, detail) = match self {
            BeamSource::Capture { combo_id } =>
                ("capture", Some(combo_id.clone())),
            BeamSource::Mix { combo_ids } =>
                ("mix", Some(combo_ids.join("+"))),
            BeamSource::Player { path } =>
                ("player", Some(path.to_string_lossy().to_string())),
            BeamSource::Silent => ("silent", None),
        };
        crate::control::SourceStatus { kind: kind.into(), detail }
    }
}

pub struct Shell {
    pub(crate) args: ShellArgs,
    pub(crate) settings: Settings,
    pub(crate) engine: AudioEngine,
    audio_events: mpsc::Receiver<AudioEvent>,
    pub(crate) computer: phosphor_dsp::Computer,
    renderer_choice: RendererChoice,

    graphics: Option<Graphics>,
    pub(crate) scope_rect: egui::Rect,
    pub(crate) actions: Vec<UiAction>,
    pub(crate) target_cache: Vec<phosphor_audio::CaptureTarget>,
    pub(crate) settings_panel_open: bool,
    /// the in-app Manual window (book icon, left of the gear)
    pub(crate) manual_open: bool,
    /// the light-streams panel (mix several apps into one beam)
    pub(crate) mix_panel_open: bool,
    /// apps ticked in the light-streams panel (combo ids)
    pub(crate) mix_selection: std::collections::HashSet<String>,
    pub(crate) active_palette: crate::theme::Palette,
    /// short-lived on-scope toast (snapshot saved, vacuum notes…)
    pub(crate) toast: Option<(String, Instant)>,
    /// the kit editor window (None = closed)
    pub(crate) kit_editor: Option<crate::chrome::KitEditorState>,
    /// the postcard-export dialog (None = closed)
    pub(crate) postcard_dialog: Option<crate::chrome::PostcardState>,
    /// decoded cover-art texture for the playing track + its source path
    pub(crate) cover_texture: Option<(std::path::PathBuf, egui::TextureHandle)>,
    /// art for the corner now-playing overlay (left of the title —
    /// Ben's last-polish ask); set at flash time from the embedded
    /// cover (own tracks) or the client's cached fetch (external)
    pub(crate) overlay_art: Option<egui::TextureHandle>,
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
    /// pointer currently down, stroke in progress (compose mode)
    pub(crate) compose_drawing: bool,
    /// the in-progress stroke, egui logical coords (absolute)
    pub(crate) compose_stroke: Vec<egui::Pos2>,
    /// finished shape in signal space — what the loop replays
    pub(crate) compose_loop_points: Option<Vec<(f64, f64)>>,
    /// scroll-retune debounce deadline (300 ms, v3 law)
    pub(crate) compose_retune_due: Option<Instant>,
    /// AGC: the gain actually driving the computer (v3 _effective_gain
    /// — equals settings.gain unless auto-gain is gliding it)
    pub(crate) effective_gain: f32,
    /// AGC peak tracker: instant attack, slow release (v3 law)
    auto_gain_peak: f32,
    /// gain the graticule was last derived at (re-derive on >2% moves
    /// only, so auto-gain's tiny per-frame glides stay free — v3 law)
    grid_gain: f32,
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
    /// cached _NET_WORKAREA (x, y, w, h) + when it was queried — the
    /// settle path reads this instead of shelling out to xprop every
    /// tick (a moved panel is rare; 30 s staleness is fine)
    workarea_cache: Option<((i32, i32, i32, i32), Instant)>,
    /// a WM move/resize drag of the mini is live — while set, defer the
    /// square-enforcing request_inner_size so it can't fight the drag
    mini_drag_active: bool,
    /// set when set_mini_mode enters mini; a 400 ms grace during which
    /// the Resized burst we caused ourselves must NOT be read as a
    /// user corner-drag skew (would schedule a spurious re-square)
    mini_entering: Option<Instant>,
    /// the scope context menu is open this frame — while true the mini
    /// settle NEVER re-squares/snaps (the window moving under an open
    /// menu was Ben's "right click glitches out a ton")
    pub(crate) context_menu_open: bool,
    /// a click outside the menu asked it to close (honored inside the
    /// menu closure next frame — reliable even when a WM grab or the
    /// fullscreen surface swallows the release egui would need)
    pub(crate) close_menu_request: bool,
    /// external now-playing signature (title|artist of the linked
    /// player) — flashes the corner overlay on change (v3 watcher law)
    last_external_signature: Option<String>,
    cursor_position: (f64, f64),
    /// pointer over the scope widget last frame (egui occlusion-aware:
    /// false under chrome, panels, or floating dialogs) — the wheel
    /// gate. `wants_pointer_input()` was WRONG here: the CentralPanel
    /// counts as an egui area, so it was true over the bare scope and
    /// silently killed every scope-wheel behavior (found live by the
    /// compose-retune receipt; gain/dolly/mini-resize wheels rode the
    /// same dead branch).
    scope_hovered: bool,
    mini_last_click: Option<Instant>,
    /// the Konami visitor swim (verbatim v3 turtle)
    visitor_started: Option<Instant>,
    pub(crate) exporting: bool,
    pub(crate) export_results:
        Option<mpsc::Receiver<Result<std::path::PathBuf, String>>>,
    mpris: Option<crate::mpris::MprisHandle>,
    /// the MPRIS *client*: other players, watched and driven
    pub(crate) mpris_client:
        Option<crate::mpris_client::MprisClientHandle>,
    /// last own-track desktop-notification id (replaced, not stacked)
    notification_id: u32,
    /// the control socket (None headless / when the bind failed)
    control: Option<crate::control::ControlHandle>,
    /// a deferred snapshot/clip reply: the export runs on a thread, so
    /// the control reply is held until its result lands. (kind, sender)
    control_export_reply:
        Option<(&'static str, mpsc::Sender<serde_json::Value>)>,

    // quiet law state
    quiet_frame_count: u32,
    fade_out_frames_remaining: u32,
    render_loop_active: bool,

    // pacing + receipts
    monitor_hz: f64,
    next_frame_due: Option<Instant>,
    next_frame_anchor: Option<Instant>,
    pub(crate) chrome_dirty: bool,
    last_frame_time: Option<Instant>,
    pub(crate) started: Instant,
    /// a sub-1 s transition was requested and awaits the
    /// photosensitivity prompt (holds the value the user asked for;
    /// the applied setting stays pinned at 1.0 s meanwhile)
    pub(crate) epilepsy_prompt: Option<f64>,
    /// sub-1 s transitions were confirmed once this session — the
    /// prompt returns next launch (safety over convenience)
    pub(crate) epilepsy_ack: bool,
    /// track-mode cycle: which color slot the beam is resting on
    pub(crate) cycle_song_index: usize,
    /// a song-change crossfade is in flight since this instant (the
    /// sweep from the previous slot takes beam_cycle_seconds)
    pub(crate) cycle_song_fade: Option<Instant>,
    fps_frames: u32,
    fps_window_start: Instant,
    pub last_fps: f64,
    /// rolling frame-work times (ms) while advancing — p99 source
    work_ms_ring: [f32; 240],
    work_ms_count: usize,
    last_work_ms: f32,
    /// redraw gaps > 1.5× the pacing period while actively rendering
    dropped_frames: u32,
    last_segment_count: usize,
    /// segments accumulated over the current fps window → seg/s (the
    /// per-frame count is misleading: 384 kHz reconstruction arrives
    /// in ~8k bursts every ~21 ms with zeros between — measured)
    segments_window: usize,
    segments_per_second: f64,
    pub(crate) capture_on: bool,
    pub(crate) beam_source: BeamSource,
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
        let initial_gain = settings.gain;
        let track_notifications = settings.track_notifications;
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
            manual_open: false,
            mix_panel_open: false,
            mix_selection: Default::default(),
            active_palette: crate::theme::palette("blossom"),
            toast: None,
            kit_editor: None,
            postcard_dialog: None,
            cover_texture: None,
            overlay_art: None,
            player: Default::default(),
            file_dialog: None,
            konami_progress: 0,
            camera_yaw: 0.0,
            camera_pitch: 0.0,
            camera_dolly: 3.2,
            orbit_last_interaction: Instant::now(),
            composing: false,
            compose_drawing: false,
            compose_stroke: Vec::new(),
            compose_loop_points: None,
            compose_retune_due: None,
            effective_gain: initial_gain,
            auto_gain_peak: 0.0,
            grid_gain: initial_gain,
            is_mini: false,
            is_fullscreen: false,
            normal_geometry: None,
            mini_settle: None,
            app_vacuum: None,
            ctrl_down: false,
            quit_requested: false,
            text_focus_ids: std::collections::HashSet::new(),
            mini_resquare: None,
            workarea_cache: None,
            mini_drag_active: false,
            mini_entering: None,
            context_menu_open: false,
            close_menu_request: false,
            last_external_signature: None,
            cursor_position: (0.0, 0.0),
            scope_hovered: false,
            mini_last_click: None,
            visitor_started: None,
            exporting: false,
            export_results: None,
            mpris: crate::mpris::spawn(),
            mpris_client: crate::mpris_client::spawn(track_notifications),
            notification_id: 0,
            // the control socket is bound in run() once the event loop
            // (and its wake proxy) exist
            control: None,
            control_export_reply: None,
            quiet_frame_count: 0,
            fade_out_frames_remaining: FADE_OUT_FRAMES,
            render_loop_active: true,
            monitor_hz: 0.0,
            next_frame_due: None,
            next_frame_anchor: None,
            chrome_dirty: true,
            last_frame_time: None,
            started: Instant::now(),
            epilepsy_prompt: None,
            epilepsy_ack: false,
            cycle_song_index: 0,
            cycle_song_fade: None,
            fps_frames: 0,
            fps_window_start: Instant::now(),
            last_fps: 0.0,
            work_ms_ring: [0.0; 240],
            work_ms_count: 0,
            last_work_ms: 0.0,
            dropped_frames: 0,
            last_segment_count: 0,
            segments_window: 0,
            segments_per_second: 0.0,
            capture_on: false,
            beam_source: BeamSource::Silent,
            status_line: String::new(),
        })
    }

    fn init_graphics(&mut self, event_loop: &ActiveEventLoop) {
        #[allow(unused_imports)]
        use winit::platform::x11::WindowAttributesExtX11;
        let mut attributes = Window::default_attributes()
            .with_title("Phosphor")
            // WM_CLASS "phosphor" — the .desktop StartupWMClass match
            // (v3 did this via GLib.set_prgname)
            .with_name("phosphor", "phosphor")
            .with_transparent(true)
            .with_inner_size(winit::dpi::LogicalSize::new(
                self.settings.window_width.max(320) as f64,
                self.settings.window_height.max(240) as f64,
            ));
        // the 4-panel scope icon (embedded, decoded once)
        if let Some(icon) = load_window_icon() {
            attributes = attributes.with_window_icon(Some(icon));
        }
        // restore the remembered window position (v3 law)
        if let (Some(x), Some(y)) =
            (self.settings.window_x, self.settings.window_y)
        {
            attributes = attributes.with_position(
                winit::dpi::PhysicalPosition::new(x as i32, y as i32));
        }
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
        // GPU timestamps for the nerd HUD, when the adapter has them
        // (RADV does) — a missing feature just means gpu_ms reads None.
        let timestamp_features = adapter.features()
            & wgpu::Features::TIMESTAMP_QUERY;
        let (device, queue) = pollster::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("phosphor live"),
                required_features: timestamp_features,
                ..Default::default()
            }))
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
        // The type system (regalia wave): IBM Plex Sans for prose —
        // egui's default Ubuntu-Light was the "text hard to read"
        // culprit (a thin face at small sizes) — JetBrains Mono for
        // DATA (the skill's mono rule), a Medium family for headings,
        // and the Phosphor icon font for glyphs (no raw-unicode tofu).
        {
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "plex-sans".into(),
                egui::FontData::from_static(include_bytes!(
                    "../assets/fonts/IBMPlexSans-Regular.ttf")).into());
            fonts.font_data.insert(
                "plex-sans-medium".into(),
                egui::FontData::from_static(include_bytes!(
                    "../assets/fonts/IBMPlexSans-Medium.ttf")).into());
            fonts.font_data.insert(
                "jetbrains-mono".into(),
                egui::FontData::from_static(include_bytes!(
                    "../assets/fonts/JetBrainsMono-Regular.ttf")).into());
            if let Some(family) = fonts.families
                .get_mut(&egui::FontFamily::Proportional)
            {
                family.insert(0, "plex-sans".into());
            }
            if let Some(family) = fonts.families
                .get_mut(&egui::FontFamily::Monospace)
            {
                family.insert(0, "jetbrains-mono".into());
            }
            fonts.families.insert(
                egui::FontFamily::Name("plex-medium".into()),
                vec!["plex-sans-medium".into(), "plex-sans".into()]);
            egui_phosphor::add_to_fonts(
                &mut fonts, egui_phosphor::Variant::Regular);
            egui_ctx.set_fonts(fonts);
        }
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
            RendererChoice::Cpu => Some(CpuScope::fresh()),
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
                    if self.player.playing.is_some() {
                        // v3 law (phosphor.py capture-toggle): while a
                        // track is loaded the toggle is PLAY/PAUSE —
                        // Space muscle memory. Starting device capture
                        // over the playing track double-feeds the ring
                        // (found live: the Attack Vector receipt drew
                        // both streams at once).
                        let paused = !self.player.paused;
                        if !paused && self.capture_on {
                            // resuming the track takes the beam back:
                            // capture stops first, or both would feed
                            // the ring at once (the double-feed law,
                            // now symmetric with target-pick)
                            self.engine.stop_capture();
                            self.capture_on = false;
                        }
                        self.engine.set_playback_paused(paused);
                        self.player.paused = paused;
                        self.set_mpris_status(
                            if paused { "Paused" } else { "Playing" });
                        self.sync_beam_source(None);
                        self.wake_render_loop();
                        continue;
                    }
                    // starting capture leaves compose; the new stream
                    // replaces the loop, no explicit stop (v3 law)
                    self.exit_compose(false);
                    self.start_capture_from_settings();
                }
                UiAction::CaptureOff => {
                    self.engine.stop_capture();
                    self.capture_on = false;
                    self.status_line = "idle".into();
                    self.sync_beam_source(None);
                    self.wake_render_loop();
                }
                UiAction::TargetPicked(combo_id) => {
                    // switching away restores the sound (v3 law)
                    self.engine.vacuum_release();
                    if self.composing {
                        // picking a target while composing only
                        // records it — the loop keeps playing (v3)
                        continue;
                    }
                    // A pick is an explicit "scope this": the playing
                    // track PAUSES (not stops — Space brings it back)
                    // and capture starts even from idle. The old guard
                    // read is_playing_file() AFTER stopping playback,
                    // so it was always false → nothing started, the
                    // fade completed, and the scope froze black with
                    // the combo still claiming the old source.
                    if self.engine.is_playing_file() && !self.player.paused
                    {
                        self.engine.set_playback_paused(true);
                        self.player.paused = true;
                        self.set_mpris_status("Paused");
                    }
                    if self.engine.start_capture(&combo_id) {
                        self.capture_on = true;
                        self.status_line = format!("scoping {combo_id}");
                        self.sync_beam_source(Some(combo_id));
                    } else {
                        self.capture_on = false;
                        self.status_line =
                            format!("capture failed: {combo_id}");
                        self.toast_now(format!(
                            "couldn't scope {combo_id} — see terminal"));
                        self.sync_beam_source(None);
                    }
                    // ALWAYS wake: even a failed start must repaint so
                    // the scope shows its labeled state, never a
                    // frozen stale frame
                    self.wake_render_loop();
                }
                UiAction::StartMix(combo_ids) => {
                    self.engine.vacuum_release();
                    if self.composing {
                        continue;
                    }
                    if self.engine.is_playing_file()
                        && !self.player.paused
                    {
                        self.engine.set_playback_paused(true);
                        self.player.paused = true;
                        self.set_mpris_status("Paused");
                    }
                    let connected =
                        self.engine.start_capture_mix(&combo_ids);
                    if connected > 0 {
                        self.capture_on = true;
                        self.beam_source = BeamSource::Mix {
                            combo_ids: combo_ids.clone(),
                        };
                        self.status_line = format!(
                            "mixing {connected} light streams");
                    } else {
                        self.capture_on = false;
                        self.sync_beam_source(None);
                        self.status_line =
                            "mix: no app streams connected".into();
                        self.toast_now(String::from(
                            "no running app streams matched — are \
                             they playing right now?"));
                    }
                    self.wake_render_loop();
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
                    // a broken kit renders plain, never silently: the
                    // user just picked it — say so (audit: the CLI
                    // path warns on stderr, the GUI needs a toast)
                    if self.settings.kit_enabled
                        && let Some(path) = &self.settings.kit_path
                        && let Err(error) = phosphor_proto::phoskit::load(
                            std::path::Path::new(path))
                    {
                        self.toast_now(format!("kit ignored: {error}"));
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
                    self.computer.beam_energy = self.settings.beam_energy;
                    if self.settings.auto_gain {
                        // re-measure from the next sound (v3 law);
                        // the glide picks up from wherever it was
                        self.auto_gain_peak = 0.0;
                    } else {
                        self.effective_gain = self.settings.gain;
                        self.computer.gain = self.settings.gain;
                    }
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
                    let mut settings = self.settings.clone();
                    let rate = self.settings.scope_sample_rate;
                    // cycle origin so exports are WYSIWYG: the clip
                    // re-lives the last N seconds of color, the
                    // snapshot lands on the color on screen right now.
                    // Track mode: the color rests between songs — the
                    // export FREEZES on the current interpolated color
                    // (collapse the clone to a one-color cycle).
                    if settings.beam_cycle_mode == "track"
                        && beam_cycle_animating(&settings)
                    {
                        settings.custom_beam_color =
                            cycle_beam_color_phase(
                                &settings, self.beam_cycle_phase());
                        settings.beam_cycle_count = 1;
                    }
                    let cycle_t0 = self.started.elapsed().as_secs_f64()
                        - if snapshot { 0.0 } else { seconds as f64 };
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
                                history, settings, rate, cycle_t0)
                        } else {
                            crate::exports::save_clip(
                                history, settings, rate, cycle_t0)
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
                    if !paused && self.capture_on {
                        // resume takes the beam back from capture
                        // (double-feed law, same as the LIVE toggle)
                        self.engine.stop_capture();
                        self.capture_on = false;
                        self.sync_beam_source(None);
                    }
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
                    if self.composing {
                        self.exit_compose(true);
                    } else {
                        self.enter_compose();
                    }
                }
                UiAction::ExportDrawing => {
                    self.export_compose_drawing();
                }
                UiAction::MiniToggle => {
                    // a menu left open across the switch would wear the
                    // old mode's geometry — ask it to close first
                    self.close_menu_request = true;
                    let enable = !self.is_mini;
                    self.set_mini_mode(enable, graphics);
                }
                UiAction::PinToggle => {
                    self.settings.pinned = !self.settings.pinned;
                    self.apply_window_level(graphics);
                }
                UiAction::FullscreenToggle => {
                    // same courtesy as MiniToggle: no menu survives a
                    // mode switch with stale geometry
                    self.close_menu_request = true;
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
                UiAction::OpenKitEditor => {
                    // seed from the current kit if one is loaded, else
                    // a single rotate stage
                    let stages = self.settings.kit_path.as_deref()
                        .and_then(|p| phosphor_proto::phoskit::load(
                            std::path::Path::new(p)).ok())
                        .map(|kit| (kit.name, kit.author, kit.stages))
                        .unwrap_or_else(|| (
                            "my kit".into(),
                            self.settings.postcard_credit.clone(),
                            vec![("rotate".into(),
                                  phosphor_proto::phoskit::default_params(
                                      "rotate"))]));
                    self.kit_editor = Some(crate::chrome::KitEditorState {
                        name: stages.0, author: stages.1, stages: stages.2,
                    });
                }
                UiAction::OpenPostcard => {
                    if let Some(path) = self.player.playing.clone() {
                        self.postcard_dialog =
                            Some(crate::chrome::PostcardState {
                                title: path.file_stem()
                                    .map(|s| s.to_string_lossy().to_string())
                                    .unwrap_or_default(),
                                credit: self.settings.postcard_credit.clone(),
                                source: path,
                            });
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
                                    self.app_vacuum = Some(key.clone());
                                    self.sync_beam_source(
                                        Some(monitor_id));
                                    self.wake_render_loop();
                                    // say what the light state IS —
                                    // the ⌀ mystery, spelled out
                                    self.status_line = format!(
                                        "vacuum · {key} plays as light \
                                         only (no sound)");
                                    self.toast_now(format!(
                                        "{key} → vacuum: light only, \
                                         sound returns when released"));
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
                            self.sync_beam_source(Some(id));
                        } else {
                            self.sync_beam_source(None);
                        }
                        self.status_line =
                            "vacuum released — sound restored".into();
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
            self.apply_external_command(command);
        }
    }

    /// The one code path every external controller (MPRIS media keys AND
    /// the control socket's transport verbs) funnels through — reuse,
    /// not fork.
    pub(crate) fn apply_external_command(
        &mut self, command: crate::mpris::MprisCommand) {
        use crate::mpris::MprisCommand;
        // Transport verbs follow THE BEAM (Ben's patch list): while
        // capture scopes a linked player — even with a local track
        // loaded-and-paused underneath — play/pause/next/previous
        // drive that player. The local file keeps Space (the beam
        // arbiter) and the playlist panel.
        if matches!(command,
                    MprisCommand::Next | MprisCommand::Previous
                    | MprisCommand::PlayPause | MprisCommand::Play
                    | MprisCommand::Pause)
            && let Some(external) = self.linked_external_player()
            && let Some(client) = &self.mpris_client
        {
            use crate::mpris_client::ClientCommand;
            let bus = external.bus_name;
            let _ = client.commands.send(match command {
                MprisCommand::Next => ClientCommand::Next(bus),
                MprisCommand::Previous => ClientCommand::Previous(bus),
                MprisCommand::Play => ClientCommand::Play(bus),
                MprisCommand::Pause => ClientCommand::Pause(bus),
                _ => ClientCommand::PlayPause(bus),
            });
            return;
        }
        let playing = self.player.playing.is_some();
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
                self.sync_beam_source(None);
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

    /// Drain control-socket requests: apply each verb and reply. Sits
    /// beside service_mpris in the tick (woken by the proxy). Every
    /// error reply names its fix (station convention).
    fn service_control(&mut self) {
        use serde_json::json;
        let Some(control) = &self.control else { return };
        let mut requests = Vec::new();
        while let Ok(request) = control.requests.try_recv() {
            requests.push(request);
        }
        for request in requests {
            use crate::control::ControlVerb;
            let reply: serde_json::Value = match request.verb {
                ControlVerb::Transport(command) => {
                    self.apply_external_command(command);
                    json!({"status": "ok", "verb": "transport"})
                }
                ControlVerb::Mode(name) => {
                    match name.parse::<phosphor_dsp::Mode>() {
                        Ok(_) => {
                            self.settings.display_mode = name.clone();
                            self.actions.push(UiAction::ModeChanged);
                            json!({"status": "ok", "verb": "mode",
                                   "result": {"mode": name}})
                        }
                        Err(message) => json!({
                            "status": "error", "error": message,
                            "fix": format!("one of: {}",
                                phosphor_dsp::Mode::ALL
                                    .map(phosphor_dsp::Mode::name)
                                    .join(", ")),
                        }),
                    }
                }
                ControlVerb::Theme(name) => {
                    if crate::chrome::THEME_NAMES.contains(&name.as_str()) {
                        self.settings.theme_name = name.clone();
                        self.actions.push(UiAction::SaveSettings);
                        self.chrome_dirty = true;
                        json!({"status": "ok", "verb": "theme",
                               "result": {"theme": name}})
                    } else {
                        json!({
                            "status": "error",
                            "error": format!("unknown theme '{name}'"),
                            "fix": format!("one of: {}",
                                crate::chrome::THEME_NAMES.join(", ")),
                        })
                    }
                }
                ControlVerb::UiStyle(name) => {
                    if crate::theme::PALETTES.iter()
                        .any(|p| p.id == name)
                    {
                        self.settings.ui_style = name.clone();
                        self.actions.push(UiAction::SaveSettings);
                        self.chrome_dirty = true;
                        json!({"status": "ok", "verb": "ui",
                               "result": {"ui_style": name}})
                    } else {
                        let ids: Vec<&str> = crate::theme::PALETTES
                            .iter().map(|p| p.id).collect();
                        json!({
                            "status": "error",
                            "error": format!("unknown ui style '{name}'"),
                            "fix": format!("one of: {}", ids.join(", ")),
                        })
                    }
                }
                ControlVerb::Capture(on) => {
                    self.actions.push(if on {
                        UiAction::CaptureOn
                    } else {
                        UiAction::CaptureOff
                    });
                    json!({"status": "ok", "verb": "capture",
                           "result": {"on": on}})
                }
                ControlVerb::Target(id) => {
                    // "mix:app:a+app:b" folds several app streams
                    if let Some(list) = id.strip_prefix("mix:") {
                        let members: Vec<String> = list
                            .split('+')
                            .map(str::to_string)
                            .filter(|m| !m.is_empty())
                            .collect();
                        self.actions.push(UiAction::StartMix(members));
                    } else {
                        self.actions.push(
                            UiAction::TargetPicked(id.clone()));
                    }
                    json!({"status": "ok", "verb": "target",
                           "result": {"id": id}})
                }
                ControlVerb::Raise => {
                    if let Some(graphics) = &self.graphics {
                        graphics.window.set_minimized(false);
                        graphics.window.focus_window();
                        json!({"status": "ok", "verb": "raise"})
                    } else {
                        json!({
                            "status": "error",
                            "error": "no window to raise yet",
                            "fix": "retry once the GUI has mapped",
                        })
                    }
                }
                ControlVerb::Open(path) => {
                    let path = std::path::PathBuf::from(
                        path.strip_prefix("file://").unwrap_or(&path));
                    if crate::player::is_audio_path(&path) && path.exists()
                    {
                        self.actions.push(UiAction::PlayPath(path.clone()));
                        if let Some(graphics) = &self.graphics {
                            graphics.window.set_minimized(false);
                            graphics.window.focus_window();
                        }
                        json!({"status": "ok", "verb": "open",
                               "result": {"path": path.to_string_lossy()}})
                    } else {
                        json!({
                            "status": "error",
                            "error": format!(
                                "not a playable audio file: {}",
                                path.display()),
                            "fix": "give an existing wav/flac/mp3/ogg/\
                                    m4a/opus/phos path",
                        })
                    }
                }
                verb @ (ControlVerb::Snapshot | ControlVerb::Clip) => {
                    let (kind, action) =
                        if matches!(verb, ControlVerb::Snapshot) {
                            ("snapshot", UiAction::SaveSnapshot)
                        } else {
                            ("clip", UiAction::SaveClip)
                        };
                    if self.exporting {
                        json!({
                            "status": "error",
                            "error": "export already running",
                            "fix": "wait for the current export",
                        })
                    } else {
                        self.actions.push(action);
                        // reply is deferred until the export thread lands
                        self.control_export_reply =
                            Some((kind, request.reply));
                        continue;
                    }
                }
                ControlVerb::Quit => {
                    // reply first, THEN ask to quit
                    let _ = request.reply.send(
                        json!({"status": "ok", "verb": "quit"}));
                    self.actions.push(UiAction::Quit);
                    continue;
                }
            };
            let _ = request.reply.send(reply);
            // keep the served status consistent with the ack we just
            // sent: a follow-up `status` on a fresh connection must not
            // race the end-of-tick refresh and read the stale frame.
            self.update_control_snapshot();
        }
    }

    /// Refresh the status snapshot the control socket serves (cheap;
    /// once per tick). `status` replies read this instantly, no wake.
    fn update_control_snapshot(&self) {
        // keep the client's notification gate current (atomic, cheap)
        if let Some(client) = &self.mpris_client {
            client.notify_enabled.store(
                self.settings.track_notifications,
                std::sync::atomic::Ordering::Relaxed);
        }
        let Some(control) = &self.control else { return };
        let metadata = self.engine.current_track_metadata()
            .unwrap_or_default();
        let snapshot = crate::control::StatusSnapshot {
            running: true,
            pid: std::process::id(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            mode: self.settings.display_mode.clone(),
            theme: self.settings.theme_name.clone(),
            ui_style: self.settings.ui_style.clone(),
            capture: crate::control::CaptureStatus {
                on: self.capture_on,
                target_id: self.settings.target_id.clone(),
            },
            source: self.beam_source.status(),
            player: crate::control::PlayerStatus {
                track: self.player.playing.as_ref()
                    .map(|p| p.to_string_lossy().to_string()),
                title: metadata.title,
                artist: metadata.artist,
                position_seconds: self.engine.playback_position_seconds(),
                duration_seconds: self.player.duration,
                paused: self.player.paused,
            },
            volume: self.settings.playback_volume,
            gain: crate::control::GainStatus {
                setting: self.settings.gain,
                effective: self.effective_gain,
                auto: self.settings.auto_gain,
            },
            kit: crate::control::KitStatus {
                enabled: self.settings.kit_enabled,
                path: self.settings.kit_path.clone(),
            },
            window: crate::control::WindowStatus {
                mini: self.is_mini,
                fullscreen: self.is_fullscreen,
            },
            vacuum: crate::control::VacuumStatus {
                file: self.settings.vacuum_enabled,
                app: self.app_vacuum.clone(),
            },
            quiet: crate::control::QuietStatus {
                render_active: self.render_loop_active,
            },
            fps: self.last_fps,
            beam_cycle: beam_cycle_animating(&self.settings)
                .then(|| crate::control::BeamCycleStatus {
                    colors: self.settings.beam_cycle_count,
                    seconds: self.settings.beam_cycle_seconds,
                    mode: self.settings.beam_cycle_mode.clone(),
                    current: cycle_beam_color_phase(
                        &self.settings, self.beam_cycle_phase()),
                }),
        };
        *control.shared.status.lock().unwrap() = snapshot;
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

    /// Decode the engine's extracted cover art into an egui texture
    /// (png/jpeg only, thumbnailed) — the transport shows it framed.
    fn load_cover_art(&mut self, path: &std::path::Path) {
        self.cover_texture = None;
        let Some(art) = self.engine.current_cover_art() else { return };
        let Some(graphics) = &self.graphics else { return };
        let Ok(decoded) = image::load_from_memory(&art.data) else { return };
        let thumb = decoded.thumbnail(96, 96).to_rgba8();
        let size = [thumb.width() as usize, thumb.height() as usize];
        let image = egui::ColorImage::from_rgba_unmultiplied(
            size, thumb.as_raw());
        let texture = graphics.egui_ctx.load_texture(
            "cover", image, egui::TextureOptions::LINEAR);
        self.cover_texture = Some((path.to_path_buf(), texture));
    }

    /// Show a transient on-scope toast now.
    pub(crate) fn toast_now(&mut self, message: impl Into<String>) {
        self.toast = Some((message.into(), Instant::now()));
        self.chrome_dirty = true;
    }

    /// Export the playing file as a .phos postcard on a worker thread.
    pub(crate) fn export_postcard(&mut self,
                                  dialog: &crate::chrome::PostcardState) {
        // credit persists (v3: the "Trace by" field writes the setting)
        self.settings.postcard_credit = dialog.credit.clone();
        self.actions.push(UiAction::SaveSettings);
        let source = dialog.source.clone();
        let title = dialog.title.clone();
        let credit = dialog.credit.clone();
        let (sender, receiver) = mpsc::channel();
        self.export_results = Some(receiver);
        self.exporting = true;
        self.toast_now("exporting postcard…");
        std::thread::spawn(move || {
            let _ = sender.send(crate::exports::export_postcard(
                &source, &title, &credit));
        });
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
            // order to minimise the Resized/Moved thrash: decorations
            // off first, then position, then ONE request_inner_size.
            // The grace stamps NOW so the burst these calls provoke is
            // read as ours, not a user corner-drag skew.
            self.mini_entering = Some(Instant::now());
            window.set_decorations(false);
            if let (Some(x), Some(y)) =
                (self.settings.mini_x, self.settings.mini_y)
            {
                window.set_outer_position(
                    winit::dpi::PhysicalPosition::new(x as i32, y as i32));
            }
            let size = self.settings.mini_size.clamp(140, 1000) as u32;
            let _ = window.request_inner_size(
                winit::dpi::PhysicalSize::new(size, size));
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

    /// The work area, served from the 30 s cache — shells out to xprop
    /// only when the cache is empty or stale. Called on the event-loop
    /// thread at every settle, so the subprocess must NOT run per tick.
    fn workarea_cached(&mut self) -> Option<(i32, i32, i32, i32)> {
        let fresh = self.workarea_cache
            .is_some_and(|(_, at)| workarea_cache_fresh(at.elapsed()));
        if !fresh
            && let Some(area) = Self::workarea()
        {
            self.workarea_cache = Some((area, Instant::now()));
        }
        self.workarea_cache.map(|(area, _)| area)
    }

    /// v3 _snap_mini_to_edges: within 32 px of a work-area edge →
    /// flush to it; position persisted.
    fn snap_mini_to_edges(&mut self, graphics: &Graphics) {
        const SNAP: i32 = 32;
        let Some((area_x, area_y, area_w, area_h)) = self.workarea_cached()
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
                (c.frame_size.0 as u32, c.frame_size.1 as u32)
            }))
            .filter(|(w, h)| *w > 0 && *h > 0)
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
                graphics.scope_cpu = Some(CpuScope::fresh());
            }
        }
        self.apply_render_settings(graphics);
    }

    /// The beam-cycle ring phase right now (see
    /// `cycle_beam_color_phase`): timer mode reads the wall clock;
    /// track mode rests on a slot and sweeps one unit per song change.
    pub(crate) fn beam_cycle_phase(&self) -> f64 {
        let leg = self.settings.beam_cycle_seconds.clamp(0.1, 60.0);
        if self.settings.beam_cycle_mode == "track" {
            let count =
                self.settings.beam_cycle_count.clamp(1, 3) as f64;
            let index = self.cycle_song_index as f64;
            match self.cycle_song_fade {
                Some(started) => {
                    let fraction =
                        (started.elapsed().as_secs_f64() / leg).min(1.0);
                    (index - 1.0).rem_euclid(count) + fraction
                }
                None => index,
            }
        } else {
            self.started.elapsed().as_secs_f64() / leg
        }
    }

    /// The theme this frame — THE one resolution every live consumer
    /// shares (surface clear, GPU renderer, raster jobs, chrome
    /// accents, probe).
    pub(crate) fn current_theme(&self) -> phosphor_beam::Theme {
        if beam_cycle_animating(&self.settings) {
            build_theme_phase(&self.settings, self.beam_cycle_phase())
        } else {
            build_theme(&self.settings)
        }
    }

    /// Does the cycle need frames RIGHT NOW? Timer mode always (the
    /// color never stops moving); track mode only while a song-change
    /// crossfade is in flight — between songs the color rests and the
    /// quiet law applies unmodified.
    pub(crate) fn beam_cycle_needs_frames(&self) -> bool {
        if !beam_cycle_animating(&self.settings) {
            return false;
        }
        if self.settings.beam_cycle_mode == "track" {
            let leg = self.settings.beam_cycle_seconds.clamp(0.1, 60.0);
            return self.cycle_song_fade
                .is_some_and(|started| {
                    started.elapsed().as_secs_f64() < leg
                });
        }
        true
    }

    /// A new song began (own player track start, or the scoped
    /// external player changed track): in track mode, advance the
    /// ring one slot and start the crossfade.
    pub(crate) fn beam_cycle_song_changed(&mut self) {
        if self.settings.beam_cycle_mode != "track"
            || !beam_cycle_animating(&self.settings)
        {
            return;
        }
        let count = self.settings.beam_cycle_count.clamp(1, 3) as usize;
        self.cycle_song_index = (self.cycle_song_index + 1) % count;
        self.cycle_song_fade = Some(Instant::now());
        self.wake_render_loop();
    }

    fn apply_render_settings(&self, graphics: &mut Graphics) {
        let theme = build_theme(&self.settings);
        let grid_fraction =
            phosphor_beam::grid_spacing_fraction(self.settings.gain);
        // σ display-scale law: on-screen beam width = beam_focus
        // logical px at any DPI (v3 parity — v4 traces physical px)
        let scale = graphics.window.scale_factor() as f32;
        if let Some(gpu) = graphics.scope_gpu.as_mut() {
            gpu.beam_focus = self.settings.beam_focus;
            gpu.persistence = self.settings.persistence;
            gpu.theme = theme;
            gpu.grid_enabled = self.settings.grid_enabled;
            gpu.grid_spacing_fraction = grid_fraction;
            gpu.display_scale = scale;
        }
        // CPU path: nothing to push — every RasterJob carries the full
        // settings snapshot, so the worker is always current.
    }

    /// Autosize: scale the trace to fill the screen (v3
    /// _update_auto_gain, constants verbatim). The tracked peak
    /// attacks instantly (nothing clips off-screen) and releases
    /// slowly, and the applied gain glides so the picture breathes
    /// rather than jumping between loud and quiet passages.
    fn update_auto_gain(&mut self, peak: f32) {
        if !self.settings.auto_gain {
            return;
        }
        self.auto_gain_peak = peak.max(self.auto_gain_peak * 0.999);
        let target = (0.92 / self.auto_gain_peak.max(0.01))
            .clamp(0.1, 6.0);
        self.effective_gain +=
            (target - self.effective_gain) * 0.05;
        self.computer.gain = self.effective_gain;
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
            self.sync_beam_source(Some(combo_id));
        } else {
            self.status_line = "no capture target".into();
            self.sync_beam_source(None);
        }
    }

    /// The external player the beam is scoping right now, if any —
    /// app capture matches by key; whole-output capture takes whoever
    /// is Playing (v3's watcher law, now with hands).
    pub(crate) fn linked_external_player(
        &self) -> Option<crate::mpris_client::ExternalPlayer>
    {
        let client = self.mpris_client.as_ref()?;
        let players = client.players.lock().unwrap();
        let app_key = match &self.beam_source {
            BeamSource::Capture { combo_id } =>
                combo_id.strip_prefix("app:"),
            BeamSource::Mix { .. } => None,
            // the built-in player owns the transport; silent scopes
            // nothing
            BeamSource::Player { .. } | BeamSource::Silent => {
                return None;
            }
        };
        crate::mpris_client::linked_player(&players, app_key)
    }

    /// Re-derive `beam_source` from live session state: capture (with
    /// its combo id) outranks the player session; a loaded track counts
    /// as `Player` playing OR paused; else silent. Call after ANY
    /// transition that touched `capture_on` or `player.playing`.
    pub(crate) fn sync_beam_source(&mut self, fresh_capture_id: Option<String>) {
        self.beam_source = if self.capture_on {
            let combo_id = fresh_capture_id.or_else(|| {
                match &self.beam_source {
                    BeamSource::Capture { combo_id } =>
                        Some(combo_id.clone()),
                    BeamSource::Mix { .. } => None, // mix keeps itself
                    _ => self.settings.target_id.clone(),
                }
            });
            match (combo_id, &self.beam_source) {
                (_, BeamSource::Mix { combo_ids }) =>
                    BeamSource::Mix { combo_ids: combo_ids.clone() },
                (Some(combo_id), _) => BeamSource::Capture { combo_id },
                (None, _) => BeamSource::Silent,
            }
        } else if let Some(path) = self.player.playing.clone() {
            BeamSource::Player { path }
        } else {
            BeamSource::Silent
        };
    }

    /// p99 of the recorded frame-work times (0 until enough frames).
    fn work_p99_ms(&self) -> f32 {
        let filled = self.work_ms_count.min(self.work_ms_ring.len());
        if filled < 8 {
            return 0.0;
        }
        let mut window: Vec<f32> =
            self.work_ms_ring[..filled].to_vec();
        window.sort_by(|a, b| a.total_cmp(b));
        window[((filled as f32 * 0.99) as usize).min(filled - 1)]
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
        // dropped-frame receipt: a gap past 1.5× the pacing period
        // while the loop was actively rendering is a missed frame
        if self.render_loop_active
            && let Some(previous) = self.last_frame_time
        {
            let period_hz = self.cap_hz();
            if period_hz > 0.0 {
                let gap = now.duration_since(previous).as_secs_f64();
                if gap > 1.5 / period_hz {
                    self.dropped_frames =
                        self.dropped_frames.saturating_add(1);
                }
            }
        }
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
                    self.sync_beam_source(None);
                    self.chrome_dirty = true;
                }
                AudioEvent::TargetsChanged
                | AudioEvent::DefaultSinkChanged => {
                    targets_dirty = true;
                }
                AudioEvent::PlaybackEnded => {
                    if self.composing {
                        // the drawn loop repeats forever; if its
                        // decoder dies it was killed externally —
                        // just invite another stroke (v3 law)
                        self.status_line =
                            "✏ loop stopped — draw to start again"
                                .into();
                        self.chrome_dirty = true;
                        continue;
                    }
                    self.handle_track_finished();
                }
                AudioEvent::TrackStarted { path } => {
                    if self.composing {
                        // the compose loop is not a track: no player
                        // bookkeeping, no overlay, no MPRIS
                        self.chrome_dirty = true;
                        continue;
                    }
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
                    let basename = path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    // Track state lives ONLY in the transport row now
                    // (Ben: "two places say when a track is playing —
                    // consolidate"). The toolbar status stays for
                    // capture/system notes; clear it so it isn't stale.
                    self.status_line.clear();
                    if self.settings.show_now_playing {
                        let title = metadata.title.clone()
                            .unwrap_or_else(|| basename.clone());
                        // .phos: subtitle is "trace by <credit>" —
                        // the postcard credit fade (v3 law). Certain
                        // artists get a nod (undocumented, v3 table).
                        // Art + track + artist — album (Ben's card).
                        let mut subtitle = match (&metadata.artist,
                                                  &metadata.album) {
                            (Some(artist), Some(album))
                                if !album.is_empty() =>
                                Some(format!("{artist} — {album}")),
                            (artist, _) => artist.clone(),
                        };
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
                    // change-color-on-song: every real track start
                    // counts (manual, next/prev, gapless splice) —
                    // compose loops bailed out above
                    self.beam_cycle_song_changed();
                    self.queue_gapless_next();
                    self.mpris_track_changed();
                    self.load_cover_art(&path);
                    // the overlay wears the cover too (left of the
                    // title) — None when the track carries no art so
                    // the previous track's art never lingers
                    self.overlay_art = self.cover_texture.as_ref()
                        .filter(|(source, _)| *source == path)
                        .map(|(_, texture)| texture.clone());
                    // the systemwide toast with the album art (Ben's
                    // ask) — embedded cover written to the runtime
                    // dir for the image-path hint
                    if self.settings.track_notifications {
                        let art_path = self.engine.current_cover_art()
                            .and_then(|art| {
                                let path =
                                    crate::notify::own_art_path();
                                std::fs::create_dir_all(
                                    path.parent()?).ok()?;
                                std::fs::write(&path, &art.data)
                                    .ok()?;
                                Some(path)
                            });
                        let title = metadata.title.clone()
                            .unwrap_or_else(|| basename.clone());
                        self.notification_id =
                            crate::notify::notify_track_with_file(
                                &title,
                                metadata.artist.as_deref()
                                    .unwrap_or(""),
                                "Phosphor",
                                art_path.as_deref(),
                                self.notification_id);
                    }
                    self.chrome_dirty = true;
                }
            }
        }
        if targets_dirty {
            self.refresh_target_cache();
            self.chrome_dirty = true;
        }

        // ---- external now-playing: the corner overlay follows the
        // linked player's track changes (the v3 watcher law — Ben:
        // "name of song didn't show when the spotify song changed") ----
        if let Some(external) = self.linked_external_player() {
            let signature = external.title.as_deref().map(|title| {
                format!("{title}|{}",
                        external.artist.as_deref().unwrap_or(""))
            });
            if signature.is_some()
                && signature != self.last_external_signature
            {
                let was_first = self.last_external_signature.is_none();
                self.last_external_signature = signature;
                if !was_first {
                    // the scoped player moved to a new song — the
                    // color advances even when the overlay card is
                    // disabled (the cycle is not a notification)
                    self.beam_cycle_song_changed();
                }
                // announce CHANGES while listening, not the state we
                // walked in on (mirrors the notification law)
                if !was_first && self.settings.show_now_playing
                    && let Some(title) = &external.title
                {
                    // art + track + artist — album · via player
                    // (Ben's spec for the corner card)
                    let mut parts: Vec<String> = Vec::new();
                    if let Some(artist) = &external.artist {
                        parts.push(artist.clone());
                    }
                    if let Some(album) = &external.album
                        && !album.is_empty()
                    {
                        parts.push(album.clone());
                    }
                    let mut line = parts.join(" — ");
                    if line.is_empty() {
                        line = format!("via {}", external.identity);
                    } else {
                        line.push_str(&format!(
                            "  ·  via {}", external.identity));
                    }
                    let subtitle = Some(line);
                    // the cached art (fetched on the client thread)
                    // becomes the overlay thumbnail — decode is a
                    // few ms once per track change
                    self.overlay_art = external.art_local.as_deref()
                        .and_then(|art_path| {
                            let graphics = self.graphics.as_ref()?;
                            let bytes = std::fs::read(art_path).ok()?;
                            let decoded =
                                image::load_from_memory(&bytes).ok()?;
                            let thumb =
                                decoded.thumbnail(96, 96).to_rgba8();
                            let size = [thumb.width() as usize,
                                        thumb.height() as usize];
                            Some(graphics.egui_ctx.load_texture(
                                "overlay-art",
                                egui::ColorImage::from_rgba_unmultiplied(
                                    size, &thumb),
                                Default::default()))
                        });
                    self.player.flash_now_playing(
                        title, subtitle.as_deref());
                    self.chrome_dirty = true;
                    // a change can land while the scope sleeps (paused
                    // player, quiet capture) — without a wake the
                    // flash would never paint in ANY view, and mini/
                    // fullscreen live in exactly those quiet corners
                    self.wake_render_loop();
                }
            }
        } else {
            self.last_external_signature = None;
        }

        // ---- MPRIS: media keys arrive as Player method calls ----
        self.service_mpris();
        // ---- control socket: CLI status/ctl/tap over the Unix socket ----
        self.service_control();

        // ---- export results ----
        if let Some(receiver) = &self.export_results
            && let Ok(result) = receiver.try_recv()
        {
            self.export_results = None;
            self.exporting = false;
            // answer a deferred control snapshot/clip request, if any
            if let Some((kind, reply)) = self.control_export_reply.take() {
                let value = match &result {
                    Ok(path) => serde_json::json!({
                        "status": "ok", "verb": kind,
                        "result": {"path": path.to_string_lossy()},
                    }),
                    Err(error) => serde_json::json!({
                        "status": "error", "error": error,
                        "fix": "check ~/Pictures/Phosphor is writable",
                    }),
                };
                let _ = reply.send(value);
            }
            let message = match result {
                Ok(path) => {
                    let name = path.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    format!("saved {name}")
                }
                Err(error) => error,
            };
            self.toast = Some((message, Instant::now()));
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

        // ---- compose: debounced scroll-retune regenerates the loop ----
        self.service_compose_retune();

        // ---- samples + quiet law (visitor overrides the sleep; a
        // stroke in progress previews as segments and drains any
        // still-playing loop audio so it can't burst in later) ----
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
            self.update_auto_gain(peak);
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
        } || visitor_active || self.compose_drawing
            // an animating beam cycle is an animation: the clear
            // color, grid tint, and beam hue move even in silence, so
            // the loop keeps pacing (the user opted in by adding
            // colors; frames stay capped and near-empty when quiet).
            // Track mode only needs frames DURING a song-change fade.
            || self.beam_cycle_needs_frames();

        let Some(mut graphics) = self.graphics.take() else {
            return false;
        };
        let work_started = Instant::now();
        let keep_going = self.frame(&mut graphics, &samples, advancing);
        if advancing {
            let work_ms = work_started.elapsed().as_secs_f32() * 1e3;
            self.last_work_ms = work_ms;
            self.work_ms_ring[self.work_ms_count
                              % self.work_ms_ring.len()] = work_ms;
            self.work_ms_count += 1;
            // dip log: a frame past 2× the pacing period, named
            let period_hz = self.cap_hz();
            if self.args.fps_log && period_hz > 0.0
                && work_ms as f64 > 2000.0 / period_hz
            {
                eprintln!(
                    "{}",
                    serde_json::json!({
                        "dip": true,
                        "t": (self.started.elapsed().as_secs_f64()
                              * 10.0).round() / 10.0,
                        "work_ms": (work_ms * 100.0).round() / 100.0,
                        "gpu_ms": graphics.scope_gpu.as_ref()
                            .and_then(|g| g.gpu_frame_ms()),
                        "segments": self.last_segment_count,
                    })
                );
            }
        }
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
                        "work_ms": (self.last_work_ms * 100.0)
                            .round() / 100.0,
                        "p99_ms": (self.work_p99_ms() * 100.0)
                            .round() / 100.0,
                        "gpu_ms": self.graphics.as_ref()
                            .and_then(|g| g.scope_gpu.as_ref())
                            .and_then(|g| g.gpu_frame_ms()),
                        "drops": self.dropped_frames,
                    })
                );
            }
            self.segments_per_second =
                self.segments_window as f64 / window_elapsed.as_secs_f64();
            self.segments_window = 0;
            self.fps_frames = 0;
            self.fps_window_start = Instant::now();
        }

        // ---- control socket: refresh the status the CLI reads ----
        self.update_control_snapshot();
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
            // the graticule tracks gain (volts/div); re-derive it only
            // on real movement so auto-gain's per-frame glides and the
            // wheel's small steps stay free (v3: 2% threshold)
            let live_gain = if self.settings.auto_gain {
                self.effective_gain
            } else {
                self.settings.gain
            };
            if (live_gain - self.grid_gain).abs()
                > self.grid_gain.abs() * 0.02
            {
                self.grid_gain = live_gain;
                let fraction =
                    phosphor_beam::grid_spacing_fraction(live_gain);
                if let Some(gpu) = graphics.scope_gpu.as_mut() {
                    gpu.grid_spacing_fraction = fraction;
                }
                // CPU path: the next RasterJob carries it (grid_gain)
            }
            let mut segments: Vec<[f32; 5]> = if self.compose_drawing {
                // the stroke in progress previews directly as
                // segments (the audio was already drained this tick)
                self.compose_preview_segments(
                    trace_w as f32, trace_h as f32)
            } else {
                self.computer.compute(
                    samples, trace_w as f32, trace_h as f32)
                    .to_vec()
            };
            // the visitor swims OVER whatever the audio draws
            if let Some(started) = self.visitor_started {
                segments.extend(crate::exports::visitor_segments(
                    started.elapsed().as_secs_f64(),
                    trace_w as f32, trace_h as f32));
            }
            let segments = &segments[..];
            self.last_segment_count = segments.len();
            self.segments_window += segments.len();
            // ---- tap: broadcast a cheap frame observation to any
            // `phosphor tap` subscribers (skip everything when none) ----
            if let Some(control) = &self.control {
                let mut taps = control.shared.taps.lock().unwrap();
                if !taps.is_empty() {
                    let peak = samples
                        .iter()
                        .fold(0.0f32, |peak, s| peak.max(s.abs()));
                    let ts_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis())
                        .unwrap_or(0);
                    let event = crate::control::build_frame_event(
                        segments, self.computer.mode.name(), peak,
                        trace_w as f32, trace_h as f32, ts_ms);
                    // prune subscribers whose pipe has closed
                    taps.retain(|sender| sender
                        .send(crate::control::TapEvent::Frame(event.clone()))
                        .is_ok());
                }
            }
            if let Some(gpu) = graphics.scope_gpu.as_mut() {
                if let Err(error) = gpu.resize(scope_width, scope_height) {
                    eprintln!("phosphor: scope resize: {error}");
                }
                gpu.advance(segments);
            }
            if let Some(cpu) = graphics.scope_cpu.as_mut() {
                // v3's CPU resolution law lives in trace_w/h (scope ×
                // fraction). The job snapshot makes the worker current
                // without any cross-thread settings plumbing.
                let fraction =
                    self.settings.cairo_resolution.clamp(0.25, 1.0);
                cpu.worker.submit(crate::raster_worker::RasterJob {
                    segments: segments.to_vec(),
                    width: trace_w as usize,
                    height: trace_h as usize,
                    beam_focus: self.settings.beam_focus,
                    persistence: self.settings.persistence,
                    theme: self.current_theme(),
                    grid_enabled: self.settings.grid_enabled,
                    grid_spacing_fraction:
                        phosphor_beam::grid_spacing_fraction(
                            self.grid_gain),
                    display_scale: pixels_per_point * fraction,
                });
            }
        }

        // CPU path: upload the newest PUBLISHED frame (the worker owns
        // the raster; a slow raster no longer drags the chrome — #5)
        if let Some(cpu) = graphics.scope_cpu.as_mut()
            && let Some(frame) = cpu.worker.take_frame()
        {
            cpu.frame_size = (frame.width, frame.height);
            cpu.raster_ms = Some(frame.raster_ms);
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [frame.width, frame.height], &frame.pixels);
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
        // HUD numbers snapshot (the egui closure can't reach graphics)
        let graphics_scope_gpu_ms = graphics.scope_gpu.as_ref()
            .and_then(|g| g.gpu_frame_ms());
        let cpu_raster_ms = graphics.scope_cpu.as_ref()
            .and_then(|c| c.raster_ms);
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
            }
            // the playlist opens EVERYWHERE (L) — floating window in
            // mini/fullscreen, docked panel otherwise
            self.ui_playlist_panel(ctx);
            // dialogs float above chrome, hidden only in fullscreen
            if !self.is_fullscreen {
                self.ui_kit_editor(ctx);
                self.ui_postcard_dialog(ctx);
                self.ui_manual_window(ctx);
                self.ui_mix_panel(ctx);
            }
            // the photosensitivity prompt outranks the fullscreen
            // hide: a safety question is never deferred
            self.ui_epilepsy_prompt(ctx);
            // (the bottom status bar is gone — track state lives in the
            //  transport row, fps is a scope overlay, transient notes
            //  are on-scope toasts; consolidation per the design pass)
            let central = egui::CentralPanel::default()
                .frame(egui::Frame::NONE);
            central.show(ctx, |ui| {
                scope_rect_out = ui.max_rect();
                let scope_response = ui.interact(
                    scope_rect_out, ui.id().with("scope"),
                    egui::Sense::click_and_drag());
                self.scope_hovered = scope_response.hovered();
                // left-press outside an open menu dismisses it — the
                // normal-view behavior, made to work in fullscreen and
                // mini too (Ben's patch list). "Outside" = the press
                // landed on a layer BELOW Order::Foreground: the menu
                // and every submenu are Foreground popups, so item
                // presses never count. (BUGLOG #1: testing a hovered
                // flag measured via ui_contains_pointer at the TOP of
                // the menu closure reads an empty min_rect → always
                // false → every item press closed the menu before its
                // release could land — "menu items don't work,
                // hotkeys do".)
                if self.context_menu_open
                    && ui.ctx().input(|i| i.pointer.primary_pressed())
                {
                    // press_origin is None when the release arrived in
                    // the same input batch (a fast click) — fall back
                    // to interact_pos; the pointer hasn't moved between
                    // the two within one frame in any way that matters
                    let press_on_menu = ui.ctx()
                        .input(|i| i.pointer.press_origin()
                                    .or(i.pointer.interact_pos()))
                        .and_then(|pos| ui.ctx().layer_id_at(pos))
                        .is_some_and(|layer| {
                            layer.order == egui::Order::Foreground
                        });
                    if !press_on_menu {
                        self.close_menu_request = true;
                    }
                }
                self.ui_context_menu(&scope_response);
                if scope_response.double_clicked() && self.is_mini {
                    self.actions.push(UiAction::MiniToggle);
                }
                // compose: the pointer draws (desktop only, like v3);
                // compose forces XY so orbit-drag can't collide
                if self.composing && !self.is_mini {
                    self.compose_pointer(ui, &scope_response);
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
                // — with the album art left of the title (the last
                // missing polish), fading on the same curve
                if let Some((title, subtitle, opacity)) =
                    self.player.overlay_visible()
                {
                    let alpha = (opacity * 255.0) as u8;
                    let mut position = scope_rect_out.min
                        + egui::vec2(12.0, 12.0);
                    let painter = ui.painter();
                    if let Some(art) = &self.overlay_art {
                        let side = 44.0;
                        let art_rect = egui::Rect::from_min_size(
                            position, egui::vec2(side, side));
                        painter.image(
                            art.id(), art_rect,
                            egui::Rect::from_min_max(
                                egui::pos2(0.0, 0.0),
                                egui::pos2(1.0, 1.0)),
                            egui::Color32::from_white_alpha(alpha));
                        painter.rect_stroke(
                            art_rect, 0.0,
                            egui::Stroke::new(
                                1.0,
                                self.active_palette.line
                                    .gamma_multiply(opacity)),
                            egui::StrokeKind::Inside);
                        position.x += side + 10.0;
                    }
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
                // no-signal hint: capture is live but the target has
                // been silent past the sleep window — a dark scope is
                // now a LABELED state, never a mystery black screen
                if matches!(self.beam_source,
                            BeamSource::Capture { .. }
                            | BeamSource::Mix { .. })
                    && self.quiet_frame_count > QUIET_FRAMES_BEFORE_SLEEP
                {
                    let label = self.beam_source.combo_label(|id| {
                        self.target_cache.iter()
                            .find(|t| t.combo_id() == id)
                            .map(|t| t.label.clone())
                    });
                    let anchor = egui::pos2(
                        scope_rect_out.center().x,
                        scope_rect_out.max.y - 18.0);
                    ui.painter().text(
                        anchor, egui::Align2::CENTER_BOTTOM,
                        format!("no signal · {label}"),
                        egui::FontId::monospace(12.0),
                        self.active_palette.muted);
                    // the resting beam: a real CRT never shows
                    // nothing — with no deflection the electron beam
                    // sits at dead center as a small dot. Painted
                    // chrome-side (the engine is asleep; this costs
                    // zero frames), it settles in over ~a second via
                    // egui's one-shot animation and then holds still.
                    let settle = ui.ctx().animate_bool_with_time(
                        egui::Id::new("resting-beam"), true, 1.1);
                    let beam = self.current_theme().beam_color;
                    let center = scope_rect_out.center();
                    let paint_dot = |radius: f32, alpha: f32| {
                        ui.painter().circle_filled(
                            center, radius,
                            egui::Color32::from_rgba_unmultiplied(
                                (beam[0] * 255.0) as u8,
                                (beam[1] * 255.0) as u8,
                                (beam[2] * 255.0) as u8,
                                (alpha * settle) as u8));
                    };
                    paint_dot(9.0, 14.0);  // far halo
                    paint_dot(5.0, 46.0);  // glow
                    paint_dot(2.4, 210.0); // the beam itself
                } else {
                    // re-arm the settle so the dot fades in fresh the
                    // next time the scope goes quiet
                    ui.ctx().animate_bool_with_time(
                        egui::Id::new("resting-beam"), false, 0.2);
                }
                // fps overlay: top-right of the scope, mono, all modes
                // (mini + fullscreen included — Ben's requested home)
                if self.settings.show_fps {
                    // fps plate: smaller type, ACCENT digits on a
                    // surface plate (ink-on-plate was invisible over a
                    // hot beam — Ben's "same color, no delineation"),
                    // and F cycles fps → nerd HUD → off
                    let mut lines =
                        vec![format!("{:.0} fps", self.last_fps)];
                    if self.settings.show_fps_detail {
                        lines.push(format!(
                            "cpu {:>5.2} ms · p99 {:>5.2}",
                            self.last_work_ms, self.work_p99_ms()));
                        let gpu_ms = graphics_scope_gpu_ms;
                        if let Some(gpu_ms) = gpu_ms {
                            lines.push(format!(
                                "gpu {gpu_ms:>5.2} ms · beam"));
                        }
                        if let Some(raster_ms) = cpu_raster_ms {
                            lines.push(format!(
                                "raster {raster_ms:>5.2} ms · worker"));
                        }
                        lines.push(format!(
                            "{:.0}k seg/s · {} kHz · {}",
                            self.segments_per_second / 1000.0,
                            self.settings.scope_sample_rate / 1000,
                            if cpu_texture_id.is_some() { "cpu" }
                            else { "gpu" }));
                        lines.push(format!(
                            "{}×{} @{:.2} · {} drops",
                            trace_w, trace_h, pixels_per_point,
                            self.dropped_frames));
                    }
                    let font = egui::FontId::monospace(11.0);
                    let anchor = egui::pos2(
                        scope_rect_out.max.x - 10.0,
                        scope_rect_out.min.y + 8.0);
                    let painter = ui.painter();
                    let galleys: Vec<_> = lines.iter().map(|line| {
                        painter.layout_no_wrap(
                            line.clone(), font.clone(),
                            self.active_palette.accent)
                    }).collect();
                    let width = galleys.iter()
                        .map(|g| g.size().x)
                        .fold(0.0f32, f32::max);
                    let line_height = galleys.first()
                        .map(|g| g.size().y).unwrap_or(12.0);
                    let height =
                        line_height * galleys.len() as f32
                        + 2.0 * (galleys.len().saturating_sub(1)) as f32;
                    let box_rect = egui::Rect::from_min_size(
                        egui::pos2(anchor.x - width - 6.0,
                                   anchor.y - 2.0),
                        egui::vec2(width + 12.0, height + 4.0));
                    painter.rect_filled(box_rect, 0.0,
                        self.active_palette.surface.gamma_multiply(0.82));
                    painter.rect_stroke(box_rect, 0.0,
                        egui::Stroke::new(1.0, self.active_palette.line),
                        egui::StrokeKind::Inside);
                    let mut y = anchor.y;
                    for line in &lines {
                        painter.text(
                            egui::pos2(anchor.x, y),
                            egui::Align2::RIGHT_TOP, line.clone(),
                            font.clone(),
                            self.active_palette.accent);
                        y += line_height + 2.0;
                    }
                }
                // transient toast: bottom-center, sharp hairline frame,
                // fades over ~2.5 s (snapshot saved, vacuum notes…)
                if let Some((message, shown)) = self.toast.clone() {
                    let age = shown.elapsed().as_secs_f32();
                    if age < 2.5 {
                        let opacity =
                            (1.0 - (age - 1.8).max(0.0) / 0.7).clamp(0.0, 1.0);
                        let painter = ui.painter();
                        let galley = painter.layout_no_wrap(
                            message.clone(), egui::FontId::monospace(12.0),
                            self.active_palette.ink);
                        let center = egui::pos2(
                            scope_rect_out.center().x,
                            scope_rect_out.max.y - 30.0);
                        let box_rect = egui::Rect::from_center_size(
                            center,
                            galley.size() + egui::vec2(18.0, 10.0));
                        let alpha = (opacity * 255.0) as u8;
                        painter.rect_filled(box_rect, 0.0, with_toast_alpha(
                            self.active_palette.surface, alpha));
                        painter.rect_stroke(box_rect, 0.0,
                            egui::Stroke::new(1.0, with_toast_alpha(
                                self.active_palette.line_strong, alpha)),
                            egui::StrokeKind::Inside);
                        painter.text(center, egui::Align2::CENTER_CENTER,
                            message, egui::FontId::monospace(12.0),
                            with_toast_alpha(self.active_palette.ink, alpha));
                        ui.ctx().request_repaint();
                    } else {
                        self.toast = None;
                    }
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
        // The theme resolves PER FRAME here: the beam color cycle
        // (v4.1) animates the Custom beam — timer mode on the wall
        // clock, track mode per song (v4.2) — so the clear color, the
        // GPU renderer, and the raster jobs all read the same instant.
        let theme = self.current_theme();
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
            gpu.theme = theme; // the cycle's per-frame color (v4.1)
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

impl ApplicationHandler<()> for Shell {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_none() {
            self.init_graphics(event_loop);
        }
    }

    /// The control socket's wake: a queued request is waiting. Request a
    /// redraw so the next tick drains it (mirrors wake_render_loop's
    /// window poke, without disturbing the quiet-law counters).
    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {
        if let Some(graphics) = &self.graphics {
            graphics.window.request_redraw();
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
                    let entering = self.mini_entering.is_some_and(|t| {
                        t.elapsed() < Duration::from_millis(400)
                    });
                    let side = size.width.max(size.height)
                        .clamp(140, 1000) as i64;
                    // a skew only counts as a user corner-drag when it
                    // is NOT part of the set_mini_mode entry burst —
                    // otherwise our own request_inner_size loops
                    if size.width != size.height && !entering {
                        self.mini_resquare = Some(side);
                    }
                    self.settings.mini_size = side;
                    self.mini_settle = Some(
                        Instant::now() + Duration::from_millis(180));
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_position = (position.x, position.y);
                // hover hint: advertise the mini resize zones so the
                // edges/corners are discoverable (Default in the move
                // interior). Cheap — only while mini.
                if self.is_mini
                    && let Some(graphics) = &self.graphics
                {
                    let size = graphics.window.inner_size();
                    let icon = mini_resize_zone(
                        position.x, position.y,
                        size.width as f64, size.height as f64,
                    )
                    .map(resize_cursor)
                    .unwrap_or(winit::window::CursorIcon::Default);
                    graphics.window.set_cursor(icon);
                }
            }
            WindowEvent::MouseInput {
                state: winit::event::ElementState::Pressed,
                button: winit::event::MouseButton::Left, ..
            } if self.composing && !self.is_mini => {
                // the render loop may be asleep between strokes; a
                // press must wake it or egui never sees the drag
                // start (v3: press itself started the render loop)
                self.wake_render_loop();
            }
            WindowEvent::MouseInput {
                state: winit::event::ElementState::Pressed,
                button: winit::event::MouseButton::Left, ..
            } if self.is_mini => {
                // With the context menu open, a left-press OUTSIDE the
                // menu dismisses it — and never starts a WM drag (the
                // WM grab used to swallow the release egui needed). A
                // press ON the menu (Foreground layer: menu + submenus)
                // belongs to egui — starting a drag OR dismissing here
                // ate every item click in mini (BUGLOG #1).
                if self.context_menu_open {
                    let on_menu = self.graphics.as_ref()
                        .is_some_and(|graphics| {
                            let ppp = graphics.egui_ctx
                                .pixels_per_point();
                            let (x, y) = self.cursor_position;
                            let pos = egui::pos2(x as f32 / ppp,
                                                 y as f32 / ppp);
                            graphics.egui_ctx.layer_id_at(pos)
                                .is_some_and(|layer| {
                                    layer.order
                                        == egui::Order::Foreground
                                })
                        });
                    if !on_menu {
                        self.close_menu_request = true;
                    }
                    self.wake_render_loop();
                    return;
                }
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
                        // full 8-zone hit-test: corners/edges → WM
                        // resize, interior → WM move (drag never orbits)
                        match mini_resize_zone(x, y, size.width as f64,
                                               size.height as f64) {
                            Some(dir) => {
                                let _ = graphics.window
                                    .drag_resize_window(dir);
                            }
                            None => {
                                let _ = graphics.window.drag_window();
                            }
                        }
                        // a WM drag is now live: defer square-enforcing
                        // re-squares until it ends (kills the mid-drag
                        // resize→resquare→Resized feedback loop)
                        self.mini_drag_active = true;
                    }
                }
            }
            WindowEvent::MouseInput {
                state: winit::event::ElementState::Released,
                button: winit::event::MouseButton::Left, ..
            } if self.is_mini => {
                // the WM drag grab ends on button-up (winit gives no
                // dedicated drag-end event); clear the defer flag and
                // re-arm the settle so ONE re-square + ONE snap lands
                if self.mini_drag_active {
                    self.mini_drag_active = false;
                    self.mini_settle = Some(
                        Instant::now() + Duration::from_millis(180));
                }
            }
            WindowEvent::CursorLeft { .. } if self.is_mini => {
                // pointer left the window: any WM drag is over, and the
                // resize hint no longer applies
                self.mini_drag_active = false;
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
                // scope-wheel gate: the scope widget's own hover state
                // (occlusion-aware, from the last egui pass). NOT
                // wants_pointer_input() — the CentralPanel is an egui
                // area, so that was true over the bare scope and every
                // scope-wheel behavior silently died.
                if self.scope_hovered {
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
                    } else if self.composing && !self.is_mini {
                        // scroll while composing = pitch (v3: ×1.06
                        // per notch, 20–400 Hz, regenerate debounced)
                        self.retune_compose(notches);
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
            WindowEvent::Moved(position) => {
                if self.is_mini {
                    // magnetism: settle 180 ms after the LAST move
                    self.mini_settle = Some(
                        Instant::now() + Duration::from_millis(180));
                } else if !self.is_fullscreen {
                    // persist the normal-window position (v3 law: not
                    // while tiled/maximized/mini) — restored at launch
                    let maximized = self.graphics.as_ref()
                        .is_some_and(|g| g.window.is_maximized());
                    if !maximized {
                        self.settings.window_x = Some(position.x as i64);
                        self.settings.window_y = Some(position.y as i64);
                    }
                }
            }
            WindowEvent::DroppedFile(path) => {
                // whole-window drop target (v3 §1.5): .phoskit files
                // validate → install → activate; audio files become
                // the playlist verbatim and the first one plays
                let lower = path.to_string_lossy().to_lowercase();
                if lower.ends_with(".phoskit") {
                    self.import_kit_file(&path);
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
            if self.context_menu_open {
                // never re-square/snap under an OPEN menu — the window
                // sliding out from beneath it was the mini right-click
                // glitch. Defer until the menu closes.
                self.mini_settle = Some(
                    Instant::now() + Duration::from_millis(180));
            } else if self.mini_drag_active {
                // A WM drag is still marked live and no button-release
                // or cursor-leave cleared it (winit gives no drag-end
                // event, and some WMs swallow the release). Re-squaring
                // now would fight the WM's active grab, so DON'T — clear
                // the flag (belt-and-braces) and defer the single
                // re-square to the next quiet settle.
                self.mini_drag_active = false;
                self.mini_settle = Some(
                    Instant::now() + Duration::from_millis(180));
            } else {
                self.mini_settle = None;
                if let Some(graphics) = self.graphics.take() {
                    // square first (a corner/edge drag may have skewed
                    // it), then snap to the work-area edges — ONE of
                    // each, now that the drag has truly ended
                    if let Some(side) = self.mini_resquare.take() {
                        let _ = graphics.window.request_inner_size(
                            winit::dpi::PhysicalSize::new(
                                side as u32, side as u32));
                    }
                    self.snap_mini_to_edges(&graphics);
                    self.graphics = Some(graphics);
                }
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

/// Decode the embedded 4-panel scope icon for the window title bar /
/// taskbar. None on any decode error (an icon is never load-bearing).
fn load_window_icon() -> Option<winit::window::Icon> {
    let bytes = include_bytes!("../assets/icon-64.png");
    let decoder = png::Decoder::new(std::io::Cursor::new(&bytes[..]));
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    buffer.truncate(info.buffer_size());
    // The asset is RGBA8, but a re-export once flattened it to RGB and
    // the icon silently vanished from the window list — accept both.
    let rgba = match info.color_type {
        png::ColorType::Rgba => buffer,
        png::ColorType::Rgb => buffer
            .chunks_exact(3)
            .flat_map(|px| [px[0], px[1], px[2], 255])
            .collect(),
        _ => return None,
    };
    winit::window::Icon::from_rgba(rgba, info.width, info.height).ok()
}

/// Mini resize hit-test (PURE). Corners win a 26 px box at each corner;
/// edges take an 8 px margin; the interior is a move zone → None.
/// Corners take precedence over edges (a corner box overlaps two edge
/// margins). Coordinates are physical pixels inside the mini window.
fn mini_resize_zone(x: f64, y: f64, width: f64, height: f64)
    -> Option<winit::window::ResizeDirection> {
    use winit::window::ResizeDirection;
    const CORNER: f64 = 26.0;
    const EDGE: f64 = 8.0;
    let left = x <= CORNER;
    let right = x >= width - CORNER;
    let top = y <= CORNER;
    let bottom = y >= height - CORNER;
    // corners first (they overlap the edge margins)
    if top && left {
        return Some(ResizeDirection::NorthWest);
    }
    if top && right {
        return Some(ResizeDirection::NorthEast);
    }
    if bottom && left {
        return Some(ResizeDirection::SouthWest);
    }
    if bottom && right {
        return Some(ResizeDirection::SouthEast);
    }
    // then edges
    if x <= EDGE {
        return Some(ResizeDirection::West);
    }
    if x >= width - EDGE {
        return Some(ResizeDirection::East);
    }
    if y <= EDGE {
        return Some(ResizeDirection::North);
    }
    if y >= height - EDGE {
        return Some(ResizeDirection::South);
    }
    None
}

/// The winit cursor icon that advertises a resize zone (hover hint).
fn resize_cursor(dir: winit::window::ResizeDirection)
    -> winit::window::CursorIcon {
    use winit::window::{CursorIcon, ResizeDirection};
    match dir {
        ResizeDirection::North => CursorIcon::NResize,
        ResizeDirection::South => CursorIcon::SResize,
        ResizeDirection::East => CursorIcon::EResize,
        ResizeDirection::West => CursorIcon::WResize,
        ResizeDirection::NorthEast => CursorIcon::NeResize,
        ResizeDirection::NorthWest => CursorIcon::NwResize,
        ResizeDirection::SouthEast => CursorIcon::SeResize,
        ResizeDirection::SouthWest => CursorIcon::SwResize,
    }
}

/// Work-area cache TTL decision (PURE): true → the cached value is
/// still usable, false → re-query xprop. 30 s: a panel rarely moves.
fn workarea_cache_fresh(age: Duration) -> bool {
    age < Duration::from_secs(30)
}

/// Scale a chrome color's alpha for the fading toast.
fn with_toast_alpha(color: egui::Color32, alpha: u8) -> egui::Color32 {
    let a = (color.a() as u16 * alpha as u16 / 255) as u8;
    egui::Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a)
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
    // with_user_event: the control socket's server thread pokes an
    // EventLoopProxy to wake the (otherwise ControlFlow::Wait) loop so
    // it drains and answers a queued command.
    let event_loop = match EventLoop::<()>::with_user_event().build() {
        Ok(event_loop) => event_loop,
        Err(error) => {
            eprintln!("phosphor: event loop: {error}");
            return 4;
        }
    };
    shell.control = crate::control::spawn(event_loop.create_proxy());
    match event_loop.run_app(&mut shell) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("phosphor: {error}");
            4
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{mini_resize_zone, workarea_cache_fresh};
    use std::time::Duration;
    use winit::window::ResizeDirection as R;

    // A representative mini window; the corners are 26 px boxes and the
    // edges an 8 px margin.
    const W: f64 = 400.0;
    const H: f64 = 400.0;

    #[test]
    fn each_corner() {
        assert_eq!(mini_resize_zone(1.0, 1.0, W, H), Some(R::NorthWest));
        assert_eq!(mini_resize_zone(399.0, 1.0, W, H), Some(R::NorthEast));
        assert_eq!(mini_resize_zone(1.0, 399.0, W, H), Some(R::SouthWest));
        assert_eq!(mini_resize_zone(399.0, 399.0, W, H), Some(R::SouthEast));
        // exactly on the 26 px corner boundary still counts as corner
        assert_eq!(mini_resize_zone(26.0, 26.0, W, H), Some(R::NorthWest));
    }

    #[test]
    fn each_edge() {
        // mid-span of each edge, inside the 8 px margin but clear of the
        // 26 px corner boxes
        assert_eq!(mini_resize_zone(200.0, 2.0, W, H), Some(R::North));
        assert_eq!(mini_resize_zone(200.0, 398.0, W, H), Some(R::South));
        assert_eq!(mini_resize_zone(398.0, 200.0, W, H), Some(R::East));
        assert_eq!(mini_resize_zone(2.0, 200.0, W, H), Some(R::West));
    }

    #[test]
    fn interior_is_move() {
        assert_eq!(mini_resize_zone(200.0, 200.0, W, H), None);
        assert_eq!(mini_resize_zone(50.0, 50.0, W, H), None);
        // just inside every margin
        assert_eq!(mini_resize_zone(9.0, 200.0, W, H), None);
        assert_eq!(mini_resize_zone(200.0, 9.0, W, H), None);
    }

    #[test]
    fn corner_beats_edge() {
        // (25,25): within both the North and West edge margins? No — it
        // is >8 from either edge, but it IS inside the 26 px corner box,
        // so it must resolve to the NW corner, never an edge.
        assert_eq!(mini_resize_zone(25.0, 25.0, W, H), Some(R::NorthWest));
        // a point hard on the left edge but far down the side is West,
        // not a corner (task's x=3,y=300 case)
        assert_eq!(mini_resize_zone(3.0, 300.0, W, H), Some(R::West));
    }

    #[test]
    fn workarea_cache_ttl() {
        assert!(workarea_cache_fresh(Duration::from_secs(0)));
        assert!(workarea_cache_fresh(Duration::from_secs(29)));
        assert!(!workarea_cache_fresh(Duration::from_secs(30)));
        assert!(!workarea_cache_fresh(Duration::from_secs(120)));
    }
}
