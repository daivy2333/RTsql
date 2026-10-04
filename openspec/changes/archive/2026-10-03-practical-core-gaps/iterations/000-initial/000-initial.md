# Iteration 000 / Cycle 000: 正确性与诚实化小批（建表页初始化 + 非 INT PK 拒绝 + dump 渲染）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1、T2、T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R1/R4/R5 全部 scenario（proposal S1–S5、S18–S21）；design D1/D2/D3；集中决策 1（计划期拒绝 + 通道兼容）与 2（最小修复）
- Excluded scope: R2 聚合表达式嵌入（Iteration 001）、R3 UPDATE 表达式（Iteration 002）、文档同步（Iteration 002 T10）、open 期自愈（否决项）

**Objective**

新建即含零行表的库 `rtsql check` 干净通过（全建表通道）；非 INTEGER 显式 PRIMARY KEY 计划期点名拒绝且隐式主键/import 通道零波及；dump DDL 渲染与实际强制语义一致且新产物 restore 往返可达；损坏检测与既有面零回归。

**Investigation Facts**

- Current Baseline: HEAD 9a103c0，工作区含 `rtsql-docs/SKILL.md` 未提交文档修改（本轮直接修复，与本 change 代码面无冲突）；全量 1802 tests / 0 failures / 2 ignored（2026-10-03 Plan Review 采信，覆盖面未变化）。探针实测（release 二进制 2026-10-03 16:35 构建）：裸建表后 check 两行 error（`free space offset 0 lies outside the data area [16, 4096]` + `data page of table 'e' has page type 0x00, expected 0x03`）exit 1；同表 INSERT 一次后 check exit 0；TRUNCATE 后空表 check exit 0——**0x00 head 页可直接 INSERT 且写入后转绿**（写路径对未初始化页自带 init，与恢复重放「未初始化页 init」同源），存量空表「首次写入转绿」无需新写路径。
- Current-State Evidence:
  - `src/storage/data/table_manager.rs:398` `create_table_with_constraints(name, columns, pk: &str)`——`:424` `let page_id = self.buffer_pool.storage().allocate_page().await?;` 后直接进入 PK 索引创建（spawn_blocking IndexManager::new），数据页从不写入。全部建表通道收敛于此入口：CTAS（pipeline `execute_ctas_routed` → 本函数）、import 自动建表（CREATE TABLE DDL → `execute_sql`）、restore（同）、ATTACH 目标库（同）。
  - `table_manager.rs:1512-1517`（truncate_table 内）——现成的页初始化先例：`let guard = self.buffer_pool.get_page(head).await?; guard.modify_page(|page| { let _ = SlottedPage::init(page, 0x03); }); drop(guard);`
  - `src/parser/planner/ddl_dml.rs:950` `extract_primary_key(columns, constraints) -> Result<Option<String>>`——列级与表级 `PRIMARY KEY` 归一为小写列名，0/1/多候选三臂。`:996` `build_create_table`——`:1094` 调用 extract，`:1120-1130` 列级 UNIQUE 检查（`col.data_type != ColumnType::Int` 判断先例，PK 列经 `primary_key.as_deref() != Some(col.name.as_str())` guard 跳过）、`:1145-1170` 表级 UNIQUE（`is_pk` guard）。`column_defs` 元素携 `data_type: ColumnType`（前段 `convert_data_type` 已解析）。CreateTableNode `{ table_name, columns, primary_key }`。
  - 隐式主键通道：extract 返回 None 时 CreateTableNode.primary_key = None，引擎侧「未声明主键 → 首列隐式主键」在下游（CreateTableExecutor）fallback——**显式与否在 planner 处可区分，检查只挂显式分支**。import 自动建表不显式声明 PK（`cli-noninteractive-shell` 既有 Requirement 明文），FLOAT/TEXT 首列重复合法行为不受波及。
  - `src/cli/lifecycle.rs:963` `create_table_sql(table: &CatalogRow, columns: &[CatalogColumnRow])`——`:973` `if col.column_name == table.pk_column { part.push_str(" PRIMARY KEY"); }`；UNIQUE 尾缀独立判断 `col.unique`。PK 列的 UNIQUE 被 PK 消费不落独立标志（ddl_dml `:1128`/`:1149` 两处 is_pk guard；MS23 语义），存量 TEXT PK 列 `col.unique == false`——渲染修复后无 UNIQUE 残留。
  - check 面零回归锚点：`tests/check_command_test.rs` 8 用例（含损坏注入 exit 1、干净单行 exit 0）；`validate_slots`/页 census 在 `src/storage/integrity.rs`。
