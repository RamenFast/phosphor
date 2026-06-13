# SPDX-License-Identifier: GPL-3.0-or-later
"""CPU (cairo) renderer with two-layer P7 phosphor simulation.

Real P7 phosphor fluoresces blue-white where the electron beam lands, then
phosphoresces in green as it decays. We model that with two energy layers:
a fast-decaying "flash" layer and a slow-decaying "glow" layer, both stored
as pure intensity (A8) and tinted with the theme's flash/beam colors only
when composited — so theme changes apply to light already on screen, the
way changing nothing but your tinted glasses would.

The energy layers can run below screen resolution (`resolution` 0.5..1.0)
to trade sharpness for speed; compositing scales them back up smoothly.

The same core renders live frames and offline export frames, so saved clips
look exactly like the screen.
"""

import cairo

try:
    import numpy
except ImportError:
    numpy = None

ALPHA_BUCKET_COUNT = 10
FLASH_DECAY_ALPHA = 0.50   # fraction of the flash layer removed every frame
FLASH_STAMP_STRENGTH = 0.9
GLOW_STAMP_STRENGTH = 0.8


class CairoBeamCore:
    """Owns the energy layers and knows how to advance and composite them."""

    def __init__(self):
        self.width = 0              # logical (widget) size
        self.height = 0
        self.resolution = 1.0       # energy buffer scale relative to logical
        self.buffer_width = 0
        self.buffer_height = 0
        self.beam_focus = 1.6       # beam sigma in logical pixels
        self.flash_layer = None
        self.glow_layer = None
        self.fresh_strokes = None

    def ensure_size(self, width, height, resolution=None):
        if resolution is not None and resolution != self.resolution:
            self.resolution = resolution
            self.flash_layer = None
        if self.flash_layer is None or width != self.width or height != self.height:
            self.width, self.height = width, height
            self.buffer_width = max(2, int(width * self.resolution))
            self.buffer_height = max(2, int(height * self.resolution))
            self.flash_layer = cairo.ImageSurface(
                cairo.FORMAT_A8, self.buffer_width, self.buffer_height)
            self.glow_layer = cairo.ImageSurface(
                cairo.FORMAT_A8, self.buffer_width, self.buffer_height)
            self.fresh_strokes = cairo.ImageSurface(
                cairo.FORMAT_A8, self.buffer_width, self.buffer_height)

    def advance(self, segments, persistence):
        """Decay both layers, then stamp this frame's beam energy into them."""
        glow_decay_alpha = max(0.02, (1.0 - persistence) * 0.6)
        for layer, decay_alpha in ((self.flash_layer, FLASH_DECAY_ALPHA),
                                   (self.glow_layer, glow_decay_alpha)):
            context = cairo.Context(layer)
            context.set_operator(cairo.OPERATOR_DEST_OUT)
            context.set_source_rgba(0, 0, 0, decay_alpha)
            context.paint()

        if segments is None or len(segments) == 0:
            return

        scratch = cairo.Context(self.fresh_strokes)
        scratch.set_operator(cairo.OPERATOR_CLEAR)
        scratch.paint()
        scratch.set_operator(cairo.OPERATOR_ADD)
        # segments come in logical pixels; draw into the (possibly smaller)
        # energy buffer by scaling the context
        scratch.scale(self.resolution, self.resolution)
        scratch.set_line_width(max(0.8, self.beam_focus * 1.25))
        scratch.set_line_cap(cairo.LINE_CAP_ROUND)

        if numpy is not None and isinstance(segments, numpy.ndarray):
            bucket_indexes = numpy.clip(
                (segments[:, 4] * ALPHA_BUCKET_COUNT).astype(int),
                0, ALPHA_BUCKET_COUNT - 1)
            segments_by_bucket = [
                segments[bucket_indexes == bucket][:, :4].tolist()
                for bucket in range(ALPHA_BUCKET_COUNT)]
        else:
            segments_by_bucket = [[] for _ in range(ALPHA_BUCKET_COUNT)]
            for start_x, start_y, end_x, end_y, intensity in segments:
                bucket = min(ALPHA_BUCKET_COUNT - 1, int(intensity * ALPHA_BUCKET_COUNT))
                segments_by_bucket[bucket].append((start_x, start_y, end_x, end_y))
        for bucket, bucket_segments in enumerate(segments_by_bucket):
            if not bucket_segments:
                continue
            scratch.set_source_rgba(1, 1, 1, (bucket + 0.5) / ALPHA_BUCKET_COUNT)
            for start_x, start_y, end_x, end_y in bucket_segments:
                scratch.move_to(start_x, start_y)
                scratch.line_to(end_x, end_y)
            scratch.stroke()

        for layer, strength in ((self.flash_layer, FLASH_STAMP_STRENGTH),
                                (self.glow_layer, GLOW_STAMP_STRENGTH)):
            context = cairo.Context(layer)
            context.set_operator(cairo.OPERATOR_ADD)
            context.set_source_rgba(1, 1, 1, strength)
            context.mask_surface(self.fresh_strokes, 0, 0)

    def composite(self, context, width, height, theme, grid_enabled,
                  grid_spacing_fraction=0.1125):
        background_red, background_green, background_blue = theme.background_color
        context.set_source_rgb(background_red, background_green, background_blue)
        context.paint()
        if grid_enabled:
            self._draw_graticule(context, width, height, theme.grid_color,
                                 grid_spacing_fraction)

        context.set_operator(cairo.OPERATOR_ADD)
        upscale = 1.0 / self.resolution
        beam_red, beam_green, beam_blue = theme.beam_color
        flash_red, flash_green, flash_blue = theme.flash_color
        context.save()
        context.scale(upscale, upscale)
        context.set_source_rgba(beam_red, beam_green, beam_blue, 1.0)
        context.mask_surface(self.glow_layer, 0, 0)
        context.set_source_rgba(flash_red, flash_green, flash_blue, 0.8)
        context.mask_surface(self.flash_layer, 0, 0)
        context.restore()
        context.set_operator(cairo.OPERATOR_OVER)

    @staticmethod
    def _draw_graticule(context, width, height, grid_color, spacing_fraction):
        red, green, blue = grid_color
        center_x, center_y = width / 2, height / 2
        spacing = max(8.0, spacing_fraction * min(width, height))

        context.set_line_width(1.0)
        context.set_source_rgba(red, green, blue, 0.07)
        # minor divisions march outward from the center so the grid tracks
        # the beam's amplitude scale as gain (zoom) changes
        offset = spacing
        while offset < max(center_x, center_y):
            for x in (center_x - offset, center_x + offset):
                if 0 <= x <= width:
                    context.move_to(x, 0); context.line_to(x, height)
            for y in (center_y - offset, center_y + offset):
                if 0 <= y <= height:
                    context.move_to(0, y); context.line_to(width, y)
            offset += spacing
        context.stroke()

        context.set_source_rgba(red, green, blue, 0.16)
        context.move_to(center_x, 0); context.line_to(center_x, height)
        context.move_to(0, center_y); context.line_to(width, center_y)
        context.stroke()


class OfflineFrameRenderer:
    """Headless renderer used by snapshot and clip export."""

    def __init__(self, width, height, theme, persistence, grid_enabled,
                 beam_focus=1.6, grid_spacing_fraction=0.1125):
        self.width = width
        self.height = height
        self.theme = theme
        self.persistence = persistence
        self.grid_enabled = grid_enabled
        self.grid_spacing_fraction = grid_spacing_fraction
        self.core = CairoBeamCore()
        self.core.beam_focus = beam_focus
        self.core.ensure_size(width, height)
        self.frame_surface = cairo.ImageSurface(cairo.FORMAT_ARGB32, width, height)

    def render_frame(self, segments):
        """Advance the phosphor and return the composited frame surface."""
        self.core.advance(segments, self.persistence)
        context = cairo.Context(self.frame_surface)
        self.core.composite(context, self.width, self.height, self.theme,
                            self.grid_enabled, self.grid_spacing_fraction)
        self.frame_surface.flush()
        return self.frame_surface

    def frame_bytes(self):
        """Current frame as raw BGRA bytes (cairo's native little-endian layout)."""
        return bytes(self.frame_surface.get_data())
