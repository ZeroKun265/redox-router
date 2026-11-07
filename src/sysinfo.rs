//! System information and full diagnostic dump for issue reporting.
//! Captures OS/environment info, RedoxRouter state, all setups/saved links/rules,
//! the complete PipeWire graph (nodes, ports, links, volumes, routes, bleed state),
//! and a chronological ring buffer of recent PipeWire events and commands.

use crate::graph::{Command, Direction, Event};
use crate::links;
use crate::rules::GroupEntry;
use crate::storage;
use crate::RedoxRouter;
use eframe::egui;
use std::fmt::Write as _;

const MAX_EVENT_LOG: usize = 300;

fn read_os_pretty_name() -> String {
    if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
        for line in content.lines() {
            if let Some(val) = line.strip_prefix("PRETTY_NAME=") {
                return val.trim_matches('"').to_string();
            }
        }
    }
    std::env::consts::OS.to_string()
}

fn read_kernel_release() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

fn env_or(key: &str, fallback: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| fallback.to_string())
}

impl RedoxRouter {
    pub(crate) fn log_trace(&self, msg: impl Into<String>) {
        let elapsed = self.started.elapsed().as_secs_f64();
        let line = format!("[+{elapsed:8.3}s] {}", msg.into());
        let mut log = self.event_log.borrow_mut();
        if log.len() >= MAX_EVENT_LOG {
            log.pop_front();
        }
        log.push_back(line);
    }

    pub(crate) fn log_pw_event(&self, ev: &Event) {
        match ev {
            Event::NodeAdded(n) => {
                self.log_trace(format!(
                    "EVENT NodeAdded #{} name={:?} desc={:?} class={:?} dev={:?} prof={:?}",
                    n.id, n.name, n.description, n.media_class, n.device_id, n.profile_device
                ));
            }
            Event::PortAdded(p) => {
                let node_name = self
                    .graph
                    .nodes
                    .get(&p.node_id)
                    .map_or("?", |n| n.name.as_str());
                self.log_trace(format!(
                    "EVENT PortAdded #{} on node #{} ({}) dir={:?} name={:?}",
                    p.id, p.node_id, node_name, p.direction, p.name
                ));
            }
            Event::LinkAdded(l) => {
                let pair = (l.out_port, l.in_port);
                let out_n = self.graph.nodes.get(&l.out_node).map_or("?", |n| n.name.as_str());
                let out_p = self.graph.ports.get(&l.out_port).map_or("?", |p| p.name.as_str());
                let in_n = self.graph.nodes.get(&l.in_node).map_or("?", |n| n.name.as_str());
                let in_p = self.graph.ports.get(&l.in_port).map_or("?", |p| p.name.as_str());
                self.log_trace(format!(
                    "EVENT LinkAdded #{} #{}({}):#{}({}) -> #{}({}):#{}({}) [req={} owned_pair={}]",
                    l.id,
                    l.out_node,
                    out_n,
                    l.out_port,
                    out_p,
                    l.in_node,
                    in_n,
                    l.in_port,
                    in_p,
                    self.requested.contains(&pair),
                    self.graph.owned_pairs.contains(&pair)
                ));
            }
            Event::Removed(id) => {
                if let Some(n) = self.graph.nodes.get(id) {
                    self.log_trace(format!(
                        "EVENT Removed Node #{} name={:?} class={:?}",
                        id, n.name, n.media_class
                    ));
                } else if let Some(p) = self.graph.ports.get(id) {
                    self.log_trace(format!(
                        "EVENT Removed Port #{} (node #{}, {:?}, name={:?})",
                        id, p.node_id, p.direction, p.name
                    ));
                } else if let Some(l) = self.graph.links.get(id) {
                    self.log_trace(format!(
                        "EVENT Removed Link #{} (#{}:#{} -> #{}:#{}, owned={})",
                        id,
                        l.out_node,
                        l.out_port,
                        l.in_node,
                        l.in_port,
                        self.graph.is_owned(l)
                    ));
                }
            }
            Event::OwnedNodeAdded(id) => {
                self.log_trace(format!("EVENT OwnedNodeAdded #{id}"));
            }
            Event::OwnedAdded { out_port, in_port } => {
                self.log_trace(format!("EVENT OwnedAdded ({out_port} -> {in_port})"));
            }
            Event::OwnedRemoved { out_port, in_port } => {
                self.log_trace(format!("EVENT OwnedRemoved ({out_port} -> {in_port})"));
            }
            Event::NodeStateChanged { node_id, is_running } => {
                let node_name = self
                    .graph
                    .nodes
                    .get(node_id)
                    .map_or("?", |n| n.name.as_str());
                self.log_trace(format!(
                    "EVENT NodeStateChanged #{} ({}) running={}",
                    node_id, node_name, is_running
                ));
            }
            Event::Error(e) => {
                self.log_trace(format!("EVENT Error: {e}"));
            }
            Event::Volume { .. } | Event::RouteVolume { .. } => {}
        }
    }

