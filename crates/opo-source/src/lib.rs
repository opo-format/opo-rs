//! YAML sources of OPO objects (proposal 0002).
//!
//! - [`yaml`]: the safe YAML subset sources are written in.
//! - [`Library`]: the registries (`references.yaml`, `styles/materials.yaml`,
//!   `styles/hatches.yaml`).
//! - [`compile`]: an object source (`object.yaml`, `layers/*.yaml`) → IFCX documents. Values
//!   with provenance become plain values plus `opo::provenance`; plain IFC values
//!   (`bsi::ifc::prop::*`) are computed from `opo::ifc.properties`. Baking is left to
//!   `opo_build::bake`.
//! - [`export`]: the reverse, for converting existing objects into sources.
//! - [`generate`]: class templates from archetypes, the per-class table and IFC data.

pub mod yaml;

mod catalogue;
mod compile;
mod export;
mod ifc_uml;
mod library;
mod template;

pub use catalogue::{Catalogue, Entry, catalogue};
pub use compile::{Compiled, compile, compile_dir, read_source_dir};
pub use export::{Exported, export, export_dir};
pub use ifc_uml::extract_ifc_uml;
pub use library::Library;
pub use template::{Generated, GeneratorOptions, IfcData, TemplateSources, generate};

use std::fmt;
use std::path::PathBuf;

/// An error reading, compiling or exporting sources.
#[derive(Debug)]
pub enum Error {
    /// A file could not be read or written.
    Io(PathBuf, std::io::Error),
    /// A YAML file is not valid safe YAML.
    Yaml(yaml::YamlError),
    /// A source does not have the shape proposal 0002 defines.
    Source(String),
    /// An IFCX document could not be read.
    Ifcx(String),
    /// Running the object (for plain IFC values) failed.
    Recipe(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(p, e) => write!(f, "{}: {e}", p.display()),
            Error::Yaml(e) => write!(f, "{e}"),
            Error::Source(m) | Error::Ifcx(m) | Error::Recipe(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

impl From<yaml::YamlError> for Error {
    fn from(e: yaml::YamlError) -> Self {
        Error::Yaml(e)
    }
}

/// Read and parse a YAML file; a missing file is `None`.
pub(crate) fn read_yaml(path: &std::path::Path) -> Result<Option<serde_json::Value>, Error> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(yaml::parse(&text, &path.display().to_string())?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Io(path.into(), e)),
    }
}
