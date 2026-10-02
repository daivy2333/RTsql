# Iteration 000 / Cycle 000：附加库普通写入与生命周期

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- 所属 Iteration：000-initial。
- Change tasks：T1、T2、T3（tasks 1.1–1.3）。
- Acceptance gaps：None。
- Repair items：None。
- Inherited scope：获批 R1–R5/R8、S1–S7/S9；普通附加库写入及主库兼容性。
- Excluded scope：R7 INSERT SELECT 与 R6 最终文档在下一逻辑迭代；不预建后续目录，不实施新语法或附加 DDL。

**Objective**

附加库 VALUES/UPSERT/REPLACE、主键等值 UPDATE/DELETE 独立提交；事务、WAL、索引与失败回滚始终归属目标；成功、fail-fast 和信号取消都关闭已注册附加实例；主库显式事务不被当作多库事务。

**Investigation Facts**

- Current Baseline：2026-10-02，master HEAD `4e1944f`，工作区已有 MS28/MS29 产品、文档和规格未提交修改。该现场只用于诊断，不作为匹配/冻结或验证准入字段。Plan 未修改产品代码。用户批准需求原话「批准，记得遵守各类规范」，三项推荐范围均获批；同日完整计划批准原话「批准」。
- 当前行为：ATTACH 在 CLI 注册表中存 Arc<Database>，普通可写会话以 RR + 主库相同 key/lock_wait 打开，只读会话用共享锁。Database 不存 attach 状态；lib 执行路由默认空。ObjectName 保留分段进行别名判断，注册失败不改表；裸名恒主库、未命中别名沿扁平名。
- 已有验证：本轮真实执行下方 8 目标定向命令，133 passed/0 failed，exit 0；不是全量声明。覆盖代码在结果产生后未修改，直接采信，不再重复基线。

```bash
cargo test --test attach_session_test --test cross_db_query_test --test ctas_test --test subset_insert_test --test upsert_test --test tx_statement_test --test readonly_session_test --test memory_db_test -- --test-threads=1
```

决定性原生输出（按执行目标顺序，8 行）：

```text
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.58s
test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.35s
test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.48s
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.33s
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.75s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.30s
test result: ok. 32 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.59s
```

对应目标：attach_session、cross_db_query、ctas、memory_db、readonly_session、subset_insert、tx_statement、upsert。

原生命令拒绝观察（各 exit 3，非测试成功声明）：

```bash
target/debug/rtsql :memory: "CREATE TABLE t (id INT PRIMARY KEY, v INT); INSERT INTO t VALUES (1, 2); UPDATE t SET v = (SELECT 9) WHERE id = 1"
target/debug/rtsql :memory: "CREATE TABLE t (id INT PRIMARY KEY, v INT); INSERT INTO t SELECT 1, 2"
target/debug/rtsql :memory: "CREATE TABLE t (id INT PRIMARY KEY, v INT); INSERT INTO t VALUES (1, 2); DELETE FROM t WHERE id IN (SELECT 1)"
```

```text
UPDATE: Plan error: Unsupported value type
INSERT SELECT: Plan error: Unsupported statement type: INSERT ... SELECT
DELETE: Plan error: Parse error: DELETE requires primary key equality in WHERE clause
```

CLI 由以上 cargo test 构建；三条内存输入不创建数据库文件。UPDATE/DELETE 无已支持子查询形态，因此保留原拒绝面是 R8 的完整落实，不是需求简化。

**Current-State Evidence / Code and Critical Path**

| 文件与符号 | 实际入口、数据与责任 |
|---|---|
| cli/mod.rs::execute_main_command → execute_command_inner_mode | mode helper open 与 work 两阶段 select；信号构造先于 open。主库 close 在 select 外；work 工厂私有，目前只借 Database |
| cli/mod.rs::run_sql → attach_database / AttachSession | run_sql 局部创建 registry 与 TransactionSession；遍历 AST；普通语句创建 AttachRoute、RoutedTables，调用 plan_stage_routed，再按 session.tx_id 分发 execute 两叶子；所有错误 return 前 rollback_session 主库。正常尾忽略附加 close 错误，fail-fast/取消绕过尾部 |
| cli/lifecycle.rs::import_csv | 唯一生命周期直接调用 execute_command_inner_mode（内存 import）；其他调用通过 execute_command_inner 委托壳。不涉及附加，只需兼容私有参数适配 |
| pipeline.rs::register_table/register_table_schema | 根据 ObjectName.parts 判附加；用附加本地名取 TableMeta，把限定名 schema/PK/类型/defaults/unique/secondary 信息注册给 PlanBuilder；routed[限定名]={db,local_name}；不重解析计划扁平串 |
| ddl_dml.rs::build_insert/build_update/build_delete | INSERT validate target/view 后 reject_attached_write、extract_insert_values/map_insert_values/build_upsert_action；UPDATE/DELETE reject 后 extract_pk_from_where；UPDATE 单赋值且仅字面量/日期族/NULL。移除仅三处 DML 附加拒绝，不删除 DDL/CTAS 或附加标记 |
| query.rs::extract_pk_from_where | 只接受裸 pk 标识符与字面量直接等值，反向同款；其他条件返回 None，没有子查询编译或部分谓词执行。UPDATE 缺键最终 KeyNotFound；DELETE 缺键 affected 0 |
| pipeline.rs::execute_stage_routed | readonly_rejection 主库前置。Insert/Upsert/Update/Delete 根 begin 主库；abort_tables 原计划表名查主库；create_executor_from_plan；错误 abort，成功 Database::commit。需要整条资源归属切换 |
| pipeline.rs::execute_stage_in_tx_routed | readonly 守卫后普通写使用传入主库 tx_id，目前无附加目标专用拒绝。必须在构造执行器之前加入目标拒绝 |
| pipeline.rs::routed_db/load_table_meta/create_executor_from_plan | 现有 helper 返回附加 Database/local_name 或主库/原名。读扫描已路由；四个写臂仍直接用主库 table_manager/buffer_pool/transaction_manager/wal_buffer；Insert/Upsert 还传 TableManager |
| executor/{insert,upsert,update,delete}.rs | WAL table_name 和 record_version 键取 TableMeta.name。NOT NULL/类型门/unique/secondary 路径已有；不复制，不改 metadata 为 alias.table |
| database.rs::commit/maybe_auto_checkpoint/close | commit 后水位检查单点，失败仍已提交；close=checkpoint，无工作跳过；只读/内存 no-op。附加目前默认水位，env 仅主库模式入口应用 |
| transaction/manager.rs::abort；Database::rollback | abort 使用本地表名与目标 buffer_pool 还原已记录版本及索引；不得混入主库 metadata 或主库 tx_id |

