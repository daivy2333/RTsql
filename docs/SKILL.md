# RTsql Agent Operations Guide

[English README](../README.md) | [简体中文 README](../README.zh-CN.md)

Use this guide to install, operate, inspect, and uninstall RTsql from an automated shell or an AI agent. RTsql is a one-shot CLI: each invocation opens a database, performs the requested work, closes the database, and exits. There is no interactive REPL.

## Operating rules

- Run commands from a trusted RTsql source checkout.
- Use an isolated `RTSQL_HOME` during tests, examples, and migrations.
- Treat SQL text, dump files, and database files as untrusted input.
- Do not use `install.sh --uninstall --purge-data` without explicit user approval; it deletes the data directory.
- Do not retry a locked database blindly. Exit code 4 means another RTsql process owns the file lock.
- Do not retry an encryption error with different keys until the intended key source and database type are confirmed.
- Quote SQL as one shell argument and use semicolons only when multiple statements are intended.
- Prefer explicit `--format` in automation so output does not depend on TTY detection.
- Use `RTSQL_KEY` instead of a command-line value when the execution environment may expose process arguments. Environment variables are not a secret store; protect the environment and process table accordingly.

## Install and deploy

### Prerequisites

Verify the required tools before building:

```bash
command -v cargo
rustc --version
bash --version
command -v strip
```

Installation requires a Rust toolchain and standard Unix user tools. `strip` is optional because the installer skips it with a warning when unavailable. RTsql currently targets Linux and macOS. The script does not use `sudo` and does not download dependencies beyond Cargo's normal build behavior.

### Build and install

From the repository root:

```bash
./install.sh
export PATH="$HOME/.local/bin:$PATH"
rtsql --version
```

The default binary path is `~/.local/bin/rtsql`. The script performs these operations:

1. Check that `cargo` exists.
2. Run `cargo build --release` in the repository.
3. Strip `target/release/rtsql` when `strip` is available.
4. Install the binary as `$PREFIX/bin/rtsql`.
5. Install completions for `$SHELL` when that shell executable exists.
6. Print a PATH hint when `$PREFIX/bin` is absent from `PATH`.

Use an isolated prefix:

```bash
PREFIX_DIR=$(mktemp -d)
./install.sh --prefix "$PREFIX_DIR"
"$PREFIX_DIR/bin/rtsql" --version
```

Skip completion installation:

```bash
PREFIX_DIR=$(mktemp -d)
./install.sh --prefix "$PREFIX_DIR" --no-completions
```

`--prefix` changes the binary location only. Completions use these user-level paths:

| Shell | Completion file |
|---|---|
| bash | `~/.local/share/bash-completion/completions/rtsql` |
| zsh | `~/.zsh/completions/_rtsql` |
| fish | `~/.config/fish/completions/rtsql.fish` |

For zsh, add `~/.zsh/completions` to `fpath` if it is not already configured.

Generate a completion script without installing it:

```bash
rtsql completions bash > rtsql.bash
rtsql completions zsh > _rtsql
rtsql completions fish > rtsql.fish
```

The completions command is hidden from `rtsql --help`; it does not open a database.

### Build without installing

```bash
cargo build --release
./target/release/rtsql --version
```

## Usage

### Database names and paths

A database argument follows one of two rules:

- A bare name such as `demo` resolves to `$RTSQL_HOME/db/demo.db`.
- An argument containing `/`, such as `./demo.db` or `/var/lib/rtsql/demo.db`, is used directly.

`RTSQL_HOME` defaults to `$HOME/.rtsql`. Use a separate home for automation:

```bash
export RTSQL_HOME=$(mktemp -d)
rtsql new demo
rtsql list
```

A subcommand name takes precedence over a bare database name. To open a database literally named `list`, pass a path such as `./list.db`.

### Main SQL command

```bash
rtsql <db> <sql>
rtsql --format <table|json|csv|tsv> <db> <sql>
```

Examples:

