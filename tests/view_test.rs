//! Integration tests for MS21 Iteration 005 (T5-a…d): CREATE/DROP VIEW
//! （delta spec `sql-views` R1–R3，design D12 创建期验证 + CTE 通道展开、
//! D13 事务边界、D14 错误面、D15 dump 不携带视图）。
//!
//! Scenario mapping (requirement → scenario → test):
//!   R1-S1 创建与查询视图                    create_and_query_view
//!   R1-S2 显式列名清单                      explicit_column_list
//!   R1-S3 跨重启持久                        view_persists_across_reopen
//!   R1-S4 嵌套视图展开                      nested_view_expansion
//!   R1-S5 循环引用拒绝                      cycle_reference_rejected
//!   R1-S6 定义错误与重名拒绝                definition_error_and_duplicate_rejected
//!   R1-S7 OR REPLACE / MATERIALIZED / 附加库引用拒绝
//!                                    modifier_rejections + view_definition_attach_reference_rejected
//!   R2-S1 DML 穿透视图拒绝                  dml_through_view_rejected (T5-c)
//!   R2-S2 DROP VIEW 删除与不存在拒绝        drop_view_deletes_and_missing_rejected (T5-d)
//!   R2-S3 视图非基表目标拒绝                view_not_base_table_target (T5-d)
//!   R3-S1 定义变更后解析一致                definition_change_resolves_consistently (T5-d)
//!   R3-S2 组合面零特殊化                    view_combination_surface (T5-c/d)
//!   R3-S3 既有面零回归                      全量矩阵（各 GREEN condition）

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::{tempdir, TempDir};

fn db_path(dir: &TempDir) -> PathBuf {
    dir.path().join("test")
}

async fn open_db() -> (Database, TempDir) {
    let dir = tempdir().unwrap();
    let db = Database::open(&db_path(&dir)).await.unwrap();
    (db, dir)
}

async fn exec_ok(db: &Database, sql: &str, what: &str) {
    let resp = to_response(db.execute_sql(sql).await);
    assert!(
        !matches!(resp, Response::Error { .. }),
        "{what} failed: {sql:?} -> {resp:?}"
    );
}

async fn count_rows(db: &Database, sql: &str, what: &str) -> Vec<Vec<serde_json::Value>> {
    match to_response(db.execute_sql(sql).await) {
        Response::QueryResult { rows } => rows,
        other => panic!("{what}: expected QueryResult, got {other:?}"),
    }
}

fn expect_error(resp: Response, what: &str, needle: &str) {
    match resp {
        Response::Error { message, .. } => assert!(
            message.contains(needle),
            "{what}: error must name {needle:?}, got: {message}"
        ),
        other => panic!("{what}: expected Error naming {needle:?}, got {other:?}"),
    }
}

/// 种子：`t(id INT PRIMARY KEY, v INT)` 三行 (1,10) (2,200) (3,300)。
async fn seed_table(db: &Database) {
    exec_ok(db, "CREATE TABLE t (id INT PRIMARY KEY, v INT)", "create t").await;
    for (id, v) in [(1, 10), (2, 200), (3, 300)] {
        exec_ok(db, &format!("INSERT INTO t VALUES ({id}, {v})"), "insert").await;
    }
}

// ===========================================================================
// R1-S1: 创建与查询视图
// ===========================================================================

/// R1-S1：创建视图后 `SELECT * FROM big` 返回过滤行，列名 `id, v`，结果
/// 与内联派生表查询一致。
#[tokio::test]
async fn create_and_query_view() {
    let (db, _dir) = open_db().await;
    seed_table(&db).await;

    exec_ok(
        &db,
        "CREATE VIEW big AS SELECT id, v FROM t WHERE v > 100",
        "create view",
    )
    .await;

    let rows = count_rows(&db, "SELECT * FROM big", "query view").await;
    assert_eq!(rows.len(), 2, "view must return the v > 100 rows");
    assert_eq!(rows[0][0].as_i64().unwrap(), 2);
    assert_eq!(rows[1][0].as_i64().unwrap(), 3);

    // 与内联派生表逐字节一致
    let inlined = count_rows(
        &db,
        "SELECT * FROM (SELECT id, v FROM t WHERE v > 100) AS big",
        "inline",
    )
    .await;
    let via_view = count_rows(&db, "SELECT * FROM big", "view").await;
    assert_eq!(
        inlined, via_view,
        "expansion must equal inline derived table"
    );
}

