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
    let spec = spec_dir();
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
    let spec = spec_dir();
    let out = opo()
        .args(["check", "--root"])
        .arg(&spec)
        .arg(spec.join("examples/door/single-swing-door"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stdout).contains("baked mesh differs"));
}
