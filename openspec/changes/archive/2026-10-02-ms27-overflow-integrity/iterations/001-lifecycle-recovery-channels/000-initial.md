# Iteration 001 / Cycle 000: 生命周期、恢复与全通道

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T5、T6、T7、T8、T12（T12 为 2026-10-02 用户指令 replan 并入）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R1–R5；Iteration 000 交付的溢出表示（tag 0x08）、overflow.rs 链原语、写路径溢出分支、三读消费点两相装配全部继续有效并被本 Iteration 消费
- Excluded scope: check 命令（T9–T11，Iteration 002）、性能优化（提交热路径 O(row) 装配为 MS22 候选观察，见 000 Act Response 未解决问题 2）、VACUUM/GC 生产激活、BLOB/DECIMAL、页校验和、catalog 行溢出（DEFAULT 长度边界保持页内容量约束，design D11）

**Objective**

超页行达到 R1–R5 全部可验证状态：崩溃恢复两态一致（WAL 重放溢出感知）；GC/drop/TRUNCATE/写入失败无孤儿链；全部写通道与运行形态（加密/`:memory:`/只读）+ ~1MB 基线 e2e 可用；CREATE INDEX 对含超页行的表可达（000 Review 发现的装配消费遗漏，见 Investigation Facts）；catalog DEFAULT 段与 tuple 值表示语义一致（T12）；使用文档同步。

**Background**

2026-10-02 Iteration 000 Plan Review `accepted` 后，Act 未启动、本 Cycle 无任何执行痕迹；用户指令将 000 Review 报告的 catalog DEFAULT 段长字符串回绕并入当前 Iteration 一并完成并重新规划（原话：「把这个issue直接放到当前iter的工作里面，重新规划，一起完成了吧」，登记于 proposal 集中决策 3）。本 Plan Context 于 Act 交接前按该指令修订：新增 T12 契约与对应调查事实，T5–T8 契约不变；修订不构成已交接 Cycle 的改写。

**Investigation Facts**

