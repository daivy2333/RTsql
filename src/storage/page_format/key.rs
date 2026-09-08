use std::cmp::Ordering;

/// Key 最大长度（32 bytes）
pub const MAX_KEY_LEN: usize = 32;

/// 固定长度 Key（M2 简化实现）
///
/// **R-T0b-R2 修复点（Cycle 002-rework）**：`PartialEq`/`Eq` 不再依赖
/// `len` 字段，只比较 `data`（32 字节固定）。`len` 在 `Key::deserialize`
/// 通过尾部零扫描推断，对全零或尾部为零的键（如 i64 BE 0、高位零整数）
/// 会推断为 0，导致 `key == *key` 误判为不等，进而 `LeafNode::update`
/// 错误返回 `KeyNotFound`。`cmp` 已改用 `full_data()`，`PartialEq` 对齐
/// 后 `find`/`update`/`delete` 的 `==` 判定与排序语义保持一致。
#[derive(Debug, Clone)]
pub struct Key {
    data: [u8; MAX_KEY_LEN],
    len: u8, // 实际长度（<= 32）— 不参与 PartialEq / cmp
}

impl PartialEq for Key {
    fn eq(&self, other: &Self) -> bool {
        self.data == other.data
    }
}

impl Eq for Key {}

impl Key {
    /// 从字节切片创建 Key
    pub fn new(bytes: &[u8]) -> Self {
        assert!(
            bytes.len() <= MAX_KEY_LEN,
            "Key too long: {} > {}",
            bytes.len(),
            MAX_KEY_LEN
        );

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
        let len = data
            .iter()
            .rposition(|&b| b != 0)
            .map(|i| i + 1)
            .unwrap_or(0);

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
    /// 使用 `full_data()` 固定 32 字节字典序比较。
    ///
    /// **R-T0b-R2 修复点（Cycle 002-rework）**：`as_bytes()` 的切片长度依赖
    /// `len` 字段，而 `len` 来自 `Key::new` 的输入字节数。当键数据为全零
    /// 或尾部为零（如 i64 BE 0、4-byte 小整数高位为零）时，`deserialize`
    /// 推断的 `len` 会偏小（`Key::deserialize` 用 `rposition(|&b| b != 0)`
    /// 推断长度，r#f"全零键" 时返回 None → len=0），导致 `as_bytes()` 返回
    /// 空切片，比较语义被破坏，B-Tree 搜索最小键 / 含尾部零的键全部失败。
    ///
    /// 改用 `full_data()` 固定 32 字节比较：所有键在序列化时已经填充到
    /// 32 字节（`Key::serialize` 拷贝 `self.data` 全长），比较一致；不同
    /// 实际长度的键（如 5-byte 字符串与 8-byte i64）因高位为零填充，
    /// 短键 < 长键仍按字典序正确（如 "hello" 数据 5 字节后填 0 vs
    /// i64 0 的全零 32 字节，因 'h'=0x68 > 0x00 仍正确保持 "hello" > 0）。
    fn cmp(&self, other: &Self) -> Ordering {
        self.full_data().cmp(other.full_data())
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
