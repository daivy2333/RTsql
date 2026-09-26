# Iteration 000 / Cycle 000: 约束诚实化与 NOT NULL 强制

## Plan Context

- Status: ready
- Iteration: 000-not-null-honesty
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 1.7
- Depends on: None
- Stable baseline: 建表声明的 NOT NULL 在 INSERT/UPDATE 被零副作用强制并点名拒绝；CHECK/FK/方言项建表点名拒绝且不留半成品表；TableMeta 携带 not_null 标志且恢复读回一致；全量测试零回归
- Verification boundary: planner_test 拒绝矩阵 + constraint_enforcement_test（NOT NULL 面）+ cli_test 错误面/会话用例 + `cargo test` 全绿
- Diagnostic boundary: `src/parser/planner/ddl_dml.rs`、`src/executor/{insert,update}.rs`、`src/storage/{data/table_manager.rs,error.rs}` 与本 Cycle
- Deferred tasks: 2.1-2.9（Iteration 001，UNIQUE 端到端）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: MS23 全部 requirement 的 Iteration 000 部分（R1 NOT NULL、R2 诚实化、R6 零回归）；用户四项裁定（proposal 已记录）
- Excluded scope: UNIQUE 强制与 DDL 策略面一切内容（D4-D10，Iteration 001）；DEFAULT 应用；FOREIGN KEY 强制；CLI 管理面（MS19）

**Objective**

建表 DDL 中声明的 NOT NULL 在写入路径被强制（零副作用、点名列名拒绝），CHECK/FOREIGN KEY/方言项在计划期被点名拒绝而非静默忽略；约束标志经 TableMeta 抵达执行器；全量既有语义零回归。

**Background**

MS23 数据完整性约束执行面，统一执行序首位。Explorer 缺口调查（2026-09-24，用户批准「这些问题存在且应当解决」）：NOT NULL/UNIQUE 仅持久化不消费、CHECK/FK 解析期静默忽略——「DDL 静默接受、运行期永不生效」静默错误结果类缺陷。本 Iteration 先收其中不依赖索引演进的部分（T02+T03），为 Iteration 001（T01 UNIQUE）建立 TableMeta 标志通道与错误面先例。

**Investigation Facts**

- Current Baseline: master @ d8165fc。最后一个触及 Rust 表面的提交为 145bba4（MS13+MS17 初版）；`git diff 145bba4..HEAD -- src/ tests/ Cargo.toml Cargo.lock benches/` 为空，MS17 收口的 1101 tests pass / 0 failures / 2 ignored 结论按公共规则 › 验证 采信（覆盖范围未变化）。工作树另有未提交的纯文档批次（ARC-202609242151 清理 + MS23-MS27 路线），与本 Cycle 无关，不得触碰。
- Current-State Evidence:
  - 列级约束解析：`PlanBuilder::extract_column_constraints`（`src/parser/planner/ddl_dml.rs:369-396`）——`NotNull`/`Unique{is_primary:false}`/`Default` 产出 `ColumnConstraint`；`Null`/`ForeignKey`/`Check`/`DialectSpecific` 落 `_ => {}` 静默忽略（:390-391 注释明写）。
  - 表级约束消费：`build_create_table`（`ddl_dml.rs:472-509`）仅经 `extract_primary_key`（:430-469）消费表级 `Unique{is_primary:true}`；表级 Check/ForeignKey/Unique{false}/Index 类无任何消费点（静默丢弃）。
  - 约束标志流向：`ColumnDef::to_schema_column`（`src/executor/plan.rs:204-238`）折叠 constraints 为 `storage::data::ColumnSchema{not_null, unique, default_value}` → `CreateTableExecutor::next`（`src/executor/create_table.rs:49-57`）→ `TableManager::create_table_with_constraints`（`table_manager.rs:230-331`，入参四元组已含 not_null）→ `CatalogColumnRow.not_null/unique` 持久化（`catalog.rs:653-765`）。恢复路径 `scan_columns` 返回的 `CatalogColumnRow` 已携带 not_null（`open_or_init` 现丢弃，仅取 name/type，`table_manager.rs:167-171`）。
  - 运行时形状：`TableMeta`（`table_manager.rs:50-58`）字段 `name/columns: Vec<(String, ColumnType)>/pk_column/pk_index/index_manager/data_page_head/data_page_tail`——无约束标志。产品构造点恰 3 处：`open_or_init` :187、`create_table_with_constraints` :280、`replace_index_manager` :361（:361 克隆 old 其余字段）。tests/ 无 `TableMeta {` 直构（grep 0 命中）。
  - INSERT 路径：`InsertExecutor::next`（`src/executor/insert.rs:100-205`）逐行顺序——日期 coerce（:112-117）→ Int 键位类型预检（:124-132，`StorageError::KeyTypeMismatch`）→ PK 重复预检（:140-150，`index_manager.search` → `DuplicateKey`）→ serialize → 数据页写（:162-168）→ 页可见性（:171-174）→ WAL（:178-186）→ record_version（:189-191）→ PK 索引插入（:194-199）。无 not_null/unique 消费点。
  - UPDATE 路径：`UpdateExecutor::next`（`src/executor/update.rs:76-229`）——Step 1 索引定位（:84-87，`KeyNotFound`）→ MS16 键位校验区（:93-121，仅 SET 目标为 PK 列时）→ 日期 coerce（:126-135）→ 读旧 tuple/改值/序列化 → Step 6 写入（:163-165）→ WAL → Step 7 PK 索引三分支（:191-226）。planner 仅支持单列 SET + PK 等值 WHERE（`build_update`，`ddl_dml.rs:533-565`，`assignments.len() != 1 → UnsupportedStatement`）。
  - 错误面：`StorageError`（`src/storage/error.rs:8`，thiserror）——`KeyTypeMismatch{column,expected,actual}`（:39）为点名列名先例；`DuplicateKey`（:35）。CLI 渲染：仅 `DatabaseLocked`（exit 4）/`InvalidKey`（exit 5）有专用臂（`src/cli/mod.rs:294-299`），其余 StorageError 经 `sql_failure_status`（`cli/mod.rs:486`）进 Sql 失败路径（exit 3），错误文本自动进消息。
  - 会话回滚：CLI `run_sql` 每调用新建 `TransactionSession`，语句失败统一 `rollback_session` + 事务上下文后缀（MS11-T02 契约，`tx_statement_test.rs` 19 用例既有）；本 Iteration 不新增事务分支。
  - 计划错误面：`PlanError`（planner 枚举）已有 `UnsupportedStatement`/`ParseError(String)`/`MultiplePrimaryKey` 等变体；新点名变体仿此风格。
  - 既有测试面：`tests/planner_test.rs:140`（DDL plan 形状）、`tests/cli_test.rs:1043,1081`（schema 渲染 NOT NULL/UNIQUE）均只断言 DDL/渲染，无「向 NOT NULL 列插 NULL 或向 UNIQUE 列插重复值」的现存用例（已核对）——预期既有用例零校准。
