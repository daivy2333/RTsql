# Iteration 000 / Cycle 000: 记录测量并维持刷盘、驱逐和文件锁

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `proposal.md` 的六个场景，`design.md` 的 D1 到 D5，M18 的恢复合同，M19 的串行测试约束。本轮不执行 T3。
- Excluded scope: 结构感知刷盘、no-steal、每次提交的全量检查点、重放时的索引增量维护、跨进程快照、缓存失效、新的检查点协议、脏页批量写、io_uring、索引页格式、节点锁、第二个变更、Iteration 001 的全量。

**Objective**

执行记录写明两组测量结果，以及刷盘、驱逐和文件锁保持 `0019bf1` 的代码。`src/`、`tests/` 和 `Cargo.toml` 没有差异。

**Investigation Facts**

- Starting point: 主分支 `0019bf127df5304f26e89927e336150fc6472b11`。工作区在测量结束后是干净的。没有其他活跃变更。I031 的 8.86s 和 40.7s 是 2026-09-08 debug 标定，不作为本轮数字。
- Current-State Evidence:
  - 池容量 100 页，`src/database.rs` 里 `BufferPool::new(100, …)`。
  - `BufferPool::evict_one`（`src/storage/buffer_pool.rs`）按时钟选页。脏页复制后 `write_page`，成功且未被再次弄脏才从池中移除。
  - `flush_all` 按 DashMap 遍历收集脏页，再按收集顺序逐页 `write_page`。检查点在 `src/wal/checkpoint.rs` 调用这次 `flush_all`，随后写位点并截断 WAL。
  - `src/wal/recovery.rs`：存在需要重放的已提交数据记录时不消费磁盘 B 树；`redo_count > 0` 时调用 `rebuild_pk_indexes`，从数据页重建主键、唯一和二级索引。`redo_count == 0` 不进入重建。
  - 旧索引释放走 `IndexManager::collect_all_pages_tolerant`。页类型非法时打印跳过子树。`database.rs` 在恢复后调用空闲页集合重建，遍历失败则打印放弃，打开仍继续。
  - `FileStorage::open_with_lock` 对主文件 `try_lock`。`open_readonly_with_lock` 对主文件 `try_lock_shared`。锁跟随描述符。`Database::close` 做检查点，不 drop 存储，因此不释放这把锁。
  - 只读打开在锁之后检查未决 WAL，有未决记录就拒绝。
  - 恢复计时的负载是 `tests/cli_test.rs` 的 `build_big_wal`：表 `t(id INT PRIMARY KEY)`，50 行一个显式事务，drop 且不 `close`。规划时在该函数的打开之后临时增加了 `SELECT COUNT(*)`、`close`、drop 和第二次 `Database::open`。补丁已撤掉，提交的测试仍只打印不洁打开时间。
  - 带行数的那一次 release 运行，两条命令都有 `--test-threads=1`：

```text
RTSQL_CALIBRATION_ROWS=10000 cargo test --release --offline --test cli_test calibration_recovery_time -- --ignored --nocapture --test-threads=1
RTSQL_CALIBRATION_ROWS=40000 cargo test --release --offline --test cli_test calibration_recovery_time -- --ignored --nocapture --test-threads=1
```

    | N | 构建 | 不洁打开 | 行数 | 检查点后打开 | 再次行数 | 不洁打开时的非法页 |
    |---|---|---|---|---|---|---|
    | 10000 | 885.910664ms | 536.642784ms | 10000 | 24.985182ms | 10000 | `PageId(217)` 类型 `0x00` |
    | 40000 | 3.509424522s | 3.81395909s | 40000 | 100.101745ms | 40000 | `PageId(1156)` 类型 `0x00` |

    两次不洁打开都有 `free-page-rebuild` 放弃，原因是 `expected 0x1, got 0x0`。检查点后的打开没有非法页日志。
  - `close` 之后句柄还活着就再次 `open`，返回 `DatabaseLocked`。drop 之后的第二次打开才是上表的检查点后打开。
  - 锁等待用 release 的 `rtsql`。`rtsql new` 建库，插入一行，另一进程对主文件持有独占 flock 时：

