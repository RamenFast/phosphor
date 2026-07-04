// SPDX-License-Identifier: GPL-3.0-or-later
//! The one engine. Port of core/src/lib.rs (Computer / Upsampler / Fft /
//! KitStage) extended to ALL 11 v3 modes: xy, xy45, xy_swirl, xy_dots,
//! xyz_takens, helix, waveform, ring, spectrum, spectrum_radial, tunnel.
//!
//! Ground truth and gates (wave 1 step 3):
//! - `tests/golden/cases/` — Python-reference segments, tolerance
//!   0.05 px coordinates / 5e-3 intensity / exact segment counts.
//! - `tests/golden/native-v3/` — the ONLY oversampling truth (v3's
//!   Python path never oversamples); os2/os4 cases pin the sinc.
//! - `tests/golden/kits/` — raw kit audio. Kit parity law: f64 phase
//!   accumulators advanced per chunk by 2π·hz·frames/rate with
//!   `rem_euclid`, f64 trig cast to f32 BEFORE the f32 sample math,
//!   integer-sample channel delays, state zeroed on reset/configure.
//!
//! Semantics that must survive the port (from the fixture capture):
//! - takens pre-lock: default tau = 0.004 × rate; emit ramps up as
//!   history − 2τ grows.
//! - helix at 48 kHz is history-capped (8192 frames) below its 0.35 s
//!   span — v3 truth, preserved.
//! - silence is processed, not skipped (quiet-gating is the app's job).
//! - swirl phase advances by sample count, never wall clock.

pub mod modes {}
pub mod kit {}
pub mod oversample {}
