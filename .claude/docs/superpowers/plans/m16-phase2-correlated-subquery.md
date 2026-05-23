# M16-Phase2: Correlated Subquery Support — Implementation Plan

> 日期: 2026-05-23
> 状态: Plan Finalized (Momus review pending)
> 依赖: M16-Phase1 (completed)

## Architecture Overview

### Design Decision 1: ParameterExpression + Trait Methods

A new `ParameterExpression` type in `src/executor/predicate.rs` stores a `Mutex<Value>` for interior mutability. Outer references become `ParameterExpression` nodes in the predicate tree instead of `ColumnExpression`. Value injection happens via trait methods `Predicate::inject_parameters()` and `Expression::set_parameter_value()`. Since `PhysicalPlan` derives `Clone` (shallow — `Arc` refs shared), injecting into a cloned plan updates the shared `ParameterExpression` arc, which is safe because we never execute two clones concurrently (Volcano model: inject → execute → discard, per row).

### Design Decision 2: SemiJoin/AntiJoin Dual-Path Plan Storage

Each executor stores both `right: Box<dyn Executor + Send>` (independent fast path) and `right_plan: Option<PhysicalPlan> + database: Arc<Database>` (correlated path). When `correlated_params.is_empty()`, use the pre-built executor. When non-empty, clone `right_plan`, inject values per left row, build executor, execute, discard.

### Design Decision 3: Planner `inner_table_names` Context

`PlanBuilder` gets a field `inner_table_names: Option<Vec<String>>`. Before building a subquery body, set it to the subquery's FROM table names. In `build_expression`, when encountering `CompoundIdentifier(table, column)` where `table` is NOT in `inner_table_names`, create `ParameterExpression` instead of `ColumnExpression`.

### Design Decision 4: Multi-Level Correlated Error (Plan Time)

In `collect_outer_column_refs`, when encountering nested subquery expressions while scanning a subquery that already has outer references, return `PlanError::CorrelatedParamError("Multi-level correlated subqueries are not supported")` via `has_outer_refs_outside()` helper.

---

## Trait Additions

### Expression trait additions (`predicate.rs`)

```rust
pub trait Expression: Send + Sync + Debug {
    fn evaluate(&self, row: &[Value]) -> Result<Value, Box<dyn std::error::Error + Send + Sync>>;
    fn set_parameter_value(&self, _param_name: &str, _value: &Value) -> bool { false }
}
```

### Predicate trait additions (`predicate.rs`)

```rust
pub trait Predicate: Send + Sync + Debug {
    fn evaluate(&self, row: &[Value]) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>;
    fn inject_parameters(&self, _params: &[(String, Value)]) {}
}
```

### ParameterExpression

```rust
pub struct ParameterExpression {
    pub param_name: String,
    value: Mutex<Value>,
}
```

---

## Task Dependency Graph

```
T1 ──────┬───────────────┐
         ↓               ↓
         T2              T3
         │         ┌─────┼─────┐
         │         ↓     ↓     ↓
         │         T4    T5    T6
         │         └─────┼─────┘
         │               ↓
         └──────────────→T7
                         ↓
                         T8
```

## Execution Waves

| Wave | Tasks | Parallel? |
|------|-------|-----------|
| 1 | T1 (CorrelatedParam + ParameterExpression) | Sequential (foundational) |
| 2 | T2 (Planner) + T3 (inject function) | PARALLEL |
| 3 | T4 (SemiJoin) + T5 (AntiJoin) + T6 (SubqueryEval) | PARALLEL |
| 4 | T7 (Pipeline wiring) | Sequential (depends on T4+T5+T6) |
| 5 | T8 (Tests) | Sequential (depends on T2+T7) |

**Critical path**: T1 → T3 → T4 → T7 → T8 (5 steps)

---

## Task 1: CorrelatedParam Refactor + ParameterExpression + Trait Methods

**Depends on**: Nothing  
**Blocks**: T2, T3, T4, T5, T6  
**Files**: `src/executor/plan.rs`, `src/executor/predicate.rs`, `src/executor/mod.rs`

