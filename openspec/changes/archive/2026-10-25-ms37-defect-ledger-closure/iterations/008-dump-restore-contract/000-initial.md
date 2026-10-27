# Iteration 008 / Cycle 000: dump / restore 的合同面（UNIQUE 按整数类型渲染 + 多表部分恢复成文）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T11（ISS33）、T12（ISS32）
- Acceptance gaps: None
- Repair items: None
- Inherited scope:
  - change `design.md` **D5**（`UNIQUE` 只在列类型为整数时输出，与 `PRIMARY KEY` 同型）。**不得另选方向**。
  - change `design.md` **D3**（维持逐条自动提交、失败即停；不实现原子 restore，不做全量预检）。**不得另选方向**。
  - change `specs/cli-noninteractive-shell/spec.md`、`specs/sql-constraint-enforcement/spec.md`、`specs/legacy-format-compat/spec.md`。这三份增量已经写好，本 Cycle **不改规格正文**。
  - Iteration 007 已交付的在途写入标记、按页失败集合和锚点提升。**零改动**。
- Excluded scope:
  - T13–T16 → Iteration 009。全量回归不在本 Cycle。
  - ISS32 / ISS33 台账条目的编辑，以及 Runbook、Issue 正文。
  - 全局 `.agents/docs/tasks.md` / `SNAPSHOT` / 项目记忆 / references。
  - 主规格库 `openspec/specs/`（收尾时由文档维护合并）。
  - 计划期放开非整数 `UNIQUE`，原子 restore，全量预检，跨语句事务。

**Objective**

1. 存量库里非整数列上的 `UNIQUE` 标志，`dump` 不再把它写进 DDL。该产物可以 `restore` 进空库，行还在，新库没有这个唯一约束。整数列上的 `UNIQUE` 仍输出，恢复后仍强制。旧 dump 文本里已经写着非整数 `UNIQUE` 的，恢复仍被拒绝（ISS33）。
2. 多表 dump 在靠后的语句被拒绝时，恢复以退出码 3 停下，点名语句序号，并写明此前语句已经生效；已经执行的表留在库里，不回滚，也不报成功（ISS32）。这条只加回归，不改 `restore`。

**Investigation Facts**

- Starting point:
  - Iteration 007 / Cycle 000 的 Act Response（2026-10-26，`reported`）与本 Plan Review（`accepted`）是起点。007 改了缓冲池在途标记、`failed_pages` 和释放锚点。**没有**改 `create_table_sql`、`restore` 或计划期的 `UNIQUE` 拒绝。
  - 007 的定向回归一批 4 个目标（实际目标名 `drop_persistence_test`，不是计划里写错的 `drop_persistence_boundary_test`），批内 `--test-threads=1`，然后单独 `cargo test --lib -- --test-threads=1`。4 个目标 11 / 12 / 2 / 3 passed，`--lib` 421 passed / 0 failed。全量仍只预算在 Iteration 009 / T16。
  - 工作区产品改动未提交。本 Cycle 从当前代码实施，不回退 000–007。
