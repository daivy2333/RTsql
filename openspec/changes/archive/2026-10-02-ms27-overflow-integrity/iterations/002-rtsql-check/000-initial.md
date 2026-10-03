# Iteration 002 / Cycle 000: rtsql check 完整性校验

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T9、T10、T11
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R6/R7（delta spec `data-integrity-verification`）；Iteration 000/001 交付并被本 Iteration 消费的页类型分类（0x01 树叶 / 0x02 树内 / 0x03 slotted 数据·catalog / 0x04 溢出 / 0x00 空闲零页）、`overflow::{decode_locator, collect_overflow_pages, parse_overflow_page, read_overflow_chain}`、`DataChainStatus` 数据链预检先例、`SlottedPageRef` header/slot 访问器全部继续有效
- Excluded scope: 自动修复、页校验和、生产读路径 panic 面修复（Issue 候选另报）、跨附加库编排、性能优化、VACUUM/GC

**Objective**

`rtsql check <db>` 只读结构扫描可用：干净库 exit 0；损坏检出并以「页位置 + 损坏语义」逐行报告（exit 1）；全程不修改库文件；既有打开与生命周期边界（锁/钥/`:memory:`/`--readonly`/pending WAL）按 R7 沿用；命令面与使用文档同步。完成后 R6/R7 全部可验证，change 实施面完成。

**Investigation Facts**

- Current Baseline: master @ 81c8e6f + Iteration 000/001 实施面（工作区未提交：12 修改 + 2 新增文件）。全量 1783 tests / 0 failed / 2 ignored；fmt/clippy 干净（001 Act Response 验证表，Plan Review 001 accepted 采信）。溢出能力 R1–R5 已达可验证状态。
- Current-State Evidence（本 Iteration 展开审读，file:line 已核实）：

  CLI 面（`src/cli/mod.rs`）：
  - `Command` 枚举最近先例 `Checkpoint { db }`（:158）+ 分派臂（:298-300）→ `lifecycle::checkpoint_db(db, key, lock_wait)`；`Check { db }` 同型（clap derive 结构 + `lifecycle_db_target` 名单）。
  - `lifecycle_db_target`（:351-372）：db 目标子命令名单——增 `Command::Check { db }` 臂后，`:memory:` 组合守卫（:249 前置、exit 2 Usage）与 `--readonly` × 生命周期子命令守卫（:257-261 exit 2）自动生效，无需新守卫代码。
  - `execute_command_inner_mode`（:519 起，参数含 `readonly: bool, in_memory: bool`）：`readonly=true` 经 `Database::open_readonly_with_lock_wait` 共享锁打开（缺失文件拒绝、WAL 未决 `WalPendingRecovery` 拒绝）——check 的只读编排通道；open 错误经 `open_error_status`（:471-489）映射：`DatabaseLocked` → `Locked` exit 4、`InvalidKey` → exit 5、其余（含 `WalPendingRecovery`）→ `General` exit 1（错误文本携带打开语境与存储错误详情）。check 直接复用，不新增映射。
  - `checkpoint_db`（`src/cli/lifecycle.rs:207-262`）为最接近的编排先例：`resolve_existing_db(db)`（缺失 → `General` exit 1 `does not exist`）→ `super::execute_command_inner(&db_path, key, lock_wait, closure, sigint, sigterm)`；check 改走 `_mode` 的 readonly 臂，闭包内执行校验并渲染报告。

  存储扫描面（全部只读可达）：
  - 页类型现值：0x01 `LEAF_NODE` / 0x02 `INTERNAL_NODE`（`src/storage/btree/node.rs:7-8`）；0x03 slotted 页——数据页与 catalog 页同型（`src/storage/catalog.rs:558,622` 均 `SlottedPage::init(page, 0x03)`）；0x04 溢出页（`overflow.rs::OVERFLOW_PAGE_TYPE`）；0x00 空闲页全零（`BufferPool::free_page` 清零落盘，`buffer_pool.rs:439`）。
  - 页总数：`AsyncStorage::page_count()`（`async_storage.rs:22` trait 方法，FileStorage/MemoryStorage 均实现）；文件布局 = 64B 格式头（`file_header.rs::HEADER_SIZE`）+ 4096B/页。
  - `SlottedPageRef`（`slotted_page.rs:152-196`）：`header()` → `SlottedPageHeader`（`page_type`/`next_page_id`/`free_space_offset`）、`slot_count()`、`get_slot(index) -> Option<Slot>`、`get_slot_data(&slot) -> &[u8]`。现有访问器不做 bounds 校验（损坏注入下越界读是本任务新增防护面：check 侧迭代前校验 slot offset/length 落在页界内且 slot 区与数据区不交叠，越界即 finding 而非 panic）。
  - 数据链：`TableManager::data_chain_status(&table)`（`table_manager.rs:1003`，`DataChainStatus { terminal, cyclic, tail }` :70）既有 walk + catalog tail 对照先例；`collect_data_pages`（:1358 区域，私有）沿 `next_page_id` 收集——check 的链 walk 自行实现（需逐页产出 slot 扫描与防环 visited，单靠收集器不够）。
  - 溢出链：`overflow::collect_overflow_pages`（防环）、`parse_overflow_page`（页类型 + payload 界校验）、`read_overflow_chain`（total_len 对照）、`decode_locator`（slot tuple 段判定）——001 释放面已消费同型扫描，check 直接复用。
  - 索引树：`IndexManager::root_page_id()`（`index_manager.rs:58`）、`collect_all_pages`（:405，严格 Result）、`collect_all_pages_tolerant`（:469，洞容忍返回 `Vec<PageId>` 不报错）——树遍历复用 tolerant 收集器 + check 侧防环/页类型/文件界校验自行报告；表三类根来自 `TableMeta`（PK `index_manager`、`unique_indexes`、`secondary_indexes`）。
  - catalog：`db.table_manager.catalog()` → `scan_tables()`/`scan_columns()`（dump 先例 `lifecycle.rs:309-318`）；kind=View 行无数据页与索引（001 已知：`open_or_init` 跳过视图行接线）——check 对 View 行跳过数据/索引遍历，仅 catalog 解析面覆盖。
  - lib 入口接缝：`db.buffer_pool` / `db.table_manager` 为 Database 公开字段（dump/checkpoint 既有消费）；新模块 `src/storage/integrity.rs` 以 `(&BufferPool, &TableManager)` 为入参（storage 层内聚，不反向依赖 Database），导出 `IntegrityFinding`/`IntegrityReport` 结构与 async 校验入口。
  - 损坏注入测试面：明文库 `close()` 后以 `HEADER_SIZE + page_id * 4096` 偏移 `std::fs` 直改文件字节即可注入页级损坏（重新打开后 BufferPool 从盘载入被改页）；加密库页内容为密文、不可定位注入——加密矩阵只验打开边界与干净往返。
