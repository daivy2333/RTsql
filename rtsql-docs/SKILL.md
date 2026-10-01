---
name: rtsql-docs
description: "Use when working with the rtsql CLI or install.sh: install, create, query, back up, restore, import, analyze, encrypt, troubleshoot, or uninstall an RTsql database. Provides safe one-shot workflows, storage paths, failure handling, and cleanup rules."
compatibility: "RTsql CLI on Linux or macOS; Bash and standard Unix user tools for install.sh."
metadata:
  author: RTsql
  version: "0.2.0"
---

# RTsql Operations

[English README](../README.md) | [简体中文 README](../README.zh-CN.md)

Use this skill to operate an RTsql database from a shell or an automated agent. RTsql is a one-shot CLI: each invocation opens a database, performs the requested work, closes the database, and exits. It has no interactive REPL or database server.

## Operating rules

- Run installation commands from a trusted RTsql source checkout.
- Use an isolated `RTSQL_HOME` for examples, tests, and migrations.
- Treat SQL text, dump files, and database files as untrusted input.
- Quote SQL as one shell argument. Use semicolons only when multiple statements are intended.
- Use an explicit `--format` in automation so output does not depend on TTY detection.
- Keep `BEGIN`, its statements, and `COMMIT` or `ROLLBACK` in one invocation.
- Do not retry exit code `4` blindly; another RTsql process owns the database lock.
- Do not retry encryption errors with guessed keys. Confirm the database type and key source first.
- Use `RTSQL_KEY` instead of a command-line key when process arguments may be visible. Environment variables are not a secret store.
- Obtain explicit approval before running `install.sh --uninstall --purge-data`.

## Install and deploy

### Prerequisites

Check the build tools before installing:

```bash
command -v cargo
rustc --version
bash --version
command -v strip
```

RTsql targets Linux and macOS. `strip` is optional; the installer skips it with a warning when it is unavailable. The installer does not use `sudo` and does not download anything outside Cargo's normal build behavior.

### Install

From the repository root:

```bash
./install.sh
source ~/.bashrc
rtsql --version
```

The default installation is `$HOME/.local/bin/rtsql`. The installer:

1. Checks for `cargo`.
2. Builds the release binary in the repository.
3. Strips the binary when `strip` is available.
4. Installs `$PREFIX/bin/rtsql`.
5. Installs completions for the detected shell.
6. Adds the selected binary directory to `$HOME/.bashrc` when the matching PATH export is absent.

Opening a new terminal is equivalent to `source ~/.bashrc`. The current shell does not receive the new PATH value until it reloads the file.

Use another installation prefix:

```bash
PREFIX_DIR=$(mktemp -d)
./install.sh --prefix "$PREFIX_DIR"
source ~/.bashrc
rtsql --version
```

Skip completion installation:

```bash
./install.sh --prefix "$PREFIX_DIR" --no-completions
```

`--prefix` changes the binary location. Completions remain in the standard user directories:

| Shell | Completion file |
|---|---|
| bash | `~/.local/share/bash-completion/completions/rtsql` |
| zsh | `~/.zsh/completions/_rtsql` |
| fish | `~/.config/fish/completions/rtsql.fish` |

For zsh, add `~/.zsh/completions` to `fpath` when needed. Generate a completion script without installing it:

```bash
rtsql completions bash > rtsql.bash
rtsql completions zsh > _rtsql
rtsql completions fish > rtsql.fish
```

The `completions` command is hidden from `rtsql --help` and does not open a database.

Build without installing:

```bash
cargo build --release
./target/release/rtsql --version
```

## Resolve a database

A database argument follows two rules:

- A bare name such as `demo` resolves to `$RTSQL_HOME/db/demo.db`.
- An argument containing `/`, such as `./demo.db` or `/var/lib/rtsql/demo.db`, is used as a file path.

`RTSQL_HOME` defaults to `$HOME/.rtsql`. Use a separate home for automation:

```bash
export RTSQL_HOME=$(mktemp -d)
rtsql new demo
rtsql list
```

A subcommand name takes precedence over a bare database name. To open a database literally named `list`, use a path such as `./list.db`.

`:memory:` is a reserved main-command target for a database that never touches disk:

```bash
rtsql :memory: "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT); INSERT INTO t VALUES (1, 'Ada'); SELECT name FROM t"
```

It creates no `.db`, `.wal`, or `.checkpoint` file, `close()` and `checkpoint()` do nothing, and the data is gone when the process exits. Use it for tests, throwaway calculations, and multi-statement pipelines that should leave no cleanup behind. Reject lifecycle subcommands, `--key`, and `--readonly` for `:memory:` with exit code `2` instead of silently ignoring them.

## Command reference

### Main command

