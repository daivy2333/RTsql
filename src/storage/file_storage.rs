use std::fs::OpenOptions;
use std::os::unix::fs::FileExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use rand::RngCore;
use tokio::task::spawn_blocking;

use super::crypto::{
    derive_key, PageCipher, DEFAULT_KDF_M_KIB, DEFAULT_KDF_P, DEFAULT_KDF_T,
    ENCRYPTED_PAGE_RECORD_SIZE,
};
use super::file_header::{FileHeader, HeaderError, FLAG_ENCRYPTED, HEADER_SIZE};
use crate::storage::{AsyncStorage, Page, PageId, Result, StorageError};

/// Map a module-private header classification to a `StorageError` with the
/// path attached (MS10-T03 design D5).
fn header_rejection(err: HeaderError, path: &Path) -> StorageError {
    let p = path.display().to_string();
    match err {
        HeaderError::BadMagic | HeaderError::ZeroVersion => StorageError::NotADatabase(p),
        HeaderError::NewerVersion(found) => {
            StorageError::NewerFileVersion(format!("file version {found}: {p}"))
        }
        HeaderError::UnknownFlags(bits) => {
            StorageError::IncompatibleHeader(format!("unknown feature flags: {bits:#x} ({p})"))
        }
        HeaderError::PageMismatch(found) => StorageError::IncompatibleHeader(format!(
            "header page size {found}, expected {} ({p})",
            Page::PAGE_SIZE
        )),
        HeaderError::ReservedNonZero(region) => {
            StorageError::IncompatibleHeader(format!("{region} region must be zero ({p})"))
        }
    }
}

pub struct FileStorage {
    file: Arc<std::fs::File>,
    /// 逻辑页大小（恒 4096，加密不改变页格式）
    page_size: usize,
    /// 单页磁盘记录长度：明文库 = page_size，加密库 = 4124
    record_size: usize,
    /// 加密库持有的页加密器；明文库恒 None（模式在 open 期一次定型）
    cipher: Option<Arc<PageCipher>>,
    file_len: AtomicU64,
    free_pages: Mutex<Vec<u64>>,
}

impl FileStorage {
    pub fn open(path: &Path) -> Result<Self> {
        Self::open_with_key(path, None)
    }

