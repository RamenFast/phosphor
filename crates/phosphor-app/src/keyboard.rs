// SPDX-License-Identifier: GPL-3.0-or-later
//! Chrome pass iii — the keyboard (UI-SPEC §1, verbatim semantics):
//! no modifier checks anywhere (Ctrl+S still snapshots — v3 quirk kept),
//! the Konami tracker runs on every keypress in every mode with the
//! partial-reset rule (a repeated Up re-arms to 1, not 0), arrow keys
//! both advance Konami AND nudge the 3D camera, and the Escape cascade
//! is compose → fullscreen → mini → close.

use winit::keyboard::{Key, NamedKey};

use crate::shell::{Shell, UiAction};

pub enum KeyOutcome {
    Handled,
    CloseRequested,
    Unhandled,
}

const KONAMI: [KonamiStep; 10] = [
    KonamiStep::Up, KonamiStep::Up, KonamiStep::Down, KonamiStep::Down,
    KonamiStep::Left, KonamiStep::Right, KonamiStep::Left,
    KonamiStep::Right, KonamiStep::B, KonamiStep::A,
];

#[derive(PartialEq, Clone, Copy)]
enum KonamiStep {
    Up,
    Down,
    Left,
    Right,
    B,
    A,
}

fn konami_step(key: &Key) -> Option<KonamiStep> {
    match key {
        Key::Named(NamedKey::ArrowUp) => Some(KonamiStep::Up),
        Key::Named(NamedKey::ArrowDown) => Some(KonamiStep::Down),
        Key::Named(NamedKey::ArrowLeft) => Some(KonamiStep::Left),
        Key::Named(NamedKey::ArrowRight) => Some(KonamiStep::Right),
        // unshifted only — Shift+B must NOT match (v3 keyval law)
        Key::Character(text) if text.as_str() == "b" => Some(KonamiStep::B),
        Key::Character(text) if text.as_str() == "a" => Some(KonamiStep::A),
        _ => None,
    }
}

impl Shell {
    pub(crate) fn handle_key(&mut self, key: &Key) -> KeyOutcome {
        // ---- Konami (always, before everything) ----
        let step = konami_step(key);
        if step == Some(KONAMI[self.konami_progress]) {
            self.konami_progress += 1;
            if self.konami_progress == KONAMI.len() {
                self.konami_progress = 0;
                self.begin_visitor();
                return KeyOutcome::Handled;
            }
        } else {
            self.konami_progress =
                usize::from(step == Some(KONAMI[0]));
        }

        // ---- 3D camera nudge (falls through from Konami, v3 law) ----
        let is_3d = matches!(self.settings.display_mode.as_str(),
                             "xyz_takens" | "helix");
        if is_3d {
            let nudge = match key {
                Key::Named(NamedKey::ArrowLeft) => Some((-0.1, 0.0)),
                Key::Named(NamedKey::ArrowRight) => Some((0.1, 0.0)),
                Key::Named(NamedKey::ArrowUp) => Some((0.0, -0.1)),
                Key::Named(NamedKey::ArrowDown) => Some((0.0, 0.1)),
                _ => None,
            };
            if let Some((yaw, pitch)) = nudge {
                self.camera_yaw += yaw;
                self.camera_pitch =
                    (self.camera_pitch + pitch).clamp(-1.45, 1.45);
                self.push_camera();
                self.mark_orbit_interaction();
                return KeyOutcome::Handled;
            }
        }

        // ---- the single-key table (§1.4; both cases, no modifiers) ----
        let character = match key {
            Key::Character(text) => {
                let mut chars = text.as_str().chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => Some(c.to_ascii_lowercase()),
                    _ => None,
                }
            }
            _ => None,
        };
        match (key, character) {
            (Key::Named(NamedKey::Space), _) => {
                self.actions.push(if self.capture_on {
                    UiAction::CaptureOff
                } else {
                    UiAction::CaptureOn
                });
            }
            (_, Some('o')) => self.actions.push(UiAction::OpenFile),
            (_, Some('d')) => self.actions.push(UiAction::ComposeToggle),
            (_, Some('m')) => self.actions.push(UiAction::MiniToggle),
            (_, Some('s')) => self.actions.push(UiAction::SaveSnapshot),
            (_, Some('c')) => self.actions.push(UiAction::SaveClip),
            (_, Some('p')) => self.actions.push(UiAction::PinToggle),
            (_, Some('g')) => {
                self.settings.grid_enabled = !self.settings.grid_enabled;
                self.actions.push(UiAction::RenderTuning);
            }
            (_, Some('l')) => {
                self.player.panel_open = !self.player.panel_open;
                self.settings.playlist_panel_open = self.player.panel_open;
            }
            (_, Some('f')) => {
                self.settings.show_fps = !self.settings.show_fps;
            }
            (_, Some('q')) => return KeyOutcome::CloseRequested,
            (Key::Named(NamedKey::F11), _) => {
                self.actions.push(UiAction::FullscreenToggle);
            }
            (Key::Named(NamedKey::Escape), _) => {
                // cascade: compose → fullscreen → mini → close
                if self.composing {
                    self.actions.push(UiAction::ComposeToggle);
                } else if self.is_fullscreen {
                    self.actions.push(UiAction::FullscreenToggle);
                } else if self.is_mini {
                    self.actions.push(UiAction::MiniToggle);
                } else {
                    return KeyOutcome::CloseRequested;
                }
            }
            _ => return KeyOutcome::Unhandled,
        }
        KeyOutcome::Handled
    }
}
