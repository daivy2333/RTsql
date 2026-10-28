//! Catalog durability integration tests.
//!
//! `__tables` row updates used to run as "append a new copy of the row, then
//! delete the old one" and returned `Ok(())` when the page was full. Every
//! fixed-width update therefore leaked the row's bytes, and once the catalog
//! page was exhausted every further catalog write vanished silently: a fresh
//! process then read a stale data-page tail, so committed rows became
//! permanently invisible to scans and dumps while point lookups kept working
//! (the index trees were already rooted).
//!
//! The tests below pin durability across independent sessions (the one-shot CLI
//! main usage) and the persistence of index-root write-backs.

#![cfg(all(feature = "network", feature = "cli"))]
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use tempfile::{tempdir, TempDir};

const ROWS_PER_ROUND: i64 = 1_000;
const LIB_ROUNDS: usize = 15;
const CLI_ROUNDS: usize = 12;
/// Bulk-import control size. The import path commits per row (one WAL
/// `sync_all` each, measured 4.1 ms/row), so 30k rows would need ~123 s and
/// blow the one-shot child deadline; 5k rows keeps the control at ~21 s while
/// still spanning ~120 catalog tail updates — the regime the fix removes.
const IMPORT_ROWS: i64 = 5_000;

const CREATE_SQL: &str = "CREATE TABLE people (id INT PRIMARY KEY, name VARCHAR(64))";

/// One single-statement multi-row INSERT. Names are padded to the declared
/// `VARCHAR(64)` width so each round allocates enough data pages to move the
/// catalog tail many times — the pre-fix leak had to reach page exhaustion.
fn insert_sql(first_id: i64, rows: i64) -> String {
    let mut sql = String::with_capacity(rows as usize * 96);
    sql.push_str("INSERT INTO people (id, name) VALUES ");
    for i in 0..rows {
        if i > 0 {
            sql.push(',');
        }
        let id = first_id + i;
        sql.push_str(&format!("({id}, '{id:0>64}')"));
    }
    sql
}

async fn count_rows(path: &Path) -> i64 {
    let db = Database::open(path).await.expect("open for count");
    let count = match to_response(db.execute_sql("SELECT COUNT(*) FROM people").await) {
        Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 1, "COUNT(*) must return one row");
            rows[0][0].as_i64().expect("COUNT(*) is numeric")
        }
        other => panic!("expected QueryResult, got {other:?}"),
    };
    db.close().await.expect("close after count");
    count
}

/// A single-value aggregate query (`COUNT` / `MIN` / `MAX`).
async fn scalar(db: &Database, sql: &str) -> i64 {
    match to_response(db.execute_sql(sql).await) {
        Response::QueryResult { rows } => {
            assert_eq!(rows.len(), 1, "{sql} must return one row");
            rows[0][0]
                .as_i64()
                .unwrap_or_else(|| panic!("{sql} must be numeric"))
        }
        other => panic!("expected QueryResult for {sql}, got {other:?}"),
    }
}

/// One write session: open → one INSERT statement → close, released so the
/// next round can take the advisory lock.
async fn insert_round(path: &Path, first_id: i64) {
    let db = Database::open(path).await.expect("open for insert");
    match to_response(db.execute_sql(&insert_sql(first_id, ROWS_PER_ROUND)).await) {
        Response::AffectedRows { count } => {
            assert_eq!(count as i64, ROWS_PER_ROUND, "all rows of the round commit")
        }
        other => panic!("expected AffectedRows, got {other:?}"),
    }
    db.close().await.expect("close after insert");
    drop(db);
}

