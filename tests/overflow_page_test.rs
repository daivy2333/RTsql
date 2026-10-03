//! MS27 Iteration 000：超页元组溢出页链（T3 写路径 / T4 读路径）
//!
//! change: 2026-10-02-ms27-overflow-integrity
//!
//! 病灶：slot_data（22B VersionHeader + tuple 字节）超过内联容量
//! 4074B（4096 − 16 页头 − 6 slot）时 `add_slot` 必失败 → PageFull，
//! 长文本/JSON 场景结构性不可存储；且表示层 String u16 长度前缀对
//! >65535B 值静默回绕（表示面由 tuple.rs T1 修复）。
//!
//! 目标语义（delta spec `overflow-page-storage`，本文件覆盖 Iteration 000 面）：
//! - T3：slot_data > 4074 → 溢出链写入 + 31B locator slot（链页分配失败
//!   或 locator 落位失败 → 清理本次已分配链页后冒原错误）；≤4074 既有路径
//!   逐字节不变。
//! - T4：读路径三消费点（read_tuple_from_data_page / DataScan /
//!   find_visible_version）对 locator 形态出页闭包后装配；内联路径零拷贝
//!   零回归。边界 4073/4074/4075（slot_data 口径）、chunk 4080±1、UPDATE
//!   增长/缩回、混排、点查、干净重开。
//!
//! Iteration 001 将扩展：崩溃恢复两态、GC/drop/TRUNCATE 释放、加密/
//! `:memory:`/只读、全写通道、~1MB 基线。

use rtsql::storage::data::TableManager;
use rtsql::storage::page_format::{overflow, ColumnType, SlottedPageRef};
use rtsql::storage::{BufferPool, FileStorage, PageId, RowId};
use rtsql::transaction::VersionHeader;
use std::sync::Arc;
use tempfile::{tempdir, TempDir};

/// 库级装配：BufferPool + TableManager（catalog 引导占页 0–1）+ 单 Int 列表，
/// 供 T3 写路径直写见证（raw tuple bytes 不经 SQL 解析）。
async fn setup_raw() -> (
    Arc<BufferPool>,
    Arc<TableManager>,
    Arc<rtsql::storage::data::TableMeta>,
    TempDir,
) {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let pool = Arc::new(BufferPool::new(16, storage.clone()).unwrap());
    let tm = TableManager::new(pool.clone(), storage).await.unwrap();
    tm.create_table("t", vec![("id".to_string(), ColumnType::Int)], "id")
        .await
        .unwrap();
    let meta = tm.get_table("t").await.unwrap();
    (pool, tm, meta, dir)
}

/// 直查某 RowId 的 slot 数据字节（VersionHeader + tuple 段）。
async fn slot_bytes(pool: &Arc<BufferPool>, row_id: RowId) -> Vec<u8> {
    let page_id = PageId(row_id.page_id as u64);
    pool.with_page_data(page_id, |data| {
        let slotted = SlottedPageRef::new(data);
        let (slot, _) = slotted
            .get_slot_by_logical_id(row_id.slot_id)
            .expect("slot must exist for the returned RowId");
        Ok(slotted.get_slot_data(&slot).to_vec())
    })
    .await
    .unwrap()
}

// ===========================================================================
// T3：写路径溢出分支
// ===========================================================================

/// >4074B slot_data 经溢出链写入：数据页 slot 为 31B locator
/// （[vh 22B][0xFF][total_len u32][first_page u32]），链页数与 chunk 计划一致。
#[tokio::test]
async fn oversized_write_places_locator_and_chain() {
    let (pool, tm, meta, _dir) = setup_raw().await;

    let tuple = vec![0xABu8; 10_000]; // slot_data = 10_022 > 4074
    let vh = VersionHeader::new(1, None);
    let row_id = tm.write_tuple(&meta, &vh, &tuple).await.unwrap();

    let slot = slot_bytes(&pool, row_id).await;
    assert_eq!(
        slot.len(),
        rtsql::transaction::VersionHeader::SIZE + overflow::LOCATOR_SIZE,
        "overflow slot must carry header + 9B locator, got {}",
        slot.len()
    );
    let locator = &slot[rtsql::transaction::VersionHeader::SIZE..];
    let (total_len, first_page) =
        overflow::decode_locator(locator).expect("locator segment must decode");
    assert_eq!(total_len as usize, tuple.len());
    assert_ne!(first_page, 0, "chain must start at a real page");

    let pages = overflow::collect_overflow_pages(&pool, first_page)
        .await
        .unwrap();
    assert_eq!(
        pages.len(),
        overflow::chunk_plan(tuple.len()).len(),
        "chain page count must match the chunk plan"
    );
    let assembled = overflow::read_overflow_chain(&pool, first_page, total_len)
        .await
        .unwrap();
    assert_eq!(assembled, tuple, "chain payload must roundtrip");
}

