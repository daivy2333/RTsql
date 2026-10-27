use std::fmt;

use crate::storage::Page;

/// Slot: pointing to row data in the page
#[derive(Debug, Clone, Copy)]
pub struct Slot {
    pub logical_id: u16, // Logical ID (stable across compact)
    pub offset: u16,     // Offset into Row Data area
    pub length: u16,     // Row length
}

impl Slot {
    pub const SIZE: usize = 6; // u16 + u16 + u16
}

/// 页格式层的**结构化写入失败**——区分「页满（可换页继续）」与「页损坏
/// （不可重试）」与「等长不符」。
///
/// 既有写入原语以 `String` 表达失败 ⇒ 调用点**无法**区分「页满」与「页
/// 损坏」⇒ 损坏页上的落位失败被当作「页满」⇒ 换新页继续写入（损坏页被
/// 跳过、数据继续落盘）。本类型让写侧能够分流。
///
/// 既有错误文案**逐字**保持（页满 / 等长不符 / 槽下标越界 / logical_id
/// 不存在）⇒ 索引树侧（`StorageError::Io(std::io::Error::other(e))`）与
/// 恢复面（`format!("add_slot failed: {}", e)`）的既有映射逐字继续编译。
#[derive(Debug, Clone)]
pub enum PageWriteError {
    /// 页满——调用方可换新页继续。
    PageFull,
    /// 页头声称的槽数超出槽目录容量（页级损坏）。
    SlotCountExceedsCapacity { slot_count: usize },
    /// 槽负载区间越出页的数据区。
    SlotPayloadOutOfBounds { slot_index: usize, slot: Slot },
    /// 原地改写要求等长。
    LengthMismatch {
        slot_index: usize,
        slot_length: usize,
        write_length: usize,
    },
    /// 槽下标越界。
    SlotIndexOutOfRange,
    /// logical_id 不存在。
    LogicalIdNotFound { logical_id: u16 },
}

impl fmt::Display for PageWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PageFull => write!(f, "No enough space in page"),
            Self::SlotCountExceedsCapacity { slot_count } => write!(
                f,
                "corrupted page: slot count {slot_count} exceeds the page slot directory \
                 capacity of {MAX_PAGE_SLOTS} slots"
            ),
            Self::SlotPayloadOutOfBounds { slot_index, slot } => {
                let start = slot.offset as usize;
                let end = start + slot.length as usize;
                write!(
                    f,
                    "slot {slot_index} range [{start}, {end}) lies outside the page data area"
                )
            }
            Self::LengthMismatch {
                slot_length,
                write_length,
                ..
            } => write!(
                f,
                "in-place update requires equal length: slot {slot_length} bytes, \
                 got {write_length} bytes"
            ),
            Self::SlotIndexOutOfRange => write!(f, "Slot index out of range"),
            Self::LogicalIdNotFound { logical_id } => {
                write!(f, "logical_id {logical_id} not found")
            }
        }
    }
}

impl std::error::Error for PageWriteError {}

/// 页内槽目录的最大条目数——槽目录自页尾向上生长（每项 `Slot::SIZE` 字节），
/// 不得触及页头。页头声称的槽数超过该上限时，槽目录定位会下溢成越界切片，
/// 故访问器在读槽**之前**先做此界限判断。
///
/// 全项目**唯一**定义（页格式层）：索引树访问器的读槽界限与数据页组的
/// 页级判据共用它（取值不变 ⇒ 索引树路径零回归）。
pub const MAX_PAGE_SLOTS: usize = (Page::PAGE_SIZE - SlottedPageHeader::SIZE) / Slot::SIZE;

/// 槽负载是否完整落在页的数据区内——**读侧与写侧共用**的唯一判界实现
/// （数据页路径）。
///
/// 语义与既有读侧私有判据**逐字**不变：`start >= SlottedPageHeader::SIZE
/// && end <= slot_area_start`，其中 `slot_area_start =
/// PAGE_SIZE.saturating_sub(slot_count * Slot::SIZE)`——槽目录自页尾向上
/// 生长 ⇒ 负载既不得与页头重叠，也不得触及槽目录区。
///
/// 读侧（`SlottedPageRef::get_slot_data_checked`）与写侧（版本头原地写 /
/// 槽原地改写）共用它 ⇒ 两侧的界限口径不会漂移。
pub fn slot_payload_in_bounds(slot_count: usize, start: usize, end: usize) -> bool {
    let slot_area_start = Page::PAGE_SIZE.saturating_sub(slot_count * Slot::SIZE);
    start >= SlottedPageHeader::SIZE && end <= slot_area_start
}