    pub(crate) fn log_pw_command(&self, cmd: &Command) {
        match cmd {
            Command::CreateLink {
                out_node,
                out_port,
                in_node,
                in_port,
                lingering,
            } => {
                self.log_trace(format!(
                    "CMD CreateLink #{out_node}:#{out_port} -> #{in_node}:#{in_port} linger={lingering}"
                ));
            }
            Command::DestroyLink {
                link_id,
                out_port,
                in_port,
            } => {
                self.log_trace(format!(
                    "CMD DestroyLink link_id={link_id:?} ({out_port} -> {in_port})"
                ));
            }
            Command::TakeOver {
                link_id,
                out_node,
                out_port,
                in_node,
                in_port,
                lingering,
            } => {
                self.log_trace(format!(
                    "CMD TakeOver link_id=#{link_id} #{out_node}:#{out_port} -> #{in_node}:#{in_port} linger={lingering}"
                ));
            }
            Command::CreateSink { name, description } => {
                self.log_trace(format!(
                    "CMD CreateSink name={name:?} desc={description:?}"
                ));
            }
            Command::CreateSource { name, description } => {
                self.log_trace(format!(
                    "CMD CreateSource name={name:?} desc={description:?}"
                ));
            }
            Command::DestroySink { node_id } => {
                self.log_trace(format!("CMD DestroySink node_id=#{node_id}"));
            }
            Command::SetMute { node_id, mute } => {
                self.log_trace(format!("CMD SetMute node_id=#{node_id} mute={mute}"));
            }
            Command::SetVolume { .. } => {}
        }
    }

    /// Generate a comprehensive, human- and machine-readable diagnostic report
    /// covering system info, RedoxRouter state, setups, rules, PipeWire graph, and event log.
    pub(crate) fn generate_system_report(&self) -> String {
        let mut out = String::with_capacity(16384);
        let uptime = self.started.elapsed().as_secs_f64();

        let _ = writeln!(out, "================================================================================");
        let _ = writeln!(out, "REDOX ROUTER — SYSTEM & PIPEWIRE DIAGNOSTIC REPORT");
        let _ = writeln!(out, "================================================================================");
        let _ = writeln!(out);

        // 1. System & Environment
        let _ = writeln!(out, "=== 1. SYSTEM & ENVIRONMENT ===");
        let _ = writeln!(out, "RedoxRouter Version : {}", env!("CARGO_PKG_VERSION"));
        let _ = writeln!(out, "Process Uptime      : {uptime:.3}s");
        let _ = writeln!(out, "OS / Distribution   : {}", read_os_pretty_name());
        let _ = writeln!(out, "Kernel Release      : {}", read_kernel_release());
        let _ = writeln!(out, "Architecture        : {} ({})", std::env::consts::ARCH, std::env::consts::OS);
        let _ = writeln!(
            out,
            "Desktop / Session   : XDG_CURRENT_DESKTOP={:?}, XDG_SESSION_TYPE={:?}, WAYLAND_DISPLAY={:?}, DISPLAY={:?}",
            env_or("XDG_CURRENT_DESKTOP", ""),
            env_or("XDG_SESSION_TYPE", ""),
            env_or("WAYLAND_DISPLAY", ""),
            env_or("DISPLAY", "")
        );
        let _ = writeln!(out, "REDOX_DEBUG Env     : {:?}", env_or("REDOX_DEBUG", "<unset>"));
        let _ = writeln!(
            out,
            "Config Directory    : {}",
            storage::config_dir_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "<unavailable>".to_string())
        );
        let _ = writeln!(out);

