// SPDX-License-Identifier: GPL-3.0-or-later
//! Agent-facing CLI clients: `probe` (one-shot status), `ctl` (one
//! reply per verb), `tap` (NDJSON stream), and `schema` (the machine
//! map). Station convention (NexusFormStationWork/CONVENTION.md):
//! every verb speaks `--json`, auto-switches to JSON the moment stdout
//! is a pipe, one-shots emit EXACTLY ONE envelope object, streams emit
//! NDJSON whose FIRST line is a `hello` event, and every error carries
//! both an `error` and a `fix`. Exit codes: 0 ok, 2 unavailable,
//! 3 bad arguments, 4 runtime failure.
//!
//! This is the CLIENT half. The GUI process is the server (shell.rs +
//! control.rs); the wire protocol on `$XDG_RUNTIME_DIR/phosphor/
//! ctl.sock` is frozen — see the ops handled below.

use std::io::{BufRead, BufReader, IsTerminal, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use crate::protocol::{CtlRequest, end_event};

const TOOL: &str = "phosphor";

fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// RFC3339 timestamp via the `date -Is` subprocess idiom (mirrors
/// exports::timestamp), with an epoch-seconds fallback so the envelope
/// always has a `ts` even if `date` is missing.
fn now() -> String {
    let output = std::process::Command::new("date")
        .arg("-Is")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if output.is_empty() {
        format!(
            "{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        )
    } else {
        output
    }
}

/// JSON when `--json` is passed OR stdout is not a TTY (isatty switch).
fn wants_json(args: &[String]) -> bool {
    args.iter().any(|a| a == "--json") || !std::io::stdout().is_terminal()
}

// ---------------------------------------------------------------------------
// Envelope builders (unit-tested)
// ---------------------------------------------------------------------------

fn base_envelope(status: &str) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("status".into(), json!(status));
    map.insert("tool".into(), json!(TOOL));
    map.insert("version".into(), json!(version()));
    map.insert("ts".into(), json!(now()));
    map
}

fn ok_envelope() -> Map<String, Value> {
    base_envelope("ok")
}

/// An error envelope. An error without a `fix` is a bug (convention),
/// so both are required by the signature.
fn error_envelope(error: &str, fix: &str) -> Map<String, Value> {
    let mut map = base_envelope("error");
    map.insert("error".into(), json!(error));
    map.insert("fix".into(), json!(fix));
    map
}

/// Emit exactly one JSON object on stdout (pure — no diagnostics here).
fn emit_json(map: &Map<String, Value>) {
    if let Ok(text) = serde_json::to_string(&Value::Object(map.clone())) {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{text}");
    }
}

// ---------------------------------------------------------------------------
// Socket discovery + request helpers
// ---------------------------------------------------------------------------

/// `id -u` (fallback dir only); XDG_RUNTIME_DIR is the primary path.
fn uid() -> String {
    std::process::Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "0".into())
}

fn socket_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        dirs.push(PathBuf::from(runtime).join("phosphor"));
    }
    dirs.push(PathBuf::from(format!("/tmp/phosphor-{}", uid())));
    dirs
}

/// An explicit socket override: `--socket <path>` on the CLI, else the
/// `PHOSPHOR_CTL_SOCKET` environment variable. `None` = auto-discover.
fn socket_override(args: &[String]) -> Option<PathBuf> {
    if let Some(index) = args.iter().position(|a| a == "--socket")
        && let Some(path) = args.get(index + 1)
    {
        return Some(PathBuf::from(path));
    }
    std::env::var_os("PHOSPHOR_CTL_SOCKET").map(PathBuf::from)
}

/// Connect order: `ctl.sock` first, then any `ctl-*.sock` in the dir,
/// newest mtime first — across every candidate dir.
fn socket_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for dir in socket_dirs() {
        candidates.push(dir.join("ctl.sock"));
        let mut globbed: Vec<(PathBuf, SystemTime)> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let is_ctl = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("ctl-") && n.ends_with(".sock"))
                    .unwrap_or(false);
                if is_ctl {
                    let mtime = entry
                        .metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(UNIX_EPOCH);
                    globbed.push((path, mtime));
                }
            }
        }
        globbed.sort_by_key(|entry| std::cmp::Reverse(entry.1));
        candidates.extend(globbed.into_iter().map(|(p, _)| p));
    }
    candidates
}

