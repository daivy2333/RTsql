//! Profiling module for measuring SQL execution pipeline performance
//!
//! M14 Phase 2 T1: Provides task-local storage for timing measurements

use std::collections::HashMap;
use std::time::Duration;
use tokio::task_local;

task_local! {
    pub static PROFILING_DATA: std::cell::RefCell<HashMap<&'static str, Duration>>;
}

/// Initialize profiling data for current task
pub fn init_profiling() {
    PROFILING_DATA.with(|data| {
        *data.borrow_mut() = HashMap::new();
    });
}

/// Record timing for a specific stage
pub fn record_time(stage: &'static str, duration: Duration) {
    PROFILING_DATA.with(|data| {
        data.borrow_mut().insert(stage, duration);
    });
}

/// Get all recorded timings
pub fn get_timings() -> HashMap<&'static str, Duration> {
    PROFILING_DATA.with(|data| {
        data.borrow_mut().clone()
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