//! Plain data model of the PipeWire graph. No PipeWire types here, so it can
//! be unit-tested without a running daemon.

use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
}

#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub id: u32,
    pub name: String,
    pub description: String,
    pub media_class: String,
    /// The sound card / device object this node belongs to, and which of its
    /// profile devices (route device) the node is.
    pub device_id: Option<u32>,
    pub profile_device: Option<u32>,
    /// Created by RedoxRouter.
    pub is_owned: bool,
}

#[derive(Debug, Clone)]
pub struct PortInfo {
    pub id: u32,
    pub node_id: u32,
    pub name: String,
    pub direction: Direction,
}

#[derive(Debug, Clone)]
pub struct LinkInfo {
    pub id: u32,
    pub out_node: u32,
    pub out_port: u32,
    pub in_node: u32,
    pub in_port: u32,
}

#[derive(Debug)]
pub enum Event {
    NodeAdded(NodeInfo),
    PortAdded(PortInfo),
    LinkAdded(LinkInfo),
    /// Any global object was removed; the graph drops it if it knows the id.
    Removed(u32),
    /// RedoxRouter created a node (e.g. virtual sink).
    OwnedNodeAdded(u32),
    /// RedoxRouter created a link between these ports. The registry event for
    /// the new link follows later; that link is then marked as ours.
    OwnedAdded { out_port: u32, in_port: u32 },
    OwnedRemoved { out_port: u32, in_port: u32 },
    /// Volume-related Props of an audio node; only the fields that were present.
    Volume {
        node_id: u32,
        master: Option<f32>,
        channels: Option<Vec<f32>>,
        mute: Option<bool>,
    },
    /// Volume of a sound card route (the "hardware" volume desktop mixers use).
    RouteVolume {
        device_id: u32,
        route_device: u32,
        master: Option<f32>,
        channels: Option<Vec<f32>>,
        mute: Option<bool>,
    },
    Error(String),
}

/// UI -> PipeWire thread.
#[derive(Debug)]
pub enum Command {
    CreateLink { out_node: u32, out_port: u32, in_node: u32, in_port: u32 },
    DestroyLink { link_id: Option<u32>, out_port: u32, in_port: u32 },
    /// Set the same linear gain on all `channels` channels (0 = master volume only).
    SetVolume { node_id: u32, linear: f32, channels: usize },
    SetMute { node_id: u32, mute: bool },
    /// Destroy another program's link (`link_id`) and create our own between
    /// the same ports.
    TakeOver { link_id: u32, out_node: u32, out_port: u32, in_node: u32, in_port: u32 },
    /// Create a virtual null audio sink.
    CreateSink { name: String, description: String },
    /// Destroy a sink / node by its global ID.
    DestroySink { node_id: u32 },
}

/// Property lookup, so parsing does not depend on the pipewire crate.
pub type Props<'a> = &'a dyn Fn(&str) -> Option<String>;

impl NodeInfo {
    pub fn from_props(id: u32, get: Props) -> NodeInfo {
        NodeInfo {
            id,
            name: get("node.name").unwrap_or_default(),
            description: get("node.description")
                .or_else(|| get("node.nick"))
                .unwrap_or_default(),
            media_class: get("media.class").unwrap_or_default(),
            device_id: get("device.id").and_then(|s| s.parse().ok()),
            profile_device: get("card.profile.device").and_then(|s| s.parse().ok()),
            is_owned: get("redox.owned").as_deref() == Some("true"),
        }
    }

    /// Application streams can be controlled (volume slider); devices are
    /// only displayed.
    pub fn is_stream(&self) -> bool {
        self.media_class.starts_with("Stream/")
    }

    /// True for audio, video, or MIDI sink nodes (e.g. "Audio/Sink").
    pub fn is_sink(&self) -> bool {
        self.media_class.to_lowercase().contains("sink")
    }

