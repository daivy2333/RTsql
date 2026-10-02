# Iteration 001 / Cycle 000-initial: DEFAULT 声明期一致（CREATE TABLE/TypedString 往返/ALTER）与文档收尾

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T4, T5, T6, T7
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 默认假设 7-9（归一形态、错误面分类、迭代序）；用户裁定 3（T05 DATE 往返修复 + T06 ALTER 同款与 T04 同轮交付）；design D6-D9；Iteration 000 全部交付面（T1/T2/T3/T8 已 accepted——见 Current Baseline）。本文所有行号基于当前工作区（Iteration 000 T8 守卫合入后）复核（2026-10-01 Plan Review 展开时读码）。
- Excluded scope: import 输入面（000 已收口）；写入类型门（sql-write-surface R2）运行期语义变更；dump 渲染格式变更；CHECK/FK 强制执行；`DEFAULT <表达式>`（非字面量）支持；`ALTER TABLE ADD COLUMN` 同名列冲突守卫（design D10 明示本 change 不扩围）。

**Objective**

建表/加列声明的 DEFAULT 字面量要么与列类型一致（含 FLOAT 升格归一、日期族解析验证）要么计划期点名拒绝零副作用——「建表成功但应用默认值的 INSERT 必然失败」分裂形态消除；日期族 DEFAULT 的 dump→restore 往返修复（TypedString 臂）；SKILL.md 与 README 双语逐条一致。

**Investigation Facts**

- Current Baseline: 分支 master @ 4e1944f；工作区含 Iteration 000 交付面（已 accepted：T1 stdin 通道、T2 自动建表〔修订语义〕、T3 `:memory:` 豁免、T8 重复列名守卫）与 MS28 收尾未提交项（无交集）。全量 **1672 tests / 0 failed / 2 ignored**（001-replan Act Response 分批 M19，2026-10-01；Plan Review accepted 采信）。`tests/planner_test.rs` 现 93 tests（含 T8 +3）；`tests/import_input_surface_test.rs` 12 tests。DEFAULT 声明面本 Iteration 前零校验（Bool 列 `DEFAULT 1` 建表被接受，任意字面量变体持久化——proposal 实证）。
- Current-State Evidence（2026-10-01 展开时读码复核，行号 = 当前工作区）：
  - **T4/T5 核心**：`extract_default_value`（`src/parser/planner/ddl_dml.rs:722`，`pub(crate)`）现臂：`Expr::Value` → `value_from_sqlparser`；`Identifier` 仅 NULL；一元负号折叠 Int/Float；**TypedString 落 `_ => Err(PlanError::UnsupportedValue)` 兜底臂**（T5 插入点——臂序在 catch-all 之前）。消费点 1：`extract_column_constraints`（:679）`ColumnOption::Default(expr)` 臂 :695-698 → `ColumnConstraint::DefaultValue(Value)`。
  - **T4 消费点**：`build_create_table` column_defs 装配（:856-868，`ColumnDef { name, data_type, constraints }`——data_type 与 DefaultValue 同结构体可直接比对）→ **T8 重复列名守卫（:870-879）** → `extract_primary_key`（:882）。T4 校验/归一循环置于 T8 守卫之后、PK 提取之前（与 T8 同为副作用前计划期门；两循环先后序非实质）。归一需改写 column_defs 中 `DefaultValue` 载荷（FLOAT 列 Int → Float）。
  - **T6 消费点**：`build_alter_table` AddColumn 臂（:1099-1130）——`default_value: Option<Value>` 局部经同一 `extract_default_value`（:1105）提取，`column_type` 在侧；NOT NULL 无 DEFAULT 既有拒绝（:1113）保持；`default_value` 于 :1130 装入节点。T6 在提取后消费 `validate_default_value`。
  - **错误面先例**：`PlanError::CtasColumnTypeConflict(String)`（`src/parser/error.rs:23`）→ 分类 :84（SQLSTATE 58000）→ 分类表测试行 :188。T4 新变体 `DefaultTypeMismatch(String)` 同型三处（变体 + 分类臂 + 分类表行）。
  - **日期族通道**：`coerce_datetime_write`（`src/executor/datetime.rs:268`，`pub(crate)`）——写入边界强制解析同源，签名 `(value, col_type)` 形态，Err = 解析失败；本 Iteration 预期零改动（仅消费）。
  - **dump 渲染**：`typed_datetime_literal`（`src/cli/lifecycle.rs:473`，`create_table_sql` 于 :373 消费）——日期族 DEFAULT 渲染 `DEFAULT DATE '…'` / `DEFAULT TIMESTAMP '…'`；本 Iteration 零改动（T5 修复的是 restore 侧解析）。
  - **既有 DEFAULT 测试面**（零回归参照）：`tests/subset_insert_test.rs`（6，DEFAULT 应用/跨重启/dump→restore）、`tests/write_type_conformance_test.rs`（13，写入类型门矩阵）、`tests/cli_test.rs` DEFAULT 相关组、`tests/planner_test.rs` 建表组（现 93）。
