# tasks: MS10-T02 跨进程文件锁 + 优雅停机

> 状态：Replan（2026-09-06，Gate 2 前审计：WalReader 嗅探 P0 修复纳入范围，Iteration 拆分）已完成，Gate 2 待用户批准后交 openspec-act。
> **Review 裁定（2026-09-06，Cycle 000-initial Plan Review = `replan-required`）**：T0 完成；T1 被 sigkill e2e 暴露的引擎级恢复正确性缺陷阻塞（Blocker Handoff，Plan 独立复现实证）。新增全局任务 **T0b**（WAL 重放位置寻址幂等修复，design D7，delta spec `wal-recovery-replay-integrity`）归 Iteration 000，后继 **Replan Cycle 001** 承载 T0b + T1 收尾。Iteration 001 不变。
> **Review 裁定 2（2026-09-07，Cycle 001-replan Plan Review = `rework-required`）**：T0b 核心成立、sigkill e2e 转绿（Plan 独立复跑：witness 5/5、cli_test 14/3 白名单/2 ignored、clippy/fmt/validate PASS）；但深度验证揭示三个**既有 B-Tree 多页规模缺口**进入验收路径（最小键 search 盲区 → 重复 PK 被接受 / delete 重平衡 Page-full 泄漏 → 无中位点 checkpoint 混合 WAL 打开失败 / update 内部节点未实现），且见证 ② 断言放宽违反契约（预期值 9950 系推导错误，真值 9951 精确可达）。**Rework Cycle 002** 以 4 个 repair item（R-T0b-R1..R4）完成既有 Acceptance；B-Tree 修复面扩大需用户 Gate 2 批准。Map 不变。
> **Review 裁定 3（2026-09-07，Cycle 002-rework Plan Review = `rework-required`）**：R-T0b-R2/R3/R4 成立（独立复跑 4/4 + 既有 B-Tree 套件零回归；G1 根因 = `Key::deserialize` 尾部零扫描推断 len，修复为 32 字节定长比较，覆盖整个尾部零键类）；Act 发现的 stale catalog root 缺陷经独立复现确认。Plan Review 3 另发现 **G5：DataScan 对被更新行版本双计（运行期即复现：100 行 + 10 UPDATE → COUNT=110）**——R2-S1 计数精确被 G4（stale root）+ G5 双重阻塞。**Rework Cycle 003** 以 R-T0b-R5（catalog root 同步）/ R-T0b-R6（扫描替代集合去重）/ R-T0b-R1 完成化 / R-Gate 收口；executor 扫描语义修复面扩大需用户 Gate 2 批准。Map 不变。
> **Review 裁定 4（2026-09-08，Cycle 003-rework Plan Review = `rework-required`）**：R6 完成（独立复跑 GREEN + 回归面全绿）；R5 实现完成、见证 GREEN，但 G4 完整闭合被**撕裂树**阻塞——Plan 独立复现（裸读磁盘树：`catalog_root=155` 正确、`scan_all` 184/10000、`collect_all_pages` 命中 `InvalidPageType` 洞页）：中位点 checkpoint 后页驱逐按 LRU 而非树拓扑刷盘，磁盘树含洞/孤儿，恢复重放对其任何消费都不健全；任何 catalog root 策略无法闭合。**裁定 design D10（方向 a′）：`redo_count > 0` 时恢复路径零消费磁盘索引树**——redo 去索引化（磁盘版本多映射派生 old_row_id）+ 重放后从最终数据页重建 PK 索引（判重显式报错保 K05）+ 洞容忍释放旧树。**Rework Cycle 004** 以 R-T0b-R7/R8 + R-T0b-R1 收口 + R-Gate 执行；修复面扩大（recovery 重放臂 + index_manager 洞容忍收集 + table_manager 换入 API）需用户 Gate 2 批准。Map 不变。新发现 `mark_tx_aborted` 为 no-op（`mark_uncommitted_aborted` 空转）记 improvement 候选，不在本 change 处理。
> **Review 裁定 5（2026-09-08，Cycle 004-rework Plan Review = `accepted`）**：R7/R8 落地（Plan 独立审读 + 独立复跑全过）——`mixed_dml_recovery_semantics` 转绿（wal_recovery_large **7/7 全绿 0 ignored**）、全量白名单口径确认（cli_test 14/3/2，失败恰为 T4-RED 三用例）、clippy/fmt/validate 全过、见证文件零修改（mtime 核对）。偏差 1（redo header 提交编码 `None`→`Some(tx)`，R6 压制与 R8 重建谓词的必要条件）判定为契约内消解（Plan Context 已将编码核实留给 Act；只影响恢复侧已提交重放行）。**Iteration 000 完成**；展开 `iterations/001-lock-shutdown/000-initial.md`（status `ready`，Gate 2 依据 = change 级批准 2026-09-06 + 基线独立复核，契约以本文件 T2-T5 为权威）。改进候选三件随收尾处理：撕裂树运行期根修/驱逐改造、`mark_tx_aborted` 空转补全、update→delete 旧值重现。
> **Review 裁定 6（2026-09-08，Cycle 001-lock-shutdown/000-initial Plan Review = `accepted`）**：T2/T3 存量收编（hunk 摘除 RED 复现在案）、T4 生产接线（两阶段 select + `Signaled` 枚举 + D5-⑤ 结构测试 + 打开阶段 WAL>2KB 断言）落地——Plan 独立审读 + 独立复跑全过：全量门 **636/0/2**（白名单清零）、cli_test 17/0/2、结构测试绿、clippy exit 0（仅 `.cargo/config` 环境提示）/ fmt 0 / validate PASS、Preserve 面零 diff（`Cargo.toml`/`file_storage.rs`/`error.rs` 相对 `590fdc6` 未动）。偏差 1（D5 标定落点偏移：D10 改变恢复耗时量级，40k→8.86s 取代 20k 档，e2e ①③ 信号确定落在打开阶段）判定为 ACT-DEVIATION（根因 BASELINE-CHANGED，D5 自身「倍增 N」升级路径已穷尽），spec SHALL 阶段无关、THEN 子句覆盖无缺口（close 执行由结构测试确定性承担）。**Iteration 001 完成，change 实施面完成**——收尾待用户指令：commit、docs-maintainer 全局同步与归档、improvement 登记（撕裂树运行期根修+量化数据 40k→8.86s/160k→40.7s、`mark_tx_aborted` 空转、update→delete 旧值重现）、Recorder 落撕裂树 Incident。
> 工作区并入（2026-09-06，Plan 修订）：replan 前被中断的未提交实现已盘点实测并并入计划——T2/T3 存量实现（RED 以「临时摘除 hunk」复现）、T4 测试脚手架（生产接线缺位）、4 个既有测试夹具锁适配（T2 必要后果，断言语义零修改）、T1 门对 3 个 Iteration-001 信号 RED 见证设白名单。详见 design「工作区存量」节 / D5 / D6。
> 审计依据：本会话实验（严格帧遍历 2046 帧完好/嗅探误判 79 帧；执行阶段秒级语句不可构造；多行 INSERT Page full 观察）+ 本轮受影响 6 测试目标实测（2026-09-06）+ Plan Review 独立复现（10k 行干净重开 13190 ≠ 10000、checkpoint 后仍 13190）。

