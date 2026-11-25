//! One type per `opo::` attribute schema.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as Json};

/// Deserialise a field so that an explicit `null` is kept (`Some(Json::Null)`), unlike a
/// missing field (`None`).
fn keep_null<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Json>, D::Error> {
    Json::deserialize(d).map(Some)
}

/// A FEEL expression (spec §4.4), kept as written.
pub type Expr = String;

/// An ordered map of names to JSON values (used for open objects).
pub type JsonMap = Map<String, Json>;

// ---------------------------------------------------------------- opo::object

/// `opo::object`: identity of the object (spec §4.1, §4.14).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    /// Unique id, such as `opo:library/templates/IfcWall`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// What the package holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ObjectKind>,
    /// The archetype of a class template (spec §4.16).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archetype: Option<Archetype>,
    /// Object version (semantic versioning).
    pub version: String,
    /// OPO spec version, `major.minor`.
    pub opo: String,
    /// Licence (SPDX expression or text).
    pub license: String,
    /// Author.
    pub author: String,
    /// Description.
    pub description: String,
    /// Tags.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Sources the object is based on (spec §4.14).
    pub references: Vec<Reference>,
}

/// `opo::object.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    /// A library object.
    Object,
    /// A generated class template.
    ClassTemplate,
    /// Materials and hatches only.
    Styles,
}

/// The archetypes of spec §4.16.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum Archetype {
    Point,
    Linear,
    Area,
    Wall,
    Hosted,
    Flight,
    Fitting,
    Assembly,
    Any,
}

/// One entry of `opo::object.references`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reference {
    /// Role: parametric basis, IFC parameters, norm, rule source…
    pub role: String,
    /// Name of the source.
    pub name: String,
    /// What was taken from it.
    pub used: String,
    /// Licence of the source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// Link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    /// Version of the source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

// ---------------------------------------------------------------- opo::parameters

/// Parameter types (spec §4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum ParameterType {
    Length,
    Angle,
    Number,
    Integer,
    Boolean,
    Choice,
    Text,
    Material,
    Colour,
    Layers,
    PropertySet,
}

/// One entry of `opo::parameters`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    /// snake_case name; property-set parameters use the IFC name (`Pset_…`).
    pub name: String,
    /// Type.
    #[serde(rename = "type")]
    pub kind: ParameterType,
    /// Default value; its JSON type follows `kind`.
    pub default: Json,
    /// UI label.
    pub label: String,
    /// UI group.
    pub group: String,
    /// Help text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// FEEL condition: show the parameter only when true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<Expr>,
    /// The parameter may be left out (property sets that apply to many classes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optional: Option<bool>,
    /// Choices (`choice`), or the string `"materials"` (`material`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Options>,
    /// UI labels per option.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option_labels: Option<Map<String, Json>>,
    /// Limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<Limits>,
    /// Properties of a `property_set` parameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub properties: Option<Vec<PsetProperty>>,
}

/// `options` of a parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Options {
    /// A list of choices.
    List(Vec<String>),
    /// A keyword: `"materials"` (the object's materials).
    Keyword(String),
}

/// Parameter limits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// Minimum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Maximum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    /// Step, one value or per unit system.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<Step>,
    /// Layer thickness limits (`layers` parameters).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thickness: Option<Range>,
}

/// A step, one value or one per unit system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Step {
    /// One step for all unit systems.
    One(f64),
    /// Metric and imperial steps.
    PerSystem {
        /// Metric step.
        metric: f64,
        /// Imperial step.
        imperial: f64,
    },
}

/// A min/max range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Range {
    /// Minimum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    /// Maximum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

/// One property of a `property_set` parameter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PsetProperty {
    /// IFC property name.
    pub name: String,
    /// IFC measure type, or null when IFC gives none.
    pub measure: Option<String>,
    /// Enumeration values.
    #[serde(default, rename = "enum", skip_serializing_if = "Option::is_none")]
    pub enumeration: Option<Vec<String>>,
    /// FEEL expression that sets the value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Expr>,
}

