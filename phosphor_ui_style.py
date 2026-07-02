# SPDX-License-Identifier: GPL-3.0-or-later
"""UI chrome styles for Phosphor.

Six switchable looks ("system" adds nothing, "dark" only flips the GTK dark
preference):

  black — AMOLED pink/gold: pure black, flat, gold means active.
  bloom — the AMOLED look with defined, softly glowing controls.
  stone — Stonework 95: hard edges, chiseled bevels, granite and parchment.
  aero  — Aero glass: translucent Frutiger-era gradients and gloss.
  light — Ice Blue ❄: frost palette; the file chooser forces symbolic
          icons so nothing can vanish against the pale chrome.

Every style covers the same selector set, so switching never leaves a
widget half-themed.
"""

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk  # noqa: E402

UI_STYLE_CHOICES = (("system", "System"), ("dark", "Dark"),
                    ("light", "Ice Blue ❄"), ("black", "AMOLED pink"),
                    ("bloom", "Bloom · neon"), ("stone", "Stonework 95"),
                    ("stonebloom", "Stonework · bloom"),
                    ("aero", "Aero glass"))
DARK_STYLES = ("dark", "black", "bloom", "stone", "stonebloom")

# Always loaded: the overlay chips that float on the scope.
BASE_UI_CSS = b"""
#fps-overlay {
    background-color: rgba(0, 0, 0, 0.55);
    color: #7dff9e;
    padding: 2px 8px;
    border-radius: 6px;
    font-family: monospace;
    font-size: 11px;
}
#now-playing {
    background-color: rgba(0, 0, 0, 0.6);
    color: #e8fff0;
    padding: 7px 14px;
    border-radius: 9px;
    font-size: 13px;
}
/* glass scope: the window opens a smoked pane so the desktop shows
   through the scope; each style tints its own pane */
window.glass-scope decoration {
    background-color: transparent;
    background-image: none;
}
window.glass-scope {
    background-color: rgba(18, 18, 22, 0.32);
    background-image: none;
}
"""

# AMOLED UI style: pure-black window, soft multi-shade pinks, warm gold
# reserved for anything selected/active/value-like. One control language:
# flat controls flush with the black, 6px radius everywhere, chrome only on
# hover/press/toggle — the scope stays the brightest thing in the window.
BLACK_UI_CSS = b"""
window, headerbar, dialog, popover, popover.background, menu, .background {
    background-color: #000000;
    color: #f2aed8;
}
/* explicit label/cell colors win over the system theme, so popover and
   dropdown text stays readable no matter which GTK theme is underneath */
label, popover label, menu label, cellview { color: #f2aed8; }
headerbar {
    min-height: 32px;
    padding: 0 4px;
    box-shadow: none;
    border-bottom: 1px solid #1d0916;
}
headerbar .title, headerbar label { color: #fbcfe8; }
button, combobox button.combo, spinbutton button, button.color,
button.scale {
    background-image: none;
    background-color: transparent;
    color: #f2aed8;
    border: 1px solid transparent;
    border-radius: 6px;
    padding: 2px 9px;
    min-height: 24px;
    box-shadow: none;
    transition: background-color 120ms ease;
}
spinbutton button { padding: 1px 5px; }
button:hover { background-color: #2b0d20; }
button:active { background-color: #3c142d; }
button:checked { background-color: #2b2208; color: #ffdf87; }
button:checked label, button:checked image { color: #ffdf87; }
combobox arrow { color: #e078b8; }
/* values read gold; a box appears only while editing */
entry, spinbutton, spinbutton entry {
    background-image: none;
    background-color: transparent;
    color: #ffdf87;
    border: 1px solid transparent;
    border-radius: 6px;
    min-height: 22px;
    box-shadow: none;
}
entry:focus, spinbutton entry:focus {
    background-color: #120510;
    border-color: #57203f;
}
scale trough {
    background-color: #2b0d20;
    border: none;
    border-radius: 999px;
    min-height: 4px;
}
scale highlight { background-color: #e078b8; border-radius: 999px; }
scale slider {
    background-color: #fbcfe8;
    border: 1px solid #b65c92;
    border-radius: 999px;
    min-width: 14px;
    min-height: 14px;
}
scale slider:hover { background-color: #ffffff; }
switch {
    background-image: none;
    background-color: #2b0d20;
    border: 1px solid #57203f;
    border-radius: 999px;
}
switch:checked { background-color: #97276b; border-color: #e078b8; }
switch slider {
    background-image: none;
    background-color: #fbcfe8;
    border: 1px solid #b65c92;
    border-radius: 999px;
    min-width: 18px;
    min-height: 18px;
    margin: 2px;
}
menu menuitem:hover, popover modelbutton:hover { background-color: #2b0d20; }
menu menuitem:hover label { color: #fbcfe8; }
menu, popover { border: 1px solid #2b0d20; border-radius: 8px; }
menu check, menu radio { color: #e078b8; }
menu check:checked, menu radio:checked { color: #ffdf87; }
separator { background-color: #1d0916; min-width: 1px; min-height: 1px; }
scrollbar, scrollbar trough { background-color: transparent; }
scrollbar slider {
    background-color: #57203f;
    border-radius: 999px;
    min-width: 6px;
    min-height: 24px;
}
scrollbar slider:hover { background-color: #97276b; }
.settings-section {
    color: #b65c92;
    font-size: 10px;
    font-weight: bold;
    letter-spacing: 2px;
}
tooltip, tooltip.background {
    background-color: #120510;
    color: #fbcfe8;
    border: 1px solid #57203f;
}
tooltip label { color: #fbcfe8; }
/* file chooser, kept on-theme: pink on black */
dialog headerbar { background-color: #000000; }
filechooser, filechooser box, filechooser paned { background-color: #000000; }
placessidebar, placessidebar list { background-color: #0d040a; }
placessidebar row label { color: #f2aed8; }
placessidebar row:selected { background-color: #97276b; }
placessidebar row:selected label { color: #ffdf87; }
treeview.view { background-color: #0d040a; color: #f2aed8; }
treeview.view:selected { background-color: #97276b; color: #ffdf87; }
treeview.view header button { background-color: #120510; color: #f2aed8; }
actionbar { background-color: #000000; }
*:selected { background-color: #97276b; color: #ffdf87; }
#fps-overlay { color: #ffdf87; }
#now-playing { color: #fbcfe8; border: 1px solid #57203f; }
window.glass-scope {
    background-color: rgba(0, 0, 0, 0.30);
    background-image: none;
}
#playlist-panel { background-color: #0d040a; border-left: 1px solid #1d0916; }
#playlist-panel list, #playlist-panel row { background-color: transparent; }
#playlist-panel row { border-radius: 6px; }
#playlist-panel row:selected { background-color: #2b2208; }
#playlist-panel row:selected label { color: #ffdf87; }
#playlist-panel row:hover { background-color: #2b0d20; }
"""

