// SPDX-License-Identifier: GPL-3.0-or-later
//! The .phos signal postcard: a fixed 256-byte header, then raw s16le
//! stereo at the header's rate.
//!
//! Header bytes: `PHOSC1` + JSON (UTF-8, Python `json.dumps` shape:
//! `{"key": value, …}` with `", "` / `": "` separators, non-ASCII kept
//! raw), space-padded to byte 255, byte 256 is `\n`. Writing trims the
//! free-text fields (title, credit, source) through the ladder
//! 80 → 48 → 24 → 8 → 0 **characters** until the record fits — a
//! shared .phos never fails over a long album title. Fields keep their
//! original order on rewrite (`export_postcard` law), which is why the
//! golden round-trip can demand byte equality.

use std::fmt;
use std::io::Read;

pub const MAGIC: &[u8; 6] = b"PHOSC1";
pub const HEADER_BYTES: usize = 256;
pub const INT16_SCALE: f32 = 32767.0;
const TRIM_LADDER: [usize; 5] = [80, 48, 24, 8, 0];
const TRIMMED_KEYS: [&str; 3] = ["title", "credit", "source"];

#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Int(i64),
    Text(String),
}

impl Field {
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Field::Int(value) => Some(*value),
            Field::Text(_) => None,
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Field::Text(value) => Some(value),
            Field::Int(_) => None,
        }
    }
}

/// Parsed .phos header: fields in file order (order is part of the
/// byte-exact rewrite contract).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Header {
    pub fields: Vec<(String, Field)>,
}

#[derive(Debug)]
pub struct PhosError(pub String);

impl fmt::Display for PhosError {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

impl std::error::Error for PhosError {}

impl Header {
    pub fn get(&self, key: &str) -> Option<&Field> {
        self.fields.iter()
            .find(|(name, _)| name == key)
            .map(|(_, field)| field)
    }

    pub fn set_text(&mut self, key: &str, value: &str) {
        if let Some(slot) = self.fields.iter_mut()
            .find(|(name, _)| name == key) {
            slot.1 = Field::Text(value.to_string());
        } else {
            self.fields.push((key.to_string(),
                              Field::Text(value.to_string())));
        }
    }

    pub fn set_int(&mut self, key: &str, value: i64) {
        if let Some(slot) = self.fields.iter_mut()
            .find(|(name, _)| name == key) {
            slot.1 = Field::Int(value);
        } else {
            self.fields.push((key.to_string(), Field::Int(value)));
        }
    }

    pub fn rate(&self) -> Option<u32> {
        self.get("rate")?.as_int().map(|value| value as u32)
    }

    pub fn frames(&self) -> Option<u64> {
        self.get("frames")?.as_int().map(|value| value as u64)
    }
}

/// Parse a 256-byte header record. `None` when the magic is absent
/// (not a .phos file); `Err` when it claims to be one but is broken.
pub fn parse_header(header: &[u8]) -> Result<Option<Header>, PhosError> {
    if header.len() < HEADER_BYTES || !header.starts_with(MAGIC) {
        return Ok(None);
    }
    let body = &header[MAGIC.len()..HEADER_BYTES];
    let text = std::str::from_utf8(body)
        .map_err(|_| PhosError("phos header is not UTF-8".into()))?
        .trim();
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| PhosError(format!("phos header JSON: {error}")))?;
    let map = value.as_object()
        .ok_or_else(|| PhosError("phos header: expected an object".into()))?;
    let mut fields = Vec::with_capacity(map.len());
    for (key, value) in map {
        let field = match value {
            serde_json::Value::String(text) => Field::Text(text.clone()),
            serde_json::Value::Number(number) => Field::Int(
                number.as_i64().ok_or_else(|| PhosError(format!(
                    "phos header {key}: expected an integer")))?),
            _ => return Err(PhosError(format!(
                "phos header {key}: expected string or integer"))),
        };
        fields.push((key.clone(), field));
    }
    Ok(Some(Header { fields }))
}

/// Read the header of a file at `path`; `Ok(None)` when it isn't .phos.
pub fn read_header(path: &std::path::Path)
                   -> Result<Option<Header>, PhosError> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| PhosError(format!("{}: {error}",
                                           path.display())))?;
    let mut header = [0u8; HEADER_BYTES];
    match file.read_exact(&mut header) {
        Ok(()) => parse_header(&header),
        Err(_) => Ok(None),             // shorter than a header: not phos
    }
}

/// Python-json.dumps-shaped serialization: `", "` / `": "` separators,
/// raw non-ASCII, control characters escaped the way Python does.
fn dumps(fields: &[(String, Field)]) -> String {
    let mut out = String::from("{");
    for (index, (key, field)) in fields.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        push_json_string(&mut out, key);
        out.push_str(": ");
        match field {
            Field::Int(value) => out.push_str(&value.to_string()),
            Field::Text(value) => push_json_string(&mut out, value),
        }
    }
    out.push('}');
    out
}

