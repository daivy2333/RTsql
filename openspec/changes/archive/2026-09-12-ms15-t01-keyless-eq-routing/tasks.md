# tasks — MS15-T01 键位等值路由修复（I036）

> 规划：openspec-plan 2026-09-12；Gate 1 已批准（2026-09-12 用户批准：范围 = I036 已记录方案（方向 A）；形态 2 处置 = 独立后续 change，方向 B 为候选，待 I 登记）。
> 设计：design.md D1-D5。
> 状态：Iteration 000 已实施且 Plan Review accepted（2026-09-12，Act Response + Review 见 `iterations/000-initial/000-initial.md`；Next Iteration: None，实施侧完成，收尾归档待用户指令）。Plan Context 见 `iterations/000-initial/000-initial.md`（ready）。

## Task List

| Task | 状态 | 目标 | 关键产出 | Iteration |
|---|---|---|---|---|
| T1 | done | RED 测试见证：R1 四行为场景 + plan 形状断言 | 新增 `tests/keyless_eq_routing_test.rs`：String 隐式键列简单等值 / AND 组合 / Float 键列 Float 字面量 / 声明 TEXT PRIMARY KEY 行集断言（目标行为，修复前 RED）+ 非键控腿 plan 形状 `DataScan`/`Filter(DataScan)` 断言 | 000 |
| T2 | done | 路由修复实现（design D2） | `src/parser/planner/query.rs`：`has_non_keyable_pk_literal_leg` 分类 helper + has_pk_eq 分支条件收窄，非键控腿落入既有 OR/下推臂；T1 全部转 GREEN | 000 |
| T3 | done | R2/R3 回归锁定 | 同测试文件追加：R3-S1 Int 键列非 Int 字面量空结果不变（变更前后 GREEN）、R1-S5 restart 可达性；复核 `pushdown_test.rs` 两 PK 形状用例零修改通过 | 000 |
| T4 | done | 全量收尾 | `cargo test` 全量 853 passed / 0 failed / 2 ignored（基线 845 只增不减）、clippy/fmt 0、`openspec validate --changes` PASS、CLI 探针复核三缺陷形态输出正确行集 | 000 |

## Iteration Plan

### Iteration 000: 键位等值对无键行全形态可达 + 可键控路由零回归

- Tasks: T1, T2, T3, T4
- Depends on: None
- Stable baseline: R1 五场景全绿（无键行键位等值全形态可达，restart 保持）；R2/R3 锁定绿（可键控路由形状与既有结果逐字节保持）；全量回归零修改
- Verification boundary: T3/T4 全绿 + clippy/fmt 0 + validate PASS + CLI 探针复核
- Diagnostic boundary: `src/parser/planner/query.rs`（SELECT 单表 WHERE 路由段）+ `tests/keyless_eq_routing_test.rs`
- Non-goals: 形态 2（待用户裁定）、I037/I034/I039、GC、OR→IndexScan 优化、存储/执行器层修改

### 平衡审计

单 Iteration：全部任务服务同一可验证成果（键位等值路由正确性），改动面单点（query.rs 路由段）+ 单测试文件，工作量适中（1 判定扩展 + 1 分支条件 + 1 测试文件 ~10 用例），稳定基线/验证边界/诊断边界独立清晰；无过碎（不产生不可独立验证的半成果）与过重（不跨故障域）问题。

## 相关依据

- improvements I036（MS10-T05 001-rework + MS11-T03 双重实证）；本 change 调查补充三形态探针实锤（2026-09-12，含形态 2 新发现）
- tasks MS15-T01（缺陷依据：I036）；执行序 MS15 首项
