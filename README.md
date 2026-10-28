# RTsql

[English](README.md) | [简体中文](README.zh-CN.md) | [Agent operations guide](rtsql-docs/SKILL.md)

RTsql is an embedded relational database written in Rust. It uses Tokio tasks for asynchronous I/O, stores each database in one main file, and provides a one-shot CLI for SQL execution and database management.

On 2026-10-28, Linux x86_64 `cargo build --locked --offline --release --bin rtsql` produced a release binary. That binary created a temporary database at a path containing `/`, then ran `CREATE TABLE t (id INT)`, `INSERT INTO t (id) VALUES (1)`, and `SELECT id FROM t`. The query output contained `{"columns":["id"],"rows":[[1]]}`. The same statements completed with a static `x86_64-unknown-linux-musl` binary built by `CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=/opt/musl/x86_64-linux-musl-cross/bin/x86_64-linux-musl-gcc cargo rustc --locked --offline --release --bin rtsql --target x86_64-unknown-linux-musl -- -C target-feature=+crt-static`. `ldd` reported `statically linked`. macOS was not run. The repository can cross-build static RISC-V 64 Linux (musl) binaries. It does not require a database server.

## Quick start

### Prerequisites

- A stable Rust toolchain with `cargo`
- `bash` and standard Unix user tools; `strip` is optional
- Git if you are cloning the repository

### Install from source

```bash
git clone git@github.com:daivy2333/RTsql.git
cd RTsql
./install.sh
source ~/.bashrc
rtsql --version
```

`install.sh` builds the release binary, installs it under `~/.local/bin`, installs completions for the current shell when available, and adds the installation directory to `~/.bashrc` when needed. It does not use `sudo` or invoke a separate downloader; Cargo may fetch declared Rust dependencies during the build.

Use another installation prefix or skip completions with:

```bash
./install.sh --prefix "$HOME/.local-rtsql"
./install.sh --prefix "$HOME/.local-rtsql" --no-completions
```

The prefix controls the binary location. Completion files remain in their standard per-user shell directories.

### Cross-build for RISC-V 64 Linux (musl)

On a Linux build host, RTsql can cross-compile a static RISC-V 64 binary and package it for deployment — the script never runs the RISC-V binary itself:

```bash
rustup target add riscv64gc-unknown-linux-musl
./build-riscv64-musl.sh
```

The target is fixed to `riscv64gc-unknown-linux-musl`; the build links with `riscv64-linux-musl-gcc` and enables `+crt-static`, so the result is a fully static RISC-V ELF with no dynamic interpreter dependency. Prerequisites (`cargo`, `rustup`, `riscv64-linux-musl-gcc`, and GNU `tar`) are checked up front: missing items are reported with install hints and the script stops — it never auto-installs, never uses `sudo`, never uploads anything, and never writes global Cargo configuration.

A successful run publishes into `dist/riscv64gc-unknown-linux-musl/`:

- `rtsql` — the stripped static binary
- `rtsql-v<version>-riscv64gc-unknown-linux-musl.tar.gz` — the binary plus both READMEs

Pass `--output-dir DIR` to publish under a different output root, and `--help` for the built-in reference. Copy the archive to the target device and deploy there. A cross-build is not runtime validation: version, CRUD, encryption, and recovery checks must be run on real RISC-V hardware, and this repository does not claim them from a cross-build alone.

### Create and query a database

```bash
rtsql new demo
rtsql demo "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT, age INTEGER)"
rtsql --format table demo "INSERT INTO people VALUES (1, 'Ada', 36), (2, 'Lin', 41)"
rtsql --format table demo "SELECT id, name, age FROM people WHERE age >= 40"
rtsql schema demo
```

A bare database name is stored under `$RTSQL_HOME/db/<name>.db`; the default is `$HOME/.rtsql/db`. An argument containing `/` is used as a file path directly.

### Use an SQL transaction

Each CLI invocation is one-shot. Send `BEGIN`, the transaction statements, and `COMMIT` or `ROLLBACK` in the same SQL argument:

```bash
rtsql --format table demo "BEGIN; INSERT INTO people VALUES (3, 'Kai', 22); COMMIT;"
rtsql --format table demo "BEGIN; INSERT INTO people VALUES (4, 'Mira', 29); ROLLBACK;"
rtsql --format table demo "SELECT id, name FROM people ORDER BY id"
```

Without an explicit transaction, each statement in a semicolon-separated SQL string is committed separately. If the input ends with an active transaction, RTsql rolls it back and reports the rollback on stderr.

### Back up and restore

```bash
rtsql dump demo > demo.sql
rtsql new demo-restored
rtsql restore demo-restored demo.sql
rtsql --format table demo-restored "SELECT id, name, age FROM people ORDER BY id"
```

