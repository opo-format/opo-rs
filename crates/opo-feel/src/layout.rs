//! `layout`: divide a length by architectural rules (proposal 0001 §1, spec 0.2).
//!
//! `layout(length, pattern, [distribution, origin, infill, end_with, within])`, or
//! `layout(length, {pattern, distribution, …})` with the optional arguments in a context.
//! The result is `{bays: [{index, start, end, size, cls}], lines: [{index, at, cls}]}`; an
//! impossible layout (no `best_division` gives a positive scale, bad arguments) is `null`.

use std::collections::BTreeMap;

use crate::Error;
use crate::value::{EPSILON, Value};

/// One pattern entry.
#[derive(Debug, Clone)]
struct Entry {
    size: f64,
    flexible: bool,
    cls: String,
}

/// The arguments of one layout.
#[derive(Debug, Clone)]
struct Args {
    pattern: Vec<Value>,
    distribution: String,
    origin: String,
    infill: String,
    end_with: Option<usize>,
    within: Vec<f64>,
}

/// One bay before numbering.
struct Bay {
    start: f64,
    end: f64,
    cls: String,
}

const NAMES: [&str; 6] = [
    "pattern",
    "distribution",
    "origin",
    "infill",
    "end_with",
    "within",
];

/// Evaluate `layout(…)`.
pub(crate) fn call(args: &[Value]) -> Result<Value, Error> {
    if args.len() < 2 || args.len() > 7 {
        return Err(Error::Arity {
            function: "layout".into(),
            given: args.len(),
        });
    }
    let Some(length) = args[0].as_number() else {
        return Ok(Value::Null);
    };
    let named: BTreeMap<String, Value> = match (&args[1], args.len()) {
        (Value::Context(c), 2) => c.clone(),
        _ => NAMES
            .iter()
            .zip(&args[1..])
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
    };
    Ok(parse(&named)
        .and_then(|a| layout(length, &a))
        .unwrap_or(Value::Null))
}

/// Read the arguments; `None` if one has the wrong type.
fn parse(named: &BTreeMap<String, Value>) -> Option<Args> {
    let text = |k: &str, default: &str| match named.get(k) {
        None | Some(Value::Null) => Some(default.to_string()),
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    };
    let pattern = match named.get("pattern")? {
        Value::List(l) if !l.is_empty() => l.clone(),
        _ => return None,
    };
    let end_with = match named.get("end_with") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let n = v.as_number()?;
            if n < 1.0 - EPSILON || (n - n.round()).abs() > EPSILON {
                return None;
            }
            Some(n.round() as usize)
        }
    };
    let within = match named.get("within") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::List(l)) => l.iter().map(Value::as_number).collect::<Option<_>>()?,
        _ => return None,
    };
    Some(Args {
        pattern,
        distribution: text("distribution", "fixed")?,
        origin: text("origin", "start")?,
        infill: text("infill", "partial")?,
        end_with,
        within,
    })
}

fn entries(pattern: &[Value]) -> Option<Vec<Entry>> {
    pattern
        .iter()
        .map(|v| {
            let Value::Context(c) = v else { return None };
            let size = c.get("size")?.as_number().filter(|s| *s > 0.0)?;
            let flexible = match c.get("flexible") {
                None | Some(Value::Null) => false,
                Some(Value::Bool(b)) => *b,
                _ => return None,
            };
            let cls = match c.get("cls") {
                None | Some(Value::Null) => "main".to_string(),
                Some(Value::String(s)) => s.clone(),
                _ => return None,
            };
            Some(Entry {
                size,
                flexible,
                cls,
            })
        })
        .collect()
}

