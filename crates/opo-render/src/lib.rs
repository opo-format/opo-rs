//! SVG drawings of OPO objects.
//!
//! - [`plan_svg`] draws a 2D representation (e.g. `plan.medium`) at a drawing scale: regions
//!   hatched from the object's own `opo::materials` and `opo::hatches` (decision 0011), lines
//!   styled by their semantic class (decision 0006).
//! - [`model_svg`] draws the solids of a 3D representation (e.g. `model.medium`) in an
//!   axonometric view, shaded and coloured by material.
//!
//! Output is deterministic SVG 1.1 text, so drawings can be compared and diffed. Lengths in the
//! SVG are model metres; line widths and hatch spacing are given on paper (millimetres and
//! paper metres) and multiplied by the scale.

use std::fmt::Write as _;

use opo_feel::Value;
use opo_model::{
    HatchKind, LineClass, Representation, RepresentationItem, ScaleMode, TypeDefinition,
};
use opo_recipe::{Evaluation, Geometry, Shape, resolve_material};

/// An error drawing an object.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// The representation does not exist.
    NoRepresentation(String),
    /// The recipe output could not be read.
    Recipe(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoRepresentation(r) => write!(f, "no representation {r}"),
            Error::Recipe(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

/// Format a coordinate with at most 6 decimals (micrometres), without trailing zeros.
fn n(x: f64) -> String {
    let s = format!("{:.6}", x + 0.0);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_owned() }
}

/// Line style on paper: width in mm, dash pattern in mm, colour.
struct Pen {
    width: f64,
    dash: &'static [f64],
    colour: &'static str,
}

fn pen(class: Option<LineClass>, role: Option<&str>) -> Pen {
    if role == Some("clearance") {
        return Pen {
            width: 0.13,
            dash: &[0.5, 1.0],
            colour: "#777777",
        };
    }
    let mut p = match class.unwrap_or(LineClass::Projected) {
        LineClass::Cut => Pen {
            width: 0.35,
            dash: &[],
            colour: "#000000",
        },
        LineClass::Projected => Pen {
            width: 0.18,
            dash: &[],
            colour: "#000000",
        },
        LineClass::Overhead => Pen {
            width: 0.18,
            dash: &[3.0, 1.5],
            colour: "#000000",
        },
        LineClass::Hidden => Pen {
            width: 0.18,
            dash: &[1.5, 1.5],
            colour: "#000000",
        },
        LineClass::Annotation => Pen {
            width: 0.13,
            dash: &[],
            colour: "#555555",
        },
    };
    if role == Some("outline") {
        p.width *= 1.8;
    }
    p
}

fn stroke(p: &Pen, scale: f64) -> String {
    let k = scale / 1000.0;
    let mut s = format!(
        "fill=\"none\" stroke=\"{}\" stroke-width=\"{}\" stroke-linejoin=\"round\"",
        p.colour,
        n(p.width * k)
    );
    if !p.dash.is_empty() {
        let d: Vec<String> = p.dash.iter().map(|x| n(x * k)).collect();
        write!(s, " stroke-dasharray=\"{}\"", d.join(" ")).unwrap();
    }
    s
}

/// A bounding box that grows.
#[derive(Clone, Copy)]
struct Bounds {
    min: [f64; 2],
    max: [f64; 2],
}

impl Bounds {
    fn new() -> Self {
        Bounds {
            min: [f64::INFINITY; 2],
            max: [f64::NEG_INFINITY; 2],
        }
    }
    fn add(&mut self, p: [f64; 2]) {
        for i in 0..2 {
            self.min[i] = self.min[i].min(p[i]);
            self.max[i] = self.max[i].max(p[i]);
        }
    }
    fn empty(&self) -> bool {
        self.min[0] > self.max[0]
    }
    fn diagonal(&self) -> f64 {
        ((self.max[0] - self.min[0]).powi(2) + (self.max[1] - self.min[1]).powi(2)).sqrt()
    }
}

fn representation<'a>(def: &'a TypeDefinition, id: &str) -> Result<&'a Representation, Error> {
    def.representation(id)
        .ok_or_else(|| Error::NoRepresentation(id.into()))
}

