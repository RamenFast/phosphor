// SPDX-License-Identifier: GPL-3.0-or-later
//! Chrome pass ii — the player: playlist model, both advance state
//! machines (PLAYER-SPEC laws, kept deliberately asymmetric: manual
//! step ALWAYS wraps and ignores repeat; auto-advance obeys repeat
//! with repeat=="one" beating shuffle), seek with the 250 ms debounce,
//! the ⌀ file-vacuum toggle, and the now-playing fade (0.09/33 ms in,
//! hold until call-time+4 s, fade out).
//!
//! Gapless is new in v4: the deterministic next track preloads into
//! the engine; shuffle and repeat-one splice at EOF the old way.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::shell::{Shell, UiAction};
use egui_phosphor::regular as icon;

/// v3 AUDIO_FILE_EXTENSIONS, verbatim (13 entries).
pub const AUDIO_FILE_EXTENSIONS: [&str; 13] = [
    ".mp3", ".flac", ".ogg", ".oga", ".opus", ".wav", ".m4a", ".aac",
    ".wma", ".aif", ".aiff", ".mka", ".phos",
];

pub fn is_audio_path(path: &Path) -> bool {
    let lower = path.to_string_lossy().to_lowercase();
    AUDIO_FILE_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
}

/// v3 format_time: truncates, no hours — 90 min shows "125:07".
pub fn format_time(seconds: f64) -> String {
    let whole = seconds.max(0.0) as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
}

pub struct PlayerState {
    pub playlist: Vec<PathBuf>,
    pub playlist_index: usize,
    pub playing: Option<PathBuf>,
    pub duration: Option<f64>,
    pub paused: bool,
    /// (target seconds, when the slider went still)
    pub seek_debounce: Option<(f64, Instant)>,
    /// slider position while the user owns it
    pub drag_position: Option<f64>,
    pub panel_open: bool,
    // now-playing overlay animation (v3 constants)
    overlay_title: String,
    overlay_subtitle: Option<String>,
    overlay_phase: OverlayPhase,
    overlay_opacity: f32,
    overlay_hold_until: Instant,
    overlay_last_tick: Instant,
}

impl Default for PlayerState {
    fn default() -> PlayerState {
        PlayerState {
            playlist: Vec::new(),
            playlist_index: 0,
            playing: None,
            duration: None,
            paused: false,
            seek_debounce: None,
            drag_position: None,
            panel_open: false,
            overlay_title: String::new(),
            overlay_subtitle: None,
            overlay_phase: OverlayPhase::Off,
            overlay_opacity: 0.0,
            overlay_hold_until: Instant::now(),
            overlay_last_tick: Instant::now(),
        }
    }
}

#[derive(Default, PartialEq)]
enum OverlayPhase {
    #[default]
    Off,
    In,
    Hold,
    Out,
}

impl PlayerState {
    /// v3 flash_now_playing: text swaps instantly, animation restarts
    /// from blank; hold_until is call time + 4 s (fade-in eats into it).
    pub fn flash_now_playing(&mut self, title: &str, subtitle: Option<&str>) {
        self.overlay_title = title.to_string();
        self.overlay_subtitle = subtitle.map(str::to_string);
        self.overlay_phase = OverlayPhase::In;
        self.overlay_opacity = 0.0;
        self.overlay_hold_until = Instant::now() + Duration::from_secs(4);
        self.overlay_last_tick = Instant::now();
    }

    /// Advance the fade (called per frame; v3 ticked at 33 ms — scale
    /// the 0.09 step to the real frame gap).
    pub fn tick_overlay(&mut self) {
        if self.overlay_phase == OverlayPhase::Off {
            return;
        }
        let now = Instant::now();
        let ticks = now.duration_since(self.overlay_last_tick)
            .as_secs_f32() / 0.033;
        self.overlay_last_tick = now;
        match self.overlay_phase {
            OverlayPhase::In => {
                self.overlay_opacity =
                    (self.overlay_opacity + 0.09 * ticks).min(1.0);
                if self.overlay_opacity >= 1.0 {
                    self.overlay_phase = OverlayPhase::Hold;
                }
            }
            OverlayPhase::Hold => {
                if now >= self.overlay_hold_until {
                    self.overlay_phase = OverlayPhase::Out;
                }
            }
            OverlayPhase::Out => {
                self.overlay_opacity -= 0.09 * ticks;
                if self.overlay_opacity <= 0.0 {
                    self.overlay_opacity = 0.0;
                    self.overlay_phase = OverlayPhase::Off;
                }
            }
            OverlayPhase::Off => {}
        }
    }

