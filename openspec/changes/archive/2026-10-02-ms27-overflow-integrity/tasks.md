# 任务与逻辑迭代

需求于 2026-10-02 获批（Gate 1）；完整计划于 2026-10-02 获批（Gate 2，T02 方案「仅 rtsql check」同批批准）。复选框表示实施状态。2026-10-02 Iteration 000 Review `accepted` 后，用户指令将 catalog DEFAULT 段长字符串回绕修复并入 Iteration 001 重新规划（原话见 proposal 集中决策 3；design D11）——T12 为 replan 新增任务，其余 Iteration Map 不变。

## 1. 超页元组表示与存储原语（关联 MS27-T01）

- [x] 1.1 T1：长字符串增量 tag 0x08（serialize/deserialize_tuple_width/deserialize_value_refs/compute_tuple_size 四入口 + 65535/65536 切换点 + unknown-tag 测试校准）（R1）；依赖 None；以 tuple 单测 RED→GREEN 验收。（2026-10-02 Act：tuple 单测 29 passed）
- [x] 1.2 T2：溢出页格式与链原语模块 `src/storage/page_format/overflow.rs`（locator 编解码 0xFF、页头 0x04、chunk 分解、链装配读、链收集/释放；页级与 pool 级两层 API）（R1/R4）；依赖 T1；以 overflow 模块单测 RED→GREEN 验收。（2026-10-02 Act：10 新单测含失败注入清理见证）
- [x] 1.3 T3：写路径溢出分支 `write_tuple_to_data_page`（>4074 直走溢出：分配链→写 chunk→末步落 locator；失败清理已分配页；≤4074 逐字节不变）（R1/R4/R5）；依赖 T2；以 overflow_page_test 写入/边界/失败清理用例 RED→GREEN 验收。（2026-10-02 Act：写侧 3 用例 + data_page 单测零回归）
- [x] 1.4 T4：读路径两相装配（`read_tuple_from_data_page` Overflow 装配；DataScan `YieldOverflow` 出页闭包后装配；`find_visible_version` 可见溢出版本出锁装配；内联路径零拷贝保持）（R1/R5）；依赖 T3；以 overflow_page_test 读回/UPDATE 增长缩回/多行混排/重开用例 GREEN 验收。（2026-10-02 Act：overflow_page_test 9 passed；lib 383 passed）

## 2. 生命周期、恢复与全通道（关联 MS27-T01）