## Iteration Plan

### Iteration 000: WAL 恢复逐帧无歧义与重放正确性（引擎正确性前提） — completed（2026-09-08，Cycle 004-rework Review accepted；D7+D8+D9+D10 修订后收口）

- Tasks: T0, T0b, T1
- Depends on: None
- Stable baseline: 任意大小完好 WAL 恢复成功（D0，已完成）；**驱逐规模（>BufferPool 容量 100 页）+ 未 checkpoint WAL 崩溃恢复数据精确**（行数 = 已提交数、无重复丢失、PK 索引一致、Update/Delete 语义正确、恢复重跑幂等，D7/T0b）；混合格式流兼容；损坏帧仍显式报错（K05）；614 基线零回归
- Verification boundary: `cargo test --all` 除 T4-RED 白名单（T1 枚举的 3 个 Iteration-001 信号用例）外全绿（新增 `tests/wal_recovery_large_test.rs` 承载 D0 + D7 见证；既有测试断言语义零修改，4 文件夹具锁适配见 design D6）；clippy/fmt/validate 全 0/PASS
- Diagnostic boundary: `src/wal/reader.rs`、`src/wal/recovery.rs`、`src/storage/data_page.rs`、`tests/wal_recovery_large_test.rs`
- Non-goals: `WALBuffer::do_flush` 并发互斥（improvement 候选）；Checkpoint 记录改新格式（T03 范畴）；magic 文件头（T03）；DDL 无 WAL 记录的持久化模型（既有，夹具以 checkpoint 处理）

