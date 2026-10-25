//! MS36-T01（change `2026-10-24-ms36-legacy-format-and-dump-compat`，
//! Iteration 000 / Cycle 000）：旧代际库文件与旧 dump 文本的兼容支持范围，
//! 由仓库内的**固定字节样本**锁定。
//!
//! 样本来源：更早版本的产品构建（revision `d8165fc`——MS23 落库前的最后
//! 一次提交）产出的真实字节，作为固定输入提交在 `tests/fixtures/legacy/`。
//! 旧构建命令、样本清单与再生步骤见 `tests/fixtures/legacy/README.md`。
//! 运行期不做任何版本/哈希/修订比对——样本只是固定输入。
//!
//! 本文件另含 **G2 代际**样本（`legacy_g2.db`，revision `6e8dcf5` 的旧构建
//! 产出：目录行含唯一根段、不含二级索引段与表类型字节）的文件级断言，以及
//! 迁移路径实测记录里三条一次性观察的常态化用例。
//!
//! 断言面只用既有可观察面：
//! - 目录行原始字节（旧代际尾段缺失 / 当前布局尾段）；
//! - 行集与行数；
//! - 错误文本的关键片段与语句序号、失败退出码；
//! - 完整性扫描的 error 级发现数；
//! - 库文件大小（写入是否分配了新页的可观察证据）。
//!
//! 全部用例走真实入口：真实库文件（`Database::open`）与真实 CLI
//! （`CARGO_BIN_EXE_rtsql` 子进程——restore/dump 的既有合同与退出码语义
//! 只在 CLI 面上可观察）。
//!
//! 本文件是**刻画断言**（characterization）：断言的是当前已成立的既有行为，
//! 首次运行即 GREEN；产品代码零改动。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use rtsql::database::Database;
use rtsql::result::{value_to_json, SqlOutcome};
use rtsql::storage::integrity::check_integrity;
use rtsql::storage::page_format::SlottedPageRef;
use rtsql::storage::{
    AsyncStorage, ColumnType, FileStorage, PageId, COLUMNS_PAGE_ID, TABLES_PAGE_ID,
};
use tempfile::tempdir;

// ---------------------------------------------------------------------------
// 夹具与工具
// ---------------------------------------------------------------------------

/// 仓库内固定样本的路径。
fn fixture_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("legacy")
        .join(name)
}

/// 把固定样本复制到临时目录（样本本身只读，运行期一律操作副本）。
fn copy_fixture(dir: &Path, name: &str) -> PathBuf {
    let src = fixture_path(name);
    assert!(
        src.exists(),
        "fixture missing: {} (run the reproduction steps in tests/fixtures/legacy/README.md)",
        src.display()
    );
    let dst = dir.join(name);
    std::fs::copy(&src, &dst).expect("copy fixture into temp dir");
    dst
}

/// CLI 子进程的输出（真实入口的退出码与用户实际所见文本）。
struct CliOut {
    code: i32,
    stdout: String,
    stderr: String,
}

/// 运行真实 CLI。`RTSQL_KEY` 被移除，避免环境里的密钥影响开库形态。
fn run_cli(args: &[&str]) -> CliOut {
    let out = Command::new(env!("CARGO_BIN_EXE_rtsql"))
        .args(args)
        .env_remove("RTSQL_KEY")
        .output()
        .expect("spawn rtsql");
    CliOut {
        code: out.status.code().expect("rtsql exit code"),
        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
    }
}

/// CLI 错误输出（JSON 形态）中的消息文本。
fn cli_error_message(stderr: &str) -> String {
    serde_json::from_str::<serde_json::Value>(stderr.trim())
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.as_str().map(|s| s.to_string()))
        })
        .unwrap_or_else(|| stderr.to_string())
}

/// CLI JSON 输出中的行集。
fn cli_rows(stdout: &str) -> Vec<Vec<serde_json::Value>> {
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("JSON stdout");
    v["rows"]
        .as_array()
        .expect("rows array")
        .iter()
        .map(|r| r.as_array().expect("row array").clone())
        .collect()
}

/// 库 API 的行集（原生 `Value` 经产品自带的 JSON 渲染）。
fn db_rows(outcome: SqlOutcome) -> Vec<Vec<serde_json::Value>> {
    match outcome {
        SqlOutcome::Rows { rows, .. } => rows
            .into_iter()
            .map(|row| row.into_iter().map(value_to_json).collect())
            .collect(),
        SqlOutcome::AffectedRows(n) => panic!("expected a row set, got {n} affected rows"),
    }
}

/// 读取一条目录链上的全部**存活**行（原始字节 + logical_id）。
///
/// 删除的槽保留长度为 0——按长度过滤即排除死槽残留。
async fn read_chain_rows(path: &Path, root: u64) -> Vec<(u16, Vec<u8>)> {
    let storage = FileStorage::open(path).expect("open data file");
    let mut out: Vec<(u16, Vec<u8>)> = Vec::new();
    let mut current = root;
    let mut visited: HashSet<u64> = HashSet::new();
    while visited.insert(current) {
        let page = storage.read_page(PageId(current)).await.expect("read page");
        let slotted = SlottedPageRef::new(&page.data[..]);
        for i in 0..slotted.slot_count() {
            let Some(slot) = slotted.get_slot(i) else {
                continue;
            };
            if slot.length == 0 {
                continue;
            }
            out.push((slot.logical_id, slotted.get_slot_data(&slot).to_vec()));
        }
        let next = slotted.header().next_page_id;
        if next == 0 {
            break;
        }
        current = next as u64;
    }
    out
}

/// `__tables` 行的表名（行首 `u16 长度 | 名字`）。
fn tables_row_name(row: &[u8]) -> Option<String> {
    if row.len() < 2 {
        return None;
    }
    let name_len = u16::from_le_bytes([row[0], row[1]]) as usize;
    if row.len() < 2 + name_len {
        return None;
    }
    std::str::from_utf8(&row[2..2 + name_len])
        .ok()
        .map(|s| s.to_string())
}

