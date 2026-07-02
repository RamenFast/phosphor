# SPDX-License-Identifier: GPL-3.0-or-later
"""MPRIS for Phosphor, both directions.

MprisService publishes Phosphor's file playback as a standard media player
(org.mpris.MediaPlayer2.phosphor): media keys, the sound applet, and
playerctl all control the scope's built-in player.

MprisWatcher listens to every *other* player on the bus, so when the scope
is tracing a browser or Spotify, Phosphor still knows what song just came
on — the now-playing overlay works for music Phosphor isn't playing itself.
"""

import os

from gi.repository import Gio, GLib

BUS_NAME = "org.mpris.MediaPlayer2.phosphor"
OBJECT_PATH = "/org/mpris/MediaPlayer2"
PLAYER_INTERFACE = "org.mpris.MediaPlayer2.Player"
ROOT_INTERFACE = "org.mpris.MediaPlayer2"

INTROSPECTION_XML = """
<node>
  <interface name="org.mpris.MediaPlayer2">
    <method name="Raise"/>
    <method name="Quit"/>
    <property name="CanQuit" type="b" access="read"/>
    <property name="CanRaise" type="b" access="read"/>
    <property name="HasTrackList" type="b" access="read"/>
    <property name="Identity" type="s" access="read"/>
    <property name="DesktopEntry" type="s" access="read"/>
    <property name="SupportedUriSchemes" type="as" access="read"/>
    <property name="SupportedMimeTypes" type="as" access="read"/>
  </interface>
  <interface name="org.mpris.MediaPlayer2.Player">
    <method name="Next"/>
    <method name="Previous"/>
    <method name="Pause"/>
    <method name="PlayPause"/>
    <method name="Stop"/>
    <method name="Play"/>
    <method name="Seek"><arg name="Offset" type="x" direction="in"/></method>
    <method name="SetPosition">
      <arg name="TrackId" type="o" direction="in"/>
      <arg name="Position" type="x" direction="in"/>
    </method>
    <method name="OpenUri"><arg name="Uri" type="s" direction="in"/></method>
    <signal name="Seeked"><arg name="Position" type="x"/></signal>
    <property name="PlaybackStatus" type="s" access="read"/>
    <property name="Rate" type="d" access="readwrite"/>
    <property name="Metadata" type="a{sv}" access="read"/>
    <property name="Volume" type="d" access="readwrite"/>
    <property name="Position" type="x" access="read"/>
    <property name="MinimumRate" type="d" access="read"/>
    <property name="MaximumRate" type="d" access="read"/>
    <property name="CanGoNext" type="b" access="read"/>
    <property name="CanGoPrevious" type="b" access="read"/>
    <property name="CanPlay" type="b" access="read"/>
    <property name="CanPause" type="b" access="read"/>
    <property name="CanSeek" type="b" access="read"/>
    <property name="CanControl" type="b" access="read"/>
  </interface>
</node>
"""


