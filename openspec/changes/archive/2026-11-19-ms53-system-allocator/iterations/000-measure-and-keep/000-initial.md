# Iteration 000 / Cycle 000: 测量执行路径的分配请求并保留系统分配器

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2
- Inherited scope: `proposal.md` 的四个场景，以及 `design.md` 的 D1 到 D5。结论是保留系统默认分配器。
- Excluded scope: 行锁、节点锁、页 I/O、io_uring、导入攒批、嵌套循环移动实现的再改、诊断新指标、`jemalloc`、`mimalloc`、行为规格增量、全量套件。

**Objective**

一次 release 测试打印解析、计划、执行的分配次数和字节数，并核对同一条连接的行与缺失表错误。执行记录写明保留系统默认分配器。本 Cycle 不改产品代码。

**Investigation Facts**

- Starting point: 分支 `changes/2026-11-19-ms53-system-allocator`，基点 `4ebbdfdf420b0c4eb6c59b5fef7814dc92523461`。没有其他活跃变更。`tests/ms53_allocator_measure.rs` 不存在。仓内没有 `#[global_allocator]`。`Cargo.toml` 没有 `jemalloc` 或 `mimalloc`。在该基点执行 `cargo test --offline --release --test ms53_allocator_measure -- --test-threads=1 --nocapture`，退出码 101，错误为 `no test target named ms53_allocator_measure`。
- Current-State Evidence: `Database::execute_sql` 调用 `pipeline::execute`。缓存未命中时 `execute_staged` 依次调用 `parse_stage`、`plan_stage`、`execute_stage`；命中时只调用 `execute_stage`（`src/pipeline.rs`）。`plan_stage` 不读缓存，SELECT 构建后写入缓存。`is_pure_equi_join_on` 对 `>` 返回 false，计划器构造 `PhysicalPlan::NestedLoopJoin`（`src/parser/planner/ddl_dml.rs`、`src/parser/planner/query.rs`）。`join_reorder_candidate` 要求 `joins.len() >= 2`，这条语句只有一个 JOIN，叶序替换不换计划。缺表消息由 `register_table` 的 `Table '{}' not found: {}` 和 `StorageError::TableNotFound` 的 `table not found: {0}` 拼成，SQLSTATE 为 `42P01`。默认诊断采集三项都关闭。`is_profiling_enabled` 查看环境变量 `RTSQL_PROFILING`。`process_memory_peak_bytes` 读的是 `ru_maxrss`。`benches/pipeline_stages_bench.rs` 只测墙钟。
- Code and Critical Path: 测试先 `Database::open_in_memory`，用 `execute_sql` 建表并插入。被测语句是 `SELECT l.k, l.name, r.v FROM l JOIN r ON l.k > r.k`。计数窗口包住 `parse_stage`、`plan_stage(&db, sql, &stmt, false)`、`execute_stage(&db, plan, false)`。然后 `execute_sql` 再执行这条 SELECT，再执行 `SELECT * FROM missing`。建表和插入不在计数窗口里。

**Task Definitions**

### T1: 打印三段分配请求并核对行与错误

