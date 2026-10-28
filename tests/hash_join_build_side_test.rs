//! 等值哈希连接始终构建右输入时，输出按左行分组。

use rtsql::database::Database;
use rtsql::executor::Value;
use rtsql::SqlOutcome;
use tempfile::TempDir;

const JOIN_SQL: &str = "SELECT l.tag, r.tag FROM l INNER JOIN r ON l.k = r.k";

async fn join_tags(left_tags: &[&str], right_tags: &[&str]) -> Vec<Vec<Value>> {
    let dir = TempDir::new().unwrap();
    let db = Database::open(&dir.path().join("t.db")).await.unwrap();
    db.execute_sql("CREATE TABLE l (id INT PRIMARY KEY, k INT, tag STRING)")
        .await
        .unwrap();
    db.execute_sql("CREATE TABLE r (id INT PRIMARY KEY, k INT, tag STRING)")
        .await
        .unwrap();
    for (offset, tag) in left_tags.iter().enumerate() {
        let id = offset + 1;
        db.execute_sql(&format!("INSERT INTO l VALUES ({id}, 1, '{tag}')"))
            .await
            .unwrap();
    }
    for (offset, tag) in right_tags.iter().enumerate() {
        let id = offset + 1;
        db.execute_sql(&format!("INSERT INTO r VALUES ({id}, 1, '{tag}')"))
            .await
            .unwrap();
    }
    match db.execute_sql(JOIN_SQL).await.unwrap() {
        SqlOutcome::Rows { rows, .. } => rows,
        SqlOutcome::AffectedRows(n) => panic!("expected rows, got {n} affected"),
    }
}

fn pair(left: &str, right: &str) -> Vec<Value> {
    vec![
        Value::String(left.to_string()),
        Value::String(right.to_string()),
    ]
}

fn grouped_by_left(left: &[&str], right: &[&str]) -> Vec<Vec<Value>> {
    let mut rows = Vec::new();
    for left_tag in left {
        for right_tag in right {
            rows.push(pair(left_tag, right_tag));
        }
    }
    rows
}

#[tokio::test]
async fn hash_join_equal_counts_probe_left() {
    let left = ["L1", "L2"];
    let right = ["R1", "R2"];
    assert_eq!(
        join_tags(&left, &right).await,
        grouped_by_left(&left, &right)
    );
}

#[tokio::test]
async fn hash_join_larger_left_still_probes_left() {
    let left = ["L1", "L2", "L3", "L4", "L5"];
    let right = ["R1", "R2"];
    assert_eq!(
        join_tags(&left, &right).await,
        grouped_by_left(&left, &right)
    );
}

#[tokio::test]
async fn hash_join_builds_right_when_right_is_larger() {
    let left = ["L1", "L2"];
    let right = ["R1", "R2", "R3", "R4", "R5"];
    assert_eq!(
        join_tags(&left, &right).await,
        grouped_by_left(&left, &right)
    );
}
