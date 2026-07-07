// SPDX-License-Identifier: GPL-3.0-or-later
//! The chrome design system (Ben's data-representation house style).
//!
//! Hard rules, from the skill — non-negotiable:
//! - **sharp corners everywhere** (`corner_radius = 0`); no pills.
//! - hairline 1 px low-opacity strokes are the frames.
//! - monospace for all DATA (values, ids, fps, time, labels).
//! - **dimensional hierarchy**: a few important controls are carved
//!   (beveled/inset — the "stone toggle" feel); everything lower-tier
//!   stays flat and boxy. Depth encodes importance; shape never
//!   changes between states, surface does.
//!
//! Six themes, all token-driven off [`Palette`]. Blossom is default.
//! egui reads only the tokens, so a theme swap is one struct.

use egui::Color32;

/// One theme = one token block. Colors are the skill's tokens verbatim
/// where a matching concept exists (plane/surface/ink/line/accent/
/// stone); the CRT-native themes (chromacore/basalt/afterglow) are
/// originals in the same shape.
#[derive(Clone, Copy)]
pub struct Palette {
    pub id: &'static str,
    pub label: &'static str,
    pub dark: bool,
    pub plane: Color32,     // window/page fill (behind panels)
    pub surface: Color32,   // panel/card face
    pub surface_2: Color32, // recessed / lower tier
    pub ink: Color32,       // primary text
    pub ink_2: Color32,     // secondary text
    pub muted: Color32,     // faint text / disabled
    pub line: Color32,      // hairline frame
    pub line_strong: Color32,
    pub accent: Color32,    // the one bold hue (selection, active)
    pub on_accent: Color32,
    // carved-stone triple for dimensional controls
    pub stone: Color32,
    pub stone_hi: Color32,
    pub stone_lo: Color32,
    /// afterglow samples the live beam color into `accent` at runtime.
    pub accent_follows_beam: bool,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}
const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    Color32::from_rgba_premultiplied(r, g, b, a)
}

