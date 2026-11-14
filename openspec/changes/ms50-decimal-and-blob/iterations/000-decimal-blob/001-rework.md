# Iteration 000 / Cycle 001: 声明别名与 BLOB 主键

## Plan Context

- Status: ready
- Cycle Type: rework

**Cycle Scope**

- Change tasks: T2
- Acceptance gaps: NUMERIC 没有按 DECIMAL 的精度规则声明；BYTEA、BINARY、VARBINARY 没有成为 BLOB，带长度时也没有拒绝；BLOB 主键在建表时被拒绝，等值过滤读不到行
- Repair items: T2-R1, T2-R2, T2-R3
- Inherited scope: `decimal-blob-types` 的五条要求；父 Cycle 已经落地的 DECIMAL 与 BLOB 声明、写入、比较、重开、超页、dump 和 G1 样本；格式版本 1；任何 `cargo test` 带 `--test-threads=1`
- Excluded scope: 结果游标；DECIMAL 算术；索引键编码；格式版本或目录代际；CAST 互转；把 `DEC` 映射成 DECIMAL；全量分批；`ms50-result-consumption`

**Objective**

NUMERIC 与 DECIMAL 使用同一套精度规则。BYTEA、BINARY、VARBINARY 在不带长度时成为 BLOB 列，带长度时拒绝建表。BLOB 可以做主键，等值过滤读到该行，并且 `to_key` 仍是 `None`。父 Cycle 已通过的声明、写入、比较和旧库行为保持。

**Background**

父 Cycle 是 `000-initial.md`。Act Response 为 `reported`。场景级的 DECIMAL 与 BLOB 行为已经由定向测试覆盖。规格「声明类型」和 proposal 已锁定口径第 4 条还要求 NUMERIC 与三个字节别名。规格「同类型比较」还要求 BLOB 主键等值可达。父 Cycle 的 Task Definition 只写了 DECIMAL 与 BLOB，Act 按该正文实现并记录了偏差。原 Task Definition 约束不了这三项，所以本 Cycle 用 repair item 补执行依据。Iteration Plan 不改。全量仍留在尚未展开的 Iteration 001。

**Investigation Facts**

- Starting point: 分支 `changes/ms50-decimal-and-blob`，HEAD `ee3ed58e3b127220cf5e41bb17cb44717e755da6`。审查时工作区干净。父 Cycle 的定向测试结论沿用，本 Cycle 不重跑那些已经覆盖且代码尚未再改的命令。规划阶段没有跑测试。
- Current-State Evidence:
  - `PlanBuilder::convert_data_type`（`src/parser/planner/ddl_dml.rs`）只对 `DataType::Decimal` 调用 `decimal_column_type`，只对 `DataType::Blob(None)` 返回 `ColumnType::Blob`。`DataType::Blob(Some(_))` 的错误文本是 `BLOB does not take a length`。其余类型落入 `Ok(ColumnType::String)`。
  - `decimal_column_type` 对 `ExactNumberInfo::None` 和精度不在 1..=38 时返回 `DECIMAL requires precision`；`scale > precision` 时返回 `DECIMAL scale must not exceed precision`；`ExactNumberInfo::Precision` 的 scale 为 0。
  - 项目用 `GenericDialect` 解析（`src/parser/ast.rs`）。sqlparser 0.44 的 `parse_data_type` 把 `NUMERIC`、`DECIMAL`、`DEC`、`BYTEA`、`BINARY`、`VARBINARY`、`BLOB` 解析成彼此独立的 `DataType` 变体。因此 NUMERIC 与字节别名今天不会进入上面的两个臂。
  - 父 Cycle 的 RED 已观察到 String 列的 schema 文本是 `TEXT`。`column_type_sql` 对 `ColumnType::Decimal` 写出 `DECIMAL(p,s)`，对 `ColumnType::Blob` 写出 `BLOB`。
  - 显式主键检查在 `build_create_table`：类型不是 `ColumnType::Int` 且不是 `ColumnType::Decimal` 时返回 `PlanError::UnsupportedConstraint("NON-INTEGER PRIMARY KEY")`，显示文本是 `Unsupported constraint: NON-INTEGER PRIMARY KEY`。
  - `Value::to_key` 对 `Blob` 返回 `None`。`secondary_index_key` 对 `ColumnType::Blob` 返回 `None`。`pk_type_known_non_int` 对任何非 `Int` 的已注册主键类型返回 true，等值因此走数据页。插入路径对非 Int 主键不走 Int 键拒绝；`to_key` 为 `None` 时不写索引项。DECIMAL 主键已经沿这条路径可读，BLOB 被挡住的位置只有建表检查。
