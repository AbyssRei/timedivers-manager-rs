use std::path::{Path, PathBuf};

use anyhow::Context as _;

use crate::constants::{NONE_BRANCH, effective_branch};
use crate::steam::acf::parse_acf;

/// A Steam game detected from `appmanifest_*.acf` files.
#[derive(Debug, Clone)]
pub struct SteamGame {
    pub app_id: String,
    pub name: String,
    pub installdir: String,
    /// Installed depot ids, in manifest order.
    pub depots: Vec<String>,
    pub steamapps_folder: PathBuf,
    /// Effective branch name; `"None"` for the default branch.
    pub beta: String,
}

impl SteamGame {
    pub fn has_beta(&self) -> bool {
        self.beta != NONE_BRANCH
    }
}

fn is_appmanifest(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with("appmanifest_") && n.ends_with(".acf"))
}

/// Detects installed games across all configured steamapps folders. The same
/// app id is only reported once (first folder wins), and games without
/// `appid`/`name`/`installdir`/depots are skipped — both matching the
/// original `detect_games`.
pub fn detect_games(steamapps_folders: &[String]) -> Vec<SteamGame> {
    let mut games: Vec<SteamGame> = Vec::new();
    for folder in steamapps_folders {
        let dir = PathBuf::from(folder);
        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut manifests: Vec<PathBuf> = read_dir
            .flatten()
            .map(|e| e.path())
            .filter(|p| is_appmanifest(p))
            .collect();
        manifests.sort();
        for acf_path in manifests {
            let game = (|| -> anyhow::Result<Option<SteamGame>> {
                let data = parse_acf(&acf_path).context("parse acf")?;
                let app_id = data.get_str("appid").context("no appid")?.to_string();
                let name = data.get_str("name").context("no name")?.to_string();
                let installdir = data
                    .get_str("installdir")
                    .context("no installdir")?
                    .to_string();
                let depots = data.key_list("InstalledDepots");
                if depots.is_empty() {
                    return Ok(None);
                }
                let beta = data
                    .get_map("UserConfig")
                    .and_then(|u| u.get_str("BetaKey").map(str::to_string))
                    .unwrap_or_else(|| NONE_BRANCH.to_string());
                Ok(Some(SteamGame {
                    app_id,
                    name,
                    installdir,
                    depots,
                    steamapps_folder: dir.clone(),
                    beta: effective_branch(&beta).to_string(),
                }))
            })();
            match game {
                Ok(Some(game)) => {
                    if !games.iter().any(|g| g.app_id == game.app_id) {
                        games.push(game);
                    }
                }
                _ => continue,
            }
        }
    }
    games
}

/// Games sorted by lowercase display name (original `_refresh_games` order).
pub fn sorted_by_name(mut games: Vec<SteamGame>) -> Vec<SteamGame> {
    games.sort_by_key(|a| a.name.to_lowercase());
    games
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn detects_games_from_acf_files() {
        let tmp = std::env::temp_dir().join(format!("tdm-detect-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let steamapps = tmp.join("steamapps");
        write(
            &steamapps.join("appmanifest_553850.acf"),
            r#""AppState" { "appid" "553850" "name" "HELLDIVERS 2" "installdir" "Helldivers 2"
                "InstalledDepots" { "553851" { "ManifestID" "1" } }
                "UserConfig" { "BetaKey" "public" } }"#,
        );
        write(
            &steamapps.join("appmanifest_1174180.acf"),
            r#""AppState" { "appid" "1174180" "name" "Red Dead Redemption 2" "installdir" "RDR2"
                "InstalledDepots" { "1174181" { "ManifestID" "2" } } }"#,
        );
        // Incomplete manifest without depots must be skipped.
        write(
            &steamapps.join("appmanifest_1.acf"),
            r#""AppState" { "appid" "1" "name" "broken" "installdir" "x" "InstalledDepots" { } }"#,
        );

        let games = sorted_by_name(detect_games(&[steamapps.to_string_lossy().into()]));
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].name, "HELLDIVERS 2");
        assert_eq!(games[1].app_id, "1174180");
        assert!(!games[0].has_beta());
        assert_eq!(games[0].depots, vec!["553851"]);
        let _ = fs::remove_dir_all(&tmp);
    }
}
