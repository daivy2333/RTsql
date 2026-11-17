## Why

连接顺序现在按 FROM 书写顺序组装成左深树。计划器没有行数，也没有列的不同值个数，所以换不成更便宜的左深顺序。MS40 已经交付排序合并，并撤回了按行数交换哈希构建侧的接线。这次做的是其余部分：会话内统计、连接代价，以及在不改变结果多重集合的前提下重排。

## What Changes

- 新增 `ANALYZE TABLE <表> [COMPUTE STATISTICS]`。它按当前快照统计基表，把结果留在本次打开的会话里，并交回每列的行数、NULL 个数和不同值个数。
- 成功的统计会清空计划缓存，并把既有的计划版本戳加一。之后的预编译语句按新统计重计划。
- 关闭数据库、DDL 导致版本戳递增，或统计列名与当前表列不一致时，这些数字不再参与计划。INSERT、UPDATE、DELETE 不更新统计。
- 纯内连接等值链在同时满足规格里的条件时，按代价选择左深叶序。最终输出列序仍是书写顺序。哈希连接仍然构建右输入。两表连接不交换左右。
- 没有可用统计、表数超过 8、或书写顺序的代价算溢出时，保持书写顺序。已有统计但比较失败时，`EXPLAIN` 多一行 `JoinOrder written`。比较成功时多一行 `JoinOrder estimated`。
- 不改页格式、目录行和 WAL。不重开构建侧的 10% 时间门。

## 已批准的方案

目标注入视为 Gate 1 的需求与范围批准，也视为下列默认假设的批准。这不是跳过。

统计保存在本次打开的会话里。一次一发的命令可以在同一会话里先统计再查询；关闭后没有统计，计划回到书写顺序。写入目录尾段会碰旧库打开路径，而这次的验收允许没有统计时保持现有选择。

重排只发生在同时满足这些条件的查询上：顶层单个 FROM、至少三张主库基表、每一步都是内连接且 ON 为纯列等值、有 ORDER BY、没有 LIMIT、OFFSET、FETCH 和 LIMIT BY，并且不在子查询、派生表、CTE 定义或视图展开里。这样换叶序不会改变已规定的无 ORDER BY 探测次序，也不会让 LIMIT 在并列键上拿走另一行。

「结果与重排前一致」指同一数据上的行多重集合相同，并且序列满足该语句的 ORDER BY。相等键之间的相对次序不要求与未重排计划相同。现有排序使用 `sort_unstable_by`，连接规格把有 ORDER BY 时的次序交给 ORDER BY。无 ORDER BY 的语句不重排。

代价里探测一行权为 1，构建右输入一行权为 2，输出一行权为 1。2 只用来区分叶序，不是实测耗时。权重都为 1 时，这套基数公式下的合法左深顺序会打成平手，重排不会生效。不采用实测权重，因为那会重开已经撤回的构建侧评估。

被替代的做法还有：在 EXPLAIN 或普通 SELECT 的计划阶段扫表取统计（`EXPLAIN` 不得读取目标语句的行）；无界枚举或贪心（8 张表以内全枚举，超出则保持书写顺序）；把单个连接的左右输入按行数对调（那是已收尾的构建侧选择）。

## 场景草图

失败边界：统计或重排之后，行多重集合与未重排的执行不同；无 ORDER BY、两表连接、外连接或带 LIMIT 的语句换了叶序；`EXPLAIN` 为了显示计划而扫了目标表的行；失败的 `ANALYZE` 仍留下统计或推进了版本戳。

