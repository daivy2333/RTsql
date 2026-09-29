# 非键列 INSERT/UPDATE 写入值与列声明类型之间无类型校验（类型不匹配值静默持久化）

- Status: closed
- Filed: 2026-09-25
- Closed: 2026-09-26
- Source: Plan Review（MS23 Iteration 001 修复轮 Review F4 finding；Act Response Remaining Issues #3 同源）
- Environment: Linux x86_64 / master 工作树（登记时为 MS23 F1 修复轮后，全量 1152 tests 基线；关闭时为 MS24 Iteration 000 交付后，全量 1239 tests）/ Rust 2021 + Tokio / sqlparser-rs 0.44

## 缺陷描述

写入路径对「非键、非唯一列」的 INSERT/UPDATE 值与列声明类型之间全程无类型校验，类型不匹配值被静默序列化落库：

- 预期：写入值与目标列声明类型不兼容时应显式拒绝（或经声明语义转换），不产生损坏数据。
- 实际：`UPDATE t SET int_col = 'abc' WHERE id = 1`、`INSERT INTO t VALUES ('abc', ...)`（int_col 为非 PK 普通 INT 列）均成功落库——计划期 `build_update`（`src/parser/planner/ddl_dml.rs:621-697`）对 SET 值无类型校验（列存在性也是运行期 Step 3 才查），`extract_insert_values` 同样不校验字面量类型与列类型；执行期类型门仅覆盖 PK 键列（MS16 `KeyTypeMismatch`，`src/executor/update.rs:110-121` 区）、日期 coerce 仅覆盖日期族（`coerce_datetime_write`）；`serialize_tuple`（`src/storage/page_format/tuple.rs:67`）按值变体序列化、不与 schema 交叉校验。
- 后果：损坏值持久化后，读回在 `deserialize_tuple` 处才暴露（Inferred——具体读回失败形态未独立测试确认，未实施验证）。
- 根因：Confirmed（上述位点均为登记时独立追读的当前实现）。

## 影响

当前：所有含非键列的表可经 INSERT/UPDATE 写入类型不匹配值（如 String/Float/Bool 写入 INT 列），数据以错误变体标签持久化，属「静默错误结果」正确性同类；无已知生产故障事件。潜在：损坏行读回报错或语义错乱；dump→restore、import CSV 通道的类型面未逐一验证（Inferred，可能同样无门）。

## 事件记录

None（缺陷在 MS23 Iteration 001 Plan Review 期间经代码审计定位，未爆发为独立故障事件。其唯一强制面边缘曾在 MS23 工作树内以 UPDATE 维护区 unwrap panic 形态短暂存在——未发布、未产生用户影响，已在同 change F1 修复轮收口：INT UNIQUE 列非 Int 值现为 `KeyTypeMismatch` 点名拒绝）。

## 处置

- Status: closed（2026-09-26，用户指令关闭）。关闭原因：`fixed`——缺陷面已由 change `2026-09-25-ms24-write-surface-completion` Iteration 000（R2 写入值类型一致门）端到端修复并通过验收，该 change 已收尾归档为 `openspec/changes/archive/2026-09-25-ms24-write-surface-completion/`。
- 修复落点：`StorageError::ColumnTypeMismatch { column, expected, actual }` 新变体（`src/storage/error.rs`）；`InsertExecutor`（既有 UNIQUE 预检之后、serialize 之前）与 `UpdateExecutor`（碰撞预检之后、serialize 之前）逐列校验，NULL 豁免、日期族 String 经既有 coerce 落类型后一致、FLOAT 列整数值就地升格改写 `new_value`、其余跨类型点名拒绝且零副作用；dump/restore 与 CSV import 通道不误报（同一门覆盖）。行为规格：`openspec/specs/sql-write-surface/spec.md` R2「写入值类型一致门」。
- 状态迁移记录：
  - 2026-09-25 `open` → `scheduled`：用户批准并入 MS24 新增任务行 MS24-T03（tasks.md）。
  - 2026-09-26 `scheduled` → `closed`：用户指令关闭；修复随 MS24 change 交付，`tests/write_type_conformance_test.rs` 13 用例矩阵（RED 先行 6 failed 见证）与全量 `cargo test` 1239 passed / 0 failed / 2 ignored 承载。
- 关联背景：唯一列边缘已由 MS23 change `2026-09-24-ms23-constraint-enforcement` F1 修复收口（INT UNIQUE 列 `KeyTypeMismatch` 守卫）；PK 键列面由 MS16 既有键位门覆盖。
- 登记时保留的判断（不改写）：「与既定『显式类型检查、不隐式转换』语义对齐，避免隐式转换漂移」——修复仅对 FLOAT 列做无损升格，未引入其他隐式转换。
- 遗留（不属本 Issue 面，另行跟踪）：声明期 DEFAULT 字面量与列类型的校验缺失（Bool 列 `DEFAULT 1` 建表被接受、应用默认值时才被写入门拒绝）——tasks.md `MS24-T04`，`planned` 未实施。

## 证据

- 登记时证据：`openspec/changes/archive/2026-09-24-ms23-constraint-enforcement/iterations/001-unique-enforcement/000-initial.md` — Plan Review（F4 定性与 F1 修复收口记录）与 Act Response（Remaining Issues #3）
- 登记时代码位点（2026-09-25 独立追读，MS23 F1 修复轮工作树）：`src/parser/planner/ddl_dml.rs:621-697`（build_update 无 SET 类型校验）、`src/executor/update.rs:110-121`（MS16 仅 PK 门）、`src/executor/value.rs:92-104`（`to_key` 仅 Int 产键）、`src/storage/page_format/tuple.rs:47-67`（serialize/compute 无 schema 交叉校验）
- 关闭时证据：`openspec/changes/archive/2026-09-25-ms24-write-surface-completion/iterations/000-type-gate-subset-insert/000-initial.md` — 任务 1.1-1.3 契约与 Act Response Verification 表（`cargo test --test write_type_conformance_test` 13 passed、RED 先行 6 failed；`cargo test --lib` 311 passed；全量 `cargo test` 1182 passed / 0 failed / 2 ignored exit 0）；行为规格 `openspec/specs/sql-write-surface/spec.md` R2 及其 6 场景；`openspec/changes/archive/2026-09-25-ms24-write-surface-completion/iterations/001-upsert-replace/001-replan.md` 采信后续全量 1239 tests（表面零变化）
