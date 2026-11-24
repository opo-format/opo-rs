//! The recipe operations of spec §4.5 (all but `for_each` and `instance`, which the runner
//! handles because their arguments are evaluated lazily).

use geo::{BooleanOps, Coord, LineString, MultiPolygon, Polygon, unary_union};
use opo_feel::Value;
use opo_model::Op;

use crate::Error;
use crate::geometry::{Geometry, Plane, Shape, Solid, flatten, list_value};
use crate::mesh::{polygon, prism_faces, triangulate};

type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn norm(a: V3) -> f64 {
    dot(a, a).sqrt()
}
fn unit(a: V3) -> V3 {
    scale(a, 1.0 / norm(a))
}

// ------------------------------------------------------------------ argument conversion

fn num(op: Op, name: &str, v: &Value) -> Result<f64, Error> {
    v.as_number().ok_or_else(|| {
        Error::Op(format!(
            "{}: '{name}' must be a number, got {v:?}",
            op.name()
        ))
    })
}

fn coords(op: Op, name: &str, v: &Value, dims: usize) -> Result<Vec<f64>, Error> {
    let list = v.as_list().ok_or_else(|| {
        Error::Op(format!(
            "{}: '{name}' must be a point, got {v:?}",
            op.name()
        ))
    })?;
    if list.len() < dims {
        return Err(Error::Op(format!(
            "{}: '{name}' needs {dims} coordinates",
            op.name()
        )));
    }
    list.iter().map(|x| num(op, name, x)).collect()
}

fn p2(op: Op, name: &str, v: &Value) -> Result<[f64; 2], Error> {
    let c = coords(op, name, v, 2)?;
    Ok([c[0], c[1]])
}

fn p3(op: Op, name: &str, v: &Value) -> Result<V3, Error> {
    let c = coords(op, name, v, 3)?;
    Ok([c[0], c[1], c[2]])
}

fn points2(op: Op, name: &str, v: &Value) -> Result<Vec<[f64; 2]>, Error> {
    if let Ok(g) = flatten(v) {
        if let [
            Geometry {
                shape: Shape::Curve { points, .. },
                ..
            },
        ] = g.as_slice()
        {
            return Ok(points.clone());
        }
    }
    let list = v
        .as_list()
        .ok_or_else(|| Error::Op(format!("{}: '{name}' must be a list of points", op.name())))?;
    list.iter().map(|p| p2(op, name, p)).collect()
}

fn points3(op: Op, name: &str, v: &Value) -> Result<Vec<V3>, Error> {
    if let Ok(g) = flatten(v) {
        if let [
            Geometry {
                shape: Shape::Curve3 { points },
                ..
            },
        ] = g.as_slice()
        {
            return Ok(points.clone());
        }
    }
    let list = v.as_list().ok_or_else(|| {
        Error::Op(format!(
            "{}: '{name}' must be a list of 3D points",
            op.name()
        ))
    })?;
    list.iter().map(|p| p3(op, name, p)).collect()
}

fn opt_num(op: Op, name: &str, v: Option<&Value>, default: f64) -> Result<f64, Error> {
    match v {
        None | Some(Value::Null) => Ok(default),
        Some(x) => num(op, name, x),
    }
}

fn regions(v: &Value) -> Result<Vec<(Plane, MultiPolygon<f64>)>, Error> {
    flatten(v)?
        .into_iter()
        .map(|g| match g.shape {
            Shape::Region { plane, shape } => Ok((plane, shape)),
            other => Err(Error::Op(format!(
                "expected a region, got {}",
                kind(&other)
            ))),
        })
        .collect()
}

fn kind(s: &Shape) -> &'static str {
    match s {
        Shape::Region { .. } => "region",
        Shape::Curve { .. } => "curve",
        Shape::Curve3 { .. } => "3D curve",
        Shape::Solid(_) => "solid",
        Shape::Text { .. } => "text",
    }
}

fn region(plane: Plane, shape: MultiPolygon<f64>) -> Value {
    Geometry::new(Shape::Region { plane, shape }).into_value()
}

fn curve(plane: Plane, points: Vec<[f64; 2]>) -> Value {
    Geometry::new(Shape::Curve { plane, points }).into_value()
}

