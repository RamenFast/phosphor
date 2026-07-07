// SPDX-License-Identifier: GPL-3.0-or-later
//! Chrome pass iv — snapshot/clip exports and the visitor.
//!
//! Exports re-render audio through a FRESH offline pipeline (never
//! the live renderer — v3 law, §13): snapshot = last 1.5 s of history,
//! 1.2 s warmup at 60 fps, final frame to PNG in ~/Pictures/Phosphor;
//! clip = last 10 s to mp4 with its own sound via the render encoder.
//! The CPU renderer draws exports, exactly like v3's Cairo recorder.
//! Size law verbatim: xy_dots is NOT square (1080×720 — the quirk).

use std::io::Write as _;
use std::path::{Path, PathBuf};

use phosphor_proto::settings::Settings;

pub const EXPORT_FPS: u32 = 60;
const SNAPSHOT_WARMUP_SECONDS: f32 = 1.2;

/// v3 recorder export_size — note xy_dots stays wide.
fn export_size(display_mode: &str) -> (u32, u32) {
    match display_mode {
        "xy" | "xy45" | "xy_swirl" | "ring" | "tunnel" | "xyz_takens"
        | "helix" => (720, 720),
        _ => (1080, 720),
    }
}

fn pictures_directory() -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join("Pictures/Phosphor")
}

pub(crate) fn timestamp() -> String {
    // %Y%m%d-%H%M%S without a chrono dep
    let output = std::process::Command::new("date")
        .arg("+%Y%m%d-%H%M%S")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if output.is_empty() {
        format!("{}", std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs()).unwrap_or(0))
    } else {
        output
    }
}

/// Minimal RIFF/WAVE writer: f32 stereo → 16-bit PCM (what the clip
/// muxer wants and what postcards use anyway).
pub(crate) fn write_wav(path: &Path, samples: &[f32], rate: u32)
    -> std::io::Result<()>
{
    let mut file = std::io::BufWriter::new(std::fs::File::create(path)?);
    let data_bytes = (samples.len() * 2) as u32;
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + data_bytes).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?; // PCM
    file.write_all(&2u16.to_le_bytes())?; // stereo
    file.write_all(&rate.to_le_bytes())?;
    file.write_all(&(rate * 4).to_le_bytes())?;
    file.write_all(&4u16.to_le_bytes())?;
    file.write_all(&16u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&data_bytes.to_le_bytes())?;
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
        file.write_all(&value.to_le_bytes())?;
    }
    Ok(())
}

fn offline_pipeline(settings: &Settings, rate: u32, width: u32, height: u32)
    -> Result<(phosphor_dsp::Computer, phosphor_render_cpu::CpuRenderer),
              String>
{
    let computer = crate::render::build_computer(settings, rate)
        .map_err(|(_, m)| m)?;
    let mut renderer = phosphor_render_cpu::CpuRenderer::new(
        width as usize, height as usize, 1);
    renderer.beam_focus = settings.beam_focus;
    renderer.persistence = settings.persistence;
    renderer.theme = crate::render::build_theme(settings);
    renderer.grid_enabled = settings.grid_enabled;
    renderer.grid_spacing_fraction =
        phosphor_beam::grid_spacing_fraction(settings.gain);
    Ok((computer, renderer))
}

/// Snapshot: warm the glow on the tail, write the final frame as PNG.
/// Returns the written path. `cycle_t0` is the shell's beam-cycle
/// clock at the moment of the request — the PNG lands on the color
/// that was on screen (the warmup walks up to it).
pub fn save_snapshot(history: Vec<f32>, settings: Settings, rate: u32,
                     cycle_t0: f64)
                     -> Result<PathBuf, String> {
    if history.len() < 12_000 {
        return Err("nothing captured yet to export".into());
    }
    let (width, height) = export_size(&settings.display_mode);
    let (mut computer, mut renderer) =
        offline_pipeline(&settings, rate, width, height)?;
    let warmup_samples =
        ((SNAPSHOT_WARMUP_SECONDS * rate as f32) as usize * 2)
        .min(history.len());
    let tail = &history[history.len() - warmup_samples..];
    let per_frame = (rate / EXPORT_FPS) as usize * 2;
    let frame_count = tail.chunks(per_frame.max(2)).count().max(1);
    for (index, chunk) in tail.chunks(per_frame.max(2)).enumerate() {
        renderer.theme = crate::render::build_theme_at(
            &settings,
            cycle_t0
                - (frame_count - index) as f64 / f64::from(EXPORT_FPS));
        let segments = computer.compute(chunk, width as f32, height as f32);
        renderer.advance(segments);
    }
    renderer.theme = crate::render::build_theme_at(&settings, cycle_t0);
    let pixels = renderer.composite().to_vec();

    let directory = pictures_directory();
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let path = directory.join(format!("phosphor-{}.png", timestamp()));
    let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(
        std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut writer| writer.write_image_data(&pixels))
        .map_err(|e| e.to_string())?;
    Ok(path)
}

