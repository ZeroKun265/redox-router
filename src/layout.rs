//! Pure layout math for the node canvas (std only, unit-testable).
//! All coordinates are f32 pairs; "world" units are unzoomed canvas units.

pub const NODE_W: f32 = 200.0;
pub const HEADER_H: f32 = 26.0;
pub const ROW_H: f32 = 20.0;
/// Height of the volume slider row at the bottom of audio nodes.
pub const VOL_ROW_H: f32 = 24.0;
pub const PAD: f32 = 8.0;
pub const MAX_NODE_W: f32 = 480.0;
/// Limits when the user resizes a node by hand.
pub const MIN_RESIZE_W: f32 = 120.0;
pub const MAX_RESIZE_W: f32 = 1200.0;
/// Height of the nickname tab above a node.
pub const TAB_H: f32 = 18.0;
/// Horizontal distance between the left edges of the three initial columns.
pub const COL_PITCH: f32 = 340.0;
pub const ROW_GAP: f32 = 20.0;
pub const MIN_ZOOM: f32 = 0.2;
pub const MAX_ZOOM: f32 = 4.0;
/// Base grid spacing in world units (matches the dot grid step of 28.0).
pub const GRID_STEP: f32 = 28.0;

pub type P = (f32, f32);

use serde::{Deserialize, Serialize};

/// How cables are drawn between ports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LinkStyle {
    #[default]
    Curved,
    /// Horizontal and vertical segments with rounded corners.
    Zigzag,
}

impl LinkStyle {
    pub const ALL: [LinkStyle; 2] = [LinkStyle::Curved, LinkStyle::Zigzag];

    pub fn label(self) -> &'static str {
        match self {
            LinkStyle::Curved => "Curved cables",
            LinkStyle::Zigzag => "Zigzag cables",
        }
    }
}

/// Corner points of a horizontal/vertical cable from output `a` (leaves to the
/// right) to input `b` (enters from the left). `stub` is the minimum straight
/// piece next to a port; `lane` shifts the middle segment so cables between the
/// same nodes do not lie exactly on top of each other.
pub fn zigzag_points(a: P, b: P, stub: f32, lane: f32) -> Vec<P> {
    let mut v = vec![a];
    if b.0 - a.0 >= 2.0 * stub {
        let xm = ((a.0 + b.0) / 2.0 + lane).clamp(a.0 + stub, b.0 - stub);
        v.push((xm, a.1));
        v.push((xm, b.1));
    } else {
        // Input is left of (or just right of) the output: go around.
        let (x1, x2) = (a.0 + stub, b.0 - stub);
        let mut ym = (a.1 + b.1) / 2.0 + lane;
        if (a.1 - b.1).abs() < stub {
            ym = a.1.max(b.1) + stub * 2.0 + lane.abs();
        }
        v.extend([(x1, a.1), (x1, ym), (x2, ym), (x2, b.1)]);
    }
    v.push(b);
    v.dedup_by(|p, q| (p.0 - q.0).abs() < 1e-3 && (p.1 - q.1).abs() < 1e-3);
    v
}

/// Round the corners of a polyline with quadratic curves (`radius` is capped at
/// half of each adjacent segment). First and last point are kept.
pub fn round_corners(pts: &[P], radius: f32, segs: usize) -> Vec<P> {
    if pts.len() < 3 || radius <= 0.0 {
        return pts.to_vec();
    }
    let len = |p: P, q: P| ((q.0 - p.0).powi(2) + (q.1 - p.1).powi(2)).sqrt();
    let mut out = vec![pts[0]];
    for w in pts.windows(3) {
        let (p, c, n) = (w[0], w[1], w[2]);
        let (l1, l2) = (len(p, c), len(c, n));
        if l1 < 1e-3 || l2 < 1e-3 {
            out.push(c);
            continue;
        }
        let r = radius.min(l1 / 2.0).min(l2 / 2.0);
        let s = (c.0 + (p.0 - c.0) / l1 * r, c.1 + (p.1 - c.1) / l1 * r);
        let e = (c.0 + (n.0 - c.0) / l2 * r, c.1 + (n.1 - c.1) / l2 * r);
        for i in 0..=segs {
            let t = i as f32 / segs as f32;
            let u = 1.0 - t;
            out.push((
                u * u * s.0 + 2.0 * u * t * c.0 + t * t * e.0,
                u * u * s.1 + 2.0 * u * t * c.1 + t * t * e.1,
            ));
        }
    }
    out.push(pts[pts.len() - 1]);
    out
}

