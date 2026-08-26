# 任务清单：MS06-T02 PlanCache DashMap + SQL 规范化

> 关联里程碑：MS06-T02（MS06 稳定性与正确性收口 / PlanCache 改 DashMap + SQL 规范化 key + 替换 `std::sync::Mutex`）
> 关联 design：`design.md`
> 关联 proposal：`proposal.md`
> 关联 Iteration：仅一个 Iteration `000-initial`，6 个 task（T0 前置 + T1-T5）全部归属此 Iteration
>
> 修订记录：2026-08-25 Plan 审计后修订——新增 T0（基线 clippy 归零，用户批准并入）、修正 4.6 import 覆盖范围、V6 改为限定路径并记录工作区前置条件、RTM 增加 R5

## Iteration Plan

### Iteration 000: PlanCache 重写 + 基线归零

- Tasks: T0, T1, T2, T3, T4, T5
- Depends on: None
- Stable baseline: T0 后 `cargo clippy --all-targets -- -D warnings` 全绿；100 并发同 SELECT 全部 hit 且 <5s 完成；大小写/空白变体 100% hit；DML/DDL 行为不变；现有 487 tests pass
- Verification boundary: 5 项独立验证（T0 clippy 门禁 / plan_cache 单元测试 / pipeline_test 既有测试 / plan_cache_test 集成测试 / 100 并发场景）
- Diagnostic boundary: 11 个具体文件（design §6 表 L1-L7 七处 + `src/plan_cache.rs` / `src/database.rs` / `src/pipeline.rs` / `tests/executor_test.rs`）+ 1 个新增（`tests/plan_cache_test.rs`）
- Non-goals: LRU 精确淘汰、参数化查询缓存、cache 持久化、字符串字面量转义引号、T0 清单之外的清理重构

### 平衡审计

- T1-T5 共 5 个 task 紧密耦合（`Arc<PlanCache>` 类型变化必须同步改 5 个调用点）
- 拆分 T1 单独成 Iteration 不可行（脱离调用点无法编译）
- T0 是 V2 clippy 门禁的前置条件：基线 7 处既有错误中至少一处由 MS06-T01 提交引入，无法在本 change 外假设他人修复；经用户 2026-08-25 批准并入本 Iteration，全部为机械修复，与 T1-T5 无代码重叠
- 单 Iteration 6 task 是最小可工作单元
- 现有 487 tests pass 是充分 stable baseline

## 0. 前置：基线 clippy 归零（T0）

> 用户 2026-08-25 批准并入本 change：V2 门禁要求 clippy 全绿，而当前基线存在 7 处既有错误（位置与修复方案见 design §6 表 L1-L7）。T0 全部为机械修复，不改变任何运行时行为。

- [ ] 0.1 L1 `src/transaction/version_chain.rs:114`：删除 cfg(test) 内未使用的 `use crate::storage::PageId;`
- [ ] 0.2 L2 `src/executor/data_scan.rs`：删除 `DataScanExecutor.table_meta` 字段声明（line 37）及构造时的字段存储（line 63）；`new()` 参数与签名不变（参数仍用于 schema 提取与 data_page_head）
- [ ] 0.3 L3 `src/storage/buffer_pool.rs:139`：按 rustdoc 规范重排 SAFETY 列表缩进
- [ ] 0.4 L4 `src/transaction/manager.rs:380`：`tx2_id` 改名 `_tx2_id`
- [ ] 0.5 L5 `src/storage/page_format/tuple.rs:315`：`&[value.clone()]` → `std::slice::from_ref(&value)`
- [ ] 0.6 L6 `src/storage/page_visibility.rs`：`test_clone_and_copy` 函数加 `#[allow(clippy::clone_on_copy)]` + 意图注释；保留 `.clone()` 断言（该测试用于验证 Clone impl）
- [ ] 0.7 L7 `src/executor/value_ref.rs:207`：删除 `drop(vr);` 行
- [ ] 0.8 `tests/executor_test.rs`：line 903 / 975 / 1062 三处函数内 `use std::sync::{Arc, Mutex};` → `use std::sync::Arc;`（Mutex 当前即未被使用）
- [ ] 0.9 `cargo build && cargo test --lib` 通过（数量与结果与基线一致，0 failures）
- [ ] 0.10 `cargo clippy --all-targets -- -D warnings` 全绿（若暴露表外既有 lint：机械项就地修复并在 Act Response 记录；实质项停止返回 Plan）

## 1. `src/plan_cache.rs` 内核重写（T1）

