use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use crate::{
    evaluator::{self, File, Glob, Result},
    utils,
};

/// Read a `.gitignore` file at the given path and parse it into a [`crate::evaluator::File`] struct, which contains
/// the base path (the directory containing the `.gitignore` file), the content (a vector of `Glob` patterns),
/// and the checksum of the file content (used for caching purposes).
pub fn read_gitignore(gitignore_path: impl AsRef<Path>) -> Result<File> {
    let base_path = gitignore_path
        .as_ref()
        .parent()
        .unwrap_or_else(|| Path::new(""));

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
    let xdg_config_home_config = xdir::config().map(|p| p.join("git").join("config"));

    log::debug!("$XDG_CONFIG_HOME git config defined as: {xdg_config_home_config:?}");

    if let Some(path) = xdg_config_home_config {
        if let Some(exclude_file) = get_exclude_file_config_from_gitconfig(path) {
            return Some(exclude_file);
        }
    }

    // Its default value is `$XDG_CONFIG_HOME/git/ignore`. If `$XDG_CONFIG_HOME` is either not set or empty,
    // $HOME/.config/git/ignore is used instead (handled by xdir).
    let default = xdir::config().map(|path| path.join("git").join("ignore"));

    log::debug!("No core.excludesfile set in gitconfig file. Using default path: {default:?}",);

    default
}