`dump` writes SQL text and does not encrypt that file. Protect the dump as you would any plaintext database export. `restore` requires an empty target database; pass `-` instead of a file path to read the dump from stdin.

#### Backup boundaries and the database file family

Prefer `dump` / `restore` for backups. A database on disk is a family of three files derived from one name — `<db>.db`, `<db>.wal`, and `<db>.checkpoint` — and a file-level copy is only as safe as the state you copied.

Copying `<db>.db` alone is equivalent to a full backup **only after the writer closed cleanly** (one-shot commands checkpoint on close, so the WAL is already truncated). While a writer is still running, or after a crash, the main file holds only the checkpointed prefix: the measured rows below come from a session killed with `SIGKILL` after committing 937 of 938 rows, where the main file alone showed **1 of 938 rows** — silently incomplete, with no error on open.

| Form | Result |
|---|---|
| Copy `<db>.db` after a clean close | Complete (1 of 1 rows) |
| Copy `<db>.db` + `<db>.wal` after a clean close | Complete |
| Copy `<db>.db` + `<db>.checkpoint` after a clean close | Complete |
| Copy `<db>.db` alone with an uncheckpointed tail | **Incomplete: 1 of 938 rows** |
| Copy `<db>.db` + `<db>.wal` with an uncheckpointed tail | Complete (938 rows, WAL replayed on open) |
| Move the pair to another directory, names unchanged | Complete (938 rows) |
| Rename the main file only, leaving the WAL behind | **Incomplete: 1 of 938 rows**, and the orphaned `<db>.wal` is ignored |

Companion names are derived from the main file name, so any rename or move must carry `<db>.wal` and `<db>.checkpoint` along under the same base name. A file-level copy taken while a writer is active should include `<db>.wal`; otherwise close the writer first — or run `rtsql checkpoint <db>` to fold the WAL into the main file — and verify with `rtsql <db> "SELECT count(*) FROM t"` on the copy.

Schema changes are durable at the statement boundary: a successful `CREATE TABLE`, `CREATE INDEX`, `CREATE VIEW`, `ALTER TABLE ADD COLUMN`, or `CREATE TABLE AS SELECT` writes its catalog pages to disk before the statement returns — the `__tables` row lands last, as the commit point — so a process killed at any later instant still finds the schema on reopen, and WAL replay recovers the rows. The remaining crash windows are `DROP` statements (whose page release is checkpoint-based) and DDL on encrypted databases (whose WAL and checkpoint sidecars are plaintext by design); after either, verify with `rtsql check <db>` or prefer `dump` / `restore` for backups.

`rtsql delete <db>` removes one database family; `./install.sh --uninstall --purge-data` wipes the whole data directory during uninstall. Neither is recoverable through RTsql.

### Delete a database

```bash
rtsql delete demo --dry-run
rtsql delete demo
```

`delete` removes `<db>.db` together with its `.wal` and `.checkpoint` companions, reports the freed bytes per file, and exits 0. It is a file-level management operation: it does not open the database, does not validate content, and needs no `--key` even for an encrypted database. A database currently open in another session is refused with exit code 4 and nothing is deleted; a missing database is refused with exit code 1. Companion files that are already absent are reported as `absent` and are not an error.

### Create an encrypted database

```bash
rtsql new secure --key 'replace-with-a-password'
rtsql --key 'replace-with-a-password' secure "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT)"
rtsql --key 'replace-with-a-password' secure "INSERT INTO notes VALUES (1, 'private')"
rtsql --key 'replace-with-a-password' secure "SELECT id, body FROM notes"
```

`RTSQL_KEY` is equivalent to `--key`; an explicit `--key` takes precedence:

```bash
RTSQL_KEY='replace-with-a-password' rtsql secure "SELECT id, body FROM notes"
```

To migrate by export and import:

```bash
rtsql dump demo > demo.sql
rtsql new secure-copy --key 'replace-with-a-password'
rtsql --key 'replace-with-a-password' restore secure-copy demo.sql
```

Opening an encrypted database without a key, opening a plaintext database with a key, and using the wrong key all return exit code 5. An empty key is rejected with exit code 2 before a database is opened.

### Uninstall

Run these commands from the source checkout:

```bash
./install.sh --uninstall
```

This removes the installed binary and completion files but keeps `$RTSQL_HOME` (or `$HOME/.rtsql` by default).

```bash
./install.sh --uninstall --purge-data
```

`--purge-data` additionally prints and deletes the data directory. This deletion is not recoverable through RTsql.

## CLI surface

Run `rtsql --help` for the built-in reference. The main form is:

```text
rtsql [OPTIONS] [DB] [SQL] [COMMAND]
```

