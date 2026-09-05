//! MS08-T02: DataScan 链预取行为等价测试
//!
//! R3 场景覆盖（change `2026-09-05-ms08-t01-t02-pread-prefetch`）：
//! - 预取下全表扫描行序与结果等价（多页数据集，开/关预取对照）
//! - 预取不破坏谓词下推与 LIMIT 语义（含 limit=0、谓词全过滤）
//! - 链尾/空表/单页路径：不预取 PageId(0)，扫描正常结束
//!
//! 预取默认关闭（replan 2026-09-05）：全部 ON 路径显式 `.with_prefetch(true)`；
//! `new` 默认值由 `src/executor/data_scan.rs` 模块内单测守卫。

use rtsql::executor::{
    ColumnExpression, ComparisonOp, ComparisonPredicate, ConstantExpression, DataScanExecutor,
    ExecResult, Executor, InsertExecutor, PredicateRef, Value,
};
use rtsql::storage::{
    data::TableManager,
    page_format::{ColumnType, SlottedPageRef},
    BufferPool, FileStorage, PageId, Result,
};
use rtsql::transaction::TransactionManager;
use std::sync::Arc;
use tempfile::tempdir;

const ROWS: usize = 2000;

/// 建表并批量插入 `row_count` 行（id = 1..=row_count），返回扫描所需句柄。
/// BufferPool 容量取 8 制造 miss/驱逐压力，使预取路径被真实覆盖。
async fn setup_table(row_count: usize) -> (Arc<TableManager>, Arc<BufferPool>, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let storage = Arc::new(FileStorage::open(&dir.path().join("test.db")).unwrap());
    let buffer_pool = Arc::new(BufferPool::new(8, storage.clone()).unwrap());
    let table_mgr = TableManager::new(buffer_pool.clone(), storage)
        .await
        .unwrap();
    table_mgr
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await
        .unwrap();

    if row_count > 0 {
        let table_meta = table_mgr.get_table("test").await.unwrap();
        let values: Vec<Vec<Value>> = (1..=row_count as i64)
            .map(|i| vec![Value::Int(i)])
            .collect();
        let mut insert = InsertExecutor::new(
            table_meta,
            buffer_pool.clone(),
            Arc::new(TransactionManager::new()),
            values,
            0,
            None,
        );
        assert_eq!(
            insert.next().await.unwrap(),
            Some(ExecResult::AffectedRows(row_count as u64))
        );
    }
    (table_mgr, buffer_pool, dir)
}

/// 沿数据页链统计页数（守卫多页前提；next_page_id == 0 为链尾哨兵）
async fn count_chain_pages(buffer_pool: &BufferPool, head: PageId) -> usize {
    let mut count = 0usize;
    let mut current = head;
    loop {
        count += 1;
        let next: u32 = buffer_pool
            .with_page_data(current, |data| -> Result<u32> {
                Ok(SlottedPageRef::new(data).header().next_page_id)
            })
            .await
            .unwrap();
        if next == 0 {
            break;
        }
        current = PageId(next as u64);
    }
    count
}

/// 扫描全部行，按产出顺序收集 id
async fn scan_ids(executor: &mut DataScanExecutor) -> Vec<i64> {
    let mut ids = Vec::new();
    while let Some(result) = executor.next().await.unwrap() {
        match result {
            ExecResult::Row(values) => match &values[0] {
                Value::Int(id) => ids.push(*id),
                other => panic!("expected Int, got {:?}", other),
            },
            other => panic!("expected Row, got {:?}", other),
        }
    }
    ids
}

/// 复用项目既有 ComparisonPredicate 构造 `id > threshold`
fn id_above(threshold: i64) -> PredicateRef {
    Arc::new(ComparisonPredicate {
        left: Arc::new(ColumnExpression {
            column_name: "id".to_string(),
            column_index: 0,
        }),
        op: ComparisonOp::Gt,
        right: Arc::new(ConstantExpression {
            value: Value::Int(threshold),
        }),
    })
}

