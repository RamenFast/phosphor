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
}

pub fn parse_args(arguments: &[String]) -> ShellArgs {
    let mut args = ShellArgs {
        fps_log: false,
        exit_after: None,
        visitor: false,
        mini: false,
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

pub struct Shell {
    args: ShellArgs,
    settings: Settings,
    engine: AudioEngine,
    audio_events: mpsc::Receiver<AudioEvent>,
    computer: phosphor_dsp::Computer,
    renderer_choice: RendererChoice,

    graphics: Option<Graphics>,
    scope_rect: egui::Rect,

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
    capture_on: bool,
    status_line: String,
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
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: capabilities.formats[0],
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
        // monitor.
        self.start_capture_from_settings();
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

    fn wake_render_loop(&mut self) {
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

    /// The frame cadence: user cap, else the monitor's refresh
    /// (v3's "0 = uncapped/monitor"). 0.0 = genuinely uncapped.
    fn cap_hz(&self) -> f64 {
        if self.settings.max_fps > 0 {
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
        while let Ok(event) = self.audio_events.try_recv() {
            match event {
                AudioEvent::StreamEnded => {
                    self.capture_on = false;
                    self.status_line = "stream ended".into();
                }
                AudioEvent::TargetsChanged
                | AudioEvent::DefaultSinkChanged => {}
                AudioEvent::PlaybackEnded => {
                    self.status_line = "playback ended".into();
                }
                AudioEvent::TrackStarted { path } => {
                    self.status_line =
                        format!("playing {}", path.display());
                }
            }
        }

        // ---- samples + quiet law ----
        let samples = self.engine.take_stereo_samples();
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
        };

        let Some(mut graphics) = self.graphics.take() else {
            return false;
        };
        let keep_going = self.frame(&mut graphics, &samples, advancing);
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

        if advancing {
            let segments = self.computer.compute(
                samples, scope_width as f32, scope_height as f32);
            if let Some(gpu) = graphics.scope_gpu.as_mut() {
                if let Err(error) = gpu.resize(scope_width, scope_height) {
                    eprintln!("phosphor: scope resize: {error}");
                }
                gpu.advance(segments);
            }
            if let Some(cpu) = graphics.scope_cpu.as_mut() {
                let (w, h) = (cpu.renderer.width(), cpu.renderer.height());
                if w != scope_width as usize || h != scope_height as usize {
                    let mut renderer = CpuRenderer::new(
                        scope_width as usize, scope_height as usize, 1);
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
        let status = self.status_line.clone();
        let fps = self.last_fps;
        let cpu_texture_id =
            graphics.scope_cpu.as_ref()
                .and_then(|c| c.texture.as_ref().map(|t| t.id()));
        let mut scope_rect_out = self.scope_rect;
        let full_output = graphics.egui_ctx.run(raw_input, |ctx| {
            egui::TopBottomPanel::bottom("transport").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Phosphor v4");
                    ui.separator();
                    ui.label(status.as_str());
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| { ui.label(format!("{fps:.0} fps")); });
                });
            });
            let central = egui::CentralPanel::default()
                .frame(egui::Frame::NONE);
            central.show(ctx, |ui| {
                scope_rect_out = ui.max_rect();
                if let Some(texture_id) = cpu_texture_id {
                    ui.painter().image(
                        texture_id, scope_rect_out,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0),
                                                 egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE);
                }
            });
        });
        self.scope_rect = scope_rect_out;
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
        let theme = build_theme(&self.settings);
        let background = wgpu::Color {
            r: (theme.background_color[0] as f64).powf(2.2),
            g: (theme.background_color[1] as f64).powf(2.2),
            b: (theme.background_color[2] as f64).powf(2.2),
            a: 1.0,
        };
        if let Some(gpu) = graphics.scope_gpu.as_mut() {
            gpu.composite_into(
                &mut encoder, &view,
                (scope_physical.min.x.max(0.0),
                 scope_physical.min.y.max(0.0),
                 scope_physical.width().max(1.0),
                 scope_physical.height().max(1.0)),
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
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(graphics) = self.graphics.as_mut() {
                    graphics.config.width = size.width.max(1);
                    graphics.config.height = size.height.max(1);
                    graphics.surface.configure(&graphics.device,
                                               &graphics.config);
                    graphics.window.request_redraw();
                }
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
        if self.render_loop_active
            && let Some(due) = self.next_frame_due
        {
            if Instant::now() >= due {
                self.next_frame_due = None;
                if let Some(graphics) = &self.graphics {
                    graphics.window.request_redraw();
                }
                event_loop.set_control_flow(ControlFlow::Poll);
            } else {
                event_loop.set_control_flow(ControlFlow::WaitUntil(due));
            }
        } else {
            // faded out: pure event-driven idle (zero CPU) — capture
            // or playback start calls wake_render_loop
            event_loop.set_control_flow(ControlFlow::Wait);
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
