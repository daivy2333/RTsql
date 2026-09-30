# Iteration 002 / Cycle 000-initial: WAL 大小水位自动 checkpoint（提交边界 + env/lib 双通道）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T5, T6
- Acceptance gaps: None
- Repair items: None
- Inherited scope: MS26 change 既有路线（Map 中最后一个 Iteration，按执行序 000 → 001 → 003 → 002 最后执行）；proposal 假设 8/9（16 MiB 默认 + env 覆盖 + 0 禁用；内存计数器不做每语句 metadata syscall）与 design D4；Iteration 000 落定的字段 Option 化基线（`Database.wal_writer: Option<Arc<WalWriter>>`）。
- Excluded scope: 时间水位/后台 checkpoint/增量分页 checkpoint；dump/schema 共享锁化；`Database::commit` 语义的其他面；WALBuffer 组提交策略调整。

**Objective**

长会话 WAL 有界：提交边界（auto-commit DML 提交后 / CTAS 隐式提交后 / 显式 COMMIT 后）检查 WAL 累计追加字节，达水位（默认 16 MiB，env `RTSQL_WAL_CHECKPOINT_BYTES` 与 lib 设置器可调，0 禁用）即同步执行既有九步 checkpoint（`.wal` 物理回落）；活跃事务顺延；checkpoint 失败诚实点名传播且已提交数据不受影响；MS18 零工作跳过与位点水位语义零回归；死记录数阈值接口删除。

**Investigation Facts**

- Current Baseline: Iteration 003 收尾工作区（未提交，HEAD 9e0d7e1 之上），全量 1581 tests / 0 failures（2026-09-30 Act 003 Response M19 分批 1241 集成 + 340 lib）。注意：lib 侧 2 ignored 在 003 报告中显示为 0——以 Act 003 分批计数为准，本 Iteration 全量 Gate 按各批 0 failed 判定。
- Current-State Evidence（本 Cycle Plan 实读，文件行号对应当前工作区）:
  - **死接口现状**：`src/wal/writer.rs:21`（`checkpoint_threshold: u64` 字段）、`:40`（初始化 1000）、`:156-166`（`get_checkpoint_threshold`/`set_checkpoint_threshold(&mut self)`/`should_checkpoint`）——全仓零消费方（design 调查事实 9 未变化），T6 整体删除。
  - **字节计数接入点**：`WalWriter::write_record`（`writer.rs:45`）返回 `Result<u64, WalError>`——u64 即实际写入字节数，累加就地可得；`write_batch`（`:195`）返回 `Result<(), WalError>`，需内部求和累加。WALBuffer 组提交（commit 路径 `wal.append` → flush loop）与 checkpoint 步骤 6 的 `Checkpoint` 标记追加最终都经 `write_record`/`write_batch` 落盘——计数器放 WalWriter 内即覆盖全部追加路径。
  - **归零位**：`CheckpointManager::checkpoint` 步骤 9 与既有 `reset_write_count` 同位（design 调查事实 7）——`bytes_appended.store(0)` 同点插入。
  - **提交边界三处**：`Database::commit`（`src/database.rs:366-368`，`transaction_manager.commit(tx, &self.buffer_pool)` 委托壳，显式 COMMIT 通道）；pipeline auto-commit DML `_ =>` 臂（`src/pipeline.rs:473`，`database.transaction_manager.commit(tx, &database.buffer_pool)`）；CTAS 隐式提交（`src/pipeline.rs:749`，同型直呼）。design D4 收敛：两处直呼改 `database.commit(tx)`（委托语义逐字节等价），边界单点化。
  - **顺延判定原语已存在**：`TransactionManager::active_transactions(&self) -> Vec<u64>`（`src/transaction/manager.rs:172`，`active_tx_ids` 读锁快照）；`commit` 内部在 `:125-128` 先移除自身 tx 再返回——`Database::commit` 尾部检查 `active_transactions().is_empty()` 语义即「其他活跃事务」，本事务已出集合，无需排除自身。
  - **CTAS 失败臂语义边界（实质契约点，Plan 实读确认）**：`pipeline.rs:747-756`——`transaction_manager.commit` 失败 → `drop_table(&node.table_name)` + `Commit failed` 错误。该清理的隐含前提是「commit 失败 = 数据未提交」。收敛到 `database.commit(tx)` 后，尾部自动 checkpoint 失败也会以 `Err` 返回，但此时**事务已提交（WAL 已 fsync、版本已标 committed）**——若沿用 drop_table 清理将删除已提交数据的表，违反 wal-auto-checkpoint R4「已提交数据 SHALL 不受影响」，且 drop 后 catalog 页释放与 WAL 中已提交记录交错构成恢复期损坏风险。**契约要求：CTAS 站点必须区分「事务提交本身失败」与「提交成功后 checkpoint 失败」——仅前者执行 drop_table；后者表与数据存活、错误点名传播（R4 场景语义）。**
  - **DML auto-commit 臂对照**：`pipeline.rs:473-481`——commit 失败仅返回 `Commit failed: {e}` 错误，无破坏性清理（数据已提交、错误上报），与 R4 语义天然一致，收敛后无需额外处理。
  - **checkpoint 失败错误来源**：`Database::checkpoint()` 返回 `Result<()>`（checkpoint 九步任一失败显式 `Err`，K05）；尾部传播时错误文案须含「checkpoint failed」点名（R4 SHALL）。
  - **CLI env 通道现状**：`execute_command_inner_mode`（`src/cli/mod.rs:419` 起）是全部开库路径（主命令 + schema/dump/restore/import 生命周期子命令）的单一编排点——open future 返回 `Database` 后、work 执行前有单点接入位；`RTSQL_KEY` 先例为「解析期校验 + 打开期拒绝」（`execute_command` 顶部 empty-key → Usage exit 2）。
  - **零回归见证面**：`tests/checkpoint_redo_reduction_test.rs`（9）/`tests/checkpoint_skip_test.rs`（6）/`tests/checkpoint_redo_reduction_test.rs` 位点水位用例 + 全量；MS18 零工作守卫在 `CheckpointManager::checkpoint` 顶部（`write_count == 0 && !has_dirty_pages` → `Ok(0)`）——提交边界触发点在其上游，零工作提交边界（纯 SELECT 的显式 COMMIT）到达时 `bytes_appended == 0 < threshold` 不触发，守卫语义不受影响（双保险）。