- Code and Critical Path: `build_create_table`/`build_alter_table`（ddl_dml.rs）→ `extract_default_value`/新 `validate_default_value`（同文件）→ `PlanError::DefaultTypeMismatch`（error.rs，经既有计划错误分类 → CLI exit 3）→ catalog 持久化通路零改动（归一仅改计划期载荷）；restore 链路 TypedString 臂 → String 目录形态（与手写 String 声明同态）→ 既有写入门 coerce 通路。

**Implementation Guidance**

实施顺序 T4 → T5 → T6（共享 `validate_default_value` 单点；T5 解析失败复用 T4 错误变体）→ T7 文档收尾。

- D6 归一规则（`validate_default_value(col_name, col_type, value) -> Result<Option<Value>, PlanError>`，`None` = 原样、`Some(v)` = 归一）：`Value::Null` → `Ok(None)`；`(Float, Value::Int(n))` → `Ok(Some(Value::Float(n as f64)))`；`(Date|Timestamp, _)` → `coerce_datetime_write` 验证（Ok → `Ok(None)` String 形态保留，Err → `Err(DefaultTypeMismatch)`）；其余同型 → `Ok(None)`；其余异型 → `Err(DefaultTypeMismatch(...))`（点名列名、期望类型名、实际值类型名——`column_type_name` 同款大写形态）。
- D7 TypedString 臂：`DataType::Date` / `DataType::Timestamp(None)`（`Timestamp(_, None)` 形态以 sqlparser 0.44 实际变体为准——Act 实现时按编译器提示穷尽）→ 构造 `Value::String(value)` 经 `coerce_datetime_write` 验证后返回；其余 data_type → 既有 `UnsupportedValue` 不变。
- T7 文档面：SKILL.md 命令行功能目录增 import stdin/自动建表/`:memory:` 豁免与拒绝面、DEFAULT 声明语义；README 双语对应段逐条核验；`rtsql --help` / `rtsql import --help` 抽样自动执行。

**Task Contracts**

### T4: CREATE TABLE 声明期 DEFAULT 校验/归一

- Requirement/Scenario: sql-write-surface ADDED「声明期 DEFAULT 类型一致」（BOOL 拒绝 / 跨类型矩阵 / FLOAT 升格归一 / 日期族解析 / DEFAULT NULL 豁免 / 既有零回归）
- Depends on: None
- Targets: `src/parser/planner/ddl_dml.rs::build_create_table`（T8 守卫后新校验循环）+ 新 `validate_default_value`（同文件）；`src/parser/error.rs`（新变体 + 分类臂 + 分类表测试行）
- Current behavior: 任意字面量 DEFAULT 被接受并持久化（Bool 列 `DEFAULT 1` 建表成功），应用默认值的 INSERT 由写入类型门拒绝——声明/写入分裂
- Required behavior: 计划期校验/归一——NULL 豁免；FLOAT×Int 升格 `Float(n as f64)` 归一持久化（dump 渲染跟随 `2.0`）；日期族 String 经 `coerce_datetime_write` 验证（非法日期 `DEFAULT '2026-13-99'` 点名拒绝）；其余跨类型（BOOL 列 DEFAULT 1、INT 列 DEFAULT 'x'、STRING 列 DEFAULT 1、INT 列 DEFAULT 1.5）`DefaultTypeMismatch` 点名拒绝（列名/期望/实际）；拒绝零副作用（表不存在）
- Preserve: 既有建表与既有 DEFAULT 场景（无 DEFAULT、类型一致、`DEFAULT NULL`）逐字节零回归；catalog 持久化通路零改动；T8 守卫行为零变化
- Forbidden: 不动写入类型门运行期语义；不动 `create_table_with_constraints` 签名；不给 `DEFAULT <表达式>`（非字面量）开支持
- Test witness: 新 `tests/default_declaration_test.rs` 建表矩阵 + `tests/planner_test.rs` 计划期单元——**RED**：`CREATE TABLE t (b BOOL DEFAULT 1)` 现状建表成功（分裂形态可达，实现前直证在案由 proposal 实证承担）
- GREEN condition: 拒绝/归一矩阵全绿 + 既有建表/DEFAULT 组零回归
- Verification: `cargo test --test default_declaration_test`；`cargo test --test planner_test`；`cargo test --test subset_insert_test`；`cargo test --test write_type_conformance_test`
- Stop when: 归一载荷改写与 catalog 持久化通路冲突（如 DefaultValue 无法在不改签名前提下归一）——返回 Plan

