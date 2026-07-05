// SPDX-License-Identifier: GPL-3.0-or-later
//! Compose mode, the shell side: draw a shape on the scope, hear it —
//! the state machine ported from v3 phosphor.py §compose. The math
//! lives in phosphor_dsp::compose (studio reuses it — one engine rule).
//!
//! The mode's laws, verbatim from v3:
//! - entering stops capture/playback and forces XY (drawing only makes
//!   sense there); the pointer becomes a crosshair over the scope.
//! - the in-progress stroke previews directly as segments, restamped
//!   every frame at intensity 0.25 — with per-frame decay this settles
//!   at a steady brightness, like a held trace. Any still-playing loop
//!   audio is drained meanwhile so it can't burst in later.
//! - release inverts the display transform (gain-aware, so the loop
//!   plays back exactly where it was drawn) and starts a seamless
//!   looping WAV; scroll retunes 20–400 Hz, regenerating debounced.
//! - drawing and retuning are desktop-only (mini keeps its own mouse).

use std::time::{Duration, Instant};

use phosphor_dsp::compose as dsp;

/// v3 COMPOSE_PREVIEW_INTENSITY: restamped every frame while drawing.
pub(crate) const PREVIEW_INTENSITY: f32 = 0.25;
/// v3 COMPOSE_MINIMUM_POINTS.
pub(crate) const MINIMUM_POINTS: usize = 8;
/// Short file looped forever by the player.
const LOOP_FILE_SECONDS: f64 = 1.0;
/// A shareable take of the drawing.
const EXPORT_SECONDS: f64 = 10.0;
const TOO_SMALL: &str = "✏ shape too small — draw a bigger one";

fn loop_wave_path() -> std::path::PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_default();
    std::path::PathBuf::from(home)
        .join(".cache/phosphor/compose-loop.wav")
}

fn export_directory() -> std::path::PathBuf {
    let home = std::env::var_os("HOME").unwrap_or_default();
    std::path::PathBuf::from(home).join("Music/Phosphor")
}

fn export_drawing_wav(points: &[(f64, f64)], frequency_hz: f64,
                      sample_rate: u32)
    -> Result<std::path::PathBuf, String>
{
    let cycle = dsp::loop_samples(points, frequency_hz, sample_rate)
        .map_err(|error| error.to_string())?;
    let cycle_count =
        ((EXPORT_SECONDS * frequency_hz).round() as usize).max(1);
    let frames = dsp::tile_cycle(&cycle, cycle_count);
    let directory = export_directory();
    std::fs::create_dir_all(&directory)
        .map_err(|error| error.to_string())?;
    let path = directory.join(format!("phosphor-drawing-{}.wav",
                                      crate::exports::timestamp()));
    crate::exports::write_wav(&path, &frames, sample_rate)
        .map_err(|error| error.to_string())?;
    Ok(path)
}

impl crate::shell::Shell {
    pub(crate) fn enter_compose(&mut self) {
        if self.composing {
            return;
        }
        self.engine.stop_capture();
        self.capture_on = false;
        if self.engine.is_playing_file() {
            self.engine.stop_playback();
        }
        // a stale gapless preload must not splice into the loop
        self.engine.set_next_track(None);
        self.player.playing = None;
        self.player.paused = false;
        self.set_mpris_status("Stopped");
        self.composing = true;
        self.compose_drawing = false;
        self.compose_stroke.clear();
        self.compose_loop_points = None;
        // drawing only makes sense in XY (v3 law)
        if self.settings.display_mode != "xy" {
            self.settings.display_mode = "xy".into();
            if let Ok(mode) = "xy".parse::<phosphor_dsp::Mode>() {
                self.computer.mode = mode;
            }
        }
        self.status_line =
            "✏ draw a shape on the scope — release to hear it".into();
        self.toast_now(
            "✏ draw a shape on the scope — release to hear it");
        self.wake_render_loop();
        self.chrome_dirty = true;
    }

    /// `stop_loop: false` when a new stream is about to replace the
    /// loop anyway (starting capture or a track — v3 law).
    pub(crate) fn exit_compose(&mut self, stop_loop: bool) {
        if !self.composing {
            return;
        }
        self.composing = false;
        self.compose_drawing = false;
        self.compose_stroke.clear();
        self.compose_retune_due = None;
        if stop_loop && self.engine.is_playing_file() {
            self.engine.stop_playback();
            self.status_line = "idle".into();
            // fade the glow like capture-off does (v3: 90 frames)
            self.wake_render_loop();
        }
        self.chrome_dirty = true;
    }

    /// Pointer traffic on the scope while composing (desktop only —
    /// v3 blocked drawing in mini). Runs inside the egui pass, so
    /// repaints go through the ctx, not the winit window.
    pub(crate) fn compose_pointer(&mut self, ui: &egui::Ui,
                                  response: &egui::Response) {
        if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if response.drag_started() {
            self.compose_drawing = true;
            self.compose_stroke.clear();
            if let Some(position) = response.interact_pointer_pos() {
                self.compose_stroke.push(position);
            }
            self.wake_render_loop();
        } else if response.dragged() {
            if let Some(position) = response.interact_pointer_pos()
                && self.compose_stroke.last() != Some(&position)
            {
                self.compose_stroke.push(position);
            }
            ui.ctx().request_repaint();
        }
        if response.drag_stopped() {
            self.finish_compose_stroke();
        } else if response.clicked() {
            // a press that never traveled: v3's too-small path
            self.toast_now(TOO_SMALL);
        }
    }