/// Slotted Page Header (16 bytes)
#[derive(Debug, Clone, Copy)]
pub struct SlottedPageHeader {
    pub page_type: u8,          // 0x01=Leaf, 0x02=Internal, 0x03=Data
    pub slot_count: u16,        // Current number of slots
    pub free_space_offset: u16, // Start of Row Data area (after header)
    pub next_page_id: u32,      // Next page ID (for linked list)
    pub next_logical_id: u16,   // Next logical ID to allocate
    _padding: [u8; 3],          // Padding to 16 bytes
}

impl SlottedPageHeader {
    pub const SIZE: usize = 16;

    pub fn new(page_type: u8) -> Self {
        Self {
            page_type,
            slot_count: 0,
            free_space_offset: Self::SIZE as u16,
            next_page_id: 0,
            next_logical_id: 0,
            _padding: [0; 3],
        }
    }

    /// Serialize to byte slice
    pub fn serialize(&self, buf: &mut [u8]) {
        buf[0] = self.page_type;
        buf[1..3].copy_from_slice(&self.slot_count.to_le_bytes());
        buf[3..5].copy_from_slice(&self.free_space_offset.to_le_bytes());
        buf[5..9].copy_from_slice(&self.next_page_id.to_le_bytes());
        buf[9..11].copy_from_slice(&self.next_logical_id.to_le_bytes());
        buf[11..14].copy_from_slice(&self._padding);
    }

    /// Deserialize from byte slice
    pub fn deserialize(buf: &[u8]) -> Self {
        let page_type = buf[0];
        let slot_count = u16::from_le_bytes([buf[1], buf[2]]);
        let free_space_offset = u16::from_le_bytes([buf[3], buf[4]]);
        let next_page_id = u32::from_le_bytes([buf[5], buf[6], buf[7], buf[8]]);
        let next_logical_id = u16::from_le_bytes([buf[9], buf[10]]);
        let _padding = buf[11..14].try_into().unwrap();

        Self {
            page_type,
            slot_count,
            free_space_offset,
            next_page_id,
            next_logical_id,
            _padding,
        }
    }

    /// **页级判据**：页头声称的槽数是否超出该页的槽目录容量。
    ///
    /// 阈值与 `integrity::validate_slots` 的 `slot count exceeds the page
    /// capacity` violation **同源**（`slot_count > MAX_PAGE_SLOTS`，即
    /// `slot_count ≥ 681`）⇒ 同时覆盖「槽目录压到页头」与「槽目录定位下溢」
    /// 两种页头损坏形态。
    ///
    /// **任何按页头 `slot_count` 迭代槽的读点都必须先问这个判据**：页头
    /// 声称的槽数越界时，`get_slot` 的槽目录定位
    /// `PAGE_SIZE - (index + 1) * Slot::SIZE` 会整数下溢 ⇒ 越界切片 panic
    /// ⇒ 进程 abort（`get_slot` 本身**不改**其 `None` 语义——把越界折叠成
    /// `None` 会让「页损坏」与「槽不存在」不可区分 ⇒ 静默跳过 = 静默错读）。
    pub fn slot_count_exceeds_capacity(&self) -> bool {
        self.slot_count as usize > MAX_PAGE_SLOTS
    }
}

/// Slotted Page format reader/writer
pub struct SlottedPage<'a> {
    pub(crate) page: &'a mut Page,
    header: SlottedPageHeader,
}

/// Read-only slotted page accessor from raw bytes (zero-copy).
/// Used with PageGuard::page_data() to avoid 4KB page clone.
pub struct SlottedPageRef<'a> {
    data: &'a [u8],
    header: SlottedPageHeader,
}

impl<'a> SlottedPageRef<'a> {
    /// Create a read-only SlottedPageRef from page data bytes.
    pub fn new(data: &'a [u8]) -> Self {
        let header = SlottedPageHeader::deserialize(&data[..SlottedPageHeader::SIZE]);
        Self { data, header }
    }

    /// Get slot count
    pub fn slot_count(&self) -> usize {
        self.header.slot_count as usize
    }

