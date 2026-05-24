// BTree 核心逻辑（Task 5 实现）
use std::sync::Arc;

use crate::storage::{
    btree::node::{
        InternalNode, InternalNodeRef, LeafNode, LeafNodeRef, INTERNAL_NODE, LEAF_NODE,
    },
    page_format::{Key, RowId},
    PageId, PageGuard, Result, StorageError,
};

use super::{AsyncPageLoader, SyncPageLoader};

const MIN_KEYS: usize = 48;

type LeafEntries = Vec<(Key, RowId)>;
type InternalSeps = Vec<(Key, u32)>;

/// Split 操作的结果（用于 split 传播）
pub struct SplitResult {
    /// 上推到父节点的分割 key
    pub middle_key: Key,
    /// 新分裂出的右页 PageId
    pub new_page_id: PageId,
}

/// Merge 操作的结果（用于 merge 传播）
pub struct MergeInfo {
    pub freed_page_id: PageId,
    pub separator_key: Key,
    pub new_root: Option<PageId>,
}

fn find_child_position(iref: &InternalNodeRef, key: &Key) -> (PageId, usize) {
    let count = iref.key_count();
    if count == 0 {
        return (PageId(iref.leftmost_child() as u64), 0);
    }
    let mut lo: usize = 0;
    let mut hi: usize = count;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if let Some(mid_key) = iref.get_key(mid) {
            match mid_key.cmp(key) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => {
                    let child = iref
                        .get_child_page_id(mid)
                        .unwrap_or(iref.leftmost_child());
                    return (PageId(child as u64), mid + 1);
                }
            }
        } else {
            hi = mid;
        }
    }
    if lo == 0 {
        (PageId(iref.leftmost_child() as u64), 0)
    } else {
        let child = iref
            .get_child_page_id(lo - 1)
            .unwrap_or(iref.leftmost_child());
        (PageId(child as u64), lo)
    }
}

fn sibling_ids(parent_guard: &PageGuard, child_index: usize) -> (Option<PageId>, Option<PageId>) {
    let d = parent_guard.page_data();
    let iref = InternalNodeRef::new(&d);
    let leftmost = iref.leftmost_child();
    let total_children = iref.key_count() + 1;

    let left = if child_index > 0 {
        if child_index == 1 {
            Some(PageId(leftmost as u64))
        } else {
            iref
                .get_child_page_id(child_index - 2)
                .map(|c| PageId(c as u64))
        }
    } else {
        None
    };

    let right = if child_index < total_children - 1 {
        iref
            .get_child_page_id(child_index)
            .map(|c| PageId(c as u64))
    } else {
        None
    };

    (left, right)
}

pub struct BTree {
    loader: Arc<SyncPageLoader>,
    root_page_id: PageId,
}

impl BTree {
    /// Create a new BTree with an empty LeafNode as root
    pub fn new(loader: Arc<SyncPageLoader>) -> Result<Self> {
        // Allocate a page for the root
        let root_page_id = loader.allocate_page()?;

        // Initialize it as an empty LeafNode
        {
            let guard = loader.load_page(root_page_id)?;
            guard.modify_page(|page| {
                LeafNode::init(page);
            });
        }

        Ok(Self {
            loader,
            root_page_id,
        })
    }

    /// Get the root page ID
    pub fn root_page_id(&self) -> PageId {
        self.root_page_id
    }

    /// Create BTree from existing root page (for write operations)
    pub fn from_root(root_page_id: PageId, loader: Arc<SyncPageLoader>) -> Self {
        Self {
            loader,
            root_page_id,
        }
    }

    /// Search for a key in the BTree
    /// Returns the RowId if found, None if not found
    pub fn search(&self, key: &[u8]) -> Result<Option<RowId>> {
        let key_obj = Key::new(key);
        self.search_from_page(self.root_page_id, &key_obj)
    }

    /// Recursive search from a page
    fn search_from_page(&self, page_id: PageId, key: &Key) -> Result<Option<RowId>> {
        let guard = self.loader.load_page(page_id)?;
        let data_guard = guard.page_data();

        if data_guard[0] == LEAF_NODE {
            let leaf = LeafNodeRef::new(&data_guard);
            let (found, pos) = leaf.find_key_position_binary(key);
            if found {
                Ok(leaf.get_row_id(pos))
            } else {
                Ok(None)
            }
        } else {
            let internal = InternalNodeRef::new(&data_guard);
            let child_page_id = internal.find_child_page_id_binary(key);
            drop(data_guard);
            drop(guard);
            self.search_from_page(PageId(child_page_id as u64), key)
        }
    }

    /// Async search — direct async path without spawn_blocking/block_on
    pub async fn search_async(&self, key: &Key, loader: &AsyncPageLoader) -> Result<Option<RowId>> {
        self.search_from_page_async(self.root_page_id, key, loader)
            .await
    }

