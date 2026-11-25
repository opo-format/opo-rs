//! Validate OPO objects.
//!
//! [`check_dir`] checks an unpacked object folder (`object.ifcx` + `layers/*.ifcx`):
//!
//! 1. **Structure**: every `opo::` attribute is declared by the schema and reads into its type;
//!    the type definition passes [`TypeDefinition::validate`]; no dangling links; layers do not
//!    overlap (spec §5.2); rule-set nodes match the declared rule sets.
//! 2. **Names**: every formula parses and uses only names that exist where it is evaluated
//!    (parameters, slots, earlier derived values, context, earlier recipe steps, `for_each`
//!    variables, operations).
//! 3. **IFC**: property and quantity names are in the IFC 4.3 templates of the class (when
//!    [`IfcTemplates`] are given).
//! 4. **Evaluation**: the object is run at its defaults, for every predefined type, every
//!    option of every choice, every boolean flipped, and every variant in its layers. In each
//!    case the recipe runs, error checks pass (warnings too at the defaults), `model.medium` is
//!    not empty (an opening has a void), IFC properties and quantities evaluate, active ports
//!    are not null, materials exist, and the mesh volume is within 2 % of `GrossVolume`.
//! 5. **Baked geometry**: the baked meshes match the recipe solids at the defaults.

mod compare;

pub use compare::{CompareOptions, compare_dirs};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use opo_feel::{Value, free_names, parse};
use opo_ifcx::{Composition, Document, compose, layer_overlaps, load_object_dir};
use opo_model::{ATTRIBUTE_NAMES, Op, ParameterType, RuleSetNode, TypeDefinition};
use opo_recipe::{Evaluation, Input, Resolver, run};
use serde_json::{Map, Value as Json};

/// Names every formula may use: the context roots (spec §4.4, §4.7).
pub const CONTEXT_ROOTS: [&str; 5] = ["host", "level", "view", "rules", "flip"];

/// IFC 4.3 property and quantity set templates per class: class → set name → property names.
#[derive(Debug, Clone, Default)]
pub struct IfcTemplates {
    by_class: HashMap<String, HashMap<String, HashSet<String>>>,
}

impl IfcTemplates {
    /// Read the extracted template table (`{class: [{name, props: [{name}]}]}`, as
    /// the psets.json written by `opo ifc`).
    pub fn from_json(json: &Json) -> Self {
        let mut by_class = HashMap::new();
        for (class, sets) in json.as_object().into_iter().flatten() {
            let mut m = HashMap::new();
            for s in sets.as_array().into_iter().flatten() {
                let name = s["name"].as_str().unwrap_or_default().to_owned();
                let props = s["props"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p["name"].as_str().map(str::to_owned))
                    .collect();
                m.insert(name, props);
            }
            by_class.insert(class.clone(), m);
        }
        IfcTemplates { by_class }
    }

    /// Read the table from a file.
    pub fn read(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let json: Json =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Self::from_json(&json))
    }

    fn has(&self, class: &str, set: &str, prop: &str) -> bool {
        self.by_class
            .get(class)
            .and_then(|m| m.get(set))
            .is_some_and(|p| p.contains(prop))
    }
}

/// Resolves object ids that start with a prefix to folders under a root, e.g.
/// `opo-format.org/library/templates/IfcMember` → `<root>/library/templates/IfcMember`.
pub struct DirResolver {
    /// Id prefix, e.g. `opo-format.org/`.
    pub prefix: String,
    /// Folder the rest of the id is relative to.
    pub root: PathBuf,
}

impl Resolver for DirResolver {
    fn resolve(&self, id: &str) -> Option<TypeDefinition> {
        let rest = id.strip_prefix(&self.prefix)?;
        load_type(&self.root.join(rest)).ok()
    }
}

/// Read the type definition of an object folder (base document only).
pub fn load_type(dir: &Path) -> Result<TypeDefinition, String> {
    let obj = load_object_dir(dir).map_err(|e| e.to_string())?;
    let c = compose(std::slice::from_ref(&obj.base)).map_err(|e| e.to_string())?;
    let t = c
        .type_node()
        .ok_or("no type node (a node with opo::object)")?;
    TypeDefinition::from_attributes(&c.node(t).expect("type node exists").attributes)
        .map_err(|e| e.to_string())
}

