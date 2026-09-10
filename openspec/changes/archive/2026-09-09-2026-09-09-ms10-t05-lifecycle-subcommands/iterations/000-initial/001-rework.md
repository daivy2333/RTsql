# Iteration 000 / Cycle 001-rework: 约束持久化通道与 R-schema S1 收口

## Plan Context

- Status: ready（2026-09-09 用户批准实施，Gate 2 通过）
- Iteration: 000-initial
- Cycle: 001-rework
- Cycle Type: rework
- Parent cycle: [000-initial.md](000-initial.md)（blocked；Review Result: rework-required，PLAN-OMISSION）

**Iteration Scope**

- Change tasks: T5（经由 repair items，Iteration Map 不变）
- Depends on: None（T1-T4 已在父 Cycle 完成且全绿，本 Cycle 不触碰其产出）
- Stable baseline: 与父 Cycle 相同 + R-schema S1 转绿（schema/dump 输出真实持久化约束）
- Verification boundary: `cargo test --all` 全绿（含父 Cycle 遗留的 `test_schema_outputs_ddl`）；clippy/fmt/validate 全 0/PASS；既有 671+ 测试零修改
- Diagnostic boundary: `src/executor/create_table.rs`、`src/storage/data/table_manager.rs`、`src/cli/lifecycle.rs`（仅当渲染缺陷时）、`tests/cli_test.rs`
- Deferred tasks: T6, T7, T8, T9（Iteration 001）

**Cycle Scope**

- Trigger: rework-required（父 Cycle Plan Review：PLAN-OMISSION，阻塞 R-schema S1）
- Acceptance gaps: R-schema S1「表结构 DDL 输出」——schema 输出缺 NOT NULL/UNIQUE（上游建库链丢弃约束，非 DDL 生成器缺陷）
- Repair items: T5-R1（约束持久化通道）、T5-R2（S1 见证转绿 + UNIQUE 往返见证）
- Inherited scope: 父 Cycle 已完成的 T1-T4 全部产出（入口分发/resolve helper/new/list）保持冻结；DDL 生成器 `create_table_sql` 及其 2 lib 单测已正确、不动；schema 骨架（scan/排序/输出流程）不动
- Excluded scope: NOT NULL/UNIQUE 的运行时强制（INSERT/恢复路径校验）——明确不做（见设计决策 1）；`Database::create_table`/`TableManager::create_table` 公开签名变化——明确不做（见设计决策 2）；planner 层修改（`extract_column_constraints`/`build_create_table` 已正确产出约束，不动）；dump/restore/import（Iteration 001）

**Objective**

SQL 建库链将 NOT NULL/UNIQUE 约束持久化到 catalog 现有字段（写入路径今日硬编码 false），使 `rtsql schema`（与 Iteration 001 的 dump）输出与持久化面一致的约束信息；R-schema S1 转绿，既有全部行为零回归。

**Background**

父 Cycle T5 落地 DDL 生成器与 schema 命令后发现：catalog 序列化格式支持 not_null/unique（catalog.rs:651-672），但 SQL 建库链在执行器→存储边界丢弃约束、写入时硬编码 false——约束自 MS07-T01 落地 catalog 起即为「结构在场、语义缺席」。父 Cycle 契约 Forbidden 禁改 storage 层，Act 正确阻塞。父 Cycle Review 分类 PLAN-OMISSION（Plan 未核实写入路径），需要新执行契约。

**Current Baseline**

- revision `a5b0a5f` + 父 Cycle 工作树实施（未 commit：`src/cli/mod.rs`、`src/cli/resolve.rs`、`src/cli/lifecycle.rs` 新增、`tests/cli_test.rs`）
- 独立验证（2026-09-09，父 Cycle Review）：`cargo test --all` → lib 192/0，cli_test 34/1/2（唯一失败 `test_schema_outputs_ddl`）；clippy 0 / fmt 干净 / validate PASS
- T1-T4 产出与既有 25 用例零修改已核实

**Current-State Evidence**