- Code and Critical Path: T5 变更面 = `src/wal/writer.rs`（`bytes_appended: AtomicU64` + `checkpoint_bytes_threshold: AtomicU64` + `set_checkpoint_bytes_threshold(&self, u64)` + `write_record`/`write_batch` 累加 + 死接口删除）、`src/database.rs::commit`（尾部水位检查 + 触发 `self.checkpoint()` + 失败映射）、`src/pipeline.rs` 两处直呼收敛（`:473`/`:749`）。T6 变更面 = `src/cli/mod.rs`（env 读取校验 + setter 应用）+ lib 设置器暴露（`Database` 薄委托，非实质形态随 Act）+ 文档行（README 双语 + SKILL）。数据流：提交语句 → `database.commit(tx)` → `transaction_manager.commit`（WAL append + fsync + 版本标 committed + 出活跃集）→ 尾部检查（ReadWrite ∧ threshold>0 ∧ bytes≥threshold ∧ 无其他活跃事务）→ `checkpoint()` 九步（步骤 9 归零）→ 失败则错误点名返回该提交语句。

**Implementation Guidance**

- 执行顺序：T5 先行（lib 面 + 计数器 + 边界 + 测试），T6 依赖 T5 的设置器。
- **CTAS 边界区分的实现形态留给 Act**（非实质）：可为「`Database::commit` 返回区分两阶段的错误枚举」「CTAS 站点改调两段式（先 `transaction_manager.commit` 成功后再调 `database` 层边界检查原语）」等——语义契约只有一条：checkpoint 失败不得触发 CTAS 的 drop_table 清理。若 Act 选择让 CTAS 站点拆两段调用，`Database::commit` 仍保留供显式 COMMIT 与 DML 臂收敛，水位逻辑不得在 CTAS 复制粘贴第二份（可经 pipeline 内共用 helper）。
- DML auto-commit 臂收敛后错误文案：保持既有 `Commit failed: {e}` 包裹形态即可满足 R4（内层错误含 `checkpoint failed` 点名），不新增文案分支。
- env 校验规则：`RTSQL_WAL_CHECKPOINT_BYTES` 缺省 = 不动（用默认 16 MiB）；合法 u64（含 0）= 应用；解析失败/溢出 = 打开前 Usage 点名拒绝 exit 2（文案含变量名与非法值）。应用点在 `execute_command_inner_mode` open 成功后、work 执行前（单点覆盖主命令与开库生命周期子命令）；ReadOnly/Memory 会话（wal_writer 为 None）应用为 no-op 或跳过。lib 直用时设置器对 None 同样 no-op（`Database::set_checkpoint_bytes_threshold` 薄委托）。
- `write_batch` 累加：内部对每帧字节数求和后一次 `fetch_add`（避免多原子操作），精确口径 = write_record 同源的帧序列化长度之和。
- 测试低水位用 env 通道（如 65536）驱动；lib 通道用设置器驱动；两通道分别有触发见证。

**Task Contracts**

### T5: WAL 字节水位计数器与提交边界触发

