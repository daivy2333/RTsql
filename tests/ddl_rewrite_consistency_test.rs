//! change 2026-10-24-open-defect-batch-storage-ddl-legacy-evidence
//! Iteration 001（Cycle 000-initial）：列改写的数据链一致性与放弃可见性。
//! Iteration 003（Cycle 000-initial）：改写窗口内的读一致性（ISS25）。
//!
//! 承载缺陷：
//! - ISS22：完成的 `ALTER TABLE DROP COLUMN` 在数据页写满后丢行——改写的
//!   DELETE 腿与 INSERT 腿各持一条独立尾指针（`old.data_page_tail` 与
//!   `new_meta.data_page_tail`），页写满时两腿各自分配新页并覆盖同一条链上
//!   的 `next` 指针，先分配的页成为孤儿，落在孤儿页上的新行不可达。
//! - ISS24：被放弃的 `ALTER TABLE DROP COLUMN` 隐藏了它开始改写的那一行。
//! - ISS25：「提交完成 → 目录应用」窗口内并发读该表 ⇒ 按位置解码窄化后的
//!   已提交行 ⇒ 静默错读（`v` 列读到 `w` 的值、`w` 列读 NULL）。
//!
//! 判定全部落在 `cargo test` 内：数据面读 `SELECT` 的可见行，数据链面读
//! 页链可达性与页级普查（数据页首字节 `0x03`，目录页另计），版本面读
//! `VersionHeader` 的 `create_tx_id` / `next_version`。不引入身份证明机制、
//! 不建框架外判定层、不以 timer 阈值作行为判据（放弃夹具的 `ANCHOR_TIMEOUT`
//! 只作夹具失控兜底，与既有先例同）。

use rtsql::database::Database;
use rtsql::executor::ddl_commit_window::{arm_ddl_commit_window, CommitWindowGate};
use rtsql::result::{SqlError, SqlOutcome};
use rtsql::storage::page_format::{deserialize_row, ColumnType, RowId, SlottedPageRef};
use rtsql::storage::{PageId, Result as StorageResult};
use rtsql::transaction::VersionHeader;
use rtsql::Value;
use std::collections::BTreeSet;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;
use tempfile::TempDir;

/// 建表语句（三列）。
const CREATE_SQL: &str = "CREATE TABLE t (id INT PRIMARY KEY, v INT, w INT)";
/// 被测语句：删掉中间列。
const DROP_SQL: &str = "ALTER TABLE t DROP COLUMN v";
/// T4 的源表行数——跨过数据页容量边界（登记现场 40 → 25）。
const ROWS_T4: i64 = 40;
/// T6 / T7 的源表行数——登记现场规模（30 → 29）。
const ROWS_T6: i64 = 30;
/// 数据页（槽页）的首字节类型。
const DATA_PAGE_TYPE: u8 = 0x03;
/// 目录面占用的固定页号（`TABLES_PAGE_ID` / `COLUMNS_PAGE_ID`）。
const CATALOG_ROOTS: [u64; 2] = [0, 1];
/// 锚点观察上限——夹具失控的兜底，触发即判失败（不是产品结论）。
const ANCHOR_TIMEOUT: Duration = Duration::from_secs(10);

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("ddl_rewrite_consistency.db")
}

/// 建表 + 预置 `rows` 行（`t(id INT PRIMARY KEY, v INT, w INT)`，值取
/// `id` / `id * 10` / `id * 100`——三列取值互不相同，行宽错误可被读出）。
async fn setup(dir: &TempDir, rows: i64) -> Database {
    let db = Database::open(&db_path(dir)).await.unwrap();
    db.execute_sql(CREATE_SQL).await.unwrap();
    for id in 1..=rows {
        db.execute_sql(&format!(
            "INSERT INTO t VALUES ({id}, {}, {})",
            id * 10,
            id * 100
        ))
        .await
        .unwrap();
    }
    db
}

/// `SELECT * FROM t` 的可见行（表扫描路径，不经索引）。
async fn scan_star(db: &Database) -> Vec<Vec<Value>> {
    match db.execute_sql("SELECT * FROM t").await.unwrap() {
        SqlOutcome::Rows { rows, .. } => rows,
        other => panic!("SELECT * must return rows, got {other:?}"),
    }
}

