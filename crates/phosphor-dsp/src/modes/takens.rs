// SPDX-License-Identifier: GPL-3.0-or-later
//! xyz_takens: delay-embed the mono signal — each new sample becomes
//! the point (x(t), x(t−τ), x(t−2τ)); the signal plotted against its
//! own past reconstructs the attractor (Takens' theorem, on a scope).
//!
//! τ follows the dominant pitch: a quarter of its period, found by
//! autocorrelation over the recent window every 4th probe (never per
//! frame — the probe is the expensive part) and smoothed so the
//! attractor morphs instead of popping. Silence keeps the last τ.
//! Pre-lock default: τ = 0.004 × rate, with the emit = history − 2τ
//! warmup ramp — the fixtures' first calls pin both.
//!
//! The autocorrelation runs in f64 (numpy's ran in f32); only the
//! integer argmax survives into the output, and every fixture case
//! verifies those integer decisions end to end.

use rustfft::num_complex::Complex;

use crate::{age_weight64, Computer, TAKENS_HIGH_HZ, TAKENS_LOW_HZ,
            TAKENS_TAU_SMOOTHING};

impl Computer {
    pub(crate) fn takens(&mut self, samples: &[f32], width: f32, height: f32) {
        let count = samples.len() / 2;
        if count == 0 {
            return;
        }
        for frame in samples.chunks_exact(2) {
            self.mono_history.push((frame[0] + frame[1]) * 0.5);
        }
        let excess = self.mono_history.len() as isize
            - self.mono_history_limit as isize;
        if excess > 0 {
            // trimming shifts indexes; the embed below only looks back
            // 2τ from the tail, which the limit always preserves
            self.mono_history.drain(..excess as usize);
        }
        self.update_tau();
        let tau = self.takens_tau.unwrap_or_else(|| {
            2usize.max((0.004 * self.sample_rate as f64) as usize)
        });
        let history_length = self.mono_history.len();
        let emit = count
            .min(self.max_points_feed)
            .min(history_length.saturating_sub(2 * tau));
        if emit < 1 {
            return;
        }

        let gain = self.gain as f64;
        let energy = self.beam_energy as f64;
        let distance_scale = self.distance_scale_feed;
        let (width64, height64) = (width as f64, height as f64);
        let segment_count = emit - usize::from(self.takens_last.is_none());
        let mut previous = self.takens_last.map(|(a, b, c)| {
            self.camera.project(a, b, c, gain, width64, height64)
        });
        let mut segment_index = 0usize;
        let mut last_point = self.takens_last.unwrap_or((0.0, 0.0, 0.0));
        for offset in 0..emit {
            let index = history_length - emit + offset;
            let point = (self.mono_history[index] as f64,
                         self.mono_history[index - tau] as f64,
                         self.mono_history[index - 2 * tau] as f64);
            let (x, y, fog) = self.camera.project(point.0, point.1, point.2,
                                                  gain, width64, height64);
            if let Some((px, py, _)) = previous {
                let distance = (x - px).hypot(y - py) * distance_scale;
                let mut intensity = (energy / (distance + 0.7)).min(1.0) * fog;
                if let Some(weight) = age_weight64(self.frame_glow_keep as f64,
                                                   segment_count, segment_index)
                {
                    intensity *= weight;
                }
                self.segments.push(px as f32, py as f32, x as f32, y as f32,
                                   intensity as f32);
                segment_index += 1;
            }
            previous = Some((x, y, fog));
            last_point = point;
        }
        self.takens_last = Some(last_point);
    }

    /// Follow the dominant pitch. The probe counter only resets when a
    /// probe actually runs (window big enough) — v3 semantics, pinned
    /// by the fixtures' warmup calls.
    fn update_tau(&mut self) {
        self.probes_since_tau += 1;
        if self.probes_since_tau < 4 {
            return;
        }
        let window = self.mono_history.len().min(self.autocorr_window);
        if window < 1024 {
            return;
        }
        self.probes_since_tau = 0;
        let tail = &self.mono_history[self.mono_history.len() - window..];
        let mean = tail.iter().map(|&value| value as f64).sum::<f64>()
            / window as f64;
        let mut buffer: Vec<Complex<f64>> = tail
            .iter()
            .map(|&value| Complex::new(value as f64 - mean, 0.0))
            .collect();
        let peak_input = buffer
            .iter()
            .fold(0.0f64, |acc, value| acc.max(value.re.abs()));
        if peak_input < 1e-3 {
            return; // silence: hold the shape
        }
        // circular autocorrelation via the power spectrum, numpy's
        // irfft(rfft(x)·conj) — full complex round trip, then 1/n
        let forward = self.tau_planner.plan_fft_forward(window);
        forward.process(&mut buffer);
        for bin in buffer.iter_mut() {
            *bin = Complex::new(bin.norm_sqr(), 0.0);
        }
        let inverse = self.tau_planner.plan_fft_inverse(window);
        inverse.process(&mut buffer);
        let autocorr: Vec<f64> =
            buffer.iter().map(|bin| bin.re / window as f64).collect();

        let low_lag = 2usize.max(
            (self.sample_rate as f64 / TAKENS_HIGH_HZ) as usize);
        let high_lag = (window / 2).min(
            (self.sample_rate as f64 / TAKENS_LOW_HZ) as usize);
        if high_lag <= low_lag {
            return;
        }
        let mut lag = low_lag;
        let mut best = f64::NEG_INFINITY;
        for (offset, &value) in autocorr[low_lag..high_lag].iter().enumerate() {
            if value > best {
                best = value;
                lag = low_lag + offset;
            }
        }
        if autocorr[lag] < 0.15 * autocorr[0] {
            return; // aperiodic: hold the shape
        }
        let probed = 2usize.max(lag / 4);
        self.takens_tau = Some(match self.takens_tau {
            None => probed,
            Some(current) => 2usize.max(
                (TAKENS_TAU_SMOOTHING * current as f64
                    + (1.0 - TAKENS_TAU_SMOOTHING) * probed as f64)
                    .round() as usize),
        });
    }
}