// ===========================================================================
// R1-S2: 显式列名清单
// ===========================================================================

/// R1-S2：显式列清单覆盖输出列名（行数据仍为 id, v）。
#[tokio::test]
async fn explicit_column_list() {
    let (db, _dir) = open_db().await;
    seed_table(&db).await;

    exec_ok(
        &db,
        "CREATE VIEW vv (a, b) AS SELECT id, v FROM t",
        "create",
    )
    .await;
    let rows = count_rows(&db, "SELECT a, b FROM vv", "query renamed").await;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0][0].as_i64().unwrap(), 1, "row data is id");
    assert_eq!(rows[0][1].as_i64().unwrap(), 10, "row data is v");
}

/// 每个测试独立的仓库现场：TempDir + 预建 `db/` 集中存储目录
/// （夹具模式与 cross_db_query_test 一致）。
fn cli_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join("db")).unwrap();
    dir
}

/// R1-S1/S2（CLI e2e）：表头形态——缺省取定义输出表头（id, v）；显式列
/// 清单覆盖为 (a, b)。lib `QueryResult` 不携带表头，经 CLI table 渲染断言。
#[test]
fn view_header_shape() {
    let dir = cli_fixture();
    // T10 机械校准：主命令不再隐式建库，先经 new 通道创建（断言语义不变）。
    let new_out = run_cli(dir.path(), &["new", "main"]);
    assert_eq!(new_out.code, Some(0), "new main failed: {}", new_out.stderr);
    let seed = run_cli(
        dir.path(),
        &[
            "main",
            "CREATE TABLE t (id INT PRIMARY KEY, v INT); INSERT INTO t VALUES (1, 10)",
        ],
    );
    assert_eq!(seed.code, Some(0), "seed failed: {}", seed.stderr);

    let out = run_cli(
        dir.path(),
        &[
            "main",
            "CREATE VIEW big AS SELECT id, v FROM t; CREATE VIEW vv (a, b) AS SELECT id, v FROM t",
        ],
    );
    assert_eq!(out.code, Some(0), "create views failed: {}", out.stderr);

    let q1 = run_cli(
        dir.path(),
        &["main", "--format", "table", "SELECT * FROM big"],
    );
    assert_eq!(q1.code, Some(0), "query big failed: {}", q1.stderr);
    let header = q1.stdout.lines().next().unwrap_or("");
    assert!(
        header.contains("id") && header.contains("v"),
        "default header must be the definition output columns, got: {q1:?}"
    );

    let q2 = run_cli(
        dir.path(),
        &["main", "--format", "table", "SELECT a, b FROM vv"],
    );
    assert_eq!(q2.code, Some(0), "query vv failed: {}", q2.stderr);
    let header = q2.stdout.lines().next().unwrap_or("");
    assert!(
        header.contains("a") && header.contains("b"),
        "explicit list must rename output columns, got: {q2:?}"
    );
}

// ===========================================================================
// R1-S3: 跨重启持久
// ===========================================================================

/// R1-S3：创建视图后显式 close，重开数据库视图可查，定义与结果一致。
#[tokio::test]
async fn view_persists_across_reopen() {
    let dir = tempdir().unwrap();
    {
        let db = Database::open(&db_path(&dir)).await.unwrap();
        seed_table(&db).await;
        exec_ok(
            &db,
            "CREATE VIEW big AS SELECT id, v FROM t WHERE v > 100",
            "create view",
        )
        .await;
        db.close().await.unwrap();
    }
    {
        let db = Database::open(&db_path(&dir)).await.unwrap();
        let rows = count_rows(&db, "SELECT * FROM big", "query after reopen").await;
        assert_eq!(rows.len(), 2, "view must survive reopen");
        let n = count_rows(&db, "SELECT COUNT(*) FROM big", "count").await;
        assert_eq!(n[0][0].as_i64().unwrap(), 2);
    }
}

// ===========================================================================
// R1-S4: 嵌套视图展开
// ===========================================================================

