//! MS10-T03: 主库文件格式头集成测试 —— 头生命周期、拒绝矩阵与锁优先守卫。
//!
//! 场景（change spec database-file-format-header R1/R2/R3）：
//! - 新库（0 字节）打开即写 64B 头，重开（v1）校验通过，页数据经 64+N*4096 访问；
//! - 非 RTsql / 新版 / 未知 flag / 截断 / <64B / 旧无头文件按原因分类拒绝；
//! - 锁先于头校验：被持锁的坏文件报 `DatabaseLocked` 而非格式错误。

use rtsql::database::Database;
use rtsql::storage::{
    AsyncStorage, FileStorage, PageId, StorageError, FLAG_ENCRYPTED, FORMAT_VERSION, HEADER_SIZE,
};
use tempfile::tempdir;

/// unwrap_err 的等价形式：不要求 Ok 侧类型实现 Debug。
fn expect_err<T>(result: Result<T, StorageError>) -> StorageError {
    match result {
        Ok(_) => panic!("expected open to be rejected, got Ok"),
        Err(e) => e,
    }
}

/// 按字段构造头字节（独立于 `FileHeader::encode` 的布局夹具）
fn header_bytes(version: u32, flags: u32, page_size: u32) -> Vec<u8> {
    let mut buf = vec![0u8; HEADER_SIZE];
    buf[..8].copy_from_slice(b"RTSQLDB\0");
    buf[8..12].copy_from_slice(&version.to_le_bytes());
    buf[12..16].copy_from_slice(&flags.to_le_bytes());
    buf[16..20].copy_from_slice(&page_size.to_le_bytes());
    buf
}

/// 合法头 + n 页零数据
fn header_plus_pages(version: u32, flags: u32, page_size: u32, pages: usize) -> Vec<u8> {
    let mut bytes = header_bytes(version, flags, page_size);
    bytes.extend(std::iter::repeat(0u8).take(pages * 4096));
    bytes
}

// ---- R1: 头布局与生命周期 ----

/// 新库打开即带头（magic/version=1/flags=0/page_size=4096），header-only
/// 文件 page_count 为 0，重开校验通过。
#[tokio::test]
async fn test_new_database_starts_with_header_and_reopens() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("fresh.db");
    {
        let storage = FileStorage::open(&path).unwrap();
        assert_eq!(storage.page_count(), 0, "header-only file has 0 pages");
    }

    let bytes = std::fs::read(&path).unwrap();
    assert!(
        bytes.len() >= HEADER_SIZE,
        "file must start with the {HEADER_SIZE}-byte header, len={}",
        bytes.len()
    );
    assert_eq!(&bytes[..8], b"RTSQLDB\0", "magic mismatch");
    assert_eq!(
        u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        FORMAT_VERSION
    );
    assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 0);
    assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 4096);
    assert!(
        bytes[20..HEADER_SIZE].iter().all(|&b| b == 0),
        "salt/reserved must be zero"
    );

    let reopened = FileStorage::open(&path);
    assert!(
        reopened.is_ok(),
        "v1 file must reopen: {:?}",
        reopened.err()
    );
}

/// v1 库分配 1 页后：文件长度 = 64 + 4096，重开 page_count 为 1
/// （页寻址 +64 平移的可观察见证）。
#[tokio::test]
async fn test_v1_file_reopens_and_page_io_shifts_by_header() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("v1.db");
    {
        let storage = FileStorage::open(&path).unwrap();
        let id = storage.allocate_page().await.unwrap();
        assert_eq!(id, PageId(0));
    }

    let len = std::fs::metadata(&path).unwrap().len();
    assert_eq!(
        len,
        (HEADER_SIZE + 4096) as u64,
        "1-page file = header + 4096"
    );

    let reopened = FileStorage::open(&path).unwrap();
    assert_eq!(reopened.page_count(), 1);
}

// ---- R2: 格式错误显式拒绝 ----

/// 4KiB 垃圾文件（大小合法的非 RTsql 文件）→ NotADatabase
#[tokio::test]
async fn test_garbage_4k_rejected() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("garbage4k.db");
    std::fs::write(&path, vec![0xA5u8; 4096]).unwrap();

    let err = expect_err(FileStorage::open(&path));
    assert!(matches!(err, StorageError::NotADatabase(_)), "got: {err:?}");
}

/// 8KiB 垃圾文件（此前 SIGABRT 的回归场景）→ NotADatabase，消息含路径
#[tokio::test]
async fn test_garbage_8k_rejected() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("garbage8k.db");
    std::fs::write(&path, vec![0x5Au8; 8192]).unwrap();

    let err = expect_err(FileStorage::open(&path));
    assert!(matches!(err, StorageError::NotADatabase(_)), "got: {err:?}");
    if let StorageError::NotADatabase(msg) = &err {
        assert!(
            msg.contains(path.to_str().unwrap()),
            "message must contain the path: {msg}"
        );
    }
}