- Current-State Evidence（行号是当前工作区，符号名是定位依据）：
  - `src/cli/lifecycle.rs::create_table_sql`：`PRIMARY KEY` 只在列名等于主键且 `ColumnType::Int` 时输出。`UNIQUE` 在 `col.unique` 为真时**无条件**输出，不看类型。`schema`（约 366 行）和 `dump`（约 438 行）都调用这一个函数。
  - 类型渲染是标准长名：`Int` → `INTEGER`，`String(_)` → `TEXT`，长度不进 DDL。
  - 非整数 `UNIQUE` 不能从 SQL 建出来。`src/parser/planner/ddl_dml.rs` 约 1293 行：非主键列、声明了 `UNIQUE`、类型不是 `Int`，计划期返回 `UnsupportedConstraint("UNIQUE (INT columns only)")`。显示文本是 `Unsupported constraint: UNIQUE (INT columns only)`。
  - 存量形态用 `TableManager::create_table_with_constraints` 直接写入。元组第 4 个 `bool` 是 `unique`，会进目录列。非 `Int` 的 `unique` 不建唯一索引（约 804 行只给 `Int ∧ unique ∧ 非主键` 建树），所以该标志从未被强制。先例：`tests/cli_test.rs::test_dump_pk_rendering_matches_enforced_semantics`。
  - 因此当前 `dump` 会写出 `TEXT UNIQUE` 一类 DDL，`restore` 在计划期被拒绝，往返走不通。这是 ISS33。
  - `src/cli/lifecycle.rs::restore`：先要求空库，再逐条 `plan_stage` / `execute_stage`。任一条失败调用 `sql_failure_status`，`k > 1` 且不在显式事务里时，文案追加 `previous statement(s) were committed`。退出码 3 来自 `ExitStatus::Sql`。没有跨语句回滚，没有预检。
  - 既有单表失败用例 `tests/cli_test.rs::test_restore_fail_fast` 锁的是重复主键（语句 3 / 共 4，退出码 3，前序已提交，只留下第一行）。它不是多表，也不是非整数 `UNIQUE`。
  - 旧 dump 文本 `tests/fixtures/legacy/legacy_dump_non_integer_unique.sql` 的第一条就是 `STRING UNIQUE`。`tests/legacy_compat_test.rs::legacy_dump_with_non_integer_unique_is_rejected` 锁退出码 3、`UNIQUE (INT columns only)`、`statement 1 of 2 failed`。这条输入面保持。
  - 整数 `UNIQUE` 往返已由 `tests/cli_test.rs::unique_table_dump_restore_enforcement_roundtrip` 锁住：DDL 含 `UNIQUE`，恢复后重复值被 `Duplicate key` 拒绝。
  - `schema` 的整数 `UNIQUE` 由 `tests/cli_test.rs::test_schema_unique_roundtrip` 锁住。
  - 同文件单元测试 `ddl_generator_quotes_and_orders_constraints` 的夹具是整数主键且 `unique == true`，期望整句 `INTEGER PRIMARY KEY NOT NULL UNIQUE`。条件收紧后这句仍成立。
  - `README.md` 与 `README.zh-CN.md` 的存量兼容段只写了：含非整数 `UNIQUE` 的旧库可以打开，这些列不被强制。没有写「当前导出不渲染 `UNIQUE`，因此可以往返」。`PRIMARY KEY` 那半句已经写了不渲染、可以往返。范围外拒绝和「去掉 `UNIQUE` 后再恢复」的迁移路径已经在这两段里。
- Code and Critical Path:
  - 渲染：`create_table_sql` ← `dump` / `schema`。
  - 拒绝面（不改）：`ddl_dml.rs` 的非整数 `UNIQUE` 分支 ← `restore` 的 `plan_stage`。
  - 部分恢复（不改）：`restore` ← `sql_failure_message`。
  - 观察入口：真实 `rtsql` 二进制（`tests/cli_test.rs` 的 `run_cli` / `CARGO_BIN_EXE_rtsql`）。存量库用 `Database::open` + `create_table_with_constraints`，与主键那条用例同一通道。

**Implementation Guidance**

1. 先做 T11 的渲染条件，再做 T12 的回归。T12 用手工 dump 文本，不依赖 T11 的新产物；T12 的用例在改 `restore` 之前就应当是绿的。
2. `UNIQUE` 的条件与主键同型：`col.unique && matches!(col.column_type, ColumnType::Int)`。`NOT NULL`、`DEFAULT`、主键条件不动。
3. 文档只补半句，与主键那半句并列。不改拒绝面措辞，不改迁移路径里「去掉声明」和「改类型并转换行值」两句。

**Task Definitions**

### T11: 非整数 UNIQUE 的 dump 不再输出该约束，整数 UNIQUE 与旧 dump 拒绝保持（ISS33）

