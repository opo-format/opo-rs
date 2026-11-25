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
        for ((lo, hi), x) in self.min.iter_mut().zip(self.max.iter_mut()).zip(p) {
            *lo = lo.min(x);
            *hi = hi.max(x);
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

fn items_of(rep: &Representation) -> impl Iterator<Item = &RepresentationItem> {
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

/// The display colour of one geometry of a representation item: the item's material, or the
/// material a nested object gave it, looked up in the object's materials; grey when unknown.
pub fn solid_colour(
    def: &TypeDefinition,
    item: &RepresentationItem,
    ev: &Evaluation,
    g: &Geometry,
) -> String {
    let key = match item.material.as_deref() {
        Some(m) => material_key(m, item, ev, g),
        None => match &g.material {
            Some(Value::String(s)) => Some(s.clone()),
            _ => None,
        },
    };
    key.and_then(|m| {
        def.materials
            .iter()
            .flatten()
            .find(|(k, _)| *k == m)
            .and_then(|(_, mat)| mat.colour.clone())
    })
    .unwrap_or_else(|| "#bbbbbb".into())
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
/// coloured by material, shaded by a light from the upper left and outlined part by part. The
/// view is drawn with a depth buffer and embedded as an image, so hidden parts stay hidden.
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
    // Every visible triangle with its solid, three depths and a shaded colour.
    let mut tris: Vec<Tri> = Vec::new();
    let mut bounds = Bounds::new();
    let mut solid_no = 0u32;
    for item in items_of(rep) {
        // A void cuts the host; it is not drawn.
        if item.role.as_deref() == Some("void") {
            continue;
        }
        for g in ev
            .items(&item.geometry)
            .map_err(|e| Error::Recipe(e.to_string()))?
        {
            let Some(s) = g.solid() else { continue };
            let colour = solid_colour(def, item, ev, &g);
            let base = hex(&colour);
            solid_no += 1;
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
                let rgb = [0, 1, 2].map(|i| (base[i] * shade).round().clamp(0.0, 255.0) as u8);
                let proj = [a, b, c].map(|p| [dot(p, right), dot(p, up)]);
                proj.iter().for_each(|q| bounds.add(*q));
                tris.push(Tri {
                    screen: proj,
                    depth: [a, b, c].map(|p| dot(p, eye)),
                    rgb,
                    solid: solid_no,
                });
            }
        }
    }
    let body = if tris.is_empty() {
        String::new()
    } else {
        raster_image(&tris, &bounds)
    };
    Ok(svg_document(bounds, &body, true))
}

/// One visible triangle of the 3D view.
struct Tri {
    screen: [[f64; 2]; 3],
    /// Distance towards the eye at each corner: larger is nearer.
    depth: [f64; 3],
    rgb: [u8; 3],
    solid: u32,
}

/// Pixels on the longer side of the 3D image, before 2× supersampling.
const IMAGE_PIXELS: usize = 1600;

/// Draw the triangles with a depth buffer, so every part hides what is behind it, outline each
/// part, and return an SVG `<image>` over the drawing's bounds.
fn raster_image(tris: &[Tri], b: &Bounds) -> String {
    let ss = 2usize;
    let (w_m, h_m) = (b.max[0] - b.min[0], b.max[1] - b.min[1]);
    let scale = (IMAGE_PIXELS * ss) as f64 / w_m.max(h_m).max(1e-9);
    let (w, h) = (
        ((w_m * scale).ceil() as usize).max(1),
        ((h_m * scale).ceil() as usize).max(1),
    );
    let mut depth = vec![f64::NEG_INFINITY; w * h];
    let mut id = vec![0u32; w * h];
    let mut rgb = vec![[0u8; 3]; w * h];
    for t in tris {
        // Screen y grows downwards in the image.
        let p = t
            .screen
            .map(|q| [(q[0] - b.min[0]) * scale, (b.max[1] - q[1]) * scale]);
        let area =
            (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
        if area.abs() < 1e-12 {
            continue;
        }
        let x0 = p
            .iter()
            .map(|q| q[0])
            .fold(f64::INFINITY, f64::min)
            .floor()
            .max(0.0) as usize;
        let x1 = (p
            .iter()
            .map(|q| q[0])
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil() as usize)
            .min(w);
        let y0 = p
            .iter()
            .map(|q| q[1])
            .fold(f64::INFINITY, f64::min)
            .floor()
            .max(0.0) as usize;
        let y1 = (p
            .iter()
            .map(|q| q[1])
            .fold(f64::NEG_INFINITY, f64::max)
            .ceil() as usize)
            .min(h);
        for y in y0..y1 {
            for x in x0..x1 {
                let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                let e = |i: usize, j: usize| {
                    ((p[j][0] - p[i][0]) * (py - p[i][1]) - (p[j][1] - p[i][1]) * (px - p[i][0]))
                        / area
                };
                let (l0, l1, l2) = (e(1, 2), e(2, 0), e(0, 1));
                if l0 < -1e-9 || l1 < -1e-9 || l2 < -1e-9 {
                    continue;
                }
                let z = l0 * t.depth[0] + l1 * t.depth[1] + l2 * t.depth[2];
                let k = y * w + x;
                // Ties go to the part drawn first, so touching faces don't flicker.
                if z > depth[k] + 1e-7 {
                    depth[k] = z;
                    id[k] = t.solid;
                    rgb[k] = t.rgb;
                }
            }
        }
    }
    // Outline where one part meets another or the background.
    let edge = |k: usize, x: usize, y: usize| {
        let me = id[k];
        (x + 1 < w && id[k + 1] != me) || (y + 1 < h && id[k + w] != me)
    };
    let (ow, oh) = (w.div_ceil(ss), h.div_ceil(ss));
    let mut out = vec![0u8; ow * oh * 4];
    for oy in 0..oh {
        for ox in 0..ow {
            let (mut acc, mut n_in) = ([0f64; 4], 0f64);
            for dy in 0..ss {
                for dx in 0..ss {
                    let (x, y) = (ox * ss + dx, oy * ss + dy);
                    if x >= w || y >= h {
                        continue;
                    }
                    let k = y * w + x;
                    let line = edge(k, x, y);
                    if id[k] == 0 && !line {
                        continue;
                    }
                    let c = if line {
                        [40.0, 40.0, 40.0]
                    } else {
                        rgb[k].map(f64::from)
                    };
                    acc[0] += c[0];
                    acc[1] += c[1];
                    acc[2] += c[2];
                    n_in += 1.0;
                }
            }
            let o = (oy * ow + ox) * 4;
            if n_in > 0.0 {
                for i in 0..3 {
                    out[o + i] = (acc[i] / n_in).round() as u8;
                }
                out[o + 3] = (255.0 * n_in / (ss * ss) as f64).round() as u8;
            }
        }
    }
    let mut png_bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png_bytes, ow as u32, oh as u32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        if let Ok(mut wr) = enc.write_header() {
            let _ = wr.write_image_data(&out);
        }
    }
    format!(
        "<image x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" preserveAspectRatio=\"none\" href=\"data:image/png;base64,{}\"/>\n",
        n(b.min[0]),
        n(-b.max[1]),
        n(ow as f64 * ss as f64 / scale),
        n(oh as f64 * ss as f64 / scale),
        base64(&png_bytes)
    )
}

/// Standard base64 (RFC 4648) with padding.
fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let v = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                s.push(A[(v >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
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
    fn nearer_parts_hide_farther_ones_whatever_the_order() {
        let tri = |depth: f64, rgb: [u8; 3], solid: u32| Tri {
            screen: [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            depth: [depth; 3],
            rgb,
            solid,
        };
        let mut b = Bounds::new();
        b.add([0.0, 0.0]);
        b.add([1.0, 1.0]);
        // The near red triangle comes first; the far blue one must not cover it.
        let svg = raster_image(&[tri(1.0, [255, 0, 0], 1), tri(-1.0, [0, 0, 255], 2)], &b);
        let data = svg
            .split("base64,")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        let bytes = unbase64(data);
        let dec = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut r = dec.read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size()];
        let info = r.next_frame(&mut buf).unwrap();
        let (w, h) = (info.width as usize, info.height as usize);
        let k = ((h * 3 / 4) * w + w / 4) * 4;
        assert_eq!(&buf[k..k + 4], &[255, 0, 0, 255], "red in front");
        let corner = ((h / 8) * w + w * 7 / 8) * 4;
        assert_eq!(buf[corner + 3], 0, "empty outside the triangle");
    }

    fn unbase64(s: &str) -> Vec<u8> {
        const A: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let v: Vec<u32> = s
            .bytes()
            .filter(|c| *c != b'=')
            .map(|c| A.find(c as char).unwrap() as u32)
            .collect();
        let mut out = Vec::new();
        for c in v.chunks(4) {
            let n = c
                .iter()
                .enumerate()
                .fold(0u32, |a, (i, x)| a | x << (18 - 6 * i));
            for i in 0..c.len() - 1 {
                out.push((n >> (16 - 8 * i)) as u8);
            }
        }
        out
    }

    #[test]
    fn base64_matches_rfc_4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn colours_parse() {
        assert_eq!(hex("#ff0080"), [255.0, 0.0, 128.0]);
        assert_eq!(hex("red"), [187.0; 3]);
    }
}
