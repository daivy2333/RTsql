# 组合二级索引与范围访问任务

对应路线：MS39。来源：I085 后半。Gate 1 与 Gate 2 为 PASS，批准来自本次目标对已写范围的授权，不是跳过。本文件不更新全局里程碑状态。

## 当前批次状态

- 需求与 BDD：completed。
- 000：执行依据是 `iterations/000-key-and-catalog/000-initial.md`。Review Result 为 accepted。
- 001：执行依据是 `iterations/001-create-maintain-recover/000-initial.md`。Review Result 为 accepted。
- 002：执行依据是 `iterations/002-equality-and-range-routing/000-initial.md`。Review Result 为 accepted。
- 003：执行依据是 `iterations/003-fault-boundary-and-full-suite/000-initial.md`。Plan Context 为 ready。
- Persisted Evidence：none。
- 无跳过项。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | 世代 1 的键派生和范围哨兵。先行测试 RED 后实现 GREEN。世代 0 的整数大端字节保持 | completed |
| T2 | 000 | T1 | 目录尾段、内存元数据的额外列号和世代。旧单列记录字节不变，组合记录可往返。SQL 仍拒绝多列索引 | completed |
| T3 | 001 | 000 accepted | `CREATE INDEX` 建组合索引和新建 INT 索引，回填、DML、回滚、恢复使用同一派生。删列守卫认识全部列号 | completed |
| T4 | 002 | 001 accepted | 等值与范围路由、EXPLAIN、补扫和索引选择。结果与全表扫描相同 | completed |
| T5 | 003 | 002 accepted | 取消、只读、内存、加密、附加库和旧库组合；README；按 M19 分批的一次全量 | pending |

## 1. 键与目录

- [x] 1.1 T1：为有序整数、组合拼接、超长与 NULL、哨兵端点建立先行测试并实现派生。验证：`cargo test --offline --lib executor::value::tests` 中新增测试与 `secondary_index_key_preserves_int_bytes` 均为 passed、failed 为 0。
- [x] 1.2 T2：目录尾段只在世代非 0 或多列时出现，旧单列行长度不变。验证：catalog 往返测试 passed、failed 为 0；`string_index_rejection_readonly_and_transaction` 仍因 `MULTI-COLUMN INDEX` 拒绝多列建索引。

## 2. 生命周期

- [x] 2.1 T3：SQL 创建组合索引和新建单列 INT 索引，条目与存活行一致，含回滚和未决 WAL。验证：本轮新增的库或集成测试直接比较条目与查询行，failed 为 0。

## 3. 路由

- [x] 3.1 T4：等值走 `IndexScanAll`，范围走 `IndexRangeScan`，字符串范围带 `omitted=scan`，旧 INT 范围仍扫描。验证：SQL 测试比较 EXPLAIN 文本和行多重集合，failed 为 0。

## 4. 收尾

- [ ] 4.1 T5：补齐故障与运行形态，更新两份 README 的二级索引段落，并按 Iteration 003 的分批规则跑完一次全量。缺一批不得记为通过。

## Iteration Plan

### Iteration 000：键派生与目录世代

- Tasks：T1、T2。
- Depends on：None。
- Stable result：世代 1 的拼接规则和哨兵函数可单测。目录能保存组合列号和世代。已有单列索引的磁盘尾段不增长。用户仍不能通过 SQL 创建组合索引。
- Verification boundary：`executor::value` 与 catalog 的库测试，加上一条仍拒绝多列 `CREATE INDEX` 的集成测试。不跑全量。高开销全量留在 003，整个变更只预算一次。
- Diagnostic boundary：键字节、哨兵比较和 `__tables` 尾段。计划器、执行器和 DML 不在本轮修改。
- Non-goals：SQL 开放组合索引、范围执行器、README。

### Iteration 001：创建、维护与恢复

- Tasks：T3。
- Depends on：000 accepted。
- Stable result：SQL 可以创建规格允许的组合索引和新建单列 INT 索引。回填、插入、更新、删除、UPSERT、回滚和未决 WAL 重建后的条目与存活行一致。查询仍可以继续扫描，本轮不要求出现索引计划。
- Verification boundary：通过真实 `Database::execute_sql` 或 `CreateIndexExecutor` 建索引，再用 `search_all` / `scan_all` 观察条目，用 `SELECT` 观察行。不跑全量。
- Diagnostic boundary：创建、回填、DML 和恢复的派生调用。范围路由的错误归 002，不在本轮用扫描结果冒充索引命中。
- Non-goals：EXPLAIN 路由、范围行走、文档。