- Requirement/Scenario: CLI「INTEGER 唯一列的 dump 输出与往返」「非 INTEGER UNIQUE 存量表 dump 渲染与往返」；SCE「dump/restore 往返保持唯一强制」「存量非 INT UNIQUE 库的边界已知」；LFC「非整数唯一列的旧 dump 明确拒绝」
- Depends on: None
- Targets:
  - `src/cli/lifecycle.rs::create_table_sql`
  - `README.md` 与 `README.zh-CN.md` 的存量兼容段（各补半句）
  - `tests/cli_test.rs`（新增一条真实 CLI 用例）
- Current behavior:
  - `col.unique` 为真就输出 ` UNIQUE`，不看类型。
  - 用 `create_table_with_constraints` 建出的 `TEXT` + `unique` 表，`dump` 含 `UNIQUE`，`restore` 在计划期失败，往返走不通。
  - 整数 `UNIQUE` 会输出，恢复后仍拒绝重复值。
  - 旧 dump 文本里的非整数 `UNIQUE` 被拒绝，退出码 3，错误含 `UNIQUE (INT columns only)` 和语句序号。
- Required behavior:
  1. 只有 `ColumnType::Int` 且 `unique` 的列输出 ` UNIQUE`。`String` / `Float` / `Bool` / `Date` / `Timestamp` 即使目录里 `unique` 为真，也不输出。
  2. 存量非整数 `UNIQUE` 表：`dump` 的建表语句不含 `UNIQUE`；`rtsql new` + `rtsql restore` 退出码 0；行与原库一致，包括该列上的重复值；恢复后的表再插入一个重复值仍然成功。`schema` 与 `dump` 共用这个函数，源库的 `schema` 同样不含这个 `UNIQUE`。
  3. 整数 `UNIQUE` 的 dump 仍含 `UNIQUE`，恢复后重复值仍被拒绝。`schema` 的整数 `UNIQUE` 仍渲染。
  4. 计划期拒绝不变：`CREATE TABLE` 里对非整数列写 `UNIQUE` 仍然失败，表不被创建。旧 dump 文件 `tests/fixtures/legacy/legacy_dump_non_integer_unique.sql` 的恢复结果不变。
  5. 两份 README 的存量兼容段，在「这些列不被强制」之后补上当前导出不渲染 `UNIQUE`、因此 `dump` → `restore` 可以往返。英文对应句同样补上。范围外拒绝和已有迁移路径保持原句。
- Preserve:
  - 主键只在整数列输出 `PRIMARY KEY`。
  - `ddl_generator_quotes_and_orders_constraints` 的整句期望。
  - 计划期文案 `UNIQUE (INT columns only)`。
  - 磁盘格式、目录里的 `unique` 标志本身（只改渲染，不改存量标志）。
- Forbidden:
  - 改 `ddl_dml.rs` 的拒绝条件，或让非整数 `UNIQUE` 的 `CREATE TABLE` 成功。
  - 改 `restore` 的失败即停、退出码或「此前语句已提交」文案。
  - 改 `tests/fixtures/legacy/legacy_dump_non_integer_unique.sql`。
  - 改 Iteration 007 的缓冲池、失败集合、锚点。
  - 跑全量回归。改写 README 整段。改主规格库。
- First test:
  - 位置：`tests/cli_test.rs`，真实二进制。
  - 夹具：`Database::open` + `create_table_with_constraints`，表 `s`，列 `id INT` 主键、`email` 为 `ColumnType::String` 且 `unique = true`。插入两行，`email` 同为 `a@b.c`。这条在变更前就能插入，因为该标志不建索引。
  - 断言：`dump` 含 `CREATE TABLE "s" ("id" INTEGER PRIMARY KEY, "email" TEXT);`，不含 `UNIQUE`；`schema` 同样不含这个 `UNIQUE`；`new` + `restore` 退出码 0；新库两行都在；再插入一行相同 `email` 退出码 0。
  - RED（变更前）：`dump` 含 `TEXT UNIQUE`，断言失败。这是行为失败，不是编译失败。若 `restore` 先被跑到，退出码是 3。
