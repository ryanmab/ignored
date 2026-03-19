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
        // Patterns read from a `.gitignore` file in the same directory as
        // the path, or in any parent directory (up to the top-level of
        // the working tree)
        let git_root = match self.evaluate_gitignore_files(path.as_ref()) {
            (_, Some(is_ignored)) => {
                log::debug!(
                    "{} is ignored by .gitignore: {is_ignored}",
                    path.as_ref().display()
                );

                return is_ignored;
            }
            (git_root, None) => git_root,
        };

        // Patterns read from $GIT_DIR/info/exclude.
        if let Some(ref git_root) = git_root {
            if let Some(is_ignored) = self.evaluate_git_exclude_file(git_root, path.as_ref()) {
                return is_ignored;
            }
        }

        // Patterns read from the file specified by the configuration variable core.excludesFile.
        if let Some(is_ignored) =
            self.evaluate_global_git_excludes_file(git_root.as_ref(), path.as_ref())
        {
            return is_ignored;
        }

        false
    }

    /// Evaluate the repositories `.gitignore` files to determine if a given file or path is
    /// ignored.
    ///
    /// This is the first of three methods of ignoring files in git.
    ///
    /// This follows the precedence rules defined in the [git documentation](https://git-scm.com/docs/gitignore#_description).
    ///
    /// During traversal it also records the closest relative git root (directory containing a
    /// `.git`), which is beneficial for the second evaluation method - which is an ignore file
    /// listed in the git root (`.git/info/exclude`).
    ///
    /// This method returns true or false, which denotes whether the file is ignored or not, only if the path was
    /// matched in at least one `.gitignore` file. If not, [`Option::None`] will be returned,
    /// denoting that no `.gitignore` file matched the path in either direction.
    fn evaluate_gitignore_files(&self, path: impl AsRef<Path>) -> (Option<PathBuf>, Option<bool>) {
        let mut closest_git_root = self.get_closest_already_encountered_git_root(&path);

        let path_parts = path.as_ref().iter().collect::<Vec<&OsStr>>();
        let closest_git_root_offset = closest_git_root
            .as_ref()
            .map_or(1, |root| root.components().count());

        let mut is_in_git_root = closest_git_root.is_some();
        let mut is_ignored = None;

        for i in closest_git_root_offset..path_parts.len() {
            let base_path: PathBuf = path_parts[0..i].iter().collect();

            if closest_git_root
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
                    is_ignored = None;

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

                // Update the closest git root as we've now encountered one we previously didn't
                // know about.
                closest_git_root = Some(base_path.clone());
            } else {
                // We've still not reached a git root (i.e. a `.git` folder). Conforming to git's
                // semantics this means any `.gitignore` files don't apply.
                continue;
            }

            let potential_gitignore = base_path.join(".gitignore");

            let gitignore_file = match self
                .get_or_parse_gitignore(Option::<&PathBuf>::None, potential_gitignore.as_path())
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
                log::debug!(
                    "{} is ignored so {} is ignored by association.",
                    parent_path.as_path().display(),
                    path.as_ref().display()
                );

                return (closest_git_root, Some(true));
            }

            if let Some(result) = gitignore_file.is_ignored(path.as_ref()) {
                // Patterns in the higher level files are overridden by those in
                // lower level files down to the directory containing the file.
                //
                // We _have to_ check patterns in the higher levels _first_ because
                // they might ignore whole directories which will prevent evaluations
                // in the lower levels from having any effect.
                is_ignored = Some(result);
            }
        }

        (closest_git_root, is_ignored)
    }

    /// Evaluate the repositories `.git/info/exclude` located at the root of the working tree.
    ///
    /// This is the second of three methods of ignoring files in git.
    ///
    /// This follows the precedence rules defined in the [git documentation](https://git-scm.com/docs/gitignore#_description).
    ///
    /// This method returns true or false, which denotes whether the file is ignored or not, only if the path was
    /// matched in `.git/info/exclude`. If not, [`Option::None`] will be returned, denoting that the path was not listed.
    fn evaluate_git_exclude_file(
        &self,
        git_root: &impl AsRef<Path>,
        path: impl AsRef<Path>,
    ) -> Option<bool> {
        let exclude_file = git_root.as_ref().join(".git").join("info").join("exclude");

        let gitignore_file = match self.get_or_parse_gitignore(Some(git_root), &exclude_file) {
            Ok(file) => file,
            Err(e) => {
                log::error!(
                    "Failed to read .gitignore file at {}: {:?}",
                    exclude_file.display(),
                    e
                );

                None
            }
        };

        if let Some(gitignore_file) = gitignore_file {
            if let Some(is_ignored) = gitignore_file.is_ignored(&path) {
                log::debug!(
                    "{} is ignored by {}: {is_ignored}",
                    exclude_file.as_path().display(),
                    path.as_ref().display()
                );

                return Some(is_ignored);
            }
        }

        None
    }

    /// Evaluate the users global `.gitignore` file (located by default at `$XDG_CONFIG_HOME/git/ignore`, or
    /// if `$XDG_CONFIG_HOME` is either not set or empty, `$HOME/.config/git/ignore.`, and customised using
    /// `core.excludesfile` in global git configuration).
    ///
    /// This is the third of three methods of ignoring files in git.
    ///
    /// This follows the precedence rules defined in the [git documentation](https://git-scm.com/docs/gitignore#_description),
    /// and the git config rules defined in the [git documentation](https://git-scm.com/docs/git-config#FILES).
    ///
    /// This method returns true or false, which denotes whether the file is ignored or not, only if the path was
    /// matched in the global git ignore file. If not, [`Option::None`] will be returned, denoting that the path was not listed.
    fn evaluate_global_git_excludes_file(
        &self,
        git_root: Option<&impl AsRef<Path>>,
        path: impl AsRef<Path>,
    ) -> Option<bool> {
        let exclude_file = utils::get_global_git_exclude_file_path()?;

        let gitignore_file = match self.get_or_parse_gitignore(git_root, &exclude_file) {
            Ok(file) => file,
            Err(e) => {
                log::error!(
                    "Failed to read global .gitignore file at {}: {:?}",
                    exclude_file.display(),
                    e
                );

                None
            }
        };

        if let Some(gitignore_file) = gitignore_file {
            if let Some(is_ignored) = gitignore_file.is_ignored(&path) {
                log::debug!(
                    "{} is ignored by {}: {is_ignored}",
                    path.as_ref().display(),
                    exclude_file.as_path().display()
                );

                return Some(is_ignored);
            }
        }

        None
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
                        .is_none_or(|previous_match| git_root.as_path().starts_with(previous_match))
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
    ///
    /// Optionally, provide a base path to override the path to which all glob patterns defined inside the file
    /// should be relative to.
    ///
    /// When no base path is provided, the base path is assumed to be relative to the file being read. This is fine for
    /// regular `.gitignore` files, however, when dealing with both global exclude files, and git root exclude files, the
    /// base path provided will be the closest git root, not the file itself.
    fn get_or_parse_gitignore(
        &self,
        base_path: Option<&impl AsRef<Path>>,
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
                    base_path.as_ref(),
                    potential_gitignore.as_ref(),
                )?)))
            }
            Entry::Vacant(e) => {
                let gitignore_file = Arc::new(utils::read_gitignore(
                    base_path.as_ref(),
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

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        str::FromStr,
        sync::{Mutex, RwLock},
    };

    use rstest::rstest;

    use crate::evaluator::Evaluator;

    #[rstest]
    #[case(
        vec![],
        "some/path/",
        None
    )]
    #[case(
        vec![
            PathBuf::from_str("some/path/").unwrap()
        ],
        "some/path/here/1",
        Some(PathBuf::from_str("some/path/").unwrap())
    )]
    #[case(
        vec![
            PathBuf::from_str("some/path/here/").unwrap(),
            PathBuf::from_str("some/path/").unwrap()
        ],
        "some/path/here/1",
        Some(PathBuf::from_str("some/path/here/").unwrap())
    )]
    #[case(
        vec![
            PathBuf::from_str("some/path/").unwrap(),
            PathBuf::from_str("some/path/here/").unwrap()
        ],
        "some/path/here/1",
        Some(PathBuf::from_str("some/path/here/").unwrap())
    )]
    #[case(
        vec![
            PathBuf::from_str("some/path/").unwrap(),
            PathBuf::from_str("some/path/here/").unwrap()
        ],
        "different/parent/some/path/here/1",
        None
    )]
    #[case(
        vec![
            PathBuf::from_str("some/path/").unwrap(),
            PathBuf::from_str("some/path/here/").unwrap()
        ],
        "unrelated/path",
        None
    )]
    #[case(
        vec![
            PathBuf::from_str("some/path/").unwrap(),
            PathBuf::from_str("some/path/here/").unwrap()
        ],
        "some/",
        None
    )]
    pub fn test_get_closest_already_encountered_git_root(
        #[case] git_roots: Vec<PathBuf>,
        #[case] path: &str,
        #[case] expected_git_root: Option<PathBuf>,
    ) {
        let evaluator = Evaluator {
            git_roots: RwLock::new(git_roots),
            files: Mutex::default(),
        };

        assert_eq!(
            evaluator.get_closest_already_encountered_git_root(path),
            expected_git_root
        );
    }
}