/// First socket that accepts a connection, or an error (used by callers
/// to decide between "not running" (ok/2) and a runtime failure). An
/// override (`--socket` / `PHOSPHOR_CTL_SOCKET`) pins ONE socket — with
/// several instances up, auto-discovery is otherwise implicit.
fn connect_to(override_path: Option<PathBuf>) -> Result<UnixStream, String> {
    if let Some(path) = override_path {
        return UnixStream::connect(&path).map_err(|e| format!("socket {}: {e}", path.display()));
    }
    for path in socket_candidates() {
        if let Ok(stream) = UnixStream::connect(&path) {
            return Ok(stream);
        }
    }
    Err("no control socket".into())
}

fn connect() -> Result<UnixStream, String> {
    connect_to(None)
}

fn send_line(stream: &mut UnixStream, value: &Value) -> Result<(), String> {
    let mut line = serde_json::to_string(value).map_err(|e| e.to_string())?;
    line.push('\n');
    stream.write_all(line.as_bytes()).map_err(|e| e.to_string())
}

/// Send one request line, read exactly one reply line, parse it.
fn request(stream: UnixStream, message: &Value, timeout: Duration) -> Result<Value, String> {
    let mut stream = stream;
    let _ = stream.set_read_timeout(Some(timeout));
    send_line(&mut stream, message)?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let read = reader.read_line(&mut line).map_err(|e| e.to_string())?;
    if read == 0 {
        return Err("phosphor closed the connection without replying".into());
    }
    serde_json::from_str(line.trim()).map_err(|e| format!("unreadable reply from phosphor: {e}"))
}

// ---------------------------------------------------------------------------
// single-instance forward (called from main before spawning a window)
// ---------------------------------------------------------------------------

/// A plain GUI launch while an instance is live becomes a focus (+ an
/// `open` when a file was passed). `None` = no live instance, launch
/// normally. `Some(code)` = handled here, exit with it.
pub fn forward_to_running_instance(play_path: Option<&str>) -> Option<i32> {
    let stream = connect().ok()?;
    let raise = request(
        stream,
        &json!({"op": "ctl", "verb": "raise"}),
        Duration::from_secs(3),
    );
    match raise {
        Ok(reply) if reply.get("status").and_then(Value::as_str) == Some("ok") => {
            if let Some(path) = play_path {
                // canonicalize client-side: the running instance has a
                // different cwd, so a relative path would miss
                let absolute = std::fs::canonicalize(path)
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| path.to_string());
                if let Ok(stream) = connect() {
                    let _ = request(
                        stream,
                        &json!({"op": "ctl", "verb": "open",
                                "args": {"path": absolute}}),
                        Duration::from_secs(5),
                    );
                }
            }
            eprintln!("phosphor: already running — focused the existing window");
            Some(0)
        }
        _ => {
            // A socket answered but raise didn't: hung or ancient
            // build. Spawning a second window on top would be worse —
            // say what to do instead.
            eprintln!(
                "phosphor: an instance is already running but did not \
                 respond\n  fix: focus it manually, or `phosphor ctl \
                 quit` and relaunch"
            );
            Some(2)
        }
    }
}

// ---------------------------------------------------------------------------
// probe
// ---------------------------------------------------------------------------

