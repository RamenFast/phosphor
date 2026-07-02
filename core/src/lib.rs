// SPDX-License-Identifier: GPL-3.0-or-later
//! Native signal core for Phosphor: turns raw stereo samples into beam
//! segments, mirroring phosphor_signal.SegmentComputer exactly (the Python
//! implementation stays as the fallback and the reference for parity tests).
//!
//! A segment is (x0, y0, x1, y1, intensity 0..1), written as 5 f32s into a
//! caller-provided buffer. On top of the Python behavior this core adds an
//! oversampling stage: a polyphase windowed-sinc upsampler that multiplies
//! the XY feed rate in-process, so "Ultra" scope detail no longer needs the
//! full-rate stream piped through PulseAudio.
//!
//! C ABI (used from Python via ctypes):
//!   pc_version() -> u32
//!   pc_new() -> handle          pc_free(handle)
//!   pc_configure(handle, sample_rate, oversample)
//!   pc_reset(handle)
//!   pc_compute(handle, mode, gain, beam_energy, glow_keep,
//!              samples, n_floats, width, height,
//!              out, out_capacity_segments) -> segments written
//!
//! Not thread-safe per handle; Phosphor serializes access with its existing
//! compute lock.

use std::f32::consts::PI;

pub const API_VERSION: u32 = 1;

const BASE_SAMPLE_RATE: f32 = 48000.0;
const MAX_POINTS_PER_FRAME: usize = 4000;
const WAVEFORM_WINDOW: usize = 1600;
const WAVEFORM_HISTORY: usize = 8192;
const WAVEFORM_TRIGGER_SEARCH: usize = 2400;
const FFT_BASE_SIZE: usize = 1024;
const SPECTRUM_BAR_COUNT: usize = 56;
const SPECTRUM_LOW_HZ: f32 = 35.0;
const SPECTRUM_HIGH_HZ: f32 = 18000.0;
const SQRT_HALF: f32 = std::f32::consts::FRAC_1_SQRT_2;

// Polyphase upsampler: half-width in input frames (16 taps per output).
const SINC_HALF_WIDTH: usize = 8;
const SINC_CUTOFF: f32 = 0.9; // of the input Nyquist, keeps the kernel short

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Xy = 0,
    Xy45 = 1,
    XyDots = 2,
    Waveform = 3,
    Spectrum = 4,
    SpectrumRadial = 5,
}

