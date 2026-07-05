# Phosphor Scope — the panel applet

A tiny live vectorscope in your **Cinnamon** panel: the beam in
miniature, fed by the real engine. Hover for a bigger scope, click
for modes and themes, flip the CRT power switch when you want the
panel dark.

```
applet/install.sh     # copy into ~/.local/share/cinnamon/applets
```

then add **"Phosphor Scope"** from Menu → Applets (or restart
Cinnamon with Alt-F2 `r` if it was already added).

## How it works — one engine, zero bundled code

The applet contains **no signal engine at all**. It spawns

```
phosphor feed
```

and draws the beam-segment stream that arrives on stdout (NDJSON,
the locked v3 protocol). All the math — capture, oversampling, kit
chains — happens in the same Rust engine the main app uses, so the
panel and the window can never disagree. If the capture dies (device
unplugged, stream ends), the feed re-resolves the default output
once a second and reconnects; the applet just keeps drawing.

Requires `phosphor` ≥ 4.0 on the PATH (the .deb/.rpm put it there).

## Desktop support, honestly

| Desktop | Status |
|---|---|
| Cinnamon 6.0 – 6.6 | ✅ this applet |
| GNOME / KDE / anything else | ❌ the applet uses Cinnamon's applet API (`imports.ui.applet`, St) and cannot load there. **The main `phosphor` app runs fine on any desktop** — this folder is only the panel widget. |

The main app needs no applet and the applet is optional sugar; they
share nothing but the `phosphor` binary.

## Files

- `phosphor-scope@phosphor/applet.js` — the drawing + menus (GJS)
- `phosphor-scope@phosphor/metadata.json` — Cinnamon manifest
- `phosphor-scope@phosphor/settings-schema.json` — refresh rate,
  size, theme, power
- `install.sh` — user-level install/refresh
