use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use jiff::Timestamp;

use crate::config::{MANIFESTS_DIR, ron_options};
use crate::constants::PATCH_TITLE_KEY;

/// `branch -> date -> {depot_id -> manifest_id, "patch_title" -> title}`
pub type ManifestMap = BTreeMap<String, BTreeMap<String, ManifestEntry>>;
pub type ManifestEntry = BTreeMap<String, String>;

pub fn manifests_path(base_dir: &Path, app_id: &str) -> PathBuf {
    base_dir.join(MANIFESTS_DIR).join(format!("{app_id}.ron"))
}

/// Download size per manifest id as reported by SteamDB (`manifest_id -> bytes`).
pub type ManifestSizes = BTreeMap<String, u64>;

pub fn manifest_sizes_path(base_dir: &Path, app_id: &str) -> PathBuf {
    base_dir
        .join(MANIFESTS_DIR)
        .join(format!("{app_id}.sizes.ron"))
}

pub fn load_manifest_sizes(base_dir: &Path, app_id: &str) -> ManifestSizes {
    let path = manifest_sizes_path(base_dir, app_id);
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| ron_options().from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_manifest_sizes(
    base_dir: &Path,
    app_id: &str,
    sizes: &ManifestSizes,
) -> anyhow::Result<PathBuf> {
    let path = manifest_sizes_path(base_dir, app_id);
    std::fs::create_dir_all(path.parent().context("manifests dir")?)?;
    let text = ron_options()
        .to_string(sizes)
        .context("serialize manifest sizes")?;
    std::fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

pub fn load_manifests(base_dir: &Path, app_id: &str) -> ManifestMap {
    let path = manifests_path(base_dir, app_id);
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| ron_options().from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save_manifests(
    base_dir: &Path,
    app_id: &str,
    manifests: &ManifestMap,
) -> anyhow::Result<PathBuf> {
    let path = manifests_path(base_dir, app_id);
    std::fs::create_dir_all(path.parent().context("manifests dir")?)?;
    let text = ron_options()
        .to_string(manifests)
        .context("serialize manifests")?;
    std::fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

pub fn patch_title_of(entry: &ManifestEntry) -> Option<&str> {
    entry.get(PATCH_TITLE_KEY).map(String::as_str)
}

/// Parses SteamDB date strings into the canonical `%Y-%m-%d [%H:%M:%S]` form.
/// Falls back to the original text with a warning, like `normalize_date`.
pub fn normalize_date(date_str: &str) -> String {
    let cleaned = date_str.replace('\u{2013}', "-").trim().to_string();
    // SteamDB timestamps are UTC; drop the suffix and parse naively.
    let stripped = cleaned.strip_suffix(" UTC").unwrap_or(&cleaned);
    if let Ok(dt) = jiff::civil::DateTime::strptime("%d %B %Y - %H:%M:%S", stripped) {
        return dt.strftime("%Y-%m-%d %H:%M:%S").to_string();
    }
    if let Ok(d) = jiff::civil::Date::strptime("%d %B %Y", stripped) {
        return d.strftime("%Y-%m-%d").to_string();
    }
    eprintln!("Warning: Could not parse date '{date_str}', leaving as-is");
    date_str.to_string()
}

/// Parses a manifest date key (`%Y-%m-%d %H:%M:%S` or `%Y-%m-%d`); `None`
/// sorts lowest, matching the `datetime.min` fallback in the original.
pub fn parse_manifest_date_key(key: &str) -> Option<Timestamp> {
    let dt = jiff::civil::DateTime::strptime("%Y-%m-%d %H:%M:%S", key)
        .ok()
        .or_else(|| {
            jiff::civil::Date::strptime("%Y-%m-%d", key)
                .ok()
                .map(|d| d.at(0, 0, 0, 0))
        })?;
    dt.to_zoned(jiff::tz::TimeZone::UTC)
        .ok()
        .map(|z| z.timestamp())
}

/// Backfills manifest ids for dates where a depot is missing, using the most
/// recent known value — a faithful port of `merge_manifests`.
pub fn merge_manifests(branch_data: &mut BTreeMap<String, ManifestEntry>, depots: &[String]) {
    let mut last_known: BTreeMap<String, String> = BTreeMap::new();
    let mut dates: Vec<String> = branch_data.keys().cloned().collect();
    dates.sort_by_key(|d| parse_manifest_date_key(d));
    for date in dates {
        let Some(entry) = branch_data.get_mut(&date) else {
            continue;
        };
        for depot in depots {
            let key = depot.as_str();
            if !entry.contains_key(key)
                && let Some(manifest_id) = last_known.get(key)
            {
                entry.insert(key.to_string(), manifest_id.clone());
            }
            if let Some(manifest_id) = entry.get(key) {
                last_known.insert(key.to_string(), manifest_id.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_steamdb_dates() {
        assert_eq!(
            normalize_date("12 September 2025 - 12:34:56 UTC"),
            "2025-09-12 12:34:56"
        );
        assert_eq!(normalize_date("12 September 2025"), "2025-09-12");
        assert_eq!(
            normalize_date("3 August 2024 - 08:00:00 UTC"),
            "2024-08-03 08:00:00"
        );
        // Unknown formats pass through untouched.
        assert_eq!(normalize_date("weird date"), "weird date");
    }

    #[test]
    fn handles_en_dash() {
        assert_eq!(
            normalize_date("12 September 2025 \u{2013} 12:34:56 UTC"),
            "2025-09-12 12:34:56"
        );
    }

    #[test]
    fn backfills_missing_depots() {
        let mut branch: BTreeMap<String, ManifestEntry> = BTreeMap::new();
        let mut e1 = ManifestEntry::new();
        e1.insert("100".into(), "m100a".into());
        e1.insert("200".into(), "m200a".into());
        branch.insert("2026-09-01 10:00:00".into(), e1);
        let mut e2 = ManifestEntry::new();
        e2.insert("100".into(), "m100b".into());
        branch.insert("2026-09-02 10:00:00".into(), e2);

        let depots = vec!["100".to_string(), "200".to_string()];
        merge_manifests(&mut branch, &depots);

        assert_eq!(branch["2026-09-02 10:00:00"]["100"], "m100b");
        // Depot 200 was not present on 09-02; it inherits the previous value.
        assert_eq!(branch["2026-09-02 10:00:00"]["200"], "m200a");
    }

    #[test]
    fn patch_title_round_trip() {
        let mut entry = ManifestEntry::new();
        entry.insert(PATCH_TITLE_KEY.into(), "Patch 01.003.200".into());
        assert_eq!(patch_title_of(&entry), Some("Patch 01.003.200"));
    }

    #[test]
    fn manifests_round_trip_through_ron() {
        let base = std::env::temp_dir().join(format!("tdm-ron-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();

        let mut manifests: ManifestMap = Default::default();
        let mut dates = BTreeMap::new();
        let mut entry = ManifestEntry::new();
        entry.insert("553851".into(), "8610520335704945941".into());
        entry.insert(PATCH_TITLE_KEY.into(), "Patch 01.000.005".into());
        dates.insert("2024-02-12".to_string(), entry);
        manifests.insert("None".to_string(), dates);

        let path = save_manifests(&base, "553850", &manifests).unwrap();
        assert!(path.extension().is_some_and(|e| e == "ron"));
        assert_eq!(load_manifests(&base, "553850"), manifests);
        std::fs::remove_dir_all(&base).ok();
    }
}
