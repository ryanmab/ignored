use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, RwLock},
};

use crate::{constant, evaluator::utils};

#[derive(Debug)]
pub struct ConfigFile {
    /// The path to the config file.
    ///
    /// In priority order, this path will be either:
    ///
    /// 1. `$XDG_CONFIG_HOME/git/config`
    /// 2. `$HOME/.config/git/config`
    /// 3. `$HOME/.gitconfig`
    #[allow(dead_code)]
    pub path: PathBuf,

    /// The path to the exclude file defined in the config
    /// file (if present).
    ///
    /// For example:
    ///
    /// ```toml
    /// [core]
    /// excludesfile = "some/path"
    /// ```
    pub exclude_file_path: Option<PathBuf>,

    /// The checksum of the file content as it was when the [`ConfigFile::exclude_file_path`] path was parsed, used
    /// for caching purposes.
    pub checksum: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct ConfigHandler {
    git_config_path: RwLock<HashMap<PathBuf, Arc<ConfigFile>>>,
}

impl ConfigHandler {
    /// Get the global git exclude file path (defined either by default in `$XDG_CONFIG_HOME`/`$HOME`)
    /// or explicitly set using `core.excludesfile` in the git config file.
    pub fn get_global_git_exclude_file_path(&self) -> Option<PathBuf> {
        for config_path in [
            // When the `XDG_CONFIG_HOME` environment variable is not set or empty,
            // `$HOME/.config/` is used as `$XDG_CONFIG_HOME` (handled by xdir).
            xdir::config().map(|p| p.join(constant::GLOBAL_GIT_CONFIG_PATH)),
            // Legacy `.gitconfig` files are stored in $HOME (`~/.gitconfig`).
            xdir::home().map(|p| p.join(constant::LEGACY_GLOBAL_GIT_CONFIG_PATH)),
        ] {
            log::debug!("Attempting read of git config file potentially in: {config_path:?}");

            if let Some(path) = config_path.as_ref() {
                let guard = self.git_config_path.read().ok()?;

                if let Ok(Some(git_config_file)) =
                    utils::read_git_config(path, guard.get(path).map(Arc::clone))
                {
                    drop(guard);

                    let exclude_file_path = git_config_file
                        .exclude_file_path
                        .as_ref()
                        .map(std::borrow::ToOwned::to_owned);

                    self.git_config_path
                        .write()
                        .ok()?
                        .insert(path.clone(), git_config_file);

                    if exclude_file_path.is_some() {
                        // We've found an exclude file in the config, we can return here and
                        // avoid any further work.
                        log::debug!(
                            "Git config file set core.excludesfile as: {exclude_file_path:?}"
                        );

                        return exclude_file_path;
                    }
                }
            }
        }

        // If `$XDG_CONFIG_HOME` is either not set or empty, `$HOME/.config/git/ignore` is
        // used instead (handled by xdir).
        let default = xdir::config().map(|p| p.join(constant::DEFAULT_GLOBAL_GIT_EXCLUDE_PATH));

        log::debug!(
            "No valid core.excludesfile config found in any git config file. Using default path: {default:?}"
        );

        default
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::process::Stdio;
    use std::{path::Path, path::PathBuf};
    use temp_env::with_vars;
    use tempfile::TempDir;

    use crate::constant;

    use crate::evaluator::git_config::ConfigHandler;

    /// Write a config file with arbitrary contents
    fn write_git_config(path: &Path, contents: &str) {
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
            write_git_config(
                &temp_home
                    .path()
                    .join(".config")
                    .join(constant::GLOBAL_GIT_CONFIG_PATH),
                contents,
            );
        }

        if let Some(contents) = xdg_contents {
            write_git_config(
                &temp_xdg.path().join(constant::GLOBAL_GIT_CONFIG_PATH),
                contents,
            );
        }

        let env_vec: Vec<(&str, Option<&Path>)> = env_vars
            .iter()
            .map(|(key, opt_path)| (*key, opt_path.as_ref().map(PathBuf::as_path)))
            .collect();

        with_vars(env_vec, || {
            let config_handler = ConfigHandler::default();
            let path = config_handler.get_global_git_exclude_file_path();

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
                "{path:?} does not match git cli path: {git_returned_path:?}"
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
            write_git_config(
                &temp_home
                    .path()
                    .join(".config")
                    .join(constant::GLOBAL_GIT_CONFIG_PATH),
                contents,
            );
        }

        if let Some(contents) = xdg_contents {
            write_git_config(
                &temp_xdg.path().join(constant::GLOBAL_GIT_CONFIG_PATH),
                contents,
            );
        }

        let env_vec: Vec<(&str, Option<&Path>)> = env_vars
            .iter()
            .map(|(key, opt_path)| (*key, opt_path.as_ref().map(PathBuf::as_path)))
            .collect();

        with_vars(env_vec, || {
            let config_handler = ConfigHandler::default();
            let path = config_handler.get_global_git_exclude_file_path();

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
