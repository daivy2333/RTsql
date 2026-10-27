//! 组合索引和新建整数索引的条目与存活行。
//!
//! 语句从 `Database::execute_sql` 进入。条目用索引的 `scan_all` 观察，行用 `SELECT` 观察。

use rtsql::database::Database;
use rtsql::storage::page_format::deserialize_row;
use rtsql::storage::{read_tuple_from_data_page, ColumnType, IndexManager, Key, RowId};
use rtsql::{SqlError, SqlOutcome, Value};
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

async fn select_rows(db: &Database, sql: &str) -> Vec<Vec<Value>> {
    match db.execute_sql(sql).await.expect(sql) {
        SqlOutcome::Rows { rows, .. } => rows,
        other => panic!("{sql}: expected rows, got {other:?}"),
    }
}

fn stored_key(bytes: &[u8]) -> [u8; 32] {
    *Key::new(bytes).full_data()
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

fn column_at(columns: &[(String, ColumnType)], name: &str) -> (usize, ColumnType) {
    columns
        .iter()
        .enumerate()
        .find(|(_, (column_name, _))| column_name == name)
        .map(|(index, (_, column_type))| (index, column_type.clone()))
        .unwrap_or_else(|| panic!("column {name} is missing"))
}

fn derived_key(
    columns: &[(String, ColumnType)],
    values: &[Value],
    ordinals: &[usize],
    generation: u8,
) -> Option<Key> {
    let types: Vec<ColumnType> = ordinals
        .iter()
        .map(|index| columns[*index].1.clone())
        .collect();
    let picked: Vec<Value> = ordinals
        .iter()
        .map(|index| values[*index].clone())
        .collect();
    Value::derive_secondary_index_key(&types, &picked, generation)
}

fn stored_derived(
    columns: &[(String, ColumnType)],
    values: &[Value],
    ordinals: &[usize],
    generation: u8,
) -> Option<[u8; 32]> {
    derived_key(columns, values, ordinals, generation).map(|key| *key.full_data())
}

async fn explain(db: &Database, sql: &str) -> String {
    let SqlOutcome::Rows { rows, .. } = db
        .execute_sql(&format!("EXPLAIN {sql}"))
        .await
        .unwrap_or_else(|error| panic!("EXPLAIN {sql} failed: {error}"))
    else {
        panic!("EXPLAIN {sql}: expected rows");
    };
    rows.into_iter()
        .map(|row| match &row[0] {
            Value::String(text) => text.clone(),
            other => panic!("plan cell must be a string, got {other:?}"),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

async fn must_err(db: &Database, sql: &str) -> SqlError {
    db.execute_sql(sql).await.expect_err(sql)
}

struct ObservedIndex {
    generation: u8,
    column_index: usize,
    extra_column_indexes: Vec<usize>,
    index: Arc<IndexManager>,
}

async fn observed_index(db: &Database, table: &str, name: &str) -> ObservedIndex {
    let meta = db.get_table(table).await.expect(table);
    let index = meta
        .secondary_indexes
        .iter()
        .find(|index| index.name == name)
        .unwrap_or_else(|| panic!("{table} has no index {name}"));
    ObservedIndex {
        generation: index.generation,
        column_index: index.column_index,
        extra_column_indexes: index.extra_column_indexes.clone(),
        index: index.index.clone(),
    }
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

async fn ids_for(db: &Database, table: &str, index: &IndexManager, key: &Key) -> Vec<i64> {
    let mut ids = Vec::new();
    for rid in index.search_all(key.as_bytes()).await.expect("search_all") {
        let row = row_at(db, table, rid).await;
        let Value::Int(id) = row[0] else {
            panic!("id must be an integer, got {:?}", row[0]);
        };
        ids.push(id);
    }
    ids.sort();
    ids
}

#[tokio::test]
async fn composite_backfill_matches_live_rows() {
    let dir = tempdir().unwrap();
    let db = Arc::new(
        Database::open(&db_path(dir.path(), "backfill"))
            .await
            .unwrap(),
    );
    exec(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, age INT, name VARCHAR)",
    )
    .await;

    let overlong = "a".repeat(23);
    let rows = vec![
        vec![Value::Int(1), Value::Int(10), Value::String("dup".into())],
        vec![Value::Int(2), Value::Int(10), Value::String("dup".into())],
        vec![Value::Int(3), Value::Int(11), Value::Null],
        vec![
            Value::Int(4),
            Value::Int(12),
            Value::String(overlong.clone()),
        ],
        vec![Value::Int(5), Value::Int(13), Value::String("ok".into())],
        vec![Value::Int(6), Value::Null, Value::String("gap".into())],
    ];
    for row in &rows {
        let name = match &row[2] {
            Value::String(text) => format!("'{text}'"),
            Value::Null => "NULL".to_string(),
            other => panic!("name must be a string or NULL, got {other:?}"),
        };
        let age = match &row[1] {
            Value::Int(value) => value.to_string(),
            Value::Null => "NULL".to_string(),
            other => panic!("age must be an integer or NULL, got {other:?}"),
        };
        let id = match &row[0] {
            Value::Int(value) => value,
            other => panic!("id must be an integer, got {other:?}"),
        };
        exec(&db, &format!("INSERT INTO t VALUES ({id}, {age}, {name})")).await;
    }

    let before = select_rows(&db, "SELECT id, age, name FROM t ORDER BY id").await;
    exec(&db, "CREATE INDEX idx_age_name ON t(age, name)").await;
    let after = select_rows(&db, "SELECT id, age, name FROM t ORDER BY id").await;
    assert_eq!(after, before);
    assert_eq!(after, rows);

    let meta = db.get_table("t").await.expect("t");
    let (age_index, _) = column_at(&meta.columns, "age");
    let (name_index, _) = column_at(&meta.columns, "name");
    let index = meta
        .secondary_indexes
        .iter()
        .find(|index| index.name == "idx_age_name")
        .expect("idx_age_name");
    assert_eq!(index.generation, 1);
    assert_eq!(index.column_index, age_index);
    assert_eq!(index.extra_column_indexes, vec![name_index]);

    let ordinals = [age_index, name_index];
    let mut expected = Vec::new();
    for row in &rows {
        match stored_derived(&meta.columns, row, &ordinals, 1) {
            Some(key) => expected.push(key),
            None => {
                let name = &row[2];
                assert!(
                    matches!(row[1], Value::Null)
                        || matches!(name, Value::Null)
                        || matches!(name, Value::String(text) if text == &overlong),
                    "only NULL and the overlong row stay out of the tree, got {row:?}"
                );
            }
        }
    }
    expected.sort();
    assert_eq!(
        expected.len(),
        3,
        "duplicate pair and the short row enter the tree"
    );
    assert_eq!(scanned_keys(&index.index).await, expected);

    let age_plan = explain(&db, "SELECT id, age, name FROM t WHERE age = 10").await;
    assert!(
        age_plan.contains("DataScan") && !age_plan.contains("IndexScanAll"),
        "a predicate on one column of a composite index keeps the scan: {age_plan}"
    );
    assert_eq!(
        select_rows(
            &db,
            "SELECT id, age, name FROM t WHERE age = 10 ORDER BY id"
        )
        .await,
        rows[..2]
    );
}

#[tokio::test]
async fn new_single_column_int_index_uses_ordered_key() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(dir.path(), "ordered"))
        .await
        .unwrap();
    exec(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, age INT, name VARCHAR)",
    )
    .await;
    exec(&db, "INSERT INTO t VALUES (1, -7, 'a')").await;
    exec(&db, "INSERT INTO t VALUES (2, 3, 'b')").await;
    let negative_before = select_rows(
        &db,
        "SELECT id, age, name FROM t WHERE age = -7 ORDER BY id",
    )
    .await;
    let positive_before =
        select_rows(&db, "SELECT id, age, name FROM t WHERE age = 3 ORDER BY id").await;

    exec(&db, "CREATE INDEX idx_age ON t(age)").await;
    assert_eq!(
        select_rows(
            &db,
            "SELECT id, age, name FROM t WHERE age = -7 ORDER BY id"
        )
        .await,
        negative_before
    );
    assert_eq!(
        select_rows(&db, "SELECT id, age, name FROM t WHERE age = 3 ORDER BY id").await,
        positive_before
    );
    let plan = explain(&db, "SELECT id, age, name FROM t WHERE age = 3").await;
    assert!(
        plan.contains("IndexScanAll") && plan.contains("index=idx_age"),
        "plan must name idx_age on IndexScanAll, got: {plan}"
    );

    let meta = db.get_table("t").await.expect("t");
    let (age_index, _) = column_at(&meta.columns, "age");
    let age = observed_index(&db, "t", "idx_age").await;
    assert_eq!(age.generation, 1);
    assert_eq!(age.column_index, age_index);
    assert!(age.extra_column_indexes.is_empty());
    let ordered = derived_key(
        &meta.columns,
        &[Value::Int(1), Value::Int(-7), Value::String("a".into())],
        &[age_index],
        1,
    )
    .expect("negative int encodes");
    let legacy = Value::Int(-7).to_key().expect("legacy int key");
    assert_ne!(*ordered.full_data(), *legacy.full_data());
    let scanned = scanned_keys(&age.index).await;
    assert!(scanned.contains(ordered.full_data()));
    assert!(!scanned.contains(legacy.full_data()));

    let name_before = select_rows(&db, "SELECT id, name FROM t WHERE name = 'a' ORDER BY id").await;
    exec(&db, "CREATE INDEX idx_name ON t(name)").await;
    assert_eq!(
        select_rows(&db, "SELECT id, name FROM t WHERE name = 'a' ORDER BY id").await,
        name_before
    );
    let name = observed_index(&db, "t", "idx_name").await;
    let meta = db.get_table("t").await.expect("t");
    let (name_index, name_type) = column_at(&meta.columns, "name");
    assert_eq!(name.generation, 0);
    assert_eq!(name.column_index, name_index);
    assert!(name.extra_column_indexes.is_empty());
    let legacy_string = Value::String("a".into())
        .secondary_index_key(&name_type)
        .expect("string key");
    let derived = derived_key(
        &meta.columns,
        &[Value::Int(1), Value::Int(-7), Value::String("a".into())],
        &[name_index],
        0,
    )
    .expect("generation 0 string key");
    assert_eq!(legacy_string, derived);
    assert!(scanned_keys(&name.index)
        .await
        .contains(derived.full_data()));
    let name_plan = explain(&db, "SELECT id, name FROM t WHERE name = 'a'").await;
    assert!(
        name_plan.contains("IndexScanAll") && name_plan.contains("index=idx_name"),
        "plan must name idx_name on IndexScanAll, got: {name_plan}"
    );
}

#[tokio::test]
async fn composite_update_keeps_the_other_duplicate() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(dir.path(), "update"))
        .await
        .unwrap();
    exec(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, age INT, name VARCHAR)",
    )
    .await;
    exec(&db, "INSERT INTO t VALUES (1, 10, 'dup')").await;
    exec(&db, "INSERT INTO t VALUES (2, 10, 'dup')").await;
    exec(&db, "INSERT INTO t VALUES (3, 10, NULL)").await;
    exec(&db, "CREATE INDEX idx_age_name ON t(age, name)").await;

    exec(&db, "UPDATE t SET age = 11 WHERE id = 1").await;
    let meta = db.get_table("t").await.expect("t");
    let (age_index, _) = column_at(&meta.columns, "age");
    let (name_index, _) = column_at(&meta.columns, "name");
    let ordinals = [age_index, name_index];
    let old_key = derived_key(
        &meta.columns,
        &[Value::Int(0), Value::Int(10), Value::String("dup".into())],
        &ordinals,
        1,
    )
    .expect("old key");
    let new_key = derived_key(
        &meta.columns,
        &[Value::Int(0), Value::Int(11), Value::String("dup".into())],
        &ordinals,
        1,
    )
    .expect("new key");
    let index = observed_index(&db, "t", "idx_age_name").await;
    assert_eq!(ids_for(&db, "t", &index.index, &old_key).await, vec![2]);
    assert_eq!(ids_for(&db, "t", &index.index, &new_key).await, vec![1]);

    exec(&db, "UPDATE t SET name = NULL WHERE id = 2").await;
    assert!(ids_for(&db, "t", &index.index, &old_key).await.is_empty());
    assert_eq!(
        select_rows(&db, "SELECT id, age, name FROM t WHERE id = 2").await,
        vec![vec![Value::Int(2), Value::Int(10), Value::Null]]
    );

    exec(&db, "UPDATE t SET name = 'up' WHERE id = 3").await;
    let appeared = derived_key(
        &meta.columns,
        &[Value::Int(0), Value::Int(10), Value::String("up".into())],
        &ordinals,
        1,
    )
    .expect("key appears when NULL becomes a short string");
    assert_eq!(ids_for(&db, "t", &index.index, &appeared).await, vec![3]);
    assert_eq!(
        select_rows(&db, "SELECT id, age, name FROM t ORDER BY id").await,
        vec![
            vec![Value::Int(1), Value::Int(11), Value::String("dup".into())],
            vec![Value::Int(2), Value::Int(10), Value::Null],
            vec![Value::Int(3), Value::Int(10), Value::String("up".into())],
        ]
    );
}

