// SPDX-License-Identifier: GPL-3.0-or-later
//! The sample ring — v3's `AudioCaptureStream` buffer laws, verbatim:
//! a *pending* buffer the scope drains (capped at 1 s of backlog so a
//! stalled UI never replays the past) and a *history* buffer holding
//! the last 10 s for snapshot/clip export. Both trim amortized: the
//! front of a contiguous buffer is only cut once the overshoot is
//! worth one big move (> cap/4), and always on a whole-frame boundary.

/// Seconds of backlog the scope may fall behind before old audio drops.
pub const PENDING_BACKLOG_SECONDS: usize = 1;
/// Seconds of rolling history kept for clip/snapshot export.
pub const CLIP_SECONDS: usize = 10;
/// Samples per stereo frame (L, R).
const FRAME: usize = 2;

pub struct SampleRing {
    pending: Vec<f32>,
    history: Vec<f32>,
    max_pending: usize, // in samples
    max_history: usize, // in samples
    sample_rate: u32,
}

impl SampleRing {
    pub fn new(sample_rate: u32) -> SampleRing {
        let mut ring = SampleRing {
            pending: Vec::new(),
            history: Vec::new(),
            max_pending: 0,
            max_history: 0,
            sample_rate: 0,
        };
        ring.configure_sample_rate(sample_rate);
        ring
    }

    /// Set the scope feed rate; caps change immediately, contents
    /// survive until the next trim (v3 semantics).
    pub fn configure_sample_rate(&mut self, sample_rate: u32) {
        self.sample_rate = sample_rate;
        self.max_pending = sample_rate as usize * FRAME * PENDING_BACKLOG_SECONDS;
        self.max_history = sample_rate as usize * FRAME * CLIP_SECONDS;
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Append interleaved stereo samples (from a PW buffer or decoder).
    pub fn push_interleaved(&mut self, samples: &[f32]) {
        self.pending.extend_from_slice(samples);
        Self::trim_front(&mut self.pending, self.max_pending);
        self.history.extend_from_slice(samples);
        Self::trim_front(&mut self.history, self.max_history);
    }

    /// Append raw little-endian f32 bytes exactly as PW hands them out.
    pub fn push_interleaved_le_bytes(&mut self, bytes: &[u8]) {
        let whole = bytes.len() - bytes.len() % 4;
        self.pending.reserve(whole / 4);
        self.history.reserve(whole / 4);
        for quad in bytes[..whole].chunks_exact(4) {
            let value = f32::from_le_bytes([quad[0], quad[1], quad[2], quad[3]]);
            self.pending.push(value);
            self.history.push(value);
        }
        Self::trim_front(&mut self.pending, self.max_pending);
        Self::trim_front(&mut self.history, self.max_history);
    }

    /// v3 `_trim_front`: cap a rolling buffer, amortized — deleting the
    /// front memmoves everything behind it, so wait until the overshoot
    /// is worth one big move instead of paying a full move per chunk.
    fn trim_front(buffer: &mut Vec<f32>, max_samples: usize) {
        if buffer.len() <= max_samples {
            return;
        }
        let mut overflow = buffer.len() - max_samples;
        if overflow > max_samples / 4 {
            overflow -= overflow % FRAME; // keep frame alignment
            buffer.drain(..overflow);
        }
    }

    /// Drain captured audio as flat interleaved samples [L, R, L, R…].
    pub fn take_stereo_samples(&mut self) -> Vec<f32> {
        let usable = self.pending.len() - self.pending.len() % FRAME;
        if usable == 0 {
            return Vec::new();
        }
        let mut taken: Vec<f32> = self.pending.drain(..usable).collect();
        if !self.pending.is_empty() {
            // a lone half-frame stays behind for its partner
            taken.truncate(usable);
        }
        taken
    }

    /// The most recent `seconds` of audio for export, frame-aligned at
    /// the front (v3 dropped a leading partial frame the same way).
    pub fn copy_history(&self, seconds: f32) -> Vec<f32> {
        let wanted_frames = (seconds * self.sample_rate as f32) as usize;
        let wanted = (wanted_frames * FRAME).min(self.history.len());
        let mut start = self.history.len() - wanted;
        start += (self.history.len() - start) % FRAME;
        self.history[start..].to_vec()
    }

    /// Pending is cleared on stop; history is kept so a clip can still
    /// be saved right after pausing (v3 law).
    pub fn clear_pending(&mut self) {
        self.pending.clear();
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_drains_whole_frames() {
        let mut ring = SampleRing::new(48_000);
        ring.push_interleaved(&[0.1, 0.2, 0.3, 0.4]);
        assert_eq!(ring.take_stereo_samples(), vec![0.1, 0.2, 0.3, 0.4]);
        assert!(ring.take_stereo_samples().is_empty());
    }

    #[test]
    fn pending_caps_at_one_second_amortized() {
        let rate = 1_000u32; // tiny rate keeps the test readable
        let mut ring = SampleRing::new(rate);
        let cap = rate as usize * 2; // samples
        // push 3 s of audio in 100-sample chunks
        for chunk_index in 0..(3 * cap / 100) {
            let base = chunk_index as f32;
            let chunk: Vec<f32> = (0..100).map(|i| base + i as f32).collect();
            ring.push_interleaved(&chunk);
            // the amortized law: never more than cap + cap/4 + one chunk
            assert!(ring.pending_len() <= cap + cap / 4 + 100);
        }
        // after a big push the buffer must have been cut back near cap
        assert!(ring.pending_len() >= cap - 1);
        // newest data survives: the last pushed value is present
        let taken = ring.take_stereo_samples();
        let last = *taken.last().unwrap();
        assert_eq!(last, (3 * cap / 100 - 1) as f32 + 99.0);
    }

    #[test]
    fn history_keeps_clip_seconds_and_alignment() {
        let rate = 100u32;
        let mut ring = SampleRing::new(rate);
        for i in 0..(CLIP_SECONDS as u32 * rate * 4) {
            ring.push_interleaved(&[i as f32, -(i as f32)]);
        }
        let hist = ring.copy_history(CLIP_SECONDS as f32);
        assert!(hist.len() <= CLIP_SECONDS * rate as usize * 2);
        assert_eq!(hist.len() % 2, 0);
        // last frame is the newest push
        let n = hist.len();
        assert_eq!(hist[n - 2], (CLIP_SECONDS as u32 * rate * 4 - 1) as f32);
        // a shorter window really is shorter
        let short = ring.copy_history(1.0);
        assert_eq!(short.len(), rate as usize * 2);
    }

    #[test]
    fn le_bytes_roundtrip() {
        let mut ring = SampleRing::new(48_000);
        let samples = [0.5f32, -0.25, 1.0, -1.0];
        let mut bytes = Vec::new();
        for s in samples {
            bytes.extend_from_slice(&s.to_le_bytes());
        }
        bytes.push(0xAA); // trailing partial value must be ignored
        ring.push_interleaved_le_bytes(&bytes);
        assert_eq!(ring.take_stereo_samples(), samples.to_vec());
    }

    #[test]
    fn clear_pending_keeps_history() {
        let mut ring = SampleRing::new(48_000);
        ring.push_interleaved(&[1.0, 2.0, 3.0, 4.0]);
        ring.clear_pending();
        assert!(ring.take_stereo_samples().is_empty());
        assert_eq!(ring.copy_history(1.0), vec![1.0, 2.0, 3.0, 4.0]);
    }
}
