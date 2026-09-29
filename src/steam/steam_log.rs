use std::collections::BTreeMap;
use std::path::PathBuf;

/// Steam's own update logs are the authoritative signal source for
/// `download_depot` sessions: every console download runs through Steam's
/// update pipeline, which logs exact per-depot totals
/// (`logs/content_log.txt`) and per-depot completion events
/// (`logs/console_log.txt`).
///
/// Both logs are append-only and parsed in full; per-depot totals from later
/// sessions override earlier ones (the latest totals are the right
/// expectation for the current download of that depot).
#[cfg(windows)]
mod imp {
    use super::*;
    use std::path::Path;

    const DEPOT_MARKER: &str = "chunks for depot ";

    /// Steam install path from the registry (`Software\Valve\Steam\SteamPath`),
    /// with the default location as fallback.
    pub fn steam_path() -> Option<PathBuf> {
        if let Ok(key) = windows_registry::CURRENT_USER.open("Software\\Valve\\Steam")
            && let Ok(path) = key.get_string("SteamPath")
        {
            return Some(PathBuf::from(path));
        }
        if let Ok(key) = windows_registry::LOCAL_MACHINE.open("Software\\WOW6432Node\\Valve\\Steam")
            && let Ok(path) = key.get_string("InstallPath")
        {
            return Some(PathBuf::from(path));
        }
        Some(PathBuf::from(r"C:\Program Files (x86)\Steam"))
    }

    pub fn content_log_path() -> Option<PathBuf> {
        Some(steam_path()?.join("logs").join("content_log.txt"))
    }

    pub fn console_log_path() -> Option<PathBuf> {
        Some(steam_path()?.join("logs").join("console_log.txt"))
    }

    /// Live state of one console `download_depot` as logged in
    /// `console_log.txt`.
    #[derive(Debug, Clone, PartialEq)]
    pub struct ConsoleDepotDownload {
        pub files: u64,
        pub download_bytes: u64,
        pub complete: bool,
    }

    /// Extracts per-depot expectations from `content_log.txt`: pairs of
    /// `AppID <id> update started : download 0/<D> … stage 0/<S>` followed by
    /// `Downloading … chunks for depot <X>` become
    /// `depot X -> (download D, stage S)`. Later rounds override earlier ones
    /// (re-downloads update the expectation).
    pub fn session_totals(
        content_log: &Path,
        app_id: &str,
        depots: &[String],
    ) -> BTreeMap<String, (u64, u64)> {
        let Ok(data) = std::fs::read(content_log) else {
            return BTreeMap::new();
        };
        // Append-only log; per-depot totals from later sessions override
        // earlier ones (the latest totals are the right expectation for the
        // current download of that depot).
        let text = String::from_utf8_lossy(&data);
        let started_marker = format!("AppID {app_id} update started : ");

        let mut out: BTreeMap<String, (u64, u64)> = BTreeMap::new();
        let mut pending: Option<(u64, u64)> = None;
        for line in text.lines() {
            if line.contains(&started_marker) {
                pending = parse_update_started(line, app_id);
                continue;
            }
            if let (Some((download, stage)), Some(depot)) =
                (pending.take(), parse_downloading_depot(line, depots))
            {
                out.insert(depot, (download, stage));
            }
        }
        out
    }

    fn parse_update_started(line: &str, app_id: &str) -> Option<(u64, u64)> {
        let marker = format!("AppID {app_id} update started : ");
        let rest = line.split_once(&marker)?.1;
        let download = parse_field(rest, "download 0/")?;
        let stage = parse_field(rest, "stage 0/")?;
        Some((download, stage))
    }

    fn parse_field(line: &str, marker: &str) -> Option<u64> {
        let rest = line.split_once(marker)?.1;
        let token = rest
            .split([' ', ','])
            .find(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_digit()))?;
        token.parse::<u64>().ok()
    }

    /// `Downloading 252 chunks for depot 553851 (5937460960958490081)` — only
    /// depots in `depots` are accepted.
    fn parse_downloading_depot(line: &str, depots: &[String]) -> Option<String> {
        let idx = line.find(DEPOT_MARKER)?;
        let rest = &line[idx + DEPOT_MARKER.len()..];
        let depot = rest
            .split(|c: char| !c.is_ascii_digit())
            .find(|t| !t.is_empty())?;
        depots.iter().find(|d| d == &depot).cloned()
    }

    /// Extracts per-depot download states from `console_log.txt`:
    /// `Downloading depot <id> (<N> files, <S> MB) …` starts a session and
    /// `Depot download complete : "…depot_<id>" (…)` completes it. A new
    /// `Downloading` line for the same depot resets its completion state
    /// (re-downloads).
    pub fn console_download_states(
        console_log: &Path,
        depots: &[String],
    ) -> BTreeMap<String, ConsoleDepotDownload> {
        let Ok(data) = std::fs::read(console_log) else {
            return BTreeMap::new();
        };
        let text = String::from_utf8_lossy(&data);
        let download_marker = "Downloading depot ";
        let complete_marker = "Depot download complete : ";
        let depot_prefix = "depot_";

        let mut out: BTreeMap<String, ConsoleDepotDownload> = BTreeMap::new();
        let mut pending_depot: Option<String> = None;
        for line in text.lines() {
            if let Some(rest) = line.split_once(download_marker).map(|(_, r)| r) {
                // `Downloading depot 553851 (38 files, 157 MB) ...`
                let Some(depot) = rest
                    .split(|c: char| !c.is_ascii_digit())
                    .find(|t| !t.is_empty())
                else {
                    continue;
                };
                if !depots.iter().any(|d| d == depot) {
                    continue;
                }
                let files = rest
                    .split_once('(')
                    .and_then(|(_, meta)| meta.split(" files").next())
                    .and_then(|n| n.trim().parse::<u64>().ok());
                let download_bytes = rest
                    .split_once(" files, ")
                    .and_then(|(_, size)| size.split(')').next())
                    .and_then(crate::util::parse_size_text)
                    .unwrap_or(0);
                if let Some(files) = files {
                    out.insert(
                        depot.to_string(),
                        ConsoleDepotDownload {
                            files,
                            download_bytes,
                            complete: false,
                        },
                    );
                    pending_depot = Some(depot.to_string());
                }
                continue;
            }
            if let Some(rest) = line.split_once(complete_marker).map(|(_, r)| r) {
                // `Depot download complete : "…depot_<id>" (manifest …)`
                let depot = rest.split_once(depot_prefix).and_then(|(_, r)| {
                    r.split(|c: char| !c.is_ascii_digit())
                        .next()
                        .map(str::to_string)
                });
                if let Some(depot) = depot.filter(|d| depots.iter().any(|d2| d2 == d)) {
                    let entry = out.entry(depot.clone()).or_insert(ConsoleDepotDownload {
                        files: 0,
                        download_bytes: 0,
                        complete: false,
                    });
                    entry.complete = true;
                    if pending_depot.as_deref() == Some(depot.as_str()) {
                        pending_depot = None;
                    }
                }
            }
        }
        out
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    pub struct ConsoleDepotDownload {
        pub files: u64,
        pub download_bytes: u64,
        pub complete: bool,
    }

    pub fn content_log_path() -> Option<PathBuf> {
        None
    }

    pub fn console_log_path() -> Option<PathBuf> {
        None
    }

    pub fn session_totals(
        _content_log: &Path,
        _app_id: &str,
        _depots: &[String],
    ) -> BTreeMap<String, (u64, u64)> {
        BTreeMap::new()
    }

    pub fn console_download_states(
        _console_log: &Path,
        _depots: &[String],
    ) -> BTreeMap<String, ConsoleDepotDownload> {
        BTreeMap::new()
    }
}