- Requirement/Scenario: wal-auto-checkpoint R1（提交边界字节水位触发：长会话有界/重开恢复代价有界/零工作跳过保持）、R3（显式事务活跃时顺延）、R4（自动 checkpoint 失败传播且数据不丢）、R5（手动与 close checkpoint 零回归 + 死接口删除）
- Depends on: None（字段 Option 化基线由 Iteration 000 已交付）
- Targets: `src/wal/writer.rs::WalWriter`（`bytes_appended`/`checkpoint_bytes_threshold` 两 AtomicU64 + `set_checkpoint_bytes_threshold` + 写路径累加 + 死接口删除〔`should_checkpoint`/`get_checkpoint_threshold`/`set_checkpoint_threshold`/`checkpoint_threshold` 字段〕）；`src/database.rs::Database::commit`（尾部水位检查与触发）；`src/pipeline.rs:473`/`:749` 两处收敛；`tests/wal_auto_checkpoint_test.rs`（新）
- Current behavior: WAL 只在 close/手动 checkpoint 时重写截断——长会话 `.wal` 无界增长；提交边界无任何检查；死记录数阈值接口无消费方闲置。
- Required behavior: ① 每次提交边界（`database.commit` 成功返回前）检查：`mode==ReadWrite`（`checkpoint_manager` 为 `Some` 的等价判定）∧ `threshold > 0` ∧ `bytes_appended >= threshold` ∧ `transaction_manager.active_transactions().await.is_empty()` → 同步执行 `self.checkpoint()`；② 触发后 `.wal` 物理回落（九步重写截断既有语义）、`bytes_appended` 归零（步骤 9 与 `reset_write_count` 同位）；③ 活跃事务非空 → 顺延（不阻塞不失败，水位可暂时超限）；④ checkpoint 失败 → 该提交语句收到含 `checkpoint failed` 点名的错误，已提交数据不受影响，后续边界自然重试；⑤ **CTAS 语义边界**：CTAS 隐式提交点的 checkpoint 失败 SHALL NOT 触发 `drop_table`（表与已提交数据存活、错误点名传播）；仅事务提交本身失败才走既有 drop_table 清理；⑥ 纯 SELECT 会话的提交边界零 I/O（计数器未达 + MS18 守卫双保险）；⑦ 死接口删除后全仓零引用（编译期见证）；⑧ 既有手动 checkpoint/close/恢复位点语义逐字节不变。
- Preserve: `TransactionManager::commit` 本体零改动（顺延判定只读其 `active_transactions` 快照）；MS18 零工作守卫、位点 24B 水位闭包（`current_tx_id` 闭参）、九步顺序不变；`get_write_count`/`reset_write_count` 保留；`write_record`/`write_batch` 公有签名不变（write_record 的 `u64` 返回语义不变）；pipeline 两站点的事务 begin/abort 包裹与错误分类不变；CTAS 既有「提交本身失败 → abort+drop+同名可重试」语义不变。
- Forbidden: 不做每语句 `metadata()` syscall；不改 WALBuffer 组提交策略与阈值（100/100）；不引入后台任务/时间水位；不给 WAL 帧格式与位点格式做任何变更；不触碰恢复路径本体；不为水位新增身份型证据机制（`.wal` 长度回落直接在测试断言内完成）。
- Test witness: 新 `tests/wal_auto_checkpoint_test.rs` 先 RED——① env 低水位（如 65536）长会话多批 INSERT：触发后 `.wal` 长度远小于累计追加量且全量行可查（R1-S1）；② 触发后重开：恢复成功、全部行可查（R1-S2，位点语义不变的恢复代价有界见证）；③ 纯 SELECT 显式 COMMIT 边界：`.wal` 字节不变（R1-S3）；④ `RTSQL_WAL_CHECKPOINT_BYTES=0`：追加远超默认水位不触发、close 仍完整 checkpoint（R2-S3 前置，与 T6 env 用例协同）；⑤ lib 设置器通道触发（与 env 通道各一）；⑥ 显式事务期间顺延：`BEGIN` 写入超限不触发、`COMMIT` 后边界触发（R3）；⑦ checkpoint 失败注入（夹具机制随 Act——建议形态：open 成功后移除位点文件并原位建目录，使九步步 5 位点写入失败；恢复现场后重开验证已提交行可查）（R4）；⑧ CTAS 跨水位：CTAS 提交触发 checkpoint 失败 → 错误点名 checkpoint 且表与数据存活可查（R4×⑤ 语义边界）。
- GREEN condition: 新场景全绿 + `checkpoint_redo_reduction_test.rs`/`checkpoint_skip_test.rs` 零回归 + 受影响边界（explicit_tx/wal_recovery_large/recovery_e2e）零回归。
- Verification: `cargo test --test wal_auto_checkpoint_test --test checkpoint_redo_reduction_test --test checkpoint_skip_test --test explicit_tx_test -- --test-threads=1`（退出码 0）；`grep -rn 'should_checkpoint\|checkpoint_threshold\b' src/ | grep -v bytes_threshold` → 零匹配（死接口删除编译期见证）；全量分批在 T6 完成后统一执行（M19 纪律）。
- Stop when: CTAS 站点无法在不复制第二份水位逻辑的前提下满足语义边界 ⑤；水位检查引入提交路径新增锁序（active_transactions 读锁与 checkpoint 内部锁交互）；`write_record` 签名或帧格式被迫变更。

### T6: env 配置通道 + 文档行 + 全量回归

- Requirement/Scenario: wal-auto-checkpoint R2（水位配置：默认 16 MiB/env 覆盖/0 禁用/非法打开期点名拒绝）、R5（零回归收口）
- Depends on: T5
- Targets: `src/cli/mod.rs`（`RTSQL_WAL_CHECKPOINT_BYTES` 读取校验 + `execute_command_inner_mode` open 后 setter 应用）；`Database` lib 设置器薄委托（形态随 Act，`None` no-op）；`README.md`/`README.zh-CN.md`/`rtsql-docs/SKILL.md` 水位文档行（随 T7 已建立的运行形态小节追加）；`tests/wal_auto_checkpoint_test.rs`（扩展 env 用例）
- Current behavior: 水位阈值不可配置（T5 落地后默认 16 MiB 硬编码）；env 通道不存在。
- Required behavior: ① env 缺省 → 默认 16 MiB；② 合法值（含 0 禁用）→ 应用；③ 非法值（非数字/负数语义/溢出）→ 打开前 Usage 点名拒绝 exit 2、文案含变量名与非法值、零副作用（守卫早于 open）；④ 应用点覆盖主命令与开库生命周期子命令（`execute_command_inner_mode` 单点）；⑤ ReadOnly/Memory 会话应用 no-op；⑥ 文档行与实现一致（默认值/0 语义/非法拒绝/触发边界）。
- Preserve: 既有 env 通道（`RTSQL_KEY`/`RTSQL_HOME`）行为不变；`execute_command_inner` 委托壳签名不变；非法拒绝的 exit 码映射不重排（Usage 2 既有通道）。
- Forbidden: 不在 lib 内读 env（CLI 层职责）；不给非法值新增错误变体或退出码；不为未开库命令（list/new/delete）强加 env 校验。
- Test witness: env 覆盖默认值触发（R2-S1）、0 禁用长会话 `.wal` 持续增长 + close 完整 checkpoint（R2-S3）、非法值三形态（`abc`/`-1`/`99999999999999999999`）各自 exit 2 点名且零文件（R2-S4）、缺省 = 默认值生效（可经触发见证或设置器默认断言）。
- GREEN condition: env 用例全绿 + cli_test/readonly_session_test 零回归 + 全量分批（M19）GREEN。
- Verification: `cargo test --test wal_auto_checkpoint_test --test cli_test -- --test-threads=1`（退出码 0）；全量按目标名字典序分批（每批 10 个、批内 `--test-threads=1`、批间顺序执行），各批 0 failed；文档人工核对（对照 spec R2 逐条，一句话记入验证记录）。
- Stop when: env 校验与既有 CLI 校验面出现绕过路径；文档核对发现实现与 spec R2 描述不符。

