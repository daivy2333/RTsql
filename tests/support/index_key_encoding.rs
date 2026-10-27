//! 索引键布局原型。只被评估测试和 benchmark 引用，不进入产品库。
//!
//! 测试和 benchmark 各自包含本文件，两边用到的函数不同。
#![allow(dead_code)]

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rtsql::storage::page_format::{PageWriteError, SlottedPage, SlottedPageRef};
use rtsql::storage::{
    AsyncStorage, BufferPool, FileStorage, Key, Page, PageId, RowId, StorageError, MAX_KEY_LEN,
};

pub const WORKLOAD_LEN: usize = 16384;
pub const HOT_SET_LEN: usize = 4096;
pub const WIDE_INT_SEED: u64 = 38;

pub const PROTO_PAGE_TYPE: u8 = 0x03;
pub const RESTART_INTERVAL: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Fixed32,
    OrderedVariable,
    PrefixRestart,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogicalValue {
    Int(i64),
    Str(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tail {
    Leaf(u32, u16),
    Internal(u32),
}

impl Tail {
    pub fn leaf(row: RowId) -> Self {
        Self::Leaf(row.page_id, row.slot_id)
    }

    pub fn kind(self) -> TailKind {
        match self {
            Self::Leaf(_, _) => TailKind::Leaf,
            Self::Internal(_) => TailKind::Internal,
        }
    }

    fn write(self, out: &mut Vec<u8>) {
        match self {
            Self::Leaf(page_id, slot_id) => {
                let mut buf = [0u8; RowId::SIZE];
                RowId::new(page_id, slot_id).serialize(&mut buf);
                out.extend_from_slice(&buf);
            }
            Self::Internal(child) => out.extend_from_slice(&child.to_le_bytes()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TailKind {
    Leaf,
    Internal,
}

impl TailKind {
    fn len(self) -> usize {
        match self {
            Self::Leaf => RowId::SIZE,
            Self::Internal => 4,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub value: LogicalValue,
    pub tail: Tail,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotLoc {
    pub page_index: usize,
    pub slot_index: usize,
}

#[derive(Clone, Debug)]
pub struct PackedLoad {
    pub pages: Vec<Vec<u8>>,
    pub locations: Vec<SlotLoc>,
    pub payload_bytes: u64,
    pub slot_bytes: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CandidateError {
    Unsupported,
    LengthExceedsU16,
    RecordDoesNotFit,
    Truncated,
    LostRestart,
    BadEncoding,
}

pub fn encode_ordered_int(value: i64) -> Vec<u8> {
    if value == 0 {
        return vec![0x80];
    }
    if value > 0 {
        let magnitude = magnitude_bytes(value as u64);
        let mut out = Vec::with_capacity(1 + magnitude.len());
        out.push(0x80 + magnitude.len() as u8);
        out.extend_from_slice(&magnitude);
        out
    } else {
        let magnitude = magnitude_bytes(value.unsigned_abs());
        let mut out = Vec::with_capacity(1 + magnitude.len());
        out.push(0x80 - magnitude.len() as u8);
        for byte in magnitude {
            out.push(!byte);
        }
        out
    }
}

pub fn decode_ordered_int(bytes: &[u8]) -> Result<i64, CandidateError> {
    if bytes.is_empty() {
        return Err(CandidateError::Truncated);
    }
    let header = bytes[0];
    if header == 0x80 {
        if bytes.len() != 1 {
            return Err(CandidateError::BadEncoding);
        }
        return Ok(0);
    }
    if (0x81..=0x88).contains(&header) {
        let len = (header - 0x80) as usize;
        if bytes.len() != 1 + len || bytes[1] == 0 {
            return Err(CandidateError::BadEncoding);
        }
        let magnitude = read_magnitude(&bytes[1..])?;
        if magnitude > i64::MAX as u64 {
            return Err(CandidateError::BadEncoding);
        }
        return Ok(magnitude as i64);
    }
    if (0x78..=0x7F).contains(&header) {
        let len = (0x80 - header) as usize;
        if bytes.len() != 1 + len || bytes[1] == 0xFF {
            return Err(CandidateError::BadEncoding);
        }
        let mut inverted = bytes[1..].to_vec();
        for byte in &mut inverted {
            *byte = !*byte;
        }
        let magnitude = read_magnitude(&inverted)?;
        if magnitude > i64::MIN.unsigned_abs() {
            return Err(CandidateError::BadEncoding);
        }
        if magnitude == i64::MIN.unsigned_abs() {
            return Ok(i64::MIN);
        }
        return Ok(-(magnitude as i64));
    }
    Err(CandidateError::BadEncoding)
}

pub fn encode_ordered_str(value: &str) -> Result<Vec<u8>, CandidateError> {
    let raw = value.as_bytes();
    let mut out = Vec::with_capacity(raw.len() + 2);
    for byte in raw {
        if *byte == 0 {
            out.push(0x00);
            out.push(0xFF);
        } else {
            out.push(*byte);
        }
    }
    out.push(0x00);
    out.push(0x00);
    if out.len() > u16::MAX as usize {
        return Err(CandidateError::LengthExceedsU16);
    }
    Ok(out)
}

pub fn decode_ordered_str(bytes: &[u8]) -> Result<String, CandidateError> {
    let (text, used) = decode_escaped_prefix(bytes)?;
    if used != bytes.len() {
        return Err(CandidateError::BadEncoding);
    }
    Ok(text)
}

pub fn fixed32_key_bytes(value: &LogicalValue) -> Result<[u8; MAX_KEY_LEN], CandidateError> {
    match value {
        LogicalValue::Int(n) => Ok(*Key::new(&n.to_be_bytes()).full_data()),
        LogicalValue::Str(text) => {
            let escaped = encode_ordered_str(text)?;
            if escaped.len() > MAX_KEY_LEN {
                return Err(CandidateError::Unsupported);
            }
            Ok(*Key::new(&escaped).full_data())
        }
    }
}

pub fn decode_fixed32_string(key: &[u8; MAX_KEY_LEN]) -> Result<String, CandidateError> {
    let (text, used) = decode_escaped_prefix(key)?;
    if key[used..].iter().any(|byte| *byte != 0) {
        return Err(CandidateError::BadEncoding);
    }
    Ok(text)
}

pub fn ordered_key_bytes(value: &LogicalValue) -> Result<Vec<u8>, CandidateError> {
    match value {
        LogicalValue::Int(n) => Ok(encode_ordered_int(*n)),
        LogicalValue::Str(text) => encode_ordered_str(text),
    }
}

pub fn encode_for_layout(layout: Layout, value: &LogicalValue) -> Result<Vec<u8>, CandidateError> {
    match layout {
        Layout::Fixed32 => Ok(fixed32_key_bytes(value)?.to_vec()),
        Layout::OrderedVariable | Layout::PrefixRestart => ordered_key_bytes(value),
    }
}

pub fn pack_prefix_ints(values: &[i64]) -> Result<Vec<Vec<u8>>, CandidateError> {
    let records = values
        .iter()
        .enumerate()
        .map(|(index, value)| Record {
            value: LogicalValue::Int(*value),
            tail: Tail::leaf(RowId::new(index as u32, index as u16)),
        })
        .collect::<Vec<_>>();
    Ok(pack_records(Layout::PrefixRestart, &records)?.pages)
}

pub fn pack_records(layout: Layout, records: &[Record]) -> Result<PackedLoad, CandidateError> {
    if records.is_empty() {
        return Ok(PackedLoad {
            pages: Vec::new(),
            locations: Vec::new(),
            payload_bytes: 0,
            slot_bytes: 0,
        });
    }
    let tail_kind = records[0].tail.kind();
    let domain = domain_of(&records[0].value);
    for record in records {
        if record.tail.kind() != tail_kind || domain_of(&record.value) != domain {
            return Err(CandidateError::BadEncoding);
        }
    }

    let mut pages = Vec::new();
    let mut locations = Vec::with_capacity(records.len());
    let mut payload_bytes = 0u64;
    let mut page = new_proto_page(0);
    let mut page_keys: Vec<Vec<u8>> = Vec::new();

    for record in records {
        let complete = complete_key(layout, &record.value)?;
        let placed = place_record(layout, &complete, record.tail, &mut page, &mut page_keys)?;
        payload_bytes += placed.payload_len as u64;
        if placed.moved_to_new_page {
            pages.push(placed.finished_page);
        }
        locations.push(SlotLoc {
            page_index: pages.len(),
            slot_index: placed.slot_index,
        });
        page_keys = placed.page_keys;
        page = placed.page;
    }
    if !page_keys.is_empty() {
        pages.push(page.data.to_vec());
    }
    let slot_bytes = locations.len() as u64 * 6;
    Ok(PackedLoad {
        pages,
        locations,
        payload_bytes,
        slot_bytes,
    })
}

pub fn prefix_slot_prefix_len(page: &[u8], slot_index: usize) -> Result<u16, CandidateError> {
    let data = slot_payload(page, slot_index)?;
    if data.len() < 2 {
        return Err(CandidateError::Truncated);
    }
    Ok(u16::from_le_bytes([data[0], data[1]]))
}

pub fn read_prefix_int_slot(page: &[u8], slot_index: usize) -> Result<i64, CandidateError> {
    match read_slot(
        Layout::PrefixRestart,
        Domain::Int,
        TailKind::Leaf,
        page,
        slot_index,
    )? {
        (LogicalValue::Int(value), _) => Ok(value),
        _ => Err(CandidateError::BadEncoding),
    }
}

pub fn read_prefix_step(
    domain: Domain,
    tail_kind: TailKind,
    page: &[u8],
    slot_index: usize,
    previous: Option<&[u8]>,
) -> Result<(LogicalValue, Tail, Vec<u8>), CandidateError> {
    let data = slot_payload(page, slot_index)?;
    decode_prefix_record(domain, tail_kind, data, previous, slot_index)
}

pub fn read_slot(
    layout: Layout,
    domain: Domain,
    tail_kind: TailKind,
    page: &[u8],
    slot_index: usize,
) -> Result<(LogicalValue, Tail), CandidateError> {
    match layout {
        Layout::PrefixRestart => read_prefix_slot(domain, tail_kind, page, slot_index),
        Layout::Fixed32 | Layout::OrderedVariable => {
            let data = slot_payload(page, slot_index)?;
            decode_record(layout, domain, tail_kind, data, None)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain {
    Int,
    Str,
}

pub fn byte_cmp(left: &[u8], right: &[u8]) -> std::cmp::Ordering {
    left.cmp(right)
}

pub fn page_free_space(page: &[u8]) -> Result<usize, CandidateError> {
    if page.len() != Page::PAGE_SIZE {
        return Err(CandidateError::Truncated);
    }
    let view = SlottedPageRef::new(page);
    if view.slot_count_exceeds_capacity() {
        return Err(CandidateError::BadEncoding);
    }
    let start = view.header().free_space_offset as usize;
    let end = Page::PAGE_SIZE - view.slot_count() * 6;
    Ok(end.saturating_sub(start))
}

pub struct CountingStorage {
    inner: FileStorage,
    reads: AtomicU64,
}

impl CountingStorage {
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        Ok(Self {
            inner: FileStorage::open(path)?,
            reads: AtomicU64::new(0),
        })
    }

    pub fn read_count(&self) -> u64 {
        self.reads.load(Ordering::SeqCst)
    }

    pub fn reset_reads(&self) {
        self.reads.store(0, Ordering::SeqCst);
    }
}

#[async_trait]
impl AsyncStorage for CountingStorage {
    async fn read_page(&self, page_id: PageId) -> rtsql::storage::Result<Page> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.inner.read_page(page_id).await
    }

    async fn write_page(&self, page_id: PageId, page: &Page) -> rtsql::storage::Result<()> {
        self.inner.write_page(page_id, page).await
    }

    async fn allocate_page(&self) -> rtsql::storage::Result<PageId> {
        self.inner.allocate_page().await
    }

    async fn free_page(&self, page_id: PageId) -> rtsql::storage::Result<()> {
        self.inner.free_page(page_id).await
    }

    async fn sync(&self) -> rtsql::storage::Result<()> {
        self.inner.sync().await
    }

    fn page_count(&self) -> u64 {
        self.inner.page_count()
    }
}

pub async fn write_pages(
    storage: &CountingStorage,
    pages: &[Vec<u8>],
) -> Result<Vec<PageId>, StorageError> {
    let mut ids = Vec::with_capacity(pages.len());
    for bytes in pages {
        let id = storage.allocate_page().await?;
        let mut page = Page::new(id);
        page.data.copy_from_slice(bytes);
        storage.write_page(id, &page).await?;
        ids.push(id);
    }
    storage.sync().await?;
    Ok(ids)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheStats {
    pub requests: u64,
    pub reads: u64,
    pub hits: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadReport {
    pub sequential: CacheStats,
    pub hot_warmup: CacheStats,
    pub hot_repeat: CacheStats,
}

#[derive(Debug)]
pub enum MeasureError {
    Candidate(CandidateError),
    Storage(String),
}

impl std::fmt::Display for MeasureError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Candidate(err) => write!(formatter, "candidate {err:?}"),
            Self::Storage(err) => write!(formatter, "storage {err}"),
        }
    }
}

pub fn values_short_int() -> Vec<LogicalValue> {
    (1..=WORKLOAD_LEN as i64).map(LogicalValue::Int).collect()
}

pub fn values_symmetric_int() -> Vec<LogicalValue> {
    (-8192..8192).map(LogicalValue::Int).collect()
}

pub fn values_wide_int() -> Vec<LogicalValue> {
    let mut rng = StdRng::seed_from_u64(WIDE_INT_SEED);
    let mut values = (0..WORKLOAD_LEN)
        .map(|_| rng.gen::<i64>())
        .collect::<Vec<_>>();
    for (index, value) in [i64::MIN, i64::MAX, -256, -255, -1, 0, 1, 255, 256]
        .into_iter()
        .enumerate()
    {
        values[index] = value;
    }
    values.into_iter().map(LogicalValue::Int).collect()
}

pub fn values_common_string() -> Vec<LogicalValue> {
    let mut values = Vec::with_capacity(WORKLOAD_LEN);
    for index in 0..WORKLOAD_LEN / 2 {
        let token = index % (WORKLOAD_LEN / 4);
        values.push(LogicalValue::Str(format!("前{token:05x}")));
        values.push(LogicalValue::Str(format!("你common-prefix-{token:07x}")));
    }
    values
}

pub fn values_long_string(len: usize) -> Vec<LogicalValue> {
    const PREFIX: &str = "long-shared-prefix";
    (0..WORKLOAD_LEN)
        .map(|index| {
            let rest = len - PREFIX.len();
            let suffix = format!("{index:0width$x}", width = rest);
            LogicalValue::Str(format!("{PREFIX}{suffix}"))
        })
        .collect()
}

pub fn records_for(values: &[LogicalValue], kind: TailKind) -> Vec<Record> {
    let mut indexed = values
        .iter()
        .cloned()
        .enumerate()
        .collect::<Vec<(usize, LogicalValue)>>();
    indexed.sort_by(|left, right| left.1.cmp(&right.1).then(left.0.cmp(&right.0)));
    indexed
        .into_iter()
        .map(|(origin, value)| Record {
            value,
            tail: match kind {
                TailKind::Leaf => Tail::Leaf(origin as u32, origin as u16),
                TailKind::Internal => Tail::Internal(origin as u32),
            },
        })
        .collect()
}

pub async fn measure_file_reads(
    packed: &PackedLoad,
    records: &[Record],
    layout: Layout,
    capacities: &[usize],
) -> Result<Vec<ReadReport>, MeasureError> {
    let dir = tempfile::tempdir().map_err(|err| MeasureError::Storage(err.to_string()))?;
    let storage = Arc::new(
        CountingStorage::open(&dir.path().join("pages"))
            .map_err(|err| MeasureError::Storage(err.to_string()))?,
    );
    let ids = write_pages(&storage, &packed.pages)
        .await
        .map_err(|err| MeasureError::Storage(err.to_string()))?;
    let accesses = packed
        .locations
        .iter()
        .enumerate()
        .map(|(index, loc)| (ids[loc.page_index], loc.slot_index, index))
        .collect::<Vec<_>>();
    let hot_len = HOT_SET_LEN.min(accesses.len());
    let mut reports = Vec::with_capacity(capacities.len());
    for capacity in capacities {
        let sequential = {
            let pool = new_pool(&storage, *capacity)?;
            storage.reset_reads();
            scan_loaded(&storage, &pool, layout, records, &accesses, 1).await?
        };
        let (hot_warmup, hot_repeat) = {
            let pool = new_pool(&storage, *capacity)?;
            storage.reset_reads();
            let warmup =
                scan_loaded(&storage, &pool, layout, records, &accesses[..hot_len], 1).await?;
            storage.reset_reads();
            let repeat =
                scan_loaded(&storage, &pool, layout, records, &accesses[..hot_len], 4).await?;
            (warmup, repeat)
        };
        reports.push(ReadReport {
            sequential,
            hot_warmup,
            hot_repeat,
        });
    }
    Ok(reports)
}

fn new_pool(storage: &Arc<CountingStorage>, capacity: usize) -> Result<BufferPool, MeasureError> {
    BufferPool::new(capacity, storage.clone()).map_err(|err| MeasureError::Storage(err.to_string()))
}

async fn scan_loaded(
    storage: &CountingStorage,
    pool: &BufferPool,
    layout: Layout,
    records: &[Record],
    accesses: &[(PageId, usize, usize)],
    repeats: usize,
) -> Result<CacheStats, MeasureError> {
    let domain = domain_of(&records[0].value);
    let kind = records[0].tail.kind();
    let mut requests = 0u64;
    for _ in 0..repeats {
        for (page_id, slot_index, record_index) in accesses {
            let guard = pool
                .get_page(*page_id)
                .await
                .map_err(|err| MeasureError::Storage(err.to_string()))?;
            let view = guard.page_data();
            let (value, tail) = read_slot(layout, domain, kind, &view, *slot_index)
                .map_err(MeasureError::Candidate)?;
            if value != records[*record_index].value || tail != records[*record_index].tail {
                return Err(MeasureError::Candidate(CandidateError::BadEncoding));
            }
            requests += 1;
        }
    }
    let reads = storage.read_count();
    Ok(CacheStats {
        requests,
        reads,
        hits: requests.saturating_sub(reads),
    })
}

fn domain_of(value: &LogicalValue) -> Domain {
    match value {
        LogicalValue::Int(_) => Domain::Int,
        LogicalValue::Str(_) => Domain::Str,
    }
}

fn magnitude_bytes(magnitude: u64) -> Vec<u8> {
    let be = magnitude.to_be_bytes();
    let start = be.iter().position(|byte| *byte != 0).expect("non-zero");
    be[start..].to_vec()
}

fn read_magnitude(bytes: &[u8]) -> Result<u64, CandidateError> {
    if bytes.is_empty() || bytes.len() > 8 || bytes[0] == 0 {
        return Err(CandidateError::BadEncoding);
    }
    let mut buf = [0u8; 8];
    buf[8 - bytes.len()..].copy_from_slice(bytes);
    Ok(u64::from_be_bytes(buf))
}

fn decode_escaped_prefix(bytes: &[u8]) -> Result<(String, usize), CandidateError> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != 0 {
            out.push(bytes[index]);
            index += 1;
            continue;
        }
        if index + 1 >= bytes.len() {
            return Err(CandidateError::Truncated);
        }
        match bytes[index + 1] {
            0x00 => {
                let text = String::from_utf8(out).map_err(|_| CandidateError::BadEncoding)?;
                return Ok((text, index + 2));
            }
            0xFF => {
                out.push(0);
                index += 2;
            }
            _ => return Err(CandidateError::BadEncoding),
        }
    }
    Err(CandidateError::Truncated)
}

fn complete_key(layout: Layout, value: &LogicalValue) -> Result<Vec<u8>, CandidateError> {
    encode_for_layout(layout, value)
}

struct Placed {
    page: Page,
    page_keys: Vec<Vec<u8>>,
    slot_index: usize,
    payload_len: usize,
    moved_to_new_page: bool,
    finished_page: Vec<u8>,
}

fn place_record(
    layout: Layout,
    complete: &[u8],
    tail: Tail,
    page: &mut Page,
    page_keys: &mut Vec<Vec<u8>>,
) -> Result<Placed, CandidateError> {
    let bytes = encode_record(layout, complete, page_keys, tail)?;
    match try_add(page, &bytes) {
        Ok(slot_index) => {
            let mut keys = std::mem::take(page_keys);
            keys.push(complete.to_vec());
            Ok(Placed {
                page: std::mem::replace(page, new_proto_page(0)),
                page_keys: keys,
                slot_index,
                payload_len: bytes.len(),
                moved_to_new_page: false,
                finished_page: Vec::new(),
            })
        }
        Err(PageWriteError::PageFull) if page_keys.is_empty() => {
            Err(CandidateError::RecordDoesNotFit)
        }
        Err(PageWriteError::PageFull) => {
            let finished_page = page.data.to_vec();
            let mut fresh = new_proto_page(0);
            let mut fresh_keys = Vec::new();
            let restarted = encode_record(layout, complete, &fresh_keys, tail)?;
            match try_add(&mut fresh, &restarted) {
                Ok(slot_index) => {
                    fresh_keys.push(complete.to_vec());
                    Ok(Placed {
                        page: fresh,
                        page_keys: fresh_keys,
                        slot_index,
                        payload_len: restarted.len(),
                        moved_to_new_page: true,
                        finished_page,
                    })
                }
                Err(PageWriteError::PageFull) => Err(CandidateError::RecordDoesNotFit),
                Err(_) => Err(CandidateError::BadEncoding),
            }
        }
        Err(_) => Err(CandidateError::BadEncoding),
    }
}

fn encode_record(
    layout: Layout,
    complete: &[u8],
    page_keys: &[Vec<u8>],
    tail: Tail,
) -> Result<Vec<u8>, CandidateError> {
    if complete.len() > u16::MAX as usize {
        return Err(CandidateError::LengthExceedsU16);
    }
    let mut out = Vec::new();
    match layout {
        Layout::Fixed32 => {
            if complete.len() != MAX_KEY_LEN {
                return Err(CandidateError::BadEncoding);
            }
            out.extend_from_slice(complete);
        }
        Layout::OrderedVariable => {
            out.extend_from_slice(&(complete.len() as u16).to_le_bytes());
            out.extend_from_slice(complete);
        }
        Layout::PrefixRestart => {
            let restart = page_keys.is_empty() || page_keys.len().is_multiple_of(RESTART_INTERVAL);
            let (prefix_len, suffix) = if restart {
                (0usize, complete)
            } else {
                let previous = page_keys.last().map(Vec::as_slice).unwrap_or(&[]);
                let prefix_len = common_prefix_len(previous, complete);
                (prefix_len, &complete[prefix_len..])
            };
            if suffix.len() > u16::MAX as usize {
                return Err(CandidateError::LengthExceedsU16);
            }
            out.extend_from_slice(&(prefix_len as u16).to_le_bytes());
            out.extend_from_slice(&(suffix.len() as u16).to_le_bytes());
            out.extend_from_slice(suffix);
        }
    }
    tail.write(&mut out);
    if out.len() > max_record_payload() {
        return Err(CandidateError::RecordDoesNotFit);
    }
    Ok(out)
}

fn common_prefix_len(left: &[u8], right: &[u8]) -> usize {
    left.iter()
        .zip(right.iter())
        .take_while(|(a, b)| a == b)
        .count()
}

fn max_record_payload() -> usize {
    Page::PAGE_SIZE - 16 - 6
}

fn new_proto_page(index: u64) -> Page {
    let mut page = Page::new(PageId(index));
    let _ = SlottedPage::init(&mut page, PROTO_PAGE_TYPE);
    page
}

fn try_add(page: &mut Page, bytes: &[u8]) -> Result<usize, PageWriteError> {
    SlottedPage::new(page)
        .add_slot(bytes)
        .map(|(_, slot_index)| slot_index)
}

fn slot_payload(page: &[u8], slot_index: usize) -> Result<&[u8], CandidateError> {
    if page.len() != Page::PAGE_SIZE {
        return Err(CandidateError::Truncated);
    }
    let view = SlottedPageRef::new(page);
    if view.slot_count_exceeds_capacity() {
        return Err(CandidateError::BadEncoding);
    }
    let slot = view
        .get_slot(slot_index)
        .ok_or(CandidateError::BadEncoding)?;
    view.get_slot_data_checked(&slot)
        .ok_or(CandidateError::Truncated)
}

fn read_prefix_slot(
    domain: Domain,
    tail_kind: TailKind,
    page: &[u8],
    slot_index: usize,
) -> Result<(LogicalValue, Tail), CandidateError> {
    let restart = slot_index - (slot_index % RESTART_INTERVAL);
    let mut previous: Option<Vec<u8>> = None;
    let mut last = None;
    for index in restart..=slot_index {
        let data = slot_payload(page, index)?;
        let (value, tail, key) =
            decode_prefix_record(domain, tail_kind, data, previous.as_deref(), index)?;
        previous = Some(key);
        last = Some((value, tail));
    }
    last.ok_or(CandidateError::BadEncoding)
}

fn decode_prefix_record(
    domain: Domain,
    tail_kind: TailKind,
    data: &[u8],
    previous: Option<&[u8]>,
    index: usize,
) -> Result<(LogicalValue, Tail, Vec<u8>), CandidateError> {
    let tail_len = tail_kind.len();
    if data.len() < 4 + tail_len {
        return Err(CandidateError::Truncated);
    }
    let prefix_len = u16::from_le_bytes([data[0], data[1]]);
    let suffix_len = u16::from_le_bytes([data[2], data[3]]) as usize;
    let suffix_end = 4usize
        .checked_add(suffix_len)
        .ok_or(CandidateError::Truncated)?;
    let total = suffix_end
        .checked_add(tail_len)
        .ok_or(CandidateError::Truncated)?;
    if total > data.len() {
        return Err(CandidateError::Truncated);
    }
    if total != data.len() {
        return Err(CandidateError::BadEncoding);
    }
    if index.is_multiple_of(RESTART_INTERVAL) && prefix_len != 0 {
        return Err(CandidateError::LostRestart);
    }
    let suffix = &data[4..suffix_end];
    let key = if prefix_len == 0 {
        suffix.to_vec()
    } else {
        let previous = previous.ok_or(CandidateError::LostRestart)?;
        let prefix_len = prefix_len as usize;
        if prefix_len > previous.len() {
            return Err(CandidateError::LostRestart);
        }
        let mut key = Vec::with_capacity(prefix_len + suffix.len());
        key.extend_from_slice(&previous[..prefix_len]);
        key.extend_from_slice(suffix);
        key
    };
    let value = decode_domain(domain, &key)?;
    let tail = parse_tail(tail_kind, &data[suffix_end..total])?;
    Ok((value, tail, key))
}

fn decode_record(
    layout: Layout,
    domain: Domain,
    tail_kind: TailKind,
    data: &[u8],
    previous: Option<&[u8]>,
) -> Result<(LogicalValue, Tail), CandidateError> {
    match layout {
        Layout::PrefixRestart => {
            let (value, tail, _) = decode_prefix_record(domain, tail_kind, data, previous, 0)?;
            Ok((value, tail))
        }
        Layout::Fixed32 => {
            let tail_len = tail_kind.len();
            if data.len() < MAX_KEY_LEN + tail_len {
                return Err(CandidateError::Truncated);
            }
            if data.len() != MAX_KEY_LEN + tail_len {
                return Err(CandidateError::BadEncoding);
            }
            let mut key = [0u8; MAX_KEY_LEN];
            key.copy_from_slice(&data[..MAX_KEY_LEN]);
            let value = decode_fixed_domain(domain, &key)?;
            let tail = parse_tail(tail_kind, &data[MAX_KEY_LEN..])?;
            Ok((value, tail))
        }
        Layout::OrderedVariable => {
            let tail_len = tail_kind.len();
            if data.len() < 2 + tail_len {
                return Err(CandidateError::Truncated);
            }
            let key_len = u16::from_le_bytes([data[0], data[1]]) as usize;
            let key_end = 2usize
                .checked_add(key_len)
                .ok_or(CandidateError::Truncated)?;
            let total = key_end
                .checked_add(tail_len)
                .ok_or(CandidateError::Truncated)?;
            if total > data.len() {
                return Err(CandidateError::Truncated);
            }
            if total != data.len() {
                return Err(CandidateError::BadEncoding);
            }
            let value = decode_domain(domain, &data[2..key_end])?;
            let tail = parse_tail(tail_kind, &data[key_end..total])?;
            Ok((value, tail))
        }
    }
}

fn decode_domain(domain: Domain, key: &[u8]) -> Result<LogicalValue, CandidateError> {
    match domain {
        Domain::Int => Ok(LogicalValue::Int(decode_ordered_int(key)?)),
        Domain::Str => Ok(LogicalValue::Str(decode_ordered_str(key)?)),
    }
}

fn decode_fixed_domain(
    domain: Domain,
    key: &[u8; MAX_KEY_LEN],
) -> Result<LogicalValue, CandidateError> {
    match domain {
        Domain::Int => {
            let mut be = [0u8; 8];
            be.copy_from_slice(&key[..8]);
            if key[8..].iter().any(|byte| *byte != 0) {
                return Err(CandidateError::BadEncoding);
            }
            Ok(LogicalValue::Int(i64::from_be_bytes(be)))
        }
        Domain::Str => Ok(LogicalValue::Str(decode_fixed32_string(key)?)),
    }
}

fn parse_tail(kind: TailKind, bytes: &[u8]) -> Result<Tail, CandidateError> {
    match kind {
        TailKind::Leaf => {
            if bytes.len() != RowId::SIZE {
                return Err(CandidateError::Truncated);
            }
            let row = RowId::deserialize(bytes);
            Ok(Tail::Leaf(row.page_id, row.slot_id))
        }
        TailKind::Internal => {
            if bytes.len() != 4 {
                return Err(CandidateError::Truncated);
            }
            let mut buf = [0u8; 4];
            buf.copy_from_slice(bytes);
            Ok(Tail::Internal(u32::from_le_bytes(buf)))
        }
    }
}