pub fn run_probe(args: &[String]) -> i32 {
    let json = wants_json(args);

    // Offline probe is designed but not built — say so honestly.
    if args.iter().any(|a| a == "--at") {
        let envelope = error_envelope(
            "offline probe (--at) lands with the studio wave",
            "use live probe, or phosphor render for offline frames",
        );
        if json {
            emit_json(&envelope);
        } else {
            eprintln!(
                "phosphor probe: offline probe (--at) lands with the \
                 studio wave\n  fix: use live probe, or phosphor render \
                 for offline frames"
            );
        }
        return 2;
    }

    let stream = match connect_to(socket_override(args)) {
        Ok(stream) => stream,
        Err(_) => {
            // A status tool saying "not running" is a valid answer, not
            // an error: ok envelope, running:false, exit 0.
            let mut envelope = ok_envelope();
            envelope.insert("running".into(), json!(false));
            if json {
                emit_json(&envelope);
            } else {
                println!("phosphor: not running (no control socket)");
            }
            return 0;
        }
    };

    match request(stream, &json!({"op": "status"}), Duration::from_secs(5)) {
        Ok(snapshot) => {
            let mut envelope = ok_envelope();
            if let Some(fields) = snapshot.as_object() {
                for (key, value) in fields {
                    envelope.insert(key.clone(), value.clone());
                }
            }
            if json {
                emit_json(&envelope);
            } else {
                print_probe_human(&snapshot);
            }
            0
        }
        Err(error) => {
            let envelope = error_envelope(
                &format!("could not read status: {error}"),
                "check that the phosphor GUI is healthy; restart it if the \
                 socket is stale",
            );
            if json {
                emit_json(&envelope);
            } else {
                eprintln!("phosphor probe: {error}");
            }
            4
        }
    }
}