- Code and Critical Path: T9 = 新 `src/storage/integrity.rs`（census + catalog + 逐表数据链/slot/溢出链/索引树遍历 + findings/汇总，只读）→ R6/R7 lib 面；T10 = `src/cli/mod.rs`（Command 枚举 + lifecycle_db_target + 分派臂）+ `src/cli/lifecycle.rs::check_db`（readonly 编排 + 报告渲染 + 退出码）+ `tests/check_command_test.rs` → R6/R7 命令面；T11 = README 双语 + `rtsql-docs/SKILL.md` → R7 文档面。

**Implementation Guidance**

- 扫描顺序建议：页 census（`page_count()` 界内逐页分类 + 未知类型 finding）→ catalog 解析（`scan_tables`/`scan_columns` 失败即 finding）→ 逐 Table 行（View 跳过③④⑤）：③ 数据链 walk（防环 visited、terminal 对照 catalog tail、页类型校验）+ bounds-checked slot 迭代 + locator 收集；④ 溢出链逐链校验（复用 overflow 原语）；⑤ 三类索引根遍历（tolerant 收集 + 防环 + 页类型/文件界校验）→ 未被任何 slot 引用的 0x04 页（census 溢出页集合 − 各链页集合）报告 info 级。
- findings 分级：`error`（结构损坏）与 `info`（未引用溢出链等已知边界——001 Act Response 未解决问题 2 的承接）。exit 1 仅由 error 级决定；info 级不改退出码（design D5/D6 交接口径）。
- 输出形态：每条 finding 恰一行（含页位置与损坏语义）；干净库 stdout 恰一行确认；末行汇总含计数。具体文案非实质由 Act 定，测试以行数与「含页号/语义子串」断言。
- pending WAL：readonly 打开被 `WalPendingRecovery` 拒绝 → 复用 `open_error_status` 的 General exit 1（错误文本点名 WAL 未决）；SHALL NOT 为 check 改用读写打开（会触发恢复写，违反 R7 只读）。
- T9 测试注入形态：干净库建表写行 → `close()` → 按页偏移直改文件字节 → 重开（readonly 或 lib 打开）执行校验断言检出。RED 见证 = 实现前注入损坏不被检出（或访问 panic）。

**Task Contracts**

### T9: lib 完整性校验入口

