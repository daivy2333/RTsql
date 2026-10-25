# Iteration 000 / Cycle 000: 索引写侧原子性与取消/回滚后的键释放

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2, T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: change `2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog` 的需求、规格增量 `specs/index-write-side-atomicity/spec.md`、design D1–D4 与 D7；project-model M13（异步执行原则）、M19（全量分批）、M20（范围外缺陷承载闭环）；公共规则 TDD、Gate、验证、行为约束。
- Excluded scope: DDL 目录面（Iteration 001 的 T4–T6）；MS35 空闲页跨重启复用；MS36 历史库与 dump 兼容；I096 分级诊断系统；MS34 已验收的取消语义面；`openspec/specs/` 的合并（由 docs-maintainer 收尾执行）。

**Objective**

主键/唯一索引的结构变更、根页 id 发布与反向映射更新成为**不可分割且串行**的写单元：语句被取消时该单元整体落地或整体未开始，不产生「换根已发生但根页 id 未发布」或「正向条目已存在但反向映射缺失」的半写状态；被取消语句写入的**全部**键在回滚后释放；取消/回滚之后索引结构自洽（后续写不 abort、完整性无 Error）。

**Investigation Facts**

- Starting point:
  - 工作区 HEAD `0a08b14`（ahead of origin 9）；MS34 收尾已完成（规格合并 72→73 域、change 归档）。工作区仍含 MS34 的产品代码与测试文件改动（未提交）——属既有现场，**不回滚、不清理、不提交**。
  - 既有可沿用结论：`openspec/specs/statement-cancellation-outcome/spec.md`（MS34 合并）R2 的已知边界明确「键的释放不属取消域保证面，由存储域的索引写侧契约承载」——本 Cycle 交付该契约。
  - 既有缺陷正文（**不在本文件复制正文**，按指针引用）：`.agents/issues/ISS20-index-write-side-atomicity.md`（E1/E2、根因、gdb 现场、6 次未命中的竞态依赖）、`.agents/issues/ISS21-ddl-catalog-transactional.md`（Iteration 001 用）；调查正文 `openspec/changes/archive/2026-10-24-ms34-cancellation-timeout-transaction/iterations/001-caller-visible-outcomes-and-final-regression/001-replan.md` 的 `Plan Review` 与 `Act Response` 的 Experience Candidates 段。
  - 变更前验证结论（沿用，不重跑）：MS34 归档 Cycle 的 Act Response 记录了其定向用例与收尾全量分批结果；覆盖范围未变化的部分直接沿用，本 Cycle 只对新增/受影响的定向用例做验证。
- Current-State Evidence:
  - **E1/E2 入口**：`src/storage/btree/index_manager.rs:239-258`（`IndexManager::insert`）——`:240` 读 `root_page_id`；`:244-248` `spawn_blocking` 内做 B-Tree 结构变更（**tokio 的 blocking 任务在 JoinHandle 被丢弃后仍会跑完**）；`:251-254` 根页 id 发布 + 目录根同步（`sync_root_to_catalog`，async）；`:256` 反向映射 `row_to_key.write().await.insert(...)`。三段被两个 await 切开 ⇒ 取消落在 `:248` 与 `:256` 之间 ⇒ 换根未发布 + 反向映射缺失。
  - **并发面**：同一文件 `:261-282`（`IndexManager::delete`）——`:263` **先**删反向映射、`:270-274` 再删正向条目、`:276-279` 根页 id 发布；两条路径各自 `spawn_blocking` 一个临时 `BTree::from_root(root_page_id, ..)`，彼此无写者串行化 ⇒ 并发写者交错改同一页 / 用旧根覆盖新根。
  - **回滚分桶**：`src/transaction/manager.rs:339-355`——按 `find_key_by_row_id` 是否命中分 A 趟（有条目→删除/回退）与 B 趟（墓碑→从前驱版本还原）；反向映射缺失 ⇒ 误判为 B 趟 ⇒ 正向条目不被移除 ⇒ 键残留（E2）。
  - **损坏入口**：`src/storage/btree/btree.rs:237-327`（`BTree::insert_into_page`：`:243` 取页、`:248` 叶子/内部判定 `data_guard[0] == LEAF_NODE`、`:319-327` 取 `child_page_id` 递归下降）；ISS20 记录的 gdb 现场：339+ 层 `insert_into_page`、`child_page_id_u32 = 0`、`BTree::root_page_id = 75` 而该行 `RowId.page_id` 也是 75（根页 id 指向非 B 树节点页）⇒ 进程栈溢出 abort。
  - **夹具几何（已按代码推导）**：页 4096（`Page::PAGE_SIZE`）；叶条目 38 字节（`MAX_KEY_LEN 32 + RowId::SIZE 6`）+ `Slot::SIZE 6`；`SlottedPageHeader::SIZE = 16` ⇒ **单叶最多 92 条**（`16 + 93×44 > 4096`）；`MIN_KEYS = 46`（`src/storage/btree/btree.rs:20`）⇒ 第 93 个键触发叶分裂（46/47）并换根，新根页在闭包内创建、`Ok(Some(new_root))` 返回，**根页 id 的发布在闭包之外**（`index_manager.rs:252`）。
  - **测试入口与既有约定**：`tests/statement_cancellation_test.rs`——`abandon_statement(db, sql)` 用「轮询 `active_transactions()` 中 `get_tx_versions(tx_id).len() >= 2`」作锚点，`yield_now` 轮询，`ANCHOR_TIMEOUT` **只作夹具失控兜底**（`.expect("the abandon anchor must be reachable: ...")`），不是行为判据；既有 DDL 回收用例 `abandoned_alter_table_drop_column_is_reclaimed`（`:316`）**只断言回收契约，不断言目录/数据面形态**（ISS21 的落点）。
  - **测试支持夹具先例**：`src/database.rs:870` `Database::arm_io_failure(&self, point: IoFailPoint)`——产品代码内的受控故障注入夹具（本 Cycle 的写侧暂停点沿用同一形态）。
  - **完整性检查**：`src/storage/integrity.rs` 的 `check_integrity`（`:87`）逐表校验数据链与索引树（`:218` `check_table` → `:413` `check_index_tree`），Error 级发现可作为断言对象。
- Code and Critical Path:
  - `src/storage/btree/index_manager.rs`：`IndexManager`（`root_page_id: AtomicU64`、`row_to_key: tokio::sync::RwLock<HashMap<RowId, Vec<u8>>>`、`catalog_ctx`、`sync_loader`）——本 Cycle 的主要改动面。
  - `src/storage/btree/btree.rs`：`BTree::insert` / `BTree::delete` / `merge_leaves` / `shrink_root`——结构变更与换根/合并的实现；**本 Cycle 不改其语义**（只读依赖）。
  - `src/transaction/manager.rs`：`abort_by_id`（`:160`）→ `abort_cleanup_versions`（`:331` 起，A/B 分桶在 `:339-355`）——回滚清理路径；分桶逻辑**不需改动**（根因在源头消除）。
  - `src/executor/insert.rs:275-286`：`record_version` → `index_manager.insert` 的写入顺序（版本登记在前、索引写入在后）——取消窗口的来源顺序。
  - `src/database.rs` / `src/pipeline.rs`：语句执行与取消（future 被丢弃）的入口；MS34 已验收语义**不改**。

**Implementation Guidance**

