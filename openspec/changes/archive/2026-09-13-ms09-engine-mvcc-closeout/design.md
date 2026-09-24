# MS09 引擎能力与 MVCC 收尾 — Design

> 调查基线:2026-09-13,HEAD e51c4a3,工作区含 MS16 未提交实施改动(与 SNAPSHOT 记录一致,基线 892 tests pass / 0 failed / 2 ignored 采信)。全部事实由 Plan 直接读取代码核实(未委托子代理)。

## D0 调查总结(Current-State Evidence)

**可见性架构现状**:

- 扫描三路径:`ScanExecutor`(索引 scan_all 驱动,无快照时裸读 slot,`scan.rs:55-98`)、`DataScanExecutor`(数据页链驱动,R6 superseded-map 抑制 + 逐 slot 检查,`data_scan.rs:326-505`)、`IndexScan/IndexScanAll`(索引点查/全遍历,`pipeline.rs:476-500`)。**生产查询路径所有扫描构造传 `snapshot: None`**(`pipeline.rs:457/468/483/496`);显式事务路径(`execute_stage_in_tx`)传 tx_id 但仅 DML 消费,查询可见性同样无快照(`pipeline.rs:269-270` 注释)。`Transaction.snapshot` 全仓无消费方。
- `VersionHeader` 22B:`create_tx_id`(8B)+ `commit_tx_id`(8B,`UNSET_TX_ID=u64::MAX` 即 None)+ `next_version`(6B)(`version_chain.rs:16-20`)。墓碑 = **就地**把最新版本 header 的 `commit_tx_id` 改写为 `DELETED_TX_ID=u64::MAX-1` 哨兵(`delete.rs:62-74`),`next_version` 保留指向前驱;`commit()` 守卫保留哨兵(`version_chain.rs:56-67`)。**已提交删除与未提交删除在页面上不可分辨,且删除者 tx_id 不在任何字段中**。
- I033 根因:`superseder_suppresses`(`data_scan.rs:310-321`)对 `is_deleted()` 恒 `false`;墓碑 slot 自身被 `:422-424` 跳过后,前驱经 superseded-map 回溯不被抑制 → 产出 pre-update 版本。UPDATE 建独立新 slot(`update.rs:143-149`,`next_version→old`),DELETE 不建 slot。
- `commit_mark_versions`(`manager.rs:239-254`)对 `tx_versions` 记录的 rid 写真实 commit_tx_id(墓碑守卫跳过)+ `clear_all_visible`;`abort_cleanup_versions`(`manager.rs:262-305`)索引修复 + `mark_deleted` 墓碑化回滚版本。
- 恢复:`full_recover` **只重放 committed 事务记录**(`recovery.rs:458/472`);Delete redo 臂镜像运行期就地墓碑(`recovery.rs:741-792`);`mark_uncommitted_aborted` → `BufferPool::mark_tx_aborted` **no-op**(`buffer_pool.rs:369-373`,`recovery.rs:978-990`)——被驱逐/checkpoint 落盘的未提交行重启后对无快照扫描复活(I032 实际影响,非纯理论)。
- 页级摘要:`all_visible` 由写路径清除、仅 `snapshot.is_some()` 时惰性置位(`data_scan.rs:462-476`)——生产恒 false,快速路径实际不生效;DataScan 的 `is_deleted` 检查(:422)先于 all_visible 分支(:428),墓碑不受快速路径绕过。

**Join 现状**:`JoinNode { left, right, conditions: Vec<JoinCondition>, output_columns }`;`JoinExecutor` 纯等值 Hash INNER(`join.rs`,NULL 键不匹配);`extract_join_conditions`(`ddl_dml.rs:29-76`)仅接受 AND 组合的列=列等值腿,**其余(含一切非等值)计划期 `PlanError::UnsupportedExpression`("Unsupported expression type",`error.rs:17/:67`)响亮拒绝**;JOIN 类型仅 Inner(`query.rs:171-175`)。`tests/join_test.rs` 7 用例全为直连执行器等值用例,无非等值拒绝锁定。