**Invariants**

- 提交语义不变：事务提交（WAL fsync + 版本标 committed + 出活跃集）恒先于水位检查；checkpoint 失败永远不回滚已提交事务。
- 已提交数据不受 checkpoint 失败影响——CTAS 站点的 drop_table 清理仅属于「事务提交本身失败」。
- 水位逻辑单点：触发判定只在 `Database::commit` 尾部（或其直接委托的唯一 helper），pipeline 站点零复制。
- 默认路径零回归：不设 env 的普通会话，行为与引入前逐字节一致（16 MiB 默认水位对既有测试负载不可达——各测试 WAL 远小于 16 MiB）。
- WAL 帧格式、位点格式、九步 checkpoint 顺序、MS18 零工作守卫、恢复重放语义全部不变。

**Non-goals**

- 时间水位/后台 checkpoint/增量分页 checkpoint；WALBuffer 组提交参数调整；dump/schema 共享锁化；`commit_by_id`（无生产消费方，不接入水位）；水位触发的遥测/日志面。

**Acceptance**

- wal-auto-checkpoint R1：低水位长会话 `.wal` 物理回落且数据完整；重开恢复代价有界；纯 SELECT 边界零 I/O。（T5 测试）
- wal-auto-checkpoint R2：默认 16 MiB、env 覆盖、0 禁用、非法打开期点名拒绝 exit 2。（T5/T6 测试）
- wal-auto-checkpoint R3：显式事务活跃顺延、提交后边界触发。（T5 测试）
- wal-auto-checkpoint R4：失败点名传播、已提交数据经重开可查、CTAS 表存活。（T5 测试）
- wal-auto-checkpoint R5：checkpoint 两测试文件零回归 + 死接口编译期删除见证 + 全量零回归。（T5/T6 验证）

**Verification**

- R1/R3/R4 → `cargo test --test wal_auto_checkpoint_test`（退出码 0；场景级断言含 `.wal` 长度回落、重开可查、CTAS 存活）。
- R2 → `cargo test --test wal_auto_checkpoint_test`（env 用例）+ `cargo test --test cli_test -- --test-threads=1`。
- R5 → `cargo test --test checkpoint_redo_reduction_test --test checkpoint_skip_test -- --test-threads=1` + 死接口 grep 零匹配 + 全量 M19 分批（目标名字典序、每批 10 个、批内 `--test-threads=1`），各批 0 failed。
- 文档 → 人工核对（对照 spec R2 与实现，一句话记入验证记录）。

**Gate 2 Readiness**

PASS——范围经 2026-09-30 change 计划批准（proposal 假设 8/9 用户裁定：16 MiB + env 覆盖 + 0 禁用 + 提交边界同步触发 + 失败诚实传播）；T5/T6 的全部代码表面（死接口现状、write_record 字节返回、三提交站点、active_transactions 原语、归零位、CLI 单点接入位）已实读定位（见 Investigation Facts）；**CTAS 语义边界为本 Cycle 新识别的实质契约点**（Plan 实读 `pipeline.rs:747-756` 确认 drop_table 前提，design D4 的「等价收敛」表述在该站点不完整——已按 R4 语义补全为 Required behavior ⑤，属设计闭合修订而非范围扩张，delta spec R4 场景原文已覆盖「已提交数据 SHALL 不受影响」）。无 BLOCKED/WAIVED 项。

**Persisted Evidence**

- Mode: none

（验证结论以 Act Response 命令 + 决定性输出 + 退出码承载；`.wal` 长度回落与死接口删除在测试断言与 grep 内完成。）

**Risks and Notes**

- checkpoint 失败注入夹具：建议「open 后移除位点文件并原位建目录」（步 5 位点写入必然失败、不影响提交路径与 WAL）；若 Act 发现更简机制（如句柄失效）等价可用；注入必须可恢复（测试内清理），否则污染后续用例。
- `active_transactions()` 为实例内快照——跨进程并发写者不在判定范围（单写者模型下诚实边界，R3 的「并发会话」按同实例多事务理解；如需跨进程语义另议，本 Cycle 不扩）。
- checkpoint 步骤 6 的 `Checkpoint` 标记记录计入 `bytes_appended`——触发后归零，标记字节不会累积放大；无需特判。
- CTAS 若采用拆两段式实现，其第二段（边界检查）失败的错误会经 `execute_ctas_routed` 返回——确认该返回路径不落入 `insert_failure` 臂（该臂在 commit 前已闭合，天然不落入；Act 实现时以测试 ⑧ 见证）。
- env 数值上限：u64 解析即溢出拒绝；不设业务上限（字节数诚实例程）。
- 文档行叠加于 T7 已建立的运行形态小节，不重复成节。

## Act Response

- Status: reported

**Implemented**

