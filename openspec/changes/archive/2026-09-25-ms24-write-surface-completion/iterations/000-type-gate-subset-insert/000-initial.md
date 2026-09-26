# Iteration 000 / Cycle 000: 写入类型门与子集 INSERT/DEFAULT

## Plan Context

- Status: ready
- Iteration: 000-type-gate-subset-insert
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 1.7
- Depends on: None
- Stable baseline: 类型门全写面生效（含 Float 升格、既有错误面优先级不变、dump/restore/import 零误报）；子集 INSERT 端到端 + DEFAULT 持久化/应用/渲染闭环；全量零回归（校准逐条记录）
- Verification boundary: write_type_conformance_test + subset_insert_test + planner_test 矩阵 + catalog 单测 + cli_test 往返与渲染 + `cargo test` 全绿
- Diagnostic boundary: `src/executor/{insert,update}.rs`、`src/storage/{error.rs,catalog.rs,data/table_manager.rs}`、`src/parser/planner/{mod,ddl_dml}.rs`、`src/cli/lifecycle.rs` 与本 Cycle
- Deferred tasks: 2.1-2.6（Iteration 001 UPSERT/REPLACE）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: change 全部 R1/R2/R5 requirement 与 design D1/D2/D3/D7；MS23 既有约束执行语义（NOT NULL/UNIQUE 错误面与优先级）
- Excluded scope: UPSERT/REPLACE（Iteration 001）；多列 UPDATE SET；表达式赋值；键编码扩展（I024）

**Objective**

INSERT/UPDATE 在任何写入前校验值变体与列声明类型一致（跨类型点名拒绝、FLOAT 列整数值升格、既有错误面优先级与文本不变），全写面通道（含 dump/restore/import）零误报；显式列清单 INSERT 接受表列子集，省略列取声明 DEFAULT（catalog 持久化、跨重启生效、dump/schema 渲染保真）或 NULL，`DEFAULT` 关键字等价省略；全量测试零回归。

**Background**

MS24 统一执行序第二位。三类缺口：ISS04 非键列写入类型校验缺失（R29，静默写坏）；子集 INSERT 被 `map_insert_values`「恰为全列排列」拒绝；DEFAULT 解析进 `ColumnSchema` 后被建表路径整体丢弃（不持久化、不消费、dump 不渲染）。用户已批准需求基线（Gate 1，2026-09-25）与语义裁定（Float 升格等）。

**Investigation Facts**

- Current Baseline: master 工作树 @ MS23 收尾（2026-09-25，1152 tests / 0 failures / 2 ignored 全量新鲜；本 Iteration 涉及表面自该结论零变化——git 现状确认 MS23 改动即当前工作树）。基线结论直接采信（来源 SNAPSHOT 同步状态 current + MS23 收尾验证记录）。
- Current-State Evidence（本会话独立追读）：
  - INSERT 计划链：`build_insert`（`src/parser/planner/ddl_dml.rs:114`）→ `extract_insert_values`（:219，值臂：`Expr::Value`/NULL Identifier/负数折叠/TypedString 日期族；其余 `UnsupportedValue`）→ `map_insert_values`（:151，:195-201 全列排列强校验、:206 行长校验、:203-215 重排）。`extract_insert_values` 仅 `build_insert` 单一调用点。
  - VALUES 中 `DEFAULT` 关键字：sqlparser 0.44 `Expr` 枚举无 `Default` 变体（ast/mod.rs 全枚举核对），`parse_values` → `parse_expr` 无 DEFAULT 特判 → 落 `Expr::Identifier("DEFAULT")` → 现被 :240-245 臂 `UnsupportedValue` 拒绝。
  - InsertExecutor（`src/executor/insert.rs:100-248`）逐行序列：coerce（:112）→ NOT NULL（:121）→ PK 键位门（:135）→ PK 重复预检（:151）→ UNIQUE 预检 + F1 守卫（:169）→ serialize（:186）→ 写页/WAL/版本/PK 条目/唯一条目（:196-242）。
  - UpdateExecutor（`src/executor/update.rs:76-314`）序：Step1 索引定位（:84，KeyNotFound）→ NOT NULL（:92）→ PK 门 + rekey 预检（:111-139）→ coerce（:144）→ 读旧 tuple（:156）→ 唯一旧值快照（:165）→ 改值（:179）→ UNIQUE 碰撞预检（:190）→ serialize（:209）→ 写/版本/索引维护（:217-311）。
  - DEFAULT 丢弃链：`extract_default_value`（ddl_dml.rs:412）→ `ColumnSchema.default_value`（`src/executor/plan.rs:227`）→ `CreateTableExecutor` 转 4 元组丢弃 default（`src/executor/create_table.rs` to_schema_column→to_tuple）→ `create_table_with_constraints`（`src/storage/data/table_manager.rs:307`）入参 `(String, ColumnType, bool, bool)`；`CatalogColumnRow`（`src/storage/catalog.rs:71-78`）无 default 字段；`TableMeta`（`table_manager.rs:50`）无 defaults。
  - catalog 列行序列化（`catalog.rs:738-855`）：`serialize_catalog_column_row` 固定布局至 `u8 unique` 结束；`deserialize_catalog_column_row` 顺序读、不拒绝尾随字节（追加读可行，MS23 表行 unique_roots 与 checkpoint 24B 位点先例）。
  - PlanBuilder（`src/parser/planner/mod.rs:99-122`）：`tables`/`primary_keys`/`primary_key_types` 三通道；注册点 `pipeline.rs::register_table`（:1084-1119）从 `TableMeta` 取列名与 PK 类型（`set_pk_column_type` :146 加性先例）。DML 不进 plan cache（`is_cacheable` 仅 SELECT，pipeline.rs:1123）。
  - dump/schema 渲染：`create_table_sql`（`src/cli/lifecycle.rs:577`）消费 `CatalogRow`+`[CatalogColumnRow]`，注释明写「DEFAULT 不在 catalog 持久化面，不输出」；日期族 typed 字面量 helper `typed_datetime_literal`（:288）与 `sql_literal`（:266）可复用。
  - import 值类型：`csv_value`（lifecycle.rs:541）按列类型产出 Int/Float/Bool/NULL/String；日期族 String 透传经 coerce 落类型——类型门在 coerce 后不误报。dump 按列类型生成类型化字面量（sql_literal/typed_datetime_literal）——自洽不误报。
  - Value 类型面（`src/executor/value.rs:50-64`）：Int/String/Null/Float/Bool/Date/Timestamp 七变体；`to_key` 仅 Int（:92）。
  - StorageError 点名先例：`KeyTypeMismatch { column, expected, actual }`（`src/storage/error.rs:55`）、`NullConstraintViolation { column }`（:62）。
  - 受影响既有测试：`tests/insert_column_list_test.rs`（MS16，7 用例；`partial_list_rejected_at_plan_time_no_panic` :126 按新语义校准）；catalog 单测 `sample_col` 夹具（catalog.rs:886）需补字段；`create_table_with_constraints` 4 元组构造点波及面（create_table.rs + 测试夹具）。
