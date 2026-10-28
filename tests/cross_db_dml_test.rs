//! Integration tests for MS20-T02 (`cross-db-dml` R1–R5, R8): routed writes to
//! attached databases, per-file independent commits, explicit-transaction /
//! read-only / DDL boundaries and crash recovery.
//!
//! Fixture (three CLI-created databases under an isolated `RTSQL_HOME`):
//!   main `app`     t(id,name) = (1,'a'),(2,'b')   — bare name stays main
//!                  "o.u"(id) = 5                  — literal dotted decoy
//!   attached `o`   u(id,tag) = (1,'x'),(9,'y')    — routed write target
//!                  t(id,tag) = (77,'attached')    — same-name decoy
//!                  c(id,n NOT NULL,uniq UNIQUE) + idx_c_n   — constraints/index
//!                  d(id,x DEFAULT 7,s DEFAULT 'z')          — DEFAULT application
//!                  f(id,x FLOAT,dt DATE)                    — typed values
//!   attached `p`   v(id,label) = (1,'p1')          — second target (partial commit)
//!
//! Scenario mapping (requirement → scenario → test):
//!   R1/S1 附加库写入且主库不变   attached_values_upsert_update_delete_route_and_reopen
//!   R1/S2 约束错误沿用单库保证   attached_defaults_constraints_and_indexes
//!        多行语句前行写入后行失败的回滚  attached_multi_row_failure_rolls_back_target
//!   R2/S1 后续失败保留此前提交   independent_file_commits_survive_later_error
//!   R3/S1 显式事务中拒绝附加写   attached_dml_in_active_tx_rejected_and_main_rolled_back
//!   R3/S2 只读会话拒绝附加写     readonly_attached_dml_rejected
//!   R3/R4 SQLSTATE 断言（0A000/25006）  routed_write_rejections_carry_sqlstate
//!   R4    DDL/CTAS 附加目标拒绝  existing_attached_ddl_and_ctas_rejections
//!   R4/S6 名字与未命中           routing_names_and_zero_match
//!   R5/S3 后续 SQL 错误仍关闭    attached_close_after_fail_fast
//!   R5/S5 目标库恢复索引一致     attached_routed_writes_crash_recovery
//!   R5    目标 checkpoint 失败保数据  attached_commit_checkpoint_failure_preserves_data
//!   R5/A5 `:memory:` 主库写附加   memory_primary_routes_write_to_attached_file
//!   R5/A5 加密附加同 key 写入重开  encrypted_attached_same_key_write_and_reopen
//!   R8/S9 子查询形态继续拒绝     update_delete_subquery_shapes_still_rejected
//!   （信号关闭 / close 失败状态保持见 `src/cli/mod.rs` 单元测试）

#![cfg(all(feature = "network", feature = "cli"))]
use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rtsql::parser::parse_sql;
use rtsql::pipeline::{
    execute_stage_in_tx_routed, execute_stage_routed, plan_stage_routed, AttachRoute, RoutedTables,
};
use rtsql::transaction::IsolationLevel;
use rtsql::PhysicalPlan;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::{tempdir, TempDir};

struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// 运行 rtsql 二进制并等待退出；60s 未退出则 kill 并 panic（夹具模式与
/// tests/cross_db_query_test.rs 一致）。
fn run_cli(dir: &Path, args: &[&str]) -> CliOutput {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
        .env_remove("RTSQL_BUSY_TIMEOUT")
        .env_remove("RTSQL_WAL_CHECKPOINT_BYTES")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rtsql binary");
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

/// 每个测试独立的仓库现场：TempDir + 预建 `db/` 集中存储目录。
fn cli_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("db")).unwrap();
    dir
}

fn run_cli_ok(dir: &Path, db: &str, sql: &str, what: &str) {
    let out = run_cli(dir, &[db, sql]);
    assert_eq!(out.code, Some(0), "{what} failed: {}", out.stderr);
}

fn assert_failure(out: &CliOutput, code: i32, needle: &str, what: &str) {
    assert_eq!(out.code, Some(code), "{what}: stderr was {}", out.stderr);
    assert!(
        out.stderr.contains(needle),
        "{what}: expected stderr containing {needle:?}, got {:?}",
        out.stderr
    );
}

fn assert_rows(out: &CliOutput, rows: &str, what: &str) {
    assert_eq!(out.code, Some(0), "{what}: stderr was {}", out.stderr);
    assert!(
        out.stdout.contains(rows),
        "{what}: expected stdout containing {rows}, got {:?}",
        out.stdout
    );
}

/// 主库 `app` + 附加库 `o`（含约束/索引/DEFAULT/类型表）+ `p`。
fn cross_db_fixture() -> TempDir {
    let dir = cli_fixture();
    for name in ["app", "o", "p"] {
        let out = run_cli(dir.path(), &["new", name]);
        assert_eq!(out.code, Some(0), "new {name} failed: {}", out.stderr);
    }
    run_cli_ok(
        dir.path(),
        "app",
        "CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR); \
         INSERT INTO t VALUES (1, 'a'); \
         INSERT INTO t VALUES (2, 'b'); \
         CREATE TABLE \"o.u\" (id INT); \
         INSERT INTO \"o.u\" VALUES (5)",
        "seed main db",
    );
    run_cli_ok(
        dir.path(),
        "o",
        "CREATE TABLE u (id INT PRIMARY KEY, tag VARCHAR); \
         INSERT INTO u VALUES (1, 'x'); \
         INSERT INTO u VALUES (9, 'y'); \
         CREATE TABLE t (id INT PRIMARY KEY, tag VARCHAR); \
         INSERT INTO t VALUES (77, 'attached'); \
         CREATE TABLE c (id INT PRIMARY KEY, n INT NOT NULL, uniq INT UNIQUE); \
         CREATE INDEX idx_c_n ON c (n); \
         CREATE TABLE d (id INT PRIMARY KEY, x INT DEFAULT 7, s VARCHAR DEFAULT 'z'); \
         CREATE TABLE f (id INT PRIMARY KEY, x FLOAT, dt DATE)",
        "seed attached o",
    );
    run_cli_ok(
        dir.path(),
        "p",
        "CREATE TABLE v (id INT PRIMARY KEY, label VARCHAR); \
         INSERT INTO v VALUES (1, 'p1')",
        "seed attached p",
    );
    dir
}

