use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value as Json};

use crate::attributes::*;

/// Every `opo::` attribute declared by schema 0.1 (`schema/opo@v0.1.ifcx`).
pub const ATTRIBUTE_NAMES: [&str; 18] = [
    "opo::object",
    "opo::parameters",
    "opo::slots",
    "opo::rules",
    "opo::recipe",
    "opo::representations",
    "opo::host",
    "opo::handles",
    "opo::materials",
    "opo::ifc",
    "opo::values",
    "opo::inputs",
    "opo::representation",
    "opo::hatches",
    "opo::ports",
    "opo::line_class",
    "opo::note",
    "opo::ruleset",
];

/// An attribute that does not match its schema.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelError {
    /// The attribute name, e.g. `opo::parameters`.
    pub attribute: String,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.attribute, self.message)
    }
}

impl std::error::Error for ModelError {}

/// The `opo::` definition carried by a type node (spec §5.1): everything needed to evaluate
/// the object.
#[derive(Debug, Clone, PartialEq)]
pub struct TypeDefinition {
    /// `opo::object`.
    pub object: Object,
    /// `opo::parameters`.
    pub parameters: Vec<Parameter>,
    /// `opo::slots`.
    pub slots: Option<Vec<Slot>>,
    /// `opo::rules`.
    pub rules: Option<Rules>,
    /// `opo::recipe`.
    pub recipe: Vec<RecipeStep>,
    /// `opo::representations`.
    pub representations: Vec<Representation>,
    /// `opo::host`.
    pub host: Option<Host>,
    /// `opo::handles`.
    pub handles: Option<Vec<Handle>>,
    /// `opo::materials`, in file order.
    pub materials: Option<Vec<(String, Material)>>,
    /// `opo::hatches`, in file order.
    pub hatches: Option<Vec<(String, Hatch)>>,
    /// `opo::ports`.
    pub ports: Option<Vec<Port>>,
    /// `opo::ifc`.
    pub ifc: Ifc,
}

fn read<T: DeserializeOwned>(
    attrs: &Map<String, Json>,
    name: &str,
) -> Result<Option<T>, ModelError> {
    match attrs.get(name) {
        None | Some(Json::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|e| ModelError {
                attribute: name.into(),
                message: e.to_string(),
            }),
    }
}

fn required<T: DeserializeOwned>(attrs: &Map<String, Json>, name: &str) -> Result<T, ModelError> {
    read(attrs, name)?.ok_or_else(|| ModelError {
        attribute: name.into(),
        message: "missing".into(),
    })
}

/// A map attribute (materials, hatches) as ordered (key, value) pairs.
fn read_map<T: DeserializeOwned>(
    attrs: &Map<String, Json>,
    name: &str,
) -> Result<Option<Vec<(String, T)>>, ModelError> {
    let Some(m) = read::<Map<String, Json>>(attrs, name)? else {
        return Ok(None);
    };
    m.into_iter()
        .map(|(k, v)| {
            serde_json::from_value(v)
                .map(|t| (k.clone(), t))
                .map_err(|e| ModelError {
                    attribute: name.into(),
                    message: format!("{k}: {e}"),
                })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn write_map<T: Serialize>(attrs: &mut Map<String, Json>, name: &str, entries: &[(String, T)]) {
    let m: Map<String, Json> = entries
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                serde_json::to_value(v).expect("model types serialise"),
            )
        })
        .collect();
    attrs.insert(name.into(), Json::Object(m));
}

fn write<T: Serialize>(attrs: &mut Map<String, Json>, name: &str, value: &T) {
    attrs.insert(
        name.into(),
        serde_json::to_value(value).expect("model types serialise"),
    );
}

impl TypeDefinition {
    /// Read the `opo::` attributes of a type node. `opo::object`, `opo::parameters`,
    /// `opo::recipe`, `opo::representations` and `opo::ifc` are required; the others may be
    /// absent. Attributes that are not `opo::` are ignored.
    pub fn from_attributes(attrs: &Map<String, Json>) -> Result<Self, ModelError> {
        Ok(TypeDefinition {
            object: required(attrs, "opo::object")?,
            parameters: required(attrs, "opo::parameters")?,
            slots: read(attrs, "opo::slots")?,
            rules: read(attrs, "opo::rules")?,
            recipe: required(attrs, "opo::recipe")?,
            representations: required(attrs, "opo::representations")?,
            host: read(attrs, "opo::host")?,
            handles: read(attrs, "opo::handles")?,
            materials: read_map(attrs, "opo::materials")?,
            hatches: read_map(attrs, "opo::hatches")?,
            ports: read(attrs, "opo::ports")?,
            ifc: required(attrs, "opo::ifc")?,
        })
    }