- Code and Critical Path: 写路径执行器（insert.rs/update.rs）持有 `Arc<TableMeta>`（类型门数据源）；列元数据三段通道 catalog 列行（持久）→ TableMeta.defaults（内存）→ PlanBuilder.table_defaults（计划期填充数据源）；dump 渲染自 catalog 列行读出。错误面经既有 Sql 失败路径渲染（exit 3，无专用 CLI 臂）。

**Implementation Guidance**

顺序：1.1 错误面 → 1.4 持久化通道（表结构先行，1.5 依赖其内存承载）→ 1.5 计划期通道与填充 → 1.6 端到端与渲染 → 1.2/1.3 类型门（可与 1.4-1.6 并行，同批全量回归收口于 1.7）。类型门先建 RED 矩阵再实现；升级改写就地发生（升格值进入后续 serialize）。关键技术细节：DEFAULT payload 定宽编码复用 TAG_* 值域；`map_insert_values` 的 `InsertValue` 中间形态仅存在于 build_insert 内部（外部签名不变）；既有错误面优先级通过「门追加在既有校验之后」结构性保证，无需重排。

**Behavioral Change**

- 当前：非键列类型不匹配值静默落库（String/Float 写入 INT 列）；子集列清单计划期拒绝；DEFAULT 声明被丢弃（不持久化不渲染不生效）；`DEFAULT` 关键字值 `UnsupportedValue` 拒绝。
- 目标：跨类型写入以 `ColumnTypeMismatch { column, expected, actual }` 点名拒绝（零副作用），FLOAT 列 Int 升格，NULL 豁免，日期族 coerce 后一致；子集清单合法且省略列取 DEFAULT/NULL（NOT NULL 无 DEFAULT 列由既有门点名拒绝）；DEFAULT 全闭环（建表持久化、跨重启生效、dump/schema 渲染、restore 保真）；`DEFAULT` 关键字等价省略。
- 接口/错误语义：`create_table_with_constraints` 签名 4→5 元组（pub(crate) 内部 API，调用点 create_table 壳与 CreateTableExecutor）；新 StorageError 变体；PlanBuilder 加性字段（`new()` 构造、pub(crate) 可见性，既有 API 签名不变）。

**Task Contracts**

### 1.1: ColumnTypeMismatch 错误面就位

- Requirement/Scenario: R2 全场景（错误载体）
- Depends on: None
- Targets: `src/storage/error.rs`（新变体）
- Current behavior: 无一般类型不匹配错误变体（仅 `KeyTypeMismatch`）
- Required behavior: `StorageError::ColumnTypeMismatch { column: String, expected: String, actual: String }` 存在且 Display 点名三要素（镜像 KeyTypeMismatch 文案形状）
- Required changes: 新变体 + Display
- Preserve: 既有变体与文案逐字节不变
- Forbidden: 不改任何既有错误文案；不加专用 CLI 臂
- Test witness: `write_type_conformance_test` 首个拒绝用例 RED（错误不存在编译失败即 RED 形态；若先写变体后写用例则以用例断言文本驱动）
- GREEN condition: 用例断言通过
- Verification: `cargo test --test write_type_conformance_test`，退出码 0
- Stop when: 错误面需要文案/变体形状与本契约冲突的新决定

### 1.2: INSERT 一般类型门

