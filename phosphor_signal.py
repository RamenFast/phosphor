"""Turns raw stereo samples into beam segments for the renderers.

A segment is (x0, y0, x1, y1, intensity 0..1). Both renderers consume the
same segments, and the offline exporter replays audio through this same
class, so what you record is exactly what you saw.

Modes:
  xy              — left channel deflects X, right deflects Y (scope music)
  xy45            — goniometer rotation: X = side (L-R), Y = mid (L+R).
                    Ordinary stereo songs collapse to a diagonal line in raw
                    XY; rotated, mono energy stands upright and stereo width
                    blooms sideways.
  xy_dots         — XY sampled as discrete dots, the way a vectorscope's
                    dot display looks; great for seeing sample density.
  waveform        — dual trace with rising-edge triggering so pitched sounds
                    stand still instead of crawling.
  spectrum        — log-frequency bars from a pure-python FFT, with fast
                    attack and slow phosphor-style fall.
  spectrum_radial — the same analysis swept around a circle, bars radiating
                    outward from a quiet center.
"""

import math
from array import array

try:
    import numpy
except ImportError:        # pure-python fallback keeps the package light
    numpy = None

MAX_POINTS_PER_FRAME = 4000      # bound per-frame work if the UI hiccups
WAVEFORM_WINDOW = 1600           # samples shown per trace
WAVEFORM_HISTORY = 8192
FFT_SIZE = 1024
SPECTRUM_BAR_COUNT = 56
SPECTRUM_LOW_HZ = 35.0
SPECTRUM_HIGH_HZ = 18000.0
SAMPLE_RATE = 48000

SQRT_HALF = math.sqrt(0.5)