/// `SELECT id FROM t` 的可见 id（保持 ISS24 登记时的查询形态）。
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

/// 库内全部数据页（首字节 `0x03`），排除目录面自己的页链。
async fn data_pages(db: &Database) -> BTreeSet<u32> {
    let mut catalog_pages = BTreeSet::new();
    for root in CATALOG_ROOTS {
        for page in chain_pages(db, PageId(root)).await {
            catalog_pages.insert(page);
        }
    }
    let page_count = db.buffer_pool.storage().page_count();
    let mut pages = BTreeSet::new();
    for page_id in 0..page_count {
        if catalog_pages.contains(&(page_id as u32)) {
            continue;
        }
        let is_data = db
            .buffer_pool
            .with_page_data(PageId(page_id), |data| -> StorageResult<bool> {
                Ok(data.first().copied() == Some(DATA_PAGE_TYPE))
            })
            .await
            .unwrap();
        if is_data {
            pages.insert(page_id as u32);
        }
    }
    pages
}

/// 主键索引条目指向的行所在页（活行在链上的落点）。
async fn live_row_pages(db: &Database, ids: &[i64]) -> BTreeSet<u32> {
    let meta = db.get_table("t").await.unwrap();
    let mut pages = BTreeSet::new();
    for id in ids {
        let key = id.to_be_bytes();
        let row_id = meta
            .index_manager
            .search(&key)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("primary key entry for id {id} must exist"));
        pages.insert(row_id.page_id);
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
                        let first_int = deserialize_row(
                            tuple_bytes,
                            &[ColumnType::Int, ColumnType::Int, ColumnType::Int],
                            &[],
                        )
                        .ok()
                        .and_then(|values| match values.first() {
                            Some(Value::Int(id)) => Some(*id),
                            _ => None,
                        });
                        slots.push(SlotView {
                            page_id,
                            slot_id: slot.logical_id,
                            create_tx: vh.create_tx_id(),
                            deleted: vh.is_deleted(),
                            next: vh.next_version(),
                            first_int,
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

/// 锚点：活跃事务集合里出现「已记录版本数 ≥2」的事务。
///
/// 改写循环逐行登记版本（每行产出墓碑 + 新行两个版本）⇒ 锚点命中即
/// 「语句已进入数据面改写」。夹具形态复用既有先例
/// （`tests/ddl_catalog_transactional_test.rs:63-105`）。
async fn anchor_tx(db: &Database) -> Option<u64> {
    for tx_id in db.transaction_manager.active_transactions().await {
        if db.transaction_manager.get_tx_versions(tx_id).await.len() >= 2 {
            return Some(tx_id);
        }
    }
    None
}

/// 放弃一条正在执行的语句：锚点命中即丢弃语句 future。
///
/// 语句在锚点命中前就跑完 ⇒ 夹具规模不对（锚点必须在语句完成前可达），
/// 此时直接失败而不是把「跑完」当成被放弃。
async fn abandon_statement(db: &Database, sql: &str) -> u64 {
    let mut statement = Box::pin(db.execute_sql(sql));
    let mut anchor = Box::pin(async {
        tokio::time::timeout(ANCHOR_TIMEOUT, async {
            loop {
                if let Some(tx_id) = anchor_tx(db).await {
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

    // 放弃语句 future——执行器停在执行期，隐式事务留在活跃集里。
    drop(statement);
    tx_id
}

// ---------------------------------------------------------------------------
// T4 / AR1：完成的列改写跨数据页边界后行数守恒，且数据页链不分叉。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn drop_column_across_page_boundary_keeps_every_row_on_one_chain() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir, ROWS_T4).await;

    let before = scan_star(&db).await;
    assert_eq!(
        before.len(),
        ROWS_T4 as usize,
        "fixture must hold {ROWS_T4} rows before the rewrite (ISS22 hits N>30)"
    );

    db.execute_sql(DROP_SQL).await.unwrap();

    // AR1-a：行数守恒。
    let after = scan_star(&db).await;
    assert_eq!(
        after.len(),
        ROWS_T4 as usize,
        "completed DROP COLUMN must preserve every row; got {} of {ROWS_T4}",
        after.len()
    );

    // AR1-b：活行集合不变（逐行可数）。
    let ids_after: BTreeSet<i64> = scan_id(&db).await.into_iter().collect();
    let ids_before: BTreeSet<i64> = before
        .iter()
        .map(|row| match row.first() {
            Some(Value::Int(id)) => *id,
            other => panic!("id column must be an int, got {other:?}"),
        })
        .collect();
    assert_eq!(
        ids_after, ids_before,
        "completed DROP COLUMN must preserve the row set"
    );

    // AR1-c：改写后的行全部落在同一条数据页链上（无孤儿页）。
    let meta = db.get_table("t").await.unwrap();
    let chain: BTreeSet<u32> = chain_pages(&db, meta.data_page_head)
        .await
        .into_iter()
        .collect();
    let row_pages = live_row_pages(&db, &ids_after.iter().copied().collect::<Vec<_>>()).await;
    let off_chain_rows: Vec<u32> = row_pages.difference(&chain).copied().collect();
    assert!(
        off_chain_rows.is_empty(),
        "every rewritten row must live on the data chain; off-chain row pages: {off_chain_rows:?} (chain: {chain:?})"
    );

    // AR1-d：库内不存在掉出链的数据页（改写期间分配的页全部可达）。
    let orphans: Vec<u32> = data_pages(&db).await.difference(&chain).copied().collect();
    assert!(
        orphans.is_empty(),
        "the rewrite must not orphan a data page; orphan pages: {orphans:?} (chain: {chain:?})"
    );
}

// ---------------------------------------------------------------------------
// T6 / AR2：被放弃的列改写在回收后不隐藏它已开始改写的行。
//
// 判别读数（design.md D5 协议，硬性）——RED 必须给出：
//   1. M1 读数：缺失行（改写已触及的行）的槽位是否在数据页链上；
//   2. M2 读数：该行的版本链上是否存在 `create_tx_id == 0` 的指向者；
//   3. 层次读数：缺失行在「全表扫描 / 点查与索引扫描 / 计划期活行判定 /
//      墓碑中性化」四层中的哪一层消失。
// ---------------------------------------------------------------------------

/// 一次放弃 + 回收后的判别读数。
#[derive(Debug)]
struct Readings {
    /// 被放弃的改写已经触及的旧行（其墓碑的 `next_version` 目标）。
    touched: Vec<RowId>,
    /// 触及行所在页是否都在链上（M1 读数）。
    touched_pages_on_chain: bool,
    /// 触及行的指向者里是否有已回滚（`create_tx_id == 0`）的（M2 读数）。
    has_aborted_pointer: bool,
    /// 尚未被中和的指向者的 `create_tx_id`（未中和 ⇒ 仍按删除者事务状态求值）。
    pointer_create_tx: Vec<u64>,
    /// 指向者是否全部已被中和（`create_tx_id == 0`）。
    pointers_neutralized: bool,
    /// 触及行的主键索引条目是否在位。
    pk_entry_present: Vec<bool>,
}

/// 读出一次放弃 + 回收后的判别读数。
///
/// 墓碑归属：本夹具里被放弃的改写是唯一被回滚的事务 ⇒ 该事务的墓碑要么
/// 仍带着它的 `create_tx_id`（未被中和），要么已被中和为 `create_tx_id == 0`。
/// 两者都计入「该事务写下的指向者」。
async fn readings(db: &Database, tx_id: u64) -> Readings {
    let meta = db.get_table("t").await.unwrap();
    let chain: BTreeSet<u32> = chain_pages(db, meta.data_page_head)
        .await
        .into_iter()
        .collect();
    let slots = dump_chain(db, meta.data_page_head).await;

    // 该事务写下的墓碑（指向者）：未被中和的仍带它的 tx id，已中和的带 0。
    let pointers: Vec<&SlotView> = slots
        .iter()
        .filter(|s| s.deleted && s.next.is_some() && (s.create_tx == tx_id || s.create_tx == 0))
        .collect();
    let touched: Vec<RowId> = pointers.iter().filter_map(|s| s.next).collect();

    let touched_pages_on_chain = touched.iter().all(|rid| chain.contains(&rid.page_id));
    let pointer_create_tx: Vec<u64> = pointers.iter().map(|s| s.create_tx).collect();
    let has_aborted_pointer = pointers.iter().any(|s| s.create_tx == 0);
    let pointers_neutralized = !pointers.is_empty() && pointers.iter().all(|s| s.create_tx == 0);

    let mut pk_entry_present = Vec::new();
    for rid in &touched {
        let old = slots.iter().find(|s| s.rid() == *rid);
        match old.and_then(|s| s.first_int) {
            Some(id) => {
                let key = id.to_be_bytes();
                pk_entry_present.push(meta.index_manager.search(&key).await.unwrap().is_some());
            }
            None => pk_entry_present.push(false),
        }
    }

    Readings {
        touched,
        touched_pages_on_chain,
        has_aborted_pointer,
        pointer_create_tx,
        pointers_neutralized,
        pk_entry_present,
    }
}

#[tokio::test]
async fn abandoned_drop_column_keeps_rewritten_row_visible() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir, ROWS_T6).await;
    // 回收触发用的写入目标表。
    db.execute_sql("CREATE TABLE t2 (id INT PRIMARY KEY)")
        .await
        .unwrap();

    // 放弃改写（锚点 = 活跃事务已记录版本数 ≥ 2 ⇒ 语句已进入数据面改写）。
    let tx_id = abandon_statement(&db, DROP_SQL).await;

    // 下一条写语句触发库属隐式事务回收。
    db.execute_sql("INSERT INTO t2 VALUES (1)").await.unwrap();

    let read = readings(&db, tx_id).await;
    eprintln!("ISS24 discriminating readings: {read:#?}");
    eprintln!(
        "ISS24 layer reading: touched rows' pointer create_tx = {:?} (0 == neutralized); \
         touched pages on chain = {}; aborted pointer present = {}; pk entries present = {:?}",
        read.pointer_create_tx,
        read.touched_pages_on_chain,
        read.has_aborted_pointer,
        read.pk_entry_present,
    );

    // AR2-a：逐行可数——行数不减、行集合完整（登记现场 30 → 29）。
    let ids = scan_id(&db).await;
    let id_set: BTreeSet<i64> = ids.iter().copied().collect();
    let expected: BTreeSet<i64> = (1..=ROWS_T6).collect();
    let missing: Vec<i64> = expected.difference(&id_set).copied().collect();
    assert!(
        missing.is_empty(),
        "the abandoned rewrite must not hide the row it started rewriting; missing ids: {missing:?} (visible: {ids:?})"
    );
    assert_eq!(
        ids.len(),
        ROWS_T6 as usize,
        "the abandoned rewrite must not change the visible row count; visible: {ids:?}"
    );

    // AR2-b：点查命中（索引条目面）。
    for id in 1..=ROWS_T6 {
        assert_eq!(
            scan_by_pk(&db, id).await,
            1,
            "point lookup for id {id} must hit exactly one row after the reclaim"
        );
    }

    // AR2-c：墓碑中性化——被放弃的改写留下的指向者必须已被中和
    // （`create_tx_id == 0`），否则它按删除者事务状态压制其前驱版本。
    assert!(
        read.pointers_neutralized,
        "every tombstone the abandoned rewrite wrote must be neutralized (create_tx_id == 0); \
         pointer create_tx ids: {:?}; touched rows: {:?}",
        read.pointer_create_tx, read.touched
    );
}

// ---------------------------------------------------------------------------
// T6 / DCT R3：放弃的删列改写——重开库后目录与数据面一致。
// ---------------------------------------------------------------------------

/// 目录面（列定义 + 列数）与数据面（行数 + 行宽）的紧凑快照。
#[derive(Debug)]
struct Shape {
    columns: Vec<String>,
    column_count: u32,
    row_count: usize,
    widths: BTreeSet<usize>,
}

async fn observe_shape(db: &Database) -> Shape {
    let mut cols = db.table_manager.catalog().scan_columns("t").await.unwrap();
    cols.sort_by_key(|c| c.column_index);
    let columns: Vec<String> = cols.into_iter().map(|c| c.column_name).collect();

    let column_count = db
        .table_manager
        .catalog()
        .scan_tables()
        .await
        .unwrap()
        .into_iter()
        .find(|r| r.table_name == "t")
        .expect("the table must have a catalog row")
        .column_count;

    let rows = scan_star(db).await;
    Shape {
        columns,
        column_count,
        row_count: rows.len(),
        widths: rows.iter().map(|r| r.len()).collect(),
    }
}

#[tokio::test]
async fn abandoned_drop_column_reopen_keeps_all_rows() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir, ROWS_T6).await;
    db.execute_sql("CREATE TABLE t2 (id INT PRIMARY KEY)")
        .await
        .unwrap();

    // 放弃改写 + 回收。
    let _tx_id = abandon_statement(&db, DROP_SQL).await;
    db.execute_sql("INSERT INTO t2 VALUES (1)").await.unwrap();

    // 放弃的语句不留目录面变更，数据面行数不减、行宽不变。
    let after = observe_shape(&db).await;
    assert_eq!(
        after.columns,
        vec!["id", "v", "w"],
        "the abandoned DDL must leave the catalog columns unchanged, got {after:?}"
    );
    assert_eq!(
        after.column_count, 3,
        "the abandoned DDL must leave the catalog column count unchanged, got {after:?}"
    );
    assert_eq!(
        after.row_count, ROWS_T6 as usize,
        "the abandoned DDL must not change the visible row count, got {after:?}"
    );
    assert_eq!(
        after.widths,
        BTreeSet::from([3usize]),
        "every row must keep the pre-statement width, got {after:?}"
    );

    // 重开库后：目录面与数据面仍一致（DCT R3）。
    db.close().await.unwrap();
    drop(db);
    let db = Database::open(&db_path(&dir)).await.unwrap();

    let reopened = observe_shape(&db).await;
    assert_eq!(
        reopened.columns,
        vec!["id", "v", "w"],
        "after close + reopen: the abandoned DDL must leave the catalog columns unchanged, got {reopened:?}"
    );
    assert_eq!(
        reopened.column_count, 3,
        "after close + reopen: the catalog column count must stay at the pre-statement count, got {reopened:?}"
    );
    assert_eq!(
        reopened.row_count, ROWS_T6 as usize,
        "after close + reopen: every row must still be visible, got {reopened:?}"
    );
    assert_eq!(
        reopened.widths,
        BTreeSet::from([3usize]),
        "after close + reopen: every row must keep the pre-statement width, got {reopened:?}"
    );
}

// ---------------------------------------------------------------------------
// T7 / MTV R1：已回滚的墓碑不抑制其前驱版本（抑制集合的活性规则）。
//
// 构造：被放弃的改写已回滚 ⇒ 其墓碑带 `create_tx_id == 0` 但**保留链指针**
// （`VersionHeader::mark_aborted` 的既有语义）。抑制集合若不识别 aborted 标记，
// 就会把该前驱版本当成「被指向的版本」⇒ 后续改写**跳过**它 ⇒ 该行的数据
// 停留在旧行宽 ⇒ 数据面与目录面不一致。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn rolled_back_rewrite_does_not_suppress_predecessor_for_later_rewrite() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir, ROWS_T6).await;
    db.execute_sql("CREATE TABLE t2 (id INT PRIMARY KEY)")
        .await
        .unwrap();

    // 放弃改写（删**末列** `w`）+ 回收。改写的墓碑指向被触及行的旧版本。
    let tx_id = abandon_statement(&db, "ALTER TABLE t DROP COLUMN w").await;
    db.execute_sql("INSERT INTO t2 VALUES (1)").await.unwrap();

    let read = readings(&db, tx_id).await;
    eprintln!("M2 discriminating readings: {read:#?}");

    // 再删中间列 `v`（正常完成）⇒ 改写必须覆盖**每一行**，剩余列 `w` 的值
    // 必须是原表的第三列（`id * 100`）。若改写跳过了被已回滚墓碑指向的行，
    // 该行仍持有旧行宽 ⇒ `w` 读到原表的第二列（`id * 10`）。
    db.execute_sql(DROP_SQL).await.unwrap();

    // 断言逐行可数。
    match db.execute_sql("SELECT id, w FROM t").await.unwrap() {
        SqlOutcome::Rows { rows, .. } => {
            let mut wrong = Vec::new();
            for row in rows {
                let id = match row.first() {
                    Some(Value::Int(id)) => *id,
                    other => panic!("id column must be an int, got {other:?}"),
                };
                let value = match row.get(1) {
                    Some(Value::Int(v)) => *v,
                    other => panic!("w column must be an int, got {other:?}"),
                };
                if value != id * 100 {
                    wrong.push((id, value));
                }
            }
            assert!(
                wrong.is_empty(),
                "a rolled-back rewrite must not suppress its predecessor version for later \
                 rewrites; rows keeping the stale width: {wrong:?}"
            );
        }
        other => panic!("SELECT id, w must return rows, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// T6 / ISS24：更窄的放弃窗口——放弃落在「写出版本槽」与「登记版本」之间。
//
// 上面的用例用既有夹具锚点（已登记版本数 ≥ 2）⇒ 放弃必然落在**登记之后**。
// 本用例的锚点更早：链上已出现由活跃事务创建的版本槽（该槽尚未登记）⇒
// 放弃落在「版本槽已写、版本未登记」的窗口。此时回收看不到该版本——若墓碑
// 已带链指针，它会按删除者的事务状态压制其前驱版本 ⇒ 被放弃的改写隐藏了
// 它开始改写的行，且该行的主键条目已被删除且不再被还原。
// ---------------------------------------------------------------------------

/// 锚点：链上已出现由活跃事务创建的版本槽（该槽尚未登记）。
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

#[tokio::test]
async fn abandoned_drop_column_before_version_registration_keeps_row_visible() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir, ROWS_T6).await;
    db.execute_sql("CREATE TABLE t2 (id INT PRIMARY KEY)")
        .await
        .unwrap();

    // 放弃改写，锚点 = 「链上已出现本事务写的版本槽」。
    let mut statement = Box::pin(db.execute_sql(DROP_SQL));
    let mut anchor = Box::pin(async {
        tokio::time::timeout(ANCHOR_TIMEOUT, async {
            loop {
                if let Some(tx_id) = anchor_slot_written(&db).await {
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
    drop(statement);
    let _ = tx_id;

    // 下一条写语句触发回收。
    db.execute_sql("INSERT INTO t2 VALUES (1)").await.unwrap();

    // 逐行可数：行数不减、行集合完整。
    let ids = scan_id(&db).await;
    let id_set: BTreeSet<i64> = ids.iter().copied().collect();
    let expected: BTreeSet<i64> = (1..=ROWS_T6).collect();
    let missing: Vec<i64> = expected.difference(&id_set).copied().collect();
    assert!(
        missing.is_empty(),
        "an abandoned rewrite must not hide the row it started rewriting, even when the \
         abandon lands before the version is registered; missing ids: {missing:?} (visible: {ids:?})"
    );

    // 点查命中（索引条目面）。
    for id in 1..=ROWS_T6 {
        assert_eq!(
            scan_by_pk(&db, id).await,
            1,
            "point lookup for id {id} must hit exactly one row after the reclaim"
        );
    }
}

// ---------------------------------------------------------------------------
// T6 / ISS24：INSERT 腿的放弃窗口——窄化副本已写、尚未登记。
//
// 窄化副本（`VersionHeader::new(tx_id, None)`）在登记之前被放弃时，回收
// 看不到它 ⇒ 它既不被中和（`is_deleted` 不成立）也不被提交 ⇒ 无快照的扫描
// 仍会把它当作可见行（与旧行重复）。锚点 = 「链上已出现本事务写的**非墓碑**
// 版本槽」。
// ---------------------------------------------------------------------------

/// 锚点：链上已出现由活跃事务创建的**非墓碑**版本槽（窄化副本）。
async fn anchor_narrow_copy_written(db: &Database) -> Option<u64> {
    for tx_id in db.transaction_manager.active_transactions().await {
        let meta = db.get_table("t").await.unwrap();
        for slot in dump_chain(db, meta.data_page_head).await {
            if slot.create_tx == tx_id && !slot.deleted {
                return Some(tx_id);
            }
        }
    }
    None
}

#[tokio::test]
async fn abandoned_drop_column_before_narrow_copy_registration_keeps_row_count() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir, ROWS_T6).await;
    db.execute_sql("CREATE TABLE t2 (id INT PRIMARY KEY)")
        .await
        .unwrap();

    let mut statement = Box::pin(db.execute_sql(DROP_SQL));
    let mut anchor = Box::pin(async {
        tokio::time::timeout(ANCHOR_TIMEOUT, async {
            loop {
                if let Some(tx_id) = anchor_narrow_copy_written(&db).await {
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
    drop(statement);
    let _ = tx_id;

    db.execute_sql("INSERT INTO t2 VALUES (1)").await.unwrap();

    // 逐行可数：行数不减、id 集合完整（窄化副本不得作为额外的可见行残留）。
    let ids = scan_id(&db).await;
    let id_set: BTreeSet<i64> = ids.iter().copied().collect();
    let expected: BTreeSet<i64> = (1..=ROWS_T6).collect();
    let missing: Vec<i64> = expected.difference(&id_set).copied().collect();
    assert!(
        missing.is_empty(),
        "an abandoned rewrite must not hide a row; missing ids: {missing:?} (visible: {ids:?})"
    );
    assert_eq!(
        ids.len(),
        ROWS_T6 as usize,
        "an abandoned rewrite must not leave a visible narrow copy behind; visible: {ids:?}"
    );
}

// ---------------------------------------------------------------------------
// T10 / T11：改写在途窗口内的读一致性（ISS25）。
//
// 窗口 = 「提交完成 → 目录应用」：数据面已是窄化后的已提交行，目录与活布局
// 仍是旧布局 ⇒ 按位置解码读到错位值（`v` 列读到 `w` 的值、`w` 列读 NULL）。
//
// 夹具形态：路径限定 + 一次消费 + 默认不生效的注入点
// （`rtsql::executor::ddl_commit_window`）——真实暂停在窗口内，不是替身。
// 锚点用可观察状态（`gate.reached()`）+ `yield_now` 轮询定位；`ANCHOR_TIMEOUT`
// 只作夹具失控兜底，不是行为判据。
// ---------------------------------------------------------------------------

/// T10 / T11 的源表行数——≪ 300（ISS23 约束）。
const ROWS_T10: i64 = 5;

/// 把一条语句停在「提交完成 → 目录应用」窗口内。
///
/// 可观察锚点（`gate.reached()`）+ `yield_now` 轮询定位窗口。语句在锚点命中前
/// 就跑完 ⇒ 夹具失效，直接失败（不把「跑完」当成窗口）。语句在窗口内**不被
/// 推进**——调用方观测完成后 `gate.release()` 再 await 语句。
async fn park_at_commit_window<F>(mut statement: &mut Pin<Box<F>>, gate: &CommitWindowGate)
where
    F: Future<Output = Result<SqlOutcome, SqlError>>,
{
    let mut anchor = Box::pin(async {
        tokio::time::timeout(ANCHOR_TIMEOUT, async {
            loop {
                if gate.reached() {
                    return true;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the commit-window anchor must be reachable: the fixture statement did not reach it in time")
    });
    tokio::select! {
        biased;
        _ = &mut anchor => {}
        outcome = &mut statement => panic!(
            "statement finished before the commit window was reached: {outcome:?}"
        ),
    };
}

/// T10 / ISS25：在途窄化的表被读时必须被**明确拒绝**（不得返回按位置错位的值）。
///
/// 变更前的失败形态 = 读到错位值（`v` 列显示 `w` 的值、`w` 列显示 NULL）——
/// 即 ISS25 的真实复现，而不是「读到旧宽行」「读到空结果」或「读到明确错误」。
#[tokio::test]
async fn read_during_narrowing_in_flight_is_refused() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir, ROWS_T10).await;

    let gate = arm_ddl_commit_window(&db_path(&dir));
    let mut statement = Box::pin(db.execute_sql(DROP_SQL));
    park_at_commit_window(&mut statement, &gate).await;

    // 窗口内读该表：必须被明确拒绝（点名该表正在被修改）。
    match db.execute_sql("SELECT * FROM t").await {
        Err(err) => {
            let message = err.to_string();
            assert!(
                message.contains("'t'"),
                "the refusal must name the table being modified, got: {message}"
            );
            assert!(
                message.contains("ALTER TABLE") || message.contains("being modified"),
                "the refusal must say the table is being modified, got: {message}"
            );
        }
        Ok(SqlOutcome::Rows { rows, .. }) => panic!(
            "reading a table whose DROP COLUMN rewrite is in flight must be refused; \
             the read returned misaligned rows instead: {rows:?}"
        ),
        Ok(other) => panic!(
            "reading a table whose DROP COLUMN rewrite is in flight must be refused; got {other:?}"
        ),
    }

    // 放行 → 语句正常走完（目录窄化生效）。
    gate.release();
    let outcome = statement.await;
    assert!(
        outcome.is_ok(),
        "the DROP COLUMN statement must succeed after the window is released: {outcome:?}"
    );
}

/// T11 对照一：改写完成后读路径恢复正常（窄化后的行、正确的列值）。
#[tokio::test]
async fn read_after_narrowing_completes_returns_narrowed_rows() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir, ROWS_T10).await;

    let gate = arm_ddl_commit_window(&db_path(&dir));
    let mut statement = Box::pin(db.execute_sql(DROP_SQL));
    park_at_commit_window(&mut statement, &gate).await;
    gate.release();
    statement
        .await
        .unwrap_or_else(|e| panic!("the DROP COLUMN statement must succeed: {e:?}"));

    // 改写完成后：两列、每行 = (id, w 的值)，不得再给在途拒绝。
    let rows = scan_star(&db).await;
    assert_eq!(
        rows.len(),
        ROWS_T10 as usize,
        "the completed rewrite must keep every row; got {rows:?}"
    );
    let mut seen: BTreeSet<(i64, i64)> = BTreeSet::new();
    for row in &rows {
        assert_eq!(
            row.len(),
            2,
            "the narrowed table must expose two columns; got {row:?}"
        );
        match (row.first(), row.get(1)) {
            (Some(Value::Int(id)), Some(Value::Int(w))) => {
                seen.insert((*id, *w));
            }
            other => panic!("the narrowed row must be (id, w); got {other:?}"),
        }
    }
    let expected: BTreeSet<(i64, i64)> = (1..=ROWS_T10).map(|id| (id, id * 100)).collect();
    assert_eq!(
        seen, expected,
        "the completed rewrite must expose the kept column's own values; got {rows:?}"
    );
}

/// T11 对照二：未改写的表在同一窗口内读路径不受影响（不得被在途拒绝误伤）。
#[tokio::test]
async fn read_of_other_table_during_narrowing_is_unaffected() {
    let dir = TempDir::new().unwrap();
    let db = setup(&dir, ROWS_T10).await;
    db.execute_sql("CREATE TABLE t2 (id INT PRIMARY KEY, x INT)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t2 VALUES (1, 11)")
        .await
        .unwrap();

    let gate = arm_ddl_commit_window(&db_path(&dir));
    let mut statement = Box::pin(db.execute_sql(DROP_SQL));
    park_at_commit_window(&mut statement, &gate).await;

    // 未改写的表：正常返回其行（窗口只约束被改写的那张表）。
    match db.execute_sql("SELECT * FROM t2").await.unwrap() {
        SqlOutcome::Rows { rows, .. } => assert_eq!(
            rows,
            vec![vec![Value::Int(1), Value::Int(11)]],
            "an untouched table must stay readable during another table's narrowing"
        ),
        other => panic!("SELECT * FROM t2 must return rows, got {other:?}"),
    }

    gate.release();
    statement
        .await
        .unwrap_or_else(|e| panic!("the DROP COLUMN statement must succeed: {e:?}"));
}