- [ ] 1.1 改 `pub struct PlanCache { cache: HashMap<String, PhysicalPlan>, max_size }` → `{ map: DashMap<String, PhysicalPlan>, max_size }`
- [ ] 1.2 改 `use std::collections::HashMap;` → `use dashmap::DashMap;`
- [ ] 1.3 改 `new()` / `with_capacity()`：内部初始化 `DashMap::new()`
- [ ] 1.4 改 `get(&mut self, sql) -> Option<&PhysicalPlan>` → `get(&self, sql) -> Option<PhysicalPlan>`（内部用 `normalize_sql_key` + `self.map.get(&key).map(|e| e.value().clone())`）
- [ ] 1.5 改 `put(&mut self, sql, plan)` → `put(&self, sql, plan)`（内部用 `normalize_sql_key` + 满则驱逐 + `self.map.insert(key, plan)`）
- [ ] 1.6 改 `clear(&mut self)` → `clear(&self)`（内部 `self.map.clear()`）
- [ ] 1.7 `len()` / `is_empty()` 改 `&self` + `self.map.len()` / `self.map.is_empty()`
- [ ] 1.8 新增 `pub fn normalize_sql_key(sql: &str) -> String`：单引号 toggle + ASCII lowercase + 空白折叠 + trim
- [ ] 1.9 新增 `#[cfg(test)] mod tests`：10 个单测覆盖 normalize + cache 行为
- [ ] 1.10 `cargo build` 通过
- [ ] 1.11 `cargo test --lib plan_cache::tests` 全部 10 个测试通过

## 2. `src/database.rs` 持有者类型变更（T2）

- [ ] 2.1 改 `pub plan_cache: Arc<Mutex<PlanCache>>` → `pub plan_cache: Arc<PlanCache>`（line 22）
- [ ] 2.2 改 `let plan_cache = Arc::new(Mutex::new(PlanCache::new()));` → `let plan_cache = Arc::new(PlanCache::new());`（line 64）
- [ ] 2.3 改 `pub fn plan_cache_len(&self) -> usize { self.plan_cache.lock().unwrap().len() }` → `self.plan_cache.len()`（line 95）
- [ ] 2.4 grep 确认 `use std::sync::{Arc, Mutex};` 中 `Mutex` 不再被 plan_cache 字段使用；如整文件无 `Mutex` 用法则删除该 import
- [ ] 2.5 `cargo build` 通过

## 3. `src/pipeline.rs` 5 处调用点改造（T3）

- [ ] 3.1 line 56 `let mut cache = database.plan_cache.lock().unwrap(); cache.get(sql).cloned()` → `database.plan_cache.get(sql)`
- [ ] 3.2 line 62 `let mut cache = database.plan_cache.lock().unwrap(); cache.get(sql).cloned()` → `database.plan_cache.get(sql)`
- [ ] 3.3 line 145 `database.plan_cache.lock().unwrap().clear();` → `database.plan_cache.clear();`
- [ ] 3.4 line 169 `database.plan_cache.lock().unwrap().clear();` → `database.plan_cache.clear();`
- [ ] 3.5 line 206-208 `let mut cache = database.plan_cache.lock().unwrap(); cache.put(sql.to_string(), plan.clone());` → `database.plan_cache.put(sql.to_string(), plan.clone());`
- [ ] 3.6 `cargo build` 通过
- [ ] 3.7 `cargo clippy` 通过（无新 warning）

## 4. `tests/executor_test.rs` 5 处 test setup 适配（T4）

- [ ] 4.1 line 705 `plan_cache: Arc::new(Mutex::new(rtsql::plan_cache::PlanCache::new()))` → `plan_cache: Arc::new(rtsql::plan_cache::PlanCache::new())`
- [ ] 4.2 line 745 同 4.1
- [ ] 4.3 line 796 同 4.1
- [ ] 4.4 line 842 同 4.1
- [ ] 4.5 line 879 同 4.1
- [ ] 4.6 line 14 顶层 `use std::sync::{Arc, Mutex};` → `use std::sync::Arc;`（T4 改造后顶层 Mutex 不再被使用；line 903/975/1062 三处函数内 import 已由 T0/0.8 处理，此处不再涉及）
- [ ] 4.7 `cargo build --tests` 通过
- [ ] 4.8 `cargo test --test executor_test` 全部通过

## 5. `tests/plan_cache_test.rs` 新增（T5）

- [ ] 5.1 创建 `tests/plan_cache_test.rs`
- [ ] 5.2 编写 7 个集成测试：case 变体 hit / 空白变体 hit / 字符串字面量大小写不 hit / 100 并发同 SELECT / DML 不进 cache / DDL clear / `normalize_sql_key` 公开函数
- [ ] 5.3 `cargo test --test plan_cache_test` 全部通过
- [ ] 5.4 `cargo test` 全套 487 + 新增测试 pass（0 failures）

## 全量验证

