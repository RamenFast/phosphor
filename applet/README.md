# Phosphor Scope — Cinnamon applet

A live vectorscope in your Cinnamon panel, like a CPU monitor but for sound.
It reuses Phosphor's exact signal path: a small headless helper
(`phosphor_applet_feed.py`) captures your default output's monitor with
`parec`, runs the same `SegmentComputer`, auto-gains the trace, and streams
beam segments to the applet, which paints them with Cairo.

## Install

```bash
applet/install.sh
```

Then right-click your panel → **Applets** (or **Menu → Applets**), find
**Phosphor Scope**, and add it. If you reinstall over a running copy, remove
and re-add it (or reload Cinnamon with `Ctrl+Alt+Esc`) to pick up changes.

## Use

- **In the panel:** a live mini scope. Silence shows a faint resting dot.
- **Hover:** a larger scope pops up, with mode buttons and actions.
- **Click:** toggles that popup.
- **Open Phosphor:** launches the full app.
- **Pin floating preview:** opens the always-on-top mini player (`phosphor --mini`).

## Settings

Right-click the applet → *Configure*:

- **Trace colour** — *Follow panel theme* (blends in like a system monitor) or
  a fixed *Phosphor colour*.
- **Phosphor colour** — which preset to use when not following the theme.
- **Scope mode** — XY, goniometer, XY dots, waveform, spectrum, or radial
  spectrum (also switchable from the hover popup).
- **Width in the panel** — how many pixels wide the scope is.

## How it finds the helper

The installer bundles `phosphor_applet_feed.py` (and the `phosphor_audio` /
`phosphor_signal` modules it imports) into the applet directory, so it is
self-contained. If those aren't present it falls back to a `.deb` install at
`/usr/lib/phosphor/`.