/// Lay out `length`, interval by interval.
fn layout(length: f64, a: &Args) -> Option<Value> {
    if length <= EPSILON {
        return None;
    }
    let mut breaks: Vec<f64> = a
        .within
        .iter()
        .copied()
        .filter(|w| *w > EPSILON && *w < length - EPSILON)
        .collect();
    breaks.sort_by(f64::total_cmp);
    breaks.dedup_by(|x, y| (*x - *y).abs() < EPSILON);
    let mut edges = vec![0.0];
    edges.extend(&breaks);
    edges.push(length);
    let mut bays = Vec::new();
    for w in edges.windows(2) {
        for b in interval(w[1] - w[0], a)? {
            bays.push(Bay {
                start: b.start + w[0],
                end: b.end + w[0],
                cls: b.cls,
            });
        }
    }
    let ctx = |pairs: Vec<(&str, Value)>| {
        Value::Context(pairs.into_iter().map(|(k, v)| (k.into(), v)).collect())
    };
    let mut lines = vec![(0.0, "border")];
    for b in &bays {
        let at = b.end;
        let cls = if (at - length).abs() < EPSILON {
            "border"
        } else if breaks.iter().any(|x| (x - at).abs() < EPSILON) {
            "break"
        } else {
            "division"
        };
        lines.push((at, cls));
    }
    let bays = bays
        .iter()
        .enumerate()
        .map(|(i, b)| {
            ctx(vec![
                ("index", Value::Number(i as f64)),
                ("start", Value::Number(b.start)),
                ("end", Value::Number(b.end)),
                ("size", Value::Number(b.end - b.start)),
                ("cls", Value::String(b.cls.clone())),
            ])
        })
        .collect();
    let lines = lines
        .iter()
        .enumerate()
        .map(|(i, (at, cls))| {
            ctx(vec![
                ("index", Value::Number(i as f64)),
                ("at", Value::Number(*at)),
                ("cls", Value::String(cls.to_string())),
            ])
        })
        .collect();
    Some(ctx(vec![
        ("bays", Value::List(bays)),
        ("lines", Value::List(lines)),
    ]))
}

/// Equal bays of one class.
fn equal(length: f64, n: usize, cls: &str) -> Vec<Bay> {
    (0..n)
        .map(|i| Bay {
            start: length * i as f64 / n as f64,
            end: length * (i + 1) as f64 / n as f64,
            cls: cls.to_string(),
        })
        .collect()
}

/// Lay out one interval from 0 to `length`.
fn interval(length: f64, a: &Args) -> Option<Vec<Bay>> {
    match a.distribution.as_str() {
        "positions" => {
            let mut at: Vec<f64> = a
                .pattern
                .iter()
                .map(Value::as_number)
                .collect::<Option<_>>()?;
            at.retain(|x| *x > EPSILON && *x < length - EPSILON);
            at.sort_by(f64::total_cmp);
            at.dedup_by(|x, y| (*x - *y).abs() < EPSILON);
            let mut edges = vec![0.0];
            edges.extend(at);
            edges.push(length);
            Some(
                edges
                    .windows(2)
                    .map(|w| Bay {
                        start: w[0],
                        end: w[1],
                        cls: "main".into(),
                    })
                    .collect(),
            )
        }
        "count" | "max_spacing" => {
            let e = entries(&a.pattern)?;
            let [e] = e.as_slice() else { return None };
            let n = if a.distribution == "count" {
                if e.size < 1.0 - EPSILON || (e.size - e.size.round()).abs() > EPSILON {
                    return None;
                }
                e.size.round() as usize
            } else {
                ((length / e.size) - EPSILON).ceil().max(1.0) as usize
            };
            Some(equal(length, n, &e.cls))
        }
        "fixed" => fixed(length, &entries(&a.pattern)?, a),
        "best_division" => best_division(length, &entries(&a.pattern)?, a.end_with),
        _ => None,
    }
}

