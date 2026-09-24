# Iteration 000 / Cycle 000: surface-defects 初始执行

## Plan Context

- Status: ready
- Iteration: 000-surface-defects
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T1, T2, T3, T4
- Depends on: None
- Stable baseline: 全量假失败源消除（I041）；IN×JOIN 拒绝文案与事实相符（ISS02）；标量子查询表头与行形状一致（ISS03）；含引号字符表名 import 可达（I048）；既有相关套件零回归。
- Verification boundary: 四任务新增测试全绿 + 既有 subquery/cli 投影/import 套件零修改 + Iteration 末全量 `cargo test --no-fail-fast` 一次绿。
- Diagnostic boundary: `src/cli/resolve.rs`、`src/parser/error.rs`、`src/parser/planner/subquery.rs`、`src/parser/planner/query.rs`、`src/cli/lifecycle.rs`、`tests/{subquery_test,cli_test}.rs`。
- Deferred tasks: T5, T6（Iteration 001）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 全部五项缺陷清账承诺中本 Iteration 的 T1-T4；design D2/D3/D4/D5。
- Excluded scope: T5/T6（可见性摘要与收尾）；IN×JOIN 能力解锁；多列 IN 首列静默语义裁决；执行器行产出改动；ISS 台账落账。

**Objective**

四个用户可见面/验收门缺陷按 Task Contract 修复并有 RED→GREEN 测试见证；既有相关套件零回归；全量测试一次通过。

**Background**

MS17-T02 缺陷清账（proposal Why 节）第一棒 Iteration。I041 先行以稳定后续全量门（MS17 验证边界要求）；T2-T4 为 planner/CLI 表面小缺陷批（MS15-Rest 批处理先例）。来源台账：`.claude/issues/ISS02-in-subquery-join-plan-rejection.md`、`.claude/issues/ISS03-scalar-subquery-header-shape-mismatch.md`、improvements I041/I048。

**Investigation Facts**

- Current Baseline: master `7364bc9` + MS13 实施 + 收尾 docs sync（未提交工作区）；1050 tests pass / 0 failed / 2 ignored（2026-09-23 MS13 收口，本会话未修改覆盖范围内代码表面，采信）；clippy/fmt/validate 全 0/PASS。全部结论为本会话（2026-09-23）新鲜读码与二进制探针取证。
- Current-State Evidence:
  - **T1**：`src/cli/resolve.rs:80-127` 两个 `#[test]`（`test_bare_name_env_cases`/`test_db_dir_env_cases`）各自持独立 `EnvGuard` 改写进程全局 `HOME`/`RTSQL_HOME`；cargo test 默认并行 → guard drop 恢复窗口撞对方断言窗口。文件头 doc 注释（:41-42）已宣称「涉及 env 的用例集中在单个 #[test] 内顺序执行」——实现与注释不一致。全仓 grep 确认无其他 env 改写测试（`set_var`/`remove_var` 仅此文件）。
  - **T2**：`subquery.rs:386-440` `get_subquery_first_column` 臂覆盖 Scan/DataScan/Filter/Aggregate/SemiJoin/AntiJoin，`Join`/`NestedLoopJoin` 落 `:438` `_ => Err(PlanError::SubqueryReturnsMultipleColumns)`；`error.rs:99-102` Display 文案 `Subquery returns multiple columns (IN subquery requires single column)`。本会话二进制探针：单列 JOIN 子查询 exit 3 误报多列；多列+JOIN 同文案；`WHERE+JOIN` 子查询报既有 `Unsupported statement type`（相符，不动）；`ORDER BY r.a` 子查询报更早的 `ORDER BY only supports column names`（相符，不动）。调用方唯一：`subquery.rs:49`（IN 子查询 SemiJoin/AntiJoin 装配的 right_column 元数据；非首列 IN 子查询行值经子查询计划自身投影，探针 `IN (SELECT r.b FROM r)` 结果正确——改臂不触碰值比较）。校准面：`tests/nested_loop_join_test.rs:267` 仅注释引用旧文案（无断言锁定，注释描述的不可达性在修后仍成立）；`tests/subquery_test.rs` 无文案断言。
  - **T3**：`query.rs:114` `PhysicalPlan::SubqueryEval(node) => self.get_plan_output_columns(&node.input)` 未计入插入列；`SubqueryEvalNode`（`executor/plan.rs:436-447`）`output_column`/`result_column_index` 均 `pub`；执行器插入点 `subquery_eval.rs:187-192`（`if idx <= row.len() { insert } else { push }`）；包装点 `query.rs:1198-1212`（右到左，result_column_index = proj_idx − 前置标量数）。表头消费方：CLI `run_sql`（`cli/mod.rs:363`）唯一；lib `Response::Rows` 无列元数据。`subquery_test.rs` 只断言行值（20+ 用例零校准）；cli_test 无标量子查询用例。台账实证形状：`{"columns":["id","name","dept","salary"],"rows":[[1,"East","Alice",10,50000],...]}`（4 列头对 5 值行）。
  - **T4**：`lifecycle.rs:496` `format!("INSERT INTO {} VALUES ({});", table, values.join(", "))` 实参原文插值；实参匹配在 `:424` 附近 `db.get_table(&table)` 逐字比对（不动）。`quote_ident`（`:594-596`）= 包裹 + 内部引号加倍。`object_name_to_table_name`（`ast.rs:239-245`）= `id.value.to_lowercase()` join，**不去引号**（sqlparser 解析时已解转义）。可达链：`CREATE TABLE "a""b"(i INT)` → catalog 名 `a"b`（现行可达）→ import 实参 `a"b` → `get_table` 命中 → INSERT 原文 `INSERT INTO a"b …` SQL 解析报错（缺陷形态）；quote_ident 后 `"a""b"` → 解析 value `a"b` → 归一化相等 ✓。裸名 `items` → `"items"` → 解析 `items` = 现状等价 ✓。
  - 测试基建镜像：lib 级 SQL 错误断言参考 `tests/subquery_test.rs` 既有形态；cli e2e 参考 `tests/cli_test.rs` 既有 import/投影用例结构。