/// What [`check_dir`] needs besides the folder.
pub struct Options<'a> {
    /// Finds nested objects.
    pub resolver: &'a dyn Resolver,
    /// IFC templates for the property-name check; `None` skips it.
    pub ifc: Option<&'a IfcTemplates>,
}

/// The result of checking one object.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// The object id (or the folder when it cannot be read).
    pub object: String,
    /// Every problem found; empty means the object is valid.
    pub errors: Vec<String>,
    /// Number of evaluation cases run.
    pub cases: usize,
    /// Number of cases whose mesh volume was compared with `GrossVolume`.
    pub volume_checks: usize,
    /// Values marked `to_verify` in `opo::provenance` (`file: node: path`); not errors, but
    /// they block a stable release (decision 0017).
    pub to_verify: Vec<String>,
}

impl Report {
    /// True if no problem was found.
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

/// A sample custom outline (an arched opening) for polygon slots that start empty.
fn sample_arch() -> Json {
    let mut p = vec![
        serde_json::json!([-0.5, 0.9]),
        serde_json::json!([0.5, 0.9]),
        serde_json::json!([0.5, 1.9]),
    ];
    for a in (15..180).step_by(15) {
        let t = (a as f64).to_radians();
        p.push(serde_json::json!([0.5 * t.cos(), 1.9 + 0.5 * t.sin()]));
    }
    p.push(serde_json::json!([-0.5, 1.9]));
    Json::Array(p)
}

/// Check an unpacked object folder (see the crate documentation).
pub fn check_dir(dir: &Path, opts: &Options) -> Report {
    let mut r = Report {
        object: dir.display().to_string(),
        ..Report::default()
    };
    let obj = match load_object_dir(dir) {
        Ok(o) => o,
        Err(e) => {
            r.errors.push(e.to_string());
            return r;
        }
    };
    let mut docs: Vec<(String, &Document)> = vec![("object.ifcx".into(), &obj.base)];
    docs.extend(obj.layers.iter().map(|(n, d)| (format!("layers/{n}"), d)));
    let all: Vec<Document> = docs.iter().map(|(_, d)| (*d).clone()).collect();
    let comp = match compose(&all) {
        Ok(c) => c,
        Err(e) => {
            r.errors.push(e.to_string());
            return r;
        }
    };
    let base = compose(std::slice::from_ref(&obj.base)).expect("base composes");
    let Some(type_path) = base.type_node().map(str::to_owned) else {
        r.errors
            .push("object.ifcx: no type node (a node with opo::object)".into());
        return r;
    };
    let def =
        match TypeDefinition::from_attributes(&base.node(&type_path).expect("exists").attributes) {
            Ok(d) => d,
            Err(e) => {
                r.errors.push(format!("object.ifcx: {e}"));
                return r;
            }
        };
    r.object = def.object.id.clone();

    structure(&docs, &comp, &obj.layers, &def, &mut r.errors);
    r.to_verify = to_verify(&docs);
    names(&def, &mut r.errors);
    if let Some(t) = opts.ifc {
        ifc_names(&def, t, &mut r.errors);
    }
    evaluation(&docs, &comp, &def, opts, &mut r);
    baked(&obj.base, &def, opts, &mut r.errors);
    r
}

// ------------------------------------------------------------------ 1. structure

fn structure(
    docs: &[(String, &Document)],
    comp: &Composition,
    layers: &[(String, Document)],
    def: &TypeDefinition,
    errs: &mut Vec<String>,
) {
    for (rel, d) in docs {
        for n in &d.data {
            for (k, v) in n.attributes.iter().flatten() {
                if !k.starts_with("opo::") {
                    continue;
                }
                if !ATTRIBUTE_NAMES.contains(&k.as_str()) {
                    errs.push(format!(
                        "{rel}: attribute {k} is not declared in the schema"
                    ));
                } else if let Err(e) = parse_attribute(k, v) {
                    errs.push(format!("{rel}: {k} on {}: {e}", n.path));
                }
                if k == "opo::provenance" {
                    if let Ok(p) = serde_json::from_value::<opo_model::Provenance>(v.clone()) {
                        for e in opo_model::check_provenance(&p) {
                            errs.push(format!("{rel}: {} on {}: {e}", k, n.path));
                        }
                        for path in p.keys() {
                            if provenance_target(comp, &n.path, path).is_none() {
                                errs.push(format!(
                                    "{rel}: provenance '{path}' on {} names no value",
                                    n.path
                                ));
                            }
                        }
                    }
                }
                if k == "opo::ruleset" {
                    if let Ok(node) = serde_json::from_value::<RuleSetNode>(v.clone()) {
                        errs.extend(
                            def.check_rule_set(&node)
                                .into_iter()
                                .map(|e| format!("{rel}: {e}")),
                        );
                    }
                }
            }
        }
    }
    for i in def.validate() {
        errs.push(format!("object.ifcx: {i}"));
    }
    for (from, to) in comp.dangling() {
        errs.push(format!("dangling reference {to} from {from}"));
    }
    for o in layer_overlaps(layers) {
        errs.push(format!(
            "layers {} and {} both set {} on {}",
            o.layers.0, o.layers.1, o.attribute, o.path
        ));
    }
}

/// The value a provenance path names on a node: `<attribute>.<key>…` where the first part is
/// the attribute without `opo::`, objects are entered by key and lists by the `name` or `id`
/// of an element.
fn provenance_target<'a>(comp: &'a Composition, node: &str, path: &str) -> Option<&'a Json> {
    let mut parts = path.split('.');
    let attrs = &comp.node(node)?.attributes;
    let mut v = attrs.get(&format!("opo::{}", parts.next()?))?;
    for p in parts {
        v = match v {
            Json::Object(m) => m.get(p)?,
            Json::Array(a) => a.iter().find(|e| {
                e.get("name").and_then(Json::as_str) == Some(p)
                    || e.get("id").and_then(Json::as_str) == Some(p)
            })?,
            _ => return None,
        };
    }
    Some(v)
}

