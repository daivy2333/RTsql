/// Sentinel for "no real min_create_tx_id is known for this page".
///
/// Used by summary first-builds where no insert has recorded a writer id yet
/// (`set_all_visible` on an entry-less page, and `clear_all_visible` first-builds
/// before the write path merges the real id). An UNKNOWN entry must never feed
/// the `all_invisible_for` comparison — unknown falls back to per-row checks.
/// MS17-T02 close-out: before the sentinel guard, a stale `{u64::MAX,
/// all_visible: false}` entry (scan-armed, then written) reported the whole
/// page all-invisible and silently hid committed rows.
pub const MIN_CREATE_UNKNOWN: u64 = u64::MAX;

/// Per-page MVCC visibility summary for fast-path skipping.
///
/// `all_visible`: every slot's version has been committed (commit_tx_id != UNSET).
///   Cleared by any write (INSERT/DELETE/UPDATE/COMMIT), lazily re-set on read.
///
/// `min_create_tx_id`: minimum create_tx_id among all slots on this page.
///   If `min_create_tx_id > snapshot.high_water()`, the entire page is
///   invisible to that snapshot (all rows created after the snapshot's
///   visibility high-water mark). The high-water mark — not the reader's
///   own id — is the correct comparison source for both snapshot shapes
///   (MS09 Iter000 002-rework). A value of [`MIN_CREATE_UNKNOWN`] means no
///   real writer id is known yet; it must never feed the comparison —
///   unknown falls back to per-row checks (MS17-T02 close-out).
#[derive(Debug, Clone, Copy, Default)]
pub struct PageVisibilityInfo {
    pub min_create_tx_id: u64,
    pub all_visible: bool,
}

impl PageVisibilityInfo {
    /// Returns true if every row on this page is invisible to the given snapshot
    /// because all rows were created after the snapshot started.
    ///
    /// The [`MIN_CREATE_UNKNOWN`] sentinel is excluded from the comparison: a
    /// stale `{UNKNOWN, all_visible: false}` entry (scan-armed first-build, then
    /// written) must fall back to per-row checks instead of reporting the whole
    /// page invisible (MS17-T02 close-out).
    pub fn all_invisible_for(&self, snapshot_tx_id: u64) -> bool {
        self.min_create_tx_id != MIN_CREATE_UNKNOWN && self.min_create_tx_id > snapshot_tx_id
    }

    /// Create a fresh info with default (all_visible=false, min_create_tx_id=0).
    /// This is the safe default — falls through to per-row checks.
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_invisible_when_min_gt_snapshot() {
        let info = PageVisibilityInfo {
            min_create_tx_id: 100,
            all_visible: false,
        };
        assert!(info.all_invisible_for(50)); // snapshot 50 < min 100 → invisible
        assert!(!info.all_invisible_for(100)); // equal → NOT strictly greater
        assert!(!info.all_invisible_for(150)); // snapshot 150 > min 100 → not invisible
    }

    #[test]
    fn test_default_is_safe() {
        let info = PageVisibilityInfo::default();
        assert!(!info.all_visible);
        assert_eq!(info.min_create_tx_id, 0);
        assert!(!info.all_invisible_for(0));
    }

    #[test]
    fn test_new_equals_default() {
        let a = PageVisibilityInfo::new();
        let b = PageVisibilityInfo::default();
        assert_eq!(a.all_visible, b.all_visible);
        assert_eq!(a.min_create_tx_id, b.min_create_tx_id);
    }

    #[test]
    #[allow(clippy::clone_on_copy)] // test intent: explicitly verify the Clone impl on a Copy type
    fn test_clone_and_copy() {
        let info = PageVisibilityInfo {
            min_create_tx_id: 42,
            all_visible: true,
        };
        let copy = info;
        assert_eq!(copy.min_create_tx_id, 42);
        assert!(copy.all_visible);
        let clone = info.clone();
        assert_eq!(clone.min_create_tx_id, 42);
        assert!(clone.all_visible);
    }

    /// MS17-T02 T5: the UNKNOWN sentinel must never report the whole page
    /// invisible — unknown min_create falls back to per-row checks.
    #[test]
    fn test_all_invisible_unknown_sentinel_falls_through() {
        let info = PageVisibilityInfo {
            min_create_tx_id: MIN_CREATE_UNKNOWN,
            all_visible: false,
        };
        assert!(!info.all_invisible_for(0));
        assert!(!info.all_invisible_for(u64::MAX - 1));
    }

    /// MS17-T02 T5: the sentinel guard only special-cases UNKNOWN; real tx ids
    /// keep the strict-greater semantics (mirror of
    /// `test_all_invisible_when_min_gt_snapshot`).
    #[test]
    fn test_real_values_unchanged_by_sentinel_guard() {
        let info = PageVisibilityInfo {
            min_create_tx_id: 100,
            all_visible: false,
        };
        assert!(info.all_invisible_for(50)); // snapshot 50 < min 100 → invisible
        assert!(!info.all_invisible_for(100)); // equal → NOT strictly greater
        assert!(!info.all_invisible_for(150)); // snapshot 150 > min 100 → not invisible
    }
}
