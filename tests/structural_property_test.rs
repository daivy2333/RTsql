//! 结构性质测试（MS30-T01 / I091）。
//!
//! 三组性质，全部固定种子可重现、无时间敏感断言、全部位于 `cargo test`
//! 集合内（规格 R3/R4）：
//! 1. 编码往返：`serialize_tuple` → `deserialize_tuple[_width]` 对各类型值
//!    （含 NULL、65535/65536B 长字符串 tag 切换边界）无损；`compute_tuple_size`
//!    与实际写入长度一致。
//! 2. B-Tree 插删查不变量：经 `IndexManager` 插入固定种子 (key, row_id) 序列
//!    （含重复键、乱序插入），`search_all`/`scan_all` 与 BTreeMap 逻辑模型
//!    一致；交错 `delete_exact` 后仍一致。
//! 3. 版本链可见性不变量：经 lib 显式事务的写入/更新/删除/回滚序列后，
//!    可见行集与串行执行模型一致。

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rtsql::storage::page_format::{
    compute_tuple_size, deserialize_tuple, deserialize_tuple_width, serialize_tuple, ColumnType,
    RowId,
};
use rtsql::storage::{btree::IndexManager, BufferPool, FileStorage};
use rtsql::transaction::Transaction;
use rtsql::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tempfile::tempdir;

// ---------------------------------------------------------------------------
// 性质 1：编码往返无损
// ---------------------------------------------------------------------------

/// 单行往返：compute_tuple_size == 实际写入长度；deserialize_tuple 与
/// deserialize_tuple_width 均还原原值（宽度 = schema 长度）。
fn roundtrip(values: &[Value], schema: &[ColumnType]) {
    let size = compute_tuple_size(values, schema);
    let mut buf = vec![0u8; size];
    let written = serialize_tuple(values, schema, &mut buf)
        .unwrap_or_else(|e| panic!("serialize_tuple failed for {values:?}: {e}"));
    assert_eq!(
        written, size,
        "compute_tuple_size {size} != actual written {written} for {values:?}"
    );

    let (decoded, width) = deserialize_tuple_width(&buf[..written], schema)
        .unwrap_or_else(|e| panic!("deserialize_tuple_width failed for {values:?}: {e}"));
    assert_eq!(
        width,
        schema.len(),
        "decoded width {width} != schema width for {values:?}"
    );
    assert_eq!(
        &decoded, values,
        "roundtrip mismatch (width-aware) for {values:?}"
    );

    let exact = deserialize_tuple(&buf[..written], schema)
        .unwrap_or_else(|e| panic!("deserialize_tuple failed for {values:?}: {e}"));
    assert_eq!(&exact, values, "roundtrip mismatch (exact) for {values:?}");
}

/// 长字符串 tag 切换边界：≤65535B 走 `[0x02][u16 len]`，>65535B 走
/// `[0x08][u32 len]`（tuple.rs STRING_U16_MAX 语义）。
#[test]
fn tuple_codec_roundtrip_long_string_boundaries() {
    for len in [0usize, 1, 2, 100, 65534, 65535, 65536, 65537, 70000] {
        let s = "x".repeat(len);
        roundtrip(&[Value::String(s)], &[ColumnType::String(u16::MAX)]);
    }
}