# Bloom: the AMOLED palette, but every control is *defined* — visible
# borders and a soft neon halo, like the chrome itself is phosphorescing.
BLOOM_UI_CSS = b"""
window, headerbar, dialog, popover, popover.background, menu, .background {
    background-color: #000000;
    color: #f6bede;
}
label, popover label, menu label, cellview { color: #f6bede; }
headerbar {
    min-height: 34px;
    padding: 0 4px;
    border-bottom: 1px solid #57203f;
    box-shadow: 0 1px 8px rgba(224, 120, 184, 0.25);
}
headerbar .title, headerbar label { color: #fbcfe8; }
button, combobox button.combo, spinbutton button, button.color {
    background-image: none;
    background-color: #14060f;
    color: #f6bede;
    border: 1px solid #57203f;
    border-radius: 8px;
    padding: 2px 10px;
    min-height: 25px;
    box-shadow: 0 0 5px rgba(224, 120, 184, 0.18);
    transition: all 140ms ease;
}
spinbutton button { padding: 1px 6px; }
button:hover {
    background-color: #2b0d20;
    border-color: #b65c92;
    box-shadow: 0 0 9px rgba(224, 120, 184, 0.45);
}
button:active { background-color: #3c142d; }
button:checked {
    background-color: #2b2208;
    color: #ffdf87;
    border-color: #b8963f;
    box-shadow: 0 0 9px rgba(255, 223, 135, 0.4);
}
button:checked label, button:checked image { color: #ffdf87; }
combobox arrow { color: #e078b8; }
entry, spinbutton, spinbutton entry {
    background-image: none;
    background-color: #0d040a;
    color: #ffdf87;
    border: 1px solid #3c142d;
    border-radius: 8px;
    min-height: 23px;
    box-shadow: none;
}
entry:focus, spinbutton entry:focus {
    border-color: #e078b8;
    box-shadow: 0 0 7px rgba(224, 120, 184, 0.4);
}
scale trough {
    background-color: #14060f;
    border: 1px solid #3c142d;
    border-radius: 999px;
    min-height: 6px;
}
scale highlight {
    background-color: #e078b8;
    border-radius: 999px;
    box-shadow: 0 0 6px rgba(224, 120, 184, 0.6);
}
scale slider {
    background-color: #fbcfe8;
    border: 1px solid #e078b8;
    border-radius: 999px;
    min-width: 15px;
    min-height: 15px;
    box-shadow: 0 0 7px rgba(251, 207, 232, 0.5);
}
switch {
    background-image: none;
    background-color: #14060f;
    border: 1px solid #57203f;
    border-radius: 999px;
}
switch:checked {
    background-color: #97276b;
    border-color: #e078b8;
    box-shadow: 0 0 8px rgba(224, 120, 184, 0.5);
}
switch slider {
    background-image: none;
    background-color: #fbcfe8;
    border: 1px solid #b65c92;
    border-radius: 999px;
    min-width: 18px;
    min-height: 18px;
    margin: 2px;
}
menu menuitem:hover, popover modelbutton:hover { background-color: #2b0d20; }
menu, popover {
    border: 1px solid #57203f;
    border-radius: 10px;
    box-shadow: 0 0 14px rgba(224, 120, 184, 0.3);
}
menu check, menu radio { color: #e078b8; }
menu check:checked, menu radio:checked { color: #ffdf87; }
separator { background-color: #3c142d; min-width: 1px; min-height: 1px; }
scrollbar, scrollbar trough { background-color: transparent; }
scrollbar slider {
    background-color: #97276b;
    border-radius: 999px;
    min-width: 7px;
    min-height: 24px;
    box-shadow: 0 0 5px rgba(224, 120, 184, 0.4);
}
.settings-section {
    color: #e078b8;
    font-size: 10px;
    font-weight: bold;
    letter-spacing: 2px;
}
tooltip, tooltip.background {
    background-color: #120510;
    color: #fbcfe8;
    border: 1px solid #b65c92;
}
tooltip label { color: #fbcfe8; }
dialog headerbar { background-color: #000000; }
filechooser, filechooser box, filechooser paned { background-color: #000000; }
placessidebar, placessidebar list { background-color: #0d040a; }
placessidebar row label { color: #f6bede; }
placessidebar row:selected { background-color: #97276b; }
placessidebar row:selected label { color: #ffdf87; }
treeview.view { background-color: #0d040a; color: #f6bede; }
treeview.view:selected { background-color: #97276b; color: #ffdf87; }
treeview.view header button { background-color: #120510; color: #f6bede; }
actionbar { background-color: #000000; }
*:selected { background-color: #97276b; color: #ffdf87; }
#fps-overlay { color: #ffdf87; }
#now-playing {
    color: #fbcfe8;
    border: 1px solid #b65c92;
    box-shadow: 0 0 10px rgba(224, 120, 184, 0.35);
}
window.glass-scope {
    background-color: rgba(0, 0, 0, 0.30);
    background-image: none;
}
#playlist-panel { background-color: #0d040a; border-left: 1px solid #57203f; }
#playlist-panel list, #playlist-panel row { background-color: transparent; }
#playlist-panel row { border-radius: 8px; }
#playlist-panel row:selected { background-color: #2b2208; }
#playlist-panel row:selected label { color: #ffdf87; }
#playlist-panel row:hover { background-color: #2b0d20; }
"""

