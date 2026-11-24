//! The safe YAML subset of OPO sources (proposal 0002, `docs/library-sources.md`).
//!
//! - One document; no anchors, aliases, merge keys (`<<`) or tags; keys are unique strings.
//! - Plain scalars follow the YAML 1.2 core schema: `null` / `~` / empty, `true` / `false`,
//!   integers, floats. Quoted scalars are always strings. Infinity and NaN are refused (JSON
//!   has no such numbers).
//! - Mappings keep their key order.

use serde_json::{Map, Number, Value as Json};
use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser};
use yaml_rust2::scanner::{Marker, TScalarStyle};

/// A YAML error with its position.
#[derive(Debug, Clone, PartialEq)]
pub struct YamlError {
    /// File or source name.
    pub source: String,
    /// 1-based line (0 when unknown).
    pub line: usize,
    /// What is wrong.
    pub message: String,
}

impl std::fmt::Display for YamlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}: {}", self.source, self.line, self.message)
    }
}

impl std::error::Error for YamlError {}

enum Frame {
    Seq(Vec<Json>),
    Map(Map<String, Json>, Option<String>),
}

struct Builder {
    stack: Vec<Frame>,
    root: Option<Json>,
    documents: usize,
    error: Option<(usize, String)>,
}

impl Builder {
    fn fail(&mut self, mark: Marker, msg: impl Into<String>) {
        if self.error.is_none() {
            self.error = Some((mark.line(), msg.into()));
        }
    }

    fn push_value(&mut self, v: Json, mark: Marker) {
        match self.stack.last_mut() {
            None => self.root = Some(v),
            Some(Frame::Seq(items)) => items.push(v),
            Some(Frame::Map(m, key)) => match key.take() {
                None => match v {
                    Json::String(k) => {
                        if k == "<<" {
                            self.fail(mark, "merge keys (<<) are not allowed");
                        } else if m.contains_key(&k) {
                            self.fail(mark, format!("duplicate key '{k}'"));
                        } else {
                            *key = Some(k);
                        }
                    }
                    other => self.fail(mark, format!("keys must be text, not {other}")),
                },
                Some(k) => {
                    m.insert(k, v);
                }
            },
        }
    }
}

/// A plain scalar under the YAML 1.2 core schema.
fn plain(s: &str) -> Result<Json, String> {
    match s {
        "" | "~" | "null" | "Null" | "NULL" => return Ok(Json::Null),
        "true" | "True" | "TRUE" => return Ok(Json::Bool(true)),
        "false" | "False" | "FALSE" => return Ok(Json::Bool(false)),
        ".inf" | ".Inf" | ".INF" | "+.inf" | "-.inf" | ".nan" | ".NaN" | ".NAN" => {
            return Err(format!(
                "'{s}' is not a finite number; quote it if it is text"
            ));
        }
        _ => {}
    }
    let digits = |t: &str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
    let unsigned = s.strip_prefix(['-', '+']).unwrap_or(s);
    if digits(unsigned) {
        if let Ok(i) = s.parse::<i64>() {
            return Ok(Json::Number(i.into()));
        }
    }
    if let Some(h) = s.strip_prefix("0x") {
        if let Ok(i) = i64::from_str_radix(h, 16) {
            return Ok(Json::Number(i.into()));
        }
    }
    if let Some(o) = s.strip_prefix("0o") {
        if let Ok(i) = i64::from_str_radix(o, 8) {
            return Ok(Json::Number(i.into()));
        }
    }
    // [-+]? ( \.[0-9]+ | [0-9]+ ( \.[0-9]* )? ) ( [eE] [-+]? [0-9]+ )?
    let (mantissa, exp) = match unsigned.find(['e', 'E']) {
        Some(i) => (&unsigned[..i], Some(&unsigned[i + 1..])),
        None => (unsigned, None),
    };
    let mantissa_ok = match mantissa.split_once('.') {
        Some((a, b)) => {
            (digits(a) || a.is_empty())
                && (digits(b) || b.is_empty())
                && !(a.is_empty() && b.is_empty())
        }
        None => digits(mantissa),
    };
    let exp_ok = exp.is_none_or(|e| digits(e.strip_prefix(['-', '+']).unwrap_or(e)));
    if mantissa_ok && exp_ok && (mantissa.contains('.') || exp.is_some()) {
        if let Some(n) = s.parse::<f64>().ok().and_then(Number::from_f64) {
            return Ok(Json::Number(n));
        }
    }
    Ok(Json::String(s.to_owned()))
}