/// 边界内侧（slot_data 4073/4074）保持内联：slot 数据长度即 slot_data 长度，
/// tuple 首字节为值 tag（非 0xFF），不分配溢出链。
#[tokio::test]
async fn inline_boundary_slots_stay_inline() {
    let (pool, tm, meta, _dir) = setup_raw().await;

    // slot_data = 22 + tuple；tuple 4051 → 4073，tuple 4052 → 4074（恰满）
    for tuple_len in [4051usize, 4052] {
        let tuple = vec![0x5Au8; tuple_len];
        let vh = VersionHeader::new(tuple_len as u64, None);
        let row_id = tm.write_tuple(&meta, &vh, &tuple).await.unwrap();

        let slot = slot_bytes(&pool, row_id).await;
        assert_eq!(
            slot.len(),
            22 + tuple_len,
            "slot_data {}/{tuple_len} must stay inline",
            22 + tuple_len
        );
        assert_ne!(
            slot[22],
            overflow::LOCATOR_TAG,
            "inline tuple segment must not start with the locator marker"
        );
    }
}

/// 边界外侧（slot_data 4075）走溢出：数据页 slot 恰为 31B locator。
#[tokio::test]
async fn just_over_boundary_goes_overflow() {
    let (pool, tm, meta, _dir) = setup_raw().await;

    let tuple = vec![0x11u8; 4053]; // slot_data = 4075
    let vh = VersionHeader::new(1, None);
    let row_id = tm.write_tuple(&meta, &vh, &tuple).await.unwrap();

    let slot = slot_bytes(&pool, row_id).await;
    assert_eq!(
        slot.len(),
        31,
        "4075 slot_data must become a 31B locator slot"
    );
    assert_eq!(slot[22], overflow::LOCATOR_TAG);
    let (_, first_page) = overflow::decode_locator(&slot[22..]).unwrap();
    let assembled = overflow::read_overflow_chain(&pool, first_page, 4053)
        .await
        .unwrap();
    assert_eq!(assembled, tuple);
}

// ===========================================================================
// T4：读路径两相装配
// ===========================================================================

use rtsql::database::Database;
use rtsql::network::protocol::Response;
use rtsql::transaction::IsolationLevel;
use serde_json::Value as Json;

fn db_path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("sql.db")
}

async fn exec_ok(db: &Database, sql: &str) {
    let resp = db.execute_sql(sql).await;
    assert!(
        !matches!(resp, Response::Error { .. }),
        "statement failed: {sql:?} -> {resp:?}"
    );
}

fn query_rows(resp: Response) -> Vec<Vec<Json>> {
    match resp {
        Response::QueryResult { rows } => rows,
        other => panic!("Expected QueryResult, got {other:?}"),
    }
}

/// SQL 层装配：给定字节长度的 String 值写读逐字节一致。
/// slot_data = 22 + (3 + N)：N=4048 → 4073（内联）、4049 → 4074（内联恰满）、
/// 4050 → 4075（溢出）——4073/4074/4075 边界三值。
#[tokio::test]
async fn sql_boundary_4073_4074_4075_roundtrip() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v STRING(65535))").await;

    let sizes = [4048usize, 4049, 4050];
    for (i, &n) in sizes.iter().enumerate() {
        let payload = format!("'{}'", "a".repeat(n));
        exec_ok(&db, &format!("INSERT INTO t VALUES ({i}, {payload})")).await;
    }

    let rows = query_rows(db.execute_sql("SELECT k, v FROM t ORDER BY k").await);
    assert_eq!(rows.len(), 3, "all three boundary rows must scan back");
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(row[0].as_i64(), Some(i as i64));
        assert_eq!(
            row[1].as_str().map(|s| s.len()),
            Some(sizes[i]),
            "row {i} payload length must roundtrip byte-exact"
        );
    }

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// chunk 边界：tuple_bytes = N+3 跨 4080 页容量（4079 单页 / 4080 恰满单页 /
/// 4081 跨两页 / 8158 跨两页）。
#[tokio::test]
async fn sql_chunk_boundary_sizes_roundtrip() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v STRING(65535))").await;

    // tuple_bytes 长度 = N + 3：4079 → [4079]；4080 → [4080]；4081 → [4080,1]；
    // 8158 → [4080,4078]
    let sizes = [4076usize, 4077, 4078, 8155];
    for (i, &n) in sizes.iter().enumerate() {
        let payload = format!("'{}'", "b".repeat(n));
        exec_ok(&db, &format!("INSERT INTO t VALUES ({i}, {payload})")).await;
    }

    let rows = query_rows(db.execute_sql("SELECT k, v FROM t ORDER BY k").await);
    assert_eq!(rows.len(), sizes.len());
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(row[1].as_str().map(|s| s.len()), Some(sizes[i]), "row {i}");
    }

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// UPDATE 增长（内联 → 超页）与缩回（超页 → 内联）：新版本各自形态正确，
/// 读回逐字节一致。
#[tokio::test]
async fn update_grow_into_overflow_and_shrink_back() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v STRING(65535))").await;
    exec_ok(&db, "INSERT INTO t VALUES (1, 'small')").await;

    let big = "x".repeat(5000);
    exec_ok(&db, &format!("UPDATE t SET v = '{big}' WHERE k = 1")).await;
    let rows = query_rows(db.execute_sql("SELECT v FROM t WHERE k = 1").await);
    assert_eq!(rows[0][0].as_str().map(|s| s.len()), Some(5000));

    exec_ok(&db, "UPDATE t SET v = 'tiny' WHERE k = 1").await;
    let rows = query_rows(db.execute_sql("SELECT v FROM t WHERE k = 1").await);
    assert_eq!(rows[0][0].as_str(), Some("tiny"));

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// 超页行与内联行混排：全表扫描与计数正确（无静默跳行/错位）。
#[tokio::test]
async fn mixed_inline_and_oversized_rows_scan() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v STRING(65535))").await;

    // 交替内联（<4049）与超页（>4050）
    for k in 1..=6i64 {
        let n = if k % 2 == 1 { 100 } else { 6000 };
        let payload = format!("'{}'", "m".repeat(n));
        exec_ok(&db, &format!("INSERT INTO t VALUES ({k}, {payload})")).await;
    }

    let rows = query_rows(
        db.execute_sql("SELECT k, length(v) FROM t ORDER BY k")
            .await,
    );
    assert_eq!(rows.len(), 6, "all rows must scan back exactly once");
    for (i, row) in rows.iter().enumerate() {
        let k = i as i64 + 1;
        let expect = if k % 2 == 1 { 100 } else { 6000 };
        assert_eq!(row[0].as_i64(), Some(k));
        assert_eq!(row[1].as_i64(), Some(expect), "row k={k}");
    }

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// 主键点查命中超页行（find_visible_version 装配面）。
#[tokio::test]
async fn pk_point_lookup_hits_oversized_row() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v STRING(65535))").await;

    let big = "p".repeat(9000);
    exec_ok(&db, &format!("INSERT INTO t VALUES (7, '{big}')")).await;
    exec_ok(&db, "INSERT INTO t VALUES (8, 'inline')").await;

    let rows = query_rows(db.execute_sql("SELECT v FROM t WHERE k = 7").await);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0].as_str().map(|s| s.len()), Some(9000));

    // 点查内联邻居不受扰
    let rows = query_rows(db.execute_sql("SELECT v FROM t WHERE k = 8").await);
    assert_eq!(rows[0][0].as_str(), Some("inline"));

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