// ---------------------------------------------------------------------------
// R1 — routed ordinary DML (VALUES / UPSERT / REPLACE / UPDATE / DELETE)
// ---------------------------------------------------------------------------

/// R1/S1：附加库 VALUES/UPSERT/REPLACE/UPDATE/DELETE 全部路由到附加库
/// （UPDATE 含主键列 rekey 形态），主库同名 decoy 与主库表零变化；
/// 关闭重开后结果一致。
#[test]
fn attached_values_upsert_update_delete_route_and_reopen() {
    let dir = cross_db_fixture();

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; \
             INSERT INTO o.u VALUES (5, 'z'); \
             INSERT INTO o.u VALUES (5, 'z2') ON CONFLICT (id) DO UPDATE SET tag = 'z3'; \
             REPLACE INTO o.u VALUES (1, 'r'); \
             UPDATE o.u SET tag = 'q' WHERE id = 9; \
             DELETE FROM o.u WHERE id = 9; \
             UPDATE o.u SET id = 2 WHERE id = 5",
        ],
    );
    assert_eq!(out.code, Some(0), "attached DML: {}", out.stderr);

    // 新调用重开：附加库行持久化（id 5 rekey 为 2）。
    let attached = run_cli(dir.path(), &["o", "SELECT * FROM u ORDER BY id"]);
    assert_rows(
        &attached,
        r#""rows":[[1,"r"],[2,"z3"]]"#,
        "attached rows after reopen (incl. rekey)",
    );

    // 主库同名 decoy 表未被写入；主库 t 未被写入。
    let decoy = run_cli(dir.path(), &["app", "SELECT * FROM \"o.u\" ORDER BY id"]);
    assert_rows(&decoy, r#""rows":[[5]]"#, "main decoy untouched");
    let main_t = run_cli(dir.path(), &["app", "SELECT * FROM t ORDER BY id"]);
    assert_rows(&main_t, r#""rows":[[1,"a"],[2,"b"]]"#, "main t untouched");
}

/// R1/S2：附加库写入沿用单库 DEFAULT / 类型门 / NOT NULL / UNIQUE /
/// 二级索引语义。
#[test]
fn attached_defaults_constraints_and_indexes() {
    let dir = cross_db_fixture();

    // 子集列清单 INSERT → DEFAULT 生效。
    let out = run_cli(
        dir.path(),
        &["app", "ATTACH 'o' AS o; INSERT INTO o.d (id) VALUES (1)"],
    );
    assert_eq!(out.code, Some(0), "subset insert: {}", out.stderr);
    let defaults = run_cli(dir.path(), &["o", "SELECT * FROM d ORDER BY id"]);
    assert_rows(&defaults, r#""rows":[[1,7,"z"]]"#, "DEFAULT applied");

    // FLOAT / DATE / NULL 值经路由写入。
    let typed = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; \
             INSERT INTO o.f VALUES (1, 1.5, DATE '2024-01-02'); \
             INSERT INTO o.f VALUES (2, NULL, NULL)",
        ],
    );
    assert_eq!(typed.code, Some(0), "typed insert: {}", typed.stderr);

    // NOT NULL 违反 → 点名拒绝，零副作用。
    let not_null = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; INSERT INTO o.c (id, n, uniq) VALUES (1, NULL, 100)",
        ],
    );
    assert_failure(&not_null, 3, "NOT NULL", "NOT NULL enforcement");

    // UNIQUE 违反 → DuplicateKey，当前语句零副作用。
    let first = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; INSERT INTO o.c (id, n, uniq) VALUES (1, 10, 100)",
        ],
    );
    assert_eq!(first.code, Some(0), "first c insert: {}", first.stderr);
    let dup = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; INSERT INTO o.c (id, n, uniq) VALUES (2, 11, 100)",
        ],
    );
    assert_failure(&dup, 3, "Duplicate key", "UNIQUE enforcement");

    // 二级索引等值路由命中附加库。
    let routed = run_cli(
        dir.path(),
        &["app", "ATTACH 'o' AS o; SELECT * FROM o.c WHERE n = 10"],
    );
    assert_rows(&routed, r#""rows":[[1,10,100]]"#, "secondary index route");
}

