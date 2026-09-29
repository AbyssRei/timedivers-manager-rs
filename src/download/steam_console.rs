use anyhow::Context as _;
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

/// First steamapps folder that has `content/app_{app_id}` (original
/// `_find_content_folder`).
pub fn find_content_folder(steamapps_folders: &[String], app_id: &str) -> Option<PathBuf> {
    for folder in steamapps_folders {
        let candidate = Path::new(folder)
            .join("content")
            .join(format!("app_{app_id}"));
        if candidate.is_dir() {
            return Some(Path::new(folder).join("content"));
        }
    }
    None
}

pub fn depot_cache_dir(content_folder: &Path, app_id: &str, depot_id: &str) -> PathBuf {
    content_folder
        .join(format!("app_{app_id}"))
        .join(format!("depot_{depot_id}"))
}

/// `download_depot {app} {depot} {manifest}` — the Steam console command.
pub fn download_command(app_id: &str, depot_id: &str, manifest_id: &str) -> String {
    format!("download_depot {app_id} {depot_id} {manifest_id}")
}

/// Moves a single file, renaming when possible and falling back to
/// copy + delete across filesystems (`shutil.move` semantics).
fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(from, to)?;
            std::fs::remove_file(from)
        }
    }
}

fn count_files(dir: &Path) -> usize {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count()
}

/// Moves every file from the depot cache folders into `dest_folder`,
/// reporting `(done, total)` per file — the original import with its
/// file-count progress bar.
pub fn import_version(
    content_folder: &Path,
    app_id: &str,
    depots: &[String],
    dest_folder: &Path,
    progress: impl Fn(usize, usize) + Sync,
) -> anyhow::Result<()> {
    let depot_dirs: Vec<PathBuf> = depots
        .iter()
        .map(|d| depot_cache_dir(content_folder, app_id, d))
        .filter(|p| p.is_dir())
        .collect();

    let total: usize = depot_dirs.iter().map(|d| count_files(d)).sum();
    let total = total.max(1);
    let done = std::sync::atomic::AtomicUsize::new(0);
    progress(0, total);

    std::fs::create_dir_all(dest_folder)?;
    for depot_dir in &depot_dirs {
        for entry in walkdir::WalkDir::new(depot_dir)
            .sort_by_file_name()
            .into_iter()
            .filter_map(Result::ok)
        {
            let rel = entry.path().strip_prefix(depot_dir)?;
            let target_dir = dest_folder.join(rel);
            if entry.file_type().is_dir() {
                std::fs::create_dir_all(&target_dir)?;
            } else {
                move_file(entry.path(), &target_dir)?;
                let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                progress(n, total);
            }
        }
    }
    Ok(())
}

/// Clears the depot cache folders after import; returns folders that could
/// not be removed (surfaced as the original's "Cleanup Warning").
pub fn clear_depot_folders(
    content_folder: &Path,
    app_id: &str,
    depots: &[String],
) -> Vec<(PathBuf, io::Error)> {
    let mut failures = Vec::new();
    for depot_id in depots {
        let dir = depot_cache_dir(content_folder, app_id, depot_id);
        if dir.is_dir()
            && let Err(err) = std::fs::remove_dir_all(&dir)
        {
            failures.push((dir, err));
        }
    }
    failures
}

/// Learned expectation of one depot: exact size and file count of the last
/// manifest imported, plus which manifest it was learned from.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DepotMetaEntry {
    pub bytes: u64,
    pub files: u64,
    #[serde(default)]
    pub manifest: Option<String>,
}

/// Per-depot learned expectations persisted next to the manifests so later
/// downloads of the same depot+manifest can show percentage progress and
/// switch to Complete deterministically.
pub type DepotMeta = BTreeMap<String, DepotMetaEntry>;

pub fn load_depot_meta(path: &Path) -> DepotMeta {
    let options =
        ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| options.from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_depot_meta(path: &Path, meta: &DepotMeta) -> anyhow::Result<()> {
    let options =
        ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, options.to_string(meta)?)
        .with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Learns the actual sizes of each depot cache folder (call right after a
/// finished import, before the folders are cleared) and merges them into the
/// depot metadata file.
/// Human-readable byte count (KB/MB/GB with one decimal).
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let value = bytes as f64;
    if value >= KB * KB * KB {
        format!("{:.1} GB", value / (KB * KB * KB))
    } else if value >= KB * KB {
        format!("{:.1} MB", value / (KB * KB))
    } else if value >= KB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_content_folder_by_app() {
        let tmp = std::env::temp_dir().join(format!("tdm-content-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let lib = tmp.join("lib1");
        let app_dir = lib.join("content").join("app_553850");
        std::fs::create_dir_all(&app_dir).unwrap();
        let folders = vec![
            tmp.join("missing").to_string_lossy().into(),
            lib.to_string_lossy().into(),
        ];
        let found = find_content_folder(&folders, "553850").unwrap();
        assert_eq!(found, lib.join("content"));

        assert!(!depot_cache_dir(&found, "553850", "553851").exists());
        std::fs::create_dir_all(depot_cache_dir(&found, "553850", "553851")).unwrap();
        assert!(depot_cache_dir(&found, "553850", "553851").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn builds_download_commands() {
        assert_eq!(
            download_command("553850", "553851", "7654321"),
            "download_depot 553850 553851 7654321"
        );
    }

    #[test]
    fn import_moves_files_and_reports_progress() {
        let tmp = std::env::temp_dir().join(format!("tdm-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let content = tmp.join("content");
        let depot = depot_cache_dir(&content, "10", "11");
        std::fs::create_dir_all(depot.join("sub")).unwrap();
        std::fs::write(depot.join("a.txt"), b"a").unwrap();
        std::fs::write(depot.join("sub").join("b.txt"), b"b").unwrap();

        let dest = tmp.join("dest");
        let seen = std::sync::Mutex::new(Vec::new());
        import_version(&content, "10", &["11".into()], &dest, |done, total| {
            seen.lock().unwrap().push((done, total));
        })
        .unwrap();

        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"a");
        assert_eq!(std::fs::read(dest.join("sub").join("b.txt")).unwrap(), b"b");
        // The depot cache files were moved away (folders may remain).
        assert!(!depot.join("a.txt").exists());

        let seen = seen.lock().unwrap();
        assert_eq!(*seen.last().unwrap(), (2, 2));
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
