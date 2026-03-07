use std::sync::OnceLock;

use crate::evaluator::Evaluator;

#[doc(hidden)]
#[allow(dead_code)]
pub static EVALUATOR: OnceLock<Evaluator> = OnceLock::new();

/// Check if an arbitrary file or folder is ignored according to `.gitignore` rules in the
/// current repository.
///
/// # Caching
///
/// In order to reduce IO overhead and improve performance, calls to repeatedly assess to the same
/// `.gitignore` files are cached in memory. The first time a `.gitignore` file is encountered, it
/// is read from disk and parsed, and the resulting data structure is stored in a global cache.
///
/// Subsequent assessments of the `.gitignore` file will use the cached version. The cached file is
/// invalidated when the file is modified on disk.
///
/// If this mechanism is not desirable, the [`crate::evaluator::Evaluator`] can be used directly, which
/// provides more control over caching and other aspects of evaluation.
///
/// # Examples
///
/// ```rust
/// use std::path::Path;
/// use ignored::is_ignored;
///
/// # std::fs::create_dir("tests/fixtures/mock-project/.git");
/// let ignored = is_ignored!("tests/fixtures/mock-project/file.tmp");
///
/// assert!(ignored);
/// ```
#[macro_export]
macro_rules! is_ignored {
    ($path:expr) => {{
        $crate::macros::is_ignored::EVALUATOR
            .get_or_init($crate::evaluator::Evaluator::default)
            .is_ignored($path)
    }};
}

#[cfg(test)]
mod tests {
    use crate::utils;
    use std::{path::PathBuf, process::Command};

    #[test_log::test(rstest::rstest)]
    #[case(vec!["root.txt"])]
    #[case(vec!["src","root.txt"])]
    #[case(vec!["build","artifact.bin"])]
    #[case(vec!["src","build","nested.bin"])]
    #[case(vec!["tmp","keep.txt"])]
    #[case(vec!["src","lib.rs"])]
    #[case(vec!["src","main.rs"])]
    #[case(vec!["src","module","deep.log"])]
    #[case(vec!["src","module","deep_keep.log"])]
    #[case(vec!["src","module","a.tmp"])]
    #[case(vec!["src","module","b.tmp"])]
    #[case(vec!["src","module","file1.txt"])]
    #[case(vec!["src","module","fileA.txt"])]
    #[case(vec!["fileA.txt"])]
    #[case(vec!["file1.txt"])]
    #[case(vec!["file9.txt"])]
    #[case(vec!["fileZ.txt"])]
    #[case(vec!["filex.txt"])]
    #[case(vec!["literal","#hash.txt"])]
    #[case(vec!["literal","!bang.txt"])]
    #[case(vec!["literal","space file.txt"])]
    #[case(vec!["vendor","ignored.txt"])]
    #[case(vec!["vendor","keep.me"])]
    #[case(vec!["foo"])]
    #[case(vec!["foo "])]
    #[case(vec!["bar"])]
    #[case(vec!["bar "])]
    #[case(vec!["baz", "foo", "bar"])]
    #[case(vec!["build_dir"])]
    #[case(vec!["build_dir","file.txt"])]
    #[case(vec!["src","build","file.txt"])]
    #[case(vec!["other","src","build","file.txt"])]
    #[case(vec!["a","b"])]
    #[case(vec!["a","x","b"])]
    #[case(vec!["a","x","y","b"])]
    #[case(vec!["nested","build"])]
    #[case(vec!["nested","sub","build"])]
    #[case(vec!["file_vs_dir","same"])]
    #[case(vec!["file_vs_dir","same_dir","file.txt"])]
    #[case(vec!["double_negation","important.tmp"])]
    #[case(vec![".gitignore_should_be_ignored"])]
    #[case(vec!["emptydir"])]
    #[case(vec!["file1.dat"])]
    #[case(vec!["file12.dat"])]
    #[case(vec!["x","y","z","globfoo.txt"])]
    #[case(vec!["globdir","subdir","file.txt"])]
    #[case(vec!["a","b","c","globbar.txt"])]
    #[case(vec!["anchored.txt"])]
    #[case(vec!["src","anchored.txt"])]
    #[case(vec!["dironly","sub","file.txt"])]
    #[case(vec!["dironly"])]
    #[case(vec!["literal","file*.txt"])]
    #[case(vec!["literal","file?.txt"])]
    #[case(vec!["literal","file[abc].txt"])]
    #[case(vec!["literal","filea.txt"])]
    #[case(vec!["precedence.log"])]
    #[case(vec!["important.log"])]
    #[case(vec!["pruned","deep","keep.txt"])]
    #[case(vec!["deep","a","b","c","match.txt"])]
    #[case(vec!["deep","match.txt"])]
    #[case(vec!["prefix","a","b","suffix.log"])]
    #[case(vec!["prefix","suffix.log"])]
    #[case(vec!["anydepth","x","y","z","file.tmp"])]
    #[case(vec!["file.tmp"])]
    #[case(vec!["dirslash"])]
    #[case(vec!["dirslash","file.txt"])]
    #[case(vec!["dirslash_file"])]
    #[case(vec!["escaped",r"space\ "])]
    #[case(vec!["escaped","space "])]
    #[case(vec!["escaped","space"]) ]
    #[case(vec!["escaped","!literal.txt"])]
    #[case(vec!["escaped","#literal.txt"])]
    #[case(vec!["multi","slash","file.txt"])]
    #[case(vec!["reinclude","keep.txt"])]
    #[case(vec!["reinclude","other.txt"])]
    #[case(vec!["prune_dir","deep","file.txt"])]
    #[case(vec!["prune_dir","deep","important.txt"])]
    #[case(vec!["escaped","back\\slash.txt"])]
    #[case(vec!["escaped","back-slash.txt"])]
    #[case(vec!["escaped","back\\-slash.txt"])]
    #[case(vec!["charclass","filea.log"])]
    #[case(vec!["charclass","filed.log"])]
    #[case(vec!["qmark","file1.txt"])]
    #[case(vec!["qmark","file12.txt"])]
    #[case(vec!["range","file5.txt"])]
    #[case(vec!["range","filex.txt"])]
    #[case(vec!["anchored_dir","file.txt"])]
    #[case(vec!["sub","anchored_dir","file.txt"])]
    #[case(vec!["trailing_space","foo"])]
    #[case(vec!["trailing_space","foo "])]
    #[case(vec!["double_star_root","file.txt"])]
    #[case(vec!["dotfile_root",".hidden.txt"])]
    #[case(vec!["dotfile_dir",".hidden.txt"])]
    #[case(vec!["builder"])]
    #[case(vec!["build"])]
    #[case(vec!["unicode","fileé.txt"])]
    #[case(vec!["ignored_outside_git_root.txt"])]
    fn test_matches_git_check_ignore(#[case] path: Vec<&str>) {
        let temp = tempfile::tempdir().expect("Should be able to create a temporary directory");

        utils::copy_recursively("tests/fixtures/", temp.path())
            .expect("Should be able to copy the mock project");
        let repo_path = temp.path().join("mock-project");

        Command::new("git")
            .arg("init")
            .arg(repo_path.as_path())
            .output()
            .expect("Should be able to initialize a git repository");

        let relative_path = path.iter().collect::<PathBuf>();
        let file_path = repo_path
            .iter()
            .filter_map(|part| part.to_str())
            .chain(path.into_iter())
            .collect::<PathBuf>();

        let result = is_ignored!(file_path.as_path());
        let expected = crate::utils::git_check_ignore(repo_path, file_path.as_path());

        assert_eq!(
            result,
            expected,
            "Expected is_ignored! to match git check-ignore for {}. is_ignored! returned {result}, but git check-ignore returned {expected}",
            relative_path.display()
        );
    }