/// One layer of a `layers` value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    /// Material key.
    pub material: String,
    /// Thickness in metres, > 0.
    pub thickness: f64,
    /// Function of the layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function: Option<LayerFunction>,
    /// The structural core.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core: Option<bool>,
}

/// Layer functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum LayerFunction {
    Structure,
    Finish,
    Insulation,
    Membrane,
    Air,
    Substrate,
    Other,
}

// ---------------------------------------------------------------- opo::slots

/// Input slot geometry types (spec §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum SlotType {
    Point,
    Polyline,
    Polyline3,
    Polygon,
    PolygonList,
    Curve,
    Region,
}

/// Planes (spec §4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plane {
    /// Plan, normal +z.
    Xy,
    /// Object face, normal +y.
    Xz,
    /// Side, normal +x.
    Yz,
    /// Local u/v plane of a swept profile.
    Profile,
}

/// One entry of `opo::slots`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slot {
    /// snake_case name.
    pub name: String,
    /// Geometry type.
    #[serde(rename = "type")]
    pub kind: SlotType,
    /// Plane the geometry is drawn in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plane: Option<Plane>,
    /// Default geometry, or null when the slot starts empty. Always written.
    pub default: Option<Vec<Json>>,
    /// The slot may stay empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optional: Option<bool>,
    /// Validity rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<SlotRules>,
    /// Help text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Validity rules of a slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SlotRules {
    /// The curve must be open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open: Option<bool>,
    /// The curve must be closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<bool>,
    /// The curve must not cross itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub non_self_intersecting: Option<bool>,
    /// Minimum segment length in metres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_segment_length: Option<f64>,
    /// Must lie inside another slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inside: Option<String>,
    /// Must fit the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fits_host: Option<bool>,
    /// Other rules.
    #[serde(flatten)]
    pub extra: JsonMap,
}

// ---------------------------------------------------------------- opo::rules

/// `opo::rules`: derived values, checks and rule sets (spec §4.4).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    /// Derived values, evaluated in order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived: Option<Vec<Derived>>,
    /// Checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks: Option<Vec<Check>>,
    /// Rule sets the object reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rulesets: Option<Vec<RuleSet>>,
}

/// A derived value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Derived {
    /// snake_case name.
    pub name: String,
    /// FEEL expression.
    pub expr: Expr,
    /// Note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Check {
    /// snake_case id.
    pub id: String,
    /// FEEL condition that must be true.
    pub expr: Expr,
    /// Severity.
    pub severity: Severity,
    /// Message shown when the check fails.
    pub message: String,
}

/// Check severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// A rule set an object declares in `opo::rules.rulesets` (spec §4.4). Its values come from a
/// rule-set node ([`RuleSetNode`], decision 0020).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSet {
    /// snake_case id (e.g. `accessibility`); formulas read `rules.<id>.<field>`.
    pub id: String,
    /// Default rule set, or null for none. Kept exactly as written (absent, null or text).
    #[serde(
        default,
        deserialize_with = "keep_null",
        skip_serializing_if = "Option::is_none"
    )]
    pub default: Option<Json>,
    /// Note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Field descriptions: field name to type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Map<String, Json>>,
}

/// `opo::ruleset`: one regional rule set on its own node, usually in a layer (spec §4.4,
/// decision 0020).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSetNode {
    /// The declared rule-set id it gives values for.
    pub id: String,
    /// Display name, e.g. `ISO 21542:2021`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Link to the rule set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    /// The regulation and clauses the values come from.
    pub source: String,
    /// Note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Field name to FEEL expression or literal.
    pub values: JsonMap,
}

// ------------------------------------------------------------- opo::provenance

/// Where a value comes from (decision 0017).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceStatus {
    /// Taken from a norm.
    Norm,
    /// Taken from a maker's data.
    Maker,
    /// Common practice, not from a single source.
    Typical,
    /// Chosen to illustrate; not from a source.
    Illustrative,
    /// Not yet checked against its source.
    ToVerify,
}