- Requirement/Scenario: R2 S1-S6
- Depends on: 1.1
- Targets: `src/executor/insert.rs`（next 内既有 UNIQUE 预检后、serialize 前插入逐列校验；升格就地改写行值）
- Current behavior: 非键列值变体不与 schema 交叉校验（String 落 INT 列静默持久化）
- Required behavior: 逐列校验——`Null` 豁免；日期族列经 coerce 后一致（Date 值入 Date 列）；FLOAT 列收 `Int(n)` → 行值改写 `Float(n as f64)` 通过；STRING 列收 `String` / INT 列收 `Int` / BOOL 列收 `Bool` 一致通过；其余组合 → `ColumnTypeMismatch`，零副作用（未触任何写入）
- Required changes: 校验循环 + 升格改写；错误触发优先级位于既有 NOT NULL/PK/UNIQUE 门之后（其文本与行为逐字节不变）
- Preserve: 既有全部门语义与顺序；`serialize_tuple`/恢复路径不动；InsertExecutor 其余行为不变
- Forbidden: 不改计划期解析面（类型门不在 planner）；不引入隐式转换（仅 Float 列 Int 升格一种）；不给 key/unique 列改错误文本
- Test witness: `tests/write_type_conformance_test.rs`（新建，lib API helpers 模式）：非键 INT 收 String 拒绝点名 / 非键 STRING 收 Int 拒绝 / FLOAT 收 Int 升格读回 / 日期族 String 成功与非法日期仍拒 / NULL 豁免（含 NOT NULL 既有拒绝不变）/ 既有键列文本优先（Int PK 收 String 仍 `KeyTypeMismatch`）/ 零副作用（拒绝后行数不变）/ dump→restore 与 import 通道不误报
- GREEN condition: 矩阵全绿 + 既有约束/键位矩阵零变化
- Verification: `cargo test --test write_type_conformance_test` + `cargo test --test constraint_enforcement_test --test key_type_conformance_test`，退出码 0
- Stop when: 升格或优先级与本契约冲突的实质行为差异

### 1.3: UPDATE 一般类型门

- Requirement/Scenario: R2 S1/S2/S5
- Depends on: 1.1
- Targets: `src/executor/update.rs`（UNIQUE 碰撞预检后、serialize 前对赋值列校验）
- Current behavior: SET 值变体不与列声明类型交叉校验（仅 PK 门/唯一守卫/日期 coerce）
- Required behavior: 赋值列逐条同 1.2 规则（含升格改写 `new_value`）；既有错误优先级不变
- Preserve: Step 顺序与既有门文本；单列 SET 限制不变
- Forbidden: 不扩多列 SET；不动 rekey/唯一预检逻辑
- Test witness: write_type_conformance_test UPDATE 臂（`UPDATE t SET n='abc'` 拒绝点名 / 升格 / 既有 PK 门优先 / 零副作用原值保持）
- GREEN condition: 矩阵全绿
- Verification: `cargo test --test write_type_conformance_test`，退出码 0
- Stop when: 同 1.2

### 1.4: DEFAULT 持久化通道

- Requirement/Scenario: R1 S5/S6（持久化底座）
- Depends on: None
- Targets: `src/storage/catalog.rs`（`CatalogColumnRow.default_value` + serialize 尾部追加 `u8 has_default | [u8 tag | payload]` + deserialize 可选读）；`src/storage/data/table_manager.rs`（`TableMeta.defaults`；`create_table_with_constraints` 4→5 元组；`create_table` 壳补 `None`；`open_or_init` 读回）；`src/executor/create_table.rs`（透传 `schema_col.default_value`）
- Current behavior: DEFAULT 解析进 ColumnSchema 后被 CreateTableExecutor 丢弃；catalog 无字段；重开即失
- Required behavior: 建表 DEFAULT 字面量（`extract_default_value` 支持的字面量形态）持久化；旧格式列行（无尾随块）读回 `None`；`DEFAULT NULL` 保真往返；重开后 `TableMeta.defaults` 与建表一致
- Required changes: 结构体 + 编解码 + 三构造点接线；TAG_* 值域复用（Int 8B/Float 8B/Bool 1B/Date 4B/Timestamp 8B/String u16len+bytes/Null 0B）
- Preserve: 既有列行布局前缀逐字节不变（兼容读）；`serialize_catalog_column_row` 对无 default 行输出与旧格式逐字节一致
- Forbidden: 不动表行（__tables）格式；不做值类型扩展（DEFAULT 仅字面量七变体）
- Test witness: `src/storage/catalog.rs` `#[cfg(test)]`——新格式往返（七变体 + DEFAULT NULL）/ 旧格式字节流（无尾块）兼容读 / `create_table_with_constraints` 后 reopen 断言 defaults（table_manager 单测或集成夹具）；RED 先行（字段不存在编译 RED，逐用例驱动）
- GREEN condition: 单测全绿
- Verification: `cargo test --lib storage::catalog`（模块过滤）+ `cargo test --test constraint_enforcement_test`（构造点回归），退出码 0
- Stop when: 兼容读发现旧格式反序列化拒绝尾随字节（与调查矛盾，需重新设计）

### 1.5: PlanBuilder defaults 通道与子集填充

- Requirement/Scenario: R1 S1-S4/S7/S8
- Depends on: 1.4
- Targets: `src/parser/planner/mod.rs`（`table_defaults` 字段 + `set_table_defaults` + `new()` 初始化）；`src/pipeline.rs::register_table`（从 `TableMeta.defaults` 传递）；`src/parser/planner/ddl_dml.rs`（`extract_insert_values` 行值 `InsertValue` 化 + DEFAULT 关键字臂；`map_insert_values` 子集放宽 + 填充）
- Current behavior: 列清单必须恰为全列排列（子集 `insert_count_error` 拒绝）；`DEFAULT` 关键字 `UnsupportedValue`
- Required behavior: 子集清单（互异已知列）合法；行长校验对清单长度；重排后省略位与 DEFAULT 位填充 `table_defaults`（有则克隆、无则 `Value::Null`）；全列清单与无清单行为逐字节不变；未知列 `ColumnNotFound`、重复列、无清单行长度拒绝不变
- Required changes: 中间形态 `InsertValue = Val(Value) | DefaultKeyword` 仅 build_insert 内部；输出恒全宽
- Preserve: MS16 既有拒绝文案与乱序映射语义；`build_insert` 对外签名可扩参但 DML 不进 plan cache 无缓存键影响
- Forbidden: 不在计划期做 NOT NULL 拒绝（执行器单点强制）；不改 `extract_insert_values` 非 insert 消费面（无其他消费点）
- Test witness: `tests/planner_test.rs` 增量——子集 plan 构造成功（节点值全宽）/ 未知列 / 重复列 / DEFAULT 关键字臂（`INSERT INTO t (id, name) VALUES (1, DEFAULT)` → name 位为声明 DEFAULT 值）；先行 RED
- GREEN condition: planner 增量全绿
- Verification: `cargo test --test planner_test`，退出码 0
- Stop when: sqlparser 实测 `VALUES (DEFAULT)` 解析形态与 Identifier 假设不符（DESIGN-INVALID 返回 Plan）