- Current Baseline: master @ 81c8e6f + Iteration 000 实施面（工作区未提交：5 修改 + 2 新增文件）。lib 383 passed / 0 failed；tests/overflow_page_test.rs 9 passed；projection/pushdown/gc 定向回归绿；fmt/clippy 干净（Iteration 000 Act Response 验证表，Plan Review 000 accepted 采信）。溢出行已可在运行期存在并经三个读消费点（`read_tuple_from_data_page`/DataScan/`find_visible_version`）正确装配。
- Current-State Evidence（000 Plan Review 独立审读 + 本 Cycle 补查，file:line 均已核实）：

  恢复面消费点（`src/wal/recovery.rs`，WAL 记录 tuple 字段恒为完整 tuple 字节——上游 executor 序列化，不经页 slot）：
  - `redo_tuple_at_row_id`（:136-221）：拼 slot_data = vh ++ tuple_bytes 后**整段在 `guard.modify_page` 闭包内**写（:157-187）；slot 已存在幂等跳过（:169-172）；`add_slot_with_logical_id` 按 WAL logical_id 强制落位（MS20-T02 T8 语义，:174-180）；闭包外步骤 3 链接 prev 页（:190-204）、步骤 4 data_page_tail 跟随、步骤 5 可见性镜像。溢出行 redo 必须把链写移出页闭包（await 约束），幂等跳过与强制落位语义保持。
  - `build_pk_version_maps`（:303-390）：预扫页闭包内取 `slot_data[VersionHeader::SIZE..]`（:346）→ `extract_pk_key` 失败入 keyless 桶按 tuple 原始字节（:352-356）。locator slot 会把 9B locator 字节当 tuple 身份——错误。装配须在闭包外。
  - `derive_old_row_id`（:391-448）：单次页读返回 slot tuple bytes（:424）后与 WAL `old_tuple` 逐字节比较（:441-445）。locator slot 必失配 `RedoFailed`。
  - `rebuild_pk_indexes`（:928 起）：页扫闭包内 `extract_index_keys(&slot_data[22..])`（:1063-1068）。locator → 垃圾键。
  - `mark_uncommitted_aborted`（:1295 起，:1340 消费）：header-only 收集 fixes 后走版本头写路径——溢出安全，零改动。
  - `extract_pk_key`（:222-247）/`extract_index_keys`（:249-301）：输入为完整 tuple bytes——装配后直接复用，签名不改。
  - 恢复测试夹具先例：`wal_buffer.shutdown()` = 崩溃模拟（不 checkpoint，DDL 不入 WAL——Iteration 000 偏差 3 的教训：崩溃恢复用例的建表须在 seed 前先 `close()` 落盘 catalog 或沿用库内已持久 schema）；`tests/checkpoint_command_test.rs:26` `seed_pending_wal`、`tests/wal_recovery_large_test.rs:28` unclean-shutdown 形态可参照。

  生命周期释放面：
  - `delete_tuple_from_data_page`（`src/storage/data_page.rs:211`）：现仅删 slot（SlottedPage 层），无溢出感知。
  - `gc_table`（`src/storage/data/table_manager.rs:125-154`）：header-only 遍历（`read_version_header`），删除委托 `delete_tuple_from_data_page`（:148）——单点接线后自动受益，本函数零改动。
  - `drop_table`（:1285）/`truncate_table`（:1412）→ `collect_data_pages`（:1358，私有）：沿数据页 `next_page_id` 链收集后逐页 `free_page`，不感知 slot 内 locator。溢出链页须在释放数据页前从 slot 收集（header-only 扫描 + locator 解码），best-effort 释放（对齐既有释放语义，design D5）。
  - 顺序约束（design D5）：`delete_tuple_from_data_page` 先删 slot 再释放链（先链后 slot 引入 dangling 窗口）。

  000 Review 发现的运行期装配消费遗漏：
  - `CREATE INDEX` 存量回填（`src/executor/create_index.rs:118-163`）：页闭包内对 `slot_data[VersionHeader::SIZE..]` 直接 `deserialize_row`（:145-149）。超页行 slot 的 tuple 段是 9B locator → 未知 tag 0xFF 点名报错，建索引失败。属 R1「溢出对 SQL 层透明」面；修复 = 与恢复面同型的两相装配（闭包内收集 locator 引用，出闭包装配后派生键）。

  catalog DEFAULT 段（T12，2026-10-02 replan 补查）：
  - 序列化 `serialize_catalog_column_row`（`src/storage/catalog.rs:1198-1260`，`pub(crate)` 纯函数）：String DEFAULT 臂（:1233-1237）恒 `TAG_STRING + s.len() as u16` → >65535B 字面量长度前缀回绕；其余 tag（Int/Null/Float/Bool/Date/Timestamp）无长度面。
  - 反序列化 `deserialize_catalog_column_row`（:1262-1438，`pub(crate)` 纯函数）：DEFAULT tag match（:1373-1426）String 臂 u16 长度（:1375-1394）、未知 tag `unknown default value tag` 点名拒绝（:1421-1425）；`has_default=0`/缺段兼容读 None（:1344-1348）。
  - 可达性：`insert_column_row`（:295-302）经 `append_to_chain` 页内落位，catalog 列行容量 = 页内联上限 ~4074B → >65535B DEFAULT 字面量先在链追加页满失败，回绕生产结构不可达——T12 为表示一致性硬化，非用户可见行为修复（design D11）。
  - 单测先例：`column_row_with_default_roundtrip_all_variants`（:2328）行级往返形态；`no_default_row_output_is_byte_identical_to_legacy`（:2356）与 legacy 兼容读用例为 Preserve 锚点。

  000 已交付、本 Iteration 直接复用的原语（`src/storage/page_format/overflow.rs`）：`write_overflow_chain`（含失败自清理）、`read_overflow_chain`（total_len 校验 + 防环上界）、`collect_overflow_pages`、`free_overflow_chain`、`encode/decode_locator`、`is_overflow_ref`、`chunk_plan`、`INLINE_SLOT_MAX = 4074`。
- Code and Critical Path: T5 = redo 记录消费（recovery.rs 四点 + redo 重排两相）→ R3；T6 = 释放面单点（data_page.rs delete + table_manager drop/truncate 收集）→ R4；T7 = 全通道/形态/基线 e2e（CLI + lib）+ 回填见证 → R1/R2/R3/R5；T12 = catalog DEFAULT 段纯函数两臂（catalog.rs）→ R1 表示一致面；T8 = 文档。加密（4124B 步长、AAD=page_id）与 `:memory:` 后端在存储层自动承接溢出页（000 已验证 lib 面），T7 只做端到端见证。

**Implementation Guidance**

- 装配消费统一模式（与 000 T4 同型）：页闭包内只做 header 解析 + `is_overflow_ref` 判定 + 收集 `(RowId, total_len, first_page)` 引用；出闭包 `read_overflow_chain` 装配；以完整 tuple bytes 继续既有逻辑（`extract_pk_key`/`extract_index_keys`/字节比较/`deserialize_row`）。不在页锁闭包内 await/递归 BufferPool。
- redo 重排（T5）：`redo_tuple_at_row_id` 改三段——① 读闭包：未初始化页 init 0x03 + 幂等检查（slot 已存在 → skip 标记）；② 闭包外：非 skip 时 `write_overflow_chain(tuple_bytes)`（>4074 才走）或直用内联 bytes；③ 落位闭包：`add_slot_with_logical_id(locator_slot_data 或内联 slot_data, row_id.slot_id)`。步骤 3/4/5（链链接、tail、可见性镜像）保持在数据页上，语义不变。链写失败 → `WalError::RedoFailed` 点名（K05：物理落位失败类）。
- 释放顺序（T6）：collect 链页 → 删 slot → free 链（delete 单点）；drop/truncate 先全表扫 slot 收集链页再随数据页释放。free 均 best-effort（对齐既有 drop 语义），失败不阻断主流程。
- T7 规模基线：~1MB String 单行 INSERT → SELECT 读回 → `shutdown()` 崩溃模拟 → 重开恢复 → 读回一致（~2MB WAL 帧，design D10 已确认帧 len u32 无约束）。边界矩阵沿用 000 的 4073/4074/4075（slot_data 口径）与 chunk 4080±1，不重复建测，只补通道/形态轴。
- 崩溃恢复用例的建表落盘：先 `db.close()`（DDL 入 checkpoint）再 `shutdown()` 制造 pending WAL——DDL 不入 WAL（000 偏差 3 教训）。