    #[allow(clippy::only_used_in_recursion)]
    fn search_from_page_async<'a>(
        &'a self,
        page_id: PageId,
        key: &'a Key,
        loader: &'a AsyncPageLoader,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<RowId>>> + Send + 'a>>
    {
        Box::pin(async move {
            let child_page_id = {
                let guard = loader.load_page(page_id).await?;
                let data_guard = guard.page_data();

                if data_guard[0] == LEAF_NODE {
                    let leaf = LeafNodeRef::new(&data_guard);
                    let (found, pos) = leaf.find_key_position_binary(key);
                    if found {
                        return Ok(leaf.get_row_id(pos));
                    } else {
                        return Ok(None);
                    }
                } else {
                    let internal = InternalNodeRef::new(&data_guard);
                    internal.find_child_page_id_binary(key)
                }
            }; // guard and data_guard dropped here, before recursive await

            self.search_from_page_async(PageId(child_page_id as u64), key, loader)
                .await
        })
    }

    /// Insert a key and RowId into the BTree.
    /// Returns Ok(Some(new_root_page_id)) if a root split occurred (caller should update root),
    /// Returns Ok(None) if insertion succeeded without root split.
    pub fn insert(&mut self, key: &[u8], row_id: RowId) -> Result<Option<PageId>> {
        let key_obj = Key::new(key);

        let split = self.insert_into_page(self.root_page_id, &key_obj, &row_id)?;

        if let Some(split_result) = split {
            // Root split: create a new root InternalNode
            let new_root_page_id = self.loader.allocate_page()?;
            let guard = self.loader.load_page(new_root_page_id)?;
            guard.modify_page(|page| {
                let mut new_root = InternalNode::init(page);
                // Set leftmost_child to the old root page id
                new_root.set_leftmost_child(self.root_page_id.0 as u32);
                // Insert separator for the split
                new_root.insert_separator(&split_result.middle_key, split_result.new_page_id)?;
                Ok::<(), StorageError>(())
            })?;

            self.root_page_id = new_root_page_id;
            Ok(Some(new_root_page_id))
        } else {
            Ok(None)
        }
    }

    /// Recursive insert into a page. Returns Some(SplitResult) if the page split,
    /// None if insertion completed without split.
    fn insert_into_page(
        &self,
        page_id: PageId,
        key: &Key,
        row_id: &RowId,
    ) -> Result<Option<SplitResult>> {
        let guard = self.loader.load_page(page_id)?;

        // Determine node type using zero-copy read
        let is_leaf = {
            let data_guard = guard.page_data();
            data_guard[0] == LEAF_NODE
        };

        if is_leaf {
            // Try direct insert first
            let insert_result: std::result::Result<usize, StorageError> =
                guard.modify_page(|page| {
                    let mut leaf = LeafNode::from_page(page)?;
                    leaf.insert(key, row_id)
                });

            match insert_result {
                Ok(_) => Ok(None), // Insert succeeded, no split needed
                Err(StorageError::PageFull) => {
                    // Need to split the leaf node
                    let new_page_id = self.loader.allocate_page()?;

                    // 1. Split the original leaf (rebuilds it with left half)
                    let leaf_split = guard.modify_page(|page| {
                        let mut leaf = LeafNode::from_page(page)?;
                        leaf.split(new_page_id)
                    })?;

                    // 2. Re-insert the new entry into the appropriate half after split
                    let key_in_right = key >= &leaf_split.middle_key;

                    if !key_in_right {
                        // Key belongs to the left (original) half
                        // Use insert (not insert_simple) to maintain sorted order
                        guard.modify_page(|page| {
                            let mut leaf = LeafNode::from_page(page)?;
                            leaf.insert(key, row_id)?;
                            Ok::<(), StorageError>(())
                        })?;
                    }

                    // 3. Initialize the new leaf page with right entries
                    //    (and include the new entry if it belongs to the right half)
                    let new_guard = self.loader.load_page(new_page_id)?;
                    new_guard.modify_page(|page| {
                        let mut new_leaf = LeafNode::init(page);

                        // Collect all entries for the right page (sorted + new entry if applicable)
                        let mut right_with_new = leaf_split.right_entries.clone();
                        if key_in_right {
                            right_with_new.push((key.clone(), *row_id));
                            right_with_new.sort_by(|(k1, _), (k2, _)| k1.cmp(k2));
                        }

                        for (k, r) in &right_with_new {
                            new_leaf.insert_simple(k, r)?;
                        }
                        // Maintain linked list: new page's next = old page's old next
                        new_leaf.set_next_leaf_page_id(leaf_split.old_next_page_id);
                        Ok::<(), StorageError>(())
                    })?;

                    // 4. Update original page's next pointer to the new page
                    guard.modify_page(|page| {
                        let mut leaf = LeafNode::from_page(page)?;
                        leaf.set_next_leaf_page_id(new_page_id.0 as u32);
                        Ok::<(), StorageError>(())
                    })?;

                    Ok(Some(SplitResult {
                        middle_key: leaf_split.middle_key,
                        new_page_id,
                    }))
                }
                Err(e) => Err(e), // Other error, propagate
            }
        } else {
            // Internal node: find the child to recurse into
            let child_page_id_u32 = {
                let data_guard = guard.page_data();
                let internal_ref = InternalNodeRef::new(&data_guard);
                internal_ref.find_child_page_id_binary(key)
            };
            let child_page_id = PageId(child_page_id_u32 as u64);

            // Recursively insert into child
            let child_split = self.insert_into_page(child_page_id, key, row_id)?;

            if let Some(child_split_result) = child_split {
                // Child split: need to insert a separator into this internal node
                let separator_insert_result: std::result::Result<usize, StorageError> = guard
                    .modify_page(|page| {
                        let mut internal = InternalNode::from_page(page)?;
                        internal.insert_separator(
                            &child_split_result.middle_key,
                            child_split_result.new_page_id,
                        )
                    });

                match separator_insert_result {
                    Ok(_) => Ok(None), // Separator inserted successfully
                    Err(StorageError::PageFull) => {
                        // Internal node is also full, need to split it
                        let new_page_id = self.loader.allocate_page()?;

                        // 1. Split the original internal node
                        let internal_split = guard.modify_page(|page| {
                            let mut internal = InternalNode::from_page(page)?;
                            internal.split(new_page_id)
                        })?;

                        // 2. Re-insert the pending separator into the appropriate half
                        //    The separator that caused PageFull was child_split_result.middle_key.
                        let sep_in_right =
                            child_split_result.middle_key >= internal_split.middle_key;

                        if !sep_in_right {
                            // Separator belongs to the left (original) half
                            guard.modify_page(|page| {
                                let mut internal = InternalNode::from_page(page)?;
                                internal.insert_separator(
                                    &child_split_result.middle_key,
                                    child_split_result.new_page_id,
                                )?;
                                Ok::<(), StorageError>(())
                            })?;
                        }

                        // 3. Initialize the new internal node page
                        //    (and include the pending separator if it belongs to the right half)
                        let new_guard = self.loader.load_page(new_page_id)?;
                        new_guard.modify_page(|page| {
                            let mut new_internal = InternalNode::init(page);
                            // Set leftmost_child
                            new_internal.set_leftmost_child(internal_split.new_leftmost_child);
                            // Insert right separators
                            for (k, child_id) in &internal_split.right_separators {
                                new_internal
                                    .insert_separator_simple(k, PageId(*child_id as u64))?;
                            }
                            if sep_in_right {
                                new_internal.insert_separator(
                                    &child_split_result.middle_key,
                                    child_split_result.new_page_id,
                                )?;
                            }
                            Ok::<(), StorageError>(())
                        })?;

                        Ok(Some(SplitResult {
                            middle_key: internal_split.middle_key,
                            new_page_id,
                        }))
                    }
                    Err(e) => Err(e), // Other error, propagate
                }
            } else {
                // Child did not split, insertion complete
                Ok(None)
            }
        }
    }