    /// Best human-readable label.
    pub fn label(&self) -> &str {
        if self.description.is_empty() {
            &self.name
        } else {
            &self.description
        }
    }

    /// Stable persistence key. Different streams from the same application
    /// (e.g. Discord input vs output) can share `node.name` but have different
    /// `media.class` (e.g. "Stream/Input/Audio" vs "Stream/Output/Audio").
    pub fn key(&self) -> String {
        if self.media_class.is_empty() {
            self.name.clone()
        } else {
            format!("{}|{}", self.name, self.media_class)
        }
    }
}

impl PortInfo {
    pub fn from_props(id: u32, get: Props) -> Option<PortInfo> {
        let node_id = get("node.id")?.parse().ok()?;
        let direction = match get("port.direction")?.as_str() {
            "in" => Direction::In,
            "out" => Direction::Out,
            _ => return None,
        };
        let name = get("port.name").or_else(|| get("port.alias")).unwrap_or_default();
        Some(PortInfo { id, node_id, name, direction })
    }
}

impl LinkInfo {
    pub fn from_props(id: u32, get: Props) -> Option<LinkInfo> {
        let num = |k: &str| get(k)?.parse::<u32>().ok();
        Some(LinkInfo {
            id,
            out_node: num("link.output.node")?,
            out_port: num("link.output.port")?,
            in_node: num("link.input.node")?,
            in_port: num("link.input.port")?,
        })
    }
}

/// Volume state of an audio node, as reported by its Props.
#[derive(Debug, Clone)]
pub struct NodeVolume {
    pub master: f32,
    pub channels: Vec<f32>,
    pub mute: bool,
}

impl NodeVolume {
    fn new() -> NodeVolume {
        NodeVolume { master: 1.0, channels: Vec::new(), mute: false }
    }

    /// Update only the fields that were reported.
    fn merge(&mut self, master: Option<f32>, channels: Option<Vec<f32>>, mute: Option<bool>) {
        if let Some(m) = master {
            self.master = m;
        }
        if let Some(c) = channels {
            self.channels = c;
        }
        if let Some(m) = mute {
            self.mute = m;
        }
    }

    /// Linear gain heard: master times the average channel volume.
    pub fn effective(&self) -> f32 {
        let avg = if self.channels.is_empty() {
            1.0
        } else {
            self.channels.iter().sum::<f32>() / self.channels.len() as f32
        };
        self.master * avg
    }
}

#[derive(Default)]
pub struct Graph {
    pub nodes: BTreeMap<u32, NodeInfo>,
    pub ports: BTreeMap<u32, PortInfo>,
    pub links: BTreeMap<u32, LinkInfo>,
    /// Port pairs we asked PipeWire to link, waiting for or backing an owned link.
    pub owned_pairs: HashSet<(u32, u32)>,
    /// Ids of links that are ours. Everything else is external. Keyed by link id
    /// (not ports) so that when we take over an external link, the removal of
    /// the old link with the same ports does not clear ownership of the new one.
    pub owned_ids: HashSet<u32>,
    /// Ids of nodes created by RedoxRouter (e.g. virtual sinks).
    pub owned_node_ids: HashSet<u32>,
    /// Volume of audio nodes whose Props we watch.
    pub volumes: BTreeMap<u32, NodeVolume>,
    /// Sound card route volumes (what the desktop mixer shows) by
    /// (device id, route device number). Read-only: devices are controlled by
    /// the system volume control, not by RedoxRouter.
    pub routes: BTreeMap<(u32, u32), NodeVolume>,
    pub error: Option<String>,
}