        // 2. RedoxRouter Runtime Status
        let active_name = self
            .setups
            .items
            .get(self.setups.active)
            .map_or("?", |s| s.name.as_str());
        let viewing_name = self
            .setups
            .items
            .get(self.viewing)
            .map_or("?", |s| s.name.as_str());

        let _ = writeln!(out, "=== 2. REDOX ROUTER STATUS ===");
        let _ = writeln!(out, "Active Setup        : #{} {:?}", self.setups.active, active_name);
        let _ = writeln!(
            out,
            "Viewed Setup        : #{} {:?} (virtual_mode = {})",
            self.viewing,
            viewing_name,
            self.viewing != self.setups.active
        );
        let _ = writeln!(out, "Current PW Error    : {:?}", self.graph.error);
        let _ = writeln!(out, "Last Save Error     : {:?}", self.save_error);
        let _ = writeln!(
            out,
            "Active Theme        : {:?} (dark={}, grid={:?}, ui_font_delta={:+.1}, node_font_delta={:+.1})",
            self.theme.name,
            self.theme.is_dark,
            self.theme.grid_pattern,
            self.theme.ui_font_size_delta,
            self.theme.node_font_size_delta
        );
        let _ = writeln!(
            out,
            "Theme Link Effects  : thickness=({:.1}, {:.1}, {:.1}) | bloom=({}, {}, {}) | grad=({}, {}, {} mode={:?})",
            self.theme.cable_thickness_owned,
            self.theme.cable_thickness_switch,
            self.theme.cable_thickness_external,
            self.theme.cable_glow_owned,
            self.theme.cable_glow_switch,
            self.theme.cable_glow_external,
            self.theme.cable_grad_owned_enabled,
            self.theme.cable_grad_switch_enabled,
            self.theme.cable_grad_external_enabled,
            self.theme.cable_grad_switch_mode
        );
        let _ = writeln!(
            out,
            "Viewed Canvas Opts  : bleed={:?} ({}) | sinks={:?} ({}) | show_external={} | link_style={:?} | snap={} | minimap={}",
            self.canvas.auto_connect_prevention,
            self.canvas.auto_connect_prevention.label(),
            self.canvas.sink_hide_mode,
            self.canvas.sink_hide_mode.label(),
            self.canvas.show_external,
            self.canvas.link_style,
            self.canvas.snap_to_grid,
            self.canvas.show_minimap
        );
        let mut owned_ids: Vec<u32> = self.graph.owned_ids.iter().copied().collect();
        owned_ids.sort_unstable();
        let mut owned_pairs: Vec<(u32, u32)> = self.graph.owned_pairs.iter().copied().collect();
        owned_pairs.sort_unstable();
        let mut owned_nodes: Vec<u32> = self.graph.owned_node_ids.iter().copied().collect();
        owned_nodes.sort_unstable();
        let mut running_nodes: Vec<u32> = self.graph.running_nodes.iter().copied().collect();
        running_nodes.sort_unstable();
        let mut requested: Vec<(u32, u32)> = self.requested.iter().copied().collect();
        requested.sort_unstable();
        let mut severed: Vec<u32> = self.severed_links.iter().copied().collect();
        severed.sort_unstable();

        let _ = writeln!(out, "graph.owned_ids     : ({}) {:?}", owned_ids.len(), owned_ids);
        let _ = writeln!(out, "graph.owned_pairs   : ({}) {:?}", owned_pairs.len(), owned_pairs);
        let _ = writeln!(out, "graph.owned_node_ids: ({}) {:?}", owned_nodes.len(), owned_nodes);
        let _ = writeln!(out, "graph.running_nodes : ({}) {:?}", running_nodes.len(), running_nodes);
        let _ = writeln!(out, "requested_pairs     : ({}) {:?}", requested.len(), requested);
        let _ = writeln!(out, "severed_links       : ({}) {:?}", severed.len(), severed);
        let _ = writeln!(out, "bleed_pending_count : {}", self.bleed_pending.len());
        let _ = writeln!(out);