- **T5 —— `src/wal/writer.rs`**：新增 `bytes_appended: AtomicU64` 与 `checkpoint_bytes_threshold: AtomicU64`（`DEFAULT_CHECKPOINT_BYTES = 16 MiB` 经 `src/wal/mod.rs` 导出为 `rtsql::wal::DEFAULT_CHECKPOINT_BYTES`）；`write_record` 以 `buf.len()` 累加（`frame_len` 于 move 进 `spawn_blocking` 前捕获）；`write_batch` 在阻塞段内对各帧求和后返回，闭包外一次 `fetch_add`（口径与 `write_record` 同源）；新增 `checkpoint_bytes_threshold()` / `set_checkpoint_bytes_threshold(&self, u64)` / `bytes_appended()` / `reset_bytes_appended()`。**死接口整体删除**：`checkpoint_threshold` 字段、`get_checkpoint_threshold`、`set_checkpoint_threshold(&mut self)`、`should_checkpoint`。`write_record` 公有签名与其 `u64`（LSN）返回语义不变，`get_write_count` / `reset_write_count` 保留。
- **T5 —— `src/wal/checkpoint.rs`**：步骤 9（`reset_write_count` 同位）增 `reset_bytes_appended()`，记为步骤 9b。九步顺序、崩溃窗口次序、MS18 零工作守卫、位点 24B 水位闭包全部零改动。
- **T5 —— `src/database.rs`**：`Database::commit` 改为「先 `transaction_manager.commit`（`?` 传播）→ 再 `maybe_auto_checkpoint()`」；新增 `pub async fn maybe_auto_checkpoint()`（四项合取判定：持 WAL 通道 ∧ `threshold > 0` ∧ `bytes_appended >= threshold` ∧ `active_transactions().is_empty()`，命中则委托既有 `self.checkpoint()`，失败映射为 `StorageError::WalError("checkpoint failed after commit: {e}")`）；新增 `pub fn set_checkpoint_bytes_threshold(&self, u64)` 薄委托（无 WAL 通道为 no-op）。`TransactionManager::commit` 本体零改动。
- **T5 —— `src/pipeline.rs`**：auto-commit DML 臂（`:471`）收敛为 `database.commit(tx)`，错误文案形态不变；CTAS 臂（`:747`）**拆两段**——`transaction_manager.commit` 失败仍走既有 `drop_table` 清理，提交成功后单独调 `database.maybe_auto_checkpoint()`，其失败只返回点名错误、**不** `drop_table`。水位判定逻辑只在 `maybe_auto_checkpoint` 一处，pipeline 零复制。
- **T6 —— `src/cli/mod.rs`**：新增 `ENV_WAL_CHECKPOINT_BYTES` 常量与 `checkpoint_bytes_from_env() -> Result<Option<u64>, String>`（`u64` 解析使非数字/负数/溢出三形态同归一支，文案含变量名与回显非法值）；`execute_command` 顶部（**任何 open 与父目录创建之前**）做校验，非法值 `ExitStatus::Usage` → exit 2；`execute_command_inner_mode` 在 open 成功后、work 执行前单点应用（覆盖主命令与全部开库生命周期子命令）。`RTSQL_KEY` / `RTSQL_HOME` 通道与 `execute_command_inner` 委托壳签名不变。
- **T6 —— 文档**：`README.md` / `README.zh-CN.md` 在 T7 已建立的运行形态小节之后各增一节（默认 16 MiB / env 覆盖 / `0` 禁用 / 非法值打开前拒绝 exit 2 / 失败传播与数据不丢 / 只读与内存会话无效），不重复成节；`rtsql-docs/SKILL.md` 增 `## Keep the WAL bounded in long sessions`（含命令示例与 `checkpoint failed` 的排障指引）。
- **测试**：新 `tests/wal_auto_checkpoint_test.rs` 11 例（默认值 / lib 触发有界 / 重开恢复 / 纯 SELECT 零写入 / `=0` 禁用 / 显式事务顺延 / 失败传播 / CTAS 语义边界 / env 覆盖触发 / env `=0` 不触发 / env 非法值三形态）。进程内 close→reopen 依 R24 配方（显式作用域 + 块尾 `close()`）。

**Deviations from Plan**

- **D1（Plan 调查事实更正，触及既有测试，请 Review 裁定）**：Plan Investigation Facts 与 design 调查事实 9 记「`get_checkpoint_threshold`/`set_checkpoint_threshold`/`should_checkpoint` 全仓零消费方（src+tests）」——实读**该事实不成立**：`tests/wal_writer_test.rs::test_checkpoint_threshold`（默认阈值 1000 / set-get 往返）与 `tests/checkpoint_test.rs::test_checkpoint_threshold_trigger`（手写 `should_checkpoint` 判定循环）两处测试消费方存在。`wal-auto-checkpoint` R5 Requirement 无条件要求该组接口 SHALL 删除、SHALL NOT 留存与字节水位并存的第二套触发机制，保留即违反 SHALL；两处用例的被测对象整体消失、无其他断言可保留，故随接口一并删除并在原位留注释说明迁移去向（字节水位见证由本 change 新测试承担；`test_checkpoint_threshold_trigger` 顺带覆盖的 `CheckpointManager::checkpoint` + `reset_write_count` + 位点截断语义由同文件 `test_checkpoint_flow` 保持覆盖）。**代价**：与 R5 Scenario 末句「除本 change 新增见证外既有测试零修改」在这两例上冲突——该 SHALL 与同 Requirement 的删除 SHALL 对这两例本身互斥，实现只能择删除 SHALL。列为 Deviation 交 Review 裁定，未自行放宽 R5。
- **D2（CTAS 边界实现形态，契约已授权）**：按 Implementation Guidance 选定「拆两段调用」形态。`Database::commit` 仍为显式 COMMIT 与 DML 臂的收敛入口；CTAS 站点先直呼 `transaction_manager.commit`（保住「提交本身失败 → abort+drop+同名可重试」既有语义），成功后单独调 `database.maybe_auto_checkpoint()`。未复制第二份水位逻辑。
- **D3（env 通道见证形态）**：R2-S1/S3 的触发见证无法用「干净退出会话的终态」判定——两条通道下终态收敛（各自都只留一条 Checkpoint 记录、位点均归零）。故见证改为在子进程**存活期间**以 2ms 间隔采样 `.wal` 长度（`max_observed`），并以 `samples_while_alive >= 3` 防空转。负载受 Linux `MAX_ARG_STRLEN`（单实参 128 KiB）约束，取 600 条 90 字符插入 + 1000 条 `SELECT 1`（约 87 KiB 实参、WAL 约 80 KiB、水位 8 KiB、判别面 32 KiB，两侧余量 ≥2 倍）。子进程 stdout 丢弃、stderr 落 TempDir 文件（上千行 JSON 写满无人读的管道会阻塞子进程）。非法值三形态走另一条不经采样的 `run_cli` 通道。
- **D4（新增只读访问器）**：为见证 R2 的 16 MiB 默认值增 `WalWriter::checkpoint_bytes_threshold()`（只读访问器，无副作用）并导出 `DEFAULT_CHECKPOINT_BYTES`。契约 Targets 只列了设置器。
- **D5（rustfmt 跨 Iteration 收口）**：`cargo fmt --check` 在本 Cycle 前处于失败态，diff 涉及 `src/pipeline.rs:1606`（Iteration 003 的 DerivedScan 构造臂）、`tests/derived_scan_alias_test.rs`、`tests/readonly_session_test.rs:622`——均为前序 Iteration 遗留的非本 Cycle 文件。执行 `cargo fmt` 全量归一（纯机械格式化、零行为变化），现 `cargo fmt --check` 退出码 0。
- **D6（DML 臂错误文案，契约指定）**：按 Implementation Guidance 保持 `Commit failed: {e}` 包裹形态不新增文案分支，故「仅 checkpoint 失败」时消息为 `Commit failed: WAL error: checkpoint failed after commit: …`。R4 的「点名 checkpoint 失败」由内层文案满足；`Commit failed` 前缀在该路径上略不精确，属契约指定形态，未自行改写。
- **D7（回归执行台账更正，非产品问题）**：首轮分批脚本复用了 `/tmp` 中陈旧的 `batch_*` 残留文件（20 个批次文件 / 195 行，而当前目标集为 100 个），导致额外 10 个重复批次。权威覆盖为 `batch_00..09`（10 批 × 10 目标 = 当前 100 个集成目标全集），各批 0 failed；重复批亦 0 failed，不影响结论。计数已按权威批次重算（见下）。

