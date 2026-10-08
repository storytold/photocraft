//! Error type for every fallible operation in this crate.

use thiserror::Error;

/// Errors produced while parsing asset files.
///
/// Parsers never panic on malformed input; every failure is reported through this type.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum AssetError {
    /// The input ended before a structure was complete.
    #[error("unexpected end of data at offset {offset} (needed {needed} more bytes)")]
    UnexpectedEof {
        /// Offset where the read started.
        offset: usize,
        /// Number of bytes that were requested but not available.
        needed: usize,
    },
    /// A four-byte signature or tag did not match.
    #[error("invalid signature {found:?}, expected {expected}")]
    InvalidSignature {
        /// Human-readable description of the expected signature(s).
        expected: &'static str,
        /// The bytes that were found.
        found: [u8; 4],
    },
    /// A count or nesting depth exceeded the safety limits.
    #[error("limit exceeded: {0}")]
    LimitExceeded(&'static str),
    /// Structurally invalid data.
    #[error("invalid data: {0}")]
    Invalid(String),
    /// A version or data type this crate does not read.
    #[error("unsupported: {0}")]
    Unsupported(String),
}

impl AssetError {
    pub(crate) fn invalid(msg: impl Into<String>) -> Self {
        AssetError::Invalid(msg.into())
    }
}

/// Convenience alias.
pub type Result<T, E = AssetError> = std::result::Result<T, E>;