/// R1/S2（Plan Review F1）：附加库**多行** INSERT 前行已写入产生版本与
/// 索引条目、后行约束失败——当前语句整体回滚：目标行零残留、PK 与唯一值
/// 可重新使用、二级索引在回滚后重建的条目上路由；主库与此前提交保持。
#[test]
fn attached_multi_row_failure_rolls_back_target() {
    let dir = cross_db_fixture();

    // 两行 VALUES：第一行 (1,10,100) 已写入，第二行 n=NULL 违反 NOT NULL
    // → 语句失败（此前单行/前置拒绝输入证明不了回滚路径）。
    let out = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; \
             INSERT INTO o.c (id, n, uniq) VALUES (1, 10, 100), (2, NULL, 101)",
        ],
    );
    assert_failure(&out, 3, "NOT NULL", "multi-row insert fails on row 2");

    // 回滚见证：目标表零残留（首行也被还原）。
    let c = run_cli(dir.path(), &["o", "SELECT * FROM c ORDER BY id"]);
    assert_rows(&c, r#""rows":[]"#, "failed statement fully rolled back");

    // 键 id=1 与唯一值 100 可重新使用；二级索引路由到重建后的条目。
    let retry = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; INSERT INTO o.c (id, n, uniq) VALUES (1, 10, 100)",
        ],
    );
    assert_eq!(
        retry.code,
        Some(0),
        "keys reusable after rollback: {}",
        retry.stderr
    );
    let idx = run_cli(
        dir.path(),
        &["app", "ATTACH 'o' AS o; SELECT * FROM o.c WHERE n = 10"],
    );
    assert_rows(
        &idx,
        r#""rows":[[1,10,100]]"#,
        "secondary index after rollback",
    );

    // 主库与此前提交（fixture 的 o.u 两行）保持。
    let main_t = run_cli(dir.path(), &["app", "SELECT * FROM t ORDER BY id"]);
    assert_rows(&main_t, r#""rows":[[1,"a"],[2,"b"]]"#, "main untouched");
    let o_u = run_cli(dir.path(), &["o", "SELECT COUNT(*) FROM u"]);
    assert_rows(&o_u, r#""rows":[[2]]"#, "prior commits kept");
}

// ---------------------------------------------------------------------------
// R2 — per-file independent commits
// ---------------------------------------------------------------------------

/// R2/S1：主库 + 两个附加库依次提交后，下一条 DML 失败不撤销此前提交。
#[test]
fn independent_file_commits_survive_later_error() {
    let dir = cross_db_fixture();

    let out = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; ATTACH 'p' AS p; \
             INSERT INTO t VALUES (3, 'c'); \
             INSERT INTO o.u VALUES (5, 'z'); \
             INSERT INTO p.v VALUES (2, 'q'); \
             INSERT INTO o.u VALUES (5, 'dup')",
        ],
    );
    assert_failure(&out, 3, "statement 6 of 6 failed", "fail-fast on 6th");
    assert!(
        out.stderr.contains("previous statement(s) were committed"),
        "auto-commit context must report earlier commits kept: {}",
        out.stderr
    );

    let main_t = run_cli(dir.path(), &["app", "SELECT * FROM t ORDER BY id"]);
    assert_rows(
        &main_t,
        r#""rows":[[1,"a"],[2,"b"],[3,"c"]]"#,
        "main commit kept",
    );
    let o_u = run_cli(dir.path(), &["o", "SELECT * FROM u ORDER BY id"]);
    assert_rows(
        &o_u,
        r#""rows":[[1,"x"],[5,"z"],[9,"y"]]"#,
        "attached o commit kept",
    );
    let p_v = run_cli(dir.path(), &["p", "SELECT * FROM v ORDER BY id"]);
    assert_rows(
        &p_v,
        r#""rows":[[1,"p1"],[2,"q"]]"#,
        "attached p commit kept",
    );
}

// ---------------------------------------------------------------------------
// R3 — explicit transaction / read-only boundaries
// ---------------------------------------------------------------------------