/// One entry of `opo::provenance`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvenanceEntry {
    /// Id of the source in the library's references registry, or a citation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Status of the value.
    pub status: ProvenanceStatus,
    /// What the value is, or what to check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// `opo::provenance`: value path on the same node (e.g. `values.nominal_width`,
/// `parameters.handle_height.default`, `ruleset.values.min_clear_width`) → entry.
pub type Provenance = std::collections::BTreeMap<String, ProvenanceEntry>;

/// Problems in an `opo::provenance` map: path shape, a missing source for `norm` and
/// `maker`, a missing note for the other statuses, empty texts.
pub fn check_provenance(p: &Provenance) -> Vec<String> {
    let mut errs = Vec::new();
    for (path, e) in p {
        let mut parts = path.split('.');
        let head_ok = parts.next().is_some_and(|h| {
            h.starts_with(|c: char| c.is_ascii_lowercase())
                && h.chars().all(|c| c.is_ascii_lowercase() || c == '_')
        });
        let rest: Vec<&str> = parts.collect();
        if !head_ok
            || rest.is_empty()
            || rest
                .iter()
                .any(|s| s.is_empty() || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))
        {
            errs.push(format!(
                "provenance path '{path}' is not attribute.key[.key…]"
            ));
        }
        let empty = |t: &Option<String>| t.as_deref().is_some_and(str::is_empty);
        if empty(&e.source) || empty(&e.note) {
            errs.push(format!(
                "provenance '{path}': source and note must not be empty"
            ));
        }
        match e.status {
            ProvenanceStatus::Norm | ProvenanceStatus::Maker if e.source.is_none() => errs.push(
                format!("provenance '{path}': status {:?} needs a source", e.status),
            ),
            ProvenanceStatus::Typical
            | ProvenanceStatus::Illustrative
            | ProvenanceStatus::ToVerify
                if e.note.is_none() =>
            {
                errs.push(format!(
                    "provenance '{path}': status {:?} needs a note",
                    e.status
                ))
            }
            _ => {}
        }
    }
    errs
}

// ---------------------------------------------------------------- opo::recipe

/// Recipe operations (spec §4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum Op {
    Rectangle,
    Polygon,
    Circle,
    Polyline,
    Line,
    Arc,
    Band,
    Difference,
    Union,
    BoundaryCurves,
    Text,
    Extrude,
    Sweep,
    Revolve,
    Box,
    Cylinder,
    Path3,
    Move,
    Rotate,
    Mirror,
    ForEach,
    Instance,
    Grid,
}

impl Op {
    /// Every operation.
    pub const ALL: [Op; 23] = [
        Op::Rectangle,
        Op::Polygon,
        Op::Circle,
        Op::Polyline,
        Op::Line,
        Op::Arc,
        Op::Band,
        Op::Difference,
        Op::Union,
        Op::BoundaryCurves,
        Op::Text,
        Op::Extrude,
        Op::Sweep,
        Op::Revolve,
        Op::Box,
        Op::Cylinder,
        Op::Path3,
        Op::Move,
        Op::Rotate,
        Op::Mirror,
        Op::ForEach,
        Op::Instance,
        Op::Grid,
    ];

