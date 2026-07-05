// SPDX-License-Identifier: GPL-3.0-or-later
//! `phosphor` — one binary, subcommand-first, GUI as the default
//! command. CLI surface is a contract (V4PLAN): `render`, `bench`,
//! `probe`, `tap`, `ctl`, `feed`, `kit validate|inspect`,
//! `studio render|validate|inspect|preview|build`, plus the legacy
//! flag spellings v3 users have in muscle memory / scripts:
//! `--render`, `--mini`, `--screensaver`, `--visitor`.
//! All agent-grade: `--output json`, exit codes 0/2/3/4.
//!
//! State after waves 1–2.6: the GUI (default command), `render`,
//! `bench`, and `feed` (the Cinnamon-applet panel feed) are real;
//! `probe`/`tap`/`ctl`/`kit` land with wave 3 and `studio` with
//! wave 4 — until then those arms exit 2 with a short directive
//! message (never a silent success — no fallback paths in v4).

use std::process::ExitCode;

mod bench;
mod chrome;
mod feed;
mod compose;
mod exports;
mod keyboard;
mod mpris;
mod player;
mod render;
mod shell;
mod theme;
mod signals;

const PENDING: &[&str] = &["probe", "tap", "ctl", "kit", "studio"];

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let first = arguments.first().map(String::as_str);
    let code = match first {
        Some("--version") | Some("-V") => {
            println!("phosphor {} (v4)", env!("CARGO_PKG_VERSION"));
            0
        }
        Some("render") => render::run(&arguments[1..]),
        Some("--render") => {
            // v3 muscle memory: `phosphor --render in out.mp4 [--rate N]`
            let rest: Vec<String> = arguments.iter().skip(1)
                .cloned().collect();
            render::run(&rest)
        }
        Some("bench") => bench::run(&arguments[1..]),
        Some("feed") => feed::run(&arguments[1..]),
        Some(name) if PENDING.contains(&name) => {
            eprintln!("phosphor {name}: not built yet (v4 wave in \
                       progress; see V4PLAN.md)");
            2
        }
        // GUI is the default command (flags like --mini/--visitor/
        // --fps-log fall through to the shell's own parser)
        _ => shell::run(&arguments),
    };
    ExitCode::from(code as u8)
}