/// Every `to_verify` entry of `opo::provenance` in the documents.
fn to_verify(docs: &[(String, &Document)]) -> Vec<String> {
    let mut out = Vec::new();
    for (rel, d) in docs {
        for n in &d.data {
            let Some(v) = n.attributes.as_ref().and_then(|a| a.get("opo::provenance")) else {
                continue;
            };
            if let Ok(p) = serde_json::from_value::<opo_model::Provenance>(v.clone()) {
                for (path, e) in p {
                    if e.status == opo_model::ProvenanceStatus::ToVerify {
                        out.push(format!("{rel}: {}: {path}", n.path));
                    }
                }
            }
        }
    }
    out
}

fn parse_attribute(name: &str, v: &Json) -> Result<(), String> {
    fn p<T: serde::de::DeserializeOwned>(v: &Json) -> Result<(), String> {
        serde_json::from_value::<T>(v.clone())
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    use opo_model as m;
    match name {
        "opo::object" => p::<m::Object>(v),
        "opo::parameters" => p::<Vec<m::Parameter>>(v),
        "opo::slots" => p::<Vec<m::Slot>>(v),
        "opo::rules" => p::<m::Rules>(v),
        "opo::recipe" => p::<Vec<m::RecipeStep>>(v),
        "opo::representations" => p::<Vec<m::Representation>>(v),
        "opo::host" => p::<m::Host>(v),
        "opo::handles" => p::<Vec<m::Handle>>(v),
        "opo::materials" => p::<HashMap<String, m::Material>>(v),
        "opo::hatches" => p::<HashMap<String, m::Hatch>>(v),
        "opo::ports" => p::<Vec<m::Port>>(v),
        "opo::ifc" => p::<m::Ifc>(v),
        "opo::values" | "opo::inputs" => p::<Map<String, Json>>(v),
        "opo::representation" | "opo::note" => p::<String>(v),
        "opo::line_class" => p::<m::LineClass>(v),
        "opo::ruleset" => p::<m::RuleSetNode>(v),
        "opo::provenance" => p::<m::Provenance>(v),
        _ => Ok(()),
    }
}

// ------------------------------------------------------------------ 2. names

struct Names<'a> {
    known: HashSet<String>,
    errs: &'a mut Vec<String>,
}

