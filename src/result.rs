use crate::error::Error;

/// Result type to keep things DRY.
pub type Result<T> = std::result::Result<T, Error>;
