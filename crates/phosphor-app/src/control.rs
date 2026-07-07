// SPDX-License-Identifier: GPL-3.0-or-later
//! The control socket — a Unix-domain NDJSON server that lets the CLI
//! (`phosphor status`, `phosphor ctl …`, `phosphor tap`) read the live
//! shell's state and drive it while it runs, without stealing focus or
//! waking the window when it doesn't have to.
//!
//! Structurally this mirrors `mpris.rs`: `spawn()` binds the socket on a
//! dedicated thread and hands back a handle whose `shared` half the
//! shell writes into every tick and whose `requests` half the shell
//! drains next to the MPRIS commands. The wake problem (an idle winit
//! loop drains nothing) is solved by an `EventLoopProxy<()>`: after
//! enqueueing a request the server pokes the proxy so the shell wakes,
//! services the request, and replies. `status` and `tap` never wake the
//! shell — they answer straight from `shared`.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::mpris::MprisCommand;

/// A verb the shell must apply. Transport verbs reuse the MPRIS command
/// path (one code path for every external controller); the rest map to
/// the same UiActions the chrome pushes.
pub(crate) enum ControlVerb {
    Transport(MprisCommand),
    Mode(String),
    Theme(String),
    UiStyle(String),
    Capture(bool),
    Target(String),
    Snapshot,
    Clip,
    /// focus + deiconify the window (single-instance forward, agents)
    Raise,
    /// load a file into the player (single-instance file forward)
    Open(String),
    Quit,
}

/// A request handed to the shell with a one-shot reply channel back to
/// the connection thread that's blocking on it.
pub(crate) struct ControlRequest {
    pub verb: ControlVerb,
    pub reply: mpsc::Sender<Value>,
}

/// A frame observation streamed to `tap` subscribers. Pre-serialized by
/// the shell so the server threads just write the line.
pub(crate) enum TapEvent {
    Frame(Value),
}

/// The live status the shell refreshes once per tick. Field names are
/// the wire contract (Worker B's `phosphor schema` publishes them).
#[derive(Clone, Default, serde::Serialize)]
pub(crate) struct StatusSnapshot {
    pub running: bool,
    pub pid: u32,
    pub version: String,
    pub mode: String,
    pub theme: String,
    pub ui_style: String,
    pub capture: CaptureStatus,
    /// what actually feeds the beam right now — the single truth the
    /// combo renders and agents should trust over capture.target_id
    pub source: SourceStatus,
    pub player: PlayerStatus,
    pub volume: f32,
    pub gain: GainStatus,
    pub kit: KitStatus,
    pub window: WindowStatus,
    pub vacuum: VacuumStatus,
    pub quiet: QuietStatus,
    pub fps: f64,
    /// present while the Custom-theme color cycle is animating (v4.1);
    /// `current` is the interpolated beam color this tick — agents can
    /// watch it move without screenshotting
    pub beam_cycle: Option<BeamCycleStatus>,
}

#[derive(Clone, Default, serde::Serialize)]
pub(crate) struct BeamCycleStatus {
    pub colors: i64,
    pub seconds: f64,
    pub current: [f32; 3],
}

#[derive(Clone, Default, serde::Serialize)]
pub(crate) struct CaptureStatus {
    pub on: bool,
    pub target_id: Option<String>,
}

#[derive(Clone, Default, serde::Serialize)]
pub(crate) struct SourceStatus {
    /// "capture" | "player" | "mix" | "silent"
    pub kind: String,
    /// capture: the combo id · player: the file path · mix: "a+b+c"
    pub detail: Option<String>,
}

#[derive(Clone, Default, serde::Serialize)]
pub(crate) struct PlayerStatus {
    pub track: Option<String>,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub position_seconds: f64,
    pub duration_seconds: Option<f64>,
    pub paused: bool,
}

#[derive(Clone, Default, serde::Serialize)]
pub(crate) struct GainStatus {
    pub setting: f32,
    pub effective: f32,
    pub auto: bool,
}

#[derive(Clone, Default, serde::Serialize)]
pub(crate) struct KitStatus {
    pub enabled: bool,
    pub path: Option<String>,
}

#[derive(Clone, Default, serde::Serialize)]
pub(crate) struct WindowStatus {
    pub mini: bool,
    pub fullscreen: bool,
}

#[derive(Clone, Default, serde::Serialize)]
pub(crate) struct VacuumStatus {
    pub file: bool,
    pub app: Option<String>,
}

