// SPDX-License-Identifier: GPL-3.0-or-later
//! Formats as contracts. The bytes are the spec:
//!
//! - `.phos` — FIXED 256-byte header (`PHOSC1` + JSON, fit-trim ladder
//!   80/48/24/8/0 chars on title/credit/source, space-padded, newline
//!   terminated), s16le stereo payload at the header rate, decode
//!   contract `f32 = s16 / 32767.0`. Pinned by `tests/golden/phos/`.
//! - `.phoskit` — kit JSON; canonical stage packing [(op, [p0..p3])]
//!   with the OPERATIONS table's defaults/clamps (gated by
//!   `tests/golden/kits/`).
//! - settings — v3's `~/.config/phosphor/settings.json`, read with v3
//!   semantics: unknown keys ignored, missing file = defaults.
//!
//! Nothing in this crate computes; it parses, validates, and writes.
//! Error text stays short and directive — a 7B model must repair its
//! kit in one round-trip (Ben's law).

pub mod phos;
pub mod phoskit;
pub mod settings;