**平衡审计（D7 修订后复审）**：T0（reader 解析）+ T0b（重放正确性）+ T1（验证门）同属「kill 后 WAL 恢复 e2e 可达」这一单一引擎正确性成果；故障域连续（WAL 解析 → 重放 → 数据/索引一致），验证命令与诊断域重叠。T0b 使 sigkill e2e 的数据完整性断言可达，是 Iteration 001 的硬前提。仍不过重：重放修复面集中于 `redo_record` + 1 个写入 helper（~150 行级），见证测试复用同一夹具形态（50 行/事务建库 + drop 不 close + 重开断言）。

### Iteration 001: 并发互斥与可中断的 CLI — completed（2026-09-08，Cycle 000-initial Review accepted）

- Tasks: T2, T3, T4, T5
- Depends on: Iteration 000（kill-recovery e2e 与大 WAL 打开阶段 fixture 依赖大 WAL 恢复正确；基线 614 tests）
- Working-tree note（2026-09-06 Plan 修订）: T2/T3 生产代码与测试、T4 e2e 脚手架已存在于工作区（未提交、未验证，见 design「工作区存量」节）；Act 按契约复核收编（含 RED 复现），不重写
- Stable baseline: 打开被占用库 → `DatabaseLocked` / CLI exit 4；SIGINT/SIGTERM 优雅停机（close checkpoint → 130/143）；同进程双开被拒；正常路径零回归
- Verification boundary: `cargo test --all` 全绿（含 Iteration 000 白名单中的 3 个信号用例转绿；`tests/database_file_lock_test.rs` 已就位 + `tests/cli_test.rs` 增补；既有测试断言语义零修改，4 文件夹具锁适配见 design D6）；clippy/fmt/validate 全 0/PASS
- Diagnostic boundary: `src/storage/{file_storage,error}.rs`、`src/cli/mod.rs`、`Cargo.toml`、`tests/{database_file_lock_test,cli_test}.rs`
- Non-goals: T03 格式头、T04 多语句分片、T05 生命周期子命令、锁等待、Windows/非 Linux、server 路径、REPL、lib API 签名

**平衡审计**：锁（引擎层）、退出码映射（CLI）、信号停机（CLI）、回归门同属"进程生命周期安全"单一验收域；故障域重叠（文件 fd / 信号 / 进程退出码），锁与信号共享同一集成夹具与 open 路径。拆分产生不可独立验收中间态。不拆分。

## Tasks

### T0: WAL 恢复逐帧无歧义解析（Replan 新增）

