//! Object source → IFCX documents (proposal 0002 §1, §2, §5).

use std::path::Path;

use opo_build::node_id;
use opo_feel::Value;
use opo_ifcx::{Document, Header, Import, Node};
use opo_model::TypeDefinition;
use opo_recipe::{Input, Resolver, run};
use serde_json::{Map, Value as Json, json};

use crate::{Error, Library, read_yaml};

/// The schema documents every OPO file imports (spec §5).
pub const IMPORTS: [&str; 4] = [
    "https://ifcx.dev/@standards.buildingsmart.org/ifc/core/ifc@v5a.ifcx",
    "https://ifcx.dev/@standards.buildingsmart.org/ifc/core/prop@v5a.ifcx",
    "https://ifcx.dev/@openusd.org/usd@v1.ifcx",
    "https://opo-format.org/schema/0.1/opo@v0.1.ifcx",
];

/// The compiled documents of one object (without baked geometry).
#[derive(Debug, Clone, PartialEq)]
pub struct Compiled {
    /// `object.ifcx`.
    pub base: Document,
    /// `layers/<name>.ifcx`, in source order, with their file names.
    pub layers: Vec<(String, Document)>,
}

fn err(msg: impl Into<String>) -> Error {
    Error::Source(msg.into())
}

/// Read `object.yaml` and `layers/*.yaml` of a source folder (layers in file-name order).
pub fn read_source_dir(dir: &Path) -> Result<(Json, Vec<(String, Json)>), Error> {
    let object = read_yaml(&dir.join("object.yaml"))?
        .ok_or_else(|| err(format!("{}: no object.yaml", dir.display())))?;
    let mut layers = Vec::new();
    let ldir = dir.join("layers");
    if ldir.is_dir() {
        let mut files: Vec<_> = std::fs::read_dir(&ldir)
            .map_err(|e| Error::Io(ldir.clone(), e))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
            .collect();
        files.sort();
        for f in files {
            let stem = f.file_stem().unwrap().to_string_lossy().into_owned();
            layers.push((stem, read_yaml(&f)?.unwrap_or(Json::Null)));
        }
    }
    Ok((object, layers))
}

/// Compile a source folder (see [`compile`]).
pub fn compile_dir(
    dir: &Path,
    lib: &Library,
    base: &str,
    resolver: &dyn Resolver,
    timestamp: &str,
) -> Result<Compiled, Error> {
    let (object, layers) = read_source_dir(dir)?;
    if object.get("status").and_then(Json::as_str) == Some("planned") {
        return Err(Error::Source(format!(
            "{}: a planned object is a catalogue entry and is not built",
            dir.display()
        )));
    }
    compile(&object, &layers, lib, base, resolver, timestamp)
}

// ------------------------------------------------------------------ provenance

fn is_provenance(m: &Map<String, Json>) -> bool {
    m.contains_key("value")
        && m.contains_key("status")
        && m.keys()
            .all(|k| ["value", "source", "status", "note"].contains(&k.as_str()))
}

/// Replace provenance objects by their value and record `path → {source, status, note}`.
fn strip(v: &mut Json, path: &str, prov: &mut Map<String, Json>) -> Result<(), Error> {
    let join = |k: &str| {
        if path.is_empty() {
            k.to_owned()
        } else {
            format!("{path}.{k}")
        }
    };
    match v {
        Json::Object(m) if is_provenance(m) => {
            let status = m
                .get("status")
                .and_then(Json::as_str)
                .unwrap_or_default()
                .to_owned();
            if !["norm", "maker", "typical", "illustrative", "to_verify"].contains(&status.as_str())
            {
                return Err(err(format!("{path}: unknown status '{status}'")));
            }
            if ["norm", "maker"].contains(&status.as_str()) && !m.contains_key("source") {
                return Err(err(format!("{path}: a {status} value needs a source")));
            }
            if ["typical", "illustrative", "to_verify"].contains(&status.as_str())
                && !m.contains_key("note")
            {
                return Err(err(format!("{path}: a {status} value needs a note")));
            }
            let value = m.remove("value").expect("checked");
            prov.insert(path.to_owned(), Json::Object(std::mem::take(m)));
            *v = value;
        }
        Json::Object(m) => {
            for (k, x) in m.iter_mut() {
                strip(x, &join(k), prov)?;
            }
        }
        Json::Array(a) => {
            for (i, x) in a.iter_mut().enumerate() {
                strip(x, &join(&i.to_string()), prov)?;
            }
        }
        _ => {}
    }
    Ok(())
}