/// Union of polygons; also repairs self-intersections (as a zero buffer does). A single
/// valid polygon is returned as it is: the polygon library snaps coordinates to a grid, so
/// skipping it keeps simple shapes exact.
fn normalise(polys: &[Polygon<f64>]) -> MultiPolygon<f64> {
    use geo::Validation;
    match polys {
        [p] if p.is_valid() => MultiPolygon(vec![p.clone()]),
        _ => unary_union(polys.iter()),
    }
}

/// Union of several shapes (see [`normalise`]).
fn union_all<'a>(shapes: impl IntoIterator<Item = &'a MultiPolygon<f64>>) -> MultiPolygon<f64> {
    let polys: Vec<Polygon<f64>> = shapes
        .into_iter()
        .flat_map(|m| m.0.iter().cloned())
        .collect();
    normalise(&polys)
}

fn display(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_json().to_string(),
    }
}

// ------------------------------------------------------------------ dispatch

/// Apply an operation to positional arguments (in the order of [`Op::arguments`]); a missing
/// optional argument is `null`.
pub(crate) fn apply(op: Op, plane: Plane, a: &[Value]) -> Result<Value, Error> {
    let arg = |i: usize| a.get(i).unwrap_or(&Value::Null);
    Ok(match op {
        Op::Rectangle => {
            let ([x0, y0], [x1, y1]) = (p2(op, "min", arg(0))?, p2(op, "max", arg(1))?);
            region(
                plane,
                MultiPolygon(vec![polygon(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]])]),
            )
        }
        Op::Polygon => region(
            plane,
            normalise(&[polygon(&points2(op, "points", arg(0))?)]),
        ),
        Op::Circle => {
            let (c, r) = (p2(op, "center", arg(0))?, num(op, "radius", arg(1))?);
            let segments = opt_num(op, "segments", a.get(2), 48.0)? as i64;
            // A 4q-gon with q = max(4, segments ÷ 4), as a buffered point.
            let n = 4 * (segments / 4).max(4);
            let pts: Vec<[f64; 2]> = (0..n)
                .map(|k| {
                    let t = std::f64::consts::TAU * k as f64 / n as f64;
                    [c[0] + r * t.cos(), c[1] + r * t.sin()]
                })
                .collect();
            region(plane, MultiPolygon(vec![polygon(&pts)]))
        }
        Op::Polyline => curve(plane, points2(op, "points", arg(0))?),
        Op::Line => curve(
            plane,
            vec![p2(op, "start", arg(0))?, p2(op, "end", arg(1))?],
        ),
        Op::Arc => {
            let (c, r) = (p2(op, "center", arg(0))?, num(op, "radius", arg(1))?);
            let (a0, a1) = (
                num(op, "start_angle", arg(2))?,
                num(op, "end_angle", arg(3))?,
            );
            let segments = opt_num(op, "segments", a.get(4), 32.0)?;
            let n = ((segments * (a1 - a0).abs() / 360.0).floor() as usize + 2).max(2);
            let pts = (0..n)
                .map(|k| {
                    let t = (a0 + (a1 - a0) * k as f64 / (n - 1) as f64).to_radians();
                    [c[0] + r * t.cos(), c[1] + r * t.sin()]
                })
                .collect();
            curve(plane, pts)
        }
        Op::Band => {
            let path = points2(op, "path", arg(0))?;
            let (l, r) = (num(op, "left", arg(1))?, num(op, "right", arg(2))?);
            let mut ring = offset(&path, l);
            ring.extend(offset(&path, r).into_iter().rev());
            region(plane, normalise(&[polygon(&ring)]))
        }
        Op::Difference => {
            let a_regions = regions(arg(0))?;
            let Some(&(p, _)) = a_regions.first() else {
                return Err(Error::Op("difference: 'a' is empty".into()));
            };
            let sa = union_all(a_regions.iter().map(|(_, s)| s));
            let b_regions = regions(arg(1))?;
            let shape = if b_regions.is_empty() {
                sa
            } else {
                sa.difference(&union_all(b_regions.iter().map(|(_, s)| s)))
            };
            region(p, shape)
        }
        Op::Union => {
            let items = flatten(arg(0))?;
            let same_plane = items
                .iter()
                .all(|g| matches!(g.shape, Shape::Region { .. }))
                && items.windows(2).all(|w| match (&w[0].shape, &w[1].shape) {
                    (Shape::Region { plane: a, .. }, Shape::Region { plane: b, .. }) => a == b,
                    _ => false,
                });
            if !items.is_empty() && same_plane {
                let rs = regions(arg(0))?;
                region(rs[0].0, union_all(rs.iter().map(|(_, s)| s)))
            } else {
                list_value(items)
            }
        }
        Op::BoundaryCurves => {
            let mut out = Vec::new();
            for (p, shape) in regions(arg(0))? {
                for poly in &shape {
                    for ring in std::iter::once(poly.exterior()).chain(poly.interiors()) {
                        out.push(curve(p, ring.coords().map(|c| [c.x, c.y]).collect()));
                    }
                }
            }
            Value::List(out)
        }
        Op::Text => {
            let at = p2(op, "at", arg(0))?;
            let height = opt_num(op, "height", a.get(2), 0.0025)?;
            Geometry::new(Shape::Text {
                plane,
                at,
                value: display(arg(1)),
                height,
            })
            .into_value()
        }
        Op::Extrude => extrude(arg(0), num(op, "from", arg(1))?, num(op, "to", arg(2))?)?,
        Op::Sweep => sweep(
            arg(0),
            &points3(op, "path", arg(1))?,
            opt_num(op, "roll", a.get(2), 0.0)?,
        )?,
        Op::Revolve => {
            let (c, k) = (p3(op, "center", arg(1))?, unit(p3(op, "axis", arg(2))?));
            let angle = num(op, "angle", arg(3))?;
            let segments = opt_num(op, "segments", a.get(4), 24.0)?;
            revolve(arg(0), c, k, angle, segments)?
        }
        Op::Box => {
            let (lo, hi) = (p3(op, "min", arg(0))?, p3(op, "max", arg(1))?);
            let rect = apply(
                Op::Rectangle,
                Plane::Xy,
                &[point(&lo[..2]), point(&hi[..2])],
            )?;
            extrude(&rect, lo[2], hi[2])?
        }
        Op::Cylinder => {
            let (s, e, r) = (
                p3(op, "start", arg(0))?,
                p3(op, "end", arg(1))?,
                num(op, "radius", arg(2))?,
            );
            let segments = opt_num(op, "segments", a.get(3), 24.0)?;
            let disc = apply(
                Op::Circle,
                Plane::Yz,
                &[
                    point(&[0.0, 0.0]),
                    Value::Number(r),
                    Value::Number(segments),
                ],
            )?;
            sweep(&disc, &[s, e], 0.0)?
        }
        Op::Path3 => Geometry::new(Shape::Curve3 {
            points: points3(op, "points", arg(0))?,
        })
        .into_value(),
        Op::Move => {
            let by = coords(op, "by", arg(1), 2)?;
            let items = flatten(arg(0))?;
            if by.len() == 2 {
                list_value(transform_2d(items, |[u, v]| [u + by[0], v + by[1]])?)
            } else {
                let by = [by[0], by[1], by[2]];
                list_value(transform_3d(items, |p| add(p, by), false)?)
            }
        }
        Op::Rotate => {
            let c = coords(op, "center", arg(1), 2)?;
            let t = num(op, "angle", arg(2))?.to_radians();
            let (cs, sn) = (t.cos(), t.sin());
            let items = flatten(arg(0))?;
            if c.len() == 2 {
                list_value(transform_2d(items, |[u, v]| {
                    let (x, y) = (u - c[0], v - c[1]);
                    [c[0] + x * cs - y * sn, c[1] + x * sn + y * cs]
                })?)
            } else {
                list_value(transform_3d(
                    items,
                    |p| {
                        let (x, y) = (p[0] - c[0], p[1] - c[1]);
                        [c[0] + x * cs - y * sn, c[1] + x * sn + y * cs, p[2]]
                    },
                    false,
                )?)
            }
        }
        Op::Mirror => {
            let axis = match arg(1) {
                Value::String(s) if s == "x" || s == "y" => s.clone(),
                other => {
                    return Err(Error::Op(format!(
                        "mirror: axis must be \"x\" or \"y\", got {other:?}"
                    )));
                }
            };
            let items = flatten(arg(0))?;
            let flat2d = items.iter().any(|g| {
                matches!(
                    g.shape,
                    Shape::Region { .. } | Shape::Curve { .. } | Shape::Text { .. }
                )
            });
            let x = axis == "x";
            if flat2d {
                list_value(transform_2d(
                    items,
                    |[u, v]| if x { [-u, v] } else { [u, -v] },
                )?)
            } else {
                list_value(transform_3d(
                    items,
                    |p| {
                        if x {
                            [-p[0], p[1], p[2]]
                        } else {
                            [p[0], -p[1], p[2]]
                        }
                    },
                    true,
                )?)
            }
        }
        Op::ForEach | Op::Instance => {
            return Err(Error::Op(format!("{} is handled by the runner", op.name())));
        }
    })
}

