// SPDX-License-Identifier: GPL-3.0-or-later
//! File playback — v3's ffmpeg|pacat pipeline as one native thread.
//!
//! One resampled stream is the whole truth: the decode thread converts
//! the file to f32 stereo at the scope pipe rate, and THE SAME chunks
//! feed the audible ring (PW playback stream) and the scope ring — the
//! picture stays sample-locked to the sound, v3's ffmpeg-rate law.
//!
//! Pacing:
//! - audible: the bounded audible ring blocks the decoder exactly like
//!   pacat's stdin pipe did. Pause = the PW stream goes inactive → the
//!   ring stays full → the decoder freezes on backpressure (v3 used
//!   SIGSTOP; the clock stops the same way).
//! - vacuum: no audible ring, the reader paces itself — the rolling
//!   deadline advances by each chunk's duration and re-anchors if we
//!   fall >0.25 s behind. NEVER an `-re`-style throttle: pauses and
//!   stalls must resume at real time instead of bursting (v3 law,
//!   learned the hard way with SIGCONT).
//!
//! Gapless (new in v4; v3 respawned the pipeline per track): when the
//! current file ends and a next path is queued, decode continues into
//! the same rings without touching the PW stream.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use rubato::Resampler;

use crate::engine::AudioEvent;
use crate::metadata::{probe_metadata_with_art, CoverArt, TrackMetadata};
use crate::ring::SampleRing;

/// Frames per decode/resample block (per channel).
const BLOCK_FRAMES: usize = 1024;
/// The audible ring holds ~100 ms — the same order of lead v3 had
/// (pacat --latency-msec=60 plus the OS pipe).
const AUDIBLE_RING_SECONDS: f64 = 0.1;
/// Re-anchor threshold for the vacuum rolling deadline (v3 law).
const REANCHOR_SECONDS: f64 = 0.25;
/// EOF drain patience (v3 waited up to 5 s for pacat to finish).
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------------------
// Audible ring: the backpressure pipe between decode and the PW stream
// ---------------------------------------------------------------------------

pub struct AudibleRing {
    buffer: Mutex<VecDeque<f32>>,
    space: Condvar,
    capacity: usize,
    closed: AtomicBool,
}

impl AudibleRing {
    pub fn new(sample_rate: u32) -> Arc<AudibleRing> {
        let capacity =
            ((sample_rate as f64 * AUDIBLE_RING_SECONDS) as usize).max(BLOCK_FRAMES) * 2;
        Arc::new(AudibleRing {
            buffer: Mutex::new(VecDeque::with_capacity(capacity + BLOCK_FRAMES * 2)),
            space: Condvar::new(),
            capacity,
            closed: AtomicBool::new(false),
        })
    }

    /// Block until the whole chunk is in (the pacing — this is pacat's
    /// stdin pipe: writes wait for room). False = ring closed.
    pub fn push_blocking(&self, samples: &[f32]) -> bool {
        let mut remaining = samples;
        let mut buffer = self.buffer.lock().unwrap();
        while !remaining.is_empty() {
            if self.closed.load(Ordering::Relaxed) {
                return false;
            }
            if buffer.len() < self.capacity {
                let take = (self.capacity - buffer.len()).min(remaining.len());
                buffer.extend(remaining[..take].iter().copied());
                remaining = &remaining[take..];
                continue;
            }
            let (guard, _timeout) = self
                .space
                .wait_timeout(buffer, Duration::from_millis(200))
                .unwrap();
            buffer = guard;
        }
        true
    }

    /// PW process callback: fill `out` (f32), zero-padding on underrun.
    /// Returns how many samples were real audio.
    pub fn pop_into(&self, out: &mut [f32]) -> usize {
        let mut buffer = self.buffer.lock().unwrap();
        let real = out.len().min(buffer.len());
        for slot in out.iter_mut().take(real) {
            *slot = buffer.pop_front().unwrap();
        }
        drop(buffer);
        if real > 0 {
            self.space.notify_one();
        }
        for slot in out.iter_mut().skip(real) {
            *slot = 0.0;
        }
        real
    }

    /// Wait until the tail has actually been played out (EOF drain).
    pub fn drain_wait(&self, timeout: Duration) {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if self.closed.load(Ordering::Relaxed) || self.buffer.lock().unwrap().is_empty() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn clear(&self) {
        self.buffer.lock().unwrap().clear();
        self.space.notify_all();
    }

    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.space.notify_all();
    }
}

