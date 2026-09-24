//! 主数据库文件 64 字节格式头（MS10-T03，design D1/D2）。
//!
//! 布局（D1，offset 单位字节）：
//!
//! ```text
//!  0.. 8  magic           b"RTSQLDB\0"
//!  8..12  format_version  u32 LE（= FORMAT_VERSION）
//! 12..16  flags           u32 LE（bit0 = encrypted，MS17-T01 激活）
//! 16..20  page_size       u32 LE（= 4096）
//! 20..52  salt            32B（Argon2id 盐；明文库必须全 0，加密库必须非 0）
//! 52..64  reserved        12B（明文库必须全 0；加密库存放 KDF 参数
//!                          m_kib/t/p 三个 u32 LE）
//! ```
//!
//! 头位于页空间之外：页 N 的文件偏移 = `HEADER_SIZE + N * page_size`。
//! 模块只做内存编解码，零 crate 内部依赖；`HeaderError` 为模块内私有
//! 分类，由 `FileStorage::open` 映射为 `StorageError` 并附路径。

/// 头大小（字节）
pub const HEADER_SIZE: usize = 64;
/// 当前格式版本；`decode` 只接受 `1..=FORMAT_VERSION`
pub const FORMAT_VERSION: u32 = 1;
/// 加密 flag 位（MS17-T01 激活；置位的文件按加密库处理）
pub const FLAG_ENCRYPTED: u32 = 1;
/// 当前构建支持的 flag 位集合。bit0 = encrypted；其余位拒绝——
/// 前向防护：不把更新版本的文件当明文库解析（spec「未知特性 flag 拒绝」）。
pub const KNOWN_FLAGS_MASK: u32 = FLAG_ENCRYPTED;

/// magic 标识
const MAGIC: [u8; 8] = *b"RTSQLDB\0";
/// 页大小字段唯一合法值（与 `Page::PAGE_SIZE` 一致）
const PAGE_SIZE: u32 = 4096;

/// 解码后的文件头
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileHeader {
    pub version: u32,
    pub flags: u32,
    pub page_size: u32,
    /// Argon2id 盐；明文库恒全 0
    pub salt: [u8; 32],
    /// KDF 参数区（m_kib/t/p 三个 u32 LE）；明文库恒全 0
    pub kdf_params: [u8; 12],
}

/// 头解码错误分类（模块私有；由 `FileStorage` 映射为 `StorageError` 变体）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeaderError {
    /// magic 不符
    BadMagic,
    /// version == 0：未初始化或损坏，按非 RTsql 文件拒绝
    ZeroVersion,
    /// version 高于当前支持（携带 found）
    NewerVersion(u32),
    /// 含当前构建不支持的 flag 位（携带越界位）
    UnknownFlags(u32),
    /// page_size 与本构建不符（携带 found）
    PageMismatch(u32),
    /// salt/reserved 区非 0（携带区域名）
    ReservedNonZero(&'static str),
}

impl FileHeader {
    /// 当前构建创建新库时写入的头（明文库：盐/参数区恒零）
    pub fn current() -> Self {
        Self {
            version: FORMAT_VERSION,
            flags: 0,
            page_size: PAGE_SIZE,
            salt: [0; 32],
            kdf_params: [0; 12],
        }
    }

    /// 当前构建创建加密新库时写入的头（加密位 + 盐 + KDF 参数编码）。
    /// 不做值域校验——非法组合的拒绝发生在 `decode`。
    pub(crate) fn current_encrypted(salt: [u8; 32], m_kib: u32, t: u32, p: u32) -> Self {
        let mut kdf_params = [0u8; 12];
        kdf_params[0..4].copy_from_slice(&m_kib.to_le_bytes());
        kdf_params[4..8].copy_from_slice(&t.to_le_bytes());
        kdf_params[8..12].copy_from_slice(&p.to_le_bytes());
        Self {
            version: FORMAT_VERSION,
            flags: FLAG_ENCRYPTED,
            page_size: PAGE_SIZE,
            salt,
            kdf_params,
        }
    }

    /// 解码 KDF 参数区为 `(m_kib, t, p)`
    pub fn kdf_params(&self) -> (u32, u32, u32) {
        (
            u32::from_le_bytes(self.kdf_params[0..4].try_into().unwrap()),
            u32::from_le_bytes(self.kdf_params[4..8].try_into().unwrap()),
            u32::from_le_bytes(self.kdf_params[8..12].try_into().unwrap()),
        )
    }

    /// 编码为 64 字节头；明文库 salt/参数区恒为 0
    pub fn encode(&self) -> [u8; HEADER_SIZE] {
        let mut buf = [0u8; HEADER_SIZE];
        buf[..8].copy_from_slice(&MAGIC);
        buf[8..12].copy_from_slice(&self.version.to_le_bytes());
        buf[12..16].copy_from_slice(&self.flags.to_le_bytes());
        buf[16..20].copy_from_slice(&self.page_size.to_le_bytes());
        buf[20..52].copy_from_slice(&self.salt);
        buf[52..].copy_from_slice(&self.kdf_params);
        buf
    }

