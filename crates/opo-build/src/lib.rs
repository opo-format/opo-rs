//! Build steps for OPO objects.
//!
//! - [`bake`] runs an object's recipe at its defaults and stores the result as the baked
//!   fallback (spec §4.13, §5.1): `usd::usdgeom::mesh` nodes for the solids of `model.medium`,
//!   `usd::usdgeom::basiscurves` nodes for `plan.medium`, and hidden `IfcOpeningElement`
//!   meshes for the void of a hosted object (`opo::host.void`), as children of the type node.
//! - [`pack`] and [`unpack`] convert between an unpacked object folder and a `.opo` zip
//!   package (spec §5.2, decision 0004).

use std::collections::HashSet;
use std::fmt;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use opo_feel::Value;
use opo_ifcx::{Document, Node, compose};
use opo_model::TypeDefinition;
use opo_recipe::{Input, Resolver, Shape, resolve_material, run};
use serde_json::{Map, Value as Json, json};
use uuid::Uuid;

/// Namespace of the deterministic node ids OPO tools generate (UUID v5).
pub const NAMESPACE: Uuid = Uuid::from_bytes([
    0x6f, 0x70, 0x6f, 0x2d, 0x66, 0x6f, 0x72, 0x6d, 0x61, 0x74, 0x2e, 0x6f, 0x72, 0x67, 0x00, 0x00,
]);

/// The representations that are baked.
pub const BAKED: [&str; 2] = ["model.medium", "plan.medium"];

/// An error building an object.
#[derive(Debug)]
pub enum Error {
    /// The document has no type node or its definition does not read.
    Definition(String),
    /// The recipe failed.
    Recipe(opo_recipe::Error),
    /// A file could not be read or written.
    Io(PathBuf, std::io::Error),
    /// A package is not a valid zip or lacks `object.ifcx`.
    Package(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Definition(m) | Error::Package(m) => f.write_str(m),
            Error::Recipe(e) => write!(f, "recipe: {e}"),
            Error::Io(p, e) => write!(f, "{}: {e}", p.display()),
        }
    }
}

impl std::error::Error for Error {}

/// A deterministic node id: UUID v5 of `name` in [`NAMESPACE`].
pub fn node_id(name: &str) -> String {
    Uuid::new_v5(&NAMESPACE, name.as_bytes()).to_string()
}

/// Round to 9 decimals (nanometres) so baked coordinates do not carry float noise.
fn r9(x: f64) -> f64 {
    (x * 1e9).round() / 1e9 + 0.0
}

/// What [`bake`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BakeReport {
    /// Mesh nodes written.
    pub meshes: usize,
    /// Curve nodes written.
    pub curves: usize,
    /// Old baked nodes removed.
    pub removed: usize,
}

fn is_baked(a: &Map<String, Json>) -> bool {
    a.contains_key("usd::usdgeom::mesh") || a.contains_key("usd::usdgeom::basiscurves")
}