- Requirement/Scenario: `proposal.md` 场景 1、2、3。
- Depends on: None
- Targets: `tests/ms53_allocator_measure.rs` 的 `allocator_stage_counts_and_sql_results`。不修改 `src/` 和 `Cargo.toml`。
- Current behavior: 没有这个测试。系统分配器、嵌套循环路由和缺表错误见 Investigation Facts。
- Required behavior: 测试在 release 下退出码 0。`is_profiling_enabled()` 为 false。计划根节点是 `PhysicalPlan::NestedLoopJoin`。`execute_stage` 与随后的 `execute_sql` 都返回一行，值为 `Value::Int(2)`、`Value::String` 内容 `b`、`Value::Int(10)`，列名依次为 `k`、`name`、`v`。缺表错误的消息是 `Table 'missing' not found: table not found: missing`，`sqlstate` 是 `Some("42P01")`。输出含六个 `name=value` 字段，三段的次数和字节数都大于 0。测试不比较这些数与任何截止值。
- Preserve: `src/`、`Cargo.toml`、嵌套循环的移入再移回、`RowLockTable`、诊断阶段名、错误文本和 SQLSTATE。
- Forbidden: 修改 `src/`；在 `Cargo.toml` 加入 `jemalloc` 或 `mimalloc`；在库或二进制上添加 `#[global_allocator]`；在测试里根据数字切换分配器；计数路径里获取锁、格式化或打印；把建表和插入算进六个字段；调用 `set_diagnostics` 或设置 `RTSQL_PROFILING`。
- First test: 文件尚不存在时，同一条命令的退出码是 101，错误文本含 `no test target named ms53_allocator_measure`。这是计划在基点 `4ebbdfdf420b0c4eb6c59b5fef7814dc92523461` 上的观察。Act 沿用它，写文件之前不再跑。写入测试后再跑同一条命令。
- GREEN condition: 同一条命令退出码 0，failed 为 0，输出含六个字段，行、列名和错误文本符合 Required behavior。
- Verification: 写入测试后只跑这一条命令一次。失败表示计划形状、行、错误文本或计数窗口与设计不一致。次数为 0 是测量无效，不是保留结论的输入。
- Stop when: `RTSQL_PROFILING` 已存在；计划根节点不是 `NestedLoopJoin`；行或列名不符；错误文本或 SQLSTATE 不符；任一段次数或字节数为 0。停止并回到计划。不重跑来换一组数字，不改产品代码。

语句、数据和计数合同：

- 表 `l` 的列是 `k INT PRIMARY KEY, name TEXT`，行是 `(1, 'a')` 和 `(2, 'b')`。表 `r` 的列是 `k INT PRIMARY KEY, v INT`，行是 `(1, 10)` 和 `(3, 30)`。
- 计数器的 `alloc`、`alloc_zeroed`、`realloc`、`dealloc` 都转给 `std::alloc::System`。前三个在非空指针时各计一次。`alloc` 与 `alloc_zeroed` 的字节数是 `layout.size()`，`realloc` 的字节数是 `new_size`。`dealloc` 和空指针不计。
- 每段只有前后各一次原子读，差值是该段的次数和字节数。打印在断言之前，名字与 `proposal.md` 一致。
- `plan_stage` 和 `execute_stage` 的 `profiling` 参数都是 false。

### T2: 记下六个整数并保留系统分配器

- Requirement/Scenario: `proposal.md` 场景 4。
- Depends on: T1
- Targets: 本 Cycle 的 Act Response。不修改其他文件。
- Current behavior: 还没有这次打印，也没有本变更的保留记录。
- Required behavior: 执行记录抄写 T1 输出的六个整数，不改写数字。同一节写明保留系统默认分配器。说明这些数字不与截止值比较。`git diff` 相对本 Cycle 开始时的 HEAD，不含 `src/` 与 `Cargo.toml`。新增的代码文件只有 `tests/ms53_allocator_measure.rs`。
- Preserve: T1 的打印原文。
- Forbidden: 看到数字后增加比例门槛或切换条件；引入 `jemalloc` 或 `mimalloc`；修改 `src/`；把次数为 0 写成保留结论。
- First test: 无新测试。输入是 T1 那一次命令的输出。
- GREEN condition: 执行记录里的六个整数与命令输出一致，并写有保留系统默认分配器。`src/` 与 `Cargo.toml` 的 diff 为空。
- Verification: 对照 T1 的输出核对六个整数。再用 `git diff --name-only HEAD -- src Cargo.toml` 确认没有路径。`HEAD` 是实施开始时的变更分支顶端。
- Stop when: T1 的字段缺失，或任一数字为 0，或 diff 出现 `src/` 或 `Cargo.toml`。停止。不要重跑测试来换一组数字，也不要改产品代码来清掉 diff。

**Invariants**

SQL 结果、错误文本、SQLSTATE、诊断阶段名、磁盘格式、`RowLockTable` 和库的全局分配器在本 Cycle 结束时与起点相同。测试文件可以新增。嵌套循环组合行仍然是移入再移回。

