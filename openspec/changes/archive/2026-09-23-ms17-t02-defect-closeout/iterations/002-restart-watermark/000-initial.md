# Iteration 002 / Cycle 000: restart-watermark 初始执行

## Plan Context

- Status: ready
- Iteration: 002-restart-watermark
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T7, T8
- Depends on: Iteration 001（visibility-summary-closeout——Plan Review accepted，2026-09-23；全量 1061 稳定门）
- Stable baseline: 干净关闭重开后 RC 高水位健全（已提交行立即可见、重启后 id 不复用、位点文件向后兼容读取）；change 二次收口
- Verification boundary: 位点单测 + RC 重开 e2e（RED 先行）全绿 + checkpoint/恢复既有套件零修改（`checkpoint_test.rs` 预授权签名适配点除外，断言集不变）+ T8 全量验证记录
- Diagnostic boundary: `src/wal/checkpoint.rs`（位点读写 + checkpoint 水位捕获）、`src/wal/recovery.rs::full_recover`、`src/database.rs::{open,checkpoint}`、`tests/{isolation_level_test,checkpoint_test}.rs`
- Deferred tasks: None

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal What Changes 7/8 + design D8（位点 16B→24B、水位捕获时机、兼容读、拒绝替代方案 e/f）+ delta spec `transaction-isolation-levels` 新增 Requirement S1-S4；Iteration 001 稳定基线（全量 1061 一次绿 + accepted Review，采信其 Act Response）
- Excluded scope: WAL 帧格式与主库文件头（零触碰）；数据页派生水位（D8 拒绝）；加密域（MS17-T01）；ISS 台账落账（Recorder 流程）；页级摘要（Iteration 001 已收口）

**Objective**

checkpoint 截断 WAL 后，恢复对分配器水位的认知不再依赖 WAL 观测——位点文件携带 tx watermark，重开后 `advance_past(max(WAL 观测, 位点水位))`，RC 语句高水位恢复健全：干净关闭重开已提交行立即可见、重启后 id 不复用、16B 旧位点/无位点路径行为不变；随后 change 二次收口（T8）。

**Background**

Iteration 001 T5 夹具实施期发现：`close()`（checkpoint 截断 WAL）→ reopen `redo_count == 0` → 恢复观测不到事务 id → `advance_past(0)` no-op → 分配器归零，而 RC `statement_view` 以分配器当前值为高水位（`database.rs:77-81` 注释自证的健全性前提失效）→ 重启前已提交行对 RC 不可见、随新分配渐进「复现」。Plan Review 独立读码证实机理链（Iteration 001 Plan Review F2，NEW-EVIDENCE）。用户 2026-09-23 裁定作为 Iteration 002 并入本 change 修复（proposal 用户决策 5，取代 Iteration 001 Review 的 `Next Iteration: None` 记录）。

**Investigation Facts**

- Current Baseline: master `7364bc9` + MS13 实施 + docs sync + 本 change T1-T6 实施（均未提交）；**1061 tests pass / 0 failed / 2 ignored**（Iteration 001 T6 全量 `--no-fail-fast` 一次绿，本 Iteration 未修改其覆盖范围内表面，采信）；clippy/fmt 0、validate PASS（changes 1 + specs 36）。
- Current-State Evidence:
  - **缺陷链（全部本会话读码/探针实证）**：`Database::open` `database.rs:79-86` 以 `recovery_result` 三集（committed/aborted/uncommitted）取 max 喂 `advance_past`——分配器水位唯一恢复来源为 WAL 重放观测；checkpoint 九步（`checkpoint.rs:94-133`）步骤 7 `rewrite_truncate(lsn)` 保留 [lsn..end)（含 Checkpoint 帧）后，干净 close 残余 WAL 无 Begin/Commit 帧 → 三集皆空 → max=0 → 分配器从零起步；`statement_snapshot` RC 臂（`database.rs:127-139`）以 `current_tx_id()` 为高水位。运行时实证：`tests/isolation_level_test.rs` T5 夹具首跑前置断言 `left: 0, right: 3`（close+reopen RC SELECT 0 行）；RR 同形 reopen 有行。
  - **位点文件面**：`<db>.checkpoint` 伴生文件，16B = lsn u64 LE + timestamp u64 LE；`read_site_file`（`checkpoint.rs:23-43`，pub(crate) 自由函数，<16B → None；恢复端 `recovery.rs:402` 与 `CheckpointManager::read_checkpoint_site` 共享语义）；`write_checkpoint_site`（`checkpoint.rs:67-86`，truncate+write+sync_all）。两次位点写入：步骤 5 截断前 (lsn, ts)、步骤 8 截断后 (0, ts2)。
  - **调用链收口**：`close()`（`database.rs:228-230`）→ `Database::checkpoint()`（`:234-240`，pub 签名不变）→ `CheckpointManager::checkpoint()`（`checkpoint.rs:94`，pub——**唯一生产接线点**，CLI 优雅停机 `cli/mod.rs:519` 经 `db.checkpoint()` 同源）；`RecoveryResult`（`recovery.rs:19-23`）唯一消费方为 `database.rs` open。
  - **水位捕获点可用性**：`TransactionManager::current_tx_id()`（`manager.rs:223-225`，同步读分配器当前值）；`Database` 持有 `transaction_manager`（`statement_snapshot` 已用）。
  - **测试适配面**：`tests/checkpoint_test.rs` 3 处直接构造 `CheckpointManager::new`（:15/:50/:75）+ 1 处直接调用 `manager.checkpoint()`（:53）；`tests/checkpoint_redo_reduction_test.rs`（:224/:253/:286）与 CLI 全走 `db.checkpoint()`（Database 层签名不变，零适配）。位点消费测试基建：`recovery.rs:406-409` `redo_from` 过滤（位点 lsn > wal_len 代际失效 → 全量重放）既有语义。
  - **Iteration 001 T5 夹具现状**：`rc_scan_then_delete_keeps_remaining_rows_reachable_after_restart` 含 scratch 表 6 INSERT+6 DELETE 抬水位 workaround（doc 注记「pre-existing issue, reported separately」）——本 Iteration 修复后移除（doc 同步改写），断言集不变。