### T5: TypedString 臂与 dump→restore 往返修复

- Requirement/Scenario: sql-write-surface ADDED「TypedString 声明与 dump→restore 往返修复」「非日期族 TypedString 维持拒绝」
- Depends on: T4（解析失败复用 `DefaultTypeMismatch`）
- Targets: `src/parser/planner/ddl_dml.rs::extract_default_value`（catch-all 前新 TypedString 臂）
- Current behavior: `DEFAULT DATE '2026-01-02'` restore 报 `Unsupported value type` exit 3——dump 渲染的日期族 DEFAULT 不可重建（实证缺陷）
- Required behavior: Date/Timestamp TypedString 经 `coerce_datetime_write` 验证后返回 `Value::String(value)`（目录形态与手写 String 声明同态）；dump→restore 往返对日期族 DEFAULT 成立（restore 后 schema 含 DEFAULT、省略列 INSERT 行为与原库一致）；非日期族 TypedString 维持 `UnsupportedValue`
- Preserve: dump 渲染（`typed_datetime_literal`）与写入门 coerce 通路零改动；`extract_default_value` 既有三臂（Value/Identifier NULL/负号）行为逐字节
- Forbidden: 不动 `typed_datetime_literal`；不动 catalog 日期族 String 存储形态；不给非日期族 TypedString 开支持
- Test witness: `tests/default_declaration_test.rs` 往返 e2e——**RED**：修复前 restore 失败（proposal 实证序列：建表 → dump → restore 报 `Unsupported value type` exit 3）
- GREEN condition: 往返 e2e 全绿 + 非日期族拒绝见证 + 既有 dump/restore 组零回归
- Verification: `cargo test --test default_declaration_test`；`cargo test --test cli_test`（dump/restore 组）
- Stop when: TypedString 臂与 sqlparser 0.44 变体形态不匹配导致无法穷尽——返回 Plan

### T6: ALTER ADD COLUMN 同款规则

- Requirement/Scenario: sql-write-surface ADDED「ALTER ADD COLUMN 同款规则」
- Depends on: T4（共享 `validate_default_value`）
- Targets: `src/parser/planner/ddl_dml.rs::build_alter_table` AddColumn 臂（:1099-1130，`extract_default_value` 后消费）
- Current behavior: `ALTER TABLE t ADD COLUMN b BOOL DEFAULT 1` 计划期被接受（同款分裂）
- Required behavior: AddColumn 的 DEFAULT 声明适用同款校验/归一——`ADD COLUMN d DATE DEFAULT '2026-01-02'` 建列成功（存量行读回声明默认值）、跨类型点名拒绝零副作用（表结构不变）
- Preserve: NOT NULL 无 DEFAULT 既有拒绝（:1113）零变化；ALTER 既有守卫矩阵（多操作/索引引用/最后一列/事务内）零变化
- Forbidden: 不动 ALTER DROP COLUMN 面；不给 ADD COLUMN 增加同名列冲突守卫（design D10 明示范围外）
- Test witness: `tests/default_declaration_test.rs` ALTER 矩阵——**RED**：`ADD COLUMN b BOOL DEFAULT 1` 现状成功（实现前直证）
- GREEN condition: ALTER 拒绝/接受矩阵全绿 + `tests/alter_table_test.rs`（16）零回归
- Verification: `cargo test --test default_declaration_test`；`cargo test --test alter_table_test`
- Stop when: AddColumn 的 column_type 与 default_value 交汇点无法在不改节点签名前提下消费校验——返回 Plan