# Stonework 95: hard edges, chiseled bevels, granite and parchment — a
# Windows-95 skeleton wearing dwarven masonry. Zero border radius anywhere;
# light falls from the top-left, exactly like it used to.
STONE_UI_CSS = b"""
window, headerbar, dialog, popover, popover.background, menu, .background {
    background-color: #4a453e;
    color: #e8dcc0;
}
label, popover label, menu label, cellview { color: #e8dcc0; }
headerbar {
    background-image: linear-gradient(#5a544b, #4a453e);
    min-height: 34px;
    padding: 0 4px;
    box-shadow: none;
    border-bottom: 2px solid #2e2a25;
}
headerbar .title, headerbar label { color: #f4ead0; font-weight: bold; }
button, combobox button.combo, spinbutton button, button.color {
    background-image: linear-gradient(#6b655b, #57524a);
    color: #e8dcc0;
    border: 2px solid;
    border-color: #8a8375 #2e2a25 #2e2a25 #8a8375;
    border-radius: 0;
    padding: 0 6px;
    min-height: 20px;
    box-shadow: none;
    transition: none;
}
spinbutton button { padding: 0 4px; }
button:hover { background-image: linear-gradient(#756e63, #605a51); }
button:active, button:checked {
    background-image: linear-gradient(#3e3a34, #4a453e);
    border-color: #2e2a25 #8a8375 #8a8375 #2e2a25;
}
button:checked { color: #ffd76e; }
button:checked label, button:checked image { color: #ffd76e; }
combobox arrow { color: #c9b98a; }
entry, spinbutton, spinbutton entry {
    background-image: none;
    background-color: #322e29;
    color: #ffd76e;
    border: 2px solid;
    border-color: #2e2a25 #8a8375 #8a8375 #2e2a25;
    border-radius: 0;
    min-height: 22px;
    box-shadow: none;
}
entry:focus, spinbutton entry:focus { background-color: #3a352f; }
scale trough {
    background-color: #322e29;
    border: 2px solid;
    border-color: #2e2a25 #8a8375 #8a8375 #2e2a25;
    border-radius: 0;
    min-height: 8px;
}
scale highlight { background-color: #b8963f; border-radius: 0; }
scale slider {
    background-image: linear-gradient(#6b655b, #57524a);
    border: 2px solid;
    border-color: #8a8375 #2e2a25 #2e2a25 #8a8375;
    border-radius: 0;
    min-width: 12px;
    min-height: 20px;
}
switch {
    background-image: none;
    background-color: #322e29;
    border: 2px solid;
    border-color: #2e2a25 #8a8375 #8a8375 #2e2a25;
    border-radius: 0;
}
switch:checked { background-color: #6e5a24; }
switch slider {
    background-image: linear-gradient(#6b655b, #57524a);
    border: 2px solid;
    border-color: #8a8375 #2e2a25 #2e2a25 #8a8375;
    border-radius: 0;
    min-width: 16px;
    min-height: 16px;
    margin: 1px;
}
menu menuitem:hover, popover modelbutton:hover { background-color: #6e5a24; }
menu menuitem:hover label { color: #f4ead0; }
menu, popover {
    border: 2px solid;
    border-color: #8a8375 #2e2a25 #2e2a25 #8a8375;
    border-radius: 0;
}
menu check, menu radio { color: #c9b98a; }
menu check:checked, menu radio:checked { color: #ffd76e; }
separator { background-color: #2e2a25; min-width: 2px; min-height: 2px; }
scrollbar, scrollbar trough { background-color: #3a352f; }
scrollbar slider {
    background-image: linear-gradient(#6b655b, #57524a);
    border: 2px solid;
    border-color: #8a8375 #2e2a25 #2e2a25 #8a8375;
    border-radius: 0;
    min-width: 12px;
    min-height: 24px;
}
.settings-section {
    color: #c9b98a;
    font-size: 10px;
    font-weight: bold;
    letter-spacing: 3px;
}
tooltip, tooltip.background {
    background-color: #322e29;
    color: #f4ead0;
    border: 1px solid #8a8375;
}
tooltip label { color: #f4ead0; }
dialog headerbar { background-image: linear-gradient(#5a544b, #4a453e); }
filechooser, filechooser box, filechooser paned { background-color: #4a453e; }
placessidebar, placessidebar list { background-color: #3e3a34; }
placessidebar row label { color: #e8dcc0; }
placessidebar row:selected { background-color: #6e5a24; }
placessidebar row:selected label { color: #ffd76e; }
treeview.view { background-color: #322e29; color: #e8dcc0; }
treeview.view:selected { background-color: #6e5a24; color: #ffd76e; }
treeview.view header button { background-color: #57524a; color: #e8dcc0; }
actionbar { background-color: #4a453e; }
*:selected { background-color: #6e5a24; color: #ffd76e; }
#fps-overlay { color: #ffd76e; }
#now-playing {
    background-color: rgba(50, 46, 41, 0.85);
    color: #f4ead0;
    border: 2px solid;
    border-color: #8a8375 #2e2a25 #2e2a25 #8a8375;
    border-radius: 0;
}
window.glass-scope {
    background-color: rgba(56, 52, 46, 0.38);
    background-image: none;
}
#playlist-panel { background-color: #3e3a34; border-left: 2px solid #2e2a25; }
#playlist-panel list, #playlist-panel row { background-color: transparent; }
#playlist-panel row { border-radius: 0; }
#playlist-panel row:selected { background-color: #6e5a24; }
#playlist-panel row:selected label { color: #ffd76e; }
#playlist-panel row:hover { background-color: #57524a; }
"""