impl Names<'_> {
    fn need(&mut self, expr: Option<&Json>, at: &str, extra: &HashSet<String>) {
        let Some(Json::String(src)) = expr else {
            return;
        };
        match parse(src) {
            Err(e) => self.errs.push(format!("{at}: does not parse: {e}")),
            Ok(e) => {
                let mut miss: Vec<String> = free_names(&e)
                    .into_iter()
                    .filter(|n| !self.known.contains(n) && !extra.contains(n))
                    .collect();
                if !miss.is_empty() {
                    miss.sort();
                    self.errs
                        .push(format!("{at}: unknown names {miss:?} in {src:?}"));
                }
            }
        }
    }

    fn need_str(&mut self, expr: Option<&str>, at: &str, extra: &HashSet<String>) {
        self.need(expr.map(|s| Json::String(s.into())).as_ref(), at, extra);
    }
}

fn names(def: &TypeDefinition, errs: &mut Vec<String>) {
    let mut known: HashSet<String> = def.parameters.iter().map(|p| p.name.clone()).collect();
    known.extend(def.slots.iter().flatten().map(|s| s.name.clone()));
    known.extend(CONTEXT_ROOTS.iter().map(|s| s.to_string()));
    known.extend(Op::ALL.iter().map(|o| o.name().to_string()));
    let mut n = Names { known, errs };
    let none = HashSet::new();

    // Derived values see the ones before them.
    for d in def.derived() {
        n.need_str(Some(&d.expr), &format!("derived {}", d.name), &none);
        n.known.insert(d.name.clone());
    }
    for p in &def.parameters {
        n.need_str(
            p.visible.as_deref(),
            &format!("parameter {}.visible", p.name),
            &none,
        );
        for pr in p.properties.iter().flatten() {
            n.need_str(
                pr.value.as_deref(),
                &format!("{}.{}", p.name, pr.name),
                &none,
            );
        }
    }
    for c in def.checks() {
        n.need_str(Some(&c.expr), &format!("check {}", c.id), &none);
    }
    let mut done = HashSet::new();
    for s in &def.recipe {
        let mut avail = done.clone();
        if s.op == Op::ForEach {
            for (key, default) in [("as", "item"), ("index", "i")] {
                avail.insert(
                    s.args
                        .get(key)
                        .and_then(Json::as_str)
                        .unwrap_or(default)
                        .to_owned(),
                );
            }
        }
        n.need_str(s.when.as_deref(), &format!("step {}.when", s.id), &done);
        for (k, v) in &s.args {
            if s.op == Op::ForEach && (k == "as" || k == "index") {
                continue;
            }
            if s.op == Op::Instance && k == "representation" {
                continue;
            }
            // Grid class rules are FEEL over the member being classed.
            if s.op == Op::Grid && k == "classes" {
                if let Json::Object(rules) = v {
                    for (who, expr) in rules {
                        let mut local = avail.clone();
                        local.insert(who.clone());
                        n.need(Some(expr), &format!("step {}.classes.{who}", s.id), &local);
                    }
                }
                continue;
            }
            n.need(Some(v), &format!("step {}.{k}", s.id), &avail);
        }
        done.insert(s.id.clone());
    }
    for r in &def.representations {
        for it in r.items.iter().chain(r.overlay.iter()).flatten() {
            for (k, v) in [("material", &it.material), ("hatch", &it.hatch)] {
                if let Some(v) = v.as_deref().filter(|v| !v.starts_with("each.")) {
                    n.need_str(Some(v), &format!("representation {}.{k}", r.id), &none);
                }
            }
        }
    }
    for p in def.ports.iter().flatten() {
        for (k, v) in [
            ("nominal_size", Some(&p.nominal_size)),
            ("position", Some(&p.position)),
            ("direction", Some(&p.direction)),
            ("when", p.when.as_ref()),
        ] {
            n.need_str(
                v.map(String::as_str),
                &format!("port {}.{k}", p.name),
                &none,
            );
        }
    }
    for (k, v) in def.ifc.properties.iter().flatten() {
        n.need(Some(v), &format!("ifc property {k}"), &none);
    }
    for (q, qs) in def.ifc.quantity_sets().unwrap_or_default() {
        for (k, v) in &qs.values {
            n.need(Some(v), &format!("quantity {q}.{k}"), &none);
        }
    }
    if let Some(Json::Object(mls)) = def.ifc.extra.get("material_layer_set") {
        for (k, v) in mls.iter().filter(|(k, _)| *k != "note") {
            n.need(Some(v), &format!("material_layer_set.{k}"), &none);
        }
    }
    if let Some(Json::Object(pd)) = def.ifc.extra.get("profile_def") {
        for (kind, v) in pd {
            match v {
                Json::Object(fields) => {
                    for (k, x) in fields.iter().filter(|(k, _)| *k != "entity") {
                        n.need(Some(x), &format!("profile_def.{kind}.{k}"), &none);
                    }
                }
                other => n.need(Some(other), &format!("profile_def.{kind}"), &none),
            }
        }
    }
    for (k, v) in def.host.iter().flat_map(|h| h.provides.iter().flatten()) {
        n.need(Some(v), &format!("host.provides.{k}"), &none);
    }
}

