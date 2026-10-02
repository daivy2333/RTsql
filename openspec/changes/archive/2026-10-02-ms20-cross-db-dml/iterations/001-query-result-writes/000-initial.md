# Iteration 001 / Cycle 000：查询结果写入与完整使用说明

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- 所属 Iteration：001-query-result-writes（change tasks 2.1–2.3，T4、T5、T6）。
- Change tasks：T4、T5、T6。
- Acceptance gaps：None。
- Repair items：None。
- Inherited scope：获批 R1–R7；Iteration 000 已交付并 accepted 的附加库写面（路由/独立提交/关闭/边界/恢复）全部保持；design D4 的 INSERT SELECT 设计与 R7 delta spec（`specs/sql-write-surface/spec.md`）为本轮需求权威。
- Excluded scope：附加 DDL/CTAS、跨文件原子事务、UPDATE/DELETE 语法扩展（子查询仍拒绝）、存储格式、性能优化/bench、流式化；不预建后继目录。

**Objective**

`INSERT INTO <既有表> SELECT ...` 主库与附加库目标端到端可用：源以完整 SELECT 编译与路由能力求值并先物化为原生 Value 行，再经既有 Insert/Upsert 执行器单事务写入（类型/NOT NULL/PK/UNIQUE/二级索引/冲突动作语义沿用）；README 双语与仓库 rtsql-docs 使用面同步；按 M19 分批串行完成全量回归，使本 change 可进入收尾。

**Investigation Facts**

- Current Baseline：2026-10-02，Iteration 000 accepted（见 `iterations/000-initial/000-initial.md` 最终 Act Response 与 Review）。master HEAD `4e1944f`，工作区含 MS28/MS29 与本 change T1–T3 未提交修改（该现场只用于诊断）。Iteration 000 交付面：三处 DML planner 拒绝已移除（DDL/CTAS/view 拒绝保留）；pipeline DML 根目标绑定（`routed_db` 选库、begin/abort/commit/`abort_tables` 全目标、abort 键本地名、非显式根目标只读检查）；四写构造臂取目标库 TableManager/BufferPool/TransactionManager/WAL；`attached_write_in_tx_rejection` 0A000（只判目标、先于构造）；CLI `AttachSession` 由 `execute_command_inner_mode` 在 select 外持有、`close_attached_databases` 逐个关闭、成功遇关闭失败 → exit 1 点名别名、原状态保持；`attach_database` 打开成功后应用 checkpoint 水位 env；`run_sql` 只借用会话；`import_csv` mode 调用已适配。验证结论：`cross_db_dml_test` 16 passed/0 failed、`--lib cli::tests` 4 passed/0 failed、fmt/clippy clean（2026-10-02 Act 真实执行，Review 采信）。
- 当前行为（INSERT SELECT）：`extract_insert_values`（ddl_dml.rs:593）只接受 `SetExpr::Values`，其余 source 在 :679 返回 `PlanError::UnsupportedStatement("INSERT ... SELECT")`——原生命令实证 exit 3（Iteration 000 调查记录）。
- 源查询默认库恒主库：T1–T3 实施未更换 `create_executor_from_plan` 的主库参数（design D1 保留），裸名/扁平名源解析主库、限定名经 RoutedTables 路由，均直接可用。

**Current-State Evidence / Code and Critical Path**

