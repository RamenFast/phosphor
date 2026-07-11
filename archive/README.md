# Archive — utilized artifacts, kept whole

Created 2026-07-10 at Ben's ask ("cleanup dead code / already-utilized
artifacts; extract what's still alive"). Everything here DID its job and is
kept for provenance, not for use. Nothing in this folder compiles, ships, or
binds. Code comments citing these docs by name (V4PLAN, APPLET-PLAN, PARITY)
resolve here.

| artifact | what it was | why it's done |
|---|---|---|
| `PromptV4KeepPlanOffIt.md` | Ben's original v4 rewrite prompt | v4 shipped (4.0.0, 2026-07-05) |
| `V4PLAN.md` | the approved v4 rewrite plan, waves 1–4 | executed through 4.6.2 |
| `APPLET-PLAN.md` | the applet GJS-rewrite plan | applet 2.1.0 live on Ben's panel |
| `PARITY.md` | the v4.0 receipts ledger (waves truth→ensemble) | the release it gated shipped; BUGLOG is the living ledger |
| `SPIKES.md` | wave-1 risk spikes (winit/Muffin, egui+wgpu) | questions answered, laws absorbed into code comments |
| `spike_shell.rs` | the wave-1 shell spike (was phosphor-app example) | superseded by the real shell; kept frozen, no longer compiles |
| `NEXUS-FORM-STATION.md` | station-convention questionnaire | answered in place, wave 3.2 |
| `PHOSPHOR-STATION.html` | station doc page | unregistered in concourse (verified 2026-07-10); station facts live in docs/AGENTS.md |
| `ARC-BRIEF.md` | the audit arc's shared laws | arc CLOSED 2026-07-10; standing laws now in docs/dev/BEST-PRACTICES.md + BUGLOG |
| `BEST-PRACTICES-DRAFT.md` | the GPT's 1388-line rules draft | distilled into docs/dev/BEST-PRACTICES.md (the binding version) |
| `phosphor-studio/` | the studio stub crate (was in the workspace) | audit §5.2: removed from the active workspace; FUTURE.md #1 — the studio returns as a fresh build, not from this stub |
| `phosphor-studio.1.scd` | the v3-era studio man page | studio uninstalled since v4; source material for its return |

Still alive, for the record: `docs/dev/AUDIT.md` and `BLOOM-SPEC.md` (source
docs for the six in-flight plans), `SCOPE-IDEAS.md` (future context), the
audio examples `capture_probe`/`playback_probe`/`thread_probe`/
`symphonia_interleave_probe` (receipt rigs the SPSC + downmix repairs will
run before/after), and every ledger: BUGLOG, BENCH, GOLDEN, FUTURE.
