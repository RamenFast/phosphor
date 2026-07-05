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
        Some("--help") | Some("-h") | Some("help") => {
            print_help();
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
        Some("--screensaver") => {
            eprintln!("phosphor --screensaver: not built yet (returns \
                       after 4.0)\nfix: run `phosphor` normally, or \
                       `phosphor --mini` for the tiny scope");
            2
        }
        Some(name) if PENDING.contains(&name) => {
            eprintln!("phosphor {name}: not built yet (returns after \
                       4.0)\nfix: see the roadmap issues on GitHub");
            2
        }
        // GUI is the default command
        _ => launch_gui(&arguments),
    };
    ExitCode::from(code as u8)
}

/// The GUI path with a strict front door: unknown flags print help and
/// exit 3 — `phosphor --help` once launched a window; never again. A
/// plain human launch forwards to an already-running instance (focus +
/// optional file) instead of spawning a second window.
fn launch_gui(arguments: &[String]) -> i32 {
    const FLAGS: &[&str] = &["--mini", "--visitor", "--fps-log"];
    const FLAGS_WITH_VALUE: &[&str] = &["--exit-after"];
    let mut iterator = arguments.iter();
    while let Some(argument) = iterator.next() {
        let flag = argument.as_str();
        if FLAGS.contains(&flag) {
            continue;
        }
        if FLAGS_WITH_VALUE.contains(&flag) {
            iterator.next();
            continue;
        }
        if flag.starts_with('-') {
            eprintln!("phosphor: unknown option '{flag}'\n");
            print_help();
            return 3;
        }
        // positional = a file to open; shell::parse_args validates it
    }

    // Scripted/receipt runs opt out of single-instance; so does the
    // wrapped --background GUI (its point is to coexist).
    let scripted = arguments.iter().any(|a| {
        a == "--fps-log" || a == "--exit-after" || a == "--visitor"
    }) || std::env::var_os("PHOSPHOR_BACKGROUND").is_some()
        || std::env::var_os("PHOSPHOR_NO_SINGLE_INSTANCE").is_some();
    if !scripted {
        let play_path = arguments.iter()
            .find(|a| !a.starts_with('-'))
            .map(String::as_str);
        if let Some(code) = agent::forward_to_running_instance(play_path) {
            return code;
        }
    }
    shell::run(arguments)
}

fn print_help() {
    println!(
        "phosphor {} — a GPU oscilloscope for everything your PC plays\n\
         \n\
         usage:\n\
         \x20 phosphor [FILE]              open the scope (or focus the running one)\n\
         \x20 phosphor --mini              tiny always-on-top scope\n\
         \x20 phosphor --background        headless GUI on a private display (agents)\n\
         \x20 phosphor render IN OUT.mp4   offline render of a track or .phos\n\
         \x20 phosphor bench               performance gates (JSON)\n\
         \x20 phosphor probe [--json]      one-shot live status\n\
         \x20 phosphor ctl VERB [ARGS]     drive the running scope\n\
         \x20 phosphor tap                 NDJSON stream of beam frames\n\
         \x20 phosphor feed                beam-segment feed (panel applet)\n\
         \x20 phosphor kit validate|inspect FILE.phoskit…\n\
         \x20 phosphor schema              the machine-readable surface\n\
         \n\
         options:\n\
         \x20 --fps-log                    JSON fps lines on stderr\n\
         \x20 --exit-after SECONDS         quit after a timed run\n\
         \x20 -V, --version                print the version\n\
         \n\
         exit codes: 0 ok · 2 unavailable · 3 bad arguments · 4 runtime\n\
         docs: man phosphor · docs/MANUAL.md · docs/AGENTS.md",
        env!("CARGO_PKG_VERSION")
    );
}
