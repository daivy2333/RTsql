# 非键列 INSERT/UPDATE 写入值与列声明类型之间无类型校验（类型不匹配值静默持久化）

- Status: open
- Filed: 2026-09-25
- Source: Plan Review（MS23 Iteration 001 修复轮 Review F4 finding；Act Response Remaining Issues #3 同源）
- Environment: Linux x86_64 / master 工作树（MS23 F1 修复轮后，全量 1152 tests 基线）/ Rust 2021 + Tokio / sqlparser-rs 0.44

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

- Status: open，待用户决定立项（未排期 MS）。
- 关联：发现源为 MS23 Plan Review F4；唯一列边缘已由 MS23 change `2026-09-24-ms23-constraint-enforcement` F1 修复收口（INT UNIQUE 列 KeyTypeMismatch 守卫，`src/executor/insert.rs`/`update.rs`）；PK 键列面由 MS16 既有键位门覆盖；本 Issue 指非键非唯一列的残余面。
- 建议排查面：`build_update`/`extract_insert_values` 计划期类型门或执行器写入前置校验；`serialize_tuple` schema 交叉校验评估；dump/restore/import 通道类型面核查。
- 立项注意：与既定「显式类型检查、不隐式转换」行为（README 类型语义段）对齐，避免修复引入隐式转换语义漂移。

## 证据

- `openspec/changes/2026-09-24-ms23-constraint-enforcement/iterations/001-unique-enforcement/000-initial.md` — Plan Review（F4 定性与 F1 修复收口记录）与 Act Response（Remaining Issues #3）
- 代码位点（2026-09-25 独立追读，MS23 F1 修复轮工作树）：`src/parser/planner/ddl_dml.rs:621-697`（build_update 无 SET 类型校验）、`src/executor/update.rs:110-121`（MS16 仅 PK 门）、`src/executor/value.rs:92-104`（`to_key` 仅 Int 产键）、`src/storage/page_format/tuple.rs:47-67`（serialize/compute 无 schema 交叉校验）
