//! Class templates from archetypes, the per-class table and IFC data (proposal 0002 §4,
//! decision 0010).

use std::collections::HashMap;
use std::path::Path;

use serde_json::{Map, Value as Json, json};

use crate::{Error, Library, read_yaml};

/// IFC data extracted from the pinned IFC release (`library/ifc/`).
#[derive(Debug, Clone, Default)]
pub struct IfcData {
    /// Class → predefined types (without USERDEFINED / NOTDEFINED).
    pub types: HashMap<String, Vec<String>>,
    /// Class → property and quantity set templates (`{name, kind, generic, applies_to, props}`).
    pub sets: HashMap<String, Vec<Json>>,
}

impl IfcData {
    /// From `elements.json` (`[[depth, class, abstract, types]…]`) and `psets.json`
    /// (`{class: [set…]}`).
    pub fn from_json(elements: &Json, psets: &Json) -> Self {
        let mut types = HashMap::new();
        for e in elements.as_array().into_iter().flatten() {
            if let (Some(class), Some(t)) = (
                e.get(1).and_then(Json::as_str),
                e.get(3).and_then(Json::as_array),
            ) {
                types.insert(
                    class.to_owned(),
                    t.iter()
                        .filter_map(|x| x.as_str().map(str::to_owned))
                        .collect(),
                );
            }
        }
        let sets = psets
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, v)| (k.clone(), v.as_array().cloned().unwrap_or_default()))
            .collect();
        IfcData { types, sets }
    }

    /// Read `<dir>/elements.json` and `<dir>/psets.json`.
    pub fn load(dir: &Path) -> Result<Self, Error> {
        let read = |f: &str| -> Result<Json, Error> {
            let p = dir.join(f);
            let text = std::fs::read_to_string(&p).map_err(|e| Error::Io(p.clone(), e))?;
            serde_json::from_str(&text).map_err(|e| Error::Source(format!("{}: {e}", p.display())))
        };
        Ok(Self::from_json(
            &read("elements.json")?,
            &read("psets.json")?,
        ))
    }
}

/// Settings of the generator.
#[derive(Debug, Clone)]
pub struct GeneratorOptions {
    /// Version written to every template.
    pub version: String,
    /// Name of the generator in each template's references.
    pub generator_name: String,
}

impl Default for GeneratorOptions {
    fn default() -> Self {
        GeneratorOptions {
            version: "0.1.0".into(),
            generator_name: "opo generate (opo-rs)".into(),
        }
    }
}

/// The archetypes and class table of a library.
#[derive(Debug, Clone, Default)]
pub struct TemplateSources {
    /// Archetype name → archetype source.
    pub archetypes: Map<String, Json>,
    /// `classes.yaml`: class → class entry, in file order.
    pub classes: Map<String, Json>,
}

impl TemplateSources {
    /// Read `<root>/templates/archetypes/*.yaml` and `<root>/templates/classes.yaml`.
    pub fn load(root: &Path) -> Result<Self, Error> {
        let dir = root.join("templates/archetypes");
        let mut archetypes = Map::new();
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .map_err(|e| Error::Io(dir.clone(), e))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "yaml"))
            .collect();
        files.sort();
        for f in files {
            let v = read_yaml(&f)?.unwrap_or(Json::Null);
            let name = v
                .get("archetype")
                .and_then(Json::as_str)
                .map(str::to_owned)
                .ok_or_else(|| Error::Source(format!("{}: archetype is required", f.display())))?;
            archetypes.insert(name, v);
        }
        let classes = match read_yaml(&root.join("templates/classes.yaml"))? {
            Some(Json::Object(m)) => m,
            _ => {
                return Err(Error::Source(
                    "templates/classes.yaml must be a map of IFC classes".into(),
                ));
            }
        };
        Ok(TemplateSources {
            archetypes,
            classes,
        })
    }
}

fn err(m: impl Into<String>) -> Error {
    Error::Source(m.into())
}

