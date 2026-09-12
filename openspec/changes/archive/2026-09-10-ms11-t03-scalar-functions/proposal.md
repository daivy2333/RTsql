# MS11-T03: 标量函数库第一批（string 6 + math 4）

## Why

tasks MS11-T03：MS11-T01 表达式层落地后，agent 日常过滤和派生列的剩余缺口是标量函数——`WHERE upper(name) = 'X'`、`SELECT length(desc) FROM t` 这类形态当前全部落在 `Expr::Function` 臂的 `PlanError::UnsupportedExpression`（`src/parser/planner/expression.rs:272-294` 只放行 COALESCE）。R18 主题 7 结论（agent 是写 SQL 的）把函数与表达式四件套、事务语句同列为"agent 写 SQL 的日常件"；表达式求值内核、投影路由、表头派生在 MS11-T01/T02 均已就绪，本 change 只补函数分派与语义实现，实现面小、同域验收。

用户决策（2026-09-10，集中裁定 4 项语义 + 2 项默认假设）：

1. **floor/ceil 返回 Float**——SQLite REAL 先例，`floor(3.7)` 输出 `3.0`；与 sqlite_compare 基准一致。
2. **trim 仅剥空格**——SQL 标准 / SQLite TRIM 语义，TAB、换行保留。
3. **string 函数类型严格**——非字符串入参报 `ValueError::TypeMismatch`（对齐 LIKE 先例，`src/executor/predicate.rs:231-237`）；CAST 是唯一显式转换通道，不引入隐式转换。
4. **substr/round 边缘参数对齐 SQLite**——`substr('abc',0,2)='a'`、`substr('abc',2,-1)='a'`、`substr('abc',-2)='bc'`、`round(123.4,-1)=120.0`；BDD 按此写场景。
5. **abs 保持入参类型**（默认假设）——`abs(-5)=5`（Int）、`abs(-5.5)=5.5`（Float），SQLite 同型返回；非数值入参 TypeMismatch。
6. **round 的 digits 参数接受 Int 或 Float**（默认假设）——Float digits 按向零截断转 Int（SQLite 语义）。

## What Changes

- 新 capability spec `sql-scalar-functions`（6 Requirement）：
  - R1 函数注册与分派机制：名称大小写不敏感；plan 期 arity 校验；AST 拒绝面（OVER 窗口、DISTINCT、FILTER、命名参数、通配符参数、零参调用）显式点名拒绝；未知名维持既有 `Unsupported expression type` 文案
  - R2 string 函数六件：upper/lower/length/substr/replace/trim 语义 + 严格类型 + SQLite 边缘 + 默认表头与 AS 别名
  - R3 math 函数四件：abs（同型）/round（half-away-from-zero，1-2 参）/floor/ceil（返回 Float）
  - R4 NULL 语义与嵌套：任一参数 NULL → NULL（参数求值错误仍先传播）；参数可为任意值表达式（嵌套函数、COALESCE、CAST、负数字面量）；行级求值与三值折叠既有语义组合
  - R5 调用面与边界：WHERE 双路径（下推 / Filter / OR 组合）结果正确；聚合混用、HAVING 中标量函数保持既有拒绝；ORDER BY 引用表达式项别名静默保持输入序（既有语义文档化锁定）
  - R6 既有语义零回归
- 实现：planner `Expr::Function` 臂从"仅 COALESCE"扩展为查函数元数据表（名称/arity/AST 拒绝面校验）+ 构造统一 `FunctionExpression`；新模块 `src/executor/function.rs` 承载分派、求值与单测

不改变：聚合五函数路径（SELECT 列表先于表达式项检测）、COALESCE 既有臂、pipeline/CLI/渲染层、plan cache 键（SQL 文本规范化，函数名大小写变体天然分键）。

## Scenario Sketch

见 delta spec `specs/sql-scalar-functions/spec.md` 各 Scenario（R1×4、R2×6、R3×3、R4×3、R5×5、R6×2，共 23 个）。关键形态：

- 前置 `CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)` + 种子行；触发 SELECT/WHERE 中的函数调用；观察行集、表头、错误消息或退出码。
- 失败边界：未知函数、OVER/DISTINCT/FILTER/命名参数、arity 错误（plan 期）、类型不符（执行期 TypeMismatch）、NULL 入参出 NULL、ORDER BY 表达式别名静默保持输入序。

## Out of Scope

- 窗口函数（OVER/PARTITION BY）——tasks.md MS11 Non-goals；本 change 显式拒绝
- UDF、聚合函数扩展、日期/时间类型与函数（MS13）
- no-FROM SELECT 中的函数常量折叠（`SELECT upper('abc')` 受 I035 约束）
- ORDER BY 表达式别名的排序能力（既有静默语义锁定，不修复；如需改进另立 improvement）