| 文件与符号 | 实际入口、数据与责任 |
|---|---|
| `src/parser/planner/mod.rs:406`（build_plan Insert 臂） | `self.build_insert(table_name, columns, source, on, replace_into)`——build_plan 持 `&mut self`，`build_insert` 改 `&mut self` 后调用链不变 |
| `ddl_dml.rs::build_insert`（:232） | 目标校验序：`reject_view_write` → `validate_table` → 列清单 → `extract_insert_values`（仅 VALUES）→ `map_insert_values`（列清单校验：未知 `ColumnNotFound` / 重复 / 数量 `insert_count_error`；省略位与 DEFAULT 位填 defaults/NULL、显式位映射）→ `build_upsert_action`（:281，仲裁目标 `resolve_conflict_target` 仅 INT 声明 PK 或唯一列；DO UPDATE 三形态赋值；`replace_into` → `Arbiter::All + Replace`）→ 产出 `Insert`/`Upsert` |
| `query.rs:1140 build_query(&mut self, &Query)` | 完整 SELECT 编译入口（WITH/DISTINCT/集合操作/JOIN/视图展开钩子）；CTAS（ddl_dml.rs:1084）与视图定义（:1454）先例同型调用；源必须以顶层上下文编译，不设 `building_subquery` 抑制分支 |
| `query.rs:51 get_plan_output_columns` | 输出列名提取；DML 四变体臂返回 `Vec::new()`——新变体需加臂（match 穷尽，编译强制）；源宽度计划期判定可复用本函数递归源子树 |
| `src/executor/plan.rs` | `InsertNode { table_name, columns, values: Vec<Vec<Value>> }`（:182）、`UpsertNode { table_name, values, arbiter, action }`（:233）、`ConflictArbiter`/`ConflictAction`——新 `InsertSelect` 变体与节点落此（节点携带：限定目标名、源计划子树、源宽度、目标位置映射、全宽 DEFAULT/NULL 填充、可选仲裁/动作） |
| `pipeline.rs:1906 extract_all_table_names` | `Statement::Insert` 分支现仅返回目标（:1912）；需保留目标并追加 `extract_all_query_table_names(source)`（:1952，CTE 感知——WITH 定义名过滤、后定义遮蔽）。`register_table`（:2267）消费该清单：附加命中 → schema 注册 + `mark_attached_table` + RoutedTables 条目（local_name）；主库 miss → 视图豁免（`register_view_base_tables_recursive`）或 42P01 |
| `pipeline.rs:177 plan_stage_routed` | INSERT 落 `_` 通用臂（register_table + build_plan）；缓存纪律 `is_cacheable(stmt)` 仅 SELECT（:2349）——新 DML 计划结构性不进 cache，无需新增守卫 |
| `pipeline.rs:300 execute_stage_routed` | DML 臂（:399–497）：`readonly_rejection` 主库前置 → 目标名 → `routed_db` 选目标 → 非显式根目标只读检查 → begin → `abort_tables`（本地名键）→ `create_executor_from_plan(..., Some(tx_id), None, routed)` → 错误 abort / 成功 `target_db.commit()`（Commit failed 文案，内层点名 checkpoint failed）|
| `pipeline.rs:929 execute_stage_in_tx_routed` | `readonly_rejection`（25006 优先）→ `attached_write_in_tx_rejection`（0A000，只判目标）→ DDL/CTAS/索引/ALTER/TRUNCATE 点名臂 → `_` 臂：`statement_snapshot(Some(tx_id))` + `create_executor_from_plan(Some(tx_id))`（Insert/Upsert 由此执行，主库显式事务复用） |
| `pipeline.rs:545 readonly_rejection` | 13 写变体显式清单 + 防御 `_ => return None`——**新变体必须显式加臂**，否则静默逃逸只读守卫 |
| `pipeline.rs:580 attached_write_in_tx_rejection` | 四 DML 变体 + `_ => None`——新变体目标命中 routed 需 0A000，同理必须显式加臂 |
| `pipeline.rs:619 execute_ctas_routed` | 物化先例：`statement_snapshot(None)` → `create_executor_from_plan(*select, None, snapshot, routed)` → `next()` 循环收集 `Vec<Vec<Value>>`（`Ok(Some(_)) | Ok(None)` 终止，`Err` 即返回）——T5 物化复用此模式，不调用建表/推导/清理 |
| `executor/plan_format.rs` | `render_node` 叶子穷尽清单（:65–81，新变体编译强制）+ `describe`（:86，Insert :136 / Upsert :142 同型）——新变体需描述行 + 源子树渲染 |
| `executor/correlated.rs` | 叶子/DML 清单（:81 起）`=> {}`——新变体需下探源子树（关联参数注入递归源，CTAS 臂 :91 先例同型） |
| `cli/mod.rs run_sql` | 每语句 `plan_stage_routed` → `get_plan_output_columns` → 按 `session.tx_id()` 分发两个 routed 叶子 → `AffectedRows`/`QueryResult` 渲染——INSERT SELECT 走既有分发，输出 `affected_rows` 无需新臂 |

测试入口：`tests/cross_db_dml_test.rs`（夹具模式先例：cross_db_fixture 三库、routed lib 入口 helper、位点目录注入）；`tests/ctas_test.rs`（物化/类型面回归）；`tests/planner_test.rs`（计划形状）；`ddl_dml.rs` 文件内 `test_build_insert` 模块（:1875，planner 单测先例）。T5 新文件 `tests/insert_select_test.rs` 复用同一夹具形态。

**Implementation Guidance**

顺序 T4 → T5 → T6。T4 先以 planner 单测/计划形状测试建立 RED（现拒绝点 :679 使 INSERT SELECT 形态全部失败），再改 `build_insert` 为 `&mut self` 并分派：`source.body` 非 `SetExpr::Values` 时走 INSERT SELECT 臂——顶层上下文 `build_query` 编译源，`get_plan_output_columns` 取源宽度，列清单校验复用 `map_insert_values` 的清单验证语义（未知/重复；无清单 = 全列按位），数量对源宽度判定（空源同样校验——结构验证不得依赖数据行），产出 `InsertSelect` 节点（仲裁/动作经 `build_upsert_action` 复用解析）。同批补齐新变体的穷尽消费臂（readonly_rejection、attached_write_in_tx_rejection、get_plan_output_columns、plan_format、correlated——后两者编译强制）。T5 在两个 execute 叶子加 InsertSelect 臂：物化（CTAS 模式）→ 逐行按预编译位置填全宽（源 NULL 保持，省略位填 defaults/NULL）→ 归一为 `Insert`/`Upsert` 计划落入既有 DML 根/`_` 臂（不复制提交/回滚/abort 逻辑）；行宽不符运行期点名错误，不截断 zip；隐式路径物化成功后才 begin 目标；主库显式事务物化用 `statement_snapshot(Some(tx_id))` 并复用该 tx_id。T6 文档示例逐一在隔离 TempDir 实跑后写入，最后按 M19 完成分批全量。私有节点字段形状、辅助函数命名、`describe` 文案、归一化触点（臂内联 or 先归一再递归）为非实质选择。

**Task Contracts**

### T4：INSERT SELECT 查询计划、源表注册、目标列映射与枚举消费面