    /// The name used in files and in inline FEEL calls.
    pub fn name(self) -> &'static str {
        match self {
            Op::Rectangle => "rectangle",
            Op::Polygon => "polygon",
            Op::Circle => "circle",
            Op::Polyline => "polyline",
            Op::Line => "line",
            Op::Arc => "arc",
            Op::Band => "band",
            Op::Difference => "difference",
            Op::Union => "union",
            Op::BoundaryCurves => "boundary_curves",
            Op::Text => "text",
            Op::Extrude => "extrude",
            Op::Sweep => "sweep",
            Op::Revolve => "revolve",
            Op::Box => "box",
            Op::Cylinder => "cylinder",
            Op::Path3 => "path3",
            Op::Move => "move",
            Op::Rotate => "rotate",
            Op::Mirror => "mirror",
            Op::ForEach => "for_each",
            Op::Instance => "instance",
            Op::Grid => "grid",
        }
    }

    /// The operation with this name.
    pub fn from_name(name: &str) -> Option<Op> {
        Op::ALL.into_iter().find(|o| o.name() == name)
    }

    /// Required and optional argument names, in positional order (spec §4.5 table).
    pub fn arguments(self) -> (&'static [&'static str], &'static [&'static str]) {
        match self {
            Op::Rectangle | Op::Box => (&["min", "max"], &[]),
            Op::Polygon | Op::Polyline | Op::Path3 => (&["points"], &[]),
            Op::Circle => (&["center", "radius"], &["segments"]),
            Op::Line => (&["start", "end"], &[]),
            Op::Arc => (
                &["center", "radius", "start_angle", "end_angle"],
                &["segments"],
            ),
            Op::Band => (&["path", "left", "right"], &[]),
            Op::Difference => (&["a", "b"], &[]),
            Op::Union => (&["items"], &[]),
            Op::BoundaryCurves => (&["region"], &[]),
            Op::Text => (&["at", "value"], &["height"]),
            Op::Extrude => (&["profile", "from", "to"], &[]),
            Op::Sweep => (&["profile", "path"], &["roll"]),
            Op::Revolve => (&["profile", "center", "axis", "angle"], &["segments"]),
            Op::Cylinder => (&["start", "end", "radius"], &["segments"]),
            Op::Move => (&["geometry", "by"], &[]),
            Op::Rotate => (&["geometry", "center", "angle"], &[]),
            Op::Mirror => (&["geometry", "axis"], &[]),
            Op::ForEach => (&["items", "do"], &["as", "index"]),
            Op::Instance => (&["object"], &["values", "inputs", "representation"]),
            Op::Grid => (
                &["surface", "u", "v"],
                &["join", "classes", "overrides", "widths"],
            ),
        }
    }
}

/// One step of `opo::recipe`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecipeStep {
    /// snake_case id, referenced by later steps and representations.
    pub id: String,
    /// Operation.
    pub op: Op,
    /// Plane for 2D operations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plane: Option<Plane>,
    /// FEEL condition; a false condition produces nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Expr>,
    /// Note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Arguments: FEEL expressions or literal JSON values.
    pub args: JsonMap,
}

// ---------------------------------------------------------------- opo::representations

/// Representation contexts (spec §4.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum Context {
    Model,
    Plan,
    Rcp,
    Elevation,
    Section,
    Axis,
}

/// Levels of detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum Lod {
    Low,
    Medium,
    High,
    All,
}

/// Semantic line classes (spec §4.6, decision 0006).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum LineClass {
    Cut,
    Projected,
    Hidden,
    Overhead,
    Annotation,
}

/// One entry of `opo::representations`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Representation {
    /// `context` or `context.lod`, e.g. `model.medium`.
    pub id: String,
    /// Context.
    pub context: Context,
    /// Level of detail.
    pub lod: Lod,
    /// Drawing scale range.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<ScaleRange>,
    /// Authored from recipe steps, or derived from another representation.
    pub source: RepresentationSource,
    /// IFC representation context.
    pub ifc: IfcRepresentation,
    /// Items (authored representations).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<RepresentationItem>>,
    /// Items drawn on top of a derived representation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay: Option<Vec<RepresentationItem>>,
    /// How a derived representation is made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derive: Option<Derive>,
    /// Note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Surface style.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<String>,
}

/// Scale range of a representation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScaleRange {
    /// Smallest scale denominator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_scale: Option<f64>,
    /// Largest scale denominator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_scale: Option<f64>,
}

/// Where a representation comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepresentationSource {
    /// Listed recipe items.
    Authored,
    /// Cut or projected from another representation.
    Derived,
}

/// IFC representation context and target view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IfcRepresentation {
    /// Body, Plan, Profile, Axis, Annotation, Box or FootPrint.
    pub context: String,
    /// IFC target view (`MODEL_VIEW`, `PLAN_VIEW`…).
    #[serde(rename = "targetView")]
    pub target_view: String,
}

