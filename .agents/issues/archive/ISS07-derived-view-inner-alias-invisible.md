# ISS07 派生表与视图的内层别名/表达式项外层不可见（CTE 通道正常）

- Status: closed
- Filed: 2026-10-12
- Source: 外部评测（2026-10-12，P1-3）+ Explorer 核实（`.agents/analysis/2026-10-12-external-eval-defect-verification.md`，R32）
- Environment: Linux x86_64 (WSL2)、rtsql 0.2.0、HEAD `44d04d9` debug 构建

## 缺陷描述

FROM 派生表与视图的内层 SELECT 若含列别名或计算（表达式）列，外层引用这些列失败；裸列名形态正常：

- `SELECT v FROM (SELECT age AS v FROM people) t` → 42703 `Column not found: v`
- `SELECT tier FROM (SELECT CASE … END AS tier FROM people) t` → 58000 `Invalid aggregate argument: Expected column name`（纯 SELECT 无聚合无 GROUP BY 却收到聚合错误文案）
- `CREATE VIEW vw AS SELECT age AS v FROM people` 后 `SELECT v FROM vw` → 42703（**视图通道同样中招，超出评测报告范围**）
- 对照：内层两条查询独立执行均正常；`WITH x AS (SELECT age AS v FROM people) SELECT v FROM x` **CTE 通道正常**——与语料 `sql-views` R3「CTE 通道展开与内联派生表逐字节一致」矛盾。

机制：FROM 派生臂（`src/parser/planner/query.rs:409-447`）置 `building_subquery=true`，该上下文对内层 SELECT 表达式项/别名顶层 Projection 路由的抑制（MS11-T01 为 IN/EXISTS 标量子查询设计）一并生效；change `2026-10-04-external-review-gaps` T5 的 `building_from_derived` 只放宽了集合操作守卫、未解除该抑制。视图展开走同一通道。58000 文案来自 `expr_to_column_name` 对非列表达式抛 `InvalidAggregateArgument`（`src/parser/planner/expression.rs:875`）。

预期：文档声明的「FROM derived table」能力下，内层输出列（含别名与计算列）外层可引用，三通道一致。实际：两通道失败、错误文案误导。

## 影响

文档声明能力不可用；视图带别名/计算列定义即不可查询。评测实测命中。

## 事件记录

- 2026-10-12：外部评测实测派生表两形态失败；Explorer 复现并扩展发现视图通道同病、CTE 通道正常的三通道不一致。检测：查询报错。恢复：改写内层为裸列名（计算列无规避）。

## 处置

closed（2026-10-14）——change `2026-10-13-e2e-verification-and-defect-closure` Iteration 003 T4 修复归档。`building_from_derived` 放行扩至 DISTINCT 门 + 五分类门 + `alias_reroutable`，视图展开（`resolve_view_reference`）同型置位；派生表/视图内层别名与表达式列外层可见，IN/EXISTS/标量子查询抑制逐字节保持。处置要求中的语料-代码矛盾同步收敛：规格合并 `sql-views` MODIFIED ×1（视图三通道派生语义一致收口）；`tests/derived_scan_alias_test.rs` +5、`tests/view_test.rs` +4 见证。

## 证据

`.agents/analysis/2026-10-12-external-eval-defect-verification.md`（P1-3 段七形态矩阵与代码落点）；复现现场 `/tmp/rt-verify/p1/d.db`。
