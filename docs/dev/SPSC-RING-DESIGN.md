# SPSC Block-Ring Transport Design (va-design-spsc)

Design doc only — no code changes. Replaces the `Arc<Mutex<SampleRing>>` /
mix-member `Arc<Mutex<Vec<f32>>>` capture transport and the
`Mutex<VecDeque<f32>>+Condvar` playback `AudibleRing` with lock-free SPSC
block rings, **without changing the `AudioEngine` facade API** the shell
consumes (`take_stereo_samples` engine.rs:381, `pending_scope_samples`
engine.rs:387, `copy_history` engine.rs:428, `start_capture(_mix)`,
`set_volume` engine.rs:356, etc.).

## 0. Problem recap (from upstream verification)

- **Capture** `.process` closure (engine.rs:1308-1345) runs on the
  `phosphor-audio-pw` main-loop thread (connect flags at engine.rs:1371-1376
  lack `RT_PROCESS`). It locks the scope ring (engine.rs:1323) or a mix-member
  `Mutex<Vec<f32>>` (engine.rs:1328-1340), with reserve/convert/drain inside
  the lock (ring.rs:58-69, ring.rs:74-83). Contended by the shell in
  `take_stereo_samples` (engine.rs:383) and `fold_mix_into_ring`
  (engine.rs:404-424). Hazard class: main-loop stalls → late buffer recycling
  → capture drops. Not hard-RT, but worth fixing.
- **Playback** `.process` closure (engine.rs:1430-1470) runs on the PW graph
  RT data thread (`RT_PROCESS`, engine.rs:1496-1507). It calls
  `AudibleRing::pop_into` (playback.rs:96-110): mutex lock, per-sample
  `pop_front().unwrap()`, `Condvar::notify_one` (futex syscall) — the mutex is
  also held by the decode thread's `push_blocking` (playback.rs:72-92) under a
  `wait_timeout` loop. This is the genuine RT violation.

## 1. Core primitive: `BlockRing`

One new module `crates/phosphor-audio/src/spsc.rs`:

```rust
/// Fixed-capacity SPSC ring of f32 samples. Power-of-two capacity in
/// samples (interleaved stereo, always frame-aligned writes/reads).
pub struct BlockRing {
    buf: Box<[UnsafeCell<f32>]>,      // capacity = 2^k samples
    write_pos: CachePadded<AtomicUsize>, // monotonically increasing sample index
    read_pos:  CachePadded<AtomicUsize>,
    closed:    AtomicBool,
}
pub struct Producer(Arc<BlockRing>);  // !Clone — single producer
pub struct Consumer(Arc<BlockRing>);  // !Clone — single consumer
```

- **Indices are monotonic** (wrap by masking with `cap-1`), so
  `write_pos - read_pos` is the fill level without ambiguity; no separate
  "full" flag. Ordering: producer publishes with `write_pos.store(Release)`
  after copying; consumer `read_pos.store(Release)` after copying;
  each side `load(Acquire)` the other's index. Standard SPSC proof.
- **No allocation, no locks, no syscalls** in `push`/`pop`. Two-segment
  `copy_from_slice` handles wraparound (no per-sample loop).
- Small (~120 lines), dependency-free (hand-rolled rather than pulling in
  `rtrb`/`ringbuf`; keeps GPL tree dep-light and testable under loom — but
  adopting `rtrb` is an acceptable variant, see Risks §8.5).

### Block vs sample granularity

**Frame granularity with block-sized transfers.** The ring stores raw f32
samples (not boxed blocks): PW quanta vary (typically 256–2048 frames), and a
sample-indexed ring with slice copies gives block-copy performance without
requiring quantum == block size. Writes and reads are always rounded down to
whole stereo frames (FRAME=2, ring.rs:14 law preserved). A boxed-block ring
(Vec<Box<[f32]>>) would force an allocation/recycle pool for no benefit here.

### Sizing

- **Capture scope ring**: capacity = next_pow2(sample_rate * 2ch *
  PENDING_BACKLOG_SECONDS) → 131072 samples @48k (~1.37 s). Preserves the 1 s
  backlog law (ring.rs:10) as the *overrun horizon* instead of an amortized
  trim.
- **Mix member rings**: same sizing as the scope ring (each member is a full
  stereo feed folded shell-side).
- **Playback audible ring**: capacity = next_pow2(current AudibleRing cap;
  keep the existing constant `AUDIBLE_RING_SECONDS = 0.1` playback.rs:41, cap
  ≈ 0.1 s * rate * 2ch, floor BLOCK_FRAMES*2, playback.rs:59-62) — 9600
  samples @48k → next_pow2 = 16384 (~0.17 s). NOTE: an earlier draft of this
  doc wrongly said 1 s / 131072; the real depth is **0.1 s**. The Condvar
  backpressure is replaced by decode-thread parking (see §4).