// ---------------------------------------------------------------------------
// Decoders: symphonia for real audio files, a raw reader for .phos
// ---------------------------------------------------------------------------

enum TrackDecoder {
    Symphonia(SymphoniaTrack),
    Phos(PhosTrack),
}

impl TrackDecoder {
    fn open(path: &Path) -> Result<TrackDecoder, String> {
        if crate::metadata::is_phos_path(path) {
            Ok(TrackDecoder::Phos(PhosTrack::open(path)?))
        } else {
            Ok(TrackDecoder::Symphonia(SymphoniaTrack::open(path)?))
        }
    }

    fn sample_rate(&self) -> u32 {
        match self {
            TrackDecoder::Symphonia(t) => t.sample_rate,
            TrackDecoder::Phos(t) => t.rate,
        }
    }

    /// Next interleaved-stereo f32 chunk at the file's native rate.
    /// Ok(empty) = EOF.
    fn next_chunk(&mut self) -> Result<Vec<f32>, String> {
        match self {
            TrackDecoder::Symphonia(t) => t.next_chunk(),
            TrackDecoder::Phos(t) => t.next_chunk(),
        }
    }

    /// Seek; returns the actually-reached position in seconds.
    fn seek(&mut self, seconds: f64) -> f64 {
        match self {
            TrackDecoder::Symphonia(t) => t.seek(seconds),
            TrackDecoder::Phos(t) => t.seek(seconds),
        }
    }
}

struct SymphoniaTrack {
    format: Box<dyn symphonia::core::formats::FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    sample_rate: u32,
    channels: usize,
    sample_buffer: Option<symphonia::core::audio::SampleBuffer<f32>>,
}

impl SymphoniaTrack {
    fn open(path: &Path) -> Result<SymphoniaTrack, String> {
        use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
        use symphonia::core::formats::FormatOptions;
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;
        use symphonia::core::probe::Hint;

        let file =
            std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let stream = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(extension);
        }
        let format_options = FormatOptions {
            enable_gapless: true,
            ..Default::default()
        };
        let probed = symphonia::default::get_probe()
            .format(&hint, stream, &format_options, &MetadataOptions::default())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| format!("{}: no audio track", path.display()))?;
        let sample_rate = track
            .codec_params
            .sample_rate
            .ok_or_else(|| format!("{}: unknown sample rate", path.display()))?;
        let channels = track
            .codec_params
            .channels
            .map(|c| c.count())
            .unwrap_or(2)
            .max(1);
        let decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let track_id = track.id;
        Ok(SymphoniaTrack {
            format,
            decoder,
            track_id,
            sample_rate,
            channels,
            sample_buffer: None,
        })
    }

    fn next_chunk(&mut self) -> Result<Vec<f32>, String> {
        use symphonia::core::audio::SampleBuffer;
        use symphonia::core::errors::Error as SymError;
        loop {
            let packet = match self.format.next_packet() {
                Ok(packet) => packet,
                Err(SymError::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(Vec::new()); // EOF
                }
                Err(SymError::ResetRequired) => return Ok(Vec::new()),
                Err(e) => return Err(e.to_string()),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(decoded) => decoded,
                Err(SymError::DecodeError(_)) => continue, // skip bad frame
                Err(SymError::IoError(e))
                    if e.kind() == std::io::ErrorKind::UnexpectedEof =>
                {
                    return Ok(Vec::new());
                }
                Err(e) => return Err(e.to_string()),
            };
            if decoded.frames() == 0 {
                continue;
            }
            let spec = *decoded.spec();
            let needs_new = self
                .sample_buffer
                .as_ref()
                .map(|b| b.capacity() < decoded.capacity() * spec.channels.count())
                .unwrap_or(true);
            if needs_new {
                self.sample_buffer = Some(SampleBuffer::<f32>::new(
                    decoded.capacity() as u64,
                    spec,
                ));
            }
            let buffer = self.sample_buffer.as_mut().unwrap();
            buffer.copy_interleaved_ref(decoded);
            let interleaved = buffer.samples();
            let channels = spec.channels.count().max(1);
            self.channels = channels;
            // To stereo: mono duplicates; >2ch takes the front pair
            // (v3's ffmpeg downmixed; front L/R keeps the lead content).
            let frames = interleaved.len() / channels;
            let mut stereo = Vec::with_capacity(frames * 2);
            match channels {
                1 => {
                    for &sample in interleaved {
                        stereo.push(sample);
                        stereo.push(sample);
                    }
                }
                2 => stereo.extend_from_slice(interleaved),
                _ => {
                    for frame in interleaved.chunks_exact(channels) {
                        stereo.push(frame[0]);
                        stereo.push(frame[1]);
                    }
                }
            }
            return Ok(stereo);
        }
    }

    fn seek(&mut self, seconds: f64) -> f64 {
        use symphonia::core::formats::{SeekMode, SeekTo};
        use symphonia::core::units::Time;
        let target = Time::from(seconds.max(0.0));
        match self.format.seek(
            SeekMode::Accurate,
            SeekTo::Time {
                time: target,
                track_id: Some(self.track_id),
            },
        ) {
            Ok(seeked) => {
                self.decoder.reset();
                let time_base = self
                    .format
                    .tracks()
                    .iter()
                    .find(|t| t.id == self.track_id)
                    .and_then(|t| t.codec_params.time_base);
                match time_base {
                    Some(tb) => {
                        let time = tb.calc_time(seeked.actual_ts);
                        time.seconds as f64 + time.frac
                    }
                    None => seconds,
                }
            }
            Err(_) => 0.0,
        }
    }
}