**Acceptance**

- T1 的命令退出码 0，failed 为 0。输出含六个字段。行、列名、错误文本和 SQLSTATE 符合 T1 的 Required behavior。三段次数和字节数都大于 0。
- T2 抄写的整数与该次输出一致，并写明保留系统默认分配器。
- `src/` 与 `Cargo.toml` 相对实施开始时的 HEAD 没有差异。
- 测量无效时 Act Response 为 `blocked`，记录里不出现「保留系统默认分配器」。

**Verification**

`cargo test --offline --release --test ms53_allocator_measure -- --test-threads=1 --nocapture`

退出码 0，failed 为 0。输出含六个字段。这一个测试覆盖场景 1、2、3。场景 4 由 T2 对照这次输出写入执行记录，不另跑命令。数字不与截止值比较。

**Gate 2 Readiness**

PASS。审查项全部通过。

**Risks and Notes**

- 六个整数的具体值要等 T1 运行后才知道。它们不决定接口、错误文本或是否引入分配器。
- 次数包含阶段 `.await` 期间运行时发出的请求。执行记录保持原样，不扣减。
- 验证结果写入 Act Response。不建 `evidence/` 目录。

## Act Response

- Status: reported

保留系统默认分配器。下面六个整数是这一次命令的打印原文，不与截止值比较，也不从中扣减阶段等待期间运行时发出的请求。

```
parse_allocs=244 parse_bytes=22993 plan_allocs=309 plan_bytes=14284 execute_allocs=107 execute_bytes=15330
```

新增 `tests/ms53_allocator_measure.rs`，测试函数是 `allocator_stage_counts_and_sql_results`。`StageAllocator` 实现 `GlobalAlloc`，`alloc`、`alloc_zeroed`、`realloc` 和 `dealloc` 都转给 `std::alloc::System`。前三个只在返回非空指针时各计一次；`alloc` 与 `alloc_zeroed` 的字节数是 `layout.size()`，`realloc` 的字节数是 `new_size`。`#[global_allocator]` 只写在这个测试文件。`src/` 与 `Cargo.toml` 相对实施开始时的 `50e0241e338c3bfa41e58de5ebc3ccd9a5c774a5` 没有差异。

每段的次数和字节数各读一次原子整数，读数放在 `load_counts`。这不改变窗口：建表和插入在窗口之外，打印在断言之前。

`cargo test --offline --release --test ms53_allocator_measure -- --test-threads=1 --nocapture` 退出码 0。结果是 `test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out`。输出含上面六个字段。`is_profiling_enabled()` 为 false。计划根节点是 `NestedLoopJoin`。`execute_stage` 与随后的 `execute_sql` 都返回一行，值依次是 `Int(2)`、内容为 `b` 的 `String`，以及 `Int(10)`。列名依次是 `k`、`name` 和 `v`。缺表错误消息是 `Table 'missing' not found: table not found: missing`，`sqlstate` 是 `Some("42P01")`。

`git diff --name-only HEAD -- src Cargo.toml` 没有输出。当时的 `HEAD` 是 `50e0241e338c3bfa41e58de5ebc3ccd9a5c774a5`。

分配路径不获取锁，不格式化，也不打印。测试不按这六个数字切换分配器。

Experience Candidates: None

未解决问题：无。

## Plan Review

- Review Result: accepted

**Findings**

对照 `tests/ms53_allocator_measure.rs`，以及 merge-base `4ebbdfdf420b0c4eb6c59b5fef7814dc92523461` 到 `c4eda21595e0851a6a019676c7556cbcc5d99e4c` 的 diff。这次 diff 新增变更文档和这一个测试。`src/` 与 `Cargo.toml` 不在其中。从实施起点 `50e0241e338c3bfa41e58de5ebc3ccd9a5c774a5` 到这个提交，`git diff --name-only -- src Cargo.toml` 没有输出。

