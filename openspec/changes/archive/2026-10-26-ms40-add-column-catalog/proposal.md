# Proposal: ADD COLUMN 目录面（ISS39、ISS40）

## Why

`.agents/issues/ISS39-add-column-races-inflight-narrowing.md` 与 `.agents/issues/ISS40-add-column-abandoned-during-column-count-flush.md` 仍是 `open`。`.agents/docs/tasks.md` 的 MS40 把这两条收进「ADD COLUMN 目录面」：同表 `ADD COLUMN` 与在途 `DROP COLUMN` 交错时，发布用的活布局不得取自已被加列改过的布局，该窗口内的进程内读保持自愈；`ADD COLUMN` 在列数刷盘期间被放弃时，目录列数回到语句执行前的值，不留下已经上调并落盘的列数。

本次 goal 原文批准本变更的需求与范围（Gate 1）以及计划（Gate 2）。这是批准，不是跳过。

## What Changes

- ISS40：`ADD COLUMN` 在列数刷盘期间被放弃时，补偿必须执行。列数刷盘已经成功时，回退后的列数要落盘，崩溃重开仍看不到新列。刷盘返回错误的失败路径保持现有补偿：只改脏页，补偿本身不刷盘。
- ISS39：正常路径的窄化发布安装的活布局，取自取得窄化令牌时捕获的布局，不取自发布当时可能已被 `ADD COLUMN` 改过的活布局。同表 `ADD COLUMN` 在令牌持有期间等待，不被令牌拒绝；令牌清除后按当时的活布局追加。发布完成后，同一进程内的读取与关闭再打开所见的列集合、列数和行值一致。改写窗口内的读拒绝保持。
- 不改磁盘格式，不新增公共 API。

## 场景草图

缺口扫描：Happy Path（成功加列、成功删列）、Sad Path（加列刷盘失败、删列提交前放弃、同表第二条删列被拒绝）、Edge（刷盘已经成功之后才放弃、发布幂等、跨表）、组合（在途删列与加列交错、放弃与失败补偿的分流）、取消（future 被丢弃；超时到期同样是丢弃该 future）。

已闭合的缺口：交错时「加列被拒绝」与「加列先成功再被发布覆盖掉」都会改变可观察结果。Issue 与 MS40 都没有写这两句。既有规格写了答案：`ADD COLUMN` 不得因窄化令牌被拒绝（`ddl-layout-publication`「窄化令牌不约束 ADD COLUMN」），成功的加列跨重启仍在（`alter-table-column-evolution`），在途删列的读拒绝保持（`ddl-rewrite-consistency`）。因此加列等待，两条都成功，发布不读被加列改过的布局。不另向用户征询。

| 场景 | 前置 | 动作 | 可观察结果 | 失败边界 |
|---|---|---|---|---|
| 刷盘期间放弃加列 | 表有两列且有数据 | 列数刷盘期间丢弃 `ADD COLUMN` 的 future | 处理结束后列数与可见列定义回到语句前；不 `close` 就崩溃再打开，新列仍不可见 | 刷盘返回错误的失败路径不得改成补偿自己刷盘 |
| 成功加列 | 同上 | `ADD COLUMN` 返回成功 | `close` 再打开，新列在，列数增加 | 补偿不得误伤成功路径 |
| 加列刷盘失败 | 同上 | 列数刷盘返回错误 | 语句失败；列定义与列数回到语句前；回退保持脏页；之后一次成功刷盘再打开，新列不可见 | 与上一条放弃路径分流 |
| 在途删列时加列 | 三列表，删列已提交、目录尚未发布 | 同表 `ADD COLUMN` | 不加列期间目录列数与列名保持发布前的值；读该表仍被点名拒绝；加列不因令牌返回拒绝错误；两条都完成后，进程内 `SELECT` 的列、列数、行值与关闭再打开相同 | 同表第二条删列仍立即拒绝，不改成排队 |
| 跨表加列 | 表 `t` 的删列在途 | 对另一张表 `ADD COLUMN` | 该加列完成，不等待 `t` 的发布；`t` 的目录不被这次加列改写 | |
| 删列提交前放弃 | 三列表 | 提交前丢弃删列 future | 目录列数与列定义不变 | 本变更不改这条 |

## Capabilities

### New Capabilities

- 无

### Modified Capabilities

- `ddl-catalog-transactional`：被放弃的 `ADD COLUMN` 在列数刷盘已经成功时，不留下已落盘的列数上调，可见列定义同时回到语句前。
- `ddl-layout-publication`：失败路径的「补偿不刷盘」继续只约束刷盘未成功的路径。同表 `ADD COLUMN` 在窄化令牌持有期间等待而不被拒绝，发布安装的活布局不取自被加列改过的布局，进程内读与重开一致。

## Impact

- 代码：`src/executor/alter_table.rs` 的 `ADD COLUMN` 臂，`src/storage/data/table_manager.rs` 的 `alter_add_column` 与正常路径发布，以及一处默认不生效的列数刷盘暂停点。`update_table_column_count_ddl` 的既有刷盘顺序不被删列路径改写。
- 行为域：上列两个规格。`ddl-rewrite-consistency` 的在途读拒绝、`alter-table-column-evolution` 的成功加列、`statement-cancellation-outcome` 的「提交已完成但调用方未收到结果视为已提交」保持。
- 约束：M19（全量分批，只放在最后一个 Iteration）。不改 M20 的正文。本变更本地规则：实施中新登记的实质缺陷留在 MS40，不新建里程碑；与本变更故障域相同的由后续 Iteration 修，本 Cycle 不扩范围。
- 明确不做：Join / SMJ / Hash 构建侧，ISS36，其他目录面行为，磁盘格式与公共 API。

## 回滚

磁盘格式不变，没有数据迁移。回滚本变更的产品改动后，旧库按现有打开路径读取。失败路径补偿与成功加列的既有用例是回滚后的对照。