测试入口：cross_db_query_test 的两条一期 DML 拒绝用例需迁移；保留全部读/名字/DDL 用例。readonly_session_test::r5_readonly_session_attaches_readonly 可能消费旧拒绝文案，校准为 25006。upsert_test::crash_recovery_keeps_upsert_state 和 secondary_index_test 的恢复夹具采用 schema checkpoint、WAL shutdown、drop 不 close、重开。cli/mod.rs tests::phase2_signal_runs_close_and_returns_signaled 已有 Notify 注入信号工厂，无需新产品开关。busy_timeout_test 含 ATTACH 预算及信号等待测试。

**Implementation Guidance**

先为普通路由和目标归属建立 RED，再切换根事务及写执行器资源；随后建立退出路径 RED，将注册表所有权移到取消边界外；最后验证显式事务、readonly、DDL 和恢复组合。源查询默认库必须保留为主库，以供下轮使用。私有辅助函数命名、拆分和等价控制流不固定。

**Task Contracts**

### T1：附加库普通 DML 路由与独立提交

- Requirement/Scenario：R1/R2/R4，S1/S2/S3/S6；设计选择 D1。
- Depends on：None。
- Targets：ddl_dml.rs::build_insert/build_update/build_delete；pipeline.rs::execute_stage_routed/create_executor_from_plan/routed_db/load_table_meta；tests/cross_db_query_test.rs；新增 tests/cross_db_dml_test.rs。
- Current behavior：三处 planner 附加拒绝；执行根及四写构造臂全主库归属。
- Required behavior：仅移除三处 DML 拒绝；DML 根从 routed 选择目标，begin/abort/commit 全目标；abort map 用本地 metadata 名；四写构造臂取目标本地 metadata 和全部目标资源。主库同名与字面含点 decoy 不变。每条独立提交，后续失败不能撤销此前主库或任一附加成功语句。复用默认值、类型门、冲突仲裁及索引维护。
- Preserve：主库执行、裸名/扁平名回落、attached 标记与 DDL/CTAS/view 拒绝、输出计数和 fail-fast 状态；资源归属不能以字符串 split 推断。
- Forbidden：改写 SQL、改写 TableMeta.name、复制执行器/事务算法、跨文件事务协调、存储格式/依赖变更。
- Test witness：先建 cross_db_dml_test::attached_values_upsert_update_delete_route_and_reopen（同名主库 decoy、附加 VALUES/UPSERT/REPLACE/rekey/DELETE 与主库独立查验），::attached_defaults_constraints_and_indexes（子集/default、INT/Float/Date/NULL、NOT NULL/UNIQUE 和二级索引），::independent_file_commits_survive_later_error（主库+两个附加先成功、第三语句失败）。`cargo test --test cross_db_dml_test -- --test-threads=1` 在旧 planner 上应因附加写拒绝 RED。
- GREEN condition：以上用例正确行/受影响数、失败语句目标零残留（含多行首行已写后后行失败）、此前提交保留，主库 decoy 不变。一期拒绝用例改为对应成功断言；原零副作用拒绝测试改用仍拒绝 DDL，不删除目标不误写断言。
- Verification：T1 定向同命令 GREEN；`cargo test --test cross_db_query_test --test subset_insert_test --test upsert_test --test write_type_conformance_test --test update_index_maintenance_test --test secondary_index_test -- --test-threads=1` 全部 0 failed。恢复专项属于 T3，未完成 T3 不宣称当前迭代完成。
- Stop when：发现路由需要更改元数据本地名、单库索引/回滚算法或名字语义，或现有已支持语句无法复用；返回 Plan。

### T2：所有退出路径关闭附加库

