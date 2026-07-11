// SPDX-License-Identifier: GPL-3.0-or-later
//! STATUS: EXPERIMENTAL STUB — no code yet. The studio tier returns
//! after 4.0 (issue #1); `phosphor studio` exits 2 with a pointer to
//! the roadmap until then. The doc below is the contract it ports.
//!
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

/// The stub contract: this crate ships empty modules and nothing else.
/// If real code lands, this test is the reminder to remove the
/// EXPERIMENTAL STUB status header and `publish = false`.
#[cfg(test)]
mod tests {
    #[test]
    fn stub_contract_lib_has_no_public_items_yet() {
        let source = include_str!("lib.rs");
        assert!(
            source.contains("EXPERIMENTAL STUB"),
            "status header must survive until studio returns"
        );
        let placeholder_modules = source
            .lines()
            .filter(|line| line.starts_with("pub mod "))
            .count();
        assert_eq!(
            placeholder_modules, 2,
            "still just the two empty placeholder modules"
        );
    }
}
