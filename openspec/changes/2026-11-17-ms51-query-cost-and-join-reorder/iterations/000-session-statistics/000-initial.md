# Iteration 000 / Cycle 000: 会话统计与左深代价

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `specs/query-cost-and-join-order/spec.md` 的前三条需求，以及「左深连接的代价」。`specs/sql-parameter-binding/spec.md` 里成功统计会推进 `schema_version` 的那一句。Gate 1 已通过。
- Excluded scope: T3 的叶序替换、`JoinOrder` 行、T4 的全量。本 Cycle 不调用代价函数做计划，不修改连接执行器。

**Objective**

一次打开的数据库可以对主库基表执行 `ANALYZE TABLE`，交回与可见行一致的计数，并在失败时不替换统计、不推进版本戳。左深代价函数对规格中的输入返回规定的叶序或「不足以重排」。连接计划仍按书写顺序装配。

**Investigation Facts**

- Starting point: 计划基线是 master `97d183c9777716e5d54e5596ea857aab2c7821d1`。本 Cycle 的工作区从该提交建立。没有运行 `cargo test`。`ANALYZE TABLE` 目前没有产品实现，`Statement::Analyze` 会落到计划器的未支持路径。
- Current-State Evidence:
  - 连接装配在 `src/parser/planner/query.rs` 的 `build_from_clause_with_projection`。它只走 `from[0]`，按 `joins` 收成左深树，右子节点是下一张表。`integer_pk_base_scan_join` 决定排序合并。有 `ORDER BY` 时 `build_query` 传入的第三参为真，连接节点携带全部列。
  - 计划分派在 `src/parser/planner/mod.rs` 的 `build_plan`。`Statement::Explain` 的 `analyze` 标志已经拒绝 `EXPLAIN ANALYZE`。本次语句是 sqlparser 的 `Statement::Analyze`，语法是 `ANALYZE TABLE <名>`，可选 `COMPUTE STATISTICS`。`NOSCAN`、`PARTITION`、`FOR COLUMNS`、`CACHE METADATA` 由解析器放进该变体的字段。
  - 执行分派在 `src/pipeline.rs` 的 `execute_stage`。`bump_schema_version` 清空 `plan_cache` 并把 `schema_version` 加一。预编译语句比较这个版本戳。`plan_stage_impl` 只把可缓存的 `Statement::Query` 放入缓存。
  - 附加表入口是 `plan_stage_routed` 与 `AttachRoute::new`。扁平名是小写段用 `.` 连接，例如 `o.t`。
  - 显式事务内的 `CREATE INDEX` 在进入执行器之前返回没有前缀的 `SqlError`，SQLSTATE `0A000`。`ANALYZE` 的事务内拒绝使用同一形态。
  - 查询内存超限的现有文本是 `Execution error: query memory limit exceeded (limit N bytes)`，SQLSTATE `53200`。上限由 `Database::set_query_memory_limit` 设置，0 表示不限制。
  - `Database::execute_sql` 返回 `SqlOutcome::Rows { columns, rows }` 或 `SqlError { message, sqlstate }`。`plan_cache_len` 与 `schema_version` 是公开读口。`begin` 与 `execute_in_tx` 是事务入口。`open_readonly` 与 `open_in_memory` 是另外两个打开入口。
  - `Value` 的 `==` 认为 `Float(-0.0)` 与 `Float(0.0)` 相等，DECIMAL 走 `canonical_decimal`。`Hash` 对浮点使用 `to_bits()`，不能用来数不同值。
- Code and Critical Path: 新的纯计算放在 `src/table_stats.rs`，不引用 `Database`。`Database` 持有克隆共享的统计表。本 Cycle 不把统计快照注入 `PlanBuilder`。`execute_stage` 识别 `PhysicalPlan::Analyze`，扫描、计算、保存，然后清空缓存并增加 `schema_version`。`bump_schema_version` 额外使全部统计对后续计划不可见。

