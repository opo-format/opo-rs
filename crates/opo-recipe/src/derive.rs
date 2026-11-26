//! Plans derived from 3D geometry (spec §4.6, derived representations): the horizontal cut
//! through the solids at the view's cut height, and the edges below it that are visible from
//! above. Plans made this way always match the model.

use std::collections::HashMap;

use geo::{BooleanOps, Coord, LineString, MultiPolygon, Polygon};

use opo_feel::Value;
use opo_model::{
    Derive, DrawingView, Lod, LodTable, Representation, RepresentationItem, TypeDefinition,
    Visibility,
};

use crate::Error;
use crate::geometry::Solid;
use crate::run::{Evaluation, resolve_material};

type V3 = [f64; 3];
/// A point rounded to micrometres, for matching shared vertices.
type Key3 = (i64, i64, i64);
/// An edge's two ends and the normals of the faces that meet there.
type EdgeFaces = (V3, V3, Vec<V3>);

/// A tiny offset so that a cut exactly through vertices (a cut at a tread's height) does not
/// produce degenerate pieces.
const NUDGE: f64 = 1.234e-7;

fn key2(p: [f64; 2]) -> (i64, i64) {
    ((p[0] * 1e6).round() as i64, (p[1] * 1e6).round() as i64)
}

fn key3(p: V3) -> Key3 {
    (
        (p[0] * 1e6).round() as i64,
        (p[1] * 1e6).round() as i64,
        (p[2] * 1e6).round() as i64,
    )
}

/// The area where a horizontal plane at height `h` cuts a solid, as polygons in plan.
pub fn section(s: &Solid, h: f64) -> MultiPolygon<f64> {
    let h = h + NUDGE;
    let mut segs: Vec<[[f64; 2]; 2]> = Vec::new();
    for f in &s.faces {
        let p = [s.points[f[0]], s.points[f[1]], s.points[f[2]]];
        let d = [p[0][2] - h, p[1][2] - h, p[2][2] - h];
        let mut cut = Vec::new();
        for i in 0..3 {
            let (a, b) = (i, (i + 1) % 3);
            if (d[a] < 0.0) != (d[b] < 0.0) {
                let t = d[a] / (d[a] - d[b]);
                cut.push([
                    p[a][0] + t * (p[b][0] - p[a][0]),
                    p[a][1] + t * (p[b][1] - p[a][1]),
                ]);
            }
        }
        if cut.len() == 2 && key2(cut[0]) != key2(cut[1]) {
            segs.push([cut[0], cut[1]]);
        }
    }
    // Chain the segments into closed loops.
    let mut at: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (i, sg) in segs.iter().enumerate() {
        at.entry(key2(sg[0])).or_default().push(i);
        at.entry(key2(sg[1])).or_default().push(i);
    }
    let mut used = vec![false; segs.len()];
    let mut out = MultiPolygon::<f64>(vec![]);
    for start in 0..segs.len() {
        if used[start] {
            continue;
        }
        used[start] = true;
        let mut ring = vec![segs[start][0], segs[start][1]];
        loop {
            let end = key2(*ring.last().expect("two points"));
            if end == key2(ring[0]) {
                break;
            }
            let next = at
                .get(&end)
                .and_then(|l| l.iter().copied().find(|&j| !used[j]));
            let Some(j) = next else { break };
            used[j] = true;
            let sg = segs[j];
            ring.push(if key2(sg[0]) == end { sg[1] } else { sg[0] });
        }
        if ring.len() < 4 {
            continue;
        }
        let poly = Polygon::new(
            LineString::from(
                ring.iter()
                    .map(|p| Coord { x: p[0], y: p[1] })
                    .collect::<Vec<_>>(),
            ),
            vec![],
        );
        // Even-odd: a hole loop inside an outer loop removes its area.
        out = out.xor(&MultiPolygon(vec![poly]));
    }
    out
}

