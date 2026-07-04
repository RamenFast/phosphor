// SPDX-License-Identifier: GPL-3.0-or-later
//! Band rasterizer: segments deposit Gaussian beam energy into the two
//! planes, 8 pixels at a time. The math is phosphor_beam::deposit exactly
//! — erf along the axis, Gaussian across — vectorized with `wide` and
//! dispatched once per frame to an AVX2+FMA clone when the CPU has them
//! (Zen 2 does; there is no AVX-512 path on purpose).

use wide::{f32x8, CmpGt, CmpLt};

use phosphor_beam::{BEAM_RADIUS_SIGMAS, GLOW_COUPLING};

/// One prepared segment in energy-buffer pixels.
#[derive(Clone, Copy)]
pub(crate) struct PreparedSegment {
    pub p0: [f32; 2],
    pub tangent: [f32; 2],
    pub length: f32,
    pub intensity: f32,
    /// Buffer-row span this segment can touch (quad extent, clamped).
    pub row_start: usize,
    pub row_end: usize,
    pub column_start: usize,
    pub column_end: usize,
}

pub(crate) fn prepare(segment: &[f32; 5], pixel_scale: f32, radius: f32,
                      buffer_width: usize, buffer_height: usize)
                      -> Option<PreparedSegment> {
    let p0 = [segment[0] * pixel_scale, segment[1] * pixel_scale];
    let p1 = [segment[2] * pixel_scale, segment[3] * pixel_scale];
    let direction = [p1[0] - p0[0], p1[1] - p0[1]];
    let length = (direction[0] * direction[0]
                  + direction[1] * direction[1]).sqrt();
    let tangent = if length > 1e-4 {
        [direction[0] / length, direction[1] / length]
    } else {
        [1.0, 0.0]
    };
    let min_x = p0[0].min(p1[0]) - radius;
    let max_x = p0[0].max(p1[0]) + radius;
    let min_y = p0[1].min(p1[1]) - radius;
    let max_y = p0[1].max(p1[1]) + radius;
    if max_x < 0.0 || max_y < 0.0
        || min_x >= buffer_width as f32 || min_y >= buffer_height as f32 {
        return None;
    }
    let row_start = min_y.max(0.0) as usize;
    let row_end = (max_y.ceil() as usize + 1).min(buffer_height);
    let column_start = min_x.max(0.0) as usize;
    let column_end = (max_x.ceil() as usize + 1).min(buffer_width);
    if row_start >= row_end || column_start >= column_end {
        return None;
    }
    Some(PreparedSegment {
        p0,
        tangent,
        length,
        intensity: segment[4],
        row_start,
        row_end,
        column_start,
        column_end,
    })
}

/// erf, Abramowitz & Stegun 7.1.27 — the exact polynomial from
/// phosphor_beam::erf_approximation, 8 lanes wide.
#[inline(always)]
fn erf_x8(x: f32x8) -> f32x8 {
    let a = x.abs();
    let mut d = f32x8::ONE
        + (f32x8::splat(0.278393)
           + (f32x8::splat(0.230389) + f32x8::splat(0.078108) * a * a) * a)
            * a;
    d *= d;
    // true divide, not recip(): the ~12-bit RCPPS estimate would spend
    // the whole 5e-4 error budget of the approximation by itself
    let magnitude = f32x8::ONE - f32x8::ONE / (d * d);
    let negative = x.cmp_lt(f32x8::ZERO);
    negative.blend(-magnitude, magnitude)
}

