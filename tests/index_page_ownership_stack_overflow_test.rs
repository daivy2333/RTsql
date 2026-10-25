//! `index-page-ownership`（IPO）R3 —— ISS23（B 树插入路径无限递归致栈溢出）
//! 的回归用例。
//!
//! change: `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence`
//! （Iteration 000）
//!
//! 场景（真实入口、真实文件库、真实 `Database::open`）：
//! 1. 写入 250 行 ⇒ 主键索引分裂成多片叶；
//! 2. 删除 125 行 ⇒ 叶欠载 ⇒ 兄弟合并 ⇒ **被合并掉的页被释放**（B-2：修复前
//!    同一页号被释放两次 ⇒ 同一页号两次进入待释放集合）；
//! 3. `checkpoint` ⇒ 释放完成的页进入**可分配集合**（`promote_free_releases`，
//!    与 `src/wal/checkpoint.rs` 第 2 步同一条入口）；
//! 4. 再插入 ⇒ 分配到被释放的页 ⇒ **修复前同一页号被交给两个对象** ⇒ 索引
//!    结构被破坏 ⇒ 插入路径下降成环 ⇒ 栈溢出、进程 abort（ISS23 的登记症状：
//!    `thread 'tokio-rt-worker' has overflowed its stack` → SIGABRT）。
//!
//! 规模：250 行（< 300，ISS23 的规模约束内）。
//!
//! **独立成文件的原因**：本用例在缺陷未修复时以**进程 abort**（栈溢出）失败。
//! 与其它用例同处一个测试二进制时，abort 会带走同批其它用例的观测结果
//! （`index_page_ownership_test.rs` 的 RED 观测需要与本用例分开）。

use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use tempfile::TempDir;

/// 行数（< 300）。
const ROWS: i64 = 250;

async fn ok(db: &Database, sql: &str) {
    let resp = to_response(db.execute_sql(sql).await);
    assert!(
        !matches!(resp, Response::Error { .. }),
        "{sql} must succeed: {resp:?}"
    );
}

async fn insert_range(db: &Database, from: i64, to: i64) {
    for start in (from..to).step_by(50) {
        let end = (start + 50).min(to);
        let values: Vec<String> = (start..end).map(|i| format!("({i}, {i})")).collect();
        ok(db, &format!("INSERT INTO t VALUES {}", values.join(","))).await;
    }
}

/// 被释放的页不得同时属于两个对象——插入路径的下降必然终止（不 abort）。
#[tokio::test]
async fn released_index_page_never_breaks_the_insert_descent() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.db");
    let db = Database::open(&path).await.unwrap();
    ok(&db, "CREATE TABLE t (id INT PRIMARY KEY, n INT)").await;

    insert_range(&db, 0, ROWS).await;

    // 删除 ⇒ 主键索引的叶发生兄弟合并 ⇒ 被合并掉的页被释放。
    let tx = db.begin().await.unwrap();
    for id in 0..(ROWS / 2) {
        to_response(
            db.execute_in_tx(&format!("DELETE FROM t WHERE id = {id}"), &tx)
                .await,
        );
    }
    db.commit(tx).await.unwrap();

    // checkpoint ⇒ 释放完成的页进入可分配集合。
    db.checkpoint().await.unwrap();

    // 插入 ⇒ 分配到被释放的页 ⇒ 插入路径的下降。
    insert_range(&db, ROWS, ROWS * 2).await;
    insert_range(&db, ROWS * 2, ROWS * 3).await;

    // 语句正常返回，行全部可读（修复前：进程 abort 在此之前）。
    let count = match to_response(db.execute_sql("SELECT COUNT(*) FROM t").await) {
        Response::QueryResult { rows } => rows,
        other => panic!("unexpected: {other:?}"),
    };
    assert_eq!(
        count,
        vec![vec![serde_json::json!(ROWS / 2 + ROWS + ROWS)]],
        "every inserted row must stay visible"
    );

    db.close().await.unwrap();
}