- Requirement/Scenario: R6（损坏检出与位置报告、干净库通过）；R7（只读不改）
- Depends on: None 硬前置（消费 000/001 页类型与溢出原语；按序执行在 001 后）
- Targets: 新 `src/storage/integrity.rs`（`IntegrityFinding`/`IntegrityReport` + async 校验入口，入参 `&BufferPool` + `&TableManager`）
- Current behavior: 无任何全库结构校验能力；仅有 dump 每表 `data_chain_status` 预检（stderr 单行告警，rc 0）与 `overflow::` 链原语（释放面消费）。
- Required behavior: ① 页 census——`page_count()` 界内逐页分类（0x00/0x01/0x02/0x03/0x04），未知类型逐页 finding（含页号 + 实际字节）；② catalog `scan_tables`/`scan_columns` 解析失败即 finding；③ 逐 Table 行——数据链 walk（防环、terminal 对照 catalog tail、页类型校验）+ bounds-checked slot 迭代（slot offset/length 越页界或 slot 区/数据区交叠即 finding，不 panic）+ locator 溢出链校验（防环、页类型/payload 界、total_len 对照）+ PK/唯一/二级三类索引根遍历（防环、页类型、文件界内）；④ 未被任何 slot 引用的 0x04 页报告为 info 级；⑤ `IntegrityReport` 携带 findings（页位置 + 语义 + error/info 分级）与 census 汇总；全程只读（`with_page_data`/`SlottedPageRef` 消费，不写页不清脏位不触发 checkpoint）。
- Preserve: 不修改任何页与文件字节；WAL 帧 CRC 与恢复行为零变化；既有 `data_chain_status` 与 dump 预检行为不变；`overflow::` 原语语义不变（只消费不改动）。
- Forbidden: 不自动修复；不实现页校验和；不修改生产读路径（读路径 panic 面为 Issue 候选另报）；不做跨附加库编排；check 自身对损坏输入 SHALL NOT panic（一律 finding）。
- Test witness: `src/storage/integrity.rs` 模块单测——干净库零 error finding + census 计数正确（含溢出页计数）；逐类注入检出（明文库 close 后按 `HEADER_SIZE + page_id*4096` 直改文件字节）：数据页字节翻转（slot 越界或 census 语义）、溢出链 next 指针成环、树根页类型破坏、未引用溢出链 info 级报告（RED：实现前各注入不被检出）。命令 `cargo test --lib integrity -- --test-threads=1`。
- GREEN condition: 新单测全绿 + lib 零回归。
- Verification: `cargo test --lib -- --test-threads=1` 0 failed。
- Stop when: census/遍历无法在只读约束内实现（需写页、清脏位或绕过 BufferPool 直写），或溢出/树遍历对损坏输入必然 panic 且无法以 finding 收敛（返回 Plan）。

### T10: CLI `check` 子命令

- Requirement/Scenario: R6（命令面检出与报告）；R7（边界沿用、退出码、不改文件）
- Depends on: T9
- Targets: `src/cli/mod.rs`（`Command::Check { db }` 变体、`lifecycle_db_target` Check 臂、分派臂）、`src/cli/lifecycle.rs::check_db`、新 `tests/check_command_test.rs`
- Current behavior: 无 check 子命令（编译期不存在）。
- Required behavior: ① `rtsql check <db>`：`resolve_existing_db` 缺失 exit 1；readonly 打开（`execute_command_inner_mode` readonly=true）共享锁 + 锁等待预算，冲突 exit 4、错钥 exit 5（`open_error_status` 既有承载）、pending WAL 拒绝开库 exit 1（不改用读写打开）；② 执行 `check_integrity` 渲染报告——每条 finding 一行（页位置 + 语义），末行汇总含计数，干净库 stdout 恰一行确认；③ 退出码：干净 exit 0、存在 error 级 finding exit 1（info 级不改退出码）、锁 4、钥 5、`:memory:`/`--readonly` 组合 exit 2（`lifecycle_db_target`/全局守卫自动）；④ `--format` 对本命令忽略（对齐 dump/schema 既有形态）。
- Preserve: 既有子命令分派与守卫零变化（`lifecycle_db_target` 增臂不影响既有臂）；`open_error_status` 映射不变；信号优雅停机编排不变（`execute_command_inner_mode` 既有两阶段）。
- Forbidden: 不新增退出码语义；不接入 plan/执行管道；不做自动修复；不为 check 绕过 readonly 通道。
- Test witness: `tests/check_command_test.rs` 矩阵——干净库 exit 0 + 确认行；逐类损坏注入 exit 1 且输出含页号与语义子串；pending WAL exit 1；加密库对钥 exit 0/错钥 exit 5；`--readonly`/`:memory:` exit 2；缺库 exit 1；锁冲突 exit 4（短预算校准，沿 MS28 D8 先例）；校验前后主库文件全字节 read 对比不变（R7 目标行为直接判定，非身份型证据）。RED：命令不存在（编译期）+ 守卫/退出码矩阵缺臂。
- GREEN condition: check_command_test 全矩阵绿。
- Verification: `cargo test --test check_command_test -- --test-threads=1` 0 failed。
- Stop when: readonly 打开通道无法承载结构扫描（如加密页只读解密不可用），或注入矩阵与退出码映射出现 spec 外的新语义决定（返回 Plan）。