- **Status (2026-09-06 末, Cycle 000-initial Act Response `blocked` 但 T0 自身 GREEN)**: ✅ 完成（独立 Plan 审查通过）。`src/wal/reader.rs::read_next_with_lsn` 判别与回退重排落地；见证 2 用例（`small_wal_recovers_unchanged` / `large_wal_recovers_after_unclean_shutdown`）GREEN；混合流（旧格式 Checkpoint 帧 + 新格式帧）由 7.7MB 实测覆盖。父 Cycle 因 T0 揭示的引擎级恢复正确性缺陷（sigkill e2e）blocked，replan 后由 T0b 修复面承接（不属 T0 范围）。
- Requirement/Scenario: `wal-recovery-frame-parsing` 全部 3 场景
- Depends on: None
- Targets: `src/wal/reader.rs::WalReader::read_next_with_lsn`
- 当前行为: 格式嗅探（`reader.rs:63-65`「byte[8] 合法且 byte[0] 非法 → 新格式」）在文件偏移低字节 ∈ {0x01..0x09} 时把新格式帧误判为旧格式，恢复 derail → `Incomplete WAL record` → `Database::open` 失败（实验实证：2046 帧完好 WAL 被误判 79 帧，首例 offset 1033）
- 目标行为: 按 design D0——歧义偏移（byte[0] 为合法 type 值 0x01-0x09）先按新格式解析并以 CRC 验证为接受判据；失败（CRC/结构/EOF）seek 回帧首按旧格式解析；两路皆败显式报错。非歧义偏移维持现有分支。混合流（新格式 + 旧格式 Checkpoint 记录）兼容
- Required changes: `read_next_with_lsn` 判别与回退逻辑重排（约 +30 行）；`deserialize_with_lsn`/`deserialize` 语义不动
- Preserve: 新格式帧 CRC 验证语义；旧格式流兼容（既有 `wal_record_test`/`wal_handle_test` 守护）；损坏帧显式报错（K05）；`seek_to(lsn)`/位点语义不变
- Forbidden: 不改 writer/checkpoint/BufferPool；不引入格式版本头；不做 do_flush 并发修复
- Test witness（RED 先行）: 新建 `tests/wal_recovery_large_test.rs`——lib 侧建库 + 单条多行 `INSERT INTO t VALUES (0),(1),...,(499)`（约 500 条 WAL 记录，>1KB）→ `wal_buffer.shutdown().await` + drop 不 close → `Database::open` 重开。RED：当前返回 `Err(WalError ...)`（Incomplete WAL record）；GREEN：恢复成功 + `SELECT COUNT(*)` == 500。补第二条用例：小 WAL（<19 条，如 5 行）重开恒成功（防修复破坏既有路径）
- GREEN condition: 两用例绿 + `cargo test --all` 零回归（既有 checkpoint/recovery 套件覆盖混合流与损坏帧语义）
- Verification: `cargo test --test wal_recovery_large_test` 输出+退出码记 Act Response
- Stop when: 修复后发现旧格式回退路径与新格式 CRC 判定存在无法消解的双匹配（语义需重开）；或大 WAL 恢复暴露 reader 之外的位点语义错误

### T0b: WAL 重放位置寻址幂等修复（Review 裁定新增）

