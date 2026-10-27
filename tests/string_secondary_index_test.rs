//! 存储层二级索引的字符串生命周期。
//!
//! SQL `CREATE INDEX` 接受字符串列。本文件观察的索引仍由 `CreateIndexExecutor` 建立。
//! 条目用 `search_all` / `scan_all` 观察，行用 `SELECT *` 观察。

use rtsql::database::Database;
use rtsql::executor::{CreateIndexExecutor, CreateIndexNode, ExecResult, Executor, PhysicalPlan};
use rtsql::storage::integrity::check_integrity;
use rtsql::storage::page_format::deserialize_row;
use rtsql::storage::{read_tuple_from_data_page, ColumnType, IndexManager, Key, RowId};
use rtsql::{SqlOutcome, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tempfile::tempdir;

fn db_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(name)
}

async fn exec(db: &Database, sql: &str) {
    db.execute_sql(sql)
        .await
        .unwrap_or_else(|error| panic!("{sql} failed: {error}"));
}

async fn select_all(db: &Database, sql: &str) -> Vec<Vec<Value>> {
    match db.execute_sql(sql).await.expect(sql) {
        SqlOutcome::Rows { rows, .. } => rows,
        other => panic!("{sql}: expected rows, got {other:?}"),
    }
}

async fn string_column(db: &Database, table: &str, name: &str) -> (usize, ColumnType) {
    let meta = db.get_table(table).await.expect(table);
    let (index, (_, column_type)) = meta
        .columns
        .iter()
        .enumerate()
        .find(|(_, (column_name, _))| column_name == name)
        .unwrap_or_else(|| panic!("{table}.{name} is missing"));
    assert!(
        matches!(column_type, ColumnType::String(_)),
        "{table}.{name} is {:?}; string index lifecycle requires a string column",
        column_type
    );
    (index, column_type.clone())
}

fn key_bytes(text: &str, column_type: &ColumnType) -> Vec<u8> {
    Value::String(text.to_string())
        .secondary_index_key(column_type)
        .unwrap_or_else(|| panic!("{text:?} must fit in a secondary key"))
        .as_bytes()
        .to_vec()
}

fn stored_key(bytes: &[u8]) -> [u8; 32] {
    *Key::new(bytes).full_data()
}

async fn index_of(db: &Database, table: &str, name: &str) -> Arc<IndexManager> {
    let meta = db.get_table(table).await.expect(table);
    meta.secondary_indexes
        .iter()
        .find(|index| index.name == name)
        .unwrap_or_else(|| panic!("{table} has no index {name}"))
        .index
        .clone()
}

async fn rids_for(index: &IndexManager, key: &[u8]) -> Vec<RowId> {
    let mut found = index.search_all(key).await.expect("search_all");
    found.sort_by_key(|rid| (rid.page_id, rid.slot_id));
    found
}

async fn scanned_keys(index: &IndexManager) -> Vec<[u8; 32]> {
    let mut keys = index
        .scan_all()
        .await
        .expect("scan_all")
        .into_iter()
        .map(|(bytes, _)| stored_key(&bytes))
        .collect::<Vec<_>>();
    keys.sort();
    keys
}

async fn create_string_index(
    db: &Arc<Database>,
    table: &str,
    index: &str,
    column_index: usize,
    column_name: &str,
) {
    let plan = PhysicalPlan::CreateIndex(CreateIndexNode {
        index_name: index.to_string(),
        table_name: table.to_string(),
        column_index,
        column_name: column_name.to_string(),
    });
    let mut executor = CreateIndexExecutor::new(plan, Arc::clone(db));
    let outcome = executor.next().await.expect("create string index");
    assert!(matches!(outcome, Some(ExecResult::AffectedRows(_))));
}

async fn row_at(db: &Database, table: &str, rid: RowId) -> Vec<Value> {
    let meta = db.get_table(table).await.expect(table);
    let schema: Vec<ColumnType> = meta
        .columns
        .iter()
        .map(|(_, column_type)| column_type.clone())
        .collect();
    let bytes = read_tuple_from_data_page(&db.buffer_pool, rid, |_, bytes| Ok(bytes.to_vec()))
        .await
        .expect("row at index rid");
    deserialize_row(&bytes, &schema, &meta.defaults).expect("decode indexed row")
}

