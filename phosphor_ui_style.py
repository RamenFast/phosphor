# SPDX-License-Identifier: GPL-3.0-or-later
"""UI chrome styles for Phosphor.

Three switchable styles ("system" adds nothing, "dark" only flips the GTK
dark preference): a pure-black AMOLED pink/gold look and a bright neutral
light look. Both are flat — controls sit flush with the background and only
show chrome on hover/press/toggle — so the scope stays the brightest thing
in the window.
"""

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk  # noqa: E402

UI_STYLE_CHOICES = (("system", "System"), ("dark", "Dark"),
                    ("light", "Light"), ("black", "AMOLED pink"))

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
#playlist-panel { background-color: #0d040a; border-left: 1px solid #1d0916; }
#playlist-panel list, #playlist-panel row { background-color: transparent; }
#playlist-panel row { border-radius: 6px; }
#playlist-panel row:selected { background-color: #2b2208; }
#playlist-panel row:selected label { color: #ffdf87; }
#playlist-panel row:hover { background-color: #2b0d20; }
"""

# Light UI style: bright neutral chrome with a blue accent. Everything is
# flat — buttons, combos, and spin steppers sit flush with the background
# (one shared corner radius) and only show chrome on hover/press/toggle.
LIGHT_UI_CSS = b"""
window, headerbar, dialog, popover, popover.background, menu, .background {
    background-color: #fafafa;
    color: #303030;
}
/* explicit label/cell colors win over the system theme, so popover and
   dropdown text stays readable even on top of a dark GTK theme */
label, popover label, menu label, cellview { color: #303030; }
menu check, menu radio { color: #1c4e9e; }
headerbar {
    background-image: none;
    background-color: #f0f0f0;
    min-height: 32px;
    padding: 0 4px;
    box-shadow: none;
    border-bottom: 1px solid #d8d8d8;
}
headerbar .title, headerbar label { color: #303030; }
button, combobox button.combo, spinbutton button, button.color {
    background-image: none;
    background-color: transparent;
    color: #303030;
    border: 1px solid transparent;
    border-radius: 6px;
    padding: 2px 9px;
    min-height: 24px;
    box-shadow: none;
    transition: background-color 120ms ease;
}
spinbutton button { padding: 1px 5px; }
button:hover { background-color: #e7e7e7; }
button:active { background-color: #dadada; }
button:checked { background-color: #dceaff; color: #1c4e9e; }
button:checked label, button:checked image { color: #1c4e9e; }
/* entries are flat too; a box appears only while editing */
entry, spinbutton, spinbutton entry {
    background-image: none;
    background-color: transparent;
    color: #222222;
    border: 1px solid transparent;
    border-radius: 6px;
    min-height: 22px;
    box-shadow: none;
}
entry:focus, spinbutton entry:focus {
    background-color: #ffffff;
    border-color: #7aa7e0;
}
scale trough { background-color: #e4e4e4; border: none; }
scale highlight { background-color: #5a8fd6; }
scale slider { background-color: #ffffff; border: 1px solid #9a9a9a; }
switch {
    background-image: none;
    background-color: #e0e0e0;
    border: 1px solid #c9c9c9;
    border-radius: 999px;
}
switch:checked { background-color: #5a8fd6; }
switch slider {
    background-image: none;
    background-color: #ffffff;
    border: 1px solid #9a9a9a;
    border-radius: 999px;
    min-width: 18px;
    min-height: 18px;
    margin: 2px;
}
menu menuitem:hover, popover modelbutton:hover { background-color: #e8eef8; }
separator { background-color: #d8d8d8; min-width: 1px; min-height: 1px; }
scrollbar, scrollbar trough { background-color: transparent; }
scrollbar slider {
    background-color: #c9c9c9;
    border-radius: 999px;
    min-width: 6px;
    min-height: 24px;
}
scrollbar slider:hover { background-color: #9a9a9a; }
.settings-section {
    color: #8a8a8a;
    font-size: 10px;
    font-weight: bold;
    letter-spacing: 2px;
}
tooltip, tooltip.background {
    background-color: #303030;
    color: #fafafa;
}
/* file chooser: force the whole dialog light so it can't come up in the
   system dark theme with our dark label color on top (unreadable) */
dialog headerbar { background-color: #f0f0f0; }
filechooser, filechooser box, filechooser paned { background-color: #fafafa; }
placessidebar, placessidebar list { background-color: #f0f0f0; }
placessidebar row label { color: #303030; }
placessidebar row:selected { background-color: #5a8fd6; }
placessidebar row:selected label { color: #ffffff; }
treeview.view { background-color: #ffffff; color: #303030; }
treeview.view:selected { background-color: #5a8fd6; color: #ffffff; }
treeview.view header button { background-color: #f0f0f0; color: #303030; }
actionbar { background-color: #f0f0f0; }
*:selected { background-color: #5a8fd6; color: #ffffff; }
#fps-overlay { background-color: rgba(255, 255, 255, 0.75); color: #1c4e9e; }
#now-playing {
    background-color: rgba(255, 255, 255, 0.85);
    color: #1a1a1a;
    border: 1px solid #d8d8d8;
}
#playlist-panel { background-color: #f0f0f0; }
#playlist-panel list, #playlist-panel row { background-color: transparent; }
#playlist-panel row:selected { background-color: #dceaff; }
#playlist-panel row:selected label { color: #1c4e9e; }
#playlist-panel row:hover { background-color: #e7e7e7; }
"""

_STYLE_CSS = {"black": BLACK_UI_CSS, "light": LIGHT_UI_CSS}


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
        "gtk-application-prefer-dark-theme", style in ("dark", "black"))
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
