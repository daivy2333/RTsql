use std::collections::HashSet;

/// Snapshot represents the view of the database at a specific point in time
/// Used for Repeatable Read isolation level
pub struct Snapshot {
    tx_id: u64,
    active_tx_ids: HashSet<u64>,
}

impl Snapshot {
    pub fn new(tx_id: u64, active_tx_ids: Vec<u64>) -> Self {
        Self {
            tx_id,
            active_tx_ids: active_tx_ids.into_iter().collect(),
        }
    }

    pub fn tx_id(&self) -> u64 {
        self.tx_id
    }

    /// Check if a version is visible to this snapshot (Repeatable Read rules)
    ///
    /// A version is visible if:
    /// 1. The creating transaction has committed (commit_tx_id exists)
    /// 2. The creating transaction ID < snapshot ID (before this snapshot)
    /// 3. The creating transaction is NOT in the active list (not active when snapshot was taken)
    pub fn is_visible(&self, create_tx_id: u64, commit_tx_id: Option<u64>) -> bool {
        // Rule 1: must be committed
        let _commit_tx_id = match commit_tx_id {
            Some(id) => id,
            None => return false,
        };

        // Rule 2: create_tx_id <= snapshot tx_id
        if create_tx_id > self.tx_id {
            return false;
        }

        // Rule 3: not in active list
        if self.active_tx_ids.contains(&create_tx_id) {
            return false;
        }

        true
    }

    /// Check if self-created uncommitted version is visible
    /// A transaction should see its own uncommitted writes
    pub fn is_visible_self(&self, create_tx_id: u64, commit_tx_id: Option<u64>) -> bool {
        create_tx_id == self.tx_id && commit_tx_id.is_none()
    }

    /// Check if a transaction ID is in the active set (used by page-level visibility)
    pub fn contains_active_tx(&self, tx_id: u64) -> bool {
        self.active_tx_ids.contains(&tx_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_visible_committed_before() {
        // Snapshot TxId=5, active=[2, 3]
        // Version create_tx_id=1, commit_tx_id=Some(4)
        // 1 < 5, committed, not in active -> visible
        let snapshot = Snapshot::new(5, vec![2, 3]);
        assert!(snapshot.is_visible(1, Some(4)));
    }

    #[test]
    fn test_snapshot_not_visible_uncommitted() {
        // Snapshot TxId=5
        // Version create_tx_id=4, commit_tx_id=None
        // Uncommitted -> not visible
        let snapshot = Snapshot::new(5, vec![]);
        assert!(!snapshot.is_visible(4, None));
    }

    #[test]
    fn test_snapshot_not_visible_active_tx() {
        // Snapshot TxId=5, active=[4]
        // Version create_tx_id=4, commit_tx_id=Some(6)
        // 4 in active list -> not visible (even though committed)
        let snapshot = Snapshot::new(5, vec![4]);
        assert!(!snapshot.is_visible(4, Some(6)));
    }

    #[test]
    fn test_snapshot_not_visible_after_snapshot() {
        // Snapshot TxId=5
        // Version create_tx_id=6, commit_tx_id=Some(7)
        // 6 > 5 -> not visible
        let snapshot = Snapshot::new(5, vec![]);
        assert!(!snapshot.is_visible(6, Some(7)));
    }

    #[test]
    fn test_snapshot_visible_self_created() {
        // Snapshot TxId=5, active=[5] (self in active list)
        // Version create_tx_id=5, commit_tx_id=None
        // Self-created uncommitted -> visible
        let snapshot = Snapshot::new(5, vec![5]);
        assert!(snapshot.is_visible_self(5, None));
    }
}
