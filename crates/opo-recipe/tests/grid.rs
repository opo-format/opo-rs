//! A `grid` step inside a recipe: class rules, overrides, widths and members placed with
//! `for_each` over its segments and cells (proposal 0001 §2).

use std::path::PathBuf;

use opo_feel::Value;
use opo_ifcx::{compose, load_object_dir};
use opo_model::TypeDefinition;
use opo_recipe::{Input, NoNesting, run};
use serde_json::json;

fn spec_dir() -> PathBuf {
    std::env::var_os("OPO_SPEC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../opo-spec"))
}

/// The IfcOpeningElement fixture with its recipe replaced.
fn object(recipe: serde_json::Value) -> TypeDefinition {
    let dir = spec_dir().join("conformance/fixtures/library/templates/IfcOpeningElement");
    let obj = load_object_dir(&dir).unwrap();
    let c = compose(std::slice::from_ref(&obj.base)).unwrap();
    let mut attrs = c.node(c.type_node().unwrap()).unwrap().attributes.clone();
    attrs.insert("opo::recipe".into(), recipe);
    TypeDefinition::from_attributes(&attrs).unwrap()
}

fn recipe() -> serde_json::Value {
    json!([
        {"id": "g", "op": "grid", "args": {
            "surface": "{baseline: [[0, 0], [3, 0]], base: 0, top: 3}",
            "u": "{pattern: [{size: 1, flexible: true}], distribution: \"best_division\"}",
            "v": "{pattern: [{size: 0.8, cls: \"spandrel\"}, {size: 2.2, flexible: true, cls: \"vision\"}], distribution: \"best_division\"}",
            "classes": {"segment": "if segment.direction = \"u\" and segment.line_cls = \"division\" then \"sill\" else segment.cls"},
            "overrides": "edits",
            "widths": "{boundary: 0.1, mullion: 0.05, sill: 0.05}"
        }},
        {"id": "frames", "op": "for_each", "args": {"items": "g.segments", "as": "s",
            "do": "box([min(s.start[1], s.end[1]) - 0.025, -0.05, min(s.start[3], s.end[3]) - 0.025], [max(s.start[1], s.end[1]) + 0.025, 0.05, max(s.start[3], s.end[3]) + 0.025])"}},
        {"id": "panels", "op": "for_each", "args": {"items": "g.cells", "as": "c",
            "do": "box([c.panel_outline[1][1], -0.01, c.panel_outline[1][3]], [c.panel_outline[3][1], 0.01, c.panel_outline[3][3]])"}}
    ])
}

fn grid_value(input: &Input) -> Value {
    let ev = run(&object(recipe()), input, &NoNesting).unwrap();
    ev.geometry["g"].clone()
}

fn list<'a>(v: &'a Value, k: &str) -> &'a Vec<Value> {
    match v {
        Value::Context(c) => match &c[k] {
            Value::List(l) => l,
            _ => panic!("{k}"),
        },
        _ => panic!(),
    }
}

fn get<'a>(v: &'a Value, k: &str) -> &'a Value {
    match v {
        Value::Context(c) => &c[k],
        _ => panic!(),
    }
}

#[test]
fn grid_step_feeds_for_each() {
    let mut input = Input::default();
    input.inputs.insert("edits".into(), json!([]));
    let g = grid_value(&input);
    // 3 columns, 2 rows (spandrel 0.8, vision 2.2).
    assert_eq!(list(&g, "cells").len(), 6);
    assert_eq!(list(&g, "segments").len(), 3 * 3 + 4 * 2);
    let sills = list(&g, "segments")
        .iter()
        .filter(|s| get(s, "cls") == &Value::String("sill".into()))
        .count();
    assert_eq!(sills, 3, "the class rule renames the middle transoms");
    let ev = run(&object(recipe()), &input, &NoNesting).unwrap();
    assert_eq!(ev.items("frames").unwrap().len(), 17);
    assert_eq!(ev.items("panels").unwrap().len(), 6);
    // Panels are inset by half the member widths: the first cell starts at 0.05, 0.05.
    let first = &list(&g, "cells")[0];
    assert_eq!(
        get(first, "insets"),
        &Value::List(
            vec![0.05, 0.025, 0.025, 0.05]
                .into_iter()
                .map(Value::Number)
                .collect()
        )
    );
}

#[test]
fn overrides_reach_the_grid() {
    let mut input = Input::default();
    input.inputs.insert(
        "edits".into(),
        json!([{"remove": "h1.1"}, {"remove": "nope"}]),
    );
    let g = grid_value(&input);
    assert_eq!(list(&g, "cells").len(), 5);
    assert_eq!(list(&g, "unused_overrides").len(), 1);
}