fn point(c: &[f64]) -> Value {
    Value::List(c.iter().map(|x| Value::Number(*x)).collect())
}

// ------------------------------------------------------------------ 2D helpers

/// Offset an open polyline by `d` (positive = left of the direction of travel) with mitred
/// joins; a join longer than 10 × |d| is bevelled.
fn offset(path: &[[f64; 2]], d: f64) -> Vec<[f64; 2]> {
    if d.abs() < 1e-12 || path.len() < 2 {
        return path.to_vec();
    }
    let normal = |a: [f64; 2], b: [f64; 2]| {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l = (dx * dx + dy * dy).sqrt();
        [-dy / l, dx / l]
    };
    let segs: Vec<[f64; 2]> = path
        .windows(2)
        .filter(|w| w[0] != w[1])
        .map(|w| normal(w[0], w[1]))
        .collect();
    let pts: Vec<[f64; 2]> = {
        let mut p = vec![path[0]];
        for q in &path[1..] {
            if p.last() != Some(q) {
                p.push(*q);
            }
        }
        p
    };
    if segs.is_empty() {
        return path.to_vec();
    }
    let mut out = vec![[pts[0][0] + d * segs[0][0], pts[0][1] + d * segs[0][1]]];
    for j in 1..pts.len() - 1 {
        let (n0, n1) = (segs[j - 1], segs[j]);
        let c = n0[0] * n1[0] + n0[1] * n1[1];
        let p = pts[j];
        if c > 1.0 - 1e-12 {
            out.push([p[0] + d * n0[0], p[1] + d * n0[1]]);
        } else if (2.0 / (1.0 + c)).sqrt() > 10.0 {
            out.push([p[0] + d * n0[0], p[1] + d * n0[1]]);
            out.push([p[0] + d * n1[0], p[1] + d * n1[1]]);
        } else {
            let k = d / (1.0 + c);
            out.push([p[0] + k * (n0[0] + n1[0]), p[1] + k * (n0[1] + n1[1])]);
        }
    }
    let last = pts[pts.len() - 1];
    let n = segs[segs.len() - 1];
    out.push([last[0] + d * n[0], last[1] + d * n[1]]);
    out
}

