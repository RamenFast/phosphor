// SPDX-License-Identifier: GPL-3.0-or-later
//! The two renderers against each other: same phosphor-dsp segment
//! streams, same beam law, frames compared on a downsampled luminance
//! grid. GPU-vs-CPU sharpness (v3's "GPU softer" bug) is measured, not
//! assumed. Skips with a notice when no adapter exists; on Ben's box
//! RADV runs these headless.

use std::str::FromStr;

use phosphor_dsp::{Computer, Mode};
use phosphor_render_cpu::CpuRenderer;
use phosphor_render_gpu::GpuRenderer;

const WIDTH: usize = 800;
const HEIGHT: usize = 600;
/// Worst allowed cell delta on the 16×16 luminance grid, u8 scale.
/// exp/pow differ slightly between GPU fast-math and CPU libm; the
/// grid mean absorbs per-pixel rounding while catching real drift.
const CELL_TOLERANCE: f32 = 2.5;

fn luminance_grid(rgba: &[u8], width: usize, height: usize)
                  -> [f32; 256] {
    let mut grid = [0.0f32; 256];
    let mut counts = [0u32; 256];
    for y in 0..height {
        let cell_y = y * 16 / height;
        for x in 0..width {
            let cell_x = x * 16 / width;
            let base = (y * width + x) * 4;
            let luminance = 0.2126 * rgba[base] as f32
                + 0.7152 * rgba[base + 1] as f32
                + 0.0722 * rgba[base + 2] as f32;
            grid[cell_y * 16 + cell_x] += luminance;
            counts[cell_y * 16 + cell_x] += 1;
        }
    }
    for (value, count) in grid.iter_mut().zip(counts) {
        *value /= count.max(1) as f32;
    }
    grid
}

fn worst_cell_delta(a: &[u8], b: &[u8]) -> f32 {
    let grid_a = luminance_grid(a, WIDTH, HEIGHT);
    let grid_b = luminance_grid(b, WIDTH, HEIGHT);
    grid_a.iter().zip(grid_b)
        .map(|(x, y)| (x - y).abs())
        .fold(0.0, f32::max)
}

fn mean_gradient(rgba: &[u8], width: usize, height: usize) -> f64 {
    let luma = |x: usize, y: usize| -> f64 {
        let base = (y * width + x) * 4;
        0.2126 * rgba[base] as f64 + 0.7152 * rgba[base + 1] as f64
            + 0.0722 * rgba[base + 2] as f64
    };
    let mut total = 0.0;
    let mut count = 0u64;
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let gx = luma(x + 1, y) - luma(x - 1, y);
            let gy = luma(x, y + 1) - luma(x, y - 1);
            total += (gx * gx + gy * gy).sqrt();
            count += 1;
        }
    }
    total / count as f64
}

fn gpu_or_skip(width: u32, height: u32, supersample: u32)
               -> Option<GpuRenderer> {
    match GpuRenderer::new_offscreen(width, height, supersample) {
        Ok(renderer) => Some(renderer),
        Err(error) => {
            eprintln!("SKIP: no usable GPU adapter ({error})");
            None
        }
    }
}

fn read_golden_input(name: &str) -> Vec<f32> {
    let path = format!("{}/../../tests/golden/inputs/{name}",
                       env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|_| panic!("missing golden input {path}"));
    bytes.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
        .collect()
}

fn noise_segments(count: usize, seed: &mut u64) -> Vec<[f32; 5]> {
    // LCG noise: screen-diagonal jumps, the fill-rate worst case
    let mut random = || {
        *seed = seed.wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((*seed >> 33) as f32 / (1u64 << 31) as f32) - 1.0
    };
    let mut previous = [WIDTH as f32 / 2.0, HEIGHT as f32 / 2.0];
    (0..count).map(|_| {
        let next = [(random() * 0.5 + 0.5) * WIDTH as f32,
                    (random() * 0.5 + 0.5) * HEIGHT as f32];
        let segment = [previous[0], previous[1], next[0], next[1],
                       0.05 + 0.4 * (random() * 0.5 + 0.5)];
        previous = next;
        segment
    }).collect()
}

fn run_stream(frames: &[Vec<[f32; 5]>], supersample: usize)
              -> Option<(Vec<u8>, Vec<u8>)> {
    let mut cpu = CpuRenderer::new(WIDTH, HEIGHT, supersample);
    let mut gpu = gpu_or_skip(WIDTH as u32, HEIGHT as u32,
                              supersample as u32)?;
    assert_eq!(gpu.supersample(), supersample as u32,
               "GPU shed supersample unexpectedly");
    let mut cpu_frame = Vec::new();
    let mut gpu_frame = Vec::new();
    for segments in frames {
        cpu_frame = cpu.render(segments).to_vec();
        gpu.advance(segments);
        gpu_frame = gpu.composite_and_read();
    }
    Some((cpu_frame, gpu_frame))
}