- Code and Critical Path: T1 修改面 = `table_manager.rs::create_table_with_constraints` 单点（allocate_page 与索引创建之间插初始化，镜像 ：1512 先例）。T2 修改面 = `ddl_dml.rs::build_create_table`（extract 之后、UNIQUE 检查前后皆可，先于 CreateTableNode 构造）+ 既有 `PlanError::UnsupportedConstraint` 通道（`parser/error.rs`，错误携带能力名、SQLSTATE 既有映射，无新变体）。T3 修改面 = `lifecycle.rs::create_table_sql` 单条件。三者互不依赖，可并行实施、共享回归边界。

**Implementation Guidance**

- T1 初始化语句镜像 ：1512-1517 原文形态（get_page → modify_page → SlottedPage::init(page, 0x03) → drop）；失败语义 `?` 传播，与既有「allocate 后续步失败」页生命周期一致，不新增清理路径。init 后页置脏，随正常 close 落盘（探针 TRUNCATE 用例证明该模式持久）。
- T2 判定形态对齐 ：1130 UNIQUE 检查先例：`pk 类型 != ColumnType::Int` → `Err(PlanError::UnsupportedConstraint("NON-INTEGER PRIMARY KEY"))`。表级 PK 已被 extract 归一为列名，同一检查点按列名查 `column_defs` 覆盖两形态。测试用例覆盖 FLOAT/TEXT/BOOLEAN/DATE/TIMESTAMP × 列级/表级矩阵；INTEGER PK 全形态（`INT`/`INTEGER` 拼写经 convert_data_type 归一）保持可达。
- T3 渲染条件加 `&& matches!(col.column_type, ColumnType::Int)`；存量夹具表经 lib 直建（含非 INT PK catalog 行）构造，dump→`rtsql new`→restore 往返断言 DDL 不含 PRIMARY KEY 且行集等价。
- 测试文件落位：T1 → `tests/check_command_test.rs` 扩展（空表干净/通道矩阵/两态/损坏注入零回归）+ 存量 0x00 夹具转绿用例；T2 → `tests/planner_test.rs` 拒绝矩阵 + `tests/constraint_enforcement_test.rs` e2e；T3 → `tests/cli_test.rs` dump 用例。

**Task Contracts**

### T1: 建表首数据页初始化——新建零行表 check 干净

- Requirement/Scenario: R1（S1 新建空库 check 干净 / S2 通道全覆盖 / S3 持久两态 / S4 损坏检测不放松 / S5 存量空表最小修复语义）
- Depends on: None
- Targets: `src/storage/data/table_manager.rs::create_table_with_constraints`（allocate_page 之后、PK 索引创建之前）
- Current behavior: 首数据页分配后保持零字节（page type 0x00）；裸建表后 `rtsql check` 报两行 error、exit 1；首次 INSERT 或 TRUNCATE 后转绿
- Required behavior: 建表路径将首数据页初始化为合法零 slot 数据页（page type 0x03）；新建即含零行表的库 check exit 0 恰一行干净确认、dump 无链告警；初始化持久（close 重开两态一致）；打开路径零自愈写入
- Preserve: `check` 对 0x00 页 error 分级与 `validate_slots`/页 census 全部不动（损坏注入用例零回归）；`truncate_table` head 重置逻辑不动；写路径（含 0x00 页 init 兼容）不动；create 失败页生命周期语义不变
- Forbidden: 不修改 integrity.rs、slotted_page.rs（init 原语本体）、恢复面；不做 open 期自愈
- Test witness: `tests/check_command_test.rs` 新用例 RED——`create_db → CREATE TABLE（零行）→ check` 修复前 exit 1；存量夹具用例（lib 直建后不初始化模拟旧库——修复后该形态仅能经夹具构造）首次 INSERT/TRUNCATE 转绿断言
- GREEN condition: 全部新用例 GREEN + check_command_test 既有 8 用例零回归
- Verification: `cargo test --test check_command_test -- --test-threads=1`（0 failed）；定向 `--test-locked` 无；损坏注入用例原样通过
- Stop when: init 语句与 truncate 先例不一致导致 check 仍报错，或损坏注入用例行为变化（0x00 判定面被放松）——返回 Plan