fn print_probe_human(snapshot: &Value) {
    let get_str = |key: &str| snapshot.get(key).and_then(Value::as_str);
    let mode = get_str("mode").unwrap_or("?");
    let theme = get_str("theme").unwrap_or("?");
    let ui = get_str("ui_style").unwrap_or("?");
    let fps = snapshot.get("fps").and_then(Value::as_f64).unwrap_or(0.0);

    let capture_on = snapshot
        .get("capture")
        .and_then(|c| c.get("on"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    println!("phosphor: running");
    println!("  mode     {mode}");
    println!("  theme    {theme}");
    println!("  ui       {ui}");
    println!("  capture  {}", if capture_on { "on" } else { "off" });

    if let Some(player) = snapshot.get("player") {
        let title = player.get("title").and_then(Value::as_str).unwrap_or("");
        let artist = player.get("artist").and_then(Value::as_str).unwrap_or("");
        let position = player
            .get("position_seconds")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        let duration = player
            .get("duration_seconds")
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        if !title.is_empty() || !artist.is_empty() {
            let who = if artist.is_empty() {
                title.to_string()
            } else {
                format!("{artist} — {title}")
            };
            println!(
                "  track    {who}  [{}/{}]",
                clock(position),
                clock(duration)
            );
        }
    }
    println!("  fps      {fps:.0}");
}

fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

// ---------------------------------------------------------------------------
// ctl
// ---------------------------------------------------------------------------

const CTL_USAGE: &str = "\
usage: phosphor ctl <verb> [value] [--json] [--socket <path>]
  play [path]        pause | toggle | stop | next | previous
  seek <±seconds>    volume <0..1>
  mode <name>        theme <name>        ui <style>
  capture on|off     target <id>
  open <path>        raise
  snapshot           clip                quit";

/// Map a positional CLI verb + args to the frozen wire message
/// `{"op":"ctl","verb":..,"args":{..}}` via the SHARED typed request
/// (protocol.rs) — the server parses the same type back, so what this
/// builds the server accepts by construction (BUGLOG #16). `Err` is a
/// bad-arguments (exit 3) message.
fn build_ctl_message(verb: &str, rest: &[&str]) -> Result<Value, String> {
    CtlRequest::from_cli(verb, rest).map(|request| request.to_wire())
}

pub fn run_ctl(args: &[String]) -> i32 {
    let json = wants_json(args);
    let socket = socket_override(args);
    let mut skip_next = false;
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| {
            if skip_next {
                skip_next = false;
                return false;
            }
            if a.as_str() == "--socket" {
                skip_next = true;
                return false;
            }
            a.as_str() != "--json"
        })
        .map(String::as_str)
        .collect();

    let verb = match positional.first() {
        Some(verb) => *verb,
        None => {
            eprintln!("{CTL_USAGE}");
            if json {
                emit_json(&error_envelope(
                    "no verb given",
                    "pick a verb, e.g. `phosphor ctl pause`",
                ));
            }
            return 3;
        }
    };

    let message = match build_ctl_message(verb, &positional[1..]) {
        Ok(message) => message,
        Err(problem) => {
            eprintln!("phosphor ctl: {problem}\n{CTL_USAGE}");
            if json {
                emit_json(&error_envelope(&problem, "see `phosphor ctl` usage above"));
            }
            return 3;
        }
    };

    let stream = match connect_to(socket) {
        Ok(stream) => stream,
        Err(_) => {
            let envelope = error_envelope("phosphor is not running", "start the GUI: phosphor");
            if json {
                emit_json(&envelope);
            } else {
                eprintln!("phosphor ctl: phosphor is not running\n  fix: start the GUI: phosphor");
            }
            return 2;
        }
    };

    match request(stream, &message, Duration::from_secs(30)) {
        Ok(reply) => {
            let status = reply.get("status").and_then(Value::as_str).unwrap_or("");
            if status == "ok" {
                let mut envelope = ok_envelope();
                envelope.insert("verb".into(), json!(verb));
                if let Some(result) = reply.get("result") {
                    envelope.insert("result".into(), result.clone());
                }
                if json {
                    emit_json(&envelope);
                } else {
                    print_ctl_confirmation(verb, &positional[1..], &reply);
                }
                0
            } else {
                let error = reply
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("phosphor refused the command")
                    .to_string();
                let fix = reply
                    .get("fix")
                    .and_then(Value::as_str)
                    .unwrap_or("check the verb and value against `phosphor schema`")
                    .to_string();
                let envelope = error_envelope(&error, &fix);
                if json {
                    emit_json(&envelope);
                } else {
                    eprintln!("phosphor ctl: {error}\n  fix: {fix}");
                }
                4
            }
        }
        Err(error) => {
            let envelope = error_envelope(
                &error,
                "check that the phosphor GUI is healthy; restart it if the \
                 socket is stale",
            );
            if json {
                emit_json(&envelope);
            } else {
                eprintln!("phosphor ctl: {error}");
            }
            4
        }
    }
}

fn print_ctl_confirmation(verb: &str, rest: &[&str], reply: &Value) {
    // snapshot/clip land a path in result; echo it. Value-carrying verbs
    // echo the value; the rest just confirm.
    if let Some(path) = reply
        .get("result")
        .and_then(|r| r.get("path"))
        .and_then(Value::as_str)
    {
        println!("{verb} → {path}");
        return;
    }
    match verb {
        "seek" | "volume" | "mode" | "theme" | "ui" | "capture" | "target" | "play" | "open" => {
            if let Some(value) = rest.first() {
                println!("{verb} → {value}");
            } else {
                println!("{verb} ✓");
            }
        }
        _ => println!("{verb} ✓"),
    }
}

// ---------------------------------------------------------------------------
// tap (NDJSON stream)
// ---------------------------------------------------------------------------

pub fn run_tap(args: &[String]) -> i32 {
    // Streams are JSON by nature; `--json` is accepted but changes nothing.
    let _ = wants_json(args);

    let stream = match connect_to(socket_override(args)) {
        Ok(stream) => stream,
        Err(_) => {
            emit_json(&error_envelope(
                "phosphor is not running",
                "start the GUI: phosphor",
            ));
            return 2;
        }
    };

    let mut stream = stream;
    if send_line(&mut stream, &json!({"op": "tap"})).is_err() {
        emit_json(&error_envelope(
            "could not subscribe to the beam tap",
            "check that the phosphor GUI is healthy; restart it if needed",
        ));
        return 4;
    }

    // First line is the SERVER-emitted hello (protocol v1): it names
    // which instance answered (pid, socket, versions). We relay it —
    // and every later line — verbatim. When the stream ends we say WHY
    // with a final `end` event: a vanished server exits 4, our own
    // consumer hanging up (EPIPE) stays a clean 0.
    let mut lines_seen: u64 = 0;
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = match line {
            Ok(line) => line,
            Err(_) => {
                // read error: the server vanished mid-stream
                emit_json_line(&end_event("read-error"));
                return 4;
            }
        };
        let mut out = std::io::stdout().lock();
        if writeln!(out, "{line}").is_err() {
            // EPIPE — the consumer closed. Clean exit, no unwrap.
            return 0;
        }
        lines_seen += 1;
    }
    // EOF from the server side: it quit (or never said hello at all).
    emit_json_line(&end_event("server-eof"));
    if lines_seen == 0 { 4 } else { 0 }
}