fn push_json_string(out: &mut String, text: &str) {
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            control if (control as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", control as u32));
            }
            plain => out.push(plain),
        }
    }
    out.push('"');
}

/// The fixed 256-byte record for `fields`, trimming free text through
/// the ladder until it fits (v3 `pack_header` law, byte-compatible).
pub fn pack_header(fields: &[(String, Field)])
                   -> Result<[u8; HEADER_BYTES], PhosError> {
    for keep in TRIM_LADDER {
        let mut candidate: Vec<(String, Field)> = Vec::new();
        for (key, field) in fields {
            if TRIMMED_KEYS.contains(&key.as_str()) {
                let text = match field {
                    Field::Text(text) => text.clone(),
                    Field::Int(value) => value.to_string(),
                };
                let trimmed: String = text.chars().take(keep).collect();
                if trimmed.is_empty() {
                    continue;               // Python pops empty fields
                }
                candidate.push((key.clone(), Field::Text(trimmed)));
            } else {
                candidate.push((key.clone(), field.clone()));
            }
        }
        let mut encoded = Vec::from(MAGIC.as_slice());
        encoded.extend_from_slice(dumps(&candidate).as_bytes());
        if encoded.len() < HEADER_BYTES {
            let mut record = [b' '; HEADER_BYTES];
            record[..encoded.len()].copy_from_slice(&encoded);
            record[HEADER_BYTES - 1] = b'\n';
            return Ok(record);
        }
    }
    Err(PhosError("phos header does not fit".into()))
}

/// s16le stereo payload → interleaved f32 (`s16 / 32767.0`).
pub fn decode_payload(payload: &[u8]) -> Vec<f32> {
    payload.chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]) as f32
             / INT16_SCALE)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;

    fn golden_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/golden/phos")
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn golden_phos_parse_and_byte_exact_repack() {
        for name in ["plain", "unicode", "overlong"] {
            let raw = std::fs::read(golden_dir()
                                    .join(format!("{name}.phos")))
                .expect("golden phos");
            let expect: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(
                    golden_dir().join(format!("{name}.json")))
                    .expect("golden json")).expect("json");

            let header = parse_header(&raw).expect("parse")
                .expect("is phos");
            let parsed = &expect["header_parsed"];
            for (key, value) in parsed.as_object().unwrap() {
                let field = header.get(key)
                    .unwrap_or_else(|| panic!("{name}: missing {key}"));
                match value {
                    serde_json::Value::Number(number) => assert_eq!(
                        field.as_int(), number.as_i64(),
                        "{name}.{key}"),
                    serde_json::Value::String(text) => assert_eq!(
                        field.as_text(), Some(text.as_str()),
                        "{name}.{key}"),
                    _ => panic!("unexpected golden type"),
                }
            }

            // byte-exact rewrite: parse -> pack == original record
            let repacked = pack_header(&header.fields).expect("pack");
            assert_eq!(&repacked[..], &raw[..HEADER_BYTES],
                       "{name}: header repack diverged");

            // payload contract
            let payload = &raw[HEADER_BYTES..];
            assert_eq!(hex(&Sha256::digest(payload)),
                       expect["payload_sha256"].as_str().unwrap(),
                       "{name}: payload hash");
            let decoded = decode_payload(
                &payload[..32.min(payload.len())]);
            let want: Vec<f32> = expect["first_decoded_f32"]
                .as_array().unwrap().iter()
                .map(|value| value.as_f64().unwrap() as f32).collect();
            assert_eq!(&decoded[..want.len()], &want[..],
                       "{name}: decode contract");
            assert_eq!(header.frames().unwrap() as usize,
                       payload.len() / 4, "{name}: frame count");
        }
    }

    #[test]
    fn fit_trim_ladder_pins_at_24() {
        let expect: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(golden_dir().join("overlong.json"))
                .unwrap()).unwrap();
        let requested = expect["requested"].as_object().unwrap();
        let mut fields = vec![
            ("rate".to_string(), Field::Int(48000)),
            ("frames".to_string(), Field::Int(4800)),
        ];
        for key in ["source", "title", "credit"] {
            fields.push((key.to_string(), Field::Text(
                requested[key].as_str().unwrap().to_string())));
        }
        let packed = pack_header(&fields).unwrap();
        let header = parse_header(&packed).unwrap().unwrap();
        for key in ["title", "credit", "source"] {
            assert_eq!(header.get(key).unwrap().as_text().unwrap()
                       .chars().count(), 24, "{key} should pin at 24");
        }
    }
}
