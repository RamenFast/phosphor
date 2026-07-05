// SPDX-License-Identifier: GPL-3.0-or-later
//! `phosphor kit validate|inspect <file>` — the agent-facing lens on a
//! .phoskit transform chain. Loading/validation lives in
//! phosphor_proto::phoskit (v3-verbatim clamps and its short, directive
//! error voice); this module only wraps that in the station envelope so
//! a small model can repair its kit in one round-trip. Every error names
//! a fix and points at docs/phoskit.schema.json.

use std::io::{IsTerminal, Write};
use std::path::Path;

use phosphor_proto::phoskit::{self, OPERATIONS};
use serde_json::{json, Map, Value};

const TOOL: &str = "phosphor";
const OPS_FIX: &str = "see docs/phoskit.schema.json; ops: rotate, midside, \
                       ringmod, wobble, matrix, chandelay";

const USAGE: &str = "\
usage: phosphor kit <validate|inspect> <file.phoskit> [--json]
  validate   is this a well-formed kit?  (ok | error+fix)
  inspect    list its stages, params, and what each op does";

fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

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
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        )
    } else {
        output
    }
}

fn wants_json(args: &[String]) -> bool {
    args.iter().any(|a| a == "--json") || !std::io::stdout().is_terminal()
}

fn base_envelope(status: &str) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("status".into(), json!(status));
    map.insert("tool".into(), json!(TOOL));
    map.insert("version".into(), json!(version()));
    map.insert("ts".into(), json!(now()));
    map
}

fn emit_json(map: &Map<String, Value>) {
    if let Ok(text) = serde_json::to_string(&Value::Object(map.clone())) {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{text}");
    }
}

pub fn run(args: &[String]) -> i32 {
    let json = wants_json(args);
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| a.as_str() != "--json")
        .map(String::as_str)
        .collect();

    let verb = match positional.first() {
        Some(verb) => *verb,
        None => {
            eprintln!("{USAGE}");
            return 3;
        }
    };
    let file = match positional.get(1) {
        Some(file) => *file,
        None => {
            eprintln!("phosphor kit: {verb} needs a file\n{USAGE}");
            return 3;
        }
    };

    match verb {
        "validate" => validate(Path::new(file), json),
        "inspect" => inspect(Path::new(file), json),
        other => {
            eprintln!("phosphor kit: unknown verb '{other}'\n{USAGE}");
            3
        }
    }
}

/// exit 3 if the file is missing/unreadable (usage-grade), else 4 if it
/// exists but is not a well-formed kit.
fn load_failure_code(path: &Path) -> i32 {
    if path.is_file() {
        4
    } else {
        3
    }
}

fn validate(path: &Path, json: bool) -> i32 {
    match phoskit::load(path) {
        Ok(kit) => {
            let mut envelope = base_envelope("ok");
            envelope.insert("valid".into(), json!(true));
            envelope.insert("name".into(), json!(kit.name));
            envelope.insert("stages".into(), json!(kit.stages.len()));
            if json {
                emit_json(&envelope);
            } else {
                println!(
                    "valid: {} ({} stage{})",
                    kit.name,
                    kit.stages.len(),
                    if kit.stages.len() == 1 { "" } else { "s" }
                );
            }
            0
        }
        Err(error) => {
            let code = load_failure_code(path);
            let mut envelope = base_envelope("error");
            envelope.insert("valid".into(), json!(false));
            envelope.insert("error".into(), json!(error));
            envelope.insert("fix".into(), json!(OPS_FIX));
            if json {
                emit_json(&envelope);
            } else {
                eprintln!("invalid: {error}\n  fix: {OPS_FIX}");
            }
            code
        }
    }
}

