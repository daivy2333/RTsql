//! change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
//! Iteration 005（Cycle 000-initial）：DML 墓碑的放弃窗口收敛（ISS24 的 DML 同形面）。
//!
//! 承载缺陷：`DELETE`（与 REPLACE 的冲突行删除段）把墓碑**带链指针**写入、版本
//! **登记在最后** ⇒ 放弃落在「写出版本槽」与「登记版本」之间时，该墓碑未被登记 ⇒
//! 回收的覆盖面（由登记的时点决定）看不到它 ⇒ 它按「删除者事务状态」压制其前驱
//! 版本 ⇒ 被放弃的 `DELETE` 让它已删除的那一行从数据面消失（登记现场：可见 id =
//! `[2..30]`、29 行）。
//!
//! 夹具形态复用既有先例（`tests/ddl_rewrite_consistency_test.rs`）：真实文件库 +
//! 真实 `Database::open` + 真实语句 future + 锚点式放弃（丢弃语句 future）+ 下一条
//! 写语句触发库属隐式事务回收。锚点用可观察状态（数据链上出现由活跃事务创建的
//! 版本槽）+ `yield_now` 轮询定位；`ANCHOR_TIMEOUT` 只作夹具失控兜底，不是行为判据。
//! 判定全部落在 `cargo test` 内（可见 id 集合 / 行数 / 点查命中数 / 索引条目计数 /
//! 槽头字节 / WAL 记录字段），不引入身份证明机制、不建框架外判定层。

use rtsql::database::Database;
use rtsql::result::SqlOutcome;
use rtsql::storage::page_format::{deserialize_row, ColumnType, RowId, SlottedPageRef};
use rtsql::storage::{PageId, Result as StorageResult};
use rtsql::transaction::VersionHeader;
use rtsql::wal::{WalReader, WalRecord};
use rtsql::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;

/// 建表语句（两列）。
const CREATE_SQL: &str = "CREATE TABLE t (id INT PRIMARY KEY, n INT)";
/// 被测语句：删掉 id = 1 的那一行。
const DELETE_SQL: &str = "DELETE FROM t WHERE id = 1";
/// 被测语句：REPLACE 的冲突行删除段（与 DELETE 同形）。
const REPLACE_SQL: &str = "REPLACE INTO t VALUES (1, 999)";
/// 源表行数（≪ 300——ISS23 约束；登记现场 30 → 29）。
const ROWS: i64 = 30;
/// 锚点观察上限——夹具失控的兜底，触发即判失败（不是产品结论）。
const ANCHOR_TIMEOUT: Duration = Duration::from_secs(10);

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("dml_tombstone_abandon_window.db")
}

/// 建表 + 预置 `ROWS` 行 + 回收触发用的写入目标表。
async fn setup(dir: &TempDir) -> Database {
    let db = Database::open(&db_path(dir)).await.unwrap();
    db.execute_sql(CREATE_SQL).await.unwrap();
    for id in 1..=ROWS {
        db.execute_sql(&format!("INSERT INTO t VALUES ({id}, {})", id * 10))
            .await
            .unwrap();
    }
    // 回收触发用的写入目标表（下一条写语句触发库属隐式事务回收）。
    db.execute_sql("CREATE TABLE t2 (id INT PRIMARY KEY)")
        .await
        .unwrap();
    db
}

/// `SELECT id FROM t` 的可见 id。
async fn scan_id(db: &Database) -> Vec<i64> {
    match db.execute_sql("SELECT id FROM t").await.unwrap() {
        SqlOutcome::Rows { rows, .. } => rows
            .into_iter()
            .map(|row| match row.first() {
                Some(Value::Int(id)) => *id,
                other => panic!("id column must be an int, got {other:?}"),
            })
            .collect(),
        other => panic!("SELECT id must return rows, got {other:?}"),
    }
}

/// 点查：`SELECT * FROM t WHERE id = ?` 的命中行数。
async fn scan_by_pk(db: &Database, id: i64) -> usize {
    match db
        .execute_sql(&format!("SELECT * FROM t WHERE id = {id}"))
        .await
        .unwrap()
    {
        SqlOutcome::Rows { rows, .. } => rows.len(),
        other => panic!("point lookup must return rows, got {other:?}"),
    }
}