#[derive(Clone, Default, serde::Serialize)]
pub(crate) struct QuietStatus {
    pub render_active: bool,
}

/// Shared state the shell writes and the server threads read.
pub(crate) struct ControlShared {
    pub status: Mutex<StatusSnapshot>,
    pub taps: Mutex<Vec<mpsc::Sender<TapEvent>>>,
}

/// The shell-side handle. Dropping it removes our own socket file (a
/// clean-shutdown courtesy; kill -9 is covered by the next launch's
/// sweep).
pub(crate) struct ControlHandle {
    pub shared: Arc<ControlShared>,
    pub requests: mpsc::Receiver<ControlRequest>,
    socket_path: PathBuf,
}

impl Drop for ControlHandle {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

/// One parsed wire request.
enum Op {
    Status,
    Tap,
    Ctl(ControlVerb),
}

/// Where to bind, and whether an existing `ctl.sock` must be removed
/// first (it was stale). Pure so the launch logic is unit-testable.
struct SocketChoice {
    path: PathBuf,
    remove_existing: bool,
}

fn socket_target(dir: &Path, ctl_exists: bool, ctl_alive: bool,
                 pid: u32) -> SocketChoice {
    if ctl_exists && ctl_alive {
        // another instance owns ctl.sock — take a per-pid slot
        SocketChoice {
            path: dir.join(format!("ctl-{pid}.sock")),
            remove_existing: false,
        }
    } else {
        // free, or a stale file we must clear before binding
        SocketChoice {
            path: dir.join("ctl.sock"),
            remove_existing: ctl_exists,
        }
    }
}

/// A socket is "alive" iff something is accepting connections on it.
fn is_socket_alive(path: &Path) -> bool {
    UnixStream::connect(path).is_ok()
}

/// `$XDG_RUNTIME_DIR/phosphor`, else `/tmp/phosphor-$UID`.
fn control_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(base) if !base.is_empty() =>
            PathBuf::from(base).join("phosphor"),
        _ => {
            // /proc/self is owned by our uid — no libc, no subprocess
            let uid = std::fs::metadata("/proc/self")
                .map(|m| m.uid())
                .unwrap_or(1000);
            PathBuf::from(format!("/tmp/phosphor-{uid}"))
        }
    }
}

/// Parse one request line. `Ok(op)` needs handling; `Err(value)` is a
/// ready-to-write error reply the shell never has to see (malformed
/// JSON, unknown op/verb).
fn parse_request(line: &str) -> Result<Op, Value> {
    let value: Value = serde_json::from_str(line.trim()).map_err(|e| {
        json!({
            "status": "error",
            "error": format!("malformed json: {e}"),
            "fix": "send one NDJSON object per line, e.g. {\"op\":\"status\"}",
        })
    })?;
    let op = value.get("op").and_then(Value::as_str).unwrap_or("");
    match op {
        "status" => Ok(Op::Status),
        "tap" => Ok(Op::Tap),
        "ctl" => {
            let verb = value.get("verb").and_then(Value::as_str)
                .unwrap_or("");
            let args = value.get("args").cloned().unwrap_or(json!({}));
            parse_verb(verb, &args).map(Op::Ctl)
        }
        other => Err(json!({
            "status": "error",
            "error": format!("unknown op '{other}'"),
            "fix": "use op status | ctl | tap; run: phosphor schema",
        })),
    }
}