// ===========================================================================
// Iteration 001 / T6：生命周期释放（GC/drop/TRUNCATE 无孤儿链）
// ===========================================================================

/// UPDATE 缩回后 gc_table 释放旧超页版本的链页：v1 超页（链 A）被 v2 内联
/// 头取代 → gc 收集 v1 → 链 A 页回收可复用（同 000 free_overflow_chain
/// 单测的分配见证形态）。
#[tokio::test]
async fn gc_releases_oversized_version_chain() {
    let (pool, tm, meta, _dir) = setup_raw().await;

    // v1：已提交超页版本（链 A）
    let vh1 = VersionHeader::new(1, Some(1));
    let big = vec![0xAAu8; 10_000];
    let rid1 = tm.write_tuple(&meta, &vh1, &big).await.unwrap();
    let slot1 = slot_bytes(&pool, rid1).await;
    let (_, first_a) = overflow::decode_locator(&slot1[VersionHeader::SIZE..]).unwrap();
    let chain_a = overflow::collect_overflow_pages(&pool, first_a)
        .await
        .unwrap();

    // v2：已提交内联头，next → v1（UPDATE 缩回形态）；登记索引供 gc 扫描
    let vh2 = VersionHeader::new(2, Some(2)).with_next_version(rid1);
    let rid2 = tm.write_tuple(&meta, &vh2, b"small").await.unwrap();
    let key = 7i64.to_be_bytes();
    meta.index_manager.insert(&key, rid2).await.unwrap();

    let cleaned = meta.gc_table(&pool).await.unwrap();
    assert_eq!(cleaned, 1, "the oversized committed v1 must be collected");

    // RED：链 A 未释放时这里分配到的是 fresh 页（集合失配）
    let mut reclaimed = Vec::new();
    for _ in 0..chain_a.len() {
        let id = pool.storage().allocate_page().await.unwrap();
        reclaimed.push(id.0 as u32);
    }
    let mut expect = chain_a.clone();
    expect.sort();
    reclaimed.sort();
    assert_eq!(
        reclaimed, expect,
        "gc must free the oversized version's chain pages"
    );
}

/// drop_table 后溢出链页随数据页/索引页一并回收：全量分配回收页集合
/// （数据页 + 索引页 + 链页）与 fresh 分配结果集合相等。
#[tokio::test]
async fn drop_table_releases_overflow_chain_pages() {
    let (pool, tm, meta, _dir) = setup_raw().await;

    let vh = VersionHeader::new(1, Some(1));
    let big = vec![0xBBu8; 9_000];
    let rid = tm.write_tuple(&meta, &vh, &big).await.unwrap();
    let slot = slot_bytes(&pool, rid).await;
    let (_, first) = overflow::decode_locator(&slot[VersionHeader::SIZE..]).unwrap();
    let chain = overflow::collect_overflow_pages(&pool, first)
        .await
        .unwrap();
    let index_pages = meta.index_manager.collect_all_pages().await.unwrap();

    tm.drop_table("t").await.unwrap();

    let mut expected: Vec<u32> = chain.clone();
    expected.extend(index_pages.iter().map(|p| p.0 as u32));
    expected.push(rid.page_id as u32); // 单行表恰一个数据页
    expected.sort();

    // RED：链页未释放时 fresh 分配混入，集合失配
    let mut reclaimed = Vec::new();
    for _ in 0..expected.len() {
        let id = pool.storage().allocate_page().await.unwrap();
        reclaimed.push(id.0 as u32);
    }
    reclaimed.sort();
    assert_eq!(
        reclaimed, expected,
        "drop must free data + index + overflow chain pages"
    );
}