/// Menu order. Blossom Dark is THE default (wired in settings since
/// the regalia wave); the last four are the distinct-visual-system
/// additions — bevel city, true black, warm paper, amber CRT — so
/// switching themes changes the ROOM, not just the paint.
pub const PALETTES: [Palette; 12] = [
    // ── Blossom — warm sakura, rice-paper, ink (skill default) ──
    Palette {
        id: "blossom", label: "Blossom", dark: false,
        plane: rgb(0xf3, 0xe6, 0xe6), surface: rgb(0xfc, 0xf4, 0xf3),
        surface_2: rgb(0xf6, 0xe9, 0xe9),
        ink: rgb(0x2b, 0x21, 0x28), ink_2: rgb(0x5f, 0x4f, 0x57),
        muted: rgb(0x9c, 0x88, 0x90),
        line: rgba(43, 33, 40, 41), line_strong: rgba(43, 33, 40, 77),
        accent: rgb(0xc8, 0x5a, 0x7c), on_accent: rgb(0xff, 0xf7, 0xf9),
        stone: rgb(0xe7, 0xda, 0xd9), stone_hi: rgb(0xff, 0xfa, 0xfa),
        stone_lo: rgb(0xc9, 0xb6, 0xb8),
        accent_follows_beam: false,
    },
    // ── Blossom Dark — warm wine-plum ground, sakura-rose accent
    //    that breathes with the live beam (afterglow fusion). This
    //    is deliberately WARM (wine/plum), not the cool plum-black
    //    of `dark`. The wanted dark default. ──
    Palette {
        id: "blossom_dark", label: "Blossom Dark", dark: true,
        plane: rgb(0x1c, 0x10, 0x16), surface: rgb(0x28, 0x18, 0x21),
        surface_2: rgb(0x33, 0x21, 0x2c),
        ink: rgb(0xf5, 0xea, 0xef), ink_2: rgb(0xc9, 0xb0, 0xbc),
        muted: rgb(0x91, 0x79, 0x86),
        line: rgba(244, 233, 238, 36), line_strong: rgba(244, 233, 238, 82),
        // brighter, warmer sakura than the cool `dark` (which it must
        // not read like); it also breathes with the live beam
        accent: rgb(0xec, 0x8f, 0xac), on_accent: rgb(0x1a, 0x0e, 0x14),
        // carved wine-stone: warm catch-light, deep plum shadow
        stone: rgb(0x3b, 0x26, 0x31), stone_hi: rgb(0x55, 0x39, 0x48),
        stone_lo: rgb(0x1d, 0x11, 0x17),
        accent_follows_beam: true,
    },
    // ── Light — cool neutral instrument ──
    Palette {
        id: "light", label: "Light", dark: false,
        plane: rgb(0xea, 0xee, 0xf2), surface: rgb(0xff, 0xff, 0xff),
        surface_2: rgb(0xf5, 0xf8, 0xfa),
        ink: rgb(0x0e, 0x16, 0x20), ink_2: rgb(0x43, 0x51, 0x5e),
        muted: rgb(0x7a, 0x88, 0x94),
        line: rgba(14, 22, 32, 31), line_strong: rgba(14, 22, 32, 71),
        accent: rgb(0x0c, 0x94, 0xa2), on_accent: rgb(0xff, 0xff, 0xff),
        stone: rgb(0xe4, 0xe9, 0xee), stone_hi: rgb(0xff, 0xff, 0xff),
        stone_lo: rgb(0xc2, 0xcc, 0xd4),
        accent_follows_beam: false,
    },
    // ── Dark — deep instrument, plum-tinted near-black ──
    Palette {
        id: "dark", label: "Dark", dark: true,
        plane: rgb(0x0a, 0x08, 0x10), surface: rgb(0x14, 0x10, 0x19),
        surface_2: rgb(0x1b, 0x15, 0x22),
        ink: rgb(0xf0, 0xea, 0xf0), ink_2: rgb(0xb3, 0xa6, 0xb3),
        muted: rgb(0x7d, 0x6f, 0x7d),
        line: rgba(240, 234, 240, 31), line_strong: rgba(240, 234, 240, 66),
        accent: rgb(0xe7, 0x8a, 0xa6), on_accent: rgb(0x16, 0x08, 0x10),
        stone: rgb(0x24, 0x1d, 0x29), stone_hi: rgb(0x33, 0x28, 0x38),
        stone_lo: rgb(0x14, 0x0f, 0x18),
        accent_follows_beam: false,
    },
    // ── Chromacore — terminal/NFO: near-black, cyan, structural ──
    Palette {
        id: "chromacore", label: "Chromacore", dark: true,
        plane: rgb(0x08, 0x08, 0x10), surface: rgb(0x0d, 0x0d, 0x16),
        surface_2: rgb(0x12, 0x12, 0x1e),
        ink: rgb(0xe8, 0xe8, 0xf0), ink_2: rgb(0xa8, 0xb4, 0xc4),
        muted: rgb(0x60, 0x6a, 0x7e),
        line: rgba(0, 229, 255, 28), line_strong: rgba(0, 229, 255, 64),
        accent: rgb(0x00, 0xe5, 0xff), on_accent: rgb(0x03, 0x0a, 0x0e),
        stone: rgb(0x10, 0x18, 0x1e), stone_hi: rgb(0x1a, 0x2a, 0x30),
        stone_lo: rgb(0x06, 0x0c, 0x10),
        accent_follows_beam: false,
    },
    // ── Basalt — carved stone: strata grays, mica-glint accent ──
    Palette {
        id: "basalt", label: "Basalt", dark: true,
        plane: rgb(0x17, 0x17, 0x19), surface: rgb(0x22, 0x22, 0x25),
        surface_2: rgb(0x1a, 0x1a, 0x1d),
        ink: rgb(0xdb, 0xd7, 0xce), ink_2: rgb(0x9a, 0x96, 0x8e),
        muted: rgb(0x66, 0x63, 0x5d),
        line: rgba(0, 0, 0, 110), line_strong: rgba(0, 0, 0, 150),
        accent: rgb(0x9c, 0xb4, 0xc9), on_accent: rgb(0x10, 0x12, 0x15),
        // real stonemasonry: high catch-light, deep shadow, cool basalt
        stone: rgb(0x2c, 0x2c, 0x30), stone_hi: rgb(0x4a, 0x4a, 0x50),
        stone_lo: rgb(0x0e, 0x0e, 0x10),
        accent_follows_beam: false,
    },
    // ── Afterglow — CRT panel whose chrome remembers the beam ──
    Palette {
        id: "afterglow", label: "Afterglow", dark: true,
        plane: rgb(0x05, 0x06, 0x07), surface: rgb(0x0b, 0x0d, 0x0e),
        surface_2: rgb(0x10, 0x13, 0x14),
        ink: rgb(0xd6, 0xe0, 0xdc), ink_2: rgb(0x8c, 0x9c, 0x96),
        muted: rgb(0x55, 0x62, 0x5d),
        line: rgba(255, 255, 255, 20), line_strong: rgba(255, 255, 255, 46),
        accent: rgb(0x63, 0xff, 0xb0), on_accent: rgb(0x02, 0x08, 0x05),
        stone: rgb(0x12, 0x16, 0x15), stone_hi: rgb(0x20, 0x28, 0x25),
        stone_lo: rgb(0x06, 0x0a, 0x08),
        accent_follows_beam: true,
    },
    // ── Stonework 95 — the v3 cult favorite reborn: warm platinum
    //    chrome, navy accent, and the LOUDEST stone triple in the
    //    table (near-white catch-light over deep slate shadow) so
    //    every bevel reads like a machine from 1995. ──
    Palette {
        id: "stonework95", label: "Stonework 95", dark: false,
        plane: rgb(0xc8, 0xc5, 0xbd), surface: rgb(0xd9, 0xd6, 0xce),
        surface_2: rgb(0xc3, 0xc0, 0xb8),
        ink: rgb(0x1a, 0x1a, 0x1f), ink_2: rgb(0x45, 0x45, 0x4d),
        muted: rgb(0x6e, 0x6e, 0x76),
        line: rgba(26, 26, 31, 64), line_strong: rgba(26, 26, 31, 115),
        accent: rgb(0x20, 0x32, 0x8c), on_accent: rgb(0xf2, 0xf2, 0xf7),
        stone: rgb(0xd4, 0xd0, 0xc8), stone_hi: rgb(0xff, 0xff, 0xfb),
        stone_lo: rgb(0x86, 0x83, 0x7c),
        accent_follows_beam: false,
    },
    // ── AMOLED — true #000, hot pink, maximum contrast: the panel
    //    disappears and only light remains (v3's AMOLED pink nod). ──
    Palette {
        id: "amoled", label: "AMOLED", dark: true,
        plane: rgb(0x00, 0x00, 0x00), surface: rgb(0x00, 0x00, 0x00),
        surface_2: rgb(0x0d, 0x0d, 0x0d),
        ink: rgb(0xff, 0xff, 0xff), ink_2: rgb(0xc4, 0xc4, 0xc4),
        muted: rgb(0x8a, 0x8a, 0x8a),
        line: rgba(255, 255, 255, 46), line_strong: rgba(255, 255, 255, 92),
        accent: rgb(0xff, 0x2d, 0x7e), on_accent: rgb(0xff, 0xff, 0xff),
        stone: rgb(0x14, 0x14, 0x14), stone_hi: rgb(0x33, 0x33, 0x33),
        stone_lo: rgb(0x00, 0x00, 0x00),
        accent_follows_beam: false,
    },
    // ── Paper — warm cream, espresso ink, a vermilion seal: the
    //    scope as a printed instrument sheet. Reads in sunlight. ──
    Palette {
        id: "paper", label: "Paper", dark: false,
        plane: rgb(0xf2, 0xec, 0xdf), surface: rgb(0xfa, 0xf6, 0xec),
        surface_2: rgb(0xec, 0xe5, 0xd6),
        ink: rgb(0x2e, 0x28, 0x20), ink_2: rgb(0x5c, 0x52, 0x44),
        muted: rgb(0x8f, 0x84, 0x72),
        line: rgba(46, 40, 32, 46), line_strong: rgba(46, 40, 32, 92),
        accent: rgb(0xc3, 0x3d, 0x2e), on_accent: rgb(0xfd, 0xf9, 0xf2),
        stone: rgb(0xe8, 0xe0, 0xd0), stone_hi: rgb(0xff, 0xfd, 0xf6),
        stone_lo: rgb(0xc0, 0xb5, 0xa0),
        accent_follows_beam: false,
    },
    // ── CRT Amber — the P3 phosphor homage: lamp-black chassis, every
    //    tone an amber temperature. Monochrome discipline: even ink is
    //    banked-fire amber, so the room glows like 1979. ──
    Palette {
        id: "amber", label: "CRT Amber", dark: true,
        plane: rgb(0x0e, 0x08, 0x02), surface: rgb(0x17, 0x0e, 0x04),
        surface_2: rgb(0x20, 0x14, 0x06),
        ink: rgb(0xff, 0xc9, 0x66), ink_2: rgb(0xc9, 0x96, 0x42),
        muted: rgb(0x8a, 0x66, 0x2e),
        line: rgba(255, 176, 0, 38), line_strong: rgba(255, 176, 0, 84),
        accent: rgb(0xff, 0xb0, 0x00), on_accent: rgb(0x1a, 0x0f, 0x00),
        stone: rgb(0x24, 0x17, 0x08), stone_hi: rgb(0x45, 0x2d, 0x10),
        stone_lo: rgb(0x08, 0x05, 0x01),
        accent_follows_beam: false,
    },
    // ── Fable — the model that built v4 signs the guestbook: a
    //    storyteller's room. Abyssal sea-green ground (the deep the
    //    turtle swims), moonlit ink, seafoam hairlines, and a warm
    //    LANTERN-GOLD accent that holds its own light instead of
    //    following the beam — the beam paints the scope, the lantern
    //    lights the margins. Shell-carved stone triple. 🐢 ──
    Palette {
        id: "fable", label: "Fable", dark: true,
        plane: rgb(0x0a, 0x14, 0x11), surface: rgb(0x11, 0x1e, 0x1a),
        surface_2: rgb(0x17, 0x28, 0x22),
        ink: rgb(0xea, 0xf2, 0xec), ink_2: rgb(0xad, 0xc4, 0xb8),
        muted: rgb(0x6e, 0x87, 0x7b),
        line: rgba(158, 232, 200, 34),
        line_strong: rgba(158, 232, 200, 72),
        accent: rgb(0xea, 0xc2, 0x79), on_accent: rgb(0x14, 0x1a, 0x10),
        stone: rgb(0x1c, 0x2e, 0x27), stone_hi: rgb(0x2e, 0x46, 0x3b),
        stone_lo: rgb(0x0c, 0x16, 0x12),
        accent_follows_beam: false,
    },
];

