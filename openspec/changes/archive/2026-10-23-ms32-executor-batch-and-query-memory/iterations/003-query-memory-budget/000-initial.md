# Iteration 003 / Cycle 000: 查询内存上限

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: 4.1、4.2、4.3、4.4
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 的查询内存上限段落；design D4（预算对象与配置通道）、D5（计费落点）；`query-memory-budget` 全部 Requirement 的行为面。
- Excluded scope: 最终结果集计费、CLI flag、取消与超时、外部排序溢写、data_scan.rs:687 格式清理（另行轻量处理）。

**Objective**

每语句查询内存预算成立：默认 0 不限制；`Database::set_query_memory_limit` 与 `RTSQL_QUERY_MEMORY_BYTES` 双通道配置，非法值打开前 exit 2；驻留点按 16 字节 + 字符串长度计费，超限整句失败 `Execution error: query memory limit exceeded (limit N bytes)` + SQLSTATE 53200，下一条语句不继承计数。

**Background**

Iteration 000–002 已交付默认 `next_batch`、哈希 JOIN 左表逐行与有界排序；本轮在其上建立资源契约。小右表 + 大左表 JOIN 在小上限下成功直接消费 Iteration 001，`LIMIT` 有界排序在小上限下成功直接消费 Iteration 002。

**Investigation Facts**

- Starting point: Iteration 002 最终 Act Response（2026-10-23）+ accepted Review：有界排序交付（`SortNode.bounded` 两包装点赋值、sort.rs 驻留 ≤ K）、sort_test 净 +2 条、全量 M19 14 批 2128 passed / 0 failed / 2 ignored、六触碰文件 fmt/clippy 净。本轮沿该现场，未重跑。
- Current-State Evidence:
  - 错误漏斗：`execute_executor`（`src/pipeline.rs:1590` 附近）把执行器 `Err(StorageError)` 统一转为 `SqlError { message: "Execution error: {e}", sqlstate: None }`——所有执行错误现无 SQLSTATE。新变体需在此加分类臂。
  - 建执行器错误转换臂（现为 `e.to_string()` + `sqlstate: None`）：主读路径 `pipeline.rs:641`；in-tx `:569`；in-tx routed `:1298`；CTAS select `:852`。`SqlError` 定义 `src/result.rs:40`（`message` + `sqlstate: Option<String>`）。
  - INSERT SELECT 源物化：`materialize_insert_select_source`（`pipeline.rs:755`）返回 `Result<Vec<Vec<Value>>, String>`——错误经 `e.to_string()` 折成 String，消费点 `:504`（隐式事务臂）与 `:1268`（in-tx 臂）以 `sqlstate: None` 转 `SqlError`。**String 通道丢失变体信息，超限分类穿不过去**——返回类型需携 带 `StorageError`（或等价分类），属 D4「INSERT SELECT 三条都断言同一消息和 53200」的必要后果。
  - 语句入口（预算读取点）：`execute_stage` `pipeline.rs:383`（委托 `execute_stage_routed:393`）、`execute_stage_in_tx:1168`、`execute_stage_in_tx_routed:1178`、`execute_with_params:113`、`execute_in_tx_with_params:1103`；预编译 `PreparedStatement::execute`（`src/database.rs`，经闸门后走 pipeline 上述入口）。执行器工厂 `create_executor_from_plan`（`pipeline.rs:2282` 起递归，签名 database/tx_id/snapshot/routed）——预算 `Arc` 作为新参数穿线，子查询重建臂（`SubqueryEval`/`ScalarSubqueryFilter`/`Semi`/`Anti`/`DerivedScan` 的重建调用点）沿用同一 `Arc`。
  - `Database` 字段先例：`schema_version: Arc<AtomicU64>`（`src/database.rs:114–118`，open 初始化 `:385`）。`query_memory_limit: AtomicU64` 默认 0 同构放置（D4 未要求跨克隆共享，`AtomicU64` 克隆取值快照即可）。
  - `StorageError` 尾部（`src/storage/error.rs`）：`ExecutionError(String)` 的 Display 无类别前缀纪律（ISS19 单层前缀由漏斗赋予）——`QueryMemoryExceeded { limit }` 的 Display 应为 `query memory limit exceeded (limit {limit} bytes)`，经漏斗后成固定公共消息。
  - CLI 通道先例：打开前校验位 `src/cli/mod.rs:301`（`checkpoint_bytes_from_env` Err → `ExitStatus::Usage`，exit 2 零副作用）；解析函数 `:422`；打开后设置器应用位两处 `:655–656`（主命令，含只读/内存会话说明）与 `:784–785`（ATTACH 附加库）。`RTSQL_QUERY_MEMORY_BYTES` 在同三点位并行接入，非法值消息点名变量与非法值。
  - D5 驻留点锚位：`scan.rs:22 results`；`join.rs:36 right_hashmap`（右行 + 哈希键，左行只作当前探测行不计）；`nested_loop_join.rs:38 right_rows`；`semi_join.rs:49` / `anti_join.rs:61 right_hashmap`（及相关缓存）；`sort.rs` `sorted_rows`/有界 `retained`；`distinct.rs:39` 输出行 + `seen` 集合副本；`set_operation.rs:32 rows` + `seen`/右侧 `HashSet`；`aggregate.rs` 分组键与状态值（纯数值累加器不计）；`subquery_eval.rs:43 correlated_cache`；`scalar_subquery_filter.rs` 缓存；`pipeline.rs:2097` 附近 DerivedScan 臂源行；`materialize_insert_select_source:755` 源行；CTAS `execute_ctas_routed:829` 的 select 源。`DataScan`/`IndexScan`/`IndexScanAll` 只持当前行不加整表计费。
  - 计费口径（D4）：`value_bytes` = String 取 `16 + len`、其余恒 16；行 = 各值之和；驻留集合插入时计费、丢弃时按相同字节释放；最终 `execute_executor` 结果 `Vec` 不计。
  - 测试先例：CLI 非法 env 值 → exit 2 + 文案含变量名与非法值 + 零库文件，见 `tests/wal_auto_checkpoint_test.rs:593–606`（helper `:131`，120s 超时）；本 change SQL 级新测试先例 `tests/join_test.rs`（Iteration 001）与 `tests/sort_test.rs`（Iteration 002）的 `Database::execute_sql` 形态。