/// TRUNCATE 后溢出链页回收且表定义保留：链页按 truncate 既有释放语义被
/// 立即清零落盘（free-list 弹出顺序受树重建页流转影响，集合断言不稳健，
/// 以清零见证）；get_table 可达、tail 回头部后写读恢复正常。
#[tokio::test]
async fn truncate_releases_overflow_chain_pages_keeps_definition() {
    let (pool, tm, meta, _dir) = setup_raw().await;

    let vh = VersionHeader::new(1, Some(1));
    let big = vec![0xCCu8; 9_000];
    let rid = tm.write_tuple(&meta, &vh, &big).await.unwrap();
    let slot = slot_bytes(&pool, rid).await;
    let (_, first) = overflow::decode_locator(&slot[VersionHeader::SIZE..]).unwrap();
    let chain = overflow::collect_overflow_pages(&pool, first)
        .await
        .unwrap();

    tm.truncate_table("t").await.unwrap();

    // 表定义保留（同时取新 PK 根）：树重建从 free-list 弹页，被弹中的链页
    // 合法复用为新根——链页集合必须恰为「清零页 ∪ 新 PK 根」。
    let meta2 = tm.get_table("t").await.unwrap();
    let new_pk_root = meta2.index_manager.root_page_id().0 as u32;
    assert!(
        chain.contains(&new_pk_root),
        "rebuilt PK root must be reclaimed from the freed chain pages, got {new_pk_root}"
    );
    // RED：链页未释放时磁盘上仍是 0x04 溢出页映像，非零
    for raw in &chain {
        if *raw == new_pk_root {
            continue;
        }
        let page = pool.storage().read_page(PageId(*raw as u64)).await.unwrap();
        assert!(
            page.data.iter().all(|&b| b == 0),
            "chain page {raw} must be zeroed on disk after truncate"
        );
    }

    // 写读恢复（tail 回头部）
    let vh2 = VersionHeader::new(9, Some(9));
    let rid2 = tm.write_tuple(&meta2, &vh2, b"after").await.unwrap();
    let read_back =
        rtsql::storage::read_tuple_from_data_page(&pool, rid2, |_, bytes| Ok(bytes.to_vec()))
            .await
            .unwrap();
    assert_eq!(read_back, b"after");
}

// ===========================================================================
// Iteration 001 / T5：恢复面溢出感知 + CREATE INDEX 回填装配消费
// ===========================================================================

/// 超页 INSERT 后崩溃（shutdown + drop 不 close）→ 重开 full_recover 无
/// RedoFailed：行读回逐字节一致、PK 索引点查可达；重复恢复（再次崩溃重开
/// 重放同一 WAL）幂等——slot 已存在跳过且不重写链。
#[tokio::test]
async fn oversized_insert_survives_crash_recovery() {
    let dir = tempdir().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v STRING(65535))").await;
        db.checkpoint().await.unwrap();
        let big = "c".repeat(9000);
        exec_ok(&db, &format!("INSERT INTO t VALUES (1, '{big}')")).await;
        db.wal_buffer.as_ref().unwrap().shutdown().await;
    } // drop without close = crash

    // RED：重放 slot_data = 22 + 9003 无法经 add_slot_with_logical_id 落位
    {
        let db = Database::open(&path).await.unwrap();
        let rows = query_rows(db.execute_sql("SELECT v FROM t WHERE k = 1").await);
        assert_eq!(rows.len(), 1, "recovered row must be visible via PK lookup");
        assert_eq!(rows[0][0].as_str().map(|s| s.len()), Some(9000));
        db.wal_buffer.as_ref().unwrap().shutdown().await;
    } // crash again: the same WAL replays a second time

    // 幂等：重复恢复不重放失败、不重复分配链页，行保持一致
    {
        let db = Database::open(&path).await.unwrap();
        let rows = query_rows(db.execute_sql("SELECT v FROM t WHERE k = 1").await);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0].as_str().map(|s| s.len()), Some(9000));
        db.close().await.unwrap();
    }
}

