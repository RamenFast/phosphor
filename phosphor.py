#!/usr/bin/env python3
"""Phosphor — a software XY oscilloscope for everything your PC plays.

In XY mode the left audio channel drives the beam horizontally and the
right channel drives it vertically, so "oscilloscope music"
(Jerobeam Fenderson and friends) draws its hidden pictures on screen.
Goniometer, waveform, and spectrum modes make ordinary music look good too.

Capture taps PulseAudio/PipeWire monitors — whole outputs, single
applications, or microphones — and costs nothing while toggled off.
Audio files can also be played directly (decoded by ffmpeg, audible
through pacat) so you can scope a track without a separate player.

Keys:  Space capture · O open file · M mini · S snapshot · C save clip
       P pin · G grid · F fps · scroll = gain (Ctrl+scroll in mini = resize)
       Q quit
Mini mode: drag with the left button, double-click to restore,
right-click anywhere for the menu.
"""

import os
import threading
import time

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GLib  # noqa: E402

import phosphor_recorder
from phosphor_audio import (AudioCaptureStream, default_monitor_target_id,
                            list_capture_targets)
from phosphor_render_cairo import CairoBeamCore
from phosphor_render_gl import GL_BINDINGS_AVAILABLE, GLBeamRenderer
from phosphor_settings import (CUSTOM_THEME_NAME, THEME_PRESETS, Settings,
                               grid_spacing_fraction)
from phosphor_signal import SegmentComputer

APPLICATION_ID = "io.github.ben.Phosphor"
APPLICATION_VERSION = "2.3.0"
PROJECT_DIRECTORY = os.path.dirname(os.path.abspath(__file__))
QUIET_PEAK_THRESHOLD = 1e-4
QUIET_FRAMES_BEFORE_SLEEP = 120
MINI_SIZE_PRESETS = (("Small", 200), ("Medium", 280), ("Large", 380))
AUDIO_FILE_EXTENSIONS = (".mp3", ".flac", ".ogg", ".oga", ".opus", ".wav",
                         ".m4a", ".aac", ".wma", ".aif", ".aiff", ".mka")
REACQUIRE_POLL_LIMIT = 180   # seconds to wait for a dead app stream to return

DISPLAY_MODES = (
    ("xy", "XY (scope art)"),
    ("xy45", "XY · goniometer"),
    ("xy_dots", "XY · dots"),
    ("waveform", "Waveform"),
    ("spectrum", "Spectrum"),
    ("spectrum_radial", "Spectrum · radial"),
)

GPU_QUALITY_CHOICES = (("1", "Standard"), ("2", "High · 2× supersampled"),
                       ("3", "Ultra · 3× supersampled"))
CPU_RESOLUTION_CHOICES = (("1.0", "Full resolution"),
                          ("0.75", "Balanced · 75%"),
                          ("0.5", "Fast · 50%"))
UI_STYLE_CHOICES = (("system", "System"), ("dark", "Dark"),
                    ("light", "Light"), ("black", "AMOLED pink"))

# Always loaded: just the FPS overlay chip.
BASE_UI_CSS = b"""
#fps-overlay {
    background-color: rgba(0, 0, 0, 0.55);
    color: #7dff9e;
    padding: 2px 8px;
    border-radius: 6px;
    font-family: monospace;
    font-size: 11px;
}
"""

# AMOLED UI style: pure-black window, soft multi-shade pinks (filled
# controls, not hollow outlines), warm yellow for anything selected/active.
BLACK_UI_CSS = b"""
window, headerbar, popover, popover.background, menu, .background {
    background-color: #000000;
    color: #f2aed8;
}
label { color: #f2aed8; }
headerbar {
    min-height: 32px;
    padding: 0 4px;
    box-shadow: none;
    border-bottom: 1px solid #1d0916;
}
headerbar .title { color: #fbcfe8; }
button {
    background-image: none;
    background-color: #1a0713;
    color: #f2aed8;
    border: 1px solid #57203f;
    border-radius: 6px;
    padding: 1px 8px;
    min-height: 22px;
}
button:hover { background-color: #2b0d20; border-color: #b65c92; }
button:active { background-color: #3c142d; }
button:checked {
    background-color: #2b2208;
    color: #ffdf87;
    border-color: #97772b;
}
button:checked label, button:checked image { color: #ffdf87; }
entry, spinbutton, spinbutton entry {
    background-image: none;
    background-color: #120510;
    color: #ffdf87;
    border: 1px solid #57203f;
    min-height: 20px;
}
combobox button.combo { padding: 1px 6px; }
scale trough { background-color: #2b0d20; border: 1px solid #57203f; }
scale highlight { background-color: #e078b8; }
scale slider { background-color: #fbcfe8; border: 1px solid #b65c92; }
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
menu, popover { border: 1px solid #2b0d20; }
#fps-overlay { color: #ffdf87; }
"""