- Code and Critical Path: `Database::execute_sql`/`execute_in_tx`/`PreparedStatement::execute` → `execute_stage[_routed][_in_tx]`（预算构造）→ `create_executor_from_plan`（Arc 穿线）→ 各驻留算子 charge/release → `QueryMemoryExceeded` 上抛 → 漏斗与三转换臂分类 53200。CLI：`execute_command` 顶部校验 → 打开后设置器。

**Implementation Guidance**

按 D4：预算对象语句内共享（`Arc`），`limit == 0` 时 `charge` 直接成功不改计数——默认面零开销零行为差异。按 D5 逐驻留点接入；`INSERT SELECT`/`CTAS`/`DerivedScan` 源行在函数返回前释放。释放按插入时记录的同一字节数回退，不重算。`materialize_insert_select_source` 返回类型从 `String` 改为携带 `StorageError` 的形态时，宽高错误等既有消息保持逐字节不变（仅超限获得分类）。

**Task Definitions**

### 4.1: 预算对象与配置通道

- Requirement/Scenario: query-memory-budget「默认不限制」与「非法配置在打开前拒绝」。
- Depends on: Iteration 002
- Targets: `src/database.rs`（`query_memory_limit: AtomicU64` 字段 + `set_query_memory_limit`）；`src/executor/query_budget.rs`（新模块：预算对象 + `value_bytes` 口径）；`src/pipeline.rs` 语句入口族（预算构造）；`src/cli/mod.rs:301` 校验位、`:422` 旁解析函数、`:655`/`:784` 应用位；`tests/query_memory_test.rs`（新文件）
- Current behavior: 无查询内存概念；`RTSQL_QUERY_MEMORY_BYTES` 未被消费，任意值下行为不变。
- Required behavior: 默认 0 = 不限制（既有查询全部成功，行为与 Iteration 002 终态一致）。`Database::set_query_memory_limit(bytes)` 只影响之后开始的语句。`RTSQL_QUERY_MEMORY_BYTES` 非法值在打开前 exit 2、零库文件、stderr 点名变量与非法值；合法值打开后应用（主命令与 ATTACH 两处，同 checkpoint 水位通道）。
- Preserve: 既有 env 通道行为（KEY/BUSY_TIMEOUT/WAL_CHECKPOINT）逐字节不变；未设置时零行为差异。
- Forbidden: 新增 CLI flag；在执行中途改变已开始语句的限额；非法值在打开后才拒绝。
- First test: `tests/query_memory_test.rs` 新建两条并先于产品代码运行：`default_zero_limit_allows_existing_queries`——不设上限建表插入若干行后 SELECT/JOIN/ORDER BY 全成功（改前 GREEN，锁定默认面）；`invalid_env_value_rejected_before_open`——spawn CLI 传非法 `RTSQL_QUERY_MEMORY_BYTES`，断言 exit 2、stderr 含变量名与非法值、零库文件（沿 `wal_auto_checkpoint_test.rs` helper 先例；改前 RED——当前 env 被忽略退出码非 2）。命令：`cargo test --test query_memory_test -- --test-threads=1`。
- GREEN condition: 两条测试通过；`set_query_memory_limit` 存在且语义正确；默认 0 时无计费开销路径成立（charge 短路）。
- Verification: `cargo test --test query_memory_test -- --test-threads=1` 退出码 0。
- Stop when: 校验位插入点与 `checkpoint_bytes_from_env` 现有早退结构冲突，或设置器应用位覆盖不了 ATTACH/只读/内存形态。