- **Status (2026-09-07 13:35, Cycle 001-replan Act Response `reported`)**: ✅ 完成。`src/wal/recovery.rs::redo_record` 三臂位置寻址重写 + `redo_tuple_at_row_id` helper + `RedoContext` 页链/tail 追踪 + `extract_pk_key` 索引键提取。见证 3 用例（`eviction_scale_recovery_row_integrity` / `mixed_dml_recovery_semantics` / `recovery_rerun_is_idempotent`）GREEN；既有 2 用例（D0）零修改保持绿。Minor finding: `BTree::update` 内部节点递归缺口（5k+ Update 路径）归 M-S08 后续 milestone，不在本 change 范围。
- Requirement/Scenario: `wal-recovery-replay-integrity` 全部 4 场景（2 Requirement）
- Depends on: T0
- Targets: `src/wal/recovery.rs::redo_record`（+ 重放写入 helper，落点 `src/storage/data_page.rs` 或 recovery 私有，非实质）
- 当前行为: redo 追加式（`row_id: _` 忽略）——驱逐规模 + 未 checkpoint WAL 重开：行数虚增（Plan 复现 10k→13190 ≠ 10000、160k→+3190）、头部重复、原链尾部孤儿化、checkpoint 后持久化；重放不重建 PK 索引、不重放 Delete 墓碑、Update 重放丢版本链（design D7 根因 ①-⑥）
- 目标行为: design D7——Insert/Update 按 `row_id` 目标写入（slot 已存在跳过、`add_slot` 稠密落位校验、未初始化页 init）；Update header 重建 `next_version`（old_row_id 由 old_tuple 提取 PK 经索引推导）；Delete 墓碑 + 索引双步重放；Insert 重建 PK 索引（search 判重：同位跳过/缺失插入/他位显式报错）；页间切换重建 next 与 tail（首条记录不置 next）；重放不 `allocate_page`；镜像 M21 可见性维护
- Required changes: `redo_record` 重写（Insert/Update/Delete 三臂）+ 1 个目标写入 helper（约 120-180 行）；`deserialize_tuple`/`Value::to_key`/`index_manager.search` 均为既有 API 复用
- Preserve: WAL 记录格式、writer/record/buffer/checkpoint 语义、site/位点与事务分类语义、`mark_uncommitted_aborted`、页格式、catalog 结构、DDL 持久化模型；既有 `wal_recovery_large_test` 2 用例（D0 见证）语义零修改
- Forbidden: writer/record/buffer/checkpoint/TM/页格式修改；`allocate_page` 于重放路径；格式版本头；do_flush 并发修复
- Test witness（RED 先行，扩展现有 `tests/wal_recovery_large_test.rs`，3 新用例）: ① 驱逐规模行完整性——1 万行（50 行/显式事务 ×200，create_table 后**不 checkpoint**、drop 不 close）重开 → `COUNT(*)` 精确 10000（RED：13190，Plan 已实测）+ 索引判重探针（`INSERT` 已存在 PK → DuplicateKey 错误；新 PK → 成功）；② Update/Delete 混合——建库 checkpoint 中位点（5k 行处）+ 继续 5k 行 + UPDATE 100 行 + DELETE 50 行 → drop 不 close 重开 → 更新行新值、被删行 0 行、总数精确（终态 10050）；③ 恢复重跑幂等——①恢复后无写入再 drop → 重开 → COUNT 仍 10000。三用例 RED 先行（①确定性 RED 已由 Plan 复现实证；②③ Act 记录实际 RED 形态），GREEN = 全绿 + 既有 2 用例绿
- GREEN condition: 3 新用例绿 + `cargo test --all` 零回归（白名单口径同 T1）
- Verification: `cargo test --test wal_recovery_large_test` 输出+退出码记 Act Response
- Stop when: 目标落位出现无法以「slot 已存在跳过 + 稠密落位校验」消解的页状态（如 slot 稀疏/乱序——页格式假设失效需重开设计）；或索引推导 old_row_id 在合法序列上不可判定（Update 前行未入索引且其 Insert 不在重放窗口）→ 返回 Plan

### T1: Iteration 000 验证门