**关联子查询现状**:标量 `SubqueryEvalExecutor` 每外层行 `plan.clone()` → `inject_correlated_values` → `create_executor_from_plan` 全量重执行(`subquery_eval.rs:111-156`,文件头自注 "re-executed per outer row (no caching)");非关联臂已有 `cached_result: Option<Value>` 求值一次(`:149-155`);Semi/AntiJoin 关联臂同型逐行重建(`semi_join.rs:189-224`/`anti_join.rs:185-218`)。参数值 `Vec<(String, Value)>` 满足 Clone+Eq+Hash(`Value` 手工 Hash,`value.rs:61-77`)。执行器树每语句重建 → 语句级缓存的生命周期由构造面天然保证。`lru` 0.12.5 为声明未用的死依赖;仓库唯一缓存先例 PlanCache 为 DashMap 非 LRU。无执行次数计数探针先例(tests 风格禁止 test-only hooks)。

## D1 墓碑表达:就地标记 → 独立版本 slot(I033 核心)

DELETE 不再就地改写最新版本 header,改为**写入独立墓碑 slot**:`VersionHeader { create_tx_id: 删除者tx_id, commit_tx_id: DELETED_TX_ID 哨兵, next_version: → 被删行 rid }`,tuple 为空。要点:

- 22B 格式不变;删除者 tx_id 自描述保留 → 「已提交墓碑 vs 未提交墓碑」可在运行期按删除者活跃性判定,重启后按恢复语义判定(见 D3)。
- 前驱行 header 不再被破坏:未提交删除的并发扫描回溯到 pre-delete 版本(修复前回溯越过它产出 pre-update 版本——调查实证的未提交形态伴生缺陷,随本设计一并消除)。
- 索引移除保持即时(`delete.rs:85` 原样):点查路径不遇墓碑,行为不变。
- `tx_versions` 记录对象从被删行 rid 改为墓碑 slot rid(commit 期 `write_commit_tx_id` 对哨兵守卫为无害 no-op;abort 期按 D2 中性化)。
- WAL `WalRecord::Delete { tx_id, table_name, row_id=被删行 }` 记录格式不变,重放侧由 redo 臂重建墓碑 slot(见 D3)。
- 就地标记的旧形态仅存在于旧二进制产物,预发布无兼容要求(MS16 先例)。

替代案否决:header 格式扩展(需增 flags 字节,页格式变更 + 全序列化面回归,收益不高于 slot 化);运行期 pending-delete registry(跨重启不自描述,重启后仍需 header 判定,回到原点)。

## D2 抑制判定与 abort 中性化约定

**标记约定**(统一运行期与恢复):

- 删除墓碑 slot:`create_tx_id = 删除者`,`commit_tx_id = SENTINEL`。
- aborted 版本(任何来源):`mark_aborted()` = `create_tx_id = 0` + `commit_tx_id = SENTINEL`(`next_version` 保留)。`is_deleted()` 仍为 true(现有 `manager.rs` 单测的 `is_deleted` 断言保持通过)。

**抑制规则**(重写 `superseder_suppresses`,墓碑作为 superseder):

- `create_tx_id == 0`(aborted 标记)→ 不抑制。
- 有 RC 快照:`create_tx == snapshot.tx_id`(自身删除,对己可见为已删)→ 抑制;`snapshot.contains_active(create_tx)`(语句开始时仍活跃)→ 不抑制;否则(语句开始前已提交)→ 抑制。
- 无快照(RR 现状路径):读取时 `active_tx_ids.contains(create_tx)` → 不抑制;否则 → 抑制。DataScan 需获得活跃集合——构造参数增加 `Option<Arc<TransactionManager>>`(生产传 `Some`,极简单元测试可 `None`,语义等价于空活跃集)。

**abort 中性化**:`abort_cleanup_versions` 的墓碑化从 `mark_deleted` 改为 `mark_aborted`;对 DELETE 的墓碑 slot rid(新记录对象)同样 `mark_aborted` → 调查推演全链自洽:回滚后扫描产出回滚前最新已提交版本,无复活、无误抑制(推演记录见 Iteration 000 Plan Context)。

**已知边界(不扩入本 change,登记 Issue 候选)**:未提交删除期间点查不可达(索引即时移除,现状即如此)与 DELETE 回滚后 PK 点查不可达(`abort_cleanup` 的 `find_key_by_row_id` 对已移除条目返回 None,现状即如此)——两态为预存索引时序边界,修复需延迟索引移除或 abort 恢复条目,涉及同事务 delete+insert-same-key 语义,超出本 change 范围。

## D3 恢复侧(I033 恢复一致 + I032 实施)

