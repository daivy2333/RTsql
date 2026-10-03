use crate::storage::buffer_pool::BufferPool;
use crate::storage::data::TableMeta;
use crate::storage::page_format::{overflow, RowId, SlottedPage, SlottedPageRef};
use crate::storage::{PageId, Result, StorageError};
use crate::transaction::VersionHeader;
use std::sync::Arc;

/// Write a tuple with MVCC version header into a data page.
/// Auto-allocates a new page if the current tail page is full.
/// Returns RowId pointing to the stored slot.
///
/// MS27 T3 (D4): slot_data longer than `overflow::INLINE_SLOT_MAX` (4074B)
/// no longer fails with PageFull — the tuple bytes go to a freshly allocated
/// overflow chain and the data page slot carries a 31B locator
/// (`[VersionHeader][0xFF][total_len][first_overflow_page]`). The locator is
/// placed last: a crash before placement leaves the chain unreferenced, never
/// dangling. Any failure frees the chain pages allocated by this call.
pub async fn write_tuple_to_data_page(
    buffer_pool: &Arc<BufferPool>,
    table_meta: &Arc<TableMeta>,
    version_header: &VersionHeader,
    tuple_bytes: &[u8],
) -> Result<RowId> {
    let mut slot_data = version_header.to_bytes();
    slot_data.extend_from_slice(tuple_bytes);

    // MS27 T3: oversized rows divert to the overflow chain before any data
    // page is touched; the ≤4074 path below is byte-identical to legacy.
    let mut overflow_first_page: Option<u32> = None;
    if slot_data.len() > overflow::INLINE_SLOT_MAX {
        let first = overflow::write_overflow_chain(buffer_pool, tuple_bytes).await?;
        let locator = overflow::encode_locator(tuple_bytes.len() as u32, first);
        let mut locator_slot = version_header.to_bytes();
        locator_slot.extend_from_slice(&locator);
        slot_data = locator_slot;
        overflow_first_page = Some(first);
    }

    let tail_id = *table_meta.data_page_tail.lock().unwrap();

    let guard = buffer_pool.get_page(tail_id).await?;

    let add_result: std::result::Result<(u16, usize), String> = guard.modify_page(|page| {
        let page_type = page.data[0];
        let mut slotted = if page_type == 0 {
            SlottedPage::init(page, 0x03)
        } else {
            SlottedPage::new(page)
        };
        slotted.add_slot(&slot_data)
    });

    match add_result {
        Ok((logical_id, _slot_index)) => Ok(RowId::new(tail_id.0 as u32, logical_id)),
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

            let placement = new_guard.modify_page(|page| {
                let mut slotted = SlottedPage::new(page);
                slotted.add_slot(&slot_data)
            });

            let (logical_id, _slot_index): (u16, usize) = match placement {
                Ok(v) => v,
                Err(_) => {
                    // MS27 T3 (R4): the locator never made it onto a data
                    // page — free the chain this call allocated so the
                    // failed write leaks nothing. The free is best-effort;
                    // the original PageFull error surfaces either way.
                    if let Some(first) = overflow_first_page {
                        let _ = overflow::free_overflow_chain(buffer_pool, first).await;
                    }
                    return Err(StorageError::PageFull);
                }
            };

            *table_meta.data_page_tail.lock().unwrap() = new_page_id;

            Ok(RowId::new(new_page_id.0 as u32, logical_id))
        }
    }
}

/// Read a tuple and its version header from a data page by RowId.
///
/// M20: closure-based zero-copy API. The closure receives
/// `(VersionHeader, &[u8])` where `&[u8]` is the tuple payload
/// (no VersionHeader prefix). The page lock is held for the closure's
/// lifetime; the closure must not call `.await` or recursively invoke
/// `BufferPool` methods.
///
/// Callers that need owned bytes (e.g. write paths, WAL) can call
/// `bytes.to_vec()` inside the closure.
///
/// MS27 T4: for an overflow row the slot carries a locator instead of tuple
/// bytes — the locator is read inside the page lock, the payload is assembled
/// from the overflow chain outside it, and the closure then consumes the
/// assembled bytes. Inline rows keep the zero-copy path untouched.
pub async fn read_tuple_from_data_page<F, R>(
    buffer_pool: &BufferPool,
    row_id: RowId,
    f: F,
) -> Result<R>
where
    F: FnOnce(VersionHeader, &[u8]) -> Result<R>,
{
    enum SlotRead<R> {
        /// Closure already consumed inside the page lock (zero-copy inline row).
        Inline(R),
        /// Locator read; the closure is still pending and runs after assembly.
        Overflow {
            vh: VersionHeader,
            total_len: u32,
            first_page: u32,
        },
    }

    let page_id = PageId(row_id.page_id as u64);
    let mut f_opt = Some(f);

    let read = buffer_pool
        .with_page_data(page_id, |data| -> Result<SlotRead<R>> {
            let slotted = SlottedPageRef::new(data);
            let (slot, _) = slotted
                .get_slot_by_logical_id(row_id.slot_id)
                .ok_or(StorageError::SlotNotFound(row_id))?;
            let slot_data = slotted.get_slot_data(&slot);
            let version_header = VersionHeader::from_bytes(&slot_data[..VersionHeader::SIZE])
                .ok_or_else(|| {
                    StorageError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "malformed version header",
                    ))
                })?;
            let tuple_bytes = &slot_data[VersionHeader::SIZE..];

            if overflow::is_overflow_ref(tuple_bytes) {
                let (total_len, first_page) =
                    overflow::decode_locator(tuple_bytes).ok_or_else(|| {
                        StorageError::Io(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "malformed overflow locator",
                        ))
                    })?;
                return Ok(SlotRead::Overflow {
                    vh: version_header,
                    total_len,
                    first_page,
                });
            }

            let f = f_opt.take().expect("closure should be available");
            f(version_header, tuple_bytes).map(SlotRead::Inline)
        })
        .await?;

    match read {
        SlotRead::Inline(result) => Ok(result),
        SlotRead::Overflow {
            vh,
            total_len,
            first_page,
        } => {
            let bytes = overflow::read_overflow_chain(buffer_pool, first_page, total_len).await?;
            let f = f_opt.take().expect("closure should be available");
            f(vh, &bytes)
        }
    }
}

