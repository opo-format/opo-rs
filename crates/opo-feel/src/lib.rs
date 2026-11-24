//! Evaluator for the FEEL subset used by OPO formulas (OPO spec §4.4).
//!
//! OPO formulas are FEEL expressions (OMG DMN) restricted to a listed subset, with numbers as
//! IEEE 754 doubles and a few OPO additions (trigonometry in degrees, unit helpers).
//!
//! ```
//! use opo_feel::{eval_str, Scope, Value};
//!
//! let mut scope = Scope::new();
//! scope.set("width", Value::Number(0.9));
//! let leaves = eval_str("if width >= mm(1200) then 2 else 1", &scope).unwrap();
//! assert_eq!(leaves, Value::Number(1.0));
//! ```
//!
//! Applications add their own functions (for example the recipe operations of spec §4.5) by
//! implementing [`Env::call`].

mod builtins;
mod error;
mod eval;
mod lexer;
mod parser;
mod value;

pub use builtins::is_builtin;
pub use error::Error;
pub use eval::{Env, Scope, eval};
pub use parser::{Expr, Source, free_names, parse};
pub use value::{EPSILON, Value};

/// Parse and evaluate an expression in one step.
///
/// Prefer [`parse`] then [`eval`] when the same expression is evaluated many times.
pub fn eval_str(src: &str, env: &dyn Env) -> Result<Value, Error> {
    eval(&parse(src)?, env)
}
