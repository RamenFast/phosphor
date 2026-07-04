// SPDX-License-Identifier: GPL-3.0-or-later
//! A1/A2 live receipt: spawn the engine, list targets exactly the way
//! the shell will, then capture a target for a few seconds and report
//! what actually arrived (frames, RMS, dominant frequency estimate by
//! zero crossings). Run while a known tone plays to verify the whole
//! path: registry mirror → resolve → stream → ring → take.
//!
//! Usage: capture_probe [combo_id] [seconds]
//!   no args: list targets + default monitor id and exit.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use phosphor_audio::AudioEngine;

fn main() {
    let mut args = std::env::args().skip(1);
    let combo_id = args.next();
    let seconds: f32 = args
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(3.0);

    let (event_sender, event_receiver) = mpsc::channel();
    let engine = AudioEngine::spawn(48_000, event_sender).expect("engine spawn");

    println!("targets:");
    for target in engine.targets() {
        println!("  {:<44} {}", target.combo_id(), target.label);
    }
    println!(
        "default: {}",
        engine.default_monitor_target_id().unwrap_or_default()
    );

    let Some(combo_id) = combo_id else { return };
    assert!(
        engine.start_capture(&combo_id),
        "combo id did not resolve: {combo_id}"
    );

    let started = Instant::now();
    let mut samples: Vec<f32> = Vec::new();
    while started.elapsed() < Duration::from_secs_f32(seconds) {
        std::thread::sleep(Duration::from_millis(50));
        samples.extend(engine.take_stereo_samples());
        while let Ok(event) = event_receiver.try_recv() {
            println!("event: {event:?}");
        }
    }
    engine.stop_capture();

    let frames = samples.len() / 2;
    let mut sum_squares = 0.0f64;
    let mut crossings = 0u32;
    let mut previous_left = 0.0f32;
    for frame in samples.chunks_exact(2) {
        let left = frame[0];
        sum_squares += (left as f64) * (left as f64);
        if previous_left <= 0.0 && left > 0.0 {
            crossings += 1;
        }
        previous_left = left;
    }
    let rms = (sum_squares / frames.max(1) as f64).sqrt();
    let estimated_hz = crossings as f32 / seconds;
    println!(
        "captured {frames} frames in {seconds} s ({:.0} frames/s), rms {rms:.4}, ~{estimated_hz:.0} Hz",
        frames as f32 / seconds
    );
    let history = engine.copy_history(1.0);
    println!("history(1s): {} samples", history.len());
}
