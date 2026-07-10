# ARC BRIEF — audit-repair + Recurrence Bloom arc (2026-07-10)

**STATUS: ARC CLOSED 2026-07-10.** HANDOFF.md carries the outcome and the
in-flight queue; this file is the historical brief. One correction learned at
close: gate law 5's `cargo fmt` line was never real in this repo (see
BEST-PRACTICES.md §1) — the binding gate is clippy -D warnings + full tests.

Shared laws for every agent working this arc. Read this WHOLE file before touching anything.

## The arc

Ben asked for: (1) full investigation + repair of the `phosphor_codebase_audit.md` findings,
(2) a complete plan for a new scope mode "Recurrence Bloom" (`recurrence_bloom_scope_spec.md`)
with a companion theme, (3) skill updates (claude + jcode harnesses), (4) HANDOFF.md and
README.md refreshed. Baseline: v4.6.2, commit fcd3a12, all suites green.

Input docs (were repo root; moved home at arc close — the docs-final move):
- `docs/dev/AUDIT.md` — the GPT's static audit (it could NOT build; we can)
- `docs/dev/BLOOM-SPEC.md` — the new scope's spec
- `docs/dev/SCOPE-IDEAS.md` — five more scope concepts (context, not tasks)
- `docs/dev/BEST-PRACTICES-DRAFT.md` — draft dev rules doc, to be critiqued/distilled

## Standing repo laws (violations have shipped bugs before)

1. **Read `docs/dev/BUGLOG.md` before touching UI/input/menu/playlist/window code.**
   Fix a bug → append the entry (symptom · root cause · law · receipt), cite `BUGLOG #N` in code.
2. **Receipts exercise the user's ACTUAL gesture.** "The menu opened" is not a receipt;
   clicking the item is. A fix without a receipt is not fixed.
3. **UI receipts run at 2560×1440** (standing law in BUGLOG).
4. **Ben's live scope owns the default ctl.sock.** NEVER `ctl quit`/`target` against the
   default socket. Isolate test instances: own `XDG_RUNTIME_DIR` (short /tmp path, SUN_LEN
   ~108 bytes) + `PIPEWIRE_RUNTIME_DIR=/run/user/1000` + `PULSE_SERVER=unix:/run/user/1000/pulse/native`,
   or `PHOSPHOR_NO_SINGLE_INSTANCE=1` / `--background`.
5. **Gates before every commit:** `cargo fmt --all -- --check`,
   `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
   Goldens: an intentional golden change needs a migration note in the commit explaining
   whether physics, normalization, or raster precision changed.
6. **Commit as you go**, one commit per coherent repair, repo style: `fix(scope): story` /
   `feat(scope): story` — story-first, lowercase, like the existing log. Commit ONLY your
   own files (`git add <paths>`, never `-A`) — other agents share this tree.
7. **No release, no tag, no version bump.** Ben decides releases. Note release-worthiness
   in your report instead.
8. **Time is seconds, not frames.** Any new decay/animation/timeout states its clock.
9. **One fact, one representation.** No new duplicated schemas/lists/constants.
10. **Don't break the applet feed contract** (frozen v3 protocol, the one no-envelope surface).

## Tree-sharing protocol

Multiple agents work this repo in sequence (implementation nodes are serialized by the
task graph). Before starting: `git status --short` — if you see uncommitted changes that
are not yours, DO NOT revert/commit them; work around them and mention it in your report.
Leave the tree clean of YOUR changes (committed) when you finish. Untracked root docs
(the four inputs + this brief) stay untracked until the docs node organizes them.

## Report contract

Every node's `complete_node` artifact carries: findings, evidence as `file:line` or
commit hashes, validation (exact commands + results), open_questions, honest confidence,
what_i_did_not_check. Downstream nodes are hydrated with your artifact — write for them.