/// Clip: last 10 s re-rendered at 60 fps, muxed with its own audio.
/// `cycle_t0` = the beam-cycle clock at the clip's FIRST frame, so the
/// export re-lives the color sweep the user just watched.
pub fn save_clip(history: Vec<f32>, settings: Settings, rate: u32,
                 cycle_t0: f64)
                 -> Result<PathBuf, String> {
    if history.len() < 12_000 {
        return Err("nothing captured yet to export".into());
    }
    let (width, height) = export_size(&settings.display_mode);
    let (mut computer, mut renderer) =
        offline_pipeline(&settings, rate, width, height)?;

    let directory = pictures_directory();
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let stamp = timestamp();
    let wav_path = std::env::temp_dir()
        .join(format!("phosphor-clip-{stamp}.wav"));
    write_wav(&wav_path, &history, rate).map_err(|e| e.to_string())?;
    let out_path = directory.join(format!("phosphor-{stamp}.mp4"));

    let mut encoder = std::process::Command::new("ffmpeg")
        .args(["-y", "-loglevel", "error",
               "-f", "rawvideo", "-pix_fmt", "rgba",
               "-s", &format!("{width}x{height}"),
               "-r", &EXPORT_FPS.to_string(), "-i", "-"])
        .arg("-i").arg(&wav_path)
        .args(["-map", "0:v", "-map", "1:a",
               "-c:v", "libx264", "-preset", "veryfast", "-crf", "18",
               "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest"])
        .arg(&out_path)
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    let mut stdin = encoder.stdin.take().ok_or("ffmpeg stdin")?;

    let per_frame = (rate / EXPORT_FPS) as usize * 2;
    for (index, chunk) in history.chunks(per_frame.max(2)).enumerate() {
        renderer.theme = crate::render::build_theme_at(
            &settings, cycle_t0 + index as f64 / f64::from(EXPORT_FPS));
        let segments = computer.compute(chunk, width as f32, height as f32);
        renderer.advance(segments);
        if stdin.write_all(renderer.composite()).is_err() {
            break;
        }
    }
    drop(stdin);
    let encode_error = crate::render::stderr_tail(&mut encoder);
    let status = encoder.wait().map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&wav_path);
    if !status.success() {
        return Err(if encode_error.is_empty() {
            "clip encode failed".into()
        } else {
            format!("clip encode failed: {encode_error}")
        });
    }
    Ok(out_path)
}

/// Export a signal postcard (§13/§5.1-9b): decode the source to s16le
/// stereo at a compact audible rate, prepend the fit-trimmed 256-byte
/// header (title/credit/source/rate/frames — proto's pack_header,
/// golden-tested), write `.phos`. ffmpeg is the decode pipe (the same
/// contracted role it has in render/clip).
pub const POSTCARD_RATE: u32 = 48_000;