### 1.6: 子集 INSERT 端到端与 dump/schema DEFAULT 渲染

- Requirement/Scenario: R1 S1-S6
- Depends on: 1.4, 1.5
- Targets: 新 `tests/subset_insert_test.rs`；`src/cli/lifecycle.rs::create_table_sql`（DEFAULT 渲染：日期族 `typed_datetime_literal` 包裹、其余 `sql_literal`、`DEFAULT NULL` 字面渲染）；`tests/cli_test.rs` 增量
- Current behavior: 子集 INSERT 全链路不可达；dump DDL 无 DEFAULT 子句
- Required behavior: 子集 INSERT 端到端（DEFAULT 取值/NULL/NOT NULL 拒绝点名/全列零回归）；重开后子集 INSERT 取 DEFAULT；dump→restore 往返后 schema 文本含 DEFAULT 且行为一致；`rtsql schema` 含 DEFAULT
- Required changes: 测试与渲染函数（渲染为纯函数扩展，`CatalogColumnRow.default_value` 数据源已由 1.4 就位）
- Preserve: dump/restore 既有往返用例零变化（无 DEFAULT 表逐字节一致）
- Forbidden: 不改 dump 数据 INSERT 生成；不做 DEFAULT 表达式（仅字面量）
- Test witness: subset_insert_test（lib API e2e）+ cli_test（dump 含 DEFAULT / restore 往返 / schema 渲染）；先行 RED
- GREEN condition: 全绿
- Verification: `cargo test --test subset_insert_test --test cli_test`，退出码 0
- Stop when: 渲染往返发现字面量保真缺口（如引号转义）需新决定

### 1.7: 校准与全量回归收口

- Requirement/Scenario: R5 S1/S2
- Depends on: 1.2, 1.3, 1.5, 1.6
- Targets: `tests/insert_column_list_test.rs`（:126 用例按子集新语义重写 + 防回归意图承接）；`create_table_with_constraints` 5 元组波及的测试构造点
- Current behavior: 子集拒绝断言与新语义冲突；部分测试夹具 4 元组构造
- Required behavior: 校准后全量 `cargo test` 全绿；每条校准在 Act Response 记录理由
- Preserve: 各校准用例原防回归意图（未知列/重复列拒绝、无 panic、映射正确性）
- Forbidden: 不为通过而弱化断言；不删防回归意图
- Test witness: `cargo test` 全量输出（0 failures）
- GREEN condition: 1152 + 净增用例全绿（ignored 计数不增）
- Verification: `cargo test`，退出码 0
- Stop when: 出现无法归因本 change 的回归（BASELINE-CHANGED 返回 Plan）

**Invariants**

- 既有 PK 键位门（`KeyTypeMismatch`）、INT 唯一列 F1 守卫、NOT NULL 门（`NullConstraintViolation`）的触发优先级与错误文本逐字节不变（一般类型门在其后）。
- InsertExecutor/UpdateExecutor 既有写入序列、MVCC/WAL/版本链语义不变；恢复路径（`rebuild_pk_indexes` 等）不动。
- catalog 列行既有布局前缀逐字节不变（向后兼容读）；表行格式不动。
- 全列清单与无清单 INSERT 的计划与执行行为逐字节不变；MS16 未知列/重复列/行长度拒绝保持。
- DML 不进 plan cache；`PlanBuilder` 公共 API 既有签名不变（加性）。
- dump/restore/import 通道行为除新增 DEFAULT 渲染外不变；import 不被类型门误报。
- 不建身份型证据工程；验证用原生 `cargo test` 退出码与输出。

**Non-goals**

- UPSERT/REPLACE（Iteration 001）；DO UPDATE 赋值面；多列 UPDATE SET 语句；DEFAULT 表达式（非字面量）；键编码扩展（I024）；计划期 NOT NULL 重复拒绝；非 INT PK 键控语义变化（一般类型门仅覆盖其写入值类型面）。

**Acceptance**

R1：子集 INSERT 全场景（S1-S8）经 `subset_insert_test` + planner_test + cli_test 见证；DEFAULT 持久化经 catalog 单测 + 重启用例 + dump 往返见证。R2：类型门矩阵（S1-S6）经 `write_type_conformance_test` 见证，既有错误面优先级经既有矩阵（constraint/key_type/planner）零变化见证。R5：`cargo test` 全绿（1.7）。映射见 change tasks.md RTM。

**Verification**