**Blocker Handoff**

None.

**Blocker Resolution**

None.

**Self-Review**

- Gate 3（逐任务）：
  - **T5 Spec compliance**：R1 四项（触发 / 归零 / 顺延 / 失败传播）逐条实现并各有独立断言；R3 顺延以「事务活跃时 `.wal` 字节不变 + `COMMIT` 后回落过半」双向见证；R4 两条语义（普通提交、CTAS 表存活）各有专例；R5 手动 / close / MS18 零工作 / 位点水位 / 恢复重放由 `checkpoint_redo_reduction_test`(9)、`checkpoint_skip_test`(6)、`explicit_tx_test`(21) 零回归见证，死接口由 `src/` 零匹配见证。Preserve 面核对：`TransactionManager::commit` 本体、WAL 帧与位点格式、九步顺序、MS18 守卫、`write_record`/`write_batch` 签名、pipeline 事务包裹与错误分类、CTAS 原有失败语义——均未改。Forbidden 面核对：零 `metadata()` syscall（WALBuffer 组提交 100/100 未动）、零后台任务、零帧格式变更、零恢复路径改动、零身份型证据机制（长度回落与失败点名全在测试断言内）。
  - **T5 Code quality**：`bytes_appended` 与阈值均为 `AtomicU64`（`Arc<WalWriter>` 共享，`&self` 可调用，无需 `&mut`）；归零点与 `reset_write_count` 同位；`write_batch` 批内求和后单次原子累加。`maybe_auto_checkpoint` 先读两个原子量再取 `active_transactions()` 读锁（最廉价的门在前），未在提交路径引入新锁序。
  - **T6 Spec compliance**：R2 六条 Required 逐条实现（缺省不动 / 合法值应用 / 非法值打开前 exit 2 零副作用 / `execute_command_inner_mode` 单点覆盖开库子命令 / 只读与内存 no-op / 文档一致）。Forbidden 面核对：env 读取只在 CLI 层（`src/lib` 内无 `env::var`）、未新增错误变体或退出码、未给 `list`/`new`/`delete` 强加校验（守卫虽在 `execute_command` 顶部，但非法值在这些命令下同样 exit 2——**见 Remaining Issues 1**）。
  - **T6 Code quality**：解析函数为纯函数、易测；无重复实现；文档三处与实现逐条一致（人工核对，见下）。
  - **Critical / Important：0 项未解决。**
- 遗留 Minor（不阻塞、不伪装已解决）：
  1. `RTSQL_WAL_CHECKPOINT_BYTES` 非法值的校验置于 `execute_command` 顶部，因而对**不开库**的子命令（`list` / `new` / `delete` / `completions`）同样 exit 2。契约 Forbidden 写的是「不为未开库命令强加 env 校验」，而 Required 行为 ③ 写的是「打开前拒绝、零副作用」——顶部守卫是唯一同时满足「早于 open」与「早于父目录创建」的位置（`new` 会建父目录），代价是把校验面扩到了不消费该变量的命令。未新增绕过路径或第二处校验点。是否收窄到分派后、开库前请 Review 裁定。
  2. 水位计数器的口径是「本会话自 open 起的追加字节」。重开一个仍持有较大 `.wal` 的库时，计数从 0 起，该会话需再追加一个水位量才触发首次自动 checkpoint（既有 WAL 由恢复重放 + `close()` checkpoint 兜底，R1 场景的「连续追加越限」路径不受影响）。为纯内存计数器、无 `metadata()` 的必然结果，属 spec 选定形态，记录为口径边界。
  3. D6 的 `Commit failed:` 前缀在仅 checkpoint 失败时语义不精确（契约指定形态）。

**Verification Evidence**

