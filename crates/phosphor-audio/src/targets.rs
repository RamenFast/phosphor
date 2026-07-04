// SPDX-License-Identifier: GPL-3.0-or-later
//! Capture targets — v3's identity scheme, verbatim, so settings
//! migrate untouched (phosphor_audio.py:50–116 is the spec):
//!
//! - combo ids are `device:<pulse-source-name>` / `app:<stable-key>`;
//!   a sink's monitor keeps the pulse spelling `<sink-node-name>.monitor`
//!   because that is exactly what v3 wrote into settings.json.
//! - app targets are keyed by application NAME, not stream index/serial:
//!   indexes are reassigned every time playback restarts, the name stays
//!   put, letting the scope re-find "Google Chrome" forever. Two streams
//!   from one app disambiguate with `+` suffixes in announce order.
//! - list order: playing apps first (announce order), then monitors
//!   sorted by label, then mics sorted by label.

use crate::mirror::{GraphMirror, NodeClass, NodeInfo};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TargetKind {
    Device,
    App,
}

/// How the engine should actually connect a capture stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectSpec {
    /// Capture a sink's monitor ports (`stream.capture.sink = true`).
    SinkMonitor { node_name: String },
    /// Capture a real source (mic / line-in) directly.
    SourceDevice { node_name: String },
    /// Capture one playing app's stream node.
    AppStream { global_id: u32, serial: u64 },
}

#[derive(Clone, Debug)]
pub struct CaptureTarget {
    pub kind: TargetKind,
    pub label: String,
    pub stable_key: String,
    pub connect: ConnectSpec,
}

impl CaptureTarget {
    /// v3's persistent identity: `f"{kind}:{stable_key}"`.
    pub fn combo_id(&self) -> String {
        match self.kind {
            TargetKind::Device => format!("device:{}", self.stable_key),
            TargetKind::App => format!("app:{}", self.stable_key),
        }
    }
}

/// Assign v3 stable keys to the current app streams: application.name,
/// `+`-suffixed on collision, in announce order. One function is the
/// single source of truth for both listing and combo-id resolution.
fn app_keys(mirror: &GraphMirror) -> Vec<(String, NodeInfo)> {
    let mut seen: Vec<String> = Vec::new();
    let mut keyed = Vec::new();
    for node in mirror.nodes_of_class(NodeClass::AppStream) {
        let mut key = node
            .app_name
            .clone()
            .unwrap_or_else(|| format!("stream-{}", node.serial.unwrap_or(node.global_id as u64)));
        while seen.contains(&key) {
            key.push('+'); // two streams from one app
        }
        seen.push(key.clone());
        keyed.push((key, node.clone()));
    }
    keyed
}