/// Parse a global `gitconfig` file and extract the `excludesfile` config if present.
///
/// If the path is not present, or the `gitconfig` file could not be read, None will be
/// returned.
pub fn get_exclude_file_config_from_gitconfig(gitconfig: impl AsRef<Path>) -> Option<PathBuf> {
    let gitconfig = gitconfig.as_ref();

    if !gitconfig.exists() {
        log::trace!("Gitconfig file not found at: {}", gitconfig.display());

        return None;
    }

    let regex = regex::Regex::new("excludesfile\\s*=\\s*(?<path>[^\\s]+)")
        .expect("gitconfig exclude file regex should always be valid");

    let Ok(contents) = std::fs::read_to_string(gitconfig) else {
        log::warn!("Unable to read Gitconfig file at: {} ", gitconfig.display());

        return None;
    };

    let captures = regex.captures(&contents);

    let path = captures
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
    use std::path::PathBuf;
    use std::process::Stdio;
    use temp_env::with_vars;
    use tempfile::TempDir;

    /// Write a gitconfig file with arbitrary contents
    fn write_gitconfig(path: &PathBuf, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test_log::test(rstest::rstest)]
    #[case(
        Some("[core]\n\texcludesfile = /home/excludes\n"),
        None,
        vec![
            "HOME",
            "USERPROFILE"
        ],
        Some(PathBuf::from("/home/excludes"))
    )]
    #[case(
        None,
        Some("[core]\n\texcludesfile = /xdg/excludes\n"),
        vec![
            "HOME",
            "USERPROFILE",
            "XDG_CONFIG_HOME",
        ],
        Some(PathBuf::from("/xdg/excludes"))
    )]
    #[case(
        Some("[core]\n\texcludesfile = /home/excludes\n"),
        Some("[core]\n\texcludesfile = /xdg/excludes\n"),
        vec![
            "HOME",
            "USERPROFILE",
        ],
        Some(PathBuf::from("/home/excludes"))
    )]
    #[case(
        Some("[core]\n\texcludesfile = /home/excludes\n"),
        Some("[core]\n\texcludesfile = /xdg/excludes\n"),
        vec![
            "HOME",
            "XDG_CONFIG_HOME",
        ],
        Some(PathBuf::from("/xdg/excludes")) // XDG takes precedence over HOME
    )]
    #[case(
        Some("[CORE]\n# comment\n excludesfile   =   /home/excludes  \n"),
        Some("[core]\n\texcludesfile=/xdg/excludes\n"),
        vec![
            "HOME",
            "USERPROFILE",
            "XDG_CONFIG_HOME",
        ],
        Some(PathBuf::from("/xdg/excludes")) // XDG still takes precedence
    )]
    #[case(
        None,
        Some("[core]\n  excludesfile   =   /xdg/excludes   \n"),
        vec![
            "HOME",
            "USERPROFILE",
            "XDG_CONFIG_HOME",
        ],
        Some(PathBuf::from("/xdg/excludes"))
    )]
    fn test_exclude_file_paths(
        #[case] home_contents: Option<&str>,
        #[case] xdg_contents: Option<&str>,
        #[case] env_vars: Vec<&str>,
        #[case] expected_exclude: Option<PathBuf>,
    ) {
        let temp_home = TempDir::new().unwrap();
        let temp_xdg = TempDir::new().unwrap();

        let home_path = temp_home.path();
        let xdg_path = temp_xdg.path();

        if let Some(contents) = home_contents {
            let home_gitconfig = home_path.join(".config").join("git").join("config");
            write_gitconfig(&home_gitconfig, contents);
        }

        if let Some(contents) = xdg_contents {
            let xdg_gitconfig = xdg_path.join("git").join("config");
            write_gitconfig(&xdg_gitconfig, contents);
        }

        let vars = env_vars
            .iter()
            .map(|name| match *name {
                "HOME" | "USERPROFILE" => (*name, Some(home_path)),
                "XDG_CONFIG_HOME" => (*name, Some(xdg_path)),
                _ => unreachable!(),
            })
            .collect::<Vec<_>>();

        with_vars(vars, || {
            use std::process::Command;

            let path = super::get_global_git_exclude_file_path();

            assert_eq!(
                path, expected_exclude,
                "{path:?} does not match the patch expected in the data provider: {expected_exclude:?}"
            );

            // Run the git cli to see the _actual_ path returned by git (for parity)
            let output = Command::new("git")
                .arg("config")
                .arg("--get")
                .arg("core.excludesfile")
                .stdout(Stdio::piped())
                .output()
                .expect("failed to run git");

            let git_returned_path = String::from_utf8(output.stdout)
                .ok()
                .and_then(|stdout| {
                    if !stdout.is_empty() {
                        return Some(stdout.trim_end().to_string());
                    }

                    None
                })
                .map(PathBuf::from);

            assert_eq!(
                path, git_returned_path,
                "{path:?} does not match the path returned by the git cli: {git_returned_path:?}"
            );
        });
    }

    #[test_log::test(rstest::rstest)]
    #[case(
        None,
        None,
        vec![],
    )]
    #[case(
        None,
        None,
        vec![
            "HOME",
            "USERPROFILE"
        ],
    )]
    #[case(
        None,
        None,
        vec![
            "HOME",
            "USERPROFILE",
            "XDG_CONFIG_HOME",
        ],
    )]
    #[case(
        Some("[CORE]\n# comment\n"),
        None,
        vec![
            "HOME",
            "USERPROFILE",
        ],
    )]
    #[case(
        None,
        Some("[CORE]\n# comment\n"),
        vec![
            "HOME",
            "USERPROFILE",
        ],
    )]
    #[case(
        Some("[CORE]\n# comment\n"),
        Some("[CORE]\n# comment\n"),
        vec![
            "HOME",
            "USERPROFILE",
            "XDG_CONFIG_HOME",
        ],
    )]
    fn test_handles_defaults_when_excludes_file_is_not_set_in_config(
        #[case] home_contents: Option<&str>,
        #[case] xdg_contents: Option<&str>,
        #[case] env_vars: Vec<&str>,
    ) {
        let temp_home = TempDir::new().unwrap();
        let temp_xdg = TempDir::new().unwrap();

        let home_path = temp_home.path();
        let xdg_path = temp_xdg.path();

        if let Some(contents) = home_contents {
            let home_gitconfig = home_path.join(".config").join("git").join("config");
            write_gitconfig(&home_gitconfig, contents);
        }

        if let Some(contents) = xdg_contents {
            let xdg_gitconfig = xdg_path.join("git").join("config");
            write_gitconfig(&xdg_gitconfig, contents);
        }

        let vars = env_vars
            .iter()
            .map(|name| match *name {
                "HOME" | "USERPROFILE" => (*name, Some(home_path)),
                "XDG_CONFIG_HOME" => (*name, Some(xdg_path)),
                _ => unreachable!(),
            })
            .collect::<Vec<_>>();

        with_vars(vars, || {
            use std::process::Command;

            let path = super::get_global_git_exclude_file_path();

            assert!(
                path.as_ref().is_some_and(|s| s.ends_with("git/ignore")),
                "{path:?} is not the default excludes file path (ending in .config/git/ignore)"
            );

            // Run the git cli to see the that git doesn't identify an excludes file defined in
            // config (for parity)
            let output = Command::new("git")
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
