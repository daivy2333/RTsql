# Iteration 003 / Cycle 000-initial: 派生基座投影收口 + 变更收尾批

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T8, T9, T10, T7
- Acceptance gaps: None
- Repair items: None
- Inherited scope: MS26 change 既有路线（本 Iteration 为扩围批）；T8 = Act 001 报告的 Issue 候选修复，**2026-09-30 用户指令**「Issue 候选给出下一轮 iter，在本 change 解决」为范围批准；T9/T10/T7 = Iteration 000/001 Plan Review Minor 与 Act Remaining Issues 的收尾（同批无耦合）；执行序先于 Iteration 002（静默错误结果收口优先于新特性）。
- Excluded scope: WAL 字节水位（Iteration 002，其后执行）；JOIN 组合布局解析扩展；视图/CTE 语义其他面；stats/sample/profile 的 `:memory:` 点名拒绝；锁等待语义。

**Objective**

读面静默错误结果收口：单表派生基座（FROM 子查询 / CTE 引用 / 视图展开，共用 `DerivedScan` 通道）上的纯列清单 SELECT 按外层清单裁剪输出行与表头，未知外层列计划期点名拒绝，`SELECT *`/聚合/别名项恒等面不回归；同批完成收尾——`--readonly`×子命令组合拒绝自动化用例、测试 clippy 告警清零、孤儿 `:memory:.wal` 移除、`--readonly`/`:memory:` 文档面同步。

**Investigation Facts**

- Current Baseline: Iteration 001 收尾工作区（未提交，HEAD 9e0d7e1 之上），全量 1569 tests / 0 failures / 2 ignored（2026-09-30 Act 001 Response M19 分批，用户裁定「测试可信」，Plan Review accepted 采信）。
- Current-State Evidence（本 Cycle Plan 实读定位，文件行号对应当前工作区）:
  - **根因一（解析）**：`src/parser/planner/query.rs:1428-1432`——投影解析的 `base_schema` 仅从 `PhysicalPlan::Scan(node)` 提取；`DerivedScan` 落入 `None => None` 恒等臂，注释明写「Non-Scan bases (DerivedScan, …) carry no base schema to resolve against — keep the identity projection (design: conservative, do not widen the change surface)」——MS10-T01 时代的保守收窄，在 DerivedScan 成为 FROM 子查询（MS09）/CTE（MS25）/视图（MS21）共用通道后成为静默错误结果。
  - **根因二（装配）**：`query.rs:1663-1671`——无 WHERE 装配臂 `match base_plan { Scan → DataScan(携带 projection), _ => base_plan }`：DerivedScan 直通，即使索引解析出来也无处承载（`Scan`/`DataScan`/`Filter`/`Sort`/`IndexScan*` 节点均有 `projection` 字段，`DerivedScanNode` 没有）。
  - **根因三（表头）**：`query.rs:55`——`get_plan_output_columns` 的 DerivedScan 臂 `node.columns.clone()` 不应用投影（Scan/DataScan 臂经 `projected_columns` 应用）——表头返回派生全集，与行一致地多列；该函数同时是 CTAS 列名提取（`build_create_table_as_select`）、聚合 `input_schema`、派生表列注册（`register_table`，嵌套派生）三消费面。
  - **执行器**：`src/executor/derived_scan.rs`——`DerivedScanExecutor` 仅持 `rows: Vec<Vec<Value>>` 逐行透传，无投影概念。
  - **自动生效面（修复后无需额外改动）**：WHERE 路径 `FilterNode { input: DerivedScan, projection: proj_or_empty }`（`query.rs:1560-1567` 等四处 Filter 臂已携带投影，`FilterExecutor` 既有 `with_projection` 真投影）；ORDER BY 路径 `sort_projection` 消费同一 `projection_indices`（`query.rs:2012-2016`，`SortNode.projection` 既有承载）；`resolve_projection_indices`（`query.rs:2803`）为纯函数不改。
  - **构造点**：`DerivedScanNode` 构造仅 3 处（`query.rs:381` FROM 派生表 / `:923` CTE / `:969` 视图），均需补新字段默认值。
  - **Issue 候选证据**（Act 001，文件库实跑，与内存实现无关）：`SELECT n FROM (SELECT id, n FROM t) AS d`、`WITH c AS (SELECT id, n FROM t) SELECT n FROM c`、`SELECT n FROM v`（v 为同定义视图）三者均输出 `{"columns":["id","n"],"rows":[[1,10],[2,20]]}`；基表 `SELECT n FROM t` 正常单列。
  - **T9 面**：组合拒绝已实现（Iteration 000 偏差 6，`src/cli/mod.rs:218-222` `Some(_) if args.readonly` → Usage exit 2），仅缺自动化用例；`tests/readonly_session_test.rs` 现 13 例。
  - **T10 面**：`cargo clippy --tests` 5 条告警——`tests/readonly_session_test.rs:84/211/291`（empty line after outer attribute，`/// --- Rx` 分隔注释后空行）+ `tests/executor_test.rs:838/863`（unused variable `table_manager`，Iteration 000 偏差 3 机械改写残留绑定）；仓库根 0 字节 `:memory:.wal`（2026-09-05 产物，**git 已跟踪**，产生它的 `WalWriter::open(":memory:")` 夹具副作用已由 Iteration 000 消除）。
  - **T7 面**：`README.md` / `README.zh-CN.md` / `rtsql-docs/SKILL.md` 命令面均无 `--readonly` 与 `:memory:` 条目（Iteration 000/001 Remaining Issues）。
