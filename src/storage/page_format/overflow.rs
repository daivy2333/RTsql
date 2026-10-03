//! MS27 Iteration 000 (T2): overflow page format and chain primitives.
//!
//! A row whose `slot_data` (VersionHeader + tuple bytes) exceeds the inline
//! capacity of a data page slot ([`INLINE_SLOT_MAX`] = 4096 − 16 − 6 = 4074B)
//! is stored as:
//!
//! - data page slot: `[VersionHeader 22B][locator 9B]`, locator =
//!   `[0xFF][total_len u32 LE][first_overflow_page u32 LE]`;
//! - a chain of overflow pages ([`OVERFLOW_PAGE_TYPE`] = 0x04), each carrying
//!   a raw payload chunk of at most [`OVERFLOW_PAYLOAD_MAX`] (4080B) bytes
//!   starting at offset 16. Header field reuse: `free_space_offset` = this
//!   page's payload length, `next_page_id` = chain successor (0 terminates).
//!
//! The 0xFF marker cannot collide with an inline tuple: a tuple's first byte
//! is always a value tag in 0x01..=0x08 (tuple.rs tag domain).
//!
//! Overflow pages are independent pages — they never join a data page chain,
//! so data-page chain walkers (`data_chain_status`, dump precheck, DataScan
//! page hops) never see them.

use crate::storage::buffer_pool::BufferPool;
use crate::storage::page_format::{Slot, SlottedPageHeader};
use crate::storage::{Page, PageId, Result, StorageError};

/// First byte of the 9-byte locator that replaces the tuple bytes of an
/// oversized slot. Outside the tuple tag domain (0x01..=0x08).
pub const LOCATOR_TAG: u8 = 0xFF;
/// Locator length: tag + u32 total_len + u32 first_overflow_page.
pub const LOCATOR_SIZE: usize = 9;
/// Page type for overflow chunk pages (data pages are 0x03).
pub const OVERFLOW_PAGE_TYPE: u8 = 0x04;
/// Max payload bytes per overflow page: 4096 − 16B header.
pub const OVERFLOW_PAYLOAD_MAX: usize = Page::PAGE_SIZE - SlottedPageHeader::SIZE;
/// slot_data lengths at or below this stay inline (existing path,
/// byte-identical); anything above goes through an overflow chain.
pub const INLINE_SLOT_MAX: usize = Page::PAGE_SIZE - SlottedPageHeader::SIZE - Slot::SIZE;

/// Encode the 9-byte overflow locator written after the inline VersionHeader.
pub fn encode_locator(total_len: u32, first_overflow_page: u32) -> [u8; LOCATOR_SIZE] {
    let mut out = [0u8; LOCATOR_SIZE];
    out[0] = LOCATOR_TAG;
    out[1..5].copy_from_slice(&total_len.to_le_bytes());
    out[5..9].copy_from_slice(&first_overflow_page.to_le_bytes());
    out
}

/// Decode a locator from the tuple segment of a slot. Returns `None` when the
/// segment is shorter than [`LOCATOR_SIZE`] or does not start with
/// [`LOCATOR_TAG`] (i.e. it is an inline tuple).
pub fn decode_locator(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < LOCATOR_SIZE || data[0] != LOCATOR_TAG {
        return None;
    }
    let total_len = u32::from_le_bytes([data[1], data[2], data[3], data[4]]);
    let first_page = u32::from_le_bytes([data[5], data[6], data[7], data[8]]);
    Some((total_len, first_page))
}

/// Cheap hot-path predicate: does this tuple segment reference an overflow
/// chain (as opposed to being an inline tuple)?
pub fn is_overflow_ref(tuple_bytes: &[u8]) -> bool {
    tuple_bytes.len() >= LOCATOR_SIZE && tuple_bytes[0] == LOCATOR_TAG
}

