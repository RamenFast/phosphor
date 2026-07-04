// SPDX-License-Identifier: GPL-3.0-or-later
//! Native PipeWire capture/playback (pipewire-rs), symphonia decode
//! (ffmpeg survives ONLY as the mp4 mux pipe in render), vacuum
//! routing, per-app capture, multi-app mixing, gapless preload, cover
//! art from symphonia metadata. Wave 2; riskiest port = vacuum.
//!
//! Vacuum invariants (hard-won, port verbatim):
//! - the reader paces itself: rolling deadline, re-anchor when >0.25 s
//!   behind; NEVER an `-re`-style throttle (bursts after SIGCONT).
//! - restore is sacred AND insufficient alone: every launch sweeps
//!   stale vacuum artifacts (atexit does not survive kill -9).
//! - check return codes, not stdout (pactl is silent on success).
//! - escape hatch if PW node routing misbehaves: hybrid mode keeps a
//!   pactl subprocess for module load/unload ONLY.

pub mod engine;
pub mod metadata;
pub mod mirror;
pub mod playback;
pub mod ring;
pub mod targets;

pub use engine::{AudioEngine, AudioEvent};
pub use metadata::{probe_metadata, CoverArt, TrackMetadata};
pub use ring::{SampleRing, CLIP_SECONDS, PENDING_BACKLOG_SECONDS};
pub use targets::{CaptureTarget, ConnectSpec, TargetKind};

/// The null sink apps play into during vacuum (v3's name, kept so the
/// sweep also catches leftovers from a crashed v3).
pub const VACUUM_SINK_NAME: &str = "phosphor_vacuum";