fn sorted_by_id(mut rows: Vec<Vec<Value>>) -> Vec<Vec<Value>> {
    rows.sort_by(|left, right| match (&left[0], &right[0]) {
        (Value::Int(a), Value::Int(b)) => a.cmp(b),
        _ => panic!("id column must be an integer"),
    });
    rows
}

#[tokio::test]
async fn string_index_backfills_duplicates_null_and_utf8() {
    let dir = tempdir().unwrap();
    let db = Arc::new(
        Database::open(&db_path(dir.path(), "backfill"))
            .await
            .unwrap(),
    );
    exec(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR, note INT)",
    )
    .await;
    exec(&db, "INSERT INTO t VALUES (1, 'dup', 1)").await;
    exec(&db, "INSERT INTO t VALUES (2, 'dup', 2)").await;
    exec(&db, "INSERT INTO t VALUES (3, '', 3)").await;
    exec(&db, "INSERT INTO t VALUES (4, '你好', 4)").await;
    exec(&db, "INSERT INTO t VALUES (5, 'ab\0', 5)").await;
    exec(&db, "INSERT INTO t VALUES (6, 'pre-a', 6)").await;
    exec(&db, "INSERT INTO t VALUES (7, 'pre-b', 7)").await;
    exec(&db, "INSERT INTO t VALUES (8, NULL, 8)").await;

    db.execute_sql("CREATE INDEX idx_sql ON t(name)")
        .await
        .expect("SQL CREATE INDEX on a string column is accepted");
    db.execute_sql("DROP INDEX idx_sql")
        .await
        .expect("drop the SQL index before the executor builds the observed index");

    let (column_index, column_type) = string_column(&db, "t", "name").await;
    create_string_index(&db, "t", "idx_name", column_index, "name").await;

    let rows = sorted_by_id(select_all(&db, "SELECT * FROM t").await);
    assert_eq!(
        rows,
        vec![
            vec![Value::Int(1), Value::String("dup".into()), Value::Int(1)],
            vec![Value::Int(2), Value::String("dup".into()), Value::Int(2)],
            vec![Value::Int(3), Value::String(String::new()), Value::Int(3)],
            vec![Value::Int(4), Value::String("你好".into()), Value::Int(4)],
            vec![Value::Int(5), Value::String("ab\0".into()), Value::Int(5)],
            vec![Value::Int(6), Value::String("pre-a".into()), Value::Int(6)],
            vec![Value::Int(7), Value::String("pre-b".into()), Value::Int(7)],
            vec![Value::Int(8), Value::Null, Value::Int(8)],
        ]
    );

    let index = index_of(&db, "t", "idx_name").await;
    let indexed = ["dup", "", "你好", "ab\0", "pre-a", "pre-b"];
    let mut expected = Vec::new();
    for text in indexed {
        let key = key_bytes(text, &column_type);
        let found = rids_for(&index, &key).await;
        let expected_count = if text == "dup" { 2 } else { 1 };
        assert_eq!(found.len(), expected_count, "{text:?} must be backfilled");
        for _ in &found {
            expected.push(stored_key(&key));
        }
    }
    expected.sort();
    assert_eq!(scanned_keys(&index).await, expected);
}

