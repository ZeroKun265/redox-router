//! RedoxRouter by ZeroKun265 - a node-based PipeWire patchbay.
//! Current state: node canvas; you create/delete your own ("owned") links,
//! other programs' links are hidden by default. See ROADMAP.md.

mod canvas;
mod graph;
mod layout;
mod links;
mod pw_backend;
mod rules;
mod setups;
mod volume;
mod storage;

use canvas::{Action, Canvas};
use eframe::egui;
use graph::{Command, Direction, Graph, NodeInfo};
use pw_backend::CommandSender;
use rules::{Mode, Action as RuleAction, Field, GroupEntry, NodeText, Rule, RuleSet};
use std::collections::{BTreeMap, HashSet};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

struct RedoxRouter {
    rx: Receiver<graph::Event>,
    cmd: CommandSender,
    graph: Graph,
    canvas: Canvas,
    show_debug: bool,
    show_rules: bool,
    show_links: bool,
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
    show_create_sink: bool,
    new_sink_name: String,
    new_sink_desc: String,
    saved_sinks: Vec<storage::SavedSink>,
    last_save: Instant,
    save_error: Option<String>,
}

impl RedoxRouter {
    fn new(rx: Receiver<graph::Event>, cmd: CommandSender) -> Self {
        let setups = storage::load_setups();
        let viewing = setups.active;
        let mut canvas = Canvas::with_saved(setups.items[viewing].data.clone());
        canvas.rules.library = storage::load_library();
        let saved_sinks = storage::load_sinks();
        for sink in &saved_sinks {
            let _ = cmd.send(Command::CreateSink {
                name: sink.name.clone(),
                description: sink.description.clone(),
            });
        }
        Self {
            rx,
            cmd,
            graph: Graph::default(),
            canvas,
            setups,
            viewing,
            renaming: None,
            inactive_notice: false,
            confirm_reset: false,
            show_debug: false,
            show_rules: false,
            show_links: false,
            show_create_sink: false,
            new_sink_name: String::new(),
            new_sink_desc: String::new(),
            saved_sinks,
            requested: HashSet::new(),
            severed_links: HashSet::new(),
            prompt: None,
            last_save: Instant::now(),
            save_error: None,
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
            match &ev {
                graph::Event::Removed(id) => {
                    self.severed_links.remove(id);
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
            self.saved_sinks.retain(|s| s.name != n.name);
            let _ = storage::save_sinks(&self.saved_sinks);
        }
        self.send(Command::DestroySink { node_id });
    }

    fn send(&self, cmd: Command) {
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
                let cmd = Command::CreateLink {
                    out_node: po.node_id,
                    out_port,
                    in_node: pi.node_id,
                    in_port,
                };
                self.requested.insert((out_port, in_port));
                self.remember_link(out_port, in_port);
                self.send(cmd);
            }
            Action::Disconnect { out_port, in_port } => {
                let link_id = self.graph.find_link(out_port, in_port).map(|l| l.id);
                if self.graph.find_link(out_port, in_port).is_some_and(|l| self.graph.is_owned(l)) {
                    if let Some(s) = links::from_ports(&self.graph, out_port, in_port) {
                        if links::remove(&mut self.canvas.links, &s) {
                            self.canvas.dirty = true;
                        }
                    }
                }
                self.requested.remove(&(out_port, in_port));
                self.send(Command::DestroyLink { link_id, out_port, in_port });
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
        let cmd = Command::TakeOver {
            link_id: l.id,
            out_node: l.out_node,
            out_port,
            in_node: l.in_node,
            in_port,
        };
        self.canvas.revealed.remove(&(out_port, in_port));
        self.requested.insert((out_port, in_port));
        self.remember_link(out_port, in_port);
        self.send(cmd);
    }

    /// Save a connection of ours by names so it comes back after a restart.
    fn remember_link(&mut self, out_port: u32, in_port: u32) {
        if let Some(s) = links::from_ports(&self.graph, out_port, in_port) {
            if links::add(&mut self.canvas.links, s) {
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
        let todo = links::missing(&self.graph, links, &self.requested);
        for (out_port, in_port) in todo {
            let (Some(po), Some(pi)) =
                (self.graph.ports.get(&out_port), self.graph.ports.get(&in_port))
            else {
                continue;
            };
            let cmd = Command::CreateLink {
                out_node: po.node_id,
                out_port,
                in_node: pi.node_id,
                in_port,
            };
            self.requested.insert((out_port, in_port));
            self.send(cmd);
        }
    }

    /// Sever unmanaged / external links to prevent audio bleed according to the active mode.
    fn enforce_bleed_prevention(&mut self) {
        let (active_links, mode) = if self.viewing == self.setups.active {
            (&self.canvas.links, self.canvas.auto_connect_prevention)
        } else {
            let d = &self.setups.items[self.setups.active].data;
            (&d.links, d.auto_connect_prevention)
        };

        let todo = links::bleed_links_to_sever(
            &self.graph,
            active_links,
            mode,
            &self.severed_links,
        );

        for (link_id, out_port, in_port) in todo {
            self.severed_links.insert(link_id);
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
        let external = self.canvas.show_external;
        let mut c = Canvas::with_saved(self.setups.items[idx].data.clone());
        c.rules.library = library;
        c.rules.library_dirty = library_dirty;
        c.show_external = external;
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
        self.requested.clear();
        self.severed_links.clear();
        self.canvas.dirty = true;
    }

    fn tabs_bar(&mut self, ctx: &egui::Context) {
        enum TabOp {
            View(usize),
            Add,
            Duplicate(usize),
            Rename(usize),
            Delete(usize),
            MakeActive,
        }
        let mut op = None;
        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (i, s) in self.setups.items.iter().enumerate() {
                    let active = i == self.setups.active;
                    let text = if active { format!("● {}", s.name) } else { s.name.clone() };
                    let r = ui.selectable_label(i == self.viewing, text);
                    if r.clicked() {
                        op = Some(TabOp::View(i));
                    }
                    r.context_menu(|ui| {
                        if ui.button("Rename…").clicked() {
                            op = Some(TabOp::Rename(i));
                            ui.close_menu();
                        }
                        if ui.button("Duplicate").clicked() {
                            op = Some(TabOp::Duplicate(i));
                            ui.close_menu();
                        }
                        let can_delete = !active && self.setups.items.len() > 1;
                        if ui.add_enabled(can_delete, egui::Button::new("Delete")).clicked() {
                            op = Some(TabOp::Delete(i));
                            ui.close_menu();
                        }
                    });
                }
                if ui
                    .button("+")
                    .on_hover_text("New setup: copies this layout and rules, without links")
                    .clicked()
                {
                    op = Some(TabOp::Add);
                }
                if self.viewing != self.setups.active {
                    if ui.button("Make active").clicked() {
                        op = Some(TabOp::MakeActive);
                    }
                    ui.weak("Not live: edits change this setup's saved links only; they connect when you make it active (● = live).");
                }
            });
        });
        match op {
            Some(TabOp::View(i)) => self.switch_to(i),
            Some(TabOp::Add) => {
                self.sync_current();
                let mut data = self.setups.items[self.viewing].data.clone();
                data.links.clear();
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
        self.saved_sinks.clear();
        let _ = storage::save_sinks(&self.saved_sinks);
        self.setups = setups::SetupList::new("Main", storage::LayoutFile::default());
        self.viewing = 0;
        self.canvas = Canvas::with_saved(storage::LayoutFile::default());
        self.canvas.rules.library_dirty = true; // writes the empty library
        self.canvas.dirty = true;
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
        }
        if yes || no {
            self.confirm_reset = false;
        }
    }

    fn create_sink_window(&mut self, ctx: &egui::Context) {
        if !self.show_create_sink {
            return;
        }
        let mut create = false;
        let mut cancel = false;
        egui::Window::new("Create Virtual Sink")
            .collapsible(false)
            .resizable(false)
            .default_width(320.0)
            .show(ctx, |ui| {
                ui.label("Create a virtual audio sink (null audio sink):");
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
                let sink = storage::SavedSink {
                    name: name.clone(),
                    description: description.clone(),
                };
                if let Some(existing) = self.saved_sinks.iter_mut().find(|s| s.name == sink.name) {
                    existing.description = sink.description;
                } else {
                    self.saved_sinks.push(sink);
                }
                let _ = storage::save_sinks(&self.saved_sinks);
                let already_live = self
                    .graph
                    .nodes
                    .values()
                    .any(|n| self.graph.is_node_owned(n) && n.name == name);
                if !already_live {
                    self.send(Command::CreateSink { name, description });
                }
            }
            self.show_create_sink = false;
        }
        if cancel {
            self.show_create_sink = false;
        }
    }

    fn inactive_window(&mut self, ctx: &egui::Context) {
        if !self.inactive_notice {
            return;
        }
        let (mut ok, mut activate) = (false, false);
        egui::Window::new("Setup not active").collapsible(false).resizable(false).show(ctx, |ui| {
            ui.label(
                "Connections can only be created, deleted or taken over on the active setup \
                 (marked ●). The links you see here belong to the live setup, so nothing was changed.",
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
                cancel = ui.button("Cancel").clicked();
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
        egui::Window::new("Saved links").open(&mut open).default_width(520.0).show(ctx, |ui| {
            ui.label(
                "Links you create are saved by node and port name and reconnected when both \
                 exist. Untick to pause one; Remove forgets it (the live link stays).",
            );
            if self.canvas.links.is_empty() {
                ui.weak("No saved links yet.");
            }
            for (i, l) in self.canvas.links.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(&mut l.enabled, "").changed();
                    ui.label(l.label());
                    let state = match links::resolve(&self.graph, l) {
                        Some((o, n))
                            if self.viewing == self.setups.active
                                && self.graph.find_link(o, n).is_some() =>
                        {
                            "connected"
                        }
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
            self.canvas.links.remove(i);
            changed = true;
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
                Op::DeleteLib(name) => rs.delete_from_library(&name),
            }
        }
        if lib_changed {
            rs.library_dirty = true;
        }
        if setup_changed {
            self.canvas.dirty = true;
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

fn debug_list(ui: &mut egui::Ui, g: &Graph) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        for n in g.nodes.values() {
            let title = format!("#{}  {}  [{}]", n.id, n.label(), n.media_class);
            egui::CollapsingHeader::new(title)
                .id_salt(n.id)
                .show(ui, |ui| {
                    for p in g.ports_of(n.id, Direction::In) {
                        ui.label(format!("in   #{} {}", p.id, p.name));
                    }
                    for p in g.ports_of(n.id, Direction::Out) {
                        ui.label(format!("out  #{} {}", p.id, p.name));
                    }
                });
        }
        ui.separator();
        ui.label("Links (* = created by RedoxRouter)");
        for l in g.links.values() {
            let port = |id| g.ports.get(&id).map_or("?", |p| p.name.as_str());
            let node = |id| g.nodes.get(&id).map_or("?", |n| n.label());
            ui.label(format!(
                "{} {}:{}  ->  {}:{}",
                if g.is_owned(l) { "*" } else { " " },
                node(l.out_node),
                port(l.out_port),
                node(l.in_node),
                port(l.in_port)
            ));
        }
    });
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

impl eframe::App for RedoxRouter {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_events();
        self.canvas.virtual_mode = self.viewing != self.setups.active;
        self.auto_connect();
        self.enforce_bleed_prevention();

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong("RedoxRouter");
                ui.label(format!(
                    "{} nodes, {} ports, {} links ({} yours)",
                    self.graph.nodes.len(),
                    self.graph.ports.len(),
                    self.graph.links.len(),
                    self.graph.owned_ids.len()
                ));
                if ui.button("Reset all settings…").clicked() {
                    self.confirm_reset = true;
                }
                if ui.button("Reset layout").clicked() {
                    self.canvas.reset_layout();
                }
                let hidden: Vec<String> = self.canvas.hidden.iter().cloned().collect();
                if !hidden.is_empty() {
                    ui.menu_button(format!("Hidden ({})", hidden.len()), |ui| {
                        for name in &hidden {
                            let present = self
                                .graph
                                .nodes
                                .values()
                                .any(|n| n.key() == *name || n.name == *name);
                            let label = if present {
                                format!("Show {name}")
                            } else {
                                format!("Show {name} (not present)")
                            };
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
                ui.checkbox(&mut self.canvas.show_external, "Show external links");
                ui.menu_button(format!("Bleed: {}", self.canvas.auto_connect_prevention.label()), |ui| {
                    for mode in storage::AutoConnectPrevention::ALL {
                        if ui.selectable_label(self.canvas.auto_connect_prevention == mode, mode.label()).clicked() {
                            self.canvas.auto_connect_prevention = mode;
                            self.severed_links.clear();
                            self.canvas.dirty = true;
                            ui.close_menu();
                        }
                    }
                })
                .response
                .on_hover_text("Audio bleed / auto-connection prevention:\n• Allow all: normal PipeWire behavior (leaves external/system links alone)\n• Block for managed nodes: automatically severs external/default connections for nodes configured in saved links (so audio only routes to your specified sinks)\n• Block for all new apps: severs external connections for all application streams until you explicitly route them");
                ui.menu_button(format!("Sinks: {}", self.canvas.sink_hide_mode.label()), |ui| {
                    for mode in storage::SinkHideMode::ALL {
                        if ui.selectable_label(self.canvas.sink_hide_mode == mode, mode.label()).clicked() {
                            self.canvas.sink_hide_mode = mode;
                            self.canvas.dirty = true;
                            ui.close_menu();
                        }
                    }
                })
                .response
                .on_hover_text("Control visibility of audio sinks. In the Rules window, add rules with 'Show' action to selectively unhide specific sinks.");
                if ui
                    .button("Create sink…")
                    .on_hover_text("Create a virtual null audio sink")
                    .clicked()
                {
                    self.show_create_sink = true;
                    self.new_sink_name = format!(
                        "Virtual_Sink_{}",
                        self.graph.nodes.values().filter(|n| n.is_sink()).count() + 1
                    );
                    self.new_sink_desc = "Virtual Sink".to_string();
                }
                let sinks: Vec<(u32, String)> = self
                    .graph
                    .nodes
                    .values()
                    .filter(|n| n.is_sink())
                    .map(|n| {
                        let tag = if self.graph.is_node_owned(n) { " [Redox]" } else { "" };
                        (n.id, format!("#{} {}{}", n.id, n.label(), tag))
                    })
                    .collect();
                if !sinks.is_empty() {
                    ui.menu_button("Destroy sink", |ui| {
                        for (id, label) in &sinks {
                            if ui.button(label).clicked() {
                                self.destroy_sink(*id);
                                ui.close_menu();
                            }
                        }
                    });
                }
                ui.toggle_value(&mut self.show_rules, "Rules");
                ui.toggle_value(&mut self.show_links, format!("Saved links ({})", self.canvas.links.len()));
                ui.checkbox(&mut self.show_debug, "Debug list");
                let mut clear_err = false;
                if let Some(e) = &self.graph.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, e);
                    if ui.small_button("✕").on_hover_text("Dismiss error").clicked() {
                        clear_err = true;
                    }
                }
                if clear_err {
                    self.graph.error = None;
                }
                if let Some(e) = &self.save_error {
                    ui.colored_label(egui::Color32::LIGHT_RED, format!("layout not saved: {e}"));
                }
                // A selected external link can be turned into one of ours.
                let external = self.canvas.selected().filter(|&(o, i)| {
                    self.graph
                        .find_link(o, i)
                        .is_some_and(|l| !self.graph.is_owned(l))
                });
                if let Some((o, i)) = external {
                    if ui.button("Make Redox connection (selected link)").clicked() {
                        self.take_over(o, i);
                    }
                }
            });
            ui.small("Drag from a port to another port to connect. Click a link, then press Delete to remove it. Right-click a node to hide it. Your links vanish when RedoxRouter closes.");
        });

        self.tabs_bar(ctx);

        if self.show_debug {
            egui::SidePanel::right("debug").show(ctx, |ui| debug_list(ui, &self.graph));
        }

        let mut action = None;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| action = self.canvas.show(ui, &self.graph));
        if let Some(a) = action {
            self.handle(a);
        }

        self.prompt_window(ctx);
        self.rules_window(ctx);
        self.links_window(ctx);
        self.rename_window(ctx);
        self.inactive_window(ctx);
        self.reset_window(ctx);
        self.create_sink_window(ctx);
        self.maybe_save(ctx);
    }
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "RedoxRouter",
        options,
        Box::new(|cc| {
            let (tx, rx) = mpsc::channel();
            let cmd = pw_backend::spawn(tx, cc.egui_ctx.clone());
            Ok(Box::new(RedoxRouter::new(rx, cmd)))
        }),
    )
}
