# Wave-1 spikes — the risks V4PLAN named, measured (2026-07-04)

All three step-7 spikes ran on the real desktop (X11, Muffin, RADV,
2560×1440@165). Code: `crates/phosphor-app/examples/spike_shell.rs`
(`cargo run --example spike_shell -p phosphor-app -- [--fullscreen]
[--seconds N]`). Debug builds — the numbers below carry that handicap
and still land the argument.

## 1. Present pacing: the stack does ~6× the display rate

`PresentMode` support on this surface: Immediate, **Mailbox**, Fifo,
FifoRelaxed. With Mailbox + continuous redraw + a full egui pass:

| mode | sustained fps |
|---|---|
| fullscreen 2560×1440 | ~950 |
| windowed 900×600 | ~1010 |

v3's live ceiling was 163 fps at the same resolution with the GTK3
frame clock in charge (see BENCH.md: >165 was architecturally
impossible, `vblank_mode=0` included). The wgpu event loop with
mailbox present clears that ceiling by ~6× in a debug build with the
GPU still mostly loafing. The V4PLAN pacing bet is proven; what
remains is keeping that margin once the beam passes replace the clear.

## 2. Transparency: glass mode is viable in Vulkan under Muffin

Surface alpha modes reported: **PreMultiplied**, Inherit — not the
feared Opaque-only. A 35 %-alpha pane composited correctly: the
desktop (browser text and all) reads through the tint, verified by
root-capture pixel inspection while the spike ran. Glass mode ports to
wgpu without the GTK RGBA-visual dance; the v3 recipe's *verification
habit* (pixel-sample a root capture, never trust a flag) carries over.

## 3. Drag-and-drop: events wired

`WindowEvent::HoveredFile` / `DroppedFile` handlers compile and log on
winit 0.30's X11 XDND path. Live-drop receipt pending a human hand on
the real desktop (the handlers print `hover:`/`drop:` lines and list
drops in the spike UI) — wave 2's playlist DnD carries the actual
acceptance test.

## Version pins that matter

egui 0.33 ↔ egui-wgpu 0.33 ↔ **wgpu 27** ↔ winit 0.30. The egui glue
crates define the set; V4PLAN said "wgpu v28 era" but egui-wgpu 0.33
links 27 — bump the whole quartet together or not at all
(workspace `Cargo.toml` carries the comment).

## What step 4 (renderers) inherits

- Choose present mode from capabilities (Mailbox first, Immediate
  fallback) and alpha mode PreMultiplied-first — exactly as the spike
  does.
- `desired_maximum_frame_latency: 2` behaved; revisit under the beam
  load.
- egui-wgpu's `RenderPass::forget_lifetime()` pattern is required to
  mix egui into our own encoder pass — the scope canvas as a raw pass
  in the same surface (V4PLAN) slots into this shape naturally.