pub fn export_postcard(source: &Path, title: &str, credit: &str)
                       -> Result<PathBuf, String> {
    use phosphor_proto::phos::{self, Field};

    // decode → interleaved s16le stereo @ POSTCARD_RATE
    let output = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(source)
        .args(["-f", "s16le", "-ac", "2",
               "-ar", &POSTCARD_RATE.to_string(), "-"])
        .output()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    if !output.status.success() {
        return Err("could not decode the track".into());
    }
    let body = output.stdout;
    let frames = (body.len() / 4) as i64; // 2 ch × s16
    if frames == 0 {
        return Err("nothing to encode".into());
    }

    let source_name = source.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut fields: Vec<(String, Field)> = vec![
        ("rate".into(), Field::Int(POSTCARD_RATE as i64)),
        ("frames".into(), Field::Int(frames)),
        ("source".into(), Field::Text(source_name.clone())),
    ];
    if !title.trim().is_empty() {
        fields.push(("title".into(), Field::Text(title.trim().to_string())));
    }
    if !credit.trim().is_empty() {
        fields.push(("credit".into(),
                     Field::Text(credit.trim().to_string())));
    }
    let header = phos::pack_header(&fields).map_err(|e| e.0)?;

    let stem = title.trim().replace(['/', ' '], "-").to_lowercase();
    let stem = if stem.is_empty() { "postcard".to_string() } else { stem };
    let directory = pictures_directory();
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let path = directory.join(format!("{stem}-{}.phos", timestamp()));
    let mut file = std::fs::File::create(&path)
        .map_err(|e| e.to_string())?;
    file.write_all(&header).map_err(|e| e.to_string())?;
    file.write_all(&body).map_err(|e| e.to_string())?;
    Ok(path)
}

// ---------------------------------------------------------------------------
// The visitor (§16.1 — you know the code). Ported verbatim: nine
// ellipse outlines, paddle flap, off-left → off-right in 7 s.
// ---------------------------------------------------------------------------

pub const VISITOR_SWIM_SECONDS: f64 = 7.0;

pub fn visitor_segments(elapsed: f64, width: f32, height: f32)
                        -> Vec<[f32; 5]> {
    if elapsed > VISITOR_SWIM_SECONDS {
        return Vec::new();
    }
    let progress = (elapsed / VISITOR_SWIM_SECONDS) as f32;
    let center_x = width * (-0.18 + 1.36 * progress);
    let center_y = height
        * (0.5 + 0.055 * (elapsed * 2.1).sin() as f32);
    let scale = width.min(height) * 0.105;
    let paddle = ((elapsed * 5.0).sin() * 0.12) as f32;

    let mut segments = Vec::new();
    let mut ellipse = |ex: f32, ey: f32, rx: f32, ry: f32, points: u32| {
        let mut previous: Option<(f32, f32)> = None;
        for index in 0..=points {
            let angle = index as f32 / points as f32
                * std::f32::consts::TAU;
            let x = center_x + (ex + rx * angle.cos()) * scale;
            let y = center_y + (ey + ry * angle.sin()) * scale;
            if let Some((px, py)) = previous {
                segments.push([px, py, x, y, 0.85]);
            }
            previous = Some((x, y));
        }
    };
    ellipse(0.0, 0.0, 1.00, 0.72, 22);            // shell
    ellipse(0.0, 0.0, 0.62, 0.45, 22);            // shell pattern
    ellipse(1.28, 0.0, 0.30, 0.24, 22);           // head
    ellipse(1.36, -0.08, 0.05, 0.05, 8);          // eye
    ellipse(0.70, -0.64 - paddle, 0.34, 0.15, 22); // front-right flipper
    ellipse(0.70, 0.64 + paddle, 0.34, 0.15, 22);  // front-left flipper
    ellipse(-0.72, -0.66 + paddle, 0.28, 0.13, 22); // back-right flipper
    ellipse(-0.72, 0.66 - paddle, 0.28, 0.13, 22);  // back-left flipper
    ellipse(-1.16, 0.0, 0.13, 0.09, 10);          // tail
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_law_keeps_the_xy_dots_quirk() {
        assert_eq!(export_size("xy"), (720, 720));
        assert_eq!(export_size("xyz_takens"), (720, 720));
        assert_eq!(export_size("xy_dots"), (1080, 720), "wide — v3 quirk");
        assert_eq!(export_size("waveform"), (1080, 720));
    }

    #[test]
    fn visitor_swims_and_ends() {
        let mid = visitor_segments(3.5, 700.0, 700.0);
        assert!(!mid.is_empty());
        // nine closed paths: 8×22 + 8 + 10 segments
        assert_eq!(mid.len(), 22 * 7 + 8 + 10);
        assert!(visitor_segments(7.5, 700.0, 700.0).is_empty());
        // mid-swim the turtle is on screen
        let xs: Vec<f32> = mid.iter().map(|s| s[0]).collect();
        let min = xs.iter().cloned().fold(f32::MAX, f32::min);
        let max = xs.iter().cloned().fold(f32::MIN, f32::max);
        assert!(min > 0.0 && max < 700.0);
    }
}
