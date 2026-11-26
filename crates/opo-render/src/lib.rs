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

/// Colour of every dashed line (opening symbols, hidden and overhead lines): light grey.
const DASHED: &str = "#a6a6a6";

fn pen(class: Option<LineClass>, role: Option<&str>) -> Pen {
    if role == Some("opening") {
        // Opening symbols (sash triangles, swing lines) are drawn dashed.
        return Pen {
            width: 0.18,
            dash: &[2.0, 1.0],
            colour: DASHED,
        };
    }
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
            colour: DASHED,
        },
        LineClass::Hidden => Pen {
            width: 0.18,
            dash: &[1.5, 1.5],
            colour: DASHED,
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

/// A dashed polyline split at its corners (turns over 30°), each piece with the dash pattern
/// stretched so it starts and ends with a dash: dashed lines meet at their corners.
fn dashed_polyline(points: &[[f64; 2]], p: &Pen, scale: f64, body: &mut String) {
    let k = scale / 1000.0;
    let mut pieces: Vec<Vec<[f64; 2]>> = vec![vec![]];
    for (i, q) in points.iter().enumerate() {
        pieces.last_mut().expect("one").push(*q);
        if i > 0 && i + 1 < points.len() {
            let (a, b, c) = (points[i - 1], *q, points[i + 1]);
            let (u, v) = ([b[0] - a[0], b[1] - a[1]], [c[0] - b[0], c[1] - b[1]]);
            let (lu, lv) = (u[0].hypot(u[1]), v[0].hypot(v[1]));
            if lu > 1e-12
                && lv > 1e-12
                && (u[0] * v[0] + u[1] * v[1]) / (lu * lv) < 30f64.to_radians().cos()
            {
                pieces.push(vec![*q]);
            }
        }
    }
    let solid = Pen {
        width: p.width,
        dash: &[],
        colour: p.colour,
    };
    for piece in pieces.iter().filter(|x| x.len() > 1) {
        let len: f64 = piece
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
            .sum();
        let pat: Vec<f64> = p.dash.iter().map(|x| x * k).collect();
        let period: f64 = pat.iter().sum();
        let first = pat[0];
        // Whole periods plus a closing dash fill the piece exactly.
        let reps = ((len - first) / period).round().max(0.0);
        let f = if reps < 1.0 {
            1.0
        } else {
            len / (reps * period + first)
        };
        let d: Vec<String> = pat.iter().map(|x| n(x * f)).collect();
        let pts: Vec<String> = piece
            .iter()
            .map(|q| format!("{},{}", n(q[0]), n(-q[1])))
            .collect();
        writeln!(
            body,
            "<polyline points=\"{}\" {} stroke-dasharray=\"{}\"/>",
            pts.join(" "),
            stroke(&solid, scale),
            d.join(" ")
        )
        .unwrap();
    }
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
    static TABLE: std::sync::LazyLock<opo_model::LodTable> =
        std::sync::LazyLock::new(opo_model::LodTable::default);
    opo_recipe::derive::drawn_items(rep, &TABLE)
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
    let (mut defs, mut body, mut bounds) = (String::new(), String::new(), Bounds::new());
    if let Some(d) = &rep.derive {
        derived_plan(
            def,
            ev,
            d,
            rep.lod == opo_model::Lod::Low,
            scale,
            &mut defs,
            &mut body,
            &mut bounds,
        )?;
    }
    let (d2, b2, bb) = draw_2d(def, ev, rep, scale, false)?;
    defs.push_str(&d2);
    body.push_str(&b2);
    if bb.min[0] <= bb.max[0] {
        bounds.add(bb.min);
        bounds.add(bb.max);
    }
    let mut all = String::new();
    if !defs.is_empty() {
        writeln!(all, "<defs>\n{defs}</defs>").unwrap();
    }
    all.push_str(&body);
    Ok(svg_document(bounds, &all, true))
}

/// A host's plan with one hosted object in it (proposal 0004): the host's layers cut at the
/// view's cut height with the object's void cut out, and the object's own plan placed at `x`
/// along the host on its seat. At low detail the host is one region filled solid (if its core
/// is mass construction); at medium its layers are hatched and stop square at the void; at
/// high the object's reveal rules are applied too.
#[allow(clippy::too_many_arguments)]
pub fn hosted_plan_svg(
    host_def: &TypeDefinition,
    host_ev: &Evaluation,
    layer_step: &str,
    hosted_def: &TypeDefinition,
    hosted_ev: &Evaluation,
    hosted_representation: &str,
    x: f64,
    scale: f64,
) -> Result<String, Error> {
    use opo_model::Lod;
    let rep = representation(hosted_def, hosted_representation)?;
    let lod = rep.lod;
    let host =
        opo_recipe::host::provides(host_def, host_ev).map_err(|e| Error::Recipe(e.to_string()))?;
    let seat = opo_recipe::host::seat_y(hosted_def, hosted_ev, &host);
    let regions = opo_recipe::host::cut_host_plan(
        host_def,
        host_ev,
        layer_step,
        hosted_def,
        hosted_ev,
        x,
        lod == Lod::High || lod == Lod::All,
        lod == Lod::Low,
    )
    .map_err(|e| Error::Recipe(e.to_string()))?;
    let (mut defs, mut body, mut bounds) = (String::new(), String::new(), Bounds::new());
    let p = pen(Some(LineClass::Cut), None);
    let mut clip = 5000usize;
    for r in &regions {
        let (hatch, mass) = material_look(host_def, r.material.as_deref());
        let poche = lod == Lod::Low;
        region_fill(
            &r.shape,
            &mut body,
            if poche && mass { POCHE } else { "#ffffff" },
        );
        region_svg(
            &r.shape,
            if poche { None } else { hatch.as_ref() },
            &p,
            scale,
            &mut defs,
            &mut body,
            &mut clip,
            &mut bounds,
        );
    }
    // The hosted object's own plan, moved onto its seat.
    let (mut d2, mut b2, mut bb) = (String::new(), String::new(), Bounds::new());
    if let Some(d) = &rep.derive {
        derived_plan(
            hosted_def,
            hosted_ev,
            d,
            lod == Lod::Low,
            scale,
            &mut d2,
            &mut b2,
            &mut bb,
        )?;
    }
    let (d3, b3, b3b) = draw_2d(hosted_def, hosted_ev, rep, scale, false)?;
    d2.push_str(&d3);
    b2.push_str(&b3);
    for q in [b3b.min, b3b.max] {
        if b3b.min[0] <= b3b.max[0] {
            bb.add(q);
        }
    }
    if bb.min[0] <= bb.max[0] {
        bounds.add([bb.min[0] + x, bb.min[1] + seat]);
        bounds.add([bb.max[0] + x, bb.max[1] + seat]);
    }
    defs.push_str(&d2);
    writeln!(
        body,
        "<g transform=\"translate({} {})\">\n{b2}</g>",
        n(x),
        n(-seat)
    )
    .unwrap();
    let mut all = String::new();
    if !defs.is_empty() {
        writeln!(all, "<defs>\n{defs}</defs>").unwrap();
    }
    all.push_str(&body);
    Ok(svg_document(bounds, &all, true))
}

/// A material's cut hatch, and whether it is mass construction (concrete, masonry, earth),
/// which is filled solid at low detail.
fn material_look(def: &TypeDefinition, key: Option<&str>) -> (Option<opo_model::Hatch>, bool) {
    use opo_model::MaterialCategory as C;
    let Some(m) = key else { return (None, false) };
    let mat = def
        .materials
        .iter()
        .flatten()
        .find(|(k, _)| k.as_str() == m)
        .map(|(_, v)| v);
    let mass = mat.is_some_and(|v| matches!(v.category, C::Concrete | C::Masonry | C::Earth));
    let hatch = mat.and_then(|v| {
        def.hatches
            .iter()
            .flatten()
            .find(|(k, _)| **k == v.cut.hatch)
            .map(|(_, hd)| hd.clone())
    });
    (hatch, mass)
}

/// A plan derived from a 3D representation (spec §4.6): the cut through its solids at the
/// view's cut height, hatched by material, and the edges below the cut seen from above.
#[allow(clippy::too_many_arguments)]
fn derived_plan(
    def: &TypeDefinition,
    ev: &Evaluation,
    d: &opo_model::Derive,
    poche: bool,
    scale: f64,
    defs: &mut String,
    body: &mut String,
    bounds: &mut Bounds,
) -> Result<(), Error> {
    use opo_model::DeriveMethod;
    let h = opo_recipe::derive::cut_height(ev);
    let solids: Vec<(opo_recipe::Solid, Option<opo_model::Hatch>, bool)> =
        opo_recipe::derive::source_solids(
            def,
            ev,
            d,
            Some(opo_model::DrawingView::Plan),
            &opo_model::LodTable::default(),
        )
        .map_err(|e| Error::Recipe(e.to_string()))?
        .into_iter()
        .map(|(s, key)| {
            use opo_model::MaterialCategory as C;
            // Only solid construction (concrete, masonry, earth) is filled at low detail.
            let mass = key.as_ref().is_some_and(|m| {
                def.materials.iter().flatten().any(|(k, mat)| {
                    k == m && matches!(mat.category, C::Concrete | C::Masonry | C::Earth)
                })
            });
            let hatch = key
                .and_then(|m| {
                    def.materials
                        .iter()
                        .flatten()
                        .find(|(k, _)| *k == m)
                        .map(|(_, mat)| mat.cut.hatch.clone())
                })
                .and_then(|h| {
                    def.hatches
                        .iter()
                        .flatten()
                        .find(|(k, _)| *k == h)
                        .map(|(_, hd)| hd.clone())
                });
            (s, hatch, mass)
        })
        .collect();
    if matches!(d.method, DeriveMethod::Project | DeriveMethod::CutOrProject) {
        let refs: Vec<&opo_recipe::Solid> = solids.iter().map(|(s, _, _)| s).collect();
        let p = pen(Some(LineClass::Projected), None);
        let mut path = String::new();
        for [a, b] in opo_recipe::derive::visible_edges_below(&refs, h) {
            bounds.add(a);
            bounds.add(b);
            write!(path, "M{} {}L{} {}", n(a[0]), n(-a[1]), n(b[0]), n(-b[1])).unwrap();
        }
        if !path.is_empty() {
            writeln!(body, "<path d=\"{path}\" {}/>", stroke(&p, scale)).unwrap();
        }
    }
    if matches!(d.method, DeriveMethod::Cut | DeriveMethod::CutOrProject) {
        let p = pen(Some(LineClass::Cut), None);
        let mut clip = 1000usize;
        for (s, hatch, mass) in &solids {
            let shape = opo_recipe::derive::section(s, h);
            if !shape.0.is_empty() {
                // White under the cut so the edges below it do not show through. At low detail
                // (small scales) cut concrete and masonry are filled solid and nothing is
                // hatched; frames and glass are outlined.
                region_fill(&shape, body, if poche && *mass { POCHE } else { "#ffffff" });
                region_svg(
                    &shape,
                    if poche { None } else { hatch.as_ref() },
                    &p,
                    scale,
                    defs,
                    body,
                    &mut clip,
                    bounds,
                );
            }
        }
    }
    Ok(())
}

/// Fill of cut solids in a low-detail derived plan (poché).
const POCHE: &str = "#3c3c3c";

/// A plain fill under or of a cut region.
fn region_fill(shape: &opo_recipe::MultiPolygon<f64>, body: &mut String, colour: &str) {
    let mut d = String::new();
    for poly in shape {
        for ring in std::iter::once(poly.exterior()).chain(poly.interiors()) {
            d.push_str(&ring_path(ring.coords().map(|c| [c.x, c.y]), -1.0));
            d.push(' ');
        }
    }
    writeln!(
        body,
        "<path d=\"{}\" fill=\"{colour}\" fill-rule=\"evenodd\" stroke=\"none\"/>",
        d.trim_end()
    )
    .unwrap();
}

/// One region on a plan: its hatch (if any) clipped to it, then its outline.
#[allow(clippy::too_many_arguments)]
fn region_svg(
    shape: &opo_recipe::MultiPolygon<f64>,
    hatch: Option<&opo_model::Hatch>,
    p: &Pen,
    scale: f64,
    defs: &mut String,
    body: &mut String,
    clip: &mut usize,
    bounds: &mut Bounds,
) {
    let k = scale / 1000.0;
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
        return;
    }
    match hatch.map(|h| h.kind) {
        Some(HatchKind::Solid) => {
            let h = hatch.expect("checked");
            writeln!(
                body,
                "<path d=\"{d}\" fill=\"{}\" fill-rule=\"evenodd\" stroke=\"none\"/>",
                h.colour.as_deref().unwrap_or("#333333")
            )
            .unwrap();
        }
        Some(HatchKind::Lines) => {
            let h = hatch.expect("checked");
            *clip += 1;
            writeln!(
                defs,
                "<clipPath id=\"c{clip}\"><path d=\"{d}\" clip-rule=\"evenodd\"/></clipPath>"
            )
            .unwrap();
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
                    return;
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
                    return; // a hatch far too dense for the region: skip rather than flood
                }
                let dashes: Vec<f64> = f.dashes.iter().flatten().map(|x| x * hk).collect();
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
                        write!(lines, " stroke-dasharray=\"{}\"", d.join(" ")).unwrap();
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
        stroke(p, scale)
    )
    .unwrap();
}