    /// **页级判据**（见 [`SlottedPageHeader::slot_count_exceeds_capacity`]）——
    /// 页头声称的槽数是否超出该页的槽目录容量。取槽**之前**必须先问它。
    pub fn slot_count_exceeds_capacity(&self) -> bool {
        self.header.slot_count_exceeds_capacity()
    }

    /// Get a specific slot (read-only)
    ///
    /// 页头声称的槽数越界时，槽目录定位会整数下溢 ⇒ 越界切片 panic ⇒ 进程
    /// abort。调用方必须先问 [`Self::slot_count_exceeds_capacity`]。
    pub fn get_slot(&self, index: usize) -> Option<Slot> {
        if index >= self.slot_count() {
            return None;
        }
        let slot_start = Page::PAGE_SIZE - (index + 1) * Slot::SIZE;
        let slot_buf = &self.data[slot_start..slot_start + Slot::SIZE];
        let logical_id = u16::from_le_bytes([slot_buf[0], slot_buf[1]]);
        let offset = u16::from_le_bytes([slot_buf[2], slot_buf[3]]);
        let length = u16::from_le_bytes([slot_buf[4], slot_buf[5]]);
        Some(Slot {
            logical_id,
            offset,
            length,
        })
    }

    /// Get slot by logical_id (read-only), returns (Slot, slot_index)
    pub fn get_slot_by_logical_id(&self, logical_id: u16) -> Option<(Slot, usize)> {
        for i in 0..self.slot_count() {
            if let Some(slot) = self.get_slot(i) {
                if slot.logical_id == logical_id {
                    return Some((slot, i));
                }
            }
        }
        None
    }

    /// Get data for a specific slot (read-only, zero-copy)
    pub fn get_slot_data(&self, slot: &Slot) -> &'a [u8] {
        let start = slot.offset as usize;
        let end = start + slot.length as usize;
        &self.data[start..end]
    }

    /// 有界槽读——槽负载越界时返回 `None`（**不做任何越界切片**）。
    ///
    /// 判据与索引树既有有界读（`src/storage/btree/node.rs` 的
    /// `slot_payload_in_bounds`）同形：负载不与页头重叠、不越过槽目录区
    /// （槽目录自页尾向上生长）。未越界的槽与 [`Self::get_slot_data`]
    /// 逐字节等价。判据实现 = [`slot_payload_in_bounds`]（读侧与写侧共用）。
    pub fn get_slot_data_checked(&self, slot: &Slot) -> Option<&'a [u8]> {
        let start = slot.offset as usize;
        let end = start.checked_add(slot.length as usize)?;
        if !slot_payload_in_bounds(self.slot_count(), start, end) {
            return None;
        }
        self.data.get(start..end)
    }

    /// Get header (read-only)
    pub fn header(&self) -> &SlottedPageHeader {
        &self.header
    }
}

impl<'a> SlottedPage<'a> {
    /// Create SlottedPage from existing Page (read/write mode)
    pub fn new(page: &'a mut Page) -> Self {
        let header = SlottedPageHeader::deserialize(&page.data[..SlottedPageHeader::SIZE]);
        Self { page, header }
    }

    /// Initialize an empty SlottedPage
    pub fn init(page: &'a mut Page, page_type: u8) -> Self {
        let header = SlottedPageHeader::new(page_type);
        header.serialize(&mut page.data[..SlottedPageHeader::SIZE]);
        Self { page, header }
    }

    /// Get header
    pub fn header(&self) -> &SlottedPageHeader {
        &self.header
    }

    /// Get slot count
    pub fn slot_count(&self) -> usize {
        self.header.slot_count as usize
    }

    /// **页级判据**（见 [`SlottedPageHeader::slot_count_exceeds_capacity`]）——
    /// 页头声称的槽数是否超出该页的槽目录容量。取槽**之前**必须先问它。
    pub fn slot_count_exceeds_capacity(&self) -> bool {
        self.header.slot_count_exceeds_capacity()
    }

