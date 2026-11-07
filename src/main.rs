//! RedoxRouter by ZeroKun265 - a node-based PipeWire patchbay.
//! Current state: node canvas; you create/delete your own ("owned") links,
//! other programs' links are hidden by default. See ROADMAP.md.

mod canvas;
mod cli;
mod graph;
mod layout;
mod markdown;
mod links;
mod pw_backend;
mod rules;
mod setups;
mod sysinfo;
mod volume;
mod storage;
mod theme;

use canvas::{Action, Canvas};
use eframe::egui;
use graph::{Command, Graph, NodeInfo};
use pw_backend::CommandSender;
use rules::{Mode, Action as RuleAction, Field, GroupEntry, NodeText, Rule, RuleSet};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use theme::{GridPattern, SwitchGradMode, Theme};

struct RedoxRouter {
    rx: Receiver<graph::Event>,
    cli_rx: Receiver<cli::IncomingCli>,
    cmd: CommandSender,
    graph: Graph,
    canvas: Canvas,
    show_debug: bool,
    show_sysinfo: bool,
    sysinfo_save_path: String,
    sysinfo_status: Option<Result<String, String>>,
    show_rules: bool,
    show_links: bool,
    show_theme: bool,
    theme: Theme,
    theme_export_path: String,
    theme_import_path: String,
    theme_raw_json: String,
    theme_status: Option<Result<String, String>>,
    setups: setups::SetupList<storage::LayoutFile>,
    /// Index of the setup shown on the canvas (the active one is live).
    viewing: usize,
    renaming: Option<(usize, String)>,
    /// Shown when an edit was refused because the viewed setup is not active.
    inactive_notice: bool,
    confirm_reset: bool,
    /// Port pairs we already asked to link (so auto-connect asks only once).
    requested: HashSet<(u32, u32)>,
    /// Link IDs we have already asked to destroy for bleed prevention.
    severed_links: HashSet<u32>,
    /// Ports the user tried to connect although another program already links them.
    prompt: Option<(u32, u32)>,
    show_create_sink: Option<storage::VirtualDeviceKind>,
    new_sink_name: String,
    new_sink_desc: String,
    /// Exporting a setup: (setup index, destination file path, status/error message).
    exporting_setup: Option<(usize, String, Option<Result<String, String>>)>,
    /// Importing a setup: (file path input, raw JSON input, error message).
    importing_setup: Option<(String, String, Option<String>)>,
    /// Node search query (`Ctrl+F`).
    search_query: String,
    search_focus: bool,
    last_save: Instant,
    save_error: Option<String>,
    tools_flyout: Option<ToolsSubmenu>,
    /// When RedoxRouter started (nodes seen in the first seconds were already running).
    started: Instant,
    /// Bleed decision per node key, kept for 5 minutes after the node disappears.
    node_sessions: HashMap<String, NodeSession>,
    /// Live nodes: (when they appeared, bleed decision).
    node_bleed: HashMap<u32, (Instant, links::BleedInfo)>,
    /// External links created recently that bleed prevention still has to look at once.
    bleed_pending: Vec<(u32, Instant)>,
    /// Ring buffer of recent PipeWire events and commands for issue diagnostics.
    event_log: RefCell<VecDeque<String>>,
}

#[derive(Clone, Copy, Debug)]
struct NodeSession {
    info: links::BleedInfo,
    /// When the node disappeared from PipeWire, or None if currently present.
    disconnected_at: Option<Instant>,
}

/// Nodes appearing this soon after RedoxRouter starts were already running.
const STARTUP_WINDOW: Duration = Duration::from_secs(3);
/// External links created this soon after their app appeared count as auto-connections.
const AUTO_LINK_WINDOW: Duration = Duration::from_secs(5);
/// A node that comes back within this time keeps its first-appearance bleed decision.
const SESSION_KEEP: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, PartialEq, Eq)]
enum ToolsSubmenu {
    DestroySink,
    DestroyMic,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TabOp {
    View(usize),
    Add,
    Duplicate(usize),
    Rename(usize),
    Export(usize),
    Import,
    Delete(usize),
    MakeActive,
}

impl RedoxRouter {
    fn new(rx: Receiver<graph::Event>, cli_rx: Receiver<cli::IncomingCli>, cmd: CommandSender) -> Self {
        let setups = storage::load_setups();
        let viewing = setups.active;
        let theme = storage::load_theme();
        let mut canvas = Canvas::with_saved(setups.items[viewing].data.clone());
        canvas.theme = theme.clone();
        canvas.rules.library = storage::load_library();
        for sink in &canvas.virtual_devices {
            let msg = match sink.kind {
                storage::VirtualDeviceKind::Sink => Command::CreateSink {
                    name: sink.name.clone(),
                    description: sink.description.clone(),
                },
                storage::VirtualDeviceKind::Source => Command::CreateSource {
                    name: sink.name.clone(),
                    description: sink.description.clone(),
                },
            };
            let _ = cmd.send(msg);
        }
        Self {
            rx,
            cli_rx,
            cmd,
            graph: Graph::default(),
            canvas,
            setups,
            viewing,
            renaming: None,
            inactive_notice: false,
            confirm_reset: false,
            show_debug: false,
            show_sysinfo: false,
            sysinfo_save_path: storage::config_dir_path()
                .map(|d| d.join("system-report.txt").display().to_string())
                .unwrap_or_else(|| "~/redox-router-report.txt".to_string()),
            sysinfo_status: None,
            show_rules: false,
            show_links: false,
            show_theme: false,
            theme_export_path: storage::themes_dir()
                .map(|d| d.join(format!("{}.json", theme.name.to_lowercase().replace(' ', "_"))).display().to_string())
                .unwrap_or_else(|| "~/.config/RedoxRouter/themes/theme.json".to_string()),
            theme_import_path: String::new(),
            theme_raw_json: String::new(),
            theme_status: None,
            theme,
            show_create_sink: None,
            new_sink_name: String::new(),
            new_sink_desc: String::new(),
            exporting_setup: None,
            importing_setup: None,
            search_query: String::new(),
            search_focus: false,
            requested: HashSet::new(),
            severed_links: HashSet::new(),
            prompt: None,
            last_save: Instant::now(),
            save_error: None,
            tools_flyout: None,
            node_sessions: HashMap::new(),
            started: Instant::now(),
            node_bleed: HashMap::new(),
            bleed_pending: Vec::new(),
            event_log: RefCell::new(VecDeque::new()),
        }
    }

    /// Save the layout and rule library at most once per second after a change.
    fn maybe_save(&mut self, ctx: &egui::Context) {
        let layout_dirty = self.canvas.dirty;
        let library_dirty = self.canvas.rules.library_dirty;
        if !layout_dirty && !library_dirty {
            return;
        }
        if self.last_save.elapsed() >= Duration::from_secs(1) {
            let mut err = None;
            if layout_dirty {
                self.sync_current();
                err = storage::save_setups(&self.setups).err();
            }
            if library_dirty {
                err = err.or(storage::save_library(&self.canvas.rules.library).err());
            }
            self.save_error = err;
            self.canvas.dirty = false;
            self.canvas.rules.library_dirty = false;
            self.last_save = Instant::now();
        } else {
            ctx.request_repaint_after(Duration::from_millis(1100));
        }
    }

    fn drain_events(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            self.log_pw_event(&ev);
            match &ev {
                graph::Event::NodeAdded(ref n) => {
                    let key = n.key();
                    let now = Instant::now();
                    let previous = self.node_sessions.get(&key).and_then(|s| {
                        s.disconnected_at.map(|t| (s.info, t.elapsed()))
                    });
                    let info = links::bleed_decision(
                        previous,
                        SESSION_KEEP,
                        self.is_node_in_saved_links(n),
                        self.started.elapsed() < STARTUP_WINDOW,
                    );
                    self.node_sessions.insert(key, NodeSession { info, disconnected_at: None });
                    self.node_bleed.insert(n.id, (now, info));
                }
                graph::Event::PortAdded(ref p) => {
                    // If a node had no live links yet when its ports appeared (e.g. a browser tab
                    // creating audio ports only when playback starts), refresh its `appeared` time
                    // so the 5-second auto-link bleed window starts from port creation.
                    let has_links = self
                        .graph
                        .links
                        .values()
                        .any(|l| l.out_node == p.node_id || l.in_node == p.node_id);
                    if !has_links {
                        let managed = self
                            .graph
                            .nodes
                            .get(&p.node_id)
                            .is_some_and(|n| self.is_node_in_saved_links(n));
                        if let Some((appeared, info)) = self.node_bleed.get_mut(&p.node_id) {
                            *appeared = Instant::now();
                            info.managed_at_launch = managed;
                        }
                    }
                }
                graph::Event::LinkAdded(ref link) => {
                    let pair = (link.out_port, link.in_port);
                    if self.requested.contains(&pair) || self.graph.owned_pairs.contains(&pair) {
                        self.graph.owned_pairs.insert(pair);
                        self.remember_live_link(link.out_port, link.in_port);
                        self.requested.remove(&pair);
                    } else {
                        self.bleed_pending.push((link.id, Instant::now()));
                    }
                }
                graph::Event::OwnedAdded { out_port, in_port } => {
                    let pair = (*out_port, *in_port);
                    if self.graph.find_link(*out_port, *in_port).is_some() {
                        self.remember_live_link(*out_port, *in_port);
                    }
                    self.requested.remove(&pair);
                }
                graph::Event::Error(ref err) => {
                    if err.contains("Format negotiation") || err.contains("pipewire error") {
                        self.requested.clear();
                    }
                }
                graph::Event::Removed(id) => {
                    self.severed_links.remove(id);
                    if let Some(n) = self.graph.nodes.get(id) {
                        let key = n.key();
                        if let Some(session) = self.node_sessions.get_mut(&key) {
                            session.disconnected_at = Some(Instant::now());
                        }
                    }
                    self.node_bleed.remove(id);
                    self.bleed_pending.retain(|(l, _)| l != id);
                    if self.graph.ports.contains_key(id) {
                        self.requested.retain(|&(o, i)| o != *id && i != *id);
                    }
                    if self.graph.owned_ids.contains(id) {
                        if let Some(l) = self.graph.links.get(id) {
                            self.requested.remove(&(l.out_port, l.in_port));
                        }
                    }
                }
                graph::Event::OwnedRemoved { out_port, in_port } => {
                    self.requested.remove(&(*out_port, *in_port));
                }
                _ => {}
            }
            self.graph.apply(ev);
        }
    }

    fn destroy_sink(&mut self, node_id: u32) {
        if let Some(n) = self.graph.nodes.get(&node_id) {
            self.canvas.virtual_devices.retain(|s| s.name != n.name);
            if self.viewing != self.setups.active {
                self.setups.items[self.setups.active]
                    .data
                    .virtual_devices
                    .retain(|s| s.name != n.name);
            }
            self.canvas.dirty = true;
            let active_devs = if self.viewing == self.setups.active {
                &self.canvas.virtual_devices
            } else {
                &self.setups.items[self.setups.active].data.virtual_devices
            };
            let _ = storage::save_sinks(active_devs);
        }
        self.send(Command::DestroySink { node_id });
    }

    /// Reconcile live Redox-owned virtual sinks/mics in PipeWire with the active setup's `virtual_devices`.
    fn reconcile_virtual_devices(&mut self) {
        let desired = if self.viewing == self.setups.active {
            self.canvas.virtual_devices.clone()
        } else {
            self.setups.items[self.setups.active].data.virtual_devices.clone()
        };
        let to_destroy: Vec<u32> = self
            .graph
            .nodes
            .values()
            .filter(|n| self.graph.is_node_owned(n))
            .filter(|n| !desired.iter().any(|d| d.name == n.name))
            .map(|n| n.id)
            .collect();
        for node_id in to_destroy {
            self.send(Command::DestroySink { node_id });
        }
        for dev in &desired {
            let already_live = self
                .graph
                .nodes
                .values()
                .any(|n| self.graph.is_node_owned(n) && n.name == dev.name);
            if !already_live {
                let cmd = match dev.kind {
                    storage::VirtualDeviceKind::Sink => Command::CreateSink {
                        name: dev.name.clone(),
                        description: dev.description.clone(),
                    },
                    storage::VirtualDeviceKind::Source => Command::CreateSource {
                        name: dev.name.clone(),
                        description: dev.description.clone(),
                    },
                };
                self.send(cmd);
            }
        }
        let _ = storage::save_sinks(&desired);
    }

    fn create_virtual_device(
        &mut self,
        name: String,
        description: String,
        kind: storage::VirtualDeviceKind,
        on_active: bool,
    ) {
        let dev = storage::SavedSink {
            name: name.clone(),
            description: description.clone(),
            kind,
        };
        let target_list = if on_active && self.viewing != self.setups.active {
            &mut self.setups.items[self.setups.active].data.virtual_devices
        } else {
            &mut self.canvas.virtual_devices
        };
        if let Some(existing) = target_list.iter_mut().find(|s| s.name == dev.name) {
            existing.description = dev.description;
            existing.kind = kind;
        } else {
            target_list.push(dev);
        }
        self.canvas.dirty = true;
        if on_active || self.viewing == self.setups.active {
            let already_live = self
                .graph
                .nodes
                .values()
                .any(|n| self.graph.is_node_owned(n) && n.name == name);
            if !already_live {
                let cmd = match kind {
                    storage::VirtualDeviceKind::Sink => Command::CreateSink { name, description },
                    storage::VirtualDeviceKind::Source => Command::CreateSource { name, description },
                };
                self.send(cmd);
            }
        }
    }

