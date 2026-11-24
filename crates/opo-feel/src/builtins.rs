//! Built-in functions (spec §4.4): the FEEL subset plus OPO's unit helpers and trigonometry
//! in degrees.

use crate::Error;
use crate::value::{EPSILON, Value};

const NAMES: [&str; 28] = [
    "mm",
    "cm",
    "m",
    "inch",
    "ft",
    "deg",
    "min",
    "max",
    "abs",
    "sqrt",
    "ceiling",
    "floor",
    "round",
    "modulo",
    "sin",
    "cos",
    "tan",
    "atan2",
    "sum",
    "count",
    "sublist",
    "flatten",
    "append",
    "concatenate",
    "reverse",
    "index_of",
    "list_contains",
    "not",
];

/// True if `name` is a built-in function.
pub fn is_builtin(name: &str) -> bool {
    NAMES.contains(&name)
}

fn arity(name: &str, args: &[Value], ok: impl Fn(usize) -> bool) -> Result<(), Error> {
    if ok(args.len()) {
        Ok(())
    } else {
        Err(Error::Arity {
            function: name.into(),
            given: args.len(),
        })
    }
}

fn num(v: &Value) -> Option<f64> {
    v.as_number()
}

/// Apply a number function to the single argument; non-numbers give null.
fn unary(name: &str, args: &[Value], f: impl Fn(f64) -> f64) -> Result<Value, Error> {
    arity(name, args, |n| n == 1)?;
    Ok(match num(&args[0]) {
        Some(x) => {
            let r = f(x);
            if r.is_finite() {
                Value::Number(r)
            } else {
                Value::Null
            }
        }
        None => Value::Null,
    })
}

/// The numbers of `f(list)` or `f(a, b, …)`; `None` if any item is not a number.
fn numbers(args: &[Value]) -> Option<Vec<f64>> {
    let items: &[Value] = match args {
        [Value::List(l)] => l,
        _ => args,
    };
    items.iter().map(num).collect()
}

/// Round half to even at `scale` decimals; values within [`EPSILON`] of a half are halves.
fn round_half_even(x: f64, scale: i32) -> f64 {
    let f = 10f64.powi(scale);
    let y = x * f;
    let lo = y.floor();
    let frac = y - lo;
    let r = if (frac - 0.5).abs() < EPSILON {
        if lo % 2.0 == 0.0 { lo } else { lo + 1.0 }
    } else if frac > 0.5 {
        lo + 1.0
    } else {
        lo
    };
    r / f
}

fn flatten_into(items: &[Value], out: &mut Vec<Value>) {
    for v in items {
        match v {
            Value::List(l) => flatten_into(l, out),
            other => out.push(other.clone()),
        }
    }
}

