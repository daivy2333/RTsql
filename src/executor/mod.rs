//! Execution engine - Physical plan execution, async iterator
//!
//! M5: Implement async fn next() -> Result<Option<Row>>

mod plan;
mod value;

pub use plan::{PhysicalPlan, ScanNode, IndexScanNode, InsertNode, UpdateNode, DeleteNode};
pub use value::Value;