- Code and Critical Path: T8 变更面 = `src/executor/plan.rs::DerivedScanNode`（增 `projection: Vec<usize>`）→ 3 构造点补默认 → `query.rs` 投影解析 `base_schema` 增 DerivedScan 臂 + 无 WHERE 装配臂 DerivedScan 分支携带 `proj_or_empty` 重建节点 → `derived_scan.rs::DerivedScanExecutor` 物化行裁剪（空 = 恒等）→ `pipeline.rs::create_executor_from_plan` DerivedScan 臂接线 → `query.rs::get_plan_output_columns` DerivedScan 臂改 `projected_columns`（CTAS/聚合/注册三消费面随之校正）→ `plan_format.rs`/`correlated.rs` 穷尽臂适配（如需）。T9 = `tests/readonly_session_test.rs` +1 例。T10 = 两测试文件纯清理 + `git rm ':memory:.wal'`。T7 = 三处文档命令面。

**Implementation Guidance**

- 执行顺序：T8 → T9 → T10 → T7（T8 为主体；其余为无耦合收尾，任一失败不阻塞其余——MS19/MS25 批处理纪律）。
- T8 裁剪载体镜像 Scan/DataScan 同型：`DerivedScanNode.projection` 语义 = 对物化行按索引裁剪（空 = 恒等）；无 WHERE 臂重建节点（`PhysicalPlan` 为 owned local，解构重建或 `Clone` 后改字段均等价，非实质）。
- T8 的严格解析复用既有 `resolve_projection_indices(&projection_columns, &derived_columns)`——限定名/非限定名的解析行为与 Scan 基座自动同型（同一纯函数消费），无需新解析逻辑。
- T8 的 `get_plan_output_columns` DerivedScan 臂改投影感知后，注意嵌套派生（derived over derived）与 CTE 定义内引用的列注册自动取裁剪后形状——与外层语义一致，无需特判。
- T10 的 clippy 清理为纯删除（空行/改 `_table_manager` 或消费绑定），不得引发任何断言变化；孤儿文件 `git rm` 随收尾批次入库。
- T7 文档以实现行为为准（exit 码/错误面/限制），不写未实现能力；三处风格随既有条目。

**Task Contracts**

### T8: 派生表/CTE/视图基座的外层投影收口

