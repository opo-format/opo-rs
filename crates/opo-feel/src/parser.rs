use std::collections::BTreeSet;

use crate::Error;
use crate::builtins::is_builtin;
use crate::lexer::{Tok, Token, tokenize};
use crate::value::Value;

/// A parsed expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// A literal number, string, boolean or `null`.
    Literal(Value),
    /// A name from the scope.
    Name(String),
    /// A list literal `[a, b]`.
    List(Vec<Expr>),
    /// A context literal `{a: 1, "b c": 2}`.
    Context(Vec<(String, Expr)>),
    /// Context access `base.key`; on a list it maps over the items.
    Path(Box<Expr>, String),
    /// A call `name(args)`.
    Call(String, Vec<Expr>),
    /// Indexing `list[i]`, 1-based; negative counts from the end.
    Index(Box<Expr>, Box<Expr>),
    /// Unary minus.
    Negate(Box<Expr>),
    /// `+ - * / **`.
    Arithmetic(char, Box<Expr>, Box<Expr>),
    /// `= != < <= > >=`.
    Compare(&'static str, Box<Expr>, Box<Expr>),
    /// `and`.
    And(Box<Expr>, Box<Expr>),
    /// `or`.
    Or(Box<Expr>, Box<Expr>),
    /// `if c then a else b`.
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    /// `for x in s, y in t return body`.
    For(Vec<(String, Source)>, Box<Expr>),
}

/// The source of one `for` binding.
#[derive(Debug, Clone, PartialEq)]
pub enum Source {
    /// A range `a..b` (spec §4.4: ranges are only allowed here).
    Range(Expr, Expr),
    /// Any expression giving a list.
    List(Expr),
}

/// Parse an expression of the FEEL subset.
pub fn parse(src: &str) -> Result<Expr, Error> {
    let mut p = Parser {
        tokens: tokenize(src)?,
        pos: 0,
    };
    let e = p.expr()?;
    match p.peek() {
        Tok::End => Ok(e),
        t => Err(p.error(format!("unexpected {} after the expression", describe(t)))),
    }
}

/// The names an expression reads from its scope: every name that is not bound by a `for`
/// loop and not a built-in function. Called names are included when they are not built-ins
/// (they are functions the application must provide).
pub fn free_names(expr: &Expr) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    walk(expr, &mut Vec::new(), &mut out);
    out
}

fn walk(e: &Expr, bound: &mut Vec<String>, out: &mut BTreeSet<String>) {
    match e {
        Expr::Literal(_) => {}
        Expr::Name(n) => {
            if !bound.contains(n) {
                out.insert(n.clone());
            }
        }
        Expr::List(items) => items.iter().for_each(|x| walk(x, bound, out)),
        Expr::Context(entries) => entries.iter().for_each(|(_, x)| walk(x, bound, out)),
        Expr::Path(b, _) | Expr::Negate(b) => walk(b, bound, out),
        Expr::Call(name, args) => {
            if !is_builtin(name) && !bound.contains(name) {
                out.insert(name.clone());
            }
            args.iter().for_each(|x| walk(x, bound, out));
        }
        Expr::Index(a, b)
        | Expr::Arithmetic(_, a, b)
        | Expr::Compare(_, a, b)
        | Expr::And(a, b)
        | Expr::Or(a, b) => {
            walk(a, bound, out);
            walk(b, bound, out);
        }
        Expr::If(c, a, b) => {
            walk(c, bound, out);
            walk(a, bound, out);
            walk(b, bound, out);
        }
        Expr::For(binds, body) => {
            let depth = bound.len();
            for (name, src) in binds {
                match src {
                    Source::Range(a, b) => {
                        walk(a, bound, out);
                        walk(b, bound, out);
                    }
                    Source::List(x) => walk(x, bound, out),
                }
                bound.push(name.clone());
            }
            walk(body, bound, out);
            bound.truncate(depth);
        }
    }
}