# Stonework · bloom: the same chiseled masonry, but the runes are lit —
# ember light seeps out of whatever you touch, gold means molten.
STONEBLOOM_UI_CSS = b"""
window, headerbar, dialog, popover, popover.background, menu, .background {
    background-color: #38342e;
    color: #ecdfc2;
}
label, popover label, menu label, cellview { color: #ecdfc2; }
headerbar {
    background-image: linear-gradient(#4a453e, #38342e);
    min-height: 34px;
    padding: 0 4px;
    border-bottom: 2px solid #211d18;
    box-shadow: 0 1px 10px rgba(255, 190, 80, 0.14);
}
headerbar .title, headerbar label { color: #f4ead0; font-weight: bold; }
button, combobox button.combo, spinbutton button, button.color {
    background-image: linear-gradient(#57524a, #45403a);
    color: #ecdfc2;
    border: 2px solid;
    border-color: #7d766a #211d18 #211d18 #7d766a;
    border-radius: 0;
    padding: 0 6px;
    min-height: 20px;
    box-shadow: 0 0 5px rgba(255, 190, 80, 0.10);
    transition: box-shadow 140ms ease;
}
spinbutton button { padding: 0 4px; }
button:hover {
    background-image: linear-gradient(#615a50, #4e4841);
    color: #ffd76e;
    box-shadow: 0 0 11px rgba(255, 190, 80, 0.5);
}
button:active, button:checked {
    background-image: linear-gradient(#3a2d10, #4a3a14);
    border-color: #211d18 #7d766a #7d766a #211d18;
}
button:checked {
    color: #ffcf5e;
    box-shadow: 0 0 12px rgba(255, 200, 90, 0.55);
}
button:checked label, button:checked image { color: #ffcf5e; }
combobox arrow { color: #d8b96a; }
entry, spinbutton, spinbutton entry {
    background-image: none;
    background-color: #262218;
    color: #ffcf5e;
    border: 2px solid;
    border-color: #211d18 #7d766a #7d766a #211d18;
    border-radius: 0;
    min-height: 22px;
    box-shadow: none;
}
entry:focus, spinbutton entry:focus {
    background-color: #2e2a24;
    box-shadow: 0 0 9px rgba(255, 190, 80, 0.4);
}
scale trough {
    background-color: #262218;
    border: 2px solid;
    border-color: #211d18 #7d766a #7d766a #211d18;
    border-radius: 0;
    min-height: 8px;
}
scale highlight {
    background-image: linear-gradient(#ffcf5e, #c98f2e);
    border-radius: 0;
    box-shadow: 0 0 7px rgba(255, 200, 90, 0.6);
}
scale slider {
    background-image: linear-gradient(#615a50, #4e4841);
    border: 2px solid;
    border-color: #7d766a #211d18 #211d18 #7d766a;
    border-radius: 0;
    min-width: 12px;
    min-height: 20px;
}
scale slider:hover { box-shadow: 0 0 9px rgba(255, 190, 80, 0.55); }
switch {
    background-image: none;
    background-color: #262218;
    border: 2px solid;
    border-color: #211d18 #7d766a #7d766a #211d18;
    border-radius: 0;
}
switch:checked {
    background-color: #6e5312;
    box-shadow: 0 0 9px rgba(255, 200, 90, 0.5);
}
switch slider {
    background-image: linear-gradient(#615a50, #4e4841);
    border: 2px solid;
    border-color: #7d766a #211d18 #211d18 #7d766a;
    border-radius: 0;
    min-width: 16px;
    min-height: 16px;
    margin: 1px;
}
menu menuitem:hover, popover modelbutton:hover { background-color: #6e5312; }
menu menuitem:hover label { color: #ffe9b0; }
menu, popover {
    border: 2px solid;
    border-color: #7d766a #211d18 #211d18 #7d766a;
    border-radius: 0;
    box-shadow: 0 0 16px rgba(255, 190, 80, 0.22);
}
menu check, menu radio { color: #d8b96a; }
menu check:checked, menu radio:checked { color: #ffcf5e; }
separator { background-color: #211d18; min-width: 2px; min-height: 2px; }
scrollbar, scrollbar trough { background-color: #2e2a24; }
scrollbar slider {
    background-image: linear-gradient(#615a50, #4e4841);
    border: 2px solid;
    border-color: #7d766a #211d18 #211d18 #7d766a;
    border-radius: 0;
    min-width: 12px;
    min-height: 24px;
}
scrollbar slider:hover { box-shadow: 0 0 8px rgba(255, 190, 80, 0.5); }
.settings-section {
    color: #d8b96a;
    font-size: 10px;
    font-weight: bold;
    letter-spacing: 3px;
}
tooltip, tooltip.background {
    background-color: #262218;
    color: #ffe9b0;
    border: 1px solid #c98f2e;
}
tooltip label { color: #ffe9b0; }
dialog headerbar { background-image: linear-gradient(#4a453e, #38342e); }
filechooser, filechooser box, filechooser paned { background-color: #38342e; }
placessidebar, placessidebar list { background-color: #2e2a24; }
placessidebar row label { color: #ecdfc2; }
placessidebar row:selected { background-color: #6e5312; }
placessidebar row:selected label { color: #ffcf5e; }
treeview.view { background-color: #262218; color: #ecdfc2; }
treeview.view:selected { background-color: #6e5312; color: #ffcf5e; }
treeview.view header button { background-color: #45403a; color: #ecdfc2; }
actionbar { background-color: #38342e; }
*:selected { background-color: #6e5312; color: #ffcf5e; }
#fps-overlay { color: #ffcf5e; }
#now-playing {
    background-color: rgba(38, 34, 29, 0.88);
    color: #f4ead0;
    border: 2px solid;
    border-color: #7d766a #211d18 #211d18 #7d766a;
    border-radius: 0;
    box-shadow: 0 0 12px rgba(255, 190, 80, 0.3);
}
window.glass-scope {
    background-color: rgba(38, 34, 30, 0.38);
    background-image: none;
}
#playlist-panel { background-color: #2e2a24; border-left: 2px solid #211d18; }
#playlist-panel list, #playlist-panel row { background-color: transparent; }
#playlist-panel row { border-radius: 0; }
#playlist-panel row:selected { background-color: #6e5312; }
#playlist-panel row:selected label { color: #ffcf5e; }
#playlist-panel row:hover { background-color: #4e4841; }
"""