### T7: 文档同步与 validate 收尾

- Requirement/Scenario: 文档任务（无 spec 映射，验收 = 文档与实现逐条一致）
- Depends on: T4, T5, T6（文档消费全 change 最终语义）
- Targets: `rtsql-docs/SKILL.md`（命令行功能目录）、`README.md`/`README.zh-CN.md`（对应段）
- Current behavior: SKILL.md/README 为 MS28 收尾后状态——import 仍记载「目标表必须已存在、不收 stdin、`:memory:` 拒绝」，无 DEFAULT 声明语义段
- Required behavior: import stdin `-` 通道、自动建表（类型推导/首列隐式主键/守卫拒绝/零副作用）、`:memory:` import 豁免与 `--key` 拒绝、DEFAULT 声明期校验/归一语义、日期族 DEFAULT 往返——逐条与实现一致；README 双语对应段同步
- Preserve: 文档既有结构与其余段落零改动（MS28 收尾内容不重写）
- Forbidden: 不变更实现行为迁就文档；不引入实现未含的文档承诺
- Test witness: 人工逐条核对 + `rtsql --help` / `rtsql import --help` 抽样自动执行（输出与文档陈述一致）
- GREEN condition: SKILL.md 与 README 双语逐条一致；`openspec validate 2026-10-01-ms29-import-input-and-default-declaration` 通过
- Verification: `openspec validate <change>`；抽样命令执行
- Stop when: 文档与实现出现无法用措辞调和的语义分歧——返回 Plan

**Invariants**

- 写入类型门（sql-write-surface R2）运行期语义零变化——本 Iteration 只加声明期校验/归一，门本身不动。
- 既有 DEFAULT 接受面（无 DEFAULT、类型一致、`DEFAULT NULL`）的持久化、应用、dump 渲染逐字节零回归；FLOAT 升格归一只影响新声明（存量 catalog 数据零触碰）。
- dump 渲染格式（含 `typed_datetime_literal`）零变更；restore 链路修复仅加解析臂。
- `coerce_datetime_write`/`datetime.rs` 零改动（仅消费）；CTAS 通道与 T8 守卫零变化。
- 错误面：声明期拒绝经既有计划错误映射 exit 3；拒绝零副作用（表/列不存在于失败结果）。
- Iteration 000 交付面（import 输入面）零回退。

**Non-goals**

- import 攒批提交（MS22）；CHECK/FK 强制执行；`DEFAULT <表达式>` 支持；dump 渲染格式变更；`ALTER TABLE ADD COLUMN` 同名守卫；JSON/BLOB 承载（I069）。

**Acceptance**

1. 建表声明期矩阵：BOOL/跨类型拒绝点名零副作用、FLOAT 升格归一（读回 2.0 + dump `DEFAULT 2.0` + 重开/dump→restore 一致）、日期族 String 解析通过与非法拒绝、`DEFAULT NULL` 豁免（sql-write-surface ADDED；T4）。
2. 日期族 DEFAULT dump→restore 往返修复：restore 成功重建、schema 含 DEFAULT、行为与原库一致；非日期族 TypedString 维持拒绝（sql-write-surface ADDED；T5）。
3. ALTER ADD COLUMN 同款：跨类型拒绝零副作用、日期族建列成功存量行读回默认值（sql-write-surface ADDED；T6）。
4. 既有面零回归：既有 DEFAULT 全部用例、subset_insert/write_type_conformance/alter_table/planner 建表与 CTAS 组、Iteration 000 交付面（T4, T5, T6）。
5. 全量分批零回归（基线 1672 不降）+ SKILL.md/README 双语逐条一致 + `openspec validate` 通过（T4-T7）。

**Verification**

