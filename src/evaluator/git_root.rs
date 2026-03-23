use std::{
    path::{Path, PathBuf},
    sync::RwLock,
};

use crate::constant;

#[derive(Debug, Default)]
pub struct RootHandler {
    /// A list of previously encountered git roots (directories with a `.git` inside them).
    ///
    /// This is an optimisation which allows frequent evaluations in the same/similar directory
    /// trees to skip traversal of the common ancestor directories, by jumping straight to the
    /// commonest (already encountered) git root.
    git_roots: RwLock<Vec<PathBuf>>,
}

impl RootHandler {
    /// Record a given path as a git root, if the `.git` path is present.
    ///
    /// Returns true if the path is a valid git root. Otherwise false will be returned.
    pub fn record(&self, git_root: impl AsRef<Path>) -> bool {
        if !git_root.as_ref().join(constant::LOCAL_GIT_PATH).exists() {
            // There's no git root path here, so we shouldn't record it.
            return false;
        }

        match self.git_roots.write() {
            Ok(mut guard) => {
                let git_root = git_root.as_ref().to_path_buf();

                if guard.contains(&git_root) {
                    // This is a previously encountered root, no need to record it again.
                    return true;
                }

                guard.push(git_root);
            }
            Err(e) => {
                log::error!(
                    "Unable to update git roots with newly encountered git root ({}): {}",
                    git_root.as_ref().display(),
                    e
                );
            }
        }

        true
    }

    /// Get the closest known git root
    ///
    /// This _only_ looks for already encountered git roots, and even when one is returned,
    /// doesn't guarantee another git root further down the directory tree won't be encountered
    /// (i.e. a `.git` where there is a `.git` in a parent directory).
    pub fn get_closest(&self, path: impl AsRef<Path>) -> Option<PathBuf> {
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

    /// Get the path to the local (git repository scoped - `.git/info/exclude`) exclude path.
    ///
    /// This path contains a regular `.gitignore` formatted file with patterns for files to
    /// ignore in the repository.
    pub fn get_exclude_path(&self, git_root: impl AsRef<Path>) -> Option<PathBuf> {
        let git_root = git_root.as_ref().to_path_buf();

        if !self.git_roots.read().ok()?.contains(&git_root) {
            log::warn!(
                "Local exclude path was requested for git root which has not been recorded: {}",
                git_root.display()
            );

            return None;
        }

        let exclude_file = git_root.join(constant::LOCAL_GIT_EXCLUDE_PATH);

        if !exclude_file.exists() {
            return None;
        }

        Some(exclude_file)
    }
}

#[cfg(test)]
mod tests {
    use crate::evaluator::git_root::RootHandler;
    use std::{path::PathBuf, str::FromStr, sync::RwLock};

    use rstest::rstest;

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
        let handler = RootHandler {
            git_roots: RwLock::new(git_roots),
        };

        assert_eq!(handler.get_closest(path), expected_git_root);
    }
}