fn normal(a: V3, b: V3, c: V3) -> V3 {
    let (u, v) = (
        [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
        [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
    );
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if l < 1e-15 {
        [0.0; 3]
    } else {
        [n[0] / l, n[1] / l, n[2] / l]
    }
}

/// The edges of a solid where its faces meet at more than `angle` degrees (the edges a
/// drawing shows), plus any open edges.
pub fn feature_edges(s: &Solid, angle: f64) -> Vec<[V3; 2]> {
    let cos_max = angle.to_radians().cos();
    let mut edges: HashMap<(Key3, Key3), EdgeFaces> = HashMap::new();
    for f in &s.faces {
        let p = [s.points[f[0]], s.points[f[1]], s.points[f[2]]];
        let n = normal(p[0], p[1], p[2]);
        if n == [0.0; 3] {
            continue;
        }
        for i in 0..3 {
            let (a, b) = (p[i], p[(i + 1) % 3]);
            let (ka, kb) = (key3(a), key3(b));
            if ka == kb {
                continue;
            }
            let k = if ka < kb { (ka, kb) } else { (kb, ka) };
            edges.entry(k).or_insert((a, b, vec![])).2.push(n);
        }
    }
    let mut out: Vec<[V3; 2]> = edges
        .into_values()
        .filter(|(_, _, ns)| {
            ns.len() != 2
                || ns[0][0] * ns[1][0] + ns[0][1] * ns[1][1] + ns[0][2] * ns[1][2] < cos_max
        })
        .map(|(a, b, _)| [a, b])
        .collect();
    out.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// A height map of the geometry below a cut, seen from above.
struct DepthMap {
    min: [f64; 2],
    px: f64,
    w: usize,
    h: usize,
    z: Vec<f64>,
}

impl DepthMap {
    fn new(tris: &[[V3; 3]]) -> Option<Self> {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for t in tris {
            for p in t {
                lo = [lo[0].min(p[0]), lo[1].min(p[1])];
                hi = [hi[0].max(p[0]), hi[1].max(p[1])];
            }
        }
        if lo[0] > hi[0] {
            return None;
        }
        let px = ((hi[0] - lo[0]).max(hi[1] - lo[1]) / 1500.0).max(1e-4);
        let (w, h) = (
            ((hi[0] - lo[0]) / px).ceil() as usize + 3,
            ((hi[1] - lo[1]) / px).ceil() as usize + 3,
        );
        let mut m = DepthMap {
            min: [lo[0] - px, lo[1] - px],
            px,
            w,
            h,
            z: vec![f64::NEG_INFINITY; w * h],
        };
        for t in tris {
            m.fill(t);
        }
        Some(m)
    }

    fn cell(&self, x: f64, y: f64) -> (isize, isize) {
        (
            ((x - self.min[0]) / self.px).floor() as isize,
            ((y - self.min[1]) / self.px).floor() as isize,
        )
    }

    fn fill(&mut self, t: &[V3; 3]) {
        let (x0, y0) = self.cell(
            t[0][0].min(t[1][0]).min(t[2][0]),
            t[0][1].min(t[1][1]).min(t[2][1]),
        );
        let (x1, y1) = self.cell(
            t[0][0].max(t[1][0]).max(t[2][0]),
            t[0][1].max(t[1][1]).max(t[2][1]),
        );
        let den =
            (t[1][1] - t[2][1]) * (t[0][0] - t[2][0]) + (t[2][0] - t[1][0]) * (t[0][1] - t[2][1]);
        if den.abs() < 1e-18 {
            return; // a vertical face: its top edge is drawn as an edge
        }
        for j in y0.max(0)..=y1.min(self.h as isize - 1) {
            for i in x0.max(0)..=x1.min(self.w as isize - 1) {
                let (x, y) = (
                    self.min[0] + (i as f64 + 0.5) * self.px,
                    self.min[1] + (j as f64 + 0.5) * self.px,
                );
                let a = ((t[1][1] - t[2][1]) * (x - t[2][0]) + (t[2][0] - t[1][0]) * (y - t[2][1]))
                    / den;
                let b = ((t[2][1] - t[0][1]) * (x - t[2][0]) + (t[0][0] - t[2][0]) * (y - t[2][1]))
                    / den;
                let c = 1.0 - a - b;
                if a < -1e-9 || b < -1e-9 || c < -1e-9 {
                    continue;
                }
                let z = a * t[0][2] + b * t[1][2] + c * t[2][2];
                let k = j as usize * self.w + i as usize;
                if z > self.z[k] {
                    self.z[k] = z;
                }
            }
        }
    }

    /// The lowest height in the 3×3 cells round a point (so an edge on the rim of a face counts
    /// as seen).
    fn low(&self, x: f64, y: f64) -> f64 {
        let (ci, cj) = self.cell(x, y);
        let mut z = f64::INFINITY;
        for j in cj - 1..=cj + 1 {
            for i in ci - 1..=ci + 1 {
                if i < 0 || j < 0 || i >= self.w as isize || j >= self.h as isize {
                    return f64::NEG_INFINITY;
                }
                z = z.min(self.z[j as usize * self.w + i as usize]);
            }
        }
        z
    }
}

/// The part of a triangle at or below height `h` (none, one or two triangles).
fn clip_tri(t: [V3; 3], h: f64) -> Vec<[V3; 3]> {
    let inside: Vec<bool> = t.iter().map(|p| p[2] <= h).collect();
    let n = inside.iter().filter(|x| **x).count();
    if n == 3 {
        return vec![t];
    }
    if n == 0 {
        return vec![];
    }
    let mut poly = Vec::new();
    for i in 0..3 {
        let (a, b) = (t[i], t[(i + 1) % 3]);
        if inside[i] {
            poly.push(a);
        }
        if inside[i] != inside[(i + 1) % 3] {
            let s = (h - a[2]) / (b[2] - a[2]);
            poly.push([a[0] + s * (b[0] - a[0]), a[1] + s * (b[1] - a[1]), h]);
        }
    }
    (1..poly.len() - 1)
        .map(|i| [poly[0], poly[i], poly[i + 1]])
        .collect()
}

/// The edge pieces at or below height `h` that are seen from above, as 2D lines in plan.
pub fn visible_edges_below(solids: &[&Solid], h: f64) -> Vec<[[f64; 2]; 2]> {
    let h = h + NUDGE;
    let mut tris = Vec::new();
    for s in solids {
        for f in &s.faces {
            tris.extend(clip_tri(
                [s.points[f[0]], s.points[f[1]], s.points[f[2]]],
                h,
            ));
        }
    }
    let Some(depth) = DepthMap::new(&tris) else {
        return vec![];
    };
    let tol = 1e-3 + 2.0 * depth.px;
    let mut out = Vec::new();
    for s in solids {
        for [mut a, mut b] in feature_edges(s, 20.0) {
            // Clip to the part below the cut.
            if a[2] > h && b[2] > h {
                continue;
            }
            if a[2] > h || b[2] > h {
                if a[2] > h {
                    std::mem::swap(&mut a, &mut b);
                }
                let s = (h - a[2]) / (b[2] - a[2]);
                b = [a[0] + s * (b[0] - a[0]), a[1] + s * (b[1] - a[1]), h];
            }
            let len = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
            if len < 1e-6 {
                continue; // vertical: a point in plan
            }
            let n = ((len / (depth.px * 0.5)).ceil() as usize).max(1);
            let at = |k: usize| {
                let t = k as f64 / n as f64;
                [
                    a[0] + t * (b[0] - a[0]),
                    a[1] + t * (b[1] - a[1]),
                    a[2] + t * (b[2] - a[2]),
                ]
            };
            let mut run: Option<[f64; 2]> = None;
            let mut last = [a[0], a[1]];
            for k in 0..=n {
                let p = at(k);
                let seen = p[2] + tol >= depth.low(p[0], p[1]);
                match (seen, run) {
                    (true, None) => run = Some([p[0], p[1]]),
                    (false, Some(r)) => {
                        out.push([r, last]);
                        run = None;
                    }
                    _ => {}
                }
                last = [p[0], p[1]];
            }
            if let Some(r) = run {
                out.push([r, last]);
            }
        }
    }
    out.retain(|[p, q]| (p[0] - q[0]).abs() + (p[1] - q[1]).abs() > 1e-6);
    out
}

/// The view's cut height (`view.cut_height`, 1.2 m by default).
pub fn cut_height(ev: &Evaluation) -> f64 {
    match ev.eval("view.cut_height") {
        Ok(Value::Number(x)) => x,
        _ => 1.2,
    }
}

/// The solids a derived representation is made from, with their material keys: every solid
/// of the source representation except voids, excluded parts, and parts whose category the
/// level-of-detail table hides or draws as a symbol in `view` (proposal 0005). The level is
/// the source representation's.
pub fn source_solids(
    def: &TypeDefinition,
    ev: &Evaluation,
    d: &Derive,
    view: Option<DrawingView>,
    table: &LodTable,
) -> Result<Vec<(Solid, Option<String>)>, Error> {
    let from = def
        .representation(&d.from)
        .ok_or_else(|| Error::Op(format!("derived from a missing representation {}", d.from)))?;
    let mut out = Vec::new();
    for item in from.items.iter().flatten() {
        let hidden = view.is_some_and(|v| {
            table.resolve(item.category, v, from.lod, d.lod_exceptions.as_ref()) != Visibility::Show
        });
        if hidden
            || item.role.as_deref() == Some("void")
            || d.exclude_parts
                .iter()
                .flatten()
                .any(|p| item.part.as_deref() == Some(p.as_str()))
        {
            continue;
        }
        for g in ev.items(&item.geometry)? {
            let Some(s) = g.solid() else { continue };
            let key = match resolve_material(item, &ev.env, &g)? {
                Some(Value::String(k)) if !k.is_empty() => Some(k),
                _ => None,
            };
            out.push((s.clone(), key));
        }
    }
    Ok(out)
}

/// Whether an overlay item of a derived representation is drawn: an item with a category is
/// the symbol for that category, drawn only where the table says `symbol` (proposal 0005);
/// an item without one is always drawn.
pub fn overlay_shown(
    item: &RepresentationItem,
    d: &Derive,
    view: Option<DrawingView>,
    lod: Lod,
    table: &LodTable,
) -> bool {
    match (item.category, view) {
        (Some(_), Some(v)) => {
            table.resolve(item.category, v, lod, d.lod_exceptions.as_ref()) == Visibility::Symbol
        }
        _ => true,
    }
}

/// A representation's items and the overlay items it draws (see [`overlay_shown`]).
pub fn drawn_items<'a>(
    rep: &'a Representation,
    table: &'a LodTable,
) -> impl Iterator<Item = &'a RepresentationItem> + 'a {
    let view = DrawingView::of(rep.context);
    rep.items
        .iter()
        .flatten()
        .chain(
            rep.overlay
                .iter()
                .flatten()
                .filter(move |i| match &rep.derive {
                    Some(d) => overlay_shown(i, d, view, rep.lod, table),
                    None => true,
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::Area;

    fn cube(x0: f64, z0: f64, size: f64) -> Solid {
        let p = |x: f64, y: f64, z: f64| [x0 + x * size, x0 + y * size, z0 + z * size];
        let points = vec![
            p(0.0, 0.0, 0.0),
            p(1.0, 0.0, 0.0),
            p(1.0, 1.0, 0.0),
            p(0.0, 1.0, 0.0),
            p(0.0, 0.0, 1.0),
            p(1.0, 0.0, 1.0),
            p(1.0, 1.0, 1.0),
            p(0.0, 1.0, 1.0),
        ];
        let faces = vec![
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [1, 2, 6],
            [1, 6, 5],
            [2, 3, 7],
            [2, 7, 6],
            [3, 0, 4],
            [3, 4, 7],
        ];
        Solid::new(points, faces)
    }

    #[test]
    fn a_cut_through_a_cube_is_its_square() {
        let c = cube(0.0, 0.0, 2.0);
        assert!((section(&c, 1.0).unsigned_area() - 4.0).abs() < 1e-9);
        assert_eq!(section(&c, 3.0).0.len(), 0, "above the cube");
        assert_eq!(
            feature_edges(&c, 20.0).len(),
            12,
            "the diagonals are not drawn"
        );
    }

    #[test]
    fn hidden_edges_are_left_out() {
        // A small cube under a big one's overhang is hidden; the big one's outline is seen.
        let big = cube(0.0, 1.0, 2.0);
        let small = cube(0.5, 0.0, 0.5);
        let lines = visible_edges_below(&[&big, &small], 10.0);
        let total: f64 = lines
            .iter()
            .map(|[p, q]| ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt())
            .sum();
        assert!(
            (total - 16.0).abs() < 0.05,
            "two squares of 2 m seen from above: {total}"
        );
        let no_cut = visible_edges_below(&[&small], 10.0);
        assert!(!no_cut.is_empty(), "alone, the small cube is seen");
    }
}
