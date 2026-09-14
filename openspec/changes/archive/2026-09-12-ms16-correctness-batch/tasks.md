# tasks — MS16 正确性收口第二批（I046 + 键列写入类型强制 + I047 + INSERT 列清单映射）

> 规划：openspec-plan 2026-09-12；Gate 1 已批准（2026-09-12：R1-R5 基线 + 集中决策三项；范围按用户指示扩展——探针新发现并入直接修复、不登记 improvement，原话见 proposal）。
> 设计：design.md D1-D7。
> 状态：**Iteration 000 完成**（Plan Review 2026-09-12 accepted）——000-initial（T1/T2 完成，T3/T4 因 BH-1/BH-2 阻塞，Plan Review 裁定 replan-required，见 proposal 裁定记录 5-7）；001-replan（用户 2026-09-12 批准 Gate 2，原话「更改gate状态，开始实施」）：T7/T8 完成，T3 显式列序场景补写（8/8）、T4 收尾解除，全量 888 passed / 0 failed / 2 ignored（Plan Review 独立复跑一致）；001-replan Plan Review accepted（gaps 闭合、F5 Act Minor 勘误为 I045 既有噪声）。**Iteration 001 进行中**——000-initial（T5 完成 RED 见证；T6 实现 + 目标套件 9 GREEN + 锚点 26 GREEN，全量收尾因 BH-3 阻塞，Plan Review 2026-09-13 裁定 replan-required，见 proposal 裁定记录 8-9）；001-replan（用户 2026-09-13 批准 Gate 2，原话「批准」）：T9 完成——6 用例校准（寻址/定位改行当前键，断言语义零改动）+ 全量收尾 892 passed / 0 failed / 2 ignored、clippy/fmt/validate 全过；001-replan Plan Review **accepted**（Plan 独立复跑一致：三校准套件 3+4+3 GREEN、全量 892/0/2、`src/` 零改动、受影响面闭合经受全量证实）。**本 change 全部 Iteration 完成（R1-R6 Covered），达收尾条件**——待 openspec-docs-maintainer 收尾（delta specs 合并主语料库、change 归档、SNAPSHOT/tasks 同步、I046/I047 处置落账）。

## Task List

