#!/usr/bin/env python3
"""Phosphor — a software XY oscilloscope for everything your PC plays.

In XY mode the left audio channel drives the beam horizontally and the
right channel drives it vertically, so "oscilloscope music"
(Jerobeam Fenderson and friends) draws its hidden pictures on screen.
Goniometer, waveform, and spectrum modes make ordinary music look good too.

Capture taps PulseAudio/PipeWire monitors — whole outputs, single
applications, or microphones — and costs nothing while toggled off.

Keys:  Space capture · M mini · S snapshot · C save clip · P pin
       G grid · scroll = gain (Ctrl+scroll in mini = resize) · Q quit
Mini mode: drag with the left button, double-click to restore,
right-click anywhere for the menu.
"""

import os
import threading

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GLib  # noqa: E402

import phosphor_recorder
from phosphor_audio import (AudioCaptureStream, default_monitor_target_id,
                            list_capture_targets)
from phosphor_render_cairo import CairoBeamCore
from phosphor_render_gl import GL_BINDINGS_AVAILABLE, GLBeamRenderer
from phosphor_settings import (CUSTOM_THEME_NAME, THEME_PRESETS, Settings)
from phosphor_signal import SegmentComputer

PROJECT_DIRECTORY = os.path.dirname(os.path.abspath(__file__))
QUIET_PEAK_THRESHOLD = 1e-4
QUIET_FRAMES_BEFORE_SLEEP = 120
MINI_SIZE_PRESETS = (("Small", 200), ("Medium", 280), ("Large", 380))

DISPLAY_MODES = (
    ("xy", "XY (scope art)"),
    ("xy45", "XY · goniometer"),
    ("waveform", "Waveform"),
    ("spectrum", "Spectrum"),
)


class LiveCairoRenderer(Gtk.DrawingArea):
    """CPU fallback renderer widget; same advance() interface as the GL one."""

    def __init__(self):
        super().__init__()
        self.core = CairoBeamCore()
        self.theme = None
        self.persistence = 0.7
        self.grid_enabled = True
        self.connect("draw", self._on_draw)

    def advance(self, segments):
        allocation = self.get_allocation()
        if allocation.width < 2 or allocation.height < 2:
            return
        self.core.ensure_size(allocation.width, allocation.height)
        self.core.advance(segments, self.persistence)
        self.queue_draw()

    def _on_draw(self, _widget, context):
        allocation = self.get_allocation()
        if self.theme is None:
            return False
        self.core.ensure_size(allocation.width, allocation.height)
        self.core.composite(context, allocation.width, allocation.height,
                            self.theme, self.grid_enabled)
        return False