- 先做 T1（建立触发能力 + 观察 RED），再做 T2（实施修复），最后 T3（端到端回归）。理由：TDD 铁律要求先有 RED。
- 写侧暂停点的语义位置固定为「**结构变更已生效、根页 id 尚未发布**」，修复前后不变（修复后该位置即闭包内「结构变更之后、发布之前」）。未装载时不产生行为差异。
- 写单元的实现形状：一个 `spawn_blocking` 闭包内完成「获取写锁 → 读当前根 → B-Tree 结构变更 → 根页 id 发布（`AtomicU64::store`）→ 反向映射更新」；反向映射容器改为 `std::sync::RwLock`（闭包内同步访问，调用点不跨 await 持锁，符合 M13）；`find_key_by_row_id` 的公开签名保持 `async`，内部改同步加锁。
- `delete` 的两段写入同样收进同一单元，且顺序改为「先删正向条目、后删反向映射」（当前是反过来的，会产生同形半写状态）。
- 写侧串行化：每个 `IndexManager` 一把 `std::sync::Mutex<()>`，在闭包**内部**获取/释放。
- `sync_root_to_catalog`（async）留在闭包之外、闭包返回之后执行（持久化关注点，不放进结构原子性单元；既有语义不变）。
- 暂停点的实现要点：「到达」信号用可在阻塞线程触发的通知（测试侧可 await 或 `yield_now` 轮询）；「放行」用 `Mutex<bool>` + `Condvar`（阻塞线程等待）。**不得**引入 run-id / 指纹 / 时间戳 / 判定层。

**Task Definitions**

### T1: 建立确定性触发 ISS20 窗口的能力 + 先行 RED 用例

- Requirement/Scenario: `index-write-side-atomicity` — 「写在结构变更与根页 id 发布之间被取消时索引保持自洽」；「被取消语句写入的第二个键也能再次插入」。
- Depends on: None
- Targets: `src/storage/btree/index_manager.rs`（`IndexManager`：新增写侧暂停点与其装载入口）、`tests/index_write_atomicity_test.rs`（新增）。
- Current behavior: 写侧没有可控的暂停点；取消落在结构变更与根页 id 发布之间时，索引出现「树已换根、已发布根仍是旧根」且反向映射缺失 ⇒ 新半叶的键不可达、该键残留。
- Required behavior: 装载暂停点后，一个索引写者能在「结构变更已生效、根页 id 尚未发布」处被确定性地停住并被放行；用例观察到：取消该语句后，键 1..92 中至少一个不可查（换根丢失的右半），且该语句写入的键再次插入报重复键。
- Preserve: 未装载暂停点时索引写路径行为逐字节不变（既有测试保持 GREEN）；既有 `IndexManager` 公开 API 的语义与签名不变（除新增的测试支持入口）。
- Forbidden: 不修改 `BTree` 的结构变更语义；不引入身份证明机制或判定层；不改 `statement-cancellation-outcome` 域的既有语义；不在 `cargo test` 之外建脚本或专用命令。
- First test: `tests/index_write_atomicity_test.rs` 新增用例——建表（单列主键）→ 插入 92 行 → 装载写侧暂停点 → 后台任务执行第 93 行的插入 → 轮询「到达」信号（`yield_now`；超时只作夹具失控兜底，触发即失败）→ 丢弃语句 future → 放行 → 执行下一条写语句触发回滚 → 断言键 1..92 全部可查、键 93 可再次插入。**变更前观察 RED**（键 47..92 不可查 / 键 93 报重复键）。命令：`cargo test --test index_write_atomicity_test -- --test-threads=1`。
- GREEN condition: 本任务不追求 GREEN——本任务的完成判据是「暂停点夹具就位 + 用例稳定 RED」。RED 必须**每次**复现（连续 3 次运行同一结果），否则触发「三次失败必须反思」并回到 Plan。
- Verification: `cargo test --test index_write_atomicity_test -- --test-threads=1`（观察 RED）；`cargo test --lib --bins -- --test-threads=1`（确认暂停点未装载时既有行为不变，仍 GREEN）。
- Stop when: 暂停点无法在目标位置停住写者；或 RED 不稳定复现（⇒ 说明触发方案不成立，返回 Plan）。

### T2: 索引写单元原子化与写侧串行化

- Requirement/Scenario: `index-write-side-atomicity` — 全部四个 requirement（写单元不可分割、写侧串行化、键全释放、结构一致）。
- Depends on: T1
- Targets: `src/storage/btree/index_manager.rs`（`IndexManager::insert` / `delete` / `delete_exact` / `update`、`row_to_key` 的锁类型、写锁字段）、`src/storage/btree/btree.rs`（只读依赖；除非 T1 的暂停点语义位置无法在 `IndexManager` 内表达，否则不改）。
- Current behavior: 结构变更、根页 id 发布、反向映射更新分三段 await 写入；两个写者可并发改同一页并互相覆盖根页 id；`delete` 先删反向映射后删正向条目。
- Required behavior: 结构变更 + 根页 id 发布 + 反向映射更新在同一个 `spawn_blocking` 闭包内完成（不可被取消分割）；同一索引的结构变更串行；`delete` 先删正向条目后删反向映射。
- Preserve: `IndexManager` 的公开读写语义（`search` / `find_key_by_row_id` / `scan_all` / `delete_exact` 的精确删除语义）逐条不变；`delete_exact` 仍不换根；`IndexManager::insert` 对重复键的既有错误语义不变；MS13（不跨 await 持 std 锁）；暂停点的语义位置与装载入口保持不变。
- Forbidden: 不改 `BTree` 的分裂/合并/换根算法；不改 `statement-cancellation-outcome` 的取消语义；不改 `abort_cleanup_versions` 的分桶逻辑；不引入身份证明机制；不做与 ISS20 无关的重构。
- First test: 沿用 T1 的用例（它此时应转 GREEN）；先确认 RED（`cargo test --test index_write_atomicity_test -- --test-threads=1` 失败于「键不可达 / 键残留」），再实施。
- GREEN condition: T1 用例通过（键 1..92 全部可查、键 93 可再次插入、完整性无 Error、进程未 abort）；`cargo test --test statement_cancellation_test -- --test-threads=1` 仍全绿（既有取消语义未回归）。
- Verification: `cargo test --test index_write_atomicity_test -- --test-threads=1`；`cargo test --test statement_cancellation_test -- --test-threads=1`；`cargo test --lib --bins -- --test-threads=1`。
- Stop when: 需要改变 `BTree` 的结构语义或改变公开索引 API 的错误语义才能达成（⇒ 返回 Plan）；或写单元的不可中断性无法在不引入身份机制的前提下达成（⇒ 返回 Plan）。

### T3: 取消/回滚后的键全释放与索引一致性端到端回归

