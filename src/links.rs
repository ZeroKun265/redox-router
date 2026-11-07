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
    /// Points (world coordinates) the cable is routed through.
    #[serde(default)]
    pub waypoints: Vec<(f32, f32)>,
    /// Keep this PipeWire link alive even after RedoxRouter quits (`object.linger = true`).
    #[serde(default)]
    pub lingering: bool,
    /// Render an inline on/off switch box on this cable so it can be toggled without forgetting it.
    #[serde(default)]
    pub has_switch: bool,
    /// Optional world position `(x, y)` of the switch box; when `None`, sits at the cable midpoint.
    #[serde(default)]
    pub switch_pos: Option<(f32, f32)>,
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
        waypoints: Vec::new(),
        lingering: false,
        has_switch: false,
        switch_pos: None,
    })
}

/// Index of the saved link with these ends.
pub fn find(links: &[SavedLink], l: &SavedLink) -> Option<usize> {
    links.iter().position(|x| x.same_ends(l))
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
            || !g.can_link_ports(pair.0, pair.1)
        {
            continue;
        }
        out.push(pair);
    }
    out
}

/// Whether the saved link matching `(out_port, in_port)` is configured to linger after quit.
pub fn is_lingering(g: &Graph, links: &[SavedLink], out_port: u32, in_port: u32) -> bool {
    from_ports(g, out_port, in_port)
        .and_then(|s| find(links, &s))
        .is_some_and(|i| links[i].lingering)
}

/// Swap common stereo suffixes/tokens on a port name (`FL`<->`FR`, `_L`<->`_R`, `_1`<->`_2`, `left`<->`right`).
fn swap_stereo_name(name: &str) -> Option<String> {
    for (a, b) in [
        ("_FL", "_FR"),
        ("_FR", "_FL"),
        ("FL", "FR"),
        ("FR", "FL"),
        ("_L", "_R"),
        ("_R", "_L"),
        ("_1", "_2"),
        ("_2", "_1"),
        ("_left", "_right"),
        ("_right", "_left"),
        ("Left", "Right"),
        ("Right", "Left"),
    ] {
        if let Some(prefix) = name.strip_suffix(a) {
            return Some(format!("{prefix}{b}"));
        }
    }
    None
}

/// Find the companion stereo port on `node_id` in direction `dir` for `port_id`.
fn sibling_stereo_port(g: &Graph, node_id: u32, port_id: u32, dir: Direction) -> Option<u32> {
    let port = g.ports.get(&port_id)?;
    let ports = g.ports_of(node_id, dir);
    if let Some(target_name) = swap_stereo_name(&port.name) {
        if let Some(p) = ports.iter().find(|p| p.id != port_id && p.name == target_name) {
            return Some(p.id);
        }
    }
    // Fallback: if the node has exactly 2 ports in this direction, pick the other one.
    if ports.len() == 2 {
        return ports.iter().find(|p| p.id != port_id).map(|p| p.id);
    }
    None
}

/// Given a connection `(out_port, in_port)`, find the matching stereo partner pair
/// `(partner_out_port, partner_in_port)` on the same two nodes, if both exist.
pub fn stereo_partner_ports(g: &Graph, out_port: u32, in_port: u32) -> Option<(u32, u32)> {
    let po = g.ports.get(&out_port)?;
    let pi = g.ports.get(&in_port)?;
    let out2 = sibling_stereo_port(g, po.node_id, out_port, Direction::Out)?;
    let in2 = sibling_stereo_port(g, pi.node_id, in_port, Direction::In)?;
    Some((out2, in2))
}

/// Pair up output ports of `out_node` with input ports of `in_node` for 1-click stereo/multi-channel routing.
pub fn stereo_node_pairs(g: &Graph, out_node: u32, in_node: u32) -> Vec<(u32, u32)> {
    if out_node == in_node {
        return Vec::new();
    }
    let (Some(no), Some(ni)) = (g.nodes.get(&out_node), g.nodes.get(&in_node)) else {
        return Vec::new();
    };
    if !no.can_link_to(ni) {
        return Vec::new();
    }
    let outs = g.ports_of(out_node, Direction::Out);
    let ins = g.ports_of(in_node, Direction::In);
    if outs.is_empty() || ins.is_empty() {
        return Vec::new();
    }
    // First try matching FL/FR by channel suffix; otherwise pair in order up to 2 (or min len).
    let mut pairs = Vec::new();
    let mut used_in = HashSet::new();
    for o in &outs {
        let ch = if o.name.ends_with("FL") || o.name.ends_with("_L") || o.name.ends_with("_1") {
            Some(0)
        } else if o.name.ends_with("FR") || o.name.ends_with("_R") || o.name.ends_with("_2") {
            Some(1)
        } else {
            None
        };
        if let Some(ch) = ch {
            if let Some(i) = ins.iter().find(|i| {
                !used_in.contains(&i.id)
                    && ((ch == 0
                        && (i.name.ends_with("FL")
                            || i.name.ends_with("_L")
                            || i.name.ends_with("_1")))
                        || (ch == 1
                            && (i.name.ends_with("FR")
                                || i.name.ends_with("_R")
                                || i.name.ends_with("_2"))))
            }) {
                used_in.insert(i.id);
                pairs.push((o.id, i.id));
            }
        }
    }
    if pairs.is_empty() {
        for (o, i) in outs.iter().zip(ins.iter()) {
            pairs.push((o.id, i.id));
        }
    }
    pairs
}

