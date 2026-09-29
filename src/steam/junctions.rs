use std::io;
use std::path::Path;

/// True when `path` is an NTFS junction (reparse point), like the original
/// `is_junction` (FILE_ATTRIBUTE_REPARSE_POINT).
pub fn is_junction(path: &Path) -> bool {
    junction::exists(path).unwrap_or(false)
}

/// Creates a junction at `link` pointing to `target`.
pub fn create_junction(link: &Path, target: &Path) -> io::Result<()> {
    junction::create(target, link)
}

/// Removes a junction (only the link itself, never the target contents).
/// `RemoveDirectoryW` deletes the reparse-point entry, leaving the target
/// directory untouched.
pub fn remove_junction(link: &Path) -> io::Result<()> {
    std::fs::remove_dir(link)
}

/// Renames a directory, mirroring `os.rename` in the original setup flow.
pub fn rename_dir(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::rename(from, to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junction_lifecycle() -> io::Result<()> {
        let base = std::env::temp_dir().join(format!("tdm-junction-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let target = base.join("target");
        let link = base.join("link");
        std::fs::create_dir_all(&target)?;
        std::fs::write(target.join("file.txt"), b"hello")?;

        assert!(!is_junction(&link));
        create_junction(&link, &target)?;
        assert!(is_junction(&link));
        assert!(link.join("file.txt").exists());
        assert_eq!(
            junction::get_target(&link).ok().map(|p| p
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()),
            Some("target".to_string())
        );

        remove_junction(&link)?;
        assert!(!link.exists());
        assert!(target.join("file.txt").exists());
        std::fs::remove_dir_all(&base)?;
        Ok(())
    }
}