fn app_label(node: &NodeInfo) -> String {
    let parts: Vec<&str> = [node.app_name.as_deref(), node.media_name.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    let body = if parts.is_empty() {
        format!(
            "stream #{}",
            node.serial.unwrap_or(node.global_id as u64)
        )
    } else {
        parts.join(" — ")
    };
    format!("APP · {body}")
}

/// All capturable things: playing apps first, then monitors, then mics.
pub fn list_capture_targets(mirror: &GraphMirror) -> Vec<CaptureTarget> {
    let mut targets = Vec::new();

    for (key, node) in app_keys(mirror) {
        let Some(serial) = node.serial else { continue };
        targets.push(CaptureTarget {
            kind: TargetKind::App,
            label: app_label(&node),
            stable_key: key,
            connect: ConnectSpec::AppStream {
                global_id: node.global_id,
                serial,
            },
        });
    }

    let mut monitors: Vec<CaptureTarget> = mirror
        .nodes_of_class(NodeClass::Sink)
        .into_iter()
        .map(|node| {
            let description = node
                .description
                .clone()
                .unwrap_or_else(|| node.node_name.clone());
            CaptureTarget {
                kind: TargetKind::Device,
                label: format!("OUT · {description}"),
                stable_key: format!("{}.monitor", node.node_name),
                connect: ConnectSpec::SinkMonitor {
                    node_name: node.node_name.clone(),
                },
            }
        })
        .collect();
    monitors.sort_by(|a, b| a.label.cmp(&b.label));

    let mut microphones: Vec<CaptureTarget> = mirror
        .nodes_of_class(NodeClass::Source)
        .into_iter()
        .map(|node| {
            let description = node
                .description
                .clone()
                .unwrap_or_else(|| node.node_name.clone());
            CaptureTarget {
                kind: TargetKind::Device,
                label: format!("IN · {description}"),
                stable_key: node.node_name.clone(),
                connect: ConnectSpec::SourceDevice {
                    node_name: node.node_name.clone(),
                },
            }
        })
        .collect();
    microphones.sort_by(|a, b| a.label.cmp(&b.label));

    targets.extend(monitors);
    targets.extend(microphones);
    targets
}

/// v3 `default_monitor_target_id`: `device:{default_sink}.monitor`.
pub fn default_monitor_target_id(mirror: &GraphMirror) -> Option<String> {
    mirror
        .default_sink
        .as_ref()
        .map(|sink| format!("device:{sink}.monitor"))
}

/// Resolve a persisted combo id against the current graph. Returns
/// None when the target is gone (v3 showed it as unavailable).
pub fn resolve_combo_id(mirror: &GraphMirror, combo_id: &str) -> Option<ConnectSpec> {
    if let Some(name) = combo_id.strip_prefix("device:") {
        if let Some(sink_name) = name.strip_suffix(".monitor") {
            // Prefer the sink whose monitor this is; a real source that
            // happens to end in ".monitor" would be a pulse artifact.
            if mirror.find_node_by_name(NodeClass::Sink, sink_name).is_some() {
                return Some(ConnectSpec::SinkMonitor {
                    node_name: sink_name.to_string(),
                });
            }
            return None;
        }
        return mirror
            .find_node_by_name(NodeClass::Source, name)
            .map(|node| ConnectSpec::SourceDevice {
                node_name: node.node_name.clone(),
            });
    }
    if let Some(wanted_key) = combo_id.strip_prefix("app:") {
        for (key, node) in app_keys(mirror) {
            if key == wanted_key {
                return node.serial.map(|serial| ConnectSpec::AppStream {
                    global_id: node.global_id,
                    serial,
                });
            }
        }
        return None;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mirror::GraphMirror;

    use crate::mirror::NodeAnnounce;

    fn fixture() -> GraphMirror {
        let mut m = GraphMirror::default();
        // announce order: chrome, chrome again (two streams), mpv
        m.upsert_node(10, NodeClass::AppStream, NodeAnnounce {
            serial: Some(110), node_name: "chrome-node",
            app_name: Some("Google Chrome"), media_name: Some("Song A"),
            ..Default::default()
        });
        m.upsert_node(11, NodeClass::AppStream, NodeAnnounce {
            serial: Some(111), node_name: "chrome-node-2",
            app_name: Some("Google Chrome"), ..Default::default()
        });
        m.upsert_node(12, NodeClass::AppStream, NodeAnnounce {
            serial: Some(112), node_name: "mpv-node",
            media_name: Some("movie.mkv"), ..Default::default()
        });
        m.upsert_node(20, NodeClass::Sink, NodeAnnounce {
            serial: Some(120), node_name: "alsa_output.b",
            description: Some("Zeta Analog"), ..Default::default()
        });
        m.upsert_node(21, NodeClass::Sink, NodeAnnounce {
            serial: Some(121), node_name: "alsa_output.a",
            description: Some("Alpha HDMI"), ..Default::default()
        });
        m.upsert_node(30, NodeClass::Source, NodeAnnounce {
            serial: Some(130), node_name: "alsa_input.mic",
            description: Some("Q2U Microphone"), ..Default::default()
        });
        m.default_sink = Some("alsa_output.b".to_string());
        m
    }

    #[test]
    fn ordering_apps_then_monitors_then_mics() {
        let targets = list_capture_targets(&fixture());
        let ids: Vec<String> = targets.iter().map(|t| t.combo_id()).collect();
        assert_eq!(
            ids,
            vec![
                "app:Google Chrome",
                "app:Google Chrome+",
                "app:stream-112",
                "device:alsa_output.a.monitor", // monitors sorted by label
                "device:alsa_output.b.monitor",
                "device:alsa_input.mic",
            ]
        );
    }

    #[test]
    fn labels_match_v3_shapes() {
        let targets = list_capture_targets(&fixture());
        assert_eq!(targets[0].label, "APP · Google Chrome — Song A");
        assert_eq!(targets[1].label, "APP · Google Chrome");
        assert_eq!(targets[2].label, "APP · movie.mkv");
        assert_eq!(targets[3].label, "OUT · Alpha HDMI");
        assert_eq!(targets[5].label, "IN · Q2U Microphone");
    }

    #[test]
    fn default_monitor_id_matches_v3() {
        assert_eq!(
            default_monitor_target_id(&fixture()).as_deref(),
            Some("device:alsa_output.b.monitor")
        );
    }

    #[test]
    fn resolve_roundtrips_every_listed_target() {
        let mirror = fixture();
        for target in list_capture_targets(&mirror) {
            let resolved = resolve_combo_id(&mirror, &target.combo_id())
                .unwrap_or_else(|| panic!("unresolvable: {}", target.combo_id()));
            assert_eq!(resolved, target.connect, "{}", target.combo_id());
        }
    }

    #[test]
    fn resolve_misses_are_none_not_wrong() {
        let mirror = fixture();
        assert!(resolve_combo_id(&mirror, "device:gone.monitor").is_none());
        assert!(resolve_combo_id(&mirror, "app:Spotify").is_none());
        assert!(resolve_combo_id(&mirror, "garbage").is_none());
    }

    #[test]
    fn dedup_keys_follow_announce_order_after_first_stream_ends() {
        let mut mirror = fixture();
        // Chrome's first stream ends: the second stream now owns the
        // bare key — same as v3 re-listing sink-inputs.
        mirror.remove_global(10);
        let spec = resolve_combo_id(&mirror, "app:Google Chrome").unwrap();
        assert_eq!(
            spec,
            ConnectSpec::AppStream { global_id: 11, serial: 111 }
        );
    }
}
