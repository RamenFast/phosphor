// SPDX-License-Identifier: GPL-3.0-or-later
//! `phosphor bench` — the permanent regression gate. Uncapped offscreen
//! rendering of the SHA-pinned baseline signals through the full
//! compute → deposit → composite pipeline, both renderers, with the v4
//! gate table from BENCH.md checked in-code. A failed gate is a failed
//! process: this command exists so a perf regression cannot land quietly.
//!
//! Signals come from the wav files the v3 baseline ran (verified by
//! SHA-256 against tests/bench/results/v3-baseline.json). sweep and
//! chaos regenerate bit-identically when the files are gone; noise and
//! scene cannot (numpy PCG64 / studio compiler) — their runs are
//! skipped with a notice, and any GATE that loses its signal fails
//! loudly rather than vanishing.
//!
//! GPU timing is submission-side per frame with a hard sync every 32
//! frames and a final full sync inside the wall clock — throughput,
//! not per-frame latency; percentiles on the GPU rows read accordingly.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

use crate::render::{self, RendererKind};
use crate::signals;
use phosphor_proto::settings::Settings;

const BASELINE_RESULTS: &str = "tests/bench/results/v3-baseline.json";
const SIGNAL_DIRECTORY: &str = "/tmp/phosphor-bench";
const SYNC_EVERY: u64 = 32;

struct RunSpec {
    name: &'static str,
    renderer: RendererKind,
    width: u32,
    height: u32,
    supersample: u32,
    signal: &'static str,
    rate: u32,
    /// v4 gate floor in fps-equivalent (None = informational row).
    gate: Option<f64>,
}

/// The v4 gate rows from BENCH.md ("What v4 must beat"), plus context.
const RUNS: [RunSpec; 7] = [
    RunSpec { name: "offline-96k-sweep", renderer: RendererKind::Cpu,
              width: 720, height: 720, supersample: 1, signal: "sweep",
              rate: 96000, gate: Some(171.0) },
    RunSpec { name: "offline-384k-sweep", renderer: RendererKind::Cpu,
              width: 720, height: 720, supersample: 1, signal: "sweep",
              rate: 384000, gate: Some(79.0) },
    RunSpec { name: "cpu-live-noise-384k", renderer: RendererKind::Cpu,
              width: 2560, height: 1440, supersample: 1,
              signal: "noise", rate: 384000, gate: Some(6.0) },
    RunSpec { name: "gpu-max-sweep-384k", renderer: RendererKind::Gpu,
              width: 2560, height: 1440, supersample: 2,
              signal: "sweep", rate: 384000, gate: Some(326.0) },
    RunSpec { name: "gpu-max-noise-384k", renderer: RendererKind::Gpu,
              width: 2560, height: 1440, supersample: 2,
              signal: "noise", rate: 384000, gate: None },
    RunSpec { name: "gpu-max-chaos-384k", renderer: RendererKind::Gpu,
              width: 2560, height: 1440, supersample: 2,
              signal: "chaos", rate: 384000, gate: None },
    RunSpec { name: "gpu-offline-384k-sweep",
              renderer: RendererKind::Gpu, width: 720, height: 720,
              supersample: 1, signal: "sweep", rate: 384000,
              gate: None },
];