/// Split `total_len` payload bytes into per-page chunk lengths, each at most
/// [`OVERFLOW_PAYLOAD_MAX`] (last page carries the remainder). Empty input
/// produces an empty plan (no pages).
pub fn chunk_plan(total_len: usize) -> Vec<usize> {
    if total_len == 0 {
        return Vec::new();
    }
    let full_pages = total_len / OVERFLOW_PAYLOAD_MAX;
    let remainder = total_len % OVERFLOW_PAYLOAD_MAX;
    let mut plan = vec![OVERFLOW_PAYLOAD_MAX; full_pages];
    if remainder > 0 {
        plan.push(remainder);
    }
    plan
}

/// Build a self-describing overflow page: header (page_type 0x04,
/// free_space_offset = payload length, next_page_id = chain successor) plus
/// the payload at offset 16.
pub fn build_overflow_page(page_id: PageId, payload: &[u8], next_page: u32) -> Result<Page> {
    if payload.len() > OVERFLOW_PAYLOAD_MAX {
        return Err(StorageError::Internal(format!(
            "overflow chunk too large: {} > {OVERFLOW_PAYLOAD_MAX}",
            payload.len()
        )));
    }
    let mut page = Page::new(page_id);
    let mut header = SlottedPageHeader::new(OVERFLOW_PAGE_TYPE);
    header.free_space_offset = payload.len() as u16;
    header.next_page_id = next_page;
    header.serialize(&mut page.data[..SlottedPageHeader::SIZE]);
    page.data[SlottedPageHeader::SIZE..SlottedPageHeader::SIZE + payload.len()]
        .copy_from_slice(payload);
    Ok(page)
}

/// Validate an overflow page image and return `(next_page_id, payload)`.
/// Errors name the page and the violated expectation (page type / payload
/// length bound).
pub fn parse_overflow_page(page_id: PageId, data: &[u8]) -> Result<(u32, &[u8])> {
    let header = SlottedPageHeader::deserialize(&data[..SlottedPageHeader::SIZE]);
    if header.page_type != OVERFLOW_PAGE_TYPE {
        return Err(StorageError::Internal(format!(
            "page {page_id} is not an overflow page (page_type {:#x}, expected {:#x})",
            header.page_type, OVERFLOW_PAGE_TYPE
        )));
    }
    let payload_len = header.free_space_offset as usize;
    if payload_len > OVERFLOW_PAYLOAD_MAX {
        return Err(StorageError::Internal(format!(
            "overflow page {page_id} declares {payload_len} payload bytes, bound is {OVERFLOW_PAYLOAD_MAX}"
        )));
    }
    Ok((
        header.next_page_id,
        &data[SlottedPageHeader::SIZE..SlottedPageHeader::SIZE + payload_len],
    ))
}

/// Write `data` as a chain of freshly allocated overflow pages and return the
/// first page id (0 only for empty input — no pages allocated; unreachable
/// through the data-page write path, where the catalog bootstrap always owns
/// pages 0–1). Any failure frees every page allocated by this call before the
/// error propagates: no locator has been placed yet, so a partial chain would
/// be unreferenced leakage.
pub async fn write_overflow_chain(buffer_pool: &BufferPool, data: &[u8]) -> Result<u32> {
    let chunks = chunk_plan(data.len());
    if chunks.is_empty() {
        return Ok(0);
    }
    let mut allocated: Vec<u32> = Vec::with_capacity(chunks.len());

    let result = write_chain_pages(buffer_pool, data, &chunks, &mut allocated).await;
    if result.is_err() {
        for raw_id in allocated.drain(..) {
            let _ = buffer_pool.free_page(PageId(raw_id as u64)).await;
        }
    }
    result
}

async fn write_chain_pages(
    buffer_pool: &BufferPool,
    data: &[u8],
    chunks: &[usize],
    allocated: &mut Vec<u32>,
) -> Result<u32> {
    // Allocate every page id up front so the chain pointers are known before
    // any page body is written; failures free the whole recorded set.
    for _ in chunks {
        let page_id = buffer_pool.storage().allocate_page().await?;
        allocated.push(page_id.0 as u32);
    }

    let mut offset = 0usize;
    for (i, &raw_id) in allocated.iter().enumerate() {
        let next = allocated.get(i + 1).copied().unwrap_or(0);
        let chunk = &data[offset..offset + chunks[i]];
        offset += chunks[i];

        let page_id = PageId(raw_id as u64);
        let page = build_overflow_page(page_id, chunk, next)?;
        let guard = buffer_pool.get_page(page_id).await?;
        guard.modify_page(|p| {
            *p = page;
        });
    }
    Ok(allocated[0])
}

