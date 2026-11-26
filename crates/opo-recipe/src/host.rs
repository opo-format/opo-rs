//! The host agreement between a wall and the objects it hosts (proposal 0004, spec §4.7).
//!
//! A host (wall) provides named values ([`provides`]); a hosted object (door, window) reads
//! them as `host.<name>`, says where it seats ([`seat_y`]) and gives back its void and reveal
//! rules. The host applies them to its own drawing ([`cut_host_plan`]). This module covers a
//! straight host along +x with the hosted object at a distance along it, which is what the
//! reference renderer needs; applications with their own geometry engine do the same with it.

use geo::{BooleanOps, MultiPolygon};
use serde_json::{Map, Value as Json, json};

use crate::Error;
use crate::derive::{cut_height, section};
use crate::geometry::Solid;
use crate::run::{Evaluation, resolve_material};
use opo_model::TypeDefinition;

/// The values a host provides at its defaults: every `opo::host.provides` formula evaluated.
pub fn provides(def: &TypeDefinition, ev: &Evaluation) -> Result<Map<String, Json>, Error> {
    let mut out = Map::new();
    for (k, v) in def.host.iter().flat_map(|h| h.provides.iter().flatten()) {
        let expr = v
            .as_str()
            .ok_or_else(|| Error::Op(format!("host.provides.{k} is not a formula")))?;
        out.insert(k.clone(), ev.eval(expr)?.to_json());
    }
    Ok(out)
}

/// A hosted object's `seat` (proposal 0004): the plane of the host its y = 0 lines up with,
/// and an offset formula.
fn seat_of(def: &TypeDefinition) -> (String, Option<String>) {
    let seat = def.host.as_ref().and_then(|h| h.extra.get("seat"));
    let to = seat
        .and_then(|s| s.get("to"))
        .and_then(Json::as_str)
        .unwrap_or("axis")
        .to_owned();
    let offset = seat
        .and_then(|s| s.get("offset"))
        .and_then(Json::as_str)
        .map(str::to_owned);
    (to, offset)
}

/// The y of a named plane of the host (a seat name), from the host's values.
pub fn plane_y(name: &str, host: &Map<String, Json>) -> f64 {
    let g = |k: &str| host.get(k).and_then(Json::as_f64).unwrap_or(0.0);
    match name {
        "face_front" => g("face_front"),
        "face_back" => -g("face_back"),
        "core_front" => g("core_front"),
        "core_back" => -g("core_back"),
        "core_centre" => (g("core_front") - g("core_back")) / 2.0,
        _ => 0.0,
    }
}

/// Where a hosted object's y = 0 sits in the host: the seat plane plus the offset, evaluated
/// in the hosted object's own evaluation (which was run with the host's values).
pub fn seat_y(def: &TypeDefinition, ev: &Evaluation, host: &Map<String, Json>) -> f64 {
    let (to, offset) = seat_of(def);
    let off = offset
        .and_then(|e| ev.eval(&e).ok())
        .and_then(|v| v.to_json().as_f64())
        .unwrap_or(0.0);
    plane_y(&to, host) + off
}

/// The context to run a hosted object with: the host's values under `host`.
pub fn hosted_context(host: &Map<String, Json>) -> Map<String, Json> {
    let mut c = Map::new();
    c.insert("host".into(), Json::Object(host.clone()));
    c
}

