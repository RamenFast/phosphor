// SPDX-License-Identifier: GPL-3.0-or-later
//! THE beam model — one definition of what phosphor looks like, consumed
//! by both renderers. Ported from v3's GL shaders (phosphor_render_gl.py),
//! which are the canonical physics:
//!
//! - Two energy layers per pixel: flash (P7 fluorescence, fast) and glow
//!   (phosphorescence, slow). Decay each frame: `E = max(E·keep − 0.0004, 0)`
//!   — the floor lets faint trails truly reach zero. Flash keep is 0.50;
//!   glow keep is `1 − max(0.02, (1 − persistence)·0.6)`.
//! - Deposit: the analytic line integral of a Gaussian beam swept along
//!   the segment — erf() along the axis (sums exactly across consecutive
//!   segments: joints never double-deposit), Gaussian across it. Glow
//!   receives 0.85× the flash deposit. Energy is normalized against the
//!   LOGICAL focus (1.6 / max(0.4, focus)) so focus and supersampling
//!   redistribute energy instead of dimming the trace.
//! - Composite: exact box-average of the supersampled energy (bilinear
//!   would blend 2×2 of a 3×3 kernel and shimmer), phosphor saturation
//!   tonemap `1 − e^(−0.7·E)`, then LINEAR-LIGHT blending: theme colors
//!   are decoded (^2.2), light adds linearly, the sum re-encodes (^1/2.2).
//!   Procedural graticule and the glass-alpha law live in the same pixel
//!   law so both renderers agree everywhere.
//!
//! Sharpness history, so nobody "fixes" this backwards: v3.5's GL
//! composite already blended in linear light with an exact box
//! downfilter. The lived "GPU softer than CPU" came from the two
//! renderers having DIFFERENT beam physics — Cairo stamped hard round-cap
//! strokes (all edge), GL deposited a true Gaussian (physically soft).
//! v4 gives both renderers this one Gaussian model; sharpness parity is
//! by construction, and the wave-1 exit criterion (GPU ≥ CPU) is a
//! measured invariant, not a tuning fight.

/// Fraction of flash energy kept each frame (P7 fluorescence dies fast).
pub const FLASH_KEEP: f32 = 0.50;
/// Subtracted after the multiplicative decay so trails reach true zero.
pub const ENERGY_FLOOR: f32 = 0.0004;
/// The glow layer receives this fraction of every flash deposit.
pub const GLOW_COUPLING: f32 = 0.85;
/// Quad half-width in sigmas: covers the Gaussian to 0.2 % of peak
/// (below one 8-bit step), ~25 % less fill than 4σ.
pub const BEAM_RADIUS_SIGMAS: f32 = 3.5;
/// Phosphor saturation constant in `1 − e^(−k·E)`.
pub const TONEMAP_K: f32 = 0.7;
/// Flash tint contribution is scaled by this at composite.
pub const FLASH_COMPOSITE_WEIGHT: f32 = 0.6;
/// Display gamma both directions (v3 used 2.2, not the sRGB piecewise).
pub const GAMMA: f32 = 2.2;

/// Per-frame keep factor for the glow layer at a given persistence.
#[inline]
pub fn glow_keep(persistence: f32) -> f32 {
    1.0 - (0.02f32).max((1.0 - persistence) * 0.6)
}

/// Beam sigma in energy-buffer pixels: logical focus × (scale·supersample).
#[inline]
pub fn beam_sigma(beam_focus: f32, pixel_scale: f32) -> f32 {
    beam_focus.max(0.4) * pixel_scale
}

/// Normalized against the logical focus so focus/supersample changes
/// redistribute energy instead of dimming the trace.
#[inline]
pub fn beam_normalization(beam_focus: f32) -> f32 {
    1.6 / beam_focus.max(0.4)
}

