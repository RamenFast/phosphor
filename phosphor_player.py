# SPDX-License-Identifier: GPL-3.0-or-later
"""File playback for Phosphor: the built-in media player.

Owns everything about playing audio files on the scope — the folder
playlist and its side panel, shuffle/repeat, the headerbar transport, seek
slider and volume, track metadata (tags feed the now-playing overlay and
MPRIS), pause/resume, and auto-advance. The window stays the coordinator:
the player drives the shared capture stream (ffmpeg → pacat) and asks the
window to sync its Live toggle and render loop.
"""

import os
import random
import subprocess
import threading

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, GLib  # noqa: E402

import phosphor_mpris
from phosphor_audio import probe_metadata

AUDIO_FILE_EXTENSIONS = (".mp3", ".flac", ".ogg", ".oga", ".opus", ".wav",
                         ".m4a", ".aac", ".wma", ".aif", ".aiff", ".mka")

# quiet nods to the artists this scope was built around
ARTIST_NODS = {"jerobeam fenderson": "🍄 the real deal",
               "brakence": "🫧 there are hidden pictures in here"}


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
        self.current_metadata = None       # ffprobe tags for the loaded file
        self.playlist = []
        self.playlist_index = -1
        self._position_update_source = None
        self._seek_debounce_source = None
        self._volume_apply_source = None
        self._build_transport_widgets()
        self._build_playlist_panel()
        self.mpris = phosphor_mpris.MprisService(self)

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
        self.volume_button = Gtk.VolumeButton()
        self.volume_button.set_value(self.settings.playback_volume)
        self.volume_button.set_tooltip_text(
            "Track volume — just this stream, not the whole system")
        self.volume_button.connect("value-changed", self._on_volume_changed)
        self.transport_box.pack_start(self.volume_button, False, False, 0)
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

    def _build_playlist_panel(self):
        """Side panel: the folder playlist with shuffle and repeat."""
        self.panel_revealer = Gtk.Revealer()
        self.panel_revealer.set_transition_type(
            Gtk.RevealerTransitionType.SLIDE_LEFT)
        self.panel_revealer.set_reveal_child(self.settings.playlist_panel_open)

        panel = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=2)
        panel.set_name("playlist-panel")
        panel.set_size_request(250, -1)

        header = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=2)
        for edge in ("start", "end", "top"):
            getattr(header, f"set_margin_{edge}")(4)
        self.shuffle_toggle = Gtk.ToggleButton()
        self.shuffle_toggle.add(Gtk.Image.new_from_icon_name(
            "media-playlist-shuffle-symbolic", Gtk.IconSize.BUTTON))
        self.shuffle_toggle.set_tooltip_text("Shuffle")
        self.shuffle_toggle.set_active(self.settings.shuffle)
        self.shuffle_toggle.connect("toggled", self._on_shuffle_toggled)
        header.pack_start(self.shuffle_toggle, False, False, 0)

        self.repeat_button = Gtk.Button()
        self.repeat_image = Gtk.Image.new_from_icon_name(
            "media-playlist-repeat-symbolic", Gtk.IconSize.BUTTON)
        self.repeat_button.add(self.repeat_image)
        self.repeat_button.connect("clicked", lambda _b: self._cycle_repeat())
        self._show_repeat_state()
        header.pack_start(self.repeat_button, False, False, 0)

        self.playlist_count_label = Gtk.Label()
        self.playlist_count_label.set_halign(Gtk.Align.END)
        header.pack_end(self.playlist_count_label, True, True, 4)
        panel.pack_start(header, False, False, 0)

        self.playlist_box = Gtk.ListBox()
        self.playlist_box.set_activate_on_single_click(False)
        self.playlist_box.connect("row-activated", self._on_playlist_row_activated)
        scroller = Gtk.ScrolledWindow()
        scroller.set_policy(Gtk.PolicyType.NEVER, Gtk.PolicyType.AUTOMATIC)
        scroller.add(self.playlist_box)
        panel.pack_start(scroller, True, True, 0)
        self.panel_revealer.add(panel)

    def refresh_playlist_panel(self):
        for child in self.playlist_box.get_children():
            self.playlist_box.remove(child)
        for index, path in enumerate(self.playlist):
            row = Gtk.ListBoxRow()
            is_current = index == self.playlist_index
            label = Gtk.Label()
            name = GLib.markup_escape_text(os.path.basename(path))
            label.set_markup(f"<b>▶ {name}</b>" if is_current else name)
            label.set_xalign(0.0)
            label.set_ellipsize(2)      # Pango.EllipsizeMode.MIDDLE
            label.set_tooltip_text(os.path.basename(path))
            for edge in ("start", "end"):
                getattr(label, f"set_margin_{edge}")(8)
            label.set_margin_top(3)
            label.set_margin_bottom(3)
            row.add(label)
            self.playlist_box.add(row)
            if is_current:
                self.playlist_box.select_row(row)
        self.playlist_box.show_all()
        count = len(self.playlist)
        self.playlist_count_label.set_text(
            f"{count} track{'s' if count != 1 else ''}" if count else "")

    def _on_playlist_row_activated(self, _box, row):
        index = row.get_index()
        if 0 <= index < len(self.playlist):
            self.play_file(self.playlist[index], rebuild_playlist=False)

    def set_panel_open(self, open_panel):
        self.settings.playlist_panel_open = open_panel
        self.panel_revealer.set_reveal_child(open_panel)

    def _on_shuffle_toggled(self, toggle):
        self.settings.shuffle = toggle.get_active()
        self.settings.save()

    def _cycle_repeat(self):
        order = ("off", "all", "one")
        current = order.index(self.settings.repeat_mode) \
            if self.settings.repeat_mode in order else 0
        self.settings.repeat_mode = order[(current + 1) % len(order)]
        self._show_repeat_state()
        self.settings.save()

    def _show_repeat_state(self):
        mode = self.settings.repeat_mode
        icon_name = ("media-playlist-repeat-song-symbolic" if mode == "one"
                     else "media-playlist-repeat-symbolic")
        self.repeat_image.set_from_icon_name(icon_name, Gtk.IconSize.BUTTON)
        self.repeat_image.set_opacity(1.0 if mode != "off" else 0.4)
        self.repeat_button.set_tooltip_text(f"Repeat: {mode}")

    # ------------------------------------------------------------------ volume

    def _on_volume_changed(self, _button, value):
        self.settings.playback_volume = max(0.0, min(1.0, value))
        if self._volume_apply_source is not None:
            GLib.source_remove(self._volume_apply_source)
        self._volume_apply_source = GLib.timeout_add(
            150, self._apply_volume_now)

    def _apply_volume_now(self):
        self._volume_apply_source = None
        threading.Thread(target=self._set_stream_volume, daemon=True).start()
        return GLib.SOURCE_REMOVE

    def _set_stream_volume(self):
        """Set the pacat stream's volume via pactl (worker thread). Only
        Phosphor's own playback moves — never the whole system."""
        percent = int(round(self.settings.playback_volume * 100))
        try:
            listing = subprocess.run(["pactl", "list", "sink-inputs"],
                                     capture_output=True, text=True,
                                     timeout=5).stdout
        except (OSError, subprocess.TimeoutExpired):
            return
        current_index = None
        for line in listing.splitlines():
            line = line.strip()
            if line.startswith("Sink Input #"):
                current_index = line.split("#", 1)[1]
            elif (current_index is not None
                    and line == 'application.name = "Phosphor"'):
                try:
                    subprocess.run(["pactl", "set-sink-input-volume",
                                    current_index, f"{percent}%"],
                                   capture_output=True, timeout=5)
                except (OSError, subprocess.TimeoutExpired):
                    pass
                return

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
        # attach a precomputed scope stream (or the live pipe) before the
        # decoder starts, so the audible pipe rate is already right
        window.prepare_scope_feed(path)
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
        self.refresh_playlist_panel()
        if self.settings.playback_volume < 0.995:
            GLib.timeout_add(300, self._apply_volume_now)
        window._start_render_loop()

    def play_dropped(self, paths):
        """Files dropped on the scope: one plays with its folder, several
        become the playlist themselves."""
        audio = [path for path in paths
                 if path.lower().endswith(AUDIO_FILE_EXTENSIONS)
                 and os.path.exists(path)]
        if not audio:
            self.window.status_label.set_text("drop audio files to play them")
            return
        if len(audio) == 1:
            self.play_file(audio[0])
        else:
            self.playlist = audio
            self.play_file(audio[0], rebuild_playlist=False)

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
            self.current_metadata = None
            self.position_box.hide()
            self.window.detach_precomputed()
            self.mpris.notify_track_changed()
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
        if self.settings.shuffle and len(self.playlist) > 1:
            index = random.choice([i for i in range(len(self.playlist))
                                   if i != self.playlist_index])
        else:
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
        self.mpris.notify_playback_state()

    def handle_track_finished(self, finished_file):
        """Natural end of a track: repeat/shuffle decide what comes next."""
        repeat = self.settings.repeat_mode
        if repeat == "one" and 0 <= self.playlist_index < len(self.playlist):
            self.play_file(self.playlist[self.playlist_index],
                           rebuild_playlist=False)
        elif (self.settings.shuffle and len(self.playlist) > 1) or (
                self.playlist and self.playlist_index < len(self.playlist) - 1):
            self.play_next_track()
        elif repeat == "all" and self.playlist:
            self.play_file(self.playlist[0], rebuild_playlist=False)
        else:
            self.set_file_loaded(False)
            self.window.status_label.set_text(f"finished: {finished_file}")

    # ---------------------------------------------------------- track position

    def _setup_position_slider(self, path):
        """One ffprobe pass per track (worker thread, so starts never stall
        the render loop): duration feeds the seek slider, tags feed the
        now-playing overlay and MPRIS."""
        self.track_duration_seconds = None
        self.current_metadata = None
        self.position_box.hide()

        def probe_worker():
            metadata = probe_metadata(path)
            GLib.idle_add(self._apply_probed_metadata, path, metadata)
        threading.Thread(target=probe_worker, daemon=True).start()

    def _apply_probed_metadata(self, path, metadata):
        if path != self.playing_path:
            return False                # track changed while probing
        self.current_metadata = metadata
        title = metadata.get("title") or self.playing_file
        subtitle = " — ".join(part for part in (metadata.get("artist"),
                                                metadata.get("album")) if part)
        nod = ARTIST_NODS.get((metadata.get("artist") or "").strip().lower())
        if nod:
            subtitle = f"{subtitle}  ·  {nod}" if subtitle else nod
        self.window.flash_now_playing(title, subtitle or None)
        self.mpris.notify_track_changed()
        self._show_position_slider(path, metadata.get("duration"))
        return False

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

    def seek_to(self, seconds):
        """Programmatic seek (MPRIS SetPosition/Seek): slider follows."""
        if self.playing_path is None:
            return
        if self.track_duration_seconds is not None:
            seconds = max(0.0, min(seconds, self.track_duration_seconds))
            self.position_scale.set_value(seconds)
            self._refresh_time_label(seconds)
        self._perform_seek(seconds)

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
        self.window.on_playback_restarted(target_seconds)
        if self.settings.playback_volume < 0.995:
            GLib.timeout_add(300, self._apply_volume_now)
        if was_paused:
            self.capture_stream.set_playback_paused(True)
        else:
            self.window.quiet_frame_count = 0
            self.window._start_render_loop()
        return GLib.SOURCE_REMOVE
