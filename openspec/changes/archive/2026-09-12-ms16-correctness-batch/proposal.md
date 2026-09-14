# MS16 正确性收口第二批：键位等值全形态可达 + 键列写入类型强制 + rekey 索引一致性

## Why

MS15 收尾裁定留下两项已实证的「静默错误结果」类残差（improvements I046/I047，均有探针或代码级证据），同属「索引可达性/一致性」故障域，先行于一切新能力收口。本 change 调查期间（2026-09-12，revision d8a244f 二进制探针）又发现同根缺陷的第三形态：

- `INSERT INTO t (id INT PRIMARY KEY, v INT) VALUES (5.0, 1)` 不做列类型校验、成功落库（序列化按值打 tag），该行键位为 Float、成为无键行，`SELECT * FROM t WHERE id = 5` 经 IndexScan 静默空集——**Int 键列上同样存在 I046 类漏行**，方向 B（按声明类型路由）修不到。

用户裁定（2026-09-12 原话：「既然在这里发现另外的缺陷，我们顺便就在同一个change计划然后修复了吧，不登记直接计划解决吧，继续」）：该残差不登记 improvement，并入本 change 直接修复——根因收口为键列写入类型强制（Int 键列只接受 Int 或 NULL），使「Int 键列 ⇒ IndexScan 路由可靠」成为可维护的不变量，与路由侧类型感知互为前提。

三项工作聚合为同一阶段成果「MS16 正确性收口第二批」：初版前不携带已知静默错误结果。

## What Changes

1. **键列类型感知路由（I046 方向 B）**——`PlanBuilder` 加性传递键列声明类型（`pipeline.rs` 注册点已有 `ColumnType`）；SELECT 单表 WHERE 路由两处判定（`extract_pk_from_where` 臂、`has_pk_eq` → `Filter(Scan)` 臂）加类型门：键列声明类型非 Int（Float/String/Bool）时键位等值形态（简单/AND/反向）统一回退既有谓词下推 `DataScan` / `Filter(DataScan)` 臂，行内求值按 `Value::equals` 语义（Int↔Float 隐式转换已确认，value.rs:118-137）。Int 键列路由形状逐字节不变。
2. **键列写入类型强制（调查新发现，根因收口）**——`InsertExecutor`（键位值，先于 DuplicateKey 预检）与 `UpdateExecutor`（SET 键列，写入数据页/WAL/索引之前）前置校验：Int 键列只接受 Int 或 NULL，越界类型报新加性错误 `StorageError::KeyTypeMismatch`（exit 3，零副作用）。非 Int 键列不新增拒绝（正确性无必要，行为保持）。
3. **rekey 索引维护（I047）**——`UpdateExecutor` Step 7 三分支化：新值不可键控 → 删旧键条目（I037 语义保持）；新键 == 旧键 → 既有 update 路径保持；新键可键控且不等 → **任何写入前**以新键查索引，命中即 `DuplicateKey` 拒绝（零副作用，与 INSERT 先查后写模式一致），否则删旧键条目 + 插新键条目。此后新键点查可达、旧键点查空集、旧键 INSERT 不再误拒、崩溃恢复两态一致。
4. **INSERT 列清单映射修复（Plan Review 审计裁定并入，BH-2）**——`InsertNode.columns` 全链路无消费点（值按表列序位置解释）：乱序清单静默错位、部分清单触发 `tuple.rs:38` 断言 panic（exit 101）、未知列静默接受（三者均探针实证）。`build_insert` 在 plan 期校验列清单（恰为表列的一个排列：未知/重复/数量不符拒绝）并按清单重排值序；键位类型校验作用于重排后的键位值。新增 delta spec `insert-column-list-mapping`。

Delta specs：