/// `__tables` 行的 G1 固定段长度（不含当前布局追加的三段尾）。
///
/// G1 布局：`u16 名字长 | 名字 | u32 数据页头 | u32 索引根 | u32 主键索引 |
///           u16 主键列长 | 主键列 | u32 列数 | u32 数据页尾指针`。
fn g1_tables_row_len(row: &[u8]) -> usize {
    let name_len = u16::from_le_bytes([row[0], row[1]]) as usize;
    let mut p = 2 + name_len;
    p += 4 * 3; // 数据页头 | 索引根 | 主键索引
    let pk_len = u16::from_le_bytes([row[p], row[p + 1]]) as usize;
    p += 2 + pk_len; // 主键列名
    p += 4 * 2; // 列数 | 数据页尾指针
    p
}

/// 同名行折叠取最新的一份（logical_id 最大者）——与目录扫描的同名折叠规则一致。
fn find_table_row(rows: &[(u16, Vec<u8>)], table_name: &str) -> Option<Vec<u8>> {
    rows.iter()
        .filter(|(_, bytes)| tables_row_name(bytes).as_deref() == Some(table_name))
        .max_by_key(|(logical_id, _)| *logical_id)
        .map(|(_, bytes)| bytes.clone())
}

fn newest_table_row(rows: &[(u16, Vec<u8>)], table_name: &str) -> Vec<u8> {
    find_table_row(rows, table_name)
        .unwrap_or_else(|| panic!("no catalog row for table {table_name}"))
}

/// `__columns` 行的 (表名, 列名)。
fn column_row_names(row: &[u8]) -> (String, String) {
    let table_len = u16::from_le_bytes([row[0], row[1]]) as usize;
    let table = std::str::from_utf8(&row[2..2 + table_len])
        .expect("utf8 table name")
        .to_string();
    let p = 2 + table_len + 4; // 跳过列序
    let column_len = u16::from_le_bytes([row[p], row[p + 1]]) as usize;
    let column = std::str::from_utf8(&row[p + 2..p + 2 + column_len])
        .expect("utf8 column name")
        .to_string();
    (table, column)
}

/// `__columns` 行的 G1 固定段长度（不含 MS24 的 DEFAULT 段）。
///
/// G1 布局：`u16 表名长 | 表名 | u32 列序 | u16 列名长 | 列名 | u8 类型标签 |
///           [u16 串长（仅字符串列）] | u8 NOT NULL | u8 UNIQUE`。
fn g1_columns_row_len(row: &[u8], is_string: bool) -> usize {
    let table_len = u16::from_le_bytes([row[0], row[1]]) as usize;
    let mut p = 2 + table_len;
    p += 4; // 列序
    let column_len = u16::from_le_bytes([row[p], row[p + 1]]) as usize;
    p += 2 + column_len; // 列名
    p += 1; // 类型标签
    if is_string {
        p += 2; // 串长
    }
    p += 2; // NOT NULL | UNIQUE
    p
}

// ---------------------------------------------------------------------------
// R1：最旧代际库文件属于支持范围
// ---------------------------------------------------------------------------

/// 固定样本确实是旧代际（最旧目录行代际）的库文件，且当前构建能读出它的
/// 表名、列名、列类型与主键列——证明样本落在声明的支持范围内。
///
/// 静态核对：提交的样本文件本身，`__tables` 行止于数据页尾指针（无唯一根段
/// / 无二级索引段 / 无表类型字节），`__columns` 行无 DEFAULT 段。
#[tokio::test]
async fn legacy_generation_db_is_in_supported_range() {
    let dir = tempdir().unwrap();
    let db = copy_fixture(dir.path(), "legacy_g1.db");

    let table_rows = read_chain_rows(&db, TABLES_PAGE_ID).await;
    assert!(!table_rows.is_empty(), "__tables chain must not be empty");
    for name in ["users", "audit", "events"] {
        let row = newest_table_row(&table_rows, name);
        let g1_len = g1_tables_row_len(&row);
        assert_eq!(
            row.len(),
            g1_len,
            "table '{name}' catalog row must end at the legacy data-tail pointer \
             (no unique-roots / secondary-index / kind tail), got {} bytes",
            row.len()
        );
    }
    assert!(
        find_table_row(&table_rows, "discarded").is_none(),
        "the table dropped by the legacy build must be gone from the directory"
    );

    let column_rows = read_chain_rows(&db, COLUMNS_PAGE_ID).await;
    assert!(!column_rows.is_empty(), "__columns chain must not be empty");

    // 打开读目录：表名、列名、列类型、主键列可读。
    let dbh = Database::open(&db).await.expect("open legacy db");
    let mut string_columns: HashSet<(String, String)> = HashSet::new();
    for name in ["users", "audit", "events"] {
        let meta = dbh
            .get_table(name)
            .await
            .unwrap_or_else(|e| panic!("table '{name}' must be readable: {e:?}"));
        assert_eq!(meta.pk_column, "id");
        let column_names: Vec<&str> = meta.columns.iter().map(|(c, _)| c.as_str()).collect();
        match name {
            "users" => {
                assert_eq!(column_names, vec!["id", "name", "score"]);
                assert_eq!(meta.columns[0].1, ColumnType::Int);
                assert!(matches!(meta.columns[1].1, ColumnType::String(_)));
                assert_eq!(meta.columns[2].1, ColumnType::Float);
            }
            "audit" => {
                assert_eq!(column_names, vec!["id", "note"]);
                assert!(matches!(meta.columns[1].1, ColumnType::String(_)));
            }
            "events" => {
                assert_eq!(column_names, vec!["id", "day", "at", "ok"]);
                assert_eq!(meta.columns[1].1, ColumnType::Date);
                assert_eq!(meta.columns[2].1, ColumnType::Timestamp);
                assert_eq!(meta.columns[3].1, ColumnType::Bool);
            }
            _ => unreachable!(),
        }
        for (column, column_type) in &meta.columns {
            if matches!(column_type, ColumnType::String(_)) {
                string_columns.insert((name.to_string(), column.clone()));
            }
        }
    }
    dbh.close().await.expect("close legacy db");
    drop(dbh);

    // `__columns` 行无 DEFAULT 段：行长等于 G1 固定段长度。
    for (_, row) in &column_rows {
        let (table, column) = column_row_names(row);
        assert!(
            ["users", "audit", "events"].contains(&table.as_str()),
            "unexpected column row for table '{table}'"
        );
        let is_string = string_columns.contains(&(table.clone(), column.clone()));
        assert_eq!(
            row.len(),
            g1_columns_row_len(row, is_string),
            "column '{table}.{column}' must carry no DEFAULT section"
        );
    }
}

