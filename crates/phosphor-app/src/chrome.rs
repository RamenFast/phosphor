// SPDX-License-Identifier: GPL-3.0-or-later
//! Chrome pass i — transport row, sliders, settings panel (UI-SPEC
//! §4.3–4.5, labels and semantics verbatim; egui owns the look, that
//! is a feature). The save-immediately table from §3.2 is law: keys
//! marked "yes" call save() at the moment they change, everything
//! else waits for the clean-shutdown catch-all.

use phosphor_audio::TargetKind;

use crate::shell::{Shell, UiAction};

/// v3 DISPLAY_MODES, id → label, exact order.
pub const DISPLAY_MODES: [(&str, &str); 11] = [
    ("xy", "XY (scope art)"),
    ("xy45", "XY · goniometer"),
    ("xy_swirl", "XY · swirl"),
    ("xy_dots", "XY · dots"),
    ("xyz_takens", "3D · attractor"),
    ("helix", "3D · time helix"),
    ("waveform", "Waveform"),
    ("ring", "Ring · oscillogram"),
    ("spectrum", "Spectrum"),
    ("spectrum_radial", "Spectrum · radial"),
    ("tunnel", "Spectrum · tunnel"),
];

pub const THEME_NAMES: [&str; 10] = [
    "P7 Green", "Amber", "Ice Blue", "White", "Vaporwave",
    "Red Phosphor", "Ultraviolet", "Solar Gold", "Cyan Tube", "Custom",
];

const SCOPE_RATE_CHOICES: [(u32, &str); 4] = [
    (48_000, "Standard · 48 kHz"),
    (96_000, "Fine · 96 kHz"),
    (192_000, "Ultra · 192 kHz"),
    (384_000, "Extreme · 384 kHz"),
];

const GPU_QUALITY_CHOICES: [(u32, &str); 3] = [
    (1, "Standard"),
    (2, "High · 2× supersampled"),
    (3, "Ultra · 3× supersampled"),
];

const CPU_RESOLUTION_CHOICES: [(f32, &str); 3] = [
    (1.0, "Full resolution"),
    (0.75, "Balanced · 75%"),
    (0.5, "Fast · 50%"),
];

const MAX_FPS_PRESETS: [i64; 10] = [0, 30, 60, 90, 120, 144, 165, 240, 360, 480];

/// UI styles as DATA (V4PLAN: egui owns the chrome completely — the
/// 8 v3 style ids keep their names; "system" is dark, like v3's
/// pass-through resolves on Ben's desktop).
/// (id, panel_fill, window_fill, accent, text, translucent_panels)
pub const UI_STYLES: [(&str, [u8; 3], [u8; 3], [u8; 3], [u8; 3], bool); 8] = [
    ("system",     [30, 30, 34],    [24, 24, 27],  [110, 170, 255], [222, 222, 222], false),
    ("dark",       [30, 30, 34],    [24, 24, 27],  [110, 170, 255], [222, 222, 222], false),
    ("light",      [242, 242, 245], [250, 250, 250], [40, 110, 220], [30, 30, 30],   false),
    ("black",      [0, 0, 0],       [0, 0, 0],     [90, 240, 130],  [200, 210, 200], false),
    ("bloom",      [34, 26, 34],    [26, 20, 28],  [255, 120, 190], [235, 220, 232], false),
    ("stone",      [44, 42, 40],    [36, 34, 32],  [200, 170, 120], [225, 220, 210], false),
    ("stonebloom", [42, 36, 42],    [34, 28, 34],  [235, 150, 180], [230, 220, 226], false),
    ("aero",       [40, 52, 66],    [30, 40, 54],  [140, 200, 255], [230, 240, 250], true),
];

pub const MINI_SIZE_PRESETS: [(&str, i64); 4] = [
    ("Small", 200), ("Medium", 280), ("Large", 380), ("Extra large", 520),
];

