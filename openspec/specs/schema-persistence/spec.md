# schema-persistence Specification

## Purpose
TBD - created by archiving change 2026-08-26-ms07-t01-schema-persistence. Update Purpose after archive.
## Requirements
### Requirement: 系统表持久化 schema

`TableManager` SHALL persist table definitions to system tables `__tables` and `__columns` so that schemas survive `Database` process restart. `CREATE TABLE` SHALL write one row to `__tables` and N rows to `__columns` (one per column); `DROP TABLE` SHALL remove those rows; `Database::open` SHALL read those system tables to rebuild all `TableMeta` instances.

#### Scenario: New db bootstrap + create_table + restart recovers schema

- **GIVEN** a non-existent db file path
- **WHEN** `Database::open(path)` is called for the first time
- **THEN** page 0 SHALL be allocated and initialized as an empty `__tables` SlottedPage; page 1 SHALL be allocated and initialized as an empty `__columns` SlottedPage
- **WHEN** a client executes `CREATE TABLE users (id INT PRIMARY KEY, name TEXT)`
- **THEN** `__tables` SHALL contain one row for `users` with `data_page_head`, `index_root_page_id`, `pk_index == 0`, `pk_column == "id"`, `column_count == 2`, and `data_page_tail`
- **AND** `__columns` SHALL contain two rows for `users` (column 0 = `id` Int, column 1 = `name` String)
- **WHEN** `Database` is shut down and re-opened
- **THEN** `database.get_table("users")` SHALL return `Ok(Arc<TableMeta>)`
- **AND** the returned `TableMeta` SHALL have `columns.len() == 2`, `pk_index == 0`, `pk_column == "id"`
- **AND** `data_page_head` SHALL equal the value persisted before shutdown

#### Scenario: Restart preserves DML state

- **GIVEN** a `users` table has been created and rows `(1, 'alice')` and `(2, 'bob')` have been inserted
- **WHEN** `Database` is restarted
- **THEN** `SELECT * FROM users` SHALL return 2 rows
- **AND** `SELECT * FROM users WHERE id = 1` SHALL return one row (proves the BTree index is bound to the persisted root, not a fresh empty root)

#### Scenario: drop_table removes from catalog and persists

- **GIVEN** a `users` table is persisted in `__tables` and `__columns`
- **WHEN** a client executes `DROP TABLE users`
- **THEN** `__tables` SHALL no longer contain a `users` row
- **AND** `__columns` SHALL no longer contain any row with `table_name == "users"`
- **WHEN** `Database` is restarted
- **THEN** `database.get_table("users")` SHALL return `Err(TableNotFound)`

### Requirement: IndexManager::from_root path

`IndexManager` SHALL provide a `from_root(buffer_pool, root_page_id)` constructor that binds to an existing BTree root page without allocating a new one. This is used by `Catalog::recover` to rebind to a persisted index root after restart.

#### Scenario: from_root binds to persisted root

- **GIVEN** a BTree with root at `PageId(N)` is persisted on disk
- **WHEN** `IndexManager::from_root(buffer_pool, PageId(N))` is called
- **THEN** the returned `IndexManager.root_page_id()` SHALL equal `PageId(N)`
- **AND** `search(key)` on existing keys SHALL succeed without allocating a new page

#### Scenario: from_root does not allocate a new page

- **GIVEN** an empty `buffer_pool` with `page_count == M`
- **WHEN** `IndexManager::from_root(buffer_pool, PageId(K))` is called with `K < M`
- **THEN** `page_count` SHALL remain `M` (no new allocation)

### Requirement: Reserved system table names

`__tables` and `__columns` SHALL be reserved identifiers. `TableManager::create_table` and `TableManager::drop_table` SHALL reject any attempt to use those names as user table names.

#### Scenario: CREATE TABLE __tables is rejected

- **WHEN** a client executes `CREATE TABLE __tables (id INT)`
- **THEN** `TableManager::create_table` SHALL return `Err(StorageError::ReservedTableName("__tables"))`
- **AND** the system table `__tables` itself SHALL remain intact

#### Scenario: DROP TABLE __tables is rejected

- **WHEN** a client executes `DROP TABLE __tables`
- **THEN** `TableManager::drop_table` SHALL return `Err(StorageError::ReservedTableName("__tables"))`