// ---------------------------------------------------------------------------
// R2：旧库可打开、可查询、可写入
// ---------------------------------------------------------------------------

/// 旧代际库能打开、能查询；写入后重开可读回新行，既有已提交行不丢失；
/// 完整性扫描的 error 级发现数为 0。
#[tokio::test]
async fn legacy_db_opens_queries_and_writes() {
    let dir = tempdir().unwrap();
    let db = copy_fixture(dir.path(), "legacy_g1.db");

    {
        let dbh = Database::open(&db).await.expect("open legacy db");
        let rows = db_rows(
            dbh.execute_sql("SELECT * FROM users")
                .await
                .expect("query users"),
        );
        assert_eq!(
            rows,
            vec![
                vec![
                    serde_json::json!(1),
                    serde_json::json!("alice"),
                    serde_json::json!(9.5)
                ],
                vec![
                    serde_json::json!(2),
                    serde_json::json!("bob"),
                    serde_json::json!(7.25)
                ],
                vec![
                    serde_json::json!(3),
                    serde_json::json!("carol"),
                    serde_json::json!(8.0)
                ],
            ]
        );

        let outcome = dbh
            .execute_sql("INSERT INTO users VALUES (4, 'dave', 6.0)")
            .await
            .expect("insert into legacy table");
        assert_eq!(outcome, SqlOutcome::AffectedRows(1));

        dbh.close().await.expect("close after write");
    }

    {
        let dbh = Database::open(&db).await.expect("reopen legacy db");
        let rows = db_rows(
            dbh.execute_sql("SELECT * FROM users")
                .await
                .expect("query users after write"),
        );
        assert_eq!(rows.len(), 4, "the inserted row must survive the reopen");
        assert_eq!(
            rows[3],
            vec![
                serde_json::json!(4),
                serde_json::json!("dave"),
                serde_json::json!(6.0)
            ]
        );
        // 既有已提交行不丢失。
        assert_eq!(rows[0][1], serde_json::json!("alice"));
        assert_eq!(rows[2][1], serde_json::json!("carol"));

        let report = check_integrity(&dbh.buffer_pool, &dbh.table_manager)
            .await
            .expect("integrity check");
        assert_eq!(
            report.error_count(),
            0,
            "integrity findings: {:?}",
            report.findings
        );

        dbh.close().await.expect("close reopened db");
    }
}

// ---------------------------------------------------------------------------
// R2：写入使被触及的表目录行升级为当前布局，其它表保持旧代际
// ---------------------------------------------------------------------------

/// 写入把**被写到**的表的目录行重写为当前布局（尾段：唯一根计数 0、
/// 二级索引计数 0、表类型字节 0x01）；同库**未被写到**的表目录行仍止于
/// 旧代际的数据页尾指针。
///
/// 这次写入跨过尾页容量 ⇒ **确实分配了新页**。因此本用例同时锁定
/// 「会触发页分配的写入」路径上的数据存活：重开后既有已提交行与新增行
/// 都可查、完整性扫描零 error（R2「写入 SHALL NOT 丢失既有已提交数据」与
/// R3「活页不进入可分配页集合」在该路径上的回归锁定）。
#[tokio::test]
async fn legacy_db_write_upgrades_touched_table_row_to_current_layout() {
    let dir = tempdir().unwrap();
    let db = copy_fixture(dir.path(), "legacy_g1.db");

    let size_before_write = {
        let dbh = Database::open(&db).await.expect("open legacy db");
        // 打开时的可分配页集合重建可能改写库文件，故「写入前」的文件大小取
        // 打开之后的观测值，使随后的增长可归因于这次写入。
        let size = std::fs::metadata(&db).expect("stat legacy db").len();
        // 追加到数据链尾页之外 ⇒ 分配新页 ⇒ 目录行被重写。
        // （该表的尾页在约 90 行后填满；此处取 120 行留出余量，规模远小于
        //   ISS23 记录的栈溢出触发面。）
        let mut values: Vec<String> = Vec::new();
        for i in 4..124 {
            values.push(format!("({i}, 'user{i}', 1.5)"));
        }
        let sql = format!("INSERT INTO users VALUES {};", values.join(", "));
        let outcome = dbh.execute_sql(&sql).await.expect("bulk insert");
        assert_eq!(outcome, SqlOutcome::AffectedRows(120));
        dbh.close().await.expect("close after write");
        size
    };

    // 写入确实分配了新页（否则新行会落在既有尾页内，文件不会增长）。
    let size_after_write = std::fs::metadata(&db).expect("stat legacy db").len();
    assert!(
        size_after_write > size_before_write,
        "the bulk write must allocate a new page \
         (db file grew from {size_before_write} to {size_after_write} bytes)"
    );

    let table_rows = read_chain_rows(&db, TABLES_PAGE_ID).await;
    let users = newest_table_row(&table_rows, "users");
    let g1_len = g1_tables_row_len(&users);
    assert_eq!(
        users.len(),
        g1_len + 9,
        "the written table's catalog row must carry the current-layout tail \
         (u32 unique roots | u32 secondary indexes | u8 kind), got {} bytes",
        users.len()
    );
    assert_eq!(
        &users[g1_len..],
        &[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01],
        "current-layout tail = unique-root count 0 | secondary-index count 0 | kind = Table"
    );

    for untouched in ["audit", "events"] {
        let row = newest_table_row(&table_rows, untouched);
        assert_eq!(
            row.len(),
            g1_tables_row_len(&row),
            "table '{untouched}' was never written — its catalog row must stay legacy"
        );
    }

    // 重开：会分配页的写入之后，既有已提交数据与新增行都仍可查，
    // 完整性扫描零 error。
    {
        let dbh = Database::open(&db)
            .await
            .expect("reopen legacy db after write");
        let users_rows = db_rows(
            dbh.execute_sql("SELECT * FROM users")
                .await
                .expect("query users after write"),
        );
        assert_eq!(
            users_rows.len(),
            123,
            "the pre-existing rows and the newly written rows must all be readable"
        );
        let mut ids: Vec<i64> = users_rows
            .iter()
            .map(|r| r[0].as_i64().expect("id column is an integer"))
            .collect();
        ids.sort_unstable();
        assert_eq!(
            ids,
            (1..=123).collect::<Vec<i64>>(),
            "every written id must be readable exactly once after the reopen"
        );
        let name_of = |id: i64| -> String {
            users_rows
                .iter()
                .find(|r| r[0].as_i64() == Some(id))
                .unwrap_or_else(|| panic!("row with id {id} must be present"))
                .get(1)
                .and_then(|v| v.as_str())
                .expect("name column is a string")
                .to_string()
        };
        // 既有已提交行未因（会分配页的）写入而丢失。
        assert_eq!(name_of(1), "alice", "the first committed row must survive");
        assert_eq!(name_of(2), "bob", "the second committed row must survive");
        assert_eq!(name_of(3), "carol", "the third committed row must survive");
        // 新写入的行同样可读。
        assert_eq!(
            name_of(123),
            "user123",
            "the last written row must be readable"
        );

        // 未被写到的表：既有已提交数据同样存活。
        let audit_rows = db_rows(
            dbh.execute_sql("SELECT * FROM audit")
                .await
                .expect("query audit after write"),
        );
        assert_eq!(
            audit_rows,
            vec![vec![serde_json::json!(1), serde_json::json!("kept")]],
            "the untouched table's committed row must survive the allocating write"
        );
        let events_rows = db_rows(
            dbh.execute_sql("SELECT * FROM events")
                .await
                .expect("query events after write"),
        );
        assert_eq!(
            events_rows.len(),
            1,
            "the second untouched table's committed row must survive the allocating write"
        );

        let report = check_integrity(&dbh.buffer_pool, &dbh.table_manager)
            .await
            .expect("integrity check");
        assert_eq!(
            report.error_count(),
            0,
            "integrity findings after the allocating write: {:?}",
            report.findings
        );

        dbh.close().await.expect("close reopened legacy db");
    }
}