/// Assemble the full payload by walking the chain starting at `first_page`.
/// Validates every page's type and payload bound, and that the accumulated
/// bytes equal the locator's declared `total_len`. A chain longer than the
/// declared total could possibly produce is reported as cyclic/corrupt.
pub async fn read_overflow_chain(
    buffer_pool: &BufferPool,
    first_page: u32,
    total_len: u32,
) -> Result<Vec<u8>> {
    if first_page == 0 {
        if total_len == 0 {
            return Ok(Vec::new());
        }
        return Err(StorageError::Internal(
            "overflow locator has first_page=0 with a nonzero total_len".into(),
        ));
    }

    // Upper bound of well-formed chain length for this declared total; the
    // walk aborts beyond it (protects against a cyclic `next_page_id`).
    let max_pages = total_len as usize / OVERFLOW_PAYLOAD_MAX + 1;
    let mut out: Vec<u8> = Vec::new();
    let mut next = Some(first_page);
    let mut visited = 0usize;
    while let Some(raw_id) = next {
        visited += 1;
        if visited > max_pages {
            return Err(StorageError::Internal(format!(
                "overflow chain from page {first_page} exceeds its declared total_len {total_len} — cyclic or corrupt"
            )));
        }
        let page_id = PageId(raw_id as u64);
        let (next_raw, chunk) = buffer_pool
            .with_page_data(page_id, |data| {
                parse_overflow_page(page_id, data).map(|(n, payload)| (n, payload.to_vec()))
            })
            .await?;
        out.extend_from_slice(&chunk);
        next = if next_raw == 0 { None } else { Some(next_raw) };
    }

    if out.len() != total_len as usize {
        return Err(StorageError::Internal(format!(
            "overflow chain from page {first_page} assembles {} bytes but the locator declares {total_len}",
            out.len()
        )));
    }
    Ok(out)
}

/// Collect the page ids of the chain starting at `first_page` (empty for 0),
/// validating each page. Cycles are rejected by id tracking.
pub async fn collect_overflow_pages(buffer_pool: &BufferPool, first_page: u32) -> Result<Vec<u32>> {
    if first_page == 0 {
        return Ok(Vec::new());
    }
    let mut pages: Vec<u32> = Vec::new();
    let mut next = Some(first_page);
    while let Some(raw_id) = next {
        if pages.contains(&raw_id) {
            return Err(StorageError::Internal(format!(
                "overflow chain from page {first_page} is cyclic at page {raw_id}"
            )));
        }
        pages.push(raw_id);
        let page_id = PageId(raw_id as u64);
        let next_raw = buffer_pool
            .with_page_data(page_id, |data| {
                parse_overflow_page(page_id, data).map(|(n, _)| n)
            })
            .await?;
        next = if next_raw == 0 { None } else { Some(next_raw) };
    }
    Ok(pages)
}

/// Free every page of the chain starting at `first_page` (no-op for 0).
pub async fn free_overflow_chain(buffer_pool: &BufferPool, first_page: u32) -> Result<()> {
    let pages = collect_overflow_pages(buffer_pool, first_page).await?;
    for raw_id in pages {
        buffer_pool.free_page(PageId(raw_id as u64)).await?;
    }
    Ok(())
}

/// Two-phase slot tuple resolution (MS27 T5/T6): what a page-lock closure
/// hands out for the tuple segment of a slot. Inline rows carry an owned copy
/// (the closure cannot hold the slice past the lock); overflow rows carry the
/// locator fields, and the payload is assembled outside the lock via
/// [`TupleRef::resolve`].
pub enum TupleRef {
    Inline(Vec<u8>),
    Overflow { total_len: u32, first_page: u32 },
}

