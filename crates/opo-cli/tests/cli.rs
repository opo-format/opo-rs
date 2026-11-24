//! The `opo` binary: help, usage errors, and `opo check` on the pinned spec's templates.

use std::path::PathBuf;
use std::process::Command;

fn opo() -> Command {
    Command::new(env!("CARGO_BIN_EXE_opo"))
}

fn spec_dir() -> PathBuf {
    std::env::var_os("OPO_SPEC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../opo-spec"))
}

#[test]
fn help_and_usage_errors() {
    let out = opo().arg("--help").output().unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("opo check"));
    assert_eq!(
        opo().arg("frobnicate").output().unwrap().status.code(),
        Some(2)
    );
    assert_eq!(opo().arg("check").output().unwrap().status.code(), Some(2));
}

#[test]
fn check_templates() {
    let spec = spec_dir().join("conformance/fixtures");
    let out = opo()
        .args(["check", "--root"])
        .arg(&spec)
        .arg(spec.join("library/templates/IfcWall"))
        .arg(spec.join("library/templates/IfcSlab"))
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
    assert!(
        text.contains("IfcWall") && text.contains("27 cases: OK"),
        "{text}"
    );
    assert!(text.contains("volume check 19 cases: OK"), "{text}");
}

#[test]
fn check_reports_problems_with_exit_code_1() {
    let spec = spec_dir().join("conformance/fixtures");
    let tmp = std::env::temp_dir().join(format!("opo-cli-broken-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    copy_dir(&spec.join("library/templates/IfcBoiler"), &tmp);
    let file = tmp.join("object.ifcx");
    let text = std::fs::read_to_string(&file).unwrap().replacen(
        "\"opo::object\"",
        "\"opo::colour\": \"red\", \"opo::object\"",
        1,
    );
    std::fs::write(&file, text).unwrap();
    let out = opo()
        .args(["check", "--root"])
        .arg(&spec)
        .arg(&tmp)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("opo::colour is not declared"));
}

fn copy_dir(src: &std::path::Path, dst: &std::path::Path) {
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

#[test]
fn bake_pack_unpack_check() {
    let spec = spec_dir().join("conformance/fixtures");
    let tmp = std::env::temp_dir().join(format!("opo-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let door = tmp.join("examples/door/single-swing-door");
    copy_dir(&spec.join("examples/door/single-swing-door"), &door);
    let ok = |c: &mut Command| {
        let out = c.output().unwrap();
        assert!(
            out.status.success(),
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    // The base defaults to the folder relative to --root.
    let text = ok(opo().args(["bake", "--root"]).arg(&tmp).arg(&door));
    assert!(text.contains("meshes"), "{text}");
    // Nested objects resolve under the spec; the rebaked door passes the check.
    ok(opo().args(["check", "--root"]).arg(&spec).arg(&door));
    let package = tmp.join("door.opo");
    ok(opo().arg("pack").arg(&door).arg("-o").arg(&package));
    let out = tmp.join("door-unpacked");
    ok(opo().arg("unpack").arg(&package).arg("-o").arg(&out));
    assert_eq!(
        std::fs::read(door.join("object.ifcx")).unwrap(),
        std::fs::read(out.join("object.ifcx")).unwrap()
    );
    // --base is needed outside --root.
    assert_eq!(
        opo()
            .args(["bake", "--root"])
            .arg(&spec)
            .arg(&door)
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
}

#[test]
fn render_plan_and_model() {
    let spec = spec_dir().join("conformance/fixtures");
    let wall = spec.join("library/templates/IfcWall");
    let out = opo()
        .args(["render", "--root"])
        .arg(&spec)
        .arg(&wall)
        .args([
            "--scale",
            "20",
            "--inputs",
            "{\"path\": [[0, 0], [1.2, 0]]}",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("<clipPath"));
    let out = opo()
        .args(["render", "--root"])
        .arg(&spec)
        .arg(&wall)
        .args(["--view", "model"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("<polygon"));
    assert_eq!(
        opo()
            .args(["render"])
            .arg(&wall)
            .args(["--view", "side"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        opo()
            .args(["render"])
            .arg(&wall)
            .args(["--values", "[1]"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
}

#[test]
fn export_then_build() {
    let spec = spec_dir().join("conformance/fixtures");
    let tmp = std::env::temp_dir().join(format!("opo-cli-src-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let lib = tmp.join("lib");
    let ok = |c: &mut Command| {
        let out = c.output().unwrap();
        assert!(
            out.status.success(),
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    };
    ok(opo()
        .arg("export")
        .arg(spec.join("library/templates/IfcWall"))
        .args(["--base", "library/templates/IfcWall", "--library"])
        .arg(&lib)
        .arg("-o")
        .arg(lib.join("templates/IfcWall")));
    assert!(
        lib.join("styles/materials.yaml").exists()
            && lib
                .join("templates/IfcWall/layers/types-examples.yaml")
                .exists()
    );
    let out = tmp.join("out");
    ok(opo()
        .arg("build")
        .arg(lib.join("templates/IfcWall"))
        .arg("--library")
        .arg(&lib)
        .arg("-o")
        .arg(&out)
        .args(["--timestamp", "2026-11-24"]));
    let built = out.join("library/templates/IfcWall");
    assert!(
        built.join("object.ifcx").exists() && built.join("layers/types-examples.ifcx").exists()
    );
    ok(opo().args(["check", "--root"]).arg(&spec).arg(&built));
    ok(opo()
        .arg("build")
        .arg(lib.join("templates/IfcWall"))
        .arg("--library")
        .arg(&lib)
        .arg("-o")
        .arg(&out)
        .arg("--package"));
    assert!(out.join("library/templates/IfcWall.opo").exists());
    assert_eq!(
        opo()
            .arg("build")
            .arg(lib.join("templates/IfcWall"))
            .output()
            .unwrap()
            .status
            .code(),
        Some(2),
        "--library is required"
    );
}

#[test]
fn compare_folders() {
    let spec = spec_dir().join("conformance/fixtures");
    let wall = spec.join("library/templates/IfcWall");
    let out = opo().arg("compare").arg(&wall).arg(&wall).output().unwrap();
    assert!(out.status.success());
    let out = opo()
        .arg("compare")
        .arg(&wall)
        .arg(spec.join("library/templates/IfcSlab"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("type node id"));
}

#[test]
fn ifc_from_uml() {
    let dir = std::env::temp_dir().join(format!("opo-cli-ifc-{}", std::process::id()));
    let (src, out) = (dir.join("schemas"), dir.join("out"));
    std::fs::create_dir_all(&src).unwrap();
    // No .uml files: a usage error.
    let r = opo().args(["ifc"]).arg(&src).output().unwrap();
    assert_eq!(r.status.code(), Some(2));
    std::fs::write(
        src.join("a.uml"),
        r#"<?xml version="1.0"?>
<uml:Model xmlns:xmi="http://www.omg.org/spec/XMI/20131001" xmlns:uml="http://www.eclipse.org/uml2/5.0.0/UML" xmi:id="m" name="m">
  <packagedElement xmi:type="uml:Class" xmi:id="cl_IfcElement" name="IfcElement" isAbstract="true"/>
  <packagedElement xmi:type="uml:Class" xmi:id="cl_IfcBox" name="IfcBox">
    <generalization xmi:type="uml:Generalization" xmi:id="g" general="cl_IfcElement"/>
  </packagedElement>
  <packagedElement xmi:type="uml:Class" xmi:id="cl_Pset_BoxCommon" name="Pset_BoxCommon">
    <ownedAttribute xmi:type="uml:Property" xmi:id="p" name="Label">
      <type xmi:type="uml:DataType" href="m.uml#dt_IfcLabel"/>
    </ownedAttribute>
  </packagedElement>
  <packagedElement xmi:type="uml:Dependency" xmi:id="d" client="cl_Pset_BoxCommon" supplier="cl_IfcBox"/>
</uml:Model>
"#,
    )
    .unwrap();
    let r = opo()
        .arg("ifc")
        .arg(&src)
        .arg("-o")
        .arg(&out)
        .output()
        .unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    let read = |f: &str| -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(out.join(f)).unwrap()).unwrap()
    };
    assert_eq!(read("elements.json")[1][1], "IfcBox");
    assert_eq!(
        read("psets.json")["IfcBox"][0]["props"][0]["measure"],
        "IfcLabel"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn catalogue_and_planned_objects() {
    let root = std::env::temp_dir().join(format!("opo-cli-catalogue-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let obj = root.join("objects/door-double-swing");
    std::fs::create_dir_all(&obj).unwrap();
    std::fs::write(
        obj.join("object.yaml"),
        "id: \"door-double-swing\"\nname: \"Double swing door\"\nstatus: \"planned\"\npriority: \"P2\"\ncatalogue_id: \"O-02\"\nifc:\n  class: \"IfcDoor\"\nsheet:\n  scope: \"Two leaves in one frame.\"\n",
    )
    .unwrap();
    // A planned object is listed but not built.
    let r = opo()
        .arg("build")
        .arg(&obj)
        .arg("--library")
        .arg(&root)
        .output()
        .unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert!(String::from_utf8_lossy(&r.stdout).contains("planned, not built"));
    assert!(
        !root
            .join("build")
            .join("library/objects/door-double-swing")
            .exists()
    );
    let md = root.join("build/catalogue.md");
    let r = opo()
        .args(["catalogue", "--library"])
        .arg(&root)
        .arg("-o")
        .arg(&md)
        .output()
        .unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stdout));
    assert!(
        std::fs::read_to_string(&md)
            .unwrap()
            .contains("| O-02 | Double swing door |")
    );
    // A problem in an entry: exit code 1.
    std::fs::write(
        obj.join("object.yaml"),
        "id: \"door-double-swing\"\nname: \"X\"\nstatus: \"planned\"\n",
    )
    .unwrap();
    let r = opo()
        .args(["catalogue", "--library"])
        .arg(&root)
        .output()
        .unwrap();
    assert_eq!(r.status.code(), Some(1));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn coverage_table() {
    let root = std::env::temp_dir().join(format!("opo-cli-coverage-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("templates")).unwrap();
    std::fs::create_dir_all(root.join("ifc")).unwrap();
    std::fs::write(
        root.join("ifc/elements.json"),
        r#"[[0, "IfcElement", true, null], [1, "IfcBuiltElement", false, null], [2, "IfcWall", false, ["STANDARD"]]]"#,
    )
    .unwrap();
    std::fs::write(
        root.join("templates/classes.yaml"),
        "IfcWall:\n  archetype: \"wall\"\n",
    )
    .unwrap();
    let md = root.join("build/coverage.md");
    let r = opo()
        .args(["coverage", "--library"])
        .arg(&root)
        .arg("--ifc")
        .arg(root.join("ifc"))
        .arg("-o")
        .arg(&md)
        .output()
        .unwrap();
    assert!(r.status.success(), "{}", String::from_utf8_lossy(&r.stderr));
    assert!(
        std::fs::read_to_string(&md)
            .unwrap()
            .contains("| · `IfcWall` | wall | yes | STANDARD | — |")
    );
    // A class that is not a concrete element class: exit code 1.
    std::fs::write(
        root.join("templates/classes.yaml"),
        "IfcFoo:\n  archetype: \"wall\"\n",
    )
    .unwrap();
    let r = opo()
        .args(["coverage", "--library"])
        .arg(&root)
        .arg("--ifc")
        .arg(root.join("ifc"))
        .output()
        .unwrap();
    assert_eq!(r.status.code(), Some(1));
    std::fs::remove_dir_all(&root).unwrap();
}