/// 主键索引里某个键的条目数（重复条目的观测面）。
async fn pk_entry_count(db: &Database, id: i64) -> usize {
    let meta = db.get_table("t").await.unwrap();
    let key = (id as i64).to_be_bytes();
    meta.index_manager
        .scan_all()
        .await
        .unwrap()
        .into_iter()
        .filter(|(k, _)| k.as_slice() == key.as_slice())
        .count()
}

/// 从 `head` 沿 `next_page_id` 走完整条数据页链，返回页号序列。
async fn chain_pages(db: &Database, head: PageId) -> Vec<u32> {
    let mut pages = Vec::new();
    let mut current = Some(head);
    while let Some(pid) = current {
        let next = db
            .buffer_pool
            .with_page_data(pid, |data| -> StorageResult<u32> {
                Ok(SlottedPageRef::new(data).header().next_page_id)
            })
            .await
            .unwrap();
        pages.push(pid.0 as u32);
        current = if next == 0 {
            None
        } else {
            Some(PageId(next as u64))
        };
    }
    pages
}

/// 数据链上每个槽位的版本头视图（判别读数用）。
#[derive(Debug, Clone)]
struct SlotView {
    page_id: u32,
    slot_id: u16,
    create_tx: u64,
    deleted: bool,
    next: Option<RowId>,
    first_int: Option<i64>,
    /// 槽头原始字节（版本头的逐字节读数）。
    header_bytes: Vec<u8>,
}

impl SlotView {
    fn rid(&self) -> RowId {
        RowId::new(self.page_id, self.slot_id)
    }
}

/// 走完数据页链，收集每个槽位的版本头与首列值。
async fn dump_chain(db: &Database, head: PageId) -> Vec<SlotView> {
    let mut views = Vec::new();
    for page_id in chain_pages(db, head).await {
        let slots = db
            .buffer_pool
            .with_page_data(
                PageId(page_id as u64),
                |data| -> StorageResult<Vec<SlotView>> {
                    let slotted = SlottedPageRef::new(data);
                    let mut slots = Vec::new();
                    for index in 0..slotted.slot_count() {
                        let Some(slot) = slotted.get_slot(index) else {
                            continue;
                        };
                        let slot_data = slotted.get_slot_data(&slot);
                        if slot_data.len() < VersionHeader::SIZE {
                            continue;
                        }
                        let Some(vh) = VersionHeader::from_bytes(&slot_data[..VersionHeader::SIZE])
                        else {
                            continue;
                        };
                        let tuple_bytes = &slot_data[VersionHeader::SIZE..];
                        let first_int =
                            deserialize_row(tuple_bytes, &[ColumnType::Int, ColumnType::Int], &[])
                                .ok()
                                .and_then(|values| match values.first() {
                                    Some(Value::Int(id)) => Some(*id),
                                    _ => None,
                                });
                        slots.push(SlotView {
                            page_id: page_id as u32,
                            slot_id: slot.logical_id,
                            create_tx: vh.create_tx_id(),
                            deleted: vh.is_deleted(),
                            next: vh.next_version(),
                            first_int,
                            header_bytes: slot_data[..VersionHeader::SIZE].to_vec(),
                        });
                    }
                    Ok(slots)
                },
            )
            .await
            .unwrap();
        views.extend(slots);
    }
    views
}

/// 锚点：数据链上已出现由活跃事务创建的版本槽。
///
/// 夹具形态复用既有先例（`tests/ddl_rewrite_consistency_test.rs` 的
/// `anchor_slot_written`）。
async fn anchor_slot_written(db: &Database) -> Option<u64> {
    for tx_id in db.transaction_manager.active_transactions().await {
        let meta = db.get_table("t").await.unwrap();
        for slot in dump_chain(db, meta.data_page_head).await {
            if slot.create_tx == tx_id {
                return Some(tx_id);
            }
        }
    }
    None
}