impl Mode {
    fn from_u32(value: u32) -> Option<Mode> {
        match value {
            0 => Some(Mode::Xy),
            1 => Some(Mode::Xy45),
            2 => Some(Mode::XyDots),
            3 => Some(Mode::Waveform),
            4 => Some(Mode::Spectrum),
            5 => Some(Mode::SpectrumRadial),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// FFT: iterative radix-2, precomputed tables — same shape as the Python
// fallback, plenty fast at the 1k–8k sizes the spectrum uses.
// ---------------------------------------------------------------------------

struct Fft {
    size: usize,
    bit_reversed: Vec<usize>,
    cosines: Vec<f32>,
    sines: Vec<f32>,
    hann: Vec<f32>,
}

impl Fft {
    fn new(size: usize) -> Fft {
        assert!(size.is_power_of_two());
        let levels = size.trailing_zeros();
        let bit_reversed = (0..size)
            .map(|i| i.reverse_bits() >> (usize::BITS - levels))
            .collect();
        let cosines = (0..size / 2)
            .map(|i| (2.0 * PI * i as f32 / size as f32).cos())
            .collect();
        let sines = (0..size / 2)
            .map(|i| (2.0 * PI * i as f32 / size as f32).sin())
            .collect();
        // numpy.hanning: symmetric window, denominator N-1
        let hann = (0..size)
            .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / (size as f32 - 1.0)).cos())
            .collect();
        Fft { size, bit_reversed, cosines, sines, hann }
    }

    /// Hann-windowed magnitude spectrum; `samples.len() == size`, output
    /// holds bins 0..=size/2 (the rfft layout the Python side uses).
    fn magnitudes(&self, samples: &[f32], out: &mut Vec<f32>) {
        let size = self.size;
        let mut real = vec![0.0f32; size];
        let mut imaginary = vec![0.0f32; size];
        for i in 0..size {
            real[i] = samples[self.bit_reversed[i]] * self.hann[self.bit_reversed[i]];
        }
        let mut half_block = 1;
        while half_block < size {
            let table_step = size / (half_block * 2);
            let mut block_start = 0;
            while block_start < size {
                let mut table_index = 0;
                for position in block_start..block_start + half_block {
                    let partner = position + half_block;
                    let cosine = self.cosines[table_index];
                    let sine = self.sines[table_index];
                    let real_product = real[partner] * cosine + imaginary[partner] * sine;
                    let imaginary_product =
                        imaginary[partner] * cosine - real[partner] * sine;
                    real[partner] = real[position] - real_product;
                    imaginary[partner] = imaginary[position] - imaginary_product;
                    real[position] += real_product;
                    imaginary[position] += imaginary_product;
                    table_index += table_step;
                }
                block_start += half_block * 2;
            }
            half_block *= 2;
        }
        out.clear();
        out.extend((0..=size / 2).map(|i| real[i].hypot(imaginary[i])));
    }
}

// ---------------------------------------------------------------------------
// Polyphase windowed-sinc upsampler (stereo interleaved), streaming.
// ---------------------------------------------------------------------------

struct Upsampler {
    factor: usize,
    // taps[phase][k] weights x[m - half + 1 + k] for output time m + phase/N
    taps: Vec<Vec<f32>>,
    // last 2*half-1 input frames, interleaved, carried between calls
    tail: Vec<f32>,
}

impl Upsampler {
    fn new(factor: usize) -> Upsampler {
        let half = SINC_HALF_WIDTH;
        let mut taps = Vec::with_capacity(factor);
        for phase in 0..factor {
            let fraction = phase as f32 / factor as f32;
            let mut row = Vec::with_capacity(2 * half);
            for k in 0..2 * half {
                // tap k multiplies the input sample at offset (k - half + 1)
                // relative to the base frame; u is its distance from the
                // output instant, in input-sample units
                let u = (k as f32 - (half as f32 - 1.0)) - fraction;
                let sinc = if u == 0.0 {
                    1.0
                } else {
                    (PI * SINC_CUTOFF * u).sin() / (PI * SINC_CUTOFF * u)
                };
                // Blackman window over the kernel's span
                let normalized = u / half as f32;
                let window = if normalized.abs() >= 1.0 {
                    0.0
                } else {
                    0.42 + 0.5 * (PI * normalized).cos()
                        + 0.08 * (2.0 * PI * normalized).cos()
                };
                row.push(SINC_CUTOFF * sinc * window);
            }
            // exact unit DC gain per phase: no brightness/level drift
            let sum: f32 = row.iter().sum();
            for tap in row.iter_mut() {
                *tap /= sum;
            }
            taps.push(row);
        }
        let mut upsampler = Upsampler { factor, taps, tail: Vec::new() };
        upsampler.reset();
        upsampler
    }

    fn reset(&mut self) {
        self.tail = vec![0.0; (2 * SINC_HALF_WIDTH - 1) * 2];
    }

