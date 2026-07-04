// SPDX-License-Identifier: GPL-3.0-or-later
//! waveform (two triggered oscillogram traces) — verbatim from the v3
//! core — and ring (the python-lineage member: the oscillogram bent
//! around a circle, time sweeping the angle, amplitude moving the
//! radius; left ring inner, right ring outer, same trigger).
//!
//! Trigger bound note: the Python reference's search range excludes its
//! lower bound (`range(start, low, -1)`); the v3 core included one
//! extra frame. The Python semantics are the v4 reference and the
//! difference is observable only when the sole zero-crossing sits
//! exactly on that boundary frame.

use crate::{Computer, RING_TRACE_POINTS};

impl Computer {
    /// Frame index of the latest rising zero-crossing of the left
    /// channel that leaves a full window to display.
    pub(crate) fn trigger_offset(&self) -> Option<usize> {
        let history = &self.waveform_history;
        let frame_count = history.len() / 2;
        if frame_count < self.waveform_window + 1 {
            return None;
        }
        let search_start = frame_count - self.waveform_window;
        let lower = search_start.saturating_sub(self.waveform_trigger_search);
        ((lower + 1)..=search_start)
            .rev()
            .find(|&frame| history[2 * (frame - 1)] < 0.0
                && history[2 * frame] >= 0.0)
    }

    pub(crate) fn waveform(&mut self, width: f32, height: f32) {
        let frame_count = self.waveform_history.len() / 2;
        if frame_count < 4 {
            return;
        }
        let window = self.waveform_window.min(frame_count);
        let start_frame = self.trigger_offset().unwrap_or(frame_count - window);
        let amplitude = height * 0.21 * self.gain;
        let step = (window / (width as usize).max(64)).max(1);
        for (channel, baseline) in [(0usize, height * 0.28), (1usize, height * 0.72)] {
            let mut previous: Option<(f32, f32)> = None;
            for offset in (0..window).step_by(step) {
                let frame = start_frame + offset;
                if frame >= frame_count {
                    break;
                }
                let x = width * offset as f32 / window as f32;
                let y = baseline
                    - self.waveform_history[2 * frame + channel] * amplitude;
                if let Some((px, py)) = previous {
                    self.segments.push(px, py, x, y, 0.85);
                }
                previous = Some((x, y));
            }
        }
    }

    pub(crate) fn ring(&mut self, width: f32, height: f32) {
        let frame_count = self.waveform_history.len() / 2;
        if frame_count < 8 {
            return;
        }
        let window = self.waveform_window.min(frame_count);
        let start_frame = self.trigger_offset().unwrap_or(frame_count - window);
        let center_x = width as f64 / 2.0;
        let center_y = height as f64 / 2.0;
        let base = width.min(height) as f64;
        let step = (window / RING_TRACE_POINTS).max(1);
        for (channel, ring_radius) in [(0usize, base * 0.24), (1usize, base * 0.36)] {
            let amplitude = base * 0.09 * self.gain as f64;
            let mut previous: Option<(f64, f64)> = None;
            for offset in (0..window).step_by(step) {
                let frame = start_frame + offset;
                if frame >= frame_count {
                    break;
                }
                let angle = std::f64::consts::TAU * offset as f64 / window as f64
                    - std::f64::consts::FRAC_PI_2;
                let radius = ring_radius
                    + self.waveform_history[2 * frame + channel] as f64 * amplitude;
                let x = center_x + angle.cos() * radius;
                let y = center_y + angle.sin() * radius;
                if let Some((px, py)) = previous {
                    self.segments.push(px as f32, py as f32, x as f32, y as f32,
                                       0.8);
                }
                previous = Some((x, y));
            }
        }
    }
}
