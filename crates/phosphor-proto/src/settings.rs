// SPDX-License-Identifier: GPL-3.0-or-later
//! v3's settings file, read with v3's exact semantics: missing file or
//! broken JSON → all defaults; unknown keys ignored on read but
//! PRESERVED on write (v3 keeps running against the same file during
//! the migration — v4 must never eat a key it doesn't own).
//!
//! Full key set per the v3 table (UI-SPEC §3.2): defaults verbatim,
//! `scope_sample_rate` re-validated against v3's VALID_SCOPE_RATES,
//! `gl_supersample` clamped to v3's 1..=3, `max_fps` to -1..=1000
//! (-1 = Uncapped, a v4 addition; v3 clamps it to 0 = Monitor).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const VALID_SCOPE_RATES: [u32; 4] = [48_000, 96_000, 192_000, 384_000];

#[derive(Clone, Debug)]
pub struct Settings {
    // window & mini
    pub window_width: i64,
    pub window_height: i64,
    pub window_x: Option<i64>,
    pub window_y: Option<i64>,
    pub start_in_mini: bool,
    pub mini_size: i64,
    pub mini_x: Option<i64>,
    pub mini_y: Option<i64>,
    // signal
    pub display_mode: String,
    pub gain: f32,
    pub auto_gain: bool,
    pub persistence: f32,
    pub beam_energy: f32,
    pub beam_focus: f32,
    pub scope_sample_rate: u32,
    pub precompute_enabled: bool,
    pub compose_frequency_hz: f32,
    // look
    pub theme_name: String,
    pub custom_beam_color: [f32; 3],
    pub custom_grid_color: [f32; 3],
    pub amoled_background: bool,
    pub grid_enabled: bool,
    pub scope_glass: bool,
    pub glass_tint: f32,
    pub glass_tints: BTreeMap<String, f32>,
    pub ui_style: String,
    // kit
    pub kit_path: Option<String>,
    pub kit_enabled: bool,
    // renderer
    pub renderer: String,
    pub gl_supersample: u32,
    pub cairo_resolution: f32,
    pub show_pin_button: bool,
    pub show_fps: bool,
    pub max_fps: i64,
    // capture & player
    pub target_id: Option<String>,
    pub pinned: bool,
    pub show_now_playing: bool,
    pub playback_volume: f32,
    pub shuffle: bool,
    pub repeat_mode: String,
    pub playlist_panel_open: bool,
    pub postcard_credit: String,
    pub vacuum_enabled: bool,
    /// Every key from the file we do not own — written back verbatim
    /// so a v3 running against the same file never loses anything.
    pub unknown: serde_json::Map<String, serde_json::Value>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            window_width: 980,
            window_height: 640,
            window_x: None,
            window_y: None,
            start_in_mini: false,
            mini_size: 280,
            mini_x: None,
            mini_y: None,
            display_mode: "xy".into(),
            gain: 1.0,
            auto_gain: false,
            persistence: 0.7,
            beam_energy: 8.0,
            beam_focus: 1.6,
            scope_sample_rate: 96_000,
            precompute_enabled: false,
            compose_frequency_hz: 80.0,
            theme_name: "P7 Green".into(),
            custom_beam_color: [0.42, 1.0, 0.55],
            custom_grid_color: [0.35, 1.0, 0.45],
            amoled_background: false,
            grid_enabled: true,
            scope_glass: false,
            glass_tint: 0.5,
            glass_tints: BTreeMap::new(),
            ui_style: "dark".into(),
            kit_path: None,
            kit_enabled: false,
            renderer: "gl".into(),
            gl_supersample: 1,
            cairo_resolution: 1.0,
            show_pin_button: true,
            show_fps: false,
            max_fps: 0,
            target_id: None,
            pinned: false,
            show_now_playing: true,
            playback_volume: 1.0,
            shuffle: false,
            repeat_mode: "off".into(),
            playlist_panel_open: false,
            postcard_credit: String::new(),
            vacuum_enabled: false,
            unknown: serde_json::Map::new(),
        }
    }
}

pub fn default_path() -> PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config/phosphor/settings.json")
}

fn color(value: &serde_json::Value) -> Option<[f32; 3]> {
    let list = value.as_array()?;
    if list.len() != 3 {
        return None;
    }
    Some([list[0].as_f64()? as f32,
          list[1].as_f64()? as f32,
          list[2].as_f64()? as f32])
}

fn optional_string(value: &serde_json::Value) -> Option<Option<String>> {
    match value {
        serde_json::Value::Null => Some(None),
        other => other.as_str().map(|s| Some(s.to_string())),
    }
}