    /// Interleaved stereo in -> interleaved stereo out (factor× the frames).
    fn process(&mut self, input: &[f32], output: &mut Vec<f32>) {
        let half = SINC_HALF_WIDTH;
        let mut buffer = std::mem::take(&mut self.tail);
        buffer.extend_from_slice(input);
        let frames = buffer.len() / 2;
        output.clear();
        if frames >= 2 * half {
            output.reserve((frames - (2 * half - 1)) * self.factor * 2);
            for base in 0..frames - (2 * half - 1) {
                for phase in 0..self.factor {
                    let taps = &self.taps[phase];
                    let mut left = 0.0f32;
                    let mut right = 0.0f32;
                    for (k, tap) in taps.iter().enumerate() {
                        left += buffer[(base + k) * 2] * tap;
                        right += buffer[(base + k) * 2 + 1] * tap;
                    }
                    output.push(left);
                    output.push(right);
                }
            }
        }
        let keep_from = buffer.len().saturating_sub((2 * half - 1) * 2);
        buffer.drain(..keep_from);
        self.tail = buffer;
    }
}

// ---------------------------------------------------------------------------
// The computer
// ---------------------------------------------------------------------------

pub struct Computer {
    sample_rate: f32,
    oversample: usize,
    // xy: distances between consecutive samples shrink as the rate rises;
    // normalizing them back keeps dwell-time brightness identical
    sample_distance_scale: f32,
    max_points_per_frame: usize,
    waveform_window: usize,
    waveform_history_limit: usize,
    waveform_trigger_search: usize,
    fft: Fft,
    bar_bin_ranges: [(usize, usize); SPECTRUM_BAR_COUNT],
    upsampler: Option<Upsampler>,
    upsample_buffer: Vec<f32>,
    magnitude_buffer: Vec<f32>,
    last_beam: Option<(f32, f32)>,
    waveform_history: Vec<f32>,
    spectrum_levels: [f32; SPECTRUM_BAR_COUNT],
    frames_since_fft: u32,
}

fn bar_bin_ranges(sample_rate: f32, fft_size: usize) -> [(usize, usize); SPECTRUM_BAR_COUNT] {
    let ratio = SPECTRUM_HIGH_HZ / SPECTRUM_LOW_HZ;
    let hz_per_bin = sample_rate / fft_size as f32;
    let mut ranges = [(0usize, 0usize); SPECTRUM_BAR_COUNT];
    for (bar, range) in ranges.iter_mut().enumerate() {
        let low_hz = SPECTRUM_LOW_HZ * ratio.powf(bar as f32 / SPECTRUM_BAR_COUNT as f32);
        let high_hz =
            SPECTRUM_LOW_HZ * ratio.powf((bar + 1) as f32 / SPECTRUM_BAR_COUNT as f32);
        let low_bin = ((low_hz / hz_per_bin) as usize).max(1);
        let high_bin = ((high_hz / hz_per_bin).ceil() as usize).max(low_bin + 1);
        *range = (low_bin, high_bin.min(fft_size / 2));
    }
    ranges
}

impl Computer {
    pub fn new() -> Computer {
        let mut computer = Computer {
            sample_rate: BASE_SAMPLE_RATE,
            oversample: 1,
            sample_distance_scale: 1.0,
            max_points_per_frame: MAX_POINTS_PER_FRAME,
            waveform_window: WAVEFORM_WINDOW,
            waveform_history_limit: WAVEFORM_HISTORY,
            waveform_trigger_search: WAVEFORM_TRIGGER_SEARCH,
            fft: Fft::new(FFT_BASE_SIZE),
            bar_bin_ranges: bar_bin_ranges(BASE_SAMPLE_RATE, FFT_BASE_SIZE),
            upsampler: None,
            upsample_buffer: Vec::new(),
            magnitude_buffer: Vec::new(),
            last_beam: None,
            waveform_history: Vec::new(),
            spectrum_levels: [0.0; SPECTRUM_BAR_COUNT],
            frames_since_fft: 0,
        };
        computer.configure(BASE_SAMPLE_RATE as u32, 1);
        computer
    }