### 4.2: 驻留点计费与释放

- Requirement/Scenario: query-memory-budget「驻留字节按固定口径计费」（派生表/右表 JOIN/无界排序/DISTINCT/INSERT SELECT 五形状）与「哈希 JOIN 不驻留整表左输入」「LIMIT 直接约束排序时只驻留有界行」的内存面。
- Depends on: 4.1
- Targets: `src/executor/query_budget.rs` 口径 + D5 全部驻留点（Investigation Facts 锚位清单）
- Current behavior: 各算子驻留集合无上界；任意上限设置后查询仍成功。
- Required behavior: 上限 > 0 时各驻留点插入前 charge、丢弃时 release；超限返回 `QueryMemoryExceeded`。最终 `execute_executor` 结果 `Vec` 不计。直接 `SELECT *`（单行驻留）在 16 字节上限下成功；派生表、右表 JOIN、无界排序、`DISTINCT … LIMIT 1` 在小上限下按 spec 失败；小右表 + 大左表 JOIN 与 `LIMIT` 有界排序在小上限下成功。
- Preserve: 计费不得改变任何结果行、错误文案（除新增超限错误）与执行顺序；默认 0 时全部路径短路。
- Forbidden: 对最终结果行计费；跨语句共享计数；为计费新增采样/监控接口。
- First test: `tests/query_memory_test.rs` 增五形状测试（改前 RED——当前小上限下不失败）：`direct_scan_succeeds_under_16_bytes`、`derived_table_fails_under_small_limit`、`join_right_side_fails_under_small_limit`、`unbounded_sort_fails_under_small_limit`、`distinct_limit_fails_under_small_limit`；另两条正向：`small_right_large_left_join_succeeds_under_small_limit`、`bounded_sort_limit_succeeds_under_small_limit`。命令同 4.1。
- GREEN condition: 七条测试通过；release 与 charge 同字节回退（无泄漏累积——同语句多次查询不误超）。
- Verification: `cargo test --test query_memory_test -- --test-threads=1` 退出码 0。
- Stop when: 某 D5 驻留点的数据结构无法在插入/丢弃点取得字节口径（回到计划，不猜口径）。

### 4.3: 三入口错误面与语句隔离

