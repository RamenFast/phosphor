// SPDX-License-Identifier: GPL-3.0-or-later
//! Chrome pass iii — the keyboard (UI-SPEC §1, verbatim semantics):
//! no modifier checks anywhere (Ctrl+S still snapshots — v3 quirk kept),
//! the Konami tracker runs on every keypress in every mode with the
//! partial-reset rule (a repeated Up re-arms to 1, not 0), arrow keys
//! both advance Konami AND nudge the 3D camera, and the Escape cascade
//! is compose → fullscreen → mini → close.
//!
//! The decision logic (key → command, Konami advance, the cascade) is
//! pure and pinned by the tests below; `handle_key` only performs the
//! side effects.

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

#[derive(PartialEq, Clone, Copy, Debug)]
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

/// One keypress against the tracker: (next progress, sequence fired).
/// The partial-reset rule: a mismatch re-arms to 1 if the key was the
/// sequence's own first step, else 0.
fn advance_konami(progress: usize, step: Option<KonamiStep>)
    -> (usize, bool)
{
    if step == Some(KONAMI[progress]) {
        let next = progress + 1;
        if next == KONAMI.len() {
            (0, true)
        } else {
            (next, false)
        }
    } else {
        (usize::from(step == Some(KONAMI[0])), false)
    }
}

/// The single-key table (§1.4; both cases, no modifiers) as data.
#[derive(PartialEq, Clone, Copy, Debug)]
enum KeyCommand {
    CaptureToggle,
    OpenFile,
    ComposeToggle,
    MiniToggle,
    Snapshot,
    Clip,
    PinToggle,
    GridToggle,
    PlaylistToggle,
    FpsToggle,
    Quit,
    FullscreenToggle,
    Escape,
}

fn key_command(key: &Key) -> Option<KeyCommand> {
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
        (Key::Named(NamedKey::Space), _) => Some(KeyCommand::CaptureToggle),
        (_, Some('o')) => Some(KeyCommand::OpenFile),
        (_, Some('d')) => Some(KeyCommand::ComposeToggle),
        (_, Some('m')) => Some(KeyCommand::MiniToggle),
        (_, Some('s')) => Some(KeyCommand::Snapshot),
        (_, Some('c')) => Some(KeyCommand::Clip),
        (_, Some('p')) => Some(KeyCommand::PinToggle),
        (_, Some('g')) => Some(KeyCommand::GridToggle),
        (_, Some('l')) => Some(KeyCommand::PlaylistToggle),
        (_, Some('f')) => Some(KeyCommand::FpsToggle),
        (_, Some('q')) => Some(KeyCommand::Quit),
        (Key::Named(NamedKey::F11), _) => Some(KeyCommand::FullscreenToggle),
        (Key::Named(NamedKey::Escape), _) => Some(KeyCommand::Escape),
        _ => None,
    }
}

/// The Escape cascade: compose → fullscreen → mini → close.
#[derive(PartialEq, Clone, Copy, Debug)]
enum EscapeStep {
    LeaveCompose,
    LeaveFullscreen,
    LeaveMini,
    Close,
}

fn escape_step(composing: bool, fullscreen: bool, mini: bool)
    -> EscapeStep
{
    if composing {
        EscapeStep::LeaveCompose
    } else if fullscreen {
        EscapeStep::LeaveFullscreen
    } else if mini {
        EscapeStep::LeaveMini
    } else {
        EscapeStep::Close
    }
}

impl Shell {
    /// F and the context-menu FPS item share ONE state machine:
    /// off → fps counter → nerd HUD → off (the menu used to be a
    /// bare show_fps checkbox that couldn't reach the HUD). The
    /// choice persists immediately — surviving a crash, not just a
    /// clean quit ("user preferences saved on restart").
    pub(crate) fn cycle_fps(&mut self) {
        match (self.settings.show_fps, self.settings.show_fps_detail) {
            (false, _) => {
                self.settings.show_fps = true;
                self.settings.show_fps_detail = false;
            }
            (true, false) => {
                self.settings.show_fps_detail = true;
            }
            (true, true) => {
                self.settings.show_fps = false;
                self.settings.show_fps_detail = false;
            }
        }
        self.actions.push(UiAction::SaveSettings);
    }

