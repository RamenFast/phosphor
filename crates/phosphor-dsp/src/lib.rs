// SPDX-License-Identifier: GPL-3.0-or-later
//! The one engine. samples → beam segments, all 11 v3 scope modes, the
//! kit chain, and windowed-sinc oversampling — the union of v3's two
//! engines (phosphor_signal.py and core/src/lib.rs), gated segment-by-
//! segment by tests/golden/.
//!
//! Numeric doctrine (learned from the fixture capture, do not "fix"):
//! - The xy/waveform/spectrum family is ported VERBATIM from the v3
//!   Rust core, f32 op order untouched — the native-v3 fixtures were
//!   captured from that exact code.
//! - The five Python-only modes (xy_swirl, xyz_takens, helix, ring,
//!   tunnel) compute in f64 and cast to f32 at emission, mirroring the
//!   Python reference's data flow (every value entering that math is an
//!   exact f32 widening, so uniform f64 sits far inside the 0.05 px
//!   contract).
//! - Two scaling laws coexist, as in v3: the xy family scales
//!   max-points/distance by feed × oversample (only it ever
//!   oversamples); swirl and takens scale by the feed ratio alone.
//! - Kit law: f64 phase accumulators advanced per chunk by
//!   2π·hz·frames/rate with rem_euclid; f64 trig cast to f32 BEFORE the
//!   f32 sample math; integer-sample delays; state zeroed on
//!   reset/configure.
//! - Silence is processed, not skipped; swirl phase advances by sample
//!   count, never wall clock; takens pre-lock tau = 0.004 × rate with
//!   the emit = history − 2τ warmup ramp.

use std::str::FromStr;

mod camera;
mod fft;
mod kit;
mod modes;
mod oversample;

pub use camera::Camera;
pub use kit::{KitOp, KitStage, MAX_KIT_STAGES};

use fft::Fft;
use oversample::Upsampler;

pub const BASE_SAMPLE_RATE: f32 = 48000.0;
pub(crate) const MAX_POINTS_PER_FRAME: usize = 4000;
pub(crate) const WAVEFORM_WINDOW: usize = 1600;
pub(crate) const WAVEFORM_HISTORY: usize = 8192;
pub(crate) const WAVEFORM_TRIGGER_SEARCH: usize = 2400;
pub(crate) const FFT_BASE_SIZE: usize = 1024;
pub(crate) const SPECTRUM_BAR_COUNT: usize = 56;
pub(crate) const SPECTRUM_LOW_HZ: f32 = 35.0;
pub(crate) const SPECTRUM_HIGH_HZ: f32 = 18000.0;
pub(crate) const SQRT_HALF: f32 = std::f32::consts::FRAC_1_SQRT_2;
pub(crate) const SWIRL_RADIANS_PER_SECOND: f64 = 0.35;
pub(crate) const TUNNEL_RINGS: usize = 8;
pub(crate) const TUNNEL_RING_POINTS: usize = 60;
pub(crate) const RING_TRACE_POINTS: usize = 720;
pub(crate) const TAKENS_AUTOCORR_WINDOW: usize = 4096;
pub(crate) const TAKENS_LOW_HZ: f64 = 50.0;
pub(crate) const TAKENS_HIGH_HZ: f64 = 900.0;
pub(crate) const TAKENS_TAU_SMOOTHING: f64 = 0.8;
pub(crate) const HELIX_SECONDS: f64 = 0.35;
pub(crate) const HELIX_MAX_POINTS: usize = 1800;

/// One beam segment: (x0, y0, x1, y1, intensity 0..1).
pub type Segment = [f32; 5];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Xy,
    Xy45,
    XySwirl,
    XyDots,
    XyzTakens,
    Helix,
    Waveform,
    Ring,
    Spectrum,
    SpectrumRadial,
    Tunnel,
}

impl Mode {
    pub const ALL: [Mode; 11] = [
        Mode::Xy, Mode::Xy45, Mode::XySwirl, Mode::XyDots, Mode::XyzTakens,
        Mode::Helix, Mode::Waveform, Mode::Ring, Mode::Spectrum,
        Mode::SpectrumRadial, Mode::Tunnel,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Mode::Xy => "xy",
            Mode::Xy45 => "xy45",
            Mode::XySwirl => "xy_swirl",
            Mode::XyDots => "xy_dots",
            Mode::XyzTakens => "xyz_takens",
            Mode::Helix => "helix",
            Mode::Waveform => "waveform",
            Mode::Ring => "ring",
            Mode::Spectrum => "spectrum",
            Mode::SpectrumRadial => "spectrum_radial",
            Mode::Tunnel => "tunnel",
        }
    }
}