- Requirement/Scenario: query-memory-budget「超限整句失败」（三公共入口同一消息与 53200；下一条语句不继承；写语句零副作用）。
- Depends on: 4.2
- Targets: `src/storage/error.rs`（`QueryMemoryExceeded { limit }` 变体）；`src/pipeline.rs:1590` 漏斗分类臂、`:641`/`:569`/`:1298`/`:852` create 转换臂、`:504`/`:1268` InsertSelect 转换臂（含 `materialize_insert_select_source` 返回类型调整）；`tests/query_memory_test.rs`
- Current behavior: 漏斗与全部转换臂 `sqlstate: None`；InsertSelect String 通道丢失变体。
- Required behavior: `Database::execute_sql`、`Database::execute_in_tx`、`PreparedStatement::execute` 三入口超限都返回消息 `Execution error: query memory limit exceeded (limit N bytes)` 与 SQLSTATE `53200`；DerivedScan 建执行器期超限同面；超限后的下一条语句从零计数成功；`INSERT SELECT` 超限零写入（目标表无行）。
- Preserve: 其他错误的文案与 sqlstate 逐字节不变；InsertSelect 宽高错误等既有消息不变。
- Forbidden: 为超限新增独立错误前缀层级；改动其他 StorageError 变体的 Display。
- First test: `tests/query_memory_test.rs` 增：三入口各一条断言消息与 `sqlstate == Some("53200")`；`next_statement_does_not_inherit_count`（超限语句后同语句重跑在足额上限下成功）；`insert_select_over_limit_writes_nothing`（改前 RED——当前不失败或无 53200）。
- GREEN condition: 全部通过；grep 核对五转换臂均含 53200 分类。
- Verification: `cargo test --test query_memory_test -- --test-threads=1` 退出码 0。
- Stop when: 某入口的错误路径绕过全部五转换臂（回到计划补臂，不复制漏斗）。

### 4.4: 全量分批回归

- Requirement/Scenario: 内存上限不改既有行为的回归面。
- Depends on: 4.3
- Targets: 无产品代码
- Current behavior: Iteration 002 结束时 2128 passed / 0 failed / 2 ignored；本轮新增测试数由 Act 记录并对账。
- Required behavior: 按项目模型 M19 分批（同 1.4/2.2/3.2），每批 0 failed。
- Preserve: 不新增测试框架外的判定脚本。
- Forbidden: 一次无过滤的 `cargo test`。
- First test: 无。消费 4.1–4.3 已通过的测试。
- GREEN condition: 每批 failed 0；总数 = 2128 + 本轮净增数。
- Verification: 各批计数写入 Act Response。
- Stop when: 某批失败。范围内先修；范围外回到计划。

**Invariants**

- 默认 0（未设置）时系统行为与 Iteration 002 终态一致。
- 超限错误消息与 SQLSTATE 固定；其他错误面零变化。
- 计费口径单源（`value_bytes`）；charge/release 同字节。
- 语句级预算生命周期：语句结束计数归零，不跨语句。

**Non-goals**

最终结果集计费、CLI flag、取消与超时、外部排序溢写、跨算子资源管理器、data_scan.rs:687 格式清理。

**Acceptance**

- 未设置时既有查询成功；非法 env exit 2 零库文件（4.1）。
- 16 字节上限直接扫描成功；五形状按 spec 失败/成功；JOIN 左表与有界排序的内存收益在小上限下成立（4.2）。
- 三入口同消息同 53200；下一条语句不继承；INSERT SELECT 零写入（4.3）。
- 4.4 各批 0 failed，总数对账。

**Verification**

- 全部经 `Database` 公共入口或真实 CLI 进程（`tests/query_memory_test.rs`），无内存采样判定——判定只看成功/失败/消息/SQLSTATE/表行数。
- 4.4 沿用 M19 分批。

**Gate 2 Readiness**

PASS。任务 4.1–4.4 属 2026-10-23 用户原话「批准」的四轮计划，机制为 design D4/D5 既定内容，本轮无新增范围。无 BLOCKED，无 WAIVED。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

计费点遗漏会让「应当失败」的形状成功——spec 五形状测试逐点锁住（design 风险行）。`materialize_insert_select_source` 返回类型调整是既有 String 通道穿不过 SQLSTATE 的必要后果，已在 Task Contract 点名。并发语句各自持语句级预算，`Database` 上限读取为快照语义。

## Act Response

- Status: reported

### 实际改动

**4.1 预算对象与配置通道**