/// UPDATE 增长/缩回后的崩溃恢复两态：增长落超页 → 崩溃重开读回大值；
/// 缩回内联 → 再次崩溃重开读回小值。第二次重开的 pre-scan 与 rebuild 扫描
/// 磁盘上的 locator slot（checkpoint 持久化的旧版本），缩回重放的
/// derive_old_row_id 逐字节比较超页旧版本。
#[tokio::test]
async fn oversized_update_crash_recovery_two_states() {
    let dir = tempdir().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v STRING(65535))").await;
        db.checkpoint().await.unwrap();
        exec_ok(&db, "INSERT INTO t VALUES (1, 'small')").await;
        db.checkpoint().await.unwrap();
        let big = "u".repeat(5000);
        exec_ok(&db, &format!("UPDATE t SET v = '{big}' WHERE k = 1")).await;
        db.wal_buffer.as_ref().unwrap().shutdown().await;
    } // crash #1

    {
        let db = Database::open(&path).await.unwrap();
        let rows = query_rows(db.execute_sql("SELECT v FROM t WHERE k = 1").await);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0].as_str().map(|s| s.len()), Some(5000));
        exec_ok(&db, "UPDATE t SET v = 'tiny' WHERE k = 1").await;
        db.wal_buffer.as_ref().unwrap().shutdown().await;
    } // crash #2

    {
        let db = Database::open(&path).await.unwrap();
        let rows = query_rows(db.execute_sql("SELECT v FROM t WHERE k = 1").await);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0].as_str(), Some("tiny"));
        db.close().await.unwrap();
    }
}

/// keyless（NULL PK）超页行崩溃恢复：无键版本按 tuple 原始字节入 keyless
/// 桶——locator slot 未经装配时桶键是 9B locator 字节，与 WAL tuple 失配。
#[tokio::test]
async fn keyless_oversized_row_crash_recovery() {
    let dir = tempdir().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v STRING(65535))").await;
        db.checkpoint().await.unwrap();
        let big = "n".repeat(9000);
        exec_ok(&db, &format!("INSERT INTO t VALUES (NULL, '{big}')")).await;
        db.wal_buffer.as_ref().unwrap().shutdown().await;
    } // crash

    {
        let db = Database::open(&path).await.unwrap();
        let rows = query_rows(db.execute_sql("SELECT COUNT(*) FROM t").await);
        assert_eq!(
            rows[0][0],
            serde_json::json!(1),
            "keyless oversized row must replay"
        );
        let rows = query_rows(
            db.execute_sql("SELECT length(v) FROM t WHERE k IS NULL")
                .await,
        );
        assert_eq!(rows[0][0], serde_json::json!(9000));
        db.close().await.unwrap();
    }
}

/// 含超页行的表 CREATE INDEX（存量回填）：回填对 locator slot 不得按 9B
/// 原始字节反序列化——两相装配后派生索引键，建索引成功且等值点查路由可达。
#[tokio::test]
async fn create_index_backfill_with_oversized_rows() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(
        &db,
        "CREATE TABLE t (k INT PRIMARY KEY, v STRING(65535), n INT)",
    )
    .await;

    let big = "b".repeat(8000);
    exec_ok(&db, &format!("INSERT INTO t VALUES (1, '{big}', 5)")).await;
    exec_ok(&db, "INSERT INTO t VALUES (2, 'inline', 6)").await;

    // RED：回填页闭包内对 9B locator 直接 deserialize_row → unknown tag 报错
    exec_ok(&db, "CREATE INDEX idx_n ON t (n)").await;

    let rows = query_rows(db.execute_sql("SELECT k FROM t WHERE n = 5").await);
    assert_eq!(
        rows.len(),
        1,
        "index-routed lookup must reach the oversized row"
    );
    assert_eq!(rows[0][0], serde_json::json!(1));
    let rows = query_rows(db.execute_sql("SELECT k FROM t WHERE n = 6").await);
    assert_eq!(rows[0][0], serde_json::json!(2));

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

// ===========================================================================
// Iteration 001 / T7：通道与形态 e2e + 规模基线
// ===========================================================================