| Command | Purpose |
|---|---|
| `rtsql <db> <sql>` | Execute one SQL statement or a semicolon-separated script |
| `rtsql new <target>` | Create an empty named database or path |
| `rtsql list` | List databases in the centralized storage directory |
| `rtsql schema <db>` | Print user-table DDL (column types render as standard names: `INTEGER`, `TEXT`, `FLOAT`, `BOOLEAN`, `DATE`, `TIMESTAMP`) |
| `rtsql dump <db>` | Export DDL and rows as SQL text (same type-name rendering; a `dump` → `restore` round-trip recreates the schema and rows) |
| `rtsql restore <db> <file>` | Restore a dump into an empty database; `-` reads stdin |
| `rtsql import <db> <table> <file\|-> --csv` | Import CSV with a matching header row, or auto-create the table from the header; `-` reads stdin; `:memory:` targets are accepted |
| `rtsql delete <db> [--dry-run]` | Delete a database and its `.wal` / `.checkpoint` companions; `--dry-run` only lists them |
| `rtsql checkpoint <db>` | Flush committed data into the main file (truncate WAL, advance the checkpoint site); the clean sync point before a file-level backup or handoff |
| `rtsql check <db>` | Read-only structural integrity scan (page census, catalog, data/overflow chains, index trees); damage is reported one line per finding with its page and cause, a clean database prints a single confirmation. Exits `0` clean, `1` on error-level findings (informational ones do not change the code), `4` on lock conflict, `5` on a wrong key; `:memory:` and `--readonly` combinations are usage errors (`2`). Never repairs or modifies the file; a database with pending WAL must first be opened read-write once to recover (a pure-read session suffices — replay runs at open and the read-write close truncates a WAL that held transaction records) |
| `rtsql stats <db> <table>` | Summarize count, null rate, distinct values, ranges, and numeric percentiles |
| `rtsql sample <db> <table> [n]` | reservoir-sample rows; default `n` is 10 |
| `rtsql profile <db> <table> [--top n]` | Profile columns and frequent String values; default top count is 5, maximum 20 |
| `rtsql completions <bash\|zsh\|fish>` | Generate a completion script; this command is hidden from `--help` |

### Read-only sessions

`rtsql --readonly <db> <sql>` opens the database with an advisory shared lock instead of the default exclusive lock, so many readers can observe one file at the same time while a writer still runs alone.

- **Many readers, one writer:** read-only sessions share the lock with each other; a read-write session excludes readers and other writers, and a read-only session excludes a read-write one. A conflict waits out a configurable budget — `--busy-timeout <ms>` or `RTSQL_BUSY_TIMEOUT`, default 5000 ms; `0` waits only for an internal ~8 ms visibility floor — and exit code `4` is reported only when the budget is exhausted. The budget applies to every command that opens a database, for the exclusive and the shared lock alike; `:memory:` accepts and ignores it, and the `delete` lock probe never waits.
- **No pending WAL:** a read-only session runs no recovery, so an open is refused when the `.wal` still holds unreplayed records. The message names the read-write path that performs the recovery; the exit code is `1`.
- **No writes at all:** recovery and checkpoint are skipped, and `INSERT` / `UPDATE` / `DELETE` / `UPSERT` / `REPLACE` and every DDL statement are refused at plan time with SQLSTATE `25006` (exit code `3`). `SELECT`, `ATTACH`, `DETACH`, and transaction statements are unaffected. The CLI renders the code: text formats end the stderr error line with ` [SQLSTATE 25006]`, and `--format json` reports the error as a single-line JSON object on stderr carrying a `sqlstate` field (omitted when the error has no class).
- **Cross-database:** `ATTACH` inside a read-only session opens the attached database read-only as well.
- **Encrypted databases:** pass `--key` as usual; a wrong key still fails with exit code `5`.
- **Main command only:** combining `--readonly` with a lifecycle or analysis subcommand is a usage error (exit code `2`) — those commands keep their read-write open.

### In-memory databases

`rtsql :memory: <sql>` runs the whole session in memory. The full SQL surface is available, including DDL, transactions, and `ATTACH`, and no file is created: there is no `.db`, `.wal`, or `.checkpoint` companion, `close()` and `checkpoint()` do nothing, and the data is discarded when the process exits.

Lifecycle subcommands, `--key`, and `--readonly` are usage errors for `:memory:` (exit code `2`).

### Cross-database sessions

One CLI session can register additional database files with `ATTACH '<path>' AS <alias>` and address them through qualified names (`alias.table`); bare names keep pointing at the main database. The session holds each attached file's advisory lock for its lifetime, and the `--busy-timeout` budget applies to those locks exactly as it does to the main file.

```bash
rtsql app "ATTACH 'orders' AS o; \
  INSERT INTO o.recent SELECT id, name FROM t WHERE id > 100; \
  SELECT COUNT(*) FROM o.recent"
```

