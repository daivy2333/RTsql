# Iteration 000 / Cycle 000: catalog 持久化正确性（P0 静默丢数据收口）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2, T3, T4
- Acceptance gaps: None（initial）
- Repair items: None
- Inherited scope: change proposal/design 全部 P0 相关决策（design D1 等长原地更新、D2 变长重定位、D3 诚实化）；`catalog-update-persistence` delta spec R1-R6；任务映射见 change `tasks.md`
- Excluded scope: P1 别名解析（Iteration 001）、P2 dump 预检与 P3 主命令显式开库及文档（Iteration 002）、既有损坏库修复工具、页格式变更、WAL/checkpoint 链路改动

**Objective**

catalog 行更新在跨进程多轮写入下持久正确：等长更新原地写回（窒息页恢复更新能力、零泄漏）、变长更新跨页重定位（同名恰一行）、全部失败路径诚实传播；多轮「独立会话写→关闭→追加」行数单调、五面一致；UNIQUE 与二级索引根分裂重开保持。修复前 T3 同测试可见证 RED。

**Investigation Facts**

- Current Baseline: 分支 master @ 1668c95（v0.2.0，工作区 clean），全量 1590 tests / 0 failures（SNAPSHOT 2026-09-30，Plan 采信）。本 change 调查会话已在同代码上实测复现 P0：30 轮 × 1000 行跨进程 INSERT 自 ~11-12 轮起 count 冻结（实测 11015）、dump 静默残缺、文件虚胖、全程 exit 0；单进程 `import --csv` 30000 行完全正常。
- Current-State Evidence（全部为 2026-09-30 实读 + 页级取证，revision 1668c95）：
  - **缺陷 A（空间泄漏）**：`src/storage/page_format/slotted_page.rs` `delete_slot`（约 :249 起）只压缩页尾 slot 数组、减 `slot_count` 并整体回写 header，**`free_space_offset` 永不回退**——死行数据区字节永久泄漏。`SlottedPage` header 16B 布局：page_type(0)、slot_count(1..3 LE u16)、free_space_offset(3..5)、next_page_id(5..9 LE u32)、next_logical_id(9..11)；slot 数组自页尾向下 6B/个（logical_id/offset/length 各 u16 LE）。`add_slot`（:202-246）空间检查失败在**任何变更前**返回 Err（header 未触碰）；成功路径在 :242-243 整体 `serialize` header。
  - **缺陷 B（静默丢弃）**：`src/storage/catalog.rs:736-813` `update_field_in_chain`——沿 `__tables` 链找首个同名行 → `add_slot(new_bytes)` + `delete_slot_by_logical_id(old)`；`add_slot` Err 时走 `Err(_) => return Ok(())`（:793-796，注释自称 "extremely unlikely"）。**页满 = 更新无痕消失**。损坏库尸检：`__tables` 页（page 0）活行 logical_id=**112**（append+delete 112 轮）、fso=4084（仅剩 4B）——约 95-100 次更新即窒息。
  - **触发面**：`update_table_tail`（catalog.rs:361）由 `TableManager::write_tuple`（table_manager.rs:1492-1510）在每次数据页自动分配时调用（:1503-1508）；~1000 行/语句 ≈ 12 次页分配 ≈ 12 次更新/轮。同路径调用方还有：`update_table_column_count`（:339，ALTER）、`update_table_root`（:390）、`update_unique_index_root`（:418）、`update_secondary_index_root`（:457）、`append_secondary_index`（:495，CREATE INDEX 注册——**唯一变长调用方**）。运行期根分裂写回在 `src/storage/btree/index_manager.rs:126-135`；恢复期写回在 `src/wal/recovery.rs:1217-1240`；truncate 写回在 table_manager.rs:1375/1414-1423。
  - **等长事实**：上述调用方除 `append_secondary_index`（及 DROP INDEX 删段）外全部是定宽字段值变化（u32），序列化行长恒等——等长原地写回覆盖全部热路径。
  - **损坏形态实测**：孤儿页段逐轮累积（每轮 12 数据页独立成链）；第 10 轮尾页 332 的 next 被最后一轮覆写为 984（last-writer-wins）；扫描链（head→next）只见头部段+最后一轮；索引树根早期定型故点查/MIN/MAX 完好。`scan_tables`（catalog.rs:263）沿链逐行读、无去重——同名多副本时 `open_or_init` 的 HashMap insert 后者胜。
  - **测试入口**：`tests/schema_persistence_test.rs`（catalog 往返先例）、`tests/checkpoint_redo_reduction_test.rs` / `checkpoint_skip_test.rs`（close/checkpoint 路径）、`tests/secondary_index_test.rs`（22）、`tests/constraint_enforcement_test.rs`（UNIQUE 矩阵）、`tests/drop_table_free_test.rs`。lib 层 open/close/execute 模式见任一集成测试头部。
