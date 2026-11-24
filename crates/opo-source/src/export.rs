//! IFCX object folder → object source (the reverse of [`crate::compile`]), for converting
//! existing objects into library sources.

use std::path::Path;

use opo_build::node_id;
use opo_ifcx::{ObjectDir, compose, load_object_dir};
use serde_json::{Map, Value as Json, json};

use crate::{Error, Library};

/// An exported object: its source, its layer sources and the registry entries it uses.
#[derive(Debug, Clone, PartialEq)]
pub struct Exported {
    /// `object.yaml`.
    pub object: Json,
    /// `layers/<stem>.yaml`.
    pub layers: Vec<(String, Json)>,
    /// References, materials and hatches the object uses.
    pub library: Library,
}

fn err(msg: impl Into<String>) -> Error {
    Error::Source(msg.into())
}

/// A registry id from a name: lower case, other characters as `-`.
fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() || c == '.' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
        if out.len() >= 60 {
            break;
        }
    }
    out.trim_matches(['-', '.']).to_owned()
}

/// A list of `{field: k, ...}` as a named map.
fn named_map(v: Option<&Json>, field: &str) -> Option<Json> {
    let items = v?.as_array()?;
    let mut m = Map::new();
    for it in items {
        let mut body = it.as_object()?.clone();
        let k = body.shift_remove(field)?.as_str()?.to_owned();
        m.insert(k, Json::Object(body));
    }
    Some(Json::Object(m))
}

fn reference_kind(role: &str) -> &'static str {
    let r = role.to_lowercase();
    if r.contains("ifc") {
        "ifc"
    } else if r.contains("norm") {
        "norm"
    } else if r.contains("rule") || r.contains("regulation") {
        "regulation"
    } else if r.contains("parametric") {
        "parametric_basis"
    } else if r.contains("maker") || r.contains("manufacturer") {
        "maker"
    } else {
        "other"
    }
}

/// Export an unpacked object folder. `base` is its repository path (as for compile).
pub fn export_dir(dir: &Path, base: &str) -> Result<Exported, Error> {
    let obj = load_object_dir(dir).map_err(|e| Error::Ifcx(e.to_string()))?;
    export(&obj, base)
}