fn optional_int(value: &serde_json::Value) -> Option<Option<i64>> {
    match value {
        serde_json::Value::Null => Some(None),
        other => other.as_i64().map(Some),
    }
}

/// The keys v4 owns (everything else round-trips through `unknown`).
const OWNED_KEYS: &[&str] = &[
    "window_width", "window_height", "window_x", "window_y",
    "start_in_mini", "mini_size", "mini_x", "mini_y",
    "display_mode", "gain", "auto_gain", "persistence", "beam_energy",
    "beam_focus", "scope_sample_rate", "precompute_enabled",
    "compose_frequency_hz", "theme_name", "custom_beam_color",
    "custom_grid_color", "amoled_background", "grid_enabled",
    "scope_glass", "glass_tint", "glass_tints", "ui_style", "kit_path",
    "kit_enabled", "renderer", "gl_supersample", "cairo_resolution",
    "show_pin_button", "show_fps", "max_fps", "target_id", "pinned",
    "show_now_playing", "playback_volume", "shuffle", "repeat_mode",
    "playlist_panel_open", "postcard_credit", "vacuum_enabled",
];

impl Settings {
    pub fn load(path: &Path) -> Settings {
        let mut settings = Settings::default();
        let Ok(text) = std::fs::read_to_string(path) else {
            return settings;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
        else {
            return settings;
        };
        let Some(map) = value.as_object() else { return settings };

        macro_rules! take {
            ($key:literal, $slot:expr, $convert:expr) => {
                if let Some(value) = map.get($key) {
                    if let Some(converted) = $convert(value) {
                        $slot = converted;
                    }
                }
            };
        }
        let float = |value: &serde_json::Value| {
            value.as_f64().map(|number| number as f32)
        };
        let integer = serde_json::Value::as_i64;
        let string = |value: &serde_json::Value| {
            value.as_str().map(str::to_string)
        };

        take!("window_width", settings.window_width, integer);
        take!("window_height", settings.window_height, integer);
        take!("window_x", settings.window_x, optional_int);
        take!("window_y", settings.window_y, optional_int);
        take!("start_in_mini", settings.start_in_mini,
              serde_json::Value::as_bool);
        take!("mini_size", settings.mini_size, integer);
        take!("mini_x", settings.mini_x, optional_int);
        take!("mini_y", settings.mini_y, optional_int);
        take!("display_mode", settings.display_mode, string);
        take!("gain", settings.gain, float);
        take!("auto_gain", settings.auto_gain,
              serde_json::Value::as_bool);
        take!("persistence", settings.persistence, float);
        take!("beam_energy", settings.beam_energy, float);
        take!("beam_focus", settings.beam_focus, float);
        take!("scope_sample_rate", settings.scope_sample_rate,
              |value: &serde_json::Value| value.as_u64()
              .map(|number| number as u32));
        take!("precompute_enabled", settings.precompute_enabled,
              serde_json::Value::as_bool);
        take!("compose_frequency_hz", settings.compose_frequency_hz,
              float);
        take!("theme_name", settings.theme_name, string);
        take!("custom_beam_color", settings.custom_beam_color, color);
        take!("custom_grid_color", settings.custom_grid_color, color);
        take!("amoled_background", settings.amoled_background,
              serde_json::Value::as_bool);
        take!("grid_enabled", settings.grid_enabled,
              serde_json::Value::as_bool);
        take!("scope_glass", settings.scope_glass,
              serde_json::Value::as_bool);
        take!("glass_tint", settings.glass_tint, float);
        if let Some(map) = map.get("glass_tints")
            .and_then(|value| value.as_object())
        {
            settings.glass_tints = map
                .iter()
                .filter_map(|(key, value)| {
                    value.as_f64().map(|v| (key.clone(), v as f32))
                })
                .collect();
        }
        take!("ui_style", settings.ui_style, string);
        take!("kit_path", settings.kit_path, optional_string);
        take!("kit_enabled", settings.kit_enabled,
              serde_json::Value::as_bool);
        take!("renderer", settings.renderer, string);
        take!("gl_supersample", settings.gl_supersample,
              |value: &serde_json::Value| value.as_u64()
              .map(|number| (number as u32).clamp(1, 3)));
        take!("cairo_resolution", settings.cairo_resolution, float);
        take!("show_pin_button", settings.show_pin_button,
              serde_json::Value::as_bool);
        take!("show_fps", settings.show_fps,
              serde_json::Value::as_bool);
        take!("max_fps", settings.max_fps,
              |value: &serde_json::Value| value.as_i64()
              .map(|number| number.clamp(-1, 1000))); // -1 = Uncapped (v4)
        take!("target_id", settings.target_id, optional_string);
        take!("pinned", settings.pinned, serde_json::Value::as_bool);
        take!("show_now_playing", settings.show_now_playing,
              serde_json::Value::as_bool);
        take!("playback_volume", settings.playback_volume, float);
        take!("shuffle", settings.shuffle, serde_json::Value::as_bool);
        take!("repeat_mode", settings.repeat_mode, string);
        take!("playlist_panel_open", settings.playlist_panel_open,
              serde_json::Value::as_bool);
        take!("postcard_credit", settings.postcard_credit, string);
        take!("vacuum_enabled", settings.vacuum_enabled,
              serde_json::Value::as_bool);

        // v3 re-validated the rate in phosphor.py, not the settings
        // module; v4 has one loader so the law lives here.
        if !VALID_SCOPE_RATES.contains(&settings.scope_sample_rate) {
            settings.scope_sample_rate = 96_000;
        }

        settings.unknown = map
            .iter()
            .filter(|(key, _)| !OWNED_KEYS.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        settings
    }

    /// Serialize every owned key + all preserved unknown keys.
    pub fn to_json(&self) -> serde_json::Value {
        let mut map = self.unknown.clone();
        let f = |x: f32| {
            serde_json::Number::from_f64(x as f64)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null)
        };
        let color = |c: [f32; 3]| {
            serde_json::Value::Array(vec![f(c[0]), f(c[1]), f(c[2])])
        };
        let opt_string = |v: &Option<String>| match v {
            Some(s) => serde_json::Value::String(s.clone()),
            None => serde_json::Value::Null,
        };
        let opt_int = |v: &Option<i64>| match v {
            Some(n) => serde_json::Value::from(*n),
            None => serde_json::Value::Null,
        };
        map.insert("window_width".into(), self.window_width.into());
        map.insert("window_height".into(), self.window_height.into());
        map.insert("window_x".into(), opt_int(&self.window_x));
        map.insert("window_y".into(), opt_int(&self.window_y));
        map.insert("start_in_mini".into(), self.start_in_mini.into());
        map.insert("mini_size".into(), self.mini_size.into());
        map.insert("mini_x".into(), opt_int(&self.mini_x));
        map.insert("mini_y".into(), opt_int(&self.mini_y));
        map.insert("display_mode".into(),
                   self.display_mode.clone().into());
        map.insert("gain".into(), f(self.gain));
        map.insert("auto_gain".into(), self.auto_gain.into());
        map.insert("persistence".into(), f(self.persistence));
        map.insert("beam_energy".into(), f(self.beam_energy));
        map.insert("beam_focus".into(), f(self.beam_focus));
        map.insert("scope_sample_rate".into(),
                   self.scope_sample_rate.into());
        map.insert("precompute_enabled".into(),
                   self.precompute_enabled.into());
        map.insert("compose_frequency_hz".into(),
                   f(self.compose_frequency_hz));
        map.insert("theme_name".into(), self.theme_name.clone().into());
        map.insert("custom_beam_color".into(),
                   color(self.custom_beam_color));
        map.insert("custom_grid_color".into(),
                   color(self.custom_grid_color));
        map.insert("amoled_background".into(),
                   self.amoled_background.into());
        map.insert("grid_enabled".into(), self.grid_enabled.into());
        map.insert("scope_glass".into(), self.scope_glass.into());
        map.insert("glass_tint".into(), f(self.glass_tint));
        map.insert(
            "glass_tints".into(),
            serde_json::Value::Object(
                self.glass_tints
                    .iter()
                    .map(|(key, value)| (key.clone(), f(*value)))
                    .collect(),
            ),
        );
        map.insert("ui_style".into(), self.ui_style.clone().into());
        map.insert("kit_path".into(), opt_string(&self.kit_path));
        map.insert("kit_enabled".into(), self.kit_enabled.into());
        map.insert("renderer".into(), self.renderer.clone().into());
        map.insert("gl_supersample".into(), self.gl_supersample.into());
        map.insert("cairo_resolution".into(), f(self.cairo_resolution));
        map.insert("show_pin_button".into(),
                   self.show_pin_button.into());
        map.insert("show_fps".into(), self.show_fps.into());
        map.insert("max_fps".into(), self.max_fps.into());
        map.insert("target_id".into(), opt_string(&self.target_id));
        map.insert("pinned".into(), self.pinned.into());
        map.insert("show_now_playing".into(),
                   self.show_now_playing.into());
        map.insert("playback_volume".into(), f(self.playback_volume));
        map.insert("shuffle".into(), self.shuffle.into());
        map.insert("repeat_mode".into(),
                   self.repeat_mode.clone().into());
        map.insert("playlist_panel_open".into(),
                   self.playlist_panel_open.into());
        map.insert("postcard_credit".into(),
                   self.postcard_credit.clone().into());
        map.insert("vacuum_enabled".into(), self.vacuum_enabled.into());
        serde_json::Value::Object(map)
    }

    /// Write back (v3 wrote indent=2 JSON; the directory is created
    /// like v3's os.makedirs(exist_ok=True)).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(&self.to_json())
            .unwrap_or_else(|_| "{}".into());
        std::fs::write(path, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_v3_defaults() {
        let settings = Settings::load(Path::new("/nonexistent/nope"));
        assert_eq!(settings.display_mode, "xy");
        assert_eq!(settings.scope_sample_rate, 96000);
        assert_eq!(settings.theme_name, "P7 Green");
        assert!(settings.grid_enabled);
        assert_eq!(settings.window_width, 980);
        assert_eq!(settings.renderer, "gl");
        assert_eq!(settings.max_fps, 0);
        assert_eq!(settings.repeat_mode, "off");
    }

    #[test]
    fn unknown_keys_ignored_known_keys_taken() {
        let directory = std::env::temp_dir()
            .join("phosphor-proto-settings-test");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        std::fs::write(&path, r#"{"display_mode": "helix",
            "scope_sample_rate": 384000, "martian_field": 9,
            "amoled_background": true, "gl_supersample": 7}"#).unwrap();
        let settings = Settings::load(&path);
        assert_eq!(settings.display_mode, "helix");
        assert_eq!(settings.scope_sample_rate, 384000);
        assert!(settings.amoled_background);
        assert_eq!(settings.gl_supersample, 3, "clamped to v3's 1..3");
        assert_eq!(settings.gain, 1.0, "untouched default");
        assert_eq!(settings.unknown.get("martian_field"),
                   Some(&serde_json::Value::from(9)));
    }

    #[test]
    fn invalid_scope_rate_falls_back() {
        let directory = std::env::temp_dir()
            .join("phosphor-proto-settings-rate-test");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        std::fs::write(&path, r#"{"scope_sample_rate": 44100}"#).unwrap();
        assert_eq!(Settings::load(&path).scope_sample_rate, 96000);
    }

    /// The write-back correctness item (wave-2 plan): round-trip a
    /// settings file — every key v4 does not own must survive verbatim,
    /// owned keys must reflect the struct.
    #[test]
    fn write_back_preserves_foreign_keys() {
        let directory = std::env::temp_dir()
            .join("phosphor-proto-settings-roundtrip");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        let original = r#"{
            "display_mode": "ring",
            "gain": 2.5,
            "scope_sample_rate": 192000,
            "glass_tints": {"aero": 0.42, "black": 0.8},
            "window_x": 120,
            "kit_path": null,
            "some_v5_experiment": {"nested": [1, 2, 3]},
            "another_unknown": "keep me"
        }"#;
        std::fs::write(&path, original).unwrap();
        let mut settings = Settings::load(&path);
        settings.gain = 3.0; // the one legitimate change
        settings.save(&path).unwrap();

        let before: serde_json::Value =
            serde_json::from_str(original).unwrap();
        let after: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&path).unwrap()).unwrap();
        // foreign keys byte-identical
        assert_eq!(after.get("some_v5_experiment"),
                   before.get("some_v5_experiment"));
        assert_eq!(after.get("another_unknown"),
                   before.get("another_unknown"));
        // owned keys reflect struct state
        assert_eq!(after.get("gain"),
                   Some(&serde_json::Value::from(3.0)));
        assert_eq!(after.get("display_mode"),
                   Some(&serde_json::Value::from("ring")));
        assert_eq!(after.get("glass_tints").unwrap()
                   .get("aero").unwrap().as_f64().unwrap(),
                   0.41999998688697815_f64.min(0.42001), "f32 round-trip");
        assert_eq!(after.get("window_x"),
                   Some(&serde_json::Value::from(120)));
        // defaults materialize for keys absent before (v3 did the same
        // on its first save)
        assert_eq!(after.get("renderer"),
                   Some(&serde_json::Value::from("gl")));
    }
}