- Code and Critical Path:
  - T2 变更面：`src/parser/error.rs`（enum 变体 + Display 臂）、`src/parser/planner/subquery.rs`（match 臂）。
  - T3 变更面：`src/parser/planner/query.rs:114` 单臂。
  - T4 变更面：`src/cli/lifecycle.rs:496` 单行。
  - T1 变更面：`src/cli/resolve.rs` tests 模块（无产品代码）。
  - 四个变更面互不重叠；无共享状态、无并发边界、无数据迁移。

**Implementation Guidance**

建议顺序 T1 → T2 → T3 → T4（T1 最小且独立；T2/T3/T4 各自独立可并行理解，但按编号顺序提交验证面最清晰）。T2 的新变体插入位置建议紧邻 `SubqueryReturnsMultipleColumns`（error.rs:47）保持子查询错误分组；match 臂写在 `_` fallback 之前与 SemiJoin/AntiJoin 臂同风格。T3 注意保持 `columns` 变量可变性与既有臂风格一致；`min` 守卫必须有（执行器 push 兜底分支的镜像）。T4 注意 import 的 `table` 变量在闭包内为 `String`（`lifecycle.rs:415` `let table = table.to_string()`），`quote_ident(&table)` 借用即可。测试先行：每任务先写 RED 用例观察失败，再实施。

**Behavioral Change**

- T1：无产品行为变化（测试结构重构）。
- T2：`IN (SELECT … JOIN …)` 的计划期错误文案由 `Subquery returns multiple columns (IN subquery requires single column)` 变为 `IN subquery with JOIN is not supported`（Join/NestedLoopJoin 形态）；其余 IN 形态错误面逐字节不变。
- T3：含标量子查询项的查询，CLI 表头（四格式）在标量位置新增列名；lib 行值不变。
- T4：import 对含引号字符表名从 SQL 解析报错变为可达；裸名 import 行为逐字节不变。

**Task Contracts**

### T1: resolve env 测试合并为单测试顺序执行

- Requirement/Scenario: 测试基建（无行为 requirement；RTM 行「I041」）
- Depends on: None
- Targets: `src/cli/resolve.rs` tests 模块
- Current behavior: 两个 `#[test]` 并行运行，各自 EnvGuard 改写进程全局 `HOME`/`RTSQL_HOME`，存在 drop 恢复窗口竞态（全量约 1/6 假失败）
- Required behavior: 单一 `#[test]`（建议名 `test_env_resolution_cases`）顺序执行两组断言（逐条保留）；doc 注释与实现一致
- Required changes: 合并测试函数；删除旧两个测试函数；Guard 结构原样
- Preserve: 两组断言的断言内容逐条保留（不改期望值）；`resolve_db_path`/`rtsql_home`/`db_dir` 产品代码零改动
- Forbidden: 修改 resolve.rs 非测试代码；引入新依赖或测试工具
- Test witness: 变更前 `cargo test --lib resolve` 绿（现状）；变更后同一命令绿且测试函数数 -1（3→2）
- GREEN condition: `cargo test --lib resolve` 全绿
- Verification: `cargo test --lib resolve`（决定性输出 ≤5 行，退出码 0）
- Stop when: 合并后断言出现非预期失败（说明既有断言间存在隐藏顺序依赖——返回 Plan）