impl MarkedEventReceiver for Builder {
    fn on_event(&mut self, ev: Event, mark: Marker) {
        if self.error.is_some() {
            return;
        }
        match ev {
            Event::DocumentStart => {
                self.documents += 1;
                if self.documents > 1 {
                    self.fail(mark, "only one YAML document per file");
                }
            }
            Event::Alias(_) => self.fail(mark, "aliases (*name) are not allowed"),
            Event::Scalar(s, style, anchor, tag) => {
                if anchor != 0 {
                    return self.fail(mark, "anchors (&name) are not allowed");
                }
                if tag.is_some() {
                    return self.fail(mark, "tags (!name) are not allowed");
                }
                let is_key = matches!(self.stack.last(), Some(Frame::Map(_, None)));
                let v = if style == TScalarStyle::Plain && !is_key {
                    match plain(&s) {
                        Ok(v) => v,
                        Err(e) => return self.fail(mark, e),
                    }
                } else {
                    Json::String(s)
                };
                self.push_value(v, mark);
            }
            Event::SequenceStart(anchor, tag) | Event::MappingStart(anchor, tag)
                if anchor != 0 || tag.is_some() =>
            {
                self.fail(
                    mark,
                    if anchor != 0 {
                        "anchors (&name) are not allowed"
                    } else {
                        "tags (!name) are not allowed"
                    },
                );
            }
            Event::SequenceStart(..) => self.stack.push(Frame::Seq(Vec::new())),
            Event::MappingStart(..) => self.stack.push(Frame::Map(Map::new(), None)),
            Event::SequenceEnd => {
                if let Some(Frame::Seq(items)) = self.stack.pop() {
                    self.push_value(Json::Array(items), mark);
                }
            }
            Event::MappingEnd => {
                if let Some(Frame::Map(m, _)) = self.stack.pop() {
                    self.push_value(Json::Object(m), mark);
                }
            }
            _ => {}
        }
    }
}

/// Parse a source file into JSON. `source` names the file in errors. An empty file is `null`.
pub fn parse(text: &str, source: &str) -> Result<Json, YamlError> {
    let mut b = Builder {
        stack: Vec::new(),
        root: None,
        documents: 0,
        error: None,
    };
    let mut p = Parser::new_from_str(text);
    if let Err(e) = p.load(&mut b, true) {
        return Err(YamlError {
            source: source.into(),
            line: e.marker().line(),
            message: e.info().to_owned(),
        });
    }
    if let Some((line, message)) = b.error {
        return Err(YamlError {
            source: source.into(),
            line,
            message,
        });
    }
    Ok(b.root.unwrap_or(Json::Null))
}

// ------------------------------------------------------------------ writing

fn plain_key(k: &str) -> bool {
    let mut c = k.chars();
    matches!(c.next(), Some(ch) if ch.is_ascii_alphabetic() || ch == '_')
        && k.chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "_.-:".contains(ch))
        && !k.ends_with(':')
        && !matches!(
            k,
            "null"
                | "Null"
                | "NULL"
                | "true"
                | "True"
                | "TRUE"
                | "false"
                | "False"
                | "FALSE"
                | "y"
                | "n"
                | "yes"
                | "no"
                | "on"
                | "off"
        )
}

fn key(k: &str) -> String {
    if plain_key(k) {
        k.to_owned()
    } else {
        Json::String(k.to_owned()).to_string()
    }
}

fn number_list(a: &[Json]) -> bool {
    !a.is_empty() && a.iter().all(Json::is_number)
}

