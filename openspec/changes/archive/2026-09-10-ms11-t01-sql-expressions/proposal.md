# proposal: MS11-T01 SQL 表达式四件套与值表达式（WHERE/SELECT）

## Why

- 路线图 MS11-T01（`.claude/docs/tasks.md`，2026-09-06 用户批准的应用层轨道）。R18 主题 7 结论："agent 是写 SQL 的"——过滤与派生列是 agent 日常件；CLI/存储面已由 MS10 收口，SQL 表达式是应用层下一块缺口。
- 现状：`build_where`/`build_expression`（`src/parser/planner/expression.rs:97-238`）仅支持六个比较 + AND/OR + 字面量/列引用；IN/LIKE/BETWEEN/IS NULL/CASE/COALESCE/CAST（含否定形式）全部落 `UnsupportedExpression`。SELECT 侧投影是纯列索引裁剪（`apply_projection`），不存在表达式求值机制，`SELECT CASE ... FROM t` 不可达。
- 2026-09-10 用户批准以 MS11 为下一个 change 方向并指示"开始计划吧"。

## Scope Decisions（2026-09-10 决策轮默认，待 Gate 1 审计）

决策轮已向用户提交，未获即时回复；按推荐默认继续，全部记录于此。**2026-09-10 用户审计并批准计划（原话："批准"），含本节全部范围默认项与"默认假设"9 条。**

1. **范围 = 仅 MS11-T01**：T02 事务语句、T03 标量函数各自独立 change（路线图预期 MS11 共 3 change）。
2. **SELECT 派生列纳入**，作为独立 Iteration（路线图 Outcome 明确 "WHERE/SELECT 支持"，stable baseline 含"派生列全绿"）。
3. **NULL 语义 = 三值逻辑**：谓词求值内部 True/False/Unknown，既有形态可观察行为不变。
4. **仅 I040 并入**。I035（no-FROM SELECT）/ I034（裸 DataScan CLI 表头）保持独立 I 项——未获用户点名，不扩展（I034 可在 T03 投影工作中顺带）。

## What Changes

- **谓词表达式**：`[NOT] IN (值列表)`、`[NOT] BETWEEN low AND high`、`[NOT] LIKE 模式`、`IS [NOT] NULL`、`NOT <谓词>` 进入 WHERE（非聚合查询）。
- **三值求值内核**：`Predicate` 求值内部引入 True/False/Unknown；比较与新算子遇 NULL 产生 Unknown，AND/OR 按 SQL 三值表组合，行选择将 Unknown 折叠为不匹配。既有比较/AND/OR 的行选择结果不变。
- **值表达式**：searched/simple CASE、COALESCE、CAST 实现为 `Expression` 实现，既作谓词操作数又作 SELECT 投影项。
- **SELECT 派生列**：非聚合查询的 SELECT 列表接受本 spec 值表达式项，逐行求值输出；支持 AS 别名，无别名用表达式 Display 文本作列名；与既有列裁剪投影、CLI 四格式渲染兼容。
- **下推协同**：新谓词与 Filter 共享求值器，满足既有门槛（无 OR、非 PK 等值形态）时装入 DataScan；`contains_or`/`has_pk_equality` 遍历扩展覆盖新变体（防 CASE 内 OR 被误下推）。
- **I040**：`extract_insert_values` 接受 `UnaryOp::Minus` 折叠负数字面量，INSERT VALUES / restore / import 负数数据可达。
- 新增 capability spec：`sql-expression-evaluation`。

## 默认假设（additive 语义，Gate 1 一并确认）

1. LIKE 仅作用于 String 值；任一操作数非 String（含 NULL 走三值）→ 执行期类型错误。`%` 任意串、`_` 单字符；`ESCAPE` 子句显式拒绝。
2. CAST 目标 Int/Float/String/Bool：数值↔字符串按值解析/格式化；Float→Int 截断向零（SQLite 同款，文档化）；非法转换（`'abc' AS INT`）执行期报错；CAST NULL → NULL。TRY_CAST 明确拒绝。
3. CASE 支持 searched（`CASE WHEN cond THEN r ... [ELSE e] END`）与 simple（`CASE operand WHEN v THEN r ...`）两形态；缺省 ELSE → NULL；simple CASE 的 operand=NULL 不匹配任何 WHEN（三值语义）。
4. COALESCE ≥1 参数，返回首个非 NULL，全 NULL → NULL。
5. 无别名派生列列名 = sqlparser Display 文本（如 `CASE WHEN ... END`、`x > 5` 的原文形态）；有别名用别名。
6. ORDER BY 不支持引用派生列/别名（保持 `extract_column_name` 现状报错）。
7. IN 常量列表不新增 IndexScan 路由：PK 列 IN 走通用 DataScan/Filter 路径（天然规避 I036 的不可达形态）。
8. 谓词仅 WHERE 位置（非聚合查询）；HAVING 中新表达式不支持（现状报错不变）。
9. 既有全量测试零修改通过（704 pass / 2 ignored 基线）；新错误文案 additive，不改既有文案；PhysicalPlan 节点集合不变（M01）。

## Non-goals

- MS11-T02（事务语句）、MS11-T03（标量函数与注册机制）——独立 change。
- I035 no-FROM SELECT、I034 CLI 表头裁剪——保持独立 I 项。
- 算术运算（`+ - * /`）、ILIKE/RLIKE/SIMILAR TO、IS [NOT] UNKNOWN、窗口函数。
- IN 子查询 / EXISTS（既有 `try_build_where_subquery` 路径不变）。
- IN 常量列表的索引路由优化；新表达式性能基准（功能 change，不适用 MS08 纪律）。

## Impact

- **specs**: 新增 `sql-expression-evaluation`（约 6 Requirement）。
- **code**: `src/executor/predicate.rs`（新谓词/表达式实现 + 三值内核）、`src/parser/planner/expression.rs`（转换臂）、`query.rs`（下推判定遍历、投影解析、`get_plan_output_columns`）、投影消费执行器（既有 `with_projection` 六处，Iter 001）、`ddl_dml.rs`（I040）。
- **tests**: `predicate_test.rs`（单测）、`planner_test.rs`（plan 形态）、`pushdown_test.rs`（下推等价）、新增表达式 E2E 套件、投影/派生列套件（Iter 001）、负数字面量用例。