class MprisService:
    """Phosphor's own playback on the bus. All calls arrive on the GTK main
    loop, so handlers touch the player directly."""

    def __init__(self, player):
        self.player = player
        self._connection = None
        self._registrations = []
        node = Gio.DBusNodeInfo.new_for_xml(INTROSPECTION_XML)
        self._interfaces = {info.name: info for info in node.interfaces}
        Gio.bus_own_name(Gio.BusType.SESSION, BUS_NAME,
                         Gio.BusNameOwnerFlags.NONE,
                         self._on_bus_acquired, None, None)

    def _on_bus_acquired(self, connection, _name):
        self._connection = connection
        for interface in self._interfaces.values():
            registration = connection.register_object(
                OBJECT_PATH, interface, self._on_method_call,
                self._on_get_property, None)
            self._registrations.append(registration)

    # -- outgoing notifications ------------------------------------------------

    def emit_changes(self, *property_names):
        """PropertiesChanged for the Player interface."""
        if self._connection is None:
            return
        changed = {name: self._player_property(name)
                   for name in property_names}
        self._connection.emit_signal(
            None, OBJECT_PATH, "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            GLib.Variant("(sa{sv}as)", (PLAYER_INTERFACE, changed, [])))

    def notify_track_changed(self):
        self.emit_changes("Metadata", "PlaybackStatus", "CanGoNext",
                          "CanGoPrevious", "CanSeek")

    def notify_playback_state(self):
        self.emit_changes("PlaybackStatus")

    # -- incoming --------------------------------------------------------------

    def _on_method_call(self, _connection, _sender, _path, interface, method,
                        parameters, invocation):
        player = self.player
        if interface == ROOT_INTERFACE:
            if method == "Raise":
                player.window.present()
            invocation.return_value(None)   # Quit intentionally ignored
            return
        if method in ("Play", "Pause", "PlayPause"):
            paused = player.capture_stream.playback_paused
            if (method == "PlayPause" or (method == "Play") == paused):
                player.toggle_play_pause()
        elif method == "Next":
            player.play_next_track()
        elif method == "Previous":
            player.play_previous_track()
        elif method == "Stop":
            if player.playing_file is not None:
                player.window.capture_toggle.set_active(False)
        elif method == "Seek":
            offset_microseconds, = parameters.unpack()
            position = (player.capture_stream.playback_position_seconds
                        + offset_microseconds / 1e6)
            player.seek_to(max(0.0, position))
        elif method == "SetPosition":
            _track_id, position_microseconds = parameters.unpack()
            player.seek_to(max(0.0, position_microseconds / 1e6))
        elif method == "OpenUri":
            uri, = parameters.unpack()
            filename = Gio.File.new_for_uri(uri).get_path()
            if filename and os.path.exists(filename):
                player.play_file(filename)
        invocation.return_value(None)

    def _on_get_property(self, _connection, _sender, _path, interface, name):
        if interface == ROOT_INTERFACE:
            return self._root_property(name)
        return self._player_property(name)

    def _root_property(self, name):
        values = {
            "CanQuit": GLib.Variant("b", False),
            "CanRaise": GLib.Variant("b", True),
            "HasTrackList": GLib.Variant("b", False),
            "Identity": GLib.Variant("s", "Phosphor"),
            "DesktopEntry": GLib.Variant("s", "phosphor"),
            "SupportedUriSchemes": GLib.Variant("as", ["file"]),
            "SupportedMimeTypes": GLib.Variant("as", ["audio/mpeg",
                                                      "audio/flac",
                                                      "audio/ogg",
                                                      "audio/x-wav"]),
        }
        return values.get(name)

    def _player_property(self, name):
        player = self.player
        loaded = player.playing_file is not None
        if name == "PlaybackStatus":
            if not loaded:
                status = "Stopped"
            else:
                status = ("Paused" if player.capture_stream.playback_paused
                          else "Playing")
            return GLib.Variant("s", status)
        if name == "Metadata":
            return GLib.Variant("a{sv}", self._metadata())
        if name == "Position":
            position = (player.capture_stream.playback_position_seconds
                        if loaded else 0.0)
            return GLib.Variant("x", int(position * 1e6))
        if name in ("Rate", "MinimumRate", "MaximumRate"):
            return GLib.Variant("d", 1.0)
        if name == "Volume":
            return GLib.Variant("d", float(player.settings.playback_volume))
        if name in ("CanGoNext", "CanGoPrevious"):
            return GLib.Variant("b", len(player.playlist) > 1)
        if name in ("CanPlay", "CanPause", "CanControl"):
            return GLib.Variant("b", True)
        if name == "CanSeek":
            return GLib.Variant("b", loaded
                                and player.track_duration_seconds is not None)
        return None

    def _metadata(self):
        player = self.player
        if player.playing_path is None:
            return {"mpris:trackid":
                    GLib.Variant("o", "/org/mpris/MediaPlayer2/TrackList/NoTrack")}
        tags = player.current_metadata or {}
        metadata = {
            "mpris:trackid": GLib.Variant(
                "o", f"/io/github/ben/Phosphor/track/{abs(hash(player.playing_path))}"),
            "xesam:url": GLib.Variant(
                "s", Gio.File.new_for_path(player.playing_path).get_uri()),
            "xesam:title": GLib.Variant(
                "s", tags.get("title") or player.playing_file),
        }
        if player.track_duration_seconds:
            metadata["mpris:length"] = GLib.Variant(
                "x", int(player.track_duration_seconds * 1e6))
        if tags.get("artist"):
            metadata["xesam:artist"] = GLib.Variant("as", [tags["artist"]])
        if tags.get("album"):
            metadata["xesam:album"] = GLib.Variant("s", tags["album"])
        return metadata


class MprisWatcher:
    """Track changes from every other player on the bus.

    on_now_playing(title, artist, album, source_identity) fires whenever a
    playing player's metadata changes — debounced against repeats, and never
    for Phosphor's own service.
    """

    def __init__(self, on_now_playing):
        self.on_now_playing = on_now_playing
        self._last_seen = {}
        self._identities = {}
        self._connection = Gio.bus_get_sync(Gio.BusType.SESSION, None)
        self._own_name = self._connection.get_unique_name()
        self._connection.signal_subscribe(
            None, "org.freedesktop.DBus.Properties", "PropertiesChanged",
            OBJECT_PATH, PLAYER_INTERFACE, Gio.DBusSignalFlags.NONE,
            self._on_properties_changed)

    def _on_properties_changed(self, _connection, sender, _path, _interface,
                               _signal, parameters):
        if sender == self._own_name:
            return
        _iface, changed, _invalidated = parameters.unpack()
        metadata = changed.get("Metadata")
        status = changed.get("PlaybackStatus")
        if metadata is None or status == "Stopped":
            return
        title = metadata.get("xesam:title")
        if not title:
            return
        artists = metadata.get("xesam:artist") or []
        artist = ", ".join(artists) if isinstance(artists, list) else str(artists)
        album = metadata.get("xesam:album") or ""
        key = (title, artist, album)
        if self._last_seen.get(sender) == key:
            return
        self._last_seen[sender] = key
        self._resolve_identity(sender, lambda identity: self.on_now_playing(
            title, artist, album, identity))

    def _resolve_identity(self, sender, callback):
        """The player's human name ("Spotify", "Firefox"), cached."""
        if sender in self._identities:
            callback(self._identities[sender])
            return

        def finish(connection, result):
            identity = ""
            try:
                reply = connection.call_finish(result)
                identity = reply.unpack()[0]
            except GLib.Error:
                pass
            self._identities[sender] = identity
            callback(identity)

        self._connection.call(
            sender, OBJECT_PATH, "org.freedesktop.DBus.Properties", "Get",
            GLib.Variant("(ss)", (ROOT_INTERFACE, "Identity")),
            GLib.VariantType("(v)"), Gio.DBusCallFlags.NONE, 1000, None,
            finish)
