//! Compare two object folders (e.g. a rebuilt object with the committed one).

use std::path::Path;

use opo_ifcx::{Composition, compose, load_object_dir};
use serde_json::Value as Json;

/// What [`compare_dirs`] leaves out.
#[derive(Debug, Clone, Default)]
pub struct CompareOptions {
    /// Ignore the `generator` entry of `opo::object.references` (its name changes with the
    /// tool that generated the object).
    pub ignore_generator: bool,
}

fn close(a: &Json, b: &Json) -> bool {
    match (a, b) {
        (Json::Number(x), Json::Number(y)) => {
            let (x, y) = (
                x.as_f64().unwrap_or(f64::NAN),
                y.as_f64().unwrap_or(f64::NAN),
            );
            (x - y).abs() <= 1e-9 * y.abs().max(1.0)
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

/// The first place two values differ, as `path: a vs b`.
fn first_diff(a: &Json, b: &Json, path: &str) -> String {
    let short = |v: &Json| {
        let s = v.to_string();
        if s.len() > 120 {
            format!(
                "{}…",
                &s[..s.char_indices().nth(120).map_or(s.len(), |(i, _)| i)]
            )
        } else {
            s
        }
    };
    match (a, b) {
        (Json::Object(x), Json::Object(y)) => {
            let mut keys: Vec<&String> = x.keys().chain(y.keys()).collect();
            keys.sort();
            keys.dedup();
            for k in keys {
                match (x.get(k), y.get(k)) {
                    (Some(p), Some(q)) if !close(p, q) => {
                        return first_diff(p, q, &format!("{path}.{k}"));
                    }
                    (Some(_), None) => return format!("{path}.{k} only in the first"),
                    (None, Some(_)) => return format!("{path}.{k} only in the second"),
                    _ => {}
                }
            }
            format!("{path}: key order differs")
        }
        (Json::Array(x), Json::Array(y)) if x.len() == y.len() => {
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                if !close(p, q) {
                    return first_diff(p, q, &format!("{path}[{i}]"));
                }
            }
            format!("{path}: equal")
        }
        (Json::Array(x), Json::Array(y)) => format!("{path}: {} items vs {}", x.len(), y.len()),
        _ => format!("{path}: {} vs {}", short(a), short(b)),
    }
}

fn type_node(c: &Composition) -> Option<String> {
    c.type_node().map(str::to_owned)
}

fn without_generator(v: &Json) -> Json {
    let mut v = v.clone();
    if let Some(refs) = v.get_mut("references").and_then(Json::as_array_mut) {
        refs.retain(|r| r.get("role").and_then(Json::as_str) != Some("generator"));
    }
    v
}

/// Compare two unpacked object folders: the type node's `opo::` and `bsi::` attributes,
/// parameter order, the names of its children (baked parts), and every layer node by id
/// (inheritance and attributes). Numbers compare within 1e-9. Returns the differences.
pub fn compare_dirs(a: &Path, b: &Path, opts: &CompareOptions) -> Result<Vec<String>, String> {
    let (oa, ob) = (
        load_object_dir(a).map_err(|e| e.to_string())?,
        load_object_dir(b).map_err(|e| e.to_string())?,
    );
    let (ca, cb) = (
        compose(std::slice::from_ref(&oa.base)).map_err(|e| e.to_string())?,
        compose(std::slice::from_ref(&ob.base)).map_err(|e| e.to_string())?,
    );
    let mut out = Vec::new();
    let (Some(ta), Some(tb)) = (type_node(&ca), type_node(&cb)) else {
        return Err("both folders need a type node".into());
    };
    if ta != tb {
        out.push(format!("type node id {ta} vs {tb}"));
    }
    let (na, nb) = (ca.node(&ta).expect("exists"), cb.node(&tb).expect("exists"));
    let mut keys: Vec<&String> = na
        .attributes
        .keys()
        .chain(nb.attributes.keys())
        .filter(|k| k.starts_with("opo::") || k.starts_with("bsi::"))
        .collect();
    keys.sort();
    keys.dedup();
    for k in keys {
        let (mut x, mut y) = (na.attributes.get(k).cloned(), nb.attributes.get(k).cloned());
        if k == "opo::object" && opts.ignore_generator {
            x = x.map(|v| without_generator(&v));
            y = y.map(|v| without_generator(&v));
        }
        match (x, y) {
            (Some(p), Some(q)) if !close(&p, &q) => out.push(first_diff(&p, &q, k)),
            (Some(_), None) => out.push(format!("{k} only in the first")),
            (None, Some(_)) => out.push(format!("{k} only in the second")),
            _ => {}
        }
    }
    let names = |n: &opo_ifcx::ComposedNode| -> Vec<String> {
        n.attributes
            .get("opo::parameters")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
            .filter_map(|p| p["name"].as_str().map(str::to_owned))
            .collect()
    };
    if names(na) != names(nb) {
        out.push("parameter order differs".into());
    }
    let (ka, kb): (Vec<&String>, Vec<&String>) =
        (na.children.keys().collect(), nb.children.keys().collect());
    if ka != kb {
        out.push(format!("children differ: {ka:?} vs {kb:?}"));
    }
    for (file, da) in &oa.layers {
        let Some((_, db)) = ob.layers.iter().find(|(f, _)| f == file) else {
            out.push(format!("layer {file} only in the first"));
            continue;
        };
        let (la, lb) = (
            compose(std::slice::from_ref(da)).map_err(|e| e.to_string())?,
            compose(std::slice::from_ref(db)).map_err(|e| e.to_string())?,
        );
        for p in la.paths().iter().chain(lb.paths()) {
            match (la.node(p), lb.node(p)) {
                (Some(x), Some(y)) => {
                    if x.inherits != y.inherits {
                        out.push(format!("{file}: {p} inherits differently"));
                    }
                    let (ja, jb) = (
                        Json::Object(x.attributes.clone()),
                        Json::Object(y.attributes.clone()),
                    );
                    if !close(&ja, &jb) {
                        out.push(format!("{file}: {}", first_diff(&ja, &jb, p)));
                    }
                }
                (Some(_), None) => out.push(format!("{file}: node {p} only in the first")),
                (None, Some(_)) => out.push(format!("{file}: node {p} only in the second")),
                (None, None) => {}
            }
        }
    }
    for (file, _) in &ob.layers {
        if !oa.layers.iter().any(|(f, _)| f == file) {
            out.push(format!("layer {file} only in the second"));
        }
    }
    out.dedup();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn first_difference_is_named() {
        assert_eq!(
            first_diff(&json!({"a": {"b": 1}}), &json!({"a": {"b": 2}}), "x"),
            "x.a.b: 1 vs 2"
        );
        assert_eq!(
            first_diff(&json!([1, 2]), &json!([1]), "x"),
            "x: 2 items vs 1"
        );
        assert!(close(&json!(0.1 + 0.2), &json!(0.3)));
    }

    #[test]
    fn generator_reference_can_be_ignored() {
        let v =
            json!({"references": [{"role": "generator", "name": "a"}, {"role": "IFC parameters"}]});
        assert_eq!(
            without_generator(&v)["references"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}