- **约束丢弃链（父 Cycle Review 已独立复核）**：
  1. planner 侧正确：`extract_column_constraints`（`src/parser/planner/ddl_dml.rs:170-198`）产出 `ColumnConstraint::{NotNull, Unique}`；`build_create_table`（`ddl_dml.rs:280`，签名即含 constraints）将约束装入 `CreateTableNode.columns: Vec<ColumnDef>`；
  2. `ColumnDef`（`src/executor/plan.rs:166-230`）持 `constraints: Vec<ColumnConstraint>`，`to_schema_column()` 把 NotNull/Unique 装入 `storage::data::ColumnSchema` 对应字段（plan.rs:205-224）；
  3. **丢弃点**：`CreateTableExecutor`（`src/executor/create_table.rs:46-54`）对每个 ColumnDef 调 `to_schema_column().to_tuple()`，压成 `(String, ColumnType)`，约束丢弃；
  4. **无通道**：`TableManager::create_table(name, columns: Vec<(String, ColumnType)>, pk)`（`table_manager.rs:209-214`）签名无约束参数；
  5. **硬编码**：catalog 列行写入 `not_null: false, unique: false`（`table_manager.rs:282-293`）。
- **调用方普查（决定 API 策略）**：`Database::create_table` 在 tests/benches 有 76 处调用——公开签名必须保持；`TableManager::create_table` 直调 5 处：`database.rs:101`（Database 委托）、`executor/create_table.rs:71`（SQL 路径，唯一需要真实约束的调用方）、`transaction/manager.rs:335/530`（src 内测试）、`data_page.rs:161`（src 内测试）。
- **catalog 写入点**：`create_table` 内 `catalog_cols` 构造（table_manager.rs:282-293）→ `catalog.insert_table(&catalog_row, &catalog_cols)`；序列化端 `serialize_catalog_column_row`（catalog.rs:652-672）已按字段写 not_null/unique 字节，读取端 `scan_columns` 反序列化——**两端就绪，只差传入真实值**。
- **运行时语义现状**：约束解析后全链无任何强制（INSERT 不校验 NOT NULL，UNIQUE 非 PK 无索引）——持久化真实值不改变任何执行路径行为，仅 catalog 元数据变真。
- **restore 语义推论**：restore（Iteration 001）经 SQL `CREATE TABLE` 重建 → 走同一 executor 路径 → 约束自然随库持久化，无需额外处理。

**Relevant Code**

- `src/executor/create_table.rs` — `CreateTableExecutor::execute`（约束提取与传参改造点）
- `src/storage/data/table_manager.rs` — `create_table`（委托壳）+ 新 `create_table_with_constraints`（catalog 写入填充真实值）
- `src/executor/plan.rs` — `ColumnDef.constraints`（只读复用，零修改）
- `src/cli/lifecycle.rs` — schema/dump 消费端（零修改，S1 转绿后自然通过）
- `tests/cli_test.rs` — `test_schema_outputs_ddl`（保持 RED 现状为本 Cycle 起点见证）

**Critical Path**

SQL 建表：planner（约束已入 plan）→ `CreateTableExecutor`：从 `ColumnDef.constraints`（或 `ColumnSchema` 字段，非实质）提取 `(not_null, unique)` → 构造 `Vec<(String, ColumnType, bool, bool)>` → `create_table_with_constraints` → catalog 行填真实值 → `scan_columns` 读回 → schema 输出含约束。`Database::create_table` 与 `TableManager::create_table` 旧签名 → 委托新方法、flags=false → 76+5 处既有调用方行为逐字不变。PK 不走此通道（独立持久化于 `pk_column`，现状正确）。

**Implementation Guidance**