/// Export an object (see [`export_dir`]).
pub fn export(obj: &ObjectDir, base: &str) -> Result<Exported, Error> {
    let comp = compose(std::slice::from_ref(&obj.base)).map_err(|e| Error::Ifcx(e.to_string()))?;
    let type_path = comp
        .type_node()
        .ok_or_else(|| err("no type node"))?
        .to_owned();
    let a = &comp.node(&type_path).expect("exists").attributes;
    let get = |k: &str| a.get(k);
    let o = get("opo::object")
        .and_then(Json::as_object)
        .ok_or_else(|| err("no opo::object"))?;
    let mut lib = Library::default();

    let mut src = Map::new();
    src.insert("id".into(), json!(base.rsplit('/').next().unwrap_or(base)));
    for k in ["name", "version"] {
        src.insert(k.into(), o.get(k).cloned().unwrap_or(Json::Null));
    }
    if let Some(tn) = get("bsi::ifc::prop::TypeName").filter(|tn| Some(*tn) != o.get("name")) {
        src.insert("type_name".into(), tn.clone());
    }
    if let Some(l) = o.get("license").filter(|l| l.as_str() != Some("CC0-1.0")) {
        src.insert("license".into(), l.clone());
    }
    if let Some(au) = o
        .get("author")
        .and_then(Json::as_str)
        .filter(|a| *a != "OPO format contributors")
    {
        src.insert("authors".into(), json!([au]));
    }
    src.insert(
        "description".into(),
        o.get("description").cloned().unwrap_or(Json::Null),
    );
    for k in ["tags", "archetype", "kind"] {
        if let Some(v) = o.get(k) {
            src.insert(k.into(), v.clone());
        }
    }
    let mut refs = Vec::new();
    for r in o
        .get("references")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
    {
        let name = r.get("name").and_then(Json::as_str).unwrap_or_default();
        let mut entry = Map::new();
        entry.insert("title".into(), json!(name));
        entry.insert(
            "kind".into(),
            json!(reference_kind(
                r.get("role").and_then(Json::as_str).unwrap_or_default()
            )),
        );
        for (from, to) in [
            ("license", "licence"),
            ("uri", "url"),
            ("version", "version"),
        ] {
            if let Some(v) = r.get(from) {
                entry.insert(to.into(), v.clone());
            }
        }
        // The same name with other details gets its own id.
        let mut id = slug(name);
        let mut n = 2;
        while lib.references.get(&id).is_some_and(|e| {
            e.get("title") != entry.get("title") || e != &Json::Object(entry.clone())
        }) {
            id = format!("{}-{n}", slug(name));
            n += 1;
        }
        lib.references.insert(id.clone(), Json::Object(entry));
        refs.push(json!({"ref": id, "role": r.get("role"), "used": r.get("used")}));
    }
    src.insert("references".into(), Json::Array(refs));
    if let Some(v) = named_map(get("opo::parameters"), "name") {
        src.insert("parameters".into(), v);
    }
    if let Some(v) = named_map(get("opo::slots"), "name") {
        src.insert("slots".into(), v);
    }
    if let Some(r) = get("opo::rules").and_then(Json::as_object) {
        let mut rules = Map::new();
        if let Some(d) = r.get("derived").and_then(Json::as_array) {
            let mut m = Map::new();
            for x in d {
                let name = x
                    .get("name")
                    .and_then(Json::as_str)
                    .ok_or_else(|| err("derived value without a name"))?;
                let v = match x.get("note") {
                    None => x.get("expr").cloned().unwrap_or(Json::Null),
                    Some(note) => json!({"expr": x.get("expr"), "note": note}),
                };
                m.insert(name.into(), v);
            }
            rules.insert("derived".into(), Json::Object(m));
        }
        for (k, f) in [("checks", "id"), ("rulesets", "id")] {
            if let Some(v) = named_map(r.get(k), f) {
                rules.insert(k.into(), v);
            }
        }
        src.insert("rules".into(), Json::Object(rules));
    }
    for (attr, k, field) in [
        ("opo::recipe", "recipe", "id"),
        ("opo::representations", "representations", "id"),
    ] {
        if let Some(v) = named_map(get(attr), field) {
            src.insert(k.into(), v);
        }
    }
    for (attr, k) in [
        ("opo::host", "host"),
        ("opo::handles", "handles"),
        ("opo::ports", "ports"),
    ] {
        if let Some(v) = get(attr) {
            src.insert(k.into(), v.clone());
        }
    }
    if let Some(m) = get("opo::materials").and_then(Json::as_object) {
        src.insert(
            "materials".into(),
            Json::Array(m.keys().map(|k| json!(k)).collect()),
        );
        lib.materials.extend(m.clone());
    }
    if let Some(h) = get("opo::hatches").and_then(Json::as_object) {
        lib.hatches.extend(h.clone());
    }
    src.insert(
        "ifc".into(),
        get("opo::ifc").cloned().ok_or_else(|| err("no opo::ifc"))?,
    );
    let computed: Vec<String> = a
        .get("opo::ifc")
        .and_then(|i| i.get("properties"))
        .and_then(Json::as_object)
        .into_iter()
        .flatten()
        .map(|(k, _)| k.clone())
        .filter(|k| k.starts_with("bsi::ifc::prop::"))
        .collect();

    let mut layers = Vec::new();
    for (file, doc) in &obj.layers {
        let stem = file.trim_end_matches(".ifcx").to_owned();
        let rel = format!("layers/{file}");
        let lc = compose(std::slice::from_ref(doc)).map_err(|e| Error::Ifcx(e.to_string()))?;
        let mut l = Map::new();
        if let Some(p) = lc.with_attribute("opo::ruleset").next() {
            let n = lc.node(p).expect("exists");
            l.insert("kind".into(), json!("ruleset"));
            l.extend(
                n.attributes["opo::ruleset"]
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            );
            if p != node_id(&format!("{base}/{rel}/ruleset")) {
                l.insert("node".into(), json!(p));
            }
            layers.push((stem, Json::Object(l)));
            continue;
        }
        l.insert(
            "kind".into(),
            json!(
                if stem.starts_with("types") || stem.starts_with("profiles") {
                    "types"
                } else {
                    "sizes"
                }
            ),
        );
        let parents: std::collections::HashSet<String> = lc
            .paths()
            .iter()
            .flat_map(|p| {
                lc.node(p)
                    .expect("exists")
                    .inherit_paths()
                    .map(|(_, t)| t.to_owned())
                    .collect::<Vec<_>>()
            })
            .collect();
        let systems: Vec<&String> = lc.paths().iter().filter(|p| parents.contains(*p)).collect();
        if systems.len() > 1 {
            return Err(err(format!(
                "{rel}: more than one system node is not supported"
            )));
        }
        let entry = |p: &str, name_key: &str, generated: String| -> Map<String, Json> {
            let n = lc.node(p).expect("exists");
            let mut e = Map::new();
            let name = n
                .attributes
                .get(name_key)
                .or_else(|| n.attributes.get("bsi::ifc::prop::Name"))
                .or_else(|| n.attributes.get("bsi::ifc::prop::TypeName"));
            e.insert("name".into(), name.cloned().unwrap_or(Json::Null));
            if p != generated {
                e.insert("node".into(), json!(p));
            }
            if let Some(v) = n
                .attributes
                .get("opo::values")
                .filter(|v| v.as_object().is_some_and(|m| !m.is_empty()))
            {
                e.insert("values".into(), v.clone());
            }
            if let Some(note) = n.attributes.get("opo::note") {
                e.insert("note".into(), note.clone());
            }
            let plain: Map<String, Json> = n
                .attributes
                .iter()
                .filter(|(k, _)| {
                    k.starts_with("bsi::ifc::prop::")
                        && !k.ends_with("::TypeName")
                        && !k.ends_with("::Name")
                        && !computed.contains(k)
                })
                .map(|(k, v)| {
                    (
                        k.trim_start_matches("bsi::ifc::prop::").to_owned(),
                        v.clone(),
                    )
                })
                .collect();
            if !plain.is_empty() {
                e.insert("ifc".into(), Json::Object(plain));
            }
            e
        };
        if let Some(s) = systems.first() {
            l.insert(
                "system".into(),
                Json::Object(entry(
                    s,
                    "bsi::ifc::prop::Name",
                    node_id(&format!("{base}/{rel}/system")),
                )),
            );
        }
        let mut variants = Vec::new();
        for p in lc.paths() {
            if systems.contains(&p)
                || !lc
                    .node(p)
                    .expect("exists")
                    .attributes
                    .contains_key("bsi::ifc::prop::TypeName")
            {
                continue;
            }
            let name = lc.node(p).expect("exists").attributes["bsi::ifc::prop::TypeName"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            variants.push(Json::Object(entry(
                p,
                "bsi::ifc::prop::TypeName",
                node_id(&format!("{base}/{rel}/{name}")),
            )));
        }
        l.insert("variants".into(), Json::Array(variants));
        layers.push((stem, Json::Object(l)));
    }
    Ok(Exported {
        object: Json::Object(src),
        layers,
        library: lib,
    })
}

impl Exported {
    /// Write `object.yaml` and `layers/*.yaml` into a folder.
    pub fn write(&self, dir: &Path) -> Result<(), Error> {
        std::fs::create_dir_all(dir.join("layers")).map_err(|e| Error::Io(dir.into(), e))?;
        let w = |p: std::path::PathBuf, v: &Json| {
            std::fs::write(&p, crate::yaml::write(v)).map_err(|e| Error::Io(p.clone(), e))
        };
        w(dir.join("object.yaml"), &self.object)?;
        for (stem, l) in &self.layers {
            w(dir.join(format!("layers/{stem}.yaml")), l)?;
        }
        if self.layers.is_empty() {
            let _ = std::fs::remove_dir(dir.join("layers"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(
            slug("ISO 21542:2021 Building construction"),
            "iso-21542-2021-building-construction"
        );
        assert_eq!(slug("dormakaba bim-all-doors"), "dormakaba-bim-all-doors");
    }

    #[test]
    fn reference_kinds() {
        assert_eq!(reference_kind("IFC parameters"), "ifc");
        assert_eq!(reference_kind("Norm for sizes"), "norm");
        assert_eq!(reference_kind("generator"), "other");
    }
}
