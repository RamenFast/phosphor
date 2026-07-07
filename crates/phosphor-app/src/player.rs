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
        // a track replaces the compose loop; no explicit stop needed —
        // start_file below stops the previous stream (v3 law)
        self.exit_compose(false);
        if rebuild_playlist {
            let (playlist, index) = build_playlist(path);
            self.player.playlist = playlist;
            self.player.playlist_index = index;
        } else if let Some(index) =
            self.player.playlist.iter().position(|p| p == path)
        {
            self.player.playlist_index = index;
        } else {
            // the path came from OUTSIDE the list (ctl open, MPRIS
            // OpenUri, file-manager forward): the file-dialog law
            // applies — folder siblings become the playlist. Without
            // this the playlist stayed EMPTY on external opens: bare
            // panel, dead next/previous, no gapless (BUGLOG #3).
            // Drag-drop is unaffected — it seeds its single-track
            // list before calling here, so its path is always found.
            let (playlist, index) = build_playlist(path);
            self.player.playlist = playlist;
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
        self.sync_beam_source(None);
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
            self.sync_beam_source(None);
        }
    }

    /// The transport row (§4.2) follows THE BEAM: while capture scopes
    /// a linked player, its controls drive that player — even if a
    /// local track sits loaded-and-paused underneath (Ben's patch:
    /// "intelligently recognize what is playing"). The built-in row
    /// renders when the player session owns the beam.
    pub(crate) fn ui_transport(&mut self, ui: &mut egui::Ui) {
        if self.linked_external_player().is_some() {
            self.ui_external_transport(ui);
            return;
        }
        let Some(playing) = self.player.playing.clone() else {
            return;
        };
        ui.horizontal(|ui| {
            // cover-art thumbnail (hairline-framed), when the playing
            // track carries embedded art
            if let Some((source, texture)) = &self.cover_texture
                && *source == playing
            {
                let rect = ui.image(egui::load::SizedTexture::new(
                    texture.id(), egui::vec2(22.0, 22.0))).rect;
                ui.painter().rect_stroke(rect, 0.0,
                    egui::Stroke::new(1.0, self.active_palette.line),
                    egui::StrokeKind::Inside);
            }
            if self.bevel_button(ui, icon::SKIP_BACK,
                                 "Previous track in folder").clicked()
            {
                self.actions.push(UiAction::PlayerPrevious);
            }
            let play_label = if self.player.paused { icon::PLAY } else { icon::PAUSE };
            if self.bevel_button(ui, play_label,
                                 "Play/pause the loaded file").clicked()
            {
                self.actions.push(UiAction::PlayerTogglePause);
            }
            if self.bevel_button(ui, icon::SKIP_FORWARD,
                                 "Next track in folder").clicked()
            {
                self.actions.push(UiAction::PlayerNext);
            }
            let mut volume = self.settings.playback_volume;
            if ui.add(egui::Slider::new(&mut volume, 0.0..=1.0)
                      .show_value(false)
                      .trailing_fill(true))
                .on_hover_text("Track volume — just this stream, not \
                                the whole system")
                .changed()
            {
                self.settings.playback_volume = volume;
                self.engine.set_volume(cubic_volume(volume));
            }
            // the readout the volume never had (slider audit)
            ui.label(egui::RichText::new(
                format!("{:>3.0} %", volume * 100.0)).monospace());
            // Vacuum is a signature control → carved/dimensional.
            // Icon-font glyph (U+2300 was a tofu candidate).
            if self.carved_toggle(ui, icon::PROHIBIT,
                self.settings.vacuum_enabled,
                "Vacuum — light only: the track plays full-tilt into \
                 the void,\nnothing reaches the speakers, the beam \
                 sees everything.\n(Sound can't cross a vacuum; a CRT \
                 is a vacuum tube.)")
            {
                self.settings.vacuum_enabled = !self.settings.vacuum_enabled;
                self.actions.push(UiAction::PlayerVacuumToggled);
                self.actions.push(UiAction::SaveSettings);
            }
            if self.bevel_toggle(ui, icon::SHUFFLE, self.settings.shuffle,
                                 "Shuffle")
            {
                self.settings.shuffle = !self.settings.shuffle;
                self.actions.push(UiAction::SaveSettings);
                self.actions.push(UiAction::GaplessRequeue);
            }
            let repeat_label = match self.settings.repeat_mode.as_str() {
                "all" => icon::REPEAT,
                "one" => icon::REPEAT_ONCE,
                _ => icon::ARROW_RIGHT,
            };
            if self.bevel_toggle(ui, repeat_label,
                                 self.settings.repeat_mode != "off",
                                 "Repeat: off → all → one")
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
            if self.bevel_toggle(ui, icon::LIST, self.player.panel_open,
                                 "Playlist (L)")
            {
                self.player.panel_open = !self.player.panel_open;
                self.settings.playlist_panel_open = self.player.panel_open;
            }

            // position + time
            if let Some(duration) = self.player.duration.filter(|d| *d > 0.0) {
                let live_position = self.engine.playback_position_seconds();
                let mut shown = self.player.drag_position
                    .unwrap_or(live_position)
                    .min(duration);
                let slider = ui.add(
                    egui::Slider::new(&mut shown, 0.0..=duration)
                        .show_value(false)
                        .trailing_fill(true));
                let response = slider
                    .on_hover_text("Track position — drag to seek");
                if response.dragged() || response.changed() {
                    self.player.drag_position = Some(shown);
                    self.player.seek_debounce =
                        Some((shown, Instant::now()));
                }
                // time is DATA → mono (the readability audit)
                ui.label(egui::RichText::new(
                    format!("{} / {}", format_time(shown),
                            format_time(duration))).monospace());
            }
            ui.label(
                playing.file_name().map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default());
        });
    }

    /// The external transport: drives the MPRIS player the beam is
    /// scoping (Spotify, a browser…). Drawn only while capture is the
    /// source and a matching player exists.
    fn ui_external_transport(&mut self, ui: &mut egui::Ui) {
        let Some(player) = self.linked_external_player() else { return };
        let Some(client) = &self.mpris_client else { return };
        let commands = client.commands.clone();
        ui.horizontal(|ui| {
            ui.add_enabled_ui(player.can_control, |ui| {
                if self.bevel_button(ui, icon::SKIP_BACK,
                                     "Previous track").clicked()
                {
                    let _ = commands.send(
                        crate::mpris_client::ClientCommand::Previous(
                            player.bus_name.clone()));
                }
                let glyph = if player.status == "Playing" {
                    icon::PAUSE
                } else {
                    icon::PLAY
                };
                if self.bevel_button(ui, glyph, "Play/pause").clicked() {
                    let _ = commands.send(
                        crate::mpris_client::ClientCommand::PlayPause(
                            player.bus_name.clone()));
                }
                if self.bevel_button(ui, icon::SKIP_FORWARD,
                                     "Next track").clicked()
                {
                    let _ = commands.send(
                        crate::mpris_client::ClientCommand::Next(
                            player.bus_name.clone()));
                }
            });
            let what = match (&player.title, &player.artist) {
                (Some(title), Some(artist)) =>
                    format!("{artist} — {title}"),
                (Some(title), None) => title.clone(),
                _ => String::new(),
            };
            if !what.is_empty() {
                ui.label(what);
            }
            ui.label(egui::RichText::new(
                format!("via {}", player.identity))
                .small().color(self.active_palette.muted))
                .on_hover_text(
                    "The beam is scoping this player, so the \
                     transport drives it (MPRIS)");
        });
    }

    /// One slide, both shapes (docked + mini's slide-over) — matches
    /// the house animation_time neighborhood.
    pub(crate) const PANE_SLIDE_SECONDS: f32 = 0.15;

    /// Playlist pane (key L): click to play, current highlighted.
    /// ONE shape in every view — it slides in from the LEFT (Ben:
    /// "the L menu appears consistently sliding in from the left on
    /// all views"). Normal + fullscreen dock it (the scope yields the
    /// strip); mini can't afford to dock a 200–520 px square, so it
    /// gets a left-anchored slide-OVER wearing the same pane clothes,
    /// riding the same slide. L works EVERYWHERE.
    pub(crate) fn ui_playlist_panel(&mut self, ctx: &egui::Context) {
        let open = self.player.panel_open;
        if self.is_mini {
            let slide = ctx.animate_bool_with_time(
                egui::Id::new("mini-playlist-slide"), open,
                Self::PANE_SLIDE_SECONDS);
            if slide <= 0.0 {
                return;
            }
            let content = ctx.content_rect();
            let width = (content.width() * 0.72).min(200.0);
            let offset_x = -(1.0 - slide) * width;
            let panel_fill = ctx.style().visuals.panel_fill;
            let edge = self.active_palette.line_strong;
            egui::Area::new(egui::Id::new("mini-playlist"))
                .anchor(egui::Align2::LEFT_TOP, [offset_x, 0.0])
                .show(ctx, |ui| {
                    egui::Frame::NONE
                        .fill(panel_fill)
                        .inner_margin(egui::Margin::same(8))
                        .show(ui, |ui| {
                            ui.set_width(width - 16.0);
                            ui.set_min_height(content.height() - 16.0);
                            self.playlist_pane_contents(ui);
                        });
                    // the pane's right hairline — the docked panel's
                    // resize edge, mimed so the two shapes match
                    let rect = ui.min_rect();
                    ui.painter().line_segment(
                        [egui::pos2(rect.max.x, rect.min.y),
                         egui::pos2(rect.max.x, content.max.y)],
                        egui::Stroke::new(1.0, edge));
                });
            return;
        }
        egui::SidePanel::left("playlist")
            .default_width(240.0)
            .show_animated(ctx, open, |ui| {
                self.playlist_pane_contents(ui);
            });
    }

    /// The pane's one body: heading + close affordance + the rows
    /// (shared by the docked panel and mini's slide-over).
    fn playlist_pane_contents(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Playlist");
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::Center),
                |ui| {
                    if ui.button(icon::X)
                        .on_hover_text("Close (L)")
                        .clicked()
                    {
                        self.player.panel_open = false;
                        self.settings.playlist_panel_open = false;
                    }
                });
        });
        ui.separator();
        self.playlist_rows(ui);
    }

    fn playlist_rows(&mut self, ui: &mut egui::Ui) {
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