    fn drain_cli(&mut self) {
        while let Ok(inc) = self.cli_rx.try_recv() {
            let resp = match inc.req {
                cli::CliRequest::ListSetups => {
                    let lines: Vec<String> = self
                        .setups
                        .items
                        .iter()
                        .enumerate()
                        .map(|(i, s)| {
                            if i == self.setups.active {
                                format!("* {}", s.name)
                            } else {
                                format!("  {}", s.name)
                            }
                        })
                        .collect();
                    cli::CliResponse {
                        ok: true,
                        message: lines.join("\n"),
                    }
                }
                cli::CliRequest::ActivateSetup(target) => {
                    let idx = self
                        .setups
                        .items
                        .iter()
                        .position(|s| s.name == target)
                        .or_else(|| {
                            self.setups
                                .items
                                .iter()
                                .position(|s| s.name.eq_ignore_ascii_case(&target))
                        });
                    if let Some(i) = idx {
                        self.switch_to(i);
                        self.make_active();
                        cli::CliResponse {
                            ok: true,
                            message: format!("Activated setup \"{}\"", self.setups.items[i].name),
                        }
                    } else {
                        cli::CliResponse {
                            ok: false,
                            message: format!("No setup named \"{target}\""),
                        }
                    }
                }
                cli::CliRequest::NextSetup => {
                    let next = (self.setups.active + 1) % self.setups.items.len();
                    self.switch_to(next);
                    self.make_active();
                    cli::CliResponse {
                        ok: true,
                        message: format!("Activated setup \"{}\"", self.setups.items[next].name),
                    }
                }
                cli::CliRequest::PrevSetup => {
                    let len = self.setups.items.len();
                    let prev = (self.setups.active + len - 1) % len;
                    self.switch_to(prev);
                    self.make_active();
                    cli::CliResponse {
                        ok: true,
                        message: format!("Activated setup \"{}\"", self.setups.items[prev].name),
                    }
                }
                cli::CliRequest::CreateSink { name, description } => {
                    self.create_virtual_device(
                        name.clone(),
                        description,
                        storage::VirtualDeviceKind::Sink,
                        true,
                    );
                    cli::CliResponse {
                        ok: true,
                        message: format!("Created virtual sink \"{name}\""),
                    }
                }
                cli::CliRequest::CreateMic { name, description } => {
                    self.create_virtual_device(
                        name.clone(),
                        description,
                        storage::VirtualDeviceKind::Source,
                        true,
                    );
                    cli::CliResponse {
                        ok: true,
                        message: format!("Created virtual microphone \"{name}\""),
                    }
                }
            };
            let _ = inc.reply.send(resp);
        }
    }

    fn send(&self, cmd: Command) {
        self.log_pw_command(&cmd);
        if self.cmd.send(cmd).is_err() {
            eprintln!("PipeWire thread is not running; command dropped");
        }
    }

    fn handle(&mut self, action: Action) {
        // On an inactive setup, connect/disconnect only edit its saved links.
        if self.viewing != self.setups.active {
            match action {
                Action::Connect { out_port, in_port } => self.remember_link(out_port, in_port),
                Action::Disconnect { out_port, in_port } => {
                    if let Some(s) = links::from_ports(&self.graph, out_port, in_port) {
                        if links::remove(&mut self.canvas.links, &s) {
                            self.canvas.dirty = true;
                        }
                    }
                }
                _ => {} // volume etc. stay live on every tab
            }
            if matches!(action, Action::Connect { .. } | Action::Disconnect { .. }) {
                return;
            }
        }
        match action {
            Action::Connect { out_port, in_port } => {
                if !self.graph.can_link_ports(out_port, in_port) {
                    return;
                }
                if let Some(l) = self.graph.find_link(out_port, in_port) {
                    if !self.graph.is_owned(l) {
                        self.prompt = Some((out_port, in_port));
                    }
                    return; // owned already: nothing to do
                }
                let (Some(po), Some(pi)) =
                    (self.graph.ports.get(&out_port), self.graph.ports.get(&in_port))
                else {
                    return;
                };
                let lingering = links::is_lingering(&self.graph, &self.canvas.links, out_port, in_port);
                let cmd = Command::CreateLink {
                    out_node: po.node_id,
                    out_port,
                    in_node: pi.node_id,
                    in_port,
                    lingering,
                };
                self.requested.insert((out_port, in_port));
                self.send(cmd);
            }
            Action::Disconnect { out_port, in_port } => {
                let live = self.graph.find_link(out_port, in_port);
                let link_id = live.map(|l| l.id);
                if live.is_none_or(|l| self.graph.is_owned(l)) {
                    if let Some(s) = links::from_ports(&self.graph, out_port, in_port) {
                        if links::remove(&mut self.canvas.links, &s) {
                            self.canvas.dirty = true;
                        }
                    }
                }
                self.requested.remove(&(out_port, in_port));
                self.send(Command::DestroyLink { link_id, out_port, in_port });
            }
            Action::TakeOver { out_port, in_port } => {
                self.take_over(out_port, in_port);
            }
            Action::SetLinkLinger { out_port, in_port, lingering } => {
                if self.viewing == self.setups.active {
                    if let (Some(po), Some(pi)) =
                        (self.graph.ports.get(&out_port), self.graph.ports.get(&in_port))
                    {
                        let link_id = self.graph.find_link(out_port, in_port).map(|l| l.id);
                        self.send(Command::DestroyLink { link_id, out_port, in_port });
                        self.requested.insert((out_port, in_port));
                        self.send(Command::CreateLink {
                            out_node: po.node_id,
                            out_port,
                            in_node: pi.node_id,
                            in_port,
                            lingering,
                        });
                    }
                }
            }
            Action::SetLinkEnabled { out_port, in_port, enabled } => {
                if self.viewing == self.setups.active {
                    if enabled {
                        if let (Some(po), Some(pi)) =
                            (self.graph.ports.get(&out_port), self.graph.ports.get(&in_port))
                        {
                            let lingering =
                                links::is_lingering(&self.graph, &self.canvas.links, out_port, in_port);
                            self.requested.insert((out_port, in_port));
                            self.send(Command::CreateLink {
                                out_node: po.node_id,
                                out_port,
                                in_node: pi.node_id,
                                in_port,
                                lingering,
                            });
                        }
                    } else {
                        let link_id = self.graph.find_link(out_port, in_port).map(|l| l.id);
                        self.requested.remove(&(out_port, in_port));
                        self.send(Command::DestroyLink { link_id, out_port, in_port });
                    }
                }
            }
            Action::SetVolume { node_id, linear, channels } => {
                // No local guess: the slider follows what PipeWire reports back.
                self.send(Command::SetVolume { node_id, linear, channels });
            }
            Action::SetMute { node_id, mute } => {
                self.send(Command::SetMute { node_id, mute });
            }
            Action::DestroySink { node_id } => {
                self.destroy_sink(node_id);
            }
        }
    }

    /// Replace another program's link between these ports with one of ours.
    fn take_over(&mut self, out_port: u32, in_port: u32) {
        if self.viewing != self.setups.active {
            self.inactive_notice = true;
            return;
        }
        let Some(l) = self.graph.find_link(out_port, in_port) else { return };
        if self.graph.is_owned(l) {
            return;
        }
        let lingering = links::is_lingering(&self.graph, &self.canvas.links, out_port, in_port);
        let cmd = Command::TakeOver {
            link_id: l.id,
            out_node: l.out_node,
            out_port,
            in_node: l.in_node,
            in_port,
            lingering,
        };
        self.canvas.revealed.remove(&(out_port, in_port));
        self.requested.insert((out_port, in_port));
        self.remember_live_link(out_port, in_port);
        self.send(cmd);
    }

    /// Save a connection of ours on the currently viewed canvas.
    fn remember_link(&mut self, out_port: u32, in_port: u32) {
        if !self.graph.can_link_ports(out_port, in_port) {
            return;
        }
        if let Some(s) = links::from_ports(&self.graph, out_port, in_port) {
            if links::add(&mut self.canvas.links, s) {
                self.canvas.dirty = true;
            }
        }
    }

    /// Save a live PipeWire connection of ours into the active setup's saved links.
    fn remember_live_link(&mut self, out_port: u32, in_port: u32) {
        if !self.graph.can_link_ports(out_port, in_port) {
            return;
        }
        if let Some(s) = links::from_ports(&self.graph, out_port, in_port) {
            let target_links = if self.viewing == self.setups.active {
                &mut self.canvas.links
            } else {
                &mut self.setups.items[self.setups.active].data.links
            };
            if links::add(target_links, s) {
                self.canvas.dirty = true;
            }
        }
    }

    /// Create saved links whose nodes and ports exist now.
    fn auto_connect(&mut self) {
        let links = if self.viewing == self.setups.active {
            &self.canvas.links
        } else {
            &self.setups.items[self.setups.active].data.links
        };
        for l in links.iter().filter(|l| l.enabled) {
            if let Some((out_port, in_port)) = links::resolve(&self.graph, l) {
                let pair = (out_port, in_port);
                if !self.requested.contains(&pair) && !self.graph.owned_pairs.contains(&pair) {
                    if let Some(live) = self.graph.find_link(out_port, in_port) {
                        self.graph.owned_ids.insert(live.id);
                    }
                }
            }
        }
        let todo = links::missing(&self.graph, links, &self.requested);
        for (out_port, in_port) in todo {
            let (Some(po), Some(pi)) =
                (self.graph.ports.get(&out_port), self.graph.ports.get(&in_port))
            else {
                continue;
            };
            let lingering = links::is_lingering(&self.graph, links, out_port, in_port);
            let cmd = Command::CreateLink {
                out_node: po.node_id,
                out_port,
                in_node: pi.node_id,
                in_port,
                lingering,
            };
            self.requested.insert((out_port, in_port));
            self.send(cmd);
        }
    }

    /// Whether a node is referenced by any link in the active setup.
    fn is_node_in_saved_links(&self, node: &graph::NodeInfo) -> bool {
        let active_links = if self.viewing == self.setups.active {
            &self.canvas.links
        } else {
            &self.setups.items[self.setups.active].data.links
        };
        links::node_in_saved_links(active_links, &node.name, &node.key())
    }

    /// Bleed prevention: look once at each newly created external link and sever it
    /// if it is an auto-connection (created within a few seconds of its app
    /// appearing) of an app the active mode blocks. Links that already existed are
    /// never touched, so switching modes or adding a Redox link keeps them.
    fn enforce_bleed_prevention(&mut self) {
        if self.bleed_pending.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.bleed_pending);
        let mut candidates = Vec::new();
        for (link_id, seen_at) in pending {
            let Some(l) = self.graph.links.get(&link_id) else { continue };
            let auto_link = [l.out_node, l.in_node].iter().any(|n| {
                self.node_bleed.get(n).is_some_and(|(appeared, _)| {
                    seen_at.saturating_duration_since(*appeared) <= AUTO_LINK_WINDOW
                })
            });
            if auto_link {
                candidates.push(link_id);
            }
        }
        if candidates.is_empty() {
            return;
        }

        let (active_links, mode) = if self.viewing == self.setups.active {
            (&self.canvas.links, self.canvas.auto_connect_prevention)
        } else {
            let d = &self.setups.items[self.setups.active].data;
            (&d.links, d.auto_connect_prevention)
        };
        let info: HashMap<u32, links::BleedInfo> =
            self.node_bleed.iter().map(|(id, (_, i))| (*id, *i)).collect();
        let todo = links::bleed_links_to_sever(&self.graph, active_links, mode, &candidates, &info);

