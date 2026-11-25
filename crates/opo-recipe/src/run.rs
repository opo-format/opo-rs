//! Running an object: environment, derived values, recipe steps, nested objects.

use std::collections::BTreeMap;

use opo_feel::{Env, Scope, Value, eval_str};
use opo_model::{Op, RepresentationItem, TypeDefinition};
use serde_json::{Map, Value as Json};

use crate::geometry::{Geometry, Plane, flatten, list_value};
use crate::{Error, ops};

/// Finds nested objects by id (spec §4.9).
pub trait Resolver {
    /// The type definition of the object with this id, if known.
    fn resolve(&self, object_id: &str) -> Option<TypeDefinition>;
}

/// A resolver that knows no objects (for objects without nesting).
pub struct NoNesting;

impl Resolver for NoNesting {
    fn resolve(&self, _: &str) -> Option<TypeDefinition> {
        None
    }
}

/// What to run an object with.
#[derive(Debug, Clone, Default)]
pub struct Input {
    /// Parameter values on top of the defaults.
    pub values: Map<String, Json>,
    /// Drawn inputs (slots) on top of the defaults.
    pub inputs: Map<String, Json>,
    /// Host, level, view, rule-set and flip context on top of the defaults. A context value
    /// that is an object is merged into the default of the same name; others replace it.
    pub context: Map<String, Json>,
}

/// The result of running an object.
#[derive(Debug, Clone)]
pub struct Evaluation {
    /// Parameters, slots, context and derived values.
    pub env: Scope,
    /// Geometry of each recipe step by id (`null` for a step whose `when` was false).
    pub geometry: BTreeMap<String, Value>,
}

impl Evaluation {
    /// Evaluate a FEEL expression against the parameters, context and derived values.
    pub fn eval(&self, expr: &str) -> Result<Value, Error> {
        eval_str(expr, &self.env).map_err(|e| Error::Feel {
            at: expr.into(),
            source: e,
        })
    }

    /// Every geometry of a step, flattened.
    pub fn items(&self, step: &str) -> Result<Vec<Geometry>, Error> {
        self.geometry
            .get(step)
            .map(flatten)
            .unwrap_or(Ok(Vec::new()))
    }
}

/// Maximum depth of nested objects.
pub const MAX_DEPTH: usize = 8;

/// Run an object's recipe (spec §4.5).
pub fn run(
    def: &TypeDefinition,
    input: &Input,
    resolver: &dyn Resolver,
) -> Result<Evaluation, Error> {
    Runner {
        resolver,
        context: &input.context,
        depth: 0,
    }
    .run(def, &input.values, &input.inputs)
}