    /// `sample_rate` is the incoming feed rate; `oversample` multiplies it
    /// in-core for the XY modes (waveform/spectrum window sizes scale with
    /// the feed rate alone — their time spans are what must stay put).
    pub fn configure(&mut self, sample_rate: u32, oversample: u32) {
        let oversample = (oversample.max(1) as usize).min(16);
        self.sample_rate = sample_rate as f32;
        self.oversample = oversample;
        let feed_ratio = self.sample_rate / BASE_SAMPLE_RATE;
        let effective_ratio = feed_ratio * oversample as f32;
        self.sample_distance_scale = effective_ratio;
        self.max_points_per_frame =
            (MAX_POINTS_PER_FRAME as f32 * effective_ratio) as usize;
        self.waveform_window = (WAVEFORM_WINDOW as f32 * feed_ratio) as usize;
        self.waveform_history_limit = (WAVEFORM_HISTORY as f32 * feed_ratio) as usize;
        self.waveform_trigger_search =
            (WAVEFORM_TRIGGER_SEARCH as f32 * feed_ratio) as usize;
        // growing the FFT with the rate keeps the same analysis time window,
        // so bass resolution doesn't degrade at fine feed rates
        let fft_size = FFT_BASE_SIZE * (feed_ratio.round().max(1.0) as usize);
        if self.fft.size != fft_size {
            self.fft = Fft::new(fft_size);
        }
        self.bar_bin_ranges = bar_bin_ranges(self.sample_rate, fft_size);
        self.upsampler = if oversample > 1 {
            Some(Upsampler::new(oversample))
        } else {
            None
        };
        self.reset();
    }

    pub fn reset(&mut self) {
        self.last_beam = None;
        self.waveform_history.clear();
        self.spectrum_levels = [0.0; SPECTRUM_BAR_COUNT];
        if let Some(upsampler) = self.upsampler.as_mut() {
            upsampler.reset();
        }
    }

    /// Returns segments written to `out` (each 5 f32s), at most `capacity`.
    #[allow(clippy::too_many_arguments)]
    pub fn compute(&mut self, mode: Mode, gain: f32, beam_energy: f32,
                   glow_keep: f32, samples: &[f32], width: f32, height: f32,
                   out: &mut SegmentSink) -> usize {
        let samples = &samples[..samples.len() - samples.len() % 2];
        match mode {
            Mode::Xy | Mode::Xy45 | Mode::XyDots => {
                if self.upsampler.is_some() {
                    let mut buffer = std::mem::take(&mut self.upsample_buffer);
                    self.upsampler.as_mut().unwrap().process(samples, &mut buffer);
                    self.xy_modes(mode, gain, beam_energy, glow_keep,
                                  &buffer, width, height, out);
                    self.upsample_buffer = buffer;
                } else {
                    self.xy_modes(mode, gain, beam_energy, glow_keep,
                                  samples, width, height, out);
                }
            }
            _ => {
                self.extend_waveform_history(samples);
                match mode {
                    Mode::Waveform => self.waveform(gain, width, height, out),
                    Mode::Spectrum => {
                        self.update_spectrum_levels(gain);
                        self.spectrum(width, height, out);
                    }
                    Mode::SpectrumRadial => {
                        self.update_spectrum_levels(gain);
                        self.spectrum_radial(width, height, out);
                    }
                    _ => unreachable!(),
                }
            }
        }
        out.count
    }

    // -- XY -------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn xy_modes(&mut self, mode: Mode, gain: f32, beam_energy: f32,
                glow_keep: f32, samples: &[f32], width: f32, height: f32,
                out: &mut SegmentSink) {
        let mut samples = samples;
        if samples.len() > 2 * self.max_points_per_frame {
            samples = &samples[samples.len() - 2 * self.max_points_per_frame..];
            if mode != Mode::XyDots {
                self.last_beam = None; // gap in the trace, don't bridge it
            }
        }
        if samples.len() < 2 {
            return;
        }
        let center_x = width / 2.0;
        let center_y = height / 2.0;
        let radius = width.min(height) * 0.45;
        let deflection = gain * radius;
        let rotate = mode == Mode::Xy45;
        let point_of = |left: f32, right: f32| -> (f32, f32) {
            let (horizontal, vertical) = if rotate {
                ((left - right) * SQRT_HALF, (left + right) * SQRT_HALF)
            } else {
                (left, right)
            };
            (center_x + horizontal * deflection, center_y - vertical * deflection)
        };

        if mode == Mode::XyDots {
            // discrete-dot display; a finer feed stamps proportionally more
            // dots along the same path, so each is scaled down to keep the
            // overall brightness unchanged
            let dot_intensity = 1.0 / self.sample_distance_scale;
            let count = samples.len() / 2;
            for index in 0..count {
                let (x, y) = point_of(samples[2 * index], samples[2 * index + 1]);
                let weight =
                    age_weight(glow_keep, count, index).unwrap_or(1.0) * dot_intensity;
                out.push(x - 0.8, y, x + 0.8, y, weight);
            }
            return;
        }

        let count = samples.len() / 2 + usize::from(self.last_beam.is_some());
        let segment_count = count - 1;
        let (mut previous_x, mut previous_y) = match self.last_beam {
            Some(beam) => beam,
            None => point_of(samples[0], samples[1]),
        };
        let skip_first = usize::from(self.last_beam.is_none());
        for index in skip_first..samples.len() / 2 {
            let (x, y) = point_of(samples[2 * index], samples[2 * index + 1]);
            let distance = (x - previous_x).hypot(y - previous_y)
                * self.sample_distance_scale;
            let mut intensity = (beam_energy / (distance + 0.7)).min(1.0);
            let segment_index = index - skip_first;
            if let Some(weight) = age_weight(glow_keep, segment_count, segment_index) {
                intensity *= weight;
            }
            out.push(previous_x, previous_y, x, y, intensity);
            previous_x = x;
            previous_y = y;
        }
        self.last_beam = Some((previous_x, previous_y));
    }