        for (link_id, out_port, in_port) in todo {
            if !self.severed_links.insert(link_id) {
                continue;
            }
            self.send(Command::DestroyLink {
                link_id: Some(link_id),
                out_port,
                in_port,
            });
        }
    }

    /// Store the canvas state into the setup being viewed.
    fn sync_current(&mut self) {
        self.setups.items[self.viewing].data = self.canvas.to_saved();
    }

    /// Show another setup on the canvas (its links are not touched).
    fn switch_to(&mut self, idx: usize) {
        if idx == self.viewing || idx >= self.setups.items.len() {
            return;
        }
        self.sync_current();
        let library = std::mem::take(&mut self.canvas.rules.library);
        let library_dirty = self.canvas.rules.library_dirty;
        let mut c = Canvas::with_saved(self.setups.items[idx].data.clone());
        c.rules.library = library;
        c.rules.library_dirty = library_dirty;
        c.theme = self.theme.clone();
        c.dirty = true;
        self.canvas = c;
        self.viewing = idx;
        self.prompt = None;
    }

    /// Make the viewed setup the live one: remove our links it does not use;
    /// auto-connect then creates the ones it wants.
    fn make_active(&mut self) {
        if self.viewing == self.setups.active {
            return;
        }
        self.sync_current();
        self.setups.active = self.viewing;
        let keep: HashSet<(u32, u32)> = self
            .canvas
            .links
            .iter()
            .filter(|l| l.enabled)
            .filter_map(|l| links::resolve(&self.graph, l))
            .collect();
        let drop: Vec<(Option<u32>, u32, u32)> = self
            .graph
            .links
            .values()
            .filter(|l| self.graph.is_owned(l))
            .filter(|l| !keep.contains(&(l.out_port, l.in_port)))
            .map(|l| (Some(l.id), l.out_port, l.in_port))
            .collect();
        for (link_id, out_port, in_port) in drop {
            self.send(Command::DestroyLink { link_id, out_port, in_port });
        }
        self.reconcile_virtual_devices();
        self.requested.clear();
        self.severed_links.clear();
        self.canvas.dirty = true;
    }

    fn render_tabs_bar_left(&mut self, ui: &mut egui::Ui, op: &mut Option<TabOp>) {
        for (i, s) in self.setups.items.iter().enumerate() {
            let is_viewing = i == self.viewing;
            let is_active = i == self.setups.active;
            let label = if is_active {
                format!("* {}", s.name)
            } else {
                s.name.clone()
            };

            let r = if is_viewing {
                let text = egui::RichText::new(&label).strong().color(egui::Color32::WHITE);
                ui.add(
                    egui::Button::new(text)
                        .fill(self.theme.c_accent())
                        .corner_radius(egui::CornerRadius::same(12)),
                )
            } else if is_active {
                let text = egui::RichText::new(&label).strong();
                ui.add(
                    egui::Button::new(text)
                        .stroke(egui::Stroke::new(1.5_f32, self.theme.c_accent()))
                        .corner_radius(egui::CornerRadius::same(12)),
                )
            } else {
                let text = egui::RichText::new(&label);
                ui.add(
                    egui::Button::new(text)
                        .corner_radius(egui::CornerRadius::same(12)),
                )
            };
            let r = if is_active {
                r.on_hover_text("Active setup (live in PipeWire)")
            } else {
                r
            };

            if r.clicked() {
                *op = Some(TabOp::View(i));
            }
            r.context_menu(|ui| {
                if ui.button("Rename…").clicked() {
                    *op = Some(TabOp::Rename(i));
                    ui.close_menu();
                }
                if ui.button("Duplicate").clicked() {
                    *op = Some(TabOp::Duplicate(i));
                    ui.close_menu();
                }
                if ui.button("Export…").clicked() {
                    *op = Some(TabOp::Export(i));
                    ui.close_menu();
                }
                let can_delete = !is_active && self.setups.items.len() > 1;
                if ui.add_enabled(can_delete, egui::Button::new("Delete")).clicked() {
                    *op = Some(TabOp::Delete(i));
                    ui.close_menu();
                }
            });
        }
        if ui
            .add(egui::Button::new(egui::RichText::new("+").strong()).corner_radius(egui::CornerRadius::same(12)))
            .on_hover_text("New setup: copies this layout and rules, without links or virtual devices")
            .clicked()
        {
            *op = Some(TabOp::Add);
        }
        if ui
            .add(egui::Button::new("Export…").corner_radius(egui::CornerRadius::same(12)))
            .on_hover_text("Export the viewed setup to a portable JSON file (library rule groups are embedded as unique local groups)")
            .clicked()
        {
            *op = Some(TabOp::Export(self.viewing));
        }
        if ui
            .add(egui::Button::new("Import…").corner_radius(egui::CornerRadius::same(12)))
            .on_hover_text("Import a setup from a JSON file or pasted JSON")
            .clicked()
        {
            *op = Some(TabOp::Import);
        }
        if self.viewing != self.setups.active {
            if ui
                .add(egui::Button::new(egui::RichText::new("Make active").strong()).fill(self.theme.c32_alpha(self.theme.accent, 75)).corner_radius(egui::CornerRadius::same(12)))
                .on_hover_text("Make this setup the live PipeWire configuration")
                .clicked()
            {
                *op = Some(TabOp::MakeActive);
            }
        }
    }

    fn render_tabs_bar_right(&mut self, ui: &mut egui::Ui) {
        let sinks: Vec<(u32, String)> = self
            .graph
            .nodes
            .values()
            .filter(|n| self.graph.is_node_owned(n) && n.is_sink())
            .map(|n| (n.id, format!("#{} {}", n.id, n.label())))
            .collect();
        let mics: Vec<(u32, String)> = self
            .graph
            .nodes
            .values()
            .filter(|n| self.graph.is_node_owned(n) && n.is_source())
            .map(|n| (n.id, format!("#{} {}", n.id, n.label())))
            .collect();

        // Tools popup menu: vertical list with left-opening hover flyouts
        ui.menu_button(egui::RichText::new("Tools").strong(), |ui| {
            ui.set_width(245.0);
            let mut close_menu = false;
            let mut active_flyout = self.tools_flyout;

            // 1. Virtual Devices
            let r_create_sink = ui.add(egui::Button::new("Create virtual sink…").wrap());
            if r_create_sink.clicked() {
                self.show_create_sink = Some(storage::VirtualDeviceKind::Sink);
                self.new_sink_name = format!(
                    "Virtual_Sink_{}",
                    sinks.len() + 1
                );
                self.new_sink_desc = "Virtual Sink".to_string();
                close_menu = true;
            }
            if r_create_sink.hovered() {
                active_flyout = None;
            }

            let sink_count = sinks.len();
            let sink_label = if sink_count > 0 {
                format!("Destroy virtual sink ({sink_count})    <")
            } else {
                "Destroy virtual sink (0)    <".to_string()
            };
            let r_destroy_sink = ui.add_enabled(sink_count > 0, egui::Button::new(sink_label).wrap());
            if r_destroy_sink.hovered() && sink_count > 0 {
                active_flyout = Some(ToolsSubmenu::DestroySink);
            }

            let r_create_mic = ui.add(egui::Button::new("Create virtual microphone…").wrap());
            if r_create_mic.clicked() {
                self.show_create_sink = Some(storage::VirtualDeviceKind::Source);
                self.new_sink_name = format!(
                    "Virtual_Mic_{}",
                    mics.len() + 1
                );
                self.new_sink_desc = "Virtual Mic".to_string();
                close_menu = true;
            }
            if r_create_mic.hovered() {
                active_flyout = None;
            }

            let mic_count = mics.len();
            let mic_label = if mic_count > 0 {
                format!("Destroy virtual mic ({mic_count})    <")
            } else {
                "Destroy virtual mic (0)    <".to_string()
            };
            let r_destroy_mic = ui.add_enabled(mic_count > 0, egui::Button::new(mic_label).wrap());
            if r_destroy_mic.hovered() && mic_count > 0 {
                active_flyout = Some(ToolsSubmenu::DestroyMic);
            }

            ui.separator();

            // 2. Windows & Panels
            let r_rules = ui.add(egui::Button::new("Filter rules").wrap());
            if r_rules.clicked() {
                self.show_rules = !self.show_rules;
                close_menu = true;
            }
            if r_rules.hovered() {
                active_flyout = None;
            }

            let r_theme = ui.add(egui::Button::new("Theme editor").wrap());
            if r_theme.clicked() {
                self.show_theme = !self.show_theme;
                close_menu = true;
            }
            if r_theme.hovered() {
                active_flyout = None;
            }

            let r_links = ui.add(egui::Button::new("Saved links").wrap());
            if r_links.clicked() {
                self.show_links = !self.show_links;
                close_menu = true;
            }
            if r_links.hovered() {
                active_flyout = None;
            }

            let r_sysinfo = ui.add(egui::Button::new("System info / issue report…").wrap());
            if r_sysinfo.clicked() {
                self.show_sysinfo = !self.show_sysinfo;
                close_menu = true;
            }
            if r_sysinfo.hovered() {
                active_flyout = None;
            }

            ui.separator();

            // 3. Maintenance
            let r_reset_layout = ui.add(egui::Button::new("Reset layout to columns").wrap());
            if r_reset_layout.clicked() {
                self.canvas.reset_layout();
                close_menu = true;
            }
            if r_reset_layout.hovered() {
                active_flyout = None;
            }

            let r_reset_all = ui.add(egui::Button::new("Reset all settings…").wrap());
            if r_reset_all.clicked() {
                self.confirm_reset = true;
                close_menu = true;
            }
            if r_reset_all.hovered() {
                active_flyout = None;
            }

            // Submenu flyout opening to the left
            if let Some(flyout) = active_flyout {
                let trigger_rect = match flyout {
                    ToolsSubmenu::DestroySink => r_destroy_sink.rect,
                    ToolsSubmenu::DestroyMic => r_destroy_mic.rect,
                };
                let menu_rect = ui.min_rect();
                let flyout_w = 260.0_f32;
                let flyout_x = (menu_rect.left() - flyout_w - 4.0).max(4.0);
                let flyout_y = (trigger_rect.top() - 4.0).max(4.0);
                let flyout_pos = egui::pos2(flyout_x, flyout_y);

                let area_resp = egui::Area::new(egui::Id::new("tools_hover_flyout"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(flyout_pos)
                    .show(ui.ctx(), |ui| {
                        let frame = egui::Frame::NONE
                            .fill(self.theme.c_window_bg())
                            .corner_radius(egui::CornerRadius::same(10))
                            .stroke(egui::Stroke::new(
                                1.0_f32,
                                if self.theme.is_dark { egui::Color32::from_gray(65) } else { egui::Color32::from_gray(190) },
                            ))
                            .shadow(egui::Shadow {
                                offset: [0, 4],
                                blur: 16,
                                spread: 0,
                                color: egui::Color32::from_black_alpha(if self.theme.is_dark { 120 } else { 35 }),
                            })
                            .inner_margin(egui::Margin::symmetric(10, 8));

                        frame.show(ui, |ui| {
                            ui.set_width(flyout_w - 20.0);
                            match flyout {
                                ToolsSubmenu::DestroySink => {
                                    ui.label(egui::RichText::new("Destroy virtual sink:").strong());
                                    ui.separator();
                                    if sinks.is_empty() {
                                        ui.weak("No virtual sinks present");
                                    } else {
                                        for (id, label) in &sinks {
                                            if ui.add(egui::Button::new(label).wrap()).clicked() {
                                                self.destroy_sink(*id);
                                                close_menu = true;
                                            }
                                        }
                                    }
                                }
                                ToolsSubmenu::DestroyMic => {
                                    ui.label(egui::RichText::new("Destroy virtual microphone:").strong());
                                    ui.separator();
                                    if mics.is_empty() {
                                        ui.weak("No virtual mics present");
                                    } else {
                                        for (id, label) in &mics {
                                            if ui.add(egui::Button::new(label).wrap()).clicked() {
                                                self.destroy_sink(*id);
                                                close_menu = true;
                                            }
                                        }
                                    }
                                }
                            }
                        });
                    });

                let flyout_rect = area_resp.response.rect;
                let mouse_pos = ui.ctx().input(|i| i.pointer.hover_pos());
                let mouse_in_flyout = mouse_pos.is_some_and(|p| flyout_rect.contains(p));
                let mouse_in_trigger = mouse_pos.is_some_and(|p| trigger_rect.contains(p));

                if mouse_in_flyout || mouse_in_trigger {
                    self.tools_flyout = Some(flyout);
                } else {
                    self.tools_flyout = active_flyout;
                }
            } else {
                self.tools_flyout = None;
            }

            if close_menu {
                self.tools_flyout = None;
                ui.close_menu();
            }
        });

        // Sinks dropdown
        let sinks_text = format!("Sinks: {}", self.canvas.sink_hide_mode.label());
        ui.menu_button(sinks_text, |ui| {
            for mode in storage::SinkHideMode::ALL {
                if ui.selectable_label(self.canvas.sink_hide_mode == mode, mode.label()).clicked() {
                    self.canvas.sink_hide_mode = mode;
                    self.canvas.dirty = true;
                    ui.close_menu();
                }
            }
        }).response.on_hover_text("Control visibility of audio sinks");

        // Bleed dropdown
        let bleed_text = format!("Bleed: {}", self.canvas.auto_connect_prevention.label());
        ui.menu_button(bleed_text, |ui| {
            for mode in storage::AutoConnectPrevention::ALL {
                if ui.selectable_label(self.canvas.auto_connect_prevention == mode, mode.label()).clicked() {
                    self.canvas.auto_connect_prevention = mode;
                    self.severed_links.clear();
                    self.canvas.dirty = true;
                    ui.close_menu();
                }
            }
        }).response.on_hover_text("Audio bleed / auto-connection prevention.\nOnly cuts the automatic links an app gets in its first seconds; existing links are never removed.\n• Allow all: normal PipeWire behavior\n• Block for managed nodes: apps that had saved links when they appeared\n• Block for all new apps: also every app launched after RedoxRouter started\nApps that come back within 5 minutes keep their first decision.");

        // Show external links toggle
        ui.horizontal(|ui| {
            ui.label("Show external links");
            if toggle_switch(ui, &mut self.canvas.show_external).on_hover_text("Show connections created outside RedoxRouter").changed() {
                self.canvas.dirty = true;
            }
        });

        let hidden: Vec<String> = self.canvas.hidden.iter().cloned().collect();
        if !hidden.is_empty() {
            ui.menu_button(format!("Hidden ({})", hidden.len()), |ui| {
                for name in &hidden {
                    let present = self.graph.nodes.values().any(|n| n.key() == *name || n.name == *name);
                    let label = if present { format!("Show {name}") } else { format!("Show {name} (not present)") };
                    if ui.button(label).clicked() {
                        self.canvas.unhide(name);
                        ui.close_menu();
                    }
                }
                ui.separator();
                if ui.button("Show all").clicked() {
                    self.canvas.unhide_all();
                    ui.close_menu();
                }
            });
        }
    }

    fn tabs_bar(&mut self, ctx: &egui::Context) {
        let mut op = None;
        egui::TopBottomPanel::top("tabs").frame(bar_frame(&self.theme)).show(ctx, |ui| {
            let total_width = ui.available_width();
            if total_width >= 960.0 {
                let mut fit_on_one_line = true;
                ui.horizontal(|ui| {
                    self.render_tabs_bar_left(ui, &mut op);
                    if ui.available_width() >= 480.0 {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.render_tabs_bar_right(ui);
                        });
                    } else {
                        fit_on_one_line = false;
                    }
                });
                if !fit_on_one_line {
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        self.render_tabs_bar_right(ui);
                    });
                }
            } else {
                ui.horizontal_wrapped(|ui| {
                    self.render_tabs_bar_left(ui, &mut op);
                });
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    self.render_tabs_bar_right(ui);
                });
            }
        });
        match op {
            Some(TabOp::View(i)) => self.switch_to(i),
            Some(TabOp::Add) => {
                self.sync_current();
                let mut data = self.setups.items[self.viewing].data.clone();
                data.links.clear();
                data.virtual_devices.clear();
                let i = self.setups.add("New setup", data);
                self.switch_to(i);
            }
            Some(TabOp::Duplicate(i)) => {
                self.sync_current();
                let data = self.setups.items[i].data.clone();
                let name = format!("{} copy", self.setups.items[i].name);
                self.setups.add(&name, data);
                self.canvas.dirty = true;
            }
            Some(TabOp::Rename(i)) => self.renaming = Some((i, self.setups.items[i].name.clone())),
            Some(TabOp::Export(i)) => {
                self.sync_current();
                if let Some(s) = self.setups.items.get(i) {
                    let path = storage::default_export_path(&s.name).display().to_string();
                    self.exporting_setup = Some((i, path, None));
                }
            }
            Some(TabOp::Import) => {
                let default_dir = storage::exports_dir()
                    .map(|d| format!("{}/", d.display()))
                    .unwrap_or_default();
                self.importing_setup = Some((default_dir, String::new(), None));
            }
            Some(TabOp::Delete(i)) => {
                if i == self.viewing {
                    self.switch_to(self.setups.active);
                }
                if self.setups.delete(i, &mut self.viewing) {
                    self.canvas.dirty = true;
                }
            }
            Some(TabOp::MakeActive) => self.make_active(),
            None => {}
        }
    }

    /// Back to a fresh install: one empty setup, no rules, no library, no saved
    /// links or virtual sinks. Our live links and sinks are removed too.
    fn reset_all(&mut self) {
        let live: Vec<(Option<u32>, u32, u32)> = self
            .graph
            .links
            .values()
            .filter(|l| self.graph.is_owned(l))
            .map(|l| (Some(l.id), l.out_port, l.in_port))
            .collect();
        for (link_id, out_port, in_port) in live {
            self.send(Command::DestroyLink { link_id, out_port, in_port });
        }
        let owned_sinks: Vec<u32> = self
            .graph
            .nodes
            .values()
            .filter(|n| self.graph.is_node_owned(n))
            .map(|n| n.id)
            .collect();
        for node_id in owned_sinks {
            self.send(Command::DestroySink { node_id });
        }
        let _ = storage::save_sinks(&[]);
        self.setups = setups::SetupList::new("Main", storage::LayoutFile::default());
        self.viewing = 0;
        self.theme = Theme::default();
        let _ = storage::save_theme(&self.theme);
        let mut canvas = Canvas::with_saved(storage::LayoutFile::default());
        canvas.theme = self.theme.clone();
        canvas.rules.library_dirty = true; // writes the empty library
        canvas.dirty = true;
        self.canvas = canvas;
        self.requested.clear();
        self.severed_links.clear();
        self.prompt = None;
        self.renaming = None;
    }

    fn reset_window(&mut self, ctx: &egui::Context) {
        if !self.confirm_reset {
            return;
        }
        let (mut yes, mut no) = (false, false);
        egui::Window::new("Reset all settings").collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(
                "This deletes every setup, saved link, rule, rule library group, node position, \
                 nickname and hidden node, and removes your live links. It cannot be undone.",
            );
            ui.horizontal(|ui| {
                yes = ui.button("Reset everything").clicked();
                no = ui.button("Cancel").clicked();
            });
        });
        if yes {
            self.reset_all();
            self.theme.apply(ctx);
        }
        if yes || no {
            self.confirm_reset = false;
        }
    }

    fn create_sink_window(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.show_create_sink else {
            return;
        };
        let is_mic = kind == storage::VirtualDeviceKind::Source;
        let title = if is_mic { "Create Virtual Microphone" } else { "Create Virtual Sink" };
        let desc_label = if is_mic {
            "Create a virtual microphone (Audio/Source/Virtual):"
        } else {
            "Create a virtual audio sink (null audio sink):"
        };
        let mut create = false;
        let mut cancel = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .default_width(320.0)
            .show(ctx, |ui| {
                ui.label(desc_label);
                ui.add_space(4.0);
                egui::Grid::new("create_sink_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label("Technical Name:");
                    ui.add(egui::TextEdit::singleline(&mut self.new_sink_name).hint_text("node.name").desired_width(180.0));
                    ui.end_row();

                    ui.label("Description:");
                    ui.add(egui::TextEdit::singleline(&mut self.new_sink_desc).hint_text("label shown on node").desired_width(180.0));
                    ui.end_row();
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Create").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        create = true;
                    }
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        cancel = true;
                    }
                });
            });
        if create {
            let name = self.new_sink_name.trim().to_string();
            let desc = self.new_sink_desc.trim().to_string();
            if !name.is_empty() {
                let description = if desc.is_empty() { name.clone() } else { desc };
                self.create_virtual_device(name, description, kind, false);
            }
            self.show_create_sink = None;
        }
        if cancel {
            self.show_create_sink = None;
        }
    }

    fn export_window(&mut self, ctx: &egui::Context) {
        let Some((idx, ref mut path_str, ref mut status)) = self.exporting_setup else {
            return;
        };
        let Some(setup) = self.setups.items.get(idx) else {
            self.exporting_setup = None;
            return;
        };
        let json_res = storage::export_setup_json(setup, &self.canvas.rules.library);
        let mut close = false;
        let mut save_file = false;
        let mut copy_clip = false;
        egui::Window::new(format!("Export setup \"{}\"", setup.name))
            .collapsible(false)
            .resizable(true)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.label(
                    "Exports this setup as a standalone, 100% portable JSON file. \
                     Any library rule groups used by this setup are embedded as unique local groups.",
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("File path:");
                    ui.add(
                        egui::TextEdit::singleline(path_str)
                            .hint_text("/path/to/setup.json")
                            .desired_width(340.0),
                    );
                });
                ui.horizontal(|ui| {
                    if ui.button("Save to file").clicked() {
                        save_file = true;
                    }
                    if ui.button("Copy JSON to clipboard").clicked() {
                        copy_clip = true;
                    }
                    if ui.button("Close").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        close = true;
                    }
                });
                if let Some(res) = status {
                    match res {
                        Ok(msg) => {
                            ui.colored_label(egui::Color32::from_rgb(110, 210, 140), msg);
                        }
                        Err(err) => {
                            ui.colored_label(egui::Color32::LIGHT_RED, err);
                        }
                    }
                }
                if let Ok(ref json) = json_res {
                    ui.collapsing("Preview exported JSON", |ui| {
                        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                            let mut preview = json.as_str();
                            ui.add(
                                egui::TextEdit::multiline(&mut preview)
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(f32::INFINITY),
                            );
                        });
                    });
                }
            });
        if save_file {
            let expanded = storage::expand_tilde(path_str);
            if let Some(setup) = self.setups.items.get(idx) {
                let res = storage::export_setup_to_path(&expanded, setup, &self.canvas.rules.library)
                    .map(|()| format!("Saved to {}", expanded.display()));
                if let Some((_, _, st)) = &mut self.exporting_setup {
                    *st = Some(res);
                }
            }
        }
        if copy_clip {
            if let Ok(json) = json_res {
                ctx.copy_text(json);
                if let Some((_, _, st)) = &mut self.exporting_setup {
                    *st = Some(Ok("Copied portable setup JSON to clipboard.".to_string()));
                }
            }
        }
        if close {
            self.exporting_setup = None;
        }
    }

    fn import_window(&mut self, ctx: &egui::Context) {
        let Some((ref mut path_str, ref mut raw_json, ref mut err_msg)) = self.importing_setup else {
            return;
        };
        let export_files = storage::list_export_files();
        let mut close = false;
        let mut load_from_path: Option<std::path::PathBuf> = None;
        let mut load_from_raw = false;
        egui::Window::new("Import Setup")
            .collapsible(false)
            .resizable(true)
            .default_width(480.0)
            .show(ctx, |ui| {
                if !export_files.is_empty() {
                    ui.strong("Saved exports in config folder:");
                    egui::ScrollArea::vertical().max_height(120.0).show(ui, |ui| {
                        for file in &export_files {
                            ui.horizontal(|ui| {
                                let name = file
                                    .file_name()
                                    .and_then(|s| s.to_str())
                                    .unwrap_or("setup.json");
                                ui.monospace(name);
                                if ui.small_button("Import").clicked() {
                                    load_from_path = Some(file.clone());
                                }
                            });
                        }
                    });
                    ui.separator();
                }
                ui.label("Import from a JSON file path:");
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(path_str)
                            .hint_text("~/path/to/setup.json")
                            .desired_width(320.0),
                    );
                    if ui.button("Load file").clicked() {
                        load_from_path = Some(storage::expand_tilde(path_str));
                    }
                });
                ui.separator();
                ui.label("Or paste exported setup JSON below:");
                egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(raw_json)
                            .font(egui::TextStyle::Monospace)
                            .hint_text("{ \"name\": \"...\", \"data\": { ... } }")
                            .desired_rows(5)
                            .desired_width(f32::INFINITY),
                    );
                });
                if let Some(err) = err_msg {
                    ui.colored_label(egui::Color32::LIGHT_RED, err);
                }
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!raw_json.trim().is_empty(), egui::Button::new("Import pasted JSON"))
                        .clicked()
                    {
                        load_from_raw = true;
                    }
                    if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        close = true;
                    }
                });
            });

        let imported = if let Some(path) = load_from_path {
            Some(storage::import_setup_from_path(&path, &self.canvas.rules.library))
        } else if load_from_raw {
            let json = self
                .importing_setup
                .as_ref()
                .map(|(_, j, _)| j.clone())
                .unwrap_or_default();
            Some(storage::parse_imported_setup(&json, "Imported", &self.canvas.rules.library))
        } else {
            None
        };

        if let Some(res) = imported {
            match res {
                Ok(setup) => {
                    self.sync_current();
                    let idx = self.setups.add(&setup.name, setup.data);
                    self.switch_to(idx);
                    self.canvas.dirty = true;
                    self.importing_setup = None;
                }
                Err(e) => {
                    if let Some((_, _, err)) = &mut self.importing_setup {
                        *err = Some(e);
                    }
                }
            }
        } else if close {
            self.importing_setup = None;
        }
    }

    fn inactive_window(&mut self, ctx: &egui::Context) {
        if !self.inactive_notice {
            return;
        }
        let (mut ok, mut activate) = (false, false);
        egui::Window::new("Setup not active").collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(
                "Connections can only be taken over on the active setup (marked *). \
                 The live PipeWire links you see here belong to the active setup, so nothing was changed.",
            );
            ui.horizontal(|ui| {
                activate = ui.button("Make this setup active").clicked();
                ok = ui.button("OK").clicked();
            });
        });
        if activate {
            self.make_active();
        }
        if ok || activate {
            self.inactive_notice = false;
        }
    }

    fn rename_window(&mut self, ctx: &egui::Context) {
        let Some((idx, text)) = &mut self.renaming else { return };
        let (mut done, mut cancel) = (false, false);
        egui::Window::new("Rename setup").collapsible(false).resizable(false).show(ctx, |ui| {
            let r = ui.text_edit_singleline(text);
            r.request_focus();
            ui.horizontal(|ui| {
                done = ui.button("OK").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter));
                cancel = ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
            });
        });
        if done {
            let (idx, text) = (*idx, text.clone());
            self.setups.rename(idx, &text);
            self.canvas.dirty = true;
            self.renaming = None;
        } else if cancel {
            self.renaming = None;
        }
    }

    fn links_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_links;
        let mut remove = None;
        let mut changed = false;
        let mut to_disconnect: Vec<(u32, u32)> = Vec::new();
        let mut to_linger: Vec<(u32, u32, bool)> = Vec::new();
        egui::Window::new("Saved links").open(&mut open).default_width(560.0).show(ctx, |ui| {
            ui.label(
                "Links you create are saved by node and port name and reconnected when both \
                 exist. Untick to turn off/pause; Remove forgets it.",
            );
            if self.canvas.links.is_empty() {
                ui.weak("No saved links yet.");
            }
            for (i, l) in self.canvas.links.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut l.enabled, "").changed() {
                        changed = true;
                        if !l.enabled {
                            if let Some(pair) = links::resolve(&self.graph, l) {
                                to_disconnect.push(pair);
                            }
                        }
                    }
                    if ui
                        .checkbox(&mut l.has_switch, "switch")
                        .on_hover_text("Show an inline on/off switch box on this cable")
                        .changed()
                    {
                        changed = true;
                        if !l.has_switch && !l.enabled {
                            l.enabled = true;
                        }
                    }
                    if ui
                        .checkbox(&mut l.lingering, "linger")
                        .on_hover_text("Keep link alive after RedoxRouter exits")
                        .changed()
                    {
                        changed = true;
                        if l.enabled {
                            if let Some((o, n)) = links::resolve(&self.graph, l) {
                                to_linger.push((o, n, l.lingering));
                            }
                        }
                    }
                    ui.label(l.label());
                    let state = match links::resolve(&self.graph, l) {
                        Some((o, n))
                            if self.viewing == self.setups.active
                                && self.graph.find_link(o, n).is_some() =>
                        {
                            "connected"
                        }
                        Some(_) if !l.enabled => "switched off",
                        Some(_) => "ready",
                        None => "waiting for nodes",
                    };
                    ui.weak(state);
                    if ui.button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
            }
        });
        if let Some(i) = remove {
            if self.viewing == self.setups.active {
                if let Some(pair) = links::resolve(&self.graph, &self.canvas.links[i]) {
                    to_disconnect.push(pair);
                }
            }
            self.canvas.links.remove(i);
            changed = true;
        }
        if self.viewing == self.setups.active {
            for (out_port, in_port) in to_disconnect {
                let link_id = self.graph.find_link(out_port, in_port).map(|l| l.id);
                self.requested.remove(&(out_port, in_port));
                self.send(Command::DestroyLink { link_id, out_port, in_port });
            }
            for (out_port, in_port, lingering) in to_linger {
                if self.graph.find_link(out_port, in_port).is_some() {
                    self.handle(Action::SetLinkLinger { out_port, in_port, lingering });
                }
            }
        }
        if changed {
            self.canvas.dirty = true;
            self.requested.clear();
        }
        self.show_links = open;
    }

    fn rules_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_rules;
        let mut setup_changed = false;
        let mut lib_changed = false;
        let mut op: Option<Op> = None;
        egui::Window::new("Rules")
            .open(&mut open)
            .default_width(720.0)
            .default_height(460.0)
            .show(ctx, |ui| {
                ui.label(
                    "Rules run top to bottom (Hide / Show sets visibility; the first Color wins). \
                     Text matches anywhere (case-insensitive); use * (any text) and ? (one \
                     character) for wildcards. \"Title\" is the text shown on the node. \
                     Combine 'Hide all sinks' with 'Show' rules to filter which sinks to display.",
                );
                let mut classes: Vec<String> = self
                    .graph
                    .nodes
                    .values()
                    .map(|n| n.media_class.clone())
                    .filter(|c| !c.is_empty())
                    .collect();
                classes.sort();
                classes.dedup();
                let cx = RuleCtx {
                    nodes: &self.graph.nodes,
                    nicks: &self.canvas.nicknames,
                    classes: &classes,
                };
                if ui.button("Add group").clicked() {
                    op = Some(Op::AddGroup);
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let RuleSet { groups, library, .. } = &mut self.canvas.rules;
                    for (gi, entry) in groups.iter_mut().enumerate() {
                        ui.group(|ui| match entry {
                            GroupEntry::Local(g) => {
                                ui.horizontal(|ui| {
                                    setup_changed |= ui.checkbox(&mut g.enabled, "").changed();
                                    setup_changed |= ui
                                        .add(
                                            egui::TextEdit::singleline(&mut g.name)
                                                .hint_text("group name")
                                                .desired_width(180.0),
                                        )
                                        .changed();
                                    group_buttons(ui, gi, "To library", Op::ToLibrary(gi), &mut op);
                                });
                                ui.indent(("rules", gi), |ui| {
                                    setup_changed |= rules_ui(ui, gi, &mut g.rules, &cx);
                                });
                            }
                            GroupEntry::Library { name, enabled } => {
                                ui.horizontal(|ui| {
                                    setup_changed |= ui.checkbox(enabled, "").changed();
                                    ui.strong(name.as_str());
                                    ui.weak("library group (shared)");
                                    group_buttons(ui, gi, "Copy here", Op::Unlink(gi), &mut op);
                                });
                                match library.iter_mut().find(|g| g.name == *name) {
                                    Some(g) => {
                                        ui.indent(("rules", gi), |ui| {
                                            lib_changed |= rules_ui(ui, gi, &mut g.rules, &cx);
                                        });
                                    }
                                    None => {
                                        ui.weak("This group is missing from the library.");
                                    }
                                }
                            }
                        });
                    }
                    ui.separator();
                    ui.strong("Library (shared by all setups)");
                    if library.is_empty() {
                        ui.weak("Empty. Use \"To library\" on a group to share it.");
                    }
                    for g in library.iter() {
                        let used = groups.iter().any(
                            |e| matches!(e, GroupEntry::Library { name, .. } if *name == g.name),
                        );
                        ui.horizontal(|ui| {
                            ui.label(format!("{} ({} rules)", g.name, g.rules.len()));
                            if used {
                                ui.weak("in use here");
                            } else if ui.button("Use here").clicked() {
                                op = Some(Op::UseLib(g.name.clone()));
                            }
                            if ui.button("Delete from library").clicked() {
                                op = Some(Op::DeleteLib(g.name.clone()));
                            }
                        });
                    }
                });
            });
        self.show_rules = open;

        let rs = &mut self.canvas.rules;
        if let Some(op) = op {
            setup_changed = true;
            match op {
                Op::AddGroup => rs.add_group(),
                Op::Remove(i) => {
                    rs.groups.remove(i);
                }
                Op::ToLibrary(i) => rs.to_library(i),
                Op::Unlink(i) => rs.unlink(i),
                Op::Up(i) if i > 0 => rs.groups.swap(i - 1, i),
                Op::Down(i) if i + 1 < rs.groups.len() => rs.groups.swap(i, i + 1),
                Op::Up(_) | Op::Down(_) => {}
                Op::UseLib(name) => rs.use_library(&name),
                Op::DeleteLib(name) => {
                    if let Some(src) = rs.library.iter().find(|g| g.name == name).cloned() {
                        for setup in &mut self.setups.items {
                            for entry in &mut setup.data.groups {
                                if let GroupEntry::Library { name: n, enabled } = entry {
                                    if *n == name {
                                        let mut g = src.clone();
                                        g.enabled = *enabled;
                                        *entry = GroupEntry::Local(g);
                                    }
                                }
                            }
                        }
                    }
                    rs.delete_from_library(&name);
                }
            }
        }
        if lib_changed {
            rs.library_dirty = true;
        }
        if setup_changed {
            self.canvas.dirty = true;
        }
    }

    fn theme_window(&mut self, ctx: &egui::Context) {
        if !self.show_theme {
            return;
        }
        let mut open = self.show_theme;
        let mut theme_changed = false;
        let mut apply_preset: Option<Theme> = None;
        let mut export_to_file = false;
        let mut copy_clipboard = false;
        let mut load_from_path: Option<std::path::PathBuf> = None;
        let mut load_from_pasted = false;
        let mut save_to_theme_path: Option<std::path::PathBuf> = None;
        let mut save_as_new_named: Option<String> = None;
        let mut delete_theme_path: Option<std::path::PathBuf> = None;
        let mut export_clipboard_theme: Option<Theme> = None;

        let dark = Theme::dark();
        let light = Theme::light();

        egui::Window::new("Theme Editor")
            .open(&mut open)
            .resizable(true)
            .default_width(640.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.strong("Built-in Presets:");
                    if ui.button("Redox Dark").on_hover_text("Original dark theme").clicked() {
                        apply_preset = Some(Theme::dark());
                    }
                    if ui.button("Redox Light").on_hover_text("Clean high-contrast light theme").clicked() {
                        apply_preset = Some(Theme::light());
                    }
                    if ui.button("Catppuccin").on_hover_text("Catppuccin Mocha dark pastel palette").clicked() {
                        apply_preset = Some(Theme::catppuccin());
                    }
                    if ui.button("Nord").on_hover_text("Arctic north-bluish palette").clicked() {
                        apply_preset = Some(Theme::nord());
                    }
                    if ui.button("Cyberpunk").on_hover_text("High-contrast neon cyberpunk palette").clicked() {
                        apply_preset = Some(Theme::cyberpunk());
                    }
                    ui.separator();
                    ui.label("Name:");
                    if ui.text_edit_singleline(&mut self.theme.name).changed() {
                        theme_changed = true;
                    }
                    let saved = storage::list_saved_themes();
                    ui.menu_button("Save to theme", |ui| {
                        if !saved.is_empty() {
                            ui.label(egui::RichText::new("Update existing theme:").strong());
                            for (path, res) in &saved {
                                let name = match res {
                                    Ok(t) => t.name.clone(),
                                    Err(_) => path.file_stem().and_then(|s| s.to_str()).unwrap_or("theme").to_string(),
                                };
                                if ui.button(format!("Update \"{}\"", name)).clicked() {
                                    save_to_theme_path = Some(path.clone());
                                    ui.close_menu();
                                }
                            }
                            ui.separator();
                        }
                        if ui.button(format!("Save as new: \"{}\"", self.theme.name)).clicked() {
                            save_as_new_named = Some(self.theme.name.clone());
                            ui.close_menu();
                        }
                    });
                });

                if ui
                    .checkbox(&mut self.theme.is_dark, "Dark base visuals")
                    .on_hover_text("Switches between dark and light base widget typography and background foundations")
                    .changed()
                {
                    theme_changed = true;
                }

                ui.separator();

                egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
                    ui.collapsing("Typography & Font Sizes", |ui| {
                        ui.horizontal(|ui| {
                            ui.label("UI font size adjustment:");
                            if ui
                                .add(
                                    egui::Slider::new(&mut self.theme.ui_font_size_delta, -4.0..=12.0)
                                        .step_by(0.5)
                                        .suffix(" pt"),
                                )
                                .on_hover_text("Adjust all window and panel font sizes")
                                .changed()
                            {
                                theme_changed = true;
                            }
                            if ui.small_button("-").on_hover_text("Decrease UI font size by 1 pt").clicked() {
                                self.theme.ui_font_size_delta = (self.theme.ui_font_size_delta - 1.0).max(-4.0);
                                theme_changed = true;
                            }
                            if ui.small_button("+").on_hover_text("Increase UI font size by 1 pt").clicked() {
                                self.theme.ui_font_size_delta = (self.theme.ui_font_size_delta + 1.0).min(12.0);
                                theme_changed = true;
                            }
                            if ui.small_button("Reset").on_hover_text("Reset to standard UI font size").clicked() {
                                self.theme.ui_font_size_delta = 0.0;
                                theme_changed = true;
                            }
                        });

                        ui.horizontal(|ui| {
                            ui.label("Graph node font size adjustment:");
                            if ui
                                .add(
                                    egui::Slider::new(&mut self.theme.node_font_size_delta, -4.0..=12.0)
                                        .step_by(0.5)
                                        .suffix(" pt"),
                                )
                                .on_hover_text("Adjust canvas node title, port, and volume font sizes")
                                .changed()
                            {
                                theme_changed = true;
                            }
                            if ui.small_button("-").on_hover_text("Decrease node font size by 1 pt").clicked() {
                                self.theme.node_font_size_delta = (self.theme.node_font_size_delta - 1.0).max(-4.0);
                                theme_changed = true;
                            }
                            if ui.small_button("+").on_hover_text("Increase node font size by 1 pt").clicked() {
                                self.theme.node_font_size_delta = (self.theme.node_font_size_delta + 1.0).min(12.0);
                                theme_changed = true;
                            }
                            if ui.small_button("Reset").on_hover_text("Reset to standard node font size").clicked() {
                                self.theme.node_font_size_delta = 0.0;
                                theme_changed = true;
                            }
                        });

                        if ui
                            .button("Reset all font sizes to default")
                            .on_hover_text("Reset both UI and node font scaling to 0 pt")
                            .clicked()
                        {
                            self.theme.ui_font_size_delta = 0.0;
                            self.theme.node_font_size_delta = 0.0;
                            theme_changed = true;
                        }
                    });

                    let color_row = |ui: &mut egui::Ui, changed: &mut bool, label: &str, color: &mut [u8; 3], dark_c: [u8; 3], light_c: [u8; 3]| {
                        ui.label(label);
                        if ui.color_edit_button_srgb(color).changed() {
                            *changed = true;
                        }
                        ui.horizontal(|ui| {
                            if ui
                                .small_button("Reset to dark default")
                                .on_hover_text("Reset this color to its dark theme default")
                                .clicked()
                            {
                                *color = dark_c;
                                *changed = true;
                            }
                            if ui
                                .small_button("Reset to light default")
                                .on_hover_text("Reset this color to its light theme default")
                                .clicked()
                            {
                                *color = light_c;
                                *changed = true;
                            }
                        });
                        ui.end_row();
                    };

                    ui.collapsing("UI Visuals", |ui| {
                        egui::Grid::new("theme_ui_grid").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
                            color_row(ui, &mut theme_changed, "Panel background:", &mut self.theme.panel_bg, dark.panel_bg, light.panel_bg);
                            color_row(ui, &mut theme_changed, "Window background:", &mut self.theme.window_bg, dark.window_bg, light.window_bg);
                            color_row(ui, &mut theme_changed, "Accent color:", &mut self.theme.accent, dark.accent, light.accent);
                            color_row(ui, &mut theme_changed, "Buttons (inactive):", &mut self.theme.widget_bg, dark.widget_bg, light.widget_bg);
                            color_row(ui, &mut theme_changed, "Buttons (hovered):", &mut self.theme.widget_hover, dark.widget_hover, light.widget_hover);
                            color_row(ui, &mut theme_changed, "Buttons (active / selected):", &mut self.theme.widget_active, dark.widget_active, light.widget_active);
                            color_row(ui, &mut theme_changed, "Faint background:", &mut self.theme.faint_bg, dark.faint_bg, light.faint_bg);
                            color_row(ui, &mut theme_changed, "Text edit / extreme background:", &mut self.theme.extreme_bg, dark.extreme_bg, light.extreme_bg);
                        });
                    });

                    ui.collapsing("Canvas & Grid", |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Grid pattern:");
                            egui::ComboBox::from_id_salt("theme_grid_pattern_combo")
                                .selected_text(match self.theme.grid_pattern {
                                    GridPattern::Dots => "Dots",
                                    GridPattern::Lines => "Lines",
                                    GridPattern::None => "None",
                                })
                                .show_ui(ui, |ui| {
                                    if ui.selectable_value(&mut self.theme.grid_pattern, GridPattern::Dots, "Dots").changed() {
                                        theme_changed = true;
                                    }
                                    if ui.selectable_value(&mut self.theme.grid_pattern, GridPattern::Lines, "Lines").changed() {
                                        theme_changed = true;
                                    }
                                    if ui.selectable_value(&mut self.theme.grid_pattern, GridPattern::None, "None").changed() {
                                        theme_changed = true;
                                    }
                                });
                        });
                        egui::Grid::new("theme_canvas_grid").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
                            color_row(ui, &mut theme_changed, "Canvas background:", &mut self.theme.canvas_bg, dark.canvas_bg, light.canvas_bg);
                            color_row(ui, &mut theme_changed, "Grid dot / line:", &mut self.theme.grid_dot, dark.grid_dot, light.grid_dot);
                        });
                    });

                    ui.collapsing("Nodes", |ui| {
                        egui::Grid::new("theme_nodes_grid").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
                            color_row(ui, &mut theme_changed, "Node card body:", &mut self.theme.node_body, dark.node_body, light.node_body);
                            color_row(ui, &mut theme_changed, "Node outline / border:", &mut self.theme.node_stroke, dark.node_stroke, light.node_stroke);
                            color_row(ui, &mut theme_changed, "Port text in node:", &mut self.theme.node_text, dark.node_text, light.node_text);
                            color_row(ui, &mut theme_changed, "Col 0: Sources / Capture header:", &mut self.theme.col_source, dark.col_source, light.col_source);
                            color_row(ui, &mut theme_changed, "Col 1: Applications / Streams header:", &mut self.theme.col_stream, dark.col_stream, light.col_stream);
                            color_row(ui, &mut theme_changed, "Col 2: Sinks / Playback header:", &mut self.theme.col_sink, dark.col_sink, light.col_sink);
                        });
                        ui.separator();
                        if ui.checkbox(&mut self.theme.node_glow_enabled, "Node activity glow pulse")
                            .on_hover_text("Slow, faint expanding glow around nodes actively streaming audio or video (unmuted)")
                            .changed()
                        {
                            theme_changed = true;
                        }
                        if self.theme.node_glow_enabled {
                            egui::Grid::new("theme_node_glow_grid").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
                                color_row(ui, &mut theme_changed, "Source activity glow:", &mut self.theme.node_glow_source, dark.node_glow_source, light.node_glow_source);
                                color_row(ui, &mut theme_changed, "Stream activity glow:", &mut self.theme.node_glow_stream, dark.node_glow_stream, light.node_glow_stream);
                                color_row(ui, &mut theme_changed, "Sink activity glow:", &mut self.theme.node_glow_sink, dark.node_glow_sink, light.node_glow_sink);
                            });
                        }
                    });

                    ui.collapsing("Ports", |ui| {
                        egui::Grid::new("theme_ports_grid").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
                            color_row(ui, &mut theme_changed, "Output port:", &mut self.theme.port_out, dark.port_out, light.port_out);
                            color_row(ui, &mut theme_changed, "Input port:", &mut self.theme.port_in, dark.port_in, light.port_in);
                        });
                    });

                    ui.collapsing("Cables & Selection", |ui| {
                        egui::Grid::new("theme_cables_grid").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
                            color_row(ui, &mut theme_changed, "Owned cable:", &mut self.theme.cable_owned, dark.cable_owned, light.cable_owned);
                            color_row(ui, &mut theme_changed, "Switch cable:", &mut self.theme.cable_switch, dark.cable_switch, light.cable_switch);
                            color_row(ui, &mut theme_changed, "External cable:", &mut self.theme.cable_external, dark.cable_external, light.cable_external);
                            color_row(ui, &mut theme_changed, "Selected cable / highlight:", &mut self.theme.cable_selected, dark.cable_selected, light.cable_selected);
                        });
                    });

                    ui.collapsing("Link Effects", |ui| {
                        ui.label(egui::RichText::new("Cable Thickness:").strong());
                        egui::Grid::new("cable_thick_grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                            ui.label("Owned cable thickness:");
                            if ui.add(egui::Slider::new(&mut self.theme.cable_thickness_owned, 0.5..=6.0).step_by(0.1).suffix(" px")).changed() {
                                theme_changed = true;
                            }
                            ui.end_row();
                            ui.label("Switch cable thickness:");
                            if ui.add(egui::Slider::new(&mut self.theme.cable_thickness_switch, 0.5..=6.0).step_by(0.1).suffix(" px")).changed() {
                                theme_changed = true;
                            }
                            ui.end_row();
                            ui.label("External cable thickness:");
                            if ui.add(egui::Slider::new(&mut self.theme.cable_thickness_external, 0.5..=6.0).step_by(0.1).suffix(" px")).changed() {
                                theme_changed = true;
                            }
                            ui.end_row();
                        });

                        ui.separator();
                        ui.label(egui::RichText::new("Bloom / Glow Stroke:").strong());
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut self.theme.cable_glow_owned, "Owned cables").changed() {
                                theme_changed = true;
                            }
                            if ui.checkbox(&mut self.theme.cable_glow_switch, "Switch cables").changed() {
                                theme_changed = true;
                            }
                            if ui.checkbox(&mut self.theme.cable_glow_external, "External cables").changed() {
                                theme_changed = true;
                            }
                            ui.separator();
                            if ui
                                .button("Reset / Enable all bloom")
                                .on_hover_text("Turn on bloom glow for all 3 cable types")
                                .clicked()
                            {
                                self.theme.cable_glow_owned = true;
                                self.theme.cable_glow_switch = true;
                                self.theme.cable_glow_external = true;
                                theme_changed = true;
                            }
                        });

                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Cable Gradients:").strong());
                            if ui
                                .button("Disable all gradients")
                                .on_hover_text("Turn off gradients for all cable types")
                                .clicked()
                            {
                                self.theme.cable_grad_owned_enabled = false;
                                self.theme.cable_grad_switch_enabled = false;
                                self.theme.cable_grad_external_enabled = false;
                                theme_changed = true;
                            }
                        });
                        // Owned gradient
                        if ui.checkbox(&mut self.theme.cable_grad_owned_enabled, "Owned cable gradient").changed() {
                            theme_changed = true;
                        }
                        if self.theme.cable_grad_owned_enabled {
                            ui.horizontal(|ui| {
                                ui.label("Start color:");
                                if ui.color_edit_button_srgb(&mut self.theme.cable_grad_owned_start).changed() {
                                    theme_changed = true;
                                }
                                ui.label("End color:");
                                if ui.color_edit_button_srgb(&mut self.theme.cable_grad_owned_end).changed() {
                                    theme_changed = true;
                                }
                            });
                        }

                        // Switch gradient
                        if ui.checkbox(&mut self.theme.cable_grad_switch_enabled, "Switch cable gradient").changed() {
                            theme_changed = true;
                        }
                        if self.theme.cable_grad_switch_enabled {
                            ui.horizontal(|ui| {
                                ui.label("Mode:");
                                if ui.radio_value(&mut self.theme.cable_grad_switch_mode, SwitchGradMode::Simple, "Simple (2 colors)").changed() {
                                    theme_changed = true;
                                }
                                if ui.radio_value(&mut self.theme.cable_grad_switch_mode, SwitchGradMode::Advanced, "Advanced (4 colors)").changed() {
                                    theme_changed = true;
                                }
                            });
                            match self.theme.cable_grad_switch_mode {
                                SwitchGradMode::Simple => {
                                    ui.horizontal(|ui| {
                                        ui.label("Start color:");
                                        if ui.color_edit_button_srgb(&mut self.theme.cable_grad_switch_start).changed() {
                                            theme_changed = true;
                                        }
                                        ui.label("End color:");
                                        if ui.color_edit_button_srgb(&mut self.theme.cable_grad_switch_end).changed() {
                                            theme_changed = true;
                                        }
                                        ui.label(egui::RichText::new("(In -> Switch mid -> Out)").weak());
                                    });
                                }
                                SwitchGradMode::Advanced => {
                                    ui.horizontal(|ui| {
                                        ui.label("In:");
                                        if ui.color_edit_button_srgb(&mut self.theme.cable_grad_switch_start).changed() {
                                            theme_changed = true;
                                        }
                                        ui.label("Switch In:");
                                        if ui.color_edit_button_srgb(&mut self.theme.cable_grad_switch_mid_in).changed() {
                                            theme_changed = true;
                                        }
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("Switch Out:");
                                        if ui.color_edit_button_srgb(&mut self.theme.cable_grad_switch_mid_out).changed() {
                                            theme_changed = true;
                                        }
                                        ui.label("Out:");
                                        if ui.color_edit_button_srgb(&mut self.theme.cable_grad_switch_end).changed() {
                                            theme_changed = true;
                                        }
                                    });
                                }
                            }
                        }

                        // External gradient
                        if ui.checkbox(&mut self.theme.cable_grad_external_enabled, "External cable gradient").changed() {
                            theme_changed = true;
                        }
                        if self.theme.cable_grad_external_enabled {
                            ui.horizontal(|ui| {
                                ui.label("Start color:");
                                if ui.color_edit_button_srgb(&mut self.theme.cable_grad_external_start).changed() {
                                    theme_changed = true;
                                }
                                ui.label("End color:");
                                if ui.color_edit_button_srgb(&mut self.theme.cable_grad_external_end).changed() {
                                    theme_changed = true;
                                }
                            });
                        }
                    });

                    ui.separator();

                    ui.collapsing("Import / Export Theme", |ui| {
                        ui.label("Export current theme to a JSON file:");
                        ui.horizontal(|ui| {
                            ui.label("Path:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.theme_export_path)
                                    .hint_text("~/.config/RedoxRouter/themes/my_theme.json")
                                    .desired_width(260.0),
                            );
                            if ui.button("Save to file").clicked() {
                                export_to_file = true;
                            }
                            if ui.button("Copy JSON").clicked() {
                                copy_clipboard = true;
                            }
                        });

                        ui.separator();

                        ui.label("Import theme from a JSON file:");
                        ui.horizontal(|ui| {
                            ui.label("Path:");
                            ui.add(
                                egui::TextEdit::singleline(&mut self.theme_import_path)
                                    .hint_text("~/.config/RedoxRouter/themes/theme.json")
                                    .desired_width(260.0),
                            );
                            if ui.button("Load file").clicked() {
                                load_from_path = Some(storage::expand_tilde(&self.theme_import_path));
                            }
                        });

                        ui.add_space(4.0);
                        ui.label("Or paste theme JSON:");
                        egui::ScrollArea::vertical().max_height(110.0).show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut self.theme_raw_json)
                                    .font(egui::TextStyle::Monospace)
                                    .hint_text("{\n  \"name\": \"...\",\n  ...\n}")
                                    .desired_rows(4)
                                    .desired_width(f32::INFINITY),
                            );
                        });
                        if ui
                            .add_enabled(!self.theme_raw_json.trim().is_empty(), egui::Button::new("Import pasted JSON"))
                            .clicked()
                        {
                            load_from_pasted = true;
                        }
                    });

                    ui.separator();

                    egui::CollapsingHeader::new("My Themes")
                        .default_open(true)
                        .show(ui, |ui| {
                            let saved_themes = storage::list_saved_themes();
                            ui.horizontal(|ui| {
                                ui.label("Saved & imported themes in ~/.config/RedoxRouter/themes:");
                                ui.menu_button("Save to theme", |ui| {
                                    if !saved_themes.is_empty() {
                                        ui.label(egui::RichText::new("Update existing theme:").strong());
                                        for (path, res) in &saved_themes {
                                            let name = match res {
                                                Ok(t) => t.name.clone(),
                                                Err(_) => path.file_stem().and_then(|s| s.to_str()).unwrap_or("theme").to_string(),
                                            };
                                            if ui.button(format!("Update \"{}\"", name)).clicked() {
                                                save_to_theme_path = Some(path.clone());
                                                ui.close_menu();
                                            }
                                        }
                                        ui.separator();
                                    }
                                    if ui.button(format!("Save as new: \"{}\"", self.theme.name)).clicked() {
                                        save_as_new_named = Some(self.theme.name.clone());
                                        ui.close_menu();
                                    }
                                });
                            });

                            if saved_themes.is_empty() {
                                ui.weak("No themes saved yet. Click 'Save to theme' above to save the current theme or import one.");
                            } else {
                                egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                                    for (path, res) in &saved_themes {
                                        let fname = path.file_name().and_then(|s| s.to_str()).unwrap_or("theme.json");
                                        let (name, is_valid) = match res {
                                            Ok(t) => (t.name.clone(), true),
                                            Err(_) => (format!("{fname} (invalid)"), false),
                                        };
                                        let is_active = self.theme.name == name;
                                        let display_text = if is_active {
                                            format!("* {} ({})", name, fname)
                                        } else {
                                            format!("{} ({})", name, fname)
                                        };

                                        ui.horizontal(|ui| {
                                            let r = ui.selectable_label(is_active, display_text)
                                                .on_hover_text("Click to load theme. Right-click for options (Export, Delete).");
                                            if r.clicked() && is_valid {
                                                load_from_path = Some(path.clone());
                                            }
                                            r.context_menu(|ui| {
                                                if ui.button("Load theme").clicked() {
                                                    load_from_path = Some(path.clone());
                                                    ui.close_menu();
                                                }
                                                if ui.button("Export JSON to clipboard").clicked() {
                                                    if let Ok(t) = res {
                                                        export_clipboard_theme = Some(t.clone());
                                                    }
                                                    ui.close_menu();
                                                }
                                                if ui.button("Delete theme").clicked() {
                                                    delete_theme_path = Some(path.clone());
                                                    ui.close_menu();
                                                }
                                            });
                                        });
                                    }
                                });
                            }
                        });
                });

                if let Some(status) = &self.theme_status {
                    match status {
                        Ok(msg) => {
                            ui.colored_label(egui::Color32::from_rgb(110, 210, 140), msg);
                        }
                        Err(err) => {
                            ui.colored_label(egui::Color32::LIGHT_RED, err);
                        }
                    }
                }
            });

        self.show_theme = open;

        if let Some(preset) = apply_preset {
            self.theme = preset;
            if let Some(dir) = storage::themes_dir() {
                let stem = self.theme.name.to_lowercase().replace(' ', "_");
                self.theme_export_path = dir.join(format!("{stem}.json")).display().to_string();
            }
            theme_changed = true;
            self.theme_status = Some(Ok(format!("Switched to preset \"{}\"", self.theme.name)));
        }

        if let Some(path) = save_to_theme_path {
            match storage::export_theme_to_path(&path, &self.theme) {
                Ok(()) => {
                    self.theme_status = Some(Ok(format!("Updated theme at {}", path.display())));
                }
                Err(e) => {
                    self.theme_status = Some(Err(format!("Failed to update theme: {e}")));
                }
            }
        }

        if let Some(name) = save_as_new_named {
            match storage::save_named_theme(&name, &self.theme) {
                Ok(path) => {
                    self.theme_status = Some(Ok(format!("Saved theme \"{}\" to {}", name, path.display())));
                }
                Err(e) => {
                    self.theme_status = Some(Err(format!("Failed to save theme: {e}")));
                }
            }
        }

        if let Some(path) = delete_theme_path {
            match storage::delete_theme_file(&path) {
                Ok(()) => {
                    self.theme_status = Some(Ok(format!("Deleted theme file {}", path.display())));
                }
                Err(e) => {
                    self.theme_status = Some(Err(format!("Failed to delete theme: {e}")));
                }
            }
        }

        if let Some(theme) = export_clipboard_theme {
            match storage::export_theme_json(&theme) {
                Ok(json) => {
                    ctx.copy_text(json);
                    self.theme_status = Some(Ok(format!("Theme \"{}\" copied to clipboard", theme.name)));
                }
                Err(e) => {
                    self.theme_status = Some(Err(format!("Failed to export theme: {e}")));
                }
            }
        }

        if export_to_file {
            let path = storage::expand_tilde(&self.theme_export_path);
            let res = storage::export_theme_to_path(&path, &self.theme)
                .map(|()| format!("Saved theme to {}", path.display()));
            self.theme_status = Some(res);
        }

        if copy_clipboard {
            match storage::export_theme_json(&self.theme) {
                Ok(json) => {
                    ctx.copy_text(json);
                    self.theme_status = Some(Ok("Theme JSON copied to clipboard".to_string()));
                }
                Err(e) => {
                    self.theme_status = Some(Err(format!("Failed to serialize theme: {e}")));
                }
            }
        }

        if let Some(path) = load_from_path {
            match storage::import_theme_from_path(&path) {
                Ok(imported) => {
                    // Also ensure imported theme is saved into themes/ if it came from outside
                    if let Some(dir) = storage::themes_dir() {
                        if !path.starts_with(&dir) {
                            let _ = storage::save_named_theme(&imported.name, &imported);
                        }
                    }
                    self.theme = imported;
                    theme_changed = true;
                    self.theme_status = Some(Ok(format!("Loaded theme \"{}\" from {}", self.theme.name, path.display())));
                }
                Err(e) => {
                    self.theme_status = Some(Err(format!("Failed to load theme: {e}")));
                }
            }
        }

        if load_from_pasted {
            match storage::parse_imported_theme(&self.theme_raw_json) {
                Ok(imported) => {
                    // Save pasted theme into themes/ so it appears in My Themes
                    let _ = storage::save_named_theme(&imported.name, &imported);
                    self.theme = imported;
                    theme_changed = true;
                    self.theme_status = Some(Ok(format!("Imported and saved theme \"{}\"", self.theme.name)));
                }
                Err(e) => {
                    self.theme_status = Some(Err(format!("Invalid theme JSON: {e}")));
                }
            }
        }

        if theme_changed {
            self.theme.apply(ctx);
            self.canvas.theme = self.theme.clone();
            let _ = storage::save_theme(&self.theme);
        }
    }

    fn prompt_window(&mut self, ctx: &egui::Context) {
        let Some((out_port, in_port)) = self.prompt else { return };
        let mut close = false;
        let mut take = false;
        egui::Window::new("Already connected")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("These ports are already connected by another program.");
                ui.horizontal(|ui| {
                    if ui.button("Show as visible only").clicked() {
                        self.canvas.revealed.insert((out_port, in_port));
                        close = true;
                    }
                    if ui.button("Make Redox connection").clicked() {
                        take = true;
                        close = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
        if take {
            self.take_over(out_port, in_port);
        }
        if close {
            self.prompt = None;
        }
    }
}

/// Something the user asked for in the Rules window, applied after drawing.
enum Op {
    AddGroup,
    Remove(usize),
    ToLibrary(usize),
    Unlink(usize),
    Up(usize),
    Down(usize),
    UseLib(String),
    DeleteLib(String),
}

/// Up / Down / (library action) / Remove buttons of a group header.
fn group_buttons(ui: &mut egui::Ui, gi: usize, link_label: &str, link_op: Op, op: &mut Option<Op>) {
    if ui.small_button("Up").clicked() {
        *op = Some(Op::Up(gi));
    }
    if ui.small_button("Down").clicked() {
        *op = Some(Op::Down(gi));
    }
    if ui.button(link_label).clicked() {
        *op = Some(link_op);
    }
    if ui.button("Remove group").clicked() {
        *op = Some(Op::Remove(gi));
    }
}

struct RuleCtx<'a> {
    nodes: &'a BTreeMap<u32, NodeInfo>,
    nicks: &'a BTreeMap<String, String>,
    classes: &'a [String],
}

/// Editable list of rules plus an "Add rule" button; true if anything changed.
fn rules_ui(ui: &mut egui::Ui, salt: usize, rules: &mut Vec<Rule>, cx: &RuleCtx) -> bool {
    let mut changed = false;
    let mut remove = None;
    for (i, rule) in rules.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            changed |= ui.checkbox(&mut rule.enabled, "").changed();
            egui::ComboBox::from_id_salt(("rule_field", salt, i))
                .width(130.0)
                .selected_text(rule.field.label())
                .show_ui(ui, |ui| {
                    for f in Field::ALL {
                        changed |= ui.selectable_value(&mut rule.field, f, f.label()).changed();
                    }
                });
            egui::ComboBox::from_id_salt(("rule_mode", salt, i))
                .width(130.0)
                .selected_text(rule.mode.label())
                .show_ui(ui, |ui| {
                    for m in Mode::ALL {
                        changed |= ui.selectable_value(&mut rule.mode, m, m.label()).changed();
                    }
                });
            if rule.mode == Mode::Empty {
                ui.weak("(blank)");
            } else if rule.field == Field::MediaClass {
                // Dropdown of the media classes currently present.
                let mut opts = cx.classes.to_vec();
                if !rule.pattern.is_empty() && !opts.contains(&rule.pattern) {
                    opts.push(rule.pattern.clone());
                }
                let shown = if rule.pattern.is_empty() {
                    "choose media class".to_string()
                } else {
                    rule.pattern.clone()
                };
                egui::ComboBox::from_id_salt(("rule_class", salt, i))
                    .width(170.0)
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        for c in &opts {
                            changed |= ui.selectable_value(&mut rule.pattern, c.clone(), c).changed();
                        }
                    });
            } else {
                changed |= ui
                    .add(
                        egui::TextEdit::singleline(&mut rule.pattern)
                            .hint_text("text or pattern")
                            .desired_width(170.0),
                    )
                    .changed();
            }
            let action_text = match rule.action {
                RuleAction::Hide => "Hide",
                RuleAction::Show => "Show",
                RuleAction::Color(_) => "Color",
            };
            egui::ComboBox::from_id_salt(("rule_action", salt, i))
                .width(75.0)
                .selected_text(action_text)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(matches!(rule.action, RuleAction::Hide), "Hide").clicked() {
                        rule.action = RuleAction::Hide;
                        changed = true;
                    }
                    if ui.selectable_label(matches!(rule.action, RuleAction::Show), "Show").clicked() {
                        rule.action = RuleAction::Show;
                        changed = true;
                    }
                    if ui.selectable_label(matches!(rule.action, RuleAction::Color(_)), "Color").clicked() {
                        if !matches!(rule.action, RuleAction::Color(_)) {
                            rule.action = RuleAction::Color([90, 90, 140]);
                            changed = true;
                        }
                    }
                });
            if let RuleAction::Color(c) = &mut rule.action {
                changed |= ui.color_edit_button_srgb(c).changed();
            }
            let n = cx
                .nodes
                .values()
                .filter(|n| {
                    rule.matches(&NodeText {
                        title: n.label(),
                        nickname: cx
                            .nicks
                            .get(&n.key())
                            .or_else(|| cx.nicks.get(&n.name))
                            .map_or("", |s| s.as_str()),
                        name: &n.name,
                        description: &n.description,
                        media_class: &n.media_class,
                    })
                })
                .count();
            ui.weak(format!("{n} nodes"));
            if ui.button("Remove").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        rules.remove(i);
        changed = true;
    }
    if ui.button("Add rule").clicked() {
        rules.push(Rule::new());
        changed = true;
    }
    changed
}

