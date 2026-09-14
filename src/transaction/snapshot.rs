use std::collections::HashSet;

/// Snapshot represents the view of the database at a specific point in time
///
/// Two constructors (MS09 Iter000 001-replan, D10):
/// - [`Snapshot::new`] — transaction snapshot where `tx_id` is both the
///   reader's identity and the visibility high-water mark (Repeatable Read
///   `begin`, and every historical caller).
/// - [`Snapshot::statement_view`] — Read Committed statement view where the
///   high-water mark (allocator current at statement start) is decoupled
///   from the reader's own id, so an active writer sharing the allocator's
///   current value cannot pass as "self" and a transaction that committed
///   between two statements of a reader transaction is not excluded.
///
/// `Clone` (MS09 Iter000): the Read Committed statement snapshot threads
/// through `create_executor_from_plan` by value and is stored by executors
/// that rebuild sub-plans per outer row. The clone is a plain field copy —
/// visibility semantics are untouched.
#[derive(Clone)]
pub struct Snapshot {
    tx_id: u64,
    high_water: u64,
    active_tx_ids: HashSet<u64>,
}

impl Snapshot {
    pub fn new(tx_id: u64, active_tx_ids: Vec<u64>) -> Self {
        Self {
            tx_id,
            high_water: tx_id,
            active_tx_ids: active_tx_ids.into_iter().collect(),
        }
    }

    /// Statement view for Read Committed (MS09 Iter000 001-replan, D10).
    ///
    /// `high_water` is the allocator's current value at statement start:
    /// every transaction already committed by then has an id ≤ it.
    /// `self_tx_id` carries only the reader's identity (0 for auto-commit
    /// statements, which own no in-flight versions — real version ids are
    /// always > 0, id 0 is the aborted marker), so `is_visible_self` never
    /// collides with an unrelated active writer.
    pub fn statement_view(high_water: u64, self_tx_id: u64, active_tx_ids: Vec<u64>) -> Self {
        Self {
            tx_id: self_tx_id,
            high_water,
            active_tx_ids: active_tx_ids.into_iter().collect(),
        }
    }

    pub fn tx_id(&self) -> u64 {
        self.tx_id
    }

    /// The visibility high-water mark: every transaction committed before the
    /// view was taken has an id ≤ it (MS09 Iter000 002-rework — page-level
    /// fast paths must consume this, not the reader's own id).
    pub fn high_water(&self) -> u64 {
        self.high_water
    }

    /// Check if a version is visible to this snapshot
    ///
    /// A version is visible if:
    /// 1. The creating transaction has committed (commit_tx_id exists)
    /// 2. The creating transaction ID <= the snapshot's high-water mark
    /// 3. The creating transaction is NOT in the active list (not active when snapshot was taken)
    pub fn is_visible(&self, create_tx_id: u64, commit_tx_id: Option<u64>) -> bool {
        // Rule 1: must be committed
        let _commit_tx_id = match commit_tx_id {
            Some(id) => id,
            None => return false,
        };

        // Rule 2: create_tx_id <= high_water
        if create_tx_id > self.high_water {
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

    // MS09 Iter000 001-replan (T6-R1/D10): statement view separates the
    // visibility high-water mark from the reader's own identity.

    #[test]
    fn test_statement_view_hides_active_writer_at_high_water() {
        // R2-S1 shape: auto-commit statement — the allocator's current value
        // equals the still-active writer's id. The uncommitted row must stay
        // invisible; self id 0 can never match a real create_tx_id.
        let snapshot = Snapshot::statement_view(7, 0, vec![7]);
        assert!(!snapshot.is_visible(7, None));
        assert!(!snapshot.is_visible_self(7, None));
    }

    #[test]
    fn test_statement_view_sees_commit_between_statements() {
        // R2-S2 shape: a transaction that began after the reader began but
        // committed before the current statement — create > self, create <=
        // high_water, committed, not active -> visible.
        let snapshot = Snapshot::statement_view(9, 5, vec![]);
        assert!(snapshot.is_visible(9, Some(9)));
    }

    #[test]
    fn test_statement_view_self_write_visible() {
        // Explicit transaction: its own uncommitted write stays visible,
        // other transactions' uncommitted writes do not.
        let snapshot = Snapshot::statement_view(9, 5, vec![5]);
        assert!(snapshot.is_visible_self(5, None));
        assert!(!snapshot.is_visible(5, None));
    }
}
