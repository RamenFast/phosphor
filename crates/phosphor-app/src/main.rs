// SPDX-License-Identifier: GPL-3.0-or-later
//! `phosphor` — one binary, subcommand-first, GUI as the default
//! command. CLI surface is a contract (V4PLAN): `render`, `bench`,
//! `probe`, `tap`, `ctl`, `feed`, `kit validate|inspect`,
//! `studio render|validate|inspect|preview|build`, plus the legacy
//! flag spellings v3 users have in muscle memory / scripts:
//! `--render`, `--mini`, `--screensaver`, `--visitor`.
//! All agent-grade: `--output json`, exit codes 0/2/3/4.
//!
//! Wave-1 scaffold: dispatch skeleton only. Each arm lands with its
//! wave; until then it exits 2 with a short directive message (never
//! a silent success — no fallback paths anywhere in v4).

use std::process::ExitCode;

const SUBCOMMANDS: &[&str] = &[
    "render", "bench", "probe", "tap", "ctl", "feed", "kit", "studio",
];

fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let first = arguments.first().map(String::as_str);
    match first {
        Some("--version") | Some("-V") => {
            println!("phosphor {} (v4 scaffold)", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some(name) if SUBCOMMANDS.contains(&name) => {
            eprintln!("phosphor {name}: not built yet (v4 wave in progress; \
                       see V4PLAN.md)");
            ExitCode::from(2)
        }
        Some("--render") => {
            eprintln!("phosphor --render: not built yet (v4 wave 1 step 5)");
            ExitCode::from(2)
        }
        _ => {
            eprintln!("phosphor GUI: not built yet (v4 wave 2); v3 remains \
                       the daily driver until the parity checklist passes");
            ExitCode::from(2)
        }
    }
}