- Rate changes (`configure_sample_rate` engine.rs:233): allocate a **new** ring
  pair on the engine loop thread and swap; never resize in place.

## 1.5 Scope-ring producer topology (va-gap-scope-ring-producers)

The scope `Arc<Mutex<SampleRing>>` (engine.rs `self.ring`) has **three**
producer classes today:

1. **Capture** `.process` closure on the pw main-loop thread
   (engine.rs:1322-1323, via `CaptureDestination::ScopeRing`).
2. **Player decode thread** via `push_chunk` →
   `scope_ring.lock().unwrap().push_interleaved(chunk)` (playback.rs:735;
   ring handed over at `spawn_player`, engine.rs:262 / playback.rs:533).
3. **Shell thread** via `fold_mix_into_ring` (engine.rs:404-424), which
   pushes summed mix-member audio into the same ring on
   `take_stereo_samples` (engine.rs:381-383).

**(a) Can capture and playback produce concurrently? YES, transiently.**
There is no engine-level mutual exclusion: `StartCapture` and playback
setup are independent commands, and the UI enforces exclusivity only by
convention, with holes:

- Starting playback stops capture first (player.rs:193 `stop_capture()`
  before `start_file`), and unpausing playback stops capture
  (shell.rs:1044-1050 "double-feed law"). But `stop_capture` is an async
  command send; the pw loop may run one more capture `.process` after the
  player thread has already started pushing. Brief true concurrency.
- Picking a capture target while a track plays only **pauses** playback
  (shell.rs:856-861), it does not stop the player thread. A paused audible
  player is frozen by PW backpressure (stream inactive → `push_blocking`
  blocks), so it stops producing, but a **vacuum** player pauses via a
  command the decode loop polls (playback.rs:596-611) — until it polls,
  it keeps pushing. So capture + playback producers can overlap for real,
  short windows. The `Mutex<SampleRing>` makes this safe today (just
  interleaved garbage on screen momentarily); an SPSC ring makes it UB.

**(b) Clearing on mode switch: yes.** `Command::StopCapture` clears pending
(engine.rs:763-764), `stop_playback` clears pending after joining the player
thread (engine.rs:292, history kept — clip-after-stop law). No ring swap;
the same ring object lives for the engine's lifetime (except the
`configure_sample_rate` path).

**(c) Consequence for the SPSC design: keep one logical scope stream but
serialize producers with an explicit handoff, not MPSC.** Options ranked:

- **Chosen: producer-handoff token.** A single `Producer` handle wrapped in
  a small `ProducerSlot` owned by the engine loop thread. `StartCapture`
  claims it (installing it in the capture closure); `spawn_player` requests
  it via a command that first tears down / fences the capture stream **and
  waits for an ack** before the player thread receives the Producer. The
  ack closes the today's race window (a strict improvement). The shell-side
  mix fold (producer 3) disappears in this design anyway (§3: fold feeds
  pending/history directly on the consumer thread), leaving exactly two
  possible producers, never concurrent after the fence.
- Rejected: MPSC ring — pays CAS costs on the RT-adjacent path forever to
  serve a transition that happens at human timescale.
- Rejected: ring-per-mode with consumer-side select — doubles memory,
  complicates history/pending laws, and the consumer must still arbitrate
  ordering during the overlap window.

Migration note (§6 step 3): the handoff ack rides the existing pipewire
command channel + a oneshot, mirroring the `CreatePlayback` pattern.

## 2. Capture path redesign

### Producer contract (pw main-loop callback, engine.rs:1308-1345)

- Convert LE bytes → f32 into a small persistent scratch `Vec` (loop-thread
  local, amortized like the playback scratch), then one `producer.push(&scratch)`.
  Or better: two-segment write directly into the ring
  (`producer.write_slots(n) -> (&mut [f32], &mut [f32])`) doing
  `from_le_bytes` conversion in place — zero intermediate copy.
- Never blocks. On overrun (free < n): **drop oldest** — producer advances
  `read_pos` is NOT allowed in SPSC; instead the producer drops the *newest
  overflow tail* OR (preferred, matches "stalled UI never replays the past",
  ring.rs:3-5) the producer pushes what fits and increments an
  `overrun_frames: AtomicU64` counter; the consumer, seeing fill == cap on its
  next take, is by construction reading data at most cap-seconds old. To keep
  the exact "newest survives" law, use the **producer-side skip-ahead
  variant**: since the true consumer only reads on the shell tick, we instead
  make the *consumer* discard down to 1 s of backlog before reading
  (`consumer.skip_to_latest(max_pending)`), which is SPSC-legal. Net policy:
  ring cap ≥ pending law cap, producer drops only when the shell is fully
  stalled > ~1.37 s, and the consumer trims to 1 s on read. Existing test
  `pending_caps_at_one_second_amortized` (ring.rs:132) semantics preserved:
  newest data survives.

