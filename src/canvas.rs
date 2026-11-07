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
use crate::theme::{GridPattern, SwitchGradMode, Theme};
use eframe::egui::{
    self, pos2, vec2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Shape, Stroke,
    Vec2,
};
use std::collections::{BTreeSet, HashMap, HashSet};

fn lerp_color32(c1: Color32, c2: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0_f32, 1.0_f32);
    let r = (c1.r() as f32 * (1.0 - t) + c2.r() as f32 * t).round() as u8;
    let g = (c1.g() as f32 * (1.0 - t) + c2.g() as f32 * t).round() as u8;
    let b = (c1.b() as f32 * (1.0 - t) + c2.b() as f32 * t).round() as u8;
    let a = (c1.a() as f32 * (1.0 - t) + c2.a() as f32 * t).round() as u8;
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

fn build_polyline_ribbon(
    pts: &[Pos2],
    width: f32,
    c_start: Color32,
    c_end: Color32,
    alpha_override: Option<u8>,
) -> egui::Mesh {
    let mut mesh = egui::Mesh::default();
    if pts.len() < 2 {
        return mesh;
    }
    let mut clean_pts: Vec<Pos2> = Vec::with_capacity(pts.len());
    for &p in pts {
        if let Some(&last) = clean_pts.last() {
            if (p - last).length_sq() < 1e-4 {
                continue;
            }
        }
        clean_pts.push(p);
    }
    let n = clean_pts.len();
    if n < 2 {
        return mesh;
    }
    let half_w = width * 0.5;

    let mut dirs: Vec<Vec2> = Vec::with_capacity(n - 1);
    for w in clean_pts.windows(2) {
        let delta = w[1] - w[0];
        let len = delta.length();
        dirs.push(if len > 1e-4 { delta / len } else { vec2(1.0, 0.0) });
    }

    for i in 0..n {
        let t = i as f32 / (n - 1) as f32;
        let mut color = lerp_color32(c_start, c_end, t);
        if let Some(a) = alpha_override {
            color = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), a);
        }

        let normal = if i == 0 {
            vec2(-dirs[0].y, dirs[0].x)
        } else if i == n - 1 {
            vec2(-dirs[n - 2].y, dirs[n - 2].x)
        } else {
            let d1 = dirs[i - 1];
            let d2 = dirs[i];
            let avg = d1 + d2;
            let avg_len = avg.length();
            if avg_len > 1e-4 {
                let norm = vec2(-avg.y / avg_len, avg.x / avg_len);
                let n1 = vec2(-d1.y, d1.x);
                let cos_half = (norm.x * n1.x + norm.y * n1.y).abs();
                let miter = (1.0 / cos_half.max(0.5)).min(1.8);
                norm * miter
            } else {
                vec2(-d1.y, d1.x)
            }
        };

        let offset = normal * half_w;
        let p = clean_pts[i];
        mesh.colored_vertex(p + offset, color);
        mesh.colored_vertex(p - offset, color);
    }

    for i in 0..(n - 1) {
        let base = (i * 2) as u32;
        mesh.add_triangle(base, base + 1, base + 2);
        mesh.add_triangle(base + 1, base + 3, base + 2);
    }

    mesh
}

fn draw_cable_path(
    painter: &egui::Painter,
    pts: &[Pos2],
    w_stroke: f32,
    solid_col: Color32,
    gradient: Option<(Color32, Color32)>,
    glow: bool,
) {
    if pts.len() < 2 {
        return;
    }
    match gradient {
        Some((c_start, c_end)) => {
            if glow {
                let glow_mesh = build_polyline_ribbon(pts, w_stroke * 3.2, c_start, c_end, Some(30));
                painter.add(Shape::mesh(glow_mesh));
            }
            let cable_mesh = build_polyline_ribbon(pts, w_stroke, c_start, c_end, None);
            painter.add(Shape::mesh(cable_mesh));
        }
        None => {
            if glow {
                let glow_c = Color32::from_rgba_unmultiplied(solid_col.r(), solid_col.g(), solid_col.b(), 30);
                painter.add(Shape::line(pts.to_vec(), Stroke::new(w_stroke * 3.2, glow_c)));
            }
            painter.add(Shape::line(pts.to_vec(), Stroke::new(w_stroke, solid_col)));
        }
    }
}

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
    TakeOver { out_port: u32, in_port: u32 },
    SetLinkLinger { out_port: u32, in_port: u32, lingering: bool },
    SetLinkEnabled { out_port: u32, in_port: u32, enabled: bool },
    SetVolume { node_id: u32, linear: f32, channels: usize },
    SetMute { node_id: u32, mute: bool },
    DestroySink { node_id: u32 },
}

#[derive(Clone, Copy)]
struct Pending {
    port: u32,
    from_out: bool,
}

#[derive(Clone, Debug)]
pub struct StereoSwitchPromptData {
    pub link_idx: usize,
    pub partner_idx: usize,
    #[allow(dead_code)]
    pub pair: (u32, u32),
    pub partner_pair: Option<(u32, u32)>,
}

#[derive(Clone, Debug)]
pub struct StereoDisconnectPromptData {
    pub link_idx: usize,
    pub partner_idx: usize,
    pub pair: (u32, u32),
    pub partner_pair: Option<(u32, u32)>,
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
    /// Screen position where a left-drag box started, and whether Ctrl was held (zoom-to-box).
    box_start: Option<(Pos2, bool)>,
    /// Links the user asked to delete with several selected; the app drains it.
    pub extra_disconnects: Vec<(u32, u32)>,
    /// Extra links the user asked to connect (e.g. Shift+drag stereo pair or "Connect stereo to…").
    pub extra_connects: Vec<(u32, u32)>,
    /// Extra links whose enabled state changed (e.g. companion stereo switch).
    pub extra_enables: Vec<(u32, u32, bool)>,
    /// Prompt requesting confirmation to couple with companion stereo switch.
    pub prompt_stereo_switch: Option<StereoSwitchPromptData>,
    /// Prompt requesting confirmation when disconnecting companion stereo link.
    pub prompt_stereo_disconnect: Option<StereoDisconnectPromptData>,
    /// Per-setup virtual sinks and microphones.
    pub virtual_devices: Vec<crate::storage::SavedSink>,
    /// Snap nodes, blocks, comments, and waypoints to the grid (or hold Ctrl).
    pub snap_to_grid: bool,
    /// Show the corner minimap.
    pub show_minimap: bool,
    /// Cached world dimensions `(w, h)` of visible nodes this frame, for fit-to-view and align/distribute.
    node_sizes: HashMap<u32, (f32, f32)>,
    /// Last canvas viewport size in screen pixels.
    last_rect_size: Vec2,
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
    /// Link right-clicked on the canvas: `((out_port, in_port), click_pos, is_owned)`.
    ctx_link: Option<((u32, u32), Pos2, bool)>,
    /// Audio bleed / auto-connection prevention mode.
    pub auto_connect_prevention: crate::storage::AutoConnectPrevention,
    /// External links the user chose to show individually ("visible only").
    pub revealed: HashSet<(u32, u32)>,
    /// Active visual theme.
    pub theme: Theme,
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
            extra_connects: Vec::new(),
            extra_enables: Vec::new(),
            prompt_stereo_switch: None,
            prompt_stereo_disconnect: None,
            virtual_devices: Vec::new(),
            snap_to_grid: false,
            show_minimap: true,
            node_sizes: HashMap::new(),
            last_rect_size: vec2(1000.0, 700.0),
            notes: Vec::new(),
            note_edit: None,
            group_sel: None,
            note_edit_first: false,
            note_buf: String::new(),
            ctx_pos: None,
            ctx_link: None,
            auto_connect_prevention: crate::storage::AutoConnectPrevention::Off,
            revealed: HashSet::new(),
            theme: Theme::default(),
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
/// (used for the dots that flow along links), along with normalized progress `0.0..=1.0`.
fn dots_along(pts: &[Pos2], spacing: f32, phase: f32) -> Vec<(Pos2, f32)> {
    let total: f32 = pts.windows(2).map(|w| (w[1] - w[0]).length()).sum();
    if total <= 0.0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut next = phase.rem_euclid(spacing);
    let mut walked = 0.0;
    for w in pts.windows(2) {
        let seg = (w[1] - w[0]).length();
        if seg <= 0.0 {
            continue;
        }
        while next <= walked + seg {
            let p = w[0] + (w[1] - w[0]) * ((next - walked) / seg);
            out.push((p, (next / total).clamp(0.0, 1.0)));
            next += spacing;
        }
        walked += seg;
    }
    out
}

fn draw_flowing_dots(
    painter: &egui::Painter,
    pts: &[Pos2],
    w_stroke: f32,
    solid_col: Color32,
    gradient: Option<(Color32, Color32)>,
    time: f32,
) {
    let r_dot = (w_stroke * 0.75).clamp(1.5, 3.0);
    for (p, t) in dots_along(pts, 26.0, time * 38.0) {
        let base = match gradient {
            Some((c1, c2)) => lerp_color32(c1, c2, t),
            None => solid_col,
        };
        let dot_col = Color32::from_rgba_unmultiplied(
            base.r().saturating_add(68),
            base.g().saturating_add(68),
            base.b().saturating_add(68),
            232,
        );
        painter.circle_filled(p, r_dot, dot_col);
    }
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
        pts = layout::round_corners(&pts, 12.0 * z, 10);
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
        layout::LinkStyle::Curved => {
            let dist = (b - a).length();
            let segs = ((dist / 7.0_f32).round() as usize).clamp(36, 128);
            layout::bezier_points((a.x, a.y), (b.x, b.y), segs, 40.0 * z)
        }
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

/// Split a polyline at its arc-length midpoint into `(before, mid_point, after)`.
fn split_polyline_mid(pts: &[Pos2]) -> (Vec<Pos2>, Pos2, Vec<Pos2>) {
    if pts.is_empty() {
        return (Vec::new(), pos2(0.0, 0.0), Vec::new());
    }
    if pts.len() == 1 {
        return (vec![pts[0]], pts[0], vec![pts[0]]);
    }
    let total: f32 = pts.windows(2).map(|w| (w[1] - w[0]).length()).sum();
    let half = total * 0.5;
    let mut walked = 0.0_f32;
    let mut before = vec![pts[0]];
    for (idx, w) in pts.windows(2).enumerate() {
        let seg = (w[1] - w[0]).length();
        if walked + seg >= half && seg > 0.0 {
            let t = ((half - walked) / seg).clamp(0.0, 1.0);
            let mid = w[0] + (w[1] - w[0]) * t;
            before.push(mid);
            let mut after = vec![mid];
            after.extend_from_slice(&pts[idx + 1..]);
            return (before, mid, after);
        }
        walked += seg;
        before.push(w[1]);
    }
    let last = *pts.last().unwrap();
    (before, last, vec![last])
}

/// Returns (drag started, drag stopped) for the hit area around a port.
fn port_interact(ui: &mut egui::Ui, port_id: u32, anchor: Pos2, z: f32) -> (bool, bool) {
    let size = Vec2::splat(14.0 * z.max(0.7));
    let resp = ui.interact(
        Rect::from_center_size(anchor, size),
        ui.id().with(("port", port_id)),
        Sense::drag(),
    );
    (
        resp.drag_started_by(egui::PointerButton::Primary),
        resp.drag_stopped_by(egui::PointerButton::Primary),
    )
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
            virtual_devices: f.virtual_devices,
            snap_to_grid: f.snap_to_grid,
            show_minimap: f.show_minimap,
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
            virtual_devices: self.virtual_devices.clone(),
            snap_to_grid: self.snap_to_grid,
            show_minimap: self.show_minimap,
            notes: self.notes.clone(),
            auto_connect_prevention: self.auto_connect_prevention,
            hide_all_sinks: self.sink_hide_mode == crate::storage::SinkHideMode::HideAll,
        }
    }

