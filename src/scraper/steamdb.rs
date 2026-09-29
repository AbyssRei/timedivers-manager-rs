use std::collections::BTreeMap;

use jiff::Timestamp;
use scraper::{ElementRef, Html, Selector};

use crate::constants::PATCH_TITLE_KEY;
use crate::manifest::{ManifestEntry, ManifestMap, normalize_date, parse_manifest_date_key};

/// Parses a SteamDB depot manifests page into `branch -> date -> manifest_id`.
/// Mirrors `parse_table`: the correct table carries `seen date` + `manifestid`
/// headers; the branch comes from `code.js-branch` in the manifest cell.
/// Result of parsing a depot manifests page: `branch -> date -> manifest_id`
/// plus, when the table carries a size column, `manifest_id -> bytes`.
pub struct ParsedManifests {
    pub branches: BTreeMap<String, BTreeMap<String, String>>,
    /// Download size per manifest id as reported by SteamDB. Downloaded
    /// content decompresses on disk, so this differs from the on-disk size —
    /// it is what the Steam console itself displays.
    pub sizes: BTreeMap<String, u64>,
}

/// Converts a SteamDB size text like `"157 MB"` / `"1.2 GB"` into bytes.
pub fn parse_size_text(text: &str) -> Option<u64> {
    let text = text.trim().to_lowercase();
    let mut split = text.splitn(2, ' ');
    let number: f64 = split.next()?.replace(',', "").parse().ok()?;
    let unit = split.next()?.trim();
    let multiplier = match unit {
        "b" => 1.0,
        "kb" => 1024.0,
        "mb" => 1024.0 * 1024.0,
        "gb" => 1024.0 * 1024.0 * 1024.0,
        "tb" => 1024.0f64 * 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((number * multiplier) as u64)
}

pub fn parse_manifests_table(html: &str, depot_id: &str) -> ParsedManifests {
    let document = Html::parse_document(html);
    let table_sel = Selector::parse("table.table").unwrap();
    let th_sel = Selector::parse("th").unwrap();
    let tr_sel = Selector::parse("tbody tr").unwrap();
    let td_sel = Selector::parse("td").unwrap();
    let a_sel = Selector::parse("a").unwrap();
    let branch_sel = Selector::parse("code.js-branch").unwrap();

    let mut table: Option<ElementRef> = None;
    let mut size_col: Option<usize> = None;
    for t in document.select(&table_sel) {
        let headers: Vec<String> = t
            .select(&th_sel)
            .map(|th| th.text().collect::<String>().trim().to_lowercase())
            .collect();
        if headers.iter().any(|h| h == "seen date") && headers.iter().any(|h| h == "manifestid") {
            size_col = headers.iter().position(|h| h.contains("size"));
            table = Some(t);
            break;
        }
    }
    let Some(table) = table else {
        eprintln!("No correct table found for depot {depot_id}");
        return ParsedManifests {
            branches: BTreeMap::new(),
            sizes: BTreeMap::new(),
        };
    };

    let mut result: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut sizes: BTreeMap<String, u64> = BTreeMap::new();
    for row in table.select(&tr_sel) {
        let cols: Vec<ElementRef> = row.select(&td_sel).collect();
        if cols.len() < 3 {
            continue;
        }
        let manifest_cell = cols[2];
        let Some(manifest_link) = manifest_cell.select(&a_sel).next() else {
            continue;
        };
        let branch_name = manifest_cell
            .select(&branch_sel)
            .next()
            .map(|code| code.text().collect::<String>().trim().to_string())
            .unwrap_or_else(|| "None".to_string());
        let date_str = normalize_date(cols[0].text().collect::<String>().trim());
        let manifest_id = manifest_link.text().collect::<String>().trim().to_string();
        if let Some(cell) = size_col.and_then(|ix| cols.get(ix)) {
            let size_text = cell.text().collect::<String>();
            if let Some(bytes) = parse_size_text(&size_text) {
                sizes.insert(manifest_id.clone(), bytes);
            }
        }
        result
            .entry(branch_name)
            .or_default()
            .entry(date_str)
            .or_insert(manifest_id);
    }

    // Day collapse: when only one manifest is known for a day, the key drops
    // the time component (`2026-09-23` instead of `2026-09-23 01:12:34`).
    let mut collapsed = BTreeMap::new();
    for (branch_name, dates) in result {
        let mut day_counts: BTreeMap<String, usize> = BTreeMap::new();
        for key in dates.keys() {
            *day_counts.entry(key[..10].to_string()).or_default() += 1;
        }
        let cleaned: BTreeMap<String, String> = dates
            .into_iter()
            .map(|(key, manifest_id)| {
                let day = key[..10].to_string();
                let key = if day_counts[&day] == 1 { day } else { key };
                (key, manifest_id)
            })
            .collect();
        collapsed.insert(branch_name, cleaned);
    }
    ParsedManifests {
        branches: collapsed,
        sizes,
    }
}

/// Parses a SteamDB patch notes page into `(date_key, title)` pairs,
/// mirroring `scrape_patch_titles`.
pub fn parse_patch_titles(html: &str) -> Vec<(String, String)> {
    let document = Html::parse_document(html);
    let builds_sel = Selector::parse("tbody#js-builds").unwrap();
    let Some(table) = document.select(&builds_sel).next() else {
        eprintln!("No correct patch notes table found.");
        return Vec::new();
    };
    let tr_sel = Selector::parse("tr").unwrap();
    let td_sel = Selector::parse("td").unwrap();
    let title_link_sel = Selector::parse("a.b").unwrap();

    let mut patch_titles = Vec::new();
    for row in table.select(&tr_sel) {
        let cols: Vec<ElementRef> = row.select(&td_sel).collect();
        if cols.len() < 4 {
            continue;
        }
        let title_cell = cols[3];
        let title = title_cell
            .select(&title_link_sel)
            .next()
            .map(|a| a.text().collect::<String>().trim().to_string())
            .unwrap_or_else(|| title_cell.text().collect::<String>().trim().to_string());
        if title.is_empty() {
            continue;
        }
        let date_key = match row.value().attr("data-date") {
            Some(data_date) => data_date
                .parse::<i64>()
                .ok()
                .and_then(|secs| jiff::Timestamp::from_second(secs).ok())
                .map(|ts| ts.strftime("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| normalize_date(cols[0].text().collect::<String>().trim())),
            None => normalize_date(cols[0].text().collect::<String>().trim()),
        };
        patch_titles.push((date_key, title));
    }
    patch_titles
}

/// Attaches patch titles to the manifest date whose timestamp is closest on
/// the same day — a faithful port of the matching loop in `scraper.main`.
pub fn apply_patch_titles(manifests: &mut ManifestMap, patch_titles: &[(String, String)]) {
    let mut day_titles: BTreeMap<String, Vec<(Option<Timestamp>, String)>> = BTreeMap::new();
    for (key, title) in patch_titles {
        day_titles
            .entry(key[..10].to_string())
            .or_default()
            .push((parse_manifest_date_key(key), title.clone()));
    }

    for branch_data in manifests.values_mut() {
        let mut day_manifests: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for key in branch_data.keys() {
            day_manifests
                .entry(key[..10].to_string())
                .or_default()
                .push(key.clone());
        }

        for (day, titles) in &day_titles {
            let Some(m_keys) = day_manifests.get(day) else {
                continue;
            };
            for (pt_dt, title) in titles {
                let Some(closest) = m_keys.iter().min_by_key(|k| {
                    let dt = parse_manifest_date_key(k);
                    match (dt, pt_dt.as_ref().copied()) {
                        (Some(a), Some(b)) => (a.as_second() - b.as_second()).abs(),
                        _ => i64::MAX,
                    }
                }) else {
                    continue;
                };
                if let Some(entry) = branch_data.get_mut(closest) {
                    let entry: &mut ManifestEntry = entry;
                    entry.insert(PATCH_TITLE_KEY.to_string(), title.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depot_html(rows: &[(&str, &str, &str)]) -> String {
        let rows: String = rows
            .iter()
            .map(|(date, manifest, branch)| {
                let branch_html = if branch.is_empty() {
                    String::new()
                } else {
                    format!(r#" <code class="js-branch">{branch}</code>"#)
                };
                format!(
                    r#"<tr>
<td>{date}</td><td>abc123</td>
<td><a href="/manifests/7654321/">{manifest}</a>{branch_html}</td>
</tr>"#
                )
            })
            .collect();
        format!(
            r#"<html><body><table class="table">
<thead><tr><th>Seen Date</th><th>Size</th><th>ManifestID</th></tr></thead>
<tbody>{rows}</tbody></table></body></html>"#
        )
    }

    #[test]
    fn parses_manifest_rows() {
        let html = depot_html(&[
            ("12 September 2025 - 12:34:56 UTC", "1111111111", ""),
            (
                "11 September 2025 - 10:00:00 UTC",
                "2222222222",
                "beta-branch",
            ),
        ]);
        let parsed = parse_manifests_table(&html, "553851").branches;
        // One manifest per branch-day collapses to the day-only key.
        assert_eq!(parsed["None"]["2025-09-12"], "1111111111");
        assert_eq!(parsed["beta-branch"]["2025-09-11"], "2222222222");
    }

    #[test]
    fn collapses_single_manifest_days() {
        let html = depot_html(&[
            ("12 September 2025 - 12:34:56 UTC", "1111111111", ""),
            ("11 September 2025 - 10:00:00 UTC", "2222222222", ""),
            ("11 September 2025 - 18:00:00 UTC", "3333333333", ""),
        ]);
        let parsed = parse_manifests_table(&html, "553851").branches;
        // One manifest that day -> day-only key.
        assert_eq!(parsed["None"]["2025-09-12"], "1111111111");
        // Two manifests that day -> full timestamps kept.
        assert!(parsed["None"].contains_key("2025-09-11 10:00:00"));
        assert!(parsed["None"].contains_key("2025-09-11 18:00:00"));
    }

    #[test]
    fn rejects_tables_without_expected_headers() {
        let html = r#"<table class="table"><thead><tr><th>Other</th></tr></thead>
            <tbody><tr><td><a>x</a></td><td></td><td><a>1</a></td></tr></tbody></table>"#;
        assert!(parse_manifests_table(html, "1").branches.is_empty());
    }

    #[test]
    fn parses_patch_notes() {
        let html = r#"<html><body><table><tbody id="js-builds">
<tr data-date="1757678096"><td>12 September 2025</td><td></td><td></td>
<td><a href="/app/553850/patchnotes/123" class="b">Patch 01.003.200</a></td></tr>
<tr data-date="1757000000"><td>4 September 2025</td><td></td><td></td>
<td><a href="/app/553850/patchnotes/124" class="b">Patch 01.003.100</a></td></tr>
</tbody></table></body></html>"#;
        let titles = parse_patch_titles(html);
        assert_eq!(titles.len(), 2);
        assert_eq!(titles[0].0, "2025-09-12 11:54:56");
        assert_eq!(titles[0].1, "Patch 01.003.200");
    }

    #[test]
    fn matches_patch_title_to_closest_manifest() {
        let mut manifests: ManifestMap = BTreeMap::new();
        let mut dates = BTreeMap::new();
        let mut entry1 = ManifestEntry::new();
        entry1.insert("553851".into(), "m1".into());
        dates.insert("2025-09-12 09:00:00".to_string(), entry1.clone());
        let mut entry2 = ManifestEntry::new();
        entry2.insert("553851".into(), "m2".into());
        dates.insert("2025-09-12 18:00:00".to_string(), entry2);
        manifests.insert("None".into(), dates);

        let titles = vec![("2025-09-12 17:55:00".to_string(), "Patch B".to_string())];
        apply_patch_titles(&mut manifests, &titles);
        let branch = &manifests["None"];
        assert_eq!(branch["2025-09-12 18:00:00"][PATCH_TITLE_KEY], "Patch B");
        assert!(!branch["2025-09-12 09:00:00"].contains_key(PATCH_TITLE_KEY));
    }
}