class OscilloscopeWindow(Gtk.Window):
    def __init__(self):
        super().__init__(title="Phosphor")
        self.settings = Settings.load()
        self.set_default_size(self.settings.window_width, self.settings.window_height)

        icon_path = os.path.join(PROJECT_DIRECTORY, "phosphor-scope.svg")
        if os.path.exists(icon_path):
            try:
                self.set_icon_from_file(icon_path)
            except GLib.Error:
                pass

        self.segment_computer = SegmentComputer()
        self.segment_computer.mode = self.settings.display_mode
        self.segment_computer.gain = self.settings.gain
        self.segment_computer.beam_energy = self.settings.beam_energy

        self.capture_stream = AudioCaptureStream(
            on_stream_ended=lambda: GLib.idle_add(self._handle_stream_died))
        self.capture_targets = {}
        self.is_mini_mode = False
        self.tick_callback_id = None
        self.fade_out_frames_remaining = 0
        self.quiet_frame_count = 0
        self.exporting = False

        self._build_renderers()
        self._build_user_interface()
        self._apply_theme()

        self.connect("key-press-event", self._on_key_press)
        self.connect("delete-event", self._on_delete)
        self.connect("destroy", lambda _w: Gtk.main_quit())

    # ------------------------------------------------------------------ UI --

    def _build_renderers(self):
        self.cairo_renderer = LiveCairoRenderer()
        self.gl_available = GL_BINDINGS_AVAILABLE
        self.gl_renderer = GLBeamRenderer(on_failure=self._on_gl_failure) \
            if self.gl_available else None

        self.display_stack = Gtk.Stack()
        self.display_stack.add_named(self.cairo_renderer, "cairo")
        self.cairo_renderer.show()
        if self.gl_renderer is not None:
            self.display_stack.add_named(self.gl_renderer, "gl")
            self.gl_renderer.show()
        # children must be shown before set_visible_child_name has any effect
        if self.settings.renderer == "gl" and self.gl_renderer is not None:
            self.display_stack.set_visible_child_name("gl")
        else:
            self.settings.renderer = "cairo"
            self.display_stack.set_visible_child_name("cairo")

    def active_renderer(self):
        if self.settings.renderer == "gl" and self.gl_renderer is not None:
            return self.gl_renderer
        return self.cairo_renderer

    def _build_user_interface(self):
        layout = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.controls_container = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.controls_container.pack_start(self._build_main_toolbar_row(), False, False, 0)
        self.controls_container.pack_start(self._build_slider_toolbar_row(), False, False, 0)
        layout.pack_start(self.controls_container, False, False, 0)

        event_box = Gtk.EventBox()
        event_box.add_events(Gdk.EventMask.BUTTON_PRESS_MASK | Gdk.EventMask.SCROLL_MASK)
        event_box.connect("button-press-event", self._on_button_press)
        event_box.connect("scroll-event", self._on_scroll)
        event_box.add(self.display_stack)
        layout.pack_start(event_box, True, True, 0)
        self.add(layout)

    def _build_main_toolbar_row(self):
        row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        for edge in ("start", "end"):
            getattr(row, f"set_margin_{edge}")(6)
        row.set_margin_top(4)

        self.capture_toggle = Gtk.ToggleButton(label="⏻ Live")
        self.capture_toggle.set_tooltip_text("Toggle audio capture (Space). Off = zero CPU.")
        self.capture_toggle.connect("toggled", self._on_capture_toggled)
        row.pack_start(self.capture_toggle, False, False, 0)

        self.target_combo = Gtk.ComboBoxText()
        self.target_combo.set_tooltip_text(
            "What to scope: APP = one playing application, "
            "OUT = everything on that output, IN = microphones")
        self._populate_targets()
        self.target_combo.connect("changed", self._on_target_changed)
        row.pack_start(self.target_combo, False, False, 0)

        refresh_button = Gtk.Button.new_from_icon_name("view-refresh-symbolic",
                                                       Gtk.IconSize.BUTTON)
        refresh_button.set_tooltip_text("Re-scan devices and playing apps")
        refresh_button.connect("clicked", lambda _b: self._populate_targets())
        row.pack_start(refresh_button, False, False, 0)

        self.mode_combo = Gtk.ComboBoxText()
        for mode_id, mode_label in DISPLAY_MODES:
            self.mode_combo.append(mode_id, mode_label)
        self.mode_combo.set_active_id(self.settings.display_mode)
        self.mode_combo.connect("changed", self._on_mode_changed)
        row.pack_start(self.mode_combo, False, False, 0)

        self.pin_toggle = Gtk.ToggleButton(label="📌")
        self.pin_toggle.set_tooltip_text("Keep window above others (P)")
        self.pin_toggle.set_active(self.settings.pinned)
        self.pin_toggle.connect("toggled", self._on_pin_toggled)
        row.pack_start(self.pin_toggle, False, False, 0)

        snapshot_button = Gtk.Button(label="📷")
        snapshot_button.set_tooltip_text("Snapshot to ~/Pictures/Phosphor (S)")
        snapshot_button.connect("clicked", lambda _b: self.save_snapshot())
        row.pack_start(snapshot_button, False, False, 0)

        clip_button = Gtk.Button(label="⏺")
        clip_button.set_tooltip_text("Save the last 10 s as mp4 with sound (C)")
        clip_button.connect("clicked", lambda _b: self.save_clip())
        row.pack_start(clip_button, False, False, 0)

        row.pack_start(self._build_settings_button(), False, False, 0)

        mini_button = Gtk.Button(label="Mini")
        mini_button.set_tooltip_text(
            "Borderless always-on-top mini view (M).\n"
            "Drag to move, Ctrl+scroll to resize, double-click to restore.")
        mini_button.connect("clicked", lambda _b: self.set_mini_mode(True))
        row.pack_end(mini_button, False, False, 0)

        self.status_label = Gtk.Label(label="idle")
        self.status_label.set_ellipsize(3)  # Pango.EllipsizeMode.END
        self.status_label.set_xalign(1.0)
        row.pack_end(self.status_label, True, True, 0)
        return row

    def _build_slider_toolbar_row(self):
        row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        for edge in ("start", "end"):
            getattr(row, f"set_margin_{edge}")(6)
        row.set_margin_bottom(4)

        def add_slider(name, tooltip, low, high, initial, on_change):
            box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=4)
            box.pack_start(Gtk.Label(label=name), False, False, 0)
            scale = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL,
                                             low, high, (high - low) / 200)
            scale.set_value(initial)
            scale.set_draw_value(True)
            scale.set_value_pos(Gtk.PositionType.RIGHT)
            scale.connect("format-value",
                          lambda _s, value: f"{value / high * 100:3.0f}%")
            scale.set_size_request(150, -1)
            scale.set_tooltip_text(tooltip)
            scale.connect("value-changed", lambda widget: on_change(widget.get_value()))
            box.pack_start(scale, True, True, 0)
            row.pack_start(box, True, True, 0)
            return scale

        self.gain_scale = add_slider(
            "Gain", "Deflection scale (also mouse scroll)", 0.1, 6.0,
            self.settings.gain, self._on_gain_changed)
        add_slider(
            "Glow", "Phosphor persistence — how long trails linger", 0.0, 0.98,
            self.settings.persistence, self._on_persistence_changed)
        add_slider(
            "Beam", "Beam brightness budget — higher keeps fast strokes visible",
            1.0, 30.0, self.settings.beam_energy, self._on_beam_changed)
        return row

    def _build_settings_button(self):
        popover = Gtk.Popover()
        grid = Gtk.Grid(row_spacing=8, column_spacing=10)
        for edge in ("start", "end", "top", "bottom"):
            getattr(grid, f"set_margin_{edge}")(12)

        grid.attach(Gtk.Label(label="Renderer", xalign=0), 0, 0, 1, 1)
        self.renderer_combo = Gtk.ComboBoxText()
        if self.gl_available:
            self.renderer_combo.append("gl", "GPU · CRT beam (recommended)")
        self.renderer_combo.append("cairo", "CPU · cairo")
        self.renderer_combo.set_active_id(self.settings.renderer)
        self.renderer_combo.connect("changed", self._on_renderer_changed)
        grid.attach(self.renderer_combo, 1, 0, 1, 1)

        grid.attach(Gtk.Label(label="Theme", xalign=0), 0, 1, 1, 1)
        self.theme_combo = Gtk.ComboBoxText()
        for theme_name in list(THEME_PRESETS) + [CUSTOM_THEME_NAME]:
            self.theme_combo.append(theme_name, theme_name)
        self.theme_combo.set_active_id(self.settings.theme_name)
        self.theme_combo.connect("changed", self._on_theme_changed)
        grid.attach(self.theme_combo, 1, 1, 1, 1)

        def color_button(initial_rgb, on_set):
            rgba = Gdk.RGBA()
            rgba.red, rgba.green, rgba.blue, rgba.alpha = (*initial_rgb, 1.0)
            button = Gtk.ColorButton.new_with_rgba(rgba)
            def changed(widget):
                color = widget.get_rgba()
                on_set([color.red, color.green, color.blue])
                self._apply_theme()
            button.connect("color-set", changed)
            return button

        grid.attach(Gtk.Label(label="Custom beam", xalign=0), 0, 2, 1, 1)
        self.custom_beam_button = color_button(
            self.settings.custom_beam_color,
            lambda rgb: setattr(self.settings, "custom_beam_color", rgb))
        grid.attach(self.custom_beam_button, 1, 2, 1, 1)

        grid.attach(Gtk.Label(label="Custom grid", xalign=0), 0, 3, 1, 1)
        self.custom_grid_button = color_button(
            self.settings.custom_grid_color,
            lambda rgb: setattr(self.settings, "custom_grid_color", rgb))
        grid.attach(self.custom_grid_button, 1, 3, 1, 1)

        def add_switch(label, active, row_index, on_state):
            grid.attach(Gtk.Label(label=label, xalign=0), 0, row_index, 1, 1)
            switch = Gtk.Switch(halign=Gtk.Align.START)
            switch.set_active(active)
            switch.connect("state-set", on_state)
            grid.attach(switch, 1, row_index, 1, 1)
            return switch

        add_switch("Grid", self.settings.grid_enabled, 4, self._on_grid_switched)
        add_switch("AMOLED black", self.settings.amoled_background, 5,
                   self._on_amoled_switched)

        grid.show_all()
        popover.add(grid)
        self._update_custom_color_sensitivity()

        settings_button = Gtk.MenuButton(label="⚙")
        settings_button.set_tooltip_text("Renderer, theme, grid, AMOLED")
        settings_button.set_popover(popover)
        return settings_button

    def _update_custom_color_sensitivity(self):
        is_custom = self.settings.theme_name == CUSTOM_THEME_NAME
        self.custom_beam_button.set_sensitive(is_custom)
        self.custom_grid_button.set_sensitive(is_custom)

    # -------------------------------------------------------------- targets --

    def _populate_targets(self):
        previous_choice = self.target_combo.get_active_id() or self.settings.target_id
        self.target_combo.remove_all()
        self.capture_targets = {}
        for target in list_capture_targets():
            self.capture_targets[target.combo_id] = target
            self.target_combo.append(target.combo_id, target.label)
        for candidate in (previous_choice, default_monitor_target_id()):
            if candidate and candidate in self.capture_targets:
                self.target_combo.set_active_id(candidate)
                break
        if self.target_combo.get_active_id() is None:
            self.target_combo.set_active(0)

    def _on_target_changed(self, _combo):
        target_id = self.target_combo.get_active_id()
        if target_id is None:
            return
        self.settings.target_id = target_id
        if self.capture_stream.is_running:
            self.capture_stream.start(self.capture_targets[target_id])

    # -------------------------------------------------------------- capture --

    def _on_capture_toggled(self, toggle):
        if toggle.get_active():
            target_id = self.target_combo.get_active_id()
            if target_id is None or target_id not in self.capture_targets:
                toggle.set_active(False)
                return
            try:
                self.capture_stream.start(self.capture_targets[target_id])
            except OSError as error:
                self.status_label.set_text(f"capture failed: {error}")
                toggle.set_active(False)
                return
            self.quiet_frame_count = 0
            self.status_label.set_text("● live")
            self._start_render_loop()
        else:
            self.capture_stream.stop()
            self.status_label.set_text("idle — no capture, no CPU")
            self.fade_out_frames_remaining = 90

    def _handle_stream_died(self):
        if self.capture_toggle.get_active():
            self.capture_toggle.set_active(False)
            self.status_label.set_text("stream ended — app stopped or device gone")
            self._populate_targets()

    # --------------------------------------------------------- render loop --

    def _start_render_loop(self):
        if self.tick_callback_id is None:
            self.tick_callback_id = self.add_tick_callback(self._on_tick)

    def _on_tick(self, _widget, _frame_clock):
        allocation = self.display_stack.get_allocation()
        if allocation.width < 2 or allocation.height < 2:
            return GLib.SOURCE_CONTINUE

        if self.capture_stream.is_running:
            samples = self.capture_stream.take_stereo_samples()
            # The monitor delivers zeros while nothing plays; detect silence by
            # content and stop redrawing once the glow has settled.
            is_quiet = (not samples
                        or max(max(samples), -min(samples)) < QUIET_PEAK_THRESHOLD)
            self.quiet_frame_count = self.quiet_frame_count + 1 if is_quiet else 0
            if self.quiet_frame_count > QUIET_FRAMES_BEFORE_SLEEP:
                return GLib.SOURCE_CONTINUE
            segments = self.segment_computer.compute(
                samples, allocation.width, allocation.height)
            self.active_renderer().advance(segments)
        else:
            self.active_renderer().advance([])
            self.fade_out_frames_remaining -= 1
            if self.fade_out_frames_remaining <= 0:
                self.tick_callback_id = None
                return GLib.SOURCE_REMOVE
        return GLib.SOURCE_CONTINUE

    def _wake_renderer(self):
        """Repaint once after appearance changes, even while quiet/idle."""
        self.quiet_frame_count = 0
        renderer = self.active_renderer()
        if isinstance(renderer, LiveCairoRenderer):
            renderer.queue_draw()
        else:
            renderer.queue_render()

    # ----------------------------------------------------- settings changes --

    def _apply_theme(self):
        theme = self.settings.current_theme()
        for renderer in (self.cairo_renderer, self.gl_renderer):
            if renderer is None:
                continue
            renderer.theme = theme
            renderer.persistence = self.settings.persistence
            renderer.grid_enabled = self.settings.grid_enabled
        self._wake_renderer()

    def _on_gain_changed(self, value):
        self.settings.gain = value
        self.segment_computer.gain = value

    def _on_persistence_changed(self, value):
        self.settings.persistence = value
        self._apply_theme()

    def _on_beam_changed(self, value):
        self.settings.beam_energy = value
        self.segment_computer.beam_energy = value

    def _on_mode_changed(self, combo):
        mode = combo.get_active_id()
        if mode is None:
            return
        self.settings.display_mode = mode
        self.segment_computer.mode = mode
        self.segment_computer.reset()
        self.settings.save()

    def _on_theme_changed(self, combo):
        theme_name = combo.get_active_id()
        if theme_name is None:
            return
        self.settings.theme_name = theme_name
        self._update_custom_color_sensitivity()
        self._apply_theme()
        self.settings.save()

    def _on_renderer_changed(self, combo):
        renderer_id = combo.get_active_id()
        if renderer_id is None:
            return
        self.settings.renderer = renderer_id
        self.display_stack.set_visible_child_name(renderer_id)
        self._apply_theme()
        self.settings.save()

    def _on_grid_switched(self, _switch, state):
        self.settings.grid_enabled = state
        self._apply_theme()
        return False

    def _on_amoled_switched(self, _switch, state):
        self.settings.amoled_background = state
        self._apply_theme()
        return False

    def _on_pin_toggled(self, toggle):
        self.settings.pinned = toggle.get_active()
        if not self.is_mini_mode:
            self.set_keep_above(self.settings.pinned)

    def _on_gl_failure(self, message):
        self.gl_available = False
        self.settings.renderer = "cairo"
        self.display_stack.set_visible_child_name("cairo")
        if hasattr(self, "renderer_combo"):
            self.renderer_combo.set_active_id("cairo")
        self.status_label.set_text(f"GPU renderer unavailable, using CPU ({message})")
        self._apply_theme()

    # --------------------------------------------------------------- export --

    def _current_export_audio(self, seconds):
        audio = self.capture_stream.copy_history(seconds)
        if len(audio) < 48000:  # ~an eighth of a second
            self.status_label.set_text("nothing captured yet to export")
            return None
        return audio

    def save_snapshot(self):
        audio = self._current_export_audio(seconds=1.5)
        if audio is None or self.exporting:
            return
        def worker_done(path):
            self.exporting = False
            self.status_label.set_text(f"snapshot saved: {path}")
            return False
        def worker():
            try:
                path = phosphor_recorder.save_snapshot(audio, self.settings)
                GLib.idle_add(worker_done, path)
            except (RuntimeError, OSError) as error:
                GLib.idle_add(self._export_failed, str(error))
        self.exporting = True
        self.status_label.set_text("rendering snapshot…")
        threading.Thread(target=worker, daemon=True).start()

    def save_clip(self):
        audio = self._current_export_audio(seconds=10)
        if audio is None or self.exporting:
            return
        self.exporting = True
        self.status_label.set_text("rendering clip…")
        phosphor_recorder.save_clip_async(
            audio, self.settings,
            on_progress=lambda fraction: GLib.idle_add(
                self.status_label.set_text, f"rendering clip… {fraction * 100:.0f}%"),
            on_done=lambda path: GLib.idle_add(self._export_done, path),
            on_error=lambda message: GLib.idle_add(self._export_failed, message))

    def _export_done(self, path):
        self.exporting = False
        self.status_label.set_text(f"clip saved: {path}")
        return False

    def _export_failed(self, message):
        self.exporting = False
        self.status_label.set_text(f"export failed: {message}")
        return False

    # ------------------------------------------------------------ mini mode --

    def set_mini_mode(self, enabled):
        if enabled == self.is_mini_mode:
            return
        self.is_mini_mode = enabled
        if enabled:
            self.settings.window_width, self.settings.window_height = self.get_size()
            position = self.get_position()
            self.settings.window_x, self.settings.window_y = position
            self.controls_container.hide()
            self.set_decorated(False)
            self.set_keep_above(True)
            self.resize(self.settings.mini_size, self.settings.mini_size)
            if self.settings.mini_x is not None:
                self.move(self.settings.mini_x, self.settings.mini_y)
        else:
            position = self.get_position()
            self.settings.mini_x, self.settings.mini_y = position
            self.controls_container.show()
            self.set_decorated(True)
            self.set_keep_above(self.settings.pinned)
            self.resize(self.settings.window_width, self.settings.window_height)
            if self.settings.window_x is not None:
                self.move(self.settings.window_x, self.settings.window_y)
        self._wake_renderer()

    def _resize_mini(self, size):
        self.settings.mini_size = max(140, min(640, int(size)))
        if self.is_mini_mode:
            self.resize(self.settings.mini_size, self.settings.mini_size)

    # ----------------------------------------------------------------- input --

    def _show_context_menu(self, event):
        menu = Gtk.Menu()

        def add_item(label, callback, sensitive=True):
            item = Gtk.MenuItem(label=label)
            item.connect("activate", lambda _i: callback())
            item.set_sensitive(sensitive)
            menu.append(item)

        capturing = self.capture_toggle.get_active()
        add_item("Pause capture" if capturing else "Resume capture",
                 lambda: self.capture_toggle.set_active(not capturing))
        add_item("Snapshot  (S)", self.save_snapshot)
        add_item("Save last 10 s  (C)", self.save_clip)
        menu.append(Gtk.SeparatorMenuItem())
        if self.is_mini_mode:
            for preset_label, preset_size in MINI_SIZE_PRESETS:
                size = preset_size
                add_item(f"Mini size: {preset_label}",
                         lambda s=size: self._resize_mini(s))
            add_item("Restore window  (M)", lambda: self.set_mini_mode(False))
        else:
            add_item("Mini view  (M)", lambda: self.set_mini_mode(True))
        menu.append(Gtk.SeparatorMenuItem())
        add_item("Quit  (Q)", self.close)
        menu.show_all()
        menu.popup_at_pointer(event)

    def _on_button_press(self, _widget, event):
        if event.button == 3 and event.type == Gdk.EventType.BUTTON_PRESS:
            self._show_context_menu(event)
            return True
        if not self.is_mini_mode:
            return False
        if event.type == Gdk.EventType._2BUTTON_PRESS:
            self.set_mini_mode(False)
        elif event.type == Gdk.EventType.BUTTON_PRESS and event.button == 1:
            self.begin_move_drag(event.button, int(event.x_root),
                                 int(event.y_root), event.time)
        return True

    def _on_scroll(self, _widget, event):
        scroll_up = event.direction == Gdk.ScrollDirection.UP
        if self.is_mini_mode and event.state & Gdk.ModifierType.CONTROL_MASK:
            factor = 1.12 if scroll_up else 1 / 1.12
            self._resize_mini(self.settings.mini_size * factor)
            return True
        factor = 1.12 if scroll_up else 1 / 1.12
        self.gain_scale.set_value(min(6.0, max(0.1, self.settings.gain * factor)))
        return True

    def _on_key_press(self, _widget, event):
        key = event.keyval
        if key == Gdk.KEY_space:
            self.capture_toggle.set_active(not self.capture_toggle.get_active())
        elif key in (Gdk.KEY_m, Gdk.KEY_M):
            self.set_mini_mode(not self.is_mini_mode)
        elif key in (Gdk.KEY_s, Gdk.KEY_S):
            self.save_snapshot()
        elif key in (Gdk.KEY_c, Gdk.KEY_C):
            self.save_clip()
        elif key in (Gdk.KEY_p, Gdk.KEY_P):
            self.pin_toggle.set_active(not self.pin_toggle.get_active())
        elif key in (Gdk.KEY_g, Gdk.KEY_G):
            self.settings.grid_enabled = not self.settings.grid_enabled
            self._apply_theme()
        elif key == Gdk.KEY_Escape:
            if self.is_mini_mode:
                self.set_mini_mode(False)
            else:
                self.close()
        elif key in (Gdk.KEY_q, Gdk.KEY_Q):
            self.close()
        else:
            return False
        return True

    # ------------------------------------------------------------- lifecycle --

    def apply_remembered_view(self):
        """Restore where and how the window was last time."""
        if self.settings.start_in_mini:
            self.set_mini_mode(True)
        elif self.settings.window_x is not None:
            self.move(self.settings.window_x, self.settings.window_y)
        if self.settings.pinned and not self.is_mini_mode:
            self.set_keep_above(True)

    def _on_delete(self, _widget, _event):
        self.settings.start_in_mini = self.is_mini_mode
        position = self.get_position()
        if self.is_mini_mode:
            self.settings.mini_x, self.settings.mini_y = position
        else:
            self.settings.window_width, self.settings.window_height = self.get_size()
            self.settings.window_x, self.settings.window_y = position
        self.settings.save()
        self.capture_stream.stop()
        return False


def main():
    window = OscilloscopeWindow()
    window.show_all()
    window.apply_remembered_view()
    window.capture_toggle.set_active(True)  # start live; toggling off is free
    Gtk.main()


if __name__ == "__main__":
    main()