### T2: 非 INTEGER PRIMARY KEY 计划期拒绝

- Requirement/Scenario: R4（S18 显式非 INTEGER PK 建表拒绝 / S19 存量表继续可用 / S21 隐式主键语义零变化——S21 场景同时锚定 import 通道）
- Depends on: None
- Targets: `src/parser/planner/ddl_dml.rs::build_create_table`（extract_primary_key 之后）
- Current behavior: `CREATE TABLE s (name TEXT PRIMARY KEY, ...)` 计划期接受、建表成功；重复插入零警告（keyless row）
- Required behavior: 显式 PK 列声明类型 ≠ ColumnType::Int 时计划期 `UnsupportedConstraint("NON-INTEGER PRIMARY KEY")` 拒绝（exit 3 零副作用）；列级与表级 PK 两形态同点覆盖；INTEGER PK 全形态保持可达
- Preserve: 隐式首列主键语义零变化（无 PK 声明 → 首列 fallback 不受检查波及）；import 自动建表通道零变化（INT 首列重复 DuplicateKey、FLOAT/TEXT 首列重复合法）；存量非 INT PK 表打开/读写/删除/dump 不触碰（拒绝面仅新 DDL 计划期）；错误面走既有 `UnsupportedConstraint` 通道（SQLSTATE 映射不变）
- Forbidden: 不修改 extract_primary_key 签名与归一逻辑；不触碰 keyless row 语义与 TableManager 建表执行器；不新增错误变体
- Test witness: `tests/planner_test.rs` 拒绝矩阵 RED（五类型 × 列级/表级）+ `tests/constraint_enforcement_test.rs` e2e RED（零副作用 + 隐式主键对照 + 存量夹具可用）
- GREEN condition: 矩阵与 e2e 全 GREEN + constraint_enforcement_test 既有 24 用例零回归 + import_input_surface_test 零回归（隐式 PK 通道不受波及的直接见证）
- Verification: `cargo test --test planner_test --test constraint_enforcement_test --test import_input_surface_test -- --test-threads=1`（0 failed）
- Stop when: 检查无法在 planner 处区分显式/隐式 PK（契约前提失效），或存量表可用性被破坏——返回 Plan

### T3: dump DDL 渲染适配——PK 渲染与实际强制语义一致

- Requirement/Scenario: R5（S20 非 INTEGER PK 存量表 dump 渲染与往返 / INTEGER PK 逐字节不变）
- Depends on: None（与 T2 语义配套，可独立实施验证）
- Targets: `src/cli/lifecycle.rs::create_table_sql`（:973 PK 渲染条件）
- Current behavior: PK 列无条件渲染 ` PRIMARY KEY`（以 `col.column_name == table.pk_column` 判定）
- Required behavior: 仅当 PK 列 `column_type == ColumnType::Int` 时渲染 ` PRIMARY KEY`；INTEGER PK 表输出逐字节不变；存量非 INT PK 表与隐式主键非 INT 首列表 DDL 不含 PRIMARY KEY，dump→new→restore 往返可达
- Preserve: NOT NULL/UNIQUE/DEFAULT 尾缀渲染不动；`rtsql schema` 渲染语义不在本任务调整范围（schema 复用 create_table_sql 时随 T3 自然对齐——若 schema 输出变化属预期后果，在 Response 记录）；quote_ident/转义/日期字面量渲染不动
- Forbidden: 不修改 dump 行数据导出与链预检；不修改 restore 逻辑
- Test witness: `tests/cli_test.rs` dump 用例 RED——存量 TEXT PK 表 dump 产物含 PRIMARY KEY（修复前）且 restore 往返在新拒绝面下失败
- GREEN condition: 新用例 GREEN（DDL 不含 PRIMARY KEY、往返行集等价）+ INTEGER PK 逐字节断言 + 既有 dump/restore 用例零回归
- Verification: `cargo test --test cli_test -- --test-threads=1`（0 failed）
- Stop when: 渲染条件引入 UNIQUE/NOT NULL 尾缀副作用，或 schema 输出变化超出「移除无效 PK 标记」范围——返回 Plan

