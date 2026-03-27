use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
};

use sha2::{Digest, Sha256};

use crate::{
    constant,
    evaluator::{self, git_config::ConfigFile},
};

/// Parse a global git config file and extract the `excludesfile` config if present.
///
/// If the path is not present, or the git config file could not be read, [`Option::None`] will be
/// returned.
pub fn read_git_config(
    path: impl AsRef<Path>,
    existing_file: Option<Arc<ConfigFile>>,
) -> Result<Option<Arc<ConfigFile>>, evaluator::Error> {
    let config_path = path.as_ref();

    if !config_path.exists() {
        log::trace!("Config file not found at: {}", config_path.display());

        return Ok(None);
    }

    let mut file = File::open(config_path).map_err(|e| evaluator::Error::FileError {
        file: config_path.to_path_buf(),
        source: e,
    })?;

    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| evaluator::Error::FileError {
        file: config_path.to_path_buf(),
        source: e,
    })?;

    let target_checksum = hasher.finalize();

    if existing_file
        .as_ref()
        .is_some_and(|existing_file| existing_file.checksum == target_checksum.as_slice())
    {
        log::trace!(
            "Already parsed git config file: {}",
            path.as_ref().display()
        );

        return Ok(existing_file);
    }

    file.seek(SeekFrom::Start(0))
        .map_err(|e| evaluator::Error::FileError {
            file: config_path.to_path_buf(),
            source: e,
        })?;

    let mut contents = String::new();

    if let Err(e) = file.read_to_string(&mut contents) {
        return Err(evaluator::Error::FileError {
            file: config_path.to_path_buf(),
            source: e,
        });
    }

    let path = constant::GLOBAL_GIT_CONFIG_EXCLUDE_PATH_REGEX
        .captures_iter(&contents)
        .last()
        .and_then(|captures| captures.name("path"))
        .map(|m| m.as_str())
        .map(PathBuf::from);

    log::trace!(
        "Config file at {} defines core.excludesfile as: {:?}",
        config_path.display(),
        path
    );

    Ok(Some(Arc::new(ConfigFile {
        path: config_path.to_path_buf(),
        exclude_file_path: path,
        checksum: target_checksum.to_vec(),
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::Path;
    use std::sync::Arc;
    use temp_env::with_vars;
    use tempfile::tempdir;

    use proptest::prelude::*;
    use std::path::PathBuf;

    fn write_file(path: &Path, contents: &str) {
        let mut file = File::create(path).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
    }

    #[test]
    fn returns_none_if_file_does_not_exist() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("nonexistent");

        let result = read_git_config(&path, None).unwrap();

        assert!(result.is_none());
    }

    #[rstest]
    #[case("[core]\n\texcludesfile = {path}\n", true)]
    #[case("[core]\n", false)]
    #[case("", false)]
    fn parses_excludesfile_correctly(#[case] template: &str, #[case] has_path: bool) {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config");
        let exclude_path = dir.path().join("ignore");

        let contents = template.replace("{path}", &exclude_path.to_string_lossy());
        write_file(&config_path, &contents);

        let result = read_git_config(&config_path, None).unwrap().unwrap();

        assert_eq!(result.exclude_file_path.is_some(), has_path);

        if has_path {
            assert_eq!(
                result.exclude_file_path.as_deref(),
                Some(exclude_path.as_path())
            );
        }
    }

    #[test]
    fn last_match_wins() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config");

        let first = dir.path().join("first");
        let second = dir.path().join("second");

        let contents = format!(
            "
            [core]
                excludesfile = {}
            [core]
                excludesfile = {}
            ",
            first.display(),
            second.display()
        );

        write_file(&config_path, &contents);

        let result = read_git_config(&config_path, None).unwrap().unwrap();

        assert_eq!(result.exclude_file_path.as_deref(), Some(second.as_path()));
    }

    #[test]
    fn reuses_existing_file_if_checksum_matches() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config");

        let exclude = dir.path().join("ignore");

        write_file(
            &config_path,
            &format!("[core]\n\texcludesfile = {}\n", exclude.display()),
        );

        let first = read_git_config(&config_path, None).unwrap().unwrap();
        let second = read_git_config(&config_path, Some(first.clone()))
            .unwrap()
            .unwrap();

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn reparses_if_checksum_differs() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config");

        let foo = dir.path().join("foo");
        let bar = dir.path().join("bar");

        write_file(
            &config_path,
            &format!("[core]\n\texcludesfile = {}\n", foo.display()),
        );

        let first = read_git_config(&config_path, None).unwrap().unwrap();

        write_file(
            &config_path,
            &format!("[core]\n\texcludesfile = {}\n", bar.display()),
        );

        let second = read_git_config(&config_path, Some(first.clone()))
            .unwrap()
            .unwrap();

        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(second.exclude_file_path.as_deref(), Some(bar.as_path()));
    }

    #[test]
    fn returns_error_if_path_is_not_a_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("not_a_file");

        // Create directory instead of file
        fs::create_dir(&path).unwrap();

        let result = read_git_config(&path, None);

        assert!(result.is_err());
    }

    #[test]
    fn works_with_temp_env_vars() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config");
        let exclude = dir.path().join("ignore");

        write_file(
            &config_path,
            &format!("[core]\n\texcludesfile = {}\n", exclude.display()),
        );

        with_vars(
            vec![
                ("HOME", Some("fake_home")),
                ("XDG_CONFIG_HOME", Some("fake_xdg")),
            ],
            || {
                let result = read_git_config(&config_path, None).unwrap().unwrap();
                assert!(result.exclude_file_path.is_some());
            },
        );
    }

    #[rstest]
    #[case("excludesfile={path}")]
    #[case("excludesfile = {path}")]
    #[case("excludesfile=    {path}")]
    #[case("   excludesfile = {path}")]
    fn tolerant_to_whitespace(#[case] template: &str) {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config");
        let exclude = dir.path().join("ignore");

        let line = template.replace("{path}", &exclude.to_string_lossy());
        let contents = format!("[core]\n{line}\n");

        write_file(&config_path, &contents);

        let result = read_git_config(&config_path, None).unwrap().unwrap();

        assert_eq!(result.exclude_file_path.as_deref(), Some(exclude.as_path()));
    }

    fn gitconfig_strategy() -> impl Strategy<Value = (String, Option<PathBuf>)> {
        let path_strategy = "[a-zA-Z0-9._/-]{1,50}".prop_map(PathBuf::from);

        // Whether we include an excludesfile entry
        let include = any::<bool>();

        (include, path_strategy).prop_map(|(include, path)| {
            if include {
                let content = format!(
                    "
                    [core]
                        excludesfile = {}
                    ",
                    path.display()
                );
                (content, Some(path))
            } else {
                let content = "
                    [user]
                        name = test
                "
                .to_string();
                (content, None)
            }
        })
    }

    proptest! {
        #[test]
        fn fuzz_parsing_does_not_panic((contents, expected_path) in gitconfig_strategy()) {
            let dir = tempdir().unwrap();
            let config_path = dir.path().join("config");

            write_file(&config_path, &contents);

            let result = read_git_config(&config_path, None);

            // Should never panic or error for valid UTF-8 input
            prop_assert!(result.is_ok());

            let parsed = result.unwrap().unwrap();

            match expected_path {
                Some(ref expected) => {
                    prop_assert_eq!(
                        parsed.exclude_file_path.as_deref(),
                        Some(expected.as_path())
                    );
                }
                None => {
                    prop_assert!(parsed.exclude_file_path.is_none());
                }
            }
        }
    }

    proptest! {
        #[test]
        fn fuzz_last_match_wins(paths in proptest::collection::vec("[a-zA-Z0-9._/-]{1,30}", 1..5)) {
            use std::fmt::Write;

            let dir = tempdir().unwrap();
            let config_path = dir.path().join("config");

            let mut contents = String::from("[core]\n");

            for p in &paths {
                writeln!(contents, "excludesfile = {p}").expect("Should always be able to write to string");
            }

            write_file(&config_path, &contents);

            let result = read_git_config(&config_path, None).unwrap().unwrap();

            let expected = PathBuf::from(paths.last().unwrap());

            prop_assert_eq!(
                result.exclude_file_path.as_deref(),
                Some(expected.as_path())
            );
        }
    }

    #[test]
    fn parses_typical_linux_gitconfig() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config");

        let contents = "
        [user]
            name = Jane Doe
            email = jane@example.com

        [core]
            excludesfile = ~/.config/git/ignore

        [init]
            defaultBranch = main
        ";

        write_file(&config_path, contents);

        let result = read_git_config(&config_path, None).unwrap().unwrap();

        assert_eq!(
            result.exclude_file_path.as_deref(),
            Some(Path::new("~/.config/git/ignore"))
        );
    }

    #[test]
    fn parses_typical_windows_gitconfig() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config");

        let contents = r"
        [core]
            excludesfile = C:\\Users\\test\\gitignore_global
        [credential]
            helper = manager
        ";

        write_file(&config_path, contents);

        let result = read_git_config(&config_path, None).unwrap().unwrap();

        assert_eq!(
            result.exclude_file_path.as_deref(),
            Some(Path::new(r"C:\\Users\\test\\gitignore_global"))
        );
    }

    #[test]
    fn parses_messy_realworld_config() {
        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config");

        // Real-world style: comments, spacing, duplicates, unrelated sections
        let contents = "
        # global config

        [user]
            email = foo@bar.com

        [core]
            excludesfile = /first/path

        # override later
        [core]
            excludesfile = /final/path

        [alias]
            co = checkout
        ";

        write_file(&config_path, contents);

        let result = read_git_config(&config_path, None).unwrap().unwrap();

        assert_eq!(
            result.exclude_file_path.as_deref(),
            Some(Path::new("/final/path"))
        );
    }

    #[test]
    fn handles_large_realistic_config() {
        use std::fmt::Write;

        let dir = tempdir().unwrap();
        let config_path = dir.path().join("config");

        let mut contents = String::new();

        // Simulate a large config file
        for i in 0..1000 {
            writeln!(contents, "[section{i}]\nkey{i} = value{i}")
                .expect("Should always be able to write to file");
        }

        contents.push_str("[core]\nexcludesfile = /large/test/path\n");

        write_file(&config_path, &contents);

        let result = read_git_config(&config_path, None).unwrap().unwrap();

        assert_eq!(
            result.exclude_file_path.as_deref(),
            Some(Path::new("/large/test/path"))
        );
    }
}