/// The 2D items of a representation as SVG: clip and pattern definitions, the drawing, and its
/// bounds (y flipped for SVG). In an elevation, swing lines are drawn dashed like opening
/// symbols.
fn draw_2d(
    def: &TypeDefinition,
    ev: &Evaluation,
    rep: &opo_model::Representation,
    scale: f64,
    elevation: bool,
) -> Result<(String, String, Bounds), Error> {
    let mut bounds = Bounds::new();
    let mut defs = String::new();
    let mut body = String::new();
    let mut clip = 0usize;
    for item in items_of(rep) {
        let role = match item.role.as_deref() {
            Some("swing") if elevation => Some("opening"),
            r => r,
        };
        let p = pen(item.class, role);
        let geoms = ev
            .items(&item.geometry)
            .map_err(|e| Error::Recipe(e.to_string()))?;
        for g in &geoms {
            match &g.shape {
                Shape::Region { shape, .. } => {
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
                    region_svg(
                        shape,
                        hatch.as_ref(),
                        &p,
                        scale,
                        &mut defs,
                        &mut body,
                        &mut clip,
                        &mut bounds,
                    );
                }
                Shape::Curve { points, .. } => {
                    points.iter().for_each(|q| bounds.add(*q));
                    if p.dash.is_empty() {
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
                    } else {
                        dashed_polyline(points, &p, scale, &mut body);
                    }
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
    Ok((defs, body, bounds))
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
/// view is drawn with a depth buffer and embedded as an image, so hidden parts stay hidden;
/// glass is drawn see-through.
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
    let (tris, bounds) = view_triangles(def, ev, rep, eye, right, up)?;
    let body = if tris.is_empty() {
        String::new()
    } else {
        raster_image(&tris, &bounds)
    };
    Ok(svg_document(bounds, &body, true))
}

/// Draw an elevation: the 3D representation seen square from the −y side (x to the right, z
/// up), drawn as in [`model_svg`], with the 2D items of the elevation representation, if there
/// is one, drawn over it (opening symbols, outlines). `scale` sets the paper line widths.
pub fn elevation_svg(
    def: &TypeDefinition,
    ev: &Evaluation,
    model_id: &str,
    elevation_id: Option<&str>,
    scale: f64,
) -> Result<String, Error> {
    let rep = representation(def, model_id)?;
    let (tris, mut bounds) = view_triangles(
        def,
        ev,
        rep,
        [0.0, -1.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
    )?;
    let mut overlay = String::new();
    if let Some(id) = elevation_id {
        let (defs, body, b) = draw_2d(def, ev, representation(def, id)?, scale, true)?;
        if !defs.is_empty() {
            writeln!(overlay, "<defs>\n{defs}</defs>").unwrap();
        }
        overlay.push_str(&body);
        bounds.add(b.min);
        bounds.add(b.max);
    }
    let mut all = if tris.is_empty() {
        String::new()
    } else {
        raster_image(&tris, &bounds)
    };
    all.push_str(&overlay);
    Ok(svg_document(bounds, &all, true))
}

/// Every visible triangle of a 3D representation seen along `-eye`, with its screen bounds.
fn view_triangles(
    def: &TypeDefinition,
    ev: &Evaluation,
    rep: &opo_model::Representation,
    eye: V3,
    right: V3,
    up: V3,
) -> Result<(Vec<Tri>, Bounds), Error> {
    let mut parts = Vec::new();
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
            parts.push((
                s.clone(),
                solid_colour(def, item, ev, &g),
                solid_is_glass(def, item, ev, &g),
            ));
        }
    }
    Ok(triangles(&parts, eye, right, up))
}

/// Every visible triangle of a list of solids (with colour and glass flag) seen along `-eye`.
fn triangles(
    parts: &[(opo_recipe::Solid, String, bool)],
    eye: V3,
    right: V3,
    up: V3,
) -> (Vec<Tri>, Bounds) {
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
    let mut tris: Vec<Tri> = Vec::new();
    let mut bounds = Bounds::new();
    for (solid_no, (s, colour, glass)) in parts.iter().enumerate() {
        let base = hex(colour);
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
                solid: solid_no as u32 + 1,
                alpha: if *glass { GLASS_ALPHA } else { 1.0 },
            });
        }
    }
    (tris, bounds)
}