- **Status (2026-09-07 13:35, Cycle 001-replan Act Response `reported`)**: ✅ 完成。`cargo test` 11 个 binary 总计 240 + 14（cli_test，T4-RED 白名单 3 失败按契约排除）通过；`test_sigkill_leaves_recoverable_db` 在 T0b 修复后转绿（cli_test 14 passed 含此条）；`cargo clippy --all-targets` 0 warning；`cargo fmt --check` 0 diff；`openspec validate 2026-09-06-ms10-t02-file-lock-graceful-shutdown` valid。
- Requirement/Scenario: 全部（回归门）
- Depends on: T0, T0b（Replan Cycle 001 内 T0b 先行，T1 收尾）
- Targets: 无新代码；`cargo test --all`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate 2026-09-06-ms10-t02-file-lock-graceful-shutdown`
- 当前行为: Cycle 000（Act Response `blocked`）实测——T0 后白名单口径除 `test_sigkill_leaves_recoverable_db` 外全绿（3 信号用例呈预期真 T4 RED `code==None`）；clippy/fmt/validate PASS；sigkill 项命中 Gate 6 阻塞（恢复正确性缺陷，归因 T0b，见 Cycle 000 Blocker Handoff）
- 目标行为: **T4-RED 白名单**（`test_sigint_during_run_graceful_130`、`test_sigint_during_open_130`、`test_sigterm_during_run_143`——Iteration 001 T4 的 RED 见证，无 T4 生产代码必失败）之外全部通过，0 unexpected failed；`test_sigkill_leaves_recoverable_db` 在 T0b 后**必须转绿**（其失败归因 T0b，不属白名单）；测试总数 = 614 + 工作区已就位新增 + wal_recovery_large_test 用例数（2 + T0b 新增 3）
- Test witness: 各命令决定性输出（≤20 行）与退出码记 Act Response
- GREEN condition: 四项达标（`cargo test --all` 按白名单口径 + clippy/fmt/validate 全 0/PASS）
- Stop when: 白名单之外的回归失败且无法归因于 T0/T0b → BASELINE-CHANGED 返回 Plan

### T2: 打开即独占锁（引擎层）

- **Status (2026-09-08, Cycle 000-initial Act Response `reported`)**: ✅ 完成。存量实现复核收编（生产代码零修改）；RED 复现——临时摘除 try_lock hunk（11 行）→ `database_file_lock_test` 1 passed / 3 failed（①③④ 以 `expected DatabaseLocked, got Ok` 失败，② 天然绿，如约）→ 恢复 hunk → 4 passed / 0 failed（exit 0）；恢复后与 HEAD 零 diff。
- Requirement/Scenario: `database-file-lock` R1 全部 4 场景 + R2 全部 2 场景
- Depends on: Iteration 000
- Targets: `src/storage/file_storage.rs::FileStorage::open`、`src/storage/error.rs::StorageError`（+`DatabaseLocked(String)`）
- 当前行为: open 无跨进程锁；双持有者静默双写
- 目标行为: open 打开 fd 后立即 `try_lock()`（design D1）：`WouldBlock` → `Err(StorageError::DatabaseLocked(path.display().to_string()))`；其他 IO 错误 → `Err(StorageError::Io(e))` 原样传播；成功 → 锁随 fd 生命周期保持
- Required changes: open 内锁分支（约 8-12 行）+ error.rs additive 变体（`#[error("database is locked: {0}")]`）
- Preserve: `Database::open` 签名与成功路径零变化；`AsyncStorage` trait、页读写/分配/释放语义零变化；锁在 `WalWriter::open` 与 `RecoveryManager::full_recover` 之前完成（`database.rs:30` 落点保证）
- Forbidden: 锁等待/重试/busy timeout；`.wal`/`.checkpoint` 独立锁；network 路径；产品新依赖
- Test witness（**工作区已就位，4 用例实测绿**；RED 复现见下）: `tests/database_file_lock_test.rs`——① 同进程两次 `Database::open(同一 path)`：第一次 ok，第二次 `matches!(err, StorageError::DatabaseLocked(_))`；② drop 第一次实例后重开 ok；③ 两次 `FileStorage::open(同一 path)`：第二次 `DatabaseLocked`；④ 错误消息含路径。**RED 复现（存量实现下）**：临时摘除 `file_storage.rs` 的 try_lock 分支（11 行 hunk）→ ①③④ 失败（第二次 open 成功）、② 天然绿 → 恢复 hunk → GREEN；复现输出与退出码记 Act Response
- GREEN condition: 四用例绿 + `cargo test --all` 零回归
- Verification: `cargo test --test database_file_lock_test` 记 Act Response
- Stop when: `try_lock` 暴露非 `WouldBlock` 的冲突类错误语义（D1 错误分类需重开）；或存量实现与契约 Preserve/Forbidden 冲突且无法以局部调整消解

### T3: CLI 锁冲突退出码 4