/// Abramowitz & Stegun 7.1.27, max error ~5e-4 — plenty for beam energy.
/// Both renderers MUST use this same approximation (the GPU shader
/// carries an identical WGSL transcription) or snapshots drift.
#[inline]
pub fn erf_approximation(x: f32) -> f32 {
    let sign_x = if x < 0.0 { -1.0 } else { 1.0 };
    let a = x.abs();
    let mut d = 1.0 + (0.278393 + (0.230389 + 0.078108 * a * a) * a) * a;
    d *= d;
    sign_x - sign_x / (d * d)
}

/// Energy deposited at a pixel (`along`, `perpendicular` in the segment's
/// frame, buffer pixels) by a segment of `length` — the scalar reference
/// both rasterizers implement.
#[inline]
pub fn deposit(along: f32, perpendicular: f32, length: f32, sigma: f32,
               intensity: f32, normalization: f32) -> f32 {
    let inverse_sigma_sqrt2 = std::f32::consts::FRAC_1_SQRT_2 / sigma;
    let along_integral = 0.5
        * (erf_approximation(along * inverse_sigma_sqrt2)
           - erf_approximation((along - length) * inverse_sigma_sqrt2));
    let cross_section =
        (-perpendicular * perpendicular / (2.0 * sigma * sigma)).exp();
    intensity * cross_section * along_integral * normalization
}

// ---------------------------------------------------------------------------
// Themes (data, not code — v3's presets round-trip exactly)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Theme {
    pub beam_color: [f32; 3],
    pub flash_color: [f32; 3],
    pub grid_color: [f32; 3],
    pub background_color: [f32; 3],
}

pub const THEME_PRESETS: [(&str, Theme); 9] = [
    ("P7 Green", Theme {
        beam_color: [0.42, 1.0, 0.55], flash_color: [0.72, 0.85, 1.0],
        grid_color: [0.35, 1.0, 0.45],
        background_color: [0.013, 0.022, 0.015],
    }),
    ("Amber", Theme {
        beam_color: [1.0, 0.62, 0.12], flash_color: [1.0, 0.93, 0.65],
        grid_color: [1.0, 0.62, 0.12],
        background_color: [0.028, 0.016, 0.0],
    }),
    ("Ice Blue", Theme {
        beam_color: [0.35, 0.75, 1.0], flash_color: [0.85, 0.94, 1.0],
        grid_color: [0.35, 0.75, 1.0],
        background_color: [0.0, 0.015, 0.03],
    }),
    ("White", Theme {
        beam_color: [0.92, 0.95, 1.0], flash_color: [1.0, 1.0, 1.0],
        grid_color: [0.75, 0.8, 0.85],
        background_color: [0.016, 0.016, 0.02],
    }),
    ("Vaporwave", Theme {
        beam_color: [1.0, 0.30, 0.88], flash_color: [0.65, 0.95, 1.0],
        grid_color: [0.55, 0.40, 0.95],
        background_color: [0.02, 0.0, 0.03],
    }),
    ("Red Phosphor", Theme {
        beam_color: [1.0, 0.22, 0.16], flash_color: [1.0, 0.82, 0.70],
        grid_color: [1.0, 0.28, 0.22],
        background_color: [0.03, 0.004, 0.0],
    }),
    ("Ultraviolet", Theme {
        beam_color: [0.62, 0.40, 1.0], flash_color: [0.86, 0.80, 1.0],
        grid_color: [0.58, 0.40, 1.0],
        background_color: [0.014, 0.0, 0.03],
    }),
    ("Solar Gold", Theme {
        beam_color: [1.0, 0.84, 0.30], flash_color: [1.0, 1.0, 0.86],
        grid_color: [0.92, 0.76, 0.30],
        background_color: [0.026, 0.018, 0.0],
    }),
    ("Cyan Tube", Theme {
        beam_color: [0.20, 1.0, 0.92], flash_color: [0.85, 1.0, 1.0],
        grid_color: [0.22, 0.90, 0.85],
        background_color: [0.0, 0.024, 0.026],
    }),
];

impl Theme {
    pub fn preset(name: &str) -> Option<Theme> {
        THEME_PRESETS.iter()
            .find(|(preset_name, _)| *preset_name == name)
            .map(|(_, theme)| *theme)
    }