- Delete redo 臂(`recovery.rs:741-792`):就地墓碑改写 → 写独立墓碑 slot(`create_tx = record.tx_id`——redo 只处理 committed 事务,天然满足「重启后墓碑 = 已提交删除」;`next_version → record.row_id`),幂等性由 redo 单次性 + 重复 slot 无害性保障(重复墓碑经 map「最新创建者胜出」收敛,`data_scan.rs:249-257`)。
- **I032 处置:实施**。`mark_uncommitted_aborted`(`recovery.rs:978-990`)实现为:经 `TableManager` 各表数据页链迭代,`create_tx_id ∈ uncommitted_tx_ids ∧ commit_tx_id == UNSET` 的 slot 改写为 `mark_aborted()`——消除「未提交行被驱逐/checkpoint 落盘后重启复活」窗口;对 `rebuild_pk_indexes`(`:817+`,先 mark 后重建,顺序 `:479 < :484` 不变)无影响(墓碑 slot 被链尾回溯的 is_deleted 检查排除)。
- `BufferPool::mark_tx_aborted` no-op(含失实的 "active_tx_ids preserved across restarts" TODO 注释)删除,调用点改指新实现。I032 的「实施」由恢复侧真实标记满足,登记 promoted。

## D4 Read Committed(D4 = 配置面 + 每语句快照穿线)

- `IsolationLevel` 枚举(`transaction` 模块):`RepeatableRead`(默认)/ `ReadCommitted`。`Database::open_with_isolation(path, level)` 新增;`open(path)` 委托 RR——38 个 `Database::open` 调用点零改动。字段 `pub isolation`(Database 为 pub 字段结构体)。
- **快照构造**:RC 模式下每条语句执行前构造 `Snapshot::new(reader_tx_id, active_now)`:
  - auto-commit 查询(`execute_stage` 查询臂):`reader_tx_id = allocator.current()`(不需 WAL BeginTxn;单调性保证已提交事务 id ≤ 当前值)、`active_now = active_transactions()`。
  - 显式事务内语句(`execute_stage_in_tx`):`reader_tx_id = tx_id`、`active_now = active_transactions()`(含自身——自身写经 `is_visible_self` 放行)。
  - RR 模式:两条路径维持现状(`snapshot: None`),逐字节零回归。
- **穿线**:`create_executor_from_plan` 增加 snapshot 参数(`Option<Snapshot>`),四个扫描构造点传 `Some`(RC)/`None`(RR);子查询逐行重建(`subquery_eval.rs:121-126`)与 Semi/Anti 关联重建、DerivedScan 物化同参数穿线(RC 下子查询见语句快照)。DML 臂不受影响(tx_id 语义不变)。
- **DataScan 可见性**:有快照时逐 slot 检查为 `is_visible ∨ is_visible_self`(现状只有 `is_visible`,`data_scan.rs:430`——显式事务内自身未提交写需放行);墓碑抑制按 D2。
- 语义边界(诚实记录):默认 RR 路径保持现状无快照语义(其他事务未提交写对扫描可见——预存行为,非本 change 扩大或收窄);RC 严格于 RR 现状(排除脏读)。RR 的真快照化不在本 change。

## D5 NLJ + 启发式(T02)

- 新计划节点 `NestedLoopJoinNode { left, right, predicate: PredicateRef, output_columns }` + `NestedLoopJoinExecutor`:对左输入每行 × 右输入每行,组合行(左行 ++ 右行)上求值谓词,三值语义真(非 Unknown/假)组合按 `output_columns` 产出;流式 Volcano 形态(右输入每左行重扫描或物化,非实质选择留 Act)。
- **ON 分类启发式**(planner,`build_from_clause_with_projection` / `extract_join_conditions` 扩展):AND 分解后**全部腿**为可解析列=列等值 → 既有 Hash 路径逐字节保持;**任一腿**非等值(或 ON 整体非纯列等值,如含字面量/表达式)→ NLJ,ON 整体经既有 WHERE 谓词编译机制(`expression.rs`)在组合行布局(左表偏移 0..n、右表偏移 n..n+m,绝对索引)上编译。混合腿(等值+非等值)→ NLJ 全谓词评估(Hash 无残余谓词机制,保持纯等值边界)。
- JOIN 类型面:仅 INNER(与现状对齐,`query.rs:171-175` 保持);LEFT/RIGHT/FULL 维持 `UnsupportedJoinType`。
- 注册面清单:`PhysicalPlan` 枚举(plan.rs)、`create_executor_from_plan` 新臂(`pipeline.rs:588` 邻接)、`get_plan_output_columns` 新臂(`query.rs:77-90`)、`inject_correlated_values` 新臂递归左右子树(`correlated.rs`——关联 ON 经 WHERE 编译器参数化后需注入)、`extract_column_indices` 新臂(`pipeline.rs:740+`,嵌套/关联面)、`Plan` Clone(derive 已有)。非等值形态从 `Unsupported expression type` 拒绝改为正确结果(解锁,`tests/` 无既有拒绝锁定,零校准)。