/// Merge a named map: entries of `over` override `base` entries key by key; new names are
/// appended.
fn merge_named(base: Option<&Json>, over: Option<&Json>) -> Option<Json> {
    let mut out = base.and_then(Json::as_object).cloned().unwrap_or_default();
    for (k, v) in over.and_then(Json::as_object).into_iter().flatten() {
        match (out.get_mut(k), v) {
            (Some(Json::Object(b)), Json::Object(o)) => b.extend(o.clone()),
            _ => {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    (base.is_some() || over.is_some()).then_some(Json::Object(out))
}

fn concat(base: Option<&Json>, over: Option<&Json>) -> Option<Json> {
    let mut out: Vec<Json> = base.and_then(Json::as_array).cloned().unwrap_or_default();
    for v in over.and_then(Json::as_array).into_iter().flatten() {
        if !out.contains(v) {
            out.push(v.clone());
        }
    }
    (base.is_some() || over.is_some()).then_some(Json::Array(out))
}

fn class_uri(class: &str) -> String {
    format!("https://identifier.buildingsmart.org/uri/buildingsmart/ifc/4.3/class/{class}")
}

/// A generated class template: object source, layer sources, registry entries it adds.
pub type Generated = (Json, Vec<(String, Json)>, Library);

/// Generate the object source of one class template, its layer sources, and the registry
/// entries it adds (the IFC and generator references).
pub fn generate(
    class: &str,
    entry: &Json,
    sources: &TemplateSources,
    ifc: &IfcData,
    opts: &GeneratorOptions,
) -> Result<Generated, Error> {
    let e = entry
        .as_object()
        .ok_or_else(|| err(format!("classes.yaml: {class} must be a map")))?;
    let arch_name = e
        .get("archetype")
        .and_then(Json::as_str)
        .ok_or_else(|| err(format!("{class}: archetype is required")))?;
    let arch = sources
        .archetypes
        .get(arch_name)
        .ok_or_else(|| err(format!("{class}: no archetype '{arch_name}'")))?;
    let a = arch.as_object().cloned().unwrap_or_default();
    let s = |k: &str| {
        e.get(k)
            .and_then(Json::as_str)
            .ok_or_else(|| err(format!("{class}: {k} is required")))
    };
    let types = ifc
        .types
        .get(class)
        .ok_or_else(|| err(format!("{class} is not in the IFC data")))?;
    let default_type = s("default_type")?;

    // Parameters: IFC type first, then the archetype and class parameters, then the
    // property sets.
    let mut params = Map::new();
    let mut options: Vec<Json> = types.iter().map(|t| json!(t)).collect();
    options.extend([json!("USERDEFINED"), json!("NOTDEFINED")]);
    params.insert(
        "predefined_type".into(),
        json!({"type": "choice", "default": default_type, "label": "Predefined type", "group": "IFC", "options": options,
               "description": format!("IFC 4.3 {class}TypeEnum. Property sets that apply only to some types follow this choice.")}),
    );
    params.insert(
        "object_type".into(),
        json!({"type": "text", "default": "", "label": "Object type (user-defined)", "group": "IFC", "visible": "predefined_type = \"USERDEFINED\""}),
    );
    if let Some(Json::Object(m)) = merge_named(a.get("parameters"), e.get("parameters")) {
        params.extend(m);
    }

    let mut ifc_in = a
        .get("ifc")
        .and_then(Json::as_object)
        .cloned()
        .unwrap_or_default();
    let class_ifc = e
        .get("ifc")
        .and_then(Json::as_object)
        .cloned()
        .unwrap_or_default();
    // A formula applies wherever the class has that property (proposal 0002 §4): property-set
    // keys whose set the class does not have are dropped; IFC attributes (bsi::…) are kept.
    let class_sets: Vec<&str> = ifc
        .sets
        .get(class)
        .into_iter()
        .flatten()
        .filter_map(|s| s["name"].as_str())
        .collect();
    let properties: Map<String, Json> =
        merge_named_flat(ifc_in.get("properties"), class_ifc.get("properties"))
            .into_iter()
            .filter(|(k, _)| {
                k.starts_with("bsi::")
                    || k.split_once('.')
                        .is_some_and(|(set, _)| class_sets.contains(&set))
            })
            .collect();
    let qto_formulas = merge_named_flat(ifc_in.get("quantities"), class_ifc.get("quantities"));
    let mut quantities = Map::new();
    for set in ifc.sets.get(class).into_iter().flatten() {
        let name = set["name"].as_str().unwrap_or_default();
        let props: Vec<&Json> = set["props"].as_array().into_iter().flatten().collect();
        if set["kind"] == "qto" {
            let mut values = Map::new();
            let mut geom = Vec::new();
            for p in props {
                let pn = p["name"].as_str().unwrap_or_default();
                match qto_formulas.get(pn) {
                    Some(f) => {
                        values.insert(pn.into(), f.clone());
                    }
                    None => geom.push(json!(pn)),
                }
            }
            quantities.insert(
                name.into(),
                json!({"values": values, "from_geometry": geom}),
            );
            continue;
        }
        let mut list = Vec::new();
        for p in props {
            let mut d = Map::new();
            d.insert("name".into(), p["name"].clone());
            d.insert(
                "measure".into(),
                p.get("measure").cloned().unwrap_or(Json::Null),
            );
            if let Some(en) = p
                .get("enum")
                .filter(|x| x.as_array().is_some_and(|a| !a.is_empty()))
            {
                d.insert("enum".into(), en.clone());
            }
            if let Some(f) = properties.get(&format!(
                "{name}.{}",
                p["name"].as_str().unwrap_or_default()
            )) {
                d.insert("value".into(), f.clone());
            }
            list.push(Json::Object(d));
        }
        let mut par = Map::new();
        for (k, v) in [
            ("type", json!("property_set")),
            ("default", json!({})),
            ("label", json!(name)),
            ("group", json!("IFC data")),
            ("properties", Json::Array(list)),
            ("optional", json!(set["generic"].as_bool().unwrap_or(false))),
        ] {
            par.insert(k.into(), v);
        }
        if let Some(at) = set["applies_to"].as_array().filter(|a| !a.is_empty()) {
            let items: Vec<String> = at.iter().map(|x| x.to_string()).collect();
            par.insert(
                "visible".into(),
                json!(format!(
                    "list_contains([{}], predefined_type)",
                    items.join(", ")
                )),
            );
        }
        params.insert(name.into(), Json::Object(par));
    }

    let mut ifc_out = Map::new();
    ifc_out.insert("class".into(), json!(class));
    ifc_out.insert("predefinedType".into(), json!("if predefined_type = \"USERDEFINED\" or predefined_type = \"NOTDEFINED\" then predefined_type else predefined_type"));
    ifc_out.insert(
        "objectType".into(),
        json!("if predefined_type = \"USERDEFINED\" then object_type else null"),
    );
    ifc_out.insert(
        "property_sets".into(),
        json!("all parameters of type property_set, with the values set by `value` formulas"),
    );
    ifc_out.insert("properties".into(), Json::Object(properties));
    ifc_out.insert("quantities".into(), Json::Object(quantities));
    ifc_out.insert("parameter_pset".into(), json!("OPO_Parameters"));
    ifc_in.remove("properties");
    ifc_in.remove("quantities");
    ifc_in.extend(
        class_ifc
            .into_iter()
            .filter(|(k, _)| k != "properties" && k != "quantities"),
    );
    ifc_out.extend(ifc_in);

    // Identity and references.
    let mut lib = Library::default();
    let ifc_ref = format!("ifc-4.3-add2-{}", class.to_lowercase());
    lib.references.insert(
        ifc_ref.clone(),
        json!({"title": format!("IFC 4.3 ADD2 {class}, {class}TypeEnum and its property and quantity set templates"), "kind": "ifc", "licence": "buildingSMART", "url": class_uri(class)}),
    );
    lib.references.insert(
        "opo-template-generator".into(),
        json!({"title": opts.generator_name, "kind": "other"}),
    );
    let mut refs = vec![
        json!({"ref": ifc_ref, "role": "IFC parameters", "used": "Class, predefined types, property and quantity set names, property names, value types and enumerations (names only)."}),
        json!({"ref": "opo-template-generator", "role": "generator", "used": format!("Generated from the `{arch_name}` archetype (spec/objects/archetypes.md).")}),
    ];
    refs.extend(
        concat(a.get("references"), e.get("references"))
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default(),
    );

    let mut src = Map::new();
    src.insert("id".into(), json!(class));
    src.insert("name".into(), json!(s("name")?));
    src.insert("kind".into(), json!("class_template"));
    src.insert("archetype".into(), json!(arch_name));
    src.insert("version".into(), json!(opts.version));
    src.insert("description".into(), json!(s("description")?));
    src.insert("tags".into(), json!([class, arch_name, "class template"]));
    src.insert("references".into(), Json::Array(refs));
    src.insert("parameters".into(), Json::Object(params));
    for k in ["slots", "recipe", "representations"] {
        if let Some(v) = merge_named(a.get(k), e.get(k)) {
            src.insert(k.into(), v);
        }
    }
    let mut rules = Map::new();
    for k in ["derived", "checks", "rulesets"] {
        let (ar, er) = (
            a.get("rules").and_then(|r| r.get(k)),
            e.get("rules").and_then(|r| r.get(k)),
        );
        let merged = if k == "derived" {
            (ar.is_some() || er.is_some()).then(|| Json::Object(merge_named_flat(ar, er)))
        } else {
            merge_named(ar, er)
        };
        if let Some(v) = merged {
            rules.insert(k.into(), v);
        }
    }
    src.insert("rules".into(), Json::Object(rules));
    if let Some(h) = merge_named_value(a.get("host"), e.get("host")) {
        src.insert("host".into(), h);
    }
    for k in ["handles", "ports", "materials"] {
        let v = if k == "handles" {
            e.get(k).or(a.get(k)).cloned()
        } else {
            concat(a.get(k), e.get(k))
        };
        if let Some(v) = v {
            src.insert(k.into(), v);
        }
    }
    src.insert("ifc".into(), Json::Object(ifc_out));
    let layers: Vec<(String, Json)> = e
        .get("layers")
        .and_then(Json::as_object)
        .into_iter()
        .flatten()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    Ok((Json::Object(src), layers, lib))
}

/// Merge two flat maps; `over` wins.
fn merge_named_flat(base: Option<&Json>, over: Option<&Json>) -> Map<String, Json> {
    let mut out = base.and_then(Json::as_object).cloned().unwrap_or_default();
    out.extend(over.and_then(Json::as_object).cloned().unwrap_or_default());
    out
}

/// A map value merged key by key (e.g. `host`), or the one given.
fn merge_named_value(base: Option<&Json>, over: Option<&Json>) -> Option<Json> {
    match (base, over) {
        (Some(Json::Object(b)), Some(Json::Object(o))) => {
            let mut m = b.clone();
            m.extend(o.clone());
            Some(Json::Object(m))
        }
        (b, o) => o.or(b).cloned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (TemplateSources, IfcData) {
        let mut s = TemplateSources::default();
        s.archetypes.insert(
            "point".into(),
            json!({"archetype": "point", "description": "d",
                   "parameters": {"width": {"type": "length", "default": 0.5, "label": "W", "group": "Size"}},
                   "recipe": {"body": {"op": "box", "args": {"min": "[0, 0, 0]", "max": "[width, width, width]"}}},
                   "representations": {"model.medium": {"context": "model", "lod": "medium", "source": "authored",
                       "ifc": {"context": "Body", "targetView": "MODEL_VIEW"}, "items": [{"geometry": "body"}]}},
                   "ifc": {"quantities": {"Width": "width"}}}),
        );
        s.classes.insert("IfcBoiler".into(), json!({"archetype": "point", "name": "Boiler", "description": "A boiler.", "default_type": "WATER",
            "parameters": {"width": {"default": 0.45}, "symbol": {"type": "text", "default": "BLR", "label": "S", "group": "Plan"}},
            "ifc": {"properties": {"Pset_BoilerTypeCommon.Capacity": "1", "Pset_WallCommon.IsExternal": "true"}}}));
        let ifc = IfcData::from_json(
            &json!([[4, "IfcBoiler", false, ["STEAM", "WATER"]]]),
            &json!({"IfcBoiler": [
                {"name": "Pset_BoilerTypeCommon", "kind": "pset", "generic": false, "applies_to": [], "props": [{"name": "Capacity", "measure": "IfcVolumeMeasure"}]},
                {"name": "Pset_Steam", "kind": "pset", "generic": true, "applies_to": ["STEAM"], "props": [{"name": "Pressure", "measure": "IfcPressureMeasure", "enum": []}]},
                {"name": "Qto_BoilerBaseQuantities", "kind": "qto", "generic": false, "applies_to": [], "props": [{"name": "Width"}, {"name": "GrossWeight"}]}]}),
        );
        (s, ifc)
    }

    #[test]
    fn generates_from_archetype_class_and_ifc() {
        let (s, ifc) = fixture();
        let (src, layers, lib) = generate(
            "IfcBoiler",
            &s.classes["IfcBoiler"],
            &s,
            &ifc,
            &GeneratorOptions::default(),
        )
        .unwrap();
        assert!(layers.is_empty());
        let p = src["parameters"].as_object().unwrap();
        assert_eq!(
            p.keys().collect::<Vec<_>>(),
            [
                "predefined_type",
                "object_type",
                "width",
                "symbol",
                "Pset_BoilerTypeCommon",
                "Pset_Steam"
            ]
        );
        assert_eq!(
            p["predefined_type"]["options"],
            json!(["STEAM", "WATER", "USERDEFINED", "NOTDEFINED"])
        );
        assert_eq!(
            p["width"]["default"],
            json!(0.45),
            "class overrides key by key"
        );
        assert_eq!(p["width"]["label"], json!("W"));
        assert_eq!(
            p["Pset_BoilerTypeCommon"]["properties"][0]["value"],
            json!("1")
        );
        assert_eq!(
            p["Pset_Steam"]["visible"],
            json!("list_contains([\"STEAM\"], predefined_type)")
        );
        assert_eq!(p["Pset_Steam"]["optional"], json!(true));
        assert_eq!(
            src["ifc"]["quantities"]["Qto_BoilerBaseQuantities"],
            json!({"values": {"Width": "width"}, "from_geometry": ["GrossWeight"]})
        );
        assert!(lib.references.contains_key("ifc-4.3-add2-ifcboiler"));
        assert!(
            src["ifc"]["properties"]
                .get("Pset_WallCommon.IsExternal")
                .is_none(),
            "IfcBoiler has no Pset_WallCommon"
        );
    }

    #[test]
    fn generated_source_compiles() {
        let (s, ifc) = fixture();
        let (src, layers, lib) = generate(
            "IfcBoiler",
            &s.classes["IfcBoiler"],
            &s,
            &ifc,
            &GeneratorOptions::default(),
        )
        .unwrap();
        let c = crate::compile(
            &src,
            &layers,
            &lib,
            "library/templates/IfcBoiler",
            &opo_recipe::NoNesting,
            "2026-11-24",
        )
        .unwrap();
        let a = c.base.data[0].attributes.as_ref().unwrap();
        assert_eq!(a["opo::object"]["kind"], json!("class_template"));
        assert_eq!(
            a["opo::object"]["references"][0]["license"],
            json!("buildingSMART")
        );
    }

    #[test]
    fn unknown_class_or_archetype_is_an_error() {
        let (s, ifc) = fixture();
        assert!(
            generate(
                "IfcFoo",
                &s.classes["IfcBoiler"],
                &s,
                &ifc,
                &GeneratorOptions::default()
            )
            .is_err()
        );
        let bad = json!({"archetype": "linear", "name": "x", "description": "d", "default_type": "WATER"});
        assert!(generate("IfcBoiler", &bad, &s, &ifc, &GeneratorOptions::default()).is_err());
    }
}