- **Status (2026-09-08, Cycle 000-initial Act Response `reported`)**: ✅ 完成。存量实现复核收编（生产代码零修改）；RED 复现——临时摘除 Locked 分支（5 行）→ `test_lock_conflict_exit_4` 失败（exit 1 + General 文案，如约）→ 恢复 → GREEN（exit 0）；`test_corrupt_file_open_fails_exit_1` 守护保持绿。
- Requirement/Scenario: `cli-noninteractive-shell` 修改 R1 的场景"锁冲突退出码 4"
- Depends on: T2
- Targets: `src/cli/mod.rs::execute_command`（`Database::open` Err 分支 `:95-104`）
- 当前行为: （基线）所有 open 错误 → `ExitStatus::General` → exit 1；工作区存量已完成 `DatabaseLocked → Locked` 映射（与 D3 一致，`test_lock_conflict_exit_4` 实测绿）
- 目标行为: `StorageError::DatabaseLocked(_)` → `ExitStatus::Locked(format!("database is locked: {}", db_path.display()))` → exit 4 + stderr（经既有 `emit_stderr`/映射表，`src/cli/mod.rs:40-52` 现成）；其余 open 错误维持 General(1)（design D3）
- Required changes: open Err 分支 match 扩展；无其他
- Preserve: 页对齐/权限/redo 失败仍 exit 1（既有 `test_corrupt_file_open_fails_exit_1` 守护）；`resolve_db_path` 不动
- Forbidden: Locked 以外退出码映射；锁重试；文案偏离 `database is locked` 前缀
- Test witness（**工作区已就位，实测绿**）: `tests/cli_test.rs::test_lock_conflict_exit_4`——测试进程 `std::fs::File::open(目标 db)` + `try_lock()` 持有 → `run_cli` spawn → `code==Some(4)`、`stderr` 以 `database is locked` 开头、`stdout.is_empty()`；drop 持有者 → 再 spawn → `code==Some(0)`。**RED 复现（存量实现下）**：临时摘除 `cli/mod.rs` 的 Locked 分支（5 行 hunk）→ exit 1 且文案不同 → 恢复 hunk → GREEN；复现输出记 Act Response
- GREEN condition: 新用例绿 + 既有 12 用例零修改绿
- Verification: `cargo test --test cli_test` 记 Act Response
- Stop when: Locked 分支无法与既有错误路径隔离（结构冲突）

### T4: 优雅停机（信号 → close → 130/143）

- **Status (2026-09-08, Cycle 000-initial Act Response `reported`)**: ✅ 完成。D4 落地——`execute_command` 重构为 `execute_command_inner`（两阶段 select + `for<'a>` FnOnce work 工厂 + 信号工厂注入）；`ExitStatus::Signaled(i32)`（exit 128+signum，无 stderr）；close Err 语义扩一条（Signaled 保持信号码 + stderr 提示）。D5-⑤ 结构测试 GREEN（Notify 握手使信号确定落在阶段 2：close 已执行 WAL<1KB + Signaled(2)）；D5-② 增补 WAL>2KB 断言。标定重测（D10 重建后）：40k→8.86s、160k→40.7s，`WAL_ROWS=40_000`（T≥500ms 达标；D=200ms 低于 T/4 → e2e ①③ 信号落在打开阶段，用例注释容忍，偏差 1 记 Act Response）。cli_test 17 passed / 0 failed / 2 ignored（白名单 3 用例转绿）。
- Requirement/Scenario: `cli-noninteractive-shell` 新增"优雅停机（信号接线 close）"全部 4 场景
- Depends on: T2（打开阶段取消依赖锁随 fd 释放；夹具共用）＋ Iteration 000（大 WAL 恢复正确）
- Targets: `src/cli/mod.rs::execute_command`（两阶段 select 重构 + 信号 future 注入点）、`ExitStatus`（+`Signaled(i32)`）、`Cargo.toml`（tokio features +`"signal"`）
- 当前行为: 无信号处理（**存量核对：`cli/mod.rs` 无任何信号接线，D4 生产代码缺位**）；SIGINT/SIGTERM 默认终止（子进程 `status.code()==None`），close 不触发；Cargo signal feature 与 cli_test 信号段已就位（实测 4 RED，当前失败模式为 T0 后果：子进程因 `Incomplete WAL record` 提前退出 1）
- 目标行为: design D4——`execute_command` 拆两阶段：阶段 1（open）select 信号臂 → `Signaled` 无 close 立即返回；阶段 2（run_sql）select 信号臂 → `db.close()` → `Signaled`；close Err 与 Signaled 并存 → 保持 Signaled + stderr 提示 close 失败（其余现状）。`Signaled(signum)` → exit `128+signum`（SIGINT=2→130、SIGTERM=15→143），`message()` 返回 None。tokio features +`"signal"`（工作区已接入）
- Required changes: execute_command 结构重构（db 提升为阶段间值；信号 future 注入点供 `#[cfg(test)]` 使用）、枚举 +1 变体 + 映射（Cargo.toml 已就位）
- Preserve: `run_sql`/渲染/resolve 不感知信号；无信号路径逐字节一致（既有 12 用例 + 全量守护）；close 正常路径语义（Success→General 映射）零变化；信号安装前窗口按默认终止（固有竞态，不处理）；close 期间信号不新增 select
- Forbidden: SIGHUP/其他信号；close 加 select/超时；130/143 以外信号退出码；WAL/恢复代码（D0 已在 000 完成）
- Test witness（e2e 4 用例**已就位、天然 RED 已实测**；结构测试与生产接线待建；按 design D5 并入版）:
  - ① 库级结构测试（`src/cli/mod.rs` `#[cfg(test)]`，确定性，**待建**）：信号 future 注入 + pending 慢工作负载 → phase-2 select 走信号臂 → 断言 `close()` 已执行（**WAL 截断 <1KB**——阶段判别观测物）+ 返回 `Signaled(2)`。RED：改造前无该编排结构（编译失败即为 RED）
  - ② e2e 打开阶段 SIGINT（**已就位** `test_sigint_during_open_130`，确定性锁锚点：轮询子进程持有主文件锁 ≤10s → 立即 `libc::kill(SIGINT)`）：断言 `code==Some(130)`；**Act 增补 1 行断言：WAL 长度仍大（>2KB，spec「无 close」的直接观测物）**；重开数据完整性由 ④ 覆盖，本用例不重复。标定：`#[ignore]` `calibration_recovery_time`（`RTSQL_CALIBRATION_ROWS` 可调）实测恢复耗时 T，要求 T≥500ms 且 200ms ∈ [T/4, T/2]，不足则倍增 N（T 记 Act Response）。RED：T0 落地后自然 RED（`status.code()==None`）
  - ③ e2e SIGTERM（**已就位** `test_sigterm_during_run_143`，夹具 `build_big_wal`：50 行/事务分块显式事务 × `WAL_ROWS=20_000`，drop 不 close；`SIGNAL_DELAY_MS=200`）→ `code==Some(143)`
  - ④ e2e kill -9（**已就位** `test_sigkill_leaves_recoverable_db`；守护性，T0 后、T4 前即应转绿）：SIGKILL → `code==None` → 重开 spawn → `code==Some(0)` + 数据完整（锁无残留 + 恢复成功；Iteration 000 已保证大 WAL 恢复正确）
  - ⑤ 无信号回归：既有 12 cli 用例零修改守护（已实测绿）