#[test]
fn sweep_stream_matches() {
    let samples = read_golden_input("sweep-48000.f32");
    let mut computer = Computer::new();
    computer.mode = Mode::from_str("xy").unwrap();
    computer.gain = 1.3;
    computer.set_sample_rate(48000, 1);
    let frames: Vec<Vec<[f32; 5]>> = samples.chunks(1600)
        .take(8)
        .map(|chunk| computer.compute(chunk, WIDTH as f32,
                                      HEIGHT as f32).to_vec())
        .collect();
    let Some((cpu_frame, gpu_frame)) = run_stream(&frames, 1) else {
        return;
    };
    let worst = worst_cell_delta(&cpu_frame, &gpu_frame);
    println!("sweep worst cell delta: {worst:.3} (u8)");
    assert!(worst <= CELL_TOLERANCE, "sweep diverged: {worst}");
}

#[test]
fn noise_stream_matches_supersampled() {
    let mut seed = 0x9805F0u64;
    let frames: Vec<Vec<[f32; 5]>> = (0..5)
        .map(|_| noise_segments(2400, &mut seed))
        .collect();
    let Some((cpu_frame, gpu_frame)) = run_stream(&frames, 2) else {
        return;
    };
    let worst = worst_cell_delta(&cpu_frame, &gpu_frame);
    println!("noise ss2 worst cell delta: {worst:.3} (u8)");
    assert!(worst <= CELL_TOLERANCE, "noise ss2 diverged: {worst}");
}

#[test]
fn takens_stream_matches() {
    let mut computer = Computer::new();
    computer.mode = Mode::from_str("xyz_takens").unwrap();
    computer.set_sample_rate(48000, 1);
    let mut frames = Vec::new();
    let mut phase = 0.0f64;
    for _ in 0..12 {
        let chunk: Vec<f32> = (0..1600).map(|_| {
            phase += 220.0 / 48000.0 * std::f64::consts::TAU / 2.0;
            (phase.sin() * 0.6) as f32
        }).collect();
        frames.push(computer.compute(&chunk, WIDTH as f32,
                                     HEIGHT as f32).to_vec());
    }
    let Some((cpu_frame, gpu_frame)) = run_stream(&frames, 1) else {
        return;
    };
    let worst = worst_cell_delta(&cpu_frame, &gpu_frame);
    println!("takens worst cell delta: {worst:.3} (u8)");
    assert!(worst <= CELL_TOLERANCE, "takens diverged: {worst}");
}

#[test]
fn gpu_sharpness_not_softer_than_cpu() {
    // fine high-rate pattern at supersample 2 — where v3's GPU went
    // soft; grid off to isolate the beam
    let samples = read_golden_input("sweep-96000.f32");
    let mut computer = Computer::new();
    computer.mode = Mode::from_str("xy").unwrap();
    computer.gain = 1.3;
    computer.set_sample_rate(96000, 1);
    let frames: Vec<Vec<[f32; 5]>> = samples.chunks(3200)
        .take(10)
        .map(|chunk| computer.compute(chunk, WIDTH as f32,
                                      HEIGHT as f32).to_vec())
        .collect();

    let mut cpu = CpuRenderer::new(WIDTH, HEIGHT, 2);
    cpu.grid_enabled = false;
    let Some(mut gpu) = gpu_or_skip(WIDTH as u32, HEIGHT as u32, 2)
    else {
        return;
    };
    gpu.grid_enabled = false;
    let mut cpu_frame = Vec::new();
    let mut gpu_frame = Vec::new();
    for segments in &frames {
        cpu_frame = cpu.render(segments).to_vec();
        gpu.advance(segments);
        gpu_frame = gpu.composite_and_read();
    }
    let cpu_sharpness = mean_gradient(&cpu_frame, WIDTH, HEIGHT);
    let gpu_sharpness = mean_gradient(&gpu_frame, WIDTH, HEIGHT);
    println!("sharpness (mean gradient): GPU {gpu_sharpness:.4} vs \
              CPU {cpu_sharpness:.4}");
    assert!(gpu_sharpness >= cpu_sharpness * 0.98,
            "GPU is softer than CPU again: {gpu_sharpness:.4} vs \
             {cpu_sharpness:.4}");
}

#[test]
fn allocation_pressure_sheds_supersample() {
    // 2000×2000 at ss8 wants a 16000² energy texture — past the default
    // max_texture_dimension_2d, so allocation must fail, shed, and land
    // healthy at ss1 (the v3 blank-scope bug class, now a contract)
    match GpuRenderer::new_offscreen(2000, 2000, 8) {
        Ok(renderer) => {
            assert!(renderer.shed_supersample,
                    "impossible allocation did not shed");
            assert_eq!(renderer.supersample(), 1);
            println!("shed to supersample 1, energy format {:?}",
                     renderer.energy_format());
        }
        Err(error) => {
            // acceptable only when there is no adapter at all
            eprintln!("SKIP or legitimate failure: {error}");
            assert!(error.contains("no adapter"),
                    "shed path failed outright: {error}");
        }
    }
}
