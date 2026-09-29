use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

/// Live observation of one depot cache folder.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DepotSnapshot {
    pub exists: bool,
    pub bytes: u64,
    pub files: u64,
}

/// Sums the size and file count of a depot cache folder (Steam writes files
/// into it progressively while `download_depot` runs).
pub fn snapshot_depot(dir: &Path) -> DepotSnapshot {
    let mut snap = DepotSnapshot::default();
    let Ok(top) = std::fs::read_dir(dir) else {
        return snap;
    };
    snap.exists = true;
    let mut stack = vec![top];
    while let Some(rd) = stack.pop() {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Ok(child) = std::fs::read_dir(&path) {
                    stack.push(child);
                }
            } else if let Ok(meta) = entry.metadata() {
                snap.bytes += meta.len();
                snap.files += 1;
            }
        }
    }
    snap
}

/// Machine states for one depot download being watched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepotPhase {
    /// Steam has not started this depot yet (no cache folder).
    Pending,
    /// The cache folder exists and bytes arrived recently.
    Downloading,
    /// Both the expected size and file count are reached (and the tail has
    /// settled), or no growth for `STABLE_FOR` as a fallback.
    Complete,
}

/// How long a depot folder must stop growing before it is considered complete
/// without (or despite) expected metadata.
pub const STABLE_FOR: Duration = Duration::from_secs(20);

/// Settle window after the expected size/file count are reached: lets the
/// last files land before switching to Complete.
pub const SETTLE_FOR: Duration = Duration::from_secs(5);

/// Expected thresholds for one depot: the download size (as reported by
/// Steam/SteamDB) and, when exactly known, the expected file count.
#[derive(Debug, Clone, Copy)]
pub struct DepotExpectation {
    pub bytes: u64,
    /// Only known when learned from a previous import of the same manifest.
    pub files: Option<u64>,
}

/// One depot's rendered watch state.
#[derive(Debug, Clone, PartialEq)]
pub struct DepotWatch {
    pub depot: String,
    pub phase: DepotPhase,
    pub bytes: u64,
    pub files: u64,
    pub expected_bytes: Option<u64>,
    pub expected_files: Option<u64>,
}

/// Per-depot download state machine. Each `poll` applies one observation
/// round: growth refreshes the activity timer; a depot completes when the
/// expected size AND file count are reached and the tail has settled, or —
/// when no expectation is known/reachable — after the stability fallback.
pub struct DepotWatcher {
    pub expected: BTreeMap<String, DepotExpectation>,
    prev: BTreeMap<String, (u64, u64)>,
    last_change: BTreeMap<String, Instant>,
}

impl DepotWatcher {
    pub fn new(expected: BTreeMap<String, DepotExpectation>) -> Self {
        Self {
            expected,
            prev: BTreeMap::new(),
            last_change: BTreeMap::new(),
        }
    }

    /// Seeds the baseline without triggering a "growth" transition, so
    /// already-downloaded depots show Complete immediately after open.
    pub fn seed(&mut self, depots: &[String], snapshots: &[DepotSnapshot], now: Instant) {
        let started = now - STABLE_FOR;
        for (depot, snap) in depots.iter().zip(snapshots) {
            self.prev.insert(depot.clone(), (snap.bytes, snap.files));
            self.last_change.entry(depot.clone()).or_insert(started);
        }
    }