- `cargo test --test default_declaration_test`（1-3）、`cargo test --test planner_test`（1/4 计划期单元）、`cargo test --test subset_insert_test` + `cargo test --test write_type_conformance_test` + `cargo test --test alter_table_test` + `cargo test --test cli_test`（4）、`cargo test --test import_input_surface_test`（4，Iteration 000 面不回退）、全量分批 M19（5）、`openspec validate` + 文档抽样（5）。
- RED 见证：T4（BOOL DEFAULT 1 建表成功）、T5（restore `Unsupported value type` exit 3——proposal 实证序列）、T6（ADD COLUMN BOOL DEFAULT 1 成功），记录于 Act Response。
- 人工步骤：T7 文档逐条核对（最短操作：对照 SKILL.md/README 陈述与 `--help` 实际输出，所见即判定）。

**Gate 2 Readiness**

其余检查项 PASS（无 Missing requirement、无未批准裁剪、追踪完整、验证充分、无身份型证据工程、Evidence 模式 none）。调查闭合：本 Cycle 展开时全部行号锚点经读码复核（T8 守卫合入后 :870-879 新锚点已计入）；T4-T7 消费点、错误面先例、日期族通道、dump 渲染与既有测试面均已定位。Gate 2 承接 change 级批准（2026-10-01 用户原话「批准，交 openspec-act」，覆盖 Gate 1 + 全部 Iteration 执行就绪；replan 未改动本 Iteration 范围——父 Cycle Review「001 = T4-T7 不变」）。

**Persisted Evidence**

- Mode: none

（Act Response 记录命令、决定性输出与退出码即足；全部验证可低成本重跑。）

## Act Response

- Status: reported

**Implemented**

T4/T5/T6/T7 全部完成并 GREEN。

- `src/parser/error.rs`：新变体 `DefaultTypeMismatch(String)`（CtasColumnTypeConflict 同型）+ Display 臂（`DEFAULT type mismatch: {detail}`）+ SQLSTATE 分类表测试行（58000 兜底）。
- `src/parser/planner/ddl_dml.rs`：新模块级自由函数 `validate_default_value(col_name, col_type, value) -> Result<Option<Value>, PlanError>`（D6 单点：NULL 豁免；FLOAT×Int→升格 `Float(n as f64)`；Date/Timestamp→经 `coerce_datetime_write` 验证〔Ok→`Ok(None)` String 形态保留，Err→`DefaultTypeMismatch`〕；四组同型放行；其余异型 `DefaultTypeMismatch` 点名列名/期望/实际）+ `column_type_name`/`value_type_name` 两个错误面大写名 helper；`build_create_table` 于 T8 守卫后、PK 提取前新增校验/归一循环（原地改写 `DefaultValue` 载荷）；`extract_default_value` 于 catch-all 前新增 `Expr::TypedString` 臂（Date / `Timestamp(_, TimezoneInfo::None)` → 验证后返回 `Value::String`，其余 data_type 维持 `UnsupportedValue`）；`build_alter_table` AddColumn 于 DEFAULT 提取后消费同一单点（NOT NULL 无 DEFAULT 既有拒绝保持）。
- `tests/default_declaration_test.rs`（新，14 用例）：T4 建表矩阵 7（BOOL×INT 拒绝 / 跨类型矩阵 / 非法日期拒绝 / FLOAT 升格跨重启 / 日期族 String 接受 / DEFAULT NULL 豁免 / BOOL 同型对照）+ T5 4（Date·Timestamp TypedString 接受 / 非日期族拒绝 / dump→restore 往返 e2e / FLOAT 归一 dump 渲染 e2e）+ T6 2（ALTER BOOL×INT 拒绝零副作用 / DATE 建列存量行读回）；`tests/planner_test.rs` +3 计划期单元（计划期拒绝 / 归一载荷 `Float(2.0)` / NULL 与同型零回归）。
- T7 文档：`rtsql-docs/SKILL.md`（`:memory:` 拒绝句 import 豁免注记、生命周期表 import 行、SQL 能力表 Write 行、ALTER 条目、Import CSV 段重写——stdin / 自动建表推导与首列隐式主键 / `:memory:` 组合）；`README.md`/`README.zh-CN.md`（命令表 import 行 + 写面「声明默认值」新条目）。

**Deviations from Plan**