- Requirement/Scenario：R7（计划面：列数/未知列/重复目标列源求值前拒绝、空源结构验证、源编译能力全量）；R4（附加目标仍走路由标记）。
- Depends on：Iteration 000 accepted（已满足）。
- Targets：`src/executor/plan.rs`（`PhysicalPlan::InsertSelect` + 节点）；`src/parser/planner/ddl_dml.rs::build_insert`（+ mod.rs Insert 臂调用适配）；`src/pipeline.rs::extract_all_table_names`（INSERT 分支补源）、`readonly_rejection`、`attached_write_in_tx_rejection`；`src/parser/planner/query.rs::get_plan_output_columns`；`src/executor/plan_format.rs`、`src/executor/correlated.rs`；`tests/planner_test.rs` 或 `ddl_dml.rs` 文件内测试模块。
- Current behavior：INSERT source 非 VALUES 在 `extract_insert_values:679` 点名 `UnsupportedStatement("INSERT ... SELECT")`；`extract_all_table_names` INSERT 分支仅目标。
- Required behavior：SELECT source 产出 `InsertSelect` 计划节点——源以顶层上下文编译（WITH/DISTINCT/派生表/视图/集合操作完整消费，不误用 `building_subquery`），节点携带目标限定名、源计划、源宽度、位置映射、全宽 DEFAULT/NULL 填充、可选仲裁/动作（`build_upsert_action` 复用，`REPLACE INTO ... SELECT` 同型）；列清单校验与 VALUES 共享（未知列 `ColumnNotFound`、重复列点名）；清单/表宽与源宽度不符计划期拒绝（空源同样成立——结构验证不依赖行）；`extract_all_table_names` 保留目标并追加源查询表名（CTE 感知），源表经 `register_table` 注册 schema/路由/视图豁免；新计划不进 plan cache（既有 `is_cacheable` 结构性保证）；新变体在 readonly（25006）/显式事务附加目标（0A000）/输出列（空）/plan_format（描述+源子树）/correlated（下探源）各消费面显式覆盖。
- Preserve：VALUES 分支逐字节不变（既有 `test_build_insert` 保持）；三处 DML 附加目标放行与 DDL/CTAS/view 拒绝面不变；`EXPLAIN` 仅 SELECT 的拒绝面不变；主库参数不被改写。
- Forbidden：改写 SQL/源 AST、经 JSON 或 SQL 文本往返、在 T4 引入执行行为（执行归 T5）、为枚举变体添加不可达兜底臂替代显式处理、扩大 UPDATE/DELETE 语法。
- Test witness：先建 planner 级 RED——INSERT SELECT 形态当前全部 `UnsupportedStatement("INSERT ... SELECT")` 失败；新断言覆盖：主库目标+附加源计划形状（RoutedTables 含源限定名、目标标记）、子集/重排列清单映射、清单数量与源宽度不符拒绝、未知列拒绝、重复目标列拒绝、空源结构合法（计划期通过）、`REPLACE INTO ... SELECT` 仲裁形态、VALUES 回归不变。执行 `cargo test --test planner_test -- --test-threads=1`（及对应文件内单测命令）RED。
- GREEN condition：上述计划面用例全绿；`cargo test --lib parser:: -- --test-threads=1`（ddl_dml 单测）0 failed；crate 编译通过（穷尽臂由编译强制证明）。
- Verification：`cargo test --test planner_test -- --test-threads=1`；`cargo test --lib -- --test-threads=1` 中 ddl_dml/planner 相关模块；`cargo clippy --lib --bin rtsql --tests -- -D warnings` 无新告警。
- Stop when：源编译需要扩大 `build_query` 能力（递归 CTE、LATERAL 等）、列映射与 VALUES 语义无法共享、或附加源视图引用需要新命名空间语义——返回 Plan。

### T5：原生物化、单事务写入、冲突动作、失败及主库显式事务

- Requirement/Scenario：R7 全部场景（多库写入/写回主库与同表/列映射 DEFAULT/空源/源失败与插入失败/冲突动作/主库显式事务）；R2（目标单事务独立提交）、R3（附加目标显式事务拒绝沿用）、R4（只读目标 25006 沿用）、R5（提交边界 checkpoint 失败保数据）。
- Depends on：T4。
- Targets：`src/pipeline.rs::execute_stage_routed`、`execute_stage_in_tx_routed`（各加 InsertSelect 臂：物化 → 全宽填充 → 归一为 Insert/Upsert 进既有路径）；新增 `tests/insert_select_test.rs`。
- Current behavior：`InsertSelect` 计划在执行根落入查询 `_` 臂（无写语义）；VALUES 写面已可用。
- Required behavior：两个叶子在既有守卫（readonly 25006 优先、显式事务附加目标 0A000）后执行——隐式路径：`statement_snapshot(None)` 物化源为原生 Value 行（CTAS 模式，错误即返回、目标零触碰），物化完成后才 begin 目标事务；显式主库路径：物化用 `statement_snapshot(Some(tx_id))`，插入复用会话 tx_id 不另 commit，附加源按现有查询语义；逐行按预编译位置填全宽（源 NULL 保持、省略位 DEFAULT/NULL），归一为既有 `Insert`/`Upsert` 后走既有执行器/commit/abort/checkpoint 水位路径；同表读写只消费物化前行（物化先于 begin/写入，结构性保证）；行宽与源宽度不符运行期点名错误不截断；空源归一为零 values 写计划 → `affected_rows=0`；源失败不写目标、插入中途失败回滚本语句全部行与索引；输出为受影响行数。
- Preserve：既有 VALUES/Upsert/REPLACE 语义与错误面逐字节不变；附加目标只读/显式事务拒绝先于物化（源不求值）；commit 后 checkpoint 失败保持已提交数据（`Commit failed`/`checkpoint failed` 文案）；不复制 DML 根事务逻辑。
- Forbidden：JSON/SQL 文本中转、循环逐行 `execute_sql`（破坏语句原子性）、流式化或新执行器状态机、跨文件事务协调、为目标归一改写 `TableMeta.name`。
- Test witness：T4 GREEN 后建立 `tests/insert_select_test.rs` RED（此时执行面未接，相关用例失败），覆盖 RTM 列名：`insert_select_cross_db_join_and_main_target`（主库+附加库 JOIN 源写附加/主库目标、行与计数、源表不变、重开一致）、`column_mapping_defaults_and_empty_source`（子集/重排/DEFAULT/NULL 保真/类型门与 VALUES 同、空源 affected 0）、`self_source_materializes_once`（同表写回有限结束、只消费写入前行）、`select_conflict_actions`（DO NOTHING/DO UPDATE/REPLACE 主库与附加目标、计数与既有仲裁一致）、`source_error_no_target_write`（源求值失败目标不变）、`insert_select_late_constraint_error_rolls_back_target`（多行中途违反唯一/NOT NULL → 目标全部回滚、索引条目还原、已有数据保持）、`insert_select_main_explicit_tx_commit_and_rollback`（BEGIN 内主库目标插入 COMMIT/ROLLBACK 两态、附加源不变）、`insert_select_target_auto_checkpoint_failure_preserves_commit`（目标 1 字节水位 + 位点目录注入，提交保持）。
- GREEN condition：上述矩阵全绿；受影响回归（写面/CTAS/跨库/只读）0 failed。
- Verification：`cargo test --test insert_select_test -- --test-threads=1`；`cargo test --test cross_db_dml_test --test ctas_test --test subset_insert_test --test upsert_test --test readonly_session_test --test cross_db_query_test -- --test-threads=1` 每次 0 failed；`cargo fmt --check` 与 `cargo clippy --lib --bin rtsql --tests -- -D warnings`。
- Stop when：物化需要流式/分批语义、显式事务与物化快照出现跨库一致性需求、或同表写回出现自读自写环——返回 Plan。