    pub fn delete(&mut self, key: &[u8]) -> Result<Option<PageId>> {
        let key_obj = Key::new(key);
        let merge_info = self.delete_from_page(self.root_page_id, &key_obj, None, None)?;
        if let Some(mi) = merge_info {
            if let Some(nr) = mi.new_root {
                self.root_page_id = nr;
            }
            Ok(mi.new_root)
        } else {
            Ok(None)
        }
    }

    fn delete_from_page(
        &self,
        page_id: PageId,
        key: &Key,
        parent_page_id: Option<PageId>,
        child_index_in_parent: Option<usize>,
    ) -> Result<Option<MergeInfo>> {
        let guard = self.loader.load_page(page_id)?;
        let is_leaf = { let d = guard.page_data(); d[0] == LEAF_NODE };
        drop(guard);

        if is_leaf {
            self.delete_from_leaf(page_id, key, parent_page_id, child_index_in_parent)
        } else {
            self.delete_from_internal(page_id, key, parent_page_id, child_index_in_parent)
        }
    }

    fn delete_from_leaf(
        &self,
        page_id: PageId,
        key: &Key,
        parent_page_id: Option<PageId>,
        child_index_in_parent: Option<usize>,
    ) -> Result<Option<MergeInfo>> {
        let guard = self.loader.load_page(page_id)?;
        guard.modify_page(|page_mut| {
            let mut leaf = LeafNode::from_page(page_mut)?;
            leaf.delete(key)?;
            Ok::<(), StorageError>(())
        })?;

        let guard = self.loader.load_page(page_id)?;
        let underflow = {
            let d = guard.page_data();
            LeafNodeRef::new(&d).key_count() < MIN_KEYS
        };
        drop(guard);

        if !underflow {
            return Ok(None);
        }
        let parent_id = match parent_page_id {
            Some(p) => p,
            None => return Ok(None),
        };
        let child_index = child_index_in_parent.unwrap();
        let mi = self.handle_leaf_underflow(page_id, child_index, parent_id)?;
        Ok(mi)
    }

    fn handle_leaf_underflow(
        &self,
        page_id: PageId,
        child_index: usize,
        parent_id: PageId,
    ) -> Result<Option<MergeInfo>> {
        let parent_guard = self.loader.load_page(parent_id)?;
        let (left_sib, right_sib) = sibling_ids(&parent_guard, child_index);
        drop(parent_guard);

        if let Some(right_id) = right_sib {
            if self.leaf_key_count(right_id)? > MIN_KEYS {
                return self.redistribute_leaf_right(page_id, right_id, parent_id, child_index);
            }
        }
        if let Some(left_id) = left_sib {
            if self.leaf_key_count(left_id)? > MIN_KEYS {
                return self.redistribute_leaf_left(left_id, page_id, parent_id, child_index);
            }
        }

        if let Some(left_id) = left_sib {
            Ok(Some(self.merge_leaves(left_id, page_id, parent_id, child_index)?))
        } else if let Some(right_id) = right_sib {
            Ok(Some(self.merge_leaves(page_id, right_id, parent_id, child_index + 1)?))
        } else {
            let guard = self.loader.load_page(parent_id)?;
            let (is_internal, parent_kc) = {
                let d = guard.page_data();
                (d[0] == INTERNAL_NODE, InternalNodeRef::new(&d).key_count())
            };
            drop(guard);
            if is_internal && parent_kc == 0 {
                Ok(Some(MergeInfo {
                    freed_page_id: PageId(0),
                    separator_key: Key::new(b""),
                    new_root: Some(page_id),
                }))
            } else {
                Err(StorageError::Io(std::io::Error::other(
                    "leaf underflow with no siblings",
                )))
            }
        }
    }