- Requirement/Scenario: join-output-shape-resolution R5（ADDED，本 change delta spec——五场景：裁剪/CTE 视图等价/未知列 42703/组合面/恒等面）
- Depends on: None
- Targets: `src/executor/plan.rs::DerivedScanNode`（增 `projection: Vec<usize>`）；`src/parser/planner/query.rs`（`base_schema` 提取增 DerivedScan 臂、无 WHERE 装配臂 DerivedScan 分支、3 处构造点、`get_plan_output_columns` DerivedScan 臂投影感知）；`src/executor/derived_scan.rs::DerivedScanExecutor`（物化行按节点投影裁剪）；`src/pipeline.rs::create_executor_from_plan` DerivedScan 臂（接线）；`src/executor/plan_format.rs`/`correlated.rs`（穷尽臂适配，如需）
- Current behavior: 单表派生基座纯列清单外层投影静默丢弃——行与表头均为派生全集（三通道同病）；未知外层列静默多列返回；CTAS 以派生全集建表列。
- Required behavior: ① 纯列清单按外层清单裁剪行与表头（`SELECT n FROM (SELECT id, n FROM t) AS d` → 一列 `["n"]`）；② WHERE（Filter 既有投影字段自动生效）与 ORDER BY（Sort 承载路径自动生效）组合同型；③ 外层未知列计划期 `ColumnNotFound`（42703）点名；④ `SELECT *`/通配、聚合路径（输入全形状）、表达式与别名项（既有顶层 Projection 通路，MS25 T10）行为逐字节不变；⑤ 三通道（FROM 子查询/CTE/视图）等价生效；⑥ CTAS 列名取裁剪后表头并物化对应值；⑦ 嵌套派生与派生表注册列序一致。
- Preserve: Scan/DataScan/IndexScan/IndexScanAll 基座行为逐字节不变（`base_schema` 仅加臂）；`resolve_projection_indices`/`projected_columns` 纯函数语义不变；JOIN 组合布局路径（`join_combined_columns`/`resolve_join_projection_indices`）不变；聚合/分组解析不变；`error-reporting` 既有分类不重排（ColumnNotFound 复用既有 42703 通道）；plan cache 键语义不变。
- Forbidden: 不引入新计划节点类型；不改 `DerivedScanExecutor` 的物化语义（仅加裁剪）；不动 JOIN/聚合解析面；不给视图/CTE 通道加特殊分支（三通道共享同一节点与同一修复）。
- Test witness: `tests/derived_scan_alias_test.rs` 扩展先 RED——三通道 × 纯列清单裁剪（行+表头）、WHERE 组合、ORDER BY 组合、未知列 42703、CTAS 列名与物化、`SELECT *` 恒等、`COUNT(*)` 恒等、`n AS x` 别名通路不回归、嵌套派生列序。GREEN + 既有测试如断言旧多列行为按新语义校准（白名单逐条记录 Act Response——BH-1 型校准段）。
- GREEN condition: 新场景全绿 + derived_scan_alias/cte/view/subquery/join_output_resolution/aggregate_output_shape 零回归 + 全量分批 GREEN。
- Verification: `cargo test --test derived_scan_alias_test --test cte_test --test view_test --test subquery_test --test join_output_resolution_test --test aggregate_output_shape_test -- --test-threads=1`（退出码 0）；全量按 M19 分批纪律（目标名字典序、每批 10 个、批内 `--test-threads=1`），各批 0 failed。
- Stop when: 派生基座裁剪与 JOIN/聚合路径出现形状冲突；`get_plan_output_columns` 投影感知后派生表注册或嵌套派生出现列序错位；ORDER BY 装饰-还原路径与裁剪交互出现行错位。

### T9: `--readonly`×生命周期子命令组合拒绝自动化用例

- Requirement/Scenario: readonly-open-mode R1 场景增补（「--readonly 与生命周期子命令组合拒绝」，本 change delta spec 已修订）
- Depends on: None
- Targets: `tests/readonly_session_test.rs`（+1 例）；无产品代码变化
- Current behavior: 组合拒绝（Iteration 000 偏差 6 实现，exit 2 点名）仅有人工 CLI 实跑见证
- Required behavior: `rtsql --readonly schema <db>`（及任一生命周期子命令代称）exit 2 点名拒绝——自动化断言锁定
- Preserve: 既有 13 例逐字节不动
- Forbidden: 不改产品代码
- Test witness: 新用例 RED（断言当前实现应过——若 RED 则实现与偏差 6 描述不符）→ GREEN
- GREEN condition: `cargo test --test readonly_session_test` → 14 passed / 0 failed
- Verification: `cargo test --test readonly_session_test`（退出码 0）
- Stop when: 实现行为与偏差 6 描述不符（返回 Plan）

### T10: 测试告警与工作区孤儿清理

- Requirement/Scenario: 无 spec Requirement——Act 001 Remaining Issues + Iteration 000 Plan Review Minor；孤儿清理依据 Surgical Changes（`:memory:.wal` 产生自旧 executor_test 夹具副作用，该副作用已由 Iteration 000 消除——本次改动产生的孤儿）
- Depends on: None
- Targets: `tests/readonly_session_test.rs:84/211/291`（3 条 empty-line-after-outer-attribute）、`tests/executor_test.rs:838/863`（2 条 unused variable `table_manager` 残留绑定）、仓库根 `:memory:.wal`（`git rm`，随收尾批次提交删除）
- Current behavior: `cargo clippy --tests` 5 条告警；仓库根存在 0 字节跟踪文件 `:memory:.wal`
- Required behavior: `cargo clippy --tests` 0 warning；孤儿文件移除；测试行为零变化（纯清理）
- Preserve: 全部测试断言与产品代码零变化
- Forbidden: 不顺手重构其他测试代码
- Test witness: 清理前后 `cargo test --test readonly_session_test --test executor_test` 计数一致（14 + 既有 executor_test 数）
- GREEN condition: clippy 0 warning + 两测试文件零回归
- Verification: `cargo clippy --tests 2>&1 | tail -3`（0 warning）；`cargo test --test readonly_session_test --test executor_test`（退出码 0）；`ls ':memory:.wal'` → 不存在
- Stop when: 清理引发任何断言变化（不应发生）