- Requirement/Scenario：R5，S3/S7；设计选择 D3。
- Depends on：T1。
- Targets：cli/mod.rs::execute_main_command/execute_command_inner/execute_command_inner_mode/run_sql/attach_database/AttachSession 与文件内 tests；cli/lifecycle.rs::import_csv 单个 mode 调用适配；cross_db_dml_test。
- Current behavior：registry 属 run_sql work；只有正常尾尽力 close，错误与信号取消绕过；主库水位 env 在 mode 入口应用，附加默认水位。
- Required behavior：AttachSession 由阶段 2 select 外编排器持有，work 借用；run_sql 使用该表，无自己的关闭尾。私有 mode work 传可变注册表借用，委托壳适配原单参数 closure，直接 import mode 调用忽略新参数。work 正常结束/错误/被信号取消后，逐个 await 所有已注册库 close，再保留原主库 close 路径；任何一次失败不能跳过后续关闭。成功遇关闭错误→exit 1，点名别名；已有执行/锁/密钥错误或信号状态保持，关闭错误 stderr 记录。打开成功的附加库复用既有合法 checkpoint env 与设置器，commit 在目标库检查水位。
- Preserve：130/143、注册失败零副作用、budget/key/memory/readonly、主库 close 错误优先级、生命周期命令原业务；取消期间未成功注册 open 仍走原机制。不得为异步 close 新建 Drop/spawn 协议。
- Forbidden：持 std Mutex 跨 await、复制 open/signal 编排、独立清理线程或异步 Drop、新增 CLI/测试专用参数、run-id/指纹/判定器。
- Test witness：T1 GREEN 后建立 `attached_close_after_fail_fast`（此前附加提交、后续错误，读到提交且 WAL checkpoint 已完成、锁能释放）；文件内 `phase2_signal_closes_attached_databases` 复用 Notify 注入 SIGINT/SIGTERM，work 附加提交后挂起，退出原生 Signaled，重开数据及 WAL/锁直判；`attached_close_failure_preserves_primary_status` 用现有 checkpoint 文件路径置目录的注入形态，恢复后读取提交，校验成功→1、原 SQL/信号状态保持且每库均尝试；`attached_commit_uses_own_checkpoint_threshold` 设置小水位，目标 WAL 有界且主库不被误 checkpoint。旧局部 registry/忽略 close 形态观察 RED。
- GREEN condition：新增退出/错误优先级用例与原单库信号用例通过，全部注册实例在退出后可重开；失败无错误吞没，已提交数据保留。信号取消未提交语句维持既有单库保证，不能宣称跨库补偿。
- Verification：`cargo test --lib cli::tests -- --test-threads=1`；`cargo test --test cross_db_dml_test --test attach_session_test --test busy_timeout_test --test memory_db_test --test wal_auto_checkpoint_test -- --test-threads=1`，每次 0 failed。新测试名称在合法本地等价命名时可记录偏差。
- Stop when：取消边界导致必须改变单库未提交语义、公开会话 API 或其他生命周期业务；或需要新增产品开关才能观察目标，返回 Plan。

### T3：事务/只读/DDL/恢复与语法组合收口

- Requirement/Scenario：R1–R5/R8，S2/S4/S5/S6/S7/S9；设计选择 D2/D5。
- Depends on：T1/T2。
- Targets：pipeline.rs::execute_stage_in_tx_routed/execute_stage_routed/readonly_rejection；tests/cross_db_dml_test.rs、cross_db_query_test.rs、readonly_session_test.rs；planner 仅为定位既有拒绝面（不扩展）。
- Current behavior：执行期只读守卫已存在，附加 planner 通用拒绝曾先行；显式主库 tx_id 不具备附加目标事务身份。
- Required behavior：只读优先 25006；非显式根同时检查实际目标 readonly；显式根仅在写目标命中 routed 时 0A000 点名拒绝，发生于构造执行器/begin 之前，不因源 routed 非空误拒主库写。CLI 现有错误回滚主库 session，不回滚此前附加提交。覆盖 DDL/CTAS 仍拒绝，UPDATE 字面量单列+pk 等值、DELETE pk 等值维持，子查询赋值/条件拒绝。UPDATE 缺键 KeyNotFound、DELETE 缺键 count=0，不能统一为零行。恢复时用真实 routed pipeline 写附加库，不用单库直接写代替路由见证。
- Preserve：主库事务+附加查询/ATTACH 合法、只读共享锁及零写入、附加 DDL 点名、未注册别名与 quoted 扁平名回落、单库错误优先级。
- Forbidden：BEGIN 管多库、扩大 UPDATE/DELETE 条件或赋值、替换现有回滚/恢复算法、为测试结果建外部验证脚本。
- Test witness：`attached_dml_in_active_tx_rejected_and_main_rolled_back` 覆盖 INSERT/UPSERT/REPLACE/UPDATE/DELETE，先 BEGIN 写主库，再尝试附加目标，原 SQL exit3 及0A000（pipeline单测）点名，重开主库已回滚、附加不变；`readonly_attached_dml_rejected`（含 BEGIN）25006；`routing_names_and_zero_match` 检查名称与 UPDATE/DELETE 不同缺键行为；`update_delete_subquery_shapes_still_rejected` 同库/跨库源均拒绝；`attached_routed_writes_crash_recovery` schema+二级索引先 checkpoint，真实 routed DML/失败回滚后目标 WAL shutdown、drop 不 close→重开，查询行/PK/唯一占用/二级路由一致；`attached_commit_checkpoint_failure_preserves_data` 复用 checkpoint-site 置目录，目标 commit 已完成、错误点名，修复夹具后 close/重开可读。新拒绝策略应先 RED；原 DDL/语法拒绝与恢复夹具未改覆盖面可先采信 GREEN 后保持。
- GREEN condition：新边界/恢复矩阵全部通过；附加正确读写且显式/只读失败不误写；新增实现与现行能力拒绝一致。
- Verification：`cargo test --test cross_db_dml_test --test cross_db_query_test --test ctas_test --test tx_statement_test --test readonly_session_test --test attach_session_test -- --test-threads=1`；`cargo fmt --check` 与 `cargo clippy --lib --bin rtsql --tests -- -D warnings`。clippy 是编译/lint，无并发测试负载；若已有无关 lint，仅记录具体基线并返回 Plan，不顺手清理。已在 T1/T2 通过且覆盖未变的验证直接采信，不再次运行。
- Stop when：当前 Acceptance 需要扩展单库拒绝语义、附加视图支持或多库事务；实质范围问题返回 Plan。相同问题连续三次失败按 Gate5 停止。

