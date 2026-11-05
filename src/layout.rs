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

pub type P = (f32, f32);

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
}
