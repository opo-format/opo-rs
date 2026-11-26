//! Wall joins (decision 0026): the application says which walls are in the project; this
//! module finds where their paths meet and trims their plan shapes so they join cleanly. A
//! wall is one solid (decision 0028), so a join only shares out the overlap:
//!
//! - **L** (two path ends meet): both walls are extended past the corner and split along the
//!   line through the corner that halves the angle between them (a mitre);
//! - **T** (a path end meets another wall's path away from its ends): the meeting wall runs
//!   into the other and stops at its far face; the other runs through;
//! - **any other overlap** (walls crossing): the wall listed first runs through.
//!
//! Each wall's layers are the bands of its build-up along its extended path, kept inside the
//! wall's share, so hatching follows each wall to the join.

use geo::{BooleanOps, LineString, MultiPolygon, Polygon};
use opo_feel::Value;

use crate::Error;
use crate::run::Evaluation;

/// What the join module needs of one wall, read from a wall's evaluation (the wall class
/// template's names: `path`, `left_face`, `total_thickness`, `layers`, `layer_start`).
#[derive(Debug, Clone)]
pub struct WallPlan {
    /// The wall path in plan.
    pub path: Vec<[f64; 2]>,
    /// Offset of the left (front) face from the path, positive to the left.
    pub left: f64,
    /// Total thickness.
    pub thickness: f64,
    /// Layers front to back: offset of the layer's front face from the wall's front face,
    /// thickness, material key and function.
    pub layers: Vec<(f64, f64, Option<String>, Option<String>)>,
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(x) => Some(*x),
        _ => None,
    }
}

