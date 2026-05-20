use crate::storage::Page;

/// Slot: pointing to row data in the page
#[derive(Debug, Clone, Copy)]
pub struct Slot {
    pub offset: u16,  // Offset into Row Data area
    pub length: u16,  // Row length
}

impl Slot {
    pub const SIZE: usize = 4;  // u16 + u16
}

/// Slotted Page Header (16 bytes)
#[derive(Debug, Clone, Copy)]
pub struct SlottedPageHeader {
    pub page_type: u8,          // 0x01=Leaf, 0x02=Internal, 0x03=Data
    pub slot_count: u16,        // Current number of slots
    pub free_space_offset: u16, // Start of Row Data area (after header)
    pub next_page_id: u32,      // Next page ID (for linked list)
    _padding: [u8; 5],          // Padding to 16 bytes
}

impl SlottedPageHeader {
    pub const SIZE: usize = 16;

    pub fn new(page_type: u8) -> Self {
        Self {
            page_type,
            slot_count: 0,
            free_space_offset: Self::SIZE as u16,  // Initially points right after header
            next_page_id: 0,
            _padding: [0; 5],
        }
    }

    /// Serialize to byte slice
    pub fn serialize(&self, buf: &mut [u8]) {
        buf[0] = self.page_type;
        buf[1..3].copy_from_slice(&self.slot_count.to_le_bytes());
        buf[3..5].copy_from_slice(&self.free_space_offset.to_le_bytes());
        buf[5..9].copy_from_slice(&self.next_page_id.to_le_bytes());
        buf[9..14].copy_from_slice(&self._padding);
    }

    /// Deserialize from byte slice
    pub fn deserialize(buf: &[u8]) -> Self {
        let page_type = buf[0];
        let slot_count = u16::from_le_bytes([buf[1], buf[2]]);
        let free_space_offset = u16::from_le_bytes([buf[3], buf[4]]);
        let next_page_id = u32::from_le_bytes([buf[5], buf[6], buf[7], buf[8]]);
        let _padding = buf[9..14].try_into().unwrap();

        Self {
            page_type,
            slot_count,
            free_space_offset,
            next_page_id,
            _padding,
        }
    }
}

/// Slotted Page format reader/writer
pub struct SlottedPage<'a> {
    pub(crate) page: &'a mut Page,
    header: SlottedPageHeader,
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

        let offset = u16::from_le_bytes([slot_buf[0], slot_buf[1]]);
        let length = u16::from_le_bytes([slot_buf[2], slot_buf[3]]);

        Some(Slot { offset, length })
    }

    /// Get data for a specific slot
    pub fn get_slot_data(&self, slot: &Slot) -> &[u8] {
        let start = slot.offset as usize;
        let end = start + slot.length as usize;
        &self.page.data[start..end]
    }

    /// Add a new slot
    pub fn add_slot(&mut self, data: &[u8]) -> Result<usize, String> {
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

        let slot = Slot {
            offset: free_space_start as u16,
            length: data_len as u16,
        };

        self.page.data[slot_start..slot_start + Slot::SIZE].copy_from_slice(&[
            (slot.offset & 0xFF) as u8,
            ((slot.offset >> 8) & 0xFF) as u8,
            (slot.length & 0xFF) as u8,
            ((slot.length >> 8) & 0xFF) as u8,
        ]);

        // 5. Update header
        self.header.slot_count += 1;
        self.header.free_space_offset += data_len as u16;
        self.header.serialize(&mut self.page.data[..SlottedPageHeader::SIZE]);

        Ok(slot_index)
    }

    /// Delete a slot (mark as deleted, don't reclaim space immediately)
    pub fn delete_slot(&mut self, index: usize) -> Result<(), String> {
        if index >= self.slot_count() {
            return Err("Slot index out of range".to_string());
        }

        // Mark as 0 (indicating deleted)
        let slot_start = Page::PAGE_SIZE - (index + 1) * Slot::SIZE;
        self.page.data[slot_start..slot_start + Slot::SIZE].copy_from_slice(&[0, 0, 0, 0]);

        Ok(())
    }

    /// Calculate available space
    pub fn free_space(&self) -> usize {
        let free_space_start = self.header.free_space_offset as usize;
        let free_space_end = Page::PAGE_SIZE - self.slot_count() * Slot::SIZE;
        free_space_end - free_space_start
    }

    /// Sync header to page data
    pub fn sync_header(&mut self) {
        self.header.serialize(&mut self.page.data[..SlottedPageHeader::SIZE]);
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
    use crate::storage::{PageId, Page};

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
        let index = slotted.add_slot(data).unwrap();
        assert_eq!(index, 0);
        assert_eq!(slotted.slot_count(), 1);

        let slot = slotted.get_slot(0).unwrap();
        assert_eq!(slotted.get_slot_data(&slot), data);
    }

    #[test]
    fn test_slotted_page_add_multiple_slots() {
        let mut page = Page::new(PageId(0));
        let mut slotted = SlottedPage::init(&mut page, 0x01);

        slotted.add_slot(b"data1").unwrap();
        slotted.add_slot(b"data2").unwrap();

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
        assert_eq!(initial_free - after_free, 4 + Slot::SIZE);  // 4 bytes data + 4 bytes slot
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
}