- **Writes route to the attached file:** `INSERT`, `UPDATE`, `DELETE`, and the upsert forms on `alias.table` run against that file with its own transaction, WAL, and buffer pool. Every file commits independently: when a statement fails, the statements before it stay committed — including the ones that touched other files.
- **No cross-file transactions:** a write to an attached database inside `BEGIN` is refused with SQLSTATE `0A000` (exit code `3`) before any row is touched. Main-database writes keep the full `BEGIN` / `COMMIT` / `ROLLBACK` semantics, and an explicit main-database transaction can read attached databases.
- **DDL stays local:** attached databases take queries and DML only — `CREATE TABLE`, `DROP TABLE`, `CREATE TABLE AS SELECT`, and other DDL on an attached target are refused at plan time.
- **`INSERT ... SELECT`:** `INSERT INTO <existing table> SELECT ...` fully materializes the source rows, then writes them through the regular insert path. The source can join tables across databases, write back into its own target table (it consumes only the rows read before the statement began), and carry `ON CONFLICT DO NOTHING` / `DO UPDATE` or `REPLACE INTO`. An explicit column list maps by position, and omitted columns take their declared `DEFAULT` or `NULL`; a source that fails leaves the target untouched, and an empty source reports `affected_rows: 0`.
- **Read-only sessions** open attached databases read-only, and every write is refused with SQLSTATE `25006` as usual.

### Automatic checkpoint by WAL size

A read-write session checkpoints the WAL automatically once the bytes appended since the last checkpoint reach a threshold, so a long-running session keeps its `.wal` file bounded. The check happens at each commit boundary (after an auto-commit statement, and after an explicit `COMMIT`), and it is deferred while another transaction of the same session is still active.

- **Default:** 16 MiB. Ordinary sessions stay far below it and keep the existing behavior, where only `close()` and an explicit checkpoint truncate the WAL.
- **Explicit:** `rtsql checkpoint <db>` runs the same truncation on demand and prints one confirmation line — a clean database reports `no pending work` and still exits `0`. Use it as the sync point before an external backup or file handoff.
- **Environment variable:** `RTSQL_WAL_CHECKPOINT_BYTES=<bytes>` overrides the threshold; the same value is reachable from the library through `Database::set_checkpoint_bytes_threshold`. A value of `0` disables the automatic checkpoint — the WAL keeps growing and `close()` still performs a full checkpoint.
- **Invalid values:** a non-numeric, negative, or overflowing value is refused before the database is opened, with a message naming the variable and the rejected value (exit code `2`); no file is created.
- **Failures:** if the automatic checkpoint fails, the statement that triggered it reports the error and names the checkpoint failure (exit code `3`). The data it committed stays durable — the commit is fsynced before the checkpoint runs, and a later commit retries.
- **Read-only and in-memory sessions:** they hold no WAL, so the setting has no effect there.

### Output formats

`--format` accepts `table`, `json`, `csv`, and `tsv`. RTsql uses `table` for an interactive terminal and `json` otherwise unless the option is set. The `table` format truncates any cell longer than 64 characters (headers included) to its first 61 characters followed by `...`; `json`, `csv`, and `tsv` always carry the full value.

A script may pack several statements into one invocation, separated by `;`. Each statement executes in order with its own auto-commit, and a failure stops the script with its statement number. Under `--format json`, the output is JSONL — one standalone JSON object per statement line — and a successful DDL statement reports `{"affected_rows":0}`, like any other zero-row statement.

### Exit codes

| Code | Meaning |
|---:|---|
| 0 | Success |
| 1 | General I/O, storage, or format error |
| 2 | CLI usage error |
| 3 | SQL parse or execution error |
| 4 | Database is locked by another process (after the `--busy-timeout` budget) |
| 5 | Encryption-key error |
| 130 / 143 | Terminated by SIGINT / SIGTERM |

## SQL and engine capabilities