/// R1-S4：视图引用既有视图（嵌套展开），结果与两层内联展开一致。
#[tokio::test]
async fn nested_view_expansion() {
    let (db, _dir) = open_db().await;
    seed_table(&db).await;

    exec_ok(
        &db,
        "CREATE VIEW big AS SELECT id, v FROM t WHERE v > 100",
        "v1",
    )
    .await;
    exec_ok(
        &db,
        "CREATE VIEW big2 AS SELECT id FROM big WHERE id > 2",
        "v2",
    )
    .await;

    let rows = count_rows(&db, "SELECT * FROM big2", "nested query").await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0].as_i64().unwrap(), 3);

    let inlined = count_rows(
        &db,
        "SELECT * FROM (SELECT id FROM (SELECT id, v FROM t WHERE v > 100) AS big WHERE id > 2) AS big2",
        "inline nested",
    )
    .await;
    assert_eq!(inlined, rows, "nested expansion must equal inline form");
}

// ===========================================================================
// R1-S5: 循环引用拒绝
// ===========================================================================

/// R1-S5：创建期引用尚不存在的视图 → 计划期点名拒绝、零副作用（循环
/// 结构性不可能——被引用方必须已存在且已通过同一验证；无 OR REPLACE，
/// 已创建视图的定义不可变更，链无法回指）。
#[tokio::test]
async fn cycle_reference_rejected() {
    let (db, _dir) = open_db().await;
    seed_table(&db).await;

    // v1 引用尚不存在的 v2 → 拒绝（42P01：引用对象不存在）
    let resp = to_response(db.execute_sql("CREATE VIEW v1 AS SELECT * FROM v2").await);
    expect_error(resp, "self/forward reference", "not found");

    // 零副作用：v1 未被创建（注册表与 catalog 均无 v1）
    assert!(
        db.views.read().await.get("v1").is_none(),
        "rejected view must not be registered"
    );
    let resp = to_response(db.execute_sql("SELECT * FROM v1").await);
    expect_error(resp, "v1", "not found");
}

// ===========================================================================
// R1-S6: 定义错误与重名拒绝
// ===========================================================================

/// R1-S6：定义引用不存在的表/列、对既有视图名或表名重复创建 → 全部点名
/// 拒绝，既有对象零变化。
#[tokio::test]
async fn definition_error_and_duplicate_rejected() {
    let (db, _dir) = open_db().await;
    seed_table(&db).await;
    exec_ok(&db, "CREATE VIEW big AS SELECT id, v FROM t", "seed view").await;

    // 定义引用不存在的表
    let resp = to_response(
        db.execute_sql("CREATE VIEW bad AS SELECT * FROM nope")
            .await,
    );
    expect_error(resp, "missing table", "not found");
    // 定义引用不存在的列
    let resp = to_response(
        db.execute_sql("CREATE VIEW bad AS SELECT missing FROM t")
            .await,
    );
    expect_error(resp, "missing column", "not found");

    // 与既有视图重名
    let resp = to_response(db.execute_sql("CREATE VIEW big AS SELECT id FROM t").await);
    expect_error(resp, "duplicate view", "already exists");
    // 与既有基表重名
    let resp = to_response(
        db.execute_sql("CREATE VIEW t AS SELECT 1 AS x FROM t WHERE id = 1")
            .await,
    );
    expect_error(resp, "duplicate table name", "already exists");

    // 既有对象零变化：big 定义未变（仍为 v 全集 3 行）、t 未受影响
    let rows = count_rows(&db, "SELECT COUNT(*) FROM big", "big intact").await;
    assert_eq!(
        rows[0][0].as_i64().unwrap(),
        3,
        "existing view definition unchanged"
    );
    let rows = count_rows(&db, "SELECT COUNT(*) FROM t", "t intact").await;
    assert_eq!(rows[0][0].as_i64().unwrap(), 3, "existing table unchanged");
}

// ===========================================================================
// R1-S7: OR REPLACE / MATERIALIZED / 附加库引用拒绝
// ===========================================================================