/// A reveal rule of a hosted object (proposal 0004).
#[derive(Debug, Clone, PartialEq)]
struct Reveal {
    function: String,
    /// The plane a layer returns to: a fixed y, or the frame's back and front faces (a layer
    /// behind the frame returns to its back face, one in front to its front face).
    to: Target,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Target {
    Plane(f64),
    Frame(f64, f64),
}

fn reveals(
    def: &TypeDefinition,
    ev: &Evaluation,
    host: &Map<String, Json>,
    seat: f64,
) -> Vec<Reveal> {
    let Some(list) = def
        .host
        .as_ref()
        .and_then(|h| h.extra.get("reveal"))
        .and_then(Json::as_array)
    else {
        return Vec::new();
    };
    list.iter()
        .filter(|r| r.get("rule").and_then(Json::as_str) == Some("return"))
        .filter_map(|r| {
            let function = r.get("function")?.as_str()?.to_owned();
            let to = r.get("to").and_then(Json::as_str).unwrap_or("axis");
            let target = if to == "frame" {
                let e = r.get("frame_y")?.as_str()?;
                let d = ev.eval(e).ok()?.to_json().as_f64()?.abs();
                Target::Frame(seat - d, seat + d)
            } else {
                Target::Plane(plane_y(to, host))
            };
            Some(Reveal {
                function,
                to: target,
            })
        })
        .collect()
}

/// Move a solid.
pub fn translated(s: &Solid, d: [f64; 3]) -> Solid {
    Solid {
        points: s
            .points
            .iter()
            .map(|p| [p[0] + d[0], p[1] + d[1], p[2] + d[2]])
            .collect(),
        faces: s.faces.clone(),
    }
}

/// One cut region of a host's plan, with its material key and layer function.
#[derive(Debug, Clone)]
pub struct HostRegion {
    /// The cut shape.
    pub shape: MultiPolygon<f64>,
    /// Material key.
    pub material: Option<String>,
    /// Layer function (`structure`, `finish`…), if known.
    pub function: Option<String>,
}

/// The host's plan cut at the view's cut height with a hosted object at `x` along it: each
/// layer of the host (`opo::host.provides.layers`, in order back to front, matched to the
/// host's layer solids) minus the hosted object's void, and, when `reveals` is true, the
/// layers the object's reveal rules return drawn into the reveal. At `merge`, all layers are
/// one region with the core's material (low detail).
#[allow(clippy::too_many_arguments)]
pub fn cut_host_plan(
    host_def: &TypeDefinition,
    host_ev: &Evaluation,
    layer_step: &str,
    hosted_def: &TypeDefinition,
    hosted_ev: &Evaluation,
    x: f64,
    reveals_on: bool,
    merge: bool,
) -> Result<Vec<HostRegion>, Error> {
    let host = provides(host_def, host_ev)?;
    let h = cut_height(host_ev);
    let seat = seat_y(hosted_def, hosted_ev, &host);
    // The void, cut at the same height, in host coordinates.
    let mut void = MultiPolygon::<f64>(Vec::new());
    let void_step = hosted_def
        .host
        .as_ref()
        .and_then(|hh| hh.void.as_ref())
        .and_then(|v| v.get("geometry"))
        .and_then(Json::as_str)
        .unwrap_or("void");
    for g in hosted_ev.items(void_step)? {
        if let Some(s) = g.solid() {
            void = void.union(&section(&translated(s, [x, seat, 0.0]), h));
        }
    }
    let functions: Vec<Option<String>> = host
        .get("layers")
        .and_then(Json::as_array)
        .map(|ls| {
            ls.iter()
                .map(|l| l.get("function").and_then(Json::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let item = opo_model::RepresentationItem {
        geometry: layer_step.into(),
        material: Some("each.material".into()),
        hatch: None,
        class: None,
        role: None,
        part: None,
        category: None,
    };
    let mut layers = Vec::new();
    for (i, g) in host_ev.items(layer_step)?.iter().enumerate() {
        let Some(s) = g.solid() else { continue };
        let material = match resolve_material(&item, &host_ev.env, g)? {
            Some(opo_feel::Value::String(k)) if !k.is_empty() => Some(k),
            _ => None,
        };
        let shape = section(s, h);
        layers.push(HostRegion {
            shape,
            material,
            function: functions.get(i).cloned().flatten(),
        });
    }
    // The void's sides along x, for reveal returns.
    let (vx0, vx1) = void
        .0
        .iter()
        .flat_map(|p| p.exterior().coords())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), c| {
            (a.min(c.x), b.max(c.x))
        });
    let rules = if reveals_on {
        reveals(hosted_def, hosted_ev, &host, seat)
    } else {
        Vec::new()
    };
    let mut out = Vec::new();
    for l in &layers {
        let mut shape = l.shape.difference(&void);
        if vx0 < vx1
            && let Some(r) = rules
                .iter()
                .find(|r| Some(&r.function) == l.function.as_ref())
        {
            let (ly0, ly1) = l
                .shape
                .0
                .iter()
                .flat_map(|p| p.exterior().coords())
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), c| {
                    (a.min(c.y), b.max(c.y))
                });
            let t = ly1 - ly0;
            let to_y = match r.to {
                Target::Plane(y) => y,
                Target::Frame(_, front) if ly0 >= front - 1e-9 => front,
                Target::Frame(back, _) => back,
            };
            let span = if to_y < ly0 - 1e-9 {
                Some((to_y, ly0))
            } else if to_y > ly1 + 1e-9 {
                Some((ly1, to_y))
            } else {
                None
            };
            if let Some((y0, y1)) = span {
                for (a, b) in [(vx0, vx0 + t), (vx1 - t, vx1)] {
                    let rect = rectangle(a, y0, b, y1);
                    shape = shape.union(&rect);
                }
            }
        }
        out.push(HostRegion {
            shape,
            material: l.material.clone(),
            function: l.function.clone(),
        });
    }
    if merge {
        let core = out
            .iter()
            .find(|r| r.function.as_deref() == Some("structure"))
            .and_then(|r| r.material.clone());
        let shape = out
            .iter()
            .fold(MultiPolygon(Vec::new()), |acc, r| acc.union(&r.shape));
        return Ok(vec![HostRegion {
            shape,
            material: core,
            function: Some("structure".into()),
        }]);
    }
    Ok(out)
}

/// A solid of a host-and-hosted model, with its material key.
#[derive(Debug, Clone)]
pub struct ModelPart {
    /// The solid, in host coordinates.
    pub solid: Solid,
    /// Material key.
    pub material: Option<String>,
    /// True for the host's pieces, false for the hosted object's.
    pub from_host: bool,
}

fn prism(shape: &MultiPolygon<f64>, lo: f64, hi: f64) -> Result<Vec<Solid>, Error> {
    if shape.0.is_empty() || hi - lo < 1e-9 {
        return Ok(Vec::new());
    }
    let region = crate::geometry::Geometry::new(crate::geometry::Shape::Region {
        plane: crate::geometry::Plane::Xy,
        shape: shape.clone(),
    })
    .into_value();
    let v = crate::ops::extrude(&region, lo, hi)?;
    Ok(crate::geometry::flatten(&v)?
        .iter()
        .filter_map(|g| g.solid().cloned())
        .collect())
}

/// The 3D model of a host with one hosted object at `x` (proposal 0004), for views of the
/// pair: each host layer split around the hosted object's void (left, right, below and
/// above it), the layers its reveal rules return lining the void's sides when `reveals_on`,
/// and the hosted object's solids of `hosted_model` moved onto their seat. Straight hosts
/// along +x with box-shaped voids; applications with a solid modeller subtract the void.
/// With `drawing`, the hosted object's parts follow the level-of-detail table for that view.
#[allow(clippy::too_many_arguments)]
pub fn hosted_model(
    host_def: &TypeDefinition,
    host_ev: &Evaluation,
    layer_step: &str,
    hosted_def: &TypeDefinition,
    hosted_ev: &Evaluation,
    hosted_model: &str,
    x: f64,
    reveals_on: bool,
    drawing: Option<(opo_model::DrawingView, &opo_model::LodTable)>,
) -> Result<Vec<ModelPart>, Error> {
    let host = provides(host_def, host_ev)?;
    let seat = seat_y(hosted_def, hosted_ev, &host);
    let void_step = hosted_def
        .host
        .as_ref()
        .and_then(|hh| hh.void.as_ref())
        .and_then(|v| v.get("geometry"))
        .and_then(Json::as_str)
        .unwrap_or("void");
    let (mut v0, mut v1) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for g in hosted_ev.items(void_step)? {
        if let Some(s) = g.solid() {
            for p in &translated(s, [x, seat, 0.0]).points {
                for k in 0..3 {
                    v0[k] = v0[k].min(p[k]);
                    v1[k] = v1[k].max(p[k]);
                }
            }
        }
    }
    let rules = if reveals_on {
        reveals(hosted_def, hosted_ev, &host, seat)
    } else {
        Vec::new()
    };
    let functions: Vec<Option<String>> = host
        .get("layers")
        .and_then(Json::as_array)
        .map(|ls| {
            ls.iter()
                .map(|l| l.get("function").and_then(Json::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let item = opo_model::RepresentationItem {
        geometry: layer_step.into(),
        material: Some("each.material".into()),
        hatch: None,
        class: None,
        role: None,
        part: None,
        category: None,
    };
    let mut out = Vec::new();
    for (i, g) in host_ev.items(layer_step)?.iter().enumerate() {
        let Some(s) = g.solid() else { continue };
        let material = match resolve_material(&item, &host_ev.env, g)? {
            Some(opo_feel::Value::String(k)) if !k.is_empty() => Some(k),
            _ => None,
        };
        let (zb, zt) = s
            .points
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| {
                (a.min(p[2]), b.max(p[2]))
            });
        let plan = section(s, (zb + zt) / 2.0);
        let ext = |f: fn(&geo::Coord<f64>) -> f64| {
            plan.0
                .iter()
                .flat_map(|p| p.exterior().coords())
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), c| {
                    (a.min(f(c)), b.max(f(c)))
                })
        };
        let ((lx0, lx1), (ly0, ly1)) = (ext(|c| c.x), ext(|c| c.y));
        // The layer's face in elevation (x, z) minus the void, made thick along y: one solid
        // per layer, so no seams show.
        let mut face = rectangle(lx0, zb, lx1, zt);
        if v0[0] < v1[0] {
            face = face.difference(&rectangle(v0[0], v0[2], v1[0], v1[2]));
        }
        let mut solids = Vec::new();
        if !face.0.is_empty() && ly1 - ly0 > 1e-9 {
            let region = crate::geometry::Geometry::new(crate::geometry::Shape::Region {
                plane: crate::geometry::Plane::Xz,
                shape: face,
            })
            .into_value();
            let v = crate::ops::extrude(&region, ly0, ly1)?;
            solids.extend(
                crate::geometry::flatten(&v)?
                    .iter()
                    .filter_map(|g| g.solid().cloned()),
            );
        }
        if v0[0] < v1[0]
            && let Some(r) = rules
                .iter()
                .find(|r| Some(&r.function) == functions.get(i).and_then(|f| f.as_ref()))
        {
            let t = ly1 - ly0;
            let to_y = match r.to {
                Target::Plane(y) => y,
                Target::Frame(_, front) if ly0 >= front - 1e-9 => front,
                Target::Frame(back, _) => back,
            };
            let span = if to_y < ly0 - 1e-9 {
                Some((to_y, ly0))
            } else if to_y > ly1 + 1e-9 {
                Some((ly1, to_y))
            } else {
                None
            };
            if let Some((y0, y1)) = span {
                for (a, b) in [(v0[0], v0[0] + t), (v1[0] - t, v1[0])] {
                    solids.extend(prism(
                        &rectangle(a, y0, b, y1),
                        v0[2].max(zb),
                        v1[2].min(zt),
                    )?);
                }
            }
        }
        for solid in solids {
            out.push(ModelPart {
                solid,
                material: material.clone(),
                from_host: true,
            });
        }
    }
    let rep = hosted_def
        .representation(hosted_model)
        .ok_or_else(|| Error::Op(format!("no representation {hosted_model}")))?;
    for item in rep.items.iter().flatten() {
        let hidden = drawing.is_some_and(|(v, t)| {
            t.resolve(item.category, v, rep.lod, None) != opo_model::Visibility::Show
        });
        if hidden || item.role.as_deref() == Some("void") {
            continue;
        }
        for g in hosted_ev.items(&item.geometry)? {
            let Some(s) = g.solid() else { continue };
            let material = match resolve_material(item, &hosted_ev.env, &g)? {
                Some(opo_feel::Value::String(k)) if !k.is_empty() => Some(k),
                _ => None,
            };
            out.push(ModelPart {
                solid: translated(s, [x, seat, 0.0]),
                material,
                from_host: false,
            });
        }
    }
    Ok(out)
}

/// Turn a solid so that a horizontal cut at height `h` of the result is the vertical cut at
/// x = −`h` of the original, seen looking towards +x: (x, y, z) becomes (−y, z, −x).
pub fn for_x_section(s: &Solid) -> Solid {
    Solid::new(
        s.points.iter().map(|p| [-p[1], p[2], -p[0]]).collect(),
        s.faces.clone(),
    )
}

fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64) -> MultiPolygon<f64> {
    use geo::{LineString, Polygon};
    MultiPolygon(vec![Polygon::new(
        LineString::from(vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]),
        vec![],
    )])
}

