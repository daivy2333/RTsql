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

/// Slotted Page Header (16 bytes)
#[derive(Debug, Clone, Copy)]
pub struct SlottedPageHeader {
    pub page_type: u8,          // 0x01=Leaf, 0x02=Internal, 0x03=Data
    pub slot_count: u16,        // Current number of slots
    pub free_space_offset: u16, // Start of Row Data area (after header)
    pub next_page_id: u32,      // Next page ID (for linked list)
    pub next_logical_id: u16,    // Next logical ID to allocate
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

    /// Get a specific slot (read-only)
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

    /// Get a specific slot
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
    pub fn add_slot(&mut self, data: &[u8]) -> Result<(u16, usize), String> {
        // 1. Calculate required space
        let data_len = data.len();
        let needed_space = Slot::SIZE + data_len;

        // 2. Check available space
        let free_space_start = self.header.free_space_offset as usize;
        let free_space_end = Page::PAGE_SIZE - self.slot_count() * Slot::SIZE;

        if free_space_start + needed_space > free_space_end {
            return Err("No enough space in page".to_string());
        }

        // 3. Write data (starting from free_space_start)
        self.page.data[free_space_start..free_space_start + data_len].copy_from_slice(data);

        // 4. Write slot (from page end upward)
        let slot_index = self.slot_count();
        let slot_start = Page::PAGE_SIZE - (slot_index + 1) * Slot::SIZE;

        let logical_id = self.header.next_logical_id;
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
        self.header.next_logical_id += 1;
        self.header
            .serialize(&mut self.page.data[..SlottedPageHeader::SIZE]);

        Ok((logical_id, slot_index))
    }

    /// Delete a slot by physical index (compact slots by moving them backward)
    pub fn delete_slot(&mut self, index: usize) -> Result<(), String> {
        if index >= self.slot_count() {
            return Err("Slot index out of range".to_string());
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
    pub fn delete_slot_by_logical_id(&mut self, logical_id: u16) -> Result<(), String> {
        let (_, slot_index) = self
            .get_slot_by_logical_id(logical_id)
            .ok_or_else(|| format!("logical_id {} not found", logical_id))?;
        self.delete_slot(slot_index)
    }

    /// Calculate available space
    pub fn free_space(&self) -> usize {
        let free_space_start = self.header.free_space_offset as usize;
        let free_space_end = Page::PAGE_SIZE - self.slot_count() * Slot::SIZE;
        free_space_end - free_space_start
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
}
