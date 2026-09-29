use std::path::{Path, PathBuf};

use anyhow::Context as _;

use crate::constants::NONE_BRANCH;
use crate::manifest::ManifestEntry;

/// Builds the per-depot DepotDownloader command lines (one per depot with a
/// known manifest), mirroring `download_version` in the original.
pub fn build_batch_lines(
    app_id: &str,
    depots: &[String],
    branch: &str,
    entry: &ManifestEntry,
    username: &str,
    remember_password: bool,
    dest_folder: &Path,
) -> Vec<String> {
    let mut lines = Vec::new();
    for depot_id in depots {
        let Some(manifest_id) = entry.get(depot_id) else {
            continue;
        };
        let mut cmd =
            format!("DepotDownloader.exe -app {app_id} -depot {depot_id} -manifest {manifest_id}");
        if branch != NONE_BRANCH {
            cmd.push_str(&format!(" -beta {branch}"));
        }
        cmd.push_str(&format!(
            " -username \"{username}\" -dir \"{}\"",
            dest_folder.display()
        ));
        if remember_password {
            cmd.push_str(" -remember-password");
        }
        lines.push(cmd);
    }
    lines.push("exit".to_string());
    lines
}

/// Writes the batch file next to the app (temp file, kept so the spawned
/// console can read it) and runs it in a new console window.
pub fn run_downloader(base_dir: &Path, lines: &[String]) -> anyhow::Result<PathBuf> {
    let batch_path = base_dir.join(format!(
        "tdm-download-{}.bat",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default()
    ));
    std::fs::write(&batch_path, lines.join("\r\n") + "\r\n")
        .with_context(|| format!("write {}", batch_path.display()))?;

    let mut cmd = std::process::Command::new("cmd.exe");
    cmd.arg("/c").arg(&batch_path).current_dir(base_dir);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(crate::constants::windows::CREATE_NEW_CONSOLE);
    }
    cmd.spawn().context("spawn DepotDownloader batch")?;
    Ok(batch_path)
}

/// The DepotDownloader executable is expected next to the app binary.
pub fn downloader_exe(base_dir: &Path) -> PathBuf {
    base_dir.join("DepotDownloader.exe")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::PATCH_TITLE_KEY;

    #[test]
    fn builds_expected_commands() {
        let mut entry = ManifestEntry::new();
        entry.insert("553851".into(), "111".into());
        entry.insert("553852".into(), "222".into());
        entry.insert(PATCH_TITLE_KEY.into(), "Patch".into());
        let lines = build_batch_lines(
            "553850",
            &["553851".into(), "553852".into(), "553853".into()],
            NONE_BRANCH,
            &entry,
            "user",
            true,
            Path::new("C:/games/H2_v2026-01-01"),
        );
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            "DepotDownloader.exe -app 553850 -depot 553851 -manifest 111 -username \"user\" -dir \"C:/games/H2_v2026-01-01\" -remember-password"
        );
        // Depot 553853 has no manifest -> skipped; trailing "exit".
        assert!(lines[1].contains("-depot 553852 -manifest 222"));
        assert_eq!(lines[2], "exit");
    }

    #[test]
    fn beta_branch_adds_flag() {
        let mut entry = ManifestEntry::new();
        entry.insert("1".into(), "m".into());
        let lines = build_batch_lines(
            "10",
            &["1".into()],
            "beta-experimental",
            &entry,
            "",
            false,
            Path::new("d"),
        );
        assert!(lines[0].contains(" -beta beta-experimental "));
        assert!(!lines[0].contains("-remember-password"));
    }
}
