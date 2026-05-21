use crate::storage::buffer_pool::BufferPool;
use crate::storage::data::TableMeta;
use crate::storage::page_format::{RowId, SlottedPage};
use crate::storage::{PageId, Result, StorageError};
use crate::transaction::VersionHeader;
use std::sync::Arc;

/// Write a tuple with MVCC version header into a data page.
/// Auto-allocates a new page if the current tail page is full.
/// Returns RowId pointing to the stored slot.
pub async fn write_tuple_to_data_page(
    buffer_pool: &Arc<BufferPool>,
    table_meta: &Arc<TableMeta>,
    version_header: &VersionHeader,
    tuple_bytes: &[u8],
) -> Result<RowId> {
    let mut slot_data = version_header.to_bytes();
    slot_data.extend_from_slice(tuple_bytes);

    let tail_id = *table_meta.data_page_tail.lock().unwrap();

    let guard = buffer_pool.get_page(tail_id).await?;

    let add_result: std::result::Result<usize, String> = guard.modify_page(|page| {
        let page_type = page.data[0];
        let mut slotted = if page_type == 0 {
            SlottedPage::init(page, 0x03)
        } else {
            SlottedPage::new(page)
        };
        slotted.add_slot(&slot_data)
    });

    match add_result {
        Ok(slot_idx) => Ok(RowId::new(tail_id.0 as u32, slot_idx as u16)),
        Err(_) => {
            let new_page_id = buffer_pool.storage().allocate_page().await?;

            let new_guard = buffer_pool.get_page(new_page_id).await?;
            new_guard.modify_page(|page| {
                SlottedPage::init(page, 0x03);
            });

            guard.modify_page(|page| {
                let next_id = new_page_id.0 as u32;
                page.data[5..9].copy_from_slice(&next_id.to_le_bytes());
            });

            let slot_idx: usize = new_guard
                .modify_page(|page| {
                    let mut slotted = SlottedPage::new(page);
                    slotted.add_slot(&slot_data)
                })
                .map_err(|_| StorageError::PageFull)?;

            *table_meta.data_page_tail.lock().unwrap() = new_page_id;

            Ok(RowId::new(new_page_id.0 as u32, slot_idx as u16))
        }
    }
}

/// Read a tuple and its version header from a data page by RowId.
pub async fn read_tuple_from_data_page(
    buffer_pool: &BufferPool,
    row_id: RowId,
) -> Result<(VersionHeader, Vec<u8>)> {
    let page_id = PageId(row_id.page_id as u64);
    let guard = buffer_pool.get_page(page_id).await?;

    let mut page_clone = guard.page();
    let slotted = SlottedPage::new(&mut page_clone);

    let slot = slotted
        .get_slot(row_id.slot_id as usize)
        .ok_or(StorageError::SlotNotFound(row_id))?;

    let slot_data = slotted.get_slot_data(&slot);

    let version_header =
        VersionHeader::from_bytes(&slot_data[..VersionHeader::SIZE]).ok_or_else(|| {
            StorageError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "malformed version header",
            ))
        })?;

    let tuple_bytes = slot_data[VersionHeader::SIZE..].to_vec();

    Ok((version_header, tuple_bytes))
}

/// Update only the version header in a data page slot (M10)
pub async fn update_version_header_in_data_page(
    buffer_pool: &BufferPool,
    row_id: RowId,
    new_header: VersionHeader,
    _tuple_bytes: &[u8],
) -> Result<()> {
    let page_id = PageId(row_id.page_id as u64);
    let slot_id = row_id.slot_id as usize;

    let page_guard = buffer_pool.get_page(page_id).await?;

    let result: std::result::Result<(), String> = page_guard.modify_page(|page| {
        let slotted = SlottedPage::new(page);

        let slot = slotted
            .get_slot(slot_id)
            .ok_or_else(|| format!("slot {} not found", slot_id))?;
        let slot_offset = slot.offset as usize;

        // Write the new header in place
        let header_bytes = new_header.to_bytes();
        page.data[slot_offset..slot_offset + VersionHeader::SIZE].copy_from_slice(&header_bytes);

        Ok(())
    });

    result.map_err(|_| StorageError::SlotNotFound(row_id))?;
    Ok(())
}