    pub fn overlay_visible(&self) -> Option<(String, Option<String>, f32)> {
        if self.overlay_phase == OverlayPhase::Off {
            return None;
        }
        Some((self.overlay_title.clone(), self.overlay_subtitle.clone(),
              self.overlay_opacity))
    }
}

/// v3 _build_playlist: every audio file beside the opened one,
/// case-insensitive alphabetical; the opened file is guaranteed in.
pub fn build_playlist(path: &Path) -> (Vec<PathBuf>, usize) {
    let directory = path.parent().unwrap_or(Path::new("."));
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    names.sort_by_key(|name| name.to_lowercase());
    let mut playlist: Vec<PathBuf> = names
        .iter()
        .filter(|name| {
            let lower = name.to_lowercase();
            AUDIO_FILE_EXTENSIONS.iter().any(|ext| lower.ends_with(ext))
        })
        .map(|name| directory.join(name))
        .collect();
    if !playlist.iter().any(|p| p == path) {
        playlist.insert(0, path.to_path_buf());
    }
    let index = playlist.iter().position(|p| p == path).unwrap_or(0);
    (playlist, index)
}

impl Shell {
    pub(crate) fn play_file(&mut self, path: &Path, rebuild_playlist: bool) {
        if rebuild_playlist {
            let (playlist, index) = build_playlist(path);
            self.player.playlist = playlist;
            self.player.playlist_index = index;
        } else if let Some(index) =
            self.player.playlist.iter().position(|p| p == path)
        {
            self.player.playlist_index = index;
        }
        self.engine.stop_capture();
        self.capture_on = false;
        let vacuum = self.settings.vacuum_enabled;
        self.engine.set_volume(cubic_volume(self.settings.playback_volume));
        self.engine.start_file(path, 0.0, false, vacuum);
        self.player.playing = Some(path.to_path_buf());
        self.player.paused = false;
        self.player.duration = None; // TrackStarted fills it
        self.wake_render_loop();
        self.queue_gapless_next();
    }

    /// Deterministic-next preload (albums get true gapless; shuffle
    /// and repeat-one keep the EOF splice).
    pub(crate) fn queue_gapless_next(&mut self) {
        let next = if self.settings.repeat_mode == "one"
            || self.settings.shuffle
        {
            None
        } else if self.player.playlist_index + 1 < self.player.playlist.len() {
            self.player.playlist
                .get(self.player.playlist_index + 1).cloned()
        } else if self.settings.repeat_mode == "all" {
            self.player.playlist.first().cloned()
        } else {
            None
        };
        self.engine.set_next_track(next);
    }

    /// v3 _step_playlist: modulo wrap, repeat ignored, shuffle picks
    /// uniformly among the other indices (both directions).
    pub(crate) fn step_playlist(&mut self, step: i64) {
        if self.player.playlist.is_empty() {
            return;
        }
        let length = self.player.playlist.len();
        let index = if self.settings.shuffle && length > 1 {
            let mut pick = pseudo_random(length - 1);
            if pick >= self.player.playlist_index {
                pick += 1; // uniform over i != current
            }
            pick
        } else {
            (self.player.playlist_index as i64 + step)
                .rem_euclid(length as i64) as usize
        };
        let path = self.player.playlist[index].clone();
        self.play_file(&path, false);
    }

