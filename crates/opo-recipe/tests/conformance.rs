//! Runs `conformance/objects/*.json` of the pinned spec: for every case, the derived values,
//! checks, IFC properties, quantities, ports, and the solid count and volume of every item of
//! `model.medium`, `model.high` and `void`, and expressions over data steps (`steps`).

use std::path::{Path, PathBuf};

use opo_feel::Value;
use opo_ifcx::{compose, load_object_dir};
use opo_model::TypeDefinition;
use opo_recipe::{Evaluation, Input, Resolver, run};
use serde_json::{Map, Value as Json};

fn spec_dir() -> PathBuf {
    std::env::var_os("OPO_SPEC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../opo-spec"))
}

fn load(dir: &Path) -> TypeDefinition {
    let obj = load_object_dir(dir).unwrap_or_else(|e| panic!("{e}"));
    let c = compose(std::slice::from_ref(&obj.base)).unwrap();
    let t = c.type_node().expect("type node");
    TypeDefinition::from_attributes(&c.node(t).unwrap().attributes)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
}

/// Object ids `opo-format.org/<path>` are folders of the standard repository.
struct SpecResolver(PathBuf);

impl Resolver for SpecResolver {
    fn resolve(&self, id: &str) -> Option<TypeDefinition> {
        id.strip_prefix("opo-format.org/")
            .map(|p| load(&self.0.join(p)))
    }
}

/// Numbers within 1e-9 (relative 1e-9 above 1); lists and objects item by item.
fn close(got: &Json, want: &Json) -> bool {
    match (got, want) {
        (Json::Number(a), Json::Number(b)) => {
            let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            (a - b).abs() <= 1e-9 * b.abs().max(1.0)
        }
        (Json::Array(a), Json::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| close(x, y))
        }
        (Json::Object(a), Json::Object(b)) => {
            a.len() == b.len() && a.iter().all(|(k, x)| b.get(k).is_some_and(|y| close(x, y)))
        }
        _ => got == want,
    }
}

fn obj(v: Option<&Json>) -> Map<String, Json> {
    v.and_then(Json::as_object).cloned().unwrap_or_default()
}

fn cmp(out: &mut Vec<String>, what: String, got: Json, want: &Json) {
    if !close(&got, want) {
        out.push(format!("{what}: want {want}, got {got}"));
    }
}

