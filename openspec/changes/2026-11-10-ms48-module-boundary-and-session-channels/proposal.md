# 模块边界不拆，会话队列改用异步通道

对应路线：MS48。来源：I074 的拆分余项、I012 的通道余项。约束：M19、M21、M23。已交付边界：`openspec/changes/archive/2026-10-28-ms42-module-boundaries/`、`openspec/changes/archive/2026-10-28-ms43-service-lifecycle/`。分析入口：`.agents/analysis/future-work-priority-and-dependencies.md` 里点名 I074、I012 的段落。

Gate 1 为 PASS。批准来自本次 goal 对 tasks.md 里 MS48 的 Outcome、Scope、Non-goals、Split signals、Stable result，以及「多批次串行、整个变更只允许一次全量」的授权。这是批准，不是跳过。检查项都已满足。本文件不更新全局里程碑状态。

## Why

MS48 要把已裁定的模块边界收成可独立构建的组合，或写明没有独立构建或裁剪收益所以不拆；同时把会话里尚未替换的生产者-消费者队列改成已有的异步传递，WAL、提交和检查点的职责归属保持不变。可选构建已由 cli、network、crypto 三个 feature 提供。六层依赖环仍在，没有第二套实现可替换。仍是队列且被分析点名的，只剩 WAL 记录缓冲和待提交事务号。

## What Changes

- 不新增 crate，不把现有包拆成 workspace。`Cargo.toml` 保持单一 `[package]`，feature 表不改。理由见下文默认假设 1，并在 `design.md` 留档。
- `WALBuffer` 的记录队列和待提交事务号队列从 `Mutex<Vec<_>>` 改为 `tokio::sync::mpsc` 无界通道。生产者发送，只有已持有刷盘闸门的 `flush_locked` 取走。
- 提交完成仍用按 `tx_id` 登记的 `Notify`。`append_commit_and_wait` 在 `write_batch` 失败时仍返回 `Ok(())`。
- 不改 `Database::checkpoint`、`Database::close`、`TransactionManager::commit` 的闸门与调用顺序。不给检查点或缓冲池新增通道或后台任务。

## Capabilities

### New Capabilities

无。

### Modified Capabilities

无。对外可观察的 SQL 结果、错误、WAL 内容、checkpoint、close 和诊断等待类别保持 `session-service-lifecycle` 与 `unified-diagnostics` 的现有需求。通道类型和「不拆包」是本变更的设计约束，不新增行为规格。`.openspec.yaml` 因此设置 `skip_specs: true`。

## Impact

- 代码：只改 `src/wal/buffer.rs` 的两条队列及其模块内测试。
- 不改：`src/database.rs`、`src/transaction/manager.rs`、`src/wal/checkpoint.rs`、`src/wal/writer.rs`、`src/storage/buffer_pool.rs`、`Cargo.toml`、诊断类别、磁盘格式、公开 SQL。
- 关联：MS48；I074、I012；M19（全量只在最后一轮，分批，每次测试 `--test-threads=1`）；M21（环与六层归属保持）；M23（实施中新发现的实质缺陷不在本计划预建轮次）。
- 基线：本计划在 `56da53b` 上实际跑过 `cargo test --offline --lib wal::buffer::tests -- --test-threads=1`（7 passed）和 `cargo test --offline --test wal_buffer_test -- --test-threads=1`（5 passed）。

## 需求

1. 库保持一个 package。不新增 workspace 成员，不改 cli、network、crypto 的 feature 组合。
2. WAL 记录和待提交事务号经无界异步通道传递。追加后、刷盘取走前，记录不在 WAL 文件中，并可从记录通道取出。待提交事务号在等待返回前已在其通道上。取走并完成既有刷盘后，等待返回 `Ok(())`。
3. 容量、定时器、暂停、恢复、`shutdown`、停止后的提交等待、组提交和重叠 `do_flush` 的现有结果保持。`write_batch` 失败仍唤醒等待者并返回 `Ok(())`。
4. 会话闸门、刷盘闸门、checkpoint 的暂停与恢复、`close` 的停止循环，保持 MS43 已交付的归属。诊断里一次成功插入的提交子阶段仍给出 Lock、Queue、Io、Schedule。
5. 整个变更只在最后一轮按 M19 做一次分批全量。每一批独立的 `cargo test`，都带 `--test-threads=1`。中间轮次不跑全量。

## 不做

- 解开 M21 记录的依赖环，或为了解环改公开 SQL 行为。
- 把 cli、network、crypto 再拆成 crate。feature 已经能单独构建和裁剪。
- 把 `Notify` 换成 `oneshot`，或把 `flush_notify` / `state_notify` / 会话闸门 / 刷盘闸门改掉。
- 给 `CheckpointManager` 或 `BufferPool` 加通道、`watch` 或后台任务。I012 原文里的这两项在当前代码中没有生产者-消费者队列。
- 替换其他 `Mutex`（空闲页、行锁、DDL 窗口、缓冲池缓存）。
- I093 依赖升级、I095 按需定制、诊断上报、SQL 语义与性能改动。
- 预建「修复尚未发现的缺陷」的轮次。