- Code and Critical Path:
  - 变更面（产品代码 3 文件）：`src/wal/checkpoint.rs`（`read_site_file` 24B 兼容读 / `write_checkpoint_site` 增水位参数 / `CheckpointManager::checkpoint` 增 `tx_watermark: impl Fn() -> u64` 于 LSN 捕获后调用，两次位点写入携带 / 新 `#[cfg(test)]` 位点单测）、`src/wal/recovery.rs`（`RecoveryResult` 增 `checkpoint_tx_watermark: Option<u64>`，`full_recover` 从位点直填）、`src/database.rs`（open 合并 max、checkpoint 传闭包 `|| self.transaction_manager.current_tx_id()` + doc）。
  - 数据流：checkpoint 时（LSN 捕获后）读分配器 → 位点 24B 落盘 → 重开恢复读位点 → `advance_past(max(WAL 观测, 水位))` → RC 语句高水位 = 分配器当前值 ≥ 一切历史 id。
  - 健全性推演（design D8）：位点前缀内落盘 Begin 的事务 id ≤ 水位（id 分配 → Begin 写入 → LSN 捕获 → 水位读取的时序偏序）；水位读取后分配的 id 其 Begin 落 offset ≥ lsn（WAL 追加只写）→ 重放尾部观测覆盖。任意崩溃点 max(WAL, 水位) ≥ 一切已分配历史 id。

**Implementation Guidance**

顺序：位点单测与 24B 读写（先观察 RED：新断言对现 16B 实现失败/编译期演进）→ `CheckpointManager::checkpoint` 闭包参数与两次位点写入 → `RecoveryResult` 字段 + `full_recover` 直填 → `Database::open` 合并 + `Database::checkpoint` 闭包 → e2e（先 RED：重开 RC SELECT 0 行）→ `checkpoint_test.rs` 机械适配 → T5 夹具移除 scratch（doc 改写，断言不动）→ T8。位点兼容读三分支（≥24B / 16B..24B / <16B）为纯函数逻辑，建议以手工构造文件字节直测。

**Behavioral Change**

- 修复前：干净 close → 重开，RC 语句高水位 ≈ 0，重启前已提交行不可见（随新 DML 渐进复现）；`checkpoint_test.rs` 以 16B 位点语义运行。
- 修复后：位点文件 24B 携带水位；重开即 `advance_past` 越过重启前一切 id；RC 首条语句即可见全部已提交行；16B 旧位点/无位点/撕裂短写路径行为与现状一致（无水位 → 不差于现状）。RR、redo 过滤、WAL 帧格式、主库文件头零变化。

**Task Contracts**

### T7: RC 重启可见性高水位修复（checkpoint 位点水位）

