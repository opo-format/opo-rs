//! Geometry kernel that runs OPO recipes (spec §4.5).
//!
//! [`run`] builds an object's environment (parameter defaults, values, drawn inputs, host and
//! view context, derived values) and runs its recipe steps in order. Steps produce geometry:
//! 2D regions and curves in a plane, 3D curves, closed triangle meshes ([`Solid`]) and text.
//! Operations can also be called inline inside a FEEL argument (`extrude(circle([0, 0], r), 0,
//! h)`); geometry travels through formulas as an opaque value.
//!
//! 2D unions and differences are exact polygon operations; 3D unions are lists of solids (no
//! solid boolean operations). Curved shapes are polygons: a circle with `segments` = 48 is a
//! 48-gon.

pub mod derive;
mod geometry;
mod grid;
mod mesh;
mod ops;
mod run;

use std::fmt;

pub use geo::MultiPolygon;
pub use geometry::{Geometry, Plane, Shape, Solid, flatten};
pub use run::{Evaluation, Input, MAX_DEPTH, NoNesting, Resolver, resolve_material, run};

/// An error running a recipe.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// A formula failed.
    Feel {
        /// Where: derived value, step argument…
        at: String,
        /// The formula error.
        source: opo_feel::Error,
    },
    /// An operation got arguments it cannot use.
    Op(String),
    /// A nested object could not be found or used.
    Resolve(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Feel { at, source } => write!(f, "{at}: {source}"),
            Error::Op(m) | Error::Resolve(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}