/// Views of a host with one hosted object (see [`hosted_view_svg`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostedView {
    /// The shaded 3D view.
    Model,
    /// Square on to the host's outside face (decision 0024).
    Elevation,
    /// A vertical cut across the host through the hosted object's centre, seen along +x.
    Section,
}

/// A host with one hosted object at `x` (proposal 0004) in 3D, in elevation from the host's
/// outside face (decision 0024), or in section across the host through the object. The
/// hosted object's level of detail is that of `hosted_model`; elevation and section follow
/// the level-of-detail table; reveals are shown at high detail; cut regions are filled solid
/// at low detail (mass construction) and hatched otherwise.
#[allow(clippy::too_many_arguments)]
pub fn hosted_view_svg(
    host_def: &TypeDefinition,
    host_ev: &Evaluation,
    layer_step: &str,
    hosted_def: &TypeDefinition,
    hosted_ev: &Evaluation,
    hosted_model: &str,
    x: f64,
    view: HostedView,
    scale: f64,
) -> Result<String, Error> {
    use opo_model::Lod;
    let lod = representation(hosted_def, hosted_model)?.lod;
    let table = opo_model::LodTable::default();
    let drawing = match view {
        HostedView::Model => None,
        _ => Some((opo_model::DrawingView::SectionElevation, &table)),
    };
    let parts = opo_recipe::host::hosted_model(
        host_def,
        host_ev,
        layer_step,
        hosted_def,
        hosted_ev,
        hosted_model,
        x,
        matches!(lod, Lod::High | Lod::All),
        drawing,
    )
    .map_err(|e| Error::Recipe(e.to_string()))?;
    let def_of = |p: &opo_recipe::host::ModelPart| if p.from_host { host_def } else { hosted_def };
    let colour = |p: &opo_recipe::host::ModelPart| {
        p.material
            .as_ref()
            .and_then(|m| {
                def_of(p)
                    .materials
                    .iter()
                    .flatten()
                    .find(|(k, _)| *k == *m)
                    .and_then(|(_, mat)| mat.colour.clone())
            })
            .unwrap_or_else(|| "#bbbbbb".into())
    };
    let glass = |p: &opo_recipe::host::ModelPart| {
        p.material.as_ref().is_some_and(|m| {
            def_of(p).materials.iter().flatten().any(|(k, mat)| {
                k == m && matches!(mat.category, opo_model::MaterialCategory::Glass)
            })
        })
    };
    match view {
        HostedView::Model | HostedView::Elevation => {
            let list: Vec<_> = parts
                .iter()
                .map(|p| (p.solid.clone(), colour(p), glass(p)))
                .collect();
            let (eye, right, up) = if view == HostedView::Model {
                let (elev, azim) = (24f64.to_radians(), (-58f64).to_radians());
                (
                    [elev.cos() * azim.cos(), elev.cos() * azim.sin(), elev.sin()],
                    [-azim.sin(), azim.cos(), 0.0],
                    [
                        -elev.sin() * azim.cos(),
                        -elev.sin() * azim.sin(),
                        elev.cos(),
                    ],
                )
            } else {
                let host = opo_recipe::host::provides(host_def, host_ev)
                    .map_err(|e| Error::Recipe(e.to_string()))?;
                if host.get("exterior").and_then(|v| v.as_str()) == Some("back") {
                    ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0])
                } else {
                    ([0.0, 1.0, 0.0], [-1.0, 0.0, 0.0], [0.0, 0.0, 1.0])
                }
            };
            let (tris, bounds) = triangles(&list, eye, right, up);
            let body = if tris.is_empty() {
                String::new()
            } else {
                raster_image(&tris, &bounds)
            };
            Ok(svg_document(bounds, &body, true))
        }
        HostedView::Section => {
            let h = -x;
            let turned: Vec<_> = parts
                .iter()
                .map(|p| (opo_recipe::host::for_x_section(&p.solid), p))
                .collect();
            let (mut defs, mut body, mut bounds) = (String::new(), String::new(), Bounds::new());
            let refs: Vec<&opo_recipe::Solid> = turned.iter().map(|(s, _)| s).collect();
            let pp = pen(Some(LineClass::Projected), None);
            let mut path = String::new();
            for [a, b] in opo_recipe::derive::visible_edges_below(&refs, h) {
                bounds.add(a);
                bounds.add(b);
                write!(path, "M{} {}L{} {}", n(a[0]), n(-a[1]), n(b[0]), n(-b[1])).unwrap();
            }
            if !path.is_empty() {
                writeln!(body, "<path d=\"{path}\" {}/>", stroke(&pp, scale)).unwrap();
            }
            let p = pen(Some(LineClass::Cut), None);
            let mut clip = 9000usize;
            let poche = lod == Lod::Low;
            // At low detail the host's layers are one region with its core's material.
            let mut cuts: Vec<(opo_recipe::MultiPolygon<f64>, &opo_recipe::host::ModelPart)> =
                Vec::new();
            let mut host_cut = opo_recipe::MultiPolygon::<f64>(Vec::new());
            let mut core: Option<&opo_recipe::host::ModelPart> = None;
            for (s, part) in &turned {
                let shape = opo_recipe::derive::section(s, h);
                if shape.0.is_empty() {
                    continue;
                }
                if poche && part.from_host {
                    use opo_recipe::BooleanOps;
                    host_cut = host_cut.union(&shape);
                    if core.is_none() || material_look(host_def, part.material.as_deref()).1 {
                        core = Some(part);
                    }
                } else {
                    cuts.push((shape, part));
                }
            }
            if let Some(c) = core {
                cuts.insert(0, (host_cut, c));
            }
            for (shape, part) in &cuts {
                let shape = shape.clone();
                let (hatch, mass) = material_look(def_of(part), part.material.as_deref());
                region_fill(
                    &shape,
                    &mut body,
                    if poche && mass { POCHE } else { "#ffffff" },
                );
                region_svg(
                    &shape,
                    if poche { None } else { hatch.as_ref() },
                    &p,
                    scale,
                    &mut defs,
                    &mut body,
                    &mut clip,
                    &mut bounds,
                );
            }
            let mut all = String::new();
            if !defs.is_empty() {
                writeln!(all, "<defs>\n{defs}</defs>").unwrap();
            }
            all.push_str(&body);
            Ok(svg_document(bounds, &all, true))
        }
    }
}