struct SignalSource {
    path: PathBuf,
    sha256: String,
    verified: bool,
    regenerated: bool,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Hash 4 KB from the frame's CENTER — the corners are background and
/// grid, identical across signals; the beam lives in the middle.
fn frame_checksum(frame: &[u8]) -> String {
    let start = (frame.len() / 2).saturating_sub(2048);
    let end = (start + 4096).min(frame.len());
    hex(&Sha256::digest(&frame[start..end]))[..16].to_string()
}

fn baseline_signal_hashes() -> std::collections::HashMap<String, String> {
    let mut hashes = std::collections::HashMap::new();
    let Ok(text) = std::fs::read_to_string(BASELINE_RESULTS) else {
        return hashes;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
    else {
        return hashes;
    };
    if let Some(live) = value["live"].as_object() {
        for run in live.values() {
            if let (Some(name), Some(sha)) =
                (run["signal"]["name"].as_str(),
                 run["signal"]["sha256"].as_str()) {
                hashes.insert(name.to_string(), sha.to_string());
            }
        }
    }
    hashes
}

fn locate_signal(name: &str,
                 baseline: &std::collections::HashMap<String, String>)
                 -> Option<SignalSource> {
    let recorded = Path::new(SIGNAL_DIRECTORY)
        .join(format!("signal-{name}-240s.wav"));
    if recorded.exists() {
        let digest = hex(&Sha256::digest(
            std::fs::read(&recorded).ok()?));
        let verified = baseline.get(name) == Some(&digest);
        return Some(SignalSource { path: recorded, sha256: digest,
                                   verified, regenerated: false });
    }
    if signals::regenerable(name) {
        let directory = std::env::temp_dir().join("phosphor-bench-rust");
        std::fs::create_dir_all(&directory).ok()?;
        let path = directory.join(format!("signal-{name}.wav"));
        if !path.exists() {
            signals::write_wav(name, 30, &path).ok()?;
        }
        let digest = hex(&Sha256::digest(std::fs::read(&path).ok()?));
        return Some(SignalSource { path, sha256: digest,
                                   verified: false, regenerated: true });
    }
    None
}

/// Decode `seconds` of a wav to interleaved f32 at `rate` via ffmpeg —
/// the same pipe the render command uses, so the bench measures the
/// pipeline the product ships.
fn decode_signal(path: &Path, rate: u32, seconds: u32)
                 -> Result<Vec<f32>, String> {
    let mut child = Command::new("ffmpeg")
        .args(["-v", "error", "-t", &seconds.to_string()])
        .arg("-i").arg(path)
        .args(["-f", "f32le", "-ac", "2", "-ar",
               &rate.to_string(), "-"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("ffmpeg: {error}"))?;
    let mut bytes = Vec::new();
    child.stdout.take().expect("stdout")
        .read_to_end(&mut bytes)
        .map_err(|error| format!("ffmpeg read: {error}"))?;
    if !child.wait().map(|status| status.success()).unwrap_or(false) {
        return Err("ffmpeg decode failed".into());
    }
    Ok(bytes.chunks_exact(4)
       .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2],
                                        chunk[3]]))
       .collect())
}

struct RunResult {
    frames: u64,
    fps_equivalent: f64,
    fps_p50: f64,
    fps_p5: f64,
    fps_p1: f64,
    checksum: String,
    shed_supersample: bool,
}

fn execute(spec: &RunSpec, samples: &[f32], seconds: u32)
           -> Result<RunResult, String> {
    let settings = Settings {
        display_mode: "xy".into(),
        gl_supersample: spec.supersample,
        ..Settings::default()
    };
    let (mut computer, mut sink) = render::build_pipeline(
        &settings, spec.rate, spec.width, spec.height, spec.renderer)
        .map_err(|(_, message)| message)?;

    let chunk_floats = (spec.rate / render::EXPORT_FPS) as usize * 2;
    let frame_budget = u64::from(seconds) * u64::from(render::EXPORT_FPS);
    let available = (samples.len() / chunk_floats) as u64;
    let frames = frame_budget.min(available);
    if frames == 0 {
        return Err("signal shorter than one frame".into());
    }

    let mut times = Vec::with_capacity(frames as usize);
    let mut checksum = String::new();
    let mut shed = false;
    let wall = std::time::Instant::now();
    for index in 0..frames {
        let start = (index as usize) * chunk_floats;
        let frame_started = std::time::Instant::now();
        let segments = computer.compute(
            &samples[start..start + chunk_floats],
            spec.width as f32, spec.height as f32);
        match &mut sink {
            render::FrameSink::Cpu(renderer) => {
                renderer.advance(segments);
                let frame = renderer.composite();
                if index == frames - 1 {
                    checksum = frame_checksum(frame);
                }
            }
            render::FrameSink::Gpu(renderer) => {
                renderer.advance(segments);
                renderer.composite_submit();
                if index % SYNC_EVERY == SYNC_EVERY - 1 {
                    renderer.wait_idle();
                }
                shed = renderer.shed_supersample;
            }
        }
        times.push(frame_started.elapsed().as_secs_f64());
    }
    if let render::FrameSink::Gpu(renderer) = &mut sink {
        let frame = renderer.composite_and_read();   // full sync + pixels
        checksum = frame_checksum(&frame);
    }
    let total = wall.elapsed().as_secs_f64();

    times.sort_unstable_by(f64::total_cmp);
    let percentile_time = |fraction: f64| {
        let index = ((times.len() as f64 * fraction) as usize)
            .min(times.len() - 1);
        times[index]
    };
    Ok(RunResult {
        frames,
        fps_equivalent: frames as f64 / total,
        fps_p50: 1.0 / percentile_time(0.50).max(1e-9),
        fps_p5: 1.0 / percentile_time(0.95).max(1e-9),
        fps_p1: 1.0 / percentile_time(0.99).max(1e-9),
        checksum,
        shed_supersample: shed,
    })
}

