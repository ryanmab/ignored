//! An evaluator for `.gitignore` files in a given directory and its parent directories.
//!
//! The full specification of the `.gitignore` format, along with the behavior and hierarchy of `.gitignore` files,
//! can be found in the [git documentation](https://git-scm.com/docs/gitignore).

mod error;
mod evaluator;
mod file;
mod glob;
mod types;
mod utils;

pub use evaluator::Evaluator;
pub use file::File;
pub use glob::Glob;

pub use error::Error;
pub(crate) use types::Result;
