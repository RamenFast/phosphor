// SPDX-License-Identifier: GPL-3.0-or-later
//! The XY family. xy/xy45/xy_dots are VERBATIM from core/src/lib.rs
//! (f32 op order pinned by the native-v3 fixtures). xy_swirl is the
//! python-lineage member: rotate the whole chunk by the swirl phase
//! (f64 trig cast to f32 before the f32 sample math, like the kit law),
//! then trace it through the same segment path at the FEED distance
//! scale — swirl never oversamples in v3 and the fixtures pin that.

use crate::{age_weight, Computer, Mode, SQRT_HALF, SWIRL_RADIANS_PER_SECOND};

impl Computer {
    pub(crate) fn xy_modes(&mut self, samples: &[f32], width: f32, height: f32) {
        let dots = self.mode == Mode::XyDots;
        let rotate = self.mode == Mode::Xy45;
        self.xy_trace(samples, width, height, rotate, dots,
                      self.distance_scale_effective, self.max_points_effective);
    }

    pub(crate) fn swirl(&mut self, samples: &[f32], width: f32, height: f32) {
        // trig from the CURRENT phase; the phase then advances by this
        // chunk's duration — sample count, never wall clock
        let cosine = self.swirl_phase.cos() as f32;
        let sine = self.swirl_phase.sin() as f32;
        let frames = samples.len() / 2;
        self.swirl_phase = (self.swirl_phase
            + frames as f64 / self.sample_rate as f64 * SWIRL_RADIANS_PER_SECOND)
            .rem_euclid(std::f64::consts::TAU);
        let mut rotated = std::mem::take(&mut self.swirl_buffer);
        rotated.clear();
        rotated.reserve(samples.len());
        for frame in samples.chunks_exact(2) {
            let (left, right) = (frame[0], frame[1]);
            rotated.push(left * cosine - right * sine);
            rotated.push(left * sine + right * cosine);
        }
        let scale = self.distance_scale_feed as f32;
        let max_points = self.max_points_feed;
        self.xy_trace(&rotated, width, height, false, false, scale, max_points);
        self.swirl_buffer = rotated;
    }

    /// The shared trace: verbatim core geometry. `distance_scale` and
    /// `max_points` differ between the oversampling family (effective)
    /// and swirl (feed) — v3 truth, both pinned by fixtures.
    #[allow(clippy::too_many_arguments)]
    fn xy_trace(&mut self, samples: &[f32], width: f32, height: f32,
                rotate: bool, dots: bool, distance_scale: f32,
                max_points: usize) {
        let mut samples = samples;
        if samples.len() > 2 * max_points {
            samples = &samples[samples.len() - 2 * max_points..];
            if !dots {
                self.last_beam = None; // gap in the trace, don't bridge it
            }
        }
        if samples.len() < 2 {
            return;
        }
        let center_x = width / 2.0;
        let center_y = height / 2.0;
        let radius = width.min(height) * 0.45;
        let deflection = self.gain * radius;
        let point_of = |left: f32, right: f32| -> (f32, f32) {
            let (horizontal, vertical) = if rotate {
                ((left - right) * SQRT_HALF, (left + right) * SQRT_HALF)
            } else {
                (left, right)
            };
            (center_x + horizontal * deflection, center_y - vertical * deflection)
        };

        if dots {
            // discrete-dot display; a finer feed stamps proportionally
            // more dots along the same path, so each is scaled down to
            // keep the overall brightness unchanged
            let dot_intensity = 1.0 / distance_scale;
            let count = samples.len() / 2;
            for index in 0..count {
                let (x, y) = point_of(samples[2 * index], samples[2 * index + 1]);
                let weight = age_weight(self.frame_glow_keep, count, index)
                    .unwrap_or(1.0)
                    * dot_intensity;
                self.segments.push(x - 0.8, y, x + 0.8, y, weight);
            }
            return;
        }

        let count = samples.len() / 2 + usize::from(self.last_beam.is_some());
        let segment_count = count - 1;
        let (mut previous_x, mut previous_y) = match self.last_beam {
            Some(beam) => beam,
            None => point_of(samples[0], samples[1]),
        };
        let skip_first = usize::from(self.last_beam.is_none());
        for index in skip_first..samples.len() / 2 {
            let (x, y) = point_of(samples[2 * index], samples[2 * index + 1]);
            let distance = (x - previous_x).hypot(y - previous_y) * distance_scale;
            let mut intensity = (self.beam_energy / (distance + 0.7)).min(1.0);
            let segment_index = index - skip_first;
            if let Some(weight) =
                age_weight(self.frame_glow_keep, segment_count, segment_index)
            {
                intensity *= weight;
            }
            self.segments.push(previous_x, previous_y, x, y, intensity);
            previous_x = x;
            previous_y = y;
        }
        self.last_beam = Some((previous_x, previous_y));
    }
}
