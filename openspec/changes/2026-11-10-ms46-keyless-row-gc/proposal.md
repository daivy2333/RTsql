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
- 带键链的遍历、谓词和释放不变。已经从索引删掉的带键删除链，仍然不回收。
- 不新增调用 `gc_table` 的生产路径。

无 **BREAKING**。不改磁盘格式，不改 SQL 结果和错误文本，不改 MVCC 可见性。

## Capabilities

### New Capabilities

- `version-chain-gc`：`gc_table` 对版本链的回收合同。包括无键链按既有谓词回收，以及带键链保持现有结果。

### Modified Capabilities

无。`overflow-page-storage` 已经要求：旧版本槽被 `gc_table` 回收时，溢出链一并释放。本变更继续调用同一个删除函数，不改该 Requirement 的文字。`free-page-lifecycle`、`mvcc-tombstone-visibility` 的 Requirement 不改。

## Impact

- 代码：`src/storage/data/table_manager.rs` 的 `TableMeta::gc_table`。
- 测试：`tests/gc_test.rs` 增加无键链与带键删除链的用例。`tests/overflow_page_test.rs` 的 `gc_releases_oversized_version_chain` 保持 `cleaned == 1`，本变更不改它的断言。
- 不改：`delete_tuple_from_data_page`、空闲页生命周期、版本头格式、索引格式、Scan / Insert / Update / Delete 的定位方式、checkpoint、恢复。
- 关联：MS46；I038；M10；M19。全量不在本 change 的 Iteration 里。

## 回滚

无格式迁移。回滚只恢复 `gc_table` 的索引枚举实现。已有库文件不用迁移。回滚之后，无键行的旧版本重新不被 GC 走到；带键行与回滚前相同。

## 需求

下列 1–6 由 MS46 正文直接确定。机制选择不在这里分叉，写在 `design.md`。

1. 键位不能进入主键索引的行，其旧版本链能被 `gc_table` 回收。
2. 回收使用与现有 GC 相同的可见性判断和释放规则。现有判断是：链头留下；`commit_tx_id()` 为 `Some` 且不是链头的版本删除。不引入快照或活跃事务集合来保留更旧的版本。
3. 仍然作为该行当前版本的链头不回收。链上只有这一个版本时，清理计数不增加。
4. 带键行的既有 GC 合同不变。索引仍然指向的链，清理结果与现在相同。索引条目已经移除的带键删除链，继续不被回收。
5. 不提高 GC 速度，不改变 MVCC 可见性，不改索引格式、空闲页生命周期或刷盘策略，不做节点级锁、分配器或 clone 消除。
6. 若回收必须改可见性语义或空闲页合同，停止并退回范围审议，不硬做。

## 不做

- 不在 SQL、checkpoint、恢复或普通 DML 里调用 `gc_table`。它仍然只在被调用时工作。
- 不改 `delete_tuple_from_data_page` 的槽删除、溢出链释放和释放完成标记。
- 不改 `logical_id` 的分配。删除槽不回收 `logical_id`。
- 不把「快照还能看见的旧版本」加进保留条件。那会改变现有 GC，并触发 MS46 的拆分信号。
- 不回收「键还能进索引、只是索引条目已经被 DELETE 拿掉」的链。
- 不新增 GC 的 SQL 语法、公共包装 API 或自动调度。
- 不在本 change 的任何 Iteration 里跑全量，也不亲自跑全量。
- 不新增操作标识、运行标识、内容哈希或其他身份证明机制，不新增判定层。
- 不修改 `openspec/specs/` 的权威正文。规格增量只放在本变更的 delta。
- 不规划 MS47。

## 场景缺口

BDD 扫描覆盖了 Happy Path、Sad Path、Edge、组合路径，以及错误和兼容。没有需要退回给用户的缺口。

已经由里程碑和现有 `gc_table` 闭合、不再单列待决的解释：