### T6：README 双语、仓库 rtsql-docs 使用面同步及最终分批回归

- Requirement/Scenario：R6（使用说明：各文件独立提交边界、显式事务限制、INSERT SELECT 可达）+ 全部 requirement 的最终回归面。
- Depends on：T5。
- Targets：`README.md`、`README.zh-CN.md`、`rtsql-docs/SKILL.md`；全量测试（M19 口径）。
- Current behavior：文档含 ATTACH 只读跨查/CTAS 使用面（MS20 一期措辞），无跨库 DML/INSERT SELECT 说明；`sql-attach-session` 时代的只读声明已在 Iteration 000 校准。
- Required behavior：双语 README 与仓库 rtsql-docs 增补/修订跨库写入使用说明——附加库 DML 路由与独立提交（此前文件提交不被后续语句失败撤销）、显式事务附加写入 0A000 限制、只读 25006、`INSERT ... SELECT` 跨库/同表用法与空源/冲突动作语义、busy-timeout 与锁行为不回退；每个写入文档的 SQL 示例先在隔离 TempDir 实际运行并核对输出；最终全量按 M19 执行。
- Preserve：既有文档结构与其余章节内容；不写跨文件原子承诺；不删除既有示例。
- Forbidden：性能声明、未实跑的示例、变更范围外的文档重写。
- Test witness：文档示例逐条命令 + 输出记录于 Act Response（每条 ≤20 行决定性输出）；M19 全量分批——`--lib`/`--bins` 与全部集成测试目标按名字典序分批（每批约 8–12 个目标），每批独立 `cargo test <明确目标列表> -- --test-threads=1`，逐批记录 `test result: ok` 行；不使用无目标过滤的全量调用。
- GREEN condition：文档内容 Review 逐条通过（示例实跑输出与声明一致）；全部批次 0 failed；`cargo fmt --check` 与 clippy clean。
- Verification：示例运行命令与输出（Act Response）；M19 各批命令与原生结果行；批次清单与目标覆盖在 Act Response 列全。
- Stop when：文档示例暴露与 spec 冲突的行为（返回 Plan 核对语义）；全量出现失败（定位归属后按三次失败规则处理）。

**Invariants**

- 源查询默认库恒主库；附加目标与附加源均经计划期 RoutedTables 判定，不 split 平坦字符串。
- 物化完成先于目标 begin/任何写入；同表写回不消费本语句新插入行。
- 每库事务/WAL/缓冲池/索引归属一致；主库显式事务复用 tx_id，不引入第二提交点。
- 只读/显式事务守卫先于源求值与目标写入；新变体不得逃逸任何写守卫清单。
- 复用现有实现优先；不改存储格式、不加依赖、不重构公共 API、不清理用户无关修改。
- 每项任务先建立 RED 或合法未失效 GREEN 见证再修改；完成后先 spec review 再 code review，Gate 3 结果写 Act Response。

**Non-goals**

附加 DDL/CTAS、DETACH/REPL/serve、跨文件原子提交、UPDATE/DELETE 子查询扩展、递归 CTE/LATERAL、存储格式、性能优化/bench、流式化物化、持久化 Evidence 与工程经验产物、全局状态维护、归档或提交。

**Acceptance**