fn check_case(def: &TypeDefinition, ev: &Evaluation, expected: &Json, out: &mut Vec<String>) {
    for (name, want) in obj(expected.get("derived")) {
        let got = opo_feel::Env::get(&ev.env, &name)
            .map(Value::to_json)
            .unwrap_or(Json::Null);
        cmp(out, format!("derived {name}"), got, &want);
    }
    for c in def.checks() {
        let want = &expected["checks"][&c.id];
        let got = ev
            .eval(&c.expr)
            .map(|v| v.to_json())
            .unwrap_or_else(|e| Json::String(e.to_string()));
        cmp(out, format!("check {}", c.id), got, want);
    }
    for (k, want) in obj(expected.get("ifc_properties")) {
        let expr = &def.ifc.properties.as_ref().unwrap()[&k];
        let got = match expr {
            Json::String(s) => ev
                .eval(s)
                .map(|v| v.to_json())
                .unwrap_or_else(|e| Json::String(e.to_string())),
            lit => lit.clone(),
        };
        cmp(out, format!("property {k}"), got, &want);
    }
    let qsets = def.ifc.quantity_sets().unwrap();
    for (k, want) in obj(expected.get("quantities")) {
        let (set, name) = k.split_once('.').unwrap();
        let qs = &qsets.iter().find(|(n, _)| n == set).unwrap().1;
        let got = ev
            .eval(qs.values[name].as_str().unwrap())
            .map(|v| v.to_json())
            .unwrap_or(Json::Null);
        cmp(out, format!("quantity {k}"), got, &want);
    }
    let want_ports = obj(expected.get("ports"));
    for p in def.ports.iter().flatten() {
        if let Some(w) = &p.when {
            if !matches!(ev.eval(w), Ok(Value::Bool(true))) {
                if want_ports.contains_key(&p.name) {
                    out.push(format!("port {} should be active", p.name));
                }
                continue;
            }
        }
        let Some(want) = want_ports.get(&p.name) else {
            out.push(format!("port {} should not be active", p.name));
            continue;
        };
        for (field, expr) in [
            ("nominal_size", &p.nominal_size),
            ("position", &p.position),
            ("direction", &p.direction),
        ] {
            let got = ev.eval(expr).map(|v| v.to_json()).unwrap_or(Json::Null);
            cmp(out, format!("port {}.{field}", p.name), got, &want[field]);
        }
    }
    // Data steps (a grid): FEEL expressions over the step results and the environment.
    if let Some(Json::Array(steps)) = expected.get("steps") {
        let mut scope = ev.env.clone();
        for (k, v) in &ev.geometry {
            scope.set(k.clone(), v.clone());
        }
        for st in steps {
            let expr = st["expr"].as_str().unwrap();
            let got = opo_feel::eval_str(expr, &scope)
                .map(|v| v.to_json())
                .unwrap_or_else(|e| Json::String(e.to_string()));
            cmp(out, format!("step expression {expr}"), got, &st["expected"]);
        }
    }
    for (rep, items) in obj(expected.get("geometry")) {
        for item in items.as_array().unwrap() {
            let step = item["geometry"].as_str().unwrap();
            let solids: Vec<_> = match ev.items(step) {
                Ok(g) => g.into_iter().filter_map(|g| g.solid().cloned()).collect(),
                Err(e) => {
                    out.push(format!("{rep} {step}: {e}"));
                    continue;
                }
            };
            let (want_n, want_v, tol) = (
                item["solids"].as_u64().unwrap() as usize,
                item["volume"].as_f64().unwrap(),
                item["tolerance"].as_f64().unwrap(),
            );
            let got_v: f64 = solids.iter().map(|s| s.volume()).sum();
            if solids.len() != want_n {
                out.push(format!(
                    "{rep} {step}: {} solids, want {want_n}",
                    solids.len()
                ));
            }
            let ok = if want_v == 0.0 {
                got_v.abs() < 1e-12
            } else {
                (got_v - want_v).abs() <= tol * want_v.abs()
            };
            if !ok {
                out.push(format!(
                    "{rep} {step}: volume {got_v}, want {want_v} (±{tol} relative)"
                ));
            }
        }
    }
}

#[test]
fn object_cases() {
    let spec = spec_dir();
    let suite = spec.join("conformance/objects");
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
    let resolver = SpecResolver(spec.join("conformance/fixtures"));

    let (mut total, mut failed, mut report) = (0, 0, Vec::new());
    for fx in &fixtures {
        let file: Json = serde_json::from_str(&std::fs::read_to_string(fx).unwrap()).unwrap();
        let def = load(&suite.join(file["object"].as_str().unwrap()));
        let fname = fx.file_name().unwrap().to_string_lossy().into_owned();
        for case in file["cases"].as_array().unwrap() {
            total += 1;
            let input = Input {
                values: obj(case.get("values")),
                inputs: obj(case.get("inputs")),
                context: obj(case.get("context")),
            };
            let mut problems = Vec::new();
            match run(&def, &input, &resolver) {
                Ok(ev) => check_case(&def, &ev, &case["expected"], &mut problems),
                Err(e) => problems.push(format!("run failed: {e}")),
            }
            if !problems.is_empty() {
                failed += 1;
                if report.len() < 40 {
                    report.push(format!(
                        "{fname} {}:\n    {}",
                        case["id"].as_str().unwrap(),
                        problems.join("\n    ")
                    ));
                }
            }
        }
    }
    assert!(
        failed == 0,
        "{failed} of {total} object cases fail:\n{}",
        report.join("\n")
    );
    assert!(total >= 265, "{total} cases: some suites are missing");
    println!("{total} object cases pass");
}
