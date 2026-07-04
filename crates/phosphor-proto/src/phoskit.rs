// SPDX-License-Identifier: GPL-3.0-or-later
//! .phoskit — a shareable transform chain. phosphor_kit.py is the spec;
//! this port keeps its exact defaults, clamps, packed [(op, [p0..p3])]
//! canonical form, and its error VOICE: short and directive, naming the
//! exact JSON path, so a small model can repair its kit in one
//! round-trip. Gated by tests/golden/kits/ (starter-kit stage tables).

use std::path::Path;

pub const FORMAT_VERSION: i64 = 1;
pub const MAX_STAGES: usize = 16;
pub const PARAMETERS_PER_STAGE: usize = 4;

/// One op's parameter table: (json_key, default, min, max) per slot.
pub type OpTable = &'static [(&'static str, f64, f64, f64)];

/// Canonical parameter layout per op: (json_key, default, min, max).
/// Order defines the packed [p0..p3] the engine consumes.
// 3.14159 is v3's LITERAL clamp bound (phosphor_kit.py), not an
// approximation of π — substituting the constant would change the
// format contract by 2.65e-6 radians.
#[allow(clippy::approx_constant)]
pub const OPERATIONS: [(&str, OpTable); 6] = [
    ("rotate", &[("hz", 0.05, -4.0, 4.0),
                 ("angle", 0.0, -3.14159, 3.14159)]),
    ("midside", &[("width", 1.4, 0.0, 4.0)]),
    ("ringmod", &[("hz", 3.0, 0.0, 30.0),
                  ("depth", 0.2, 0.0, 1.0)]),
    ("wobble", &[("hz", 0.7, 0.0, 8.0),
                 ("depth", 0.35, 0.0, 1.5)]),
    ("matrix", &[("a", 1.0, -2.0, 2.0), ("b", 0.0, -2.0, 2.0),
                 ("c", 0.0, -2.0, 2.0), ("d", 1.0, -2.0, 2.0)]),
    ("chandelay", &[("ms", 5.0, 0.0, 50.0),
                    ("channel", 1.0, 0.0, 1.0)]),
];

pub type Stage = (String, [f64; PARAMETERS_PER_STAGE]);

pub struct Kit {
    pub name: String,
    pub author: String,
    pub stages: Vec<Stage>,
}

fn known_ops() -> String {
    let mut names: Vec<&str> =
        OPERATIONS.iter().map(|(name, _)| *name).collect();
    names.sort_unstable();
    names.join(", ")
}

/// Validated canonical stages from the raw JSON `stages` value.
pub fn canonical_stages(raw: &serde_json::Value)
                        -> Result<Vec<Stage>, String> {
    let list = raw.as_array()
        .filter(|list| !list.is_empty())
        .ok_or("stages: expected a non-empty list")?;
    if list.len() > MAX_STAGES {
        return Err(format!("stages: at most {MAX_STAGES} allowed"));
    }
    let mut stages = Vec::with_capacity(list.len());
    for (index, stage) in list.iter().enumerate() {
        let object = stage.as_object()
            .filter(|object| object.contains_key("op"))
            .ok_or(format!("stages[{index}]: expected {{'op': …}}"))?;
        let op = object["op"].as_str().unwrap_or("");
        let table = OPERATIONS.iter()
            .find(|(name, _)| *name == op)
            .map(|(_, parameters)| *parameters)
            .ok_or(format!("stages[{index}]: unknown op '{op}' \
                            (known: {})", known_ops()))?;
        let mut packed = [0.0f64; PARAMETERS_PER_STAGE];
        for (slot, (key, default, low, high)) in table.iter().enumerate() {
            let value = match object.get(*key) {
                None => *default,
                Some(value) => value.as_f64().ok_or(format!(
                    "stages[{index}].{key}: expected a number"))?,
            };
            packed[slot] = value.clamp(*low, *high);
        }
        stages.push((op.to_string(), packed));
    }
    Ok(stages)
}

/// (name, author, canonical stages) from a .phoskit file — v3 `load`.
pub fn load(path: &Path) -> Result<Kit, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    let document: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| format!("not a phoskit document ({error})"))?;
    let object = document.as_object()
        .filter(|object| object.contains_key("stages"))
        .ok_or("not a phoskit document")?;
    let version = object.get("phoskit")
        .and_then(|value| value.as_i64()).unwrap_or(1);
    if version > FORMAT_VERSION {
        return Err(format!("phoskit version {version} is newer than \
                            this Phosphor understands"));
    }
    let stem = path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = object.get("name")
        .and_then(|value| value.as_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .unwrap_or(stem);
    let author = object.get("author")
        .and_then(|value| value.as_str())
        .unwrap_or("").to_string();
    Ok(Kit { name, author, stages: canonical_stages(&object["stages"])? })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn repo() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn starter_kits_match_golden_canonical_stages() {
        for kit_name in ["haunt", "heartbeat", "orbit"] {
            let kit = load(&repo().join(
                format!("kits/{kit_name}.phoskit"))).expect("load");
            let golden: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(repo().join(format!(
                    "tests/golden/kits/kitaudio-starter-{kit_name}__sine__48000.json")))
                    .expect("golden")).unwrap();
            let want = golden["kit"]["stages"].as_array().unwrap();
            assert_eq!(kit.stages.len(), want.len(), "{kit_name}");
            for (stage, golden_stage) in kit.stages.iter().zip(want) {
                assert_eq!(stage.0,
                           golden_stage[0].as_str().unwrap(),
                           "{kit_name} op");
                let parameters = golden_stage[1].as_array().unwrap();
                for (slot, parameter) in parameters.iter().enumerate() {
                    assert_eq!(stage.1[slot],
                               parameter.as_f64().unwrap(),
                               "{kit_name} p{slot}");
                }
            }
        }
    }

    #[test]
    fn error_voice_is_short_and_directive() {
        let bad = serde_json::json!([{"op": "sparkle"}]);
        let error = canonical_stages(&bad).unwrap_err();
        assert_eq!(error, "stages[0]: unknown op 'sparkle' (known: \
                   chandelay, matrix, midside, ringmod, rotate, wobble)");
        let bad = serde_json::json!([{"op": "rotate", "hz": "fast"}]);
        assert_eq!(canonical_stages(&bad).unwrap_err(),
                   "stages[0].hz: expected a number");
        assert_eq!(canonical_stages(&serde_json::json!([])).unwrap_err(),
                   "stages: expected a non-empty list");
    }

    #[test]
    fn parameters_clamp_and_default() {
        let stages = canonical_stages(&serde_json::json!(
            [{"op": "ringmod", "hz": 900.0}])).unwrap();
        assert_eq!(stages[0].1, [30.0, 0.2, 0.0, 0.0]);
    }
}