- Requirement/Scenario: transaction-isolation-levels（delta）新增 Requirement「RC 重启后可见性高水位健全（checkpoint 水位持久化）」S1（干净重开立即可见）/ S2（位点往返与旧格式兼容）/ S3（重启后 id 不复用）/ S4（既有 checkpoint/恢复语义零回归）
- Depends on: None（Iteration 001 稳定门为 Iteration 级依赖）
- Targets: `src/wal/checkpoint.rs::read_site_file`、`write_checkpoint_site`、`CheckpointManager::checkpoint`；`src/wal/recovery.rs::RecoveryResult`、`full_recover`；`src/database.rs::open`（advance_past 合并点）、`checkpoint`（闭包传递）
- Current behavior: 位点 16B 无水位；干净 close 重开 RC `SELECT` 返回 0 行（Iteration 001 夹具前置断言失败现场）；`checkpoint_test.rs` 3 构造 + 1 调用点以现签名运行
- Required behavior: 位点 24B（lsn u64 LE + timestamp u64 LE + tx watermark u64 LE）；`read_site_file` ≥24B 携带水位 / 16B..24B 旧格式无水位 / <16B None；`CheckpointManager::checkpoint` 在 LSN 捕获后调用 `tx_watermark()` 并于步骤 5/8 两次位点写入携带；`full_recover` 填 `checkpoint_tx_watermark`；`Database::open` `advance_past(max(WAL max_tx_id, watermark.unwrap_or(0)))`；`Database::checkpoint` 传 `|| self.transaction_manager.current_tx_id()`；干净 close 重开 RC `SELECT * FROM t` 立即返回三行；重开后新 INSERT id > 水位且新旧行同语句共存
- Required changes: 上述五点符号级变更 + `CheckpointManager::checkpoint`/`write_checkpoint_site` doc 注记水位语义与捕获时机；`tests/checkpoint_test.rs` 直接构造/调用点适配新签名（断言集不变）；`tests/isolation_level_test.rs` 新增 e2e + T5 夹具移除 scratch 块（doc 注记改写为修复后语义）
- Preserve: WAL 帧格式与 `WalRecord::Checkpoint { lsn, timestamp }` 结构（位点扩展不进 WAL 帧）；`rewrite_truncate` 本体与九步次序（仅位点写入内容扩展）；位点撕裂写安全退化语义（<16B → None）与 `redo_from` 过滤（lsn > wal_len 代际失效全量重放）；`CheckpointManager::new` 签名；`Database::checkpoint`/`close` pub 签名；`RecoveryManager::recover`（非 full）签名与行为；`advance_past` 本体；`TransactionManager::current_tx_id`；行级 MVCC 可见性语义与 Iteration 001 哨兵修复面；RR 路径行为
- Forbidden: 水位经调用前捕获的参数传入（必须在 LSN 捕获后经闭包读取——D8 健全性关键）；修改主库文件头或 WAL 帧格式；数据页派生方案；加密相关面；`checkpoint_redo_reduction_test`/`recovery_test`/`recovery_e2e_test` 修改；放宽或删除 `checkpoint_test.rs` 既有断言
- Test witness（RED 先行）:
  - `tests/isolation_level_test.rs` 新 e2e `rc_reopen_sees_committed_rows_without_new_dml`（镜像 T5 夹具：作用域内 open RC + 建表 3 行已提交 + 显式 `close()` → 重开 RC → 立即 `SELECT * FROM t` 断言 3 行——现状 0 行 RED；追加 S3 见证：重开 INSERT 新行（不同 PK）后同语句 SELECT 返回 4 行）
  - `src/wal/checkpoint.rs` 新 `#[cfg(test)]` 位点单测（tempdir + 手工字节构造）：24B 往返（write 带水位 → read 携带水位且 lsn/ts 不变）；16B 旧格式（手工写 16 字节）→ 读为无水位；<16B → None（现状语义保持断言）——对现 16B 实现 RED（新 API/新断言形态）
  - `checkpoint_test.rs` 适配点仅签名（断言零变化）
  - T5 夹具 `rc_scan_then_delete_keeps_remaining_rows_reachable_after_restart`：移除 scratch 块后全绿（毒化链见证不依赖 scratch——scratch 只写 scratch 页，t 页条目状态不受影响；doc 注记改写）
- GREEN condition: 上述新用例全绿 + `checkpoint_test`（适配后断言集不变）+ `checkpoint_redo_reduction_test`（9）+ `recovery_test` + `recovery_e2e_test` 零修改全绿 + `isolation_level_test` 全绿（既有 8 + 新 1）
- Verification: `cargo test --test isolation_level_test`、`cargo test --test checkpoint_test`、`cargo test --test checkpoint_redo_reduction_test`、`cargo test --test recovery_test --test recovery_e2e_test`、`cargo test --lib wal`（决定性输出 ≤10 行/项，退出码 0；RED 记录写入 Act Response）
- Stop when: e2e 修复后仍 0 行且排除夹具问题（作用域/锁/时序——结构链断裂返回 Plan，不得以插入 DML 造假见证）；位点兼容读与 `redo_from` 消费语义冲突（互斥不可调和）；`checkpoint_test` 适配暴露断言依赖 16B 位点形状（需 Review 裁定，不得静默放宽）

### T8: 追加收尾全量验证与结构自检

