# MS13 分析函数 — Tasks

> 全局任务编号 T1–T13；Iteration 规划见文末 Iteration Plan。状态：`pending` / `in-progress` / `done` / `skipped`。

## T1: datetime 日历数学模块（RED 先行）

- 状态: done
- 新增 `src/executor/datetime.rs`：civil 算法（days_from_civil/civil_from_days）、parse_date/format_date、parse_timestamp/format_timestamp、字段抽取（year/month/day/hour/minute/second）、截断（trunc to unit）、区间算术（add months 锚定截月末 + 微秒加减）、datediff、IntervalParts 解析（`'N unit'` 与 `N UNIT` 两形态）。
- 单元测试 ~20（闰年 400 年规则/月末/负区间/往返/边界年 0001-9999/微秒 6 位）。
- 验证: `cargo test -p rtsql datetime`（或模块过滤）全绿。

## T2: Value/ValueRef/tuple/catalog 类型底座

- 状态: done
- `Value::Date/Timestamp` + `ValueRef` 两变体；equals/gt/lt/ge/le 同型臂、Hash、Display（D4 格式）、`to_key→None`、`as_value_ref`/`to_value`、`lt_agg` 两臂。
- `tuple.rs`：TAG 0x06/0x07 + compute/serialize/deserialize ×2 + 单测往返/截断/损坏拒绝。
- 双 `ColumnType` 枚举加变体；`catalog.rs` COL_TAG 0x05/0x06 持久化往返单测。
- `sort.rs compare_values` 两显式臂。
- 验证: 模块单测 + `tests/datetime_type_test.rs` 序列化间接见证起步。

## T3: DDL 映射与 schema/dump 类型面

- 状态: done
- `convert_data_type`：Date/Datetime/naive Timestamp 映射；Tz 变体/Time/Interval 显式拒绝（点名）。
- `to_schema_column` 两臂；`create_table_sql` 渲染 DATE/TIMESTAMP（schema 命令同源）。
- 验证: e2e——建表落列/`schema` 输出 DATE/TIMESTAMP/INTERVAL 列拒绝（datetime_type_test 前段）。

## T4: 类型字面量 + 写入强制解析

- 状态: done
- `build_expression`/`build_where`/`extract_insert_values`/UPDATE SET 四点 TypedString 臂（D5）；`ast.rs extract_columns` 放行 TypedString + BinaryOp。
- `StorageError::InvalidDateTime`；InsertExecutor 逐列强制解析与类型收口（String→parse / 拒绝非日期族非空值 / Null 放行）；UpdateExecutor SET 同规则。
- 验证: e2e——类型字面量/裸字符串强制/非法拒绝零副作用/UPDATE SET 三形态/恢复两态（close→reopen 等值）。

## T5: 比较/排序/PK 路由/CAST/渲染/导入导出收口

- 状态: done
- CAST 矩阵（D8）+ CastType 映射臂 + evaluate_ref Copy 直回。
- `value_to_json` 两臂；dump `sql_literal` 类型化字面量（列类型在循环内可用）；`csv_value` 两臂（空→NULL/非空透传）。
- Date PK 路由回归（MS16 回退路径 e2e）；dump→restore→dump 恒等（含日期列）。
- 验证: `tests/datetime_type_test.rs` 全套（~25 用例成型）+ 既有 keyless_eq/gc/version_chain 相关零修改。

## T6: 日期函数族 + I043/I044

- 状态: done
- REGISTRY +12 项（NOW/DATE/YEAR/MONTH/DAY/HOUR/MINUTE/SECOND/DATE_TRUNC/DATEDIFF）；eval_scalar 臂（datetime.rs helper；严格类型/NULL 短路沿用 D3 求值序）；date_trunc 单位校验。
- ABS `checked_abs` 溢出错误；ROUND digits ±308 饱和卫兵（D15）。
- `tests/datetime_function_test.rs` ~20 + `scalar_function_test.rs` 增 I043 4 用例/I044 3 用例。
- 验证: 两套件全绿 + 既有 scalar_function_test 零修改。

## T7: INTERVAL 表达式算术

- 状态: done
- `Expr::Interval` 双入口接线（两形态解析/多字段拒绝）；`IntervalArithExpression` 节点（datetime.rs）；BinaryOp 构建点识别（Interval 腿 + Date/Timestamp 对侧）；独立投影项拒绝。
- datetime_function_test 增 INTERVAL 用例（日/时/月末锚定/负区间/拒绝面）。
- 验证: 套件全绿。

## T8: GROUP BY 表达式/别名/位置 + 混合投影

- 状态: done
- `AggregateNode.group_key_exprs` 加性字段；planner 解析序（列名→别名→文本→位置）+ 不匹配显式错误；`extract_group_key` 求值化；混合投影解锁检查；条件 Projection 包装（纯列名序形态零触碰，D12）。
- `tests/group_by_expr_test.rs` ~10（date_trunc 三形态等价/交错序/NULL 键归并/错误面/既有零回归）。
- 验证: 套件全绿 + 既有聚合/GROUP BY 测试零修改。

## T9: no-FROM SELECT 虚拟单行

