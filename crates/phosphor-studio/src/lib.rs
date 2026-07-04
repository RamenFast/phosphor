// SPDX-License-Identifier: GPL-3.0-or-later
//! Scene compiler (port of phosphor_studio.py: shape_points →
//! constant-speed traversal → animate → frames; one-engine rule — the
//! compose resampler lives in phosphor-dsp, never a third path) plus
//! the wave-4 timeline tier: `timeline.json` + `studio build` → one
//! flac, beat grid (pure-Rust onset detection, aubio CLI fallback),
//! morphs, wireframe3d through the shared camera, Hershey vector font,
//! multi-stroke retrace blanking, camera automation keyframes.
//!
//! Gates: `tests/studio_golden.json` hashes port over (`--record`
//! re-pins deliberately); `scenes/stress-knot.scene.json` is both a
//! bench workload and a compiler fixture. CLI contract: exit codes
//! 0/2/3/4, `--output json`, errors carry a JSON path.

pub mod scene {}
pub mod timeline {}