### T7: 文档同步（`--readonly` 与 `:memory:` 命令面）

- Requirement/Scenario: 无 spec Requirement——Iteration 000 Plan Review Minor 2 + Act 001 Remaining Issues；2026-09-30 用户指令文档债随本 change 收口
- Depends on: T8（避免文档描述与收口后行为冲突；T9/T10 不影响文档）
- Targets: `README.md`、`README.zh-CN.md`、`rtsql-docs/SKILL.md` 命令面
- Current behavior: 三处文档均无 `--readonly` 与 `:memory:` 条目
- Required behavior: 双语 README 与 rtsql-docs SKILL 命令面包含 `--readonly` flag（共享锁多读单写、25006 写拒绝、WAL 未决 exit 1、锁冲突 exit 4、生命周期子命令组合 exit 2）与 `:memory:` 形态（零文件、进程退出即弃、生命周期子命令与 `--key`/`--readonly` 组合 exit 2）；描述与实现行为一致，风格随既有条目
- Preserve: 既有文档内容除新增条目与必要联动（如命令示例/能力清单）外不改写
- Forbidden: 不改产品代码；不新增 spec Requirement；不写未实现能力（锁等待、内存库持久化等）
- Test witness: 无自动化（文档内容）——人工最短步骤：阅读三处文档新增小节，逐条对照本 change delta spec 的行为描述（所见即判定；执行人一句话记入验证记录）
- GREEN condition: 三处文档条目齐全且与实现一致
- Verification: 人工核对（最短操作与直接可观察结果如上）
- Stop when: 文档过程中发现实现行为与 delta spec 描述不符（返回 Plan）

**Invariants**

- Scan/DataScan/IndexScan 基座与 JOIN/聚合解析面逐字节不变——T8 是派生基座的纯加性收口。
- 三通道（FROM 子查询/CTE/视图）共享同一 `DerivedScan` 节点与同一修复，无通道特判。
- 严格投影解析纪律一致：派生基座未知外层列与基表基座同为计划期 `ColumnNotFound`（42703）。
- exit 码映射与 `error-reporting` 分类表不重排。
- 纯清理任务（T9/T10）不得引入行为变化。

**Non-goals**

- WAL 字节水位（Iteration 002，其后执行）；JOIN 组合布局解析扩展（`resolve_join_projection_indices` 面）；递归 CTE/物化视图；stats/sample/profile 的 `:memory:` 点名拒绝；内存库持久化/加密。

**Acceptance**

- join-output-shape-resolution R5：五场景（裁剪/CTE 视图等价/未知列 42703/组合面含 CTAS/恒等面）全部有测试见证。（T8 测试）
- readonly-open-mode R1 增补场景：`--readonly`×生命周期子命令组合拒绝 exit 2 有自动化见证。（T9 测试）
- 收尾面：`cargo clippy --tests` 0 warning、孤儿 `:memory:.wal` 移除、测试行为零变化。（T10 验证）
- 文档面：三处文档命令面含 `--readonly`/`:memory:` 且与实现一致。（T7 人工核对）

**Verification**

- R5 → `cargo test --test derived_scan_alias_test --test cte_test --test view_test --test subquery_test --test join_output_resolution_test --test aggregate_output_shape_test -- --test-threads=1`（退出码 0）。
- R1 增补 → `cargo test --test readonly_session_test`（14 passed）。
- T10 → `cargo clippy --tests`（0 warning）+ `ls ':memory:.wal'`（不存在）。
- 全量 → M19 分批纪律（目标名字典序、每批 10 个、批内 `--test-threads=1`、批间顺序执行），各批 0 failed；结论注明分批方式与各批计数。
- T7 → 人工核对（最短操作：阅读三处文档新增小节逐条对照 delta spec；一句话记入验证记录）。

**Gate 2 Readiness**

PASS——T8 范围经 2026-09-30 用户指令批准（「Issue 候选给出下一轮 iter，我们在本 change 解决」；delta spec `join-output-shape-resolution` ADDED R5 已随本 Cycle 写入 change specs）；根因三处（解析/装配/表头）与自动生效面（Filter/Sort 承载）均已实读定位（见 Investigation Facts），构造点与执行器消费面穷举（DerivedScanNode 仅 3 构造点）；T9/T10/T7 为已验证事实的机械收尾。无 BLOCKED/WAIVED 项。

**Persisted Evidence**

- Mode: none

（验证结论以 Act Response 命令 + 决定性输出 + 退出码承载；文档核对为人工一句话记录。）

**Risks and Notes**

