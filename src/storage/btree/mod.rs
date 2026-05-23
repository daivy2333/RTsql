// Module inception: `btree` module contains `BTree` type
// This is a standard naming pattern (like std::collections::hash_map::HashMap)
// Renaming would add complexity for minimal benefit
#[allow(clippy::module_inception)]
mod btree;

mod async_loader;
mod index_manager;
mod node;
mod sync_loader;

pub use async_loader::AsyncPageLoader;
pub use btree::{BTree, SplitResult};
pub use index_manager::IndexManager;
pub use node::{
    InternalNode, InternalNodeRef, InternalSplitData, LeafNode, LeafNodeRef, LeafSplitData, Node,
    INTERNAL_NODE, LEAF_NODE,
};
pub use sync_loader::SyncPageLoader;