/// The default context: a 0.2 m host from −0.1 to 0.1 m and 3 m high, a 3 m storey, a 1.2 m
/// plan cut, no rule sets, each flip axis at its first declared value (conformance/README.md).
fn environment(
    def: &TypeDefinition,
    values: &Map<String, Json>,
    inputs: &Map<String, Json>,
    context: &Map<String, Json>,
) -> Result<Scope, Error> {
    let mut env: Map<String, Json> = Map::new();
    for p in &def.parameters {
        // A class map's value is its classes (`component_for` reads it).
        let v = match (&p.classes, &p.default) {
            (Some(c), Json::Null) => Json::Object(c.clone()),
            _ => p.default.clone(),
        };
        env.insert(p.name.clone(), v);
    }
    for s in def.slots.iter().flatten() {
        env.insert(s.name.clone(), s.default.clone().unwrap_or(Json::Null));
    }
    // Interface inputs that are not slots are null until a parent passes them.
    for i in def.interface.iter().flat_map(|i| i.inputs.iter()) {
        env.entry(i.clone()).or_insert(Json::Null);
    }
    env.extend(values.iter().map(|(k, v)| (k.clone(), v.clone())));
    env.extend(inputs.iter().map(|(k, v)| (k.clone(), v.clone())));
    for s in def.slots.iter().flatten() {
        if let (Some(n), Some(Json::Array(p))) = (s.count, env.get(&s.name)) {
            if p.len() != n as usize {
                return Err(Error::Op(format!(
                    "slot {} needs {n} points, got {}",
                    s.name,
                    p.len()
                )));
            }
        }
    }

    let mut host = serde_json::json!({"thickness": 0.2, "y_min": -0.1, "y_max": 0.1, "bottom": 0.0, "top": 3.0});
    for (k, decl) in def
        .host
        .as_ref()
        .and_then(|h| h.inputs.as_ref())
        .into_iter()
        .flatten()
    {
        host[k] = decl.get("default").cloned().unwrap_or(Json::Null);
    }
    let mut flip = Map::new();
    for (axis, decl) in def
        .host
        .as_ref()
        .and_then(|h| h.flip.as_ref())
        .into_iter()
        .flatten()
    {
        if let Some(first) = decl
            .get("values")
            .and_then(Json::as_array)
            .and_then(|v| v.first())
        {
            flip.insert(axis.clone(), first.clone());
        }
    }
    if let Some(Json::Object(f)) = values.get("flip") {
        flip.extend(f.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    let mut ctx = serde_json::json!({
        "host": host, "level": {"elevation": 0.0, "height": 3.0}, "view": {"cut_height": 1.2}, "rules": {}, "flip": flip
    });
    for (k, v) in context {
        match (ctx.get_mut(k), v) {
            (Some(Json::Object(base)), Json::Object(add)) => {
                base.extend(add.iter().map(|(a, b)| (a.clone(), b.clone())))
            }
            _ => {
                ctx[k] = v.clone();
            }
        }
    }
    env.extend(ctx.as_object().expect("context is an object").clone());

    let mut scope = Scope::from_context(&Value::from_json(&Json::Object(env)));
    for d in def.derived() {
        let v = eval_str(&d.expr, &scope).map_err(|e| Error::Feel {
            at: format!("derived {}", d.name),
            source: e,
        })?;
        scope.set(d.name.clone(), v);
    }
    Ok(scope)
}

struct Runner<'a> {
    resolver: &'a dyn Resolver,
    context: &'a Map<String, Json>,
    depth: usize,
}

/// Names visible to a recipe step: `for_each` variables, then step results, then the
/// environment. Calls go to the recipe operations.
struct StepEnv<'a> {
    env: &'a Scope,
    geometry: &'a BTreeMap<String, Value>,
    locals: Vec<(String, Value)>,
    plane: Plane,
    runner: &'a Runner<'a>,
}

impl Env for StepEnv<'_> {
    fn get(&self, name: &str) -> Option<&Value> {
        self.locals
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
            .or_else(|| self.geometry.get(name))
            .or_else(|| self.env.get(name))
    }

    fn call(&self, name: &str, args: Vec<Value>) -> Option<Result<Value, opo_feel::Error>> {
        let op = Op::from_name(name)?;
        let result = match op {
            Op::Instance => {
                let rep = match args.get(3) {
                    Some(Value::String(s)) => s.clone(),
                    _ => "model.medium".into(),
                };
                self.runner.instance(
                    args.first().unwrap_or(&Value::Null),
                    args.get(1).unwrap_or(&Value::Null),
                    args.get(2).unwrap_or(&Value::Null),
                    &rep,
                )
            }
            Op::ForEach | Op::Grid => Err(Error::Op(format!("{name} cannot be called inline"))),
            Op::Reported => self.runner.reported(
                args.first().unwrap_or(&Value::Null),
                args.get(1).unwrap_or(&Value::Null),
                args.get(2).unwrap_or(&Value::Null),
                args.get(3).unwrap_or(&Value::Null),
            ),
            _ => ops::apply(op, self.plane, &args),
        };
        Some(result.map_err(|e| opo_feel::Error::Eval(e.to_string())))
    }
}

impl StepEnv<'_> {
    fn eval(&self, arg: &Json, at: &str) -> Result<Value, Error> {
        match arg {
            Json::String(src) => eval_str(src, self).map_err(|e| Error::Feel {
                at: at.into(),
                source: e,
            }),
            literal => Ok(Value::from_json(literal)),
        }
    }
}

fn to_json_map(v: &Value) -> Map<String, Json> {
    match v.to_json() {
        Json::Object(m) => m,
        _ => Map::new(),
    }
}