**Invariants**

- `rtsql check` 的损坏检测强度只增不减：0x00 页 error 分级、validate_slots、census、溢出链与索引树扫描全部不动。
- 全部建表通道（CREATE TABLE/CTAS/import 自动建表/restore/ATTACH）收敛于 `create_table_with_constraints` 单点，不得在通道层分别打补丁。
- 拒绝面诚实化：新拒绝走既有 `UnsupportedConstraint` 通道点名能力；既有拒绝文案（NonAggregated 等）不因本 change 改写。
- 存量库兼容：本 Iteration 三个任务都不触碰存量表的打开/读写/删除路径；dump 语义变化仅限「移除从未被强制的声明」。
- 全量测试基线 1802 passed / 0 failed / 2 ignored 不回归（本 Iteration 净增用例）。

**Non-goals**

聚合表达式嵌入与 UPDATE 表达式（Iteration 001/002）；SKILL.md/README 同步（Iteration 002 T10）；非整数键真实强制；open 期自愈；`rtsql schema` 独立渲染逻辑；旧 dump 文件（含 TEXT PRIMARY KEY DDL）的 restore 兼容垫片——接受不兼容并由 T10 注记。

**Acceptance**

- S1–S5：零行表 check 干净（S1）；CTAS 空源/restore 空表/ATTACH 目标库建表 check 干净（S2）；close 重开仍干净（S3）；损坏注入 error 分级 exit 1 保持（S4）；存量 0x00 夹具表打开不自愈、首次写入/TRUNCATE 转绿（S5）。
- S18–S21：非 INTEGER 显式 PK 计划期拒绝零副作用（S18）；存量 TEXT PK 表打开/读写/dump 正常（S19）；INTEGER PK dump 逐字节不变、非 INT PK/隐式首列表 dump 往返可达（S20）；import 自动建表 INT 首列重复 fail-fast、FLOAT/TEXT 首列重复合法——逐字节与今日一致（S21）。
- 映射：R1→T1、R4→T2、R5→T3（tasks.md RTM）；场景→design D1/D2/D3→Targets→测试文件链路闭合。

**Verification**

- S1：`rtsql new` + 裸 CREATE TABLE + `rtsql check` → exit 0 恰一行 `check ok: ...`（自动，check_command_test）。
- S2：三通道建零行表 + check exit 0（自动，同文件参数化用例）。
- S3：建表 close 重开 + check（自动）。
- S4：既有损坏注入用例原样通过（自动，零修改断言）。
- S5：lib 夹具构造 0x00 head 表 → 打开无写入（check 维持既有 error 报告）→ INSERT 或 TRUNCATE → check exit 0（自动）。
- S18：planner_test 矩阵（五类型 × 列级/表级 → UnsupportedConstraint 文案含能力名）+ e2e exit 3 零副作用（表不存在）。
- S19：lib 夹具存量表 e2e（读写行集不变）。
- S20：cli_test dump 用例（INTEGER PK 逐字节 + 非 INT PK 往返）。
- S21：import_input_surface_test 既有用例零回归（INT 首列 DuplicateKey / FLOAT·TEXT 首列重复合法矩阵已在既有用例覆盖）。
- 收尾：fmt/clippy 干净 + 定向回归（check/约束/import/cli dump 套件）0 failed。

**Gate 2 Readiness**

PASS——2026-10-03 依据用户原始指令授权（原话：「剩下的那些功能问题，我们直接开始计划到一个change里面进行解决，开始吧」）；四项实质决策已由用户在 Gate 1 集中决策亲自选定（非 INTEGER PK=计划期拒绝+通道兼容、存量空表=最小修复、聚合嵌套=标准 SQL 面、UPDATE SET=值表达式全表面），本计划严格按其执行，无新增用户面向选择、无 Simplified requirement。交互式 Gate 2 确认未获应答，已知风险：用户尚未对详细计划做事后审计——实施未开始前该风险可逆，审计后可经 replan 修订。检查项：无 Missing、调查完整（探针闭合未知项）、设计闭合（D1–D6 无 TBD）、任务契约自包含（T1–T3）、验证直接（无身份型证据工程）、Persisted Evidence none。无 BLOCKED/WAIVED 项。