- Requirement/Scenario: 全部域零回归（RTM「全部域零回归（追加）」行，design D7/D8）
- Depends on: T7
- Targets: change 级验证，无产品代码变更面
- Current behavior: Iteration 001 末全量 1061 一次绿（采信）；clippy/fmt 0、validate PASS
- Required behavior: T7 合入后全量零回归 + 静态门 + 结构自检刷新
- Required changes: 无代码变更；执行全量 `cargo test --no-fail-fast`（基线 1061 + T7 新增，零回归）、`cargo clippy --all-targets -- -D warnings` 0、`cargo fmt --check` 0、`openspec validate` PASS；结构自检——tasks T1-T8 状态与实际一致、delta spec（5 个域）与 design D1-D8 与实现一致、3 Iteration × Cycle 文件齐全、Review Result 000/001 accepted / 002 pending（本 Response）
- Preserve: 既有测试套件零修改（T7 预授权适配点与新用例除外）
- Forbidden: 为判定验证结果新增脚本/封装/判定器；重跑已通过验证增强信心
- Test witness: 各命令决定性输出与退出码记入 Act Response（全量输出 ≤20 行）
- GREEN condition: 全量 0 failed、clippy/fmt 0、validate PASS、自检各项一致
- Verification: 上述四命令 + 结构自检对照（逐项在 Act Response 列结论）
- Stop when: 全量出现非预期失败（先对照 Iteration 001 Act Response Changed Files 与 T7 变更面定位归属，再判回归或基线漂移；无法归属时返回 Plan）

**Invariants**

- WAL 帧格式、`rewrite_truncate`、checkpoint 九步崩溃窗口次序（位点先于截断、截断后位点置 0）不变。
- 位点撕裂写安全退化语义保持（<16B → None → 全量重放）；`redo_from` 过滤语义不变。
- RR 无快照路径逐字节不变；行级 MVCC 可见性语义零触碰（本 Iteration 只恢复分配器水位信息，不改任何可见性规则）。
- `Database::checkpoint`/`close` 公共 API 签名不变；plan cache、退出码分类不变。
- Iteration 001 哨兵修复面（`page_visibility`/`buffer_pool`）零触碰。

**Non-goals**

- WAL 帧格式与主库文件头变更（水位走位点伴生文件）。
- 数据页派生水位（design D8 拒绝方案 e）；加密域（MS17-T01，含位点加密）。
- ISS 台账落账与 improvement 候选登记（Recorder/docs-maintainer 按用户指令）。
- 位点撕裂写窗口的缺陷残留消除（16..23B 按旧格式无水位——保守，不差于现状；spec Sad 面已锁定）。

**Acceptance**

- T7：delta spec S1-S4 全部有测试见证——S1 新 e2e（重开即见 3 行 + 新 INSERT 共存 4 行）、S2 位点单测三分支、S3 e2e 内嵌（新行可见即新 id > 水位越过旧行 create_tx 的行为后果）+ S1 同证、S4 既有套件零修改 + 适配点断言集不变（requirement→scenario→design D8→T7→代码→测试链路）。
- T8：全量 `--no-fail-fast` 零回归（基线 1061 + 新增）+ clippy/fmt 0 + validate PASS + 结构自检通过。

**Verification**

- T7：`cargo test --test isolation_level_test` → 既有 8 + 新 1 全绿，exit 0；`cargo test --lib wal` → 位点单测全绿，exit 0；`cargo test --test checkpoint_test` → 适配后断言集不变全绿；`cargo test --test checkpoint_redo_reduction_test`（9）与 `--test recovery_test --test recovery_e2e_test` → 零修改全绿。RED 记录（重开 0 行 / 位点新断言对 16B 实现）写入 Act Response。
- T8：`cargo test --no-fail-fast`（决定性输出 ≤20 行）；`cargo clippy --all-targets -- -D warnings` → 0；`cargo fmt --check` → 0；`openspec validate` → PASS。判定以被测命令退出码与原生输出为准，无判定层。

**Gate 2 Readiness**

- 无 Missing requirement：RTM 新增 2 行 Covered（transaction-isolation-levels 新 R 四场景 + 追加零回归）（PASS）。
- 无 Simplified requirement（PASS；撕裂写窗口残留为显式 Non-goal + spec Sad 面，非简化）。
- 调查完整：缺陷链/位点面/调用链/捕获点/适配面五点读码实证，行号本 Cycle 全部复核（PASS；Investigation Facts）。
- 设计闭合：D8 位点扩展 + 水位捕获时序推演 + 崩溃窗口演化 + 兼容读三分支 + 拒绝替代方案 e/f（PASS；design D8）。
- 任务可执行：T7/T8 契约有代码位置、行为变化、RED 见证与停止条件（PASS；Task Contracts）。
- 分轮合理：Iteration 002 单独成轮（001 已 accepted 冻结 + 独立验收边界与诊断面），T8 收口并入（PASS；tasks.md Iteration Plan 平衡审计）。
- 追踪完整：requirement→scenario→design→task→代码→测试链路齐备（PASS；RTM）。
- 验证充分：S1-S4 逐场景最简直接判定（e2e 行断言 + 位点字节单测 + 套件零修改），无身份型证据工程/判定层（PASS）。
- 无需 Act 决定的实质未知项：e2e RED 可构造性有 Iteration 001 夹具现场直接支撑（同形态前置断言失败即 RED）；构造失败走 T7 Stop when（PASS；残余不确定项入 Risks）。
- OpenSpec tasks/specs/design/当前 Iteration/Cycle 一致（PASS；本次同源写入——tasks T7/T8、Iteration 002、delta spec、design D8、本 Cycle）。
- Persisted Evidence 明确：`none`（PASS；各验证输出 ≤20 行可入 Act Response，无不可低成本重跑项）。
- 计划批准：范围与纳入经用户 2026-09-23 指令批准（proposal 用户决策 5 原话记录）；D8 设计与任务契约经用户 2026-09-23 审计批准（「批准」）——Status 已置 `ready`。