```text
rtsql --busy-timeout 300 --readonly <path> "SELECT id FROM t"
```

    退出码 4，306ms，stdout 为空，stderr 为 `{"error":"database is locked: <path>"}`。锁释放后同一命令退出码 0，5ms，stdout 为 `{"columns":["id"],"rows":[[1]]}`。
- Code and Critical Path:
  - 不洁打开：`Database::open` → `RecoveryManager::full_recover` → 重放数据页 → `rebuild_pk_indexes` → 容忍式释放旧树 → 空闲页集合重建（失败则放弃）。
  - 检查点：`Database::close` → `checkpoint_under_session_gate` → `CheckpointManager::checkpoint` → `BufferPool::flush_all`。
  - 跨进程只读：CLI `--readonly` → `Database::open_readonly_with_lock_wait` → `try_lock_shared`。写者的独占锁来自 `try_lock`。
  - 调用者保持不动：`evict_one` 的调用者是池内缺页，`flush_all` 的调用者是检查点。本轮不改这些边。

**Implementation Guidance**

本轮没有产品代码可写。把上表和锁等待的退出码抄进 Act Response，并写明两个结论：不改刷盘和驱逐；跨进程一写多读不成立，保持文件锁。然后跑下面的 diff。不要重跑计时，不要把临时的 `COUNT` 补丁加回 `tests/cli_test.rs`。

**Task Definitions**

### T1: 记录恢复代价并保持刷盘和驱逐

- Requirement/Scenario: 提案场景 1、2、3、6。不洁打开有行数和墙钟，检查点后的打开更快，刷盘和驱逐保持测量时的代码。
- Depends on: None
- Targets: 不修改文件。核对对象是 `src/storage/buffer_pool.rs` 的 `evict_one` 与 `flush_all`，`src/wal/checkpoint.rs` 的 `checkpoint`，`src/wal/recovery.rs` 的 `rebuild_pk_indexes` 调用条件。
- Current behavior: 10000 行不洁打开 536.642784ms，行数 10000，检查点后打开 24.985182ms。40000 行不洁打开 3.81395909s，行数 40000，检查点后打开 100.101745ms。不洁打开会遇到类型 `0x00` 的旧索引页并跳过子树。`redo_count > 0` 时重建索引，刷盘顺序不在这条路径上。
- Required behavior: Act Response 写上表和这句话：不改刷盘，不改驱逐，不改「有重放就重建索引」。`git diff` 为空。
- Preserve: M18。检查点完成且没有重放时使用磁盘树。脏页写出失败时脏标记按现有 `flush_all` 交还。空闲页重建失败时打开仍成功。
- Forbidden: 修改 `evict_one`、`flush_all`、`flush_pages`、`rebuild_pk_indexes` 的门控；把每次提交改成全量检查点；为了跳过重建去改重放；新增计时测试或判定脚本。
- First test: 先行观察就是上表，已经在 `0019bf1` 上跑过。Act 不重跑。
- GREEN condition: `git diff 0019bf127df5304f26e89927e336150fc6472b11 -- src tests Cargo.toml` 无输出，退出码 0。
- Verification: 上面这条 diff。有输出表示产品树已经离开测量时的代码，本轮结论不能再沿用。无输出表示刷盘和驱逐仍是测量时的代码。
- Stop when: diff 非空；或要让不洁打开变快就必须改重建门控。停止并回到计划。不在本任务里改恢复。

### T2: 记录锁等待并保持文件锁

