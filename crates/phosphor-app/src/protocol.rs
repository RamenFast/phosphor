// SPDX-License-Identifier: GPL-3.0-or-later
//! The ctl wire protocol — ONE typed representation of every control
//! verb, used by BOTH halves: the CLI client (`agent.rs`) builds a
//! `CtlRequest` from positional argv and serializes it, the socket
//! server (`control.rs`) parses the same type back off the wire. A verb
//! that round-trips here round-trips on the socket by construction
//! (BUGLOG #16 — the two halves drifted when each kept its own copy).
//!
//! Wire framing (frozen): one NDJSON object per line,
//! `{"op":"ctl","verb":<verb>,"args":{…}}`. The applet `feed` protocol
//! is a separate, frozen surface and is NOT touched by this module.

use serde_json::{Value, json};

/// Version of the ctl/tap protocol itself (independent of the app
/// version). Bump when the wire shape changes incompatibly.
pub(crate) const PROTOCOL_VERSION: u32 = 1;

/// A capture target id: a PipeWire node number, or a name key such as
/// `app:Spotify`, `device:…`, `mix:app:a+app:b`. The CLI documents
/// both; the wire carries a JSON integer or string accordingly.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum TargetId {
    Node(i64),
    Name(String),
}

impl TargetId {
    fn to_wire(&self) -> Value {
        match self {
            TargetId::Node(n) => json!(n),
            TargetId::Name(s) => json!(s),
        }
    }
}

impl std::fmt::Display for TargetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TargetId::Node(n) => write!(f, "{n}"),
            TargetId::Name(s) => write!(f, "{s}"),
        }
    }
}

/// A wire-level parse error: what went wrong plus a fix the caller can
/// EXECUTE verbatim (fixes are covered by an executability test — a
/// fix that doesn't re-parse is a bug).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct WireError {
    pub error: String,
    pub fix: String,
}

impl WireError {
    fn new(error: &str, fix: &str) -> Self {
        WireError {
            error: error.into(),
            fix: fix.into(),
        }
    }

    /// The ready-to-write NDJSON error reply.
    pub fn to_value(&self) -> Value {
        json!({"status": "error", "error": self.error, "fix": self.fix})
    }
}

/// Every ctl verb, with its typed arguments. THE canonical list — the
/// CLI parser, the server parser, and the round-trip tests all walk
/// `CtlRequest::EXAMPLES` so a verb can't exist on one side only.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CtlRequest {
    Play { path: Option<String> },
    Pause,
    Toggle,
    Stop,
    Next,
    Previous,
    Seek { seconds: f64 },
    Volume { value: f64 },
    Gain { value: Option<f64>, auto: bool },
    Mode { name: String },
    Theme { name: String },
    Ui { name: String },
    Capture { on: bool },
    Target { id: TargetId },
    Snapshot,
    Clip,
    Raise,
    Open { path: String },
    Quit,
}