# Light UI style: bright neutral chrome with a blue accent.
LIGHT_UI_CSS = b"""
window, headerbar, popover, popover.background, menu, .background {
    background-color: #fafafa;
    color: #303030;
}
label { color: #303030; }
headerbar {
    background-image: none;
    background-color: #f0f0f0;
    min-height: 32px;
    padding: 0 4px;
    box-shadow: none;
    border-bottom: 1px solid #d8d8d8;
}
button {
    background-image: none;
    background-color: #ffffff;
    color: #303030;
    border: 1px solid #c9c9c9;
    border-radius: 6px;
    padding: 1px 8px;
    min-height: 22px;
}
button:hover { border-color: #8f8f8f; }
button:checked {
    background-color: #dceaff;
    color: #1c4e9e;
    border-color: #7aa7e0;
}
button:checked label, button:checked image { color: #1c4e9e; }
entry, spinbutton, spinbutton entry {
    background-image: none;
    background-color: #ffffff;
    color: #222222;
    border: 1px solid #c9c9c9;
    min-height: 20px;
}
scale trough { background-color: #e4e4e4; border: 1px solid #cfcfcf; }
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
#fps-overlay { background-color: rgba(255, 255, 255, 0.75); color: #1c4e9e; }
"""


class LiveCairoRenderer(Gtk.DrawingArea):
    """CPU renderer widget; signal math and phosphor decay run on a worker
    thread with a latest-frame mailbox, so a slow frame drops instead of
    bogging the UI down."""

    def __init__(self, segment_computer, compute_lock):
        super().__init__()
        self.core = CairoBeamCore()
        self.segment_computer = segment_computer
        self.compute_lock = compute_lock
        self.theme = None
        self.persistence = 0.7
        self.grid_enabled = True
        self.grid_spacing_fraction = 0.1125
        self.resolution = 1.0
        self.beam_focus = 1.6
        self._core_lock = threading.Lock()
        self._mailbox = None               # only the newest frame survives
        self._mailbox_condition = threading.Condition()
        worker = threading.Thread(target=self._worker_loop, daemon=True)
        worker.start()
        self.connect("draw", self._on_draw)

    def advance(self, samples):
        """Queue this frame's samples; the worker does the heavy lifting."""
        allocation = self.get_allocation()
        if allocation.width < 2 or allocation.height < 2:
            return
        with self._mailbox_condition:
            self._mailbox = (samples, allocation.width, allocation.height,
                             self.persistence, self.resolution, self.beam_focus)
            self._mailbox_condition.notify()

    def _worker_loop(self):
        while True:
            with self._mailbox_condition:
                while self._mailbox is None:
                    self._mailbox_condition.wait()
                (samples, width, height,
                 persistence, resolution, beam_focus) = self._mailbox
                self._mailbox = None
            with self.compute_lock:
                segments = self.segment_computer.compute(samples, width, height)
            with self._core_lock:
                self.core.beam_focus = beam_focus
                self.core.ensure_size(width, height, resolution)
                self.core.advance(segments, persistence)
            GLib.idle_add(self.queue_draw)

    def _on_draw(self, _widget, context):
        allocation = self.get_allocation()
        if self.theme is None:
            return False
        with self._core_lock:
            self.core.ensure_size(allocation.width, allocation.height,
                                  self.resolution)
            self.core.composite(context, allocation.width, allocation.height,
                                self.theme, self.grid_enabled,
                                self.grid_spacing_fraction)
        return False