- 既有测试可能存在断言旧多列行为的用例（ Act 001 已把 memory_db 视图用例改为 `SELECT *` 规避）——按 BH-1 型校准处理，白名单逐条记录 Act Response；若校准面超出「派生基座子集选择」形态（如断言依赖恒等多列的语义场景），返回 Plan。
- `get_plan_output_columns` DerivedScan 臂改投影感知影响三个消费面（CTAS 列名/聚合 input_schema/派生列注册）——均为「输出形状」语义的正向校正；若聚合路径出现列索引错位（input_schema 与行裁剪不同步），按 Stop when 返回 Plan。
- `sort_projection` 的装饰-还原路径（MS25 T7 decorate-sort-undecorate）对派生基座的适配依赖既有 `projection_indices` 消费点——实现时 ORDER BY 组合场景先行验证。
- 孤儿 `:memory:.wal` 为 git 跟踪文件，`git rm` 后随本 change 收尾批次提交；在此之前工作区状态与 Act 001 报告一致。
- 文档面（T7）无自动化判定——人工核对按公共规则 › 验证 的人工步骤纪律执行。

## Act Response

- Status: reported

**Implemented**

- **T8 —— 派生基座外层投影收口**（`join-output-shape-resolution` R5 五场景全覆盖）
  - `src/executor/plan.rs::DerivedScanNode` 增 `projection: Vec<usize>` 字段（语义与 `ScanNode`/`DataScanNode.projection` 同型：按索引裁剪，空 = 恒等）。
  - `src/parser/planner/query.rs` 五处：① `base_schema` 增 `DerivedScan` 臂（`query.rs:1432-1439`，纯列清单按派生列集严格解析，未知列走既有 `ColumnNotFound`/42703 通道）；② 无 WHERE 装配臂增 `DerivedScan` 分支携带 `proj_or_empty`（`:1683-1689`）；③ `get_plan_output_columns` 的 `DerivedScan` 臂改投影感知（`:55`），CTAS 列名 / 聚合 `input_schema` / 派生表列注册三消费面随之校正；④ `is_base_scan_chain` 增 `DerivedScan` 臂（`:2792`）——ORDER BY 形态下裁剪由 Sort 承担（既有承载点，`SortNode.projection` 复用）；⑤ 三构造点（FROM 派生表 `:385`、CTE `:930`、视图 `:977`）补恒等默认值。
  - `src/executor/derived_scan.rs::DerivedScanExecutor::new(rows, projection)` —— 物化行经项目既有 `apply_projection` 助手裁剪（与 `DataScanExecutor`/`FilterExecutor` 同一实现，零新裁剪语义）。
  - `src/pipeline.rs:1598` DerivedScan 构造臂接线传递 `node.projection`。
  - `tests/derived_scan_alias_test.rs` 新增 R5 分组 11 例（三通道裁剪行+表头、WHERE/ORDER BY 组合、未知列 42703 三通道、CTAS 列名与物化、`*`/`COUNT(*)` 恒等、别名通路不回归、嵌套列序重排）。
  - `EXPLAIN` 渲染（`plan_format.rs`）与相关子查询注入（`correlated.rs`）无需适配——`projection` 不改变两者的既有遍历/渲染语义。
- **T9 —— `--readonly` × 子命令组合拒绝自动化见证**：`tests/readonly_session_test.rs` 新增 `cli_readonly_rejected_with_lifecycle_subcommands`（`schema`/`dump`/`delete`/`restore`/`import` 五形态 exit 2 + stderr 点名 flag 归属；`new` 额外见证守卫早于父目录创建）。既有 13 例未改动。
- **T10 —— 测试告警与工作区孤儿清理**：`tests/readonly_session_test.rs` 三处 `///` 分隔注释后的空行（84/211/291）合并入同一文档块；`tests/executor_test.rs:838/863` 两处未用绑定 `table_manager` 删除；仓库根 0 字节跟踪文件 `:memory:.wal` 经 `git rm -- ':(literal):memory:.wal'` 移除（该路径含 `:` 前缀，须 literal pathspec）。
- **T7 —— 文档同步**：`README.md` / `README.zh-CN.md` 在 CLI 命令面新增 `### Read-only sessions` 与 `### In-memory databases` 两小节（共享锁多读单写、WAL 未决 exit 1、25006 写拒绝 exit 3、锁冲突 exit 4、加密 `--key`、子命令组合 exit 2；内存库零文件、退出即弃、拒绝组合 exit 2），并在能力清单增一条运行形态条目；`rtsql-docs/SKILL.md` 在 `## Resolve a database` 增 `:memory:` 运行规则、在 `## Manage files and locks` 增只读共享锁与两类专属拒绝的操作指引。

**Deviations from Plan**