- **DDL and DML:** `CREATE TABLE`, `DROP TABLE`, `SELECT`, `INSERT` (including `ON CONFLICT DO NOTHING` / `DO UPDATE` and `REPLACE INTO`), `UPDATE`, and `DELETE`.
- **Queries:** `WHERE`, `JOIN`, `GROUP BY`, `HAVING`, `ORDER BY`, `LIMIT`, and `OFFSET`.
- **Expressions:** `IN`, `BETWEEN`, `LIKE`, `IS NULL`, `NOT`, `CASE`, `COALESCE`, `CAST`, and arithmetic operators.
- **Scalar functions:** `upper`, `lower`, `length`, `substr`, `replace`, `trim`, `abs`, `round`, `floor`, and `ceil`.
- **Date and time:** `DATE`, `TIMESTAMP`, `now`, `date`, `year`, `month`, `day`, `hour`, `minute`, `second`, `date_trunc`, `datediff`, and `INTERVAL` arithmetic.
- **Subqueries:** scalar subqueries — in `WHERE` comparisons and the select list, correlated included, with statement-level result reuse for repeated parameters — plus `IN`, `EXISTS`, and derived tables. A scalar subquery returning more than one row is refused by name (`Subquery returns multiple rows`); an empty result compares as `NULL` under three-valued logic; scalar subqueries stay refused inside a `JOIN` base `WHERE` clause.
- **Grouping:** group by column, alias, expression text, or ordinal position.
- **Aggregate expressions:** aggregates nest inside SELECT value expressions (`SELECT MAX(id) + 1`), `HAVING` conditions (`HAVING MAX(id) + 1 > 2`), and aggregate-query `ORDER BY` keys (`ORDER BY MAX(v)`); a select list can mix aggregates with plain constants (`SELECT MAX(id), 1`); the same aggregate call appearing several times shares one output column. Aggregates inside `WHERE`/`JOIN ... ON`, aggregate nesting, and non-aggregated column references without `GROUP BY` are refused at plan time.
- **Constant queries:** expressions without `FROM`, such as `SELECT 1 + 1`.
- **Transactions:** explicit library transactions and CLI `BEGIN` / `COMMIT` / `ROLLBACK` sessions.
- **MVCC:** Repeatable Read by default and opt-in Read Committed through `Database::open_with_isolation`.
- **Storage:** persistent schema, B-Tree primary-key indexes, WAL with frame checksums, checkpointing, crash recovery, and page reuse. A single-column non-unique `CREATE INDEX` accepts INTEGER and string declarations (TEXT, VARCHAR, CHAR, and CLOB). It serves equality predicates automatically, and its row set always matches a full table scan exactly — same rows, no duplicates, no misses — at any table size, including after restart and crash recovery. A string is read through the index when its complete key encoding fits in 32 bytes. A longer string is stored in full, and an equality query scans that complete literal; the same fallback holds when another column puts the row on an overflow page. Values are not truncated, and columns are not given a length limit. NULL is not entered in the index. Duplicate strings are allowed. PRIMARY KEY and UNIQUE still accept only integers. INTEGER and string columns can form a multi-column non-unique index, and the key is still 32 bytes. A newly created single-column INTEGER index serves range predicates in signed numeric order. A single-column INTEGER index created before the upgrade keeps its original key bytes: equality is unchanged, and range predicates still scan. When a range could miss a non-NULL row, the result also includes rows that are not in the index and for which the predicate holds, matching a full table scan.
- **Long values:** row values larger than the single-page inline capacity (~4KB) are stored automatically through overflow page chains, with no artificial length limit. Overflow is transparent to the SQL layer: `INSERT`/`UPDATE`/`SELECT`, `dump`/`restore`/`import`, crash recovery, GC and `DROP TABLE`/`TRUNCATE` release, encrypted databases, `:memory:`, and read-only sessions all handle it the same way (rows beyond page capacity previously failed with `No enough space in page`; that failure mode is gone).
- **Runtime forms:** read-only sessions over a shared lock with many concurrent readers and zero writes, plus `:memory:` databases that keep everything in memory for the life of the process.

The engine uses explicit type checks and three-valued predicate logic. It does not implicitly convert incompatible SQL types.

### Constraints

Every constraint a `CREATE TABLE` statement accepts is either enforced or rejected by name at plan time — a declared constraint is never silently ignored.