`StageAllocator` 的 `alloc`、`alloc_zeroed`、`realloc` 和 `dealloc` 都转给 `std::alloc::System`。`alloc` 和 `alloc_zeroed` 在返回非空指针时各计一次，字节数是 `layout.size()`。`realloc` 在返回非空指针时计一次，字节数是 `new_size`。`dealloc` 和空指针不计。这条路径没有锁，没有格式化，也没有打印。`#[global_allocator]` 只写在这个测试文件。`src/` 和 `Cargo.toml` 里没有 `jemalloc`、`mimalloc` 或 `#[global_allocator]`。

计数窗口包住三次调用：`parse_stage`，然后 `plan_stage(&db, SELECT_SQL, stmt, false)`，然后 `execute_stage(&db, plan, false)`。建表和插入在第一次读数之前。每段前后各调用一次 `load_counts`，次数和字节数各读一次。六个差值在 `println!` 之前算完，打印在断言之前。测试不把这些数和截止值比较，也不按它们更换分配器。

`is_profiling_enabled()` 在准备数据之前断言为 false。计划用 `PhysicalPlan::NestedLoopJoin` 匹配根节点。`assert_join_row` 要求列名依次是 `k`、`name`、`v`，行是 `Int(2)`、内容为 `b` 的 `String`，以及 `Int(10)`。`execute_stage` 和随后的 `execute_sql` 都经过这个断言。缺表消息是 `Table 'missing' not found: table not found: missing`，`sqlstate` 是 `Some("42P01")`。六个数都断言大于 0。

`l` 的行是 `(1, 'a')` 和 `(2, 'b')`，`r` 的行是 `(1, 10)` 和 `(3, 30)`。语句是 `SELECT l.k, l.name, r.v FROM l JOIN r ON l.k > r.k`。

执行记录中的打印与 `println!` 相同：

```
parse_allocs=244 parse_bytes=22993 plan_allocs=309 plan_bytes=14284 execute_allocs=107 execute_bytes=15330
```

六个整数都大于 0。同一节写明保留系统默认分配器，并写明不与截止值比较。

验证沿用本 Cycle 的 Act Response，来源提交 `c4eda21595e0851a6a019676c7556cbcc5d99e4c`。命令是 `cargo test --offline --release --test ms53_allocator_measure -- --test-threads=1 --nocapture`。退出码 0，结果是 `1 passed; 0 failed`。该提交之后测试文件没有再改，工作区也是干净的，所以不重跑。

`src/executor/nested_loop_join.rs` 的组合求值仍是取出左行和右行、`append` 之后按左列数 `split_off` 放回。该文件不在本次 diff 里。

非阻塞：`design.md` 的 D1 写「一行一个」。测试把六个字段打在同一行。执行依据只要求输出含这六个 `name=value` 字段。执行记录抄的是这一行。不改打印，也不重跑。

**Evidence**

- 测试：`tests/ms53_allocator_measure.rs` 的 `allocator_stage_counts_and_sql_results`。
- 打印与退出码：本 Cycle 的 Act Response，提交 `c4eda21595e0851a6a019676c7556cbcc5d99e4c`。
- diff：`git diff --name-status 4ebbdfdf420b0c4eb6c59b5fef7814dc92523461..c4eda21595e0851a6a019676c7556cbcc5d99e4c`。`src/` 与 `Cargo.toml` 相对 `50e0241e338c3bfa41e58de5ebc3ccd9a5c774a5` 为空。
- 产品分配器：`src/` 和 `Cargo.toml` 没有 `global_allocator`、`jemalloc` 或 `mimalloc`。
- 嵌套循环：`src/executor/nested_loop_join.rs` 的 `next` 仍是移入再移回。该文件不在 diff 里。
- Persisted Evidence 为 none。没有要求 `evidence/` 目录。

**Follow-up Decision**

T1 和 T2 的验收都满足。上面一项不挡住验收，不留在当前 Cycle 修，也不另开返工。任务表没有未展开的后续 Iteration。

**Next Cycle**

None

**Next Iteration**

None