#[tokio::test]
async fn composite_rollback_removes_uncommitted_entry() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(dir.path(), "rollback"))
        .await
        .unwrap();
    exec(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, age INT, name VARCHAR)",
    )
    .await;
    exec(&db, "INSERT INTO t VALUES (1, 10, 'dup')").await;
    exec(&db, "CREATE INDEX idx_age_name ON t(age, name)").await;
    let index = observed_index(&db, "t", "idx_age_name").await;
    let meta = db.get_table("t").await.expect("t");
    let (age_index, _) = column_at(&meta.columns, "age");
    let (name_index, _) = column_at(&meta.columns, "name");
    let key = derived_key(
        &meta.columns,
        &[Value::Int(0), Value::Int(10), Value::String("dup".into())],
        &[age_index, name_index],
        1,
    )
    .expect("shared key");
    assert_eq!(ids_for(&db, "t", &index.index, &key).await, vec![1]);

    let tx = db.begin().await.unwrap();
    db.execute_in_tx("INSERT INTO t VALUES (2, 10, 'dup')", &tx)
        .await
        .expect("uncommitted insert");
    assert_eq!(ids_for(&db, "t", &index.index, &key).await, vec![1, 2]);
    db.rollback(tx).await.unwrap();

    assert_eq!(ids_for(&db, "t", &index.index, &key).await, vec![1]);
    assert_eq!(
        select_rows(&db, "SELECT id, age, name FROM t ORDER BY id").await,
        vec![vec![
            Value::Int(1),
            Value::Int(10),
            Value::String("dup".into())
        ]]
    );
}