    /// AMOLED applies to every theme, custom included: true black.
    pub fn with_amoled(self) -> Theme {
        Theme { background_color: [0.0; 3], ..self }
    }

    /// Derive a full theme from the two user-picked colors (v3 law).
    pub fn custom(beam_color: [f32; 3], grid_color: [f32; 3]) -> Theme {
        let flash_color =
            beam_color.map(|channel| (channel * 0.4 + 0.6).min(1.0));
        let background_color = beam_color.map(|channel| channel * 0.03);
        Theme { beam_color, flash_color, grid_color, background_color }
    }
}

/// Screen fraction of one graticule division; steps by octaves like a
/// real scope's volts/div switch so the grid stays readable at any gain.
pub fn grid_spacing_fraction(gain: f32) -> f32 {
    let mut fraction = 0.45 * gain.max(0.001) / 4.0;
    while fraction < 0.05 {
        fraction *= 2.0;
    }
    while fraction > 0.30 {
        fraction /= 2.0;
    }
    fraction
}

// ---------------------------------------------------------------------------
// Gamma-encode LUT (the CPU composite's powf eraser)
// ---------------------------------------------------------------------------

/// Cells in the encode table; the +1 entry closes the last lerp span.
pub const ENCODE_LUT_CELLS: usize = 2048;
/// Linear-light ceiling the table covers: background + grid + beam +
/// 0.6·flash can never reach it (each term ≤ 1, weights sum < 4).
const ENCODE_LUT_MAX_LINEAR: f32 = 4.0;

/// x^(1/2.2) served by table, indexed by sqrt(x) so the steep dark end
/// of the curve — where glow falloff lives — gets its resolution. In
/// sqrt domain the curve is u^(2/2.2) ≈ u^0.91, nearly linear, so 2048
/// cells + lerp keep the worst deviation from powf below ~0.13 of an
/// 8-bit step (asserted by test). encode(0) is exactly 0: AMOLED black
/// stays black by construction, not by rounding.
pub struct EncodeLut {
    table: Vec<f32>,
    scale: f32,
}

impl EncodeLut {
    pub fn new() -> EncodeLut {
        let max_u = ENCODE_LUT_MAX_LINEAR.sqrt();
        let step = max_u / ENCODE_LUT_CELLS as f32;
        let table = (0..=ENCODE_LUT_CELLS)
            .map(|index| {
                let u = index as f32 * step;
                (u * u).powf(1.0 / GAMMA)
            })
            .collect();
        EncodeLut { table, scale: ENCODE_LUT_CELLS as f32 / max_u }
    }

    #[inline]
    pub fn encode(&self, linear: f32) -> f32 {
        let scaled = (linear.max(0.0).sqrt() * self.scale)
            .min(ENCODE_LUT_CELLS as f32);
        let index = (scaled as usize).min(ENCODE_LUT_CELLS - 1);
        let fraction = scaled - index as f32;
        self.table[index]
            + (self.table[index + 1] - self.table[index]) * fraction
    }
}

impl Default for EncodeLut {
    fn default() -> EncodeLut {
        EncodeLut::new()
    }
}

/// Tonemap `1 − e^(−0.7·E)` by table. The curve's slope is bounded by
/// 0.7, so a uniform index is already sub-LSB at 2048 cells; beyond the
/// domain the true curve is within 1.4e-5 of the table's final 1-ish
/// entry. No sqrt trick needed.
pub struct TonemapLut {
    table: Vec<f32>,
    scale: f32,
}

const TONEMAP_LUT_MAX_ENERGY: f32 = 16.0;

impl TonemapLut {
    pub fn new() -> TonemapLut {
        let step = TONEMAP_LUT_MAX_ENERGY / ENCODE_LUT_CELLS as f32;
        let table = (0..=ENCODE_LUT_CELLS)
            .map(|index| 1.0 - (-TONEMAP_K * index as f32 * step).exp())
            .collect();
        TonemapLut {
            table,
            scale: ENCODE_LUT_CELLS as f32 / TONEMAP_LUT_MAX_ENERGY,
        }
    }