/// Bleed decision taken once when a node appears (kept for 5 minutes across brief
/// disappearances, see [`bleed_decision`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BleedInfo {
    /// The node had saved links in the active setup when it (first) appeared.
    pub managed_at_launch: bool,
    /// The node appeared after RedoxRouter's startup (a "new app").
    pub new_app: bool,
}

/// Decide the bleed state of a node that just appeared.
///
/// - `previous`: the decision of the last node with the same key and how long ago
///   it disappeared. If it came back within `keep_for`, its `new_app` status (whether
///   it was already running at RedoxRouter startup vs launched later) is preserved.
/// - `in_saved_links`: whether the active setup currently references the node.
/// - `during_startup`: the node was already running when RedoxRouter started.
pub fn bleed_decision(
    previous: Option<(BleedInfo, std::time::Duration)>,
    keep_for: std::time::Duration,
    in_saved_links: bool,
    during_startup: bool,
) -> BleedInfo {
    let new_app = match previous {
        Some((info, gone_for)) if gone_for < keep_for => info.new_app,
        _ => !during_startup,
    };
    BleedInfo {
        managed_at_launch: in_saved_links,
        new_app,
    }
}

/// Whether the active setup references a node (by name or composite key).
pub fn node_in_saved_links(saved_links: &[SavedLink], name: &str, key: &str) -> bool {
    saved_links.iter().any(|l| {
        (l.enabled || l.has_switch)
            && (l.out_node == name || l.out_node == key || l.in_node == name || l.in_node == key)
    })
}

