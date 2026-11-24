use crate::Error;

/// A token with the byte offset where it starts.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Token {
    pub kind: Tok,
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Tok {
    Number(f64),
    Str(String),
    /// An operator or punctuation: `.. ** <= >= != - + * / ( ) , [ ] < > = . { } :`
    Sym(&'static str),
    Name(String),
    /// `if then else for in return and or true false null`
    Keyword(&'static str),
    End,
}

const KEYWORDS: [&str; 11] = [
    "if", "then", "else", "for", "in", "return", "and", "or", "true", "false", "null",
];
const SYMBOLS: [&str; 20] = [
    "..", "**", "<=", ">=", "!=", "-", "+", "*", "/", "(", ")", ",", "[", "]", "<", ">", "=", ".",
    "{", "}",
];

pub(crate) fn tokenize(src: &str) -> Result<Vec<Token>, Error> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if c.is_ascii_digit() {
            // \d+ (\.\d+)? ([eE][-+]?\d+)?
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if i + 1 < b.len() && b[i] == b'.' && b[i + 1].is_ascii_digit() {
                i += 1;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                let mut j = i + 1;
                if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                    j += 1;
                }
                if j < b.len() && b[j].is_ascii_digit() {
                    while j < b.len() && b[j].is_ascii_digit() {
                        j += 1;
                    }
                    i = j;
                }
            }
            let n: f64 = src[start..i].parse().map_err(|_| Error::Syntax {
                message: format!("bad number '{}'", &src[start..i]),
                offset: start,
            })?;
            out.push(Token {
                kind: Tok::Number(n),
                offset: start,
            });
        } else if c == b'"' {
            // "…" where \" is an escaped quote; other backslashes are kept as written.
            i += 1;
            let mut s = String::new();
            loop {
                if i >= b.len() {
                    return Err(Error::Syntax {
                        message: "unterminated string".into(),
                        offset: start,
                    });
                }
                let ch = src[i..].chars().next().unwrap_or('\0');
                if ch == '"' {
                    i += 1;
                    break;
                }
                if ch == '\\' && i + 1 < b.len() {
                    let next = src[i + 1..].chars().next().unwrap_or('\0');
                    if next == '"' {
                        s.push('"');
                    } else {
                        s.push('\\');
                        s.push(next);
                    }
                    i += 1 + next.len_utf8();
                    continue;
                }
                s.push(ch);
                i += ch.len_utf8();
            }
            out.push(Token {
                kind: Tok::Str(s),
                offset: start,
            });
        } else if c.is_ascii_alphabetic() || c == b'_' {
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            let word = &src[start..i];
            let kind = match KEYWORDS.iter().find(|k| **k == word) {
                Some(k) => Tok::Keyword(k),
                None => Tok::Name(word.to_owned()),
            };
            out.push(Token {
                kind,
                offset: start,
            });
        } else if c == b':' {
            i += 1;
            out.push(Token {
                kind: Tok::Sym(":"),
                offset: start,
            });
        } else {
            let sym = SYMBOLS
                .iter()
                .find(|s| src[i..].starts_with(**s))
                .ok_or_else(|| Error::Syntax {
                    message: format!(
                        "unexpected character '{}'",
                        src[i..].chars().next().unwrap_or('?')
                    ),
                    offset: i,
                })?;
            i += sym.len();
            out.push(Token {
                kind: Tok::Sym(sym),
                offset: start,
            });
        }
    }
    out.push(Token {
        kind: Tok::End,
        offset: src.len(),
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Tok> {
        tokenize(src).unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn range_after_integer_is_not_a_decimal() {
        assert_eq!(
            kinds("1..3"),
            vec![Tok::Number(1.0), Tok::Sym(".."), Tok::Number(3.0), Tok::End]
        );
    }

    #[test]
    fn exponent_and_escaped_quote() {
        assert_eq!(kinds("2.5e-3"), vec![Tok::Number(0.0025), Tok::End]);
        assert_eq!(
            kinds(r#""a \"b\"""#),
            vec![Tok::Str("a \"b\"".into()), Tok::End]
        );
    }

    #[test]
    fn keywords_and_names() {
        assert_eq!(
            kinds("for x_1 in"),
            vec![
                Tok::Keyword("for"),
                Tok::Name("x_1".into()),
                Tok::Keyword("in"),
                Tok::End
            ]
        );
    }

    #[test]
    fn bad_character_is_an_error() {
        assert!(matches!(
            tokenize("1 # 2"),
            Err(Error::Syntax { offset: 2, .. })
        ));
        assert!(tokenize("\"open").is_err());
    }
}