        // 3. PipeWire Nodes & Ports
        let _ = writeln!(
            out,
            "=== 3. PIPEWIRE NODES & PORTS ({} nodes, {} ports) ===",
            self.graph.nodes.len(),
            self.graph.ports.len()
        );
        for n in self.graph.nodes.values() {
            let key = n.key();
            let is_owned = self.graph.is_node_owned(n);
            let is_running = self.graph.running_nodes.contains(&n.id);
            let is_active = self.graph.is_node_active(n.id);
            let in_saved = self.is_node_in_saved_links(n);
            let nick = self
                .canvas
                .nicknames
                .get(&key)
                .or_else(|| self.canvas.nicknames.get(&n.name));
            let manually_hidden =
                self.canvas.hidden.contains(&key) || self.canvas.hidden.contains(&n.name);
            let pos = self.canvas.pos_of(n.id);
            let size = self.canvas.size_of(n.id);

            let _ = writeln!(
                out,
                "Node #{:<4} label={:?} | name={:?} | desc={:?} | class={:?} | type={:?} | key={:?}",
                n.id,
                n.label(),
                n.name,
                n.description,
                n.media_class,
                n.media_type(),
                key
            );
            let _ = writeln!(
                out,
                "  Flags  : owned={} (prop={}, set={}) | running={} | active_glow={} | in_active_saved_links={} | dev_id={:?} | prof_dev={:?}",
                is_owned,
                n.is_owned,
                self.graph.owned_node_ids.contains(&n.id),
                is_running,
                is_active,
                in_saved,
                n.device_id,
                n.profile_device
            );
            let _ = writeln!(
                out,
                "  Canvas : pos={:?} | size={:?} | nickname={:?} | manually_hidden={}",
                pos, size, nick, manually_hidden
            );
            let node_vol = self.graph.volumes.get(&n.id).map(|v| {
                format!(
                    "master={:.3} ch={:?} mute={} eff={:.3}",
                    v.master,
                    v.channels,
                    v.mute,
                    v.effective()
                )
            });
            let route_vol = n
                .device_id
                .zip(n.profile_device)
                .and_then(|k| self.graph.routes.get(&k))
                .map(|v| {
                    format!(
                        "master={:.3} ch={:?} mute={} eff={:.3}",
                        v.master,
                        v.channels,
                        v.mute,
                        v.effective()
                    )
                });
            let _ = writeln!(
                out,
                "  Volume : node={:?} | route={:?}",
                node_vol, route_vol
            );
            let bleed_str = self.node_bleed.get(&n.id).map(|(t, info)| {
                format!(
                    "appeared_ago={:.2}s managed_at_launch={} new_app={}",
                    t.elapsed().as_secs_f64(),
                    info.managed_at_launch,
                    info.new_app
                )
            });
            let session_str = self.node_sessions.get(&key).map(|s| {
                format!(
                    "managed_at_launch={} new_app={} disconnected_ago={:?}",
                    s.info.managed_at_launch,
                    s.info.new_app,
                    s.disconnected_at.map(|d| format!("{:.2}s", d.elapsed().as_secs_f64()))
                )
            });
            let _ = writeln!(
                out,
                "  Bleed  : live={:?} | session={:?}",
                bleed_str, session_str
            );

            let ins = self.graph.ports_of(n.id, Direction::In);
            let outs = self.graph.ports_of(n.id, Direction::Out);
            for p in &ins {
                let _ = writeln!(out, "    Port #{:<4} [IN ] {:?}", p.id, p.name);
            }
            for p in &outs {
                let _ = writeln!(out, "    Port #{:<4} [OUT] {:?}", p.id, p.name);
            }
        }
        // Any orphan ports whose node_id is not in graph.nodes:
        let orphans: Vec<_> = self
            .graph
            .ports
            .values()
            .filter(|p| !self.graph.nodes.contains_key(&p.node_id))
            .collect();
        if !orphans.is_empty() {
            let _ = writeln!(out, "Orphan Ports (node not in graph.nodes):");
            for p in orphans {
                let _ = writeln!(
                    out,
                    "  Port #{:<4} node_id=#{} dir={:?} name={:?}",
                    p.id, p.node_id, p.direction, p.name
                );
            }
        }
        if !self.graph.routes.is_empty() {
            let _ = writeln!(out, "Device Route Volumes ({}):", self.graph.routes.len());
            for ((dev_id, route_dev), v) in &self.graph.routes {
                let _ = writeln!(
                    out,
                    "  Device #{dev_id} Route #{route_dev}: master={:.3} channels={:?} mute={} effective={:.3}",
                    v.master,
                    v.channels,
                    v.mute,
                    v.effective()
                );
            }
        }
        let _ = writeln!(out);

