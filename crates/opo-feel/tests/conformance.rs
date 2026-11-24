//! Runs `conformance/feel/cases.json` of the pinned spec version (submodule `opo-spec`, or the
//! directory in `OPO_SPEC_DIR`).

use std::path::PathBuf;

use opo_feel::{Scope, Value, eval_str};

fn spec_dir() -> PathBuf {
    std::env::var_os("OPO_SPEC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../opo-spec"))
}

#[test]
fn feel_cases() {
    let path = spec_dir().join("conformance/feel/cases.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "cannot read {} ({e}); run `git submodule update --init opo-spec`",
            path.display()
        )
    });
    let suite: serde_json::Value = serde_json::from_str(&text).expect("cases.json is JSON");
    let cases = suite["cases"].as_array().expect("cases is a list");
    assert!(!cases.is_empty());

    let mut failures = Vec::new();
    for case in cases {
        let id = case["id"].as_str().unwrap_or("?");
        let expr = case["expr"].as_str().expect("expr is a string");
        let scope = case
            .get("context")
            .map(|c| Scope::from_context(&Value::from_json(c)))
            .unwrap_or_default();
        let expected = Value::from_json(&case["expected"]);
        match eval_str(expr, &scope) {
            Ok(got) if got == expected => {}
            Ok(got) => failures.push(format!(
                "{id}: {expr}\n  expected {expected:?}\n  got      {got:?}"
            )),
            Err(e) => failures.push(format!("{id}: {expr}\n  error: {e}")),
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} FEEL cases fail:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
    println!("{} FEEL cases pass", cases.len());
}