use std::collections::HashMap;

/// Try to make node `id` the holder of saved-position `key`. Several live nodes
/// can share a name (e.g. two browser streams); only one may own the saved spot.
/// Succeeds if the key is free, already ours, or held by a node that is gone.
pub fn try_claim(
    claimed: &mut HashMap<String, u32>,
    is_alive: impl Fn(u32) -> bool,
    key: &str,
    id: u32,
) -> bool {
    match claimed.get(key) {
        Some(&owner) if owner != id && is_alive(owner) => false,
        _ => {
            claimed.insert(key.to_string(), id);
            true
        }
    }
}

/// Saved position for `key` if it exists and node `id` may claim it.
pub fn claim_saved(
    saved: &HashMap<String, P>,
    claimed: &mut HashMap<String, u32>,
    is_alive: impl Fn(u32) -> bool,
    key: &str,
    id: u32,
) -> Option<P> {
    let p = *saved.get(key)?;
    try_claim(claimed, is_alive, key, id).then_some(p)
}

/// Node width from measured text widths (world units): wide enough for the
/// title, and for the longest input label plus the longest output label side by
/// side so labels on the same row never overlap.
pub fn node_width(title_w: f32, max_in_w: f32, max_out_w: f32) -> f32 {
    (title_w + 16.0)
        .max(max_in_w + max_out_w + 40.0)
        .clamp(NODE_W, MAX_NODE_W)
}

/// Vertical layout of a node: `header_h` then one row per entry of `rows`
/// (row heights, wrapped text makes rows taller). Returns (total height, y of
/// each row's center), relative to the node's top edge.
pub fn node_rows(header_h: f32, rows: &[f32]) -> (f32, Vec<f32>) {
    let mut y = header_h;
    let mut centers = Vec::with_capacity(rows.len());
    for &h in rows {
        centers.push(y + h / 2.0);
        y += h;
    }
    (y + PAD, centers)
}

/// Initial column: 0 = producers, 1 = other, 2 = consumers.
pub fn column_for(media_class: &str) -> usize {
    if media_class.contains("Sink") || media_class.contains("Input") {
        2
    } else if media_class.contains("Source") || media_class.contains("Output") {
        0
    } else {
        1
    }
}

/// Stacks newly seen nodes down three columns.
#[derive(Default)]
pub struct Placer {
    next_y: [f32; 3],
}

impl Placer {
    pub fn place(&mut self, col: usize, height: f32) -> P {
        let col = col.min(2);
        let y = self.next_y[col];
        self.next_y[col] += height + ROW_GAP;
        (col as f32 * COL_PITCH, y)
    }
}

