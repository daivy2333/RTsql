/// Per-page MVCC visibility summary for fast-path skipping.
///
/// `all_visible`: every slot's version has been committed (commit_tx_id != UNSET).
///   Cleared by any write (INSERT/DELETE/UPDATE/COMMIT), lazily re-set on read.
///
/// `min_create_tx_id`: minimum create_tx_id among all slots on this page.
///   If `min_create_tx_id > snapshot.tx_id()`, the entire page is invisible
///   to that snapshot (all rows created after snapshot started).
#[derive(Debug, Clone, Copy, Default)]
pub struct PageVisibilityInfo {
    pub min_create_tx_id: u64,
    pub all_visible: bool,
}

impl PageVisibilityInfo {
    /// Returns true if every row on this page is invisible to the given snapshot
    /// because all rows were created after the snapshot started.
    pub fn all_invisible_for(&self, snapshot_tx_id: u64) -> bool {
        self.min_create_tx_id > snapshot_tx_id
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
}