- **D1（T8 测试契约校准）**：计划中「嵌套派生」见证写作 `SELECT n2 FROM (SELECT n AS n2 FROM (…) AS d) AS e`，该形态在本改动前后均报 `Column not found: n2`——根因是别名重路由 `alias_reroutable` 以 `&& !building_subquery` 为前提，子查询内的纯列别名项不进 Projection 通路，折叠后的别名名在基 schema 上查不到（该判定全部位于未被本改动触碰的行：内层基座是 `Scan`，其 `base_schema` 臂改动前后同为 `Some`）。按契约「既有断言旧多列行为按新语义校准」的白名单处理，测试改写为 `SELECT n, v FROM (SELECT v, n FROM (SELECT v, n FROM t) AS d) AS e`——同样见证嵌套派生列序与重排裁剪，且不依赖该既有边界。实现代码零调整。
- **D2（T8 裁剪实现面）**：契约 Investigation Facts 记 ORDER BY 形态「经 `sort_projection` 消费同一 `projection_indices` 自动生效」。实读发现 `sort_projection` 的承载门 `is_base_scan_chain` 只含 Scan/DataScan/IndexScan/IndexScanAll，派生基座落入 `_ => false`，故增补 `DerivedScan` 臂（该函数全仓仅一处消费点，属契约「Code and Critical Path」列明的 `query.rs` 变更面，非实质偏差）。
- **D3（R5 SHALL 带来的拒绝面变化）**：派生表显式别名列清单（`FROM (…) AS d(x, n2)`）与子查询内别名项两类形态，改前静默返回派生全集，改后按 R5「SHALL NOT 静默返回多列」以 `ColumnNotFound` 点名拒绝。二者是 Requirement 明确要求的行为，非范围扩张；已在本 Response 与 Plan Review 关注面记录。
- **D4（T10 清理手法）**：计划给出「空行/`_table_manager`/消费绑定」三选一，实际对 `executor_test.rs` 两处采用纯删除（绑定确无消费方，`Database` 句柄本身仍在作用域内），比改名更贴近「纯清理、零行为变化」。

**Blocker Handoff**

None.

**Blocker Resolution**

None.

**Self-Review**

- Critical/Important：0 项未解决。
- T8 逐项核对契约 Targets：`plan.rs` / `query.rs`（五处 + 三构造点）/ `derived_scan.rs` / `pipeline.rs` 全部处理；`plan_format.rs`、`correlated.rs` 经实读确认无需适配（新字段不参与渲染与参数注入）。未引入新计划节点类型，JOIN/聚合解析面、`resolve_projection_indices`/`projected_columns` 纯函数、`error-reporting` 分类与 plan cache 键均未改动。
- 裁剪实现复用既有 `apply_projection`（`src/executor/mod.rs:113`）而非另写一份——与 `DataScanExecutor`/`FilterExecutor` 同一实现，越界索引行为与既有扫描面一致，不引入新的静默兜底。
- 恒等面复核：`SELECT *`（`projection_indices` 因通配为 `None`）、聚合（`has_aggregates` → `None`）、表达式与别名项（`has_expression_items` → `None`）、ORDER BY（`proj_or_empty` 置空交 Sort）四条路径的 `projection` 均为空，执行器与表头走恒等分支。
- 遗留 Minor 1（观察，非本 Cycle 引入、未修改）：`src/pipeline.rs::extract_column_indices` 的 Scan/DataScan/IndexScan 臂同样不应用节点 `projection`（按全列集建索引映射）。派生基座现沿用该既有约定，两者在「投影后的行宽 vs 全列名映射」上有一致性缺口；本 Cycle 未触碰该面（契约 Forbidden：不动 JOIN 解析面），且既有 arm 行为未变，非本改动引入的回归。
- 遗留 Minor 2（观察）：`DerivedScan` 出现在 `EXPLAIN` 文本中不展示 `projection`（`plan_format.rs` 渲染节点名与列清单）。计划未要求，EXPLAIN 契约未覆盖派生节点投影展示，未扩围。
- T9/T10 复核：仅测试文件与文档改动，产品代码零变化；T10 两文件测试计数与清理前一致（39 + 14），无断言变化。

**Verification Evidence**