- [ ] V1 `cargo build` 通过
- [ ] V2 `cargo clippy --all-targets -- -D warnings` 通过（T0 完成后此门禁方可满足；T1-T5 不得引入新 warning）
- [ ] V3 `cargo test` 全套测试 pass（基线 487 + 新增 17 = 504；T0 不改变测试数量）
- [ ] V4 `cargo test --test plan_cache_test test_concurrent_hits_do_not_block_runtime` 在 < 5s 完成
- [ ] V5 `cargo test --test pipeline_test` 3 个既有测试仍通过（test_plan_cache_hit / test_ddl_clears_cache / test_dml_not_cached）
- [ ] V6 `git diff --stat -- src/ tests/` 仅包含 11 个已跟踪文件的修改：`src/{plan_cache,database,pipeline}.rs`、`src/transaction/{version_chain,manager}.rs`、`src/executor/{data_scan,value_ref}.rs`、`src/storage/buffer_pool.rs`、`src/storage/page_format/tuple.rs`、`src/storage/page_visibility.rs`、`tests/executor_test.rs`；`git status --short -- tests/` 另含新增 `tests/plan_cache_test.rs`
- [ ] V7 执行前置条件：工作区内与本 change 无关的改动（当前存在 `.agents/skills/*` 删除未提交、`.omo/` untracked）已由用户提交或还原，不混入 V6 检查

## Requirements Traceability Matrix

| Requirement | Scenario | Design Section | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R1 PlanCache key 规范化 | S1 大小写变体 hit | design §1 normalize_sql_key | T1, T5 | 000 | `src/plan_cache.rs::normalize_sql_key`, `src/plan_cache.rs::PlanCache::put/get` | `plan_cache::tests::normalize_variants_share_key`, `plan_cache_test::test_case_variant_hits_cache` | None | Covered |
| R1 PlanCache key 规范化 | S2 空白变体 hit | design §1 normalize_sql_key | T1, T5 | 000 | `src/plan_cache.rs::normalize_sql_key` | `plan_cache::tests::normalize_whitespace_collapse`, `plan_cache_test::test_whitespace_variant_hits_cache` | None | Covered |
| R1 PlanCache key 规范化 | S3 字符串字面量大小写区分 | design §1 normalize_sql_key | T1, T5 | 000 | `src/plan_cache.rs::normalize_sql_key`（toggle 状态机） | `plan_cache::tests::string_literal_case_distinguishes`, `plan_cache_test::test_string_literal_case_does_not_hit` | None | Covered |
| R1 PlanCache key 规范化 | S4 字符串字面量内容保留 | design §1 normalize_sql_key | T1 | 000 | `src/plan_cache.rs::normalize_sql_key` | `plan_cache::tests::normalize_preserves_string_literal` | None | Covered |
| R2 PlanCache 并发无锁访问 | S1 100 并发同 SELECT 全部 hit | design §1 DashMap, §3 pipeline.rs | T1, T3, T5 | 000 | `src/plan_cache.rs::PlanCache::get/put`, `src/pipeline.rs` line 56/62/206 | `plan_cache_test::test_concurrent_hits_do_not_block_runtime` | None | Covered |
| R2 PlanCache 并发无锁访问 | S2 100 并发不同 SQL 写入 | design §1 DashMap, §3 pipeline.rs | T1, T3, T5 | 000 | `src/plan_cache.rs::PlanCache::put` | `plan_cache_test::test_concurrent_hits_do_not_block_runtime`（包含预热后的并发读） | None | Covered |
| R3 DML 与 DDL 行为保持 | S1 DML 不进 cache | design §3 pipeline.rs（is_cacheable 行为不变） | T3, T5 | 000 | `src/pipeline.rs::is_cacheable`, `src/pipeline.rs` line 205-208 | `pipeline_test::test_dml_not_cached`, `plan_cache_test::test_dml_still_not_cached` | None | Covered |
| R3 DML 与 DDL 行为保持 | S2 DDL 清空 cache | design §3 pipeline.rs line 145/169 | T3, T5 | 000 | `src/pipeline.rs` line 145, 169 | `pipeline_test::test_ddl_clears_cache`, `plan_cache_test::test_ddl_still_clears_cache` | None | Covered |
| R4 Database 持有者类型 | S1 Database 字段类型 | design §2 database.rs | T2, T4 | 000 | `src/database.rs::Database.plan_cache`（line 22） | `cargo build` 编译验证；grep 类型断言 | None | Covered |
| R4 Database 持有者类型 | S2 PlanCache API 形态 | design §1 plan_cache.rs | T1 | 000 | `src/plan_cache.rs::PlanCache` 全部方法签名 | `plan_cache::tests` 全部 10 个 | None | Covered |
| R5 验证门禁可用性（clippy 基线归零） | S1 七处既有 clippy 错误清除 | design §6 表 L1-L7 | T0 | 000 | design §6 表所列七处位置 | `cargo clippy --all-targets -- -D warnings` 退出码 0 | None | Covered |
| R5 验证门禁可用性（clippy 基线归零） | S2 executor_test.rs 内部未使用 Mutex import 清除 | design §6（import 段） | T0 | 000 | `tests/executor_test.rs:903,975,1062` | 同上（unused_imports 不再出现） | None | Covered |

**矩阵完整性检查**：

- 5 个 Requirement 全部覆盖
- 12 个 Scenario 全部覆盖
- 6 个 Task 全部覆盖（T0 + T1-T5）
- 1 个 Iteration 覆盖
- 0 个 Missing
- 0 个 Simplified
- 0 个待用户批准裁剪