#[tokio::test]
async fn string_index_over_capacity_row_stays_and_is_absent_from_tree() {
    let dir = tempdir().unwrap();
    let db = Arc::new(
        Database::open(&db_path(dir.path(), "capacity"))
            .await
            .unwrap(),
    );
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;
    let inside = "a".repeat(30);
    let outside = "b".repeat(31);
    exec(&db, &format!("INSERT INTO t VALUES (1, '{inside}')")).await;
    exec(&db, &format!("INSERT INTO t VALUES (2, '{outside}')")).await;

    let (column_index, column_type) = string_column(&db, "t", "name").await;
    assert!(Value::String(outside.clone())
        .secondary_index_key(&column_type)
        .is_none());
    create_string_index(&db, "t", "idx_name", column_index, "name").await;

    let rows = sorted_by_id(select_all(&db, "SELECT * FROM t").await);
    assert_eq!(rows[1][1], Value::String(outside.clone()));
    let index = index_of(&db, "t", "idx_name").await;
    let inside_key = key_bytes(&inside, &column_type);
    assert_eq!(rids_for(&index, &inside_key).await.len(), 1);
    assert_eq!(scanned_keys(&index).await, vec![stored_key(&inside_key)]);

    exec(
        &db,
        &format!("UPDATE t SET name = '{outside}' WHERE id = 1"),
    )
    .await;
    let rows = select_all(&db, "SELECT * FROM t").await;
    assert!(rows
        .iter()
        .any(|row| row[1] == Value::String(outside.clone())));
    assert!(scanned_keys(index_of(&db, "t", "idx_name").await.as_ref())
        .await
        .is_empty());

    exec(&db, &format!("UPDATE t SET name = '{inside}' WHERE id = 1")).await;
    let index = index_of(&db, "t", "idx_name").await;
    assert_eq!(rids_for(&index, &inside_key).await.len(), 1);
    assert_eq!(scanned_keys(&index).await, vec![stored_key(&inside_key)]);
    let rows = sorted_by_id(select_all(&db, "SELECT * FROM t").await);
    assert_eq!(rows[0][1], Value::String(inside));
    assert_eq!(rows[1][1], Value::String(outside));
}

#[tokio::test]
async fn string_index_rekey_null_rollback_and_failed_write() {
    let dir = tempdir().unwrap();
    let db = Arc::new(
        Database::open(&db_path(dir.path(), "lifecycle"))
            .await
            .unwrap(),
    );
    exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;
    exec(&db, "INSERT INTO t VALUES (1, 'alpha')").await;
    exec(&db, "INSERT INTO t VALUES (2, 'dup')").await;
    exec(&db, "INSERT INTO t VALUES (3, 'dup')").await;
    let (column_index, column_type) = string_column(&db, "t", "name").await;
    create_string_index(&db, "t", "idx_name", column_index, "name").await;

    exec(&db, "UPDATE t SET name = 'beta' WHERE id = 1").await;
    let index = index_of(&db, "t", "idx_name").await;
    assert!(rids_for(&index, &key_bytes("alpha", &column_type))
        .await
        .is_empty());
    assert_eq!(
        rids_for(&index, &key_bytes("beta", &column_type))
            .await
            .len(),
        1
    );

    exec(&db, "UPDATE t SET name = NULL WHERE id = 1").await;
    let index = index_of(&db, "t", "idx_name").await;
    assert!(rids_for(&index, &key_bytes("beta", &column_type))
        .await
        .is_empty());
    assert_eq!(scanned_keys(&index).await.len(), 2);
    assert_eq!(
        select_all(&db, "SELECT * FROM t WHERE id = 1").await,
        vec![vec![Value::Int(1), Value::Null]]
    );

    exec(&db, "UPDATE t SET name = 'alpha' WHERE id = 1").await;
    exec(&db, "DELETE FROM t WHERE id = 2").await;
    let index = index_of(&db, "t", "idx_name").await;
    let dup_rids = rids_for(&index, &key_bytes("dup", &column_type)).await;
    assert_eq!(dup_rids.len(), 1);
    assert_eq!(row_at(&db, "t", dup_rids[0]).await[0], Value::Int(3));

    let before_rows = sorted_by_id(select_all(&db, "SELECT * FROM t").await);
    let before_alpha = rids_for(&index, &key_bytes("alpha", &column_type)).await;
    let before_dup = dup_rids.clone();

    let tx = db.begin().await.unwrap();
    db.execute_in_tx("INSERT INTO t VALUES (4, 'tx')", &tx)
        .await
        .unwrap();
    db.execute_in_tx("UPDATE t SET name = 'changed' WHERE id = 1", &tx)
        .await
        .unwrap();
    db.execute_in_tx("DELETE FROM t WHERE id = 3", &tx)
        .await
        .unwrap();
    db.rollback(tx).await.unwrap();

    let index = index_of(&db, "t", "idx_name").await;
    assert_eq!(
        sorted_by_id(select_all(&db, "SELECT * FROM t").await),
        before_rows
    );
    assert_eq!(
        rids_for(&index, &key_bytes("alpha", &column_type)).await,
        before_alpha
    );
    assert_eq!(
        rids_for(&index, &key_bytes("dup", &column_type)).await,
        before_dup
    );
    assert!(rids_for(&index, &key_bytes("changed", &column_type))
        .await
        .is_empty());
    assert!(rids_for(&index, &key_bytes("tx", &column_type))
        .await
        .is_empty());

    let rejected = db
        .execute_sql("INSERT INTO t VALUES (1, 'nope')")
        .await
        .expect_err("duplicate primary key must be rejected");
    assert!(
        rejected.message.contains("Duplicate key"),
        "duplicate insert must be rejected by name, got {}",
        rejected.message
    );
    assert_eq!(
        sorted_by_id(select_all(&db, "SELECT * FROM t").await),
        before_rows
    );
    assert_eq!(
        rids_for(
            index_of(&db, "t", "idx_name").await.as_ref(),
            &key_bytes("alpha", &column_type)
        )
        .await,
        before_alpha
    );

    exec(
        &db,
        "INSERT INTO t VALUES (1, 'zed') ON CONFLICT (id) DO UPDATE SET name = 'zed'",
    )
    .await;
    let index = index_of(&db, "t", "idx_name").await;
    assert!(rids_for(&index, &key_bytes("alpha", &column_type))
        .await
        .is_empty());
    assert_eq!(
        rids_for(&index, &key_bytes("zed", &column_type))
            .await
            .len(),
        1
    );
    assert_eq!(
        select_all(&db, "SELECT * FROM t WHERE id = 1").await,
        vec![vec![Value::Int(1), Value::String("zed".into())]]
    );

    exec(&db, "REPLACE INTO t VALUES (1, 'gamma')").await;
    let index = index_of(&db, "t", "idx_name").await;
    assert!(rids_for(&index, &key_bytes("zed", &column_type))
        .await
        .is_empty());
    let gamma = rids_for(&index, &key_bytes("gamma", &column_type)).await;
    assert_eq!(gamma.len(), 1);
    assert_eq!(
        row_at(&db, "t", gamma[0]).await,
        vec![Value::Int(1), Value::String("gamma".into())]
    );
    assert_eq!(
        select_all(&db, "SELECT * FROM t WHERE id = 1").await,
        vec![vec![Value::Int(1), Value::String("gamma".into())]]
    );
}