        // 4. PipeWire Live Links
        let _ = writeln!(
            out,
            "=== 4. PIPEWIRE LIVE LINKS ({} links) ===",
            self.graph.links.len()
        );
        if self.graph.links.is_empty() {
            let _ = writeln!(out, "  <none>");
        }
        for l in self.graph.links.values() {
            let owned = self.graph.is_owned(l);
            let pair = (l.out_port, l.in_port);
            let out_node_str = self
                .graph
                .nodes
                .get(&l.out_node)
                .map(|n| format!("{} ({})", n.label(), n.name))
                .unwrap_or_else(|| "?".to_string());
            let out_port_str = self
                .graph
                .ports
                .get(&l.out_port)
                .map_or("?", |p| p.name.as_str());
            let in_node_str = self
                .graph
                .nodes
                .get(&l.in_node)
                .map(|n| format!("{} ({})", n.label(), n.name))
                .unwrap_or_else(|| "?".to_string());
            let in_port_str = self
                .graph
                .ports
                .get(&l.in_port)
                .map_or("?", |p| p.name.as_str());

            let _ = writeln!(
                out,
                "Link #{:<4} [{}] #{} {} : #{} {}  -->  #{} {} : #{} {}  (owned_id={}, owned_pair={}, req={}, severed={})",
                l.id,
                if owned { "OWNED   " } else { "EXTERNAL" },
                l.out_node,
                out_node_str,
                l.out_port,
                out_port_str,
                l.in_node,
                in_node_str,
                l.in_port,
                in_port_str,
                owned,
                self.graph.owned_pairs.contains(&pair),
                self.requested.contains(&pair),
                self.severed_links.contains(&l.id)
            );
        }
        let _ = writeln!(out);