impl Graph {
    pub fn apply(&mut self, ev: Event) {
        match ev {
            Event::NodeAdded(mut n) => {
                if self.owned_node_ids.contains(&n.id) {
                    n.is_owned = true;
                }
                self.nodes.insert(n.id, n);
            }
            Event::PortAdded(p) => {
                self.ports.insert(p.id, p);
            }
            Event::LinkAdded(l) => {
                if self.owned_pairs.contains(&(l.out_port, l.in_port)) {
                    self.owned_ids.insert(l.id);
                }
                self.links.insert(l.id, l);
            }
            Event::Removed(id) => {
                self.nodes.remove(&id);
                self.owned_node_ids.remove(&id);
                self.volumes.remove(&id);
                self.routes.retain(|k, _| k.0 != id); // a device object went away
                self.ports.remove(&id);
                if let Some(l) = self.links.remove(&id) {
                    if self.owned_ids.remove(&id) {
                        self.owned_pairs.remove(&(l.out_port, l.in_port));
                    }
                }
            }
            Event::OwnedNodeAdded(id) => {
                self.owned_node_ids.insert(id);
                if let Some(n) = self.nodes.get_mut(&id) {
                    n.is_owned = true;
                }
            }
            Event::OwnedAdded { out_port, in_port } => {
                self.owned_pairs.insert((out_port, in_port));
            }
            Event::Volume { node_id, master, channels, mute } => {
                let v = self.volumes.entry(node_id).or_insert_with(NodeVolume::new);
                v.merge(master, channels, mute);
            }
            Event::RouteVolume { device_id, route_device, master, channels, mute } => {
                let v = self
                    .routes
                    .entry((device_id, route_device))
                    .or_insert_with(NodeVolume::new);
                v.merge(master, channels, mute);
            }
            Event::OwnedRemoved { out_port, in_port } => {
                self.owned_pairs.remove(&(out_port, in_port));
            }
            Event::Error(e) => self.error = Some(e),
        }
    }

    /// The volume to show for a node: its sound card route's volume if it has
    /// one (what desktop mixers show), else its own volume.
    pub fn volume_of(&self, node: &NodeInfo) -> Option<&NodeVolume> {
        if let (Some(device_id), Some(device)) = (node.device_id, node.profile_device) {
            if let Some(v) = self.routes.get(&(device_id, device)) {
                return Some(v);
            }
        }
        self.volumes.get(&node.id)
    }

    pub fn is_owned(&self, l: &LinkInfo) -> bool {
        self.owned_ids.contains(&l.id)
    }

    pub fn is_node_owned(&self, node: &NodeInfo) -> bool {
        node.is_owned || self.owned_node_ids.contains(&node.id)
    }

    /// Existing link between exactly these two ports. Prefers our own link if
    /// both an owned and an external one exist (briefly, during a take-over).
    pub fn find_link(&self, out_port: u32, in_port: u32) -> Option<&LinkInfo> {
        let mut found = None;
        for l in self.links.values() {
            if l.out_port == out_port && l.in_port == in_port {
                if self.is_owned(l) {
                    return Some(l);
                }
                found = found.or(Some(l));
            }
        }
        found
    }

