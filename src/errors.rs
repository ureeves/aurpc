use std::io;

/// Creates an error with internal io::Error of the InvalidInput kind.
pub(crate) fn invalid_input(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, reason)
}

/// Creates an error with internal io::Error of the Other kind.
pub(crate) fn other(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Other, reason)
}