    /// Get a specific slot
    ///
    /// 页头声称的槽数越界时，槽目录定位会整数下溢 ⇒ 越界切片 panic ⇒ 进程
    /// abort。调用方必须先问 [`Self::slot_count_exceeds_capacity`]。
    pub fn get_slot(&self, index: usize) -> Option<Slot> {
        if index >= self.slot_count() {
            return None;
        }

        // Slot array grows from page end upward
        let slot_start = Page::PAGE_SIZE - (index + 1) * Slot::SIZE;
        let slot_buf = &self.page.data[slot_start..slot_start + Slot::SIZE];

        let logical_id = u16::from_le_bytes([slot_buf[0], slot_buf[1]]);
        let offset = u16::from_le_bytes([slot_buf[2], slot_buf[3]]);
        let length = u16::from_le_bytes([slot_buf[4], slot_buf[5]]);

        Some(Slot {
            logical_id,
            offset,
            length,
        })
    }

    /// Get slot by logical_id, returns (Slot, slot_index)
    pub fn get_slot_by_logical_id(&self, logical_id: u16) -> Option<(Slot, usize)> {
        for i in 0..self.slot_count() {
            if let Some(slot) = self.get_slot(i) {
                if slot.logical_id == logical_id {
                    return Some((slot, i));
                }
            }
        }
        None
    }

    /// Get data for a specific slot
    pub fn get_slot_data(&self, slot: &Slot) -> &[u8] {
        let start = slot.offset as usize;
        let end = start + slot.length as usize;
        &self.page.data[start..end]
    }

    /// Add a new slot, returns (logical_id, slot_index)
    pub fn add_slot(&mut self, data: &[u8]) -> Result<(u16, usize), PageWriteError> {
        self.add_slot_inner(data, None)
    }

    /// Iteration 002 T8（R10）：恢复面专用强制落位原语——按给定 logical_id
    /// （来自 WAL 记录）写 slot，并把 `next_logical_id` 推进到其上（max）。
    /// 安全性依据：运行期页内 logical_id 单调分配且永不回收 →
    /// (page, logical_id) 唯一对应一个版本；重放按记录 id 落位即收敛，不再
    /// 依赖「重放分配序列 == 运行期分配序列」假设（中止事务的数据记录不重放
    /// 而其 id 已消耗的形态——abort 后重插——即失配源）。仅恢复面消费；
    /// 运行期 `add_slot` 及全部执行器/事务路径零变化。
    pub fn add_slot_with_logical_id(
        &mut self,
        data: &[u8],
        logical_id: u16,
    ) -> Result<usize, PageWriteError> {
        self.add_slot_inner(data, Some(logical_id))
            .map(|(_, slot_index)| slot_index)
    }

    /// 共享落位主体：数据写入 + slot 写入 + header 更新。`forced = None` →
    /// 运行期自然分配（`next_logical_id` 递增）；`Some(id)` → 恢复面按 id
    /// 落位（`next_logical_id` 推进到 id+1，取 max 不回退）。
    fn add_slot_inner(
        &mut self,
        data: &[u8],
        forced: Option<u16>,
    ) -> Result<(u16, usize), PageWriteError> {
        // 0. **页级判据前置**：页头声称的槽数越界时，下面的
        //    `PAGE_SIZE - slot_count * Slot::SIZE` 整数下溢 ⇒ 随后的负载写 /
        //    槽目录写越界 ⇒ panic / 越界写。任何算术之前先问它。
        if self.header.slot_count_exceeds_capacity() {
            return Err(PageWriteError::SlotCountExceedsCapacity {
                slot_count: self.slot_count(),
            });
        }

        // 1. Calculate required space
        let data_len = data.len();
        let needed_space = Slot::SIZE + data_len;

        // 2. Check available space
        let free_space_start = self.header.free_space_offset as usize;
        let free_space_end = Page::PAGE_SIZE - self.slot_count() * Slot::SIZE;

        if free_space_start + needed_space > free_space_end {
            return Err(PageWriteError::PageFull);
        }

        // 3. Write data (starting from free_space_start)
        self.page.data[free_space_start..free_space_start + data_len].copy_from_slice(data);

        // 4. Write slot (from page end upward)
        let slot_index = self.slot_count();
        let slot_start = Page::PAGE_SIZE - (slot_index + 1) * Slot::SIZE;

        let logical_id = forced.unwrap_or(self.header.next_logical_id);
        let slot = Slot {
            logical_id,
            offset: free_space_start as u16,
            length: data_len as u16,
        };

        self.page.data[slot_start..slot_start + Slot::SIZE].copy_from_slice(&[
            (slot.logical_id & 0xFF) as u8,
            ((slot.logical_id >> 8) & 0xFF) as u8,
            (slot.offset & 0xFF) as u8,
            ((slot.offset >> 8) & 0xFF) as u8,
            (slot.length & 0xFF) as u8,
            ((slot.length >> 8) & 0xFF) as u8,
        ]);

        // 5. Update header
        self.header.slot_count += 1;
        self.header.free_space_offset += data_len as u16;
        match forced {
            None => self.header.next_logical_id += 1,
            Some(id) => {
                self.header.next_logical_id = self.header.next_logical_id.max(id.saturating_add(1));
            }
        }
        self.header
            .serialize(&mut self.page.data[..SlottedPageHeader::SIZE]);

        Ok((logical_id, slot_index))
    }