- Requirement/Scenario: `index-write-side-atomicity` — 「被取消语句写入的第二个键也能再次插入」「隐式事务路径释放全部键」「取消或回滚之后索引结构保持一致」「并发写入不产生结构分叉」。
- Depends on: T2
- Targets: `tests/index_write_atomicity_test.rs`（扩展）、`tests/statement_cancellation_test.rs`（扩展既有取消用例的断言）。
- Current behavior: 既有取消用例只复用**首个**键（ISS20 记录），因此没有暴露「第二个键残留」；也没有断言取消后索引的结构完整性。
- Required behavior: 覆盖显式事务路径与隐式事务路径、RR 与 RC：取消一条多行 `INSERT` 后回滚/回收，该语句写入的**每一个**键都可再次插入；取消之后同一会话的后续写语句不 abort；`check_integrity` 无 Error 级发现。
- Preserve: MS34 已验收的既有断言逐条不变（只增不减）；既有夹具形态（单用例自带独立 `TempDir` 现场，不依赖并行度）不变。
- Forbidden: 不改 MS34 取消域的语义断言；不引入身份证明机制或判定层；不把全量回归放进本任务（全量预算在 Iteration 001）。
- First test: 在 `tests/index_write_atomicity_test.rs` 新增「显式事务内取消多行 INSERT → 回滚 → 全部键可再次插入」「隐式路径回收后全部键可再次插入」两组用例；变更前观察 RED（第二个键报重复键）。
- GREEN condition: 新增用例与既有 `statement_cancellation_test` 全部用例通过。
- Verification: `cargo test --test index_write_atomicity_test -- --test-threads=1`；`cargo test --test statement_cancellation_test -- --test-threads=1`。
- Stop when: 发现新的、与 ISS20 同根因但超出本 change 范围的实质缺陷（⇒ 记为 Issue 候选上报，不在本 Cycle 扩范围修复）。

**Invariants**

- 语句被取消时，索引写单元整体落地或整体未开始（不产生半写状态）。
- 同一索引的结构变更串行执行；并发写者不交错修改同一索引页。
- `IndexManager` 的既有公开语义（精确删除、重复键错误、扫描结果）逐条不变。
- M13：异步路径不持 `std::sync` 锁跨 `.await`。
- M19：全量回归分批执行（本 Cycle 不跑全量；预算在 Iteration 001）。
- 验证只落在 `cargo test` 集合内；无身份证明机制、无判定层、无 timer 阈值作行为判据。
- MS34 已验收的取消语义（丢弃/超时是取消触发源；断开不是）不变。

**Non-goals**

- DDL 目录面（Iteration 001）。
- 空闲页跨重启复用（MS35）、历史库与 dump 兼容（MS36）、分级诊断系统（I096）。
- `abort_cleanup_versions` 分桶的额外加固（根因已在源头消除）。
- 修复本 change 之前已经损坏的索引。
- 取消/超时的事务收尾语义面（MS34 已验收）。

**Acceptance**

1. **A1**（T1+T2，`index-write-side-atomicity`「写在结构变更与根页 id 发布之间被取消时索引保持自洽」）：装载写侧暂停点 → 第 93 个键触发换根 → 取消 → 放行 → 回滚后键 1..92 全部可查、进程未 abort。**验证**：`cargo test --test index_write_atomicity_test -- --test-threads=1`。
2. **A2**（T1+T2，「被取消语句写入的第二个键也能再次插入」）：同一取消场景后，被取消语句写入的键可再次插入且不报重复键。**验证**：同上命令 + 断言输出。
3. **A3**（T2+T3，「并发写入不产生结构分叉」）：两个写者并发写入后全部键可查、完整性无 Error。**验证**：`cargo test --test index_write_atomicity_test -- --test-threads=1`。
4. **A4**（T3，「隐式事务路径释放全部键」）：隐式路径回收后全部键可再次插入。**验证**：`cargo test --test index_write_atomicity_test -- --test-threads=1`。
5. **A5**（T3，「取消或回滚之后索引结构保持一致」）：取消回滚后同会话写语句不 abort、`check_integrity` 无 Error 级发现。**验证**：`cargo test --test index_write_atomicity_test -- --test-threads=1` 与 `cargo test --test statement_cancellation_test -- --test-threads=1`。
6. **A6**（T2/T3 回归）：既有 `statement_cancellation_test` 全部用例保持 GREEN；`cargo test --lib --bins -- --test-threads=1` 保持 GREEN。

**Verification**

- A1/A2/A3/A4/A5：`cargo test --test index_write_atomicity_test -- --test-threads=1`（单目标定向运行，按 M19 可用默认并行度；本 Cycle 统一带 `--test-threads=1`）。
- A5/A6：`cargo test --test statement_cancellation_test -- --test-threads=1`；`cargo test --lib --bins -- --test-threads=1`。
- 触发窗口的锚点 = 写侧暂停点的「到达」信号（可观察状态 + `yield_now` 轮询）；超时只作夹具失控兜底（触发即失败），**不作为行为判据**。
- 端到端场景（取消 → 回滚 → 后续写 → 键复用）走真实入口：`Database::execute_sql` 实际执行并丢弃语句 future，断言用户实际所见（重复键错误、查询结果、进程存活）。
- 全量回归**不在本 Cycle**——按 tasks.md 的 Iteration Plan 记在 Iteration 001 收尾。

**Gate 2 Readiness**

PASS。Gate 2 检查项：Step 5 计划审查全部通过（无 TBD/TODO、无未批准裁剪、tasks/specs/design 一致、Current-State Evidence 来自实际代码与已运行结论、任务含完整执行依据、调用者与测试入口已定位、无无关范围、Iteration Plan 覆盖全部任务并通过平衡审计且高开销验证预算明确、Plan Context 自包含、非实质未知项留在 Risks、验证直接证明目标行为且无身份机制/判定层）；RTM 无 `Missing`、无 `Simplified`；设计闭合（行为差异、接口、错误语义、关键选择已明确）；当前 Cycle 与 change 的 tasks/specs/design 一致；Persisted Evidence = `none`；**用户已批准计划**（依据 goal 指令原文，见 proposal「授权与批准依据」）。

**Persisted Evidence**

- Mode: none

理由：Act Response 足以承载验证结果（命令、实际结果、修改文件与符号）。本 Cycle 的判定全部来自 `cargo test` 的原生输出与退出码，无额外证据需求；`required` 的四个问题（支持哪个 Acceptance／为什么 Act Response 不够／为什么无法低成本重跑／缺少它会阻止哪个决定）中「Act Response 为什么不够」无法回答 ⇒ 按公共规则取 `none`。

**Risks and Notes**

- **触发方案的诚实边界**（非实质未知项，Act 不需要决定接口或错误语义）：两个写者对同一索引页交错修改需要真正的时间重叠，本 change 的确定性用例**不**强制该重叠；该风险由写侧串行化从设计上消除，其行为面断言落在 A3/A5。
- 写侧暂停点是**测试支持夹具**，形态对齐既有 `Database::arm_io_failure` 先例；未装载时不得有任何行为差异（Act 在 T1 验证）。
- 反向映射容器由 `tokio::sync::RwLock` 改为 `std::sync::RwLock` 会触及 `find_key_by_row_id` 等调用点；`find_key_by_row_id` 的公开签名保持 `async`，调用方不需要改（Act 可在实现中确认调用点清单）。
- 目录根同步（`sync_root_to_catalog`）留在写单元之外（持久化关注点）；其失败语义与既有实现一致（返回错误）。
- 若 T2 的实现发现「写单元的不可中断性」与「目录根同步必须在单元内」同时为必需（接口或错误语义变化）——**停止并返回 Plan**。
- 范围外缺陷：Act 在本 Cycle 发现的实质缺陷按公共规则作为 Issue 候选**只报告不登记**。
- 全量回归预算：整个 change 一次，记在 Iteration 001（tasks.md 已写明 M19 分批纪律）。

