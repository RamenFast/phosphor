// SPDX-License-Identifier: GPL-3.0-or-later
//! wgpu renderer: v3's decay → beam → composite pipeline, offscreen-first
//! (a texture target and readback — the window path in wave 2 reuses the
//! same passes against a surface view).
//!
//! Ports the v3 laws that matter:
//! - rg16float ping-pong energy (checked against the adapter's format
//!   features at init; rgba16float is the fallback, wastefully wide but
//!   never wrong).
//! - Energy-buffer allocation runs under wgpu error scopes; on failure
//!   the renderer SHEDS SUPERSAMPLING and retries once, and never draws
//!   into a broken target (v3's blank-scope-until-restart bug class).
//! - The beam pass is one instanced quad per segment, additive blend;
//!   worst case is 32,000 instances in a stalled frame (phosphor-dsp),
//!   so the instance buffer grows geometrically and is written whole.

use phosphor_beam::{beam_normalization, beam_sigma, glow_keep, Theme,
                    BEAM_RADIUS_SIGMAS, FLASH_KEEP};

const SEGMENT_STRIDE: u64 = 20; // p0.xy, p1.xy, intensity — five f32

pub struct GpuRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    energy_format: wgpu::TextureFormat,

    width: u32,
    height: u32,
    supersample: u32,
    /// True when allocation pressure forced supersample back to 1.
    pub shed_supersample: bool,

    energy_views: [wgpu::TextureView; 2],
    _energy_textures: [wgpu::Texture; 2],
    current: usize,
    output_texture: wgpu::Texture,
    output_view: wgpu::TextureView,

    decay_pipeline: wgpu::RenderPipeline,
    beam_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    decay_layout: wgpu::BindGroupLayout,
    composite_layout: wgpu::BindGroupLayout,
    decay_uniforms: wgpu::Buffer,
    beam_uniforms: wgpu::Buffer,
    composite_uniforms: wgpu::Buffer,
    beam_bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    instance_capacity: u64,

    pub beam_focus: f32,
    pub persistence: f32,
    pub theme: Theme,
    pub grid_enabled: bool,
    pub grid_spacing_fraction: f32,
    pub scope_alpha: f32,
    /// Surface view is sRGB: the hardware encodes, the shader must
    /// NOT apply its manual gamma (double-encode washes the beam —
    /// wave-2.5 receipt). Offline stays false → bytes unchanged.
    pub hardware_encodes: bool,
    /// Display pixels per logical point for the LIVE path. v3 traced in
    /// logical px with `pixel_scale = scale·supersample`; v4 traces in
    /// physical px, so σ needs the scale factor separately or HiDPI
    /// beams come out 1/scale too thin. Offline stays 1.0 (goldens).
    pub display_scale: f32,
    /// GPU timing across decay+beam (None when the adapter lacks
    /// TIMESTAMP_QUERY or the device wasn't given the feature).
    timer: Option<GpuTimer>,
}

/// Pass-level timestamps around the energy passes, resolved through a
/// two-slot staging ring so reading NEVER blocks the frame. The
/// measured span is decay→beam — the deposit cost, the number that
/// moves with the music.
struct GpuTimer {
    query_set: wgpu::QuerySet,
    resolve: wgpu::Buffer,
    staging: [wgpu::Buffer; 2],
    pending: [bool; 2],
    slot: usize,
    ready: std::sync::mpsc::Receiver<usize>,
    ready_sender: std::sync::mpsc::Sender<usize>,
    period_ns: f32,
    last_ms: Option<f32>,
}

impl GpuTimer {
    fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<GpuTimer> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("beam timing"),
            ty: wgpu::QueryType::Timestamp,
            count: 2,
        });
        let resolve = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("beam timing resolve"),
            size: 16,
            usage: wgpu::BufferUsages::QUERY_RESOLVE
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let staging = [0, 1].map(|i| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(if i == 0 { "beam timing 0" }
                            else { "beam timing 1" }),
                size: 16,
                usage: wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        });
        let (ready_sender, ready) = std::sync::mpsc::channel();
        Some(GpuTimer {
            query_set,
            resolve,
            staging,
            pending: [false; 2],
            slot: 0,
            ready,
            ready_sender,
            period_ns: queue.get_timestamp_period(),
            last_ms: None,
        })
    }

    /// Harvest any finished mapping (non-blocking).
    fn drain(&mut self) {
        while let Ok(slot) = self.ready.try_recv() {
            {
                let mapped = self.staging[slot].slice(..).get_mapped_range();
                let ticks: [u64; 2] = [
                    u64::from_le_bytes(mapped[0..8].try_into().unwrap()),
                    u64::from_le_bytes(mapped[8..16].try_into().unwrap()),
                ];
                if ticks[1] > ticks[0] {
                    self.last_ms = Some(
                        (ticks[1] - ticks[0]) as f32 * self.period_ns
                            / 1.0e6);
                }
            }
            self.staging[slot].unmap();
            self.pending[slot] = false;
        }
    }
}