**Invariants**

- AST ObjectName 的实际路由判定为唯一依据，不能 split 平坦字符串；写记录用目标 metadata 本地表名。
- 每库事务/WAL/缓冲池/索引归属一致，DataScan 墓碑判断仍使用自己的 manager；不协调 tx_id/快照。
- 复用现有实现优先；不改存储格式、不加依赖、不重构公共 API、不清理用户无关修改。
- 提交后的 checkpoint 失败保持数据，不执行 abort 伪装未提交；先前文件提交不会被其他文件失败撤销。
- 每项任务建立 RED 或合法未失效 GREEN 见证后修改；完成后先 spec review 再 code review，Gate3 结果写 Act Response。

**Non-goals**

INSERT SELECT、新 UPDATE/DELETE 子查询、附加 DDL/CTAS、DETACH/REPL/serve、原子多库提交、性能优化/bench、持久化 Evidence 与工程经验产物、全局文档维护、归档或提交。

**Acceptance**

| 条件 | 任务/测试与可观察量 |
|---|---|
| A1：普通附加写面完整，主库不误写 | T1/S1/S2 的行、affected数、DEFAULT/类型/唯一/二级索引结果、重开 |
| A2：独立提交与当前失败回滚 | T1/S3 多文件部分成功与多行当前语句失败后目标数据/索引 |
| A3：所有退出路径附加关闭 | T2/S7 的原退出码、checkpoint 原生结果、已提交行及锁释放；关闭失败有名且不吞原错误 |
| A4：显式/只读/DDL 边界 | T3/S4/S5 的0A000/25006、exit3与两库不变；主库事务+附加读取不回归 |
| A5：恢复与兼容性 | T3 恢复两态查询、名字/缺表/无命中和 UPDATE/DELETE 拒绝，既有跨库SELECT/CTAS/主库写面保持 |

当前迭代 RTM：R1→T1/T3、R2→T1/T3、R3→T3、R4→T1/T3、R5→T2/T3、R8→T3，全部 Covered。R6/R7 按 Map 后续实施，不在本轮裁剪或提前标完成。

**Verification**

按 Task Contract 的直接目标→受影响边界→fmt/clippy 顺序；同一覆盖通过且未改变便采信。判断仅来自 Rust 测试原生输出及退出码，无附加判定层。新增每用例独立 TempDir；CLI 显式 `--format json`、隔离 RTSQL_HOME，输出与 exit 一起检查。不操作用户数据；加密附加同 key 成功/错 key 既有拒绝，内存主库写文件附加组合纳入 T3。

本轮不跑全量，最终迭代依 M19 明确目标分批、每批串行，只在全部批各自0失败后宣称全量。不得以重复已通过验证增加信心，不保完整日志或源码副本。

**Gate 2 Readiness**

PASS：技术调查、设计、任务/RTM/分轮与验证契约已闭合；2026-10-02 用户批准完整计划，原话「批准」，Context 为 ready，可在用户下达 Act 指令后执行。Gate1 PASS。计划内没有 Missing/Simplified 或实质未知项。

**Persisted Evidence**

- Mode: none。
- 命令、最多20行决定性输出、退出码、修改符号和 Gate 结论写 Act Response；测试可低成本重跑，未存在需要保留的一次性环境。不创建 evidence/。

**Risks and Notes**

- 用户已批准各文件独立提交与显式事务限制；部分成功是目标行为。
- closure 可变借用的局部实现选择由 Act 处理，不引入异步 Drop。私有单参数委托壳保留生命周期接口。
- 代码与旧规格「计划期只读」术语不一致，当前叶子前置守卫保证任何源求值/写入前拒绝；该事实已定位，不让 Act 决定错误语义。
- 本轮结束停在 Act Response，待 Plan 独立完整 diff Review；不能创建下一轮、维护全局状态或自动归档。

## Act Response

