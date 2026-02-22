use std::{
    collections::{HashMap, hash_map::Entry},
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use crate::evaluator::{self, File, types::Result, utils};

/// An evaluator for `.gitignore` files in a given directory and its parent directories.
///
/// The evaluator maintains an internal cache of parsed `.gitignore` files to optimize performance when evaluating
/// multiple paths within the same directory structure.
///
/// The full specification of the `.gitignore` format, along with the behaviour and hierarchy of `.gitignore` files,
/// can be found in the [git documentation](https://git-scm.com/docs/gitignore#_description).
///
/// # Examples
///
/// ```rust
/// use std::path::Path;
/// use ignored::evaluator::Evaluator;
///
/// let evaluator = Evaluator::default();
/// let ignored = evaluator.is_ignored("tests/fixtures/mock-project/file.tmp");
///
/// assert!(ignored);
/// ```
#[derive(Debug, Default)]
pub struct Evaluator {
    files: Mutex<HashMap<PathBuf, Arc<File>>>,
}

impl Evaluator {
    /// Evaluate whether an arbitrary path is ignored based on the `.gitignore` files in its directory
    /// and parent directories.
    ///
    /// `ignored` follows the precedence rules defined in the [git documentation](https://git-scm.com/docs/gitignore#_description) and
    /// returns `true` if the path is ignored, and `false` otherwise.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::path::Path;
    /// use ignored::evaluator::Evaluator;
    ///
    /// let path = Path::new("tests/fixtures/mock-project/file.tmp");
    ///
    /// let evaluator = Evaluator::default();
    /// let ignored = evaluator.is_ignored(path);
    ///
    /// assert!(ignored);
    /// ```
    #[must_use]
    pub fn is_ignored(&self, path: impl AsRef<Path>) -> bool {
        let parts = path.as_ref().iter().collect::<Vec<&OsStr>>();

        let mut ignored = false;

        for i in 1..=parts.len() {
            let base_path: PathBuf = parts[0..i].iter().collect();

            let potential_gitignore = base_path.join(".gitignore");

            if !potential_gitignore.exists() {
                continue;
            }

            let gitignore_file = match self
                .get_or_parse_gitignore(base_path.as_path(), potential_gitignore.as_path())
            {
                Ok(Some(gitignore_file)) => gitignore_file,
                Ok(None) => continue,
                Err(e) => {
                    log::error!(
                        "Failed to read .gitignore file at {}: {:?}",
                        potential_gitignore.display(),
                        e
                    );

                    continue;
                }
            };

            // NB: Because `[0..=i]` is inclusive (and the range driving this loop starts at 1) it's
            // effectively the same as `[0..i+1]`, which is why it works to select the parent.
            let parent_path = parts[0..=i].iter().collect::<PathBuf>().join("");

            if gitignore_file
                .is_ignored(parent_path.as_path())
                .is_some_and(|ignored| ignored)
            {
                // Git doesn’t list excluded directories for performance reasons, so any patterns one
                // contained files have no effect, no matter where they are defined.
                //
                // In other words, despite keep.me being explicitly not ignored in the example below, the
                // vendor directory is still ignored, which causes keep.me to be ignored as well:
                //
                // ```
                // vendor/
                // !vendor/keep.me
                // ```
                ignored = true;

                log::debug!(
                    "{} is ignored so {} is ignored by association.",
                    parent_path.as_path().display(),
                    path.as_ref().display()
                );

                break;
            }

            if let Some(result) = gitignore_file.is_ignored(path.as_ref()) {
                // Patterns in the higher level files are overridden by those in
                // lower level files down to the directory containing the file.
                //
                // We _have to_ check patterns in the higher levels _first_ because
                // they might ignore whole directories which will prevent evaluations #[inline]
                // in the lower levels from having any effect.
                ignored = result;
            }
        }

        log::debug!("{} is ignored: {ignored}", path.as_ref().display());

        ignored
    }

    /// Parse a `.gitignore` file at the given path, or return a cached version if it has already been parsed
    /// and hasn't changed since.
    fn get_or_parse_gitignore(
        &self,
        base_path: impl AsRef<Path>,
        potential_gitignore: impl AsRef<Path>,
    ) -> Result<Option<Arc<File>>> {
        if !potential_gitignore.as_ref().exists() {
            return Ok(None);
        }

        let mut guard = self.files.lock().map_err(|_| {
            evaluator::Error::CachePoisoned(potential_gitignore.as_ref().to_path_buf())
        })?;

        let gitignore_file = match guard.entry(potential_gitignore.as_ref().to_path_buf()) {
            Entry::Occupied(mut e) => {
                {
                    let existing_file = e.get_mut();

                    let (target_checksum, _) = crate::utils::compute_checksum(
                        potential_gitignore.as_ref(),
                    )
                    .map_err(|e| evaluator::Error::FileError {
                        file: potential_gitignore.as_ref().to_path_buf(),
                        source: e,
                    })?;

                    if existing_file.checksum == target_checksum {
                        return Ok(Some(Arc::clone(existing_file)));
                    }
                }

                // We've parsed this file before but the content has changed. We need to re-parse
                // it from scratch
                Arc::clone(&e.insert(Arc::new(utils::read_gitignore(
                    base_path,
                    potential_gitignore.as_ref(),
                )?)))
            }
            Entry::Vacant(e) => {
                let gitignore_file = Arc::new(utils::read_gitignore(
                    base_path,
                    potential_gitignore.as_ref(),
                )?);

                // We've never encountered this file before, we need to parse it
                Arc::clone(e.insert(gitignore_file))
            }
        };

        drop(guard);

        Ok(Some(gitignore_file))
    }
}