fn inline(v: &Json) -> Option<String> {
    match v {
        Json::Array(a) if a.is_empty() => Some("[]".into()),
        Json::Object(m) if m.is_empty() => Some("{}".into()),
        Json::Array(a) if number_list(a) => Some(format!(
            "[{}]",
            a.iter().map(Json::to_string).collect::<Vec<_>>().join(", ")
        )),
        Json::Array(a)
            if a.iter()
                .all(|x| x.as_array().is_some_and(|y| number_list(y))) =>
        {
            Some(format!(
                "[{}]",
                a.iter()
                    .map(|x| inline(x).expect("number list"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        }
        Json::Array(_) | Json::Object(_) => None,
        // Strings are always double-quoted: formulas and text can never be misread.
        other => Some(other.to_string()),
    }
}

fn write_block(v: &Json, indent: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    match v {
        Json::Object(m) => {
            for (k, x) in m {
                match inline(x) {
                    Some(s) => out.push_str(&format!("{pad}{}: {s}\n", key(k))),
                    None => {
                        out.push_str(&format!("{pad}{}:\n", key(k)));
                        write_block(x, indent + 1, out);
                    }
                }
            }
        }
        Json::Array(a) => {
            for x in a {
                match inline(x) {
                    Some(s) => out.push_str(&format!("{pad}- {s}\n")),
                    None => {
                        // "- " followed by the first line of the block, the rest indented.
                        let mut inner = String::new();
                        write_block(x, indent + 1, &mut inner);
                        let body = inner
                            .strip_prefix(&"  ".repeat(indent + 1))
                            .unwrap_or(&inner);
                        out.push_str(&format!("{pad}- {body}"));
                    }
                }
            }
        }
        other => out.push_str(&format!("{pad}{}\n", inline(other).expect("scalar"))),
    }
}

/// Write JSON as source YAML: block style, two-space indent, strings double-quoted, numbers and
/// point lists in flow style. [`parse`] reads it back to the same value.
pub fn write(v: &Json) -> String {
    let mut out = String::new();
    match inline(v) {
        Some(s) if !v.is_object() && !v.is_array() => out = format!("{s}\n"),
        _ if matches!(v, Json::Object(m) if m.is_empty()) => out.push_str("{}\n"),
        _ if matches!(v, Json::Array(a) if a.is_empty()) => out.push_str("[]\n"),
        _ => write_block(v, 0, &mut out),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn p(s: &str) -> Json {
        parse(s, "t").unwrap()
    }

    #[test]
    fn core_schema_scalars() {
        assert_eq!(
            p("a: ~\nb: true\nc: 12\nd: -0.5\ne: 1e-3\nf: no\ng: '12'\nh: 0x1F\ni: .5\nj: 3."),
            json!({"a": null, "b": true, "c": 12, "d": -0.5, "e": 0.001, "f": "no", "g": "12", "h": 31, "i": 0.5, "j": 3.0})
        );
        assert_eq!(
            p("a: 1.2.3\nb: 12abc\nc: a-b\nd: e5"),
            json!({"a": "1.2.3", "b": "12abc", "c": "a-b", "d": "e5"})
        );
    }

    #[test]
    fn keys_keep_their_order() {
        let v = p("z: 1\na: 2\nm: 3");
        assert_eq!(
            v.as_object().unwrap().keys().collect::<Vec<_>>(),
            ["z", "a", "m"]
        );
    }

    #[test]
    fn unsafe_yaml_is_refused() {
        for (src, why) in [
            ("a: &x 1\nb: *x", "anchors"),
            ("a: !!str 1", "tags"),
            ("base: &b {x: 1}\nc:\n  <<: *b", "anchors"),
            ("a: 1\na: 2", "duplicate"),
            ("a: .inf", "finite"),
            ("--- 1\n--- 2", "one YAML document"),
            ("? [1, 2]\n: x", "keys must be text"),
        ] {
            let e = parse(src, "t").unwrap_err();
            assert!(e.message.contains(why), "{src}: {e}");
        }
        assert_eq!(
            parse("a:\n  b: [1,\n", "f.yaml").unwrap_err().source,
            "f.yaml"
        );
    }

    #[test]
    fn write_and_read_back() {
        let v = json!({
            "id": "door-single-swing", "version": "0.3.0", "flag": false, "nothing": null,
            "parameters": {"width": {"type": "length", "default": 0.9, "limits": {"min": 0.4}}},
            "points": [[0, 0], [1.5, 2]], "tags": ["a", "no", "1.0", "x: y", "#c"],
            "steps": [{"id": "a", "args": {"min": "[0, 0]"}}, {"id": "b", "args": {}}],
            "nested": [[{"x": 1}]], "empty": [], "map": {}, "key with space": 1, "true": 2, "Pset_X.Y": 3
        });
        let text = write(&v);
        assert_eq!(p(&text), v, "{text}");
        assert!(text.contains("points: [[0, 0], [1.5, 2]]"));
        assert!(text.contains("\"true\": 2") && text.contains("Pset_X.Y: 3"));
    }
}