- T5 TypedString 臂错误文案：解析失败复用 T4 变体达成，但臂内无列名上下文，detail 为 `invalid {DataType} literal '{value}'`（validate 层路径均携列名）——措辞非实质差异。
- 日期族验证路径的 storage 类型构造：`coerce_datetime_write` 收 storage `page_format::ColumnType`，planner `ColumnType`（executor 值型）经局部 match 映射 Date/Timestamp 两臂——Plan Context 未点名该双类型面，映射零逻辑，非实质。

**Self-Review**

- Gate 3 spec compliance：sql-write-surface ADDED 两 Requirement 全场景逐条对齐（BOOL 拒绝 / 跨类型矩阵 / FLOAT 升格含 dump 渲染 `DEFAULT 2.0` / 日期族解析与非法拒绝 / TypedString 往返 / 非日期族维持拒绝 / DEFAULT NULL 豁免 / ALTER 同款含存量行读回）；Invariants 全部保持——写入类型门运行期零改动、dump 渲染格式零变更（`typed_datetime_literal` 未触碰）、`coerce_datetime_write`/datetime.rs 零改动、T8 守卫与 CTAS 通道零变化、Iteration 000 交付面 12/12 不回退；Forbidden 未触碰（未动 `create_table_with_constraints` 签名、未开 `DEFAULT <表达式>`、未给 ADD COLUMN 加同名守卫）。
- Gate 3 code quality：diff 限于计划面文件；无计划外修改；一处自检发现已修复（新增测试未用 helper `assert_count` 移除，clippy 清零）；命名与注释沿 MS 编号惯例。
- 遗留 Minor：无。

**Verification Evidence**

- RED（实施前，`cargo test --test default_declaration_test`）：**5 passed; 9 failed**（exit 1）——T4 三拒绝形态建表成功（`AffectedRows`）、T5 TypedString `Unsupported value type`、restore 往返 `Plan error: Unsupported value type` exit 3、FLOAT dump 渲染 `DEFAULT 2`、T6 ALTER 成功——全与契约预测一致。夹具一次修正（重开用例以块作用域释放 advisory 锁——`close()` 不释放锁）。
- GREEN 目标面：`cargo test --test default_declaration_test` → **14 passed; 0 failed**（exit 0）；`cargo test --test planner_test` → **96 passed; 0 failed**（含 T4 +3）。
- 受影响边界：`subset_insert_test` 6 / `write_type_conformance_test` 13 / `alter_table_test` 16 / `import_input_surface_test` 12 / `cli_test` 117 passed, 2 ignored——全部 0 failed（既有 DEFAULT、dump/restore、import 组与 Iteration 000 面零回归）。
- 静态检查：`cargo clippy --tests` 清零（唯一告警为上述已移除的未用 helper）。
- 全量分批 M19：`cargo test --lib` 354 + 集成 106 目标分三批（437 + 246 + 652）→ **合计 1689 passed; 0 failed; 2 ignored**（= 基线 1672 + 净增 17〔default_declaration 14 + planner 3〕，精确吻合）。
- T7 抽样自动执行（真实二进制，`/tmp` 现场）：`import :memory: t - --csv` rc=0；`--key` 组合 Usage 拒绝；类型冲突 rc=1 且 schema 无该表（零副作用）；INT 首列重复 fail-fast rc=3；`BOOL DEFAULT 1` → `Plan error: DEFAULT type mismatch: column 'b': expected BOOL, got INT`；FLOAT 列 `DEFAULT 2` 建表后省略列读回 `2.0`；`rtsql --help` / `rtsql import --help` 输出与文档陈述一致；`openspec validate 2026-10-01-ms29-import-input-and-default-declaration` → **valid**。
- T7 人工步骤：SKILL.md / README 双语与实现逐条核对（含 Iteration 000 import 面：stdin `-`、自动建表推导与冲突拒绝、`:memory:` 豁免与 `--key` 拒绝、DEFAULT 声明语义、日期族往返）；核对中修正两处初稿不准确表述（数字×文本混合为冲突拒绝而非 TEXT、首列恒为隐式主键）。

**Persisted Evidence**

None required（Mode: none 维持；全部验证可低成本重跑，Act Response 记录已足）。

**Experience Candidates**

None

**Remaining Issues**

无

**Risks and Notes**

