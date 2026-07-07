// SPDX-License-Identifier: GPL-3.0-or-later
//! `phosphor render <input> <output.mp4>` — v3's offline pipeline, one
//! engine: ffmpeg decodes to f32le at the scope rate (ffmpeg is the
//! contracted decode/mux pipe for render; symphonia arrives with the
//! wave-2 shell), phosphor-dsp traces, a renderer deposits light, a
//! second ffmpeg encodes and muxes the original audio. `.phos`
//! postcards decode via their 256-byte header exactly like v3
//! (`-skip_initial_bytes 256 -f s16le`).
//!
//! Exit contract: 0 success · 2 usage · 3 invalid input content ·
//! 4 I/O or pipeline failure. No fallback paths: if the GPU renderer
//! cannot initialize, that is an error, not a silent CPU downgrade.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use phosphor_beam::Theme;
use phosphor_dsp::{Computer, KitOp, Mode};
use phosphor_proto::{phos, phoskit, settings::Settings};
use phosphor_render_cpu::CpuRenderer;
use phosphor_render_gpu::GpuRenderer;

pub const EXPORT_FPS: u32 = 60;

pub struct RenderArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    pub rate: Option<u32>,
    pub renderer: RendererKind,
    pub size: Option<(u32, u32)>,
    pub json: bool,
    pub dump_frame: Option<(u64, PathBuf)>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum RendererKind {
    Gpu,
    Cpu,
}

pub enum FrameSink {
    Cpu(Box<CpuRenderer>),
    Gpu(Box<GpuRenderer>),
}

impl FrameSink {
    pub fn advance(&mut self, segments: &[[f32; 5]]) {
        match self {
            FrameSink::Cpu(renderer) => renderer.advance(segments),
            FrameSink::Gpu(renderer) => renderer.advance(segments),
        }
    }

    pub fn frame(&mut self) -> Vec<u8> {
        match self {
            FrameSink::Cpu(renderer) => renderer.composite().to_vec(),
            FrameSink::Gpu(renderer) => renderer.composite_and_read(),
        }
    }

    /// Per-frame theme update — the beam color cycle animates offline
    /// renders on media time (one beam law: exports match live).
    pub fn set_theme(&mut self, theme: Theme) {
        match self {
            FrameSink::Cpu(renderer) => renderer.theme = theme,
            FrameSink::Gpu(renderer) => renderer.theme = theme,
        }
    }
}

fn usage() -> String {
    "usage: phosphor render <input> <output.mp4> [--rate N] \
     [--renderer gpu|cpu] [--size WxH] [--dump-frame N:PATH] \
     [--output json]".to_string()
}

pub fn parse_args(arguments: &[String]) -> Result<RenderArgs, String> {
    let mut positional = Vec::new();
    let mut rate = None;
    let mut renderer = RendererKind::Gpu;
    let mut size = None;
    let mut json = false;
    let mut dump_frame = None;
    let mut iterator = arguments.iter();
    while let Some(argument) = iterator.next() {
        match argument.as_str() {
            "--rate" => {
                rate = Some(iterator.next()
                    .and_then(|value| value.parse().ok())
                    .ok_or_else(usage)?);
            }
            "--renderer" => {
                renderer = match iterator.next().map(String::as_str) {
                    Some("gpu") => RendererKind::Gpu,
                    Some("cpu") => RendererKind::Cpu,
                    _ => return Err(usage()),
                };
            }
            "--size" => {
                let value = iterator.next().ok_or_else(usage)?;
                let (width, height) = value.split_once('x')
                    .ok_or_else(usage)?;
                size = Some((width.parse().map_err(|_| usage())?,
                             height.parse().map_err(|_| usage())?));
            }
            "--output" => {
                json = matches!(iterator.next().map(String::as_str),
                                Some("json"));
            }
            "--dump-frame" => {
                let value = iterator.next().ok_or_else(usage)?;
                let (frame, path) = value.split_once(':')
                    .ok_or_else(usage)?;
                dump_frame = Some((frame.parse().map_err(|_| usage())?,
                                   PathBuf::from(path)));
            }
            other if other.starts_with("--") => return Err(usage()),
            _ => positional.push(argument.clone()),
        }
    }
    if positional.len() != 2 {
        return Err(usage());
    }
    Ok(RenderArgs {
        input: PathBuf::from(&positional[0]),
        output: PathBuf::from(&positional[1]),
        rate,
        renderer,
        size,
        json,
        dump_frame,
    })
}