impl WallPlan {
    /// Read a wall from its evaluation.
    pub fn from_evaluation(ev: &Evaluation) -> Result<WallPlan, Error> {
        let bad = |what: &str| Error::Op(format!("wall join: the wall has no usable {what}"));
        let path = match ev.eval("path")? {
            Value::List(ps) => ps
                .iter()
                .map(|p| match p {
                    Value::List(c) if c.len() >= 2 => {
                        Some([num(&c[0]).unwrap_or(0.0), num(&c[1]).unwrap_or(0.0)])
                    }
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| bad("path"))?,
            _ => return Err(bad("path")),
        };
        let left = num(&ev.eval("left_face")?).ok_or_else(|| bad("left_face"))?;
        let thickness = num(&ev.eval("total_thickness")?).ok_or_else(|| bad("thickness"))?;
        let starts = match ev.eval("layer_start")? {
            Value::List(v) => v.iter().map(num).collect::<Option<Vec<_>>>(),
            _ => None,
        }
        .ok_or_else(|| bad("layer_start"))?;
        let layers = match ev.eval("layers")? {
            Value::List(ls) => ls,
            _ => return Err(bad("layers")),
        };
        let field = |l: &Value, k: &str| match l {
            Value::Context(m) => m.get(k).cloned(),
            _ => None,
        };
        let layers = layers
            .iter()
            .zip(starts)
            .map(|(l, s)| {
                let t = field(l, "thickness").as_ref().and_then(num).unwrap_or(0.0);
                let text = |k| match field(l, k) {
                    Some(Value::String(x)) => Some(x),
                    _ => None,
                };
                (s, t, text("material"), text("function"))
            })
            .collect();
        Ok(WallPlan {
            path,
            left,
            thickness,
            layers,
        })
    }
}

/// One wall after joining: its share of the plan, and its layer bands inside it.
#[derive(Debug, Clone)]
pub struct JoinedWall {
    /// The wall's plan shape after the joins.
    pub shape: MultiPolygon<f64>,
    /// Each layer's band inside the shape, with material key and function.
    pub layers: Vec<(MultiPolygon<f64>, Option<String>, Option<String>)>,
}

fn sub(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}

fn unit(v: [f64; 2]) -> [f64; 2] {
    let l = (v[0] * v[0] + v[1] * v[1]).sqrt();
    if l < 1e-12 {
        [0.0, 0.0]
    } else {
        [v[0] / l, v[1] / l]
    }
}

/// Distance from a point to a polyline, and the parameter of the nearest point (segment index
/// plus position along it).
fn nearest(p: [f64; 2], path: &[[f64; 2]]) -> (f64, f64) {
    let mut best = (f64::INFINITY, 0.0);
    for (i, w) in path.windows(2).enumerate() {
        let d = sub(w[1], w[0]);
        let l2 = d[0] * d[0] + d[1] * d[1];
        let t = if l2 < 1e-18 {
            0.0
        } else {
            (((p[0] - w[0][0]) * d[0] + (p[1] - w[0][1]) * d[1]) / l2).clamp(0.0, 1.0)
        };
        let q = [w[0][0] + t * d[0], w[0][1] + t * d[1]];
        let dist = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
        if dist < best.0 {
            best = (dist, i as f64 + t);
        }
    }
    best
}

/// A half-plane as a large polygon (half-size `big`; kept near the walls' size, since the
/// polygon library snaps coordinates to a grid scaled to the shapes): the side of the line
/// through `c` with direction `d` that contains `inside`.
fn half_plane(
    c: [f64; 2],
    d: [f64; 2],
    inside: [f64; 2],
    big: f64,
    grow: f64,
) -> MultiPolygon<f64> {
    let d = unit(d);
    let mut n = [-d[1], d[0]];
    if (inside[0] - c[0]) * n[0] + (inside[1] - c[1]) * n[1] < 0.0 {
        n = [-n[0], -n[1]];
    }
    let c = [c[0] - n[0] * grow, c[1] - n[1] * grow];
    let a = [c[0] - d[0] * big, c[1] - d[1] * big];
    let b = [c[0] + d[0] * big, c[1] + d[1] * big];
    let pts = [
        a,
        b,
        [b[0] + n[0] * big, b[1] + n[1] * big],
        [a[0] + n[0] * big, a[1] + n[1] * big],
        a,
    ];
    MultiPolygon(vec![Polygon::new(
        LineString::from(pts.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>()),
        vec![],
    )])
}

/// Tolerance for a path end to meet another wall's path (1 mm).
pub const MEET: f64 = 1e-3;

/// Join walls in plan (see the module documentation).
pub fn join(walls: &[WallPlan]) -> Vec<JoinedWall> {
    let n = walls.len();
    // Extend each wall's path at ends that meet another wall, so the walls overlap there.
    let mut paths: Vec<Vec<[f64; 2]>> = walls.iter().map(|w| w.path.clone()).collect();
    // (wall, other, corner, direction into the wall from the corner) for L joins.
    let mut corners: Vec<(usize, usize, [f64; 2], [f64; 2])> = Vec::new();
    // (stem wall, through wall) for T joins.
    let mut tees: Vec<(usize, usize)> = Vec::new();
    for a in 0..n {
        let pa = &walls[a].path;
        if pa.len() < 2 {
            continue;
        }
        for (end, inner) in [(0usize, 1usize), (pa.len() - 1, pa.len() - 2)] {
            let p = pa[end];
            for b in 0..n {
                if a == b || walls[b].path.len() < 2 {
                    continue;
                }
                let pb = &walls[b].path;
                let (dist, _) = nearest(p, pb);
                if dist > MEET {
                    continue;
                }
                let into = unit(sub(pa[inner], p));
                let ext = walls[b].thickness + walls[a].thickness;
                let q = [p[0] - into[0] * ext, p[1] - into[1] * ext];
                paths[a][end] = q;
                let at_end = [pb[0], pb[pb.len() - 1]]
                    .iter()
                    .any(|e| ((e[0] - p[0]).powi(2) + (e[1] - p[1]).powi(2)).sqrt() <= MEET);
                if at_end {
                    corners.push((a, b, p, into));
                } else {
                    tees.push((a, b));
                }
            }
        }
    }
    let big = paths
        .iter()
        .flatten()
        .fold(0.0f64, |m, p| m.max(p[0].abs()).max(p[1].abs()))
        * 2.0
        + 10.0;
    let band = |i: usize, l: f64, r: f64| crate::ops::band_region(&paths[i], l, r);
    let mut shapes: Vec<MultiPolygon<f64>> = (0..n)
        .map(|i| band(i, walls[i].left, walls[i].left - walls[i].thickness))
        .collect();
    // Layer shares overlap a hair (0.01 mm) across mitres, so layers of the same material
    // merge across a corner without a seam.
    let mut layer_shares = shapes.clone();
    // L joins: keep the wall's side of the mitre line.
    for &(a, b, c, into) in &corners {
        let other = corners
            .iter()
            .find(|(x, y, _, _)| *x == b && *y == a)
            .map(|t| t.3)
            .unwrap_or(into);
        let bis = unit([into[0] + other[0], into[1] + other[1]]);
        let line = if bis == [0.0, 0.0] {
            [-into[1], into[0]]
        } else {
            bis
        };
        let inside = [c[0] + into[0], c[1] + into[1]];
        shapes[a] = shapes[a].intersection(&half_plane(c, line, inside, big, 0.0));
        layer_shares[a] = layer_shares[a].intersection(&half_plane(c, line, inside, big, 1e-5));
    }
    // T joins: the stem stops at the through wall's far face.
    let originals = shapes.clone();
    for &(a, b) in &tees {
        shapes[a] = shapes[a].difference(&originals[b]);
        // The stem's layers reach a hair into the through wall, so the outline closes over
        // the join.
        let core = band(
            b,
            walls[b].left - 1e-5,
            walls[b].left - walls[b].thickness + 1e-5,
        );
        layer_shares[a] = layer_shares[a].difference(&core);
        // Drop pieces of the stem left beyond the through wall.
        let pa = &walls[a].path;
        let keep: Vec<Polygon<f64>> = shapes[a]
            .0
            .iter()
            .filter(|poly| {
                use geo::Intersects;
                pa.windows(2).any(|w| {
                    let mid = geo::Point::new((w[0][0] + w[1][0]) / 2.0, (w[0][1] + w[1][1]) / 2.0);
                    poly.intersects(&mid)
                })
            })
            .cloned()
            .collect();
        if !keep.is_empty() {
            shapes[a] = MultiPolygon(keep);
        }
        // Only the layer pieces that touch the stem's share are kept.
        let share = shapes[a].clone();
        layer_shares[a] = MultiPolygon(
            layer_shares[a]
                .0
                .iter()
                .filter(|poly| {
                    use geo::Intersects;
                    share.intersects(*poly)
                })
                .cloned()
                .collect(),
        );
    }
    // Any other overlap: the wall listed first runs through.
    for a in 0..n {
        for b in 0..a {
            if tees.contains(&(a, b))
                || tees.contains(&(b, a))
                || corners.iter().any(|c| c.0 == a && c.1 == b)
            {
                continue;
            }
            shapes[a] = shapes[a].difference(&shapes[b]);
            layer_shares[a] = layer_shares[a].difference(&shapes[b]);
        }
    }
    (0..n)
        .map(|i| {
            let w = &walls[i];
            let layers = w
                .layers
                .iter()
                .map(|(s, t, m, f)| {
                    let l = w.left - s;
                    (
                        band(i, l, l - t).intersection(&layer_shares[i]),
                        m.clone(),
                        f.clone(),
                    )
                })
                .collect();
            JoinedWall {
                shape: shapes[i].clone(),
                layers,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use geo::Area;

    fn wall(path: Vec<[f64; 2]>, t: f64) -> WallPlan {
        WallPlan {
            path,
            left: t / 2.0,
            thickness: t,
            layers: vec![(0.0, t, Some("m".into()), Some("structure".into()))],
        }
    }

    #[test]
    fn an_l_corner_is_mitred_without_overlap_or_gap() {
        let w = [
            wall(vec![[0.0, 0.0], [4.0, 0.0]], 0.2),
            wall(vec![[0.0, 0.0], [0.0, 3.0]], 0.2),
        ];
        let j = join(&w);
        let total: f64 = j.iter().map(|x| x.shape.unsigned_area()).sum();
        // Outer 4.1 × 3.1 L minus the inner 3.9 × 2.9 rectangle.
        let expected = 4.1 * 3.1 - 3.9 * 2.9;
        assert!((total - expected).abs() < 1e-6, "{total} vs {expected}");
        let overlap = j[0].shape.intersection(&j[1].shape).unsigned_area();
        assert!(overlap < 1e-9);
    }

    #[test]
    fn a_tee_stops_at_the_far_face() {
        let w = [
            wall(vec![[0.0, 0.0], [4.0, 0.0]], 0.3),
            wall(vec![[2.0, 0.0], [2.0, 3.0]], 0.1),
        ];
        let j = join(&w);
        assert!(
            (j[0].shape.unsigned_area() - 4.0 * 0.3).abs() < 1e-6,
            "through wall unchanged"
        );
        assert!(
            (j[1].shape.unsigned_area() - 0.1 * 2.85).abs() < 1e-6,
            "stem from the face"
        );
    }

    #[test]
    fn crossing_walls_let_the_first_run_through() {
        let w = [
            wall(vec![[-2.0, 0.0], [2.0, 0.0]], 0.2),
            wall(vec![[0.0, -2.0], [0.0, 2.0]], 0.2),
        ];
        let j = join(&w);
        assert!((j[0].shape.unsigned_area() - 0.8).abs() < 1e-6);
        assert!((j[1].shape.unsigned_area() - (0.8 - 0.04)).abs() < 1e-6);
    }
}