// ------------------------------------------------------------------ helpers

fn header(base: &str, rel: &str, version: &str, timestamp: &str) -> Header {
    Header {
        id: format!("opo-format.org/{base}/{rel}"),
        ifcx_version: "ifcx_alpha".into(),
        data_version: version.into(),
        author: "opo-format.org".into(),
        timestamp: timestamp.into(),
        extra: Map::new(),
    }
}

fn document(base: &str, rel: &str, version: &str, timestamp: &str, data: Vec<Node>) -> Document {
    Document {
        header: header(base, rel, version, timestamp),
        imports: IMPORTS
            .iter()
            .map(|u| Import {
                uri: (*u).into(),
                extra: Map::new(),
            })
            .collect(),
        schemas: Map::new(),
        data,
    }
}

/// A named map (`{name: {...}}`) as a list with the key under `field` first.
fn named_list(v: Option<&Json>, field: &str, what: &str) -> Result<Option<Json>, Error> {
    let Some(v) = v else { return Ok(None) };
    let m = v
        .as_object()
        .ok_or_else(|| err(format!("{what} must be a map by {field}")))?;
    let mut out = Vec::new();
    for (k, x) in m {
        let body = x
            .as_object()
            .ok_or_else(|| err(format!("{what}.{k} must be a map")))?;
        if body.contains_key(field) {
            return Err(err(format!(
                "{what}.{k}: the {field} is the key; do not repeat it"
            )));
        }
        let mut e = Map::new();
        e.insert(field.into(), json!(k));
        e.extend(body.clone());
        out.push(Json::Object(e));
    }
    Ok(Some(Json::Array(out)))
}

fn class_uri(class: &str) -> String {
    format!("https://identifier.buildingsmart.org/uri/buildingsmart/ifc/4.3/class/{class}")
}

fn r9(x: f64) -> f64 {
    (x * 1e9).round() / 1e9 + 0.0
}

/// Plain IFC values (`bsi::ifc::prop::<Attribute>`) from the `opo::ifc.properties` formulas
/// with those keys, evaluated for some values.
fn plain_ifc_values(
    def: &TypeDefinition,
    values: &Map<String, Json>,
    resolver: &dyn Resolver,
) -> Result<Map<String, Json>, Error> {
    let keys: Vec<(&String, &Json)> = def
        .ifc
        .properties
        .iter()
        .flatten()
        .filter(|(k, _)| k.starts_with("bsi::ifc::prop::"))
        .collect();
    let mut out = Map::new();
    if keys.is_empty() {
        return Ok(out);
    }
    let input = Input {
        values: values.clone(),
        ..Input::default()
    };
    let ev = run(def, &input, resolver).map_err(|e| Error::Recipe(e.to_string()))?;
    for (k, expr) in keys {
        let v = match expr {
            Json::String(e) => ev.eval(e).map_err(|e| Error::Recipe(e.to_string()))?,
            lit => Value::from_json(lit),
        };
        let j = match v {
            Value::Number(x) => json!(r9(x)),
            Value::Null => continue,
            other => other.to_json(),
        };
        out.insert(k.clone(), j);
    }
    Ok(out)
}

// ------------------------------------------------------------------ the type node