impl Runner<'_> {
    fn run(
        &self,
        def: &TypeDefinition,
        values: &Map<String, Json>,
        inputs: &Map<String, Json>,
    ) -> Result<Evaluation, Error> {
        if self.depth > MAX_DEPTH {
            return Err(Error::Op(format!("objects nested deeper than {MAX_DEPTH}")));
        }
        let env = environment(def, values, inputs, self.context)?;
        let mut geometry = BTreeMap::new();
        for step in &def.recipe {
            let plane = step.plane.map(plane_of).unwrap_or(Plane::Xy);
            let at = |what: &str| format!("recipe step {} {what}", step.id);
            let senv = StepEnv {
                env: &env,
                geometry: &geometry,
                locals: Vec::new(),
                plane,
                runner: self,
            };
            if let Some(cond) = &step.when {
                if !matches!(
                    senv.eval(&Json::String(cond.clone()), &at("when"))?,
                    Value::Bool(true)
                ) {
                    geometry.insert(step.id.clone(), Value::Null);
                    continue;
                }
            }
            let arg = |name: &str| step.args.get(name);
            let value = match step.op {
                Op::ForEach => {
                    let items = senv.eval(arg("items").unwrap_or(&Json::Null), &at("items"))?;
                    let items = match items {
                        Value::Null => Vec::new(),
                        Value::List(l) => l,
                        other => {
                            return Err(Error::Op(format!(
                                "{}: items is not a list ({other:?})",
                                at("")
                            )));
                        }
                    };
                    let name_of = |key: &str, default: &str| match arg(key) {
                        Some(Json::String(s)) => s.clone(),
                        _ => default.to_owned(),
                    };
                    let (as_name, index_name) = (name_of("as", "item"), name_of("index", "i"));
                    let body = arg("do").cloned().unwrap_or(Json::Null);
                    let mut out = Vec::new();
                    for (i, item) in items.into_iter().enumerate() {
                        let each = StepEnv {
                            env: &env,
                            geometry: &geometry,
                            locals: vec![
                                (as_name.clone(), item.clone()),
                                (index_name.clone(), Value::Number((i + 1) as f64)),
                            ],
                            plane,
                            runner: self,
                        };
                        for mut g in flatten(&each.eval(&body, &at("do"))?)? {
                            if g.each.is_none() && matches!(item, Value::Context(_)) {
                                g.each = Some(item.clone());
                            }
                            g.index = Some(i + 1);
                            out.push(g);
                        }
                    }
                    list_value(out)
                }
                Op::Grid => self.grid(step, &senv, &at)?,
                Op::Reported => {
                    let get = |k: &str| match arg(k) {
                        Some(a) => senv.eval(a, &at(k)),
                        None => Ok(Value::Null),
                    };
                    self.reported(
                        &get("object")?,
                        &get("name")?,
                        &get("values")?,
                        &get("inputs")?,
                    )?
                }
                Op::Instance => {
                    let object = senv.eval(arg("object").unwrap_or(&Json::Null), &at("object"))?;
                    let vals = senv.eval(
                        arg("values").unwrap_or(&Json::String("{}".into())),
                        &at("values"),
                    )?;
                    let ins = senv.eval(
                        arg("inputs").unwrap_or(&Json::String("{}".into())),
                        &at("inputs"),
                    )?;
                    let rep = arg("representation")
                        .and_then(Json::as_str)
                        .unwrap_or("model.medium");
                    self.instance(&object, &vals, &ins, rep)?
                }
                op => {
                    let (required, optional) = op.arguments();
                    let mut values = Vec::new();
                    for name in required {
                        let a = arg(name).ok_or_else(|| {
                            Error::Op(format!("{}: missing argument '{name}'", at("")))
                        })?;
                        values.push(senv.eval(a, &at(name))?);
                    }
                    for name in optional {
                        values.push(match arg(name) {
                            Some(a) => senv.eval(a, &at(name))?,
                            None => Value::Null,
                        });
                    }
                    ops::apply(op, plane, &values)
                        .map_err(|e| Error::Op(format!("{}: {e}", at(""))))?
                }
            };
            geometry.insert(step.id.clone(), value);
        }
        Ok(Evaluation { env, geometry })
    }

    /// Run a `grid` step (proposal 0001 §2): lay out, apply the class rules and the instance
    /// overrides, then trim members and inset panels by the class widths.
    fn grid(
        &self,
        step: &opo_model::RecipeStep,
        senv: &StepEnv<'_>,
        at: &dyn Fn(&str) -> String,
    ) -> Result<Value, Error> {
        let arg = |name: &str| step.args.get(name);
        let need = |name: &str| {
            arg(name).ok_or_else(|| Error::Op(format!("{}: missing argument '{name}'", at(""))))
        };
        let surface = senv.eval(need("surface")?, &at("surface"))?;
        let u = senv.eval(need("u")?, &at("u"))?;
        let v = senv.eval(need("v")?, &at("v"))?;
        let mut g = crate::grid::Grid::build(&surface, &u, &v)
            .map_err(|e| Error::Op(format!("{}: {e}", at(""))))?;
        // Class rules are FEEL expressions over `segment` and `cell`, evaluated per member.
        if let Some(Json::Object(rules)) = arg("classes") {
            let class = |expr: &Json, name: &str, item: Value| -> Result<Option<String>, Error> {
                let local = StepEnv {
                    env: senv.env,
                    geometry: senv.geometry,
                    locals: vec![(name.to_string(), item)],
                    plane: senv.plane,
                    runner: senv.runner,
                };
                Ok(match local.eval(expr, &at(&format!("classes.{name}")))? {
                    Value::String(s) => Some(s),
                    _ => None,
                })
            };
            if let Some(expr) = rules.get("segment") {
                for k in 0..g.segments.len() {
                    if let Some(c) = class(expr, "segment", g.segment_value(&g.segments[k]))? {
                        g.segments[k].cls = c;
                    }
                }
            }
            if let Some(expr) = rules.get("cell") {
                for k in 0..g.cells.len() {
                    if let Some(c) = class(expr, "cell", g.cell_value(&g.cells[k]))? {
                        g.cells[k].cls = c;
                    }
                }
            }
        }
        if let Some(o) = arg("overrides") {
            g.apply_overrides(&senv.eval(o, &at("overrides"))?);
        }
        let join = match arg("join") {
            Some(j) => match senv.eval(j, &at("join"))? {
                Value::String(s) => s,
                Value::Null => "u_continuous".into(),
                other => {
                    return Err(Error::Op(format!(
                        "{}: join is not text ({other:?})",
                        at("")
                    )));
                }
            },
            None => "u_continuous".into(),
        };
        let mut widths = BTreeMap::new();
        if let Some(w) = arg("widths") {
            if let Value::Context(m) = senv.eval(w, &at("widths"))? {
                for (k, x) in m {
                    if let Some(n) = x.as_number() {
                        widths.insert(k, n);
                    }
                }
            }
        }
        g.finish(&join, &widths)
            .map_err(|e| Error::Op(format!("{}: {e}", at(""))))?;
        Ok(g.value())
    }

    /// A derived value a nested object reports through its interface (proposal 0001 §3).
    fn reported(
        &self,
        object: &Value,
        name: &Value,
        values: &Value,
        inputs: &Value,
    ) -> Result<Value, Error> {
        let (Value::String(id), Value::String(name)) = (object, name) else {
            return Err(Error::Op(
                "reported(object, name, …): object and name are text".into(),
            ));
        };
        let def = self
            .resolver
            .resolve(id)
            .ok_or_else(|| Error::Resolve(format!("cannot resolve nested object {id}")))?;
        let declared = def
            .interface
            .as_ref()
            .is_some_and(|i| i.reports.iter().any(|r| r == name));
        if !declared {
            return Err(Error::Resolve(format!("{id} does not report '{name}'")));
        }
        if self.depth >= MAX_DEPTH {
            return Err(Error::Op(format!("objects nested deeper than {MAX_DEPTH}")));
        }
        // Reported values are derived values: the nested recipe does not run.
        let env = environment(
            &def,
            &to_json_map(values),
            &to_json_map(inputs),
            self.context,
        )?;
        Ok(env.get(name).cloned().unwrap_or(Value::Null))
    }

    /// Place a nested object and return the items of one of its representations.
    fn instance(
        &self,
        object: &Value,
        values: &Value,
        inputs: &Value,
        representation: &str,
    ) -> Result<Value, Error> {
        let id = match object {
            Value::String(s) => s.clone(),
            other => {
                return Err(Error::Resolve(format!(
                    "instance: object must be an id, got {other:?}"
                )));
            }
        };
        let def = self
            .resolver
            .resolve(&id)
            .ok_or_else(|| Error::Resolve(format!("cannot resolve nested object {id}")))?;
        // An object that fills a component slot needs every input of its interface.
        if let Some(i) = &def.interface {
            let given = to_json_map(inputs);
            if let Some(missing) = i.inputs.iter().find(|x| !given.contains_key(*x)) {
                return Err(Error::Resolve(format!(
                    "{id}: interface {} input '{missing}' is not passed",
                    i.name
                )));
            }
        }
        let sub = Runner {
            resolver: self.resolver,
            context: self.context,
            depth: self.depth + 1,
        }
        .run(&def, &to_json_map(values), &to_json_map(inputs))?;
        let rep = def.representation(representation).ok_or_else(|| {
            Error::Resolve(format!("{id} has no representation {representation}"))
        })?;
        let mut parts = Vec::new();
        for item in rep.items.iter().flatten() {
            for mut g in sub.items(&item.geometry)? {
                g.material = resolve_material(item, &sub.env, &g)?;
                g.nested = Some(id.clone());
                parts.push(g);
            }
        }
        Ok(list_value(parts))
    }
}