- Code and Critical Path: 写入路径 `InsertExecutor → TableManager::write_tuple → write_tuple_to_data_page（data_page.rs:11-61，自动分配分支 :34-59 以 raw bytes 写前驱 next[data[5..9]]）→ catalog.update_table_tail → update_field_in_chain`；持久化经 close → `CheckpointManager::checkpoint → BufferPool::flush_all`（无 WAL 参与，catalog 更新不在 WAL——DDL/结构变更直写页）。PageGuard 持 ref_count 防逐出（page_frame.rs `PageGuard::new`/Drop），`modify_page` 置脏。

**Implementation Guidance**

顺序：T1 原语 → T2 重构（消费 T1）→ T3/T4 e2e（消费 T2）。要点：

1. `update_slot_in_place` 复用 `get_slot_by_logical_id` 定位；**等长校验失败返回 Err**；覆写后不触碰 header 字节（不 serialize）——这是「零泄漏」的结构保证。
2. T2 的等长/变长分流以 `new_bytes.len() == old_bytes.len()` 判定；变长分支的旧副本清理需跨页收集同名 `(page, logical_id)` 后删除，保持「调用返回后全链同名恰一份且为新载荷」；`scan_tables` 去重规则 = 同名保留 logical_id 最大者（视图行与表行同名不可能共存，建表已有重名守卫）。
3. 调用方错误传播审计清单（T2 一并核对）：write_tuple（→语句失败，行经事务边界不可见）、truncate（table_manager.rs:1375/1414-1423）、ALTER（:579/:731）、index split 写回（index_manager.rs:126-135）、recovery 写回（recovery.rs:1217-1240）——不得吞错降级为 Ok。
4. `append_to_chain`（catalog.rs:583）的链头维护语义以实际代码为准（`insert_column_row` 消费先例 ：290-296）；契约按行为表达（注册重开仍在），实现细节 Act 核实采用。
5. T3 的行宽用 `STRING(64)`（每轮 ~12 次页分配，15 轮 ≈ 180 次更新 > 窒息阈值 ~95-100，保证修复前 RED 稳定复现）。

**Task Contracts**

### T1: SlottedPage 等长原地更新原语

- Requirement/Scenario: catalog-update-persistence R1（窒息页原地更新成功、副本不增长）
- Depends on: None
- Targets: `src/storage/page_format/slotted_page.rs`（新方法，`impl<'a> SlottedPage<'a>` 内，`add_slot`/`delete_slot` 邻位）+ 同文件 `#[cfg(test)]` 单测
- Current behavior: 无原地更新能力；行更新只能 append+delete（泄漏 + 页满失败）
- Required behavior: `update_slot_in_place(&mut self, logical_id: u16, data: &[u8]) -> Result<(), String>`——按 logical_id 定位 slot；`slot.length != data.len()` → Err；相等则覆写 `page.data[offset..offset+len]`；**header 三计数字段字节不变**；logical_id 不存在 → Err
- Preserve: 既有 `add_slot`/`delete_slot`/`get_slot*` 行为与页格式逐字节不变
- Forbidden: 不改 header 序列化逻辑；不引入压缩/紧凑化；不动 B-tree（0x01/0x02）页路径
- Test witness: 新单测先于实现（编译失败即 RED）——①等长覆写后 `get_slot_data` 读回新字节且旧字节不同；②不等长 Err；③缺失 logical_id Err；④连续 200 次 in-place 更新后 `slot_count`/`free_space_offset`/`next_logical_id` 与首次更新前一致
- GREEN condition: `cargo test --lib slotted_page` 全绿
- Verification: `cargo test --lib slotted_page 2>&1 | tail -5`，0 failed；失败含义 = 原语破坏页格式或定位逻辑
- Stop when: 等长约束无法满足任何现有调用方（与调查「调用方全等长」矛盾时返回 Plan）