// ------------------------------------------------------------------ 3. IFC names

fn ifc_names(def: &TypeDefinition, t: &IfcTemplates, errs: &mut Vec<String>) {
    let class = &def.ifc.class;
    // Keys `bsi::ifc::prop::<Attribute>` are IFC entity attributes (OverallWidth…), not
    // property-set entries; only `Pset_….<Property>` keys are looked up.
    for key in def
        .ifc
        .properties
        .iter()
        .flatten()
        .map(|(k, _)| k)
        .filter(|k| !k.starts_with("bsi::ifc::prop::"))
    {
        let (set, prop) = key.split_once('.').unwrap_or((key, ""));
        if !t.has(class, set, prop) {
            errs.push(format!(
                "ifc property {key} is not in the IFC 4.3 templates for {class}"
            ));
        }
    }
    for (q, qs) in def.ifc.quantity_sets().unwrap_or_default() {
        for k in qs.values.keys() {
            if !t.has(class, &q, k) {
                errs.push(format!("quantity {q}.{k} is not in the IFC 4.3 templates"));
            }
        }
    }
}

// ------------------------------------------------------------------ 4. evaluation

fn cases(
    docs: &[(String, &Document)],
    comp: &Composition,
    def: &TypeDefinition,
) -> Vec<(String, Map<String, Json>)> {
    let one = |k: &str, v: Json| {
        let mut m = Map::new();
        m.insert(k.into(), v);
        m
    };
    let mut out = vec![("default".to_owned(), Map::new())];
    for p in &def.parameters {
        let Some(opo_model::Options::List(opts)) = &p.options else {
            if p.kind == ParameterType::Boolean {
                let flipped = !p.default.as_bool().unwrap_or(false);
                out.push((
                    format!("{} = {flipped}", p.name),
                    one(&p.name, Json::Bool(flipped)),
                ));
            }
            continue;
        };
        if p.kind != ParameterType::Choice {
            continue;
        }
        for o in opts {
            // Every predefined type, including the default; other choices skip the default.
            if p.name == "predefined_type" || p.default.as_str() != Some(o) {
                let label = if p.name == "predefined_type" {
                    format!("type {o}")
                } else {
                    format!("{} = {o}", p.name)
                };
                out.push((label, one(&p.name, Json::String(o.clone()))));
            }
        }
    }
    for (rel, d) in docs.iter().skip(1) {
        for n in &d.data {
            if let Some(Json::String(type_name)) = n
                .attributes
                .as_ref()
                .and_then(|a| a.get("bsi::ifc::prop::TypeName"))
            {
                out.push((
                    format!("{rel}: {type_name}"),
                    comp.merged_attribute(&n.path, "opo::values"),
                ));
            }
        }
    }
    out
}

fn materials_in(def: &TypeDefinition, values: &Map<String, Json>) -> HashSet<String> {
    let mut out = HashSet::new();
    for p in &def.parameters {
        let v = values.get(&p.name).unwrap_or(&p.default);
        if let Json::Array(items) = v {
            out.extend(
                items
                    .iter()
                    .filter_map(|l| l.get("material").and_then(Json::as_str))
                    .map(str::to_owned),
            );
        }
        if p.kind == ParameterType::Material {
            if let Some(m) = v.as_str() {
                out.insert(m.to_owned());
            }
        }
    }
    out
}