    /// Delete a slot by physical index (compact slots by moving them backward)
    pub fn delete_slot(&mut self, index: usize) -> Result<(), PageWriteError> {
        // **页级判据前置**：页头声称的槽数越界时，下面的
        // `PAGE_SIZE - (i + k) * Slot::SIZE` 同形减法整数下溢 ⇒ 越界切片
        // panic ⇒ 进程 abort。
        if self.header.slot_count_exceeds_capacity() {
            return Err(PageWriteError::SlotCountExceedsCapacity {
                slot_count: self.slot_count(),
            });
        }

        if index >= self.slot_count() {
            return Err(PageWriteError::SlotIndexOutOfRange);
        }

        // Compact slots: move slots after index backward
        let count = self.slot_count();
        for i in index..(count - 1) {
            // Copy slot from i+1 to i using a temporary buffer
            let src_start = Page::PAGE_SIZE - (i + 2) * Slot::SIZE;
            let dst_start = Page::PAGE_SIZE - (i + 1) * Slot::SIZE;

            let slot_bytes = self.page.data[src_start..src_start + Slot::SIZE].to_vec();
            self.page.data[dst_start..dst_start + Slot::SIZE].copy_from_slice(&slot_bytes);
        }

        // Clear the last slot (now moved to position count-1)
        let last_slot_start = Page::PAGE_SIZE - count * Slot::SIZE;
        self.page.data[last_slot_start..last_slot_start + Slot::SIZE]
            .copy_from_slice(&[0, 0, 0, 0, 0, 0]);

        // Decrease slot_count
        self.header.slot_count -= 1;
        self.header
            .serialize(&mut self.page.data[..SlottedPageHeader::SIZE]);

        Ok(())
    }

    /// Delete a slot by logical_id
    pub fn delete_slot_by_logical_id(&mut self, logical_id: u16) -> Result<(), PageWriteError> {
        // **页级判据前置**：页头声称的槽数越界时，取槽遍历会走到槽目录定位
        // 下溢 ⇒ 越界切片 panic ⇒ 进程 abort。
        if self.header.slot_count_exceeds_capacity() {
            return Err(PageWriteError::SlotCountExceedsCapacity {
                slot_count: self.slot_count(),
            });
        }
        let (_, slot_index) = self
            .get_slot_by_logical_id(logical_id)
            .ok_or(PageWriteError::LogicalIdNotFound { logical_id })?;
        self.delete_slot(slot_index)
    }

    /// Overwrite an existing slot's data in place, equal length only.
    ///
    /// Header fields are untouched: `slot_count` / `free_space_offset` /
    /// `next_logical_id` keep their values and the header is not re-serialized,
    /// so a fixed-size field update never leaks row bytes nor consumes a slot.
    /// On a page whose data area is exhausted this still succeeds, because the
    /// live row keeps its original slot.
    ///
    /// 写入区间 = 该槽自身负载（等长已校验）⇒ 判界**精确**且零歧义：区间
    /// 必须完整落在页的数据区内（不触及页头与槽目录），否则以结构化的页损坏
    /// 失败结束，**不产生任何字节写入**。
    pub fn update_slot_in_place(
        &mut self,
        logical_id: u16,
        data: &[u8],
    ) -> Result<(), PageWriteError> {
        // **页级判据前置**：页头声称的槽数越界时，取槽遍历会走到槽目录定位
        // 下溢 ⇒ 越界切片 panic ⇒ 进程 abort。
        if self.header.slot_count_exceeds_capacity() {
            return Err(PageWriteError::SlotCountExceedsCapacity {
                slot_count: self.slot_count(),
            });
        }

        let (slot, slot_index) = self
            .get_slot_by_logical_id(logical_id)
            .ok_or(PageWriteError::LogicalIdNotFound { logical_id })?;

        if slot.length as usize != data.len() {
            return Err(PageWriteError::LengthMismatch {
                slot_index,
                slot_length: slot.length as usize,
                write_length: data.len(),
            });
        }

        let start = slot.offset as usize;
        let end = start + slot.length as usize;
        // 槽级判界（与读侧共用同一实现）——判据前置于任何字节写入。
        if !slot_payload_in_bounds(self.slot_count(), start, end) {
            return Err(PageWriteError::SlotPayloadOutOfBounds { slot_index, slot });
        }
        self.page.data[start..end].copy_from_slice(data);

        Ok(())
    }