struct PhosTrack {
    file: std::fs::File,
    rate: u32,
    total_frames: u64,
    read_frames: u64,
}

impl PhosTrack {
    fn open(path: &Path) -> Result<PhosTrack, String> {
        let header = phosphor_proto::phos::read_header(path)
            .map_err(|e| e.0)?
            .ok_or_else(|| format!("{}: not a .phos file", path.display()))?;
        let rate = header
            .rate()
            .ok_or_else(|| format!("{}: header has no rate", path.display()))?;
        let total_frames = header.frames().unwrap_or(u64::MAX);
        use std::io::Seek;
        let mut file =
            std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        file.seek(std::io::SeekFrom::Start(
            phosphor_proto::phos::HEADER_BYTES as u64,
        ))
        .map_err(|e| e.to_string())?;
        Ok(PhosTrack {
            file,
            rate,
            total_frames,
            read_frames: 0,
        })
    }

    fn next_chunk(&mut self) -> Result<Vec<f32>, String> {
        use std::io::Read;
        if self.read_frames >= self.total_frames {
            return Ok(Vec::new());
        }
        let frames = BLOCK_FRAMES.min((self.total_frames - self.read_frames) as usize);
        let mut raw = vec![0u8; frames * 4]; // s16le stereo
        let mut filled = 0;
        while filled < raw.len() {
            match self.file.read(&mut raw[filled..]) {
                Ok(0) => break,
                Ok(n) => filled += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.to_string()),
            }
        }
        let whole = filled - filled % 4;
        let mut samples = Vec::with_capacity(whole / 2);
        for pair in raw[..whole].chunks_exact(2) {
            let value = i16::from_le_bytes([pair[0], pair[1]]);
            // v3 divides by 32767.0 exactly — postcards must match bit-ish
            samples.push(value as f32 / phosphor_proto::phos::INT16_SCALE);
        }
        self.read_frames += (whole / 4) as u64;
        Ok(samples)
    }

    fn seek(&mut self, seconds: f64) -> f64 {
        use std::io::Seek;
        let frame = ((seconds.max(0.0) * self.rate as f64) as u64).min(self.total_frames);
        let offset = phosphor_proto::phos::HEADER_BYTES as u64 + frame * 4;
        if self.file.seek(std::io::SeekFrom::Start(offset)).is_ok() {
            self.read_frames = frame;
            frame as f64 / self.rate as f64
        } else {
            0.0
        }
    }
}

// ---------------------------------------------------------------------------
// Resampler: file rate → scope pipe rate (bypass when equal)
// ---------------------------------------------------------------------------

struct ScopeResampler {
    inner: Option<rubato::SincFixedIn<f32>>,
    input: [Vec<f32>; 2],
}