/// R3/S1：显式 BEGIN 会话内附加库写入点名拒绝（0A000），附加库不变，
/// 主库未提交修改经既有错误收口回滚。
#[test]
fn attached_dml_in_active_tx_rejected_and_main_rolled_back() {
    let dir = cross_db_fixture();

    for attached_sql in [
        "INSERT INTO o.u VALUES (5, 'z')",
        "INSERT INTO o.u VALUES (5, 'z') ON CONFLICT (id) DO NOTHING",
        "REPLACE INTO o.u VALUES (5, 'z')",
        "UPDATE o.u SET tag = 'q' WHERE id = 1",
        "DELETE FROM o.u WHERE id = 9",
    ] {
        let sql = format!("ATTACH 'o' AS o; BEGIN; INSERT INTO t VALUES (3, 'c'); {attached_sql}");
        let out = run_cli(dir.path(), &["app", &sql]);
        assert_failure(&out, 3, "inside a transaction", "attached write in tx");
    }

    // 主库事务已回滚、附加库未被写入。
    let main_t = run_cli(dir.path(), &["app", "SELECT * FROM t ORDER BY id"]);
    assert_rows(&main_t, r#""rows":[[1,"a"],[2,"b"]]"#, "main rolled back");
    let o_u = run_cli(dir.path(), &["o", "SELECT * FROM u ORDER BY id"]);
    assert_rows(
        &o_u,
        r#""rows":[[1,"x"],[9,"y"]]"#,
        "attached unchanged in tx",
    );
}

/// R3/S2：只读会话对附加库写入按 25006 拒绝且数据不变（含 BEGIN 形态）。
#[test]
fn readonly_attached_dml_rejected() {
    let dir = cross_db_fixture();

    for sql in [
        "ATTACH 'o' AS o; INSERT INTO o.u VALUES (5, 'z')",
        "ATTACH 'o' AS o; BEGIN; INSERT INTO o.u VALUES (5, 'z')",
        "ATTACH 'o' AS o; UPDATE o.u SET tag = 'q' WHERE id = 1",
        "ATTACH 'o' AS o; DELETE FROM o.u WHERE id = 9",
    ] {
        let out = run_cli(dir.path(), &["--readonly", "app", sql]);
        assert_failure(&out, 3, "read-only session", "readonly attached write");
    }

    let o_u = run_cli(dir.path(), &["o", "SELECT * FROM u ORDER BY id"]);
    assert_rows(&o_u, r#""rows":[[1,"x"],[9,"y"]]"#, "readonly data intact");
}

// ---------------------------------------------------------------------------
// R4 — DDL / CTAS attached-target rejections and name handling
// ---------------------------------------------------------------------------

/// R4：附加库 DDL 与附加目标 CTAS 继续拒绝。
#[test]
fn existing_attached_ddl_and_ctas_rejections() {
    let dir = cross_db_fixture();

    for sql in [
        "ATTACH 'o' AS o; CREATE TABLE o.newt (a INT)",
        "ATTACH 'o' AS o; DROP TABLE o.u",
        "ATTACH 'o' AS o; CREATE TABLE o.ct AS SELECT id FROM t",
    ] {
        let out = run_cli(dir.path(), &["app", sql]);
        assert_failure(&out, 3, "ATTACHED DATABASE", "attached DDL/CTAS rejection");
    }

    // 附加库 schema 与数据不变。
    let missing = run_cli(dir.path(), &["o", "SELECT * FROM newt"]);
    assert_failure(&missing, 3, "not found", "no table created");
    let o_u = run_cli(dir.path(), &["o", "SELECT * FROM u ORDER BY id"]);
    assert_rows(&o_u, r#""rows":[[1,"x"],[9,"y"]]"#, "attached u intact");
}

/// R4/S6：裸名恒主库；未注册别名回落扁平名（42P01）；含点字面表名不变；
/// UPDATE 缺键 `KeyNotFound` 与 DELETE 缺键 affected=0 不统一为零行。
#[test]
fn routing_names_and_zero_match() {
    let dir = cross_db_fixture();

    let bare = run_cli(
        dir.path(),
        &["app", "ATTACH 'o' AS o; INSERT INTO t VALUES (3, 'c')"],
    );
    assert_eq!(bare.code, Some(0), "bare insert: {}", bare.stderr);
    let main_t = run_cli(dir.path(), &["app", "SELECT * FROM t ORDER BY id"]);
    assert_rows(
        &main_t,
        r#""rows":[[1,"a"],[2,"b"],[3,"c"]]"#,
        "bare name writes main",
    );

    let fallback = run_cli(
        dir.path(),
        &["app", "ATTACH 'o' AS o; INSERT INTO z.t VALUES (1, 'a')"],
    );
    assert_failure(
        &fallback,
        3,
        "'z.t' not found",
        "unregistered alias fallback",
    );

    let dotted = run_cli(
        dir.path(),
        &["app", "ATTACH 'o' AS o; SELECT * FROM foo.bar"],
    );
    assert_failure(&dotted, 3, "not found", "literal dotted name stays flat");

    // UPDATE 未命中键 → 点名错误（非静默零行）。
    let upd = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; UPDATE o.u SET tag = 'q' WHERE id = 999",
        ],
    );
    assert_failure(&upd, 3, "Key not found", "UPDATE miss is an error");

    // DELETE 未命中键 → affected 0，退出 0。逐行精确断言：ATTACH 与 DELETE
    // 各渲染一行 `{"affected_rows":0}`——宽松 `contains("0")` 会被 ATTACH 行
    // 满足，证明不了 DELETE 自身的报告。
    let del = run_cli(
        dir.path(),
        &["app", "ATTACH 'o' AS o; DELETE FROM o.u WHERE id = 999"],
    );
    assert_eq!(del.code, Some(0), "DELETE miss exits 0: {}", del.stderr);
    let lines: Vec<&str> = del.stdout.lines().collect();
    assert_eq!(
        lines,
        vec![r#"{"affected_rows":0}"#, r#"{"affected_rows":0}"#],
        "DELETE miss must render its own affected_rows:0 line, got {:?}",
        del.stdout
    );
}

// ---------------------------------------------------------------------------
// R5 — close / recovery lifecycle
// ---------------------------------------------------------------------------

/// R5/S3：后续 SQL 失败退出后，此前写入的附加库均完成关闭（WAL 截断、
/// 重开可读）。
#[test]
fn attached_close_after_fail_fast() {
    let dir = cross_db_fixture();

    // 先向附加库 `o` 灌入 >1KB WAL（默认 16MiB 水位不触发自动 checkpoint），
    // 使 close 的 checkpoint 成为唯一截断来源。
    let mut sql = String::from("ATTACH 'o' AS o; ");
    for i in 0..300 {
        sql.push_str(&format!(
            "INSERT INTO o.u VALUES ({}, 'p{}'); ",
            1000 + i,
            i
        ));
    }
    // 最后一条重复主键 → fail-fast。
    sql.push_str("INSERT INTO o.u VALUES (1000, 'dup')");
    let out = run_cli(dir.path(), &["app", &sql]);
    assert_failure(&out, 3, "Duplicate key", "bulk then fail");

    // close 的 checkpoint 已截断附加库 WAL（未关闭则保留全部记录）。
    let wal = dir.path().join("db").join("o.wal");
    let len = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
    assert!(
        len < 1024,
        "attached WAL must be checkpointed by close after fail-fast, got {len} bytes"
    );

    // 重开可读（锁已释放）且已提交行完整。
    let o_u = run_cli(dir.path(), &["o", "SELECT COUNT(*) FROM u"]);
    assert_rows(&o_u, r#""rows":[[302]]"#, "committed rows survive");
}

/// R5/S5：经真实 routed pipeline 写入后（含 UPSERT、主键 rekey、多行语句
/// 前行已写入后行唯一冲突的失败 abort），WAL 保留提交记录而未 checkpoint
/// 时崩溃重开，目标库行 / PK 点查 / 唯一占用 / 二级索引查询一致，主库不变。
/// abort 后不再写入即崩溃（abort 后槽位重用 × 崩溃恢复是范围外既有引擎
/// 缺陷，见 Act Response Issue 候选；键重用运行期见证在 CLI 用例）。
#[tokio::test]
async fn attached_routed_writes_crash_recovery() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("main.db");
    let other_path = dir.path().join("other.db");

    {
        let other = Database::open(&other_path).await.unwrap();
        assert_ok(
            to_response(
                other
                    .execute_sql(
                        "CREATE TABLE c (id INT PRIMARY KEY, n INT NOT NULL, uniq INT UNIQUE)",
                    )
                    .await,
            ),
            "create attached c",
        );
        assert_ok(
            to_response(other.execute_sql("CREATE INDEX idx_c_n ON c (n)").await),
            "create secondary index",
        );
        other.checkpoint().await.unwrap();
        other.close().await.unwrap();
    }

    let main = Database::open(&main_path).await.unwrap();
    assert_ok(
        to_response(
            main.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
                .await,
        ),
        "create main t",
    );
    // DDL 不写 WAL：先 checkpoint 落盘 schema，再写数据（重开恢复重放）。
    main.checkpoint().await.unwrap();
    // 主库种子行：恢复后主库不变断言的基线。
    assert_ok(
        to_response(main.execute_sql("INSERT INTO t VALUES (1)").await),
        "seed main t",
    );

    let other = Arc::new(Database::open(&other_path).await.unwrap());
    let mut dbs = HashMap::new();
    dbs.insert("o".to_string(), other.clone());
    let route = AttachRoute::new(dbs);

    // 成功的 routed 写入：INSERT / UPSERT / rekey UPDATE / DELETE。
    routed_ok(
        &main,
        &route,
        "INSERT INTO o.c (id, n, uniq) VALUES (1, 10, 100)",
    )
    .await;
    routed_ok(
        &main,
        &route,
        "INSERT INTO o.c (id, n, uniq) VALUES (1, 15, 100) \
         ON CONFLICT (id) DO UPDATE SET n = 15",
    )
    .await;
    // rekey：主键列 1 → 10（PK 索引条目迁移）。
    routed_ok(&main, &route, "UPDATE o.c SET id = 10 WHERE id = 1").await;
    routed_ok(
        &main,
        &route,
        "INSERT INTO o.c (id, n, uniq) VALUES (3, 30, 300)",
    )
    .await;
    routed_ok(&main, &route, "DELETE FROM o.c WHERE id = 3").await;

    // 多行语句：前行 (2,11,101) 已写入产生版本与索引条目，后行 (4,41,101)
    // 与前行同语句内唯一冲突 → 语句失败。
    let fail = routed_exec(
        &main,
        &route,
        "INSERT INTO o.c (id, n, uniq) VALUES (2, 11, 101), (4, 41, 101)",
    )
    .await;
    assert!(
        matches!(fail, Response::Error { .. }),
        "in-statement unique conflict must fail: {fail:?}"
    );
    // 回滚见证（同实例直查目标库）：仅 rekey 后的行存活，id 2/4 零残留。
    let rows = query(&other, "SELECT * FROM c ORDER BY id").await;
    assert_eq!(
        rows,
        vec![vec![
            serde_json::json!(10),
            serde_json::json!(15),
            serde_json::json!(100)
        ]],
        "failed multi-row statement must roll back its earlier rows"
    );
    // PK 点查命中 rekey 后的行（运行期）。
    let pk = query(&other, "SELECT * FROM c WHERE id = 10").await;
    assert_eq!(
        pk,
        vec![vec![
            serde_json::json!(10),
            serde_json::json!(15),
            serde_json::json!(100)
        ]],
        "PK point query must reach the rekeyed row"
    );
    // 主库不变（崩溃前）。
    let main_rows = query(&main, "SELECT * FROM t ORDER BY id").await;
    assert_eq!(
        main_rows,
        vec![vec![serde_json::json!(1)]],
        "main db must be untouched by attached writes"
    );

    // 崩溃模拟：冲刷 WAL 后台缓冲后丢弃实例（不 close、不 checkpoint）。
    // `route` 持有 `other` 的 Arc 克隆——一并丢弃以释放文件锁。
    other.wal_buffer.as_ref().unwrap().shutdown().await;
    drop(other);
    drop(route);
    drop(main);

    // 重开目标库：行（abort 行已中性化）、PK 点查、唯一占用与二级索引一致。
    let reopened = Database::open(&other_path).await.unwrap();
    let rows = query(&reopened, "SELECT * FROM c ORDER BY id").await;
    assert_eq!(
        rows,
        vec![vec![
            serde_json::json!(10),
            serde_json::json!(15),
            serde_json::json!(100)
        ]],
        "recovered rows must match committed routed writes"
    );
    // 唯一占用恢复：100 仍被 id=10 占用。
    let dup = to_response(
        reopened
            .execute_sql("INSERT INTO c (id, n, uniq) VALUES (9, 9, 100)")
            .await,
    );
    assert!(
        matches!(dup, Response::Error { .. }),
        "unique occupancy must survive recovery: {dup:?}"
    );
    // PK 点查恢复后可达。
    let pk = query(&reopened, "SELECT * FROM c WHERE id = 10").await;
    assert_eq!(
        pk,
        vec![vec![
            serde_json::json!(10),
            serde_json::json!(15),
            serde_json::json!(100)
        ]],
        "PK point query must survive recovery"
    );
    // 二级索引路由恢复后可达。
    let routed_rows = query(&reopened, "SELECT * FROM c WHERE n = 15").await;
    assert_eq!(
        routed_rows,
        vec![vec![
            serde_json::json!(10),
            serde_json::json!(15),
            serde_json::json!(100)
        ]],
        "secondary index route must survive recovery"
    );
    reopened.close().await.unwrap();

    // 主库不变：崩溃重开后种子行原样。
    let main_reopened = Database::open(&main_path).await.unwrap();
    let main_rows = query(&main_reopened, "SELECT * FROM t ORDER BY id").await;
    assert_eq!(
        main_rows,
        vec![vec![serde_json::json!(1)]],
        "main db must be untouched by attached writes and recovery"
    );
    main_reopened.close().await.unwrap();
}

/// R5（design D3）：每个提交库按自身 checkpoint 水位检查——附加库设 1 字节
/// 水位后 routed 写入使附加 WAL 有界，主库水位 0（禁用）时主库 WAL 不被误
/// checkpoint。
#[tokio::test]
async fn attached_commit_uses_own_checkpoint_threshold() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("main.db");
    let other_path = dir.path().join("other.db");

    let other = Arc::new(Database::open(&other_path).await.unwrap());
    assert_ok(
        to_response(
            other
                .execute_sql("CREATE TABLE c (id INT PRIMARY KEY, pad VARCHAR)")
                .await,
        ),
        "create attached c",
    );
    other.checkpoint().await.unwrap();
    other.set_checkpoint_bytes_threshold(1);

    let main = Database::open(&main_path).await.unwrap();
    assert_ok(
        to_response(
            main.execute_sql("CREATE TABLE t (id INT PRIMARY KEY, pad VARCHAR)")
                .await,
        ),
        "create main t",
    );
    main.set_checkpoint_bytes_threshold(0);

    let mut dbs = HashMap::new();
    dbs.insert("o".to_string(), other.clone());
    let route = AttachRoute::new(dbs);

    for i in 0..300 {
        assert_ok(
            to_response(
                main.execute_sql(&format!("INSERT INTO t (id, pad) VALUES ({i}, 'p{i}')"))
                    .await,
            ),
            "main insert",
        );
    }
    for i in 0..20 {
        routed_ok(
            &main,
            &route,
            &format!("INSERT INTO o.c (id, pad) VALUES ({i}, 'q{i}')"),
        )
        .await;
    }

    let main_wal = std::fs::metadata(main_path.with_extension("wal"))
        .map(|m| m.len())
        .unwrap_or(0);
    let other_wal = std::fs::metadata(other_path.with_extension("wal"))
        .map(|m| m.len())
        .unwrap_or(0);
    assert!(
        main_wal >= 1024,
        "main WAL must not be checkpointed (threshold disabled), got {main_wal}"
    );
    assert!(
        other_wal < 1024,
        "attached WAL must stay bounded by its own threshold, got {other_wal}"
    );

    drop(route);
    drop(other);
    drop(main);
}

/// R5（Plan Review F3，design D3）：目标库提交后的自动 checkpoint 失败——
/// 事务已提交（错误消息点名 `Commit failed` / `checkpoint failed`），已提交
/// 行与索引保持、主库不变；修复现场后重开数据可读。
#[tokio::test]
async fn attached_commit_checkpoint_failure_preserves_data() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("main.db");
    let other_path = dir.path().join("other.db");

    let other = Arc::new(Database::open(&other_path).await.unwrap());
    assert_ok(
        to_response(
            other
                .execute_sql("CREATE TABLE c (id INT PRIMARY KEY, n INT NOT NULL, uniq INT UNIQUE)")
                .await,
        ),
        "create attached c",
    );
    assert_ok(
        to_response(other.execute_sql("CREATE INDEX idx_c_n ON c (n)").await),
        "create secondary index",
    );
    other.checkpoint().await.unwrap();
    // 1 字节水位：下一次提交边界必然触发自动 checkpoint（注入后必失败）。
    other.set_checkpoint_bytes_threshold(1);

    let main = Database::open(&main_path).await.unwrap();
    assert_ok(
        to_response(
            main.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
                .await,
        ),
        "create main t",
    );
    assert_ok(
        to_response(main.execute_sql("INSERT INTO t VALUES (1)").await),
        "seed main t",
    );

    let mut dbs = HashMap::new();
    dbs.insert("o".to_string(), other.clone());
    let route = AttachRoute::new(dbs);

    // 注入 checkpoint 步骤 5（位点写入）失败：位点路径被目录占据。
    let site = other_path.with_extension("checkpoint");
    let _ = std::fs::remove_file(&site);
    std::fs::create_dir(&site).unwrap();

    let resp = routed_exec(
        &main,
        &route,
        "INSERT INTO o.c (id, n, uniq) VALUES (1, 10, 100)",
    )
    .await;
    match resp {
        Response::Error { message, .. } => {
            assert!(
                message.contains("Commit failed") && message.contains("checkpoint failed"),
                "commit-boundary checkpoint failure must be named, got: {message}"
            );
        }
        other => panic!("expected Error from failed checkpoint, got {other:?}"),
    }

    // 已提交数据保持（R4：checkpoint 失败不回滚已提交事务）——同实例行、
    // 唯一占用与二级索引路由均在。
    let rows = query(&other, "SELECT * FROM c ORDER BY id").await;
    assert_eq!(
        rows,
        vec![vec![
            serde_json::json!(1),
            serde_json::json!(10),
            serde_json::json!(100)
        ]],
        "committed row must survive checkpoint failure"
    );
    let dup = to_response(
        other
            .execute_sql("INSERT INTO c (id, n, uniq) VALUES (2, 11, 100)")
            .await,
    );
    assert!(
        matches!(dup, Response::Error { .. }),
        "unique occupancy must be kept: {dup:?}"
    );
    let idx = query(&other, "SELECT * FROM c WHERE n = 10").await;
    assert_eq!(idx.len(), 1, "secondary index must be kept");
    // 主库不变。
    let main_rows = query(&main, "SELECT * FROM t ORDER BY id").await;
    assert_eq!(
        main_rows,
        vec![vec![serde_json::json!(1)]],
        "main db must be untouched"
    );

    // 崩溃丢弃 + 修复现场后重开：数据经 WAL 恢复。
    other.wal_buffer.as_ref().unwrap().shutdown().await;
    drop(other);
    drop(route);
    drop(main);
    let _ = std::fs::remove_dir_all(&site);
    let reopened = Database::open(&other_path).await.unwrap();
    let rows = query(&reopened, "SELECT * FROM c ORDER BY id").await;
    assert_eq!(
        rows,
        vec![vec![
            serde_json::json!(1),
            serde_json::json!(10),
            serde_json::json!(100)
        ]],
        "committed row must be recoverable after checkpoint failure"
    );
    reopened.close().await.unwrap();
}