## 场景草图

| # | 类别 | 前置 | 动作 | 可观察结果 | 失败边界 |
|---|---|---|---|---|---|
| S1 | Happy | 未启动刷盘循环，容量大于 1 | 追加一条 `BeginTxn` | 记录通道能取出该记录；WAL 文件还没有它 | 取不出，或文件里已经有它 |
| S2 | Happy | 未启动刷盘循环 | 开始 `append_commit_and_wait` | 返回前，事务号已在待提交通道上；随后一次刷盘使等待在 2 秒内返回 `Ok(())` | 通道上没有该号，或等待不返回 |
| S3 | Happy | 容量为 3，间隔很长 | 连续追加 3 条记录 | WAL 中可解析出这 3 条 | 条数不对 |
| S4 | Happy | 缓冲中有未达容量的记录 | `shutdown` | 这些记录写入 WAL；再追加未达容量的记录，定时器不再写走 | 关闭后定时器仍在写 |
| S5 | Sad | `shutdown` 已返回 | 再追加一条并 `append_commit_and_wait` | 2 秒内返回 `Ok(())`，该记录在 WAL 中 | 挂起或返回 `Err` |
| S6 | Edge | 循环已暂停 | 暂停期间追加未达容量的记录，然后恢复 | 暂停时不写盘；恢复后定时器写走 | 暂停时写盘，或恢复后永不写 |
| S7 | Edge | 两条 `do_flush` 重叠 | 并发进入 | 每条记录可解析一次 | 条数变多或变少 |
| S8 | 组合 | 读写会话已提交一行 | `close` 后重开；另一次 `checkpoint` 后不 `close` | 已提交行仍在；`close` 后定时器不再写；`checkpoint` 后定时器仍写 | 归属被改回「close 只调公开 checkpoint」之前的循环行为 |
| S9 | 组合 | 诊断打开详细追踪 | 成功插入一行 | 提交子阶段有 Lock、Queue、Io，Schedule 不超过该阶段等待合计；该行可查到 | 四类缺失，或插入结果改变 |
| S10 | 兼容 | 单 package、现有 feature | 本变更结束时的分批全量 | 每批退出码 0 且 failed 为 0；目标无缺、无重、无多余 | 缺一批，或出现第二次全量 |
| S11 | 错误 | `write_batch` 失败 | 已登记的等待者 | 仍被唤醒，等待返回 `Ok(())` | 改成 `Err` |
| S12 | 取消 | 语句在提交前被放弃 | 放弃后继续追加 | 刷盘循环仍在，缓冲池不新增任务 | 取消停掉循环或给缓冲池加 `watch` |

S11 不新造注入器。现有合同由实现保持 `write_batch` 失败分支仍唤醒并返回 `Ok(())`，用 diff 核对。S12 在第一轮用 `cancel_leaves_flush_loop_running` 核对，收尾全量会再跑到该文件所在的一批。本变更不改取消路径的文件。

## 回滚

改动在本分支的 `src/wal/buffer.rs` 和本变更文档。回滚是还原或放弃这些提交。主分支在合并前不被改写。无磁盘格式迁移，无新的错误类型。

## 默认假设

下列假设不改变 SQL 结果、错误、WAL 内容或 MS43 的职责归属。

1. 不拆 crate。cli、network、crypto 已能按 feature 单独构建和裁剪。storage、事务、执行、规划、核心之间仍有 M21 的四条环；拆开之前必须先改这些接口，而仓库里没有另一套实现在等这个边界。`AsyncStorage` 已经能在同一个包里替换文件存储和内存存储。I093、I095 不在本范围内，不能当作拆包收益。
2. 只替换两条仍是 `Mutex<Vec<_>>` 的队列。提交完成继续用 `Notify`。检查点和缓冲池不加通道。
3. 通道使用无界 `mpsc`。容量用发送之后的接收端长度判断，由追加的调用方刷盘。不用会挡住发送方的有界 `send().await`，避免暂停或调用方自己刷盘时卡住。
4. 后台循环仍等待现有 `Notify` 和定时器，不阻塞在 `recv()` 上。这样暂停、关闭和容量刷盘仍能由已持有刷盘闸门的调用方取走通道。
5. 记录和待提交事务号保持两条通道。二者今天就不是同一次入队，合成一条消息会改变中间刷盘的配对。
6. 全量只做一次，放在最后一轮，按 M19 等分批次。每次 `cargo test` 都带 `--test-threads=1`。

## 后续变更

不创建第二个变更。拆包与通道替换没有同时发生，不需要分成两个变更来分别验证。
