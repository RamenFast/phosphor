// SPDX-License-Identifier: GPL-3.0-or-later
//! Wave-1 risk spike (V4PLAN step 7): winit + wgpu + egui on X11/Muffin.
//!
//! Answers three questions with receipts, before we're pot-committed:
//!   1. Transparency: can a wgpu swapchain composite with alpha under
//!      Muffin (glass mode's future)? Prints the surface's supported
//!      alpha modes and draws a half-transparent pane to check by eye
//!      and by root-capture pixel sampling.
//!   2. Pacing: with PresentMode::Mailbox and continuous redraw, what
//!      present rate does the stack actually sustain (prints fps once
//!      a second — the >165 question v3 could never answer)?
//!   3. Drag-and-drop: winit must deliver HoveredFile/DroppedFile.
//!
//! Run windowed-transparent (default) or `--fullscreen` for the rate
//! test. Exits on Esc or after `--seconds N` (for scripted runs).

use std::sync::Arc;
use std::time::Instant;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Fullscreen, Window, WindowId};

struct Graphics {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
}

struct Spike {
    fullscreen: bool,
    seconds: Option<u64>,
    started: Instant,
    graphics: Option<Graphics>,
    frames: u32,
    last_report: Instant,
    fps_line: String,
    dropped: Vec<String>,
}

impl Spike {
    fn init(&mut self, event_loop: &ActiveEventLoop) {
        let mut attributes = Window::default_attributes()
            .with_title("phosphor v4 spike")
            .with_transparent(true);
        if self.fullscreen {
            attributes = attributes
                .with_fullscreen(Some(Fullscreen::Borderless(None)));
        } else {
            attributes = attributes.with_inner_size(
                winit::dpi::LogicalSize::new(900.0, 600.0));
        }
        let window = Arc::new(event_loop.create_window(attributes)
            .expect("window"));

        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window.clone())
            .expect("surface");
        let adapter = pollster::block_on(instance.request_adapter(
            &wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                ..Default::default()
            })).expect("adapter");
        println!("adapter: {:?}", adapter.get_info().name);
        let (device, queue) = pollster::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("device");

        let capabilities = surface.get_capabilities(&adapter);
        println!("present modes: {:?}", capabilities.present_modes);
        println!("alpha modes:   {:?}", capabilities.alpha_modes);
        let present_mode = if capabilities.present_modes
            .contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Immediate
        };
        // glass mode's fate: anything beyond Opaque lets the window's
        // alpha reach the compositor
        let alpha_mode = [wgpu::CompositeAlphaMode::PreMultiplied,
                          wgpu::CompositeAlphaMode::PostMultiplied,
                          wgpu::CompositeAlphaMode::Inherit]
            .into_iter()
            .find(|mode| capabilities.alpha_modes.contains(mode))
            .unwrap_or(wgpu::CompositeAlphaMode::Opaque);
        println!("chosen: {present_mode:?} + {alpha_mode:?}");

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

        self.graphics = Some(Graphics {
            window, surface, device, queue, config, egui_ctx,
            egui_state, egui_renderer,
        });
    }

    fn redraw(&mut self) {
        let Some(graphics) = self.graphics.as_mut() else { return };
        let frame = match graphics.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(_) => {
                graphics.surface.configure(&graphics.device,
                                           &graphics.config);
                return;
            }
        };
        let view = frame.texture.create_view(&Default::default());

        let raw_input = graphics.egui_state
            .take_egui_input(&graphics.window);
        let fps_line = self.fps_line.clone();
        let dropped = self.dropped.clone();
        let full_output = graphics.egui_ctx.run(raw_input, |ctx| {
            egui::Window::new("phosphor v4 spike").show(ctx, |ui| {
                ui.label(fps_line.as_str());
                ui.label(format!("elapsed {:.0?}",
                                 Instant::now() - self.started));
                ui.separator();
                ui.label("drop a file anywhere:");
                for path in dropped.iter().rev().take(5) {
                    ui.monospace(path);
                }
            });
        });
        let clipped = graphics.egui_ctx.tessellate(
            full_output.shapes, full_output.pixels_per_point);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [graphics.config.width,
                             graphics.config.height],
            pixels_per_point: full_output.pixels_per_point,
        };

        let mut encoder = graphics.device.create_command_encoder(
            &Default::default());
        for (id, delta) in &full_output.textures_delta.set {
            graphics.egui_renderer.update_texture(
                &graphics.device, &graphics.queue, *id, delta);
        }
        graphics.egui_renderer.update_buffers(
            &graphics.device, &graphics.queue, &mut encoder,
            &clipped, &screen);
        {
            let pass = encoder.begin_render_pass(
                &wgpu::RenderPassDescriptor {
                    label: Some("spike"),
                    color_attachments: &[Some(
                        wgpu::RenderPassColorAttachment {
                            view: &view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                // the glass question: 35 % green pane —
                                // if alpha reaches Muffin, the desktop
                                // glows through
                                load: wgpu::LoadOp::Clear(
                                    wgpu::Color {
                                        r: 0.02, g: 0.10, b: 0.03,
                                        a: 0.35,
                                    }),
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

        self.frames += 1;
        let since = Instant::now() - self.last_report;
        if since.as_secs_f64() >= 1.0 {
            self.fps_line = format!(
                "{:.0} fps ({:?})", self.frames as f64
                / since.as_secs_f64(), graphics.config.present_mode);
            println!("fps {:.0}", self.frames as f64
                     / since.as_secs_f64());
            self.frames = 0;
            self.last_report = Instant::now();
        }
        graphics.window.request_redraw();
    }
}

impl ApplicationHandler for Spike {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.graphics.is_none() {
            self.init(event_loop);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop,
                    _id: WindowId, event: WindowEvent) {
        if let Some(graphics) = self.graphics.as_mut() {
            let response = graphics.egui_state
                .on_window_event(&graphics.window, &event);
            if response.repaint {
                graphics.window.request_redraw();
            }
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::KeyboardInput { event, .. } => {
                use winit::keyboard::{Key, NamedKey};
                if event.logical_key == Key::Named(NamedKey::Escape) {
                    event_loop.exit();
                }
            }
            WindowEvent::HoveredFile(path) => {
                println!("hover: {}", path.display());
            }
            WindowEvent::DroppedFile(path) => {
                println!("drop: {}", path.display());
                self.dropped.push(path.display().to_string());
            }
            WindowEvent::Resized(size) => {
                if let Some(graphics) = self.graphics.as_mut() {
                    graphics.config.width = size.width.max(1);
                    graphics.config.height = size.height.max(1);
                    graphics.surface.configure(&graphics.device,
                                               &graphics.config);
                }
            }
            WindowEvent::RedrawRequested => {
                self.redraw();
                if let Some(limit) = self.seconds {
                    if self.started.elapsed().as_secs() >= limit {
                        event_loop.exit();
                    }
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let fullscreen = arguments.iter().any(|a| a == "--fullscreen");
    let seconds = arguments.iter()
        .position(|a| a == "--seconds")
        .and_then(|i| arguments.get(i + 1))
        .and_then(|s| s.parse().ok());
    let event_loop = EventLoop::new().expect("event loop");
    let mut spike = Spike {
        fullscreen,
        seconds,
        started: Instant::now(),
        graphics: None,
        frames: 0,
        last_report: Instant::now(),
        fps_line: String::from("warming up…"),
        dropped: Vec::new(),
    };
    event_loop.run_app(&mut spike).expect("run");
}