    fn finish_compose_stroke(&mut self) {
        self.compose_drawing = false;
        if self.compose_stroke.len() < MINIMUM_POINTS {
            self.compose_stroke.clear();
            self.status_line = TOO_SMALL.into();
            self.toast_now(TOO_SMALL);
            return;
        }
        // Invert the display transform using the current gain so the
        // loop plays back exactly where it was drawn (v3
        // _scope_points_from_widget; radius law from dsp modes/xy.rs).
        let rect = self.scope_rect;
        let center = rect.center();
        let radius = rect.width().min(rect.height()) * 0.45
            * self.effective_gain.max(0.001);
        let points: Vec<(f64, f64)> = self.compose_stroke.iter()
            .map(|position| (
                (((position.x - center.x) / radius) as f64)
                    .clamp(-1.0, 1.0),
                (((center.y - position.y) / radius) as f64)
                    .clamp(-1.0, 1.0),
            ))
            .collect();
        self.compose_stroke.clear();
        self.compose_loop_points = Some(points);
        self.restart_compose_loop();
    }

    pub(crate) fn restart_compose_loop(&mut self) {
        let Some(points) = self.compose_loop_points.clone() else {
            return;
        };
        let frequency = dsp::clamp_frequency(
            self.settings.compose_frequency_hz as f64);
        self.settings.compose_frequency_hz = frequency as f32;
        let rate = self.settings.scope_sample_rate;
        let result = dsp::loop_samples(&points, frequency, rate)
            .map_err(|error| error.to_string())
            .and_then(|cycle| {
                let cycle_count =
                    ((LOOP_FILE_SECONDS * frequency).round() as usize)
                        .max(1);
                let frames = dsp::tile_cycle(&cycle, cycle_count);
                let path = loop_wave_path();
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| error.to_string())?;
                }
                crate::exports::write_wav(&path, &frames, rate)
                    .map_err(|error| error.to_string())?;
                Ok(path)
            });
        match result {
            Ok(path) => {
                self.engine.set_volume(crate::player::cubic_volume(
                    self.settings.playback_volume));
                self.engine.start_file(
                    &path, 0.0, true, self.settings.vacuum_enabled);
                self.status_line = format!(
                    "✏ {frequency:.0} Hz loop — scroll to retune, \
                     draw to replace");
                self.wake_render_loop();
            }
            Err(error) => {
                let message = format!("compose failed: {error}");
                self.status_line = message.clone();
                self.toast_now(message);
            }
        }
        self.chrome_dirty = true;
    }

    /// Scroll while composing = pitch. Regeneration is debounced so a
    /// scroll flick only restarts the decoder once (v3: 300 ms).
    pub(crate) fn retune_compose(&mut self, notches: f64) {
        let frequency = dsp::clamp_frequency(
            self.settings.compose_frequency_hz as f64
                * 1.06f64.powf(notches));
        self.settings.compose_frequency_hz = frequency as f32;
        self.status_line = format!("✏ {frequency:.0} Hz — retuning…");
        self.compose_retune_due =
            Some(Instant::now() + Duration::from_millis(300));
    }

    /// Tick-level debounce check (the retune twin of v3's
    /// GLib.timeout_add).
    pub(crate) fn service_compose_retune(&mut self) {
        if let Some(due) = self.compose_retune_due
            && Instant::now() >= due
        {
            self.compose_retune_due = None;
            if self.composing && self.compose_loop_points.is_some() {
                self.restart_compose_loop();
            }
        }
    }

    /// The in-progress stroke as renderer segments in trace pixels —
    /// restamped every frame; per-frame decay settles it at a steady
    /// brightness, like a held trace (v3 _advance_compose_preview).
    pub(crate) fn compose_preview_segments(&self, trace_width: f32,
                                           trace_height: f32)
        -> Vec<[f32; 5]>
    {
        if self.compose_stroke.len() < 2 {
            return Vec::new();
        }
        let rect = self.scope_rect;
        let scale_x = trace_width / rect.width().max(1.0);
        let scale_y = trace_height / rect.height().max(1.0);
        self.compose_stroke.windows(2).map(|pair| [
            (pair[0].x - rect.min.x) * scale_x,
            (pair[0].y - rect.min.y) * scale_y,
            (pair[1].x - rect.min.x) * scale_x,
            (pair[1].y - rect.min.y) * scale_y,
            PREVIEW_INTENSITY,
        ]).collect()
    }

    /// Context-menu export: EXPORT_SECONDS of audio that draws the
    /// shape on any XY oscilloscope, saved beside the other exports.
    pub(crate) fn export_compose_drawing(&mut self) {
        let Some(points) = self.compose_loop_points.clone() else {
            return;
        };
        if self.exporting {
            self.status_line = "export already running".into();
            return;
        }
        let frequency = self.settings.compose_frequency_hz as f64;
        let rate = self.settings.scope_sample_rate;
        let (sender, receiver) = std::sync::mpsc::channel();
        self.export_results = Some(receiver);
        self.exporting = true;
        self.status_line = "writing drawing WAV…".into();
        std::thread::spawn(move || {
            let _ = sender.send(
                export_drawing_wav(&points, frequency, rate));
        });
    }
}