## Act Response

- Status: reported

**Implemented**

- **T1 — 写侧暂停点夹具（测试支持入口）+ 先行 RED 用例**：
  - `src/storage/btree/index_manager.rs`（新增，夹具形态对齐既有 `Database::arm_io_failure` 先例）：
    - `WritePausePosition`（唯一变体 `AfterStructureChangeBeforeRootPublish`）——语义位置固定为「结构变更已生效、根页 id 尚未发布」，**修复前后不变**；不携带任何运行身份（无 run-id / 指纹 / 时间戳）。
    - `WritePauseInner`（到达计数 `arrivals`、已放行计数 `released`、放行 `Condvar`）、`WritePauseHandle`（公开：`arrivals()` 轮询「到达」信号、`release()` 放行当前到达、`disarm()` 卸载）、`WritePauseSlot`（`Arc<Mutex<Option<Arc<WritePauseInner>>>>`）、`pause_at()`。
    - `IndexManager::arm_write_pause(position) -> WritePauseHandle`：装载暂停点。**未装载时零行为差异**：写单元只多做一次未命中检查（`pause_at` 立即返回），其余路径与变更前逐条相同。
  - `src/storage/btree/mod.rs`、`src/storage/mod.rs`：导出 `WritePauseHandle` / `WritePausePosition`（既有公开 API 的语义与签名不变）。
  - `tests/index_write_atomicity_test.rs`（新建，5 个用例）：`leaf_fills_at_92_rows_and_the_93rd_key_splits_the_root`（夹具几何自检）、`cancelled_write_keeps_index_self_consistent`、`concurrent_writers_do_not_fork_the_tree`、`cancelled_multi_row_insert_in_explicit_tx_releases_every_key`、`cancelled_multi_row_insert_in_implicit_tx_releases_every_key`。
- **T2 — 索引写单元原子化 + 写侧串行化**（`src/storage/btree/index_manager.rs`）：
  - 字段形态：`root_page_id: Arc<AtomicU64>`、`row_to_key: Arc<std::sync::RwLock<HashMap<RowId, Vec<u8>>>>`（由 `tokio::sync::RwLock` 改同步容器）、新增 `write_gate: Arc<Mutex<()>>`（写侧闸门）。
  - `insert`：结构变更 → 根页 id 发布（`root_page_id.store`）→ 反向映射更新**三者同在同一个 `spawn_blocking` 闭包内**；闸门在闭包内部获取与释放（闭包内不跨 await，M13）；根页 id 的读取也移入闭包（在闸门内读，避免并发写者读到彼此的中间根）；目录根同步 `sync_root_to_catalog` 留在写单元之外（既有持久化语义不变）。
  - `delete`：正向条目删除与反向映射清理同单元，顺序改为「先删正向条目、后删反向映射」；正向条目定位改用闭包内 `BTree::search`（与既有 `IndexManager::search` 同语义：精确匹配）。
  - `delete_exact`：结构变更与「仅当仍指向本键时才清理」的反向映射清理同单元；精确删除语义不变。
  - `update`：结构变更与反向映射更新同单元。
  - `find_key_by_row_id`：公开签名保持 `async` 不变；内部改为同步加锁，并**与写单元串行**（读到的反向映射是写单元落定后的状态；等待发生在 blocking 线程上，异步运行时线程不被阻塞、不跨 await 持锁）。
- **T3 — 取消/回滚后的键全释放与索引一致性端到端回归**：
  - `tests/index_write_atomicity_test.rs`：多行 INSERT 在显式事务内取消 → 回滚 → 全部键可再次插入；隐式路径取消 → 回收 → 全部键可再次插入；并发写者不产生结构分叉；取消后键 1..92 全部可查、键 93 可再次插入、完整性无 Error 级发现。
  - `tests/statement_cancellation_test.rs`（**只增不减**既有断言）：隐式回收用例追加「第二个键可再次插入」「索引里只剩重新插入的两个键（被放弃语句写入的 2000 个键全部已释放）」「完整性检查无 Error 级发现」；重开用例追加「第二个键可再次插入」；显式事务回滚用例与超时回滚用例追加「第一个键与第二个键可再次插入」及完整性断言；两处描述旧残留边界的注释改为描述修复后的事实（MS34 已验收的断言本身未改）。

**Deviations from Plan**

1. **夹具 API 增加 `disarm()`**（Plan 只写「装载 / 到达 / 放行」）：暂停点若不卸载，取消之后的后续写（回滚/回收触发的 `delete`、断言用的 `INSERT`）会再次被停住而挂死。`arm_write_pause(position) -> handle` 与「到达 = 可轮询计数 / 放行」按 Plan 原样实现，`disarm()` 是让夹具可用的必要补充。
2. **Plan 对单键场景的 RED 预测与实跑不同（事实修正，非实现偏差）**：Plan 的 T1「Required behavior」预测单键场景下「键 93 再次插入报重复键」。实跑为：换根丢失后该键落在**不可达**的右半叶，重复键检查走索引查找（找不到）⇒ 不报重复键；该场景的 RED 落在设计预测 (a)「键 47..92 不可达」上。「第二个键报重复键」出现在多键场景（E2 残留），与设计预测 (b) 一致。Plan 的断言集未因此放宽——T1 的完成判据「夹具在位 + 用例稳定 RED」按实跑达成。
3. **反向映射读与写单元串行（`find_key_by_row_id` 加闸门）**：Plan 的 D1–D5 未规定这一条。实测发现：即使写单元已原子化，**回滚/回收的分桶读**仍可能读到写单元尚未落定的中间态（回收在取消后立刻发生时稳定命中），导致被取消语句写入的键被误判为墓碑而不释放——即 Plan 的 Objective「被取消语句写入的**全部**键在回滚后释放」不成立。该改动落在 T2 的 Target 文件内，不改公开 API 的签名与返回值、不改 `abort_cleanup_versions` 的分桶逻辑、不新增公共 API；分类为**Plan 遗漏（实质）**，请 Plan Review 裁定：若 Review 认为超范围，需以 replan 收回（则 Objective 的「全部键释放」随之回退）。
4. **T3 对既有 MS34 用例的扩展含注释更新**：两处注释描述「回滚后被取消语句写入的键仍留在正向条目里」的旧边界（修复后不再成立），改为描述修复后的事实；既有断言逐条未动。
5. **验证预算**：按 change 预算，本 Iteration **未跑全量回归**（收尾全量在 Iteration 001 的 T6 执行，M19 分批串行）。本 Cycle 跑的是定向目标与受影响范围目标。

**Blocker Handoff**

None required — 本 Cycle 未命中 Gate 5。

**Blocker Resolution**

None — 本 Cycle 未曾阻塞。

**Self-Review**