    pub(crate) fn handle_key(&mut self, key: &Key) -> KeyOutcome {
        // ---- Konami (always, before everything) ----
        let (progress, fired) =
            advance_konami(self.konami_progress, konami_step(key));
        self.konami_progress = progress;
        if fired {
            self.begin_visitor();
            return KeyOutcome::Handled;
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

        match key_command(key) {
            Some(KeyCommand::CaptureToggle) => {
                self.actions.push(if self.capture_on {
                    UiAction::CaptureOff
                } else {
                    UiAction::CaptureOn
                });
            }
            Some(KeyCommand::OpenFile) => {
                self.actions.push(UiAction::OpenFile);
            }
            Some(KeyCommand::ComposeToggle) => {
                self.actions.push(UiAction::ComposeToggle);
            }
            Some(KeyCommand::MiniToggle) => {
                self.actions.push(UiAction::MiniToggle);
            }
            Some(KeyCommand::Snapshot) => {
                self.actions.push(UiAction::SaveSnapshot);
            }
            Some(KeyCommand::Clip) => {
                self.actions.push(UiAction::SaveClip);
            }
            Some(KeyCommand::PinToggle) => {
                self.actions.push(UiAction::PinToggle);
            }
            Some(KeyCommand::GridToggle) => {
                self.settings.grid_enabled = !self.settings.grid_enabled;
                self.actions.push(UiAction::RenderTuning);
                self.actions.push(UiAction::SaveSettings);
            }
            Some(KeyCommand::PlaylistToggle) => {
                self.player.panel_open = !self.player.panel_open;
                self.settings.playlist_panel_open = self.player.panel_open;
            }
            Some(KeyCommand::FpsToggle) => {
                self.cycle_fps();
            }
            Some(KeyCommand::Quit) => return KeyOutcome::CloseRequested,
            Some(KeyCommand::FullscreenToggle) => {
                self.actions.push(UiAction::FullscreenToggle);
            }
            Some(KeyCommand::Escape) => {
                match escape_step(self.composing, self.is_fullscreen,
                                  self.is_mini)
                {
                    EscapeStep::LeaveCompose => {
                        self.actions.push(UiAction::ComposeToggle);
                    }
                    EscapeStep::LeaveFullscreen => {
                        self.actions.push(UiAction::FullscreenToggle);
                    }
                    EscapeStep::LeaveMini => {
                        self.actions.push(UiAction::MiniToggle);
                    }
                    EscapeStep::Close => {
                        return KeyOutcome::CloseRequested;
                    }
                }
            }
            None => return KeyOutcome::Unhandled,
        }
        KeyOutcome::Handled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::SmolStr;

    fn character(text: &str) -> Key {
        Key::Character(SmolStr::new(text))
    }

    #[test]
    fn single_key_table_is_pinned() {
        // the v3 map, both cases, no modifiers (§1.4)
        let expectations: &[(Key, KeyCommand)] = &[
            (Key::Named(NamedKey::Space), KeyCommand::CaptureToggle),
            (character("o"), KeyCommand::OpenFile),
            (character("d"), KeyCommand::ComposeToggle),
            (character("M"), KeyCommand::MiniToggle),
            (character("s"), KeyCommand::Snapshot),
            (character("C"), KeyCommand::Clip),
            (character("p"), KeyCommand::PinToggle),
            (character("g"), KeyCommand::GridToggle),
            (character("l"), KeyCommand::PlaylistToggle),
            (character("f"), KeyCommand::FpsToggle),
            (character("q"), KeyCommand::Quit),
            (Key::Named(NamedKey::F11), KeyCommand::FullscreenToggle),
            (Key::Named(NamedKey::Escape), KeyCommand::Escape),
        ];
        for (key, command) in expectations {
            assert_eq!(key_command(key), Some(*command),
                       "map broke for {key:?}");
        }
        assert_eq!(key_command(&character("z")), None);
        assert_eq!(key_command(&Key::Named(NamedKey::Enter)), None);
    }

    #[test]
    fn escape_cascade_order_is_compose_fullscreen_mini_close() {
        // compose wins even in fullscreen; fullscreen beats mini
        assert_eq!(escape_step(true, true, true),
                   EscapeStep::LeaveCompose);
        assert_eq!(escape_step(false, true, true),
                   EscapeStep::LeaveFullscreen);
        assert_eq!(escape_step(false, false, true),
                   EscapeStep::LeaveMini);
        assert_eq!(escape_step(false, false, false), EscapeStep::Close);
    }

    #[test]
    fn konami_full_sequence_fires_once() {
        let sequence = [
            Key::Named(NamedKey::ArrowUp), Key::Named(NamedKey::ArrowUp),
            Key::Named(NamedKey::ArrowDown), Key::Named(NamedKey::ArrowDown),
            Key::Named(NamedKey::ArrowLeft), Key::Named(NamedKey::ArrowRight),
            Key::Named(NamedKey::ArrowLeft), Key::Named(NamedKey::ArrowRight),
            character("b"), character("a"),
        ];
        let mut progress = 0;
        let mut fired = false;
        for key in &sequence {
            let (next, done) = advance_konami(progress, konami_step(key));
            progress = next;
            fired = done;
        }
        assert!(fired);
        assert_eq!(progress, 0); // armed again from the start
    }

    #[test]
    fn konami_partial_reset_rearms_to_one() {
        // Up Up Up: the third Up mismatches step 3 (Down) but IS the
        // sequence's first step — progress re-arms to 1, not 0
        let up = konami_step(&Key::Named(NamedKey::ArrowUp));
        let (progress, _) = advance_konami(0, up);
        let (progress, _) = advance_konami(progress, up);
        assert_eq!(progress, 2);
        let (progress, fired) = advance_konami(progress, up);
        assert!(!fired);
        assert_eq!(progress, 1);
        // and a non-sequence key resets to 0
        let (progress, _) =
            advance_konami(progress, konami_step(&character("x")));
        assert_eq!(progress, 0);
    }

    #[test]
    fn shifted_b_does_not_advance_konami() {
        // v3 keyval law: Shift+B must not match
        assert_eq!(konami_step(&character("B")), None);
        assert_eq!(konami_step(&character("b")), Some(KonamiStep::B));
    }
}