```bash
rtsql demo "CREATE TABLE people (id INTEGER PRIMARY KEY, name TEXT, age INTEGER)"
rtsql --format table demo "INSERT INTO people VALUES (1, 'Ada', 36), (2, 'Lin', 41)"
rtsql --format table demo "SELECT id, name, age FROM people WHERE age >= 40"
```

Without a transaction, every semicolon-separated statement is auto-committed. On a failure, stderr identifies the statement number. Statements before a failure may already be committed unless a transaction was active.

### SQL transactions

Keep the complete transaction in one invocation:

```bash
rtsql --format table demo "BEGIN; INSERT INTO people VALUES (3, 'Kai', 22); COMMIT;"
rtsql --format table demo "BEGIN; INSERT INTO people VALUES (4, 'Mira', 29); ROLLBACK;"
rtsql --format table demo "SELECT id, name FROM people ORDER BY id"
```

A successful `COMMIT` makes the transaction durable through the normal WAL and close path. `ROLLBACK` discards its writes. If input ends while a transaction is active, RTsql rolls it back, prints `uncommitted transaction was rolled back at exit` to stderr, and retains exit code 0 for the completed CLI action.

Do not send `BEGIN` in one process and `COMMIT` in another. The CLI transaction session exists only within one invocation.

### Lifecycle commands

#### Create

```bash
rtsql new demo
rtsql new ./local.db
```

`new` creates an empty database and reports an error if the target already exists.

#### List

```bash
rtsql --format table list
```

`list` enumerates `$RTSQL_HOME/db/*.db` and does not open the listed databases.

#### Schema

```bash
rtsql schema demo
```

The output contains `CREATE TABLE` statements for user tables.

#### Dump and restore

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

A restore target must be an empty database. Dump files contain SQL and row data in plaintext; store and transfer them accordingly.

#### CSV import

Create a CSV file whose header names match the target table columns:

```bash
cat > people.csv <<'CSV'
id,name,age
3,Kai,22
4,Mira,29
CSV
rtsql import demo people people.csv --csv
```

Import processes rows sequentially and commits each row. It stops at the first conversion or execution error.

### Analytics commands

```bash
rtsql --format table stats demo people
rtsql --format table sample demo people 5
rtsql --format table profile demo people --top 10
```

- `stats` reports count, null rate, distinct count, minimum, maximum, and numeric percentiles.
- `sample` uses reservoir sampling and defaults to 10 rows.
- `profile` reports column metadata and frequent values for String columns. The default top count is 5 and the accepted maximum is 20.

### Encrypted databases

Create an encrypted database:

```bash
rtsql new secure --key 'replace-with-a-password'
rtsql --key 'replace-with-a-password' secure "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT)"
rtsql --key 'replace-with-a-password' secure "INSERT INTO notes VALUES (1, 'private')"
rtsql --key 'replace-with-a-password' secure "SELECT id, body FROM notes"
```

Use the environment channel:

```bash
RTSQL_KEY='replace-with-a-password' rtsql secure "SELECT id, body FROM notes"
```

`--key` overrides `RTSQL_KEY`. `--key` applies to every command that opens a database. `list` does not open a database and is unaffected.

Empty keys are rejected before database open. Distinguish these failures by message and exit code:

| Condition | Exit | Meaning |
|---|---:|---|
| Encrypted database, no key | 5 | Supply `--key` or `RTSQL_KEY` |
| Plaintext database, key supplied | 5 | The key does not match a plaintext database |
| Wrong key | 5 | GCM authentication reports `decryption failed (wrong key or corrupted page)` |
| Corrupted page | 5 | The same authentication failure can indicate page damage |
| Empty key | 2 | Correct the key before retrying |

Migrate by dump and restore:

```bash
rtsql dump plaintext-db > database.sql
rtsql new encrypted-copy --key 'replace-with-a-password'
rtsql --key 'replace-with-a-password' restore encrypted-copy database.sql
```

The reverse migration restores into a target created without `--key`.

### Output formats

Use `--format` in automation:

```bash
rtsql --format json demo "SELECT * FROM people"
rtsql --format csv demo "SELECT id, name, age FROM people"
rtsql --format tsv demo "SELECT id, name, age FROM people"
rtsql --format table demo "SELECT id, name, age FROM people"
```