- **Gate 3（Task Review）逐项结论**：
  - **T1 — Spec compliance：PASS**。夹具的语义位置与 Plan 的 D4 一致（结构变更已生效、根页 id 尚未发布），且修复前后同位；到达信号是可轮询计数、放行是 `Mutex<u64>` + `Condvar`（阻塞线程等待），未引入身份机制与判定层；未装载时零行为差异由 `cargo test --lib --bins`（420 passed）见证。先行测试先于产品改动存在，并在真实修复前行为上稳定 RED（3 次连续实跑全部命中产品断言，非编译错误、非夹具错误、非锚点超时）。
  - **T1 — Code quality：PASS**。夹具只在写单元内多一次未命中检查；无新增编译/clippy 告警（lib 的 14 条 clippy 告警与 MS34 基线同数，且全部落在既有代码上）；`cargo fmt --check` 对改动文件无 diff。
  - **T2 — Spec compliance：PASS**。写单元不可分割（结构变更 + 根页 id 发布 + 反向映射更新同生共死）、同一索引结构写侧串行、`delete` 的「先正向后反向」顺序、反向映射容器改同步 `RwLock` 且 `find_key_by_row_id` 签名不变——逐条对应 D1–D5；未改 `btree.rs`（仅读）、未改 `abort_cleanup_versions` 的分桶逻辑、未新增公共 API。
  - **T2 — Code quality：PASS**。闸门在闭包内获取与释放，不跨 await（M13）；反向映射读写锁同样不跨 await；错误、边界与状态生命周期正确（结构变更失败时不发布新根、不更新反向映射；`delete_exact` 的条件清理语义保留；目录根同步失败仍返回既有错误）。
  - **T3 — Spec compliance / Code quality：PASS**。显式与隐式两条路径、RR 与 RC 两种隔离级别均有覆盖；断言全部落在可观察状态上（索引查找、SQL 执行结果、完整性报告、`active_transactions`），判定层在 `cargo test` 内。
  - **未解决的 Critical / Important 问题：无。**
- 自检中发现并已修复：
  1. **测试侧的键解码错误**：并发用例最初按 `i64::to_be_bytes` 解码索引条目，但产品键的 `Key::deserialize` 按尾部零扫描推断长度（256/1024 等值的键长为 7）⇒ 用例在 `range end index 8 out of range for slice of length 7` 处失败（夹具错误，非产品 RED）。改为用产品的 `search` 逐键断言 + 条目总数断言（不再自行解码键）。
  2. **回收与写单元尾部交错（实质，已修）**：写单元原子化之后，显式事务回滚用例仍稳定报 `Duplicate key`；以「回滚前插入 200ms 延迟」的对照实验确认根因是回收的分桶读与写单元尾部交错（延迟后通过）⇒ 引入反向映射读与写单元串行（Deviations 3）。
  3. **既有 MS34 用例注释描述的旧边界已不成立**：两处注释按修复后的事实更新（断言本身未动）。
  4. **并发用例的失败计数随运行波动**（574/551/554，均 < 600）：属并发固有的写入丢失比例差异，不影响判据（断言是「恰好 600 条目且 600 个键全部可查」）；已确认失败稳定复现而非偶发。
- 遗留 Minor 问题（不阻塞，未修）：
  1. 目录根同步（`sync_root_to_catalog`）仍在写单元之外：两个写者并发发布时，目录里持久化的根可能落后于内存根。既有语义不变（Plan 的 D1 明确保留），且 WAL 重放路径重建索引，观测影响有限；本 Cycle 未复现，未登记为 Issue。
  2. `find_key_by_row_id` 现在经 `spawn_blocking`（每行一次线程跳转）：回滚/回收与 WAL 恢复的逐行分桶路径多一次跳转开销（本会话实测 `statement_cancellation_test` 8 用例 35–55s，基线 43s，波动来自新增的完整性断言）。
  3. 反向映射读与写单元的串行依赖 `std::sync::Mutex` 的等待顺序近似 FIFO：若被取消语句的写单元尚在「已 spawn 未取到闸门」状态，理论上存在极小的取闸门顺序倒置可能（未观测）。
  4. 写侧暂停点的 `disarm()` 需要测试显式调用；未卸载会让后续写被停住（本 Cycle 已在夹具内固化为取消流程的一步）。
- 无身份证明机制（无 Hash / 指纹 / run-id / session-id / revision pin / manifest / 时间顺序证明及其采集工具），无项目测试框架之外的判定层：全部断言在 `cargo test` 内，观察点为暂停点到达计数、`IndexManager::search` / `scan_all` / `find_key_by_row_id` 的可观察结果、SQL 执行结果与完整性报告。
- 改动文件：`src/storage/btree/index_manager.rs`、`src/storage/btree/mod.rs`、`src/storage/mod.rs`、`tests/index_write_atomicity_test.rs`（新建）、`tests/statement_cancellation_test.rs`（扩展）、change 的 `tasks.md`（T1/T2/T3 置 `done`）与本 Cycle 的 Act Response。工作区里 MS34 的既有现场（`src/database.rs`、`src/pipeline.rs`、`src/executor/alter_table.rs`、`src/transaction/manager.rs`、`.agents/**`、`openspec/specs/**`）未回滚、未清理、未提交。

**Verification Evidence**

- **Gate 4（Verified Completion）结论**：PASS——Task Definition 的 Verification 全部满足（T1 的 RED 命令退出码非 0 且失败信息命中产品断言；T1/T2/T3 的 GREEN 命令退出码均为 0；T2 的 GREEN condition 两个目标退出码 0；受影响范围目标全部 0 failed）。完成声明基于下表的当次真实运行；MS34 收尾全量终态（2163 passed）与既有结论（`statement_cancellation_test` 8、`explicit_tx_test` 21、`mvcc_abort_test` 3、`mvcc_commit_test` 4、`dml_tx_id_test` 6、`concurrent_test` 10、`prepared_statement_test` 14）在本会话中被覆盖范围未变地重新实跑并沿用。

| 验证 | 命令 | 实际结果 |
|---|---|---|
| T1 RED（产品改动前，3 次连续） | `cargo test --test index_write_atomicity_test -- --test-threads=1` | 退出码 101；每次 `test result: FAILED. 1 passed; 4 failed`。失败点均为产品断言：① `key 47 must stay reachable in the index after the cancelled write`（换根丢失，键 47..92 不可达）；② 显式事务用例 `INSERT INTO t VALUES (2) must succeed with affected rows, got Error { message: "Execution error: Duplicate key" }`；③ 隐式路径用例同一 `Duplicate key`；④ 并发用例 `the index must hold exactly one entry per row written by the concurrent writers`（实测 574/551/554，均 < 600）。几何自检用例 3 次均通过（单叶恰好装满 92 行、第 93 个键换根）。**RED 稳定复现**（3/3） |
| T2 中间 RED（写单元原子化后、反向映射读串行前） | 同上，显式事务用例单跑 | 仍为 `Duplicate key` ⇒ 证明残留面来自回收的分桶读与写单元尾部交错，而非写单元本身 |
| T1–T3 GREEN（5 次连续） | `cargo test --test index_write_atomicity_test -- --test-threads=1` | 退出码 0；每次 `test result: ok. 5 passed; 0 failed` |
| T2 GREEN condition | `cargo test --test statement_cancellation_test -- --test-threads=1` | 退出码 0；`test result: ok. 8 passed; 0 failed` |
| T1「未装载零行为差异」 | `cargo test --lib --bins -- --test-threads=1` | 退出码 0；`test result: ok. 420 passed; 0 failed`（与 MS34 基线 420 一致） |
| 受影响范围（定向，非全量） | `cargo test --test <target> -- --test-threads=1`，逐目标独立执行：`index_manager_test`(3)、`btree_test`(10)、`btree_split_test`(12)、`btree_merge_test`(10)、`btree_scale_test`(5)、`constraint_enforcement_test`(26)、`explicit_tx_test`(21)、`mvcc_abort_test`(3)、`mvcc_commit_test`(4)、`dml_tx_id_test`(6)、`concurrent_test`(10)、`prepared_statement_test`(14)、`insert_select_test`(10)、`key_type_conformance_test`(8)、`isolation_level_test`(9) | 退出码均为 0；15 个目标合计 171 passed / 0 failed；既有基线 21/3/4/6/10/14 与 MS34 一致 |
| 静态检查 | `cargo fmt -- --check`、`cargo clippy --all-targets` | 格式化无 diff；clippy 的 lib 告警 14 条与 MS34 基线同数，且均不落在本次改动的文件（`index_manager.rs` / `index_write_atomicity_test.rs` / `statement_cancellation_test.rs` 无告警） |

