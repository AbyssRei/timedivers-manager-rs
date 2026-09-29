use std::path::PathBuf;

use crate::constants::{NONE_BRANCH, STEAM_VERSION};
use crate::steam::detect::SteamGame;

/// `{steamapps}/common`
pub fn common_dir(game: &SteamGame) -> PathBuf {
    game.steamapps_folder.join("common")
}

/// `{steamapps}/common/{installdir}` — the junction (or real folder) Steam launches.
pub fn active_folder(game: &SteamGame) -> PathBuf {
    common_dir(game).join(&game.installdir)
}

/// `{steamapps}/common/{installdir}_steam` — where the original install is kept.
pub fn steam_folder(game: &SteamGame) -> PathBuf {
    common_dir(game).join(format!("{}_steam", game.installdir))
}

/// Version folder name suffix: `_v{date}` or `_v{date}_{branch}`.
fn version_suffix(date: &str, branch: &str) -> String {
    let safe = sanitize_date(date);
    if branch == NONE_BRANCH {
        format!("_v{safe}")
    } else {
        format!("_v{safe}_{branch}")
    }
}

/// `{steamapps}/common/{installdir}_v{date}[_{branch}]`
pub fn version_folder(game: &SteamGame, date: &str, branch: &str) -> PathBuf {
    common_dir(game).join(format!(
        "{}{}",
        game.installdir,
        version_suffix(date, branch)
    ))
}

/// True for the pseudo "Steam Version" entry, which is stored in `_steam`.
pub fn is_steam_version(date: &str) -> bool {
    date == STEAM_VERSION
}

/// Path a version points at when activated (the `_steam` folder for Steam).
pub fn version_target_folder(game: &SteamGame, date: &str, branch: &str) -> PathBuf {
    if is_steam_version(date) {
        steam_folder(game)
    } else {
        version_folder(game, date, branch)
    }
}

/// Makes a date safe for folder names: spaces become `_`, colons become `-`.
pub fn sanitize_date(date: &str) -> String {
    date.replace(' ', "_").replace(':', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_game() -> SteamGame {
        SteamGame {
            app_id: "553850".into(),
            name: "HELLDIVERS 2".into(),
            installdir: "Helldivers 2".into(),
            depots: vec!["553851".into(), "553852".into()],
            steamapps_folder: PathBuf::from("C:/Steam/steamapps"),
            beta: NONE_BRANCH.into(),
        }
    }

    #[test]
    fn sanitizes_dates() {
        assert_eq!(sanitize_date("2026-09-23 01:12:34"), "2026-09-23_01-12-34");
        assert_eq!(sanitize_date("2026-09-23"), "2026-09-23");
    }

    #[test]
    fn builds_version_folders() {
        let game = sample_game();
        assert_eq!(
            version_folder(&game, "2026-09-23 01:12:34", NONE_BRANCH),
            PathBuf::from("C:/Steam/steamapps/common/Helldivers 2_v2026-09-23_01-12-34")
        );
        assert_eq!(
            version_folder(&game, "2026-09-23", "beta-experimental"),
            PathBuf::from("C:/Steam/steamapps/common/Helldivers 2_v2026-09-23_beta-experimental")
        );
    }

    #[test]
    fn steam_version_targets_steam_folder() {
        let game = sample_game();
        assert_eq!(
            version_target_folder(&game, STEAM_VERSION, NONE_BRANCH),
            PathBuf::from("C:/Steam/steamapps/common/Helldivers 2_steam")
        );
    }
}
