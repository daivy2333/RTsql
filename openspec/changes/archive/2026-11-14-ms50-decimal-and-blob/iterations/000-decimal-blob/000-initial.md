# Iteration 000 / Cycle 000: DECIMAL 与 BLOB

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2, T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `decimal-blob-types` 的五条要求；格式版本 1；G1/G2/G3 的现有定义；M19 的 `--test-threads=1`
- Excluded scope: 结果游标；DECIMAL 算术；索引键编码；格式版本或目录代际变更；全量分批；导入攒批

**Objective**

DECIMAL 与 BLOB 可以声明、精确写入、按数值或字节序比较，并在关闭重开后读回。G1 样本仍能打开、查询和写入。

**Investigation Facts**

- Starting point: master `f323d58a452ce34467d2e33fd6dda5cebe798b56`。规划阶段没有跑测试。`FORMAT_VERSION` 为 1。没有 Decimal 或 Blob 变体。
- Current-State Evidence:
  - 执行层 `ColumnType` 在 `src/executor/value.rs`。存储层 `ColumnType` 在 `src/storage/page_format/tuple.rs`。`ColumnDef::to_schema_column`（`src/executor/plan.rs`）做转换，String 变成 `String(255)`。
  - tuple 标签：Int `0x01`、String `0x02`、Null `0x03`、Float `0x04`、Bool `0x05`、Date `0x06`、Timestamp `0x07`、LongString `0x08`。溢出定位是 `0xFF`。`INLINE_SLOT_MAX` 为 4074。超页整行走现有 locator。
  - catalog 列标签到 Timestamp `0x06`。列行布局是类型标签，String 后才有 `u16`。序列化函数是 `serialize_catalog_column_row` 和 `deserialize_catalog_column_row`。
  - `PlanBuilder::convert_data_type` 的兜底臂返回 String。`value_from_sqlparser` 把带小数点的数字解析成 Float。
  - 写入类型门在 `insert.rs`、`update.rs`、`upsert.rs`，错误是 `column '{column}' expects {expected}, got {actual}`，门在写页之前。日期强制解析是 `coerce_datetime_write`。
  - `ComparisonPredicate::evaluate_ternary` 对 Date/Timestamp 要求同一变体，否则 `ValueError::TypeMismatch`（文本 `Type mismatch`）。`pk_type_known_non_int` 把非 Int 主键等值分到数据页。
  - `tests/fixtures/legacy/legacy_g1.db` 由 `tests/legacy_compat_test.rs` 复制到临时目录后 `Database::open`。`SELECT * FROM users` 的第一行是 `1`、`alice`、`9.5`。
- Code and Critical Path: 声明从 `convert_data_type` 到两个 ColumnType，再经 `to_schema_column` 进 catalog 标签。写入从计划期原文解析进 `Value`，经写入门缩放后由 tuple 标签落页。读回走同一解码。比较在 `Value` 的顺序函数和 `ComparisonPredicate`。旧库路径不读新标签。

**Implementation Guidance**

先让 T1 的编解码测试编译并变绿，再改 SQL。穷尽匹配只为新变体加臂，已有变体的结果保持。数字记号的 Float 解析保持，DECIMAL 列在计划期用原文另解析。溢出不够长时把 BLOB 加大到超过 `INLINE_SLOT_MAX`，不新写页类型。

**Task Definitions**

### T1: 标签往返且旧值仍可读