fn solids_of(ev: &Evaluation, step: &str) -> Vec<opo_recipe::Solid> {
    ev.items(step)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|g| g.solid().cloned())
        .collect()
}

fn model_solids(def: &TypeDefinition, ev: &Evaluation) -> Vec<opo_recipe::Solid> {
    def.representation("model.medium")
        .into_iter()
        .flat_map(|r| r.items.iter().flatten())
        .flat_map(|it| solids_of(ev, &it.geometry))
        .collect()
}

fn evaluation(
    docs: &[(String, &Document)],
    comp: &Composition,
    def: &TypeDefinition,
    opts: &Options,
    r: &mut Report,
) {
    let opening = def.ifc.class == "IfcOpeningElement";
    let materials: HashSet<&str> = def
        .materials
        .iter()
        .flatten()
        .map(|(k, _)| k.as_str())
        .collect();
    let mut sample = Map::new();
    for s in def.slots.iter().flatten() {
        if s.default.is_none() && s.kind == opo_model::SlotType::Polygon {
            sample.insert(s.name.clone(), sample_arch());
        }
    }
    let qsets = def.ifc.quantity_sets().unwrap_or_default();
    let all = cases(docs, comp, def);
    r.cases = all.len();
    for (label, values) in all {
        let input = Input {
            values: values.clone(),
            inputs: sample.clone(),
            context: Map::new(),
        };
        let ev = match run(def, &input, opts.resolver) {
            Ok(ev) => ev,
            Err(e) => {
                r.errors.push(format!("{label}: recipe failed: {e}"));
                continue;
            }
        };
        if def.materials.is_some() {
            let mut merged: Map<String, Json> = def
                .parameters
                .iter()
                .map(|p| (p.name.clone(), p.default.clone()))
                .collect();
            merged.extend(values.clone());
            let mut missing: Vec<_> = materials_in(def, &merged)
                .into_iter()
                .filter(|m| !materials.contains(m.as_str()))
                .collect();
            missing.sort();
            for m in missing {
                r.errors
                    .push(format!("{label}: material {m} not in opo::materials"));
            }
        }
        let mut failed = Vec::new();
        for c in def.checks() {
            let ok = matches!(ev.eval(&c.expr), Ok(Value::Bool(true)));
            let warning = c.severity == opo_model::Severity::Warning;
            if !ok && !(label != "default" && warning) {
                failed.push(format!("{} ({:?})", c.id, c.severity).to_lowercase());
            }
        }
        if !failed.is_empty()
            && (label == "default" || failed.iter().any(|f| f.contains("(error)")))
        {
            r.errors
                .push(format!("{label}: checks not passing: {failed:?}"));
        }
        let solids = if opening {
            solids_of(&ev, "void")
        } else {
            model_solids(def, &ev)
        };
        if solids.is_empty() {
            r.errors.push(format!(
                "{label}: {}",
                if opening {
                    "no void"
                } else {
                    "model.medium is empty"
                }
            ));
        }
        for (k, v) in def.ifc.properties.iter().flatten() {
            if let Json::String(e) = v {
                if let Err(e) = ev.eval(e) {
                    r.errors.push(format!("{label}: ifc {k}: {e}"));
                }
            }
        }
        let mut gross = None;
        for (q, qs) in &qsets {
            for (k, v) in &qs.values {
                let Json::String(e) = v else { continue };
                match ev.eval(e) {
                    Ok(Value::Number(x)) if k == "GrossVolume" && gross.is_none() => {
                        gross = Some(x)
                    }
                    Ok(_) => {}
                    Err(e) => r.errors.push(format!("{label}: quantity {q}.{k}: {e}")),
                }
            }
        }
        for p in def.ports.iter().flatten() {
            if let Some(w) = &p.when {
                if !matches!(ev.eval(w), Ok(Value::Bool(true))) {
                    continue;
                }
            }
            for (k, e) in [
                ("nominal_size", &p.nominal_size),
                ("position", &p.position),
                ("direction", &p.direction),
            ] {
                if matches!(ev.eval(e), Ok(Value::Null) | Err(_)) {
                    r.errors
                        .push(format!("{label}: port {}.{k} is null", p.name));
                }
            }
        }
        if let Some(g) = gross.filter(|g| *g != 0.0) {
            r.volume_checks += 1;
            let vol: f64 = solids.iter().map(opo_recipe::Solid::volume).sum();
            let dev = (vol - g).abs() / g;
            if dev > 0.02 {
                r.errors.push(format!(
                    "{label}: mesh volume {vol:.4} m³ vs quantity GrossVolume {g:.4} m³ ({:.1} %)",
                    dev * 100.0
                ));
            }
        }
    }
}

