use std::any::Any;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// Numbers within this distance are equal (spec §4.4).
pub const EPSILON: f64 = 1e-9;

/// A FEEL value.
///
/// Numbers are IEEE 754 doubles (spec §4.4), not FEEL's decimals. Contexts keep their keys
/// sorted, so output is deterministic.
#[derive(Debug, Clone)]
pub enum Value {
    /// FEEL `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number.
    Number(f64),
    /// A string.
    String(String),
    /// A list.
    List(Vec<Value>),
    /// A context (`{a: 1}`).
    Context(BTreeMap<String, Value>),
    /// A value owned by the application (for example geometry from a recipe operation).
    /// Formulas can store it, pass it to functions and put it in lists; arithmetic, comparison
    /// and paths on it give `null`.
    Foreign(Foreign),
}

/// An application value carried through formulas (see [`Value::Foreign`]).
#[derive(Clone)]
pub struct Foreign(pub Arc<dyn Any + Send + Sync>);

impl Foreign {
    /// Wrap an application value.
    pub fn new<T: Any + Send + Sync>(value: T) -> Self {
        Foreign(Arc::new(value))
    }

    /// The value, if it has type `T`.
    pub fn downcast<T: Any>(&self) -> Option<&T> {
        self.0.downcast_ref::<T>()
    }
}

impl fmt::Debug for Foreign {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Foreign(..)")
    }
}

impl Value {
    /// FEEL equality: numbers within [`EPSILON`] are equal; lists and contexts compare item by
    /// item; values of different types are not equal; `null = null` is true.
    pub fn feel_eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Null, Value::Null) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Number(a), Value::Number(b)) => (a - b).abs() < EPSILON,
            (Value::String(a), Value::String(b)) => a == b,
            (Value::List(a), Value::List(b)) => {
                a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.feel_eq(y))
            }
            (Value::Foreign(a), Value::Foreign(b)) => Arc::ptr_eq(&a.0, &b.0),
            (Value::Context(a), Value::Context(b)) => {
                a.len() == b.len()
                    && a.iter()
                        .zip(b)
                        .all(|((ka, va), (kb, vb))| ka == kb && va.feel_eq(vb))
            }
            _ => false,
        }
    }

    /// The number, if this is a number.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// The list items, if this is a list.
    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(l) => Some(l),
            _ => None,
        }
    }

    /// Convert from JSON. JSON numbers become doubles.
    pub fn from_json(json: &serde_json::Value) -> Value {
        match json {
            serde_json::Value::Null => Value::Null,
            serde_json::Value::Bool(b) => Value::Bool(*b),
            serde_json::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(f64::NAN)),
            serde_json::Value::String(s) => Value::String(s.clone()),
            serde_json::Value::Array(a) => Value::List(a.iter().map(Value::from_json).collect()),
            serde_json::Value::Object(o) => Value::Context(
                o.iter()
                    .map(|(k, v)| (k.clone(), Value::from_json(v)))
                    .collect(),
            ),
        }
    }

    /// Convert to JSON. Non-finite numbers and foreign values become `null`.
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Value::Null => serde_json::Value::Null,
            Value::Bool(b) => serde_json::Value::Bool(*b),
            Value::Number(n) => serde_json::Number::from_f64(*n)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Value::String(s) => serde_json::Value::String(s.clone()),
            Value::List(l) => serde_json::Value::Array(l.iter().map(Value::to_json).collect()),
            Value::Context(c) => {
                serde_json::Value::Object(c.iter().map(|(k, v)| (k.clone(), v.to_json())).collect())
            }
            Value::Foreign(_) => serde_json::Value::Null,
        }
    }
}

/// Structural equality with the FEEL number tolerance (see [`Value::feel_eq`]).
impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        self.feel_eq(other)
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Number(n)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::String(s.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_within_epsilon_are_equal() {
        assert!(Value::Number(0.3).feel_eq(&Value::Number(0.1 + 0.2)));
        assert!(!Value::Number(0.1).feel_eq(&Value::Number(0.1000001)));
    }

    #[test]
    fn different_types_are_not_equal() {
        assert!(!Value::Number(1.0).feel_eq(&Value::String("1".into())));
        assert!(!Value::Null.feel_eq(&Value::Bool(false)));
    }

    #[test]
    fn foreign_values_compare_by_identity() {
        let a = Value::Foreign(Foreign::new(5_u8));
        let b = a.clone();
        assert!(a.feel_eq(&b));
        assert!(!a.feel_eq(&Value::Foreign(Foreign::new(5_u8))));
        match &a {
            Value::Foreign(f) => assert_eq!(f.downcast::<u8>(), Some(&5)),
            _ => unreachable!(),
        }
        assert_eq!(a.to_json(), serde_json::Value::Null);
    }

    #[test]
    fn json_round_trip() {
        let j: serde_json::Value = serde_json::json!({"a": [1, 2.5, null, true, "x"]});
        assert_eq!(
            Value::from_json(&j).to_json(),
            serde_json::json!({"a": [1.0, 2.5, null, true, "x"]})
        );
    }
}