/// R3/R4（Plan Review F3）：routed 拒绝面直接断言 SQLSTATE——显式事务内
/// 附加写 0A000（先于执行器构造）、只读会话 25006（含显式事务形态，只读
/// 优先于附加写拒绝）；主库事务 + 附加源 SELECT 成功的既有组合保持。
#[tokio::test]
async fn routed_write_rejections_carry_sqlstate() {
    let dir = tempdir().unwrap();
    let main_path = dir.path().join("main.db");
    let other_path = dir.path().join("other.db");
    let ro_path = dir.path().join("ro.db");

    // 种子块作用域：实例 drop 释放文件锁后才能重开（close 只落盘不释放锁）。
    {
        let other = Database::open(&other_path).await.unwrap();
        assert_ok(
            to_response(
                other
                    .execute_sql("CREATE TABLE u (id INT PRIMARY KEY, tag VARCHAR)")
                    .await,
            ),
            "create attached u",
        );
        assert_ok(
            to_response(other.execute_sql("INSERT INTO u VALUES (1, 'x')").await),
            "seed u",
        );
        other.checkpoint().await.unwrap();
        other.close().await.unwrap();
    }

    // 主库（rw）段块作用域：plan/routed 持有附加库 Arc 克隆，随块一起
    // 释放文件锁后才能进入只读段重开附加库。
    {
        let main = Database::open(&main_path).await.unwrap();
        assert_ok(
            to_response(
                main.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)")
                    .await,
            ),
            "create main t",
        );
        let other = Arc::new(Database::open(&other_path).await.unwrap());
        let mut dbs = HashMap::new();
        dbs.insert("o".to_string(), other.clone());
        let route = AttachRoute::new(dbs);

        // 显式事务 + 附加写 → 0A000。
        let tx = main.begin().await.unwrap();
        let (plan, routed) = routed_plan(&main, &route, "INSERT INTO o.u VALUES (5, 'z')").await;
        let resp = to_response(execute_stage_in_tx_routed(&main, plan, tx.id(), &routed).await);
        match resp {
            Response::Error { message, sqlstate } => {
                assert_eq!(sqlstate.as_deref(), Some("0A000"), "sqlstate: {message}");
                assert!(
                    message.contains("transaction") && message.contains("o.u"),
                    "rejection must name the transaction boundary and target, got: {message}"
                );
            }
            other => panic!("expected Error in explicit tx, got {other:?}"),
        }
        main.rollback(tx).await.unwrap();

        // 主库事务 + 附加源 SELECT 成功（既有组合保持）。
        let tx = main.begin().await.unwrap();
        let (plan, routed) = routed_plan(&main, &route, "SELECT id FROM o.u").await;
        let resp = to_response(execute_stage_in_tx_routed(&main, plan, tx.id(), &routed).await);
        match resp {
            Response::QueryResult { rows } => {
                assert_eq!(
                    rows,
                    vec![vec![serde_json::json!(1)]],
                    "attached read in tx"
                )
            }
            other => panic!("expected QueryResult, got {other:?}"),
        }
        main.rollback(tx).await.unwrap();
        main.close().await.unwrap();
    }

    // 只读会话（干净库，无 WAL 未决）+ 附加写 → 25006（非事务入口）。
    {
        let seed = Database::open(&ro_path).await.unwrap();
        seed.close().await.unwrap();
    }
    let ro = Database::open_readonly_with_lock_wait(&ro_path, None, None)
        .await
        .unwrap();
    let other = Arc::new(Database::open(&other_path).await.unwrap());
    let mut dbs = HashMap::new();
    dbs.insert("o".to_string(), other.clone());
    let route = AttachRoute::new(dbs);

    let (plan, routed) = routed_plan(&ro, &route, "INSERT INTO o.u VALUES (5, 'z')").await;
    let resp = to_response(execute_stage_routed(&ro, plan, false, &routed).await);
    match resp {
        Response::Error { message, sqlstate } => {
            assert_eq!(sqlstate.as_deref(), Some("25006"), "sqlstate: {message}");
            assert!(message.contains("read-only"), "message: {message}");
        }
        other => panic!("expected readonly Error, got {other:?}"),
    }

    // 只读 + 显式事务形态 → 25006 优先于附加写 0A000。
    let tx = ro.begin().await.unwrap();
    let (plan, routed) = routed_plan(&ro, &route, "DELETE FROM o.u WHERE id = 1").await;
    let resp = to_response(execute_stage_in_tx_routed(&ro, plan, tx.id(), &routed).await);
    match resp {
        Response::Error { message, sqlstate } => {
            assert_eq!(
                sqlstate.as_deref(),
                Some("25006"),
                "readonly must win over attached-write rejection: {message}"
            );
        }
        other => panic!("expected readonly Error in tx, got {other:?}"),
    }
    ro.rollback(tx).await.unwrap();
    drop(route);
    drop(other);
    drop(ro);
}