### T11: 文档同步

- Requirement/Scenario: R7（使用面与边界说明）
- Depends on: T10
- Targets: `README.md`、`README.zh-CN.md`、`rtsql-docs/SKILL.md`
- Current behavior: 三处文档无 check 命令记载。
- Required behavior: 双语 README 在命令/能力处补 `rtsql check <db>`（只读结构扫描、损坏报告页位置与语义、退出码 0/1/4/5、`:memory:`/`--readonly` exit 2、不修复不修改文件、pending WAL 需先以读写会话恢复后再 check）；`rtsql-docs/SKILL.md` Lifecycle 命令表 + Behavioral notes 同步。不写未实现能力。
- Preserve: 既有章节结构与既有命令描述不回归。
- Forbidden: 不新增性能声明；不写自动修复承诺。
- Test witness: 文档示例实跑（干净库与注入损坏库各实跑 1 次，所见即判定；DEFAULT 类边界无涉及则不适用）+ 逐条内容 Review。
- GREEN condition: 示例实跑结果与文档一致。
- Verification: 人工——按文档示例逐条执行，所见即判定；结果一句话记入 Act Response。
- Stop when: 文档面与实际行为冲突且原因在产品代码（返回 Plan）。

**Invariants**

- check 全程只读：不改页、不清脏位、不触发 checkpoint/WAL 写、不修改库文件与伴生文件。
- R7 既有边界映射不变：锁 exit 4、钥 exit 5、`:memory:`/`--readonly` exit 2、缺库 exit 1、pending WAL exit 1。
- 退出码由 error 级 finding 决定；info 级（未引用溢出链）不改变退出码。
- 页类型分类与 000/001 实现一致（0x01/0x02/0x03/0x04/0x00）；census 不引入新页类型假设。
- WAL 帧 CRC、恢复行为、既有 dump/schema/checkpoint 等子命令零变化。

**Non-goals**

自动修复；页校验和；VACUUM/GC 生产激活；生产读路径 panic 面修复（Issue 候选另报）；跨附加库编排；性能优化。

**Acceptance**

R6（损坏注入检出、干净库通过）——T9 单测（逐类注入检出 + census）+ T10 check_command_test 矩阵；R7（校验不修改库文件、既有边界沿用）——T10 文件全字节对比与守卫/退出码矩阵 + WAL CRC 零变化回归（M19 全量）。映射：T9→R6/R7 lib 面、T10→R6/R7 命令面、T11→R7 文档面。

**Verification**

- `cargo test --lib -- --test-threads=1` 0 failed（T9 单测 + lib 零回归）。
- `cargo test --test check_command_test -- --test-threads=1` 0 failed。
- 定向回归：`cargo test --test cli_test --test catalog_durability_test --test checkpoint_command_test -- --test-threads=1` 0 failed（CLI 面与 dump/checkpoint 相邻域）。
- `cargo fmt --check` 与 `cargo clippy --tests` 无新告警。
- M19 全量分批收尾：`--lib`/`--bins` 与全部集成测试目标按名字典序分批（每批 8–12 个目标）串行 `cargo test <明确目标> -- --test-threads=1`，每批 0 failed 方可宣称全量；不使用无目标过滤的全量调用。

**Gate 2 Readiness**