impl ScopeResampler {
    fn new(input_rate: u32, output_rate: u32) -> ScopeResampler {
        let inner = if input_rate == output_rate {
            None
        } else {
            let parameters = rubato::SincInterpolationParameters {
                sinc_len: 256,
                f_cutoff: 0.95,
                interpolation: rubato::SincInterpolationType::Cubic,
                oversampling_factor: 256,
                window: rubato::WindowFunction::BlackmanHarris2,
            };
            Some(
                rubato::SincFixedIn::<f32>::new(
                    output_rate as f64 / input_rate as f64,
                    1.0,
                    parameters,
                    BLOCK_FRAMES,
                    2,
                )
                .expect("resampler construction"),
            )
        };
        ScopeResampler {
            inner,
            input: [Vec::new(), Vec::new()],
        }
    }

    /// Feed interleaved input; emits zero or more interleaved output
    /// chunks at the pipe rate.
    fn push(&mut self, interleaved: &[f32], mut emit: impl FnMut(Vec<f32>)) {
        let Some(resampler) = self.inner.as_mut() else {
            if !interleaved.is_empty() {
                emit(interleaved.to_vec());
            }
            return;
        };
        for frame in interleaved.chunks_exact(2) {
            self.input[0].push(frame[0]);
            self.input[1].push(frame[1]);
        }
        while self.input[0].len() >= BLOCK_FRAMES {
            let left: Vec<f32> = self.input[0].drain(..BLOCK_FRAMES).collect();
            let right: Vec<f32> = self.input[1].drain(..BLOCK_FRAMES).collect();
            if let Ok(output) = resampler.process(&[left, right], None) {
                emit(interleave(&output));
            }
        }
    }

    /// Flush the tail (EOF / before a gapless splice at a new rate).
    fn finish(&mut self, mut emit: impl FnMut(Vec<f32>)) {
        let Some(resampler) = self.inner.as_mut() else { return };
        if self.input[0].is_empty() {
            return;
        }
        let waves = [self.input[0].clone(), self.input[1].clone()];
        self.input[0].clear();
        self.input[1].clear();
        if let Ok(output) = resampler.process_partial(Some(&waves), None) {
            emit(interleave(&output));
        }
    }
}

fn interleave(channels: &[Vec<f32>]) -> Vec<f32> {
    let mut out = Vec::with_capacity(channels[0].len() * 2);
    for (left, right) in channels[0].iter().zip(channels[1].iter()) {
        out.push(*left);
        out.push(*right);
    }
    out
}

// ---------------------------------------------------------------------------
// The player thread
// ---------------------------------------------------------------------------

pub enum PlayerCommand {
    /// Vacuum only — audible pause freezes via backpressure instead.
    Pause,
    Resume,
    /// Queue the next track for a gapless splice at EOF.
    SetNext(Option<PathBuf>),
    Stop,
}

pub struct PlaybackShared {
    /// Position clock in microseconds (base + frames pushed / rate).
    pub position_micros: AtomicU64,
    pub current_metadata: Mutex<TrackMetadata>,
    pub current_cover: Mutex<Option<CoverArt>>,
}

pub struct PlayerSession {
    pub control: mpsc::Sender<PlayerCommand>,
    pub shared: Arc<PlaybackShared>,
    pub audible: Option<Arc<AudibleRing>>,
    pub vacuum: bool,
    pub thread: Option<std::thread::JoinHandle<()>>,
}

pub struct PlayerConfig {
    pub path: PathBuf,
    pub seek_seconds: f64,
    pub loop_forever: bool,
    pub vacuum: bool,
    pub pipe_rate: u32,
}

pub fn spawn_player(
    config: PlayerConfig,
    scope_ring: Arc<Mutex<SampleRing>>,
    audible: Option<Arc<AudibleRing>>,
    events: mpsc::Sender<AudioEvent>,
) -> PlayerSession {
    let (control_sender, control_receiver) = mpsc::channel();
    let vacuum = config.vacuum;
    let shared = Arc::new(PlaybackShared {
        position_micros: AtomicU64::new((config.seek_seconds * 1e6) as u64),
        current_metadata: Mutex::new(TrackMetadata::default()),
        current_cover: Mutex::new(None),
    });
    let thread = {
        let shared = shared.clone();
        let audible = audible.clone();
        std::thread::Builder::new()
            .name("phosphor-audio-player".into())
            .spawn(move || {
                run_player(config, scope_ring, audible, events, shared, control_receiver);
            })
            .expect("player thread")
    };
    PlayerSession {
        control: control_sender,
        shared,
        audible,
        vacuum,
        thread: Some(thread),
    }
}

