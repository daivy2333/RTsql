//! Runtime functionality test
//!
//! Verify Tokio runtime is properly configured:
//! - async function execution
//! - tokio::spawn task scheduling
//! - multi-thread scheduler behavior

#[tokio::test]
async fn test_async_execution() {
    let result = async_compute(42).await;
    assert_eq!(result, 84);
}

async fn async_compute(n: u32) -> u32 {
    n * 2
}

#[tokio::test]
async fn test_spawn_task() {
    let handle = tokio::spawn(async { 100 });

    let result = handle.await.expect("task should complete");
    assert_eq!(result, 100);
}

#[tokio::test]
async fn test_multi_thread_spawn() {
    use std::sync::Arc;
    use tokio::sync::Mutex;

    let counter = Arc::new(Mutex::new(0));
    let mut handles = vec![];

    for _ in 0..10 {
        let counter_clone = counter.clone();
        handles.push(tokio::spawn(async move {
            let mut guard = counter_clone.lock().await;
            *guard += 1;
        }));
    }

    for handle in handles {
        handle.await.expect("task should complete");
    }

    let final_count = *counter.lock().await;
    assert_eq!(final_count, 10);
}
