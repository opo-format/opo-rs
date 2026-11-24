//! Rules of schema 0.1 that serde cannot express.

use std::collections::HashSet;
use std::fmt;

use serde_json::Value as Json;

use crate::attributes::*;
use crate::definition::TypeDefinition;

/// One problem found by [`TypeDefinition::validate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// Where: attribute and entry, e.g. `opo::recipe[frame]`.
    pub at: String,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for Issue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.at, self.message)
    }
}

/// `^[a-z][a-z0-9_]*$`
fn is_snake(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some('a'..='z'))
        && c.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == '_')
}

fn is_ifc_class(s: &str) -> bool {
    s.len() > 3
        && s.starts_with("Ifc")
        && s[3..].starts_with(|c: char| c.is_ascii_uppercase())
        && s[3..].chars().all(|c| c.is_ascii_alphanumeric())
}

fn is_colour(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

fn is_semver(s: &str) -> bool {
    let core = s.split_once('-').map_or(s, |(c, _)| c);
    let parts: Vec<_> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

/// `(model|plan|rcp|elevation|section|axis)(.(low|medium|high))?`
fn is_representation_id(s: &str) -> bool {
    let (ctx, lod) = s.split_once('.').map_or((s, None), |(c, l)| (c, Some(l)));
    ["model", "plan", "rcp", "elevation", "section", "axis"].contains(&ctx)
        && lod.is_none_or(|l| ["low", "medium", "high"].contains(&l))
}

impl TypeDefinition {
    /// Check the schema rules that the types alone do not enforce. Returns every issue found;
    /// an empty list means the definition is valid.
    pub fn validate(&self) -> Vec<Issue> {
        let mut out = Vec::new();
        let mut issue = |at: String, message: String| out.push(Issue { at, message });

        let o = &self.object;
        if !is_semver(&o.version) {
            issue(
                "opo::object.version".into(),
                format!("'{}' is not a semantic version", o.version),
            );
        }
        let mut opo = o.opo.split('.');
        let ok = matches!((opo.next(), opo.next(), opo.next()), (Some(a), Some(b), None)
            if !a.is_empty() && !b.is_empty() && a.chars().chain(b.chars()).all(|c| c.is_ascii_digit()));
        if !ok {
            issue(
                "opo::object.opo".into(),
                format!("'{}' is not major.minor", o.opo),
            );
        }

        let mut names = HashSet::new();
        for p in &self.parameters {
            let at = format!("opo::parameters[{}]", p.name);
            if !names.insert(p.name.as_str()) {
                issue(at.clone(), "duplicate name".into());
            }
            let pset = p.kind == ParameterType::PropertySet;
            let name_ok = if pset {
                p.name.starts_with(|c: char| c.is_ascii_alphabetic())
                    && p.name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_')
            } else {
                is_snake(&p.name)
            };
            if !name_ok {
                issue(
                    at.clone(),
                    "name must be snake_case (property sets: IFC name)".into(),
                );
            }
            let d = &p.default;
            let type_ok = match p.kind {
                ParameterType::Length | ParameterType::Angle | ParameterType::Number => {
                    d.is_number()
                }
                ParameterType::Integer => d.as_f64().is_some_and(|x| x.fract() == 0.0),
                ParameterType::Boolean => d.is_boolean(),
                ParameterType::Text
                | ParameterType::Material
                | ParameterType::Colour
                | ParameterType::Choice => d.is_string(),
                ParameterType::Layers => {
                    serde_json::from_value::<Vec<Layer>>(d.clone()).is_ok_and(|l| {
                        !l.is_empty()
                            && l.iter().all(|x| x.thickness > 0.0 && is_snake(&x.material))
                    })
                }
                ParameterType::PropertySet => d.is_object(),
            };
            if !type_ok {
                issue(
                    at.clone(),
                    format!("default does not fit type {:?}", p.kind),
                );
            }
            if p.kind == ParameterType::Choice {
                match &p.options {
                    Some(Options::List(l)) if !l.is_empty() => {
                        if d.as_str().is_some_and(|s| !l.iter().any(|x| x == s)) {
                            issue(at.clone(), "default is not one of the options".into());
                        }
                    }
                    _ => issue(at.clone(), "a choice needs a list of options".into()),
                }
            }
            if let Some(Options::Keyword(k)) = &p.options {
                if k != "materials" {
                    issue(at.clone(), format!("unknown options keyword '{k}'"));
                }
            }
            if pset && p.properties.is_none() {
                issue(at.clone(), "a property_set needs properties".into());
            }
        }

        for s in self.slots.iter().flatten() {
            if !is_snake(&s.name) {
                issue(
                    format!("opo::slots[{}]", s.name),
                    "name must be snake_case".into(),
                );
            }
        }

        for d in self.derived() {
            if !is_snake(&d.name) {
                issue(
                    format!("opo::rules.derived[{}]", d.name),
                    "name must be snake_case".into(),
                );
            }
        }
        for c in self.checks() {
            if !is_snake(&c.id) {
                issue(
                    format!("opo::rules.checks[{}]", c.id),
                    "id must be snake_case".into(),
                );
            }
        }

        let mut steps = HashSet::new();
        for s in &self.recipe {
            let at = format!("opo::recipe[{}]", s.id);
            if !is_snake(&s.id) {
                issue(at.clone(), "id must be snake_case".into());
            }
            if !steps.insert(s.id.as_str()) {
                issue(at.clone(), "duplicate id".into());
            }
            let (req, opt) = s.op.arguments();
            for r in req {
                if !s.args.contains_key(*r) {
                    issue(at.clone(), format!("{} needs argument '{r}'", s.op.name()));
                }
            }
            for k in s.args.keys() {
                if !req.contains(&k.as_str()) && !opt.contains(&k.as_str()) {
                    issue(at.clone(), format!("{} has no argument '{k}'", s.op.name()));
                }
            }
        }

        for r in &self.representations {
            let at = format!("opo::representations[{}]", r.id);
            if !is_representation_id(&r.id) {
                issue(at.clone(), "id must be context or context.lod".into());
            }
            match r.source {
                RepresentationSource::Authored if r.items.is_none() => {
                    issue(at.clone(), "an authored representation needs items".into())
                }
                RepresentationSource::Derived if r.derive.is_none() => {
                    issue(at.clone(), "a derived representation needs derive".into())
                }
                _ => {}
            }
            for it in r.items.iter().chain(r.overlay.iter()).flatten() {
                if !steps.contains(it.geometry.as_str()) {
                    issue(
                        at.clone(),
                        format!("item geometry '{}' is not a recipe step", it.geometry),
                    );
                }
            }
        }

        for (i, h) in self.handles.iter().flatten().enumerate() {
            if h.parameter.is_some() == h.slot.is_some() {
                issue(
                    format!("opo::handles[{i}]"),
                    "exactly one of parameter and slot".into(),
                );
            }
        }

        let hatch_keys: HashSet<&str> = self
            .hatches
            .iter()
            .flatten()
            .map(|(k, _)| k.as_str())
            .collect();
        for (k, m) in self.materials.iter().flatten() {
            let at = format!("opo::materials[{k}]");
            if !is_snake(k) {
                issue(at.clone(), "key must be snake_case".into());
            }
            if m.colour.as_deref().is_some_and(|c| !is_colour(c)) {
                issue(at.clone(), "colour must be #rrggbb".into());
            }
            for h in std::iter::once(&m.cut).chain(m.surface.as_ref()) {
                if !is_snake(&h.hatch) {
                    issue(
                        at.clone(),
                        format!("hatch key '{}' must be snake_case", h.hatch),
                    );
                } else if !hatch_keys.is_empty() && !hatch_keys.contains(h.hatch.as_str()) {
                    issue(
                        at.clone(),
                        format!("hatch '{}' is not in opo::hatches", h.hatch),
                    );
                }
            }
            if m.density.is_some_and(|x| x <= 0.0)
                || m.thermal_conductivity.is_some_and(|x| x <= 0.0)
            {
                issue(at.clone(), "density and conductivity must be > 0".into());
            }
        }
        for (k, h) in self.hatches.iter().flatten() {
            let at = format!("opo::hatches[{k}]");
            if !is_snake(k) {
                issue(at.clone(), "key must be snake_case".into());
            }
            if h.kind == HatchKind::Lines
                && (h.scale_mode.is_none() || h.lines.as_ref().is_none_or(|l| l.is_empty()))
            {
                issue(
                    at.clone(),
                    "a lines hatch needs lines and scale_mode".into(),
                );
            }
            for c in h.colour.iter().chain(h.background.iter()) {
                if !is_colour(c) {
                    issue(at.clone(), "colours must be #rrggbb".into());
                }
            }
        }

        for p in self.ports.iter().flatten() {
            let at = format!("opo::ports[{}]", p.name);
            if !is_snake(&p.name) {
                issue(at.clone(), "name must be snake_case".into());
            }
            if !["SOURCE", "SINK", "SOURCEANDSINK", "NOTDEFINED"]
                .contains(&p.flow_direction.as_str())
            {
                issue(
                    at.clone(),
                    format!("unknown flow_direction {}", p.flow_direction),
                );
            }
            if ![
                "CABLE",
                "CABLECARRIER",
                "DUCT",
                "PIPE",
                "WIRELESS",
                "USERDEFINED",
                "NOTDEFINED",
            ]
            .contains(&p.port_type.as_str())
            {
                issue(at.clone(), format!("unknown port_type {}", p.port_type));
            }
        }

        if !is_ifc_class(&self.ifc.class) {
            issue(
                "opo::ifc.class".into(),
                format!("'{}' is not an IFC class name", self.ifc.class),
            );
        }
        match &self.ifc.property_sets {
            None | Some(Json::String(_)) => {}
            Some(Json::Object(m))
                if m.values()
                    .all(|v| v.as_array().is_some_and(|a| a.iter().all(Json::is_string))) => {}
            Some(_) => issue(
                "opo::ifc.property_sets".into(),
                "must be a name or a map of names to lists".into(),
            ),
        }
        if let Err(e) = self.ifc.quantity_sets() {
            issue("opo::ifc.quantities".into(), e.to_string());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns() {
        assert!(is_snake("leaf_width2") && !is_snake("Leaf") && !is_snake("2a") && !is_snake(""));
        assert!(is_ifc_class("IfcDoor") && !is_ifc_class("Ifcdoor") && !is_ifc_class("Ifc"));
        assert!(is_colour("#a0B1c2") && !is_colour("#a0b1c") && !is_colour("a0b1c2f"));
        assert!(is_semver("0.3.0") && is_semver("1.0.0-rc.1") && !is_semver("1.0"));
        assert!(
            is_representation_id("model.medium")
                && is_representation_id("plan")
                && !is_representation_id("model.max")
        );
    }
}
