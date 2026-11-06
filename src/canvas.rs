//! The node canvas: pan (drag background), zoom (scroll / pinch), drag nodes,
//! drag from port to port to connect, click a link + Delete to remove it.
//!
//! Link ownership: links RedoxRouter created ("owned") are always shown and
//! can be deleted. External links (made by other programs) are hidden unless
//! `show_external` is on or the pair is in `revealed`, and are read-only.

use crate::graph::{Direction, Graph, NodeInfo, PortInfo};
use crate::layout::{self, Placer, HEADER_H};
use crate::volume;
use std::collections::BTreeMap;
use crate::rules::{self, GroupEntry, RuleGroup, RuleSet};
use crate::storage::{LayoutFile, Note, NoteKind};
use eframe::egui::{
    self, pos2, vec2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Shape, Stroke,
    Vec2,
};
use std::collections::{BTreeSet, HashMap, HashSet};

/// Changes asked through a comment's / block's menu, applied after drawing.
enum NoteOp {
    Edit(u64),
    Delete(u64),
    Color(u64, [u8; 3]),
}

/// Right-click menu shared by comments and blocks.
fn note_menu(ui: &mut egui::Ui, id: u64, ops: &mut Vec<NoteOp>) {
    if ui.button("Edit text").clicked() {
        ops.push(NoteOp::Edit(id));
        ui.close_menu();
    }
    ui.menu_button("Color", |ui| {
        ui.horizontal(|ui| {
            for c in SWATCHES {
                let (sw, sresp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                ui.painter().rect_filled(sw, CornerRadius::same(3), Color32::from_rgb(c[0], c[1], c[2]));
                if sresp.clicked() {
                    ops.push(NoteOp::Color(id, c));
                    ui.close_menu();
                }
            }
        });
    });
    if ui.button("Delete").clicked() {
        ops.push(NoteOp::Delete(id));
        ui.close_menu();
    }
}

/// What clicking and box-selecting may pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectMode {
    All,
    Nodes,
    Links,
}

/// What the user asked the canvas to do; the app decides how to carry it out.
#[derive(Debug, Clone, Copy)]
pub enum Action {
    Connect { out_port: u32, in_port: u32 },
    Disconnect { out_port: u32, in_port: u32 },
    SetVolume { node_id: u32, linear: f32, channels: usize },
    SetMute { node_id: u32, mute: bool },
    DestroySink { node_id: u32 },
}

#[derive(Clone, Copy)]
struct Pending {
    port: u32,
    from_out: bool,
}

pub struct Canvas {
    /// Saved links (by name); filled by the app, persisted with the layout.
    pub links: Vec<crate::links::SavedLink>,
    /// Viewing an inactive setup: draw and edit its saved links instead of the live ones.
    pub virtual_mode: bool,
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
    /// Mode for hiding sink nodes: ShowAll, HideRedox, or HideAll.
    pub sink_hide_mode: crate::storage::SinkHideMode,
    pub link_style: layout::LinkStyle,
    /// What box select / click select may pick.
    pub select_mode: SelectMode,
    /// Box-selected owned links, nodes, and text comments.
    box_links: HashSet<(u32, u32)>,
    sel_nodes: HashSet<u32>,
    sel_notes: HashSet<u64>,
    /// Screen position where a Shift+drag box started.
    box_start: Option<Pos2>,
    /// Links the user asked to delete with several selected; the app drains it.
    pub extra_disconnects: Vec<(u32, u32)>,
    /// Comments and colored blocks.
    pub notes: Vec<Note>,
    note_edit: Option<u64>,
    /// Block whose contents were selected by double-clicking its title bar;
    /// dragging that bar then carries the selected nodes along.
    group_sel: Option<u64>,
    note_edit_first: bool,
    note_buf: String,
    /// Screen position of the last right-click on empty canvas.
    ctx_pos: Option<Pos2>,
    /// Audio bleed / auto-connection prevention mode.
    pub auto_connect_prevention: crate::storage::AutoConnectPrevention,
    /// External links the user chose to show individually ("visible only").
    pub revealed: HashSet<(u32, u32)>,
}