顺序：T5-R1（存储通道 + 执行器透传）→ T5-R2（见证转绿）。`create_table_with_constraints` 的列参数建议 `Vec<(String, ColumnType, bool, bool)>`（name, type, not_null, unique——顺序与 catalog 字段对应）；旧 `create_table` 保留签名并委托（每个列 false/false）。执行器侧约束提取用 `constraints.iter().any(|c| matches!(c, ColumnConstraint::NotNull | ColumnConstraint::Unique { .. }))`（`ColumnConstraint` 来自 `crate::executor`，`Unique { is_primary }` 结构体变体注意只取 `is_primary: false`——PK 列已在 DDL 生成器经 `pk_column` 独立渲染，UNIQUE 标志若含 PK 列会双重渲染，提取时排除或依赖 planner 只对非 PK 产出 Unique，Act 以测试锁定行为）。src 内两处测试直调（transaction/manager.rs、data_page.rs）走旧签名委托，零修改。本修复不新增错误变体、不改任何错误路径。

**Behavioral Change**

| 场景 | 当前 | 目标 |
|---|---|---|
| `CREATE TABLE t (a INT NOT NULL, b STRING UNIQUE)` 建库后 `rtsql schema` | 列定义无 NOT NULL/UNIQUE（catalog false） | 输出含 `NOT NULL`/`UNIQUE`（catalog 真实值） |
| `Database::create_table` / `TableManager::create_table` 直调路径 | catalog false | 逐字不变（显式 false） |
| INSERT/恢复行为 | 无约束强制 | 逐字不变（元数据持久化，不引入强制） |
| 既有测试 | — | 零修改全绿 |

**Change Surface**

| Task/Repair | Requirement/Scenario | File/Symbol | Current Responsibility | Planned Change |
|---|---|---|---|---|
| T5-R1 | R-schema/S1 | `src/executor/create_table.rs::execute` | `to_tuple()` 压平丢约束 | 从 ColumnDef 提取 (not_null, unique) 传入新方法 |
| T5-R1 | R-schema/S1 | `src/storage/data/table_manager.rs::create_table` | 无约束通道 + 硬编码 false | 保留签名，委托新方法（false/false） |
| T5-R1 | R-schema/S1 | `src/storage/data/table_manager.rs`（新增 `create_table_with_constraints`） | — | 真实约束写 catalog 现有字段 |
| T5-R2 | R-schema/S1 | `tests/cli_test.rs::test_schema_outputs_ddl` + 新增 UNIQUE 往返见证 | RED 缺口见证 | 转绿 + UNIQUE 建库→schema 往返断言 |

**Task Contracts**

### T5-R1: 约束持久化通道（executor → table_manager → catalog）

- Requirement/Scenario: R-schema / S1（上游数据面）
- Depends on: None（父 Cycle T5 产出已冻结为调用方）
- Targets: `src/executor/create_table.rs::execute`、`src/storage/data/table_manager.rs`（`create_table` + 新增 `create_table_with_constraints`）
- Current behavior: 执行器压平丢约束；catalog 列行 not_null/unique 恒 false
- Required behavior: SQL 建库（executor 路径）将每列 NotNull/Unique 约束真实写入 catalog 既有字段；`Database::create_table` 与 `TableManager::create_table` 旧签名调用方（76+5 处）行为逐字不变（显式 false 委托）
- Required changes: 新增 `create_table_with_constraints(name, columns: Vec<(String, ColumnType, bool, bool)>, pk)`（catalog 写入消费 flags）；旧 `create_table` 委托；执行器提取约束并切换调用
- Preserve: catalog 序列化/读取端零修改；planner 零修改；错误路径与错误变体零变化；PK 独立持久化通道（`pk_column`）不动；`Database::create_table` 公开签名不动
- Forbidden: 不做运行时强制（INSERT/恢复校验）；不改 `ColumnSchema`/`ColumnDef` 结构；不动 `scan_columns`/DDL 生成器/schema 命令实现
- Test witness: 变更前 RED——`cargo test --test cli_test test_schema_outputs_ddl`（父 Cycle 遗留缺口见证）；lib 新增单测：`create_table_with_constraints` 经内存 catalog 构造后 `scan_columns` 读回断言 not_null/unique 真实值（或经 Database 级 SQL 建库 + catalog 读回，Act 按夹具成本选择，lib 级优先）
- GREEN condition: 新单测绿；执行器路径经集成见证（T5-R2）确认端到端
- Verification: `cargo test --lib`（exit 0）
- Stop when: 约束提取遇到 planner 未产出的约束形态（实质冲突 → Blocker Handoff）