/// 固定值矩阵：五类型 + NULL + 日期族 + 混合行（NULL 交错）+ 全 NULL 行。
#[test]
fn tuple_codec_roundtrip_fixed_matrix() {
    let cases: Vec<(Vec<Value>, Vec<ColumnType>)> = vec![
        (vec![Value::Int(0)], vec![ColumnType::Int]),
        (vec![Value::Int(-1)], vec![ColumnType::Int]),
        (vec![Value::Int(i64::MAX)], vec![ColumnType::Int]),
        (vec![Value::Int(i64::MIN)], vec![ColumnType::Int]),
        (
            vec![Value::String(String::new())],
            vec![ColumnType::String(64)],
        ),
        (
            vec![Value::String("hello world".into())],
            vec![ColumnType::String(64)],
        ),
        // 多字节 UTF-8：长度前缀按字节、内容按字节透明。
        (
            vec![Value::String("héllo 世界 ✓".into())],
            vec![ColumnType::String(64)],
        ),
        (vec![Value::Null], vec![ColumnType::Int]),
        (vec![Value::Null], vec![ColumnType::String(64)]),
        (vec![Value::Float(0.0)], vec![ColumnType::Float]),
        (vec![Value::Float(-1.5)], vec![ColumnType::Float]),
        (vec![Value::Float(f64::MIN)], vec![ColumnType::Float]),
        (vec![Value::Float(f64::MAX)], vec![ColumnType::Float]),
        (vec![Value::Bool(true)], vec![ColumnType::Bool]),
        (vec![Value::Bool(false)], vec![ColumnType::Bool]),
        (vec![Value::Date(0)], vec![ColumnType::Date]),
        (vec![Value::Date(-700_000)], vec![ColumnType::Date]),
        (vec![Value::Timestamp(0)], vec![ColumnType::Timestamp]),
        (
            vec![Value::Timestamp(1_700_000_000_000_000)],
            vec![ColumnType::Timestamp],
        ),
        // 混合行：NULL 交错 + 各类型同行。
        (
            vec![
                Value::Int(5),
                Value::Null,
                Value::String("x".into()),
                Value::Float(2.5),
                Value::Bool(false),
                Value::Date(100),
                Value::Timestamp(999),
            ],
            vec![
                ColumnType::Int,
                ColumnType::Int,
                ColumnType::String(64),
                ColumnType::Float,
                ColumnType::Bool,
                ColumnType::Date,
                ColumnType::Timestamp,
            ],
        ),
        (
            vec![Value::Null, Value::Null, Value::Null],
            vec![ColumnType::Int, ColumnType::String(64), ColumnType::Float],
        ),
    ];
    for (values, schema) in cases {
        roundtrip(&values, &schema);
    }
}

/// 固定种子随机行：六列 schema 200 行，覆盖 NULL 概率、正负数值、
/// 多档字符串长度；种子固定 ⇒ 可重现。
#[test]
fn tuple_codec_roundtrip_seeded_random() {
    let mut rng = StdRng::seed_from_u64(0x50C_2026_1020);
    let schema = vec![
        ColumnType::Int,
        ColumnType::String(u16::MAX),
        ColumnType::Float,
        ColumnType::Bool,
        ColumnType::Date,
        ColumnType::Timestamp,
    ];
    let str_lens = [0usize, 1, 7, 100, 1000];

    for _ in 0..200 {
        let values = vec![
            if rng.gen_bool(0.08) {
                Value::Null
            } else {
                Value::Int(rng.gen_range(-1_000_000..1_000_000))
            },
            if rng.gen_bool(0.08) {
                Value::Null
            } else {
                let len = str_lens[rng.gen_range(0..str_lens.len())];
                Value::String("a".repeat(len))
            },
            if rng.gen_bool(0.08) {
                Value::Null
            } else {
                Value::Float(rng.gen_range(-1_000_000.0..1_000_000.0))
            },
            if rng.gen_bool(0.08) {
                Value::Null
            } else {
                Value::Bool(rng.gen())
            },
            if rng.gen_bool(0.08) {
                Value::Null
            } else {
                Value::Date(rng.gen_range(-100_000..2_000_000))
            },
            if rng.gen_bool(0.08) {
                Value::Null
            } else {
                Value::Timestamp(rng.gen_range(0..2_000_000_000_000_000))
            },
        ];
        roundtrip(&values, &schema);
    }
}

// ---------------------------------------------------------------------------
// 性质 2：B-Tree 插删查不变量
// ---------------------------------------------------------------------------