说明：`cargo fmt` 只改了本次新增代码的行宽（`index_manager.rs` 的字段对齐与 `update` 闭包换行、并发用例的 `assert!` 换行），不触语义；格式化后重跑了本表 GREEN 命令中的三个主目标与受影响范围目标，结果与上表一致——上表结论基于最终代码状态。

**Persisted Evidence**

None — 模式为 `none`。本 Cycle 的验证结论由上表的真实运行结果承载：全部命令可低成本重跑（定向目标，秒级到分钟级），没有一次性环境即将消失的现场，也没有摘要会丢失决定性结构的长输出（RED/GREEN 的失败与通过信息均为短输出，已在本表逐条摘录）。本 Cycle 未创建 `evidence/` 目录。

**Experience Candidates**

1. **（可承载：Runbook）索引写侧原子性的确定性复现配方**
   - 结论：索引写侧的「结构变更已生效、根页 id 尚未发布」窗口可以用**写侧暂停点**确定性地停住并复现——不需要依赖两写者交错时序，也不需要多次尝试碰运气。
   - 根因位置：`src/storage/btree/index_manager.rs`（`IndexManager::insert` / `delete` 的写单元；暂停点装载入口 `arm_write_pause`）。
   - 复现路径或证据：本 change 的 `tests/index_write_atomicity_test.rs`（5 个用例）——建单列主键表 → 插 92 行（单叶恰好装满）→ 装载暂停点 → 后台插第 93 个键（触发叶分裂与换根）→ 轮询到达计数 → 丢弃语句 future（取消）→ 放行 → 下一条写语句触发回收 → 断言键 1..92 全部可查、键 93 可再插、完整性无 Error。实跑 3/3 稳定命中修复前的产品断言（键 47..92 不可达 / 第二个键报重复键 / 并发写者丢失 26–49 个条目）。
   - 影响面与触发条件：任何「语句在索引结构变更中途被取消」的路径（客户端断连、超时、调用方丢弃语句 future）；修复前表现为键不可查、键残留（误报重复键）、并发写者丢失条目。
   - 与本 change 范围的关系：本 change 的 T1/T2/T3 已在该范围内完成（夹具 + 写单元原子化 + 键释放）；配方本身可复用于后续同类缺陷的复现。
   - 修复方向：不适用（这是复现配方，不是缺陷修复）。
   - 承载落点：Recorder 新建 Runbook（`.agents/runbooks/`）记录「写侧暂停点复现配方：装载/到达计数/放行/卸载、取消手法（丢弃语句 future）、回收触发点、四段判据（键可达性、重复键、并发条目数、完整性）」，并登记 R 引用；ISS20 的处置指针由 Recorder 在本 change 收尾后更新。

**Remaining Issues**

- 无未解决的 Critical / Important 问题；遗留 Minor 问题四条见 Self-Review。
- 本 Cycle 未跑全量回归——按 change 预算，整个 change 一次，作为收尾验证由 Iteration 001 的 T6 执行（M19 分批串行：每批一次独立 `cargo test --test <f1> ... -- --test-threads=1`、批间顺序执行、禁止无目标过滤的全量调用、禁止默认并行度跑批、批 8–12 目标、记各批 passed/failed 并做覆盖对账）。
- 停止条件：当前 Cycle 的 T1/T2/T3 全部完成，Gate 3 / Gate 4 逐项通过，`Act Response` 状态置 `reported`。本 Cycle 未创建下一 Cycle 或下一 Iteration，未归档 change，未登记或修改任何 Issue。本 Cycle 未修改全局文档与规格（`.agents/docs/tasks.md`、`.agents/docs/SNAPSHOT.md`、项目记忆、`openspec/specs/` —— 工作区里这些文件的既有改动属 MS34 收尾现场，非本 Cycle 产生）。
- 下一步由用户审计，并调用 `openspec-plan` 对本 Cycle 做 Plan Review（重点裁定 Deviations 3：反向映射读与写单元串行是否属于本 change 范围）。

## Plan Review

- Review Result: accepted

**Findings**

> 独立审查方式：完整阅读本次改动的 diff（`src/storage/btree/index_manager.rs`、`src/storage/btree/mod.rs`、`src/storage/mod.rs`、`tests/index_write_atomicity_test.rs` 全文、`tests/statement_cancellation_test.rs` 的增补断言段）、change 的 `proposal.md` / `design.md`（D1–D7）/ `tasks.md` / `specs/index-write-side-atomicity/spec.md`，以及 ISS20 / ISS21 的缺陷正文；**未重跑已通过的验证**（公共规则 › 验证：Plan Review 的独立检查 = 独立读码与核对，不含重复运行已沿用的验证）。

**F1 — Deviation 3 裁定：Plan 遗漏，但在已批准 requirement 之内 → 接受，design 回写（D2 / D3 已修订）。**

- 分类确认：Plan 的 D1–D5 未规定「反向映射读与写单元串行」；但 Act 的实现落在 T2 的 Target 文件内（`IndexManager::find_key_by_row_id`），且 Plan 的 Implementation Guidance 与 Risks 已明确把该符号纳入本 Cycle 的改动面（「`find_key_by_row_id` 的公开签名保持 `async`，内部改同步加锁」；Risks 第 3 条列明容器改同步 `RwLock` 会触及 `find_key_by_row_id` 等调用点）⇒ **符号与改动面在 Plan 内，缺的是「读侧必须与写单元串行」这条排序约束** ⇒ 属 **Plan 遗漏**，不是 Act 偏离、不是扩范围。
- 需求内判定：Act 的中间态 RED 已证明「D1 的写单元原子化单独不足以兑现 Plan 的 Objective『被取消语句写入的**全部**键在回滚后释放』」——回滚/回收的分桶读若读到写单元尚未落定的中间态，被取消语句写入的键会被误判为墓碑而不释放（显式事务用例稳定报 `Duplicate key`）。串行化是兑现已批准 requirement `index-write-side-atomicity` R3 的**最小机制**：不改变 R3 的语义、不扩大保证面、不新增公共 API、不改 `abort_cleanup_versions` 的分桶逻辑（`find_key_by_row_id` 的签名 `async -> Option<Vec<u8>>` 与返回值不变）。
- 并发/死锁核对（独立读码）：锁序统一为 **写侧闸门 → 反向映射 `RwLock`**，五处（`insert` / `delete` / `delete_exact` / `update` / `find_key_by_row_id`）无一处反序；无 std 锁跨 `.await`（M13）；`find_key_by_row_id` 的等待落在 blocking 线程（`spawn_blocking`），不阻塞 Tokio worker；该方法的全部调用点（`src/transaction/manager.rs:347/383/422/471`、`src/wal/recovery.rs:984`）都在 async 函数里 ⇒ **不存在「blocking 闭包内再 await」**。结论：无新增死锁面、无新增锁反转。
- 处置：**不需要 replan**（目标、范围、依赖、requirement、设计方向、验证方式、验收边界均未改变）；design 的长期技术记录已就地回写（D2 增补「读侧同闸」，D3 增补「D1 单独不足」与本裁定）。

