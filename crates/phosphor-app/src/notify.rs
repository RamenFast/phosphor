// SPDX-License-Identifier: GPL-3.0-or-later
//! Desktop notifications (org.freedesktop.Notifications over zbus —
//! no extra crate): the album-art popup on track changes (Ben: "we
//! need the album artwork to popup with the name, just a systemwide
//! notification"). One notification slot per source — the id is
//! replaced, so track skips never stack toasts.
//!
//! Art handling: `file://` urls and raw paths are used in place;
//! `http(s)` art (Spotify serves https) is fetched with the system
//! `curl` into the runtime dir (subprocess law — pactl/ffmpeg/date
//! already live that way; no new deps). Callers run off the chrome
//! thread or at track-change rate, so the blocking calls are fine.

use std::path::PathBuf;

/// Send/replace a track notification with a local art file (own
/// tracks: the embedded cover written to the runtime dir; external
/// players: the client thread's cached fetch). Returns the id to pass
/// back next time (0 on failure — a failed toast never bothers the
/// beam).
pub fn notify_track_with_file(title: &str, artist: &str, source: &str,
                              art_path: Option<&std::path::Path>,
                              replaces: u32) -> u32 {
    let body = if artist.is_empty() {
        format!("via {source}")
    } else {
        format!("{artist} · via {source}")
    };
    send(title, &body, art_path, replaces)
}

fn send(summary: &str, body: &str, art: Option<&std::path::Path>,
        replaces: u32) -> u32 {
    let Ok(connection) = zbus::blocking::Connection::session() else {
        return 0;
    };
    let mut hints: std::collections::HashMap<
        &str, zbus::zvariant::Value> = std::collections::HashMap::new();
    if let Some(path) = art {
        hints.insert("image-path",
                     zbus::zvariant::Value::from(
                         path.to_string_lossy().to_string()));
    }
    hints.insert("desktop-entry",
                 zbus::zvariant::Value::from("phosphor"));
    let reply = connection.call_method(
        Some("org.freedesktop.Notifications"),
        "/org/freedesktop/Notifications",
        Some("org.freedesktop.Notifications"),
        "Notify",
        &("phosphor",                 // app name
          replaces,                   // replaces id (no stacking)
          "phosphor-scope",           // app icon (hicolor, the deb's)
          summary,
          body,
          Vec::<&str>::new(),         // actions
          hints,
          5000i32));                  // 5 s
    reply.ok()
        .and_then(|message| message.body().deserialize::<u32>().ok())
        .unwrap_or(0)
}

/// Resolve an MPRIS artUrl to a local file: file:// and plain paths
/// pass through; http(s) is fetched via `curl` into the runtime dir.
/// Blocking (up to 4 s on a cold fetch) — call from worker threads.
pub(crate) fn cache_art(url: &str) -> Option<PathBuf> {
    if let Some(path) = url.strip_prefix("file://") {
        let path = PathBuf::from(path);
        return path.exists().then_some(path);
    }
    if url.starts_with('/') {
        let path = PathBuf::from(url);
        return path.exists().then_some(path);
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("phosphor");
        std::fs::create_dir_all(&dir).ok()?;
        // one slot keyed by a cheap hash — repeated tracks hit cache
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in url.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        let path = dir.join(format!("art-{hash:016x}"));
        if path.exists() {
            return Some(path);
        }
        let status = std::process::Command::new("curl")
            .args(["-sL", "-m", "4", "-o"])
            .arg(&path)
            .arg(url)
            .status();
        if status.map(|s| s.success()).unwrap_or(false)
            && path.metadata().map(|m| m.len() > 0).unwrap_or(false)
        {
            return Some(path);
        }
        let _ = std::fs::remove_file(&path);
    }
    None
}

/// Where the built-in player's embedded cover art lands for the
/// notification's image-path hint.
pub fn own_art_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("phosphor")
        .join("now-playing-art")
}
