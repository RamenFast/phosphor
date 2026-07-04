// SPDX-License-Identifier: GPL-3.0-or-later
//! A live mirror of the PipeWire graph — the registry thread applies
//! add/update/remove events, everyone else takes cheap snapshots. This
//! replaces v3's "re-run pactl and re-parse" refresh: the combo's
//! refresh button just re-reads the mirror, which is already current.
//!
//! Pure data + pure mutations, so the identity/ordering laws are unit
//! tested without a server.

use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NodeClass {
    /// `Stream/Output/Audio` — a playing app (v3's "sink input").
    AppStream,
    /// `Audio/Sink` — an output whose monitor can be scoped.
    Sink,
    /// `Audio/Source` — a microphone/line-in.
    Source,
}

#[derive(Clone, Debug)]
pub struct NodeInfo {
    pub global_id: u32,
    pub serial: Option<u64>,
    pub class: NodeClass,
    pub node_name: String,
    pub description: Option<String>,
    pub app_name: Option<String>,
    pub media_name: Option<String>,
    /// Prior explicit routing target ("target.object" prop at announce),
    /// so a vacuum release can restore *exactly* what was there.
    pub prior_target: Option<String>,
    /// Announce order — keeps the app list stable the way v3's
    /// sink-input listing order was.
    pub order: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct LinkInfo {
    pub global_id: u32,
    pub output_node: u32,
    pub input_node: u32,
}

#[derive(Default)]
pub struct GraphMirror {
    nodes: HashMap<u32, NodeInfo>,
    links: HashMap<u32, LinkInfo>,
    /// node.name of the default sink (from the "default" metadata).
    pub default_sink: Option<String>,
    next_order: u64,
}

/// Everything a registry `global` announce tells us about a node.
#[derive(Default)]
pub struct NodeAnnounce<'a> {
    pub serial: Option<u64>,
    pub node_name: &'a str,
    pub description: Option<&'a str>,
    pub app_name: Option<&'a str>,
    pub media_name: Option<&'a str>,
    pub prior_target: Option<&'a str>,
}

impl GraphMirror {
    pub fn upsert_node(&mut self, global_id: u32, class: NodeClass, announce: NodeAnnounce) {
        let order = self
            .nodes
            .get(&global_id)
            .map(|n| n.order)
            .unwrap_or_else(|| {
                self.next_order += 1;
                self.next_order
            });
        self.nodes.insert(
            global_id,
            NodeInfo {
                global_id,
                serial: announce.serial,
                class,
                node_name: announce.node_name.to_string(),
                description: announce.description.map(str::to_string),
                app_name: announce.app_name.map(str::to_string),
                media_name: announce.media_name.map(str::to_string),
                prior_target: announce.prior_target.map(str::to_string),
                order,
            },
        );
    }

    /// Refresh the live-updating props (node info events: the song title
    /// in `media.name` changes mid-stream; registry globals do not).
    pub fn update_node_labels(
        &mut self,
        global_id: u32,
        app_name: Option<&str>,
        media_name: Option<&str>,
    ) -> bool {
        let Some(node) = self.nodes.get_mut(&global_id) else {
            return false;
        };
        let mut changed = false;
        if let Some(app) = app_name
            && node.app_name.as_deref() != Some(app)
        {
            node.app_name = Some(app.to_string());
            changed = true;
        }
        if let Some(media) = media_name
            && node.media_name.as_deref() != Some(media)
        {
            node.media_name = Some(media.to_string());
            changed = true;
        }
        changed
    }

    pub fn upsert_link(&mut self, global_id: u32, output_node: u32, input_node: u32) {
        self.links.insert(
            global_id,
            LinkInfo {
                global_id,
                output_node,
                input_node,
            },
        );
    }

    /// Remove whatever this global was. Returns the node if one died.
    pub fn remove_global(&mut self, global_id: u32) -> Option<NodeInfo> {
        self.links.remove(&global_id);
        self.nodes.remove(&global_id)
    }

    pub fn node(&self, global_id: u32) -> Option<&NodeInfo> {
        self.nodes.get(&global_id)
    }

    pub fn nodes_of_class(&self, class: NodeClass) -> Vec<&NodeInfo> {
        let mut list: Vec<&NodeInfo> =
            self.nodes.values().filter(|n| n.class == class).collect();
        list.sort_by_key(|n| n.order);
        list
    }

    pub fn find_node_by_name(&self, class: NodeClass, name: &str) -> Option<&NodeInfo> {
        self.nodes
            .values()
            .filter(|n| n.class == class && n.node_name == name)
            .min_by_key(|n| n.order)
    }

    /// Which node does this node currently feed? (Follows the links —
    /// used by the vacuum gate to *verify* a move, not to guess state.)
    pub fn link_targets_of(&self, output_node_id: u32) -> Vec<u32> {
        let mut targets: Vec<u32> = self
            .links
            .values()
            .filter(|l| l.output_node == output_node_id)
            .map(|l| l.input_node)
            .collect();
        targets.sort_unstable();
        targets.dedup();
        targets
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mirror_with_apps() -> GraphMirror {
        let mut m = GraphMirror::default();
        m.upsert_node(10, NodeClass::AppStream, NodeAnnounce {
            serial: Some(110), node_name: "firefox",
            app_name: Some("Firefox"), media_name: Some("Song A"),
            ..Default::default()
        });
        m.upsert_node(11, NodeClass::AppStream, NodeAnnounce {
            serial: Some(111), node_name: "mpv",
            app_name: Some("mpv"), ..Default::default()
        });
        m.upsert_node(20, NodeClass::Sink, NodeAnnounce {
            serial: Some(120), node_name: "alsa_output.analog",
            description: Some("Analog Stereo"), ..Default::default()
        });
        m
    }

    #[test]
    fn announce_order_is_stable_across_updates() {
        let mut m = mirror_with_apps();
        m.upsert_node(10, NodeClass::AppStream, NodeAnnounce {
            serial: Some(110), node_name: "firefox",
            app_name: Some("Firefox"), media_name: Some("Song B"),
            ..Default::default()
        });
        let apps = m.nodes_of_class(NodeClass::AppStream);
        assert_eq!(apps[0].global_id, 10, "update must not reorder");
        assert_eq!(apps[0].media_name.as_deref(), Some("Song B"));
    }

    #[test]
    fn label_update_reports_change() {
        let mut m = mirror_with_apps();
        assert!(m.update_node_labels(10, None, Some("Song C")));
        assert!(!m.update_node_labels(10, None, Some("Song C")));
        assert!(!m.update_node_labels(999, None, Some("x")));
    }

    #[test]
    fn links_track_and_remove() {
        let mut m = mirror_with_apps();
        m.upsert_link(30, 10, 20);
        assert_eq!(m.link_targets_of(10), vec![20]);
        m.remove_global(30);
        assert!(m.link_targets_of(10).is_empty());
        assert!(m.remove_global(10).is_some());
        assert!(m.remove_global(10).is_none());
    }
}