- 「仍可见」指现有谓词留下的链头，不是快照可见性。依据是 `gc_table` 只比较 `commit_tx_id()` 和索引上的 row_id，以及 `tests/gc_test.rs` 的 `test_gc_preserves_uncommitted_versions`：未提交的新版本还在时，更旧的已提交版本会被删掉。
- 「键位不可入索引」指主键值 `Value::to_key()` 为 `None`，或主键列声明类型不是 `Int`。不是「当前索引里没有条目」。已删除的带键行属于后者，合同保持不回收。
- 数据页链全扫还是无键链登记结构，是机制选择。只要可见性和释放规则不变，就在 design 里选定，不退回用户。

## 场景草图

| # | 类别 | 前置 | 动作 | 可观察结果 | 失败边界 |
|---|---|---|---|---|---|
| S1 | Happy | 整数主键表。一行经过两次已提交的非键更新后，再已提交地把主键更新为 NULL | 调用 `gc_table` 前，三个旧 row_id 仍可读。然后调用 `gc_table` | 清理计数为 3。三个旧 row_id 为 `SlotNotFound`。`SELECT` 只返回这个 NULL 主键行的最新非键值 | 计数为 0，或最新行消失，或 `SELECT` 出现旧值 |
| S2 | Edge | 整数主键表只有一行，主键为 NULL，没有旧版本 | 调用 `gc_table` | 清理计数为 0。`SELECT` 仍返回该行 | 把唯一版本删掉 |
| S3 | Edge | `STRING PRIMARY KEY` 表有已提交插入，没有旧版本 | 调用 `gc_table` | 清理计数为 0。`SELECT` 仍返回这些行 | 因整表不进主键索引而把活行删掉 |
| S4 | Sad | 已提交的整数主键行上，有一个未提交的、把主键改成 NULL 的更新 | 调用 `gc_table` | 未提交链头仍在，且 `commit_tx_id()` 为 `None`。已提交前驱为 `SlotNotFound` | 未提交链头被删，或已提交前驱被留下 |
| S5 | 兼容 | 整数主键行已提交更新后再已提交删除。索引条目已经不在 | 调用 `gc_table` | 该链贡献的清理计数为 0。删除前记录的 row_id 仍可读 | 把带键删除链上的槽删掉 |
| S6 | 兼容 | 索引仍然指向带键链的链头，链上有已提交旧版本 | 调用 `gc_table` | 与现有三个 `gc_test` 用例以及 `gc_releases_oversized_version_chain` 的断言相同。后者的清理计数仍为 1，溢出页仍按现有「释放未提升前不可分配」规则 | 为了走到无键链而改变这些断言 |
| S7 | 组合 | 同一张整数主键表里，一行保持带键并有一个已提交旧版本，另一行走 S1 的无键链 | 调用一次 `gc_table` | 带键行的最新版本仍由原键找到，它的旧版本被回收。无键链的结果与 S1 相同。计数是两边回收数之和 | 一条链的回收删掉另一条链的链头 |
| S8 | 组合 | 无键链的旧版本槽是溢出行，链头是页内行 | 调用 `gc_table` | 旧版本槽被回收，溢出链按 `delete_tuple_from_data_page` 的现有规则释放。链头仍可读。释放在提升前不可分配 | 只删槽、留下溢出孤儿页，或改动空闲页何时可分配 |
| S9 | 错误 | 数据页链的 `next_page_id` 成环 | 调用 `gc_table` | 返回 `StorageError::Internal`，消息为 `data page chain cycle while garbage collecting table '{name}'`。发现环之后不再删除无键槽 | 无限走页，或改成静默截断 |
| S10 | 错误 | 无键链头的 `next_version` 最终绕回链上已访问的 row_id | 调用 `gc_table` | 返回 `StorageError::Internal`，消息为 `version chain cycle while garbage collecting table '{name}'`。这条环上的槽在报错前不被本路径删除 | 沿 `next_version` 无限走，或把环上的槽删掉再报错 |