fn map_ring(ls: &LineString<f64>, f: &impl Fn([f64; 2]) -> [f64; 2]) -> LineString<f64> {
    LineString::from(
        ls.coords()
            .map(|c| {
                let [x, y] = f([c.x, c.y]);
                Coord { x, y }
            })
            .collect::<Vec<_>>(),
    )
}

/// Apply a 2D map to regions, curves and text; solids and 3D curves pass unchanged.
fn transform_2d(
    items: Vec<Geometry>,
    f: impl Fn([f64; 2]) -> [f64; 2],
) -> Result<Vec<Geometry>, Error> {
    Ok(items
        .into_iter()
        .map(|mut g| {
            g.shape = match g.shape {
                Shape::Region { plane, shape } => Shape::Region {
                    plane,
                    shape: MultiPolygon(
                        shape
                            .0
                            .iter()
                            .map(|p| {
                                Polygon::new(
                                    map_ring(p.exterior(), &f),
                                    p.interiors().iter().map(|r| map_ring(r, &f)).collect(),
                                )
                            })
                            .collect(),
                    ),
                },
                Shape::Curve { plane, points } => Shape::Curve {
                    plane,
                    points: points.into_iter().map(&f).collect(),
                },
                Shape::Text {
                    plane,
                    at,
                    value,
                    height,
                } => Shape::Text {
                    plane,
                    at: f(at),
                    value,
                    height,
                },
                other => other,
            };
            g
        })
        .collect())
}