- GREEN condition:
  - 上面这条为绿。
  - `unique_table_dump_restore_enforcement_roundtrip`、`test_schema_unique_roundtrip`、`test_dump_pk_rendering_matches_enforced_semantics` 仍绿。
  - `legacy_dump_with_non_integer_unique_is_rejected` 仍绿。
  - `planner_test` 的 `test_create_table_non_int_column_unique_rejected` 仍绿。
  - 两份 README 都写明当前导出不渲染非整数 `UNIQUE` 且可以往返。
- Verification:
  - 本 Cycle 的定向回归见文末。不跑全量。
- Stop when: 必须改计划期拒绝才能让往返通过；必须改磁盘格式或清掉目录里的 `unique` 标志；某个既有用例锁的是「当前 dump 对非整数列必须输出 `UNIQUE`」这句原文字面，而且不能改成新渲染、只能删掉断言。

### T12: 多表 dump 的部分恢复被真实入口锁住，restore 行为不变（ISS32）

- Requirement/Scenario: CLI「多表 dump 的部分恢复显式可见」
- Depends on: T11
- Targets:
  - `tests/cli_test.rs`（新增一条）
  - 产品代码：无。`restore` 与 `sql_failure_message` 保持现状
- Current behavior:
  - 靠后的语句在计划期失败时，`restore` 退出码 3，stderr 含 `statement k of n failed` 和 `previous statement(s) were committed`。前面已经执行的语句留在库里。后面的语句不执行。不报成功。
- Required behavior:
  1. 手工 dump 文本含两张表：先 `CREATE TABLE "kept"`（整数主键）和一行 `INSERT`，再 `CREATE TABLE "later"`（整数主键加 `TEXT UNIQUE`）和一行 `INSERT`。共 4 条语句，失败点是第 3 条。
  2. 对空库 `rtsql restore`：退出码 3；stderr 含 `statement 3 of 4 failed`、`UNIQUE (INT columns only)`、`previous statement(s) were committed`。
  3. 另一次进程打开该库：`kept` 的那一行还在；`later` 不存在。退出码不是 0。
- Preserve:
  - `test_restore_fail_fast` 的重复主键失败形态。
  - 空库拒绝、文件不存在、锁冲突的退出码。
- Forbidden:
  - 为让这条变绿而去改 `restore`、回滚已执行语句、或做全量预检。
  - 用 T11 的新 dump 产物充当「会被拒绝的靠后语句」。那条产物按 T11 不再含 `UNIQUE`，打不中这个场景。
  - 跑全量。
- First test:
  - 位置：`tests/cli_test.rs`，放在 `test_restore_fail_fast` 附近。真实 `rtsql restore`。
  - 变更前期望：**绿**。当前 `restore` 已经是这个行为。先跑这条，确认是绿，再结束本任务。若是红，停止，回到 Plan，不要改产品代码去凑绿。
- GREEN condition:
  - 这条保持绿，且 `src/cli/lifecycle.rs::restore` 与 `src/cli/mod.rs::sql_failure_message` 无 diff。
- Verification:
  - 见文末。不跑全量。
- Stop when: 这条在变更前就是红的；或者必须改 `restore` 的控制流才能让它变绿。

**Invariants**

- 计划期仍拒绝非整数列上的 `UNIQUE` 声明。整数 `UNIQUE` 仍强制。
- `restore` 仍逐条自动提交，失败即停，退出码 3，`k > 1` 时写明此前语句已提交。
- 磁盘格式不变。目录里的存量 `unique` 标志不改。
- Iteration 007 的在途写入、失败集合、锚点提升不变。
- 不引入身份证明、判定层、新产品参数或配置。