    #[test_log::test(test)]
    fn test_handles_negation() {
        let temp = tempfile::tempdir().expect("Should be able to create a temporary directory");

        utils::copy_recursively("tests/fixtures/", temp.path())
            .expect("Should be able to copy the mock project");
        let repo_path = temp.path().join("mock-project");

        Command::new("git")
            .arg("init")
            .arg(repo_path.as_path())
            .output()
            .expect("Should be able to initialize a git repository");

        let not_negated = PathBuf::from_iter(vec!["double_negation/important.tmp"]);
        let negated = "important.log";

        assert!(is_ignored!(repo_path.join(not_negated)));
        assert!(!is_ignored!(repo_path.join(negated)));
    }

    #[test_log::test(test)]
    fn test_observes_recursive_git_roots_when_ignoring() {
        let temp = tempfile::tempdir().expect("Should be able to create a temporary directory");

        utils::copy_recursively("tests/fixtures/", temp.path())
            .expect("Should be able to copy the mock project");
        let repo_path = temp.path().join("mock-project");

        // Initialise a parent git root (above `mock-project`). This means the `.gitignore`
        // which lists `ignored_outside_git_root.txt` _is_ inside a git root (and therefore)
        // applicable to the `mock-project` folder, if there isn't a child git root inside of
        // `mock-project`
        Command::new("git")
            .arg("init")
            .arg(temp.path())
            .output()
            .expect("Should be able to initialize parent git repository");

        utils::copy_recursively("tests/fixtures/mock-project", repo_path.as_path())
            .expect("Should be able to copy the mock project");

        Command::new("git")
            .arg("init")
            .arg(repo_path.as_path())
            .output()
            .expect("Should be able to initialize child git repository");

        // Shouldn't be ignored as even though the `.gitignore` which ignores this file is now
        // inside a git root too, there's still a child git root which resets the decision
        assert!(!is_ignored!(repo_path.join("ignored_outside_git_root.txt")));

        // Check this behavior also matches the git cli
        assert!(!crate::utils::git_check_ignore(
            repo_path.as_path(),
            repo_path.join("ignored_outside_git_root.txt").as_path()
        ));
    }
}