pub fn palette(id: &str) -> Palette {
    PALETTES.iter().copied().find(|p| p.id == id).unwrap_or(PALETTES[0])
}

impl Palette {
    /// Blend a beam color into the accent (afterglow only).
    pub fn with_beam(mut self, beam: [f32; 3]) -> Palette {
        if self.accent_follows_beam {
            let lift = |c: f32| (c.powf(1.0 / 2.2).clamp(0.0, 1.0)
                                 * 255.0) as u8;
            // keep it luminous but not blown — 82% toward the beam hue
            let mix = |beam_channel: u8, base: u8| {
                ((beam_channel as f32 * 0.82 + base as f32 * 0.18)
                 as u8).max(base)
            };
            let b = [lift(beam[0]), lift(beam[1]), lift(beam[2])];
            self.accent = Color32::from_rgb(
                mix(b[0], 0x30), mix(b[1], 0x40), mix(b[2], 0x38));
        }
        self
    }

    /// Translate the tokens into an egui `Style`: sharp corners,
    /// hairline frames, the accent on selection, panel fills. `alpha`
    /// dims panels when glass is on (chrome floats over the desktop).
    pub fn apply(&self, ctx: &egui::Context, panel_alpha: u8) {
        let mut visuals = if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };

        // sharp corners EVERYWHERE (the hard rule)
        let sharp = egui::CornerRadius::ZERO;
        visuals.window_corner_radius = sharp;
        visuals.menu_corner_radius = sharp;