# Aero glass: translucent Frutiger-era gradients, white sheen on every
# control, deep-sky selection — the 2000s visualizer energy, in chrome.
AERO_UI_CSS = b"""
/* a transparent decoration node + non-opaque background-color are what
   let GTK3 hand the compositor real alpha - without them the frame
   flattens everything against an opaque base */
decoration { background-color: transparent; background-image: none; }
window, dialog, .background {
    background-color: rgba(196, 228, 243, 0.60);
    background-image: linear-gradient(rgba(223, 242, 251, 0.72),
                                      rgba(184, 223, 240, 0.62) 40%,
                                      rgba(165, 212, 234, 0.58));
    color: #123a52;
}
popover, popover.background, menu {
    background-color: rgba(235, 248, 253, 0.94);
    color: #123a52;
}
label, popover label, menu label, cellview { color: #123a52; }
headerbar {
    background-image: linear-gradient(rgba(255,255,255,0.72),
                                      rgba(190, 226, 243, 0.66));
    min-height: 34px;
    padding: 0 4px;
    border-bottom: 1px solid rgba(125, 184, 214, 0.8);
    box-shadow: inset 0 1px 0 rgba(255,255,255,0.85);
}
headerbar .title, headerbar label { color: #0d3752; }
button, combobox button.combo, spinbutton button, button.color {
    background-image: linear-gradient(rgba(255,255,255,0.95),
                                      rgba(214, 238, 248, 0.9) 45%,
                                      rgba(176, 220, 240, 0.9));
    color: #123a52;
    border: 1px solid #8fc3dd;
    border-radius: 7px;
    padding: 2px 10px;
    min-height: 25px;
    box-shadow: inset 0 1px 0 rgba(255,255,255,0.9);
    transition: all 140ms ease;
}
spinbutton button { padding: 1px 6px; }
button:hover {
    background-image: linear-gradient(#ffffff, #d0ecfa 45%, #a8dcf5);
    border-color: #5aa5cc;
}
button:active {
    background-image: linear-gradient(#a8d4e8, #c8e6f4);
}
button:checked {
    background-image: linear-gradient(#8fd3f0, #4d9fca);
    color: #ffffff;
    border-color: #337fa8;
}
button:checked label, button:checked image { color: #ffffff; }
combobox arrow { color: #2f7fb8; }
entry, spinbutton, spinbutton entry {
    background-image: none;
    background-color: rgba(255, 255, 255, 0.85);
    color: #0d3752;
    border: 1px solid #8fc3dd;
    border-radius: 7px;
    min-height: 23px;
    box-shadow: inset 0 1px 2px rgba(18, 58, 82, 0.15);
}
entry:focus, spinbutton entry:focus { border-color: #2f7fb8; }
scale trough {
    background-image: linear-gradient(rgba(140, 180, 200, 0.5),
                                      rgba(220, 240, 250, 0.7));
    border: 1px solid #8fc3dd;
    border-radius: 999px;
    min-height: 6px;
}
scale highlight {
    background-image: linear-gradient(#9be0ff, #38a3dd);
    border-radius: 999px;
}
scale slider {
    background-image: linear-gradient(#ffffff, #cdeaf8);
    border: 1px solid #5aa5cc;
    border-radius: 999px;
    min-width: 15px;
    min-height: 15px;
    box-shadow: 0 1px 3px rgba(18, 58, 82, 0.3);
}
switch {
    background-image: linear-gradient(rgba(160, 200, 220, 0.6),
                                      rgba(220, 240, 250, 0.8));
    border: 1px solid #8fc3dd;
    border-radius: 999px;
}
switch:checked {
    background-image: linear-gradient(#8fd3f0, #4d9fca);
    border-color: #337fa8;
}
switch slider {
    background-image: linear-gradient(#ffffff, #d8eef8);
    border: 1px solid #5aa5cc;
    border-radius: 999px;
    min-width: 18px;
    min-height: 18px;
    margin: 2px;
}
menu menuitem:hover, popover modelbutton:hover {
    background-image: linear-gradient(#c8e9fa, #9fd4ef);
}
menu, popover { border: 1px solid #7db8d6; border-radius: 9px; }
menu check, menu radio { color: #2f7fb8; }
menu check:checked, menu radio:checked { color: #0d3752; }
separator { background-color: #9fcde2; min-width: 1px; min-height: 1px; }
scrollbar, scrollbar trough { background-color: transparent; }
scrollbar slider {
    background-image: linear-gradient(#b8e2f5, #7dbcd9);
    border: 1px solid #5aa5cc;
    border-radius: 999px;
    min-width: 8px;
    min-height: 24px;
}
.settings-section {
    color: #2f7fb8;
    font-size: 10px;
    font-weight: bold;
    letter-spacing: 2px;
}
tooltip, tooltip.background {
    background-color: rgba(13, 55, 82, 0.92);
    color: #e8f6fd;
    border: 1px solid #5aa5cc;
}
tooltip label { color: #e8f6fd; }
dialog headerbar {
    background-image: linear-gradient(rgba(255,255,255,0.9),
                                      rgba(190, 226, 243, 0.9));
}
filechooser, filechooser box, filechooser paned {
    background-color: #e4f4fb;
}
filechooser image, placessidebar image { -gtk-icon-style: symbolic; }
placessidebar, placessidebar list { background-color: #cfe9f5; }
placessidebar row label, placessidebar row image { color: #123a52; }
placessidebar row:selected {
    background-image: linear-gradient(#8fd3f0, #4d9fca);
}
placessidebar row:selected label,
placessidebar row:selected image { color: #ffffff; }
treeview.view { background-color: #f2fafd; color: #123a52; }
treeview.view:selected {
    background-image: linear-gradient(#8fd3f0, #4d9fca);
    color: #ffffff;
}
treeview.view header button { background-color: #d6edf7; color: #123a52; }
actionbar { background-color: #cfe9f5; }
*:selected { background-color: #4d9fca; color: #ffffff; }
#fps-overlay { background-color: rgba(255,255,255,0.8); color: #1d6fa0; }
#now-playing {
    background-color: rgba(255, 255, 255, 0.82);
    color: #0d3752;
    border: 1px solid #7db8d6;
}
window.glass-scope {
    background-color: rgba(196, 228, 243, 0.30);
    background-image: none;
}
#playlist-panel {
    background-color: rgba(207, 233, 245, 0.72);
    border-left: 1px solid rgba(159, 205, 226, 0.8);
}
#playlist-panel list, #playlist-panel row { background-color: transparent; }
#playlist-panel row { border-radius: 7px; }
#playlist-panel row:selected {
    background-image: linear-gradient(#8fd3f0, #4d9fca);
}
#playlist-panel row:selected label { color: #ffffff; }
#playlist-panel row:hover { background-color: rgba(255,255,255,0.6); }
"""

