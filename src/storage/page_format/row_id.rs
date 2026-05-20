use std::fmt;

/// RowId：指向数据页中的具体行
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowId {
    pub page_id: u32, // 数据页 ID
    pub slot_id: u16, // Slotted Page 中的 slot index
}

impl RowId {
    pub fn new(page_id: u32, slot_id: u16) -> Self {
        Self { page_id, slot_id }
    }

    /// 序列化到字节切片（6 bytes: u32 + u16）
    pub fn serialize(&self, buf: &mut [u8]) {
        assert!(buf.len() >= 6, "Buffer too small for RowId");

        buf[..4].copy_from_slice(&self.page_id.to_le_bytes());
        buf[4..6].copy_from_slice(&self.slot_id.to_le_bytes());
    }

    /// 从字节切片反序列化
    pub fn deserialize(buf: &[u8]) -> Self {
        assert!(buf.len() >= 6, "Buffer too small for RowId");

        let page_id = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        let slot_id = u16::from_le_bytes([buf[4], buf[5]]);

        Self { page_id, slot_id }
    }

    /// 总大小（6 bytes）
    pub const SIZE: usize = 6;
}

impl fmt::Display for RowId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RowId(page={}, slot={})", self.page_id, self.slot_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_row_id_new() {
        let row_id = RowId::new(1, 2);
        assert_eq!(row_id.page_id, 1);
        assert_eq!(row_id.slot_id, 2);
    }

    #[test]
    fn test_row_id_serialize_deserialize() {
        let row_id1 = RowId::new(123, 456);
        let mut buf = vec![0u8; RowId::SIZE];
        row_id1.serialize(&mut buf);

        let row_id2 = RowId::deserialize(&buf);
        assert_eq!(row_id1, row_id2);
    }

    #[test]
    fn test_row_id_size() {
        assert_eq!(RowId::SIZE, 6);
    }
}