- Requirement/Scenario: DECIMAL 与 BLOB 的值布局 / 新标签往返、旧标签仍然可读
- Depends on: None
- Targets: `src/executor/value.rs::Value`；`src/executor/value.rs::ColumnType`；`src/executor/value_ref.rs::ValueRef`；`src/storage/page_format/tuple.rs` 的序列化与 `ColumnType`；`src/storage/catalog.rs` 的列标签
- Current behavior: 没有这两种值和标签。旧 tuple 用 `0x01` 到 `0x08`。
- Required behavior: DECIMAL 标签 `0x09`，负载 `u8 scale` 加小端 `i128`。BLOB 长度不超过 65535 用 `0x0A` 加 `u16` 长度，否则 `0x0B` 加 `u32` 长度。系数 150、scale 2 与字节 `68 69` 往返不变。只含旧标签的 tuple 解码结果不变。`0xFF` 仍不是值标签。catalog 的 DECIMAL 标签 `0x07`，后接 precision 与 scale 各一字节；BLOB 标签 `0x08`，无额外字节。旧列行解码不变。数值相等使用规范形：1.50 与 1.5 相等。哈希与相等一致。
- Preserve: `FORMAT_VERSION` 为 1。G1/G2/G3 的字段布局不增加尾段。已有标签的字节不变。
- Forbidden: 把格式版本改成 2。把精度写成无条件追加在所有列行末尾的字段。引入十进制 crate。
- First test: `src/storage/page_format/tuple.rs` 内的 `decimal_and_blob_round_trip`。改前不能编译。命令 `cargo test --lib decimal_and_blob_round_trip -- --test-threads=1`。
- GREEN condition: 该测试通过，并且同模块里一条旧标签往返测试仍通过。
- Verification: 上面的 lib 过滤命令退出码 0、failed 为 0。这不是全量，也不是整个 `--lib`。
- Stop when: 新负载无法放进现有 tuple 流，除非先增加目录代际或格式版本。

### T2: SQL 可以声明、写入、比较和读回

- Requirement/Scenario: 声明类型 / 合法声明可见、缺少精度不建表；精确写入与读回 / 精确值读回、不能精确表示的小数被拒绝、重开与超页字节不变、dump 两代相同；同类型比较 / 数值序与尺度、跨类型拒绝、主键等值可达
- Depends on: T1
- Targets: `src/parser/planner/ddl_dml.rs::convert_data_type`；`src/parser/value.rs::value_from_sqlparser`；`src/executor/plan.rs::to_schema_column`；`src/executor/insert.rs`、`update.rs`、`upsert.rs` 的写入门；`src/executor/predicate.rs::ComparisonPredicate`；`src/executor/predicate.rs::CastType`；`src/result.rs::value_to_json`；`src/cli/lifecycle.rs::column_type_sql`；`src/network/pg_messages.rs` 的类型 OID 匹配；`src/executor/query_budget.rs::value_bytes`；`src/pipeline.rs::ctas_value_type`
- Current behavior: `DECIMAL` 和 `BLOB` 落到 String。带小数点的数字变成 Float。跨类型比较里，Date 与字符串是 `Type mismatch`。
- Required behavior:
  - `DECIMAL(10,2)` 与不带长度的 `BLOB` 建表后，schema 含这两段文本。`CREATE TABLE t (d DECIMAL)` 失败，文本含 `requires precision`，没有表 `t`。`DECIMAL(p)` 的 scale 为 0。精度不在 1..=38，或 scale 大于精度，失败且文本含 `scale must not exceed precision` 或 `requires precision`。`BLOB(10)` 失败，文本含 `does not take a length`。
  - 写入 `1.50`、整数 `12`、字符串 `'1.50'` 后，SELECT 得到 1.50、12.00、1.50。`X'6869'` 读回字节 68 69。`1.234` 写入 `DECIMAL(10,2)` 失败，文本含 `expects`，表仍为空。Float 不转成 DECIMAL。空 BLOB 用 `execute_with_params` 的 `Value::Blob(Vec::new())` 写入并读回空。
  - 关闭再打开后 1.50 仍在。5000 字节若未超过 `INLINE_SLOT_MAX`，加大到超过后再读回，字节相同。
  - `WHERE d > 1.5` 与 `ORDER BY d` 按数值。`WHERE d = 1.5` 命中 1.50。`WHERE d = '1.50'` 失败，文本含 `Type mismatch`。BLOB 与字符串比较同样失败。
  - `d DECIMAL(10,2) PRIMARY KEY` 写入 1.50 后，`WHERE d = 1.50` 返回该行。`to_key` 仍返回 `None`。
  - `SELECT d + 1` 失败，文本含 `Type mismatch`，行不变。
  - dump、restore、再 dump，两代文本相同。JSON 中 DECIMAL 是 `1.50` 这种字符串，BLOB 是小写十六进制、没有 `X'`。
  - 驻留计费：DECIMAL 为 16 字节，BLOB 为 16 加字节长度。OID：DECIMAL 1700，BLOB 17，长度 -1。已有类型的 OID 和计费不改。