- FLOAT 升格归一使新声明 catalog 存 `Float(2.0)` 而非 `Int(2)`——若存量测试夹具有 FLOAT 列 Int 字面量 DEFAULT 并断言 dump 渲染 `DEFAULT 2`，全量分批会 RED：就地按新语义校准并在 Response 记录（非实质）；无法校准的实质冲突返回 Plan。
- sqlparser 0.44 的 `DataType::Timestamp` 变体形态（`Timestamp(_, None)`）以实现时编译器穷尽提示为准——非实质。
- `Bool` 列 `DEFAULT true`（Bool 字面量）为同型接受面——矩阵中作为对照见证（防止「拒绝矩阵」误伤同型）。
- 非实质留 Act：`validate_default_value` 的签名形态与放置（自由函数 vs 关联函数）、错误文案措辞、测试函数命名。

## Plan Review

- Review Result: accepted

**Findings**

独立审计（读码 + 只读 git diff 逐 hunk 核对 Act 声明，2026-10-01）：

1. **T4 声明期校验/归一核实成立**：`validate_default_value`（ddl_dml.rs:54-98，D6 单点）——NULL 豁免、`(Float, Int)` → `Some(Float(n as f64))`、Date/Timestamp 经 `coerce_datetime_write` 验证（storage 类型局部映射零逻辑）、四组同型放行、其余异型 `DefaultTypeMismatch` 点名列名/期望/实际；`build_create_table` 校验/归一循环（:972-985）置于 T8 守卫（:961-970）后、`extract_primary_key`（:988）前，`*value = normalized` 原地改写载荷——与契约位置与语义逐条一致。
2. **T5 TypedString 臂核实成立**：`extract_default_value` 新臂（:820-838）位于 catch-all 前，`DataType::Date` / `Timestamp(_, TimezoneInfo::None)` → `coerce_datetime_write` 验证后返回 `Value::String`（与手写 String 声明同态），其余 data_type 提前返回既有 `UnsupportedValue`——非日期族拒绝面逐字节保持。
3. **T6 消费点核实成立**：`build_alter_table` AddColumn 于 DEFAULT 提取后（:1219-1225）消费同一单点，NOT NULL 无 DEFAULT 既有拒绝（:1228-1240）原序保持。
4. **错误面核实成立**：`PlanError::DefaultTypeMismatch(String)`（error.rs 三 hunk 纯新增 +7——变体/Display/分类表测试行 58000），CtasColumnTypeConflict 同型。
5. **测试见证核实成立**：`default_declaration_test.rs` 14 函数 = T4 矩阵 7 + T5 4 + T6 2，与 RTM/tasks 逐条对应；`planner_test.rs` 单 hunk 纯新增 +143（T8 3 例已于 Iteration 000 accepted + T4 3 例 :1073/:1093/:1119，断言与 Act 声明逐条吻合——计划期拒绝点名 / 归一载荷 `Float(2.0)` / NULL 与同型零回归），既有测试零修改。`error.rs`/`planner_test.rs`/`ddl_dml.rs` 全部 diff hunk 均为纯新增（无删除无改写）；写入类型门运行期（executor 面）、`typed_datetime_literal`、`coerce_datetime_write`/`datetime.rs`、CTAS 通道零触碰——Invariants 全部保持。
6. **T7 文档核实成立**：SKILL.md `:memory:` 豁免注记（:128）、生命周期表 import 行（:153）、SQL 能力表 Write 行（:233）、ALTER 条目（:265）、Import CSV 段（:290-312，stdin/自动建表/:memory: 组合/缺库 exit 1/`--key` exit 2）；README 双语 import 行（:177）与「声明默认值」段（:261，中英对称）——与实现语义逐条一致。
7. **Deviations 2 项均核实为非实质**：TypedString 臂错误文案无列名上下文（`invalid {DataType} literal '{value}'`）——delta spec 对日期族解析失败仅要求「计划期点名拒绝」，列名/期望/实际的点名要求属异型分支（已满足）；storage/planner 双 `ColumnType` 局部映射（:71-74/:826-831）零逻辑。
8. **Minor 1（记录不返工）**：FLOAT 归一的 dump→restore 往返无直接 e2e 见证（spec 场景 S3 THEN 含该形态；现有见证 = 读回 2.0 + 重开〔`float_column_int_default_normalized_across_restart`〕+ dump 渲染 `DEFAULT 2.0`〔`float_default_dump_renders_normalized_literal`〕）。独立查证：restore 链各环节均有独立见证——Float 字面量解析（既有套件 + `INT DEFAULT 1.5` 拒绝测试断言 FLOAT 类型名）、同型放行（Bool/String 对照见证）、归一持久化与读回（test 4）；同型 dump→restore 机器由日期族往返 e2e 整链见证，差异仅 DEFAULT 字面量文本。与本 change Iteration 000 Plan Review Minor 1（FLOAT 形态由代码结构 + 同臂形态执行见证共同保证）同先例，不构成 Acceptance gap。
9. **Minor 2（记录不返工）**：Act Response 自述「clippy `--tests` 清零（唯一告警为已移除的未用 helper）」与 GREEN 后复核——静态检查结论采信；全量后无后续行为面变更（工作区 git status 与 Act 声明面一致），无重跑触发情形。