impl CtlRequest {
    /// The wire verb string.
    pub fn verb(&self) -> &'static str {
        match self {
            CtlRequest::Play { .. } => "play",
            CtlRequest::Pause => "pause",
            CtlRequest::Toggle => "toggle",
            CtlRequest::Stop => "stop",
            CtlRequest::Next => "next",
            CtlRequest::Previous => "previous",
            CtlRequest::Seek { .. } => "seek",
            CtlRequest::Volume { .. } => "volume",
            CtlRequest::Gain { .. } => "gain",
            CtlRequest::Mode { .. } => "mode",
            CtlRequest::Theme { .. } => "theme",
            CtlRequest::Ui { .. } => "ui",
            CtlRequest::Capture { .. } => "capture",
            CtlRequest::Target { .. } => "target",
            CtlRequest::Snapshot => "snapshot",
            CtlRequest::Clip => "clip",
            CtlRequest::Raise => "raise",
            CtlRequest::Open { .. } => "open",
            CtlRequest::Quit => "quit",
        }
    }

    /// One example CLI invocation per verb (verb + positional args) —
    /// the round-trip test drives every one of these through the CLI
    /// parser, real socket framing, and the server parser.
    #[cfg(test)]
    pub const EXAMPLES: &'static [(&'static str, &'static [&'static str])] = &[
        ("play", &[]),
        ("play", &["/music/song.flac"]),
        ("pause", &[]),
        ("toggle", &[]),
        ("stop", &[]),
        ("next", &[]),
        ("previous", &[]),
        ("seek", &["-10"]),
        ("volume", &["0.7"]),
        ("gain", &["1.5"]),
        ("gain", &["auto"]),
        ("mode", &["ring"]),
        ("theme", &["Amber"]),
        ("ui", &["amber"]),
        ("capture", &["on"]),
        ("capture", &["off"]),
        ("target", &["42"]),
        ("target", &["app:Spotify"]),
        ("target", &["mix:app:one+app:two"]),
        ("snapshot", &[]),
        ("clip", &[]),
        ("raise", &[]),
        ("open", &["/music/song.flac"]),
        ("quit", &[]),
    ];

    /// The frozen wire message `{"op":"ctl","verb":…,"args":{…}}`.
    pub fn to_wire(&self) -> Value {
        let args: Value = match self {
            CtlRequest::Play { path: Some(path) } => json!({ "path": path }),
            CtlRequest::Play { path: None }
            | CtlRequest::Pause
            | CtlRequest::Toggle
            | CtlRequest::Stop
            | CtlRequest::Next
            | CtlRequest::Previous
            | CtlRequest::Snapshot
            | CtlRequest::Clip
            | CtlRequest::Raise
            | CtlRequest::Quit => json!({}),
            CtlRequest::Seek { seconds } => json!({ "seconds": seconds }),
            CtlRequest::Volume { value } => json!({ "value": value }),
            CtlRequest::Gain { value, auto } => {
                json!({ "value": value, "auto": auto })
            }
            CtlRequest::Mode { name } | CtlRequest::Theme { name } | CtlRequest::Ui { name } => {
                json!({ "name": name })
            }
            CtlRequest::Capture { on } => json!({ "on": on }),
            CtlRequest::Target { id } => json!({ "id": id.to_wire() }),
            CtlRequest::Open { path } => json!({ "path": path }),
        };
        json!({ "op": "ctl", "verb": self.verb(), "args": args })
    }

    /// Parse a positional CLI invocation (`phosphor ctl <verb> [value]`).
    /// `Err` is a bad-arguments (exit 3) message for a human.
    pub fn from_cli(verb: &str, rest: &[&str]) -> Result<Self, String> {
        let request = match verb {
            "play" => CtlRequest::Play {
                path: rest.first().map(|s| s.to_string()),
            },
            "pause" => CtlRequest::Pause,
            "toggle" => CtlRequest::Toggle,
            "stop" => CtlRequest::Stop,
            "next" => CtlRequest::Next,
            "previous" => CtlRequest::Previous,
            "snapshot" => CtlRequest::Snapshot,
            "clip" => CtlRequest::Clip,
            "raise" => CtlRequest::Raise,
            "quit" => CtlRequest::Quit,
            "open" => CtlRequest::Open {
                path: rest
                    .first()
                    .ok_or("open needs a path, e.g. `open /music/song.flac`")?
                    .to_string(),
            },
            "seek" => CtlRequest::Seek {
                seconds: rest
                    .first()
                    .and_then(|s| s.parse::<f64>().ok())
                    .ok_or("seek needs a number of seconds, e.g. `seek -10`")?,
            },
            "volume" => CtlRequest::Volume {
                value: rest
                    .first()
                    .and_then(|s| s.parse::<f64>().ok())
                    .filter(|v| (0.0..=1.0).contains(v))
                    .ok_or("volume needs a value in 0..1, e.g. `volume 0.7`")?,
            },
            "gain" => match rest.first().copied() {
                Some("auto") => CtlRequest::Gain {
                    value: None,
                    auto: true,
                },
                Some(value) => CtlRequest::Gain {
                    value: Some(
                        value
                            .parse::<f64>()
                            .ok()
                            .filter(|value| value.is_finite())
                            .ok_or(
                                "gain needs a number or `auto`, e.g. \
                                 `phosphor ctl gain 1.5`",
                            )?
                            .clamp(0.1, 6.0),
                    ),
                    auto: false,
                },
                None => {
                    return Err("gain needs a number or `auto`, e.g. \
                         `phosphor ctl gain 1.5`"
                        .into());
                }
            },
            "mode" => CtlRequest::Mode {
                name: rest
                    .first()
                    .ok_or("mode needs a name, e.g. `mode ring`")?
                    .to_string(),
            },
            "theme" => CtlRequest::Theme {
                name: rest
                    .first()
                    .ok_or("theme needs a name, e.g. `theme Amber`")?
                    .to_string(),
            },
            "ui" => CtlRequest::Ui {
                name: rest
                    .first()
                    .ok_or("ui needs a style, e.g. `ui chromacore`")?
                    .to_string(),
            },
            "capture" => CtlRequest::Capture {
                on: match rest.first().copied() {
                    Some("on") => true,
                    Some("off") => false,
                    _ => return Err("capture takes `on` or `off`".into()),
                },
            },
            "target" => {
                let id = rest.first().ok_or("target needs an id")?;
                // Prefer an integer id (PipeWire node); else a name key.
                CtlRequest::Target {
                    id: match id.parse::<i64>() {
                        Ok(number) => TargetId::Node(number),
                        Err(_) => TargetId::Name(id.to_string()),
                    },
                }
            }
            other => return Err(format!("unknown verb '{other}'")),
        };
        Ok(request)
    }

    /// Parse the server side of the wire: verb string + `args` object.
    /// `Err` carries a fix in the exact positional syntax the CLI
    /// accepts (the executability test runs every one).
    pub fn from_wire_args(verb: &str, args: &Value) -> Result<Self, WireError> {
        let str_arg = |key: &str| -> Option<String> {
            args.get(key).and_then(Value::as_str).map(str::to_string)
        };
        let request = match verb {
            "play" => CtlRequest::Play {
                path: str_arg("path"),
            },
            "pause" => CtlRequest::Pause,
            "toggle" => CtlRequest::Toggle,
            "stop" => CtlRequest::Stop,
            "next" => CtlRequest::Next,
            "previous" => CtlRequest::Previous,
            "snapshot" => CtlRequest::Snapshot,
            "clip" => CtlRequest::Clip,
            "raise" => CtlRequest::Raise,
            "quit" => CtlRequest::Quit,
            "seek" => CtlRequest::Seek {
                seconds: args.get("seconds").and_then(Value::as_f64).ok_or_else(|| {
                    WireError::new(
                        "seek needs a numeric 'seconds' (relative)",
                        "phosphor ctl seek -5",
                    )
                })?,
            },
            "volume" => CtlRequest::Volume {
                value: args
                    .get("value")
                    .and_then(Value::as_f64)
                    .ok_or_else(|| {
                        WireError::new(
                            "volume needs a numeric 'value' in 0..1",
                            "phosphor ctl volume 0.8",
                        )
                    })?
                    .clamp(0.0, 1.0),
            },
            "gain" => {
                if args.get("auto").and_then(Value::as_bool) == Some(true) {
                    CtlRequest::Gain {
                        value: None,
                        auto: true,
                    }
                } else {
                    CtlRequest::Gain {
                        value: Some(
                            args.get("value")
                                .and_then(Value::as_f64)
                                .filter(|value| value.is_finite())
                                .ok_or_else(|| {
                                    WireError::new(
                                        "gain needs a numeric 'value' or auto",
                                        "phosphor ctl gain 1.5",
                                    )
                                })?
                                .clamp(0.1, 6.0),
                        ),
                        auto: false,
                    }
                }
            }
            "mode" => CtlRequest::Mode {
                name: str_arg("name")
                    .ok_or_else(|| WireError::new("mode needs a 'name'", "phosphor ctl mode xy"))?,
            },
            "theme" => CtlRequest::Theme {
                name: str_arg("name").ok_or_else(|| {
                    WireError::new("theme needs a 'name'", "phosphor ctl theme Amber")
                })?,
            },
            "ui" => CtlRequest::Ui {
                name: str_arg("name")
                    .ok_or_else(|| WireError::new("ui needs a 'name'", "phosphor ctl ui amber"))?,
            },
            "capture" => CtlRequest::Capture {
                on: args.get("on").and_then(Value::as_bool).ok_or_else(|| {
                    WireError::new("capture needs a boolean 'on'", "phosphor ctl capture on")
                })?,
            },
            "target" => {
                // integer (PipeWire node) OR string (name/mix key) —
                // the schema has always advertised both.
                let id = args.get("id").and_then(|v| {
                    if let Some(n) = v.as_i64() {
                        Some(TargetId::Node(n))
                    } else {
                        v.as_str().map(|s| TargetId::Name(s.to_string()))
                    }
                });
                CtlRequest::Target {
                    id: id.ok_or_else(|| {
                        WireError::new(
                            "target needs an 'id' (integer node or name key)",
                            "phosphor ctl target 42",
                        )
                    })?,
                }
            }
            "open" => CtlRequest::Open {
                path: str_arg("path").ok_or_else(|| {
                    WireError::new("open needs a 'path'", "phosphor ctl open /music/song.flac")
                })?,
            },
            other => {
                return Err(WireError::new(
                    &format!("unknown verb '{other}'"),
                    "run: phosphor schema",
                ));
            }
        };
        Ok(request)
    }
}