`rtsql <db> <sql>` opens the database, runs the SQL, and exits. `<db>` is a bare name or a file path (see Resolve a database). `:memory:` targets a throwaway in-memory database instead.

| Surface | Effect |
|---|---|
| `--format <table\|json\|csv\|tsv>` | Output format; default is `table` on a TTY and `json` otherwise. Global flag. |
| `--key <key>` | Key for an encrypted database; `RTSQL_KEY` is the fallback. Global flag; applies to every command that opens a database. |
| `--readonly` | Open the main database read-only (shared advisory lock; writes are refused at plan time with SQLSTATE `25006`, exit `3`). Main command only; rejected with exit `2` alongside a subcommand. |
| `;`-separated statements | Each statement runs as its own auto-commit transaction, in order, with fail-fast reporting (`statement k of n failed`). Wrap atomic work in `BEGIN` … `COMMIT` inside one invocation. |

### Lifecycle subcommands

| Command | Example | Effect |
|---|---|---|
| `new` | `rtsql new demo` | Create an empty database; fails if the target already exists. This is the only creation channel for a missing database. |
| `list` | `rtsql list` | Enumerate `$RTSQL_HOME/db/*.db` without opening the files. |
| `schema` | `rtsql schema demo` | Print user-table DDL. |
| `dump` | `rtsql dump demo > demo.sql` | Export SQL text (`CREATE TABLE` + `INSERT` rows). A missing or damaged data-page chain is reported on stderr (see Behavioral notes) while the reachable part is still exported. |
| `restore` | `rtsql restore demo-restored demo.sql` | Load a dump into an existing, still-empty target database (create it with `new` first); `-` reads the dump from stdin. |
| `import` | `rtsql import demo people people.csv --csv` | Load a CSV file into an existing table; the CSV header must match the column names; rows commit one by one. |
| `delete` | `rtsql delete demo --dry-run` | Remove a database file and its `.wal`/`.checkpoint` sidecars and report each file. `--dry-run` previews only; a locked file is refused with exit `4`. |
| `stats` | `rtsql stats demo people` | Per-column row count, null rate, distinct count, min/max, numeric percentiles. |
| `sample` | `rtsql sample demo people 5` | Reservoir sample of N rows; default 10, `N` of `0` or a non-integer is a usage error. |
| `profile` | `rtsql profile demo people --top 10` | Frequent values for String columns; default top 5, maximum 20. |

### Environment variables

| Variable | Effect |
|---|---|
| `RTSQL_HOME` | Base directory for bare-name resolution (`<home>/db/<name>.db`); defaults to `$HOME/.rtsql`. |
| `RTSQL_KEY` | Default key for encrypted databases; an explicit `--key` wins. An empty value is refused with exit `2`. |
| `RTSQL_WAL_CHECKPOINT_BYTES` | Auto-checkpoint threshold in bytes (default 16 MiB); `0` disables. An invalid value is refused before the database opens with exit `2`. |

### Exit codes

| Code | Meaning |
|---:|---|
| 0 | Success |
| 1 | General error: I/O, storage, missing database, or output format |
| 2 | CLI usage error (bad flag combination, empty key, invalid environment value) |
| 3 | SQL parse or execution error |
| 4 | Database is locked by another process |
| 5 | Encryption-key error |
| 130 | SIGINT received |
| 143 | SIGTERM received |

Per-code action guidance is in Select output and handle failures.

### Behavioral notes

- The main command does not create a missing database. A missing bare name or path is refused with exit `1` and a `rtsql new <name>` hint, and no file is created. The `new` subcommand and the library API keep their creation behavior.
- `dump` runs a data-page chain integrity precheck for each user table. When a table's page chain ends before the catalog-persisted tail page, one warning line goes to stderr — naming the table, the catalog tail page, and the chain end — while the reachable rows are still exported. stdout stays pure SQL and the exit code stays `0`.
- `SELECT` resolves `FROM` table aliases across all clauses: qualified references such as `u.id` work in `SELECT`, `ON`, `WHERE`, `GROUP BY`, aggregate arguments, and `ORDER BY`; an unknown qualifier is a plan-time error.

## Execute SQL

The main command accepts a database and SQL text:

```bash
rtsql <db> <sql>
rtsql --format <table|json|csv|tsv> <db> <sql>
```

Create and query a table:

```bash
rtsql new demo
rtsql demo "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT, age INTEGER)"
rtsql --format table demo "INSERT INTO people VALUES (1, 'Ada', 36), (2, 'Lin', 41)"
rtsql --format table demo "SELECT id, name, age FROM people WHERE age >= 40"
rtsql schema demo
```

Without an explicit transaction, each semicolon-separated statement is committed independently. If a later statement fails, earlier statements may already be committed. The error identifies the failed statement.

### Transactions

Keep the complete transaction in one invocation:

