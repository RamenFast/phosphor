// SPDX-License-Identifier: GPL-3.0-or-later
//! The LIVE composite path (`composite_into`: viewport + scissor +
//! origin uniform) against the offline path. The wave-1 goldens are
//! squish-proof by construction (offline buffer size == output size),
//! so until now a live viewport bug could ship 19/19 green — these pin
//! the gap: placement is translation-only, clamping crops instead of
//! stretching, and a circle stays round on the live path.

use std::str::FromStr;

use phosphor_dsp::{Computer, Mode};
use phosphor_render_gpu::GpuRenderer;

const SCOPE_W: u32 = 320;
const SCOPE_H: u32 = 240;
const SURFACE_W: u32 = 1024;
const SURFACE_H: u32 = 768;
/// Viewport origin inside the stand-in surface (deliberately odd).
const ORIGIN_X: u32 = 137;
const ORIGIN_Y: u32 = 41;

fn gpu_or_skip() -> Option<GpuRenderer> {
    match GpuRenderer::new_offscreen(SCOPE_W, SCOPE_H, 1) {
        Ok(renderer) => Some(renderer),
        Err(error) => {
            eprintln!("SKIP: no usable GPU adapter ({error})");
            None
        }
    }
}

/// A slow circle (L=sin, R=cos) traced through the real DSP at the
/// scope's size — the canonical roundness fixture.
fn circle_frames() -> Vec<Vec<[f32; 5]>> {
    let mut computer = Computer::new();
    computer.mode = Mode::from_str("xy").unwrap();
    computer.gain = 1.0;
    computer.set_sample_rate(48000, 1);
    let mut frames = Vec::new();
    let mut phase = 0.0f64;
    for _ in 0..6 {
        let chunk: Vec<f32> = (0..3200)
            .flat_map(|_| {
                phase += 220.0 / 48000.0 * std::f64::consts::TAU;
                [(phase.sin() * 0.8) as f32, (phase.cos() * 0.8) as f32]
            })
            .collect();
        frames.push(
            computer.compute(&chunk, SCOPE_W as f32, SCOPE_H as f32)
                .to_vec());
    }
    frames
}

fn region(surface: &[u8], x0: u32, y0: u32, w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in 0..h {
        let start = (((y0 + row) * SURFACE_W + x0) * 4) as usize;
        out.extend_from_slice(&surface[start..start + (w * 4) as usize]);
    }
    out
}

fn lit_bbox(rgba: &[u8], width: u32, height: u32)
            -> Option<(u32, u32, u32, u32)> {
    let mut min_x = u32::MAX;
    let mut min_y = u32::MAX;
    let mut max_x = 0;
    let mut max_y = 0;
    for y in 0..height {
        for x in 0..width {
            let base = ((y * width + x) * 4) as usize;
            let luminance = rgba[base] as u32 + rgba[base + 1] as u32
                + rgba[base + 2] as u32;
            if luminance > 45 {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    (min_x != u32::MAX).then_some((min_x, min_y, max_x, max_y))
}

#[test]
fn live_viewport_is_translation_only() {
    let Some(mut gpu) = gpu_or_skip() else { return };
    gpu.grid_enabled = false;
    for frame in circle_frames() {
        gpu.advance(&frame);
    }
    let reference = gpu.composite_and_read();
    let surface = gpu.composite_into_read(
        SURFACE_W, SURFACE_H,
        (ORIGIN_X as f32, ORIGIN_Y as f32,
         SCOPE_W as f32, SCOPE_H as f32));
    let live = region(&surface, ORIGIN_X, ORIGIN_Y, SCOPE_W, SCOPE_H);

    // identical math, only the origin differs — bytes must match
    let differing = reference.iter().zip(&live)
        .filter(|(a, b)| a != b).count();
    assert_eq!(differing, 0,
               "live composite differs from offline in {differing} \
                bytes — the viewport path is no longer a pure \
                translation");

    // and nothing may leak outside the scissor
    let left_of = region(&surface, 0, 0, ORIGIN_X, SURFACE_H);
    assert!(left_of.chunks_exact(4).all(|p| p[0] == 0 && p[1] == 0
                                            && p[2] == 0),
            "beam bled left of the viewport");
}

#[test]
fn live_clamp_crops_never_stretches() {
    let Some(mut gpu) = gpu_or_skip() else { return };
    gpu.grid_enabled = false;
    for frame in circle_frames() {
        gpu.advance(&frame);
    }
    let reference = gpu.composite_and_read();

    // shell clamp law: x + w hangs off the surface edge → w shrinks
    // (shell.rs frame(): w = width.min(surface_w - x)); the visible
    // part must equal the reference's LEFT columns — cropped, not
    // squeezed
    let x0 = SURFACE_W - 200; // only 200 px of the 320 fit
    let clamped_w = 200u32;
    let surface = gpu.composite_into_read(
        SURFACE_W, SURFACE_H,
        (x0 as f32, ORIGIN_Y as f32,
         clamped_w as f32, SCOPE_H as f32));
    let live = region(&surface, x0, ORIGIN_Y, clamped_w, SCOPE_H);

    let mut differing = 0usize;
    for row in 0..SCOPE_H {
        let reference_start = ((row * SCOPE_W) * 4) as usize;
        let live_start = ((row * clamped_w) * 4) as usize;
        let a = &reference[reference_start
                           ..reference_start + (clamped_w * 4) as usize];
        let b = &live[live_start..live_start + (clamped_w * 4) as usize];
        differing += a.iter().zip(b).filter(|(x, y)| x != y).count();
    }
    assert_eq!(differing, 0,
               "clamped viewport does not equal the cropped reference \
                — something rescaled");
}

#[test]
fn circle_stays_round_on_the_live_path() {
    let Some(mut gpu) = gpu_or_skip() else { return };
    gpu.grid_enabled = false;
    for frame in circle_frames() {
        gpu.advance(&frame);
    }
    let surface = gpu.composite_into_read(
        SURFACE_W, SURFACE_H,
        (ORIGIN_X as f32, ORIGIN_Y as f32,
         SCOPE_W as f32, SCOPE_H as f32));
    let live = region(&surface, ORIGIN_X, ORIGIN_Y, SCOPE_W, SCOPE_H);

    let (min_x, min_y, max_x, max_y) =
        lit_bbox(&live, SCOPE_W, SCOPE_H).expect("a lit circle");
    let width = (max_x - min_x) as f64;
    let height = (max_y - min_y) as f64;
    let aspect = width / height;
    println!("live circle bbox {width}×{height}, aspect {aspect:.4}");
    assert!((aspect - 1.0).abs() < 0.03,
            "the circle is not round on the live path: aspect \
             {aspect:.4} — the goniometer-squish canary fired");
}