### T5-R2: R-schema S1 转绿与 UNIQUE 往返见证

- Requirement/Scenario: R-schema / S1（验收缺口本体）
- Depends on: T5-R1
- Targets: `tests/cli_test.rs::test_schema_outputs_ddl`（既有 RED）+ 新增 UNIQUE 见证用例
- Current behavior: `test_schema_outputs_ddl` RED（输出缺 NOT NULL）
- Required behavior: NOT NULL 断言转绿；新增集成用例：SQL 建 `UNIQUE` 列 → `rtsql schema` 输出含 `UNIQUE`（往返见证）；既有 25+10 用例与 lib 全量零回归
- Required changes: 既有用例断言零修改转绿；新增 1 个 UNIQUE 往返集成用例
- Preserve: 既有用例断言零修改；schema 命令实现（父 Cycle 产出）零修改
- Forbidden: 不放宽任何既有断言；不给 DDL 生成器加约束特判
- Test witness: RED→GREEN——先跑 `cargo test --test cli_test test_schema_outputs_ddl` 记录 RED 现状（父 Cycle 已留档），T5-R1 完成后同命令转绿；新增 `test_schema_unique_roundtrip`（RED：UNIQUE 不出现 → T5-R1 后 GREEN）
- GREEN condition: 两用例绿 + `cargo test --test cli_test` 全绿
- Verification: `cargo test --test cli_test`（exit 0）
- Stop when: 转绿依赖放宽断言或修改 schema 实现（→ Blocker Handoff）

**Invariants**

- 父 Cycle 已完成产出（T1-T4、DDL 生成器、schema 骨架）冻结不动。
- 既有全部测试（671 基线 + 父 Cycle 新增 12）零修改全绿；唯一预期状态变化 = `test_schema_outputs_ddl` RED → GREEN 与新增 UNIQUE 用例。
- `Database::create_table`/`TableManager::create_table` 公开签名零变化；planner/pipeline 层零修改；本 repair 的 storage 触碰面**仅限**建库约束通道（table_manager create_table 族 + executor create_table 传参），buffer/WAL/页/事务路径零触碰。
- **约束运行时语义不变**：持久化真实值不引入 INSERT/恢复强制（设计决策 1，Scope Control：spec 场景只要求输出真实持久化约束）。

**Non-goals**

运行时约束强制；`Database::create_table` 签名扩展；DEFAULT 约束持久化（catalog 无字段，维持父 Cycle 边界）；dump/restore/import；非 255 String 长度。

**Acceptance**

- R-schema S1：`test_schema_outputs_ddl` GREEN（NOT NULL 出现）+ `test_schema_unique_roundtrip` GREEN（UNIQUE 往返）。
- T5-R1 lib 单测：catalog 读回真实 flags。
- 零回归：`cargo test --all` 全绿（既有零修改）；clippy/fmt/validate 全 0/PASS。
- 映射：S1 → T5-R1+T5-R2 → create_table.rs/table_manager.rs/lifecycle.rs(只读) → 上述测试。

**Verification**

- `cargo test --lib`（T5-R1 单测 + 既有 192 零回归）
- `cargo test --test cli_test`（S1 转绿 + UNIQUE 用例 + 既有 34 零回归）
- `cargo test --all && cargo clippy --all-targets -- -D warnings && cargo fmt --check && openspec validate 2026-09-09-ms10-t05-lifecycle-subcommands`

**Gate 2 Readiness**

| Dimension | Status | Evidence |
|---|---|---|
| Investigation | PASS | 丢弃链五点独立复核（create_table.rs:46-54 / table_manager.rs:209-214,282-293 / plan.rs:160-230 / ddl_dml.rs:170-198）+ 调用方普查（76+5）+ catalog 两端就绪确认 |
| Design | PASS | 两项契约语义决策已裁定（元数据持久化不引入强制；签名零变化的委托策略）；恢复路径推论（restore 经 SQL 自然持久化） |
| Iteration Plan | PASS | Map 不变；repair items 映射 T5，同 Iteration 目录 |
| Cycle Scope | PASS | gap = R-schema S1 单项；inherited/excluded 明确（父 Cycle 产出冻结） |
| Task Contracts | PASS | T5-R1/T5-R2 自包含（目标符号、行为、见证、停止条件） |
| Traceability | PASS | gap→repair→file→test 链闭合（Acceptance 节） |
| Verification | PASS | RED→GREEN 见证 + 零回归门；无身份型证据 |