| 场景 | 前置 | 动作 | 可观察结果 |
|---|---|---|---|
| 统计基表 | 表里有可见行 | `ANALYZE TABLE t` | 每列一行，行数、NULL 个数、不同值个数与这些可见行一致 |
| 同义语法 | 同上 | `ANALYZE TABLE t COMPUTE STATISTICS` | 与不带 COMPUTE STATISTICS 的结果相同 |
| 空表 | 基表没有可见行 | `ANALYZE TABLE t` | 每列仍有一行，三个计数都是 0，并且被保存 |
| 不写文件 | 文件库，统计前记下主文件和 WAL 的长度 | 成功统计 | 两个长度不变，随后的查询仍读到原行 |
| 重新打开 | 统计已成功 | 关闭后再打开，执行原来的连接 | 没有 `JoinOrder` 行，叶序回到书写顺序 |
| 只读与内存库 | 只读打开，或 `:memory:` | 统计该基表 | 得到计数。只读会话不因此写文件 |
| 缺表、视图、附加表 | 对象分别是缺表、视图、已附加表 | 统计该名字 | 按规格拒绝，版本戳不变，不留下统计 |
| 其余子句 | 语句带 NOSCAN、PARTITION、FOR COLUMNS 或 CACHE METADATA | 计划该语句 | 点名拒绝，不扫描 |
| 显式事务 | 用户事务已开始 | 在该事务里统计 | 拒绝，不扫描，版本戳不变 |
| 内存上限 | 上限小于要保留的不同值 | 统计含非空值的表 | `53200`，统计不替换，版本戳不变 |
| 计划失效 | 一条查询已进入计划缓存 | 成功统计后再执行同一文本 | 缓存被清空，版本戳加一，预编译执行与直接执行一致 |
| DDL 使统计失效 | 三张表都已统计 | 建索引后再 EXPLAIN 那条可重排查询 | 没有 `JoinOrder` 行，叶序是书写顺序 |
| DML 不失效 | 已统计 | 插入一行后按旧统计重排 | 叶序可以仍按旧数字。结果行包含新行，多重集合与未重排执行相同 |
| 星型重排 | d1、fact、d2 按规格装入并统计 | 带唯一 ORDER BY 的三表内连接 | 第一条扫描行是 fact，有 `JoinOrder estimated`。行与未统计的同语句相同 |
| 并列代价 | 统计足够且书写顺序已经最省 | EXPLAIN | `JoinOrder estimated`，叶序仍是书写顺序 |
| 比较失败 | 链上基表超过 8 张，或书写顺序的乘法溢出 | EXPLAIN 这条本来可重排的语句 | `JoinOrder written`，叶序是书写顺序 |
| 不重排 | 无 ORDER BY、只有两表、含外连接或带 LIMIT | 即使三张表都已统计 | 没有 `JoinOrder` 行，叶序与统计前相同 |

## Capabilities

### New Capabilities

- `query-cost-and-join-order`：会话内基表统计、左深连接代价，以及只换计划选择的重排。

### Modified Capabilities

- `explain-plan-output`：允许在规格要求时于 `EXPLAIN` 首行之后增加一行 `JoinOrder`。未收集统计时，节点行保持原样。EXPLAIN 仍不执行目标语句。
- `sql-parameter-binding`：成功的统计与 DDL 一样推进计划版本戳。DDL 推进版本戳后，已收集的统计不再参与计划。

## Impact

- 计划入口：`src/parser/planner/query.rs` 的 `build_from_clause_with_projection`，以及 `integer_pk_base_scan_join`。语句分派在 `src/parser/planner/mod.rs` 的 `build_plan`。
- 执行入口：`src/pipeline.rs` 的 `plan_stage_impl`、`execute_stage`、`bump_schema_version`。附加表走 `plan_stage_routed` 与 `AttachRoute`。
- 新的纯计算放在 `src/table_stats.rs`。会话状态放在 `Database` 上，与 `schema_version` 一样由克隆共享。项目模型 M21 的字段个数要等收尾时再改，这次不改 `.agents/`。
- 值的相等见 `src/executor/value.rs`。`Float(-0.0) == Float(0.0)`，但两者的 `Hash` 使用不同的 `to_bits()`。不同值个数必须按 `==` 计，不能直接用 `HashSet<Value>`。
- 约束：M19 与 `.agents/runbooks/m19-batched-full-regression.md`。任何 `cargo test` 都带 `--test-threads=1`。全量按等分批次串行，整个变更只预算一次，放在最后一轮。M23 的缺陷轮次发现后再加，不预建。
- 范围来自 `.agents/docs/tasks.md` 的 MS51，承接 I016 的余项。R77 把该项记为先补统计和估算，不重开构建侧评估。不改 `openspec/specs/` 的正文，只在本变更里写增量。

## 回滚

没有目录迁移，也没有文件格式版本变化。放弃本分支即可回到统计前的书写顺序。已经打开的会话把统计留在内存里，关闭后消失。

## Gate 1

PASS。需求、场景和范围来自 MS51 与本次目标。目标注入同时批准了需求范围和上面的默认假设。没有 SKIPPED。没有未批准的裁剪。不重排无 ORDER BY 的语句，是为了不改变 `join-executor-selection` 已经规定的探测次序。那是验收边界之内的做法，不是把结果语义改写进本次变更。
