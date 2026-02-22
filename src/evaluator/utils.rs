use std::{
    io::{BufRead, BufReader},
    path::Path,
};

use crate::{
    evaluator::{self, File, Glob, Result},
    utils,
};

/// Read a `.gitignore` file at the given path and parse it into a [`crate::evaluator::File`] struct, which contains
/// the base path (the directory containing the `.gitignore` file), the content (a vector of `Glob` patterns),
/// and the checksum of the file content (used for caching purposes).
pub fn read_gitignore(
    base_path: impl AsRef<Path>,
    gitignore_path: impl AsRef<Path>,
) -> Result<File> {
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
