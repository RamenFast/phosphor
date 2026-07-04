// SPDX-License-Identifier: GPL-3.0-or-later
//! wgpu renderer: the three v3 passes (decay, beam, composite) as WGSL
//! plus the bloom chain; f16 energy buffers when `shaderFloat16` (RADV
//! reports true on Ben's 6750 XT); offscreen mode for headless render
//! and `phosphor bench`.
//!
//! Non-negotiables carried from v3 and the baseline:
//! - Mailbox present + an owned frame loop. BENCH.md's core finding:
//!   v3 GL sags to 90–104 fps at partial GPU load (amdgpu DPM suspect)
//!   — v4 must hold ≥157 fps at EVERY load level, not just heavy ones.
//! - Energy-buffer allocation failure: shed supersample, never draw
//!   into a broken target — port v3's logic onto wgpu error scopes.
//! - No fallback paths. An error surfaces; nothing silently degrades.

pub mod passes {}