```bash
rtsql --format table demo "BEGIN; INSERT INTO people VALUES (3, 'Kai', 22); COMMIT;"
rtsql --format table demo "BEGIN; INSERT INTO people VALUES (4, 'Mira', 29); ROLLBACK;"
rtsql --format table demo "SELECT id, name FROM people ORDER BY id"
```

`COMMIT` makes the transaction durable through the normal WAL and close path. `ROLLBACK` discards its writes. If input ends with an active transaction, RTsql rolls it back, reports the rollback on stderr, and keeps exit code `0` for the completed CLI action.

Do not send `BEGIN` in one process and `COMMIT` in another.

## Manage data

### Create and inspect

```bash
rtsql new demo
rtsql new ./local.db
rtsql list
rtsql schema demo
```

`new` creates an empty database and fails if the target already exists. `list` enumerates `$RTSQL_HOME/db/*.db` without opening the listed databases. `schema` prints user-table DDL.

### Dump and restore

```bash
rtsql dump demo > demo.sql
rtsql new demo-restored
rtsql restore demo-restored demo.sql
rtsql --format table demo-restored "SELECT id, name, age FROM people ORDER BY id"
```

Read a dump from stdin:

```bash
rtsql new demo-stdin
rtsql dump demo | rtsql restore demo-stdin -
```

A restore target must be empty. Dump files contain SQL and row data in plaintext; protect them as plaintext database exports.

### Import CSV

The target table must exist, and the CSV header must match its column names:

```bash
cat > people.csv <<'CSV'
id,name,age
3,Kai,22
4,Mira,29
CSV
rtsql import demo people people.csv --csv
```

Import processes rows sequentially and commits each row. It stops at the first conversion or execution error.

### Analyze data

```bash
rtsql --format table stats demo people
rtsql --format table sample demo people 5
rtsql --format table profile demo people --top 10
```

- `stats` reports count, null rate, distinct count, minimum, maximum, and numeric percentiles.
- `sample` uses reservoir sampling and defaults to 10 rows.
- `profile` reports column metadata and frequent values for String columns. The default top count is 5 and the maximum is 20.

## Use encrypted databases

Create an encrypted database and open every database command with its key:

```bash
rtsql new secure --key 'replace-with-a-password'
rtsql --key 'replace-with-a-password' secure "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT)"
rtsql --key 'replace-with-a-password' secure "INSERT INTO notes VALUES (1, 'private')"
rtsql --key 'replace-with-a-password' secure "SELECT id, body FROM notes"
```

Use the environment channel when process arguments should not contain the key:

```bash
RTSQL_KEY='replace-with-a-password' rtsql secure "SELECT id, body FROM notes"
```

`--key` overrides `RTSQL_KEY` and applies to every command that opens a database. `list` does not open a database and is unaffected.

| Condition | Exit | Action |
|---|---:|---|
| Encrypted database, no key | 5 | Supply `--key` or `RTSQL_KEY` |
| Plaintext database, key supplied | 5 | Check the database type and remove the key |
| Wrong key | 5 | Confirm the intended key; do not guess |
| Corrupted encrypted page | 5 | Treat the authentication failure as possible page damage |
| Empty key | 2 | Correct the key before opening a database |

Migrate between plaintext and encrypted databases with dump and restore:

```bash
rtsql dump plaintext-db > database.sql
rtsql new encrypted-copy --key 'replace-with-a-password'
rtsql --key 'replace-with-a-password' restore encrypted-copy database.sql
```

Restore into a target created without a key for the reverse migration. Encryption covers the main database file only; `.wal`, `.checkpoint`, and dump output remain plaintext.

## Select output and handle failures

Use an explicit format in scripts:

```bash
rtsql --format json demo "SELECT * FROM people"
rtsql --format csv demo "SELECT id, name, age FROM people"
rtsql --format tsv demo "SELECT id, name, age FROM people"
rtsql --format table demo "SELECT id, name, age FROM people"
```

Without `--format`, an interactive terminal receives a table and redirected output receives JSON.

| Exit code | Meaning | Action |
|---:|---|---|
| 0 | Success | Continue |
| 1 | General I/O, storage, missing-database, or format error | Inspect the path and storage before retrying |
| 2 | CLI usage error | Correct the command or empty-key input |
| 3 | SQL parse or execution error | Inspect the SQL and statement number |
| 4 | Database is locked | Find the owning process and release the lock |
| 5 | Encryption-key error | Confirm database type and key source |
| 130 | SIGINT | Inspect whether the close path completed |
| 143 | SIGTERM | Inspect whether the close path completed |

A handled SIGINT or SIGTERM runs the normal close/checkpoint path after the database opens. SIGKILL cannot be handled and relies on WAL recovery.

## Manage files and locks

A centralized database named `demo` uses this layout:

