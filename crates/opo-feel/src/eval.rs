use std::collections::BTreeMap;

use crate::Error;
use crate::builtins;
use crate::parser::{Expr, Source};
use crate::value::{EPSILON, Value};

/// Names and functions available to an expression.
pub trait Env {
    /// The value of a name, if defined.
    fn get(&self, name: &str) -> Option<&Value>;

    /// Call an application function (one that is not a built-in), such as a recipe
    /// operation. Return `None` if the function is not known.
    fn call(&self, name: &str, args: Vec<Value>) -> Option<Result<Value, Error>> {
        let _ = (name, args);
        None
    }
}

/// A simple scope: a map of names to values, with no application functions.
#[derive(Debug, Clone, Default)]
pub struct Scope {
    vars: BTreeMap<String, Value>,
}

impl Scope {
    /// An empty scope.
    pub fn new() -> Self {
        Self::default()
    }

    /// Define or replace a name.
    pub fn set(&mut self, name: impl Into<String>, value: Value) {
        self.vars.insert(name.into(), value);
    }

    /// A scope holding the entries of a context (other values give an empty scope).
    pub fn from_context(value: &Value) -> Self {
        match value {
            Value::Context(c) => Scope { vars: c.clone() },
            _ => Scope::new(),
        }
    }
}

impl Env for Scope {
    fn get(&self, name: &str) -> Option<&Value> {
        self.vars.get(name)
    }
}

/// One `for` variable on top of an outer scope.
struct Bound<'a> {
    outer: &'a dyn Env,
    name: &'a str,
    value: Value,
}

impl Env for Bound<'_> {
    fn get(&self, name: &str) -> Option<&Value> {
        if name == self.name {
            Some(&self.value)
        } else {
            self.outer.get(name)
        }
    }
    fn call(&self, name: &str, args: Vec<Value>) -> Option<Result<Value, Error>> {
        self.outer.call(name, args)
    }
}

/// Evaluate a parsed expression.
///
/// Semantics (spec §4.4): `null` propagates through arithmetic and comparisons; `and` and
/// `or` use three-valued logic; numbers within [`EPSILON`] compare equal, for ordering as well
/// as for `=`; type mismatches give `null`.
pub fn eval(expr: &Expr, env: &dyn Env) -> Result<Value, Error> {
    Ok(match expr {
        Expr::Literal(v) => v.clone(),
        Expr::Name(n) => match env.get(n) {
            Some(v) => v.clone(),
            None => return Err(Error::UnknownName(n.clone())),
        },
        Expr::List(items) => Value::List(
            items
                .iter()
                .map(|x| eval(x, env))
                .collect::<Result<_, _>>()?,
        ),
        Expr::Context(entries) => Value::Context(
            entries
                .iter()
                .map(|(k, x)| Ok((k.clone(), eval(x, env)?)))
                .collect::<Result<_, Error>>()?,
        ),
        Expr::Path(base, key) => path(eval(base, env)?, key),
        Expr::Call(name, args) => {
            let args: Vec<Value> = args
                .iter()
                .map(|x| eval(x, env))
                .collect::<Result<_, _>>()?;
            if builtins::is_builtin(name) {
                builtins::call(name, &args)?
            } else {
                match env.call(name, args) {
                    Some(r) => r?,
                    None => return Err(Error::UnknownName(name.clone())),
                }
            }
        }
        Expr::Index(list, index) => {
            let (list, index) = (eval(list, env)?, eval(index, env)?);
            match (list, index.as_number()) {
                (Value::List(l), Some(i)) => {
                    let i = i.trunc() as i64;
                    let n = l.len() as i64;
                    if i == 0 || i.abs() > n {
                        Value::Null
                    } else {
                        let k = if i > 0 { i - 1 } else { n + i };
                        l[k as usize].clone()
                    }
                }
                _ => Value::Null,
            }
        }
        Expr::Negate(x) => match eval(x, env)? {
            Value::Number(n) => Value::Number(-n),
            _ => Value::Null,
        },
        Expr::Arithmetic(op, a, b) => arithmetic(*op, eval(a, env)?, eval(b, env)?),
        Expr::Compare(op, a, b) => compare(op, &eval(a, env)?, &eval(b, env)?),
        Expr::And(a, b) => {
            let a = eval(a, env)?;
            if matches!(a, Value::Bool(false)) {
                return Ok(Value::Bool(false));
            }
            match (a, eval(b, env)?) {
                (_, Value::Bool(false)) => Value::Bool(false),
                (Value::Bool(true), Value::Bool(true)) => Value::Bool(true),
                _ => Value::Null,
            }
        }
        Expr::Or(a, b) => {
            let a = eval(a, env)?;
            if matches!(a, Value::Bool(true)) {
                return Ok(Value::Bool(true));
            }
            match (a, eval(b, env)?) {
                (_, Value::Bool(true)) => Value::Bool(true),
                (Value::Bool(false), Value::Bool(false)) => Value::Bool(false),
                _ => Value::Null,
            }
        }
        Expr::If(c, a, b) => {
            // A condition that is not true (false, null or another value) takes the else branch.
            if matches!(eval(c, env)?, Value::Bool(true)) {
                eval(a, env)?
            } else {
                eval(b, env)?
            }
        }
        Expr::For(binds, body) => {
            let mut out = Vec::new();
            for_loop(binds, body, env, &mut out)?;
            Value::List(out)
        }
    })
}

