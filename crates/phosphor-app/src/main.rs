// SPDX-License-Identifier: GPL-3.0-or-later
//! `phosphor` — one binary, subcommand-first, GUI as the default
//! command. CLI surface is a contract (V4PLAN): `render`, `bench`,
//! `probe`, `tap`, `ctl`, `feed`, `kit validate|inspect`,
//! `studio render|validate|inspect|preview|build`, plus the legacy
//! flag spellings v3 users have in muscle memory / scripts:
//! `--render`, `--mini`, `--screensaver`, `--visitor`; and
//! `--background` (GUI on a private Xvfb display — renders, serves the
//! control socket, never maps a window on the user's screen).
//! All agent-grade: `--output json`, exit codes 0/2/3/4.
//!
//! State after wave 3: the GUI (default command), `render`, `bench`,
//! `feed` (the Cinnamon-applet panel feed), and the agent surface —
//! `probe`/`tap`/`ctl`/`kit`/`schema` — are real. Only `studio` is
//! still pending (wave 4); its arm exits 2 with a short directive
//! message (never a silent success — no fallback paths in v4).

use std::process::ExitCode;

mod agent;
mod bench;
mod chrome;
mod control;
mod feed;
mod compose;
mod exports;
mod keyboard;
mod kit;
mod mpris;
mod player;
mod render;
mod shell;
mod theme;
mod signals;

const PENDING: &[&str] = &["studio"];

/// `--background`: run the GUI on a private Xvfb display so it renders
/// (and serves the control socket, feed, tap) without ever mapping a
/// window on the user's screen — no focus steal, game-safe. Works for
/// a human (`phosphor --background &`) and for agents that want a
/// drivable instance. Re-execs under `xvfb-run`; the env guard stops
/// recursion once we're inside the virtual display.
fn reexec_in_background(arguments: &[String]) -> i32 {
    if std::env::var_os("PHOSPHOR_BACKGROUND").is_some() {
        return -1; // already wrapped: fall through to the GUI
    }
    let rest: Vec<&String> = arguments.iter()
        .filter(|a| a.as_str() != "--background")
        .collect();
    let self_exe = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("phosphor --background: cannot find own binary: {error}");
            return 4;
        }
    };
    use std::os::unix::process::CommandExt;
    let error = std::process::Command::new("xvfb-run")
        .arg("-a")
        .args(["-s", "-screen 0 1280x800x24"])
        .arg(self_exe)
        .args(rest)
        .env("PHOSPHOR_BACKGROUND", "1")
        .exec();
    // exec only returns on failure
    eprintln!("phosphor --background: could not launch xvfb-run: {error}");
    eprintln!("fix: install it (sudo apt install xvfb), or run phosphor \
               on your display normally");
    2
}

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.iter().any(|a| a == "--background") {
        let code = reexec_in_background(&arguments);
        if code >= 0 {
            return ExitCode::from(code as u8);
        }
        // -1: we are inside the wrapped instance — run the GUI with
        // the flag stripped.
        let rest: Vec<String> = arguments.iter()
            .filter(|a| a.as_str() != "--background")
            .cloned().collect();
        return ExitCode::from(shell::run(&rest) as u8);
    }
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
        Some("probe") => agent::run_probe(&arguments[1..]),
        Some("tap") => agent::run_tap(&arguments[1..]),
        Some("ctl") => agent::run_ctl(&arguments[1..]),
        Some("kit") => kit::run(&arguments[1..]),
        Some("schema") => agent::run_schema(&arguments[1..]),
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
