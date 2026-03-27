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