impl Default for Canvas {
    fn default() -> Self {
        Self {
            links: Vec::new(),
            virtual_mode: false,
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
            sink_hide_mode: crate::storage::SinkHideMode::ShowAll,
            link_style: layout::LinkStyle::Curved,
            select_mode: SelectMode::All,
            box_links: HashSet::new(),
            sel_nodes: HashSet::new(),
            sel_notes: HashSet::new(),
            box_start: None,
            extra_disconnects: Vec::new(),
            notes: Vec::new(),
            note_edit: None,
            group_sel: None,
            note_edit_first: false,
            note_buf: String::new(),
            ctx_pos: None,
            auto_connect_prevention: crate::storage::AutoConnectPrevention::Off,
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

/// Points spaced `spacing` apart along a polyline, shifted forward by `phase`
/// (used for the dots that flow along links).
fn dots_along(pts: &[Pos2], spacing: f32, phase: f32) -> Vec<Pos2> {
    let mut out = Vec::new();
    let mut next = phase.rem_euclid(spacing);
    let mut walked = 0.0;
    for w in pts.windows(2) {
        let seg = (w[1] - w[0]).length();
        if seg <= 0.0 {
            continue;
        }
        while next <= walked + seg {
            out.push(w[0] + (w[1] - w[0]) * ((next - walked) / seg));
            next += spacing;
        }
        walked += seg;
    }
    out
}

/// Screen-space path of a cable from output `a` through `wps` to input `b`.
/// `lane_id` (the output port id) staggers zigzag cables that share two nodes.
fn link_path(
    style: layout::LinkStyle,
    a: Pos2,
    wps: &[Pos2],
    b: Pos2,
    z: f32,
    lane_id: u32,
) -> Vec<Pos2> {
    let mut seq = vec![a];
    seq.extend_from_slice(wps);
    seq.push(b);
    let mut pts: Vec<(f32, f32)> = Vec::new();
    for w in seq.windows(2) {
        let mut p = piece(style, w[0], w[1], z, lane_id);
        if !pts.is_empty() {
            p.remove(0);
        }
        pts.extend(p);
    }
    if style == layout::LinkStyle::Zigzag {
        pts = layout::round_corners(&pts, 12.0 * z, 6);
    }
    pts.into_iter().map(|(x, y)| pos2(x, y)).collect()
}

/// Comment text (small markdown subset) as a styled layout job.
fn markdown_job(src: &str, size: f32, wrap: f32) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let normal = Color32::from_gray(232);
    for (n, line) in crate::markdown::parse(src).iter().enumerate() {
        if n > 0 {
            job.append("\n", 0.0, TextFormat { font_id: FontId::proportional(size), ..Default::default() });
        }
        let scale = match line.heading {
            1 => 1.4,
            2 => 1.22,
            3 => 1.08,
            _ => 1.0,
        };
        if line.bullet {
            job.append("•  ", 0.0, TextFormat { font_id: FontId::proportional(size), color: normal, ..Default::default() });
        }
        for sp in &line.spans {
            let strong = sp.bold || line.heading > 0;
            let mut f = TextFormat {
                font_id: if sp.code {
                    FontId::monospace(size * 0.95)
                } else {
                    FontId::proportional(size * scale)
                },
                color: if sp.code {
                    Color32::from_rgb(255, 232, 170)
                } else if strong {
                    Color32::WHITE
                } else {
                    normal
                },
                italics: sp.italic,
                ..Default::default()
            };
            if sp.code {
                f.background = Color32::from_black_alpha(90);
            }
            if sp.strike {
                f.strikethrough = Stroke::new(1.0_f32, normal);
            }
            if strong {
                f.extra_letter_spacing = size * 0.04; // slightly wider = reads as bold
                f.underline = if line.heading == 1 { Stroke::new(1.0_f32, Color32::from_white_alpha(90)) } else { Stroke::NONE };
            }
            job.append(&sp.text, 0.0, f);
        }
    }
    job
}

/// Screen rect and laid-out galley of a sticky text comment (`NoteKind::Text`).
fn comment_rect_and_galley(
    painter: &egui::Painter,
    origin: Pos2,
    z: f32,
    n: &Note,
) -> (Rect, std::sync::Arc<egui::Galley>) {
    let width = (n.size.0 * z).max(40.0);
    let wrap_w = (width - 16.0 * z).max(10.0);
    let galley = if n.text.is_empty() {
        painter.layout(
            "(empty comment: double-click to edit)".to_string(),
            FontId::proportional(12.0 * z),
            Color32::from_gray(200),
            wrap_w,
        )
    } else {
        painter.layout_job(markdown_job(&n.text, 12.0 * z, wrap_w))
    };
    let r = Rect::from_min_size(
        origin + vec2(n.pos.0, n.pos.1) * z,
        vec2(width, galley.size().y + 14.0 * z),
    );
    (r, galley)
}

/// One piece of a cable between two points (corner points for zigzag).
fn piece(style: layout::LinkStyle, a: Pos2, b: Pos2, z: f32, lane_id: u32) -> Vec<(f32, f32)> {
    match style {
        layout::LinkStyle::Curved => layout::bezier_points((a.x, a.y), (b.x, b.y), 24, 40.0 * z),
        layout::LinkStyle::Zigzag => {
            let lane = ((lane_id % 7) as f32 - 3.0) * 5.0 * z;
            layout::zigzag_points((a.x, a.y), (b.x, b.y), 22.0 * z, lane)
        }
    }
}

/// Which gap between a, the waypoints and b a click at `pos` belongs to.
fn waypoint_slot(
    style: layout::LinkStyle,
    a: Pos2,
    wps: &[Pos2],
    b: Pos2,
    pos: Pos2,
    z: f32,
    lane_id: u32,
) -> usize {
    let mut seq = vec![a];
    seq.extend_from_slice(wps);
    seq.push(b);
    seq.windows(2)
        .enumerate()
        .map(|(k, w)| {
            let pts = piece(style, w[0], w[1], z, lane_id);
            (k, layout::dist_to_polyline((pos.x, pos.y), &pts))
        })
        .min_by(|x, y| x.1.total_cmp(&y.1))
        .map_or(0, |(k, _)| k)
}

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
        let sink_hide_mode = if f.sink_hide_mode != crate::storage::SinkHideMode::ShowAll {
            f.sink_hide_mode
        } else if f.hide_all_sinks {
            crate::storage::SinkHideMode::HideAll
        } else {
            f.sink_hide_mode
        };
        Self {
            pan: vec2(f.pan.0, f.pan.1),
            zoom: f.zoom.clamp(layout::MIN_ZOOM, layout::MAX_ZOOM),
            saved: f.nodes.into_iter().collect(),
            hidden: f.hidden,
            saved_w: f.widths.into_iter().collect(),
            nicknames: f.nicknames,
            links: f.links,
            sink_hide_mode,
            link_style: f.link_style,
            show_external: f.show_external,
            notes: f.notes,
            auto_connect_prevention: f.auto_connect_prevention,
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
            links: self.links.clone(),
            sink_hide_mode: self.sink_hide_mode,
            link_style: self.link_style,
            show_external: self.show_external,
            notes: self.notes.clone(),
            auto_connect_prevention: self.auto_connect_prevention,
            hide_all_sinks: self.sink_hide_mode == crate::storage::SinkHideMode::HideAll,
        }
    }