    /// 打开数据库文件；`key` 为 `Some` 时按加密库打开（0 字节文件则创建
    /// 加密新库）。打开顺序守卫：锁 → 头校验 → 密钥检查/KDF → 页长度校验
    /// （加密无密钥 / 明文带密钥在页解析与 WAL 触碰前拒绝）。
    pub fn open_with_key(path: &Path, key: Option<&str>) -> Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        // MS10-T02: advisory exclusive lock, held for the fd's lifetime and
        // released on drop/process exit. Taken before WAL open and recovery
        // so a rejected second opener never touches companion files.
        match file.try_lock() {
            Ok(()) => {}
            Err(std::fs::TryLockError::WouldBlock) => {
                return Err(StorageError::DatabaseLocked(path.display().to_string()));
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(StorageError::Io(e)),
        }

        let metadata = file.metadata()?;
        let file_len = metadata.len();
        let page_size = Page::PAGE_SIZE;

        // MS10-T03: format header — initialize (new database) or validate
        // before any page is parsed and before WAL or companion files are
        // touched (design D4). No fsync (D7): a torn header on a database
        // that has no pages yet is rejected as NotADatabase on next open.
        let (cipher, record_size, page_count) = if file_len == 0 {
            let header = match key {
                Some(_) => {
                    let mut salt = [0u8; 32];
                    rand::rngs::OsRng.fill_bytes(&mut salt);
                    FileHeader::current_encrypted(
                        salt,
                        DEFAULT_KDF_M_KIB,
                        DEFAULT_KDF_T,
                        DEFAULT_KDF_P,
                    )
                }
                None => FileHeader::current(),
            };
            file.write_all_at(&header.encode(), 0)?;
            let cipher = key.map(|password| {
                Arc::new(PageCipher::new(derive_key(
                    password.as_bytes(),
                    &header.salt,
                    DEFAULT_KDF_M_KIB,
                    DEFAULT_KDF_T,
                    DEFAULT_KDF_P,
                )))
            });
            let record_size = if cipher.is_some() {
                ENCRYPTED_PAGE_RECORD_SIZE
            } else {
                page_size
            };
            (cipher, record_size, 0)
        } else if file_len < HEADER_SIZE as u64 {
            return Err(StorageError::NotADatabase(path.display().to_string()));
        } else {
            let mut buf = [0u8; HEADER_SIZE];
            file.read_exact_at(&mut buf, 0)?;
            let header = FileHeader::decode(&buf).map_err(|e| header_rejection(e, path))?;
            let encrypted = header.flags & FLAG_ENCRYPTED != 0;
            let cipher = match (encrypted, key) {
                (true, None) => {
                    return Err(StorageError::InvalidKey(format!(
                        "database is encrypted, supply --key or RTSQL_KEY: {}",
                        path.display()
                    )));
                }
                (false, Some(_)) => {
                    return Err(StorageError::InvalidKey(format!(
                        "database is not encrypted: {}",
                        path.display()
                    )));
                }
                (true, Some(password)) => {
                    let (m_kib, t, p) = header.kdf_params();
                    Some(Arc::new(PageCipher::new(derive_key(
                        password.as_bytes(),
                        &header.salt,
                        m_kib,
                        t,
                        p,
                    ))))
                }
                (false, None) => None,
            };
            let record_size = if encrypted {
                ENCRYPTED_PAGE_RECORD_SIZE
            } else {
                page_size
            };
            let data_len = file_len - HEADER_SIZE as u64;
            if !data_len.is_multiple_of(record_size as u64) {
                return Err(StorageError::PageSizeMismatch {
                    expected: record_size,
                    actual: (data_len % record_size as u64) as usize,
                });
            }
            (cipher, record_size, data_len / record_size as u64)
        };

        Ok(Self {
            file: Arc::new(file),
            page_size,
            record_size,
            cipher,
            file_len: AtomicU64::new(page_count),
            free_pages: Mutex::new(Vec::new()),
        })
    }

    pub fn page_count(&self) -> u64 {
        self.file_len.load(Ordering::SeqCst)
    }

    fn read_page_blocking(
        file: Arc<std::fs::File>,
        cipher: Option<Arc<PageCipher>>,
        page_id: PageId,
        page_size: usize,
        record_size: usize,
    ) -> Result<Page> {
        let offset = HEADER_SIZE as u64 + page_id.to_offset(record_size);
        match cipher {
            None => {
                let mut buf = vec![0u8; page_size];
                file.as_ref().read_exact_at(&mut buf, offset)?;
                Page::from_bytes(page_id, &buf)
            }
            Some(cipher) => {
                let mut buf = vec![0u8; record_size];
                file.as_ref().read_exact_at(&mut buf, offset)?;
                let record: &[u8; ENCRYPTED_PAGE_RECORD_SIZE] =
                    (&buf[..]).try_into().expect("record length is record_size");
                let plain = cipher.decrypt_page(page_id, record).map_err(|_| {
                    StorageError::InvalidKey(format!(
                        "decryption failed (wrong key or corrupted page): page {}",
                        page_id.0
                    ))
                })?;
                Page::from_bytes(page_id, &plain)
            }
        }
    }

