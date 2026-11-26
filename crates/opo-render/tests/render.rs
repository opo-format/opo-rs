//! Plans and 3D views of the pinned spec's objects.

use std::path::PathBuf;

use opo_check::{DirResolver, load_type};
use opo_recipe::{Input, run};
use opo_render::{elevation_svg, hosted_plan_svg, model_svg, plan_svg, solid_colour};

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
    assert!(
        drawn.contains("stroke-dasharray") && !bare.contains("stroke-dasharray"),
        "the swing indicator is dashed in elevation"
    );
    assert!(elevation_svg(&def, &ev, "model.medium", Some("elevation.none"), 50.0).is_err());
}

#[test]
fn derived_plan_is_cut_from_the_model() {
    let spec = spec_dir().join("conformance/fixtures");
    let resolver = DirResolver {
        prefix: "opo-format.org/".into(),
        root: spec.clone(),
    };
    let mut def = load_type(&spec.join("examples/door/single-swing-door")).unwrap();
    let ev = run(&def, &Input::default(), &resolver).unwrap();
    let rep = def
        .representations
        .iter_mut()
        .find(|r| r.id == "plan.medium")
        .unwrap();
    rep.items = None;
    rep.overlay = None;
    rep.source = opo_model::RepresentationSource::Derived;
    rep.derive = Some(opo_model::Derive {
        from: "model.medium".into(),
        method: opo_model::DeriveMethod::CutOrProject,
        exclude_parts: None,
        lod_exceptions: None,
    });
    let svg = plan_svg(&def, &ev, "plan.medium", 50.0).unwrap();
    if let Some(dir) = std::env::var_os("OPO_RENDER_OUT") {
        std::fs::write(PathBuf::from(dir).join("door-derived-plan.svg"), &svg).unwrap();
    }
    assert!(balanced(&svg));
    assert!(svg.contains("fill=\"#ffffff\""), "cut regions");
    assert!(
        svg.contains("stroke-width=\"0.0175\""),
        "cut lines are 0.35 mm at 1:50"
    );
    // At low detail the cut is filled solid (poché).
    let rep = def
        .representations
        .iter_mut()
        .find(|r| r.id == "plan.medium")
        .unwrap();
    rep.lod = opo_model::Lod::Low;
    let low = plan_svg(&def, &ev, "plan.medium", 200.0).unwrap();
    assert!(
        !low.contains("<clipPath") && !low.contains("fill=\"#3c3c3c\""),
        "a timber door frame is outlined, not filled, at low detail"
    );
}

#[test]
fn low_detail_plans_fill_masonry_and_concrete() {
    let spec = spec_dir().join("conformance/fixtures");
    let mut def = load_type(&spec.join("library/templates/IfcWall")).unwrap();
    let mut input = Input::default();
    input
        .inputs
        .insert("path".into(), serde_json::json!([[0, 0], [1.2, 0]]));
    let ev = run(&def, &input, &opo_recipe::NoNesting).unwrap();
    let rep = def
        .representations
        .iter_mut()
        .find(|r| r.id == "plan.medium")
        .unwrap();
    rep.items = None;
    rep.overlay = None;
    rep.lod = opo_model::Lod::Low;
    rep.source = opo_model::RepresentationSource::Derived;
    rep.derive = Some(opo_model::Derive {
        from: "model.medium".into(),
        method: opo_model::DeriveMethod::Cut,
        exclude_parts: None,
        lod_exceptions: None,
    });
    let svg = plan_svg(&def, &ev, "plan.medium", 100.0).unwrap();
    assert!(svg.contains("fill=\"#3c3c3c\""), "the wall is filled solid");
}

