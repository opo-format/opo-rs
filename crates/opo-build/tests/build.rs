//! Baking and packaging against the objects of the pinned spec.

use std::path::{Path, PathBuf};

use opo_build::{bake, pack, unpack};
use opo_check::{DirResolver, IfcTemplates, Options, check_dir};
use opo_ifcx::Document;

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

fn copy_dir(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).unwrap();
    for e in std::fs::read_dir(src).unwrap() {
        let p = e.unwrap().path();
        let t = dst.join(p.file_name().unwrap());
        if p.is_dir() {
            copy_dir(&p, &t);
        } else {
            std::fs::copy(&p, &t).unwrap();
        }
    }
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("opo-build-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn children(doc: &Document) -> serde_json::Map<String, serde_json::Value> {
    doc.data
        .iter()
        .find(|n| {
            n.attributes
                .as_ref()
                .is_some_and(|a| a.contains_key("opo::object"))
        })
        .and_then(|n| n.children.clone())
        .unwrap_or_default()
}

#[test]
fn rebaked_templates_keep_their_nodes_and_pass_the_check() {
    let spec = spec_dir().join("conformance/fixtures");
    let resolver = DirResolver {
        prefix: "opo-format.org/".into(),
        root: spec.clone(),
    };
    let ifc = IfcTemplates::read(&ifc_psets()).unwrap();
    let opts = Options {
        resolver: &resolver,
        ifc: Some(&ifc),
    };
    let mut problems = Vec::new();
    for e in std::fs::read_dir(spec.join("library/templates")).unwrap() {
        let src = e.unwrap().path();
        if !src.join("object.ifcx").exists() {
            continue;
        }
        let name = src.file_name().unwrap().to_string_lossy().into_owned();
        let dst = temp(&name);
        copy_dir(&src, &dst);
        let original = Document::read(&src.join("object.ifcx")).unwrap();
        let mut doc = original.clone();
        let report = bake(&mut doc, &resolver, &format!("library/templates/{name}")).unwrap();
        if children(&doc) != children(&original) {
            problems.push(format!(
                "{name}: baked child nodes differ from the committed ones"
            ));
        }
        if report.removed != report.meshes + report.curves {
            problems.push(format!(
                "{name}: removed {} nodes but wrote {}",
                report.removed,
                report.meshes + report.curves
            ));
        }
        std::fs::write(dst.join("object.ifcx"), doc.to_json()).unwrap();
        let r = check_dir(&dst, &opts);
        problems.extend(r.errors.iter().map(|e| format!("{name} (rebaked): {e}")));
        // Baking is idempotent.
        let mut again = doc.clone();
        bake(&mut again, &resolver, &format!("library/templates/{name}")).unwrap();
        if again != doc {
            problems.push(format!("{name}: baking twice changes the document"));
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn rebaking_the_door_fixes_its_stale_meshes() {
    let spec = spec_dir().join("conformance/fixtures");
    let resolver = DirResolver {
        prefix: "opo-format.org/".into(),
        root: spec.clone(),
    };
    let opts = Options {
        resolver: &resolver,
        ifc: None,
    };
    let src = spec.join("examples/door/single-swing-door");
    let before = check_dir(&src, &opts);
    let dst = temp("door");
    copy_dir(&src, &dst);
    let mut doc = Document::read(&dst.join("object.ifcx")).unwrap();
    let report = bake(&mut doc, &resolver, "examples/door/single-swing-door").unwrap();
    assert!(report.meshes > 0 && report.curves > 0);
    std::fs::write(dst.join("object.ifcx"), doc.to_json()).unwrap();
    let after = check_dir(&dst, &opts);
    assert!(after.ok(), "{:?}", after.errors);
    // Only if the committed door is still stale is there something to fix.
    if !before.ok() {
        assert!(
            before
                .errors
                .iter()
                .all(|e| e.starts_with("baked mesh differs")),
            "{:?}",
            before.errors
        );
    }
}

#[test]
fn pack_and_unpack_round_trip() {
    let spec = spec_dir().join("conformance/fixtures");
    let src = spec.join("library/templates/IfcWall");
    let (a, b) = (temp("pack-a.opo"), temp("pack-b.opo"));
    assert_eq!(pack(&src, &a).unwrap(), 2, "object.ifcx and one layer");
    pack(&src, &b).unwrap();
    assert_eq!(
        std::fs::read(&a).unwrap(),
        std::fs::read(&b).unwrap(),
        "deterministic"
    );
    let out = temp("unpacked");
    assert_eq!(unpack(&a, &out).unwrap(), 2);
    for rel in ["object.ifcx", "layers/types-examples.ifcx"] {
        assert_eq!(
            std::fs::read(src.join(rel)).unwrap(),
            std::fs::read(out.join(rel)).unwrap(),
            "{rel}"
        );
    }
}

#[test]
fn bad_packages_are_refused() {
    let empty = temp("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert!(pack(&empty, &temp("x.opo")).is_err(), "no object.ifcx");

    // A zip without object.ifcx.
    let z = temp("no-object.opo");
    {
        use std::io::Write;
        let mut w = zip::ZipWriter::new(std::fs::File::create(&z).unwrap());
        w.start_file("readme.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(b"hello").unwrap();
        w.finish().unwrap();
    }
    assert!(unpack(&z, &temp("no-object")).is_err());

    // A zip with an entry that escapes the folder.
    let z = temp("escape.opo");
    {
        use std::io::Write;
        let mut w = zip::ZipWriter::new(std::fs::File::create(&z).unwrap());
        w.start_file("object.ifcx", zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(b"{}").unwrap();
        w.start_file("../evil.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        w.write_all(b"x").unwrap();
        w.finish().unwrap();
    }
    let out = temp("escape");
    assert!(unpack(&z, &out).is_err());
    assert!(!out.parent().unwrap().join("evil.txt").exists());
}
