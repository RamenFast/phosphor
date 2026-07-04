// SPDX-License-Identifier: GPL-3.0-or-later
//! v3's settings file, read with v3's exact semantics: missing file or
//! broken JSON → all defaults; unknown keys ignored; known keys taken
//! leniently. Only the slice the offline pipeline needs lives here —
//! the wave-2 shell will grow the rest.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Settings {
    pub display_mode: String,
    pub gain: f32,
    pub persistence: f32,
    pub beam_energy: f32,
    pub beam_focus: f32,
    pub scope_sample_rate: u32,
    pub theme_name: String,
    pub custom_beam_color: [f32; 3],
    pub custom_grid_color: [f32; 3],
    pub amoled_background: bool,
    pub grid_enabled: bool,
    pub kit_path: Option<String>,
    pub kit_enabled: bool,
    pub gl_supersample: u32,
    pub cairo_resolution: f32,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            display_mode: "xy".into(),
            gain: 1.0,
            persistence: 0.7,
            beam_energy: 8.0,
            beam_focus: 1.6,
            scope_sample_rate: 96000,
            theme_name: "P7 Green".into(),
            custom_beam_color: [0.42, 1.0, 0.55],
            custom_grid_color: [0.35, 1.0, 0.45],
            amoled_background: false,
            grid_enabled: true,
            kit_path: None,
            kit_enabled: false,
            gl_supersample: 1,
            cairo_resolution: 1.0,
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
        take!("display_mode", settings.display_mode,
              |value: &serde_json::Value| value.as_str()
              .map(str::to_string));
        take!("gain", settings.gain, float);
        take!("persistence", settings.persistence, float);
        take!("beam_energy", settings.beam_energy, float);
        take!("beam_focus", settings.beam_focus, float);
        take!("scope_sample_rate", settings.scope_sample_rate,
              |value: &serde_json::Value| value.as_u64()
              .map(|number| number as u32));
        take!("theme_name", settings.theme_name,
              |value: &serde_json::Value| value.as_str()
              .map(str::to_string));
        take!("custom_beam_color", settings.custom_beam_color, color);
        take!("custom_grid_color", settings.custom_grid_color, color);
        take!("amoled_background", settings.amoled_background,
              serde_json::Value::as_bool);
        take!("grid_enabled", settings.grid_enabled,
              serde_json::Value::as_bool);
        take!("kit_path", settings.kit_path,
              |value: &serde_json::Value| match value {
                  serde_json::Value::Null => Some(None),
                  other => other.as_str().map(|s| Some(s.to_string())),
              });
        take!("kit_enabled", settings.kit_enabled,
              serde_json::Value::as_bool);
        take!("gl_supersample", settings.gl_supersample,
              |value: &serde_json::Value| value.as_u64()
              .map(|number| (number as u32).clamp(1, 2)));
        take!("cairo_resolution", settings.cairo_resolution, float);
        settings
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
        assert_eq!(settings.gl_supersample, 2, "clamped to v3's 1..2");
        assert_eq!(settings.gain, 1.0, "untouched default");
    }
}