- GREEN condition: ①②③④ 绿 + 全量零回归（含 Iteration 000 白名单 3 用例转绿）
- Verification: `cargo test --test cli_test` + 标定数据（WAL 行数 N、恢复耗时 T、D）记 Act Response
- Stop when: 本机恢复耗时无法稳定 T≥500ms（时序构造需重设计——实质问题返回 Plan）；或 select 重构破坏既有 close 错误语义；或存量 e2e 脚手架与 D5 契约冲突无法以局部调整消解

**依赖注记**：信号发送用 dev-dependency `libc = "0.2"` + `libc::kill(pid, sig)`——libc 已由 tokio 传递引入（lockfile 零新包），产品 `[dependencies]` 零变化。

### T5: 全量回归与 Iteration 001 验证门

- **Status (2026-09-08, Cycle 000-initial Act Response `reported`)**: ✅ 完成。`cargo test --all` **636 passed / 0 failed / 2 ignored**（白名单 3 信号用例随 T4 转绿，无白名单终态口径；fmt 修正后以终态代码复跑确认）；`cargo clippy --all-targets -- -D warnings` exit 0；`cargo fmt --check` exit 0；`openspec validate 2026-09-06-ms10-t02-file-lock-graceful-shutdown` valid。
- Requirement/Scenario: 全部（回归门）
- Depends on: T2, T3, T4
- Targets: 无新代码；`cargo test --all`、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate 2026-09-06-ms10-t02-file-lock-graceful-shutdown`
- 当前行为: Iteration 000 完成后——除 T4-RED 白名单（3 个信号用例）外全绿
- 目标行为: 全部通过（含白名单 3 用例随 T4 转绿），测试总数 = 614 + 新增总数，0 failed
- Test witness: 各命令决定性输出（≤20 行）与退出码记 Act Response
- GREEN condition: 四项全绿
- Stop when: 回归失败且无法归因于 T2-T4 → BASELINE-CHANGED 返回 Plan