    #[inline]
    pub fn apply(&self, energy: f32) -> f32 {
        let scaled = (energy.max(0.0) * self.scale)
            .min(ENCODE_LUT_CELLS as f32);
        let index = (scaled as usize).min(ENCODE_LUT_CELLS - 1);
        let fraction = scaled - index as f32;
        self.table[index]
            + (self.table[index + 1] - self.table[index]) * fraction
    }
}

impl Default for TonemapLut {
    fn default() -> TonemapLut {
        TonemapLut::new()
    }
}

/// Both composite tables, built once per renderer.
#[derive(Default)]
pub struct CompositeLuts {
    pub encode: EncodeLut,
    pub tonemap: TonemapLut,
}

/// Integer-hash dither for the CPU frame loop: same sub-LSB job as the
/// canonical GLSL sin hash, minus the sin. The two renderers' dither
/// PATTERNS differ (each keeps its natural hash); both are gated below
/// ~1 LSB, so the cross-snapshot tolerance never sees them.
#[inline]
pub fn hash_dither(x: u32, y: u32) -> f32 {
    let mut h = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h as f32 * (1.0 / 4_294_967_296.0)
}

// ---------------------------------------------------------------------------
// The composite pixel law (scalar reference)
// ---------------------------------------------------------------------------

/// Everything the per-pixel composite needs besides the energy pair.
#[derive(Clone, Copy, Debug)]
pub struct CompositeParams {
    pub theme: Theme,
    pub grid_enabled: bool,
    /// Device pixels per graticule division (fraction × min(w, h)).
    pub grid_spacing: f32,
    /// 1 = opaque scope; lower = glass pane over the desktop.
    pub scope_alpha: f32,
    pub width: f32,
    pub height: f32,
}

#[inline]
pub fn srgb_to_linear(encoded: [f32; 3]) -> [f32; 3] {
    encoded.map(|channel| channel.max(0.0).powf(GAMMA))
}

#[inline]
fn grid_line(coordinate: f32, spacing: f32) -> f32 {
    let distance =
        (coordinate - spacing * (coordinate / spacing + 0.5).floor()).abs();
    1.0 - smoothstep(0.4, 1.0, distance)
}

#[inline]
fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Theme colors decoded to linear light once per frame — they are
/// constants of the composite, and decoding them per pixel is where a
/// naive CPU composite burns its whole budget (16 transcendentals per
/// pixel). Hoisting is algebraically identical to the per-pixel law.
#[derive(Clone, Copy, Debug)]
pub struct PreparedComposite {
    pub params: CompositeParams,
    pub background_linear: [f32; 3],
    pub grid_linear: [f32; 3],
    pub beam_linear: [f32; 3],
    pub flash_linear: [f32; 3],
}

impl CompositeParams {
    pub fn prepare(self) -> PreparedComposite {
        PreparedComposite {
            background_linear: srgb_to_linear(self.theme.background_color),
            grid_linear: srgb_to_linear(self.theme.grid_color),
            beam_linear: srgb_to_linear(self.theme.beam_color),
            flash_linear: srgb_to_linear(self.theme.flash_color),
            params: self,
        }
    }
}

/// The full v3 composite law for one pixel: box-averaged energy in,
/// display-encoded RGBA out (0..1, straight alpha). `x`/`y` are device
/// pixel centers measured the same way gl_FragCoord does (x + 0.5).
pub fn composite_pixel(flash_energy: f32, glow_energy: f32, x: f32, y: f32,
                       params: &CompositeParams) -> [f32; 4] {
    composite_pixel_prepared(flash_energy, glow_energy, x, y,
                             &params.prepare())
}

/// Same law with the frame-constant decodes hoisted (the hot path).
pub fn composite_pixel_prepared(flash_energy: f32, glow_energy: f32,
                                x: f32, y: f32,
                                prepared: &PreparedComposite) -> [f32; 4] {
    #[allow(clippy::excessive_precision)] // the canonical GLSL hash
    let noise = ((x * 12.9898 + y * 78.233).sin() * 43758.5453).fract();
    let noise = if noise < 0.0 { noise + 1.0 } else { noise };
    composite_pixel_impl(flash_energy, glow_energy, x, y, prepared,
                         |channel| channel.powf(1.0 / GAMMA),
                         |energy| 1.0 - (-TONEMAP_K * energy).exp(),
                         noise)
}

