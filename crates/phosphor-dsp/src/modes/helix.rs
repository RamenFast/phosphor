// SPDX-License-Identifier: GPL-3.0-or-later
//! helix: the XY figure extruded backwards in time — left/right stay
//! the screen plane, age recedes into the fog. Redrawn whole every
//! frame, so orbiting the camera swings the entire coil. The head burns
//! brighter, the tail remembers. At 48 kHz the waveform-history cap
//! (8192 frames) truncates the 0.35 s span — v3 truth, preserved.

use crate::{Computer, HELIX_MAX_POINTS, HELIX_SECONDS};

impl Computer {
    pub(crate) fn helix(&mut self, width: f32, height: f32) {
        let frame_count = self.waveform_history.len() / 2;
        let span = frame_count.min(
            (HELIX_SECONDS * self.sample_rate as f64) as usize);
        if span < 8 {
            return;
        }
        let step = 1usize.max(span / HELIX_MAX_POINTS);
        let total = span.div_ceil(step);
        let gain = self.gain as f64;
        let (width64, height64) = (width as f64, height as f64);
        let mut previous: Option<(f64, f64)> = None;
        for (order, frame) in (frame_count - span..frame_count)
            .step_by(step)
            .enumerate()
        {
            let age = 1.0 - order as f64 / 1usize.max(total - 1) as f64;
            let a = self.waveform_history[2 * frame] as f64;
            let b = self.waveform_history[2 * frame + 1] as f64;
            let c = age * 1.8 - 0.6; // newest floats near the eye
            let (x, y, fog) = self.camera.project(a, b, c, gain,
                                                  width64, height64);
            if let Some((px, py)) = previous {
                let brightness = (0.25 + 0.75 * (1.0 - age)) * fog;
                self.segments.push(px as f32, py as f32, x as f32, y as f32,
                                   brightness as f32);
            }
            previous = Some((x, y));
        }
    }
}
