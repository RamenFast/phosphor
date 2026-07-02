# SPDX-License-Identifier: GPL-3.0-or-later
"""The kit editor: compose a .phoskit transform chain against the live scope.

A kit is a chain of signal-space ops (see phosphor_kit); this dialog is how
one gets made without hand-writing JSON. Every tweak applies to the running
scope immediately — compose with the beam moving, never blind — and "Save
kit…" writes the chain into the kit directory as a shareable postcard.

The parameter rows are generated from phosphor_kit.OPERATIONS, so a new op
in the table grows an editor row automatically.
"""

import os

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, GLib  # noqa: E402

import phosphor_kit

PARAMETER_STEP = {"hz": 0.01, "angle": 0.01, "width": 0.05, "depth": 0.01,
                  "ms": 0.5, "channel": 1.0,
                  "a": 0.05, "b": 0.05, "c": 0.05, "d": 0.05}


def open_kit_editor(window):
    """Open (or present) the kit editor for the scope window."""
    existing = getattr(window, "_kit_editor", None)
    if existing is not None:
        existing.dialog.present()
        return existing
    editor = KitEditor(window)
    window._kit_editor = editor
    return editor


class KitEditor:
    def __init__(self, window):
        self.window = window
        self.dialog = Gtk.Dialog(title="Kit editor — compose a signal kit",
                                 transient_for=window, modal=False)
        self.dialog.set_default_size(430, 340)
        self.dialog.connect("response", self._on_response)
        self.dialog.connect("delete-event", self._on_deleted)

        content = self.dialog.get_content_area()
        content.set_spacing(6)
        for edge in ("start", "end", "top", "bottom"):
            getattr(content, f"set_margin_{edge}")(10)

        header = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        self.name_entry = Gtk.Entry()
        self.name_entry.set_placeholder_text("kit name")
        self.name_entry.set_width_chars(16)
        self.author_entry = Gtk.Entry()
        self.author_entry.set_placeholder_text("by")
        self.author_entry.set_text(window.settings.postcard_credit)
        self.author_entry.set_width_chars(12)
        header.pack_start(Gtk.Label(label="Name"), False, False, 0)
        header.pack_start(self.name_entry, True, True, 0)
        header.pack_start(Gtk.Label(label="by"), False, False, 0)
        header.pack_start(self.author_entry, True, True, 0)
        add_button = Gtk.MenuButton(label="＋ stage")
        add_button.set_tooltip_text("Append an op to the chain")
        add_menu = Gtk.Menu()
        for op_name in phosphor_kit.OPERATIONS:
            item = Gtk.MenuItem(label=op_name)
            item.connect("activate",
                         lambda _i, op=op_name: self._add_stage(op))
            add_menu.append(item)
        add_menu.show_all()
        add_button.set_popup(add_menu)
        header.pack_end(add_button, False, False, 0)
        content.pack_start(header, False, False, 0)

        scroller = Gtk.ScrolledWindow()
        scroller.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        self.stage_list = Gtk.ListBox()
        self.stage_list.set_selection_mode(Gtk.SelectionMode.NONE)
        scroller.add(self.stage_list)
        content.pack_start(scroller, True, True, 0)

        hint = Gtk.Label()
        hint.set_markup("<small>every tweak plays live on the scope — "
                        "ops run top to bottom</small>")
        hint.set_xalign(0.0)
        content.pack_start(hint, False, False, 0)

        self.dialog.add_button("Load current", 100)
        self.dialog.add_button("Save kit…", 101)
        self.dialog.add_button("_Close", Gtk.ResponseType.CLOSE)

        # seed from the active kit so "tweak what I hear" just works
        self._load_active_kit()
        self.dialog.show_all()

    # ------------------------------------------------------------- stages --

    def _load_active_kit(self):
        settings = self.window.settings
        if settings.kit_path and os.path.exists(settings.kit_path):
            try:
                name, author, stages = phosphor_kit.load(settings.kit_path)
            except (OSError, ValueError):
                return
            self.name_entry.set_text(name)
            if author:
                self.author_entry.set_text(author)
            for op, parameters in stages:
                self._add_stage(op, parameters, apply_now=False)

    def _add_stage(self, op_name, parameters=None, apply_now=True):
        row = Gtk.ListBoxRow()
        row.set_activatable(False)
        box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=4)
        for edge in ("start", "end"):
            getattr(box, f"set_margin_{edge}")(4)
        box.set_margin_top(2)
        box.set_margin_bottom(2)
        name_label = Gtk.Label(label=op_name)
        name_label.set_xalign(0.0)
        name_label.set_width_chars(9)
        box.pack_start(name_label, False, False, 0)

        spins = []
        specification = phosphor_kit.OPERATIONS[op_name]
        for index, (key, default, low, high) in enumerate(specification):
            value = (parameters[index] if parameters is not None
                     and index < len(parameters) else default)
            step = PARAMETER_STEP.get(key, 0.01)
            spin = Gtk.SpinButton.new_with_range(low, high, step)
            spin.set_digits(0 if step >= 1.0 else 2)
            spin.set_value(value)
            spin.set_tooltip_text(key)
            spin.connect("value-changed", lambda _s: self._apply_live())
            box.pack_start(Gtk.Label(label=key), False, False, 2)
            box.pack_start(spin, False, False, 0)
            spins.append(spin)

        remove_button = Gtk.Button.new_from_icon_name(
            "list-remove-symbolic", Gtk.IconSize.BUTTON)
        remove_button.set_relief(Gtk.ReliefStyle.NONE)
        remove_button.set_tooltip_text("Remove this stage")
        remove_button.connect(
            "clicked", lambda _b: (self.stage_list.remove(row),
                                   self._apply_live()))
        box.pack_end(remove_button, False, False, 0)

        row.add(box)
        row.op_name = op_name
        row.spins = spins
        self.stage_list.add(row)
        self.stage_list.show_all()
        if apply_now:
            self._apply_live()

    def current_stages(self):
        """Canonical [(op, [p0..p3])] straight from the widgets."""
        stages = []
        for row in self.stage_list.get_children():
            parameters = [spin.get_value() for spin in row.spins]
            parameters += [0.0] * (phosphor_kit.PARAMETERS_PER_STAGE
                                   - len(parameters))
            stages.append((row.op_name, parameters))
        return stages

    def _apply_live(self):
        self.window.apply_kit_stages_live(self.current_stages())

    # ------------------------------------------------------------ buttons --

    def _on_response(self, _dialog, response):
        if response == 100:                  # (re)load the active kit
            for row in self.stage_list.get_children():
                self.stage_list.remove(row)
            self._load_active_kit()
            self._apply_live()
        elif response == 101:
            self._save()
        elif response in (Gtk.ResponseType.CLOSE,
                          Gtk.ResponseType.DELETE_EVENT):
            self._close()

    def _save(self):
        stages = self.current_stages()
        if not stages:
            self.window.status_label.set_text("kit is empty — add a stage")
            return
        name = self.name_entry.get_text().strip() or "untitled"
        author = self.author_entry.get_text().strip()
        file_name = "".join(ch if ch.isalnum() or ch in "-_" else "-"
                            for ch in name.casefold()).strip("-") or "kit"
        path = os.path.join(phosphor_kit.KIT_DIRECTORY,
                            f"{file_name}.phoskit")
        try:
            phosphor_kit.save(path, name, author, stages)
        except OSError as error:
            self.window.status_label.set_text(f"kit save failed: {error}")
            return
        window = self.window
        window.settings.kit_path = path
        window.settings.kit_enabled = True
        if author:
            window.settings.postcard_credit = author
        window.settings.save()
        window._refresh_kit_combo()
        window.kit_combo.set_active_id(path)
        window.kit_switch.set_active(True)
        window.status_label.set_text(f"kit saved: {path}")

    def _on_deleted(self, *_args):
        self._close()
        return False

    def _close(self):
        # editor previews die with the dialog; the saved settings win again
        self.window._kit_editor = None
        GLib.idle_add(self.window._apply_kit_to_computer, True)
        self.dialog.destroy()
