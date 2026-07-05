// SPDX-License-Identifier: GPL-3.0-or-later
//! Compose mode: draw a shape on the scope, hear it — the scope run in
//! reverse. A drawn path is resampled into a closed loop traversed at
//! constant speed (left channel = X, right channel = Y) and played at a
//! chosen loop frequency; any XY oscilloscope — including Phosphor
//! itself — draws the shape back.
//!
//! Constant SPEED matters, not constant parameter: beam brightness is
//! dwell time, so uniform velocity keeps the drawn shape evenly lit
//! instead of bunching light where the mouse moved slowly.
//!
//! Verbatim port of v3's phosphor_compose.py. Wave 4's studio panel
//! reuses this math (one engine rule — never a third path).

pub const MINIMUM_FREQUENCY_HZ: f64 = 20.0;
pub const MAXIMUM_FREQUENCY_HZ: f64 = 400.0;
/// Of a cycle; rounds off pen jitter, not corners.
const SMOOTHING_FRACTION: f64 = 1.0 / 150.0;

pub fn clamp_frequency(frequency_hz: f64) -> f64 {
    frequency_hz.clamp(MINIMUM_FREQUENCY_HZ, MAXIMUM_FREQUENCY_HZ)
}

/// The loop must end where it began or every cycle gets a retrace flash.
fn close_path(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut closed = points.to_vec();
    if closed.first() != closed.last() {
        closed.push(closed[0]);
    }
    closed
}

/// Resample a drawn path into `sample_count` (x, y) pairs that traverse
/// the closed path at constant speed: walk the cumulative arc length of
/// the polyline and place samples at uniform distance steps, linearly
/// interpolating within each edge.
pub fn resample_path_constant_speed(points: &[(f64, f64)],
                                    sample_count: usize)
    -> Result<Vec<(f64, f64)>, &'static str>
{
    if points.is_empty() {
        return Err("path has no length");
    }
    let closed = close_path(points);
    let edge_lengths: Vec<f64> = closed.windows(2)
        .map(|pair| (pair[1].0 - pair[0].0).hypot(pair[1].1 - pair[0].1))
        .collect();
    let total_length: f64 = edge_lengths.iter().sum();
    if total_length <= 0.0 {
        return Err("path has no length");
    }

    let mut resampled = Vec::with_capacity(sample_count);
    let mut edge_index = 0usize;
    let mut distance_into_edge = 0.0f64;
    let step = total_length / sample_count as f64;
    for _ in 0..sample_count {
        while edge_index < edge_lengths.len() - 1
            && distance_into_edge >= edge_lengths[edge_index]
        {
            distance_into_edge -= edge_lengths[edge_index];
            edge_index += 1;
        }
        let edge_length = edge_lengths[edge_index];
        let fraction = if edge_length > 0.0 {
            distance_into_edge / edge_length
        } else {
            0.0
        };
        let (start, end) = (closed[edge_index], closed[edge_index + 1]);
        resampled.push((start.0 + (end.0 - start.0) * fraction,
                        start.1 + (end.1 - start.1) * fraction));
        distance_into_edge += step;
    }
    Ok(resampled)
}

/// Circular moving average: takes the buzz out of hand-drawn jitter
/// while leaving deliberate corners essentially intact.
fn smooth_closed_loop(samples: Vec<(f64, f64)>, window: usize)
    -> Vec<(f64, f64)>
{
    if window < 3 {
        return samples;
    }
    let count = samples.len();
    let half = (window / 2) as isize;
    let kernel_size = (2 * half + 1) as f64;
    (0..count as isize).map(|index| {
        let (mut x_total, mut y_total) = (0.0, 0.0);
        for offset in -half..=half {
            let (x, y) =
                samples[(index + offset).rem_euclid(count as isize) as usize];
            x_total += x;
            y_total += y;
        }
        (x_total / kernel_size, y_total / kernel_size)
    }).collect()
}

/// One seamless cycle of the drawn shape as (x, y) pairs in -1..1.
pub fn loop_samples(points: &[(f64, f64)], frequency_hz: f64,
                    sample_rate: u32)
    -> Result<Vec<(f64, f64)>, &'static str>
{
    let samples_per_cycle =
        ((sample_rate as f64 / frequency_hz).round() as usize).max(16);
    let cycle = resample_path_constant_speed(points, samples_per_cycle)?;
    let smoothing_window =
        (samples_per_cycle as f64 * SMOOTHING_FRACTION) as usize;
    Ok(smooth_closed_loop(cycle, smoothing_window))
}

