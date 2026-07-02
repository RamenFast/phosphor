# SPDX-License-Identifier: GPL-3.0-or-later
"""Themes and persistent settings for Phosphor.

Settings live in ~/.config/phosphor/settings.json so the scope remembers
the views you like: window geometry, mini-mode size and position, sliders,
theme, renderer, capture target — everything restores on launch.
"""

import json
import os

SETTINGS_PATH = os.path.expanduser("~/.config/phosphor/settings.json")


class Theme:
    """Colors for one scope look.

    beam_color  — the phosphor glow (slow decay layer)
    flash_color — where the beam is hitting right now (fast decay layer);
                  real P7 phosphor fluoresces blue-white, then the
                  phosphorescence decays through the glow color.
    """

    def __init__(self, beam_color, flash_color, grid_color, background_color):
        self.beam_color = beam_color
        self.flash_color = flash_color
        self.grid_color = grid_color
        self.background_color = background_color


THEME_PRESETS = {
    "P7 Green": Theme(
        beam_color=(0.42, 1.0, 0.55), flash_color=(0.72, 0.85, 1.0),
        grid_color=(0.35, 1.0, 0.45), background_color=(0.013, 0.022, 0.015),
    ),
    "Amber": Theme(
        beam_color=(1.0, 0.62, 0.12), flash_color=(1.0, 0.93, 0.65),
        grid_color=(1.0, 0.62, 0.12), background_color=(0.028, 0.016, 0.0),
    ),
    "Ice Blue": Theme(
        beam_color=(0.35, 0.75, 1.0), flash_color=(0.85, 0.94, 1.0),
        grid_color=(0.35, 0.75, 1.0), background_color=(0.0, 0.015, 0.03),
    ),
    "White": Theme(
        beam_color=(0.92, 0.95, 1.0), flash_color=(1.0, 1.0, 1.0),
        grid_color=(0.75, 0.8, 0.85), background_color=(0.016, 0.016, 0.02),
    ),
    "Vaporwave": Theme(
        beam_color=(1.0, 0.30, 0.88), flash_color=(0.65, 0.95, 1.0),
        grid_color=(0.55, 0.40, 0.95), background_color=(0.02, 0.0, 0.03),
    ),
    "Red Phosphor": Theme(
        beam_color=(1.0, 0.22, 0.16), flash_color=(1.0, 0.82, 0.70),
        grid_color=(1.0, 0.28, 0.22), background_color=(0.03, 0.004, 0.0),
    ),
    "Ultraviolet": Theme(
        beam_color=(0.62, 0.40, 1.0), flash_color=(0.86, 0.80, 1.0),
        grid_color=(0.58, 0.40, 1.0), background_color=(0.014, 0.0, 0.03),
    ),
    "Solar Gold": Theme(
        beam_color=(1.0, 0.84, 0.30), flash_color=(1.0, 1.0, 0.86),
        grid_color=(0.92, 0.76, 0.30), background_color=(0.026, 0.018, 0.0),
    ),
    "Cyan Tube": Theme(
        beam_color=(0.20, 1.0, 0.92), flash_color=(0.85, 1.0, 1.0),
        grid_color=(0.22, 0.90, 0.85), background_color=(0.0, 0.024, 0.026),
    ),
}
CUSTOM_THEME_NAME = "Custom"


def grid_spacing_fraction(gain):
    """Screen fraction of one graticule division.

    A division represents a fixed signal amplitude (1/4 of full deflection at
    unity gain), so the grid grows and shrinks as you zoom. Like a real
    scope's volts/div switch, it steps by octaves to stay readable instead of
    collapsing to mush or stretching off screen.
    """
    fraction = 0.45 * max(0.001, gain) / 4.0
    while fraction < 0.05:
        fraction *= 2.0
    while fraction > 0.30:
        fraction /= 2.0
    return fraction


def build_custom_theme(beam_color, grid_color):
    """Derive a full theme from just the two user-picked colors."""
    flash_color = tuple(min(1.0, channel * 0.4 + 0.6) for channel in beam_color)
    background_color = tuple(channel * 0.03 for channel in beam_color)
    return Theme(tuple(beam_color), flash_color, tuple(grid_color), background_color)


class Settings:
    def __init__(self):
        # window memory
        self.window_width = 980
        self.window_height = 640
        self.window_x = None
        self.window_y = None
        self.start_in_mini = False
        self.mini_size = 280
        self.mini_x = None
        self.mini_y = None
        # scope controls
        self.gain = 1.0
        self.auto_gain = False         # autosize: fit the trace to the screen
        self.persistence = 0.7
        self.beam_energy = 8.0
        self.beam_focus = 1.6          # beam sigma in pixels: lower = sharper
        self.display_mode = "xy"
        # scope feed rate: higher rates trace the true inter-sample curves
        # (sinc reconstruction via PulseAudio/ffmpeg) instead of straight
        # lines between 48 kHz samples — finer detail on scope music
        self.scope_sample_rate = 96000
        # precompute scope streams to disk on open, and play from them
        self.precompute_enabled = False
        self.compose_frequency_hz = 80.0   # loop pitch for drawn shapes
        self.pinned = False
        # player
        self.show_now_playing = True   # fading artist/title overlay
        self.playback_volume = 1.0     # file playback stream volume, 0..1
        self.shuffle = False
        self.repeat_mode = "off"       # "off" | "all" | "one"
        self.playlist_panel_open = False
        # appearance
        self.theme_name = "P7 Green"
        self.custom_beam_color = [0.42, 1.0, 0.55]
        self.custom_grid_color = [0.35, 1.0, 0.45]
        self.grid_enabled = True
        self.amoled_background = False
        self.scope_glass = False       # translucent scope pane (glass over desktop)
        self.renderer = "gl"
        self.gl_supersample = 1        # GPU energy buffer scale: 1 or 2
        self.cairo_resolution = 1.0    # CPU phosphor buffer scale: 0.5..1.0
        self.ui_style = "dark"         # "system" | "dark" | "black"
        self.show_pin_button = True
        self.show_fps = False
        self.max_fps = 0               # 0 = uncapped (monitor refresh rate)
        # capture
        self.target_id = None

    def current_theme(self):
        if self.theme_name == CUSTOM_THEME_NAME:
            theme = build_custom_theme(self.custom_beam_color,
                                       self.custom_grid_color)
        else:
            theme = THEME_PRESETS.get(self.theme_name, THEME_PRESETS["P7 Green"])
        # AMOLED applies to every theme, custom included
        if self.amoled_background:
            return Theme(theme.beam_color, theme.flash_color,
                         theme.grid_color, (0.0, 0.0, 0.0))
        return theme

    @classmethod
    def load(cls):
        settings = cls()
        try:
            with open(SETTINGS_PATH) as settings_file:
                stored = json.load(settings_file)
        except (OSError, ValueError):
            return settings
        for key, value in stored.items():
            if hasattr(settings, key):
                setattr(settings, key, value)
        return settings

    def save(self):
        os.makedirs(os.path.dirname(SETTINGS_PATH), exist_ok=True)
        try:
            with open(SETTINGS_PATH, "w") as settings_file:
                json.dump(self.__dict__, settings_file, indent=2)
        except OSError:
            pass
