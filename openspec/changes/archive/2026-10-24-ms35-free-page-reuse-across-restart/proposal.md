# MS35：空闲页跨重启复用

## Why

MS35（improvements I087）承诺「已释放的页在重新打开后仍可安全复用，并覆盖再次分配与崩溃恢复」。当前实现不满足：

- `src/storage/file_storage.rs:50` 的 `free_pages` 是**仅内存**的 `Mutex<Vec<u64>>`，`FileStorage::open` 时初始化为空（`:197`、`:253`）⇒ 释放的页只在**同一次打开**内可复用，重启后彻底丢失。
- 后果：反复「建表 → 删表」的一次性会话让库文件单调增长——释放的空间跨会话再也回不来（I087 的影响面）。
- MS33 已把「同会话页分配复用被释放页后、在分配方语句的 catalog 落盘之前崩溃」的窗口明确留给**空闲页生命周期的后续能力**（`openspec/specs/drop-persistence-boundary/spec.md` R1 末句），MS35 正是那个能力，须给出该窗口的可观察结果。

## What Changes

- **释放的持久化判定**：一个被释放的页，**只有当它的释放在崩溃边界上已经成立（持久）之后**才成为可分配页。释放在会话内先进入待释放集合，由「在释放完成之后开始且成功完成的 checkpoint 刷脏页」把它提升为可分配。⇒ MS33 留下的窗口不再以「旧表数据被清零且不可恢复」的形式出现。
- **重新打开时的空闲页重建**：打开时按**持久目录**重建空闲页集合（持久目录引用不到的页即为持久空闲页），把重建结果作为本次打开的可分配页集合。⇒ 释放的空间跨重启可用。
- **重建的失败安全**：任一遍历失败（读错误）时**整次重建放弃**（本次打开没有可分配页），只降级不复用，绝不把活页判成空闲页。
- **行为规格增量**：新增规格域 `free-page-lifecycle`（释放的持久化判定 / 重建 / 失败安全 / 崩溃后未完成释放不可分配），并 MODIFIED `drop-table-physical-free`（同进程复用的时机、跨重启释放语义）与 `drop-persistence-boundary`（R1 的窗口归属句由本域接管）。

### 路线评估摘要（完整论证见 `design.md` D1）

| 路线 | 结论 |
|---|---|
| (a) 打开时**安全重建**空闲页集合 | **选定**。空闲集合由「持久目录」推导，与所有权同源 ⇒ 崩溃边界上一致，不需要新的持久结构。 |
| (b) **持久空闲页管理**（把空闲集合单独持久化） | **排除**。空闲集合与目录是两次独立持久化，二者可能不一致：旧的空闲集合会声称「某页空闲」而新目录已把该页登记给别的对象 ⇒ 静默数据损坏；要在不引入可达性遍历的前提下避免这一点，就必须给多页无序刷盘强加跨文件的写序约束，且新增持久产物会牵动 CLI 文件族、格式版本与 MS36 的兼容面。 |

(b) 的「把内存空闲列表原样序列化成持久格式」形态正是 I087 与 MS35-T01 明确排除的「唯一方案」。

## 范围外（明确不做）

- 尾部截断、整库整理、VACUUM 式空间归并（MS35 拆分信号单列，Non-goal）。
- 无键行旧版本 GC（I038，MS35 Non-goal）。
- 文件格式版本变更、新增伴生文件、加密语义变更（`.wal` / `.checkpoint` 仍明文；MS36 负责历史库与 dump 兼容面）。
- 不改 MS33 已验收的 DROP 崩溃语义（`drop-persistence-boundary` 的 R2–R4）、不改 MS34 的 `index-write-side-atomicity` / `ddl-catalog-transactional` / `statement-cancellation-outcome` 语义。
- 不修 ISS22（数据页写满后丢行）、ISS23（BTree 插入无限递归栈溢出）、ISS24（被放弃的 DDL 隐藏改写行）、ISS25（提交点窗口位置性错读）——均为相邻 open Issue，只作约束记录，不在本 change 内修改。
- 不引入身份证明机制（Hash/指纹、revision pin、run-id、artifact manifest、日志 Hash 链、时间顺序证明等）与判定层；验证只落在 `cargo test` 集合内。

## 授权与批准依据

- **Gate 1（需求与范围批准）**：已批准。依据为 goal 指令原文「/openspec-autopilot 把已经规划的三个ms逐步完成，要求遵守各种规范，尤其是全量测试时候的多批次串行」+ 补充「因为这个项目有点大，三个ms也是串行实施就好」+「对于实施当中发现的所有issue候选，都直接寻找好的承载方式流程进行修复而不是等级」。
- **Gate 2（执行就绪批准）**：已批准（同上依据）。
- 批准**不降低**检查标准：BDD 缺口扫描、实现调查、RTM、设计闭合、Iteration 平衡审计照常完整执行；goal 原文记为**批准依据**（不是「跳过」）。
- 本轮只做计划，不实施、不改产品代码。

## 影响面

- 产品代码：`src/storage/file_storage.rs`（空闲页生命周期状态）、`src/storage/async_storage.rs`（trait 接缝）、`src/storage/data/table_manager.rs`（活页集合重建、释放完成标记）、`src/wal/checkpoint.rs`（checkpoint 钩子）、`src/database.rs`（打开钩子）、`src/executor/drop_index.rs`、`src/executor/create_index.rs`（释放完成标记）、`src/storage/page_format/overflow.rs`、`src/storage/data_page.rs`（溢出链释放与 GC 路径的释放完成标记）。
- 测试代码：新增 `tests/free_page_lifecycle_test.rs`；更新 `tests/drop_table_free_test.rs`（5 个同会话复用断言改为「释放持久后复用」）、`tests/overflow_page_test.rs`（TRUNCATE 溢出链页的清零/复用断言）与 lib 单测 `free_overflow_chain_returns_pages_for_reuse`。
- 规格：`openspec/specs/free-page-lifecycle/`（新增）、`openspec/specs/drop-table-physical-free/`、`openspec/specs/drop-persistence-boundary/`（MODIFIED）。规格合并由 `openspec-docs-maintainer` 在 change 收尾时执行。