/// 模型：key（u64，8 字节 BE 编码——定长键使 32 字节零填充比较与数值序
/// 一致）→ row_id (page_id, slot_id) 集合（多重集语义）。
type TreeModel = BTreeMap<u64, BTreeSet<(u32, u16)>>;

fn rid_key(rid: &RowId) -> (u32, u16) {
    (rid.page_id, rid.slot_id)
}

/// scan_all 返回的键经 `Key` 的 len 推断语义去尾部零（如 u64 256 的 BE
/// 8 字节 → 7 字节）；变长大端解析还原原值（空切片 = u64 0）。
fn be_trimmed_to_u64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0u64, |acc, b| (acc << 8) | u64::from(*b))
}

async fn scan_to_model(index: &IndexManager) -> TreeModel {
    let scan = index.scan_all().await.unwrap();
    let mut scan_map: TreeModel = BTreeMap::new();
    for (kb, rid) in scan {
        let key = be_trimmed_to_u64(&kb);
        scan_map.entry(key).or_default().insert(rid_key(&rid));
    }
    scan_map
}

/// 固定种子 (key, row_id) 序列：插入（含重复键）→ 全键查 + 全量枚举对模型 →
/// 种子抽取的交错删除 → 再对模型。
#[tokio::test]
async fn btree_insert_search_delete_matches_logical_model() {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("prop_btree.db")).unwrap());
    let pool = Arc::new(BufferPool::new(64, storage.clone()).unwrap());
    // BTree::new 内部 block_on——沿用 index_manager_test 的 spawn_blocking 形态。
    let index = {
        let pool = pool.clone();
        tokio::task::spawn_blocking(move || IndexManager::new(pool).unwrap())
            .await
            .unwrap()
    };

    let mut rng = StdRng::seed_from_u64(0x7EE_2026_1020);
    let mut model: TreeModel = BTreeMap::new();
    let mut inserted: Vec<(u64, RowId)> = Vec::new();

    // 400 次插入 over 150 个键空间：天然重复 + 20% 显式重复键；row_id 全程唯一。
    for i in 0..400u32 {
        let key = if rng.gen_bool(0.20) && !inserted.is_empty() {
            inserted[rng.gen_range(0..inserted.len())].0
        } else {
            rng.gen_range(0..150)
        };
        let rid = RowId::new(i, 0);
        index.insert(&key.to_be_bytes(), rid).await.unwrap();
        model.entry(key).or_default().insert(rid_key(&rid));
        inserted.push((key, rid));
    }

    for (key, expected) in &model {
        let got = index.search_all(&key.to_be_bytes()).await.unwrap();
        let got_set: BTreeSet<(u32, u16)> = got.iter().map(rid_key).collect();
        assert_eq!(
            &got_set, expected,
            "[after-insert] search_all mismatch for key {key}"
        );
    }
    assert_eq!(
        scan_to_model(&index).await,
        model,
        "[after-insert] scan_all mismatch"
    );

    // 不存在的键：空结果。
    assert!(
        index
            .search_all(&u64::MAX.to_be_bytes())
            .await
            .unwrap()
            .is_empty(),
        "search_all for absent key should be empty"
    );

    // 交错删除：约 40% 的已插入 (key, row_id) 对。
    for (key, rid) in &inserted {
        if rng.gen_bool(0.40) {
            index.delete_exact(&key.to_be_bytes(), *rid).await.unwrap();
            if let Some(set) = model.get_mut(key) {
                set.remove(&rid_key(rid));
                if set.is_empty() {
                    model.remove(key);
                }
            }
        }
    }

    for (key, expected) in &model {
        let got = index.search_all(&key.to_be_bytes()).await.unwrap();
        let got_set: BTreeSet<(u32, u16)> = got.iter().map(rid_key).collect();
        assert_eq!(
            &got_set, expected,
            "[after-delete] search_all mismatch for key {key}"
        );
    }
    assert_eq!(
        scan_to_model(&index).await,
        model,
        "[after-delete] scan_all mismatch"
    );
}

// ---------------------------------------------------------------------------
// 性质 3：版本链可见性不变量
// ---------------------------------------------------------------------------