- Code and Critical Path: DDL 文本 → `build_plan`（约束解析收紧点 1.1/1.2）→ `CreateTableNode` → `CreateTableExecutor` → `create_table_with_constraints`（标志持久化，不动）→ 打开时 `open_or_init`（标志读回点 1.3）。DML 文本 → plan → `InsertExecutor`/`UpdateExecutor`（校验点 1.4/1.5）→ `StorageError::NullConstraintViolation` → CLI Sql 失败路径（exit 3）。

**Implementation Guidance**

按任务序执行：1.1→1.2（解析收紧，纯 planner 层）→1.3（TableMeta 通道）→1.4/1.5（写路径消费）→1.6（CLI 面验证）→1.7（全量）。1.1/1.2 先行可独立 RED/GREEN；1.4 依赖 1.3 的标志通道。设计细节以 design.md D1-D3 为准（本文件与 design 冲突时以 Task Contract 为准）。

**Behavioral Change**

- 当前：`CREATE TABLE t(a INT CHECK(a>0))`、`b INT REFERENCES o(x)`、方言项建表全部成功且约束被静默丢弃；NOT NULL 列插 NULL / UPDATE 置 NULL 成功落库。
- 目标：上述形态建表以点名错误拒绝（表不创建）；NOT NULL 列 NULL 写入以 `NullConstraintViolation{column}` 拒绝（INSERT 零副作用；UPDATE 原值保持）；未声明 NOT NULL 的列（含未声明 PK 的 NULL keyless 语义）行为逐字节不变。
- 接口/状态/错误语义：新增 `StorageError::NullConstraintViolation{column}` 与 `PlanError` 点名拒绝变体；无既有错误文本变化；无存储格式、WAL、页格式变化。

**Task Contracts**

### 1.1: 列级 CHECK/FK/方言项建表点名拒绝

- Requirement/Scenario: R2 S1（列级 CHECK）、S2（列级 FK）、S3（方言项）
- Depends on: None
- Targets: `src/parser/planner/ddl_dml.rs::PlanBuilder::extract_column_constraints`
- Current behavior: `ColumnOption::Check`/`ForeignKey`/`DialectSpecific` 落 `_ => {}`，建表成功、约束静默丢弃
- Required behavior: 三类选项返回 `Err(PlanError::…)`，错误文本点名对应特性（CHECK / FOREIGN KEY / 方言特定选项）；`Null`/`Comment` 维持忽略；`NotNull`/`Unique`/`Default` 分支行为不变
- Required changes: 新增 `PlanError` 点名拒绝变体（命名仿既有 `UnsupportedStatement` 风格，文本含特性名）；`extract_column_constraints` 三臂返回错误
- Preserve: 既有五类已处理选项的解析结果与错误文案逐字节不变；`extract_default_value`/`extract_primary_key` 不动
- Forbidden: 不触碰表级约束处理（1.2 范围）；不改 `to_schema_column` 折叠逻辑；不实现 CHECK/FK 语义
- Test witness: `tests/planner_test.rs` 新增拒绝矩阵用例（CHECK/FK/AUTO_INCREMENT 方言项 × 建表期望 Err 且文本点名；NOT NULL/DEFAULT/PK 既有成功用例保持）——先观察 RED（当前成功不报错）
- GREEN condition: 矩阵全绿且 `cargo test --test planner_test` 既有用例零回归
- Verification: `cargo test --test planner_test`；退出码 0；新增用例名与断言写入 Act Response
- Stop when: sqlparser 0.44 的 `ColumnOption` 变体名与本契约书写不符（以实际枚举为准调整臂匹配，属非实质偏差可记录后继续）；或发现 CHECK 经 `TableConstraint` 承载的方言路径（返回 Plan）

