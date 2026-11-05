//! The node canvas: pan (drag background), zoom (scroll / pinch), drag nodes,
//! drag from port to port to connect, click a link + Delete to remove it.
//!
//! Link ownership: links RedoxRouter created ("owned") are always shown and
//! can be deleted. External links (made by other programs) are hidden unless
//! `show_external` is on or the pair is in `revealed`, and are read-only.

use crate::graph::{Direction, Graph, NodeInfo, PortInfo, RouteRef};
use crate::layout::{self, Placer, HEADER_H};
use crate::volume;
use std::collections::BTreeMap;
use crate::rules::{self, GroupEntry, RuleGroup, RuleSet};
use crate::storage::LayoutFile;
use eframe::egui::{
    self, pos2, vec2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Shape, Stroke,
    Vec2,
};
use std::collections::{BTreeSet, HashMap, HashSet};

/// What the user asked the canvas to do; the app decides how to carry it out.
#[derive(Debug, Clone, Copy)]
pub enum Action {
    Connect { out_port: u32, in_port: u32 },
    Disconnect { out_port: u32, in_port: u32 },
    /// `route`: set the sound card's route volume instead of the node's own.
    SetVolume { node_id: u32, linear: f32, channels: usize, route: Option<RouteRef> },
    SetMute { node_id: u32, mute: bool, route: Option<RouteRef> },
}

#[derive(Clone, Copy)]
struct Pending {
    port: u32,
    from_out: bool,
}

pub struct Canvas {
    /// Screen offset of the world origin, relative to the canvas top-left.
    pan: Vec2,
    zoom: f32,
    /// World position (top-left) of each placed node this session, by node id.
    pos: HashMap<u32, layout::P>,
    placer: Placer,
    /// Saved positions by node name; includes nodes that are not present now.
    saved: HashMap<String, layout::P>,
    /// Which live node currently owns each saved name.
    claimed: HashMap<String, u32>,
    /// Layout changed since the last save.
    pub dirty: bool,
    /// node.name of manually hidden nodes (not drawn, no ports, no links).
    pub hidden: BTreeSet<String>,
    /// Manual widths: by node id this session, and saved by node name.
    width: HashMap<u32, f32>,
    saved_w: HashMap<String, f32>,
    /// node.name -> nickname.
    pub nicknames: BTreeMap<String, String>,
    /// Filter rules (hide / color), edited from the Rules window.
    pub rules: RuleSet,
    /// Nickname being edited: (node.name, where to put the editor).
    editing: Option<(String, Pos2)>,
    edit_buf: String,
    edit_first: bool,
    /// Link being dragged out of a port.
    pending: Option<Pending>,
    /// Selected link as (out_port, in_port).
    selected: Option<(u32, u32)>,
    pub show_external: bool,
    /// External links the user chose to show individually ("visible only").
    pub revealed: HashSet<(u32, u32)>,
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            pan: vec2(20.0, 20.0),
            zoom: 1.0,
            pos: HashMap::new(),
            placer: Placer::default(),
            saved: HashMap::new(),
            claimed: HashMap::new(),
            dirty: false,
            hidden: BTreeSet::new(),
            width: HashMap::new(),
            saved_w: HashMap::new(),
            nicknames: BTreeMap::new(),
            rules: RuleSet::default(),
            editing: None,
            edit_buf: String::new(),
            edit_first: false,
            pending: None,
            selected: None,
            show_external: false,
            revealed: HashSet::new(),
        }
    }
}

/// A node that is visible this frame, with everything needed to draw it.
struct Item<'a> {
    node: &'a NodeInfo,
    ins: Vec<&'a PortInfo>,
    outs: Vec<&'a PortInfo>,
    rect: Rect,
    /// Width and header height in world units (zoom 1).
    w: f32,
    header_h: f32,
    /// Header color from a matching Color rule.
    color: Option<Color32>,
    /// Center (world units, from the node's top) of the volume slider row.
    vol_y: Option<f32>,
}

/// Colors offered in a node's right-click "Color" menu.
const SWATCHES: [[u8; 3]; 8] = [
    [55, 110, 80],
    [150, 95, 50],
    [80, 80, 95],
    [170, 70, 70],
    [70, 100, 170],
    [140, 90, 170],
    [60, 140, 150],
    [170, 150, 60],
];