- Code and Critical Path: 别名只改 `convert_data_type` 的类型匹配，存储类型仍是已有的 `Decimal` 与 `Blob`，所以 schema、写入门、标签和比较不用再做一套。BLOB 主键只放宽建表检查，查询沿用非 Int 主键的数据页等值。

**Implementation Guidance**

三个 repair item 各自先写测试并看到 RED，再改对应的匹配。不要在 T2-R1 的实现里提前接上字节别名或主键放行。`NUMERIC` 直接复用 `decimal_column_type`。字节别名复用现有的无长度接受、有长度拒绝。主键只把 `ColumnType::Blob` 加进已经放行 `Decimal` 的条件。schema 文本继续由现有 `column_type_sql` 产生，不按别名回写 `NUMERIC` 或 `BYTEA`。

**Task Definitions**

### T2-R1: NUMERIC 与 DECIMAL 使用同一精度规则

- Requirement/Scenario: 声明类型 / 合法声明可见、缺少精度不建表。来源 Cycle：`000-initial`
- Depends on: None
- Targets: `src/parser/planner/ddl_dml.rs::convert_data_type`；`src/parser/planner/ddl_dml.rs::decimal_column_type`
- Current behavior: `CREATE TABLE t (d NUMERIC)` 和 `CREATE TABLE t (d NUMERIC(10,2))` 都建成 String 列。缺精度不会失败。schema 显示 `TEXT`，不显示 `DECIMAL`。
- Required behavior: `NUMERIC(p,s)` 与 `NUMERIC(p)` 建成 `ColumnType::Decimal`，规则与 `DECIMAL` 相同：`p` 为 1..=38，`NUMERIC(p)` 的 scale 为 0，scale 大于精度失败。未写精度、精度越界或 scale 越界时语句失败，文本含 `requires precision` 或 `scale must not exceed precision`，库中没有该表。schema 与 dump 写出 `DECIMAL(p,s)`，不写出 `NUMERIC`。
- Preserve: 已有 `DECIMAL(10,2)`、`DECIMAL(10)`、`DECIMAL` 缺精度、`BLOB` 的声明结果。`DEC` 继续落入 String 兜底。
- Forbidden: 把 `DEC`、`BIGNUMERIC` 或 `BIGDECIMAL` 映射成 DECIMAL。改格式版本或列行布局。
- First test: `tests/decimal_blob_test.rs::numeric_without_precision_is_rejected`。映射未改时，`CREATE TABLE t (d NUMERIC)` 成功，测试失败。命令 `cargo test --test decimal_blob_test numeric_without_precision_is_rejected -- --test-threads=1`。
- GREEN condition: 该测试通过，并且同文件中 `NUMERIC(10,2)`、`NUMERIC(10)`、越界精度和越界 scale 的断言通过。
- Verification: 上述过滤命令退出码 0、failed 为 0。失败表示 NUMERIC 仍是 String，或拒绝文本不符合规格。
- Stop when: 让 NUMERIC 成为 DECIMAL 必须提高格式版本，或必须改写已有 String 列。

### T2-R2: 字节别名成为 BLOB

- Requirement/Scenario: 声明类型 / 合法声明可见。来源 Cycle：`000-initial`
- Depends on: None
- Targets: `src/parser/planner/ddl_dml.rs::convert_data_type`
- Current behavior: `BYTEA`、`BINARY`、`VARBINARY` 落入 String。`BINARY(10)` 与 `VARBINARY(10)` 建表成功，schema 是 `TEXT`。`BLOB(10)` 已经拒绝，文本含 `does not take a length`。
- Required behavior: 不带长度的 `BYTEA`、`BINARY`、`VARBINARY` 建成 `ColumnType::Blob`。schema 与 dump 写出 `BLOB`，不写出这些别名。`BINARY(n)` 与 `VARBINARY(n)` 失败，文本含 `does not take a length`，库中没有该表。
- Preserve: 不带长度的 `BLOB` 与 `BLOB(10)` 的现有结果。`BYTES` 继续落入 String。
- Forbidden: 给 BLOB 增加长度上限或新的 tuple 标签。
- First test: `tests/decimal_blob_test.rs::byte_aliases_declare_blob`。映射未改时，`BYTEA` 列的 schema 不含 `BLOB`。命令 `cargo test --test decimal_blob_test byte_aliases_declare_blob -- --test-threads=1`。
- GREEN condition: `BYTEA`、`BINARY`、`VARBINARY` 的 schema 含 `BLOB`，`BINARY(10)` 与 `VARBINARY(10)` 被拒绝且不建表。
- Verification: 上述过滤命令退出码 0、failed 为 0。失败表示别名仍是 String，或带长度时仍能建表。
- Stop when: 这些关键字在当前解析器里不是独立的 `DataType` 变体。