- Status: reported
- Cycle：Iteration 000 / Cycle 000（initial）。Tasks：T1、T2、T3 全部完成；Plan Review F1–F3 见证缺口已在当前 Cycle 修复（2026-10-02，Follow-up Decision 授权的有限修复）。
- 本 Response 为包含原实施与最新修复的完整当前状态。

### 实际改动

| 文件 | 符号 | 改动 |
|---|---|---|
| `src/parser/planner/ddl_dml.rs` | `build_insert` / `build_update` / `build_delete` | 移除三处 `reject_attached_write`（附加 DML 计划期拒绝）；DDL/CTAS/view 的 `reject_attached_write` 与 `attached_tables` 标记保持（原实施） |
| `src/pipeline.rs` | `create_executor_from_plan` Insert/Upsert/Update/Delete 四臂 | 经 `routed_db`/`load_table_meta` 取目标库 TableManager/BufferPool/TransactionManager/WAL 与 catalog 本地表名（主库未命中回落逐字节不变）（原实施） |
| `src/pipeline.rs` | `execute_stage_routed` DML 根 | `target_name` 取自有 String；目标经 `routed_db` 选定；begin/abort/commit 与 `abort_tables`（本地名键）全归目标库；非显式根增目标只读检查（原实施） |
| `src/pipeline.rs` | `attached_write_in_tx_rejection`（新）+ `execute_stage_in_tx_routed` | 显式事务内写节点目标命中路由表即 0A000 点名拒绝，先于执行器构造/源求值/begin；只判目标不误拒「主库目标+附加源」（原实施） |
| `src/pipeline.rs` | `AttachRoute` / `load_table_meta` / `create_executor_from_plan` 文档注释 | 三处失效表述校准（「一期只读」「写执行器不路由/写面计划期拒绝」→ MS20-T02 写面路由事实；本轮 F3 Minor，仅注释零行为变化） |
| `src/cli/mod.rs` | `execute_command_inner_mode` | 持有 `AttachSession`（取消边界外），work 工厂改双参借用；`close_attached_databases` 在 select 后逐个关闭（全部尝试），首个失败按原状态映射退出码（原实施） |
| `src/cli/mod.rs` | `execute_command_inner` / `execute_main_command` / `run_sql` / `attach_database` | 委托壳适配单参 work；主命令闭包传会话；`run_sql` 只借用会话、删除本地注册表与尾部 close；附加库打开后应用 checkpoint 水位 env（原实施） |
| `src/cli/lifecycle.rs` | `import_csv` 的 mode 调用 | 闭包适配双参（忽略会话）（原实施） |
| `tests/cross_db_dml_test.rs` | 16 用例（原 11 + 本轮 +5） | 本轮新增：`attached_multi_row_failure_rolls_back_target`（R1 多行 INSERT 前行已写、后行 NOT NULL 失败 → 目标零残留、键 id/唯一值可重用、二级索引重建条目路由、主库与此前提交保持）、`attached_commit_checkpoint_failure_preserves_data`（R5 提交边界 checkpoint 失败 → Error 点名 `Commit failed`/`checkpoint failed`，已提交行/唯一占用/二级索引保持、主库不变、修复现场重开可读）、`routed_write_rejections_carry_sqlstate`（显式事务附加写 0A000、只读 25006、只读×事务 25006 优先、主库事务+附加源 SELECT 成功保持）、`memory_primary_routes_write_to_attached_file`（`:memory:` 主库 ATTACH 写文件库）、`encrypted_attached_same_key_write_and_reopen`（`--key` 同 key 加密主库+加密附加写入重开）；`attached_values_upsert_update_delete_route_and_reopen` 补 rekey UPDATE（`SET id = 2 WHERE id = 5`）；`routing_names_and_zero_match` DELETE miss 改逐行精确断言（两行 `{"affected_rows":0}`，原 `contains("0")` 被 ATTACH 行满足）；`attached_routed_writes_crash_recovery` 重写（UPSERT/rekey/多行 abort 入见证；崩溃前后 PK 点查、唯一占用、二级索引、主库不变断言） |
| `src/cli/mod.rs` tests | 3 单测 + 2 辅助 | `phase2_signal_closes_attached_databases` 重写（work 内注册**并提交** 300 行后挂起，SIGINT/SIGTERM 双信号 `Signaled(2/15)`、附加 WAL 截断、重开行完整）；`attached_close_failure_preserves_primary_status` 重写两场景（① Success→General 点名失败别名 'a'，另一附加库 WAL 截断、主库位点写入〔非零工作 close 直接观察量〕，失败库已提交 300 行修复现场后可读；② Sql/Locked/InvalidKey/Signaled(2) 四态原退出码保持、其余库与主库仍关闭；失败由位点注入固定，断言不依赖 HashMap 遍历序）；新增 `signal_factory`（双信号工厂同型）/`seed_pending_work` 测试辅助 |

### 与计划的偏差