        // 5. Setups, Saved Links, Virtual Devices & Rules
        let _ = writeln!(
            out,
            "=== 5. SETUPS, SAVED LINKS, VIRTUAL DEVICES & RULES ({} setups) ===",
            self.setups.items.len()
        );
        let current_saved = self.canvas.to_saved();
        for (i, s) in self.setups.items.iter().enumerate() {
            let data = if i == self.viewing { &current_saved } else { &s.data };
            let flags = match (i == self.setups.active, i == self.viewing) {
                (true, true) => " [ACTIVE, VIEWING]",
                (true, false) => " [ACTIVE]",
                (false, true) => " [VIEWING]",
                (false, false) => "",
            };
            let _ = writeln!(out, "Setup #{i} {:?}{flags}:", s.name);
            let _ = writeln!(
                out,
                "  Options         : bleed={:?} | sinks={:?} | show_external={} | link_style={:?} | snap={} | minimap={} | pan={:?} | zoom={:.2}",
                data.auto_connect_prevention,
                data.sink_hide_mode,
                data.show_external,
                data.link_style,
                data.snap_to_grid,
                data.show_minimap,
                data.pan,
                data.zoom
            );
            let _ = writeln!(out, "  Virtual Devices : ({})", data.virtual_devices.len());
            for vd in &data.virtual_devices {
                let live_ids: Vec<u32> = self
                    .graph
                    .nodes
                    .values()
                    .filter(|n| self.graph.is_node_owned(n) && n.name == vd.name)
                    .map(|n| n.id)
                    .collect();
                let _ = writeln!(
                    out,
                    "    - kind={:?} name={:?} desc={:?} live_node_ids={:?}",
                    vd.kind, vd.name, vd.description, live_ids
                );
            }
            let _ = writeln!(out, "  Saved Links     : ({})", data.links.len());
            for (li, sl) in data.links.iter().enumerate() {
                let resolved = links::resolve(&self.graph, sl);
                let live_link = resolved.and_then(|(o, inp)| {
                    self.graph
                        .find_link(o, inp)
                        .map(|l| (l.id, self.graph.is_owned(l)))
                });
                let _ = writeln!(
                    out,
                    "    [{li}] {:?}:{:?} -> {:?}:{:?} | enabled={} | switch={} (pos={:?}) | linger={} | waypoints={} | resolved_ports={:?} | live_link={:?}",
                    sl.out_node,
                    sl.out_port,
                    sl.in_node,
                    sl.in_port,
                    sl.enabled,
                    sl.has_switch,
                    sl.switch_pos,
                    sl.lingering,
                    sl.waypoints.len(),
                    resolved,
                    live_link
                );
            }
            let _ = writeln!(out, "  Hidden Nodes    : ({}) {:?}", data.hidden.len(), data.hidden);
            let _ = writeln!(out, "  Nicknames       : ({}) {:?}", data.nicknames.len(), data.nicknames);
            let _ = writeln!(out, "  Notes & Blocks  : ({})", data.notes.len());
            for note in &data.notes {
                let _ = writeln!(
                    out,
                    "    - id={} kind={:?} pos={:?} size={:?} color={:?} text={:?}",
                    note.id, note.kind, note.pos, note.size, note.color, note.text
                );
            }
            let _ = writeln!(out, "  Rule Groups     : ({})", data.groups.len());
            for (gi, g_entry) in data.groups.iter().enumerate() {
                match g_entry {
                    GroupEntry::Local(rg) => {
                        let _ = writeln!(
                            out,
                            "    [{gi}] Local Group {:?} (enabled={}, rules={})",
                            rg.name,
                            rg.enabled,
                            rg.rules.len()
                        );
                        for r in &rg.rules {
                            let _ = writeln!(
                                out,
                                "      * enabled={} field={:?} mode={:?} pattern={:?} action={:?}",
                                r.enabled, r.field, r.mode, r.pattern, r.action
                            );
                        }
                    }
                    GroupEntry::Library { name, enabled } => {
                        let _ = writeln!(
                            out,
                            "    [{gi}] Library Ref {:?} (enabled={})",
                            name, enabled
                        );
                    }
                }
            }
        }
        let _ = writeln!(
            out,
            "Shared Rule Library : ({} groups)",
            self.canvas.rules.library.len()
        );
        for rg in &self.canvas.rules.library {
            let _ = writeln!(out, "  - Library Group {:?} (rules={})", rg.name, rg.rules.len());
            for r in &rg.rules {
                let _ = writeln!(
                    out,
                    "    * enabled={} field={:?} mode={:?} pattern={:?} action={:?}",
                    r.enabled, r.field, r.mode, r.pattern, r.action
                );
            }
        }
        let _ = writeln!(out);

