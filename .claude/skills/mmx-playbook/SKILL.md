---
name: mmx-playbook
description: Claude Fable — getting real work done with mmx (MiniMax CLI) as your delegation arm. Use when you want a second opinion, a draft pass, a review, research summarization, or media generation WITHOUT spawning another Claude agent — mmx text chat with MiniMax-M3 is the cheap, fast outboard brain. Field-tested patterns, exact invocations, and the pitfalls that actually bit.
---

# Claude Fable — the mmx playbook

The reference manual lives in the `mmx-cli` skill. This is the *operator's*
playbook: how I (Claude Fable 5, working in Ben's phosphor sessions) actually
use mmx to get things done, and what went wrong so it doesn't again. Ben's
standing preference: **delegate to mmx-M3 instead of spawning subagents** —
it conserves Claude usage and M3 is genuinely good at bounded, self-contained
jobs.

## The one command that matters

```bash
mmx text chat --model MiniMax-M3 \
  --message "system:<one paragraph: role, exactly what to return, hard limits>" \
  --message "user:<the complete material — it has NO other context>" \
  --non-interactive --quiet
```

- `--non-interactive --quiet` always in agent use: no prompts, stdout is
  pure answer.
- Pin `--model MiniMax-M3` explicitly; the CLI default is M2.7.
- `--output json | jq -r '.content'` when you need to script over the reply.

## What to delegate (works great)

- **Second-opinion reviews** of a document/README/plan you just wrote.
  Field-tested framing that returned genuinely useful findings:
  - system: "Review ONLY for factual inconsistencies within the text,
    confusing phrasing, ordering problems, bloat. Do NOT rewrite. List at
    most 8 concrete findings as bullets, most important first. If solid,
    say so briefly."
  - Then act on the subset that survives your own judgment (in one real
    run: 8 findings, 4 were right, 2 conflicted with the user's explicit
    wishes, 2 were taste — that ratio is normal and fine).
- **Draft passes** on bounded prose (release notes, descriptions,
  docstrings) you'll rewrite in your own voice.
- **Summarize/extract** over pasted material.
- **Media**: `mmx image generate`, `mmx music generate` (RPM=3 on the free
  music models — don't loop them), `mmx speech synthesize`.

## What NOT to delegate

- Anything needing repo context it can't see. M3 gets exactly what's in
  your `--message` strings — paste the whole artifact, never a path.
- Code changes in a live codebase (it can't run tests; you can).
- Judgment calls the user delegated to *you* specifically.

## Pitfalls that actually bit

- **Shell quoting**: long documents inline in `--message` break on quotes/
  em-dashes. Write the material to a file and pass
  `--message "user:$(cat file)"` — or `--messages-file`.
- **Role prefixes**: without `user:`/`system:` prefixes the whole string is
  one user message; a leaked "system:" line inside content gets parsed as a
  role. Keep the system framing in its own `--message`.
- **Quota**: `mmx quota show` before big batches; exit code 4 = quota,
  10 = content filter (retry with neutral wording).
- Streaming is on by default in a TTY — harmless, but `--quiet` if you're
  capturing stdout.

## Receipt pattern

When the delegated result matters, tell the user what mmx found and which
findings you accepted/rejected and why — the review is advisory, you own
the outcome.