**Task Contracts**

### T5: 恢复面溢出感知 + CREATE INDEX 回填装配消费

- Requirement/Scenario: R3（恢复两态一致、重放幂等）；R1（SQL 层透明——回填面，000 Review 000 finding 补入）
- Depends on: T4（000 已交付）
- Targets: `src/wal/recovery.rs::redo_tuple_at_row_id`、`build_pk_version_maps`、`derive_old_row_id`、`rebuild_pk_indexes`；`src/executor/create_index.rs`（回填键派生段，:118-163）
- Current behavior: 四个恢复消费点对 locator slot 按 9B 原始字节解析（keyless 身份错误 / old_row_id 比较必失配 / 索引键垃圾）；redo 在页闭包内整段写 slot_data，超页 slot_data 无法经 `add_slot_with_logical_id` 落位（>4074 页内必败）→ 含超页行的 WAL 恢复 RedoFailed 或坏数据。回填对超页表报 unknown tag 错误。
- Required behavior: ① redo：超页记录按「幂等检查 → 闭包外链写（失败自清理）→ `add_slot_with_logical_id` 落 31B locator slot」重放；slot 已存在跳过且不重写链（幂等）；K05 收敛语义与 logical_id 强制落位保持。② 三个预扫/比较消费点经两相装配取完整 tuple bytes 后进既有逻辑，函数签名不变。③ 回填经两相装配派生索引键，含超页表 CREATE INDEX 可达且键与运行期写路径一致。④ `mark_uncommitted_aborted` 保持零改动。
- Preserve: WAL 记录格式与帧解析零变化；`redo_count == 0` 路径零变化；恢复后索引重建、catalog root 写回、`replace_recovery_indexes` 换入语义不变；`add_slot_with_logical_id` 原语不改；回填对内联表行为逐字节不变。
- Forbidden: 不改 WAL record/帧格式；不实现运行期释放（T6）；不动 tuple.rs 表示层。
- Test witness: `tests/overflow_page_test.rs` 新增恢复矩阵（RED：恢复含超页行 WAL 必失败或读回不一致）——超页 INSERT 后 `shutdown()` 崩溃模拟 → 重开 `full_recover` 无 RedoFailed、行与 PK 索引与运行期一致、重复恢复幂等；UPDATE 增长/缩回后的崩溃恢复两态；keyless（无 PK 表）超页行恢复；含超页表 CREATE INDEX（回填）+ 索引点查可达。命令 `cargo test --test overflow_page_test -- --test-threads=1`。
- GREEN condition: 新用例全绿 + 既有恢复套件（`wal_recovery_large_test`/`recovery_replay_convergence_test`/`checkpoint_redo_reduction_test` 等）零回归。
- Verification: `cargo test --test overflow_page_test --test wal_recovery_large_test --test recovery_replay_convergence_test -- --test-threads=1` 0 failed；`cargo test --lib recovery -- --test-threads=1` 0 failed。
- Stop when: redo 两相重排与 MS20-T02 T8 强制落位收敛语义冲突，或幂等跳过与链写顺序无法在单线程恢复内保持（返回 Plan）。

### T6: 生命周期释放（GC/drop/TRUNCATE 无孤儿链）

- Requirement/Scenario: R4（表级释放、版本级释放；写入失败清理 000 已交付）
- Depends on: T3（000 已交付）
- Targets: `src/storage/data_page.rs::delete_tuple_from_data_page`；`src/storage/data/table_manager.rs::{drop_table, truncate_table}`（含 `collect_data_pages` 或其等价收集段）
- Current behavior: `delete_tuple_from_data_page` 仅删 slot——超页版本的溢出链成为不可达孤儿页；`drop_table`/`truncate_table` 沿数据链释放，溢出链页遗留。
- Required behavior: ① `delete_tuple_from_data_page`：slot 为 locator 时先删 slot、后 `free_overflow_chain`（顺序见 design D5）；内联行行为逐字节不变。② `drop_table`/`truncate_table`：释放数据页前全表扫 slot 收集溢出链页一并释放（best-effort，失败不阻断，对齐既有释放语义）；重开后空间状态一致。
- Preserve: `gc_table` 零改动（委托单点自动受益）；free-list 语义不变；`data_chain_status`/dump 预检不受扰（溢出页不入数据链——000 不变量）；TRUNCATE 的 checkpoint-first 步骤序不变。
- Forbidden: 不做页内 compaction；不激活 GC 生产调用者；不动回滚语义（aborted slot 及其链保持可达，R4 明文非泄漏）。
- Test witness: `tests/overflow_page_test.rs` 新增释放矩阵（RED：释放后 free-list/页计数见证链页未回收）——UPDATE 缩回后 `gc_table` 释放旧超页链（lib 面，页可回收见证同 000 `free_overflow_chain_returns_pages_for_reuse` 形态）；drop_table 后链页回收；TRUNCATE 后链页回收且表定义/索引保留语义不变；内联表 drop/TRUNCATE 零回归。
- GREEN condition: 释放矩阵全绿 + `truncate_test`/`gc_test` 定向回归 0 failed。
- Verification: `cargo test --test overflow_page_test --test truncate_test --test gc_test -- --test-threads=1` 0 failed。
- Stop when: 释放顺序与 MVCC 版本链遍历产生 dangling 窗口（返回 Plan）。