fn for_loop(
    binds: &[(String, Source)],
    body: &Expr,
    env: &dyn Env,
    out: &mut Vec<Value>,
) -> Result<(), Error> {
    let Some(((name, src), rest)) = binds.split_first() else {
        out.push(eval(body, env)?);
        return Ok(());
    };
    let items = match src {
        Source::Range(a, b) => {
            let (a, b) = (range_end(eval(a, env)?)?, range_end(eval(b, env)?)?);
            if a <= b {
                (a..=b).map(|i| Value::Number(i as f64)).collect()
            } else {
                (b..=a).rev().map(|i| Value::Number(i as f64)).collect()
            }
        }
        Source::List(x) => match eval(x, env)? {
            Value::List(l) => l,
            other => {
                return Err(Error::Eval(format!(
                    "for: the source is not a list ({other:?})"
                )));
            }
        },
    };
    for value in items {
        let scope = Bound {
            outer: env,
            name,
            value,
        };
        for_loop(rest, body, &scope, out)?;
    }
    Ok(())
}

/// Range ends must be whole numbers (within [`EPSILON`]).
fn range_end(v: Value) -> Result<i64, Error> {
    match v {
        Value::Number(n) if (n - n.round()).abs() < EPSILON && n.abs() < 1e15 => {
            Ok(n.round() as i64)
        }
        other => Err(Error::Eval(format!(
            "a range end must be a whole number, not {other:?}"
        ))),
    }
}

fn path(base: Value, key: &str) -> Value {
    match base {
        Value::List(items) => Value::List(items.into_iter().map(|b| path(b, key)).collect()),
        Value::Context(mut c) => c.remove(key).unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

fn arithmetic(op: char, a: Value, b: Value) -> Value {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => {
            let r = match op {
                '+' => a + b,
                '-' => a - b,
                '*' => a * b,
                '/' if b == 0.0 => return Value::Null,
                '/' => a / b,
                _ => a.powf(b),
            };
            if r.is_nan() {
                Value::Null
            } else {
                Value::Number(r)
            }
        }
        (Value::String(a), Value::String(b)) if op == '+' => Value::String(a + &b),
        _ => Value::Null,
    }
}

fn compare(op: &str, a: &Value, b: &Value) -> Value {
    match op {
        "=" => Value::Bool(a.feel_eq(b)),
        "!=" => Value::Bool(!a.feel_eq(b)),
        _ => {
            let ord = match (a, b) {
                (Value::Number(x), Value::Number(y)) => {
                    if (x - y).abs() < EPSILON {
                        std::cmp::Ordering::Equal
                    } else if x < y {
                        std::cmp::Ordering::Less
                    } else {
                        std::cmp::Ordering::Greater
                    }
                }
                (Value::String(x), Value::String(y)) => x.cmp(y),
                _ => return Value::Null,
            };
            use std::cmp::Ordering::*;
            Value::Bool(match op {
                "<" => ord == Less,
                "<=" => ord != Greater,
                ">" => ord == Greater,
                _ => ord != Less,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{eval_str, parse};

    fn ev(src: &str) -> Value {
        eval_str(src, &Scope::new()).unwrap()
    }

    #[test]
    fn unknown_name_is_an_error() {
        assert_eq!(
            eval_str("x + 1", &Scope::new()),
            Err(Error::UnknownName("x".into()))
        );
        assert_eq!(
            eval_str("f(1)", &Scope::new()),
            Err(Error::UnknownName("f".into()))
        );
    }

    #[test]
    fn ordering_uses_the_equality_tolerance() {
        assert_eq!(ev("0.3 < 0.3 + 1e-12"), Value::Bool(false));
        assert_eq!(ev("0.3 <= 0.3 - 1e-12"), Value::Bool(true));
        assert_eq!(ev("1 < 2"), Value::Bool(true));
    }

    #[test]
    fn type_mismatches_give_null() {
        assert_eq!(ev("1 + \"a\""), Value::Null);
        assert_eq!(ev("\"a\" < 1"), Value::Null);
        assert_eq!(ev("-\"a\""), Value::Null);
        assert_eq!(ev("(1).a"), Value::Null);
        assert_eq!(ev("(-8) ** 0.5"), Value::Null);
    }

    #[test]
    fn strings_order_lexically() {
        assert_eq!(ev("\"a\" < \"b\""), Value::Bool(true));
    }

    #[test]
    fn non_whole_range_end_is_an_error() {
        assert!(eval_str("for i in 1..2.5 return i", &Scope::new()).is_err());
        assert!(eval_str("for i in 1 return i", &Scope::new()).is_err());
    }

    #[test]
    fn for_variable_shadows_scope() {
        let mut s = Scope::new();
        s.set("i", Value::Number(10.0));
        assert_eq!(
            eval_str("for i in 1..2 return i", &s).unwrap(),
            ev("[1, 2]")
        );
    }

    struct App;
    impl Env for App {
        fn get(&self, _: &str) -> Option<&Value> {
            None
        }
        fn call(&self, name: &str, args: Vec<Value>) -> Option<Result<Value, Error>> {
            (name == "twice").then(|| Ok(Value::Number(args[0].as_number().unwrap_or(0.0) * 2.0)))
        }
    }

    #[test]
    fn application_functions_are_called() {
        assert_eq!(
            eval(&parse("twice(4) + 1").unwrap(), &App).unwrap(),
            Value::Number(9.0)
        );
    }

    #[test]
    fn builtins_take_precedence_over_application_functions() {
        struct Shadow;
        impl Env for Shadow {
            fn get(&self, _: &str) -> Option<&Value> {
                None
            }
            fn call(&self, _: &str, _: Vec<Value>) -> Option<Result<Value, Error>> {
                Some(Ok(Value::Null))
            }
        }
        assert_eq!(
            eval(&parse("abs(-1)").unwrap(), &Shadow).unwrap(),
            Value::Number(1.0)
        );
    }

    #[test]
    fn scope_from_context() {
        let s = Scope::from_context(&ev("{w: 2}"));
        assert_eq!(eval_str("w * 2", &s).unwrap(), Value::Number(4.0));
    }
}