    /// Write the definition back as `opo::` attributes. Optional attributes are written only
    /// when present (an empty list stays an empty list).
    pub fn to_attributes(&self) -> Map<String, Json> {
        let mut a = Map::new();
        write(&mut a, "opo::object", &self.object);
        write(&mut a, "opo::parameters", &self.parameters);
        if let Some(v) = &self.slots {
            write(&mut a, "opo::slots", v);
        }
        if let Some(v) = &self.rules {
            write(&mut a, "opo::rules", v);
        }
        write(&mut a, "opo::recipe", &self.recipe);
        write(&mut a, "opo::representations", &self.representations);
        if let Some(v) = &self.host {
            write(&mut a, "opo::host", v);
        }
        if let Some(v) = &self.handles {
            write(&mut a, "opo::handles", v);
        }
        if let Some(v) = &self.materials {
            write_map(&mut a, "opo::materials", v);
        }
        if let Some(v) = &self.hatches {
            write_map(&mut a, "opo::hatches", v);
        }
        if let Some(v) = &self.ports {
            write(&mut a, "opo::ports", v);
        }
        write(&mut a, "opo::ifc", &self.ifc);
        a
    }

    /// The derived values, or none.
    pub fn derived(&self) -> &[Derived] {
        self.rules
            .as_ref()
            .and_then(|r| r.derived.as_deref())
            .unwrap_or_default()
    }

    /// The checks, or none.
    pub fn checks(&self) -> &[Check] {
        self.rules
            .as_ref()
            .and_then(|r| r.checks.as_deref())
            .unwrap_or_default()
    }

    /// Check a rule-set node against this definition: its id must be declared in
    /// `opo::rules.rulesets`, and every value must name a declared field (when fields are
    /// declared).
    pub fn check_rule_set(&self, node: &RuleSetNode) -> Vec<String> {
        let declared = self
            .rules
            .as_ref()
            .and_then(|r| r.rulesets.as_deref())
            .unwrap_or_default();
        let Some(decl) = declared.iter().find(|d| d.id == node.id) else {
            return vec![format!(
                "rule set '{}' is not declared by the object",
                node.id
            )];
        };
        match &decl.fields {
            Some(fields) => node
                .values
                .keys()
                .filter(|k| !fields.contains_key(*k))
                .map(|k| format!("rule set '{}': '{k}' is not a declared field", node.id))
                .collect(),
            None => Vec::new(),
        }
    }

    /// The parameter with this name.
    pub fn parameter(&self, name: &str) -> Option<&Parameter> {
        self.parameters.iter().find(|p| p.name == name)
    }

