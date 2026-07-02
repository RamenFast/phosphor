# SPDX-License-Identifier: GPL-3.0-or-later
"""File playback for Phosphor: the built-in media player.

Owns everything about playing audio files on the scope — the folder
playlist, the headerbar transport and seek slider, pause/resume, and track
auto-advance. The window stays the coordinator: the player drives the shared
capture stream (ffmpeg → pacat) and asks the window to sync its Live toggle
and render loop.
"""

import os
import threading

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, GLib  # noqa: E402

from phosphor_audio import probe_duration_seconds

AUDIO_FILE_EXTENSIONS = (".mp3", ".flac", ".ogg", ".oga", ".opus", ".wav",
                         ".m4a", ".aac", ".wma", ".aif", ".aiff", ".mka")


def format_time(seconds):
    return f"{int(seconds) // 60}:{int(seconds) % 60:02d}"


class PhosphorPlayer:
    """Playback state + transport widgets; `window` is the coordinator."""

    def __init__(self, window):
        self.window = window
        self.settings = window.settings
        self.capture_stream = window.capture_stream
        self.playing_file = None           # basename while a file is loaded
        self.playing_path = None
        self.track_duration_seconds = None
        self.playlist = []
        self.playlist_index = -1
        self._position_update_source = None
        self._seek_debounce_source = None
        self._build_transport_widgets()

    # ---------------------------------------------------------------- widgets

    def _build_transport_widgets(self):
        # transport appears only while a file is loaded
        self.transport_box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL)
        self.transport_box.get_style_context().add_class("linked")
        for icon_name, tooltip, callback in (
                ("media-skip-backward-symbolic", "Previous track in folder",
                 self.play_previous_track),
                (None, "Play/pause the loaded file", self.toggle_play_pause),
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

        # track position: a seek slider plus elapsed/total readout, shown
        # beside the transport whenever a file with a known length plays
        self.position_box = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL,
                                    spacing=6)
        self.position_scale = Gtk.Scale.new_with_range(
            Gtk.Orientation.HORIZONTAL, 0.0, 1.0, 1.0)
        self.position_scale.set_draw_value(False)
        self.position_scale.set_size_request(170, -1)
        self.position_scale.set_tooltip_text("Track position — drag to seek")
        self.position_scale.connect("change-value",
                                    self._on_position_slider_moved)
        self.position_box.pack_start(self.position_scale, True, True, 0)
        self.time_label = Gtk.Label(label="0:00 / 0:00")
        self.position_box.pack_start(self.time_label, False, False, 0)
        self.position_box.set_no_show_all(True)

    # ---------------------------------------------------------------- playback

    def open_audio_file(self):
        dialog = Gtk.FileChooserNative.new("Play audio file", self.window,
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
        window = self.window
        window._cancel_reacquire()
        window._exit_compose_mode(stop_loop=False)
        try:
            self.capture_stream.start_file(path)
        except OSError as error:
            window.status_label.set_text(
                f"file playback failed: {error} (is ffmpeg installed?)")
            return
        if rebuild_playlist:
            self._build_playlist(path)
        else:
            self.playlist_index = self.playlist.index(path)
        self.playing_file = os.path.basename(path)
        self.playing_path = path
        self.set_file_loaded(True)
        self._setup_position_slider(path)
        window.sync_capture_toggle(True)
        window.quiet_frame_count = 0
        window.status_label.set_text(f"▶ {self.playing_file}")
        window._start_render_loop()

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

    def set_file_loaded(self, loaded):
        if not loaded:
            self.playing_file = None
            self.playing_path = None
            self.track_duration_seconds = None
            self.position_box.hide()
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

    def toggle_play_pause(self):
        if self.playing_file is None:
            return
        window = self.window
        paused = not self.capture_stream.playback_paused
        self.capture_stream.set_playback_paused(paused)
        self.play_pause_image.set_from_icon_name(
            "media-playback-start-symbolic" if paused
            else "media-playback-pause-symbolic", Gtk.IconSize.BUTTON)
        # the Live toggle mirrors whether the track is audible
        window.sync_capture_toggle(not paused)
        window.status_label.set_text(
            f"{'⏸' if paused else '▶'} {self.playing_file}")
        if not paused:
            window.quiet_frame_count = 0
            window._start_render_loop()

    def handle_track_finished(self, finished_file):
        """Natural end of a track: keep the album going."""
        if self.playlist and self.playlist_index < len(self.playlist) - 1:
            self.play_next_track()
        else:
            self.set_file_loaded(False)
            self.window.status_label.set_text(f"finished: {finished_file}")

    # ---------------------------------------------------------- track position

    def _setup_position_slider(self, path):
        """Show the seek slider whenever ffprobe can tell us the length.
        The probe runs on a worker thread so track starts never stall the
        render loop or audio pipeline."""
        self.track_duration_seconds = None
        self.position_box.hide()

        def probe_worker():
            duration = probe_duration_seconds(path)
            GLib.idle_add(self._show_position_slider, path, duration)
        threading.Thread(target=probe_worker, daemon=True).start()

    def _show_position_slider(self, path, duration):
        if path != self.playing_path or duration is None or duration <= 0:
            return False    # track changed while probing, or length unknown
        self.track_duration_seconds = duration
        self.position_scale.set_range(0.0, duration)
        self.position_scale.set_value(0.0)
        self._refresh_time_label(0.0)
        self.position_box.set_no_show_all(False)
        self.position_box.show_all()
        if self._position_update_source is None:
            self._position_update_source = GLib.timeout_add(
                250, self._update_position_display)
        return False

    def _refresh_time_label(self, position_seconds):
        self.time_label.set_text(
            f"{format_time(position_seconds)} / "
            f"{format_time(self.track_duration_seconds or 0)}")

    def _update_position_display(self):
        if self.playing_file is None or self.track_duration_seconds is None:
            self._position_update_source = None
            return GLib.SOURCE_REMOVE
        # while a user drag is pending, the slider belongs to the user
        if self._seek_debounce_source is None:
            position = min(self.capture_stream.playback_position_seconds,
                           self.track_duration_seconds)
            self.position_scale.set_value(position)
            self._refresh_time_label(position)
        return GLib.SOURCE_CONTINUE

    def _on_position_slider_moved(self, _scale, _scroll_type, value):
        """User input on the seek slider; seeks are debounced so dragging
        doesn't restart the decoder dozens of times."""
        if self.playing_path is None or self.track_duration_seconds is None:
            return False
        target = max(0.0, min(float(value), self.track_duration_seconds))
        self._refresh_time_label(target)
        if self._seek_debounce_source is not None:
            GLib.source_remove(self._seek_debounce_source)
        self._seek_debounce_source = GLib.timeout_add(
            250, self._perform_seek, target)
        return False    # let the slider follow the pointer

    def _perform_seek(self, target_seconds):
        self._seek_debounce_source = None
        if self.playing_path is None:
            return GLib.SOURCE_REMOVE
        was_paused = self.capture_stream.playback_paused
        try:
            self.capture_stream.start_file(self.playing_path,
                                           seek_seconds=target_seconds)
        except OSError as error:
            self.window.status_label.set_text(f"seek failed: {error}")
            return GLib.SOURCE_REMOVE
        if was_paused:
            self.capture_stream.set_playback_paused(True)
        else:
            self.window.quiet_frame_count = 0
            self.window._start_render_loop()
        return GLib.SOURCE_REMOVE
