use thiserror::Error;

/// Errors that can occur during lexing of `.gitignore` patterns.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum Error {
    /// A pattern contained inside a `.gitignore` file was not valid syntactically.
    #[error("Invalid pattern: {0}")]
    InvalidPattern(String),
}
