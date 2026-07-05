// SPDX-License-Identifier: GPL-3.0-or-later
//! Chrome pass i — transport row, sliders, settings panel (UI-SPEC
//! §4.3–4.5, labels and semantics verbatim; egui owns the look, that
//! is a feature). The save-immediately table from §3.2 is law: keys
//! marked "yes" call save() at the moment they change, everything
//! else waits for the clean-shutdown catch-all.

use crate::shell::{Shell, UiAction};
use egui_phosphor::regular as icon;

/// Theme-switch crossfade state (thread-local — egui runs the chrome on
/// one thread). When `ui_style` changes we lerp EVERY palette token from
/// the palette on screen toward the new one over ~180 ms (smoothstep),
/// glass panel-alpha included. Kept here (not on the Shell) so the whole
/// animation lives in the chrome layer.
struct ThemeXfade {
    last_id: Option<String>,
    from: Option<crate::theme::Palette>,
    from_alpha: u8,
    last_alpha: u8,
    started: Option<std::time::Instant>,
}

thread_local! {
    static THEME_XFADE: std::cell::RefCell<ThemeXfade> =
        const { std::cell::RefCell::new(ThemeXfade {
            last_id: None, from: None,
            from_alpha: 255, last_alpha: 255, started: None,
        }) };
}

/// Duration of the theme-switch crossfade.
const THEME_XFADE_SECS: f32 = 0.18;

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

// -1 = Uncapped (new in v4); 0 = Monitor (v3 meaning); a v3 loading
// -1 clamps it to 0 -> Monitor, so the file stays cross-compatible.
const MAX_FPS_PRESETS: [i64; 11] =
    [0, -1, 30, 60, 90, 120, 144, 165, 240, 360, 480];

pub const MINI_SIZE_PRESETS: [(&str, i64); 4] = [
    ("Small", 200), ("Medium", 280), ("Large", 380), ("Extra large", 520),
];

/// Live-editable kit — the editor window's working copy. Rows are
/// generated from `phosphor_proto::phoskit::OPERATIONS` ("extend
/// tables, not UIs"); every edit re-applies through KitChanged.
pub struct KitEditorState {
    pub name: String,
    pub author: String,
    pub stages: Vec<phosphor_proto::phoskit::Stage>,
}

/// The "Export signal postcard…" dialog's working fields.
pub struct PostcardState {
    pub title: String,
    pub credit: String,
    pub source: std::path::PathBuf,
}

impl Shell {
    /// The main toolbar row (§4.3): [⏻ Live][status…][⏺][📷][mode][⟳][target][icon]
    pub(crate) fn ui_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // Live is a PRIMARY control → carved/dimensional (the
            // "stone toggle" — depth encodes importance, skill rule).
            if self.carved_toggle(ui, "⏻ LIVE", self.capture_on,
                                  "Toggle audio capture (Space). \
                                   Off = zero CPU.")
            {
                self.actions.push(if self.capture_on {
                    UiAction::CaptureOff
                } else {
                    UiAction::CaptureOn
                });
            }
            if ui.button(icon::FOLDER_OPEN).on_hover_text("Play audio file (O)")
                .clicked()
            {
                self.actions.push(UiAction::OpenFile);
            }
            if self.settings.show_pin_button {
                let mut pinned = self.settings.pinned;
                if ui.toggle_value(&mut pinned, icon::PUSH_PIN)
                    .on_hover_text("Pin above other windows (P)")
                    .clicked()
                {
                    self.actions.push(UiAction::PinToggle);
                }
            }