- `src/executor/query_budget.rs`（新）：`QueryBudget`（`limit: u64` + `used: AtomicU64`；`charge` 超限回退计数并返回 `StorageError::QueryMemoryExceeded`、`release` 按入账同字节回退、`limit == 0` 全短路）+ 口径单源 `value_bytes`（String 16+len、其余 16）/`row_bytes`/`rows_bytes`/`string_bytes`/`param_key_bytes` + 哈希桶插入计费单源 `charge_bucket_insert`（新桶入账键字节，返回新增字节）+ 4 条单测（limit 0 短路 / 超限回退 / release 复原 / 口径）。
- `src/executor/mod.rs`：`pub(crate) mod query_budget` + `pub use query_budget::QueryBudget`。
- `src/storage/error.rs`：`QueryMemoryExceeded { limit }` 变体，Display `query memory limit exceeded (limit {limit} bytes)`（无前缀纪律同 `ExecutionError`）。
- `src/database.rs`：字段 `query_memory_limit: Arc<AtomicU64>`（open 归零、克隆共享）+ `pub fn set_query_memory_limit` + `pub(crate) fn query_memory_limit`。
- `src/cli/mod.rs`：`ENV_QUERY_MEMORY_BYTES` + `query_memory_bytes_from_env`（镜像 checkpoint 通道）三接入点——`execute_command` 顶部打开前校验（exit 2 零副作用）、`execute_command_inner_mode` open 后应用（主命令 + 生命周期子命令）、`attach_database` 附加库应用。

**4.2 语句入口预算与 D5 驻留点计费**

- `src/pipeline.rs`：`execute_stage_routed` 与 `execute_stage_in_tx_routed` 两叶子各构造语句级 `SharedBudget`（两叶子覆盖全部执行入口，含 plan cache 命中路径与预编译语句）；`create_executor_from_plan` 增 `budget: &SharedBudget` 参数全子树穿线（Arc 克隆进 future，不新增 'a 捕获）；DerivedScan 臂源行逐行入账、物化完成整段出账；`materialize_insert_select_source` 与 `execute_ctas_routed` 增 budget 参数同型计费（峰值=全部源行，物化完成出账）。
- 驻留执行器（字段 + `pub(crate) with_budget` + 计费）：`sort.rs`（全量逐行入账；有界替换先出账被逐出行再入账新行）、`join.rs`（BuildRight 经 `charge_bucket_insert`：右行 + 桶键；左行不计）、`nested_loop_join.rs`（right_rows 逐行）、`distinct.rs`（幸存行 2×：输出行 + seen 副本；瞬态克隆不计）、`set_operation.rs`（两侧分支行入账；去重/交差组合期 seen 副本与右集入账、丢弃行出账）、`aggregate.rs`（新组分组键入账 + 状态值更新前后增量入账/出账——纯数值累加器恒 0，`limit == 0` 跳过度量零开销）、`subquery_eval.rs` / `scalar_subquery_filter.rs`（相关缓存键值入账 + 重建臂传同一预算）、`semi_join.rs` / `anti_join.rs`（独立路径单次建表计费；相关路径换表先按记账字节出账旧表、缓存键值 + 行集入账）。

**4.3 三入口错误面与语句隔离**

- `src/pipeline.rs`：`storage_error_sqlstate`（超限 53200、其余 None）与 `executor_creation_error`（超限获得 `Execution error: ` 前缀 + 53200 与漏斗同面；其余错误裸文案逐字节不变）两单源 helper；漏斗 `execute_executor` + 四 create 转换臂（DML/query/in-tx/CTAS）+ 两 InsertSelect 物化臂 + CTAS charge 臂共七消费点接入；`materialize_insert_select_source` 返回类型 `String` → `StorageError`（String 通道丢失变体信息；非超限错误 Display 与既有文案逐字节一致）。
- `tests/query_memory_test.rs`：12 条（4.1 两条 + 4.2 七形状 + 4.3 三条：三入口同消息同 53200 / 语句计数不继承〔超限残留判别 + 同上限连续两次小排序〕 / INSERT SELECT 超限零写入）。

### Deviations from Plan

