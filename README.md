# RTsql

[English](README.md) | [简体中文](README.zh-CN.md) | [Agent operations guide](docs/SKILL.md)

RTsql is an embedded relational database written in Rust. It uses Tokio tasks for asynchronous I/O, stores each database in one main file, and provides a one-shot CLI for SQL execution and database management.

This repository currently targets Linux and macOS. It does not require a database server.

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
export PATH="$HOME/.local/bin:$PATH"
rtsql --version
```

`install.sh` builds the release binary, installs it under `~/.local/bin`, installs completions for the current shell when available, and prints a PATH hint when needed. It does not use `sudo` or invoke a separate downloader; Cargo may fetch declared Rust dependencies during the build.

Use another installation prefix or skip completions with:

```bash
./install.sh --prefix "$HOME/.local-rtsql"
./install.sh --prefix "$HOME/.local-rtsql" --no-completions
```

The prefix controls the binary location. Completion files remain in their standard per-user shell directories.

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
| `rtsql schema <db>` | Print user-table DDL |
| `rtsql dump <db>` | Export DDL and rows as SQL text |
| `rtsql restore <db> <file>` | Restore a dump into an empty database; `-` reads stdin |
| `rtsql import <db> <table> <file> --csv` | Import CSV with a matching header row |
| `rtsql stats <db> <table>` | Summarize count, null rate, distinct values, ranges, and numeric percentiles |
| `rtsql sample <db> <table> [n]` | reservoir-sample rows; default `n` is 10 |
| `rtsql profile <db> <table> [--top n]` | Profile columns and frequent String values; default top count is 5, maximum 20 |
| `rtsql completions <bash\|zsh\|fish>` | Generate a completion script; this command is hidden from `--help` |

### Output formats

`--format` accepts `table`, `json`, `csv`, and `tsv`. RTsql uses `table` for an interactive terminal and `json` otherwise unless the option is set.

### Exit codes

| Code | Meaning |
|---:|---|
| 0 | Success |
| 1 | General I/O, storage, or format error |
| 2 | CLI usage error |
| 3 | SQL parse or execution error |
| 4 | Database is locked by another process |
| 5 | Encryption-key error |
| 130 / 143 | Terminated by SIGINT / SIGTERM |

## SQL and engine capabilities

- **DDL and DML:** `CREATE TABLE`, `DROP TABLE`, `SELECT`, `INSERT`, `UPDATE`, and `DELETE`.
- **Queries:** `WHERE`, `JOIN`, `GROUP BY`, `HAVING`, `ORDER BY`, `LIMIT`, and `OFFSET`.
- **Expressions:** `IN`, `BETWEEN`, `LIKE`, `IS NULL`, `NOT`, `CASE`, `COALESCE`, `CAST`, and arithmetic operators.
- **Scalar functions:** `upper`, `lower`, `length`, `substr`, `replace`, `trim`, `abs`, `round`, `floor`, and `ceil`.
- **Date and time:** `DATE`, `TIMESTAMP`, `now`, `date`, `year`, `month`, `day`, `hour`, `minute`, `second`, `date_trunc`, `datediff`, and `INTERVAL` arithmetic.
- **Subqueries:** scalar subqueries, `IN`, `EXISTS`, derived tables, and correlated subqueries with statement-level result reuse for repeated parameters.
- **Grouping:** group by column, alias, expression text, or ordinal position.
- **Constant queries:** expressions without `FROM`, such as `SELECT 1 + 1`.
- **Transactions:** explicit library transactions and CLI `BEGIN` / `COMMIT` / `ROLLBACK` sessions.
- **MVCC:** Repeatable Read by default and opt-in Read Committed through `Database::open_with_isolation`.
- **Storage:** persistent schema, B-Tree primary-key indexes, WAL with frame checksums, checkpointing, crash recovery, and page reuse.

The engine uses explicit type checks and three-valued predicate logic. It does not implicitly convert incompatible SQL types.

## Encryption model

An encrypted database uses a 64-byte plaintext header followed by encrypted page records. The header contains a random 32-byte Argon2id salt and persisted KDF parameters. Each 4096-byte page is stored as a 4124-byte record containing a 12-byte nonce, ciphertext, and a 16-byte AES-GCM authentication tag. The page ID is authenticated as additional data.

The `.wal` and `.checkpoint` sidecar files remain in their existing plaintext formats. Encrypting a main database file does not encrypt an exported dump or these sidecars.

## Library API

The main entry points are:

- `Database::open(path)` — open or create with Repeatable Read.
- `Database::open_with_isolation(path, level)` — select Repeatable Read or Read Committed.
- `Database::open_with_key(path, isolation, key)` — open plaintext or encrypted storage.
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

The repository also contains Criterion benchmarks and SQLite comparison benchmarks under `benches/`.

## Known limitations

- Linux and macOS are supported; Windows file I/O is not implemented.
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
- [Agent operations guide](docs/SKILL.md)
- [Project snapshot](.claude/docs/SNAPSHOT.md)
- [Project roadmap](.claude/docs/tasks.md)
- [OpenSpec behavior specifications](openspec/specs/)

Package metadata declares `MIT OR Apache-2.0`.
