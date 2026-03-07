use std::{
    collections::{HashMap, hash_map::Entry},
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, RwLock},
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
/// use ignored::evaluator::Evaluator;
///
/// # std::fs::create_dir("tests/fixtures/mock-project/.git");
/// let evaluator = Evaluator::default();
/// let ignored = evaluator.is_ignored("tests/fixtures/mock-project/file.tmp");
///
/// assert!(ignored);
/// ```
#[derive(Debug, Default)]
pub struct Evaluator {
    /// A list of previously encountered git roots (directories with a `.git` inside them).
    ///
    /// This is an optimisation which allows frequent evaluations in the same/similar directory
    /// trees to skip traversal of the common ancestor directories, by jumping straight to the
    /// commonest (already encountered) git root.
    git_roots: RwLock<Vec<PathBuf>>,

    /// A map of previously parsed `.gitignore` files.
    ///
    /// This is an optimisation which allows the evaluator to avoid re-parsing frequently accessed
    /// `.gitignore` files.
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
    /// use ignored::evaluator::Evaluator;
    ///
    /// # std::fs::create_dir("tests/fixtures/mock-project/.git");
    /// let evaluator = Evaluator::default();
    /// let ignored = evaluator.is_ignored("tests/fixtures/mock-project/file.tmp");
    ///
    /// assert!(ignored);
    /// ```
    #[must_use]
    pub fn is_ignored(&self, path: impl AsRef<Path>) -> bool {
        let closest_already_encountered_git_root =
            self.get_closest_already_encountered_git_root(&path);

        let path_parts = path.as_ref().iter().collect::<Vec<&OsStr>>();
        let closest_already_encountered_git_root_offset = closest_already_encountered_git_root
            .as_ref()
            .map_or(1, |root| root.components().count());

        let mut is_in_git_root = closest_already_encountered_git_root.is_some();
        let mut is_ignored = false;

        for i in closest_already_encountered_git_root_offset..path_parts.len() {
            let base_path: PathBuf = path_parts[0..i].iter().collect();

            if closest_already_encountered_git_root
                .as_ref()
                .is_some_and(|git_root| git_root == &base_path)
            {
                // This is the git root we've already identified, no need to add it to the git
                // roots list.
            } else if base_path.join(".git").exists() {
                // We've encountered this git root for the first time, we need to update our list of
                // encountered git roots. We also might already be in a git root (i.e. `.git` in a
                // subdirectory of another git root), in which case we need to reset our current
                // ignored decision.

                if is_in_git_root {
                    // We've reached _another_ git root, even though we're already in a git root (i.e.
                    // a repo inside a repo). We should reset our current ignored decision.
                    is_ignored = false;

                    log::debug!(
                        "Encountered recursive git root at: {}",
                        base_path.as_path().display()
                    );
                } else {
                    is_in_git_root = true;

                    log::debug!("Encountered git root at: {}", base_path.as_path().display());
                }

                match self.git_roots.write() {
                    Ok(mut guard) => {
                        guard.push(base_path.clone());
                    }
                    Err(e) => {
                        log::error!(
                            "Unable to update git roots with newly encountered git root ({}): {}",
                            base_path.display(),
                            e
                        );
                    }
                }
            } else {
                // We've still not reached a git root (i.e. a `.git` folder). Conforming to git's
                // semantics this means any `.gitignore` files don't apply.
                continue;
            }

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
            let parent_path = path_parts[0..=i].iter().collect::<PathBuf>().join("");

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
                is_ignored = true;

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
                // they might ignore whole directories which will prevent evaluations
                // in the lower levels from having any effect.
                is_ignored = result;
            }
        }

        log::debug!("{} is ignored: {is_ignored}", path.as_ref().display());

        is_ignored
    }

    /// Get the closest already encountered git root which was found in a previous traversal of the
    /// same directory tree.
    ///
    /// This _only_ looks for already encountered git roots, and even when one is returned,
    /// doesn't guarantee another git root further down the directory tree won't be encountered
    /// (i.e. a `.git` where there is a `.git` in a parent directory).
    fn get_closest_already_encountered_git_root(&self, path: impl AsRef<Path>) -> Option<PathBuf> {
        if let Ok(already_encountered_git_roots) = self.git_roots.read() {
            return already_encountered_git_roots
                .iter()
                .fold(None, |previous_match, git_root| {
                    if !path.as_ref().starts_with(git_root) {
                        return previous_match;
                    }

                    if previous_match
                        .is_none_or(|previous_match| path.as_ref().starts_with(previous_match))
                    {
                        return Some(git_root);
                    }

                    previous_match
                })
                .map(std::borrow::ToOwned::to_owned);
        }

        None
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