fn create_energy_pair(device: &wgpu::Device, format: wgpu::TextureFormat,
                      width: u32, height: u32)
                      -> Result<[wgpu::Texture; 2], String> {
    device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    device.push_error_scope(wgpu::ErrorFilter::Validation);
    let descriptor = wgpu::TextureDescriptor {
        label: Some("phosphor energy"),
        size: wgpu::Extent3d { width, height,
                               depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    };
    let pair = [device.create_texture(&descriptor),
                device.create_texture(&descriptor)];
    let validation = pollster::block_on(device.pop_error_scope());
    let out_of_memory = pollster::block_on(device.pop_error_scope());
    if let Some(error) = validation.or(out_of_memory) {
        return Err(format!("energy allocation failed: {error}"));
    }
    Ok(pair)
}

impl GpuRenderer {
    /// Headless renderer. `supersample` may be shed to 1 under
    /// allocation pressure — check `shed_supersample`.
    pub fn new_offscreen(width: u32, height: u32, supersample: u32)
                         -> Result<GpuRenderer, String> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(
            &wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                ..Default::default()
            })).map_err(|error| format!("no adapter: {error}"))?;
        let (device, queue) = pollster::block_on(
            adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .map_err(|error| format!("no device: {error}"))?;

        // rg16float is core-spec renderable+blendable, but verify — the
        // fallback is rgba16float (wider, never wrong)
        let energy_format = Self::probe_energy_format(&adapter);
        Self::build(device, queue, energy_format, width, height,
                    supersample, wgpu::TextureFormat::Rgba8Unorm)
    }

    /// The shell's live path: same passes, the caller's device (shared
    /// with egui), compositing straight into the window surface via
    /// [`GpuRenderer::composite_into`]. Per-frame readback stays
    /// offline-only (V4PLAN law).
    pub fn new_for_surface(adapter: &wgpu::Adapter, device: wgpu::Device,
                           queue: wgpu::Queue, width: u32, height: u32,
                           supersample: u32,
                           surface_format: wgpu::TextureFormat)
                           -> Result<GpuRenderer, String> {
        let energy_format = Self::probe_energy_format(adapter);
        let mut renderer = Self::build(device, queue, energy_format,
                                       width, height, supersample,
                                       surface_format)?;
        renderer.hardware_encodes = surface_format.is_srgb();
        renderer.timer = GpuTimer::new(&renderer.device, &renderer.queue);
        eprintln!(
            "phosphor: scope {}x{} ss{} energy {:?} surface {:?} \
hw_encode={}",
            width, height, renderer.supersample, energy_format,
            surface_format, renderer.hardware_encodes);
        Ok(renderer)
    }

    fn probe_energy_format(adapter: &wgpu::Adapter) -> wgpu::TextureFormat {
        let features = adapter.get_texture_format_features(
            wgpu::TextureFormat::Rg16Float);
        if features.allowed_usages.contains(
            wgpu::TextureUsages::RENDER_ATTACHMENT) {
            wgpu::TextureFormat::Rg16Float
        } else {
            wgpu::TextureFormat::Rgba16Float
        }
    }

    fn build(device: wgpu::Device, queue: wgpu::Queue,
             energy_format: wgpu::TextureFormat, width: u32, height: u32,
             supersample: u32, composite_format: wgpu::TextureFormat)
             -> Result<GpuRenderer, String> {
        let supersample = supersample.max(1);
        let mut effective_supersample = supersample;
        let mut shed = false;
        let energy_textures = match create_energy_pair(
            &device, energy_format,
            width * supersample, height * supersample) {
            Ok(pair) => pair,
            Err(first_error) => {
                if supersample > 1 {
                    // shed VRAM, keep the beam alive (v3 law)
                    effective_supersample = 1;
                    shed = true;
                    create_energy_pair(&device, energy_format, width,
                                       height)
                        .map_err(|error| format!(
                            "{first_error}; and after shedding: {error}"))?
                } else {
                    return Err(first_error);
                }
            }
        };
        let energy_views = [
            energy_textures[0].create_view(&Default::default()),
            energy_textures[1].create_view(&Default::default()),
        ];

        let output_texture =
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("phosphor composite"),
                size: wgpu::Extent3d { width, height,
                                       depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: composite_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
        let output_view = output_texture.create_view(&Default::default());

        let shader = device.create_shader_module(
            wgpu::include_wgsl!("shaders.wgsl"));

        let uniform_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float {
                    filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };

        let decay_layout = device.create_bind_group_layout(
            &wgpu::BindGroupLayoutDescriptor {
                label: Some("decay"),
                entries: &[texture_entry(0), uniform_entry(1)],
            });
        let beam_layout = device.create_bind_group_layout(
            &wgpu::BindGroupLayoutDescriptor {
                label: Some("beam"),
                entries: &[uniform_entry(0)],
            });
        let composite_layout = device.create_bind_group_layout(
            &wgpu::BindGroupLayoutDescriptor {
                label: Some("composite"),
                entries: &[texture_entry(0), uniform_entry(1)],
            });

        let pipeline_layout = |layout: &wgpu::BindGroupLayout, label| {
            device.create_pipeline_layout(
                &wgpu::PipelineLayoutDescriptor {
                    label: Some(label),
                    bind_group_layouts: &[layout],
                    push_constant_ranges: &[],
                })
        };

        let decay_pipeline = device.create_render_pipeline(
            &wgpu::RenderPipelineDescriptor {
                label: Some("decay"),
                layout: Some(&pipeline_layout(&decay_layout, "decay")),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("fullscreen_vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("decay_fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: energy_format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview: None,
                cache: None,
            });

        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let beam_pipeline = device.create_render_pipeline(
            &wgpu::RenderPipelineDescriptor {
                label: Some("beam"),
                layout: Some(&pipeline_layout(&beam_layout, "beam")),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("beam_vs"),
                    compilation_options: Default::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: SEGMENT_STRIDE,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &[
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x4,
                                offset: 0,
                                shader_location: 0,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32,
                                offset: 16,
                                shader_location: 1,
                            },
                        ],
                    }],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("beam_fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: energy_format,
                        blend: Some(additive),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview: None,
                cache: None,
            });

        let composite_pipeline = device.create_render_pipeline(
            &wgpu::RenderPipelineDescriptor {
                label: Some("composite"),
                layout: Some(&pipeline_layout(&composite_layout,
                                              "composite")),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("fullscreen_vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("composite_fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: composite_format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview: None,
                cache: None,
            });

        let uniform_buffer = |label, size| device.create_buffer(
            &wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        let decay_uniforms = uniform_buffer("decay uniforms", 16);
        let beam_uniforms = uniform_buffer("beam uniforms", 32);
        let composite_uniforms = uniform_buffer("composite uniforms", 112);

        let beam_bind_group = device.create_bind_group(
            &wgpu::BindGroupDescriptor {
                label: Some("beam"),
                layout: &beam_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: beam_uniforms.as_entire_binding(),
                }],
            });

        let instance_capacity = 4096 * SEGMENT_STRIDE;
        let instance_buffer = device.create_buffer(
            &wgpu::BufferDescriptor {
                label: Some("beam instances"),
                size: instance_capacity,
                usage: wgpu::BufferUsages::VERTEX
                    | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

        Ok(GpuRenderer {
            device,
            queue,
            energy_format,
            width,
            height,
            supersample: effective_supersample,
            shed_supersample: shed,
            energy_views,
            _energy_textures: energy_textures,
            current: 0,
            output_texture,
            output_view,
            decay_pipeline,
            beam_pipeline,
            composite_pipeline,
            decay_layout,
            composite_layout,
            decay_uniforms,
            beam_uniforms,
            composite_uniforms,
            beam_bind_group,
            instance_buffer,
            instance_capacity,
            beam_focus: 1.6,
            persistence: 0.7,
            theme: Theme::preset("P7 Green").unwrap(),
            grid_enabled: true,
            grid_spacing_fraction: 0.1125,
            scope_alpha: 1.0,
            hardware_encodes: false,
            display_scale: 1.0,
            timer: None,
        })
    }

    pub fn energy_format(&self) -> wgpu::TextureFormat {
        self.energy_format
    }

    pub fn supersample(&self) -> u32 {
        self.supersample
    }

    /// Decay + deposit this frame's segments (logical pixels).
    pub fn advance(&mut self, segments: &[[f32; 5]]) {
        let buffer_width = self.width * self.supersample;
        let buffer_height = self.height * self.supersample;
        let pixel_scale = self.supersample as f32;
        // positions are already in trace px — only σ carries the
        // display scale (beam width parity with v3 at any DPI)
        let sigma = beam_sigma(self.beam_focus,
                               pixel_scale * self.display_scale.max(0.1));

        self.queue.write_buffer(
            &self.decay_uniforms, 0,
            float_bytes(&[FLASH_KEEP, glow_keep(self.persistence),
                          0.0, 0.0]));
        self.queue.write_buffer(
            &self.beam_uniforms, 0,
            float_bytes(&[buffer_width as f32, buffer_height as f32,
                          sigma * BEAM_RADIUS_SIGMAS, sigma,
                          beam_normalization(self.beam_focus),
                          pixel_scale, 0.0, 0.0]));

        if !segments.is_empty() {
            let needed = segments.len() as u64 * SEGMENT_STRIDE;
            if needed > self.instance_capacity {
                self.instance_capacity =
                    needed.next_power_of_two();
                self.instance_buffer = self.device.create_buffer(
                    &wgpu::BufferDescriptor {
                        label: Some("beam instances"),
                        size: self.instance_capacity,
                        usage: wgpu::BufferUsages::VERTEX
                            | wgpu::BufferUsages::COPY_DST,
                        mapped_at_creation: false,
                    });
            }
            let flat: &[f32] = unsafe {
                std::slice::from_raw_parts(
                    segments.as_ptr().cast::<f32>(), segments.len() * 5)
            };
            self.queue.write_buffer(&self.instance_buffer, 0,
                                    float_bytes(flat));
        }

        // timing: pump the callback queue, harvest finished readbacks,
        // and only write timestamps when a staging slot is free —
        // reading NEVER stalls a frame
        if self.timer.is_some() {
            let _ = self.device.poll(wgpu::PollType::Poll);
        }
        if let Some(timer) = &mut self.timer {
            timer.drain();
        }
        let time_this_frame = self.timer.as_ref()
            .map(|timer| !timer.pending[timer.slot])
            .unwrap_or(false);
        let query_set = self.timer.as_ref().map(|t| &t.query_set);

        let source = self.current;
        let target = 1 - source;
        let decay_bind_group = self.device.create_bind_group(
            &wgpu::BindGroupDescriptor {
                label: Some("decay"),
                layout: &self.decay_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &self.energy_views[source]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.decay_uniforms
                            .as_entire_binding(),
                    },
                ],
            });

        let mut encoder = self.device.create_command_encoder(
            &Default::default());
        {
            let mut pass = encoder.begin_render_pass(
                &wgpu::RenderPassDescriptor {
                    label: Some("decay"),
                    color_attachments: &[Some(
                        wgpu::RenderPassColorAttachment {
                            view: &self.energy_views[target],
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(
                                    wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                    timestamp_writes: query_set
                        .filter(|_| time_this_frame)
                        .map(|qs| wgpu::RenderPassTimestampWrites {
                            query_set: qs,
                            beginning_of_pass_write_index: Some(0),
                            // empty frame: this pass carries both ends
                            end_of_pass_write_index: segments
                                .is_empty().then_some(1),
                        }),
                    ..Default::default()
                });
            pass.set_pipeline(&self.decay_pipeline);
            pass.set_bind_group(0, &decay_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        if !segments.is_empty() {
            let mut pass = encoder.begin_render_pass(
                &wgpu::RenderPassDescriptor {
                    label: Some("beam"),
                    color_attachments: &[Some(
                        wgpu::RenderPassColorAttachment {
                            view: &self.energy_views[target],
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                    timestamp_writes: query_set
                        .filter(|_| time_this_frame)
                        .map(|qs| wgpu::RenderPassTimestampWrites {
                            query_set: qs,
                            beginning_of_pass_write_index: None,
                            end_of_pass_write_index: Some(1),
                        }),
                    ..Default::default()
                });
            pass.set_pipeline(&self.beam_pipeline);
            pass.set_bind_group(0, &self.beam_bind_group, &[]);
            pass.set_vertex_buffer(
                0, self.instance_buffer.slice(
                    ..segments.len() as u64 * SEGMENT_STRIDE));
            pass.draw(0..6, 0..segments.len() as u32);
        }
        if time_this_frame && let Some(timer) = &self.timer {
            encoder.resolve_query_set(&timer.query_set, 0..2,
                                      &timer.resolve, 0);
            encoder.copy_buffer_to_buffer(
                &timer.resolve, 0, &timer.staging[timer.slot], 0, 16);
        }
        self.queue.submit([encoder.finish()]);
        if time_this_frame && let Some(timer) = &mut self.timer {
            let slot = timer.slot;
            timer.pending[slot] = true;
            let sender = timer.ready_sender.clone();
            timer.staging[slot].slice(..).map_async(
                wgpu::MapMode::Read,
                move |result| {
                    if result.is_ok() {
                        let _ = sender.send(slot);
                    }
                });
            timer.slot = 1 - slot;
        }
        self.current = target;
    }

    /// Latest measured decay→beam GPU span in milliseconds (None until
    /// the first readback lands, or without TIMESTAMP_QUERY).
    pub fn gpu_frame_ms(&self) -> Option<f32> {
        self.timer.as_ref().and_then(|timer| timer.last_ms)
    }

    /// Uniforms + bind group for a composite pass. `origin` is the
    /// scope viewport's top-left in framebuffer pixels — 0,0 for the
    /// offscreen path (bytes identical to wave 1, goldens hold).
    fn prepare_composite(&mut self, origin: (f32, f32)) -> wgpu::BindGroup {
        let theme = self.theme;
        let composite_data: [f32; 28] = [
            theme.beam_color[0], theme.beam_color[1],
            theme.beam_color[2], 0.0,
            theme.flash_color[0], theme.flash_color[1],
            theme.flash_color[2], 0.0,
            theme.grid_color[0], theme.grid_color[1],
            theme.grid_color[2], 0.0,
            theme.background_color[0], theme.background_color[1],
            theme.background_color[2], 0.0,
            self.width as f32, self.height as f32,
            self.grid_spacing_fraction
                * self.width.min(self.height) as f32,
            if self.grid_enabled { 1.0 } else { 0.0 },
            // the WGSL field is i32; smuggle the bit pattern through
            f32::from_bits(self.supersample),
            self.scope_alpha, origin.0, origin.1,
            if self.hardware_encodes { 1.0 } else { 0.0 },
            0.0, 0.0, 0.0,
        ];
        self.queue.write_buffer(&self.composite_uniforms, 0,
                                float_bytes(&composite_data));

        self.device.create_bind_group(
            &wgpu::BindGroupDescriptor {
                label: Some("composite"),
                layout: &self.composite_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &self.energy_views[self.current]),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.composite_uniforms
                            .as_entire_binding(),
                    },
                ],
            })
    }

    /// Record the composite pass into a fresh encoder (shared by the
    /// readback path and the bench's submit-only path).
    fn encode_composite(&mut self) -> wgpu::CommandEncoder {
        let bind_group = self.prepare_composite((0.0, 0.0));
        let mut encoder = self.device.create_command_encoder(
            &Default::default());
        {
            let mut pass = encoder.begin_render_pass(
                &wgpu::RenderPassDescriptor {
                    label: Some("composite"),
                    color_attachments: &[Some(
                        wgpu::RenderPassColorAttachment {
                            view: &self.output_view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(
                                    wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                    ..Default::default()
                });
            pass.set_pipeline(&self.composite_pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder
    }

    /// The live path: composite THIS frame's glow into a window
    /// surface view, inside `viewport` (x, y, w, h in framebuffer
    /// pixels). `clear` paints the whole attachment first (the scope
    /// is the app's background; pass None to draw over existing
    /// content). The energy buffer must be sized to the viewport —
    /// call [`GpuRenderer::resize`] when the scope rect changes.
    pub fn composite_into(&mut self, encoder: &mut wgpu::CommandEncoder,
                          view: &wgpu::TextureView,
                          viewport: (f32, f32, f32, f32),
                          clear: Option<wgpu::Color>) {
        let bind_group = self.prepare_composite((viewport.0, viewport.1));
        let mut pass = encoder.begin_render_pass(
            &wgpu::RenderPassDescriptor {
                label: Some("composite live"),
                color_attachments: &[Some(
                    wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: match clear {
                                Some(color) => wgpu::LoadOp::Clear(color),
                                None => wgpu::LoadOp::Load,
                            },
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                ..Default::default()
            });
        pass.set_pipeline(&self.composite_pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.set_viewport(viewport.0, viewport.1, viewport.2, viewport.3,
                          0.0, 1.0);
        pass.set_scissor_rect(viewport.0 as u32, viewport.1 as u32,
                              (viewport.2 as u32).max(1),
                              (viewport.3 as u32).max(1));
        pass.draw(0..3, 0..1);
    }

    /// Resize the energy buffers (scope rect changed). Keeps the shed
    /// law: allocation failure drops supersampling before failing.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), String> {
        if width == self.width && height == self.height {
            return Ok(());
        }
        let width = width.max(1);
        let height = height.max(1);
        let energy_textures = match create_energy_pair(
            &self.device, self.energy_format,
            width * self.supersample, height * self.supersample) {
            Ok(pair) => pair,
            Err(first_error) => {
                if self.supersample > 1 {
                    self.supersample = 1;
                    self.shed_supersample = true;
                    create_energy_pair(&self.device, self.energy_format,
                                       width, height)
                        .map_err(|error| format!(
                            "{first_error}; and after shedding: {error}"))?
                } else {
                    return Err(first_error);
                }
            }
        };
        self.energy_views = [
            energy_textures[0].create_view(&Default::default()),
            energy_textures[1].create_view(&Default::default()),
        ];
        self._energy_textures = energy_textures;
        self.current = 0;
        self.width = width;
        self.height = height;
        Ok(())
    }

    /// Composite to the offscreen target and submit WITHOUT readback —
    /// the bench's throughput path (read back once at the end for a
    /// checksum, never per frame).
    pub fn composite_submit(&mut self) {
        let encoder = self.encode_composite();
        self.queue.submit([encoder.finish()]);
    }

    /// Block until all submitted work completes (bench pacing: bounded
    /// pipelining so throughput numbers measure the GPU, not a queue).
    pub fn wait_idle(&self) {
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
    }

    pub fn composite_and_read(&mut self) -> Vec<u8> {
        let encoder = self.encode_composite();
        self.finish_and_read_texture(encoder, None, self.width,
                                     self.height)
    }

    /// Composite into a caller-shaped stand-in surface through the
    /// LIVE path (`composite_into`: viewport + scissor + origin
    /// uniform) and read the whole surface back. This is how tests pin
    /// live-path geometry — the offline path can't catch a viewport
    /// bug by construction (buffer size == output size there).
    pub fn composite_into_read(&mut self, surface_width: u32,
                               surface_height: u32,
                               viewport: (f32, f32, f32, f32))
                               -> Vec<u8> {
        let texture = self.device.create_texture(
            &wgpu::TextureDescriptor {
                label: Some("live-path stand-in surface"),
                size: wgpu::Extent3d {
                    width: surface_width,
                    height: surface_height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
        let view = texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(
            &Default::default());
        self.composite_into(&mut encoder, &view, viewport,
                            Some(wgpu::Color::BLACK));
        self.finish_and_read_texture(encoder, Some(&texture),
                                     surface_width, surface_height)
    }

    /// Copy `texture` (default: the offline output) into a mapped
    /// buffer and return tight RGBA rows.
    fn finish_and_read_texture(&self, mut encoder: wgpu::CommandEncoder,
                               texture: Option<&wgpu::Texture>,
                               width: u32, height: u32) -> Vec<u8> {
        let bytes_per_row = (width * 4).next_multiple_of(256);
        let readback = self.device.create_buffer(
            &wgpu::BufferDescriptor {
                label: Some("readback"),
                size: bytes_per_row as u64 * height as u64,
                usage: wgpu::BufferUsages::COPY_DST
                    | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: texture.unwrap_or(&self.output_texture),
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: None,
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            });
        self.queue.submit([encoder.finish()]);

        let slice = readback.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        let _ = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
        receiver.recv().expect("map channel").expect("map failed");
        let mapped = slice.get_mapped_range();
        let mut pixels =
            Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let start = (row * bytes_per_row) as usize;
            pixels.extend_from_slice(
                &mapped[start..start + (width * 4) as usize]);
        }
        drop(mapped);
        readback.unmap();
        pixels
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }
}

/// f32 slice → bytes without a bytemuck dependency.
fn float_bytes(values: &[f32]) -> &[u8] {
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(),
                                   std::mem::size_of_val(values))
    }
}
