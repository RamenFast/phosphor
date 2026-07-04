// SPDX-License-Identifier: GPL-3.0-or-later
//! Property tests for the invariants prose can't pin: kit phase
//! wraparound under arbitrary streaming, and oversampler output
//! density (the spirit of v3's test_native_parity checks).

use proptest::prelude::*;

use phosphor_dsp::{Computer, KitOp, Mode};

proptest! {
    /// The rotate phase accumulator must stay in [0, τ) through any
    /// chunking and any hz sign — rem_euclid semantics, matching
    /// Python's `%` for floats.
    #[test]
    fn kit_phase_stays_wrapped(
        hz in -4.0f64..4.0,
        chunk_frames in proptest::collection::vec(1usize..600, 1..24),
    ) {
        let mut computer = Computer::new();
        computer.mode = Mode::Xy;
        computer.set_kit(&[(KitOp::Rotate, [hz, 0.0, 0.0, 0.0])]);
        for frames in chunk_frames {
            let samples = vec![0.25f32; frames * 2];
            computer.compute(&samples, 800.0, 600.0);
            let phase = computer.kit_phase_for_test(0);
            prop_assert!((0.0..std::f64::consts::TAU).contains(&phase),
                         "phase escaped: {phase}");
        }
    }

    /// Streamed oversampling emits factor× the frames minus only the
    /// fixed sinc latency at the head — density holds over any chunking.
    #[test]
    fn oversampler_density(
        factor in prop::sample::select(vec![2usize, 4]),
        chunk_frames in proptest::collection::vec(64usize..900, 2..12),
    ) {
        let mut computer = Computer::new();
        computer.mode = Mode::Xy;
        computer.set_sample_rate(48000, factor as u32);
        let mut base_frames = 0usize;
        let mut total_segments = 0usize;
        for (index, frames) in chunk_frames.iter().enumerate() {
            let samples: Vec<f32> = (0..frames * 2)
                .map(|i| ((index * 31 + i) as f32 * 0.013).sin() * 0.5)
                .collect();
            base_frames += frames;
            total_segments +=
                computer.compute(&samples, 800.0, 600.0).len();
        }
        // ~factor× the segments of the plain feed (minus sinc latency)
        prop_assert!(total_segments + 8 * factor >= (base_frames - 16) * factor,
                     "too sparse: {total_segments} vs {base_frames}×{factor}");
        prop_assert!(total_segments <= base_frames * factor,
                     "denser than the feed allows: {total_segments}");
    }
}
