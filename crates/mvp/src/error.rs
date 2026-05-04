use std::fmt;

#[derive(Debug)]
pub enum Error {
    Parse(String),
    Unsupported(&'static str),
    Malformed(String),
    PassFixedPointNotReached {
        iterations: usize,
    },
    PassValidationFailed {
        pass: &'static str,
        inner: Box<Error>,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Parse(s) => write!(f, "parse error: {s}"),
            Error::Unsupported(what) => write!(f, "unsupported construct: {what}"),
            Error::Malformed(s) => write!(f, "malformed source: {s}"),
            Error::PassFixedPointNotReached { iterations } => write!(
                f,
                "pass pipeline did not reach fixed point after {iterations} iterations"
            ),
            Error::PassValidationFailed { pass, inner } => {
                write!(f, "validator rejected output of pass `{pass}`: {inner}")
            }
        }
    }
}

impl std::error::Error for Error {}