fn plane_of(p: opo_model::Plane) -> Plane {
    match p {
        opo_model::Plane::Xy => Plane::Xy,
        opo_model::Plane::Xz => Plane::Xz,
        opo_model::Plane::Yz => Plane::Yz,
        opo_model::Plane::Profile => Plane::Profile,
    }
}

/// The material of a representation item for one geometry: a material resolved inside a
/// nested object wins; `each.<field>` reads the `for_each` item; otherwise the item's
/// `material` is a FEEL expression.
pub fn resolve_material(
    item: &RepresentationItem,
    env: &Scope,
    g: &Geometry,
) -> Result<Option<Value>, Error> {
    let Some(m) = &item.material else {
        return Ok(None);
    };
    if g.material.is_some() {
        return Ok(g.material.clone());
    }
    if let Some(field) = m.strip_prefix("each.") {
        return Ok(match &g.each {
            Some(Value::Context(c)) => c.get(field).cloned(),
            _ => None,
        });
    }
    eval_str(m, env).map(Some).map_err(|e| Error::Feel {
        at: format!("material of {}", item.geometry),
        source: e,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn def(recipe: Json) -> TypeDefinition {
        let a = json!({
            "opo::object": {"id": "opo-format.org/test/box", "name": "Box", "version": "0.1.0", "opo": "0.1",
                            "license": "CC0-1.0", "author": "a", "description": "d", "references": []},
            "opo::parameters": [
                {"name": "size", "type": "length", "default": 1.0, "label": "Size", "group": "G"},
                {"name": "lid", "type": "boolean", "default": false, "label": "Lid", "group": "G"}
            ],
            "opo::rules": {"derived": [{"name": "half", "expr": "size / 2"}]},
            "opo::recipe": recipe,
            "opo::representations": [{"id": "model.medium", "context": "model", "lod": "medium", "source": "authored",
                "ifc": {"context": "Body", "targetView": "MODEL_VIEW"}, "items": [{"geometry": "body", "material": "\"oak\""}]}],
            "opo::ifc": {"class": "IfcFurniture"}
        });
        TypeDefinition::from_attributes(a.as_object().unwrap()).unwrap()
    }

    fn volume(ev: &Evaluation, step: &str) -> f64 {
        ev.items(step)
            .unwrap()
            .iter()
            .filter_map(|g| g.solid().map(crate::Solid::volume))
            .sum()
    }

    fn body() -> Json {
        json!({"id": "body", "op": "box", "args": {"min": "[0, 0, 0]", "max": "[size, size, size]"}})
    }

    #[test]
    fn derived_values_and_values() {
        let d = def(json!([body()]));
        let mut input = Input::default();
        input.values.insert("size".into(), json!(2.0));
        let ev = run(&d, &input, &NoNesting).unwrap();
        assert_eq!(ev.eval("half").unwrap(), Value::Number(1.0));
        assert!((volume(&ev, "body") - 8.0).abs() < 1e-12);
    }

    #[test]
    fn false_condition_produces_nothing() {
        let d = def(
            json!([body(), {"id": "top", "op": "box", "when": "lid", "args": {"min": "[0, 0, size]", "max": "[size, size, size + 0.1]"}}]),
        );
        let ev = run(&d, &Input::default(), &NoNesting).unwrap();
        assert_eq!(ev.geometry["top"], Value::Null);
        assert!(ev.items("top").unwrap().is_empty());
    }

    #[test]
    fn for_each_records_items_and_indices() {
        let d = def(json!([{"id": "posts", "op": "for_each",
            "args": {"items": "[{x: 0, m: \"oak\"}, {x: 2, m: \"ash\"}]", "as": "p",
                     "do": "box([p.x, 0, 0], [p.x + 0.1, 0.1, 1])"}}]));
        let ev = run(&d, &Input::default(), &NoNesting).unwrap();
        let items = ev.items("posts").unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].index, Some(2));
        let item = RepresentationItem {
            geometry: "posts".into(),
            material: Some("each.m".into()),
            hatch: None,
            class: None,
            role: None,
            part: None,
        };
        assert_eq!(
            resolve_material(&item, &ev.env, &items[1]).unwrap(),
            Some(Value::String("ash".into()))
        );
    }

    #[test]
    fn inline_operations_use_the_step_plane() {
        let d = def(json!([{"id": "body", "op": "extrude", "plane": "xz",
            "args": {"profile": "rectangle([0, 0], [size, 2])", "from": 0, "to": 0.5}}]));
        let ev = run(&d, &Input::default(), &NoNesting).unwrap();
        let s = ev.items("body").unwrap()[0].solid().unwrap().clone();
        assert!(
            s.points.iter().any(|p| (p[2] - 2.0).abs() < 1e-12),
            "v maps to z in the xz plane"
        );
        assert!((s.volume() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn errors_name_the_step() {
        let d = def(
            json!([{"id": "body", "op": "box", "args": {"min": "[0, 0, 0]", "max": "missing_name"}}]),
        );
        let e = run(&d, &Input::default(), &NoNesting)
            .unwrap_err()
            .to_string();
        assert!(
            e.contains("recipe step body") && e.contains("missing_name"),
            "{e}"
        );
    }

    struct SelfNesting(TypeDefinition);
    impl Resolver for SelfNesting {
        fn resolve(&self, _: &str) -> Option<TypeDefinition> {
            Some(self.0.clone())
        }
    }

    #[test]
    fn nested_objects_and_their_materials() {
        let inner = def(json!([body()]));
        let outer = def(json!([{"id": "body", "op": "instance",
            "args": {"object": "\"opo-format.org/test/box\"", "values": "{size: 0.5}"}}]));
        let ev = run(&outer, &Input::default(), &SelfNesting(inner)).unwrap();
        let items = ev.items("body").unwrap();
        assert!((volume(&ev, "body") - 0.125).abs() < 1e-12);
        assert_eq!(items[0].material, Some(Value::String("oak".into())));
        assert_eq!(items[0].nested.as_deref(), Some("opo-format.org/test/box"));
        assert!(matches!(
            run(&outer, &Input::default(), &NoNesting),
            Err(Error::Resolve(_))
        ));
    }

    #[test]
    fn nesting_depth_is_limited() {
        let looping = def(
            json!([{"id": "body", "op": "instance", "args": {"object": "\"opo-format.org/test/box\""}}]),
        );
        let e = run(&looping, &Input::default(), &SelfNesting(looping.clone())).unwrap_err();
        assert!(e.to_string().contains("nested deeper"), "{e}");
    }

    #[test]
    fn context_merges_into_defaults() {
        let d = def(json!([body()]));
        let mut input = Input::default();
        input
            .context
            .insert("host".into(), json!({"thickness": 0.3}));
        let ev = run(&d, &input, &NoNesting).unwrap();
        assert_eq!(ev.eval("host.thickness").unwrap(), Value::Number(0.3));
        assert_eq!(ev.eval("host.top").unwrap(), Value::Number(3.0));
        assert_eq!(ev.eval("rules.accessibility").unwrap(), Value::Null);
    }
}