class OscilloscopeWindow(Gtk.ApplicationWindow):
    def __init__(self, application):
        super().__init__(application=application, title="Phosphor")
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
        self.compute_lock = threading.Lock()

        self.capture_stream = AudioCaptureStream(
            on_stream_ended=lambda: GLib.idle_add(self._handle_stream_died))
        self.capture_targets = {}
        self.playing_file = None
        self.playlist = []
        self.playlist_index = -1
        self.is_mini_mode = False
        self.tick_callback_id = None
        self.fade_out_frames_remaining = 0
        self.quiet_frame_count = 0
        self.exporting = False
        self._style_css_provider = None
        self._reacquire_target_id = None
        self._reacquire_source = None
        self._reacquire_attempts = 0
        self._geometry_apply_source = None
        # suspended until the remembered view is applied, so startup configure
        # events can't overwrite the saved geometry with the WM's placement
        self._geometry_tracking_suspended = True
        self._is_fullscreen = False
        self._last_frame_time = 0.0
        self._fps_counter = 0
        self._fps_window_start = time.monotonic()

        base_css_provider = Gtk.CssProvider()
        base_css_provider.load_from_data(BASE_UI_CSS)
        Gtk.StyleContext.add_provider_for_screen(
            Gdk.Screen.get_default(), base_css_provider,
            Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

        self._build_renderers()
        self._build_user_interface()
        self._apply_theme()
        self._apply_render_quality()
        self._apply_grid_geometry()
        self._apply_ui_style()

        self.connect("key-press-event", self._on_key_press)
        self.connect("configure-event", self._on_configure_event)
        self.connect("window-state-event", self._on_window_state_event)
        self.connect("delete-event", self._on_delete)

    # ------------------------------------------------------------------ UI --

    def _build_renderers(self):
        self.cairo_renderer = LiveCairoRenderer(self.segment_computer,
                                                self.compute_lock)
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
        self._build_header_bar()

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

        self.fps_label = Gtk.Label(label="… fps")
        self.fps_label.set_name("fps-overlay")
        self.fps_label.set_halign(Gtk.Align.END)
        self.fps_label.set_valign(Gtk.Align.START)
        for edge in ("top", "end"):
            getattr(self.fps_label, f"set_margin_{edge}")(10)
        self.fps_label.set_no_show_all(True)
        self.fps_label.set_visible(self.settings.show_fps)

        display_overlay = Gtk.Overlay()
        display_overlay.add(event_box)
        display_overlay.add_overlay(self.fps_label)
        layout.pack_start(display_overlay, True, True, 0)
        self.add(layout)

    def _build_header_bar(self):
        self.header_bar = Gtk.HeaderBar()
        self.header_bar.set_show_close_button(True)
        self.header_bar.set_title("Phosphor")
        self.set_titlebar(self.header_bar)

        open_button = Gtk.Button.new_from_icon_name("document-open-symbolic",
                                                    Gtk.IconSize.BUTTON)
        open_button.set_tooltip_text("Play an audio file on the scope (O)")
        open_button.connect("clicked", lambda _b: self.open_audio_file())
        self.header_bar.pack_start(open_button)

        # transport appears only while a file is loaded
        self.transport_box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL)
        self.transport_box.get_style_context().add_class("linked")
        for icon_name, tooltip, callback in (
                ("media-skip-backward-symbolic", "Previous track in folder",
                 self.play_previous_track),
                (None, "Play/pause the loaded file", self._on_transport_play_pause),
                ("media-skip-forward-symbolic", "Next track in folder",
                 self.play_next_track)):
            if icon_name is None:
                self.play_pause_image = Gtk.Image.new_from_icon_name(
                    "media-playback-pause-symbolic", Gtk.IconSize.BUTTON)
                button = Gtk.Button()
                button.add(self.play_pause_image)
            else:
                button = Gtk.Button.new_from_icon_name(icon_name,
                                                       Gtk.IconSize.BUTTON)
            button.set_tooltip_text(tooltip)
            button.connect("clicked", lambda _b, c=callback: c())
            self.transport_box.pack_start(button, False, False, 0)
        self.transport_box.set_no_show_all(True)
        self.header_bar.pack_start(self.transport_box)

        settings_button = self._build_settings_button()
        self.header_bar.pack_end(settings_button)

        self.pin_toggle = Gtk.ToggleButton()
        self.pin_toggle.add(Gtk.Image.new_from_icon_name("view-pin-symbolic",
                                                         Gtk.IconSize.BUTTON))
        self.pin_toggle.set_tooltip_text("Keep window above others (P)")
        self.pin_toggle.set_active(self.settings.pinned)
        self.pin_toggle.connect("toggled", self._on_pin_toggled)
        self.header_bar.pack_end(self.pin_toggle)

        mini_button = Gtk.Button.new_from_icon_name("view-restore-symbolic",
                                                    Gtk.IconSize.BUTTON)
        mini_button.set_tooltip_text(
            "Borderless always-on-top mini view (M).\n"
            "Drag to move, Ctrl+scroll to resize, double-click to restore.")
        mini_button.connect("clicked", lambda _b: self.set_mini_mode(True))
        self.header_bar.pack_end(mini_button)

    def _build_main_toolbar_row(self):
        row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        for edge in ("start", "end"):
            getattr(row, f"set_margin_{edge}")(6)
        row.set_margin_top(4)

        self.capture_toggle = Gtk.ToggleButton(label="⏻ Live")
        self.capture_toggle.set_tooltip_text("Toggle audio capture (Space). Off = zero CPU.")
        self.capture_toggle.connect("toggled", self._on_capture_toggled)
        row.pack_start(self.capture_toggle, False, False, 0)

        # status sits right beside the Live button so the capture state and
        # its explanation read as one unit
        self.status_label = Gtk.Label(label="idle")
        self.status_label.set_ellipsize(3)  # Pango.EllipsizeMode.END
        self.status_label.set_xalign(0.0)
        self.status_label.set_max_width_chars(34)
        row.pack_start(self.status_label, True, True, 0)

        clip_button = Gtk.Button(label="⏺")
        clip_button.set_tooltip_text("Save the last 10 s as mp4 with sound (C)")
        clip_button.connect("clicked", lambda _b: self.save_clip())
        row.pack_end(clip_button, False, False, 0)

        snapshot_button = Gtk.Button(label="📷")
        snapshot_button.set_tooltip_text("Snapshot to ~/Pictures/Phosphor (S)")
        snapshot_button.connect("clicked", lambda _b: self.save_snapshot())
        row.pack_end(snapshot_button, False, False, 0)

        self.mode_combo = Gtk.ComboBoxText()
        for mode_id, mode_label in DISPLAY_MODES:
            self.mode_combo.append(mode_id, mode_label)
        self.mode_combo.set_active_id(self.settings.display_mode)
        self.mode_combo.connect("changed", self._on_mode_changed)
        row.pack_end(self.mode_combo, False, False, 0)

        refresh_button = Gtk.Button.new_from_icon_name("view-refresh-symbolic",
                                                       Gtk.IconSize.BUTTON)
        refresh_button.set_tooltip_text("Re-scan devices and playing apps")
        refresh_button.connect("clicked", lambda _b: self._populate_targets())
        row.pack_end(refresh_button, False, False, 0)

        self.target_combo = Gtk.ComboBoxText()
        self.target_combo.set_tooltip_text(
            "What to scope: APP = one playing application, "
            "OUT = everything on that output, IN = microphones")
        self._populate_targets()
        self.target_combo.connect("changed", self._on_target_changed)
        row.pack_end(self.target_combo, False, False, 0)

        self.target_kind_icon = Gtk.Image.new_from_icon_name(
            "audio-speakers-symbolic", Gtk.IconSize.BUTTON)
        row.pack_end(self.target_kind_icon, False, False, 0)
        self._update_target_kind_icon()
        return row

    def _update_target_kind_icon(self):
        target_id = self.target_combo.get_active_id() or ""
        if target_id.startswith("app:"):
            icon_name = "audio-x-generic-symbolic"
        elif target_id.endswith(".monitor"):
            icon_name = "audio-speakers-symbolic"
        else:
            icon_name = "audio-input-microphone-symbolic"
        self.target_kind_icon.set_from_icon_name(icon_name, Gtk.IconSize.BUTTON)

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
            scale.set_draw_value(False)
            scale.set_size_request(130, -1)
            scale.set_tooltip_text(tooltip)
            # the percent box is editable: click and type an exact value
            percent_spin = Gtk.SpinButton.new_with_range(low / high * 100, 100, 1)
            percent_spin.set_digits(0)
            percent_spin.set_width_chars(3)
            percent_spin.set_tooltip_text(f"{name} as percent — type a value")
            syncing = {"busy": False}

            def scale_changed(widget):
                value = widget.get_value()
                if not syncing["busy"]:
                    syncing["busy"] = True
                    percent_spin.set_value(value / high * 100)
                    syncing["busy"] = False
                on_change(value)

            def spin_changed(widget):
                if syncing["busy"]:
                    return
                syncing["busy"] = True
                scale.set_value(widget.get_value() / 100 * high)
                syncing["busy"] = False
                on_change(scale.get_value())

            scale.connect("value-changed", scale_changed)
            percent_spin.connect("value-changed", spin_changed)
            percent_spin.set_value(initial / high * 100)
            box.pack_start(scale, True, True, 0)
            box.pack_start(percent_spin, False, False, 0)
            box.pack_start(Gtk.Label(label="%"), False, False, 0)
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
        next_row = [0]

        def attach(label, widget):
            grid.attach(Gtk.Label(label=label, xalign=0), 0, next_row[0], 1, 1)
            grid.attach(widget, 1, next_row[0], 1, 1)
            next_row[0] += 1
            return widget

        def combo(choices, active_id, on_changed):
            box = Gtk.ComboBoxText()
            for choice_id, choice_label in choices:
                box.append(choice_id, choice_label)
            box.set_active_id(active_id)
            box.connect("changed", on_changed)
            return box

        self.renderer_combo = Gtk.ComboBoxText()
        if self.gl_available:
            self.renderer_combo.append("gl", "GPU · CRT beam (recommended)")
        self.renderer_combo.append("cairo", "CPU · cairo")
        self.renderer_combo.set_active_id(self.settings.renderer)
        self.renderer_combo.connect("changed", self._on_renderer_changed)
        attach("Renderer", self.renderer_combo)

        attach("GPU quality", combo(GPU_QUALITY_CHOICES,
                                    str(self.settings.gl_supersample),
                                    self._on_gpu_quality_changed))
        resolution_id = min(
            (choice_id for choice_id, _ in CPU_RESOLUTION_CHOICES),
            key=lambda choice_id:
                abs(float(choice_id) - self.settings.cairo_resolution))
        attach("CPU resolution", combo(CPU_RESOLUTION_CHOICES, resolution_id,
                                       self._on_cpu_resolution_changed))

        focus_scale = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL,
                                               0.6, 3.0, 0.1)
        focus_scale.set_value(self.settings.beam_focus)
        focus_scale.set_draw_value(False)
        focus_scale.set_size_request(140, -1)
        focus_scale.set_tooltip_text(
            "Beam focus — narrower keeps dense scenes from washing out")
        focus_scale.connect("value-changed", self._on_focus_changed)
        attach("Focus", focus_scale)

        self.theme_combo = Gtk.ComboBoxText()
        for theme_name in list(THEME_PRESETS) + [CUSTOM_THEME_NAME]:
            self.theme_combo.append(theme_name, theme_name)
        self.theme_combo.set_active_id(self.settings.theme_name)
        self.theme_combo.connect("changed", self._on_theme_changed)
        attach("Theme", self.theme_combo)

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

        self.custom_beam_button = attach("Custom beam", color_button(
            self.settings.custom_beam_color,
            lambda rgb: setattr(self.settings, "custom_beam_color", rgb)))
        self.custom_grid_button = attach("Custom grid", color_button(
            self.settings.custom_grid_color,
            lambda rgb: setattr(self.settings, "custom_grid_color", rgb)))

        def switch(active, on_state):
            widget = Gtk.Switch(halign=Gtk.Align.START)
            widget.set_active(active)
            widget.connect("state-set", on_state)
            return widget

        self.grid_switch = attach("Grid", switch(self.settings.grid_enabled,
                                                 self._on_grid_switched))
        attach("AMOLED scope", switch(self.settings.amoled_background,
                                      self._on_amoled_switched))
        attach("UI style", combo(UI_STYLE_CHOICES, self.settings.ui_style,
                                 self._on_ui_style_changed))
        attach("Pin button", switch(self.settings.show_pin_button,
                                    self._on_show_pin_switched))
        self.show_fps_switch = attach("Show FPS", switch(
            self.settings.show_fps, self._on_show_fps_switched))
        max_fps_spin = Gtk.SpinButton.new_with_range(0, 240, 5)
        max_fps_spin.set_value(self.settings.max_fps)
        max_fps_spin.set_tooltip_text(
            "Frame rate cap — 0 follows the monitor refresh rate (165 Hz "
            "monitor = 165 fps); lower it to trade smoothness for power")
        max_fps_spin.connect("value-changed", self._on_max_fps_changed)
        attach("Max FPS", max_fps_spin)

        grid.show_all()
        popover.add(grid)
        self._update_custom_color_sensitivity()

        settings_button = Gtk.MenuButton()
        settings_button.add(Gtk.Image.new_from_icon_name(
            "emblem-system-symbolic", Gtk.IconSize.BUTTON))
        settings_button.set_tooltip_text("Renderer, quality, theme, UI style")
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
        self._cancel_reacquire()
        self.settings.target_id = target_id
        self._update_target_kind_icon()
        if self.capture_stream.is_running and self.playing_file is None:
            self.capture_stream.start(self.capture_targets[target_id])

    # -------------------------------------------------------------- capture --

    def _on_capture_toggled(self, toggle):
        self._cancel_reacquire()
        if toggle.get_active():
            self._set_file_loaded(False)
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
            self._set_file_loaded(False)
            self.status_label.set_text("idle — no capture, no CPU")
            self.fade_out_frames_remaining = 90

    def open_audio_file(self):
        dialog = Gtk.FileChooserNative.new("Play audio file", self,
                                           Gtk.FileChooserAction.OPEN,
                                           "_Play", "_Cancel")
        audio_filter = Gtk.FileFilter()
        audio_filter.set_name("Audio files")
        audio_filter.add_mime_type("audio/*")
        dialog.add_filter(audio_filter)
        everything_filter = Gtk.FileFilter()
        everything_filter.set_name("All files")
        everything_filter.add_pattern("*")
        dialog.add_filter(everything_filter)
        if dialog.run() == Gtk.ResponseType.ACCEPT:
            path = dialog.get_filename()
            if path:
                self.play_file(path)
        dialog.destroy()

    def play_file(self, path, rebuild_playlist=True):
        self._cancel_reacquire()
        try:
            self.capture_stream.start_file(path)
        except OSError as error:
            self.status_label.set_text(
                f"file playback failed: {error} (is ffmpeg installed?)")
            return
        if rebuild_playlist:
            self._build_playlist(path)
        else:
            self.playlist_index = self.playlist.index(path)
        self.playing_file = os.path.basename(path)
        self._set_file_loaded(True)
        # reflect "running" in the toggle without re-triggering device capture
        self.capture_toggle.handler_block_by_func(self._on_capture_toggled)
        self.capture_toggle.set_active(True)
        self.capture_toggle.handler_unblock_by_func(self._on_capture_toggled)
        self.quiet_frame_count = 0
        self.status_label.set_text(f"▶ {self.playing_file}")
        self._start_render_loop()

    def _build_playlist(self, path):
        """Folder-based discovery: every audio file beside the opened one."""
        directory = os.path.dirname(path)
        try:
            names = sorted(os.listdir(directory), key=str.casefold)
        except OSError:
            names = []
        self.playlist = [os.path.join(directory, name) for name in names
                         if name.lower().endswith(AUDIO_FILE_EXTENSIONS)]
        if path not in self.playlist:
            self.playlist.insert(0, path)
        self.playlist_index = self.playlist.index(path)

    def _set_file_loaded(self, loaded):
        if not loaded:
            self.playing_file = None
        self.transport_box.set_no_show_all(not loaded)
        if loaded:
            self.transport_box.show_all()
            self.play_pause_image.set_from_icon_name(
                "media-playback-pause-symbolic", Gtk.IconSize.BUTTON)
        else:
            self.transport_box.hide()

    def _step_playlist(self, step):
        if not self.playlist:
            return
        index = (self.playlist_index + step) % len(self.playlist)
        self.play_file(self.playlist[index], rebuild_playlist=False)

    def play_next_track(self):
        self._step_playlist(1)

    def play_previous_track(self):
        self._step_playlist(-1)

    def _on_transport_play_pause(self):
        if self.playing_file is None:
            return
        paused = not self.capture_stream.playback_paused
        self.capture_stream.set_playback_paused(paused)
        self.play_pause_image.set_from_icon_name(
            "media-playback-start-symbolic" if paused
            else "media-playback-pause-symbolic", Gtk.IconSize.BUTTON)
        self.status_label.set_text(
            f"{'⏸' if paused else '▶'} {self.playing_file}")
        if not paused:
            self.quiet_frame_count = 0
            self._start_render_loop()

    def _handle_stream_died(self):
        if not self.capture_toggle.get_active():
            return
        finished_file = self.playing_file
        died_target_id = self.settings.target_id or ""
        self.capture_toggle.handler_block_by_func(self._on_capture_toggled)
        self.capture_toggle.set_active(False)
        self.capture_toggle.handler_unblock_by_func(self._on_capture_toggled)
        self.capture_stream.stop()
        self.fade_out_frames_remaining = 90
        if finished_file is not None:
            # natural end of a track: keep the album going
            if self.playlist and self.playlist_index < len(self.playlist) - 1:
                self.play_next_track()
            else:
                self._set_file_loaded(False)
                self.status_label.set_text(f"finished: {finished_file}")
        elif died_target_id.startswith("app:"):
            # app streams die between every song; wait for the app to play
            # again instead of dumping the user onto another source
            self._begin_app_reacquire(died_target_id)
        else:
            self.status_label.set_text("stream ended — app stopped or device gone")
            self._populate_targets()

    # ----------------------------------------------------- app reacquire --

    def _begin_app_reacquire(self, combo_id):
        self._cancel_reacquire()
        self._reacquire_target_id = combo_id
        self._reacquire_attempts = 0
        application_name = combo_id.split(":", 1)[1]
        self.status_label.set_text(f"waiting for {application_name} to play again…")
        self._reacquire_source = GLib.timeout_add_seconds(1, self._poll_reacquire)

    def _cancel_reacquire(self):
        if self._reacquire_source is not None:
            GLib.source_remove(self._reacquire_source)
            self._reacquire_source = None

    def _poll_reacquire(self):
        self._reacquire_attempts += 1
        if self._reacquire_attempts > REACQUIRE_POLL_LIMIT:
            self._reacquire_source = None
            self.status_label.set_text("idle — no capture, no CPU")
            return GLib.SOURCE_REMOVE
        if any(target.combo_id == self._reacquire_target_id
               for target in list_capture_targets()):
            self._reacquire_source = None
            self._populate_targets()
            self.target_combo.set_active_id(self._reacquire_target_id)
            self.capture_toggle.set_active(True)
            return GLib.SOURCE_REMOVE
        return GLib.SOURCE_CONTINUE

    # --------------------------------------------------------- render loop --

    def _start_render_loop(self):
        if self.tick_callback_id is None:
            self.tick_callback_id = self.add_tick_callback(self._on_tick)

    def _on_tick(self, _widget, _frame_clock):
        allocation = self.display_stack.get_allocation()
        if allocation.width < 2 or allocation.height < 2:
            return GLib.SOURCE_CONTINUE

        now = time.monotonic()
        if self.settings.max_fps > 0:
            # skipped frames leave their samples queued, so the next drawn
            # frame simply traces more audio — nothing is lost to the cap
            if now - self._last_frame_time < 1.0 / self.settings.max_fps - 5e-4:
                return GLib.SOURCE_CONTINUE
        self._last_frame_time = now
        self._count_fps_frame(now)

        if self.capture_stream.is_running:
            samples = self.capture_stream.take_stereo_samples()
            # The monitor delivers zeros while nothing plays; detect silence by
            # content and stop redrawing once the glow has settled.
            is_quiet = (not samples
                        or max(max(samples), -min(samples)) < QUIET_PEAK_THRESHOLD)
            self.quiet_frame_count = self.quiet_frame_count + 1 if is_quiet else 0
            if self.quiet_frame_count > QUIET_FRAMES_BEFORE_SLEEP:
                return GLib.SOURCE_CONTINUE
            self._advance_active_renderer(samples, allocation)
        else:
            self._advance_active_renderer([], allocation)
            self.fade_out_frames_remaining -= 1
            if self.fade_out_frames_remaining <= 0:
                self.tick_callback_id = None
                return GLib.SOURCE_REMOVE
        return GLib.SOURCE_CONTINUE

    def _count_fps_frame(self, now):
        if not self.settings.show_fps:
            return
        self._fps_counter += 1
        elapsed = now - self._fps_window_start
        if elapsed >= 0.5:
            self.fps_label.set_text(f"{self._fps_counter / elapsed:.0f} fps")
            self._fps_counter = 0
            self._fps_window_start = now

    def _advance_active_renderer(self, samples, allocation):
        renderer = self.active_renderer()
        if isinstance(renderer, LiveCairoRenderer):
            renderer.advance(samples)      # worker thread computes + decays
        else:
            with self.compute_lock:
                segments = self.segment_computer.compute(
                    samples, allocation.width, allocation.height)
            renderer.advance(segments)

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
        # the signal layer pre-decays intensities by in-frame age using the
        # same per-frame glow keep the renderers apply
        self.segment_computer.frame_glow_keep = \
            1.0 - max(0.02, (1.0 - self.settings.persistence) * 0.6)
        self._wake_renderer()

    def _apply_render_quality(self):
        if self.gl_renderer is not None:
            self.gl_renderer.supersample = self.settings.gl_supersample
            self.gl_renderer.beam_focus = self.settings.beam_focus
        self.cairo_renderer.resolution = self.settings.cairo_resolution
        self.cairo_renderer.beam_focus = self.settings.beam_focus
        self._wake_renderer()

    def _apply_grid_geometry(self):
        fraction = grid_spacing_fraction(self.settings.gain)
        if self.gl_renderer is not None:
            self.gl_renderer.grid_spacing_fraction = fraction
        self.cairo_renderer.grid_spacing_fraction = fraction
        self._wake_renderer()

    def _apply_ui_style(self):
        style = self.settings.ui_style
        Gtk.Settings.get_default().set_property(
            "gtk-application-prefer-dark-theme", style in ("dark", "black"))
        screen = Gdk.Screen.get_default()
        if self._style_css_provider is not None:
            Gtk.StyleContext.remove_provider_for_screen(
                screen, self._style_css_provider)
            self._style_css_provider = None
        style_css = {"black": BLACK_UI_CSS, "light": LIGHT_UI_CSS}.get(style)
        if style_css is not None:
            self._style_css_provider = Gtk.CssProvider()
            self._style_css_provider.load_from_data(style_css)
            Gtk.StyleContext.add_provider_for_screen(
                screen, self._style_css_provider,
                Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)

    def _on_gain_changed(self, value):
        self.settings.gain = value
        self.segment_computer.gain = value
        self._apply_grid_geometry()

    def _on_persistence_changed(self, value):
        self.settings.persistence = value
        self._apply_theme()

    def _on_beam_changed(self, value):
        self.settings.beam_energy = value
        self.segment_computer.beam_energy = value

    def _on_focus_changed(self, scale):
        self.settings.beam_focus = scale.get_value()
        self._apply_render_quality()

    def _on_mode_changed(self, combo):
        mode = combo.get_active_id()
        if mode is None:
            return
        self.settings.display_mode = mode
        with self.compute_lock:
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

    def _on_gpu_quality_changed(self, combo):
        if combo.get_active_id() is None:
            return
        self.settings.gl_supersample = int(combo.get_active_id())
        self._apply_render_quality()
        self.settings.save()

    def _on_cpu_resolution_changed(self, combo):
        if combo.get_active_id() is None:
            return
        self.settings.cairo_resolution = float(combo.get_active_id())
        self._apply_render_quality()
        self.settings.save()

    def _on_ui_style_changed(self, combo):
        if combo.get_active_id() is None:
            return
        self.settings.ui_style = combo.get_active_id()
        self._apply_ui_style()
        self.settings.save()

    def _on_show_pin_switched(self, _switch, state):
        self.settings.show_pin_button = state
        self.pin_toggle.set_visible(state)
        return False

    def _on_show_fps_switched(self, _switch, state):
        self.settings.show_fps = state
        self.fps_label.set_visible(state)
        self._fps_counter = 0
        self._fps_window_start = time.monotonic()
        return False

    def _on_max_fps_changed(self, spin):
        self.settings.max_fps = int(spin.get_value())
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

    def _window_is_tiled_or_maximized(self):
        gdk_window = self.get_window()
        if gdk_window is None:
            return False
        state = gdk_window.get_state()
        return bool(state & (Gdk.WindowState.MAXIMIZED
                             | Gdk.WindowState.TILED
                             | Gdk.WindowState.FULLSCREEN))

    def _on_configure_event(self, _widget, _event):
        """Continuously remember the normal-view geometry. Reading it lazily
        at toggle time raced the window manager (and rapid M presses), which
        is what used to restore half-screen tile sizes."""
        if (self.is_mini_mode or self._geometry_tracking_suspended
                or self._window_is_tiled_or_maximized()):
            return False
        if self.get_window() is not None:
            self.settings.window_width, self.settings.window_height = self.get_size()
            self.settings.window_x, self.settings.window_y = self.get_position()
        return False

    def _resume_geometry_tracking(self):
        self._geometry_tracking_suspended = False
        return False

    # --------------------------------------------------------- fullscreen --

    def toggle_fullscreen(self):
        """Chrome-less fullscreen scope. Side benefit: compositors hand
        fullscreen windows the direct scanout path, so the full monitor
        refresh rate is reachable without the GL-compositing overhead."""
        if self._is_fullscreen:
            self.unfullscreen()
        else:
            self.fullscreen()

    def _on_window_state_event(self, _widget, event):
        fullscreen = bool(event.new_window_state & Gdk.WindowState.FULLSCREEN)
        if fullscreen == self._is_fullscreen:
            return False
        self._is_fullscreen = fullscreen
        if not self.is_mini_mode:
            if fullscreen:
                self.controls_container.hide()
                self.header_bar.hide()
            else:
                self.controls_container.show()
                self.header_bar.show()
        return False

    def set_mini_mode(self, enabled):
        if enabled == self.is_mini_mode:
            return
        self.is_mini_mode = enabled
        # rapid toggles must not let a stale pending resize fire late
        self._geometry_tracking_suspended = True
        if self._geometry_apply_source is not None:
            GLib.source_remove(self._geometry_apply_source)
            self._geometry_apply_source = None

        if enabled:
            self.unmaximize()
            self.controls_container.hide()
            self.header_bar.hide()
            self.set_decorated(False)
            self.set_keep_above(True)
            width = height = self.settings.mini_size
            x, y = self.settings.mini_x, self.settings.mini_y
        else:
            self.settings.mini_x, self.settings.mini_y = self.get_position()
            self.controls_container.show()
            self.header_bar.show()
            self.set_decorated(True)
            self.set_keep_above(self.settings.pinned)
            width, height = self.settings.window_width, self.settings.window_height
            x, y = self.settings.window_x, self.settings.window_y

        def apply_geometry():
            self.resize(width, height)
            if x is not None:
                self.move(x, y)
            self._geometry_apply_source = None
            # tracking stays off until the WM has acted on our geometry
            GLib.timeout_add(250, self._resume_geometry_tracking)
            return False

        # let the decoration change settle before moving/resizing, otherwise
        # the window manager fights us and the window flickers or re-tiles
        self._geometry_apply_source = GLib.timeout_add(50, apply_geometry)
        self._wake_renderer()

    def _resize_mini(self, size):
        self.settings.mini_size = max(140, min(640, int(size)))
        if self.is_mini_mode:
            self.resize(self.settings.mini_size, self.settings.mini_size)

    # ----------------------------------------------------------------- input --

    def _show_context_menu(self, event):
        menu = Gtk.Menu()

        def add_item(label, callback, sensitive=True, target_menu=None):
            item = Gtk.MenuItem(label=label)
            item.connect("activate", lambda _i: callback())
            item.set_sensitive(sensitive)
            (target_menu or menu).append(item)

        def add_check_item(label, active, callback, radio=False, target_menu=None):
            item = Gtk.CheckMenuItem(label=label)
            item.set_draw_as_radio(radio)
            item.set_active(active)
            item.connect("toggled", lambda widget: callback(widget.get_active()))
            (target_menu or menu).append(item)

        def add_submenu(label):
            submenu = Gtk.Menu()
            item = Gtk.MenuItem(label=label)
            item.set_submenu(submenu)
            menu.append(item)
            return submenu

        capturing = self.capture_toggle.get_active()
        add_item("Pause capture" if capturing else "Resume capture",
                 lambda: self.capture_toggle.set_active(not capturing))
        add_item("Play audio file…  (O)", self.open_audio_file)
        if self.playing_file is not None:
            paused = self.capture_stream.playback_paused
            add_item("Resume track" if paused else "Pause track",
                     self._on_transport_play_pause)
            add_item("Next track  ⏭", self.play_next_track,
                     sensitive=len(self.playlist) > 1)
            add_item("Previous track  ⏮", self.play_previous_track,
                     sensitive=len(self.playlist) > 1)
        add_item("Snapshot  (S)", self.save_snapshot)
        add_item("Save last 10 s  (C)", self.save_clip)
        menu.append(Gtk.SeparatorMenuItem())

        mode_menu = add_submenu("Display mode")
        for mode_id, mode_label in DISPLAY_MODES:
            add_check_item(mode_label, mode_id == self.settings.display_mode,
                           lambda active, m=mode_id:
                               active and self.mode_combo.set_active_id(m),
                           radio=True, target_menu=mode_menu)

        theme_menu = add_submenu("Theme")
        for theme_name in list(THEME_PRESETS) + [CUSTOM_THEME_NAME]:
            add_check_item(theme_name, theme_name == self.settings.theme_name,
                           lambda active, t=theme_name:
                               active and self.theme_combo.set_active_id(t),
                           radio=True, target_menu=theme_menu)

        add_check_item("Grid  (G)", self.settings.grid_enabled,
                       lambda active: self.grid_switch.set_active(active))
        add_check_item("Pin above  (P)", self.settings.pinned,
                       lambda active: self.pin_toggle.set_active(active))
        menu.append(Gtk.SeparatorMenuItem())

        if self.is_mini_mode:
            for preset_label, preset_size in MINI_SIZE_PRESETS:
                add_item(f"Mini size: {preset_label}",
                         lambda s=preset_size: self._resize_mini(s))
            add_item("Restore window  (M)", lambda: self.set_mini_mode(False))
        else:
            add_item("Mini view  (M)", lambda: self.set_mini_mode(True))
            add_item("Leave fullscreen  (F11)" if self._is_fullscreen
                     else "Fullscreen scope  (F11)", self.toggle_fullscreen)
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
        if isinstance(self.get_focus(), Gtk.Entry):
            return False    # typing in a percent box must not fire shortcuts
        key = event.keyval
        if key == Gdk.KEY_space:
            self.capture_toggle.set_active(not self.capture_toggle.get_active())
        elif key in (Gdk.KEY_o, Gdk.KEY_O):
            self.open_audio_file()
        elif key in (Gdk.KEY_m, Gdk.KEY_M):
            self.set_mini_mode(not self.is_mini_mode)
        elif key in (Gdk.KEY_s, Gdk.KEY_S):
            self.save_snapshot()
        elif key in (Gdk.KEY_c, Gdk.KEY_C):
            self.save_clip()
        elif key in (Gdk.KEY_p, Gdk.KEY_P):
            self.pin_toggle.set_active(not self.pin_toggle.get_active())
        elif key in (Gdk.KEY_g, Gdk.KEY_G):
            self.grid_switch.set_active(not self.settings.grid_enabled)
        elif key in (Gdk.KEY_f, Gdk.KEY_F):
            self.show_fps_switch.set_active(not self.settings.show_fps)
        elif key == Gdk.KEY_F11:
            self.toggle_fullscreen()
        elif key == Gdk.KEY_Escape:
            if self._is_fullscreen:
                self.unfullscreen()
            elif self.is_mini_mode:
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
        self.pin_toggle.set_visible(self.settings.show_pin_button)
        if self.settings.start_in_mini:
            self.set_mini_mode(True)
        elif self.settings.window_x is not None:
            self.move(self.settings.window_x, self.settings.window_y)
        if self.settings.pinned and not self.is_mini_mode:
            self.set_keep_above(True)
        if not self.settings.start_in_mini:
            # geometry tracking begins only after the remembered placement
            # has been applied (set_mini_mode schedules its own resume)
            GLib.timeout_add(500, self._resume_geometry_tracking)

    def _on_delete(self, _widget, _event):
        self.settings.start_in_mini = self.is_mini_mode
        if self.is_mini_mode:
            self.settings.mini_x, self.settings.mini_y = self.get_position()
        # normal-view geometry is already tracked via configure events
        self.settings.save()
        self.capture_stream.stop()
        return False


class PhosphorApplication(Gtk.Application):
    def __init__(self):
        super().__init__(application_id=APPLICATION_ID)

    def do_activate(self):
        window = self.props.active_window
        if window is None:
            window = OscilloscopeWindow(application=self)
            window.show_all()
            window.apply_remembered_view()
            window.capture_toggle.set_active(True)  # start live; off is free
        window.present()


def main():
    GLib.set_prgname("phosphor")   # ties the window to phosphor.desktop's icon
    PhosphorApplication().run(None)


if __name__ == "__main__":
    main()