| 条件 | 任务/测试与可观察量 |
|---|---|
| B1：INSERT SELECT 计划面完整（映射/校验/空源/路由/消费臂） | T4 计划形状与拒绝矩阵单测 GREEN + 编译通过 |
| B2：物化写入端到端（主库/附加目标、冲突动作、空源、计数） | T5 insert_select_test 矩阵 GREEN + 重开一致 |
| B3：失败边界（源失败零写入、中途失败全回滚、行宽错误） | T5 source_error / late_constraint 用例 + 目标索引/已有数据保持断言 |
| B4：主库显式事务与运行形态（COMMIT/ROLLBACK、附加源、checkpoint 失败保提交） | T5 explicit_tx / auto_checkpoint_failure 用例 |
| B5：文档与全量（使用面同步 + M19 分批 0 failed） | T6 文档示例实跑记录 + 各批 `test result: ok` |

Iteration 001 RTM：R7→T4/T5、R2→T5（沿用 T1 见证+新增）、R3→T5（守卫沿用+显式事务主库目标见证）、R4→T5（只读目标沿用）、R5→T5（checkpoint 失败见证）、R6→T6、R1/R8 沿用 Iteration 000 见证不再重复。全部 Covered（change tasks.md RTM 为准）。

**Verification**

按 Task Contract 的直接目标 → 受影响边界 → fmt/clippy 顺序；同一覆盖通过且未改变即采信。判定仅依据 Rust 测试框架原生输出与退出码、文档示例实跑的直接可观察输出；无附加判定层。新增用例独立 TempDir、隔离 RTSQL_HOME、显式 `--format json`。最终全量仅按 M19 分批口径执行（T6），不在 T4/T5 提前宣称全量；不重复运行已通过且覆盖未变的验证。

**Gate 2 Readiness**

PASS：设计（design D4/D5）与 R7/R6 delta 规格闭合；实现调查覆盖 planner/执行/物化先例/守卫清单/文档面全部触点；T4–T6 依赖有序、契约自包含；RTM 全 Covered；无 Missing/Simplified、无实质未知项；用户已于 2026-10-02 批准完整计划（原话「批准」）；Persisted Evidence 模式 none。依赖前置：Iteration 000 已 accepted（2026-10-02）。

**Persisted Evidence**

- Mode: none。
- 命令、每项 ≤20 行决定性输出、退出码、修改符号与 Gate 结论写 Act Response；测试与文档示例均可低成本重跑，不创建 evidence/。

**Risks and Notes**

- 大源结果集整体物化占内存——沿项目当前物化形态（CTAS 同款），本期不做流式（design Risks 已接受）。
- 复合源（UNION 等）计划期宽度判定与运行行宽可能不一致——运行期行宽点名错误兜底，不静默截断。
- 附加库视图作源：现有 `register_table` 附加分支按表查询，视图引用 42P01——既有边界，不扩展（design Risks 第 5 条）。
- 日期族/类型门语义经既有执行器 coerce 与类型门通道，不新增类型逻辑。
- 范围外引擎缺陷候选（abort × 槽位重用 × 崩溃恢复，Iteration 000 报告）不在本轮处理；若 T5 恢复类见证触发该缺陷边界，按 Iteration 000 的见证分布先例调整并记录，不现场修复。
- 本轮结束停在 Act Response，待 Plan 独立完整 diff Review；不创建后继 Cycle/Iteration、不维护全局状态、不归档。

## Act Response

- Status: reported
- Date: 2026-10-02（T4 → T5 → T6 顺序执行，每任务先 RED/GREEN 见证再实施）

### 实际改动

**T4 计划面**

- `src/executor/plan.rs`：新增 `PhysicalPlan::InsertSelect(InsertSelectNode)` 变体与节点——`table_name`（限定扁平名）、`columns`（语句清单，空 = 全列按位）、`source`（顶层上下文编译的 SELECT 计划子树）、`source_width`（计划期源宽度）、`target_positions`（清单→表列位置映射，无清单 = 恒等）、`fill`（全宽 DEFAULT/NULL 填充）、`upsert: Option<(ConflictArbiter, ConflictAction)>`（复用 `build_upsert_action`）。
- `src/parser/planner/ddl_dml.rs`：`build_insert` 改 `&mut self` 并分派——`SetExpr::Values` 源（含 source 缺省的 `MissingField("VALUES")` 面）走既有路径逐字节不变；SELECT 源走新 `build_insert_select`：`build_query` 顶层编译源（不设 `building_subquery`）、`get_plan_output_columns` 取源宽度、清单→位置映射与 VALUES 共享语义（未知列 `ColumnNotFound`、重复列同文案）、清单数/表宽与源宽度不符计划期 `insert_count_error` 拒绝（空源同样成立——LIMIT 0 用例见证）、`fill` 按表列声明 DEFAULT/NULL、`build_upsert_action` 复用（`REPLACE INTO ... SELECT` 同型）。
- `src/pipeline.rs`：`extract_all_table_names` INSERT 分支保留目标并追加 `extract_all_query_table_names(source)`（CTE 感知；VALUES 源提取为空，行为不变）——源表经 `register_table` 注册 schema/路由/视图豁免；`readonly_rejection` 新增 `InsertSelect(_) => "INSERT"`（25006 面与 INSERT 同文案，先于源求值）；`attached_write_in_tx_rejection` 新增 `InsertSelect(n) => &n.table_name`（0A000，先于源求值）；`create_executor_from_plan` 补 panic 防御臂（CTAS 先例——两个 execute 叶子先物化归一，本函数不见该变体）。
- `src/parser/planner/query.rs`：`get_plan_output_columns` DML 空臂加 `InsertSelect(_)`。
- `src/executor/plan_format.rs`：`render_node` 源子树下探 + `describe` 确定性描述（列清单/源宽度/冲突动作）。
- `src/executor/correlated.rs`：源子树参数注入臂（CTAS 同型）。
- `src/executor/mod.rs`：re-export `InsertSelectNode`。