- R2-S1/S2/S3/S4：`cargo test --test write_type_conformance_test`（拒绝点名三要素 / 升格读回 / coerce 通道）退出码 0。
- R2-S5：既有 `key_type_conformance_test` + `constraint_enforcement_test` 全绿（文本零校准目标，若校准逐条记 Response 并说明）。
- R1-S1-S4/S7/S8：`cargo test --test subset_insert_test --test planner_test` 退出码 0。
- R1-S5/S6：重启用例 + `cargo test --test cli_test`（dump/restore/schema 渲染）退出码 0。
- R5：`cargo test` 全量 0 failures（计数与基线差 = 净增用例）。
- 全部为原生 cargo 输出与退出码直接判定，无封装判定层。

**Gate 2 Readiness**

- 无 Missing requirement：RTM 五行全 Covered（tasks.md）。PASS
- 无未批准 Simplified：无 Simplified 项；DO UPDATE WHERE/INSERT OR 缺口为 Iteration 001 拒绝面场景（R4 覆盖），Gate 1 已向用户明示并获批。PASS
- 调查完整：Current-State Evidence 全部来自本会话对当前工作树的独立追读（位点行号在案）；基线 1152 tests 采信 MS23 收尾结论（表面零变化经 git 现状核对）。PASS
- 设计闭合：D1-D8 行为/接口/错误/兼容语义完整；Float 升格/DEFAULT NULL 保真/优先级保持等边缘均有裁定与理由；无契约语义 TBD。PASS
- 任务可执行：1.1-1.7 每个 Task Contract 有 Targets/Current/Required/Preserve/Forbidden/witness/GREEN/Verification/Stop。PASS
- 分轮合理：000/001 依赖单向，平衡审计在 tasks.md。PASS
- 追踪完整：RTM R↔S↔D↔Task↔Code↔Test 链路闭合。PASS
- 验证充分：全部 scenario（含 sad path/edge）映射到测试命令；最简直接判定；无身份型证据工程/判定层。PASS
- Act 自包含：本 Cycle 含全部实现事实与契约，Act 无需回读 Assistant/Explorer/前序产物。PASS
- 非实质未知项入 Risks：DEFAULT 关键字解析形态（低风险，1.5 witness 确认）、5 元组波及面（机械适配）。不迫使 Act 决定契约语义。PASS
- Persisted Evidence：none（见下）。PASS

**Persisted Evidence**

- Mode: none

`none` 依据：全部验证为 `cargo test` 原生命令，输出与退出码写入 Act Response 即可承载（公共规则 › 验证）；无不可低成本重跑的现场。

- Budget: 按 Evidence 预算执行（公共规则 › 验证）。

**Risks and Notes**

- `VALUES (DEFAULT)` 的 sqlparser 解析形态按无 `Expr::Default` 变体 + Identifier 落臂推断；1.5 测试见证首验，若解析层另有形态（如 token 拒绝）属 DESIGN-INVALID 返回 Plan——非实质，不阻塞设计闭合（行为证据驱动的两分支均有明确归属）。
- `create_table_with_constraints` 5 元组签名变化的波及点（CreateTableExecutor + 测试夹具）为机械适配，允许 Act 顺带处理并记录。
- 既有测试若存在「向 String 列写 Int」类字面量（非 dump/import 通道），将按新契约校准并在 Act Response 记录——门的行为即目标行为，校准非回归。
- Iteration 001（UPSERT）的 Cycle 文件待本 Iteration accepted 后展开；tasks.md 2.x 契约框架已定稿。

## Act Response

- Status: reported

**Implemented**

按 Plan Context 全部 7 个 task 完成（change tasks.md 1.1-1.7 已勾选）：

- 1.1：`StorageError::ColumnTypeMismatch { column, expected, actual }`（镜像 `KeyTypeMismatch` thiserror 点名模式，Display「column 'x' expects INT, got String」）。
- 1.2：INSERT 一般类型门。升格趟（coerce 后、静默）+ 拒绝趟（UNIQUE 预检后、serialize 前）两趟结构，见 Deviations 1。
- 1.3：UPDATE 一般类型门——UNIQUE 碰撞预检后、serialize 前对赋值列（单列）执行；FLOAT 列 Int 值就地升格改写 `values[col_idx]` 与 `new_value`（Step 7 键位维护派生自升格后值）。
- 1.4：DEFAULT 持久化——`CatalogColumnRow.default_value: Option<Value>` + 列行尾段（有 DEFAULT 时追加 `u8 has_default=1 | tuple TAG_* tag | 定宽 payload`；无 DEFAULT 不追加任何字节，输出与旧格式逐字节一致；`has_default=0` 形态防御读为 None）；`TableMeta.defaults`（create 5 元组入参 / open_or_init 读回 / 两个 recovery swap 继承）；TAG_* 常量自 tuple.rs `pub(crate)` 导出单一来源。
- 1.5：`PlanBuilder.table_defaults` + `set_table_defaults`（镜像 `set_pk_column_type`）；`pipeline.rs::register_table` 接线；`extract_insert_values` 行值 `InsertValue = Val | DefaultKeyword` 化（`VALUES` 中 DEFAULT 关键字实测落 `Expr::Identifier("DEFAULT")`，与 Plan 调查一致）；`map_insert_values` 放宽为互异已知列子集 + 省略位/DEFAULT 位按声明填充（无则 NULL），输出恒全宽；全列与无清单路径行为不变。
- 1.6：`create_table_sql` DEFAULT 渲染（日期族经 `value_to_json`→`typed_datetime_literal` 包裹、其余 `sql_literal`、DEFAULT NULL 字面渲染）；subset_insert_test 6 用例 + cli_test S6 往返用例。
- 1.7：`insert_column_list_test::partial_list_rejected_at_plan_time_no_panic` 按子集新语义重写为 `partial_list_inserts_omitted_columns_no_panic`（原防回归意图由同文件未知列/重复列/无清单行长度用例 + planner_test 子集矩阵承接）；5 元组波及点（table_manager 测试夹具 6 处、create_table.rs、lifecycle 测试夹具）机械适配；全量 `cargo test` 全绿。