/// Best-effort single NDJSON line on stdout (stream events, not
/// envelopes — an already-closed consumer is fine).
fn emit_json_line(value: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{value}");
}

// ---------------------------------------------------------------------------
// schema (the machine map)
// ---------------------------------------------------------------------------

fn modes() -> Vec<&'static str> {
    phosphor_dsp::Mode::ALL.iter().map(|m| m.name()).collect()
}

fn ui_styles() -> Vec<&'static str> {
    crate::theme::PALETTES.iter().map(|p| p.id).collect()
}

/// The schema document — extracted so tests exercise the real thing.
fn schema_document() -> Value {
    let strict_object = |properties: Value| {
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": properties,
        })
    };

    let probe_schema = strict_object(json!({
        "status": {"enum": ["ok", "error"]},
        "tool": {"const": "phosphor"},
        "version": {"type": "string"},
        "ts": {"type": "string"},
        "running": {"type": "boolean"},
        "pid": {"type": ["integer", "null"]},
        "mode": {"type": "string"},
        "theme": {"type": "string"},
        "ui_style": {"type": "string"},
        "capture": strict_object(json!({
            "on": {"type": "boolean"},
            "target_id": {"type": ["string", "null"]},
        })),
        "source": strict_object(json!({
            "kind": {"enum": ["capture", "mix", "player", "silent"]},
            "detail": {"type": ["string", "null"]},
        })),
        "player": strict_object(json!({
            "track": {"type": ["string", "null"]},
            "title": {"type": ["string", "null"]},
            "artist": {"type": ["string", "null"]},
            "position_seconds": {"type": "number"},
            "duration_seconds": {"type": ["number", "null"]},
            "paused": {"type": "boolean"},
        })),
        "volume": {"type": "number"},
        "gain": strict_object(json!({
            "setting": {"type": "number"},
            "effective": {"type": "number"},
            "auto": {"type": "boolean"},
        })),
        "kit": strict_object(json!({
            "enabled": {"type": "boolean"},
            "path": {"type": ["string", "null"]},
        })),
        "window": strict_object(json!({
            "mini": {"type": "boolean"},
            "fullscreen": {"type": "boolean"},
        })),
        "vacuum": strict_object(json!({
            "file": {"type": "boolean"},
            "app": {"type": ["string", "null"]},
        })),
        "quiet": strict_object(json!({
            "render_active": {"type": "boolean"},
        })),
        "fps": {"type": "number"},
        // null unless the Custom-theme beam color cycle is animating
        // (v4.1); `current` moves every tick — poll it to watch the
        // color travel without screenshotting
        "beam_cycle": {"oneOf": [
            {"type": "null"},
            strict_object(json!({
                "colors": {"type": "integer",
                            "minimum": 2, "maximum": 3},
                "seconds": {"type": "number"},
                "mode": {"enum": ["timer", "track"]},
                "current": {"type": "array",
                            "items": {"type": "number"},
                            "minItems": 3, "maxItems": 3},
            })),
        ]},
    }));

    let ctl_schema = strict_object(json!({
        "status": {"enum": ["ok", "error"]},
        "tool": {"const": "phosphor"},
        "version": {"type": "string"},
        "ts": {"type": "string"},
        "verb": {"type": "string"},
        "result": {"type": ["object", "null"]},
        "error": {"type": "string"},
        "fix": {"type": "string"},
    }));

    let tap_frame_schema = strict_object(json!({
        "event": {"const": "frame"},
        "ts_ms": {"type": "integer"},
        "mode": {"type": "string"},
        "segments": {"type": "integer"},
        "bbox": {"type": ["array", "null"]},
        "centroid": {"type": ["array", "null"]},
        "peak": {"type": "number"},
        "polyline": {
            "type": "array",
            "items": {"type": "array", "items": {"type": "number"}},
        },
        "trace_size": {"type": "array", "items": {"type": "integer"}},
    }));

    let document = json!({
        "tool": TOOL,
        "version": version(),
        "convention": "NexusFormStationWork/CONVENTION.md",
        "verbs": {
            "probe": {
                "summary": "one-shot status of the running GUI",
                "flags": {"--json": "force JSON on a TTY",
                          "--at": "offline probe — lands with the studio wave"},
                "exit": {"0": "running or not-running (both ok)",
                         "2": "--at not built yet", "4": "socket unreadable"},
            },
            "ctl": {
                "summary": "send one control verb to the running GUI",
                "verbs": {
                    "play": {"args": {"path": "string?"}},
                    "pause": {"args": {}},
                    "toggle": {"args": {}},
                    "stop": {"args": {}},
                    "next": {"args": {}},
                    "previous": {"args": {}},
                    "seek": {"args": {"seconds": "number"}},
                    "volume": {"args": {"value": "number 0..1"}},
                    "mode": {"args": {"name": "string (see enums.modes)"}},
                    "theme": {"args": {"name": "string (see enums.themes)"}},
                    "ui": {"args": {"name": "string (see enums.ui_styles)"}},
                    "capture": {"args": {"on": "bool"}},
                    "target": {"args": {"id": "integer|string"},
                               "note": "mix:app:a+app:b folds several \
                                        app streams into one beam"},
                    "raise": {"args": {}, "note": "focus + deiconify the window"},
                    "open": {"args": {"path": "audio file path"},
                             "note": "load into the player and focus"},
                    "snapshot": {"args": {}, "note": "reply deferred until PNG lands"},
                    "clip": {"args": {}, "note": "reply deferred until mp4 lands"},
                    "quit": {"args": {}},
                },
            },
            "tap": {
                "summary": "NDJSON stream of beam frames",
                "events": {
                    "hello": {"tool": "string", "version": "string",
                              "protocol": "integer", "pid": "integer",
                              "socket": "string", "schema": "string",
                              "note": "first line, server-emitted — names \
                                       the instance that answered"},
                    "frame": {"ts_ms": "integer", "mode": "string",
                              "segments": "integer", "bbox": "array|null",
                              "centroid": "array|null", "peak": "number",
                              "polyline": "array of [x,y]",
                              "trace_size": "[w,h]"},
                    "tick": {"ts_ms": "integer", "note": "idle heartbeat"},
                    "end": {"reason": "server-eof|read-error",
                            "note": "client-emitted last line; exit 4 when \
                                     the server vanished before any line"},
                },
            },
            "kit": {
                "summary": "validate|inspect a .phoskit transform chain",
                "verbs": {
                    "validate": {"args": {"file": "path"}},
                    "inspect": {"args": {"file": "path"}},
                },
                "schema": "docs/phoskit.schema.json",
            },
            "feed": {
                "note": "locked v3-verbatim applet protocol; lines are \
                         {\"s\":[...]} without an event field — the \
                         documented exception",
            },
            "schema": {},
        },
        "outputs": {
            "probe": probe_schema,
            "ctl": ctl_schema,
            "tap_frame": tap_frame_schema,
        },
        "enums": {
            "modes": modes(),
            "themes": crate::chrome::THEME_NAMES,
            "ui_styles": ui_styles(),
            "exit_codes": {
                "0": "ok",
                "2": "unavailable",
                "3": "bad arguments",
                "4": "runtime failure",
            },
        },
    });

    document
}

