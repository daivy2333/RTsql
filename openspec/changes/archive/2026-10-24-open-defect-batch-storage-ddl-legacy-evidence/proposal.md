# 开放缺陷批承载 change：存储/DDL 实质缺陷 + 旧格式兼容证据缺口

## Why

MS35（空闲页跨重启复用）与 MS36（历史库与 dump 兼容）的 Plan Review 各报告了一批缺陷/证据缺口，MS34 修复批也留了 4 个 open Issue。按 project-model M20 与用户指令「**对于实施当中发现的所有 issue 候选，都直接寻找好的承载方式流程进行修复而不是等级**」，本 change 是这批候选的承载者——**不按严重等级分批、不搁置**。

本次共 12 项候选，逐项独立复核后的结论：

| 编号 | 复核结论（2026-10-24，独立复核） |
|---|---|
| ISS22 | **仍成立**。`ALTER TABLE DROP COLUMN` 的改写路径同时持有**两条独立的尾游标**（旧表 `data_page_tail` 与窄化副本 `new_meta.data_page_tail`），页写满时两腿各自新分配页并**互相覆盖链上的 `next` 指针**，先分配的页成为孤儿 ⇒ 已提交行丢失（实测 40 行剩 25）。 |
| ISS23 | **仍成立**（读码 + 既有 gdb 现场）。`btree.rs:327` 的下降递归可形成环 ⇒ 栈溢出、进程 abort。**归属判定**：首要候选通路是 ISS22 之外的 B-2（同一页被释放两次 ⇒ 同一页号被交给两个对象 ⇒ 树结构成环），本 change 先修 B-2 再复现判定。 |
| ISS24 | **仍成立**（实测 30 行剩 29、2000 行剩 1999）。根因登记为 Inferred；本次独立复核收敛出**两个候选机制**：M1 = ISS22 的链分叉（改写副本落在孤儿页上）；M2 = 「已回滚墓碑仍抑制前驱版本」的活性规则违例（`pointed` 集合不识别 aborted 标记，违反已验收的 `mvcc-tombstone-visibility` R1）。两者修复点不同，RED 复现须判别。 |
| ISS25 | **仍成立**（读码结论，未做测试验证）。`deserialize_tuple_width`（`tuple.rs:189-222`）按位置取值、窄行尾部补 NULL、不校验列序；DDL 提交点窗口内 2 宽元组在 3 列目录下被读成 `[id, w, NULL]`。全部读路径都走它。 |
| B-1 | **仍成立**。释放可分配判定挂「释放完成编号」上，而刷盘（`buffer_pool.rs:216-233` / `:264-287`）**先清脏标记再写**，写失败时脏页静默变干净 ⇒ 锚点写可能从未落盘；此后一次成功刷盘仍会提升该释放 ⇒ 未持久化的释放被提前变成可分配。 |
| B-2 | **仍成立**（本次独立复核确认）。`btree.rs:610` `merge_leaves` 释放 `right_id`，`MergeInfo.freed_page_id` 又把同一页号带回 `btree.rs:672` `handle_child_merge` 再释放一次 ⇒ **同一页进入待释放集合两次** ⇒ 提升后同一页号可被分配两次 ⇒ 交给两个对象。 |
| B-3 | **不承载**。该形态（DDL 语句边界的有序落盘会一并落盘先前 `DROP TABLE` 的目录删除）**已被 MS35/MS36 已验收规格显式界定为范围外既有行为**（`drop-persistence-boundary` R1 的崩溃点已改到「`DROP TABLE` 之后、任何后续 DDL 成功返回之前」）。修复它等于推翻已验收的崩溃模型，不属缺陷承载范围。 |
| B-4 | **仍成立**。释放路径（`drop_table` / `truncate_table` / `DROP INDEX`）用未检查的 `SlottedPageRef::get_slot_data`（`slotted_page.rs:125-129`，`&self.data[start..end]` 无边界检查）扫描溢出定位符 ⇒ 损坏页上越界切片 panic ⇒ 进程 abort。同模块的 `collect_data_chain_strict` 已用 integrity 模块的有界读并注明「生产访问器会 panic」。 |
| C-1 | **仍成立（证据缺口）**。`legacy-format-compat` 的能力矩阵中 G2 一代的 8 个能力格中 6 格标注「无 G2 专属断言（E7）」，仓库内无 G2 布局的文件级样本。 |
| C-2 | **仍成立（证据缺口）**。`legacy-format-compat` 的三条一次性条款（恢复出的库可继续写入 / 旧 G1 可由当前构建 dump / G1 可连续写两次）目前只有一次性 CLI 实测支撑（E2、E4 与 dump 恢复场景的续写/再导出分句），无常态化用例。 |
| C-3 | **仍成立（记录保真度）**。MS36 迁移路径实测记录的命令列是裸名（`rtsql new p1.db` / `rtsql dump g1_dump_probe.db`），CLI 把裸名解析到 `$RTSQL_HOME/db/<name>.db`（`src/cli/resolve.rs:30`）⇒ 按原文复跑以「does not exist」退出 1。**所记录的退出码与结果本身可复现**，失配的只是命令写法。 |

## What Changes

### 缺陷面（产品代码）