### T7: 通道与形态 e2e + 规模基线

- Requirement/Scenario: R1（S4 全写通道、S11 规模基线）、R2（S8 内存/只读、S9 加密联动）、R5（S7 旧库兼容、S10 既有拒绝面）
- Depends on: T5、T6
- Targets: `tests/overflow_page_test.rs`（扩展矩阵）；按需 `tests/cli_test.rs` 定向用例
- Current behavior: 000 已覆盖 lib 写读闭环、SQL 边界三值、chunk 边界、增长缩回、混排、点查、干净重开；通道/形态/基线轴无见证。
- Required behavior: 下列矩阵各至少 1 用例 GREEN——CLI 通道：INSERT VALUES（含子集列+DEFAULT）、UPSERT DO UPDATE、REPLACE、UPDATE 增长（CLI 面）；CTAS 物化含超页源行；INSERT SELECT 跨库写超页行；dump→restore 往返含超页行逐字节一致；import CSV 超页字段。形态：加密库（`--key`）超页写读 + 崩溃恢复；`:memory:` 超页读写；只读会话 SELECT 超页行（零写入保持）。基线：~1MB 单行 INSERT→读回→崩溃恢复→读回一致。边界补充：65535/65536 两 tag 切换点 SQL 层可达（000 已有 lib 面，此处为 e2e 确认或引用既有用例）。既有拒绝面：超长日期字面量等非 String 超大值路径既有拒绝保持（可引用既有用例，无则补 1 例）。
- Preserve: 全部既有通道对内联行行为零回归；错误面文案不变。
- Forbidden: 不做性能优化（1MB 基线只证可用性不设时延断言）；不新增能力。
- Test witness: 上述矩阵用例全 GREEN；受影响定向回归（`cross_db_dml_test`/`ctas_test`/`insert_select_test`/`import_input_surface_test`）0 failed。
- GREEN condition: 矩阵全绿。
- Verification: `cargo test --test overflow_page_test -- --test-threads=1` 0 failed + 定向回归命令 0 failed。
- Stop when: 某通道对超页行结构性不可用且修复超出「装配消费接线」范围（返回 Plan）。

### T8: 文档同步

- Requirement/Scenario: R1/R5（使用面与边界说明）
- Depends on: T7
- Targets: `README.md`、`README.zh-CN.md`、`rtsql-docs/SKILL.md`
- Current behavior: 文档无声明行值长度边界；溢出能力与 4074 阈值、locator 表示对用户不可见但长值行为变化（此前 PageFull 报错，现成功）应记录。
- Required behavior: 双语 README 在能力/边界处补长值支持说明（>4074B slot_data 走溢出页链；无人为硬上限；恢复/释放/加密/内存/只读同等可用；此前超页失败形态终止）；补 DEFAULT 字面量长度边界说明（受 catalog 列行页内容量约束、~4KB 量级、超限建表按既有页满错误拒绝——design D11）；rtsql-docs/SKILL.md 使用面同步（含 ≤65535 与 >65535 String 表示对用户透明、DUMP/IMPORT 往返不受扰）。
- Preserve: 既有章节结构；不新增性能声明。
- Forbidden: 不写未实现能力。
- Test witness: 文档示例实跑（每条可执行示例至少跑 1 次；DEFAULT 边界行以一条超容量 DEFAULT 建表实跑见证既有页满拒绝面）+ 逐条内容 Review。
- GREEN condition: 示例实跑结果与文档一致。
- Verification: 人工——按文档示例逐条执行，所见即判定；结果一句话记入 Act Response。
- Stop when: 文档面与实际行为冲突且原因在产品代码（返回 Plan）。

### T12: catalog DEFAULT 段长字符串语义（replan 并入）