impl FromStr for Mode {
    type Err = String;

    fn from_str(name: &str) -> Result<Mode, String> {
        Mode::ALL
            .into_iter()
            .find(|mode| mode.name() == name)
            .ok_or_else(|| format!("unknown mode '{name}' (known: {})",
                                   Mode::ALL.map(Mode::name).join(", ")))
    }
}

/// Growable segment store; compute() returns a slice into it.
pub(crate) struct SegmentBuffer {
    rows: Vec<Segment>,
}

impl SegmentBuffer {
    #[inline]
    pub(crate) fn push(&mut self, x0: f32, y0: f32, x1: f32, y1: f32,
                       intensity: f32) {
        self.rows.push([x0, y0, x1, y1, intensity]);
    }
}

pub struct Computer {
    // per-frame appearance, mirroring v3's SegmentComputer surface
    pub mode: Mode,
    pub gain: f32,
    pub beam_energy: f32,
    pub frame_glow_keep: f32,

    pub(crate) sample_rate: f32,
    pub(crate) oversample: usize,
    // xy family: distances between consecutive samples shrink as the
    // (oversampled) rate rises; normalizing them back keeps dwell-time
    // brightness identical
    pub(crate) distance_scale_effective: f32,
    pub(crate) max_points_effective: usize,
    // python-side modes scale by the feed rate alone (they never
    // oversample in v3, and the fixtures pin that)
    pub(crate) distance_scale_feed: f64,
    pub(crate) max_points_feed: usize,

    pub(crate) waveform_window: usize,
    pub(crate) waveform_history_limit: usize,
    pub(crate) waveform_trigger_search: usize,
    pub(crate) mono_history_limit: usize,
    pub(crate) autocorr_window: usize,

    pub(crate) fft: Fft,
    pub(crate) bar_bin_ranges: [(usize, usize); SPECTRUM_BAR_COUNT],
    pub(crate) magnitude_buffer: Vec<f32>,
    pub(crate) spectrum_levels: [f32; SPECTRUM_BAR_COUNT],
    pub(crate) frames_since_fft: u32,

    upsampler: Option<Upsampler>,
    upsample_buffer: Vec<f32>,
    kit: Vec<KitStage>,
    kit_buffer: Vec<f32>,

    pub(crate) last_beam: Option<(f32, f32)>,
    pub(crate) waveform_history: Vec<f32>,
    pub(crate) swirl_phase: f64,
    pub(crate) swirl_buffer: Vec<f32>,

    pub(crate) camera: Camera,
    pub(crate) mono_history: Vec<f32>,
    pub(crate) takens_tau: Option<usize>,
    pub(crate) takens_last: Option<(f64, f64, f64)>,
    pub(crate) probes_since_tau: u32,
    pub(crate) tau_planner: rustfft::FftPlanner<f64>,

    pub(crate) segments: SegmentBuffer,
}