| Task | 状态 | 目标 | 关键产出 | Iteration |
|---|---|---|---|---|
| T1 | done | RED 测试见证：键列类型感知路由（R1） | `tests/keyless_eq_routing_test.rs` 扩展 6 用例：Float 键列 Int 字面量简单/AND/反向行集断言 + plan 形状 `DataScan` 谓词下推断言 + String 键列 Int 字面量结果不变 + restart 保持；RED 5 failed/1 新增 GREEN 按预测观察 | 000 |
| T2 | done | 路由实现：键列类型传递 + 两处判定门（design D1/D2） | `src/parser/planner/mod.rs` 加性 `primary_key_types` + `set_pk_column_type`；`src/pipeline.rs` 注册点接线；`src/parser/planner/query.rs` `pk_type_known_non_int` + `extract_pk_from_where_gated`（门 1）+ 非 PK 臂条件扩展（门 2）；T1 14 passed 全绿 + pushdown 16 passed 锚点零修改 | 000 |
| T3 | done | RED 测试见证：Int 键列写入类型强制（R3） | 新建 `tests/key_type_conformance_test.rs` 7 用例：拒绝矩阵 + 零副作用 + NULL/非 Int 键列锚点；RED 4 failed 按预测观察；显式列序场景（R3-S8）在 T7 列清单映射修复后补写（001-replan T8），8/8 场景齐备 | 000 |
| T4 | done | 强制实现：执行器前置校验与错误变体（design D3） | `src/storage/error.rs` 加性 `KeyTypeMismatch`；`src/executor/insert.rs` 键位类型预检（先于 DuplicateKey 预检）；`src/executor/update.rs` Step 1 后键列校验；T3 8 用例全 GREEN + keyless_row 4 + update_index_maintenance 5 锚点全绿；全量收尾随 T8 解除（888 passed / 0 failed） | 000 |
| T7 | done | INSERT 列清单映射修复（R6，design D7，BH-2 裁定并入） | `src/parser/planner/ddl_dml.rs::build_insert` plan 期列清单校验（恰为表列排列：未知/重复/数量不符拒绝；无清单时行长度校验）+ `map_insert_values` 按清单重排 values + 共享 `insert_count_error` 文案；新建 `tests/insert_column_list_test.rs` 7 用例（6 场景 + 重复列 SHALL）；RED 6 failed/1 passed 按预测（部分清单/无清单 panic 于 tuple.rs:38）→ GREEN 7 passed | 000 |
| T8 | done | BH-1 校准 + R3 显式列序场景补写 + Iteration 000 全量收尾 | `tests/expression_e2e_test.rs::negative_number_literal_persists` 按 design D3 校准（负 Float 行移入 Float 键列表 `tf`，I040 覆盖保持，`t` 表断言逐字节保留）；`tests/key_type_conformance_test.rs` 补写 R3-S8 显式列序场景（重排后 id 收 5.0 → KeyTypeMismatch + COUNT 0）；`cargo test` 全量 888 passed / 0 failed / 2 ignored、clippy --all-targets -D warnings 0、fmt --check clean、`openspec validate` changes 1 PASS / specs 25 PASS | 000 |
| T5 | done | RED 测试见证：rekey 索引一致性（R4） | `tests/update_index_maintenance_test.rs` 尾部追加 I047 段 4 用例：新键可达 / 旧键清理（点查空集 + INSERT 可用）/ 碰撞写入前拒绝零副作用 / 恢复两态一致；RED 4 failed 形态与 Plan Context 预测逐条一致 + 既有 5 用例 GREEN（2026-09-13 观察） | 001 |
| T6 | done | rekey 实现：写入前碰撞预检 + Step 7 三分支（design D4） | `src/executor/update.rs` 前置块碰撞预检（新键命中即 DuplicateKey，任何写入前）+ Step 7 三分支（NULL 删 / 同键 update / rekey 先删后插）；目标套件 9 passed + 锚点套件 26 passed 全绿；全量收尾因 BH-3 阻塞移交 T9 | 001 |
| T9 | done | BH-3 校准：6 个依赖 rekey 缺陷行为的既有直连执行器测试 + Iteration 001 全量收尾 | `tests/gc_test.rs` 3 用例 + `tests/version_chain_test.rs` 2 用例 + `tests/plan_exec_test.rs::test_insert_update_scan_flow` 寻址/定位改用行当前键 + BH-3 校准注释（design D4 校准段，3 文件 +30/−7，`src/` 零触碰）；校准前 RED 与 Plan 预测逐条一致，校准后目标三套件 GREEN（gc 3 + version_chain 3 + plan_exec 4）+ 全量 892 passed / 0 failed / 2 ignored（I041 未触发）+ clippy 0 + fmt clean + openspec validate changes 1 PASS / specs 25 PASS | 001 |

## Iteration Plan

### Iteration 000: 键位等值全形态可达（路由类型门 + 键列写入类型强制 + INSERT 列清单映射）

