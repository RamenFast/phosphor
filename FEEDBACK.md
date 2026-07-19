# FEEDBACK.md — Phosphor session feedback ledger

<!-- DO NOT DELETE. Ben's permission is required to delete this file. -->
<!-- Scope: THIS repo only (Rust CRT oscilloscope (station node): probe/ctl/tap CLI, themes, packaging). Other projects carry their own
     FEEDBACK.md. -->

Every atomic claim, ask, correction, bugfix, feature request, and UI change Ben
gives in a session gets one line here, appended by the working agent as SOP.

Rules:
- One line per atomic item: `- YYYY-MM-DD [kind] description` where kind is one
  of `ask` · `correction` · `bugfix` · `feature` · `ui` · `claim`.
- On a duplicate/repeat: do NOT add a new line. Keep the existing line, tighten
  its wording if needed, and prefix a counter: `- 2x YYYY-MM-DD ...` (bump the
  counter, update the date to the latest occurrence). Repeats matter — they
  show what keeps breaking.
- Newest entries at the bottom of the ledger.
- No autonomous action is to be taken from this document. Ben mines it and
  updates AGENTS.md himself.

## Ledger

- 2026-07-18 [feature] Add a `phosphor ctl gain` verb so the Android relay companion can control desktop scope gain.
- 2026-07-18 [ask] Support `phosphor ctl gain 1.5`, clamping numeric gain to 0.1..6.0.
- 2026-07-18 [ask] Support `phosphor ctl gain auto` to enable auto-gain.
- 2026-07-18 [ask] Put both gain forms in `CtlRequest::EXAMPLES` so BUGLOG #16's real-socket round-trip test covers them.
- 2026-07-18 [correction] Keep ctl error `fix` strings executable positional syntax, never unsupported flag syntax.
- 2026-07-18 [ask] Numeric ctl gain must mirror GUI leave-auto semantics: store the clamped gain, disable auto-gain, and snap effective/computer gain immediately.
- 2026-07-18 [ask] Auto ctl gain must mirror GUI enter-auto semantics: enable auto-gain and reset the tracked auto-gain peak.
- 2026-07-18 [correction] Publish ctl gain through the existing `computer.gain` and `effective_gain` flow required by BUGLOG #13; do not add a channel.
- 2026-07-18 [ask] Persist ctl gain changes through `UiAction::SaveSettings` and its atomic-save path.
- 2026-07-18 [bugfix] A ctl gain change must trigger Theme's restyle repaint mechanism so an idle or paused scope redraws under BUGLOG #14.
- 2026-07-18 [ask] Bump the workspace version from 4.7.0 to 4.7.1.
- 2026-07-18 [ask] Gate the gain verb with workspace clippy at `-D warnings`, the full workspace test suite, and the specific ctl real-socket round-trip test.
- 2026-07-18 [correction] Do not touch the running scope's real ctl socket; use only an isolated instance or the test harness.
- 2026-07-18 [correction] Leave the implementation uncommitted and unpushed for orchestrator review.
- 2026-07-18 [ask] Write the gain-verb implementation report, including line refs and verbatim gate outputs, to the requested scratchpad path.