PASS——change 级 Gate 2 于 2026-10-02 随完整计划获批（用户选项「批准完整计划」，T02 方案「仅 rtsql check」同批批准）；本 Iteration 展开时的补充调查（CLI 编排先例 file:line、页类型常量、扫描原语可达性、lib 入口接缝、损坏注入测试面、pending WAL 语义）由 002 展开审读完成；pending WAL 语义与 error/info 分级为 R7 只读约束与 design D5/D6 交接口径的直接组合，无新增实质未知项；无 BLOCKED/WAIVED。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- 树遍历对损坏树可能遇到环/洞：`collect_all_pages_tolerant` 不报错返回可达页——防环与异常由 check 侧自行报告，SHALL NOT 让树遍历 panic。
- 加密库页注入不可行（密文不可定位）——损坏注入矩阵限于明文库；加密矩阵只验打开边界与干净往返。
- error/info 分级与退出码的组合为 R6/R7 与 design D5/D6 的推论；若 Act 实施中发现 spec 语义与该分级冲突，返回 Plan。
- 非实质选择（finding 文案、`IntegrityReport` 字段命名、单测组织、报告渲染细节）由 Act 自定。

## Act Response

- Status: reported

### Actual Changes

- **T9** — 新增 `src/storage/integrity.rs`（`check_integrity(&BufferPool, &TableManager)` 入口 + `IntegrityFinding`/`FindingSeverity`/`IntegrityReport` 结构 + 私有 `Checker` 编排）：① 页 census——`page_count()` 界内逐页首字节分类（0x00/0x01/0x02/0x03/0x04），未知类型逐页 error finding（页号字段 + `unknown page type 0x..` 语义）；② catalog `scan_tables`/`scan_columns` 解析失败即 finding（View 行只覆盖 catalog 解析面，数据/索引遍历跳过）；③ 逐 Table 行——数据链 walk（visited 防环、页类型校验、`validate_slots` bounds-checked slot 迭代〔slot_count 容量门 + fso 数据区界 + 逐 slot offset/length 越界/交叠 finding，全部先验后读，损坏输入零 panic〕、terminal 对照 catalog tail〔见偏差 2 的分级〕）+ locator 溢出链逐链校验（自实现 walk 复用 `parse_overflow_page`：防环、页类型/payload 界、装配字节总数对照 locator 声明）+ PK/唯一/二级三类索引根遍历（自实现 DFS：防环、文件界、页类型校验、内部节点 slot 条目 `[Key][child]` 结构校验 + header 最左孩子遍历；叶兄弟指针纳入遍历）；④ census 溢出页集合 − 被引用集合 → 逐页 info（`not referenced … leaked space`）。`src/storage/mod.rs` +1 行注册 `pub mod integrity`。模块单测 6 例（干净库零 finding + census 计数含溢出页 3 页精确断言；逐类注入：未知页类型 / slot_count 越界 / 溢出链成环 / 树根类型破坏 / 未引用溢出链 info），夹具经 checkpoint 后按 `HEADER_SIZE + page*4096` 直改文件字节。
- **T10** — `src/cli/mod.rs`：`Command::Check { db }` 变体（Checkpoint 后）+ `lifecycle_db_target` Check 臂（`:memory:`/`--readonly` 全局守卫自动 exit 2）+ 分派臂；`src/cli/lifecycle.rs::check_db`：`resolve_existing_db` 缺失 exit 1 → `execute_command_inner_mode` readonly=true（共享锁 + 锁等待预算；pending WAL 经 `open_error_status` 既有 General exit 1，不改读写打开）→ `check_integrity` → 渲染（每 finding 一行 `error|info: page N: <语义>`，干净库恰一行 `check ok: N pages scanned, no corruption found`，有 finding 时末行汇总计数）→ 退出码：干净 0、error 级 1（`check found N error finding(s)` 经既有 General 面落 stderr）、info 不改退出码、锁 4、钥 5。`--format` 天然不消费（对齐 dump/schema 形态）。新增 `tests/check_command_test.rs` 8 用例：干净库单行 + 主库文件全字节不变 + `--format json` 忽略、损坏注入 exit 1 报告页号与语义 + 文件不变、pending WAL exit 1 → 读写会话恢复 → 再 check exit 0（info 行锁定）、加密对钥 0/错钥 5、`--readonly`/`:memory:` exit 2、缺库 exit 1 零创建、锁冲突短预算 exit 4（MS28 D8 校准先例）、未引用溢出链 info exit 0。
- **T11** — `README.md`/`README.zh-CN.md` 命令表各 +1 行（能力、退出码 0/1/4/5、`:memory:`/`--readonly` exit 2、不修复不修改、pending WAL 先读写恢复）；`rtsql-docs/SKILL.md` Lifecycle 表 +1 行 + Behavioral notes +1 条（含恢复后 tail 良性差异的 info 语义说明）。未写未实现能力、无性能声明、既有章节零改动。
- 本 Cycle 产品代码 diff：新 `src/storage/integrity.rs` + 新 `tests/check_command_test.rs` + `src/cli/{mod,lifecycle}.rs`（+86/−1）+ `src/storage/mod.rs`（+1）+ 三处文档各 1–2 行。