        // 6. Recent Event & Command Log
        let log = self.event_log.borrow();
        let _ = writeln!(
            out,
            "=== 6. RECENT EVENT & COMMAND LOG ({} entries) ===",
            log.len()
        );
        if log.is_empty() {
            let _ = writeln!(out, "  <empty>");
        } else {
            for entry in log.iter() {
                let _ = writeln!(out, "  {entry}");
            }
        }
        let _ = writeln!(out, "================================================================================");
        out
    }

    pub(crate) fn sysinfo_window(&mut self, ctx: &egui::Context) {
        if !self.show_sysinfo {
            return;
        }
        let mut open = self.show_sysinfo;
        let report = self.generate_system_report();
        let mut clear_log = false;

        egui::Window::new("System Information & Issue Report")
            .open(&mut open)
            .default_width(780.0)
            .default_height(560.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.label(
                    "Full diagnostic snapshot of your PipeWire graph, RedoxRouter setups, bleed state, \
                     and recent event/command log. Attach or paste this report when reporting an issue.",
                );
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("Copy Full Report to Clipboard")
                                    .strong()
                                    .color(egui::Color32::WHITE),
                            )
                            .fill(self.theme.c_accent()),
                        )
                        .clicked()
                    {
                        ctx.copy_text(report.clone());
                        self.sysinfo_status =
                            Some(Ok("Copied full diagnostic report to clipboard.".to_string()));
                    }
                    if ui.button("Print to Terminal (stdout)").clicked() {
                        println!("{report}");
                        self.sysinfo_status =
                            Some(Ok("Printed full diagnostic report to stdout.".to_string()));
                    }
                    if ui.button("Clear Event Log").clicked() {
                        clear_log = true;
                    }
                });
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.label("Save to file:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.sysinfo_save_path)
                            .hint_text("~/redox-router-report.txt")
                            .desired_width(360.0),
                    );
                    if ui.button("Save Report").clicked() {
                        let expanded = storage::expand_tilde(&self.sysinfo_save_path);
                        if let Some(parent) = expanded.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        match std::fs::write(&expanded, &report) {
                            Ok(()) => {
                                self.sysinfo_status =
                                    Some(Ok(format!("Saved report to {}", expanded.display())));
                            }
                            Err(e) => {
                                self.sysinfo_status =
                                    Some(Err(format!("Could not save report: {e}")));
                            }
                        }
                    }
                });
                if let Some(ref st) = self.sysinfo_status {
                    match st {
                        Ok(msg) => {
                            ui.colored_label(egui::Color32::from_rgb(110, 210, 140), msg);
                        }
                        Err(err) => {
                            ui.colored_label(egui::Color32::LIGHT_RED, err);
                        }
                    }
                }
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let mut view_text = report.as_str();
                        ui.add(
                            egui::TextEdit::multiline(&mut view_text)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY),
                        );
                    });
            });

        if clear_log {
            self.event_log.borrow_mut().clear();
            self.sysinfo_status = Some(Ok("Cleared recent event log.".to_string()));
        }
        self.show_sysinfo = open;
    }

    pub(crate) fn debug_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("System Info / Issue Report…")
                            .strong()
                            .color(egui::Color32::WHITE),
                    )
                    .fill(self.theme.c_accent()),
                )
                .on_hover_text(
                    "Open the full PipeWire & RedoxRouter diagnostic report window to copy or save",
                )
                .clicked()
            {
                self.show_sysinfo = true;
            }
            if ui
                .button("Copy Report")
                .on_hover_text("Copy the complete diagnostic report directly to the clipboard")
                .clicked()
            {
                let report = self.generate_system_report();
                ui.ctx().copy_text(report);
                self.sysinfo_status =
                    Some(Ok("Copied full diagnostic report to clipboard.".to_string()));
            }
            if ui
                .button("Print to stdout")
                .on_hover_text("Print the complete diagnostic report to the terminal")
                .clicked()
            {
                println!("{}", self.generate_system_report());
            }
        });
        ui.separator();
        let g = &self.graph;
        egui::ScrollArea::vertical().show(ui, |ui| {
            for n in g.nodes.values() {
                let title = format!("#{}  {}  [{}]", n.id, n.label(), n.media_class);
                egui::CollapsingHeader::new(title)
                    .id_salt(n.id)
                    .show(ui, |ui| {
                        ui.monospace(format!("name: {}", n.name));
                        for p in g.ports_of(n.id, Direction::In) {
                            ui.label(format!("in   #{} {}", p.id, p.name));
                        }
                        for p in g.ports_of(n.id, Direction::Out) {
                            ui.label(format!("out  #{} {}", p.id, p.name));
                        }
                    });
            }
            ui.separator();
            ui.label("Links (* = created/owned by RedoxRouter)");
            for l in g.links.values() {
                let port = |id| g.ports.get(&id).map_or("?", |p| p.name.as_str());
                let node = |id| g.nodes.get(&id).map_or("?", |n| n.label());
                ui.label(format!(
                    "{} [#{}] #{} {}:{} (#{})  ->  #{} {}:{} (#{})",
                    if g.is_owned(l) { "*" } else { " " },
                    l.id,
                    l.out_node,
                    node(l.out_node),
                    port(l.out_port),
                    l.out_port,
                    l.in_node,
                    node(l.in_node),
                    port(l.in_port),
                    l.in_port
                ));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_and_kernel_helpers_return_non_empty() {
        assert!(!read_os_pretty_name().is_empty());
        assert!(!read_kernel_release().is_empty());
        assert_eq!(env_or("REDOX_DEFINITELY_UNSET_VAR_12345", "fallback"), "fallback");
    }
}

