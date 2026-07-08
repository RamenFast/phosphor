// SPDX-License-Identifier: GPL-3.0-or-later
//! CPU rasterizer replacing Cairo stamping: rayon row-bands, 8-wide SIMD
//! (AVX2+FMA runtime dispatch), the same analytic Gaussian beam as the
//! GPU — parity by construction, not by tuning.
//!
//! The baseline this exists to bury (BENCH.md): v3 Cairo at max settings
//! = 7 fps fullscreen with ONE core pegged, 3 fps under noise. Noise
//! (screen-diagonal segments) is the budget workload: a stalled-frame
//! call can carry 32,000 of them (phosphor-dsp worst case).
//!
//! Layout: two f32 energy planes (flash, glow) at width × height ×
//! supersample². Per frame: decay both planes (SIMD multiply, floor
//! subtract), bin segments into row bands, deposit in parallel (each
//! band owns its rows exclusively — no atomics), then composite through
//! phosphor_beam::composite_pixel with an exact box downfilter.

use rayon::prelude::*;
use wide::f32x8;

use phosphor_beam::{beam_normalization, beam_sigma, composite_pixel_fast,
                    glow_keep, hash_dither, CompositeLuts, CompositeParams,
                    Theme, ENERGY_FLOOR, FLASH_KEEP};

mod raster;

use raster::{beam_radius, deposit_rows, detect, prepare, Dispatch,
             PreparedSegment};

/// Rows per parallel band. 32 rows × 2560 px × 2 planes ≈ 640 KB of f32
/// per band at fullscreen — small enough to sit in L2 while a band's
/// segment list replays over it.
const BAND_ROWS: usize = 32;

pub struct CpuRenderer {
    width: usize,
    height: usize,
    supersample: usize,
    buffer_width: usize,
    buffer_height: usize,
    flash: Vec<f32>,
    glow: Vec<f32>,
    rgba: Vec<u8>,
    dispatch: Dispatch,
    composite_luts: CompositeLuts,

    pub beam_focus: f32,
    pub persistence: f32,
    pub theme: Theme,
    pub grid_enabled: bool,
    pub grid_spacing_fraction: f32,
    pub scope_alpha: f32,
    /// Display px per logical point × the live resolution fraction.
    /// σ carries it so the on-screen beam width equals `beam_focus`
    /// logical px at any DPI/resolution (v3 law); offline stays 1.0.
    pub display_scale: f32,
    /// Emit premultiplied alpha (for compositors that want it).
    pub premultiplied: bool,
}

impl CpuRenderer {
    pub fn new(width: usize, height: usize, supersample: usize)
               -> CpuRenderer {
        let supersample = supersample.max(1);
        let buffer_width = width * supersample;
        let buffer_height = height * supersample;
        CpuRenderer {
            width,
            height,
            supersample,
            buffer_width,
            buffer_height,
            flash: vec![0.0; buffer_width * buffer_height],
            glow: vec![0.0; buffer_width * buffer_height],
            rgba: vec![0u8; width * height * 4],
            dispatch: detect(),
            composite_luts: CompositeLuts::default(),
            beam_focus: 1.6,
            persistence: 0.7,
            theme: Theme::preset("P7 Green").unwrap(),
            grid_enabled: true,
            grid_spacing_fraction: 0.1125,
            scope_alpha: 1.0,
            display_scale: 1.0,
            premultiplied: false,
        }
    }

