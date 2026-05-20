//! Execution engine - Physical plan execution, async iterator
//!
//! M5: Implement async fn next() -> Result<Option<ExecResult>>

mod plan;
mod result;
mod value;
mod executor_trait;

pub use plan::{DeleteNode, IndexScanNode, InsertNode, PhysicalPlan, ScanNode, UpdateNode};
pub use result::ExecResult;
pub use value::Value;
pub use executor_trait::Executor;
