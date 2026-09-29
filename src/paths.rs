use std::path::PathBuf;

use anyhow::Context as _;
use directories::BaseDirs;

/// Reverse-DNS application identity — used for system notifications
/// (`set_app_identity`) and by the OS to associate the app; the data
/// directories use the conventional short name instead.
pub const APP_ID: &str = "com.abyss-rei.timedivers-manager-rs";
pub const APP_NAME: &str = "Timedivers Manager V2";

/// Data directory name under the OS data directories (conventional short
/// name, matching what most apps place under %APPDATA%).
const DIR_NAME: &str = "timedivers-manager-rs";

/// Filesystem locations for persisted state and external tools.
pub struct AppPaths {
    /// Roaming config directory (`%APPDATA%\timedivers-manager-rs`) — small
    /// user settings.
    pub config_dir: PathBuf,
    /// Local data directory (`%LOCALAPPDATA%\timedivers-manager-rs`) —
    /// manifests and browser profiles are derived data and belong on the
    /// local machine.
    pub data_dir: PathBuf,
    /// Directory of the running executable — external tools such as
    /// DepotDownloader.exe are looked up here.
    pub exe_dir: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> anyhow::Result<Self> {
        let base = BaseDirs::new().context("resolve base directories")?;
        let config_dir = base.data_dir().join(DIR_NAME);
        let data_dir = base.data_local_dir().join(DIR_NAME);
        std::fs::create_dir_all(&config_dir)
            .with_context(|| format!("create {}", config_dir.display()))?;
        std::fs::create_dir_all(&data_dir)
            .with_context(|| format!("create {}", data_dir.display()))?;
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        Ok(Self {
            config_dir,
            data_dir,
            exe_dir,
        })
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join(crate::config::CONFIG_FILE)
    }
}