### 1.2: 表级 CHECK/FK/Index 类约束点名拒绝

- Requirement/Scenario: R2 S4（表级 CHECK/FK）
- Depends on: 1.1（共用新 PlanError 变体）
- Targets: `src/parser/planner/ddl_dml.rs::PlanBuilder::build_create_table`
- Current behavior: `constraints: &[TableConstraint]` 仅经 `extract_primary_key` 消费 PK；表级 Check/ForeignKey/Index 类静默丢弃
- Required behavior: `build_create_table` 在提取列定义后遍历 `constraints`，对 `TableConstraint::Check`/`ForeignKey` 及 Index/Fulltext 类变体返回点名错误；`Unique{is_primary:true}`（PK）继续由 `extract_primary_key` 消费；`Unique{is_primary:false}` 本任务不触碰（Iteration 001）
- Required changes: `build_create_table` 新增表级约束遍历臂
- Preserve: 空约束切片与仅 PK 的建表行为不变；`extract_primary_key` 签名与行为不变
- Forbidden: 不处理表级 `Unique{is_primary:false}`（禁止提前实现映射或拒绝）；不改列级臂
- Test witness: planner_test 表级 CHECK / 表级 FK 拒绝用例（RED→GREEN）；表级 PK 既有用例保持绿
- GREEN condition: 新用例绿 + planner_test 全绿
- Verification: `cargo test --test planner_test`；退出码 0
- Stop when: sqlparser 0.44 `TableConstraint` 变体集与预期不符（按实际枚举匹配，记录偏差后继续）

### 1.3: TableMeta 携带 not_null 标志并恢复读回

- Requirement/Scenario: R1（前置通道；支撑 S1-S5 全部场景）
- Depends on: None（与 1.1/1.2 并行可）
- Targets: `src/storage/data/table_manager.rs::TableMeta`、`::create_table_with_constraints`、`::open_or_init`、`::replace_index_manager`
- Current behavior: `TableMeta` 无约束标志；`open_or_init` 自 `CatalogColumnRow` 仅取 `(column_name, column_type)`（not_null 读后即弃）
- Required behavior: `TableMeta` 新增 `pub not_null: Vec<bool>`（与 `columns` 列序对齐）；`create_table_with_constraints` 自入参四元组填入；`open_or_init` 自 `CatalogColumnRow.not_null` 读回；`replace_index_manager` 继承旧值
- Required changes: 结构体字段 + 三构造点
- Preserve: `create_table`（无约束委托壳，:209）签名与行为不变；catalog 序列化格式零变化（not_null 已持久化）；`TableMeta` 其余字段与既有消费点零变化
- Forbidden: 不在本任务加 `unique_indexes`（Iteration 001）；不改 catalog 行格式
- Test witness: `src/storage/catalog.rs` 或 `table_manager.rs` 既有 `#[cfg(test)]` 区（`create_table_with_constraints_persists_flags` :500 同区）新增单测——建含 NOT NULL 列的表 → 新 TableManager `open_or_init` 等价路径读回 `not_null` 标志为 true、无标志列为 false。先观察 RED（字段不存在编译失败即见证，或先以读回断言 RED）
- GREEN condition: 单测绿；`cargo test --lib`（或对应目标）零回归
- Verification: `cargo test --lib`；退出码 0
- Stop when: 三构造点之外出现第四个 `TableMeta` 构造点（返回 Plan 补契约）

### 1.4: INSERT NOT NULL 零副作用强制

- Requirement/Scenario: R1 S1（INSERT 拒绝）、S3（非 NULL 成功）、S4（PK+NOT NULL）、S5（未声明零回归）
- Depends on: 1.3
- Targets: `src/executor/insert.rs::InsertExecutor::next`、`src/storage/error.rs::StorageError`
- Current behavior: NOT NULL 列 NULL 值照常落库
- Required behavior: 逐行在日期 coerce（现 :112-117）之后、Int 键位类型预检（现 :124）之前，逐列检查 `self.table_meta.not_null[i] && value.is_null()`，命中返回 `Err(StorageError::NullConstraintViolation { column: <列名> })`；错误文本含列名（thiserror 格式仿 `KeyTypeMismatch`）；检查失败时未触任何数据页/WAL/索引/版本记录
- Required changes: error.rs 新变体 + insert.rs 校验臂
- Preserve: coerce → 键位类型 → PK 重复 → 写入 → 索引的既有顺序与全部既有错误文案；无键行语义（未声明 NOT NULL 的 PK 列 NULL 照旧落库不入索引）；多行 VALUES 中前序行已成功的行为保持（逐行处理，失败行之前的行已落库——与既有 KeyTypeMismatch/DuplicateKey 行为同型，契约不改变它）
- Forbidden: 不加 unique 检查；不改 coerce 与键位预检的相对顺序（NOT NULL 插在两者之间）；不动 UPDATE（1.5 范围）
- Test witness: 新建 `tests/constraint_enforcement_test.rs`——(a) INSERT NULL → Err 且文本点名；(b) 拒绝后同库重查行数不变（零副作用）；(c) 非 NULL 插入成功；(d) `PRIMARY KEY NOT NULL` 组合 NULL 键位 → NullConstraintViolation（非无键行）；(e) 未声明列 NULL/keyless 既有语义对照用例。先观察 RED（当前 (a)(b)(d) 行为为成功落库）
- GREEN condition: 五组用例绿
- Verification: `cargo test --test constraint_enforcement_test`；退出码 0
- Stop when: 发现校验点与 coerce 存在语义耦合（如 String 日期列 NULL 需 coerce 先行的例外形态与契约冲突）——返回 Plan

