// SPDX-License-Identifier: GPL-3.0-or-later
//! Chrome pass v — MPRIS via zbus (PLAYER-SPEC §8: bus
//! org.mpris.MediaPlayer2.phosphor, path /org/mpris/MediaPlayer2,
//! Identity "Phosphor", DesktopEntry "phosphor", uri schemes [file]).
//!
//! Deliberate v4 fixes to v3's documented quirks: trackids are STABLE
//! (path-hash object paths, not a salted process hash), `Seeked`
//! actually emits, `Stop` actually stops, and Volume is really
//! writable. Media keys arrive as Player method calls from Cinnamon.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Default)]
pub struct MprisTrack {
    pub path: Option<std::path::PathBuf>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_micros: Option<i64>,
}

#[derive(Default)]
pub struct MprisShared {
    pub track: Mutex<MprisTrack>,
    /// "Playing" | "Paused" | "Stopped"
    pub status: Mutex<&'static str>,
    pub position_micros: AtomicI64,
    pub can_step: std::sync::atomic::AtomicBool,
    pub volume: Mutex<f64>,
}

pub enum MprisCommand {
    Next,
    Previous,
    PlayPause,
    Play,
    Pause,
    Stop,
    SeekRelative(i64),
    SetPosition(i64),
    OpenUri(String),
    Raise,
    SetVolume(f64),
}

pub enum MprisNotify {
    TrackChanged,
    StatusChanged,
    Seeked(i64),
}

/// Stable trackid: an object path from the path bytes (FNV-1a) — the
/// same file yields the same id across relaunches (v3 bug, fixed).
fn track_object_path(track: &MprisTrack) -> String {
    match &track.path {
        Some(path) => {
            let mut hash: u64 = 0xcbf29ce484222325;
            for byte in path.to_string_lossy().as_bytes() {
                hash ^= *byte as u64;
                hash = hash.wrapping_mul(0x100000001b3);
            }
            format!("/org/phosphor/track/{hash:016x}")
        }
        None => "/org/mpris/MediaPlayer2/TrackList/NoTrack".to_string(),
    }
}

struct RootWithSender {
    commands: mpsc::Sender<MprisCommand>,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl RootWithSender {
    fn raise(&self) {
        let _ = self.commands.send(MprisCommand::Raise);
    }
    fn quit(&self) {}

    #[zbus(property)]
    fn can_quit(&self) -> bool { false }
    #[zbus(property)]
    fn can_raise(&self) -> bool { true }
    #[zbus(property)]
    fn has_track_list(&self) -> bool { false }
    #[zbus(property)]
    fn identity(&self) -> &str { "Phosphor" }
    #[zbus(property)]
    fn desktop_entry(&self) -> &str { "phosphor" }
    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        vec!["file".into()]
    }
    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        vec!["audio/mpeg".into(), "audio/flac".into(),
             "audio/ogg".into(), "audio/x-wav".into()]
    }
}