### T2: update_field_in_chain 诚实化重构（等长原地 / 变长重定位）

- Requirement/Scenario: catalog-update-persistence R1/R2/R3
- Depends on: T1
- Targets: `src/storage/catalog.rs` `update_field_in_chain`（:736-813）、`scan_tables`（:263 去重）、调用方错误传播面（table_manager.rs / index_manager.rs / recovery.rs 中列于 Investigation Facts 的六处，仅核对传播、不改语义）
- Current behavior: append+delete 单页策略；页满静默 `Ok(())`；每轮更新泄漏行宽字节；同名多副本无去重
- Required behavior: 等长更新 → 原地写回（经 T1 原语），返回后活行值即新值、页计数不变；变长更新 → 链上跨页追加新副本 + 旧副本清理，调用返回后同名恰一份；**删除静默 Ok 分支**——等长校验失败/追加失败/清理失败一律 `Err`（携带表名）；`scan_tables` 同名去重（logical_id 最大者）
- Preserve: 六处调用方对外错误语义（write_tuple 失败→语句失败零副作用；recovery 失败→恢复显式 Err）；`__columns` 链与视图行路径不动；`TABLES_PAGE_ID` 常量头不变
- Forbidden: 不改 `serialize_catalog_row` 行格式；不动 WAL/checkpoint；不为失败路径新增兜底重试
- Test witness: catalog 单测（RED 先行）——①200 次 `update_table_tail` 循环后页 slot_count/fso 恒定且重开读回末值（现状 RED：重开读回陈旧值）；②窒息页（预填至仅剩 <行宽空间）上等长更新成功（现状 RED：静默丢失）；③CREATE INDEX 增段在页满时跨页持久（重开索引在）；④手工构造同名双副本 → scan_tables 只返回 logical_id 大者；⑤页满且链满时变长更新 Err 点名表名
- GREEN condition: `cargo test --lib catalog` 全绿
- Verification: `cargo test --lib catalog 2>&1 | tail -5`，0 failed；失败含义 = 持久化仍丢失或错误语义未诚实化
- Stop when: 变长重定位需改动 `__tables` 页格式或发现既有调用方存在非等长热路径（与调查矛盾）时返回 Plan

### T3: 跨进程 durability 回归 e2e

- Requirement/Scenario: catalog-update-persistence R4/R6
- Depends on: T2
- Targets: 新文件 `tests/catalog_durability_test.rs`
- Current behavior: 15 轮 lib（`Database::open → execute INSERT 1000 行单语句 → close`，行含 `STRING(64)` 列）自 ~8-12 轮起 count 冻结（本 change 调查会话同形态实测 11015/30000）
- Required behavior: 每轮后 `COUNT(*)` 恰等于轮数 × 1000；终轮 dump 行数 = 15000；首(1)/中(7500)/尾(15000) 点查全命中；CLI 形态抽样 12 轮（独立进程，`RTSQL_HOME` 指向 tempdir）终值 12000；单进程 import 30000 行对照保持 30000
- Preserve: `STRING(64)` 行宽（保证修复前 RED 稳定）；每轮 INSERT 为单语句独立 auto-commit 会话
- Forbidden: 不为加速而缩短轮数至阈值以下；不引入专用判定工具（断言用既有 SQL 结果与断言宏）
- Test witness: 修复前运行本文件 → lib 15 轮用例 RED（冻结值 < 15000，记录冻结轮次与值）；实现后同用例 GREEN
- GREEN condition: `cargo test --test catalog_durability_test` 全绿
- Verification: `cargo test --test catalog_durability_test 2>&1 | tail -8`，0 failed；失败含义 = durability 仍未闭合或 e2e 夹具与真实路径脱节
- Stop when: 修复后仍出现轮间冻结（根因判断错误）时返回 Plan