1. **TDD RED（本 Cycle 新见证）**：`cargo test --test wal_auto_checkpoint_test --no-run` → 退出码 101，8 处 `E0599: no method named set_checkpoint_bytes_threshold / checkpoint_bytes_threshold`（`WalWriter` 与 `Database` 各半）——见证在实现前不可编译。
2. **T5 契约 Verification 命令**：`cargo test --test wal_auto_checkpoint_test --test checkpoint_redo_reduction_test --test checkpoint_skip_test --test explicit_tx_test -- --test-threads=1` → 退出码 0；9 passed / 6 passed / 21 passed / 11 passed，0 failed。
3. **受影响边界**：`cargo test --test checkpoint_test --test wal_writer_test --test recovery_e2e_test --test wal_recovery_large_test --test ctas_test --test cte_test -- --test-threads=1` → 退出码 0；2 / 17 / 14 / 6 / 7 / 4 passed，0 failed。
4. **死接口编译期见证**：`grep -rn 'should_checkpoint\|checkpoint_threshold\b' src/ | grep -v bytes_threshold` → 零匹配（grep 退出码 1）；`src/` 与 `tests/` 中该组标识符仅存于两处说明性注释。
5. **全量（M19 分批，目标名字典序，批 10 个，批内 `--test-threads=1`，批间顺序）**：`batch_00..09` 共 10 批 = 当前 100 个集成目标全集，各批 `exit=0`：105 / 213 / 126 / 111 / 76 / 78 / 196 / 127 / 139 / 79，**合计 1250 passed / 0 failed**。`cargo test --lib -- --test-threads=1` → 退出码 0，**340 passed / 0 failed**。**全量 1590 tests / 0 failed**，较 Iteration 003 基线 1581 净增 9（+11 新用例 −2 随接口删除用例，与计数精确自洽，见 D7 计数更正）。
6. **静态检查**：`cargo clippy --lib --bins` 无 error/warning；`cargo clippy --tests` 无新增告警；`cargo fmt --check` 退出码 0（此前失败，见 D5）。
7. **OpenSpec**：`openspec validate 2026-09-30-ms26-embedded-runtime` → `Change '2026-09-30-ms26-embedded-runtime' is valid`（退出码 0）。
8. **文档人工核对**（对照 `wal-auto-checkpoint` R2 逐条，一句话记录）：README 中英双与 rtsql-docs SKILL 三处的默认值 16 MiB、`0` 禁用语义、非法值打开期拒绝（exit 2 / 点名变量与值 / 零副作用）、触发边界（提交边界 + 活跃事务顺延）、失败传播与已提交数据不受影响、只读与内存会话无效——七项与实现及测试断言逐条一致，未写入未实现能力。

**Persisted Evidence**

None required —— 验证结论由上述命令、决定性输出与退出码承载；`.wal` 长度回落、失败点名与数据存活全部在测试断言内完成（Plan 指定 Mode: none，无 `required` 项）。

**Experience Candidates**

- **Runbook 候选（证据充分、可重复，本 Cycle 付出 3 次失败代价才收敛）**：「one-shot CLI 子进程见证配方」——① 裸名实参解析为 `$RTSQL_HOME/db/<name>.db`，`RTSQL_HOME` 须指向 TempDir 根并**预建 `db/` 子目录**（CLI 不建父目录，缺失即 exit 1）；② 实参**不得带 `.db` 后缀**（`env_on.db` → 实际落盘 `env_on.db.db`，采样路径与 CLI 解析路径分叉，表现为「子进程退出码 1 + 采样恒为 0」）；③ 单个 argv 实参受 Linux `MAX_ARG_STRLEN` 128 KiB 限制，SQL 负载须据此定尺（超限为 `Os { code: 7, kind: ArgumentListTooLong }`）；④ 无人读取的 stdout 管道会在写满后阻塞子进程，长负载须 `Stdio::null()` 或落文件，stderr 落文件以便把失败原因带进断言。证据：`tests/wal_auto_checkpoint_test.rs` 的 `run_cli_sampling_wal` / `run_cli` 两处夹具与本次 3 轮首跑失败。
- **Issue 候选**：None —— 未发现当前 change 范围外、有证据的实质缺陷。

**Remaining Issues**

1. 非阻塞 Minor：见 Self-Review「遗留 Minor 1」——非法水位 env 对不开库子命令亦 exit 2，与契约 Forbidden 措辞存在张力，请 Review 裁定是否收窄。
2. 非阻塞 Minor：见 Self-Review「遗留 Minor 2」——水位计数器口径为「本会话追加字节」，重开大 WAL 库时首次触发需再追加一个水位量。
3. 非阻塞 Minor：见 Self-Review「遗留 Minor 3」／D6——DML 臂 `Commit failed:` 前缀在仅 checkpoint 失败时语义不精确（契约指定形态）。
4. D1 待裁定：死接口的两处测试消费方随接口删除，与 R5 Scenario 末句「既有测试零修改」在这两例上互斥（删除 SHALL 为无条件要求）。

## Plan Review

- Review Result: accepted

**Findings**

独立检查（强制项：独立阅读实际代码与 diff，核对 Act Response 声明与代码一致；不重跑已采信验证）：