**F2 — `Arc` 化后的克隆语义：共享边界正确，无结构分叉。** `IndexManager` **没有 `Clone` 实现**，全仓只以 `Arc<IndexManager>` 持有（`src/storage/data/table_manager.rs:95` → `TableManager` → `Database` 的 `Arc<TableManager>`）。`Database` 虽 `#[derive(Clone)]`（`src/database.rs:109-111`），但其字段全为 `Arc` ⇒ `db.clone()` 共享同一 `TableMeta` ⇒ 同一 `IndexManager` ⇒ 同一 `Arc<AtomicU64>` 根页 id、同一 `Arc<RwLock>` 反向映射、同一 `Arc<Mutex>` 闸门。三个 `Arc` 包装的唯一作用是把状态**移入** `spawn_blocking` 闭包（`AtomicU64`/锁不可 `Copy`），不产生「克隆句柄各持一份根页 id」的路径。独立复核：全仓无 `impl Clone for IndexManager`、无 `IndexManager` 的按值克隆调用。

**F3 — `delete` 顺序反转为「先正向、后反向」：既有语义不变，错误路径只变得更一致。** 独立读码确认 `BTree::search` 与 `BTree::delete` → `LeafNode::delete` 都是**精确匹配**（`LeafNode::delete` 对非精确键返回 `KeyNotFound`）⇒ 两者解析到同一位置 ⇒ 反向映射清理的目标条目与被删的正向条目一致。其他调用方（`abort_cleanup_versions` 的 A 趟、WAL redo 的 Delete 重放、`alter_drop_column` 的 DELETE 腿）语义不变。**唯一的语义差异在错误路径**：`btree.delete` 失败时反向映射**不再**被清除（变更前「先清反向映射、再删正向条目」，失败也清了）⇒ 该行从「有反向映射、无正向条目」变为「两者都在位」，由回收路径删除条目——两种终态都自洽，变更后更接近「正向条目在位 ⇔ 反向映射在位」的不变式。记为 Minor（错误路径差异，无调用方依赖旧形态）。

**F4 — `find_key_by_row_id` 保留 `async` 签名：形态正确。** 公开签名与返回值不变，调用点零改动。内部改为 `spawn_blocking` + 同步加锁：等待落在 blocking 池线程，不占 Tokio worker；无 blocking 闭包内 await（调用点核对见 F1）。**Minor**：新增两条 panic 路径——`JoinHandle::unwrap()`（blocking 任务 panic 时向上传播）与 `std::sync::RwLock` 的**中毒**语义（写侧 panic 后 `row_to_key.write().unwrap()` 会使后续所有读 panic；`tokio::sync::RwLock` 不中毒）。写侧临界区只包一次 `HashMap` 插入/删除，实际触发概率极低，不阻塞 Acceptance。

**F5 — 写闸临界区覆盖完整闭环，根页读移入闭包消除了中间根窗口。** 闸门在闭包**首行**获取（`insert`/`delete`/`delete_exact`/`update`）⇒「读当前根 → 结构变更 → 发布根页 id → 更新反向映射」全程在闸内；根页 id 的读取（`root_page_id.load`）已移入闭包并在闸内 ⇒ 并发写者不可能读到彼此的中间根，「树已换根、已发布根仍是旧根」的窗口（D1 的核心失效模式）被消除。`sync_root_to_catalog` 仍在闭包外（D1 明确保留）——见 F12。

**F6 — 暂停点夹具：零行为差异成立，无全局污染，无身份机制。** 未装载时 `pause_at` 只做一次 `Mutex` 取锁 + `None` 判定即返回（写路径其余部分逐条不变），由 `cargo test --lib --bins` 420 passed（= MS34 基线）见证。夹具槽位是**每 `IndexManager` 一份**（非全局静态）⇒ 不跨用例污染；`disarm()` 用 `Arc::ptr_eq` 只清自己的槽位项 ⇒ 重新装载不会被误清；位置枚举只有一个变体，不携带任何运行身份。**Minor**：`disarm()` 需测试显式调用（Act 已列）——本 Cycle 的 `cancel_at_arrival` 夹具把「放行 + 卸载」固化为取消流程的一步，跨用例无残留。

**F7 — 无身份证明机制、无框架外判定层。** 全量阅读改动文件：位置枚举、到达计数、放行计数三者都不含 Hash/指纹/run-id/session-id/revision pin/manifest/时间戳；timer 只出现在测试夹具的失控兜底（`ANCHOR_TIMEOUT` 超时即 `expect` 失败，触发即判失败，不是行为判据）；判定全部在 `cargo test` 内，观察点是 `IndexManager::search` / `scan_all` / `find_key_by_row_id` 的可观察结果、SQL 执行结果与 `check_integrity` 的 Error 计数。无新增脚本、封装命令或专用参数。

**F8 — 四条 requirement 的判别力逐条核对（不是「用例通过」，而是「用例能区分修复前后」）。**

- R1（写单元不可分割）→ `cancelled_write_keeps_index_self_consistent`：暂停点把「结构变更已生效、根页 id 尚未发布」钉成确定窗口 ⇒ 修复前**必然**出现「键 47..92 落在丢失的右半叶、不可达」（实跑 RED 命中该产品断言）；修复后键 1..92 全部可查、键 93 可再插、完整性无 Error。判别力来自夹具的确定性，不依赖两写者交错时序。
- R2（结构写侧串行化）→ `concurrent_writers_do_not_fork_the_tree`：两个写者并发写 600 个键；修复前实测 574/551/554 条（< 600，丢失更新）⇒ 判据「恰好 600 且 600 个键全部可查」能稳定区分；修复后由闸门保证恰好 600。计数波动是并发固有的丢失比例差异，不影响判据。
- R3（全部键释放）→ `cancelled_multi_row_insert_in_explicit_tx_releases_every_key` + `..._implicit_tx_...`（两个键都可再插）+ `tests/statement_cancellation_test.rs` 的增补断言（隐式回收路径：第二个键可再插、索引里只剩重新插入的 2 个键 ⇒ 被放弃语句写入的 2000 个键全部已释放、完整性无 Error；显式回滚与超时回滚路径：第一个键与第二个键都可再插 + 完整性断言）。修复前这些断言命中 `Duplicate key`。
- R4（结构自洽）→ 5 个新用例与增补断言均带 `check_integrity` 的 `error_count() == 0` 断言；E1 的进程 abort 形态在修复后不再出现（用例能跑完即为观察）。

判别力的一处诚实边界：Act 报告的 RED 是**变更前**的 3 次连续实跑（1 passed / 4 failed，失败全在产品断言上），修复后 5 次连续 GREEN；本次 Review 未重跑（公共规则不重复已通过验证），结论由「Act 的运行记录 + 独立读码确认失效机制」共同支撑。