- **NOT NULL** is enforced on `INSERT` and `UPDATE`. A violating write fails with `NOT NULL constraint violation: column '<name>'`, leaving no side effects.
- **UNIQUE** is enforced for `INT` columns (primary-key columns excluded) through a dedicated internal unique index; duplicates are rejected with `Duplicate key`, `NULL` values are exempt, and enforcement survives restart and crash recovery. A `UNIQUE` declared on the primary-key column is consumed by the primary key's existing uniqueness and does not create a second index.
- **PRIMARY KEY** must be declared on an `INTEGER` column — that is the only form the engine enforces. An explicit non-integer `PRIMARY KEY` is refused at plan time; tables without a declared primary key keep the unchanged implicit first-column behavior.
- **Rejected at plan time (the table is not created):** `CHECK`, `FOREIGN KEY`, dialect-specific column options, an explicit non-`INTEGER` `PRIMARY KEY`, `UNIQUE` on non-`INT` columns, and composite (multi-column) `UNIQUE`.
- **Legacy compatibility:** the supported range of old databases and old dumps is defined by **the catalog-row layout generation** (`G1` = oldest: `__tables` rows end at the data-chain tail pointer; `G3` = current: the rows also carry the unique-roots, secondary-index and table-kind sections), not by the file format version — the version is a single product format contract (`v1`) that only declares the layout the product writes. Inside the range a legacy database opens, queries, takes writes, passes an integrity scan, rebuilds on open and can be dumped; a legacy database with a pending write log recovers its committed rows; an old dump restores into a new database with equivalent rows, and the restored database uses the current format, keeps accepting writes and can be dumped again. A write has one visible consequence: the catalog row of the **written** table is rewritten to the current layout, while tables that were not written keep their own generation. Databases created by older versions that contain non-`INT` `UNIQUE` columns open normally (those columns are not enforced), and the current export does not render `UNIQUE`, so `dump` → `restore` round-trips them; legacy tables declared with a non-integer `PRIMARY KEY` also open, read and dump normally — their dump DDL renders without `PRIMARY KEY`, so `dump` → `restore` round-trips them. Outside the range the input is refused explicitly and located to the statement: a non-integer `PRIMARY KEY` (`NON-INTEGER PRIMARY KEY`), a `UNIQUE` on a non-`INT` column (`UNIQUE (INT columns only)` — the plan-time rules above refuse that table itself) and a `NULL` in a `NOT NULL` column (`NOT NULL constraint violation: column '<name>'`) each fail the restore with exit code 3, naming the cause and the failing statement number — a dump produced by an older version that embeds a non-integer `... PRIMARY KEY` DDL fails for that reason, and rows written before `NOT NULL` enforcement that hold `NULL` in a `NOT NULL` column fail for that reason too. Migration paths: drop the `PRIMARY KEY` declaration from the dump and restore again; drop the `UNIQUE` declaration and restore again, or change the constrained column to an integer type **and** convert that column's row values at the same time; backfill the `NULL` and restore again, or delete that row and restore again. A file whose format version is newer than the supported one is refused (`NewerFileVersion`, exit code 1) and a file without a file header is refused (`NotADatabase`, exit code 1) — the product does not sniff old formats and does not guess a migration.

### Write surface

`INSERT` and `UPDATE` accept the SQLite-subset write surface. A rejected write leaves no side effects: `INSERT`, `UPDATE`, `DO NOTHING` and `DO UPDATE` are fully validated before any row is touched, while a `REPLACE INTO` that fails validation after deleting the conflicting rows is rolled back with the deleted row's primary-key and unique-index entries restored.

