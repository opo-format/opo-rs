use std::fmt;

/// An error found while parsing or evaluating an expression.
///
/// FEEL turns most type mismatches into `null` rather than errors (for example `1 + "a"`).
/// Errors are kept for mistakes in the formula itself: bad syntax, an unknown name, a wrong
/// number of arguments, or a range used outside a `for` loop.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// The expression is not valid FEEL-subset syntax.
    Syntax {
        /// What went wrong.
        message: String,
        /// Byte offset in the source where the problem was found.
        offset: usize,
    },
    /// A name is not defined in the scope and is not a function.
    UnknownName(String),
    /// A function is called with the wrong number of arguments.
    Arity {
        /// The function name.
        function: String,
        /// The number of arguments given.
        given: usize,
    },
    /// Any other evaluation error, such as a non-list `for` source.
    Eval(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Syntax { message, offset } => write!(f, "syntax error at {offset}: {message}"),
            Error::UnknownName(n) => write!(f, "unknown name '{n}'"),
            Error::Arity { function, given } => {
                write!(f, "{function}() does not take {given} argument(s)")
            }
            Error::Eval(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}