fn object_attribute(src: &Map<String, Json>, lib: &Library, base: &str) -> Result<Json, Error> {
    let s = |k: &str| src.get(k).and_then(Json::as_str);
    let mut o = Map::new();
    o.insert("id".into(), json!(format!("opo-format.org/{base}")));
    o.insert(
        "name".into(),
        json!(s("name").ok_or_else(|| err("object.yaml: name is required"))?),
    );
    for k in ["kind", "archetype"] {
        if let Some(v) = src.get(k) {
            o.insert(k.into(), v.clone());
        }
    }
    o.insert(
        "version".into(),
        json!(s("version").ok_or_else(|| err("object.yaml: version is required"))?),
    );
    o.insert("opo".into(), json!("0.1"));
    o.insert("license".into(), json!(s("license").unwrap_or("CC0-1.0")));
    let authors: Vec<&str> = src
        .get("authors")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
        .filter_map(Json::as_str)
        .collect();
    o.insert(
        "author".into(),
        json!(if authors.is_empty() {
            "OPO format contributors".to_owned()
        } else {
            authors.join(", ")
        }),
    );
    o.insert(
        "description".into(),
        json!(s("description").ok_or_else(|| err("object.yaml: description is required"))?),
    );
    if let Some(t) = src.get("tags") {
        o.insert("tags".into(), t.clone());
    }
    let mut refs = Vec::new();
    for r in src
        .get("references")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
    {
        let id = r
            .get("ref")
            .and_then(Json::as_str)
            .ok_or_else(|| err("references: every entry needs ref"))?;
        let reg = lib
            .references
            .get(id)
            .ok_or_else(|| err(format!("references: '{id}' is not in references.yaml")))?;
        let mut e = Map::new();
        e.insert(
            "role".into(),
            r.get("role")
                .cloned()
                .ok_or_else(|| err(format!("references.{id}: role is required")))?,
        );
        e.insert(
            "name".into(),
            reg.get("title")
                .cloned()
                .ok_or_else(|| err(format!("references.yaml: {id} has no title")))?,
        );
        e.insert(
            "used".into(),
            r.get("used")
                .cloned()
                .ok_or_else(|| err(format!("references.{id}: used is required")))?,
        );
        for (from, to) in [
            ("licence", "license"),
            ("url", "uri"),
            ("version", "version"),
        ] {
            if let Some(v) = reg.get(from) {
                e.insert(to.into(), v.clone());
            }
        }
        refs.push(Json::Object(e));
    }
    o.insert("references".into(), Json::Array(refs));
    Ok(Json::Object(o))
}

/// Material ids an object uses: listed ones first, then those in material parameters,
/// layer build-ups and layer values, in order of appearance.
fn material_ids(src: &Map<String, Json>, layers: &[(String, Json)]) -> Vec<String> {
    let mut ids: Vec<String> = src
        .get("materials")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let mut add = |id: &str| {
        if !ids.iter().any(|x| x == id) {
            ids.push(id.to_owned());
        }
    };
    let from_value = |v: &Json, add: &mut dyn FnMut(&str)| {
        if let Json::Array(items) = v {
            for l in items {
                if let Some(m) = l.get("material").and_then(Json::as_str) {
                    add(m);
                }
            }
        }
    };
    for (_, p) in src
        .get("parameters")
        .and_then(Json::as_object)
        .into_iter()
        .flatten()
    {
        let d = p.get("default").unwrap_or(&Json::Null);
        if p.get("type").and_then(Json::as_str) == Some("material") {
            if let Some(m) = d.as_str() {
                add(m);
            }
        }
        from_value(d, &mut add);
    }
    for (_, l) in layers {
        let entries = l
            .get("variants")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
            .chain(l.get("system"));
        for e in entries {
            for (_, v) in e
                .get("values")
                .and_then(Json::as_object)
                .into_iter()
                .flatten()
            {
                from_value(v, &mut add);
            }
        }
    }
    ids
}