**Implementation Guidance**

先完成 T1 的失败测试并看到失败，再实现统计。T2 在 T1 把 `src/table_stats.rs` 建好之后加入代价函数。两个任务都结束之前，不改 `build_from_clause_with_projection` 的叶序。

扫描使用现有扫描执行器，在当前快照上取出与 `SELECT *` 相同的行。不要从 `execute_sql` 递归进入。不同值用 `==` 累计，不用 `HashSet<Value>`。

保存统计和「统计替换使计划失效」是两步。第一步只替换这一张表，其他表保留。第二步清空计划缓存并把 `schema_version` 加一，不使其他表失效。`bump_schema_version` 是另一步：它在原有行为之外使所有已保存统计失效。成功的 `ANALYZE` 不调用会让统计失效的那一步。

`PhysicalPlan::Analyze` 要补上编译器报出的穷尽匹配。已有通配臂的文件保持原行为。`plan_format` 给这个节点一行确定的文本即可。`EXPLAIN ANALYZE` 仍按原错误拒绝。

**Task Definitions**

### T1: 会话内统计基表

- Requirement/Scenario: 「会话内基表统计」「失败的统计不留下数字」「统计替换使已有计划失效」中属于保存、拒绝和版本戳的场景。EXPLAIN 叶序变化留到 T3。
- Depends on: None
- Targets: `src/table_stats.rs`；`src/database.rs` 的会话字段与 `pub(crate)` 快照读取；`src/parser/planner/mod.rs::build_plan`；`src/pipeline.rs` 的 `plan_stage_impl`、`execute_stage`、`bump_schema_version`；`src/executor/plan.rs` 的 `PhysicalPlan::Analyze`；`src/executor/plan_format.rs` 的对应臂；`tests/table_statistics_test.rs`。
- Current behavior: `ANALYZE TABLE t` 不能交回计数。没有会话统计。`bump_schema_version` 只清缓存并增加版本戳。连接按书写顺序装配。
- Required behavior:
  - `ANALYZE TABLE t` 与 `ANALYZE TABLE t COMPUTE STATISTICS` 返回 `SqlOutcome::Rows`。列名依次是 `column`、`row_count`、`null_count`、`ndv`。行序与 `SELECT *` 的表头相同。`t(id INT, name STRING)` 含 `(1, 'a')` 与 `(2, NULL)` 时，两行的 `row_count` 都是 2；`id` 的 `null_count` 是 0、`ndv` 是 2；`name` 的 `null_count` 是 1、`ndv` 是 1。
  - 空表每一列的三个计数都是 0，并且快照能读到这张表。只读打开和 `:memory:` 可以统计。只读不创建库文件。成功统计前后，主文件长度与 WAL 长度各自不变。
  - 负零与两个 `==` 为真但字节不同的 DECIMAL，在 `summarize` 的单元测试里 `ndv` 为 1。
  - 失败文本和 SQLSTATE 固定为：缺表 `Plan error: Table not found: missing` / `42P01`；视图 `Plan error: ANALYZE requires a base table: v` / `0A000`；附加表 `Plan error: ANALYZE does not support attached tables: o.t` / `0A000`；`NOSCAN` 为 `Plan error: ANALYZE does not support NOSCAN` / `0A000`，`PARTITION`、`FOR COLUMNS`、`CACHE METADATA` 同样把子句名放进这句；显式事务 `ANALYZE is not supported inside a transaction` / `0A000`，且没有 `Plan error:` 或 `Execution error:` 前缀；预算 `Execution error: query memory limit exceeded (limit 1 bytes)` / `53200`；保存前版本戳已变化 `ANALYZE aborted because the schema changed` / `58000`，没有前缀。这些失败不替换该表统计，不增加 `schema_version`。
  - 成功后 `plan_cache_len` 为 0，`schema_version` 比成功前大 1。同一会话里另一张表的统计仍能读到。
  - `bump_schema_version` 之后，箱内快照不再返回保存过的统计。
  - 关闭再打开后快照为空。
  - 统计成功之后，两表 `ORDER BY` 查询的 EXPLAIN 仍没有 `JoinOrder`，第一条 `Scan ` 仍是书写顺序的左表。