/// Call a built-in function. The caller has checked [`is_builtin`].
pub(crate) fn call(name: &str, args: &[Value]) -> Result<Value, Error> {
    Ok(match name {
        "mm" => return unary(name, args, |v| v / 1000.0),
        "cm" => return unary(name, args, |v| v / 100.0),
        "m" | "deg" => return unary(name, args, |v| v),
        "inch" => return unary(name, args, |v| v * 0.0254),
        "ft" => return unary(name, args, |v| v * 0.3048),
        "abs" => return unary(name, args, f64::abs),
        "sqrt" => return unary(name, args, f64::sqrt),
        // Values within EPSILON of a whole number are that number (ceiling(2.8 / 0.175) = 16).
        "ceiling" => return unary(name, args, |v| (v - EPSILON).ceil()),
        "floor" => return unary(name, args, |v| (v + EPSILON).floor()),
        "sin" => return unary(name, args, |a| a.to_radians().sin()),
        "cos" => return unary(name, args, |a| a.to_radians().cos()),
        "tan" => return unary(name, args, |a| a.to_radians().tan()),
        "atan2" => {
            arity(name, args, |n| n == 2)?;
            match (num(&args[0]), num(&args[1])) {
                (Some(y), Some(x)) => Value::Number(y.atan2(x).to_degrees()),
                _ => Value::Null,
            }
        }
        "round" => {
            arity(name, args, |n| n == 1 || n == 2)?;
            let scale = if args.len() == 2 {
                num(&args[1])
            } else {
                Some(0.0)
            };
            match (num(&args[0]), scale) {
                (Some(x), Some(s)) => Value::Number(round_half_even(x, s.trunc() as i32)),
                _ => Value::Null,
            }
        }
        "modulo" => {
            arity(name, args, |n| n == 2)?;
            match (num(&args[0]), num(&args[1])) {
                (Some(_), Some(0.0)) => Value::Null,
                // The result has the sign of the divisor.
                (Some(a), Some(b)) => Value::Number(a - b * (a / b).floor()),
                _ => Value::Null,
            }
        }
        "min" | "max" => {
            arity(name, args, |n| n >= 1)?;
            match numbers(args) {
                Some(xs) if !xs.is_empty() => Value::Number(if name == "min" {
                    xs.into_iter().fold(f64::INFINITY, f64::min)
                } else {
                    xs.into_iter().fold(f64::NEG_INFINITY, f64::max)
                }),
                _ => Value::Null,
            }
        }
        "sum" => {
            arity(name, args, |n| n >= 1)?;
            // sum([]) is null, as in FEEL.
            match numbers(args) {
                Some(xs) if !xs.is_empty() => Value::Number(xs.iter().sum()),
                _ => Value::Null,
            }
        }
        "count" => {
            arity(name, args, |n| n == 1)?;
            match &args[0] {
                Value::List(l) => Value::Number(l.len() as f64),
                _ => Value::Null,
            }
        }
        "sublist" => {
            arity(name, args, |n| n == 2 || n == 3)?;
            let (Value::List(l), Some(start)) = (&args[0], num(&args[1])) else {
                return Ok(Value::Null);
            };
            let n = l.len() as i64;
            let start = start.trunc() as i64;
            let s = if start > 0 { start - 1 } else { n + start }.clamp(0, n) as usize;
            let e = match args.get(2) {
                None => l.len(),
                Some(len) => match num(len) {
                    Some(len) => (s + len.max(0.0).trunc() as usize).min(l.len()),
                    None => return Ok(Value::Null),
                },
            };
            Value::List(l[s..e].to_vec())
        }
        "flatten" => {
            arity(name, args, |n| n == 1)?;
            match &args[0] {
                Value::List(l) => {
                    let mut out = Vec::new();
                    flatten_into(l, &mut out);
                    Value::List(out)
                }
                _ => Value::Null,
            }
        }
        "append" => {
            arity(name, args, |n| n >= 1)?;
            match &args[0] {
                Value::List(l) => Value::List(l.iter().chain(&args[1..]).cloned().collect()),
                _ => Value::Null,
            }
        }
        "concatenate" => {
            let mut out = Vec::new();
            for a in args {
                match a {
                    Value::List(l) => out.extend(l.iter().cloned()),
                    _ => return Ok(Value::Null),
                }
            }
            Value::List(out)
        }
        "reverse" => {
            arity(name, args, |n| n == 1)?;
            match &args[0] {
                Value::List(l) => Value::List(l.iter().rev().cloned().collect()),
                _ => Value::Null,
            }
        }
        "index_of" => {
            arity(name, args, |n| n == 2)?;
            match &args[0] {
                Value::List(l) => Value::List(
                    l.iter()
                        .enumerate()
                        .filter(|(_, v)| v.feel_eq(&args[1]))
                        .map(|(i, _)| Value::Number((i + 1) as f64))
                        .collect(),
                ),
                _ => Value::Null,
            }
        }
        "list_contains" => {
            arity(name, args, |n| n == 2)?;
            match &args[0] {
                Value::List(l) => Value::Bool(l.iter().any(|v| v.feel_eq(&args[1]))),
                _ => Value::Null,
            }
        }
        "not" => {
            arity(name, args, |n| n == 1)?;
            match &args[0] {
                Value::Bool(b) => Value::Bool(!b),
                _ => Value::Null,
            }
        }
        _ => return Err(Error::UnknownName(name.into())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Scope, eval_str};

    fn ev(src: &str) -> Value {
        eval_str(src, &Scope::new()).unwrap()
    }

    #[test]
    fn wrong_argument_count_is_an_error() {
        assert!(matches!(
            eval_str("abs(1, 2)", &Scope::new()),
            Err(Error::Arity { .. })
        ));
        assert!(matches!(
            eval_str("count()", &Scope::new()),
            Err(Error::Arity { .. })
        ));
    }

    #[test]
    fn invalid_arguments_give_null() {
        assert_eq!(ev("sqrt(-1)"), Value::Null);
        assert_eq!(ev("min([])"), Value::Null);
        assert_eq!(ev("max([1, null])"), Value::Null);
        assert_eq!(ev("sum([1, null])"), Value::Null);
        assert_eq!(ev("modulo(1, 0)"), Value::Null);
        assert_eq!(ev("count(1)"), Value::Null);
        assert_eq!(ev("mm(\"a\")"), Value::Null);
        assert_eq!(ev("concatenate([1], 2)"), Value::Null);
    }

    #[test]
    fn round_ties_go_to_even_with_tolerance() {
        assert_eq!(round_half_even(0.125, 2), 0.12);
        assert_eq!(round_half_even(-2.5, 0), -2.0);
        assert_eq!(round_half_even(1250.0, -2), 1200.0);
        // 2.675 is 2.67499999… in binary; within the tolerance it is a half, so it goes to even.
        assert_eq!(round_half_even(2.675, 2), 2.68);
    }

    #[test]
    fn floor_and_ceiling_snap_near_whole_numbers() {
        assert_eq!(ev("floor(2.9999999999)"), Value::Number(3.0));
        assert_eq!(ev("ceiling(3.0000000001)"), Value::Number(3.0));
    }

    #[test]
    fn sublist_edges() {
        assert_eq!(ev("sublist([1, 2, 3], 5)"), ev("[]"));
        assert_eq!(ev("sublist([1, 2, 3], -9, 1)"), ev("[1]"));
        assert_eq!(ev("sublist([1, 2, 3], 2, 9)"), ev("[2, 3]"));
    }

    #[test]
    fn index_of_and_contains_use_number_tolerance() {
        assert_eq!(ev("index_of([0.3, 1], 0.1 + 0.2)"), ev("[1]"));
        assert_eq!(ev("list_contains([0.3], 0.1 + 0.2)"), Value::Bool(true));
    }

    #[test]
    fn builtin_list_is_complete() {
        for n in ["mm", "not", "list_contains", "atan2"] {
            assert!(is_builtin(n));
        }
        assert!(!is_builtin("length"));
        assert!(!is_builtin("extrude"));
    }
}