- Preserve: FLOAT 列写入 `1.5` 仍得到 Float。Date 的比较和标签不改。写入失败不落页。
- Forbidden: 实现 DECIMAL 加减乘除。为这两种类型写 B-Tree 键。把历史 String 列打开时改成 DECIMAL。修改结果游标。
- First test: `tests/decimal_blob_test.rs::schema_shows_decimal_and_blob`。T1 完成后、DDL 映射未改时失败。命令 `cargo test --test decimal_blob_test schema_shows_decimal_and_blob -- --test-threads=1`。
- GREEN condition: 同文件中声明、写入、拒绝、比较、主键、重开、超页、dump 和算术拒绝的测试通过。
- Verification: `cargo test --test decimal_blob_test -- --test-threads=1` 退出码 0、failed 为 0。失败表示声明、字节或比较与规格不一致。
- Stop when: 精确写入必须改 FLOAT 字面量的现有解析结果，或读回必须提高格式版本。

### T3: 旧库样本仍可打开并写入

- Requirement/Scenario: 旧库打开结果 / G1 样本打开查询并写入
- Depends on: T1
- Targets: `tests/decimal_blob_test.rs`；样本 `tests/fixtures/legacy/legacy_g1.db`；`Database::open`
- Current behavior: `tests/legacy_compat_test.rs::legacy_db_opens_queries_and_writes` 复制该样本，打开后 `SELECT * FROM users` 含 `(1, alice, 9.5)`，并可插入 `(4, dave, 6.0)`。
- Required behavior: 本变更的测试做同样的打开、查询和插入。alice 行仍在，dave 行可读。打开不因格式版本或目录代际失败。测试不改写样本文件本身，只操作临时副本。不断言哈希、修订或时间。
- Preserve: 样本文件字节不入库修改。`legacy_compat_test` 的既有断言不改。
- Forbidden: 为了让样本打开而增加格式嗅探，或在打开时重写全部目录行。
- First test: `tests/decimal_blob_test.rs::legacy_g1_opens_queries_and_writes`。若新标签解码破坏旧行，该测试失败。命令 `cargo test --test decimal_blob_test legacy_g1_opens_queries_and_writes -- --test-threads=1`。
- GREEN condition: 该测试通过。
- Verification: 与 T2 同一条整文件命令，退出码 0、failed 为 0。
- Stop when: 样本打开失败的原因是目录代际字段不够，而不是新标签的解码。那属于代际变更，回到计划。

**Invariants**

- 任何 `cargo test` 带 `--test-threads=1`。
- 不跑无目标过滤的 `cargo test`，也不在本 Cycle 跑全量分批。
- 已有六种类型的标签、比较和 FLOAT 字面量保持。
- 新匹配臂不改变旧变体的结果。

**Non-goals**

结果游标。DECIMAL 算术。索引键。格式版本和目录代际。打开时迁移旧的 String 列。全量分批。导入攒批。

**Acceptance**

- T1 证明新标签往返、旧标签仍可读、1.50 与 1.5 相等。
- T2 证明 schema、精确写入、拒绝舍入、数值比较、跨类型错误、主键可达、重开、超页字节、dump 两代相同，以及 `d + 1` 失败。
- T3 证明 G1 样本打开后 alice 行仍在，并能写入 dave。

**Verification**

```text
cargo test --lib decimal_and_blob_round_trip -- --test-threads=1
cargo test --test decimal_blob_test -- --test-threads=1
```

两条都要退出码 0 且 failed 为 0。观察的是库 API 的结果值和 CLI dump 文本。这不是全量。

**Gate 2 Readiness**

PASS。无 BLOCKED，无 SKIPPED。无 TBD；无未批准裁剪；tasks、spec、design 与本 Cycle 一致；证据来自上述代码阅读；三个任务足以让 Act 写第一个测试；全量预算在 Iteration 001，不在本 Cycle；Persisted Evidence 为 none。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