/// Deposit one segment across the rows a band owns. `flash`/`glow` are
/// the BAND's slices (rows [band_row, band_row + rows)); rayon hands each
/// band exclusive access, so no atomics anywhere in the hot path.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
fn deposit_rows_impl(segment: &PreparedSegment, sigma: f32,
                     normalization: f32, band_row: usize, row_start: usize,
                     row_end: usize, buffer_width: usize,
                     flash: &mut [f32], glow: &mut [f32]) {
    let inverse_sigma_sqrt2 =
        f32x8::splat(std::f32::consts::FRAC_1_SQRT_2 / sigma);
    let inverse_two_sigma_squared =
        f32x8::splat(-1.0 / (2.0 * sigma * sigma));
    let scale =
        f32x8::splat(segment.intensity * normalization);
    let length = f32x8::splat(segment.length);
    let tangent_x = f32x8::splat(segment.tangent[0]);
    let tangent_y = f32x8::splat(segment.tangent[1]);
    let p0_x = f32x8::splat(segment.p0[0]);
    let p0_y = f32x8::splat(segment.p0[1]);
    let coupling = f32x8::splat(GLOW_COUPLING);
    let lane_offsets =
        f32x8::from([0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0]);

    let radius = sigma * BEAM_RADIUS_SIGMAS;
    let row_start = row_start.max(segment.row_start);
    let row_end = row_end.min(segment.row_end);
    for row in row_start..row_end {
        let center_y = row as f32 + 0.5;
        // exact x-span of the beam tube on this row: intersect the two
        // linear constraints (|perp| ≤ R, along ∈ [−R, len+R]). Without
        // this a screen-diagonal noise segment evaluates its whole
        // bounding RECTANGLE (~40× the tube) — the difference between
        // 286 ms and single-digit ms per frame.
        let delta_y = center_y - segment.p0[1];
        let (mut span_low, mut span_high) =
            (segment.column_start as f32, segment.column_end as f32);
        let tangent = segment.tangent;
        if tangent[0].abs() > 1e-6 {
            let a = segment.p0[0] + (-radius - delta_y * tangent[1])
                / tangent[0];
            let b = segment.p0[0]
                + (segment.length + radius - delta_y * tangent[1])
                / tangent[0];
            span_low = span_low.max(a.min(b));
            span_high = span_high.min(a.max(b));
        } else if !(-radius..=segment.length + radius)
            .contains(&(delta_y * tangent[1])) {
            continue;
        }
        if tangent[1].abs() > 1e-6 {
            let a = segment.p0[0]
                + (delta_y * tangent[0] - radius) / tangent[1];
            let b = segment.p0[0]
                + (delta_y * tangent[0] + radius) / tangent[1];
            span_low = span_low.max(a.min(b));
            span_high = span_high.min(a.max(b));
        } else if (delta_y * tangent[0]).abs() > radius {
            continue;
        }
        // half-pixel slack against rounding at the tube edge
        let first_column = ((span_low - 1.0).floor().max(
            segment.column_start as f32)) as usize;
        let last_column = ((span_high + 1.0).ceil().min(
            segment.column_end as f32)) as usize;
        if first_column >= last_column {
            continue;
        }

        let pixel_y = f32x8::splat(center_y) - p0_y;
        let row_base = (row - band_row) * buffer_width;
        let mut column = first_column;
        while column < last_column {
            let pixel_x = f32x8::splat(column as f32 + 0.5) + lane_offsets
                - p0_x;
            let along = pixel_x * tangent_x + pixel_y * tangent_y;
            let perpendicular = pixel_y * tangent_x - pixel_x * tangent_y;
            let along_integral = f32x8::splat(0.5)
                * (erf_x8(along * inverse_sigma_sqrt2)
                   - erf_x8((along - length) * inverse_sigma_sqrt2));
            let cross_section = (perpendicular * perpendicular
                                 * inverse_two_sigma_squared).exp();
            let energy = scale * cross_section * along_integral;
            // skip stores when the whole vector is effectively zero —
            // most of a long segment's AABB is empty corner
            if !energy.cmp_gt(f32x8::splat(1e-7)).any() {
                column += 8;
                continue;
            }
            let base = row_base + column;
            let lanes = 8.min(last_column - column);
            if lanes == 8 {
                // full-width read-modify-write — the scalar 8-lane loop
                // here was a measurable chunk of the frame at 32k segs
                let current: [f32; 8] =
                    flash[base..base + 8].try_into().unwrap();
                let updated = f32x8::from(current) + energy;
                flash[base..base + 8]
                    .copy_from_slice(&updated.to_array());
                let current: [f32; 8] =
                    glow[base..base + 8].try_into().unwrap();
                let updated = f32x8::from(current) + energy * coupling;
                glow[base..base + 8]
                    .copy_from_slice(&updated.to_array());
            } else {
                let deposits: [f32; 8] = energy.into();
                let glow_deposits: [f32; 8] = (energy * coupling).into();
                for lane in 0..lanes {
                    flash[base + lane] += deposits[lane];
                    glow[base + lane] += glow_deposits[lane];
                }
            }
            column += 8;
        }
    }
}

// Same body, compiled with AVX2+FMA enabled; picked at runtime.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2,fma")]
#[allow(clippy::too_many_arguments)]
unsafe fn deposit_rows_avx2(segment: &PreparedSegment, sigma: f32,
                            normalization: f32, band_row: usize,
                            row_start: usize, row_end: usize,
                            buffer_width: usize, flash: &mut [f32],
                            glow: &mut [f32]) {
    deposit_rows_impl(segment, sigma, normalization, band_row, row_start,
                      row_end, buffer_width, flash, glow);
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Dispatch {
    Avx2,
    Portable,
}

pub(crate) fn detect() -> Dispatch {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") && is_x86_feature_detected!("fma")
        {
            return Dispatch::Avx2;
        }
    }
    Dispatch::Portable
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn deposit_rows(dispatch: Dispatch, segment: &PreparedSegment,
                           sigma: f32, normalization: f32, band_row: usize,
                           row_start: usize, row_end: usize,
                           buffer_width: usize, flash: &mut [f32],
                           glow: &mut [f32]) {
    match dispatch {
        #[cfg(target_arch = "x86_64")]
        Dispatch::Avx2 => unsafe {
            deposit_rows_avx2(segment, sigma, normalization, band_row,
                              row_start, row_end, buffer_width, flash,
                              glow);
        },
        _ => deposit_rows_impl(segment, sigma, normalization, band_row,
                               row_start, row_end, buffer_width, flash,
                               glow),
    }
}

pub(crate) fn beam_radius(sigma: f32) -> f32 {
    sigma * BEAM_RADIUS_SIGMAS
}
