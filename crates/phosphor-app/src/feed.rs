// SPDX-License-Identifier: GPL-3.0-or-later
//! `phosphor feed` — the headless panel feed for the Cinnamon applet.
//! One JSON line of beam segments per frame on stdout; mode/fps/quit
//! commands on stdin. Protocol and constants are VERBATIM from v3's
//! phosphor_applet_feed.py (the applet's paint code is unchanged) —
//! see APPLET-PLAN.md. The applet owns this process: stdin EOF or a
//! broken stdout pipe are both a clean exit.
//!
//! One deliberate deviation from v3: on the capture-loss retry path we
//! re-resolve the default monitor FRESH each tick, so the feed follows
//! a default-sink change instead of going dark (v3 went silent when the
//! default output moved).

use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use phosphor_audio::{AudioEngine, AudioEvent};
use phosphor_dsp::{Computer, Mode};

const COORDINATE_BOX: f32 = 1000.0;
const DEFAULT_FPS: u32 = 30;
const CAPTURE_SAMPLE_RATE: u32 = 16_000;
const MAX_SEGMENTS_PER_FRAME: usize = 500;
const AGC_TARGET_FILL: f32 = 0.9;
const AGC_NOISE_FLOOR: f32 = 0.005;
const AGC_MAX_GAIN: f32 = 40.0;
const AGC_RELEASE: f32 = 0.92;

const VALID_MODES: &[&str] = &["xy", "xy45", "xy_swirl", "xy_dots",
    "waveform", "ring", "spectrum", "spectrum_radial", "tunnel"];

enum Command {
    Mode(String),
    Fps(u32),
    Quit,
}

/// Parse one stdin command line into a `Command`. Pure for testing:
/// `quit` → Quit; `fps <n>` clamps to 5..=480 (garbage ignored like
/// Python's ValueError swallow); `mode <m>` returns the RAW name —
/// validity is checked at apply time against `VALID_MODES`. Unknown or
/// empty lines yield None.
fn parse_command(line: &str) -> Option<Command> {
    let mut parts = line.split_whitespace();
    match parts.next()? {
        "quit" => Some(Command::Quit),
        "mode" => parts.next().map(|name| Command::Mode(name.to_string())),
        "fps" => parts.next()
            .and_then(|value| value.parse::<u32>().ok())
            .map(|value| Command::Fps(value.clamp(5, 480))),
        _ => None,
    }
}

/// stdin reader thread → channel. Parses each line; sends Quit on the
/// `quit` command AND on EOF (the applet closed the pipe). Never joined
/// — main just returns and process exit reaps it. Every send is
/// best-effort: the receiver may already be gone.
fn spawn_stdin_reader() -> mpsc::Receiver<Command> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if let Some(command) = parse_command(&line) {
                let is_quit = matches!(command, Command::Quit);
                let _ = sender.send(command);
                if is_quit {
                    return;
                }
            }
        }
        // EOF (or a read error): the applet is gone — ask main to quit.
        let _ = sender.send(Command::Quit);
    });
    receiver
}

/// One AGC step. Returns `(tracked_peak, gain)` for the next frame:
/// instant attack (a louder frame snaps the tracked peak up), geometric
/// release otherwise; above the noise floor the gain fills to target,
/// below it the gain eases back toward unity (never amplify silence).
fn agc_step(samples: &[f32], tracked_peak: f32, gain: f32) -> (f32, f32) {
    let frame_peak = samples.iter().fold(0.0_f32, |peak, value| peak.max(value.abs()));
    let tracked = if frame_peak > tracked_peak {
        frame_peak
    } else {
        tracked_peak * AGC_RELEASE + frame_peak * (1.0 - AGC_RELEASE)
    };
    let gain = if tracked > AGC_NOISE_FLOOR {
        (AGC_TARGET_FILL / tracked).clamp(1.0, AGC_MAX_GAIN)
    } else {
        gain * 0.9 + 0.1
    };
    (tracked, gain)
}

/// Flatten beam segments to the compact int run the applet expects.
/// Past the cap, stride-downsample (v3 law: thin the whole shape, never
/// cut its tail) — matching Python's `int()` truncation with the first
/// segment always kept. Coordinates round to nearest; intensity is
/// `round(i * 255)` clamped to 0..=255.
fn encode_segments(segments: &[[f32; 5]]) -> Vec<i64> {
    let picked: Vec<[f32; 5]> = if segments.len() > MAX_SEGMENTS_PER_FRAME {
        let step = segments.len() as f64 / MAX_SEGMENTS_PER_FRAME as f64;
        (0..MAX_SEGMENTS_PER_FRAME)
            .map(|index| segments[(index as f64 * step) as usize])
            .collect()
    } else {
        segments.to_vec()
    };
    picked.iter()
        .flat_map(|[x0, y0, x1, y1, intensity]| {
            [
                x0.round() as i64,
                y0.round() as i64,
                x1.round() as i64,
                y1.round() as i64,
                ((intensity * 255.0).round() as i64).clamp(0, 255),
            ]
        })
        .collect()
}

