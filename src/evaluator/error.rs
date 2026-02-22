use std::{io, path::PathBuf};

use thiserror::Error;

use crate::lexer;

/// Errors that can occur during evaluation of `.gitignore` files
/// and patterns.
#[derive(Debug, Error)]
pub enum Error {
    /// A pattern contained inside a `.gitignore` file was not valid syntactically.
    #[error("Invalid pattern: {pattern}")]
    InvalidPattern {
        /// The pattern that was invalid.
        pattern: String,

        /// The underlying error from the lexer (if available).
        source: Option<lexer::Error>,
    },
    /// A pattern contained inside a `.gitignore` file did not compile to a valid
    /// regular expression, and therefore could not be evaluated.
    #[error("Pattern ({pattern}) did not compile to a valid regular expression: {source}")]
    InvalidRegex {
        /// The pattern used to create the regular expression.
        pattern: String,

        /// The regex expression that was invalid.
        regex: String,

        /// The underlying error from the regex compiler.
        source: regex::Error,
    },
    /// An error occurred while trying to read a `.gitignore` file because the file cache which
    /// stores the contents of previously read files is poisoned.
    #[error("Unable to read gitignore file, as the underlying file cache is poisoned: {0}")]
    CachePoisoned(PathBuf),

    /// An error occurred while trying to read a `.gitignore` file.
    #[error("Unable to read {file}: {source}")]
    FileError {
        /// The path to the `.gitignore` file that could not be read.
        file: PathBuf,

        /// The underlying error from the filesystem.
        source: io::Error,
    },
}