### 1.5: UPDATE SET NOT NULL 强制

- Requirement/Scenario: R1 S2（UPDATE 拒绝 + 原值保持）、S3
- Depends on: 1.3、1.4（共用错误变体）
- Targets: `src/executor/update.rs::UpdateExecutor::next`
- Current behavior: SET 目标列为 NOT NULL 时置 NULL 成功
- Required behavior: Step 1 索引定位（KeyNotFound 保持优先）之后、MS16 键位校验区同层，SET 目标列 `not_null` 且 `new_value` 为 NULL → `Err(StorageError::NullConstraintViolation { column })`，位于任何写入（Step 6）之前，原行保持
- Required changes: update.rs 校验臂（列名→标志查 `table_meta.not_null` 按 `columns` 列序定位）
- Preserve: KeyNotFound 优先序；MS16 键位校验行为与顺序；日期 coerce 行为；Step 7 索引三分支
- Forbidden: 不改 planner 单列 SET 限制；不动 INSERT；不加唯一校验
- Test witness: constraint_enforcement_test——UPDATE 置 NULL → Err 点名 + 原值保持；SET 非 NULL 成功；目标行不存在时 KeyNotFound 优先。RED→GREEN
- GREEN condition: 用例绿 + update 既有测试零回归
- Verification: `cargo test --test constraint_enforcement_test --test executor_test`；退出码 0
- Stop when: 无（契约面清晰）

### 1.6: CLI 错误面与会话接线验证

- Requirement/Scenario: R1 S1/S2 的 CLI 可观察面；R6 S3
- Depends on: 1.4、1.5
- Targets: `tests/cli_test.rs`（纯测试任务；产品侧预期零改动——StorageError 新变体经既有 Sql 失败路径自动渲染，无专用臂）
- Current behavior: n/a（验证任务）
- Required behavior: (a) auto-commit `INSERT` 违反 NOT NULL → CLI exit 3、stderr 文本含列名与 statement k of n 模板；(b) 显式事务内违反 → 自动回滚 + 事务上下文后缀 + exit 0（既有 MS11-T02 契约，回滚后事务不残留）且表无残留行；随后可正常写入
- Required changes: 仅新增 cli_test 用例；若 (a)/(b) 任一与预期不符（如新错误意外落 exit 1），先按证据定位渲染面，再返回 Plan——不得静默加专用臂
- Preserve: 既有 exit code 映射（0/1/2/3/4/5）零变化
- Forbidden: 不改 `sql_failure_status`、不加 StorageError 专用 CLI 臂
- Test witness: cli_test 两条新用例（RED 不适用——预期既有机制直接满足；若 GREEN 直接过，作为既有机制覆盖的见证记录）
- GREEN condition: 用例绿 + cli_test 既有 78 用例零回归
- Verification: `cargo test --test cli_test`；退出码 0
- Stop when: 任一预期观察不符（渲染路径假设失效）

### 1.7: 全量回归

- Requirement/Scenario: R6 S1-S3
- Depends on: 1.1-1.6
- Targets: 全仓库
- Current behavior: 1101 tests pass（采信结论，见 Current Baseline）
- Required behavior: `cargo test` 全绿；如出现失败用例，逐条归因：属本 change 契约校准 → 修改并记录理由；非本 change 面 → 返回 Plan
- Required changes: 预期零修改（已核对无既有用例依赖静默行为）；出现校准则逐条记录
- Preserve: 非 MS23 面的用例零修改
- Forbidden: 不得为过测试弱化断言
- Test witness: `cargo test` 完整输出末尾统计行
- GREEN condition: 0 failures；ignored 数量与基线一致（2）
- Verification: `cargo test`；退出码 0；统计行写入 Act Response
- Stop when: 出现无法归因本 change 的失败

**Invariants**

- 无约束表的全部 DML/DDL 行为逐字节不变（R6）。
- PK 索引既有语义（DuplicateKey/KeyTypeMismatch/无键行落库不入索引）不变。
- catalog 序列化格式、页格式、WAL 记录格式零变化（本 Iteration 无格式演进）。
- 既有错误文案零变化；新错误只增不改。
- CLI exit code 映射（0/1/2/3/4/5）不变。
- 恢复（`recovery.rs`）零变化（本 Iteration 不触恢复面）。