    /// v3 handle_track_finished: exact precedence.
    pub(crate) fn handle_track_finished(&mut self) {
        let finished = self
            .player
            .playing
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let repeat = self.settings.repeat_mode.clone();
        let length = self.player.playlist.len();
        if repeat == "one" && self.player.playlist_index < length {
            let path =
                self.player.playlist[self.player.playlist_index].clone();
            self.play_file(&path, false);
        } else if (self.settings.shuffle && length > 1)
            || (length > 0 && self.player.playlist_index + 1 < length)
        {
            self.step_playlist(1);
        } else if repeat == "all" && length > 0 {
            let path = self.player.playlist[0].clone();
            self.play_file(&path, false);
        } else {
            self.player.playing = None;
            self.player.duration = None;
            self.status_line = format!("finished: {finished}");
        }
    }

    /// The transport row (§4.2) — drawn only while a file is loaded.
    pub(crate) fn ui_transport(&mut self, ui: &mut egui::Ui) {
        let Some(playing) = self.player.playing.clone() else { return };
        ui.horizontal(|ui| {
            if ui.button(icon::SKIP_BACK).on_hover_text("Previous track in folder")
                .clicked()
            {
                self.actions.push(UiAction::PlayerPrevious);
            }
            let play_label = if self.player.paused { icon::PLAY } else { icon::PAUSE };
            if ui.button(play_label)
                .on_hover_text("Play/pause the loaded file").clicked()
            {
                self.actions.push(UiAction::PlayerTogglePause);
            }
            if ui.button(icon::SKIP_FORWARD).on_hover_text("Next track in folder")
                .clicked()
            {
                self.actions.push(UiAction::PlayerNext);
            }
            let mut volume = self.settings.playback_volume;
            if ui.add(egui::Slider::new(&mut volume, 0.0..=1.0)
                      .show_value(false))
                .on_hover_text("Track volume — just this stream, not \
                                the whole system")
                .changed()
            {
                self.settings.playback_volume = volume;
                self.engine.set_volume(cubic_volume(volume));
            }
            // Vacuum is a signature control → carved/dimensional.
            if self.carved_toggle(ui, "\u{2300}",
                self.settings.vacuum_enabled,
                "Vacuum mode — the track plays as light only: nothing \
                 reaches\nthe speakers, the beam sees everything. \
                 (Sound can't cross\na vacuum; a CRT is a vacuum tube.)")
            {
                self.settings.vacuum_enabled = !self.settings.vacuum_enabled;
                self.actions.push(UiAction::PlayerVacuumToggled);
                self.actions.push(UiAction::SaveSettings);
            }
            let mut shuffle = self.settings.shuffle;
            if ui.toggle_value(&mut shuffle, icon::SHUFFLE).clicked() {
                self.settings.shuffle = shuffle;
                self.actions.push(UiAction::SaveSettings);
                self.actions.push(UiAction::GaplessRequeue);
            }
            let repeat_label = match self.settings.repeat_mode.as_str() {
                "all" => icon::REPEAT,
                "one" => icon::REPEAT_ONCE,
                _ => icon::ARROW_RIGHT,
            };
            if ui.button(repeat_label)
                .on_hover_text("Repeat: off → all → one").clicked()
            {
                self.settings.repeat_mode =
                    match self.settings.repeat_mode.as_str() {
                        "off" => "all".into(),
                        "all" => "one".into(),
                        _ => "off".into(),
                    };
                self.actions.push(UiAction::SaveSettings);
                self.actions.push(UiAction::GaplessRequeue);
            }
            let mut panel = self.player.panel_open;
            if ui.toggle_value(&mut panel, icon::LIST)
                .on_hover_text("Playlist (L)").clicked()
            {
                self.player.panel_open = panel;
                self.settings.playlist_panel_open = panel;
            }

            // position + time
            if let Some(duration) = self.player.duration.filter(|d| *d > 0.0) {
                let live_position = self.engine.playback_position_seconds();
                let mut shown = self.player.drag_position
                    .unwrap_or(live_position)
                    .min(duration);
                let slider = ui.add(
                    egui::Slider::new(&mut shown, 0.0..=duration)
                        .show_value(false));
                let response = slider
                    .on_hover_text("Track position — drag to seek");
                if response.dragged() || response.changed() {
                    self.player.drag_position = Some(shown);
                    self.player.seek_debounce =
                        Some((shown, Instant::now()));
                }
                ui.label(format!("{} / {}", format_time(shown),
                                 format_time(duration)));
            }
            ui.label(
                playing.file_name().map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default());
        });
    }

