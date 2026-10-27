# 索引键评估与字符串二级索引任务

对应路线：MS38；来源 I024、I085 前半。需求范围已于 2026-10-27 经用户确认；随后用户回复「批准」，详细计划审计通过。本文件不更新全局里程碑状态。

## 当前批次状态

- 需求与 BDD：completed，Gate 1 PASS。
- 000：accepted。T1–T3 完成。依据是 `iterations/000-key-encoding-evaluation/000-initial.md` 的 Review Result = accepted。该审查的非阻塞发现第 1 条仍留在原处：审查当时没有改本文件。
- 布局选择：保留现有 32 字节固定键，采用 fixed32。批准来自本次执行 goal，不是跳过。
- 001：accepted。T4、T5 完成。依据是 `iterations/001-fixed32-string-index-lifecycle/000-initial.md` 的 Review Result = accepted。
- 002：accepted。T6 完成。依据是 `iterations/002-sql-declaration-and-equality/000-initial.md` 的 Review Result = accepted。该审查的两条非阻塞发现留在原处，不在本文件复制。定向验证是 `string_index_sql_test` 8 passed / 0 failed，`secondary_index_test` 27 passed / 0 failed，`string_secondary_index_test` 6 passed / 0 failed。
- 003：当前可执行轮次。000 的 Review Result 为 replan-required。执行依据是 `iterations/003-fault-boundary-and-closeout/001-replan.md`，Plan Context 为 ready，Gate 2 PASS。批准来自用户原话「这个后续解决，直接收尾吧」，不是跳过。T7 已完成。T8 的状态见任务表。全局 fmt 退出码 1、deny clippy 退出码 101 留在 000 的 Blocker Handoff，不再作为本变更的完成条件。ISS36 保持 open。
- Persisted Evidence：none。无跳过的必需验证。

## 全部任务与归属

下表是完整工作分配。003 是当前可执行轮次。每项只归属一轮；详细执行依据在该轮 Cycle。

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | 先行测试定义整数保序、完整字符串、恢复往返和旧布局刻画；实现仅测试使用的三种编码原型；新测试 RED→GREEN | completed |
| T2 | 000 | T1 | 复用 SlottedPage/BufferPool/真实存储，覆盖完整装页回读、容量边界与页请求/读取计数；先行测试 RED→GREEN，直接观察实际页与值 | completed |
| T3 | 000 | T2 | Criterion 三方案页级读写测量与页数/缓存数据，给出筛选结论和能力边界，满足首轮 Acceptance；不得据此切生产布局 | completed |
| T4 | 001 | T3 accepted + 用户作出编码选择 | 实际树层的格式方案与回归；fixed32 分支保持条目格式；候选分支以实际树收益与兼容合同再次通过采用门 | completed |
| T5 | 001 | T4 | 二级专属字符串键派生、回填与所有 DML 维护/回滚/恢复闭合，现有 PK/唯一语义保持；没有路由也可从数据页验证完整行 | completed |
| T6 | 002 | T5 accepted | CREATE INDEX 字符串类型开放、类型匹配等值路由、EXPLAIN 与输出形状、物理容量外完整值扫描、建后删后等价 | completed |
| T7 | 003 | T6 accepted | 取消/超时、失败、干净重开与未决 WAL 恢复，旧库/内存/只读/加密/附加 DML 及 DEFAULT/overflow 的可达组合；直接库或 CLI 入口验证 | completed |
| T8 | 003 | T7 | 收尾一次 M19 分批全量，受影响格式/lint 与 OpenSpec 检查，用户文档同步实际类型与长值回落行为。全局 fmt/clippy 失败留在 003/000 的 Blocker Handoff，不再作为完成条件 | completed |

T1–T3 的执行依据在 `iterations/000-key-encoding-evaluation/000-initial.md`，该 Cycle 已 accepted。T4、T5 的执行依据在 `iterations/001-fixed32-string-index-lifecycle/000-initial.md`，Review Result 为 accepted。T6 的执行依据在 `iterations/002-sql-declaration-and-equality/000-initial.md`，Review Result 为 accepted。T7 已完成，执行依据在 `iterations/003-fault-boundary-and-closeout/000-initial.md`，Review Result 为 replan-required。T8 的修订验收在 `iterations/003-fault-boundary-and-closeout/001-replan.md`。T8 的状态见任务表。

## Iteration Plan

### Iteration 000：编码正确性与真实页测量