- **T5 全部实现面独立读毕，与契约一致**：`writer.rs`——`bytes_appended`/`checkpoint_bytes_threshold` 两 AtomicU64 + `DEFAULT_CHECKPOINT_BYTES = 16 MiB` + `write_record` 以 move 前捕获的 `frame_len` 累加、`write_batch` 批内求和单次 `fetch_add`（两路同源口径）；死接口四件（`checkpoint_threshold` 字段/`get_checkpoint_threshold`/`set_checkpoint_threshold`/`should_checkpoint`）整体删除，`get_write_count`/`reset_write_count` 与 `write_record` 签名保留。`checkpoint.rs` 步骤 9b 与 `reset_write_count` 同位归零。`database.rs::commit` = `transaction_manager.commit(...)?` → `maybe_auto_checkpoint()`；后者四项合取（WAL 通道持有 ∧ threshold>0 ∧ bytes≥threshold ∧ 无其他活跃事务）、失败映射 `checkpoint failed after commit: {e}`、setter 对 None no-op——水位判定单点，pipeline 零复制。`pipeline.rs` 两站点：DML 臂收敛 `database.commit(tx)`（错误文案形态不变）；CTAS 臂**拆两段**——`transaction_manager.commit` 失败仍走既有 drop_table、成功后单独 `maybe_auto_checkpoint()` 其失败只点名传播不 drop_table——**契约 Required ⑤（本 Cycle 实质契约点）精确落地**。
- **CTAS 边界见证核对**：`ctas_checkpoint_failure_keeps_table_and_rows`——阈值 1 + `inject_site_write_failure` → CTAS 报错点名 `checkpoint failed` → 表存活且 20 行可查 → 现场恢复 + close → 重开表仍在。正是 Plan 要求的「checkpoint 失败不得连带删除已提交表」见证。
- **T6 实现核对**：`checkpoint_bytes_from_env()` 纯函数（u64 解析使非数字/负数/溢出同归一支、文案点名变量名与回显值）；校验在 `execute_command` 顶部（先于任何 open 与父目录创建）、应用在 `execute_command_inner_mode` open 后 work 前单点；lib 无 `env::var`（Forbidden 守住）。
- **测试 11 例与契约 witness 一一对应**（默认值/lib 触发有界/重开恢复/纯 SELECT 零写入/0 禁用/显式事务顺延/失败传播/CTAS 边界/env 触发/env 0 禁用/env 非法三形态）；env 触发见证的存活期采样形态（D3）系终态收敛下唯一可判别方案，负载按 `MAX_ARG_STRLEN` 定尺、stderr 落文件防管道阻塞——合理。

**裁定项（Act 请求与豁免，2026-09-30 用户授权「给出豁免」）**：

1. **D1（死接口测试消费方删除）——裁定：删除 SHALL 优先，非阻塞**。Plan Investigation Facts「全仓零消费方（src+tests）」**不成立**（`wal_writer_test::test_checkpoint_threshold`、`checkpoint_test::test_checkpoint_threshold_trigger` 两处测试消费方）——分类为 **Plan 调查错误**（沿自 change design 调查事实 9，本轮 Plan 未复核 tests 面），Act 实读发现并如实上报，处置正确。R5 的「删除 SHALL（无条件）」与「既有测试零修改」对这两例本身互斥；零修改条款保护的是行为覆盖测试不被静默语义漂移，而被测对象整体消失的用例无此保护需求，且 `checkpoint_test::test_checkpoint_flow` 保持 `CheckpointManager` + `reset_write_count` + 位点截断覆盖。R5 Scenario「零修改」按「存留测试零修改」解释。
2. **Minor 1（非法 env 对不开库子命令亦 exit 2）——裁定：豁免，维持顶部守卫**。Forbidden 条款的letter被违反（Act 自报），但：(a) `RTSQL_KEY` 空值校验在同一位置（`execute_command` 顶部、:217-220）已对全部命令生效——全局 env 校验于分派前是本 CLI 既有纪律；(b) 单点守卫满足「早于 open」与「早于父目录创建」双重约束，替代方案需引入命令分类谓词、收益边际；(c) 诚实 fail-closed 优于部分路径静默忽略。
3. **新发现 Minor（非阻塞，记录待未来顺带修正）**：`src/cli/mod.rs:222-224` 守卫注释称「校验只对开库路径生效……故不施加该校验」，与代码实际行为（无条件全局校验）**相反**——误导性注释，零行为影响；本 Review 按「非阻塞 Minor 不创建修复」记录，未来触碰该区域的 change 顺带改正（应改述为 RTSQL_KEY 同位全局纪律）。
4. Minor 2（计数器口径为「本会话自 open 起追加字节」）——spec 选定形态（proposal 假设 9 内存计数器）的必然边界，重开大 WAL 库由既有恢复 + close checkpoint 兜底，接受。Minor 3（`Commit failed:` 前缀）——契约指定形态，接受。D7（批次台账更正）——诚实报告，权威批次覆盖成立。

**Acceptance Gaps**

None——wal-auto-checkpoint R1–R5 全部场景有代码与测试见证对应。

**Evidence**

- 本 Review 新增检查：上述独立 diff 审读（writer/checkpoint/database/pipeline/cli 五文件变更面全文、CTAS 边界测试正文、CLI 守卫位置与注释定点核对、测试用例清单比对）。
- 采信 Act Response 验证结论（2026-09-30，用户对既有轮次裁定「测试可信」、覆盖面未失效）：RED 8 处 E0599 → GREEN；死接口 grep 零匹配；受影响边界（checkpoint/wal_writer/recovery_e2e/wal_recovery_large/ctas/cte/explicit_tx）0 failed；全量 M19 权威分批 **1590 passed / 0 failed**（1250 集成 + 340 lib，计数与 +11/−2 自洽）；clippy/fmt/validate clean。

**Follow-up Decision**

既有 Acceptance 全部满足；D1 裁定删除 SHALL 优先、Minor 1 经用户授权豁免、其余 Minor 均留档不返工 → `accepted`。

**Iteration Plan Update**

None——Iteration Map 不变。

**Next Cycle**

None。

**Next Iteration**

None——本 Iteration 为 change Iteration Map 的最后一个（执行序 000 → 001 → 003 → 002 全部 accepted），change 进入收尾（增量规格合并与归档由 openspec-docs-maintainer 按用户 2026-09-30 指令执行）。
