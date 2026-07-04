// SPDX-License-Identifier: GPL-3.0-or-later
//! CPU rasterizer replacing Cairo stamping: rayon tiles, 8-wide SIMD
//! (AVX2+FMA via runtime dispatch through `wide`; Zen 2 target, no
//! AVX-512). Consumes the same beam model as the GPU path — parity by
//! construction, verified by perceptual-hash snapshot tests.
//!
//! The baseline this exists to bury (BENCH.md): v3 Cairo at max
//! settings = 7 fps fullscreen with ONE core pegged (GIL), 45→22 fps
//! declining at defaults via the skipped-frame backlog spiral. The
//! noise stress signal (screen-diagonal segments) is the fill-rate
//! worst case — budget against it, not against pretty lissajous.

pub mod raster {}