### T4: 索引根持久化专项

- Requirement/Scenario: catalog-update-persistence R5
- Depends on: T2
- Targets: `tests/catalog_durability_test.rs`（追加用例组）
- Current behavior: 根分裂的 catalog 写回走同一静默路径（index_manager.rs:126-135）；窒息后分裂不持久（重开树退化/缺失）
- Required behavior: ①UNIQUE 列插入 ~5000 行促唯一树多次分裂 → close/reopen → 重插已存在值 `DuplicateKey` 拒绝、新值正常；②建二级索引 → 插入 ~5000 行促分裂 → close/reopen → 等值查询结果与全扫描一致且计划路由索引；③重开后 `DROP INDEX` 成功
- Preserve: 分裂/写回的运行期行为不变（仅持久化通道经 T2 修复）
- Forbidden: 不调整 B-tree 分裂阈值或页格式
- Test witness: 新用例；现状（T2 前）预期 RED（重开唯一约束失效或索引路由失效——若现状恰未触发分裂则记录实测并按实际阈值加大行数，仍以「重开保持」为断言）
- GREEN condition: 同文件全绿
- Verification: `cargo test --test catalog_durability_test 2>&1 | tail -8`；失败含义 = 根写回仍有静默丢失
- Stop when: 分裂写回在同步上下文无法异步传播错误且无法在 T2 契约内解决时返回 Plan

**Invariants**

- 页格式（4096B / 16B header / 6B slot）与 `serialize_catalog_row` 行格式逐字节不变。
- lib `Database::open` 建库行为、WAL/checkpoint/恢复链路（含 MS18 零工作跳过）不变。
- `PageGuard` ref_count 防逐出与 `modify_page` 置脏语义不变。
- 全部验证走被测对象原生退出码与输出；不新增判定层、身份型证据或专用工具。

**Non-goals**

- 既有损坏库孤儿段的重链/回收与修复工具；slotted page 全量紧凑化；catalog 页分裂能力扩展；P1/P2/P3 面（后继 Iteration）。

**Acceptance**

R1：窒息页原地更新成功 + 200 次循环零泄漏 + 重开持久（T1/T2 单测）。
R2：页满非等长 Err 点名 + tail 推进失败零副作用（T2 单测 + 调用方传播核对）。
R3：CREATE INDEX 跨页持久 + 双副本确定性读（T2 单测）。
R4：15 轮 lib + 12 轮 CLI 行数单调、五面一致（T3，RED→GREEN）。
R5：UNIQUE/二级索引分裂重开保持（T4）。
R6：全量分批（M19）0 failures，白名单外零校准。

**Verification**

- R1/R2/R3：`cargo test --lib slotted_page catalog`（0 failed；RED 先行记录）。
- R4/R5：`cargo test --test catalog_durability_test`（修复前 RED 记录冻结值，修复后 0 failed）。
- R6：全量分批（100 集成目标 ×N 批 + lib），0 failures。
- 人工步骤：无（全部自动判定）。

**Gate 2 Readiness**

全部 PASS（2026-09-30 用户批准计划：原文「批准」）——调查、设计、任务契约、验证与分轮就绪；无实质未知项（`append_to_chain` 链头维护语义与 index split 写回 await 形态为非实质局部选择，已给行为级契约）；Persisted Evidence = none；无 BLOCKED / WAIVED 项。proposal 默认假设 9 条随计划一并获批。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- 窒息阈值随行宽/表名宽度浮动（~95-100 次更新）；测试已取余量（200 循环 / 15 轮）。若 RED 未在预期轮次出现，按实际冻结值记录，不影响 GREEN 判据。
- index split 写回（index_manager.rs:126-135）若处于无法传播 async 错误的上下文，按 D3 语义需要在调用侧处理——Act 核实；若超出「仅核对传播」范围即 Stop 返回 Plan。
- 变长更新的追加+清理两步在进程内原子（catalog Mutex），跨进程崩溃窗口由 scan_tables 去重兜底；该窗口为 design D2 已接受风险。
- 既有损坏库（用户现存 3 个）在本修复后 tail 更新恢复工作，但孤儿段不可自动找回——已在 proposal 假设 3 声明。

## Act Response

