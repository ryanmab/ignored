use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    path::Path,
};

#[cfg(test)]
use proptest::prelude::*;

/// Computes the Sha256 checksum of the given file and returns it along with a `File` handle to the beginning
/// of the file.
pub fn compute_checksum(path: impl AsRef<Path>) -> io::Result<(Vec<u8>, File)> {
    let mut file = File::open(path.as_ref())?;

    let mut hasher = Sha256::new();

    // Incrementally read large files in 4KB chunks and update
    // the hash until we've reached the end.
    let mut buffer = [0; 4096]; // 4KB
    loop {
        let bytes_read = file.read(&mut buffer)?;

        if bytes_read == 0 {
            // We've reached the end of the file - the hash is now complete.
            break;
        }

        hasher.update(&buffer[..bytes_read]);
    }

    file.seek(SeekFrom::Start(0))?;

    Ok((hasher.finalize().to_vec(), file))
}

/// Checks if the given file is ignored by git in the specified repository path. This function is only used in
/// tests, and acts as a reference implementation to verify the correctness of the `is_ignored!` macro.
///
/// [`https://git-scm.com/docs/git-check-ignore`]
#[cfg(test)]
pub fn git_check_ignore(repo_path: impl AsRef<Path>, file: impl AsRef<Path>) -> bool {
    use std::process::Command;

    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path.as_ref())
        .arg("check-ignore")
        .arg(file.as_ref())
        .output()
        .expect("failed to run git");

    output.status.success()
}

/// Copy files from source to destination recursively.
#[cfg(test)]
pub fn copy_recursively(source: impl AsRef<Path>, destination: impl AsRef<Path>) -> io::Result<()> {
    use std::fs;

    fs::create_dir_all(&destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let filetype = entry.file_type()?;
        if filetype.is_dir() {
            copy_recursively(entry.path(), destination.as_ref().join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), destination.as_ref().join(entry.file_name()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
pub fn get_gitignore_pattern_fuzzing_strategy() -> impl Strategy<Value = String> {
    use proptest::strategy::Strategy;

    let literal = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._-";
    let literal_component = (1..50).prop_map(move |len| {
        (0..len)
            .map(|_| {
                let idx = fastrand::usize(..literal.len());
                literal.chars().nth(idx).unwrap()
            })
            .collect::<String>()
    });

    let component = prop_oneof![
        literal_component,
        Just("*".to_string()),
        Just("**".to_string()),
        Just("?".to_string()),
        Just("[abc]".to_string()),
        Just("[0-9]".to_string()),
        Just(r"\!".to_string()),
        Just(r"\#".to_string()),
        Just(r"\ ".to_string()),
        Just(r"\\".to_string())
    ];

    prop::collection::vec(component, 1..10).prop_map(|parts| {
        let mut s = parts.join("");

        match fastrand::usize(0..=6) {
            0 => s = format!("# {s}"),
            1 => s = format!("!{s}"),
            2 => s = format!("/{s}"),
            _ => {}
        }

        if fastrand::bool() {
            s.push('/');
        }

        s
    })
}