### T2: IN 子查询 JOIN 形态诚实拒绝

- Requirement/Scenario: in-subquery-join-rejection R1 S1/S2/S3 + R2 S1/S2
- Depends on: None
- Targets: `src/parser/error.rs`（`PlanError` enum + Display）、`src/parser/planner/subquery.rs::get_subquery_first_column`
- Current behavior: 单列 `IN (SELECT … JOIN …)` 报 `Plan error: Subquery returns multiple columns (IN subquery requires single column)`（exit 3，文案与事实不符）
- Required behavior: Join/NestedLoopJoin 形态报 `Plan error: IN subquery with JOIN is not supported`（exit 3）；WHERE+JOIN 维持 `Unsupported statement type`；`_` fallback 与既有形态臂逐字节不变
- Required changes: 新增 `PlanError::InSubqueryJoinUnsupported` 变体 + Display 臂；`get_subquery_first_column` 补 `PhysicalPlan::Join(_) | PhysicalPlan::NestedLoopJoin(_)` 拒绝臂
- Preserve: 既有六个形态臂与 `_` fallback 逐字节；非 JOIN 单列 IN 可达性与结果集；`subquery.rs:49` 调用方语义（right_column 元数据）
- Forbidden: 修改 `extract_correlated_params`/`collect_outer_column_refs`（ON 关联注册属 improvement 候选）；修改多列无 JOIN 形态行为；修改任何执行器
- Test witness: `tests/subquery_test.rs` 追加 4 用例（单列 JOIN 文案 / 多列+JOIN 文案 / WHERE+JOIN 既有文案 / 非 JOIN 单列 IN 可达）；RED = 前两用例断言新文案失败（现状报多列误报）
- GREEN condition: 4 用例全绿 + `cargo test --test subquery_test --test nested_loop_join_test` 既有用例零修改全绿
- Verification: `cargo test --test subquery_test`（决定性输出 ≤10 行，退出码 0）
- Stop when: 新文案在非 JOIN 形态出现（臂匹配过宽），或既有用例出现非预期失败

### T3: 标量子查询表头在标量位置携带列名

- Requirement/Scenario: cli-noninteractive-shell 新增 Requirement S1/S2/S3
- Depends on: None
- Targets: `src/parser/planner/query.rs::get_plan_output_columns` SubqueryEval 臂（:114）
- Current behavior: 表头 = 输入计划列名（N 列），行 = N+1 值（标量在 `result_column_index`），json `columns` 数 ≠ `rows` 行宽，标量列名丢失
- Required behavior: 表头在 `min(result_column_index, len)` 位置插入 `node.output_column`，`columns` 数 == 行宽；四格式一致受益
- Required changes: SubqueryEval 臂改为取输入列名向量后按 `min(result_column_index, columns.len())` `insert` `node.output_column`
- Preserve: 执行器行产出（`subquery_eval.rs`）零改动；`SubqueryEvalNode` 构造点（`query.rs:1198-1212`）零改动；Projection 包装臂、I034 既有投影表头行为逐字节；关联与非关联标量子查询行值不变
- Forbidden: 修改执行器插入语义；修改其他 `get_plan_output_columns` 臂
- Test witness: `tests/cli_test.rs` 追加 2 用例——(a) json：建 `emp/dept` 关联数据，`SELECT id, (SELECT region FROM dept WHERE dept.rid = emp.id) AS region FROM emp`，断言 `columns == ["id","region"]` 且 `columns.len() == rows[0].len()`（RED：现状 columns 4 元素或无 region）；(b) table：标量位于末列，断言表头三列（RED：现状两列）
- GREEN condition: 2 用例全绿
- Verification: `cargo test --test cli_test`（决定性输出 ≤10 行，退出码 0）
- Stop when: 表头插列导致任何既有表头用例失败且无法归因为本缺陷校准（返回 Plan）

### T4: import 表名实参 quote_ident 转义可达

