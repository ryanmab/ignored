use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    sync::OnceLock,
};

use regex::Regex;

use crate::{
    evaluator::{self, File, Glob, Result},
    utils,
};

/// Read a `.gitignore` file at the given path and parse it into a [`crate::evaluator::File`] struct.
///
/// A file contains a checksum (a unique value which can be used to safely identify if the file has changed
/// since last being read) which is useful for caching. It contains a base path, which is the path to which
/// all glob patterns defined inside are relative to (when matching arbitrary paths at runtime).
///
/// When no base path is provided during the reading process (like in the case of all `.gitignore` files), the
/// base path is assumed to be relative to the `.gitignore`. However, when dealing with both global exclude
/// files, and git root exclude files, the base path provided will be the closest git root, not the file itself.
pub fn read_gitignore(
    base_path: Option<impl AsRef<Path>>,
    gitignore_path: impl AsRef<Path>,
) -> Result<File> {
    let base_path = base_path.map_or_else(
        || {
            gitignore_path
                .as_ref()
                .parent()
                .map_or_else(PathBuf::new, Path::to_path_buf)
        },
        |base_path| base_path.as_ref().to_path_buf(),
    );

    let (checksum, file) = utils::compute_checksum(gitignore_path.as_ref()).map_err(|e| {
        evaluator::Error::FileError {
            file: gitignore_path.as_ref().to_path_buf(),
            source: e,
        }
    })?;

    let reader = BufReader::new(file);
    let mut content = Vec::<Glob>::new();

    for line in reader.lines() {
        let line = line.unwrap_or_default();

        let glob = Glob::try_from(line.as_str())?;

        if glob.is_empty() {
            continue;
        }

        content.push(glob);
    }

    Ok(File::new(base_path, content, checksum))
}

/// Get the global git exclude file path (defined either by default in `$XDG_CONFIG_HOME`/`$HOME`)
/// or explicitly set using `core.excludesfile` in the git config file.
pub fn get_global_git_exclude_file_path() -> Option<PathBuf> {
    // When the `XDG_CONFIG_HOME` environment variable is not set or empty,
    // `$HOME/.config/` is used as `$XDG_CONFIG_HOME` (handled by xdir).
    let config_path = xdir::config().map(|p| p.join("git").join("config"));

    log::debug!("Git config path defined as: {config_path:?}");

    if let Some(path) = config_path {
        if let Some(exclude_file) = get_exclude_file_config_from_gitconfig(path) {
            return Some(exclude_file);
        }
    }

    // Its default value is `$XDG_CONFIG_HOME/git/ignore`. If `$XDG_CONFIG_HOME` is either not
    // set or empty, `$HOME/.config/git/ignore` is used instead (handled by xdir).
    let default = xdir::config().map(|path| path.join("git").join("ignore"));

    log::debug!("No core.excludesfile set in gitconfig file. Using default path: {default:?}");

    default
}