**Persisted Evidence**

- Mode: none

（全部验证可低成本重跑，Act Response 命令 + 决定性输出 + 退出码足以承载。）

**Risks and Notes**

- 存量空表在首次写入/TRUNCATE 前维持 check error 报告——集中决策 2 已接受的代价，非缺陷。
- 旧 dump 文件含 `TEXT PRIMARY KEY` DDL 在 T2 落地后 restore 失败（exit 3）——接受的不兼容，T10 注记；本 Iteration 不做兼容垫片。
- `rtsql schema` 对存量非 INT PK 表的输出随 T3 移除无效 PK 标记（create_table_sql 复用）——预期后果，Act 在 Response 记录实际输出形态。
- 非实质留 Act：init 语句的精确落点（allocate 与索引创建之间的任意位置）、测试夹具构造方式（lib 直建 catalog 行 vs 二进制夹具）。

## Act Response

- Status: reported

**Implemented**

- T1（R1）`src/storage/data/table_manager.rs::create_table_with_constraints`：`:424 allocate_page()` 之后、索引创建之前插入 head 页初始化（镜像 truncate `:1512-1517` 先例：`get_page` + `modify_page` + `SlottedPage::init(page, 0x03)` + drop；`?` 传播，init 置脏随 close 落盘）。单点覆盖全部建表通道。
- T2（R4）`src/parser/planner/ddl_dml.rs::build_create_table`：`extract_primary_key`（`:1094`）之后按列名查 `column_defs`，显式 PK 列 `data_type != ColumnType::Int` → 既有 `PlanError::UnsupportedConstraint("NON-INTEGER PRIMARY KEY")` 通道（无新变体、SQLSTATE 映射不变）。`if let Some(find)` 挂载：隐式主键（extract None）与未知列 PK（下游既有 ColumnNotFound 路径）均不波及。
- T3（R5）`src/cli/lifecycle.rs::create_table_sql`：`:972` PK 渲染条件加 `&& matches!(col.column_type, ColumnType::Int)`。INTEGER PK 输出逐字节不变（新用例断言）；`rtsql schema` 复用同一渲染函数，存量非 INT PK 表输出随之移除无效 PK 标记（Plan Context 已预判的预期后果）。
- 新测试：`tests/check_command_test.rs` +3（S1+S3 空表干净/重开、S2 通道矩阵〔CTAS 空源/import 仅表头/restore 空表 dump〕、S5 存量 0x00 夹具转绿特征化）；`tests/planner_test.rs` +1（五类型 × 列级/表级拒绝矩阵 + INTEGER 全形态 + 隐式主键对照）；`tests/constraint_enforcement_test.rs` +2（拒绝 e2e 零副作用矩阵 + INTEGER/隐式保持 + 存量 TEXT PK 表可用）；`tests/cli_test.rs` +1（dump PK 渲染 + 往返）。
- 语义校准（T2/T3 新拒绝面/渲染面的预期后果，5 处既有测试更新，均带 change 注记）：`src/cli/lifecycle.rs` 单测 `ddl_generator_quotes_and_orders_constraints`（夹具 pk 列 BOOL→INT，测试面为引号转义与约束序）；`tests/cli_test.rs::pk_bool_unique_dump_form_restores` → `legacy_bool_pk_dump_form_restores`（lib 直建存量夹具，断言 `("pk" BOOL NOT NULL, ...)` 往返）；`tests/ctas_test.rs` 2 处 schema 断言（非 INT 隐式首列去 PRIMARY KEY 标记）；`tests/datetime_type_test.rs::date_pk_equality_routing_fallback` 与 `tests/keyless_eq_routing_test.rs::text_primary_key_equality_reaches_keyless_rows`（lib 直建存量夹具，行为断言本身不变）。

**Deviations from Plan**

