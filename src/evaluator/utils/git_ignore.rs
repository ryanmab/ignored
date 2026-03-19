use std::{
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

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