/// Delete a tuple from a data page by marking its slot as deleted (M10 GC)
/// This is used for garbage collection to remove old committed versions.
pub async fn delete_tuple_from_data_page(
    buffer_pool: &BufferPool,
    row_id: RowId,
) -> Result<()> {
    let page_id = PageId(row_id.page_id as u64);
    let slot_id = row_id.slot_id as usize;

    let page_guard = buffer_pool.get_page(page_id).await?;

    let result: std::result::Result<(), String> = page_guard.modify_page(|page| {
        let mut slotted = SlottedPage::new(page);
        slotted.delete_slot(slot_id)
    });

    result.map_err(|_| StorageError::SlotNotFound(row_id))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::data::TableManager;
    use crate::storage::page_format::ColumnType;
    use crate::storage::FileStorage;
    use tempfile::tempdir;

    async fn setup() -> (Arc<BufferPool>, Arc<TableMeta>, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
        let pool = Arc::new(BufferPool::new(10, storage).unwrap());
        let tm = TableManager::new(pool.clone());
        tm.create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
            .await
            .unwrap();
        let table = tm.get_table("test").await.unwrap();
        (pool, table, dir)
    }

    #[tokio::test]
    async fn write_read_single_tuple() {
        let (pool, table, _dir) = setup().await;

        let vh = VersionHeader::new(1, None);
        let tuple = b"hello";

        let row_id = write_tuple_to_data_page(&pool, &table, &vh, tuple)
            .await
            .unwrap();
        let (read_vh, read_tuple) = read_tuple_from_data_page(&pool, row_id).await.unwrap();

        assert_eq!(read_vh.create_tx_id(), 1);
        assert_eq!(read_vh.commit_tx_id(), None);
        assert_eq!(read_tuple, b"hello");
    }

    #[tokio::test]
    async fn write_read_multiple_tuples() {
        let (pool, table, _dir) = setup().await;

        let vh1 = VersionHeader::new(1, None);
        let vh2 = VersionHeader::new(2, Some(2));
        let vh3 = VersionHeader::new(3, Some(4));

        let rid1 = write_tuple_to_data_page(&pool, &table, &vh1, b"tuple-a")
            .await
            .unwrap();
        let rid2 = write_tuple_to_data_page(&pool, &table, &vh2, b"tuple-b")
            .await
            .unwrap();
        let rid3 = write_tuple_to_data_page(&pool, &table, &vh3, b"tuple-c")
            .await
            .unwrap();

        let (r1, d1) = read_tuple_from_data_page(&pool, rid1).await.unwrap();
        let (r2, d2) = read_tuple_from_data_page(&pool, rid2).await.unwrap();
        let (r3, d3) = read_tuple_from_data_page(&pool, rid3).await.unwrap();

        assert_eq!(d1, b"tuple-a");
        assert_eq!(d2, b"tuple-b");
        assert_eq!(d3, b"tuple-c");

        assert_eq!(r1.create_tx_id(), 1);
        assert_eq!(r2.create_tx_id(), 2);
        assert_eq!(r3.create_tx_id(), 3);
    }

    #[tokio::test]
    async fn page_full_auto_allocate() {
        let (pool, table, _dir) = setup().await;

        let mut row_ids = Vec::new();
        for i in 0u64..10 {
            let vh = VersionHeader::new(i, None);
            let data = vec![(i % 256) as u8; 1000];
            let rid = write_tuple_to_data_page(&pool, &table, &vh, &data)
                .await
                .unwrap();
            row_ids.push(rid);
        }

        for (i, rid) in row_ids.iter().enumerate() {
            let (vh, data) = read_tuple_from_data_page(&pool, *rid).await.unwrap();
            assert_eq!(vh.create_tx_id(), i as u64);
            assert_eq!(data.len(), 1000);
        }

        let first_page = row_ids[0].page_id;
        let has_other_page = row_ids.iter().any(|r| r.page_id != first_page);
        assert!(
            has_other_page,
            "should have auto-allocated at least one new page"
        );
    }

    #[tokio::test]
    async fn read_invalid_slot() {
        let (pool, table, _dir) = setup().await;

        let vh = VersionHeader::new(1, None);
        let row_id = write_tuple_to_data_page(&pool, &table, &vh, b"test")
            .await
            .unwrap();

        let bad_rid = RowId::new(row_id.page_id, 999);
        let result = read_tuple_from_data_page(&pool, bad_rid).await;
        assert!(result.is_err(), "reading non-existent slot must error");
    }

    #[tokio::test]
    async fn version_header_roundtrip() {
        let (pool, table, _dir) = setup().await;

        let vh = VersionHeader::new(42, None);
        let tuple = b"roundtrip-data";

        let row_id = write_tuple_to_data_page(&pool, &table, &vh, tuple)
            .await
            .unwrap();

        let (read_vh, read_data) = read_tuple_from_data_page(&pool, row_id).await.unwrap();

        assert_eq!(read_vh.create_tx_id(), 42);
        assert_eq!(read_vh.commit_tx_id(), None);
        assert_eq!(read_data, b"roundtrip-data");
    }
}
