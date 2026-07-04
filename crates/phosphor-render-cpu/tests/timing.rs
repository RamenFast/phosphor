// SPDX-License-Identifier: GPL-3.0-or-later
//! The CPU rasterizer against BENCH.md's budget workloads. Prints
//! ms/frame — run `cargo test -p phosphor-render-cpu --release
//! -- --nocapture timing` for the real numbers (debug numbers are
//! meaningless and the assert is only a sanity ceiling).

use std::time::Instant;

use phosphor_render_cpu::CpuRenderer;

fn noise_segments(count: usize, width: f32, height: f32, seed: &mut u64)
                  -> Vec<[f32; 5]> {
    let mut random = || {
        *seed = seed.wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (*seed >> 33) as f32 / (1u64 << 31) as f32
    };
    let mut previous = [width / 2.0, height / 2.0];
    (0..count).map(|_| {
        let next = [random() * width, random() * height];
        let segment = [previous[0], previous[1], next[0], next[1],
                       0.05 + 0.4 * random()];
        previous = next;
        segment
    }).collect()
}

fn time_case(width: usize, height: usize, supersample: usize,
             segment_count: usize) {
    let mut renderer = CpuRenderer::new(width, height, supersample);
    let mut seed = 0x9805F0u64;
    let frames: Vec<Vec<[f32; 5]>> = (0..13)
        .map(|_| noise_segments(segment_count, width as f32,
                                height as f32, &mut seed))
        .collect();
    for warmup in &frames[..3] {
        renderer.render(warmup);
    }
    let started = Instant::now();
    for segments in &frames[3..] {
        renderer.render(segments);
    }
    let per_frame = started.elapsed() / (frames.len() - 3) as u32;
    println!("cpu {width}x{height} ss{supersample} {segment_count} noise \
              segments [{}]: {:.2} ms/frame ({:.0} fps-equivalent)",
             renderer.simd_label(),
             per_frame.as_secs_f64() * 1000.0,
             1.0 / per_frame.as_secs_f64());
    // sanity ceiling only; the honest numbers come from --release
    if !cfg!(debug_assertions) {
        assert!(per_frame.as_secs_f64() < 2.0,
                "grotesquely slow for release: {per_frame:?}");
    }
}

#[test]
fn timing_noise_workloads() {
    time_case(1600, 1000, 1, 2400);
    if cfg!(debug_assertions) {
        // the full matrix in a debug build takes minutes and proves
        // nothing; the honest numbers are release-only by design
        return;
    }
    time_case(1600, 1000, 1, 32000);
    // the v3-comparable rows: BENCH.md's Cairo ran native 2560×1440
    // (no supersample) and did 333 ms/frame under noise
    time_case(2560, 1440, 1, 2400);
    time_case(2560, 1440, 1, 32000);
    time_case(2560, 1440, 2, 2400);
    time_case(2560, 1440, 2, 32000);
}