// ---------------------------------------------------------------------------
// tap stream events — built here so client and server agree on shape
// ---------------------------------------------------------------------------

/// The tap handshake — SERVER-emitted first line since protocol v1
/// (it used to be fabricated client-side, which proved nothing about
/// who answered). Carries enough to identify the instance.
pub(crate) fn hello_event(pid: u32, socket: &str) -> Value {
    json!({
        "event": "hello",
        "tool": "phosphor",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": PROTOCOL_VERSION,
        "pid": pid,
        "socket": socket,
        "schema": "phosphor schema",
    })
}

/// The tap end-of-stream marker the CLIENT emits before exiting, so a
/// consumer can tell a vanished server from its own clean shutdown.
pub(crate) fn end_event(reason: &str) -> Value {
    json!({ "event": "end", "reason": reason })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_example_round_trips_cli_to_wire_to_request() {
        for (verb, rest) in CtlRequest::EXAMPLES {
            let cli =
                CtlRequest::from_cli(verb, rest).unwrap_or_else(|e| panic!("from_cli {verb}: {e}"));
            let wire = cli.to_wire();
            assert_eq!(wire["op"], json!("ctl"));
            assert_eq!(wire["verb"], json!(*verb));
            let parsed = CtlRequest::from_wire_args(wire["verb"].as_str().unwrap(), &wire["args"])
                .unwrap_or_else(|e| panic!("from_wire {verb}: {}", e.error));
            assert_eq!(parsed, cli, "{verb} must round-trip losslessly");
        }
    }

    #[test]
    fn numeric_target_stays_numeric_on_the_wire_and_back() {
        let request = CtlRequest::from_cli("target", &["42"]).unwrap();
        assert_eq!(
            request,
            CtlRequest::Target {
                id: TargetId::Node(42)
            }
        );
        let wire = request.to_wire();
        assert_eq!(wire["args"]["id"], json!(42), "integer, not \"42\"");
        let back = CtlRequest::from_wire_args("target", &wire["args"]).unwrap();
        assert_eq!(back, request);
    }

    #[test]
    fn every_wire_fix_is_an_executable_cli_command() {
        // Trigger each missing-argument error and re-parse its fix
        // through the CLI parser — a fix the CLI rejects is a bug.
        for verb in [
            "seek", "volume", "gain", "mode", "theme", "ui", "capture", "target", "open",
        ] {
            let err = CtlRequest::from_wire_args(verb, &json!({}))
                .err()
                .unwrap_or_else(|| panic!("{verb} with no args must error"));
            let fix = err.fix.clone();
            if let Some(rest) = fix.strip_prefix("phosphor ctl ") {
                let words: Vec<&str> = rest.split_whitespace().collect();
                let (fix_verb, fix_args) = words.split_first().unwrap();
                CtlRequest::from_cli(fix_verb, fix_args)
                    .unwrap_or_else(|e| panic!("fix for {verb} ('{fix}') does not parse: {e}"));
            } else {
                panic!(
                    "fix for {verb} should be a phosphor ctl command, \
                        got '{fix}'"
                );
            }
        }
    }

    #[test]
    fn hello_and_end_events_carry_their_identities() {
        let hello = hello_event(1234, "/run/user/1000/phosphor/ctl.sock");
        assert_eq!(hello["event"], json!("hello"));
        assert_eq!(hello["tool"], json!("phosphor"));
        assert_eq!(hello["pid"], json!(1234));
        assert_eq!(hello["protocol"], json!(PROTOCOL_VERSION));
        assert!(hello["socket"].as_str().unwrap().ends_with("ctl.sock"));

        let end = end_event("server-eof");
        assert_eq!(end["event"], json!("end"));
        assert_eq!(end["reason"], json!("server-eof"));
    }
}