fn parse_verb(verb: &str, args: &Value) -> Result<ControlVerb, Value> {
    let str_arg = |key: &str| -> Option<String> {
        args.get(key).and_then(Value::as_str).map(str::to_string)
    };
    let f64_arg = |key: &str| -> Option<f64> {
        args.get(key).and_then(Value::as_f64)
    };
    let bool_arg = |key: &str| -> Option<bool> {
        args.get(key).and_then(Value::as_bool)
    };
    let verb = match verb {
        "play" => match str_arg("path") {
            Some(path) => ControlVerb::Transport(MprisCommand::OpenUri(path)),
            None => ControlVerb::Transport(MprisCommand::Play),
        },
        "pause" => ControlVerb::Transport(MprisCommand::Pause),
        "toggle" => ControlVerb::Transport(MprisCommand::PlayPause),
        "stop" => ControlVerb::Transport(MprisCommand::Stop),
        "next" => ControlVerb::Transport(MprisCommand::Next),
        "previous" => ControlVerb::Transport(MprisCommand::Previous),
        "seek" => {
            let seconds = f64_arg("seconds").ok_or_else(|| json!({
                "status": "error",
                "error": "seek needs a numeric 'seconds' (relative)",
                "fix": "phosphor ctl seek --seconds -5",
            }))?;
            ControlVerb::Transport(
                MprisCommand::SeekRelative((seconds * 1e6) as i64))
        }
        "volume" => {
            let value = f64_arg("value").ok_or_else(|| json!({
                "status": "error",
                "error": "volume needs a numeric 'value' in 0..1",
                "fix": "phosphor ctl volume --value 0.8",
            }))?;
            ControlVerb::Transport(
                MprisCommand::SetVolume(value.clamp(0.0, 1.0)))
        }
        "mode" => ControlVerb::Mode(str_arg("name").ok_or_else(|| json!({
            "status": "error",
            "error": "mode needs a 'name'",
            "fix": "phosphor ctl mode --name xy",
        }))?),
        "theme" => ControlVerb::Theme(str_arg("name").ok_or_else(|| json!({
            "status": "error",
            "error": "theme needs a 'name'",
            "fix": "phosphor ctl theme --name blossom",
        }))?),
        "ui" => ControlVerb::UiStyle(str_arg("name").ok_or_else(|| json!({
            "status": "error",
            "error": "ui needs a 'name'",
            "fix": "phosphor ctl ui --name dark",
        }))?),
        "capture" => ControlVerb::Capture(bool_arg("on").ok_or_else(|| {
            json!({
                "status": "error",
                "error": "capture needs a boolean 'on'",
                "fix": "phosphor ctl capture --on true",
            })
        })?),
        "target" => ControlVerb::Target(str_arg("id").ok_or_else(|| json!({
            "status": "error",
            "error": "target needs an 'id'",
            "fix": "phosphor ctl target --id monitor:0 (see: phosphor status)",
        }))?),
        "snapshot" => ControlVerb::Snapshot,
        "clip" => ControlVerb::Clip,
        "raise" => ControlVerb::Raise,
        "open" => ControlVerb::Open(str_arg("path").ok_or_else(|| json!({
            "status": "error",
            "error": "open needs a 'path'",
            "fix": "phosphor ctl open --path /music/song.flac",
        }))?),
        "quit" => ControlVerb::Quit,
        other => return Err(json!({
            "status": "error",
            "error": format!("unknown verb '{other}'"),
            "fix": "run: phosphor schema",
        })),
    };
    Ok(verb)
}

/// Geometry of a frame's segments in trace-pixel coords: bounding box
/// [minx,miny,maxx,maxy], centroid [x,y] (mean of segment midpoints),
/// and a decimated midpoint polyline (≤ `max_poly` points). All None /
/// empty when there are no segments. Pure — unit-tested.
struct FrameGeometry {
    bbox: Option<[f32; 4]>,
    centroid: Option<[f32; 2]>,
    polyline: Vec<[f32; 2]>,
}

fn frame_geometry(segments: &[[f32; 5]], max_poly: usize) -> FrameGeometry {
    if segments.is_empty() {
        return FrameGeometry {
            bbox: None, centroid: None, polyline: Vec::new(),
        };
    }
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut sum_x = 0.0f32;
    let mut sum_y = 0.0f32;
    for seg in segments {
        let [x0, y0, x1, y1, _] = *seg;
        min_x = min_x.min(x0).min(x1);
        min_y = min_y.min(y0).min(y1);
        max_x = max_x.max(x0).max(x1);
        max_y = max_y.max(y0).max(y1);
        sum_x += (x0 + x1) * 0.5;
        sum_y += (y0 + y1) * 0.5;
    }
    let n = segments.len() as f32;
    let bbox = [min_x, min_y, max_x, max_y];
    let centroid = [sum_x / n, sum_y / n];

    let max_poly = max_poly.max(1);
    let stride = segments.len().div_ceil(max_poly).max(1);
    let polyline: Vec<[f32; 2]> = segments
        .iter()
        .step_by(stride)
        .map(|s| [(s[0] + s[2]) * 0.5, (s[1] + s[3]) * 0.5])
        .collect();

    FrameGeometry {
        bbox: Some(bbox), centroid: Some(centroid), polyline,
    }
}