/// R3/等价：多页数据集上开启与关闭预取的全表扫描逐行相等
#[tokio::test]
async fn test_prefetch_multipage_scan_equivalence() {
    let (table_mgr, buffer_pool, _dir) = setup_table(ROWS).await;
    let table_meta = table_mgr.get_table("test").await.unwrap();

    // 前提守卫：数据确实跨 ≥3 页，否则等价断言不构成多页链验证
    let pages = count_chain_pages(&buffer_pool, table_meta.data_page_head).await;
    assert!(pages >= 3, "expected >= 3 data pages, got {}", pages);

    let mut prefetch_on =
        DataScanExecutor::new(table_meta.clone(), buffer_pool.clone(), None, None, None)
            .with_prefetch(true);
    let ids_on = scan_ids(&mut prefetch_on).await;

    let mut prefetch_off =
        DataScanExecutor::new(table_meta.clone(), buffer_pool.clone(), None, None, None)
            .with_prefetch(false);
    let ids_off = scan_ids(&mut prefetch_off).await;

    assert_eq!(ids_on.len(), ROWS);
    assert_eq!(
        ids_on, ids_off,
        "prefetch must not change row order or content"
    );

    // 内容守卫：两轮扫描都覆盖全部插入行
    let mut sorted = ids_on;
    sorted.sort_unstable();
    let expected: Vec<i64> = (1..=ROWS as i64).collect();
    assert_eq!(sorted, expected);
}

/// R3/谓词+LIMIT：预取开启时谓词/LIMIT 组合结果与关闭时一致
#[tokio::test]
async fn test_prefetch_predicate_and_limit_equivalence() {
    let (table_mgr, buffer_pool, _dir) = setup_table(ROWS).await;
    let table_meta = table_mgr.get_table("test").await.unwrap();

    // (谓词, scan_cap, 期望行数)
    let cases: Vec<(Option<PredicateRef>, Option<usize>, usize)> = vec![
        (Some(id_above(1000)), None, 1000),     // 谓词命中一半
        (Some(id_above(1999)), Some(7), 1),     // 谓词 + LIMIT（命中数 < cap）
        (None, Some(0), 0),                     // limit=0 → 立即结束
        (None, Some(5), 5),                     // 纯 LIMIT
        (Some(id_above(ROWS as i64)), None, 0), // 谓词全过滤 → 空
    ];

    for (case, (predicate, scan_cap, expected)) in cases.into_iter().enumerate() {
        let mut on = DataScanExecutor::new(
            table_meta.clone(),
            buffer_pool.clone(),
            None,
            predicate.clone(),
            scan_cap,
        )
        .with_prefetch(true);
        let mut off = DataScanExecutor::new(
            table_meta.clone(),
            buffer_pool.clone(),
            None,
            predicate,
            scan_cap,
        )
        .with_prefetch(false);

        let ids_on = scan_ids(&mut on).await;
        let ids_off = scan_ids(&mut off).await;
        assert_eq!(ids_on, ids_off, "case {} mismatch", case);
        assert_eq!(ids_on.len(), expected, "case {} wrong row count", case);
    }
}

/// R3/链尾：空表与单页表在预取开启下正常结束（链尾 PageId(0) 不预取）
#[tokio::test]
async fn test_prefetch_chain_tail_paths() {
    // 空表：data_page_head 无槽位，扫描立即结束
    {
        let (table_mgr, buffer_pool, _dir) = setup_table(0).await;
        let table_meta = table_mgr.get_table("test").await.unwrap();
        let mut executor =
            DataScanExecutor::new(table_meta.clone(), buffer_pool.clone(), None, None, None)
                .with_prefetch(true);
        assert!(scan_ids(&mut executor).await.is_empty());
    }

    // 单页表：链尾 next_page_id == 0，行序与关闭预取一致
    {
        let (table_mgr, buffer_pool, _dir) = setup_table(3).await;
        let table_meta = table_mgr.get_table("test").await.unwrap();
        let pages = count_chain_pages(&buffer_pool, table_meta.data_page_head).await;
        assert_eq!(pages, 1, "3 rows must fit one page");

        let mut on =
            DataScanExecutor::new(table_meta.clone(), buffer_pool.clone(), None, None, None)
                .with_prefetch(true);
        let mut off =
            DataScanExecutor::new(table_meta, buffer_pool, None, None, None).with_prefetch(false);
        assert_eq!(scan_ids(&mut on).await, vec![1, 2, 3]);
        assert_eq!(scan_ids(&mut off).await, vec![1, 2, 3]);
    }
}