- Status: reported
- Cycle: Iteration 000 / Cycle 000（initial）
- Implemented on: master @ `1668c95` 之上（工作区未提交）

**Implemented**

T1 `src/storage/page_format/slotted_page.rs`

- 新增 `SlottedPage::update_slot_in_place(&mut self, logical_id: u16, data: &[u8]) -> Result<(), String>`（`add_slot`/`delete_slot_by_logical_id` 邻位）：按 logical_id 定位 slot → 等长校验（不等长 `Err`）→ 覆写数据区；**不重写 header**，故 `slot_count`/`free_space_offset`/`next_logical_id` 与页内 slot 位置全部不变，数据区耗尽时仍可写入。
- 新增 5 个单测：等长覆写字节级断言、不等长拒绝（短/长双向且原字节不变）、logical_id 缺失拒绝、header 16 字节原始字节不变、200 次连续更新三计数字段与 `free_space` 恒定。

T2 `src/storage/catalog.rs`

- `update_field_in_chain` 拆为「定位 + 写回」两趟：等长分支走 `update_slot_in_place`；变长分支走 `append_to_chain` 追加后由新 helper `delete_named_copies_except` 清理链上同名旧副本（按 `(page_id, logical_id)` 精确跳过刚追加的那一份，同页旧副本一并清除），调用返回后同名恰一份。**页满静默 `Ok(())` 分支已删除**，全部失败路径 `StorageError::Internal` 携表名传播。
- `append_to_chain` 返回值 `Result<()>` → `Result<(PageId, u16)>`（3 处调用方 `?` 后丢弃，语义不变）。
- `scan_chain` 返回 `Vec<T>` → `Vec<(u16, T)>`；`scan_tables` 经新 helper `collapse_newest_same_name` 同名去重（保留 logical_id 最大者、保持首现顺序），`scan_columns` 丢弃 logical_id 字段，行为不变。
- 新增 5 个单测 + 3 个测试夹具 helper（`page_space`/`suffocate_page`/`reopen_tables`/`filler_row`/`live_row`）：200 次更新零泄漏且重开读回末值、窒息页等长更新落盘、页满时变长更新跨页持久、双副本去重、变长更新失败点名表名且零副作用。

T3 / T4 `tests/catalog_durability_test.rs`（新增，4 个用例）

- `lib_sessions_keep_every_committed_row_visible`：15 轮 `open → 单语句 INSERT 1000 行 → close`，每轮用**新会话**断言 `COUNT(*)` 恰为轮数×1000；终态五面一致（`COUNT(*)`=15000、`MIN/MAX(id)`=1/15000、全键空间范围扫描=15000、首/中/尾点查全命中、`dump` 导出 `INSERT INTO` 行数=15000）。
- `cli_sessions_keep_every_committed_row_visible`：`RTSQL_HOME` 指向 tempdir（预建 `db/`）、裸名实参、stdout/stderr 落文件（避免管道阻塞）、120s 死线的独立 CLI 进程 12 轮，终值 12000。
- `single_process_bulk_import_stays_correct`：单进程 `rtsql import --csv` 5000 行对照。
- `index_root_write_backs_survive_reopen`：两段写入会话（4000 + 8000 行）分别覆盖 UNIQUE 树与二级索引树 → 重开后 UNIQUE 值重插被拒（新值正常）、`EXPLAIN` 仍路由 `IndexScanAll`、索引路由与 `code + 0` 扫描路由结果逐值一致、`DROP INDEX` 可达。

**Deviations from Plan**