/// One item of a representation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepresentationItem {
    /// Recipe step id.
    pub geometry: String,
    /// Material key (or FEEL expression).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    /// Hatch key (or FEEL expression).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hatch: Option<String>,
    /// Line class.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<LineClass>,
    /// Role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Part name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
}

/// How a derived representation is made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Derive {
    /// Representation id to derive from.
    pub from: String,
    /// Method.
    pub method: DeriveMethod,
    /// Parts to leave out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_parts: Option<Vec<String>>,
}

/// Derivation methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum DeriveMethod {
    Cut,
    Project,
    CutOrProject,
}

// ---------------------------------------------------------------- opo::host

/// `opo::host`: host behaviour (spec §4.7). Open: unknown keys are kept in `extra`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Host {
    /// The object is a host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_host: Option<bool>,
    /// IFC classes it accepts as hosted objects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepts: Option<Vec<String>>,
    /// IFC classes it can be hosted by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hosts: Option<Vec<String>>,
    /// Values it provides to hosted objects (FEEL).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provides: Option<Map<String, Json>>,
    /// Host inputs it reads, with types and defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Map<String, Json>>,
    /// The void it cuts in its host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub void: Option<Json>,
    /// Flip axes: `{axis: {label, values}}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flip: Option<Map<String, Json>>,
    /// Other keys (joins, insertion, snap points, level, placement…).
    #[serde(flatten)]
    pub extra: JsonMap,
}

// ---------------------------------------------------------------- opo::handles

/// One entry of `opo::handles` (spec §4.8).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Handle {
    /// Parameter it edits (exactly one of `parameter` and `slot`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter: Option<String>,
    /// Slot it edits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<String>,
    /// Handle kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<HandleKind>,
    /// Views it appears in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<Vec<String>>,
    /// Position (FEEL).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Expr>,
    /// Centre for angle and arc handles (FEEL).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub center: Option<Expr>,
    /// Axis of movement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub axis: Option<String>,
    /// Visibility condition (FEEL).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible: Option<Expr>,
    /// Drag both sides symmetrically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symmetric: Option<bool>,
}

/// Handle kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum HandleKind {
    Points,
    Linear,
    Angle,
    Arc,
}

// ---------------------------------------------------------------- opo::materials, opo::hatches

/// One entry of `opo::materials` (spec §4.6, decision 0011).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    /// Label.
    pub label: String,
    /// Category.
    pub category: MaterialCategory,
    /// Render colour `#rrggbb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<String>,
    /// Cut hatch.
    pub cut: HatchRef,
    /// Surface hatch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<HatchRef>,
    /// Density, kg/m³.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub density: Option<f64>,
    /// Thermal conductivity, W/(m·K).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thermal_conductivity: Option<f64>,
    /// Name written to `IfcMaterial`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ifc_name: Option<String>,
    /// Note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Material categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum MaterialCategory {
    Concrete,
    Masonry,
    Steel,
    Metal,
    Timber,
    Insulation,
    Glass,
    Finish,
    Membrane,
    Earth,
    Plastic,
    Other,
}

/// A reference to a hatch key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HatchRef {
    /// Hatch key in `opo::hatches`.
    pub hatch: String,
}

/// One entry of `opo::hatches`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hatch {
    /// Label.
    pub label: String,
    /// Solid fill, line families or none.
    pub kind: HatchKind,
    /// Line spacing in model or paper units (required for `lines`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_mode: Option<ScaleMode>,
    /// Line families (required for `lines`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lines: Option<Vec<LineFamily>>,
    /// Line or fill colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<String>,
    /// Background colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// Line weight, > 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_weight: Option<f64>,
    /// Note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Hatch kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum HatchKind {
    Solid,
    Lines,
    None,
}

/// Hatch scale modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[allow(missing_docs)]
pub enum ScaleMode {
    Model,
    Paper,
}

/// One line family of a hatch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineFamily {
    /// Angle in degrees.
    pub angle: f64,
    /// Origin `[x, y]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<[f64; 2]>,
    /// Offset `[along, across]` between lines.
    pub offset: [f64; 2],
    /// Dash pattern: + dash, − gap, 0 dot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dashes: Option<Vec<f64>>,
}