    /// 解码 64 字节头，按 D4 值域分类拒绝
    pub(crate) fn decode(buf: &[u8; HEADER_SIZE]) -> Result<FileHeader, HeaderError> {
        if buf[..8] != MAGIC {
            return Err(HeaderError::BadMagic);
        }
        let version = u32::from_le_bytes(buf[8..12].try_into().unwrap());
        let flags = u32::from_le_bytes(buf[12..16].try_into().unwrap());
        let page_size = u32::from_le_bytes(buf[16..20].try_into().unwrap());
        if version == 0 {
            return Err(HeaderError::ZeroVersion);
        }
        if version > FORMAT_VERSION {
            return Err(HeaderError::NewerVersion(version));
        }
        let unknown_flags = flags & !KNOWN_FLAGS_MASK;
        if unknown_flags != 0 {
            return Err(HeaderError::UnknownFlags(unknown_flags));
        }
        if page_size != PAGE_SIZE {
            return Err(HeaderError::PageMismatch(page_size));
        }
        let mut header = FileHeader {
            version,
            flags,
            page_size,
            salt: [0; 32],
            kdf_params: [0; 12],
        };
        if flags & FLAG_ENCRYPTED != 0 {
            header.salt.copy_from_slice(&buf[20..52]);
            header.kdf_params.copy_from_slice(&buf[52..]);
            if header.salt == [0; 32] {
                return Err(HeaderError::ReservedNonZero("salt"));
            }
            let (m_kib, t, p) = header.kdf_params();
            if super::crypto::validate_kdf_params(m_kib, t, p).is_err() {
                return Err(HeaderError::ReservedNonZero("reserved"));
            }
        } else {
            if buf[20..52].iter().any(|&b| b != 0) {
                return Err(HeaderError::ReservedNonZero("salt"));
            }
            if buf[52..].iter().any(|&b| b != 0) {
                return Err(HeaderError::ReservedNonZero("reserved"));
            }
        }
        Ok(header)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 独立于 encode 构造的合法头字节（布局交叉验证夹具）
    fn valid_header_bytes() -> [u8; HEADER_SIZE] {
        let mut buf = [0u8; HEADER_SIZE];
        buf[..8].copy_from_slice(&MAGIC);
        buf[8..12].copy_from_slice(&1u32.to_le_bytes());
        buf[12..16].copy_from_slice(&0u32.to_le_bytes());
        buf[16..20].copy_from_slice(&4096u32.to_le_bytes());
        buf
    }

    #[test]
    fn encode_layout_matches_d1() {
        let buf = FileHeader::current().encode();
        assert_eq!(&buf[..8], &MAGIC);
        assert_eq!(u32::from_le_bytes(buf[8..12].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(buf[12..16].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(buf[16..20].try_into().unwrap()), 4096);
        assert!(
            buf[20..].iter().all(|&b| b == 0),
            "salt/reserved must be zero"
        );
    }

    #[test]
    fn roundtrip_preserves_fields() {
        let header = FileHeader::current();
        assert_eq!(FileHeader::decode(&header.encode()), Ok(header));
    }

    #[test]
    fn bad_magic_rejected() {
        let mut buf = valid_header_bytes();
        buf[0] = b'X';
        assert_eq!(FileHeader::decode(&buf), Err(HeaderError::BadMagic));
    }

    #[test]
    fn version_zero_rejected_as_uninitialized() {
        let mut buf = valid_header_bytes();
        buf[8..12].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(FileHeader::decode(&buf), Err(HeaderError::ZeroVersion));
    }

    #[test]
    fn newer_version_classified_with_found_value() {
        let mut buf = valid_header_bytes();
        buf[8..12].copy_from_slice(&99u32.to_le_bytes());
        assert_eq!(FileHeader::decode(&buf), Err(HeaderError::NewerVersion(99)));
    }

    #[test]
    fn encrypted_header_roundtrip() {
        let header = FileHeader::current_encrypted([7u8; 32], 19456, 2, 1);
        assert_eq!(FileHeader::decode(&header.encode()), Ok(header));
    }

    #[test]
    fn encrypted_header_salt_all_zero_rejected() {
        let buf = FileHeader::current_encrypted([0u8; 32], 19456, 2, 1).encode();
        assert_eq!(
            FileHeader::decode(&buf),
            Err(HeaderError::ReservedNonZero("salt"))
        );
    }

    #[test]
    fn encrypted_header_invalid_kdf_params_rejected() {
        let buf = FileHeader::current_encrypted([7u8; 32], 19456, 0, 1).encode();
        assert_eq!(
            FileHeader::decode(&buf),
            Err(HeaderError::ReservedNonZero("reserved"))
        );
        let buf = FileHeader::current_encrypted([7u8; 32], 512, 2, 1).encode();
        assert_eq!(
            FileHeader::decode(&buf),
            Err(HeaderError::ReservedNonZero("reserved"))
        );
    }

    #[test]
    fn unknown_flag_bit1_still_rejected() {
        let mut buf = valid_header_bytes();
        buf[12..16].copy_from_slice(&0b10u32.to_le_bytes());
        assert_eq!(
            FileHeader::decode(&buf),
            Err(HeaderError::UnknownFlags(0b10))
        );
    }

    #[test]
    fn unknown_flag_bit_rejected() {
        let mut buf = valid_header_bytes();
        buf[12..16].copy_from_slice(&0b100u32.to_le_bytes());
        assert_eq!(
            FileHeader::decode(&buf),
            Err(HeaderError::UnknownFlags(0b100))
        );
    }

    #[test]
    fn wrong_page_size_rejected_with_found_value() {
        let mut buf = valid_header_bytes();
        buf[16..20].copy_from_slice(&8192u32.to_le_bytes());
        assert_eq!(
            FileHeader::decode(&buf),
            Err(HeaderError::PageMismatch(8192))
        );
    }

    #[test]
    fn nonzero_salt_rejected() {
        let mut buf = valid_header_bytes();
        buf[20] = 1;
        assert_eq!(
            FileHeader::decode(&buf),
            Err(HeaderError::ReservedNonZero("salt"))
        );
    }

    #[test]
    fn nonzero_reserved_rejected() {
        let mut buf = valid_header_bytes();
        buf[HEADER_SIZE - 1] = 1;
        assert_eq!(
            FileHeader::decode(&buf),
            Err(HeaderError::ReservedNonZero("reserved"))
        );
    }
}