1. **删除 `append_to_chain` 末尾的 `bp.free_page(new_id)`（计划外，R3 达成必需）**：该调用逐出新页且不落盘，并 `storage.free_page` 把页号推入存储 free list，跨页追加的 catalog 行随即丢失。首次 GREEN 尝试时 `append_secondary_index_on_full_page_relocates_and_persists` 直接报 `row must exist`。删除后新用例与既有 `secondary_index_registration_lifecycle_roundtrip` 同时通过；新页留在缓存中为 dirty，交由既有 flush 路径落盘。
2. `scan_chain` / `append_to_chain` 签名变化（见上），私有函数，3 + 2 处调用方语义不变。
3. **T3 import 对照行数 30000 → 5000**：实测 `rtsql import` 2000 行耗时 8.246s（≈4.1 ms/行，逐行 auto-commit + `sync_all`），30000 行 ≈123s，超过 one-shot CLI 120s 死线。5000 行仍跨 ~120 次 catalog tail 更新（修复前页窒息阈值量级），对照语义不变。
4. **T4 断言形态调整（按契约「若现状恰未触发分裂则记录实测」回退分支）**：root 页 id 在首次分裂后再移动需 ~10⁵ 条目量级，实测 5000 行与 30000 行两种单段形态在修复前均保持 GREEN（root 位移发生在会话早期，catalog 页窒息在数千行之后），因此无法用可行行数造出 RED。改为两段写入会话（更贴近「页已窒息后仍需 root 写回」的真实条件）并以「重开保持」作为 R5 断言。
5. 测试夹具两处修正：窒息页填充行必须是**合法 catalog 行**（filler 表名，非 0xEE 垃圾字节，否则 `scan_chain` 解析报 `catalog row too short`）；`dump` 目标用绝对路径（库不在 CLI home 下，裸名会解析到 `$RTSQL_HOME/db/`）。
6. 补充 R4「五面一致」缺失的两面：范围扫描与 `MIN/MAX` 断言（T3 初始实现只有 count/点查/dump）。

**Self-Review Findings**

- **Critical（已修复）**：变长更新的旧副本清理最初按「整页跳过」实现（`page_id != keep.0`），同页旧副本因此残留，第二次追加时从链首读到的是陈旧副本 → 既有用例 `secondary_index_registration_lifecycle_roundtrip` RED（期望 2 个二级索引，实得 1）。改为按 `(page_id, logical_id)` 精确跳过后全绿。
- **Important（已修复）**：clippy `type_complexity` 2 处（`Vec<(u16, Vec<u8>)>` / `Option<(u16, usize, Vec<u8>)>`）→ 提取 `PageRows` / `LocatedUpdate` type alias，`cargo clippy --all-targets` 现无 lib 告警。
- **Minor（遗留）**：`collapse_newest_same_name` 的 tie-break 采用 logical_id 最大者，而 `next_logical_id` 是**页内**计数器。变长更新若把新副本追加到新页（lid 从 0 重启）且随后的清理步骤未完成，崩溃窗口内去重会选中较旧副本。仅影响 CREATE/DROP INDEX 的注册面（tail/root/unique root/column_count 全部走等长原地路径，不产生副本）；design D2 已声明接受该崩溃窗口。是否改为「链序最后一份」tie-break 请 Plan Review 决定。
- 既有 `delete_from_chain` 的 `let _ = delete_slot_by_logical_id` 不在本 Cycle 契约内，未改动。

**Verification Evidence**

1. `cargo test --lib -- --test-threads=1` → `test result: ok. 350 passed; 0 failed`（基线 340 + 新增 10），exit 0。
2. `cargo test --test catalog_durability_test -- --test-threads=1` → `4 passed; 0 failed`，40.18s，exit 0。
3. **修复前 RED 见证**（`git checkout -- src/storage/catalog.rs src/storage/page_format/slotted_page.rs` 回到 `1668c95` 后运行，还原以 `md5sum -c` 校验通过）：
   - `cargo test --lib catalog` → 5 个新用例 FAILED。`fixed-width updates must not grow slot_count / free_space_offset: left (1, 4056), right (1, 56)`（200 次更新泄漏 4000 字节）；窒息页用例 `left: 7, right: 777`（更新被静默丢弃）；双副本用例 `left: 2, right: 1`。
   - `cargo test --test catalog_durability_test` → `lib_sessions...` 于第 5 轮 RED：`round 5: a fresh session must see all 5000 committed rows, left: 4822, right: 5000`；`cli_sessions...` RED：`left: 4822, right: 12000`；两个对照用例在修复前 GREEN（证明对照有效）。exit 1。