- Tasks: T1, T2, T3, T4（000-initial 已执行，见其 Act Response）+ T7, T8（001-replan 执行，Plan Review 裁定新增）
- Depends on: None
- Stable baseline: 非 Int 键列键位等值全形态正确可达（DataScan 求值，restart 保持）；Int 键列路由形状与结果逐字节保持；Int 键列越界写入显式拒绝零副作用、NULL/合规写入与非 Int 键列行为保持；INSERT 列清单恰为表列排列时值正确落位（键位校验作用于重排后键位值）、非法清单计划期响亮拒绝（panic 消除）；既有测试除 `negative_number_literal_persists` 按 design D3 校准外零修改；全量回归 0 failed（基线 867 只增不减）
- Verification boundary: T1-T4 + T7/T8 全绿 + 既有锚点套件零修改（T8 校准除外）+ clippy/fmt 0 + `openspec validate` PASS
- Diagnostic boundary: `src/parser/planner/{mod,query,ddl_dml}.rs` + `src/pipeline.rs` 注册点 + `src/executor/{insert,update}.rs` 前置校验 + `src/storage/error.rs` + 测试四文件（keyless_eq_routing / key_type_conformance / insert_column_list / expression_e2e 校准）
- Non-goals: rekey 索引维护（→001）；UPDATE/DELETE 非 Int 键列 KeyNotFound 保持；非键列类型校验；partial INSERT（NULL 填充）支持；存量越界行迁移

### Iteration 001: rekey 索引一致性（I047）

- Tasks: T5, T6, T9（T9 为 2026-09-13 Plan Review BH-3 裁定新增）
- Depends on: Iteration 000（写入前校验区已建立，T6 碰撞预检并入同区；Step 7 既有 I037 分支为 T6 三分支基线）
- Stable baseline: rekey 后新键点查可达、旧键点查空集、旧键 INSERT 可用、碰撞写入前拒绝零副作用、恢复两态一致；同键/NULL/非键列分支逐字节保持；全量回归除 T9 校准 6 处外零修改（BH-3 裁定，原「全量回归零修改」已证伪修订）
- Verification boundary: T5/T6/T9 全绿 + 既有锚点套件零修改（T9 校准 6 处除外）+ 全量 0 failed + clippy/fmt 0 + `openspec validate` PASS
- Diagnostic boundary: `src/executor/update.rs` + `tests/update_index_maintenance_test.rs` + 三校准测试文件（gc_test / version_chain_test / plan_exec_test）
- Non-goals: 多行 UPDATE 语义（planner 单行契约不变）；索引层 DuplicateKey 机制（node.rs 禁用检查不恢复）；性能优化

### 平衡审计

- Iteration 000：路由（planner）、写入强制（executor）、列清单映射（planner build_insert）跨模块，但共同形成单一可验证成果「键位等值过滤对全类型键列与全可写数据形态行集正确」——路由门修正声明类型面、写入强制封堵存储值面、列清单映射保证键位值按用户赋值到达键位（显式列序场景的前提），缺一该成果不成立；T7 由 Plan Review 裁定并入（BH-2 同域缺陷），不改变成果边界；不拆分。
- Iteration 001：独立验收成果（索引两态一致性），故障域（update 执行器索引维护）与 000 的路由/写入契约面不同；依赖 000 的前置校验区结构，故后置。
- 无过碎（每 Iteration 均有独立 spec delta 与 RED→GREEN 见证）与过重（单 Iteration 不跨验收成果）问题。

## 相关依据

- improvements I046（MS15-T01 调查新发现 + 用户方向 B 裁定）、I047（MS15-Rest 调查新发现 + 探针实证）；本 change 调查补充探针新发现（Int 键列 + Float 存储值，2026-09-12，d8a244f 二进制探针），经用户裁定并入直接修复
- Plan Review 2026-09-12 审计新裁定：BH-1 校准、BH-2 列清单映射并入（proposal 裁定记录 5-7；三形态探针实证：错位/panic exit 101/未知列接受）
- Plan Review 2026-09-13 审计新裁定：BH-3 校准（6 个直连执行器测试依赖 rekey 缺陷行为——gc_test ×3 / version_chain_test ×2 / plan_exec_test ×1，按行当前键寻址；UpdateExecutor 全部测试用法排查闭合受影响面；proposal 裁定记录 8-9，Review Result replan-required）
- tasks MS16-T01/T02（2026-09-12 路线重排新建 MS16，ready，无前置依赖）