- Tasks：T1、T2、T3。
- Depends on：None。Cycle `iterations/000-key-encoding-evaluation/000-initial.md` 的 Review Result 为 accepted。
- Stable result：三方案的可重跑原型、支持域、页数/缓存读数/读写吞吐与有边界的推荐。生产行为不变。结果足以让用户决定下一轮调查采用哪个方案，不能宣称已交付字符串 SQL。
- Verification boundary：候选测试、既有 Key 与二级索引回归、指定 Criterion；不跑全量。页级测试经真实 SlottedPage/FileStorage/BufferPool API，不伪称 SQL 端到端。
- Diagnostic boundary：候选编码、测试页记录布局与原型 IO 负载；固定 Key/产品 B-Tree 的已知限制不在本轮修改。
- Non-goals：产品接口、持久格式、全局 to_key、SQL 守卫、第二套 B-Tree、生产采用。
- 平衡审计：编码、页负载与测量共用一个候选原型，拆开不能形成可信采用输入；SQL 与恢复是另一结果，留后续轮次。本轮不承载整个里程碑。
- 停止边界：已到达。accepted Review 没有展开 001。布局选择由后续 goal 关闭，见 Iteration 001。

### Iteration 001：选定树层方案与字符串索引生命周期

- Tasks：T4、T5。
- Depends on：000 accepted。布局选择已关闭：保留现有 32 字节固定键，采用 fixed32。执行依据是 `iterations/001-fixed32-string-index-lifecycle/000-initial.md`，Gate 2 PASS。
- Stable result：生产存储层可建立、维护和恢复正确字符串索引，旧库与既有索引保持；SQL 声明和路由仍由下一轮开放。
- Verification boundary：实际 B-Tree 插删查、多层树、重复键、页级判界、失败写入、回滚、恢复与旧库定向验证。本轮采用 fixed32，不补变长或前缀方案的实际树测量，也不重跑已完成的 Criterion。不跑全量。
- Diagnostic boundary：二级键派生、现有树条目、catalog/root 与索引生命周期。本轮不接线 ordered-variable 或 prefix-restart。如需改变已批准的长值回落合同，返回需求与设计。
- Non-goals：字符串 PK/UNIQUE、查询范围路由、组合索引、SQL 新语法。
- 平衡审计：树存储与写入恢复必须在同一完整键语义下验收；把只有前向写而无撤销的半成品分轮会失去稳定结果。fixed32 不改变文件格式，没有独立迁移结果，Map 不调整。002、003 仍只留在本文件，本轮不创建它们的目录。

### Iteration 002：SQL 声明与等值查询可达

- Tasks：T6。
- Depends on：001 accepted。执行依据是 `iterations/002-sql-declaration-and-equality/000-initial.md`，Review Result 为 accepted。该审查的两条非阻塞发现留在该 Cycle，不在本文件复制。
- Stable result：用户可创建字符串二级索引；匹配类型、物理可容纳的裸列字面量等值查询走树，超容量完整值查询扫描；建前、建后、删后结果与列元数据严格一致。
- Verification boundary：Database 真入口与 CLI 的 TEXT/VARCHAR、重复键跨多叶、双向等值、NULL/空串/UTF-8/尾零/共享前缀；投影、聚合、DEFAULT 与派生消费；原有修饰拒绝、只读和事务错误面；参数/组合/范围等未扩展形式维持扫描。不跑全量。
- Diagnostic boundary：ddl_dml/query 计划臂、IndexScanAll 形状与 Pipeline 构造；不在这一轮重设存储格式。
- Non-goals：已交付的树算法优化、完整代价模型、字符串唯一约束。
- 平衡审计：DDL 与查询组成用户可用能力，共同验收后形成稳定检查点；只有一个 Task 不代表机械拆分，范围含声明和消费配对及多种输出路径。

### Iteration 003：故障边界、运行形态与收尾