Iteration 001 才跑本里程碑唯一一次全量，目录现在不创建。另一个变更不跑全量。

5000 字节若仍能内联，测试把 BLOB 加到超过 `INLINE_SLOT_MAX` 再断言读回，不新增溢出格式。

## Act Response

- Status: reported

**实际改动**

- `Value::Decimal([u8; 17])` 与 `Value::Blob`。负载是 `u8 scale` 加小端 `i128`。`Value::decimal` / `decimal_parts` 读写系数和 scale。相等和哈希使用去掉末尾 0 的规范形，所以 1.50 与 1.5 相等且哈希相同。`to_key` 对这两种值返回 `None`。
- `ValueRef::Decimal` / `Blob` 借用字节，不把 `i128` 放进枚举。比较与哈希和 `Value` 使用同一规范形。
- tuple 标签：DECIMAL `0x09`，短 BLOB `0x0A` 加 `u16` 长度，长 BLOB `0x0B` 加 `u32` 长度。`0xFF` 仍不是值标签。
- catalog 列标签：DECIMAL `0x07`，后接 precision 与 scale 各一字节；BLOB `0x08`，无额外字节。旧列行不追加尾段。
- 执行层与存储层 `ColumnType` 增加 `Decimal { precision, scale }` 和 `Blob`。`to_schema_column` 与 `ALTER TABLE` 按这两个字段映射。
- `convert_data_type` 接受 `DECIMAL(p)` 与 `DECIMAL(p,s)`，拒绝缺精度、精度不在 1..=38、scale 大于精度，以及 `BLOB(n)`。`column_type_sql` 改为返回 `String`，写出 `DECIMAL(10,2)` 和 `BLOB`。
- `value_from_sqlparser` 仍把带小数点的数字解析成 Float。目标列或比较对侧是 DECIMAL 时，计划期用原文解析。BLOB 列的 `X'...'` 在计划期解析成字节。写入门把整数、十进制文本和已有 DECIMAL 缩放到列的 scale；不能精确放下或 Float 输入返回 `column '...' expects ...`。失败发生在写页之前。
- 比较对 DECIMAL 与 BLOB 要求同变体，否则 `Type mismatch`。同变体 DECIMAL 按数值序。`d + 1` 仍走原有非数值算术拒绝。
- 显式 `DECIMAL` 主键可以建表。其它非整数显式主键仍拒绝。
- JSON 与显示：DECIMAL 为 `1.50` 这种字符串，BLOB 为小写十六进制。dump 的 BLOB 字面量写成 `X'...'`，恢复后两代文本相同。
- 驻留计费：DECIMAL 16 字节，BLOB 为 16 加字节长度。OID：DECIMAL 1700，BLOB 17，长度 -1。已有类型的 OID 和计费分支未改。
- `tests/decimal_blob_test.rs`：声明、写入、比较、主键、重开、超页、dump 和 G1 样本。

**偏差**

- `Value` 保存 17 字节线格式，`ValueRef` 借用这段字节。这样现有 `ValueRef` 大小上限仍然成立，往返仍保留写入时的 scale。
- 只把 `DECIMAL` 映射为新类型。`NUMERIC` 与 `DEC` 继续落入原来的 String 兜底。任务点名的是 `DECIMAL`。
- `CREATE TABLE AS SELECT` 从 DECIMAL 值推导列时，precision 取 `max(scale, 38)`。任务没有单独的推导场景。
- 全局最优里，显式主键只对 DECIMAL 放开。BLOB 主键仍走原有非整数拒绝。任务要求的是 DECIMAL 主键等值可达，并且 `to_key` 仍为 `None`。

**自检**

Gate 3：T1 的标签、往返、旧标签和规范形相等已由 `decimal_and_blob_round_trip` 覆盖。T2 的声明、精确写入、拒绝、数值比较、跨类型错误、主键、重开、超页、dump 和算术拒绝已由 `decimal_blob_test` 覆盖。T3 的 G1 打开、alice 行和 dave 写入已由同文件的 `legacy_g1_opens_queries_and_writes` 覆盖。样本文件字节未改。`FORMAT_VERSION` 未改。