/// 写通道矩阵：INSERT VALUES、子集列+DEFAULT（超页值与大值混排）、
/// UPSERT DO UPDATE（excluded 引用）、REPLACE、UPDATE 增长——全部经超页
/// 行形态读回逐字节一致。
#[tokio::test]
async fn write_channel_matrix_oversized() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(
        &db,
        "CREATE TABLE w (k INT PRIMARY KEY, v STRING(65535) DEFAULT 'dflt', n INT)",
    )
    .await;

    // INSERT VALUES 超页
    let big1 = "i".repeat(5000);
    exec_ok(&db, &format!("INSERT INTO w VALUES (1, '{big1}', 10)")).await;
    // 子集列清单 + 超页值
    let big2 = "s".repeat(4500);
    exec_ok(
        &db,
        &format!("INSERT INTO w (k, v, n) VALUES (2, '{big2}', 20)"),
    )
    .await;
    // 子集列省略 v → DEFAULT 应用（内联）
    exec_ok(&db, "INSERT INTO w (k, n) VALUES (3, 30)").await;
    // UPSERT DO UPDATE（excluded 引用）落超页
    let big3 = "u".repeat(5200);
    exec_ok(
        &db,
        &format!(
            "INSERT INTO w VALUES (3, '{big3}', 33) ON CONFLICT (k) DO UPDATE SET v = excluded.v"
        ),
    )
    .await;
    // REPLACE 落超页
    let big4 = "r".repeat(5300);
    exec_ok(&db, &format!("REPLACE INTO w VALUES (3, '{big4}', 34)")).await;
    // UPDATE 增长落超页
    let big5 = "g".repeat(6000);
    exec_ok(&db, &format!("UPDATE w SET v = '{big5}' WHERE k = 1")).await;

    let rows = query_rows(
        db.execute_sql("SELECT k, length(v) FROM w ORDER BY k")
            .await,
    );
    assert_eq!(rows.len(), 3, "all rows must scan back exactly once");
    assert_eq!(rows[0][0], serde_json::json!(1));
    assert_eq!(rows[0][1], serde_json::json!(6000), "UPDATE-grown row");
    assert_eq!(rows[1][0], serde_json::json!(2));
    assert_eq!(rows[1][1], serde_json::json!(4500), "subset-insert row");
    assert_eq!(rows[2][0], serde_json::json!(3));
    assert_eq!(rows[2][1], serde_json::json!(5300), "REPLACE-wins row");

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// 65535/65536 两 tag 切换点 SQL 层可达：两行都经溢出链存取，长度逐字节
/// 一致（tuple.rs 单测覆盖表示面，此处为 e2e 确认）。
#[tokio::test]
async fn long_string_tag_switch_point_sql_layer() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE b (k INT PRIMARY KEY, v STRING(65535))").await;

    let a = "a".repeat(65535);
    let c = "c".repeat(65536);
    exec_ok(&db, &format!("INSERT INTO b VALUES (1, '{a}')")).await;
    exec_ok(&db, &format!("INSERT INTO b VALUES (2, '{c}')")).await;

    let rows = query_rows(
        db.execute_sql("SELECT k, length(v) FROM b ORDER BY k")
            .await,
    );
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0][1], serde_json::json!(65535), "TAG_STRING boundary");
    assert_eq!(
        rows[1][1],
        serde_json::json!(65536),
        "TAG_LONG_STRING boundary"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// CTAS 物化含超页源行：建表→物化→读回一致。
#[tokio::test]
async fn ctas_materializes_oversized_source_rows() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE src (k INT PRIMARY KEY, v STRING(65535))").await;
    let big = "m".repeat(7000);
    exec_ok(&db, &format!("INSERT INTO src VALUES (1, '{big}')")).await;

    exec_ok(&db, "CREATE TABLE dst AS SELECT * FROM src").await;
    let rows = query_rows(
        db.execute_sql("SELECT length(v) FROM dst WHERE k = 1")
            .await,
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0], serde_json::json!(7000));

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// INSERT SELECT 跨库写超页行：附加库源 → 主库目标物化写入。ATTACH 为
/// CLI 会话专属（lib 面点名拒绝），故走子进程 e2e——ATTACH 与 INSERT
/// SELECT 同一会话多语句执行。
#[test]
fn insert_select_writes_oversized_row_cross_db() {
    let dir = cli_fixture();
    let aux_path = dir.path().join("aux.db");
    let aux = aux_path.to_str().unwrap();
    let out = run_cli(dir.path(), &["new", aux]);
    assert_eq!(out.code, Some(0), "aux new failed: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &[aux, "CREATE TABLE s (k INT PRIMARY KEY, v STRING(65535))"],
    );
    assert_eq!(out.code, Some(0), "aux create failed: {}", out.stderr);
    let big = "x".repeat(7500);
    let out = run_cli(
        dir.path(),
        &[aux, &format!("INSERT INTO s VALUES (1, '{big}')")],
    );
    assert_eq!(out.code, Some(0), "aux seed failed: {}", out.stderr);

    let out = run_cli(dir.path(), &["new", "app"]);
    assert_eq!(out.code, Some(0), "new failed: {}", out.stderr);
    let out = run_cli(
        dir.path(),
        &["app", "CREATE TABLE t (k INT PRIMARY KEY, v STRING(65535))"],
    );
    assert_eq!(out.code, Some(0), "create failed: {}", out.stderr);

    let attach_insert = format!("ATTACH '{aux}' AS aux; INSERT INTO t SELECT k, v FROM aux.s");
    let out = run_cli(dir.path(), &["app", &attach_insert]);
    assert_eq!(
        out.code,
        Some(0),
        "attach+insert-select failed: {} / {}",
        out.stdout,
        out.stderr
    );

    let sel = run_cli(dir.path(), &["app", "SELECT length(v) FROM t WHERE k = 1"]);
    assert_eq!(sel.code, Some(0), "select failed: {}", sel.stderr);
    let rendered: Json = serde_json::from_str(&sel.stdout).expect("json render");
    assert_eq!(rendered["rows"][0][0], serde_json::json!(7500));
}

