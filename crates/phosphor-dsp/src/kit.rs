// SPDX-License-Identifier: GPL-3.0-or-later
//! Signal kit chain (.phoskit): stereo transforms applied upstream of
//! every mode. Ported verbatim from core/src/lib.rs; the parity law
//! (gated byte-close by tests/golden/kits/): phase accumulators in f64
//! advanced per chunk by 2π·hz·frames/rate with euclidean wraparound;
//! trig computed in f64 and cast to f32 before the f32 sample math;
//! channel delays are exact integer sample counts, state zeroed on
//! reset/configure.

use std::collections::VecDeque;

const TAU64: f64 = std::f64::consts::TAU;
pub const MAX_KIT_STAGES: usize = 16;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum KitOp {
    Rotate = 0,
    Midside = 1,
    Ringmod = 2,
    Wobble = 3,
    Matrix = 4,
    Chandelay = 5,
}

impl KitOp {
    pub fn from_name(name: &str) -> Option<KitOp> {
        match name {
            "rotate" => Some(KitOp::Rotate),
            "midside" => Some(KitOp::Midside),
            "ringmod" => Some(KitOp::Ringmod),
            "wobble" => Some(KitOp::Wobble),
            "matrix" => Some(KitOp::Matrix),
            "chandelay" => Some(KitOp::Chandelay),
            _ => None,
        }
    }
}

pub struct KitStage {
    pub op: KitOp,
    pub params: [f64; 4],
    pub phase: f64,
    delay: VecDeque<f32>,
}

impl KitStage {
    pub fn new(op: KitOp, params: [f64; 4]) -> KitStage {
        KitStage { op, params, phase: 0.0, delay: VecDeque::new() }
    }

    /// Zero the run state; delay length derives from the current rate.
    pub fn reset(&mut self, sample_rate: f64) {
        self.phase = 0.0;
        self.delay.clear();
        if self.op == KitOp::Chandelay {
            let count = (self.params[0] / 1000.0 * sample_rate).round() as usize;
            self.delay = std::iter::repeat_n(0.0f32, count).collect();
        }
    }

    /// Transform interleaved stereo in place.
    pub fn process(&mut self, buffer: &mut [f32], sample_rate: f64) {
        let count = buffer.len() / 2;
        match self.op {
            KitOp::Rotate | KitOp::Wobble => {
                let delta = TAU64 * self.params[0] / sample_rate;
                for i in 0..count {
                    let phase = self.phase + delta * i as f64;
                    let angle = if self.op == KitOp::Rotate {
                        phase + self.params[1]
                    } else {
                        self.params[1] * phase.sin()
                    };
                    let cosine = angle.cos() as f32;
                    let sine = angle.sin() as f32;
                    let left = buffer[2 * i];
                    let right = buffer[2 * i + 1];
                    buffer[2 * i] = left * cosine - right * sine;
                    buffer[2 * i + 1] = left * sine + right * cosine;
                }
                self.phase = (self.phase + delta * count as f64).rem_euclid(TAU64);
            }
            KitOp::Midside => {
                let width = self.params[0] as f32;
                let half_plus = 0.5 * (1.0 + width);
                let half_minus = 0.5 * (1.0 - width);
                for frame in buffer.chunks_exact_mut(2) {
                    let (left, right) = (frame[0], frame[1]);
                    frame[0] = half_plus * left + half_minus * right;
                    frame[1] = half_minus * left + half_plus * right;
                }
            }
            KitOp::Ringmod => {
                let delta = TAU64 * self.params[0] / sample_rate;
                let depth = self.params[1];
                for i in 0..count {
                    let phase = self.phase + delta * i as f64;
                    let gain = (1.0 - depth * (0.5 + 0.5 * phase.sin())) as f32;
                    buffer[2 * i] *= gain;
                    buffer[2 * i + 1] *= gain;
                }
                self.phase = (self.phase + delta * count as f64).rem_euclid(TAU64);
            }
            KitOp::Matrix => {
                let a = self.params[0] as f32;
                let b = self.params[1] as f32;
                let c = self.params[2] as f32;
                let d = self.params[3] as f32;
                for frame in buffer.chunks_exact_mut(2) {
                    let (left, right) = (frame[0], frame[1]);
                    frame[0] = a * left + b * right;
                    frame[1] = c * left + d * right;
                }
            }
            KitOp::Chandelay => {
                if !self.delay.is_empty() {
                    let channel = usize::from(self.params[1] >= 0.5);
                    for i in 0..count {
                        self.delay.push_back(buffer[2 * i + channel]);
                        buffer[2 * i + channel] = self.delay.pop_front().unwrap();
                    }
                }
            }
        }
    }
}