- Requirement/Scenario: table-name-resolution 新增 Requirement S1/S2
- Depends on: None
- Targets: `src/cli/lifecycle.rs::import_csv` 的 INSERT 构造（:496）
- Current behavior: 表名实参原文插值 `INSERT INTO {table} …`；含引号字符的 catalog 表名（`CREATE TABLE "a""b"` → `a"b`）在 SQL 解析面报错，import 不可达
- Required behavior: 表名经 `quote_ident` 包裹；转义名 import 落库成功、行可回读；裸名行为逐字节不变
- Required changes: `:496` 改 `format!("INSERT INTO {} VALUES ({});", quote_ident(&table), values.join(", "))`；错误信息中表名展示保持实参原文（诊断可读性）
- Preserve: `get_table` 实参比对、CSV 表头双向匹配、逐条 auto-commit、affected 输出语义零改动；`quote_ident` 本体零改动
- Forbidden: 修改实参匹配语义（逐字比对）；修改 dump/schema/分析命令面
- Test witness: `tests/cli_test.rs` 追加用例——`CREATE TABLE "a""b"(i INT)` → 写 CSV（表头 `i`）→ `import … 'a"b' … --csv` 断言 affected 行数与 `SELECT * FROM "a""b"` 回读（RED：现状 exit 3 SQL 解析错误）；既有裸名 import 用例零修改通过
- GREEN condition: 新用例全绿
- Verification: `cargo test --test cli_test`（决定性输出 ≤10 行，退出码 0）
- Stop when: 转义名在 `get_table` 比对面出现非预期失配（说明实参约定理解有误——返回 Plan）

**Invariants**

- RR 默认路径（无快照）行为逐字节不变；执行器行产出与 MVCC 语义零触碰（本 Iteration 不进入 storage/）。
- 既有公共 API、plan cache 键、退出码分类不变。
- 既有测试套件零修改通过（新增用例除外；无校准项——调查确认无断言锁定被改行为）。

**Non-goals**

- T5/T6；IN×JOIN 能力解锁；多列 IN 首列静默语义；ON 关联参数注册；ISS 台账落账；性能 bench。

**Acceptance**

- T1：`cargo test --lib resolve` 绿，测试函数 3→2（requirement：无行为域，RTM「I041」行）。
- T2：in-subquery-join-rejection R1 S1/S2/S3 + R2 S1/S2 场景全部有测试见证；`subquery_test`/`nested_loop_join_test` 零修改。
- T3：cli-noninteractive-shell 新增 Requirement S1/S2 场景测试见证；S3 零回归经既有 cli_test 全绿证明。
- T4：table-name-resolution 新增 Requirement S1/S2 场景测试见证。
- Iteration 级：全量 `cargo test --no-fail-fast` 一次绿（基线 1050 + 新增），clippy/fmt 0。

**Verification**

- T1：`cargo test --lib resolve` → 全绿，exit 0。
- T2：`cargo test --test subquery_test` → 新 4 用例 + 既有全绿，exit 0；RED 记录（新文案断言失败于现状行为）写入 Act Response。
- T3：`cargo test --test cli_test` → 新 2 用例 + 既有全绿，exit 0；RED 记录同上。
- T4：`cargo test --test cli_test` → 新用例 + 既有 import 组全绿，exit 0；RED 记录同上。
- Iteration 末：`cargo test --no-fail-fast` 一次全绿（决定性输出 ≤20 行）；每项判定以被测命令退出码与原生输出为准，无判定层。

**Gate 2 Readiness**

- 无 Missing requirement：RTM 7 行全部 Covered（PASS；`openspec/changes/2026-09-23-ms17-t02-defect-closeout/tasks.md` RTM）。
- 无 Simplified requirement（PASS；proposal Out of Scope 均为非本 change 范围且已登记候选，非需求裁剪）。
- 调查完整：五个变更面（T1-T4）入口/符号/调用者/校准面均有本会话新鲜读码与二进制探针证据（PASS；Investigation Facts + design.md Current-State Evidence）。
- 设计闭合：四个任务的行为差异、错误语义、变更面、Preserve/Forbidden 明确（PASS；design D2/D3/D4/D5）。
- 任务可执行：每任务有代码位置、行为变化、RED 见证与停止条件（PASS；Task Contracts）。
- 分轮合理：Iteration Plan 两轮，依赖有序，000 四任务同面批处理有先例与审计记录（PASS；tasks.md Iteration Plan + design D6）。
- 追踪完整：requirement→scenario→design→task→代码→测试链路齐备（PASS；RTM）。
- 验证充分：覆盖全部已批准 scenario（含 sad path：T2 拒绝文案、T4 RED 错误形态；edge：T3 末列/越界 min 守卫），每条最简直接判定（PASS；Cycle Verification 节）。
- 无身份型证据工程/判定层（PASS；D7）。
- 无实质未知项需 Act 决定契约语义；非实质未知项（T4 历史完整带引号名的文件级 fixture 不可达——以转义名形态作 e2e 见证，代码路径同覆盖）记录于 design D4 与 Risks（PASS）。
- OpenSpec tasks/specs/design/当前 Iteration/Cycle 一致（PASS；本文件与 tasks.md RTM 同源撰写）。
- Persisted Evidence 明确：两 Iteration 均 `none`（PASS；白名单核对——各验证命令输出 ≤20 行可入 Act Response，无不可低成本重跑项）。