**Persisted Evidence**

- Mode: none

（Act Response 承载全部验证输出；无不可重跑项，无 Evidence 预算消耗。）

- Budget: 按公共规则 Evidence 预算执行（0 文件）。

**Risks and Notes**

- 撕裂写窗口（位点落盘 16..23B）按旧格式解析无水位 → 该次重开缺陷残留（不差于现状）；位点写本就 truncate+write+sync_all，概率窗口极小，不为本加原子化机制（身份型证据工程边界外，属过度工程）。
- 水位捕获时机是健全性关键：契约 Forbidden 已禁参数预捕获形态；Review 时按此核对 diff。
- T5 夹具移除 scratch 后若毒化链见证失效（如置位时序变化导致 t 页有条目、`or_insert` 臂不可达）→ T7 Stop when 返回 Plan，不得以恢复 scratch 之外的放宽断言处理。
- 新 e2e 的首会话须显式作用域 + `close()`（advisory 锁不随遮蔽 drop——Iteration 001 Deviations 1a 实证）。
- 非实质选择：位点读取返回形态（扩展元组 vs 小结构体）与单测组织位置留给 Act 按仓库先例就近决定。

## Act Response

- Status: reported

**Implemented**

- T7（位点 24B + 水位贯通 + 消费合并，产品代码 4 文件）：
  - `CheckpointSite` 新 pub 结构体（lsn / timestamp / `tx_watermark: Option<u64>`，Copy——恢复端 match 按值消费所需）；`wal/mod.rs` 导出。
  - `read_site_file` 兼容读三分支：≥24B 携带水位 / 16B..24B 旧格式无水位 / <16B None（既有撕裂安全退化原样保留）。
  - `write_checkpoint_site(lsn, timestamp, tx_watermark)` 写 24B（truncate+write+sync_all 序不变）。
  - `CheckpointManager::checkpoint(tx_watermark: impl Fn() -> u64)`：步骤 1 LSN 捕获**之后**步骤 1b 调用闭包读取水位（D8 健全性关键，非实质新步骤、无副作用只读观测），步骤 5 与步骤 8 两次位点写入均携带；九步崩溃窗口次序不变。方法与写入函数 doc 注记水位语义、捕获时机与禁止形态。
  - `RecoveryResult` 增 `checkpoint_tx_watermark: Option<u64>`；`full_recover` 从位点直填（`records.is_empty()` 早退路径同样携带；水位独立于 `redo_from`——代际失效只作废 lsn 过滤，`redo_from` 过滤逻辑本身零改动）。
  - `Database::open`：`advance_past(max(WAL 观测最大 id, 位点水位.unwrap_or(0)))`；`Database::checkpoint` 传 `|| self.transaction_manager.current_tx_id()`（pub 签名不变，CLI 优雅停机同源受益）。
- T7（测试见证）：
  - `tests/isolation_level_test.rs` 新 e2e `rc_reopen_sees_committed_rows_without_new_dml`：显式作用域 + `close()` → 重开 RC 立即 `SELECT * FROM t` 断言 3 行（S1）+ 重开 INSERT (4,40) 后同语句 SELECT 断言 4 行（S3）。**RED 现场观察**：修复前首断言 `left: [], right: [[1,10],[2,20],[3,30]]`（0 行），与 Plan 预测逐字一致。
  - `src/wal/checkpoint.rs` 新 `#[cfg(test)]` 位点单测 3 项（S2）：24B 往返（write 带水位 42 → read 全等）/ 16B 旧格式手工字节 → 无水位 / 8 字节短写 → None。**RED 现场观察**（编译演进）：`cannot find struct CheckpointSite` + `takes 2 arguments but 3 supplied` + `no field lsn on type (u64, u64)`。
  - T5 夹具 `rc_scan_then_delete_keeps_remaining_rows_reachable_after_restart` 移除 scratch 抬水位块（12 条 DML），doc 注记改写为修复后语义，断言集零改动，移除后全绿。
- T8：全量 + 静态门 + validate + 结构自检（见 Verification Evidence；tasks T1-T8 状态行已同步 done）。

**Changed Files and Symbols**