    /// Playlist side panel (key L): click to play, current highlighted.
    pub(crate) fn ui_playlist_panel(&mut self, ctx: &egui::Context) {
        if !self.player.panel_open {
            return;
        }
        egui::SidePanel::left("playlist")
            .default_width(240.0)
            .show(ctx, |ui| {
                ui.heading("Playlist");
                ui.separator();
                let mut clicked: Option<PathBuf> = None;
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (index, path) in
                        self.player.playlist.iter().enumerate()
                    {
                        let name = path.file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();
                        let current =
                            index == self.player.playlist_index
                            && self.player.playing.is_some();
                        if ui.selectable_label(current, name).clicked() {
                            clicked = Some(path.clone());
                        }
                    }
                });
                if let Some(path) = clicked {
                    self.actions.push(UiAction::PlayPath(path));
                }
            });
    }

    /// 250 ms after the slider went still: the real seek (v3 law).
    pub(crate) fn service_seek_debounce(&mut self) {
        let Some((target, at)) = self.player.seek_debounce else { return };
        if at.elapsed() < Duration::from_millis(250) {
            return;
        }
        self.player.seek_debounce = None;
        self.player.drag_position = None;
        let Some(path) = self.player.playing.clone() else { return };
        let was_paused = self.player.paused;
        let vacuum = self.settings.vacuum_enabled;
        self.engine.start_file(&path, target, false, vacuum);
        self.computer.reset(); // no beam line bridging old→new position
        if was_paused {
            self.engine.set_playback_paused(true);
        }
        self.queue_gapless_next();
        self.mpris_seeked(target); // v4 fix: Seeked really emits
        self.wake_render_loop();
    }
}

/// v3 set volume as a pulse percentage; PW stream volume is linear
/// amplitude, and pulse's percent scale is perceptual (cubic).
pub fn cubic_volume(fraction: f32) -> f32 {
    fraction.clamp(0.0, 1.0).powi(3)
}

/// Tiny deterministic-enough pick for shuffle (v3 used random.choice;
/// the distribution matters, the generator does not).
fn pseudo_random(bound: usize) -> usize {
    use std::time::{SystemTime, UNIX_EPOCH};
    if bound == 0 {
        return 0;
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0);
    (nanos ^ (nanos >> 13) ^ (nanos << 7)) % bound
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn time_format_truncates_no_hours() {
        assert_eq!(format_time(0.0), "0:00");
        assert_eq!(format_time(59.9), "0:59");
        assert_eq!(format_time(60.0), "1:00");
        assert_eq!(format_time(90.0 * 60.0 + 7.4), "90:07");
        assert_eq!(format_time(125.0 * 60.0 + 7.0), "125:07");
    }

    #[test]
    fn audio_extensions_match_case_insensitively() {
        assert!(is_audio_path(Path::new("/x/SONG.FLAC")));
        assert!(is_audio_path(Path::new("/x/trace.phos")));
        assert!(!is_audio_path(Path::new("/x/notes.txt")));
    }

    #[test]
    fn playlist_builds_sorted_casefold_and_contains_opened() {
        let dir = std::env::temp_dir().join("phosphor-playlist-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["b.flac", "A.mp3", "c.txt", "D.phos"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let opened = dir.join("b.flac");
        let (playlist, index) = build_playlist(&opened);
        let names: Vec<String> = playlist.iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["A.mp3", "b.flac", "D.phos"]);
        assert_eq!(index, 1);
        // opened file outside the listing still lands at index 0
        let stray = dir.join("zz-not-listed.ogg");
        std::fs::write(&stray, b"x").unwrap();
        std::fs::remove_file(&stray).unwrap();
        let (playlist2, index2) = build_playlist(&stray);
        assert_eq!(playlist2[0], stray);
        assert_eq!(index2, 0);
    }
}
