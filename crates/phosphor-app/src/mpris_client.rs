// SPDX-License-Identifier: GPL-3.0-or-later
//! The MPRIS *client* half — watch the other players on the session
//! bus and drive the one the beam is scoping (Ben: "if music is
//! coming from spotify or another music app, [the controls should]
//! control that"). v3's phosphor_mpris only WATCHED for now-playing
//! titles; v4 watches and drives.
//!
//! Same idiom as the server (mpris.rs): one dedicated thread, zbus
//! blocking, a command mailbox. The thread polls the bus roster +
//! player properties (~1 Hz — metadata churn is slow) and executes
//! transport commands the moment they arrive (100 ms mailbox tick).
//! Track changes on external players are noticed HERE and, when
//! enabled, become desktop notifications with the album art — the
//! shell never blocks on any of it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExternalPlayer {
    pub bus_name: String,
    pub identity: String,
    pub desktop_entry: String,
    /// "Playing" | "Paused" | "Stopped"
    pub status: String,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub art_url: Option<String>,
    /// artUrl resolved to a LOCAL file (fetched on this thread — the
    /// overlay and the notification both read it; the chrome thread
    /// never waits on a download)
    pub art_local: Option<std::path::PathBuf>,
    pub can_control: bool,
}

pub enum ClientCommand {
    PlayPause(String),
    Play(String),
    Pause(String),
    Next(String),
    Previous(String),
}

pub struct MprisClientHandle {
    pub players: Arc<Mutex<Vec<ExternalPlayer>>>,
    pub commands: mpsc::Sender<ClientCommand>,
    /// desktop notifications on external track changes (settings-fed)
    pub notify_enabled: Arc<AtomicBool>,
}

/// Case-insensitive match between a capture-target app key ("Spotify",
/// "firefox"…) and a player's identity/desktop entry. Pure — tested.
pub fn player_matches_app(player: &ExternalPlayer, app_key: &str) -> bool {
    let key = app_key.to_lowercase();
    let key = key.trim_end_matches(|c: char| c == '+' || c.is_ascii_digit());
    if key.is_empty() {
        return false;
    }
    let identity = player.identity.to_lowercase();
    let desktop = player.desktop_entry.to_lowercase();
    identity.contains(key) || desktop.contains(key)
        || key.contains(&identity) && !identity.is_empty()
}

/// Pick the player the beam is listening to: an app key match first,
/// else (whole-output capture) whoever is actually Playing.
pub fn linked_player(players: &[ExternalPlayer], app_key: Option<&str>)
    -> Option<ExternalPlayer>
{
    match app_key {
        Some(key) => players.iter()
            .find(|p| player_matches_app(p, key))
            .cloned(),
        None => players.iter()
            .find(|p| p.status == "Playing")
            .cloned(),
    }
}

pub fn spawn(notify_enabled: bool) -> Option<MprisClientHandle> {
    let players: Arc<Mutex<Vec<ExternalPlayer>>> = Arc::default();
    let (command_sender, command_receiver) = mpsc::channel();
    let notify_flag = Arc::new(AtomicBool::new(notify_enabled));

    let players_for_thread = players.clone();
    let notify_for_thread = notify_flag.clone();
    std::thread::Builder::new()
        .name("phosphor-mpris-client".into())
        .spawn(move || {
            let Ok(connection) = zbus::blocking::Connection::session()
            else {
                eprintln!("phosphor: mpris client: no session bus");
                return;
            };
            client_loop(&connection, &players_for_thread,
                        &command_receiver, &notify_for_thread);
        })
        .ok()?;

    Some(MprisClientHandle {
        players,
        commands: command_sender,
        notify_enabled: notify_flag,
    })
}

