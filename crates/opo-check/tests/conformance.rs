//! Checks the door and the nine class templates of the pinned spec. They are valid, and the
//! number of evaluation cases and volume checks per template matches the prototype checker
//! (`tools/check_opo.py`). Broken copies are reported.

use std::path::{Path, PathBuf};

use opo_check::{DirResolver, IfcTemplates, Options, check_dir};

fn spec_dir() -> PathBuf {
    std::env::var_os("OPO_SPEC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../opo-spec"))
}

/// The IFC property set table written by `just ifc` (or `$OPO_IFC_DIR/psets.json`).
fn ifc_psets() -> PathBuf {
    let p = std::env::var_os("OPO_IFC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/ifc"))
        .join("psets.json");
    assert!(p.exists(), "{} is missing; run `just ifc`", p.display());
    p
}

fn options(spec: &Path) -> (DirResolver, IfcTemplates) {
    (
        DirResolver {
            prefix: "opo-format.org/".into(),
            root: spec.to_path_buf(),
        },
        IfcTemplates::read(&ifc_psets()).expect("IFC templates"),
    )
}

#[test]
fn reference_objects_are_valid() {
    let spec = spec_dir().join("conformance/fixtures");
    let (resolver, ifc) = options(&spec);
    let opts = Options {
        resolver: &resolver,
        ifc: Some(&ifc),
    };
    // (template, cases, volume checks) as reported by tools/check_opo.py
    let expected = [
        ("IfcBeam", 29, 29),
        ("IfcBoiler", 6, 0),
        ("IfcElementAssembly", 32, 0),
        ("IfcMember", 33, 33),
        ("IfcOpeningElement", 8, 0),
        ("IfcPipeFitting", 10, 0),
        ("IfcSlab", 19, 19),
        ("IfcStair", 20, 0),
        ("IfcWall", 27, 0),
    ];
    let mut problems = Vec::new();
    for (name, cases, volumes) in expected {
        let r = check_dir(&spec.join("library/templates").join(name), &opts);
        problems.extend(r.errors.iter().map(|e| format!("{name}: {e}")));
        if (r.cases, r.volume_checks) != (cases, volumes) {
            problems.push(format!(
                "{name}: {} cases, {} volume checks; want {cases}, {volumes}",
                r.cases, r.volume_checks
            ));
        }
    }
    let door = check_dir(&spec.join("examples/door/single-swing-door"), &opts);
    problems.extend(door.errors.iter().map(|e| format!("door: {e}")));
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Copy an object folder to a temporary place.
fn copy(src: &Path, name: &str) -> PathBuf {
    let dst = std::env::temp_dir().join(format!("opo-check-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    std::fs::create_dir_all(dst.join("layers")).unwrap();
    for e in std::fs::read_dir(src)
        .unwrap()
        .chain(std::fs::read_dir(src.join("layers")).into_iter().flatten())
    {
        let p = e.unwrap().path();
        if p.is_file() {
            let rel = p.strip_prefix(src).unwrap();
            std::fs::copy(&p, dst.join(rel)).unwrap();
        }
    }
    dst
}

fn edit(dir: &Path, f: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>)) {
    let path = dir.join("object.ifcx");
    let mut doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let node = doc["data"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|n| n["attributes"].get("opo::object").is_some())
        .unwrap();
    f(node["attributes"].as_object_mut().unwrap());
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();
}

#[test]
fn broken_objects_are_reported() {
    let spec = spec_dir().join("conformance/fixtures");
    let (resolver, ifc) = options(&spec);
    let opts = Options {
        resolver: &resolver,
        ifc: Some(&ifc),
    };
    let src = spec.join("library/templates/IfcBoiler");

    let d = copy(&src, "names");
    edit(&d, |a| {
        a["opo::rules"]["checks"][0]["expr"] = "no_such_name > 0".into()
    });
    let r = check_dir(&d, &opts);
    assert!(
        r.errors
            .iter()
            .any(|e| e.contains("unknown names [\"no_such_name\"]")),
        "{:?}",
        r.errors
    );

    let d = copy(&src, "ifc");
    edit(&d, |a| {
        a["opo::ifc"]["properties"] =
            serde_json::json!({"Pset_BoilerTypeCommon.NoSuchProperty": "1"});
    });
    let r = check_dir(&d, &opts);
    assert!(
        r.errors.iter().any(|e| e.contains("NoSuchProperty")),
        "{:?}",
        r.errors
    );

    let d = copy(&src, "attr");
    edit(&d, |a| {
        a.insert("opo::colour".into(), "red".into());
    });
    let r = check_dir(&d, &opts);
    assert!(
        r.errors
            .iter()
            .any(|e| e.contains("opo::colour is not declared")),
        "{:?}",
        r.errors
    );

    let d = copy(&src, "baked");
    let path = d.join("object.ifcx");
    let text = std::fs::read_to_string(&path).unwrap();
    let mut doc: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mesh = doc["data"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find_map(|n| n["attributes"].get_mut("usd::usdgeom::mesh"))
        .unwrap();
    mesh["points"][0][0] = serde_json::json!(99.0);
    std::fs::write(&path, doc.to_string()).unwrap();
    let r = check_dir(&d, &opts);
    assert!(
        r.errors.iter().any(|e| e.contains("baked mesh differs")),
        "{:?}",
        r.errors
    );
}

#[test]
fn provenance_is_checked_and_to_verify_reported() {
    let spec = spec_dir().join("conformance/fixtures");
    let (resolver, ifc) = options(&spec);
    let opts = Options {
        resolver: &resolver,
        ifc: Some(&ifc),
    };
    let src = spec.join("library/templates/IfcBoiler");
    let first = |a: &serde_json::Map<String, serde_json::Value>| {
        a["opo::parameters"][0]["name"].as_str().unwrap().to_owned()
    };

    let d = copy(&src, "provenance-ok");
    edit(&d, |a| {
        let p = first(a);
        a.insert(
            "opo::provenance".into(),
            serde_json::json!({
                format!("parameters.{p}.default"): {"source": "x", "status": "to_verify", "note": "Check."}
            }),
        );
    });
    let r = check_dir(&d, &opts);
    assert!(r.ok(), "{:?}", r.errors);
    assert_eq!(r.to_verify.len(), 1);

    let d = copy(&src, "provenance-bad");
    edit(&d, |a| {
        a.insert(
            "opo::provenance".into(),
            serde_json::json!({
                "parameters.no_such_parameter.default": {"status": "norm", "source": "x"},
                "values.width": {"status": "maker"}
            }),
        );
    });
    let r = check_dir(&d, &opts);
    for want in ["no_such_parameter.default' on", "needs a source"] {
        assert!(
            r.errors.iter().any(|e| e.contains(want)),
            "{want}: {:?}",
            r.errors
        );
    }
}