    /// Calculate available space
    ///
    /// 无错误通道 ⇒ 页头损坏时**安全降级为 0**（调用方判定「无可用空间」
    /// ⇒ 不会写入）。页头槽数越界时，同形减法会整数下溢 ⇒ 越界值回绕成
    /// 巨大值 ⇒ 判定为「有空间」⇒ 写入越界。
    pub fn free_space(&self) -> usize {
        if self.header.slot_count_exceeds_capacity() {
            return 0;
        }
        let free_space_start = self.header.free_space_offset as usize;
        let free_space_end = Page::PAGE_SIZE - self.slot_count() * Slot::SIZE;
        // `free_space_start > free_space_end`（页头损坏）时不再下溢 ⇒ 0。
        free_space_end.saturating_sub(free_space_start)
    }

    /// Sync header to page data
    pub fn sync_header(&mut self) {
        self.header
            .serialize(&mut self.page.data[..SlottedPageHeader::SIZE]);
    }

    /// Reload header from page data (after raw page data was modified externally)
    pub fn reload_header(&mut self) {
        self.header = SlottedPageHeader::deserialize(&self.page.data[..SlottedPageHeader::SIZE]);
    }

    /// Get page id
    pub fn page_id(&self) -> crate::storage::PageId {
        self.page.id
    }