**F9 — 规格增量 `specs/index-write-side-atomicity/spec.md` 准确，无「未验收写成已验收」。** 逐条核对：R1 + 场景 → T1/T2 的单键取消用例；R2 + 场景 → 并发用例；R3 + 两个场景 → 显式/隐式两用例 + MS34 增补断言；R4 + 场景 → 完整性断言（5 个新用例 + 增补断言）。每条 Requirement 的陈述都有本 change 的用例覆盖；没有把 D1 保留项（目录根同步在写单元外）写成已验收。规格措辞只描述可观察行为（键可达性、重复键、完整性报告），不含实现手段。⇒ 可并入 `openspec/specs/`。

**F10 — Minor（不阻塞）：`tests/statement_cancellation_test.rs:794-797` 的注释仍描述修复前的旧边界。** 该段仍写「被放弃语句的键在回收后仍有一个残留的主键索引条目……同键再插会撞 DuplicateKey」——本 change 修复后该形态不再成立（键全部释放）。注释不影响任何断言（该用例的写入键 9001/9002 本就在被放弃语句的键区间之外），属纯注释陈旧。**同时**：Act 的 Deviations 4 称「两处注释已改为描述修复后的事实」，实读后 `:794-797` 这一处仍是旧描述 ⇒ Act 自述与文件现状有出入（Minor）。按 Review 只读约束，本 Cycle 不改测试文件；记为 finding。

**F11 — Minor（不阻塞）：`find_key_by_row_id` 的 panic 路径**（见 F4）。

**F12 — Minor（不阻塞，同时作为 Issue 候选上报）：目录根同步（`sync_root_to_catalog`）仍在写单元之外。** D1 明确保留该项（持久化关注点不入结构原子性单元），Act 未复现、未登记为 Issue。但独立读码确认其机制：两个写者并发换根时，A 的 `sync_root_to_catalog(Ra)` 与 B 的 `sync_root_to_catalog(Rb)` 是闭包返回后的**并发**目录写（`Catalog::update_table_root` 取目录锁后「后写者胜」）；若 A 的写晚于 B 落盘，目录里持久化的根落后于内存根，直到下一次换根才被覆盖。内存根本身在闸门保护下始终正确（与修复前相比严格变好——修复前内存根就可能被过期根覆盖）。按 M20 与用户「issue 候选一律找承载方式」的指令，本项作为 **Issue 候选**上报（见交付段），由 Recorder 决定承载；本 change 不扩范围修复。

**F13 — Minor（不阻塞）：闸门公平性假设。** Act 记录了「写单元尚在『已 spawn 未取到闸门』时的取闸顺序倒置」的理论可能。独立核对：后果有界——读者若先取到闸，读到的是写单元落定前的映射 ⇒ 该行被回滚分桶误判为墓碑 ⇒ 其正向条目不被移除 ⇒ 最多残留一个键条目（E2 形态），**不会**产生结构损坏（E1 形态），因为写单元本身是原子的。被覆盖场景由暂停点夹具钉死（写单元在暂停点已持闸），确定性成立。

**F14 — Minor（不阻塞）：`find_key_by_row_id` 的逐行线程跳转。** 回滚/回收与 WAL 恢复的逐行分桶路径每次读多一跳 `spawn_blocking`。既有语义不变。

**F15 — Act Response 的一处自述与现状的出入**：见 F10（Deviations 4 的「两处注释」）。不影响 Acceptance 判定。

**Acceptance Gaps**

None。Cycle 的 A1–A6 六项验收全部由 Act 的真实运行覆盖，且经本次独立读码与 diff 核对，与代码现状一致；未发现「用例通过但缺陷仍在」的情形（R1/R2/R3 的判别力见 F8；R3 的确定性由暂停点夹具保证）。

**Evidence**

- 实现（本次独立阅读的当前文件状态）：`src/storage/btree/index_manager.rs:38-137`（暂停点夹具）、`:142-165`（字段与 `Arc` 化）、`:173-209`（`new` / `from_root`）、`:230-240`（`arm_write_pause`）、`:374-414`（`insert` 写单元）、`:421-462`（`delete` 写单元与顺序反转）、`:476-501`（`delete_exact`）、`:561-583`（`update`）、`:595-604`（`find_key_by_row_id` 串行读）；导出见 `src/storage/btree/mod.rs:14`、`src/storage/mod.rs:20`。
- 语义等价核对：`src/storage/btree/btree.rs`（`search` 精确匹配、`delete` → `LeafNode::delete` 精确匹配、`delete_exact` 不换根）、`src/storage/page_format/key.rs`（`Key::new` 的 32 字节定长比较 ⇒ 测试用 `i64::to_be_bytes` 查键不受尾部零截断影响）。
- 共享边界与调用链核对：`src/database.rs:109-111`（`Database: Clone`，字段全 `Arc`）、`src/storage/data/table_manager.rs:95`（`TableMeta.index_manager: Arc<IndexManager>`）、`src/transaction/manager.rs:321-355`（A/B 分桶，**未改**）、`src/wal/recovery.rs:984`（redo 删除路径）、`src/executor/insert.rs:275-286`（`record_version` → `index_manager.insert` 的顺序前提）。
- 需求与缺陷正文：change 的 `specs/index-write-side-atomicity/spec.md`（R1–R4 逐条核对见 F9）、`design.md`（D1–D7）、`tasks.md`（RTM 与 Iteration Plan）、`.agents/issues/ISS20-index-write-side-atomicity.md`（E1/E2 根因；回归要求第 4 条：全部被取消行的键都能再次插入 + `check_integrity` 无 Error 级发现 + 同句柄后续写语句不得崩溃）。
- 沿用（未重跑）的运行结论（来源：Act Response「Verification Evidence」表）：T1 RED 连续 3 次稳定命中产品断言；T1–T3 GREEN 连续 5 次；`statement_cancellation_test` 8 passed；`--lib --bins` 420 passed（= MS34 基线）；15 个受影响目标 171 passed / 0 failed；`cargo fmt --check` 无 diff；clippy lib 告警 14 条（= 基线，改动文件内无新增）。本次 Review 未重跑任何一条（公共规则 › 验证：独立检查 = 独立读码与核对，不重复已沿用的验证）。

**Follow-up Decision**

**接受本 Cycle**（`Review Result: accepted`）：A1–A6 全部满足，且经独立读码与 diff 核对未发现结论与代码矛盾。Deviation 3 裁定为「Plan 遗漏但在已批准 requirement 内」——不需要新的执行依据、不需要 replan；需要的是 design 的长期记录回写，已在本次 Review 就地完成（D2 / D3 修订）。其余 finding（F3、F4、F10、F11、F13、F14、F15）为 Minor，F12 为 Minor 且同时作为 Issue 候选上报，均**不阻塞** Acceptance；按公共规则「不阻塞 Acceptance 的 Minor finding 不创建修复 Cycle」，本 Iteration 不创建 rework / replan Cycle。

**Iteration Plan Update**

无。Iteration 000 的范围、稳定结果、验证边界与诊断边界均不改变；Deviation 3 的裁定不改变 requirement、设计方向、验证方式或验收边界（design 的记录性修订已就地完成）。

**Next Cycle**

None（当前 Iteration 已被接受；不创建 rework / replan Cycle）。

**Next Iteration**

`openspec/changes/2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog/iterations/001-ddl-catalog-transactional/000-initial.md`（Iteration 001：DDL 目录事务化 T4–T6 + 本 change 的收尾 M19 分批全量回归）。