# Ice Blue ❄: bright frost chrome with a glacial accent. The file chooser
# forces symbolic icons colored like the text, so no icon theme underneath
# can render them invisible against the pale surfaces.
ICE_UI_CSS = b"""
window, headerbar, dialog, popover, popover.background, menu, .background {
    background-color: #f0f7fb;
    color: #29465c;
}
label, popover label, menu label, cellview { color: #29465c; }
headerbar {
    background-image: none;
    background-color: #e1eef6;
    min-height: 32px;
    padding: 0 4px;
    box-shadow: none;
    border-bottom: 1px solid #c2dcea;
}
headerbar .title, headerbar label { color: #1c3a50; }
button, combobox button.combo, spinbutton button, button.color {
    background-image: none;
    background-color: transparent;
    color: #29465c;
    border: 1px solid transparent;
    border-radius: 6px;
    padding: 2px 9px;
    min-height: 24px;
    box-shadow: none;
    transition: background-color 120ms ease;
}
spinbutton button { padding: 1px 5px; }
button:hover { background-color: #ddedf6; }
button:active { background-color: #cbe3f0; }
button:checked { background-color: #cde8f7; color: #135a8c; }
button:checked label, button:checked image { color: #135a8c; }
combobox arrow { color: #2f7fb8; }
entry, spinbutton, spinbutton entry {
    background-image: none;
    background-color: transparent;
    color: #10344e;
    border: 1px solid transparent;
    border-radius: 6px;
    min-height: 22px;
    box-shadow: none;
}
entry:focus, spinbutton entry:focus {
    background-color: #ffffff;
    border-color: #7ab4d8;
}
scale trough {
    background-color: #d8e9f2;
    border: none;
    border-radius: 999px;
    min-height: 4px;
}
scale highlight { background-color: #4295cc; border-radius: 999px; }
scale slider {
    background-color: #ffffff;
    border: 1px solid #7ab4d8;
    border-radius: 999px;
    min-width: 14px;
    min-height: 14px;
}
switch {
    background-image: none;
    background-color: #d8e9f2;
    border: 1px solid #b5d4e5;
    border-radius: 999px;
}
switch:checked { background-color: #4295cc; }
switch slider {
    background-image: none;
    background-color: #ffffff;
    border: 1px solid #7ab4d8;
    border-radius: 999px;
    min-width: 18px;
    min-height: 18px;
    margin: 2px;
}
menu menuitem:hover, popover modelbutton:hover { background-color: #ddedf6; }
menu, popover { border: 1px solid #c2dcea; border-radius: 8px; }
menu check, menu radio { color: #2f7fb8; }
menu check:checked, menu radio:checked { color: #135a8c; }
separator { background-color: #c2dcea; min-width: 1px; min-height: 1px; }
scrollbar, scrollbar trough { background-color: transparent; }
scrollbar slider {
    background-color: #b5d4e5;
    border-radius: 999px;
    min-width: 6px;
    min-height: 24px;
}
scrollbar slider:hover { background-color: #7ab4d8; }
.settings-section {
    color: #4a7fa0;
    font-size: 10px;
    font-weight: bold;
    letter-spacing: 2px;
}
tooltip, tooltip.background {
    background-color: #1c3a50;
    color: #eaf6fd;
}
tooltip label { color: #eaf6fd; }
/* file chooser: force everything frost-light AND force symbolic icons;
   full-color icon themes made for dark desktops used to vanish here */
dialog headerbar { background-color: #e1eef6; }
filechooser, filechooser box, filechooser paned { background-color: #f0f7fb; }
filechooser image, placessidebar image, pathbar image, button.dialog image {
    -gtk-icon-style: symbolic;
    color: #2f6d96;
}
placessidebar, placessidebar list { background-color: #e1eef6; }
placessidebar row label { color: #29465c; }
placessidebar row:selected { background-color: #4295cc; }
placessidebar row:selected label,
placessidebar row:selected image { color: #ffffff; }
treeview.view { background-color: #ffffff; color: #29465c; }
treeview.view:selected { background-color: #4295cc; color: #ffffff; }
treeview.view header button { background-color: #e1eef6; color: #29465c; }
actionbar { background-color: #e1eef6; }
*:selected { background-color: #4295cc; color: #ffffff; }
#fps-overlay { background-color: rgba(255, 255, 255, 0.8); color: #135a8c; }
#now-playing {
    background-color: rgba(255, 255, 255, 0.88);
    color: #10344e;
    border: 1px solid #b5d4e5;
}
window.glass-scope {
    background-color: rgba(240, 247, 251, 0.40);
    background-image: none;
}
#playlist-panel { background-color: #e1eef6; border-left: 1px solid #c2dcea; }
#playlist-panel list, #playlist-panel row { background-color: transparent; }
#playlist-panel row { border-radius: 6px; }
#playlist-panel row:selected { background-color: #cde8f7; }
#playlist-panel row:selected label { color: #135a8c; }
#playlist-panel row:hover { background-color: #ddedf6; }
"""