- [x] 2.1 T5：恢复面溢出感知（`redo_tuple_at_row_id` 溢出分支链写+末步 locator+幂等；`build_pk_version_maps` keyless/`rebuild_pk_indexes`·`extract_index_keys`/`derive_old_row_id` 装配消费）（R3）；依赖 T4；以崩溃恢复两态用例（seed_pending_wal 同型夹具）RED→GREEN 与既有恢复套件零回归验收。（2026-10-02 Act：恢复 4 用例 RED〔`No enough space in page`/`unknown tag byte: 0xff`〕→ 三段式 redo + 四消费点 TupleRef 两相装配 GREEN；CREATE INDEX 回填遗漏同轮修复；wal_recovery_large 7 / recovery_replay_convergence 3 零回归）
- [x] 2.2 T6：生命周期释放（`delete_tuple_from_data_page` 先删 slot 后释放链；`gc_table`；`drop_table`/`truncate_table` collect 溢出链页随数据页释放）（R4）；依赖 T3；以 GC/drop/TRUNCATE 释放用例 GREEN 验收。（2026-10-02 Act：释放 3 用例 RED〔链页未回收〕→ delete 单点 + collect_overflow_chain_pages 接线 GREEN；truncate 见证按清零落盘+新根源自链页回收〔树重建页流转使集合断言不稳健，Act Response 偏差 1〕；truncate_test 7 / gc_test 3 零回归）
- [x] 2.3 T7：通道与形态 e2e + 规模基线（CLI INSERT VALUES/子集+DEFAULT/UPSERT/REPLACE/UPDATE、CTAS、INSERT SELECT、dump→restore、import；加密、`:memory:`、只读；4073/4074/4075 与 chunk 边界；~1MB 基线含恢复；含超页表 CREATE INDEX 回填见证——000 Review finding）（R1/R2/R3/R5）；依赖 T5/T6；以 overflow_page_test 全通道矩阵与受影响定向回归 0 failed 验收。（2026-10-02 Act：10 新用例〔矩阵/tag 切换点/CTAS/跨库/加密/:memory:/只读/1MB 基线/dump-restore/import〕GREEN；INSERT SELECT 跨库走子进程——ATTACH 为 CLI 会话专属〔偏差 2〕；定向回归 cross_db_dml 17 / ctas 17 / insert_select 10 / import_input_surface 12 零回归）
- [x] 2.4 T8：文档同步（README 双语长值支持与边界说明〔含 DEFAULT 字面量长度受 catalog 列行页内容量约束、超限建表既有页满拒绝〕；rtsql-docs/SKILL.md）（R1/R5）；依赖 T7；以文档示例实跑 + 逐条内容 Review 验收。（2026-10-02 Act：双语 README「长值存储」bullet + DEFAULT 边界句 + SKILL.md Behavioral note；人工实跑——5000 字符 DEFAULT 建表 `Page full` exit 3 零副作用、小值 DEFAULT 建表并经省略列应用正常）
- [x] 2.5 T12：catalog DEFAULT 段长字符串语义（`serialize_catalog_column_row`/`deserialize_catalog_column_row` >65535B String 走 `[0x08][u32 len]`、≤65535 字节布局不变、0x08 截断/坏 UTF-8 诚实拒绝；表示一致性硬化——生产不可达性分析与边界文档化见 design D11）（R1）；依赖 T1（已完成）；以 catalog 单测 65535/65536 切换点往返 RED→GREEN 验收。（2026-10-02 用户指令 replan 并入）（2026-10-02 Act：4 新单测 RED〔65536 回绕〕→ 两臂实现 GREEN；65535 既有布局锚点零回归，lib catalog 35 passed）

## 3. rtsql check 完整性校验（关联 MS27-T02）

- [x] 3.1 T9：lib 校验入口（页 census、catalog/数据链/溢出链/索引树结构遍历、bounds-checked slot 迭代、问题清单与汇总结构；只读）（R6/R7）；依赖 None（复用 000/001 的 0x04 页结构）；以 lib 单测（干净库 + 逐类损坏注入检出）RED→GREEN 验收。（2026-10-03 Act：6 新单测 RED〔stub 空报告〕→ 实现 GREEN；树遍历自实现 walk（DAG re-visit 静默跳过对齐 tolerant 先例）+ Gate 3 补内部节点最左孩子遍历；lib integrity 6 passed）
- [x] 3.2 T10：CLI `check` 子命令（Command 枚举 + `lifecycle_db_target` Check 臂 + `resolve_existing_db` + readonly 开库编排 + 退出码 0/1/4/5 + `--format` 忽略 + tests/check_command_test.rs 矩阵）（R6/R7）；依赖 T9；以 check_command_test 全矩阵 GREEN 验收。（2026-10-03 Act：8 用例 RED〔`check` 被主命令当作库名解析〕→ 实现 GREEN；恢复后库 terminal>持久化 tail 为已知良性形态按 info 报告〔recovery 重放只推进内存 tail，无条件 error 违反 R6 干净库场景〕、截断类 terminal≤tail 保持 error；check_command_test 8 passed）
- [x] 3.3 T11：文档同步（README 双语命令表/退出码段；rtsql-docs/SKILL.md Lifecycle 表 + Behavioral notes）（R7）；依赖 T10；以文档示例实跑 + 逐条内容 Review + fmt/clippy + M19 全量分批 0 failed 验收。（2026-10-03 Act：双语 README 命令表 +1 行、SKILL.md Lifecycle 表 +1 行与 Behavioral note +1 条〔含恢复后 tail 良性差异 info 语义〕；人工实跑干净库单行确认 exit 0 与注入损坏库逐行报告 exit 1；fmt --check clean、clippy --tests 无新告警、M19 全量 12 批 0 failed〔1797 tests / 0 failed / 2 ignored〕）

