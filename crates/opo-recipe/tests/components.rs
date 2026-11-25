//! Component parameters, `component_for`, `reported` and interface inputs (proposal 0001 §3).

use std::path::PathBuf;

use opo_feel::Value;
use opo_ifcx::{compose, load_object_dir};
use opo_model::TypeDefinition;
use opo_recipe::{Input, Resolver, run};
use serde_json::{Value as Json, json};

fn spec_dir() -> PathBuf {
    std::env::var_os("OPO_SPEC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../opo-spec"))
}

/// The IfcOpeningElement fixture with some attributes replaced.
fn object(replace: Json) -> TypeDefinition {
    let dir = spec_dir().join("conformance/fixtures/library/templates/IfcOpeningElement");
    let obj = load_object_dir(&dir).unwrap();
    let c = compose(std::slice::from_ref(&obj.base)).unwrap();
    let mut attrs = c.node(c.type_node().unwrap()).unwrap().attributes.clone();
    for (k, v) in replace.as_object().unwrap() {
        attrs.insert(k.clone(), v.clone());
    }
    attrs.remove("opo::slots");
    attrs.remove("opo::host");
    TypeDefinition::from_attributes(&attrs).unwrap()
}

fn model(items: &str) -> Json {
    json!([{"id": "model.medium", "context": "model", "lod": "medium", "source": "authored",
            "ifc": {"context": "Body", "targetView": "MODEL_VIEW"}, "items": [{"geometry": items}]}])
}

fn mullion() -> TypeDefinition {
    object(json!({
        "opo::parameters": [{"name": "face_w", "type": "length", "default": 0.05, "label": "Face", "group": "G"}],
        "opo::rules": {"derived": [
            {"name": "face_width", "expr": "face_w"}, {"name": "depth", "expr": "0.1"}, {"name": "offset", "expr": "0"}
        ]},
        "opo::interface": {"name": "frame",
            "inputs": ["path", "normal", "trim_start", "trim_end", "left_cell", "right_cell"],
            "reports": ["face_width", "depth", "offset"]},
        "opo::recipe": [{"id": "body", "op": "box", "args": {
            "min": "[path[1][1] - face_width / 2, -depth / 2, path[1][3] + trim_start]",
            "max": "[path[1][1] + face_width / 2, depth / 2, path[2][3] - trim_end]"}}],
        "opo::representations": model("body")
    }))
}

struct Lib;

impl Resolver for Lib {
    fn resolve(&self, id: &str) -> Option<TypeDefinition> {
        (id == "test/mullion").then(mullion)
    }
}

fn wall(do_expr: &str) -> TypeDefinition {
    object(json!({
        "opo::parameters": [
            {"name": "frame_classes", "type": "component", "interface": "frame", "default": null,
             "label": "Frames", "group": "G", "classes": {"mullion": "test/mullion", "default": "test/mullion"}}
        ],
        "opo::rules": {"derived": []},
        "opo::recipe": [
            {"id": "g", "op": "grid", "args": {
                "surface": "{baseline: [[0, 0], [2, 0]], base: 0, top: 2}",
                "u": "{pattern: [{size: 2}], distribution: \"count\"}",
                "v": "{pattern: [{size: 1}], distribution: \"count\"}",
                "widths": "{mullion: reported(component_for(frame_classes, \"mullion\"), \"face_width\", {face_w: 0.08})}"
            }},
            {"id": "frames", "op": "for_each", "args": {"items": "g.segments", "as": "s", "do": do_expr}}
        ],
        "opo::representations": model("frames")
    }))
}

const PLACE: &str = "if s.cls = \"mullion\" then instance(component_for(frame_classes, s.cls), {}, {path: s.path, normal: s.normal, trim_start: s.trim_start, trim_end: s.trim_end, left_cell: null, right_cell: null}) else null";

#[test]
fn components_fill_the_grid_and_report_widths() {
    let ev = run(&wall(PLACE), &Input::default(), &Lib).unwrap();
    assert_eq!(
        ev.items("frames").unwrap().len(),
        1,
        "one mullion between two columns"
    );
    let Value::Context(g) = &ev.geometry["g"] else {
        panic!()
    };
    let Value::List(cells) = &g["cells"] else {
        panic!()
    };
    let Value::Context(c) = &cells[0] else {
        panic!()
    };
    assert_eq!(
        c["insets"],
        Value::List(
            vec![0.0, 0.04, 0.0, 0.0]
                .into_iter()
                .map(Value::Number)
                .collect()
        ),
        "the right side is inset by half the reported 0.08 face width"
    );
}

#[test]
fn missing_interface_inputs_and_unreported_values_fail() {
    let short = PLACE.replace(", left_cell: null, right_cell: null", "");
    let err = run(&wall(&short), &Input::default(), &Lib)
        .unwrap_err()
        .to_string();
    assert!(err.contains("input 'left_cell' is not passed"), "{err}");

    let mut def = wall(PLACE);
    def.recipe[0].args.insert(
        "widths".into(),
        json!("{mullion: reported(\"test/mullion\", \"face_w\")}"),
    );
    let err = run(&def, &Input::default(), &Lib).unwrap_err().to_string();
    assert!(err.contains("does not report 'face_w'"), "{err}");
}