/// The shared rule-set layers an object gets: `ruleset-<id>-<variant>` for every rule set it
/// declares in `rules.rulesets` and has no rule-set layer of its own for (proposal 0002). When
/// the declaration lists `fields`, only those values are kept, so an object declares just the
/// fields it uses.
fn shared_rulesets(object: &Json, layers: &[(String, Json)], lib: &Library) -> Vec<(String, Json)> {
    let declared: Vec<(String, Option<Vec<String>>)> = match object.pointer("/rules/rulesets") {
        Some(Json::Object(m)) => m.iter().map(|(k, d)| (k.clone(), fields_of(d))).collect(),
        Some(Json::Array(a)) => a
            .iter()
            .filter_map(|r| {
                r.get("id")
                    .and_then(Json::as_str)
                    .map(|id| (id.to_owned(), fields_of(r)))
            })
            .collect(),
        _ => Vec::new(),
    };
    let own: Vec<&str> = layers
        .iter()
        .filter(|(_, l)| l.get("kind").and_then(Json::as_str) == Some("ruleset"))
        .filter_map(|(_, l)| l.get("id").and_then(Json::as_str))
        .collect();
    let mut out = Vec::new();
    for (id, fields) in declared
        .iter()
        .filter(|(id, _)| !own.contains(&id.as_str()))
    {
        for (variant, l) in lib.rulesets.get(id).into_iter().flatten() {
            let mut l = l.clone();
            if let (Some(fields), Some(Json::Object(values))) = (fields, l.get_mut("values")) {
                values.retain(|k, _| fields.contains(k));
            }
            out.push((format!("ruleset-{id}-{variant}"), l));
        }
    }
    out
}

/// The field names of a rule-set declaration, if it lists them.
fn fields_of(declaration: &Json) -> Option<Vec<String>> {
    declaration
        .get("fields")
        .and_then(Json::as_object)
        .map(|f| f.keys().cloned().collect())
}

