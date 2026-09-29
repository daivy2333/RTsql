# MS18 one-shot 路径性能收口（close 零工作跳过 checkpoint + CLI runtime flavor 实测定形） — Proposal

## Why

one-shot CLI（`rtsql <db> <sql>`）是本数据库的主消费形态（agent/脚本高频），但固定开销与 SQLite 存在量级差距（2026-09-24 R26 runbook 实测：`SELECT 1` 10.77ms/run vs sqlite3 1.15ms/run、峰值 RSS 16.7MiB vs 4.1MiB）。固定开销的两个已定位来源（improvements I062/I063，2026-09-24 实测登记）：

- **零写入会话 close 全价 checkpoint（I062）**：`Database::close()` 无条件转调全量 checkpoint（`src/database.rs` close → `checkpoint()` → `checkpoint_manager.checkpoint`：刷脏页 + 写位点 + WAL 重写截断）。零写入会话（WAL 无未决记录、缓冲池无脏页）三项工作全为无谓 I/O（位点 fsync + WAL 重写），每次 one-shot 白付。
- **CLI runtime 规格未定形（I063）**：`src/main.rs` `#[tokio::main]` 默认 multi_thread + num_cpus worker（实测机 32 线程）——one-shot 无并发连接需求，worker 线程与运行时结构为 RSS 大头（BufferPool 本身仅 100 页 ≈400KB）。
- **README 对比板块数字随之失效**：优化落地后「性能与资源对比」板块的 one-shot 时延/RSS 数字不再代表当前版本，需随本 change 重测刷新（R26 口径）。

**调查修正（本 change Plan 调查发现，修订 I062 原方案）**：I062 原文方案为「WAL 文件长度探针为零/低于阈值时跳过」。调查发现 DDL 不走事务包裹、不写 WAL（`src/pipeline.rs` DDL 直包执行器；`create_table.rs`/`drop_table.rs` 零 WAL 引用），schema 持久化完全依赖 checkpoint 刷脏页（`src/database.rs` MS07-T01 注释与 `schema-persistence` 语料同证）——「仅按 WAL 长度跳过」会把纯 DDL 会话（空 WAL + 脏页）的建表丢失。安全的跳过条件必须同时含「无脏页」；「低于阈值跳过」被用户裁定否决（close 后 WAL 残留 → 下次打开 `redo_count > 0` 触发全量索引重建，I031 量化 40k 行 8.86s，且违背 project-model M18「checkpoint-clean 关闭磁盘树可信」约束）。

## What Changes

- **Checkpoint 零工作跳过（T1，I062）**：checkpoint 触发时先做零工作判定——WAL 自上次 checkpoint 以来无追加记录（未决记录为零）**且**缓冲池无脏页时，checkpoint 整体无操作：SHALL NOT 刷脏页、SHALL NOT 写位点、SHALL NOT 重写/截断 WAL；任一非空时既有全量 checkpoint 行为逐字节不变。判定覆盖全部触发路径（close 自动路径、显式 `checkpoint()` 调用、open 恢复后路径），公开 API 签名不变。跳过与执行两种结果下重开（含崩溃恢复）语义与既有行为等价——「close 即落盘」承诺不变：无未决工作时本无落盘工作。
- **CLI runtime flavor 实测定形（T2，I063）**：按 R26 runbook 口径对 `current_thread` 与 `multi_thread`（现状）各跑三组实测——one-shot 时延（50 次循环）、峰值 RSS（`/usr/bin/time -v`，one-shot 与负载两形态）、负载吞吐（2000 行加载负载 elapsed）；并验证引擎内部 `spawn_blocking`（WAL/页 I/O）与 DataScan 预取在 current_thread 下的行为。**切换判据（用户批准）**：RSS 下降 ≥15% 且 one-shot 时延与负载吞吐回归 ≤5% → 切换 `current_thread`（`#[tokio::main(flavor = "current_thread")]`）；否则保持 multi_thread。三组数据与决策理由留档 design.md；先量化再决定（MS08 纪律）。
- **README 双语对比板块刷新（T3）**：全部实施完成后按 R26 重测受影响数字（one-shot 时延、RSS；引擎级数字不受本 change 影响、保持原值），更新 `README.md` / `README.zh-CN.md`「性能与资源对比」板块并附口径与测量日期。