- Requirement/Scenario: R1（DEFAULT 段与 tuple 值表示同语义；「DEFAULT 段长字符串表示一致」场景）
- Depends on: T1（Iteration 000 已交付，`TAG_LONG_STRING = 0x08` 常量与语义已存在，`pub(crate)` 可复用）
- Targets: `src/storage/catalog.rs::serialize_catalog_column_row`（String DEFAULT 臂，:1233-1237）、`deserialize_catalog_column_row`（DEFAULT tag match，:1373-1426）
- Current behavior: DEFAULT 段 String 恒 `[0x02][u16 len][bytes]`，>65535B 字面量长度前缀 `as u16` 回绕；反序列化对 0x08 报 `unknown default value tag`。生产结构不可达（`insert_column_row` 经 `append_to_chain` 页内落位，列行容量 ~4074B，超长 DEFAULT 先页满失败）——本任务为表示一致性硬化，非用户可见行为修复（design D11）。
- Required behavior: 序列化 >65535B String DEFAULT 走 `[0x08][u32 len LE][bytes]`（≤65535 逐字节不变）；反序列化增 0x08 臂（截断/坏 UTF-8 诚实拒绝，错误文案随既有 `default payload` 家族风格）；`has_default=0`/缺段兼容读与其余 tag 零变化。
- Preserve: 既有 catalog 文件零迁移打开（≤65535 布局字节不变）；`insert_column_row`/`append_to_chain` 不改（页内容量边界保持——catalog 行溢出为 change Non-goal）；MS29 `validate_default_value` 声明期校验语义不变；DEFAULT 应用（INSERT 经 tuple 序列化）不变（000 T1 已覆盖）。
- Forbidden: 不为 catalog 行引入溢出页；不动 view_sql/表名/列名等其他 u16 段（均有声明期上界）；不新增 DEFAULT 长度声明校验（既有语义）。
- Test witness: `catalog.rs` 单测新例（RED：65536B DEFAULT 段往返——现序列化长度回绕致读回截断断言失败）：65535/65536 切换点（含段字节布局断言：65535 首 tag 0x02、65536 首 tag 0x08 + u32 前缀）、>65535 往返等价、≤65535 段字节布局与既有不变、0x08 截断拒绝；仿既有 `column_row_with_default_roundtrip_all_variants`（:2328）行级往返形态。命令 `cargo test --lib catalog -- --test-threads=1`。
- GREEN condition: 新单测全绿 + 既有 catalog 单测零回归。
- Verification: `cargo test --lib catalog -- --test-threads=1` 0 failed。
- Stop when: DEFAULT 段格式变更与 `scan_columns` 消费面存在契约外耦合（返回 Plan）。

**Invariants**

- ≤4074B 内联路径逐字节不变（000 不变量延续）。
- VersionHeader 恒内联数据页 slot；版本链遍历、superseded map、abort 标记、commit 传播零感知溢出。
- 溢出页永不进入数据页 tail 链；`data_chain_status`/dump 预检零影响。
- locator/链写序：链先写、locator 最后落位；写入与重放失败均不产生 dangling locator。
- 页锁闭包内不得 await/递归 BufferPool（全部新增装配在闭包外）。
- WAL 帧格式、CRC、`redo_count == 0` 恢复路径零变化。
- catalog DEFAULT 段 ≤65535B 字节布局不变（存量库文件零迁移）；catalog 列行保持页内容量边界（DEFAULT 长度超限按既有页满错误拒绝）。

**Non-goals**

check 命令（Iteration 002）；提交热路径 O(row) 装配优化（MS22 候选）；页校验和；VACUUM/GC 生产激活；自动修复；BLOB/DECIMAL；跨附加库编排。

**Acceptance**

R3（S5）：超页行崩溃恢复两态一致 + 幂等——T5 恢复矩阵见证；R4（S6）：GC/drop/TRUNCATE/写入失败无孤儿链——T6 释放矩阵（写入失败清理 000 已见证）；R1 全通道（S4）+ 规模基线（S11）——T7 矩阵；R2（S8/S9）——T7 形态轴；R5（S7/S10）——T7 边界轴 + M19 全量零回归；R1 透明性回填面——T5 CREATE INDEX 用例；R1 DEFAULT 段表示一致面——T12 catalog 单测。映射：T5→R3/R1、T6→R4、T7→R1/R2/R5、T12→R1、T8→R1/R5 文档面。

**Verification**

- `cargo test --test overflow_page_test -- --test-threads=1` 0 failed（恢复/释放/通道/基线矩阵全量）。
- `cargo test --lib catalog -- --test-threads=1` 0 failed（T12 DEFAULT 段矩阵）。
- 定向回归：`cargo test --test wal_recovery_large_test --test recovery_replay_convergence_test --test truncate_test --test gc_test --test ctas_test --test insert_select_test --test cross_db_dml_test --test import_input_surface_test -- --test-threads=1` 0 failed。
- `cargo test --lib -- --test-threads=1` 0 failed。
- `cargo fmt --check` 与 `cargo clippy --tests` 无新告警。
- M19 全量分批收尾：`--lib`/`--bins` 与全部集成测试目标按名字典序分批（每批 8–12 个目标）串行 `cargo test <明确目标> -- --test-threads=1`，每批 0 failed 方可宣称全量；不使用无目标过滤的全量调用。