| 文件 | 符号/区域 |
|---|---|
| `src/wal/checkpoint.rs` | 新 `CheckpointSite`；`read_site_file`（24B 兼容读）；`CheckpointManager::write_checkpoint_site`（3 参 24B）；`CheckpointManager::checkpoint`（增 `tx_watermark` 闭包参 + 步骤 1b + 两次位点写入）；新 `mod tests`（3 单测） |
| `src/wal/mod.rs` | 导出 `CheckpointSite` |
| `src/wal/recovery.rs` | `RecoveryResult.checkpoint_tx_watermark` 新字段；`full_recover`（位点直填 + `redo_from` match 结构化） |
| `src/database.rs` | `open_with_isolation`（`advance_past(max(wal_observed_max, site_watermark))`）；`checkpoint`（闭包传递 + doc） |
| `tests/isolation_level_test.rs` | 新 `rc_reopen_sees_committed_rows_without_new_dml`；T5 夹具 scratch 块移除 + doc 改写 |
| `tests/checkpoint_test.rs` | 签名机械适配（`write_checkpoint_site` 3 参；2 处 `checkpoint(\|\| 0)`；3 处位点读改字段访问），断言集零变化 |
| `tests/recovery_test.rs` | `test_recover_from_checkpoint` 1 处 `checkpoint(\|\| 0)` 签名适配，断言零变化（见 Deviations 1） |

**Deviations from Plan**

1. `tests/recovery_test.rs:79` 直连 `manager.checkpoint()` 一处签名适配——Plan Investigation Facts 测试适配面遗漏该调用点（记录为「checkpoint_test.rs 3 构造 + 1 调用」）。同一 `checkpoint(|| 0)` 机械适配类、断言集零变化，`recover`（非 full）路径与断言零触碰；`checkpoint_test.rs:88` 实为第 2 处直连调用（Plan 记 1 处），同批适配。两处均为可直定位的机械签名适配，不构成实质问题，未触发 Stop when。
2. 位点读取返回形态选用小结构体 `CheckpointSite`（Plan 预留 Act 决定项）：3 字段含 1 个 Option，命名字段与仓库先例（`PageVisibilityInfo`/`RecoveryResult`）一致，恢复端 `site.and_then(|s| s.tx_watermark)` 可读。
3. 过程记录（非偏差，透明披露）：首次全量运行 exit 0 但输出截取丢失聚合数；为完成证据记录补一次全量捕获（非信心重跑）；fmt 检出新增代码格式漂移并格式化后，对字节级最终树再跑一次全量——**最终记录以最终态运行为准**。位点单测首跑 1 夹具错误（tempdir 目录误作 db 文件路径 → `IsADirectory`），修正为目录下文件路径后全绿（产品代码零改动）。

**Blocker Handoff**

None required

**Blocker Resolution**

None

**Self-Review**

- Plan compliance: PASS——S1-S4 逐场景见证齐全（S1 e2e / S2 三单测 / S3 e2e 内嵌 / S4 既有套件零断言变化）；契约五点符号级变更逐项落地；Preserve 面逐项核对（WAL 帧格式、`WalRecord::Checkpoint`、`rewrite_truncate` 本体与九步次序、`CheckpointManager::new`、`Database::checkpoint/close` pub 签名、`RecoveryManager::recover`、`advance_past`、`current_tx_id`、RR 路径、Iter001 哨兵修复面均零触碰）；Forbidden 逐项核对（水位为闭包 LSN 后读取，非预捕获参数；未触文件头/帧格式/数据页派生/加密面；`checkpoint_redo_reduction_test` 与 `recovery_e2e_test` 零修改；`checkpoint_test.rs` 断言未放宽）。
- Full diff reviewed: PASS——完整 diff 逐 hunk 复查（含 fmt 后终态重读 `checkpoint.rs` 全文与 `recovery.rs`/`database.rs` 变更区）：变更面严格限于 7 文件；调用面闭合（`write_checkpoint_site`/`read_site_file`/`checkpoint`/`RecoveryResult` 全部消费点核对）；跨任务交互（T5 夹具与新 e2e 独立 tempdir 无干扰；步骤 1b 只读观测不改变崩溃窗口语义）；无计划外修改。
- Critical findings unresolved: 0
- Important findings unresolved: 0
- Minor findings unresolved: 0（过程中修复 2 项：单测夹具路径错误、fmt 格式漂移——均已终态全量重验覆盖）

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| T7 RED（行为） | `cargo test --test isolation_level_test rc_reopen_sees_committed_rows_without_new_dml`（修复前） | `assertion left == right failed: S1: ... got []`；FAILED. 0 passed; 1 failed | RC 干净重开高水位缺陷现场 | RED ✅ |
| T7 RED（位点 API） | `cargo test --lib wal`（新单测对旧实现） | `error[E0422]: cannot find ... CheckpointSite`、`error[E0061]: ... 3 arguments`（编译演进） | 位点新 API/新断言形态 | RED ✅ |
| T7 GREEN（位点单测） | `cargo test --lib wal` | `test result: ok. 8 passed; 0 failed`（5 record + 3 位点），exit 0 | `read_site_file` 三分支 + 24B 写读 | PASS |
| T7 GREEN（隔离 e2e） | `cargo test --test isolation_level_test` | `test result: ok. 9 passed; 0 failed`（既有 8 + 新 1，含 T5 夹具去 scratch），exit 0 | S1/S3 + 哨兵链回归见证 | PASS |
| T7 GREEN（适配面） | `cargo test --test checkpoint_test` | `test result: ok. 3 passed; 0 failed`，exit 0 | 适配后断言集不变 | PASS |
| T7 GREEN（零修改面） | `cargo test --test checkpoint_redo_reduction_test` / `--test recovery_test --test recovery_e2e_test` | `9 passed` / `3 passed` + `6 passed`，exit 0 | 位点消费过滤 / 恢复语义零回归（S4） | PASS |
| T8 全量 | `cargo test --no-fail-fast`（终态树） | `passed=1065 failed=0 ignored=2`（77 套件全 ok；= 基线 1061 + T7 新增 4），FULL_EXIT=0 | 全部域零回归 | PASS |
| T8 clippy | `cargo clippy --all-targets -- -D warnings` | `Finished dev profile`，0 warning（仅 cargo config 弃用环境提示），exit 0 | 全部 target | PASS |
| T8 fmt | `cargo fmt --check` | 无 diff 输出，exit 0 | 全部 .rs | PASS |
| T8 validate | `openspec validate --all` | `Totals: 37 passed, 0 failed (37 items)`（36 specs + 1 change），exit 0 | specs + change | PASS |