/// Build the `frame` tap value the shell broadcasts. Coords rounded to
/// one decimal to keep the NDJSON lines small.
pub(crate) fn build_frame_event(
    segments: &[[f32; 5]], mode: &str, peak: f32,
    trace_w: f32, trace_h: f32, ts_ms: u128) -> Value {
    let round1 = |v: f32| ((v * 10.0).round() / 10.0) as f64;
    let geometry = frame_geometry(segments, 64);
    json!({
        "event": "frame",
        "ts_ms": ts_ms,
        "mode": mode,
        "segments": segments.len(),
        "bbox": geometry.bbox.map(|b| b.map(round1)),
        "centroid": geometry.centroid.map(|c| c.map(round1)),
        "peak": (peak as f64 * 10000.0).round() / 10000.0,
        "polyline": geometry.polyline.iter()
            .map(|p| [round1(p[0]), round1(p[1])])
            .collect::<Vec<_>>(),
        "trace_size": [trace_w as i64, trace_h as i64],
    })
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Bind the control socket and start the accept loop. `None` (with a
/// note on stderr) when the socket can't be bound — headless / locked
/// runs keep working without a control surface.
pub(crate) fn spawn(proxy: winit::event_loop::EventLoopProxy<()>)
    -> Option<ControlHandle> {
    let dir = control_dir();
    if let Err(error) = std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
    {
        eprintln!("phosphor: control dir {}: {error}", dir.display());
        return None;
    }

    // Sweep stale per-pid slots (kill -9 leaves them; atexit doesn't
    // survive it — same law as the vacuum sweep).
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("ctl-") && name.ends_with(".sock")
                && !is_socket_alive(&path)
            {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    let ctl_path = dir.join("ctl.sock");
    let ctl_exists = ctl_path.exists();
    let ctl_alive = ctl_exists && is_socket_alive(&ctl_path);
    let choice = socket_target(&dir, ctl_exists, ctl_alive,
                               std::process::id());
    if choice.remove_existing {
        let _ = std::fs::remove_file(dir.join("ctl.sock"));
    }

    let listener = match UnixListener::bind(&choice.path) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("phosphor: control socket {}: {error}",
                      choice.path.display());
            return None;
        }
    };

    let shared = Arc::new(ControlShared {
        status: Mutex::new(StatusSnapshot::default()),
        taps: Mutex::new(Vec::new()),
    });
    let (request_sender, request_receiver) = mpsc::channel();

    let shared_for_accept = shared.clone();
    std::thread::Builder::new()
        .name("phosphor-control".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let shared = shared_for_accept.clone();
                let requests = request_sender.clone();
                let proxy = proxy.clone();
                std::thread::spawn(move || {
                    handle_connection(stream, shared, requests, proxy);
                });
            }
        })
        .ok()?;

    Some(ControlHandle {
        shared,
        requests: request_receiver,
        socket_path: choice.path,
    })
}