/// Update only the version header in a data page slot (M10)
pub async fn update_version_header_in_data_page(
    buffer_pool: &BufferPool,
    row_id: RowId,
    new_header: VersionHeader,
    _tuple_bytes: &[u8],
) -> Result<()> {
    let page_id = PageId(row_id.page_id as u64);

    let page_guard = buffer_pool.get_page(page_id).await?;

    let result: std::result::Result<(), String> = page_guard.modify_page(|page| {
        let slotted = SlottedPage::new(page);

        let (slot, _) = slotted
            .get_slot_by_logical_id(row_id.slot_id)
            .ok_or_else(|| format!("logical_id {} not found", row_id.slot_id))?;
        let slot_offset = slot.offset as usize;

        let header_bytes = new_header.to_bytes();
        page.data[slot_offset..slot_offset + VersionHeader::SIZE].copy_from_slice(&header_bytes);
        Ok(())
    });

    result.map_err(|_| StorageError::SlotNotFound(row_id))?;
    Ok(())
}

/// Delete a tuple from a data page by marking its slot as deleted (M10 GC)
/// This is used for garbage collection to remove old committed versions.
///
/// MS27 T6 (design D5): an overflow version's slot carries a locator — the
/// slot is removed first, then the chain pages are freed (a reader that
/// misses the slot already treats the row as gone; freeing first would leave
/// the locator pointing at recycled pages). Chain free is best-effort, same
/// policy as drop_table's physical release.
pub async fn delete_tuple_from_data_page(buffer_pool: &BufferPool, row_id: RowId) -> Result<()> {
    let page_id = PageId(row_id.page_id as u64);

    let page_guard = buffer_pool.get_page(page_id).await?;

    // Peek the slot before removal: is its tuple segment an overflow locator?
    let overflow_first: Option<u32> = {
        let probe: std::result::Result<Option<u32>, ()> = page_guard.modify_page(|page| {
            let slotted = SlottedPageRef::new(&page.data[..]);
            let Some((slot, _)) = slotted.get_slot_by_logical_id(row_id.slot_id) else {
                return Err(());
            };
            let slot_data = slotted.get_slot_data(&slot);
            if slot_data.len() < VersionHeader::SIZE {
                return Ok(None);
            }
            Ok(overflow::decode_locator(&slot_data[VersionHeader::SIZE..]).map(|(_, first)| first))
        });
        probe.map_err(|_| StorageError::SlotNotFound(row_id))?
    };

    let result: std::result::Result<(), String> = page_guard.modify_page(|page| {
        let mut slotted = SlottedPage::new(page);
        slotted.delete_slot_by_logical_id(row_id.slot_id)
    });

    result.map_err(|_| StorageError::SlotNotFound(row_id))?;
    drop(page_guard);
    // M21: Clear page visibility summary after DELETE
    buffer_pool.clear_all_visible(page_id);
    // MS27 T6: the slot is gone — free the now-unreferenced chain.
    if let Some(first) = overflow_first {
        if let Err(e) = overflow::free_overflow_chain(buffer_pool, first).await {
            eprintln!(
                "[delete_tuple_from_data_page] free_overflow_chain({first}) failed: {}",
                e
            );
        }
    }
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
        let pool = Arc::new(BufferPool::new(10, storage.clone()).unwrap());
        let tm = TableManager::new(pool.clone(), storage).await.unwrap();
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
        let (read_vh, read_tuple) =
            read_tuple_from_data_page(&pool, row_id, |vh, bytes| Ok((vh, bytes.to_vec())))
                .await
                .unwrap();

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

        let (r1, d1) = read_tuple_from_data_page(&pool, rid1, |vh, bytes| Ok((vh, bytes.to_vec())))
            .await
            .unwrap();
        let (r2, d2) = read_tuple_from_data_page(&pool, rid2, |vh, bytes| Ok((vh, bytes.to_vec())))
            .await
            .unwrap();
        let (r3, d3) = read_tuple_from_data_page(&pool, rid3, |vh, bytes| Ok((vh, bytes.to_vec())))
            .await
            .unwrap();

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
            let (vh, data) =
                read_tuple_from_data_page(&pool, *rid, |vh, bytes| Ok((vh, bytes.to_vec())))
                    .await
                    .unwrap();
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
        let result = read_tuple_from_data_page(&pool, bad_rid, |_, _| Ok(())).await;
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

        let (read_vh, read_data) =
            read_tuple_from_data_page(&pool, row_id, |vh, bytes| Ok((vh, bytes.to_vec())))
                .await
                .unwrap();

        assert_eq!(read_vh.create_tx_id(), 42);
        assert_eq!(read_vh.commit_tx_id(), None);
        assert_eq!(read_data, b"roundtrip-data");
    }
}