impl Shell {
    /// The main toolbar row (§4.3): [⏻ Live][status…][⏺][📷][mode][⟳][target][icon]
    pub(crate) fn ui_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let mut live = self.capture_on;
            if ui.toggle_value(&mut live, "⏻ Live")
                .on_hover_text("Toggle audio capture (Space). Off = zero CPU.")
                .clicked()
            {
                self.actions.push(if live {
                    UiAction::CaptureOn
                } else {
                    UiAction::CaptureOff
                });
            }
            if ui.button("📂").on_hover_text("Play audio file (O)")
                .clicked()
            {
                self.actions.push(UiAction::OpenFile);
            }
            if self.settings.show_pin_button {
                let mut pinned = self.settings.pinned;
                if ui.toggle_value(&mut pinned, "📌")
                    .on_hover_text("Pin above other windows (P)")
                    .clicked()
                {
                    self.actions.push(UiAction::PinToggle);
                }
            }

            // pack_end order, right → left
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let kind_icon = match self.settings.target_id.as_deref() {
                    Some(id) if id.starts_with("app:") => "🎵",
                    Some(id) if id.ends_with(".monitor") => "🔊",
                    Some(_) => "🎙",
                    None => "🔊",
                };
                ui.label(kind_icon);

                let selected_label = self
                    .target_cache
                    .iter()
                    .find(|t| Some(t.combo_id()) == self.settings.target_id)
                    .map(|t| t.label.clone())
                    .unwrap_or_else(|| {
                        self.settings.target_id.clone()
                            .unwrap_or_else(|| "—".into())
                    });
                egui::ComboBox::from_id_salt("target")
                    .width(240.0)
                    .selected_text(selected_label)
                    .show_ui(ui, |ui| {
                        let mut clicked = None;
                        for target in &self.target_cache {
                            let id = target.combo_id();
                            let checked =
                                Some(&id) == self.settings.target_id.as_ref();
                            if ui.selectable_label(checked, &target.label)
                                .clicked()
                            {
                                clicked = Some(id);
                            }
                        }
                        if let Some(id) = clicked {
                            self.settings.target_id = Some(id.clone());
                            self.actions.push(UiAction::TargetPicked(id));
                        }
                    })
                    .response
                    .on_hover_text(
                        "What to scope: APP = one playing application, \
                         OUT = everything on that output, IN = microphones");

                if ui.button("⟳")
                    .on_hover_text("Re-scan devices and playing apps")
                    .clicked()
                {
                    self.actions.push(UiAction::RefreshTargets);
                }

                let mode_label = DISPLAY_MODES
                    .iter()
                    .find(|(id, _)| *id == self.settings.display_mode)
                    .map(|(_, label)| *label)
                    .unwrap_or("XY (scope art)");
                egui::ComboBox::from_id_salt("mode")
                    .selected_text(mode_label)
                    .show_ui(ui, |ui| {
                        for (id, label) in DISPLAY_MODES {
                            if ui.selectable_label(
                                self.settings.display_mode == id, label)
                                .clicked()
                            {
                                self.settings.display_mode = id.to_string();
                                self.actions.push(UiAction::ModeChanged);
                            }
                        }
                    });

                if ui.button("📷")
                    .on_hover_text("Snapshot to ~/Pictures/Phosphor (S)")
                    .clicked()
                {
                    self.actions.push(UiAction::SaveSnapshot);
                }
                if ui.button("⏺")
                    .on_hover_text("Save the last 10 s as mp4 with sound (C)")
                    .clicked()
                {
                    self.actions.push(UiAction::SaveClip);
                }
                if ui.toggle_value(&mut self.settings_panel_open, "⚙")
                    .on_hover_text("Settings")
                    .clicked()
                {}