    /// Pan and zoom so all placed nodes and notes fit inside the current canvas viewport.
    pub fn fit_to_view(&mut self) {
        let mut min_x = f32::INFINITY;
        let mut min_y = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for (&id, &(x, y)) in &self.pos {
            let (w, h) = self.node_sizes.get(&id).copied().unwrap_or((layout::NODE_W, 80.0));
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x + w);
            max_y = max_y.max(y + h);
        }
        for n in &self.notes {
            min_x = min_x.min(n.pos.0);
            min_y = min_y.min(n.pos.1);
            max_x = max_x.max(n.pos.0 + n.size.0.max(80.0));
            max_y = max_y.max(n.pos.1 + n.size.1.max(40.0));
        }
        if min_x.is_finite() && max_x.is_finite() {
            let (pan, zoom) = layout::fit_to_view(
                (min_x, min_y),
                (max_x, max_y),
                (self.last_rect_size.x, self.last_rect_size.y),
                48.0,
            );
            self.pan = vec2(pan.0, pan.1);
            self.zoom = zoom;
            self.dirty = true;
        }
    }

    pub fn zoom_in(&mut self) {
        self.zoom = (self.zoom * 1.2).clamp(layout::MIN_ZOOM, layout::MAX_ZOOM);
        self.dirty = true;
    }

    pub fn zoom_out(&mut self) {
        self.zoom = (self.zoom / 1.2).clamp(layout::MIN_ZOOM, layout::MAX_ZOOM);
        self.dirty = true;
    }

    pub fn reset_view(&mut self) {
        self.pan = vec2(20.0, 20.0);
        self.zoom = 1.0;
        self.dirty = true;
    }

    /// Center the viewport on `node_id` and select it.
    pub fn focus_node(&mut self, node_id: u32) {
        if let Some(&(x, y)) = self.pos.get(&node_id) {
            let (w, h) = self
                .node_sizes
                .get(&node_id)
                .copied()
                .unwrap_or((layout::NODE_W, 80.0));
            let cx = x + w * 0.5;
            let cy = y + h * 0.5;
            self.pan = vec2(
                self.last_rect_size.x * 0.5 - cx * self.zoom,
                self.last_rect_size.y * 0.5 - cy * self.zoom,
            );
            self.sel_nodes.clear();
            self.sel_notes.clear();
            self.sel_nodes.insert(node_id);
            self.dirty = true;
        }
    }

    fn apply_node_alignment(&mut self, g: &Graph, op: impl FnOnce(&mut [(u32, layout::P, layout::P)])) {
        let mut list: Vec<(u32, layout::P, layout::P)> = self
            .sel_nodes
            .iter()
            .filter_map(|&id| {
                let p = *self.pos.get(&id)?;
                let sz = self.node_sizes.get(&id).copied().unwrap_or((layout::NODE_W, 80.0));
                Some((id, p, sz))
            })
            .collect();
        if list.len() < 2 {
            return;
        }
        op(&mut list);
        let snap = self.snap_to_grid;
        for (id, mut p, _) in list {
            if snap {
                p = layout::snap_pos(p, layout::GRID_STEP * 0.5);
            }
            self.pos.insert(id, p);
            if let Some(k) = g.nodes.get(&id).map(|n| n.key()) {
                let alive = |nid| g.nodes.contains_key(&nid);
                if layout::try_claim(&mut self.claimed, alive, &k, id) {
                    self.saved.insert(k, p);
                }
            }
        }
        self.dirty = true;
    }

    pub fn align_selected_left(&mut self, g: &Graph) {
        self.apply_node_alignment(g, layout::align_left);
    }

    pub fn align_selected_top(&mut self, g: &Graph) {
        self.apply_node_alignment(g, layout::align_top);
    }

    pub fn distribute_selected_vertical(&mut self, g: &Graph) {
        self.apply_node_alignment(g, layout::distribute_vertical);
    }

    pub fn distribute_selected_horizontal(&mut self, g: &Graph) {
        self.apply_node_alignment(g, layout::distribute_horizontal);
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
            if self.sel_notes.contains(&note.id) && Some(note.id) != skip_note {
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

    pub fn pos_of(&self, node_id: u32) -> Option<(f32, f32)> {
        self.pos.get(&node_id).copied()
    }

    pub fn size_of(&self, node_id: u32) -> Option<(f32, f32)> {
        self.node_sizes.get(&node_id).copied()
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
        self.last_rect_size = rect.size();
        if !ui.ctx().wants_keyboard_input()
            && ui.input(|i| i.key_pressed(egui::Key::F) || i.key_pressed(egui::Key::Home))
        {
            self.fit_to_view();
        }
        let snap_active = self.snap_to_grid;
        let grid_unit = layout::GRID_STEP * 0.5;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(0), self.theme.c_canvas_bg());
        match self.theme.grid_pattern {
            GridPattern::Dots => {
                let mut step = 28.0 * self.zoom;
                while step < 18.0 {
                    step *= 2.0;
                }
                let origin = rect.min + self.pan;
                let x0 = rect.min.x + (origin.x - rect.min.x).rem_euclid(step);
                let y0 = rect.min.y + (origin.y - rect.min.y).rem_euclid(step);
                let dot = self.theme.c_grid_dot();
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
            GridPattern::Lines => {
                let mut step = 28.0 * self.zoom;
                while step < 18.0 {
                    step *= 2.0;
                }
                let origin = rect.min + self.pan;
                let x0 = rect.min.x + (origin.x - rect.min.x).rem_euclid(step);
                let y0 = rect.min.y + (origin.y - rect.min.y).rem_euclid(step);
                let line_col = self.theme.c32_alpha(
                    self.theme.grid_dot,
                    if self.theme.is_dark { 55 } else { 75 },
                );
                let stroke = Stroke::new(1.0_f32, line_col);
                let mut x = x0;
                while x < rect.max.x {
                    painter.line_segment([pos2(x, rect.min.y), pos2(x, rect.max.y)], stroke);
                    x += step;
                }
                let mut y = y0;
                while y < rect.max.y {
                    painter.line_segment([pos2(rect.min.x, y), pos2(rect.max.x, y)], stroke);
                    y += step;
                }
            }
            GridPattern::None => {}
        }

        // Pan with middle mouse button only; left-drag on empty canvas starts a box
        // (selection box, or Ctrl+drag zoom-to-fit box).
        if bg.drag_started_by(egui::PointerButton::Primary) {
            if let Some(pos) = bg.interact_pointer_pos() {
                let is_zoom = ui.input(|i| i.modifiers.ctrl);
                self.box_start = Some((pos, is_zoom));
            }
        }
        if ui.rect_contains_pointer(rect)
            && ui.input(|i| i.pointer.button_down(egui::PointerButton::Middle))
        {
            let d = ui.input(|i| i.pointer.delta());
            if d != Vec2::ZERO {
                self.pan += d;
            }
        }
        if ui.input(|i| i.pointer.button_released(egui::PointerButton::Middle)) {
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
            if self.sel_notes.contains(&n.id) {
                painter.rect_stroke(
                    r.expand(2.0 * z),
                    CornerRadius::same(9),
                    Stroke::new(2.0_f32, self.theme.c_accent()),
                    egui::StrokeKind::Outside,
                );
            }
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
                .on_hover_text("Click: select block (Delete removes it). Drag: move the block. Double-click: select the nodes and comments inside, then drag to move them too");
            if resp.clicked() && self.select_mode != SelectMode::Links {
                let add = ui.input(|i| i.modifiers.shift);
                if !add {
                    self.sel_nodes.clear();
                    self.sel_notes.clear();
                    self.box_links.clear();
                    self.selected = None;
                    self.group_sel = None;
                }
                self.sel_notes.insert(n.id);
            }
            if resp.dragged_by(egui::PointerButton::Primary) {
                let d = resp.drag_delta() / z;
                self.notes[i].pos = (n.pos.0 + d.x, n.pos.1 + d.y);
                if self.group_sel == Some(n.id) || self.sel_notes.contains(&n.id) {
                    self.move_selected(g, d, None, Some(n.id));
                }
            }
            if resp.drag_stopped_by(egui::PointerButton::Primary) {
                if snap_active {
                    self.notes[i].pos = layout::snap_pos(self.notes[i].pos, grid_unit);
                }
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
            if gresp.dragged_by(egui::PointerButton::Primary) {
                let d = gresp.drag_delta() / z;
                self.notes[i].size = ((n.size.0 + d.x).max(80.0), (n.size.1 + d.y).max(40.0));
            }
            if gresp.drag_stopped_by(egui::PointerButton::Primary) {
                self.dirty = true;
            }
            let gs = Stroke::new((1.5 * z).max(1.0), Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 200));
            painter.line_segment([r.max + vec2(-10.0, -2.0) * z, r.max + vec2(-2.0, -10.0) * z], gs);
            painter.line_segment([r.max + vec2(-6.0, -2.0) * z, r.max + vec2(-2.0, -6.0) * z], gs);
        }
        // Text width at zoom 1, used to size nodes so labels do not overlap.
        let ndelta = self.theme.node_font_size_delta;
        let (title_font, port_font) = (
            FontId::proportional((13.0 + ndelta).max(8.0)),
            FontId::proportional((11.0 + ndelta).max(7.0)),
        );
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
        self.node_sizes.clear();
        let mut items: Vec<Item> = Vec::new();
        let mut anchors: HashMap<u32, Pos2> = HashMap::new();
        let mut drag_started: Option<Pending> = None;
        let mut drag_stopped = false;
        let mut to_hide: Vec<String> = Vec::new();
        let mut color_set: Vec<(String, [u8; 3])> = Vec::new();
        let mut color_clear: Vec<String> = Vec::new();
        let mut stereo_connect_req: Option<(u32, u32)> = None;
        let mut align_op: Option<u8> = None;
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
            self.node_sizes.insert(node.id, (w, h));
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
                        if g.is_node_owned(node) && node.is_sink() {
                            if ui.button("Destroy virtual sink").clicked() {
                                action = Some(Action::DestroySink { node_id: node.id });
                                ui.close_menu();
                            }
                        } else if g.is_node_owned(node) && node.is_source() {
                            if ui.button("Destroy virtual mic").clicked() {
                                action = Some(Action::DestroySink { node_id: node.id });
                                ui.close_menu();
                            }
                        }
                        if !outs.is_empty() {
                            let targets: Vec<(u32, String)> = g
                                .nodes
                                .values()
                                .filter(|o| o.id != node.id && node.can_link_to(o) && !g.ports_of(o.id, Direction::In).is_empty())
                                .map(|o| (o.id, o.label().to_string()))
                                .collect();
                            if !targets.is_empty() {
                                ui.menu_button("Connect stereo to…", |ui| {
                                    for (tid, tlabel) in &targets {
                                        if ui.button(tlabel).clicked() {
                                            stereo_connect_req = Some((node.id, *tid));
                                            ui.close_menu();
                                        }
                                    }
                                });
                            }
                        }
                        if self.sel_nodes.len() >= 2 && self.sel_nodes.contains(&node.id) {
                            ui.menu_button("Align / Distribute", |ui| {
                                if ui.button("Align left").clicked() {
                                    align_op = Some(1);
                                    ui.close_menu();
                                }
                                if ui.button("Align top").clicked() {
                                    align_op = Some(2);
                                    ui.close_menu();
                                }
                                if ui.button("Distribute vertically").clicked() {
                                    align_op = Some(3);
                                    ui.close_menu();
                                }
                                if ui.button("Distribute horizontally").clicked() {
                                    align_op = Some(4);
                                    ui.close_menu();
                                }
                            });
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
                        self.box_links.clear();
                        self.selected = None;
                        self.group_sel = None;
                    }
                    self.sel_nodes.insert(node.id);
                }
                if resp.dragged_by(egui::PointerButton::Primary) {
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
                if resp.drag_stopped_by(egui::PointerButton::Primary) {
                    if snap_active {
                        p = layout::snap_pos(p, grid_unit);
                        self.pos.insert(node.id, p);
                        if let Some(ref k) = key {
                            let alive = |id| g.nodes.contains_key(&id);
                            if layout::try_claim(&mut self.claimed, alive, k, node.id) {
                                self.saved.insert(k.clone(), p);
                            }
                        }
                        for &sid in &self.sel_nodes {
                            if sid != node.id {
                                if let Some(sp) = self.pos.get_mut(&sid) {
                                    *sp = layout::snap_pos(*sp, grid_unit);
                                    let snapped = *sp;
                                    if let Some(sk) = g.nodes.get(&sid).map(|n| n.key()) {
                                        let alive = |id| g.nodes.contains_key(&id);
                                        if layout::try_claim(&mut self.claimed, alive, &sk, sid) {
                                            self.saved.insert(sk, snapped);
                                        }
                                    }
                                }
                            }
                        }
                    }
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
                if resp.dragged_by(egui::PointerButton::Primary) {
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
                if resp.drag_stopped_by(egui::PointerButton::Primary) {
                    self.dirty = true;
                }
            }
            items.push(Item { node, ins, outs, rect: r, w, header_h, color, vol_y });
        }

        for name in to_hide {
            self.hide(name);
        }
        if let Some((src_node, dst_node)) = stereo_connect_req {
            let pairs = crate::links::stereo_node_pairs(g, src_node, dst_node);
            self.extra_connects.extend(pairs);
        }
        match align_op {
            Some(1) => self.align_selected_left(g),
            Some(2) => self.align_selected_top(g),
            Some(3) => self.distribute_selected_vertical(g),
            Some(4) => self.distribute_selected_horizontal(g),
            _ => {}
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
        // Holding Shift also connects the matching stereo partner ports (e.g. FL + FR).
        if drag_stopped {
            if let Some(from) = self.pending.take() {
                let pointer = ui.input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos()));
                if let Some(target) = pointer.and_then(|pos| self.target_at(pos, &anchors, g, from)) {
                    let (out_port, in_port) = if from.from_out {
                        (from.port, target)
                    } else {
                        (target, from.port)
                    };
                    action = Some(Action::Connect { out_port, in_port });
                    if ui.input(|i| i.modifiers.shift) {
                        if let Some(partner) = crate::links::stereo_partner_ports(g, out_port, in_port) {
                            self.extra_connects.push(partner);
                        }
                    }
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
        // Switch links remain drawn even when switched OFF (`enabled = false`).
        let mut shapes: Vec<((u32, u32), Vec<Pos2>)> = Vec::new();
        let mut saved_of: HashMap<(u32, u32), usize> = HashMap::new();
        for (i, l) in self.links.iter().enumerate() {
            if let Some(p) = crate::links::resolve(g, l) {
                saved_of.insert(p, i);
            }
        }
        let pairs: Vec<((u32, u32), bool)> = if self.virtual_mode {
            let mut v: Vec<((u32, u32), bool)> = self
                .links
                .iter()
                .filter(|l| l.enabled || l.has_switch)
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
            let mut v: Vec<((u32, u32), bool)> = g
                .links
                .values()
                .map(|l| ((l.out_port, l.in_port), g.is_owned(l)))
                .collect();
            for l in self.links.iter().filter(|l| l.has_switch) {
                if let Some(p) = crate::links::resolve(g, l) {
                    if !v.iter().any(|&(existing, _)| existing == p) {
                        v.push((p, true));
                    }
                }
            }
            v
        };
        let time = ui.input(|i| i.time) as f32;
        let to_screen = |w: (f32, f32)| origin + vec2(w.0, w.1) * z;
        let mut selectable: HashSet<(u32, u32)> = HashSet::new();
        let mut handles: Vec<(usize, usize, Pos2)> = Vec::new();
        let mut switch_handles: Vec<(usize, (u32, u32), Pos2, bool, bool, bool)> = Vec::new();
        let mut sw_positions: HashMap<usize, Pos2> = HashMap::new();
        for (pair, owned) in pairs {
            if !(owned || self.show_external || self.revealed.contains(&pair)) {
                continue;
            }
            let (Some(a), Some(b)) = (anchors.get(&pair.0), anchors.get(&pair.1)) else {
                continue;
            };
            let saved_idx = saved_of.get(&pair).copied();
            let wps: Vec<Pos2> = saved_idx
                .map(|i| self.links[i].waypoints.iter().map(|&w| to_screen(w)).collect())
                .unwrap_or_default();
            if owned {
                selectable.insert(pair);
            }
            let picked = self.selected == Some(pair) || self.box_links.contains(&pair);
            if picked {
                if let Some(i) = saved_idx.filter(|&i| !self.links[i].waypoints.is_empty()) {
                    handles.extend(wps.iter().enumerate().map(|(k, p)| (i, k, *p)));
                }
            }
            let has_switch = owned && saved_idx.is_some_and(|i| self.links[i].has_switch);
            let switch_on = saved_idx.map_or(true, |i| self.links[i].enabled);
            if has_switch {
                let sidx = saved_idx.unwrap();
                let (pts_before, sw_mid, pts_after, pts_all) =
                    if let Some(sp) = self.links[sidx].switch_pos {
                        let mid = to_screen(sp);
                        let k = waypoint_slot(self.link_style, *a, &wps, *b, mid, z, pair.0);
                        let before = link_path(self.link_style, *a, &wps[..k], mid, z, pair.0);
                        let after = link_path(self.link_style, mid, &wps[k..], *b, z, pair.0);
                        let mut all = before.clone();
                        if after.len() > 1 {
                            all.extend_from_slice(&after[1..]);
                        }
                        (before, mid, after, all)
                    } else {
                        let all = link_path(self.link_style, *a, &wps, *b, z, pair.0);
                        let (before, mid, after) = split_polyline_mid(&all);
                        (before, mid, after, all)
                    };
                let w_stroke = if picked {
                    (3.0 * z).clamp(2.0, 4.5)
                } else {
                    (self.theme.cable_thickness_switch * z).clamp(1.0, 6.0)
                };
                let active_col = if picked {
                    self.theme.c_cable_selected()
                } else {
                    self.theme.c_cable_switch()
                };
                let (grad_before, grad_after) = if !picked && self.theme.cable_grad_switch_enabled {
                    match self.theme.cable_grad_switch_mode {
                        SwitchGradMode::Simple => {
                            let c0 = self.theme.c_cable_grad_switch_start();
                            let c1 = self.theme.c_cable_grad_switch_end();
                            let cmid = lerp_color32(c0, c1, 0.5_f32);
                            (Some((c0, cmid)), Some((cmid, c1)))
                        }
                        SwitchGradMode::Advanced => {
                            let c_in = self.theme.c_cable_grad_switch_start();
                            let c_sw_in = self.theme.c_cable_grad_switch_mid_in();
                            let c_sw_out = self.theme.c_cable_grad_switch_mid_out();
                            let c_out = self.theme.c_cable_grad_switch_end();
                            (Some((c_in, c_sw_in)), Some((c_sw_out, c_out)))
                        }
                    }
                } else {
                    (None, None)
                };
                let glow = self.theme.cable_glow_switch;

                // First half (up to the switch): always colored and animated.
                draw_cable_path(&painter, &pts_before, w_stroke, active_col, grad_before, glow);
                draw_flowing_dots(&painter, &pts_before, w_stroke, active_col, grad_before, time);

                // Second half (after the switch): colored + animated only when the switch is ON.
                if switch_on {
                    draw_cable_path(&painter, &pts_after, w_stroke, active_col, grad_after, glow);
                    draw_flowing_dots(&painter, &pts_after, w_stroke, active_col, grad_after, time);
                } else {
                    let off_col = if picked {
                        Color32::from_rgba_unmultiplied(195, 160, 75, 130)
                    } else {
                        Color32::from_rgba_unmultiplied(110, 108, 128, 125)
                    };
                    let stroke_after = Stroke::new((1.6_f32 * z).clamp(1.0, 2.5), off_col);
                    painter.add(Shape::line(pts_after, stroke_after));
                }

                let is_coupled = self.links[sidx].stereo_coupled;
                sw_positions.insert(sidx, sw_mid);
                switch_handles.push((sidx, pair, sw_mid, switch_on, picked, is_coupled));
                shapes.push((pair, pts_all));
            } else {
                let pts = link_path(self.link_style, *a, &wps, *b, z, pair.0);
                let w_stroke = if picked {
                    (3.0 * z).clamp(2.0, 4.5)
                } else if owned {
                    (self.theme.cable_thickness_owned * z).clamp(1.0, 6.0)
                } else {
                    (self.theme.cable_thickness_external * z).clamp(0.8, 5.0)
                };
                let (solid_col, grad, glow) = if picked {
                    (self.theme.c_cable_selected(), None, true)
                } else if owned {
                    let g = if self.theme.cable_grad_owned_enabled {
                        Some((
                            self.theme.c_cable_grad_owned_start(),
                            self.theme.c_cable_grad_owned_end(),
                        ))
                    } else {
                        None
                    };
                    (self.theme.c_cable_owned(), g, self.theme.cable_glow_owned)
                } else {
                    let g = if self.theme.cable_grad_external_enabled {
                        Some((
                            self.theme.c_cable_grad_external_start(),
                            self.theme.c_cable_grad_external_end(),
                        ))
                    } else {
                        None
                    };
                    (self.theme.c_cable_external(), g, self.theme.cable_glow_external)
                };

                draw_cable_path(&painter, &pts, w_stroke, solid_col, grad, glow);
                // Dots flowing from the output port to the input port.
                draw_flowing_dots(&painter, &pts, w_stroke, solid_col, grad, time);
                shapes.push((pair, pts));
            }
        }
        if !shapes.is_empty() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
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
            if resp.dragged_by(egui::PointerButton::Primary) {
                let d = resp.drag_delta() / z;
                if let Some(w) = self.links.get_mut(i).and_then(|l| l.waypoints.get_mut(k)) {
                    *w = (w.0 + d.x, w.1 + d.y);
                    self.dirty = true;
                }
            }
            if resp.drag_stopped_by(egui::PointerButton::Primary) && snap_active {
                if let Some(w) = self.links.get_mut(i).and_then(|l| l.waypoints.get_mut(k)) {
                    *w = layout::snap_pos(*w, grid_unit);
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
                Stroke::new(2.0_f32, self.theme.c_cable_selected()),
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

        // Inline switch boxes on switch cables: click to toggle ON/OFF, drag to reposition.
        let zz = z.clamp(0.75, 1.3);

        // Draw connecting bridge between companion stereo switches
        for &(sidx, _, sw_pos, _, _, is_coupled) in &switch_handles {
            if is_coupled {
                if let Some(pidx) = crate::links::find_stereo_partner_idx(&self.links, sidx, Some(g)) {
                    if sidx < pidx {
                        if let Some(&p_pos) = sw_positions.get(&pidx) {
                            let bridge_col = self.theme.c_cable_switch().gamma_multiply(0.65);
                            let stroke = Stroke::new((1.5 * zz).max(1.0), bridge_col);
                            painter.line_segment([sw_pos, p_pos], stroke);
                            let mid = pos2((sw_pos.x + p_pos.x) * 0.5, (sw_pos.y + p_pos.y) * 0.5);
                            let badge_size = vec2(16.0, 14.0) * zz;
                            let badge_rect = Rect::from_center_size(mid, badge_size);
                            painter.rect_filled(badge_rect, CornerRadius::same(3), self.theme.c_node_body());
                            painter.rect_stroke(badge_rect, CornerRadius::same(3), stroke, egui::StrokeKind::Inside);
                            painter.text(mid, egui::Align2::CENTER_CENTER, "🔗", egui::FontId::proportional(9.0 * zz), self.theme.c_node_text());
                        }
                    }
                }
            }
        }

        for (sidx, pair, sw_pos, sw_on, picked, is_coupled) in switch_handles {
            let sw_size = vec2(32.0, 16.0) * zz;
            let sw_rect = Rect::from_center_size(sw_pos, sw_size);
            let partner_idx = crate::links::find_stereo_partner_idx(&self.links, sidx, Some(g));
            let tip = if is_coupled {
                if sw_on {
                    "Stereo Switch (Coupled) is ON: click to turn both L+R off, drag to move"
                } else {
                    "Stereo Switch (Coupled) is OFF: click to turn both L+R on, drag to move"
                }
            } else {
                if sw_on {
                    "Switch is ON: click to turn off, drag to move"
                } else {
                    "Switch is OFF: click to turn on, drag to move"
                }
            };
            let resp = ui
                .interact(
                    sw_rect.expand(2.0),
                    ui.id().with(("sw_box", sidx)),
                    Sense::click_and_drag(),
                )
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(tip);
            handle_hit |= resp.hovered() || resp.dragged();
            if resp.clicked() {
                let new_on = !sw_on;
                if let Some(l) = self.links.get_mut(sidx) {
                    l.enabled = new_on;
                    self.dirty = true;
                }
                action = Some(Action::SetLinkEnabled {
                    out_port: pair.0,
                    in_port: pair.1,
                    enabled: new_on,
                });
                if is_coupled {
                    if let Some(pidx) = partner_idx {
                        if self.links.get(pidx).is_some_and(|l| l.stereo_coupled) {
                            self.links[pidx].enabled = new_on;
                            self.dirty = true;
                            if let Some(pp) = crate::links::resolve(g, &self.links[pidx]) {
                                self.extra_enables.push((pp.0, pp.1, new_on));
                            }
                        }
                    }
                }
            }
            if resp.dragged_by(egui::PointerButton::Primary) {
                let d = resp.drag_delta() / z;
                let cur = self
                    .links
                    .get(sidx)
                    .and_then(|l| l.switch_pos)
                    .unwrap_or_else(|| {
                        let w = (sw_pos - origin) / z;
                        (w.x, w.y)
                    });
                if let Some(l) = self.links.get_mut(sidx) {
                    l.switch_pos = Some((cur.0 + d.x, cur.1 + d.y));
                    self.dirty = true;
                }
                if is_coupled {
                    if let Some(pidx) = partner_idx {
                        if self.links.get(pidx).is_some_and(|l| l.stereo_coupled) {
                            let p_pos = sw_positions.get(&pidx).copied().unwrap_or(sw_pos);
                            let p_cur = self
                                .links
                                .get(pidx)
                                .and_then(|l| l.switch_pos)
                                .unwrap_or_else(|| {
                                    let w = (p_pos - origin) / z;
                                    (w.x, w.y)
                                });
                            if let Some(pl) = self.links.get_mut(pidx) {
                                pl.switch_pos = Some((p_cur.0 + d.x, p_cur.1 + d.y));
                                self.dirty = true;
                            }
                        }
                    }
                }
            }
            if resp.drag_stopped_by(egui::PointerButton::Primary) && snap_active {
                if let Some(l) = self.links.get_mut(sidx) {
                    if let Some(sp) = l.switch_pos {
                        l.switch_pos = Some(layout::snap_pos(sp, grid_unit));
                        self.dirty = true;
                    }
                }
                if is_coupled {
                    if let Some(pidx) = partner_idx {
                        if let Some(pl) = self.links.get_mut(pidx) {
                            if let Some(sp) = pl.switch_pos {
                                pl.switch_pos = Some(layout::snap_pos(sp, grid_unit));
                                self.dirty = true;
                            }
                        }
                    }
                }
            }
            resp.context_menu(|ui| {
                if is_coupled {
                    if ui
                        .button(if sw_on { "Turn stereo switch off" } else { "Turn stereo switch on" })
                        .clicked()
                    {
                        let new_on = !sw_on;
                        if let Some(l) = self.links.get_mut(sidx) {
                            l.enabled = new_on;
                            self.dirty = true;
                        }
                        action = Some(Action::SetLinkEnabled {
                            out_port: pair.0,
                            in_port: pair.1,
                            enabled: new_on,
                        });
                        if let Some(pidx) = partner_idx {
                            if self.links.get(pidx).is_some_and(|l| l.stereo_coupled) {
                                self.links[pidx].enabled = new_on;
                                self.dirty = true;
                                if let Some(pp) = crate::links::resolve(g, &self.links[pidx]) {
                                    self.extra_enables.push((pp.0, pp.1, new_on));
                                }
                            }
                        }
                        ui.close_menu();
                    }
                    if ui
                        .button("Decouple stereo switch")
                        .on_hover_text("Allow each channel to be switched independently")
                        .clicked()
                    {
                        if let Some(l) = self.links.get_mut(sidx) {
                            l.stereo_coupled = false;
                            self.dirty = true;
                        }
                        if let Some(pidx) = partner_idx {
                            if let Some(pl) = self.links.get_mut(pidx) {
                                pl.stereo_coupled = false;
                                self.dirty = true;
                            }
                        }
                        ui.close_menu();
                    }
                    let has_custom_pos = self.links.get(sidx).is_some_and(|l| l.switch_pos.is_some())
                        || partner_idx.is_some_and(|p| self.links.get(p).is_some_and(|l| l.switch_pos.is_some()));
                    if ui
                        .add_enabled(has_custom_pos, egui::Button::new("Reset switch positions"))
                        .clicked()
                    {
                        if let Some(l) = self.links.get_mut(sidx) {
                            l.switch_pos = None;
                            self.dirty = true;
                        }
                        if let Some(pidx) = partner_idx {
                            if let Some(pl) = self.links.get_mut(pidx) {
                                pl.switch_pos = None;
                                self.dirty = true;
                            }
                        }
                        ui.close_menu();
                    }
                    if ui.button("Remove stereo switch (both channels)").clicked() {
                        if let Some(l) = self.links.get_mut(sidx) {
                            let was_off = !l.enabled;
                            l.has_switch = false;
                            l.stereo_coupled = false;
                            l.switch_pos = None;
                            l.enabled = true;
                            self.dirty = true;
                            if was_off {
                                action = Some(Action::SetLinkEnabled {
                                    out_port: pair.0,
                                    in_port: pair.1,
                                    enabled: true,
                                });
                            }
                        }
                        if let Some(pidx) = partner_idx {
                            if let Some(pl) = self.links.get_mut(pidx) {
                                let p_was_off = !pl.enabled;
                                pl.has_switch = false;
                                pl.stereo_coupled = false;
                                pl.switch_pos = None;
                                pl.enabled = true;
                                self.dirty = true;
                                if p_was_off {
                                    if let Some(pp) = crate::links::resolve(g, pl) {
                                        self.extra_enables.push((pp.0, pp.1, true));
                                    }
                                }
                            }
                        }
                        ui.close_menu();
                    }
                    if ui.button("Remove switch (this channel only)").clicked() {
                        if let Some(l) = self.links.get_mut(sidx) {
                            let was_off = !l.enabled;
                            l.has_switch = false;
                            l.stereo_coupled = false;
                            l.switch_pos = None;
                            l.enabled = true;
                            self.dirty = true;
                            if was_off {
                                action = Some(Action::SetLinkEnabled {
                                    out_port: pair.0,
                                    in_port: pair.1,
                                    enabled: true,
                                });
                            }
                        }
                        if let Some(pidx) = partner_idx {
                            if let Some(pl) = self.links.get_mut(pidx) {
                                pl.stereo_coupled = false;
                                self.dirty = true;
                            }
                        }
                        ui.close_menu();
                    }
                } else {
                    if let Some(pidx) = partner_idx {
                        if self.links.get(pidx).is_some_and(|l| l.has_switch) {
                            if ui
                                .button("Couple with companion stereo switch")
                                .on_hover_text("Lock L+R switches together to toggle both in 1 click")
                                .clicked()
                            {
                                if let Some(l) = self.links.get_mut(sidx) {
                                    l.stereo_coupled = true;
                                    self.dirty = true;
                                }
                                if let Some(pl) = self.links.get_mut(pidx) {
                                    pl.stereo_coupled = true;
                                    pl.enabled = sw_on;
                                    self.dirty = true;
                                    if let Some(pp) = crate::links::resolve(g, pl) {
                                        self.extra_enables.push((pp.0, pp.1, sw_on));
                                    }
                                }
                                ui.close_menu();
                            }
                            ui.separator();
                        }
                    }
                    if ui
                        .button(if sw_on { "Turn switch off" } else { "Turn switch on" })
                        .clicked()
                    {
                        let new_on = !sw_on;
                        if let Some(l) = self.links.get_mut(sidx) {
                            l.enabled = new_on;
                            self.dirty = true;
                        }
                        action = Some(Action::SetLinkEnabled {
                            out_port: pair.0,
                            in_port: pair.1,
                            enabled: new_on,
                        });
                        ui.close_menu();
                    }
                    let has_custom_pos = self.links.get(sidx).is_some_and(|l| l.switch_pos.is_some());
                    if ui
                        .add_enabled(has_custom_pos, egui::Button::new("Reset switch position"))
                        .clicked()
                    {
                        if let Some(l) = self.links.get_mut(sidx) {
                            l.switch_pos = None;
                            self.dirty = true;
                        }
                        ui.close_menu();
                    }
                    if ui.button("Remove switch").clicked() {
                        if let Some(l) = self.links.get_mut(sidx) {
                            let was_off = !l.enabled;
                            l.has_switch = false;
                            l.switch_pos = None;
                            l.enabled = true;
                            self.dirty = true;
                            if was_off {
                                action = Some(Action::SetLinkEnabled {
                                    out_port: pair.0,
                                    in_port: pair.1,
                                    enabled: true,
                                });
                            }
                        }
                        ui.close_menu();
                    }
                }
                ui.separator();
                if ui.button("Delete link").clicked() {
                    let mut prompted = false;
                    if let Some(pidx) = partner_idx {
                        let partner_pair = crate::links::resolve(g, &self.links[pidx]);
                        let partner_connected = partner_pair.is_some_and(|(po, pi)| g.find_link(po, pi).is_some());
                        if is_coupled || partner_connected {
                            self.prompt_stereo_disconnect = Some(StereoDisconnectPromptData {
                                link_idx: sidx,
                                partner_idx: pidx,
                                pair,
                                partner_pair,
                            });
                            prompted = true;
                        }
                    }
                    if !prompted {
                        action = Some(Action::Disconnect {
                            out_port: pair.0,
                            in_port: pair.1,
                        });
                    }
                    self.selected = None;
                    ui.close_menu();
                }
            });

            let sw_col = self.theme.c_cable_switch();
            let rad = (sw_size.y * 0.5).round() as u8;
            painter.rect_filled(
                sw_rect.translate(vec2(0.0, 2.0 * zz)).expand(1.5 * zz),
                CornerRadius::same(rad),
                Color32::from_black_alpha(if self.theme.is_dark { 90 } else { 40 }),
            );
            let bg_fill = if sw_on {
                lerp_color32(self.theme.c_node_body(), sw_col, 0.35)
            } else {
                self.theme.c_node_body()
            };
            painter.rect_filled(sw_rect, CornerRadius::same(rad), bg_fill);
            let border_col = if picked || resp.hovered() {
                self.theme.c_cable_selected()
            } else if sw_on {
                sw_col
            } else if self.theme.is_dark {
                Color32::from_rgb(105, 100, 125)
            } else {
                Color32::from_rgb(160, 158, 175)
            };
            painter.rect_stroke(
                sw_rect,
                CornerRadius::same(rad),
                Stroke::new((1.6 * zz).max(1.0), border_col),
                egui::StrokeKind::Inside,
            );
            let knob_x = if sw_on {
                sw_rect.max.x - sw_size.y * 0.5
            } else {
                sw_rect.min.x + sw_size.y * 0.5
            };
            let knob_col = if sw_on {
                lighten(sw_col)
            } else if self.theme.is_dark {
                Color32::from_gray(145)
            } else {
                Color32::from_gray(135)
            };
            painter.circle_filled(
                pos2(knob_x, sw_rect.center().y),
                sw_size.y * 0.34,
                knob_col,
            );

            // Channel indicator ("L" / "R") inside switch pill
            let ch_tag = crate::links::channel_label(&self.links[sidx].out_port);
            if let Some(tag) = ch_tag {
                let tag_x = if sw_on {
                    sw_rect.min.x + sw_size.y * 0.45
                } else {
                    sw_rect.max.x - sw_size.y * 0.45
                };
                let tag_pos = pos2(tag_x, sw_rect.center().y);
                let tag_col = if sw_on {
                    Color32::WHITE
                } else {
                    Color32::from_gray(if self.theme.is_dark { 165 } else { 125 })
                };
                painter.text(
                    tag_pos,
                    egui::Align2::CENTER_CENTER,
                    tag,
                    egui::FontId::monospace((10.0 * zz).clamp(8.0, 14.0)),
                    tag_col,
                );
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

        // Right-click on a link or on empty canvas.
        if bg.secondary_clicked() {
            if let Some(pos) = bg.interact_pointer_pos() {
                let hit = shapes
                    .iter()
                    .map(|(pair, pts)| {
                        let line: Vec<(f32, f32)> = pts.iter().map(|p| (p.x, p.y)).collect();
                        (*pair, layout::dist_to_polyline((pos.x, pos.y), &line))
                    })
                    .filter(|(_, d)| *d <= 8.0)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(pair, _)| pair);
                if let Some(pair) = hit {
                    let owned = selectable.contains(&pair);
                    self.ctx_link = Some((pair, pos, owned));
                    self.selected = Some(pair);
                } else {
                    self.ctx_link = None;
                    self.ctx_pos = Some(pos);
                }
            }
        }
        let mut new_note: Option<NoteKind> = None;
        bg.context_menu(|ui| {
            if let Some((pair, click_pos, owned)) = self.ctx_link {
                if owned {
                    let saved_idx = crate::links::from_ports(g, pair.0, pair.1)
                        .and_then(|s| crate::links::find(&self.links, &s));
                    if let Some(i) = saved_idx {
                        let is_coupled = self.links[i].stereo_coupled;
                        let partner_idx = crate::links::find_stereo_partner_idx(&self.links, i, Some(g));
                        if !self.links[i].has_switch {
                            if ui
                                .button("Make switch")
                                .on_hover_text("Add an inline on/off switch box on this connection")
                                .clicked()
                            {
                                if let Some(pidx) = partner_idx {
                                    let partner_pair = crate::links::resolve(g, &self.links[pidx]);
                                    self.prompt_stereo_switch = Some(StereoSwitchPromptData {
                                        link_idx: i,
                                        partner_idx: pidx,
                                        pair,
                                        partner_pair,
                                    });
                                } else {
                                    self.links[i].has_switch = true;
                                    self.links[i].stereo_coupled = false;
                                    self.links[i].switch_pos = None;
                                    self.dirty = true;
                                }
                                ui.close_menu();
                            }
                        } else {
                            if is_coupled {
                                let on = self.links[i].enabled;
                                if ui
                                    .button(if on { "Turn stereo switch off" } else { "Turn stereo switch on" })
                                    .clicked()
                                {
                                    let new_on = !on;
                                    self.links[i].enabled = new_on;
                                    self.dirty = true;
                                    action = Some(Action::SetLinkEnabled {
                                        out_port: pair.0,
                                        in_port: pair.1,
                                        enabled: new_on,
                                    });
                                    if let Some(pidx) = partner_idx {
                                        if self.links.get(pidx).is_some_and(|l| l.stereo_coupled) {
                                            self.links[pidx].enabled = new_on;
                                            self.dirty = true;
                                            if let Some(pp) = crate::links::resolve(g, &self.links[pidx]) {
                                                self.extra_enables.push((pp.0, pp.1, new_on));
                                            }
                                        }
                                    }
                                    ui.close_menu();
                                }
                                if ui.button("Decouple stereo switch").clicked() {
                                    self.links[i].stereo_coupled = false;
                                    if let Some(pidx) = partner_idx {
                                        if let Some(pl) = self.links.get_mut(pidx) {
                                            pl.stereo_coupled = false;
                                        }
                                    }
                                    self.dirty = true;
                                    ui.close_menu();
                                }
                                if ui.button("Remove stereo switch (both channels)").clicked() {
                                    let was_off = !self.links[i].enabled;
                                    self.links[i].has_switch = false;
                                    self.links[i].stereo_coupled = false;
                                    self.links[i].switch_pos = None;
                                    self.links[i].enabled = true;
                                    self.dirty = true;
                                    if was_off {
                                        action = Some(Action::SetLinkEnabled {
                                            out_port: pair.0,
                                            in_port: pair.1,
                                            enabled: true,
                                        });
                                    }
                                    if let Some(pidx) = partner_idx {
                                        if let Some(pl) = self.links.get_mut(pidx) {
                                            let p_was_off = !pl.enabled;
                                            pl.has_switch = false;
                                            pl.stereo_coupled = false;
                                            pl.switch_pos = None;
                                            pl.enabled = true;
                                            self.dirty = true;
                                            if p_was_off {
                                                if let Some(pp) = crate::links::resolve(g, pl) {
                                                    self.extra_enables.push((pp.0, pp.1, true));
                                                }
                                            }
                                        }
                                    }
                                    ui.close_menu();
                                }
                            } else {
                                if let Some(pidx) = partner_idx {
                                    if self.links.get(pidx).is_some_and(|l| l.has_switch) {
                                        if ui.button("Couple with companion stereo switch").clicked() {
                                            let on = self.links[i].enabled;
                                            self.links[i].stereo_coupled = true;
                                            if let Some(pl) = self.links.get_mut(pidx) {
                                                pl.stereo_coupled = true;
                                                pl.enabled = on;
                                                if let Some(pp) = crate::links::resolve(g, pl) {
                                                    self.extra_enables.push((pp.0, pp.1, on));
                                                }
                                            }
                                            self.dirty = true;
                                            ui.close_menu();
                                        }
                                    }
                                }
                                let on = self.links[i].enabled;
                                if ui
                                    .button(if on { "Turn switch off" } else { "Turn switch on" })
                                    .clicked()
                                {
                                    self.links[i].enabled = !on;
                                    self.dirty = true;
                                    action = Some(Action::SetLinkEnabled {
                                        out_port: pair.0,
                                        in_port: pair.1,
                                        enabled: !on,
                                    });
                                    ui.close_menu();
                                }
                                if self.links[i].switch_pos.is_some()
                                    && ui.button("Reset switch position").clicked()
                                {
                                    self.links[i].switch_pos = None;
                                    self.dirty = true;
                                    ui.close_menu();
                                }
                                if ui.button("Remove switch").clicked() {
                                    let was_off = !self.links[i].enabled;
                                    self.links[i].has_switch = false;
                                    self.links[i].switch_pos = None;
                                    self.links[i].enabled = true;
                                    self.dirty = true;
                                    if was_off {
                                        action = Some(Action::SetLinkEnabled {
                                            out_port: pair.0,
                                            in_port: pair.1,
                                            enabled: true,
                                        });
                                    }
                                    ui.close_menu();
                                }
                            }
                        }
                        ui.separator();
                    }
                    if ui.button("Add waypoint here").clicked() {
                        if let (Some(i), Some(&a), Some(&b)) =
                            (saved_idx, anchors.get(&pair.0), anchors.get(&pair.1))
                        {
                            let existing: Vec<Pos2> =
                                self.links[i].waypoints.iter().map(|&w| to_screen(w)).collect();
                            let k = waypoint_slot(self.link_style, a, &existing, b, click_pos, z, pair.0);
                            let world = (click_pos - origin) / z;
                            self.links[i].waypoints.insert(k, (world.x, world.y));
                            self.dirty = true;
                        }
                        ui.close_menu();
                    }
                    let has_wps = saved_idx.is_some_and(|i| !self.links[i].waypoints.is_empty());
                    if ui.add_enabled(has_wps, egui::Button::new("Clear waypoints")).clicked() {
                        if let Some(i) = saved_idx {
                            self.links[i].waypoints.clear();
                            self.dirty = true;
                        }
                        ui.close_menu();
                    }
                    if let Some(i) = saved_idx {
                        let mut linger = self.links[i].lingering;
                        if ui
                            .checkbox(&mut linger, "Keep after quit")
                            .on_hover_text("Set object.linger = true so PipeWire keeps this link alive after RedoxRouter closes")
                            .changed()
                        {
                            self.links[i].lingering = linger;
                            self.dirty = true;
                            action = Some(Action::SetLinkLinger {
                                out_port: pair.0,
                                in_port: pair.1,
                                lingering: linger,
                            });
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                    if ui.button("Delete link").clicked() {
                        let mut prompted = false;
                        if let Some(i) = saved_idx {
                            if let Some(pidx) = crate::links::find_stereo_partner_idx(&self.links, i, Some(g)) {
                                let partner_pair = crate::links::resolve(g, &self.links[pidx]);
                                let partner_connected = partner_pair.is_some_and(|(po, pi)| g.find_link(po, pi).is_some());
                                if self.links[i].stereo_coupled || partner_connected {
                                    self.prompt_stereo_disconnect = Some(StereoDisconnectPromptData {
                                        link_idx: i,
                                        partner_idx: pidx,
                                        pair,
                                        partner_pair,
                                    });
                                    prompted = true;
                                }
                            }
                        }
                        if !prompted {
                            action = Some(Action::Disconnect {
                                out_port: pair.0,
                                in_port: pair.1,
                            });
                        }
                        self.selected = None;
                        ui.close_menu();
                    }
                } else {
                    if ui.button("Make Redox connection").clicked() {
                        action = Some(Action::TakeOver {
                            out_port: pair.0,
                            in_port: pair.1,
                        });
                        ui.close_menu();
                    }
                    if self.revealed.contains(&pair) {
                        if ui.button("Hide external link").clicked() {
                            self.revealed.remove(&pair);
                            self.selected = None;
                            ui.close_menu();
                        }
                    }
                }
            } else {
                if ui.button("Add comment here").clicked() {
                    new_note = Some(NoteKind::Text);
                    ui.close_menu();
                }
                if ui.button("Add block here").clicked() {
                    new_note = Some(NoteKind::Block);
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("Fit to view (F)").clicked() {
                    self.fit_to_view();
                    ui.close_menu();
                }
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

        // Left-drag box: either Ctrl+drag to fit the drawn box to screen,
        // or normal/Shift+drag to select (Shift adds to selection).
        if let (Some((start, start_ctrl)), true) =
            (self.box_start, bg.drag_stopped_by(egui::PointerButton::Primary))
        {
            let end = ui.input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos()));
            if let Some(end) = end {
                let bx = Rect::from_two_pos(start, end);
                let is_zoom = start_ctrl || ui.input(|i| i.modifiers.ctrl);
                if is_zoom {
                    if bx.width() > 8.0 && bx.height() > 8.0 {
                        let w_min = (bx.min - origin) / z;
                        let w_max = (bx.max - origin) / z;
                        let (pan, zoom) = layout::fit_to_view(
                            (w_min.x, w_min.y),
                            (w_max.x, w_max.y),
                            (rect.width(), rect.height()),
                            16.0,
                        );
                        self.pan = vec2(pan.0, pan.1);
                        self.zoom = zoom;
                        self.dirty = true;
                    }
                } else {
                    let add = ui.input(|i| i.modifiers.shift);
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
                            match note.kind {
                                NoteKind::Text => {
                                    let (cr, _) = comment_rect_and_galley(&painter, origin, z, note);
                                    if cr.intersects(bx) {
                                        self.sel_notes.insert(note.id);
                                    }
                                }
                                NoteKind::Block => {
                                    let br = Rect::from_min_size(
                                        origin + vec2(note.pos.0, note.pos.1) * z,
                                        vec2(note.size.0, note.size.1) * z,
                                    );
                                    if bx.contains_rect(br) {
                                        self.sel_notes.insert(note.id);
                                    }
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
            }
            self.box_start = None;
        }
        // Drop selections that no longer exist.
        self.box_links.retain(|p| selectable.contains(p));
        self.sel_nodes.retain(|id| g.nodes.contains_key(id));
        self.sel_notes.retain(|id| self.notes.iter().any(|n| n.id == *id));
        let can_delete_key = !ui.ctx().wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Delete));
        if can_delete_key && !self.box_links.is_empty() {
            self.extra_disconnects.extend(self.box_links.drain());
        }
        if can_delete_key && !self.sel_notes.is_empty() {
            self.notes.retain(|n| !self.sel_notes.contains(&n.id));
            self.sel_notes.clear();
            self.group_sel = None;
            self.dirty = true;
        }

        // Left-click selects the nearest link (Shift+click adds); click on empty space clears.
        if bg.clicked() && !handle_hit {
            let add = ui.input(|i| i.modifiers.shift);
            if !add {
                self.sel_nodes.clear();
                self.sel_notes.clear();
                self.box_links.clear();
                self.group_sel = None;
            }
            let hit_link = if self.select_mode == SelectMode::Nodes {
                None
            } else {
                bg.interact_pointer_pos().and_then(|pos| {
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
                })
            };
            if add {
                if let Some(pair) = hit_link {
                    if let Some(prev) = self.selected {
                        if selectable.contains(&prev) {
                            self.box_links.insert(prev);
                        }
                    }
                    if selectable.contains(&pair) {
                        self.box_links.insert(pair);
                    }
                    self.selected = Some(pair);
                }
            } else {
                self.selected = hit_link;
            }
        }
        // Delete removes the selected link, but only if we own it.
        if let Some(pair) = self.selected {
            // Forget the selection if the link is gone or no longer drawn
            // (e.g. one of its nodes was hidden).
            let gone = !self.virtual_mode
                && g.find_link(pair.0, pair.1).is_none()
                && !selectable.contains(&pair);
            if gone || !shapes.iter().any(|(p, _)| *p == pair) {
                self.selected = None;
            } else if can_delete_key {
                let owned = self.virtual_mode
                    || selectable.contains(&pair)
                    || g.find_link(pair.0, pair.1).is_some_and(|l| g.is_owned(l));
                if owned {
                    let mut prompted = false;
                    let saved_idx = crate::links::from_ports(g, pair.0, pair.1)
                        .and_then(|s| crate::links::find(&self.links, &s));
                    if let Some(i) = saved_idx {
                        if let Some(p_idx) = crate::links::find_stereo_partner_idx(&self.links, i, Some(g)) {
                            let partner_pair = crate::links::resolve(g, &self.links[p_idx]);
                            let partner_connected = partner_pair.is_some_and(|(po, pi)| g.find_link(po, pi).is_some());
                            if self.links[i].stereo_coupled || partner_connected {
                                self.prompt_stereo_disconnect = Some(StereoDisconnectPromptData {
                                    link_idx: i,
                                    partner_idx: p_idx,
                                    pair,
                                    partner_pair,
                                });
                                prompted = true;
                            }
                        }
                    }
                    if !prompted {
                        action = Some(Action::Disconnect { out_port: pair.0, in_port: pair.1 });
                    }
                    self.selected = None;
                }
            }
        }

        // Pass 3: nodes.
        let any_node_active = self.theme.node_glow_enabled
            && items
                .iter()
                .any(|it| it.rect.intersects(rect) && g.is_node_active(it.node.id));
        if any_node_active {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        }
        for it in &items {
            let r = it.rect;
            if !r.intersects(rect) {
                continue;
            }
            let is_active = g.is_node_active(it.node.id);
            if is_active && self.theme.node_glow_enabled {
                let glow_col = match layout::column_for(&it.node.media_class) {
                    0 => self.theme.c_node_glow_source(),
                    2 => self.theme.c_node_glow_sink(),
                    _ => self.theme.c_node_glow_stream(),
                };
                for offset in [0.0_f32, 0.5_f32] {
                    let t = (time * 0.6_f32 + offset).fract();
                    let ease = 1.0_f32 - (1.0_f32 - t) * (1.0_f32 - t);
                    let expanded = r.expand((2.0_f32 + 8.0_f32 * ease) * z);
                    let alpha = ((1.0_f32 - t).powi(2) * 55.0_f32) as u8;
                    let stroke_col = Color32::from_rgba_unmultiplied(
                        glow_col.r(),
                        glow_col.g(),
                        glow_col.b(),
                        alpha,
                    );
                    let fill_col = Color32::from_rgba_unmultiplied(
                        glow_col.r(),
                        glow_col.g(),
                        glow_col.b(),
                        alpha / 5,
                    );
                    painter.rect_filled(expanded, CornerRadius::same(10), fill_col);
                    painter.rect_stroke(
                        expanded,
                        CornerRadius::same(10),
                        Stroke::new((1.5_f32 * (1.0_f32 - 0.4_f32 * t) * z).max(0.8_f32), stroke_col),
                        egui::StrokeKind::Inside,
                    );
                }
            }
            let default_color = match layout::column_for(&it.node.media_class) {
                0 => self.theme.c_col_source(),
                2 => self.theme.c_col_sink(),
                _ => self.theme.c_col_stream(),
            };
            let header_color = it.color.unwrap_or(default_color);
            // Soft shadow, thin border, body, then the colored header with a
            // subtle gloss and a divider line.
            painter.rect_filled(
                r.translate(vec2(0.0, 3.0 * z)).expand(3.0 * z),
                CornerRadius::same(9),
                Color32::from_black_alpha(if self.theme.is_dark { 55 } else { 25 }),
            );
            if self.sel_nodes.contains(&it.node.id) {
                painter.rect_filled(
                    r.expand(3.0 * z),
                    CornerRadius::same(9),
                    self.theme.c32_alpha(self.theme.accent, 150),
                );
            }
            painter.rect_filled(r.expand(z), CornerRadius::same(7), self.theme.c_node_stroke());
            // Nickname tab above the top-left corner, drawn BEFORE the node body and
            // reaching down behind the header, so it looks fused to the node and a
            // node drawn on top of it covers both. Click the pencil to rename.
            let show_text = z >= 0.4;
            let ndelta = self.theme.node_font_size_delta;
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
                    painter.layout_no_wrap(n.clone(), FontId::proportional((11.0 + ndelta).max(7.0) * z), Color32::WHITE)
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
                painter.rect_filled(full.expand(z), top, self.theme.c_node_stroke());
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
                let ps = Stroke::new((1.6 * z).max(1.0), Color32::from_white_alpha(220));
                painter.line_segment([c + vec2(-3.5, 3.5) * z, c + vec2(3.0, -3.0) * z], ps);
                painter.line_segment([c + vec2(-4.5, 5.5) * z, c + vec2(1.0, 5.5) * z], ps);
                if resp.clicked() {
                    self.editing = Some((nkey, tab.left_top()));
                    self.edit_buf = nick.unwrap_or_default();
                    self.edit_first = true;
                }
            }
            if self.virtual_mode && g.is_node_owned(it.node) {
                let tab_h = layout::TAB_H * z;
                let lift = 3.0 * z;
                let info_w = 22.0 * z;
                let info_tab = Rect::from_min_size(
                    pos2(r.max.x - info_w, r.max.y),
                    vec2(info_w, tab_h + lift),
                );
                let info_full = Rect::from_min_max(
                    pos2(info_tab.min.x, r.max.y - 12.0 * z),
                    info_tab.max,
                );
                let info_resp = ui
                    .interact(info_tab, ui.id().with(("virt_info", it.node.id)), Sense::hover())
                    .on_hover_text(
                        "Virtual nodes are per-tab and only show up as live nodes in PipeWire \
                         (and external tools like qpwgraph) when this setup tab is active.\n\
                         Click \"Make active\" in the tab bar to make this tab and its virtual nodes live.",
                    );
                let base_gray = if self.theme.is_dark {
                    Color32::from_rgb(76, 80, 92)
                } else {
                    Color32::from_rgb(165, 170, 182)
                };
                let info_fill = if info_resp.hovered() {
                    lighten(base_gray)
                } else {
                    base_gray
                };
                let bottom_round = CornerRadius { nw: 0, ne: 0, sw: 6, se: 6 };
                painter.rect_filled(info_full.expand(z), bottom_round, self.theme.c_node_stroke());
                painter.rect_filled(info_full, bottom_round, info_fill);
                let ic = info_tab.center();
                let icon_col = Color32::from_white_alpha(230);
                let is_stroke = Stroke::new((1.8_f32 * z).max(1.1_f32), icon_col);
                painter.circle_filled(ic + vec2(0.0, -4.0 * z), (1.3_f32 * z).max(1.0_f32), icon_col);
                painter.line_segment([ic + vec2(0.0, -1.0 * z), ic + vec2(0.0, 4.5 * z)], is_stroke);
            }
            painter.rect_filled(r, CornerRadius::same(6), self.theme.c_node_body());
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
                Stroke::new(1.0_f32, Color32::from_black_alpha(if self.theme.is_dark { 90 } else { 40 })),
            );

            let clip = painter.with_clip_rect(r.intersect(rect));
            if show_text {
                let white = Color32::WHITE;
                let g = lay(it.node.label(), FontId::proportional((13.0 + ndelta).max(8.0) * z), white, (it.w - 16.0) * z);
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
                    self.theme.c_port_out()
                } else {
                    self.theme.c_port_in()
                };
                painter.circle(
                    a,
                    4.5 * z,
                    color,
                    Stroke::new(
                        1.5 * z,
                        if self.theme.is_dark {
                            Color32::from_rgb(24, 25, 30)
                        } else {
                            Color32::from_rgb(210, 212, 220)
                        },
                    ),
                );
                if show_text {
                    let port_col = self.theme.c_node_text();
                    let g = lay(&port.name, FontId::proportional((11.0 + ndelta).max(7.0) * z), port_col, half * z);
                    let dy = -g.size().y / 2.0;
                    let pos = if is_out {
                        a + vec2(-8.0 * z - g.size().x, dy)
                    } else {
                        a + vec2(8.0 * z, dy)
                    };
                    clip.galley(pos, g, port_col);
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
                if can_edit && (resp.dragged_by(egui::PointerButton::Primary) || resp.clicked()) {
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
                    (
                        if self.theme.is_dark { Color32::from_gray(55) } else { Color32::from_gray(215) },
                        Color32::from_gray(100),
                    )
                } else {
                    (
                        if self.theme.is_dark { Color32::from_gray(58) } else { Color32::from_gray(215) },
                        self.theme.c_accent(),
                    )
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
                    let col = if vol.mute { Color32::from_rgb(230, 120, 110) } else { self.theme.c_node_text() };
                    let g = lay(&text, FontId::proportional((11.0 + ndelta).max(7.0) * z), col, 1000.0);
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
                    self.theme.c32_alpha(self.theme.accent, 150),
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
                    self.box_links.clear();
                    self.selected = None;
                    self.group_sel = None;
                }
                self.sel_notes.insert(n.id);
            }
            if resp.dragged_by(egui::PointerButton::Primary) {
                let d = resp.drag_delta() / z;
                self.notes[i].pos = (n.pos.0 + d.x, n.pos.1 + d.y);
                if self.sel_notes.contains(&n.id) {
                    self.move_selected(g, d, None, Some(n.id));
                }
            }
            if resp.drag_stopped_by(egui::PointerButton::Primary) {
                if snap_active {
                    self.notes[i].pos = layout::snap_pos(self.notes[i].pos, grid_unit);
                }
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
            if gresp.dragged_by(egui::PointerButton::Primary) {
                let d = gresp.drag_delta().x / z;
                self.notes[i].size.0 = (n.size.0 + d).max(80.0);
            }
            if gresp.drag_stopped_by(egui::PointerButton::Primary) {
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

        // Pass 4: the link currently being dragged, plus a growing/fading ping circle
        // when close enough to a compatible port to connect on release.
        if let Some(from) = self.pending {
            let pointer = ui.input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos()));
            if let (Some(&a), Some(m)) = (anchors.get(&from.port), pointer) {
                let target_port = self.target_at(m, &anchors, g, from);
                let (s, e) = if from.from_out { (a, m) } else { (m, a) };
                let pts = link_path(self.link_style, s, &[], e, z, 0);
                painter.add(Shape::line(
                    pts,
                    Stroke::new((2.4 * z).clamp(1.2, 3.4), self.theme.c_cable_selected()),
                ));
                if let Some(tid) = target_port {
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
                    let port_col = if from.from_out {
                        self.theme.c_port_in() // target is an input port
                    } else {
                        self.theme.c_port_out() // target is an output port
                    };
                    let mut ping_ports = vec![tid];
                    if ui.input(|i| i.modifiers.shift) {
                        let (out_p, in_p) = if from.from_out {
                            (from.port, tid)
                        } else {
                            (tid, from.port)
                        };
                        if let Some((out2, in2)) = crate::links::stereo_partner_ports(g, out_p, in_p) {
                            ping_ports.push(if from.from_out { in2 } else { out2 });
                        }
                    }
                    for pid in ping_ports {
                        if let Some(&t_anchor) = anchors.get(&pid) {
                            for offset in [0.0_f32, 0.5_f32] {
                                let t = (time * 1.6 + offset).fract();
                                let ease = 1.0 - (1.0 - t) * (1.0 - t);
                                let radius = (4.5 + 16.0 * ease) * z.max(0.75);
                                let alpha = ((1.0 - t).powi(2) * 235.0) as u8;
                                let ring_col = Color32::from_rgba_unmultiplied(
                                    port_col.r(),
                                    port_col.g(),
                                    port_col.b(),
                                    alpha,
                                );
                                let fill_col = Color32::from_rgba_unmultiplied(
                                    port_col.r(),
                                    port_col.g(),
                                    port_col.b(),
                                    alpha / 5,
                                );
                                painter.circle_filled(t_anchor, radius, fill_col);
                                painter.circle_stroke(
                                    t_anchor,
                                    radius,
                                    Stroke::new((2.2 * (1.0 - 0.5 * t) * z).max(1.0), ring_col),
                                );
                            }
                            painter.circle(
                                t_anchor,
                                5.6 * z,
                                port_col,
                                Stroke::new(1.8 * z, Color32::WHITE),
                            );
                        }
                    }
                }
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

        // Box being dragged (selection box, or Ctrl+drag zoom-to-screen box).
        if let Some((start, start_ctrl)) = self.box_start {
            if let Some(cur) = ui.input(|i| i.pointer.latest_pos()) {
                let bx = Rect::from_two_pos(start, cur);
                let is_zoom = start_ctrl || ui.input(|i| i.modifiers.ctrl);
                if is_zoom {
                    painter.rect_filled(
                        bx,
                        CornerRadius::same(3),
                        self.theme.c32_alpha(self.theme.cable_selected, 28),
                    );
                    painter.rect_stroke(
                        bx,
                        CornerRadius::same(3),
                        Stroke::new(1.5_f32, self.theme.c_cable_selected()),
                        egui::StrokeKind::Inside,
                    );
                    if bx.width() > 60.0 && bx.height() > 24.0 {
                        painter.text(
                            bx.min + vec2(6.0, 4.0),
                            egui::Align2::LEFT_TOP,
                            "Fit box to screen",
                            FontId::proportional(11.0),
                            self.theme.c_cable_selected(),
                        );
                    }
                } else {
                    painter.rect_filled(
                        bx,
                        CornerRadius::same(2),
                        self.theme.c32_alpha(self.theme.accent, 30),
                    );
                    painter.rect_stroke(
                        bx,
                        CornerRadius::same(2),
                        Stroke::new(1.0_f32, self.theme.c_accent()),
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }

        // Bottom-left: interactive Minimap.
        if self.show_minimap && (!items.is_empty() || !self.notes.is_empty()) {
            let map_size = vec2(170.0, 110.0);
            let map_rect = Rect::from_min_size(
                pos2(rect.min.x + 12.0, rect.max.y - map_size.y - 12.0),
                map_size,
            );
            let mut min_x = f32::INFINITY;
            let mut min_y = f32::INFINITY;
            let mut max_x = f32::NEG_INFINITY;
            let mut max_y = f32::NEG_INFINITY;
            for it in &items {
                if let Some(&(x, y)) = self.pos.get(&it.node.id) {
                    let (w, h) = self.node_sizes.get(&it.node.id).copied().unwrap_or((it.w, 60.0));
                    min_x = min_x.min(x);
                    min_y = min_y.min(y);
                    max_x = max_x.max(x + w);
                    max_y = max_y.max(y + h);
                }
            }
            for n in &self.notes {
                min_x = min_x.min(n.pos.0);
                min_y = min_y.min(n.pos.1);
                max_x = max_x.max(n.pos.0 + n.size.0.max(80.0));
                max_y = max_y.max(n.pos.1 + n.size.1.max(40.0));
            }
            // Include current camera viewport in world bounds so the camera box stays visible.
            let cam_min = (-self.pan.x / z, -self.pan.y / z);
            let cam_max = (
                (rect.width() - self.pan.x) / z,
                (rect.height() - self.pan.y) / z,
            );
            min_x = min_x.min(cam_min.0) - 40.0;
            min_y = min_y.min(cam_min.1) - 40.0;
            max_x = max_x.max(cam_max.0) + 40.0;
            max_y = max_y.max(cam_max.1) + 40.0;

            let world_w = (max_x - min_x).max(1.0);
            let world_h = (max_y - min_y).max(1.0);
            let inner = map_rect.shrink(6.0);
            let scale = (inner.width() / world_w).min(inner.height() / world_h);
            let map_origin = inner.center() - vec2(world_w * scale * 0.5, world_h * scale * 0.5);
            let to_map = |wx: f32, wy: f32| {
                map_origin + vec2((wx - min_x) * scale, (wy - min_y) * scale)
            };

            painter.rect_filled(
                map_rect,
                CornerRadius::same(8),
                self.theme.c32_alpha(self.theme.panel_bg, 220),
            );

            let mp = painter.with_clip_rect(inner);
            for n in &self.notes {
                if n.kind == NoteKind::Block {
                    let a = to_map(n.pos.0, n.pos.1);
                    let b = to_map(n.pos.0 + n.size.0, n.pos.1 + n.size.1);
                    let c = n.color;
                    mp.rect_filled(
                        Rect::from_min_max(a, b),
                        CornerRadius::same(2),
                        Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 50),
                    );
                }
            }
            for it in &items {
                if let Some(&(x, y)) = self.pos.get(&it.node.id) {
                    let (w, h) = self.node_sizes.get(&it.node.id).copied().unwrap_or((it.w, 60.0));
                    let a = to_map(x, y);
                    let b = to_map(x + w, y + h);
                    let default_col = match layout::column_for(&it.node.media_class) {
                        0 => self.theme.c_col_source(),
                        2 => self.theme.c_col_sink(),
                        _ => self.theme.c_col_stream(),
                    };
                    let col = if self.sel_nodes.contains(&it.node.id) {
                        self.theme.c_accent()
                    } else {
                        it.color.unwrap_or(default_col)
                    };
                    mp.rect_filled(Rect::from_min_max(a, b), CornerRadius::same(2), col);
                }
            }
            let vp_rect = Rect::from_min_max(
                to_map(cam_min.0, cam_min.1),
                to_map(cam_max.0, cam_max.1),
            );
            mp.rect_stroke(
                vp_rect,
                CornerRadius::same(2),
                Stroke::new(1.2_f32, self.theme.c_accent()),
                egui::StrokeKind::Inside,
            );

            let mresp = ui
                .interact(map_rect, ui.id().with("minimap"), Sense::click_and_drag())
                .on_hover_text("Minimap: click or drag to pan");
            if mresp.clicked() || mresp.dragged_by(egui::PointerButton::Primary) {
                if let Some(p) = mresp.interact_pointer_pos() {
                    let wx = min_x + (p.x - map_origin.x) / scale;
                    let wy = min_y + (p.y - map_origin.y) / scale;
                    self.pan = vec2(rect.width() * 0.5 - wx * z, rect.height() * 0.5 - wy * z);
                    self.dirty = true;
                }
            }
        }

        // Bottom-right: what box select / clicks may pick, plus Align/Distribute when 2+ nodes are selected.
        let mut bottom_align_op: Option<u8> = None;
        egui::Area::new(egui::Id::new("select_mode"))
            .anchor(egui::Align2::RIGHT_BOTTOM, vec2(-12.0, -12.0))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    if self.sel_nodes.len() >= 2 {
                        ui.horizontal(|ui| {
                            ui.weak(format!("{} nodes:", self.sel_nodes.len()));
                            if ui.small_button("Align L").on_hover_text("Align selected nodes to left edge").clicked() {
                                bottom_align_op = Some(1);
                            }
                            if ui.small_button("Align T").on_hover_text("Align selected nodes to top edge").clicked() {
                                bottom_align_op = Some(2);
                            }
                            if ui.small_button("Dist V").on_hover_text("Distribute selected nodes vertically").clicked() {
                                bottom_align_op = Some(3);
                            }
                            if ui.small_button("Dist H").on_hover_text("Distribute selected nodes horizontally").clicked() {
                                bottom_align_op = Some(4);
                            }
                        });
                    }
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
                    ui.small("Left-drag = select • Shift = add • Ctrl+drag = zoom box • Middle-drag = pan");
                });
            });
        match bottom_align_op {
            Some(1) => self.align_selected_left(g),
            Some(2) => self.align_selected_top(g),
            Some(3) => self.distribute_selected_vertical(g),
            Some(4) => self.distribute_selected_horizontal(g),
            _ => {}
        }
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
                let Some(p) = g.ports.get(id) else { return false };
                if p.node_id == src_node || (p.direction == Direction::Out) == from.from_out {
                    return false;
                }
                let (out_p, in_p) = if from.from_out { (from.port, **id) } else { (**id, from.port) };
                g.can_link_ports(out_p, in_p)
            })
            .map(|(id, a)| (*id, a.distance(pos)))
            .filter(|(_, d)| *d <= reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_polyline_at_midpoint() {
        let pts = vec![pos2(0.0, 0.0), pos2(10.0, 0.0), pos2(10.0, 20.0)];
        let (before, mid, after) = split_polyline_mid(&pts);
        // Total length is 10 + 20 = 30, so midpoint is at distance 15 -> (10.0, 5.0).
        assert!((mid.x - 10.0).abs() < 1e-4 && (mid.y - 5.0).abs() < 1e-4);
        assert_eq!(before.first().copied(), Some(pos2(0.0, 0.0)));
        assert_eq!(before.last().copied(), Some(mid));
        assert_eq!(after.first().copied(), Some(mid));
        assert_eq!(after.last().copied(), Some(pos2(10.0, 20.0)));
    }

    #[test]
    fn polyline_ribbon_mesh_generation() {
        let pts = vec![pos2(0.0, 0.0), pos2(100.0, 0.0), pos2(100.0, 100.0)];
        let mesh = build_polyline_ribbon(&pts, 4.0, Color32::RED, Color32::BLUE, None);
        assert_eq!(mesh.vertices.len(), 6);
        assert_eq!(mesh.indices.len(), 12);
        assert_eq!(mesh.vertices[0].color, Color32::RED);
        assert_eq!(mesh.vertices[5].color, Color32::BLUE);
    }
}