/// Apply a 3D map to solids and 3D curves; 2D geometry is an error. A mirror also flips the
/// faces so they stay outward.
fn transform_3d(
    items: Vec<Geometry>,
    f: impl Fn(V3) -> V3,
    mirror: bool,
) -> Result<Vec<Geometry>, Error> {
    items
        .into_iter()
        .map(|mut g| {
            g.shape = match g.shape {
                Shape::Solid(s) => {
                    let mut faces = s.faces;
                    if mirror {
                        faces.iter_mut().for_each(|t| t.swap(1, 2));
                    }
                    Shape::Solid(Solid {
                        points: s.points.into_iter().map(&f).collect(),
                        faces,
                    })
                }
                Shape::Curve3 { points } => Shape::Curve3 {
                    points: points.into_iter().map(&f).collect(),
                },
                other => return Err(Error::Op(format!("3D transform on a {}", kind(&other)))),
            };
            Ok(g)
        })
        .collect()
}

// ------------------------------------------------------------------ 3D helpers

fn extrude(profile: &Value, lo: f64, hi: f64) -> Result<Value, Error> {
    let mut out = Vec::new();
    for (plane, shape) in regions(profile)? {
        for poly in &shape {
            let t = triangulate(poly)?;
            let mut pts = Vec::with_capacity(2 * t.verts.len());
            for w in [lo, hi] {
                for [u, v] in &t.verts {
                    pts.push(plane.to_3d(*u, *v, w)?);
                }
            }
            out.push(Geometry::new(Shape::Solid(Solid::new(
                pts,
                prism_faces(&t),
            ))));
        }
    }
    Ok(list_value(out))
}

/// Direction, across and up vectors of a path segment (up is +z unless the segment is nearly
/// vertical, then +x).
fn frame(d: V3) -> (V3, V3, V3) {
    let d = unit(d);
    let up = if d[2].abs() < 0.99 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let across = unit(cross(up, d));
    (d, across, cross(d, across))
}

fn sweep(profile: &Value, path: &[V3], roll: f64) -> Result<Value, Error> {
    let mut out = Vec::new();
    let polys: Vec<Polygon<f64>> = regions(profile)?
        .into_iter()
        .flat_map(|(_, s)| s.0)
        .collect();
    let (cr, sr) = (roll.to_radians().cos(), roll.to_radians().sin());
    for w in path.windows(2) {
        let seg = sub(w[1], w[0]);
        let l = norm(seg);
        if l < 1e-9 {
            continue;
        }
        let (d, across, vert) = frame(seg);
        let (a2, v2) = (
            add(scale(across, cr), scale(vert, sr)),
            add(scale(across, -sr), scale(vert, cr)),
        );
        for poly in &polys {
            let t = triangulate(poly)?;
            let base: Vec<V3> = t
                .verts
                .iter()
                .map(|[u, v]| add(w[0], add(scale(a2, *u), scale(v2, *v))))
                .collect();
            let top: Vec<V3> = base.iter().map(|b| add(*b, scale(d, l))).collect();
            out.push(Geometry::new(Shape::Solid(Solid::new(
                [base, top].concat(),
                prism_faces(&t),
            ))));
        }
    }
    Ok(list_value(out))
}

fn revolve(profile: &Value, c: V3, k: V3, angle: f64, segments: f64) -> Result<Value, Error> {
    let rot = |p: V3, a: f64| {
        let a = a.to_radians();
        let p = sub(p, c);
        add(
            c,
            add(
                add(scale(p, a.cos()), scale(cross(k, p), a.sin())),
                scale(k, dot(k, p) * (1.0 - a.cos())),
            ),
        )
    };
    let steps = ((segments * angle.abs() / 360.0).floor() as usize + 1).max(2);
    let mut out = Vec::new();
    for (plane, shape) in regions(profile)? {
        for poly in &shape {
            let cap = triangulate(&polygon(&ring_ccw(poly)))?;
            let ring = &cap.verts;
            let m = ring.len();
            let mut pts = Vec::with_capacity((steps + 1) * m);
            for s in 0..=steps {
                let a = angle * s as f64 / steps as f64;
                for [u, v] in ring {
                    pts.push(rot(plane.to_3d(*u, *v, 0.0)?, a));
                }
            }
            let mut faces = Vec::new();
            for s in 0..steps {
                for i in 0..m {
                    let (a0, b0) = (s * m + i, s * m + (i + 1) % m);
                    faces.push([a0, b0, b0 + m]);
                    faces.push([a0, b0 + m, a0 + m]);
                }
            }
            let last = steps * m;
            faces.extend(cap.tris.iter().map(|&[a, b, c]| [a, c, b]));
            faces.extend(
                cap.tris
                    .iter()
                    .map(|&[a, b, c]| [a + last, b + last, c + last]),
            );
            out.push(Geometry::new(Shape::Solid(Solid::new(pts, faces))));
        }
    }
    Ok(list_value(out))
}