**Non-goals**

- UNIQUE 强制与 DDL 策略面全部内容（Iteration 001：2.1-2.9）
- DEFAULT 值应用（MS24-T01）；FOREIGN KEY/CHECK 语义实现（永久 Non-goal，只诚实化）
- `install.sh`、README、runbook（2.9 收口时统一处理文档）
- 工作树中未提交的文档批次（ARC-202609242151 + 路线规划）——不得触碰、不得暂存

**Acceptance**

- R1：NOT NULL 列 INSERT/UPDATE NULL 写入被点名拒绝且零副作用（constraint_enforcement_test S1-S5 五组）；未声明列零回归。
- R2：列级与表级 CHECK/FK/方言项建表计划期点名拒绝，表不创建（planner_test 矩阵）。
- R6：`cargo test` 全绿（0 failures，ignored=2），无未归因修改。
- 映射：R1→D1/D2→1.3/1.4/1.5/1.6；R2→D3→1.1/1.2；R6→1.7（tasks.md RTM）。

**Verification**

| Scenario | 判定 |
|---|---|
| R2 拒绝矩阵 | `cargo test --test planner_test` 退出码 0，新用例绿 |
| R1 执行面 | `cargo test --test constraint_enforcement_test` 退出码 0，五组用例绿 |
| R1 CLI 面 | `cargo test --test cli_test` 退出码 0，新用例绿 |
| R6 回归 | `cargo test` 退出码 0，统计 `0 failures; 2 ignored` |

全部为测试框架原生退出码与统计输出判定；无人工步骤；无 Evidence 目录要求。

**Gate 2 Readiness**

- 无 Missing requirement：PASS——RTM 六行全 Covered（tasks.md），无 Simplified。
- 调查完整：PASS——Current-State Evidence 列全入口/调用链/构造点/测试面（本文件），基线经只读检查采信（145bba4 后 Rust 面 diff 空）。
- 设计闭合：PASS——design.md D1-D3 定案标志通道、校验位置、错误面；无影响契约语义的 TBD。
- 任务可执行：PASS——七个 Task Contract 均有 Targets/Current/Required/Preserve/Forbidden/见证/停止条件。
- 分轮合理：PASS——Iteration Plan 平衡审计记录于 tasks.md；单 Iteration 承载单一正确性成果。
- 追踪完整：PASS——RTM requirement×scenario×design×task×code×test 全链接。
- 验证充分：PASS——覆盖 R1/R2 全部 scenario（含 sad path：拒绝矩阵、零副作用、回滚接线）与 R6 回归；全部最简直接判定（测试框架原生退出码）。
- 无身份型证据工程：PASS——无哈希/run-id/判定层；验证用既有 cargo test。
- 无实质未知项：PASS——sqlparser 变体名核对类非实质项已写入 Task Contract 停止条件（遇实名不符按实际枚举调整并记录）。
- OpenSpec 一致：PASS——proposal/design/tasks/specs/cycle 相互一致；`openspec validate --strict` 于交付前执行。
- Persisted Evidence：PASS——Mode none（下方声明），全部验证 Act Response 可承载。
- 用户批准计划：**PASS——用户于 2026-09-24 规划交付后明确答复「批准」（原话），Gate 2 全部 12 项 PASS，Plan Context 置 ready。**

**Persisted Evidence**

- Mode: none

Act Response 记录命令、决定性输出（每项 ≤20 行）、退出码即满足全部验收判定；无不可复现依赖、无一次性环境、无需要留场的现场。Budget：不适用。

**Risks and Notes**

- sqlparser 0.44 `ColumnOption`/`TableConstraint` 确切变体名以实际代码为准（`cargo doc`/源码可查）；臂匹配形态差异属非实质调整，记录于 Act Response Deviations。
- `NOT NULL` 与日期 coerce 的相对顺序已定（coerce 先）；String 日期列 + NULL 组合由 coerce 原样通过后落 NOT NULL 校验，无冲突形态。
- 既有用例若出现意外依赖静默行为的失败（调查判断为零），按 1.7 契约归因处理，不得静默改断言。
- 多行 VALUES 逐行语义（失败行前已落库）为既有同型行为，保持不变；如用户预期为语句级原子，属新需求返回 Plan。

## Act Response

- Status: reported

**Implemented**

Iteration 000 全部 7 个 task 完成（1.1-1.7），全部按契约 RED→GREEN（1.6 按契约为既有机制见证、直接 GREEN）。列级与表级 CHECK/FOREIGN KEY/Index 类/方言项建表计划期点名拒绝（`PlanError::UnsupportedConstraint(&'static str)`，文本点名特性，表不创建）；`TableMeta` 新增 `not_null: Vec<bool>`（与 columns 列序对齐）经 create/open_or_init/replace 三构造点接线，恢复读回一致；INSERT 在日期 coerce 后、键位预检前逐列强制 NOT NULL，UPDATE 在 Step 1 后、任何写入前对 SET 目标列强制——违反返回新 `StorageError::NullConstraintViolation { column }`（thiserror 点名，经既有 Sql 失败路径渲染 exit 3，无专用 CLI 臂），零副作用；未声明列（含未声明 NOT NULL 的 PK 键位 keyless 语义）行为逐字节不变。