    /// Move every selected node and text comment by `d` (world units), except `skip_node` / `skip_note`.
    fn move_selected(&mut self, g: &Graph, d: Vec2, skip_node: Option<u32>, skip_note: Option<u64>) {
        let ids: Vec<u32> = self.sel_nodes.iter().copied().filter(|&o| Some(o) != skip_node).collect();
        for o in ids {
            let Some(op) = self.pos.get_mut(&o) else { continue };
            *op = (op.0 + d.x, op.1 + d.y);
            let np = *op;
            if let Some(k) = g.nodes.get(&o).map(|n| n.key()) {
                let alive = |id| g.nodes.contains_key(&id);
                if layout::try_claim(&mut self.claimed, alive, &k, o) {
                    self.saved.insert(k, np);
                }
            }
        }
        for note in &mut self.notes {
            if note.kind == NoteKind::Text
                && self.sel_notes.contains(&note.id)
                && Some(note.id) != skip_note
            {
                note.pos = (note.pos.0 + d.x, note.pos.1 + d.y);
            }
        }
    }

    pub fn hide(&mut self, name: String) {
        self.hidden.insert(name);
        self.dirty = true;
    }

    pub fn unhide(&mut self, name: &str) {
        self.hidden.remove(name);
        if let Some((bare, _)) = name.split_once('|') {
            self.hidden.remove(bare);
        }
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
        painter.rect_filled(rect, CornerRadius::same(0), Color32::from_rgb(22, 23, 28));
        {
            // Dot grid that follows pan and zoom (spacing doubles when too dense).
            let mut step = 28.0 * self.zoom;
            while step < 18.0 {
                step *= 2.0;
            }
            let origin = rect.min + self.pan;
            let x0 = rect.min.x + (origin.x - rect.min.x).rem_euclid(step);
            let y0 = rect.min.y + (origin.y - rect.min.y).rem_euclid(step);
            let dot = Color32::from_rgb(44, 46, 56);
            let mut y = y0;
            while y < rect.max.y {
                let mut x = x0;
                while x < rect.max.x {
                    painter.circle_filled(pos2(x, y), 1.1, dot);
                    x += step;
                }
                y += step;
            }
        }

        // Pan.
        if bg.drag_started() && ui.input(|i| i.modifiers.shift) {
            self.box_start = bg.interact_pointer_pos();
        }
        if bg.dragged() && self.box_start.is_none() {
            self.pan += bg.drag_delta();
        }
        if bg.drag_stopped() && self.box_start.is_none() {
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

        // Right-click on empty canvas: add a comment or a block there.
        if bg.secondary_clicked() {
            self.ctx_pos = bg.interact_pointer_pos();
        }
        let mut new_note: Option<NoteKind> = None;
        bg.context_menu(|ui| {
            if ui.button("Add comment here").clicked() {
                new_note = Some(NoteKind::Text);
                ui.close_menu();
            }
            if ui.button("Add block here").clicked() {
                new_note = Some(NoteKind::Block);
                ui.close_menu();
            }
        });
        if let (Some(kind), Some(p)) = (new_note, self.ctx_pos) {
            let w = (p - origin) / z;
            let id = self.notes.iter().map(|n| n.id).max().unwrap_or(0) + 1;
            let (size, text, color) = match kind {
                NoteKind::Text => ((200.0, 0.0), String::new(), SWATCHES[7]),
                NoteKind::Block => ((340.0, 220.0), "Group".to_string(), SWATCHES[6]),
            };
            self.note_buf = text.clone();
            self.notes.push(Note { id, kind, pos: (w.x, w.y), size, text, color });
            self.note_edit = Some(id);
            self.note_edit_first = true;
            self.dirty = true;
        }

        // Blocks: translucent colored areas behind links and nodes. Only the
        // title bar is grabbed, so links and nodes over a block stay usable.
        let mut ops: Vec<NoteOp> = Vec::new();
        let mut select_group: Option<u64> = None;
        for i in 0..self.notes.len() {
            if self.notes[i].kind != NoteKind::Block {
                continue;
            }
            let n = self.notes[i].clone();
            let r = Rect::from_min_size(origin + vec2(n.pos.0, n.pos.1) * z, vec2(n.size.0, n.size.1) * z);
            if !r.intersects(rect) {
                continue;
            }
            let c = n.color;
            painter.rect_filled(r, CornerRadius::same(8), Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 40));
            painter.rect_stroke(
                r,
                CornerRadius::same(8),
                Stroke::new(1.5_f32, Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 150)),
                egui::StrokeKind::Inside,
            );
            let bar = Rect::from_min_size(r.min, vec2(r.width(), 22.0 * z));
            painter.rect_filled(
                bar,
                CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 },
                Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 110),
            );
            if z >= 0.4 {
                painter.with_clip_rect(bar.intersect(rect)).text(
                    bar.left_center() + vec2(8.0 * z, 0.0),
                    egui::Align2::LEFT_CENTER,
                    &n.text,
                    FontId::proportional(12.0 * z),
                    Color32::WHITE,
                );
            }
            let resp = ui
                .interact(bar, ui.id().with(("note", n.id)), Sense::click_and_drag())
                .on_hover_cursor(egui::CursorIcon::Grab)
                .on_hover_text("Drag: move the block. Double-click: select the nodes and comments inside, then drag to move them too");
            if resp.dragged() {
                let d = resp.drag_delta() / z;
                self.notes[i].pos = (n.pos.0 + d.x, n.pos.1 + d.y);
                if self.group_sel == Some(n.id) {
                    self.move_selected(g, d, None, None);
                }
            }
            if resp.drag_stopped() {
                self.dirty = true;
            }
            if resp.double_clicked() {
                select_group = Some(n.id);
            }
            resp.context_menu(|ui| note_menu(ui, n.id, &mut ops));
            let grip = Rect::from_min_max(r.max - vec2(14.0, 14.0) * z.max(0.8), r.max);
            let gresp = ui
                .interact(grip, ui.id().with(("note_grip", n.id)), Sense::drag())
                .on_hover_cursor(egui::CursorIcon::ResizeNwSe);
            if gresp.dragged() {
                let d = gresp.drag_delta() / z;
                self.notes[i].size = ((n.size.0 + d.x).max(80.0), (n.size.1 + d.y).max(40.0));
            }
            if gresp.drag_stopped() {
                self.dirty = true;
            }
            let gs = Stroke::new((1.5 * z).max(1.0), Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 200));
            painter.line_segment([r.max + vec2(-10.0, -2.0) * z, r.max + vec2(-2.0, -10.0) * z], gs);
            painter.line_segment([r.max + vec2(-6.0, -2.0) * z, r.max + vec2(-2.0, -6.0) * z], gs);
        }
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
            let key = (!node.name.is_empty()).then(|| node.key());
            if key.as_ref().is_some_and(|k| self.hidden.contains(k) || self.hidden.contains(&node.name)) {
                continue;
            }
            let nickname = self
                .nicknames
                .get(&node.key())
                .or_else(|| self.nicknames.get(&node.name))
                .map_or("", |s| s.as_str());
            let text = rules::NodeText {
                title: node.label(),
                nickname,
                name: &node.name,
                description: &node.description,
                media_class: &node.media_class,
            };
            let is_sink = node.is_sink();
            let is_redox = g.is_node_owned(node);
            let default_hidden = match self.sink_hide_mode {
                crate::storage::SinkHideMode::ShowAll => false,
                crate::storage::SinkHideMode::HideRedox => is_sink && is_redox,
                crate::storage::SinkHideMode::HideAll => is_sink,
            };
            let outcome = rules::evaluate(self.rules.active_rules(), &text, default_hidden);
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
            // A manual width (saved by name/key) overrides the automatic one.
            if !self.width.contains_key(&node.id) {
                if let Some(ref k) = key {
                    if let Some(&sw) = self.saved_w.get(k).or_else(|| self.saved_w.get(&node.name)) {
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
            let has_vol = g.volume_of(node).is_some();
            let mut all_rows = row_h.clone();
            if has_vol {
                all_rows.push(layout::VOL_ROW_H);
            }
            let (h, centers) = layout::node_rows(header_h, &all_rows);
            let vol_y = has_vol.then(|| centers[row_h.len()]);
            let col = layout::column_for(&node.media_class);
            // Saved nodes are matched by key (or legacy name; ids change between sessions).
            let mut p = match self.pos.get(&node.id) {
                Some(&p) => p,
                None => {
                    let saved = key.as_ref().and_then(|k| {
                        layout::claim_saved(
                            &self.saved,
                            &mut self.claimed,
                            |id| g.nodes.contains_key(&id),
                            k,
                            node.id,
                        )
                        .or_else(|| {
                            layout::claim_saved(
                                &self.saved,
                                &mut self.claimed,
                                |id| g.nodes.contains_key(&id),
                                &node.name,
                                node.id,
                            )
                        })
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
                if let Some(ref k) = key {
                    resp.context_menu(|ui| {
                        if ui.button("Hide node").clicked() {
                            to_hide.push(k.clone());
                            ui.close_menu();
                        }
                        if node.is_sink() {
                            if ui.button("Destroy sink").clicked() {
                                action = Some(Action::DestroySink { node_id: node.id });
                                ui.close_menu();
                            }
                        } else if node.is_source() {
                            if ui.button("Destroy mic").clicked() {
                                action = Some(Action::DestroySink { node_id: node.id });
                                ui.close_menu();
                            }
                        }
                        ui.menu_button("Color", |ui| {
                            ui.horizontal(|ui| {
                                for c in SWATCHES {
                                    let (sw, sresp) =
                                        ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                                    let fill = Color32::from_rgb(c[0], c[1], c[2]);
                                    ui.painter().rect_filled(sw, CornerRadius::same(3), fill);
                                    if sresp.clicked() {
                                        color_set.push((k.clone(), c));
                                        ui.close_menu();
                                    }
                                }
                            });
                            if ui.button("Reset color").clicked() {
                                color_clear.push(k.clone());
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
                if resp.clicked() && self.select_mode != SelectMode::Links {
                    let add = ui.input(|i| i.modifiers.shift);
                    if !add {
                        self.sel_nodes.clear();
                        self.sel_notes.clear();
                        self.group_sel = None;
                    }
                    self.sel_nodes.insert(node.id);
                    self.box_links.clear();
                    self.selected = None;
                }
                if resp.dragged() {
                    let d = resp.drag_delta() / z;
                    p = (p.0 + d.x, p.1 + d.y);
                    self.pos.insert(node.id, p);
                    // Dragging one selected node moves the whole selection.
                    if self.sel_nodes.contains(&node.id) {
                        self.move_selected(g, d, Some(node.id), None);
                    }
                    if let Some(ref k) = key {
                        let alive = |id| g.nodes.contains_key(&id);
                        if layout::try_claim(&mut self.claimed, alive, k, node.id) {
                            self.saved.insert(k.clone(), p);
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
                    if let Some(ref k) = key {
                        let alive = |id| g.nodes.contains_key(&id);
                        if layout::try_claim(&mut self.claimed, alive, k, node.id) {
                            self.saved_w.insert(k.clone(), nw);
                        }
                    }
                }
                if resp.double_clicked() {
                    self.width.remove(&node.id);
                    if let Some(ref k) = key {
                        self.saved_w.remove(k);
                        self.saved_w.remove(&node.name);
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

        // Double-click on a block title: select the nodes and text comments fully inside it.
        if let Some(bid) = select_group {
            if let Some(n) = self.notes.iter().find(|n| n.id == bid) {
                let br = Rect::from_min_size(origin + vec2(n.pos.0, n.pos.1) * z, vec2(n.size.0, n.size.1) * z);
                self.sel_nodes.clear();
                self.sel_notes.clear();
                self.box_links.clear();
                self.selected = None;
                for it in &items {
                    if br.contains_rect(it.rect) {
                        self.sel_nodes.insert(it.node.id);
                    }
                }
                for note in &self.notes {
                    if note.kind == NoteKind::Text {
                        let (cr, _) = comment_rect_and_galley(&painter, origin, z, note);
                        if br.contains_rect(cr) {
                            self.sel_notes.insert(note.id);
                        }
                    }
                }
                self.group_sel = Some(bid);
            }
        }

        // Pass 2: links (under the nodes). External ones only when revealed.
        let mut shapes: Vec<((u32, u32), Vec<Pos2>)> = Vec::new();
        let pairs: Vec<((u32, u32), bool)> = if self.virtual_mode {
            let mut v: Vec<((u32, u32), bool)> = self
                .links
                .iter()
                .filter(|l| l.enabled)
                .filter_map(|l| crate::links::resolve(g, l))
                .map(|p| (p, true))
                .collect();
            for l in g.links.values().filter(|l| !g.is_owned(l)) {
                let p = (l.out_port, l.in_port);
                if !v.iter().any(|&(existing, _)| existing == p) {
                    v.push((p, false));
                }
            }
            v
        } else {
            g.links.values().map(|l| ((l.out_port, l.in_port), g.is_owned(l))).collect()
        };
        let time = ui.input(|i| i.time) as f32;
        // Saved links that have waypoints, by live port pair.
        let mut wp_of: HashMap<(u32, u32), usize> = HashMap::new();
        for (i, l) in self.links.iter().enumerate() {
            if !l.waypoints.is_empty() {
                if let Some(p) = crate::links::resolve(g, l) {
                    wp_of.insert(p, i);
                }
            }
        }
        let to_screen = |w: (f32, f32)| origin + vec2(w.0, w.1) * z;
        let mut selectable: HashSet<(u32, u32)> = HashSet::new();
        let mut handles: Vec<(usize, usize, Pos2)> = Vec::new();
        if !pairs.is_empty() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        }
        for (pair, owned) in pairs {
            if !(owned || self.show_external || self.revealed.contains(&pair)) {
                continue;
            }
            let (Some(a), Some(b)) = (anchors.get(&pair.0), anchors.get(&pair.1)) else {
                continue;
            };
            let wps: Vec<Pos2> = wp_of
                .get(&pair)
                .map(|&i| self.links[i].waypoints.iter().map(|&w| to_screen(w)).collect())
                .unwrap_or_default();
            if owned {
                selectable.insert(pair);
            }
            let picked = self.selected == Some(pair) || self.box_links.contains(&pair);
            if picked {
                if let Some(&i) = wp_of.get(&pair) {
                    handles.extend(wps.iter().enumerate().map(|(k, p)| (i, k, *p)));
                }
            }
            let pts = link_path(self.link_style, *a, &wps, *b, z, pair.0);
            let stroke = if picked {
                Stroke::new((3.0 * z).clamp(2.0, 4.5), Color32::from_rgb(255, 206, 84))
            } else if owned {
                Stroke::new((2.2 * z).clamp(1.2, 3.2), Color32::from_rgb(92, 196, 230))
            } else {
                Stroke::new(
                    (1.5 * z).clamp(1.0, 2.5),
                    Color32::from_rgba_unmultiplied(150, 150, 165, 170),
                )
            };
            {
                // Soft glow under every link (grey for external ones).
                let c = stroke.color;
                let glow = Stroke::new(
                    stroke.width * 3.2,
                    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), 30),
                );
                painter.add(Shape::line(pts.clone(), glow));
            }
            painter.add(Shape::line(pts.clone(), stroke));
            // Dots flowing from the output port to the input port.
            let c = stroke.color;
            let dot = Color32::from_rgba_unmultiplied(
                c.r().saturating_add(70),
                c.g().saturating_add(70),
                c.b().saturating_add(70),
                230,
            );
            let r_dot = (stroke.width * 0.75).clamp(1.5, 3.0);
            for p in dots_along(&pts, 26.0, time * 38.0) {
                painter.circle_filled(p, r_dot, dot);
            }
            shapes.push((pair, pts));
        }

        // Waypoint handles of picked links: drag to move, double-click to remove.
        let mut handle_hit = false;
        let mut remove_wp: Option<(usize, usize)> = None;
        for (i, k, p) in handles {
            let size = vec2(16.0, 16.0) * z.max(0.8);
            let resp = ui.interact(
                Rect::from_center_size(p, size),
                ui.id().with(("wp", i, k)),
                Sense::click_and_drag(),
            );
            handle_hit |= resp.hovered() || resp.dragged();
            if resp.dragged() {
                let d = resp.drag_delta() / z;
                if let Some(w) = self.links.get_mut(i).and_then(|l| l.waypoints.get_mut(k)) {
                    *w = (w.0 + d.x, w.1 + d.y);
                    self.dirty = true;
                }
            }
            if resp.double_clicked() {
                remove_wp = Some((i, k));
            }
            let hot = resp.hovered() || resp.dragged();
            painter.circle(
                p,
                (5.0 * z).clamp(4.0, 7.0),
                if hot { Color32::WHITE } else { Color32::from_gray(225) },
                Stroke::new(2.0_f32, Color32::from_rgb(255, 206, 84)),
            );
        }
        if let Some((i, k)) = remove_wp {
            if let Some(l) = self.links.get_mut(i) {
                if k < l.waypoints.len() {
                    l.waypoints.remove(k);
                    self.dirty = true;
                }
            }
        }
        // Double-click on one of our links adds a waypoint there.
        if bg.double_clicked() && !handle_hit {
            if let Some(pos) = bg.interact_pointer_pos() {
                let hit = shapes
                    .iter()
                    .filter(|(pair, _)| selectable.contains(pair))
                    .map(|(pair, pts)| {
                        let line: Vec<(f32, f32)> = pts.iter().map(|p| (p.x, p.y)).collect();
                        (*pair, layout::dist_to_polyline((pos.x, pos.y), &line))
                    })
                    .filter(|(_, d)| *d <= 8.0)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(pair, _)| pair);
                if let (Some(pair), true) = (hit, self.select_mode != SelectMode::Nodes) {
                    let idx = crate::links::from_ports(g, pair.0, pair.1)
                        .and_then(|s| crate::links::find(&self.links, &s));
                    if let (Some(i), Some(&a), Some(&b)) =
                        (idx, anchors.get(&pair.0), anchors.get(&pair.1))
                    {
                        let existing: Vec<Pos2> =
                            self.links[i].waypoints.iter().map(|&w| to_screen(w)).collect();
                        let k = waypoint_slot(self.link_style, a, &existing, b, pos, z, pair.0);
                        let world = (pos - origin) / z;
                        self.links[i].waypoints.insert(k, (world.x, world.y));
                        self.selected = Some(pair);
                        self.dirty = true;
                    }
                }
            }
        }

        // Shift+drag box: pick nodes and/or our own links inside it.
        if let (Some(start), true) = (self.box_start, bg.drag_stopped()) {
            let end = ui.input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos()));
            if let Some(end) = end {
                let bx = Rect::from_two_pos(start, end);
                let add = ui.input(|i| i.modifiers.ctrl);
                if !add {
                    self.sel_nodes.clear();
                    self.sel_notes.clear();
                    self.box_links.clear();
                    self.selected = None;
                    self.group_sel = None;
                }
                if self.select_mode != SelectMode::Links {
                    for it in &items {
                        if it.rect.intersects(bx) {
                            self.sel_nodes.insert(it.node.id);
                        }
                    }
                    for note in &self.notes {
                        if note.kind == NoteKind::Text {
                            let (cr, _) = comment_rect_and_galley(&painter, origin, z, note);
                            if cr.intersects(bx) {
                                self.sel_notes.insert(note.id);
                            }
                        }
                    }
                }
                if self.select_mode != SelectMode::Nodes {
                    for (pair, pts) in &shapes {
                        if selectable.contains(pair) && pts.iter().any(|p| bx.contains(*p)) {
                            self.box_links.insert(*pair);
                        }
                    }
                }
            }
            self.box_start = None;
        }
        // Drop selections that no longer exist.
        self.box_links.retain(|p| selectable.contains(p));
        self.sel_nodes.retain(|id| g.nodes.contains_key(id));
        self.sel_notes.retain(|id| self.notes.iter().any(|n| n.id == *id && n.kind == NoteKind::Text));
        if ui.input(|i| i.key_pressed(egui::Key::Delete)) && !self.box_links.is_empty() {
            self.extra_disconnects.extend(self.box_links.drain());
        }

        // Click selects the nearest link; click on empty space clears.
        if bg.clicked() {
            self.sel_nodes.clear();
            self.sel_notes.clear();
            self.box_links.clear();
            self.group_sel = None;
        }
        if bg.clicked() && self.select_mode == SelectMode::Nodes {
            self.selected = None;
        } else if bg.clicked() {
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
            let gone = !self.virtual_mode && g.find_link(pair.0, pair.1).is_none();
            if gone || !shapes.iter().any(|(p, _)| *p == pair) {
                self.selected = None;
            } else if ui.input(|i| i.key_pressed(egui::Key::Delete)) {
                let owned = self.virtual_mode
                    || g.find_link(pair.0, pair.1).is_some_and(|l| g.is_owned(l));
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
                0 => Color32::from_rgb(46, 125, 98),
                2 => Color32::from_rgb(176, 104, 52),
                _ => Color32::from_rgb(84, 88, 112),
            };
            let header_color = it.color.unwrap_or(default_color);
            // Soft shadow, thin border, body, then the colored header with a
            // subtle gloss and a divider line.
            painter.rect_filled(
                r.translate(vec2(0.0, 3.0 * z)).expand(3.0 * z),
                CornerRadius::same(9),
                Color32::from_black_alpha(55),
            );
            if self.sel_nodes.contains(&it.node.id) {
                painter.rect_filled(
                    r.expand(3.0 * z),
                    CornerRadius::same(9),
                    Color32::from_rgba_unmultiplied(92, 196, 230, 150),
                );
            }
            painter.rect_filled(r.expand(z), CornerRadius::same(7), Color32::from_gray(72));
            // Nickname tab above the top-left corner, drawn BEFORE the node body and
            // reaching down behind the header, so it looks fused to the node and a
            // node drawn on top of it covers both. Click the pencil to rename.
            let show_text = z >= 0.4;
            if !it.node.name.is_empty() {
                let nkey = it.node.key();
                let nick = self
                    .nicknames
                    .get(&nkey)
                    .or_else(|| self.nicknames.get(&it.node.name))
                    .cloned();
                let tab_h = layout::TAB_H * z;
                let lift = 3.0 * z; // sits a little higher above the header
                let text_g = nick.as_ref().filter(|_| show_text).map(|n| {
                    painter.layout_no_wrap(n.clone(), FontId::proportional(11.0 * z), Color32::WHITE)
                });
                let text_w = text_g.as_ref().map_or(0.0, |g| g.size().x + 6.0 * z);
                let tab_w = (text_w + 22.0 * z).min(r.width());
                let top_y = r.min.y - tab_h - lift;
                // Visible part (for text, pencil and clicks) and the part hidden
                // behind the node.
                let tab = Rect::from_min_size(pos2(r.min.x, top_y), vec2(tab_w, tab_h + lift));
                let full = Rect::from_min_max(tab.min, pos2(tab.max.x, r.min.y + 14.0 * z));
                let resp = ui
                    .interact(tab, ui.id().with(("tab", it.node.id)), Sense::click())
                    .on_hover_text("Rename (nickname)");
                let fill = if resp.hovered() { lighten(header_color) } else { header_color };
                let top = CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 };
                painter.rect_filled(full.expand(z), top, Color32::from_gray(72));
                painter.rect_filled(full, top, fill);
                painter.rect_filled(
                    Rect::from_min_size(tab.min, vec2(tab.width(), tab.height() / 2.0)),
                    top,
                    Color32::from_white_alpha(16),
                );
                if let Some(g) = text_g {
                    let pos = tab.min + vec2(6.0 * z, (tab.height() - g.size().y) / 2.0);
                    painter.with_clip_rect(tab).galley(pos, g, Color32::WHITE);
                }
                // Pencil, drawn with lines so it needs no special font.
                let c = pos2(tab.max.x - 11.0 * z, tab.center().y);
                let ps = Stroke::new((1.6 * z).max(1.0), Color32::from_gray(205));
                painter.line_segment([c + vec2(-3.5, 3.5) * z, c + vec2(3.0, -3.0) * z], ps);
                painter.line_segment([c + vec2(-4.5, 5.5) * z, c + vec2(1.0, 5.5) * z], ps);
                if resp.clicked() {
                    self.editing = Some((nkey, tab.left_top()));
                    self.edit_buf = nick.unwrap_or_default();
                    self.edit_first = true;
                }
            }
            painter.rect_filled(r, CornerRadius::same(6), Color32::from_rgb(38, 40, 48));
            let header = Rect::from_min_size(r.min, vec2(r.width(), it.header_h * z));
            let top_round = CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 };
            painter.rect_filled(header, top_round, header_color);
            painter.rect_filled(
                Rect::from_min_size(header.min, vec2(header.width(), header.height() / 2.0)),
                top_round,
                Color32::from_white_alpha(16),
            );
            painter.line_segment(
                [header.left_bottom(), header.right_bottom()],
                Stroke::new(1.0_f32, Color32::from_black_alpha(90)),
            );

            let clip = painter.with_clip_rect(r.intersect(rect));
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
                painter.circle(a, 4.5 * z, color, Stroke::new(1.5 * z, Color32::from_rgb(24, 25, 30)));
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
            if let (Some(cy), Some(vol)) = (it.vol_y, g.volume_of(it.node)) {
                // Streams get a slider; devices (sound cards, virtual sinks) only a
                // read-only bar, they are controlled by the system volume control.
                let can_edit = it.node.is_stream();
                let s = volume::linear_to_slider(vol.effective()).min(volume::SLIDER_MAX);
                let top = r.min.y + (cy - layout::VOL_ROW_H / 2.0) * z;
                let bottom = r.min.y + (cy + layout::VOL_ROW_H / 2.0) * z;
                let track = Rect::from_min_max(
                    pos2(r.min.x + 10.0 * z, r.min.y + (cy - 3.0) * z),
                    pos2(r.max.x - 56.0 * z, r.min.y + (cy + 3.0) * z),
                );
                let hit = track.expand2(vec2(6.0 * z, 7.0 * z));
                let sense = if can_edit { Sense::click_and_drag() } else { Sense::click() };
                let resp = ui.interact(hit, ui.id().with(("vol", it.node.id)), sense);
                if !can_edit {
                    resp.clone().on_hover_text("Device volume: change it in your system volume control");
                }
                let mut new_s = None;
                if can_edit && (resp.dragged() || resp.clicked()) {
                    if let Some(p) = resp.interact_pointer_pos() {
                        let t = ((p.x - track.min.x) / track.width()).clamp(0.0, 1.0);
                        new_s = Some(t * volume::SLIDER_MAX);
                    }
                }
                if can_edit && resp.double_clicked() {
                    new_s = Some(1.0);
                }
                if let Some(ns) = new_s.filter(|ns| (ns - s).abs() > 0.004) {
                    if action.is_none() {
                        action = Some(Action::SetVolume {
                            node_id: it.node.id,
                            linear: volume::slider_to_linear(ns),
                            channels: vol.channels.len(),
                        });
                    }
                }

                let (track_col, fill_col) = if vol.mute {
                    (Color32::from_gray(55), Color32::from_gray(100))
                } else {
                    (Color32::from_gray(58), Color32::from_rgb(92, 196, 230))
                };
                painter.rect_filled(track, CornerRadius::same(3), track_col);
                let frac = (s / volume::SLIDER_MAX).clamp(0.0, 1.0);
                let filled =
                    Rect::from_min_max(track.min, pos2(track.min.x + track.width() * frac, track.max.y));
                painter.rect_filled(filled, CornerRadius::same(3), fill_col);
                if can_edit {
                    painter.circle(
                        pos2(filled.max.x, track.center().y),
                        5.5 * z,
                        Color32::from_gray(238),
                        Stroke::new(1.5 * z, Color32::from_black_alpha(110)),
                    );
                }

                let label = Rect::from_min_max(pos2(track.max.x + 8.0 * z, top), pos2(r.max.x - 4.0 * z, bottom));
                let lresp = ui
                    .interact(label, ui.id().with(("mute", it.node.id)), Sense::click())
                    .on_hover_text(if can_edit {
                        "Click to mute / unmute"
                    } else {
                        "Device volume: change it in your system volume control"
                    });
                if can_edit && lresp.clicked() && action.is_none() {
                    action = Some(Action::SetMute { node_id: it.node.id, mute: !vol.mute });
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

        }

        // Comments: sticky text cards above the nodes.
        for i in 0..self.notes.len() {
            if self.notes[i].kind != NoteKind::Text {
                continue;
            }
            let n = self.notes[i].clone();
            let c = n.color;
            let (r, galley) = comment_rect_and_galley(&painter, origin, z, &n);
            if !r.intersects(rect) {
                continue;
            }
            painter.rect_filled(
                r.translate(vec2(0.0, 3.0 * z)).expand(2.0 * z),
                CornerRadius::same(8),
                Color32::from_black_alpha(55),
            );
            if self.sel_notes.contains(&n.id) {
                painter.rect_filled(
                    r.expand(3.0 * z),
                    CornerRadius::same(9),
                    Color32::from_rgba_unmultiplied(92, 196, 230, 150),
                );
            }
            painter.rect_filled(r, CornerRadius::same(6), Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 235));
            painter.rect_filled(
                Rect::from_min_size(r.min, vec2(r.width(), 5.0 * z)),
                CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 },
                Color32::from_white_alpha(40),
            );
            if z >= 0.4 {
                painter
                    .with_clip_rect(r.intersect(rect))
                    .galley(r.min + vec2(8.0 * z, 8.0 * z), galley, Color32::WHITE);
            }
            let resp = ui
                .interact(r, ui.id().with(("note", n.id)), Sense::click_and_drag())
                .on_hover_cursor(egui::CursorIcon::Grab);
            if resp.clicked() && self.select_mode != SelectMode::Links {
                let add = ui.input(|i| i.modifiers.shift);
                if !add {
                    self.sel_nodes.clear();
                    self.sel_notes.clear();
                    self.group_sel = None;
                }
                self.sel_notes.insert(n.id);
                self.box_links.clear();
                self.selected = None;
            }
            if resp.dragged() {
                let d = resp.drag_delta() / z;
                self.notes[i].pos = (n.pos.0 + d.x, n.pos.1 + d.y);
                if self.sel_notes.contains(&n.id) {
                    self.move_selected(g, d, None, Some(n.id));
                }
            }
            if resp.drag_stopped() {
                self.dirty = true;
            }
            if resp.double_clicked() {
                ops.push(NoteOp::Edit(n.id));
            }
            resp.context_menu(|ui| note_menu(ui, n.id, &mut ops));
            // Right edge: drag to change the width.
            let grip = Rect::from_min_max(pos2(r.max.x - 8.0 * z.max(0.8), r.min.y), r.max);
            let gresp = ui
                .interact(grip, ui.id().with(("note_grip", n.id)), Sense::drag())
                .on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
            if gresp.dragged() {
                let d = gresp.drag_delta().x / z;
                self.notes[i].size.0 = (n.size.0 + d).max(80.0);
            }
            if gresp.drag_stopped() {
                self.dirty = true;
            }
        }
        for op in ops {
            match op {
                NoteOp::Delete(id) => {
                    self.notes.retain(|n| n.id != id);
                    self.sel_notes.remove(&id);
                    self.dirty = true;
                }
                NoteOp::Color(id, c) => {
                    if let Some(n) = self.notes.iter_mut().find(|n| n.id == id) {
                        n.color = c;
                        self.dirty = true;
                    }
                }
                NoteOp::Edit(id) => {
                    if let Some(n) = self.notes.iter().find(|n| n.id == id) {
                        self.note_buf = n.text.clone();
                        self.note_edit = Some(id);
                        self.note_edit_first = true;
                    }
                }
            }
        }

        // Pass 4: the link currently being dragged.
        if let Some(from) = self.pending {
            let pointer = ui.input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos()));
            if let (Some(&a), Some(m)) = (anchors.get(&from.port), pointer) {
                let (s, e) = if from.from_out { (a, m) } else { (m, a) };
                let pts = link_path(self.link_style, s, &[], e, z, 0);
                painter.add(Shape::line(
                    pts,
                    Stroke::new((2.4 * z).clamp(1.2, 3.4), Color32::from_rgb(255, 206, 84)),
                ));
            }
        }

        // Comment / block text editor: click away saves, Escape cancels.
        if let Some(id) = self.note_edit {
            match self.notes.iter().position(|n| n.id == id) {
                None => self.note_edit = None,
                Some(idx) => {
                    let n = self.notes[idx].clone();
                    let at = origin + vec2(n.pos.0, n.pos.1) * z;
                    let mut done: Option<bool> = None;
                    egui::Area::new(egui::Id::new("note_editor"))
                        .order(egui::Order::Foreground)
                        .fixed_pos(at)
                        .show(ui.ctx(), |ui| {
                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                let resp = if n.kind == NoteKind::Text {
                                    ui.add(
                                        egui::TextEdit::multiline(&mut self.note_buf)
                                            .hint_text("comment (markdown: # title, - list, **bold**, *italic*, `code`)")
                                            .desired_rows(3)
                                            .desired_width(240.0),
                                    )
                                } else {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut self.note_buf)
                                            .hint_text("block title")
                                            .desired_width(240.0),
                                    )
                                };
                                if self.note_edit_first {
                                    resp.request_focus();
                                    self.note_edit_first = false;
                                }
                                if resp.lost_focus() {
                                    done = Some(!ui.input(|i| i.key_pressed(egui::Key::Escape)));
                                }
                            });
                        });
                    if let Some(save) = done {
                        if save {
                            self.notes[idx].text = self.note_buf.clone();
                            self.dirty = true;
                        }
                        self.note_edit = None;
                    }
                }
            }
        }

        // Box being dragged.
        if let Some(start) = self.box_start {
            if let Some(cur) = ui.input(|i| i.pointer.latest_pos()) {
                let bx = Rect::from_two_pos(start, cur);
                painter.rect_filled(bx, CornerRadius::same(2), Color32::from_rgba_unmultiplied(92, 196, 230, 30));
                painter.rect_stroke(
                    bx,
                    CornerRadius::same(2),
                    Stroke::new(1.0_f32, Color32::from_rgb(92, 196, 230)),
                    egui::StrokeKind::Inside,
                );
            }
        }

        // Bottom-right: what box select / clicks may pick.
        egui::Area::new(egui::Id::new("select_mode"))
            .anchor(egui::Align2::RIGHT_BOTTOM, vec2(-12.0, -12.0))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.weak("Select");
                        for (m, label) in [
                            (SelectMode::All, "All"),
                            (SelectMode::Nodes, "Nodes"),
                            (SelectMode::Links, "Links"),
                        ] {
                            ui.selectable_value(&mut self.select_mode, m, label);
                        }
                    });
                    ui.small("Shift+drag = box, Ctrl adds");
                });
            });
        match self.select_mode {
            SelectMode::Nodes => {
                self.box_links.clear();
                self.selected = None;
            }
            SelectMode::Links => {
                self.sel_nodes.clear();
                self.sel_notes.clear();
            }
            SelectMode::All => {}
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
                        if let Some((bare, _)) = name.split_once('|') {
                            self.nicknames.remove(bare);
                        }
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