**Persisted Evidence**

- Mode: none

（Act Response 承载全部验证输出；无不可重跑项，无 Evidence 预算消耗。）

- Budget: 按公共规则 Evidence 预算执行（0 文件）。

**Risks and Notes**

- T3 的 json 断言形态以台账实证形状（4 列头对 5 值行）为 RED 参照；Act 以实际探针输出为准校准断言细节（列名集合以实际 schema 为准），不放宽「columns 数 == 行宽」与「别名出现」的断言语义。
- T4 的 `'a"b'` 实参在 shell 层的引号转义由测试内 `Command` 参数传递承载（非 shell 字符串），cli_test 既有模式已如此；若测试基建走 shell 字符串需注意 POSIX 引号规则（非实质，Act 局部处理）。
- T2 的多列+JOIN 场景：现状文案碰巧相符（真多列），修后统一为 JOIN 文案——属行为改进非回归，spec S2 已锁定。
- 全量门在 T1 后稳定；若 Iteration 000 期间仍观察到 env 竞态假失败（不可能——单测试结构性消除），记录现象返回 Plan。

## Act Response

- Status: reported

**Implemented**

T1-T4 全部按 Task Contract 完成，RED→GREEN 见证齐全：

- T1（I041）：`src/cli/resolve.rs` `test_bare_name_env_cases` + `test_db_dir_env_cases` 合并为单 `test_env_resolution_cases`（断言逐条保留、`EnvGuard` 原样、产品代码零改动）；文件头 doc 注释与实现恢复一致。
- T2（ISS02）：`PlanError::InSubqueryJoinUnsupported`（文案 `IN subquery with JOIN is not supported`）+ `get_subquery_first_column` 补 `Join | NestedLoopJoin` 拒绝臂（位于 `_` fallback 前、与 SemiJoin/AntiJoin 臂同风格）；既有六形态臂与 `_` fallback 逐字节未动。`tests/subquery_test.rs` 追加 4 用例（R1-S1/S2/S3 + R2-S1）与共享夹具 `setup_o_r_s`。
- T3（ISS03）：`get_plan_output_columns` SubqueryEval 臂改为取输入列名向量后 `insert(min(result_column_index, len), node.output_column)`（镜像执行器 `row.insert`/`row.push` 双分支）；执行器、`SubqueryEvalNode` 构造点、其他臂零改动。`tests/cli_test.rs` 追加 2 用例（json + table）与夹具 `seed_emp_region`。
- T4（I048）：`import_csv` INSERT 构造表名经既有 `quote_ident` 包裹；`get_table` 实参比对、CSV 表头匹配、错误信息实参原文展示零改动。`tests/cli_test.rs` 追加 1 用例（转义名 import 落库 + 回读）。

**Changed Files and Symbols**

- `src/cli/resolve.rs` — tests 模块：`test_env_resolution_cases`（新，合并二者），`EnvGuard` 原样（无产品代码变化）。
- `src/parser/error.rs` — `PlanError::InSubqueryJoinUnsupported` 变体 + Display 臂（紧邻 `SubqueryReturnsMultipleColumns`）。
- `src/parser/planner/subquery.rs` — `get_subquery_first_column` 新增 `Join(_) | NestedLoopJoin(_)` 拒绝臂。
- `src/parser/planner/query.rs` — `get_plan_output_columns` SubqueryEval 臂（原 :114 单行 → 插列实现）。
- `src/cli/lifecycle.rs` — `import_csv` INSERT 构造（原 :496）+ 就地注释更新（表名 quote_ident 包裹；比对与错误信息仍实参原文）。
- `tests/subquery_test.rs` — +4 用例 + `setup_o_r_s` 夹具；`error_msg` 的 `#[allow(dead_code)]` 移除（T2 启用后成为本次改动孤儿）。
- `tests/cli_test.rs` — +3 用例 + `seed_emp_region` 夹具。

**Deviations from Plan**