async fn exec_ok(db: &Database, sql: &str) {
    match to_response(db.execute_sql(sql).await) {
        Response::QueryResult { .. } | Response::AffectedRows { .. } => {}
        Response::Error { message, .. } => panic!("SQL failed: {message}\n{sql}"),
        other => panic!("unexpected response {other:?} for {sql}"),
    }
}

async fn exec_in_tx_ok(db: &Database, sql: &str, tx: &Transaction) {
    match to_response(db.execute_in_tx(sql, tx).await) {
        Response::QueryResult { .. } | Response::AffectedRows { .. } => {}
        Response::Error { message, .. } => panic!("in-tx SQL failed: {message}\n{sql}"),
        other => panic!("unexpected in-tx response {other:?} for {sql}"),
    }
}

/// 快照读行集 == 串行模型（每个提交/回滚相位后调用）。
async fn verify_visible_rows(db: &Database, model: &BTreeMap<i64, i64>, phase: &str) {
    let rows = match to_response(db.execute_sql("SELECT id, bal FROM acct").await) {
        Response::QueryResult { rows } => rows,
        Response::Error { message, .. } => panic!("[{phase}] SELECT failed: {message}"),
        other => panic!("[{phase}] unexpected response {other:?}"),
    };
    let mut got: BTreeMap<i64, i64> = BTreeMap::new();
    for row in rows {
        let id = row[0]
            .as_i64()
            .unwrap_or_else(|| panic!("[{phase}] non-integer id: {:?}", row[0]));
        let bal = row[1]
            .as_i64()
            .unwrap_or_else(|| panic!("[{phase}] non-integer bal: {:?}", row[1]));
        assert!(
            got.insert(id, bal).is_none(),
            "[{phase}] duplicate visible id {id}"
        );
    }
    assert_eq!(
        &got, model,
        "[{phase}] visible row set diverges from serial model"
    );
}

/// 从候选池随机抽 count 个互不重复的 id（池由调用方按存活行构造）。
fn pick_from(rng: &mut StdRng, pool: &BTreeSet<i64>, count: usize) -> Vec<i64> {
    let mut remaining: Vec<i64> = pool.iter().copied().collect();
    assert!(
        remaining.len() >= count,
        "pool {} < count {count}",
        remaining.len()
    );
    let mut picked = Vec::with_capacity(count);
    for _ in 0..count {
        let idx = rng.gen_range(0..remaining.len());
        picked.push(remaining.swap_remove(idx));
    }
    picked
}

fn range_pool(lo: i64, hi: i64) -> BTreeSet<i64> {
    (lo..=hi).collect()
}