- **Column list:** `INSERT INTO t (col, ...) VALUES ...` accepts any subset of the table's columns in any order. Omitted columns take their declared `DEFAULT`, or `NULL` when no default is declared; a `NOT NULL` column filled that way is rejected by name. `DEFAULT` inside `VALUES` is equivalent to omitting that column. Declared defaults are persisted in the catalog, apply after restart, and are rendered by `rtsql dump` and `rtsql schema`.
- **Value types:** a written value must match the column's declared type. `NULL` is exempt, date columns accept their typed literals, a `FLOAT` column accepts integer values (stored as the equivalent float), and any other cross-type write is rejected with `column '<name>' expects <TYPE>, got <type>`. The `dump` / `restore` / `import` paths run through the same rule.
- **Declared defaults:** a `DEFAULT` literal in `CREATE TABLE` or `ALTER TABLE ADD COLUMN` is validated against the column type at plan time under the same rule — `NULL` is exempt, a `FLOAT` column upgrades an integer literal (persisted as the float), a `DATE`/`TIMESTAMP` column parses the string (an invalid date is refused), and other cross-type declarations are rejected with `DEFAULT type mismatch: column '<name>': expected <TYPE>, got <type>` before the table exists. Typed date literals such as `DEFAULT DATE '2026-01-02'` are accepted, so `dump` → `restore` round-trips tables with date defaults. A `DEFAULT` literal's length is additionally bounded by the catalog column row's in-page capacity (~4KB): an oversized default is rejected at plan time with `DEFAULT type mismatch: column '<name>': DEFAULT value is too large for the catalog page capacity` — before the table exists, with no side effects. Written values are not subject to this limit (oversized rows go through the overflow page chains).
- **`UPDATE ... SET` expressions:** several columns may be assigned at once, and each right-hand side accepts a full value expression — column references evaluate against the row's old values as one snapshot (`SET a = a + 1, b = a` reads the old `a`), plus arithmetic, `CAST`, `CASE`, `COALESCE`, and scalar functions. The write type gate, `NOT NULL`, key-column type checks, collision prechecks, rekey, and unique/secondary index maintenance all apply to the evaluated values; a failing assignment leaves the row untouched. Aggregate calls and subqueries on the right-hand side are refused at plan time.
- **Full-table `UPDATE`/`DELETE`:** omitting `WHERE` applies the statement to every row. The per-row semantics run row by row — write type gate, `NOT NULL`, key-column type checks, collision prechecks, rekey, and unique/secondary index maintenance — and a failure mid-statement rolls back everything the statement wrote (implicit-transaction semantics; inside `BEGIN` the statement is usable and can be rolled back with the transaction). `affected_rows` reports the actual row count. `TRUNCATE TABLE` remains the faster physical-release alternative.
- **`ON CONFLICT DO NOTHING`:** conflicting rows are skipped and are not counted in the affected-row count.
- **`ON CONFLICT (col) DO UPDATE SET ...`:** the conflicting row is updated in place. The right-hand side accepts a literal, `DEFAULT`, `excluded.<column>` (the row being inserted) or a bare column name (the conflicting row's old value), and several columns may be assigned at once. Only the assigned columns change.
- **Conflict target:** omitting the target arbitrates the primary key and every unique index (primary key first, then unique columns in declaration order). An explicit target must be a single column that carries uniqueness — an `INT` primary key or a `UNIQUE` column.
- **`REPLACE INTO`:** every conflicting row is deleted and the new row is inserted; one affected row is counted per inserted row.
- **Rejected at plan time (CLI exit code 3):** composite and non-unique conflict targets, reported as `ON CONFLICT clause does not match any PRIMARY KEY or UNIQUE constraint`; `ON CONFLICT ON CONSTRAINT`; `DO UPDATE WHERE`; MySQL's `ON DUPLICATE KEY UPDATE`; `REPLACE INTO` combined with an `ON CONFLICT` clause; assignment right-hand sides other than a literal, `DEFAULT`, `excluded.<column>` or a column reference, which includes arithmetic and function expressions. The `INSERT OR ...` upsert dialect is not reachable with this engine's SQL dialect.
- Primary-key and unique-index entries are maintained by all three actions, and the result survives restart and crash recovery.

## Encryption model

An encrypted database uses a 64-byte plaintext header followed by encrypted page records. The header contains a random 32-byte Argon2id salt and persisted KDF parameters. Each 4096-byte page is stored as a 4124-byte record containing a 12-byte nonce, ciphertext, and a 16-byte AES-GCM authentication tag. The page ID is authenticated as additional data.

The `.wal` and `.checkpoint` sidecar files remain in their existing plaintext formats. Encrypting a main database file does not encrypt an exported dump or these sidecars.

## Library API

The main entry points are:

- `Database::open(path)` — open or create with Repeatable Read.
- `Database::open_with_isolation(path, level)` — select Repeatable Read or Read Committed.
- `Database::open_with_key(path, isolation, key)` — open plaintext or encrypted storage.
- `Database::open_with_lock_wait(path, isolation, key, lock_wait)` — open with a configurable lock wait budget (`None` = the 5000 ms default).
- `Database::open_readonly_with_lock_wait(path, key, lock_wait)` — read-only open with the same budget.
- `Database::execute_sql(sql)` — execute SQL.
- `Database::execute_in_tx(...)` — execute inside an explicit library transaction.
- `Database::checkpoint()` — flush a checkpoint.
- `Database::close()` — flush, checkpoint, and release the file lock.

## Architecture

SQL text is parsed with `sqlparser-rs`, planned into a physical plan, and executed by a Volcano-style executor tree. The execution pipeline reads MVCC-visible rows through a DashMap-backed buffer pool and B-Tree primary-key indexes. Fixed-size slotted pages hold serialized rows and version chains. WAL records provide redo recovery, while checkpoints persist a safe replay position and bound WAL growth. Page encryption is contained in `FileStorage`; the buffer pool and executors operate on ordinary 4096-byte page images.

## Build and test

```bash
cargo build --release
cargo test --no-fail-fast
cargo clippy --all-targets -- -D warnings
cargo fmt --check
cargo bench
```

`cargo test --test e2e_test -- --test-threads=1` runs the end-to-end journey suite: real CLI subprocesses that walk whole user journeys (lifecycle, import, crash recovery, indexes, locking, cross-database, encryption, large values, analytics, output shapes, error surfaces). It is the fastest way to verify that the database works as a whole.

The repository also contains Criterion benchmarks and SQLite comparison benchmarks under `benches/`. RISC-V 64 Linux (musl) artifacts are produced by `build-riscv64-musl.sh`; see [Cross-build for RISC-V 64 Linux (musl)](#cross-build-for-risc-v-64-linux-musl).

On 2026-10-28, Linux x86_64 `cargo build --locked --offline --release --bin rtsql` produced a release binary. That binary created a temporary database at a path containing `/`, then ran `CREATE TABLE t (id INT)`, `INSERT INTO t (id) VALUES (1)`, and `SELECT id FROM t`. The query output contained `{"columns":["id"],"rows":[[1]]}`. The same statements completed with a static `x86_64-unknown-linux-musl` binary built by `CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=/opt/musl/x86_64-linux-musl-cross/bin/x86_64-linux-musl-gcc cargo rustc --locked --offline --release --bin rtsql --target x86_64-unknown-linux-musl -- -C target-feature=+crt-static`. `ldd` reported `statically linked`. macOS was not run.

## Performance and resource comparison

Measured 2026-09-24 on an Intel i9-13900HX (32 threads) Linux machine: RTsql 0.1.0 (release build) versus SQLite 3.37.2 (system `libsqlite3` linked via `rusqlite`, plus the `sqlite3` CLI). Both engines run with default settings on the same synthetic `bench (id INTEGER PRIMARY KEY, name TEXT, value INTEGER)` table; RTsql auto-commits each statement through its WAL while SQLite uses its default rollback journal. These are single-machine observations, not benchmark guarantees.

Appending a frame, a commit wait returning, and a checkpoint are three different promises. The commit wait returns after that batch's `sync_all`. Concurrent commits can share one batch, so the per-statement INSERT row is not a count of one fsync per statement. Both engines are on their default journal and sync settings. Seeing `sync_all` run, or killing the process with `SIGKILL` and reopening, does not show that power loss has been tested.

Engine-level operations (Criterion in-process benchmarks against the engine API, mean of 20 samples; reproduce with `cargo bench --bench sqlite_compare`):

| Operation | RTsql | SQLite | Faster |
|---|---|---|---|
| INSERT 100 rows (per-statement commit) | 4.8 ms (~20,800 rows/s) | 251.3 ms (~400 rows/s) | RTsql ~52x |
| PK point lookup (1k-row table) | 1.57 µs | 6.80 µs | RTsql ~4.3x |
| Full scan of 1k rows | 297 µs | 98 µs | SQLite ~3x |

CLI-level footprint (one-shot process loading 2,000 rows plus `SELECT COUNT(*)`):

| Metric | RTsql | sqlite3 |
|---|---|---|
| Load time (2,000 inserts) | 3.2 s | 5.3 s |
| Peak RSS | ~16.4 MiB | ~4.1 MiB |
| Main database file after close | 300 KiB | 48 KiB |
| One-shot `SELECT 1` latency (mean of 50 runs) | 6.5 ms | 1.2 ms |
| Binary size | 6.7 MB | 1.6 MB |

One-shot latency and peak RSS are re-measured values from 2026-09-26, after the zero-work close skip landed (read-only sessions no longer checkpoint on close; the default multi-thread runtime was retained after a measured flavor evaluation). The load-time row keeps its 2026-09-24 value because the load path is unchanged.

Reading the numbers: RTsql's per-statement insert path and index lookups are fast, its full-scan path is currently slower than SQLite's, and one-shot invocations carry the fixed cost of an async runtime, while read-only closes no longer add a checkpoint on top (zero-work closes skip it). SQLite is three decades more mature; treat these tables as a snapshot of where RTsql stands today, and re-run the benchmarks on your own hardware before drawing conclusions.

## Known limitations

- On 2026-10-28, Linux x86_64 `cargo build --locked --offline --release --bin rtsql` produced a release binary that created a temporary database at a path containing `/` and ran `CREATE TABLE t (id INT)`, `INSERT INTO t (id) VALUES (1)`, and `SELECT id FROM t`. The query output contained `{"columns":["id"],"rows":[[1]]}`. The same statements completed with a static `x86_64-unknown-linux-musl` binary built by `CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=/opt/musl/x86_64-linux-musl-cross/bin/x86_64-linux-musl-gcc cargo rustc --locked --offline --release --bin rtsql --target x86_64-unknown-linux-musl -- -C target-feature=+crt-static`. `ldd` reported `statically linked`. macOS was not run. Windows file I/O is not implemented. RISC-V 64 Linux is delivered as a static musl cross-build artifact and has not been validated on real RISC-V hardware.
- Only the main database file is encrypted. WAL, checkpoint, and dump output remain plaintext.
- There is no in-place plaintext/encrypted conversion; use `dump` and `restore`.
- Key rotation, a key-management subcommand, key caching, and `--password-file` are not included.
- Argon2id runs when an encrypted database is opened. A small dev-profile sample recorded on 2026-09-24 took about 0.35–0.39 seconds, versus sub-millisecond plaintext opens; this is an observation, not a benchmark guarantee.
- Repeatable Read and Read Committed are available; serializable isolation is not.
- Window functions, user-defined functions, time-zone types, `TIMESTAMPTZ`, and `INTERVAL` storage columns are not included.
- There is no interactive REPL, multi-user role model, or remote access control.
- CI, packaged releases, `cargo install` publication, and generated man pages are not part of this repository's current delivery surface.

## Documentation

- [简体中文 README](README.zh-CN.md)
- [Agent operations guide](rtsql-docs/SKILL.md)
- [Project snapshot](.agents/docs/SNAPSHOT.md)
- [Project roadmap](.agents/docs/tasks.md)
- [OpenSpec behavior specifications](openspec/specs/)

Package metadata declares `MIT OR Apache-2.0`.
