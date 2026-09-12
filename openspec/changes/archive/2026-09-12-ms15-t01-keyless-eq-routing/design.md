# design — MS15-T01 键位等值路由修复

## D1: 修复方向——字面量可键控性判定（I036 已批准方案）

按 I036 已记录方案与 MS15-T01 milestone 批准文本实施：**键位等值腿的字面量不可键控（`to_key()==None`）时禁用索引路由**，回退数据页行内求值。

备选方向 B（键列类型感知：planner `register_table` 加性传递 `ColumnType`，键列非 Int 时全部键位等值形态统一回退 DataScan）可同时覆盖形态 2（Int 字面量 + Float 键列，探针实锤静默漏行）与统一判定模型，但超出本 change 已批准范围（需扩 planner 元数据面 + 触达更多既有 plan 形状），记录于 proposal Out of Scope 待用户裁定。本 change 的路由判定点（`has_pk_equality` 分支条件）即方向 B 的将来落点，扩展不冲突。

可键控性权威定义：`Value::to_key()`（`src/executor/value.rs:82-90`）仅 `Value::Int` 返回 `Some`；String/Null/Float/Bool 均为 `None`。字面量经 `value_from_sqlparser`（`src/parser/value.rs:8-30`，覆盖 Number→Int 优先/Float、SingleQuotedString、Null、Boolean）。

## D2: 路由语义（WHERE 形态 × 修复前后 plan）

单表 SELECT、`base_plan = Scan` 的完整路由表（`src/parser/planner/query.rs:501-586`）：

| WHERE 形态 | 修复前 | 修复后 | 依据 |
|---|---|---|---|
| 顶层 `pk = <Int 字面量>`（简单） | `IndexScan` | `IndexScan`（不变） | extract Some + is_simple 路径 |
| 顶层 `pk = <Int 字面量>` + AND | `Filter(Scan)` | `Filter(Scan)`（不变） | extract Some + 非 simple 路径 |
| 键位等值腿含**不可键控字面量**（String/Float/Bool/NULL；简单或 AND 内） | `Filter(Scan)`（缺陷：索引遍历漏无键行） | 含 OR → `Filter(DataScan)`；否则谓词下推 `DataScan` | **本 change 修改点**：has_pk_eq 分支不放行，落入既有 OR/下推臂 |
| 键位等值腿均为可键控字面量或非字面量（列-列等）+ AND | `Filter(Scan)` | `Filter(Scan)`（不变） | 无键行被键位等值三值语义排除（NULL→Unknown；非 Int 键值 vs Int 字面量跨类型 false），索引遍历不漏行 |
| 无键位等值 + OR | `Filter(DataScan)` | 不变 | 既有臂 |
| 无键位等值、无 OR | 谓词下推 `DataScan` | 不变 | 既有臂（MS07-T06） |
| 非 Eq 运算符（Ne/Gt/…） | 谓词下推 `DataScan` | 不变 | `has_pk_equality` 仅认 Eq |

实现形态（建议，非实质细节留给 Act）：`has_pk_equality` 的结构遍历（Eq 腿 + AND 递归、OR 保守 false）升级为分类判定——返回「存在不可键控字面量腿 / 否」二值或枚举；分支条件由 `if has_pk_eq` 变为 `if has_pk_eq && 无不可键控字面量腿`。键位等值腿 = `Eq` 且一侧为键列 `Identifier`、另一侧为 `Expr::Value`；`value_from_sqlparser` 在该腿上失败（畸形字面量变体）时按既有 `extract_pk_from_where` 同样传播 `PlanError`（与顶层形态的现行错误时序一致）。

## D3: 边界语义

- **NULL 字面量腿**（`WHERE s = NULL`）：不可键控 → 路由 DataScan → 行内三值求值 Unknown → 空集；与修复前 `Filter(Scan)` 空集可观察一致（R3-S1 同理覆盖）。
- **负数字面量**（`WHERE id = -5`）：sqlparser 解析为 `UnaryOp`，非 `Expr::Value` 腿 → 分类不触发 → `Filter(Scan)` 既有路径保持（行内求值语义不变）。
- **列-列腿**（`WHERE id = n`）：非字面量腿 → 分类不触发 → 既有路由保持（正确性已分析，见 D2 表）。
- **plan cache**：键为 SQL 文本规范化（MS06-T02），路由变化按文本自然分键，无缓存污染面。
- **谓词与投影**：下推臂传入 `proj_or_empty`，谓词在投影裁剪前按全行求值（MS10-T01 真投影既有机制，`keyless_row_test.rs:66` 非键列下推先例同机）。

## D4: 方向 A 下的明确残差（本 change 不修）

1. **形态 2**：可键控 Int 字面量 + 非 Int 键列（Float 隐式 PK 表 `WHERE f = 5`，行值 5.0）→ `extract_pk_from_where` 返回键 5 → `IndexScan` 点查空索引 → 静默漏行（`5.0=5` 按 `Value::equals` 隐式转换应匹配；探针实锤 2026-09-12）。String/Bool 键列 + Int 字面量因跨类型 equals 恒 false 而巧合正确，仅 Float（Int↔Float 隐式转换）真实漏行。处置待用户裁定（见 proposal Out of Scope）。
2. **参数腿**：planner 无 `Expr::Parameter` 引用（grep 实证），参数化 WHERE 不达路由面，理论残差。
3. **子查询上下文**：单表路由修复随共享代码路径自然生效；派生表形状的本 change 不单独验证。

## D5: 测试策略

- 新增 `tests/keyless_eq_routing_test.rs`：R1 五场景（含 restart）行为断言 + 修复形态 plan 形状断言（非键控腿 → `DataScan` / `Filter(DataScan)`）；T1 阶段观察 RED（行为断言失败 + plan 形状不符），T2 后 GREEN。
- R2 锁定：复用既有 `pushdown_test.rs` 两用例（零修改通过即见证）；R3-S1 空结果不变用例新增于同文件（变更前后均 GREEN，锁路径变化不锁结果）。
- 收尾：全量 `cargo test`（基线 845 passed / 0 failed / 2 ignored，只增不减）、clippy/fmt 0、`openspec validate` PASS、CLI 探针复核（三缺陷形态输出正确行集）。
- 既有测试冲突排查结论（2026-09-12）：`pushdown_test` PK 形状断言均用可键控 Int 字面量；`expression_e2e_test.rs:521`、`scalar_function_test.rs:239-252` 的 `WHERE s = '...'` 均在声明 Int PK 表上（s 为非键列，走既有下推臂）；`keyless_row_test.rs` WHERE 均为非键列或可键控 Int。无锁定冲突。

## Risks and Notes

- has_pk_eq 分支放行条件收窄后，`is_simple_pk_equality`（`query.rs:781`，仅 extract Some 路径可达）不受影响，无连带修改。
- 分类遍历与 `extract_pk_from_where` 对字面量的解析共用 `value_from_sqlparser`，无第二转换源。
- 工作区含 MS11-T03 未提交改动（基线 179228b + 实施变更）：Act 建议在用户 commit 后开始，避免验证基线混叠；若用户选择先实施，全量基线以实施时点实测为准（845 通过状态经 Plan Review 独立复跑，2026-09-11）。