    /// The representation with this id.
    pub fn representation(&self, id: &str) -> Option<&Representation> {
        self.representations.iter().find(|r| r.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn minimal() -> Map<String, Json> {
        json!({
            "opo::object": {"id": "t", "name": "T", "version": "0.1.0", "opo": "0.1", "license": "CC0-1.0",
                            "author": "a", "description": "d", "references": []},
            "opo::parameters": [{"name": "width", "type": "length", "default": 0.9, "label": "Width", "group": "Size"}],
            "opo::recipe": [{"id": "body", "op": "box", "args": {"min": "[0, 0, 0]", "max": "[width, 1, 1]"}}],
            "opo::representations": [{"id": "model.medium", "context": "model", "lod": "medium", "source": "authored",
                                      "ifc": {"context": "Body", "targetView": "MODEL_VIEW"}, "items": [{"geometry": "body"}]}],
            "opo::ifc": {"class": "IfcFurniture"}
        })
        .as_object()
        .unwrap()
        .clone()
    }

    fn issues(edit: impl FnOnce(&mut Map<String, Json>)) -> Vec<String> {
        let mut a = minimal();
        edit(&mut a);
        TypeDefinition::from_attributes(&a)
            .unwrap()
            .validate()
            .into_iter()
            .map(|i| i.to_string())
            .collect()
    }

    #[test]
    fn minimal_definition_is_valid_and_round_trips() {
        let def = TypeDefinition::from_attributes(&minimal()).unwrap();
        assert!(def.validate().is_empty());
        assert_eq!(Json::Object(def.to_attributes()), Json::Object(minimal()));
        assert_eq!(def.parameter("width").unwrap().kind, ParameterType::Length);
        assert!(def.representation("model.medium").is_some());
        assert!(def.derived().is_empty() && def.checks().is_empty());
    }

    #[test]
    fn missing_required_attribute() {
        let mut a = minimal();
        a.shift_remove("opo::ifc");
        let e = TypeDefinition::from_attributes(&a).unwrap_err();
        assert_eq!(e.attribute, "opo::ifc");
    }

    #[test]
    fn unknown_field_in_closed_object_is_rejected() {
        let mut a = minimal();
        a["opo::parameters"][0]["colour"] = json!("red");
        assert!(TypeDefinition::from_attributes(&a).is_err());
    }

    #[test]
    fn unknown_operation_is_rejected() {
        let mut a = minimal();
        a["opo::recipe"][0]["op"] = json!("loft");
        assert!(TypeDefinition::from_attributes(&a).is_err());
    }

    #[test]
    fn wrong_recipe_arguments() {
        let v = issues(|a| a["opo::recipe"][0]["args"] = json!({"min": "[0, 0, 0]", "size": 1}));
        assert!(v.iter().any(|m| m.contains("needs argument 'max'")));
        assert!(v.iter().any(|m| m.contains("has no argument 'size'")));
    }

    #[test]
    fn default_must_fit_type() {
        let v = issues(|a| a["opo::parameters"][0]["default"] = json!("wide"));
        assert!(v[0].contains("default does not fit"));
    }

    #[test]
    fn choice_needs_options_containing_default() {
        let v = issues(|a| {
            a["opo::parameters"][0] = json!({"name": "side", "type": "choice", "default": "up", "label": "S", "group": "G",
                                             "options": ["left", "right"]})
        });
        assert!(v[0].contains("not one of the options"));
    }

    #[test]
    fn names_must_be_snake_case_and_unique() {
        let v = issues(|a| {
            let p = a["opo::parameters"][0].clone();
            let arr = a["opo::parameters"].as_array_mut().unwrap();
            arr.push(p);
            arr[0]["name"] = json!("Width");
        });
        assert!(v.iter().any(|m| m.contains("snake_case")));
        let v = issues(|a| {
            let p = a["opo::parameters"][0].clone();
            a["opo::parameters"].as_array_mut().unwrap().push(p);
        });
        assert!(v.iter().any(|m| m.contains("duplicate")));
    }

    #[test]
    fn representation_items_must_be_recipe_steps() {
        let v = issues(|a| a["opo::representations"][0]["items"][0]["geometry"] = json!("missing"));
        assert!(v[0].contains("not a recipe step"));
    }

    #[test]
    fn derived_representation_needs_derive() {
        let v = issues(|a| a["opo::representations"][0]["source"] = json!("derived"));
        assert!(v.iter().any(|m| m.contains("needs derive")));
    }

    #[test]
    fn handle_needs_exactly_one_target() {
        let v = issues(|a| {
            a.insert(
                "opo::handles".into(),
                json!([{"parameter": "width", "slot": "path"}]),
            );
        });
        assert!(v[0].contains("exactly one"));
    }

    #[test]
    fn lines_hatch_needs_lines_and_scale_mode() {
        let v = issues(|a| {
            a.insert(
                "opo::hatches".into(),
                json!({"steel": {"label": "Steel", "kind": "lines"}}),
            );
        });
        assert!(v[0].contains("lines and scale_mode"));
    }

    #[test]
    fn material_hatch_must_exist() {
        let v = issues(|a| {
            a.insert(
                "opo::hatches".into(),
                json!({"solid": {"label": "Solid", "kind": "solid"}}),
            );
            a.insert("opo::materials".into(), json!({"steel": {"label": "Steel", "category": "steel", "cut": {"hatch": "lines45"}}}));
        });
        assert!(v[0].contains("not in opo::hatches"));
    }

    #[test]
    fn ifc_class_and_versions_are_checked() {
        let v = issues(|a| {
            a["opo::ifc"]["class"] = json!("Door");
            a["opo::object"]["version"] = json!("1.0");
            a["opo::object"]["opo"] = json!("0.1.0");
        });
        assert_eq!(v.len(), 3);
    }

    #[test]
    fn ports_enumerations_are_checked() {
        let v = issues(|a| {
            a.insert("opo::ports".into(), json!([{"name": "inlet", "system_type": "DOMESTICCOLDWATER",
                "flow_direction": "IN", "port_type": "PIPE", "nominal_size": "0.02", "position": "[0,0,0]", "direction": "[0,0,1]"}]));
        });
        assert!(v[0].contains("flow_direction"));
    }

    #[test]
    fn empty_optional_attribute_is_kept() {
        let mut a = minimal();
        a.insert("opo::slots".into(), json!([]));
        let def = TypeDefinition::from_attributes(&a).unwrap();
        assert_eq!(def.to_attributes()["opo::slots"], json!([]));
    }

    #[test]
    fn rule_set_nodes_are_checked_against_declarations() {
        let mut a = minimal();
        a.insert("opo::rules".into(), json!({"rulesets": [{"id": "stairs", "default": null, "fields": {"riser_max": "length"}}]}));
        let def = TypeDefinition::from_attributes(&a).unwrap();
        let node = |id: &str, field: &str| -> RuleSetNode {
            serde_json::from_value(
                json!({"id": id, "source": "test", "values": {field: "mm(220)"}}),
            )
            .unwrap()
        };
        assert!(def.check_rule_set(&node("stairs", "riser_max")).is_empty());
        assert_eq!(def.check_rule_set(&node("guards", "riser_max")).len(), 1);
        assert_eq!(def.check_rule_set(&node("stairs", "going_min")).len(), 1);
    }

    #[test]
    fn rule_set_declaration_rejects_values() {
        let mut a = minimal();
        a.insert(
            "opo::rules".into(),
            json!({"rulesets": [{"id": "stairs", "values": {}}]}),
        );
        assert!(TypeDefinition::from_attributes(&a).is_err());
    }

    #[test]
    fn op_names_and_arguments() {
        for op in Op::ALL {
            assert_eq!(Op::from_name(op.name()), Some(op));
            assert!(!op.arguments().0.is_empty());
        }
        assert_eq!(Op::from_name("loft"), None);
    }
}
