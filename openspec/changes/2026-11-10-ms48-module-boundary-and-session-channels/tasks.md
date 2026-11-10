# 会话队列改用异步通道，包保持一个

对应路线：MS48。Gate 1 为 PASS，批准来自本次 goal，不是跳过。本文件不更新全局里程碑状态。

## 当前批次状态

- 需求与 BDD：completed。场景草图在 `proposal.md`。无行为规格增量，`.openspec.yaml` 为 `skip_specs: true`。
- 000：执行依据是 `iterations/000-session-channels/000-initial.md`。Plan Context 为 ready。Act Response 为 reported。Review Result 为 accepted。
- 001：目录已展开。执行依据是 `iterations/001-full-suite/000-initial.md`。Plan Context 为 ready。Act Response 为 reported。Review Result 为 pending。
- Persisted Evidence：none。
- 跳过项：无。

M23 只覆盖实施中新发现的实质缺陷。本计划不预建那一轮。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | 两条队列改为无界通道；既有 WAL、会话和诊断结果保持；`Cargo.toml` 仍是单 package | 完成 |
| T2 | 001 | 000 accepted | M19 分批全量一次，每批 failed 为 0 | 完成 |

## 1. 会话队列（Iteration 000）

- [x] 1.1 T1：按 `iterations/000-session-channels/000-initial.md` 的 Task Definition 执行。先加两个通道测试并看到编译失败，再改 `src/wal/buffer.rs`。通过条件是该文件列出的 GREEN 命令。不改 `Cargo.toml`，不新增 crate。

## 2. 收尾全量（Iteration 001）

- [x] 2.1 T2：T1 已被接受。按下面的分批方式跑一次全量。这是整个变更的唯一一次全量。本轮不改产品代码。执行依据是 `iterations/001-full-suite/000-initial.md`。

分批是通过条件的一部分：

1. 执行时枚举，不把规划时的文件数写死。集成测试文件来自 `ls tests/*.rs | sort`。下面记这个数量为 N。
2. 第一批单独执行：`cargo test --offline --lib --bins -- --test-threads=1`。
3. N 为 0 时没有集成测试批。N 大于 0 时，B = ceil(N / 12)，K = B × 12 − N。K 小于等于 B 时，按该排序前 K 批各 11 个文件，其余 B − K 批各 12 个。文件名去掉 `.rs` 得到 `--test` 的名字。每批一条命令：`cargo test --offline --test <名> ... -- --test-threads=1`。
4. K 大于 B，或任一集成测试批的文件数不在 8 到 12 之间时，停止并回到计划。不另造切分，不把多批合成一次无过滤的 `cargo test`。
5. 批间一条命令结束后再开始下一条。禁止一次不带 `--lib`、`--bins` 或 `--test` 过滤的 `cargo test`。禁止任何测试调用使用默认并行度。
6. 对账：全部 `--test` 名字的集合等于枚举集合，无缺失、无多余、无重复；`--lib` 和 `--bins` 只出现在第一批。
7. 每一批把退出码和 failed 计数写入 Act Response。退出码 0 且 failed 为 0 才算该批通过。任一批未跑，不得声明全量通过。
8. 某一批失败时，只把该批再跑一次。第二次仍失败则本任务失败。不改并行度，不重跑已经通过的批，不因此再跑一遍全量。

## Iteration Plan

### Iteration 000: 两条会话队列改为通道

- Tasks: T1
- Depends on: None
- 状态：已接受。Review Result 为 accepted。目录已展开。执行依据是 `iterations/000-session-channels/000-initial.md`。
- Stable result: WAL 记录和待提交事务号经无界通道传递；容量、关闭、停止后等待、checkpoint 与 close 的现有结果保持；包仍然只有一个。
- Verification boundary: 本轮只跑 Cycle 文件里的定向命令，每条都带 `--test-threads=1`，顺序执行。不跑全量。全量预算在 Iteration 001，整个变更一次。
- Diagnostic boundary: `src/wal/buffer.rs` 的发送、接收端长度和 `flush_locked` 取走。`database.rs`、`transaction/manager.rs`、`checkpoint.rs`、`buffer_pool.rs` 不在本轮的修改范围。这些文件上的测试失败时，先看 `flush_locked` 的结果是否偏离现有合同；要改它们才能通过时回到计划。
- Non-goals: 拆 crate、替换 `Notify`、给检查点或缓冲池加通道、跑全量。

平衡：两条队列由同一次 `flush_locked` 取走，容量和关闭也在这一个类型上。拆成两轮会把同一次刷盘切到两个排查范围。不拆包没有独立的产品改动，它是本轮必须保持的构建单元，不单开一轮。

### Iteration 001: 收尾全量

- Tasks: T2
- Depends on: Iteration 000 accepted
- 状态：Act Response 为 reported。Review Result 为 pending。目录已展开。执行依据是 `iterations/001-full-suite/000-initial.md`。Plan Context 为 ready。
- Stable result: 默认构建上按 M19 分批串行的一次全量，每批退出码 0 且 failed 为 0，目标对账无缺无重。
- Verification boundary: 本节 T2 的分批方式。只此一次。失败批只整批复核一次。不写「通过后再跑一次全量」。
- Diagnostic boundary: 按失败测试所在的批和目标文件定位。不把全量失败当成修改分批规则的理由。
- Non-goals: 第二次全量、非默认 feature 组合、新平台、拆 crate、改产品代码。

平衡：全量不并进 000。000 的定向测试已经能判断通道和会话归属。全量成本高，单独成轮，失败范围是某一批的测试目标。

## Requirements Traceability Matrix

| Requirement | Task | First Test | Status |
|---|---|---|---|
| 库保持一个 package，feature 不改 | T1 | `git diff -- Cargo.toml` 为空 | Covered |
| 记录经通道传递，刷盘取走前不在 WAL 文件中 | T1 | `append_enqueues_on_the_record_channel` | Covered |
| 事务号在等待返回前已在通道上，刷盘后 `Ok(())` | T1 | `commit_id_is_queued_on_the_channel` | Covered |
| 容量、关闭、停止后等待、重叠刷盘保持 | T1 | `wal::buffer::tests` 与 `wal_buffer_test` | Covered |
| `write_batch` 失败仍唤醒并返回 `Ok(())` | T1 | 保持 `flush_locked` 的失败分支；成功路径见 `commit_wait_after_shutdown_returns_ok` | Covered |
| checkpoint、close 与诊断四类保持 | T1 | `close_stops_timer_flush`；`commit_wait_classes_present` | Covered |
| 取消后刷盘循环仍在，缓冲池不加任务 | T1 | `cancel_leaves_flush_loop_running` | Covered |
| 全量只跑一次且分批串行 | T2 | T2 的分批命令 | Covered |

没有 Simplified。没有 Missing。