pub use imp::{
    ConsoleDepotDownload, console_download_states, console_log_path, content_log_path,
    session_totals,
};

/// Convenience: session totals from the default log location.
pub fn session_totals_for(app_id: &str, depots: &[String]) -> BTreeMap<String, (u64, u64)> {
    match content_log_path() {
        Some(path) => session_totals(&path, app_id, depots),
        None => BTreeMap::new(),
    }
}

/// Convenience: console download states from the default log location.
pub fn console_download_states_for(
    _app_id: &str,
    depots: &[String],
) -> BTreeMap<String, ConsoleDepotDownload> {
    match console_log_path() {
        Some(path) => console_download_states(&path, depots),
        None => BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STARTED_553851: &str = "[2026-09-29 16:35:42] AppID 553850 update started : download 0/164881856, store 0/0, reuse 0/0, delta 0/0, stage 0/251047368 ";
    const STARTED_553853: &str = "[2026-09-28 06:16:44] AppID 553850 update started : download 0/22077875216, store 0/0, reuse 0/0, delta 0/0, stage 0/23314051044 ";
    const CHUNKS_553851: &str =
        "[2026-09-29 16:35:42] Downloading 252 chunks for depot 553851 (5937460960958490081)";
    const CHUNKS_553853: &str =
        "[2026-09-28 06:16:44] Downloading 23450 chunks for depot 553853 (9020691380707780829)";
    const NOISE: &str = "[2026-09-29 16:35:43] HTTP (CDN,29) - st.dl.eccdnx.com: AuthenticateDepotID (553851) - Success!";

    fn depots() -> Vec<String> {
        vec!["553851".to_string(), "553853".to_string()]
    }

    fn parse_lines(tag: &str, lines: &[&str]) -> BTreeMap<String, (u64, u64)> {
        let dir = std::env::temp_dir().join(format!("tdm-steam-log-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("content_log.txt");
        std::fs::write(&path, lines.join("\n")).unwrap();
        let totals = session_totals(&path, "553850", &depots());
        std::fs::remove_file(&path).ok();
        totals
    }

    #[test]
    fn pairs_update_started_with_following_depot() {
        let totals = parse_lines("single", &[STARTED_553851, CHUNKS_553851, NOISE]);
        assert_eq!(totals.get("553851"), Some(&(164881856, 251047368)));
        assert!(!totals.contains_key("553853"));
    }

    #[test]
    fn resolves_multiple_depots() {
        let totals = parse_lines(
            "multi",
            &[
                STARTED_553853,
                CHUNKS_553853,
                NOISE,
                STARTED_553851,
                CHUNKS_553851,
            ],
        );
        assert_eq!(totals.get("553853"), Some(&(22077875216, 23314051044)));
        assert_eq!(totals.get("553851"), Some(&(164881856, 251047368)));
    }

    #[test]
    fn re_download_overrides_expectation() {
        let totals = parse_lines(
            "override",
            &[
                STARTED_553851,
                CHUNKS_553851,
                STARTED_553851, // re-download: newer totals override
                CHUNKS_553851,
            ],
        );
        assert_eq!(totals.get("553851"), Some(&(164881856, 251047368)));
    }

    #[test]
    fn ignores_depot_lines_without_update_started() {
        let totals = parse_lines("chunks", &[CHUNKS_553853]);
        assert!(!totals.contains_key("553853"));
    }

    #[test]
    fn unparsable_started_line_is_skipped() {
        let bad = "[2026-09-29 16:35:42] AppID 553850 update started : download 0/x, stage 0/y ";
        let totals = parse_lines("bad", &[bad, CHUNKS_553851]);
        assert!(!totals.contains_key("553851"));
    }
}
