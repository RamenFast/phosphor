// SPDX-License-Identifier: GPL-3.0-or-later
//! The spectrum family. Bar levels + the flat and radial bar displays
//! are verbatim from the v3 core; tunnel is the python-lineage member —
//! spectrum bands as concentric rings, bass innermost, each ring
//! brightening and swelling with its band. The tunnel breathes.

use std::f32::consts::PI;

use crate::{Computer, SPECTRUM_BAR_COUNT, TUNNEL_RINGS, TUNNEL_RING_POINTS};

impl Computer {
    /// Run the FFT every other frame and smooth bar levels in place —
    /// fast attack, slow phosphor fall. Cold-start calls legitimately
    /// draw nothing (FFT not warm, levels under the threshold).
    pub(crate) fn update_spectrum_levels(&mut self) {
        let frame_count = self.waveform_history.len() / 2;
        self.frames_since_fft += 1;
        if frame_count < self.fft.size || self.frames_since_fft < 2 {
            return;
        }
        self.frames_since_fft = 0;
        let tail_start = 2 * (frame_count - self.fft.size);
        let mono: Vec<f32> = self.waveform_history[tail_start..]
            .chunks_exact(2)
            .map(|frame| (frame[0] + frame[1]) * 0.5)
            .collect();
        let mut magnitudes = std::mem::take(&mut self.magnitude_buffer);
        self.fft.magnitudes(&mono, &mut magnitudes);
        let normalization = self.fft.size as f32 / 8.0;
        for (bar, &(low_bin, high_bin)) in self.bar_bin_ranges.iter().enumerate() {
            let peak = magnitudes[low_bin..high_bin]
                .iter()
                .fold(0.0f32, |acc, &value| acc.max(value));
            let level = ((peak / normalization).sqrt() * self.gain).min(1.0);
            if level > self.spectrum_levels[bar] {
                self.spectrum_levels[bar] = level; // fast attack
            } else {
                self.spectrum_levels[bar] *= 0.93; // slow phosphor fall
            }
        }
        self.magnitude_buffer = magnitudes;
    }

    pub(crate) fn spectrum(&mut self, width: f32, height: f32) {
        let baseline = height * 0.88;
        let bar_pitch = width / SPECTRUM_BAR_COUNT as f32;
        for (bar, &level) in self.spectrum_levels.iter().enumerate() {
            if level < 0.01 {
                continue;
            }
            let x = bar_pitch * (bar as f32 + 0.5);
            let top = baseline - level * height * 0.74;
            self.segments.push(x, baseline, x, top, 0.35 + 0.65 * level);
        }
    }

    /// Bars radiating from a circle: bass at twelve o'clock, clockwise.
    pub(crate) fn spectrum_radial(&mut self, width: f32, height: f32) {
        let center_x = width / 2.0;
        let center_y = height / 2.0;
        let inner_radius = width.min(height) * 0.14;
        let bar_reach = width.min(height) * 0.32;
        for (bar, &level) in self.spectrum_levels.iter().enumerate() {
            if level < 0.01 {
                continue;
            }
            let angle =
                2.0 * PI * (bar as f32 + 0.5) / SPECTRUM_BAR_COUNT as f32 - PI / 2.0;
            let (sine, cosine) = angle.sin_cos();
            let outer_radius = inner_radius + level * bar_reach;
            self.segments.push(center_x + cosine * inner_radius,
                               center_y + sine * inner_radius,
                               center_x + cosine * outer_radius,
                               center_y + sine * outer_radius,
                               0.35 + 0.65 * level);
        }
    }

    pub(crate) fn tunnel(&mut self, width: f32, height: f32) {
        self.update_spectrum_levels();
        let center_x = width as f64 / 2.0;
        let center_y = height as f64 / 2.0;
        let base = width.min(height) as f64;
        let bands = SPECTRUM_BAR_COUNT / TUNNEL_RINGS;
        for ring in 0..TUNNEL_RINGS {
            let level = self.spectrum_levels[ring * bands..(ring + 1) * bands]
                .iter()
                .fold(0.0f32, |acc, &value| acc.max(value)) as f64;
            if level < 0.02 {
                continue;
            }
            let depth = (ring as f64 / (TUNNEL_RINGS - 1) as f64).powf(1.35);
            let radius = base * (0.07 + 0.36 * depth) + level * base * 0.03;
            let intensity = 0.15 + 0.85 * level;
            let mut previous: Option<(f64, f64)> = None;
            for point in 0..=TUNNEL_RING_POINTS {
                let angle =
                    std::f64::consts::TAU * point as f64 / TUNNEL_RING_POINTS as f64;
                let x = center_x + angle.cos() * radius;
                let y = center_y + angle.sin() * radius;
                if let Some((px, py)) = previous {
                    self.segments.push(px as f32, py as f32, x as f32, y as f32,
                                       intensity as f32);
                }
                previous = Some((x, y));
            }
        }
    }
}