/// The exterior ring of a polygon, counter-clockwise, without the closing point.
fn ring_ccw(poly: &Polygon<f64>) -> Vec<[f64; 2]> {
    use geo::orient::{Direction, Orient};
    let p = poly.orient(Direction::Default);
    let mut pts: Vec<[f64; 2]> = p.exterior().coords().map(|c| [c.x, c.y]).collect();
    pts.pop();
    pts
}

#[cfg(test)]
mod tests {
    use super::*;
    use opo_feel::Value as V;

    fn n(x: f64) -> V {
        V::Number(x)
    }
    fn pt(c: &[f64]) -> V {
        point(c)
    }
    fn volume(v: &V) -> f64 {
        flatten(v)
            .unwrap()
            .iter()
            .filter_map(|g| g.solid().map(Solid::volume))
            .sum()
    }
    fn area(v: &V) -> f64 {
        use geo::Area;
        regions(v)
            .unwrap()
            .iter()
            .map(|(_, s)| s.unsigned_area())
            .sum()
    }

    #[test]
    fn box_volume() {
        let b = apply(
            Op::Box,
            Plane::Xy,
            &[pt(&[0.0, 0.0, 0.0]), pt(&[2.0, 3.0, 4.0])],
        )
        .unwrap();
        assert!((volume(&b) - 24.0).abs() < 1e-12);
    }

    #[test]
    fn extrude_in_each_plane_is_positive() {
        for plane in [Plane::Xy, Plane::Xz, Plane::Yz] {
            let r = apply(Op::Rectangle, plane, &[pt(&[0.0, 0.0]), pt(&[1.0, 2.0])]).unwrap();
            let s = apply(Op::Extrude, plane, &[r, n(0.0), n(0.5)]).unwrap();
            assert!((volume(&s) - 1.0).abs() < 1e-12);
        }
    }

    #[test]
    fn difference_and_union() {
        let a = apply(
            Op::Rectangle,
            Plane::Xy,
            &[pt(&[0.0, 0.0]), pt(&[4.0, 4.0])],
        )
        .unwrap();
        let b = apply(
            Op::Rectangle,
            Plane::Xy,
            &[pt(&[1.0, 1.0]), pt(&[2.0, 2.0])],
        )
        .unwrap();
        let d = apply(Op::Difference, Plane::Xy, &[a.clone(), b.clone()]).unwrap();
        assert!((area(&d) - 15.0).abs() < 1e-9);
        let c = apply(
            Op::Rectangle,
            Plane::Xy,
            &[pt(&[3.0, 0.0]), pt(&[6.0, 1.0])],
        )
        .unwrap();
        let u = apply(Op::Union, Plane::Xy, &[V::List(vec![a, c])]).unwrap();
        assert!((area(&u) - 18.0).abs() < 1e-9);
        let e = apply(Op::Extrude, Plane::Xy, &[d, n(0.0), n(1.0)]).unwrap();
        assert!((volume(&e) - 15.0).abs() < 1e-9);
    }

    #[test]
    fn band_with_a_corner_is_mitred() {
        let path = V::List(vec![pt(&[0.0, 0.0]), pt(&[4.0, 0.0]), pt(&[4.0, 3.0])]);
        let b = apply(Op::Band, Plane::Xy, &[path, n(0.1), n(-0.1)]).unwrap();
        // Centre-line length 7 × width 0.2; the mitred corner adds nothing net.
        assert!((area(&b) - 1.4).abs() < 1e-9, "area {}", area(&b));
    }

    #[test]
    fn band_on_a_zero_length_path_is_empty() {
        let path = V::List(vec![pt(&[1.0, 1.0]), pt(&[1.0, 1.0])]);
        let b = apply(Op::Band, Plane::Xy, &[path, n(0.1), n(-0.1)]).unwrap();
        assert_eq!(area(&b), 0.0);
    }