Changes:
- Change `CorrelatedParam.inner_column_index: usize` → `param_name: String`
- Update `CorrelatedParam::new()` signature
- Add `set_parameter_value()` default method to `Expression` trait
- Add `inject_parameters()` default method to `Predicate` trait
- Implement `inject_parameters` on `LogicalPredicate` (recurse) and `ComparisonPredicate` (delegate to expressions)
- Add `ParameterExpression` with `Mutex<Value>`, impl `Expression` with `evaluate` + `set_parameter_value`
- Unit test: `test_parameter_expression`
- Update `mod.rs` exports

**Success criteria**:
- `cargo check` — no errors
- `cargo test` — `test_parameter_expression` passes
- `CorrelatedParam` stores `param_name: String`

---

## Task 2: Planner — Outer Reference Detection + Multi-Level Error

**Depends on**: T1  
**Blocks**: T8  
**Files**: `src/parser/planner.rs`, `src/parser/error.rs`

Changes:
- Add `inner_table_names: Option<Vec<String>>` field to `PlanBuilder`
- Update `PlanBuilder::new()` to initialize it
- Modify `build_expression` `CompoundIdentifier` branch: check if table is outer → create `ParameterExpression`
- Update `collect_outer_column_refs` to use `param_name` in `CorrelatedParam::new()`
- Add `has_outer_refs_outside()` helper method
- Add multi-level detection in `InSubquery`/`Exists` branches
- Set/clear `inner_table_names` in `try_build_where_subquery` and scalar subquery `build_query`
- Handle error paths with explicit `self.inner_table_names = None`

**Success criteria**:
- `cargo check` — no errors
- Correlated query produces `CorrelatedParam { param_name: "table.column" }`
- Multi-level correlated returns `PlanError::CorrelatedParamError("Multi-level correlated subqueries are not supported")`

---

## Task 3: inject_correlated_values Function

**Depends on**: T1  
**Blocks**: T4, T5, T6  
**Files**: `src/executor/correlated.rs` (NEW), `src/executor/mod.rs`

Changes:
- New module with `inject_correlated_values(plan, param_values)`
- Recursively walks PhysicalPlan tree
- Calls `predicate.inject_parameters()` on FilterNode and HavingNode
- Recurse into child plans (SemiJoin/AntiJoin/SubqueryEval/Sort/Limit/Aggregate/Join/DerivedScan)
- Unit test: `test_inject_into_filter` using ParameterExpression in ComparisonPredicate

**Success criteria**:
- `cargo test` — `test_inject_into_filter` passes
- Injection into nested `LogicalPredicate` updates both branches

---

## Task 4: SemiJoinExecutor Correlated Execution

**Depends on**: T1, T3  
**Blocks**: T7  
**Files**: `src/executor/semi_join.rs`

Changes:
- Add fields: `right_plan: Option<PhysicalPlan>`, `database: Option<Arc<Database>>`
- Rename `_correlated_params` → `correlated_params`
- Update `new()` signature
- Add `extract_param_values()` helper
- Modify `BuildRight` phase: skip right materialization when correlated (placeholder)
- Modify `ScanLeft` phase: per-row clone plan, inject values, rebuild right executor, materialize, probe
- Add imports for `inject_correlated_values`, `create_executor_from_plan`, `Arc`

**Success criteria**:
- 14 existing independent subquery tests pass
- Correlated semi-join with `emp.dept IN (SELECT dept.id FROM dept WHERE dept.id = emp.dept)` returns correct rows

---

## Task 5: AntiJoinExecutor Correlated Execution

**Depends on**: T1, T3  
**Blocks**: T7  
**Files**: `src/executor/anti_join.rs`

Changes:
- Identical pattern to T4: same new fields, same dual-path BuildRight/ScanLeft
- Anti-join probe logic differs (output when NOT matched) but correlation injection is identical

**Success criteria**:
- Existing NOT IN/NOT EXISTS tests pass
- Correlated NOT IN returns correct non-matching rows