- Requirement/Scenario: 提案场景 4、5、6。持锁时只读命令退出码 4，放锁后读到行，文件锁保持测量时的代码。
- Depends on: None
- Targets: 不修改文件。核对对象是 `src/storage/file_storage.rs` 的 `open_with_lock` 与 `open_readonly_with_lock`。
- Current behavior: 写者持有独占锁时，`--busy-timeout 300` 的只读 CLI 退出码 4，306ms，不返回行。锁释放后退出码 0，5ms，读到 `[[1]]`。`close` 不释放锁，drop 句柄才释放。
- Required behavior: Act Response 写明上述退出码、耗时和 stderr 形态，以及这句话：跨进程一写多读不成立，保持独占锁和共享锁。与 T1 同一条 diff 为空。
- Preserve: 默认等待预算 5000ms，`Some(0)` 仍只保留约 8ms 下限。`DatabaseLocked` 的错误文本和 CLI 退出码 4。多个只读共享锁可以同时持有。未决 WAL 的只读拒绝。
- Forbidden: 放宽 `try_lock` 或 `try_lock_shared`；让只读打开忽略未决 WAL；新增快照文件、缓存失效协议或检查点发布步骤；把 I071 的长会话写进本轮。
- First test: 先行观察就是上面的 CLI 结果，已经跑过。Act 不重跑。
- GREEN condition: 与 T1 同一条 `git diff` 无输出。
- Verification: 同一条 diff。有输出则锁的测量不再对应当前代码。无输出表示文件锁仍是测量时的代码。
- Stop when: diff 非空；或持锁时只读命令在当前代码里会成功返回行。停止并回到计划。不在本任务里改锁。

**Invariants**

- `redo_count > 0` 时不消费磁盘 B 树，重放后重建索引。`redo_count == 0` 时使用磁盘树。
- 读写打开持有独占锁直到描述符释放。只读打开持有共享锁，并被独占锁排除。
- 每条以后的 `cargo test` 仍带 `--test-threads=1`。本轮自己不调用 `cargo test`。
- 不新增行为规格，不改 `openspec/specs/`。

**Non-goals**

见 Excluded scope。本轮也不跑全量。

**Acceptance**

- Act Response 含有 10000 与 40000 的不洁打开、检查点后打开和行数，以及锁等待的退出码 4 和 0。
- Act Response 写明不改刷盘和驱逐，并且跨进程一写多读不成立、保持文件锁。
- `git diff 0019bf127df5304f26e89927e336150fc6472b11 -- src tests Cargo.toml` 无输出。
- 没有新增测试，没有第二遍计时，没有全量。

**Verification**

- 场景 1 到 3 的通过依据是 Plan Context 表里的行数和墙钟。Act 沿用，不重跑。
- 场景 4 和 5 的通过依据是退出码 4 与 0，以及放锁后的 `[[1]]`。Act 沿用，不重跑。
- 场景 6 的通过依据是上面的 `git diff` 无输出。这是本轮 Act 要实际执行的命令。
- 这些场景都走已经执行过的 `Database::open` 或 `rtsql --readonly`。不另做判定脚本。

**Gate 2 Readiness**

PASS。无 BLOCKED。无 SKIPPED。计划由本次目标的批准离开 draft。

**Persisted Evidence**

- Mode: none

计时和退出码可以写进 Act Response。代码没变时，同一条 release 命令可以再跑，10000 行约一秒级、40000 行约数秒，不需要另存日志目录。

**Risks and Notes**

- 墙钟不是验收阈值。另一台机器的毫秒数不同，不构成返工，只要行数、非法页路径和退出码仍与本轮事实一致。那些事实若变了，属于 Stop when，不是在本轮里改阈值。
- 空闲页重建放弃是现有失败安全行为。不为此登记缺陷，也不预建修复轮次。
- 全量在 Iteration 001。本 Cycle 不启动它。返工本 Cycle 时仍然不跑全量。

## Act Response

Status: reported

T1 和 T2 都已完成。Gate 3 与 Gate 4 通过。Gate 5 没有命中。`src/`、`tests/` 和 `Cargo.toml` 相对 `0019bf127df5304f26e89927e336150fc6472b11` 没有差异。