**Persisted Evidence**

None required（Plan Context Mode: none；全部验证输出 ≤20 行已入本 Response，无不可低成本重跑项）

**Experience Candidates**

None

**Remaining Issues**

None

**Commit or Diff Reference**

未 commit（工作区含基线 MS13 实施 + docs sync + 本 change T1-T6 与本 Cycle 改动，待用户统一提交；对照基线 7364bc9）。本 Cycle 改动以 Changed Files 清单 7 文件为准。

## Plan Review

**Findings**

- **F1（非阻塞，Deviation 1 分类）**：Plan Investigation Facts 测试适配面记「checkpoint_test.rs 3 构造 + 1 调用」，遗漏 (a) `checkpoint_test.rs:88` 实为第 2 处直连 `checkpoint()`、(b) `tests/recovery_test.rs:79` 直连调用——归 `PLAN-OMISSION`（Plan 侧适配面清单不完整）。Act 适配为同一机械签名类 `checkpoint(|| 0)`、断言集零变化、`recover`（非 full）路径零触碰（本 Review 读码核对 `recovery_test.rs:79` 后续断言原样），不触发 Stop when，不阻塞 Acceptance。
- **F2（非阻塞，Deviation 3 过程记录）**：全量共跑三次——#1 exit 0 输出聚合数丢失、#2 补捕获、#3 fmt 漂移修复 + 夹具修正后对最终树运行并为正式记录。判定：#3 是对修改后终态的新鲜验证（fmt/夹具修正属修改后必需重跑，非信心重跑）；#2 属中间树态的证据补捕获，过程噪声已透明披露，最终记录有效性不受影响。后续以「先捕获输出再声明通过」避免。位点单测首跑夹具路径错误（tempdir 目录误作 db 文件）为测试代码修复，产品代码零改动，合规。
- **F3（记录，无需动作）**：`full_recover` 的 WAL 文件不存在早退路径（`recovery.rs:400-402`）返回 `RecoveryResult::default()`（水位 None）——该形态仅新库/无历史，无 id 需保护，与修复前行为逐字节一致，非回归。代际失效位点（`s.lsn > wal_len`）仍消费水位属安全方向：分配器水位只要求 ≥ 历史已分配 max，过度推进仅跳号无危害（design D8 推演一致）。
- **独立代码检查**：7 文件终态逐行核对与 Task Contract/Act Response 声明精确一致——`checkpoint.rs`：`CheckpointSite`（Copy + Option 水位）、`read_site_file` 三分支（≥24B 携带 / 16B..24B None / <16B None，`:44-54`）、`write_checkpoint_site` 24B（truncate+write+sync_all 序不变，`:95-113`）、`checkpoint` 步骤 1 LSN 捕获 → **步骤 1b 闭包读取**（`:133-136`，D8 健全性关键形态正确）→ 步骤 5/8 两次位点写入均携带（`:154/:169`）、九步次序保持（1b 为只读观测不改变崩溃窗口）、doc 注记水位语义与禁止形态、3 位点单测与 S2 逐字对应；`wal/mod.rs:13` 导出；`recovery.rs:28/:407/:411-417/:422-427/:499-505` 字段 + 直填 + `redo_from` match 语义保持（`s.lsn <= wal_len` 既有判定原样）+ 空 records 早退携带水位；`database.rs:79-94`（三集 wal_observed_max 既有逻辑保持 + max(site_watermark) 合并 + doc）与 `:248-254`（闭包 `|| self.transaction_manager.current_tx_id()`，pub 签名不变）；`close()`（`:236-238`）零改动。Preserve 面核对：`WalRecord::Checkpoint { lsn, timestamp }` 帧结构未触碰（`:157` 构造原形）、`CheckpointManager::new`、`advance_past`/`current_tx_id` 本体、`recover`（非 full）、RR 路径、Iteration 001 哨兵修复面均不在变更面。Forbidden 核对：水位非调用前捕获参数（闭包在方法内 LSN 后读取）✓、主库文件头/WAL 帧格式零触碰 ✓、`checkpoint_redo_reduction_test`/`recovery_e2e_test` 不在 Changed Files ✓、`checkpoint_test.rs` 断言未放宽 ✓。调用面闭合（grep 全量）：`read_site_file` 唯一生产消费点 `recovery.rs:407`；`CheckpointManager::checkpoint` 生产唯一 `database.rs:250` + 测试 3 处（与 Deviation 1 披露精确一致）；`db.checkpoint()` CLI 优雅停机 `cli/mod.rs:519` 经不变 pub 签名同源受益；`should_checkpoint` 无生产自动触发调用点（`writer.rs:165` 定义 + 测试手动使用），无遗漏适配面。