1. **`Database` 字段形态**：计划「`query_memory_limit: AtomicU64`（克隆取值快照）」——`AtomicU64` 未实现 `Clone` 而 `Database` 为 `derive(Clone)`，落地为 `Arc<AtomicU64>`（沿 `schema_version` 先例；克隆共享不改变「上限变更只影响之后开始语句」的快照语义）。
2. **4.1/4.2 边界重划**：计划 4.1 targets 含「pipeline 语句入口族（预算构造）」——无消费方的构造无法免警告编译，语句入口预算构造、工厂参数穿线与执行器 `with_budget` 接线全部移入 4.2 与首批 charge 一并落地；4.1 以预算对象 + 双通道 + charge 短路单测为 GREEN 边界（4.1 GREEN 条件全部满足）。
3. **`StorageError::QueryMemoryExceeded` 落地时点**：计划把 `src/storage/error.rs` 列在 4.3 targets——`QueryBudget::charge` 的完整语义（超限返回该变体）在 4.1 就需要它，变体随预算对象在 4.1 引入，4.3 只做 53200 分类接线与消息面。
4. **D5 锚位 `scan.rs:22 results` 不充电**：锚位清单列有该点，但 spec R3「最终返回给调用者的结果行不计」与 Task 4.2 Required behavior「直接 `SELECT *`（单行驻留）在 16 字节上限下成功」共同唯一确定 Scan 的 `results` 物化（即该查询的最终结果集）不计费；经其上层算子（Join 哈希表、Distinct、外层 Sort、派生表物化）的驻留照常计费。锚位按 Task Definition 与 spec 口径澄清，无行为分歧。

### 自检发现与遗留

- clippy `double_must_use` 警告 13 → 14（+1）：定位 `src/executor/executor_trait.rs:9`（`#[async_trait]` trait 面的 async 默认方法展开），为 Iteration 000 已 accepted 的 `next_batch` 与当前 clippy 1.99 的相互作用，非本轮触碰文件引入；本轮全部触碰文件零新增警告。既有 13 条（executor_trait/protocol/async_storage 的 async_trait 展开 ×8、into_iter ×2、empty doc comment、match 单臂、if collapse）在 HEAD 即存在，未处理（Surgical Changes）。
- 无其他 Critical/Important 遗留。

### 验证

- `cargo test --test query_memory_test -- --test-threads=1` → 12 passed / 0 failed（4.1 两例先建先跑：默认面 GREEN、非法 env 改前 RED〔exit 1〕；4.2 五失败形状改前 RED；4.3 sqlstate 断言改前 RED）。
- `cargo test --lib query_budget -- --test-threads=1` → 4 passed / 0 failed。
- 定向回归：join/chain_join/self_join/sort/order_by_alias/limit/select_distinct/set_operations/aggregate/aggregate_extension/subquery/scalar_subquery_comparison 13 文件 207 tests / 0 failed；insert_select/prepared_statement/param_binding 3 文件 52 tests / 0 failed。
- 全量 M19 分批（4.4）：`--lib --bins -- --test-threads=1` 一批 420/0/0 ignored；`tests/*.rs` 字典序 129 文件每 10 个一批 × 13 批、每批 `--test-threads=1`，共 1724 passed / 0 failed / 2 ignored。合计 **2144 passed / 0 failed / 2 ignored**，对账 2128（Iteration 002 终态）+ 12（query_memory_test）+ 4（query_budget 单测）= 2144 吻合，ran = want 129 文件全覆盖。
- `cargo fmt` 后 `cargo fmt --check` 全库零 Diff——顺带清除了 Iteration 001 Review 记录的遗留 Minor `data_scan.rs:687`（Iteration 000 测试代码的 `DataScanExecutor::new(...)` 换行，纯格式轻量项；全量回归在 fmt 之后运行，改前改后均 GREEN）。`cargo clippy --lib --bins` 本轮触碰文件零警告（见上）。

### Persisted Evidence

- None required（全部结论可由上述命令低成本重跑）。

### Experience Candidates

- None（无 change 范围外实质缺陷发现；M19 分批流程已有先例，无新 Runbook 素材）。

### 未解决问题

- 无。

## Plan Review

- Review Result: accepted

**Findings**