## Iteration Plan

### Iteration 000：溢出表示与存储原语（写读闭环）

- Tasks：T1、T2、T3、T4。
- Depends on：None。
- 阶段结果：>4074B slot_data 经 locator+溢出页链正确写入与读回（含 UPDATE 增长/缩回、多行混排、重开），≤4074B 既有路径逐字节不变。
- Stable baseline：超页行 INSERT/SELECT/UPDATE 端到端可用且重开一致；内联行为与错误面零回归。
- Verification boundary：新 tests/overflow_page_test.rs（写入/边界/失败清理/读回/增长缩回/混排/重开）+ tuple/overflow/slotted_page 单测 + storage/executor 定向回归。
- Diagnostic boundary：`src/storage/page_format/tuple.rs`、新 `overflow.rs`、`src/storage/data_page.rs`、`src/executor/data_scan.rs`、`src/storage/buffer_pool.rs::find_visible_version`。
- Non-goals：恢复重放、GC/drop/TRUNCATE 释放、全通道 e2e、加密/内存/只读形态、check 命令（均后继 Iteration）。
- 平衡审计：T1 表示、T2 原语、T3 写、T4 读是同一条「超页行落库并读回」链路的四层，缺少任何一层都不可独立验收；合为一轮形成可运行可验证的存储能力。恢复/生命周期是独立故障域（WAL 重放与释放语义），拆到 001。

### Iteration 001：生命周期、恢复与全通道

- Tasks：T5、T6、T7、T8、T12（T12 为 2026-10-02 用户指令 replan 并入，design D11）。
- Depends on：Iteration 000 的 accepted Review。
- 阶段结果：超页行崩溃恢复两态一致；GC/drop/TRUNCATE/写入失败无孤儿链；全部写通道与运行形态（加密/:memory:/只读）+ 1MB 基线 e2e 可用；catalog DEFAULT 段与 tuple 表示语义一致；使用文档同步。
- Stable baseline：溢出能力达到 R1–R5 全部可验证状态，可进入全量收尾。
- Verification boundary：overflow_page_test 恢复/释放/通道/基线矩阵 + catalog DEFAULT 段单测 + gc/recovery/checkpoint 定向回归 + fmt/clippy + M19 全量分批。
- Diagnostic boundary：`src/wal/recovery.rs`、`src/storage/data_page.rs::delete_tuple_from_data_page`、`src/storage/data/table_manager.rs`（gc/drop/truncate）、`src/executor/create_index.rs` 回填段、`src/storage/catalog.rs` DEFAULT 段、CLI 通道、README/rtsql-docs。
- Non-goals：check 命令、性能优化、VACUUM/GC 生产激活、BLOB/DECIMAL、catalog 行溢出（DEFAULT 长度边界保持页内容量约束，D11）。
- 平衡审计：T5 恢复与 T6 释放同属「溢出资源的持久化与生命周期正确性」，T7 全通道与基线是对前两者的端到端验收，T8 文档随能力定稿——合并为一轮避免「只验证不收口」的过薄迭代。T12 为单点表示硬化（两个纯函数 + 单测），工作量小、依赖已完成（T1）、诊断边界独立，作为 replan 增量并入不破坏轮次内聚；不单独成轮（会形成「一个函数两个分支」的过薄 Iteration，违反平衡审计）。
- 后续 Iteration：002。

### Iteration 002：rtsql check 完整性校验

