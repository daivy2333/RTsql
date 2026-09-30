# one-shot CLI 子进程见证配方

- Status: active
- Last validated: 2026-09-30
- Environment: Linux x86_64（WSL2）；Rust 集成测试 harness（`std::process::Command` + `env!("CARGO_BIN_EXE_rtsql")`）；被测程序为 one-shot CLI `rtsql <db> <sql>`（clap 4）
- Source: MS26 change `2026-09-30-ms26-embedded-runtime` Iteration 002 Act Response「Experience Candidates」与偏差 D3（归档 carrier `openspec/changes/archive/2026-09-30-ms26-embedded-runtime/`）；夹具 `tests/wal_auto_checkpoint_test.rs::run_cli_sampling_wal` / `run_cli`；收敛过程付出 3 轮首跑失败（缺失 `db/` 目录、实参带 `.db` 后缀、stdout 管道阻塞三类症状各一次）

## 适用范围

适用于集成测试中以子进程方式端到端见证 one-shot CLI 行为：退出码、stdout/stderr 文案、文件系统副作用（`.db`/`.wal`/`.checkpoint` 产生与回落），以及**子进程存活期间**的时变状态采样（如 `.wal` 长度上限——终态收敛的通道下唯一可判别形态）。

不适用于：lib API 层行为（直接用 `Database` 构造，见 R24 进程内 close→reopen 配方）；需要交互 stdin 的形态（one-shot 无此面）；基准测量（见 R26）。

## 前置条件

- 测试通过 `env!("CARGO_BIN_EXE_rtsql")` 取二进制路径（cargo 集成测试专用变量，勿手拼 target 路径）。
- 每用例独立 `TempDir`：`RTSQL_HOME` 指向 TempDir **根**，且**预建 `db/` 子目录**——裸名实参解析为 `$RTSQL_HOME/db/<name>.db`，CLI 除 `new` 外不创建父目录，缺失即 exit 1。
- 环境隔离：`env_remove("RTSQL_KEY")`（避免宿主密钥泄漏进测试），按需 `env_remove` 其他 `RTSQL_*` 变量（如 `RTSQL_WAL_CHECKPOINT_BYTES`），再显式注入用例 env。
- `stdin(Stdio::null())`——one-shot 不读 stdin，防止用例间 fd 泄漏。

## 操作步骤

1. **实参用裸名**：`cmd.args([db_stem, sql])`，`db_stem` 不带 `.db` 后缀——CLI 自己补后缀；写 `env_on.db` 会落盘 `env_on.db.db`，测试侧采样路径与 CLI 解析路径分叉。
2. **SQL 负载按 `MAX_ARG_STRLEN` 定尺**：单个 argv 实参受 Linux 内核 128 KiB 上限，超限 spawn 直接失败（`Os { code: 7, kind: ArgumentListTooLong }`）。大负载实测参考：600 条 90 字符 INSERT + 1000 条 `SELECT 1` ≈ 87 KiB 实参（余量 ≥2 倍）；需要更大数据量时拆多语句仍受同一上限，应改走 lib 路径或 `restore -`。
3. **stdout 丢弃或落文件**：无人读取的 stdout 管道在写满（~64 KiB 缓冲）后**阻塞子进程**，表现为用例假死直至超时杀进程。长输出负载用 `Stdio::null()`；需要断言输出时读完管道（短输出）或落文件。
4. **stderr 落 TempDir 文件**（`Stdio::from(File)`）：失败原因带进断言（`assert_eq!(code, x, "stderr: {}", read(stderr_path))`），不占管道。
5. **存活期采样**（时变状态见证）：spawn 后循环 `try_wait`，未退出则以固定间隔（实测 2ms）采样目标状态（如 `fs::metadata(wal_path).len()` 取 `max_observed`），并以最小采样次数下限（如 `samples_while_alive >= 3`）防空转误判；子进程退出后收集退出码与输出。
6. **等待必须有 deadline + kill 兜底**：`try_wait` 轮询配总超时（实测 60s），超时 `kill()` + `wait()` 后 panic——one-shot CLI 必须自行终止，挂起即缺陷。

最小骨架见证据夹具 `run_cli_sampling_wal`（采样型）与 `run_cli`（终态型：`wait_with_output()` 收集三流）。

## 验证

- 判定与实施验证同源：`cargo test --test wal_auto_checkpoint_test` → **11 passed / 0 failed，exit 0**（2026-09-30，Iteration 002 Act Response Verification Evidence 第 5 项；全量 M19 分批 1590 passed / 0 failed）。夹具内断言即成功判据：退出码、stderr 点名文案、`max_observed` 有界性、`samples_while_alive >= 3`。
- 配方各要点与失败形态的对应关系经 3 轮首跑失败实证（每轮修正一类症状后 GREEN），非推测路径。

## 失败处理

| 症状 | 根因 | 处置 |
|---|---|---|
| 子进程 exit 1 且采样值恒为 0 | `db/` 目录未预建，或实参误带 `.db` 后缀致落盘 `x.db.db` | 预建 `$RTSQL_HOME/db/`；实参改裸名 |
| spawn 即失败 `Os { code: 7, kind: ArgumentListTooLong }` | SQL 实参超 `MAX_ARG_STRLEN` 128 KiB | 压缩负载或改 lib/restore 通道 |
| 用例假死直至 deadline kill | stdout 管道写满无人读，子进程阻塞在写 | `Stdio::null()` 或落文件；断言输出改读文件 |
| 采样判别面为 0（`samples_while_alive` 不足） | 负载太小子进程瞬退，或间隔过大漏采 | 增大负载/降低采样间隔；保留下限断言防误判 |
| 断言读到空 stderr | stderr 走了管道但未在 wait 前读取顺序收尾 | stderr 落文件（步骤 4） |

诊断入口：先看 stderr 文件内容（失败原因几乎总在其中），再核对 `RTSQL_HOME` 目录树实际落盘位置与测试侧路径是否同址。

## 回滚

用例全部在 TempDir 内运行，测试结束自动清理，无持久状态与外部副作用；`kill()` 兜底不留孤儿（`wait()` 已收割）。无不可回滚步骤。

## 证据

- 来源：`openspec/changes/archive/2026-09-30-ms26-embedded-runtime/iterations/002-wal-auto-checkpoint/000-initial.md`（Act Response「Experience Candidates」第 1 项 + Deviations D3；Plan Review 采信记录见同文件）。
- 夹具：`tests/wal_auto_checkpoint_test.rs::run_cli_sampling_wal`（:143）/ `run_cli`（:204）/ `long_write_workload`（:249）。
- revision：主批次提交 `a78bae1`（2026-09-30，MS26 收尾入库）；验证环境 Linux x86_64 WSL2。
- 适用限制：`MAX_ARG_STRLEN` 为 Linux 内核常量（通常 128 KiB = 32 页），macOS 无此常量、上限机制不同——跨平台定尺前需重测。
