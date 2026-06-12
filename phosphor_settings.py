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
}
CUSTOM_THEME_NAME = "Custom"


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
        self.persistence = 0.7
        self.beam_energy = 8.0
        self.display_mode = "xy"
        self.pinned = False
        # appearance
        self.theme_name = "P7 Green"
        self.custom_beam_color = [0.42, 1.0, 0.55]
        self.custom_grid_color = [0.35, 1.0, 0.45]
        self.grid_enabled = True
        self.amoled_background = False
        self.renderer = "gl"
        # capture
        self.target_id = None

    def current_theme(self):
        if self.theme_name == CUSTOM_THEME_NAME:
            return build_custom_theme(self.custom_beam_color, self.custom_grid_color)
        theme = THEME_PRESETS.get(self.theme_name, THEME_PRESETS["P7 Green"])
        if self.amoled_background:
            return Theme(theme.beam_color, theme.flash_color, theme.grid_color, (0.0, 0.0, 0.0))
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