/// The default host values (conformance/README.md): a 0.2 m wall of one structure layer on
/// its axis, 3 m high, internal.
pub fn default_host() -> Json {
    json!({
        "thickness": 0.2, "face_front": 0.1, "face_back": 0.1, "exterior": "none",
        "layers": [{"thickness": 0.2, "material": null, "function": "structure"}],
        "core_front": 0.1, "core_back": 0.1, "base": 0.0, "height": 3.0,
        "y_min": -0.1, "y_max": 0.1, "bottom": 0.0, "top": 3.0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wall() -> Map<String, Json> {
        json!({"face_front": 0.15, "face_back": 0.1, "core_front": 0.12, "core_back": 0.03})
            .as_object()
            .unwrap()
            .clone()
    }

    #[test]
    fn seat_planes_are_measured_from_the_location_line() {
        let h = wall();
        assert_eq!(plane_y("axis", &h), 0.0);
        assert_eq!(plane_y("face_front", &h), 0.15);
        assert_eq!(plane_y("face_back", &h), -0.1);
        assert_eq!(plane_y("core_front", &h), 0.12);
        assert_eq!(plane_y("core_back", &h), -0.03);
        assert!((plane_y("core_centre", &h) - 0.045).abs() < 1e-12);
    }

    #[test]
    fn hosted_objects_get_the_host_values_as_context() {
        let c = hosted_context(&wall());
        assert_eq!(c["host"]["face_front"], json!(0.15));
        assert_eq!(default_host()["exterior"], json!("none"));
    }

    #[test]
    fn a_moved_solid_keeps_its_faces() {
        let s = Solid {
            points: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            faces: vec![[0, 1, 2]],
        };
        let t = translated(&s, [1.0, 2.0, 3.0]);
        assert_eq!(t.points[1], [2.0, 2.0, 3.0]);
        assert_eq!(t.faces, s.faces);
    }
}