独立阅读全部触碰文件 diff（query_budget.rs 新文件 + pipeline/database/cli/error/十执行器/query_memory_test），工作区与 Act 声明范围一致（`git status` 24 文件，其中 13 文件属前三轮既有交付）。

预算内核独立验证：`QueryBudget::charge` 超限回退本次入账（fetch_add + 溢出判定 + fetch_sub 回退）、`release` 按入账同字节回退、`limit == 0 || bytes == 0` 双短路；`charge_bucket_insert` 空桶入账键 + 行、既有桶只入账行——与「同一份值每个驻留副本分别计费」「哈希键算驻留」口径一致；`value_bytes`/`row_bytes`/`rows_bytes`/`param_key_bytes` 单源。语句内执行器树由 `execute_executor` 顺序驱动，无并发 charge 竞态。

接线独立验证：预算在 `execute_stage_routed` 与 `execute_stage_in_tx_routed` 两叶子构造（覆盖直接/显式事务/参数/预编译全部入口与 plan cache 命中路径）；`create_executor_from_plan` 新 `budget` 参数以 Arc 克隆进 future（不扩 `'a` 捕获）穿线全部子树；SubqueryEval/ScalarSubqueryFilter/Semi/Anti 的重建臂传同一预算（diff 逐点核对）。七处转换臂（DML create ×2、query create、CTAS create、CTAS charge、两 InsertSelect 物化）+ 漏斗全部经 `executor_creation_error`/`storage_error_sqlstate` 两单源 helper——`grep` 核对恰七消费点 + 漏斗一处；非超限错误维持裸文案 + `sqlstate: None` 逐字节不变（InsertSelect 宽高错误 String 臂原样保留）。`materialize_insert_select_source` 返回类型 String → `StorageError`，三处源行物化（InsertSelect ×2、CTAS、DerivedScan 臂）均为「逐行入账、峰值即全部源行、物化完成整段出账」——与 design D4「函数返回前释放」原文一致。INSERT SELECT 超限在目标 begin 之前返回（零写入由测试断言）。

执行器计费独立验证：Sort 全量逐行入账、有界替换先出账被逐出行再入账新行（行内容自入账后不变，重算同值）；Join/NestedLoop 右侧经单源助手、左侧只计当前探测行；Distinct 幸存行 2×（输出 + seen 副本）、重复行瞬态克隆不计（`contains` 前置判断，产出语义与原 `insert` 返回值等价）；Aggregate 新组键入账 + 状态更新前后差量入账/出账（`limit == 0` 跳过度量保持原路径零开销）；Semi/Anti 独立路径单次建表、相关路径换表按记账字节出账旧表再入账新表 + 缓存键值入账（anti 与 semi 镜像，diff 结构一致）；SetOperation 分支行入账、重复/未幸存行出账。

CLI 三点位独立验证：`execute_command` 顶部打开前校验（exit 2 零副作用）、主 mode open 后应用、`attach_database` 附加库应用——与水位通道同构；解析函数对未设置/`0`/负数/溢出四形态语义正确，文案点名变量与回显非法值。

测试独立阅读：12 条与 Task Contract 名称和断言逐字对应——非法 env 三形态 × exit 2 + 变量名 + 回显 + 双路径零库文件；直接扫描 16 字节上限 4 行全返回（多行表，锁 R3 场景）；派生表/右表 JOIN/无界排序/DISTINCT LIMIT 1 小上限失败；小右大左 JOIN 与有界排序小上限成功（分别消费 Iteration 001/002 成果）；三入口 `assert_over_limit_face` 精确断言消息全文与 `Some("53200")`；不继承计数（超限残留 96B 判别 + 同上限连续两次小排序）；INSERT SELECT 零写入且同错误面。字节数算术核对（32B/行、右表 48B、派生 128B 等）全部成立。

四项偏差逐条裁定：

