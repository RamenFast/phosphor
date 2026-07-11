// SPDX-License-Identifier: GPL-3.0-or-later
//! UX-round receipt rig: reproduce "no audio on local playback after a
//! target switch / mix round-trip" (Ben, 2026-07-10) at the engine
//! layer, against the REAL PipeWire server (the nested rig reads false
//! negatives on audio — skill gotcha). Playback volume is forced to
//! 0.0 (our own data-callback gain), so nothing is audible on the
//! machine while the stream flows.
//!
//! The audible ring is 0.1 s deep, so `playback_position_seconds`
//! advancing across 1 s windows == the PW stream is really pulling.
//!
//! Stages (each prints PASS/FAIL):
//!   A  play → advances; pause → freezes; resume → advances
//!   B  pause → capture default monitor → stop capture → resume
//!   C  new start_file WHILE capture is live (track switch mid-scope)
//!   D  pause → mix attempt that connects 0 members → resume

use std::io::Write as _;
use std::sync::mpsc;
use std::time::Duration;

use phosphor_audio::AudioEngine;

fn write_test_wav(path: &std::path::Path, seconds: u32) {
    let rate = 48_000u32;
    let frames = rate * seconds;
    let data_bytes = frames * 4; // stereo s16le
    let mut out = Vec::with_capacity(44 + data_bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&2u16.to_le_bytes()); // stereo
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for i in 0..frames {
        let t = i as f32 / rate as f32;
        let sample = ((t * 440.0 * std::f32::consts::TAU).sin() * 0.2 * 32767.0) as i16;
        out.extend_from_slice(&sample.to_le_bytes());
        out.extend_from_slice(&sample.to_le_bytes());
    }
    std::fs::File::create(path)
        .expect("wav create")
        .write_all(&out)
        .expect("wav write");
}

fn advancing(engine: &AudioEngine, window: Duration) -> (f64, f64, bool) {
    let a = engine.playback_position_seconds();
    std::thread::sleep(window);
    let b = engine.playback_position_seconds();
    (a, b, b - a > 0.3)
}

fn stage(name: &str, ok: bool, detail: String) -> bool {
    println!("{}  {name}: {detail}", if ok { "PASS" } else { "FAIL" });
    ok
}

fn main() {
    let wav = std::env::temp_dir().join("phosphor-pause-resume-probe.wav");
    write_test_wav(&wav, 120);

    let (event_sender, event_receiver) = mpsc::channel();
    let engine = AudioEngine::spawn(48_000, event_sender).expect("engine spawn");
    engine.set_volume(0.0); // silent receipt — gain is our own callback
    std::thread::sleep(Duration::from_millis(600)); // mirror warm-up

    let monitor = engine
        .default_monitor_target_id()
        .expect("no default monitor — is PipeWire up?");
    println!("monitor target: {monitor}");

    let mut all = true;

    // Stage A — plain pause/resume
    engine.start_file(&wav, 0.0, false, false);
    let (a1, a2, adv) = advancing(&engine, Duration::from_secs(1));
    all &= stage("A1 playing advances", adv, format!("{a1:.2}->{a2:.2}"));
    engine.set_playback_paused(true);
    std::thread::sleep(Duration::from_millis(300)); // let the freeze land
    let (a3, a4, adv) = advancing(&engine, Duration::from_secs(1));
    all &= stage("A2 paused freezes", !adv, format!("{a3:.2}->{a4:.2}"));
    engine.set_playback_paused(false);
    let (a5, a6, adv) = advancing(&engine, Duration::from_secs(1));
    all &= stage("A3 resume advances", adv, format!("{a5:.2}->{a6:.2}"));

    // Stage B — the Ben gesture: pause for a target pick, capture,
    // stop capture, resume (TargetPicked → Space)
    engine.set_playback_paused(true);
    assert!(engine.start_capture(&monitor), "capture start failed");
    std::thread::sleep(Duration::from_millis(1500));
    engine.stop_capture();
    engine.set_playback_paused(false);
    let (b1, b2, adv) = advancing(&engine, Duration::from_secs(1));
    all &= stage("B  resume after capture round-trip", adv, format!("{b1:.2}->{b2:.2}"));

    // Stage C — track switch while a capture is LIVE (playlist click
    // with the beam on spotify)
    engine.set_playback_paused(true);
    assert!(engine.start_capture(&monitor), "capture restart failed");
    std::thread::sleep(Duration::from_millis(500));
    engine.start_file(&wav, 0.0, false, false); // new track, capture still on
    let (c1, c2, adv) = advancing(&engine, Duration::from_secs(1));
    all &= stage("C1 new track sounds under live capture", adv, format!("{c1:.2}->{c2:.2}"));
    engine.stop_capture();
    let (c3, c4, adv) = advancing(&engine, Duration::from_secs(1));
    all &= stage("C2 still sounds after capture stops", adv, format!("{c3:.2}->{c4:.2}"));

    // Stage D — failed mix attempt (0 members connect), then resume
    engine.set_playback_paused(true);
    let connected = engine.start_capture_mix(&[String::from("app:NoSuchAppXYZ")]);
    println!("mix members connected: {connected} (0 expected)");
    std::thread::sleep(Duration::from_millis(500));
    engine.set_playback_paused(false);
    let (d1, d2, adv) = advancing(&engine, Duration::from_secs(1));
    all &= stage("D  resume after empty mix", adv, format!("{d1:.2}->{d2:.2}"));

    engine.stop_capture();
    engine.stop_playback();
    while let Ok(event) = event_receiver.try_recv() {
        println!("event: {event:?}");
    }
    let _ = std::fs::remove_file(&wav);
    println!("{}", if all { "ALL STAGES PASS" } else { "STAGE FAILURES ABOVE" });
    std::process::exit(if all { 0 } else { 1 });
}
