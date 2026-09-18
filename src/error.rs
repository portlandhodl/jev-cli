//! Errors returned by jev-cli.

/// Errors returned by this crate.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The TypeSafe API client failed.
    #[error("typesafe API error: {0}")]
    Jev(#[from] jev_sdk::Error),

    /// A local IO operation failed.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// A command-line or configuration value is invalid.
    #[error("invalid input: {0}")]
    Usage(String),

    /// The API responded successfully but with an unexpected shape.
    #[error("unexpected API response: {0}")]
    Unexpected(String),
}
