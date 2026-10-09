use std::fmt;

/// Errors when parsing a Paint.NET (.pdn) document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// File is corrupted, truncated or violates the PDN format grammar.
    Malformed(&'static str),
    /// File uses features not supported by this reader.
    Unsupported(String),
    /// File exceeds a safety limit.
    Limit(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(msg) => write!(f, "damaged PDN file: {msg}"),
            Self::Unsupported(msg) => write!(f, "unsupported PDN file: {msg}"),
            Self::Limit(msg) => write!(f, "PDN file exceeds a safety limit: {msg}"),
        }
    }
}

impl std::error::Error for Error {}
