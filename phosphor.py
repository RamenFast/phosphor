#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Phosphor — a software XY oscilloscope for everything your PC plays.

In XY mode the left audio channel drives the beam horizontally and the
right channel drives it vertically, so "oscilloscope music"
(Jerobeam Fenderson and friends) draws its hidden pictures on screen.
Goniometer, waveform, and spectrum modes make ordinary music look good too.

Capture taps PulseAudio/PipeWire monitors — whole outputs, single
applications, or microphones — and costs nothing while toggled off.
Audio files can also be played directly (decoded by ffmpeg, audible
through pacat) so you can scope a track without a separate player.
Compose mode runs the whole machine in reverse: draw a shape on the
scope and it becomes a constant-speed audio loop that draws itself.

Keys:  Space capture · O open file · L playlist · D draw (compose)
       M mini · S snapshot · C save clip · P pin · G grid · F fps
       F11 fullscreen · scroll = gain (compose: pitch · Ctrl+scroll in
       mini: resize) · Q quit
Mini mode: drag with the left button, drag the bottom-right corner to
resize, double-click to restore, right-click anywhere for the menu.
"""

import math
import os
import sys
import threading
import time

try:
    import numpy
except ImportError:
    numpy = None

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GLib  # noqa: E402

import phosphor_compose
import phosphor_mpris
import phosphor_precompute
import phosphor_recorder
import phosphor_ui_style
from phosphor_audio import (AudioCaptureStream, default_monitor_target_id,
                            list_capture_targets)
from phosphor_player import PhosphorPlayer
from phosphor_render_cairo import CairoBeamCore
from phosphor_render_gl import GL_BINDINGS_AVAILABLE, GLBeamRenderer
from phosphor_settings import (CUSTOM_THEME_NAME, THEME_PRESETS, Settings,
                               grid_spacing_fraction)
from phosphor_signal import SegmentComputer, plan_feed
from phosphor_ui_style import UI_STYLE_CHOICES

APPLICATION_ID = "io.github.ben.Phosphor"
APPLICATION_VERSION = "3.1.4"
PROJECT_DIRECTORY = os.path.dirname(os.path.abspath(__file__))
QUIET_PEAK_THRESHOLD = 1e-4
QUIET_FRAMES_BEFORE_SLEEP = 120
MINI_SIZE_PRESETS = (("Small", 200), ("Medium", 280), ("Large", 380),
                     ("Extra large", 520))
MINI_RESIZE_CORNER_PIXELS = 26       # bottom-right drag-to-resize hot zone
COMPOSE_PREVIEW_INTENSITY = 0.25     # restamped every frame while drawing
COMPOSE_MINIMUM_POINTS = 8
REACQUIRE_POLL_LIMIT = 180   # seconds to wait for a dead app stream to return
VISITOR_SWIM_SECONDS = 7.0   # you know the code

DISPLAY_MODES = (
    ("xy", "XY (scope art)"),
    ("xy45", "XY · goniometer"),
    ("xy_swirl", "XY · swirl"),
    ("xy_dots", "XY · dots"),
    ("waveform", "Waveform"),
    ("ring", "Ring · oscillogram"),
    ("spectrum", "Spectrum"),
    ("spectrum_radial", "Spectrum · radial"),
    ("tunnel", "Spectrum · tunnel"),
)

GPU_QUALITY_CHOICES = (("1", "Standard"), ("2", "High · 2× supersampled"),
                       ("3", "Ultra · 3× supersampled"))
CPU_RESOLUTION_CHOICES = (("1.0", "Full resolution"),
                          ("0.75", "Balanced · 75%"),
                          ("0.5", "Fast · 50%"))
# scope feed rate: above 48 kHz the resampler reconstructs the true curves
# between samples, so fine scope-art detail stops washing out
SCOPE_RATE_CHOICES = (("48000", "Standard · 48 kHz"),
                      ("96000", "Fine · 96 kHz"),
                      ("192000", "Ultra · 192 kHz"),
                      ("384000", "Extreme · 384 kHz"))
VALID_SCOPE_RATES = (48000, 96000, 192000, 384000)
# frame-rate cap presets; 0 = follow the monitor. The list reaches into
# high-refresh territory and the box stays editable for anything between.
MAX_FPS_PRESETS = (0, 30, 60, 90, 120, 144, 165, 240, 360, 480)

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
        self.glass_alpha = 1.0
        self._core_lock = threading.Lock()
        self._mailbox = None               # only the newest frame survives
        self._mailbox_condition = threading.Condition()
        worker = threading.Thread(target=self._worker_loop, daemon=True)
        worker.start()
        self.connect("draw", self._on_draw)

    def advance(self, samples):
        """Queue this frame's samples; the worker does the heavy lifting."""
        self._post_to_worker("samples", samples)

    def advance_segments(self, segments):
        """Queue precomputed segments (compose-mode preview strokes)."""
        self._post_to_worker("segments", segments)

    def _post_to_worker(self, payload_kind, payload):
        allocation = self.get_allocation()
        if allocation.width < 2 or allocation.height < 2:
            return
        with self._mailbox_condition:
            self._mailbox = (payload_kind, payload,
                             allocation.width, allocation.height,
                             self.persistence, self.resolution, self.beam_focus)
            self._mailbox_condition.notify()

    def _worker_loop(self):
        while True:
            with self._mailbox_condition:
                while self._mailbox is None:
                    self._mailbox_condition.wait()
                (payload_kind, payload, width, height,
                 persistence, resolution, beam_focus) = self._mailbox
                self._mailbox = None
            if payload_kind == "segments":
                segments = payload
            else:
                with self.compute_lock:
                    segments = self.segment_computer.compute(payload, width,
                                                             height)
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
                                self.grid_spacing_fraction,
                                glass_alpha=self.glass_alpha)
        return False