    /// Get page data reference
    pub fn page_data(&self) -> &[u8] {
        self.page.data.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{Page, PageId};

    #[test]
    fn test_slotted_page_init() {
        let mut page = Page::new(PageId(0));
        let slotted = SlottedPage::init(&mut page, 0x01);

        assert_eq!(slotted.slot_count(), 0);
        assert_eq!(slotted.header().page_type, 0x01);
    }

    #[test]
    fn test_slotted_page_add_slot() {
        let mut page = Page::new(PageId(0));
        let mut slotted = SlottedPage::init(&mut page, 0x01);

        let data = b"hello world";
        let (lid, idx) = slotted.add_slot(data).unwrap();
        assert_eq!(lid, 0);
        assert_eq!(idx, 0);
        assert_eq!(slotted.slot_count(), 1);

        let slot = slotted.get_slot(0).unwrap();
        assert_eq!(slotted.get_slot_data(&slot), data);
    }

    #[test]
    fn test_slotted_page_add_multiple_slots() {
        let mut page = Page::new(PageId(0));
        let mut slotted = SlottedPage::init(&mut page, 0x01);

        let (lid0, _) = slotted.add_slot(b"data1").unwrap();
        let (lid1, _) = slotted.add_slot(b"data2").unwrap();

        assert_eq!(lid0, 0);
        assert_eq!(lid1, 1);
        assert_eq!(slotted.slot_count(), 2);

        let slot0 = slotted.get_slot(0).unwrap();
        let slot1 = slotted.get_slot(1).unwrap();

        assert_eq!(slotted.get_slot_data(&slot0), b"data1");
        assert_eq!(slotted.get_slot_data(&slot1), b"data2");
    }

    #[test]
    fn test_slotted_page_free_space() {
        let mut page = Page::new(PageId(0));
        let mut slotted = SlottedPage::init(&mut page, 0x01);

        let initial_free = slotted.free_space();
        slotted.add_slot(b"test").unwrap();

        let after_free = slotted.free_space();
        assert!(after_free < initial_free);
        assert_eq!(initial_free - after_free, 4 + Slot::SIZE); // 4 bytes data + 6 bytes slot
    }

    #[test]
    fn test_slotted_page_no_space() {
        let mut page = Page::new(PageId(0));
        let mut slotted = SlottedPage::init(&mut page, 0x01);

        // Try to write data larger than page capacity
        let big_data = vec![1u8; 5000];
        let result = slotted.add_slot(&big_data);
        assert!(result.is_err());
    }

    #[test]
    fn test_logical_id_increment() {
        let mut page = Page::new(PageId(0));
        let mut slotted = SlottedPage::init(&mut page, 0x03);

        let (lid0, _) = slotted.add_slot(b"a").unwrap();
        let (lid1, _) = slotted.add_slot(b"b").unwrap();
        let (lid2, _) = slotted.add_slot(b"c").unwrap();

        assert_eq!(lid0, 0);
        assert_eq!(lid1, 1);
        assert_eq!(lid2, 2);
    }

    #[test]
    fn test_delete_preserves_logical_id() {
        let mut page = Page::new(PageId(0));
        let mut slotted = SlottedPage::init(&mut page, 0x03);

        let (lid0, _) = slotted.add_slot(b"a").unwrap();
        let (lid1, _) = slotted.add_slot(b"b").unwrap();
        let (lid2, _) = slotted.add_slot(b"c").unwrap();

        // Delete lid1 (logical_id=1)
        slotted.delete_slot_by_logical_id(lid1).unwrap();

        // lid0 and lid2 should still be accessible by logical_id
        let (slot0, _) = slotted.get_slot_by_logical_id(lid0).unwrap();
        assert_eq!(slotted.get_slot_data(&slot0), b"a");

        let (slot2, _) = slotted.get_slot_by_logical_id(lid2).unwrap();
        assert_eq!(slotted.get_slot_data(&slot2), b"c");

        // lid1 should be gone
        assert!(slotted.get_slot_by_logical_id(lid1).is_none());
    }

    #[test]
    fn test_update_slot_in_place_overwrites_data() {
        let mut page = Page::new(PageId(0));
        let mut slotted = SlottedPage::init(&mut page, 0x03);

        let (lid, _) = slotted.add_slot(b"aaaa").unwrap();
        let new_data = b"bbbb";
        assert_ne!(new_data, b"aaaa");

        slotted.update_slot_in_place(lid, new_data).unwrap();

        let (slot, _) = slotted.get_slot_by_logical_id(lid).unwrap();
        assert_eq!(slotted.get_slot_data(&slot), b"bbbb");
        assert_eq!(slotted.slot_count(), 1);
    }

    #[test]
    fn test_update_slot_in_place_rejects_length_mismatch() {
        let mut page = Page::new(PageId(0));
        let mut slotted = SlottedPage::init(&mut page, 0x03);

        let (lid, _) = slotted.add_slot(b"aaaa").unwrap();

        assert!(slotted.update_slot_in_place(lid, b"bb").is_err());
        assert!(slotted.update_slot_in_place(lid, b"bbbbbb").is_err());

        // Rejected update leaves the original bytes untouched
        let (slot, _) = slotted.get_slot_by_logical_id(lid).unwrap();
        assert_eq!(slotted.get_slot_data(&slot), b"aaaa");
    }

    #[test]
    fn test_update_slot_in_place_rejects_missing_logical_id() {
        let mut page = Page::new(PageId(0));
        let mut slotted = SlottedPage::init(&mut page, 0x03);

        let (lid, _) = slotted.add_slot(b"aaaa").unwrap();
        assert!(slotted.update_slot_in_place(lid + 100, b"bbbb").is_err());
    }

    #[test]
    fn test_update_slot_in_place_leaves_header_bytes_unchanged() {
        let mut page = Page::new(PageId(0));
        let mut slotted = SlottedPage::init(&mut page, 0x03);

        let (lid, _) = slotted.add_slot(b"aaaa").unwrap();
        let header_before = slotted.page_data()[..SlottedPageHeader::SIZE].to_vec();
        let slot_count_before = slotted.slot_count();
        let fso_before = slotted.header().free_space_offset;
        let next_lid_before = slotted.header().next_logical_id;

        slotted.update_slot_in_place(lid, b"zzzz").unwrap();

        assert_eq!(
            slotted.page_data()[..SlottedPageHeader::SIZE],
            header_before[..]
        );
        assert_eq!(slotted.slot_count(), slot_count_before);
        assert_eq!(slotted.header().free_space_offset, fso_before);
        assert_eq!(slotted.header().next_logical_id, next_lid_before);
    }

    #[test]
    fn test_update_slot_in_place_200_cycles_no_leak() {
        let mut page = Page::new(PageId(0));
        let mut slotted = SlottedPage::init(&mut page, 0x03);

        let (lid, _) = slotted.add_slot(b"aaaa").unwrap();
        let slot_count_before = slotted.slot_count();
        let fso_before = slotted.header().free_space_offset;
        let next_lid_before = slotted.header().next_logical_id;
        let free_space_before = slotted.free_space();

        for i in 0..200u16 {
            let payload = [i as u8, 0xAA, 0xBB, 0xCC];
            slotted.update_slot_in_place(lid, &payload).unwrap();
        }

        assert_eq!(slotted.slot_count(), slot_count_before);
        assert_eq!(slotted.header().free_space_offset, fso_before);
        assert_eq!(slotted.header().next_logical_id, next_lid_before);
        assert_eq!(slotted.free_space(), free_space_before);

        let (slot, _) = slotted.get_slot_by_logical_id(lid).unwrap();
        assert_eq!(slotted.get_slot_data(&slot), &[199u8, 0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn test_get_by_logical_id_after_compact() {
        let mut page = Page::new(PageId(0));
        let mut slotted = SlottedPage::init(&mut page, 0x03);

        let (lid0, _) = slotted.add_slot(b"data0").unwrap();
        let (lid1, _) = slotted.add_slot(b"data1").unwrap();
        let (lid2, _) = slotted.add_slot(b"data2").unwrap();

        // Delete lid0 → compact shifts lid1 and lid2
        slotted.delete_slot_by_logical_id(lid0).unwrap();

        // Verify lid1 and lid2 still accessible with correct data
        let (slot1, _) = slotted.get_slot_by_logical_id(lid1).unwrap();
        assert_eq!(slotted.get_slot_data(&slot1), b"data1");

        let (slot2, _) = slotted.get_slot_by_logical_id(lid2).unwrap();
        assert_eq!(slotted.get_slot_data(&slot2), b"data2");
    }

    /// T8（R10）：恢复面强制落位——跳号 id 写入推进计数器，后续自然分配
    /// 不冲突。
    #[test]
    fn test_add_slot_with_logical_id_advances_counter() {
        let mut page = Page::new(PageId(0));
        let mut slotted = SlottedPage::init(&mut page, 0x03);

        slotted.add_slot(b"row-0").unwrap(); // id 0
        slotted.add_slot(b"row-1").unwrap(); // id 1

        // 恢复面跳号落位：WAL 记录 id 5（中间 id 2..=4 已被中止事务在运行期
        // 消耗，无记录重放）。
        let idx = slotted.add_slot_with_logical_id(b"row-5", 5).unwrap();
        assert_eq!(idx, 2);
        let (slot, _) = slotted.get_slot_by_logical_id(5).unwrap();
        assert_eq!(slotted.get_slot_data(&slot), b"row-5");

        // 计数器推进到 6；后续自然分配取 6，不与强制落位冲突。
        assert_eq!(slotted.header().next_logical_id, 6);
        let (lid, _) = slotted.add_slot(b"row-6").unwrap();
        assert_eq!(lid, 6);
    }

    /// T8（R10）：低于计数器的空洞补位不回退计数器（max 语义）。
    #[test]
    fn test_add_slot_with_logical_id_keeps_counter_on_lower_gap() {
        let mut page = Page::new(PageId(0));
        let mut slotted = SlottedPage::init(&mut page, 0x03);

        slotted.add_slot_with_logical_id(b"row-0", 0).unwrap();
        slotted.add_slot_with_logical_id(b"row-2", 2).unwrap();
        assert_eq!(slotted.header().next_logical_id, 3);

        // 补位 id 1：落位成功且计数器不回退；后续自然分配仍取 3。
        slotted.add_slot_with_logical_id(b"row-1", 1).unwrap();
        assert_eq!(slotted.header().next_logical_id, 3);
        assert!(slotted.get_slot_by_logical_id(1).is_some());
        let (lid, _) = slotted.add_slot(b"row-3").unwrap();
        assert_eq!(lid, 3);
    }
}