/// v3's export-size law: square for the radial/spatial modes.
pub fn export_size(mode: Mode) -> (u32, u32) {
    match mode {
        Mode::Xy | Mode::Xy45 | Mode::XySwirl | Mode::Ring
        | Mode::Tunnel | Mode::XyzTakens | Mode::Helix => (720, 720),
        _ => (1080, 720),
    }
}

/// ffmpeg arguments that make a .phos input decodable: raw s16le at
/// the header's rate, header skipped. Empty for ordinary audio.
pub fn phos_input_arguments(header: Option<&phos::Header>)
                            -> Vec<String> {
    match header.and_then(|header| header.rate()) {
        Some(rate) => vec![
            "-skip_initial_bytes".into(),
            phos::HEADER_BYTES.to_string(),
            "-f".into(), "s16le".into(),
            "-ar".into(), rate.to_string(),
            "-ac".into(), "2".into(),
        ],
        None => Vec::new(),
    }
}

pub fn spawn_decoder(input: &Path, rate: u32,
                     phos_arguments: &[String])
                     -> std::io::Result<Child> {
    Command::new("ffmpeg")
        .args(["-v", "error"])
        .args(phos_arguments)
        .arg("-i").arg(input)
        .args(["-f", "f32le", "-ac", "2", "-ar", &rate.to_string(), "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

fn spawn_encoder(input: &Path, output: &Path, width: u32, height: u32,
                 phos_arguments: &[String]) -> std::io::Result<Child> {
    Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error",
               "-f", "rawvideo", "-pix_fmt", "rgba",
               "-s", &format!("{width}x{height}"),
               "-r", &EXPORT_FPS.to_string(), "-i", "-"])
        .args(phos_arguments)
        .arg("-i").arg(input)
        .args(["-map", "0:v", "-map", "1:a",
               "-c:v", "libx264", "-preset", "veryfast", "-crf", "18",
               "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(output)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
}

pub(crate) fn stderr_tail(child: &mut Child) -> String {
    let mut text = String::new();
    if let Some(mut stderr) = child.stderr.take() {
        let _ = stderr.read_to_string(&mut text);
    }
    text.trim().chars().rev().take(300).collect::<String>()
        .chars().rev().collect()
}

/// Computer from settings: mode/gain/beam-energy/kit — shared by the
/// offline pipeline and the live shell (one wiring, one truth).
pub fn build_computer(settings: &Settings, rate: u32)
                      -> Result<Computer, (i32, String)> {
    let mode = settings.display_mode.parse::<Mode>()
        .map_err(|error| (3, error))?;
    let mut computer = Computer::new();
    computer.mode = mode;
    computer.gain = settings.gain;
    computer.beam_energy = settings.beam_energy;
    computer.set_sample_rate(rate, 1);
    if settings.kit_enabled
        && let Some(path) = &settings.kit_path {
            // a broken kit shouldn't kill an export; render it plain —
            // but never silently (the GUI toasts on KitChanged too)
            match phoskit::load(Path::new(path)) {
                Ok(kit) => {
                    let stages: Vec<(KitOp, [f64; 4])> = kit.stages
                        .iter()
                        .filter_map(|(op, parameters)| {
                            KitOp::from_name(op)
                                .map(|op| (op, *parameters))
                        })
                        .collect();
                    computer.set_kit(&stages);
                }
                Err(error) => eprintln!(
                    "phosphor: kit ignored ({path}): {error}"),
            }
        }
    Ok(computer)
}

/// Theme from settings (Custom / preset / AMOLED) — shell-shared too.
/// Equivalent to `build_theme_at(settings, 0.0)` minus the cycle: the
/// static entry point for paths with no clock.
pub fn build_theme(settings: &Settings) -> Theme {
    let mut theme = if settings.theme_name == "Custom" {
        Theme::custom(settings.custom_beam_color,
                      settings.custom_grid_color)
    } else {
        Theme::preset(&settings.theme_name)
            .unwrap_or_else(|| phosphor_beam::THEME_PRESETS[0].1)
    };
    if settings.amoled_background {
        theme = theme.with_amoled();
    }
    theme
}

/// Is the beam color cycle actually animating? (Custom theme with two
/// or three live slots — one slot is a static custom color, exactly
/// the pre-4.1 behavior.)
pub fn beam_cycle_animating(settings: &Settings) -> bool {
    settings.theme_name == "Custom" && settings.beam_cycle_count > 1
}

/// The beam color at ring `phase` ∈ [0, count): the integer part
/// names the slot, the fraction is the progress of the crossfade
/// toward the next slot, eased with smoothstep so the beam lingers on
/// the pure colors and glides between them; the ring wraps 1→2(→3)→1.
/// Timer mode derives phase from the clock; track mode (change on
/// song) holds an integer phase and sweeps one unit per song change.
pub fn cycle_beam_color_phase(settings: &Settings, phase: f64)
                              -> [f32; 3] {
    let colors = [settings.custom_beam_color,
                  settings.custom_beam_color_2,
                  settings.custom_beam_color_3];
    let count = settings.beam_cycle_count.clamp(1, 3) as usize;
    if count == 1 {
        return colors[0];
    }
    let phase = phase.rem_euclid(count as f64);
    let index = (phase as usize).min(count - 1);
    let next = (index + 1) % count;
    let fraction = (phase - index as f64) as f32;
    let eased = fraction * fraction * (3.0 - 2.0 * fraction);
    let from = colors[index];
    let to = colors[next];
    [from[0] + (to[0] - from[0]) * eased,
     from[1] + (to[1] - from[1]) * eased,
     from[2] + (to[2] - from[2]) * eased]
}

/// Theme from an explicit ring phase — the shell's track mode feeds
/// this; the timer paths go through `build_theme_at`.
pub fn build_theme_phase(settings: &Settings, phase: f64) -> Theme {
    if !beam_cycle_animating(settings) {
        return build_theme(settings);
    }
    let mut theme = Theme::custom(
        cycle_beam_color_phase(settings, phase),
        settings.custom_grid_color);
    if settings.amoled_background {
        theme = theme.with_amoled();
    }
    theme
}

/// Theme at TIMER time `t`: the cycle animates the Custom beam (flash
/// and background derive from it, v3's Theme::custom law — the grid
/// stays the user's pick); everything else is `build_theme` verbatim.
pub fn build_theme_at(settings: &Settings, t: f64) -> Theme {
    let leg = settings.beam_cycle_seconds.clamp(0.1, 60.0);
    build_theme_phase(settings, t / leg)
}

/// Build the computer + renderer the way v3's offline pipeline did:
/// mode/gain/kit from settings, persistence and theme on the renderer.
pub fn build_pipeline(settings: &Settings, rate: u32, width: u32,
                      height: u32, kind: RendererKind)
                      -> Result<(Computer, FrameSink), (i32, String)> {
    let computer = build_computer(settings, rate)?;
    let theme = build_theme(settings);
    let grid_fraction =
        phosphor_beam::grid_spacing_fraction(settings.gain);

    let sink = match kind {
        RendererKind::Cpu => {
            let mut renderer =
                Box::new(CpuRenderer::new(width as usize,
                                          height as usize, 1));
            renderer.beam_focus = settings.beam_focus;
            renderer.persistence = settings.persistence;
            renderer.theme = theme;
            renderer.grid_enabled = settings.grid_enabled;
            renderer.grid_spacing_fraction = grid_fraction;
            FrameSink::Cpu(renderer)
        }
        RendererKind::Gpu => {
            let mut renderer = Box::new(
                GpuRenderer::new_offscreen(width, height,
                                           settings.gl_supersample)
                    .map_err(|error| (4, error))?);
            renderer.beam_focus = settings.beam_focus;
            renderer.persistence = settings.persistence;
            renderer.theme = theme;
            renderer.grid_enabled = settings.grid_enabled;
            renderer.grid_spacing_fraction = grid_fraction;
            FrameSink::Gpu(renderer)
        }
    };
    Ok((computer, sink))
}

fn write_ppm(path: &Path, rgba: &[u8], width: u32, height: u32)
             -> std::io::Result<()> {
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    write!(out, "P6\n{width} {height}\n255\n")?;
    for pixel in rgba.chunks_exact(4) {
        out.write_all(&pixel[..3])?;
    }
    Ok(())
}

pub fn run(arguments: &[String]) -> i32 {
    let args = match parse_args(arguments) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return 2;
        }
    };
    if !args.input.exists() {
        eprintln!("input not found: {}", args.input.display());
        return 4;
    }

    let phos_header = match phos::read_header(&args.input) {
        Ok(header) => header,
        Err(error) => {
            eprintln!("{error}");
            return 3;
        }
    };
    let settings = Settings::load(&phosphor_proto::settings::default_path());
    let rate = args.rate.unwrap_or(settings.scope_sample_rate);
    let mode = match settings.display_mode.parse::<Mode>() {
        Ok(mode) => mode,
        Err(error) => {
            eprintln!("{error}");
            return 3;
        }
    };
    let (width, height) = args.size.unwrap_or_else(|| export_size(mode));

    let (mut computer, mut sink) =
        match build_pipeline(&settings, rate, width, height,
                             args.renderer) {
            Ok(pipeline) => pipeline,
            Err((code, message)) => {
                eprintln!("{message}");
                return code;
            }
        };

    let phos_arguments = phos_input_arguments(phos_header.as_ref());
    let total_frames = phos_header.as_ref()
        .and_then(|header| Some(header.frames()?
                                * u64::from(EXPORT_FPS)
                                / u64::from(header.rate()?)));

    eprintln!("rendering {} → {} [{} @ {} Hz, {}]",
              args.input.display(), args.output.display(),
              settings.display_mode, rate,
              match args.renderer { RendererKind::Gpu => "gpu",
                                    RendererKind::Cpu => "cpu" });

    let mut decoder = match spawn_decoder(&args.input, rate,
                                          &phos_arguments) {
        Ok(child) => child,
        Err(error) => {
            eprintln!("ffmpeg: {error}");
            return 4;
        }
    };
    let mut encoder = match spawn_encoder(&args.input, &args.output,
                                          width, height,
                                          &phos_arguments) {
        Ok(child) => child,
        Err(error) => {
            eprintln!("ffmpeg (encode): {error}");
            return 4;
        }
    };

    let bytes_per_frame = (rate / EXPORT_FPS) as usize * 8;
    let mut buffer = vec![0u8; bytes_per_frame];
    let mut samples = vec![0f32; bytes_per_frame / 4];
    let mut decoder_out = decoder.stdout.take().expect("decoder stdout");
    let mut encoder_in = encoder.stdin.take().expect("encoder stdin");
    let started = std::time::Instant::now();
    let mut frame_index: u64 = 0;

    // read_exact error = trailing partial frame: done
    while decoder_out.read_exact(&mut buffer).is_ok() {
        for (slot, chunk) in samples.iter_mut()
            .zip(buffer.chunks_exact(4)) {
            *slot = f32::from_le_bytes([chunk[0], chunk[1], chunk[2],
                                        chunk[3]]);
        }
        // headless renders animate the cycle on MEDIA time — timer
        // mode only: in track mode one input = one song, so the color
        // holds still (slot 1, the build_pipeline theme)
        if beam_cycle_animating(&settings)
            && settings.beam_cycle_mode != "track"
        {
            sink.set_theme(build_theme_at(
                &settings,
                frame_index as f64 / f64::from(EXPORT_FPS)));
        }
        let segments = computer.compute(&samples, width as f32,
                                        height as f32);
        sink.advance(segments);
        let frame = sink.frame();
        if let Some((wanted, path)) = &args.dump_frame
            && *wanted == frame_index
                && let Err(error) = write_ppm(path, &frame, width,
                                              height) {
                    eprintln!("dump-frame: {error}");
                }
        if encoder_in.write_all(&frame).is_err() {
            break;                          // -shortest closed the pipe
        }
        frame_index += 1;
        if frame_index.is_multiple_of(60) {
            match total_frames {
                Some(total) => eprint!("\r  {frame_index}/{total} frames"),
                None => eprint!("\r  {frame_index} frames"),
            }
        }
    }

    drop(encoder_in);
    let decode_error = stderr_tail(&mut decoder);
    let decoder_status = decoder.wait();
    let encode_error = stderr_tail(&mut encoder);
    let encoder_status = encoder.wait();
    eprintln!();

    if !decoder_status.map(|status| status.success()).unwrap_or(false) {
        eprintln!("decode failed: {decode_error}");
        return 4;
    }
    if !encoder_status.map(|status| status.success()).unwrap_or(false) {
        eprintln!("encode failed: {encode_error}");
        return 4;
    }
    if frame_index == 0 {
        eprintln!("no audio decoded — is the input an audio file?");
        return 3;
    }

    let seconds = started.elapsed().as_secs_f64();
    if args.json {
        println!("{}", serde_json::json!({
            "frames": frame_index,
            "seconds": (seconds * 10.0).round() / 10.0,
            "fps_equivalent":
                ((frame_index as f64 / seconds) * 10.0).round() / 10.0,
            "output": args.output.display().to_string(),
        }));
    } else {
        eprintln!("done: {} ({} frames, {:.1}s, {:.1} fps)",
                  args.output.display(), frame_index, seconds,
                  frame_index as f64 / seconds);
    }
    0
}

#[cfg(test)]
mod cycle_tests {
    use super::*;

    /// the timer clock expressed through the phase form (what
    /// build_theme_at does internally)
    fn cycle_beam_color(settings: &Settings, t: f64) -> [f32; 3] {
        let leg = settings.beam_cycle_seconds.clamp(0.1, 60.0);
        cycle_beam_color_phase(settings, t / leg)
    }

    fn cycling_settings(count: i64, seconds: f64) -> Settings {
        Settings {
            theme_name: "Custom".into(),
            custom_beam_color: [1.0, 0.0, 0.0],
            custom_beam_color_2: [0.0, 1.0, 0.0],
            custom_beam_color_3: [0.0, 0.0, 1.0],
            beam_cycle_count: count,
            beam_cycle_seconds: seconds,
            ..Settings::default()
        }
    }

    #[test]
    fn one_slot_is_static() {
        let settings = cycling_settings(1, 3.0);
        assert!(!beam_cycle_animating(&settings));
        for t in [0.0, 1.7, 42.0] {
            assert_eq!(cycle_beam_color(&settings, t), [1.0, 0.0, 0.0]);
        }
        assert_eq!(build_theme_at(&settings, 5.0),
                   build_theme(&settings));
    }

    #[test]
    fn legs_start_pure_and_blend_midway() {
        let settings = cycling_settings(3, 3.0);
        assert!(beam_cycle_animating(&settings));
        // leg starts sit exactly on the slots
        assert_eq!(cycle_beam_color(&settings, 0.0), [1.0, 0.0, 0.0]);
        assert_eq!(cycle_beam_color(&settings, 3.0), [0.0, 1.0, 0.0]);
        assert_eq!(cycle_beam_color(&settings, 6.0), [0.0, 0.0, 1.0]);
        // midway through leg 1: an even red→green blend (smoothstep
        // of 0.5 is 0.5)
        let mid = cycle_beam_color(&settings, 1.5);
        assert!((mid[0] - 0.5).abs() < 1e-6 && (mid[1] - 0.5).abs() < 1e-6,
                "midpoint blends evenly: {mid:?}");
        // the ring wraps: leg 3 ends back on slot 1
        assert_eq!(cycle_beam_color(&settings, 9.0), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn two_slots_ping_pong_through_the_ring() {
        let settings = cycling_settings(2, 1.0);
        assert_eq!(cycle_beam_color(&settings, 0.0), [1.0, 0.0, 0.0]);
        assert_eq!(cycle_beam_color(&settings, 1.0), [0.0, 1.0, 0.0]);
        assert_eq!(cycle_beam_color(&settings, 2.0), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn phase_form_matches_track_mode_semantics() {
        let settings = cycling_settings(3, 3.0);
        // integer phases rest exactly on the slots (a track-mode hold)
        assert_eq!(cycle_beam_color_phase(&settings, 0.0),
                   [1.0, 0.0, 0.0]);
        assert_eq!(cycle_beam_color_phase(&settings, 1.0),
                   [0.0, 1.0, 0.0]);
        assert_eq!(cycle_beam_color_phase(&settings, 2.0),
                   [0.0, 0.0, 1.0]);
        // a song-change fade mid-flight: phase 1.5 = green→blue even
        let mid = cycle_beam_color_phase(&settings, 1.5);
        assert!((mid[1] - 0.5).abs() < 1e-6
                && (mid[2] - 0.5).abs() < 1e-6, "{mid:?}");
        // the ring wraps: phase 2→0 sweep lands back on red
        assert_eq!(cycle_beam_color_phase(&settings, 3.0),
                   [1.0, 0.0, 0.0]);
        // timer form is the phase form on t/leg
        assert_eq!(cycle_beam_color(&settings, 4.5),
                   cycle_beam_color_phase(&settings, 1.5));
    }

    #[test]
    fn cycle_theme_derives_flash_and_background_from_animated_beam() {
        let settings = cycling_settings(2, 1.0);
        let theme = build_theme_at(&settings, 1.0); // pure green
        assert_eq!(theme.beam_color, [0.0, 1.0, 0.0]);
        assert_eq!(theme.grid_color, settings.custom_grid_color,
                   "grid stays the user's pick");
        // Theme::custom law: background = beam * 0.03
        assert!((theme.background_color[1] - 0.03).abs() < 1e-6);
        // presets never animate
        let mut preset = cycling_settings(3, 1.0);
        preset.theme_name = "P7 Green".into();
        assert!(!beam_cycle_animating(&preset));
        assert_eq!(build_theme_at(&preset, 2.0), build_theme(&preset));
    }
}