**T5 执行面**

- `src/pipeline.rs`：`execute_stage_routed` 新增 InsertSelect 臂——`statement_snapshot(None)` 物化（`materialize_insert_select_source`，CTAS 同款消费模式：`create_executor_from_plan` → `next()` 收集原生 Value 行，错误即返回目标零触碰）→ `normalize_insert_select` → `Box::pin` 递归进既有 DML 臂（begin/abort_tables/abort/commit/checkpoint 水位零复制）；`execute_stage_in_tx_routed` 新增 InsertSelect 臂——`statement_snapshot(Some(tx_id))` 物化（会话自身未提交写可见）→ 归一 → 递归进 `_` 通用臂复用会话 tx_id（不另开提交点）。新私有 `normalize_insert_select`：逐行按预编译位置拼装全宽（源 NULL 保持、省略位 fill 落 DEFAULT/NULL）、运行期行宽不符点名错误不截断 zip、归一为既有 `Insert`/`Upsert` 计划。同表写回只消费物化前行（物化先于 begin/写入，结构性保证）；空源归一为零 values 写计划 → `affected_rows=0`。

**T6 文档**

- `README.md` / `README.zh-CN.md`：新「Cross-database sessions / 跨库会话」节（In-memory databases 与 Automatic checkpoint 之间）——ATTACH 限定名路由、每文件独立提交（后续语句失败不撤销此前提交）、`BEGIN` 内附加写 0A000、附加 DDL 计划期拒绝、`INSERT ... SELECT`（跨库 JOIN/同表写回/冲突动作/列清单位置映射/DEFAULT 填充/源失败零写入/空源 affected 0）、只读会话附加库只读与 25006；锁与 `--busy-timeout` 行为按既有声明引用未回退。
- `rtsql-docs/SKILL.md`：Sessions 能力行更新（routed writes / 0A000 / INSERT SELECT）+ 新「Cross-database sessions」小节（Transactions 之后，含实跑示例）。

**测试**

- 新文件 `tests/insert_select_test.rs`（10 用例）：RTM 八用例（`insert_select_cross_db_join_and_main_target` / `column_mapping_defaults_and_empty_source` / `self_source_materializes_once` / `select_conflict_actions` / `source_error_no_target_write` / `insert_select_late_constraint_error_rolls_back_target` / `insert_select_main_explicit_tx_commit_and_rollback` / `insert_select_target_auto_checkpoint_failure_preserves_commit`）+ 2 守卫见证（`readonly_insert_select_rejected_before_source_evaluation` / `attached_target_insert_select_in_tx_rejected`）。
- `src/parser/planner/ddl_dml.rs` 测试模块 +8：计划形状/子集重排映射/DEFAULT 填充/宽度失配两形态/未知列/重复列/空源结构合法/REPLACE 与 ON CONFLICT 仲裁。
- `tests/cross_db_dml_test.rs` +1：`insert_select_plan_registers_source_tables`（RoutedTables 含源/目标限定名）。

### 与计划的偏差

- 守卫见证用例增补：契约 Required behavior 要求「只读/显式事务附加目标拒绝先于物化（源不求值）」而 RTM 既有见证仅覆盖 VALUES 形态——在 insert_select_test 内增补 2 个 INSERT SELECT 形态守卫用例（见上）。无范围扩大（同文件、同 requirement 面）。
- 测试形态两处校准（契约内非实质选择）：`source_error_no_target_write` 初版源宽度 1 对目标 2 列被计划期宽度门先拒，改为 `(id)` 单列清单使失败只能来自运行期 CAST 求值；跨库 JOIN 用例初版用短名限定引用 `u.tag` 触发既有边界（附加限定 FROM 表 `o.u` 的隐式短名限定符不注册——纯 SELECT 同样 `Table not found: u`），改用全限定 `o.u.tag` 形态并注释既有边界。
- 归一化触点选择（契约授权的非实质选择）：两 execute 叶子均采用「物化 → 归一 → `Box::pin` 递归进入既有臂」，DML 根事务/守卫/执行器接线零复制；`fill` 以全宽向量 + 恒等映射（无清单时）承载，与 `map_insert_values` 的省略位语义一致。

### 自检发现与修复

- Gate 3 spec review：R7 七场景 + R2/R3/R4/R5 见证逐条对照通过；VALUES 分支与 `extract_insert_values` 未触碰（既有 `test_build_insert` 等原样通过）；DDL/CTAS/view 附加拒绝面未触碰；EXPLAIN 仅 SELECT 拒绝面未触碰（explain_test 10/0）；主库参数未被改写。
- 实施中修复：测试侧两处自身缺陷（上述校准）+ 两处编译期修正（`page_format::ColumnType` 与 `executor::ColumnType` 同名异型、测试 import）。
- 运行期行宽守卫（`normalize_insert_select`）为防御臂：计划期宽度校验（清单/表宽对源宽度 + 集合操作两侧列数相等）结构性封死 SQL 可达的失配形态，无独立见证用例（与 Risks 注记一致）。
- 遗留 Minor：无。

### 验证