/// R1-S7（lib 可达面）：OR REPLACE / MATERIALIZED / IF NOT EXISTS / TEMP /
/// 列选项 / 超长定义 → 计划期点名拒绝、零副作用。
#[tokio::test]
async fn modifier_rejections() {
    let (db, _dir) = open_db().await;
    seed_table(&db).await;

    for (sql, capability) in [
        (
            "CREATE OR REPLACE VIEW big AS SELECT id FROM t",
            "CREATE OR REPLACE VIEW",
        ),
        (
            "CREATE MATERIALIZED VIEW m AS SELECT id FROM t",
            "MATERIALIZED VIEW",
        ),
        (
            "CREATE VIEW IF NOT EXISTS i AS SELECT id FROM t",
            "CREATE VIEW IF NOT EXISTS",
        ),
        (
            "CREATE TEMPORARY VIEW tmp AS SELECT id FROM t",
            "TEMPORARY VIEW",
        ),
    ] {
        let resp = to_response(db.execute_sql(sql).await);
        expect_error(resp, capability, capability);
        assert!(
            db.views.read().await.is_empty(),
            "rejected creation must leave registry empty: {sql}"
        );
    }

    // 列选项（`vv (a INT)` 形态）：sqlparser parse 层即拒绝（ViewColumnDef
    // 无类型字段），planner 的 VIEW COLUMN OPTIONS 臂为直呼构造的防御臂，
    // SQL 可达面由 parse 错误覆盖——此处不再重复断言。

    // 超长定义前置点名拒绝、零副作用
    let padding = "x".repeat(5000);
    let resp = to_response(
        db.execute_sql(&format!(
            "CREATE VIEW huge AS SELECT '{padding}' AS p FROM t WHERE id = 1"
        ))
        .await,
    );
    expect_error(resp, "oversized definition", "too long");
    assert!(
        db.views.read().await.is_empty(),
        "oversized must not persist"
    );
}

#[derive(Debug)]
struct CliOutput {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// 运行 rtsql 二进制并等待退出（夹具模式与 cross_db_query_test 一致）。
fn run_cli(dir: &Path, args: &[&str]) -> CliOutput {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .current_dir(dir)
        .env("RTSQL_HOME", dir)
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
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                panic!("rtsql binary timed out");
            }
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
}

/// R1-S7（CLI e2e）：已 ATTACH 会话中，定义引用附加库（`o.t`）→ 创建期
/// 点名拒绝、零副作用（可持久对象不得携带会话限定名）。
#[test]
fn view_definition_attach_reference_rejected() {
    let dir = cli_fixture();
    // T10 机械校准：主命令不再隐式建库，先经 new 通道创建（断言语义不变）。
    let new_out = run_cli(dir.path(), &["new", "main"]);
    assert_eq!(new_out.code, Some(0), "new main failed: {}", new_out.stderr);
    let seed = run_cli(
        dir.path(),
        &[
            "main",
            "CREATE TABLE t (id INT PRIMARY KEY); INSERT INTO t VALUES (1)",
        ],
    );
    assert_eq!(seed.code, Some(0), "seed failed: {}", seed.stderr);

    let out = run_cli(
        dir.path(),
        &[
            "main",
            "ATTACH 'other.db' AS o; CREATE VIEW v AS SELECT id FROM o.t",
        ],
    );
    assert_ne!(
        out.code,
        Some(0),
        "attach-referencing view must be rejected"
    );
    assert!(
        out.stderr.contains("attached") || out.stderr.contains("ATTACH"),
        "error must name the attached-reference capability, got: {}",
        out.stderr
    );

    // 零副作用：重开会话视图不存在
    let check = run_cli(dir.path(), &["main", "SELECT * FROM v"]);
    assert_ne!(check.code, Some(0), "view must not have been created");
}

// ===========================================================================
// R2-S1: DML 穿透视图拒绝（T5-c 见证）
// ===========================================================================

/// R2-S1：INSERT/UPDATE/DELETE 目标为视图 → 计划期点名拒绝（视图只读），
/// 基表零变化。
#[tokio::test]
async fn dml_through_view_rejected() {
    let (db, _dir) = open_db().await;
    seed_table(&db).await;
    exec_ok(
        &db,
        "CREATE VIEW big AS SELECT id, v FROM t WHERE v > 100",
        "create",
    )
    .await;

    let resp = to_response(db.execute_sql("INSERT INTO big VALUES (9, 90)").await);
    expect_error(resp, "insert into view", "read-only");
    let resp = to_response(db.execute_sql("UPDATE big SET v = 999 WHERE id = 2").await);
    expect_error(resp, "update view", "read-only");
    let resp = to_response(db.execute_sql("DELETE FROM big WHERE id = 2").await);
    expect_error(resp, "delete from view", "read-only");

    // 基表零变化
    let rows = count_rows(&db, "SELECT COUNT(*) FROM t", "count").await;
    assert_eq!(rows[0][0].as_i64().unwrap(), 3, "base table untouched");
    let rows = count_rows(&db, "SELECT id FROM t WHERE id = 9", "no insert").await;
    assert!(rows.is_empty(), "no row inserted through view");
}