## D5a Iteration 001 计划闭合（2026-09-14，Plan 调查补全）

- **delta spec R2-S2 场景修订**：原场景第二腿含算术操作数（`s.b - r.a >= 2`）。调查实证 `build_expression` 无算术 BinaryOp 臂（`expression.rs:170-432`——BinaryOp 仅在 `build_where` 比较臂处理，操作数递归 build_expression 后落 `expression.rs:430` UnsupportedExpression），以现机制不可编译；引入算术表达式属新表达能力（语义面波及全部 WHERE），超出 T02 NLJ 范围。场景修订为纯比较混合腿 `ON r.a < s.b AND r.a >= 2`（r={1,2}/s={2,3} → {(2,3)}），场景意图（多腿 AND 全部评估）不变；算术表达式维持既有拒绝面，本 change 不新增。
- **ON 分类启发式处方**：`extract_join_conditions` 本体不动（`ddl_dml.rs:29-76`，Hash 路径逐字节保持）；新增结构性探测——AND 分解后每腿均为 `BinaryOp::Eq` 且两侧为 Identifier/CompoundIdentifier 列引用 → Hash；任一腿不符（含字面量/表达式/比较符非等值）→ NLJ，ON 整体经 `build_where`（`expression.rs:435-586`）编译。探测为纯结构判定不做语义解析——等值形态的既有语义错误（ColumnNotFound/AmbiguousColumn 等）经 Hash 路径原样保留，拒绝面不因探测改变。
- **组合行布局编译机制**：`PlanBuilder` 加性字段 `join_column_layout: Option<Vec<(String, Vec<String>)>>`（current_tables 序 + 右表；偏移 = 前序表宽度累计），`build_expression` 两个列解析臂在 Some 时优先消费（`expression.rs:176-233`）：限定名 → 所属表偏移 + 表内位置（inner_table_names 关联检查先行，保持子查询外引用语义）；非限定名 → 布局全表搜索（0 → ColumnNotFound；>1 → AmbiguousColumn；1 → 偏移+位置）。NLJ 分支 save/restore 包裹（`inner_table_names` 同型先例）；None 时两臂行为逐字节不变。
- **注册面枚举（T13）**：`get_plan_output_columns` 新臂（output_columns 列名，`query.rs:77-84` Join 臂同型）；`extract_column_indices` 新臂（output_columns 索引 + 首列 table_alias，`pipeline.rs:805-820` Join 臂同型）；`inject_correlated_values` 新臂（递归左右 + `predicate.inject_parameters`，`correlated.rs:52-55`/:36-42 同型）；query.rs 三个 Join 匹配点扩展——`:345`（table_name="join_result"）、`:424`（SELECT 表达式项 + JOIN 拒绝）、`:507`（WHERE + JOIN 拒绝，probe 实证 `Unsupported statement type` exit 3）——NLJ 节点与 Join 节点同语义，拒绝面不得因新节点形状漏接。
- **边界（预存，不扩大不修复）**：WHERE + JOIN（两种节点）维持计划期拒绝；SELECT 表达式项 + JOIN 维持拒绝；3+ 表链中 NLJ 左输入为投影后 Join 时，谓词布局基线（全 schema）与左执行器实际输出形状的错位与既有 Hash `build_output_row`（`join.rs:113-124`，table_alias + 全 schema column_index）同源；JOIN 类型面保持仅 INNER（`query.rs:171-175` UnsupportedJoinType 原样）。

## D6 关联子查询缓存(T04)

