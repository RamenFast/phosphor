# The golden fixtures — provenance and law

`tests/golden/` is **frozen ground truth**, captured from the live
Python v3 engine before the first line of Rust (wave 1, step 1 — tag
`v3.5.0` is the last commit whose tree contains the generators).

- `inputs/*.f32` — raw sample streams (sweeps, tones).
- `cases/*.json` + `.segments.bin` — per-mode segment outputs at the
  fixture's resolution, python-lineage.
- `native-v3/` — the v3 rust-core's own outputs (API v2 parity set).
- `kits/` — kit-chain outputs incl. the starter kits.
- `phos/`, `renders-v3-reference/` — postcard round-trips and v3
  reference PNGs (reference-only; no test diffs them).

**Consumed by:** `crates/phosphor-dsp/tests/golden_replay.rs` (and the
render crates' snapshot tests read `inputs/`). No Python remains in
this repository — the generators (`capture_golden.py`,
`golden_lib.py`, `tests/bench/*.py` and friends) did their job and
were deleted with the v4.0.0 purge.

**To regenerate** (you almost certainly should not — the point is
that v4 matches v3, and v3 is gone):

```bash
git checkout v3.5.0 -- tests/  # the generators, back from history
```

**Law:** fixtures are append-only. A change in engine behavior that
breaks replay is a bug in the engine, not in the fixture — the one
deliberate exception so far is documented in the test itself
(xy_dots-wide, pinned).