### Requirement: data_page_tail persistence

`__tables` rows SHALL include a `data_page_tail: u32` field. The `TableManager` SHALL update this field whenever a DML operation extends the data page chain beyond the current tail. After restart, the recovered `TableMeta::data_page_tail` SHALL reflect the latest value, allowing subsequent INSERTs to append correctly.

#### Scenario: Cross-page INSERT persists tail

- **GIVEN** a `users` table with `data_page_tail == X`
- **WHEN** a client issues enough `INSERT` statements to trigger data page chain extension so that `data_page_tail` becomes `Y`
- **THEN** `__tables` SHALL reflect `data_page_tail == Y` on disk
- **WHEN** `Database` is restarted
- **THEN** the recovered `TableMeta::data_page_tail` SHALL equal `Y`
- **AND** a subsequent `INSERT` SHALL append to the chain starting at `Y`

### Requirement: page 0 / page 1 reservation

page 0 SHALL be reserved for the `__tables` system table; page 1 SHALL be reserved for the `__columns` system table. The reservation SHALL be implemented by convention in `Catalog::bootstrap` and `Catalog::open`, not by special-casing in `FileStorage`.

#### Scenario: Fresh db allocates page 0 and page 1 on first open

- **GIVEN** a non-existent db file
- **WHEN** `Database::open(path)` is called
- **THEN** `FileStorage::page_count()` SHALL be at least 2 after open
- **AND** page 0 SHALL be a SlottedPage with `page_type == 0x03` and `slot_count == 0`
- **AND** page 1 SHALL be a SlottedPage with `page_type == 0x03` and `slot_count == 0`

#### Scenario: Existing db recognizes page 0 and page 1 as system tables

- **GIVEN** a previously-opened db file with `page_count >= 2`
- **WHEN** `Database::open(path)` is called
- **THEN** `Catalog::open` SHALL read page 0 as the `__tables` root
- **AND** read page 1 as the `__columns` root
- **AND** `TableManager::open_or_init` SHALL rebuild all `TableMeta` instances from those pages

### Requirement: Catalog operations under write lock

`Catalog::insert_table`, `delete_table`, and `update_table_tail` SHALL be invoked under `TableManager`'s write lock to maintain consistency between the in-memory `tables: HashMap` and the on-disk `__tables` / `__columns`.

#### Scenario: Concurrent CREATE TABLE is serialized

- **GIVEN** N concurrent `CREATE TABLE t0..t{N-1}` operations
- **WHEN** all complete
- **THEN** `__tables` SHALL contain exactly N rows
- **AND** `TableManager::tables` SHALL contain exactly N entries
- **AND** `get_table("tK")` for any `0 <= K < N` SHALL succeed

#### Scenario: Catalog write failure leaves HashMap consistent

- **GIVEN** `Catalog::insert_table` fails (for example, I/O error during page write)
- **WHEN** `TableManager::create_table` returns the error
- **THEN** `TableManager::tables` SHALL NOT contain the failed table name
- **AND** `__tables` SHALL NOT contain a row for the failed name
- **AND** a subsequent `get_table(name)` SHALL return `TableNotFound`

### Requirement: System tables bypass MVCC and WAL

`__tables` and `__columns` rows SHALL NOT participate in MVCC. They SHALL be written with a fixed `version_header { create_tx_id: 0, commit_tx_id: 1 }` so any reader sees them as committed. They SHALL NOT be written to the WAL.

#### Scenario: System table reads are independent of transaction state

- **GIVEN** a `users` row exists in `__tables`
- **WHEN** any thread calls `get_table("users")` without an active transaction
- **THEN** the row SHALL be returned
- **AND** no snapshot / active transaction lookup SHALL be required

#### Scenario: DDL operations do not write WAL records

- **GIVEN** a `Database` with WAL enabled
- **WHEN** a client executes `CREATE TABLE users (id INT)` and `DROP TABLE users`
- **THEN** no `CreateTable` / `DropTable` WAL record SHALL be written (this capability does not introduce new WAL variants in this change scope)
- **AND** schema recovery SHALL rely solely on `__tables` and `__columns` pages, not on WAL replay