/// 固定种子序列：提交插入 → 提交（更新/删除/插入）→ 回滚（插入/更新）→
/// 提交删除 → 回滚 → 提交（复活删除行/更新）；每个相位后快照读对串行模型。
#[tokio::test]
async fn version_chain_visibility_matches_serial_model() {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join("prop_vc.db"))
        .await
        .unwrap();
    exec_ok(&db, "CREATE TABLE acct (id INTEGER PRIMARY KEY, bal INT)").await;

    let mut rng = StdRng::seed_from_u64(0x0CDE_2026_1020);
    let mut model: BTreeMap<i64, i64> = BTreeMap::new();

    // Phase 1：提交插入 ids 1..=20。
    let tx = db.begin().await.unwrap();
    for id in 1..=20i64 {
        let bal = 100 + id;
        exec_in_tx_ok(&db, &format!("INSERT INTO acct VALUES ({id}, {bal})"), &tx).await;
        model.insert(id, bal);
    }
    db.commit(tx).await.unwrap();
    verify_visible_rows(&db, &model, "phase1-commit").await;

    // Phase 2：提交——更新 5 行（bal+100）、删除 3 行（与更新不相交）、
    // 插入 ids 21/22。
    let updated: Vec<i64> = pick_from(&mut rng, &range_pool(1, 20), 5);
    let del_pool: BTreeSet<i64> = range_pool(1, 20)
        .difference(&updated.iter().copied().collect())
        .copied()
        .collect();
    let deleted: Vec<i64> = pick_from(&mut rng, &del_pool, 3);
    let tx = db.begin().await.unwrap();
    for id in &updated {
        exec_in_tx_ok(
            &db,
            &format!("UPDATE acct SET bal = bal + 100 WHERE id = {id}"),
            &tx,
        )
        .await;
        *model.get_mut(id).unwrap() += 100;
    }
    for id in &deleted {
        exec_in_tx_ok(&db, &format!("DELETE FROM acct WHERE id = {id}"), &tx).await;
        model.remove(id);
    }
    for id in [21, 22] {
        let bal = 500 + id;
        exec_in_tx_ok(&db, &format!("INSERT INTO acct VALUES ({id}, {bal})"), &tx).await;
        model.insert(id, bal);
    }
    db.commit(tx).await.unwrap();
    verify_visible_rows(&db, &model, "phase2-commit").await;

    // Phase 3：回滚——插入 id 30、更新 id 1；回滚后模型不变。
    let tx = db.begin().await.unwrap();
    exec_in_tx_ok(&db, "INSERT INTO acct VALUES (30, 999)", &tx).await;
    exec_in_tx_ok(&db, "UPDATE acct SET bal = bal + 7 WHERE id = 1", &tx).await;
    db.rollback(tx).await.unwrap();
    verify_visible_rows(&db, &model, "phase3-rollback").await;

    // Phase 4：提交——从存活行（ids ≤20）删除 4 行、插入 id 23。
    let live: BTreeSet<i64> = model.keys().copied().filter(|id| *id <= 20).collect();
    let deleted2: Vec<i64> = pick_from(&mut rng, &live, 4);
    let tx = db.begin().await.unwrap();
    for id in &deleted2 {
        exec_in_tx_ok(&db, &format!("DELETE FROM acct WHERE id = {id}"), &tx).await;
        model.remove(id);
    }
    exec_in_tx_ok(&db, "INSERT INTO acct VALUES (23, 523)", &tx).await;
    model.insert(23, 523);
    db.commit(tx).await.unwrap();
    verify_visible_rows(&db, &model, "phase4-commit").await;

    // Phase 5：回滚——更新 2 存活行、插入 id 31；回滚后模型不变。
    let live: BTreeSet<i64> = model.keys().copied().collect();
    let touched: Vec<i64> = pick_from(&mut rng, &live, 2);
    let tx = db.begin().await.unwrap();
    for id in &touched {
        exec_in_tx_ok(
            &db,
            &format!("UPDATE acct SET bal = bal + 7 WHERE id = {id}"),
            &tx,
        )
        .await;
    }
    exec_in_tx_ok(&db, "INSERT INTO acct VALUES (31, 631)", &tx).await;
    db.rollback(tx).await.unwrap();
    verify_visible_rows(&db, &model, "phase5-rollback").await;

    // Phase 6：提交——复活 Phase 4 删除的一行（新 bal）、更新另一存活行。
    let revived = deleted2[0];
    let revived_bal = 8000 + revived;
    let live: BTreeSet<i64> = model.keys().copied().filter(|id| *id != revived).collect();
    let target = pick_from(&mut rng, &live, 1)[0];
    let tx = db.begin().await.unwrap();
    exec_in_tx_ok(
        &db,
        &format!("INSERT INTO acct VALUES ({revived}, {revived_bal})"),
        &tx,
    )
    .await;
    model.insert(revived, revived_bal);
    exec_in_tx_ok(
        &db,
        &format!("UPDATE acct SET bal = bal + 1 WHERE id = {target}"),
        &tx,
    )
    .await;
    *model.get_mut(&target).unwrap() += 1;
    db.commit(tx).await.unwrap();
    verify_visible_rows(&db, &model, "phase6-commit").await;
}