/// The law with every per-pixel transcendental served from tables (and
/// the dither noise supplied by the caller — the CPU loop uses the
/// integer `hash_dither`). Each table is bounded ≤ ~0.13 of an 8-bit
/// step from its exact form (asserted by tests): invisible in output.
pub fn composite_pixel_fast(flash_energy: f32, glow_energy: f32,
                            x: f32, y: f32,
                            prepared: &PreparedComposite,
                            luts: &CompositeLuts, noise: f32) -> [f32; 4] {
    composite_pixel_impl(flash_energy, glow_energy, x, y, prepared,
                         |channel| luts.encode.encode(channel),
                         |energy| luts.tonemap.apply(energy),
                         noise)
}

#[inline]
fn composite_pixel_impl<E: Fn(f32) -> f32, T: Fn(f32) -> f32>(
    flash_energy: f32, glow_energy: f32, x: f32, y: f32,
    prepared: &PreparedComposite, encode: E, tonemap: T,
    noise: f32) -> [f32; 4] {
    let params = &prepared.params;
    let flash = tonemap(flash_energy);
    let glow = tonemap(glow_energy);

    let mut color = prepared.background_linear;
    if params.grid_enabled {
        // centered so divisions track the beam's amplitude scale (zoom)
        let from_center_x = x - params.width * 0.5;
        let from_center_y = y - params.height * 0.5;
        let minor = grid_line(from_center_x, params.grid_spacing)
            .max(grid_line(from_center_y, params.grid_spacing));
        let axis = (1.0 - smoothstep(0.5, 1.2, from_center_x.abs()))
            .max(1.0 - smoothstep(0.5, 1.2, from_center_y.abs()));
        // linear-light equivalents of the old 0.07 / 0.10 display levels
        let grid_level = minor * 0.003 + axis * 0.0063;
        for (channel, grid_channel) in
            color.iter_mut().zip(prepared.grid_linear) {
            *channel += grid_channel * grid_level;
        }
    }
    for ((channel, beam_channel), flash_channel) in
        color.iter_mut().zip(prepared.beam_linear)
            .zip(prepared.flash_linear) {
        *channel += beam_channel * glow
            + flash_channel * flash * FLASH_COMPOSITE_WEIGHT;
    }
    let mut encoded = color.map(&encode);

    // dither breaks 8-bit banding rings in the dark glow falloff,
    // gated below ~1 LSB so AMOLED black stays exactly black
    let brightness = encoded[0].max(encoded[1]).max(encoded[2]);
    let dither_gate = smoothstep(0.0, 0.004, brightness);
    for channel in encoded.iter_mut() {
        *channel += (noise - 0.5) / 255.0 * dither_gate;
    }

    // glass: the pane is scope_alpha; the beam's own light raises opacity
    let alpha = (params.scope_alpha
        + (1.0 - params.scope_alpha) * brightness * 2.0).clamp(0.0, 1.0);
    [encoded[0], encoded[1], encoded[2], alpha]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keep_laws_match_v3() {
        assert!((glow_keep(0.7) - (1.0 - 0.18f32)).abs() < 1e-6);
        assert!((glow_keep(1.0) - 0.98).abs() < 1e-6); // floor 0.02
        assert!((glow_keep(0.0) - 0.4).abs() < 1e-6);
    }

    #[test]
    fn erf_is_odd_and_bounded() {
        for i in 0..200 {
            let x = (i as f32 - 100.0) / 20.0;
            let e = erf_approximation(x);
            assert!((-1.0..=1.0).contains(&e));
            assert!((e + erf_approximation(-x)).abs() < 1e-6);
        }
        assert!((erf_approximation(1.0) - 0.8427).abs() < 6e-4);
    }

    #[test]
    fn segment_joints_sum_exactly() {
        // erf along-integrals of [0,L] and [L,2L] must equal one [0,2L]
        let sigma = 1.6;
        let normalization = 1.0;
        let at = |along: f32, length: f32| {
            deposit(along, 0.4, length, sigma, 1.0, normalization)
        };
        let joined = at(3.0, 8.0);
        let split = at(3.0, 4.0)
            + deposit(3.0 - 4.0, 0.4, 4.0, sigma, 1.0, normalization);
        assert!((joined - split).abs() < 1e-5, "{joined} vs {split}");
    }

    #[test]
    fn themes_round_trip_v3() {
        assert_eq!(THEME_PRESETS.len(), 9);
        let p7 = Theme::preset("P7 Green").unwrap();
        assert_eq!(p7.beam_color, [0.42, 1.0, 0.55]);
        assert_eq!(p7.with_amoled().background_color, [0.0; 3]);
        let custom = Theme::custom([0.42, 1.0, 0.55], [0.35, 1.0, 0.45]);
        assert!((custom.flash_color[0] - (0.42 * 0.4 + 0.6)).abs() < 1e-6);
        assert!((custom.background_color[1] - 0.03).abs() < 1e-6);
        // grid spacing octave law
        assert!((grid_spacing_fraction(1.0) - 0.1125).abs() < 1e-6);
        let tiny = grid_spacing_fraction(0.001);
        assert!(tiny >= 0.05 && tiny <= 0.30);
    }

    #[test]
    fn tonemap_lut_and_hash_dither_hold_their_bounds() {
        let lut = TonemapLut::new();
        let mut worst = 0.0f32;
        for i in 0..40_000 {
            let energy = i as f32 * (20.0 / 40_000.0); // past the domain
            let exact = 1.0 - (-TONEMAP_K * energy).exp();
            worst = worst.max((lut.apply(energy) - exact).abs());
        }
        assert!(worst < 2e-5, "tonemap LUT drifted {worst}");
        let mut sum = 0.0f64;
        for x in 0..200u32 {
            for y in 0..200u32 {
                let noise = hash_dither(x, y);
                assert!((0.0..1.0).contains(&noise));
                sum += noise as f64;
            }
        }
        let mean = sum / 40_000.0;
        assert!((mean - 0.5).abs() < 0.01, "dither biased: mean {mean}");
    }

    #[test]
    fn encode_lut_stays_under_an_eighth_lsb_of_powf() {
        let lut = EncodeLut::new();
        assert_eq!(lut.encode(0.0), 0.0, "true black must stay exact");
        let mut worst = 0.0f32;
        // dense linear sweep plus log-spaced dark values where the
        // curve is steepest
        for i in 1..40_000 {
            let linear = i as f32 * (4.0 / 40_000.0);
            let delta = (lut.encode(linear)
                         - linear.powf(1.0 / GAMMA)).abs();
            worst = worst.max(delta);
        }
        for exponent in 1..240 {
            let linear = 10f32.powf(-(exponent as f32) / 30.0);
            let delta = (lut.encode(linear)
                         - linear.powf(1.0 / GAMMA)).abs();
            worst = worst.max(delta);
        }
        assert!(worst < 0.5 / 255.0 * 0.27,
                "LUT drifted {worst} from powf (limit ~0.13 LSB)");
    }

    #[test]
    fn amoled_black_does_not_sparkle() {
        let params = CompositeParams {
            theme: Theme::preset("P7 Green").unwrap().with_amoled(),
            grid_enabled: false,
            grid_spacing: 100.0,
            scope_alpha: 1.0,
            width: 800.0,
            height: 600.0,
        };
        for (x, y) in [(3.5, 7.5), (400.5, 300.5), (799.5, 0.5)] {
            let pixel = composite_pixel(0.0, 0.0, x, y, &params);
            assert_eq!(&pixel[..3], &[0.0, 0.0, 0.0],
                       "dither leaked into true black at {x},{y}");
        }
    }
}
