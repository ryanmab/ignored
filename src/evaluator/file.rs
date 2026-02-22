use std::path::{MAIN_SEPARATOR_STR, Path, PathBuf};

use crate::evaluator::Glob;

/// A parsed `.gitignore` file.
///
/// This holds the base path (the directory containing the `.gitignore` in the filesystem), the
/// content (a list of syntactically valid [`Glob`] patterns), and the checksum of the file content
/// as it was when the [`Glob`] patterns were parsed (use for caching purposes).
#[derive(Debug)]
pub struct File {
    /// The base path of the `.gitignore` file, which is the directory containing the
    /// `.gitignore` file in the filesystem.
    pub base_path: PathBuf,

    /// The content of the `.gitignore` file, which is a list of syntactically valid [`Glob`] patterns.
    pub content: Vec<Glob>,

    /// The checksum of the file content as it was when the [`Glob`] patterns were parsed, used for caching purposes.
    pub checksum: Vec<u8>,
}

impl File {
    /// Create a new [`crate::evaluator::File`] with the given base path, content, and checksum.
    #[must_use]
    pub fn new(base_path: impl AsRef<Path>, content: Vec<Glob>, checksum: Vec<u8>) -> Self {
        Self {
            base_path: base_path.as_ref().into(),
            content,
            checksum,
        }
    }

    /// Evaluate whether an arbitrary path is ignored based on the content of this `.gitignore` file.
    ///
    /// Returns `None` if none of the patterns in the `.gitignore` file match the path, which means that the
    /// path is not ignored based on this file.
    ///
    /// Otherwise, returns `Some(true)` if the pattern matches and the path **is** ignored, or `Some(false)`
    /// if the pattern matches the path, but is negated (i.e. **is not** ignored).
    pub fn is_ignored(&self, path: impl AsRef<Path>) -> Option<bool> {
        let m = path
            .as_ref()
            .as_os_str()
            .to_str()
            .unwrap_or_default()
            // Must remove the leading base path (so the path is relative to the `.gitignore` file)
            // and any leading path separator (since `.gitignore` patterns are relative to the directory
            // of the `.gitignore` file, and should not start with a path separator)
            .trim_start_matches(self.base_path.to_str().unwrap_or_default())
            .trim_start_matches(MAIN_SEPARATOR_STR);

        for glob in self
            .content
            .iter()
            // Within one level of precedence, the last matching pattern decides the outcome
            .rev()
        {
            if let Some(ignored) = glob.is_ignored(m) {
                return Some(ignored);
            }
        }

        None
    }
}