**Changed Files and Symbols**

产品代码（10 文件）：

- `src/storage/error.rs`：+`ColumnTypeMismatch`
- `src/storage/page_format/tuple.rs`：TAG_* 七常量 `pub(crate)`
- `src/storage/page_format/mod.rs`：TAG_* `pub(crate) use` 再导出
- `src/storage/catalog.rs`：`CatalogColumnRow.default_value`、`serialize_catalog_column_row` 尾段、`deserialize_catalog_column_row` 可选读 + 3 个新单测
- `src/storage/data/table_manager.rs`：`TableMeta.defaults`、`create_table_with_constraints` 4→5 元组、`create_table` 壳、`open_or_init` 读回、`replace_index_manager`/`replace_recovery_indexes` 继承 + 1 个新单测（夹具适配）
- `src/executor/create_table.rs`：5 元组透传（`schema_col.default_value`）
- `src/executor/insert.rs`：+`column_type_name`、升格趟、拒绝趟
- `src/executor/update.rs`：+`column_type_name`、赋值列类型门（含升格）
- `src/parser/planner/mod.rs`：`PlanBuilder.table_defaults` + `set_table_defaults` + `new()` 初始化
- `src/parser/planner/ddl_dml.rs`：`InsertValue`（模块私有）、`extract_insert_values`（收窄为私有 fn，见 Deviations 3）、`map_insert_values` 子集放宽与填充
- `src/pipeline.rs`：`register_table` 传递 defaults
- `src/cli/lifecycle.rs`：`create_table_sql` DEFAULT 渲染 + 1 个新单测（夹具补字段）

测试（5 文件改动 + 2 新建）：`tests/write_type_conformance_test.rs`（新，13 用例）、`tests/subset_insert_test.rs`（新，6 用例）、`tests/planner_test.rs`（+5）、`tests/cli_test.rs`（+1，S6 dump/restore/schema 往返）、`tests/insert_column_list_test.rs`（1 用例校准重写）。

**Deviations from Plan**

1. **INSERT 类型门拆两趟**（契约原文「UNIQUE 预检之后追加逐列一般门」）：FLOAT 升格若在契约位置（键位派生之后）发生，Float 键列收 Int 值时索引条目将派生自升格前 Int 值，与恢复期自 tuple 重建键位（Float → 无键）两态不一致。拆为「升格趟（coerce 后，静默无错误面）+ 拒绝趟（契约位置）」：既有 NOT NULL / PK 键位 / UNIQUE F1 / DuplicateKey 门输入不变（INT 声明列不受升格影响），错误优先级与文本逐字节保持；升格后键位派生自存储值，两态一致。UPDATE 侧按契约位置单趟（升格改写 `new_value`，Step 7 派生自升格后值，天然一致）。
2. **Float 键列收 Int 值的语义后果**（升格的必然结果，非缺陷）：该类行自 Int-tag 有键行变为 Float 值无键行——同键位重复 INSERT 不再 DuplicateKey（无键行语义，MS10-T05 先例）；PK 等值 UPDATE/DELETE 经索引不可达（KeyNotFound，键位行此前同样不可达；SELECT 经 MS16 非 Int 键路由回退谓词下推仍可达）。R3-S7 锚点（Float 键列收 Int 值保持接受）恒 GREEN。已有测试零校准，属既有矩阵之外的边缘语义演进，请 Plan Review 分类确认。
3. **`extract_insert_values` 可见性收窄 pub(crate) → 私有**：`InsertValue` 中间形态为模块私有后触发 clippy more-private-type 警告；该方法唯一调用点即同模块 `build_insert`（grep 确认），收窄可见性消除警告、中间形态保持模块内部。契约 Preserve「对外签名不变」不受影响（方法为 pub(crate) 内部面）。
4. **subset_insert_test e2e 到位即 GREEN**：1.4/1.5 先行落地使 e2e 链路完整；该面测试见证在 planner_test（编译 RED→GREEN）与 catalog 单测（字段缺失编译 RED→GREEN）层先行观察。1.6 渲染面（lifecycle/cli_test）实现后以「临时禁用渲染块」方式补观察 RED→GREEN（渲染面用例在实现后编写，RED 以禁用新行为确认测试有效性）。
5. `map_insert_values` 无清单路径支持 `DEFAULT` 关键字填充（spec 场景仅列清单形态；「DEFAULT 关键字等价省略」语义对无清单 INSERT 同样成立，D2 填充规则自然覆盖，未额外扩面）。

**Blocker Handoff**

None

**Blocker Resolution**

None

**Self-Review**