## 范围与决策记录（用户批准 2026-09-26）

1. **跳过条件 =「WAL 空 + 无脏页」双条件**（用户裁定，选项 A）：I062 原文「低于阈值跳过重写」否决——重开恢复重建代价（I031 实测）与 M18 约束；「仅 WAL 空即跳过」否决——DDL 不写 WAL，纯 DDL 会话会丢表。零工作判定含脏页探测，WAL 空态精确谓词随 Phase 2 调查定稿（0 字节 vs 格式最小空态）。
2. **T02 切换判据量化**（用户裁定）：RSS ≥15% 降且时延/吞吐回归 ≤5%（R26 口径，多次重复取稳定值）；不满足即保持现状并把数据留档。
3. **README 随本 change 刷新**（用户裁定）：受影响数字重测更新，作为 change 收尾任务。
4. **单 change 双 Iteration**（用户裁定）：Iteration 000 = T1（close/checkpoint 零工作跳过）；Iteration 001 = T2（runtime 实测决策）+ T3（README 刷新）。T02 的实测基线基于 T01 落地后的 close 路径，顺序耦合。

## Capabilities

### Modified Capabilities

- `ms07-rest-tx-checkpoint-pushdown`: 新增「Checkpoint 零工作跳过」Requirement——双条件判定（WAL 空态 + 无脏页）、三触发路径覆盖、有未决工作全量保持、重开语义等价、DDL-only 会话不丢表（变更内携带该 capability 的 ADDED delta）。

### 不设行为 spec 的交付物（MS19 备份边界先例）

- T2 的交付物是「三组实测数据 + flavor 决策（可能含 main.rs 单行切换）」，属设计决策与测量记录，决策理由与数据入 design.md，不设行为 spec；既有 CLI 行为 spec 场景（渲染/退出码/信号停机/事务会话）与全量测试作为切换后的零回归见证。
- T3 的交付物是 README 双语对比板块数字刷新，属文档面。

## Impact

- 代码：
  - `src/wal/checkpoint.rs` — 零工作判定守卫（跳过谓词 + 空态探测接线）
  - `src/wal/writer.rs` — 复用既有 `get_current_lsn()`（文件长度查询）；空态谓词如需辅助函数在此补充
  - `src/storage/buffer_pool.rs` — 脏页探测（若无现成查询则新增只读探测 API；遵守 M17 锁序，不触碰热路径）
  - `src/database.rs` — 接线点确认（close/checkpoint/open 恢复后路径）；公开签名不变
  - `src/main.rs` — T2 视实测结果可能单行 flavor 切换
  - `README.md` / `README.zh-CN.md` — 对比板块数字刷新
- 测试：
  - 新增 checkpoint 零工作跳过矩阵测试（零工作 close 跳过〔文件级见证：无位点写入/WAL 不变〕、DML 会话全量保持、DDL-only 会话全量保持〔不丢表〕、crash 脏库 close 全量保持、显式 checkpoint 零工作无操作、重开等价）；既有 `checkpoint_redo_reduction_test.rs` 零修改通过
  - 全量回归（M19 纪律：`cargo test -- --test-threads=1`）
- 测量：
  - T2 三组实测（R26 口径，两 flavor 对比）→ 决策留档 design.md
  - T3 最终数字重测 → README 刷新
- 已知边界：
  - 不做阈值跳过（重开恢复重建代价 + M18）；跳过判定不区分加密库（WAL/位点保持明文，长度探测无差别）
  - `cargo test` 自身不经 `main.rs` runtime，T2 切换不影响测试基建
  - 跳过判定为进程内观察（BufferPool 脏位 + WAL 文件长度），不引入身份型证据机制