pub fn run_schema(_args: &[String]) -> i32 {
    // schema is ALWAYS one JSON document (no envelope, no TTY switch).
    if let Ok(text) = serde_json::to_string_pretty(&schema_document()) {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{text}");
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ok_envelope_has_the_required_frame() {
        let envelope = ok_envelope();
        assert_eq!(envelope["status"], json!("ok"));
        assert_eq!(envelope["tool"], json!("phosphor"));
        assert!(envelope.contains_key("version"));
        assert!(envelope.contains_key("ts"));
    }

    #[test]
    fn error_envelope_always_carries_a_fix() {
        let envelope = error_envelope("it broke", "do this");
        assert_eq!(envelope["status"], json!("error"));
        assert_eq!(envelope["error"], json!("it broke"));
        assert_eq!(envelope["fix"], json!("do this"));
        // an error without a fix is a bug — the type forbids it.
    }

    #[test]
    fn ctl_maps_every_verb_to_the_wire() {
        let m = build_ctl_message("play", &["/tmp/x.wav"]).unwrap();
        assert_eq!(m["op"], json!("ctl"));
        assert_eq!(m["verb"], json!("play"));
        assert_eq!(m["args"]["path"], json!("/tmp/x.wav"));

        assert_eq!(build_ctl_message("play", &[]).unwrap()["args"], json!({}));
        assert_eq!(build_ctl_message("pause", &[]).unwrap()["args"], json!({}));
        assert_eq!(build_ctl_message("toggle", &[]).unwrap()["args"], json!({}));
        assert_eq!(build_ctl_message("stop", &[]).unwrap()["args"], json!({}));
        assert_eq!(build_ctl_message("next", &[]).unwrap()["args"], json!({}));
        assert_eq!(
            build_ctl_message("previous", &[]).unwrap()["args"],
            json!({})
        );
        assert_eq!(
            build_ctl_message("snapshot", &[]).unwrap()["args"],
            json!({})
        );
        assert_eq!(build_ctl_message("clip", &[]).unwrap()["args"], json!({}));
        assert_eq!(build_ctl_message("quit", &[]).unwrap()["args"], json!({}));

        assert_eq!(
            build_ctl_message("seek", &["-10"]).unwrap()["args"]["seconds"],
            json!(-10.0)
        );
        assert_eq!(
            build_ctl_message("volume", &["0.7"]).unwrap()["args"]["value"],
            json!(0.7)
        );
        assert_eq!(
            build_ctl_message("mode", &["ring"]).unwrap()["args"]["name"],
            json!("ring")
        );
        assert_eq!(
            build_ctl_message("theme", &["Amber"]).unwrap()["args"]["name"],
            json!("Amber")
        );
        assert_eq!(
            build_ctl_message("ui", &["chromacore"]).unwrap()["args"]["name"],
            json!("chromacore")
        );
        assert_eq!(
            build_ctl_message("capture", &["on"]).unwrap()["args"]["on"],
            json!(true)
        );
        assert_eq!(
            build_ctl_message("capture", &["off"]).unwrap()["args"]["on"],
            json!(false)
        );
        assert_eq!(
            build_ctl_message("target", &["42"]).unwrap()["args"]["id"],
            json!(42)
        );
        assert_eq!(
            build_ctl_message("target", &["sink.hdmi"]).unwrap()["args"]["id"],
            json!("sink.hdmi")
        );
    }

    #[test]
    fn ctl_rejects_bad_values_and_verbs() {
        assert!(build_ctl_message("seek", &[]).is_err());
        assert!(build_ctl_message("seek", &["soon"]).is_err());
        assert!(build_ctl_message("volume", &[]).is_err());
        assert!(build_ctl_message("volume", &["9.0"]).is_err());
        assert!(build_ctl_message("volume", &["-0.1"]).is_err());
        assert!(build_ctl_message("mode", &[]).is_err());
        assert!(build_ctl_message("capture", &["maybe"]).is_err());
        assert!(build_ctl_message("target", &[]).is_err());
        assert!(build_ctl_message("frobnicate", &[]).is_err());
    }

    #[test]
    fn schema_doc_parses_with_live_nonempty_enums() {
        let doc = schema_document();
        // round-trips as valid JSON
        let text = serde_json::to_string(&doc).unwrap();
        let _reparsed: Value = serde_json::from_str(&text).unwrap();

        let enums = &doc["enums"];
        let modes = enums["modes"].as_array().unwrap();
        let themes = enums["themes"].as_array().unwrap();
        let ui = enums["ui_styles"].as_array().unwrap();
        assert_eq!(modes.len(), 11, "all 11 Mode names, iterated not hardcoded");
        assert!(!themes.is_empty());
        assert!(!ui.is_empty());
        assert!(modes.iter().any(|m| m == "ring"));
        assert!(ui.iter().any(|u| u == "blossom"));
    }

    #[test]
    fn schema_outputs_are_strict() {
        let doc = schema_document();
        for output in ["probe", "ctl", "tap_frame"] {
            assert_eq!(
                doc["outputs"][output]["additionalProperties"],
                json!(false),
                "{output} schema must forbid extra properties"
            );
        }
        // and the ctl verb table is present
        assert!(doc["verbs"]["ctl"]["verbs"]["snapshot"].is_object());
        assert_eq!(
            doc["verbs"]["kit"]["schema"],
            json!("docs/phoskit.schema.json")
        );
    }

    /// Step 7 of the round trip: every verb the shared protocol knows
    /// is documented in the schema, and vice versa — the schema's verb
    /// table can't drift from the wire again.
    #[test]
    fn schema_ctl_verbs_match_the_shared_protocol_exactly() {
        let doc = schema_document();
        let table = doc["verbs"]["ctl"]["verbs"].as_object().unwrap();
        let mut protocol_verbs: Vec<&str> = CtlRequest::EXAMPLES.iter().map(|(v, _)| *v).collect();
        protocol_verbs.sort_unstable();
        protocol_verbs.dedup();
        for verb in &protocol_verbs {
            assert!(
                table.contains_key(*verb),
                "schema is missing ctl verb '{verb}'"
            );
        }
        for verb in table.keys() {
            assert!(
                protocol_verbs.contains(&verb.as_str()),
                "schema documents '{verb}' the protocol doesn't parse"
            );
        }
        // and every CLI example builds a wire message (executability)
        for (verb, rest) in CtlRequest::EXAMPLES {
            build_ctl_message(verb, rest).unwrap_or_else(|e| panic!("{verb}: {e}"));
        }
    }

    /// The probe schema and the server's StatusSnapshot are the same
    /// fact: a default snapshot must validate against the published
    /// property types (nullability included — duration_seconds,
    /// vacuum.file and capture.target_id drifted once).
    #[test]
    fn default_status_snapshot_validates_against_the_probe_schema() {
        fn check(value: &Value, schema: &Value, path: &str) {
            if let Some(kinds) = schema.get("type") {
                let kinds: Vec<String> = match kinds {
                    Value::String(s) => vec![s.clone()],
                    Value::Array(a) => a
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect(),
                    _ => vec![],
                };
                if !kinds.is_empty() {
                    let actual = match value {
                        Value::Null => "null",
                        Value::Bool(_) => "boolean",
                        Value::Number(n) if n.is_i64() || n.is_u64() => "integer",
                        Value::Number(_) => "number",
                        Value::String(_) => "string",
                        Value::Array(_) => "array",
                        Value::Object(_) => "object",
                    };
                    let fits = kinds
                        .iter()
                        .any(|k| k == actual || (k == "number" && actual == "integer"));
                    assert!(fits, "{path}: runtime {actual}, schema {kinds:?}");
                }
            }
            if let (Value::Object(fields), Some(properties)) =
                (value, schema.get("properties").and_then(Value::as_object))
            {
                for (key, field) in fields {
                    let sub = properties
                        .get(key)
                        .unwrap_or_else(|| panic!("{path}.{key}: not in the probe schema"));
                    check(field, sub, &format!("{path}.{key}"));
                }
            }
        }
        let snapshot = serde_json::to_value(crate::control::StatusSnapshot::default()).unwrap();
        let schema = schema_document()["outputs"]["probe"].clone();
        check(&snapshot, &schema, "status");
    }
}