            // pack_end order, right → left
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let kind_icon = match self.settings.target_id.as_deref() {
                    Some(id) if id.starts_with("app:") => icon::MUSIC_NOTE,
                    Some(id) if id.ends_with(".monitor") => icon::SPEAKER_HIGH,
                    Some(_) => icon::MICROPHONE,
                    None => icon::SPEAKER_HIGH,
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

                if ui.button(icon::ARROW_CLOCKWISE)
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

                if ui.button(icon::CAMERA)
                    .on_hover_text("Snapshot to ~/Pictures/Phosphor (S)")
                    .clicked()
                {
                    self.actions.push(UiAction::SaveSnapshot);
                }
                if ui.button(icon::RECORD)
                    .on_hover_text("Save the last 10 s as mp4 with sound (C)")
                    .clicked()
                {
                    self.actions.push(UiAction::SaveClip);
                }
                ui.toggle_value(&mut self.settings_panel_open, icon::GEAR)
                    .on_hover_text("Settings");

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
            let mut text_ids = std::mem::take(&mut self.text_focus_ids);
            if slider_with_percent(ui, SliderSpec {
                name: "Gain", minimum: 0.1, maximum: 6.0,
                tooltip: "Deflection scale (also mouse scroll)",
                enabled: !auto_gain,
            }, &mut gain, &mut text_ids) {
                self.settings.gain = gain;
                self.actions.push(UiAction::SignalTuning);
            }
            let mut glow = self.settings.persistence;
            if slider_with_percent(ui, SliderSpec {
                name: "Glow", minimum: 0.0, maximum: 0.98,
                tooltip: "Phosphor persistence — how long trails linger",
                enabled: true,
            }, &mut glow, &mut text_ids) {
                self.settings.persistence = glow;
                self.actions.push(UiAction::RenderTuning);
            }
            let mut beam = self.settings.beam_energy;
            if slider_with_percent(ui, SliderSpec {
                name: "Beam", minimum: 1.0, maximum: 30.0,
                tooltip: "Beam brightness budget — higher keeps fast \
                          strokes visible",
                enabled: true,
            }, &mut beam, &mut text_ids) {
                self.settings.beam_energy = beam;
                self.actions.push(UiAction::SignalTuning);
            }
            self.text_focus_ids = text_ids;
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
                    self.ui_settings_kit(ui);
                    self.ui_settings_performance(ui);
                });
            });
        self.settings_panel_open = open;
    }

    fn ui_settings_renderer(&mut self, ui: &mut egui::Ui) {
        section(ui, "RENDERER", self.active_palette.muted);
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
        section(ui, "SCOPE", self.active_palette.muted);
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
        if ui.add(egui::Slider::new(&mut focus, 0.3..=3.0)
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
                      .step_by(0.01)
                      .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                      .custom_parser(|s| {
                          s.trim().trim_end_matches('%').trim().parse::<f64>()
                              .ok().map(|p| p / 100.0)
                      })
                      .text("Glass tint"))
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
        section(ui, "APPEARANCE", self.active_palette.muted);
        // the scope's PHOSPHOR color (P7 Green, Amber…) — labeled
        // "Beam" to distinguish from the UI "Theme" combo below
        egui::ComboBox::from_label("Beam")
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
            })
            .response
            .on_hover_text("The scope's phosphor color");
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
        // Theme selector: the six palettes (theme.rs). The v3 aero-
        // coupling law retires with the old style set — glass is now a
        // fully manual toggle usable with any theme (a deliberate v4
        // divergence, recorded in PARITY.md).
        let current_style = self.settings.ui_style.clone();
        let current_label = crate::theme::palette(&current_style).label;
        egui::ComboBox::from_label("Theme")
            .selected_text(current_label)
            .show_ui(ui, |ui| {
                for palette in crate::theme::PALETTES {
                    if ui.selectable_label(current_style == palette.id,
                                           palette.label)
                        .clicked()
                        && current_style != palette.id
                    {
                        self.settings.ui_style = palette.id.to_string();
                        self.actions.push(UiAction::RenderTuning);
                        self.actions.push(UiAction::SaveSettings);
                    }
                }
            });
        ui.checkbox(&mut self.settings.show_pin_button, "Pin button");
        if ui.checkbox(&mut self.settings.show_now_playing, "Track info")
            .on_hover_text(
                "Fade the artist/title into the corner when the song \
                 changes —\nfor files Phosphor plays and for other \
                 players (MPRIS)")
            .changed()
        {}
    }

    fn ui_settings_kit(&mut self, ui: &mut egui::Ui) {
        section(ui, "SIGNAL KIT", self.active_palette.muted);
        let mut kits: Vec<std::path::PathBuf> = Vec::new();
        let mut scan = |dir: std::path::PathBuf| {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case("phoskit"))
                    {
                        kits.push(path);
                    }
                }
            }
        };
        let home = std::env::var_os("HOME").unwrap_or_default();
        scan(std::path::PathBuf::from(&home)
             .join(".local/share/phosphor/kits"));
        // the deb's starter kits (v3 shipped these too); the relative
        // dir keeps repo-cwd development working
        scan(std::path::PathBuf::from("/usr/share/phosphor/kits"));
        scan(std::path::PathBuf::from("kits"));
        kits.sort();
        let selected = self.settings.kit_path.clone();
        let selected_name = selected.as_deref()
            .and_then(|p| std::path::Path::new(p).file_stem()
                      .map(|s| s.to_string_lossy().to_string()))
            .unwrap_or_else(|| "—".into());
        egui::ComboBox::from_label("Kit")
            .selected_text(selected_name)
            .show_ui(ui, |ui| {
                for kit in &kits {
                    let name = kit.file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let is_selected = selected.as_deref()
                        == Some(kit.to_string_lossy().as_ref());
                    if ui.selectable_label(is_selected, name).clicked() {
                        self.settings.kit_path =
                            Some(kit.to_string_lossy().to_string());
                        self.actions.push(UiAction::KitChanged);
                    }
                }
            })
            .response
            .on_hover_text(
                "A .phoskit transform chain bent into whatever plays —                  rotate,
widen, ring-mod, delay… Friends send these;                  drop one on the
window to import it.");
        if ui.checkbox(&mut self.settings.kit_enabled, "Apply kit")
            .on_hover_text(
                "Run the chosen kit's ops on the signal before every                  display
mode — the figure, the goniometer, the                  tunnel, all of it")
            .changed()
        {
            self.actions.push(UiAction::KitChanged);
        }
        if ui.button("Kit editor…")
            .on_hover_text("Build or tweak a chain live and save it \
                            as a .phoskit postcard")
            .clicked()
        {
            self.actions.push(UiAction::OpenKitEditor);
        }
    }

    fn ui_settings_performance(&mut self, ui: &mut egui::Ui) {
        section(ui, "PERFORMANCE", self.active_palette.muted);
        let fps_label = match self.settings.max_fps {
            0 => "Monitor".to_string(),
            fps if fps < 0 => "Uncapped".to_string(),
            fps => fps.to_string(),
        };
        egui::ComboBox::from_label("Max FPS")
            .selected_text(fps_label)
            .show_ui(ui, |ui| {
                for preset in MAX_FPS_PRESETS {
                    let label = match preset {
                        0 => "Monitor".to_string(),
                        p if p < 0 => "Uncapped".to_string(),
                        p => p.to_string(),
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
        // afterglow / blossom_dark chrome samples the live beam color;
        // every theme reads its tokens from the palette table (theme.rs).
        let beam = crate::render::build_theme(&self.settings).beam_color;
        let target = crate::theme::palette(&self.settings.ui_style)
            .with_beam(beam);
        // glass floats the chrome over the desktop → dim the panels
        let target_alpha = if self.settings.scope_glass { 210 } else { 255 };
        let cur_id = self.settings.ui_style.clone();
        let shown_before = self.active_palette;

        // Crossfade on a theme switch: lerp every token from what is on
        // screen toward the new palette over THEME_XFADE_SECS (smoothstep).
        let (shown, alpha) = THEME_XFADE.with(|cell| {
            let mut st = cell.borrow_mut();
            if st.last_id.as_deref() != Some(cur_id.as_str()) {
                if st.last_id.is_some() {
                    // begin the fade from whatever is currently painted
                    st.from = Some(shown_before);
                    st.from_alpha = st.last_alpha;
                    st.started = Some(std::time::Instant::now());
                }
                st.last_id = Some(cur_id.clone());
            }
            let (shown, alpha) = match (st.from, st.started) {
                (Some(from), Some(started)) => {
                    let raw = (started.elapsed().as_secs_f32()
                               / THEME_XFADE_SECS).clamp(0.0, 1.0);
                    if raw >= 1.0 {
                        st.from = None;
                        st.started = None;
                        (target, target_alpha)
                    } else {
                        ctx.request_repaint();
                        let t = crate::theme::smoothstep(raw);
                        let a = (st.from_alpha as f32
                                 + (target_alpha as f32 - st.from_alpha as f32)
                                   * t) as u8;
                        (from.lerp_to(&target, t), a)
                    }
                }
                _ => (target, target_alpha),
            };
            st.last_alpha = alpha;
            (shown, alpha)
        });

        shown.apply(ctx, alpha);
        self.active_palette = shown;
    }

    /// A carved, dimensional toggle for a PRIMARY control (Live, the
    /// vacuums, transport play/pause) — the "stone" treatment. Lower-
    /// tier controls stay flat (`ui.button`); depth encodes importance.
    pub(crate) fn carved_toggle(&self, ui: &mut egui::Ui, label: &str,
                                active: bool, tooltip: &str) -> bool {
        let font = egui::FontId::monospace(13.0);
        let galley = ui.painter().layout_no_wrap(
            label.to_string(), font.clone(), self.active_palette.ink);
        let desired = egui::vec2(galley.size().x + 18.0,
                                 galley.size().y + 10.0);
        let (rect, response) =
            ui.allocate_exact_size(desired, egui::Sense::click());
        let pressed = response.is_pointer_button_down_on();
        // ease the toggled-on face between stone and the accent tint;
        // the bevel strokes stay instant (tactile). animation_time (0.12 s)
        // sets the duration.
        let face_mix = ui.ctx().animate_bool(response.id, active);
        if ui.is_rect_visible(rect) {
            self.active_palette.carve_with_face(
                ui.painter(), rect, pressed, face_mix);
            let text_color = crate::theme::lerp_ink(
                self.active_palette.ink_2, self.active_palette.ink, face_mix);
            ui.painter().text(
                rect.center(), egui::Align2::CENTER_CENTER, label, font,
                text_color);
        }
        response.on_hover_text(tooltip).clicked()
    }

    /// The kit editor window: rows generated from the OPERATIONS table.
    /// Every param edit re-applies live; Save writes a .phoskit and
    /// keeps v3's quirk (a non-blank author overwrites postcard_credit).
    pub(crate) fn ui_kit_editor(&mut self, ctx: &egui::Context) {
        use phosphor_proto::phoskit;
        let Some(mut editor) = self.kit_editor.take() else { return };
        let mut text_ids = std::mem::take(&mut self.text_focus_ids);
        let mut open = true;
        let mut changed = false;
        let mut save = false;
        egui::Window::new("Kit editor")
            .collapsible(false)
            .resizable(true)
            .default_width(360.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    let response = ui.text_edit_singleline(&mut editor.name);
                    if response.has_focus() {
                        text_ids.insert(response.id);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("By");
                    let response =
                        ui.text_edit_singleline(&mut editor.author);
                    if response.has_focus() {
                        text_ids.insert(response.id);
                    }
                });
                ui.separator();

                let mut remove: Option<usize> = None;
                for index in 0..editor.stages.len() {
                    ui.push_id(index, |ui| {
                        let current_op = editor.stages[index].0.clone();
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt("op")
                                .selected_text(&current_op)
                                .show_ui(ui, |ui| {
                                    for (name, _) in phoskit::OPERATIONS {
                                        if ui.selectable_label(
                                            current_op == name, name)
                                            .clicked()
                                            && current_op != name
                                        {
                                            editor.stages[index] = (
                                                name.to_string(),
                                                phoskit::default_params(name));
                                            changed = true;
                                        }
                                    }
                                });
                            if ui.button(icon::TRASH).clicked() {
                                remove = Some(index);
                            }
                        });
                        // one labeled drag per real param of this op
                        let op = editor.stages[index].0.clone();
                        if let Some((_, table)) = phoskit::OPERATIONS.iter()
                            .find(|(name, _)| *name == op)
                        {
                            ui.label(egui::RichText::new(
                                phoskit::op_description(&op))
                                .small().color(self.active_palette.muted));
                            for (slot, (key, _, low, high)) in
                                table.iter().enumerate()
                            {
                                let value =
                                    &mut editor.stages[index].1[slot];
                                if ui.add(egui::Slider::new(
                                    value, *low..=*high).text(*key))
                                    .changed()
                                {
                                    changed = true;
                                }
                            }
                        }
                        ui.separator();
                    });
                }
                if let Some(index) = remove {
                    editor.stages.remove(index);
                    changed = true;
                }

                ui.horizontal(|ui| {
                    if ui.button(format!("{} add stage", icon::PLUS))
                        .clicked()
                        && editor.stages.len() < 16
                    {
                        editor.stages.push((
                            "rotate".into(),
                            phoskit::default_params("rotate")));
                        changed = true;
                    }
                    if ui.button(format!("{} save", icon::FLOPPY_DISK))
                        .clicked()
                        && !editor.stages.is_empty()
                    {
                        save = true;
                    }
                });
            });

        self.text_focus_ids.extend(text_ids);
        if changed {
            // live apply: write a working kit + point settings at it
            self.apply_editor_kit(&editor);
        }
        if save {
            self.save_editor_kit(&editor);
            open = false;
        }
        if open {
            self.kit_editor = Some(editor);
        }
    }

    /// Drag-dropped .phoskit: validate, install into the user kit
    /// directory, activate, and light the switch (v3
    /// phosphor_kit.install — a broken kit never lands).
    pub(crate) fn import_kit_file(&mut self, source: &std::path::Path) {
        let basename = source.file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| source.to_string_lossy().to_string());
        if let Err(error) = phosphor_proto::phoskit::load(source) {
            self.toast_now(format!(
                "kit import failed: {basename} — {error}"));
            return;
        }
        let home = std::env::var_os("HOME").unwrap_or_default();
        let directory = std::path::PathBuf::from(home)
            .join(".local/share/phosphor/kits");
        let destination = directory.join(&basename);
        let result = std::fs::create_dir_all(&directory)
            .and_then(|()| {
                if source.canonicalize().ok()
                    != destination.canonicalize().ok()
                {
                    std::fs::copy(source, &destination)?;
                }
                Ok(())
            });
        if let Err(error) = result {
            self.toast_now(format!(
                "kit import failed: {basename} — {error}"));
            return;
        }
        self.settings.kit_path =
            Some(destination.to_string_lossy().to_string());
        self.settings.kit_enabled = true;
        self.actions.push(UiAction::KitChanged);
        self.actions.push(UiAction::SaveSettings);
        self.toast_now(format!("kit installed: {basename}"));
    }

    fn editor_working_path() -> std::path::PathBuf {
        let home = std::env::var_os("HOME").unwrap_or_default();
        std::path::PathBuf::from(home)
            .join(".local/share/phosphor/kits/_editor.phoskit")
    }

    fn apply_editor_kit(&mut self, editor: &KitEditorState) {
        let path = Self::editor_working_path();
        if phosphor_proto::phoskit::save(
            &path, &editor.name, &editor.author, &editor.stages).is_ok()
        {
            self.settings.kit_path = Some(path.to_string_lossy().to_string());
            self.settings.kit_enabled = true;
            self.actions.push(UiAction::KitChanged);
        }
    }

    fn save_editor_kit(&mut self, editor: &KitEditorState) {
        let home = std::env::var_os("HOME").unwrap_or_default();
        let file = format!("{}.phoskit",
            editor.name.trim().replace(['/', ' '], "-").to_lowercase());
        let path = std::path::PathBuf::from(home)
            .join(".local/share/phosphor/kits").join(file);
        match phosphor_proto::phoskit::save(
            &path, &editor.name, &editor.author, &editor.stages)
        {
            Ok(()) => {
                // v3 quirk KEPT: a non-blank author overwrites the
                // global postcard credit.
                if !editor.author.trim().is_empty() {
                    self.settings.postcard_credit = editor.author.clone();
                }
                self.settings.kit_path =
                    Some(path.to_string_lossy().to_string());
                self.settings.kit_enabled = true;
                self.actions.push(UiAction::KitChanged);
                self.actions.push(UiAction::SaveSettings);
                let name = path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.to_string_lossy()
                        .to_string());
                self.toast_now(format!("saved {name}"));
            }
            Err(error) => self.toast_now(error),
        }
    }

    /// The "Export signal postcard…" dialog (§5.1 item 9b): title +
    /// credit, then decode the playing file → .phos with a fit-trimmed
    /// header (proto's pack_header, golden-tested).
    pub(crate) fn ui_postcard_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.postcard_dialog.take() else { return };
        let mut text_ids = std::mem::take(&mut self.text_focus_ids);
        let mut open = true;
        let mut export = false;
        egui::Window::new("Export signal postcard")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(
                    dialog.source.file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default())
                    .small().color(self.active_palette.muted));
                ui.horizontal(|ui| {
                    ui.label("Title");
                    let response = ui.text_edit_singleline(&mut dialog.title);
                    if response.has_focus() {
                        text_ids.insert(response.id);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Trace by");
                    let response =
                        ui.text_edit_singleline(&mut dialog.credit);
                    if response.has_focus() {
                        text_ids.insert(response.id);
                    }
                });
                ui.separator();
                if ui.button(format!("{} export .phos", icon::EXPORT))
                    .clicked()
                {
                    export = true;
                }
            });
        self.text_focus_ids.extend(text_ids);
        if export {
            self.export_postcard(&dialog);
            open = false;
        }
        if open {
            self.postcard_dialog = Some(dialog);
        }
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
                // Export signal postcard — non-.phos playing (§5.1-9b)
                let is_phos = self.player.playing.as_ref()
                    .is_some_and(|p| p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| e.eq_ignore_ascii_case("phos")));
                if !is_phos
                    && ui.button("Export signal postcard…").clicked()
                {
                    self.actions.push(UiAction::OpenPostcard);
                    ui.close();
                }
            }
            if !self.is_mini {
                if ui.button("Compose · draw a shape  (D)").clicked() {
                    self.actions.push(UiAction::ComposeToggle);
                    ui.close();
                }
                if self.composing && self.compose_loop_points.is_some()
                    && ui.button("Export drawing as WAV  (10 s)")
                        .clicked()
                {
                    self.actions.push(UiAction::ExportDrawing);
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
    }
}

/// A settings section header — muted, mono, letter-spaced (the
/// terminal/NFO "quiet structural label" the design system wants).
fn section(ui: &mut egui::Ui, title: &str, muted: egui::Color32) {
    ui.add_space(12.0);
    ui.label(egui::RichText::new(title).monospace().small().color(muted));
    ui.separator();
}

/// v3's add_slider: [Label][Scale][percent spin][%] with two-way sync.
/// The spin is text-capable: while it holds focus its id goes into the
/// shell's text registry so typing digits never triggers shortcuts.
struct SliderSpec<'a> {
    name: &'a str,
    minimum: f32,
    maximum: f32,
    tooltip: &'a str,
    enabled: bool,
}

fn slider_with_percent(ui: &mut egui::Ui, spec: SliderSpec, value: &mut f32,
                       text_ids: &mut std::collections::HashSet<egui::Id>)
                       -> bool {
    let SliderSpec { name, minimum, maximum, tooltip, enabled } = spec;
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
        if spin.has_focus() {
            text_ids.insert(spin.id);
        }
        if spin.on_hover_text(format!("{name} as percent — type a value"))
            .changed()
        {
            *value = minimum + (maximum - minimum) * percent / 100.0;
            changed = true;
        }
    });
    changed
}