/// Bake the fallback geometry of a base document (see the crate documentation). Every
/// existing node with a baked mesh or curves is removed first, with its child links. New
/// nodes get ids `node_id("<base>/baked/<representation>/<item>/<n>")`, where `base` is the
/// object's folder in its repository (e.g. `library/templates/IfcWall`).
pub fn bake(doc: &mut Document, resolver: &dyn Resolver, base: &str) -> Result<BakeReport, Error> {
    let comp = compose(std::slice::from_ref(doc)).map_err(|e| Error::Definition(e.to_string()))?;
    let type_path = comp
        .type_node()
        .ok_or_else(|| Error::Definition("no type node (a node with opo::object)".into()))?
        .to_owned();
    let def = TypeDefinition::from_attributes(&comp.node(&type_path).expect("exists").attributes)
        .map_err(|e| Error::Definition(e.to_string()))?;
    let ev = run(&def, &Input::default(), resolver).map_err(Error::Recipe)?;

    // Remove old baked nodes, group nodes left empty by that, and every link to them.
    let mut old: HashSet<String> = comp
        .paths()
        .iter()
        .filter(|p| **p != type_path && is_baked(&comp.node(p).expect("exists").attributes))
        .cloned()
        .collect();
    for p in comp.paths() {
        let n = comp.node(p).expect("exists");
        let grouping_only = n
            .attributes
            .keys()
            .all(|k| k == "opo::representation" || k == "usd::usdgeom::visibility");
        if *p != type_path
            && !n.children.is_empty()
            && grouping_only
            && n.child_paths().all(|(_, c)| old.contains(c))
        {
            old.insert(p.clone());
        }
    }
    doc.data.retain(|n| !old.contains(&n.path));
    let mut report = BakeReport {
        removed: old.len(),
        ..BakeReport::default()
    };
    for n in doc.data.iter_mut() {
        if let Some(children) = &mut n.children {
            children.retain(|_, v| v.as_str().is_none_or(|p| !old.contains(p)));
            if children.is_empty() {
                n.children = None;
            }
        }
    }

    let mut children = Map::new();
    let mut nodes = Vec::new();
    for rid in BAKED {
        let Some(rep) = def.representation(rid) else {
            continue;
        };
        for (k, item) in rep
            .items
            .iter()
            .flatten()
            .chain(rep.overlay.iter().flatten())
            .enumerate()
        {
            for (j, g) in ev
                .items(&item.geometry)
                .map_err(Error::Recipe)?
                .iter()
                .enumerate()
            {
                let mut a = Map::new();
                a.insert("opo::representation".into(), json!(rid));
                match &g.shape {
                    Shape::Solid(s) => {
                        let points: Vec<Json> = s
                            .points
                            .iter()
                            .map(|p| json!([r9(p[0]), r9(p[1]), r9(p[2])]))
                            .collect();
                        let faces: Vec<usize> = s.faces.iter().flatten().copied().collect();
                        a.insert(
                            "usd::usdgeom::mesh".into(),
                            json!({"points": points, "faceVertexIndices": faces}),
                        );
                        report.meshes += 1;
                    }
                    shape => {
                        let curves = curves_of(shape).map_err(Error::Recipe)?;
                        if curves.is_empty() {
                            continue;
                        }
                        let counts: Vec<usize> = curves.iter().map(Vec::len).collect();
                        let points: Vec<Json> = curves
                            .into_iter()
                            .flatten()
                            .map(|p| json!([r9(p[0]), r9(p[1]), r9(p[2])]))
                            .collect();
                        a.insert(
                            "usd::usdgeom::basiscurves".into(),
                            json!({"points": points, "curveVertexCounts": counts}),
                        );
                        if let Some(c) = item.class {
                            a.insert(
                                "opo::line_class".into(),
                                serde_json::to_value(c).expect("serialises"),
                            );
                        }
                        report.curves += 1;
                    }
                }
                match resolve_material(item, &ev.env, g).map_err(Error::Recipe)? {
                    Some(Value::String(m)) if !m.is_empty() => {
                        a.insert("bsi::ifc::material".into(), json!({"code": m, "uri": ""}));
                    }
                    Some(Value::Null) | None => {}
                    Some(Value::String(_)) => {}
                    Some(other) => {
                        a.insert(
                            "bsi::ifc::material".into(),
                            json!({"code": other.to_json().to_string(), "uri": ""}),
                        );
                    }
                }
                let context = rid.split('.').next().unwrap_or(rid);
                let label = format!(
                    "{}{}_{}_{}",
                    context[..1].to_uppercase(),
                    &context[1..],
                    item.geometry,
                    j + 1
                );
                let path = node_id(&format!("{base}/baked/{rid}/{k}/{j}"));
                children.insert(label, json!(path.clone()));
                nodes.push(Node {
                    path,
                    children: None,
                    inherits: None,
                    attributes: Some(a),
                });
            }
        }
        if let Some(d) = &rep.derive {
            bake_derived(
                &def,
                &ev,
                d,
                rid,
                base,
                &mut children,
                &mut nodes,
                &mut report,
            )?;
        }
    }
    if let Some(n) = doc.data.iter_mut().find(|n| {
        n.path == type_path
            && n.attributes
                .as_ref()
                .is_some_and(|a| a.contains_key("opo::object"))
    }) {
        let c = n.children.get_or_insert_with(Map::new);
        c.extend(children);
    }
    doc.data.extend(nodes);
    Ok(report)
}

