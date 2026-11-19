## Why

里程碑 MS53（`.agents/docs/tasks.md`）要求复制、分配和行锁争用都有测量。只有被测量证明值得改的那一处进入实现，SQL 结果不变，其余保留原实现。

复制已经由归档变更 `openspec/changes/archive/2026-11-19-ms53-avoidable-copy/` 交付：嵌套循环的组合行改为移入再移回。该变更的设计写明，复制已经改过一处之后，后续变更只测量分配器和行锁，并写明保留原实现。本次是分配器这一处，来源是 I066。行锁是 I050，按里程碑的拆分信号另开变更，不在本次建任务。

I066 要求先看分配热点，再评估 `jemalloc` 或 `mimalloc` 能否以 feature 依赖引入。本次的结论已经确定：测量执行路径上的分配请求，写明保留系统默认分配器。不引入这两个分配器，不改 `Cargo.toml` 的依赖，不改产品代码。SQL 结果、错误文本和 SQLSTATE 保持。

## What Changes

- 新增一次 release 测试。测试在内存库上准备两张小表，对一条非等值连接分别计数 `parse_stage`、`plan_stage`、`execute_stage` 的成功分配请求次数和请求字节数，并打印这六个整数。
- 同一条语句再经 `Database::execute_sql` 执行。行的值和列名与阶段执行的结果相同。
- 同一内存库上执行 `SELECT * FROM missing`。错误文本是 `Table 'missing' not found: table not found: missing`，SQLSTATE 是 `42P01`。
- 六个整数写入执行记录，并写明保留系统默认分配器。数字不与截止值比较，也不用来改换分配器。
- 三段里任一段的次数或字节数为 0，或计划根节点不是 `NestedLoopJoin` 时，测量无效，停止。不重跑，不改产品代码。

计数器只放在这个测试二进制里，分配和释放都转给 `std::alloc::System`。库和 `rtsql` 可执行文件仍然没有 `#[global_allocator]`。

### 场景草图

1. 冷路径的三段计数。前置：`Database::open_in_memory`，已写入 `l(k INT PRIMARY KEY, name TEXT)` 的 `(1, 'a')`、`(2, 'b')` 和 `r(k INT PRIMARY KEY, v INT)` 的 `(1, 10)`、`(3, 30)`。进程未设置 `RTSQL_PROFILING`。这条 `SELECT l.k, l.name, r.v FROM l JOIN r ON l.k > r.k` 还没有经 `execute_sql` 执行。动作：依次调用 `parse_stage`、`plan_stage(..., false)`、`execute_stage(..., false)`，每次调用前后读取计数。结果：计划根节点是 `NestedLoopJoin`；行是 `Int(2)`、`String("b")`、`Int(10)`，列名依次是 `k`、`name`、`v`；打印 `parse_allocs`、`parse_bytes`、`plan_allocs`、`plan_bytes`、`execute_allocs`、`execute_bytes`。失败边界：根节点不是 `NestedLoopJoin`，或任一段次数或字节数为 0。这次结果不能写入保留结论，停止，不重跑。
2. 公共入口的同一行。前置：场景 1 的 `plan_stage` 已经把这条 SELECT 放入计划缓存。动作：`Database::execute_sql` 执行同一条语句。结果：行的值和列名与场景 1 相同。失败边界：缓存命中后的行或列名不同。停止，不改产品代码。
3. 缺失表。前置：同一个内存库。动作：`execute_sql("SELECT * FROM missing")`。结果：消息为 `Table 'missing' not found: table not found: missing`，SQLSTATE 为 `42P01`。失败边界：文本或 SQLSTATE 与这两处不同。停止，不改错误文本。
4. 保留系统分配器。前置：场景 1 的六个整数都已打印，且次数和字节数都大于 0。动作：把打印原文和「保留系统默认分配器」写入执行记录。结果：`src/` 与 `Cargo.toml` 没有差异，依赖里没有 `jemalloc` 或 `mimalloc`。失败边界：要引入另一个分配器才能解释这些数字。停止并回报。数字的大小不构成切换条件。

### 已经选定的行为

- 测量对象是这条连接的解析、计划和执行。`ON l.k > r.k` 使 `is_pure_equi_join_on` 返回 false，计划器构造 `NestedLoopJoin`。嵌套循环是上一变更改过的执行站点，分配请求记在这条路径上。
- 六个整数只描述系统分配器在这三次调用期间接到的成功请求。它们包含调用期间运行时产生的请求，不扣减，也不换算成驻留内存或 CPU 占比。
- 保留系统默认分配器。不设比例门槛，也不把次数大于 0 当作换分配器的条件。次数和字节数为 0 只说明计数窗口没有看见请求。
- 不新增诊断指标，不改 `process_memory_peak_bytes`。不改 `src/transaction/row_lock.rs`。
- 产品对外合同不变，因此不写行为规格增量。`.openspec.yaml` 使用 `skip_specs: true`。

## Capabilities

### New Capabilities

- 无。SQL 结果、错误文本和 SQLSTATE 保持现状。保留系统分配器是本变更的设计结论，写在 `design.md`。

### Modified Capabilities

- 无。

## Impact

- 代码：只新增 `tests/ms53_allocator_measure.rs`。不改 `src/`，不改 `Cargo.toml`。
- 引用：MS53、I066、M19、M21、M22。M19 要求这次测试调用带 `--test-threads=1`。M21 的分层依赖不新增边，计数器不进入库。M22 要求诊断只观察；本次不新增诊断计数。
- 测试：一条 release 命令，带 `--nocapture`，只跑一次。不跑全量。全量读不到这六个打印字段，本变更也不改跨模块公共面。
- 数据：无格式变更，无迁移。

## 回滚

没有产品改动。删掉这条测试即回到没有这次打印的状态。已有库文件不用迁移。