    fn leaf_key_count(&self, page_id: PageId) -> Result<usize> {
        let guard = self.loader.load_page(page_id)?;
        let d = guard.page_data();
        Ok(LeafNodeRef::new(&d).key_count())
    }

    fn internal_key_count(&self, page_id: PageId) -> Result<usize> {
        let guard = self.loader.load_page(page_id)?;
        let d = guard.page_data();
        Ok(InternalNodeRef::new(&d).key_count())
    }

    fn redistribute_leaf_right(
        &self,
        left_id: PageId,
        right_id: PageId,
        parent_id: PageId,
        child_index: usize,
    ) -> Result<Option<MergeInfo>> {
        let (left_entries, right_entries) = self.read_leaf_pair(left_id, right_id)?;
        let mut entries = left_entries;
        entries.extend(right_entries);
        entries.sort_by(|(a, _), (b, _)| a.cmp(b));
        let mid = entries.len() / 2;
        let (left_share, right_share) = (entries[..mid].to_vec(), entries[mid..].to_vec());
        let new_sep = right_share.first().unwrap().0.clone();
        self.rebuild_leaf(left_id, &left_share)?;
        self.rebuild_leaf(right_id, &right_share)?;
        self.update_parent_separator(parent_id, child_index, &new_sep, right_id)?;
        Ok(None)
    }

    fn redistribute_leaf_left(
        &self,
        left_id: PageId,
        right_id: PageId,
        parent_id: PageId,
        child_index: usize,
    ) -> Result<Option<MergeInfo>> {
        let (left_entries, right_entries) = self.read_leaf_pair(left_id, right_id)?;
        let mut entries = left_entries;
        entries.extend(right_entries);
        entries.sort_by(|(a, _), (b, _)| a.cmp(b));
        let mid = entries.len() / 2;
        let (left_share, right_share) = (entries[..mid].to_vec(), entries[mid..].to_vec());
        let new_sep = right_share.first().unwrap().0.clone();
        self.rebuild_leaf(left_id, &left_share)?;
        self.rebuild_leaf(right_id, &right_share)?;
        self.update_parent_separator(parent_id, child_index - 1, &new_sep, right_id)?;
        Ok(None)
    }

    fn merge_leaves(
        &self,
        left_id: PageId,
        right_id: PageId,
        parent_id: PageId,
        right_child_index: usize,
    ) -> Result<MergeInfo> {
        let right_guard = self.loader.load_page(right_id)?;
        let mut rd = vec![0u8; 4096];
        rd.copy_from_slice(right_guard.page().data.as_ref());
        let rref = LeafNodeRef::new(&rd);
        let right_next = rref.next_leaf_page_id();
        let r_entries: LeafEntries = (0..rref.key_count())
            .filter_map(|i| Some((rref.get_key(i)?, rref.get_row_id(i)?)))
            .collect();
        drop(right_guard);

        let sep_key = r_entries.first().map(|(k, _)| k.clone())
            .unwrap_or(Key::new(b""));

        let left_guard = self.loader.load_page(left_id)?;
        left_guard.modify_page(|page_mut| {
            let mut leaf = LeafNode::from_page(page_mut)?;
            for (k, r) in &r_entries {
                leaf.insert(k, r)?;
            }
            leaf.set_next_leaf_page_id(right_next);
            Ok::<(), StorageError>(())
        })?;
        self.loader.free_page(right_id)?;

        let sep_slot = right_child_index - 1;
        let parent_guard = self.loader.load_page(parent_id)?;
        parent_guard.modify_page(|page_mut| {
            let mut internal = InternalNode::from_page(page_mut)?;
            internal.remove_separator(sep_slot)?;
            Ok::<(), StorageError>(())
        })?;

        Ok(MergeInfo { freed_page_id: right_id, separator_key: sep_key, new_root: None })
    }

    fn delete_from_internal(
        &self,
        page_id: PageId,
        key: &Key,
        parent_page_id: Option<PageId>,
        child_index_in_parent: Option<usize>,
    ) -> Result<Option<MergeInfo>> {
        let (child_page_id, child_index) = {
            let guard = self.loader.load_page(page_id)?;
            let d = guard.page_data();
            find_child_position(&InternalNodeRef::new(&d), key)
        };

        match self.delete_from_page(child_page_id, key, Some(page_id), Some(child_index))? {
            None => Ok(None),
            Some(mi) => {
                if mi.new_root.is_some() {
                    return Ok(Some(mi));
                }
                self.handle_child_merge(page_id, mi, parent_page_id, child_index_in_parent)
            }
        }
    }