/// A smooth animated pill toggle switch matching the modern mockup.
fn toggle_switch(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let desired_size = egui::vec2(32.0, 18.0);
    let (rect, mut response) = ui.allocate_exact_size(desired_size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Checkbox, ui.is_enabled(), *on, ""));

    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool_responsive(response.id, *on);
        let radius = 0.5 * rect.height();
        let bg_off = if ui.visuals().dark_mode {
            egui::Color32::from_rgb(52, 54, 65)
        } else {
            egui::Color32::from_rgb(205, 210, 220)
        };
        let bg_on = ui.visuals().selection.stroke.color;
        let lerp_f = |a: f32, b: f32, t: f32| a + (b - a) * t;
        let bg_col = egui::Color32::from_rgb(
            lerp_f(bg_off.r() as f32, bg_on.r() as f32, how_on) as u8,
            lerp_f(bg_off.g() as f32, bg_on.g() as f32, how_on) as u8,
            lerp_f(bg_off.b() as f32, bg_on.b() as f32, how_on) as u8,
        );
        ui.painter().rect(
            rect,
            radius,
            bg_col,
            egui::Stroke::NONE,
            egui::StrokeKind::Inside,
        );
        let circle_x = lerp_f(rect.left() + radius, rect.right() - radius, how_on);
        let center = egui::pos2(circle_x, rect.center().y);
        let knob_col = if *on {
            egui::Color32::WHITE
        } else if ui.visuals().dark_mode {
            egui::Color32::from_gray(190)
        } else {
            egui::Color32::WHITE
        };
        ui.painter().circle(
            center,
            radius - 2.5,
            knob_col,
            egui::Stroke::new(0.5_f32, egui::Color32::from_black_alpha(40)),
        );
    }
    response
}