class PurePythonFFT:
    """Iterative radix-2 FFT with precomputed tables; fast enough at 1024."""

    def __init__(self, size):
        self.size = size
        self.levels = size.bit_length() - 1
        self.bit_reversed = [self._reverse_bits(i, self.levels) for i in range(size)]
        self.cosines = [math.cos(2 * math.pi * i / size) for i in range(size // 2)]
        self.sines = [math.sin(2 * math.pi * i / size) for i in range(size // 2)]
        self.window = [0.5 - 0.5 * math.cos(2 * math.pi * i / (size - 1)) for i in range(size)]

    @staticmethod
    def _reverse_bits(value, bit_count):
        result = 0
        for _ in range(bit_count):
            result = (result << 1) | (value & 1)
            value >>= 1
        return result

    def magnitudes(self, samples):
        """Hann-windowed magnitude spectrum of `samples` (length >= size)."""
        size = self.size
        real = [samples[i] * self.window[i] for i in range(size)]
        imaginary = [0.0] * size
        real = [real[self.bit_reversed[i]] for i in range(size)]
        imaginary = [0.0] * size

        half_block = 1
        while half_block < size:
            table_step = size // (half_block * 2)
            for block_start in range(0, size, half_block * 2):
                table_index = 0
                for position in range(block_start, block_start + half_block):
                    partner = position + half_block
                    cosine = self.cosines[table_index]
                    sine = self.sines[table_index]
                    real_product = real[partner] * cosine + imaginary[partner] * sine
                    imaginary_product = imaginary[partner] * cosine - real[partner] * sine
                    real[partner] = real[position] - real_product
                    imaginary[partner] = imaginary[position] - imaginary_product
                    real[position] += real_product
                    imaginary[position] += imaginary_product
                    table_index += table_step
            half_block *= 2

        return [math.hypot(real[i], imaginary[i]) for i in range(size // 2)]


class SegmentComputer:
    def __init__(self):
        self.mode = "xy"
        self.gain = 1.0
        self.beam_energy = 8.0
        # glow kept per frame; lets intensities pre-decay by age inside a
        # frame so trails grade smoothly instead of stepping per frame
        self.frame_glow_keep = 0.82
        self.last_beam_x = None
        self.last_beam_y = None
        self.waveform_history = array("f")
        self.fft = PurePythonFFT(FFT_SIZE)
        self._numpy_fft_window = (numpy.hanning(FFT_SIZE)
                                  if numpy is not None else None)
        self.spectrum_levels = [0.0] * SPECTRUM_BAR_COUNT
        self.frames_since_fft = 0
        self._bar_bin_ranges = self._compute_bar_bin_ranges()

    def reset(self):
        self.last_beam_x = self.last_beam_y = None
        self.waveform_history = array("f")
        self.spectrum_levels = [0.0] * SPECTRUM_BAR_COUNT

    def compute(self, samples, width, height):
        """New beam segments for this frame from this frame's new samples."""
        if self.mode in ("xy", "xy45"):
            return self._xy_segments(samples, width, height)
        if self.mode == "xy_dots":
            return self._xy_dot_segments(samples, width, height)
        self.waveform_history.extend(samples)
        excess = len(self.waveform_history) - 2 * WAVEFORM_HISTORY
        if excess > 0:
            del self.waveform_history[:excess]
        if self.mode == "waveform":
            return self._waveform_segments(width, height)
        if self.mode == "spectrum_radial":
            return self._radial_spectrum_segments(width, height)
        return self._spectrum_segments(width, height)

    # -- XY -----------------------------------------------------------------

    def _age_weights(self, count):
        """Pre-decay factor per segment: the oldest audio in a frame has
        nearly a full frame of phosphor decay behind it already. Stamping it
        dimmer makes trails grade continuously in time instead of stepping
        once per displayed frame (the 'duplicate line' artifact on slowly
        drifting scope-music sweeps)."""
        if count <= 1:
            return None
        if numpy is not None:
            ages = (count - 1 - numpy.arange(count)) / count
            return numpy.power(self.frame_glow_keep, ages)
        return [self.frame_glow_keep ** ((count - 1 - index) / count)
                for index in range(count)]

    def _xy_points_numpy(self, samples, width, height, rotate):
        frames = numpy.asarray(samples, dtype=numpy.float32).reshape(-1, 2)
        left, right = frames[:, 0], frames[:, 1]
        if rotate:
            horizontal = (left - right) * SQRT_HALF   # stereo width (side)
            vertical = (left + right) * SQRT_HALF     # mono energy (mid)
        else:
            horizontal, vertical = left, right
        radius = min(width, height) * 0.45
        x = width / 2 + horizontal * (self.gain * radius)
        y = height / 2 - vertical * (self.gain * radius)
        return x, y

    def _xy_segments(self, samples, width, height):
        if len(samples) > 2 * MAX_POINTS_PER_FRAME:
            samples = samples[-2 * MAX_POINTS_PER_FRAME:]
            self.last_beam_x = self.last_beam_y = None
        if len(samples) < 2:
            return []
        if numpy is not None:
            return self._xy_segments_numpy(samples, width, height)

        center_x, center_y = width / 2, height / 2
        radius = min(width, height) * 0.45
        rotate = self.mode == "xy45"

        segments = []
        previous_x, previous_y = self.last_beam_x, self.last_beam_y
        for index in range(0, len(samples) - 1, 2):
            left, right = samples[index], samples[index + 1]
            if rotate:
                horizontal = (left - right) * SQRT_HALF   # stereo width (side)
                vertical = (left + right) * SQRT_HALF     # mono energy (mid)
            else:
                horizontal, vertical = left, right
            x = center_x + horizontal * self.gain * radius
            y = center_y - vertical * self.gain * radius
            if previous_x is not None:
                distance = math.hypot(x - previous_x, y - previous_y)
                intensity = min(1.0, self.beam_energy / (distance + 0.7))
                segments.append((previous_x, previous_y, x, y, intensity))
            previous_x, previous_y = x, y
        self.last_beam_x, self.last_beam_y = previous_x, previous_y

        weights = self._age_weights(len(segments))
        if weights is not None:
            segments = [(x0, y0, x1, y1, intensity * weight)
                        for (x0, y0, x1, y1, intensity), weight
                        in zip(segments, weights)]
        return segments

    def _xy_segments_numpy(self, samples, width, height):
        x, y = self._xy_points_numpy(samples, width, height,
                                     rotate=self.mode == "xy45")
        if self.last_beam_x is not None:
            x = numpy.concatenate(([self.last_beam_x], x))
            y = numpy.concatenate(([self.last_beam_y], y))
        self.last_beam_x = float(x[-1])
        self.last_beam_y = float(y[-1])
        if len(x) < 2:
            return []
        distance = numpy.hypot(numpy.diff(x), numpy.diff(y))
        intensity = numpy.minimum(1.0, self.beam_energy / (distance + 0.7))
        weights = self._age_weights(len(intensity))
        if weights is not None:
            intensity = intensity * weights
        segments = numpy.empty((len(intensity), 5), dtype=numpy.float32)
        segments[:, 0] = x[:-1]
        segments[:, 1] = y[:-1]
        segments[:, 2] = x[1:]
        segments[:, 3] = y[1:]
        segments[:, 4] = intensity
        return segments

    def _xy_dot_segments(self, samples, width, height):
        """Discrete-dot vectorscope display: one short stamp per sample."""
        if len(samples) > 2 * MAX_POINTS_PER_FRAME:
            samples = samples[-2 * MAX_POINTS_PER_FRAME:]
        if len(samples) < 2:
            return []
        if numpy is not None:
            x, y = self._xy_points_numpy(samples, width, height, rotate=False)
            weights = self._age_weights(len(x))
            segments = numpy.empty((len(x), 5), dtype=numpy.float32)
            segments[:, 0] = x - 0.8
            segments[:, 1] = y
            segments[:, 2] = x + 0.8
            segments[:, 3] = y
            segments[:, 4] = 1.0 if weights is None else weights
            return segments
        center_x, center_y = width / 2, height / 2
        radius = min(width, height) * 0.45
        segments = []
        for index in range(0, len(samples) - 1, 2):
            x = center_x + samples[index] * self.gain * radius
            y = center_y - samples[index + 1] * self.gain * radius
            segments.append((x - 0.8, y, x + 0.8, y, 1.0))
        return segments

    # -- waveform -------------------------------------------------------------

    def _trigger_offset(self):
        """Frame index of the latest rising zero-crossing of the left channel
        that leaves a full window to display; None if no edge found."""
        history = self.waveform_history
        frame_count = len(history) // 2
        search_start = frame_count - WAVEFORM_WINDOW
        if search_start < 1:
            return None
        for frame in range(search_start, max(0, search_start - 2400), -1):
            if history[2 * (frame - 1)] < 0.0 <= history[2 * frame]:
                return frame
        return None

    def _waveform_segments(self, width, height):
        history = self.waveform_history
        frame_count = len(history) // 2
        if frame_count < 4:
            return []
        window = min(WAVEFORM_WINDOW, frame_count)
        start_frame = self._trigger_offset() or (frame_count - window)

        segments = []
        amplitude = height * 0.21 * self.gain
        step = max(1, window // max(64, width))
        for channel, baseline in ((0, height * 0.28), (1, height * 0.72)):
            previous = None
            for offset in range(0, window, step):
                frame = start_frame + offset
                if frame >= frame_count:
                    break
                x = width * offset / window
                y = baseline - history[2 * frame + channel] * amplitude
                if previous is not None:
                    segments.append((previous[0], previous[1], x, y, 0.85))
                previous = (x, y)
        return segments

    # -- spectrum -------------------------------------------------------------

    def _compute_bar_bin_ranges(self):
        ranges = []
        ratio = SPECTRUM_HIGH_HZ / SPECTRUM_LOW_HZ
        hz_per_bin = SAMPLE_RATE / FFT_SIZE
        for bar in range(SPECTRUM_BAR_COUNT):
            low_hz = SPECTRUM_LOW_HZ * ratio ** (bar / SPECTRUM_BAR_COUNT)
            high_hz = SPECTRUM_LOW_HZ * ratio ** ((bar + 1) / SPECTRUM_BAR_COUNT)
            low_bin = max(1, int(low_hz / hz_per_bin))
            high_bin = max(low_bin + 1, int(math.ceil(high_hz / hz_per_bin)))
            ranges.append((low_bin, min(high_bin, FFT_SIZE // 2)))
        return ranges

    def _update_spectrum_levels(self):
        """Run the FFT every other frame and smooth bar levels in place."""
        frame_count = len(self.waveform_history) // 2
        self.frames_since_fft += 1
        if frame_count >= FFT_SIZE and self.frames_since_fft >= 2:
            self.frames_since_fft = 0
            tail_start = 2 * (frame_count - FFT_SIZE)
            if numpy is not None:
                tail = numpy.asarray(self.waveform_history[tail_start:],
                                     dtype=numpy.float32).reshape(-1, 2)
                mono = tail.mean(axis=1)
                magnitudes = numpy.abs(
                    numpy.fft.rfft(mono * self._numpy_fft_window))
            else:
                mono = [
                    (self.waveform_history[tail_start + 2 * i]
                     + self.waveform_history[tail_start + 2 * i + 1]) * 0.5
                    for i in range(FFT_SIZE)
                ]
                magnitudes = self.fft.magnitudes(mono)
            for bar, (low_bin, high_bin) in enumerate(self._bar_bin_ranges):
                peak = max(magnitudes[low_bin:high_bin], default=0.0)
                level = min(1.0, (peak / (FFT_SIZE / 8)) ** 0.5 * self.gain)
                if level > self.spectrum_levels[bar]:
                    self.spectrum_levels[bar] = level            # fast attack
                else:
                    self.spectrum_levels[bar] *= 0.93            # slow fall

    def _spectrum_segments(self, width, height):
        self._update_spectrum_levels()
        segments = []
        baseline = height * 0.88
        bar_pitch = width / SPECTRUM_BAR_COUNT
        for bar, level in enumerate(self.spectrum_levels):
            if level < 0.01:
                continue
            x = bar_pitch * (bar + 0.5)
            top = baseline - level * height * 0.74
            segments.append((x, baseline, x, top, 0.35 + 0.65 * level))
        return segments

    def _radial_spectrum_segments(self, width, height):
        """Spectrum bars radiating from a circle: bass at twelve o'clock,
        sweeping clockwise to treble."""
        self._update_spectrum_levels()
        segments = []
        center_x, center_y = width / 2, height / 2
        inner_radius = min(width, height) * 0.14
        bar_reach = min(width, height) * 0.32
        for bar, level in enumerate(self.spectrum_levels):
            if level < 0.01:
                continue
            angle = 2 * math.pi * (bar + 0.5) / SPECTRUM_BAR_COUNT - math.pi / 2
            cosine, sine = math.cos(angle), math.sin(angle)
            outer_radius = inner_radius + level * bar_reach
            segments.append((center_x + cosine * inner_radius,
                             center_y + sine * inner_radius,
                             center_x + cosine * outer_radius,
                             center_y + sine * outer_radius,
                             0.35 + 0.65 * level))
        return segments
