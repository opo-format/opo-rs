//! Triangulation and mesh building shared by extrude, sweep and revolve.

use geo::orient::{Direction, Orient};
use geo::{Coord, LineString, Polygon};

use crate::Error;

/// A polygon triangulated for meshing: its vertices (exterior counter-clockwise, then holes
/// clockwise), the ring lengths, and triangles that are all counter-clockwise.
pub(crate) struct Triangulated {
    pub verts: Vec<[f64; 2]>,
    pub rings: Vec<usize>,
    pub tris: Vec<[usize; 3]>,
}

fn ring_points(ls: &LineString<f64>) -> Vec<[f64; 2]> {
    let mut pts: Vec<[f64; 2]> = ls.coords().map(|c| [c.x, c.y]).collect();
    if pts.len() > 1 && pts.first() == pts.last() {
        pts.pop();
    }
    pts
}

/// Triangulate a polygon with earcut after orienting it (exterior CCW, holes CW).
pub(crate) fn triangulate(poly: &Polygon<f64>) -> Result<Triangulated, Error> {
    let poly = poly.orient(Direction::Default);
    let mut verts = ring_points(poly.exterior());
    let mut rings = vec![verts.len()];
    let mut holes = Vec::new();
    for r in poly.interiors() {
        holes.push(verts.len());
        let pts = ring_points(r);
        rings.push(pts.len());
        verts.extend(pts);
    }
    let flat: Vec<f64> = verts.iter().flat_map(|p| [p[0], p[1]]).collect();
    let idx = earcutr::earcut(&flat, &holes, 2)
        .map_err(|e| Error::Op(format!("triangulation failed: {e:?}")))?;
    let tris = idx
        .chunks_exact(3)
        .map(|t| {
            let (a, b, c) = (verts[t[0]], verts[t[1]], verts[t[2]]);
            let cross = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
            if cross < 0.0 {
                [t[0], t[2], t[1]]
            } else {
                [t[0], t[1], t[2]]
            }
        })
        .collect();
    Ok(Triangulated { verts, rings, tris })
}

/// Faces of a prism over a triangulated profile whose base vertices are `0..n` and top
/// vertices `n..2n`: bottom cap reversed, top cap, and two triangles per ring edge.
pub(crate) fn prism_faces(t: &Triangulated) -> Vec<[usize; 3]> {
    let n = t.verts.len();
    let mut faces: Vec<[usize; 3]> = t.tris.iter().map(|&[a, b, c]| [a, c, b]).collect();
    faces.extend(t.tris.iter().map(|&[a, b, c]| [a + n, b + n, c + n]));
    let mut start = 0;
    for &m in &t.rings {
        for i in 0..m {
            let (a, b) = (start + i, start + (i + 1) % m);
            faces.push([a, b, b + n]);
            faces.push([a, b + n, a + n]);
        }
        start += m;
    }
    faces
}

/// A polygon from (u, v) points (closed automatically).
pub(crate) fn polygon(points: &[[f64; 2]]) -> Polygon<f64> {
    Polygon::new(
        LineString::from(
            points
                .iter()
                .map(|p| Coord { x: p[0], y: p[1] })
                .collect::<Vec<_>>(),
        ),
        vec![],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_with_hole() {
        let mut p = polygon(&[[0.0, 0.0], [0.0, 4.0], [4.0, 4.0], [4.0, 0.0]]); // clockwise on purpose
        p.interiors_push(LineString::from(vec![
            (1.0, 1.0),
            (2.0, 1.0),
            (2.0, 2.0),
            (1.0, 2.0),
        ]));
        let t = triangulate(&p).unwrap();
        assert_eq!(t.rings, vec![4, 4]);
        let area: f64 = t
            .tris
            .iter()
            .map(|&[a, b, c]| {
                let (a, b, c) = (t.verts[a], t.verts[b], t.verts[c]);
                ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) / 2.0
            })
            .sum();
        assert!(
            (area - 15.0).abs() < 1e-12,
            "all triangles counter-clockwise, area 16 - 1"
        );
        assert_eq!(prism_faces(&t).len(), 2 * t.tris.len() + 2 * 8);
    }
}