**Persisted Evidence**

- Mode: none

`none`：缺口与修复均可低成本重跑验证（单条 cargo test 命令），Act Response 承载即可。Budget: 本 Cycle 最多 5 个文件（含 README），整个 change 最多 20 个；单个文本文件最多 500 行且不超过 256 KiB。

**Risks and Notes**

- planner 对 PK 列可能同时产出 Unique{is_primary:true} 或 PK 列被标 Unique 的形态差异——执行器提取时以 `is_primary: false` 过滤（PK 由 `pk_column` 独立渲染，避免双重）；具体形态以 T5-R1 测试锁定，若 planner 实际不产 PK-Unique 则过滤为无害防御。
- 约束持久化后，旧库（历史建表）catalog flags 恒 false——schema 对旧库输出无 NOT NULL/UNIQUE，**真实反映持久化面**，非回归。
- 非实质留 Act：新方法命名（契约建议 `create_table_with_constraints`）、约束提取的实现位置（ColumnDef.constraints vs ColumnSchema 字段）、单测夹具形态。

## Act Response

- Status: reported（2026-09-09，T5-R1 + T5-R2 完成且全量门通过）

**Implemented**

- **T5-R1（完成）**：`TableManager::create_table_with_constraints(name, columns: Vec<(String, ColumnType, bool, bool)>, pk)` 新增——原 `create_table` 主体整体迁入，唯一行为差异为 `catalog_cols` 构造消费真实 `not_null`/`unique` flags（原硬编码 false）；旧 `create_table` 保留公开签名改为委托壳（每列显式 false/false），5 处直调方零修改（`database.rs:101` 委托、`transaction/manager.rs:335/530` 与 `data_page.rs:161` 测试）；`CreateTableExecutor::next` 经 `to_schema_column()` 既有解析提取 flags 构造 4 元组并切换调用新方法。`TableMeta.columns` 经 `schema_cols` 映射保持 `Vec<(String, ColumnType)>` 运行时形状——约束不进入内存 schema，与 `open_or_init` 恢复路径同形状（设计决策 1：仅元数据持久化，无运行时强制）。
- **T5-R2（完成）**：`test_schema_outputs_ddl` 断言零修改转绿（输出含 `"label" STRING NOT NULL`）；新增 `test_schema_unique_roundtrip`（SQL 建 `name STRING UNIQUE` → `rtsql schema` 输出含 `"name" STRING UNIQUE`），UNIQUE 往返见证落地。

**Changed Files and Symbols**

- `src/storage/data/table_manager.rs`：`create_table`（委托壳，公开签名零变化）、`create_table_with_constraints`（新 pub async；`pk_index` 闭包与 `catalog_cols` 适配 4 元组，守卫/重复检查/PK 校验/catalog 回滚逐行未动）、`tests::create_table_with_constraints_persists_flags`（新 `#[tokio::test]`，含旧签名委托 false/false 断言）。
- `src/executor/create_table.rs`：`CreateTableExecutor::next`（columns 构造 `(name, type, not_null, unique)` 4 元组 + 调用 `create_table_with_constraints`）。
- `tests/cli_test.rs`：`test_schema_unique_roundtrip`（新增）。

**Deviations from Plan**