impl RedoxRouter {
    fn render_top_left(&mut self, ui: &mut egui::Ui) {
        // Brand title (icon removed as requested)
        ui.label(
            egui::RichText::new("RedoxRouter")
                .strong()
                .size(15.0)
                .color(self.theme.c_accent()),
        )
        .on_hover_text(format!(
            "RedoxRouter: {} nodes, {} ports, {} links ({} owned)",
            self.graph.nodes.len(),
            self.graph.ports.len(),
            self.graph.links.len(),
            self.graph.owned_ids.len()
        ));

        ui.add_space(4.0);

        // Capsule search bar
        let search_frame = egui::Frame::NONE
            .fill(self.theme.c_extreme_bg())
            .corner_radius(egui::CornerRadius::same(14))
            .stroke(egui::Stroke::new(
                1.0_f32,
                if self.search_focus || !self.search_query.is_empty() {
                    self.theme.c_accent()
                } else if self.theme.is_dark {
                    egui::Color32::from_gray(55)
                } else {
                    egui::Color32::from_gray(190)
                },
            ))
            .inner_margin(egui::Margin::symmetric(10, 4));

        let mut search_resp = None;
        search_frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.search_query)
                        .hint_text("Search node (Ctrl+F)…")
                        .desired_width(160.0)
                        .frame(false),
                );
                search_resp = Some(resp);
                if !self.search_query.is_empty() {
                    if ui.small_button("x").on_hover_text("Clear search").clicked() {
                        self.search_query.clear();
                    }
                }
            });
        });

        if self.search_focus {
            if let Some(resp) = search_resp.as_ref() {
                resp.request_focus();
            }
            self.search_focus = false;
        }

        if !self.search_query.trim().is_empty() {
            let q = self.search_query.trim().to_lowercase();
            let matches: Vec<(u32, String)> = self
                .graph
                .nodes
                .values()
                .filter(|n| {
                    let nick = self
                        .canvas
                        .nicknames
                        .get(&n.key())
                        .or_else(|| self.canvas.nicknames.get(&n.name))
                        .map_or("", |s| s.as_str());
                    n.label().to_lowercase().contains(&q)
                        || n.name.to_lowercase().contains(&q)
                        || nick.to_lowercase().contains(&q)
                })
                .take(5)
                .map(|n| (n.id, format!("{} (#{})", n.label(), n.id)))
                .collect();

            if search_resp.as_ref().is_some_and(|r| r.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                if let Some((id, _)) = matches.first() {
                    self.canvas.focus_node(*id);
                    self.search_query.clear();
                }
            } else {
                for (id, label) in matches {
                    if ui.small_button(format!("Jump: {label}")).clicked() {
                        self.canvas.focus_node(id);
                        self.search_query.clear();
                    }
                }
            }
        }

        ui.add_space(2.0);

        // Capsule navigation group: [ 🔍− | 🔍+ | ⛶ | 🎯 ] with larger buttons and vertical dividers
        let nav_frame = egui::Frame::NONE
            .fill(if self.theme.is_dark { egui::Color32::from_rgb(34, 36, 44) } else { egui::Color32::from_rgb(222, 226, 232) })
            .corner_radius(egui::CornerRadius::same(14))
            .stroke(egui::Stroke::new(
                1.0_f32,
                if self.theme.is_dark { egui::Color32::from_gray(54) } else { egui::Color32::from_gray(190) },
            ))
            .inner_margin(egui::Margin::symmetric(8, 3));

        nav_frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(2.0, 0.0);
                let div_color = if self.theme.is_dark {
                    egui::Color32::from_gray(60)
                } else {
                    egui::Color32::from_gray(185)
                };
                let vdiv = |ui: &mut egui::Ui| {
                    ui.add_space(2.0);
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 16.0), egui::Sense::hover());
                    ui.painter().vline(rect.center().x, rect.y_range(), egui::Stroke::new(1.0_f32, div_color));
                    ui.add_space(2.0);
                };

                if ui.add(egui::Button::new(egui::RichText::new("−").size(15.0).strong()).frame(false))
                    .on_hover_text("Zoom out")
                    .clicked()
                {
                    self.canvas.zoom_out();
                }
                vdiv(ui);
                if ui.add(egui::Button::new(egui::RichText::new("+").size(15.0).strong()).frame(false))
                    .on_hover_text("Zoom in")
                    .clicked()
                {
                    self.canvas.zoom_in();
                }
                vdiv(ui);
                if ui.add(egui::Button::new(egui::RichText::new("Fit").size(12.5).strong()).frame(false))
                    .on_hover_text("Fit view: fit all nodes into view (F or Home)")
                    .clicked()
                {
                    self.canvas.fit_to_view();
                }
                vdiv(ui);
                if ui.add(egui::Button::new(egui::RichText::new("100%").size(11.5)).frame(false))
                    .on_hover_text("Reset view: center origin at 100% zoom")
                    .clicked()
                {
                    self.canvas.reset_view();
                }
            });
        });
    }

    fn render_top_middle(&mut self, ui: &mut egui::Ui) {
        // Snap toggle
        ui.horizontal(|ui| {
            ui.label("Snap");
            if toggle_switch(ui, &mut self.canvas.snap_to_grid)
                .on_hover_text("Snap dragged nodes, blocks, comments, waypoints, and switches to the grid")
                .changed()
            {
                self.canvas.dirty = true;
            }
        });

        ui.add_space(4.0);

        // Minimap pill
        let map_on = self.canvas.show_minimap;
        let map_btn = if map_on {
            ui.add(
                egui::Button::new(egui::RichText::new("Minimap").strong().color(egui::Color32::WHITE))
                    .fill(self.theme.c_accent())
                    .corner_radius(egui::CornerRadius::same(12)),
            )
        } else {
            ui.add(
                egui::Button::new("Minimap")
                    .corner_radius(egui::CornerRadius::same(12)),
            )
        };
        if map_btn.on_hover_text("Toggle interactive minimap in bottom-left corner").clicked() {
            self.canvas.show_minimap = !self.canvas.show_minimap;
            self.canvas.dirty = true;
        }

        ui.add_space(4.0);

        // Cable style combobox
        let cable_text = self.canvas.link_style.label();
        egui::ComboBox::from_id_salt("link_style")
            .selected_text(cable_text)
            .show_ui(ui, |ui| {
                for style in layout::LinkStyle::ALL {
                    if ui.selectable_value(&mut self.canvas.link_style, style, style.label()).changed() {
                        self.canvas.dirty = true;
                    }
                }
            });
    }

    fn render_top_right(&mut self, ui: &mut egui::Ui) {
        let sysinfo_btn = if self.show_sysinfo {
            ui.add(egui::Button::new(egui::RichText::new("SysInfo").strong()).fill(self.theme.c32_alpha(self.theme.accent, 55)))
        } else {
            ui.button("SysInfo")
        };
        if sysinfo_btn
            .on_hover_text("System information & full PipeWire/RedoxRouter diagnostic report for issue reporting")
            .clicked()
        {
            self.show_sysinfo = !self.show_sysinfo;
        }

        let debug_btn = if self.show_debug {
            ui.add(egui::Button::new(egui::RichText::new("Debug").strong()).fill(self.theme.c32_alpha(self.theme.accent, 55)))
        } else {
            ui.button("Debug")
        };
        if debug_btn.on_hover_text("Toggle node/port debug table").clicked() {
            self.show_debug = !self.show_debug;
        }

        let links_label = format!("Links ({})", self.canvas.links.len());
        let links_btn = if self.show_links {
            ui.add(egui::Button::new(egui::RichText::new(&links_label).strong()).fill(self.theme.c32_alpha(self.theme.accent, 55)))
        } else {
            ui.button(&links_label)
        };
        if links_btn.on_hover_text("Saved links window").clicked() {
            self.show_links = !self.show_links;
        }

        let rules_btn = if self.show_rules {
            ui.add(egui::Button::new(egui::RichText::new("Rules").strong()).fill(self.theme.c32_alpha(self.theme.accent, 55)))
        } else {
            ui.button("Rules")
        };
        if rules_btn.on_hover_text("Filter rules window").clicked() {
            self.show_rules = !self.show_rules;
        }

        let theme_btn = if self.show_theme {
            ui.add(egui::Button::new(egui::RichText::new("Theme").strong()).fill(self.theme.c32_alpha(self.theme.accent, 55)))
        } else {
            ui.button("Theme")
        };
        if theme_btn.on_hover_text("Live theme editor").clicked() {
            self.show_theme = !self.show_theme;
        }

        // Selected external link takeover button
        let external = self.canvas.selected().filter(|&(o, i)| {
            self.graph.find_link(o, i).is_some_and(|l| !self.graph.is_owned(l))
        });
        if let Some((o, i)) = external {
            if ui.button("Make Redox connection").clicked() {
                self.take_over(o, i);
            }
        }

        // Error indicator: styled warning capsule with clean "x" dismiss button
        let mut clear_err = false;
        if let Some(e) = &self.graph.error {
            let err_frame = egui::Frame::NONE
                .fill(if self.theme.is_dark { egui::Color32::from_rgb(60, 24, 28) } else { egui::Color32::from_rgb(255, 232, 232) })
                .corner_radius(egui::CornerRadius::same(12))
                .stroke(egui::Stroke::new(
                    1.0_f32,
                    if self.theme.is_dark { egui::Color32::from_rgb(180, 60, 60) } else { egui::Color32::from_rgb(220, 100, 100) },
                ))
                .inner_margin(egui::Margin::symmetric(8, 3));
            err_frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(6.0, 0.0);
                    let text_col = if self.theme.is_dark { egui::Color32::from_rgb(255, 140, 140) } else { egui::Color32::from_rgb(170, 30, 30) };
                    ui.colored_label(text_col, e);
                    if ui
                        .small_button("Report…")
                        .on_hover_text("Open System Info & Issue Report to inspect or copy full state")
                        .clicked()
                    {
                        self.show_sysinfo = true;
                    }
                    if ui.add(
                        egui::Button::new(egui::RichText::new("x").strong().size(13.0).color(text_col))
                            .frame(false)
                    ).on_hover_text("Dismiss error").clicked() {
                        clear_err = true;
                    }
                });
            });
        }
        if clear_err {
            self.graph.error = None;
        }
        if let Some(e) = &self.save_error {
            ui.colored_label(egui::Color32::LIGHT_RED, format!("layout not saved: {e}"));
        }
    }
}