// ---------------------------------------------------------------- opo::ports

/// One entry of `opo::ports` (spec §4.9a).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Port {
    /// snake_case name.
    pub name: String,
    /// IFC `IfcDistributionSystemEnum` value.
    pub system_type: String,
    /// SOURCE, SINK, SOURCEANDSINK or NOTDEFINED.
    pub flow_direction: String,
    /// CABLE, CABLECARRIER, DUCT, PIPE, WIRELESS, USERDEFINED or NOTDEFINED.
    pub port_type: String,
    /// Nominal size (FEEL).
    pub nominal_size: Expr,
    /// Position (FEEL).
    pub position: Expr,
    /// Direction (FEEL).
    pub direction: Expr,
    /// Condition (FEEL): the port exists only when true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<Expr>,
}

// ---------------------------------------------------------------- opo::ifc

/// `opo::ifc`: IFC class, properties and quantities (spec §4.10, §7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ifc {
    /// IFC class.
    pub class: String,
    /// Predefined type (FEEL or literal).
    #[serde(
        default,
        rename = "predefinedType",
        skip_serializing_if = "Option::is_none"
    )]
    pub predefined_type: Option<Json>,
    /// Object type (FEEL or literal).
    #[serde(
        default,
        rename = "objectType",
        skip_serializing_if = "Option::is_none"
    )]
    pub object_type: Option<Json>,
    /// Property sets: one name, or a map of set name to property names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub property_sets: Option<Json>,
    /// Property set that parameters are written to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter_pset: Option<String>,
    /// IFC properties written from FEEL expressions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub properties: Option<JsonMap>,
    /// Base quantities.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quantities: Option<serde_json::Map<String, Json>>,
    /// Other keys (material layer set, profile, decomposition, body, note).
    #[serde(flatten)]
    pub extra: JsonMap,
}

/// One quantity set of `opo::ifc.quantities`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuantitySet {
    /// Quantity name to FEEL expression.
    pub values: Map<String, Json>,
    /// Quantities measured from the geometry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_geometry: Option<Vec<String>>,
}

impl Ifc {
    /// The quantity sets, typed.
    pub fn quantity_sets(&self) -> Result<Vec<(String, QuantitySet)>, serde_json::Error> {
        self.quantities
            .iter()
            .flatten()
            .map(|(k, v)| Ok((k.clone(), serde_json::from_value(v.clone())?)))
            .collect()
    }
}

// ---------------------------------------------------------------- variants and instances

/// `opo::values`: parameter values of a variant or instance.
pub type Values = JsonMap;

/// `opo::inputs`: drawn geometry of an instance, per slot (null clears the slot).
pub type Inputs = JsonMap;

#[cfg(test)]
mod provenance_tests {
    use super::*;

    fn read(v: serde_json::Value) -> Provenance {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn valid_entries() {
        let p = read(serde_json::json!({
            "values.nominal_width": {"source": "bs-4787", "status": "norm"},
            "parameters.handle_height.default": {"status": "typical", "note": "Common height."},
            "ruleset.values.min_clear_width": {"source": "iso-21542", "status": "to_verify", "note": "Check."}
        }));
        assert!(check_provenance(&p).is_empty());
    }

    #[test]
    fn invalid_entries() {
        let p = read(serde_json::json!({
            "values.a": {"status": "maker"},
            "values.b": {"source": "x", "status": "to_verify"},
            "values": {"status": "illustrative", "note": "n"},
            "Values.c": {"status": "illustrative", "note": "n"},
            "values.d": {"source": "", "status": "norm"}
        }));
        let errs = check_provenance(&p);
        assert_eq!(errs.len(), 5, "{errs:?}");
        assert!(
            serde_json::from_value::<Provenance>(
                serde_json::json!({"values.a": {"status": "guess"}})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<Provenance>(
                serde_json::json!({"values.a": {"status": "norm", "source": "s", "extra": 1}})
            )
            .is_err()
        );
    }
}