/// Opacity of glass in the 3D views.
const GLASS_ALPHA: f64 = 0.35;

/// True if a geometry's material (the item's, or the one a nested object gave it) is glass.
fn solid_is_glass(
    def: &TypeDefinition,
    item: &RepresentationItem,
    ev: &Evaluation,
    g: &Geometry,
) -> bool {
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
            .find(|(k, _)| **k == m)
            .map(|(_, mat)| mat.category == opo_model::MaterialCategory::Glass)
    })
    .unwrap_or(false)
}

/// One visible triangle of the 3D view.
struct Tri {
    screen: [[f64; 2]; 3],
    /// Distance towards the eye at each corner: larger is nearer.
    depth: [f64; 3],
    rgb: [u8; 3],
    solid: u32,
    /// 1 for opaque parts; less for glass, drawn over the opaque parts.
    alpha: f64,
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
    // Colour and coverage per pixel, so glass can be blended over what is behind it.
    let mut col = vec![[0f64; 3]; w * h];
    let mut cover = vec![0f64; w * h];
    let mut glass_id = vec![0u32; w * h];
    let to_px = |t: &Tri| {
        t.screen
            .map(|q| [(q[0] - b.min[0]) * scale, (b.max[1] - q[1]) * scale])
    };
    // Visit the pixels a triangle covers, with the depth there.
    let cover_tri = |t: &Tri, f: &mut dyn FnMut(usize, f64)| {
        let p = to_px(t);
        let area =
            (p[1][0] - p[0][0]) * (p[2][1] - p[0][1]) - (p[2][0] - p[0][0]) * (p[1][1] - p[0][1]);
        if area.abs() < 1e-12 {
            return;
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
                f(
                    y * w + x,
                    l0 * t.depth[0] + l1 * t.depth[1] + l2 * t.depth[2],
                );
            }
        }
    };
    // Opaque parts first, with the depth buffer.
    for t in tris.iter().filter(|t| t.alpha >= 1.0) {
        let c = t.rgb.map(f64::from);
        cover_tri(t, &mut |k, z| {
            // Ties go to the part drawn first, so touching faces don't flicker.
            if z > depth[k] + 1e-7 {
                depth[k] = z;
                id[k] = t.solid;
                col[k] = c;
                cover[k] = 1.0;
            }
        });
    }
    // Then glass, far to near, blended over whatever is behind it.
    let mut glass: Vec<&Tri> = tris.iter().filter(|t| t.alpha < 1.0).collect();
    glass.sort_by(|p, q| {
        let d = |t: &Tri| t.depth.iter().sum::<f64>();
        d(p).total_cmp(&d(q))
    });
    for t in glass {
        let c = t.rgb.map(f64::from);
        let a = t.alpha;
        cover_tri(t, &mut |k, z| {
            if z > depth[k] + 1e-7 {
                for i in 0..3 {
                    col[k][i] = if cover[k] > 0.0 {
                        (a * c[i] + (1.0 - a) * cover[k] * col[k][i]) / (a + (1.0 - a) * cover[k])
                    } else {
                        c[i]
                    };
                }
                cover[k] = a + (1.0 - a) * cover[k];
                glass_id[k] = t.solid;
            }
        });
    }
    // Outline where one part meets another or the background.
    let edge = |k: usize, x: usize| {
        let (me, g) = (id[k], glass_id[k]);
        (x + 1 < w && (id[k + 1] != me || glass_id[k + 1] != g))
            || (k + w < w * h && (id[k + w] != me || glass_id[k + w] != g))
    };
    let (ow, oh) = (w.div_ceil(ss), h.div_ceil(ss));
    let mut out = vec![0u8; ow * oh * 4];
    for oy in 0..oh {
        for ox in 0..ow {
            let (mut acc, mut alpha) = ([0f64; 3], 0f64);
            for dy in 0..ss {
                for dx in 0..ss {
                    let (x, y) = (ox * ss + dx, oy * ss + dy);
                    if x >= w || y >= h {
                        continue;
                    }
                    let k = y * w + x;
                    let (c, a) = if edge(k, x) {
                        ([40.0, 40.0, 40.0], 1.0)
                    } else {
                        (col[k], cover[k])
                    };
                    for i in 0..3 {
                        acc[i] += c[i] * a;
                    }
                    alpha += a;
                }
            }
            let o = (oy * ow + ox) * 4;
            if alpha > 0.0 {
                for i in 0..3 {
                    out[o + i] = (acc[i] / alpha).round() as u8;
                }
                out[o + 3] = (255.0 * alpha / (ss * ss) as f64).round() as u8;
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
            alpha: 1.0,
        };
        let mut b = Bounds::new();
        b.add([0.0, 0.0]);
        b.add([1.0, 1.0]);
        // The near red triangle comes first; the far blue one must not cover it.
        let svg = raster_image(&[tri(1.0, [255, 0, 0], 1), tri(-1.0, [0, 0, 255], 2)], &b);
        let (buf, w, h) = decode(&svg);
        let k = ((h * 3 / 4) * w + w / 4) * 4;
        assert_eq!(&buf[k..k + 4], &[255, 0, 0, 255], "red in front");
        let corner = ((h / 8) * w + w * 7 / 8) * 4;
        assert_eq!(buf[corner + 3], 0, "empty outside the triangle");
    }

    #[test]
    fn glass_lets_what_is_behind_show_through() {
        let tri = |depth: f64, rgb: [u8; 3], solid: u32, alpha: f64| Tri {
            screen: [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            depth: [depth; 3],
            rgb,
            solid,
            alpha,
        };
        let mut b = Bounds::new();
        b.add([0.0, 0.0]);
        b.add([1.0, 1.0]);
        let svg = raster_image(
            &[
                tri(1.0, [255, 0, 0], 1, GLASS_ALPHA),
                tri(-1.0, [0, 0, 255], 2, 1.0),
            ],
            &b,
        );
        let (buf, w, h) = decode(&svg);
        let k = ((h * 3 / 4) * w + w / 4) * 4;
        let px = &buf[k..k + 4];
        assert!(px[0] > 40 && px[0] < 140, "some red from the glass: {px:?}");
        assert!(px[2] > 120, "the blue behind shows through: {px:?}");
        assert_eq!(px[3], 255);
    }

    fn decode(svg: &str) -> (Vec<u8>, usize, usize) {
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
        (buf, info.width as usize, info.height as usize)
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