### T2-R3: BLOB 主键等值可达

- Requirement/Scenario: 同类型比较 / 主键等值可达。来源 Cycle：`000-initial`
- Depends on: None
- Targets: `src/parser/planner/ddl_dml.rs` 的显式主键类型检查；`src/executor/value.rs::Value::to_key`
- Current behavior: `CREATE TABLE t (b BLOB PRIMARY KEY)` 失败，文本含 `NON-INTEGER PRIMARY KEY`。`Value::Blob` 的 `to_key` 已经是 `None`。DECIMAL 主键等值已经可读。
- Required behavior: `b BLOB PRIMARY KEY` 建表成功。写入 `X'6869'` 后，`WHERE b = X'6869'` 返回该行。`to_key` 与 `secondary_index_key` 对这个值仍返回 `None`。查询走现有的非 Int 主键数据页路径，不新增键编码。
- Preserve: `DECIMAL(10,2) PRIMARY KEY` 的等值结果。String、Float、Bool、Date、Timestamp 的显式主键仍被拒绝。非 Int 列的 UNIQUE 仍只允许 INT。
- Forbidden: 为 BLOB 实现 B-Tree 键或二级索引键。放行除 Decimal 与 Blob 以外的新主键类型。
- First test: `tests/decimal_blob_test.rs::blob_primary_key_is_reachable`。检查未改时建表失败。命令 `cargo test --test decimal_blob_test blob_primary_key_is_reachable -- --test-threads=1`。
- GREEN condition: 建表、插入和等值查询通过，并且 `to_key` 为 `None`。
- Verification: 上述过滤命令退出码 0、failed 为 0。失败表示建表仍被拒绝，或等值没有返回该行。
- Stop when: 等值读回必须新增键编码，而不能沿用非 Int 主键的数据页路径。

**Invariants**

- 任何 `cargo test` 带 `--test-threads=1`。
- 不跑无目标过滤的 `cargo test`，也不跑全量分批。
- `FORMAT_VERSION` 保持 1。已有标签和 FLOAT 字面量解析保持。
- `DEC` 保持 String 兜底。

**Non-goals**

CAST 互转。`DEC`。索引键。格式版本和目录代际。全量分批。父 Cycle 已经通过的场景不重做实现。

**Acceptance**

- T2-R1：缺精度的 NUMERIC 不建表；`NUMERIC(10,2)` 与 `NUMERIC(10)` 的 schema 分别含 `DECIMAL(10,2)` 与 `DECIMAL(10,0)`；越界精度或 scale 失败。
- T2-R2：不带长度的 BYTEA、BINARY、VARBINARY 的 schema 含 `BLOB`；`BINARY(10)` 与 `VARBINARY(10)` 不建表。
- T2-R3：BLOB 主键写入 `X'6869'` 后等值返回该行，`to_key` 为 `None`。
- 父 Cycle 的 `decimal_blob_test` 五条测试仍通过。

**Verification**

三个 repair item 各自的过滤命令先观察 RED，再观察对应 GREEN。全部修改完成后只跑一次：

```text
cargo test --test decimal_blob_test -- --test-threads=1
```

退出码 0 且 failed 为 0。这不是全量。不重跑父 Cycle 已经通过且本 Cycle 未改到的 lib 过滤测试。

**Gate 2 Readiness**

PASS。无 BLOCKED，无 SKIPPED。父 Cycle Gate 2 对标签、写入、比较、旧库和 Persisted Evidence 仍有效，本 Cycle 不重复那些维度。本 Cycle 补上的维度是别名的解析变体、现有精度函数和主键检查，事实来自上述代码。无 TBD，无未批准裁剪。计划批准来自目标原文「那就开始规划和实施这个ms，要求遵守各种规范，尤其是多批次串行测试规范，本次最多进行一次全量」，不是跳过。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

全量仍是 Iteration 001，本 Cycle 不创建该目录，也不把这次定向测试算作里程碑的那一次全量。`DEC` 与 CAST 保持现状；它们不在本 Cycle 的 Acceptance 里。

## Act Response

- Status: reported

**实际改动**