**Gate 2 Readiness**

PASS——change 级 Gate 2 于 2026-10-02 随完整计划获批（用户选项「批准完整计划」）；本 Iteration 展开时的补充调查（恢复消费点 file:line、生命周期释放面、CREATE INDEX 回填遗漏）由 000 Plan Review 独立审读完成；2026-10-02 用户指令 replan 并入 T12（proposal 集中决策 3），补充调查（DEFAULT 段序列化/反序列化符号、可达性分析、单测先例）见 Investigation Facts，无新增实质未知项；无 BLOCKED/WAIVED。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- ~1MB 行 UPDATE 产生 ~2MB WAL 帧：恢复内存峰值与 checkpoint 重写时长增大（design D10 验收基线内可控，不做优化）。
- redo 两相重排不得破坏 MS20-T02 T8 的「按 WAL logical_id 强制落位」收敛语义——幂等跳过（slot 已存在）必须在链写之前判定，避免重复恢复重复分配链页。
- 崩溃恢复用例的 DDL 落盘：先 `close()` 再 `shutdown()` 制造 pending WAL（DDL 不入 WAL，000 偏差 3 教训）。
- 频繁崩溃-恢复循环下 unreferenced 链页累积为已知边界（design D5/D6，check 将报告为信息级，不构成本 Iteration 验收项）。
- 非实质选择（装配 helper 拆分、命名、用例组织）由 Act 自定。

## Act Response

- Status: reported

**实际改动**

- T5（恢复面溢出感知 + 回填装配消费）：
  - `src/storage/page_format/overflow.rs`：新增 `pub enum TupleRef`（`Inline(Vec<u8>)` / `Overflow { total_len, first_page }`）与 `from_slot_tuple` / `resolve` 两相装配原语，供全部页闭包消费点复用。
  - `src/wal/recovery.rs::redo_tuple_at_row_id`：重排三段式——① 读闭包（未初始化页 init + 幂等检查）→ ② 闭包外溢出链写（>4074 才走，幂等命中短路在链写前；链写失败 `RedoFailed` 自清理）→ ③ `add_slot_with_logical_id` 强制落位（31B locator 或内联；落位失败 best-effort 释放已写链页）。步骤 3/4/5（链链接/tail/可见性镜像）语义不变。
  - `src/wal/recovery.rs::{build_pk_version_maps, derive_old_row_id, rebuild_pk_indexes}`：页闭包内只做 header 解析 + `is_overflow_ref` 判定（`PreScanPage`/`RebuildScanPage` 类型别名改携 `TupleRef`），出闭包 `resolve` 后进既有 `extract_pk_key`/逐字节比较/`extract_index_keys` 逻辑，函数签名零变化。
  - `src/executor/create_index.rs::backfill_tree`：回填两相装配（闭包携 `TupleRef`，链尾非墓碑出闭包装配后 `deserialize_row` 派生键）——超页表 CREATE INDEX 可达且键与运行期一致；内联表键派生逐字节等价。
  - `src/storage/page_format/mod.rs`：`pub(crate) use tuple::{... TAG_LONG_STRING ...}` 导出（T12 消费）。
- T6（生命周期释放）：
  - `src/storage/data_page.rs::delete_tuple_from_data_page`：删除前页闭包探测 locator → 删 slot → 出锁 `free_overflow_chain`（best-effort，eprintln 对齐 drop 语义）；内联行与 `SlotNotFound` 语义不变；`gc_table` 零改动单点受益。
  - `src/storage/data/table_manager.rs`：新增私有 `collect_overflow_chain_pages`（数据页 header-only 扫描 locator → `collect_overflow_pages`，best-effort）；`drop_table` 链页随 index+data 页一并释放；`truncate_table` 步骤 1 内先释放链页再释放尾页/重置头（checkpoint-first 序不变）。
- T12（catalog DEFAULT 段）：`src/storage/catalog.rs::{serialize_catalog_column_row, deserialize_catalog_column_row}`——String DEFAULT 臂 >65535B 走 `[0x08][u32 len]`（≤65535 字节布局不变）；反序列化 0x08 臂截断/坏 UTF-8 按 `default payload`/`invalid utf8 default value` 家族文案诚实拒绝。
- T7（e2e + 基线）：`tests/overflow_page_test.rs` 9 → 27 例（+18）：写通道矩阵（INSERT VALUES/子集+DEFAULT/UPSERT excluded/REPLACE/UPDATE 增长）、65535/65536 tag 切换点 SQL 层、CTAS、INSERT SELECT 跨库（子进程）、加密崩溃恢复、`:memory:`、只读、~1MB 基线含恢复、dump→restore（子进程）、import 超页字段（子进程）、既有拒绝面（超长非法日期字面量零副作用拒绝）、T5 恢复矩阵 4 例、T6 释放矩阵 3 例；新增本地子进程夹具（`run_cli`/`run_cli_stdin`/`cli_fixture`，沿 import_input_surface_test 先例）。
- T8（文档）：`README.md`/`README.zh-CN.md` 能力列表 +「长值存储」bullet、写面「声明默认值」+ DEFAULT 页内容量边界句；`rtsql-docs/SKILL.md` Behavioral notes + 长值透明 bullet（≤65535/>65535 表示对用户透明、DUMP/IMPORT 往返不受扰、DEFAULT 边界）。

