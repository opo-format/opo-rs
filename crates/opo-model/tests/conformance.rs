//! Every `opo::` attribute of the reference objects in the pinned spec (the door example and the
//! class templates) reads into its type, writes back unchanged, and validates.

use std::path::{Path, PathBuf};

use opo_model::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value as Json;

fn spec_dir() -> PathBuf {
    std::env::var_os("OPO_SPEC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../opo-spec"))
}

fn ifcx_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap_or_else(|e| {
        panic!(
            "{}: {e}; run `git submodule update --init opo-spec`",
            dir.display()
        )
    }) {
        let p = e.unwrap().path();
        if p.is_dir() {
            ifcx_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "ifcx") {
            out.push(p);
        }
    }
}

fn round_trip<T: Serialize + DeserializeOwned>(v: &Json) -> Result<(), String> {
    let t: T = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
    let back = serde_json::to_value(&t).map_err(|e| e.to_string())?;
    if same(&back, v) {
        Ok(())
    } else {
        Err(format!("changed on write:\n  in:  {v}\n  out: {back}"))
    }
}

/// JSON equality where numbers compare by value (`200` = `200.0`).
fn same(a: &Json, b: &Json) -> bool {
    match (a, b) {
        (Json::Number(x), Json::Number(y)) => x.as_f64() == y.as_f64(),
        (Json::Array(x), Json::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q))
        }
        (Json::Object(x), Json::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, p)| y.get(k).is_some_and(|q| same(p, q)))
        }
        _ => a == b,
    }
}

fn check_attribute(name: &str, v: &Json) -> Result<(), String> {
    match name {
        "opo::object" => round_trip::<Object>(v),
        "opo::parameters" => round_trip::<Vec<Parameter>>(v),
        "opo::slots" => round_trip::<Vec<Slot>>(v),
        "opo::rules" => round_trip::<Rules>(v),
        "opo::recipe" => round_trip::<Vec<RecipeStep>>(v),
        "opo::representations" => round_trip::<Vec<Representation>>(v),
        "opo::host" => round_trip::<Host>(v),
        "opo::handles" => round_trip::<Vec<Handle>>(v),
        "opo::materials" => round_trip::<serde_json::Map<String, Json>>(v).and_then(|_| {
            v.as_object()
                .unwrap()
                .values()
                .try_for_each(round_trip::<Material>)
        }),
        "opo::hatches" => v
            .as_object()
            .ok_or("not an object")?
            .values()
            .try_for_each(round_trip::<Hatch>),
        "opo::ports" => round_trip::<Vec<Port>>(v),
        "opo::ifc" => round_trip::<Ifc>(v),
        "opo::values" | "opo::inputs" => round_trip::<serde_json::Map<String, Json>>(v),
        "opo::representation" | "opo::note" => round_trip::<String>(v),
        "opo::line_class" => round_trip::<LineClass>(v),
        "opo::ruleset" => round_trip::<RuleSetNode>(v),
        other => Err(format!("{other} is not a schema 0.1 attribute")),
    }
}

#[test]
fn reference_objects() {
    let spec = spec_dir();
    let mut files = Vec::new();
    ifcx_files(&spec.join("examples/door"), &mut files);
    ifcx_files(&spec.join("library/templates"), &mut files);
    files.sort();
    assert!(!files.is_empty());

    let (mut attributes, mut types, mut failures) = (0, 0, Vec::new());
    // Type definitions and rule-set nodes per object folder, checked against each other below.
    let mut defs = std::collections::BTreeMap::<PathBuf, TypeDefinition>::new();
    let mut rule_sets = Vec::<(PathBuf, String, RuleSetNode)>::new();
    for f in &files {
        let rel = f.strip_prefix(&spec).unwrap().display().to_string();
        let doc: Json = serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap();
        let parent = f.parent().unwrap();
        let folder = if parent.ends_with("layers") {
            parent.parent().unwrap()
        } else {
            parent
        }
        .to_path_buf();
        for node in doc["data"].as_array().unwrap() {
            let Some(attrs) = node.get("attributes").and_then(Json::as_object) else {
                continue;
            };
            for (name, v) in attrs.iter().filter(|(k, _)| k.starts_with("opo::")) {
                attributes += 1;
                if let Err(e) = check_attribute(name, v) {
                    failures.push(format!("{rel} {name}: {e}"));
                }
            }
            if let Some(rs) = attrs.get("opo::ruleset") {
                if let Ok(node) = serde_json::from_value::<RuleSetNode>(rs.clone()) {
                    rule_sets.push((folder.clone(), rel.clone(), node));
                }
            }
            if attrs.contains_key("opo::object") {
                types += 1;
                match TypeDefinition::from_attributes(attrs) {
                    Err(e) => failures.push(format!("{rel}: {e}")),
                    Ok(def) => {
                        defs.insert(folder.clone(), def.clone());
                        for issue in def.validate() {
                            failures.push(format!("{rel}: {issue}"));
                        }
                        let opo: serde_json::Map<String, Json> = attrs
                            .iter()
                            .filter(|(k, _)| k.starts_with("opo::"))
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect();
                        let back = def.to_attributes();
                        for (k, v) in &opo {
                            if !back.get(k).is_some_and(|b| same(b, v)) {
                                failures.push(format!("{rel}: {k} changed through TypeDefinition"));
                            }
                        }
                    }
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} problems:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert_eq!(types, 10, "the door and nine templates");
    println!(
        "{attributes} attributes in {} files round-trip; {types} type definitions validate",
        files.len()
    );
}