| 验证 | 命令 | 决定性输出 | 退出码 |
|---|---|---|---|
| T4 RED | `cargo test --lib ddl_dml -- --test-threads=1` | 8 个 INSERT SELECT 用例失败（`Unsupported statement type: INSERT ... SELECT`），4 个既有 VALUES 用例通过 | 101 |
| T4 RED（路由） | `cargo test --test cross_db_dml_test insert_select_plan_registers` | `plan ...: Plan error: Unsupported statement type: INSERT ... SELECT` | 101 |
| T4 GREEN | `cargo test --lib ddl_dml -- --test-threads=1` | 12 passed; 0 failed | 0 |
| T4 GREEN | `cargo test --test planner_test -- --test-threads=1` | 96 passed; 0 failed | 0 |
| lib 全量 | `cargo test --lib -- --test-threads=1` | 364 passed; 0 failed | 0 |
| T5 RED | `cargo test --test insert_select_test -- --test-threads=1` | 7 failed（执行未接）→ 用例校准后 8 failed | 101 |
| T5 GREEN | `cargo test --test insert_select_test -- --test-threads=1` | 10 passed; 0 failed | 0 |
| 受影响回归 | `cargo test --test cross_db_dml_test --test ctas_test --test subset_insert_test --test upsert_test --test readonly_session_test --test cross_db_query_test -- --test-threads=1` | 17/17/6/32/14/17 passed, 0 failed | 0 |
| M19 全量 13 批（lib/bins + 108 集成目标，名字典序，每批 `--test-threads=1`） | 各批 `cargo test --lib --bins ...` / `cargo test --test <目标列表> -- --test-threads=1` | 合计 **1726 passed / 0 failed / 2 ignored**（cli_test 2 ignored 为既有；cli_test 117、planner_test 96、cross_db_dml_test 17、insert_select_test 10） | 0 |
| fmt/clippy | `cargo fmt --check` / `cargo clippy --lib --bin rtsql --tests -- -D warnings` | 0 diff / 0 告警 | 0 |
| change 校验 | `openspec validate 2026-10-02-ms20-cross-db-dml --strict` | `Change '2026-10-02-ms20-cross-db-dml' is valid` | 0 |
| 文档示例实跑（隔离 TempDir ×3） | README：跨库 INSERT SELECT + COUNT（exit 0, affected 2）；独立提交后失败语句（exit 3 fail-fast，main t=4 行 / orders recent=3 行均保持）；`BEGIN` + 附加写（exit 3, `WRITE TO ATTACHED DATABASE ... inside a transaction`）；`--readonly` INSERT SELECT（exit 3, `cannot execute INSERT in a read-only session`）；DO NOTHING 同表写回（exit 0, affected 1, count 5）。SKILL：people→orders INSERT SELECT（exit 0, count 1） | 每条输出与文档声明一致 | 0/3（预期） |

### Persisted Evidence

None required（Mode: none——测试与文档示例均可低成本重跑，Act Response 已保存决定性输出，未创建 evidence/ 目录）。

### Experience Candidates

- Issue 候选（既有边界，非本轮引入，1 项）：附加限定 FROM 表的**隐式短名限定引用**不可达——`ATTACH 'o' AS o; SELECT u.tag FROM t JOIN o.u ON t.id = u.id` 计划期 `Table not found: u`（纯 SELECT 同面，显式别名与全限定 `o.u.tag` 正常）；证据：本会话实跑对照 + `tests/insert_select_test.rs` 注释。属显式拒绝面而非静默缺陷，是否登记 Issue 由用户裁定。
- Runbook 候选：无。

### 未解决问题

None。本轮 Acceptance B1–B5 全部达成；change 全部 6 个 task 完成，等待 Plan 独立完整 diff Review（Gate 4 / Review Result）。

## Plan Review

- Review Result: accepted

**Findings**

2026-10-02 第三次审计（用户指令「实施完了，进行审计」）：独立审读 Iteration 001 完整变更面。计划面——`InsertSelectNode` 字段形状（目标限定名/列清单/源子树/源宽度/位置映射/全宽 fill/可选仲裁动作）、`build_insert` 改 `&mut self` 后 VALUES 分支逐字节不变（缺省 source 仍走 `MissingField("VALUES")` 面）、`build_insert_select`（顶层 `build_query` 编译、`get_plan_output_columns` 取源宽度、清单映射与 VALUES 共享文案、宽度对齐计划期拒绝〔空源同样成立〕、fill 按声明 DEFAULT/NULL、`build_upsert_action` 复用）、`extract_all_table_names` INSERT 分支（目标 + CTE 感知源表面，VALUES 源提取为空零回归）、全部穷尽消费臂（readonly `InsertSelect(_) => "INSERT"` 25006、显式事务目标 0A000、`get_plan_output_columns` 空臂、`plan_format` 描述+源子树下探、`correlated` 源注入、`create_executor_from_plan` panic 防御臂、re-export）。执行面——两个叶子均「守卫（顶部已完成）→ 物化 → 归一 → `Box::pin` 递归进既有臂」：隐式路径 `statement_snapshot(None)` 物化后递归 DML 根（begin/abort_tables/abort/commit/水位零复制）；显式路径 `statement_snapshot(Some(tx_id))` 物化后递归 `_` 臂复用会话 tx_id 不另开提交点；`normalize_insert_select` 运行期行宽点名错误不 zip 截断、源 NULL 保持、省略位 fill；`materialize_insert_select_source` CTAS 同款消费、错误先于目标 begin 返回。测试面——`tests/insert_select_test.rs` 10 用例（9 `#[test]` + 1 `#[tokio::test]`）与 RTM 八见证名逐一对应；`cross_db_dml_test` +1 路由见证（17 = 11 + 6 tokio）；ddl_dml 测试模块恰 +8 个 `test_insert_select_*`；`planner_test` 96 与声明一致。文档面——README 双语新「Cross-database sessions / 跨库会话」节内容逐条对应（独立提交边界、`BEGIN` 内 0A000、INSERT SELECT 语义、只读面、无跨文件原子承诺），SKILL.md Sessions 行与 Cross-database sessions 节同步；tasks.md 2.1–2.3 复选框已勾选。

