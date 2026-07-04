// SPDX-License-Identifier: GPL-3.0-or-later
//! Gate A driver — small CLI over the vacuum engine so the gate script
//! can exercise route / kill -9 / sweep / release from real processes.
//!
//!   vacuum_ctl sweep
//!       sweep stale vacuum artifacts, print "swept N", exit.
//!   vacuum_ctl route <app-substring> <seconds|forever>
//!       route the first playing app whose label contains the
//!       substring, print "ROUTED <monitor-combo-id>", hold, then
//!       release cleanly and print "RELEASED" (unless killed first —
//!       that is the point of the kill -9 test).

use std::sync::mpsc;
use std::time::Duration;

use phosphor_audio::{AudioEngine, TargetKind};

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let (event_sender, _events) = mpsc::channel();
    let engine = AudioEngine::spawn(48_000, event_sender).expect("engine");

    match arguments.first().map(String::as_str) {
        Some("sweep") => {
            let removed = engine.sweep_stale_vacuum();
            println!("swept {removed}");
        }
        Some("route") => {
            let needle = arguments.get(1).expect("route <app-substring> <hold>");
            let hold = arguments.get(2).map(String::as_str).unwrap_or("2");
            let target = engine
                .targets()
                .into_iter()
                .find(|t| t.kind == TargetKind::App && t.label.contains(needle))
                .unwrap_or_else(|| panic!("no playing app matches {needle:?}"));
            match engine.vacuum_route_app(&target.stable_key) {
                Ok(monitor_combo) => {
                    println!("ROUTED {monitor_combo}");
                    // the gate script greps this line, then may kill -9 us
                    if hold == "forever" {
                        loop {
                            std::thread::sleep(Duration::from_secs(3600));
                        }
                    }
                    let seconds: f64 = hold.parse().unwrap_or(2.0);
                    std::thread::sleep(Duration::from_secs_f64(seconds));
                    engine.vacuum_release();
                    println!("RELEASED");
                }
                Err(error) => {
                    eprintln!("route failed: {error}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("usage: vacuum_ctl sweep | route <app-substring> <seconds|forever>");
            std::process::exit(2);
        }
    }
}