### Consumer contract (shell thread via engine facade)

- `AudioEngine::take_stereo_samples` (engine.rs:381-386) keeps its signature.
  Internally: `consumer.skip_to_latest(max_pending)`, then pop all available
  whole frames into a returned `Vec<f32>` (shell-side alloc is fine, matches
  today's `take_stereo_samples` ring.rs:86-97).
- **History moves consumer-side**: `SampleRing` splits. The SPSC ring carries
  only *pending*; the 10 s `history` buffer (ring.rs:11, CLIP_SECONDS) is
  appended by the consumer after each take (shell thread), feeding
  `copy_history` (engine.rs:428) unchanged. This removes history maintenance
  from the callback entirely. Subtle change: history now only advances when
  the shell drains — acceptable because take runs every frame tick, and if the
  shell stalls > 1 s the pending law already drops audio. Document in code.
- `pending_scope_samples` (engine.rs:387) = `write_pos - read_pos` snapshot,
  lock-free.

### Wakeups

None needed: the shell polls per render frame today (pull model). No condvar,
no eventfd. Keep it.

## 3. Mix members (engine.rs:1328-1340, fold at 404-424)

- `StartMix` creates one `BlockRing` **per member stream**; each capture
  stream's callback owns exactly one `Producer` (moved into the closure — the
  type system enforces SPSC), the engine facade holds the matching `Consumer`s
  in `mix_members: Vec<MixMember { consumer: Consumer, .. }>` (no Mutex around
  the sample data; the Vec itself stays behind the existing engine-state lock,
  touched only at add/remove).
- `fold_mix_into_ring` (engine.rs:404-424) stays shell-side: for each member,
  `skip_to_latest`, pop up to the common available frame count, sum into a
  reused scratch, then append to the scope pending path (which after this
  refactor is just a shell-side Vec — the fold no longer needs the SPSC ring
  at all, it can feed pending/history directly since fold already runs on the
  consumer thread).
- **Add/remove while running**: a new member's Producer is created on the
  engine loop thread (where streams are built, engine.rs:1250) and its
  Consumer sent back to the facade over the existing command/ack channel.
  Removal: facade drops the Consumer and marks the member; the stream is torn
  down on the loop thread; the Producer detects `closed` (or simply gets
  dropped with the stream) — Arc keeps the buffer alive until both halves
  drop, so no use-after-free window. Member lists change only under the
  existing engine mutex, never in a process callback.

## 4. Playback path redesign (the real RT fix)

Replace `AudibleRing` (playback.rs:51-64) internals, keep its API surface
(`push_blocking`, `pop_into`, `close`) so playback.rs call sites don't churn:

- **Consumer = RT callback** (engine.rs:1457 `ring.pop_into`): lock-free
  two-segment copy of `frames*2` samples; on underrun, zero-fill the remainder
  (preserves `audible_ring_backpressure_and_pop` test law, playback.rs:768).
  **No notify from RT thread** — the futex `notify_one` (playback.rs:104) goes
  away.
- **Producer = decode thread** (`push_blocking` playback.rs:72-92): when the
  ring lacks space, spin-then-`thread::park_timeout(1-2ms)` loop, re-checking
  free space and `closed`. Wakeup latency for the decoder is bounded by the
  park timeout, which is fine: at 48 kHz the RT side frees a quantum's worth
  (~5-20 ms of audio) per cycle, and the decoder runs up to ~100 ms ahead
  (the 0.1 s ring depth is the backpressure clock, playback.rs:41).
  `close()` sets the flag + `unpark`s the parked decode thread (store the
  decode thread handle at ring creation) — preserves
  `audible_ring_close_unblocks_push` (playback.rs:783).
- Keep untouched (v4.6.2 house style, engine.rs:1403-1422, 1433-1434):
  atomic gain load once per cycle, per-sample multiply, `state.restore-props
  = false`, restore-target NOT set, node.latency pin, persistent scratch.
  Additional cleanups in the same pass: pre-size scratch to max quantum at
  stream build to avoid `scratch.resize` growth (engine.rs:1456), and gate the
  `PHOSPHOR_AUDIO_LOG` eprintln (engine.rs:1437-1442) to a relaxed atomic
  counter drained off-thread (or accept it as opt-in debug).

## 5. Drop-in points (file:line anchors)

