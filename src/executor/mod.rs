//! Execution engine - Physical plan execution, async iterator
//!
//! M5: Implement async fn next() -> Result<Option<Row>>

mod plan;
mod value;

pub use plan::{DeleteNode, IndexScanNode, InsertNode, PhysicalPlan, ScanNode, UpdateNode};
pub use value::Value;
