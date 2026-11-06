//! Saved links: connections stored by node name and port name (ids change every
//! session), re-created automatically whenever both ends exist.

use crate::graph::{Direction, Graph};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedLink {
    pub out_node: String,
    pub out_port: String,
    pub in_node: String,
    pub in_port: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

impl SavedLink {
    fn same_ends(&self, o: &SavedLink) -> bool {
        self.out_node == o.out_node
            && self.out_port == o.out_port
            && self.in_node == o.in_node
            && self.in_port == o.in_port
    }

    pub fn label(&self) -> String {
        format!("{}:{}  ->  {}:{}", self.out_node, self.out_port, self.in_node, self.in_port)
    }
}

/// Describe the link between two live port ids by names.
pub fn from_ports(g: &Graph, out_port: u32, in_port: u32) -> Option<SavedLink> {
    let (po, pi) = (g.ports.get(&out_port)?, g.ports.get(&in_port)?);
    Some(SavedLink {
        out_node: g.nodes.get(&po.node_id)?.name.clone(),
        out_port: po.name.clone(),
        in_node: g.nodes.get(&pi.node_id)?.name.clone(),
        in_port: pi.name.clone(),
        enabled: true,
    })
}

/// Add (or re-enable) a saved link. Returns true if anything changed.
pub fn add(links: &mut Vec<SavedLink>, l: SavedLink) -> bool {
    match links.iter_mut().find(|x| x.same_ends(&l)) {
        Some(x) if x.enabled => false,
        Some(x) => {
            x.enabled = true;
            true
        }
        None => {
            links.push(l);
            true
        }
    }
}

/// Remove the saved link with these ends. Returns true if one was removed.
pub fn remove(links: &mut Vec<SavedLink>, l: &SavedLink) -> bool {
    let before = links.len();
    links.retain(|x| !x.same_ends(l));
    links.len() != before
}

fn port_id(g: &Graph, node: &str, port: &str, dir: Direction) -> Option<u32> {
    // Lowest id first: with duplicate node names the oldest one is used.
    let n = g.nodes.values().find(|n| {
        (n.name == node || n.key() == node) && g.ports_of(n.id, dir).iter().any(|p| p.name == port)
    })?;
    g.ports_of(n.id, dir).into_iter().find(|p| p.name == port).map(|p| p.id)
}

/// Live port ids (out, in) of a saved link, if both ends currently exist.
pub fn resolve(g: &Graph, l: &SavedLink) -> Option<(u32, u32)> {
    Some((
        port_id(g, &l.out_node, &l.out_port, Direction::Out)?,
        port_id(g, &l.in_node, &l.in_port, Direction::In)?,
    ))
}

/// Enabled saved links whose ends exist but which are not connected yet and
/// were not already requested.
pub fn missing(g: &Graph, links: &[SavedLink], requested: &HashSet<(u32, u32)>) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for l in links.iter().filter(|l| l.enabled) {
        let Some(pair) = resolve(g, l) else { continue };
        if g.find_link(pair.0, pair.1).is_some()
            || g.owned_pairs.contains(&pair)
            || requested.contains(&pair)
            || out.contains(&pair)
        {
            continue;
        }
        out.push(pair);
    }
    out
}