/// R5/A5 运行形态组合：`:memory:` 主库经 ATTACH 路由写文件附加库，
/// 数据落盘且重开可读。
#[test]
fn memory_primary_routes_write_to_attached_file() {
    let dir = cross_db_fixture();

    let out = run_cli(
        dir.path(),
        &[
            ":memory:",
            "ATTACH 'o' AS o; INSERT INTO o.u VALUES (5, 'z')",
        ],
    );
    assert_eq!(
        out.code,
        Some(0),
        "memory-primary attached write: {}",
        out.stderr
    );

    let o_u = run_cli(dir.path(), &["o", "SELECT * FROM u ORDER BY id"]);
    assert_rows(
        &o_u,
        r#""rows":[[1,"x"],[5,"z"],[9,"y"]]"#,
        "attached rows persist after memory session exits",
    );
}

/// R5/A5 运行形态组合：同 key 加密主库 + 加密附加库路由写入并重开
/// （错 key 附加拒绝由 attach_session_test 既有见证覆盖，不重复）。
#[tokio::test]
async fn encrypted_attached_same_key_write_and_reopen() {
    let dir = cli_fixture();
    let app_path = dir.path().join("db").join("app.db");
    let sec_path = dir.path().join("db").join("sec.db");
    {
        let app = Database::open_with_key(&app_path, IsolationLevel::RepeatableRead, Some("k1"))
            .await
            .unwrap();
        assert_ok(
            to_response(app.execute_sql("CREATE TABLE t (id INT PRIMARY KEY)").await),
            "create encrypted app t",
        );
        app.close().await.unwrap();
        let sec = Database::open_with_key(&sec_path, IsolationLevel::RepeatableRead, Some("k1"))
            .await
            .unwrap();
        assert_ok(
            to_response(
                sec.execute_sql("CREATE TABLE u (id INT PRIMARY KEY, tag VARCHAR)")
                    .await,
            ),
            "create encrypted sec u",
        );
        sec.close().await.unwrap();
    }

    let out = run_cli(
        dir.path(),
        &[
            "--key",
            "k1",
            "app",
            "ATTACH 'sec' AS s; INSERT INTO s.u VALUES (1, 'e')",
        ],
    );
    assert_eq!(
        out.code,
        Some(0),
        "encrypted attached write: {}",
        out.stderr
    );

    let reopened = run_cli(
        dir.path(),
        &["--key", "k1", "sec", "SELECT * FROM u ORDER BY id"],
    );
    assert_rows(&reopened, r#""rows":[[1,"e"]]"#, "encrypted rows reopen");
}

// ---------------------------------------------------------------------------
// R8 — UPDATE / DELETE keep the existing syntax boundary
// ---------------------------------------------------------------------------

/// R8/S9：UPDATE 子查询赋值 / DELETE 子查询条件继续按既有拒绝面拒绝，
/// 跨库路由不把子查询变成可执行计划。
#[test]
fn update_delete_subquery_shapes_still_rejected() {
    let dir = cross_db_fixture();

    let upd = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; UPDATE o.u SET tag = (SELECT tag FROM t) WHERE id = 1",
        ],
    );
    // change 2026-10-03-practical-core-gaps T8：子查询右值拒绝面由泛化
    // `Unsupported value type` 收敛为携能力名的 `UPDATE SUBQUERY`——拒绝
    // 语义（计划期、exit 3、零副作用）不变，仅文案断言随能力名校准。
    assert_failure(&upd, 3, "UPDATE SUBQUERY", "UPDATE subquery assignment");

    let del = run_cli(
        dir.path(),
        &[
            "app",
            "ATTACH 'o' AS o; DELETE FROM o.u WHERE id IN (SELECT id FROM t)",
        ],
    );
    assert_failure(&del, 3, "primary key equality", "DELETE subquery condition");

    let o_u = run_cli(dir.path(), &["o", "SELECT * FROM u ORDER BY id"]);
    assert_rows(
        &o_u,
        r#""rows":[[1,"x"],[9,"y"]]"#,
        "rejected subquery shapes leave data intact",
    );
}