pub fn run(arguments: &[String]) -> i32 {
    let mut seconds: u32 = 4;
    let mut json = false;
    let mut iterator = arguments.iter();
    while let Some(argument) = iterator.next() {
        match argument.as_str() {
            "--seconds" => {
                seconds = match iterator.next()
                    .and_then(|value| value.parse().ok()) {
                    Some(value) => value,
                    None => {
                        eprintln!("usage: phosphor bench [--seconds N] \
                                   [--output json]");
                        return 2;
                    }
                };
            }
            "--output" => {
                json = matches!(iterator.next().map(String::as_str),
                                Some("json"));
            }
            _ => {
                eprintln!("usage: phosphor bench [--seconds N] \
                           [--output json]");
                return 2;
            }
        }
    }

    let baseline = baseline_signal_hashes();
    let mut decoded: std::collections::HashMap<(String, u32), Vec<f32>> =
        std::collections::HashMap::new();
    let mut sources: std::collections::HashMap<String, SignalSource> =
        std::collections::HashMap::new();

    let mut runs_json = Vec::new();
    let mut gates = serde_json::Map::new();
    let mut all_pass = true;

    for spec in &RUNS {
        let source = match sources.get(spec.signal) {
            Some(_) => sources.get(spec.signal),
            None => {
                if let Some(found) = locate_signal(spec.signal,
                                                   &baseline) {
                    sources.insert(spec.signal.to_string(), found);
                }
                sources.get(spec.signal)
            }
        };
        let Some(source) = source else {
            eprintln!("  {}: signal '{}' unavailable (not recorded, \
                       not regenerable)", spec.name, spec.signal);
            if let Some(required) = spec.gate {
                gates.insert(spec.name.to_string(), serde_json::json!({
                    "required": required, "measured": null,
                    "pass": false, "reason": "signal unavailable",
                }));
                all_pass = false;
            }
            continue;
        };

        let key = (spec.signal.to_string(), spec.rate);
        if !decoded.contains_key(&key) {
            match decode_signal(&source.path, spec.rate, seconds) {
                Ok(samples) => { decoded.insert(key.clone(), samples); }
                Err(error) => {
                    eprintln!("  {}: {error}", spec.name);
                    if let Some(required) = spec.gate {
                        gates.insert(spec.name.to_string(),
                                     serde_json::json!({
                            "required": required, "measured": null,
                            "pass": false, "reason": error,
                        }));
                        all_pass = false;
                    }
                    continue;
                }
            }
        }
        let samples = &decoded[&key];

        match execute(spec, samples, seconds) {
            Ok(result) => {
                eprintln!("  {:26} {:8.1} fps-eq  (p50 {:.1} · p5 \
                           {:.1} · p1 {:.1})",
                          spec.name, result.fps_equivalent,
                          result.fps_p50, result.fps_p5, result.fps_p1);
                if let Some(required) = spec.gate {
                    let pass = result.fps_equivalent >= required;
                    all_pass &= pass;
                    gates.insert(spec.name.to_string(),
                                 serde_json::json!({
                        "required": required,
                        "measured": (result.fps_equivalent * 10.0)
                            .round() / 10.0,
                        "pass": pass,
                    }));
                }
                runs_json.push(serde_json::json!({
                    "name": spec.name,
                    "renderer": match spec.renderer {
                        RendererKind::Gpu => "gpu",
                        RendererKind::Cpu => "cpu",
                    },
                    "size": format!("{}x{}", spec.width, spec.height),
                    "supersample": spec.supersample,
                    "shed_supersample": result.shed_supersample,
                    "signal": {
                        "name": spec.signal,
                        "sha256": source.sha256,
                        "verified_against_baseline": source.verified,
                        "regenerated": source.regenerated,
                    },
                    "rate": spec.rate,
                    "frames": result.frames,
                    "fps_equivalent": (result.fps_equivalent * 10.0)
                        .round() / 10.0,
                    "fps_p50": (result.fps_p50 * 10.0).round() / 10.0,
                    "fps_p5": (result.fps_p5 * 10.0).round() / 10.0,
                    "fps_p1": (result.fps_p1 * 10.0).round() / 10.0,
                    "checksum": result.checksum,
                }));
            }
            Err(error) => {
                eprintln!("  {}: FAILED: {error}", spec.name);
                if let Some(required) = spec.gate {
                    gates.insert(spec.name.to_string(),
                                 serde_json::json!({
                        "required": required, "measured": null,
                        "pass": false, "reason": error,
                    }));
                    all_pass = false;
                }
            }
        }
    }

    let report = serde_json::json!({
        "seconds": seconds,
        "runs": runs_json,
        "gates": gates,
        "all_gates_pass": all_pass,
    });
    if json {
        println!("{}", serde_json::to_string_pretty(&report)
                 .expect("json"));
    } else {
        eprintln!("gates: {}", if all_pass { "ALL PASS" } else {
            "FAILURES — see JSON" });
        println!("{}", serde_json::to_string_pretty(&report["gates"])
                 .expect("json"));
    }
    if all_pass { 0 } else { 1 }
}