- Plan compliance: PASS——7 个 Task Contract 逐项对账：Targets/Required 全部达成；Preserve 面（既有门优先级/文本、catalog 前缀布局、全列与无清单行为、dump/restore/import 通道）经既有矩阵零校准确认；Forbidden 项未触碰（无计划期 NOT NULL 重复拒绝、无隐式转换扩面、无 key/unique 文本改动、无身份型证据工程）。
- Full diff reviewed: PASS——完整 diff 独立重读（insert.rs/update.rs 门结构、catalog serde p 算术、table_manager 四构造点、planner 填充路径、pipeline 接线、lifecycle 渲染）；跨任务交互核查：类型门 × DEFAULT 填充（声明值经 coerce/升格后过门）、类型门 × 恢复通道（serialize 按值 tag，升格值往返一致）、子集放宽 × MS16 拒绝面（未知列/重复列/无清单行长度保持）。
- Critical findings unresolved: 0
- Important findings unresolved: 0
- Minor findings unresolved: 1——`DEFAULT 1` 声明于 BOOL 列在 INSERT 时被类型门拒绝（声明面不做 CREATE 期校验，错误文本清晰可定位；声明期默认值类型校验超出本 Iteration 范围，留待 Plan Review 裁定是否纳入 Iteration 001 或另行登记）。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| 类型门矩阵 | `cargo test --test write_type_conformance_test` | `13 passed; 0 failed`（RED 先行：6 failed 见证） | R2-S1..S6 全场景（error.rs 变体 + insert.rs/update.rs 门 + 优先级锚点 + dump/import 通道形态） | PASS |
| 受影响既有边界 | `cargo test --test constraint_enforcement_test --test key_type_conformance_test` | `24 passed` / `8 passed` | MS23 约束面 + MS16 键列门零回归（优先级/文本不变） | PASS |
| 子集 INSERT 端到端 | `cargo test --test subset_insert_test` | `6 passed; 0 failed` | R1-S1/S2/S3/S4/S5/S8（DEFAULT 填充/NULL/NOT NULL 拒绝/重启/全列零回归） | PASS |
| 计划期矩阵 | `cargo test --test planner_test` | `53 passed; 0 failed`（编译 RED 先行） | 1.5 子集/DEFAULT/未知列/重复列计划面 | PASS |
| dump/restore/schema | `cargo test --test cli_test test_dump_restore_roundtrip_preserves_defaults` | `1 passed` | R1-S6 全链路（真二进制） | PASS |
| catalog/table_manager 单测 | `cargo test --lib` | `311 passed; 0 failed` | 1.4 序列化往返/旧格式兼容/defaults 跨重开 + 既有 lib 面 | PASS |
| 校准面 | `cargo test --test insert_column_list_test` | `7 passed; 0 failed` | MS16 列清单矩阵按新语义（校准 1 处） | PASS |
| 全量 | `cargo test` | `1182 passed; 0 failed; 2 ignored`（基线 1152 + 净增 30，exit 0） | 全仓库（最终状态新鲜运行） | PASS |
| 静态分析 | `cargo clippy --all-targets` | `Finished`（0 warning，1 个引入面警告已修） | 全 targets | PASS |
| 格式化 | `cargo fmt --check` | 本 change 改动区零漂移（详见 Remaining Issues 注 3） | 改动面 | PASS |

**Persisted Evidence**

None required（Mode: none——全部验证为 cargo 原生命令，输出与退出码如上承载，无不可低成本重跑现场）

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| Runbook / Issue | None | | |

**Remaining Issues**

1. Deviation 2 的语义后果（Float 键列收 Int 值 → 无键行 → PK 等值 UPDATE/DELETE 不可达）请 Plan Review 分类：属目标行为自然延伸还是需补设计。
2. 混合库边缘：变更前写入的 Int-tag Float 键列行，`UPDATE SET f = <另一 Int>` 的 rekey 预检以升格前值派生键，可能对旧有键条目报 DuplicateKey（新库状态不可达；未做工程化规避）。
3. 工作树存在本 change 之前的 fmt 漂移（MS23 未提交代码：`delete.rs`/`recovery.rs`/`constraint_enforcement_test.rs`/`cli_test.rs` 旧段/table_manager 测试区），按 Surgical Changes 保持原样；本 change 改动区已 fmt 校准。建议后续独立批次全局 `cargo fmt`。

**Commit or Diff Reference**

未创建 Git commit（工作树含 MS23 既有未提交改动，commit 时机由用户指令；本 change 改动面见 Changed Files 清单）

## Plan Review

- Review Result: accepted

**Findings**

独立审查（2026-09-25）：实际代码与 diff 全量重读（12 产品文件 + 7 测试文件逐契约对账），Act Response、Self-Review 与验证证据核对完毕。工作树自 Act 验证运行后零变化（本 Review 全程只读，git status 与 Act 描述文件集一致）。

阻塞 Acceptance 的发现：无。

非阻塞 Minor findings：
1. `deserialize_catalog_column_row` 固定宽 payload 臂（TAG_INT/TAG_FLOAT/TAG_TIMESTAMP 经 `read_i64`）读后不推进 `p`——该段为行内最后字段且逐行独立反序列化，无越界后果（TAG_STRING/DATE/BOOL 臂各自校验长度）；等价控制流，不要求修改。
2. BOOL 列声明类型不匹配的 DEFAULT（如 `DEFAULT 1`）仅在 INSERT 写入期被类型门点名拒绝（Act Self-Review Minor 一致）——声明期默认值类型校验属新范围不入本 change，是否登记 improvement 候选由用户裁定（Recorder 域）。
3. proposal Impact 段「STRING PK 收 Int 值落无键行」表述与实现不符——实现为 `ColumnTypeMismatch` 拒绝（STRING 列收 Int）；实际落无键行的是 FLOAT 声明键列收 Int（升格后）。proposal 为历史现场不改写，语义以 spec R2 + 已知边界段与实现为准。

**Deviation Classification**