### 实际改动

没有修改产品符号。刷盘和驱逐仍是 `BufferPool::evict_one` 与 `flush_all`。检查点仍是 `src/wal/checkpoint.rs` 的 `checkpoint`。索引重建仍由 `redo_count > 0` 门控。文件锁仍是 `FileStorage::open_with_lock` 与 `open_readonly_with_lock`。这些符号都留在 `0019bf1`。

本文件替换了占位执行记录。`tasks.md` 里 T1、T2 的状态改为已完成。

恢复代价沿用规划时已经在 `0019bf1` 上跑过的 release 计时。本轮没有重跑。

```text
RTSQL_CALIBRATION_ROWS=10000 cargo test --release --offline --test cli_test calibration_recovery_time -- --ignored --nocapture --test-threads=1
RTSQL_CALIBRATION_ROWS=40000 cargo test --release --offline --test cli_test calibration_recovery_time -- --ignored --nocapture --test-threads=1
```

| N | 构建 | 不洁打开 | 行数 | 检查点后打开 | 再次行数 | 不洁打开时的非法页 |
|---|---|---|---|---|---|---|
| 10000 | 885.910664ms | 536.642784ms | 10000 | 24.985182ms | 10000 | `PageId(217)` 类型 `0x00` |
| 40000 | 3.509424522s | 3.81395909s | 40000 | 100.101745ms | 40000 | `PageId(1156)` 类型 `0x00` |

两次不洁打开都有 `free-page-rebuild` 放弃，原因是 `expected 0x1, got 0x0`。检查点后的打开没有非法页日志。`close` 之后句柄还活着就再次 `open`，返回 `DatabaseLocked`。drop 之后的第二次打开才是上表的检查点后打开。

不改刷盘，不改驱逐，不改「有重放就重建索引」。

锁等待沿用规划时已经跑过的 release CLI。`rtsql new` 建库并插入一行，另一进程对主文件持有独占 flock 时：

```text
rtsql --busy-timeout 300 --readonly <path> "SELECT id FROM t"
```

退出码 4，306ms，stdout 为空，stderr 为 `{"error":"database is locked: <path>"}`。锁释放后同一命令退出码 0，5ms，stdout 为 `{"columns":["id"],"rows":[[1]]}`。`close` 不释放这把锁，drop 句柄才释放。

跨进程一写多读不成立，保持独占锁和共享锁。

### 与计划的偏差

无。

### 自检

Gate 3 没有留下 Critical 或 Important，也没有遗留 Minor。产品树没有计划外修改。没有新增计时测试或判定脚本，没有把 `SELECT COUNT(*)` 补丁加回 `tests/cli_test.rs`。

### 验证

```text
git -C /home/daivy/projects/RTsql-changes/2026-11-17-ms52-torn-tree-and-cross-process diff 0019bf127df5304f26e89927e336150fc6472b11 -- src tests Cargo.toml
```

这条命令无输出，退出码 0。执行时 HEAD 是 `3f95a874fd45ef55bf844a80ceb7e9aeac1357fc`，工作区干净。刷盘、驱逐和文件锁仍是测量时的代码。

计时和锁等待沿用上面的既有结果，不重跑。没有 `cargo test`，没有全量。全量留在尚未展开的 Iteration 001。

Persisted Evidence：None required

Experience Candidates：None

未解决问题：无。

### 跳过

- 重跑计时：SKIPPED: 先行观察已经在 `0019bf1` 上跑过，Task Definition 要求 Act 不重跑。
- `cargo test` 与全量：SKIPPED: 本 Cycle 的不变量禁止调用 `cargo test`。全量属于 Iteration 001。
- `openspec instructions apply`：SKIPPED: 该命令称 `tasks.md` 没有它认得的任务条目。执行依据是本 Cycle 的 Task Definition，不重建 `tasks.md`。

## Plan Review

- Review Result: pending