    fn write_page_blocking(
        file: Arc<std::fs::File>,
        cipher: Option<Arc<PageCipher>>,
        page_id: PageId,
        record_size: usize,
        data: Box<[u8; Page::PAGE_SIZE]>,
    ) -> Result<()> {
        let offset = HEADER_SIZE as u64 + page_id.to_offset(record_size);
        match cipher {
            None => file.as_ref().write_all_at(&*data, offset)?,
            Some(cipher) => {
                let record = cipher.encrypt_page(page_id, data.as_ref());
                file.as_ref().write_all_at(&record, offset)?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl AsyncStorage for FileStorage {
    async fn read_page(&self, page_id: PageId) -> Result<Page> {
        let file = self.file.clone();
        let cipher = self.cipher.clone();
        let page_size = self.page_size;
        let record_size = self.record_size;
        spawn_blocking(move || {
            Self::read_page_blocking(file, cipher, page_id, page_size, record_size)
        })
        .await?
    }

    async fn write_page(&self, page_id: PageId, page: &Page) -> Result<()> {
        let file = self.file.clone();
        let cipher = self.cipher.clone();
        let record_size = self.record_size;
        let data = page.data.clone();
        spawn_blocking(move || Self::write_page_blocking(file, cipher, page_id, record_size, data))
            .await?
    }

    async fn allocate_page(&self) -> Result<PageId> {
        // Try free list first
        if let Some(freed_id) = self.free_pages.lock().unwrap().pop() {
            return Ok(PageId(freed_id));
        }
        // Otherwise allocate new
        let page_id = self.file_len.fetch_add(1, Ordering::SeqCst);
        let offset = HEADER_SIZE as u64 + PageId(page_id).to_offset(self.record_size);
        let file = self.file.clone();
        let record_size = self.record_size;
        match self.cipher.clone() {
            Some(cipher) => {
                // 加密库：直接写加密零页。set_len 只留裸零字节，无法通过
                // GCM 认证——明文语义"新分配页读到全零空页"由密文零页承载
                // （allocate→首读先于首写的路径依赖该语义）。
                spawn_blocking(move || {
                    let zero_page = [0u8; Page::PAGE_SIZE];
                    let record = cipher.encrypt_page(PageId(page_id), &zero_page);
                    file.as_ref().write_all_at(&record, offset)?;
                    Ok::<(), std::io::Error>(())
                })
                .await??;
            }
            None => {
                let offset = PageId(page_id).to_offset(record_size);
                spawn_blocking(move || {
                    file.as_ref()
                        .set_len(HEADER_SIZE as u64 + offset + record_size as u64)?;
                    Ok::<(), std::io::Error>(())
                })
                .await??;
            }
        }
        Ok(PageId(page_id))
    }

    async fn free_page(&self, page_id: PageId) -> Result<()> {
        self.free_pages.lock().unwrap().push(page_id.0);
        // Zero the page on disk
        let zero_page = Page::new(page_id);
        self.write_page(page_id, &zero_page).await?;
        Ok(())
    }

    async fn sync(&self) -> Result<()> {
        let file = self.file.clone();
        spawn_blocking(move || {
            file.as_ref().sync_all()?;
            Ok::<(), StorageError>(())
        })
        .await?
    }

    fn page_count(&self) -> u64 {
        self.file_len.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::FLAG_ENCRYPTED;
    use tempfile::tempdir;

    fn page_with(fill: u8) -> Page {
        let mut page = Page::new(PageId(0));
        page.data.fill(fill);
        page
    }

    /// (7) 明文库 `open` 与 `open_with_key(None)` 行为等价：字节级一致 + 往返
    #[tokio::test]
    async fn open_none_equivalent_to_open_for_plaintext() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a.db");
        let b = dir.path().join("b.db");
        for path in [&a, &b] {
            let storage = if path == &a {
                FileStorage::open(path).unwrap()
            } else {
                FileStorage::open_with_key(path, None).unwrap()
            };
            let page = page_with(0xAB);
            storage.write_page(PageId(0), &page).await.unwrap();
            let read_back = storage.read_page(PageId(0)).await.unwrap();
            assert_eq!(read_back.data.as_ref(), page.data.as_ref());
            drop(storage);
        }
        assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    }

    /// (1) 加密库落盘形态：记录步长 4124、首记录非明文、头 flags 为加密位
    #[tokio::test]
    async fn encrypted_db_on_disk_shape() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("enc.db");
        {
            let storage = FileStorage::open_with_key(&path, Some("pw")).unwrap();
            let page = page_with(0xA5);
            storage.write_page(PageId(0), &page).await.unwrap();
            storage.sync().await.unwrap();
        }
        let raw = std::fs::read(&path).unwrap();
        assert_eq!(
            raw.len() as u64,
            HEADER_SIZE as u64 + ENCRYPTED_PAGE_RECORD_SIZE as u64
        );
        assert_ne!(
            &raw[HEADER_SIZE..HEADER_SIZE + Page::PAGE_SIZE],
            page_with(0xA5).data.as_ref()
        );
        assert_eq!(
            u32::from_le_bytes(raw[12..16].try_into().unwrap()),
            FLAG_ENCRYPTED
        );
    }

    /// (2) 错误密钥：open 结构成功，首读页解密认证失败 `InvalidKey`
    #[tokio::test]
    async fn wrong_key_fails_on_first_read() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("enc.db");
        {
            let storage = FileStorage::open_with_key(&path, Some("right")).unwrap();
            storage.write_page(PageId(0), &page_with(1)).await.unwrap();
        }
        let storage = FileStorage::open_with_key(&path, Some("wrong")).unwrap();
        let err = storage.read_page(PageId(0)).await.unwrap_err();
        assert!(matches!(err, StorageError::InvalidKey(_)), "got: {err:?}");
    }

    /// (3) 明文库带钥：open 期拒绝
    #[tokio::test]
    async fn plaintext_with_key_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("plain.db");
        {
            let storage = FileStorage::open(&path).unwrap();
            storage.write_page(PageId(0), &page_with(2)).await.unwrap();
        }
        let err = match FileStorage::open_with_key(&path, Some("pw")) {
            Err(e) => e,
            Ok(_) => panic!("expected InvalidKey for plaintext opened with key"),
        };
        assert!(matches!(err, StorageError::InvalidKey(_)), "got: {err:?}");
    }

    /// (4) 加密库无钥：open 期拒绝
    #[tokio::test]
    async fn encrypted_without_key_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("enc.db");
        {
            let storage = FileStorage::open_with_key(&path, Some("pw")).unwrap();
            storage.write_page(PageId(0), &page_with(3)).await.unwrap();
        }
        let err = match FileStorage::open_with_key(&path, None) {
            Err(e) => e,
            Ok(_) => panic!("expected InvalidKey for encrypted opened without key"),
        };
        assert!(matches!(err, StorageError::InvalidKey(_)), "got: {err:?}");
    }