// ---------------------------------------------------------------------------
// R7 — INSERT SELECT 计划面（源表注册与路由标记；执行见证见 insert_select_test）
// ---------------------------------------------------------------------------

/// T4：INSERT SELECT 的计划期源表注册——主库目标 + 附加源 → 计划为
/// InsertSelect、路由表含源限定名；附加目标同样入路由表（写面由执行根
/// 选库，design D1）。
#[tokio::test]
async fn insert_select_plan_registers_source_tables() {
    let dir = tempdir().unwrap();
    let main = Database::open(&dir.path().join("main.db")).await.unwrap();
    assert_ok(
        to_response(
            main.execute_sql("CREATE TABLE t2 (id INT PRIMARY KEY, tag VARCHAR)")
                .await,
        ),
        "create main t2",
    );
    let other = Arc::new(Database::open(&dir.path().join("o.db")).await.unwrap());
    assert_ok(
        to_response(
            other
                .execute_sql("CREATE TABLE u (id INT, tag VARCHAR)")
                .await,
        ),
        "create attached u",
    );
    let mut dbs = HashMap::new();
    dbs.insert("o".to_string(), other.clone());
    let route = AttachRoute::new(dbs);

    // 附加源 → 主库目标：计划 InsertSelect，路由表含**源**限定名。
    let (plan, routed) = routed_plan(&main, &route, "INSERT INTO t2 SELECT id, tag FROM o.u").await;
    match &plan {
        PhysicalPlan::InsertSelect(node) => {
            assert_eq!(node.table_name, "t2");
            assert_eq!(node.source_width, 2);
        }
        other => panic!("expected InsertSelect plan, got {other:?}"),
    }
    assert!(
        routed.contains_key("o.u"),
        "source qualified name must be routed, routed keys: {:?}",
        routed.keys().collect::<Vec<_>>()
    );

    // 附加目标 + 主库源：目标限定名同样入路由表。
    let (_plan, routed) =
        routed_plan(&main, &route, "INSERT INTO o.u SELECT id, tag FROM t2").await;
    assert!(
        routed.contains_key("o.u"),
        "target qualified name must be routed, routed keys: {:?}",
        routed.keys().collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// helpers (lib routed pipeline)
// ---------------------------------------------------------------------------

fn assert_ok(resp: Response, what: &str) {
    assert!(
        !matches!(resp, Response::Error { .. }),
        "{what} failed: {resp:?}"
    );
}

async fn routed_exec(main: &Database, route: &AttachRoute, sql: &str) -> Response {
    let stmts = parse_sql(sql).expect("parse routed sql");
    let stmt = stmts.first().expect("one statement");
    let mut routed = RoutedTables::new();
    let plan = plan_stage_routed(main, sql, stmt, false, Some(route), &mut routed)
        .await
        .unwrap_or_else(|e| panic!("plan {sql}: {e}"));
    to_response(execute_stage_routed(main, plan, false, &routed).await)
}

/// 计划期路由判定（不执行）：返回计划与路由表，供需要显式选择执行入口
/// （显式事务 / 只读叶子）断言 SQLSTATE 的用例消费。
async fn routed_plan(
    main: &Database,
    route: &AttachRoute,
    sql: &str,
) -> (PhysicalPlan, RoutedTables) {
    let stmts = parse_sql(sql).expect("parse routed sql");
    let stmt = stmts.first().expect("one statement");
    let mut routed = RoutedTables::new();
    let plan = plan_stage_routed(main, sql, stmt, false, Some(route), &mut routed)
        .await
        .unwrap_or_else(|e| panic!("plan {sql}: {e}"));
    (plan, routed)
}

async fn routed_ok(main: &Database, route: &AttachRoute, sql: &str) {
    let resp = routed_exec(main, route, sql).await;
    assert!(
        matches!(resp, Response::AffectedRows { .. }),
        "routed {sql} must affect rows: {resp:?}"
    );
}

async fn query(db: &Database, sql: &str) -> Vec<Vec<serde_json::Value>> {
    match to_response(db.execute_sql(sql).await) {
        Response::QueryResult { rows } => rows,
        other => panic!("{sql}: expected QueryResult, got {other:?}"),
    }
}