- Tasks：T9、T10、T11。
- Depends on：None 硬前置（页 census 消费 000/001 的 0x04 分类；按序执行在 001 后）。
- 阶段结果：`rtsql check <db>` 只读结构扫描可用——干净库 exit 0，损坏检出并报告页位置与语义，不修改库文件；命令面与文档同步。
- Stable baseline：R6/R7 全部可验证；change 实施面完成。
- Verification boundary：新 tests/check_command_test.rs（干净/逐类损坏注入/pending WAL/加密/守卫/退出码）+ CLI 定向回归 + fmt/clippy + M19 全量分批收尾。
- Diagnostic boundary：lib 校验入口（新模块）、`src/cli/mod.rs` Command/lifecycle_db_target、`src/cli/lifecycle.rs`、README/rtsql-docs。
- Non-goals：自动修复、页校验和、生产读路径 panic 面修复（Issue 候选另报）、跨附加库编排。
- 平衡审计：lib 遍历（T9）、CLI 面（T10）、文档（T11）构成一个独立可验收的命令能力，与 T01 存储演进故障域不交叉，满足用户「两项独立可验收、诊断边界不交叉」的合并约束。
- 当前轮：`iterations/002-rtsql-check/000-initial.md`（2026-10-03 Iteration 001 Review accepted 后展开；同日 Plan Review `accepted`——三项 Minor finding 记录不返工、范围外 Issue 候选一项〔恢复重放不持久化 data_page_tail〕待用户裁定；无后继 Cycle，Iteration 002 完成，change 实施面收口）。

## Requirements Traceability Matrix

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| R1 超页存取可达 | T1/T2/T3/T4/T7/T12 | overflow_page_test 写读/边界/通道/基线用例 + tuple/overflow 单测 + catalog DEFAULT 段单测 + CREATE INDEX 回填见证 | Covered |
| R2 运行形态与加密联动 | T3/T7 | overflow_page_test 加密往返/`:memory:`/只读用例 | Covered |
| R3 恢复两态一致 | T5/T7 | overflow_page_test 崩溃恢复两态 + 既有恢复套件零回归 | Covered |
| R4 溢出页物理生命周期 | T2/T3/T6/T7 | overflow_page_test 释放矩阵（GC/drop/TRUNCATE/写入失败清理） | Covered |
| R5 兼容与零回归 | T3/T4/T7 | 4073/4074 边界 + 既有单页行为/错误面定向回归 + M19 全量 | Covered |
| R6 损坏检出与位置报告 | T9/T10 | check_command_test 干净通过/逐类损坏注入检出矩阵 | Covered |
| R7 校验面边界协调 | T9/T10/T11 | check_command_test 只读不改文件/--key/只读/:memory:/锁/pending WAL 用例 + WAL CRC 零变化回归 | Covered |

R1–R7 全部 Covered，无 Simplified/Missing。

## Phase / Gate 与验证状态

- CLARIFY completed；Gate 1 PASS（2026-10-02 用户批准，两项集中决策见 proposal）。
- INVESTIGATE completed；Current-State Evidence 见 design（两位只读代理符号级映射 + Plan 精读 tuple.rs/slotted_page.rs/data_page.rs/data_scan.rs/buffer_pool.rs 确认）。
- DESIGN / TASKS / RTM completed；实质未知项 None；T02 方案按里程碑约定随调查定稿为 `rtsql check` 结构扫描（design D7），随完整计划一并提交 Gate 2 批准。
- PLAN REVIEW completed（计划交付自检）；Gate 2 PASS——2026-10-02 用户批准完整计划（选项「批准完整计划」），T02 方案「仅 rtsql check」同批批准（选项「仅 rtsql check（推荐）」）；当前 Cycle Status=ready，无 BLOCKED/WAIVED。
- Persisted Evidence：none；验证结果可低成本重跑，Act Response 足以承载决定性输出。
- SKIPPED：产品代码修改（Act 职责）、性能 bench（无性能 requirement）、Plan 阶段全量测试（定向基线足够）、后续 Iteration 目录创建（需当前 Iteration accepted）。

最终回归按 M19：`--lib`/`--bins` 与全部集成测试目标按名字典序分批，每批约 8–12 个目标，串行独立 `cargo test <明确目标> -- --test-threads=1`；每批 0 failed 方可宣称全量。不使用无目标过滤的全量调用，不创建辅助判定脚本、身份字段或证据协议。