1. INSERT 类型门拆两趟（升格趟 coerce 后静默 / 拒绝趟契约位）— `ACT-DEVIATION`（合理，非阻塞）。代码核实：升格趟（insert.rs:137-143）先于键位派生（:155/:175）为恢复两态一致的必要结构——Float 键列收 Int 值若在键位派生后升格，索引条目派生自 Int、恢复期自 tuple 重建为无键，两态矛盾；拒绝趟（:218-236）位于契约位置，既有门输入与文本零变化（NOT NULL :147 / PK 门 :161 / DuplicateKey :177 / UNIQUE+F1 :195 均先行）。根因属 Plan 未预判「升格×键位派生」交互的计划面缺口，Act 修复正确且全量验证绿，不构成返工项。
2. Float 键列收 Int 值升格后落无键行 — `ACT-DEVIATION` 的自然延伸（非阻塞）：用户批准的 FLOAT 升格在 `to_key` 仅 Int 产键（I024 键编码域外）约束下的唯一一致结局；与非整数值 Float 键列既有无键行语义（MS10-T05）收敛，恢复两态一致，SELECT 经 MS16 非 Int 键路由可达。处置：sql-write-surface delta R2 已补已知边界段（本次 Review 落笔，含存量混合库 rekey 保守误报边缘）；不构成 Acceptance gap——R2/R5 场景全绿，既有矩阵零校准（无既有测试见证该形态的键控性，Act 的 Deviation 2 分类请求就此裁定）。
3. `extract_insert_values` pub(crate) → 私有 — 非实质：clippy more-private-type 警告消除；`InsertValue` 模块私有中间形态、唯一调用点 build_insert（grep + 本次重读双确认）；Preserve「对外签名不变」不受影响（pub(crate) 内部面）。
4. subset_insert_test e2e 到位即 GREEN、渲染面以「临时禁用渲染块」补观察 RED — 测试有效性见证的等价形态（TDD 意图满足：测试对行为变化敏感已实证），非阻塞。
5. 无清单 INSERT 支持 `DEFAULT` 关键字填充 — 非偏差：spec R1 requirement 正文「`VALUES` 中的 `DEFAULT` 关键字 SHALL 等价于该列为省略」未限定清单形态，D2 无清单分支 fill 规则自然覆盖（Act 记录透明性认可）。

**Acceptance Gaps**

None——R1（S1-S8 经 subset_insert_test 6 用例 + planner_test 5 增量 + cli_test dump/restore/schema 往返 + catalog/table_manager/lifecycle 单测）、R2（S1-S6 经 write_type_conformance_test 13 用例 + 既有 constraint/key_type/planner 矩阵零校准）、R5（全量 1182 全绿 + 校准 1 处已记录且防回归意图承接）全部见证。RTM 五行 Covered 与实现一致。

**Convergence**

N/A（首次 Review）

**Evidence**

- 代码独立重读：`error.rs`（ColumnTypeMismatch :63-68 镜像 KeyTypeMismatch 文案形状）；`insert.rs`（升格趟 :137-143 / NOT NULL :147 / PK 门 :161 / DuplicateKey :177 / UNIQUE+F1 :195 / 拒绝趟 :218 / serialize :239——既有门序与文本未触碰）；`update.rs`（UNIQUE 预检 :205 / 类型门 :228-248 含升格改写 values+new_value / Step 7 :299 派生自升格后值）；`catalog.rs`（尾段仅声明时追加 :777-809 / 可选读 + 截断校验 :891-989 / 3 单测 :1270-1348 含旧格式逐字节一致）；`table_manager.rs`（5 元组 :323 / 壳 :298 / defaults :411-419 / open_or_init :268 / 两 recovery swap 继承 :511/:554 / reopen 单测 :966）；planner `mod.rs`（table_defaults :114/:161）；`pipeline.rs`（register_table :1115）；`ddl_dml.rs`（InsertValue :66 / DEFAULT 臂 :285 / map_insert_values :170-248——未知列/重复列/行长拒绝与全宽填充、无清单 DEFAULT 臂）；`create_table.rs`（:68 透传）；`lifecycle.rs`（DEFAULT 渲染 :598-606 + 渲染单测 :1156）；`tuple.rs`/`page_format/mod.rs`（TAG_* pub(crate) 单一来源 :21-29/:13）。
- 测试独立重读：write_type_conformance_test 13 用例（RED 标注在案、PK/F1/DuplicateKey/NOT NULL 优先级锚点、零副作用断言、dump/import 通道形态）；subset_insert_test 6 用例（S1-S5/S8 含重启）；insert_column_list_test 校准 diff（无 panic 意图承接 + 未知列 :169 / 行长 :199 / 重复列 :256 原样）；planner_test MS24 块 :1012-1131（5 用例）；cli_test :3518 往返用例。
- 验证采信：Act Response Verification Evidence 表（2026-09-25）——全量 `cargo test` 1182 passed / 0 failed / 2 ignored exit 0、`cargo clippy --all-targets` 0 warning、各目标测试退出码在案；采信依据：覆盖范围表面（上述文件）自该运行零变化。
- Persisted Evidence：none 模式核对——无 `evidence/` 目录属预期，不作为问题。

**Follow-up Decision**

接受（`accepted`），无当前 Cycle 修复项。三项跟进（均不入本 Cycle、不阻塞）：
1. spec R2 已知边界段已由本次 Review 补入 delta（Float 键列无键行收敛 + 存量混合库 rekey 保守误报边缘）——随 change 收尾合并进主 spec。
2. BOOL 列声明类型不匹配 DEFAULT 仅写入期拒绝：是否登记 improvement 候选由用户裁定。
3. 工作树既有 fmt 漂移（MS23 未提交区）：建议用户另行安排独立 fmt 批次（Act 按 Surgical Changes 维持原样正确）。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

openspec/changes/2026-09-25-ms24-write-surface-completion/iterations/001-upsert-replace/000-initial.md（Status: ready，已展开）