_STYLE_CSS = {"black": BLACK_UI_CSS, "bloom": BLOOM_UI_CSS,
              "stone": STONE_UI_CSS, "stonebloom": STONEBLOOM_UI_CSS,
              "aero": AERO_UI_CSS, "light": ICE_UI_CSS}


def install_base_style():
    """Load the always-on CSS (overlay chips); call once at startup."""
    provider = Gtk.CssProvider()
    provider.load_from_data(BASE_UI_CSS)
    Gtk.StyleContext.add_provider_for_screen(
        Gdk.Screen.get_default(), provider,
        Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)


def apply_ui_style(style, previous_provider=None):
    """Switch the whole app to `style`; returns the new provider (or None).

    Pass the provider returned last time so the old style is removed first.
    """
    Gtk.Settings.get_default().set_property(
        "gtk-application-prefer-dark-theme", style in DARK_STYLES)
    screen = Gdk.Screen.get_default()
    if previous_provider is not None:
        Gtk.StyleContext.remove_provider_for_screen(screen, previous_provider)
    style_css = _STYLE_CSS.get(style)
    if style_css is None:
        return None
    provider = Gtk.CssProvider()
    provider.load_from_data(style_css)
    Gtk.StyleContext.add_provider_for_screen(
        screen, provider, Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
    return provider