### Deviations from Plan

1. **索引树遍历自实现 walk，未复用 `collect_all_pages_tolerant`**——Investigation Facts 将 tolerant 收集器列为复用候选；实施发现其对不可读/非法类型页 `eprintln!("[recovery] …")` 且静默吞洞、不产出 findings，与「每 finding 恰一行」的报告契约和 R6 页位置语义冲突。改为 integrity.rs 内 DFS（防环/文件界/页类型/条目结构自校验），行为契约（防环、页类型、文件界内）逐项覆盖。溢出链校验同理自实现 walk 复用 `parse_overflow_page` 原语（避免 `read_overflow_chain` 整链物化）。非实质实现路径选择，契约 Required behavior 逐项满足。
2. **数据链 tail 对照按单调性分级（terminal > tail → info；terminal ≤ tail → error）**——Plan 假设 catalog 持久化 tail 恒为链末端真相源；T10 RED→GREEN 后首跑发现：WAL 恢复重放只推进内存 `data_page_tail`（`recovery.rs` redo 步骤 4/6，`recovery.rs:218-235`）不写回 catalog 行，崩溃恢复后健康库的磁盘 tail 必然落后于链 terminal（实测 tail=2 / terminal=11）。无条件 error 使恢复后正常库违反 delta spec R6「干净库通过（exit 0）」场景。按方向拆分：terminal **领先**持久化 tail = 重放增长的已知良性形态（info，不改退出码）；terminal **落后或等于但不同** = catalog 已记录页不可达的截断类损坏（error）。两种 spec 场景（损坏检出 / 干净库通过）均满足，非新退出码语义。底层「恢复不持久化 tail」作为 Issue 候选报告（见 Experience Candidates）。
3. **树遍历对 re-visit 静默跳过，不报 cyclic**——叶兄弟链使树页图为 DAG（内部节点孩子与兄弟指针双路可达），re-visit ≠ 环；首跑在健康库误报 5 个「index tree is cyclic」。改为 re-visit 静默跳过（先例 `collect_all_pages_tolerant` 同策略），真环经 visited 终止不挂起（防环契约满足）；数据链与溢出链为线性链，revisit 即真环，保持 error。测试矩阵无需变更（树 cyclic 本就无注入用例，防挂起由 visited 结构性保证）。
4. 计划外零修改；`schema` 函数文档注释在编辑中一度被并行（已当场还原，净零变化）。

### Self-Review Findings

- T9 Gate 3 发现内部节点最左孩子存于 `header.next_page_id`（`InternalNodeRef::leftmost_child`）而遍历只走 slot 条目——最左子树不可达（损坏静默）。已补齐并重跑 integrity 6/6 GREEN。修复仅增遍历覆盖，无行为回归面。
- T10 首个 GREEN 运行暴露偏差 3 的健康库误报（`test_check_pending_wal_exit_1_then_recovers` 失败），修复后全矩阵绿。
- 遗留 Minor：无。

### Verification

- `cargo test --lib --bins -- --test-threads=1` → 393 passed / 0 failed（bins 0）；M19 批 1。
- `cargo test --test check_command_test -- --test-threads=1` → 8 passed / 0 failed（RED 见证：实现前 8 failed——`check` 被主命令当作库名解析 exit 1）。
- `cargo test --lib integrity -- --test-threads=1` → 6 passed / 0 failed（RED 见证：stub 空报告 6 failed）。
- 定向回归 `cargo test --test cli_test --test catalog_durability_test --test checkpoint_command_test -- --test-threads=1` → 117+2i / 4 / 6 全 0 failed。
- `cargo fmt --check` → clean；`cargo clippy --tests` → 无新告警（仅既有 cargo config 弃用提示）。
- M19 全量分批收尾：12 批（lib+bins 1 批 + 集成 11 批，每批 8–12 目标按名字典序）串行 `--test-threads=1`，每批 0 failed；合计 **1797 tests / 0 failed / 2 ignored**（lib 393 + 集成 1404；较 001 基线 1783 净增 +14 = check_command_test 8 + integrity 单测 6）。
- `openspec validate 2026-10-02-ms27-overflow-integrity` → valid，rc 0。
- T11 人工实跑：干净库 `check ok: 4 pages scanned, no corruption found` exit 0（恰一行）；同库页类型字节注入后逐行 `error: page 2: unknown page type 0x42` / `error: page 2: data page of table 'people' has page type 0x42, expected 0x03` + 汇总行，exit 1。pending WAL 场景由 check_command_test 自动矩阵承载。