// ------------------------------------------------------------------ 5. baked geometry

fn baked(base: &Document, def: &TypeDefinition, opts: &Options, errs: &mut Vec<String>) {
    let ev = match run(def, &Input::default(), opts.resolver) {
        Ok(ev) => ev,
        Err(e) => {
            errs.push(format!(
                "defaults without sample inputs: recipe failed: {e}"
            ));
            return;
        }
    };
    let mut expect: Vec<[f64; 3]> = model_solids(def, &ev)
        .into_iter()
        .flat_map(|s| s.points)
        .collect();
    let mut baked: Vec<[f64; 3]> = base
        .data
        .iter()
        .filter_map(|n| {
            n.attributes
                .as_ref()
                .and_then(|a| a.get("usd::usdgeom::mesh"))
        })
        .flat_map(|m| m["points"].as_array().cloned().unwrap_or_default())
        .map(|p| {
            let c: Vec<f64> = p
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Json::as_f64)
                .collect();
            [
                c.first().copied().unwrap_or(f64::NAN),
                c.get(1).copied().unwrap_or(f64::NAN),
                c.get(2).copied().unwrap_or(f64::NAN),
            ]
        })
        .collect();
    if !same_points(&mut baked, &mut expect) {
        errs.push(format!(
            "baked mesh differs from the recipe ({} baked points, {} recipe points)",
            baked.len(),
            expect.len()
        ));
    }
}

/// The same distinct points within 1e-6 m, in any order and grouping. Baked meshes may group
/// solids (one mesh per part) or repeat corners per face, and polygon libraries start rings
/// at different vertices, so the check compares sets of points rather than meshes.
fn same_points(a: &mut Vec<[f64; 3]>, b: &mut Vec<[f64; 3]>) -> bool {
    let key = |p: &[f64; 3]| p.map(|x| (x * 1e6).round() as i64);
    let close = |p: &[f64; 3], q: &[f64; 3]| p.iter().zip(q).all(|(x, y)| (x - y).abs() <= 1e-6);
    for v in [&mut *a, &mut *b] {
        v.sort_by_key(key);
        v.dedup_by(|p, q| close(p, q));
    }
    a.len() == b.len() && a.iter().zip(b.iter()).all(|(p, q)| close(p, q))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ifc_templates_lookup() {
        let t = IfcTemplates::from_json(
            &json!({"IfcWall": [{"name": "Pset_WallCommon", "props": [{"name": "IsExternal"}]}]}),
        );
        assert!(t.has("IfcWall", "Pset_WallCommon", "IsExternal"));
        assert!(!t.has("IfcWall", "Pset_WallCommon", "Colour"));
        assert!(!t.has("IfcSlab", "Pset_WallCommon", "IsExternal"));
    }

    #[test]
    fn sample_arch_is_closed_enough() {
        let a = sample_arch();
        assert_eq!(a.as_array().unwrap().len(), 15);
    }

    #[test]
    fn points_compare_in_any_order() {
        let mut a = vec![[0.0, 0.0, 0.0], [1.0, 2.0, 3.0]];
        let mut b = vec![[1.0, 2.0, 3.0 + 1e-9], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]];
        assert!(
            same_points(&mut a, &mut b),
            "order and repeated corners do not matter"
        );
        let mut c = vec![[1.0, 2.0, 3.1], [0.0, 0.0, 0.0]];
        assert!(!same_points(&mut a, &mut c));
        assert!(!same_points(&mut a, &mut vec![[0.0, 0.0, 0.0]]));
    }

    #[test]
    fn missing_folder_is_reported() {
        let r = check_dir(
            Path::new("/nonexistent/object"),
            &Options {
                resolver: &opo_recipe::NoNesting,
                ifc: None,
            },
        );
        assert!(!r.ok());
    }
}
