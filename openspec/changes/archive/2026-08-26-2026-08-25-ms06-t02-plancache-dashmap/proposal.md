## Why

`src/pipeline.rs` 中 PlanCache 访问点（`src/pipeline.rs:56, 62, 145, 169, 206`）使用 `database.plan_cache.lock().unwrap()`，而 `Database.plan_cache` 字段类型是 `Arc<std::sync::Mutex<PlanCache>>`（`src/database.rs:22, 64`）。这导致两类稳定性 / 正确性风险：

1. **跨 `.await` 持 std 锁**：line 56/62 的 cache hit check 紧接在 `.await` 之前，虽未直接跨 .await，但 M30 连接并发后 100 并发 `SELECT` 会让 `Mutex` 序列化全部 cache 访问，runtime 任务被阻塞
2. **缓存命中率因大小写 / 空白变体而降低**：`plan_cache.get(sql)` 用原 SQL 字符串做 key，`SELECT * FROM t` 与 `select * from t` / `SELECT\n*\nFROM\t t` 视为不同 entry。planner 已对 identifier 全部 `.to_lowercase()`（`src/parser/ast.rs` + `src/parser/planner.rs` 多处），生成的 `PhysicalPlan` 完全相同——只差 key 不命中

MS06-T02 在 `tasks.md` MS06 路线中列为"稳定性与正确性收口"内的次优先任务（与 T01 同类问题：T01 修 DML `tx_id=0` 注入，T02 修 cache 锁与命中率）。

## What Changes

- **（T0 前置，2026-08-25 Plan 审计后经用户批准并入）基线 clippy 归零**：当前基线 `cargo clippy --all-targets -- -D warnings` 存在 7 处既有错误（version_chain.rs 一处由 MS06-T01 提交引入）+ executor_test.rs 三处未使用 Mutex import。全部为机械修复、无行为变化，使全量验证 V2 门禁可满足；明细见 design §6
- **改 `src/plan_cache.rs`**：
  - 内部存储从 `HashMap<String, PhysicalPlan>` 改为 `DashMap<String, PhysicalPlan>`（M31 已引入 `dashmap = "6"`，Cargo.toml:21 就位）
  - 所有方法从 `&mut self` 改为 `&self`（无锁 API）
  - 新增 `normalize_sql_key(&str) -> String` 公开函数：ASCII 折叠 + 空白折叠 + trim；字符串字面量（单引号包裹）内保留原样
  - 公开方法 `get(sql) -> Option<PhysicalPlan>`（clone 返回，调用方 API 兼容）
  - 保持现有"满则驱逐一条"简单策略（不实现 LRU）
  - 新增 `#[cfg(test)] mod tests`：单测覆盖 normalize_sql_key 的多种变体
- **改 `src/database.rs`**：
  - `pub plan_cache: Arc<Mutex<PlanCache>>` → `pub plan_cache: Arc<PlanCache>`
  - `Database::open` 中 `Arc::new(Mutex::new(PlanCache::new()))` → `Arc::new(PlanCache::new())`
  - `plan_cache_len` 内 `self.plan_cache.lock().unwrap().len()` → `self.plan_cache.len()`
- **改 `src/pipeline.rs`**：
  - 5 处 `database.plan_cache.lock().unwrap()` 全部去掉，直接调用 `database.plan_cache.get(sql)` / `.put(...)` / `.clear()` / `.len()`
  - DDL clear 路径（line 145, 169）：`database.plan_cache.clear()` 直接调用
  - Cache 写入路径（line 206-208）：`database.plan_cache.put(sql.to_string(), plan.clone())` 直接调用
  - Cache 读取路径（line 56-58, 62-64）：`database.plan_cache.get(sql)` 直接调用，去掉 `.cloned()`（get 自身已 clone）
- **改 `tests/executor_test.rs`**：5 处 `Arc<Mutex::new(PlanCache::new())>` 改为 `Arc::new(PlanCache::new())`（line 705, 745, 796, 842, 879）
- **新增 `tests/plan_cache_test.rs`**：集成测试
  - SQL 规范化变体 hit：`SELECT * FROM t` / `select * from t` / `SELECT\n*\nFROM t` 共享同一 plan
  - 100 并发同 SELECT：全部 hit，runtime 不阻塞
  - DML 仍不进 cache（保持现状）
  - Cache 满后插入触发驱逐