#[tokio::test]
async fn string_index_survives_drop_of_unindexed_column() {
    let dir = tempdir().unwrap();
    let db = Arc::new(
        Database::open(&db_path(dir.path(), "drop-column"))
            .await
            .unwrap(),
    );
    exec(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR, extra INT, tail INT)",
    )
    .await;
    exec(&db, "INSERT INTO t VALUES (1, 'kept', 10, 20)").await;
    exec(&db, "INSERT INTO t VALUES (2, 'also', 11, 21)").await;
    let (column_index, column_type) = string_column(&db, "t", "name").await;
    create_string_index(&db, "t", "idx_name", column_index, "name").await;

    exec(&db, "ALTER TABLE t DROP COLUMN extra").await;
    assert_eq!(
        sorted_by_id(select_all(&db, "SELECT * FROM t").await),
        vec![
            vec![Value::Int(1), Value::String("kept".into()), Value::Int(20)],
            vec![Value::Int(2), Value::String("also".into()), Value::Int(21)],
        ]
    );
    let index = index_of(&db, "t", "idx_name").await;
    for (text, id) in [("kept", 1), ("also", 2)] {
        let found = rids_for(&index, &key_bytes(text, &column_type)).await;
        assert_eq!(found.len(), 1, "{text} must still be indexed");
        assert_eq!(
            row_at(&db, "t", found[0]).await,
            vec![
                Value::Int(id),
                Value::String(text.into()),
                Value::Int(if id == 1 { 20 } else { 21 })
            ]
        );
    }
}

