//! 打印 parse、plan、execute 三段的分配次数和字节数，并核对同一条连接的行与缺表错误。
//! 计数只发生在这个测试进程里，请求仍交给系统分配器。

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};

use rtsql::database::Database;
use rtsql::pipeline::{execute_stage, parse_stage, plan_stage};
use rtsql::profiling::is_profiling_enabled;
use rtsql::{PhysicalPlan, SqlError, SqlOutcome, Value};

static ALLOC_COUNT: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

struct StageAllocator;

unsafe impl GlobalAlloc for StageAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
        }
        new_ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static STAGE_ALLOCATOR: StageAllocator = StageAllocator;

const SELECT_SQL: &str = "SELECT l.k, l.name, r.v FROM l JOIN r ON l.k > r.k";

fn load_counts() -> (u64, u64) {
    (
        ALLOC_COUNT.load(Ordering::Relaxed),
        ALLOC_BYTES.load(Ordering::Relaxed),
    )
}

fn assert_join_row(outcome: &SqlOutcome) {
    match outcome {
        SqlOutcome::Rows { columns, rows } => {
            assert_eq!(
                columns
                    .iter()
                    .map(|column| column.name.as_str())
                    .collect::<Vec<_>>(),
                vec!["k", "name", "v"]
            );
            assert_eq!(
                rows,
                &vec![vec![
                    Value::Int(2),
                    Value::String("b".to_string()),
                    Value::Int(10),
                ]]
            );
        }
        other => panic!("expected one joined row, got {other:?}"),
    }
}

fn assert_missing_table(error: &SqlError) {
    assert_eq!(
        error.message,
        "Table 'missing' not found: table not found: missing"
    );
    assert_eq!(error.sqlstate.as_deref(), Some("42P01"));
}

#[tokio::test]
async fn allocator_stage_counts_and_sql_results() {
    assert!(!is_profiling_enabled());

    let db = Database::open_in_memory().await.expect("open memory database");
    db.execute_sql("CREATE TABLE l (k INT PRIMARY KEY, name TEXT)")
        .await
        .expect("create l");
    db.execute_sql("INSERT INTO l (k, name) VALUES (1, 'a'), (2, 'b')")
        .await
        .expect("insert l");
    db.execute_sql("CREATE TABLE r (k INT PRIMARY KEY, v INT)")
        .await
        .expect("create r");
    db.execute_sql("INSERT INTO r (k, v) VALUES (1, 10), (3, 30)")
        .await
        .expect("insert r");

    let (parse_before_allocs, parse_before_bytes) = load_counts();
    let parsed = parse_stage(SELECT_SQL).await.expect("parse");
    let (parse_after_allocs, parse_after_bytes) = load_counts();
    let stmt = parsed.first().expect("one statement");

    let (plan_before_allocs, plan_before_bytes) = load_counts();
    let plan = plan_stage(&db, SELECT_SQL, stmt, false)
        .await
        .expect("plan");
    let (plan_after_allocs, plan_after_bytes) = load_counts();
    let root_is_nested_loop = matches!(&plan, PhysicalPlan::NestedLoopJoin(_));

    let (execute_before_allocs, execute_before_bytes) = load_counts();
    let staged = execute_stage(&db, plan, false).await.expect("execute");
    let (execute_after_allocs, execute_after_bytes) = load_counts();

    let parse_allocs = parse_after_allocs - parse_before_allocs;
    let parse_bytes = parse_after_bytes - parse_before_bytes;
    let plan_allocs = plan_after_allocs - plan_before_allocs;
    let plan_bytes = plan_after_bytes - plan_before_bytes;
    let execute_allocs = execute_after_allocs - execute_before_allocs;
    let execute_bytes = execute_after_bytes - execute_before_bytes;

    println!(
        "parse_allocs={parse_allocs} parse_bytes={parse_bytes} plan_allocs={plan_allocs} plan_bytes={plan_bytes} execute_allocs={execute_allocs} execute_bytes={execute_bytes}"
    );

    assert!(root_is_nested_loop);
    assert_join_row(&staged);

    let repeated = db.execute_sql(SELECT_SQL).await.expect("execute_sql");
    assert_join_row(&repeated);

    let missing = db
        .execute_sql("SELECT * FROM missing")
        .await
        .expect_err("missing table");
    assert_missing_table(&missing);

    assert!(parse_allocs > 0);
    assert!(parse_bytes > 0);
    assert!(plan_allocs > 0);
    assert!(plan_bytes > 0);
    assert!(execute_allocs > 0);
    assert!(execute_bytes > 0);
}