class OscilloscopeWindow(Gtk.ApplicationWindow):
    def __init__(self, application):
        super().__init__(application=application, title="Phosphor")
        self.settings = Settings.load()
        self.set_default_size(self.settings.window_width, self.settings.window_height)

        # an alpha channel when the compositor offers one: opaque themes
        # look identical, and Aero glass gets genuinely see-through chrome
        rgba_visual = self.get_screen().get_rgba_visual()
        if rgba_visual is not None:
            self.set_visual(rgba_visual)

        icon_path = os.path.join(PROJECT_DIRECTORY, "phosphor-scope.svg")
        if os.path.exists(icon_path):
            try:
                self.set_icon_from_file(icon_path)
            except GLib.Error:
                pass

        if self.settings.scope_sample_rate not in VALID_SCOPE_RATES:
            self.settings.scope_sample_rate = 96000

        # the native core reconstructs high detail rates in-process from a
        # modest pipe rate; without it the pipe carries the full rate
        pipe_rate, oversample = plan_feed(self.settings.scope_sample_rate)
        self.segment_computer = SegmentComputer()
        self.segment_computer.mode = self.settings.display_mode
        self.segment_computer.gain = self.settings.gain
        self.segment_computer.beam_energy = self.settings.beam_energy
        self.segment_computer.set_sample_rate(pipe_rate, oversample)
        self.compute_lock = threading.Lock()

        self.capture_stream = AudioCaptureStream(
            on_stream_ended=lambda: GLib.idle_add(self._handle_stream_died),
            sample_rate=pipe_rate)
        self.capture_targets = {}
        self._populating_targets = False
        self.player = PhosphorPlayer(self)
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
        self._frame_work_seconds = 0.0     # python time per frame, for the chip
        self._worst_frame_gap = 0.0        # longest frame-to-frame stall
        # auto gain: the gain actually applied; tracks the slider unless
        # autosize is on, in which case it follows the signal's peak
        self._effective_gain = self.settings.gain
        self._grid_gain = self.settings.gain
        self._auto_gain_peak = 0.0
        # compose mode (draw a shape, hear it)
        self.is_composing = False
        self._compose_drawing = False
        self._compose_points = []          # widget pixels while drawing
        self._compose_loop_points = None   # finished shape in signal space
        self._compose_regenerate_source = None
        # now-playing overlay fade animation
        self._overlay_fade_source = None
        # a certain ten-key sequence invites a visitor onto the scope
        self._konami_progress = 0
        self._visitor_started = None
        # precomputed scope streams (phosphor_precompute)
        self._precomputed = None
        self._precomputed_clock = 0.0      # seconds of the stream traced so far
        self._precompute_worker_path = None
        # mini-mode corner resize
        self._mini_resize_start = None     # (start size, x_root, y_root)

        phosphor_ui_style.install_base_style()
        self._sync_glass_class()
        self._build_renderers()
        self._build_user_interface()
        self._apply_theme()
        self._apply_render_quality()
        self._apply_grid_geometry()
        self._apply_ui_style()

        try:
            self._mpris_watcher = phosphor_mpris.MprisWatcher(
                self._on_external_track)
        except GLib.Error:
            self._mpris_watcher = None   # no session bus; overlay still works

        self.connect("key-press-event", self._on_key_press)
        self.connect("map-event", lambda *_args: (self._wake_renderer(),
                                                  False)[1])
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
        self.controls_container.set_name("control-deck")
        self.controls_container.pack_start(self._build_main_toolbar_row(), False, False, 0)
        self.controls_container.pack_start(self._build_slider_toolbar_row(), False, False, 0)
        layout.pack_start(self.controls_container, False, False, 0)

        event_box = Gtk.EventBox()
        event_box.add_events(Gdk.EventMask.BUTTON_PRESS_MASK
                             | Gdk.EventMask.BUTTON_RELEASE_MASK
                             | Gdk.EventMask.POINTER_MOTION_MASK
                             | Gdk.EventMask.SCROLL_MASK)
        event_box.connect("button-press-event", self._on_button_press)
        event_box.connect("button-release-event", self._on_button_release)
        event_box.connect("motion-notify-event", self._on_motion)
        event_box.connect("scroll-event", self._on_scroll)
        event_box.add(self.display_stack)
        self.scope_event_box = event_box   # cursor changes target its window

        self.fps_label = Gtk.Label(label="… fps")
        self.fps_label.set_name("fps-overlay")
        self.fps_label.set_halign(Gtk.Align.END)
        self.fps_label.set_valign(Gtk.Align.START)
        for edge in ("top", "end"):
            getattr(self.fps_label, f"set_margin_{edge}")(10)
        self.fps_label.set_no_show_all(True)
        self.fps_label.set_visible(self.settings.show_fps)

        self.now_playing_label = Gtk.Label()
        self.now_playing_label.set_name("now-playing")
        self.now_playing_label.set_halign(Gtk.Align.START)
        self.now_playing_label.set_valign(Gtk.Align.START)
        for edge in ("top", "start"):
            getattr(self.now_playing_label, f"set_margin_{edge}")(12)
        self.now_playing_label.set_max_width_chars(46)
        self.now_playing_label.set_ellipsize(3)  # Pango.EllipsizeMode.END
        self.now_playing_label.set_no_show_all(True)

        self.display_overlay = Gtk.Overlay()
        self.display_overlay.add(event_box)
        self.display_overlay.add_overlay(self.fps_label)
        self.display_overlay.add_overlay(self.now_playing_label)

        scope_row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL)
        scope_row.pack_start(self.display_overlay, True, True, 0)
        scope_row.pack_end(self.player.panel_revealer, False, False, 0)
        layout.pack_start(scope_row, True, True, 0)
        self.add(layout)

        # audio files dropped anywhere on the window play on the scope
        self.drag_dest_set(Gtk.DestDefaults.ALL, [], Gdk.DragAction.COPY)
        self.drag_dest_add_uri_targets()
        self.connect("drag-data-received", self._on_drag_data_received)

    def _build_header_bar(self):
        self.header_bar = Gtk.HeaderBar()
        self.header_bar.set_show_close_button(True)
        self.header_bar.set_title("Phosphor")
        self.set_titlebar(self.header_bar)

        open_button = Gtk.Button.new_from_icon_name("document-open-symbolic",
                                                    Gtk.IconSize.BUTTON)
        open_button.set_tooltip_text("Play an audio file on the scope (O)")
        open_button.connect("clicked", lambda _b: self.player.open_audio_file())
        self.header_bar.pack_start(open_button)

        self.compose_toggle = Gtk.ToggleButton()
        self.compose_toggle.add(Gtk.Image.new_from_icon_name(
            "document-edit-symbolic", Gtk.IconSize.BUTTON))
        self.compose_toggle.set_tooltip_text(
            "Compose: draw a shape on the scope and hear it (D).\n"
            "Release to play the loop, scroll to tune the pitch,\n"
            "right-click to export it as oscilloscope-music WAV.")
        self.compose_toggle.connect("toggled", self._on_compose_toggled)
        self.header_bar.pack_start(self.compose_toggle)

        # transport + seek slider live in the player; shown while a file plays
        self.header_bar.pack_start(self.player.transport_box)
        self.header_bar.pack_start(self.player.position_box)

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
            "Drag to move, drag the bottom-right corner or Ctrl+scroll\n"
            "to resize, double-click to restore.")
        mini_button.connect("clicked", lambda _b: self.set_mini_mode(True))
        self.header_bar.pack_end(mini_button)

        self.playlist_toggle = Gtk.ToggleButton()
        self.playlist_toggle.add(Gtk.Image.new_from_icon_name(
            "view-list-symbolic", Gtk.IconSize.BUTTON))
        self.playlist_toggle.set_tooltip_text(
            "Playlist panel (L) — the opened file's folder;\n"
            "drop audio files anywhere to queue them instead")
        self.playlist_toggle.set_active(self.settings.playlist_panel_open)
        self.playlist_toggle.connect(
            "toggled", lambda t: self.player.set_panel_open(t.get_active()))
        self.header_bar.pack_end(self.playlist_toggle)

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
        self.gain_scale.set_sensitive(not self.settings.auto_gain)
        add_slider(
            "Glow", "Phosphor persistence — how long trails linger", 0.0, 0.98,
            self.settings.persistence, self._on_persistence_changed)
        add_slider(
            "Beam", "Beam brightness budget — higher keeps fast strokes visible",
            1.0, 30.0, self.settings.beam_energy, self._on_beam_changed)
        return row

    def _build_settings_button(self):
        """The gear popover: two columns of labeled sections."""
        popover = Gtk.Popover()
        popover.connect("show", self._on_settings_popover_toggled, True)
        popover.connect("closed", self._on_settings_popover_toggled, False)
        columns = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=18)
        for edge in ("start", "end", "top", "bottom"):
            getattr(columns, f"set_margin_{edge}")(14)
        state = {"grid": None, "row": 0}

        def column():
            if state["grid"] is not None:
                columns.pack_start(Gtk.Separator(
                    orientation=Gtk.Orientation.VERTICAL), False, False, 0)
            grid = Gtk.Grid(row_spacing=8, column_spacing=10)
            grid.set_valign(Gtk.Align.START)
            columns.pack_start(grid, False, False, 0)
            state["grid"] = grid
            state["row"] = 0

        def section(title):
            header = Gtk.Label(label=title.upper(), xalign=0)
            header.get_style_context().add_class("settings-section")
            if state["row"] > 0:
                header.set_margin_top(12)
            state["grid"].attach(header, 0, state["row"], 2, 1)
            state["row"] += 1

        def attach(label_text, widget):
            state["grid"].attach(Gtk.Label(label=label_text, xalign=0),
                                 0, state["row"], 1, 1)
            state["grid"].attach(widget, 1, state["row"], 1, 1)
            state["row"] += 1
            return widget

        def combo(choices, active_id, on_changed):
            box = Gtk.ComboBoxText()
            for choice_id, choice_label in choices:
                box.append(choice_id, choice_label)
            box.set_active_id(active_id)
            box.connect("changed", on_changed)
            return box

        def switch(active, on_state):
            widget = Gtk.Switch(halign=Gtk.Align.START)
            widget.set_active(active)
            widget.connect("state-set", on_state)
            return widget

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

        column()
        section("Renderer")
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

        section("Scope")
        scope_rate_combo = combo(SCOPE_RATE_CHOICES,
                                 str(self.settings.scope_sample_rate),
                                 self._on_scope_rate_changed)
        scope_rate_combo.set_tooltip_text(
            "Scope feed sample rate — higher rates trace the true curves\n"
            "between samples, recovering fine scope-art detail")
        attach("Scope detail", scope_rate_combo)
        precompute_switch = attach("Precompute files", switch(
            self.settings.precompute_enabled, self._on_precompute_switched))
        precompute_switch.set_tooltip_text(
            "Render opened tracks' scope streams ahead of time to\n"
            "~/.local/share/phosphor/precomputed and play from disk:\n"
            "no realtime reconstruction, nothing dropped on slow machines.\n"
            "Costs disk — ~92 MB per track-minute at 384 kHz, less at\n"
            "lower rates. Right-click the scope to clear the cache.")
        focus_scale = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL,
                                               0.6, 3.0, 0.1)
        focus_scale.set_value(self.settings.beam_focus)
        focus_scale.set_draw_value(False)
        focus_scale.set_size_request(140, -1)
        focus_scale.set_tooltip_text(
            "Beam focus — narrower keeps dense scenes from washing out")
        focus_scale.connect("value-changed", self._on_focus_changed)
        attach("Focus", focus_scale)
        self.auto_gain_switch = attach("Auto gain", switch(
            self.settings.auto_gain, self._on_auto_gain_switched))
        self.auto_gain_switch.set_tooltip_text(
            "Autosize the trace to the screen — gain follows the signal's\n"
            "peak so quiet tracks still fill the display")
        self.grid_switch = attach("Grid", switch(self.settings.grid_enabled,
                                                 self._on_grid_switched))
        attach("AMOLED scope", switch(self.settings.amoled_background,
                                      self._on_amoled_switched))
        self.glass_switch = attach("Glass scope", switch(
            self.settings.scope_glass, self._on_glass_switched))
        self.glass_switch.set_tooltip_text(
            "Translucent scope pane — the beam glows over whatever is\n"
            "behind the window (needs a compositing desktop; pairs\n"
            "beautifully with the mini view and Aero glass)")
        self.glass_tint_scale = Gtk.Scale.new_with_range(
            Gtk.Orientation.HORIZONTAL, 0.0, 0.95, 0.05)
        self.glass_tint_scale.set_value(
            self.settings.glass_tint_for(self.settings.ui_style))
        self.glass_tint_scale.set_draw_value(False)
        self.glass_tint_scale.set_size_request(140, -1)
        self.glass_tint_scale.set_tooltip_text(
            "How dark the glass smokes the desktop behind the scope —\n"
            "fully clear on the left, nearly opaque on the right.\n"
            "Remembered separately for each UI style.")
        self.glass_tint_scale.connect("value-changed",
                                      self._on_glass_tint_changed)
        self.glass_tint_scale.set_sensitive(self.settings.scope_glass)
        attach("Glass tint", self.glass_tint_scale)

        column()
        section("Appearance")
        self.theme_combo = Gtk.ComboBoxText()
        for theme_name in list(THEME_PRESETS) + [CUSTOM_THEME_NAME]:
            self.theme_combo.append(theme_name, theme_name)
        self.theme_combo.set_active_id(self.settings.theme_name)
        self.theme_combo.connect("changed", self._on_theme_changed)
        attach("Theme", self.theme_combo)
        self.custom_beam_button = attach("Custom beam", color_button(
            self.settings.custom_beam_color,
            lambda rgb: setattr(self.settings, "custom_beam_color", rgb)))
        self.custom_grid_button = attach("Custom grid", color_button(
            self.settings.custom_grid_color,
            lambda rgb: setattr(self.settings, "custom_grid_color", rgb)))
        attach("UI style", combo(UI_STYLE_CHOICES, self.settings.ui_style,
                                 self._on_ui_style_changed))
        attach("Pin button", switch(self.settings.show_pin_button,
                                    self._on_show_pin_switched))

        section("Player")
        now_playing_switch = attach("Track info", switch(
            self.settings.show_now_playing, self._on_now_playing_switched))
        now_playing_switch.set_tooltip_text(
            "Fade the artist/title into the corner when the song changes —\n"
            "for files Phosphor plays and for other players (MPRIS)")

        section("Performance")
        self.max_fps_combo = Gtk.ComboBoxText.new_with_entry()
        for fps_value in MAX_FPS_PRESETS:
            self.max_fps_combo.append(
                str(fps_value),
                "Monitor" if fps_value == 0 else str(fps_value))
        fps_entry = self.max_fps_combo.get_child()
        fps_entry.set_width_chars(8)
        self.max_fps_combo.set_tooltip_text(
            "Frame rate cap — Monitor follows the display's refresh rate;\n"
            "pick a preset or type any rate up to 1000. Lower trades\n"
            "smoothness for power; setting it equal to the refresh rate\n"
            "races vsync and drops frames, so prefer Monitor for that.")
        if not self.max_fps_combo.set_active_id(str(self.settings.max_fps)):
            fps_entry.set_text(str(self.settings.max_fps))
        self.max_fps_combo.connect("changed", self._on_max_fps_changed)
        attach("Max FPS", self.max_fps_combo)
        self.show_fps_switch = attach("Show FPS", switch(
            self.settings.show_fps, self._on_show_fps_switched))

        columns.show_all()
        popover.add(columns)
        self._update_custom_color_sensitivity()

        settings_button = Gtk.MenuButton()
        settings_button.add(Gtk.Image.new_from_icon_name(
            "emblem-system-symbolic", Gtk.IconSize.BUTTON))
        settings_button.set_tooltip_text("Renderer, quality, theme, UI style")
        settings_button.set_popover(popover)
        return settings_button

    def _on_settings_popover_toggled(self, popover, opening):
        """Scoot the scope out from under the settings popover so the
        trace stays visible while you tweak it; snaps back on close. Skipped
        when the window is too narrow to leave a useful scope."""
        margin = 0
        if opening:
            _minimum, popover_width = popover.get_preferred_width()
            if self.get_allocated_width() - popover_width >= 420:
                margin = popover_width
        self.display_overlay.set_margin_end(margin)

    def _update_custom_color_sensitivity(self):
        is_custom = self.settings.theme_name == CUSTOM_THEME_NAME
        self.custom_beam_button.set_sensitive(is_custom)
        self.custom_grid_button.set_sensitive(is_custom)

    # -------------------------------------------------------------- targets --

    def _populate_targets(self):
        """Re-scan sources. Purely a list rebuild: restoring the previous
        selection must not fire the picked-a-source side effects (that used
        to interrupt file playback on every refresh)."""
        previous_choice = self.target_combo.get_active_id() or self.settings.target_id
        self._populating_targets = True
        try:
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
        finally:
            self._populating_targets = False
        target_id = self.target_combo.get_active_id()
        if target_id is not None:
            self.settings.target_id = target_id
        if hasattr(self, "target_kind_icon"):   # first populate predates it
            self._update_target_kind_icon()

    def _on_target_changed(self, _combo):
        target_id = self.target_combo.get_active_id()
        if target_id is None or self._populating_targets:
            return
        self._cancel_reacquire()
        self.settings.target_id = target_id
        self._update_target_kind_icon()
        if self.is_composing:
            return
        if self.player.playing_file is not None:
            # picking a source while a track plays means "scope that
            # instead": unload the file and go live on the new target
            self.player.set_file_loaded(False)
            try:
                self.capture_stream.start(self.capture_targets[target_id])
            except OSError as error:
                self.status_label.set_text(f"capture failed: {error}")
                self.sync_capture_toggle(False)
                return
            self.sync_capture_toggle(True)
            self.quiet_frame_count = 0
            self.status_label.set_text("● live")
            self._start_render_loop()
        elif self.capture_stream.is_running:
            self.capture_stream.start(self.capture_targets[target_id])

    # -------------------------------------------------------------- capture --

    def _on_capture_toggled(self, toggle):
        self._cancel_reacquire()
        # while a file is loaded, the Live button means sound on/off for the
        # track: off pauses it in place (SIGSTOP - still zero CPU), on
        # resumes it. Unloading it here made Live-on come back as silent
        # device capture, which read as "won't turn back on".
        if self.player.playing_file is not None and self.capture_stream.is_running:
            if toggle.get_active() != self.capture_stream.playback_paused:
                return
            self.player.toggle_play_pause()
            if not toggle.get_active():
                self.fade_out_frames_remaining = 90
            return
        if toggle.get_active():
            self._exit_compose_mode(stop_loop=False)
            self.player.set_file_loaded(False)
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
            self.player.set_file_loaded(False)
            self.status_label.set_text("idle — no capture, no CPU")
            self.fade_out_frames_remaining = 90

    def sync_capture_toggle(self, active):
        """Reflect state in the Live toggle without re-triggering capture."""
        self.capture_toggle.handler_block_by_func(self._on_capture_toggled)
        self.capture_toggle.set_active(active)
        self.capture_toggle.handler_unblock_by_func(self._on_capture_toggled)

    # ---------------------------------------------- precomputed streams --

    def prepare_scope_feed(self, path):
        """Scope side of playing `path`: attach a matching precomputed
        stream (audible pipe drops to 48 kHz, the scope reads the stream by
        the playback clock) or fall back to the live pipe; queue generation
        when the setting asks for it."""
        track = (phosphor_precompute.find(path, self.settings.scope_sample_rate)
                 if self.settings.precompute_enabled else None)
        self.attach_precomputed(track)
        if (track is None and self.settings.precompute_enabled
                and self.settings.scope_sample_rate
                > phosphor_precompute.AUDIO_PIPE_RATE):
            self._queue_precompute(path)

    def attach_precomputed(self, track):
        previous, self._precomputed = self._precomputed, track
        if previous is not None:
            previous.close()
        self._precomputed_clock = 0.0
        if track is not None:
            self.capture_stream.configure_sample_rate(
                phosphor_precompute.AUDIO_PIPE_RATE)
            with self.compute_lock:
                self.segment_computer.set_sample_rate(track.sample_rate, 1)
                self.segment_computer.reset()
        else:
            pipe_rate, oversample = plan_feed(self.settings.scope_sample_rate)
            self.capture_stream.configure_sample_rate(pipe_rate)
            with self.compute_lock:
                self.segment_computer.set_sample_rate(pipe_rate, oversample)
                self.segment_computer.reset()

    def detach_precomputed(self):
        if self._precomputed is not None:
            self.attach_precomputed(None)

    def on_playback_restarted(self, seconds):
        """After a seek or rate reload: jump the stream clock, break the
        trace so the beam doesn\'t bridge the jump."""
        self._precomputed_clock = seconds
        with self.compute_lock:
            self.segment_computer.reset()

    def _pull_precomputed_samples(self):
        """The stream slice the playback clock crossed since the last frame.
        A stalled frame reads a longer slice — detail is never dropped, just
        traced late (capped so a long stall doesn\'t burst)."""
        position = min(self.capture_stream.playback_position_seconds,
                       self._precomputed.duration_seconds)
        start = self._precomputed_clock
        if position - start > 0.25:
            start = position - 0.25
        chunk = self._precomputed.samples_between(start, position)
        self._precomputed_clock = position
        return chunk

    def _export_detail_oversample(self):
        """Exports re-render from the 48 kHz history; match the screen."""
        if self._precomputed is not None:
            return max(1, round(self._precomputed.sample_rate
                                / self.capture_stream.sample_rate))
        return self.segment_computer.oversample

    def _queue_precompute(self, path):
        if self._precompute_worker_path is not None:
            return
        self._precompute_worker_path = path
        rate = self.settings.scope_sample_rate
        last_percent = [-1]

        def report(fraction):
            percent = int(fraction * 100)
            if percent != last_percent[0]:
                last_percent[0] = percent
                GLib.idle_add(self.status_label.set_text,
                              f"precomputing scope stream… {percent}%")

        def finished(track):
            self._precompute_worker_path = None
            if self.player.playing_path == path:
                position = self.capture_stream.playback_position_seconds
                self.attach_precomputed(track)
                self.on_playback_restarted(position)
                self.status_label.set_text(
                    f"▶ {self.player.playing_file} — precomputed ✓")
            else:
                track.close()
                self.status_label.set_text("scope stream precomputed ✓")
            return False

        def failed(message):
            self._precompute_worker_path = None
            self.status_label.set_text(f"precompute failed: {message}")
            return False

        def worker():
            try:
                track = phosphor_precompute.generate(path, rate, report)
                GLib.idle_add(finished, track)
            except (RuntimeError, OSError) as error:
                GLib.idle_add(failed, str(error))
        threading.Thread(target=worker, daemon=True).start()

    def _clear_precompute_cache(self):
        self.detach_precomputed()
        freed = phosphor_precompute.clear_cache()
        self.status_label.set_text(
            f"precomputed streams cleared ({freed / 1e6:.0f} MB)")

    def _handle_stream_died(self):
        if not self.capture_toggle.get_active():
            return
        finished_file = self.player.playing_file
        died_target_id = self.settings.target_id or ""
        self.sync_capture_toggle(False)
        self.capture_stream.stop()
        self.fade_out_frames_remaining = 90
        if self.is_composing:
            # the drawn loop repeats forever; if its decoder dies it was
            # killed externally — just invite another stroke
            self.status_label.set_text("✏ loop stopped — draw to start again")
            return
        if finished_file is not None:
            self.player.handle_track_finished(finished_file)
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

    # ------------------------------------------------------------- visitor --

    def _begin_visitor(self):
        self._visitor_started = time.monotonic()
        self.fade_out_frames_remaining = max(self.fade_out_frames_remaining,
                                             int(VISITOR_SWIM_SECONDS * 240))
        self._start_render_loop()

    def _visitor_segments(self, width, height):
        """A turtle, drawn by the beam, swimming once across the scope."""
        elapsed = time.monotonic() - self._visitor_started
        if elapsed > VISITOR_SWIM_SECONDS:
            self._visitor_started = None
            return []
        progress = elapsed / VISITOR_SWIM_SECONDS
        center_x = width * (-0.18 + 1.36 * progress)
        center_y = height * (0.5 + 0.055 * math.sin(elapsed * 2.1))
        scale = min(width, height) * 0.105
        paddle = math.sin(elapsed * 5.0) * 0.12

        def ellipse(ex, ey, rx, ry, points=22):
            return [(center_x + (ex + rx * math.cos(2 * math.pi * i / points))
                     * scale,
                     center_y + (ey + ry * math.sin(2 * math.pi * i / points))
                     * scale)
                    for i in range(points + 1)]

        paths = (
            ellipse(0.0, 0.0, 1.0, 0.72),                    # shell
            ellipse(0.0, 0.0, 0.62, 0.45),                   # shell pattern
            ellipse(1.28, 0.0, 0.30, 0.24),                  # head
            ellipse(1.36, -0.08, 0.05, 0.05, points=8),      # eye
            ellipse(0.70, -0.64 - paddle, 0.34, 0.15),       # flippers
            ellipse(0.70, 0.64 + paddle, 0.34, 0.15),
            ellipse(-0.72, -0.66 + paddle, 0.28, 0.13),
            ellipse(-0.72, 0.66 - paddle, 0.28, 0.13),
            ellipse(-1.16, 0.0, 0.13, 0.09, points=10),      # tail
        )
        segments = []
        for path in paths:
            for index in range(len(path) - 1):
                x0, y0 = path[index]
                x1, y1 = path[index + 1]
                segments.append((x0, y0, x1, y1, 0.85))
        return segments

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
        if self._last_frame_time > 0.0:
            self._worst_frame_gap = max(self._worst_frame_gap,
                                        now - self._last_frame_time)
        self._last_frame_time = now
        self._count_fps_frame(now)

        if self._visitor_started is not None:
            self.quiet_frame_count = 0     # the visitor swims through silence
        if self._compose_drawing:
            # the stroke in progress previews directly as segments; any
            # still-playing loop audio is drained so it can't burst in later
            if self.capture_stream.is_running:
                self.capture_stream.take_stereo_samples()
            self._advance_compose_preview()
            return GLib.SOURCE_CONTINUE
        if self.capture_stream.is_running:
            samples = self.capture_stream.take_stereo_samples()
            if (self._precomputed is not None
                    and self.player.playing_file is not None):
                # the live pipe only carries the audible audio; the scope
                # reads the precomputed stream by the playback clock
                samples = self._pull_precomputed_samples()
            # The monitor delivers zeros while nothing plays; detect silence by
            # content and stop redrawing once the glow has settled.
            if len(samples):
                if numpy is not None and isinstance(samples, numpy.ndarray):
                    peak = float(numpy.abs(samples).max())
                else:
                    peak = max(max(samples), -min(samples))
            else:
                peak = 0.0
            is_quiet = peak < QUIET_PEAK_THRESHOLD
            self.quiet_frame_count = self.quiet_frame_count + 1 if is_quiet else 0
            if self.quiet_frame_count > QUIET_FRAMES_BEFORE_SLEEP:
                return GLib.SOURCE_CONTINUE
            if not is_quiet:
                self._update_auto_gain(peak)
            self._advance_active_renderer(samples, allocation)
        else:
            self._advance_active_renderer([], allocation)
            if self.is_composing:
                return GLib.SOURCE_CONTINUE    # stay ready for the next stroke
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
            python_seconds = self._frame_work_seconds
            if self.gl_renderer is not None:
                python_seconds += self.gl_renderer.cpu_seconds_accumulated
                self.gl_renderer.cpu_seconds_accumulated = 0.0
            milliseconds = python_seconds / max(1, self._fps_counter) * 1000.0
            renderer_name = "GPU" if self.settings.renderer == "gl" else "CPU"
            if self.segment_computer.engine == "rust":
                renderer_name += "·rs"
            # "py" is python time per frame (not which renderer is active);
            # "max" is the worst gap between drawn frames — a big number
            # there during a hitch tells us the main loop stalled
            self.fps_label.set_text(
                f"{renderer_name} · {self._fps_counter / elapsed:.0f} fps · "
                f"{milliseconds:.1f}ms py · "
                f"max {self._worst_frame_gap * 1000.0:.0f}ms")
            self._frame_work_seconds = 0.0
            self._worst_frame_gap = 0.0
            self._fps_counter = 0
            self._fps_window_start = now

    def _advance_active_renderer(self, samples, allocation):
        work_started = time.perf_counter()
        renderer = self.active_renderer()
        visitor = (self._visitor_segments(allocation.width, allocation.height)
                   if self._visitor_started is not None else None)
        if visitor:
            with self.compute_lock:
                segments = self.segment_computer.compute(
                    samples, allocation.width, allocation.height)
            if numpy is not None and isinstance(segments, numpy.ndarray):
                extra = numpy.asarray(visitor, dtype=numpy.float32)
                segments = (numpy.concatenate((segments, extra))
                            if len(segments) else extra)
            else:
                segments = list(segments) + visitor
            if isinstance(renderer, LiveCairoRenderer):
                renderer.advance_segments(segments)
            else:
                renderer.advance(segments)
        elif isinstance(renderer, LiveCairoRenderer):
            renderer.advance(samples)      # worker thread computes + decays
        else:
            with self.compute_lock:
                segments = self.segment_computer.compute(
                    samples, allocation.width, allocation.height)
            renderer.advance(segments)
        self._frame_work_seconds += time.perf_counter() - work_started

    def _wake_renderer(self):
        """Repaint once after appearance changes, even while quiet/idle."""
        self.quiet_frame_count = 0
        renderer = self.active_renderer()
        if isinstance(renderer, LiveCairoRenderer):
            renderer.queue_draw()
        else:
            renderer.queue_render()

    # ---------------------------------------------------------- now playing --

    def flash_now_playing(self, title, subtitle=None):
        """Fade the track-info chip in over the scope, hold ~4 s, fade out."""
        if not self.settings.show_now_playing:
            return
        markup = f"<b>{GLib.markup_escape_text(title)}</b>"
        if subtitle:
            markup += f"\n<small>{GLib.markup_escape_text(subtitle)}</small>"
        label = self.now_playing_label
        label.set_markup(markup)
        if self._overlay_fade_source is not None:
            GLib.source_remove(self._overlay_fade_source)
        label.set_opacity(0.0)
        label.set_visible(True)
        state = {"phase": "in", "hold_until": time.monotonic() + 4.0}

        def step():
            opacity = label.get_opacity()
            if state["phase"] == "in":
                opacity = min(1.0, opacity + 0.09)
                label.set_opacity(opacity)
                if opacity >= 1.0:
                    state["phase"] = "hold"
            elif state["phase"] == "hold":
                if time.monotonic() >= state["hold_until"]:
                    state["phase"] = "out"
            else:
                opacity = max(0.0, opacity - 0.045)
                label.set_opacity(opacity)
                if opacity <= 0.0:
                    label.set_visible(False)
                    self._overlay_fade_source = None
                    return GLib.SOURCE_REMOVE
            return GLib.SOURCE_CONTINUE
        self._overlay_fade_source = GLib.timeout_add(33, step)

    def _on_external_track(self, title, artist, album, identity):
        """A song changed in some other player (MPRIS): show it, but only
        while the scope listens to the system, not its own file playback."""
        if (self.player.playing_file is not None
                or not self.capture_stream.is_running):
            return
        subtitle = " — ".join(part for part in (artist, album) if part)
        if identity:
            subtitle = f"{subtitle}  ·  {identity}" if subtitle else identity
        from phosphor_player import ARTIST_NODS
        nod = ARTIST_NODS.get((artist or "").strip().lower())
        if nod:
            subtitle = f"{subtitle}  ·  {nod}" if subtitle else nod
        self.flash_now_playing(title, subtitle or None)

    def _on_now_playing_switched(self, _switch, state):
        self.settings.show_now_playing = state
        if not state:
            if self._overlay_fade_source is not None:
                GLib.source_remove(self._overlay_fade_source)
                self._overlay_fade_source = None
            self.now_playing_label.set_visible(False)
        return False

    def _on_drag_data_received(self, _widget, _context, _x, _y, data,
                               _info, _time):
        paths = []
        for uri in data.get_uris():
            try:
                filename, _host = GLib.filename_from_uri(uri)
            except GLib.Error:
                continue
            if filename:
                paths.append(filename)
        if paths:
            self.player.play_dropped(paths)

    # ----------------------------------------------------- settings changes --

    def _apply_theme(self):
        theme = self.settings.current_theme()
        # glass touches only the scope: the window background vanishes
        # beneath it (chrome keeps its own deck), and the pane smokes the
        # desktop by exactly as much as this style's Glass tint asks
        glass_alpha = (self.settings.glass_tint_for(self.settings.ui_style)
                       if self.settings.scope_glass else 1.0)
        if self.gl_renderer is not None:
            self.gl_renderer.scope_alpha = glass_alpha
        self.cairo_renderer.glass_alpha = glass_alpha
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
        self._grid_gain = self._effective_gain
        fraction = grid_spacing_fraction(self._effective_gain)
        if self.gl_renderer is not None:
            self.gl_renderer.grid_spacing_fraction = fraction
        self.cairo_renderer.grid_spacing_fraction = fraction
        self._wake_renderer()

    def _apply_ui_style(self):
        self._style_css_provider = phosphor_ui_style.apply_ui_style(
            self.settings.ui_style, self._style_css_provider)

    def _update_auto_gain(self, peak):
        """Autosize: scale the trace to fill the screen. The tracked peak
        attacks instantly (nothing clips off-screen) and releases slowly,
        and the applied gain glides so the picture breathes rather than
        jumping between loud and quiet passages."""
        if not self.settings.auto_gain:
            return
        self._auto_gain_peak = max(peak, self._auto_gain_peak * 0.999)
        target_gain = min(6.0, max(0.1, 0.92 / max(self._auto_gain_peak, 0.01)))
        self._set_effective_gain(
            self._effective_gain + (target_gain - self._effective_gain) * 0.05)

    def _set_effective_gain(self, gain):
        self._effective_gain = gain
        self.segment_computer.gain = gain
        # the graticule tracks gain (volts/div); re-derive it only on real
        # movement so auto-gain's tiny per-frame glides stay free
        if abs(gain - self._grid_gain) > self._grid_gain * 0.02:
            self._apply_grid_geometry()

    def _on_auto_gain_switched(self, _switch, state):
        self.settings.auto_gain = state
        self.gain_scale.set_sensitive(not state)
        if state:
            self._auto_gain_peak = 0.0     # re-measure from the next sound
        else:
            self._set_effective_gain(self.settings.gain)
            self._apply_grid_geometry()
        return False

    def _on_gain_changed(self, value):
        self.settings.gain = value
        if not self.settings.auto_gain:
            self._set_effective_gain(value)
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

    def _on_scope_rate_changed(self, combo):
        if combo.get_active_id() is None:
            return
        rate = int(combo.get_active_id())
        if rate == self.settings.scope_sample_rate:
            return
        # capture the playback position before the stream's clock changes
        resume_seconds = self.capture_stream.playback_position_seconds
        self.settings.scope_sample_rate = rate
        # restart whatever is flowing so the new rate applies immediately
        try:
            if self.player.playing_path is not None:
                self.prepare_scope_feed(self.player.playing_path)
                self.capture_stream.start_file(self.player.playing_path,
                                               seek_seconds=resume_seconds)
                self.on_playback_restarted(resume_seconds)
            else:
                pipe_rate, oversample = plan_feed(rate)
                self.capture_stream.configure_sample_rate(pipe_rate)
                with self.compute_lock:
                    self.segment_computer.set_sample_rate(pipe_rate, oversample)
                    self.segment_computer.reset()
                if self.is_composing and self._compose_loop_points:
                    self._restart_compose_loop()
                elif self.capture_stream.is_running:
                    target_id = self.target_combo.get_active_id()
                    if target_id in self.capture_targets:
                        self.capture_stream.start(
                            self.capture_targets[target_id])
        except OSError as error:
            self.status_label.set_text(f"rate change failed: {error}")
        self.settings.save()

    def _on_cpu_resolution_changed(self, combo):
        if combo.get_active_id() is None:
            return
        self.settings.cairo_resolution = float(combo.get_active_id())
        self._apply_render_quality()
        self.settings.save()

    def _on_ui_style_changed(self, combo):
        style = combo.get_active_id()
        if style is None:
            return
        self.settings.ui_style = style
        self._apply_ui_style()
        # aero implies a glass scope; the switch stays free to override
        if self.settings.scope_glass != (style == "aero"):
            self.glass_switch.set_active(style == "aero")
        # each style remembers its own pane darkness
        self.glass_tint_scale.set_value(
            self.settings.glass_tint_for(style))
        self._apply_theme()
        self.settings.save()

    def _on_precompute_switched(self, _switch, state):
        self.settings.precompute_enabled = state
        playing = self.player.playing_path
        if state and playing is not None and self._precomputed is None:
            self._queue_precompute(playing)
        return False

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

    def _on_max_fps_changed(self, combo):
        text = combo.get_active_id()
        if text is None:                     # typed into the entry
            text = combo.get_child().get_text().strip()
        try:
            value = int(text)
        except ValueError:
            return                           # mid-edit or a preset label
        self.settings.max_fps = max(0, min(1000, value))
        self.settings.save()

    def _on_grid_switched(self, _switch, state):
        self.settings.grid_enabled = state
        self._apply_theme()
        return False

    def _on_amoled_switched(self, _switch, state):
        self.settings.amoled_background = state
        self._apply_theme()
        return False

    def _on_glass_switched(self, _switch, state):
        self.settings.scope_glass = state
        self.glass_tint_scale.set_sensitive(state)
        self._sync_glass_class()
        self._apply_theme()
        return False

    def _on_glass_tint_changed(self, scale):
        self.settings.glass_tints[self.settings.ui_style] = scale.get_value()
        self._apply_theme()

    def _sync_glass_class(self):
        """The glass-scope window class opens each style's smoked pane."""
        style_context = self.get_style_context()
        if self.settings.scope_glass:
            style_context.add_class("glass-scope")
        else:
            style_context.remove_class("glass-scope")

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
                path = phosphor_recorder.save_snapshot(
                    audio, self.settings,
                    sample_rate=self.capture_stream.sample_rate,
                    gain=self._effective_gain,
                    oversample=self._export_detail_oversample())
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
            on_error=lambda message: GLib.idle_add(self._export_failed, message),
            sample_rate=self.capture_stream.sample_rate,
            gain=self._effective_gain,
            oversample=self._export_detail_oversample())

    def _export_done(self, path):
        self.exporting = False
        self.status_label.set_text(f"clip saved: {path}")
        return False

    def _export_failed(self, message):
        self.exporting = False
        self.status_label.set_text(f"export failed: {message}")
        return False

    # ---------------------------------------------------------- compose mode --
    # Draw a shape on the scope; it becomes a constant-speed audio loop that
    # plays out loud and therefore draws itself — the scope run in reverse.

    def _on_compose_toggled(self, toggle):
        if toggle.get_active():
            self._enter_compose_mode()
        else:
            self._exit_compose_mode()

    def _enter_compose_mode(self):
        if self.is_composing:
            return
        self._cancel_reacquire()
        if self.capture_toggle.get_active():
            self.capture_toggle.set_active(False)   # stops capture/playback
        if self.capture_stream.is_running:
            self.capture_stream.stop()              # e.g. a paused track
        self.player.set_file_loaded(False)
        self.is_composing = True
        self._compose_drawing = False
        self._compose_points = []
        self._compose_loop_points = None
        self.mode_combo.set_active_id("xy")         # drawing only makes sense in XY
        self._set_scope_cursor("crosshair")
        self.status_label.set_text(
            "✏ draw a shape on the scope — release to hear it")
        self._start_render_loop()
        self._wake_renderer()

    def _exit_compose_mode(self, stop_loop=True):
        if not self.is_composing:
            return
        self.is_composing = False
        self._compose_drawing = False
        self._compose_points = []
        if self._compose_regenerate_source is not None:
            GLib.source_remove(self._compose_regenerate_source)
            self._compose_regenerate_source = None
        if stop_loop and self.capture_toggle.get_active():
            self.capture_toggle.set_active(False)
        self._set_scope_cursor(None)
        if self.compose_toggle.get_active():
            self.compose_toggle.set_active(False)   # re-entry guarded above

    def _set_scope_cursor(self, cursor_name):
        gdk_window = self.scope_event_box.get_window()
        if gdk_window is None:
            return
        cursor = (Gdk.Cursor.new_from_name(self.get_display(), cursor_name)
                  if cursor_name else None)
        gdk_window.set_cursor(cursor)

    def _advance_compose_preview(self):
        """Restamp the in-progress stroke every frame; with per-frame decay
        this settles at a steady brightness, like a held trace."""
        work_started = time.perf_counter()
        points = self._compose_points
        if len(points) < 2:
            segments = []
        elif numpy is not None:
            coordinates = numpy.asarray(points, dtype=numpy.float32)
            segments = numpy.empty((len(points) - 1, 5), dtype=numpy.float32)
            segments[:, 0:2] = coordinates[:-1]
            segments[:, 2:4] = coordinates[1:]
            segments[:, 4] = COMPOSE_PREVIEW_INTENSITY
        else:
            segments = [(points[i][0], points[i][1],
                         points[i + 1][0], points[i + 1][1],
                         COMPOSE_PREVIEW_INTENSITY)
                        for i in range(len(points) - 1)]
        renderer = self.active_renderer()
        if isinstance(renderer, LiveCairoRenderer):
            renderer.advance_segments(segments)
        else:
            renderer.advance(segments)
        self._frame_work_seconds += time.perf_counter() - work_started

    def _scope_points_from_widget(self, widget_points):
        """Invert the display transform: widget pixels -> signal space, using
        the current gain so the loop plays back exactly where it was drawn."""
        allocation = self.display_stack.get_allocation()
        center_x, center_y = allocation.width / 2, allocation.height / 2
        radius = (min(allocation.width, allocation.height) * 0.45
                  * max(0.001, self._effective_gain))
        return [(max(-1.0, min(1.0, (x - center_x) / radius)),
                 max(-1.0, min(1.0, (center_y - y) / radius)))
                for x, y in widget_points]

    def _finish_compose_stroke(self):
        self._compose_drawing = False
        if len(self._compose_points) < COMPOSE_MINIMUM_POINTS:
            self._compose_points = []
            self.status_label.set_text("✏ shape too small — draw a bigger one")
            return
        self._compose_loop_points = self._scope_points_from_widget(
            self._compose_points)
        self._compose_points = []
        self._restart_compose_loop()

    def _restart_compose_loop(self):
        frequency = phosphor_compose.clamp_frequency(
            self.settings.compose_frequency_hz)
        self.settings.compose_frequency_hz = frequency
        try:
            loop_path = phosphor_compose.write_loop_wav(
                self._compose_loop_points, frequency,
                self.capture_stream.sample_rate)
            self.capture_stream.start_file(loop_path, loop=True)
        except (ValueError, OSError) as error:
            self.status_label.set_text(f"compose failed: {error}")
            return
        self.sync_capture_toggle(True)
        self.quiet_frame_count = 0
        self.status_label.set_text(
            f"✏ {frequency:.0f} Hz loop — scroll to retune, draw to replace")
        self._start_render_loop()

    def _queue_compose_retune(self):
        """Pitch changes regenerate the loop, debounced so a scroll flick
        only restarts the decoder once."""
        if self._compose_regenerate_source is not None:
            GLib.source_remove(self._compose_regenerate_source)
        self._compose_regenerate_source = GLib.timeout_add(
            300, self._compose_retune_now)

    def _compose_retune_now(self):
        self._compose_regenerate_source = None
        if self.is_composing and self._compose_loop_points:
            self._restart_compose_loop()
        return GLib.SOURCE_REMOVE

    def export_drawing(self):
        if not self._compose_loop_points:
            return
        points = list(self._compose_loop_points)
        frequency = self.settings.compose_frequency_hz
        sample_rate = self.capture_stream.sample_rate

        def worker():
            try:
                path = phosphor_compose.export_drawing_wav(points, frequency,
                                                           sample_rate)
                GLib.idle_add(self.status_label.set_text,
                              f"drawing saved: {path}")
            except OSError as error:
                GLib.idle_add(self.status_label.set_text,
                              f"export failed: {error}")
        self.status_label.set_text("writing drawing WAV…")
        threading.Thread(target=worker, daemon=True).start()

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
        if (event.changed_mask & Gdk.WindowState.ICONIFIED
                and not (event.new_window_state & Gdk.WindowState.ICONIFIED)):
            # back from the taskbar: repaint even if the scope was asleep
            self._wake_renderer()
        fullscreen = bool(event.new_window_state & Gdk.WindowState.FULLSCREEN)
        if fullscreen == self._is_fullscreen:
            return False
        self._is_fullscreen = fullscreen
        if not self.is_mini_mode:
            if fullscreen:
                self.controls_container.hide()
                self.header_bar.hide()
                self.player.panel_revealer.hide()
            else:
                self.controls_container.show()
                self.header_bar.show()
                self.player.panel_revealer.show()
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
            self.player.panel_revealer.hide()
            self.set_decorated(False)
            self.set_keep_above(True)
            width = height = self.settings.mini_size
            x, y = self.settings.mini_x, self.settings.mini_y
        else:
            self.settings.mini_x, self.settings.mini_y = self.get_position()
            self._mini_resize_start = None
            self._set_scope_cursor(None)
            self.controls_container.show()
            self.header_bar.show()
            self.player.panel_revealer.show()
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
        self.settings.mini_size = max(140, min(1000, int(size)))
        if self.is_mini_mode:
            self.resize(self.settings.mini_size, self.settings.mini_size)

    def _in_mini_resize_corner(self, x, y):
        allocation = self.display_stack.get_allocation()
        return (allocation.width - x <= MINI_RESIZE_CORNER_PIXELS
                and allocation.height - y <= MINI_RESIZE_CORNER_PIXELS)

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
        add_item("Play audio file…  (O)", self.player.open_audio_file)
        if self.player.playing_file is not None:
            paused = self.capture_stream.playback_paused
            add_item("Resume track" if paused else "Pause track",
                     self.player.toggle_play_pause)
            add_item("Next track  ⏭", self.player.play_next_track,
                     sensitive=len(self.player.playlist) > 1)
            add_item("Previous track  ⏮", self.player.play_previous_track,
                     sensitive=len(self.player.playlist) > 1)
            if len(self.player.playlist) > 1:
                # track picker in the menu, so the mini view can change
                # songs by mouse alone
                tracks_menu = add_submenu("Tracks")
                playlist = self.player.playlist
                current = self.player.playlist_index
                start = 0
                if len(playlist) > 30:      # window big folders around now
                    start = max(0, min(current - 12, len(playlist) - 25))
                    playlist = playlist[start:start + 25]
                for offset, path in enumerate(playlist):
                    add_check_item(
                        os.path.basename(path),
                        start + offset == current,
                        lambda active, p=path: active and self.player
                            .play_file(p, rebuild_playlist=False),
                        radio=True, target_menu=tracks_menu)
        if not self.is_mini_mode:
            add_check_item("Compose · draw a shape  (D)", self.is_composing,
                           lambda active: self.compose_toggle.set_active(active))
            add_check_item("Playlist panel  (L)",
                           self.settings.playlist_panel_open,
                           lambda active: self.playlist_toggle.set_active(active))
        if self.is_composing and self._compose_loop_points:
            add_item("Export drawing as WAV  (10 s)", self.export_drawing)
        add_item("Snapshot  (S)", self.save_snapshot)
        add_item("Save last 10 s  (C)", self.save_clip)
        if self.player.playing_path is not None:
            if self._precomputed is not None:
                add_item("Scope stream: precomputed ✓", lambda: None,
                         sensitive=False)
            elif self._precompute_worker_path is not None:
                add_item("Precomputing scope stream…", lambda: None,
                         sensitive=False)
            else:
                add_item("Precompute scope stream",
                         lambda: self._queue_precompute(
                             self.player.playing_path))
        cache_bytes = phosphor_precompute.cache_size_bytes()
        if cache_bytes:
            add_item(f"Clear precomputed streams ({cache_bytes / 1e6:.0f} MB)",
                     self._clear_precompute_cache)
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
        add_check_item("Show FPS  (F)", self.settings.show_fps,
                       lambda active: self.show_fps_switch.set_active(active))
        add_check_item("Auto gain — fit to screen", self.settings.auto_gain,
                       lambda active: self.auto_gain_switch.set_active(active))
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
            if (self.is_composing and event.button == 1
                    and event.type == Gdk.EventType.BUTTON_PRESS):
                self._compose_drawing = True
                self._compose_points = [(event.x, event.y)]
                self._start_render_loop()
                return True
            return False
        if event.type == Gdk.EventType._2BUTTON_PRESS:
            self.set_mini_mode(False)
        elif event.type == Gdk.EventType.BUTTON_PRESS and event.button == 1:
            if self._in_mini_resize_corner(event.x, event.y):
                self._mini_resize_start = (self.settings.mini_size,
                                           event.x_root, event.y_root)
            else:
                self.begin_move_drag(event.button, int(event.x_root),
                                     int(event.y_root), event.time)
        return True

    def _on_button_release(self, _widget, event):
        if event.button != 1:
            return False
        if self._mini_resize_start is not None:
            self._mini_resize_start = None
            return True
        if self.is_composing and self._compose_drawing:
            self._finish_compose_stroke()
            return True
        return False

    def _on_motion(self, _widget, event):
        if self._mini_resize_start is not None:
            start_size, start_x_root, start_y_root = self._mini_resize_start
            # the window stays square, so the larger axis of the drag wins
            growth = max(event.x_root - start_x_root,
                         event.y_root - start_y_root)
            self._resize_mini(start_size + growth)
            return True
        if self.is_mini_mode:
            self._set_scope_cursor(
                "se-resize" if self._in_mini_resize_corner(event.x, event.y)
                else None)
            return False
        if self.is_composing and self._compose_drawing:
            last_x, last_y = self._compose_points[-1]
            if (event.x - last_x) ** 2 + (event.y - last_y) ** 2 >= 4.0:
                self._compose_points.append((event.x, event.y))
            return True
        return False

    def _on_scroll(self, _widget, event):
        scroll_up = event.direction == Gdk.ScrollDirection.UP
        if self.is_mini_mode and event.state & Gdk.ModifierType.CONTROL_MASK:
            factor = 1.12 if scroll_up else 1 / 1.12
            self._resize_mini(self.settings.mini_size * factor)
            return True
        if self.is_composing and not self.is_mini_mode:
            factor = 1.06 if scroll_up else 1 / 1.06
            self.settings.compose_frequency_hz = phosphor_compose.clamp_frequency(
                self.settings.compose_frequency_hz * factor)
            frequency = self.settings.compose_frequency_hz
            if self._compose_loop_points:
                self.status_label.set_text(f"✏ retuning… {frequency:.0f} Hz")
                self._queue_compose_retune()
            else:
                self.status_label.set_text(f"✏ loop pitch: {frequency:.0f} Hz")
            return True
        factor = 1.12 if scroll_up else 1 / 1.12
        self.gain_scale.set_value(min(6.0, max(0.1, self.settings.gain * factor)))
        return True

    def _on_key_press(self, _widget, event):
        if isinstance(self.get_focus(), Gtk.Entry):
            return False    # typing in a percent box must not fire shortcuts
        key = event.keyval
        sequence = (Gdk.KEY_Up, Gdk.KEY_Up, Gdk.KEY_Down, Gdk.KEY_Down,
                    Gdk.KEY_Left, Gdk.KEY_Right, Gdk.KEY_Left, Gdk.KEY_Right,
                    Gdk.KEY_b, Gdk.KEY_a)
        if key == sequence[self._konami_progress]:
            self._konami_progress += 1
            if self._konami_progress == len(sequence):
                self._konami_progress = 0
                self._begin_visitor()
                return True
        else:
            self._konami_progress = 1 if key == sequence[0] else 0
        if key == Gdk.KEY_space:
            self.capture_toggle.set_active(not self.capture_toggle.get_active())
        elif key in (Gdk.KEY_o, Gdk.KEY_O):
            self.player.open_audio_file()
        elif key in (Gdk.KEY_d, Gdk.KEY_D):
            self.compose_toggle.set_active(not self.compose_toggle.get_active())
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
        elif key in (Gdk.KEY_l, Gdk.KEY_L):
            self.playlist_toggle.set_active(
                not self.playlist_toggle.get_active())
        elif key in (Gdk.KEY_f, Gdk.KEY_F):
            self.show_fps_switch.set_active(not self.settings.show_fps)
        elif key == Gdk.KEY_F11:
            self.toggle_fullscreen()
        elif key == Gdk.KEY_Escape:
            if self.is_composing:
                self.compose_toggle.set_active(False)
            elif self._is_fullscreen:
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
    def __init__(self, start_in_mini=False):
        super().__init__(application_id=APPLICATION_ID)
        self.start_in_mini = start_in_mini

    def do_activate(self):
        window = self.props.active_window
        if window is None:
            window = OscilloscopeWindow(application=self)
            window.show_all()
            window.apply_remembered_view()
            if self.start_in_mini:        # launched as a floating preview (--mini)
                window.set_mini_mode(True)
            window.capture_toggle.set_active(True)  # start live; off is free
        window.present()


def _set_process_name(name):
    """Label the process (PR_SET_NAME) so it shows as `name` in task managers."""
    try:
        import ctypes
        libc = ctypes.CDLL("libc.so.6", use_errno=True)
        buffer = ctypes.create_string_buffer(name.encode()[:15])
        libc.prctl(15, ctypes.byref(buffer), 0, 0, 0)  # 15 = PR_SET_NAME
    except Exception:
        pass


def main():
    _set_process_name("phosphor")
    GLib.set_prgname("phosphor")   # ties the window to phosphor.desktop's icon
    PhosphorApplication(start_in_mini="--mini" in sys.argv[1:]).run(None)


if __name__ == "__main__":
    main()