fn handle_connection(
    stream: UnixStream,
    shared: Arc<ControlShared>,
    requests: mpsc::Sender<ControlRequest>,
    proxy: winit::event_loop::EventLoopProxy<()>,
) {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(clone) => clone,
        Err(_) => return,
    });
    let mut writer = stream;
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
        return;
    }

    let op = match parse_request(&line) {
        Ok(op) => op,
        Err(reply) => {
            write_line(&mut writer, &reply);
            return;
        }
    };

    match op {
        Op::Status => {
            let snapshot = shared.status.lock().unwrap().clone();
            let value = serde_json::to_value(&snapshot)
                .unwrap_or_else(|_| json!({"running": true}));
            write_line(&mut writer, &value);
        }
        Op::Ctl(verb) => {
            let (reply_sender, reply_receiver) = mpsc::channel();
            let request = ControlRequest { verb, reply: reply_sender };
            if requests.send(request).is_err() {
                write_line(&mut writer, &json!({
                    "status": "error",
                    "error": "shell is not accepting commands",
                    "fix": "check that the phosphor window is still running",
                }));
                return;
            }
            // wake the idle loop so it drains the request this tick
            let _ = proxy.send_event(());
            let reply = match reply_receiver
                .recv_timeout(Duration::from_secs(10))
            {
                Ok(value) => value,
                Err(_) => json!({
                    "status": "error",
                    "error": "timed out waiting for the shell (10 s)",
                    "fix": "the window may be busy or hung; retry",
                }),
            };
            write_line(&mut writer, &reply);
        }
        Op::Tap => {
            let (tap_sender, tap_receiver) = mpsc::channel();
            shared.taps.lock().unwrap().push(tap_sender);
            loop {
                match tap_receiver.recv_timeout(Duration::from_secs(1)) {
                    Ok(TapEvent::Frame(value)) => {
                        if !write_line(&mut writer, &value) {
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        // heartbeat: prove the connection is alive when
                        // the picture is frozen (quiet law)
                        let tick = json!({
                            "event": "tick", "ts_ms": now_ms(),
                        });
                        if !write_line(&mut writer, &tick) {
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        }
    }
}

/// Write one NDJSON line + flush. Returns false when the peer is gone
/// (the shell prunes tap senders when the matching send fails).
fn write_line(writer: &mut UnixStream, value: &Value) -> bool {
    let mut line = value.to_string();
    line.push('\n');
    writer.write_all(line.as_bytes()).is_ok() && writer.flush().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verb_of(line: &str) -> ControlVerb {
        match parse_request(line) {
            Ok(Op::Ctl(verb)) => verb,
            _ => panic!("expected a ctl verb from {line}"),
        }
    }

    #[test]
    fn parses_status_and_tap() {
        assert!(matches!(parse_request(r#"{"op":"status"}"#), Ok(Op::Status)));
        assert!(matches!(parse_request(r#"{"op":"tap"}"#), Ok(Op::Tap)));
    }

    #[test]
    fn parses_every_transport_verb() {
        assert!(matches!(verb_of(r#"{"op":"ctl","verb":"pause"}"#),
            ControlVerb::Transport(MprisCommand::Pause)));
        assert!(matches!(verb_of(r#"{"op":"ctl","verb":"toggle"}"#),
            ControlVerb::Transport(MprisCommand::PlayPause)));
        assert!(matches!(verb_of(r#"{"op":"ctl","verb":"stop"}"#),
            ControlVerb::Transport(MprisCommand::Stop)));
        assert!(matches!(verb_of(r#"{"op":"ctl","verb":"next"}"#),
            ControlVerb::Transport(MprisCommand::Next)));
        assert!(matches!(verb_of(r#"{"op":"ctl","verb":"previous"}"#),
            ControlVerb::Transport(MprisCommand::Previous)));
        // play with no path resumes; with a path opens it
        assert!(matches!(verb_of(r#"{"op":"ctl","verb":"play"}"#),
            ControlVerb::Transport(MprisCommand::Play)));
        match verb_of(r#"{"op":"ctl","verb":"play","args":{"path":"/a.flac"}}"#) {
            ControlVerb::Transport(MprisCommand::OpenUri(p)) =>
                assert_eq!(p, "/a.flac"),
            _ => panic!("play path should OpenUri"),
        }
    }

    #[test]
    fn parses_seek_and_volume_args() {
        match verb_of(r#"{"op":"ctl","verb":"seek","args":{"seconds":-5.0}}"#) {
            ControlVerb::Transport(MprisCommand::SeekRelative(us)) =>
                assert_eq!(us, -5_000_000),
            _ => panic!("seek"),
        }
        match verb_of(r#"{"op":"ctl","verb":"volume","args":{"value":1.7}}"#) {
            ControlVerb::Transport(MprisCommand::SetVolume(v)) =>
                assert!((v - 1.0).abs() < 1e-9, "clamped to 1.0"),
            _ => panic!("volume"),
        }
    }

    #[test]
    fn parses_named_and_flag_verbs() {
        match verb_of(r#"{"op":"ctl","verb":"mode","args":{"name":"ring"}}"#) {
            ControlVerb::Mode(n) => assert_eq!(n, "ring"),
            _ => panic!("mode"),
        }
        match verb_of(r#"{"op":"ctl","verb":"theme","args":{"name":"dark"}}"#) {
            ControlVerb::Theme(n) => assert_eq!(n, "dark"),
            _ => panic!("theme"),
        }
        match verb_of(r#"{"op":"ctl","verb":"ui","args":{"name":"light"}}"#) {
            ControlVerb::UiStyle(n) => assert_eq!(n, "light"),
            _ => panic!("ui"),
        }
        match verb_of(r#"{"op":"ctl","verb":"capture","args":{"on":false}}"#) {
            ControlVerb::Capture(on) => assert!(!on),
            _ => panic!("capture"),
        }
        match verb_of(r#"{"op":"ctl","verb":"target","args":{"id":"monitor:0"}}"#) {
            ControlVerb::Target(id) => assert_eq!(id, "monitor:0"),
            _ => panic!("target"),
        }
        assert!(matches!(verb_of(r#"{"op":"ctl","verb":"snapshot"}"#),
            ControlVerb::Snapshot));
        assert!(matches!(verb_of(r#"{"op":"ctl","verb":"clip"}"#),
            ControlVerb::Clip));
        assert!(matches!(verb_of(r#"{"op":"ctl","verb":"quit"}"#),
            ControlVerb::Quit));
        assert!(matches!(verb_of(r#"{"op":"ctl","verb":"raise"}"#),
            ControlVerb::Raise));
        match verb_of(r#"{"op":"ctl","verb":"open","args":{"path":"/b.flac"}}"#) {
            ControlVerb::Open(p) => assert_eq!(p, "/b.flac"),
            _ => panic!("open should carry its path"),
        }
        // open without a path is a fix-bearing error
        let err = parse_request(r#"{"op":"ctl","verb":"open"}"#).err().unwrap();
        assert_eq!(err["status"], "error");
        assert!(err["fix"].as_str().unwrap().contains("--path"));
    }

    #[test]
    fn unknown_verb_and_op_carry_a_fix() {
        let err = parse_request(r#"{"op":"ctl","verb":"boom"}"#)
            .err().expect("unknown verb errors");
        assert_eq!(err["status"], "error");
        assert!(err["error"].as_str().unwrap().contains("boom"));
        assert!(err["fix"].as_str().unwrap().contains("schema"));

        let err = parse_request(r#"{"op":"wat"}"#).err().unwrap();
        assert!(err["error"].as_str().unwrap().contains("wat"));
        assert!(!err["fix"].as_str().unwrap().is_empty());
    }

    #[test]
    fn missing_required_args_error_with_fix() {
        for line in [
            r#"{"op":"ctl","verb":"seek"}"#,
            r#"{"op":"ctl","verb":"volume"}"#,
            r#"{"op":"ctl","verb":"mode"}"#,
            r#"{"op":"ctl","verb":"capture"}"#,
        ] {
            let err = parse_request(line).err()
                .unwrap_or_else(|| panic!("{line} should error"));
            assert_eq!(err["status"], "error");
            assert!(!err["fix"].as_str().unwrap().is_empty());
        }
    }

    #[test]
    fn malformed_json_is_a_clean_error() {
        let err = parse_request("not json at all").err().unwrap();
        assert_eq!(err["status"], "error");
        assert!(err["error"].as_str().unwrap().contains("malformed"));
        assert!(!err["fix"].as_str().unwrap().is_empty());
    }

    #[test]
    fn socket_target_rules() {
        let dir = Path::new("/run/phosphor");
        // free slot → ctl.sock, nothing to remove
        let c = socket_target(dir, false, false, 42);
        assert_eq!(c.path, dir.join("ctl.sock"));
        assert!(!c.remove_existing);
        // stale file → ctl.sock, remove it first
        let c = socket_target(dir, true, false, 42);
        assert_eq!(c.path, dir.join("ctl.sock"));
        assert!(c.remove_existing);
        // live owner → per-pid slot, leave theirs alone
        let c = socket_target(dir, true, true, 42);
        assert_eq!(c.path, dir.join("ctl-42.sock"));
        assert!(!c.remove_existing);
    }

    #[test]
    fn geometry_of_a_known_segment_set() {
        // two segments forming an L: (0,0)->(10,0) and (10,0)->(10,10)
        let segments = [
            [0.0f32, 0.0, 10.0, 0.0, 1.0],
            [10.0f32, 0.0, 10.0, 10.0, 1.0],
        ];
        let g = frame_geometry(&segments, 64);
        assert_eq!(g.bbox, Some([0.0, 0.0, 10.0, 10.0]));
        // midpoints (5,0) and (10,5) → mean (7.5, 2.5)
        assert_eq!(g.centroid, Some([7.5, 2.5]));
        assert_eq!(g.polyline, vec![[5.0, 0.0], [10.0, 5.0]]);
    }

    #[test]
    fn geometry_empty_is_all_none() {
        let g = frame_geometry(&[], 64);
        assert!(g.bbox.is_none());
        assert!(g.centroid.is_none());
        assert!(g.polyline.is_empty());
    }

    #[test]
    fn polyline_is_capped_and_decimated() {
        let segments: Vec<[f32; 5]> = (0..500)
            .map(|i| [i as f32, 0.0, i as f32, 1.0, 1.0])
            .collect();
        let poly = frame_geometry(&segments, 64).polyline;
        assert!(poly.len() <= 64, "got {}", poly.len());
        assert!(!poly.is_empty());
    }
}