fn lighten(c: Color32) -> Color32 {
    let up = |v: u8| v.saturating_add(24);
    Color32::from_rgb(up(c.r()), up(c.g()), up(c.b()))
}

/// Returns (drag started, drag stopped) for the hit area around a port.
fn port_interact(ui: &mut egui::Ui, port_id: u32, anchor: Pos2, z: f32) -> (bool, bool) {
    let size = Vec2::splat(14.0 * z.max(0.7));
    let resp = ui.interact(
        Rect::from_center_size(anchor, size),
        ui.id().with(("port", port_id)),
        Sense::drag(),
    );
    (resp.drag_started(), resp.drag_stopped())
}

impl Canvas {
    /// Start from a previously saved layout.
    pub fn with_saved(f: LayoutFile) -> Self {
        // Old files had one flat rule list: keep it as the first group.
        let mut groups = f.groups;
        if !f.rules.is_empty() {
            let group = RuleGroup { name: "Rules".to_string(), enabled: true, rules: f.rules };
            groups.insert(0, GroupEntry::Local(group));
        }
        Self {
            pan: vec2(f.pan.0, f.pan.1),
            zoom: f.zoom.clamp(layout::MIN_ZOOM, layout::MAX_ZOOM),
            saved: f.nodes.into_iter().collect(),
            hidden: f.hidden,
            saved_w: f.widths.into_iter().collect(),
            nicknames: f.nicknames,
            rules: RuleSet { groups, ..RuleSet::default() },
            ..Self::default()
        }
    }

    pub fn to_saved(&self) -> LayoutFile {
        LayoutFile {
            pan: (self.pan.x, self.pan.y),
            zoom: self.zoom,
            nodes: self.saved.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            hidden: self.hidden.clone(),
            widths: self.saved_w.iter().map(|(k, v)| (k.clone(), *v)).collect(),
            nicknames: self.nicknames.clone(),
            rules: Vec::new(),
            groups: self.rules.groups.clone(),
        }
    }

    pub fn hide(&mut self, name: String) {
        self.hidden.insert(name);
        self.dirty = true;
    }

    pub fn unhide(&mut self, name: &str) {
        self.hidden.remove(name);
        self.dirty = true;
    }

    pub fn unhide_all(&mut self) {
        self.hidden.clear();
        self.dirty = true;
    }

    /// Currently selected link as (out_port, in_port).
    pub fn selected(&self) -> Option<(u32, u32)> {
        self.selected
    }

    /// Forget all node positions (also the saved ones); nodes are re-stacked
    /// on the next frame.
    pub fn reset_layout(&mut self) {
        self.pos.clear();
        self.saved.clear();
        self.width.clear();
        self.saved_w.clear();
        self.claimed.clear();
        self.placer = Placer::default();
        self.dirty = true;
    }

    pub fn show(&mut self, ui: &mut egui::Ui, g: &Graph) -> Option<Action> {
        let mut action = None;
        let (rect, bg) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(0), Color32::from_gray(24));

        // Pan.
        if bg.dragged() {
            self.pan += bg.drag_delta();
        }
        if bg.drag_stopped() {
            self.dirty = true;
        }

        // Zoom around the cursor.
        if ui.rect_contains_pointer(rect) {
            let (scroll, pinch, cursor) = ui.input(|i| {
                (
                    i.smooth_scroll_delta.y,
                    i.zoom_delta(),
                    i.pointer.hover_pos(),
                )
            });
            let factor = pinch * (scroll * 0.002).exp();
            if factor != 1.0 {
                if let Some(c) = cursor {
                    let rel = c - rect.min;
                    let (pan, zoom) = layout::zoom_about(
                        (self.pan.x, self.pan.y),
                        self.zoom,
                        (rel.x, rel.y),
                        factor,
                    );
                    self.pan = vec2(pan.0, pan.1);
                    self.zoom = zoom;
                    self.dirty = true;
                }
            }
        }

        let z = self.zoom;
        let origin = rect.min + self.pan;
        let to_rect = |p: layout::P, h: f32, w: f32| {
            Rect::from_min_size(origin + vec2(p.0, p.1) * z, vec2(w, h) * z)
        };
        // Text width at zoom 1, used to size nodes so labels do not overlap.
        let (title_font, port_font) = (FontId::proportional(13.0), FontId::proportional(11.0));
        let text_w = |t: &str, f: &FontId| {
            painter
                .layout_no_wrap(t.to_string(), f.clone(), Color32::WHITE)
                .size()
                .x
        };
        // Wrapped text, at most two lines, ending in "…" when cut off.
        let lay = |t: &str, f: FontId, c: Color32, wrap_w: f32| {
            let mut job = egui::text::LayoutJob::simple(t.to_string(), f, c, wrap_w);
            job.wrap.max_rows = 2;
            job.wrap.overflow_character = Some('…');
            painter.layout_job(job)
        };

