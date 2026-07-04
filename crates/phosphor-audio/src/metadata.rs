// SPDX-License-Identifier: GPL-3.0-or-later
//! Track metadata probing — v3's ffprobe replaced by symphonia (ffmpeg
//! survives only as the mp4 mux pipe in `render`). One call serves the
//! seek slider and the now-playing overlay, exactly like v3's
//! `probe_metadata`; `.phos` postcards answer from their header
//! ("trace by <credit>" is the artist line, v3 law).

use std::path::Path;

use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTagKey, StandardVisualKey};
use symphonia::core::probe::Hint;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrackMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverArt {
    pub data: Vec<u8>,
    pub media_type: String,
}

pub fn is_phos_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("phos"))
}

/// Tags + duration; missing tags come back as None (v3 shape).
pub fn probe_metadata(path: &Path) -> TrackMetadata {
    if is_phos_path(path) {
        return phos_metadata(path);
    }
    let (metadata, _art) = probe_symphonia(path, false);
    metadata
}

/// Metadata plus embedded cover art (the playing track wants both).
pub fn probe_metadata_with_art(path: &Path) -> (TrackMetadata, Option<CoverArt>) {
    if is_phos_path(path) {
        return (phos_metadata(path), None);
    }
    probe_symphonia(path, true)
}

fn phos_metadata(path: &Path) -> TrackMetadata {
    let Ok(Some(header)) = phosphor_proto::phos::read_header(path) else {
        return TrackMetadata::default();
    };
    let text = |key: &str| {
        header
            .get(key)
            .and_then(|f| f.as_text())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let duration = match (header.frames(), header.rate()) {
        (Some(frames), Some(rate)) if rate > 0 => Some(frames as f64 / rate as f64),
        _ => None,
    };
    let fallback_title = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_string);
    TrackMetadata {
        title: text("title").or_else(|| text("source")).or(fallback_title),
        artist: text("credit").map(|credit| format!("trace by {credit}")),
        album: None,
        duration,
    }
}

fn probe_symphonia(path: &Path, want_art: bool) -> (TrackMetadata, Option<CoverArt>) {
    let Ok(file) = std::fs::File::open(path) else {
        return (TrackMetadata::default(), None);
    };
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(extension) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(extension);
    }
    let Ok(mut probed) = symphonia::default::get_probe().format(
        &hint,
        stream,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    ) else {
        return (TrackMetadata::default(), None);
    };

    let mut metadata = TrackMetadata::default();
    let mut art = None;

    // Duration from the default audio track's frame count.
    if let Some(track) = probed.format.default_track() {
        let params = &track.codec_params;
        if let (Some(frames), Some(rate)) = (params.n_frames, params.sample_rate)
            && rate > 0
        {
            metadata.duration = Some(frames as f64 / rate as f64);
        }
    }

    // Container-side metadata (ID3v2 arrives here), then format-side.
    let mut apply = |revision: &MetadataRevision| {
        for tag in revision.tags() {
            match tag.std_key {
                Some(StandardTagKey::TrackTitle) if metadata.title.is_none() => {
                    metadata.title = Some(tag.value.to_string());
                }
                Some(StandardTagKey::Artist) if metadata.artist.is_none() => {
                    metadata.artist = Some(tag.value.to_string());
                }
                Some(StandardTagKey::Album) if metadata.album.is_none() => {
                    metadata.album = Some(tag.value.to_string());
                }
                _ => {}
            }
        }
        if want_art && art.is_none() {
            let preferred = revision
                .visuals()
                .iter()
                .find(|v| v.usage == Some(StandardVisualKey::FrontCover))
                .or_else(|| revision.visuals().first());
            if let Some(visual) = preferred {
                art = Some(CoverArt {
                    data: visual.data.to_vec(),
                    media_type: visual.media_type.clone(),
                });
            }
        }
    };

    if let Some(revision) = probed.metadata.get().as_ref().and_then(|m| m.current()) {
        apply(revision);
    }
    if let Some(revision) = probed.format.metadata().current() {
        apply(revision);
    }
    (metadata, art)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phos_extension_detection_is_case_insensitive() {
        assert!(is_phos_path(Path::new("/x/y/trace.phos")));
        assert!(is_phos_path(Path::new("/x/y/TRACE.PHOS")));
        assert!(!is_phos_path(Path::new("/x/y/song.flac")));
        assert!(!is_phos_path(Path::new("/x/y/phos"))); // no extension
    }

    #[test]
    fn missing_file_yields_all_none() {
        let m = probe_metadata(Path::new("/does/not/exist.flac"));
        assert_eq!(m, TrackMetadata::default());
    }
}
