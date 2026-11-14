//! DECIMAL 与 BLOB：声明、精确写入、比较、重开、超页字节和旧库样本。

use std::path::Path;
use std::process::Command;

use rtsql::database::Database;
use rtsql::executor::Value;
use rtsql::result::{value_to_json, SqlOutcome};
use rtsql::storage::page_format::overflow::INLINE_SLOT_MAX;
use tempfile::tempdir;

fn rows(outcome: SqlOutcome) -> Vec<Vec<serde_json::Value>> {
    match outcome {
        SqlOutcome::Rows { rows, .. } => rows
            .into_iter()
            .map(|row| row.into_iter().map(value_to_json).collect())
            .collect(),
        other => panic!("expected rows, got {other:?}"),
    }
}

async fn open_db(name: &str) -> (Database, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let db = Database::open(&dir.path().join(name)).await.unwrap();
    (db, dir)
}

fn cli(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .env_remove("RTSQL_KEY")
        .output()
        .expect("spawn rtsql");
    (
        out.status.code().unwrap_or(1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn db_path(dir: &Path, name: &str) -> String {
    dir.join(name).to_str().unwrap().to_string()
}

#[tokio::test]
async fn schema_shows_decimal_and_blob() {
    let dir = tempdir().unwrap();
    let path = db_path(dir.path(), "types.db");
    {
        let db = Database::open(Path::new(&path)).await.unwrap();
        db.execute_sql("CREATE TABLE t (d DECIMAL(10,2), b BLOB)")
            .await
            .unwrap();
        db.close().await.unwrap();
    }
    let (code, stdout, stderr) = cli(&["schema", &path]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains("DECIMAL(10,2)"),
        "schema missing DECIMAL(10,2): {stdout}"
    );
    assert!(stdout.contains("BLOB"), "schema missing BLOB: {stdout}");
    assert!(
        !stdout.contains("BLOB("),
        "BLOB must not carry a length: {stdout}"
    );
}

#[tokio::test]
async fn decimal_declaration_rules() {
    let (db, _dir) = open_db("decl.db").await;

    let err = db
        .execute_sql("CREATE TABLE t (d DECIMAL)")
        .await
        .expect_err("DECIMAL without precision");
    assert!(
        err.message.contains("requires precision"),
        "{}",
        err.message
    );
    assert!(db.execute_sql("SELECT * FROM t").await.is_err());

    db.execute_sql("CREATE TABLE p (d DECIMAL(10))")
        .await
        .unwrap();
    db.close().await.unwrap();
    drop(db);
    let (code, stdout, stderr) = cli(&["schema", &db_path(_dir.path(), "decl.db")]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains("DECIMAL(10,0)"),
        "DECIMAL(p) scale is 0: {stdout}"
    );

    let (db, _dir) = open_db("decl2.db").await;
    let err = db
        .execute_sql("CREATE TABLE bad (d DECIMAL(2,3))")
        .await
        .expect_err("scale above precision");
    assert!(
        err.message.contains("scale must not exceed precision")
            || err.message.contains("requires precision"),
        "{}",
        err.message
    );
    let err = db
        .execute_sql("CREATE TABLE bad (d DECIMAL(39,0))")
        .await
        .expect_err("precision above 38");
    assert!(
        err.message.contains("scale must not exceed precision")
            || err.message.contains("requires precision"),
        "{}",
        err.message
    );
    let err = db
        .execute_sql("CREATE TABLE bad (b BLOB(10))")
        .await
        .expect_err("blob length");
    assert!(
        err.message.contains("does not take a length"),
        "{}",
        err.message
    );
    assert!(db.execute_sql("SELECT * FROM bad").await.is_err());
}

#[tokio::test]
async fn exact_write_compare_reopen_and_overflow() {
    let (db, dir) = open_db("values.db").await;
    let path = db_path(dir.path(), "values.db");
    db.execute_sql("CREATE TABLE t (d DECIMAL(10,2), b BLOB)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1.50, X'6869')")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (12, X'6869')")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES ('1.50', X'6869')")
        .await
        .unwrap();
    db.execute_with_params("INSERT INTO t VALUES (1.50, ?)", &[Value::Blob(Vec::new())])
        .await
        .unwrap();

    let got = rows(db.execute_sql("SELECT d, b FROM t").await.unwrap());
    assert_eq!(
        got,
        vec![
            vec![serde_json::json!("1.50"), serde_json::json!("6869")],
            vec![serde_json::json!("12.00"), serde_json::json!("6869")],
            vec![serde_json::json!("1.50"), serde_json::json!("6869")],
            vec![serde_json::json!("1.50"), serde_json::json!("")],
        ]
    );

    let err = db
        .execute_sql("INSERT INTO t VALUES (1.234, X'00')")
        .await
        .expect_err("inexact decimal");
    assert!(err.message.contains("expects"), "{}", err.message);
    let err = db
        .execute_with_params("INSERT INTO t VALUES (?, X'00')", &[Value::Float(1.5)])
        .await
        .expect_err("float does not become decimal");
    assert!(err.message.contains("expects"), "{}", err.message);
    assert_eq!(
        rows(db.execute_sql("SELECT COUNT(*) FROM t").await.unwrap()),
        vec![vec![serde_json::json!(4)]]
    );

    db.execute_sql("INSERT INTO t VALUES (0.50, X'01')")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (10.00, X'02')")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (2.00, X'03')")
        .await
        .unwrap();
    let ordered = rows(
        db.execute_sql("SELECT d FROM t WHERE d > 1.5 ORDER BY d")
            .await
            .unwrap(),
    );
    assert_eq!(
        ordered,
        vec![
            vec![serde_json::json!("2.00")],
            vec![serde_json::json!("10.00")],
            vec![serde_json::json!("12.00")],
        ]
    );
    let equal = rows(
        db.execute_sql("SELECT d FROM t WHERE d = 1.5")
            .await
            .unwrap(),
    );
    assert_eq!(equal.len(), 3);
    let err = db
        .execute_sql("SELECT d FROM t WHERE d = '1.50'")
        .await
        .expect_err("decimal compared with string");
    assert!(err.message.contains("Type mismatch"), "{}", err.message);
    let err = db
        .execute_sql("SELECT b FROM t WHERE b = '6869'")
        .await
        .expect_err("blob compared with string");
    assert!(err.message.contains("Type mismatch"), "{}", err.message);

    let err = db
        .execute_sql("SELECT d + 1 FROM t")
        .await
        .expect_err("no decimal arithmetic");
    assert!(err.message.contains("Type mismatch"), "{}", err.message);
    assert_eq!(
        rows(db.execute_sql("SELECT COUNT(*) FROM t").await.unwrap())[0][0],
        serde_json::json!(7)
    );

    db.execute_sql("CREATE TABLE f (x FLOAT)").await.unwrap();
    db.execute_sql("INSERT INTO f VALUES (1.5)").await.unwrap();
    assert_eq!(
        rows(db.execute_sql("SELECT x FROM f").await.unwrap()),
        vec![vec![serde_json::json!(1.5)]]
    );

    let mut blob_len = 5000;
    if blob_len <= INLINE_SLOT_MAX {
        blob_len = INLINE_SLOT_MAX + 1;
    }
    let payload = vec![0x5A; blob_len];
    db.execute_sql("CREATE TABLE wide (b BLOB)").await.unwrap();
    db.execute_with_params(
        "INSERT INTO wide VALUES (?)",
        &[Value::Blob(payload.clone())],
    )
    .await
    .unwrap();

    db.close().await.unwrap();
    drop(db);
    let db = Database::open(Path::new(&path)).await.unwrap();
    assert_eq!(
        rows(
            db.execute_sql("SELECT d FROM t WHERE d = 1.50")
                .await
                .unwrap()
        )
        .len(),
        3
    );
    let wide = rows(db.execute_sql("SELECT b FROM wide").await.unwrap());
    assert_eq!(wide, vec![vec![serde_json::json!(hex_of(&payload))]]);

    db.close().await.unwrap();
    drop(db);
    let (code, dump1, stderr) = cli(&["dump", &path]);
    assert_eq!(code, 0, "{stderr}");
    let restored = db_path(dir.path(), "restored.db");
    let (code, _, stderr) = cli(&["new", &restored]);
    assert_eq!(code, 0, "{stderr}");
    let dump_file = dir.path().join("dump.sql");
    std::fs::write(&dump_file, &dump1).unwrap();
    let (code, _, stderr) = cli(&["restore", &restored, dump_file.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    let (code, dump2, stderr) = cli(&["dump", &restored]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(dump1, dump2);
}

#[tokio::test]
async fn decimal_primary_key_is_reachable_without_index_key() {
    assert_eq!(Value::decimal(150, 2).to_key(), None);
    let (db, _dir) = open_db("pk.db").await;
    db.execute_sql("CREATE TABLE t (d DECIMAL(10,2) PRIMARY KEY)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (1.50)").await.unwrap();
    assert_eq!(
        rows(
            db.execute_sql("SELECT d FROM t WHERE d = 1.50")
                .await
                .unwrap()
        ),
        vec![vec![serde_json::json!("1.50")]]
    );
}

#[tokio::test]
async fn legacy_g1_opens_queries_and_writes() {
    let dir = tempdir().unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/legacy/legacy_g1.db");
    let db_path = dir.path().join("legacy_g1.db");
    std::fs::copy(&src, &db_path).unwrap();
    let before = std::fs::read(&src).unwrap();

    let db = Database::open(&db_path).await.expect("open g1");
    let got = rows(db.execute_sql("SELECT * FROM users").await.unwrap());
    assert!(got.iter().any(|row| {
        row.len() >= 3
            && row[0] == serde_json::json!(1)
            && row[1] == serde_json::json!("alice")
            && row[2] == serde_json::json!(9.5)
    }));
    db.execute_sql("INSERT INTO users VALUES (4, 'dave', 6.0)")
        .await
        .unwrap();
    let got = rows(db.execute_sql("SELECT * FROM users").await.unwrap());
    assert!(got.iter().any(|row| {
        row.len() >= 3
            && row[0] == serde_json::json!(4)
            && row[1] == serde_json::json!("dave")
            && row[2] == serde_json::json!(6.0)
    }));
    db.close().await.unwrap();
    assert_eq!(std::fs::read(&src).unwrap(), before);
}

#[tokio::test]
async fn numeric_without_precision_is_rejected() {
    let dir = tempdir().unwrap();
    let path = db_path(dir.path(), "numeric.db");
    let db = Database::open(Path::new(&path)).await.unwrap();

    let err = db
        .execute_sql("CREATE TABLE t (d NUMERIC)")
        .await
        .expect_err("NUMERIC without precision");
    assert!(
        err.message.contains("requires precision"),
        "{}",
        err.message
    );
    assert!(db.execute_sql("SELECT * FROM t").await.is_err());

    let err = db
        .execute_sql("CREATE TABLE bad (d NUMERIC(2,3))")
        .await
        .expect_err("scale above precision");
    assert!(
        err.message.contains("scale must not exceed precision")
            || err.message.contains("requires precision"),
        "{}",
        err.message
    );
    let err = db
        .execute_sql("CREATE TABLE bad (d NUMERIC(39,0))")
        .await
        .expect_err("precision above 38");
    assert!(
        err.message.contains("scale must not exceed precision")
            || err.message.contains("requires precision"),
        "{}",
        err.message
    );
    assert!(db.execute_sql("SELECT * FROM bad").await.is_err());

    db.execute_sql("CREATE TABLE ok (d NUMERIC(10,2), e NUMERIC(10))")
        .await
        .unwrap();
    db.close().await.unwrap();
    drop(db);

    let (code, stdout, stderr) = cli(&["schema", &path]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains("DECIMAL(10,2)"),
        "schema missing DECIMAL(10,2): {stdout}"
    );
    assert!(
        stdout.contains("DECIMAL(10,0)"),
        "schema missing DECIMAL(10,0): {stdout}"
    );
    assert!(
        !stdout.contains("NUMERIC"),
        "schema must not keep the NUMERIC alias: {stdout}"
    );

    let (code, dump, stderr) = cli(&["dump", &path]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        dump.contains("DECIMAL(10,2)") && dump.contains("DECIMAL(10,0)"),
        "dump missing DECIMAL declaration: {dump}"
    );
    assert!(
        !dump.contains("NUMERIC"),
        "dump must not keep the NUMERIC alias: {dump}"
    );
}

#[tokio::test]
async fn byte_aliases_declare_blob() {
    let dir = tempdir().unwrap();
    let path = db_path(dir.path(), "bytes.db");
    {
        let db = Database::open(Path::new(&path)).await.unwrap();
        db.execute_sql("CREATE TABLE t (a BYTEA, b BINARY, c VARBINARY)")
            .await
            .unwrap();
        db.close().await.unwrap();
    }

    let (code, stdout, stderr) = cli(&["schema", &path]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.contains("\"a\" BLOB")
            && stdout.contains("\"b\" BLOB")
            && stdout.contains("\"c\" BLOB"),
        "schema missing BLOB for byte aliases: {stdout}"
    );
    assert!(
        !stdout.contains("BYTEA") && !stdout.contains("BINARY"),
        "schema must not keep byte aliases: {stdout}"
    );

    let (code, dump, stderr) = cli(&["dump", &path]);
    assert_eq!(code, 0, "{stderr}");
    assert!(
        dump.contains("\"a\" BLOB") && dump.contains("\"b\" BLOB") && dump.contains("\"c\" BLOB"),
        "dump missing BLOB for byte aliases: {dump}"
    );
    assert!(
        !dump.contains("BYTEA") && !dump.contains("BINARY"),
        "dump must not keep byte aliases: {dump}"
    );

    let (db, _dir) = open_db("bytes-len.db").await;
    let err = db
        .execute_sql("CREATE TABLE bad (b BINARY(10))")
        .await
        .expect_err("BINARY length");
    assert!(
        err.message.contains("does not take a length"),
        "{}",
        err.message
    );
    let err = db
        .execute_sql("CREATE TABLE bad (c VARBINARY(10))")
        .await
        .expect_err("VARBINARY length");
    assert!(
        err.message.contains("does not take a length"),
        "{}",
        err.message
    );
    assert!(db.execute_sql("SELECT * FROM bad").await.is_err());
}

#[tokio::test]
async fn blob_primary_key_is_reachable() {
    let (db, _dir) = open_db("blob-pk.db").await;
    db.execute_sql("CREATE TABLE t (b BLOB PRIMARY KEY)")
        .await
        .unwrap();
    db.execute_sql("INSERT INTO t VALUES (X'6869')")
        .await
        .unwrap();
    assert_eq!(
        rows(
            db.execute_sql("SELECT b FROM t WHERE b = X'6869'")
                .await
                .unwrap()
        ),
        vec![vec![serde_json::json!("6869")]]
    );
    let blob = Value::Blob(vec![0x68, 0x69]);
    assert_eq!(blob.to_key(), None);
    assert_eq!(
        blob.secondary_index_key(&rtsql::storage::ColumnType::Blob),
        None
    );
}

fn hex_of(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