- S2 通道矩阵的 ATTACH 通道：附加库 DDL 计划期拒绝面下「ATTACH 目标库建表」不可经会话直达（附加库的表只能经其自身会话创建 = 同一 `create_table_with_constraints` 入口）。矩阵以 CTAS 空源 / restore 空表 dump / import 仅表头自动建表三通道落地，与 design D1「全部通道收敛于单点」调查结论一致（测试文件头注记说明）。
- S5 存量夹具用例为特征化见证：断言之上的「open 不自愈 / 首次写入/TRUNCATE 转绿」是集中决策 2 批准的既有行为，修复前即 GREEN（非新行为 RED）；其 RED 等价面由 S1/S2 的 exit 1 断言承载。
- 非实质：T1 init 语句落点取 allocate 与索引创建之间（契约允许区间）；测试夹具采用文件级整页归零构造存量 0x00 形态（plan 建议的 lib 直建形态在 T2 修复后已无法自然产生 0x00 head，文件级构造更直接）。

**Self-Review**

- 产品 diff 三处单点、与 Task Contract 逐一对应；无计划外修改；无新警告/死代码/身份型证据机制。
- 5 处既有测试语义校准均为 T2/T3 预期后果（新建 DDL 拒绝/渲染与强制语义一致），行为断言（路由回退、CTAS 类型推导、约束序、往返保真）本身未弱化。
- 遗留 Minor：无（无未解决的 Critical/Important）。

**Verification Evidence**

- T1 RED：`cargo test --test check_command_test -- --test-threads=1 empty_created` → FAILED，stdout 复现修复前形态 `free space offset 0 lies outside the data area [16, 4096]` + `page type 0x00, expected 0x03`，exit 1。
- T1 GREEN：`cargo test --test check_command_test -- --test-threads=1` → `test result: ok. 11 passed; 0 failed`（既有 8 + 新 3，含损坏注入用例零回归与 S5 转绿全链）。
- T2 RED：`cargo test --test planner_test -- --test-threads=1 non_integer_primary_key` → FAILED（五类型矩阵当前全部建出计划）；`cargo test --test constraint_enforcement_test -- --test-threads=1 non_integer_pk_rejected` → FAILED，拒绝对象返回 `AffectedRows { count: 0 }`。
- T2 GREEN：`cargo test --test planner_test --test constraint_enforcement_test --test import_input_surface_test -- --test-threads=1` → `26 passed` / `12 passed` / `97 passed`，三目标 0 failed（隐式主键与 import 通道零波及直接见证）。
- T3 RED：`cargo test --test cli_test -- --test-threads=1 dump_pk_rendering` → FAILED，修复前 dump 渲染 `("name" STRING PRIMARY KEY, ...)`。
- T3 GREEN：`cargo test --test cli_test -- --test-threads=1` → `test result: ok. 118 passed; 0 failed; 2 ignored`。
- Cycle 收尾（M19 全量分批，`--lib`/`--bins`/113 个集成目标按名字典序，串行 `--test-threads=1` 逐目标独立运行，全部退出码 0）：`--lib` `394 passed; 0 failed`；`--bins` ok；集成 113 目标全 ok（净增 +7：check_command +3 / planner +1 / constraint_enforcement +2 / cli +1；ctas/datetime/keyless/cli 校准零计数变化）。全量 **1809 tests / 0 failures / 2 ignored**（1802 基线 + 7）。
- `cargo fmt --check` 干净；`cargo clippy --all-targets` 0 代码警告/错误；`openspec validate --changes 2026-10-03-practical-core-gaps` → `✓ 1 passed, 0 failed`。

**Persisted Evidence**

- None required（plan 模式 none；全部验证可低成本重跑，Act Response 命令 + 输出足以承载）。

**Experience Candidates**

- Issue 候选（范围外实质缺陷候选，证据见上）：lib `Database` 句柄 drop 而不 `close()` 时，未 checkpoint 的 catalog 脏页丢失，重开恢复以 `WAL redo failed: table 't' lookup failed during redo: table not found: t` 报错——错误面把「DDL 从未持久化」诊断为「redo 目标表缺失」，排障指向性弱（本 Cycle T1 S5 夹具首版实测触发；既有夹具纪律 `checkpoint+wal shutdown+drop` 可规避，但与「close 即落盘」心智有缝）。
- Runbook 候选：None（全部验证为既有 cargo 命令常轨）。

**Remaining Issues**

- 无未解决问题。旧 dump 文件含 `TEXT PRIMARY KEY` DDL 在 T2 落地后 restore 失败（exit 3）为 design D3 已批准的不兼容，注记属 Iteration 002 T10 文档面。