    /// Ports of a node with the given direction, ordered by port id.
    pub fn ports_of(&self, node_id: u32, dir: Direction) -> Vec<&PortInfo> {
        self.ports
            .values()
            .filter(|p| p.node_id == node_id && p.direction == dir)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn props(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn parses_port() {
        let m = props(&[("node.id", "42"), ("port.direction", "out"), ("port.name", "FL")]);
        let get = |k: &str| m.get(k).cloned();
        let p = PortInfo::from_props(7, &get).unwrap();
        assert_eq!((p.id, p.node_id, p.direction), (7, 42, Direction::Out));
        assert_eq!(p.name, "FL");
    }

    #[test]
    fn port_missing_direction_is_rejected() {
        let m = props(&[("node.id", "42")]);
        let get = |k: &str| m.get(k).cloned();
        assert!(PortInfo::from_props(7, &get).is_none());
    }

    #[test]
    fn parses_link_and_rejects_partial() {
        let m = props(&[
            ("link.output.node", "1"),
            ("link.output.port", "2"),
            ("link.input.node", "3"),
            ("link.input.port", "4"),
        ]);
        let get = |k: &str| m.get(k).cloned();
        let l = LinkInfo::from_props(9, &get).unwrap();
        assert_eq!((l.out_node, l.out_port, l.in_node, l.in_port), (1, 2, 3, 4));

        let partial = props(&[("link.output.node", "1")]);
        let get = |k: &str| partial.get(k).cloned();
        assert!(LinkInfo::from_props(9, &get).is_none());
    }

    fn link(id: u32) -> LinkInfo {
        LinkInfo { id, out_node: 1, out_port: 10, in_node: 2, in_port: 20 }
    }

    #[test]
    fn ownership_follows_link_lifetime() {
        let mut g = Graph::default();
        g.apply(Event::OwnedAdded { out_port: 10, in_port: 20 });
        g.apply(Event::LinkAdded(link(5)));
        assert!(g.is_owned(&link(5)));
        assert!(g.find_link(10, 20).is_some());
        assert!(g.find_link(20, 10).is_none());
        // The link disappears (e.g. a node went away): ownership goes with it.
        g.apply(Event::Removed(5));
        assert!(g.owned_ids.is_empty() && g.owned_pairs.is_empty());
        assert!(g.find_link(10, 20).is_none());
    }

    #[test]
    fn external_link_is_not_owned() {
        let mut g = Graph::default();
        g.apply(Event::LinkAdded(link(6)));
        assert!(!g.is_owned(&link(6)));
        g.apply(Event::OwnedAdded { out_port: 10, in_port: 20 });
        g.apply(Event::OwnedRemoved { out_port: 10, in_port: 20 });
        assert!(!g.is_owned(&link(6)));
    }

    #[test]
    fn take_over_keeps_ownership_when_old_link_is_removed() {
        let mut g = Graph::default();
        g.apply(Event::LinkAdded(link(6))); // external link
        // Take-over: backend reports ownership, then PipeWire reports the old
        // link removed and the new one added.
        g.apply(Event::OwnedAdded { out_port: 10, in_port: 20 });
        assert!(!g.is_owned(&link(6)), "old link must stay external");
        g.apply(Event::Removed(6));
        g.apply(Event::LinkAdded(link(7)));
        assert!(g.is_owned(&link(7)));
        assert_eq!(g.find_link(10, 20).map(|l| l.id), Some(7));
        assert_eq!(g.owned_ids.len(), 1);
        // Deleting our link later clears everything.
        g.apply(Event::OwnedRemoved { out_port: 10, in_port: 20 });
        g.apply(Event::Removed(7));
        assert!(g.owned_ids.is_empty() && g.owned_pairs.is_empty());
    }

    #[test]
    fn find_link_prefers_owned_when_both_exist() {
        let mut g = Graph::default();
        g.apply(Event::LinkAdded(link(6)));
        g.apply(Event::OwnedAdded { out_port: 10, in_port: 20 });
        g.apply(Event::LinkAdded(link(7)));
        assert_eq!(g.find_link(10, 20).map(|l| l.id), Some(7));
    }

    #[test]
    fn volume_events_merge_fields() {
        let mut g = Graph::default();
        g.apply(Event::Volume { node_id: 3, master: None, channels: Some(vec![0.5, 0.25]), mute: None });
        let v = &g.volumes[&3];
        assert!((v.effective() - 0.375).abs() < 1e-6); // master defaults to 1.0
        assert!(!v.mute);
        // A later event with only some fields keeps the rest.
        g.apply(Event::Volume { node_id: 3, master: Some(2.0), channels: None, mute: Some(true) });
        let v = &g.volumes[&3];
        assert_eq!(v.channels, vec![0.5, 0.25]);
        assert!((v.effective() - 0.75).abs() < 1e-6);
        assert!(v.mute);
        // No channel volumes: master alone.
        g.apply(Event::Volume { node_id: 4, master: Some(0.8), channels: None, mute: None });
        assert!((g.volumes[&4].effective() - 0.8).abs() < 1e-6);
        g.apply(Event::Removed(3));
        assert!(!g.volumes.contains_key(&3));
    }

    #[test]
    fn route_volume_beats_node_volume() {
        let mut g = Graph::default();
        let node = NodeInfo {
            id: 7,
            name: "alsa_output.x".into(),
            description: String::new(),
            media_class: "Audio/Sink".into(),
            device_id: Some(2),
            profile_device: Some(1),
            is_owned: false,
        };
        assert!(!node.is_stream());
        g.apply(Event::Volume { node_id: 7, master: None, channels: Some(vec![1.0, 1.0]), mute: None });
        assert!((g.volume_of(&node).unwrap().effective() - 1.0).abs() < 1e-6);

        g.apply(Event::RouteVolume {
            device_id: 2,
            route_device: 1,
            master: None,
            channels: Some(vec![0.25, 0.25]),
            mute: Some(true),
        });
        let v = g.volume_of(&node).unwrap();
        assert!((v.effective() - 0.25).abs() < 1e-6 && v.mute);

        // Another profile device on the same card keeps using its node volume.
        let other = NodeInfo { profile_device: Some(2), ..node.clone() };
        assert!((g.volume_of(&other).unwrap().effective() - 1.0).abs() < 1e-6);

        // The card goes away: back to the node's own volume.
        g.apply(Event::Removed(2));
        assert!((g.volume_of(&node).unwrap().effective() - 1.0).abs() < 1e-6);
        let stream = NodeInfo { media_class: "Stream/Output/Audio".into(), ..node };
        assert!(stream.is_stream());
    }

    #[test]
    fn parses_device_ids_from_node_props() {
        let m: HashMap<String, String> = [("device.id", "31"), ("card.profile.device", "4")]
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let get = |k: &str| m.get(k).cloned();
        let n = NodeInfo::from_props(1, &get);
        assert_eq!((n.device_id, n.profile_device), (Some(31), Some(4)));
    }

    #[test]
    fn removal_and_port_filtering() {
        let mut g = Graph::default();
        let mk = |id, node_id, direction| PortInfo { id, node_id, name: String::new(), direction };
        g.apply(Event::PortAdded(mk(10, 1, Direction::Out)));
        g.apply(Event::PortAdded(mk(11, 1, Direction::In)));
        g.apply(Event::PortAdded(mk(12, 2, Direction::Out)));
        assert_eq!(g.ports_of(1, Direction::Out).len(), 1);
        g.apply(Event::Removed(10));
        assert!(g.ports_of(1, Direction::Out).is_empty());
        assert_eq!(g.ports.len(), 2);
    }

    #[test]
    fn node_key_distinguishes_same_name_different_media_class() {
        let discord_out = NodeInfo {
            id: 95,
            name: "WEBRTC VoiceEngine".into(),
            description: "Discord Playback".into(),
            media_class: "Stream/Output/Audio".into(),
            device_id: None,
            profile_device: None,
            is_owned: false,
        };
        let discord_in = NodeInfo {
            id: 130,
            name: "WEBRTC VoiceEngine".into(),
            description: "Discord Capture".into(),
            media_class: "Stream/Input/Audio".into(),
            device_id: None,
            profile_device: None,
            is_owned: false,
        };
        assert_ne!(discord_out.key(), discord_in.key());
        assert_eq!(discord_out.key(), "WEBRTC VoiceEngine|Stream/Output/Audio");
        assert_eq!(discord_in.key(), "WEBRTC VoiceEngine|Stream/Input/Audio");

        let legacy_no_class = NodeInfo {
            id: 1,
            name: "simple".into(),
            description: "".into(),
            media_class: "".into(),
            device_id: None,
            profile_device: None,
            is_owned: false,
        };
        assert_eq!(legacy_no_class.key(), "simple");
    }
}