/// 非 String 超大值既有拒绝保持：超长日期字面量在写入边界点名拒绝且
/// 零副作用（不走溢出路径、不落行）。
#[tokio::test]
async fn non_string_oversized_value_keeps_rejection() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    exec_ok(&db, "CREATE TABLE d (k INT PRIMARY KEY, day DATE)").await;

    let not_a_date = "z".repeat(5000);
    let resp = db
        .execute_sql(&format!("INSERT INTO d VALUES (1, '{not_a_date}')"))
        .await;
    assert!(
        matches!(resp, Response::Error { .. }),
        "oversized invalid date literal must be rejected, got {resp:?}"
    );
    let rows = query_rows(db.execute_sql("SELECT COUNT(*) FROM d").await);
    assert_eq!(
        rows[0][0],
        serde_json::json!(0),
        "rejection must be side-effect free"
    );

    db.wal_buffer.as_ref().unwrap().shutdown().await;
}

/// 加密库（AES-256-GCM 页加密）超页写读 + 崩溃恢复：4124B 加密步长下
/// 溢出链页与 locator 两态一致。
#[tokio::test]
async fn encrypted_oversized_write_read_and_crash_recovery() {
    let dir = tempdir().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open_with_key(&path, IsolationLevel::RepeatableRead, Some("k1"))
            .await
            .unwrap();
        exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v STRING(65535))").await;
        db.checkpoint().await.unwrap();
        let big = "e".repeat(8500);
        exec_ok(&db, &format!("INSERT INTO t VALUES (1, '{big}')")).await;
        db.wal_buffer.as_ref().unwrap().shutdown().await;
    } // crash

    {
        let db = Database::open_with_key(&path, IsolationLevel::RepeatableRead, Some("k1"))
            .await
            .unwrap();
        let rows = query_rows(db.execute_sql("SELECT length(v) FROM t WHERE k = 1").await);
        assert_eq!(rows.len(), 1, "encrypted oversized row must recover");
        assert_eq!(rows[0][0], serde_json::json!(8500));
        db.close().await.unwrap();
    }
}

/// `:memory:` 超页读写：MemoryStorage 后端零文件承载溢出链。
#[tokio::test]
async fn memory_db_oversized_roundtrip() {
    let db = Database::open_in_memory().await.unwrap();
    exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v STRING(65535))").await;
    let big = "q".repeat(6500);
    exec_ok(&db, &format!("INSERT INTO t VALUES (1, '{big}')")).await;
    let rows = query_rows(db.execute_sql("SELECT length(v) FROM t WHERE k = 1").await);
    assert_eq!(rows[0][0], serde_json::json!(6500));
    db.close().await.unwrap();
}

/// 只读会话 SELECT 超页行：共享锁只读打开装配读回，零写入保持。
#[tokio::test]
async fn readonly_session_selects_oversized_row() {
    let dir = tempdir().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v STRING(65535))").await;
        let big = "o".repeat(7000);
        exec_ok(&db, &format!("INSERT INTO t VALUES (1, '{big}')")).await;
        db.close().await.unwrap();
    }
    let db = Database::open_readonly(&path).await.unwrap();
    let rows = query_rows(db.execute_sql("SELECT length(v) FROM t WHERE k = 1").await);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0], serde_json::json!(7000));
}

/// ~1MB 规模基线：单行 INSERT → 读回 → 崩溃模拟 → 重开恢复 → 读回一致
/// （~2MB WAL 帧；只证可用性不设时延断言）。
#[tokio::test]
async fn one_megabyte_row_crash_recovery_baseline() {
    let dir = tempdir().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v STRING(65535))").await;
        db.checkpoint().await.unwrap();
        let big = "M".repeat(1_000_000);
        exec_ok(&db, &format!("INSERT INTO t VALUES (1, '{big}')")).await;
        let rows = query_rows(db.execute_sql("SELECT length(v) FROM t").await);
        assert_eq!(rows[0][0], serde_json::json!(1_000_000));
        db.wal_buffer.as_ref().unwrap().shutdown().await;
    } // crash

    {
        let db = Database::open(&path).await.unwrap();
        let rows = query_rows(db.execute_sql("SELECT length(v) FROM t WHERE k = 1").await);
        assert_eq!(rows.len(), 1, "1MB row must recover from WAL replay");
        assert_eq!(rows[0][0], serde_json::json!(1_000_000));
        db.close().await.unwrap();
    }
}

/// 干净重开一致：超页行落库 → close（checkpoint 落盘）→ 重开读回逐字节一致。
/// （崩溃模拟的重开两态属 Iteration 001 T5——`shutdown` ≠ `close`。）
#[tokio::test]
async fn oversized_rows_survive_clean_reopen() {
    let dir = tempdir().unwrap();
    let path = db_path(&dir);
    {
        let db = Database::open(&path).await.unwrap();
        exec_ok(&db, "CREATE TABLE t (k INT PRIMARY KEY, v STRING(65535))").await;
        let payload = format!("'{}'", "r".repeat(12000));
        exec_ok(&db, &format!("INSERT INTO t VALUES (1, {payload})")).await;
        db.close().await.unwrap();
    }
    {
        let db = Database::open(&path).await.unwrap();
        let rows = query_rows(db.execute_sql("SELECT v FROM t WHERE k = 1").await);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0].as_str().map(|s| s.len()), Some(12000));
        db.close().await.unwrap();
    }
}