- 状态: done
- `PhysicalPlan::SingleRow` + `SingleRowExecutor` + 五点接线（两构造分发/get_plan_output_columns/extract_column_indices/空行求值）；planner no-FROM 分支（表达式项 Projection/六类子句点名拒绝）。
- `tests/no_from_select_test.rs` ~8（单行常量/函数/CASE/WITH-FORM 算术解锁/拒绝面/列引用错误）。
- 验证: 套件全绿 + lib 两路径与 CLI 面一致性抽检。

## T10: stats 命令

- 状态: done
- lifecycle.rs `stats`：拉取 + 行数/null率/distinct/min/max/p50/p90/p99（最近邻秩，D14）；render 四态；错误面 exit 3。
- cli_test 增 stats 用例（数值列/日期列/空表/表不存在/格式四态抽检）。

## T11: sample 命令

- 状态: done
- reservoir sampling（rand 0.8）；N 默认 10、0/非整数 exit 2；行集 render。
- cli_test 增用例（行数/列形状/N 边界）。

## T12: profile 命令

- 状态: done
- 每列 type/min/max/top-k（String 列，k 默认 5 上限 20，并列字典序稳定，D14）。
- cli_test 增用例（混合列/并列稳定/数值列无 top-k）。

## T13: 收尾全量验证

- 状态: done
- 全量 `cargo test --no-fail-fast`（936 + 新增全绿零修改）；clippy `--all-targets -D warnings` 0；fmt 0；`openspec validate` PASS。
- change 结构自检（tasks 状态/specs/design 与实现一致）。

---

## Iteration Plan

### Iteration 000: datetime 类型底座（T01 主体）

- Tasks: T1, T2, T3, T4, T5
- Depends on: None
- Stable baseline: DATE/TIMESTAMP 全链可用——建表落列、类型化/裸字面量写入、比较/排序/PK 路由、CAST、四格式渲染、dump/restore/CSV、恢复两态一致；后续 Iteration 可在其上叠加函数与分桶。
- Verification boundary: `tests/datetime_type_test.rs` 全绿 + 既有全量零修改（T5 末跑一次全量）。
- Diagnostic boundary: `src/executor/{value,value_ref,datetime}.rs`、`src/storage/page_format/tuple.rs`、`src/storage/catalog.rs`、`src/parser/planner/{ddl_dml,expression}.rs`、`src/executor/{insert,update,sort}.rs`、`src/cli/lifecycle.rs`、`src/pipeline.rs`（value_to_json）。
- Non-goals: 日期函数/INTERVAL/GROUP BY 扩展/no-FROM/CLI 分析命令（后续 Iteration）。
- 平衡审计: 5 任务同一故障域（类型底座穿透全链），以「类型可存可查可比可恢复」为单一可验证成果；T1 纯函数模块先行（RED 可独立观察），T2–T5 按依赖链推进。工作量偏重但变更面高度同构（类型分派臂×N），拆开反而破坏「全链一致」验收边界——保留单 Iteration。

### Iteration 001: 日期函数与分桶（T02 引擎侧）

- Tasks: T6, T7, T8
- Depends on: Iteration 000（类型底座）
- Stable baseline: 日期函数族 + INTERVAL 算术 + `GROUP BY date_trunc(...)` 分桶可用；I043/I044 收口；既有标量函数/聚合面零回归。
- Verification boundary: `tests/datetime_function_test.rs` + `tests/group_by_expr_test.rs` + `tests/scalar_function_test.rs`（增 I043/I044 后）全绿 + 既有全量零修改。
- Diagnostic boundary: `src/executor/{function,datetime}.rs`、`src/parser/planner/expression.rs`（Interval 臂）、`src/parser/planner/query.rs`（GROUP BY 解析/混合投影/条件包装）、`src/executor/aggregate.rs`。
- Non-goals: CLI 分析命令、no-FROM（后续）；strftime/to_date 族（Out of Scope）。
- 平衡审计: 三任务同属「分析 SQL 引擎侧能力」成果（函数→区间→分桶为依赖递进，共用 datetime.rs 与表达式层），单一验证边界（三测试套件）；与 000 以类型/函数分层切分，诊断域不重叠。

### Iteration 002: no-FROM 与分析薄命令（T03 + T02 CLI 侧）

- Tasks: T9, T10, T11, T12, T13
- Depends on: Iteration 001（stats 依赖类型/函数无直接耦合但全量收口依赖前序全绿；T13 为全 change 收尾）
- Stable baseline: `SELECT 1+1` 可达 + WITH-FORM 算术解锁；stats/sample/profile 三命令四态输出；全 change 收口（全量/clippy/fmt/validate）。
- Verification boundary: `tests/no_from_select_test.rs` + cli_test 新增组全绿 + T13 全量验证记录。
- Diagnostic boundary: `src/parser/planner/query.rs`（no-FROM 分支）、`src/executor/`（SingleRow/plan 接线五点）、`src/pipeline.rs`（构造分发）、`src/cli/{mod,lifecycle}.rs`。
- Non-goals: REPL、分发（MS14）、加密（MS12）。
- 平衡审计: no-FROM（引擎小项）与三 CLI 命令（纯拉取计算）互不依赖但同属「便利面」层，合并为终 Iteration + 收尾任务 T13 形成 change 级完整验证闭环；单独拆出会产生无独立验收价值的过碎 Iteration。