1. （原实施保留）`attached_commit_uses_own_checkpoint_threshold` 落在 `tests/cross_db_dml_test.rs` 而非 `src/cli/mod.rs` 文件内——契约允许合法本地等价命名/位置；语义与 T2 见证要求一致。
2. （原实施保留）非显式 DML 根增加 `readonly_rejection(target_db, …)` 目标检查（design D2「同时检查实际目标 readonly」）——契约内。
3. （原实施保留）DML 根的 `target_name` 使用 `clone()`：`plan` 随后移入执行器构造，借自 `plan` 的 `&str` 无法跨越该 move；属局部实现选择（每次 DML 一次 String 分配）。
4. （原实施保留）更新 `AttachSession` 文档注释（原「一期只读」表述随能力交付失效）。
5. （本轮 F1）恢复用例的 abort 组合边界：多行失败 abort 后**不再写入即崩溃**（aborted 事务经 redo 重放 + `mark_uncommitted_aborted` 中性化入见证）；「键可重新使用」运行期见证由 CLI 用例 `attached_multi_row_failure_rolls_back_target` 承担。原因：abort 后槽位/逻辑 id 重用 × 崩溃恢复触发范围外既有引擎缺陷（见 Experience Candidates，单库零路由复现）；Review F1 要求的见证（多行失败回滚、键重用、rekey 成功与恢复组合、恢复 UPSERT、PK/主库断言）全部落位，仅分布按缺陷边界划分。
6. （本轮 F3）`routed_write_rejections_carry_sqlstate` 夹具块作用域：`RoutedTables`/plan 持有附加库 Arc 克隆且 `Database::close()` 不释放 flock（锁随实例存活至 drop），段间重开前置块作用域——测试基建事实，无产品代码影响。
7. （本轮 Minor）Review 点名两处注释外，同族失效句 `create_executor_from_plan` 文档「写执行器臂不路由（附加库写面在计划期已被拒绝）」一并校准（同一错误事实，随当前修复只改失效句）。

### 自检发现与修复

- （原实施修复保留）附加库 close 失败首版用提前 `return ExitStatus::General`，会跳过主库 close——改为原地更新 `status` 后再走主库 close。
- （原实施修复保留）`attached_routed_writes_crash_recovery` 夹具中 `AttachRoute` 仍持附加库 Arc 克隆，重开报 `DatabaseLocked`——补 `drop(route)`。
- （原实施修复保留）单测中 `Response::ok()` 笔误（`Response` 非 Result）——改 `let _ =`。
- （本轮修复）恢复用例主库 CREATE TABLE 后未 checkpoint 即写入种子行 → 崩溃重开 redo 报 `table 't' lookup failed`（DDL 不写 WAL，既有「先 checkpoint 落盘 schema」夹具纪律）——补 `main.checkpoint()`。
- （本轮修复）F3-b 第二次打开附加库报 `DatabaseLocked`——首个种子实例未 drop（close 不释放锁）+ `RoutedTables` Arc 残留——种子块作用域 + 主库段块作用域。以上均为夹具错误，非产品缺陷。
- 遗留 Minor（非阻塞）：多个附加库同时 close 失败时，退出消息取 HashMap 遍历序首个、其余落 stderr（design 允许沿注册表遍历、不新增排序协议）；本轮场景以位点注入固定失败库（'a'），断言不依赖遍历序，多库同时失败的遍历序形态仍未逐一覆盖。

### 验证

原实施 RED/GREEN 见证（实现前/后，本轮采信）：

```text
cargo test --test cross_db_dml_test -- --test-threads=1
test result: FAILED. 1 passed; 9 failed; 0 ignored     （实现前 RED）
test result: ok. 11 passed; 0 failed                    （实现后 GREEN）
```

本轮修复后（新增/修改目标 + 受影响边界；命令 → 原生输出，退出码 0）：

```text
cargo test --test cross_db_dml_test -- --test-threads=1   → 16 passed; 0 failed
cargo test --lib cli::tests -- --test-threads=1           → 4 passed; 0 failed
cargo fmt --check                                          → clean
cargo clippy --lib --bin rtsql --tests -- -D warnings      → no warnings
openspec validate 2026-10-02-ms20-cross-db-dml --strict   → valid
```

缺陷归属探针（临时文件已删，序列可低成本复现）：单库零路由执行「建表 `c(id PK, n NOT NULL, uniq UNIQUE)` + checkpoint → INSERT(1,10,100) → UPSERT … DO UPDATE → rekey UPDATE → INSERT/DELETE → 多行 INSERT 前行已写后行唯一冲突失败 → 重插 (2,11,101) → WAL shutdown + drop → reopen」，复现：

```text
WalError("WAL redo failed: redo_tuple_at_row_id for row_id RowId { page_id: 2, slot_id: 6 } failed: dense-assignment violation: WAL slot_id=6 but page assigned logical_id=5")
```

对照组同序列（无 abort / abort 后不重插）恢复均干净——缺陷在引擎恢复面而非路由面。

覆盖判定依据 Rust 测试框架原生输出与退出码。未重复运行覆盖未变的既有结论（T1/T2 定向批次、cross_db_query/attach/readonly/memory/busy-timeout 等）——本轮产品代码仅注释变化，行为零变化，原结论采信（来源：原 Act Response + Plan Review 独立审读）。未运行全量（M19 分批属 Iteration 001 T6），未运行性能 bench。

### Persisted Evidence

- None required（Act Response 已保存决定性输出；测试与探针序列均可低成本重跑）。

### Experience Candidates