struct PlayerInterface {
    shared: Arc<MprisShared>,
    commands: mpsc::Sender<MprisCommand>,
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl PlayerInterface {
    fn next(&self) {
        let _ = self.commands.send(MprisCommand::Next);
    }
    fn previous(&self) {
        let _ = self.commands.send(MprisCommand::Previous);
    }
    fn pause(&self) {
        let _ = self.commands.send(MprisCommand::Pause);
    }
    fn play_pause(&self) {
        let _ = self.commands.send(MprisCommand::PlayPause);
    }
    fn stop(&self) {
        let _ = self.commands.send(MprisCommand::Stop);
    }
    fn play(&self) {
        let _ = self.commands.send(MprisCommand::Play);
    }
    fn seek(&self, offset: i64) {
        let _ = self.commands.send(MprisCommand::SeekRelative(offset));
    }
    fn set_position(&self, _track_id: zbus::zvariant::ObjectPath<'_>,
                    position: i64) {
        let _ = self.commands.send(MprisCommand::SetPosition(position));
    }
    fn open_uri(&self, uri: String) {
        let _ = self.commands.send(MprisCommand::OpenUri(uri));
    }

    #[zbus(signal)]
    async fn seeked(context: &zbus::object_server::SignalEmitter<'_>,
                    position: i64) -> zbus::Result<()>;

    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.shared.status.lock().unwrap().to_string()
    }
    #[zbus(property)]
    fn rate(&self) -> f64 { 1.0 }
    #[zbus(property)]
    fn set_rate(&self, _rate: f64) {}
    #[zbus(property)]
    fn metadata(&self)
        -> std::collections::HashMap<String, zbus::zvariant::OwnedValue>
    {
        let track = self.shared.track.lock().unwrap().clone();
        let mut map = std::collections::HashMap::new();
        let object_path = track_object_path(&track);
        if let Ok(value) = zbus::zvariant::ObjectPath::try_from(
            object_path.as_str())
        {
            map.insert("mpris:trackid".to_string(),
                       zbus::zvariant::OwnedValue::try_from(
                           zbus::zvariant::Value::ObjectPath(value))
                       .unwrap());
        }
        if let Some(duration) = track.duration_micros {
            map.insert("mpris:length".to_string(),
                       zbus::zvariant::OwnedValue::from(duration));
        }
        if let Some(title) = &track.title {
            map.insert("xesam:title".to_string(),
                       zbus::zvariant::OwnedValue::try_from(
                           zbus::zvariant::Value::from(title.clone()))
                       .unwrap());
        }
        if let Some(artist) = &track.artist {
            map.insert("xesam:artist".to_string(),
                       zbus::zvariant::OwnedValue::try_from(
                           zbus::zvariant::Value::from(
                               vec![artist.clone()]))
                       .unwrap());
        }
        if let Some(album) = &track.album {
            map.insert("xesam:album".to_string(),
                       zbus::zvariant::OwnedValue::try_from(
                           zbus::zvariant::Value::from(album.clone()))
                       .unwrap());
        }
        if let Some(path) = &track.path {
            map.insert("xesam:url".to_string(),
                       zbus::zvariant::OwnedValue::try_from(
                           zbus::zvariant::Value::from(
                               format!("file://{}", path.display())))
                       .unwrap());
        }
        map
    }
    #[zbus(property)]
    fn volume(&self) -> f64 {
        *self.shared.volume.lock().unwrap()
    }
    #[zbus(property)]
    fn set_volume(&self, volume: f64) {
        let _ = self.commands.send(
            MprisCommand::SetVolume(volume.clamp(0.0, 1.0)));
    }
    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        self.shared.position_micros.load(Ordering::Relaxed)
    }
    #[zbus(property)]
    fn minimum_rate(&self) -> f64 { 1.0 }
    #[zbus(property)]
    fn maximum_rate(&self) -> f64 { 1.0 }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.shared.can_step.load(Ordering::Relaxed)
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.shared.can_step.load(Ordering::Relaxed)
    }
    #[zbus(property)]
    fn can_play(&self) -> bool { true }
    #[zbus(property)]
    fn can_pause(&self) -> bool { true }
    #[zbus(property)]
    fn can_seek(&self) -> bool { true }
    #[zbus(property)]
    fn can_control(&self) -> bool { true }
}

pub struct MprisHandle {
    pub shared: Arc<MprisShared>,
    pub commands: mpsc::Receiver<MprisCommand>,
    pub notify: mpsc::Sender<MprisNotify>,
}

/// Own the bus on a dedicated thread; returns None when the session
/// bus is unavailable (headless test runs keep working).
pub fn spawn() -> Option<MprisHandle> {
    let shared = Arc::new(MprisShared {
        status: Mutex::new("Stopped"),
        ..Default::default()
    });
    let (command_sender, command_receiver) = mpsc::channel();
    let (notify_sender, notify_receiver) = mpsc::channel::<MprisNotify>();

    let shared_for_thread = shared.clone();
    let build = move || -> zbus::Result<zbus::blocking::Connection> {
        let connection = zbus::blocking::connection::Builder::session()?
            .name("org.mpris.MediaPlayer2.phosphor")?
            .serve_at("/org/mpris/MediaPlayer2",
                      RootWithSender { commands: command_sender.clone() })?
            .serve_at("/org/mpris/MediaPlayer2", PlayerInterface {
                shared: shared_for_thread.clone(),
                commands: command_sender.clone(),
            })?
            .build()?;
        Ok(connection)
    };

    let (ready_sender, ready_receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("phosphor-mpris".into())
        .spawn(move || {
            let connection = match build() {
                Ok(connection) => {
                    let _ = ready_sender.send(true);
                    connection
                }
                Err(error) => {
                    eprintln!("phosphor: mpris unavailable: {error}");
                    let _ = ready_sender.send(false);
                    return;
                }
            };
            // pump notifies → D-Bus signals/prop-changed
            while let Ok(notification) = notify_receiver.recv() {
                let server = connection.object_server();
                let Ok(interface) = server
                    .interface::<_, PlayerInterface>(
                        "/org/mpris/MediaPlayer2")
                else { continue };
                let context = interface.signal_emitter();
                match notification {
                    MprisNotify::TrackChanged => {
                        let guard = interface.get();
                        let _ = zbus::block_on(async {
                            let _ = guard.metadata_changed(context).await;
                            let _ = guard
                                .playback_status_changed(context).await;
                            let _ = guard
                                .can_go_next_changed(context).await;
                            zbus::Result::Ok(())
                        });
                    }
                    MprisNotify::StatusChanged => {
                        let guard = interface.get();
                        let _ = zbus::block_on(
                            guard.playback_status_changed(context));
                    }
                    MprisNotify::Seeked(position) => {
                        let _ = zbus::block_on(
                            PlayerInterface::seeked(context, position));
                    }
                }
            }
        })
        .ok()?;

    if ready_receiver.recv().ok()? {
        Some(MprisHandle {
            shared,
            commands: command_receiver,
            notify: notify_sender,
        })
    } else {
        None
    }
}