| Site | Today | After |
|---|---|---|
| engine.rs:1323 | `ring.lock().unwrap().push_interleaved_le_bytes(data)` | `producer.push_le_bytes(data)` |
| engine.rs:1328-1340 | member `Mutex<Vec>` lock + reserve/push/drain | `member_producer.push_le_bytes(data)` |
| engine.rs:383 | `ring.lock().take_stereo_samples()` | `scope.take()` (consumer skip+pop, history append) |
| engine.rs:387-ish | pending via lock | atomic fill-level read |
| engine.rs:404-424 | fold locks members + ring | fold pops member Consumers, sums, appends shell-side |
| engine.rs:428 | `copy_history` via ring lock | reads shell-owned history Vec (no lock or a facade-local RefCell/Mutex uncontended) |
| engine.rs:233-245 | configure_sample_rate relocks | swap ring pair on loop thread |
| playback.rs:52-64 | `Mutex<VecDeque>+Condvar` | `BlockRing` + park/unpark |
| playback.rs:96-110 | pop_into per-sample under lock | two-segment lock-free copy + zero-pad |
| playback.rs:72-92 | push_blocking condvar wait | push + park_timeout loop |
| ring.rs | SampleRing (pending+history) | pending → SPSC; history → consumer-side `HistoryBuf` keeping ring.rs:74-83 amortized trim for the Vec it retains |

## 6. Migration plan (tests stay green throughout)

1. **Add `spsc.rs`** with `BlockRing` + unit tests: single-thread laws,
   wraparound at capacity boundary, frame alignment, plus a 2-thread stress
   test (producer floods random-sized chunks, consumer verifies a monotone
   counter sequence) and optionally `loom` behind a cfg. No behavior change.
2. **Playback swap** (highest value, smallest blast radius): reimplement
   `AudibleRing` on `BlockRing`, keep API. Existing 4 playback tests
   (playback.rs:765-821) must pass unmodified. Validate live with
   `examples/playback_probe.rs` (16 laws) + `audible_soak.rs`.
3. **Capture scope ring swap**: split SampleRing into `ScopePending` (SPSC) +
   `HistoryBuf` (consumer-side). Port the 5 ring.rs tests to the new pair —
   same laws, same assertions, driven through producer/consumer handles on one
   thread. Validate live with `capture_probe.rs` (RMS+freq receipt).
4. **Mix member swap**: per-member rings + shell-side fold. Add the missing
   unit test for fold (pure: N consumers pre-filled, assert summed output),
   validate live with `mix_probe.rs` (two-tone law) and `tests/vacuum/gate.sh`.
5. Each step is one commit; `cargo test -p phosphor-audio` (23 tests, growing)
   green at every commit; PARITY.md receipts re-run per step.

## 7. New tests required (gap closure from va-tests-inventory)

- Concurrent push/take stress on `BlockRing` (none exists today).
- Wraparound-at-boundary interleaved push/take.
- Underrun zero-pad and overrun newest-survives at exact-capacity edges.
- Fold-mix pure unit test (engine mix path currently has zero unit tests).
- Optional: `loom` model of producer/consumer index protocol.

## 8. Risks

1. **Wakeup latency (decoder)**: park_timeout polling replaces condvar
   precision; worst case adds ≤2 ms to decoder resume. Mitigated by the
   ~0.1 s audible ring depth (still ≥5 quanta of slack); the RT side never
   waits.
2. **Resampling interaction**: `ScopeResampler::push` drain+collect allocs
   (playback.rs:463-493) stay decode-thread-side and out of scope, but if
   quantum sizes beat against BLOCK_FRAMES the audible ring fill becomes
   burstier — the 0.1 s depth (~5x a 1024-frame quantum) absorbs it. Do not
   shrink ring capacity below the current constant.
3. **Member add/remove while running**: the closure-owned Producer must never
   be cloned; enforce via `!Clone` and moving into the stream closure. Teardown
   ordering (stream destroyed before Consumer dropped or vice versa) is safe
   under Arc, but a *reused* member slot must always get a *fresh* ring —
   never recycle a ring across streams (stale read_pos ghosts).
4. **History semantics shift** (§2): history advances only on shell take; a
   stalled shell means history misses the stall window. Today the callback
   kept history fresh regardless. If clip-export-during-UI-stall matters,
   variant: a second SPSC ring for history drained by the same consumer —
   decide during step 3; document either way.
5. **Hand-rolled unsafe**: the two-segment write path uses `UnsafeCell`.
   Alternative: `rtrb` crate (mature SPSC, chunk API). Trade: dependency vs
   audited unsafe. Recommend hand-rolled + loom given the repo's dep-light
   style, but this is reversible.
6. **pw dispatch assumption**: capture-callback thread identity was verified
   statically, not at runtime. Step 0 of implementation: add a one-shot
   thread-name eprintln probe behind PHOSPHOR_AUDIO_LOG to confirm before/after.
7. **configure_sample_rate mid-capture**: ring swap must be atomic w.r.t. the
   callback — swap the Producer inside a loop-thread command handler (same
   thread as the callback, so trivially safe), never from the shell.