    fn handle_child_merge(
        &self,
        page_id: PageId,
        merge_info: MergeInfo,
        parent_page_id: Option<PageId>,
        child_index_in_parent: Option<usize>,
    ) -> Result<Option<MergeInfo>> {
        let sep_slot = self.find_separator_slot(page_id, &merge_info.separator_key)?;
        if let Some(slot) = sep_slot {
            let guard = self.loader.load_page(page_id)?;
            guard.modify_page(|page_mut| {
                let mut internal = InternalNode::from_page(page_mut)?;
                internal.remove_separator(slot)?;
                Ok::<(), StorageError>(())
            })?;
        }
        if merge_info.freed_page_id.0 != 0 {
            self.loader.free_page(merge_info.freed_page_id)?;
        }

        let guard = self.loader.load_page(page_id)?;
        let needs_action = {
            let d = guard.page_data();
            let iref = InternalNodeRef::new(&d);
            match parent_page_id {
                None => iref.key_count() == 0,
                Some(_) => iref.key_count() < MIN_KEYS,
            }
        };
        drop(guard);

        if !needs_action {
            return Ok(None);
        }

        match parent_page_id {
            None => {
                let nr = self.shrink_root(page_id)?;
                Ok(Some(MergeInfo {
                    freed_page_id: PageId(0),
                    separator_key: merge_info.separator_key,
                    new_root: nr,
                }))
            }
            Some(pid) => {
                let ci = child_index_in_parent.unwrap();
                let mi = self.handle_internal_underflow(page_id, ci, pid)?;
                Ok(mi)
            }
        }
    }

    fn find_separator_slot(&self, page_id: PageId, target: &Key) -> Result<Option<usize>> {
        let guard = self.loader.load_page(page_id)?;
        let d = guard.page_data();
        let iref = InternalNodeRef::new(&d);
        for i in 0..iref.key_count() {
            if let Some(k) = iref.get_key(i) {
                if k == *target {
                    return Ok(Some(i));
                }
            }
        }
        Ok(None)
    }

    fn handle_internal_underflow(
        &self,
        page_id: PageId,
        child_index: usize,
        parent_id: PageId,
    ) -> Result<Option<MergeInfo>> {
        let parent_guard = self.loader.load_page(parent_id)?;
        let (left_sib, right_sib) = sibling_ids(&parent_guard, child_index);
        drop(parent_guard);

        if let Some(right_id) = right_sib {
            if self.internal_key_count(right_id)? > MIN_KEYS {
                return self.redistribute_internal_right(page_id, right_id, parent_id, child_index);
            }
        }
        if let Some(left_id) = left_sib {
            if self.internal_key_count(left_id)? > MIN_KEYS {
                return self.redistribute_internal_left(left_id, page_id, parent_id, child_index);
            }
        }

        if let Some(left_id) = left_sib {
            Ok(Some(self.merge_internal_nodes(left_id, page_id, parent_id, child_index)?))
        } else if let Some(right_id) = right_sib {
            Ok(Some(self.merge_internal_nodes(page_id, right_id, parent_id, child_index + 1)?))
        } else {
            Err(StorageError::Io(std::io::Error::other(
                "internal underflow with no siblings",
            )))
        }
    }

    fn shrink_root(&self, root_id: PageId) -> Result<Option<PageId>> {
        let guard = self.loader.load_page(root_id)?;
        let (is_internal, kc, leftmost) = {
            let d = guard.page_data();
            if d[0] == INTERNAL_NODE {
                let iref = InternalNodeRef::new(&d);
                (true, iref.key_count(), iref.leftmost_child())
            } else {
                (false, 0, 0)
            }
        };
        drop(guard);

        if is_internal && kc == 0 {
            let new_root = PageId(leftmost as u64);
            self.loader.free_page(root_id)?;
            Ok(Some(new_root))
        } else {
            Ok(None)
        }
    }

    fn redistribute_internal_right(
        &self,
        left_id: PageId,
        right_id: PageId,
        parent_id: PageId,
        left_child_index: usize,
    ) -> Result<Option<MergeInfo>> {
        let parent_sep = self.get_parent_separator_key(parent_id, left_child_index)?;
        let (left_seps, l_lm) = self.read_internal_seps(left_id)?;
        let (right_seps, r_lm) = self.read_internal_seps(right_id)?;

        let mut all = left_seps;
        if let Some(pk) = parent_sep {
            all.push((pk, r_lm));
        }
        all.extend(right_seps);
        all.sort_by(|(a, _), (b, _)| a.cmp(b));

        let mid = all.len() / 2;
        let left_share = all[..mid].to_vec();
        let new_mid = all[mid].0.clone();
        let right_share = all[mid + 1..].to_vec();
        let new_r_lm = all[mid].1;

        self.rebuild_internal(left_id, l_lm, &left_share)?;
        self.rebuild_internal(right_id, new_r_lm, &right_share)?;
        self.update_parent_separator(parent_id, left_child_index, &new_mid, right_id)?;

        Ok(None)
    }