/// Compile an object source with its layers (file stem, layer source). `base` is the
/// object's repository path (ids and node ids derive from it, e.g.
/// `library/templates/IfcWall`).
pub fn compile(
    object: &Json,
    layers: &[(String, Json)],
    lib: &Library,
    base: &str,
    resolver: &dyn Resolver,
    timestamp: &str,
) -> Result<Compiled, Error> {
    let shared = shared_rulesets(object, layers, lib);
    let with_shared: Vec<(String, Json)>;
    let layers: &[(String, Json)] = if shared.is_empty() {
        layers
    } else {
        with_shared = layers.iter().cloned().chain(shared).collect();
        &with_shared
    };
    let mut src = object
        .as_object()
        .cloned()
        .ok_or_else(|| err("object.yaml must be a map"))?;
    src.remove("sheet");
    src.remove("status");
    src.remove("template");
    src.remove("id");
    let type_name = src.shift_remove("type_name");
    let mut prov = Map::new();
    let mut body = Json::Object(src.clone());
    strip(&mut body, "", &mut prov)?;
    let src = body.as_object().cloned().expect("map");
    let version = src
        .get("version")
        .and_then(Json::as_str)
        .unwrap_or("0.1.0")
        .to_owned();

    let mut a = Map::new();
    let ifc = src
        .get("ifc")
        .and_then(Json::as_object)
        .ok_or_else(|| err("object.yaml: ifc is required"))?;
    let class = ifc
        .get("class")
        .and_then(Json::as_str)
        .ok_or_else(|| err("object.yaml: ifc.class is required"))?;
    a.insert(
        "bsi::ifc::class".into(),
        json!({"code": class, "uri": class_uri(class)}),
    );
    let name = src.get("name").cloned().unwrap_or(Json::Null);
    a.insert("bsi::ifc::prop::Name".into(), name.clone());
    a.insert("bsi::ifc::prop::TypeName".into(), type_name.unwrap_or(name));
    a.insert("opo::object".into(), object_attribute(&src, lib, base)?);
    a.insert(
        "opo::parameters".into(),
        named_list(src.get("parameters"), "name", "parameters")?.unwrap_or(json!([])),
    );
    if let Some(v) = named_list(src.get("slots"), "name", "slots")? {
        a.insert("opo::slots".into(), v);
    }
    if let Some(r) = src.get("rules") {
        let r = r.as_object().ok_or_else(|| err("rules must be a map"))?;
        let mut rules = Map::new();
        if let Some(d) = r.get("derived") {
            let d = d
                .as_object()
                .ok_or_else(|| err("rules.derived must be a map by name"))?;
            let list: Vec<Json> = d
                .iter()
                .map(|(k, v)| match v {
                    Json::String(e) => Ok(json!({"name": k, "expr": e})),
                    Json::Object(m) => {
                        let mut e = Map::new();
                        e.insert("name".into(), json!(k));
                        e.extend(m.clone());
                        Ok(Json::Object(e))
                    }
                    _ => Err(err(format!(
                        "rules.derived.{k} must be a formula or {{expr, note}}"
                    ))),
                })
                .collect::<Result<_, _>>()?;
            rules.insert("derived".into(), Json::Array(list));
        }
        if let Some(v) = named_list(r.get("checks"), "id", "rules.checks")? {
            rules.insert("checks".into(), v);
        }
        if let Some(v) = named_list(r.get("rulesets"), "id", "rules.rulesets")? {
            rules.insert("rulesets".into(), v);
        }
        a.insert("opo::rules".into(), Json::Object(rules));
    }
    a.insert(
        "opo::recipe".into(),
        named_list(src.get("recipe"), "id", "recipe")?.unwrap_or(json!([])),
    );
    a.insert(
        "opo::representations".into(),
        named_list(src.get("representations"), "id", "representations")?.unwrap_or(json!([])),
    );
    for (k, attr) in [("host", "opo::host"), ("handles", "opo::handles")] {
        if let Some(v) = src.get(k) {
            a.insert(attr.into(), v.clone());
        }
    }
    let ids = material_ids(&src, layers);
    if !ids.is_empty() || src.contains_key("materials") {
        let mut mats = Map::new();
        let mut hatches = Map::new();
        for id in &ids {
            let m = lib
                .materials
                .get(id)
                .ok_or_else(|| err(format!("material '{id}' is not in styles/materials.yaml")))?;
            mats.insert(id.clone(), m.clone());
            for role in ["cut", "surface"] {
                if let Some(h) = m
                    .get(role)
                    .and_then(|r| r.get("hatch"))
                    .and_then(Json::as_str)
                {
                    let hd = lib.hatches.get(h).ok_or_else(|| {
                        err(format!(
                            "hatch '{h}' of material '{id}' is not in styles/hatches.yaml"
                        ))
                    })?;
                    hatches.entry(h.to_owned()).or_insert_with(|| hd.clone());
                }
            }
        }
        a.insert("opo::materials".into(), Json::Object(mats));
        a.insert("opo::hatches".into(), Json::Object(hatches));
    }
    a.insert("opo::ifc".into(), Json::Object(ifc.clone()));
    if let Some(v) = src.get("ports") {
        a.insert("opo::ports".into(), v.clone());
    }
    if let Some(v) = src.get("interface") {
        a.insert("opo::interface".into(), v.clone());
    }

    let def = TypeDefinition::from_attributes(&a).map_err(|e| {
        err(format!(
            "object.yaml compiles to an invalid definition: {e}"
        ))
    })?;
    for (k, v) in plain_ifc_values(&def, &Map::new(), resolver)? {
        a.insert(k, v);
    }
    if !prov.is_empty() {
        a.insert("opo::provenance".into(), Json::Object(prov));
    }
    let type_path = node_id(&format!("{base}/type"));
    let base_doc = document(
        base,
        "object.ifcx",
        &version,
        timestamp,
        vec![Node {
            path: type_path.clone(),
            children: None,
            inherits: None,
            attributes: Some(a),
        }],
    );

    let mut out = Vec::new();
    for (stem, layer) in layers {
        let rel = format!("layers/{stem}.ifcx");
        let nodes = compile_layer(stem, layer, &def, base, &rel, &type_path, resolver)?;
        out.push((
            format!("{stem}.ifcx"),
            document(base, &rel, &version, timestamp, nodes),
        ));
    }
    Ok(Compiled {
        base: base_doc,
        layers: out,
    })
}

