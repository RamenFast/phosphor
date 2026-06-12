"""CPU (cairo) renderer with two-layer P7 phosphor simulation.

Real P7 phosphor fluoresces blue-white where the electron beam lands, then
phosphoresces in green as it decays. We model that with two energy layers:
a fast-decaying "flash" layer and a slow-decaying "glow" layer, both stored
as pure intensity (A8) and tinted with the theme's flash/beam colors only
when composited — so theme changes apply to light already on screen, the
way changing nothing but your tinted glasses would.

The same core renders live frames and offline export frames, so saved clips
look exactly like the screen.
"""

import cairo

ALPHA_BUCKET_COUNT = 10
FLASH_DECAY_ALPHA = 0.50   # fraction of the flash layer removed every frame
FLASH_STAMP_STRENGTH = 0.9
GLOW_STAMP_STRENGTH = 0.8


class CairoBeamCore:
    """Owns the energy layers and knows how to advance and composite them."""

    def __init__(self):
        self.width = 0
        self.height = 0
        self.flash_layer = None
        self.glow_layer = None
        self.fresh_strokes = None

    def ensure_size(self, width, height):
        if self.flash_layer is None or width != self.width or height != self.height:
            self.width, self.height = width, height
            self.flash_layer = cairo.ImageSurface(cairo.FORMAT_A8, width, height)
            self.glow_layer = cairo.ImageSurface(cairo.FORMAT_A8, width, height)
            self.fresh_strokes = cairo.ImageSurface(cairo.FORMAT_A8, width, height)

    def advance(self, segments, persistence):
        """Decay both layers, then stamp this frame's beam energy into them."""
        glow_decay_alpha = max(0.02, (1.0 - persistence) * 0.6)
        for layer, decay_alpha in ((self.flash_layer, FLASH_DECAY_ALPHA),
                                   (self.glow_layer, glow_decay_alpha)):
            context = cairo.Context(layer)
            context.set_operator(cairo.OPERATOR_DEST_OUT)
            context.set_source_rgba(0, 0, 0, decay_alpha)
            context.paint()

        if not segments:
            return

        scratch = cairo.Context(self.fresh_strokes)
        scratch.set_operator(cairo.OPERATOR_CLEAR)
        scratch.paint()
        scratch.set_operator(cairo.OPERATOR_ADD)
        scratch.set_line_width(2.0)
        scratch.set_line_cap(cairo.LINE_CAP_ROUND)

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

    def composite(self, context, width, height, theme, grid_enabled):
        background_red, background_green, background_blue = theme.background_color
        context.set_source_rgb(background_red, background_green, background_blue)
        context.paint()
        if grid_enabled:
            self._draw_graticule(context, width, height, theme.grid_color)

        context.set_operator(cairo.OPERATOR_ADD)
        beam_red, beam_green, beam_blue = theme.beam_color
        context.set_source_rgba(beam_red, beam_green, beam_blue, 1.0)
        context.mask_surface(self.glow_layer, 0, 0)
        flash_red, flash_green, flash_blue = theme.flash_color
        context.set_source_rgba(flash_red, flash_green, flash_blue, 0.8)
        context.mask_surface(self.flash_layer, 0, 0)
        context.set_operator(cairo.OPERATOR_OVER)

    @staticmethod
    def _draw_graticule(context, width, height, grid_color):
        red, green, blue = grid_color
        context.set_line_width(1.0)
        context.set_source_rgba(red, green, blue, 0.07)
        divisions = 8
        for division in range(1, divisions):
            x = width * division / divisions
            y = height * division / divisions
            context.move_to(x, 0); context.line_to(x, height)
            context.move_to(0, y); context.line_to(width, y)
        context.stroke()
        context.set_source_rgba(red, green, blue, 0.16)
        context.move_to(width / 2, 0); context.line_to(width / 2, height)
        context.move_to(0, height / 2); context.line_to(width, height / 2)
        context.stroke()


class OfflineFrameRenderer:
    """Headless renderer used by snapshot and clip export."""

    def __init__(self, width, height, theme, persistence, grid_enabled):
        self.width = width
        self.height = height
        self.theme = theme
        self.persistence = persistence
        self.grid_enabled = grid_enabled
        self.core = CairoBeamCore()
        self.core.ensure_size(width, height)
        self.frame_surface = cairo.ImageSurface(cairo.FORMAT_ARGB32, width, height)

    def render_frame(self, segments):
        """Advance the phosphor and return the composited frame surface."""
        self.core.advance(segments, self.persistence)
        context = cairo.Context(self.frame_surface)
        self.core.composite(context, self.width, self.height, self.theme, self.grid_enabled)
        self.frame_surface.flush()
        return self.frame_surface

    def frame_bytes(self):
        """Current frame as raw BGRA bytes (cairo's native little-endian layout)."""
        return bytes(self.frame_surface.get_data())