#[test]
fn part_categories_follow_the_lod_table() {
    // Proposal 0005: finish layers are hidden in a low plan and shown in a medium one; an
    // object exception can show them at low detail.
    let spec = spec_dir().join("conformance/fixtures");
    let mut def = load_type(&spec.join("library/templates/IfcWall")).unwrap();
    let mut input = Input::default();
    input
        .inputs
        .insert("path".into(), serde_json::json!([[0, 0], [1.2, 0]]));
    let ev = run(&def, &input, &opo_recipe::NoNesting).unwrap();
    for id in ["model.low", "model.medium"] {
        let m = def.representations.iter_mut().find(|r| r.id == id).unwrap();
        m.lod = if id == "model.low" {
            opo_model::Lod::Low
        } else {
            opo_model::Lod::Medium
        };
        m.items = def_items_with_finish(&m.items);
    }
    let plan = |def: &mut opo_model::TypeDefinition, from: &str, exc| {
        let rep = def
            .representations
            .iter_mut()
            .find(|r| r.id == "plan.medium")
            .unwrap();
        rep.items = None;
        rep.overlay = None;
        rep.source = opo_model::RepresentationSource::Derived;
        rep.derive = Some(opo_model::Derive {
            from: from.into(),
            method: opo_model::DeriveMethod::Cut,
            exclude_parts: None,
            lod_exceptions: exc,
        });
        plan_svg(def, &ev, "plan.medium", 50.0).unwrap()
    };
    let low = plan(&mut def, "model.low", None);
    let medium = plan(&mut def, "model.medium", None);
    assert!(
        medium.matches("<path").count() > low.matches("<path").count(),
        "the finish layers appear at medium detail"
    );
    let shown = plan(
        &mut def,
        "model.low",
        Some([(opo_model::PartCategory::Finish, opo_model::Visibility::Show)].into()),
    );
    assert_eq!(
        shown.matches("<path").count(),
        medium.matches("<path").count(),
        "an object exception shows them at low detail"
    );
}

/// Two items over the wall template's layer solids, one in the structure category and one in
/// the finish category, so the table has something to hide.
fn def_items_with_finish(
    _items: &Option<Vec<opo_model::RepresentationItem>>,
) -> Option<Vec<opo_model::RepresentationItem>> {
    let item = |cat| opo_model::RepresentationItem {
        geometry: "layer_solids".into(),
        material: Some("each.material".into()),
        hatch: None,
        class: None,
        role: None,
        part: Some("Layer".into()),
        category: Some(cat),
    };
    Some(vec![
        item(opo_model::PartCategory::Structure),
        item(opo_model::PartCategory::Finish),
    ])
}

#[test]
fn a_hosted_door_cuts_its_wall() {
    // Proposal 0004: the wall's plan loses the door's void; the door is drawn on its seat.
    let spec = spec_dir().join("conformance/fixtures");
    let resolver = DirResolver {
        prefix: "opo-format.org/".into(),
        root: spec.clone(),
    };
    let wall = load_type(&spec.join("library/templates/IfcWall")).unwrap();
    let mut win = Input::default();
    win.inputs
        .insert("path".into(), serde_json::json!([[-1.5, 0], [1.5, 0]]));
    let wev = run(&wall, &win, &opo_recipe::NoNesting).unwrap();
    let host = opo_recipe::host::provides(&wall, &wev).unwrap();
    let door = load_type(&spec.join("examples/door/single-swing-door")).unwrap();
    let din = Input {
        context: opo_recipe::host::hosted_context(&host),
        ..Input::default()
    };
    let dev = run(&door, &din, &resolver).unwrap();
    let regions =
        opo_recipe::host::cut_host_plan(&wall, &wev, "layer_solids", &door, &dev, 0.0, true, false)
            .unwrap();
    assert!(
        regions.iter().all(|r| r.shape.0.len() == 2),
        "each layer is split in two by the opening"
    );
    let merged =
        opo_recipe::host::cut_host_plan(&wall, &wev, "layer_solids", &door, &dev, 0.0, false, true)
            .unwrap();
    assert_eq!(merged.len(), 1, "at low detail the wall is one region");
    let svg = hosted_plan_svg(
        &wall,
        &wev,
        "layer_solids",
        &door,
        &dev,
        "plan.medium",
        0.0,
        50.0,
    )
    .unwrap();
    if let Some(dir) = std::env::var_os("OPO_RENDER_OUT") {
        std::fs::write(PathBuf::from(dir).join("wall-with-door.svg"), &svg).unwrap();
    }
    assert!(balanced(&svg) && svg.contains("<g transform=\"translate(0 "));
}
