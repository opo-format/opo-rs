//! Golden test (proposal 0002 §5): export the door and the nine class templates of the pinned
//! spec to YAML sources, read them back, compile them, and compare with the committed files.
//! The rebuilt objects, baked, pass `opo check`.

use std::path::{Path, PathBuf};

use opo_check::{DirResolver, IfcTemplates, Options, check_dir};
use opo_ifcx::{compose, load_object_dir};
use opo_source::{Library, compile_dir, export_dir};
use serde_json::Value as Json;

fn spec_dir() -> PathBuf {
    std::env::var_os("OPO_SPEC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../opo-spec"))
}

fn close(a: &Json, b: &Json) -> bool {
    match (a, b) {
        (Json::Number(x), Json::Number(y)) => {
            (x.as_f64().unwrap() - y.as_f64().unwrap()).abs()
                <= 1e-9 * y.as_f64().unwrap().abs().max(1.0)
        }
        (Json::Array(x), Json::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| close(p, q))
        }
        (Json::Object(x), Json::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, p)| y.get(k).is_some_and(|q| close(p, q)))
        }
        _ => a == b,
    }
}

const OBJECTS: [&str; 10] = [
    "examples/door/single-swing-door",
    "library/templates/IfcBeam",
    "library/templates/IfcBoiler",
    "library/templates/IfcElementAssembly",
    "library/templates/IfcMember",
    "library/templates/IfcOpeningElement",
    "library/templates/IfcPipeFitting",
    "library/templates/IfcSlab",
    "library/templates/IfcStair",
    "library/templates/IfcWall",
];

#[test]
fn export_compile_round_trip() {
    let spec = spec_dir();
    let resolver = DirResolver {
        prefix: "opo-format.org/".into(),
        root: spec.clone(),
    };
    let tmp = std::env::temp_dir().join(format!("opo-golden-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);

    // 1. Export every object; one shared library.
    let mut lib = Library::default();
    for base in OBJECTS {
        let ex = export_dir(&spec.join(base), base).unwrap_or_else(|e| panic!("{base}: {e}"));
        lib.merge(&ex.library)
            .unwrap_or_else(|e| panic!("{base}: {e}"));
        ex.write(&tmp.join("sources").join(base)).unwrap();
    }
    lib.save(&tmp.join("sources")).unwrap();
    let lib = Library::load(&tmp.join("sources")).unwrap();

    // 2. Compile from the YAML and compare.
    let mut problems = Vec::new();
    for base in OBJECTS {
        let built = compile_dir(
            &tmp.join("sources").join(base),
            &lib,
            base,
            &resolver,
            "2026-11-24",
        )
        .unwrap_or_else(|e| panic!("{base}: {e}"));
        let orig = load_object_dir(&spec.join(base)).unwrap();
        let oc = compose(std::slice::from_ref(&orig.base)).unwrap();
        let t = oc.type_node().unwrap().to_owned();
        let oa = &oc.node(&t).unwrap().attributes;
        let bc = compose(std::slice::from_ref(&built.base)).unwrap();
        if bc.type_node() != Some(t.as_str()) {
            problems.push(format!("{base}: type node id differs"));
            continue;
        }
        let ba = &bc.node(&t).unwrap().attributes;
        for (k, v) in oa
            .iter()
            .filter(|(k, _)| k.starts_with("opo::") || k.starts_with("bsi::"))
        {
            match ba.get(k) {
                Some(b) if close(b, v) => {}
                Some(b) => problems.push(format!("{base}: {k} differs\n   was {v}\n   now {b}")),
                None => problems.push(format!("{base}: {k} missing")),
            }
        }
        for (k, _) in ba.iter().filter(|(k, _)| !oa.contains_key(*k)) {
            problems.push(format!("{base}: {k} added"));
        }
        // Layers: same files; every node with the same id, inheritance and values.
        for (file, odoc) in &orig.layers {
            let Some((_, bdoc)) = built.layers.iter().find(|(f, _)| f == file) else {
                problems.push(format!("{base}: layer {file} missing"));
                continue;
            };
            let (ol, bl) = (
                compose(std::slice::from_ref(odoc)).unwrap(),
                compose(std::slice::from_ref(bdoc)).unwrap(),
            );
            for p in ol.paths() {
                let o = ol.node(p).unwrap();
                let Some(b) = bl.node(p) else {
                    problems.push(format!("{base}: {file}: node {p} missing"));
                    continue;
                };
                if o.inherits != b.inherits {
                    problems.push(format!("{base}: {file}: {p} inherits differently"));
                }
                for (k, v) in &o.attributes {
                    // A system node is named by Name (the prototype door used TypeName).
                    let k2 = if k == "bsi::ifc::prop::TypeName" && !b.attributes.contains_key(k) {
                        "bsi::ifc::prop::Name"
                    } else {
                        k.as_str()
                    };
                    if !b.attributes.get(k2).is_some_and(|x| close(x, v)) {
                        problems.push(format!(
                            "{base}: {file}: {p} {k}\n   was {v}\n   now {:?}",
                            b.attributes.get(k2)
                        ));
                    }
                }
            }
        }
        // 3. Bake the built object and check it.
        let out = tmp.join("built").join(base);
        std::fs::create_dir_all(out.join("layers")).unwrap();
        let mut doc = built.base.clone();
        opo_build::bake(&mut doc, &resolver, base).unwrap();
        std::fs::write(out.join("object.ifcx"), doc.to_json()).unwrap();
        for (f, d) in &built.layers {
            std::fs::write(out.join("layers").join(f), d.to_json()).unwrap();
        }
        let ifc = IfcTemplates::read(&spec.join("tools/ifc4x3_psets.json")).unwrap();
        let r = check_dir(
            &out,
            &Options {
                resolver: &resolver,
                ifc: Some(&ifc),
            },
        );
        problems.extend(r.errors.iter().map(|e| format!("{base} (built): {e}")));
    }
    assert!(
        problems.is_empty(),
        "{} problems:\n{}",
        problems.len(),
        problems.join("\n")
    );
    let _ = Path::new(&tmp);
}
