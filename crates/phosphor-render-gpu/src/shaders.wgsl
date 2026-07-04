// SPDX-License-Identifier: GPL-3.0-or-later
// The three v3 passes as WGSL. The math is phosphor-beam's law verbatim;
// coordinates are simpler than the GL original because WGSL's
// @builtin(position) is already top-left framebuffer pixels — the CPU
// renderer and this file share one convention, no flips anywhere.

// ---------------------------------------------------------------- decay

struct DecayUniforms {
    keep: vec2f,          // (flash keep, glow keep) per frame
    _pad: vec2f,
}

@group(0) @binding(0) var decay_source: texture_2d<f32>;
@group(0) @binding(1) var<uniform> decay: DecayUniforms;

@vertex
fn fullscreen_vs(@builtin(vertex_index) index: u32) -> @builtin(position) vec4f {
    let corner = vec2f(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4f(corner * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn decay_fs(@builtin(position) position: vec4f) -> @location(0) vec4f {
    let energy = textureLoad(decay_source, vec2i(position.xy), 0).rg
        * decay.keep;
    // subtract the floor so faint trails truly reach zero
    return vec4f(max(energy - 0.0004, vec2f(0.0)), 0.0, 1.0);
}

// ----------------------------------------------------------------- beam

struct BeamUniforms {
    viewport: vec2f,      // energy buffer size in pixels
    radius: f32,          // 3.5 sigma: Gaussian covered to 0.2 % of peak
    sigma: f32,
    normalization: f32,   // 1.6 / max(0.4, logical focus)
    pixel_scale: f32,     // logical -> energy-buffer pixels, applied here
    _pad: vec2f,
}

@group(0) @binding(0) var<uniform> beam: BeamUniforms;

struct BeamVarying {
    @builtin(position) position: vec4f,
    @location(0) @interpolate(flat) p0: vec2f,
    @location(1) @interpolate(flat) p1: vec2f,
    @location(2) @interpolate(flat) intensity: f32,
}

@vertex
fn beam_vs(@builtin(vertex_index) vertex_index: u32,
           @location(0) segment: vec4f,
           @location(1) intensity: f32) -> BeamVarying {
    var corner_table = array<vec2f, 6>(
        vec2f(0.0, -1.0), vec2f(1.0, -1.0), vec2f(1.0, 1.0),
        vec2f(0.0, -1.0), vec2f(1.0, 1.0), vec2f(0.0, 1.0));
    let p0 = segment.xy * beam.pixel_scale;
    let p1 = segment.zw * beam.pixel_scale;
    let direction = p1 - p0;
    let segment_length = length(direction);
    var tangent = vec2f(1.0, 0.0);
    if segment_length > 1e-4 {
        tangent = direction / segment_length;
    }
    let normal = vec2f(-tangent.y, tangent.x);
    let corner = corner_table[vertex_index];
    let position = mix(p0 - tangent * beam.radius,
                       p1 + tangent * beam.radius, corner.x)
        + normal * beam.radius * corner.y;
    var output: BeamVarying;
    let ndc = position / beam.viewport * 2.0 - 1.0;
    // top-left pixel space -> NDC (y flips once, here only)
    output.position = vec4f(ndc.x, -ndc.y, 0.0, 1.0);
    output.p0 = p0;
    output.p1 = p1;
    output.intensity = intensity;
    return output;
}

// Abramowitz & Stegun 7.1.27 — identical to phosphor_beam::erf_approximation.
fn erf_approximation(x: f32) -> f32 {
    let sign_x = sign(x);
    let a = abs(x);
    var d = 1.0 + (0.278393 + (0.230389 + 0.078108 * a * a) * a) * a;
    d = d * d;
    return sign_x - sign_x / (d * d);
}

@fragment
fn beam_fs(input: BeamVarying) -> @location(0) vec4f {
    let pixel = input.position.xy;      // already top-left pixel centers
    let to_pixel = pixel - input.p0;
    let direction = input.p1 - input.p0;
    let segment_length = length(direction);
    var tangent = vec2f(1.0, 0.0);
    if segment_length > 1e-4 {
        tangent = direction / segment_length;
    }
    let along = dot(to_pixel, tangent);
    let perpendicular = dot(to_pixel, vec2f(-tangent.y, tangent.x));

    // analytic line integral of the Gaussian beam: erf along the axis
    // sums exactly across consecutive segments — joints never
    // double-deposit, dense scope art keeps its detail
    let inverse_sigma_sqrt2 = 0.70710678 / beam.sigma;
    let along_integral = 0.5
        * (erf_approximation(along * inverse_sigma_sqrt2)
           - erf_approximation((along - segment_length)
                               * inverse_sigma_sqrt2));
    let cross_section = exp(-perpendicular * perpendicular
                            / (2.0 * beam.sigma * beam.sigma));
    let energy = input.intensity * cross_section * along_integral
        * beam.normalization;
    return vec4f(energy, energy * 0.85, 0.0, 1.0);
}

// ------------------------------------------------------------ composite

struct CompositeUniforms {
    beam_color: vec4f,
    flash_color: vec4f,
    grid_color: vec4f,
    background_color: vec4f,
    viewport: vec2f,          // output size in pixels
    grid_spacing: f32,        // device pixels per graticule division
    grid_enabled: f32,
    supersample: i32,
    scope_alpha: f32,         // 1 = opaque; lower = glass over desktop
    // top-left of the scope viewport in framebuffer pixels — 0,0 for
    // the offscreen path (bytes unchanged, goldens hold), the scope
    // rect's origin when compositing into a window surface
    origin: vec2f,
}

@group(0) @binding(0) var composite_energy: texture_2d<f32>;
@group(0) @binding(1) var<uniform> composite: CompositeUniforms;

fn grid_line(coordinate: f32, spacing: f32) -> f32 {
    let distance_to_line =
        abs(coordinate - spacing * floor(coordinate / spacing + 0.5));
    return 1.0 - smoothstep(0.4, 1.0, distance_to_line);
}

// Theme colors arrive display-encoded; light adds linearly. Decode,
// blend in linear light, re-encode — faint trails stay visible.
fn srgb_to_linear(encoded: vec3f) -> vec3f {
    return pow(max(encoded, vec3f(0.0)), vec3f(2.2));
}

@fragment
fn composite_fs(@builtin(position) position: vec4f) -> @location(0) vec4f {
    // local scope pixels: identical to position.xy offscreen (origin 0)
    let local = position.xy - composite.origin;
    var energy = vec2f(0.0);
    if composite.supersample <= 1 {
        energy = textureLoad(composite_energy, vec2i(local), 0).rg;
    } else {
        // exact box average of the supersampled energy; bilinear would
        // blend 2x2 of the 3x3 kernel and shimmer on fine detail
        let base = vec2i(local) * composite.supersample;
        var sum = vec2f(0.0);
        for (var y = 0; y < composite.supersample; y++) {
            for (var x = 0; x < composite.supersample; x++) {
                sum += textureLoad(composite_energy, base + vec2i(x, y),
                                   0).rg;
            }
        }
        energy = sum
            / f32(composite.supersample * composite.supersample);
    }
    // phosphor saturation curve
    let flash = 1.0 - exp(-0.7 * energy.r);
    let glow = 1.0 - exp(-0.7 * energy.g);

    var color = srgb_to_linear(composite.background_color.rgb);
    if composite.grid_enabled > 0.5 {
        // centered so divisions track the beam's amplitude scale (zoom)
        let from_center = local - composite.viewport * 0.5;
        let minor = max(grid_line(from_center.x, composite.grid_spacing),
                        grid_line(from_center.y, composite.grid_spacing));
        let axis = max(1.0 - smoothstep(0.5, 1.2, abs(from_center.x)),
                       1.0 - smoothstep(0.5, 1.2, abs(from_center.y)));
        // linear-light equivalents of the old 0.07 / 0.10 display levels
        color += srgb_to_linear(composite.grid_color.rgb)
            * (minor * 0.003 + axis * 0.0063);
    }
    color += srgb_to_linear(composite.beam_color.rgb) * glow
        + srgb_to_linear(composite.flash_color.rgb) * flash * 0.6;
    color = pow(color, vec3f(1.0 / 2.2));

    // hash dither breaks 8-bit banding in the dark falloff, gated below
    // ~1 LSB so AMOLED black stays exactly black (local: offline-exact)
    let noise = fract(sin(dot(local, vec2f(12.9898, 78.233)))
                      * 43758.5453);
    let brightness = max(color.r, max(color.g, color.b));
    let dither_gate = smoothstep(0.0, 0.004, brightness);
    // glass: the pane is scope_alpha; the beam's own light raises opacity
    let alpha = clamp(composite.scope_alpha
                      + (1.0 - composite.scope_alpha) * brightness * 2.0,
                      0.0, 1.0);
    return vec4f(color + vec3f((noise - 0.5) / 255.0) * dither_gate,
                 alpha);
}