/// Emit one JSON line; flush; false on ANY error (EPIPE = reader gone).
/// No unwraps and no println! — println panics on EPIPE, this must not.
fn write_line(out: &mut impl Write, payload: &serde_json::Value) -> bool {
    writeln!(out, "{payload}").is_ok() && out.flush().is_ok()
}

pub fn run(arguments: &[String]) -> i32 {
    let mut out = std::io::stdout().lock();

    // --fps N (clamp 5..=480, default 30; garbage ignored — Python parity).
    let mut fps = DEFAULT_FPS;
    let mut index = 0;
    while index < arguments.len() {
        if arguments[index] == "--fps" {
            if let Some(value) = arguments.get(index + 1)
                .and_then(|value| value.parse::<u32>().ok()) {
                fps = value.clamp(5, 480);
            }
            index += 2;
        } else {
            index += 1;
        }
    }

    let (event_sender, events) = mpsc::channel();
    let engine = match AudioEngine::spawn(CAPTURE_SAMPLE_RATE, event_sender) {
        Ok(engine) => engine,
        Err(message) => {
            write_line(&mut out, &serde_json::json!({ "error": message }));
            return 3;
        }
    };

    let monitor_id = match engine.default_monitor_target_id() {
        Some(id) => id,
        None => {
            write_line(&mut out,
                &serde_json::json!({ "error": "no output monitor source found" }));
            return 3;
        }
    };
    // A ready combo id — do NOT split on ':'. A false here is a
    // near-impossible race (target vanished between resolve and start),
    // not fatal: fall into the retry state below.
    let mut capture_alive = engine.start_capture(&monitor_id);
    let mut next_retry: Option<Instant> = if capture_alive { None } else { Some(Instant::now()) };

    let mut computer = Computer::new();
    computer.mode = Mode::Xy;
    computer.set_sample_rate(CAPTURE_SAMPLE_RATE, 1);

    let commands = spawn_stdin_reader();

    let mut interval = Duration::from_secs_f64(1.0 / fps as f64);
    let mut tracked_peak: f32 = 0.0;
    let mut current_mode = String::from("xy");

    loop {
        let frame_start = Instant::now();

        // (1) Drain commands.
        while let Ok(command) = commands.try_recv() {
            match command {
                Command::Quit => return 0,
                Command::Fps(value) => {
                    interval = Duration::from_secs_f64(1.0 / value as f64);
                }
                Command::Mode(name) => {
                    if VALID_MODES.contains(&name.as_str())
                        && name != current_mode
                        && let Ok(mode) = name.parse::<Mode>() {
                        computer.mode = mode;
                        computer.reset();
                        current_mode = name;
                    }
                }
            }
        }

        // (2) Drain engine events. StreamEnded and DefaultSinkChanged
        // both drop us into the retry path: re-resolving the default
        // monitor on the retry tick makes the feed follow a new default
        // sink — the deliberate v4 improvement (v3 went dark instead).
        while let Ok(event) = events.try_recv() {
            match event {
                AudioEvent::StreamEnded | AudioEvent::DefaultSinkChanged => {
                    capture_alive = false;
                    next_retry = Some(Instant::now());
                }
                // Targets/playback/track events don't affect the panel
                // feed — it only ever captures the default monitor.
                _ => {}
            }
        }

        // (3) Retry tick: re-query the default monitor FRESH so we
        // follow a moved default sink, then try to restart capture.
        if !capture_alive
            && let Some(due) = next_retry
            && frame_start >= due {
            match engine.default_monitor_target_id() {
                Some(id) if engine.start_capture(&id) => {
                    capture_alive = true;
                    next_retry = None;
                }
                _ => next_retry = Some(frame_start + Duration::from_secs(1)),
            }
        }

        // (4) Samples, (5) AGC.
        let samples = engine.take_stereo_samples();
        let (new_peak, gain) = agc_step(&samples, tracked_peak, computer.gain);
        tracked_peak = new_peak;
        computer.gain = gain;

        // (6) Compute + encode. Flatten immediately — compute() returns
        // a slice borrowed from `computer`.
        let encoded = encode_segments(
            computer.compute(&samples, COORDINATE_BOX, COORDINATE_BOX));

        // (7) Always emit a frame each tick, even {"s":[]} while silent
        // or not capturing (the applet's decay handles the fade).
        if !write_line(&mut out, &serde_json::json!({ "s": encoded })) {
            return 0;
        }

        // (8) Pace.
        let elapsed = frame_start.elapsed();
        std::thread::sleep(interval.saturating_sub(elapsed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_rounds_coordinates() {
        let segments = [[499.6, 0.4, 250.5, 1.49, 0.0]];
        let encoded = encode_segments(&segments);
        assert_eq!(&encoded[..4], &[500, 0, 251, 1]);
    }

    #[test]
    fn encode_clamps_intensity() {
        assert_eq!(encode_segments(&[[0.0, 0.0, 0.0, 0.0, 1.2]])[4], 255);
        assert_eq!(encode_segments(&[[0.0, 0.0, 0.0, 0.0, -0.1]])[4], 0);
        // A mid intensity rounds through *255.
        assert_eq!(encode_segments(&[[0.0, 0.0, 0.0, 0.0, 0.5]])[4], 128);
    }

    #[test]
    fn encode_passthrough_at_cap() {
        let segments = vec![[1.0, 2.0, 3.0, 4.0, 0.0]; 500];
        assert_eq!(encode_segments(&segments).len(), 500 * 5);
    }

    #[test]
    fn encode_strides_over_cap() {
        let segments: Vec<[f32; 5]> = (0..1000)
            .map(|i| [i as f32, 0.0, 0.0, 0.0, 0.0])
            .collect();
        let encoded = encode_segments(&segments);
        assert_eq!(encoded.len(), 500 * 5);
        // First segment always kept (x0 of segment 0 == 0).
        assert_eq!(encoded[0], 0);
        // step = 1000/500 = 2.0 → picks 0, 2, 4, …
        assert_eq!(encoded[5], 2);
        assert_eq!(encoded[10], 4);
    }

    #[test]
    fn encode_edge_at_501() {
        let segments = vec![[0.0, 0.0, 0.0, 0.0, 0.0]; 501];
        assert_eq!(encode_segments(&segments).len(), 500 * 5);
    }

    #[test]
    fn encode_empty_is_empty() {
        assert!(encode_segments(&[]).is_empty());
    }

    #[test]
    fn parse_quit() {
        assert!(matches!(parse_command("quit"), Some(Command::Quit)));
    }

    #[test]
    fn parse_fps_clamps() {
        assert!(matches!(parse_command("fps 1000"), Some(Command::Fps(480))));
        assert!(matches!(parse_command("fps 1"), Some(Command::Fps(5))));
        assert!(matches!(parse_command("fps 60"), Some(Command::Fps(60))));
        assert!(parse_command("fps abc").is_none());
    }

    #[test]
    fn parse_mode_raw() {
        match parse_command("mode waveform") {
            Some(Command::Mode(name)) => assert_eq!(name, "waveform"),
            _ => panic!("expected Mode"),
        }
        // Raw name returned even for a 3D mode — validity is apply-time.
        match parse_command("mode xyz_takens") {
            Some(Command::Mode(name)) => assert_eq!(name, "xyz_takens"),
            _ => panic!("expected Mode"),
        }
    }

    #[test]
    fn valid_modes_excludes_3d() {
        assert!(!VALID_MODES.contains(&"xyz_takens"));
        assert!(!VALID_MODES.contains(&"helix"));
        // ...but those do parse as real Modes — hence the string gate.
        assert!("xyz_takens".parse::<Mode>().is_ok());
        assert!("helix".parse::<Mode>().is_ok());
    }

    #[test]
    fn parse_garbage_is_none() {
        assert!(parse_command("").is_none());
        assert!(parse_command("   ").is_none());
        assert!(parse_command("nonsense here").is_none());
        assert!(parse_command("mode").is_none());
        assert!(parse_command("fps").is_none());
    }

    #[test]
    fn agc_instant_attack() {
        let samples = [0.5_f32, -0.3];
        let (tracked, gain) = agc_step(&samples, 0.1, 1.0);
        assert!((tracked - 0.5).abs() < 1e-6);
        assert!((gain - 1.8).abs() < 1e-5); // 0.9 / 0.5
    }

    #[test]
    fn agc_geometric_release() {
        // Silence after a peak: tracked decays by 0.92.
        let (tracked, _) = agc_step(&[0.0_f32], 0.4, 5.0);
        assert!((tracked - 0.4 * 0.92).abs() < 1e-6);
    }

    #[test]
    fn agc_gain_clamps_at_max() {
        // A tiny-but-above-floor peak wants huge gain; clamp at 40.
        let (_, gain) = agc_step(&[0.01_f32], 0.01, 1.0);
        assert!((gain - AGC_MAX_GAIN).abs() < 1e-4);
    }

    #[test]
    fn agc_eases_to_unity_below_floor() {
        // Below the noise floor the gain eases toward 1.0: 5.0 → 4.6.
        let (_, gain) = agc_step(&[0.0_f32], 0.0, 5.0);
        assert!((gain - 4.6).abs() < 1e-5);
    }

    #[test]
    fn agc_full_scale_never_below_unity() {
        let (_, gain) = agc_step(&[1.0_f32], 0.9, 1.0);
        assert!(gain >= 1.0);
        assert!((gain - 1.0).abs() < 1e-6);
    }
}