#[tokio::test]
async fn composite_recovery_matches_live_rows() {
    let dir = tempdir().unwrap();
    let path = db_path(dir.path(), "recover");
    let overlong = "b".repeat(23);
    {
        let db = Database::open(&path).await.unwrap();
        exec(
            &db,
            "CREATE TABLE t (id INT PRIMARY KEY, age INT, name VARCHAR)",
        )
        .await;
        db.buffer_pool.flush_all().await.unwrap();
        exec(&db, "INSERT INTO t VALUES (1, 10, 'dup')").await;
        exec(&db, "INSERT INTO t VALUES (2, 10, 'dup')").await;
        exec(&db, "INSERT INTO t VALUES (3, 11, NULL)").await;
        exec(&db, &format!("INSERT INTO t VALUES (4, 12, '{overlong}')")).await;
        exec(&db, "CREATE INDEX idx_age_name ON t(age, name)").await;
        exec(&db, "INSERT INTO t VALUES (5, 13, 'ok')").await;
        exec(&db, "UPDATE t SET age = 14 WHERE id = 1").await;
        db.wal_buffer.as_ref().unwrap().shutdown().await;
    }

    let db = Database::open(&path).await.unwrap();
    let rows = select_rows(&db, "SELECT id, age, name FROM t ORDER BY id").await;
    assert_eq!(
        rows,
        vec![
            vec![Value::Int(1), Value::Int(14), Value::String("dup".into())],
            vec![Value::Int(2), Value::Int(10), Value::String("dup".into())],
            vec![Value::Int(3), Value::Int(11), Value::Null],
            vec![Value::Int(4), Value::Int(12), Value::String(overlong)],
            vec![Value::Int(5), Value::Int(13), Value::String("ok".into())],
        ]
    );
    let meta = db.get_table("t").await.expect("t");
    let (age_index, _) = column_at(&meta.columns, "age");
    let (name_index, _) = column_at(&meta.columns, "name");
    let index = observed_index(&db, "t", "idx_age_name").await;
    assert_eq!(index.generation, 1);
    assert_eq!(index.column_index, age_index);
    assert_eq!(index.extra_column_indexes, vec![name_index]);
    let ordinals = [age_index, name_index];
    let mut expected = Vec::new();
    for row in &rows {
        if let Some(key) = stored_derived(&meta.columns, row, &ordinals, 1) {
            expected.push(key);
        }
    }
    expected.sort();
    assert_eq!(expected.len(), 3);
    assert_eq!(scanned_keys(&index.index).await, expected);
}

