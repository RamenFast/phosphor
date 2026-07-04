// SPDX-License-Identifier: GPL-3.0-or-later
//! A5 live receipt: two apps play different tones; the mix capture
//! must contain BOTH frequencies (and a control bin must not).
//!
//! Usage: mix_probe <app-substr-a> <app-substr-b> (tones at 440/660)

use std::sync::mpsc;
use std::time::Duration;

use phosphor_audio::{AudioEngine, TargetKind};

/// Goertzel magnitude of one frequency bin.
fn goertzel(samples: &[f32], sample_rate: f32, frequency: f32) -> f64 {
    let omega = 2.0 * std::f64::consts::PI * frequency as f64 / sample_rate as f64;
    let coefficient = 2.0 * omega.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for sample in samples {
        let s0 = *sample as f64 + coefficient * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - coefficient * s1 * s2).sqrt() / samples.len() as f64
}

fn main() {
    let needle_a = std::env::args().nth(1).expect("app substr a");
    let needle_b = std::env::args().nth(2).expect("app substr b");
    let (event_sender, _events) = mpsc::channel();
    let engine = AudioEngine::spawn(48_000, event_sender).expect("engine");

    let targets = engine.targets();
    let combo = |needle: &str| {
        targets
            .iter()
            .find(|t| t.kind == TargetKind::App && t.label.contains(needle))
            .map(|t| t.combo_id())
            .unwrap_or_else(|| panic!("no app matches {needle:?}"))
    };
    let ids = vec![combo(&needle_a), combo(&needle_b)];
    let started = engine.start_capture_mix(&ids);
    println!("mixing {started} members: {ids:?}");
    assert_eq!(started, 2);

    std::thread::sleep(Duration::from_millis(400)); // streams settle
    let _ = engine.take_stereo_samples(); // discard warmup
    std::thread::sleep(Duration::from_millis(1500));
    let samples = engine.take_stereo_samples();
    let left: Vec<f32> = samples.chunks_exact(2).map(|f| f[0]).collect();
    println!("mixed {} frames", left.len());

    let m440 = goertzel(&left, 48_000.0, 440.0);
    let m660 = goertzel(&left, 48_000.0, 660.0);
    let m550 = goertzel(&left, 48_000.0, 550.0); // control: nothing here
    println!("magnitude 440={m440:.7} 660={m660:.7} control550={m550:.7}");
    // ≥47k frames: the ring's 1 s backlog law may trim to exactly 1 s.
    let ok = m440 > m550 * 10.0 && m660 > m550 * 10.0 && left.len() >= 47_000;
    println!("{}", if ok { "MIX PASS" } else { "MIX FAIL" });
    std::process::exit(if ok { 0 } else { 1 });
}