    fn redistribute_internal_left(
        &self,
        left_id: PageId,
        right_id: PageId,
        parent_id: PageId,
        right_child_index: usize,
    ) -> Result<Option<MergeInfo>> {
        let parent_sep = self.get_parent_separator_key(parent_id, right_child_index - 1)?;
        let (left_seps, l_lm) = self.read_internal_seps(left_id)?;
        let (right_seps, r_lm) = self.read_internal_seps(right_id)?;

        let mut all = left_seps;
        if let Some(pk) = parent_sep {
            all.push((pk, r_lm));
        }
        all.extend(right_seps);
        all.sort_by(|(a, _), (b, _)| a.cmp(b));

        let mid = all.len() / 2;
        let left_share = all[..mid].to_vec();
        let new_mid = all[mid].0.clone();
        let right_share = all[mid + 1..].to_vec();
        let new_r_lm = all[mid].1;

        self.rebuild_internal(left_id, l_lm, &left_share)?;
        self.rebuild_internal(right_id, new_r_lm, &right_share)?;
        self.update_parent_separator(parent_id, right_child_index - 1, &new_mid, right_id)?;

        Ok(None)
    }

    fn merge_internal_nodes(
        &self,
        left_id: PageId,
        right_id: PageId,
        parent_id: PageId,
        right_child_index: usize,
    ) -> Result<MergeInfo> {
        let parent_sep = self.get_parent_separator_key(parent_id, right_child_index - 1)?;
        let (left_seps, l_lm) = self.read_internal_seps(left_id)?;
        let (right_seps, r_lm) = self.read_internal_seps(right_id)?;

        let merged_sep = left_seps
            .first()
            .map(|(k, _)| k.clone())
            .or_else(|| parent_sep.clone())
            .unwrap();

        let mut all = left_seps;
        if let Some(pk) = parent_sep {
            all.push((pk, r_lm));
        }
        all.extend(right_seps);
        all.sort_by(|(a, _), (b, _)| a.cmp(b));

        self.rebuild_internal(left_id, l_lm, &all)?;
        self.loader.free_page(right_id)?;

        let sep_slot = right_child_index - 1;
        let parent_guard = self.loader.load_page(parent_id)?;
        parent_guard.modify_page(|page_mut| {
            let mut internal = InternalNode::from_page(page_mut)?;
            internal.remove_separator(sep_slot)?;
            Ok::<(), StorageError>(())
        })?;

        Ok(MergeInfo { freed_page_id: right_id, separator_key: merged_sep, new_root: None })
    }

    fn get_parent_separator_key(&self, parent_id: PageId, slot: usize) -> Result<Option<Key>> {
        let guard = self.loader.load_page(parent_id)?;
        let d = guard.page_data();
        Ok(InternalNodeRef::new(&d).get_key(slot))
    }

    fn update_parent_separator(
        &self,
        parent_id: PageId,
        slot: usize,
        new_key: &Key,
        new_child: PageId,
    ) -> Result<()> {
        let guard = self.loader.load_page(parent_id)?;
        guard.modify_page(|page_mut| {
            let mut internal = InternalNode::from_page(page_mut)?;
            internal.remove_separator(slot)?;
            internal.insert_separator(new_key, new_child)?;
            Ok::<(), StorageError>(())
        })?;
        Ok(())
    }



    fn read_leaf_pair(&self, left_id: PageId, right_id: PageId) -> Result<(LeafEntries, LeafEntries)> {
        let lg = self.loader.load_page(left_id)?;
        let rg = self.loader.load_page(right_id)?;
        let mut ld = vec![0u8; 4096];
        ld.copy_from_slice(lg.page().data.as_ref());
        let mut rd = vec![0u8; 4096];
        rd.copy_from_slice(rg.page().data.as_ref());
        drop(lg);
        drop(rg);
        let lref = LeafNodeRef::new(&ld);
        let rref = LeafNodeRef::new(&rd);
        let left = (0..lref.key_count())
            .filter_map(|i| Some((lref.get_key(i)?, lref.get_row_id(i)?)))
            .collect();
        let right = (0..rref.key_count())
            .filter_map(|i| Some((rref.get_key(i)?, rref.get_row_id(i)?)))
            .collect();
        Ok((left, right))
    }

    fn rebuild_leaf(&self, page_id: PageId, entries: &LeafEntries) -> Result<()> {
        let guard = self.loader.load_page(page_id)?;
        let next_leaf = {
            let d = guard.page_data();
            LeafNodeRef::new(&d).next_leaf_page_id()
        };
        guard.modify_page(|page_mut| {
            let mut leaf = LeafNode::init(page_mut);
            for (k, r) in entries {
                leaf.insert_simple(k, r)?;
            }
            leaf.set_next_leaf_page_id(next_leaf);
            Ok::<(), StorageError>(())
        })?;
        Ok(())
    }

    fn read_internal_seps(&self, page_id: PageId) -> Result<(InternalSeps, u32)> {
        let guard = self.loader.load_page(page_id)?;
        let mut data = vec![0u8; 4096];
        data.copy_from_slice(guard.page().data.as_ref());
        drop(guard);
        let iref = InternalNodeRef::new(&data);
        let leftmost = iref.leftmost_child();
        let seps = (0..iref.key_count())
            .filter_map(|i| Some((iref.get_key(i)?, iref.get_child_page_id(i)?)))
            .collect();
        Ok((seps, leftmost))
    }