fn run_player(
    config: PlayerConfig,
    scope_ring: Arc<Mutex<SampleRing>>,
    audible: Option<Arc<AudibleRing>>,
    events: mpsc::Sender<AudioEvent>,
    shared: Arc<PlaybackShared>,
    control: mpsc::Receiver<PlayerCommand>,
) {
    let vacuum_paced = audible.is_none();
    let pipe_rate = config.pipe_rate;

    let mut current = match open_track(&config.path, &shared, &events) {
        Some(track) => track,
        None => {
            let _ = events.send(AudioEvent::PlaybackEnded);
            return;
        }
    };
    let mut base_seconds = if config.seek_seconds > 0.0 {
        current.seek(config.seek_seconds)
    } else {
        0.0
    };
    let mut resampler = ScopeResampler::new(current.sample_rate(), pipe_rate);
    let mut frames_out: u64 = 0;
    let mut next_track: Option<PathBuf> = None;

    // Vacuum pacing state: the rolling deadline (v3 law, verbatim).
    let mut deadline = Instant::now();

    'player: loop {
        // ---- control ----
        loop {
            match control.try_recv() {
                Ok(PlayerCommand::Pause) => {
                    // Park until Resume/Stop; the position clock stops
                    // because pushes stop (v3 vacuum gate).
                    loop {
                        match control.recv() {
                            Ok(PlayerCommand::Resume) => {
                                deadline = Instant::now(); // never burst
                                break;
                            }
                            Ok(PlayerCommand::Stop) | Err(_) => break 'player,
                            Ok(PlayerCommand::SetNext(path)) => next_track = path,
                            Ok(PlayerCommand::Pause) => {}
                        }
                    }
                }
                Ok(PlayerCommand::Resume) => {
                    deadline = Instant::now();
                }
                Ok(PlayerCommand::SetNext(path)) => next_track = path,
                Ok(PlayerCommand::Stop) => break 'player,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => break 'player,
            }
        }

        // ---- decode ----
        let native_chunk = match current.next_chunk() {
            Ok(chunk) => chunk,
            Err(error) => {
                eprintln!("phosphor-audio: decode error: {error}");
                Vec::new()
            }
        };

        if native_chunk.is_empty() {
            // EOF
            if config.loop_forever {
                current.seek(0.0);
                continue; // position keeps counting up, v3 behavior
            }
            if let Some(next_path) = next_track.take() {
                // Gapless splice: flush the old tail into the rings,
                // open the next file, keep going. (The position clock
                // resets the moment the next track opens.)
                let mut tail: Vec<Vec<f32>> = Vec::new();
                resampler.finish(|chunk| tail.push(chunk));
                for chunk in tail {
                    if !push_chunk(
                        &chunk, &audible, &scope_ring, vacuum_paced,
                        &mut deadline, pipe_rate,
                    ) {
                        break 'player;
                    }
                }
                match open_track(&next_path, &shared, &events) {
                    Some(track) => {
                        current = track;
                        resampler = ScopeResampler::new(current.sample_rate(), pipe_rate);
                        base_seconds = 0.0;
                        frames_out = 0;
                        shared.position_micros.store(0, Ordering::Relaxed);
                        continue;
                    }
                    None => {
                        let _ = events.send(AudioEvent::PlaybackEnded);
                        break 'player;
                    }
                }
            }
            // True end: flush, drain, report.
            let mut tail: Vec<Vec<f32>> = Vec::new();
            resampler.finish(|chunk| tail.push(chunk));
            for chunk in tail {
                if !push_chunk(
                    &chunk, &audible, &scope_ring, vacuum_paced,
                    &mut deadline, pipe_rate,
                ) {
                    break 'player;
                }
                frames_out += (chunk.len() / 2) as u64;
                let seconds = base_seconds + frames_out as f64 / pipe_rate as f64;
                shared
                    .position_micros
                    .store((seconds * 1e6) as u64, Ordering::Relaxed);
            }
            if let Some(ring) = &audible {
                ring.drain_wait(DRAIN_TIMEOUT); // let the tail play out
            }
            let _ = events.send(AudioEvent::PlaybackEnded);
            break;
        }

        // ---- resample + push ----
        let mut out_chunks: Vec<Vec<f32>> = Vec::new();
        resampler.push(&native_chunk, |chunk| out_chunks.push(chunk));
        for chunk in out_chunks {
            if !push_chunk(
                &chunk, &audible, &scope_ring, vacuum_paced,
                &mut deadline, pipe_rate,
            ) {
                break 'player;
            }
            frames_out += (chunk.len() / 2) as u64;
            let seconds = base_seconds + frames_out as f64 / pipe_rate as f64;
            shared
                .position_micros
                .store((seconds * 1e6) as u64, Ordering::Relaxed);
        }
    }
}

