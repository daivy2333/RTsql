//! Execution engine - Physical plan execution, async iterator
//!
//! M5: Implement async fn next() -> Result<Option<ExecResult>>

mod executor_trait;
mod index_scan;
mod insert;
mod plan;
mod result;
mod scan;
mod value;

pub use executor_trait::Executor;
pub use index_scan::IndexScanExecutor;
pub use insert::InsertExecutor;
pub use plan::{DeleteNode, IndexScanNode, InsertNode, PhysicalPlan, ScanNode, UpdateNode};
pub use result::ExecResult;
pub use scan::ScanExecutor;
pub use value::Value;