**Non-goals**

- 全量回归（Iteration 009 / T16）。
- 原子 restore、全量预检、跨语句事务。
- 非整数列 `UNIQUE` 的计划期支持。
- 语料标题、Purpose、WAL 文档（Iteration 009）。

**Acceptance**

| # | 验收条件 | 需求 / 场景 | 设计 | 任务 | 代码位置 | 测试 |
|---|---|---|---|---|---|---|
| A40 | 整数 `UNIQUE` 的 dump 仍含 `UNIQUE`，恢复后重复值被拒绝 | CLI「INTEGER 唯一列的 dump 输出与往返」；SCE「dump/restore 往返保持唯一强制」 | D5 | T11 | `create_table_sql` | `unique_table_dump_restore_enforcement_roundtrip` |
| A41 | 存量非整数 `UNIQUE` 的 dump 与 schema 不含该 `UNIQUE`；restore 退出码 0；重复值仍在；新库再插重复值成功 | CLI「非 INTEGER UNIQUE 存量表 dump 渲染与往返」；SCE「存量非 INT UNIQUE 库的边界已知」 | D5 | T11 | `create_table_sql` | `non_integer_unique_stock_dump_roundtrips_without_the_constraint` |
| A42 | 旧 dump 文本里的非整数 `UNIQUE` 仍退出码 3，点名 `UNIQUE (INT columns only)` 和语句序号；计划期建表仍拒绝 | LFC「非整数唯一列的旧 dump 明确拒绝」；SCE 该 Scenario 的旧产物子句 | D5 | T11 | 计划期分支不改 | `legacy_dump_with_non_integer_unique_is_rejected`；`test_create_table_non_int_column_unique_rejected` |
| A43 | 两份 README 写明当前导出不渲染非整数 `UNIQUE` 且可以往返；已有拒绝和迁移路径原句还在 | LFC 迁移路径补写 | D5 | T11 | `README.md`、`README.zh-CN.md` | 读这两句 |
| A44 | 多表 dump 靠后语句被拒：退出码 3，语句序号，此前已提交，前表的行还在，后表不存在，不报成功；`restore` 无改动 | CLI「多表 dump 的部分恢复显式可见」 | D3 | T12 | `restore`、`sql_failure_message` | `multi_table_restore_reports_the_partial_apply` |

**Verification**

- T11 的往返和 T12 的部分恢复都走真实 `rtsql` 二进制。断言是退出码、stderr 片段、DDL 文本和查询行。
- 定向回归只跑一批，目标按名字典序，批内 `--test-threads=1`：

```text
cargo test --test cli_test --test legacy_compat_test --test planner_test -- --test-threads=1
```

- 然后单独跑 `cargo test --lib -- --test-threads=1`。两步串行，不同时跑。
- 不跑无目标过滤的全量，不用默认并行度跑这一批。全量仍只在 Iteration 009 / T16，按该处已写的分批规则。
- 通过条件：这一批 0 failed，且 `--lib` 0 failed。T12 的新用例在任何 `restore` 改动之前已经是绿的；本 Cycle 不应产生 `restore` 改动。

**Gate 2 Readiness**

- Step 5：无 TBD。D3 / D5 已裁定，规格增量已在本 change 内，无未批准裁剪。tasks / specs / design 一致。Current-State Evidence 来自当前 `create_table_sql`、`restore`、`sql_failure_message` 和计划期拒绝。两个任务的入口、RED 或变更前即绿的形态已写明。全量不在本 Cycle。Act 只读本文件即可写第一个先行测试。验证直接看退出码、stderr、DDL 和查询行。无身份证明、无判定层。⇒ 全部 PASS。
- 用户批准：goal「继续把当前 ms 做完，记得遵守各种规范，尤其是多批次测试规范」构成 Gate 2 的批准。批准不降低判据。

**Cycle RTM**