    /// (5) 密文单字节篡改：首读认证失败
    #[tokio::test]
    async fn tampered_ciphertext_detected_on_read() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("enc.db");
        {
            let storage = FileStorage::open_with_key(&path, Some("pw")).unwrap();
            storage.write_page(PageId(0), &page_with(4)).await.unwrap();
        }
        let mut raw = std::fs::read(&path).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0x01;
        std::fs::write(&path, raw).unwrap();
        let storage = FileStorage::open_with_key(&path, Some("pw")).unwrap();
        let err = storage.read_page(PageId(0)).await.unwrap_err();
        assert!(matches!(err, StorageError::InvalidKey(_)), "got: {err:?}");
    }

    /// (6) 加密库 allocate/页数按 4124 步长
    #[tokio::test]
    async fn encrypted_allocate_uses_4124_stride() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("enc.db");
        let storage = FileStorage::open_with_key(&path, Some("pw")).unwrap();
        let id = storage.allocate_page().await.unwrap();
        assert_eq!(id, PageId(0));
        assert_eq!(storage.page_count(), 1);
        let len = std::fs::metadata(&path).unwrap().len();
        assert_eq!(len, HEADER_SIZE as u64 + ENCRYPTED_PAGE_RECORD_SIZE as u64);
    }

    /// (8) 加密库页长度校验：数据区长度非 4124 整除 → PageSizeMismatch
    /// （expected 携带 4124；weak KDF 参数仅为测试速度，语义不受影响）
    #[test]
    fn encrypted_db_rejects_non_record_aligned_length() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("enc.db");
        let header = FileHeader::current_encrypted([7u8; 32], 1024, 1, 1);
        let mut raw = header.encode().to_vec();
        raw.extend_from_slice(&vec![0u8; ENCRYPTED_PAGE_RECORD_SIZE + 100]);
        std::fs::write(&path, raw).unwrap();
        let err = match FileStorage::open_with_key(&path, Some("pw")) {
            Err(e) => e,
            Ok(_) => panic!("expected PageSizeMismatch for non-4124-aligned data region"),
        };
        assert!(
            matches!(
                err,
                StorageError::PageSizeMismatch {
                    expected: ENCRYPTED_PAGE_RECORD_SIZE,
                    ..
                }
            ),
            "got: {err:?}"
        );
    }
}