    #[test]
    fn sweep_and_cylinder() {
        let sq = apply(
            Op::Rectangle,
            Plane::Profile,
            &[pt(&[-0.05, -0.05]), pt(&[0.05, 0.05])],
        )
        .unwrap();
        let path = V::List(vec![pt(&[0.0, 0.0, 0.0]), pt(&[3.0, 0.0, 4.0])]);
        let s = apply(Op::Sweep, Plane::Xy, &[sq, path]).unwrap();
        assert!((volume(&s) - 0.01 * 5.0).abs() < 1e-12);
        let c = apply(
            Op::Cylinder,
            Plane::Xy,
            &[pt(&[0.0, 0.0, 0.0]), pt(&[0.0, 0.0, 2.0]), n(0.5)],
        )
        .unwrap();
        // 24-gon prism.
        let expected = 0.5 * 24.0 * 0.25 * (std::f64::consts::TAU / 24.0).sin() * 2.0;
        assert!((volume(&c) - expected).abs() < 1e-12);
    }

    #[test]
    fn revolve_a_rectangle_into_a_ring() {
        let r = apply(
            Op::Rectangle,
            Plane::Xz,
            &[pt(&[1.0, 0.0]), pt(&[2.0, 1.0])],
        )
        .unwrap();
        let s = apply(
            Op::Revolve,
            Plane::Xy,
            &[
                r,
                pt(&[0.0, 0.0, 0.0]),
                pt(&[0.0, 0.0, 1.0]),
                n(360.0),
                n(96.0),
            ],
        )
        .unwrap();
        let exact = std::f64::consts::PI * (4.0 - 1.0);
        assert!((volume(&s) - exact).abs() / exact < 0.01);
    }

    #[test]
    fn mirror_keeps_volume_positive() {
        let b = apply(
            Op::Box,
            Plane::Xy,
            &[pt(&[0.0, 0.0, 0.0]), pt(&[1.0, 1.0, 1.0])],
        )
        .unwrap();
        let m = apply(Op::Mirror, Plane::Xy, &[b, V::String("x".into())]).unwrap();
        let g = &flatten(&m).unwrap()[0];
        assert!(g.solid().unwrap().points.iter().all(|p| p[0] <= 0.0));
        assert!((volume(&m) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn move_and_rotate() {
        let r = apply(
            Op::Rectangle,
            Plane::Xy,
            &[pt(&[0.0, 0.0]), pt(&[1.0, 1.0])],
        )
        .unwrap();
        let m = apply(Op::Move, Plane::Xy, &[r.clone(), pt(&[2.0, 0.0])]).unwrap();
        let rot = apply(Op::Rotate, Plane::Xy, &[m, pt(&[0.0, 0.0]), n(90.0)]).unwrap();
        let Shape::Region { shape, .. } = &flatten(&rot).unwrap()[0].shape else {
            panic!()
        };
        let c = shape.0[0].exterior().0[0];
        assert!((c.x - 0.0).abs() < 1e-12 && (c.y - 2.0).abs() < 1e-12);
    }

    #[test]
    fn arc_point_count() {
        let a = apply(
            Op::Arc,
            Plane::Xy,
            &[pt(&[0.0, 0.0]), n(1.0), n(0.0), n(90.0)],
        )
        .unwrap();
        let Shape::Curve { points, .. } = &flatten(&a).unwrap()[0].shape else {
            panic!()
        };
        assert_eq!(points.len(), 10); // floor(32 × 90 / 360) + 2
    }

    #[test]
    fn boundary_curves_are_closed_rings() {
        let r = apply(
            Op::Rectangle,
            Plane::Xy,
            &[pt(&[0.0, 0.0]), pt(&[1.0, 1.0])],
        )
        .unwrap();
        let c = apply(Op::BoundaryCurves, Plane::Xy, &[r]).unwrap();
        let Shape::Curve { points, .. } = &flatten(&c).unwrap()[0].shape else {
            panic!()
        };
        assert_eq!(points.first(), points.last());
    }

    #[test]
    fn bad_arguments_are_errors() {
        assert!(apply(Op::Rectangle, Plane::Xy, &[n(1.0), pt(&[1.0, 1.0])]).is_err());
        assert!(apply(Op::Mirror, Plane::Xy, &[V::Null, V::String("z".into())]).is_err());
        assert!(apply(Op::Difference, Plane::Xy, &[V::Null, V::Null]).is_err());
    }
}