impl eframe::App for RedoxRouter {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();
        self.drain_cli();
        self.canvas.virtual_mode = self.viewing != self.setups.active;
        self.auto_connect();
        self.enforce_bleed_prevention();

        if ctx.input(|i| i.modifiers.ctrl && i.key_pressed(egui::Key::F)) {
            self.search_focus = true;
        }

        egui::TopBottomPanel::top("top").frame(bar_frame(&self.theme)).show(ctx, |ui| {
            let total_width = ui.available_width();
            if total_width >= 1060.0 {
                let mut fit_on_one_line = true;
                ui.horizontal(|ui| {
                    self.render_top_left(ui);
                    ui.add_space(6.0);
                    self.render_top_middle(ui);
                    if ui.available_width() >= 340.0 {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.render_top_right(ui);
                        });
                    } else {
                        fit_on_one_line = false;
                    }
                });
                if !fit_on_one_line {
                    ui.add_space(2.0);
                    ui.horizontal_wrapped(|ui| {
                        self.render_top_right(ui);
                    });
                }
            } else {
                ui.horizontal_wrapped(|ui| {
                    self.render_top_left(ui);
                });
                ui.add_space(3.0);
                ui.horizontal(|ui| {
                    self.render_top_middle(ui);
                    if ui.available_width() >= 340.0 {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            self.render_top_right(ui);
                        });
                    } else {
                        ui.horizontal_wrapped(|ui| {
                            self.render_top_right(ui);
                        });
                    }
                });
            }
        });

        self.tabs_bar(ctx);

        if self.show_debug {
            egui::SidePanel::right("debug").show(ctx, |ui| self.debug_panel(ui));
        }

        let mut action = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| action = self.canvas.show(ui, &self.graph));
        if let Some(a) = action {
            self.handle(a);
        }
        for (out_port, in_port) in std::mem::take(&mut self.canvas.extra_connects) {
            self.handle(Action::Connect { out_port, in_port });
        }
        for (out_port, in_port) in std::mem::take(&mut self.canvas.extra_disconnects) {
            self.handle(Action::Disconnect { out_port, in_port });
        }

        self.prompt_window(ctx);
        self.rules_window(ctx);
        self.links_window(ctx);
        self.theme_window(ctx);
        self.sysinfo_window(ctx);
        self.rename_window(ctx);
        self.inactive_window(ctx);
        self.reset_window(ctx);
        self.create_sink_window(ctx);
        self.export_window(ctx);
        self.import_window(ctx);
        self.maybe_save(ctx);
    }
}

