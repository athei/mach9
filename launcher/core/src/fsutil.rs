use std::{fs, io, path::Path};

/// Write through a temporary sibling and rename it into place, so a reader
/// never sees a half-written file.
///
/// # Errors
///
/// When the temporary file cannot be written or renamed over `path`; the
/// message names the file.
pub fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents)
        .map_err(|e| io::Error::new(e.kind(), format!("failed to write {}: {e}", tmp.display())))?;
    fs::rename(&tmp, path)
        .map_err(|e| io::Error::new(e.kind(), format!("failed to write {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_no_temporary_file_behind() {
        let dir = std::env::temp_dir().join(format!("gamelauncher-fsutil-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("game");
        write_atomic(&path, "MyGame\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "MyGame\n");
        assert!(!dir.join("game.tmp").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