- 数量闭合：1689（MS29 基线）+ Iteration 000（cross_db_dml 16 + lib 2）+ Iteration 001（insert_select 10 + cross_db_dml +1 + ddl_dml 单测 +8）= 1726，与 Act 报告的全量总数一致；`ls tests/*.rs` = 108 个集成目标与「108 集成目标」一致；无未声明的测试新增。
- 只读「源不求值」保证核实：CLI 会话附加库打开模式随主库（`attach_database` 以 `db.is_readonly()` 打开），故只读会话在函数顶部以 INSERT 形态拒绝（先于物化）；rw 主库 + 只读附加的组合在产品面不可构造，DML 根的目标只读防御检查（Iteration 000 偏差 2）覆盖库级组合。reachable 面上「附加目标只读/显式事务拒绝先于源求值」成立，见证（`readonly_insert_select_rejected_before_source_evaluation` / `attached_target_insert_select_in_tx_rejected`）与其一致。
- Minor（非阻塞，不要求修复）：① T6 Verification 原文要求「批次清单与目标覆盖在 Act Response 列全」，Act 以聚合形态记录（13 批 / 108 目标 / 合计 1726 passed / 0 failed / 2 ignored + 关键单文件计数）而非逐批罗列——决定性输出（合计与关键计数）已保存且经本审读独立闭合（数量算术 + 目标数 + 分文件 `#[test]` 计数），属记录粒度偏差，不构成验收缺口；② 文档示例逐条实跑输出以摘要形态记录（每类场景命令 + 输出 + 退出码），未逐条粘贴全部原始 stdout——在 ≤20 行决定性输出纪律内。
- 偏差分类：三项偏差（守卫见证用例增补、测试形态两处校准、归一化触点选择）均在契约授权范围内（同 requirement 面/非实质选择），已如实记录；无 Plan 遗漏或基线变化。

**Acceptance Gaps**

无。B1（计划面矩阵 +8 单测与路由见证 GREEN、编译通过）／B2（insert_select_test 10 用例含跨库 JOIN/主库目标/列映射 DEFAULT/空源 0/冲突动作/重开一致）／B3（源失败零写入〔运行期 CAST + 计划期未知列〕、多行中途 NOT NULL 全回滚 + 键可重用 + 二级索引路由）／B4（显式事务 COMMIT/ROLLBACK 两态、附加源不变、checkpoint 失败保提交）／B5（README 双语 + SKILL 同步、示例实跑、M19 13 批 1726/0/2、fmt/clippy clean）全部达成。

**Evidence**

- Act 真实执行的 RED/GREEN 与 M19 分批结论记录于当前 Act Response（T4 RED 8 failed → GREEN 12 passed；T5 RED 8 failed → GREEN 10 passed；lib 全量 364；受影响回归 6 目标 103 例 0 failed；M19 合计 1726/0/2；fmt/clippy/validate exit 0）；本次只读复审未见这些结论之后的覆盖表面变化，直接采信。
- 独立代码审读与 Act 声明逐项一致（见 Findings 逐条）；文档三处新节内容与 R6/R7 语义声明一致且不含跨文件原子承诺。
- Issue 候选（既有边界，非本轮引入，按规范只报告不落账）：附加限定 FROM 表的隐式短名限定引用不可达——`ATTACH 'o' AS o; SELECT u.tag FROM t JOIN o.u …` 计划期 `Table not found: u`（纯 SELECT 同面、显式拒绝非静默；显式别名与全限定 `o.u.tag` 正常）。Act 已附实跑对照，测试注释留证。是否登记 Issue 由用户裁定（Recorder 执行）。
- Persisted Evidence：none（模式不变；无 Blocker Handoff 待处理）。
- Gate 3：T4/T5/T6 的 spec/code review 记录于 Act Response 自检节（R7 七场景逐条对照、VALUES/DDL/CTAS/view/EXPLAIN 拒绝面未触碰核实）；Gate 4 PASS（新鲜 RED→GREEN + M19 全量 + 独立 diff 审读一致）。Gate 1/2 既有批准与就绪结论保持。
- SKIPPED：产品代码/测试修改（本次只授权 Review）、重复已通过验证（无失效疑点、无重跑触发情形）、bench（非目标）、经验落账/全局状态/归档（非本阶段授权）。

**Follow-up Decision**

接受当前 Cycle：Acceptance B1–B5 全部满足，仅两条记录粒度 Minor，不构成返工理由。Iteration 001 为 change 最后一个 Iteration，无后继 Iteration；change 全部 6 个 task 完成且两轮 Iteration 均 accepted，具备收尾条件——规格合并（4 delta 域）、SNAPSHOT/tasks 刷新与归档由 `openspec-docs-maintainer` 按用户指令执行。范围外 Issue 候选不落账。

**Iteration Plan Update**

None。

**Next Cycle**

None（Iteration 001 完成，Cycle 冻结）。

**Next Iteration**

None（无剩余 Iteration；change 实施面完成，待用户指令进入收尾）。