#[tokio::test]
async fn drop_column_sees_every_index_column() {
    let dir = tempdir().unwrap();
    let path = db_path(dir.path(), "drop");
    let db = Database::open(&path).await.unwrap();
    exec(
        &db,
        "CREATE TABLE t (id INT PRIMARY KEY, pad INT, age INT, name VARCHAR)",
    )
    .await;
    exec(&db, "INSERT INTO t VALUES (1, 7, 10, 'dup')").await;
    exec(&db, "INSERT INTO t VALUES (2, 8, 10, 'dup')").await;
    exec(&db, "CREATE INDEX idx_age_name ON t(age, name)").await;

    for column in ["age", "name"] {
        let error = must_err(&db, &format!("ALTER TABLE t DROP COLUMN {column}")).await;
        assert!(
            error.message.contains("referenced by a secondary index"),
            "{}",
            error.message
        );
    }
    let meta = db.get_table("t").await.expect("t");
    let names: Vec<&str> = meta.columns.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["id", "pad", "age", "name"]);
    let before = observed_index(&db, "t", "idx_age_name").await;
    let (age_before, _) = column_at(&meta.columns, "age");
    let (name_before, _) = column_at(&meta.columns, "name");
    assert_eq!(before.generation, 1);
    assert_eq!(before.column_index, age_before);
    assert_eq!(before.extra_column_indexes, vec![name_before]);

    exec(&db, "ALTER TABLE t DROP COLUMN pad").await;
    drop(meta);
    drop(before);
    db.close().await.unwrap();
    drop(db);

    let db = Database::open(&path).await.unwrap();
    assert_eq!(
        select_rows(&db, "SELECT id, age, name FROM t ORDER BY id").await,
        vec![
            vec![Value::Int(1), Value::Int(10), Value::String("dup".into())],
            vec![Value::Int(2), Value::Int(10), Value::String("dup".into())],
        ]
    );
    let meta = db.get_table("t").await.expect("t");
    let (age_after, _) = column_at(&meta.columns, "age");
    let (name_after, _) = column_at(&meta.columns, "name");
    assert_eq!(age_after, age_before - 1);
    assert_eq!(name_after, name_before - 1);
    let after = observed_index(&db, "t", "idx_age_name").await;
    assert_eq!(after.generation, 1);
    assert_eq!(after.column_index, age_after);
    assert_eq!(after.extra_column_indexes, vec![name_after]);
    let key = derived_key(
        &meta.columns,
        &[Value::Int(1), Value::Int(10), Value::String("dup".into())],
        &[age_after, name_after],
        1,
    )
    .expect("key after narrowing");
    assert_eq!(ids_for(&db, "t", &after.index, &key).await, vec![1, 2]);
}