1. **索引页归属唯一性**：`BTree` 叶合并路径对被合并掉的右叶页**只释放一次**；被释放的页**不会被交给两个对象**。同一根因链上的 `ISS23`（插入路径下降成环 ⇒ 栈溢出）随之判定并收敛。
2. **DDL 改写面的数据链一致性**：`ALTER TABLE DROP COLUMN` 的改写路径使用**单一尾游标**，页写满时不产生链分叉 ⇒ 已提交行不丢（ISS22）；被放弃的 DDL **不隐藏**它已开始改写的行（ISS24）。
3. **DDL 改写窗口的读一致性**：改写在途的窄化状态对读路径**可见** ⇒ 改写窗口内的读**不发生位置性静默错读**（ISS25）。**不改磁盘格式**（见 design D7）。
4. **释放路径的有界读**：释放路径改用 integrity 模块的**有界读** ⇒ 损坏页上不 panic、不 abort（B-4）。
5. **释放可分配判定的耐久性前提**：刷盘失败**不得静默丢弃**脏页写；释放的可分配判定以**锚点写已持久**为准，而非「已发出」（B-1）。

### 证据面（测试 / 样本 / 记录）

6. **G2 文件级样本**：用仓库之外的隔离克隆 + `cargo build --offline --bin rtsql` 构建旧构建，产出 **G2 布局**的库文件样本，固定入库为固定输入；派生量（如恢复行数）由断言锁定并与来源说明同步（C-1）。
7. **三条常态化用例**：恢复出的库可继续写入、旧 G1 可由当前构建 dump、G1 可连续写两次——从一次性 CLI 实测转为 `tests/legacy_compat_test.rs` 的常态化用例（C-2）。
8. **迁移路径记录的命令列**更正为可直接执行的形式（路径形式），并使该批命令的行为由常态化用例承担（C-3）。

### 行为规格

新增 4 个规格域：`index-page-ownership`、`ddl-rewrite-consistency`、`release-path-bounded-read`、`release-durability-precondition`。

**ISS24 的目标行为已由已验收规格承载**（`mvcc-tombstone-visibility` R1「未提交/已回滚墓碑 SHALL NOT 抑制、评估回溯前驱」+ `ddl-catalog-transactional` R3），本 change **不重述**该行为，只修实现并补锁定用例。

**C 组不产生规格增量**：三条条款的行为已写在 MS36 的待合并增量 `legacy-format-compat` 中；C 组交付的是**样本 + 常态化用例 + 记录更正**，规格文本的证据索引更新作为**收尾交接项**交 `openspec-docs-maintainer` 在合并时执行（本轮不修改 `openspec/specs/`）。

## 范围外（明确不做）

- 不改 MS35 / MS36 的**已验收**行为：MS35 的 `free-page-lifecycle`（含 R1「释放的持久化判定」）、MS36 的 `legacy-format-compat` 支持范围与 G1/G2/G3 代际定义。
- 不修 **B-3**（已被已验收规格显式界定为范围外既有行为）；不改 MS35 的崩溃模型。
- 不改磁盘格式：不动 `VersionHeader` 布局、不动 `__tables` / `__columns` 目录行布局（会破坏 MS36 已验收的 G1/G2/G3 兼容面）。
- 不做 `ALTER TABLE ADD COLUMN` 路径的语义变更；不改 MS21 的宽度感知解码（`deserialize_tuple_width` 的窄行补 NULL 语义对 ADD COLUMN 仍然成立）。
- 不实现分级诊断系统（I096）；不做尾部截断与整库整理；不做 I038（无键行旧版本 GC）。
- 不引入身份证明机制（Hash/指纹、revision pin、run-id、artifact manifest、日志 Hash 链、时间顺序证明及其采集/审计工具）；不建测试框架外的判定层；不用 timer 阈值作行为判据。
- 不登记 / 修改 / 关闭 Issue（属 `openspec-experience-recorder`）；不同步全局 tasks / SNAPSHOT / M / R / I（属 `openspec-docs-maintainer`）。

## 授权与批准依据

- **Gate 1（需求与范围批准）**：**已批准**。依据为 goal 指令原文「/openspec-autopilot 把已经规划的三个ms逐步完成，要求遵守各种规范，尤其是全量测试时候的多批次串行」+ 补充「因为这个项目有点大，三个ms也是串行实施就好」+「对于实施当中发现的所有issue候选，都直接寻找好的承载方式流程进行修复而不是等级」。**批准不降低检查标准**：BDD 缺口扫描、实现调查、RTM、设计闭合、Iteration 平衡审计照常完整执行。
- **Gate 2（执行就绪批准）**：**已批准**（同上依据）。
- 本轮只做计划，不实施、不改产品代码。

## 影响面

- 产品代码：`src/storage/btree/btree.rs`、`src/storage/data/table_manager.rs`、`src/storage/data_page.rs`、`src/storage/page_format/slotted_page.rs`、`src/storage/page_format/tuple.rs`、`src/storage/buffer_pool.rs`、`src/storage/free_page_rebuild.rs`（新增故障注入点）、`src/executor/data_scan.rs`、`src/executor/alter_table.rs`、`src/transaction/manager.rs`。
- 测试代码：`tests/` 下新增/扩展缺陷回归用例；`tests/legacy_compat_test.rs` 扩展三条常态化用例；`tests/fixtures/legacy/` 新增 G2 样本与来源说明更新。
- 规格：本 change 的 4 份规格增量（合并由 docs-maintainer 在收尾时执行）；`legacy-format-compat` 的证据索引更新为交接项。