        // Pass 1: place, interact (drag nodes, drag from ports), collect anchors.
        let mut items: Vec<Item> = Vec::new();
        let mut anchors: HashMap<u32, Pos2> = HashMap::new();
        let mut drag_started: Option<Pending> = None;
        let mut drag_stopped = false;
        let mut to_hide: Vec<String> = Vec::new();
        let mut color_set: Vec<(String, [u8; 3])> = Vec::new();
        let mut color_clear: Vec<String> = Vec::new();
        for node in g.nodes.values() {
            let key = (!node.name.is_empty()).then_some(node.name.as_str());
            if key.is_some_and(|k| self.hidden.contains(k)) {
                continue;
            }
            let text = rules::NodeText {
                title: node.label(),
                nickname: self.nicknames.get(&node.name).map_or("", |s| s.as_str()),
                name: &node.name,
                description: &node.description,
                media_class: &node.media_class,
            };
            let outcome = rules::evaluate(self.rules.active_rules(), &text);
            if outcome.hidden {
                continue;
            }
            let color = outcome.color.map(|[r, g, b]| Color32::from_rgb(r, g, b));
            let ins = g.ports_of(node.id, Direction::In);
            let outs = g.ports_of(node.id, Direction::Out);
            if ins.is_empty() && outs.is_empty() {
                continue; // nothing to draw or connect yet
            }
            let widest = |ports: &[&PortInfo]| {
                ports.iter().map(|p| text_w(&p.name, &port_font)).fold(0.0, f32::max)
            };
            let w_auto = layout::node_width(
                text_w(node.label(), &title_font),
                widest(&ins),
                widest(&outs),
            );
            // A manual width (saved by name) overrides the automatic one.
            if !self.width.contains_key(&node.id) {
                if let Some(k) = key {
                    if let Some(&sw) = self.saved_w.get(k) {
                        let alive = |id| g.nodes.contains_key(&id);
                        if layout::try_claim(&mut self.claimed, alive, k, node.id) {
                            self.width.insert(node.id, sw);
                        }
                    }
                }
            }
            let w = self.width.get(&node.id).copied().unwrap_or(w_auto);
            // Row heights grow when labels wrap; each side gets half the width.
            let half = ((w - 40.0) / 2.0).max(30.0);
            let label_h = |ports: &[&PortInfo], i: usize| {
                ports.get(i).map_or(0.0, |p| {
                    lay(&p.name, port_font.clone(), Color32::WHITE, half).size().y
                })
            };
            let row_h: Vec<f32> = (0..ins.len().max(outs.len()))
                .map(|i| (label_h(&ins, i).max(label_h(&outs, i)) + 6.0).max(layout::ROW_H))
                .collect();
            let title_h = lay(node.label(), title_font.clone(), Color32::WHITE, w - 16.0).size().y;
            let header_h = (title_h + 10.0).max(HEADER_H);
            // Audio nodes get a volume slider row below their ports.
            let has_vol = g.volume_target(node).is_some();
            let mut all_rows = row_h.clone();
            if has_vol {
                all_rows.push(layout::VOL_ROW_H);
            }
            let (h, centers) = layout::node_rows(header_h, &all_rows);
            let vol_y = has_vol.then(|| centers[row_h.len()]);
            let col = layout::column_for(&node.media_class);
            // Saved nodes are matched by name (ids change between sessions).
            let mut p = match self.pos.get(&node.id) {
                Some(&p) => p,
                None => {
                    let saved = key.and_then(|k| {
                        layout::claim_saved(
                            &self.saved,
                            &mut self.claimed,
                            |id| g.nodes.contains_key(&id),
                            k,
                            node.id,
                        )
                    });
                    let p = saved.unwrap_or_else(|| self.placer.place(col, h));
                    self.pos.insert(node.id, p);
                    p
                }
            };
            let mut r = to_rect(p, h, w);

            let visible = r.intersects(rect);
            if visible {
                let id = ui.id().with(("node", node.id));
                let resp = ui.interact(r, id, Sense::click_and_drag());
                if let Some(k) = key {
                    resp.context_menu(|ui| {
                        if ui.button("Hide node").clicked() {
                            to_hide.push(k.to_string());
                            ui.close_menu();
                        }
                        ui.menu_button("Color", |ui| {
                            ui.horizontal(|ui| {
                                for c in SWATCHES {
                                    let (sw, sresp) =
                                        ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                                    let fill = Color32::from_rgb(c[0], c[1], c[2]);
                                    ui.painter().rect_filled(sw, CornerRadius::same(3), fill);
                                    if sresp.clicked() {
                                        color_set.push((k.to_string(), c));
                                        ui.close_menu();
                                    }
                                }
                            });
                            if ui.button("Reset color").clicked() {
                                color_clear.push(k.to_string());
                                ui.close_menu();
                            }
                        });
                    });
                }
                // Tooltip over the title block: the technical name.
                let header = Rect::from_min_size(r.min, vec2(r.width(), header_h * z));
                let in_header = ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| header.contains(p));
                if resp.hovered() && in_header {
                    resp.clone().on_hover_ui(|ui| {
                        ui.monospace(&node.name);
                        ui.weak(format!("{}  #{}", node.media_class, node.id));
                    });
                }
                if resp.dragged() {
                    let d = resp.drag_delta() / z;
                    p = (p.0 + d.x, p.1 + d.y);
                    self.pos.insert(node.id, p);
                    if let Some(k) = key {
                        let alive = |id| g.nodes.contains_key(&id);
                        if layout::try_claim(&mut self.claimed, alive, k, node.id) {
                            self.saved.insert(k.to_string(), p);
                        }
                    }
                    r = to_rect(p, h, w);
                }
                if resp.drag_stopped() {
                    self.dirty = true;
                }
            }

