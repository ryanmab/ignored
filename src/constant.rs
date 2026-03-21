use std::sync::LazyLock;

use regex::Regex;

/// The path to the global git config file, relative to `$XDG_CONFIG_PATH` (or `$HOME/.config` when
/// not set).
pub static GLOBAL_GIT_CONFIG_PATH: &str = "git/config";

/// The path to the legacy global git config file relative to `$HOME`.
pub static LEGACY_GLOBAL_GIT_CONFIG_PATH: &str = ".gitconfig";

/// A regex to parse `core.excludesfile` from the global git config file ([`GLOBAL_GIT_CONFIG_PATH`],
/// [`LEGACY_GLOBAL_GIT_CONFIG_PATH`]).
///
/// Ideally this would be a full INI parser with support for all ways in which git config files can
/// be formatted, but this regular expression should cover _most_ of the common cases.
pub static GLOBAL_GIT_CONFIG_EXCLUDE_PATH_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    regex::Regex::new("(?i)excludesfile\\s*=[\\s\"]*(?<path>[^\\s\"]+)")
        .expect("Excludes file regex for config file should always be valid")
});

/// The default global git exclude path, when `core.excludesfile` is not defined
/// in any of the config files ([`GLOBAL_GIT_CONFIG_PATH`], [`LEGACY_GLOBAL_GIT_CONFIG_PATH`]).
pub static DEFAULT_GLOBAL_GIT_EXCLUDE_PATH: &str = "git/ignore";

/// The path to the local `.git` path relative to the repository root.
pub static LOCAL_GIT_PATH: &str = ".git";

/// The filename of the `.gitignore` files inside a repository root.
pub static GITIGNORE_FILE: &str = ".gitignore";

/// The path to the local git config file relative to the git repository
/// root.
pub static LOCAL_GIT_CONFIG_PATH: &str = ".git/info/exclude";