参数表达式的零拷贝 `evaluate_ref` 对 DECIMAL 和 BLOB 返回 Null，与既有 String 臂相同。`execute_with_params` 走拥有值通道，空 BLOB 写入已在测试中通过。

无未修复的 Critical 或 Important。遗留 Minor：零拷贝参数通道不借出 DECIMAL 和 BLOB。

Gate 4：T1、T2、T3 的 Verification 满足。Gate 5：未命中。

T4 全量：`SKIPPED: 本 Cycle 排除全量，Iteration 001 尚未展开`。

**验证**

RED：

- `cargo test --lib decimal_and_blob_round_trip -- --test-threads=1`：退出码 101。类型尚不存在，17 个编译错误。没有运行测试。
- `cargo test --test decimal_blob_test schema_shows_decimal_and_blob -- --test-threads=1`：退出码 101。schema 文本是 `CREATE TABLE "t" ("d" TEXT, "b" TEXT);`。`0 passed; 1 failed`。

GREEN：

- `cargo test --lib decimal_and_blob_round_trip -- --test-threads=1`：退出码 0。`1 passed; 0 failed`。首次实现后曾因列名长度不同使目录行长度断言失败；断言改为同名列后通过。
- `cargo test --lib serialize_int_roundtrip -- --test-threads=1`：退出码 0。`1 passed; 0 failed`。
- `cargo test --test decimal_blob_test -- --test-threads=1`：退出码 0。`5 passed; 0 failed`。第一次整文件运行在重开库时失败，原因是前一个 `Database` 句柄仍持有锁；`drop` 后再运行通过。
- `cargo test --lib value_bytes_uses_fixed_measure -- --test-threads=1`：退出码 0。`1 passed; 0 failed`。DECIMAL 为 16，3 字节 BLOB 为 19。

全量未跑。

**Persisted Evidence**

None required。

**Experience Candidates**

None

**未解决**

无。

## Plan Review

- Review Result: rework-required

**Findings**

父 Cycle 的场景级行为与代码一致，定向测试结论沿用，没有重跑。HEAD `ee3ed58e3b127220cf5e41bb17cb44717e755da6`，审查时工作区干净。diff 范围是 merge-base `f323d58` 到该 HEAD。

已满足，不构成缺口：

- T1：`decimal_and_blob_round_trip` 检查标签 `0x09`、`0x0A`、`0x0B`、系数 150 与 scale 2、字节 `68 69`、`0xFF` 拒绝、旧 Int 标签 `0x01`、1.50 与 1.5 的相等和哈希，以及 catalog 的 `0x07`/`0x08`。`FORMAT_VERSION` 仍是 1。
- T2 的场景：`decimal_blob_test` 覆盖 schema、精确写入、拒绝舍入、数值比较、跨类型错误、DECIMAL 主键、重开、超过 `INLINE_SLOT_MAX` 的字节、dump 两代相同、`d + 1` 失败。`INLINE_SLOT_MAX` 为 4074，测试里的 5000 字节会走现有溢出链。
- T3：`legacy_g1_opens_queries_and_writes` 打开副本，alice 行在插入前仍在，dave 可读，样本文件字节不变。
- OID 与计费：`row_description` 中 DECIMAL 为 1700、BLOB 为 17，长度都是 -1。`value_bytes` 对 DECIMAL 为 16，对 BLOB 为 16 加字节长度。Act 的 `value_bytes_uses_fixed_measure` 结论沿用。
- T4 标成 SKIPPED 符合本 Cycle 排除全量。Persisted Evidence 为 none，不因没有 evidence 目录记问题。

阻塞：

