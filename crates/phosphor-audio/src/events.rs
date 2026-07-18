// SPDX-License-Identifier: GPL-3.0-or-later
//! Engine → shell events. Pure std — lives outside the PipeWire-gated engine so
//! platform ports (phosphor-mobil3) consuming `default-features = false` keep the
//! event vocabulary without the session backend.

/// Events the engine reports back to the shell (poll each frame).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioEvent {
    /// The capturable-target list changed (new app, device gone…).
    TargetsChanged,
    /// The running capture stream ended on its own.
    StreamEnded,
    /// The default sink changed (v3 followed it for the ⭐ entry).
    DefaultSinkChanged,
    /// File playback reached its true end on its own (never sent for
    /// an explicit stop — v3's on_stream_ended contract).
    PlaybackEnded,
    /// A track began decoding (first play or a gapless splice);
    /// metadata + cover art are ready to read.
    TrackStarted { path: std::path::PathBuf },
}
