#!/usr/bin/env python3
"""Phosphor — a software XY oscilloscope for everything your PC plays.

In XY mode the left audio channel drives the beam horizontally and the
right channel drives it vertically, so "oscilloscope music"
(Jerobeam Fenderson and friends) draws its hidden pictures on screen.
A conventional waveform mode is included for ordinary listening.

Audio is tapped from a PulseAudio/PipeWire monitor source using `parec`,
so any sound the system plays can be visualized without rerouting
anything. While capture is toggled off the stream is closed entirely and
the render loop stops, so the app costs nothing in the background —
PipeWire suspends the monitor source again on its own.

Keys:  Space = toggle capture   M = mini mode   Scroll = gain   Q = quit
Mini mode: drag with the left mouse button, double-click to restore.
"""

import math
import re
import subprocess
import threading
from array import array

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, Gdk, GLib  # noqa: E402

import cairo  # noqa: E402

SAMPLE_RATE = 48000
BYTES_PER_STEREO_FRAME = 8          # 2 channels x float32
MAX_PENDING_BYTES = SAMPLE_RATE * BYTES_PER_STEREO_FRAME  # cap backlog at 1 s
MAX_POINTS_PER_RENDER_FRAME = 4000  # bound cairo work if the UI hiccups
PHOSPHOR_COLOR = (0.35, 1.0, 0.45)  # classic green CRT
ALPHA_BUCKET_COUNT = 10             # beam segments grouped by brightness


# --------------------------------------------------------------------------
# Audio source discovery (PulseAudio / PipeWire via pactl)
# --------------------------------------------------------------------------

class AudioSource:
    def __init__(self, device_name, description, is_monitor):
        self.device_name = device_name
        self.description = description
        self.is_monitor = is_monitor

    @property
    def label(self):
        direction = "OUT" if self.is_monitor else "IN"
        text = self.description
        if text.startswith("Monitor of "):
            text = text[len("Monitor of "):]
        return f"{direction} · {text}"


def list_audio_sources():
    """Return every capturable source: sink monitors first, then inputs."""
    try:
        output = subprocess.run(
            ["pactl", "list", "sources"],
            capture_output=True, text=True, timeout=5,
        ).stdout
    except (OSError, subprocess.TimeoutExpired):
        return []

    sources = []
    for block in re.split(r"^Source #\d+", output, flags=re.MULTILINE):
        name_match = re.search(r"^\s*Name:\s*(\S+)", block, re.MULTILINE)
        description_match = re.search(r"^\s*Description:\s*(.+)$", block, re.MULTILINE)
        if not name_match:
            continue
        device_name = name_match.group(1)
        description = description_match.group(1).strip() if description_match else device_name
        sources.append(AudioSource(device_name, description, device_name.endswith(".monitor")))

    sources.sort(key=lambda source: (not source.is_monitor, source.label))
    return sources


def default_monitor_device():
    """The monitor of whatever sink the system is currently playing through."""
    try:
        sink = subprocess.run(
            ["pactl", "get-default-sink"],
            capture_output=True, text=True, timeout=5,
        ).stdout.strip()
        return f"{sink}.monitor" if sink else None
    except (OSError, subprocess.TimeoutExpired):
        return None


# --------------------------------------------------------------------------
# Capture: a parec subprocess feeding a byte buffer from a reader thread
# --------------------------------------------------------------------------

