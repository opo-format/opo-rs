//! Plans and 3D views of the pinned spec's objects.

use std::path::PathBuf;

use opo_check::{DirResolver, load_type};
use opo_recipe::{Input, run};
use opo_render::{elevation_svg, model_svg, plan_svg, solid_colour};

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
    let spec = spec_dir().join("conformance/fixtures");
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
        assert!(model.contains("<image"), "{name}: empty model");
        // Deterministic.
        assert_eq!(plan, plan_svg(&def, &ev, "plan.medium", 50.0).unwrap());
    }
}

#[test]
fn wall_plan_is_hatched_from_its_own_materials() {
    let spec = spec_dir().join("conformance/fixtures");
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
    let spec = spec_dir().join("conformance/fixtures");
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
            .contains("<image")
    );
}

#[test]
fn voids_are_not_drawn_in_the_model_view() {
    let spec = spec_dir().join("conformance/fixtures");
    let resolver = DirResolver {
        prefix: "opo-format.org/".into(),
        root: spec.clone(),
    };
    let mut def = load_type(&spec.join("library/templates/IfcOpeningElement")).unwrap();
    let ev = run(&def, &Input::default(), &resolver).unwrap();
    let with_void = model_svg(&def, &ev, "model.medium").unwrap();
    let mut voids = 0;
    for rep in &mut def.representations {
        for item in rep.items.iter_mut().flatten() {
            if item.role.as_deref() == Some("void") {
                item.role = None;
                voids += 1;
            }
        }
    }
    assert!(voids > 0, "the opening has a void");
    let drawn = model_svg(&def, &ev, "model.medium").unwrap();
    assert!(!with_void.contains("<image"), "the void is not drawn");
    assert!(
        drawn.contains("<image"),
        "the same solid draws without the role"
    );
}

#[test]
fn nested_objects_keep_their_material_colours() {
    let spec = spec_dir().join("conformance/fixtures");
    let resolver = DirResolver {
        prefix: "opo-format.org/".into(),
        root: spec.clone(),
    };
    let def = load_type(&spec.join("examples/grid/stick-wall")).unwrap();
    let ev = run(&def, &Input::default(), &resolver).unwrap();
    let rep = def.representation("model.medium").unwrap();
    let item = rep
        .items
        .iter()
        .flatten()
        .find(|i| i.geometry == "panels")
        .unwrap();
    let g = &ev.items("panels").unwrap()[0];
    assert_eq!(
        solid_colour(&def, item, &ev, g),
        "#bcd4dc",
        "glass, from the panel object"
    );
}

#[test]
fn door_elevation_has_the_model_and_its_lines() {
    let spec = spec_dir().join("conformance/fixtures");
    let resolver = DirResolver {
        prefix: "opo-format.org/".into(),
        root: spec.clone(),
    };
    let def = load_type(&spec.join("examples/door/single-swing-door")).unwrap();
    let ev = run(&def, &Input::default(), &resolver).unwrap();
    let bare = elevation_svg(&def, &ev, "model.medium", None, 50.0).unwrap();
    assert!(balanced(&bare) && bare.contains("<image"), "front view");
    let drawn = elevation_svg(&def, &ev, "model.medium", Some("elevation.medium"), 50.0).unwrap();
    assert!(balanced(&drawn) && drawn.contains("<image"));
    assert!(
        drawn.len() > bare.len(),
        "the elevation lines are drawn over it"
    );
    assert!(elevation_svg(&def, &ev, "model.medium", Some("elevation.none"), 50.0).is_err());
}
