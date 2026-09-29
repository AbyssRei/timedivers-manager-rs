use std::collections::HashMap;
use std::path::Path;

use anyhow::Context as _;
use ron::Options;
use ron::extensions::Extensions;
use serde::{Deserialize, Serialize};

pub const CONFIG_FILE: &str = "config.ron";
pub const MANIFESTS_DIR: &str = "manifests";

/// Shared RON settings: implicit `Some` keeps the files free of `Some(...)` noise.
pub(crate) fn ron_options() -> Options {
    Options::default().with_default_extension(Extensions::IMPLICIT_SOME)
}

/// Per-game persisted state. Missing fields mean "steam"/"None".
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GameConfig {
    #[serde(default)]
    pub active_date: Option<String>,
    #[serde(default)]
    pub active_branch: Option<String>,
    #[serde(default)]
    pub steamapps_folder: Option<String>,
}

impl GameConfig {
    pub fn active_date(&self) -> &str {
        self.active_date
            .as_deref()
            .unwrap_or(crate::constants::STEAM_VERSION)
    }

    pub fn active_branch(&self) -> &str {
        self.active_branch
            .as_deref()
            .unwrap_or(crate::constants::NONE_BRANCH)
    }
}

/// Top-level application config.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub steamapps_folders: Vec<String>,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub remember_password: bool,
    #[serde(default)]
    pub current_app_id: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub games: HashMap<String, GameConfig>,
}

impl AppConfig {
    /// Loads the config; a missing or malformed file falls back to defaults,
    /// mirroring `load_config` in the original implementation.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| ron_options().from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let text = ron_options().to_string(self).context("serialize config")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        std::fs::write(path, text).with_context(|| format!("write config to {}", path.display()))
    }

    pub fn game_config(&self, app_id: &str) -> GameConfig {
        self.games.get(app_id).cloned().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ron_config() {
        let text = r#"
(
    steamapps_folders: ["C:\\Steam\\steamapps"],
    username: "user",
    remember_password: false,
    current_app_id: "1234",
    games: {
        "1234": (
            active_date: "2026-09-23 01:12:34",
            active_branch: "None",
            steamapps_folder: "C:\\Steam\\steamapps",
        ),
    },
)"#;
        let cfg: AppConfig = ron_options().from_str(text).unwrap();
        assert_eq!(cfg.steamapps_folders.len(), 1);
        assert_eq!(cfg.current_app_id.as_deref(), Some("1234"));
        let game = cfg.game_config("1234");
        assert_eq!(game.active_date(), "2026-09-23 01:12:34");
        assert_eq!(game.active_branch(), "None");
    }

    #[test]
    fn missing_fields_fall_back_to_steam() {
        let text = "(steamapps_folders: [])";
        let cfg: AppConfig = ron_options().from_str(text).unwrap();
        assert_eq!(cfg.current_app_id, None);
        assert_eq!(cfg.game_config("999").active_date(), "steam");
    }

    #[test]
    fn round_trips() {
        let mut cfg = AppConfig::default();
        cfg.steamapps_folders
            .push("D:\\SteamLibrary\\steamapps".into());
        cfg.games.insert(
            "1".into(),
            GameConfig {
                active_date: Some("steam".into()),
                active_branch: Some("None".into()),
                steamapps_folder: Some("D:\\SteamLibrary\\steamapps".into()),
            },
        );
        let text = ron_options().to_string(&cfg).unwrap();
        let back: AppConfig = ron_options().from_str(&text).unwrap();
        assert_eq!(back.steamapps_folders, cfg.steamapps_folders);
        assert_eq!(back.game_config("1").active_date(), "steam");
    }

    #[test]
    fn malformed_file_falls_back_to_default() {
        let path = std::env::temp_dir().join(format!("tdm-config-bad-{}.ron", std::process::id()));
        std::fs::write(&path, "not valid ron {{{{").unwrap();
        let cfg = AppConfig::load(&path);
        assert_eq!(cfg.steamapps_folders, Vec::<String>::new());
        std::fs::remove_file(&path).ok();
    }
}