/// Zoom by `factor` keeping the world point under `cursor` fixed.
/// `pan` and `cursor` are relative to the canvas top-left. Returns (pan, zoom).
/// Screen position = pan + world * zoom.
pub fn zoom_about(pan: P, zoom: f32, cursor: P, factor: f32) -> (P, f32) {
    let new_zoom = (zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    let world = ((cursor.0 - pan.0) / zoom, (cursor.1 - pan.1) / zoom);
    let new_pan = (cursor.0 - world.0 * new_zoom, cursor.1 - world.1 * new_zoom);
    (new_pan, new_zoom)
}

/// Sampled cubic bezier from `a` to `b` with horizontal tangents (cable look).
/// `min_ctrl` is the minimum handle length in the same units as the points.
pub fn bezier_points(a: P, b: P, segs: usize, min_ctrl: f32) -> Vec<P> {
    let dx = ((b.0 - a.0).abs() * 0.5).max(min_ctrl);
    let c1 = (a.0 + dx, a.1);
    let c2 = (b.0 - dx, b.1);
    (0..=segs)
        .map(|i| {
            let t = i as f32 / segs as f32;
            let u = 1.0 - t;
            let (w0, w1, w2, w3) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            (
                w0 * a.0 + w1 * c1.0 + w2 * c2.0 + w3 * b.0,
                w0 * a.1 + w1 * c1.1 + w2 * c2.1 + w3 * b.1,
            )
        })
        .collect()
}

/// Distance from point `p` to the segment `a`-`b`.
pub fn dist_to_segment(p: P, a: P, b: P) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

/// Distance from `p` to a polyline (infinite for fewer than two points).
pub fn dist_to_polyline(p: P, pts: &[P]) -> f32 {
    pts.windows(2)
        .map(|w| dist_to_segment(p, w[0], w[1]))
        .fold(f32::INFINITY, f32::min)
}

/// Snap a world position `(x, y)` to the nearest multiple of `grid`.
pub fn snap_pos(p: P, grid: f32) -> P {
    if grid <= 0.0 {
        return p;
    }
    ((p.0 / grid).round() * grid, (p.1 / grid).round() * grid)
}

/// Compute `(pan, zoom)` so the world bounding box `[min .. max]` fits centered inside
/// `view_size` with `margin` pixels on each side.
pub fn fit_to_view(min: P, max: P, view_size: P, margin: f32) -> (P, f32) {
    let bw = (max.0 - min.0).max(1.0);
    let bh = (max.1 - min.1).max(1.0);
    let avail_w = (view_size.0 - 2.0 * margin).max(40.0);
    let avail_h = (view_size.1 - 2.0 * margin).max(40.0);
    let zoom = (avail_w / bw).min(avail_h / bh).clamp(MIN_ZOOM, 1.5);
    let cx = (min.0 + max.0) * 0.5;
    let cy = (min.1 + max.1) * 0.5;
    let pan = (view_size.0 * 0.5 - cx * zoom, view_size.1 * 0.5 - cy * zoom);
    (pan, zoom)
}

/// Align all items `(id, pos, size)` to the minimum X coordinate among them.
pub fn align_left(items: &mut [(u32, P, P)]) {
    let Some(min_x) = items.iter().map(|(_, p, _)| p.0).reduce(f32::min) else { return };
    for (_, p, _) in items {
        p.0 = min_x;
    }
}

/// Align all items `(id, pos, size)` to the minimum Y coordinate among them.
pub fn align_top(items: &mut [(u32, P, P)]) {
    let Some(min_y) = items.iter().map(|(_, p, _)| p.1).reduce(f32::min) else { return };
    for (_, p, _) in items {
        p.1 = min_y;
    }
}

/// Distribute items `(id, pos, size)` evenly vertically (sorted by current Y).
pub fn distribute_vertical(items: &mut [(u32, P, P)]) {
    if items.len() < 2 {
        return;
    }
    items.sort_by(|a, b| a.1 .1.total_cmp(&b.1 .1).then_with(|| a.0.cmp(&b.0)));
    let first_y = items.first().unwrap().1 .1;
    let last_bottom = items.last().map(|(_, p, s)| p.1 + s.1).unwrap();
    let total_h: f32 = items.iter().map(|(_, _, s)| s.1).sum();
    let span = last_bottom - first_y;
    let gap = if span > total_h && items.len() > 1 {
        (span - total_h) / (items.len() - 1) as f32
    } else {
        ROW_GAP
    };
    let mut y = first_y;
    for (_, p, s) in items.iter_mut() {
        p.1 = y;
        y += s.1 + gap;
    }
}

/// Distribute items `(id, pos, size)` evenly horizontally (sorted by current X).
pub fn distribute_horizontal(items: &mut [(u32, P, P)]) {
    if items.len() < 2 {
        return;
    }
    items.sort_by(|a, b| a.1 .0.total_cmp(&b.1 .0).then_with(|| a.0.cmp(&b.0)));
    let first_x = items.first().unwrap().1 .0;
    let last_right = items.last().map(|(_, p, s)| p.0 + s.0).unwrap();
    let total_w: f32 = items.iter().map(|(_, _, s)| s.0).sum();
    let span = last_right - first_x;
    let gap = if span > total_w && items.len() > 1 {
        (span - total_w) / (items.len() - 1) as f32
    } else {
        40.0
    };
    let mut x = first_x;
    for (_, p, s) in items.iter_mut() {
        p.0 = x;
        x += s.0 + gap;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn rows_stack_below_header() {
        let (h, c) = node_rows(26.0, &[20.0, 40.0]);
        assert_eq!(c, vec![36.0, 66.0]);
        assert_eq!(h, 26.0 + 60.0 + PAD);
        let (h, c) = node_rows(30.0, &[]);
        assert!(c.is_empty());
        assert_eq!(h, 30.0 + PAD);
    }

    #[test]
    fn width_fits_labels_within_limits() {
        assert_eq!(node_width(10.0, 20.0, 20.0), NODE_W); // small: minimum width
        assert_eq!(node_width(300.0, 20.0, 20.0), 316.0); // title decides
        assert_eq!(node_width(50.0, 150.0, 120.0), 310.0); // in + out + gap decides
        assert_eq!(node_width(900.0, 20.0, 20.0), MAX_NODE_W); // capped
    }

    #[test]
    fn columns_by_media_class() {
        assert_eq!(column_for("Audio/Sink"), 2);
        assert_eq!(column_for("Audio/Source"), 0);
        assert_eq!(column_for("Stream/Output/Audio"), 0);
        assert_eq!(column_for("Stream/Input/Audio"), 2);
        assert_eq!(column_for("Midi/Bridge"), 1);
        assert_eq!(column_for(""), 1);
    }

    #[test]
    fn placer_stacks_per_column() {
        let mut p = Placer::default();
        let a = p.place(0, 100.0);
        let b = p.place(0, 50.0);
        let c = p.place(2, 10.0);
        assert_eq!(a, (0.0, 0.0));
        assert_eq!(b, (0.0, 100.0 + ROW_GAP));
        assert_eq!(c, (2.0 * COL_PITCH, 0.0));
    }

    #[test]
    fn zoom_keeps_cursor_point_fixed() {
        let (pan, zoom, cursor) = ((30.0, -10.0), 1.5, (200.0, 120.0));
        let world = ((cursor.0 - pan.0) / zoom, (cursor.1 - pan.1) / zoom);
        let (np, nz) = zoom_about(pan, zoom, cursor, 1.3);
        assert!(close(np.0 + world.0 * nz, cursor.0));
        assert!(close(np.1 + world.1 * nz, cursor.1));
    }

    #[test]
    fn zoom_is_clamped() {
        let (_, z) = zoom_about((0.0, 0.0), 1.0, (0.0, 0.0), 1000.0);
        assert_eq!(z, MAX_ZOOM);
        let (_, z) = zoom_about((0.0, 0.0), 1.0, (0.0, 0.0), 0.0001);
        assert_eq!(z, MIN_ZOOM);
    }

    #[test]
    fn zigzag_forward_is_axis_aligned_and_keeps_ends() {
        let v = zigzag_points((0.0, 0.0), (200.0, 80.0), 20.0, 0.0);
        assert_eq!(v, vec![(0.0, 0.0), (100.0, 0.0), (100.0, 80.0), (200.0, 80.0)]);
        // The lane moves the middle segment but never into the stubs.
        let v = zigzag_points((0.0, 0.0), (200.0, 80.0), 20.0, 500.0);
        assert_eq!(v[1].0, 180.0);
        let v = zigzag_points((0.0, 0.0), (200.0, 80.0), 20.0, -500.0);
        assert_eq!(v[1].0, 20.0);
    }

    #[test]
    fn zigzag_backward_goes_around() {
        let v = zigzag_points((300.0, 0.0), (100.0, 100.0), 20.0, 0.0);
        assert_eq!(v.first(), Some(&(300.0, 0.0)));
        assert_eq!(v.last(), Some(&(100.0, 100.0)));
        assert_eq!(v[1], (320.0, 0.0)); // leaves to the right
        assert_eq!(v[v.len() - 2], (80.0, 100.0)); // enters from the left
        for w in v.windows(2) {
            assert!(w[0].0 == w[1].0 || w[0].1 == w[1].1, "segment not axis aligned");
        }
        // Same height: detours below instead of running through the nodes.
        let v = zigzag_points((300.0, 50.0), (100.0, 50.0), 20.0, 0.0);
        assert!(v.iter().any(|p| p.1 > 50.0));
    }

    #[test]
    fn rounded_corners_keep_endpoints_and_stay_inside() {
        let v = zigzag_points((0.0, 0.0), (200.0, 80.0), 20.0, 0.0);
        let r = round_corners(&v, 15.0, 6);
        assert_eq!(r.first(), v.first());
        assert_eq!(r.last(), v.last());
        assert!(r.len() > v.len());
        assert!(r.iter().all(|p| p.0 >= 0.0 && p.0 <= 200.0 && p.1 >= 0.0 && p.1 <= 80.0));
        // Too short segments cap the radius instead of overshooting.
        let tiny = round_corners(&[(0.0, 0.0), (10.0, 0.0), (10.0, 4.0)], 50.0, 4);
        assert!(tiny.iter().all(|p| p.0 <= 10.0 + 1e-3 && p.1 >= -1e-3 && p.1 <= 4.0 + 1e-3));
        assert_eq!(round_corners(&[(0.0, 0.0), (1.0, 1.0)], 5.0, 4).len(), 2);
    }

    #[test]
    fn bezier_hits_endpoints() {
        let pts = bezier_points((0.0, 0.0), (100.0, 50.0), 10, 40.0);
        assert_eq!(pts.len(), 11);
        assert!(close(pts[0].0, 0.0) && close(pts[0].1, 0.0));
        assert!(close(pts[10].0, 100.0) && close(pts[10].1, 50.0));
    }

    #[test]
    fn segment_and_polyline_distance() {
        assert!(close(dist_to_segment((5.0, 3.0), (0.0, 0.0), (10.0, 0.0)), 3.0));
        // Beyond the end: distance to the nearest endpoint.
        assert!(close(dist_to_segment((13.0, 4.0), (0.0, 0.0), (10.0, 0.0)), 5.0));
        // Degenerate segment.
        assert!(close(dist_to_segment((3.0, 4.0), (0.0, 0.0), (0.0, 0.0)), 5.0));
        let line = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)];
        assert!(close(dist_to_polyline((12.0, 5.0), &line), 2.0));
        assert!(dist_to_polyline((0.0, 0.0), &[(1.0, 1.0)]).is_infinite());
    }

    #[test]
    fn saved_position_claims() {
        let mut saved = HashMap::new();
        saved.insert("mic".to_string(), (5.0, 6.0));
        let mut claimed = HashMap::new();
        let alive = |id: u32| id == 1 || id == 2;

        assert_eq!(claim_saved(&saved, &mut claimed, alive, "nope", 1), None);
        // First node with the name gets the saved spot.
        assert_eq!(claim_saved(&saved, &mut claimed, alive, "mic", 1), Some((5.0, 6.0)));
        // A second live node with the same name does not.
        assert_eq!(claim_saved(&saved, &mut claimed, alive, "mic", 2), None);
        // The owner can claim again (e.g. every frame).
        assert_eq!(claim_saved(&saved, &mut claimed, alive, "mic", 1), Some((5.0, 6.0)));
        // Once the owner is gone, the other node takes over the spot.
        let only_two = |id: u32| id == 2;
        assert_eq!(claim_saved(&saved, &mut claimed, only_two, "mic", 2), Some((5.0, 6.0)));
    }

    #[test]
    fn snap_fit_and_align_distribute() {
        assert_eq!(snap_pos((15.0, -13.0), 28.0), (28.0, -0.0));
        assert_eq!(snap_pos((41.0, 55.0), 28.0), (28.0, 56.0));

        let (pan, zoom) = fit_to_view((0.0, 0.0), (400.0, 200.0), (1000.0, 600.0), 50.0);
        assert!(zoom >= MIN_ZOOM && zoom <= 1.5);
        // Center of bounds (200, 100) maps to center of view (500, 300).
        assert!(close(pan.0 + 200.0 * zoom, 500.0));
        assert!(close(pan.1 + 100.0 * zoom, 300.0));

        let mut items = vec![
            (1, (100.0, 300.0), (200.0, 50.0)),
            (2, (40.0, 100.0), (200.0, 50.0)),
            (3, (80.0, 200.0), (200.0, 50.0)),
        ];
        align_left(&mut items);
        assert!(items.iter().all(|(_, p, _)| p.0 == 40.0));
        distribute_vertical(&mut items);
        assert_eq!(items[0].0, 2);
        assert_eq!(items[1].0, 3);
        assert_eq!(items[2].0, 1);
        assert!(close(items[0].1 .1, 100.0));
        assert!(close(items[1].1 .1, 200.0));
        assert!(close(items[2].1 .1, 300.0));
    }
}