struct CliRun {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Run the one-shot CLI once. `RTSQL_HOME` points at `home` (its `db/`
/// subdirectory must exist — the CLI creates no parent), arguments are bare
/// names, and both streams go to files so a long render can never block the
/// child on a full pipe.
fn run_cli(home: &Path, args: &[&str]) -> CliRun {
    let out_path = home.join("cli-stdout.txt");
    let err_path = home.join("cli-stderr.txt");
    let out_file = File::create(&out_path).expect("stdout sink");
    let err_file = File::create(&err_path).expect("stderr sink");

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rtsql"));
    cmd.args(args)
        .current_dir(home)
        .env("RTSQL_HOME", home)
        .env_remove("RTSQL_KEY")
        .env_remove("RTSQL_WAL_CHECKPOINT_BYTES")
        .stdin(Stdio::null())
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::from(err_file));
    let mut child = cmd.spawn().expect("spawn rtsql");

    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if child.try_wait().expect("try_wait").is_some() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "rtsql {args:?} did not exit within 120s — a one-shot CLI must terminate"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    let status = child.wait().expect("wait rtsql");
    CliRun {
        code: status.code().expect("exit code"),
        stdout: std::fs::read_to_string(&out_path).unwrap_or_default(),
        stderr: std::fs::read_to_string(&err_path).unwrap_or_default(),
    }
}

fn cli_home() -> TempDir {
    let home = tempdir().expect("tempdir");
    std::fs::create_dir(home.path().join("db")).expect("pre-create db/ (the CLI does not)");
    home
}

fn cli_db_path(home: &Path, stem: &str) -> PathBuf {
    home.join("db").join(format!("{stem}.db"))
}

/// Multi-round cross-session durability: every round's rows must be visible to
/// the *next* session (count), to a dump (backup completeness) and to point
/// lookups on the first / middle / last key.
#[tokio::test]
async fn lib_sessions_keep_every_committed_row_visible() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("lib-rounds.db");

    {
        let db = Database::open(&path).await.expect("open for create");
        to_response(db.execute_sql(CREATE_SQL).await);
        db.close().await.expect("close after create");
        drop(db);
    }

    for round in 1..=LIB_ROUNDS {
        insert_round(&path, (round as i64 - 1) * ROWS_PER_ROUND + 1).await;
        let expected = round as i64 * ROWS_PER_ROUND;
        let count = count_rows(&path).await;
        assert_eq!(
            count, expected,
            "round {round}: a fresh session must see all {expected} committed rows"
        );
    }

    let total = LIB_ROUNDS as i64 * ROWS_PER_ROUND;

    // The five read faces must agree: COUNT(*), a range scan over the whole
    // key space, MIN/MAX, the point lookups below and the dump.
    {
        let db = Database::open(&path).await.expect("open for scan faces");
        assert_eq!(scalar(&db, "SELECT COUNT(*) FROM people").await, total);
        assert_eq!(scalar(&db, "SELECT MIN(id) FROM people").await, 1);
        assert_eq!(scalar(&db, "SELECT MAX(id) FROM people").await, total);
        assert_eq!(
            scalar(
                &db,
                &format!("SELECT COUNT(*) FROM people WHERE id >= 1 AND id <= {total}")
            )
            .await,
            total,
            "range scan must reach every committed row"
        );
        db.close().await.expect("close after scan faces");
        drop(db);
    }

    // Point lookups across the whole key range.
    for id in [1, total / 2, total] {
        let db = Database::open(&path).await.expect("open for point query");
        match to_response(
            db.execute_sql(&format!("SELECT name FROM people WHERE id = {id}"))
                .await,
        ) {
            Response::QueryResult { rows } => {
                assert_eq!(rows.len(), 1, "point lookup for id {id} must hit")
            }
            other => panic!("expected QueryResult, got {other:?}"),
        }
        db.close().await.expect("close after point query");
        drop(db);
    }

    // Dump completeness: the scan chain must reach every committed row. The
    // database lives outside the CLI home, so it is addressed by path.
    let home = cli_home();
    let run = run_cli(
        home.path(),
        &["dump", path.to_str().expect("utf-8 db path")],
    );
    assert_eq!(run.code, 0, "dump failed, stderr: {}", run.stderr);
    let insert_lines = run
        .stdout
        .lines()
        .filter(|l| l.starts_with("INSERT INTO"))
        .count() as i64;
    assert_eq!(insert_lines, total, "dump must export every committed row");
}