fn items_of<'a>(rep: &'a Representation) -> impl Iterator<Item = &'a RepresentationItem> {
    rep.items
        .iter()
        .flatten()
        .chain(rep.overlay.iter().flatten())
}

/// The material key an expression (an item's `material` or `hatch`) gives for a geometry.
fn material_key(
    expr: &str,
    item: &RepresentationItem,
    ev: &Evaluation,
    g: &Geometry,
) -> Option<String> {
    let probe = RepresentationItem {
        material: Some(expr.into()),
        ..item.clone()
    };
    match resolve_material(&probe, &ev.env, g).ok().flatten()? {
        Value::String(s) => Some(s),
        _ => None,
    }
}

fn svg_document(b: Bounds, body: &str, flip: bool) -> String {
    let (b, empty) = if b.empty() {
        (
            Bounds {
                min: [0.0, 0.0],
                max: [1.0, 1.0],
            },
            true,
        )
    } else {
        (b, false)
    };
    let m = (b.diagonal() * 0.05).max(0.01);
    let (x, w, h) = (
        b.min[0] - m,
        b.max[0] - b.min[0] + 2.0 * m,
        b.max[1] - b.min[1] + 2.0 * m,
    );
    let y = if flip { -(b.max[1] + m) } else { b.min[1] - m };
    let mut s = String::new();
    writeln!(s, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>").unwrap();
    writeln!(
        s,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" version=\"1.1\" viewBox=\"{} {} {} {}\" width=\"{}mm\" height=\"{}mm\">",
        n(x),
        n(y),
        n(w),
        n(h),
        n(w * 1000.0 / 50.0),
        n(h * 1000.0 / 50.0)
    )
    .unwrap();
    if !empty {
        s.push_str(body);
    }
    s.push_str("</svg>\n");
    s
}

// ------------------------------------------------------------------ plans

/// The SVG dash array of a hatch family: + dash, − gap, 0 dot.
fn dash_array(dashes: &[f64]) -> Vec<f64> {
    let mut out = Vec::new();
    let mut want_dash = true;
    for &x in dashes {
        if x >= 0.0 {
            if !want_dash {
                out.push(0.0);
            }
            out.push(x);
            want_dash = false;
        } else {
            if want_dash {
                out.push(0.0);
            }
            out.push(-x);
            want_dash = true;
        }
    }
    if out.len() % 2 == 1 {
        out.push(0.0);
    }
    out
}

fn ring_path(points: impl Iterator<Item = [f64; 2]>, flip: f64) -> String {
    let mut d = String::new();
    for (i, p) in points.enumerate() {
        write!(
            d,
            "{}{} {} ",
            if i == 0 { "M" } else { "L" },
            n(p[0]),
            n(p[1] * flip)
        )
        .unwrap();
    }
    d.push('Z');
    d
}

/// Draw a 2D representation as an SVG plan at a scale (e.g. 50 for 1:50).
pub fn plan_svg(
    def: &TypeDefinition,
    ev: &Evaluation,
    representation_id: &str,
    scale: f64,
) -> Result<String, Error> {
    let rep = representation(def, representation_id)?;
    let k = scale / 1000.0;
    let mut bounds = Bounds::new();
    let mut defs = String::new();
    let mut body = String::new();
    let mut clip = 0usize;
    for item in items_of(rep) {
        let p = pen(item.class, item.role.as_deref());
        let geoms = ev
            .items(&item.geometry)
            .map_err(|e| Error::Recipe(e.to_string()))?;
        for g in &geoms {
            match &g.shape {
                Shape::Region { shape, .. } => {
                    let mut d = String::new();
                    for poly in shape {
                        for ring in std::iter::once(poly.exterior()).chain(poly.interiors()) {
                            let pts: Vec<[f64; 2]> = ring.coords().map(|c| [c.x, c.y]).collect();
                            pts.iter().for_each(|q| bounds.add([q[0], q[1]]));
                            d.push_str(&ring_path(pts.into_iter(), -1.0));
                            d.push(' ');
                        }
                    }
                    let d = d.trim_end().to_owned();
                    if d.is_empty() {
                        continue;
                    }
                    let hatch = item
                        .hatch
                        .as_deref()
                        .and_then(|h| material_key(h, item, ev, g))
                        .and_then(|m| {
                            def.materials
                                .iter()
                                .flatten()
                                .find(|(key, _)| *key == m)
                                .map(|(_, mat)| mat.cut.hatch.clone())
                        })
                        .and_then(|h| {
                            def.hatches
                                .iter()
                                .flatten()
                                .find(|(key, _)| *key == h)
                                .map(|(_, hd)| hd.clone())
                        });
                    match hatch.as_ref().map(|h| h.kind) {
                        Some(HatchKind::Solid) => {
                            let h = hatch.as_ref().expect("checked");
                            writeln!(body, "<path d=\"{d}\" fill=\"{}\" fill-rule=\"evenodd\" stroke=\"none\"/>", h.colour.as_deref().unwrap_or("#333333")).unwrap();
                        }
                        Some(HatchKind::Lines) => {
                            let h = hatch.as_ref().expect("checked");
                            clip += 1;
                            writeln!(defs, "<clipPath id=\"c{clip}\"><path d=\"{d}\" clip-rule=\"evenodd\"/></clipPath>").unwrap();
                            let hk = if h.scale_mode == Some(ScaleMode::Model) {
                                1.0
                            } else {
                                scale
                            };
                            let colour = h.colour.as_deref().unwrap_or("#222222");
                            let mut lines = String::new();
                            let (half, centre) = region_bounds(shape);
                            let r = half + 1e-3;
                            for f in h.lines.iter().flatten() {
                                let a = f.angle.to_radians();
                                let (dx, dy) = (a.cos(), a.sin());
                                let (nx, ny) = (-dy, dx);
                                let o = f.origin.unwrap_or([0.0, 0.0]);
                                let (ox, oy) = (o[0] * hk, o[1] * hk);
                                let (along, across) = (f.offset[0] * hk, f.offset[1] * hk);
                                if across.abs() < 1e-12 {
                                    continue;
                                }
                                let (cx, cy) = (centre[0], centre[1]);
                                let c_n = (cx - ox) * nx + (cy - oy) * ny;
                                let (mut i0, mut i1) = (
                                    ((c_n - r) / across).floor() as i64,
                                    ((c_n + r) / across).ceil() as i64,
                                );
                                if i0 > i1 {
                                    std::mem::swap(&mut i0, &mut i1);
                                }
                                if i1 - i0 > 5000 {
                                    continue; // a hatch far too dense for the region: skip rather than flood
                                }
                                let dashes: Vec<f64> =
                                    f.dashes.iter().flatten().map(|x| x * hk).collect();
                                let period: f64 = dashes.iter().map(|x| x.abs()).sum();
                                let da = dash_array(&dashes);
                                for i in i0..=i1 {
                                    let (bx, by) = (
                                        ox + i as f64 * (along * dx + across * nx),
                                        oy + i as f64 * (along * dy + across * ny),
                                    );
                                    let tc = (cx - bx) * dx + (cy - by) * dy;
                                    let mut t0 = tc - r;
                                    if period > 0.0 {
                                        t0 = (t0 / period).floor() * period; // dashes start at the line's base point
                                    }
                                    let t1 = tc + r;
                                    write!(
                                        lines,
                                        "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\"",
                                        n(bx + t0 * dx),
                                        n(-(by + t0 * dy)),
                                        n(bx + t1 * dx),
                                        n(-(by + t1 * dy))
                                    )
                                    .unwrap();
                                    if !da.is_empty() && period > 0.0 {
                                        let d: Vec<String> = da.iter().map(|x| n(*x)).collect();
                                        write!(lines, " stroke-dasharray=\"{}\"", d.join(" "))
                                            .unwrap();
                                    }
                                    lines.push_str("/>");
                                }
                            }
                            writeln!(
                                body,
                                "<g clip-path=\"url(#c{clip})\" stroke=\"{colour}\" stroke-width=\"{}\" stroke-linecap=\"round\">{lines}</g>",
                                n(0.13 * k)
                            )
                            .unwrap();
                        }
                        _ => {}
                    }
                    writeln!(
                        body,
                        "<path d=\"{d}\" fill-rule=\"evenodd\" {}/>",
                        stroke(&p, scale)
                    )
                    .unwrap();
                }
                Shape::Curve { points, .. } => {
                    points.iter().for_each(|q| bounds.add(*q));
                    let pts: Vec<String> = points
                        .iter()
                        .map(|q| format!("{},{}", n(q[0]), n(-q[1])))
                        .collect();
                    writeln!(
                        body,
                        "<polyline points=\"{}\" {}/>",
                        pts.join(" "),
                        stroke(&p, scale)
                    )
                    .unwrap();
                }
                Shape::Text {
                    at, value, height, ..
                } => {
                    bounds.add(*at);
                    let size = (height * scale).max(1e-4);
                    let escaped = value
                        .replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;");
                    writeln!(
                        body,
                        "<text x=\"{}\" y=\"{}\" font-family=\"sans-serif\" font-size=\"{}\" text-anchor=\"middle\" dominant-baseline=\"central\">{escaped}</text>",
                        n(at[0]),
                        n(-at[1]),
                        n(size)
                    )
                    .unwrap();
                }
                Shape::Curve3 { .. } | Shape::Solid(_) => {}
            }
        }
    }
    let mut all = String::new();
    if !defs.is_empty() {
        writeln!(all, "<defs>\n{defs}</defs>").unwrap();
    }
    all.push_str(&body);
    Ok(svg_document(bounds, &all, true))
}

/// The half-diagonal and the centre of the bounding box of a region.
fn region_bounds(shape: &opo_recipe::MultiPolygon<f64>) -> (f64, [f64; 2]) {
    let mut b = Bounds::new();
    for p in shape {
        for c in p.exterior().coords() {
            b.add([c.x, c.y]);
        }
    }
    (
        b.diagonal() / 2.0,
        [(b.min[0] + b.max[0]) / 2.0, (b.min[1] + b.max[1]) / 2.0],
    )
}

// ------------------------------------------------------------------ 3D

type V3 = [f64; 3];

fn hex(c: &str) -> [f64; 3] {
    let v =
        |i: usize| u8::from_str_radix(c.get(i..i + 2).unwrap_or("bb"), 16).unwrap_or(0xbb) as f64;
    if c.len() == 7 {
        [v(1), v(3), v(5)]
    } else {
        [187.0; 3]
    }
}

/// Draw the solids of a 3D representation in an axonometric view (elevation 24°, azimuth −58°),
/// coloured by material and shaded by a light from the upper left.
pub fn model_svg(
    def: &TypeDefinition,
    ev: &Evaluation,
    representation_id: &str,
) -> Result<String, Error> {
    let rep = representation(def, representation_id)?;
    let (elev, azim) = (24f64.to_radians(), (-58f64).to_radians());
    let eye: V3 = [elev.cos() * azim.cos(), elev.cos() * azim.sin(), elev.sin()];
    let right: V3 = [-azim.sin(), azim.cos(), 0.0];
    let up: V3 = [
        -elev.sin() * azim.cos(),
        -elev.sin() * azim.sin(),
        elev.cos(),
    ];
    let light = {
        let l: V3 = [
            eye[0] + 0.4 * up[0] - 0.5 * right[0],
            eye[1] + 0.4 * up[1] - 0.5 * right[1],
            eye[2] + 0.4 * up[2] - 0.5 * right[2],
        ];
        let len = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        [l[0] / len, l[1] / len, l[2] / len]
    };
    let dot = |a: V3, b: V3| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    // One entry per solid: (depth of its centre, its visible triangles with their depths).
    let mut solids: Vec<(f64, Vec<(f64, [[f64; 2]; 3], String)>)> = Vec::new();
    let mut bounds = Bounds::new();
    for item in items_of(rep) {
        for g in ev
            .items(&item.geometry)
            .map_err(|e| Error::Recipe(e.to_string()))?
        {
            let Some(s) = g.solid() else { continue };
            let colour = item
                .material
                .as_deref()
                .and_then(|m| material_key(m, item, ev, &g))
                .and_then(|m| {
                    def.materials
                        .iter()
                        .flatten()
                        .find(|(key, _)| *key == m)
                        .and_then(|(_, mat)| mat.colour.clone())
                })
                .unwrap_or_else(|| "#bbbbbb".into());
            let base = hex(&colour);
            let mut tris = Vec::new();
            let centre = s
                .points
                .iter()
                .fold([0.0; 3], |a, p| [a[0] + p[0], a[1] + p[1], a[2] + p[2]]);
            let k = 1.0 / s.points.len().max(1) as f64;
            let centre_depth = dot([centre[0] * k, centre[1] * k, centre[2] * k], eye);
            for f in &s.faces {
                let [a, b, c] = f.map(|i| s.points[i]);
                let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                let nrm = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                let len = dot(nrm, nrm).sqrt();
                if len < 1e-15 || dot(nrm, eye) <= 0.0 {
                    continue; // degenerate or facing away
                }
                let nrm = [nrm[0] / len, nrm[1] / len, nrm[2] / len];
                let shade = 0.55 + 0.45 * dot(nrm, light).max(0.0);
                let col = format!(
                    "#{:02x}{:02x}{:02x}",
                    (base[0] * shade).round().clamp(0.0, 255.0) as u8,
                    (base[1] * shade).round().clamp(0.0, 255.0) as u8,
                    (base[2] * shade).round().clamp(0.0, 255.0) as u8
                );
                let proj = [a, b, c].map(|p| [dot(p, right), dot(p, up)]);
                proj.iter().for_each(|q| bounds.add(*q));
                let depth = (dot(a, eye) + dot(b, eye) + dot(c, eye)) / 3.0;
                tris.push((depth, proj, col));
            }
            tris.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
            solids.push((centre_depth, tris));
        }
    }
    // Painter's order by solid: far solids first, and within a solid far triangles first. Back
    // faces are already dropped, so a convex part draws correctly; ties keep their order.
    solids.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut body = String::new();
    for (_, p, col) in solids.iter().flat_map(|(_, t)| t) {
        writeln!(
            body,
            "<polygon points=\"{},{} {},{} {},{}\" fill=\"{col}\" stroke=\"{col}\" stroke-width=\"0.0005\" stroke-linejoin=\"round\"/>",
            n(p[0][0]),
            n(-p[0][1]),
            n(p[1][0]),
            n(-p[1][1]),
            n(p[2][0]),
            n(-p[2][1])
        )
        .unwrap();
    }
    Ok(svg_document(bounds, &body, true))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_short() {
        assert_eq!(n(1.0), "1");
        assert_eq!(n(0.1234567), "0.123457");
        assert_eq!(n(-0.0000001), "0");
        assert_eq!(n(-2.5), "-2.5");
    }

    #[test]
    fn dash_arrays_alternate() {
        assert_eq!(dash_array(&[0.006, -0.0012]), vec![0.006, 0.0012]);
        assert_eq!(dash_array(&[0.0, -0.0027]), vec![0.0, 0.0027]);
        assert_eq!(dash_array(&[-0.1, 0.2]), vec![0.0, 0.1, 0.2, 0.0]);
        assert_eq!(dash_array(&[0.1, 0.2]), vec![0.1, 0.0, 0.2, 0.0]);
    }

    #[test]
    fn pens_follow_line_classes() {
        assert!(
            pen(Some(LineClass::Cut), None).width > pen(Some(LineClass::Projected), None).width
        );
        assert!(!pen(Some(LineClass::Hidden), None).dash.is_empty());
        assert_eq!(pen(None, Some("clearance")).colour, "#777777");
        assert!((pen(Some(LineClass::Cut), Some("outline")).width - 0.63).abs() < 1e-12);
    }

    #[test]
    fn colours_parse() {
        assert_eq!(hex("#ff0080"), [255.0, 0.0, 128.0]);
        assert_eq!(hex("red"), [187.0; 3]);
    }
}