**Changed Files and Symbols**

产品（7 文件）：
- `src/parser/error.rs` — `PlanError::UnsupportedConstraint(&'static str)` 新变体 + Display（"Unsupported constraint: {what}"）。
- `src/parser/planner/ddl_dml.rs` — `extract_column_constraints` 新增 Check/ForeignKey/DialectSpecific 三拒绝臂（`Null`/`Comment` 落 `_ => {}` 维持忽略）；`build_create_table` 新增表级约束遍历（Check/ForeignKey/Index/FulltextOrSpatial 点名拒绝，`Unique` 两态不触碰，PK 继续走 `extract_primary_key`）。
- `src/storage/data/table_manager.rs` — `TableMeta.not_null` 字段；`create_table_with_constraints`/`open_or_init`（自 `CatalogColumnRow.not_null` 读回）/`replace_index_manager`（继承）三构造点；推翻旧决策的过时注释与 `create_table_with_constraints` doc（"metadata only" 表述）同步更新；新增单测 `table_meta_carries_not_null_flags_across_reopen`。
- `src/executor/insert.rs` — `InsertExecutor::next` coerce 后、键位类型预检前逐列 NOT NULL 校验臂。
- `src/executor/update.rs` — `UpdateExecutor::next` Step 1 后、MS16 键位校验区同层的 SET 目标列 NOT NULL 校验臂（未知列不拦截，维持 Step 3 ColumnNotFound）。
- `src/storage/error.rs` — `StorageError::NullConstraintViolation { column: String }`（"NOT NULL constraint violation: column '{column}'"）。

测试（3 文件）：
- `tests/planner_test.rs` — +7（列级 CHECK/FK/AUTO_INCREMENT 拒绝 ×3、NULL/COMMENT 忽略 Preserve ×1、表级 CHECK/FK 拒绝 ×2、表级 PK Preserve ×1）。
- `tests/constraint_enforcement_test.rs` — 新建 8 用例（INSERT 面 a-e 五组 + UPDATE 拒绝保持/SET 非 NULL/KeyNotFound 优先三组）。
- `tests/cli_test.rs` — +2（auto-commit 违反 exit 3 + k-of-n 模板；事务内违反回滚 + 后缀 + 无残留 + 随后可写）。

**Deviations from Plan**

1. **1.6(b) 退出码：契约写「exit 0」，实际按既有 MS11-T02 机制为 fail-fast exit 3。** 证据：`src/cli/mod.rs` `sql_failure_status` 对事务内语句失败返回 `ExitStatus::Sql`（exit 3），事务上下文后缀由 `rollback_session` 驱动；既有锁定测试 `tx_statement_test::in_tx_fail_fast_notes_uncommitted` 明确断言 `Some(3)`。若按契约字面实现 exit 0 将违反本 Cycle R6 零回归与 `sql-transaction-statements` 规格 R3。契约的核心验证目标（新错误经既有 Sql 失败路径渲染、无专用 CLI 臂、回滚无残留、随后可写）全部成立；仅「exit 0」一处为 Plan 对既有行为的事实性笔误。用例按实际既有机制断言 exit 3，分类为 Plan 错误（非阻塞——Plan Review 请核验此分类）。
2. 1.6 GREEN 条件写「cli_test 既有 78 用例」——78 为 MS13 收口时点数字，实际既有 93（含 2 ignored）；按实际计数断言零回归。
3. 契约 1.2「Index/Fulltext 类变体」落地为 `TableConstraint::Index` 与 `FulltextOrSpatial` 两臂（sqlparser 0.44 实名，与契约书写一致）；变体名核对类停止条件未触发。

**Blocker Handoff**

None required.

**Blocker Resolution**

（无——本 Cycle 未进入 blocked 状态。）

**Self-Review**

