//! MS35 (`free-page-lifecycle` R3): controlled page-read failure for the
//! free-page rebuild.
//!
//! R3's scenario is "a page read fails while the free-page set is being
//! rebuilt" → the open still succeeds, the live table is still readable and
//! this open has no allocatable page. A damaged fixture cannot express that:
//! a damaged page is reported by `check_integrity` as an error-level finding,
//! which contradicts the scenario's "error count is 0". The injection point
//! below therefore provides a **one-shot**, **path-scoped** stand-in for the
//! failing read: arm it and the next rebuild of that database aborts before
//! its first page read.
//!
//! Path scoping keeps parallel test binaries from stealing each other's
//! injection; one-shot consumption makes the arming affect exactly one
//! rebuild. Nothing here is armed by default — an unarmed rebuild behaves
//! exactly as if this module did not exist.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Armed injection points, keyed by database path.
static ARMED_FAULTS: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

/// Arm a one-shot page-read failure for the next free-page rebuild of the
/// database at `path` (`free-page-lifecycle` R3; test-support hook).
pub fn arm_free_page_rebuild_fault(path: &Path) {
    let mut guard = ARMED_FAULTS.lock().unwrap();
    guard
        .get_or_insert_with(HashSet::new)
        .insert(path.to_path_buf());
}

/// Consume the injection armed for `path` (one-shot). Returns whether a
/// failure was armed for that database.
pub(crate) fn take_free_page_rebuild_fault(path: &Path) -> bool {
    let mut guard = ARMED_FAULTS.lock().unwrap();
    match guard.as_mut() {
        Some(armed) => armed.remove(path),
        None => false,
    }
}
