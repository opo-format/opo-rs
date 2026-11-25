//! Reads every object of the conformance suite (pinned spec), composes it with each of its
//! layers, and checks that the variant values it resolves are the `values` of the matching
//! object cases.

use std::path::{Path, PathBuf};

use opo_ifcx::{Document, compose, layer_overlaps, load_object_dir};
use serde_json::Value as Json;

fn spec_dir() -> PathBuf {
    std::env::var_os("OPO_SPEC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../opo-spec"))
}

/// Case id of a variant: `<layer file stem>-<TypeName>`, lower case, runs of other characters
/// replaced by `-` (as the suite's generator names them).
fn slug(stem: &str, type_name: &str) -> String {
    let mut out = String::new();
    for c in format!("{stem}-{type_name}").to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

fn same(a: &Json, b: &Json) -> bool {
    match (a, b) {
        (Json::Number(x), Json::Number(y)) => {
            (x.as_f64().unwrap() - y.as_f64().unwrap()).abs() < 1e-12
        }
        (Json::Array(x), Json::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q))
        }
        (Json::Object(x), Json::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, p)| y.get(k).is_some_and(|q| same(p, q)))
        }
        _ => a == b,
    }
}

#[test]
fn objects_compose_and_variants_resolve() {
    let suite = spec_dir().join("conformance/objects");
    let mut fixtures: Vec<PathBuf> = std::fs::read_dir(&suite)
        .unwrap_or_else(|e| {
            panic!(
                "{}: {e}; run `git submodule update --init opo-spec`",
                suite.display()
            )
        })
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    fixtures.sort();
    assert!(
        fixtures.len() >= 10,
        "the door, nine templates and the grid wall"
    );

    let (mut variants, mut failures) = (0, Vec::new());
    for fx in &fixtures {
        let spec: Json = serde_json::from_str(&std::fs::read_to_string(fx).unwrap()).unwrap();
        let dir: PathBuf = Path::new(&suite).join(spec["object"].as_str().unwrap());
        let obj = load_object_dir(&dir).unwrap_or_else(|e| panic!("{e}"));
        let name = fx.file_name().unwrap().to_string_lossy().into_owned();

        // The base alone: one type node, no dangling links, and it writes back unchanged.
        let base = compose(std::slice::from_ref(&obj.base)).unwrap();
        if base.type_node().is_none() {
            failures.push(format!("{name}: no type node"));
        }
        let text = std::fs::read_to_string(dir.join("object.ifcx")).unwrap();
        let reread = Document::from_json(&obj.base.to_json(), "written").unwrap();
        if reread != obj.base
            || serde_json::from_str::<Json>(&text).unwrap()
                != serde_json::to_value(&obj.base).unwrap()
        {
            failures.push(format!("{name}: object.ifcx changes on write"));
        }

        for o in layer_overlaps(&obj.layers) {
            failures.push(format!(
                "{name}: layers {} and {} both set {} on {}",
                o.layers.0, o.layers.1, o.attribute, o.path
            ));
        }
        for (file, _) in &obj.layers {
            let c = obj.compose_with(&[file]).unwrap();
            for (from, to) in c.dangling() {
                failures.push(format!("{name}: {file}: {from} links to missing {to}"));
            }
            let stem = file.trim_end_matches(".ifcx");
            let layer = &obj.layers.iter().find(|(n, _)| n == file).unwrap().1;
            for node in &layer.data {
                let attrs = node.attributes.clone().unwrap_or_default();
                let Some(Json::String(type_name)) = attrs.get("bsi::ifc::prop::TypeName") else {
                    continue;
                };
                // Variants inherit a system or the type; the type node itself does not.
                if c.node(&node.path).is_none_or(|n| n.inherits.is_empty()) {
                    continue;
                }
                let id = slug(stem, type_name);
                let Some(case) = spec["cases"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|k| k["id"] == id.as_str())
                else {
                    failures.push(format!("{name}: no case {id}"));
                    continue;
                };
                variants += 1;
                let got = Json::Object(c.merged_attribute(&node.path, "opo::values"));
                let want = case
                    .get("values")
                    .cloned()
                    .unwrap_or(Json::Object(Default::default()));
                if !same(&got, &want) {
                    failures.push(format!("{name}: {id}\n  want {want}\n  got  {got}"));
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
    assert!(variants >= 20, "only {variants} variant cases matched");
    println!(
        "{} objects compose; {variants} variants resolve to their case values",
        fixtures.len()
    );
}