**Deviation Classification**

- Deviation 1（适配面清单外 2 处签名适配）：`PLAN-OMISSION`——非阻塞（F1），机械类、断言零变化。
- Deviation 2（位点返回形态小结构体）：非偏差——Plan Risks 预留 Act 决定项，命名与仓库先例（`PageVisibilityInfo`/`RecoveryResult`）一致。
- Deviation 3（过程三次全量 + 单测夹具修复）：非偏差（F2），终态记录有效。

**Acceptance Gaps**

None —— T7：S1 ↔ `rc_reopen_sees_committed_rows_without_new_dml` 首断言（重开即见 3 行，RED 现场 `left: []` 与 Plan 预测逐字一致）、S2 ↔ 位点单测 3 项（24B 往返全等 / 16B 旧格式无水位 / 8B None）、S3 ↔ e2e 尾段（INSERT (4,40) 后同语句 4 行共存）、S4 ↔ 既有套件零修改 + 适配点断言集不变 + 全量零回归，requirement→scenario→design D8→T7→代码→测试链路闭合（本 Review 逐点读码核对）；T5 夹具 scratch 移除后毒化链见证独立成立（doc 改写 `:304-308`、断言集不变）。T8：全量 1065 / clippy 0 / fmt 0 / validate 37 + 结构自检一致（采信，见 Evidence）；tasks T1-T8 状态与实际一致、RTM Iter002 两行 Covered、Iteration/Cycle 文件 3×1 齐全、Review Result 000/001 accepted / 002 本 Review。

**Convergence**

N/A（首次 Review，无父 Cycle gap 可比较）

**Evidence**

- 独立读码：`src/wal/checkpoint.rs` 全文（238 行，含 3 单测）、`src/wal/recovery.rs:15-45/:395-506`、`src/wal/mod.rs:13`、`src/database.rs:60-139/:225-255`、`tests/isolation_level_test.rs:290-430`（T5 夹具 + 新 e2e 全文）、`tests/checkpoint_test.rs` 全文、`tests/recovery_test.rs:70-90`；delta spec `transaction-isolation-levels`（新增 Requirement 4 场景）与 design D8 逐点比对；调用面 grep 闭合（见 F3/独立代码检查）。
- 验证采信（公共规则 › 验证：Act 结论产生于终态树，本 Review 会话只读、覆盖面文件与 Changed Files 一致；计数自洽 1061（Iter 001 全量）+ 4（isolation_level e2e 1 + checkpoint.rs 位点单测 3）= 1065）：Act Response Verification Evidence 全表 PASS 采信——全量 `--no-fail-fast` 1065 passed / 0 failed / 2 ignored、clippy 0、fmt 0、validate 37 passed（36 specs + 1 change）。RED 两记录（行为 0 行 / 位点 API 编译演进）与修复前结构链逻辑一致，采信为 test-first 见证。
- Persisted Evidence Mode `none`：无 Evidence 目录要求，目录不存在不构成 finding。

**Follow-up Decision**

接受（accepted）。实现满足本 Iteration 全部既有 Acceptance（T7 S1-S4 逐场景见证 + T8 二次收口）；适配面 PLAN-OMISSION 为非阻塞机械偏差，无需当前 Cycle 修复。本 change 全部 Iteration（000/001/002）完成、全部任务 T1-T8 done——实现面就绪，进入收尾流程：specs 合并（5 delta 域：新增 `in-subquery-join-rejection` + 修改 4 域）与 change 归档由 `openspec-docs-maintainer` 按用户指令执行；ISS01 台账补记（MAX 毒化一并修复 + 本 Iteration 水位修复）、Runbook 候选由 `openspec-experience-recorder` 按用户指令落账。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

None（Iteration Map 仅 000/001/002，T1-T8 全部完成；本 Iteration accepted 后 change 全部 Iteration 完成）

- Review Result: accepted