// ---------------------------------------------------------------------------
// R3：打开时重建不放弃
// ---------------------------------------------------------------------------

/// 旧代际库（内含旧构建删掉的表 → 遗留空闲页）在当前打开路径上重建不放弃：
/// 打开成功、stderr 无「重建放弃」消息、活表行完整、error 级发现数为 0。
#[tokio::test]
async fn legacy_db_open_rebuild_keeps_live_table_intact() {
    let dir = tempdir().unwrap();
    let db = copy_fixture(dir.path(), "legacy_g1.db");

    // 真实 CLI 入口：用户实际所见的打开路径（stderr 是重建放弃的唯一出口）。
    let out = run_cli(&[db.to_str().unwrap(), "SELECT * FROM users;"]);
    assert_eq!(out.code, 0, "open must succeed, stderr: {}", out.stderr);
    assert!(
        !out.stderr.contains("rebuild abandoned"),
        "the open-time rebuild must not be abandoned, stderr: {}",
        out.stderr
    );
    assert_eq!(cli_rows(&out.stdout).len(), 3);

    let dbh = Database::open(&db).await.expect("open legacy db");
    let users = db_rows(
        dbh.execute_sql("SELECT * FROM users")
            .await
            .expect("query users"),
    );
    assert_eq!(users.len(), 3, "live table rows must stay intact");
    let events = db_rows(
        dbh.execute_sql("SELECT * FROM events")
            .await
            .expect("query events"),
    );
    assert_eq!(events.len(), 1, "second live table must stay intact");
    assert!(
        dbh.get_table("discarded").await.is_err(),
        "the table dropped by the legacy build must stay gone"
    );
    let report = check_integrity(&dbh.buffer_pool, &dbh.table_manager)
        .await
        .expect("integrity check");
    assert_eq!(
        report.error_count(),
        0,
        "integrity findings: {:?}",
        report.findings
    );
    dbh.close().await.expect("close legacy db");
}

// ---------------------------------------------------------------------------
// R3：旧库带未决写入日志 → 恢复出已提交行
// ---------------------------------------------------------------------------

/// 未决写入日志样本恢复后的行数参考值。
///
/// 该数字是**已提交样本字节的属性**（确定）：同一份样本字节恢复出的行集恒定。
/// 只有重新生成样本（旧构建被杀时机不同）才会改变它——那时本常量与
/// `tests/fixtures/legacy/README.md` 的参考值必须一起更新。
const RECOVERED_ROWS_REFERENCE: usize = 133;

/// 旧代际库带未决写入日志时，当前构建打开后恢复出已提交行，且 error 级
/// 发现数为 0。
///
/// 门槛是「恢复行数 == 参考值」而不是「≥ 1」：样本自带 1 行种子数据，只断言
/// 「≥ 1」时**未决日志未被重放**（例如旧格式日志帧被静默跳过）同样成立，
/// 断言就失去判别力。恢复行数等于参考值才证明未决日志被重放。
#[tokio::test]
async fn legacy_db_with_pending_wal_recovers_committed_rows() {
    let dir = tempdir().unwrap();
    let db = copy_fixture(dir.path(), "legacy_g1_pending_wal.db");
    // 写入日志必须与库文件同名同目录。
    copy_fixture(dir.path(), "legacy_g1_pending_wal.wal");

    let dbh = Database::open(&db)
        .await
        .expect("open legacy db with pending wal");
    let rows = db_rows(
        dbh.execute_sql("SELECT * FROM t")
            .await
            .expect("query recovered table"),
    );
    assert_eq!(
        rows.len(),
        RECOVERED_ROWS_REFERENCE,
        "the pending write log must replay into recovered rows \
         (expected {RECOVERED_ROWS_REFERENCE} recovered rows, got {}); \
         without the pending log only the 1 seeded row is present",
        rows.len()
    );
    // 恢复前的既有已提交行（种子行）不得因重放而丢失。
    assert!(
        rows.iter().any(|r| r[0].as_i64() == Some(0)),
        "the committed seed row must survive the replay of the pending write log"
    );
    let report = check_integrity(&dbh.buffer_pool, &dbh.table_manager)
        .await
        .expect("integrity check");
    assert_eq!(
        report.error_count(),
        0,
        "integrity findings: {:?}",
        report.findings
    );
    dbh.close().await.expect("close legacy db");
}

// ---------------------------------------------------------------------------
// R4：旧 dump 可恢复到当前库
// ---------------------------------------------------------------------------