**与计划的偏差**

1. T6 TRUNCATE 释放见证从「free-list 分配集合相等」改为「链页清零落盘 + 新 PK 根源自链页回收」：truncate 同一调用内树重建经 free-list 弹走链页（新根）并释放旧根，集合断言混入合法页流转不稳健；清零见证直接对应既有「立即清零」释放语义，同一 Acceptance 的等价观测形态。gc/drop 两用例维持集合相等见证（无并发流转，稳健）。
2. T7 INSERT SELECT 跨库用例从 lib `execute_sql` 改为 CLI 子进程：ATTACH 为 CLI 会话专属（lib 面点名拒绝 `0A000`，属既有会话边界），ATTACH + INSERT SELECT 同会话多语句执行；dump→restore/import 同为子进程（生命周期子命令无 lib 面）。
3. redo 落位失败时 best-effort 释放本次已写链页：契约未明示该臂，按 Invariant「写入与重放失败均不产生 dangling locator」与运行期写路径 T3 同型处理。
4. `TupleRef` 落位 `overflow.rs` 供 recovery/create_index 复用——契约授权的非实质选择（装配 helper 拆分自定）。

**自检发现与修复**

- 初版回填编辑在 `is_live` 已由外层保证的分支内残留死 `else`，立即简化为直接派生（Gate 3 发现当轮修复）。
- 测试夹具构造错误两处（`read_tuple_from_data_page` 返回值形态、坏 UTF-8 行残留首个 u32 前缀），修正后 RED/GREEN 判定成立（测试侧，非产品代码）。
- clippy `--tests` 报 overflow_page_test 未用 `AsyncStorage` 导入 1 条，删除后干净。
- 遗留 Minor：无。

**验证**

- RED 见证：恢复 4 用例 `redo_tuple_at_row_id ... add_slot failed: No enough space in page` ×3 + `CREATE INDEX ... unknown tag byte: 0xff`；释放 3 用例链页集合失配；T12 三例 65536/往返/截断拒绝 FAIL（65535 布局锚点 PASS）。
- `cargo test --test overflow_page_test -- --test-threads=1` → **27 passed, 0 failed**（rc 0）。
- `cargo test --lib catalog -- --test-threads=1` → **35 passed, 0 failed**（T12 4 新例 + 既有零回归）。
- 定向回归全 0 failed：`wal_recovery_large_test` 7 / `recovery_replay_convergence_test` 3 / `truncate_test` 7 / `gc_test` 3 / `cross_db_dml_test` 17 / `ctas_test` 17 / `insert_select_test` 10 / `import_input_surface_test` 12。
- `cargo test --lib -- --test-threads=1` → **387 passed, 0 failed**。
- `cargo fmt --check` 干净；`cargo clippy --tests` 无告警。
- M19 全量分批收尾（`--lib` 387 + `--bins` 0 + 10 批集成按名字典序串行）：127 / 246 / 164 / 119 / 83 / 215 / 133 / 165 / 127 / 17 passed——**每批 0 failed**，集成合计 1396 passed；总 1783 passed / 0 failed（2 ignored 沿既有）。
- `openspec validate 2026-10-02-ms27-overflow-integrity` → valid（rc 0）。
- T8 人工实跑（所见即判定）：5000 字符 DEFAULT 建表 → `statement 1 of 1 failed: Execution error: Page full` exit 3、零副作用；小值 DEFAULT 建表 exit 0，省略列 INSERT 应用默认值 `{"rows":[[1,"small"]]}`——文档声明与实际一致。

**Persisted Evidence**

None required（Mode: none；验证可低成本重跑，本 Response 承载决定性输出）。

**Experience Candidates**

None（本 Iteration 未发现 change 范围外、有证据的实质缺陷；T5 回填消费遗漏为 000 Review finding、已在本 change 内收口）。

**未解决问题**

1. （继承 000 未解决问题 2）提交热路径溢出行 O(row) 链装配为 MS22 候选观察——1MB 基线只证可用性，未设时延断言。
2. 频繁崩溃-恢复循环下 unreferenced 链页累积为已知边界（design D5/D6），将由 Iteration 002 `rtsql check` 以信息级报告，不构成本 Iteration 验收项。

## Plan Review

- Review Result: accepted

**Findings**