    pub fn simd_label(&self) -> &'static str {
        match self.dispatch {
            Dispatch::Avx2 => "avx2+fma",
            Dispatch::Portable => "portable",
        }
    }

    /// Decay both planes, then deposit this frame's segments (logical
    /// pixels; scaled by supersample here, mirroring v3's pixel_scale).
    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn advance(&mut self, segments: &[[f32; 5]]) {
        let flash_keep = f32x8::splat(FLASH_KEEP);
        let slow_keep = f32x8::splat(glow_keep(self.persistence));
        let floor = f32x8::splat(ENERGY_FLOOR);
        let zero = f32x8::ZERO;
        let decay = |plane: &mut [f32], keep: f32x8| {
            plane.par_chunks_mut(self.buffer_width * BAND_ROWS)
                .for_each(|chunk| {
                    let mut exact = chunk.chunks_exact_mut(8);
                    for lanes in exact.by_ref() {
                        let loaded: [f32; 8] = (&*lanes).try_into()
                            .expect("chunks_exact(8)");
                        let value = (f32x8::from(loaded) * keep - floor)
                            .max(zero);
                        lanes.copy_from_slice(&value.to_array());
                    }
                    for value in exact.into_remainder() {
                        *value = (*value * keep.to_array()[0]
                                  - ENERGY_FLOOR).max(0.0);
                    }
                });
        };
        decay(&mut self.flash, flash_keep);
        decay(&mut self.glow, slow_keep);

        if segments.is_empty() {
            return;
        }
        let pixel_scale = self.supersample as f32;
        // positions are in trace px — only σ carries the display scale
        let sigma = beam_sigma(self.beam_focus,
                               pixel_scale * self.display_scale.max(0.1));
        let radius = beam_radius(sigma);
        let normalization = beam_normalization(self.beam_focus);

        let prepared: Vec<PreparedSegment> = segments.iter()
            .filter_map(|segment| prepare(segment, pixel_scale, radius,
                                          self.buffer_width,
                                          self.buffer_height))
            .collect();

        // bin per band so each parallel worker only walks what it can hit
        let band_count = self.buffer_height.div_ceil(BAND_ROWS);
        let mut bins: Vec<Vec<u32>> = vec![Vec::new(); band_count];
        for (index, segment) in prepared.iter().enumerate() {
            let first = segment.row_start / BAND_ROWS;
            let last = (segment.row_end.saturating_sub(1)) / BAND_ROWS;
            for bin in &mut bins[first..=last.min(band_count - 1)] {
                bin.push(index as u32);
            }
        }

        let dispatch = self.dispatch;
        let buffer_width = self.buffer_width;
        let flash_bands = self.flash.par_chunks_mut(
            buffer_width * BAND_ROWS);
        let glow_bands = self.glow.par_chunks_mut(
            buffer_width * BAND_ROWS);
        flash_bands.zip(glow_bands).enumerate()
            .for_each(|(band, (flash, glow))| {
                let band_row = band * BAND_ROWS;
                let band_end = band_row + flash.len() / buffer_width;
                for &index in &bins[band] {
                    deposit_rows(dispatch, &prepared[index as usize],
                                 sigma, normalization, band_row, band_row,
                                 band_end, buffer_width, flash, glow);
                }
            });
    }

    /// Composite the energy planes to RGBA8 (straight alpha unless
    /// `premultiplied`); returns the frame, valid until the next call.
    pub fn composite(&mut self) -> &[u8] {
        let prepared = CompositeParams {
            theme: self.theme,
            grid_enabled: self.grid_enabled,
            grid_spacing: self.grid_spacing_fraction
                * self.width.min(self.height) as f32,
            scope_alpha: self.scope_alpha,
            width: self.width as f32,
            height: self.height as f32,
        }.prepare();
        let supersample = self.supersample;
        let buffer_width = self.buffer_width;
        let flash = &self.flash;
        let glow = &self.glow;
        let width = self.width;
        let premultiplied = self.premultiplied;
        let inverse_area = 1.0 / (supersample * supersample) as f32;
        let composite_luts = &self.composite_luts;

        self.rgba.par_chunks_mut(width * 4).enumerate()
            .for_each(|(y, row)| {
                for x in 0..width {
                    let (mut flash_energy, mut glow_energy) = (0.0, 0.0);
                    if supersample == 1 {
                        let index = y * buffer_width + x;
                        flash_energy = flash[index];
                        glow_energy = glow[index];
                    } else {
                        // exact box average — the anti-shimmer law
                        let base_y = y * supersample;
                        let base_x = x * supersample;
                        for sub_y in 0..supersample {
                            let row_base =
                                (base_y + sub_y) * buffer_width + base_x;
                            for sub_x in 0..supersample {
                                flash_energy += flash[row_base + sub_x];
                                glow_energy += glow[row_base + sub_x];
                            }
                        }
                        flash_energy *= inverse_area;
                        glow_energy *= inverse_area;
                    }
                    let pixel = composite_pixel_fast(
                        flash_energy, glow_energy,
                        x as f32 + 0.5, y as f32 + 0.5, &prepared,
                        composite_luts,
                        hash_dither(x as u32, y as u32));
                    let alpha = pixel[3];
                    let out = &mut row[x * 4..x * 4 + 4];
                    for channel in 0..3 {
                        let mut value = pixel[channel].clamp(0.0, 1.0);
                        if premultiplied {
                            value *= alpha;
                        }
                        out[channel] = (value * 255.0 + 0.5) as u8;
                    }
                    out[3] = (alpha.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                }
            });
        &self.rgba
    }

    pub fn render(&mut self, segments: &[[f32; 5]]) -> &[u8] {
        self.advance(segments);
        self.composite()
    }

    pub fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glass_alpha_carries_the_pane_and_opaque_bytes_hold() {
        // AMOLED black + grid off: zero background brightness, so the
        // pane alpha is EXACTLY scope_alpha (the glass law) and the
        // beam is the only light. Drive the beam to steady state so
        // its core saturates the alpha raise.
        let mut renderer = CpuRenderer::new(64, 64, 1);
        renderer.theme =
            Theme::preset("P7 Green").unwrap().with_amoled();
        renderer.grid_enabled = false;
        for _ in 0..50 {
            renderer.advance(&[[16.0, 16.0, 48.0, 48.0, 0.9]]);
        }
        let baseline = renderer.composite().to_vec();
        assert!(baseline.chunks(4).all(|px| px[3] == 255),
                "default composite must stay fully opaque");

        // golden safety: scope_alpha 1.0 (set explicitly, and with the
        // live path's premultiply) is byte-identical to the default
        renderer.scope_alpha = 1.0;
        renderer.premultiplied = true;
        assert_eq!(renderer.composite(), &baseline[..],
                   "opaque premultiplied composite changed bytes");
        renderer.premultiplied = false;

        // glass: background carries the pane, the beam stays lit
        renderer.scope_alpha = 0.5;
        let glass = renderer.composite().to_vec();
        for (before, after) in
            baseline.chunks(4).zip(glass.chunks(4)) {
            assert_eq!(&before[..3], &after[..3],
                       "straight-alpha glass must not touch RGB");
        }
        // corner (0,0) is > 3.5σ from the segment: pure background
        assert_eq!(glass[3], 128,
                   "background alpha must be the pane (0.5 → 128)");
        let alpha_peak = glass.chunks(4)
            .map(|px| px[3]).max().unwrap();
        assert_eq!(alpha_peak, 255,
                   "the beam core must raise the pane to opaque");

        // premultiplied glass: black background stays black, the beam
        // survives the multiply (what the live upload paints)
        renderer.premultiplied = true;
        let premultiplied = renderer.composite().to_vec();
        assert_eq!(&premultiplied[..4], &[0, 0, 0, 128],
                   "premultiplied AMOLED background must be 0,0,0,pane");
        let beam_peak = premultiplied.chunks(4)
            .map(|px| px[..3].iter().copied().max().unwrap())
            .max().unwrap();
        assert!(beam_peak > 200,
                "premultiply dimmed the beam core to {beam_peak}");
    }

    #[test]
    fn energy_decays_to_true_zero() {
        let mut renderer = CpuRenderer::new(64, 64, 1);
        renderer.advance(&[[10.0, 10.0, 50.0, 50.0, 0.9]]);
        let peak_before: f32 =
            renderer.flash.iter().cloned().fold(0.0, f32::max);
        assert!(peak_before > 0.01, "beam deposited nothing");
        for _ in 0..400 {
            renderer.advance(&[]);
        }
        assert!(renderer.flash.iter().all(|&energy| energy == 0.0),
                "flash floor never reached zero");
        assert!(renderer.glow.iter().all(|&energy| energy == 0.0),
                "glow floor never reached zero");
    }

    #[test]
    fn deposit_matches_scalar_reference() {
        // the SIMD path must agree with phosphor_beam::deposit exactly
        // enough that renderer parity is decided by physics, not lanes
        let mut renderer = CpuRenderer::new(48, 48, 1);
        renderer.persistence = 0.0;
        renderer.advance(&[[8.0, 9.0, 40.0, 31.0, 0.7]]);
        let sigma = beam_sigma(renderer.beam_focus, 1.0);
        let normalization = beam_normalization(renderer.beam_focus);
        let (p0, p1) = ([8.0f32, 9.0f32], [40.0f32, 31.0f32]);
        let direction = [p1[0] - p0[0], p1[1] - p0[1]];
        let length = (direction[0] * direction[0]
                      + direction[1] * direction[1]).sqrt();
        let tangent = [direction[0] / length, direction[1] / length];
        // the rasterizer (like v3's GL quads) cuts the Gaussian at
        // 3.5σ; compare inside that support, expect zero outside the
        // segment's AABB expansion
        let radius = sigma * phosphor_beam::BEAM_RADIUS_SIGMAS;
        let min_x = p0[0].min(p1[0]) - radius;
        let max_x = p0[0].max(p1[0]) + radius;
        let min_y = p0[1].min(p1[1]) - radius;
        let max_y = p0[1].max(p1[1]) + radius;
        let mut worst = 0.0f32;
        for y in 0..48 {
            for x in 0..48 {
                let pixel = [x as f32 + 0.5, y as f32 + 0.5];
                // prepare() rounds the quad out to whole pixels, so
                // give the float rect a 2 px skirt before demanding 0
                let inside = pixel[0] >= min_x - 2.0
                    && pixel[0] <= max_x + 2.0
                    && pixel[1] >= min_y - 2.0 && pixel[1] <= max_y + 2.0;
                let actual = renderer.flash[y * 48 + x];
                if !inside {
                    assert_eq!(actual, 0.0,
                               "deposit escaped the quad at {x},{y}");
                    continue;
                }
                let to_pixel = [pixel[0] - p0[0], pixel[1] - p0[1]];
                let along = to_pixel[0] * tangent[0]
                    + to_pixel[1] * tangent[1];
                let perpendicular = to_pixel[1] * tangent[0]
                    - to_pixel[0] * tangent[1];
                let expected = phosphor_beam::deposit(
                    along, perpendicular, length, sigma, 0.7,
                    normalization);
                // an unevaluated pixel (outside the integer quad) may
                // carry a sub-tail expectation: 3.5σ tail ≈ 0.22 % of
                // peak, the deliberate v3 cutoff — not lane drift
                if actual == 0.0 && expected < 2.5e-3 {
                    continue;
                }
                if expected > 1e-6 || actual > 1e-6 {
                    worst = worst.max((expected - actual).abs());
                }
            }
        }
        assert!(worst < 3e-5, "SIMD deposit drifted {worst} from scalar");
    }
}