Without `--format`, an interactive terminal receives a table and redirected output receives JSON.

### Exit codes and signals

| Code | Handling |
|---:|---|
| 0 | Continue |
| 1 | Inspect the storage, path, or file-format error; do not retry until the cause is known |
| 2 | Correct the command or empty-key input |
| 3 | Inspect the SQL and the reported statement number |
| 4 | Stop; another process owns the database lock |
| 5 | Inspect database encryption state and key source before retrying |
| 130 | Process received SIGINT |
| 143 | Process received SIGTERM |

A signal received after the database opens still runs the close/checkpoint path. SIGKILL cannot be handled and relies on WAL recovery.

## Database management

### Storage layout

A centralized database named `demo` uses these paths:

```text
$RTSQL_HOME/
└── db/
    └── demo.db
```

A path-opened database can have sidecars next to its main file:

```text
/path/demo.db
/path/demo.db.wal
/path/demo.db.checkpoint
```

The `.wal` file stores redo records. The `.checkpoint` file stores the safe replay position and transaction watermark. Do not edit or delete either file independently while RTsql is running. If they are missing after a clean shutdown, RTsql can rebuild recovery state from the main file.

### File locking

RTsql takes an advisory exclusive lock when opening a database. A second process receives exit code 4 before SQL execution. For diagnosis:

1. Record the exact path passed to RTsql.
2. Check for another RTsql process using that path.
3. Stop the owner gracefully when safe.
4. Retry only after the lock is released.

A stale-looking process is not permission to remove lock files; RTsql locks the main database file itself.

### File format and encryption

A plaintext database starts with a 64-byte header and stores 4096-byte page images. An encrypted database uses the same header size, with an encryption flag, a random 32-byte salt, and 12 bytes of persisted Argon2id parameters. Its page records are 4124 bytes each and contain an AES-GCM nonce, ciphertext, and authentication tag.

Encryption applies only to the main database file. The WAL, checkpoint file, and dump output are not encrypted.

### API lifecycle

Library users should close databases explicitly:

```rust
use rtsql::database::Database;
use rtsql::storage::StorageError;

#[tokio::main]
async fn main() -> Result<(), StorageError> {
    let db = Database::open(std::path::Path::new("demo.db")).await?;
    db.close().await?;
    Ok(())
}
```

The public entry points include:

- `Database::open`
- `Database::open_with_isolation`
- `Database::open_with_key`
- `Database::execute_sql`
- `Database::execute_in_tx`
- `Database::checkpoint`
- `Database::close`

`close` flushes data, writes a checkpoint, truncates the consumed WAL, and releases the file lock.

## Uninstall

Run uninstall commands from the source checkout.

### Remove the program only

```bash
./install.sh --uninstall
```

This removes:

- `$PREFIX/bin/rtsql`
- bash, zsh, and fish completion files

It does not remove `$RTSQL_HOME` or the source checkout.

### Remove program and data

Obtain explicit user approval before running:

```bash
./install.sh --uninstall --purge-data
```

The script prints the data path before deleting `$RTSQL_HOME`, or `$HOME/.rtsql` when `RTSQL_HOME` is unset. The deletion includes all databases and sidecars below that directory and is not recoverable through RTsql.

To remove only the program while retaining a custom installation prefix, pass the same prefix used at install time:

```bash
./install.sh --uninstall --prefix "$HOME/.local-rtsql"
```

## Agent completion checklist

Before reporting an RTsql operation as complete:

1. Confirm the resolved database path.
2. Confirm whether the database is plaintext or encrypted.
3. Supply the intended key through `--key` or `RTSQL_KEY` when needed.
4. Use explicit output format for machine-readable work.
5. Handle multi-statement input transactionally when atomicity is required.
6. Check the process exit code, not only stdout.
7. Treat exit codes 4 and 5 as distinct operational failures.
8. For destructive lifecycle work, verify the target and obtain approval before deleting data.