fn bar_bin_ranges(sample_rate: f32, fft_size: usize)
                  -> [(usize, usize); SPECTRUM_BAR_COUNT] {
    let ratio = SPECTRUM_HIGH_HZ / SPECTRUM_LOW_HZ;
    let hz_per_bin = sample_rate / fft_size as f32;
    let mut ranges = [(0usize, 0usize); SPECTRUM_BAR_COUNT];
    for (bar, range) in ranges.iter_mut().enumerate() {
        let low_hz =
            SPECTRUM_LOW_HZ * ratio.powf(bar as f32 / SPECTRUM_BAR_COUNT as f32);
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
            mode: Mode::Xy,
            gain: 1.0,
            beam_energy: 8.0,
            frame_glow_keep: 0.82,
            sample_rate: BASE_SAMPLE_RATE,
            oversample: 1,
            distance_scale_effective: 1.0,
            max_points_effective: MAX_POINTS_PER_FRAME,
            distance_scale_feed: 1.0,
            max_points_feed: MAX_POINTS_PER_FRAME,
            waveform_window: WAVEFORM_WINDOW,
            waveform_history_limit: WAVEFORM_HISTORY,
            waveform_trigger_search: WAVEFORM_TRIGGER_SEARCH,
            mono_history_limit: WAVEFORM_HISTORY,
            autocorr_window: TAKENS_AUTOCORR_WINDOW,
            fft: Fft::new(FFT_BASE_SIZE),
            bar_bin_ranges: bar_bin_ranges(BASE_SAMPLE_RATE, FFT_BASE_SIZE),
            magnitude_buffer: Vec::new(),
            spectrum_levels: [0.0; SPECTRUM_BAR_COUNT],
            frames_since_fft: 0,
            upsampler: None,
            upsample_buffer: Vec::new(),
            kit: Vec::new(),
            kit_buffer: Vec::new(),
            last_beam: None,
            waveform_history: Vec::new(),
            swirl_phase: 0.0,
            swirl_buffer: Vec::new(),
            camera: Camera::default(),
            mono_history: Vec::new(),
            takens_tau: None,
            takens_last: None,
            probes_since_tau: 99,
            tau_planner: rustfft::FftPlanner::new(),
            segments: SegmentBuffer { rows: Vec::new() },
        };
        computer.set_sample_rate(BASE_SAMPLE_RATE as u32, 1);
        computer
    }

    /// Scale every rate-dependent size so a finer feed only adds detail;
    /// beam brightness, the waveform's time span, and the spectrum's
    /// tuning stay where they were tuned at 48 kHz. `oversample`
    /// multiplies the XY feed in-engine (see the doctrine above).
    pub fn set_sample_rate(&mut self, sample_rate: u32, oversample: u32) {
        let oversample = (oversample.max(1) as usize).min(16);
        self.sample_rate = sample_rate as f32;
        self.oversample = oversample;
        let feed_ratio = self.sample_rate / BASE_SAMPLE_RATE;
        let effective_ratio = feed_ratio * oversample as f32;
        self.distance_scale_effective = effective_ratio;
        self.max_points_effective =
            (MAX_POINTS_PER_FRAME as f32 * effective_ratio) as usize;
        self.distance_scale_feed = feed_ratio as f64;
        self.max_points_feed = (MAX_POINTS_PER_FRAME as f32 * feed_ratio) as usize;
        self.waveform_window = (WAVEFORM_WINDOW as f32 * feed_ratio) as usize;
        self.waveform_history_limit =
            (WAVEFORM_HISTORY as f32 * feed_ratio) as usize;
        self.waveform_trigger_search =
            (WAVEFORM_TRIGGER_SEARCH as f32 * feed_ratio) as usize;
        self.mono_history_limit = (WAVEFORM_HISTORY as f32 * feed_ratio) as usize;
        self.autocorr_window =
            (TAKENS_AUTOCORR_WINDOW as f32 * feed_ratio) as usize;
        // growing the FFT with the rate keeps the same analysis time
        // window, so bass resolution doesn't degrade at fine feed rates
        let fft_size = FFT_BASE_SIZE * (feed_ratio.round().max(1.0) as usize);
        if self.fft.size != fft_size {
            self.fft = Fft::new(fft_size);
        }
        self.bar_bin_ranges = bar_bin_ranges(self.sample_rate, fft_size);
        self.upsampler =
            if oversample > 1 { Some(Upsampler::new(oversample)) } else { None };
        self.reset();
    }

    pub fn reset(&mut self) {
        self.last_beam = None;
        self.waveform_history.clear();
        self.spectrum_levels = [0.0; SPECTRUM_BAR_COUNT];
        self.frames_since_fft = 0;
        self.swirl_phase = 0.0;
        self.mono_history.clear();
        self.takens_tau = None;
        self.takens_last = None;
        self.probes_since_tau = 99;
        if let Some(upsampler) = self.upsampler.as_mut() {
            upsampler.reset();
        }
        let sample_rate = self.sample_rate as f64;
        for stage in self.kit.iter_mut() {
            stage.reset(sample_rate);
        }
    }

    /// Aim the 3D modes' shared camera; None leaves an axis alone.
    pub fn set_camera(&mut self, yaw: Option<f64>, pitch: Option<f64>,
                      dolly: Option<f64>) {
        self.camera.set(yaw, pitch, dolly);
    }

    /// Install (or clear, with an empty slice) the signal kit chain:
    /// canonical (op, [p0..p3]) stages; state starts zeroed.
    pub fn set_kit(&mut self, stages: &[(KitOp, [f64; 4])]) {
        self.kit.clear();
        let sample_rate = self.sample_rate as f64;
        for &(op, params) in stages.iter().take(MAX_KIT_STAGES) {
            let mut stage = KitStage::new(op, params);
            stage.reset(sample_rate);
            self.kit.push(stage);
        }
    }

    /// New beam segments for this frame from this frame's new samples.
    /// The returned slice lives until the next compute() call.
    pub fn compute(&mut self, samples: &[f32], width: f32, height: f32)
                   -> &[Segment] {
        self.segments.rows.clear();
        let samples = &samples[..samples.len() - samples.len() % 2];
        if !self.kit.is_empty() && !samples.is_empty() {
            let mut buffer = std::mem::take(&mut self.kit_buffer);
            buffer.clear();
            buffer.extend_from_slice(samples);
            let sample_rate = self.sample_rate as f64;
            for stage in self.kit.iter_mut() {
                stage.process(&mut buffer, sample_rate);
            }
            self.dispatch(&buffer, width, height);
            self.kit_buffer = buffer;
        } else {
            self.dispatch(samples, width, height);
        }
        &self.segments.rows
    }

    fn dispatch(&mut self, samples: &[f32], width: f32, height: f32) {
        match self.mode {
            Mode::Xy | Mode::Xy45 | Mode::XyDots => {
                if self.upsampler.is_some() {
                    let mut buffer = std::mem::take(&mut self.upsample_buffer);
                    self.upsampler.as_mut().unwrap().process(samples, &mut buffer);
                    self.xy_modes(&buffer, width, height);
                    self.upsample_buffer = buffer;
                } else {
                    self.xy_modes(samples, width, height);
                }
            }
            Mode::XySwirl => self.swirl(samples, width, height),
            Mode::XyzTakens => self.takens(samples, width, height),
            _ => {
                self.extend_waveform_history(samples);
                match self.mode {
                    Mode::Waveform => self.waveform(width, height),
                    Mode::Helix => self.helix(width, height),
                    Mode::Ring => self.ring(width, height),
                    Mode::Tunnel => self.tunnel(width, height),
                    Mode::Spectrum => {
                        self.update_spectrum_levels();
                        self.spectrum(width, height);
                    }
                    Mode::SpectrumRadial => {
                        self.update_spectrum_levels();
                        self.spectrum_radial(width, height);
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    pub(crate) fn extend_waveform_history(&mut self, samples: &[f32]) {
        self.waveform_history.extend_from_slice(samples);
        let excess = self.waveform_history.len() as isize
            - 2 * self.waveform_history_limit as isize;
        if excess > 0 {
            self.waveform_history.drain(..excess as usize);
        }
    }
}

impl Default for Computer {
    fn default() -> Self {
        Self::new()
    }
}

impl Computer {
    /// Test hook: a kit stage's phase accumulator (property tests pin
    /// the wraparound law without exposing the chain).
    #[doc(hidden)]
    pub fn kit_phase_for_test(&self, index: usize) -> f64 {
        self.kit[index].phase
    }
}

/// Pre-decay factor per segment (f32, xy-family verbatim): the oldest
/// audio in a frame has nearly a full frame of phosphor decay behind it
/// already, so stamping it dimmer makes trails grade continuously.
pub(crate) fn age_weight(glow_keep: f32, count: usize, index: usize) -> Option<f32> {
    if count <= 1 {
        return None;
    }
    let age = (count - 1 - index) as f32 / count as f32;
    Some(glow_keep.powf(age))
}

/// The same pre-decay in f64, for the python-lineage modes (their
/// reference computed weights in f64 via numpy promotion).
pub(crate) fn age_weight64(glow_keep: f64, count: usize, index: usize) -> Option<f64> {
    if count <= 1 {
        return None;
    }
    let age = (count - 1 - index) as f64 / count as f64;
    Some(glow_keep.powf(age))
}
