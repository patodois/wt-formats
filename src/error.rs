//! The error type of every reader in this crate.

use std::fmt;

/// What went wrong while reading a file.
///
/// Readers never panic on malformed input: whatever the bytes say, they return one of these.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A read or a table reaches past the end of the data it lives in.
    OutOfBounds {
        /// First byte that is missing.
        at: usize,
        /// Size of the data.
        len: usize,
    },
    /// The first four bytes are not the label the format starts with.
    BadLabel {
        /// The label found, as text.
        found: String,
    },
    /// The data is structured wrongly: a count that does not fit, a pointer to nowhere, a loop.
    Invalid(String),
    /// The data is well formed but uses something this crate does not read.
    Unsupported(String),
    /// A compressed block could not be unpacked, or unpacks to something other than its header says.
    Decompress(String),
    /// A picture could not be written.
    Encode(String),
}

/// `Result` with this crate's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub(crate) fn invalid(what: impl Into<String>) -> Self {
        Error::Invalid(what.into())
    }

    pub(crate) fn unsupported(what: impl Into<String>) -> Self {
        Error::Unsupported(what.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::OutOfBounds { at, len } => write!(f, "byte {at} is past the end of the {len} bytes of data"),
            Error::BadLabel { found } => write!(f, "unexpected label {found:?}"),
            Error::Invalid(what) => write!(f, "invalid data: {what}"),
            Error::Unsupported(what) => write!(f, "not supported: {what}"),
            Error::Decompress(what) => write!(f, "cannot unpack: {what}"),
            Error::Encode(what) => write!(f, "cannot encode: {what}"),
        }
    }
}

impl std::error::Error for Error {}
