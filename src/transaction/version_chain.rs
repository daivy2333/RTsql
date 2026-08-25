use crate::storage::RowId;

/// Constants for unset values
const UNSET_TX_ID: u64 = 0xFFFFFFFFFFFFFFFF;
/// Sentinel for deleted rows: commit_tx_id = DELETED_TX_ID means the row
/// was deleted (committed delete). Distinguished from UNSET_TX_ID (uncommitted).
const DELETED_TX_ID: u64 = 0xFFFFFFFFFFFFFFFE;
const UNSET_ROW_ID: [u8; 6] = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];

/// VersionHeader stores metadata for MVCC version chain
///
/// Layout (22 bytes):
/// - create_tx_id (8 bytes): transaction that created this version
/// - commit_tx_id (8 bytes): transaction that committed this version (UNSET = None)
/// - next_version (6 bytes): RowId pointing to previous version (UNSET = None)
pub struct VersionHeader {
    create_tx_id: u64,
    commit_tx_id: u64,     // UNSET_TX_ID = None
    next_version: [u8; 6], // RowId bytes, UNSET_ROW_ID = None
}

impl VersionHeader {
    pub fn new(create_tx_id: u64, commit_tx_id: Option<u64>) -> Self {
        Self {
            create_tx_id,
            commit_tx_id: commit_tx_id.unwrap_or(UNSET_TX_ID),
            next_version: UNSET_ROW_ID,
        }
    }

    pub fn create_tx_id(&self) -> u64 {
        self.create_tx_id
    }

    pub fn commit_tx_id(&self) -> Option<u64> {
        if self.commit_tx_id == UNSET_TX_ID {
            None
        } else {
            Some(self.commit_tx_id)
        }
    }

    pub fn next_version(&self) -> Option<RowId> {
        if self.next_version == UNSET_ROW_ID {
            None
        } else {
            Some(RowId::deserialize(&self.next_version))
        }
    }

    pub fn with_next_version(mut self, row_id: RowId) -> Self {
        row_id.serialize(&mut self.next_version);
        self
    }

    pub fn commit(mut self, commit_tx_id: u64) -> Self {
        // Preserve the tombstone (DELETED_TX_ID) marker. DeleteExecutor
        // marks the row as deleted BEFORE tx_manager.commit() runs, and
        // commit() then propagates the real tx_id to all versions in
        // tx_versions. Without this guard, commit() would silently
        // overwrite the delete sentinel and DataScan would resurrect the
        // row.
        if self.commit_tx_id != DELETED_TX_ID {
            self.commit_tx_id = commit_tx_id;
        }
        self
    }

    /// Mark this version as deleted (committed delete).
    /// Sets commit_tx_id to DELETED_TX_ID sentinel.
    pub fn mark_deleted(mut self) -> Self {
        self.commit_tx_id = DELETED_TX_ID;
        self
    }

    /// Check if this version is marked as deleted.
    pub fn is_deleted(&self) -> bool {
        self.commit_tx_id == DELETED_TX_ID
    }

    /// Serialize to bytes (22 bytes)
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(22);
        bytes.extend_from_slice(&self.create_tx_id.to_le_bytes());
        bytes.extend_from_slice(&self.commit_tx_id.to_le_bytes());
        bytes.extend_from_slice(&self.next_version);
        bytes
    }

    /// Deserialize from bytes
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 22 {
            return None;
        }

        let create_tx_id = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        let commit_tx_id = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let next_version: [u8; 6] = bytes[16..22].try_into().unwrap();

        Some(Self {
            create_tx_id,
            commit_tx_id,
            next_version,
        })
    }

    /// Header size in bytes
    pub const SIZE: usize = 22;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::PageId;

    #[test]
    fn test_version_header_new() {
        let header = VersionHeader::new(1, None);

        assert_eq!(header.create_tx_id(), 1);
        assert_eq!(header.commit_tx_id(), None);
        assert_eq!(header.next_version(), None);
    }

    #[test]
    fn test_version_header_with_next_version() {
        // Adapted to use existing RowId structure: page_id (u32) + slot_id (u16)
        let row_id = RowId::new(0x00010002, 3); // page_id = 0x00010002 (combined)
        let header = VersionHeader::new(1, None).with_next_version(row_id);

        assert_eq!(header.next_version(), Some(row_id));
    }

    #[test]
    fn test_version_header_commit() {
        let header = VersionHeader::new(1, None);
        let committed = header.commit(5);

        assert_eq!(committed.commit_tx_id(), Some(5));
    }

    #[test]
    fn test_version_header_serialize() {
        let row_id = RowId::new(0x00010002, 3);
        let header = VersionHeader::new(3, Some(5)).with_next_version(row_id);

        let bytes = header.to_bytes();
        assert_eq!(bytes.len(), 22);

        let decoded = VersionHeader::from_bytes(&bytes).unwrap();
        assert_eq!(decoded.create_tx_id(), 3);
        assert_eq!(decoded.commit_tx_id(), Some(5));
        assert_eq!(decoded.next_version(), Some(row_id));
    }

    #[test]
    fn test_version_header_size() {
        assert_eq!(VersionHeader::SIZE, 22);
    }

    #[test]
    fn test_version_header_mark_deleted() {
        let header = VersionHeader::new(1, Some(5));
        assert!(!header.is_deleted());
        assert_eq!(header.commit_tx_id(), Some(5));

        let deleted = header.mark_deleted();
        assert!(deleted.is_deleted());
        // commit_tx_id returns Some(DELETED_TX_ID), not None
        assert_eq!(deleted.commit_tx_id(), Some(DELETED_TX_ID));
    }

    #[test]
    fn test_version_header_deleted_uncommitted_distinct() {
        // Uncommitted: commit_tx_id = UNSET_TX_ID
        let uncommitted = VersionHeader::new(1, None);
        assert!(!uncommitted.is_deleted());
        assert_eq!(uncommitted.commit_tx_id(), None);

        // Deleted: commit_tx_id = DELETED_TX_ID
        let deleted = VersionHeader::new(1, None).mark_deleted();
        assert!(deleted.is_deleted());
        assert_eq!(deleted.commit_tx_id(), Some(DELETED_TX_ID));
    }
}