| Requirement / Scenario | Task | First Test | Status |
|---|---|---|---|
| CLI「非 INTEGER UNIQUE 存量表 dump 渲染与往返」 | T11 | `non_integer_unique_stock_dump_roundtrips_without_the_constraint` | Covered |
| CLI「INTEGER 唯一列的 dump 输出与往返」 | T11 | `unique_table_dump_restore_enforcement_roundtrip` | Covered |
| SCE「存量非 INT UNIQUE 库的边界已知」 | T11 | 同上新用例 + `legacy_dump_with_non_integer_unique_is_rejected` | Covered |
| LFC「非整数唯一列的旧 dump 明确拒绝」 | T11 | `legacy_dump_with_non_integer_unique_is_rejected` | Covered |
| CLI「多表 dump 的部分恢复显式可见」 | T12 | `multi_table_restore_reports_the_partial_apply` | Covered |

**Persisted Evidence**

- Mode: none

CLI 用例和三个既有目标都可以重跑。结果写入 Act Response。不建 `evidence/` 目录。

**Risks and Notes**

- `schema` 与 `dump` 共用 `create_table_sql`。非整数存量 `UNIQUE` 的 `schema` 也会不再输出该词。整数 `UNIQUE` 的 `schema` 由既有用例保住。
- 旧库文件样本（G1 / G2）的 dump 往返用例只检查表名和行值，不锁「非整数列必须输出 `UNIQUE`」这句原文。若某条断言锁的是这句原文，按 T11 的 Stop when 回到 Plan。
- 既有 fmt / clippy 基线仍按 `design.md` D13：只核对本 Cycle 触碰的文件，不重排无关文件。
- 跳过项：不跑全量；不改 `restore`；不改计划期拒绝；不改 007 的写盘失败状态；不登记 ISS32 / ISS33 的台账关闭。

**后续任务边界**

- 授权边界：无。
- 能力边界：无。
- 停止边界：本 Cycle 被接受后才展开 Iteration 009。Act 不创建下一 Iteration。
- 不在本 Cycle：T13–T16；ISS32 / ISS33 的台账指针。

## Act Response

**状态**: `reported`
**Cycle**: Iteration 008 / Cycle 000（`initial`）　**Task**: T11 + T12
**Act 执行日期**: 2026-10-26

Plan Context 保持 `ready`。本文件没有 Plan Review。没有下一 Cycle，没有下一 Iteration。

### 接手时的实际状态

- Plan Context `ready`，Review Result 仍是 `pending`，没有后继 Cycle。占位里的「未执行」不是实施记录。
- 从当前工作区实施，没有回退 Iteration 000–007。

### RED / GREEN

T11 先行测试 `non_integer_unique_stock_dump_roundtrips_without_the_constraint` 先于 `create_table_sql` 的修改运行。失败点是 dump 断言，不是编译失败。当时 stdout 含 `CREATE TABLE "s" ("id" INTEGER PRIMARY KEY, "email" TEXT UNIQUE);`。同一命令在条件收紧后通过（1 passed）。

T12 的 `multi_table_restore_reports_the_partial_apply` 在任何 `restore` 改动之前运行，1 passed。此后没有改 `restore`。

### 实际改动

| 位置 | 改动 |
|---|---|
| `src/cli/lifecycle.rs::create_table_sql` | `UNIQUE` 仅在 `col.unique && matches!(col.column_type, ColumnType::Int)` 时输出。`PRIMARY KEY`、`NOT NULL`、`DEFAULT` 未改 |
| `tests/cli_test.rs::non_integer_unique_stock_dump_roundtrips_without_the_constraint` | 存量 `TEXT` + `unique` 表的真实 CLI 往返 |
| `tests/cli_test.rs::multi_table_restore_reports_the_partial_apply` | 手工 4 语句 dump 的部分恢复。产品代码无对应改动 |
| `README.md`、`README.zh-CN.md` | 存量兼容段在「这些列不被强制」之后各补半句：当前导出不渲染 `UNIQUE`，因此 `dump` → `restore` 可以往返 |

