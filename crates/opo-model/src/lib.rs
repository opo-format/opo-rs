//! Types for the `opo::` attributes of an OPO object, as defined by the JSON Schemas in
//! `schema/0.1/` of the standard repository (spec §4, §5.1).
//!
//! Each attribute deserialises with [`serde`]. Fields the schema closes
//! (`additionalProperties: false`) reject unknown keys; open objects keep unknown keys in an
//! `extra` map so nothing is lost on a round trip. FEEL expressions are kept as text; parsing
//! and evaluating them is the job of `opo-feel` and `opo-recipe`.
//!
//! [`TypeDefinition::from_attributes`] reads every `opo::` attribute of a type node at once,
//! and [`TypeDefinition::validate`] checks the rules that serde cannot express (name patterns,
//! recipe arguments per operation, conditional requirements).

mod attributes;
mod definition;
mod validate;

pub use attributes::*;
pub use definition::{ATTRIBUTE_NAMES, ModelError, TypeDefinition};
pub use validate::Issue;

/// The schema version these types implement.
pub const SCHEMA_VERSION: &str = "0.1";