- **Issue 候选（当前 change 范围外的实质缺陷）**：abort 后槽位/逻辑 id 重用 × 崩溃恢复 redo 失败。复现序列（单库，无需 ATTACH）：建表 `c (id INT PRIMARY KEY, n INT NOT NULL, uniq INT UNIQUE)` + checkpoint → 写入若干已提交行（含 UPSERT 与 rekey UPDATE）→ 多行 `INSERT ... VALUES (2,11,101),(4,41,101)` 前行已写入、后行同语句唯一冲突 → abort → 重插同键 `(2,11,101)`（槽位/逻辑 id 重用）→ WAL shutdown + drop 不 close → 重开报 `WalError("WAL redo failed: redo_tuple_at_row_id for row_id RowId { page_id: 2, slot_id: 6 } failed: dense-assignment violation: WAL slot_id=6 but page assigned logical_id=5")`，open 失败。对照组：无 abort 序列、abort 后不重插序列恢复均干净。影响面：任何「语句部分写入后回滚 → 后续写入 → 崩溃」序列恢复失败（RedoFailed → open 报错）。范围：引擎恢复面 redo 与 abort 槽位回收的交互（`src/wal/recovery.rs` redo / abort 侧），与本 change 的跨库路由无关（单库复现；路由面仅是首个目击者）。证据：本 Response 验证节探针输出，序列如上可低成本复现。

### 未解决问题

- 无（Experience Candidates 的 Issue 候选按规范只报告不落账；建立与处置由 Recorder 按用户指令执行）。

## Plan Review

- Review Result: accepted

**Findings**

2026-10-02 第二次审计（用户指令「开始审计」）：针对 Follow-up Decision 要求的 F1–F3 当前 Cycle 修复做独立复审。完整阅读修复面——`tests/cross_db_dml_test.rs` 全文件（16 用例，与 Act 声明数目一致）、`src/cli/mod.rs` 测试模块（`signal_factory`/`seed_pending_work`/`phase2_signal_closes_attached_databases`/`attached_close_failure_preserves_primary_status`）与编排器（`execute_command_inner_mode`/`close_attached_databases`/`attach_database`/`run_sql` 尾部）、`src/pipeline.rs`（DML 根、`attached_write_in_tx_rejection`、两个 execute 叶子、四写构造臂、三处注释校准）、`src/parser/planner/ddl_dml.rs`（五处 `reject_attached_write` 保留点全部位于 DDL 构建器——create_table/alter_table/drop_table/truncate/create_index，三处 DML 构建器无拒绝调用）、`src/cli/lifecycle.rs`（import_csv mode 适配）、cross_db_query_test 两项迁移（DML 拒绝改成功路由见证、零副作用见证改用仍拒绝的 DDL 目标）与 readonly_session_test r5（实际未消费旧文案，无需校准）。修复轮产品代码仅注释变化，与 Act Response 声明一致。

- F1 已闭合：`attached_multi_row_failure_rolls_back_target` 以 CLI 多行 INSERT 前行已写、后行 NOT NULL 失败见证目标零残留（`rows:[]`）、键 id=1 与唯一值 100 可重插、二级索引在重建条目上路由、主库与此前提交保持；`attached_routed_writes_crash_recovery` 覆盖提交 INSERT/UPSERT（n=15）/rekey（1→10）/DELETE 与多行唯一冲突 abort，崩溃重开后行集、PK 点查、唯一占用（100 对 id=9 拒绝）、二级路由（n=15）、重开主库不变全部直判。偏差 5（abort 后不重插即崩溃）成立：abort 后槽位/逻辑 id 重用 × 崩溃恢复是范围外既有引擎缺陷（Act 附单库零路由探针输出，对照组干净），键重用运行期见证由 CLI 用例承担——F1 要求的见证全部落位，属合理边界划分而非验收裁剪；该缺陷按 Issue 候选报告不落账。`routing_names_and_zero_match` 的 DELETE miss 改为逐行精确断言（恰两行 `{"affected_rows":0}`），宽松 `contains("0")` 已消除；路由用例补 rekey UPDATE。
- F2 已闭合：`phase2_signal_closes_attached_databases` 在 work 内注册并**提交** 300 行后挂起（`seed_pending_work` 于 notify 前完成），SIGINT/SIGTERM 双信号分别见证 `Signaled(2/15)`、附加 WAL 截断 <1KB（close checkpoint 为唯一截断来源）、重开 300 行完整；`attached_close_failure_preserves_primary_status` 场景 1（Success → General 点名别名 'a'；b 库 WAL 截断、主库位点文件写入——两者均有待落盘工作；失败库修复现场后重开 300 行）与场景 2（Sql/Locked/InvalidKey/Signaled(2) 原退出码保持、b 与主库仍完成关闭）覆盖关闭失败矩阵；失败由位点注入固定于 'a'，断言不依赖 HashMap 遍历序。
- F3 已闭合：`attached_commit_checkpoint_failure_preserves_data` 以 1 字节水位 + 位点目录注入见证提交边界 checkpoint 失败——错误点名 `Commit failed`/`checkpoint failed`、同实例行/唯一占用/二级索引保持、主库不变、丢弃实例修复现场后重开经 WAL 恢复；`routed_write_rejections_carry_sqlstate` 在 routed 入口直接断言 SQLSTATE（显式事务附加写 0A000 先于构造、只读 25006、只读×事务 25006 优先、主库事务+附加源 SELECT 成功保持），只读优先于附加写拒绝的代码顺序（`readonly_rejection` 先于 `attached_write_in_tx_rejection`，两叶子同点）已核实；`memory_primary_routes_write_to_attached_file` 与 `encrypted_attached_same_key_write_and_reopen` 补齐运行形态组合，错 key 拒绝按 F3 指引复用 attach_session_test 既有见证不重复。
- Minor（非阻塞，不要求修复）：① T3 契约 Test witness 原文写「0A000（pipeline单测）」，实际落位为 `cross_db_dml_test` 内经 lib routed 入口的集成见证——同一入口直接断言同一 SQLSTATE，覆盖等价，记录偏差不返工；② 多个附加库同时 close 失败时退出消息取注册表遍历序首个（design 允许沿注册表遍历、不新增排序协议，Act 已记录遗留 Minor），场景断言已用位点注入固定失败库，不依赖遍历序。
- 偏差分类：F1–F3 均为 Act 偏离（测试覆盖不足/遗漏契约见证）且已闭合；偏差 5 为新证据（范围外引擎缺陷）导致的见证分布调整。两处 Minor 与偏差 1–4、6–7 均不阻塞 Acceptance。