4. 受影响边界（`schema_persistence` 8 / `checkpoint_redo_reduction` 9 / `checkpoint_skip` 6 / `drop_table_free` 6 / `truncate` 7 / `secondary_index` 22）全部 `0 failed`，已含于第 5 项，exit 0。
5. **全量分批回归（M19：101 目标 + lib/bins，每目标 `cargo test --test <name> -- --test-threads=1`，每批 10 个顺序执行，无默认并行、无单次无过滤全量调用）** → `=== TOTAL: passed=1254 failed=0 ignored=2 ===`、`FULL_REGRESSION_FAILED=0`，exit 0。叠加第 1 项 lib 350 → 合计 1604 = 变更前基线 1590 + 本次新增 14 例（5 + 5 + 4）。
6. `cargo fmt --check` → 0 diff（`cargo fmt` 仅触及本次三个文件，`git diff --stat` 确认无外溢）；`cargo clippy --all-targets` → lib 无告警。
7. `openspec validate 2026-09-30-catalog-durability-and-cli-surface --strict` → `is valid`，exit 0。
8. 六处调用方错误传播审计（只核对，未改语义）：`table_manager.rs:579`、`:731`、`:1079`、`:1146`、`:1375`、`:1414-1423`、`:1506`（`write_tuple`）、`index_manager.rs:126-135`（`sync_root_to_catalog`）、`recovery.rs:1217-1240` —— 全部 `?` / `map_err` 上抛，无吞错。
9. 采信说明：第 5 项全量批次运行时 `tests/catalog_durability_test.rs` 尚未加入五面断言；该文件随后仅追加断言并经第 2 项覆盖（40.18s 那次），其余 100 个目标不受该文件变更影响。全量批次之后仅对该测试文件执行过一次 `cargo fmt`（纯格式，无行为变化），采信第 2 项结论。

**Persisted Evidence**

None required —— 全部决定性输出短（均 ≤ 20 行）且可用上述命令低成本复现；修复前 RED 见证的命令、回退方式与决定性输出已记录在本节，未创建 `evidence/` 目录。

**Experience Candidates**

1. **Runbook 候选**：实现完成后补取「修复前 RED 见证」的可复现流程 —— 备份改动文件 → `git checkout -- <paths>` 回到基线 revision → 运行集成测试取 RED → 从备份还原 → `md5sum -c` 校验还原。本 Cycle 端到端验证两次（`catalog.rs` + `slotted_page.rs`）。证据：本节第 3 项。
2. **Issue 候选**：`rtsql import` 逐行 auto-commit 吞吐 ≈ 4.1 ms/行（2000 行 8.246s），30000 行 ≈123s，超出 one-shot CLI 合理时延。change 范围外（属 MS22 性能域）。证据：`rtsql import bench people bulk.csv --csv` 实测 `real 0m8.246s`。

**Unresolved**

- Minor：`collapse_newest_same_name` 的 tie-break 语义（见 Self-Review），待 Plan Review 裁定。
- `append_to_chain` 的 `free_page` 修正尚未反映在 delta spec R2/R3 措辞中（spec 归 Plan 维护）。
- 用户现存 3 个损坏库在本修复后 tail 更新恢复工作、孤儿段不可自动找回（proposal 假设 3）；本 change 不提供修复工具。
- change `tasks.md` 无执行状态字段（任务表 + RTM `Status: Covered` 为计划覆盖度语义），Iteration 完成状态由本 Cycle 文件承载，故未修改 change tasks。

## Plan Review

- Review Result: accepted

**Findings**

独立检查（Plan 独立阅读完整 diff：`src/storage/page_format/slotted_page.rs` +114、`src/storage/catalog.rs` +472/-57、`tests/catalog_durability_test.rs` 新增 4 用例；逐一核对 Act Response 声明与代码一致）：