    fn rebuild_internal(&self, page_id: PageId, leftmost: u32, seps: &InternalSeps) -> Result<()> {
        let guard = self.loader.load_page(page_id)?;
        guard.modify_page(|page_mut| {
            let mut internal = InternalNode::init(page_mut);
            internal.set_leftmost_child(leftmost);
            for (k, c) in seps {
                internal.insert_separator_simple(k, PageId(*c as u64))?;
            }
            Ok::<(), StorageError>(())
        })?;
        Ok(())
    }

    /// Scan all entries in the BTree by iterating through all leaf nodes.
    /// Returns all (Key, RowId) pairs in key order.
    pub fn scan_all(&self) -> Result<Vec<(Key, RowId)>> {
        let mut results = Vec::new();
        let mut page_id = self.root_page_id;
        let mut first_iteration = true;

        loop {
            let guard = self.loader.load_page(page_id)?;
            let data_guard = guard.page_data();

            // Check if it's a leaf node
            if data_guard[0] != LEAF_NODE {
                // Internal node: follow leftmost child
                let internal = InternalNodeRef::new(&data_guard);
                let child_id = internal.leftmost_child();
                drop(data_guard);
                drop(guard);
                page_id = PageId(child_id as u64);
                continue;
            }

            let leaf = LeafNodeRef::new(&data_guard);
            let count = leaf.key_count();
            let mut entries = Vec::with_capacity(count);
            for i in 0..count {
                if let (Some(key), Some(row_id)) = (leaf.get_key(i), leaf.get_row_id(i)) {
                    entries.push((key, row_id));
                }
            }

            let next_page_u32 = leaf.next_leaf_page_id();
            drop(data_guard);
            drop(guard);

            results.extend(entries);

            // Stop after first leaf if no next_leaf_page_id (single leaf tree)
            if next_page_u32 == 0 && !first_iteration {
                break;
            }
            first_iteration = false;

            // Move to next leaf, or stop if next_page_id is invalid (0 after first iteration)
            if next_page_u32 == 0 {
                break;
            }
            page_id = PageId(next_page_u32 as u64);
        }

        Ok(results)
    }

    /// Update the RowId for an existing key
    /// Returns error if key not found (KeyNotFound)
    pub fn update(&self, key: &[u8], new_row_id: RowId) -> Result<()> {
        let key_obj = Key::new(key);
        self.update_in_page(self.root_page_id, &key_obj, &new_row_id)
    }

    /// Update in a page
    fn update_in_page(&self, page_id: PageId, key: &Key, new_row_id: &RowId) -> Result<()> {
        let guard = self.loader.load_page(page_id)?;
        let page = guard.page();

        if page.data[0] == LEAF_NODE {
            // Leaf node: find and update
            let guard2 = self.loader.load_page(page_id)?;
            guard2.modify_page(|page_mut| {
                let mut leaf = LeafNode::from_page(page_mut)?;
                leaf.update(key, new_row_id)?;
                Ok(())
            })
        } else {
            // Internal node: find child and recurse
            // Simplified: not implemented yet
            Err(StorageError::Io(std::io::Error::other(
                "Internal node update not implemented yet",
            )))
        }
    }

    /// 返回所有匹配 key 的 RowId（用于非唯一索引）
    pub fn search_all(&self, key: &[u8]) -> Result<Vec<RowId>> {
        let key_obj = Key::new(key);
        self.search_all_from_page(self.root_page_id, &key_obj)
    }

    fn search_all_from_page(&self, page_id: PageId, key: &Key) -> Result<Vec<RowId>> {
        let guard = self.loader.load_page(page_id)?;
        let data_guard = guard.page_data();

        if data_guard[0] == LEAF_NODE {
            let leaf = LeafNodeRef::new(&data_guard);
            let matches = leaf.find_all_matches(key);
            let mut row_ids = Vec::new();
            for idx in matches {
                if let Some(rid) = leaf.get_row_id(idx) {
                    row_ids.push(rid);
                }
            }

            Ok(row_ids)
        } else {
            // Internal node: for non-unique keys that equal a separator, we need to
            // search both the left and right subtrees.
            let internal = InternalNodeRef::new(&data_guard);
            let count = internal.key_count();

            // Find if the key matches any separator
            let mut separator_match_idx: Option<usize> = None;
            for i in 0..count {
                if let Some(sep_key) = internal.get_key(i) {
                    if sep_key == *key {
                        separator_match_idx = Some(i);
                        break;
                    }
                }
            }

            if let Some(idx) = separator_match_idx {
                // Key matches a separator — search both left subtree (get_child_page_id(idx-1) or leftmost)
                // and right subtree (get_child_page_id(idx))
                let left_child = if idx == 0 {
                    internal.leftmost_child()
                } else {
                    internal
                        .get_child_page_id(idx - 1)
                        .unwrap_or(internal.leftmost_child())
                };
                let right_child = internal
                    .get_child_page_id(idx)
                    .unwrap_or(internal.leftmost_child());

                drop(data_guard);
                drop(guard);

                let mut results = self.search_all_from_page(PageId(left_child as u64), key)?;
                let right_results = self.search_all_from_page(PageId(right_child as u64), key)?;
                results.extend(right_results);
                Ok(results)
            } else {
                // Key doesn't match any separator — route normally
                let child_page_id = internal.find_child_page_id_binary(key);
                drop(data_guard);
                drop(guard);
                self.search_all_from_page(PageId(child_page_id as u64), key)
            }
        }
    }

