// SPDX-License-Identifier: GPL-3.0-or-later
//! Polyphase windowed-sinc upsampler (stereo interleaved, streaming),
//! ported verbatim from core/src/lib.rs. The native-v3 os2/os4 fixtures
//! are the ONLY ground truth for this stage — v3's Python path never
//! oversampled — so tap generation and the inner product order are
//! byte-faithful to the code those fixtures were captured from.

use std::f32::consts::PI;

// Half-width in input frames (16 taps per output sample).
pub const SINC_HALF_WIDTH: usize = 8;
const SINC_CUTOFF: f32 = 0.9; // of the input Nyquist, keeps the kernel short

pub struct Upsampler {
    factor: usize,
    // taps[phase][k] weights x[m - half + 1 + k] for output time m + phase/N
    taps: Vec<Vec<f32>>,
    // last 2*half-1 input frames, interleaved, carried between calls
    tail: Vec<f32>,
}

impl Upsampler {
    pub fn new(factor: usize) -> Upsampler {
        let half = SINC_HALF_WIDTH;
        let mut taps = Vec::with_capacity(factor);
        for phase in 0..factor {
            let fraction = phase as f32 / factor as f32;
            let mut row = Vec::with_capacity(2 * half);
            for k in 0..2 * half {
                // tap k multiplies the input sample at offset (k - half + 1)
                // relative to the base frame; u is its distance from the
                // output instant, in input-sample units
                let u = (k as f32 - (half as f32 - 1.0)) - fraction;
                let sinc = if u == 0.0 {
                    1.0
                } else {
                    (PI * SINC_CUTOFF * u).sin() / (PI * SINC_CUTOFF * u)
                };
                // Blackman window over the kernel's span
                let normalized = u / half as f32;
                let window = if normalized.abs() >= 1.0 {
                    0.0
                } else {
                    0.42 + 0.5 * (PI * normalized).cos()
                        + 0.08 * (2.0 * PI * normalized).cos()
                };
                row.push(SINC_CUTOFF * sinc * window);
            }
            // exact unit DC gain per phase: no brightness/level drift
            let sum: f32 = row.iter().sum();
            for tap in row.iter_mut() {
                *tap /= sum;
            }
            taps.push(row);
        }
        let mut upsampler = Upsampler { factor, taps, tail: Vec::new() };
        upsampler.reset();
        upsampler
    }

    pub fn reset(&mut self) {
        self.tail = vec![0.0; (2 * SINC_HALF_WIDTH - 1) * 2];
    }

    /// Interleaved stereo in -> interleaved stereo out (factor× the frames).
    pub fn process(&mut self, input: &[f32], output: &mut Vec<f32>) {
        let half = SINC_HALF_WIDTH;
        let mut buffer = std::mem::take(&mut self.tail);
        buffer.extend_from_slice(input);
        let frames = buffer.len() / 2;
        output.clear();
        if frames >= 2 * half {
            output.reserve((frames - (2 * half - 1)) * self.factor * 2);
            for base in 0..frames - (2 * half - 1) {
                for phase in 0..self.factor {
                    let taps = &self.taps[phase];
                    let mut left = 0.0f32;
                    let mut right = 0.0f32;
                    for (k, tap) in taps.iter().enumerate() {
                        left += buffer[(base + k) * 2] * tap;
                        right += buffer[(base + k) * 2 + 1] * tap;
                    }
                    output.push(left);
                    output.push(right);
                }
            }
        }
        let keep_from = buffer.len().saturating_sub((2 * half - 1) * 2);
        buffer.drain(..keep_from);
        self.tail = buffer;
    }
}