- `SubqueryEvalExecutor` 关联臂增加语句级缓存字段 `HashMap<Vec<(String, Value)>, Vec<Vec<Value>>>`:键 = `extract_param_values` 产物(现成 Clone+Eq+Hash),值 = 子查询完整行集;命中直接产出,未命中执行后填充。**语句生命周期由执行器每语句重建构造面保证**(调查确认 executor 不跨语句复用),无需淘汰策略——修正 proposal DA1 的 LRU 设想(语句界有界,HashMap 足够;lru 依赖保持未用,不为单一用途引入新风格先例)。
- Semi/AntiJoin 关联臂同型缓存(逐行重建右计划的两个循环,`semi_join.rs:189-224`/`anti_join.rs:185-218`)。
- 错误语义:子查询执行 `Err` 直接传播、**不缓存**(同参数重复遇错重复执行重复报错——与现状逐行重执行行为一致);多行标量错误面(`SubqueryReturnsMultipleRow`)保持。
- NULL 参数值:作为键成分结构成立(Null hash discriminant);谓词语义层 NULL→Unknown 的既有行为不变,本 change 不改变求值结果,只消除重复执行。
- 「不重复执行」的可观测性:无 test-only hooks 风格约束(仓库先例),Acceptance 以可观测结果面锁定(等价性/跨语句新鲜度/错误面),执行次数由实现与 Self-Review 代码审查保证,不建立计数机制(身份型证据工程禁令)。

## D7 行为保持与校准面预案

- 调查推演的零回归锚点:`plan_exec_test` delete 流(索引移除后扫描空集,Option C 下不变);`explicit_tx_test` abort 断言(`is_deleted()` 在 `mark_aborted` 下仍真);`version_chain_test`/`gc_test`(UpdateExecutor 建链,不触 DELETE 路径);`join_test`(直连 Hash 执行器,不触 planner);子查询全套(非关联臂 `cached_result` 不变)。
- 既有测试若在实施中暴露对旧缺陷行为的依赖,按 MS16 BH 先例在对应 delta spec 记录校准,不静默放宽断言。
- 验证纪律:每 Iteration 全量回归零修改通过为收尾门;新增测试套件各自 RED→GREEN。

## D8 Iteration 划分

- **Iteration 000(foundation)= T01 事务可见性域**:I033(D1/D2)+ I032(D3)+ RC(D4)。三者共享版本链/扫描可见性同一故障域与同一批测试入口,聚合为一个可独立验收成果「事务可见性收口」;I033 是 RC 语义正确性的前提(墓碑判定先闭合),同 Iteration 内先 I033 后 RC。
- **Iteration 001 = T02 NLJ**(独立故障域:planner/执行器 Join 面)。
- **Iteration 002 = T04 子查询缓存**(独立故障域:子查询执行器;依赖 Iteration 000 的快照穿线面为 RC 下缓存键稳定提供基础,顺序执行)。

平衡审计:000 承载 T01 全部(工作量最重但内聚——拆开则 I033 与 RC 相互触碰同一函数族,验证面重叠);001/002 各为单一可验收成果。无过碎/过重问题。

## D9 验证设计

- I033 RED:库级探针序列(INSERT→UPDATE→DELETE 各自 auto-commit)断言扫描空集(修复前 `[[1,10]]`)+ restart 变体(close→reopen 后同断言);未提交删除变体(显式事务活跃期间并发扫描断言 pre-delete 版本)。
- I032 RED:构造未提交行落盘(小 BufferPool 驱逐或直接写页)→ close → reopen → 扫描断言不可见(修复前复活)。
- RC RED:RC 模式下他事务未提交 INSERT 对本事务语句不可见(修复前可见——现状脏读)、提交后下一条语句可见;RR 默认全量零回归背书。
- NLJ RED:`ON r.a < s.b` 当前 `Plan error: Unsupported expression type`(exit 3)→ 实施后语义连接结果 + plan 形状断言(NestedLoopJoin 节点)。
- 缓存:结果等价 + 跨语句新鲜度 + 错误传播面(RED 由「等价性在缓存引入前后不变」的性质保障,新增用例先于实现断言目标行为)。
- 全量:`cargo test`(基线 892 + 新增),`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate --specs --changes` 全 0/PASS。

## D10 快照结构修订（001-replan，2026-09-13 Plan Review 裁定）

000-initial 实施实证 D4 的单 id 快照在 RC 语句视图上结构性不可满足：`Snapshot.tx_id` 同时承担 (a) 可见性高水位（`is_visible` 规则 2：create > tx_id → 不可见，`snapshot.rs:42-44`）与 (b) 自身身份（`is_visible_self`：create == tx_id，`snapshot.rs:56-58`）两个不相容角色——auto-commit 取 `current_tx_id()` 满足 (a) 但与活跃写事务 id 撞车（脏读 R2-S1 实证 `[[7,70]]`）；显式事务取 `tx.id()` 满足 (b) 但把 begin 晚于 reader 的事务的提交排除在高水位外（R2-S2 实证 `[]`）。T6 原 Preserve「Snapshot 本体不变」使契约无法达到既有 Acceptance，Review 裁定 PLAN-INVALID，本节修订设计。