// ===========================================================================
// Iteration 001 / T7：CLI 生命周期子命令通道（dump→restore / import）
// ===========================================================================

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn spawn_cli_stdin(dir: &std::path::Path, args: &[&str]) -> Child {
    Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .env_remove("RTSQL_KEY")
        .env_remove("RTSQL_BUSY_TIMEOUT")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtsql binary")
}

fn run_cli(dir: &std::path::Path, args: &[&str]) -> CliOutput {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .env_remove("RTSQL_KEY")
        .env_remove("RTSQL_BUSY_TIMEOUT")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtsql binary");
    wait_cli(&mut child)
}

fn run_cli_stdin(dir: &std::path::Path, args: &[&str], input: &str) -> CliOutput {
    let mut child = spawn_cli_stdin(dir, args);
    let mut stdin = child.stdin.take().unwrap();
    let _ = stdin.write_all(input.as_bytes());
    drop(stdin);
    wait_cli(&mut child)
}

fn wait_cli(child: &mut Child) -> CliOutput {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match child.try_wait().expect("poll rtsql status") {
            Some(status) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                child
                    .stdout
                    .take()
                    .unwrap()
                    .read_to_string(&mut stdout)
                    .unwrap();
                child
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_string(&mut stderr)
                    .unwrap();
                return CliOutput {
                    code: status.code(),
                    stdout,
                    stderr,
                };
            }
            None => {
                if Instant::now() > deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("rtsql did not exit within 60s (one-shot CLI must terminate)");
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

/// CLI 仓库现场：TempDir + 预建 `db/` 集中存储目录（RTSQL_HOME 指向）。
fn cli_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("db")).unwrap();
    dir
}

/// dump→restore 往返含超页行：dump 输出携带超页 INSERT，restore 重建后
/// 长度逐字节一致（非 TTY 默认 json 渲染，按 JSON 解析断言）。
#[test]
fn dump_restore_roundtrip_oversized_row() {
    let dir = cli_fixture();
    let out = run_cli(dir.path(), &["new", "app"]);
    assert_eq!(out.code, Some(0), "new failed: {}", out.stderr);

    let out = run_cli(
        dir.path(),
        &["app", "CREATE TABLE t (k INT PRIMARY KEY, v STRING(65535))"],
    );
    assert_eq!(out.code, Some(0), "create failed: {}", out.stderr);

    let big = "d".repeat(9000);
    let out = run_cli(
        dir.path(),
        &["app", &format!("INSERT INTO t VALUES (1, '{big}')")],
    );
    assert_eq!(out.code, Some(0), "seed failed: {}", out.stderr);

    let dump = run_cli(dir.path(), &["dump", "app"]);
    assert_eq!(
        dump.code,
        Some(0),
        "dump failed: {:?} / {:?}",
        dump.stdout,
        dump.stderr
    );
    assert!(
        dump.stdout.contains("INSERT INTO \"t\" VALUES"),
        "dump must carry the oversized INSERT"
    );

    let out = run_cli(dir.path(), &["new", "app2"]);
    assert_eq!(out.code, Some(0), "new app2 failed: {}", out.stderr);
    let out = run_cli_stdin(dir.path(), &["restore", "app2", "-"], &dump.stdout);
    assert_eq!(out.code, Some(0), "restore failed: {}", out.stderr);

    let sel = run_cli(dir.path(), &["app2", "SELECT length(v) FROM t WHERE k = 1"]);
    assert_eq!(sel.code, Some(0), "select failed: {}", sel.stderr);
    let rendered: Json = serde_json::from_str(&sel.stdout).expect("json render");
    assert_eq!(
        rendered["rows"][0][0],
        serde_json::json!(9000),
        "restored row length must roundtrip"
    );
}

/// import CSV 超页字段：既有表导入 9000 字符字段，读回长度一致。
#[test]
fn import_csv_oversized_field() {
    let dir = cli_fixture();
    let out = run_cli(dir.path(), &["new", "app"]);
    assert_eq!(out.code, Some(0), "new failed: {}", out.stderr);

    let out = run_cli(
        dir.path(),
        &["app", "CREATE TABLE t (k INT, v STRING(65535))"],
    );
    assert_eq!(out.code, Some(0), "create failed: {}", out.stderr);

    let csv_path = dir.path().join("data.csv");
    std::fs::write(&csv_path, format!("k,v\n1,{}\n", "f".repeat(9000))).unwrap();

    let out = run_cli(dir.path(), &["import", "app", "t", "data.csv", "--csv"]);
    assert_eq!(
        out.code,
        Some(0),
        "import failed: {} / {}",
        out.stdout,
        out.stderr
    );

    let sel = run_cli(dir.path(), &["app", "SELECT length(v) FROM t WHERE k = 1"]);
    assert_eq!(sel.code, Some(0), "select failed: {}", sel.stderr);
    let rendered: Json = serde_json::from_str(&sel.stdout).expect("json render");
    assert_eq!(
        rendered["rows"][0][0],
        serde_json::json!(9000),
        "imported oversized field must roundtrip"
    );
}