/// Database 级：8KiB 垃圾文件干净拒绝（D8 指定的 RED 见证场景——
/// 当前实现 catalog 解析 panic → PageGuard drop 双重 panic → abort，
/// RED 观察必须单独运行，否则终止整个测试二进制）。
#[tokio::test]
async fn garbage_8k_file_opens_with_clean_error() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("garbage.db");
    std::fs::write(&path, vec![0x7Fu8; 8192]).unwrap();

    let err = expect_err(Database::open(&path).await);
    assert!(matches!(err, StorageError::NotADatabase(_)), "got: {err:?}");
}

/// <64B 文件 → NotADatabase（替代误导性 Page size mismatch）
#[tokio::test]
async fn test_short_file_rejected() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("tiny.db");
    std::fs::write(&path, vec![b't'; 57]).unwrap();

    let err = expect_err(FileStorage::open(&path));
    assert!(matches!(err, StorageError::NotADatabase(_)), "got: {err:?}");
}

/// version=0（未初始化/损坏）→ NotADatabase
#[tokio::test]
async fn test_version_zero_rejected() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("v0.db");
    std::fs::write(&path, header_plus_pages(0, 0, 4096, 1)).unwrap();

    let err = expect_err(FileStorage::open(&path));
    assert!(matches!(err, StorageError::NotADatabase(_)), "got: {err:?}");
}

/// version=2 → NewerFileVersion，消息表明"文件由新版创建"
#[tokio::test]
async fn test_newer_version_rejected() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("v2.db");
    std::fs::write(&path, header_plus_pages(2, 0, 4096, 1)).unwrap();

    let err = expect_err(FileStorage::open(&path));
    assert!(
        matches!(err, StorageError::NewerFileVersion(_)),
        "got: {err:?}"
    );
    assert!(
        err.to_string().contains("newer version"),
        "message must say the file is newer: {err}"
    );
}

/// 加密位（当前版本不支持）→ IncompatibleHeader（前向防护：
/// 不把 MS12 密文库当明文库解析）
#[tokio::test]
async fn test_encrypted_flag_rejected() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("enc.db");
    std::fs::write(&path, header_plus_pages(1, FLAG_ENCRYPTED, 4096, 1)).unwrap();

    let err = expect_err(FileStorage::open(&path));
    assert!(
        matches!(err, StorageError::IncompatibleHeader(_)),
        "got: {err:?}"
    );
}

/// 未定义 flag 位 → IncompatibleHeader
#[tokio::test]
async fn test_unknown_flag_bit_rejected() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("flag.db");
    std::fs::write(&path, header_plus_pages(1, 0b100, 4096, 1)).unwrap();

    let err = expect_err(FileStorage::open(&path));
    assert!(
        matches!(err, StorageError::IncompatibleHeader(_)),
        "got: {err:?}"
    );
}

/// page_size ≠ 4096 → IncompatibleHeader
#[tokio::test]
async fn test_wrong_page_size_rejected() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ps.db");
    std::fs::write(&path, header_plus_pages(1, 0, 8192, 1)).unwrap();

    let err = expect_err(FileStorage::open(&path));
    assert!(
        matches!(err, StorageError::IncompatibleHeader(_)),
        "got: {err:?}"
    );
}

/// 截断文件（头 + 非整页字节数）→ PageSizeMismatch
#[tokio::test]
async fn test_truncated_file_rejected() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("trunc.db");
    let mut bytes = header_bytes(1, 0, 4096);
    bytes.extend(std::iter::repeat(0u8).take(100));
    std::fs::write(&path, bytes).unwrap();

    let err = expect_err(FileStorage::open(&path));
    assert!(
        matches!(err, StorageError::PageSizeMismatch { .. }),
        "got: {err:?}"
    );
}

/// 旧无头库（T03 之前的库：页 0/1 直接位于文件头，无 magic）→
/// NotADatabase，统一拒绝、不做嗅探迁移（用户决策 2026-09-08）
#[tokio::test]
async fn test_headerless_legacy_file_rejected() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("legacy.db");
    // 旧库页内容可以是任意非 magic 字节（此处全零页即可代表旧 catalog 页）
    std::fs::write(&path, vec![0u8; 8192]).unwrap();

    let err = expect_err(FileStorage::open(&path));
    assert!(matches!(err, StorageError::NotADatabase(_)), "got: {err:?}");
}

// ---- R3: 打开顺序守卫 ----

/// 锁先于头校验：坏文件被持锁 → DatabaseLocked（而非格式错误）。
/// GREEN 守卫（锁本就先于一切，MS10-T02 语义）。
#[tokio::test]
async fn test_locked_garbage_file_reports_locked() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("locked-garbage.db");
    std::fs::write(&path, vec![0xEEu8; 8192]).unwrap();

    let holder = std::fs::File::open(&path).unwrap();
    holder.try_lock().expect("test process acquires flock");

    let err = expect_err(FileStorage::open(&path));
    assert!(
        matches!(err, StorageError::DatabaseLocked(_)),
        "expected DatabaseLocked, got: {err:?}"
    );
}