    // -- waveform ---------------------------------------------------------------

    fn extend_waveform_history(&mut self, samples: &[f32]) {
        self.waveform_history.extend_from_slice(samples);
        let excess =
            self.waveform_history.len() as isize - 2 * self.waveform_history_limit as isize;
        if excess > 0 {
            self.waveform_history.drain(..excess as usize);
        }
    }

    /// Frame index of the latest rising zero-crossing of the left channel
    /// that leaves a full window to display.
    fn trigger_offset(&self) -> Option<usize> {
        let history = &self.waveform_history;
        let frame_count = history.len() / 2;
        if frame_count < self.waveform_window + 1 {
            return None;
        }
        let search_start = frame_count - self.waveform_window;
        let search_end = search_start.saturating_sub(self.waveform_trigger_search).max(1);
        (search_end..=search_start)
            .rev()
            .find(|&frame| history[2 * (frame - 1)] < 0.0 && history[2 * frame] >= 0.0)
    }

    fn waveform(&mut self, gain: f32, width: f32, height: f32, out: &mut SegmentSink) {
        let history = &self.waveform_history;
        let frame_count = history.len() / 2;
        if frame_count < 4 {
            return;
        }
        let window = self.waveform_window.min(frame_count);
        let start_frame = self.trigger_offset().unwrap_or(frame_count - window);
        let amplitude = height * 0.21 * gain;
        let step = (window / (width as usize).max(64)).max(1);
        for (channel, baseline) in [(0usize, height * 0.28), (1usize, height * 0.72)] {
            let mut previous: Option<(f32, f32)> = None;
            for offset in (0..window).step_by(step) {
                let frame = start_frame + offset;
                if frame >= frame_count {
                    break;
                }
                let x = width * offset as f32 / window as f32;
                let y = baseline - history[2 * frame + channel] * amplitude;
                if let Some((px, py)) = previous {
                    out.push(px, py, x, y, 0.85);
                }
                previous = Some((x, y));
            }
        }
    }

    // -- spectrum ---------------------------------------------------------------

    /// Run the FFT every other frame and smooth bar levels in place.
    fn update_spectrum_levels(&mut self, gain: f32) {
        let frame_count = self.waveform_history.len() / 2;
        self.frames_since_fft += 1;
        if frame_count < self.fft.size || self.frames_since_fft < 2 {
            return;
        }
        self.frames_since_fft = 0;
        let tail_start = 2 * (frame_count - self.fft.size);
        let mono: Vec<f32> = self.waveform_history[tail_start..]
            .chunks_exact(2)
            .map(|frame| (frame[0] + frame[1]) * 0.5)
            .collect();
        let mut magnitudes = std::mem::take(&mut self.magnitude_buffer);
        self.fft.magnitudes(&mono, &mut magnitudes);
        let normalization = self.fft.size as f32 / 8.0;
        for (bar, &(low_bin, high_bin)) in self.bar_bin_ranges.iter().enumerate() {
            let peak = magnitudes[low_bin..high_bin]
                .iter()
                .fold(0.0f32, |a, &b| a.max(b));
            let level = ((peak / normalization).sqrt() * gain).min(1.0);
            if level > self.spectrum_levels[bar] {
                self.spectrum_levels[bar] = level; // fast attack
            } else {
                self.spectrum_levels[bar] *= 0.93; // slow phosphor fall
            }
        }
        self.magnitude_buffer = magnitudes;
    }

