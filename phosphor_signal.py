# SPDX-License-Identifier: GPL-3.0-or-later
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
  xy_swirl        — the goniometer with a slowly revolving stereo field:
                    the whole figure orbits once every ~18 s.
  ring            — the oscillogram bent around a circle, one ring per
                    channel, triggered so pitched sounds hold still.
  tunnel          — spectrum bands as concentric rings pulsing outward,
                    bass innermost — the tunnel breathes with the music.
"""

import math
from array import array

try:
    import numpy
except ImportError:        # pure-python fallback keeps the package light
    numpy = None

try:
    import phosphor_core   # Rust core; optional, everything works without it
except ImportError:
    phosphor_core = None


def native_available():
    return phosphor_core is not None and phosphor_core.available()


def plan_feed(scope_rate):
    """(pipe_rate, oversample) that realizes a requested scope-detail rate.

    With the native core, high rates are reconstructed in-process by its
    windowed-sinc upsampler from a modest capture feed — same band-limited
    interpolation PulseAudio/ffmpeg would do, without piping 384 kHz audio
    around. Without it, the pipe carries the full rate as before.
    """
    if not native_available() or scope_rate <= 48000:
        return scope_rate, 1
    pipe_rate = 48000 if scope_rate <= 96000 else 96000
    return pipe_rate, max(1, scope_rate // pipe_rate)

# All rate-dependent sizes below are tuned at the base rate and scaled in
# set_sample_rate(), so a higher-fidelity feed refines the trace without
# changing brightness, the waveform's time span, or the spectrum's tuning.
BASE_SAMPLE_RATE = 48000
MAX_POINTS_PER_FRAME = 4000      # bound per-frame work if the UI hiccups
WAVEFORM_WINDOW = 1600           # samples shown per trace (at the base rate)
WAVEFORM_HISTORY = 8192
WAVEFORM_TRIGGER_SEARCH = 2400
FFT_SIZE = 1024
SPECTRUM_BAR_COUNT = 56
SPECTRUM_LOW_HZ = 35.0
SPECTRUM_HIGH_HZ = 18000.0
SWIRL_RADIANS_PER_SECOND = 0.35    # one full revolution every ~18 s
TUNNEL_RINGS = 8
TUNNEL_RING_POINTS = 60
RING_TRACE_POINTS = 720

SQRT_HALF = math.sqrt(0.5)
TAU = 2 * math.pi


class KitChain:
    """Stateful executor for a signal kit's op chain (see phosphor_kit).

    Runs upstream of every display mode, transforming interleaved stereo
    in signal space. Parity contract with the Rust core: phase accumulators
    live in f64 and advance per chunk by 2π·hz·frames/rate with euclidean
    wraparound; trig is computed in f64 and cast to f32 before the f32
    sample math; channel delays are exact integer sample counts. Both
    engines must keep these rules or tests/test_native_parity.py fails.
    """

    def __init__(self, stages):
        self.stages = stages            # canonical [(op, [p0..p3])]
        self.sample_rate = BASE_SAMPLE_RATE
        self.reset()

    def configure(self, sample_rate):
        self.sample_rate = sample_rate
        self.reset()

    def reset(self):
        self.phases = [0.0] * len(self.stages)
        self._delays = [None] * len(self.stages)

    def _delay_line(self, index, milliseconds):
        if self._delays[index] is None:
            count = int(round(milliseconds / 1000.0 * self.sample_rate))
            self._delays[index] = array("f", bytes(4 * count))
        return self._delays[index]

    def process(self, samples):
        """Transformed interleaved stereo, same layout as the input."""
        count = len(samples) // 2
        if count == 0:
            return samples
        if numpy is not None:
            return self._process_numpy(samples, count)
        return self._process_python(samples, count)

    def _process_numpy(self, samples, count):
        frames = numpy.array(samples, dtype=numpy.float32,
                             copy=True).reshape(-1, 2)
        left, right = frames[:, 0], frames[:, 1]
        for index, (op, params) in enumerate(self.stages):
            if op == "rotate":
                hz, angle = params[0], params[1]
                delta = TAU * hz / self.sample_rate
                phases = (self.phases[index] + angle
                          + delta * numpy.arange(count, dtype=numpy.float64))
                cosine = numpy.cos(phases).astype(numpy.float32)
                sine = numpy.sin(phases).astype(numpy.float32)
                left, right = (left * cosine - right * sine,
                               left * sine + right * cosine)
                self.phases[index] = (self.phases[index]
                                      + delta * count) % TAU
            elif op == "midside":
                width = numpy.float32(params[0])
                half_plus = numpy.float32(0.5) * (numpy.float32(1.0) + width)
                half_minus = numpy.float32(0.5) * (numpy.float32(1.0) - width)
                left, right = (half_plus * left + half_minus * right,
                               half_minus * left + half_plus * right)
            elif op == "ringmod":
                hz, depth = params[0], params[1]
                delta = TAU * hz / self.sample_rate
                phases = (self.phases[index]
                          + delta * numpy.arange(count, dtype=numpy.float64))
                gain = (1.0 - depth * (0.5 + 0.5 * numpy.sin(phases))) \
                    .astype(numpy.float32)
                left = left * gain
                right = right * gain
                self.phases[index] = (self.phases[index]
                                      + delta * count) % TAU
            elif op == "wobble":
                hz, depth = params[0], params[1]
                delta = TAU * hz / self.sample_rate
                phases = (self.phases[index]
                          + delta * numpy.arange(count, dtype=numpy.float64))
                angles = depth * numpy.sin(phases)
                cosine = numpy.cos(angles).astype(numpy.float32)
                sine = numpy.sin(angles).astype(numpy.float32)
                left, right = (left * cosine - right * sine,
                               left * sine + right * cosine)
                self.phases[index] = (self.phases[index]
                                      + delta * count) % TAU
            elif op == "matrix":
                a, b, c, d = (numpy.float32(value) for value in params[:4])
                left, right = a * left + b * right, c * left + d * right
            elif op == "chandelay":
                line = self._delay_line(index, params[0])
                if len(line):
                    channel = right if params[1] >= 0.5 else left
                    joined = numpy.concatenate(
                        (numpy.frombuffer(line, dtype=numpy.float32),
                         channel))
                    delayed, keep = joined[:count], joined[count:]
                    self._delays[index] = array("f", keep.tobytes())
                    if params[1] >= 0.5:
                        right = delayed
                    else:
                        left = delayed
        out = numpy.empty((count, 2), dtype=numpy.float32)
        out[:, 0] = left
        out[:, 1] = right
        return out.reshape(-1)

    def _process_python(self, samples, count):
        left = [samples[2 * i] for i in range(count)]
        right = [samples[2 * i + 1] for i in range(count)]
        for index, (op, params) in enumerate(self.stages):
            if op in ("rotate", "wobble"):
                hz = params[0]
                delta = TAU * hz / self.sample_rate
                base = self.phases[index]
                for i in range(count):
                    phase = base + delta * i
                    if op == "rotate":
                        angle = phase + params[1]
                    else:
                        angle = params[1] * math.sin(phase)
                    cosine, sine = math.cos(angle), math.sin(angle)
                    left[i], right[i] = (left[i] * cosine - right[i] * sine,
                                         left[i] * sine + right[i] * cosine)
                self.phases[index] = (base + delta * count) % TAU
            elif op == "midside":
                width = params[0]
                half_plus, half_minus = 0.5 * (1 + width), 0.5 * (1 - width)
                for i in range(count):
                    left[i], right[i] = (
                        half_plus * left[i] + half_minus * right[i],
                        half_minus * left[i] + half_plus * right[i])
            elif op == "ringmod":
                hz, depth = params[0], params[1]
                delta = TAU * hz / self.sample_rate
                base = self.phases[index]
                for i in range(count):
                    gain = 1.0 - depth * (0.5 + 0.5 * math.sin(base + delta * i))
                    left[i] *= gain
                    right[i] *= gain
                self.phases[index] = (base + delta * count) % TAU
            elif op == "matrix":
                a, b, c, d = params[:4]
                for i in range(count):
                    left[i], right[i] = (a * left[i] + b * right[i],
                                         c * left[i] + d * right[i])
            elif op == "chandelay":
                line = self._delay_line(index, params[0])
                if len(line):
                    channel = right if params[1] >= 0.5 else left
                    joined = list(line) + channel
                    delayed, keep = joined[:count], joined[count:]
                    self._delays[index] = array("f", keep)
                    if params[1] >= 0.5:
                        right = delayed
                    else:
                        left = delayed
        out = array("f", bytes(8 * count))
        for i in range(count):
            out[2 * i] = left[i]
            out[2 * i + 1] = right[i]
        return out


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
        self.spectrum_levels = [0.0] * SPECTRUM_BAR_COUNT
        self.frames_since_fft = 0
        self._swirl_phase = 0.0
        self._kit = None                # active KitChain, or None
        self._native = (phosphor_core.NativeComputer()
                        if native_available() else None)
        self.set_sample_rate(BASE_SAMPLE_RATE)

    @property
    def engine(self):
        if self._native is not None:
            return "rust"
        return "numpy" if numpy is not None else "python"

    def set_sample_rate(self, sample_rate, oversample=1):
        """Scale every rate-dependent size so a finer feed only adds detail.

        Beam intensity, the waveform's visible time span, and the spectrum's
        frequency resolution all stay exactly where they were tuned at 48 kHz.
        `oversample` multiplies the XY feed in the native core (see
        plan_feed); the Python path ignores it and traces the pipe rate.
        """
        self.sample_rate = sample_rate
        self.oversample = max(1, int(oversample))
        if self._kit is not None:
            self._kit.configure(sample_rate)
        if self._native is not None:
            self._native.configure(sample_rate, self.oversample)
        rate_ratio = sample_rate / BASE_SAMPLE_RATE
        # distances between consecutive samples shrink as the rate rises;
        # normalizing them back keeps dwell-time brightness identical
        self.sample_distance_scale = rate_ratio
        self.max_points_per_frame = int(MAX_POINTS_PER_FRAME * rate_ratio)
        self.waveform_window = int(WAVEFORM_WINDOW * rate_ratio)
        self.waveform_history_limit = int(WAVEFORM_HISTORY * rate_ratio)
        self.waveform_trigger_search = int(WAVEFORM_TRIGGER_SEARCH * rate_ratio)
        # growing the FFT with the rate keeps the same analysis time window,
        # so bass resolution doesn't degrade; the pure-python fallback stays
        # at 1024 (still correct frequencies, just coarser bars)
        if numpy is not None:
            self.fft_size = FFT_SIZE * max(1, round(rate_ratio))
            self._numpy_fft_window = numpy.hanning(self.fft_size)
            self.fft = None
        else:
            self.fft_size = FFT_SIZE
            self._numpy_fft_window = None
            self.fft = PurePythonFFT(self.fft_size)
        self._bar_bin_ranges = self._compute_bar_bin_ranges()

    def reset(self):
        self.last_beam_x = self.last_beam_y = None
        self.waveform_history = array("f")
        self.spectrum_levels = [0.0] * SPECTRUM_BAR_COUNT
        self._swirl_phase = 0.0
        if self._kit is not None:
            self._kit.reset()
        if self._native is not None:
            self._native.reset()

    def set_kit(self, stages):
        """Install a signal kit chain (canonical [(op, params)] stages from
        phosphor_kit, or None to clear). Applies to every mode, upstream of
        everything; the native core runs its own copy so states start in
        step (they advance independently per engine — cosmetic only)."""
        if stages:
            self._kit = KitChain(stages)
            self._kit.configure(self.sample_rate)
        else:
            self._kit = None
        if self._native is not None:
            self._native.set_kit(stages)

    def compute(self, samples, width, height):
        """New beam segments for this frame from this frame's new samples."""
        if (self._native is not None
                and self.mode in phosphor_core.MODE_IDS):
            return self._native.compute(self.mode, self.gain,
                                        self.beam_energy,
                                        self.frame_glow_keep, samples,
                                        width, height)
        if self._kit is not None and len(samples) >= 2:
            samples = self._kit.process(samples)
        if self.mode in ("xy", "xy45"):
            return self._xy_segments(samples, width, height)
        if self.mode == "xy_swirl":
            return self._xy_segments(self._swirl_rotate(samples),
                                     width, height)
        if self.mode == "xy_dots":
            return self._xy_dot_segments(samples, width, height)
        self.waveform_history.extend(samples)
        excess = len(self.waveform_history) - 2 * self.waveform_history_limit
        if excess > 0:
            del self.waveform_history[:excess]
        if self.mode == "waveform":
            return self._waveform_segments(width, height)
        if self.mode == "ring":
            return self._ring_segments(width, height)
        if self.mode == "tunnel":
            return self._tunnel_segments(width, height)
        if self.mode == "spectrum_radial":
            return self._radial_spectrum_segments(width, height)
        return self._spectrum_segments(width, height)

    def _swirl_rotate(self, samples):
        """Rotate the stereo field by the swirl phase, advanced in real
        time — the figure orbits while the shape stays intact."""
        cosine = math.cos(self._swirl_phase)
        sine = math.sin(self._swirl_phase)
        self._swirl_phase = (self._swirl_phase
                             + (len(samples) / 2) / self.sample_rate
                             * SWIRL_RADIANS_PER_SECOND) % (2 * math.pi)
        if numpy is not None:
            frames = numpy.asarray(samples,
                                   dtype=numpy.float32).reshape(-1, 2)
            rotated = numpy.empty_like(frames)
            rotated[:, 0] = frames[:, 0] * cosine - frames[:, 1] * sine
            rotated[:, 1] = frames[:, 0] * sine + frames[:, 1] * cosine
            return rotated.reshape(-1)
        rotated = array("f", bytes(len(samples) * 4))
        for index in range(0, len(samples) - 1, 2):
            left, right = samples[index], samples[index + 1]
            rotated[index] = left * cosine - right * sine
            rotated[index + 1] = left * sine + right * cosine
        return rotated

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
        if len(samples) > 2 * self.max_points_per_frame:
            samples = samples[-2 * self.max_points_per_frame:]
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
                distance = (math.hypot(x - previous_x, y - previous_y)
                            * self.sample_distance_scale)
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
        distance = (numpy.hypot(numpy.diff(x), numpy.diff(y))
                    * self.sample_distance_scale)
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
        if len(samples) > 2 * self.max_points_per_frame:
            samples = samples[-2 * self.max_points_per_frame:]
        if len(samples) < 2:
            return []
        # a finer feed stamps proportionally more dots along the same path;
        # scaling each stamp down keeps the overall brightness unchanged
        dot_intensity = 1.0 / self.sample_distance_scale
        if numpy is not None:
            x, y = self._xy_points_numpy(samples, width, height, rotate=False)
            weights = self._age_weights(len(x))
            segments = numpy.empty((len(x), 5), dtype=numpy.float32)
            segments[:, 0] = x - 0.8
            segments[:, 1] = y
            segments[:, 2] = x + 0.8
            segments[:, 3] = y
            segments[:, 4] = (dot_intensity if weights is None
                              else weights * dot_intensity)
            return segments
        center_x, center_y = width / 2, height / 2
        radius = min(width, height) * 0.45
        segments = []
        for index in range(0, len(samples) - 1, 2):
            x = center_x + samples[index] * self.gain * radius
            y = center_y - samples[index + 1] * self.gain * radius
            segments.append((x - 0.8, y, x + 0.8, y, dot_intensity))
        return segments

    # -- waveform -------------------------------------------------------------

    def _trigger_offset(self):
        """Frame index of the latest rising zero-crossing of the left channel
        that leaves a full window to display; None if no edge found."""
        history = self.waveform_history
        frame_count = len(history) // 2
        search_start = frame_count - self.waveform_window
        if search_start < 1:
            return None
        for frame in range(search_start,
                           max(0, search_start - self.waveform_trigger_search),
                           -1):
            if history[2 * (frame - 1)] < 0.0 <= history[2 * frame]:
                return frame
        return None

    def _waveform_segments(self, width, height):
        history = self.waveform_history
        frame_count = len(history) // 2
        if frame_count < 4:
            return []
        window = min(self.waveform_window, frame_count)
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

    def _ring_segments(self, width, height):
        """The oscillogram bent around a circle: time sweeps the angle,
        amplitude moves the radius. One ring per channel (left inner,
        right outer), triggered like the flat waveform."""
        history = self.waveform_history
        frame_count = len(history) // 2
        if frame_count < 8:
            return []
        window = min(self.waveform_window, frame_count)
        start_frame = self._trigger_offset() or (frame_count - window)
        center_x, center_y = width / 2, height / 2
        base = min(width, height)
        step = max(1, window // RING_TRACE_POINTS)
        segments = []
        for channel, ring_radius in ((0, base * 0.24), (1, base * 0.36)):
            amplitude = base * 0.09 * self.gain
            previous = None
            for offset in range(0, window, step):
                frame = start_frame + offset
                if frame >= frame_count:
                    break
                angle = 2 * math.pi * offset / window - math.pi / 2
                radius = (ring_radius
                          + history[2 * frame + channel] * amplitude)
                x = center_x + math.cos(angle) * radius
                y = center_y + math.sin(angle) * radius
                if previous is not None:
                    segments.append((previous[0], previous[1], x, y, 0.8))
                previous = (x, y)
        return segments

    def _tunnel_segments(self, width, height):
        """Spectrum bands as concentric rings — bass innermost, each ring
        brightening and swelling with its band. The tunnel breathes."""
        self._update_spectrum_levels()
        segments = []
        center_x, center_y = width / 2, height / 2
        base = min(width, height)
        bands = SPECTRUM_BAR_COUNT // TUNNEL_RINGS
        for ring in range(TUNNEL_RINGS):
            level = max(self.spectrum_levels[ring * bands:(ring + 1) * bands],
                        default=0.0)
            if level < 0.02:
                continue
            depth = (ring / (TUNNEL_RINGS - 1)) ** 1.35
            radius = base * (0.07 + 0.36 * depth) + level * base * 0.03
            intensity = 0.15 + 0.85 * level
            previous = None
            for point in range(TUNNEL_RING_POINTS + 1):
                angle = 2 * math.pi * point / TUNNEL_RING_POINTS
                x = center_x + math.cos(angle) * radius
                y = center_y + math.sin(angle) * radius
                if previous is not None:
                    segments.append((previous[0], previous[1], x, y,
                                     intensity))
                previous = (x, y)
        return segments

    # -- spectrum -------------------------------------------------------------

    def _compute_bar_bin_ranges(self):
        ranges = []
        ratio = SPECTRUM_HIGH_HZ / SPECTRUM_LOW_HZ
        hz_per_bin = self.sample_rate / self.fft_size
        for bar in range(SPECTRUM_BAR_COUNT):
            low_hz = SPECTRUM_LOW_HZ * ratio ** (bar / SPECTRUM_BAR_COUNT)
            high_hz = SPECTRUM_LOW_HZ * ratio ** ((bar + 1) / SPECTRUM_BAR_COUNT)
            low_bin = max(1, int(low_hz / hz_per_bin))
            high_bin = max(low_bin + 1, int(math.ceil(high_hz / hz_per_bin)))
            ranges.append((low_bin, min(high_bin, self.fft_size // 2)))
        return ranges

    def _update_spectrum_levels(self):
        """Run the FFT every other frame and smooth bar levels in place."""
        frame_count = len(self.waveform_history) // 2
        self.frames_since_fft += 1
        if frame_count >= self.fft_size and self.frames_since_fft >= 2:
            self.frames_since_fft = 0
            tail_start = 2 * (frame_count - self.fft_size)
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
                    for i in range(self.fft_size)
                ]
                magnitudes = self.fft.magnitudes(mono)
            for bar, (low_bin, high_bin) in enumerate(self._bar_bin_ranges):
                peak = max(magnitudes[low_bin:high_bin], default=0.0)
                level = min(1.0, (peak / (self.fft_size / 8)) ** 0.5 * self.gain)
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