- 修改 `planner-key-equality-routing`：「可键控字面量路由保持」收窄至 Int 键列 + 新增「键列类型感知路由」。
- 新增 `key-column-type-conformance`：Int 键列写入类型强制（含既有测试 `negative_number_literal_persists` 校准记录）。
- 修改 `update-index-maintenance`：新增「键位 rekey 后索引条目一致」。
- 新增 `insert-column-list-mapping`：INSERT 显式列清单映射语义。

## Out of Scope / Non-goals

- UPDATE/DELETE 键位等值对非 Int 键列维持 `KeyNotFound`（响亮报错，与 update-index-maintenance R1-S4「键位无键行对键位等值 UPDATE 不可达」已验收语义一致；用户 2026-09-12 批准保持）。
- 非键列与非 Int 键列的类型同型性强制（无正确性影响，留类型系统域）；Float 键列接受 Int 值等既有宽松行为保持。
- 既有库中已存在的越界键位行不做数据迁移（预发布产品；此后越界写入被拒绝，存量行经非键谓词仍可达）。
- restore 对历史越界 dump 文本（Float-into-Int 键列 INSERT）将响亮失败——预发布可接受，记录于 design D5；import 不受影响（`csv_value` 按列声明类型转换，lifecycle.rs:502）。
- I033/I032（→MS09）、I038/I031/I021（→MS08 实测域）、I035（→MS13）、I041 测试竞态（建议独立小 change 随带）、性能优化。

## 用户决策记录（Gate 1，2026-09-12）

1. I046 残差处置：初选「登记新 improvement」，随后用户改为**并入本 change 直接修复、不登记**（原话见上）。
2. I047 碰撞语义：**写入前拒绝**（与 INSERT 先查后写一致，拒绝零副作用）。
3. UPDATE/DELETE 非 Int 键列键位等值：**保持 KeyNotFound**，本 change 不扩突变路由。
4. Gate 1：批准需求基线与范围，继续创建 change 并规划（范围随后按决策 1 扩展为 R1-R5）。

## Plan Review 裁定记录（2026-09-12，Iteration 000 首轮审计）

5. BH-1（既有测试 `negative_number_literal_persists` 依赖「Int 隐式键列收负 Float 字面量」，与 R3 强制冲突，全量唯一确定性失败）：**校准该测试**——负 Int 行保持原表原断言；负 Float 行移入 Float 键列表 `tf(f FLOAT, s STRING)`，I040 覆盖（负 Int/负 Float 字面量折叠）完整保留；校准在 `key-column-type-conformance` delta spec 按 T8-R2 先例记录。
6. BH-2（INSERT 列清单全链路无消费点：乱序静默错位 / 部分清单 panic / 未知列静默接受）：**并入本 change 修复**（T7 列清单映射，plan 期校验 + 重排），R3 显式列序场景在映射修复后可见证。
7. Review Result：`replan-required`——同 Iteration 创建 `001-replan` Cycle 执行修订后计划（T7/T8）；replan 计划获批（Gate 2 复查）后交 Act。

## Plan Review 裁定记录（2026-09-13，Iteration 001 首轮审计）

8. BH-3（M10 时代直连执行器单测依赖 rekey 缺陷行为：`gc_test` 3 用例、`version_chain_test` 2 用例、`plan_exec_test::test_insert_update_scan_flow` 以「SET 键列 = 另一 Int 值」建链后按**旧键** search/IndexScan 定位/寻址，R4 实施后全量 6 处确定性失败）：**校准该 6 用例**——测试主题与断言语义保持，寻址与定位改用行当前键（rekey 后新值键）；受影响面经 UpdateExecutor 全部测试用法排查闭合（恰 6 用例）；校准在 `update-index-maintenance` delta spec 按 BH-1 先例记录，执行见 tasks T9。
9. Review Result：`replan-required`——Iteration 001 验收基线「全量回归零修改」被证伪，验证契约需修订（校准例外）；同 Iteration 创建 `001-replan` Cycle 执行修订后计划（T9 + 全量收尾）；replan 计划获批（Gate 2 复查）后交 Act。