### Iteration 002：等值与范围路由

- Tasks：T4。
- Depends on：001 accepted。
- Stable result：规格中的等值和范围查询有可观察计划，行多重集合与全表扫描相同。旧 INT 范围不进入 `IndexRangeScan`。
- Verification boundary：SQL 与 EXPLAIN 的集成测试，覆盖负数、组合前缀、超长字符串、不该路由的谓词。不跑全量。
- Diagnostic boundary：计划器谓词形态、哨兵消费、范围行走和补扫。条目维护错误退回 001 的验收，不在本轮改派生规则。
- Non-goals：全量回归、README、新的索引语法。

### Iteration 003：故障边界与全量

- Tasks：T5。
- Depends on：002 accepted。
- Stable result：取消、只读、内存、加密、附加库和旧库上的可达组合符合规格。README 与行为一致。全量分批全部 0 failed。
- Verification boundary：定向组合测试之后，整个变更的一次全量。目标是 `--lib`、`--bins` 和每个 `tests/*.rs` 的文件名（不含扩展名），按 C 本地字典序。设 N 为现场目标数，`B = ceil(N / 12)`。前 `B * 12 - N` 批各 11 个目标，其余各 12 个。批间串行，同一时刻只有一批。每批命令是 `cargo test --offline` 加上该批的 `--lib`、`--bins` 或 `--test <名>`，最后是 `-- --test-threads=1`。缺一批不得声明全量通过。单次定向测试可以用默认并行度。本轮另运行本变更的 `openspec validate --strict`。本变更新编辑的 Rust 文件必须能被 `cargo fmt` 接受。全局 `cargo fmt --check` 和 `cargo clippy --offline --all-targets -- -D warnings` 跑一次并记录。失败若只位于未修改文件且与 ISS36 一致，不把 ISS36 的修复加进本变更，也不因此把 T5 判失败。新编辑文件上的格式或 lint 失败属于 T5，必须在本轮修好。
- Diagnostic boundary：运行形态、取消回收、文档和全量回归。不改键编码和路由规则，除非 002 的验收实际未满足；那属于返工，不提前写进本计划。
- Non-goals：ISS36 本身、MS41–MS44、表达式索引和部分索引。

## 平衡审计

000 只冻结键和目录，完成后还没有用户可见的组合索引，所以没有把后续 SQL 放进来。001 把创建和写入收成一个可对照条目与行的结果；若再拆成「只建索引」和「再维护」，中间状态会留下过期条目，不能单独验收。002 只看计划和行集，故障矩阵放到 003，避免路由失败和取消失败混在同一轮。003 承担预算内的唯一一次全量。四轮的切分标准相同，000 没有因为编号而装下整个变更。

## Requirements Traceability Matrix

| Requirement | Task | First Test | Status |
|---|---|---|---|
| CREATE INDEX 建立单列 INT 二级索引 | T2, T3 | `ordered_int_key_matches_signed_order`；`new_single_column_int_index_uses_ordered_key` | Covered |
| 二级索引等值查询路由可达 | T4 | `composite_equality_names_index` | Covered |
| 既有语义零回归 | T2, T5 | `secondary_index_key_preserves_int_bytes` | Covered |
| 范围谓词路由到二级索引扫描 | T1, T4 | `range_sentinel_composite_prefix_excludes_next_leading_value`；T4 的 `new_int_range_includes_negatives` | Covered |
| 组合二级索引的条目与存活行一致 | T3 | `composite_update_keeps_the_other_duplicate`；`composite_rollback_removes_uncommitted_entry` | Covered |
| 唯一性边界保持在整数单列 | T3, T4 | 001 起保留的 `CREATE UNIQUE INDEX` 拒绝断言 | Covered |

没有 Simplified 行。场景、设计和代码位置分别在 proposal、design 和首轮 Cycle 中。

## 实施中新发现的缺陷

不预建轮次。若实施中出现当前任务没有覆盖的实质缺陷，先登记，再由计划在本变更增加 Iteration。该授权来自本次目标，只覆盖本变更实施中发现的缺陷，不把 MS41–MS44 或表达式、部分索引拉进来。