/// 旧 dump 恢复到当前库成功且行数据等价（含日期/时间戳/布尔字面量）；
/// 恢复出的库可再次导出并保持等价。
#[tokio::test]
async fn legacy_dump_restores_into_current_db() {
    let dir = tempdir().unwrap();
    let dump = fixture_path("legacy_dump.sql");
    let restored = dir.path().join("restored.db");

    let out = run_cli(&["new", restored.to_str().unwrap()]);
    assert_eq!(out.code, 0, "create empty target db failed: {}", out.stderr);

    let out = run_cli(&[
        "restore",
        restored.to_str().unwrap(),
        dump.to_str().unwrap(),
    ]);
    assert_eq!(out.code, 0, "restore must succeed, stderr: {}", out.stderr);

    let out = run_cli(&[restored.to_str().unwrap(), "SELECT * FROM users;"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        cli_rows(&out.stdout),
        vec![
            vec![
                serde_json::json!(1),
                serde_json::json!("alice"),
                serde_json::json!(9.5)
            ],
            vec![
                serde_json::json!(2),
                serde_json::json!("bob"),
                serde_json::json!(7.25)
            ],
            vec![
                serde_json::json!(3),
                serde_json::json!("carol"),
                serde_json::json!(8.0)
            ],
        ]
    );

    let out = run_cli(&[restored.to_str().unwrap(), "SELECT * FROM audit;"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        cli_rows(&out.stdout),
        vec![vec![serde_json::json!(1), serde_json::json!("kept")]]
    );

    // 日期 / 时间戳 / 布尔字面量的旧 dump 同样可恢复。
    let out = run_cli(&[restored.to_str().unwrap(), "SELECT * FROM events;"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        cli_rows(&out.stdout),
        vec![vec![
            serde_json::json!(1),
            serde_json::json!("2026-01-02"),
            serde_json::json!("2026-01-02 03:04:05"),
            serde_json::json!(true),
        ]]
    );

    // 恢复出的库可再次导出并保持等价。
    let out = run_cli(&["dump", restored.to_str().unwrap()]);
    assert_eq!(out.code, 0, "dump must succeed, stderr: {}", out.stderr);
    let first_dump = out.stdout.clone();
    let again = dir.path().join("again.sql");
    std::fs::write(&again, &first_dump).expect("write re-dump");
    let restored_again = dir.path().join("restored_again.db");
    let out = run_cli(&["new", restored_again.to_str().unwrap()]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let out = run_cli(&[
        "restore",
        restored_again.to_str().unwrap(),
        again.to_str().unwrap(),
    ]);
    assert_eq!(
        out.code, 0,
        "restore of the re-dump must succeed, stderr: {}",
        out.stderr
    );
    let out = run_cli(&["dump", restored_again.to_str().unwrap()]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        out.stdout, first_dump,
        "the re-exported dump must stay equivalent"
    );
}

// ---------------------------------------------------------------------------
// R5：三类范围外输入给出明确、定位到语句的拒绝
// ---------------------------------------------------------------------------

/// 非整数主键的旧 dump：恢复失败，原因点名 + 语句序号定位，失败退出码 3。
#[tokio::test]
async fn legacy_dump_with_non_integer_primary_key_is_rejected() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("target.db");
    let out = run_cli(&["new", target.to_str().unwrap()]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let dump = fixture_path("legacy_dump_non_integer_pk.sql");
    let out = run_cli(&["restore", target.to_str().unwrap(), dump.to_str().unwrap()]);
    assert_eq!(
        out.code, 3,
        "restore must fail with exit code 3, stderr: {}",
        out.stderr
    );
    let message = cli_error_message(&out.stderr);
    assert!(
        message.contains("NON-INTEGER PRIMARY KEY"),
        "the rejection must name the cause: {message}"
    );
    assert!(
        message.contains("statement 1 of 2 failed"),
        "the rejection must locate the failing statement: {message}"
    );
    assert!(
        message.contains("statement: CREATE TABLE"),
        "the rejection must quote the failing statement: {message}"
    );
}

/// 非整数 UNIQUE 列的旧 dump：恢复失败，原因点名 + 语句序号定位，失败退出码 3。
#[tokio::test]
async fn legacy_dump_with_non_integer_unique_is_rejected() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("target.db");
    let out = run_cli(&["new", target.to_str().unwrap()]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let dump = fixture_path("legacy_dump_non_integer_unique.sql");
    let out = run_cli(&["restore", target.to_str().unwrap(), dump.to_str().unwrap()]);
    assert_eq!(
        out.code, 3,
        "restore must fail with exit code 3, stderr: {}",
        out.stderr
    );
    let message = cli_error_message(&out.stderr);
    assert!(
        message.contains("UNIQUE (INT columns only)"),
        "the rejection must name the cause: {message}"
    );
    assert!(
        message.contains("statement 1 of 2 failed"),
        "the rejection must locate the failing statement: {message}"
    );
    assert!(
        message.contains("statement: CREATE TABLE"),
        "the rejection must quote the failing statement: {message}"
    );
}

/// NOT NULL 列含 NULL 的旧 dump：恢复失败，原因点名 + 列名 + 语句序号定位，
/// 失败退出码 3；失败前的语句已各自提交（restore 的既有逐条 auto-commit 合同）。
#[tokio::test]
async fn legacy_dump_with_null_in_not_null_is_rejected() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("target.db");
    let out = run_cli(&["new", target.to_str().unwrap()]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let dump = fixture_path("legacy_dump_null_in_not_null.sql");
    let out = run_cli(&["restore", target.to_str().unwrap(), dump.to_str().unwrap()]);
    assert_eq!(
        out.code, 3,
        "restore must fail with exit code 3, stderr: {}",
        out.stderr
    );
    let message = cli_error_message(&out.stderr);
    assert!(
        message.contains("NOT NULL constraint violation"),
        "the rejection must name the cause: {message}"
    );
    assert!(
        message.contains("column 's'"),
        "the rejection must name the column: {message}"
    );
    assert!(
        message.contains("statement 3 of 3 failed"),
        "the rejection must locate the failing statement: {message}"
    );
    assert!(
        message.contains("previous statement(s) were committed"),
        "the rejection must keep the per-statement auto-commit contract: {message}"
    );
}

// ---------------------------------------------------------------------------
// G2 代际（目录行含唯一根段、不含二级索引段与表类型字节）的文件级样本
// ---------------------------------------------------------------------------

/// `__columns` 行的类型标签字节（`COL_TAG_*`，与 `src/storage/catalog.rs` 同值）。
const COL_TAG_STRING: u8 = 0x02;

/// 元组格式的字符串标签（`TAG_STRING`，与 `src/storage/page_format/tuple.rs` 同值）。
const TUPLE_TAG_STRING: u8 = 0x02;

/// G2 目录行长度：G1 固定段 + `u32 唯一根计数`（唯一根数为 0 ⇒ 该段 4 字节）。
///
/// G2 布局：`u16 名字长 | 名字 | u32 数据页头 | u32 索引根 | u32 主键索引 |
///           u16 主键列长 | 主键列 | u32 列数 | u32 数据页尾指针 |
///           u32 唯一根计数 | 唯一根数 × u32 根`。
/// 该段之后**没有**二级索引段与表类型字节（当前布局才有）。
fn g2_tables_row_len(row: &[u8]) -> usize {
    g1_tables_row_len(row) + 4
}

/// G2 目录行的唯一根计数段（`u32`）。
fn g2_tables_unique_count(row: &[u8]) -> u32 {
    let off = g1_tables_row_len(row);
    u32::from_le_bytes([row[off], row[off + 1], row[off + 2], row[off + 3]])
}

/// `__columns` 行的类型标签偏移。
fn columns_type_tag_offset(row: &[u8]) -> usize {
    let table_len = u16::from_le_bytes([row[0], row[1]]) as usize;
    let mut p = 2 + table_len + 4; // 跳过列序
    let column_len = u16::from_le_bytes([row[p], row[p + 1]]) as usize;
    p += 2 + column_len; // 列名
    p
}

/// `__columns` 行的可选 DEFAULT 段（`u8 有默认值 | u8 值标签 | 载荷`）。
///
/// 返回 (段长度, 字符串默认值)；行尾**没有**该段时返回 `None`。
fn columns_default_segment(row: &[u8], base_len: usize) -> Option<(usize, String)> {
    let rest = &row[base_len..];
    if rest.is_empty() {
        return None;
    }
    assert_eq!(rest[0], 1, "the DEFAULT section must be flagged as present");
    assert_eq!(
        rest[1], TUPLE_TAG_STRING,
        "the sample's DEFAULT literal is a string"
    );
    let len = u16::from_le_bytes([rest[2], rest[3]]) as usize;
    let value = String::from_utf8(rest[4..4 + len].to_vec()).expect("utf8 default literal");
    Some((rest.len(), value))
}

/// G2 样本的目录行形态：目录行**含唯一根段**、**不含**二级索引段与表类型
/// 字节；`__columns` 行只在声明 DEFAULT 的列上带默认值段。
///
/// 静态核对：提交的样本文件本身（`legacy_g2.db`）的目录链原始字节。
#[tokio::test]
async fn legacy_g2_catalog_rows_are_in_g2_generation() {
    let dir = tempdir().unwrap();
    let db = copy_fixture(dir.path(), "legacy_g2.db");

    let table_rows = read_chain_rows(&db, TABLES_PAGE_ID).await;
    let names: Vec<String> = table_rows
        .iter()
        .map(|(_, bytes)| tables_row_name(bytes).expect("table name"))
        .collect();
    assert_eq!(
        names,
        vec!["people".to_string(), "notes".to_string()],
        "the G2 sample holds exactly the two live tables"
    );
    for (_, row) in &table_rows {
        let name = tables_row_name(row).expect("table name");
        assert_eq!(
            row.len(),
            g2_tables_row_len(row),
            "table '{name}' catalog row must end at the unique-root count \
             (no secondary-index / kind tail), got {} bytes",
            row.len()
        );
        assert_eq!(
            g2_tables_unique_count(row),
            0,
            "table '{name}' declares no unique index in the G2 generation"
        );
    }
    assert!(
        find_table_row(&table_rows, "scratch").is_none(),
        "the table dropped by the legacy build must be gone from the directory"
    );

    let column_rows = read_chain_rows(&db, COLUMNS_PAGE_ID).await;
    assert_eq!(
        column_rows.len(),
        6,
        "the G2 sample declares 3 columns on 2 live tables"
    );
    let mut default_columns = 0;
    for (_, row) in &column_rows {
        let is_string = row[columns_type_tag_offset(row)] == COL_TAG_STRING;
        let base = g1_columns_row_len(row, is_string);
        let (table, column) = column_row_names(row);
        if column == "tag" {
            let (seg_len, value) = columns_default_segment(row, base)
                .expect("the DEFAULT column must carry the default section");
            assert_eq!(value, "none", "the declared DEFAULT literal is preserved");
            assert_eq!(
                row.len(),
                base + seg_len,
                "the DEFAULT section is the tail of the column row"
            );
            default_columns += 1;
        } else {
            assert_eq!(
                row.len(),
                base,
                "column '{table}.{column}' must carry no DEFAULT section"
            );
        }
    }
    assert_eq!(default_columns, 1, "exactly one column declares a DEFAULT");
}

/// G2 样本可被当前构建打开、查询与写入；写入把**被触及**的表目录行升级为
/// 当前布局，未触及的表目录行仍停在 G2 代际；完整性扫描零 error。
#[tokio::test]
async fn legacy_g2_db_opens_queries_and_writes() {
    let dir = tempdir().unwrap();
    let db = copy_fixture(dir.path(), "legacy_g2.db");

    {
        let dbh = Database::open(&db).await.expect("open legacy g2 db");
        let people = db_rows(
            dbh.execute_sql("SELECT * FROM people")
                .await
                .expect("query people"),
        );
        assert_eq!(
            people,
            vec![
                vec![
                    serde_json::json!(1),
                    serde_json::json!("ada"),
                    serde_json::json!(9.5)
                ],
                vec![
                    serde_json::json!(2),
                    serde_json::json!("bob"),
                    serde_json::json!(7.25)
                ],
                vec![
                    serde_json::json!(3),
                    serde_json::json!("cyd"),
                    serde_json::json!(8.0)
                ],
            ]
        );
        let notes = db_rows(
            dbh.execute_sql("SELECT * FROM notes")
                .await
                .expect("query notes"),
        );
        assert_eq!(
            notes,
            vec![vec![
                serde_json::json!(1),
                serde_json::json!("first note"),
                serde_json::json!("none")
            ]],
            "the legacy build's DEFAULT value must be read back"
        );

        let outcome = dbh
            .execute_sql("INSERT INTO notes (id, body) VALUES (2, 'second note')")
            .await
            .expect("insert into legacy g2 table");
        assert_eq!(outcome, SqlOutcome::AffectedRows(1));

        // 一次**会分配新页**的写入 ⇒ 被触及的表目录行被重写为当前布局。
        // （`people` 的尾页在约 90 行后填满；此处取 120 行留出余量，规模远小于
        //   ISS23 记录的栈溢出触发面。）
        let size_before_write = std::fs::metadata(&db).expect("stat legacy g2 db").len();
        let mut values: Vec<String> = Vec::new();
        for i in 4..124 {
            values.push(format!("({i}, 'user{i}', 1.5)"));
        }
        let sql = format!("INSERT INTO people VALUES {};", values.join(", "));
        let outcome = dbh.execute_sql(&sql).await.expect("bulk insert");
        assert_eq!(outcome, SqlOutcome::AffectedRows(120));

        dbh.close().await.expect("close after write");

        let size_after_write = std::fs::metadata(&db).expect("stat legacy g2 db").len();
        assert!(
            size_after_write > size_before_write,
            "the bulk write must allocate a new page \
             (db file grew from {size_before_write} to {size_after_write} bytes)"
        );
    }

    {
        let dbh = Database::open(&db).await.expect("reopen legacy g2 db");
        let people = db_rows(
            dbh.execute_sql("SELECT * FROM people")
                .await
                .expect("query people after write"),
        );
        assert_eq!(
            people.len(),
            123,
            "the pre-existing rows and the newly written rows must all be readable"
        );
        let notes = db_rows(
            dbh.execute_sql("SELECT * FROM notes")
                .await
                .expect("query notes after write"),
        );
        assert_eq!(notes.len(), 2, "the inserted row must survive the reopen");
        assert_eq!(
            notes[1],
            vec![
                serde_json::json!(2),
                serde_json::json!("second note"),
                serde_json::json!("none")
            ],
            "the declared DEFAULT still fills the omitted column"
        );
        let report = check_integrity(&dbh.buffer_pool, &dbh.table_manager)
            .await
            .expect("integrity check");
        assert_eq!(
            report.error_count(),
            0,
            "integrity findings: {:?}",
            report.findings
        );
        dbh.close().await.expect("close reopened db");
    }

    let table_rows = read_chain_rows(&db, TABLES_PAGE_ID).await;
    let people_row = newest_table_row(&table_rows, "people");
    assert_eq!(
        people_row.len(),
        g1_tables_row_len(&people_row) + 9,
        "the written table's catalog row must be rewritten in the current layout \
         (unique roots | secondary indexes | kind), got {} bytes",
        people_row.len()
    );
    assert_eq!(
        &people_row[g1_tables_row_len(&people_row)..],
        &[0, 0, 0, 0, 0, 0, 0, 0, 0x01],
        "the current layout tail: unique roots 0 | secondary indexes 0 | table kind"
    );
    let notes_row = newest_table_row(&table_rows, "notes");
    assert_eq!(
        notes_row.len(),
        g2_tables_row_len(&notes_row),
        "the untouched table's catalog row must stay in the G2 generation"
    );
    assert_eq!(g2_tables_unique_count(&notes_row), 0);
}

/// G2 样本经 CLI 打开时，可分配页重建不被放弃；既有活表完好、被旧构建删掉的
/// 表不在目录里、完整性扫描零 error。
#[tokio::test]
async fn legacy_g2_db_open_rebuild_keeps_live_table_intact() {
    let dir = tempdir().unwrap();
    let db = copy_fixture(dir.path(), "legacy_g2.db");

    let out = run_cli(&[db.to_str().unwrap(), "SELECT * FROM people;"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert!(
        !out.stderr.contains("rebuild abandoned"),
        "the free-page rebuild must not be abandoned: {}",
        out.stderr
    );
    let rows = cli_rows(&out.stdout);
    assert_eq!(
        rows.len(),
        3,
        "the live table keeps all rows after the rebuild"
    );
    assert_eq!(
        rows[0],
        vec![
            serde_json::json!(1),
            serde_json::json!("ada"),
            serde_json::json!(9.5)
        ]
    );

    let table_rows = read_chain_rows(&db, TABLES_PAGE_ID).await;
    assert!(
        find_table_row(&table_rows, "scratch").is_none(),
        "the table dropped by the legacy build must stay gone"
    );

    let dbh = Database::open(&db).await.expect("open legacy g2 db");
    let report = check_integrity(&dbh.buffer_pool, &dbh.table_manager)
        .await
        .expect("integrity check");
    assert_eq!(
        report.error_count(),
        0,
        "integrity findings: {:?}",
        report.findings
    );
    dbh.close().await.expect("close");
}

/// G2 样本可被当前构建 dump；导出的文本可被当前构建恢复，恢复后的行集与
/// 原库一致（含声明 DEFAULT 的列）。
#[tokio::test]
async fn legacy_g2_dump_restores_into_current_db() {
    let dir = tempdir().unwrap();
    let db = copy_fixture(dir.path(), "legacy_g2.db");

    let out = run_cli(&["dump", db.to_str().unwrap()]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let dump_text = out.stdout;
    assert!(
        dump_text.contains("CREATE TABLE \"people\""),
        "the dump must export the 'people' table: {dump_text}"
    );
    assert!(
        dump_text.contains("CREATE TABLE \"notes\""),
        "the dump must export the 'notes' table: {dump_text}"
    );
    assert!(
        !dump_text.contains("scratch"),
        "the dropped table must not be exported: {dump_text}"
    );
    for value in ["'ada'", "'cyd'", "'first note'", "'none'"] {
        assert!(
            dump_text.contains(value),
            "the dump must export the row value {value}: {dump_text}"
        );
    }

    let dump_file = dir.path().join("legacy_g2_dump.sql");
    std::fs::write(&dump_file, &dump_text).expect("write dump text");
    let restored = dir.path().join("restored_g2.db");
    let out = run_cli(&["new", restored.to_str().unwrap()]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let out = run_cli(&[
        "restore",
        restored.to_str().unwrap(),
        dump_file.to_str().unwrap(),
    ]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let out = run_cli(&[restored.to_str().unwrap(), "SELECT * FROM people;"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let rows = cli_rows(&out.stdout);
    assert_eq!(rows.len(), 3, "the restored table keeps all three rows");
    assert_eq!(
        rows[2],
        vec![
            serde_json::json!(3),
            serde_json::json!("cyd"),
            serde_json::json!(8.0)
        ]
    );
    let out = run_cli(&[restored.to_str().unwrap(), "SELECT * FROM notes;"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        cli_rows(&out.stdout),
        vec![vec![
            serde_json::json!(1),
            serde_json::json!("first note"),
            serde_json::json!("none")
        ]],
        "the DEFAULT-declared column survives the dump/restore round trip"
    );
}

// ---------------------------------------------------------------------------
// 一次性实测条款常态化：迁移路径实测记录里的三条一次性观察
// ---------------------------------------------------------------------------

/// 恢复出的库**可继续写入**：恢复后的旧 dump 库能接受新行，新行可查回。
#[tokio::test]
async fn restored_legacy_db_accepts_further_writes() {
    let dir = tempdir().unwrap();
    let dump = fixture_path("legacy_dump.sql");
    let restored = dir.path().join("restored.db");

    let out = run_cli(&["new", restored.to_str().unwrap()]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let out = run_cli(&[
        "restore",
        restored.to_str().unwrap(),
        dump.to_str().unwrap(),
    ]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let out = run_cli(&[
        restored.to_str().unwrap(),
        "INSERT INTO users VALUES (4, 'dave', 6.0);",
    ]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("\"affected_rows\":1"),
        "the insert must report one affected row: {}",
        out.stdout
    );

    let out = run_cli(&[restored.to_str().unwrap(), "SELECT * FROM users;"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let rows = cli_rows(&out.stdout);
    assert_eq!(
        rows.len(),
        4,
        "the restored table keeps its three rows plus the new one"
    );
    assert_eq!(
        rows[3],
        vec![
            serde_json::json!(4),
            serde_json::json!("dave"),
            serde_json::json!(6.0)
        ]
    );

    let out = run_cli(&[
        restored.to_str().unwrap(),
        "INSERT INTO audit VALUES (2, 'after-restore');",
    ]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let out = run_cli(&[restored.to_str().unwrap(), "SELECT COUNT(*) FROM audit;"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        cli_rows(&out.stdout),
        vec![vec![serde_json::json!(2)]],
        "the restored table accepts a second write on another table too"
    );
}

/// 旧 G1 库**可由当前构建 dump**：退出码 0、无告警、导出 3 张表与全部行；
/// 导出的文本可被当前构建恢复（往返等价）。
#[tokio::test]
async fn current_build_dumps_legacy_g1_db() {
    let dir = tempdir().unwrap();
    let g1 = copy_fixture(dir.path(), "legacy_g1.db");

    let out = run_cli(&["dump", g1.to_str().unwrap()]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert!(
        out.stderr.is_empty(),
        "the dump must not warn: {}",
        out.stderr
    );
    let dump_text = out.stdout;
    for table in ["users", "audit", "events"] {
        assert!(
            dump_text.contains(&format!("CREATE TABLE \"{table}\"")),
            "the dump must export the '{table}' table: {dump_text}"
        );
    }
    for value in ["'alice'", "'bob'", "'carol'", "'kept'"] {
        assert!(
            dump_text.contains(value),
            "the dump must export the row value {value}: {dump_text}"
        );
    }

    let dump_file = dir.path().join("legacy_g1_dump.sql");
    std::fs::write(&dump_file, &dump_text).expect("write dump text");
    let restored = dir.path().join("restored_g1.db");
    let out = run_cli(&["new", restored.to_str().unwrap()]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let out = run_cli(&[
        "restore",
        restored.to_str().unwrap(),
        dump_file.to_str().unwrap(),
    ]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);

    let out = run_cli(&[restored.to_str().unwrap(), "SELECT * FROM users;"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let rows = cli_rows(&out.stdout);
    assert_eq!(rows.len(), 3, "the dumped table keeps all three rows");
    assert_eq!(
        rows[0],
        vec![
            serde_json::json!(1),
            serde_json::json!("alice"),
            serde_json::json!(9.5)
        ]
    );
    assert_eq!(
        rows[2],
        vec![
            serde_json::json!(3),
            serde_json::json!("carol"),
            serde_json::json!(8.0)
        ]
    );
    let out = run_cli(&[restored.to_str().unwrap(), "SELECT * FROM events;"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        cli_rows(&out.stdout).len(),
        1,
        "the dumped table keeps its row after the round trip"
    );
}

/// 旧 G1 库**可连续写两次**：两次写入退出码均为 0，两次写入的行都能查回。
#[tokio::test]
async fn legacy_g1_db_accepts_two_consecutive_writes() {
    let dir = tempdir().unwrap();
    let g1 = copy_fixture(dir.path(), "legacy_g1.db");

    let out = run_cli(&[
        g1.to_str().unwrap(),
        "INSERT INTO users VALUES (4, 'dave', 6.0);",
    ]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("\"affected_rows\":1"),
        "the first write must report one affected row: {}",
        out.stdout
    );

    let out = run_cli(&[
        g1.to_str().unwrap(),
        "INSERT INTO users VALUES (5, 'erin', 5.5);",
    ]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert!(
        out.stdout.contains("\"affected_rows\":1"),
        "the second write must report one affected row: {}",
        out.stdout
    );

    let out = run_cli(&[g1.to_str().unwrap(), "SELECT COUNT(*) FROM users;"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        cli_rows(&out.stdout),
        vec![vec![serde_json::json!(5)]],
        "both writes must be visible"
    );

    let out = run_cli(&[
        g1.to_str().unwrap(),
        "INSERT INTO audit VALUES (2, 'second-round');",
    ]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let out = run_cli(&[g1.to_str().unwrap(), "SELECT * FROM audit;"]);
    assert_eq!(out.code, 0, "stderr: {}", out.stderr);
    let rows = cli_rows(&out.stdout);
    assert_eq!(
        rows.len(),
        2,
        "the audit table keeps its old row plus the new one"
    );
    assert_eq!(
        rows[1],
        vec![serde_json::json!(2), serde_json::json!("second-round")]
    );
}
