use std::fmt;

#[derive(Debug)]
pub enum Error {
    Parse(String),
    Unsupported(&'static str),
    Malformed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Parse(s) => write!(f, "parse error: {s}"),
            Error::Unsupported(what) => write!(f, "unsupported construct: {what}"),
            Error::Malformed(s) => write!(f, "malformed source: {s}"),
        }
    }
}

impl std::error::Error for Error {}