fn id_or(entry: &Map<String, Json>, generated: String) -> String {
    entry
        .get("node")
        .and_then(Json::as_str)
        .map(str::to_owned)
        .unwrap_or(generated)
}

fn compile_layer(
    stem: &str,
    layer: &Json,
    def: &TypeDefinition,
    base: &str,
    rel: &str,
    type_path: &str,
    resolver: &dyn Resolver,
) -> Result<Vec<Node>, Error> {
    let l = layer
        .as_object()
        .ok_or_else(|| err(format!("layers/{stem}.yaml must be a map")))?;
    let kind = l
        .get("kind")
        .and_then(Json::as_str)
        .ok_or_else(|| err(format!("layers/{stem}.yaml: kind is required")))?;
    let mut nodes = Vec::new();
    match kind {
        "sizes" | "types" => {
            let mut parent = ("type", type_path.to_owned(), Map::new());
            if let Some(sys) = l.get("system") {
                let sys = sys
                    .as_object()
                    .ok_or_else(|| err(format!("layers/{stem}.yaml: system must be a map")))?;
                let mut values = sys.get("values").cloned().unwrap_or(json!({}));
                let mut prov = Map::new();
                strip(&mut values, "values", &mut prov)?;
                let path = id_or(sys, node_id(&format!("{base}/{rel}/system")));
                let mut a = Map::new();
                a.insert("opo::values".into(), values.clone());
                a.insert(
                    "bsi::ifc::prop::Name".into(),
                    sys.get("name")
                        .cloned()
                        .ok_or_else(|| err(format!("layers/{stem}.yaml: system needs a name")))?,
                );
                if let Some(n) = sys.get("note") {
                    a.insert("opo::note".into(), n.clone());
                }
                if !prov.is_empty() {
                    a.insert("opo::provenance".into(), Json::Object(prov));
                }
                nodes.push(Node {
                    path: path.clone(),
                    children: None,
                    inherits: Some(map1("type", type_path)),
                    attributes: Some(a),
                });
                parent = (
                    "system",
                    path,
                    values.as_object().cloned().unwrap_or_default(),
                );
            }
            for v in l
                .get("variants")
                .and_then(Json::as_array)
                .ok_or_else(|| err(format!("layers/{stem}.yaml: variants are required")))?
            {
                let v = v
                    .as_object()
                    .ok_or_else(|| err(format!("layers/{stem}.yaml: a variant must be a map")))?;
                let name = v.get("name").and_then(Json::as_str).ok_or_else(|| {
                    err(format!("layers/{stem}.yaml: every variant needs a name"))
                })?;
                let mut values = v.get("values").cloned().unwrap_or(json!({}));
                let mut prov = Map::new();
                strip(&mut values, "values", &mut prov)?;
                let mut merged = parent.2.clone();
                merged.extend(values.as_object().cloned().unwrap_or_default());
                let mut a = Map::new();
                a.insert("opo::values".into(), values);
                a.insert("bsi::ifc::prop::TypeName".into(), json!(name));
                if let Some(n) = v.get("note") {
                    a.insert("opo::note".into(), n.clone());
                }
                for (k, x) in plain_ifc_values(def, &merged, resolver)? {
                    a.insert(k, x);
                }
                for (k, x) in v.get("ifc").and_then(Json::as_object).into_iter().flatten() {
                    let mut x = x.clone();
                    strip(&mut x, &format!("ifc.{k}"), &mut prov)?;
                    a.insert(format!("bsi::ifc::prop::{k}"), x);
                }
                if !prov.is_empty() {
                    a.insert("opo::provenance".into(), Json::Object(prov));
                }
                let path = id_or(v, node_id(&format!("{base}/{rel}/{name}")));
                nodes.push(Node {
                    path,
                    children: None,
                    inherits: Some(map1(parent.0, &parent.1)),
                    attributes: Some(a),
                });
            }
        }
        "ruleset" => {
            let mut rs = Map::new();
            for k in ["id", "label", "uri", "source", "note", "values"] {
                if let Some(v) = l.get(k) {
                    rs.insert(k.into(), v.clone());
                }
            }
            let mut rs = Json::Object(rs);
            let mut prov = Map::new();
            strip(&mut rs, "ruleset", &mut prov)?;
            let mut a = Map::new();
            a.insert("opo::ruleset".into(), rs);
            if !prov.is_empty() {
                a.insert("opo::provenance".into(), Json::Object(prov));
            }
            let path = id_or(l, node_id(&format!("{base}/{rel}/ruleset")));
            nodes.push(Node {
                path,
                children: None,
                inherits: None,
                attributes: Some(a),
            });
        }
        "binding" => {
            let attrs = l
                .get("attributes")
                .and_then(Json::as_object)
                .cloned()
                .ok_or_else(|| err(format!("layers/{stem}.yaml: attributes are required")))?;
            nodes.push(Node {
                path: type_path.to_owned(),
                children: None,
                inherits: None,
                attributes: Some(attrs),
            });
        }
        other => return Err(err(format!("layers/{stem}.yaml: unknown kind '{other}'"))),
    }
    Ok(nodes)
}