1. T8 RED（建立见证）：`cargo test --test derived_scan_alias_test -- --test-threads=1` → `9 passed; 9 failed`（新增 11 例中 9 例 RED：行/表头裁剪、三通道、未知列、WHERE/ORDER BY、CTAS、嵌套；`SELECT *` 与 `COUNT(*)` 恒等例当轮即通过）。失败输出如 `left: [["b",1],["a",2]] right: [[1],[2]]` 与 `Expected Error, got QueryResult`，与 Investigation Facts 记录的 Act 001 实测一致。
2. T8 GREEN：`cargo test --test derived_scan_alias_test -- --test-threads=1` → `test result: ok. 18 passed; 0 failed`，exit 0。
3. Cycle 全量（M19 分批纪律：目标名字典序、每批 10 个、批内 `--test-threads=1`、批间顺序执行；99 个集成测试目标 = 9 批 ×10 + 末批 9）：`cargo test --test <10 targets> -- --test-threads=1` 逐批 → 105 / 214 / 126 / 111 / 76 / 78 / 196 / 127 / 139 / 69 passed，**每批 0 failed、每批 exit 0**，合计 1241 passed / 0 failed。
4. lib 单元测试：`cargo test --lib -- --test-threads=1` → `test result: ok. 340 passed; 0 failed; 0 ignored`，exit 0。全量合计 **1581 passed / 0 failed**（Iteration 001 基线 1569 + 本 Cycle 新增 12 例）。
5. T9 见证：`cargo test --test readonly_session_test -- --test-threads=1` → `14 passed; 0 failed`，exit 0（既有 13 + 新增 1，与契约 GREEN condition 一致）。
6. T10 见证：`cargo clippy --tests` → `empty line after outer attribute` 与 `unused variable` 计数均为 `0`（仅剩环境侧 `~/.cargo/config` 弃用提示，非项目告警）；`cargo test --test readonly_session_test --test executor_test -- --test-threads=1` → `39 passed` + `14 passed`，exit 0；`:memory:.wal` → `absent`（`git status` 显示 `D :memory:.wal`）。
7. OpenSpec 结构校验：`openspec validate 2026-09-30-ms26-embedded-runtime` → `Change '2026-09-30-ms26-embedded-runtime' is valid`，exit 0。
8. T7 人工核对（文档内容，无自动化判定）：逐条阅读三处新增小节并对照 delta spec 与实现行为——README 英文/中文两节的六个要点（共享锁多读单写 / exit 4 不等待 / WAL 未决 exit 1 点名恢复路径 / 25006 exit 3 / ATTACH 只读传递 / 子命令组合 exit 2；内存库零文件、`close()`/`checkpoint()` 无操作、退出即弃、拒绝组合）与 `readonly-open-mode`、`in-memory-database` 两域 Requirement 及本轮 14 + 15 例自动化见证逐条一致；`rtsql-docs/SKILL.md` 两处新增与 README 同源同义，未出现实现不具备的能力（未提锁等待、内存库持久化/加密、跨进程 MVCC 语义）。执行人判定：与实现一致。

**Persisted Evidence**

- None required（Mode: none；结论由上述命令 + 决定性输出 + 退出码承载）。

**Experience Candidates**

None（无本 change 范围外、有证据的实质缺陷；D1 的别名边界与遗留 Minor 1/2 均为既有行为，本次未新增静默错误结果面，且不满足 Issue 准入）。

**Remaining Issues**

- Iteration 002（T5 WAL 字节水位 / T6 env 配置与死接口删除）仍未展开——按 Iteration Map 待 Iteration 003 Review Result 为 `accepted` 后由 Plan 展开。
- `extract_column_indices` 不应用节点 `projection` 的既有观察（遗留 Minor 1）留在本 Cycle 记录内，未落账 Issue。

## Plan Review

- Review Result: accepted

**Findings**

独立检查（强制项：独立阅读实际代码与 diff，核对 Act Response 声明与代码一致；不重跑已采信验证）：

