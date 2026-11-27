use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use uuid::Uuid;

/// Create a private temporary file beside the destination so the final rename
/// stays on the same filesystem.
pub(crate) fn create_private_temp(path: &Path) -> Result<(File, PathBuf), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    if let Some(parent) = parent
        && !parent.is_dir()
    {
        return Err("The selected export folder does not exist.".into());
    }
    let temporary_name = format!(".koyo-export-{}.tmp", Uuid::new_v4());
    let temporary_path = path.with_file_name(temporary_name);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&temporary_path)
        .map_err(|error| format!("Could not create export file: {error}"))?;
    Ok((file, temporary_path))
}

/// Flush and atomically replace the destination. On failure, the caller keeps
/// the original destination intact.
pub(crate) fn finish_private_export(
    file: File,
    temporary_path: &Path,
    destination: &Path,
) -> Result<(), String> {
    file.sync_all()
        .map_err(|error| format!("Could not flush export file: {error}"))?;
    drop(file);
    fs::rename(temporary_path, destination)
        .map_err(|error| format!("Could not replace export file: {error}"))
}

pub(crate) fn discard_private_export(temporary_path: &Path) {
    let _ = fs::remove_file(temporary_path);
}

pub(crate) fn write_private_export(path: &Path, contents: &[u8]) -> Result<(), String> {
    let (mut file, temporary_path) = create_private_temp(path)?;
    let result = (|| {
        file.write_all(contents)
            .map_err(|error| format!("Could not write export file: {error}"))?;
        finish_private_export(file, &temporary_path, path)
    })();
    if result.is_err() {
        discard_private_export(&temporary_path);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{create_private_temp, discard_private_export, write_private_export};
    use std::fs;

    #[test]
    fn private_export_replaces_destination_and_is_owner_only() {
        let directory =
            std::env::temp_dir().join(format!("koyo-export-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&directory).expect("create temp dir");
        let path = directory.join("result.csv");
        fs::write(&path, "old").expect("seed destination");

        write_private_export(&path, b"new").expect("write export");
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        }

        let (_, temporary) = create_private_temp(&path).expect("create temp");
        discard_private_export(&temporary);
        assert!(!temporary.exists());
        let _ = fs::remove_dir_all(directory);
    }
}
