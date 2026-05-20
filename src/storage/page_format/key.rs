use std::cmp::Ordering;

/// Key 最大长度（32 bytes）
pub const MAX_KEY_LEN: usize = 32;

/// 固定长度 Key（M2 简化实现）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    data: [u8; MAX_KEY_LEN],
    len: u8,  // 实际长度（<= 32）
}

impl Key {
    /// 从字节切片创建 Key
    pub fn new(bytes: &[u8]) -> Self {
        assert!(bytes.len() <= MAX_KEY_LEN, "Key too long: {} > {}", bytes.len(), MAX_KEY_LEN);

        let mut data = [0u8; MAX_KEY_LEN];
        data[..bytes.len()].copy_from_slice(bytes);

        Self {
            data,
            len: bytes.len() as u8,
        }
    }

    /// 获取实际长度
    pub fn len(&self) -> usize {
        self.len as usize
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// 获取 Key 数据（实际长度）
    pub fn as_bytes(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }

    /// 获取完整数据（32 bytes，用于序列化）
    pub fn full_data(&self) -> &[u8; MAX_KEY_LEN] {
        &self.data
    }

    /// 序列化到字节切片
    pub fn serialize(&self, buf: &mut [u8]) {
        assert!(buf.len() >= MAX_KEY_LEN, "Buffer too small for Key");
        buf[..MAX_KEY_LEN].copy_from_slice(&self.data);
    }

    /// 从字节切片反序列化
    pub fn deserialize(buf: &[u8]) -> Self {
        assert!(buf.len() >= MAX_KEY_LEN, "Buffer too small for Key");
        let mut data = [0u8; MAX_KEY_LEN];
        data.copy_from_slice(&buf[..MAX_KEY_LEN]);

        // 找到实际长度（去除尾部 0）
        let len = data.iter().rposition(|&b| b != 0).map(|i| i + 1).unwrap_or(0);

        Self {
            data,
            len: len as u8,
        }
    }
}

impl PartialOrd for Key {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Key {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_bytes().cmp(other.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_new() {
        let key = Key::new(b"hello");
        assert_eq!(key.len(), 5);
        assert_eq!(key.as_bytes(), b"hello");
    }

    #[test]
    fn test_key_empty() {
        let key = Key::new(b"");
        assert_eq!(key.len(), 0);
        assert!(key.is_empty());
    }

    #[test]
    fn test_key_max_length() {
        let bytes = [1u8; MAX_KEY_LEN];
        let key = Key::new(&bytes);
        assert_eq!(key.len(), MAX_KEY_LEN);
    }

    #[test]
    #[should_panic]
    fn test_key_too_long() {
        let bytes = [1u8; MAX_KEY_LEN + 1];
        Key::new(&bytes);
    }

    #[test]
    fn test_key_serialize_deserialize() {
        let key1 = Key::new(b"test_key");
        let mut buf = vec![0u8; MAX_KEY_LEN];
        key1.serialize(&mut buf);

        let key2 = Key::deserialize(&buf);
        assert_eq!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn test_key_ordering() {
        let key1 = Key::new(b"a");
        let key2 = Key::new(b"b");
        let key3 = Key::new(b"a");

        assert!(key1 < key2);
        assert!(key1 == key3);
        assert!(key2 > key1);
    }
}