1. Plan Guidance 预期 `ColumnConstraint::Unique { is_primary }` 结构体变体并建议 `is_primary: false` 过滤防御——实际 `ColumnConstraint::Unique` 为单元变体（`src/executor/plan.rs:230-237`），且 planner `extract_column_constraints` 仅对 sqlparser `Unique { is_primary: false }` 产出该约束（`src/parser/planner/ddl_dml.rs:188-192`），PRIMARY KEY 走 `extract_primary_key` 独立通道，PK 列天然不携带 Unique：过滤不必要，提取直接消费 `schema_col.not_null/unique`（契约将提取位置/形态列为非实质留 Act；Guidance 自注「若 planner 实际不产 PK-Unique 则过滤为无害防御」——不引入死代码）。
2. 委托壳以 `into_iter().map().collect()` 构造 4 元组（契约语义「每列 false/false」一致，实现形态非实质）。
3. 新方法 doc comment 与 TableMeta 段注释锚定「约束为 catalog 元数据、不进入内存 schema」（设计决策 1 代码侧留痕），符合本文件 task 引用式注释惯例。

**Blocker Handoff**

None（本 Cycle 无阻塞）

**Blocker Resolution**

None（未经历阻塞）

**Self-Review**

- Plan compliance: T5-R1/T5-R2 按契约全绿；Invariants 逐项核实——父 Cycle 产出（T1-T4、DDL 生成器、schema 骨架）零触碰、既有测试零修改（唯一预期状态变化 = 2 见证转绿 + 1 新增用例）、`Database::create_table`/`TableManager::create_table` 公开签名零变化、storage 触碰面仅限建库约束通道、无运行时强制引入。
- Full diff reviewed: yes——`table_manager.rs`/`create_table.rs` 全量 diff 逐行过目（错误路径：保留名守卫/重复表双检/PK 校验/catalog 失败回滚全部未动）；cli_test.rs 新增用例与既有断言零修改核实。
- Critical findings unresolved: 0
- Important findings unresolved: 0
- Minor findings unresolved: 0

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 结论 |
|---|---|---|---|
| 见证 RED（集成） | `cargo test --test cli_test test_schema`（实施前） | `test_schema_outputs_ddl` FAIL（输出 `"label" STRING` 缺 NOT NULL）+ `test_schema_unique_roundtrip` FAIL（缺 UNIQUE）；2 passed（S2/S3） | RED 确认（缺口见证） |
| 见证 RED（lib） | `cargo test --lib storage::data::table_manager`（实施前） | `error[E0599]: no method named create_table_with_constraints` | RED 确认（新 API） |
| T5-R1 GREEN | `cargo test --lib storage::data::table_manager` | `test result: ok. 1 passed; 0 failed` | PASS |
| T5-R2 GREEN | `cargo test --test cli_test test_schema` | `test result: ok. 4 passed; 0 failed` | PASS（S1 转绿 + UNIQUE 往返） |
| 全量门 | `cargo test --all` | `passed=686 failed=0 ignored=2`（基线 671 + 父 Cycle 12 + 本 Cycle 2 集成/lib 新增中 3 计数） | PASS |
| 静态分析 | `cargo clippy --all-targets -- -D warnings` | `Finished \`dev\` profile`，0 warning | PASS |
| 格式 | `cargo fmt --check` | 无 diff | PASS |
| OpenSpec | `openspec validate 2026-09-09-ms10-t05-lifecycle-subcommands` | `Change ... is valid` | PASS |

**Persisted Evidence**

None required（`none` 模式：全部验证命令低成本可重跑，上表决定性输出足以承载 Gate 5 与 Review）

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| Runbook / Incident | None | None | None（常规 TDD 缺口修复，无新可重复操作路径或故障） |

**Remaining Issues**

1. 旧库（历史建表）catalog flags 恒 false → schema 对旧库输出无 NOT NULL/UNIQUE，真实反映持久化面（Plan Context Risks 已裁定非回归）。
2. dump/restore/import 仍为 Usage 占位（exit 2），属 Iteration 001（T6-T8）+ T9 终门；restore 经 SQL `CREATE TABLE` 重建将自然继承约束持久化（Plan Context 推论，Iteration 001 展开时由 Plan 核实）。
3. Deviation 1 的 Guidance 形态差异（单元变体 vs 结构体变体预期）已记录，供 Review 核对提取实现。

**Commit or Diff Reference**

