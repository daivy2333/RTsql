# MS09 引擎能力与 MVCC 收尾：Read Committed + 墓碑抑制语义 + NLJ + 关联子查询缓存

## Why

tasks.md MS09（2026-09-12 重排前移，用户裁定「暂时不分发，先把工作做完再谈分发」）包含三块引擎面工作，本 change 一次性收口为同一阶段成果「MS09 引擎能力与 MVCC 收尾」（用户指令 2026-09-13：把 MS09 规划成一个 change）：

1. **I033 是已实证的正确性缺陷**（MS15-Rest 收尾探针 + Plan Review 独立复现，2026-09-12，归档 change `2026-09-12-ms15-rest-correctness-batch`）：版本链 T→A（update）→ tombstone（delete）中墓碑替代者不抑制前驱，跨进程 INSERT→UPDATE→DELETE 序列扫描产出已删除行的旧版本 `[[1,10]]`（点查空集）——两步变更从扫描面消失，属「静默错误结果」类。正确性纪律先行于一切新能力。
2. **I032** `BufferPool::mark_tx_aborted` 空实现（`buffer_pool.rs:369-371`）——恢复期 mark-uncommitted-aborted 步骤空转，未提交行仅靠 header `commit_tx_id=None` 不可见性兜底；任务书授权「评估后实施或文档化」。
3. **I014（Read Committed 部分）/ I015（NLJ）/ I017（关联子查询缓存）** 为引擎能力收尾：当前仅 Repeatable Read、仅 Hash Join（非等值 ON 不可达）、关联子查询每行外层重复执行。

三块分属事务可见性 / Join 执行器 / 子查询执行器三个独立子系统，但同属 tasks.md MS09 定义的单个 milestone 成果；用户裁定聚合为一个 change。

## What Changes

1. **Read Committed 隔离级别（I014 部分，MS09-T01）**——`Database` open/构造面新增隔离级别参数，支持 `Repeatable Read`（默认，行为逐字节等价）与 `Read Committed`：RC 模式下显式事务内每条语句取执行时点的新鲜已提交快照，语句间其他事务已提交变更对后续语句立即可见；auto-commit 单语句路径行为不变。**仅 lib API 参数**（用户裁定 2026-09-13），不加 SQL `SET TRANSACTION` 语句与 CLI 面。
2. **MVCC 墓碑抑制语义（I033，MS09-T01）**——可见性评估区分「已提交墓碑」（对当前快照已提交的删除：抑制整条链，行不可见，SHALL NOT 回溯前驱）与「未提交墓碑」（不抑制、回溯前驱，产出删除前最新可见版本）；运行期与崩溃恢复后两态一致。技术方案（对照 WAL committed 集合 vs header 编码扩展）按任务书授权留实现调查裁定。
3. **I032 处置（MS09-T01 随带）**——`mark_tx_aborted` 空实现评估后实施或明确文档化「无标记」模型，随实现调查裁定并记录理由。
4. **NLJ 执行器 + 启发式切换（I015 部分，MS09-T02）**——新增 Nested Loop Join 执行器，支持任意 ON 条件；启发式：等值条件 → 既有 Hash Join 路径逐字节保持，仅非等值/无等值腿的 ON → NLJ。**非等值 JOIN 一并解锁**（用户裁定 2026-09-13）。不含 SMJ、不含代价模型。
5. **关联子查询结果缓存（I017，MS09-T04）**——关联子查询按关联参数值序列缓存结果集，缓存生命周期为单次语句执行（语句结束即弃）；相同参数值不重复执行子查询，不同参数值独立求值，缓存命中与直执行结果逐字节等价。

Delta specs（草案，随实现调查定稿）：

- 新增 `transaction-isolation-levels`：隔离级别配置面 + RC 每语句快照语义 + RR 默认零回归。
- 新增 `mvcc-tombstone-visibility`：已提交墓碑抑制整条链 / 未提交墓碑回溯前驱 / 恢复两态一致 / 既有形态零回归。
- 新增 `join-executor-selection`：等值 Hash 路径保持 + 非等值经 NLJ 可达 + 启发式选择 + NLJ 结果语义 + 既有 JOIN 零回归。
- 新增 `correlated-subquery-cache`：相同参数值复用 + 不同参数值独立 + 结果等价 + 语句生命周期 + 既有子查询零回归。

## Out of Scope / Non-goals

- Serializable / SSI（I014 剩余部分，MS09 非 target 维持）。
- SMJ（Sort-Merge Join，I015 剩余）、代价模型与 Join 重排（I016，D-candidates）。
- SQL `SET TRANSACTION ISOLATION LEVEL` 语句与 CLI 隔离级别配置面（用户裁定 2026-09-13：仅 lib API）。
- 多层关联子查询（I018 已裁定归档 2026-09-12）。
- 隔离级别运行时切换（open 时定，不支持连接中途变更）。
- 性能优化（MS08 实测域）；I038 GC 无键链、I031 撕裂树（MS08）。
- I041 测试竞态（测试基建，独立小 change）。

## 默认假设（用户未显式裁定，按合理默认补齐，可否决）

- **DA1** 子查询缓存 = 每次语句执行私有的 LRU（容量实现调查定，量级 ~1024 项），键 = 关联参数值序列；语句执行结束缓存即弃，不跨语句、不跨连接存续。
- **DA2** RC 语义 = 每语句新鲜快照（每条语句开始时取当前已提交水位）；显式事务的写冲突行为保持现状（本 change 不引入写写冲突检测）。
- **DA3** I033 技术方案与 I032「实施或文档化」由实现调查裁定（任务书预授权），design.md 记录选择理由。

## 用户决策记录（Gate 1 前集中决策，2026-09-13）

1. **RC 配置面：仅 lib API 参数**（选项「仅 lib API 参数」采纳）——Database open/构造新增隔离级别参数，默认 RR 零回归；不加 SQL 语句与 CLI 面。
2. **NLJ 范围：非等值 JOIN 一并解锁**（选项「非等值 JOIN 一并解锁」采纳）——NLJ 支持任意 ON 条件；启发式 等值→Hash（现状）、仅非等值/无等值腿→NLJ。
3. **T04 纳入本 change**（用户指令「把 ms9 规划成一个 change」；tasks.md 原注「视 MS09-T02 完成后实际场景」由本裁定取代）。