## Requirements Traceability Matrix

| Requirement (delta spec) | Scenario 代表 | Design | Task | Iter | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| datetime-type-system R1 值与存储 | 序列化往返/截断拒绝 | D1,D2 | T1,T2 | 000 | `value.rs`/`value_ref.rs`/`tuple.rs` | datetime_type_test + tuple 单测 | None | Covered |
| datetime-type-system R2 DDL 映射 | 建表落列/INTERVAL 拒绝 | D3 | T3 | 000 | `ddl_dml.rs convert_data_type`/`plan.rs`/`lifecycle.rs` | datetime_type_test DDL 段 | None | Covered |
| datetime-type-system R3 字面量与写入强制 | 类型字面量/裸串强制/非法拒绝/恢复两态 | D4,D5,D6 | T4 | 000 | `expression.rs`×2/`ddl_dml.rs`×2/`ast.rs`/`insert.rs`/`update.rs` | datetime_type_test 写入段 | None | Covered |
| datetime-type-system R4 比较与键控 | 时间序过滤/跨类型拒绝/PK 回退 | D7 | T2,T5 | 000 | `value.rs`/`sort.rs`/`query.rs`(MS16 门) | datetime_type_test 比较段 + keyless 路由用例 | None | Covered |
| datetime-type-system R5 CAST | 双向/非法与跨族拒绝 | D8 | T5 | 000 | `predicate.rs CastExpression`/`expression.rs` | datetime_type_test CAST 段 | None | Covered |
| datetime-type-system R6 渲染与导入导出 | dump 恒等/CSV 落类型 | D4,D9,D6 | T5 | 000 | `value_to_json`/`lifecycle.rs` | dump 恒等 + csv 用例 | None | Covered |
| datetime-type-system R7 零回归 | 既有全量零修改 | D16 | T5,T13 | 000,002 | — | 全量 `--no-fail-fast` | None | Covered |
| datetime-functions R1 函数族 | 抽取/截断/now/错误面/NULL | D10 | T6 | 001 | `function.rs`/`datetime.rs` | datetime_function_test | None | Covered |
| datetime-functions R2 INTERVAL | 日时算术/月末锚定/拒绝面 | D11 | T7 | 001 | `expression.rs`×2/`datetime.rs` | datetime_function_test INTERVAL 段 | None | Covered |
| datetime-functions R3 datediff | 日差/月差截断 | D10 | T6,T1 | 001 | `datetime.rs` | datetime_function_test | None | Covered |
| datetime-functions R4 零回归 | 既有函数面 | D15 | T6 | 001 | `function.rs` | scalar_function_test 零修改 | None | Covered |
| group-by-expression R1 别名/表达式/位置 | 三形态等价 | D12 | T8 | 001 | `query.rs`/`aggregate.rs`/`plan.rs` | group_by_expr_test | None | Covered |
| group-by-expression R2 匹配失败与既有保持 | 不匹配错误/NULL 归并/零回归 | D12 | T8 | 001 | 同上 | group_by_expr_test | None | Covered |
| no-from-select R1 单行可达 | 常量/函数/CASE | D13 | T9 | 002 | `query.rs`/SingleRow 五点 | no_from_select_test | None | Covered |
| no-from-select R2 拒绝面 | 通配/谓词聚合/列引用 | D13 | T9 | 002 | `query.rs` no-FROM 分支 | no_from_select_test 拒绝段 | None | Covered |
| no-from-select R3 零回归与算术解锁 | WITH-FORM id+1/全量 | D13,R4 | T9,T13 | 002 | `ast.rs`/`query.rs` | datetime_type_test（R3/S1 WITH-FORM，Iteration 000 套件）+ no_from_select_test + 全量 | None | Covered |
| cli-analytics R1 stats | 数值/日期/空表 | D14 | T10 | 002 | `lifecycle.rs stats` | cli_test stats 组 | None | Covered |
| cli-analytics R2 sample | 行数列形状/N 边界 | D14 | T11 | 002 | `lifecycle.rs sample` | cli_test sample 组 | None | Covered |
| cli-analytics R3 profile | 画像/并列稳定 | D14 | T12 | 002 | `lifecycle.rs profile` | cli_test profile 组 | None | Covered |
| cli-analytics R4 格式与错误面 | 四态/表不存在 | D14 | T10–T12 | 002 | 同上 + render 复用 | cli_test 错误/格式用例 | None | Covered |
| sql-scalar-functions R1 修改 | 大小写见证/零参 carve-out | D15 | T6 | 001 | `function.rs`(NOW 注册) | scalar_function_test I044 组 | None | Covered |
| sql-scalar-functions R3 修改 | abs 溢出/round 饱和 | D15 | T6 | 001 | `function.rs` ABS/ROUND 臂 | scalar_function_test I043 组 | None | Covered |