1. `Arc<AtomicU64>`：本工具链 rustc 1.99.1 实测 `AtomicU64` 解析为 `Atomic<u64>` 且无 `Clone` 方法（独立编译验证），Plan Context「AtomicU64 克隆取值快照」在本工具链不可编译——Plan 错误，Act 修正成立；克隆共享语义沿 `schema_version` 先例，无 spec 场景区分两种共享形态。非实质。
2. 4.1/4.2 边界重划：无消费方的构造无法免警告编译，语句入口构造与穿线移入 4.2 与首批 charge 一并落地；4.1 的 GREEN 条件（默认面 + 双通道 + 短路单测）全部满足。任务内时序调整，非实质。
3. `QueryMemoryExceeded` 变体随 4.1 引入：`charge` 完整语义需要它，4.3 只做分类接线。非实质。
4. `ScanExecutor.results` 不计费：spec R3 场景「直接全表扫描不因结果大小被拒绝」（多行表 16 字节成功）与 R5 场景「小右表加大左表」（JOIN 左腿即 Scan 物化，若计费该场景必失败）共同唯一确定不计费——**spec 场景与 design D5 清单该项冲突，spec 为行为权威，Act 裁定方向正确**。design.md D5 该行已按裁定加注。

沿用 Act 验证结论（覆盖未失效，Act 后工作区仅本轮 Review 前的文档更新）：`--test query_memory_test` 12/0；`--lib query_budget` 4/0；定向 16 文件 259/0；全量 M19 14 批 **2144 passed / 0 failed / 2 ignored** = 2128 + 12 + 4 对账吻合、129 文件 = 128 + 1 全覆盖；`cargo fmt --check` 独立复核全库零 Diff（含 data_scan.rs:683 起 Iteration 000 测试代码——Iteration 001 Review 记录的格式遗留已随 Act 的 fmt 清除，全量回归在 fmt 之后运行，改前改后均 GREEN，待授权轻量项就此了结）。

Minor findings（不阻塞，无需修复）：

- clippy 聚合 13 → 14：+1 为 `double_must_use`（executor_trait.rs:9，`#[async_trait]` trait 面默认方法展开）——源自本 change Iteration 000 已 accepted 的 `next_batch` 与 clippy 1.99 的相互作用，非本轮触碰文件引入；lint 级噪音，留收尾清理候选。
- SetOperation Intersect/Except 的右集副本在分支结束后未出账（保守多计至语句结束，语句结束随预算丢弃）——不放宽任何超限判定方向，无 spec 场景可区分，仅记录。

**Acceptance Gaps**

无。四项 Acceptance 全部满足：默认面 + 非法 env exit 2 零库文件（4.1）；16 字节直接扫描成功 + 五形状按 spec 成败 + JOIN/有界排序内存收益成立（4.2）；三入口同消息同 53200 + 不继承 + 零写入（4.3）；14 批 0 failed 且 2144 对账（4.4）。

**Evidence**

- `git diff` 全部触碰文件阅读；`grep -n "53200\|executor_creation_error\|storage_error_sqlstate" src/pipeline.rs`（七消费点 + 漏斗）
- `src/executor/query_budget.rs`（charge 回退/release/口径/桶助手）；`src/executor/sort.rs`（有界替换出入账）；`src/executor/semi_join.rs`/`anti_join.rs`（换表出账 + 缓存入账，镜像一致）
- `tests/query_memory_test.rs` 12 条逐条阅读，字节数算术核对
- rustc 1.99.1 独立编译验证 `AtomicU64` 无 `Clone`；`cargo fmt --check` 全库零 Diff
- Act Response 的定向、`--lib query_budget`、clippy 与全量 14 批结论（沿用，来源已注明）

**Follow-up Decision**

accepted。四项偏差中 1、2、3 为非实质（其中 1 为 Act 修正 Plan 的工具链事实错误，4 为 Act 按 spec 权威裁定 design 清单冲突，方向均正确）。计费对称性、错误面单源、零写入时序与语句隔离均有独立代码证据与测试锁定。无当前 Cycle 修复要求。四轮 Iteration 全部 accepted，change 的 12 个 task 全部完成。

**Iteration Plan Update**

无（design.md D5 的 `ScanExecutor.results` 行按偏差 4 裁定加注，属决策记录校准，非范围变化）。

**Next Cycle**

None

**Next Iteration**

None——四 Iteration 全部 accepted，change 无剩余 Iteration。收尾（规格合并、归档、状态同步）待用户调用 `openspec-docs-maintainer`。
