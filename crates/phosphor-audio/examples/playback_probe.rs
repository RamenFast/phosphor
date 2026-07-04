// SPDX-License-Identifier: GPL-3.0-or-later
//! A3 live receipt. Exercises the playback engine end-to-end against
//! the real PipeWire server and prints PASS/FAIL per law:
//!
//!   1. audible: position clock advances ~realtime, pause freezes it,
//!      resume continues (no burst), our node exists on the graph.
//!   2. vacuum: no playback node, yet the scope ring fills at ~realtime
//!      (the rolling-deadline law) and pause freezes the clock.
//!   3. gapless: TrackStarted twice, PlaybackEnded once, no cold gap.
//!   4. metadata + cover art from tags; .phos plays at header rate.
//!
//! Usage: playback_probe <a.mp3> <b.flac> [postcard.phos]

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use phosphor_audio::{AudioEngine, AudioEvent};

fn check(name: &str, ok: bool, detail: String) -> bool {
    println!("{} {name}: {detail}", if ok { "PASS" } else { "FAIL" });
    ok
}

fn main() {
    let mut args = std::env::args().skip(1);
    let file_a = PathBuf::from(args.next().expect("need file a"));
    let file_b = PathBuf::from(args.next().expect("need file b"));
    let phos = args.next().map(PathBuf::from);

    let (event_sender, event_receiver) = mpsc::channel();
    let engine = AudioEngine::spawn(48_000, event_sender).expect("engine");
    let mut all_ok = true;

    // ---- 1. audible playback + pause freeze -------------------------------
    engine.set_volume(0.25); // be gentle to the room
    engine.start_file(&file_a, 0.0, true, false); // loop so it never ends
    std::thread::sleep(Duration::from_millis(1200));
    let p1 = engine.playback_position_seconds();
    all_ok &= check("audible position advances", (0.6..2.5).contains(&p1),
                    format!("{p1:.2}s after 1.2s wall"));
    let has_own_node = engine
        .targets()
        .iter()
        .any(|t| t.label.contains("Phosphor"));
    all_ok &= check("playback node on graph", has_own_node,
                    "APP · Phosphor visible (v3: pacat did too)".into());

    engine.set_playback_paused(true);
    std::thread::sleep(Duration::from_millis(300));
    let p2 = engine.playback_position_seconds();
    std::thread::sleep(Duration::from_millis(700));
    let p3 = engine.playback_position_seconds();
    all_ok &= check("pause freezes clock", (p3 - p2).abs() < 0.15,
                    format!("drift {:.3}s while paused", p3 - p2));
    engine.set_playback_paused(false);
    std::thread::sleep(Duration::from_millis(600));
    let p4 = engine.playback_position_seconds();
    all_ok &= check("resume continues", p4 > p3 + 0.3 && p4 < p3 + 1.5,
                    format!("{:.2}s → {:.2}s over 0.6s wall", p3, p4));
    let metadata = engine.current_track_metadata().unwrap_or_default();
    all_ok &= check("metadata title", metadata.title.as_deref() == Some("Alpha"),
                    format!("{:?} / {:?}", metadata.title, metadata.artist));
    let art = engine.current_cover_art();
    all_ok &= check("cover art present", art.is_some(),
                    format!("{:?} bytes",
                            art.as_ref().map(|a| a.data.len())));
    engine.stop_playback();
    let ended_early: Vec<_> = event_receiver.try_iter()
        .filter(|e| *e == AudioEvent::PlaybackEnded).collect();
    all_ok &= check("explicit stop is silent", ended_early.is_empty(),
                    format!("{} PlaybackEnded events", ended_early.len()));

    // ---- 2. vacuum ---------------------------------------------------------
    let drained = engine.take_stereo_samples(); // clear
    drop(drained);
    engine.start_file(&file_a, 0.0, true, true);
    let t0 = Instant::now();
    std::thread::sleep(Duration::from_millis(1500));
    let vacuum_pos = engine.playback_position_seconds();
    let wall = t0.elapsed().as_secs_f64();
    all_ok &= check("vacuum paces ~realtime",
                    (vacuum_pos - wall).abs() < 0.35,
                    format!("pos {vacuum_pos:.2}s vs wall {wall:.2}s"));
    let scope_samples = engine.take_stereo_samples();
    all_ok &= check("vacuum feeds the scope", scope_samples.len() > 48_000,
                    format!("{} samples pending", scope_samples.len()));
    let no_own_node = !engine
        .targets()
        .iter()
        .any(|t| t.label.contains("Phosphor"));
    all_ok &= check("vacuum has no playback node", no_own_node,
                    "silent — light only".into());
    engine.set_playback_paused(true);
    let v1 = engine.playback_position_seconds();
    std::thread::sleep(Duration::from_millis(600));
    let v2 = engine.playback_position_seconds();
    all_ok &= check("vacuum pause freezes clock", (v2 - v1).abs() < 0.1,
                    format!("drift {:.3}s", v2 - v1));
    engine.set_playback_paused(false);
    std::thread::sleep(Duration::from_millis(500));
    let v3 = engine.playback_position_seconds();
    all_ok &= check("vacuum resume no burst", v3 - v2 < 0.9,
                    format!("advanced {:.2}s over 0.5s wall", v3 - v2));
    engine.stop_playback();

    // ---- 3. gapless splice -------------------------------------------------
    while event_receiver.try_recv().is_ok() {}
    engine.start_file(&file_a, 0.0, false, true); // vacuum: fast + silent
    engine.set_next_track(Some(file_b.clone()));
    let mut started = Vec::new();
    let mut ended = 0;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        match event_receiver.recv_timeout(Duration::from_millis(200)) {
            Ok(AudioEvent::TrackStarted { path }) => started.push(path),
            Ok(AudioEvent::PlaybackEnded) => {
                ended += 1;
                break;
            }
            _ => {}
        }
    }
    all_ok &= check("gapless: two TrackStarted", started.len() == 2,
                    format!("{:?}", started.iter()
                            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
                            .collect::<Vec<_>>()));
    all_ok &= check("gapless: one PlaybackEnded", ended == 1, format!("{ended}"));
    engine.stop_playback();

    // ---- 4. .phos ----------------------------------------------------------
    if let Some(phos_path) = phos {
        let metadata = phosphor_audio::probe_metadata(&phos_path);
        println!("phos metadata: {metadata:?}");
        let duration = metadata.duration.unwrap_or(0.0);
        engine.start_file(&phos_path, 0.0, false, true);
        std::thread::sleep(Duration::from_millis(800));
        let pos = engine.playback_position_seconds();
        let scope = engine.take_stereo_samples();
        // The postcard is short: position must park at its true end and
        // the scope must have received exactly its length at pipe rate.
        let expected_samples = (duration * 48_000.0 * 2.0) as usize;
        let sample_error = (scope.len() as i64 - expected_samples as i64).abs();
        all_ok &= check(".phos honors header rate/length",
                        (pos - duration).abs() < 0.05
                            && sample_error < 4_800,
                        format!("pos {pos:.2}s of {duration:.2}s, {} samples \
                                 (expected ~{expected_samples})", scope.len()));
        engine.stop_playback();
    }

    println!("{}", if all_ok { "ALL PASS" } else { "SOME FAILED" });
    std::process::exit(if all_ok { 0 } else { 1 });
}