                // status text expands in the middle (ellipsized by clip)
                let status = self.status_line.clone();
                ui.add(egui::Label::new(status).truncate());
            });
        });
    }

    /// The slider row (§4.4): Gain, Glow, Beam — scale + percent spin.
    pub(crate) fn ui_sliders(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let auto_gain = self.settings.auto_gain;
            let mut gain = self.settings.gain;
            if slider_with_percent(ui, "Gain", &mut gain, 0.1, 6.0,
                                   "Deflection scale (also mouse scroll)",
                                   !auto_gain)
            {
                self.settings.gain = gain;
                self.actions.push(UiAction::SignalTuning);
            }
            let mut glow = self.settings.persistence;
            if slider_with_percent(ui, "Glow", &mut glow, 0.0, 0.98,
                                   "Phosphor persistence — how long trails linger",
                                   true)
            {
                self.settings.persistence = glow;
                self.actions.push(UiAction::RenderTuning);
            }
            let mut beam = self.settings.beam_energy;
            if slider_with_percent(ui, "Beam", &mut beam, 1.0, 30.0,
                                   "Beam brightness budget — higher keeps \
                                    fast strokes visible",
                                   true)
            {
                self.settings.beam_energy = beam;
                self.actions.push(UiAction::SignalTuning);
            }
        });
    }

    /// The settings panel (§4.5) — two columns as a side panel; egui
    /// owns the chrome (V4PLAN: that's a feature, not a deviation).
    pub(crate) fn ui_settings_panel(&mut self, ctx: &egui::Context) {
        if !self.settings_panel_open {
            return;
        }
        let mut open = self.settings_panel_open;
        egui::SidePanel::right("settings")
            .resizable(false)
            .default_width(280.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.heading("Settings");
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if ui.button("✕").clicked() {
                                    open = false;
                                }
                            });
                    });
                    self.ui_settings_renderer(ui);
                    self.ui_settings_scope(ui);
                    self.ui_settings_appearance(ui);
                    self.ui_settings_performance(ui);
                });
            });
        self.settings_panel_open = open;
    }

    fn ui_settings_renderer(&mut self, ui: &mut egui::Ui) {
        section(ui, "RENDERER");
        let renderer_label = if self.settings.renderer == "cairo" {
            "CPU · cairo"
        } else {
            "GPU · CRT beam (recommended)"
        };
        egui::ComboBox::from_label("Renderer")
            .selected_text(renderer_label)
            .show_ui(ui, |ui| {
                for (id, label) in [("gl", "GPU · CRT beam (recommended)"),
                                    ("cairo", "CPU · cairo")] {
                    if ui.selectable_label(self.settings.renderer == id,
                                           label).clicked()
                        && self.settings.renderer != id
                    {
                        self.settings.renderer = id.to_string();
                        self.actions.push(UiAction::RendererChanged);
                        self.actions.push(UiAction::SaveSettings);
                    }
                }
            });
        let quality_label = GPU_QUALITY_CHOICES
            .iter()
            .find(|(v, _)| *v == self.settings.gl_supersample)
            .map(|(_, l)| *l)
            .unwrap_or("Standard");
        egui::ComboBox::from_label("GPU quality")
            .selected_text(quality_label)
            .show_ui(ui, |ui| {
                for (value, label) in GPU_QUALITY_CHOICES {
                    if ui.selectable_label(
                        self.settings.gl_supersample == value, label)
                        .clicked()
                    {
                        self.settings.gl_supersample = value;
                        self.actions.push(UiAction::RendererChanged);
                        self.actions.push(UiAction::SaveSettings);
                    }
                }
            });
        // nearest-value match, v3's quirk (§3.2 cairo_resolution)
        let nearest = CPU_RESOLUTION_CHOICES
            .iter()
            .min_by(|a, b| {
                (a.0 - self.settings.cairo_resolution).abs()
                    .total_cmp(&(b.0 - self.settings.cairo_resolution).abs())
            })
            .unwrap();
        egui::ComboBox::from_label("CPU resolution")
            .selected_text(nearest.1)
            .show_ui(ui, |ui| {
                for (value, label) in CPU_RESOLUTION_CHOICES {
                    if ui.selectable_label(nearest.0 == value, label)
                        .clicked()
                    {
                        self.settings.cairo_resolution = value;
                        self.actions.push(UiAction::RendererChanged);
                        self.actions.push(UiAction::SaveSettings);
                    }
                }
            });
    }

    fn ui_settings_scope(&mut self, ui: &mut egui::Ui) {
        section(ui, "SCOPE");
        let rate_label = SCOPE_RATE_CHOICES
            .iter()
            .find(|(v, _)| *v == self.settings.scope_sample_rate)
            .map(|(_, l)| *l)
            .unwrap_or("Fine · 96 kHz");
        egui::ComboBox::from_label("Scope detail")
            .selected_text(rate_label)
            .show_ui(ui, |ui| {
                for (value, label) in SCOPE_RATE_CHOICES {
                    if ui.selectable_label(
                        self.settings.scope_sample_rate == value, label)
                        .clicked()
                        && self.settings.scope_sample_rate != value
                    {
                        self.settings.scope_sample_rate = value;
                        self.actions.push(UiAction::ScopeRateChanged);
                        self.actions.push(UiAction::SaveSettings);
                    }
                }
            })
            .response
            .on_hover_text(
                "Scope feed sample rate — higher rates trace the true \
                 curves\nbetween samples, recovering fine scope-art detail");
        let mut focus = self.settings.beam_focus;
        if ui.add(egui::Slider::new(&mut focus, 0.6..=3.0)
                  .step_by(0.1).text("Focus"))
            .on_hover_text("Beam focus — narrower keeps dense scenes \
                            from washing out")
            .changed()
        {
            self.settings.beam_focus = focus;
            self.actions.push(UiAction::RenderTuning);
        }
        if ui.checkbox(&mut self.settings.auto_gain, "Auto gain")
            .on_hover_text(
                "Autosize the trace to the screen — gain follows the \
                 signal's\npeak so quiet tracks still fill the display")
            .changed()
        {
            self.actions.push(UiAction::SignalTuning);
        }
        if ui.checkbox(&mut self.settings.grid_enabled, "Grid")
            .changed()
        {
            self.actions.push(UiAction::RenderTuning);
        }
        if ui.checkbox(&mut self.settings.amoled_background, "AMOLED scope")
            .changed()
        {
            self.actions.push(UiAction::RenderTuning);
        }
        if ui.checkbox(&mut self.settings.scope_glass, "Glass scope")
            .on_hover_text(
                "Translucent scope pane — the beam glows over whatever \
                 is\nbehind the window (needs a compositing desktop; \
                 pairs\nbeautifully with the mini view and Aero glass)")
            .changed()
        {
            self.actions.push(UiAction::RenderTuning);
        }
        let style = self.settings.ui_style.clone();
        let mut tint = *self.settings.glass_tints.get(&style)
            .unwrap_or(&self.settings.glass_tint);
        ui.add_enabled_ui(self.settings.scope_glass, |ui| {
            if ui.add(egui::Slider::new(&mut tint, 0.0..=0.95)
                      .step_by(0.05).text("Glass tint"))
                .on_hover_text(
                    "How dark the glass smokes the desktop behind the \
                     scope —\nfully clear on the left, nearly opaque on \
                     the right.\nRemembered separately for each UI style.")
                .changed()
            {
                self.settings.glass_tints.insert(style, tint);
                self.actions.push(UiAction::RenderTuning);
            }
        });
    }

    fn ui_settings_appearance(&mut self, ui: &mut egui::Ui) {
        section(ui, "APPEARANCE");
        egui::ComboBox::from_label("Theme")
            .selected_text(self.settings.theme_name.clone())
            .show_ui(ui, |ui| {
                for name in THEME_NAMES {
                    if ui.selectable_label(
                        self.settings.theme_name == name, name).clicked()
                        && self.settings.theme_name != name
                    {
                        self.settings.theme_name = name.to_string();
                        self.actions.push(UiAction::RenderTuning);
                        self.actions.push(UiAction::SaveSettings);
                    }
                }
            });
        if self.settings.theme_name == "Custom" {
            let mut beam = self.settings.custom_beam_color;
            if ui.horizontal(|ui| {
                ui.label("Custom beam");
                ui.color_edit_button_rgb(&mut beam).changed()
            }).inner {
                self.settings.custom_beam_color = beam;
                self.actions.push(UiAction::RenderTuning);
            }
            let mut grid = self.settings.custom_grid_color;
            if ui.horizontal(|ui| {
                ui.label("Custom grid");
                ui.color_edit_button_rgb(&mut grid).changed()
            }).inner {
                self.settings.custom_grid_color = grid;
                self.actions.push(UiAction::RenderTuning);
            }
        }
        let current_style = self.settings.ui_style.clone();
        egui::ComboBox::from_label("UI style")
            .selected_text(current_style.clone())
            .show_ui(ui, |ui| {
                for (id, ..) in UI_STYLES {
                    if ui.selectable_label(current_style == id, id)
                        .clicked()
                        && current_style != id
                    {
                        self.settings.ui_style = id.to_string();
                        // Aero coupling (v3 §7): fires on EVERY style
                        // change — aero forces glass ON, any other
                        // style forces it OFF.
                        let want_glass = id == "aero";
                        if self.settings.scope_glass != want_glass {
                            self.settings.scope_glass = want_glass;
                        }
                        self.actions.push(UiAction::RenderTuning);
                        self.actions.push(UiAction::SaveSettings);
                    }
                }
            });
        if ui.checkbox(&mut self.settings.show_pin_button, "Pin button")
            .changed()
        {}
        if ui.checkbox(&mut self.settings.show_now_playing, "Track info")
            .on_hover_text(
                "Fade the artist/title into the corner when the song \
                 changes —\nfor files Phosphor plays and for other \
                 players (MPRIS)")
            .changed()
        {}
    }

    fn ui_settings_performance(&mut self, ui: &mut egui::Ui) {
        section(ui, "PERFORMANCE");
        let fps_label = if self.settings.max_fps == 0 {
            "Monitor".to_string()
        } else {
            self.settings.max_fps.to_string()
        };
        egui::ComboBox::from_label("Max FPS")
            .selected_text(fps_label)
            .show_ui(ui, |ui| {
                for preset in MAX_FPS_PRESETS {
                    let label = if preset == 0 {
                        "Monitor".to_string()
                    } else {
                        preset.to_string()
                    };
                    if ui.selectable_label(
                        self.settings.max_fps == preset, label).clicked()
                    {
                        self.settings.max_fps = preset;
                        self.actions.push(UiAction::SaveSettings);
                    }
                }
            })
            .response
            .on_hover_text(
                "Frame rate cap — Monitor follows the display's refresh \
                 rate;\npick a preset or type any rate up to 1000.");
        if ui.checkbox(&mut self.settings.show_fps, "Show FPS")
            .changed()
        {}
    }

    /// Apply the UI style's egui visuals (data table above). Aero and
    /// glass make the chrome slightly translucent over the desktop.
    pub(crate) fn apply_ui_style(&mut self, ctx: &egui::Context) {
        let style = UI_STYLES
            .iter()
            .find(|(id, ..)| *id == self.settings.ui_style)
            .unwrap_or(&UI_STYLES[1]);
        let (_, panel, window, accent, text, translucent) = *style;
        let alpha = if translucent || self.settings.scope_glass {
            220
        } else {
            255
        };
        let mut visuals = if style.0 == "light" {
            egui::Visuals::light()
        } else {
            egui::Visuals::dark()
        };
        visuals.panel_fill = egui::Color32::from_rgba_unmultiplied(
            panel[0], panel[1], panel[2], alpha);
        visuals.window_fill = egui::Color32::from_rgb(
            window[0], window[1], window[2]);
        visuals.selection.bg_fill = egui::Color32::from_rgb(
            accent[0], accent[1], accent[2]);
        visuals.override_text_color = Some(egui::Color32::from_rgb(
            text[0], text[1], text[2]));
        ctx.set_visuals(visuals);
    }

    /// The context menu (§5.1 tree; items land as their passes do).
    pub(crate) fn ui_context_menu(&mut self, response: &egui::Response) {
        response.context_menu(|ui| {
            let capture_label = if self.capture_on {
                "Pause capture"
            } else {
                "Resume capture"
            };
            if ui.button(capture_label).clicked() {
                self.actions.push(if self.capture_on {
                    UiAction::CaptureOff
                } else {
                    UiAction::CaptureOn
                });
                ui.close();
            }
            let app_target = self.settings.target_id.as_deref()
                .map(|id| id.starts_with("app:")).unwrap_or(false);
            if self.player.playing.is_none()
                && (app_target || self.app_vacuum.is_some())
            {
                let mut vacuum_on = self.app_vacuum.is_some();
                if ui.checkbox(&mut vacuum_on,
                               "Vacuum this app  ⌀  (light only)")
                    .clicked()
                {
                    self.actions.push(UiAction::VacuumApp(vacuum_on));
                    ui.close();
                }
            }
            if ui.button("Play audio file…  (O)").clicked() {
                self.actions.push(UiAction::OpenFile);
                ui.close();
            }
            if self.player.playing.is_some() {
                let pause_label = if self.player.paused {
                    "Resume track"
                } else {
                    "Pause track"
                };
                if ui.button(pause_label).clicked() {
                    self.actions.push(UiAction::PlayerTogglePause);
                    ui.close();
                }
                let many = self.player.playlist.len() > 1;
                if ui.add_enabled(many, egui::Button::new("Next track  ⏭"))
                    .clicked()
                {
                    self.actions.push(UiAction::PlayerNext);
                    ui.close();
                }
                if ui.add_enabled(many,
                                  egui::Button::new("Previous track  ⏮"))
                    .clicked()
                {
                    self.actions.push(UiAction::PlayerPrevious);
                    ui.close();
                }
                if many {
                    ui.menu_button("Tracks", |ui| {
                        // windowed to 25 around current if > 30 (v3)
                        let total = self.player.playlist.len();
                        let current = self.player.playlist_index;
                        let (start, end) = if total > 30 {
                            let start = current.saturating_sub(12);
                            (start, (start + 25).min(total))
                        } else {
                            (0, total)
                        };
                        let mut clicked = None;
                        for index in start..end {
                            let path = &self.player.playlist[index];
                            let name = path.file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default();
                            if ui.selectable_label(index == current, name)
                                .clicked()
                            {
                                clicked = Some(path.clone());
                            }
                        }
                        if let Some(path) = clicked {
                            self.actions.push(UiAction::PlayPath(path));
                            ui.close();
                        }
                    });
                }
            }
            if !self.is_mini {
                if ui.button("Compose · draw a shape  (D)").clicked() {
                    self.actions.push(UiAction::ComposeToggle);
                    ui.close();
                }
                let mut panel = self.player.panel_open;
                if ui.checkbox(&mut panel, "Playlist panel  (L)").clicked() {
                    self.player.panel_open = panel;
                    self.settings.playlist_panel_open = panel;
                    ui.close();
                }
            }
            if ui.button("Snapshot  (S)").clicked() {
                self.actions.push(UiAction::SaveSnapshot);
                ui.close();
            }
            if ui.button("Save last 10 s  (C)").clicked() {
                self.actions.push(UiAction::SaveClip);
                ui.close();
            }
            ui.separator();
            ui.menu_button("Display mode", |ui| {
                for (id, label) in DISPLAY_MODES {
                    if ui.selectable_label(
                        self.settings.display_mode == id, label).clicked()
                    {
                        self.settings.display_mode = id.to_string();
                        self.actions.push(UiAction::ModeChanged);
                        ui.close();
                    }
                }
            });
            ui.menu_button("Theme", |ui| {
                for name in THEME_NAMES {
                    if ui.selectable_label(
                        self.settings.theme_name == name, name).clicked()
                    {
                        self.settings.theme_name = name.to_string();
                        self.actions.push(UiAction::RenderTuning);
                        self.actions.push(UiAction::SaveSettings);
                        ui.close();
                    }
                }
            });
            if ui.checkbox(&mut self.settings.grid_enabled, "Grid  (G)")
                .clicked()
            {
                self.actions.push(UiAction::RenderTuning);
                ui.close();
            }
            if ui.checkbox(&mut self.settings.show_fps, "Show FPS  (F)")
                .clicked()
            {
                ui.close();
            }
            if ui.checkbox(&mut self.settings.auto_gain,
                           "Auto gain — fit to screen").clicked() {
                self.actions.push(UiAction::SignalTuning);
                ui.close();
            }
            if ui.checkbox(&mut self.settings.scope_glass,
                           "Glass scope — transparent background")
                .clicked()
            {
                self.actions.push(UiAction::RenderTuning);
                ui.close();
            }
            let mut pinned = self.settings.pinned;
            if ui.checkbox(&mut pinned, "Pin above  (P)").clicked() {
                self.actions.push(UiAction::PinToggle);
                ui.close();
            }
            ui.separator();
            if self.is_mini {
                ui.menu_button("Align", |ui| {
                    for (label, fx, fy) in [
                        ("◰  Top left", 0.0, 0.0),
                        ("◳  Top right", 1.0, 0.0),
                        ("◱  Bottom left", 0.0, 1.0),
                        ("◲  Bottom right", 1.0, 1.0),
                        ("▣  Center", 0.5, 0.5),
                    ] {
                        if ui.button(label).clicked() {
                            self.actions.push(
                                UiAction::AlignMini(fx, fy));
                            ui.close();
                        }
                    }
                });
                // four FLAT items, not nested (v3 port gotcha)
                for (label, size) in MINI_SIZE_PRESETS {
                    if ui.button(format!("Mini size: {label}")).clicked() {
                        self.actions.push(UiAction::MiniSizePreset(size));
                        ui.close();
                    }
                }
                if ui.button("Restore window  (M)").clicked() {
                    self.actions.push(UiAction::MiniToggle);
                    ui.close();
                }
            } else {
                if ui.button("Mini view  (M)").clicked() {
                    self.actions.push(UiAction::MiniToggle);
                    ui.close();
                }
                let fullscreen_label = if self.is_fullscreen {
                    "Leave fullscreen  (F11)"
                } else {
                    "Fullscreen scope  (F11)"
                };
                if ui.button(fullscreen_label).clicked() {
                    self.actions.push(UiAction::FullscreenToggle);
                    ui.close();
                }
            }
            ui.separator();
            if ui.button("Quit  (Q)").clicked() {
                self.actions.push(UiAction::Quit);
                ui.close();
            }
        });
    }

    /// Refresh the toolbar's target list (§4.3 semantics: rebuild,
    /// restore selection by id, else default monitor, else first).
    pub(crate) fn refresh_target_cache(&mut self) {
        self.target_cache = self.engine.targets();
        let current_ok = self
            .settings
            .target_id
            .as_ref()
            .map(|id| self.target_cache.iter().any(|t| &t.combo_id() == id))
            .unwrap_or(false);
        if !current_ok {
            self.settings.target_id = self
                .engine
                .default_monitor_target_id()
                .filter(|id| {
                    self.target_cache.iter().any(|t| &t.combo_id() == id)
                })
                .or_else(|| {
                    self.target_cache.first().map(|t| t.combo_id())
                });
        }
        let _ = self.target_cache.iter()
            .map(|t| t.kind == TargetKind::App)
            .count();
    }
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(12.0);
    ui.label(egui::RichText::new(title).small().strong());
    ui.separator();
}

/// v3's add_slider: [Label][Scale][percent spin][%] with two-way sync.
fn slider_with_percent(ui: &mut egui::Ui, name: &str, value: &mut f32,
                       minimum: f32, maximum: f32, tooltip: &str,
                       enabled: bool) -> bool {
    let mut changed = false;
    ui.add_enabled_ui(enabled, |ui| {
        ui.label(name);
        changed |= ui
            .add(egui::Slider::new(value, minimum..=maximum)
                 .show_value(false))
            .on_hover_text(tooltip)
            .changed();
        let mut percent =
            ((*value - minimum) / (maximum - minimum) * 100.0).round();
        let spin = ui.add(
            egui::DragValue::new(&mut percent)
                .range(0.0..=100.0).speed(1.0).suffix("%"));
        if spin.on_hover_text(format!("{name} as percent — type a value"))
            .changed()
        {
            *value = minimum + (maximum - minimum) * percent / 100.0;
            changed = true;
        }
    });
    changed
}