/// Tile `cycle_count` repeats of one cycle into interleaved f32 stereo
/// frames (X = left, Y = right), ready for the app's WAV writer.
pub fn tile_cycle(cycle: &[(f64, f64)], cycle_count: usize) -> Vec<f32> {
    let mut frames = Vec::with_capacity(cycle.len() * cycle_count * 2);
    for _ in 0..cycle_count {
        for &(x, y) in cycle {
            frames.push(x.clamp(-1.0, 1.0) as f32);
            frames.push(y.clamp(-1.0, 1.0) as f32);
        }
    }
    frames
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_square() -> Vec<(f64, f64)> {
        vec![(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
    }

    #[test]
    fn constant_speed_on_a_square() {
        // Perimeter 4, 16 samples → step 0.25; the corners land exactly
        // on sample boundaries, so every consecutive Euclidean distance
        // (wraparound included) is exactly the step.
        let resampled =
            resample_path_constant_speed(&unit_square(), 16).unwrap();
        assert_eq!(resampled.len(), 16);
        for index in 0..16 {
            let a = resampled[index];
            let b = resampled[(index + 1) % 16];
            let distance = (b.0 - a.0).hypot(b.1 - a.1);
            assert!((distance - 0.25).abs() < 1e-12,
                    "segment {index} has length {distance}");
        }
    }

    #[test]
    fn loop_starts_at_the_pen_down_point() {
        let resampled =
            resample_path_constant_speed(&unit_square(), 64).unwrap();
        assert_eq!(resampled[0], (0.0, 0.0));
    }

    #[test]
    fn degenerate_paths_are_refused() {
        assert!(resample_path_constant_speed(&[], 16).is_err());
        assert!(resample_path_constant_speed(&[(0.5, 0.5)], 16).is_err());
        assert!(resample_path_constant_speed(
            &[(0.5, 0.5), (0.5, 0.5)], 16).is_err());
    }

    #[test]
    fn frequency_clamps_to_the_audible_drawing_band() {
        assert_eq!(clamp_frequency(5.0), MINIMUM_FREQUENCY_HZ);
        assert_eq!(clamp_frequency(80.0), 80.0);
        assert_eq!(clamp_frequency(5000.0), MAXIMUM_FREQUENCY_HZ);
    }

    #[test]
    fn samples_per_cycle_has_a_floor() {
        // 400 Hz at a hypothetical 4 kHz rate would be 10 samples; the
        // floor keeps a drawable minimum of 16.
        let cycle = loop_samples(&unit_square(), 400.0, 4_000).unwrap();
        assert_eq!(cycle.len(), 16);
    }

    #[test]
    fn smoothing_below_window_three_is_identity() {
        let cycle = resample_path_constant_speed(&unit_square(), 32)
            .unwrap();
        assert_eq!(smooth_closed_loop(cycle.clone(), 2), cycle);
    }

    #[test]
    fn smoothing_preserves_the_centroid() {
        // A circular moving average redistributes but never invents:
        // the loop's mean point is invariant.
        let cycle = resample_path_constant_speed(&unit_square(), 150)
            .unwrap();
        let centroid = |loop_points: &[(f64, f64)]| {
            let n = loop_points.len() as f64;
            (loop_points.iter().map(|p| p.0).sum::<f64>() / n,
             loop_points.iter().map(|p| p.1).sum::<f64>() / n)
        };
        let before = centroid(&cycle);
        let after = centroid(&smooth_closed_loop(cycle, 7));
        assert!((before.0 - after.0).abs() < 1e-12);
        assert!((before.1 - after.1).abs() < 1e-12);
    }

    #[test]
    fn tile_cycle_interleaves_and_clamps() {
        let frames = tile_cycle(&[(0.5, -2.0), (1.5, 0.25)], 2);
        assert_eq!(frames,
                   vec![0.5, -1.0, 1.0, 0.25, 0.5, -1.0, 1.0, 0.25]);
    }
}