**修订方案（自身身份与高水位分离，采纳 Act 修复方向 1）**：

- `Snapshot` 结构扩展：`tx_id` 保留为自身身份语义；新增 `high_water` 字段；`is_visible` 规则 2 改用 `high_water`；`is_visible_self`、`contains_active_tx` 不变。
- 构造器双轨：既有 `Snapshot::new(tx_id, active)` 保留原语义（self == high_water == tx_id——begin 时点 RR 快照的正确形态），manager.rs 与全部测试/bench 调用点零改动；新增 `Snapshot::statement_view(high_water, self_tx_id, active)` 供 RC 语句视图。`statement_snapshot`（database.rs:119-127）切换：auto-commit → `statement_view(current_tx_id(), 0, active)`（self=0：auto-commit 无自身事务，id 0 为 aborted 标记值永不匹配真实版本）；显式事务 → `statement_view(current_tx_id(), tx_id, active)`。
- 语义论证：事务 id 单调分配 → 任意 create ≤ current 的 id 必为 committed / aborted / active 三态之一（active 集合取语句开始时点）；`high_water = current_tx_id()` 排除捕获后新 begin 的事务（其 id > current）；语句间提交的事务 id ≤ 语句开始时 current 且不在 active 集 → 可见（R2-S2 修复）；他事务未提交行 commit=None 被规则 1 排除且 self=0 不撞车（R2-S1 修复）。
- **分配器水位推进（RC 正确性前置，随本修订一并实施）**：`database.rs:74-81` 现状计算 `_max_tx_id` 后丢弃，重启后分配器从 0 重来（`tx_id.rs:8-20`）——高水位论证「≤ current 即 committed/aborted/active」要求无 id 复用，且复用会使新事务 abort 的 `mark_aborted`（create_tx 归零改写）误伤历史同 id 版本。`TransactionId` 新增 `advance_past(max_used)`（CAS 保证 counter ≥ max_used），`open_with_isolation` 用已计算的 max 推进；`current()` 返回推进后值，RC 重启后首条语句的高水位覆盖全部已恢复 committed id。
- 消费面零改动：`find_visible_version`（buffer_pool.rs:336-337）与 DataScan 抑制判定均为 `Snapshot` 方法消费，本体不动；`data_scan.rs` 快照臂的自身删除判定继续用 `tx_id`（自身身份语义不变）。

替代案否决：方向 2（Snapshot 不动、另穿 reader self-id）触碰面更大且把双参数语义散落到构造链，不优于结构分离；方向 3（缩窄验收语义、放弃「begin 晚于 reader 的提交可见」）改变 delta spec R2 既有场景，属验收让步，不采纳。

**001-replan 实施修正（002-rework 承载，2026-09-13 Plan Review）**：本节上一条「消费面零改动」论断不完整——页级可见性快路径存在第三类高水位消费点，直接以 `Snapshot::tx_id()`（自身身份）作页级可见性高水位：DataScan 页快路径 `all_invisible_for(s.tx_id())`（`data_scan.rs:410-413`，R2-S2 实证直接短路点）、`find_visible_version` all-invisible 快路径（`buffer_pool.rs:313`，RC 点查同型缺口）、`check_page_all_visible` 条件 2（`buffer_pool.rs:441`，statement_view 下保守安全仅损失优化）。`page_visibility.rs:6-8` 头注释同样把该参数记作 snapshot tx_id。三处在 statement_view 下以自身 id 判页级不可见，`min_create_tx_id ∈ (self, high_water]` 的整页被误跳。处方补全：新增 `Snapshot::high_water()` 访问器，三处消费点统一改传高水位（`new` 构造下 high_water == tx_id → RR 面与既有单测/bench 逐字节不变）；`check_page_all_visible` 条件 2 同改（`create > high_water` 才不可见），RC 下恢复 all-visible 置位优化。健全性论证：语句视图自身 id ≤ 高水位（statement_view 构造保证——reader 分配于语句前，auto-commit 为 0），`create > high_water ⇒ is_visible 规则 2 假 ∧ is_visible_self 假`，整页跳过语义成立；RC 高水位随语句单调不减，all_visible 旗对后续 RC 视图无陈旧风险（RR 生产路径不传快照，快路径休眠；测试构造的 `new` 小 id 快路径陈旧旗形态为预存形状，本修正不扩大）。
