//! Profiling module for measuring SQL execution pipeline performance
//!
//! M14 Phase 2 T1: Provides simple timing measurements for performance analysis

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task_local;

task_local! {
    pub static PROFILING_DATA: Arc<Mutex<HashMap<&'static str, Duration>>>;
}

/// Initialize profiling data for current task
pub fn init_profiling() {
    // Task-local storage is set via scope() in the pipeline
    // This function is a no-op placeholder
}

/// Record timing for a specific stage
pub fn record_time(stage: &'static str, duration: Duration) {
    PROFILING_DATA.with(|data| {
        data.lock().unwrap().insert(stage, duration);
    });
}

/// Get all recorded timings
pub fn get_timings() -> HashMap<&'static str, Duration> {
    PROFILING_DATA.with(|data| {
        data.lock().unwrap().clone()
    })
}

/// Print timings table to stderr
pub fn print_timings(total: Duration) {
    let timings = get_timings();
    let total_us = total.as_micros() as f64;

    eprintln!("Stage                    | Time (µs) | % Total");
    eprintln!("-------------------------|-----------|--------");

    // Sort by time descending for clarity
    let mut sorted: Vec<_> = timings.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));

    for (stage, time) in sorted {
        let time_us = time.as_micros() as f64;
        let percent = (time_us / total_us) * 100.0;
        eprintln!("{:23} | {:9.1} | {:6.1}%", stage, time_us, percent);
    }

    eprintln!("-------------------------|-----------|--------");
    eprintln!("{:23} | {:9.1} | {:6.1}%", "Total", total_us, 100.0);
}

/// Check if profiling is enabled via environment variable
pub fn is_profiling_enabled() -> bool {
    std::env::var("RTSQL_PROFILING").is_ok()
}

/// Create profiling scope for async execution
pub fn with_profiling_scope<F, T>(f: F) -> impl std::future::Future<Output = T>
where
    F: std::future::Future<Output = T>,
{
    PROFILING_DATA.scope(Arc::new(Mutex::new(HashMap::new())), f)
}