- Preserve: 未统计时的既有 EXPLAIN 节点行。`EXPLAIN ANALYZE` 的原拒绝。哈希连接构建右输入。页、目录和 WAL 的格式。
- Forbidden: 在本任务替换连接叶序；把统计写入目录或 WAL；用 `HashSet<Value>` 数不同值；新增对库外公开的统计读取方法；修改 `openspec/specs/` 或 `.agents/`。
- First test: 在 `tests/table_statistics_test.rs` 写 `analyze_counts_visible_rows`。表 `t(id INT, name STRING)` 插入 `(1, 'a')` 与 `(2, NULL)`，执行 `ANALYZE TABLE t`。断言就是上面列出的两个计数行。实现之前运行 `cargo test --offline --test table_statistics_test analyze_counts_visible_rows -- --test-threads=1`，预期是错误而不是这两行。本计划没有运行这条命令，实施时必须先看到这次失败。
- GREEN condition: 下面两条依次执行，退出码都是 0，failed 为 0。箱内测试函数名或模块路径要能被 `table_stats` 滤到。
  1. `cargo test --offline --lib table_stats -- --test-threads=1`
  2. `cargo test --offline --test table_statistics_test -- --test-threads=1`
- Verification: 集成测试覆盖计数、`COMPUTE STATISTICS`、空表结果、文件长度、只读、内存库、缺表、视图、`AttachRoute` 上的 `o.t`、`NOSCAN`、显式事务、1 字节预算、计划缓存与版本戳、两表 EXPLAIN 不变。箱内测试覆盖负零、DECIMAL 规范形、空表被保存、第二张表在另一次成功统计后仍在、`bump_schema_version` 使快照消失、关闭再打开后快照为空、保存前版本戳不符时不写入。预算失败的文本必须是 `Execution error: query memory limit exceeded (limit 1 bytes)`。
- Stop when: 只读扫描使主文件或 WAL 变长，或为了与 `SELECT *` 一致必须改可见性规则。停止并回到计划。不要把写入收进统计的语义。

### T2: 左深代价函数

- Requirement/Scenario: 「左深连接的代价」的全部场景。
- Depends on: T1
- Targets: `src/table_stats.rs` 中 T1 已建立的模块。新增代价函数及其单元测试。不改 `build_from_clause_with_projection`。
- Current behavior: 没有代价函数。计划器不估计连接顺序。
- Required behavior: 函数名是 `choose_left_deep_order`。同一模块提供 `order_cost`，供测试读取某个叶序的代价。输入是叶、等值边、书写限定符序，以及「这两个限定符作为第一步时是否使用排序合并代价」的判断。合法顺序由测试覆盖的判断确认：只有边 `a-b` 与 `b-c` 时，`a, b, c` 合法，`a, c, b` 不合法，选择结果不得是不合法顺序。测试能读到下面的代价和决定。
  - 星型输入：`d1` 与 `d2` 行数 2，`id` 的 ndv 为 2；`fact` 行数 100，列 `d1` 与 `d2` 的 ndv 为 2。边是 `d1.id = fact.d1`、`fact.d2 = d2.id`。第一步不是排序合并。书写顺序 `d1, fact, d2` 的代价是 506。选中 `fact, d1, d2`，代价是 408。
  - `a`、`b`、`c` 行数都是 10。边是 `a-b` 与 `b-c`，ndv 都是 10。只有 `a` 与 `b` 作为第一步时使用排序合并代价。书写顺序 `a, b, c` 的代价是 90。选中 `b, c, a`，代价是 80。
  - 同样三张表、同样两条边，行数都是 10，ndv 都是 10，但第一步都不是排序合并。四个合法顺序的代价都是 80。书写顺序 `a, b, c` 与最小代价相同，决定是已用估计，叶序仍是 `a, b, c`。
  - 9 张连成链的基表返回不足以重排，不返回另一条叶序。
  - 三张表行数都是 18446744073709551615，边的 ndv 为 1。书写顺序溢出，返回不足以重排。
  - 没有边相连的相邻叶不构成合法顺序，不得被选中。
  - 除数是两端 ndv 的较大者，小于 1 时按 1。除法向零截断。权是探测 1、构建 2、输出 1。排序合并步是左 2、右 2、输出 1。中间结果作为左输入时不用排序合并权。