- **T8 核心实现五处全部独立读毕，与契约及 Act 声明一致**：① `plan.rs:655` `DerivedScanNode.projection` 字段（语义注释与 Scan/DataScan 同型）；② `query.rs:55` `get_plan_output_columns` DerivedScan 臂改 `projected_columns`（CTAS/聚合/派生列注册三消费面随之投影感知）；③ `query.rs:1432-1439` `base_schema` 增 DerivedScan 臂（纯列清单按派生列集严格解析，未知列走既有 42703）；④ `query.rs:1683-1689` 无 WHERE 装配臂增 DerivedScan 分支（节点自身携带 `proj_or_empty`，镜像 DataScan swap；`columns` 保持全集、表头经 `projected_columns` 下游派生——与 Scan/DataScan 表头机制同源）；⑤ `query.rs:2789-2796` `is_base_scan_chain` 增 DerivedScan 臂（偏差 D2，Sort 承载门放行）；三构造点（`query.rs:385`/`:930`/`:977`）恒等默认值就位。`derived_scan.rs` 裁剪复用既有 `apply_projection`（空投影零拷贝直通）；`pipeline.rs:1598` 构造臂接线 `node.projection.clone()`。
- **形状一致性推演**（Plan 独立完成）：WHERE 路径 Filter 承载（派生节点投影保持空、Filter 投影裁剪一次）；WHERE+ORDER BY 路径 Sort 承载（`proj_or_empty` 置空 + `is_base_scan_chain` 放行使 `sort_projection` 可达）；聚合路径恒等（`has_aggregates` → `projection_indices = None`）；JOIN 路径不受影响（派生基座投影保持空，组合布局/`join_combined_columns` 未触碰）——各路径恰好裁剪一次，无双重裁剪或漏裁。
- **Forbidden 项核对**：无新计划节点类型；`DerivedScanExecutor` 仅加裁剪参数（物化语义不变）；JOIN/聚合解析面、`resolve_projection_indices`/`projected_columns` 纯函数、`error-reporting` 分类、plan cache 键全部零改动（diff 确认）；`plan_format.rs`/`correlated.rs` 零改动（git status 确认——新字段缺失任何构造点都会编译失败，build 通过即构造点穷尽）。
- **T9 核对**：`readonly_session_test.rs:597` 新用例覆盖 schema/dump/delete/restore/import 五形态 exit 2 + stderr 点名 flag 归属，`new` 另见证守卫早于父目录创建——六子命令全覆盖；既有 13 例未动（文件 26 个函数 = 13 旧 + 1 新 + 12 helper）。
- **T10 核对**：executor_test.rs 两处未用绑定纯删除（diff 确认）；`:memory:.wal` 为 staged 删除（git status `D`），随收尾批次入库；`readonly_session_test.rs` 分隔注释空行清理（Act 报告 clippy 两类告警计数 0）。
- **T7 核对**：三处文档 +60 行（README 18+18、SKILL 24）；抽读 README 新增两节——共享锁多读单写/exit 4 不等待/WAL 未决 exit 1/25006 exit 3/ATTACH 只读传递/子命令组合 exit 2/内存库零文件即弃——逐条与 delta spec 及实现一致，未写未实现能力。
- **偏差分类**：D1（嵌套派生测试改写）= BH-1 型校准，根因分析正确（`alias_reroutable` 以 `&& !building_subquery` 为前提——本 Cycle Plan 在 Iteration 001 审读时已独立确认该条件存在于未触碰代码，别名项在子查询上下文不进 Projection 通路的边界与内存实现无关）；D2 = Plan Investigation Facts 表述不完整（`sort_projection` 承载门未列 `is_base_scan_chain`）——分类为 **Plan 遗漏（表述层）**，增补臂在契约列明的 `query.rs` 变更面内，非实质；D3 = R5 SHALL「SHALL NOT 静默返回多列」的直接要求（派生表显式别名列清单与子查询内别名项由静默全集改为 42703 点名），Requirement 授权的行为变化，非范围扩张；D4 = 纯删除，更贴近零行为变化纪律。
- **验证采信**（2026-09-30，用户对既有轮次裁定「测试可信」、覆盖面未失效）：T8 RED 9 failed → GREEN 18 passed；T9 14 passed；T10 clippy 0 + 两文件计数一致；全量 M19 分批 **1581 passed / 0 failed**（1241 集成 + 340 lib）；`openspec validate` 通过。

非阻塞 Minor finding（均已在 Act Response 记录、不创建修复）：

1. `extract_column_indices` 不应用节点 `projection`（既有观察，非本 Cycle 引入）——派生基座现沿用 Scan/DataScan 同一约定，一致性缺口留档；若未来聚合参数在派生基座上出现索引错位再收口。
2. `EXPLAIN` 文本不展示派生节点 `projection`（计划未要求，契约未覆盖）。

**Acceptance Gaps**

None——join-output-shape-resolution R5 五场景、readonly-open-mode R1 增补场景、T10/T7 收尾面全部有见证对应。

**Evidence**

- 本 Review 新增检查：上述独立 diff 审读（plan/derived_scan/query/pipeline 四文件 T8 变更面全文、形状一致性五路径推演、T9 用例正文、T10 删除 hunks、T7 文档抽读、Forbidden/Preserve 定点核对）。
- 采信 Act Response 验证结论（2026-09-30）：RED→GREEN、M19 分批 1581/0、clippy/fmt/validate clean。

**Follow-up Decision**

既有 Acceptance 全部满足、无阻塞 finding、两条 Minor 均为留档观察 → `accepted`。D2 分类为 Plan 表述遗漏（非实质，Act 补查正确）；D1/D3/D4 均在授权面内。

**Iteration Plan Update**

None——Iteration Map 与平衡审计不变。

**Next Cycle**

None。

**Next Iteration**

`iterations/002-wal-auto-checkpoint/000-initial.md`（WAL 大小水位自动 checkpoint，T5/T6）——已按执行序展开（Map 最后一个 Iteration），Plan Context 状态 `ready`（Gate 2 PASS，含本 Cycle 新识别的 CTAS 语义边界契约点）。