**Acceptance Gaps**

无——五项 Acceptance 全部满足：T4 矩阵（BOOL/跨类型拒绝点名零副作用、FLOAT 升格读回 2.0 + dump `DEFAULT 2.0` + 重开、日期族解析通过与非法拒绝、DEFAULT NULL 豁免）；T5 往返修复（restore 重建 + schema 含 DEFAULT + 行为一致、非日期族维持拒绝）；T6 同款（跨类型拒绝零副作用、日期族建列存量行读回）；既有面零回归（subset_insert 6 / write_type_conformance 13 / alter_table 16 / import_input_surface 12 / cli_test 117+2i 全绿）；全量 1689 = 基线 1672 + 净增 17（default_declaration 14 + planner 3）精确吻合；SKILL.md/README 双语逐条一致 + `openspec validate` 通过。

**Evidence**

- 独立读码：`src/parser/planner/ddl_dml.rs:29-98`（helpers + validate_default_value）、`:820-838`（TypedString 臂）、`:961-985`（T8 守卫 + T4 循环及与 PK 提取的执行序）、`:1217-1240`（T6 消费 + NOT NULL 拒绝原序）；`src/parser/error.rs:24-26/:90-92/:195`（变体/Display/分类行）。
- 独立 diff 核对（只读 git）：error.rs 3 hunk +7 纯新增；planner_test.rs 单 hunk +143 纯新增；ddl_dml.rs 6 hunk 全部纯新增（import/T4 helpers/T5 臂/T8+T4 循环/T6 消费），无删除无既有代码改写。
- 测试见证独立阅读：`tests/default_declaration_test.rs` 14 函数逐条对照 delta spec 场景（S1/S2/S3/S4/TypedString/非日期族/S7/ALTER）；文档关键行独立 grep 核对（SKILL.md :128/:153/:233/:265/:290-312；README 双语 :177/:261）。
- Act Response 验证结论采信（来源：001-default-declaration/000-initial.md › Act Response › Verification Evidence，2026-10-01 本工作区分批执行；覆盖未变化——结论后工作区 git status 与 Act 声明面一致，无后续行为面变更）：RED 5/9 → GREEN 14/14、planner_test 96/0、受影响边界五文件 0 failed、全量 1689/0/2、clippy 清零、T7 真实二进制抽样、`openspec validate` valid。无重跑触发情形（无矛盾、覆盖面未变化、无失败、无第三次失败）。

**Follow-up Decision**

接受（accepted）：T4/T5/T6/T7 按契约交付且独立核实一致；Act 验证证据完整新鲜（RED 见证、GREEN、受影响边界、全量分批对账、静态检查、文档抽样与人工核对）；2 项 Deviations 均非实质且已记录；2 项 Minor finding 不阻塞 Acceptance（FLOAT 往返由分环节见证 + 同 change 000 先例共同保证），按 iteration-planning 分类「只有不阻塞 Acceptance 的 Minor finding → accepted，记录不返工」。Iteration 001 完成——change 两 Iteration（000、001）全部 accepted，无剩余 Iteration。

**Next Cycle**

None

**Next Iteration**

None（Map 最后一个 Iteration 已完成；change 进入收尾——规格合并与归档由 openspec-docs-maintainer 执行）