- Preserve: T1 的 SQL 行为。计划器仍不调用本函数。
- Forbidden: 在计划器里复制一套表数上限或溢出判断；把权改成实测时间；在本任务改连接树。
- First test: 在 `src/table_stats.rs` 的测试里写 `star_join_prefers_fact_on_the_left`，断言 506、408 和叶序 `fact, d1, d2`。函数尚不存在时，`cargo test --offline --lib table_stats -- --test-threads=1` 编译失败。先看到这次失败，再实现。
- GREEN condition: `cargo test --offline --lib table_stats -- --test-threads=1` 退出码 0，failed 为 0。T1 的集成测试不需要为了本任务再跑，除非本任务改了 T1 的生产代码。
- Verification: 上面每一条都在这个库测试里断言。星型与排序合并例子同时断言代价整数和叶序。平手例子断言代价 80 和书写叶序。
- Stop when: 需要改变除法、权重或 8 表上限才能得到这些整数。停止并回到计划。

**Invariants**

- 未收集统计时，既有语句的结果和 EXPLAIN 节点行不变。
- `EXPLAIN` 不因为统计函数的存在而读取目标查询的行。
- 成功统计不写主文件和 WAL。
- `table_stats` 不引用 `Database`，避免把会话对象拉进纯计算。
- 任何测试命令都带 `--test-threads=1`。本 Cycle 不跑全量。

**Non-goals**

不输出 `JoinOrder`。不替换叶序。不持久化统计。不交换两表连接的构建侧。不修改 `.agents/` 与 `openspec/specs/`。

**Acceptance**

- T1 的集成测试和箱内测试通过，失败路径的原文和 SQLSTATE 与规格一致，两表 EXPLAIN 仍无 `JoinOrder`。
- T2 的六个输入给出规定的代价或决定。
- `build_from_clause_with_projection` 的叶序逻辑没有被本 Cycle 修改。

**Verification**

T1 先单独跑 `analyze_counts_visible_rows` 并看到失败，实现后再跑 GREEN 的两条命令。T2 先看到编译失败，再跑库测试。两条 GREEN 命令都带 `--test-threads=1`，分开执行。不使用无过滤的 `cargo test`。

**Gate 2 Readiness**

PASS。无 BLOCKED。无 SKIPPED。

**Persisted Evidence**

- Mode: none

定向测试的命令和结果写入 Act Response 即可。本 Cycle 不产生需要单独保存的现场。

**Risks and Notes**

- 只读扫描若使文件变长，按 T1 的停止条件回到计划。这不是留给实施选择的语义。
- 穷尽匹配的文件清单以编译器输出为准。补臂时不改变原变体的行为。
- 内部字段名和未出现在测试断言里的辅助函数名可以由实施确定。SQL 列名、错误原文、代价整数和两个失效入口的差别不能改。
- 001 才会让星型查询的第一条扫描变成 `fact`。本 Cycle 的两表断言是为了防止提前替换。

## Plan Review

- Review Result: pending

**Findings**

尚未实施。

**Follow-up Decision**

等待实施。

**Next Cycle**

None

**Next Iteration**

None
