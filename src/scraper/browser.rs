//! Discovering and launching the user's Chromium-based browser for CDP.

use std::path::PathBuf;

/// A Chromium-based browser that can be driven over CDP.
#[derive(Debug, Clone)]
pub struct BrowserInfo {
    pub name: String,
    pub exe: PathBuf,
    /// Filesystem-safe identifier for the `browser_data` profile folder.
    pub slug: String,
}

#[cfg(windows)]
mod imp {
    use super::BrowserInfo;
    use std::path::{Path, PathBuf};
    use windows_registry::{CURRENT_USER, LOCAL_MACHINE};

    fn find_system_browsers() -> Vec<(String, PathBuf)> {
        let hives = [
            (LOCAL_MACHINE, r"SOFTWARE\Clients\StartMenuInternet"),
            (
                LOCAL_MACHINE,
                r"SOFTWARE\WOW6432Node\Clients\StartMenuInternet",
            ),
            (CURRENT_USER, r"SOFTWARE\Clients\StartMenuInternet"),
        ];
        let mut out = Vec::new();
        for (hive, path) in hives {
            let Ok(root) = hive.open(path) else {
                continue;
            };
            let Ok(keys) = root.keys() else {
                continue;
            };
            for key_name in keys {
                let Ok(bkey) = root.open(&key_name) else {
                    continue;
                };
                let display_name = bkey.get_string("").unwrap_or_else(|_| key_name.clone());
                let Ok(cmd_key) = bkey.open(r"shell\open\command") else {
                    continue;
                };
                let Ok(raw) = cmd_key.get_string("") else {
                    continue;
                };
                let raw = raw.trim();
                let exe = if let Some(unquoted) = raw.strip_prefix('"') {
                    unquoted.split('"').next().map(str::to_string)
                } else {
                    raw.split_whitespace().next().map(str::to_string)
                };
                if let Some(exe) = exe {
                    out.push((display_name, PathBuf::from(exe)));
                }
            }
        }
        out
    }

    /// Detects installed Chromium-based browsers, deduplicating by executable
    /// and excluding Firefox (no CDP) — mirroring `detect_browsers`.
    pub fn detect_browsers() -> Vec<BrowserInfo> {
        let mut found: Vec<BrowserInfo> = Vec::new();
        let mut seen: Vec<PathBuf> = Vec::new();
        for (name, exe) in find_system_browsers() {
            if seen.contains(&exe) || !exe.is_file() {
                continue;
            }
            let basename = exe
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_lowercase();
            if basename.contains("firefox") {
                continue;
            }
            seen.push(exe.clone());
            let slug = name.to_lowercase().replace([' ', '/'], "_");
            found.push(BrowserInfo { name, exe, slug });
        }
        found
    }

    pub fn stealth_flags() -> Vec<String> {
        vec![
            "--no-first-run".into(),
            "--no-default-browser-check".into(),
            "--disable-blink-features=AutomationControlled".into(),
            "--disable-features=msEdgeAutomationOverlay".into(),
            "--exclude-switches=enable-automation".into(),
        ]
    }

    /// Launches the user's browser with a debugging port and a dedicated
    /// profile under `browser_data`, opening the SteamDB login page.
    pub fn launch_browser(
        browser: &BrowserInfo,
        browser_data_dir: &Path,
        port: u16,
    ) -> std::io::Result<std::process::Child> {
        let user_data_dir = browser_data_dir.join(&browser.slug);
        std::fs::create_dir_all(&user_data_dir)?;
        let mut cmd = std::process::Command::new(&browser.exe);
        cmd.arg(format!("--remote-debugging-port={port}"))
            .arg(format!("--user-data-dir={}", user_data_dir.display()))
            .args(stealth_flags())
            .arg("https://steamdb.info/login/")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(crate::constants::windows::CREATE_NO_WINDOW);
        cmd.spawn()
    }
}

#[cfg(not(windows))]
mod imp {
    use super::BrowserInfo;

    pub fn detect_browsers() -> Vec<BrowserInfo> {
        Vec::new()
    }

    pub fn stealth_flags() -> Vec<String> {
        Vec::new()
    }
}

pub use imp::detect_browsers;
#[cfg(windows)]
pub(super) use imp::launch_browser;
pub use imp::stealth_flags;

#[cfg(test)]
mod tests {
    use super::*;

    /// Runtime check against the real registry (Windows only): at least one
    /// Chromium-based browser must be discoverable on a normal system.
    #[cfg(windows)]
    #[test]
    fn detects_at_least_one_chromium_browser() {
        let browsers = detect_browsers();
        assert!(!browsers.is_empty(), "no browsers detected via registry");
        for b in &browsers {
            assert!(b.exe.is_file(), "detected exe missing: {}", b.exe.display());
            assert!(!b.name.is_empty());
        }
    }
}