/// Returns external links that should be severed to prevent audio bleed according to `mode`.
///
/// - `ManagedNodes`: severs external links if:
///   - `out_node` has saved outgoing links in the active setup, OR
///   - `in_node` is a capture stream (`is_stream()`) that has saved incoming links in the active setup,
///   and this link is not one of the saved links. (Shared destination sinks like Headphones are not
///   treated as exclusive inputs, so unmanaged apps can still play to them.)
/// - `AllNodes`: severs external links if either end is an application audio stream (`is_stream`),
///   and this link is not one of the saved links.
pub fn bleed_links_to_sever(
    g: &Graph,
    saved_links: &[SavedLink],
    mode: crate::storage::AutoConnectPrevention,
    severed_already: &HashSet<u32>,
) -> Vec<(u32, u32, u32)> {
    if mode == crate::storage::AutoConnectPrevention::Off {
        return Vec::new();
    }
    let managed_out: HashSet<&str> = saved_links
        .iter()
        .filter(|l| l.enabled)
        .map(|l| l.out_node.as_str())
        .collect();
    let managed_in: HashSet<&str> = saved_links
        .iter()
        .filter(|l| l.enabled)
        .map(|l| l.in_node.as_str())
        .collect();

    let expected_pairs: HashSet<(u32, u32)> = saved_links
        .iter()
        .filter(|l| l.enabled)
        .filter_map(|l| resolve(g, l))
        .collect();

    let mut out = Vec::new();
    for link in g.links.values() {
        if g.is_owned(link) {
            continue;
        }
        let pair = (link.out_port, link.in_port);
        if expected_pairs.contains(&pair) {
            continue;
        }
        let out_node = g.nodes.get(&link.out_node);
        let in_node = g.nodes.get(&link.in_node);

        let should_sever = match mode {
            crate::storage::AutoConnectPrevention::Off => false,
            crate::storage::AutoConnectPrevention::ManagedNodes => {
                let out_managed = out_node.map_or(false, |n| {
                    managed_out.contains(n.name.as_str()) || managed_out.contains(n.key().as_str())
                });
                let in_managed = in_node.map_or(false, |n| {
                    n.is_stream()
                        && (managed_in.contains(n.name.as_str())
                            || managed_in.contains(n.key().as_str()))
                });
                out_managed || in_managed
            }
            crate::storage::AutoConnectPrevention::AllNodes => {
                let out_is_stream = out_node.map_or(false, |n| n.is_stream());
                let in_is_stream = in_node.map_or(false, |n| n.is_stream());
                out_is_stream || in_is_stream
            }
        };

        if should_sever && !severed_already.contains(&link.id) {
            out.push((link.id, link.out_port, link.in_port));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Event, LinkInfo, NodeInfo, PortInfo};

    fn node(id: u32, name: &str) -> Event {
        Event::NodeAdded(NodeInfo {
            id,
            name: name.into(),
            description: String::new(),
            media_class: String::new(),
            device_id: None,
            profile_device: None,
            is_owned: false,
        })
    }
    fn port(id: u32, node_id: u32, name: &str, direction: Direction) -> Event {
        Event::PortAdded(PortInfo { id, node_id, name: name.into(), direction })
    }
    fn graph() -> Graph {
        let mut g = Graph::default();
        g.apply(node(1, "mic"));
        g.apply(port(10, 1, "capture_FL", Direction::Out));
        g.apply(node(2, "app"));
        g.apply(port(20, 2, "input_FL", Direction::In));
        g
    }
    fn saved() -> SavedLink {
        SavedLink {
            out_node: "mic".into(),
            out_port: "capture_FL".into(),
            in_node: "app".into(),
            in_port: "input_FL".into(),
            enabled: true,
        }
    }

    #[test]
    fn names_from_ids_and_back() {
        let g = graph();
        assert_eq!(from_ports(&g, 10, 20), Some(saved()));
        assert_eq!(resolve(&g, &saved()), Some((10, 20)));
        assert_eq!(from_ports(&g, 10, 99), None);
    }

    #[test]
    fn missing_node_is_skipped_then_connected_when_it_appears() {
        let mut g = Graph::default();
        g.apply(node(1, "mic"));
        g.apply(port(10, 1, "capture_FL", Direction::Out));
        let links = vec![saved()];
        let req = HashSet::new();
        assert!(missing(&g, &links, &req).is_empty());
        g.apply(node(2, "app"));
        g.apply(port(20, 2, "input_FL", Direction::In));
        assert_eq!(missing(&g, &links, &req), vec![(10, 20)]);
    }

    #[test]
    fn no_duplicates_when_connected_pending_requested_or_disabled() {
        let mut g = graph();
        let links = vec![saved()];
        let mut req = HashSet::new();
        req.insert((10, 20));
        assert!(missing(&g, &links, &req).is_empty());
        let req = HashSet::new();
        g.apply(Event::OwnedAdded { out_port: 10, in_port: 20 });
        assert!(missing(&g, &links, &req).is_empty());
        g.apply(Event::OwnedRemoved { out_port: 10, in_port: 20 });
        let mut off = links.clone();
        off[0].enabled = false;
        assert!(missing(&g, &off, &req).is_empty());
        g.apply(Event::LinkAdded(LinkInfo { id: 5, out_node: 1, out_port: 10, in_node: 2, in_port: 20 }));
        assert!(missing(&g, &links, &req).is_empty()); // already linked (e.g. externally)
    }

    #[test]
    fn add_remove_dedupe_and_reenable() {
        let mut v = Vec::new();
        assert!(add(&mut v, saved()));
        assert!(!add(&mut v, saved()));
        v[0].enabled = false;
        assert!(add(&mut v, saved()));
        assert!(v[0].enabled);
        assert!(remove(&mut v, &saved()));
        assert!(!remove(&mut v, &saved()));
    }

    #[test]
    fn replugged_node_with_new_ids_resolves_again() {
        let mut g = graph();
        g.apply(Event::Removed(20));
        g.apply(Event::Removed(2));
        assert_eq!(resolve(&g, &saved()), None);
        g.apply(node(7, "app"));
        g.apply(port(70, 7, "input_FL", Direction::In));
        assert_eq!(resolve(&g, &saved()), Some((10, 70)));
    }

    #[test]
    fn bleed_prevention_modes() {
        use crate::storage::AutoConnectPrevention;

        let mut g = Graph::default();
        // Waterfox playback stream
        g.apply(Event::NodeAdded(NodeInfo {
            id: 1,
            name: "waterfox".into(),
            description: "Waterfox".into(),
            media_class: "Stream/Output/Audio".into(),
            device_id: None,
            profile_device: None,
            is_owned: false,
        }));
        g.apply(Event::PortAdded(PortInfo { id: 10, node_id: 1, name: "output_FL".into(), direction: Direction::Out }));

        // Unmanaged game playback stream
        g.apply(Event::NodeAdded(NodeInfo {
            id: 2,
            name: "game".into(),
            description: "Game".into(),
            media_class: "Stream/Output/Audio".into(),
            device_id: None,
            profile_device: None,
            is_owned: false,
        }));
        g.apply(Event::PortAdded(PortInfo { id: 20, node_id: 2, name: "output_FL".into(), direction: Direction::Out }));

        // Built-in speakers (hardware sink)
        g.apply(Event::NodeAdded(NodeInfo {
            id: 3,
            name: "alsa_output".into(),
            description: "Speakers".into(),
            media_class: "Audio/Sink".into(),
            device_id: None,
            profile_device: None,
            is_owned: false,
        }));
        g.apply(Event::PortAdded(PortInfo { id: 30, node_id: 3, name: "playback_FL".into(), direction: Direction::In }));

        // Virtual Music Sink
        g.apply(Event::NodeAdded(NodeInfo {
            id: 4,
            name: "Music_Sink".into(),
            description: "Music Sink".into(),
            media_class: "Audio/Sink".into(),
            device_id: None,
            profile_device: None,
            is_owned: true,
        }));
        g.apply(Event::PortAdded(PortInfo { id: 40, node_id: 4, name: "playback_FL".into(), direction: Direction::In }));

        // WirePlumber auto-connections (external links to Speakers)
        g.apply(Event::LinkAdded(LinkInfo { id: 101, out_node: 1, out_port: 10, in_node: 3, in_port: 30 }));
        g.apply(Event::LinkAdded(LinkInfo { id: 102, out_node: 2, out_port: 20, in_node: 3, in_port: 30 }));

        // User's saved links: Waterfox -> Music_Sink, and Music_Sink -> Speakers
        g.apply(Event::PortAdded(PortInfo { id: 41, node_id: 4, name: "monitor_FL".into(), direction: Direction::Out }));
        let saved_links = vec![
            SavedLink {
                out_node: "waterfox".into(),
                out_port: "output_FL".into(),
                in_node: "Music_Sink".into(),
                in_port: "playback_FL".into(),
                enabled: true,
            },
            SavedLink {
                out_node: "Music_Sink".into(),
                out_port: "monitor_FL".into(),
                in_node: "alsa_output".into(),
                in_port: "playback_FL".into(),
                enabled: true,
            },
        ];

        let empty_severed = HashSet::new();

        // 1. Off mode: nothing severed
        let to_sever = bleed_links_to_sever(&g, &saved_links, AutoConnectPrevention::Off, &empty_severed);
        assert!(to_sever.is_empty());

        // 2. ManagedNodes: only Waterfox's link to Speakers is severed, Game's link is kept!
        let to_sever = bleed_links_to_sever(&g, &saved_links, AutoConnectPrevention::ManagedNodes, &empty_severed);
        assert_eq!(to_sever, vec![(101, 10, 30)]);

        // 3. AllNodes: both Waterfox and Game are severed from Speakers!
        let mut to_sever = bleed_links_to_sever(&g, &saved_links, AutoConnectPrevention::AllNodes, &empty_severed);
        to_sever.sort_by_key(|(id, _, _)| *id);
        assert_eq!(to_sever, vec![(101, 10, 30), (102, 20, 30)]);

        // 4. Expected link: if an external link matches the saved link, it is not severed
        g.apply(Event::LinkAdded(LinkInfo { id: 103, out_node: 1, out_port: 10, in_node: 4, in_port: 40 }));
        let to_sever = bleed_links_to_sever(&g, &saved_links, AutoConnectPrevention::ManagedNodes, &empty_severed);
        assert_eq!(to_sever, vec![(101, 10, 30)]); // 103 is preserved

        // 5. Already severed: does not return again
        let mut severed = HashSet::new();
        severed.insert(101);
        let to_sever = bleed_links_to_sever(&g, &saved_links, AutoConnectPrevention::ManagedNodes, &severed);
        assert!(to_sever.is_empty());
    }
}