### Persisted Evidence

None required（计划 Mode: none；全部验证可低成本重跑，本 Response 携带决定性输出）。

### Experience Candidates

- **Issue 候选（范围外，`wal-recovery-replay-integrity` 域）**：WAL 恢复重放不持久化 `data_page_tail`——`redo_tuple_at_row_id` 只推进内存 tail（`src/wal/recovery.rs:218-235` 步骤 4/6），运行期 DML 经 `TableManager::write_tuple` 持久化 tail（`src/storage/data/table_manager.rs:1638-1656`）而恢复路径无对应写回。实证后果：崩溃恢复后健康库磁盘 tail 落后于链 terminal（本 Cycle 测试实证 tail=2/terminal=11），① `dump` 既有链完整性预检对恢复后库误告警「备份可能不完整」（既有行为，rc 0）；② 本 check 按偏差 2 以 info 报告。推演风险（未构造复现，仅代码推演）：恢复后的库再次写入时以陈旧 tail 页为写目标，该页已满时分配新页并覆写其 `next` 指针，旧 tail 与 terminal 之间的恢复页行存在链可达性风险。证据引用：本 Response 偏差 2/验证节 + 上述 file:line。是否立项由用户/Recorder 裁定，Act 不落账。

### Open Questions

None（无阻塞、无遗留 Minor）。

## Plan Review

- Review Result: accepted

### Findings

非阻塞 Minor finding 三项，不构成 Acceptance gap；偏差四项已逐项对照代码与 diff 独立核实，分类如下。

**偏差核实（独立阅读代码与 diff，非采信 Act 自检）**

1. **偏差 1（索引树遍历自实现 walk）→ Plan 遗漏，非实质。** 独立核实 `IndexManager::collect_all_pages_tolerant`（`src/storage/btree/index_manager.rs`）对不可读页确为 `eprintln!("[recovery] …")` + `continue` 静默吞洞、不产出 findings，与「每 finding 恰一行」报告契约和 R6 页位置语义冲突——Plan 将其列为复用候选确属调查遗漏；Act 的自实现 DFS 对契约 Required behavior（防环、文件界、页类型、内部节点 `[Key][child u32]` 条目、header 最左孩子、叶兄弟指针）逐项覆盖，且与 `InternalNodeRef::get_child_page_id`/`leftmost_child`、`LeafNodeRef::next_leaf_page_id` 的真实布局逐字节一致（node.rs 对照核实）。
2. **偏差 2（tail 对照按方向分级）→ Plan 错误，处置正确。** Plan 假设 catalog 持久化 tail 恒为链末端真相源；独立核实 `redo_tuple_at_row_id` 步骤 4（`src/wal/recovery.rs:233` 区域）确只写 `*table_meta.data_page_tail.lock()` 内存 tail、无 catalog 写回——恢复后健康库磁盘 tail 必然落后于链 terminal，无条件 error 违反 delta spec R6「干净库通过」场景。Act 的方向拆分（terminal > tail → info；terminal ≤ tail → error）同时满足 R6 两场景，为契约 error/info 框架内的语义收敛，非新退出码语义；`test_check_pending_wal_exit_1_then_recovers` 以端到端见证锁定该形态。底层「恢复不持久化 tail」经独立核实成立，维持为范围外 Issue 候选（见下）。
3. **偏差 3（树 re-visit 静默跳过）→ Plan 遗漏，修复正确。** 叶兄弟链使树页图为 DAG（父指针 + 兄弟指针双路可达），re-visit ≠ 环；与 `collect_all_pages_tolerant` 同策略，真环经 visited 终止不挂起。
4. **偏差 4**：非事件（编辑期并行还原，净零变化），无遗留。

**Minor findings（记录不返工）**

1. `check_data_chain` 末尾 terminal match 的 `None =>` 臂不可达——while 循环唯一正常出口是 `terminal = Some(raw)`，环/越界/不可读路径均提前 `return`。防御式死代码，非实质，不要求修复。
2. 方向分级存在固有掩蔽边界：若 catalog tail 值本身向下损坏，真实截断损坏会被判 info。这是无校验和方案下以参考值对照的结构性限制，R6 两 spec 场景均满足，代码注释与 SKILL.md Behavioral note 已如实记载，无需本 Iteration 动作。
3. T10 契约「逐类损坏注入」在 CLI 层由页类型一代表类承载、其余类由 T9 lib 单测逐类承载——与 Acceptance 既有映射（T9 单测逐类 + T10 矩阵）一致，非缺口。

**范围外 Issue 候选（确认成立，不落账）**