1. **T1 契约逐项吻合**：`update_slot_in_place` 等长校验/缺失拒绝/覆写不触 header 与注释声明一致；5 个单测覆盖契约矩阵（含 200 次更新三计数字段恒定）。
2. **T2 重构正确**：两趟定位/写回分流清晰；等长分支经 T1 原语 + `StorageError::Internal` 携表名；变长分支 `append_to_chain` + `delete_named_copies_except` 按 `(page_id, logical_id)` 精确保留（Self-Review Critical 修复属实——同页旧副本残留会导致既有 `secondary_index_registration_lifecycle_roundtrip` 读到陈旧副本）；清理失败诚实 `Err`；`scan_columns` 行为不变（map 丢弃 lid）。六处调用方传播审计（Act Evidence 8）与 diff 一致——本 Cycle 契约本就限定「仅核对传播、不改语义」，未改动调用方正确。
3. **偏差 1（`free_page` 删除）裁定为正确且必要**：旧 `append_to_chain` 在跨页追加后将刚写入的脏页逐出缓存（不落盘）并把页号推入 storage free list——跨页 catalog 追加（CREATE INDEX 注册 / ALTER ADD COLUMN 的 `__columns` 追加同路径）本就存在静默丢失。该删除是 R3（变长跨页持久）达成的必要条件，同时修复一处计划外发现的同族既有缺陷；新页留缓存交由既有 flush 路径持久化的替代实现正确（全量回归含 `drop_table_free` 6 例佐证 free-list 语义未破坏）。
4. **偏差 3/4（import 5000 行、T4 两段会话）在契约回退分支内**：import 时延实测（4.1 ms/行）支撑行数调整，对照语义（跨窒息阈值量级）保持；T4 按「现状未触发分裂则记录实测」回退分支执行，以「重开保持」作 R5 断言，符合契约原文。
5. **Minor 1（遗留，裁定：接受并记录，不强制返工）**：`collapse_newest_same_name` 以 logical_id 最大者为存活副本，而 logical_id 为页内计数——变长更新跨页追加（新页 lid 回绕）且清理步骤被进程中断时，去重可能选中较旧副本。该窗口仅在「design D2 已接受的崩溃窗口」内开启且下次成功更新自愈；按公共规则「非阻塞 Minor 不创建修复」处理。已同步修正 delta spec R3 场景措辞（移除「即最新」的时序断言，改为确定性收口表述）；`scan_tables` 文档注释中 "an append always allocates a greater logical_id" 的绝对化表述随之失准，留待该文件下次触碰时顺带修正（登记为候选，不另立任务）。
6. **Minor 2（工作区卫生，已处理）**：仓库根 `db/`（reproC/reproD 手工复现产物，非测试输出——测试夹具用 tempdir + `current_dir(home)` 隔离，已核实）为 Act 手工验证孤儿。Plan Review 已核验无引用后删除并在本节记录。
7. **偏差 5/6、Self-Review Important（clippy type_complexity → type alias）**：核实为测试夹具与 lint 修正，无行为面。
8. 验证采信：Act 全量分批（M19，1254 集成 + 350 lib = 1604，0 failed）、RED 双向见证（lib 用例第 5 轮 `4822 vs 5000`、窒息页 `7 vs 777`、泄漏 `4000 字节`）、fmt/clippy/validate 均为本会话新鲜运行，覆盖未变化，直接采信；Act Evidence 9 的采信说明（五面断言后补、由专项运行覆盖）合理。Plan 未重跑，独立核对以 diff 与声明一致性完成。

**Acceptance Gaps**

无——R1-R6 全部满足（RED→GREEN 见证齐全，全量零回归，白名单外零校准）。

**Evidence**

- 完整 diff 独立审读（本节 Findings 1-2、7）；`git status` 确认产品代码改动面与契约一致（仅两存储文件 + 一个新测试文件）。
- Act Response「Verification Evidence」1-9（新鲜运行，采信）。
- Minor 2 处置：`db/` 已删（未跟踪、无引用）；delta spec R3 场景措辞已由 Plan 修正（本 change `specs/catalog-update-persistence/spec.md`）。

**Follow-up Decision**

接受（`accepted`）：全部 Acceptance 满足，两条 Minor 均不阻塞且已有处置（记录 + 措辞修正 + 孤儿清理）。Act 报告的 Issue 候选（`rtsql import` 逐行 auto-commit ≈4.1 ms/行，属 MS22 性能域）与 Runbook 候选（修复前 RED 见证的备份-回退-还原流程）只报告不落账，由用户指令 Recorder 处置。Iteration 001 按既有 Map 展开。

**Next Cycle**

None

**Next Iteration**

`iterations/001-table-alias-resolution/000-initial.md`