独立审读范围：12 个修改文件 diff 全文（recovery.rs / create_index.rs / data_page.rs / table_manager.rs / catalog.rs / page_format/mod.rs / tuple.rs / data_scan.rs / buffer_pool.rs / README.md / README.zh-CN.md / rtsql-docs/SKILL.md）+ 2 个新增文件全文（overflow.rs 572 行、overflow_page_test.rs 1039 行），逐条核对 Act Response 声明与代码。

偏差分类（对照 Act Response 偏差 1–4）：

1. 偏差 1（T6 TRUNCATE 见证形态）：分类 Act 偏离（非阻塞）。等价性核验通过——「链页清零落盘 ∪ 新 PK 根源自链页」与集合相等见证对应同一 Acceptance（链页回收）；且经推演该断言确定性成立：truncate 释放链页前 free list 为空（夹具新库仅 catalog 0–1、数据头、PK 根已占），新 PK 根分配必然弹自链页集合。gc/drop 两用例维持集合相等见证（无并发流转，稳健）。
2. 偏差 2（INSERT SELECT 跨库走子进程）：分类 Act 偏离（非阻塞）。ATTACH 为 CLI 会话专属、lib 面点名拒绝属既有 spec 边界（`sql-attach-session`）；dump→restore/import 无 lib 面同理。契约未明示通道载体，等价可达。
3. 偏差 3（redo 落位失败 best-effort 释放本次链页）：分类 Act 偏离（非阻塞）。直接落实 Invariant「写入与重放失败均不产生 dangling locator」，与运行期写路径 T3 同型；契约未逐字明示该臂，属 Invariant 的必然推论。
4. 偏差 4（`TupleRef` 落位 `overflow.rs`）：契约明文授权的非实质选择（Plan Context Risks：装配 helper 拆分由 Act 自定）。核验 `from_slot_tuple`/`resolve` 语义正确（内联持有副本、溢出携 locator 字段出锁装配）。

Minor finding（记录不返工）：

- M1：`tasks.md` 任务行「(2026-10-02 Act：…)」日期标注与实际完成时间不符——源码最后编辑（2026-10-03 12:08）、M19 全量分批产物（12:16）与 Act Response 写入（12:17）均为 2026-10-03（文件 mtime 佐证）。历史现场日期失准，归档收尾时由 docs-maintainer 校正，不构成返工项。
- M2：`create_index.rs::backfill_tree` 键派生调用面收窄——旧代码对每个非墓碑 slot 在页闭包内 `deserialize_row`（含被指向的中间版本），新代码仅对「链尾 ∧ 非墓碑 ∧ 未被指向」的行派生（索引条目仅在此形态建立，键值逐字节等价）；对损坏中间版本的报错时点从回填启动后移至实际建条目。Plan 契约未约定损坏中间版本的反序列化行为，属等价控制流内的非实质差异。

Issue 候选：None（本 change 范围外未发现有证据的实质缺陷；T5 回填消费遗漏系 000 Review finding、已在本 change 内收口）。

**Acceptance Gaps**

None——R3（恢复两态 + 幂等，4 用例）、R4（GC/drop/TRUNCATE 释放矩阵，3 用例）、R1 全通道 + 规模基线（10 用例）、R2（加密/`:memory:`/只读）、R5（边界 + M19 全量 1783 passed / 0 failed）、R1 回填面（CREATE INDEX 用例）、R1 DEFAULT 段面（T12 4 单测）、T8 文档（双语 README + SKILL.md，DEFAULT 边界实跑见证）全部满足。

**Evidence**

- 验证采信：001 Act Response 验证表（overflow_page_test 27 passed / `cargo test --lib` 387 passed / 定向回归 8 套件 0 failed / fmt-clippy 干净 / M19 分批 10 批合计集成 1396 + lib 387 = 1783 passed 0 failed / openspec validate rc 0 / T8 人工实跑两例）。采信依据：验证于本会话真实运行、其后覆盖面未变化——时序核对源码 mtime 2026-10-03 12:08 < 测试产物 12:16 < Act Response 12:17。
- 测试断言稳健性独立核验：恢复幂等（二次重放 slot 存在短路）、gc/drop 释放集合断言在空 free list 夹具下确定性成立、truncate 清零+根源见证确定性成立（见 Findings 1）、27 例构成 9（000）+18（001）与 Act 声明一致、clippy 修复后夹具无 AsyncStorage 残留导入。
- 两道不可默认检查满足——GREEN 真实运行有记录且产物时序吻合；本 Review 独立阅读完整 diff 核对 Act 声明与代码一致（redo 三段式幂等短路在链写前、三消费点 + 回填两相装配、delete 先删 slot 后放锁释放链、drop/truncate 链页收集释放且 checkpoint-first 序不变、T12 0x08 两臂与 ≤65535 布局不变、文档内容逐条对点）。

**Follow-up Decision**

接受（`accepted`）：全部 Acceptance 满足，偏差 1–4 均为非实质且有等价论证，Minor finding M1/M2 记录不返工。Iteration Plan 不变。

**Next Cycle**

None

**Next Iteration**

`iterations/002-rtsql-check/000-initial.md`（已展开，Status: ready）