#[tokio::test]
async fn create_index_rejects_duplicate_column_and_five_ints() {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(dir.path(), "reject"))
        .await
        .unwrap();
    exec(
        &db,
        "CREATE TABLE t (
            id INT PRIMARY KEY,
            a INT, b INT, c INT, d INT, e INT,
            s1 VARCHAR, s2 VARCHAR, s3 VARCHAR, s4 VARCHAR
        )",
    )
    .await;
    exec(
        &db,
        "INSERT INTO t VALUES (1, 1, 1, 1, 1, 1, 'a', 'b', 'c', 'd')",
    )
    .await;
    let overlong = "c".repeat(30);
    exec(
        &db,
        &format!("INSERT INTO t VALUES (2, 1, 1, 1, 1, 1, '{overlong}', 'b', 'c', 'd')"),
    )
    .await;
    exec(
        &db,
        "INSERT INTO t VALUES (3, 1, 1, 1, 1, 1, 'a', NULL, 'c', 'd')",
    )
    .await;
    exec(&db, "CREATE INDEX idx_a ON t(a)").await;

    let duplicate = must_err(&db, "CREATE INDEX idx_dup ON t(a, a)").await;
    assert!(
        duplicate.message.contains("duplicate column name"),
        "{}",
        duplicate.message
    );
    let five = must_err(&db, "CREATE INDEX idx_wide ON t(a, b, c, d, e)").await;
    assert!(
        five.message
            .contains("MULTI-COLUMN INDEX EXCEEDS 32-BYTE KEY"),
        "{}",
        five.message
    );
    let meta = db.get_table("t").await.expect("t");
    let names: Vec<&str> = meta
        .secondary_indexes
        .iter()
        .map(|index| index.name.as_str())
        .collect();
    assert_eq!(names, ["idx_a"]);
    assert_eq!(
        select_rows(&db, "SELECT id FROM t ORDER BY id").await,
        vec![
            vec![Value::Int(1)],
            vec![Value::Int(2)],
            vec![Value::Int(3)]
        ]
    );

    exec(&db, "CREATE INDEX idx_mix ON t(a, s1, s2, s3, s4)").await;
    let meta = db.get_table("t").await.expect("t");
    let mix = observed_index(&db, "t", "idx_mix").await;
    assert_eq!(mix.generation, 1);
    assert_eq!(mix.extra_column_indexes.len(), 4);
    let mut ordinals = vec![mix.column_index];
    ordinals.extend(mix.extra_column_indexes.iter().copied());
    let rows = select_rows(
        &db,
        "SELECT id, a, b, c, d, e, s1, s2, s3, s4 FROM t ORDER BY id",
    )
    .await;
    let mut expected = Vec::new();
    for row in &rows {
        if let Some(key) = stored_derived(&meta.columns, row, &ordinals, 1) {
            expected.push(key);
        }
    }
    expected.sort();
    assert_eq!(expected.len(), 1, "only the short five-column row fits");
    assert_eq!(scanned_keys(&mix.index).await, expected);
}