fn fixed(length: f64, e: &[Entry], a: &Args) -> Option<Vec<Bay>> {
    let p: f64 = e.iter().map(|x| x.size).sum();
    let reps = |x: f64| ((x / p) - EPSILON).ceil().max(0.0);
    let offset = match a.origin.as_str() {
        "start" => 0.0,
        "end" => length - p * reps(length),
        "center" => {
            let first = length / 2.0 - p / 2.0;
            first - p * reps(first)
        }
        _ => return None,
    };
    let mut bays = Vec::new();
    let mut pos = offset;
    let mut i = 0;
    while pos < length - EPSILON {
        let entry = &e[i % e.len()];
        let (s, t) = (pos.max(0.0), (pos + entry.size).min(length));
        if t - s > EPSILON {
            let cut = t - s < entry.size - EPSILON;
            let cls = if cut && a.infill != "partial" {
                a.infill.clone()
            } else {
                entry.cls.clone()
            };
            bays.push(Bay {
                start: s,
                end: t,
                cls,
            });
        }
        pos += entry.size;
        i += 1;
    }
    Some(bays)
}

fn best_division(length: f64, e: &[Entry], end_with: Option<usize>) -> Option<Vec<Bay>> {
    if !e.iter().any(|x| x.flexible) {
        return None;
    }
    let tail = end_with.unwrap_or(e.len()).min(e.len());
    let sequence = |k: usize| -> Vec<&Entry> {
        let mut s: Vec<&Entry> = (0..k).flat_map(|_| e.iter()).collect();
        if tail < e.len() {
            s.extend(&e[..tail]);
        }
        s
    };
    let scale = |seq: &[&Entry]| {
        let fixed: f64 = seq.iter().filter(|x| !x.flexible).map(|x| x.size).sum();
        let flex: f64 = seq.iter().filter(|x| x.flexible).map(|x| x.size).sum();
        (length - fixed) / flex
    };
    let min = e.iter().map(|x| x.size).fold(f64::INFINITY, f64::min);
    let max_k = ((length / min).ceil() as usize).saturating_add(2).max(1);
    let mut best: Option<(usize, f64)> = None;
    for k in 1..=max_k {
        let s = scale(&sequence(k));
        if s <= EPSILON {
            continue;
        }
        let d = (s - 1.0).abs();
        // Ties go to the larger k: later k replace on equal distance.
        if best.is_none_or(|(_, bd)| d <= bd + EPSILON) {
            best = Some((k, d));
        }
    }
    let (k, _) = best?;
    let seq = sequence(k);
    let s = scale(&seq);
    let mut pos = 0.0;
    let mut bays = Vec::new();
    for x in seq {
        let size = if x.flexible { x.size * s } else { x.size };
        bays.push(Bay {
            start: pos,
            end: pos + size,
            cls: x.cls.clone(),
        });
        pos += size;
    }
    if let Some(last) = bays.last_mut() {
        last.end = length;
    }
    Some(bays)
}

#[cfg(test)]
mod tests {
    use crate::{Scope, Value, eval_str};