/// The curves of a derived plan (spec §4.6): cut outlines at the view's cut height, one node
/// per solid with its material, and the edges seen below the cut in one node.
#[allow(clippy::too_many_arguments)]
fn bake_derived(
    def: &TypeDefinition,
    ev: &opo_recipe::Evaluation,
    d: &opo_model::Derive,
    rid: &str,
    base: &str,
    children: &mut Map<String, Json>,
    nodes: &mut Vec<Node>,
    report: &mut BakeReport,
) -> Result<(), Error> {
    use opo_model::{DeriveMethod, LineClass};
    use opo_recipe::derive::{cut_height, section, source_solids, visible_edges_below};
    let h = cut_height(ev);
    let solids = source_solids(def, ev, d).map_err(Error::Recipe)?;
    let context = rid.split('.').next().unwrap_or(rid);
    let title = format!("{}{}", context[..1].to_uppercase(), &context[1..]);
    let mut push = |label: String,
                    path: String,
                    curves: Vec<Vec<[f64; 3]>>,
                    class: LineClass,
                    mat: Option<&String>| {
        let counts: Vec<usize> = curves.iter().map(Vec::len).collect();
        let points: Vec<Json> = curves
            .into_iter()
            .flatten()
            .map(|p| json!([r9(p[0]), r9(p[1]), r9(p[2])]))
            .collect();
        let mut a = Map::new();
        a.insert("opo::representation".into(), json!(rid));
        a.insert(
            "usd::usdgeom::basiscurves".into(),
            json!({"points": points, "curveVertexCounts": counts}),
        );
        a.insert(
            "opo::line_class".into(),
            serde_json::to_value(class).expect("serialises"),
        );
        if let Some(m) = mat {
            a.insert("bsi::ifc::material".into(), json!({"code": m, "uri": ""}));
        }
        let path = node_id(&path);
        children.insert(label, json!(path.clone()));
        nodes.push(Node {
            path,
            children: None,
            inherits: None,
            attributes: Some(a),
        });
        report.curves += 1;
    };
    if matches!(d.method, DeriveMethod::Cut | DeriveMethod::CutOrProject) {
        for (k, (s, mat)) in solids.iter().enumerate() {
            let shape = section(s, h);
            let rings: Vec<Vec<[f64; 3]>> = shape
                .iter()
                .flat_map(|poly| std::iter::once(poly.exterior()).chain(poly.interiors()))
                .map(|r| r.coords().map(|c| [c.x, c.y, 0.0]).collect())
                .collect();
            if !rings.is_empty() {
                push(
                    format!("{title}_cut_{}", k + 1),
                    format!("{base}/baked/{rid}/cut/{k}"),
                    rings,
                    LineClass::Cut,
                    mat.as_ref(),
                );
            }
        }
    }
    if matches!(d.method, DeriveMethod::Project | DeriveMethod::CutOrProject) {
        let refs: Vec<&opo_recipe::Solid> = solids.iter().map(|(s, _)| s).collect();
        let lines: Vec<Vec<[f64; 3]>> = visible_edges_below(&refs, h)
            .into_iter()
            .map(|[a, b]| vec![[a[0], a[1], 0.0], [b[0], b[1], 0.0]])
            .collect();
        if !lines.is_empty() {
            push(
                format!("{title}_projected"),
                format!("{base}/baked/{rid}/projected"),
                lines,
                LineClass::Projected,
                None,
            );
        }
    }
    Ok(())
}

/// Curves of a 2D or 3D shape in object coordinates: region boundaries (closed rings),
/// polylines and 3D polylines. Text has none.
fn curves_of(shape: &Shape) -> Result<Vec<Vec<[f64; 3]>>, opo_recipe::Error> {
    Ok(match shape {
        Shape::Region { plane, shape } => {
            let mut out = Vec::new();
            for poly in shape {
                for ring in std::iter::once(poly.exterior()).chain(poly.interiors()) {
                    out.push(
                        ring.coords()
                            .map(|c| plane.to_3d(c.x, c.y, 0.0))
                            .collect::<Result<Vec<_>, _>>()?,
                    );
                }
            }
            out
        }
        Shape::Curve { plane, points } => vec![
            points
                .iter()
                .map(|p| plane.to_3d(p[0], p[1], 0.0))
                .collect::<Result<Vec<_>, _>>()?,
        ],
        Shape::Curve3 { points } => vec![points.clone()],
        Shape::Solid(_) | Shape::Text { .. } => Vec::new(),
    })
}

// ------------------------------------------------------------------ packages