- Plan compliance: 7/7 task 按契约完成；校验插入位置（INSERT coerce 后/键位预检前、UPDATE Step 1 后）与契约逐字一致；Forbidden 项（不改 coerce 相对顺序、不加 unique 检查、不动 sql_failure_status、不加专用 CLI 臂、不触碰未提交文档批次）逐项核对无违反；Invariants（无约束表行为不变、既有错误文案零变化、格式/WAL/页格式零变化、exit 映射不变、恢复面零变化）成立。
- Full diff reviewed: 是——Phase 4 对 `git diff src/` 与三个测试文件全量审查；发现并修复 1 处 Important finding（`create_table_with_constraints` doc "metadata only" 与新行为矛盾，已更新注释并重跑受影响验证 lib 298 + constraint 8，均绿）；无计划外修改；工作树中既有未提交文档批次（ARC-202609242151 + 路线规划）未被触碰、未暂存。
- Critical findings unresolved: 无。
- Important findings unresolved: 无（上述注释问题已修复）。
- Minor findings unresolved: 1 项——见 Remaining Issues（存量 NOT NULL 列含 NULL 数据的 dump→restore 后果），属 Plan Review 分类事项，Act 未采取行动。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| 1.1 RED | `cargo test --test planner_test test_create_table` | 3 passed; 3 failed（三个拒绝用例当前静默成功） | `extract_column_constraints` 既有行为 | RED 见证 ✓ |
| 1.1 GREEN | `cargo test --test planner_test` | `43 passed; 0 failed`（36 既有 + 7 新增） | 列级拒绝矩阵 + 既有 DDL 解析零回归 | PASS |
| 1.2 RED | `cargo test --test planner_test test_create_table_table_level` | 1 passed; 2 failed | `build_create_table` 既有行为 | RED 见证 ✓ |
| 1.2 GREEN | `cargo test --test planner_test` | `43 passed; 0 failed` | 表级拒绝 + 表级 PK Preserve | PASS |
| 1.3 RED | `cargo test --lib table_meta_carries_not_null` | `error[E0609]: no field not_null on type Arc<TableMeta>` | `TableMeta` 无字段（编译失败即见证） | RED 见证 ✓ |
| 1.3 GREEN | `cargo test --lib` | `298 passed; 0 failed` | 三构造点 + create/reopen 读回 + 全 lib 零回归 | PASS |
| 1.4 RED | `cargo test --test constraint_enforcement_test` | 2 passed; 3 failed（(a)(b)(d) 当前成功落库） | INSERT 既有行为 | RED 见证 ✓ |
| 1.4 GREEN | `cargo test --test constraint_enforcement_test` | `5 passed; 0 failed`（当时 5 用例） | INSERT NOT NULL 五组 | PASS |
| 1.5 RED | `cargo test --test constraint_enforcement_test update_` | 2 passed; 1 failed | UPDATE 既有行为 | RED 见证 ✓ |
| 1.5 GREEN | `cargo test --test constraint_enforcement_test` | `8 passed; 0 failed` | UPDATE 三组 + INSERT 五组 | PASS |
| 1.6 | `cargo test --test cli_test not_null` | `2 passed; 0 failed`（既有机制直接满足，RED 不适用） | CLI Sql 失败路径 + 会话回滚接线 | PASS |
| 1.6 回归 | `cargo test --test cli_test` | `93 passed; 0 failed; 2 ignored` | CLI 全量（91 既有 + 2 新增） | PASS |
| 1.7 全量 | `cargo test` | `passed=1119 failed=0 ignored=2`（基线 1101+2，净增 18 精确吻合） | 全仓库 | PASS |
| 注释修复后受影响面复跑 | `cargo test --lib` + `cargo test --test constraint_enforcement_test` | `298 passed` / `8 passed` | `table_manager.rs`（注释-only diff 后的受影响表面） | PASS |
| 警告检查 | `cargo build` + 测试编译 | 无新增 warning | 全部改动文件 | PASS |
| OpenSpec | `openspec validate 2026-09-24-ms23-constraint-enforcement` | `Change ... is valid` | change 结构 | PASS |

新鲜性注记：全量 1119 结论产生于 `table_manager.rs` 注释修复之前；该修复为 doc 注释-only diff，其后按覆盖范围规则对受影响表面（该文件所在 lib + 约束集成面）复跑绿，其余套件覆盖表面 diff 为空、结论按公共规则 › 验证 采信。

**Persisted Evidence**

None required（Plan Mode: none；无白名单情形，全部验证由本 Response 承载；未创建 evidence/ 目录）。

**Experience Candidates**

None.

**Remaining Issues**

1. （Minor，交 Plan Review 分类）存量兼容后果：本 change 之前写入的「NOT NULL 声明列含 NULL」存量行（旧版不强制时可达），其 dump→restore 在新版会被 `NullConstraintViolation` 拒绝（restore fail-fast）。与 proposal 已接受的非 INT UNIQUE dump→restore 破坏同类；建议 Iteration 001 的 2.9 README 兼容边界说明一并覆盖 NOT NULL 情形。Act 未扩大范围处理。

**Commit or Diff Reference**

未提交（用户未指令 commit）；工作树含本 Cycle 7 个代码/测试文件改动 + change 内 tasks.md/本 Response 更新；既有未提交文档批次原样保留。


## Plan Review

- Review Result: accepted

**Findings**

独立检查（非 Self-Review 复述）：对工作树全量 diff（6 产品文件 + 2 测试文件修改 + 1 新测试文件）逐契约核对——

