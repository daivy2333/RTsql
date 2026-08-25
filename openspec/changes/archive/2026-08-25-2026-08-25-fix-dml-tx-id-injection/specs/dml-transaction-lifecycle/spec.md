## ADDED Requirements

### Requirement: DML execution runs inside a real transaction

The pipeline SHALL wrap every `INSERT`, `UPDATE`, and `DELETE` statement issued through `Database::execute_sql` with a real `Transaction` from `TransactionManager::begin()`. The DML executor SHALL receive the real `tx_id` from that transaction; the pipeline SHALL call `TransactionManager::commit()` on success and `TransactionManager::abort()` on failure. The `TransactionManager` SHALL be the single source of truth for `BeginTxn` / `CommitTxn` / `AbortTxn` WAL records.

#### Scenario: INSERT writes a real create_tx_id and commit_tx_id

- **WHEN** a client calls `Database::execute_sql("INSERT ...")` and the statement succeeds
- **THEN** the inserted row's `VersionHeader.create_tx_id` SHALL be the value allocated by `TransactionManager::begin()` (strictly greater than 0) and `VersionHeader.commit_tx_id` SHALL be set by `TransactionManager::commit()` to the same value

#### Scenario: UPDATE writes a real create_tx_id for the new version

- **WHEN** a client calls `Database::execute_sql("UPDATE ...")` and the statement succeeds
- **THEN** the new version's `VersionHeader.create_tx_id` SHALL be the real tx_id from `TransactionManager::begin()` and `VersionHeader.commit_tx_id` SHALL be set by `TransactionManager::commit()`

#### Scenario: DELETE marks the row as deleted and commits

- **WHEN** a client calls `Database::execute_sql("DELETE ...")` and the statement succeeds
- **THEN** the target row's `VersionHeader` SHALL be marked deleted (commit_tx_id = `DELETED_TX_ID`) by the executor and the `TransactionManager::commit()` SHALL mark the version as committed through the normal commit path

#### Scenario: DML failure aborts the transaction

- **WHEN** a DML statement fails (for example, INSERT with a duplicate primary key)
- **THEN** `TransactionManager::abort()` SHALL be called, removing the tx from `active_transactions()` and clearing `tx_versions` for that tx

#### Scenario: WAL records only the transaction manager as the writer

- **WHEN** any DML statement is executed
- **THEN** `WalRecord::BeginTxn` and `WalRecord::CommitTxn` SHALL be written by `TransactionManager::begin()` / `commit()` only; the `InsertExecutor` / `UpdateExecutor` / `DeleteExecutor` SHALL NOT write their own BeginTxn/CommitTxn records

#### Scenario: Monotonically increasing tx_ids across consecutive DML

- **WHEN** a client issues N consecutive DML statements
- **THEN** the `tx_id` observed at each `TransactionManager::begin()` SHALL be strictly greater than the previous one

### Requirement: Tombstone (DELETED_TX_ID) preservation on commit

`VersionHeader::commit()` SHALL NOT overwrite `commit_tx_id` when the current value is `DELETED_TX_ID`. This preserves the delete tombstone that `DeleteExecutor` writes before `TransactionManager::commit()` runs.

#### Scenario: Commit after DELETE preserves the tombstone

- **WHEN** `DeleteExecutor` marks a version as deleted (commit_tx_id = `DELETED_TX_ID`) and then `TransactionManager::commit()` propagates the real tx_id through `commit_mark_versions`
- **THEN** the version's `commit_tx_id` SHALL remain `DELETED_TX_ID`, and `DataScan` SHALL continue to skip the row

#### Scenario: Commit on a non-deleted version overwrites as before

- **WHEN** `TransactionManager::commit()` propagates the real tx_id through `commit_mark_versions` for a version whose `commit_tx_id` is not `DELETED_TX_ID`
- **THEN** the version's `commit_tx_id` SHALL be set to the real commit tx_id (preserves existing behavior)