## Capabilities

### New Capabilities

- `plancache-key-normalization`：PlanCache 的 key 一致性
  - 改前：原 SQL 字符串作为 key，大小写/空白变体视为不同 entry，命中率受客户端代码风格直接影响
  - 改后：所有 entry 通过 `normalize_sql_key` 归一化为同一形式，命中率仅与语义相关
  - 关联 M/K：`M01`（执行管道）、`M09`（异步协程，标准 Mutex 跨 .await 反例）、`M13`（异步执行原则）

## Impact

- **影响模块**：
  - `src/plan_cache.rs`（核心改写）
  - `src/database.rs`（持有者类型）
  - `src/pipeline.rs`（5 处调用站点）
  - `tests/executor_test.rs`（5 个 test setup + import 清理）
  - `tests/plan_cache_test.rs`（新增）
  - T0 机械修复涉及 7 个 src 文件（design §6 表 L1-L7）与 `tests/executor_test.rs` 内部 import——均不改变运行时行为
- **影响接口**：
  - `PlanCache` 公开 API 形态变化（`&mut self` → `&self`）；语义不变（cache hit 返回 clone 后的 plan）
  - `Database.plan_cache` 字段类型变化（`Arc<Mutex<PlanCache>>` → `Arc<PlanCache>`）
  - `Database.plan_cache_len` 行为不变（仍返回 cache 大小）
  - 公开 crate API（`Database::execute_sql`、`Database::open`）无变化
- **影响行为**：
  - 行为差异：相同语义 SQL 的不同写法现在 100% 命中（之前取决于字符串是否完全一致）
  - 行为差异：100 并发 SELECT 同一 SQL 不再有 `Mutex` 串行化争用
  - 行为差异：`get` 不再返回 `&PhysicalPlan` 而返回 `Option<PhysicalPlan>`（调用点 `.cloned()` 去掉即可，API 兼容）
- **兼容性**：
  - 现有 `tests/pipeline_test.rs::test_plan_cache_hit` / `test_ddl_clears_cache` / `test_dml_not_cached` 三个测试通过语义不变维持通过
  - 现有 `tests/executor_test.rs` 5 个 setup 机械调整后维持通过
  - 现有 487 tests pass 基线保持
- **风险**：
  - 中：plan cache key 改变使得内存中旧 entry（如果存在持久化）的失效语义不存在（本项目不持久化 cache，无影响）
  - 中：normalize_sql_key 误处理字符串字面量会导致语义不同的 SQL 误命中 → 单测覆盖 SP3/EC1 显式失败
  - 低：DashMap 替换 HashMap 后内存占用微增（per-shard 开销 ~50B），100 entry 限制下可忽略
- **回退方案**：git revert 本 change；PlanCache 旧 HashMap 实现已存在

## 默认假设（用户 Gate 1 可否决）

1. **规范化范围**：激进（ASCII 折叠 + 空白折叠 + trim）。所有非字符串字面量区域 `.to_ascii_lowercase()`
2. **字符串字面量**：单引号 toggle 状态机；不解决 `O''Brien` 内的转义引号（已知 edge case）
3. **淘汰策略**：保持"满则驱逐一条"，不实现 LRU（`lru` crate 已引入但本 change 不使用）
4. **持有者类型**：`Arc<PlanCache>` 替代 `Arc<Mutex<PlanCache>>`，PlanCache 内部用 DashMap 自管并发
5. **API 形态**：`get` 返回 `Option<PhysicalPlan>`（clone 出来），保持公开 trait 兼容
6. **T0 基线 clippy 修复并入本 change**：7 处既有错误 + 3 处未使用 import 就地机械修复，不建单独 change（用户 2026-08-25 决定）；L6 用 allow 保留"验证 Clone impl"的测试语义，其余直接修复