1. 1.1/1.2：`extract_column_constraints` 三拒绝臂（Check/ForeignKey/DialectSpecific → `UnsupportedConstraint` 点名，`Null`/`Comment` 落 `_ => {}`）与 `build_create_table` 表级遍历（Check/ForeignKey/Index/FulltextOrSpatial，`Unique` 两态不触碰，PK 走 `extract_primary_key`）与契约逐条一致；错误文本与 Display 实现核对无误。
2. 1.3：`TableMeta.not_null: Vec<bool>` 与三构造点（create 直填 / open_or_init 自 `CatalogColumnRow.not_null` 读回 / replace 继承）接线正确；"metadata only" 过时注释已同步更新（Act Self-Review 发现的 Important finding 修复属实——doc 注释-only diff）；新增单测 `table_meta_carries_not_null_flags_across_reopen` 覆盖 create/reopen 双路径标志一致性。
3. 1.4：INSERT 校验臂位置（coerce 后、Int 键位预检前）与契约逐字一致；索引安全性独立核对——`coerced` 长度 ≤ schema 列数（zip 截断）≤ `not_null` 长度（列序对齐），`not_null[i]` 无越界面；零副作用成立（校验点先于数据页/WAL/索引/版本记录一切写入）。
4. 1.5：UPDATE 校验臂位置（Step 1 后、MS16 键位校验同层、任何写入前）与契约一致；未知列不拦截、维持 Step 3 ColumnNotFound（契约 Preserve 面正确）；KeyNotFound 优先由位置保证。
5. 1.6：两条 cli_test 用例断言与实际机制核对一致（含 `statement k of n` 模板与事务后缀）；未新增专用 CLI 臂、`sql_failure_status` 未动（Forbidden 面核对无违反）。
6. 1.7：范围控制核对——diff 恰为声明的 10 个文件，既有未提交文档批次（tasks/improvements/references/analysis 归档移动 + archive carrier + change 目录）未被触碰；无身份型证据工程、无计划外修改、无既有断言弱化。
7. Minor finding（Act Remaining Issues #1）核实为真：旧版不强制期可达的「NOT NULL 声明列含 NULL」存量行，其 dump DDL 在 restore 时会被 `NullConstraintViolation` fail-fast 拒绝。分类为非阻塞 Minor；处置已写入 Iteration 001 任务 2.9 契约（README 兼容边界与既有「非 INT UNIQUE dump→restore 破坏」合并文档化），无需当前 Cycle 行动。

**Deviation Classification**

- Deviation 1（1.6(b) 契约写 exit 0、实际 exit 3）→ **PLAN-INVALID**（成立，维持 Act 分类「Plan 错误，非阻塞」）：独立核验——`sql_failure_status`（`src/cli/mod.rs:470` 区）对事务内语句失败一律返回 `ExitStatus::Sql`（exit 3），既有锁定测试 `tx_statement_test::in_tx_fail_fast_notes_uncommitted`（`tests/tx_statement_test.rs:557`）明断言 `Some(3)`。契约「exit 0」违反本 Cycle 自身 R6 零回归与 `sql-transaction-statements` 规格 R3 既有契约；Act 按实际既有机制断言 exit 3 正确。契约核心验证目标（Sql 失败路径渲染、无专用臂、回滚无残留、随后可写）全部成立，Acceptance 不受影响，无需修复。
- Deviation 2（cli_test 既有计数 78 vs 实际 93）→ **PLAN-INVALID**：Plan 转录笔误（78 为 MS13 收口时点数），Act 按实际计数断言零回归正确，非阻塞。
- Deviation 3（表级变体落地为 `Index`/`FulltextOrSpatial`）→ 非实质：sqlparser 0.44 实名与契约「Index/Fulltext 类变体」表述一致，停止条件未触发。
- 无 ACT-DEVIATION、无 BASELINE-CHANGED、无 NEW-EVIDENCE。

**Acceptance Gaps**

None——R1（constraint_enforcement_test 五组 + cli_test 两用例 + 代码核对）、R2（planner_test 7 新用例 + 拒绝臂核对）、R6（1119/0/2 全量统计）全部满足。

**Convergence**

N/A（本 Cycle 首次 Review，无前一版 gap 可比较）。

**Evidence**

- 独立阅读：工作树 `git diff` 全量（src/parser/error.rs、src/parser/planner/ddl_dml.rs、src/storage/data/table_manager.rs、src/storage/error.rs、src/executor/{insert,update}.rs、tests/{planner,cli}_test.rs diff + tests/constraint_enforcement_test.rs 新文件）与 `src/cli/mod.rs::sql_failure_status`、`tests/tx_statement_test.rs:546-557` 核验偏差 1。
- 采信 Act Response 验证结论（公共规则 › 验证）：全量 1119/0/2 产生于 `table_manager.rs` 注释-only 修复前，其后受影响表面（lib 298 + constraint_enforcement 8）复跑绿，其余套件覆盖表面 diff 为空；本 Review 为只读检查、未修改覆盖范围内代码，采信成立并已核对 Act 验证后无进一步代码面变化。
- Exit codes：本次 Review 的只读命令（git diff/grep/sed、openspec validate）退出码均为 0；`openspec validate --strict` 通过。

**Follow-up Decision**

accepted——既有 Acceptance 全部满足；三处偏差均为非阻塞 PLAN-INVALID（Act 实现正确、Plan 文本笔误）；Minor finding 已移交 Iteration 001 任务 2.9 契约文档面，不构成当前 Cycle 修复项，不创建 rework。Iteration 000 完成。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

`iterations/001-unique-enforcement/000-initial.md`（已展开——Plan Context ready、Gate 2 十二项 PASS、Persisted Evidence none；D6 第 3 点论据不精确处已由 Plan 澄清裁定并写入 2.3/2.4 契约）
