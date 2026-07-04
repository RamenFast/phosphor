// SPDX-License-Identifier: GPL-3.0-or-later
//! Formats as contracts. The bytes are the spec:
//!
//! - `.phos` — FIXED 256-byte header (`pack_header` fit-trim rules are
//!   pinned by `tests/golden/phos/`, including the 24-char ladder for
//!   title/credit/source), s16le stereo payload at the header rate,
//!   decode contract `f32 = s16 / 32767.0`.
//! - `.phoskit` — kit JSON; canonical stage packing [(op, [p0..p3])]
//!   with the OPERATIONS table's defaults/clamps (gated by
//!   `tests/golden/kits/`).
//! - scene / timeline JSON — the studio's source language.
//! - tap / probe / feed wire types (wave 3).
//!
//! Nothing in this crate computes; it parses, validates, and writes.
//! Error text stays short and directive — a 7B model must repair its
//! kit in one round-trip (Ben's law).

pub mod phos {}
pub mod phoskit {}
pub mod scene {}