/// The same durability through independent CLI processes — the one-shot main
/// usage, where each round is a fresh process on the same database file.
#[tokio::test]
async fn cli_sessions_keep_every_committed_row_visible() {
    let home = cli_home();
    let stem = "cli-rounds";

    let created = run_cli(home.path(), &["new", stem]);
    assert_eq!(created.code, 0, "rtsql new failed: {}", created.stderr);
    run_cli(home.path(), &[stem, CREATE_SQL]);

    for round in 1..=CLI_ROUNDS {
        let first_id = (round as i64 - 1) * ROWS_PER_ROUND + 1;
        let run = run_cli(home.path(), &[stem, &insert_sql(first_id, ROWS_PER_ROUND)]);
        assert_eq!(run.code, 0, "round {round} failed, stderr: {}", run.stderr);
    }

    let count = count_rows(&cli_db_path(home.path(), stem)).await;
    assert_eq!(
        count,
        CLI_ROUNDS as i64 * ROWS_PER_ROUND,
        "every CLI round's rows must survive for the next process"
    );
}

/// Control: a single process writing a large batch stays correct (it was
/// already correct before the fix — the loss needed a second process reading a
/// stale catalog).
#[tokio::test]
async fn single_process_bulk_import_stays_correct() {
    let home = cli_home();
    let stem = "bulk-import";
    let created = run_cli(home.path(), &["new", stem]);
    assert_eq!(created.code, 0, "rtsql new failed: {}", created.stderr);
    run_cli(home.path(), &[stem, CREATE_SQL]);

    let csv_path = home.path().join("bulk.csv");
    let mut csv = String::with_capacity(IMPORT_ROWS as usize * 24);
    csv.push_str("id,name\n");
    for id in 1..=IMPORT_ROWS {
        csv.push_str(&format!("{id},{id:0>64}\n"));
    }
    std::fs::write(&csv_path, &csv).expect("write csv");

    let run = run_cli(
        home.path(),
        &[
            "import",
            stem,
            "people",
            csv_path.to_str().unwrap(),
            "--csv",
        ],
    );
    assert_eq!(run.code, 0, "import failed, stderr: {}", run.stderr);

    let db = Database::open(&cli_db_path(home.path(), stem))
        .await
        .expect("open after import");
    match to_response(db.execute_sql("SELECT COUNT(*) FROM people").await) {
        Response::QueryResult { rows } => {
            assert_eq!(rows[0][0].as_i64(), Some(IMPORT_ROWS));
        }
        other => panic!("expected QueryResult, got {other:?}"),
    }
    db.close().await.expect("close after import");
}