    fn ev(s: &str) -> Value {
        eval_str(s, &Scope::new()).unwrap()
    }
    fn at(s: &str) -> Vec<f64> {
        match ev(&format!("for l in ({s}).lines return l.at")) {
            Value::List(l) => l.iter().map(|v| v.as_number().unwrap()).collect(),
            v => panic!("{v:?}"),
        }
    }
    fn cls(s: &str) -> Vec<String> {
        match ev(&format!("for b in ({s}).bays return b.cls")) {
            Value::List(l) => l
                .iter()
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    _ => panic!(),
                })
                .collect(),
            v => panic!("{v:?}"),
        }
    }
    fn close(a: &[f64], b: &[f64]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-9)
    }

    #[test]
    fn fixed_from_start_cuts_the_last_bay() {
        assert!(close(
            &at("layout(5, [{size: 1.5}])"),
            &[0.0, 1.5, 3.0, 4.5, 5.0]
        ));
        assert_eq!(
            cls(r#"layout(5, [{size: 1.5}], "fixed", "start", "filler")"#),
            ["main", "main", "main", "filler"]
        );
    }

    #[test]
    fn fixed_from_end_and_centre() {
        assert!(close(
            &at(r#"layout(5, [{size: 1.5}], "fixed", "end")"#),
            &[0.0, 0.5, 2.0, 3.5, 5.0]
        ));
        // The middle pattern is centred: 1.75 to 3.25.
        assert!(close(
            &at(r#"layout(5, [{size: 1.5}], "fixed", "center")"#),
            &[0.0, 0.25, 1.75, 3.25, 4.75, 5.0]
        ));
    }

    #[test]
    fn best_division_picks_the_scale_nearest_one() {
        // 5 / 1.5: k = 3 gives s = 1.111, k = 4 gives 0.833: k = 3.
        assert!(close(
            &at(r#"layout(5, [{size: 1.5, flexible: true}], "best_division")"#),
            &[0.0, 5.0 / 3.0, 10.0 / 3.0, 5.0]
        ));
        // Fixed entries keep their size: k = 3 scales the flexible entries by exactly 1.
        assert!(close(
            &at(r#"layout(4.5, [{size: 1, flexible: true}, {size: 0.5}], "best_division")"#),
            &[0.0, 1.0, 1.5, 2.5, 3.0, 4.0, 4.5]
        ));
        assert_eq!(
            ev(r#"layout(1, [{size: 2}], "best_division")"#),
            Value::Null,
            "no flexible entry"
        );
        assert_eq!(
            ev(r#"layout(0.4, [{size: 1, flexible: true}, {size: 0.5}], "best_division")"#),
            Value::Null,
            "fixed entries alone are longer than the length"
        );
    }

    #[test]
    fn best_division_tie_goes_to_more_repetitions() {
        // length 2.4, size 1 (flexible) + 0.4 fixed per pattern, end_with 1:
        // k = 1: 1, 0.4, then entry 1 → s = (2.4 - 0.4) / 2 = 1.0 exactly.
        assert!(close(
            &at(
                r#"layout(2.4, [{size: 1, flexible: true}, {size: 0.4}], "best_division", null, null, 1)"#
            ),
            &[0.0, 1.0, 1.4, 2.4]
        ));
        // 4/3: k = 1 gives s = 4/3 and k = 2 gives 2/3, both 1/3 from 1: the larger k wins.
        assert_eq!(
            at(r#"layout(4 / 3, [{size: 1, flexible: true}], "best_division")"#).len(),
            3
        );
    }

    #[test]
    fn count_max_spacing_positions() {
        assert!(close(
            &at(r#"layout(3, [{size: 4}], "count")"#),
            &[0.0, 0.75, 1.5, 2.25, 3.0]
        ));
        assert!(close(
            &at(r#"layout(3, [{size: 1.2}], "max_spacing")"#),
            &[0.0, 1.0, 2.0, 3.0]
        ));
        assert!(close(
            &at(r#"layout(3, [2, 0.5, 9], "positions")"#),
            &[0.0, 0.5, 2.0, 3.0]
        ));
        assert_eq!(ev(r#"layout(3, [{size: 2.5}], "count")"#), Value::Null);
    }

    #[test]
    fn within_lays_out_each_interval_and_numbers_on() {
        let s = r#"layout(6, [{size: 1, cls: "spandrel"}, {size: 2, flexible: true, cls: "vision"}], "best_division", null, null, null, [3])"#;
        assert!(close(&at(s), &[0.0, 1.0, 3.0, 4.0, 6.0]));
        assert_eq!(cls(s), ["spandrel", "vision", "spandrel", "vision"]);
        assert_eq!(
            ev(&format!("for l in ({s}).lines return l.cls")),
            ev(r#"["border", "division", "break", "division", "border"]"#)
        );
        assert_eq!(ev(&format!("({s}).bays[4].index")), Value::Number(3.0));
    }

    #[test]
    fn context_form_matches_positional_form() {
        assert_eq!(
            ev(r#"layout(5, {pattern: [{size: 1.5}], origin: "end"})"#),
            ev(r#"layout(5, [{size: 1.5}], "fixed", "end")"#)
        );
    }
}