impl TupleRef {
    /// Classify a slot's tuple segment read inside a page closure.
    pub fn from_slot_tuple(tuple_bytes: &[u8]) -> Self {
        if let Some((total_len, first_page)) = decode_locator(tuple_bytes) {
            Self::Overflow {
                total_len,
                first_page,
            }
        } else {
            Self::Inline(tuple_bytes.to_vec())
        }
    }

    /// Assemble the full tuple bytes (no-op copy for inline rows).
    pub async fn resolve(self, buffer_pool: &BufferPool) -> Result<Vec<u8>> {
        match self {
            Self::Inline(bytes) => Ok(bytes),
            Self::Overflow {
                total_len,
                first_page,
            } => read_overflow_chain(buffer_pool, first_page, total_len).await,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::async_storage::AsyncStorage;
    use crate::storage::FileStorage;
    use std::sync::Arc;
    use tempfile::tempdir;

    async fn setup_pool(capacity: usize) -> (Arc<BufferPool>, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
        let pool = Arc::new(BufferPool::new(capacity, storage.clone()).unwrap());
        // Catalog bootstrap reserves pages 0–1 (production shape): overflow
        // chains then never start at the locator's "no chain" sentinel 0.
        let _table_manager = crate::storage::data::TableManager::new(pool.clone(), storage)
            .await
            .unwrap();
        (pool, dir)
    }

    // ----- pure: locator codec -----

    #[test]
    fn locator_roundtrip() {
        for (total, first) in [
            (1u32, 7u32),
            (4075, 0xDEAD_BEEF),
            (0, 0),
            (u32::MAX, u32::MAX),
        ] {
            let enc = encode_locator(total, first);
            assert_eq!(enc.len(), LOCATOR_SIZE);
            assert_eq!(enc[0], LOCATOR_TAG);
            assert_eq!(decode_locator(&enc), Some((total, first)));
            assert!(is_overflow_ref(&enc));
        }
    }

    #[test]
    fn locator_rejects_non_locator_segments() {
        assert!(!is_overflow_ref(&[]));
        assert!(!is_overflow_ref(&[0xFF])); // too short
        assert!(!is_overflow_ref(&[0x02, 0, 0])); // inline string tuple
        assert_eq!(decode_locator(&[]), None);
        assert_eq!(decode_locator(&[0x01]), None);
        // an 8-byte 0xFF-prefixed segment is not a locator (too short)
        assert_eq!(decode_locator(&[0xFF; 8]), None);
        // inline tuples never start with the marker
        let inline = [0x01u8, 42, 0, 0, 0, 0, 0, 0, 0];
        assert!(!is_overflow_ref(&inline));
        assert_eq!(decode_locator(&inline), None);
    }

    // ----- pure: chunk plan -----

    #[test]
    fn chunk_plan_boundaries() {
        assert!(chunk_plan(0).is_empty());
        assert_eq!(chunk_plan(1), vec![1]);
        assert_eq!(chunk_plan(OVERFLOW_PAYLOAD_MAX), vec![OVERFLOW_PAYLOAD_MAX]);
        assert_eq!(
            chunk_plan(OVERFLOW_PAYLOAD_MAX + 1),
            vec![OVERFLOW_PAYLOAD_MAX, 1]
        );
        assert_eq!(
            chunk_plan(OVERFLOW_PAYLOAD_MAX * 2),
            vec![OVERFLOW_PAYLOAD_MAX, OVERFLOW_PAYLOAD_MAX]
        );
        assert_eq!(
            chunk_plan(OVERFLOW_PAYLOAD_MAX * 2 + 1),
            vec![OVERFLOW_PAYLOAD_MAX, OVERFLOW_PAYLOAD_MAX, 1]
        );
        // every chunk within bound, sum preserved
        let total = 100_003;
        let plan = chunk_plan(total);
        assert_eq!(plan.iter().sum::<usize>(), total);
        assert!(plan.iter().all(|&c| c <= OVERFLOW_PAYLOAD_MAX && c > 0));
    }

    // ----- pure: page build/parse -----

    #[test]
    fn overflow_page_build_parse_roundtrip() {
        let payload: Vec<u8> = (0..100u8).collect();
        let page = build_overflow_page(PageId(9), &payload, 777).unwrap();
        assert_eq!(page.id, PageId(9));
        let (next, got) = parse_overflow_page(PageId(9), page.data.as_ref()).unwrap();
        assert_eq!(next, 777);
        assert_eq!(got, payload.as_slice());

        let header = SlottedPageHeader::deserialize(&page.data[..SlottedPageHeader::SIZE]);
        assert_eq!(header.page_type, OVERFLOW_PAGE_TYPE);
        assert_eq!(header.free_space_offset as usize, payload.len());
        assert_eq!(header.next_page_id, 777);
        assert_eq!(header.slot_count, 0);
    }

    #[test]
    fn overflow_page_rejects_bad_images() {
        // chunk above the per-page bound is rejected at build time
        let too_big = vec![0u8; OVERFLOW_PAYLOAD_MAX + 1];
        assert!(build_overflow_page(PageId(1), &too_big, 0).is_err());

        // a data page image is not an overflow page
        let mut data = vec![0u8; Page::PAGE_SIZE];
        data[0] = 0x03;
        assert!(parse_overflow_page(PageId(1), &data).is_err());

        // declared payload beyond the page bound is rejected at parse time
        let mut bad = vec![0u8; Page::PAGE_SIZE];
        bad[0] = OVERFLOW_PAGE_TYPE;
        bad[3..5].copy_from_slice(&(OVERFLOW_PAYLOAD_MAX as u16 + 1).to_le_bytes());
        assert!(parse_overflow_page(PageId(1), &bad).is_err());
    }

    // ----- pool level: write + read chains -----

    #[tokio::test]
    async fn empty_payload_allocates_no_pages() {
        let (pool, _dir) = setup_pool(4).await;
        let first = write_overflow_chain(&pool, &[]).await.unwrap();
        assert_eq!(first, 0);
        assert_eq!(
            collect_overflow_pages(&pool, 0).await.unwrap(),
            Vec::<u32>::new()
        );
        assert_eq!(
            read_overflow_chain(&pool, 0, 0).await.unwrap(),
            Vec::<u8>::new()
        );
        free_overflow_chain(&pool, 0).await.unwrap();
    }

    #[tokio::test]
    async fn chain_write_read_roundtrip_boundaries() {
        let (pool, _dir) = setup_pool(16).await;
        for size in [
            1usize,
            OVERFLOW_PAYLOAD_MAX - 1,
            OVERFLOW_PAYLOAD_MAX,
            OVERFLOW_PAYLOAD_MAX + 1,
            OVERFLOW_PAYLOAD_MAX * 2,
            OVERFLOW_PAYLOAD_MAX * 2 + 1,
            100_003,
        ] {
            let data: Vec<u8> = (0..size).map(|i| (i % 251) as u8).collect();
            let first = write_overflow_chain(&pool, &data).await.unwrap();
            assert_ne!(first, 0, "size {size} must allocate a chain");

            let pages = collect_overflow_pages(&pool, first).await.unwrap();
            let expected_pages = chunk_plan(size).len();
            assert_eq!(pages.len(), expected_pages, "size {size}");
            assert_eq!(pages[0], first);

            let assembled = read_overflow_chain(&pool, first, size as u32)
                .await
                .unwrap();
            assert_eq!(assembled, data, "size {size}");
        }
    }

    #[tokio::test]
    async fn read_validates_declared_total() {
        let (pool, _dir) = setup_pool(8).await;
        let data = vec![7u8; 5000];
        let first = write_overflow_chain(&pool, &data).await.unwrap();

        // declared total shorter/longer than the chain is a corrupt-locator error
        let short = read_overflow_chain(&pool, first, (data.len() - 1) as u32).await;
        assert!(short.is_err(), "assembled != declared must error");
        let long = read_overflow_chain(&pool, first, (data.len() + 1) as u32).await;
        assert!(long.is_err(), "assembled != declared must error");
    }

    #[tokio::test]
    async fn free_overflow_chain_returns_pages_for_reuse() {
        let (pool, _dir) = setup_pool(8).await;
        let data = vec![1u8; OVERFLOW_PAYLOAD_MAX + 10];
        let first = write_overflow_chain(&pool, &data).await.unwrap();
        let pages = collect_overflow_pages(&pool, first).await.unwrap();
        assert_eq!(pages.len(), 2);

        free_overflow_chain(&pool, first).await.unwrap();

        // freed ids are reusable (free-list semantics) — the chain is gone
        let mut reclaimed = Vec::new();
        for _ in 0..pages.len() {
            let id = pool.storage().allocate_page().await.unwrap();
            reclaimed.push(id.0 as u32);
        }
        let mut expect = pages.clone();
        expect.sort();
        reclaimed.sort();
        assert_eq!(reclaimed, expect, "freed chain pages must be reclaimable");
    }

    // ----- failure cleanup: injected allocation failure -----

    /// AsyncStorage wrapper that fails `allocate_page` after a fixed number
    /// of successful calls; everything else delegates to a real FileStorage.
    struct FailAfterNAllocs {
        inner: FileStorage,
        remaining: std::sync::Mutex<usize>,
        successful: std::sync::Mutex<Vec<u32>>,
    }

    impl FailAfterNAllocs {
        fn new(inner: FileStorage, budget: usize) -> Self {
            Self {
                inner,
                remaining: std::sync::Mutex::new(budget),
                successful: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl AsyncStorage for FailAfterNAllocs {
        async fn read_page(&self, page_id: PageId) -> Result<Page> {
            self.inner.read_page(page_id).await
        }
        async fn write_page(&self, page_id: PageId, page: &Page) -> Result<()> {
            self.inner.write_page(page_id, page).await
        }
        async fn allocate_page(&self) -> Result<PageId> {
            {
                let mut rem = self.remaining.lock().unwrap();
                if *rem == 0 {
                    return Err(StorageError::Internal("injected allocation failure".into()));
                }
                *rem -= 1;
            }
            let id = self.inner.allocate_page().await?;
            self.successful.lock().unwrap().push(id.0 as u32);
            Ok(id)
        }
        async fn free_page(&self, page_id: PageId) -> Result<()> {
            self.inner.free_page(page_id).await
        }
        async fn sync(&self) -> Result<()> {
            self.inner.sync().await
        }
        fn page_count(&self) -> u64 {
            self.inner.page_count()
        }
    }

    #[tokio::test]
    async fn write_failure_frees_partially_allocated_chain() {
        let dir = tempdir().unwrap();
        let storage = Arc::new(FailAfterNAllocs::new(
            FileStorage::open(&dir.path().join("test.db")).unwrap(),
            1, // first page allocates, second fails
        ));
        let pool = Arc::new(BufferPool::new(8, storage.clone()).unwrap());

        // two chunks required → the second allocate fails
        let data = vec![9u8; OVERFLOW_PAYLOAD_MAX + 3];
        let result = write_overflow_chain(&pool, &data).await;
        assert!(result.is_err(), "injected failure must surface");

        // the first (successfully allocated) page was freed by cleanup:
        // a fresh allocate re-issues exactly that id. (Re-arm the budget —
        // it only gates the injected failure during the write under test.)
        let issued = storage.successful.lock().unwrap().clone();
        assert_eq!(issued.len(), 1, "only the first allocation succeeded");
        *storage.remaining.lock().unwrap() = 1;
        let reclaimed = pool.storage().allocate_page().await.unwrap();
        assert_eq!(
            reclaimed.0 as u32, issued[0],
            "cleanup must have freed the partially allocated chain page"
        );
    }
}