fn inspect(path: &Path, json: bool) -> i32 {
    match phoskit::load(path) {
        Ok(kit) => {
            let stages: Vec<Value> = kit
                .stages
                .iter()
                .map(|(op, packed)| {
                    let mut params = Map::new();
                    if let Some((_, table)) =
                        OPERATIONS.iter().find(|(name, _)| name == op)
                    {
                        for (slot, (key, _, _, _)) in table.iter().enumerate() {
                            params.insert(
                                (*key).to_string(),
                                json!(packed[slot]),
                            );
                        }
                    }
                    json!({
                        "op": op,
                        "params": params,
                        "description": phoskit::op_description(op),
                    })
                })
                .collect();

            if json {
                let mut envelope = base_envelope("ok");
                envelope.insert("name".into(), json!(kit.name));
                envelope.insert("author".into(), json!(kit.author));
                envelope.insert("stages".into(), json!(stages));
                emit_json(&envelope);
            } else {
                print_inspect_human(&kit.name, &kit.author, &stages);
            }
            0
        }
        Err(error) => {
            let code = load_failure_code(path);
            let mut envelope = base_envelope("error");
            envelope.insert("valid".into(), json!(false));
            envelope.insert("error".into(), json!(error));
            envelope.insert("fix".into(), json!(OPS_FIX));
            if json {
                emit_json(&envelope);
            } else {
                eprintln!("invalid: {error}\n  fix: {OPS_FIX}");
            }
            code
        }
    }
}

fn print_inspect_human(name: &str, author: &str, stages: &[Value]) {
    if author.is_empty() {
        println!("{name} — {} stage(s)", stages.len());
    } else {
        println!("{name} by {author} — {} stage(s)", stages.len());
    }
    for (index, stage) in stages.iter().enumerate() {
        let op = stage["op"].as_str().unwrap_or("?");
        let description = stage["description"].as_str().unwrap_or("");
        let params = stage["params"]
            .as_object()
            .map(|params| {
                params
                    .iter()
                    .map(|(key, value)| format!("{key}={value}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        println!("  {}. {op}  {params}", index + 1);
        if !description.is_empty() {
            println!("     {description}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn validate_accepts_a_starter_kit() {
        let code = validate(&repo().join("kits/haunt.phoskit"), true);
        assert_eq!(code, 0);
        // and the loader agrees on the shape we report
        let kit = phoskit::load(&repo().join("kits/haunt.phoskit")).unwrap();
        assert_eq!(kit.name, "haunt");
        assert_eq!(kit.stages.len(), 3);
    }

    #[test]
    fn validate_rejects_a_broken_kit_with_exit_4() {
        let dir = std::env::temp_dir().join("phosphor-kit-cli-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("broken.phoskit");
        std::fs::write(
            &path,
            r#"{"phoskit":1,"name":"bad","stages":[{"op":"sparkle"}]}"#,
        )
        .unwrap();
        // exists but invalid → runtime failure
        assert_eq!(validate(&path, true), 4);
        // and the underlying error names the fix path
        assert!(phoskit::load(&path).is_err());
    }

    #[test]
    fn validate_missing_file_is_usage_grade_exit_3() {
        let missing = repo().join("kits/does-not-exist.phoskit");
        assert_eq!(validate(&missing, true), 3);
    }

    #[test]
    fn inspect_names_every_param_slot() {
        let kit = phoskit::load(&repo().join("kits/haunt.phoskit")).unwrap();
        // haunt = chandelay, wobble, midside
        assert_eq!(kit.stages[0].0, "chandelay");
        // OPERATIONS names chandelay's slots ms, channel
        let table = OPERATIONS
            .iter()
            .find(|(name, _)| *name == "chandelay")
            .unwrap()
            .1;
        assert_eq!(table[0].0, "ms");
        assert_eq!(table[1].0, "channel");
        // inspect runs clean on a good kit
        assert_eq!(inspect(&repo().join("kits/haunt.phoskit"), true), 0);
    }

    #[test]
    fn bare_kit_is_bad_arguments() {
        assert_eq!(run(&[]), 3);
        assert_eq!(run(&["validate".to_string()]), 3);
        assert_eq!(run(&["frobnicate".to_string(), "x".to_string()]), 3);
    }
}