`src/cli/lifecycle.rs::restore` 与 `src/cli/mod.rs::sql_failure_message` 不在本 Cycle 的 diff 里。`ddl_dml.rs`、旧 dump 夹具、Iteration 007 的缓冲池 / 失败集合 / 锚点都没有改。

### 与计划的偏差

无。

### 自检

Gate 3：T11 的非整数存量 `UNIQUE` 不再进入 DDL，整数 `UNIQUE` 仍进入，计划期拒绝未动。T12 只锁已有的失败即停。没有身份证明或判定层。计划范围内没有未解决的 Critical 或 Important。

Gate 4：见下方验证，两步都是 0 failed。

Gate 5：未触发。

遗留 Minor：无。

`cargo clippy --lib --test cli_test` 在本 Cycle 新增代码上没有告警。`lifecycle.rs` 的 `restore` 里原有一处 `match` 提示（约 637 行），该函数不在本 Cycle 的 diff 里，没有改。触碰的 Rust 文件已 `rustfmt`；`lifecycle.rs` 原本就符合格式，`cli_test.rs` 只重排了这两条新用例。

### 验证

TDD 观察与收尾批次分开。收尾两步串行，没有同时跑，没有无过滤全量，没有用默认并行度。

1. `cargo test --test cli_test non_integer_unique_stock_dump_roundtrips_without_the_constraint -- --test-threads=1`

   变更前：1 failed。dump 含 `TEXT UNIQUE`。变更后：1 passed。

2. `cargo test --test cli_test multi_table_restore_reports_the_partial_apply -- --test-threads=1`

   1 passed。此时 `restore` 仍无 diff。

3. `cargo test --test cli_test --test legacy_compat_test --test planner_test -- --test-threads=1`

   三个目标在同一次调用里先后跑完。`cli_test` 126 passed / 2 ignored / 0 failed；`legacy_compat_test` 16 passed / 0 failed；`planner_test` 107 passed / 0 failed。其中 `unique_table_dump_restore_enforcement_roundtrip`、`test_schema_unique_roundtrip`、`test_dump_pk_rendering_matches_enforced_semantics`、`test_restore_fail_fast`、`legacy_dump_with_non_integer_unique_is_rejected`、`test_create_table_non_int_column_unique_rejected` 都在这批里通过。

4. `cargo test --lib -- --test-threads=1`

   421 passed / 0 failed。`ddl_generator_quotes_and_orders_constraints` 在其中通过。

没有跑全量。全量仍在 Iteration 009 / T16。

### Persisted Evidence

None required。

### Experience Candidates

None。

### Issue 候选

None。

### 未解决

无阻塞项。Iteration 008 未标 accepted，等待 Plan Review。

## Plan Review

- Review Result: accepted

**Findings**

独立阅读了 `create_table_sql`、两条新用例、两份 README 的存量兼容段，以及相对 HEAD 的 diff。`lifecycle.rs` 的 diff 只有 `UNIQUE` 的条件：`col.unique && matches!(col.column_type, ColumnType::Int)`。`PRIMARY KEY`、`NOT NULL`、`DEFAULT` 未改。`src/cli/mod.rs`、`src/parser/planner/ddl_dml.rs`、`tests/fixtures/legacy/legacy_dump_non_integer_unique.sql` 无 diff。`restore` 在同一文件里，不在 diff 中。`buffer_pool.rs` 与 `src/wal/recovery.rs` 的修改时间早于本 Cycle 对 `lifecycle.rs` 的修改，不属于这次 diff。

A40：`unique_table_dump_restore_enforcement_roundtrip` 未改，仍要求 dump 含 `UNIQUE`，恢复后重复值退出码 3 且 stderr 含 `Duplicate key`。整数列仍走新条件的真分支。该用例在收尾批次里通过。

