//! Plans and 3D views of the pinned spec's objects.

use std::path::PathBuf;

use opo_check::{DirResolver, load_type};
use opo_recipe::{Input, run};
use opo_render::{model_svg, plan_svg};

fn spec_dir() -> PathBuf {
    std::env::var_os("OPO_SPEC_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../opo-spec"))
}

fn balanced(svg: &str) -> bool {
    svg.starts_with("<?xml")
        && svg.trim_end().ends_with("</svg>")
        && svg.matches("<svg").count() == 1
}

#[test]
fn every_template_draws() {
    let spec = spec_dir();
    let resolver = DirResolver {
        prefix: "opo-format.org/".into(),
        root: spec.clone(),
    };
    for name in [
        "IfcBeam",
        "IfcBoiler",
        "IfcElementAssembly",
        "IfcMember",
        "IfcPipeFitting",
        "IfcSlab",
        "IfcStair",
        "IfcWall",
    ] {
        let def = load_type(&spec.join("library/templates").join(name)).unwrap();
        let ev = run(&def, &Input::default(), &resolver).unwrap();
        let plan = plan_svg(&def, &ev, "plan.medium", 50.0).unwrap();
        let model = model_svg(&def, &ev, "model.medium").unwrap();
        assert!(balanced(&plan) && balanced(&model), "{name}");
        assert!(
            plan.contains("<path") || plan.contains("<polyline"),
            "{name}: empty plan"
        );
        assert!(model.contains("<polygon"), "{name}: empty model");
        // Deterministic.
        assert_eq!(plan, plan_svg(&def, &ev, "plan.medium", 50.0).unwrap());
    }
}

#[test]
fn wall_plan_is_hatched_from_its_own_materials() {
    let spec = spec_dir();
    let def = load_type(&spec.join("library/templates/IfcWall")).unwrap();
    let mut input = Input::default();
    input
        .inputs
        .insert("path".into(), serde_json::json!([[0, 0], [1.2, 0]]));
    let ev = run(&def, &input, &opo_recipe::NoNesting).unwrap();
    let svg = plan_svg(&def, &ev, "plan.medium", 20.0).unwrap();
    assert!(
        svg.contains("<clipPath"),
        "line hatches are clipped to the layers"
    );
    assert!(
        svg.contains("stroke-width=\"0.007\""),
        "cut lines are 0.35 mm at 1:20"
    );
    assert!(plan_svg(&def, &ev, "plan.nonexistent", 50.0).is_err());
}

#[test]
fn door_plan_and_model() {
    let spec = spec_dir();
    let resolver = DirResolver {
        prefix: "opo-format.org/".into(),
        root: spec.clone(),
    };
    let def = load_type(&spec.join("examples/door/single-swing-door")).unwrap();
    let ev = run(&def, &Input::default(), &resolver).unwrap();
    let plan = plan_svg(&def, &ev, "plan.medium", 50.0).unwrap();
    assert!(plan.contains("<clipPath"), "timber hatch in the jambs");
    assert!(
        model_svg(&def, &ev, "model.high")
            .unwrap()
            .contains("<polygon")
    );
}