/// Parse a global `gitconfig` file and extract the `excludesfile` config if present.
///
/// If the path is not present, or the `gitconfig` file could not be read, None will be
/// returned.
pub fn get_exclude_file_config_from_gitconfig(gitconfig: impl AsRef<Path>) -> Option<PathBuf> {
    static REGEX: OnceLock<Regex> = OnceLock::new();

    let gitconfig = gitconfig.as_ref();

    if !gitconfig.exists() {
        log::trace!("Gitconfig file not found at: {}", gitconfig.display());

        return None;
    }

    // TODO: Is it worth producing a checksum and storing the contents of the file (and the
    // resulting `core.excludesfile` path) in memory to prevent repeated accesses requiring contents
    // re-matching? The assumption built-in here is that this function will be called infrequently
    // (i.e. not in a loop and likely not for every evaluation), however that isn't guaranteed.
    let Ok(contents) = std::fs::read_to_string(gitconfig) else {
        log::warn!("Unable to read Gitconfig file at: {} ", gitconfig.display());

        return None;
    };

    let captures = REGEX
        .get_or_init(|| {
            regex::Regex::new("(?i)excludesfile\\s*=[\\s\"]*(?<path>[^\\s\"]+)")
                .expect("gitconfig exclude file regex should always be valid")
        })
        .captures_iter(&contents);

    let path = captures
        .last()
        .and_then(|captures| captures.name("path"))
        .map(|m| m.as_str())
        .map(PathBuf::from);

    log::trace!(
        "Gitconfig file at {} defines core.excludesfile as: {:?}",
        gitconfig.display(),
        path
    );

    path
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Stdio;
    use temp_env::with_vars;
    use tempfile::TempDir;

    /// Write a gitconfig file with arbitrary contents
    fn write_gitconfig(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test_log::test(rstest::rstest)]
    #[case(
        Some("[core]\n\texcludesfile = /home/excludes\n"),
        None,
        vec![
            ("HOME", true),
            ("USERPROFILE", true),
            ("XDG_CONFIG_HOME", false),
        ],
        Some(PathBuf::from("/home/excludes"))
    )]
    #[case(
        None,
        Some("[core]\n\texcludesfile = /xdg/excludes\n"),
        vec![
            ("HOME", true),
            ("USERPROFILE", true),
            ("XDG_CONFIG_HOME", true),
        ],
        Some(PathBuf::from("/xdg/excludes"))
    )]
    #[case(
        Some("[core]\n\texcludesfile = /home/excludes\n"),
        Some("[core]\n\texcludesfile = /xdg/excludes\n"),
        vec![
            ("HOME", true),
            ("USERPROFILE", true),
            ("XDG_CONFIG_HOME", true),
        ],
        Some(PathBuf::from("/xdg/excludes"))
    )]
    #[case(
        Some("[core]\n\texcludesfile = /home/excludes\n"),
        Some("[core]\n\texcludesfile = /xdg/excludes\n"),
        vec![
            ("HOME", true),
            ("XDG_CONFIG_HOME", true),
            ("USERPROFILE", false),
        ],
        Some(PathBuf::from("/xdg/excludes")) // XDG takes precedence
    )]
    #[case(
        Some("[CORE]\n# comment\n excludesfile   =   \"/home/excludes\"  \n"),
        Some("[core]\n\texcludesfile=/xdg/excludes\n"),
        vec![
            ("HOME", true),
            ("USERPROFILE", true),
            ("XDG_CONFIG_HOME", true),
        ],
        Some(PathBuf::from("/xdg/excludes")) // XDG still takes precedence
    )]
    #[case(
        None,
        Some("[core]\n  excludesfile   =   /xdg/excludes   \n"),
        vec![
            ("HOME", true),
            ("USERPROFILE", true),
            ("XDG_CONFIG_HOME", true),
        ],
        Some(PathBuf::from("/xdg/excludes"))
    )]
    #[case(
        Some("[core]\nexcludesfile = /first/path\n[core]\nexcludesfile = \"/second/path\"\n"),
        None,
        vec![
            ("HOME", true),
            ("USERPROFILE", true),
            ("XDG_CONFIG_HOME", false),
        ],
        Some(PathBuf::from("/second/path"))
    )]
    #[case(
        Some("[core]\n\texcludesfile\t=\t/path/with/spaces\n"),
        None,
        vec![
            ("HOME", true),
            ("USERPROFILE", true),
            ("XDG_CONFIG_HOME", false),
        ],
        Some(PathBuf::from("/path/with/spaces"))
    )]
    fn test_parsing_excludes_file_from_config(
        #[case] home_contents: Option<&str>,
        #[case] xdg_contents: Option<&str>,
        #[case] env_keys: Vec<(&str, bool)>,
        #[case] expected_exclude: Option<PathBuf>,
    ) {
        let temp_home = TempDir::new().unwrap();
        let temp_xdg = TempDir::new().unwrap();

        let env_vars: Vec<(&str, Option<PathBuf>)> = env_keys
            .into_iter()
            .map(|(key, set)| {
                let path = if set {
                    match key {
                        "HOME" | "USERPROFILE" => Some(temp_home.path().to_path_buf()),
                        "XDG_CONFIG_HOME" => Some(temp_xdg.path().to_path_buf()),
                        _ => None,
                    }
                } else {
                    None
                };
                (key, path)
            })
            .collect();

        if let Some(contents) = home_contents {
            let home_gitconfig = temp_home.path().join(".config").join("git").join("config");
            write_gitconfig(&home_gitconfig, contents);
        }

        if let Some(contents) = xdg_contents {
            let xdg_gitconfig = temp_xdg.path().join("git").join("config");
            write_gitconfig(&xdg_gitconfig, contents);
        }

        let env_vec: Vec<(&str, Option<&Path>)> = env_vars
            .iter()
            .map(|(key, opt_path)| (*key, opt_path.as_ref().map(PathBuf::as_path)))
            .collect();

        with_vars(env_vec, || {
            let path = super::get_global_git_exclude_file_path();

            assert_eq!(
                path, expected_exclude,
                "{path:?} does not match expected: {expected_exclude:?}"
            );

            let output = std::process::Command::new("git")
                .arg("config")
                .arg("--get")
                .arg("core.excludesfile")
                .stdout(Stdio::piped())
                .output()
                .expect("failed to run git");

            let git_returned_path = String::from_utf8(output.stdout).ok().and_then(|stdout| {
                if stdout.is_empty() {
                    return None;
                }

                Some(PathBuf::from(stdout.trim_end()))
            });

            assert_eq!(
                path, git_returned_path,
                "{path:?} does not match git CLI path: {git_returned_path:?}"
            );
        });
    }

    #[test_log::test(rstest::rstest)]
    #[case(
        None,
        None,
        vec![
            ("HOME", false),
            ("USERPROFILE", false),
            ("XDG_CONFIG_HOME", false),
        ],
    )]
    #[case(
        None,
        None,
        vec![
            ("HOME", true),
            ("USERPROFILE", true),
            ("XDG_CONFIG_HOME", false),
        ],
    )]
    #[case(
        None,
        None,
        vec![
            ("HOME", true),
            ("USERPROFILE", true),
            ("XDG_CONFIG_HOME", true),
        ],
    )]
    #[case(
        Some("[CORE]\n# comment\n"),
        None,
        vec![
            ("HOME", true),
            ("USERPROFILE", true),
            ("XDG_CONFIG_HOME", false),
        ],
    )]
    #[case(
        None,
        Some("[CORE]\n# comment\n"),
        vec![
            ("HOME", true),
            ("USERPROFILE", true),
            ("XDG_CONFIG_HOME", true),
        ],
    )]
    #[case(
        Some("[CORE]\n# comment\n"),
        Some("[CORE]\n# comment\n"),
        vec![
            ("HOME", true),
            ("USERPROFILE", true),
            ("XDG_CONFIG_HOME", true),
        ],
    )]
    #[case(
        None,
        None,
        vec![
            ("HOME", false),
            ("USERPROFILE", false),
            ("XDG_CONFIG_HOME", true),
        ],
    )]
    fn test_handles_defaults_when_excludes_file_is_not_set_in_config(
        #[case] home_contents: Option<&str>,
        #[case] xdg_contents: Option<&str>,
        #[case] env_keys: Vec<(&str, bool)>,
    ) {
        let temp_home = TempDir::new().unwrap();
        let temp_xdg = TempDir::new().unwrap();

        let env_vars: Vec<(&str, Option<PathBuf>)> = env_keys
            .into_iter()
            .map(|(key, set)| {
                let path = if set {
                    match key {
                        "HOME" | "USERPROFILE" => Some(temp_home.path().to_path_buf()),
                        "XDG_CONFIG_HOME" => Some(temp_xdg.path().to_path_buf()),
                        _ => None,
                    }
                } else {
                    None
                };
                (key, path)
            })
            .collect();

        if let Some(contents) = home_contents {
            let home_gitconfig = temp_home.path().join(".config").join("git").join("config");
            write_gitconfig(&home_gitconfig, contents);
        }

        if let Some(contents) = xdg_contents {
            let xdg_gitconfig = temp_xdg.path().join("git").join("config");
            write_gitconfig(&xdg_gitconfig, contents);
        }

        let env_vec: Vec<(&str, Option<&Path>)> = env_vars
            .iter()
            .map(|(key, opt_path)| (*key, opt_path.as_ref().map(PathBuf::as_path)))
            .collect();

        with_vars(env_vec, || {
            let path = super::get_global_git_exclude_file_path();

            assert!(
                path.as_ref().is_some_and(|s| s.ends_with("git/ignore")),
                "{path:?} is not the default excludes file path (ending in .config/git/ignore)"
            );

            let output = std::process::Command::new("git")
                .arg("config")
                .arg("--get")
                .arg("core.excludesfile")
                .stdout(Stdio::piped())
                .output()
                .expect("failed to run git");

            let git_returned_path = String::from_utf8(output.stdout)
                .ok()
                .map(|s| s.trim_end().is_empty())
                .or(Some(true));

            assert_eq!(Some(true), git_returned_path);
        });
    }
}