A41：`non_integer_unique_stock_dump_roundtrips_without_the_constraint` 用 `Database::open` 和 `create_table_with_constraints` 建 `TEXT` 且 `unique = true` 的表，经 `CARGO_BIN_EXE_rtsql` 插入两行相同 email。断言 dump 与 schema 都是 `CREATE TABLE "s" ("id" INTEGER PRIMARY KEY, "email" TEXT);`，且全文不含 `UNIQUE`；`new` + `restore` 退出码 0；两行都在；再插入一行相同 email 退出码 0。

A42：`legacy_dump_with_non_integer_unique_is_rejected` 与 `test_create_table_non_int_column_unique_rejected` 的文件不在本 Cycle diff 里。计划期字符串 `UNIQUE (INT columns only)` 仍在 `ddl_dml.rs`。两条都在收尾批次里通过。

A43：两份 README 都在「这些列不被强制」之后补了当前导出不渲染 `UNIQUE`、因此 `dump` → `restore` 可以往返。拒绝原因、退出码 3 和两条迁移路径的原句还在。diff 各只有这一处插入。

A44：`multi_table_restore_reports_the_partial_apply` 的手工文本是 4 条语句，第 3 条为 `TEXT UNIQUE`。断言退出码 3，stderr 含 `statement 3 of 4 failed`、`UNIQUE (INT columns only)`、`previous statement(s) were committed`；重开后 `kept` 的行是 `[[1]]`；查询 `later` 的退出码不是 0。`sql_failure_message` 在 `k > 1` 且不在事务里时仍追加 `previous statement(s) were committed`。

偏差：无。Act 写的改动与 diff 一致。没有阻塞 Acceptance 的发现，也没有需要返工的 Minor finding。

范围外实质缺陷：无。

验证沿用 Act Response（2026-10-26），本审查没有重跑。沿用条件：报告的命令和计数与当前代码一致；Cycle 文件的写入时间晚于 `lifecycle.rs`、`cli_test.rs` 和两份 README，没有「结论产生之后这些文件又被改过」的证据。`cli_test.rs` 的测试属性 128 个、`#[ignore]` 2 个，与 126 passed / 2 ignored 一致。`legacy_compat_test.rs` 16 个、`planner_test.rs` 107 个，与报告一致。`--lib` 的 421 与 Iteration 007 的 421 相同，本 Cycle 没有新增库内测试。

分批核对：一批 3 个目标，名字典序，批内 `--test-threads=1`：`cli_test`、`legacy_compat_test`、`planner_test`，命令是 `cargo test --test cli_test --test legacy_compat_test --test planner_test -- --test-threads=1`。然后单独 `cargo test --lib -- --test-threads=1`。两步串行。T11 / T12 的先行观察也带了 `--test-threads=1`。没有无过滤全量，没有默认并行度。结果：126 passed / 2 ignored / 0 failed，16 passed / 0 failed，107 passed / 0 failed；`--lib` 421 passed / 0 failed。

**Evidence**

- 代码：`src/cli/lifecycle.rs` 的 `create_table_sql`；`tests/cli_test.rs` 的 `non_integer_unique_stock_dump_roundtrips_without_the_constraint`、`multi_table_restore_reports_the_partial_apply`、`unique_table_dump_restore_enforcement_roundtrip`；`src/cli/mod.rs` 的 `sql_failure_message`；`README.md` 与 `README.zh-CN.md` 的存量兼容段。
- 验证来源：本 Cycle Act Response「验证」节。命令与计数见 Findings。本审查未重跑。

**Follow-up Decision**

A40–A44 都有对应代码或未改动的拒绝面，以及已经跑过的测试。当前 Cycle 不返工。Iteration Plan 不改。

**Next Cycle**

None

**Next Iteration**

`openspec/changes/2026-10-25-ms37-defect-ledger-closure/iterations/009-corpus-and-full-regression/000-initial.md`