    pub fn poll(
        &mut self,
        depots: &[String],
        snapshots: &[DepotSnapshot],
        now: Instant,
    ) -> Vec<DepotWatch> {
        let mut out = Vec::new();
        for (depot, snap) in depots.iter().zip(snapshots) {
            let phase = if !snap.exists {
                self.last_change.remove(depot);
                DepotPhase::Pending
            } else {
                let grew = self
                    .prev
                    .get(depot)
                    .map(|&(pb, pf)| snap.bytes > pb || snap.files > pf)
                    .unwrap_or(true);
                let last_change = self.last_change.entry(depot.clone()).or_insert(now);
                if grew {
                    *last_change = now;
                }
                let stable_for = now.duration_since(*last_change);
                let thresholds_met = self
                    .expected
                    .get(depot)
                    .map(|e| snap.bytes >= e.bytes && e.files.is_none_or(|ef| snap.files >= ef))
                    .unwrap_or(false);
                let settled = thresholds_met && stable_for >= SETTLE_FOR;
                if settled || stable_for >= STABLE_FOR {
                    DepotPhase::Complete
                } else {
                    DepotPhase::Downloading
                }
            };
            self.prev.insert(depot.clone(), (snap.bytes, snap.files));
            let (expected_bytes, expected_files) = match self.expected.get(depot) {
                Some(e) => (Some(e.bytes), e.files),
                None => (None, None),
            };
            out.push(DepotWatch {
                depot: depot.clone(),
                phase,
                bytes: snap.bytes,
                files: snap.files,
                expected_bytes,
                expected_files,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn watcher() -> DepotWatcher {
        DepotWatcher::new(BTreeMap::new())
    }

    fn snap(exists: bool, bytes: u64, files: u64) -> DepotSnapshot {
        DepotSnapshot {
            exists,
            bytes,
            files,
        }
    }

    fn depots() -> Vec<String> {
        vec!["1".to_string()]
    }

    #[test]
    fn preexisting_depot_is_complete_immediately() {
        let mut w = watcher();
        let t0 = Instant::now();
        w.seed(&depots(), &[snap(true, 1000, 10)], t0);
        let watch = w.poll(&depots(), &[snap(true, 1000, 10)], t0);
        assert_eq!(watch[0].phase, DepotPhase::Complete);
        assert_eq!(watch[0].bytes, 1000);
    }

    #[test]
    fn missing_folder_is_pending() {
        let mut w = watcher();
        let t0 = Instant::now();
        let watch = w.poll(&depots(), &[snap(false, 0, 0)], t0);
        assert_eq!(watch[0].phase, DepotPhase::Pending);
    }

    #[test]
    fn appearing_then_growing_is_downloading() {
        let mut w = watcher();
        let t0 = Instant::now();
        let watch = w.poll(&depots(), &[snap(false, 0, 0)], t0);
        assert_eq!(watch[0].phase, DepotPhase::Pending);

        let t1 = t0 + Duration::from_secs(1);
        let watch = w.poll(&depots(), &[snap(true, 500, 2)], t1);
        assert_eq!(watch[0].phase, DepotPhase::Downloading);

        let t2 = t1 + Duration::from_secs(1);
        let watch = w.poll(&depots(), &[snap(true, 900, 3)], t2);
        assert_eq!(watch[0].phase, DepotPhase::Downloading);
    }

    #[test]
    fn stable_folder_completes_after_timeout() {
        let mut w = watcher();
        let t0 = Instant::now();
        w.poll(&depots(), &[snap(true, 500, 2)], t0);
        let t1 = t0 + Duration::from_secs(25);
        let watch = w.poll(&depots(), &[snap(true, 500, 2)], t1);
        assert_eq!(watch[0].phase, DepotPhase::Complete);
    }

    #[test]
    fn growth_reverts_complete_back_to_downloading() {
        let mut w = watcher();
        let t0 = Instant::now();
        w.poll(&depots(), &[snap(true, 500, 2)], t0);
        let t1 = t0 + Duration::from_secs(25);
        let watch = w.poll(&depots(), &[snap(true, 500, 2)], t1);
        assert_eq!(watch[0].phase, DepotPhase::Complete);

        let t2 = t1 + Duration::from_secs(1);
        let watch = w.poll(&depots(), &[snap(true, 700, 4)], t2);
        assert_eq!(watch[0].phase, DepotPhase::Downloading);
    }

    #[test]
    fn expected_size_and_count_complete_after_settle() {
        let mut expected = BTreeMap::new();
        expected.insert(
            "1".to_string(),
            DepotExpectation {
                bytes: 1000,
                files: Some(3),
            },
        );
        let mut w = DepotWatcher::new(expected);
        let t0 = Instant::now();
        // below both thresholds: still downloading
        let watch = w.poll(&depots(), &[snap(true, 900, 2)], t0);
        assert_eq!(watch[0].phase, DepotPhase::Downloading);

        // both thresholds reached, settle window not yet passed
        let t1 = t0 + Duration::from_secs(1);
        let watch = w.poll(&depots(), &[snap(true, 1000, 3)], t1);
        assert_eq!(watch[0].phase, DepotPhase::Downloading);

        // settled: size AND file count reached -> Complete
        let t2 = t0 + Duration::from_secs(6);
        let watch = w.poll(&depots(), &[snap(true, 1000, 3)], t2);
        assert_eq!(watch[0].phase, DepotPhase::Complete);
        assert_eq!(watch[0].expected_bytes, Some(1000));
        assert_eq!(watch[0].expected_files, Some(3));
    }

    #[test]
    fn thresholds_met_while_growing_waits_for_settle() {
        // Learned expectations from a smaller previous manifest: the growing
        // download crosses the thresholds early, but must not complete until
        // growth stops and the settle window passes.
        let mut expected = BTreeMap::new();
        expected.insert(
            "1".to_string(),
            DepotExpectation {
                bytes: 500,
                files: Some(2),
            },
        );
        let mut w = DepotWatcher::new(expected);
        let t0 = Instant::now();
        let watch = w.poll(&depots(), &[snap(true, 600, 2)], t0);
        assert_eq!(watch[0].phase, DepotPhase::Downloading);

        let t1 = t0 + Duration::from_secs(1);
        let watch = w.poll(&depots(), &[snap(true, 800, 3)], t1);
        assert_eq!(watch[0].phase, DepotPhase::Downloading);

        // stable for 3s: still inside the settle window
        let t2 = t1 + Duration::from_secs(3);
        let watch = w.poll(&depots(), &[snap(true, 800, 3)], t2);
        assert_eq!(watch[0].phase, DepotPhase::Downloading);

        // stable beyond SETTLE_FOR: Complete
        let t3 = t2 + Duration::from_secs(6);
        let watch = w.poll(&depots(), &[snap(true, 800, 3)], t3);
        assert_eq!(watch[0].phase, DepotPhase::Complete);
    }

    #[test]
    fn unreachable_expectation_falls_back_to_stability() {
        let mut expected = BTreeMap::new();
        expected.insert(
            "1".to_string(),
            DepotExpectation {
                bytes: 1_000_000,
                files: Some(999),
            },
        );
        let mut w = DepotWatcher::new(expected);
        let t0 = Instant::now();
        let watch = w.poll(&depots(), &[snap(true, 500, 2)], t0);
        assert_eq!(watch[0].phase, DepotPhase::Downloading);

        let t1 = t0 + Duration::from_secs(25);
        let watch = w.poll(&depots(), &[snap(true, 500, 2)], t1);
        assert_eq!(watch[0].phase, DepotPhase::Complete);
    }
}