    /// 删除所有匹配 key 的 entries，返回删除数量（用于非唯一索引）
    pub fn delete_by_key(&mut self, key: &[u8]) -> Result<(usize, Option<PageId>)> {
        let key_obj = Key::new(key);
        let count = self.delete_all_from_page(self.root_page_id, &key_obj)?;
        Ok((count, None))
    }

    fn delete_all_from_page(&self, page_id: PageId, key: &Key) -> Result<usize> {
        let guard = self.loader.load_page(page_id)?;
        let data_guard = guard.page_data();

        if data_guard[0] == LEAF_NODE {
            let leaf_ref = LeafNodeRef::new(&data_guard);
            let matches = leaf_ref.find_all_matches(key);
            let count = matches.len();
            drop(data_guard);
            drop(guard);

            if count > 0 {
                let guard2 = self.loader.load_page(page_id)?;
                guard2.modify_page(|page_mut| {
                    let mut leaf = LeafNode::from_page(page_mut)?;
                    for idx in matches.into_iter().rev() {
                        leaf.delete_slot(idx)?;
                    }
                    Ok::<(), StorageError>(())
                })?;
            }

            Ok(count)
        } else {
            let internal = InternalNodeRef::new(&data_guard);
            let kc = internal.key_count();

            let mut separator_match_idx: Option<usize> = None;
            for i in 0..kc {
                if let Some(sep_key) = internal.get_key(i) {
                    if sep_key == *key {
                        separator_match_idx = Some(i);
                        break;
                    }
                }
            }

            if let Some(idx) = separator_match_idx {
                let left_child = if idx == 0 {
                    internal.leftmost_child()
                } else {
                    internal.get_child_page_id(idx - 1).unwrap_or(internal.leftmost_child())
                };
                let right_child = internal.get_child_page_id(idx).unwrap_or(internal.leftmost_child());

                drop(data_guard);
                drop(guard);

                let left_count = self.delete_all_from_page(PageId(left_child as u64), key)?;
                let right_count = self.delete_all_from_page(PageId(right_child as u64), key)?;
                Ok(left_count + right_count)
            } else {
                let child_page_id = internal.find_child_page_id_binary(key);
                drop(data_guard);
                drop(guard);
                self.delete_all_from_page(PageId(child_page_id as u64), key)
            }
        }
    }

    pub fn delete_exact(&mut self, key: &[u8], row_id: RowId) -> Result<Option<PageId>> {
        let key_obj = Key::new(key);
        self.delete_exact_from_page(self.root_page_id, &key_obj, &row_id)?;
        Ok(None)
    }

    fn delete_exact_from_page(&self, page_id: PageId, key: &Key, row_id: &RowId) -> Result<()> {
        let guard = self.loader.load_page(page_id)?;
        let data_guard = guard.page_data();

        if data_guard[0] == LEAF_NODE {
            let leaf_ref = LeafNodeRef::new(&data_guard);
            let matches = leaf_ref.find_all_matches(key);

            let target_idx = matches
                .iter()
                .find(|idx| leaf_ref.get_row_id(**idx) == Some(*row_id))
                .copied();

            let key_has_matches = !matches.is_empty();
            let next_page = if key_has_matches { leaf_ref.next_leaf_page_id() } else { 0 };

            drop(data_guard);
            drop(guard);

            if let Some(idx) = target_idx {
                let guard2 = self.loader.load_page(page_id)?;
                guard2.modify_page(|page_mut| {
                    let mut leaf = LeafNode::from_page(page_mut)?;
                    leaf.delete_slot(idx)?;
                    Ok::<(), StorageError>(())
                })?;
                Ok(())
            } else if key_has_matches && next_page != 0 {
                self.delete_exact_from_page(PageId(next_page as u64), key, row_id)
            } else {
                Err(StorageError::KeyNotFound)
            }
        } else {
            let internal = InternalNodeRef::new(&data_guard);
            let kc = internal.key_count();

            let mut separator_match_idx: Option<usize> = None;
            for i in 0..kc {
                if let Some(sep_key) = internal.get_key(i) {
                    if sep_key == *key {
                        separator_match_idx = Some(i);
                        break;
                    }
                }
            }

            if let Some(idx) = separator_match_idx {
                let left_child = if idx == 0 {
                    internal.leftmost_child()
                } else {
                    internal.get_child_page_id(idx - 1).unwrap_or(internal.leftmost_child())
                };
                let right_child = internal.get_child_page_id(idx).unwrap_or(internal.leftmost_child());
                drop(data_guard);
                drop(guard);

                match self.delete_exact_from_page(PageId(left_child as u64), key, row_id) {
                    Ok(()) => Ok(()),
                    Err(StorageError::KeyNotFound) => {
                        self.delete_exact_from_page(PageId(right_child as u64), key, row_id)
                    }
                    Err(e) => Err(e),
                }
            } else {
                let child_page_id = internal.find_child_page_id_binary(key);
                drop(data_guard);
                drop(guard);
                self.delete_exact_from_page(PageId(child_page_id as u64), key, row_id)
            }
        }
    }
}
