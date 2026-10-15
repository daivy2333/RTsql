# Design: AntiJoin 关联参数绑定期加固

## 缺陷与根因（Plan Review Finding 1，代码级 Confirmed 2026-10-15）

`AntiJoinExecutor::extract_param_values`（`src/executor/anti_join.rs:151-161`）：

```rust
self.correlated_params
    .iter()
    .filter_map(|cp| {
        self.left_column_indices
            .get(&cp.outer_column.to_lowercase())
            .map(|&idx| (cp.param_name.clone(), left_row[idx].clone()))
    })
    .collect()
```

`outer_column` 未命中 `left_column_indices` 时该参数被**整体省略**（非 `unwrap_or(0)` 首列回落——SemiJoin/ScalarSubqueryFilter/SubqueryEval 三处的形态）。下游双重后果：

1. `inject_correlated_values(&plan, &param_values)` 收不到该参数 → 右计划中对应 `ParameterExpression` 未注入、求值 NULL（`correlated.rs` Projection 臂注记明示「未注入时 ParameterExpression 求值为 Null，静默错」）；
2. `correlated_rows_cache` 以 `Vec<(String, Value)>` 为键——缺项键使「缺参列值不同的外层行」共享同一缓存项（键短项化），跨行错结果复用。

可达性：前 change T1（`extract_subquery_table_names` 声明限定符化）后，正常 SQL 经计划期分类产生的相关参数列按构造必在外层行中，该路径不可经 SQL 触达——与前三执行器加固前同等，属「未来漏分类显式报错」的防御纵深。

## 修复设计与选择理由

### T1：filter_map → Result 点名报错

同构于前 change T2 三处：签名改 `Result<Vec<(String, Value)>>`，`filter_map` 改 `map` + `ok_or_else`（`StorageError::ExecutionError`，消息 `correlated parameter column not found in outer row: {cp.outer_column}`），调用点 `anti_join.rs:209` `?` 上浆。命中臂 `(cp.param_name.clone(), left_row[idx].clone())` 逐字节保持（含直接索引语义，见假设 A3）。

- **理由**：与 R2 既有 requirement（前 change 落库）同模，错误类型/文案/SQLSTATE（58000 兜底）全一致，四执行器绑定期错误面收敛为同一形态；单点改动，调用点唯一。
- **拒绝的替代**：(a) 保持丢弃并记日志——无日志通道，静默性不变；(b) 统一四执行器命中路径为 `.get(idx).cloned().unwrap_or(Value::Null)`——改变 AntiJoin 命中路径现状（行短于索引表时直接索引 panic vs 三处的 NULL 回退），超出 Finding 1 范围且引入新行为差异。
- **调用点位置注意**：`extract_param_values` 调用处于 `if let (Some(right_plan), Some(database))` 块内（`anti_join.rs:206-267`）——`right_plan`/`database` 为 None 时相关分支整体跳过（右表维持 BuildRight 的空态），不经过绑定期；先行测试构造必须给齐两字段，否则测试落空（不经过被测路径）。`conditions` 取空（NOT EXISTS 形态）即可使断言聚焦绑定期错误，不必构造 NOT IN 键面。

### 同类面审计（本 change 范围划定）

全库 `extract_param_values` 共四处，本 change 后全部 `Result` 化完成收口。`inject_correlated_values` / `Expression::inject_parameters` / `set_parameter_value` 的「未注入参数求值 NULL」面为更深层同类点（需改表达式层签名，连锁全部谓词/表达式类型），前 change T2 已审计为非目标，维持。

## 影响面与兼容性

- 错误面：新增一条不可经正常 SQL 触达的报错路径；可达形态（NOT EXISTS / NOT IN 独立与相关）行为零变化。
- 缓存：`correlated_rows_cache` 键结构不变（完整 param 集合仍为键）；错误不缓存（前 change 语义沿用）。
- 退出码 / SQLSTATE / PG 面：零变化（58000 兜底）。
- 性能：每左行一次 HashMap 查找失败臂改 error 构造，命中臂指令路径不变。

## 实现顺序与依赖

T1（唯一任务：加固 + 先行单测）→ 全量 `cargo test` 一次（Iteration Gate）。无任务间依赖。