---

## Task 6: SubqueryEvalExecutor Correlated Execution

**Depends on**: T1, T3  
**Blocks**: T7  
**Files**: `src/executor/subquery_eval.rs`

Changes:
- Add field: `outer_column_indices: HashMap<String, usize>`
- Rename `_correlated_params` → `correlated_params`
- Update `new()` to accept `outer_column_indices`
- Add `extract_param_values()` helper
- Modify `next()`: when correlated, clone plan, inject values, create executor, execute fresh per row (skip cache)

**Success criteria**:
- Existing scalar subquery tests pass
- `SELECT dept.name, (SELECT AVG(emp.salary) FROM emp WHERE emp.dept = dept.id) AS avg_sal FROM dept` returns per-department averages

---

## Task 7: Pipeline Wiring

**Depends on**: T4, T5, T6  
**Blocks**: T8  
**Files**: `src/pipeline.rs`

Changes:
- SemiJoin case: clone right plan before moving, pass `right_plan` + `Arc<Database>` to constructor
- AntiJoin case: same pattern
- SubqueryEval case: compute `outer_column_indices` from input plan, pass to constructor

**Success criteria**:
- `cargo build` succeeds
- All 14 existing subquery tests pass (zero regression)

---

## Task 8: Integration Tests + Placeholder Fixes

**Depends on**: T2, T7  
**Files**: `tests/subquery_test.rs`

Changes:
- Fix `test_correlated_where_in_basic`: replace invalid query with `emp.dept IN (SELECT dept.id FROM dept WHERE dept.id = emp.dept)`
- Fix `test_correlated_scalar_subquery`: replace with valid per-dept averages assertions
- Add `test_correlated_exists`: EXISTS with correlated condition
- Add `test_correlated_not_exists`: NOT EXISTS with correlated condition
- Add `test_correlated_not_in`: NOT IN with correlated condition
- Add `test_correlated_null_outer_value`: NULL outer column → SQL 3-value logic
- Add `test_correlated_empty_right`: empty subquery result → no matches
- Add `test_multi_level_correlated_error`: multi-level returns clear PlanError message

**Success criteria**:
- `cargo test --test subquery_test` — 20+ tests pass
- `cargo test` — all ~121+ tests pass
- Multi-level correlated returns clear error
- NULL outer values handled correctly
- Empty subquery results handled correctly

---

## Commit Strategy

### Commit 1: Foundation
```
feat(M16): add correlated subquery foundation - ParameterExpression, planner detection, injection
```
T1 + T2 + T3 — independently compilable

### Commit 2: Execution + Tests
```
feat(M16-P2): implement correlated subquery execution across all executors
```
T4 + T5 + T6 + T7 + T8 — end-to-end working

## Files Modified

| File | Tasks | Action |
|------|-------|--------|
| `src/executor/plan.rs` | T1 | Modify CorrelatedParam |
| `src/executor/predicate.rs` | T1 | Add ParameterExpression + trait methods |
| `src/executor/correlated.rs` | T3 | **NEW** inject function |
| `src/executor/mod.rs` | T1, T3 | Add exports |
| `src/parser/planner.rs` | T2 | Add inner_table_names, outer ref detection, multi-level error |
| `src/parser/error.rs` | T2 | (existing CorrelatedParamError variant) |
| `src/executor/semi_join.rs` | T4 | Dual-path execution |
| `src/executor/anti_join.rs` | T5 | Dual-path execution |
| `src/pipeline.rs` | T7 | Pass new constructor args |
| `tests/subquery_test.rs` | T8 | Fix + add tests |

## Risk Mitigation

| Risk | Mitigation |
|------|------------|
| Arc sharing across clones | Safe: inject-before-execute, never concurrent |
| `extract_column_indices` on unusual shapes | `unwrap_or` defaults for missing columns |
| Outer column not in row indices | Return `Value::Null` |
| Async recursion in tree walk | `inject_correlated_values` is sync, async only in executor rebuild |
