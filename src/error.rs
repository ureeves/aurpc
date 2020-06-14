use std::{fmt, io};

/// Error type returned by this crate.
///
/// The errors can originate from multiple sources - from the underlying UDP
/// socket, or conditions like a timeout.
#[derive(Debug)]
pub struct Error {
    io_err: io::Error,
    reason: Option<&'static str>,
}

impl Error {
    /// Creates an error with internal io::Error of the InvalidInput kind.
    pub(crate) fn invalid_input(reason: &'static str) -> Self {
        Self {
            io_err: io::Error::from(io::ErrorKind::InvalidInput),
            reason: Some(reason),
        }
    }

    /// Creates an error with internal io::Error of the Other kind.
    pub(crate) fn other(reason: &'static str) -> Self {
        Self {
            io_err: io::Error::from(io::ErrorKind::Other),
            reason: Some(reason),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        match self.reason {
            Some(reason) => {
                write!(formatter, "aurpc: {}: {}", self.io_err, reason)
            }
            None => write!(formatter, "aurpc: {}", self.io_err),
        }
    }
}

/// Allows for the use of the ? operator with an io::Error. Adds no additional
/// reason for this error.
impl From<io::Error> for Error {
    fn from(io_err: io::Error) -> Self {
        Self {
            io_err,
            reason: None,
        }
    }
}