- Tasks：T7、T8。
- Depends on：002 accepted。T7 的执行依据是 `iterations/003-fault-boundary-and-closeout/000-initial.md`，该 Cycle 的 Review Result 为 replan-required。T8 的修订验收在同目录 `001-replan.md`，Plan Context 为 ready，Gate 2 PASS。批准来自用户原话「这个后续解决，直接收尾吧」，不是跳过。这是本变更最后一轮。不把 ISS36 另立一轮。
- Stable result：字符串能力在失败、放弃、恢复、受支持运行形态和旧库上有直接运行证据，用户文档与实际行为一致。全局 fmt/clippy 的既有失败留在 000 的 Blocker Handoff，不阻止本变更进入最终 Review。
- Verification boundary：T7 定向结果、13 批全量、两份 README 和 OpenSpec 检查已经记录，不重跑。当时的全量规则是：目标为 `--lib`、`--bins` 与全部 `tests/*.rs` 文件名（不含扩展名），按 C 本地字典序排列。批数 `B = ceil(N / 12)`，前 `B * 12 - N` 批各 11 个目标，其余各 12 个。批间串行，每批 `cargo test --offline` 加上该批的 `--lib` / `--bins` / `--test <名>`，末尾 `-- --test-threads=1`。现场是 `N=156`、`B=13`、每批 12 个目标，每批 failed 为 0。本变更触碰文件的格式和 lint 也已完成。全局 `cargo fmt --check` 退出码 1，`cargo clippy --offline --all-targets -- -D warnings` 退出码 101，原文在 000 的 Blocker Handoff。这两条失败不是本轮完成条件。
- Diagnostic boundary：跨入口接线、已有取消/恢复/释放约定的组合缺口及收尾回归。新故障域或范围外实质缺陷只报告候选，不夹带修复。
- 收尾边界：用户已决定不在本变更处理 ISS36。不修这两条命令，不清基线，不把 ISS36 标成已关闭。这两条失败不再是 T8 保持未完成的理由。任务表里的 T8 在 Act 写入前为 blocked，写入后为 completed。
- Non-goals：基于结果顺便进行性能优化、框架外验收脚本或身份归属工具。全局 fmt/clippy 修复。关闭 ISS36。
- 平衡审计：故障边界和一次收尾仍是同一轮。replan 只改完成条件，不增加产品任务。把状态记录拆成新 Iteration 不会形成新的产品结果，因此不拆。ISS36 不进入本 Map。

## 需求追踪

| Requirement | Task | First Test | Status |
|---|---|---|---|
| R1 编码评估 | T1–T3、T4 现有树回归 | `integer_candidate_preserves_signed_order`、`page_metrics_cover_equal_inputs` | Covered（000 已测；001 不重跑 Criterion，T4 回归现有 38/36 条目） |
| R2 单列字符串索引 | T4–T6 | `string_index_backfills_duplicates_null_and_utf8` | Covered（001 做回填与容量；SQL 声明在 T6） |
| R3 等值查询 | T6 | `string_index_equality_matches_scan_multiset_and_columns` | Covered（含容量外扫描；不扩大语法） |
| R4 生命周期一致性 | T5、T7 | `string_index_rekey_null_rollback_and_failed_write`、`cancelled_string_index_write_is_reclaimed` | Covered（共用二级派生、回滚源与原子写单元） |
| R5 持久化与兼容 | T4、T5、T7、T8 | `string_index_clean_reopen_and_pending_wal_agree`、`legacy_db_accepts_string_index_without_losing_int_data` | Covered（001 覆盖重开、未决 WAL 和 legacy_g1；其余运行形态在 T7） |
| R6 存储安全回归 | T2、T4、T7、T8 | `candidate_page_roundtrip_checks_lengths_and_capacity`、既有 `index_read_bounded_read_test` | Covered（fixed32 不改条目宽度，T4 用现有判界回归） |

Covered 表示范围、场景和计划验证均已分配，不代表实现或测试通过。没有 Simplified；固定布局长值扫描是 proposal 已批准的「支持域外明确回落」设计具体化，不限制表字段合法长度。如用户要求超容量字符串也必须走索引，须先修订需求与本 Map。

## 决策与交接

- 000 已 accepted。T1–T3 完成状态以该 Cycle 的 accepted Review 为依据；审查正文里的任务状态偏离不删除。
- 布局选择：保留现有 32 字节固定键，采用 fixed32。两种候选的热读没有同时守住已批准的 5% 吞吐线；页数和写、冷读收益不能单独切换生产格式。不采用 ordered-variable 或 prefix-restart，不降低筛选门槛，不重跑 Criterion。
- 001 已 accepted。T4、T5 不再交给 Act。002 已 accepted。T6 不再交给 Act。两条非阻塞发现留在 002 的 Plan Review。
- 003 已展开。`000-initial.md` 的 Plan Context 保持 ready，Review Result 为 replan-required。T8 的执行依据改为 `iterations/003-fault-boundary-and-closeout/001-replan.md`，Plan Context 为 ready。批准来自用户原话「这个后续解决，直接收尾吧」，不是跳过。本次规划不调用 Act，不改全局里程碑、SNAPSHOT 或主规格，不关闭 ISS36。证据为 none。T8 的完成标记由该 Cycle 的 Act 写入。
