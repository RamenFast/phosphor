// SPDX-License-Identifier: GPL-3.0-or-later
//! Debug soak: loop a file audibly and print the position clock once a
//! second so the consumption rate is visible (pair with `pw-top`).

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn main() {
    let path = PathBuf::from(std::env::args().nth(1).expect("file"));
    let seconds: u64 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(6);
    let (tx, _rx) = mpsc::channel();
    let engine = phosphor_audio::AudioEngine::spawn(48_000, tx).expect("engine");
    engine.set_volume(0.15);
    engine.start_file(&path, 0.0, true, false);
    let start = Instant::now();
    for _ in 0..seconds {
        std::thread::sleep(Duration::from_secs(1));
        let drained = engine.take_stereo_samples().len();
        println!(
            "wall {:>5.2}s position {:>5.2}s scope+{} samples",
            start.elapsed().as_secs_f64(),
            engine.playback_position_seconds(),
            drained,
        );
    }
    engine.stop_playback();
}