/// The files of an unpacked object folder that go into its package: `object.ifcx`,
/// `layers/*.ifcx` and everything under `assets/`, as (path in the package, file).
pub fn package_files(dir: &Path) -> Result<Vec<(String, PathBuf)>, Error> {
    let base = dir.join("object.ifcx");
    if !base.is_file() {
        return Err(Error::Package(format!("{}: no object.ifcx", dir.display())));
    }
    let mut out = vec![("object.ifcx".to_owned(), base)];
    let layers = dir.join("layers");
    if layers.is_dir() {
        let mut l: Vec<PathBuf> = std::fs::read_dir(&layers)
            .map_err(|e| Error::Io(layers.clone(), e))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "ifcx"))
            .collect();
        l.sort();
        out.extend(l.into_iter().map(|p| {
            (
                format!("layers/{}", p.file_name().unwrap().to_string_lossy()),
                p,
            )
        }));
    }
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) -> Result<(), Error> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|e| Error::Io(dir.into(), e))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        entries.sort();
        for p in entries {
            let name = format!("{prefix}/{}", p.file_name().unwrap().to_string_lossy());
            if p.is_dir() {
                walk(&p, &name, out)?;
            } else if p.is_file() {
                out.push((name, p));
            }
        }
        Ok(())
    }
    let assets = dir.join("assets");
    if assets.is_dir() {
        walk(&assets, "assets", &mut out)?;
    }
    Ok(out)
}

/// Write the package of an unpacked object folder. The zip is deterministic: entries in a
/// fixed order, deflated, with a fixed timestamp, so the same folder always gives the same
/// bytes.
pub fn pack(dir: &Path, out: &Path) -> Result<usize, Error> {
    let files = package_files(dir)?;
    let file = std::fs::File::create(out).map_err(|e| Error::Io(out.into(), e))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default())
        .unix_permissions(0o644);
    for (name, path) in &files {
        let bytes = std::fs::read(path).map_err(|e| Error::Io(path.clone(), e))?;
        zip.start_file(name.as_str(), options)
            .map_err(|e| Error::Package(e.to_string()))?;
        zip.write_all(&bytes)
            .map_err(|e| Error::Io(out.into(), e))?;
    }
    zip.finish().map_err(|e| Error::Package(e.to_string()))?;
    Ok(files.len())
}

/// Extract a package into a folder. Entries outside the folder (absolute paths, `..`) are
/// refused; a package without `object.ifcx` is an error.
pub fn unpack(package: &Path, dir: &Path) -> Result<usize, Error> {
    let file = std::fs::File::open(package).map_err(|e| Error::Io(package.into(), e))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| Error::Package(format!("{}: {e}", package.display())))?;
    if zip.by_name("object.ifcx").is_err() {
        return Err(Error::Package(format!(
            "{}: no object.ifcx",
            package.display()
        )));
    }
    let mut n = 0;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|e| Error::Package(e.to_string()))?;
        let Some(rel) = entry.enclosed_name() else {
            return Err(Error::Package(format!(
                "{}: unsafe entry {}",
                package.display(),
                entry.name().map(|n| n.into_owned()).unwrap_or_default()
            )));
        };
        let target = dir.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| Error::Io(target.clone(), e))?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::Io(parent.into(), e))?;
        }
        let mut bytes = Vec::new();
        entry
            .read_to_end(&mut bytes)
            .map_err(|e| Error::Io(package.into(), e))?;
        std::fs::write(&target, bytes).map_err(|e| Error::Io(target.clone(), e))?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_ids_match_the_prototype_namespace() {
        // uuid5(UUID("6f706f2d-666f-726d-6174-2e6f72670000"), "library/templates/IfcWall/type")
        assert_eq!(
            NAMESPACE.to_string(),
            "6f706f2d-666f-726d-6174-2e6f72670000"
        );
        assert_eq!(
            node_id("library/templates/IfcWall/type"),
            "a41e731a-35e8-5a90-b59d-f64bbbcc63eb"
        );
        assert_eq!(node_id("a"), node_id("a"));
        assert_ne!(node_id("a"), node_id("b"));
    }

    #[test]
    fn rounding_removes_float_noise() {
        assert_eq!(r9(0.1 + 0.2), 0.3);
        assert_eq!(r9(-0.0), 0.0);
        assert!(r9(-1e-12).is_sign_positive());
    }
}