    fn spectrum(&self, width: f32, height: f32, out: &mut SegmentSink) {
        let baseline = height * 0.88;
        let bar_pitch = width / SPECTRUM_BAR_COUNT as f32;
        for (bar, &level) in self.spectrum_levels.iter().enumerate() {
            if level < 0.01 {
                continue;
            }
            let x = bar_pitch * (bar as f32 + 0.5);
            let top = baseline - level * height * 0.74;
            out.push(x, baseline, x, top, 0.35 + 0.65 * level);
        }
    }

    /// Bars radiating from a circle: bass at twelve o'clock, clockwise.
    fn spectrum_radial(&self, width: f32, height: f32, out: &mut SegmentSink) {
        let center_x = width / 2.0;
        let center_y = height / 2.0;
        let inner_radius = width.min(height) * 0.14;
        let bar_reach = width.min(height) * 0.32;
        for (bar, &level) in self.spectrum_levels.iter().enumerate() {
            if level < 0.01 {
                continue;
            }
            let angle =
                2.0 * PI * (bar as f32 + 0.5) / SPECTRUM_BAR_COUNT as f32 - PI / 2.0;
            let (sine, cosine) = angle.sin_cos();
            let outer_radius = inner_radius + level * bar_reach;
            out.push(center_x + cosine * inner_radius,
                     center_y + sine * inner_radius,
                     center_x + cosine * outer_radius,
                     center_y + sine * outer_radius,
                     0.35 + 0.65 * level);
        }
    }
}

impl Default for Computer {
    fn default() -> Self {
        Self::new()
    }
}

/// Pre-decay factor per segment: the oldest audio in a frame has nearly a
/// full frame of phosphor decay behind it already, so stamping it dimmer
/// makes trails grade continuously instead of stepping once per frame.
fn age_weight(glow_keep: f32, count: usize, index: usize) -> Option<f32> {
    if count <= 1 {
        return None;
    }
    let age = (count - 1 - index) as f32 / count as f32;
    Some(glow_keep.powf(age))
}

/// Bounded writer over the caller's (x0, y0, x1, y1, intensity) f32 buffer.
pub struct SegmentSink<'a> {
    buffer: &'a mut [f32],
    count: usize,
}