- T2-R1 完成。`convert_data_type` 把 `DataType::Numeric` 与 `DataType::Decimal` 并到同一臂，精度规则仍由 `decimal_column_type` 决定。缺精度、精度不在 1..=38 时文本含 `requires precision`；scale 大于精度时文本含 `scale must not exceed precision`。schema 与 dump 写出 `DECIMAL(p,s)`，不写出 `NUMERIC`。
- T2-R2 完成。无长度的 `BYTEA`、`BINARY`、`VARBINARY` 与 `BLOB` 一样建成 `ColumnType::Blob`。`BINARY(n)`、`VARBINARY(n)` 与 `BLOB(n)` 共用 `BLOB does not take a length`。schema 与 dump 写出 `BLOB`。
- T2-R3 完成。显式主键检查在已放行 `Decimal` 的条件上加入 `ColumnType::Blob`。`Value::to_key` 与 `secondary_index_key` 未改，对 `X'6869'` 仍返回 `None`。
- `tests/decimal_blob_test.rs` 增加 `numeric_without_precision_is_rejected`、`byte_aliases_declare_blob`、`blob_primary_key_is_reachable`。

**偏差**

计划列出的主键改动只在建表检查。建表和插入通过后，`WHERE b = X'6869'` 仍在 `route_secondary_index` 失败：十六进制字面量经 `value_from_sqlparser` 得到 `UnsupportedValue`，查询到不了数据页。调用点把这个错误当作没有索引路由，随后已有的 `literal_for_column` 按 BLOB 列解析字面量，等值走原来的非 Int 主键数据页。没有新增键编码。

这个选择优先复用已有解析和数据页路径。另一条路是让 `value_from_sqlparser` 把所有十六进制字面量变成 `Blob`，非 BLOB 列上的错误面也会变，所以没有采用。成功的索引路由不受影响。不能被 `literal_for_column` 接受的字面量仍在 `build_where` 返回 `Unsupported value type`。

`DEC`、`BYTES`、`BIGNUMERIC`、`BIGDECIMAL` 仍走 String 兜底。`FORMAT_VERSION` 未改。

**自检**

Gate 3：T2-R1 的缺精度、`NUMERIC(10,2)`、`NUMERIC(10)`、越界精度和越界 scale 由 `numeric_without_precision_is_rejected` 覆盖。T2-R2 的三个无长度别名和两处带长度拒绝由 `byte_aliases_declare_blob` 覆盖。T2-R3 的建表、插入、等值、`to_key` 和 `secondary_index_key` 由 `blob_primary_key_is_reachable` 覆盖。String、Float、Bool、Date、Timestamp 的显式主键条件未放行。非 Int 列的 UNIQUE 检查未改。

只放行建表时，T2-R3 的第一次 GREEN 失败，原因是上面的路由错误。补上调用点后同一测试通过。无未修复的 Critical 或 Important。

遗留 Minor：主键检查上方的注释仍写显式主键必须是 INTEGER。父 Cycle 放行 DECIMAL 时就已与代码不一致，本 Cycle 没有改这句注释。

Gate 4：三个 repair item 的过滤命令和本 Cycle 的 `decimal_blob_test` 均满足。Gate 5：未命中。

全量：`SKIPPED: 本 Cycle 排除全量，Iteration 001 尚未展开`。父 Cycle 已通过且本 Cycle 未改到的 lib 过滤测试未重跑。

**验证**

RED：

- `cargo test --test decimal_blob_test numeric_without_precision_is_rejected -- --test-threads=1`：退出码 101。`CREATE TABLE t (d NUMERIC)` 返回 `AffectedRows(0)`。`0 passed; 1 failed`，5 filtered out。
- `cargo test --test decimal_blob_test byte_aliases_declare_blob -- --test-threads=1`：退出码 101。schema 为 `CREATE TABLE "t" ("a" TEXT, "b" TEXT, "c" TEXT);`。`0 passed; 1 failed`，6 filtered out。
- `cargo test --test decimal_blob_test blob_primary_key_is_reachable -- --test-threads=1`：退出码 101。建表错误是 `Unsupported constraint: NON-INTEGER PRIMARY KEY`。`0 passed; 1 failed`，7 filtered out。

GREEN：

- `cargo test --test decimal_blob_test numeric_without_precision_is_rejected -- --test-threads=1`：退出码 0。`1 passed; 0 failed`，5 filtered out。
- `cargo test --test decimal_blob_test byte_aliases_declare_blob -- --test-threads=1`：退出码 0。`1 passed; 0 failed`，6 filtered out。
- `cargo test --test decimal_blob_test blob_primary_key_is_reachable -- --test-threads=1`：只改主键检查后退出码 101，`WHERE b = X'6869'` 为 `Unsupported value type`，`0 passed; 1 failed`。路由改完后同一命令退出码 0，`1 passed; 0 failed`，7 filtered out。
- `cargo test --test decimal_blob_test -- --test-threads=1`：退出码 0。`8 passed; 0 failed`，0 filtered out。

全量未跑。

**Persisted Evidence**

None required。

**Experience Candidates**

None

**未解决**

无。