fn client_loop(connection: &zbus::blocking::Connection,
               players: &Arc<Mutex<Vec<ExternalPlayer>>>,
               commands: &mpsc::Receiver<ClientCommand>,
               notify_enabled: &Arc<AtomicBool>) {
    let mut ticks_since_refresh = u32::MAX / 2; // refresh immediately
    // per-bus last-seen track (title|artist) for change detection
    let mut last_track: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    let mut notification_id: u32 = 0;
    loop {
        match commands.recv_timeout(Duration::from_millis(100)) {
            Ok(command) => {
                let (bus, method) = match &command {
                    ClientCommand::PlayPause(bus) => (bus, "PlayPause"),
                    ClientCommand::Play(bus) => (bus, "Play"),
                    ClientCommand::Pause(bus) => (bus, "Pause"),
                    ClientCommand::Next(bus) => (bus, "Next"),
                    ClientCommand::Previous(bus) => (bus, "Previous"),
                };
                let _ = connection.call_method(
                    Some(bus.as_str()),
                    "/org/mpris/MediaPlayer2",
                    Some("org.mpris.MediaPlayer2.Player"),
                    method,
                    &());
                // the roster will look stale for up to a second —
                // refresh right away so the play glyph flips with it
                ticks_since_refresh = u32::MAX / 2;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        ticks_since_refresh = ticks_since_refresh.saturating_add(1);
        if ticks_since_refresh < 10 {
            continue; // ~1 Hz roster refresh
        }
        ticks_since_refresh = 0;

        let mut roster = refresh_players(connection);
        // resolve art HERE (blocking curl is fine on this thread):
        // file:// instantly, https via the cache — once per url
        for player in &mut roster {
            player.art_local = player.art_url.as_deref()
                .and_then(crate::notify::cache_art);
        }
        // external track-change → desktop notification (with art)
        if notify_enabled.load(Ordering::Relaxed) {
            for player in &roster {
                let Some(title) = &player.title else { continue };
                let signature = format!(
                    "{title}|{}", player.artist.as_deref().unwrap_or(""));
                let previous = last_track
                    .insert(player.bus_name.clone(), signature.clone());
                // only NOTIFY on a change while audible — never on
                // first sight (startup would toast every idle player)
                if previous.is_some()
                    && previous.as_deref() != Some(signature.as_str())
                    && player.status == "Playing"
                {
                    notification_id =
                        crate::notify::notify_track_with_file(
                            title,
                            player.artist.as_deref().unwrap_or(""),
                            &player.identity,
                            player.art_local.as_deref(),
                            notification_id);
                }
            }
        } else {
            // keep signatures current so re-enabling doesn't replay
            for player in &roster {
                if let Some(title) = &player.title {
                    last_track.insert(
                        player.bus_name.clone(),
                        format!("{title}|{}",
                                player.artist.as_deref().unwrap_or("")));
                }
            }
        }
        *players.lock().unwrap() = roster;
    }
}

fn refresh_players(connection: &zbus::blocking::Connection)
    -> Vec<ExternalPlayer>
{
    let Ok(proxy) = zbus::blocking::fdo::DBusProxy::new(connection)
    else {
        return Vec::new();
    };
    let Ok(names) = proxy.list_names() else { return Vec::new() };
    let mut roster = Vec::new();
    for name in names {
        let name = name.to_string();
        if !name.starts_with("org.mpris.MediaPlayer2.")
            || name == "org.mpris.MediaPlayer2.phosphor"
        {
            continue; // not a player, or ourselves
        }
        if let Some(player) = read_player(connection, &name) {
            roster.push(player);
        }
    }
    roster
}

fn read_player(connection: &zbus::blocking::Connection, bus: &str)
    -> Option<ExternalPlayer>
{
    let root = zbus::blocking::Proxy::new(
        connection, bus.to_string(), "/org/mpris/MediaPlayer2",
        "org.mpris.MediaPlayer2").ok()?;
    let player = zbus::blocking::Proxy::new(
        connection, bus.to_string(), "/org/mpris/MediaPlayer2",
        "org.mpris.MediaPlayer2.Player").ok()?;

    let identity: String = root.get_property("Identity").unwrap_or_default();
    let desktop_entry: String =
        root.get_property("DesktopEntry").unwrap_or_default();
    let status: String =
        player.get_property("PlaybackStatus").unwrap_or_default();
    let can_control: bool =
        player.get_property("CanControl").unwrap_or(true);
    let metadata: std::collections::HashMap<
        String, zbus::zvariant::OwnedValue> =
        player.get_property("Metadata").unwrap_or_default();

    let string_of = |key: &str| -> Option<String> {
        metadata.get(key).and_then(|value| {
            if let Ok(text) = <&str>::try_from(value) {
                return Some(text.to_string());
            }
            // xesam:artist is (usually) a list of strings
            <Vec<String>>::try_from(value.try_clone().ok()?)
                .ok()?
                .into_iter()
                .next()
        })
    };

    Some(ExternalPlayer {
        bus_name: bus.to_string(),
        identity,
        desktop_entry,
        status,
        title: string_of("xesam:title"),
        artist: string_of("xesam:artist"),
        album: string_of("xesam:album"),
        art_url: string_of("mpris:artUrl"),
        art_local: None, // resolved by the caller (client_loop)
        can_control,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(identity: &str, desktop: &str, status: &str)
        -> ExternalPlayer
    {
        ExternalPlayer {
            bus_name: format!("org.mpris.MediaPlayer2.{}",
                              identity.to_lowercase()),
            identity: identity.into(),
            desktop_entry: desktop.into(),
            status: status.into(),
            ..Default::default()
        }
    }

    #[test]
    fn app_key_matches_identity_and_desktop_entry() {
        let spotify = player("Spotify", "spotify", "Playing");
        assert!(player_matches_app(&spotify, "Spotify"));
        assert!(player_matches_app(&spotify, "spotify"));
        // the dedup suffix from the target scheme ("spotify+2") still
        // finds its player
        assert!(player_matches_app(&spotify, "spotify+2"));
        assert!(!player_matches_app(&spotify, "firefox"));
        let firefox = player("Mozilla Firefox", "firefox", "Paused");
        assert!(player_matches_app(&firefox, "firefox"));
    }

    #[test]
    fn linked_player_prefers_key_match_else_playing() {
        let roster = vec![
            player("Mozilla Firefox", "firefox", "Paused"),
            player("Spotify", "spotify", "Playing"),
        ];
        assert_eq!(
            linked_player(&roster, Some("firefox")).unwrap().identity,
            "Mozilla Firefox");
        // whole-output capture: whoever is audible
        assert_eq!(linked_player(&roster, None).unwrap().identity,
                   "Spotify");
        assert!(linked_player(&roster, Some("vlc")).is_none());
    }
}