- Plan 遗漏。规格「声明类型」要求 `NUMERIC(p,s)` 与 `NUMERIC(p)` 成为 DECIMAL 列，缺精度或越界时拒绝。proposal 已锁定口径第 4 条要求未写精度的 NUMERIC 拒绝，并且 BYTEA、BINARY、VARBINARY 不接受长度。`convert_data_type` 只匹配 `DataType::Decimal` 与 `DataType::Blob`。sqlparser 0.44 把 NUMERIC、BYTEA、BINARY、VARBINARY 解析成独立变体，这些名字落入 String 兜底。Act 按任务正文只实现了 DECIMAL 与 BLOB，并在偏差中记录了这一点。任务 Required behavior 没有写下已经批准的 SHALL，原执行依据约束不了修复。
- Plan 遗漏。规格「同类型比较」要求 BLOB 作主键时等值过滤仍能读到该行。design 的键路径是两种类型都返回 `None`，等值走非 Int 分流。建表检查只放行 Int 和 Decimal，BLOB 主键返回 `Unsupported constraint: NON-INTEGER PRIMARY KEY`。`to_key` 对 Blob 已经是 `None`。任务正文只要求 DECIMAL 主键。

非阻塞：

- `Value::Decimal([u8; 17])` 保存 scale 加小端 `i128`。比较、哈希、显示和往返仍使用系数与 scale。这是同一负载的线格式。
- design 写了字符串与 DECIMAL 的 CAST 互转，以及 DECIMAL 到 Int 的精确转换。`CastType` 仍是原来的六种，这些转换落入 `Type mismatch`。本 Cycle 的 Acceptance 和规格场景没有 CAST。补上会扩大本轮验收，不进入返工。
- `overflow.rs` 的注释仍写值标签域 `0x01..=0x08`。`decode_locator` 只认 `0xFF`，新标签不会被当成定位字节。
- 参数表达式的 `evaluate_ref` 对 DECIMAL 和 BLOB 返回 Null，与既有 String 臂相同。Act 已记录。
- `ctas_value_type` 把 DECIMAL 精度写成 `scale.max(38)`。没有对应验收场景。
- `DEC` 继续是 String。规格没有要求 `DEC`。
- RTM 中的 `exact_decimal_and_blob_round_trip_sql` 与 `decimal_orders_numerically` 不是独立测试函数。对应断言在 `exact_write_compare_reopen_and_overflow`。

**Acceptance Gaps**

- NUMERIC 缺精度时仍能建表，`NUMERIC(10,2)` 与 `NUMERIC(10)` 不会成为 DECIMAL 列。
- BYTEA、BINARY、VARBINARY 不会成为 BLOB；`BINARY(n)` 与 `VARBINARY(n)` 不会因长度被拒绝。
- `BLOB PRIMARY KEY` 在建表时被拒绝，等值读回没有落地。

**Evidence**

沿用父 Cycle Act Response 的 GREEN，来源是本文件的验证节，覆盖范围在 `ee3ed58` 之后没有新的产品代码改动：

- `cargo test --lib decimal_and_blob_round_trip -- --test-threads=1`：退出码 0，`1 passed; 0 failed`。
- `cargo test --lib serialize_int_roundtrip -- --test-threads=1`：退出码 0，`1 passed; 0 failed`。
- `cargo test --test decimal_blob_test -- --test-threads=1`：退出码 0，`5 passed; 0 failed`。
- `cargo test --lib value_bytes_uses_fixed_measure -- --test-threads=1`：退出码 0，`1 passed; 0 failed`。

代码：`convert_data_type` 的匹配停在 `DataType::Decimal` 与 `DataType::Blob`；主键检查在 `ddl_dml.rs` 的 `ColumnType::Int` 与 `ColumnType::Decimal` 条件；`pk_type_known_non_int` 对非 Int 返回 true；`Value::to_key` 对 Blob 返回 `None`。解析变体来自 sqlparser 0.44 的 `parse_data_type`。本审查没有重跑测试，也没有跑全量。

**Follow-up Decision**

三项缺口都是当前 Iteration 已批准规格里的行为，不是新的验收边界。原 Task Definition 的 Required behavior 没有这些结果，Act 已经按该正文做完并记录偏差。有限修复不能再靠这份执行依据约束，所以不留在当前 Cycle。需求、Iteration Plan 和全量预算都不变，所以不是 replan。Iteration 001 的全量目录不展开。

**Next Cycle**

`iterations/000-decimal-blob/001-rework.md`

**Next Iteration**

None