/// R1/T5-c：IN 子查询引用视图可达（DerivedScan 基座首列名臂）。
#[tokio::test]
async fn in_subquery_with_view() {
    let (db, _dir) = open_db().await;
    seed_table(&db).await;
    exec_ok(
        &db,
        "CREATE VIEW big AS SELECT id, v FROM t WHERE v > 100",
        "create",
    )
    .await;

    let rows = count_rows(
        &db,
        "SELECT id FROM t WHERE id IN (SELECT id FROM big)",
        "in subquery",
    )
    .await;
    assert_eq!(rows.len(), 2, "IN subquery over view must reach the rows");
    let ids: Vec<i64> = rows.iter().map(|r| r[0].as_i64().unwrap()).collect();
    assert_eq!(ids, vec![2, 3]);
}

// ===========================================================================
// R2-S2: DROP VIEW 删除与不存在拒绝
// ===========================================================================

/// R2-S2：DROP VIEW 删除定义（重开持久）；此后查询点名拒绝、基表不受
/// 影响；再删/删不存在点名拒绝；IF EXISTS 点名拒绝。
#[tokio::test]
async fn drop_view_deletes_and_missing_rejected() {
    let dir = tempdir().unwrap();
    {
        let db = Database::open(&db_path(&dir)).await.unwrap();
        seed_table(&db).await;
        exec_ok(
            &db,
            "CREATE VIEW big AS SELECT id, v FROM t WHERE v > 100",
            "create",
        )
        .await;

        exec_ok(&db, "DROP VIEW big", "drop view").await;
        let resp = to_response(db.execute_sql("SELECT * FROM big").await);
        expect_error(resp, "query dropped view", "not found");
        // 基表数据不受影响
        let rows = count_rows(&db, "SELECT COUNT(*) FROM t", "count").await;
        assert_eq!(rows[0][0].as_i64().unwrap(), 3);

        // 再删 / 删不存在 → 点名拒绝
        let resp = to_response(db.execute_sql("DROP VIEW big").await);
        expect_error(resp, "re-drop", "not found");
        let resp = to_response(db.execute_sql("DROP VIEW nope").await);
        expect_error(resp, "drop missing", "not found");
        // IF EXISTS 点名拒绝（Forbidden）
        let resp = to_response(db.execute_sql("DROP VIEW IF EXISTS big").await);
        expect_error(resp, "if exists", "IF EXISTS");

        db.close().await.unwrap();
    }
    {
        // 删除跨重启持久：重开后仍不存在
        let db = Database::open(&db_path(&dir)).await.unwrap();
        let resp = to_response(db.execute_sql("SELECT * FROM big").await);
        expect_error(resp, "dropped view stays dropped", "not found");
    }
}

// ===========================================================================
// R2-S3: 视图非基表目标拒绝
// ===========================================================================

/// R2-S3：TRUNCATE / CREATE INDEX / ALTER / DROP TABLE 目标为视图名 →
/// 点名拒绝（目标非基表 / 表不存在）。
#[tokio::test]
async fn view_not_base_table_target() {
    let (db, _dir) = open_db().await;
    seed_table(&db).await;
    exec_ok(
        &db,
        "CREATE VIEW big AS SELECT id, v FROM t WHERE v > 100",
        "create",
    )
    .await;

    let resp = to_response(db.execute_sql("TRUNCATE TABLE big").await);
    expect_error(resp, "truncate view", "not found");
    let resp = to_response(db.execute_sql("CREATE INDEX idx_big ON big(id)").await);
    expect_error(resp, "index on view", "not found");
    let resp = to_response(db.execute_sql("ALTER TABLE big ADD COLUMN w INT").await);
    expect_error(resp, "alter view", "not found");
    let resp = to_response(db.execute_sql("DROP TABLE big").await);
    expect_error(resp, "drop table on view", "not found");

    // 上述目标拒绝后视图仍可查、基表完好
    let rows = count_rows(&db, "SELECT COUNT(*) FROM big", "view intact").await;
    assert_eq!(rows[0][0].as_i64().unwrap(), 2);
}

// ===========================================================================
// R3-S1: 定义变更后解析一致
// ===========================================================================