/// 放弃一条正在执行的语句：锚点命中即丢弃语句 future。
///
/// 返回 `(tx_id, 放弃时该事务是否已登记版本)`——后者用于报告放弃实际落在
/// 「写出版本槽 → 登记版本」窗口的哪一段（语句 future 在锚点命中后不再被轮询，
/// 现场冻结 ⇒ 该读数就是放弃点的现场）。
async fn abandon_statement(db: &Database, sql: &str) -> (u64, bool) {
    let mut statement = Box::pin(db.execute_sql(sql));
    let mut anchor = Box::pin(async {
        tokio::time::timeout(ANCHOR_TIMEOUT, async {
            loop {
                if let Some(tx_id) = anchor_slot_written(db).await {
                    return tx_id;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect(
            "the abandon anchor must be reachable: the fixture statement did not reach it in time",
        )
    });

    let tx_id = tokio::select! {
        biased;
        tx_id = &mut anchor => tx_id,
        outcome = &mut statement => panic!(
            "statement finished before the abandon anchor was reached: {outcome:?}"
        ),
    };

    let registered = !db
        .transaction_manager
        .get_tx_versions(tx_id)
        .await
        .is_empty();
    // 放弃语句 future——执行器停在执行期，隐式事务留在活跃集里。
    drop(statement);
    (tx_id, registered)
}

/// 触发库属隐式事务回收（下一条写语句）。
async fn trigger_reclaim(db: &Database) {
    db.execute_sql("INSERT INTO t2 VALUES (1)").await.unwrap();
}

// ---------------------------------------------------------------------------
// T14 / AR1：被放弃的 `DELETE` 不隐藏它已删除的那一行。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn abandoned_delete_keeps_the_deleted_row_visible() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir).await;

    let before = scan_id(&db).await;
    assert_eq!(
        before.len(),
        ROWS as usize,
        "the fixture must hold {ROWS} rows before the delete"
    );

    // 锚点式放弃 `DELETE FROM t WHERE id = 1`。
    let (tx_id, registered) = abandon_statement(&db, DELETE_SQL).await;
    eprintln!(
        "abandon-window reading: tx {tx_id}; version registered at the abandon point = \
         {registered} (false = 放弃落在「写出版本槽」与「登记版本」之间)"
    );

    // 下一条写语句触发库属隐式事务回收。
    trigger_reclaim(&db).await;

    // AR1-a：逐行可数——可见 id 集合完整、行数不减（登记现场 30 → 29，缺 id = 1）。
    let ids = scan_id(&db).await;
    let id_set: BTreeSet<i64> = ids.iter().copied().collect();
    let expected: BTreeSet<i64> = (1..=ROWS).collect();
    let missing: Vec<i64> = expected.difference(&id_set).copied().collect();
    assert!(
        missing.is_empty(),
        "an abandoned DELETE must not hide the row it deleted; missing ids: {missing:?} \
         (visible: {ids:?})"
    );
    assert_eq!(
        ids.len(),
        ROWS as usize,
        "an abandoned DELETE must not change the visible row count; visible: {ids:?}"
    );

    // AR1-b：点查命中数恒为 1（含被删的那个 id）。
    for id in 1..=ROWS {
        assert_eq!(
            scan_by_pk(&db, id).await,
            1,
            "point lookup for id {id} must hit exactly one row after the reclaim"
        );
    }

    // AR1-c：索引条目面——被删键的条目恰好一条（回收的条目还原不得留下重复条目）。
    assert_eq!(
        pk_entry_count(&db, 1).await,
        1,
        "the reclaimed PK index must hold exactly one entry for the deleted key"
    );

    // AR1-d：回收后同键插入不被假阳性 `DuplicateKey` 拒绝——键在该行真正被删之后
    // 必须可再次使用（残留条目会让它被假阳性拒绝）。
    db.execute_sql(DELETE_SQL).await.unwrap();
    assert_eq!(
        scan_id(&db).await.len(),
        ROWS as usize - 1,
        "the committed DELETE must remove the row"
    );
    db.execute_sql("INSERT INTO t VALUES (1, 777)")
        .await
        .expect(
            "the reclaimed key must be insertable again: a leftover index entry would \
                 reject it with a false-positive DuplicateKey",
        );
    assert_eq!(
        scan_by_pk(&db, 1).await,
        1,
        "the re-inserted row must be reachable by point lookup"
    );
}

// ---------------------------------------------------------------------------
// T14 / AR4：REPLACE 的冲突行删除段与 `DELETE` 同形。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn abandoned_replace_keeps_the_conflict_row_visible() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir).await;

    let before = scan_id(&db).await;
    assert_eq!(
        before.len(),
        ROWS as usize,
        "the fixture must hold {ROWS} rows before the replace"
    );

    // 锚点式放弃 `REPLACE INTO t VALUES (1, 999)`（冲突行的删除段与 DELETE 同形）。
    let (tx_id, registered) = abandon_statement(&db, REPLACE_SQL).await;
    eprintln!(
        "abandon-window reading (REPLACE): tx {tx_id}; version registered at the abandon \
         point = {registered} (false = 放弃落在「写出版本槽」与「登记版本」之间)"
    );

    // 下一条写语句触发库属隐式事务回收。
    trigger_reclaim(&db).await;

    // 逐行可数：可见 id 集合完整、行数不减。
    let ids = scan_id(&db).await;
    let id_set: BTreeSet<i64> = ids.iter().copied().collect();
    let expected: BTreeSet<i64> = (1..=ROWS).collect();
    let missing: Vec<i64> = expected.difference(&id_set).copied().collect();
    assert!(
        missing.is_empty(),
        "an abandoned REPLACE must not hide the conflict row it deleted; missing ids: \
         {missing:?} (visible: {ids:?})"
    );
    assert_eq!(
        ids.len(),
        ROWS as usize,
        "an abandoned REPLACE must not change the visible row count; visible: {ids:?}"
    );

    // 点查命中数恒为 1（含冲突的那个 id）。
    for id in 1..=ROWS {
        assert_eq!(
            scan_by_pk(&db, id).await,
            1,
            "point lookup for id {id} must hit exactly one row after the reclaim"
        );
    }
}

// ---------------------------------------------------------------------------
// T15 / AR3：墓碑槽的终态不变——提交后的删除留下规范编码的墓碑槽。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn committed_delete_keeps_the_canonical_tombstone_slot() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir).await;

    // 一次真实提交的删除。
    db.execute_sql(DELETE_SQL).await.unwrap();

    // 数据面：被删行不可见（既有语义不变）。
    let ids = scan_id(&db).await;
    assert_eq!(
        ids.len(),
        ROWS as usize - 1,
        "a committed DELETE must hide the deleted row; visible: {ids:?}"
    );
    assert!(
        !ids.contains(&1),
        "a committed DELETE must hide the deleted row; visible: {ids:?}"
    );

    // 版本面：墓碑槽的终态字节 = 规范编码（create_tx = 删除者、删除哨兵、
    // next_version → 被删行）。
    let meta = db.get_table("t").await.unwrap();
    let slots = dump_chain(&db, meta.data_page_head).await;
    let deleted_row = slots
        .iter()
        .find(|s| s.first_int == Some(1))
        .expect("the deleted row must keep its own slot (the tombstone is a separate slot)");
    let tombstone = slots
        .iter()
        .find(|s| s.deleted && s.next == Some(deleted_row.rid()))
        .expect("a committed DELETE must leave a tombstone slot pointing at the deleted row");
    let expected_header = VersionHeader::new(tombstone.create_tx, None)
        .with_next_version(deleted_row.rid())
        .mark_deleted()
        .to_bytes();
    assert_eq!(
        tombstone.header_bytes, expected_header,
        "the tombstone slot's final bytes must stay the canonical tombstone encoding \
         (create_tx = the deleter, delete sentinel, next_version -> the deleted row)"
    );
    assert_ne!(
        tombstone.create_tx, 0,
        "a committed tombstone must belong to its deleter transaction"
    );

    // 索引条目面：被删键的条目已清理（条目集合不变）。
    assert_eq!(
        pk_entry_count(&db, 1).await,
        0,
        "a committed DELETE must clear the deleted key's index entry"
    );

    // WAL 面：`Delete` 记录携带**被删行**的 rid（不是墓碑槽的 rid）——恢复期按它
    // 重建墓碑槽。
    db.wal_buffer.as_ref().unwrap().shutdown().await;
    let wal_path = db_path(&dir).with_extension("wal");
    let records = WalReader::open(&wal_path).unwrap().read_all().unwrap();
    let delete_records: Vec<&WalRecord> = records
        .iter()
        .filter(|r| matches!(r, WalRecord::Delete { .. }))
        .collect();
    assert_eq!(
        delete_records.len(),
        1,
        "the committed DELETE must leave exactly one WAL Delete record; records: {records:?}"
    );
    match delete_records[0] {
        WalRecord::Delete {
            row_id, table_name, ..
        } => {
            assert_eq!(table_name, "t", "the WAL Delete record must name the table");
            assert_eq!(
                *row_id,
                deleted_row.rid(),
                "the WAL Delete record must carry the deleted row's rid"
            );
            assert_ne!(
                *row_id,
                tombstone.rid(),
                "the WAL Delete record must not carry the tombstone slot's rid"
            );
        }
        other => panic!("expected a WAL Delete record, got {other:?}"),
    }
}
