# ISS20 / ISS21 承载 change：索引写侧原子性与 DDL 目录事务化

## Why

MS34（取消/超时事务收尾）实施期暴露两个范围外实质缺陷，已按 project-model M20 登记为 Issue（**不按等级搁置，走完整承载闭环**）：

- **ISS20**（`.agents/issues/ISS20-index-write-side-atomicity.md`，`open`，R50）：索引写侧无原子性。
  - **E1**：显式事务内取消多行 `INSERT … SELECT` 后回滚会破坏 B-Tree 结构——后续索引插入沿「子指针 = 0」无限下降 → 栈溢出 → 进程 abort（SIGABRT）。
  - **E2**：被取消语句写入的键（**首个键之外**）残留在主键索引正向条目，同键再插报 `DuplicateKey`。
  - 根因：`src/storage/btree/index_manager.rs:239-257` 把 B-Tree 条目、根页 id 发布与 `row_to_key` 反向映射分两段 await 写入；与 `index_manager.rs:261-282` 的 merge / 换根路径之间没有写者串行化；`src/transaction/manager.rs:339-355` 的 `abort_cleanup_versions` 分桶只查反向映射，把「有正向条目、反向映射缺失」的行误判为墓碑而跳过。
- **ISS21**（`.agents/issues/ISS21-ddl-catalog-transactional.md`，`open`，R51）：被放弃的 `ALTER TABLE t DROP COLUMN v` 之后，表处于「目录已窄化 + 数据面仍混合宽度」的不一致状态。根因：`src/storage/data/table_manager.rs:792-802` 先改目录并 `flush_all`，窄化不在事务内。

`openspec/specs/statement-cancellation-outcome/spec.md`（MS34 刚合并）R2 的**已知边界**已明确把「键的释放」移出取消域的保证面，指向「存储域的索引写侧契约」。**本 change 就是那个契约的承载者**：ISS20 修好后，键释放成为已验收行为，随本 change 的规格增量进入 `openspec/specs/`。

## What Changes

- **索引写单元原子化**：`IndexManager` 的结构变更路径（`insert` / `delete` / `delete_exact` / `update`）把「B-Tree 结构变更 → 根页 id 发布 → 反向映射更新」收进**同一个不可中断单元**（单个 `spawn_blocking` 闭包）。tokio 的 blocking 任务在 JoinHandle 被丢弃后仍会跑完 ⇒ 语句被取消时该单元整体落地或整体未开始，不产生半写状态。
- **索引写侧串行化**：同一索引的结构变更串行执行（每索引一把写锁），消除并发写者对同一页的交错修改与根页 id 的丢失更新。
- **取消/回滚后的键释放**：回滚路径能识别并移除被取消语句占用的**全部**键（不止首个键）。
- **DDL 目录变更的事务化**：`ALTER TABLE DROP COLUMN` 的目录窄化延后到语句确定完成（提交点）之后；被放弃或失败的语句不留下任何目录面变更。
- **行为规格增量**：新增两个规格域——`index-write-side-atomicity`（索引写侧原子性 / 取消后键释放）与 `ddl-catalog-transactional`（DDL 目录变更的事务性）。

## 范围外（明确不做）

- 不改 MS34 已验收的取消语义（丢弃/超时是取消触发源；断开不是取消触发源）。
- 不实现分级诊断系统（I096）。
- 不做 MS35（空闲页跨重启复用）/ MS36（历史库与 dump 兼容）的范围。
- 不做与两个缺陷无关的重构。
- 不引入身份证明机制（Hash/指纹、revision pin、run-id、artifact manifest、日志 Hash 链、时间顺序证明等）与判定层；验证只落在 `cargo test` 集合内。

## 授权与批准依据

- **Gate 1（需求与范围批准）**：已批准。依据为 goal 指令原文「/openspec-autopilot 把已经规划的三个ms逐步完成，要求遵守各种规范，尤其是全量测试时候的多批次串行」+ 补充「因为这个项目有点大，三个ms也是串行实施就好」+「对于实施当中发现的所有issue候选，都直接寻找好的承载方式流程进行修复而不是等级」。批准不降低检查标准：BDD 缺口扫描、实现调查、RTM、设计闭合、Iteration 平衡审计照常完整执行。
- **Gate 2（执行就绪批准）**：已批准（同上依据）。
- 本轮只做计划，不实施、不改产品代码。

## 影响面

- 产品代码：`src/storage/btree/index_manager.rs`、`src/storage/btree/btree.rs`、`src/transaction/manager.rs`、`src/storage/data/table_manager.rs`、`src/executor/alter_table.rs`、`src/storage/catalog.rs`。
- 测试代码：`tests/` 下新增索引写侧原子性回归用例，并扩展 `tests/statement_cancellation_test.rs` 的 DDL 回收用例断言。
- 规格：`openspec/specs/index-write-side-atomicity/`、`openspec/specs/ddl-catalog-transactional/`（合并由 docs-maintainer 在收尾时执行）。