        let panel = with_alpha(self.surface, panel_alpha);
        visuals.panel_fill = panel;
        visuals.window_fill = with_alpha(self.surface, panel_alpha);
        visuals.window_stroke = egui::Stroke::new(1.0, self.line_strong);
        // the plane sits behind panels (menu shadows, gaps)
        visuals.extreme_bg_color = self.plane;
        visuals.faint_bg_color = self.surface_2;
        // NO override_text_color: it forced `ink` onto accent-filled
        // selection rows too (ink-on-pink — the theme-picker clash).
        // Text color flows from each widget's fg_stroke; selected rows
        // get `on_accent` via selection.stroke below.
        visuals.override_text_color = None;
        visuals.weak_text_alpha = 0.7; // was 0.55 — part of "hard to read"
        visuals.hyperlink_color = self.accent;
        visuals.selection.bg_fill = self.accent;
        visuals.selection.stroke = egui::Stroke::new(1.0, self.on_accent);

        // Flat, hairline-framed widgets — but more *defined* than a bare
        // hairline: resting buttons carry a line_strong frame and read as
        // faintly raised stone vs the recessed panel; hover pulls an
        // accent-tinted frame; active goes full accent. Depth (bevels)
        // stays reserved for the few carved controls (chrome.rs) — these
        // stay flat-tier, only their surface and frame move.
        let hairline = egui::Stroke::new(1.0, self.line);
        let hairline_strong = egui::Stroke::new(1.0, self.line_strong);
        // a faintly-raised stone face so buttons separate from panels
        let button_face = lerp(self.surface_2, self.stone, 0.35);
        let widgets = &mut visuals.widgets;
        for w in [&mut widgets.noninteractive, &mut widgets.inactive,
                  &mut widgets.hovered, &mut widgets.active,
                  &mut widgets.open] {
            w.corner_radius = sharp;
        }
        widgets.noninteractive.bg_fill = panel;
        widgets.noninteractive.weak_bg_fill = panel;
        widgets.noninteractive.bg_stroke = hairline;
        // labels read PRIMARY (ink) — secondary text opts in via
        // RichText(ink_2/muted), never the other way around
        widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, self.ink);

        widgets.inactive.bg_fill = button_face;
        widgets.inactive.weak_bg_fill = self.surface_2;
        widgets.inactive.bg_stroke = hairline_strong;
        widgets.inactive.fg_stroke = egui::Stroke::new(1.0, self.ink);

        widgets.hovered.bg_fill = lerp(button_face, self.accent, 0.14);
        widgets.hovered.weak_bg_fill =
            lerp(self.surface_2, self.accent, 0.14);
        widgets.hovered.bg_stroke =
            egui::Stroke::new(1.0, lerp(self.line_strong, self.accent, 0.5));
        widgets.hovered.fg_stroke = egui::Stroke::new(1.0, self.ink);
        widgets.hovered.expansion = 1.0;

        widgets.active.bg_fill = lerp(self.surface_2, self.accent, 0.30);
        widgets.active.weak_bg_fill =
            lerp(self.surface_2, self.accent, 0.30);
        widgets.active.bg_stroke = egui::Stroke::new(1.0, self.accent);
        widgets.active.fg_stroke = egui::Stroke::new(1.0, self.ink);
        widgets.active.expansion = -0.5;

        widgets.open.bg_fill = self.surface_2;
        widgets.open.bg_stroke = hairline_strong;

        ctx.set_visuals(visuals);

        // Spacing + the mono data face: install a Style with tightened,
        // consistent spacing (breathing room without sprawl).
        let mut style = (*ctx.style()).clone();
        // free eased hover/active crossfades on every widget (restrained,
        // ~120 ms — the house-style "beautiful but purposeful" motion).
        style.animation_time = 0.12;
        style.spacing.button_padding = egui::vec2(8.0, 4.0);
        style.spacing.item_spacing = egui::vec2(7.0, 5.0);
        style.spacing.window_margin = egui::Margin::same(8);
        // the type scale: Plex body at a readable size (egui default
        // was 14 in a thin face), mono for data, Medium for headings
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::proportional(14.5));
        style.text_styles.insert(
            egui::TextStyle::Button,
            egui::FontId::proportional(14.5));
        style.text_styles.insert(
            egui::TextStyle::Small,
            egui::FontId::proportional(11.5));
        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::new(
                16.5, egui::FontFamily::Name("plex-medium".into())));
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::monospace(12.5));
        ctx.set_style(style);
    }

    /// Paint a carved, dimensional control background into `rect` (the
    /// "stone" treatment — bevel light top-left, shadow bottom-right;
    /// pressed = it sinks). For the FEW important controls only. The
    /// toggled-on look eases in via `face_mix`
    /// (0 = resting stone, 1 = fully accent-tinted "on"). The face color
    /// and accent rim fade with `face_mix`; the bevel strokes stay
    /// instant (tactile). Callers pass an animated bool for a smooth,
    /// non-snapping press/active transition.
    pub fn carve_with_face(&self, painter: &egui::Painter, rect: egui::Rect,
                           pressed: bool, face_mix: f32) {
        let t = face_mix.clamp(0.0, 1.0);
        let on_face = lerp(self.stone, self.accent, 0.34);
        let face = lerp(self.stone, on_face, t);
        painter.rect_filled(rect, 0.0, face);
        let hi = egui::Stroke::new(1.0, self.stone_hi);
        let lo = egui::Stroke::new(1.0, self.stone_lo);
        let (top_left, bottom_right) = if pressed { (lo, hi) } else { (hi, lo) };
        // top + left = catch-light; bottom + right = shadow
        painter.line_segment([rect.left_top(), rect.right_top()], top_left);
        painter.line_segment([rect.left_top(), rect.left_bottom()], top_left);
        painter.line_segment(
            [rect.left_bottom(), rect.right_bottom()], bottom_right);
        painter.line_segment(
            [rect.right_top(), rect.right_bottom()], bottom_right);
        if t > 0.003 {
            // a hairline accent rim marks it "on" — fades with the face
            let rim = self.accent.gamma_multiply(t);
            painter.rect_stroke(
                rect.shrink(1.0), 0.0,
                egui::Stroke::new(1.0, rim),
                egui::StrokeKind::Inside);
        }
    }

    /// Blend every color token from `self` toward `other` by `t`
    /// (0 = self, 1 = other). Used for the theme-switch crossfade;
    /// non-color identity fields (`id`/`label`/`dark`/beam flag) take
    /// the destination (`other`).
    pub fn lerp_to(&self, other: &Palette, t: f32) -> Palette {
        let l = |a: Color32, b: Color32| lerp(a, b, t);
        Palette {
            id: other.id, label: other.label, dark: other.dark,
            plane: l(self.plane, other.plane),
            surface: l(self.surface, other.surface),
            surface_2: l(self.surface_2, other.surface_2),
            ink: l(self.ink, other.ink),
            ink_2: l(self.ink_2, other.ink_2),
            muted: l(self.muted, other.muted),
            line: lerp_rgba(self.line, other.line, t),
            line_strong: lerp_rgba(self.line_strong, other.line_strong, t),
            accent: l(self.accent, other.accent),
            on_accent: l(self.on_accent, other.on_accent),
            stone: l(self.stone, other.stone),
            stone_hi: l(self.stone_hi, other.stone_hi),
            stone_lo: l(self.stone_lo, other.stone_lo),
            accent_follows_beam: other.accent_follows_beam,
        }
    }
}

fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    if alpha == 255 {
        color
    } else {
        Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(),
                                        alpha)
    }
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()),
                      mix(a.b(), b.b()))
}

/// Lerp including the (premultiplied) alpha channel — for the hairline
/// `line`/`line_strong` tokens, which carry a low opacity.
fn lerp_rgba(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t) as u8;
    Color32::from_rgba_premultiplied(mix(a.r(), b.r()), mix(a.g(), b.g()),
                                     mix(a.b(), b.b()), mix(a.a(), b.a()))
}

/// Public RGB lerp for callers easing a text/ink color (chrome.rs).
pub fn lerp_ink(a: Color32, b: Color32, t: f32) -> Color32 {
    lerp(a, b, t)
}

/// Smoothstep ease for the theme crossfade (t·t·(3−2t)).
pub fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twelve_palettes_blossom_family_first() {
        assert_eq!(PALETTES.len(), 12);
        // the guestbook signature: Fable is the twelfth room, dark,
        // and its lantern accent holds its own light (no beam follow)
        assert_eq!(PALETTES[11].id, "fable");
        assert!(palette("fable").dark);
        assert!(!palette("fable").accent_follows_beam);
        assert_eq!(PALETTES[0].id, "blossom");
        assert!(!PALETTES[0].dark, "blossom is a warm light theme");
        // Blossom Dark sits right after blossom AND is the settings
        // default (the wanted default, actually wired now)
        assert_eq!(PALETTES[1].id, "blossom_dark");
        assert_eq!(
            phosphor_proto::settings::Settings::default().ui_style,
            "blossom_dark");
        // the distinct four exist and split 2 light / 2 dark
        assert!(!palette("stonework95").dark);
        assert!(palette("amoled").dark);
        assert!(!palette("paper").dark);
        assert!(palette("amber").dark);
        // AMOLED means TRUE black
        assert_eq!(palette("amoled").plane, Color32::from_rgb(0, 0, 0));
        // Stonework's bevel range is the loudest in the table
        let range = |p: &Palette| {
            (p.stone_hi.r() as i32 - p.stone_lo.r() as i32).abs()
        };
        let stonework = range(&palette("stonework95"));
        assert!(PALETTES.iter().all(|p| range(p) <= stonework),
                "stonework95 must carry the strongest bevel");
    }

    #[test]
    fn blossom_dark_is_warm_afterglow_and_unique() {
        let bd = palette("blossom_dark");
        assert_eq!(bd.id, "blossom_dark");
        assert!(bd.dark, "blossom_dark is a dark theme");
        assert!(bd.accent_follows_beam,
                "blossom_dark breathes with the live beam");
        // the accent must actually move with the beam
        assert_ne!(bd.accent, bd.with_beam([0.2, 1.0, 0.4]).accent);
        // distinctly WARMer ground than the cool `dark` (more red than blue)
        assert!(bd.plane.r() > bd.plane.b(),
                "blossom_dark ground is warm wine, not cool plum-black");
        // all ids unique across the table
        let mut ids: Vec<&str> = PALETTES.iter().map(|p| p.id).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "palette ids must be unique");
    }

    #[test]
    fn afterglow_accent_follows_beam() {
        let base = palette("afterglow").accent;
        let lit = palette("afterglow").with_beam([0.42, 1.0, 0.55]).accent;
        assert_ne!(base, lit, "afterglow chrome must remember the beam");
        // a non-following theme ignores the beam
        assert_eq!(palette("basalt").accent,
                   palette("basalt").with_beam([1.0, 0.0, 0.0]).accent);
    }

    #[test]
    fn unknown_id_falls_back_to_blossom() {
        assert_eq!(palette("no-such-theme").id, "blossom");
        // old v3 ids resolve to blossom too (migration)
        assert_eq!(palette("stone").id, "blossom");
        assert_eq!(palette("aero").id, "blossom");
    }
}