未 commit（工作树现场：父 Cycle 产出（`src/cli/mod.rs`、`src/cli/resolve.rs`、`src/cli/lifecycle.rs`、`tests/cli_test.rs`）+ 本 rework（`src/storage/data/table_manager.rs`、`src/executor/create_table.rs`、`tests/cli_test.rs` 增量）+ change 目录与 R20 登记等规划产物）

## Plan Review

- Review Result: accepted（2026-09-09）

**Findings**

1. **T5-R1/T5-R2 独立验证 PASS**：Plan 独立重跑 `cargo test --all` → **零失败**（lib 193 passed = 192 基线 + 1 新单测；cli_test 36 passed / 2 ignored = 34 + S1 转绿 + UNIQUE 新增）；`cargo clippy --all-targets -- -D warnings` 退出码 0；`cargo fmt --check` 干净；`openspec validate` PASS。
2. **Diff 逐项比对契约吻合**：`table_manager.rs`——`create_table` 保留公开签名改为委托壳（每列显式 false/false），`create_table_with_constraints` 承载原主体（保留名守卫/重复双检/PK 校验/catalog 失败回滚逐行未动），`catalog_cols` 消费真实 flags，`TableMeta` 经 `schema_cols` 保持 `Vec<(String, ColumnType)>` 运行时形状（设计决策 1 落实：约束不入内存 schema，与 `open_or_init` 恢复路径同形状）；`create_table.rs`——4 元组构造复用 `to_schema_column()` 既有解析、切换新方法；新 lib 单测同时锁定「新路径真实持久化」与「旧签名委托恒 false」两侧。
3. **Act 偏差 1-3 均非实质，接受**：① `ColumnConstraint::Unique` 为单元变体（Plan Guidance 误判为结构体变体——Guidance 依据的 `is_primary: false` 匹配在 planner `ddl_dml.rs:188-192` 已完成过滤，PK 列天然不携带 Unique，无需防御性过滤，不引入死代码的选择正确）；② 委托壳 `into_iter().map().collect()` 形态；③ doc comment 锚定元数据语义（符合本文件注释惯例）。
4. **Minor**：新单测依赖 `tempdir` 夹具（与 src 内既有测试惯例一致）；无其他发现。

**Deviation Classification**

None（Act 无偏离；父 Cycle 的 PLAN-OMISSION 已由本 Cycle 关闭。Guidance 中 `Unique` 变体形态预判偏差属非实质 Guidance 错误，Act 处理正确，不构成 finding 级偏差）

**Acceptance Gaps**

None——R-schema S1 已满足（`test_schema_outputs_ddl` 断言零修改转绿 + `test_schema_unique_roundtrip` 往返见证 + T5-R1 lib 单测）；Iteration 000 全部 Acceptance（R1 分发/R-new/R-list/R-schema）达成。

**Convergence**

reduced——父 Cycle 唯一 gap（R-schema S1）完全关闭，无剩余、无扩大。

**Evidence**

- `cargo test --all` → 47 个测试二进制全 ok，零 FAILED（lib 193/0；cli_test 36/0/2）
- `cargo clippy --all-targets -- -D warnings` → exit 0（仅 cargo config 弃用提示，非代码告警）；`cargo fmt --check` → 干净；`openspec validate 2026-09-09-ms10-t05-lifecycle-subcommands` → valid
- 代码核实：`git diff src/storage/data/table_manager.rs src/executor/create_table.rs`（委托壳/新方法/4 元组/错误路径未动）；`tests/cli_test.rs::test_schema_unique_roundtrip`；`table_manager.rs` tests `create_table_with_constraints_persists_flags`

**Follow-up Decision**

接受：Acceptance 全部满足且无阻塞项，Iteration 000 完成。按 Map 展开 Iteration 001（`../001-data-plane/000-initial.md`，T6-T9：dump/restore/import --csv + 回归门），其 Plan Context 状态 `draft`，Gate 2 待用户批准后交 openspec-act。

**Iteration Plan Update**

None（Map 不变）

**Next Cycle**

None（Iteration 000 完成于本 Cycle accepted）

**Next Iteration**

`../001-data-plane/000-initial.md`（已展开，Status: draft）