fn describe(t: &Tok) -> String {
    match t {
        Tok::Number(n) => format!("number {n}"),
        Tok::Str(s) => format!("string \"{s}\""),
        Tok::Sym(s) => format!("'{s}'"),
        Tok::Name(n) => format!("name '{n}'"),
        Tok::Keyword(k) => format!("'{k}'"),
        Tok::End => "end of input".into(),
    }
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

type R = Result<Expr, Error>;

impl Parser {
    fn peek(&self) -> &Tok {
        &self.tokens[self.pos].kind
    }

    fn next(&mut self) -> Tok {
        let t = self.tokens[self.pos].kind.clone();
        if !matches!(t, Tok::End) {
            self.pos += 1;
        }
        t
    }

    fn error(&self, message: String) -> Error {
        Error::Syntax {
            message,
            offset: self.tokens[self.pos].offset,
        }
    }

    fn accept_sym(&mut self, s: &str) -> bool {
        if matches!(self.peek(), Tok::Sym(x) if *x == s) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn accept_kw(&mut self, k: &str) -> bool {
        if matches!(self.peek(), Tok::Keyword(x) if *x == k) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect_sym(&mut self, s: &str) -> Result<(), Error> {
        if self.accept_sym(s) {
            Ok(())
        } else {
            Err(self.error(format!("expected '{s}', found {}", describe(self.peek()))))
        }
    }

    fn expect_kw(&mut self, k: &str) -> Result<(), Error> {
        if self.accept_kw(k) {
            Ok(())
        } else {
            Err(self.error(format!("expected '{k}', found {}", describe(self.peek()))))
        }
    }

    fn expr(&mut self) -> R {
        if self.accept_kw("if") {
            let c = self.expr()?;
            self.expect_kw("then")?;
            let a = self.expr()?;
            self.expect_kw("else")?;
            let b = self.expr()?;
            return Ok(Expr::If(Box::new(c), Box::new(a), Box::new(b)));
        }
        if self.accept_kw("for") {
            let mut binds = Vec::new();
            loop {
                let name = match self.next() {
                    Tok::Name(n) => n,
                    t => {
                        return Err(self.error(format!(
                            "expected a name after 'for', found {}",
                            describe(&t)
                        )));
                    }
                };
                self.expect_kw("in")?;
                binds.push((name, self.for_source()?));
                if !self.accept_sym(",") {
                    break;
                }
            }
            self.expect_kw("return")?;
            let body = self.expr()?;
            return Ok(Expr::For(binds, Box::new(body)));
        }
        self.disjunction()
    }

    /// `a..b` or any expression. Ranges are tried first and the parser backs up if the
    /// source is not a range.
    fn for_source(&mut self) -> Result<Source, Error> {
        let mark = self.pos;
        if let Ok(start) = self.additive() {
            if self.accept_sym("..") {
                return Ok(Source::Range(start, self.additive()?));
            }
        }
        self.pos = mark;
        Ok(Source::List(self.expr()?))
    }

    fn disjunction(&mut self) -> R {
        let mut e = self.conjunction()?;
        while self.accept_kw("or") {
            e = Expr::Or(Box::new(e), Box::new(self.conjunction()?));
        }
        Ok(e)
    }

    fn conjunction(&mut self) -> R {
        let mut e = self.comparison()?;
        while self.accept_kw("and") {
            e = Expr::And(Box::new(e), Box::new(self.comparison()?));
        }
        Ok(e)
    }

    fn comparison(&mut self) -> R {
        let e = self.additive()?;
        if matches!(self.peek(), Tok::Sym("..")) {
            return Err(
                self.error("a range (a..b) is only allowed as the source of a for loop".into())
            );
        }
        for op in ["=", "!=", "<", "<=", ">", ">="] {
            if self.accept_sym(op) {
                let rhs = self.additive()?;
                return Ok(Expr::Compare(op, Box::new(e), Box::new(rhs)));
            }
        }
        Ok(e)
    }

    fn additive(&mut self) -> R {
        let mut e = self.multiplicative()?;
        loop {
            let op = if self.accept_sym("+") {
                '+'
            } else if self.accept_sym("-") {
                '-'
            } else {
                return Ok(e);
            };
            e = Expr::Arithmetic(op, Box::new(e), Box::new(self.multiplicative()?));
        }
    }

    fn multiplicative(&mut self) -> R {
        let mut e = self.power()?;
        loop {
            let op = if self.accept_sym("*") {
                '*'
            } else if self.accept_sym("/") {
                '/'
            } else {
                return Ok(e);
            };
            e = Expr::Arithmetic(op, Box::new(e), Box::new(self.power()?));
        }
    }

    /// `**` is right-associative.
    fn power(&mut self) -> R {
        let e = self.unary()?;
        if self.accept_sym("**") {
            return Ok(Expr::Arithmetic('^', Box::new(e), Box::new(self.power()?)));
        }
        Ok(e)
    }

    fn unary(&mut self) -> R {
        if self.accept_sym("-") {
            return Ok(Expr::Negate(Box::new(self.unary()?)));
        }
        self.postfix()
    }

    fn postfix(&mut self) -> R {
        let mut e = self.atom()?;
        loop {
            if self.accept_sym(".") {
                let key = match self.next() {
                    Tok::Name(n) => n,
                    Tok::Keyword(k) => k.to_owned(),
                    t => {
                        return Err(self
                            .error(format!("expected a name after '.', found {}", describe(&t))));
                    }
                };
                e = Expr::Path(Box::new(e), key);
            } else if matches!(self.peek(), Tok::Sym("(")) {
                let Expr::Name(name) = e else {
                    return Err(self.error("only a function name can be called".into()));
                };
                self.pos += 1;
                let args = self.items(")")?;
                e = Expr::Call(name, args);
            } else if self.accept_sym("[") {
                let i = self.expr()?;
                self.expect_sym("]")?;
                e = Expr::Index(Box::new(e), Box::new(i));
            } else {
                return Ok(e);
            }
        }
    }

    /// Comma-separated expressions up to `close` (already past the opening bracket).
    fn items(&mut self, close: &str) -> Result<Vec<Expr>, Error> {
        let mut out = Vec::new();
        if self.accept_sym(close) {
            return Ok(out);
        }
        loop {
            out.push(self.expr()?);
            if self.accept_sym(close) {
                return Ok(out);
            }
            self.expect_sym(",")?;
        }
    }

    fn atom(&mut self) -> R {
        match self.next() {
            Tok::Number(n) => Ok(Expr::Literal(Value::Number(n))),
            Tok::Str(s) => Ok(Expr::Literal(Value::String(s))),
            Tok::Keyword("true") => Ok(Expr::Literal(Value::Bool(true))),
            Tok::Keyword("false") => Ok(Expr::Literal(Value::Bool(false))),
            Tok::Keyword("null") => Ok(Expr::Literal(Value::Null)),
            Tok::Name(n) => Ok(Expr::Name(n)),
            Tok::Sym("(") => {
                let e = self.expr()?;
                self.expect_sym(")")?;
                Ok(e)
            }
            Tok::Sym("[") => Ok(Expr::List(self.items("]")?)),
            Tok::Sym("{") => {
                let mut entries = Vec::new();
                if self.accept_sym("}") {
                    return Ok(Expr::Context(entries));
                }
                loop {
                    let key = match self.next() {
                        Tok::Name(n) | Tok::Str(n) => n,
                        Tok::Keyword(k) => k.to_owned(),
                        t => {
                            return Err(self
                                .error(format!("expected a context key, found {}", describe(&t))));
                        }
                    };
                    self.expect_sym(":")?;
                    entries.push((key, self.expr()?));
                    if self.accept_sym("}") {
                        return Ok(Expr::Context(entries));
                    }
                    self.expect_sym(",")?;
                }
            }
            t => {
                self.pos = self.pos.saturating_sub(1);
                Err(self.error(format!("unexpected {}", describe(&t))))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_outside_for_is_rejected() {
        assert!(matches!(parse("1..3"), Err(Error::Syntax { .. })));
        assert!(matches!(parse("count(1..3)"), Err(Error::Syntax { .. })));
    }

    #[test]
    fn for_source_may_be_any_list_expression() {
        let e = parse("for x in [1, 2] return x").unwrap();
        assert!(matches!(e, Expr::For(ref b, _) if matches!(b[0].1, Source::List(_))));
        let e = parse("for i in 1..count(p) return i").unwrap();
        assert!(matches!(e, Expr::For(ref b, _) if matches!(b[0].1, Source::Range(..))));
    }

    #[test]
    fn power_is_right_associative() {
        assert_eq!(
            parse("2 ** 3 ** 2").unwrap(),
            parse("2 ** (3 ** 2)").unwrap()
        );
    }

    #[test]
    fn only_names_can_be_called() {
        assert!(parse("[f](1)").is_err());
        assert!(parse("a.b(1)").is_err());
    }

    #[test]
    fn trailing_input_is_an_error() {
        assert!(matches!(parse("1 2"), Err(Error::Syntax { offset: 2, .. })));
    }

    #[test]
    fn free_names_skip_builtins_and_loop_variables() {
        let e = parse("for i in 1..n return sum(layers[i].t) + extrude(p, 0, h)").unwrap();
        let names: Vec<_> = free_names(&e).into_iter().collect();
        assert_eq!(names, ["extrude", "h", "layers", "n", "p"]);
    }

    #[test]
    fn keyword_keys_in_paths_and_contexts() {
        assert!(parse("{in: 1}.in").is_ok());
    }
}