/// R3-S1：同会话 DROP 后重建（不同定义）→ 按新定义展开；DROP 后未重建
/// 期间查询点名拒绝（旧计划不复活——plan cache 全清）。
#[tokio::test]
async fn definition_change_resolves_consistently() {
    let (db, _dir) = open_db().await;
    seed_table(&db).await;
    exec_ok(
        &db,
        "CREATE VIEW big AS SELECT id, v FROM t WHERE v > 100",
        "v1",
    )
    .await;

    // 首查建立缓存
    let rows = count_rows(&db, "SELECT * FROM big", "q1").await;
    assert_eq!(rows.len(), 2);

    exec_ok(&db, "DROP VIEW big", "drop").await;
    // 未重建期间查询点名拒绝（不得复用旧展开结果）
    let resp = to_response(db.execute_sql("SELECT * FROM big").await);
    expect_error(resp, "dropped between", "not found");

    exec_ok(
        &db,
        "CREATE VIEW big AS SELECT id, v FROM t WHERE v > 200",
        "v2",
    )
    .await;
    let rows = count_rows(&db, "SELECT * FROM big", "q2").await;
    assert_eq!(rows.len(), 1, "v > 200 matches only id=3 (v=300)");
    // 新定义的行为边界：新增 id=4 (v=150) 不在新定义（旧定义 v > 100 会包含）
    exec_ok(&db, "INSERT INTO t VALUES (4, 150)", "insert").await;
    let rows = count_rows(&db, "SELECT * FROM big", "q3").await;
    assert_eq!(rows.len(), 1, "new definition (v > 200) must exclude v=150");
}

// ===========================================================================
// R3-S2: 组合面零特殊化
// ===========================================================================

/// R3-S2：视图与基表 JOIN + WHERE + ORDER BY、子查询引用视图——行为与
/// 等价内联派生表查询一致（结果与错误面）。
#[tokio::test]
async fn view_combination_surface() {
    let (db, _dir) = open_db().await;
    seed_table(&db).await;
    exec_ok(
        &db,
        "CREATE TABLE u (k INT PRIMARY KEY, tag VARCHAR(10))",
        "create u",
    )
    .await;
    for (k, tag) in [(1, "x"), (2, "y"), (3, "z")] {
        exec_ok(
            &db,
            &format!("INSERT INTO u VALUES ({k}, '{tag}')"),
            "insert u",
        )
        .await;
    }
    exec_ok(
        &db,
        "CREATE VIEW big AS SELECT id, v FROM t WHERE v > 100",
        "create view",
    )
    .await;

    // JOIN + WHERE + ORDER BY（视图在左；ORDER BY 用裸列名——限定名排序键
    // 是既有引擎拒绝面，内联形态同款，等价比较不受影响）
    let via_view = count_rows(
        &db,
        "SELECT big.id, u.tag FROM big JOIN u ON big.id = u.k WHERE u.tag != 'y' ORDER BY id",
        "join via view",
    )
    .await;
    let inlined = count_rows(
        &db,
        "SELECT b.id, u.tag FROM (SELECT id, v FROM t WHERE v > 100) AS b JOIN u ON b.id = u.k WHERE u.tag != 'y' ORDER BY id",
        "join inline",
    )
    .await;
    assert_eq!(via_view, inlined, "JOIN combination must equal inline form");
    assert_eq!(via_view.len(), 1, "tag != 'y' leaves only (3, z)");
    assert_eq!(via_view[0][0].as_i64().unwrap(), 3);

    // 视图在 JOIN 右侧（内联对照不可表达——JOIN 右臂对 TableFactor::Derived
    // 是既有拒绝面；视图引用在语法上是普通表名，按语义结果直接断言）
    let via_view = count_rows(
        &db,
        "SELECT u.k, big.v FROM u JOIN big ON u.k = big.id ORDER BY k",
        "join right view",
    )
    .await;
    assert_eq!(via_view.len(), 2, "rows (2, 200) and (3, 300)");
    assert_eq!(via_view[0][0].as_i64().unwrap(), 2);
    assert_eq!(via_view[0][1].as_i64().unwrap(), 200);

    // IN 子查询引用视图（IN 与其他谓词的 AND 组合是既有拒绝面——基表同款
    // 报错，视图零特殊化继承之，不入正断言）
    let rows = count_rows(
        &db,
        "SELECT tag FROM u WHERE k IN (SELECT id FROM big)",
        "in subquery via view",
    )
    .await;
    assert_eq!(rows.len(), 2, "u rows with k in (2, 3)");

    // 聚合 over 视图
    let rows = count_rows(&db, "SELECT COUNT(*), SUM(v) FROM big", "aggregate").await;
    assert_eq!(rows[0][0].as_i64().unwrap(), 2);
    assert_eq!(rows[0][1].as_i64().unwrap(), 500);

    // 嵌套视图 + WHERE + ORDER BY（R1-S4 组合扩展）
    exec_ok(
        &db,
        "CREATE VIEW big2 AS SELECT id FROM big WHERE id > 2",
        "create nested",
    )
    .await;
    let rows = count_rows(
        &db,
        "SELECT id FROM big2 WHERE id > 1 ORDER BY id DESC",
        "nested + where + order",
    )
    .await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0].as_i64().unwrap(), 3);
}