impl<'a> SegmentSink<'a> {
    pub fn new(buffer: &'a mut [f32]) -> SegmentSink<'a> {
        SegmentSink { buffer, count: 0 }
    }

    #[inline]
    fn push(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, intensity: f32) {
        let offset = self.count * 5;
        if offset + 5 <= self.buffer.len() {
            self.buffer[offset..offset + 5].copy_from_slice(&[x0, y0, x1, y1, intensity]);
            self.count += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// C ABI
// ---------------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn pc_version() -> u32 {
    API_VERSION
}

#[no_mangle]
pub extern "C" fn pc_new() -> *mut Computer {
    Box::into_raw(Box::new(Computer::new()))
}

/// # Safety
/// `handle` must come from pc_new and not have been freed.
#[no_mangle]
pub unsafe extern "C" fn pc_free(handle: *mut Computer) {
    if !handle.is_null() {
        drop(Box::from_raw(handle));
    }
}

/// # Safety
/// `handle` must come from pc_new.
#[no_mangle]
pub unsafe extern "C" fn pc_configure(handle: *mut Computer, sample_rate: u32,
                                      oversample: u32) {
    if let Some(computer) = handle.as_mut() {
        computer.configure(sample_rate, oversample);
    }
}

/// # Safety
/// `handle` must come from pc_new.
#[no_mangle]
pub unsafe extern "C" fn pc_reset(handle: *mut Computer) {
    if let Some(computer) = handle.as_mut() {
        computer.reset();
    }
}

/// Writes up to `capacity` segments (5 f32 each) into `out`; returns the
/// number written.
///
/// # Safety
/// `handle` from pc_new; `samples` valid for `sample_count` f32s; `out`
/// valid for `capacity * 5` f32s.
#[no_mangle]
pub unsafe extern "C" fn pc_compute(handle: *mut Computer, mode: u32, gain: f32,
                                    beam_energy: f32, glow_keep: f32,
                                    samples: *const f32, sample_count: usize,
                                    width: f32, height: f32,
                                    out: *mut f32, capacity: usize) -> usize {
    let (Some(computer), Some(mode)) = (handle.as_mut(), Mode::from_u32(mode)) else {
        return 0;
    };
    let samples = if samples.is_null() || sample_count == 0 {
        &[]
    } else {
        std::slice::from_raw_parts(samples, sample_count)
    };
    if out.is_null() || capacity == 0 {
        return 0;
    }
    let out = std::slice::from_raw_parts_mut(out, capacity * 5);
    let mut sink = SegmentSink::new(out);
    computer.compute(mode, gain, beam_energy, glow_keep, samples, width, height,
                     &mut sink)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn compute_vec(computer: &mut Computer, mode: Mode, samples: &[f32],
                   width: f32, height: f32) -> Vec<[f32; 5]> {
        let mut buffer = vec![0.0f32; (samples.len() / 2 + 64).max(4096) * 5
            * computer.oversample];
        let mut sink = SegmentSink::new(&mut buffer);
        let count = computer.compute(mode, 1.0, 8.0, 0.82, samples, width,
                                     height, &mut sink);
        (0..count)
            .map(|i| {
                let mut segment = [0.0; 5];
                segment.copy_from_slice(&buffer[i * 5..i * 5 + 5]);
                segment
            })
            .collect()
    }

    fn sine_samples(frames: usize) -> Vec<f32> {
        (0..frames)
            .flat_map(|i| {
                let phase = 2.0 * PI * 440.0 * i as f32 / 48000.0;
                [phase.sin() * 0.5, phase.cos() * 0.5]
            })
            .collect()
    }

    #[test]
    fn xy_segment_geometry_and_continuity() {
        let mut computer = Computer::new();
        let samples = sine_samples(800);
        let segments = compute_vec(&mut computer, Mode::Xy, &samples, 800.0, 600.0);
        assert_eq!(segments.len(), 799); // n-1 on the first frame
        // circle of radius 0.5 * gain * 0.45 * min(w,h) around center
        for segment in &segments {
            let dx = segment[0] - 400.0;
            let dy = segment[1] - 300.0;
            assert!((dx.hypot(dy) - 0.5 * 0.45 * 600.0).abs() < 2.0);
            assert!(segment[4] > 0.0 && segment[4] <= 1.0);
        }
        // second call bridges from the last beam position: n segments now
        let more = compute_vec(&mut computer, Mode::Xy, &samples, 800.0, 600.0);
        assert_eq!(more.len(), 800);
        assert_eq!(&more[0][0..2], &segments[798][2..4]);
    }

    #[test]
    fn xy45_rotates_mono_upright() {
        let mut computer = Computer::new();
        // identical channels = pure mono: xy45 must stand it upright (x const)
        let samples: Vec<f32> = (0..200)
            .flat_map(|i| {
                let value = (i as f32 / 200.0) - 0.5;
                [value, value]
            })
            .collect();
        let segments = compute_vec(&mut computer, Mode::Xy45, &samples, 1000.0, 1000.0);
        for segment in &segments {
            assert!((segment[0] - 500.0).abs() < 0.001);
            assert!((segment[2] - 500.0).abs() < 0.001);
        }
    }

    #[test]
    fn dots_intensity_scales_with_rate() {
        let mut computer = Computer::new();
        let samples = sine_samples(100);
        let base = compute_vec(&mut computer, Mode::XyDots, &samples, 500.0, 500.0);
        computer.configure(96000, 1);
        let fine = compute_vec(&mut computer, Mode::XyDots, &samples, 500.0, 500.0);
        // last dot has weight 1.0 * dot_intensity in both cases
        assert!((base.last().unwrap()[4] - 1.0).abs() < 1e-5);
        assert!((fine.last().unwrap()[4] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn waveform_triggers_and_draws_two_traces() {
        let mut computer = Computer::new();
        let samples = sine_samples(4000);
        let segments = compute_vec(&mut computer, Mode::Waveform, &samples,
                                   800.0, 600.0);
        assert!(!segments.is_empty());
        let top_trace = segments.iter().filter(|s| s[1] < 300.0).count();
        let bottom_trace = segments.iter().filter(|s| s[1] >= 300.0).count();
        assert!(top_trace > 100 && bottom_trace > 100);
        // triggered: the first x=0 point of the top trace sits at the rising
        // zero crossing of the left channel -> y == baseline (within a step)
        let first = segments.iter().find(|s| s[0] == 0.0).unwrap();
        assert!((first[1] - 600.0 * 0.28).abs() < 600.0 * 0.21 * 0.1);
    }

    #[test]
    fn spectrum_peaks_near_input_frequency() {
        let mut computer = Computer::new();
        let samples = sine_samples(2048);
        // levels update on every second compute call
        compute_vec(&mut computer, Mode::Spectrum, &samples, 1000.0, 1000.0);
        let segments = compute_vec(&mut computer, Mode::Spectrum, &[], 1000.0, 1000.0);
        assert!(!segments.is_empty());
        let strongest = segments
            .iter()
            .max_by(|a, b| a[4].partial_cmp(&b[4]).unwrap())
            .unwrap();
        // 440 Hz in the 35..18000 log sweep of 56 bars over width 1000:
        // bar = 56 * ln(440/35)/ln(18000/35) ~ 22.7 -> x ~ (22.5..23.5)*17.86
        let bar = (strongest[0] / (1000.0 / 56.0) - 0.5).round();
        assert!((21.0..=25.0).contains(&bar), "peak in bar {bar}");
    }

    #[test]
    fn upsampler_traces_finer_but_same_shape() {
        let mut coarse = Computer::new();
        let mut fine = Computer::new();
        fine.configure(48000, 4);
        let samples = sine_samples(1000);
        let base = compute_vec(&mut coarse, Mode::Xy, &samples, 1000.0, 1000.0);
        let dense = compute_vec(&mut fine, Mode::Xy, &samples, 1000.0, 1000.0);
        // ~4x the segments (minus sinc latency at the stream head)
        assert!(dense.len() > base.len() * 3 && dense.len() <= base.len() * 4 + 4);
        // all dense points still on the same circle: the interpolation is
        // band-limited reconstruction, not linear subdivision artifacts
        for segment in dense.iter().skip(50) {
            let radius = (segment[0] - 500.0).hypot(segment[1] - 500.0);
            assert!((radius - 0.5 * 0.45 * 1000.0).abs() < 3.0,
                    "off-circle: {radius}");
        }
    }

    #[test]
    fn upsampler_dc_gain_is_unity() {
        let mut upsampler = Upsampler::new(8);
        let input = vec![0.25f32; 2 * 256];
        let mut output = Vec::new();
        upsampler.process(&input, &mut output);
        let settled = &output[output.len() / 2..];
        for &value in settled {
            assert!((value - 0.25).abs() < 1e-4);
        }
    }

    #[test]
    fn capacity_is_respected() {
        let mut computer = Computer::new();
        let samples = sine_samples(500);
        let mut buffer = vec![0.0f32; 10 * 5];
        let mut sink = SegmentSink::new(&mut buffer);
        let count = computer.compute(Mode::Xy, 1.0, 8.0, 0.82, &samples,
                                     500.0, 500.0, &mut sink);
        assert_eq!(count, 10);
    }
}
