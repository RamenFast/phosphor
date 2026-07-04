// SPDX-License-Identifier: GPL-3.0-or-later
//! Deterministic bench signals — the Rust twin of tests/bench/signals.py
//! for the generators that port exactly (closed-form math, no RNG):
//! sweep and chaos. noise (numpy PCG64 stream) and scene (the studio
//! compiler) canNOT be regenerated here bit-identically; the bench
//! prefers the SHA-verified wav files and only falls back to these two.
//!
//! Conversion law matches numpy `astype("<i2")`: clamp to [-1, 1],
//! scale by 32767, truncate toward zero.

use std::io::Write;
use std::path::Path;

pub const RATE: u32 = 48000;

fn sample_pair(name: &str, t: f64) -> (f64, f64) {
    match name {
        "sweep" => {
            let frequency = 220.0 + 400.0 * ((t % 8.0) / 8.0);
            (0.6 * (std::f64::consts::TAU * frequency * t).sin(),
             0.6 * (std::f64::consts::TAU * frequency * 1.5 * t + 0.7)
                 .sin())
        }
        "chaos" => {
            let mut out = [0.0f64; 2];
            for (channel, value) in out.iter_mut().enumerate() {
                let mut total = 0.0;
                let mut weight_sum = 0.0;
                for k in 0..4u32 {
                    let k1 = (k + 1) as f64;
                    let base = 110.0 * k1
                        * (1.0 + 0.011 * channel as f64 * k1);
                    let lfo = 0.13 * k1;
                    let depth = 55.0 * k1;
                    let phase = std::f64::consts::TAU
                        * (base * t
                           - depth / (std::f64::consts::TAU * lfo)
                           * (std::f64::consts::TAU * lfo * t
                              + channel as f64).cos());
                    let weight = 1.0 / k1;
                    total += weight * phase.sin();
                    weight_sum += weight;
                }
                *value = 0.9 * total / weight_sum;
            }
            (out[0], out[1])
        }
        _ => (0.0, 0.0),
    }
}

/// Write `seconds` of a regenerable signal as a 48 kHz s16 stereo wav.
pub fn write_wav(name: &str, seconds: u32, path: &Path)
                 -> std::io::Result<()> {
    let frame_count = (seconds * RATE) as usize;
    let data_bytes = frame_count * 4;
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    out.write_all(b"RIFF")?;
    out.write_all(&(36 + data_bytes as u32).to_le_bytes())?;
    out.write_all(b"WAVEfmt ")?;
    out.write_all(&16u32.to_le_bytes())?;
    out.write_all(&1u16.to_le_bytes())?;           // PCM
    out.write_all(&2u16.to_le_bytes())?;           // stereo
    out.write_all(&RATE.to_le_bytes())?;
    out.write_all(&(RATE * 4).to_le_bytes())?;     // byte rate
    out.write_all(&4u16.to_le_bytes())?;           // block align
    out.write_all(&16u16.to_le_bytes())?;          // bits
    out.write_all(b"data")?;
    out.write_all(&(data_bytes as u32).to_le_bytes())?;
    for index in 0..frame_count {
        let t = index as f64 / RATE as f64;
        let (left, right) = sample_pair(name, t);
        for value in [left, right] {
            let scaled = (value.clamp(-1.0, 1.0) * 32767.0) as i16;
            out.write_all(&scaled.to_le_bytes())?;
        }
    }
    Ok(())
}

pub fn regenerable(name: &str) -> bool {
    matches!(name, "sweep" | "chaos")
}