/// Global look: apply active theme to egui context.
fn apply_theme(ctx: &egui::Context, theme: &Theme) {
    theme.apply(ctx);
}

fn bar_frame(theme: &Theme) -> egui::Frame {
    theme.bar_frame()
}

fn main() -> eframe::Result {
    match cli::parse_args(std::env::args().skip(1)) {
        cli::ParsedArgs::Help => {
            println!("{}", cli::usage());
            return Ok(());
        }
        cli::ParsedArgs::Error(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
        cli::ParsedArgs::Request(req) => match cli::send_request(&req) {
            Ok(resp) => {
                if resp.ok {
                    println!("{}", resp.message);
                    return Ok(());
                } else {
                    eprintln!("{}", resp.message);
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        },
        cli::ParsedArgs::RunGui => {}
    }

    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "RedoxRouter",
        options,
        Box::new(|cc| {
            let (tx, rx) = mpsc::channel();
            let theme = storage::load_theme();
            apply_theme(&cc.egui_ctx, &theme);
            let cmd = pw_backend::spawn(tx, cc.egui_ctx.clone());
            let cli_rx = cli::spawn_listener(cc.egui_ctx.clone());
            Ok(Box::new(RedoxRouter::new(rx, cli_rx, cmd)))
        }),
    )
}