WAL 恢复重放不持久化 `data_page_tail`（`recovery.rs` redo 步骤 4 只推进内存 tail，运行期 `TableManager::write_tuple` 有持久化而恢复路径无对应写回）：独立核实属实；实证后果（dump 既有链预检对恢复后库误告警、check 以 info 报告 tail 落后）与推演风险（恢复后再写入以陈旧 tail 页为写目标的链可达性风险，Act 已注明未构造复现）记录于 Act Response Experience Candidates。是否立项由用户裁定；若立项走 `openspec-experience-recorder`，域为 `wal-recovery-replay-integrity`。

### Acceptance Gaps

None。

- R6 损坏检出与位置报告：integrity.rs 6 单测逐类注入检出（未知页类型 / slot_count 越界 / 溢出链成环 / 树根类型破坏 / 未引用溢出链 info / 干净库 census）+ check_command_test 损坏注入 e2e（exit 1 + 页号 + 语义子串）——代码与测试独立读毕，与代码一致。
- R6 干净库通过：`test_check_clean_db_single_line_file_unchanged`（exit 0 + 恰一行确认）+ `clean_db_zero_findings_and_census_counts`（census 精确断言含 3 溢出页）。
- R7 只读不改文件：两用例全字节 read 对比 + check 全程 `with_page_data` 只读访问器消费（无写页/清脏位/checkpoint 通道）+ readonly 打开编排（`check_db` 走 `execute_command_inner_mode` readonly=true，独立核实）。
- R7 既有边界沿用：锁 4（短预算 D8 校准）/错钥 5/`:memory:`·`--readonly` exit 2/缺库 exit 1 零创建/pending WAL exit 1 且恢复后 exit 0——测试矩阵逐项独立读毕。
- T11 文档：三处 diff 逐行读毕，check 行为描述与测试见证逐条对应（退出码、pending WAL 指引、恢复后良性 info 语义），无未实现能力、无性能声明。
- 本 Cycle 002 范围外 diff（tuple/overflow/data_page/data_scan/buffer_pool/catalog/table_manager/recovery/create_index 等）属 Iteration 000/001 已 accepted 实施面，不在本次 Review 范围。

### Evidence

- 独立阅读（本 Review）：`src/storage/integrity.rs` 全文、`tests/check_command_test.rs` 全文、`git diff src/cli/{mod,lifecycle}.rs src/storage/mod.rs README.md README.zh-CN.md rtsql-docs/SKILL.md`、消费契约抽查（`overflow.rs::decode_locator/parse_overflow_page/OVERFLOW_PAYLOAD_MAX`、`slotted_page.rs::SlottedPageRef::get_slot` 与 `validate_slots` 前置校验的防 panic 序、`node.rs` 内部节点条目与最左孩子/叶兄弟布局、`table_manager.rs::TableMeta.data_page_tail`、`index_manager.rs::collect_all_pages_tolerant`、`recovery.rs` redo 步骤 4）。损坏输入零 panic 面独立复核：全部 accessor 调用前有界校验（`validate_slots` slot_count 容量门先于 `get_slot`、`parse_overflow_page` payload 上界 4080 = PAGE_SIZE−16、`decode_locator` 探测切片有 `end-start > 22` 门）。
- 采信 Act Response 未失效结论（来源：Act Response › Verification，2026-10-03 新鲜执行，其后工作区未变化）：`cargo test --lib --bins` 393/0、`check_command_test` 8/0（RED 见证 8 failed）、`integrity` 6/0（RED 见证 stub 6 failed）、定向回归 cli/catalog_durability/checkpoint_command 0 failed、fmt clean、clippy 无新告警、M19 全量 12 批 **1797 tests / 0 failed / 2 ignored**、`openspec validate` valid、T11 人工实跑两例。Plan Review 未触发公共规则 › 验证 的重跑情形，不重复运行。

### Follow-up Decision

接受（`accepted`）：三项 Minor finding 均不阻塞 Acceptance；四项偏差全部核实为非实质（Plan 遗漏 ×2、Plan 错误 ×1 且处置正确、非事件 ×1），行为契约、不变量与 R6/R7 场景全部由代码与测试见证满足；无当前 Cycle 修复项，不创建 rework/replan Cycle。范围外 Issue 候选一项维持报告状态，待用户裁定。

### Next Cycle

None

### Next Iteration

None（Iteration 002 为 Iteration Map 最后一轮；change 实施面全部完成，R1–R7 全 Covered。后续收尾——delta specs 两新域合并、SNAPSHOT/tasks 刷新与归档——属 `openspec-docs-maintainer` 职责，等待用户指令。）
