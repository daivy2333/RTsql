# 无键行版本链进入 GC

来源：里程碑 MS46（`.agents/docs/tasks.md`，Status ready），承接 I038。关联约束：M10、M19。本次 goal 同时要求：任何测试调用带 `--test-threads=1`，多个测试目标逐个执行；本 change 的 Iteration 不安排全量，全量留到 MS46 与 MS47 都收尾合回主分支之后，按 `.agents/runbooks/m19-batched-full-regression.md` 分批串行，只跑一次。

Gate 1 为 PASS。批准来自本次 goal，覆盖需求与范围。这不是跳过：下面的缺口扫描已经做完，没有留下会改变可见结果、而里程碑又没有选定的解释。本文件不更新里程碑状态，不改 SNAPSHOT，不改改进台账，不改 `openspec/specs/` 里的权威正文。

## Why

`TableMeta::gc_table` 只从主键索引的 `scan_all` 结果出发，沿 `next_version` 回收旧版本。键位不能进入主键索引的行不在这份结果里，它的旧版本就一直留在数据页上。I038 记录的影响是空间不回收，不是查询结果错误。

MS46 补的是这条链的覆盖面。可见性判断和释放规则保持现有 GC。带键行已经能被索引走到的合同不变。

## What Changes

- `gc_table` 在现有索引遍历之外，能到达无键行的版本链。
- 无键链上，已提交且不是链头的版本被回收；链头留下。回收仍调用 `delete_tuple_from_data_page`。
- 返回的清理计数包含这些新回收的版本。
- 带键链的遍历、谓词和释放不变。已经从索引删掉的带键删除链不回收。
- 不新增调用 `gc_table` 的生产路径。

无 **BREAKING**。不改磁盘格式，不改 MVCC 可见性规则。不调用 `gc_table` 的语句，SQL 结果和错误文本不变。

## Capabilities

### New Capabilities

- `row-version-gc`：`gc_table` 对无键行版本链的回收合同，以及带键行现有结果保持不变。

### Modified Capabilities

无。`overflow-page-storage` 已经要求：旧版本槽被 `gc_table` 回收时，溢出链一并释放。本变更继续调用同一个删除函数，不改该 Requirement 的文字。`free-page-lifecycle`、`mvcc-tombstone-visibility` 的 Requirement 不改。

## Impact

- 代码：`src/storage/data/table_manager.rs` 的 `TableMeta::gc_table`。`src/executor/data_scan.rs` 只改替代判断使用的行身份：删槽之后按槽的 `logical_id`，不再用目录下标。`superseder_suppresses` 的可见性规则不改。
- 测试：`tests/gc_test.rs` 增加无键链、带键删除链、二级索引和损坏页用例。既有三则带键用例和 `tests/overflow_page_test.rs` 的 `gc_releases_oversized_version_chain` 不改断言。
- 不改：`delete_tuple_from_data_page`、空闲页生命周期、版本头格式、索引格式、Scan / Insert / Update / Delete 的定位方式、checkpoint、恢复。
- 关联：MS46；I038；M10；M19。全量不在本 change 的 Iteration 里。

## 回滚

无格式迁移。回滚去掉 `gc_table` 的数据页遍历，并把 `DataScanExecutor::next` 的替代查找恢复为目录下标。已有库文件不用迁移。回滚之后，无键行的旧版本重新不被 GC 走到；带键行与回滚前相同。已经回收的槽不会因为回滚代码而重新出现。

## 需求

下列 1–6 由 MS46 正文直接确定。机制选择写在 `design.md`：数据页链全扫。

1. 键位不能进入主键索引的行，其旧版本链能被 `gc_table` 回收。主键列上的值 `to_key()` 为 `None` 即属此类，包括 NULL 和非整数键位。
2. 回收使用与现有 GC 相同的可见性判断和释放规则。现有判断是：链头留下；`commit_tx_id()` 为 `Some` 且不是链头的版本删除。不引入快照或活跃事务集合来保留更旧的版本。
3. 仍然作为该行当前版本的链头不回收。链上只有这一个版本时，清理计数为 0。未提交的无键链头留下，它身后已提交的旧版本回收。
4. 带键行的既有 GC 合同不变。索引仍然指向的链，清理结果与现在相同。已提交 `DELETE` 之后不在主键索引里的墓碑链不回收。
5. 不提高 GC 速度，不改变 MVCC 可见性，不改索引格式、空闲页生命周期或刷盘策略，不做节点级锁、分配器或 clone 消除。
6. 若回收必须改可见性语义或空闲页合同，停止并退回范围审议，不硬做。

## 不做