fn map1(k: &str, v: &str) -> Map<String, Json> {
    let mut m = Map::new();
    m.insert(k.into(), json!(v));
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_is_stripped_and_checked() {
        let mut v = json!({"parameters": {"h": {"default": {"value": 1.0, "source": "iso", "status": "norm"}}}, "x": [{"value": 2, "status": "to_verify", "note": "check"}]});
        let mut prov = Map::new();
        strip(&mut v, "", &mut prov).unwrap();
        assert_eq!(v, json!({"parameters": {"h": {"default": 1.0}}, "x": [2]}));
        assert_eq!(
            prov["parameters.h.default"],
            json!({"source": "iso", "status": "norm"})
        );
        assert_eq!(prov["x.0"], json!({"status": "to_verify", "note": "check"}));
        for bad in [
            json!({"value": 1, "status": "norm"}),
            json!({"value": 1, "status": "typical"}),
            json!({"value": 1, "status": "guess", "note": "n"}),
        ] {
            let mut b = bad;
            assert!(strip(&mut b, "p", &mut Map::new()).is_err());
        }
    }

    #[test]
    fn named_maps_become_lists() {
        let v = named_list(
            Some(&json!({"b": {"type": "length"}, "a": {"type": "angle"}})),
            "name",
            "parameters",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            v,
            json!([{"name": "b", "type": "length"}, {"name": "a", "type": "angle"}])
        );
        assert!(named_list(Some(&json!({"b": {"name": "b"}})), "name", "parameters").is_err());
    }

    #[test]
    fn shared_rulesets_are_added_unless_the_object_has_its_own() {
        let mut lib = Library::default();
        let uk = json!({"kind": "ruleset", "id": "stairs", "label": "UK", "source": "s", "values": {"riser_max": "0.22"}});
        let us = json!({"kind": "ruleset", "id": "stairs", "label": "US", "source": "s", "values": {"riser_max": "0.196"}});
        lib.rulesets
            .insert("stairs".into(), vec![("uk".into(), uk), ("us".into(), us)]);
        lib.rulesets.insert(
            "guards".into(),
            vec![("uk".into(), json!({"kind": "ruleset", "id": "guards"}))],
        );
        let object = json!({"rules": {"rulesets": {"stairs": {"default": null}}}});
        let names: Vec<String> = shared_rulesets(&object, &[], &lib)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(names, ["ruleset-stairs-uk", "ruleset-stairs-us"]);
        // Only the declared fields are kept.
        let narrow =
            json!({"rules": {"rulesets": {"stairs": {"fields": {"going_min": "length"}}}}});
        let got = shared_rulesets(&narrow, &[], &lib);
        assert_eq!(got[0].1["values"], json!({}));
        let own = vec![(
            "my-stairs".to_owned(),
            json!({"kind": "ruleset", "id": "stairs"}),
        )];
        assert!(shared_rulesets(&object, &own, &lib).is_empty());
        assert!(shared_rulesets(&json!({}), &[], &lib).is_empty());
    }
}