// ===========================================================================
// R4（change 2026-10-13-e2e-verification-and-defect-closure T4）：视图内层
// 输出列外层可见——delta spec `join-output-shape-resolution` ADDED R4 +
// `sql-views` MODIFIED「视图解析与计划一致性」。
//
// 病灶（pre-fix 实测）：视图展开把 `building_from_derived` 置假，定义内的
// `col AS alias`、表达式列与 DISTINCT 走表达式子查询抑制——外层按别名引用
// 收 42703，DISTINCT 静默跳过；与 CTE/派生表通道「逐字节一致」声明矛盾。
// 修复：视图展开与 FROM 派生同型置位，分类门只看
// `building_subquery && !building_from_derived`。
// ===========================================================================

/// R4-S1：视图定义含别名列可查询（修复前 42703 `Column not found`）。
#[tokio::test]
async fn r4_view_inner_alias_outer_visible() {
    let (db, _dir) = open_db().await;
    seed_table(&db).await;
    exec_ok(&db, "CREATE VIEW va AS SELECT v AS val FROM t", "create va").await;

    let rows = count_rows(&db, "SELECT val FROM va ORDER BY val", "query va").await;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0][0].as_i64().unwrap(), 10);
    assert_eq!(rows[1][0].as_i64().unwrap(), 200);
    assert_eq!(rows[2][0].as_i64().unwrap(), 300);
}

/// R4-S2：视图定义含表达式（计算）列可查询。
#[tokio::test]
async fn r4_view_inner_expression_outer_visible() {
    let (db, _dir) = open_db().await;
    seed_table(&db).await;
    exec_ok(
        &db,
        "CREATE VIEW ve AS SELECT v + 1 AS inc FROM t",
        "create ve",
    )
    .await;

    let rows = count_rows(&db, "SELECT inc FROM ve ORDER BY inc", "query ve").await;
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0][0].as_i64().unwrap(), 11);
    assert_eq!(rows[1][0].as_i64().unwrap(), 201);
    assert_eq!(rows[2][0].as_i64().unwrap(), 301);
}

/// R4-S3：视图定义含 DISTINCT 真实去重（修复前静默跳过）。
#[tokio::test]
async fn r4_view_distinct_dedups() {
    let (db, _dir) = open_db().await;
    exec_ok(&db, "CREATE TABLE t2 (k INT, v INT)", "create t2").await;
    for (k, v) in [(1, 5), (2, 7), (3, 5)] {
        exec_ok(
            &db,
            &format!("INSERT INTO t2 VALUES ({k}, {v})"),
            "insert t2",
        )
        .await;
    }
    exec_ok(
        &db,
        "CREATE VIEW vd AS SELECT DISTINCT v FROM t2",
        "create vd",
    )
    .await;

    let rows = count_rows(&db, "SELECT v FROM vd ORDER BY v", "query vd").await;
    assert_eq!(rows.len(), 2, "DISTINCT view must dedup");
    assert_eq!(rows[0][0].as_i64().unwrap(), 5);
    assert_eq!(rows[1][0].as_i64().unwrap(), 7);
}

/// R4-S4：嵌套视图（视图定义引用含别名列的视图）零回归可达。
#[tokio::test]
async fn r4_nested_view_over_alias_view() {
    let (db, _dir) = open_db().await;
    seed_table(&db).await;
    exec_ok(&db, "CREATE VIEW va AS SELECT v AS val FROM t", "create va").await;
    exec_ok(
        &db,
        "CREATE VIEW vb AS SELECT val FROM va WHERE val > 100",
        "create vb",
    )
    .await;

    let rows = count_rows(&db, "SELECT val FROM vb ORDER BY val", "query vb").await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0][0].as_i64().unwrap(), 200);
    assert_eq!(rows[1][0].as_i64().unwrap(), 300);
}