```text
$RTSQL_HOME/
└── db/
    └── demo.db
```

A path-opened database can have sidecars beside its main file:

```text
/path/demo.db
/path/demo.db.wal
/path/demo.db.checkpoint
```

The `.wal` file stores redo records. The `.checkpoint` file stores the safe replay position and transaction watermark. Do not edit or delete either file independently while RTsql is running. Missing sidecars after a clean shutdown can be rebuilt from the main file.

RTsql takes an advisory exclusive lock when opening a database. A second process receives exit code `4` before SQL execution. Record the exact path, find the owning process, stop it gracefully, and retry only after the lock is released. Do not remove lock files to bypass the lock.

`--readonly` takes an advisory shared lock instead, which enables many readers over one file while a writer stays alone:

```bash
rtsql --readonly demo "SELECT count(*) FROM people"
rtsql --readonly demo "ATTACH './other.db' AS other; SELECT p.name FROM people p JOIN other.team t ON t.id = p.id"
```

Read-only sessions never write: they skip recovery and checkpoint, and they refuse `INSERT` / `UPDATE` / `DELETE` / `UPSERT` / `REPLACE` and all DDL at plan time with SQLSTATE `25006` (exit code `3`). `SELECT`, `ATTACH`, `DETACH`, and transaction statements still work, and an attached database opens read-only too.

Two refusals are specific to read-only opens:

- Exit code `1` with a message naming pending WAL: the database has unreplayed `.wal` records. Open it once without `--readonly` to complete recovery, then retry read-only. The refusal leaves every file unchanged.
- Exit code `2`: `--readonly` was combined with a subcommand. `--readonly` belongs to the main command; subcommands keep their read-write open.

An encrypted database opens read-only with `--key` as usual. A read-write session excludes readers and other writers, and a read-only session excludes a read-write one; conflicts are reported immediately as exit code `4` rather than waited out.

A plaintext database starts with a 64-byte header and stores 4096-byte page images. An encrypted database uses the same header size, a 32-byte random salt, persisted Argon2id parameters, and 4124-byte encrypted page records containing a nonce, ciphertext, and authentication tag.

## Keep the WAL bounded in long sessions

A read-write session truncates the `.wal` file at `close()`. A long-running session that keeps writing also checkpoints automatically once the bytes appended since the last checkpoint reach a threshold (16 MiB by default), which keeps the file from growing without bound:

```bash
RTSQL_WAL_CHECKPOINT_BYTES=1048576 rtsql demo "INSERT INTO log (id, line) VALUES (1, 'first')"
```

- The check runs at each commit boundary — after an auto-commit statement and after an explicit `COMMIT` — and is deferred while another transaction of the same session is still active, so it never interleaves with in-flight work.
- `RTSQL_WAL_CHECKPOINT_BYTES=0` disables the automatic checkpoint. The `.wal` then grows for the whole session and `close()` still truncates it.
- A non-numeric, negative, or overflowing value is refused before the database opens: exit code `2` with a message naming the variable and the rejected value, and no file is created. Fix the value or unset the variable.
- If the automatic checkpoint fails, the statement that triggered it reports the failure and names the checkpoint (exit code `3`). Committed data is unaffected — the commit is fsynced first, and a later commit retries. Treat a repeated `checkpoint failed` message as a disk or file-system problem on the database's directory and check the free space and write permissions of the `.wal` and `.checkpoint` sidecars.
- Read-only and `:memory:` sessions hold no WAL, so the setting does nothing there.

## Uninstall

Run uninstall commands from the source checkout.

Remove the program while keeping data:

```bash
./install.sh --uninstall
```

This removes the selected `$PREFIX/bin/rtsql`, shell completion files, and the PATH export line for that prefix from `$HOME/.bashrc`. It leaves `$RTSQL_HOME`, its databases, and the source checkout intact.

For a custom prefix, pass the same prefix used during installation:

```bash
./install.sh --uninstall --prefix "$HOME/.local-rtsql"
```

Remove the program and all data only after explicit approval:

```bash
./install.sh --uninstall --purge-data
```

The installer prints the data path before deleting `$RTSQL_HOME`, or `$HOME/.rtsql` when `RTSQL_HOME` is unset. The deletion includes databases and sidecars and cannot be recovered through RTsql.

## Completion checklist

Before reporting an RTsql operation as complete:

1. Confirm the resolved database path.
2. Confirm whether the database is plaintext or encrypted.
3. Supply the intended key through `--key` or `RTSQL_KEY` when needed.
4. Use an explicit output format for machine-readable work.
5. Keep multi-statement atomic work inside one transaction invocation.
6. Check the process exit code, not only stdout.
7. Treat exit codes `4` and `5` as distinct operational failures.
8. Verify the target before destructive lifecycle commands.