/// Which of the `candidates` (ids of external links that were just created by
/// someone else, e.g. WirePlumber's auto-connect right after an app appeared)
/// should be severed to prevent audio bleed. Links that already existed are never
/// passed in, so switching modes or adding a Redox link never removes them.
///
/// - `ManagedNodes`: sever if the app end (`out_node`, or `in_node` when it is a
///   capture stream) was managed when it appeared AND is still in the saved links.
/// - `AllNodes`: the above, plus any link whose app stream end is a new app
///   (appeared after RedoxRouter started).
///
/// Links matching a saved link are kept. Returns `(link_id, out_port, in_port)`.
pub fn bleed_links_to_sever(
    g: &Graph,
    saved_links: &[SavedLink],
    mode: crate::storage::AutoConnectPrevention,
    candidates: &[u32],
    info: &std::collections::HashMap<u32, BleedInfo>,
) -> Vec<(u32, u32, u32)> {
    use crate::storage::AutoConnectPrevention as M;
    if mode == M::Off {
        return Vec::new();
    }
    let expected_pairs: HashSet<(u32, u32)> = saved_links
        .iter()
        .filter(|l| l.enabled)
        .filter_map(|l| resolve(g, l))
        .collect();

    let mut out = Vec::new();
    for id in candidates {
        let Some(link) = g.links.get(id) else { continue };
        if g.is_owned(link) || expected_pairs.contains(&(link.out_port, link.in_port)) {
            continue;
        }
        // (node, counts as the app end of this link)
        let ends = [
            (g.nodes.get(&link.out_node), true),
            (g.nodes.get(&link.in_node), false),
        ];
        let mut sever = false;
        for (node, is_out) in ends {
            let Some(n) = node else { continue };
            let i = info.get(&n.id).copied().unwrap_or_default();
            let app_end = is_out || n.is_stream();
            let managed = app_end
                && i.managed_at_launch
                && node_in_saved_links(saved_links, &n.name, &n.key());
            let new_app = n.is_stream() && i.new_app;
            sever |= match mode {
                M::Off => false,
                M::ManagedNodes => managed,
                M::AllNodes => managed || new_app,
            };
        }
        if sever {
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
            waypoints: Vec::new(),
            lingering: false,
            has_switch: false,
            switch_pos: None,
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
                waypoints: Vec::new(),
                lingering: false,
                has_switch: false,
                switch_pos: None,
            },
            SavedLink {
                out_node: "Music_Sink".into(),
                out_port: "monitor_FL".into(),
                in_node: "alsa_output".into(),
                in_port: "playback_FL".into(),
                enabled: true,
                waypoints: Vec::new(),
                lingering: false,
                has_switch: false,
                switch_pos: None,
            },
        ];

        use std::collections::HashMap;
        let all = [101u32, 102];
        // Both apps were running at startup; only waterfox was managed then.
        let mut info: HashMap<u32, BleedInfo> = HashMap::new();
        info.insert(1, BleedInfo { managed_at_launch: true, new_app: false });
        info.insert(2, BleedInfo { managed_at_launch: false, new_app: false });

        // 1. Off mode: nothing severed
        assert!(bleed_links_to_sever(&g, &saved_links, AutoConnectPrevention::Off, &all, &info).is_empty());

        // 2. ManagedNodes: only Waterfox's link to Speakers is severed, Game's link is kept
        let to_sever = bleed_links_to_sever(&g, &saved_links, AutoConnectPrevention::ManagedNodes, &all, &info);
        assert_eq!(to_sever, vec![(101, 10, 30)]);

        // 3. AllNodes: Game was already running at startup, so it is NOT a new app -> kept
        let to_sever = bleed_links_to_sever(&g, &saved_links, AutoConnectPrevention::AllNodes, &all, &info);
        assert_eq!(to_sever, vec![(101, 10, 30)]);

        // 4. AllNodes: Game launched after startup -> its auto-link is severed too
        info.insert(2, BleedInfo { managed_at_launch: false, new_app: true });
        let mut to_sever = bleed_links_to_sever(&g, &saved_links, AutoConnectPrevention::AllNodes, &all, &info);
        to_sever.sort_by_key(|(id, _, _)| *id);
        assert_eq!(to_sever, vec![(101, 10, 30), (102, 20, 30)]);
        info.insert(2, BleedInfo { managed_at_launch: false, new_app: false });

        // 5. Links that are not candidates (already existed) are never touched,
        //    e.g. when the user switches modes later
        assert!(bleed_links_to_sever(&g, &saved_links, AutoConnectPrevention::AllNodes, &[], &info).is_empty());

        // 6. An external link matching a saved link is kept
        g.apply(Event::LinkAdded(LinkInfo { id: 103, out_node: 1, out_port: 10, in_node: 4, in_port: 40 }));
        assert!(bleed_links_to_sever(&g, &saved_links, AutoConnectPrevention::ManagedNodes, &[103], &info).is_empty());

        // 7. Game (unmanaged at launch) gets a Redox link later: its default link is kept
        let mut updated_saved = saved_links.clone();
        updated_saved.push(SavedLink {
            out_node: "game".into(),
            out_port: "output_FL".into(),
            in_node: "Music_Sink".into(),
            in_port: "playback_FL".into(),
            enabled: true,
            waypoints: Vec::new(),
            lingering: false,
            has_switch: false,
            switch_pos: None,
        });
        let to_sever = bleed_links_to_sever(&g, &updated_saved, AutoConnectPrevention::ManagedNodes, &all, &info);
        assert_eq!(to_sever, vec![(101, 10, 30)]);

        // 8. Waterfox was managed at launch but all its saved links were removed -> not managed
        let only_sink = vec![saved_links[1].clone()];
        assert!(bleed_links_to_sever(&g, &only_sink, AutoConnectPrevention::ManagedNodes, &all, &info).is_empty());
    }

    #[test]
    fn bleed_decision_keeps_state_for_five_minutes() {
        use std::time::Duration;
        let keep = Duration::from_secs(300);
        // Fresh node after startup, in saved links
        let d = bleed_decision(None, keep, true, false);
        assert_eq!(d, BleedInfo { managed_at_launch: true, new_app: true });
        // Already running at startup, not in saved links
        let d = bleed_decision(None, keep, false, true);
        assert_eq!(d, BleedInfo { managed_at_launch: false, new_app: false });
        // Came back after 1 minute after user added a saved link: managed_at_launch is now true,
        // while its pre-existing startup status (new_app = false) is preserved for 5 minutes.
        let prev = BleedInfo { managed_at_launch: false, new_app: false };
        assert_eq!(
            bleed_decision(Some((prev, Duration::from_secs(60))), keep, true, false),
            BleedInfo { managed_at_launch: true, new_app: false }
        );
        // Came back after 6 minutes: decided again from scratch (new_app becomes true)
        let d = bleed_decision(Some((prev, Duration::from_secs(360))), keep, true, false);
        assert_eq!(d, BleedInfo { managed_at_launch: true, new_app: true });
    }

    #[test]
    fn stereo_partner_and_node_pairs() {
        let mut g = graph();
        g.apply(port(11, 1, "capture_FR", Direction::Out));
        g.apply(port(21, 2, "input_FR", Direction::In));
        assert_eq!(stereo_partner_ports(&g, 10, 20), Some((11, 21)));
        assert_eq!(stereo_partner_ports(&g, 11, 21), Some((10, 20)));
        assert_eq!(stereo_node_pairs(&g, 1, 2), vec![(10, 20), (11, 21)]);
    }
}