## Plan Review

- Review Result: accepted

**Findings**

1. **（Plan 遗漏，非阻塞，Review 已修正 spec）** S2 场景的「ATTACH 目标库建表」通道在既有附加库 DDL 计划期拒绝面（`cross-db-namespace-query` R3，先于本 change 存在）下不可经 SQL 会话构造——Plan 起草时未核对该场景可构造性。Act 以 import 仅表头自动建表通道等效替换并在测试文件头注记论证（全部通道收敛于 `create_table_with_constraints` 单点，机制级覆盖完整）。Review 已把 delta spec 的 S2 场景 GIVEN 修正为实测通道集（CTAS 空源 / import 仅表头 / restore 空表 dump），Requirement 通道清单保持原样（机制为真）。
2. **（Minor，Review 现场修复，非 Act 工作面）** `rtsql-docs/SKILL.md` 能力表中 Aggregate 与 Write 行之间被本会话更早的现状边界文档编辑引入一个空行，markdown 表格被截断——Review 审读 SKILL.md diff 时发现并当场修复（该编辑先于 change 创建，不属于 T1–T3 变更面）。
3. **（Minor 观察，不返工）** S5 夹具在 seed 一行后整页归零 head 页——该形态强于自然存量形态（真实存量 0x00 head 仅在零行表出现；夹具状态额外销毁了 seed 行并遗留 key 1 的悬挂 PK 索引条目）。测试断言仅覆盖结构面（error 保持 → 写入/TRUNCATE 转绿），未做数据保全声明，与 S5 意图一致；仅作记录。
4. **（Issue 候选，范围外，只报告不落账）** Act 报告：lib `Database` 句柄 drop 而不 `close()` 时未 checkpoint 的 catalog 脏页丢失，重开恢复以 `WAL redo failed: table 't' lookup failed during redo: table not found: t` 报错——错误面把「DDL 从未持久化」诊断为「redo 目标表缺失」，排障指向性弱。是否立 Issue 由用户裁定（Recorder 职责）。

偏差分类：上述 1 为 Plan 遗漏（非阻塞）；其余无 Act 偏离、无基线变化。Act Response 声明与代码逐项核对一致（三处产品 diff 与契约吻合、5 处校准均带 change 注记且行为断言未弱化、+7 净增与 1809 全量自洽、无计划外修改、无身份型证据机制）。

**Acceptance Gaps**

无——S1–S5、S18–S21 全部有 GREEN 见证（Act Verification Evidence，覆盖面未失效，采信；RED→GREEN 链完整：T1 空表 exit 1 → exit 0 单行、T2 五类型矩阵建出计划 → 点名拒绝、T3 dump 携无效 PK 标记 → 语义一致渲染）。fmt/clippy 干净、全量 1809/0/2、`openspec validate` 通过。

**Evidence**

- 独立审读完整 diff：`src/storage/data/table_manager.rs`（init 块镜像 `:1512-1517` truncate 先例，落点 allocate 之后索引创建之前）、`src/parser/planner/ddl_dml.rs`（`if let Some` 双层挂载——隐式 PK 与未知列 PK 均不波及，`UnsupportedConstraint` 既有通道）、`src/cli/lifecycle.rs`（单条件 + 单元夹具 BOOL→INT 校准）、`tests/`{check_command,planner,constraint_enforcement,cli,ctas,datetime_type,keyless_eq_routing}`_test.rs`（新增 7 用例 + 5 处校准全部带 change 注记）。
- Act 的测试运行结论采信（来源：Act Response Verification Evidence，2026-10-03；无异常信号触发定向重跑——结论与 diff 一致、被覆盖表面在结论产生后未变化）。

**Follow-up Decision**

接受：Acceptance 全部满足，无阻塞 finding。Finding 1 已由 Review 修正 spec（无需 Act 返工）；Finding 2 已现场修复；Finding 3 仅记录；Finding 4 作为 Issue 候选留用户裁定。Iteration 000 完成。

**Iteration Plan Update**

（无——Map 不变）

**Next Cycle**

None

**Next Iteration**

`iterations/001-aggregate-embedding/000-initial.md`（已按 Map 展开，Plan Context 就绪）