class AudioCaptureStream:
    """Owns the parec process. While stopped, nothing runs and nothing polls."""

    def __init__(self, on_stream_ended):
        self._process = None
        self._reader_thread = None
        self._pending_bytes = bytearray()
        self._lock = threading.Lock()
        self._on_stream_ended = on_stream_ended  # called from reader thread

    @property
    def is_running(self):
        return self._process is not None

    def start(self, device_name):
        self.stop()
        self._process = subprocess.Popen(
            [
                "parec",
                f"--device={device_name}",
                "--format=float32le",
                f"--rate={SAMPLE_RATE}",
                "--channels=2",
                "--latency-msec=20",
                "--raw",
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        self._reader_thread = threading.Thread(
            target=self._reader_loop, args=(self._process,), daemon=True
        )
        self._reader_thread.start()

    def stop(self):
        process, self._process = self._process, None
        if process is not None:
            process.terminate()
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                process.kill()
        with self._lock:
            self._pending_bytes.clear()

    def _reader_loop(self, process):
        while True:
            chunk = process.stdout.read(8192)
            if not chunk:
                break
            with self._lock:
                self._pending_bytes.extend(chunk)
                overflow = len(self._pending_bytes) - MAX_PENDING_BYTES
                if overflow > 0:
                    # Drop the oldest audio, keeping stereo-frame alignment.
                    overflow -= overflow % BYTES_PER_STEREO_FRAME
                    del self._pending_bytes[:overflow]
        if self._process is process:
            # The stream died on its own (device removed, daemon restart…).
            self._on_stream_ended()

    def take_stereo_samples(self):
        """Drain captured audio as a flat float array [L, R, L, R, ...]."""
        with self._lock:
            usable = len(self._pending_bytes) - (len(self._pending_bytes) % BYTES_PER_STEREO_FRAME)
            if usable == 0:
                return array("f")
            raw = bytes(self._pending_bytes[:usable])
            del self._pending_bytes[:usable]
        samples = array("f")
        samples.frombytes(raw)
        return samples


# --------------------------------------------------------------------------
# Rendering: persistent phosphor surface with decay + speed-based brightness
# --------------------------------------------------------------------------

class PhosphorRenderer:
    """Simulates a CRT: a glow surface that fades each frame, onto which the
    beam draws additively. Segment brightness falls with beam speed, which is
    what makes oscilloscope music look like it does on a real scope."""

    def __init__(self):
        self.surface = None
        self.width = 0
        self.height = 0
        self.persistence = 0.7       # 0..1, how long the glow lingers
        self.gain = 1.0
        self.beam_energy = 8.0       # brightness budget per pixel of travel
        self.last_beam_x = None
        self.last_beam_y = None

    def ensure_surface(self, width, height):
        if self.surface is None or width != self.width or height != self.height:
            self.width, self.height = width, height
            self.surface = cairo.ImageSurface(cairo.FORMAT_ARGB32, width, height)
            self.last_beam_x = self.last_beam_y = None

    def fade_frame(self):
        fade_alpha = max(0.02, (1.0 - self.persistence) * 0.6)
        context = cairo.Context(self.surface)
        context.set_operator(cairo.OPERATOR_DEST_OUT)
        context.set_source_rgba(0, 0, 0, fade_alpha)
        context.paint()

    def draw_xy(self, samples):
        """samples is a flat [L, R, L, R, ...] float array."""
        if len(samples) > 2 * MAX_POINTS_PER_RENDER_FRAME:
            samples = samples[-2 * MAX_POINTS_PER_RENDER_FRAME:]
            self.last_beam_x = self.last_beam_y = None

        center_x, center_y = self.width / 2, self.height / 2
        radius = min(self.width, self.height) * 0.45

        segments_by_alpha_bucket = [[] for _ in range(ALPHA_BUCKET_COUNT)]
        previous_x, previous_y = self.last_beam_x, self.last_beam_y
        for index in range(0, len(samples) - 1, 2):
            x = center_x + samples[index] * self.gain * radius
            y = center_y - samples[index + 1] * self.gain * radius
            if previous_x is not None:
                distance = math.hypot(x - previous_x, y - previous_y)
                alpha = min(1.0, self.beam_energy / (distance + 0.7))
                bucket = min(ALPHA_BUCKET_COUNT - 1, int(alpha * ALPHA_BUCKET_COUNT))
                segments_by_alpha_bucket[bucket].append((previous_x, previous_y, x, y))
            previous_x, previous_y = x, y
        self.last_beam_x, self.last_beam_y = previous_x, previous_y

        context = cairo.Context(self.surface)
        context.set_operator(cairo.OPERATOR_ADD)
        context.set_line_width(2.0)
        context.set_line_cap(cairo.LINE_CAP_ROUND)
        red, green, blue = PHOSPHOR_COLOR
        for bucket, segments in enumerate(segments_by_alpha_bucket):
            if not segments:
                continue
            context.set_source_rgba(red, green, blue, (bucket + 0.5) / ALPHA_BUCKET_COUNT)
            for start_x, start_y, end_x, end_y in segments:
                context.move_to(start_x, start_y)
                context.line_to(end_x, end_y)
            context.stroke()

    def draw_waveform(self, history):
        """history is a flat [L, R, ...] float array of the most recent audio."""
        if len(history) < 4:
            return
        context = cairo.Context(self.surface)
        context.set_operator(cairo.OPERATOR_ADD)
        context.set_line_width(1.5)
        red, green, blue = PHOSPHOR_COLOR
        context.set_source_rgba(red, green, blue, 0.85)

        frame_count = len(history) // 2
        amplitude = self.height * 0.22 * self.gain
        for channel, baseline in ((0, self.height * 0.28), (1, self.height * 0.72)):
            step = max(1, frame_count // max(1, self.width))
            context.move_to(0, baseline - history[channel] * amplitude)
            for frame_index in range(step, frame_count, step):
                x = self.width * frame_index / frame_count
                y = baseline - history[2 * frame_index + channel] * amplitude
                context.line_to(x, y)
            context.stroke()

    def composite(self, context, width, height):
        context.set_source_rgb(0.02, 0.03, 0.02)
        context.paint()
        self._draw_graticule(context, width, height)
        if self.surface is not None:
            context.set_source_surface(self.surface, 0, 0)
            context.paint()

    @staticmethod
    def _draw_graticule(context, width, height):
        red, green, blue = PHOSPHOR_COLOR
        context.set_line_width(1.0)
        context.set_source_rgba(red, green, blue, 0.07)
        divisions = 8
        for division in range(1, divisions):
            x = width * division / divisions
            y = height * division / divisions
            context.move_to(x, 0); context.line_to(x, height)
            context.move_to(0, y); context.line_to(width, y)
        context.stroke()
        context.set_source_rgba(red, green, blue, 0.16)
        context.move_to(width / 2, 0); context.line_to(width / 2, height)
        context.move_to(0, height / 2); context.line_to(width, height / 2)
        context.stroke()


# --------------------------------------------------------------------------
# The window
# --------------------------------------------------------------------------

class OscilloscopeWindow(Gtk.Window):
    WAVEFORM_HISTORY_FRAMES = 4096

    def __init__(self):
        super().__init__(title="Phosphor")
        self.set_default_size(720, 600)

        self.renderer = PhosphorRenderer()
        self.capture_stream = AudioCaptureStream(
            on_stream_ended=lambda: GLib.idle_add(self._handle_stream_died)
        )
        self.display_mode = "xy"
        self.waveform_history = array("f")
        self.is_mini_mode = False
        self.tick_callback_id = None
        self.fade_out_frames_remaining = 0
        self.quiet_frame_count = 0
        self.normal_size = (720, 600)

        layout = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
        self.toolbar = self._build_toolbar()
        layout.pack_start(self.toolbar, False, False, 0)

        self.drawing_area = Gtk.DrawingArea()
        self.drawing_area.connect("draw", self._on_draw)
        self.drawing_area.add_events(
            Gdk.EventMask.BUTTON_PRESS_MASK | Gdk.EventMask.SCROLL_MASK
        )
        self.drawing_area.connect("button-press-event", self._on_button_press)
        self.drawing_area.connect("scroll-event", self._on_scroll)
        layout.pack_start(self.drawing_area, True, True, 0)
        self.add(layout)

        self.connect("key-press-event", self._on_key_press)
        self.connect("destroy", self._on_destroy)

    # -- toolbar -----------------------------------------------------------

    def _build_toolbar(self):
        toolbar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=6)
        toolbar.set_margin_start(6); toolbar.set_margin_end(6)
        toolbar.set_margin_top(4); toolbar.set_margin_bottom(4)

        self.capture_toggle = Gtk.ToggleButton(label="⏻ Live")
        self.capture_toggle.set_tooltip_text("Toggle audio capture (Space). Off = zero CPU.")
        self.capture_toggle.connect("toggled", self._on_capture_toggled)
        toolbar.pack_start(self.capture_toggle, False, False, 0)

        self.source_combo = Gtk.ComboBoxText()
        self.source_combo.set_tooltip_text("Audio source: OUT = what the PC plays, IN = microphones")
        self._populate_sources()
        self.source_combo.connect("changed", self._on_source_changed)
        toolbar.pack_start(self.source_combo, False, False, 0)

        refresh_button = Gtk.Button.new_from_icon_name("view-refresh-symbolic", Gtk.IconSize.BUTTON)
        refresh_button.set_tooltip_text("Re-scan audio devices")
        refresh_button.connect("clicked", lambda _button: self._populate_sources())
        toolbar.pack_start(refresh_button, False, False, 0)

        self.mode_combo = Gtk.ComboBoxText()
        self.mode_combo.append("xy", "XY (scope art)")
        self.mode_combo.append("wave", "Waveform")
        self.mode_combo.set_active_id("xy")
        self.mode_combo.connect("changed", self._on_mode_changed)
        toolbar.pack_start(self.mode_combo, False, False, 0)

        toolbar.pack_start(self._make_slider(
            "Gain", 0.1, 6.0, self.renderer.gain,
            lambda value: setattr(self.renderer, "gain", value)), False, False, 0)
        toolbar.pack_start(self._make_slider(
            "Glow", 0.0, 0.98, self.renderer.persistence,
            lambda value: setattr(self.renderer, "persistence", value)), False, False, 0)
        toolbar.pack_start(self._make_slider(
            "Beam", 1.0, 30.0, self.renderer.beam_energy,
            lambda value: setattr(self.renderer, "beam_energy", value)), False, False, 0)

        self.status_label = Gtk.Label(label="idle")
        self.status_label.set_ellipsize(3)  # Pango.EllipsizeMode.END
        toolbar.pack_end(self.status_label, True, True, 0)

        mini_button = Gtk.Button(label="Mini")
        mini_button.set_tooltip_text("Borderless always-on-top mini view (M). Double-click it to restore.")
        mini_button.connect("clicked", lambda _button: self.set_mini_mode(True))
        toolbar.pack_end(mini_button, False, False, 0)
        return toolbar

    def _make_slider(self, name, low, high, initial, on_change):
        box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=2)
        box.pack_start(Gtk.Label(label=name), False, False, 0)
        scale = Gtk.Scale.new_with_range(Gtk.Orientation.HORIZONTAL, low, high, (high - low) / 100)
        scale.set_value(initial)
        scale.set_draw_value(False)
        scale.set_size_request(85, -1)
        scale.connect("value-changed", lambda widget: on_change(widget.get_value()))
        box.pack_start(scale, False, False, 0)
        if name == "Gain":
            self.gain_scale = scale
        return box

    def _populate_sources(self):
        previous_choice = self.source_combo.get_active_id()
        self.source_combo.remove_all()
        for source in list_audio_sources():
            self.source_combo.append(source.device_name, source.label)
        preferred = previous_choice or default_monitor_device()
        if preferred:
            self.source_combo.set_active_id(preferred)
        if self.source_combo.get_active_id() is None:
            self.source_combo.set_active(0)

    # -- capture lifecycle ---------------------------------------------------

    def _on_capture_toggled(self, toggle):
        if toggle.get_active():
            device = self.source_combo.get_active_id()
            if device is None:
                toggle.set_active(False)
                return
            try:
                self.capture_stream.start(device)
            except OSError as error:
                self.status_label.set_text(f"capture failed: {error}")
                toggle.set_active(False)
                return
            self.status_label.set_text("● live")
            self._start_render_loop()
        else:
            self.capture_stream.stop()
            self.status_label.set_text("idle — no capture, no CPU")
            self.fade_out_frames_remaining = 90  # let the glow die out

    def _on_source_changed(self, _combo):
        if self.capture_stream.is_running and self.source_combo.get_active_id():
            self.capture_stream.start(self.source_combo.get_active_id())

    def _on_mode_changed(self, combo):
        self.display_mode = combo.get_active_id() or "xy"
        self.waveform_history = array("f")

    def _handle_stream_died(self):
        if self.capture_toggle.get_active():
            self.capture_toggle.set_active(False)
            self.status_label.set_text("stream ended — device gone?")
            self._populate_sources()

    # -- render loop ---------------------------------------------------------

    def _start_render_loop(self):
        if self.tick_callback_id is None:
            self.tick_callback_id = self.drawing_area.add_tick_callback(self._on_tick)

    def _on_tick(self, _widget, _frame_clock):
        allocation = self.drawing_area.get_allocation()
        if allocation.width < 2 or allocation.height < 2:
            return GLib.SOURCE_CONTINUE
        self.renderer.ensure_surface(allocation.width, allocation.height)

        if self.capture_stream.is_running:
            samples = self.capture_stream.take_stereo_samples()
            # The monitor delivers zeros while nothing plays, so detect silence
            # by content and stop redrawing once the glow has settled — an
            # idle-but-armed scope then costs almost nothing.
            is_quiet = not samples or max(max(samples), -min(samples)) < 1e-4
            self.quiet_frame_count = self.quiet_frame_count + 1 if is_quiet else 0
            if self.quiet_frame_count > 120:
                return GLib.SOURCE_CONTINUE
            self.renderer.fade_frame()
            if self.display_mode == "xy":
                self.renderer.draw_xy(samples)
            else:
                self.waveform_history.extend(samples)
                excess = len(self.waveform_history) - 2 * self.WAVEFORM_HISTORY_FRAMES
                if excess > 0:
                    del self.waveform_history[:excess]
                self.renderer.draw_waveform(self.waveform_history)
        else:
            self.renderer.fade_frame()
            self.fade_out_frames_remaining -= 1
            if self.fade_out_frames_remaining <= 0:
                self.drawing_area.queue_draw()
                self.tick_callback_id = None
                return GLib.SOURCE_REMOVE

        self.drawing_area.queue_draw()
        return GLib.SOURCE_CONTINUE

    def _on_draw(self, _widget, context):
        allocation = self.drawing_area.get_allocation()
        self.renderer.composite(context, allocation.width, allocation.height)
        return False

    # -- mini mode & input -----------------------------------------------------

    def set_mini_mode(self, enabled):
        if enabled == self.is_mini_mode:
            return
        self.is_mini_mode = enabled
        if enabled:
            self.normal_size = self.get_size()
            self.toolbar.hide()
            self.set_decorated(False)
            self.set_keep_above(True)
            self.resize(260, 260)
        else:
            self.toolbar.show()
            self.set_decorated(True)
            self.set_keep_above(False)
            self.resize(*self.normal_size)

    def _on_button_press(self, _widget, event):
        if not self.is_mini_mode:
            return False
        if event.type == Gdk.EventType._2BUTTON_PRESS:
            self.set_mini_mode(False)
        elif event.type == Gdk.EventType.BUTTON_PRESS and event.button == 1:
            self.begin_move_drag(event.button, int(event.x_root), int(event.y_root), event.time)
        return True

    def _on_scroll(self, _widget, event):
        factor = 1.12 if event.direction == Gdk.ScrollDirection.UP else 1 / 1.12
        new_gain = min(6.0, max(0.1, self.renderer.gain * factor))
        self.gain_scale.set_value(new_gain)
        return True

    def _on_key_press(self, _widget, event):
        key = event.keyval
        if key == Gdk.KEY_space:
            self.capture_toggle.set_active(not self.capture_toggle.get_active())
        elif key in (Gdk.KEY_m, Gdk.KEY_M):
            self.set_mini_mode(not self.is_mini_mode)
        elif key in (Gdk.KEY_q, Gdk.KEY_Q, Gdk.KEY_Escape):
            self.close()
        else:
            return False
        return True

    def _on_destroy(self, _widget):
        self.capture_stream.stop()
        Gtk.main_quit()


def main():
    window = OscilloscopeWindow()
    window.show_all()
    window.capture_toggle.set_active(True)  # start live; toggle off costs nothing
    Gtk.main()


if __name__ == "__main__":
    main()