/// One chunk into the world. Audible first (its backpressure is the
/// clock), scope second — the beam never runs ahead of the ear by more
/// than the ring depth, exactly like pacat's pipe. Returns false when
/// the audible ring is closed (stop).
fn push_chunk(
    chunk: &[f32],
    audible: &Option<Arc<AudibleRing>>,
    scope_ring: &Arc<Mutex<SampleRing>>,
    vacuum_paced: bool,
    deadline: &mut Instant,
    pipe_rate: u32,
) -> bool {
    if vacuum_paced {
        // Rolling deadline: sleep up to it; if we're >0.25 s late,
        // re-anchor instead of bursting to catch up (the -re failure).
        let now = Instant::now();
        if *deadline > now {
            std::thread::sleep(*deadline - now);
        } else if now.duration_since(*deadline).as_secs_f64() > REANCHOR_SECONDS {
            *deadline = now;
        }
    }
    if let Some(ring) = audible
        && !ring.push_blocking(chunk)
    {
        return false;
    }
    scope_ring.lock().unwrap().push_interleaved(chunk);
    if vacuum_paced {
        *deadline += Duration::from_secs_f64(chunk.len() as f64 / 2.0 / pipe_rate as f64);
    }
    true
}

fn open_track(
    path: &Path,
    shared: &Arc<PlaybackShared>,
    events: &mpsc::Sender<AudioEvent>,
) -> Option<TrackDecoder> {
    match TrackDecoder::open(path) {
        Ok(track) => {
            let (metadata, art) = probe_metadata_with_art(path);
            *shared.current_metadata.lock().unwrap() = metadata;
            *shared.current_cover.lock().unwrap() = art;
            let _ = events.send(AudioEvent::TrackStarted {
                path: path.to_path_buf(),
            });
            Some(track)
        }
        Err(error) => {
            eprintln!("phosphor-audio: open failed: {error}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audible_ring_backpressure_and_pop() {
        let ring = AudibleRing::new(48_000);
        assert!(ring.push_blocking(&[0.5f32; 512]));
        let mut out = [0f32; 256];
        assert_eq!(ring.pop_into(&mut out), 256);
        assert_eq!(out[0], 0.5);
        // underrun zero-pads
        let mut big = [1.0f32; 1024];
        let real = ring.pop_into(&mut big);
        assert_eq!(real, 256);
        assert_eq!(big[real], 0.0);
        assert_eq!(*big.last().unwrap(), 0.0);
    }

    #[test]
    fn audible_ring_close_unblocks_push() {
        let ring = AudibleRing::new(1_000); // tiny: capacity ~200 samples
        let ring2 = ring.clone();
        let pusher = std::thread::spawn(move || {
            // way over capacity: must block until close, then bail
            ring2.push_blocking(&vec![0.0f32; 100_000])
        });
        std::thread::sleep(Duration::from_millis(60));
        ring.close();
        assert!(!pusher.join().unwrap());
    }

    #[test]
    fn resampler_bypass_at_equal_rates() {
        let mut resampler = ScopeResampler::new(48_000, 48_000);
        let mut got = Vec::new();
        resampler.push(&[1.0, 2.0, 3.0, 4.0], |c| got.extend(c));
        assert_eq!(got, vec![1.0, 2.0, 3.0, 4.0]);
        assert!(resampler.inner.is_none());
    }

    #[test]
    fn resampler_ratio_roughly_holds() {
        let mut resampler = ScopeResampler::new(48_000, 96_000);
        let mut out_samples = 0usize;
        let input: Vec<f32> = (0..48_000 * 2)
            .map(|i| ((i / 2) as f32 * 0.01).sin())
            .collect();
        resampler.push(&input, |c| out_samples += c.len());
        resampler.finish(|c| out_samples += c.len());
        let expected = input.len() * 2;
        let tolerance = expected / 20; // sinc latency etc.
        assert!(
            (out_samples as i64 - expected as i64).unsigned_abs() as usize <= tolerance,
            "{out_samples} vs {expected}"
        );
    }
}
