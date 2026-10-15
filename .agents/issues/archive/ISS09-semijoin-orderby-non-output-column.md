# ISS09 IN/EXISTS 子查询（SemiJoin 路径）下 ORDER BY 非输出列 42703

- Status: closed
- Filed: 2026-10-12
- Source: 外部评测（2026-10-12，P1-5）+ Explorer 核实（`.agents/analysis/2026-10-12-external-eval-defect-verification.md`，R32）
- Environment: Linux x86_64 (WSL2)、rtsql 0.2.0、HEAD `44d04d9` debug 构建

## 缺陷描述

WHERE 带 IN/EXISTS 子查询时，ORDER BY 引用表内非输出列计划期 42703；同查询去掉子查询、排输出列、或 IN 改字面量清单均正常：

- ✗ `SELECT name FROM people WHERE id IN (SELECT did FROM dept) ORDER BY id` → 42703 `Column not found: id`
- ✓ 同查询无 ORDER BY / `ORDER BY name`（输出列）/ `WHERE id IN (1,3) ORDER BY id`（字面量）/ 无子查询 `ORDER BY id`

机制：SemiJoin/AntiJoin 节点的 `output_columns` 直接按 SELECT 投影列装配（`src/parser/planner/subquery.rs:63-64`、`:110-111` `build_output_columns_for_table(table_name, projection_columns)`），不像其他基座保持全宽行、由 Sort/Projection 负责裁剪；排序键计划期校验（`src/parser/planner/query.rs:2703-2730`）以 `get_plan_output_columns`（SemiJoin 臂 = 投影窄集）为可解析集 → 非输出列被拒。字面量 IN 走谓词下推、行保持全宽故不受影响。

预期：排序键语义与无子查询基座一致（非输出列按输入行排序）。实际：SemiJoin 形状特殊化导致同语义查询被点名拒绝。

## 影响

带 IN 子查询 + 按隐藏键排序的常见形态不可用；与既有排序语义不一致。评测实测命中。

## 事件记录

- 2026-10-12：外部评测实测；Explorer 五形态对照复现并定位装配点与校验点。检测：计划期报错。恢复：把排序列加进 SELECT 输出。

## 处置

closed（2026-10-14）——change `2026-10-13-e2e-verification-and-defect-closure` Iteration 003 T5 修复归档。采用原方向候选一：SemiJoin/AntiJoin 基座全宽承载——`build_full_output_columns_for_table` 全列装配 + 节点 `projection` 裁剪位（空 = 恒等）+ `build_output_row` 左行直通 + `sort_projection` 增 SemiJoin/AntiJoin 臂 + `get_plan_output_columns`/`plan_output_names` 两表头消费面对齐；ORDER BY 非输出列可用，字面量 IN/EXISTS/相关/AntiJoin 零回归；规格合并 `join-output-shape-resolution` ADDED（SemiJoin-AntiJoin 基座全宽承载与排序键）。

## 证据

`.agents/analysis/2026-10-12-external-eval-defect-verification.md`（P1-5 段）；复现现场 `/tmp/rt-verify/p1/d.db`。