/// B-tree root write-backs go through the same catalog row, and a root move is
/// only observable after the catalog page is already exhausted: a root splits
/// early in a session (~100 entries), while the catalog page exhausts only
/// after a few thousand row writes. The witness therefore spans two write
/// sessions — the second one inserts past the tree depth that moves the root
/// while the catalog page can no longer accept an update.
#[tokio::test]
async fn index_root_write_backs_survive_reopen() {
    const FIRST_BATCH: i64 = 4_000;
    const SECOND_BATCH: i64 = 8_000;
    const TOTAL: i64 = FIRST_BATCH + SECOND_BATCH;

    let dir = tempdir().unwrap();

    // UNIQUE tree: a value inserted in the second session must still be
    // enforced as a duplicate after the restart.
    let uniq_path = dir.path().join("uniq.db");
    {
        let db = Database::open(&uniq_path).await.expect("open uniq");
        to_response(
            db.execute_sql(
                "CREATE TABLE uniq_rows (id INT PRIMARY KEY, code INT UNIQUE, tag VARCHAR(64))",
            )
            .await,
        );
        insert_codes(&db, "uniq_rows", 1, FIRST_BATCH).await;
        db.close().await.expect("close uniq batch 1");
        drop(db);
    }
    {
        let db = Database::open(&uniq_path).await.expect("reopen uniq");
        insert_codes(&db, "uniq_rows", FIRST_BATCH + 1, SECOND_BATCH).await;
        db.close().await.expect("close uniq batch 2");
        drop(db);
    }
    {
        let db = Database::open(&uniq_path)
            .await
            .expect("reopen uniq after batch 2");
        for code in [1, TOTAL / 2, TOTAL] {
            match to_response(db
                .execute_sql(&format!(
                    "INSERT INTO uniq_rows (id, code, tag) VALUES (999999, {code}, 'dup')"
                ))
                .await)
            {
                Response::Error { message, .. } => assert!(
                    !message.is_empty(),
                    "UNIQUE value {code} must be rejected after reopen"
                ),
                other => panic!(
                    "UNIQUE value {code} written in the last session must still be enforced, got {other:?}"
                ),
            }
        }
        match to_response(
            db.execute_sql(
                "INSERT INTO uniq_rows (id, code, tag) VALUES (999999, 8888888, 'fresh')",
            )
            .await,
        ) {
            Response::AffectedRows { count } => assert_eq!(count, 1),
            other => panic!("a new UNIQUE value must still insert, got {other:?}"),
        }
        db.close().await.expect("close uniq 2");
        drop(db);
    }

    // Secondary index: the root registered in the catalog must still route
    // equality queries after the restart, and agree with the scan route.
    let sec_path = dir.path().join("secondary.db");
    {
        let db = Database::open(&sec_path).await.expect("open secondary");
        to_response(
            db.execute_sql("CREATE TABLE idx_rows (id INT PRIMARY KEY, code INT, tag VARCHAR(64))")
                .await,
        );
        to_response(
            db.execute_sql("CREATE INDEX idx_code ON idx_rows (code)")
                .await,
        );
        insert_codes(&db, "idx_rows", 1, FIRST_BATCH).await;
        db.close().await.expect("close secondary batch 1");
        drop(db);
    }
    {
        let db = Database::open(&sec_path).await.expect("reopen secondary");
        insert_codes(&db, "idx_rows", FIRST_BATCH + 1, SECOND_BATCH).await;
        db.close().await.expect("close secondary batch 2");
        drop(db);
    }
    {
        let db = Database::open(&sec_path)
            .await
            .expect("reopen secondary after batch 2");

        let plan = match to_response(
            db.execute_sql("EXPLAIN SELECT id FROM idx_rows WHERE code = 4242")
                .await,
        ) {
            Response::QueryResult { rows } => rows
                .iter()
                .map(|r| {
                    r.iter()
                        .map(|c| c.to_string())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .collect::<Vec<_>>()
                .join("\n"),
            other => panic!("expected QueryResult, got {other:?}"),
        };
        assert!(
            plan.contains("IndexScanAll"),
            "equality query must route the secondary index after reopen, plan:\n{plan}"
        );

        for code in [1, 4242, TOTAL] {
            let indexed = match to_response(
                db.execute_sql(&format!("SELECT id FROM idx_rows WHERE code = {code}"))
                    .await,
            ) {
                Response::QueryResult { rows } => rows,
                other => panic!("expected QueryResult, got {other:?}"),
            };
            assert_eq!(indexed.len(), 1, "index route must find code {code}");
            assert_eq!(indexed[0][0].as_i64(), Some(code));

            // Same predicate without the plain-column shape → scan route; both
            // routes must agree.
            let scanned = match to_response(
                db.execute_sql(&format!("SELECT id FROM idx_rows WHERE code + 0 = {code}"))
                    .await,
            ) {
                Response::QueryResult { rows } => rows,
                other => panic!("expected QueryResult, got {other:?}"),
            };
            assert_eq!(indexed, scanned, "index route must equal the scan route");
        }

        drop(to_response(db.execute_sql("DROP INDEX idx_code").await));
        db.close().await.expect("close secondary 2");
        drop(db);
    }
}

/// One single-statement INSERT of `count` `(id, code, tag)` rows, with `code`
/// starting at `first_code`.
async fn insert_codes(db: &Database, table: &str, first_code: i64, count: i64) {
    let mut sql = format!("INSERT INTO {table} (id, code, tag) VALUES ");
    for i in 0..count {
        if i > 0 {
            sql.push(',');
        }
        let code = first_code + i;
        sql.push_str(&format!("({code},{code},'c{code}')"));
    }
    match to_response(db.execute_sql(&sql).await) {
        Response::AffectedRows { count: n } => assert_eq!(n as i64, count),
        other => panic!("expected AffectedRows, got {other:?}"),
    }
}