#[tokio::test]
async fn string_index_clean_reopen_and_pending_wal_agree() {
    let dir = tempdir().unwrap();
    let path = db_path(dir.path(), "reopen");
    {
        let db = Arc::new(Database::open(&path).await.unwrap());
        exec(&db, "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;
        exec(&db, "INSERT INTO t VALUES (1, 'alpha')").await;
        exec(&db, "INSERT INTO t VALUES (2, '你好')").await;
        let (column_index, column_type) = string_column(&db, "t", "name").await;
        create_string_index(&db, "t", "idx_name", column_index, "name").await;
        db.buffer_pool.flush_all().await.unwrap();
        let index = index_of(&db, "t", "idx_name").await;
        assert_eq!(
            rids_for(&index, &key_bytes("alpha", &column_type))
                .await
                .len(),
            1
        );
        assert_eq!(
            rids_for(&index, &key_bytes("你好", &column_type))
                .await
                .len(),
            1
        );
        db.close().await.unwrap();
    }
    {
        let db = Arc::new(Database::open(&path).await.unwrap());
        let (_, column_type) = string_column(&db, "t", "name").await;
        let index = index_of(&db, "t", "idx_name").await;
        assert_eq!(
            rids_for(&index, &key_bytes("alpha", &column_type))
                .await
                .len(),
            1
        );
        assert_eq!(
            rids_for(&index, &key_bytes("你好", &column_type))
                .await
                .len(),
            1
        );
        assert_eq!(select_all(&db, "SELECT * FROM t").await.len(), 2);
        let report = check_integrity(&db.buffer_pool, &db.table_manager)
            .await
            .unwrap();
        assert_eq!(report.error_count(), 0);

        exec(&db, "INSERT INTO t VALUES (3, 'late')").await;
        assert_eq!(
            rids_for(
                index_of(&db, "t", "idx_name").await.as_ref(),
                &key_bytes("late", &column_type)
            )
            .await
            .len(),
            1
        );
        db.wal_buffer.as_ref().unwrap().shutdown().await;
    }
    let db = Database::open(&path).await.unwrap();
    let (_, column_type) = string_column(&db, "t", "name").await;
    let index = index_of(&db, "t", "idx_name").await;
    assert_eq!(
        rids_for(&index, &key_bytes("alpha", &column_type))
            .await
            .len(),
        1
    );
    assert_eq!(
        rids_for(&index, &key_bytes("你好", &column_type))
            .await
            .len(),
        1
    );
    assert_eq!(
        rids_for(&index, &key_bytes("late", &column_type))
            .await
            .len(),
        1
    );
    assert_eq!(
        sorted_by_id(select_all(&db, "SELECT * FROM t").await),
        vec![
            vec![Value::Int(1), Value::String("alpha".into())],
            vec![Value::Int(2), Value::String("你好".into())],
            vec![Value::Int(3), Value::String("late".into())],
        ]
    );
    let report = check_integrity(&db.buffer_pool, &db.table_manager)
        .await
        .unwrap();
    assert_eq!(report.error_count(), 0);
}

#[tokio::test]
async fn legacy_db_accepts_string_index_without_losing_int_data() {
    let dir = tempdir().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("legacy")
        .join("legacy_g1.db");
    let path = db_path(dir.path(), "legacy_g1.db");
    std::fs::copy(&source, &path).expect("copy legacy fixture");

    let db = Arc::new(Database::open(&path).await.unwrap());
    let original = sorted_by_id(select_all(&db, "SELECT * FROM users").await);
    assert_eq!(
        original,
        vec![
            vec![
                Value::Int(1),
                Value::String("alice".into()),
                Value::Float(9.5)
            ],
            vec![
                Value::Int(2),
                Value::String("bob".into()),
                Value::Float(7.25)
            ],
            vec![
                Value::Int(3),
                Value::String("carol".into()),
                Value::Float(8.0)
            ],
        ]
    );

    db.execute_sql("CREATE INDEX idx_sql ON users(name)")
        .await
        .expect("SQL CREATE INDEX on users.name is accepted");
    db.execute_sql("DROP INDEX idx_sql")
        .await
        .expect("drop the SQL index before the executor builds the observed index");

    let (column_index, column_type) = string_column(&db, "users", "name").await;
    create_string_index(&db, "users", "idx_name", column_index, "name").await;
    let index = index_of(&db, "users", "idx_name").await;
    for name in ["alice", "bob", "carol"] {
        let found = rids_for(&index, &key_bytes(name, &column_type)).await;
        assert_eq!(found.len(), 1, "{name} must enter the existing tree");
        assert_eq!(
            row_at(&db, "users", found[0]).await[1],
            Value::String(name.into())
        );
    }
    assert_eq!(
        select_all(&db, "SELECT * FROM users WHERE id = 1").await,
        vec![original[0].clone()]
    );
    assert_eq!(
        sorted_by_id(select_all(&db, "SELECT * FROM users").await),
        original
    );
}
