## ADDED Requirements

### Requirement: PageVisibilityInfo tracks per-page MVCC visibility summary
The system SHALL maintain an in-memory `PageVisibilityInfo` for each data page, containing `min_create_tx_id` (minimum create_tx_id among all rows) and `all_visible` (true if all rows are committed). This summary SHALL be stored in a `DashMap<PageId, PageVisibilityInfo>` within `BufferPool`.

#### Scenario: Initial state for a new page
- **WHEN** a data page is first accessed and has no visibility entry
- **THEN** the system SHALL behave as if `min_create_tx_id=0` and `all_visible=false` (fall through to per-row checks)

#### Scenario: Crash recovery
- **WHEN** the system restarts after a crash
- **THEN** the DashMap SHALL be empty and all scans SHALL fall back to per-row VersionHeader checks without correctness issues

### Requirement: Fast-path skip for all-visible pages
When `all_visible` is true for a page, the visibility check SHALL skip per-row `VersionHeader` parsing and treat ALL rows on that page as visible to any snapshot.

#### Scenario: All rows committed, scan hits fast path
- **WHEN** a DataScan encounters a page where all rows have `commit_tx_id != UNSET` (all_visible=true)
- **THEN** the scan SHALL return all rows from the page without parsing individual VersionHeaders

#### Scenario: Lazy setting of all_visible
- **WHEN** a scan discovers that every row on a page is committed during per-row traversal
- **THEN** the system SHALL set `all_visible=true` in the visibility map for subsequent scans

### Requirement: Fast-path skip for all-invisible pages
When `snapshot.tx_id < min_create_tx_id` for a page, the visibility check SHALL skip the entire page because all rows were created after the snapshot began.

#### Scenario: Snapshot predates all creates
- **WHEN** a DataScan with snapshot at tx_id=5 encounters a page where `min_create_tx_id = 10`
- **THEN** the scan SHALL skip the entire page without accessing any slots

#### Scenario: Edge case where snapshot equals min_create_tx_id
- **WHEN** a DataScan with snapshot at tx_id=10 encounters a page where `min_create_tx_id = 10`
- **THEN** the scan SHALL fall through to per-row checks (snapshot.tx_id is NOT strictly less than min_create_tx_id)

### Requirement: INSERT updates visibility summary
When a new row is inserted into a data page, the system SHALL update `min_create_tx_id = min(current, new_row.create_tx_id)` and clear `all_visible = false`.

#### Scenario: Single INSERT on an empty page
- **WHEN** the first row is inserted into a data page with `create_tx_id=42`
- **THEN** `min_create_tx_id` SHALL be set to 42 and `all_visible` SHALL be false

#### Scenario: INSERT on a page that was all-visible
- **WHEN** a new row is inserted into an all-visible page
- **THEN** `all_visible` SHALL be cleared to false and `min_create_tx_id` SHALL be updated to include the new row's create_tx_id

### Requirement: DELETE clears all_visible flag
When a row is deleted from a data page, the system SHALL clear `all_visible = false` for that page (visibility of remaining rows now depends on individual commit_tx_id checks).

#### Scenario: DELETE on an all-visible page
- **WHEN** a row is deleted from an all-visible page (slot marked as deleted via GC or version chain)
- **THEN** `all_visible` SHALL be cleared to false

### Requirement: Visibility summary correctness guarantees
The visibility summary SHALL be a pure optimization — incorrect or missing entries SHALL never cause incorrect query results. The system SHALL always fall back to per-row VersionHeader checks when the summary is ambiguous or absent.

#### Scenario: False negative (all_visible=false when page is actually all-visible)
- **WHEN** the visibility map incorrectly marks a page as not-all-visible
- **THEN** the scan SHALL fall back to per-row checks and still return correct results

#### Scenario: Concurrent read and write
- **WHEN** a reader checks the visibility map while a writer is concurrently inserting a row
- **THEN** the reader SHALL see either the old or new summary; either way, correctness SHALL be preserved by per-row fallback for ambiguous cases