**Acceptance Gaps**

无。A1–A5 全部有落位见证：A1 route_and_reopen（含 rekey）+ defaults_constraints_and_indexes；A2 independent_file_commits_survive_later_error + multi_row_failure_rolls_back_target；A3 close_after_fail_fast + phase2_signal_closes_attached_databases + close_failure_preserves_primary_status；A4 attached_dml_in_active_tx + readonly_attached_dml + routed_write_rejections_carry_sqlstate + existing_attached_ddl_and_ctas_rejections；A5 crash_recovery（含 UPSERT/rekey/恢复后 PK/唯一/二级）+ commit_checkpoint_failure_preserves_data + routing_names_and_zero_match + update_delete_subquery_shapes + memory/encrypted/threshold 组合。

**Evidence**

- 修复轮 GREEN 由 Act 真实执行并记录于当前 Act Response（`cargo test --test cross_db_dml_test` 16 passed/0 failed、`cargo test --lib cli::tests` 4 passed/0 failed、fmt clean、clippy 无告警、`openspec validate --strict` valid，退出码 0）；本次只读复审未见这些结论之后的覆盖表面变化，直接采信。原实施 RED（1 passed/9 failed）与首轮定向 GREEN 采信首轮 Act Response 与首轮 Review 记录。
- 独立代码审读与 Act 声明逐项一致：planner 三处 DML 放行/五处 DDL 保留；DML 根目标绑定（begin/abort/commit/`abort_tables` 全目标库、abort 键本地名、`target_name` 自有 String）；显式事务 0A000 只判目标且先于构造/begin；非显式根目标只读检查；close 编排（select 外持会话、逐个 await 全部尝试、成功 → exit 1 点名别名、原状态保持 stderr 记录、主库 close 优先级不变）；附加库 checkpoint 水位 env 应用；注释校准仅三处失效句加同族一处，零行为变化。
- 无身份型证据工程：Notify 工厂与 checkpoint 位点目录注入均为既有故障注入夹具形态（wal_auto_checkpoint/MS18 先例同型）；无新增判定层、run-id、指纹、产品开关或专用协议。
- Issue 候选（当前 change 范围外实质缺陷，按规范只报告不落账）：abort 后槽位/逻辑 id 重用 × 崩溃恢复 redo 失败（dense-assignment violation → open 失败；单库零路由复现，复现序列与探针输出见 Act Response Experience Candidates，对照组干净）。建立与处置由 Recorder 按用户指令执行。
- Persisted Evidence：none（模式不变，无 Evidence 目录要求；无 Blocker Handoff 待处理）。
- Gate 3：T1–T3 的 spec/code review 已由 Act 补记于 Act Response（两轮自检发现与修复记录）；Gate 4 PASS（修复轮新鲜 GREEN + 本次独立 diff 审读一致）。Gate 1/2 既有批准与就绪结论保持。
- SKIPPED：产品代码/测试修改（本次只授权 Review）、重复已通过验证（无失效疑点、无重跑触发情形）、全量/bench（属 Iteration 001 T6 与非目标）、经验落账/全局状态/归档（非本阶段授权）。

**Follow-up Decision**

接受当前 Cycle：F1–F3 修复满足全部既有 Acceptance，仅余两条非阻塞 Minor（见证位置等价、多库同时关闭失败的遍历序角落），不构成返工理由。Iteration Plan 与 Map 不变（T4–T6 保持原契约方向）；按既有批准展开 Iteration 001——`iterations/001-query-result-writes/000-initial.md`（Plan Context ready，T4 计划面 → T5 执行面 → T6 文档与 M19 全量分批，Current Baseline 引用本轮最终 Act Response 与本 Review）。范围外 Issue 候选不落账，交 Recorder 按用户指令处置。

**Iteration Plan Update**

None。

**Next Cycle**

None（Iteration 000 完成，Cycle 冻结）。

**Next Iteration**

`iterations/001-query-result-writes/000-initial.md`（T4–T6：查询结果写入与完整使用说明）。
