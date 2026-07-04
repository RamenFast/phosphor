// SPDX-License-Identifier: GPL-3.0-or-later
//! The beam model — one definition of what phosphor looks like.
//! Sigma (focus), energy deposit per segment (dwell-time brightness:
//! `min(1, beam_energy / (distance·rate_scale + 0.7))`, age pre-decay
//! `glow_keep^age`), decay layers (flash → glow), tonemap, and the
//! shared 3D orbit camera (yaw/pitch/dolly, fog `clamp(0.9−0.3z)`).
//!
//! Both renderers consume this crate; neither may own beam math.
//! GPU sharpness ≥ CPU is a wave-1 exit criterion: the sharpness bug
//! class (linear-filtered RG16F supersampling + tonemap softness) dies
//! here, with sRGB-correct output and a proper supersample downfilter.
//! Themes ship as data files (v3's 8 scope themes + 8 UI styles port;
//! AMOLED-black stays first-class).

pub mod model {}
pub mod camera {}
pub mod themes {}