            // Ports are interacted after the node so they win the hit test.
            let sides = [(&ins, false), (&outs, true)];
            for (ports, is_out) in sides {
                for (row, port) in ports.iter().enumerate() {
                    let anchor = r.min + vec2(if is_out { w } else { 0.0 }, centers[row]) * z;
                    anchors.insert(port.id, anchor);
                    if visible {
                        let (started, stopped) = port_interact(ui, port.id, anchor, z);
                        if started {
                            drag_started = Some(Pending { port: port.id, from_out: is_out });
                        }
                        drag_stopped |= stopped;
                    }
                }
            }
            // Resize grip in the bottom-right corner (double-click: automatic width).
            if visible {
                let zz = z.max(0.7);
                let grip = Rect::from_min_max(r.max - vec2(12.0, 10.0) * zz, r.max);
                let resp = ui
                    .interact(grip, ui.id().with(("grip", node.id)), Sense::click_and_drag())
                    .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
                if resp.dragged() {
                    let nw = (w + resp.drag_delta().x / z)
                        .clamp(layout::MIN_RESIZE_W, layout::MAX_RESIZE_W);
                    self.width.insert(node.id, nw);
                    if let Some(k) = key {
                        let alive = |id| g.nodes.contains_key(&id);
                        if layout::try_claim(&mut self.claimed, alive, k, node.id) {
                            self.saved_w.insert(k.to_string(), nw);
                        }
                    }
                }
                if resp.double_clicked() {
                    self.width.remove(&node.id);
                    if let Some(k) = key {
                        self.saved_w.remove(k);
                    }
                    self.dirty = true;
                }
                if resp.drag_stopped() {
                    self.dirty = true;
                }
            }
            items.push(Item { node, ins, outs, rect: r, w, header_h, color, vol_y });
        }

        for name in to_hide {
            self.hide(name);
        }
        if !color_set.is_empty() || !color_clear.is_empty() {
            for (name, c) in color_set {
                self.rules.set_manual_color(&name, c);
            }
            for name in color_clear {
                self.rules.clear_manual_color(&name);
            }
            self.dirty = true;
        }

        if let Some(p) = drag_started {
            self.pending = Some(p);
            self.selected = None;
        }

        // Finish a link drag: connect to the nearest compatible port, if any.
        if drag_stopped {
            if let Some(from) = self.pending.take() {
                let pointer = ui.input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos()));
                if let Some(target) = pointer.and_then(|pos| self.target_at(pos, &anchors, g, from)) {
                    action = Some(if from.from_out {
                        Action::Connect { out_port: from.port, in_port: target }
                    } else {
                        Action::Connect { out_port: target, in_port: from.port }
                    });
                }
            }
        }
        if self.pending.is_some() && !ui.input(|i| i.pointer.any_down()) {
            self.pending = None; // drag ended without us seeing the stop event
        }

        // Pass 2: links (under the nodes). External ones only when revealed.
        let mut shapes: Vec<((u32, u32), Vec<Pos2>)> = Vec::new();
        for l in g.links.values() {
            let pair = (l.out_port, l.in_port);
            let owned = g.is_owned(l);
            if !(owned || self.show_external || self.revealed.contains(&pair)) {
                continue;
            }
            let (Some(a), Some(b)) = (anchors.get(&l.out_port), anchors.get(&l.in_port)) else {
                continue;
            };
            let pts: Vec<Pos2> = layout::bezier_points((a.x, a.y), (b.x, b.y), 24, 40.0 * z)
                .into_iter()
                .map(|(x, y)| pos2(x, y))
                .collect();
            let stroke = if self.selected == Some(pair) {
                Stroke::new((3.5 * z).clamp(2.0, 5.0), Color32::from_rgb(255, 220, 80))
            } else if owned {
                Stroke::new((2.0 * z).clamp(1.0, 3.0), Color32::from_rgb(120, 180, 255))
            } else {
                Stroke::new(
                    (1.5 * z).clamp(1.0, 2.5),
                    Color32::from_rgba_unmultiplied(150, 150, 165, 130),
                )
            };
            painter.add(Shape::line(pts.clone(), stroke));
            shapes.push((pair, pts));
        }

        // Click selects the nearest link; click on empty space clears.
        if bg.clicked() {
            self.selected = bg.interact_pointer_pos().and_then(|pos| {
                shapes
                    .iter()
                    .map(|(pair, pts)| {
                        let d = layout::dist_to_polyline(
                            (pos.x, pos.y),
                            &pts.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>(),
                        );
                        (*pair, d)
                    })
                    .filter(|(_, d)| *d <= 7.0)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(pair, _)| pair)
            });
        }
        // Delete removes the selected link, but only if we own it.
        if let Some(pair) = self.selected {
            // Forget the selection if the link is gone or no longer drawn
            // (e.g. one of its nodes was hidden).
            if g.find_link(pair.0, pair.1).is_none() || !shapes.iter().any(|(p, _)| *p == pair) {
                self.selected = None;
            } else if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                let owned = g.find_link(pair.0, pair.1).is_some_and(|l| g.is_owned(l));
                if owned {
                    action = Some(Action::Disconnect { out_port: pair.0, in_port: pair.1 });
                    self.selected = None;
                }
            }
        }

        // Pass 3: nodes.
        for it in &items {
            let r = it.rect;
            if !r.intersects(rect) {
                continue;
            }
            let default_color = match layout::column_for(&it.node.media_class) {
                0 => Color32::from_rgb(55, 110, 80),
                2 => Color32::from_rgb(150, 95, 50),
                _ => Color32::from_rgb(80, 80, 95),
            };
            let header_color = it.color.unwrap_or(default_color);
            painter.rect_filled(r.expand(z), CornerRadius::same(5), Color32::from_gray(90));
            painter.rect_filled(r, CornerRadius::same(4), Color32::from_gray(45));
            let header = Rect::from_min_size(r.min, vec2(r.width(), it.header_h * z));
            painter.rect_filled(
                header,
                CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 },
                header_color,
            );

            let clip = painter.with_clip_rect(r.intersect(rect));
            let show_text = z >= 0.4;
            if show_text {
                let white = Color32::WHITE;
                let g = lay(it.node.label(), FontId::proportional(13.0 * z), white, (it.w - 16.0) * z);
                let pos = r.min + vec2(8.0 * z, (it.header_h * z - g.size().y) / 2.0);
                clip.galley(pos, g, white);
            }
            let half = ((it.w - 40.0) / 2.0).max(30.0);
            let ports = it
                .ins
                .iter()
                .map(|p| (p, false))
                .chain(it.outs.iter().map(|p| (p, true)));
            for (port, is_out) in ports {
                let Some(&a) = anchors.get(&port.id) else { continue };
                let color = if is_out {
                    Color32::from_rgb(110, 210, 140)
                } else {
                    Color32::from_rgb(240, 170, 90)
                };
                painter.circle_filled(a, 4.0 * z, color);
                if show_text {
                    let gray = Color32::from_gray(210);
                    let g = lay(&port.name, FontId::proportional(11.0 * z), gray, half * z);
                    let dy = -g.size().y / 2.0;
                    let pos = if is_out {
                        a + vec2(-8.0 * z - g.size().x, dy)
                    } else {
                        a + vec2(8.0 * z, dy)
                    };
                    clip.galley(pos, g, gray);
                }
            }

            // Volume slider: drag or click the track; double-click = 100%;
            // click the percentage to mute.
            if let (Some(cy), Some(target)) = (it.vol_y, g.volume_target(it.node)) {
                let vol = target.vol;
                let s = volume::linear_to_slider(vol.effective()).min(volume::SLIDER_MAX);
                let top = r.min.y + (cy - layout::VOL_ROW_H / 2.0) * z;
                let bottom = r.min.y + (cy + layout::VOL_ROW_H / 2.0) * z;
                let track = Rect::from_min_max(
                    pos2(r.min.x + 10.0 * z, r.min.y + (cy - 3.0) * z),
                    pos2(r.max.x - 56.0 * z, r.min.y + (cy + 3.0) * z),
                );
                let hit = track.expand2(vec2(6.0 * z, 7.0 * z));
                let resp = ui.interact(hit, ui.id().with(("vol", it.node.id)), Sense::click_and_drag());
                let mut new_s = None;
                if resp.dragged() || resp.clicked() {
                    if let Some(p) = resp.interact_pointer_pos() {
                        let t = ((p.x - track.min.x) / track.width()).clamp(0.0, 1.0);
                        new_s = Some(t * volume::SLIDER_MAX);
                    }
                }
                if resp.double_clicked() {
                    new_s = Some(1.0);
                }
                if let Some(ns) = new_s.filter(|ns| (ns - s).abs() > 0.004) {
                    if action.is_none() {
                        action = Some(Action::SetVolume {
                            node_id: it.node.id,
                            linear: volume::slider_to_linear(ns),
                            channels: vol.channels.len(),
                            route: target.route,
                        });
                    }
                }

                let (track_col, fill_col) = if vol.mute {
                    (Color32::from_gray(55), Color32::from_gray(100))
                } else {
                    (Color32::from_gray(70), Color32::from_rgb(110, 170, 230))
                };
                painter.rect_filled(track, CornerRadius::same(3), track_col);
                let frac = (s / volume::SLIDER_MAX).clamp(0.0, 1.0);
                let filled =
                    Rect::from_min_max(track.min, pos2(track.min.x + track.width() * frac, track.max.y));
                painter.rect_filled(filled, CornerRadius::same(3), fill_col);
                painter.circle_filled(pos2(filled.max.x, track.center().y), 5.0 * z, Color32::from_gray(225));

                let label = Rect::from_min_max(pos2(track.max.x + 8.0 * z, top), pos2(r.max.x - 4.0 * z, bottom));
                let lresp = ui
                    .interact(label, ui.id().with(("mute", it.node.id)), Sense::click())
                    .on_hover_text("Click to mute / unmute");
                if lresp.clicked() && action.is_none() {
                    action = Some(Action::SetMute {
                        node_id: it.node.id,
                        mute: !vol.mute,
                        route: target.route,
                    });
                }
                if show_text {
                    let text = if vol.mute { "muted".to_string() } else { format!("{}%", volume::percent(s)) };
                    let col = if vol.mute { Color32::from_rgb(230, 120, 110) } else { Color32::from_gray(210) };
                    let g = lay(&text, FontId::proportional(11.0 * z), col, 1000.0);
                    let pos = pos2(label.min.x, (top + bottom) / 2.0 - g.size().y / 2.0);
                    clip.galley(pos, g, col);
                }
            }

            // Resize grip marks.
            let gs = Stroke::new((1.5 * z).max(1.0), Color32::from_gray(130));
            painter.line_segment([r.max + vec2(-10.0, -2.0) * z, r.max + vec2(-2.0, -10.0) * z], gs);
            painter.line_segment([r.max + vec2(-6.0, -2.0) * z, r.max + vec2(-2.0, -6.0) * z], gs);

            // Nickname tab above the top-left corner; click the pencil to rename.
            if !it.node.name.is_empty() {
                let nick = self.nicknames.get(&it.node.name).cloned();
                let tab_h = layout::TAB_H * z;
                let text_g = nick.as_ref().filter(|_| show_text).map(|n| {
                    painter.layout_no_wrap(n.clone(), FontId::proportional(11.0 * z), Color32::WHITE)
                });
                let text_w = text_g.as_ref().map_or(0.0, |g| g.size().x + 6.0 * z);
                let tab_w = (text_w + 22.0 * z).min(r.width());
                let tab = Rect::from_min_size(pos2(r.min.x, r.min.y - tab_h), vec2(tab_w, tab_h));
                let resp = ui
                    .interact(tab, ui.id().with(("tab", it.node.id)), Sense::click())
                    .on_hover_text("Rename (nickname)");
                let fill = if resp.hovered() { lighten(header_color) } else { header_color };
                let top = CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 };
                painter.rect_filled(tab.expand(z), top, Color32::from_gray(90));
                painter.rect_filled(tab, top, fill);
                if let Some(g) = text_g {
                    let pos = tab.min + vec2(6.0 * z, (tab_h - g.size().y) / 2.0);
                    painter.with_clip_rect(tab).galley(pos, g, Color32::WHITE);
                }
                // Pencil, drawn with lines so it needs no special font.
                let c = pos2(tab.max.x - 11.0 * z, tab.center().y);
                let ps = Stroke::new((1.6 * z).max(1.0), Color32::from_gray(205));
                painter.line_segment([c + vec2(-3.5, 3.5) * z, c + vec2(3.0, -3.0) * z], ps);
                painter.line_segment([c + vec2(-4.5, 5.5) * z, c + vec2(1.0, 5.5) * z], ps);
                if resp.clicked() {
                    self.editing = Some((it.node.name.clone(), tab.left_top()));
                    self.edit_buf = nick.unwrap_or_default();
                    self.edit_first = true;
                }
            }
        }

        // Pass 4: the link currently being dragged.
        if let Some(from) = self.pending {
            let pointer = ui.input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos()));
            if let (Some(&a), Some(m)) = (anchors.get(&from.port), pointer) {
                let (s, e) = if from.from_out { (a, m) } else { (m, a) };
                let pts = layout::bezier_points((s.x, s.y), (e.x, e.y), 24, 40.0 * z)
                    .into_iter()
                    .map(|(x, y)| pos2(x, y))
                    .collect();
                painter.add(Shape::line(
                    pts,
                    Stroke::new((2.0 * z).clamp(1.0, 3.0), Color32::from_rgb(255, 220, 80)),
                ));
            }
        }

        // Nickname editor: Enter or clicking away saves, Escape cancels.
        if let Some((name, pos)) = self.editing.clone() {
            let mut done: Option<bool> = None;
            egui::Area::new(egui::Id::new("nickname_editor"))
                .order(egui::Order::Foreground)
                .fixed_pos(pos)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut self.edit_buf)
                                .hint_text("nickname")
                                .desired_width(160.0),
                        );
                        if self.edit_first {
                            resp.request_focus();
                            self.edit_first = false;
                        }
                        if resp.lost_focus() {
                            let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
                            done = Some(!esc);
                        }
                    });
                });
            if let Some(save) = done {
                if save {
                    let t = self.edit_buf.trim().to_string();
                    if t.is_empty() {
                        self.nicknames.remove(&name);
                    } else {
                        self.nicknames.insert(name, t);
                    }
                    self.dirty = true;
                }
                self.editing = None;
            }
        }

        action
    }

    /// Nearest port to `pos` that can accept a link dragged from `from`:
    /// opposite direction and on a different node.
    fn target_at(
        &self,
        pos: Pos2,
        anchors: &HashMap<u32, Pos2>,
        g: &Graph,
        from: Pending,
    ) -> Option<u32> {
        let src_node = g.ports.get(&from.port)?.node_id;
        let reach = 14.0 * self.zoom.max(0.7);
        anchors
            .iter()
            .filter(|(id, _)| {
                g.ports.get(id).is_some_and(|p| {
                    p.node_id != src_node && (p.direction == Direction::Out) != from.from_out
                })
            })
            .map(|(id, a)| (*id, a.distance(pos)))
            .filter(|(_, d)| *d <= reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    }
}