1. **T3 json 断言列名集合校准（Plan Risks 预授权路径）**：契约测试见证示例断言 `columns == ["id","region"]` 按实际代码形态不可达——行形状被 Preserve 锁定为「全外层行 + 标量插 index 1」（本会话二进制探针：`{"columns":["id","name","salary"],"rows":[[1,"East","Alice",50000],...]}`）。修后表头为 `["id","region","name","salary"]`（4 列 == 4 值）。按 Cycle Risks 指示以实际探针校准断言列名集合，未放宽「别名出现于标量位置（index 1）」与「columns 数 == rows 行宽」两条断言语义。**关联发现（交 Plan Review）**：delta spec `cli-noninteractive-shell` S1 THEN 穷举 `columns` 为 `["id", "region"]` 与同 Requirement 正文「json 输出 `columns` 数组长度 SHALL 与 `rows` 每行值数一致」+ Preserve 行形状自相矛盾（2 列头对 4 值行会重现本缺陷）；实现满足 Requirement 正文全部 SHALL，spec 场景穷举值需 Review 裁定修订。
2. **Gate 5 首跑修复（记录）**：clippy 首跑 6 处 `needless_borrow`（新夹具 `setup_o_r_s` 内 `exec(&db, …)`，`db` 已为 `&Database`）→ 修正后复跑归零；`cargo fmt --check` 首跑 2 处换行（新增 cli_test 代码）→ `cargo fmt` 后归零。均为新增测试代码局部问题，非产品代码。

**Blocker Handoff**

None required

**Blocker Resolution**

None

**Self-Review**

- Plan compliance: 四任务变更面与契约 Targets/Required changes 一致；Preserve 项（执行器行产出、`SubqueryEvalNode` 构造、其他 `get_plan_output_columns` 臂、`get_table` 逐字比对、`quote_ident` 本体、六形态臂与 `_` fallback、既有断言集）逐项核对零触碰；Forbidden（`extract_correlated_params`/多列无 JOIN 形态/执行器）未进入 diff。
- Full diff reviewed: 是。本 Cycle diff 限 7 文件（上列）；工作区其余改动均为 Plan Context 声明的基线（MS13 实施 + 收尾 docs sync，未提交）。跨任务交互检查：T2（parser error 面）/T3（表头面）/T4（import 面）变更面互不重叠；T1 仅测试结构。无计划外修改。
- Critical findings unresolved: 无。
- Important findings unresolved: 无。
- Minor findings unresolved: delta spec S1 穷举值矛盾（见 Deviations 1，属 spec 文字修订，交 Plan Review）；`tests/nested_loop_join_test.rs:267` 注释引用旧文案（Plan 调查已知，无断言锁定，未动）。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| T1 变更前 GREEN | `cargo test --lib resolve` | `3 passed; 0 failed`，exit 0 | resolve.rs 既有 3 测试（现状见证） | PASS |
| T1 变更后 | `cargo test --lib resolve` | `2 passed; 0 failed`（3→2），exit 0 | 合并后测试 + 产品代码零改动 | PASS |
| T2 RED | `cargo test --test subquery_test in_subquery` | 2 failed：JOIN 文案断言失败于现状 `Plan error: Subquery returns multiple columns (IN subquery requires single column)`；exit 101 | 单列/多列 JOIN 现状误报形态 | PASS（预期 RED） |
| T2 GREEN | `cargo test --test subquery_test` | `32 passed; 0 failed`（28 既有 + 4 新增），exit 0 | 新拒绝臂 + 既有子查询面零回归 | PASS |
| T2 边界 | `cargo test --test nested_loop_join_test` | `9 passed; 0 failed`，exit 0 | NestedLoopJoin 形态既有面 | PASS |
| T3 RED | `cargo test --test cli_test scalar_subquery` | 2 failed：json 实际 `columns` 为 `["id","name","salary"]`（对 4 值行）、table 表头无 `region`；exit 101 | ISS03 缺陷现状形态（与台账实证同形） | PASS（预期 RED） |
| T3 GREEN | `cargo test --test cli_test scalar_subquery` | `2 passed; 0 failed`，exit 0 | 表头插列 + 行形状保持 | PASS |
| T4 RED | `cargo test --test cli_test test_import_escaped_quoted_table_name` | 1 failed：import exit ≠ 0（SQL 解析报错）；exit 101 | I048 缺陷现状形态 | PASS（预期 RED） |
| T4 GREEN + 边界 | `cargo test --test cli_test` | `81 passed; 0 failed; 2 ignored`（78 既有 + 3 新增），exit 0 | import/dump 既有组零修改 + 三新用例 | PASS |
| Iteration 全量 | `cargo test --no-fail-fast` | 退出码 0，全部套件 `0 failed`（含 subquery_test 32 / cli_test 81 / resolve 2；预期总数 1056 = 基线 1050 − T1 合并 1 + 新增 7） | 全仓测试面 | PASS |
| clippy | `cargo clippy --all-targets -- -D warnings` | 首跑 6 处 needless_borrow（新增测试代码）→ 修复；复跑 `Finished` 0 warning，exit 0 | 全目标静态分析 | PASS |
| fmt | `cargo fmt --check` | 首跑 2 处 diff（新增测试代码）→ `cargo fmt`；复跑 exit 0 | 全仓格式 | PASS |
| OpenSpec | `openspec validate --changes` / `--specs` | `1 passed, 0 failed` / `36 passed, 0 failed`，exit 0 | change 结构 + specs 语料库 | PASS |

