//! RedoxRouter by ZeroKun265 - a node-based PipeWire patchbay.
//! Current state: node canvas; you create/delete your own ("owned") links,
//! other programs' links are hidden by default. See ROADMAP.md.

mod canvas;
mod graph;
mod layout;
mod pw_backend;
mod rules;
mod volume;
mod storage;

use canvas::{Action, Canvas};
use eframe::egui;
use graph::{Command, Direction, Graph, NodeInfo};
use pw_backend::CommandSender;
use rules::{Action as RuleAction, Field, GroupEntry, NodeText, Rule, RuleSet};
use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

struct RedoxRouter {
    rx: Receiver<graph::Event>,
    cmd: CommandSender,
    graph: Graph,
    canvas: Canvas,
    show_debug: bool,
    show_rules: bool,
    /// Ports the user tried to connect although another program already links them.
    prompt: Option<(u32, u32)>,
    last_save: Instant,
    save_error: Option<String>,
}

impl RedoxRouter {
    fn new(rx: Receiver<graph::Event>, cmd: CommandSender) -> Self {
        Self {
            rx,
            cmd,
            graph: Graph::default(),
            canvas: {
                let mut c = Canvas::with_saved(storage::load().unwrap_or_default());
                c.rules.library = storage::load_library();
                c
            },
            show_debug: false,
            show_rules: false,
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
                err = storage::save(&self.canvas.to_saved()).err();
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
            self.graph.apply(ev);
        }
    }

    fn send(&self, cmd: Command) {
        if self.cmd.send(cmd).is_err() {
            eprintln!("PipeWire thread is not running; command dropped");
        }
    }

    fn handle(&mut self, action: Action) {
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
                self.send(cmd);
            }
            Action::Disconnect { out_port, in_port } => {
                self.send(Command::DestroyLink { out_port, in_port });
            }
            Action::SetVolume { node_id, linear, channels, route } => {
                // No local guess: the slider follows what PipeWire reports back.
                self.send(match route {
                    Some(r) => Command::SetRouteVolume {
                        device_id: r.device_id,
                        route_index: r.index,
                        route_device: r.device,
                        linear,
                        channels,
                    },
                    None => Command::SetVolume { node_id, linear, channels },
                });
            }
            Action::SetMute { node_id, mute, route } => {
                self.send(match route {
                    Some(r) => Command::SetRouteMute {
                        device_id: r.device_id,
                        route_index: r.index,
                        route_device: r.device,
                        mute,
                    },
                    None => Command::SetMute { node_id, mute },
                });
            }
        }
    }

    /// Replace another program's link between these ports with one of ours.
    fn take_over(&mut self, out_port: u32, in_port: u32) {
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
        self.send(cmd);
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
                    "Groups run top to bottom (Hide wins over Color; the first Color wins). \
                     Text matches anywhere (case-insensitive); use * (any text) and ? (one \
                     character) to match the whole text. \"Title\" is the text shown on the node. \
                     Library groups are shared by every setup: edit one and it changes everywhere.",
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
            if rule.field == Field::MediaClass {
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
            let mut hide = matches!(rule.action, RuleAction::Hide);
            egui::ComboBox::from_id_salt(("rule_action", salt, i))
                .width(70.0)
                .selected_text(if hide { "Hide" } else { "Color" })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut hide, true, "Hide");
                    ui.selectable_value(&mut hide, false, "Color");
                });
            if hide != matches!(rule.action, RuleAction::Hide) {
                rule.action = if hide { RuleAction::Hide } else { RuleAction::Color([90, 90, 140]) };
                changed = true;
            }
            if let RuleAction::Color(c) = &mut rule.action {
                changed |= ui.color_edit_button_srgb(c).changed();
            }
            let n = cx
                .nodes
                .values()
                .filter(|n| {
                    rule.matches(&NodeText {
                        title: n.label(),
                        nickname: cx.nicks.get(&n.name).map_or("", |s| s.as_str()),
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
                if ui.button("Reset layout").clicked() {
                    self.canvas.reset_layout();
                }
                let hidden: Vec<String> = self.canvas.hidden.iter().cloned().collect();
                if !hidden.is_empty() {
                    ui.menu_button(format!("Hidden ({})", hidden.len()), |ui| {
                        for name in &hidden {
                            let present = self.graph.nodes.values().any(|n| &n.name == name);
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
                ui.toggle_value(&mut self.show_rules, "Rules");
                ui.checkbox(&mut self.show_debug, "Debug list");
                if let Some(e) = &self.graph.error {
                    ui.colored_label(egui::Color32::LIGHT_RED, e);
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