- 不在 SQL、checkpoint、恢复或普通 DML 里调用 `gc_table`。
- 不改 `delete_tuple_from_data_page` 的槽删除、溢出链释放和释放完成标记。`gc_table` 不自行提升空闲页。
- 不改 `logical_id` 的分配。
- 不把快照可见性加进保留条件。
- 不回收带键墓碑链。
- 不把第一段 `scan_all` 改成数据页扫描。
- 不为数据页链增加环检测。`build_superseded_map` 今天也没有。
- 不新增 GC 的 SQL 语法或 CLI。
- 不让 `DELETE` 或全表 `UPDATE` 找到无键行。
- 不在本 change 的任何 Iteration 里跑全量，也不亲自跑全量。
- 不新增身份证明机制或判定层。
- 不修改 `openspec/specs/` 的权威正文。
- 不规划 MS47。

## 场景缺口

BDD 扫描覆盖了 Happy Path、Sad Path、Edge、组合路径，以及错误和兼容。没有需要退回给用户的缺口。

已经闭合的解释：

- 「仍可见」指现有谓词留下的链头，不是快照可见性。依据是 `gc_table` 的判定，以及 `test_gc_preserves_uncommitted_versions`：未提交新版本还在时，更旧的已提交版本会被删掉。
- 「键位不可入索引」指主键值 `to_key()` 为 `None`，不是「当前索引里没有条目」。已删除的带键行属于后者。
- 显式非 INTEGER `PRIMARY KEY` 在计划期被拒绝。隐式首列可以是 STRING。这种单版本行与 NULL 键的单版本行一样，链头留下、计数为 0。
- 数据页链全扫还是无键链登记，选定全扫，理由在 `design.md`。
- 损坏页使用现有 `CorruptedDataPage` 文本。数据页成环不在本次加检测。

## 场景草图

| # | 类别 | 前置 | 动作 | 可观察结果 | 失败边界 |
|---|---|---|---|---|---|
| S1 | Happy | `t(id INT, v INT)` 已提交 `(1, 10)`，再已提交 `v = 11`，再已提交 `id = NULL, v = 12` | 调用 `gc_table` | 清理计数为 2。设为 NULL 之前索引指向的槽为 `SlotNotFound`。`SELECT` 只有 `[NULL, 12]` | 计数为 0，或当前行消失 |
| S2 | Edge | 只已提交插入 `(NULL, 1)` | 调用 `gc_table` | 清理计数为 0。`SELECT` 仍为 `[NULL, 1]` | 删掉唯一版本 |
| S3 | Edge | `t(s STRING, n INT)` 只已提交插入 `('a', 1)` | 调用 `gc_table` | 清理计数为 0。`SELECT` 仍为 `['a', 1]` | 因字符串键不在索引里而删掉该行 |
| S4 | Sad | 已提交 `(1, 10)` 后，显式事务里 `UPDATE` 为 `id NULL, v = 20` 且未提交 | 调用 `gc_table`，再在同一事务 `SELECT` | 清理计数为 1。旧槽为 `SlotNotFound`。`SELECT` 只有 `[NULL, 20]` | 未提交链头被删，或旧值仍出现 |
| S5 | 兼容 | 现有三个带键 GC 用例的前置 | 调用 `gc_table` | 断言与现在相同：清理至少 2 / 至少 1 / 至少 6，最新版本仍由当前键读到，未提交头的 `commit_tx_id()` 为 `None` | 为了无键链改掉这些断言 |
| S6 | 兼容 | 已提交插入整数键行后再已提交 `DELETE` | 调用 `gc_table` | 清理计数为 0。删除前的槽仍可读 | 回收带键墓碑链 |
| S7 | 组合 | 同表一条带键链和一条无键链，每条都有两个已提交旧版本 | 调用一次 `gc_table` | 清理计数为 4。带键最新值仍由原键读到。`SELECT` 含带键最新值和无键当前值，不含旧值 | 一条链删掉另一条的链头 |
| S8 | 组合 | 无键链头的非键整数列上有二级索引，条目指向链头 | 调用 `gc_table`，再按该列当前值等值查询 | 清理计数为 2。查询返回链头的当前值 | 链头被删，二级索引指向已删除的槽 |
| S9 | 组合 | 已提交超页行，再已提交地变成 NULL 键的页内短版本 | 调用 `gc_table`，并在释放提升前后按链页数分配 | 清理计数为 1。`SELECT` 读到短字符串。提升前分配不到这些链页，提升后取回同一批 | 只删槽而留下溢出页，或改变提升条件 |
| S10 | 错误 | 已有无键行，库能打开，数据页头 `slot_count` 被写成 700 | 调用 `gc_table` | 错误文本含 `corrupted data page`、该页号和 `slot count 700`。进程不中止 | 当成空表返回成功，或 panic |
| S11 | 错误 | 已有无键行，库能打开，数据页槽 0 的 offset 被写成 `0xFFFF` | 调用 `gc_table` | 错误文本含 `corrupted data page`、该页号和 `slot 0`。进程不中止 | 当成空表返回成功，或 panic |