**Persisted Evidence**

None required（Plan Context Mode: none；全部验证命令可低成本重跑，Act Response 承载决定性输出）

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| Improvement | 多列无 JOIN IN 子查询「静默取首列」语义裁决（显式拒绝 or 行值 IN） | 本 Response Deviations/验证 + proposal Out of Scope（本会话探针 `IN (SELECT r.a, s.b FROM r, s)` 静默按首列比较 exit 0） | change 范围外预登记候选，本 Cycle 未触碰该形态（Forbidden 项保持） |
| Improvement | IN×JOIN 能力解锁（`get_subquery_first_column` Join 形态臂取列 + `extract_correlated_params` ON 遍历注册关联参数） | 本 Response T2 实现 + proposal Out of Scope（新拒绝臂使缺口面有了与事实相符的错误文案） | change 范围外预登记候选 |
| Issue（台账修订） | ISS01 台账需补记 MAX 毒化方向（`set_all_visible` 无条目 `or_insert MAX` + 写后 `clear_all_visible` 不重置 → `{min=MAX, all_visible=false}` 整页误判不可见）与修复结果 | proposal Why 节 Plan 调查新发现 + Iteration 001 T5 将实施 | Recorder 落账属 Iteration 001/用户指令流程，Act 只报告 |

**Remaining Issues**

- delta spec `cli-noninteractive-shell`「标量子查询输出列的表头形状」S1 THEN 穷举列名集合 `["id", "region"]` 与行形状 Preserve 矛盾（Deviations 1）——需 Plan Review 裁定 spec 文字修订；行为实现满足 Requirement 正文全部 SHALL。
- 无其他遗留。

**Commit or Diff Reference**

未创建 commit（工作区含基线 MS13 实施与 docs sync 待用户统一提交；本 Cycle 改动以本 Response Changed Files 清单为准）

## Plan Review

- Review Result: accepted

**Findings**

- **F1（非阻塞，Plan 侧产物，已由本 Review 修正）**：delta spec `cli-noninteractive-shell`「标量子查询输出列的表头形状」S1 THEN 穷举 `columns` 为 `["id", "region"]`，与同 Requirement 正文「`columns` 数组长度 SHALL 与 `rows` 每行值数一致」及 Preserve 行形状（全外层行 + 标量插 index 1 → 4 列对 4 值）自相矛盾——2 列头对 4 值行会重现本缺陷本身。实现满足正文全部 SHALL；Act 测试按 Cycle Risks 预授权路径以实际探针校准断言（`["id","region","name","salary"]`，列数 == 行宽 == 4，别名在 index 1），未放宽断言语义。本 Review 已将 THEN 修正为实际行为穷举。
- **F2（非阻塞，Plan 侧产物，已由本 Review 修正；本 Review 新发现）**：同 spec S2「标量子查询位于末列」THEN「表头 `id | name | region` 三列，与三值行对齐（change 前表头两列对三值行）」与自身 GIVEN（emp 三列）不符——实际输出 4 列 `[id, name, region, salary]` 对 4 值行，change 前为 `[id, name, salary]` 三列头对 4 值行。Act 的 table 用例断言（表头含三列名、行含 region 值）正确见证 Requirement 正文，但用例名 `test_scalar_subquery_last_column_table_header` 与 doc 注释（「三列与三值行对齐（修前两列头丢第三值）」）沿袭同一形状误读——Minor（断言正确，名称/注释属局部描述失准，不阻塞，不要求修复）。spec 场景标题与 THEN 已由本 Review 修正为实际行为。
- **F3（Minor，不处理）**：`tests/nested_loop_join_test.rs:267` doc 注释以旧误报文案 `Subquery returns multiple columns` 作为「e2e 关联 ON×JOIN 不可达」机理依据——该注释是 MS09 001-rework 见证改形的历史现场记录；修后机理变为新拒绝臂（「不可达」结论仍成立，引用文案失准）。历史注释无断言锁定，不要求修复。
- **F4（记录，无需动作）**：Act 首跑 clippy 6 处 `needless_borrow` / fmt 2 处换行均位于本 Cycle 新增测试代码，修复后复跑归零——新增代码首跑修复，非已通过验证的重试，Gate 5 无违例；`error_msg` 的 `#[allow(dead_code)]` 移除属本次改动孤儿清理，合规。
- **独立代码检查**：四任务变更面逐点读码与 Task Contract 一致——T1 合并测试断言集完整、`EnvGuard` 原样、产品代码零改动；T2 新臂位于 `_` fallback 前、既有六形态臂与 `_` 逐字节未动、变体紧邻 `SubqueryReturnsMultipleColumns`；T3 `min` 守卫与执行器 `subquery_eval.rs:186-192` 的 `idx <= len ? insert : push` 双分支精确镜像（idx > len → insert at len ≡ push 位置）、其他臂零触碰；T4 `quote_ident(&table)` 单点替换、`get_table` 逐字比对与错误信息实参原文未动。Forbidden 面（`extract_correlated_params`、多列无 JOIN 形态、执行器行产出）未进入变更面。

**Deviation Classification**

- Deviation 1（T3 json 断言校准）：`ACT-DEVIATION`，走 Cycle Risks 预授权路径，断言语义未放宽——合规；其根因（spec THEN 穷举错误）为 `PLAN-INVALID`（Plan 侧 spec 文字缺陷，非代码缺陷），已随本 Review 修正（F1/F2）。
- Deviation 2（clippy/fmt 首跑修复）：非偏差，新增测试代码首跑修复（F4）。

**Acceptance Gaps**

None —— T1（resolve 3→2 全绿）、T2（新 4 用例 + subquery_test 32 / nested_loop_join_test 9 零修改全绿）、T3（新 2 用例 + cli_test 81 全绿含 I034 既有表头组）、T4（转义名 import 用例 + import/dump 组零修改）全部满足；Iteration 级全量 `--no-fail-fast` 一次绿（预期 1056）+ clippy/fmt 归零。

**Convergence**

N/A（首次 Review，无父 Cycle gap 可比较）

**Evidence**

- 独立读码：`src/cli/resolve.rs:79-118`（合并测试）、`src/parser/error.rs:48-49`（变体）与 `:105-107`（Display）、`src/parser/planner/subquery.rs:438-443`（新拒绝臂；既有臂 :391-437 与 `_` fallback 未动）、`src/parser/planner/query.rs:114-122`（表头插列臂）、`src/cli/lifecycle.rs:494-501`（quote_ident 包裹）、`src/executor/subquery_eval.rs:186-192`（镜像依据）、`tests/subquery_test.rs:732-820`（4 新用例 + 夹具）、`tests/cli_test.rs:2793-2924`（3 新用例 + 夹具）。
- 验证采信（公共规则 › 验证：本会话同一工作区，覆盖面文件经读码与 Act Response 记录一致、结论产生后未变化，预期计数自洽 1050−1+7=1056 / subquery 28+4=32 / cli 78+3=81 / resolve 3→2）：Act Response Verification Evidence 全表 PASS 结论采信——全量 `--no-fail-fast` 一次绿（exit 0）、clippy/fmt 复跑 0、`openspec validate` changes 1 passed + specs 36 passed。
- Persisted Evidence Mode `none`：本 Cycle 无 Evidence 目录要求，目录不存在不构成 finding。

**Follow-up Decision**

接受（accepted）。实现满足全部既有 Acceptance；两处 delta spec 文字错误为 Plan 侧产物缺陷，已由本 Review 直接修正（Plan 对 specs 产物的职责范围），无需 Act 当前 Cycle 修复，不构成 rework/replan。三条范围外候选（IN×JOIN 能力解锁、多列 IN 首列静默语义裁决、ISS01 台账补记 MAX 毒化）已由 Act Response Experience Candidates 报告，落账走 Recorder/后续规划流程，不阻塞本 Iteration。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

`iterations/001-visibility-summary-closeout/000-initial.md`（按 Iteration Map 展开；依赖 Iteration 000 全量门稳定——Act 已验证一次绿，采信）
