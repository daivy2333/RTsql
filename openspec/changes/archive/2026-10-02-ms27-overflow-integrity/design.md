# Design — MS27 存储演进：超页元组溢出页链 + 数据完整性校验

## Current-State Evidence（调查结论，2026-10-02 本 change Phase 2 实测代码阅读）

表示层与页格式：

- `Slot` 6B（logical_id/offset/length 各 u16，`slotted_page.rs:5-13`）；`SlottedPageHeader` 16B（page_type/slot_count/free_space_offset/next_page_id@5..9/next_logical_id，`:16-24`）。页类型现行 0x01 Leaf / 0x02 Internal / 0x03 Data。
- `add_slot_inner`（`slotted_page.rs:225-274`）容量检查 `free_space_start + 6 + data_len > 4096 - slot_count*6` → `"No enough space in page"`；`length: data_len as u16`。内联上限 = 4096 − 16 − 6 = **4074B**（`VersionHeader` 22B + tuple ≤ 4052B）。
- tuple 序列化 tag 域 0x01–0x07（`tuple.rs:21-29`），**String 长度前缀 u16 且 `as u16` 静默回绕**（`tuple.rs:88-97`）——>65535B 值在表示层不可表示（潜在坏数据，本 change 以新 tag 一并修复表示面）；`deserialize_*` 对未知 tag 显式拒绝（`unknown tag byte`）；`deserialize_value_refs` 零拷贝变体同域。`deserialize_unknown_tag_rejected` 测试（`tuple.rs:780-784`）锚定 0x08 为未知 tag——启用 0x08 后需校准为其他未用字节。
- `ColumnType::String(u16)` 声明 max_len；insert/update 写路径不校验值长 ≤ max_len（既有语义，本 change 不改变）。

写读通道：

- 写：`write_tuple_to_data_page`（`data_page.rs:11-61`）——tail 页 `add_slot(slot_data=vh++tuple)`；失败→`allocate_page()` 新页→旧 tail `data[5..9]` 挂链→新页 `add_slot`，再失败映射 `StorageError::PageFull`（此时新页已挂链不回收——既有残留形态）。页满分配路径对 >4074B 必然走到 PageFull。
- 读（闭包零拷贝，页锁内禁止 await/递归 BufferPool）：
  - `read_tuple_from_data_page`（`data_page.rs:73-100`）`with_page_data` 内 `f(VersionHeader, tuple_bytes)`；
  - `DataScanExecutor::next`（`data_scan.rs:427-503`）`with_page_data` 状态机 `PageAction::{YieldValue,SkipSlot,JumpToPage,Done}`，物化 `deserialize_row` 在页闭包内；
  - `BufferPool::find_visible_version`（`buffer_pool.rs:281-375`）内部枚举 `VisibilityResult::{Visible,NotVisible,NotFound}` 模式，可见版本在页闭包内消费闭包 f。
  - 三处的 VersionHeader 均在 slot 数据前 22B 内联读取——**版本链遍历只消费 header**。
- WAL：`Insert{tuple_data}` / `Update{old_tuple,new_tuple}`（`record.rs:53-87`，帧 `[lsn8][type1][len4][body][crc4]`，len u32）；恢复 `redo_tuple_at_row_id`（`recovery.rs:136-213`）按 `add_slot_with_logical_id` 强制落位、slot 已存在幂等跳过、prev 页挂链；`build_pk_version_maps` keyless 桶按 tuple 原始字节追踪；`rebuild_pk_indexes`→`extract_index_keys` 消费 `slot_data[22..]`；`derive_old_row_id` tuple 逐字节校验。
- 生命周期：`delete_slot` 只压缩 slot 目录不回收数据区（无页内 compaction）；GC 唯一实现 `TableMeta::gc_table`（`table_manager.rs:125-154`）**无生产调用者**（仅测试）；`abort_cleanup_versions` **从不删 slot**（仅 `mark_aborted` 改 22B 头）；`drop_table`/`truncate_table` 经 `collect_data_pages`（沿 `next_page_id` 链）逐页 `free_page`，best-effort。
- 存储：`AsyncStorage::{read_page,write_page,allocate_page,free_page,page_count}`；FileStorage free-list **进程内存态不持久化**（freed 页盘上为全零页）；MemoryStorage 同型（LIFO free-list + 单调 page id）。加密页 4124B（nonce12+ct4096+tag16，`crypto.rs:17`）、**AAD=page_id**、解密失败统一 `InvalidKey("decryption failed (wrong key or corrupted page): page N")`（`file_storage.rs:325-337`）；页 N 偏移 `64 + N*record_size`。
- 完整性现状：明文数据页零校验；页头解析零校验；`get_slot_data` 越界 slice **可 panic**（`slotted_page.rs:125-129/195-199`）；catalog 行损坏阻断 open；WAL 帧 CRC 存在；加密页损坏经 GCM tag 检出但与错钥同文案。可复用遍历：`data_chain_status`、`collect_all_pages[_tolerant]`、`scan_tables/scan_views/scan_columns`。
- CLI：`Command` 枚举 `cli/mod.rs:111-190`；`lifecycle_db_target` `:memory:` 守卫；`resolve_existing_db` 缺库 exit 1；`execute_command_inner_mode` readonly 臂 `:543-552`（`open_readonly_with_lock_wait`，锁预算/key 穿线）；`open_readonly` 对 pending WAL 经 `assert_no_pending_wal`（`database.rs:37-54`，私有）拒绝（"open read-write once to recover"）；退出码 `ExitStatus`（0/1/2/3/4/5/128+n）。测试 `CARGO_BIN_EXE_rtsql` 真二进制 + `RTSQL_HOME` TempDir。文档同步点：README.md/README.zh-CN.md 命令表与退出码段、rtsql-docs/SKILL.md Lifecycle 表。

## 目标行为

R1–R7 见 proposal。核心：>4074B slot_data（22B VersionHeader + tuple）经溢出页链存取，SQL 层透明，全写通道、两态恢复、生命周期释放；数据页损坏可检出并报告位置（`rtsql check`）。

## 关键决策

### D1 溢出表示 = locator 模式（VersionHeader 恒内联）

主页 slot 恒存 `[VersionHeader 22B][locator]`；locator = `[0xFF marker][total_len u32][first_overflow_page u32]`（9B，slot.length=31）。理由：

- 版本链遍历（可见性判定、superseded map、abort 标记、commit 传播）只读 header——全部零感知、零改动；
- slot 目录 6B 定长不变，`add_slot/delete_slot/compaction` 全复用；
- 0xFF 在 tuple tag 域（0x01–0x08）之外，内联 payload 首字节恒为 tag 域值，判定无歧义；
- 整行外移方案（header 也在溢出页）会使版本链遍历跨页 await，否决。

### D2 长字符串表示 = 增量新 tag 0x08（u32 长度前缀）

`TAG_LONG_STRING 0x08 [u32 len][bytes]`；序列化仅当 `s.len() > 65535` 使用，≤65535 逐字节维持 `TAG_STRING`（存量数据与既有单页行零变化）；deserialization 三个入口（`deserialize_tuple_width`/`deserialize_value_refs`/catalog DEFAULT 段复用同 tag 域）全部接受两 tag。`compute_tuple_size` 按同公式。旧行二进制读新库：0x08 → 既有 `unknown tag byte` 诚实拒绝（向前不承诺）。顺带消除 `as u16` 静默回绕（>65535 值从「写坏」变为「正确表示」）。测试校准：`deserialize_unknown_tag_rejected` 的 0x08 字面量改为未用字节（如 0x7F），断言语义不变。

### D3 溢出页格式 = 复用 SlottedPageHeader 布局，page_type 0x04

每页：`[SlottedPageHeader 16B]{page_type=0x04, next_page_id=链后继或 0, free_space_offset=本页 payload 字节数, 其余 0}` + payload 区 `[16..16+payload_len)`。chunk 满 4080B/页，末页余量。自描述（每页自带 payload_len，check 可校验）；页头解析器复用；page_type 0x04 为新分类（check 页 census 识别，既有 btree `collect_all_pages` 只从 catalog 根遍历不受扰）。locator 的 total_len 与链上 payload_len 之和一致性在装配时校验。

### D4 内联阈值 4074 与零回归边界

`slot_data.len() ≤ 4074` → 既有路径逐字节不变（含错误面）；`> 4074` → 直接走溢出分支，**不再先分配空数据页**（既有「分配新页挂链后 add_slot 二次失败留空页」的残留形态仅对原本必失败的 >4074 写入消除，≤4074 路径不动）。写序：分配溢出链页→写 chunk→**最后**落 locator slot（locator 未落位前崩溃 = 链页 unreferenced，无 dangling 引用）；任一步失败 → 释放本次已分配链页后冒错（写入失败不泄漏）。

### D5 生命周期语义（与既有版本模型对齐）

- **abort/回滚不删 slot**（既有语义）：回滚的超页版本 slot 保留（aborted 头），其溢出链保持可达，随 slot 存续——非泄漏；GC 删除该 slot 时一并释放链。
- 释放点：`delete_tuple_from_data_page`（GC 消费）先删 slot 再释放链（先链后 slot 会引入 dangling 窗口）；`drop_table`/`truncate_table` 先收集全部 slot 的溢出链页再随数据页释放（best-effort，同既有释放语义）。
- UPDATE 增长/缩回 = 新版本 slot（溢出/内联各自形态），旧版本链随旧 slot 生命周期——MVCC 语义零变化。
- 已知边界：free-list 本就不持久化，崩溃窗口的半写链页与恢复重分配旧链页均成为 unreferenced 页（盘上全零或孤儿非零）——不构成可达数据损坏，`check` 将其报告为 unreferenced（信息级）。

### D6 恢复 = 重放期重分配链页 + locator 末步落位幂等

`redo_tuple_at_row_id` 增加溢出分支：slot 不存在 → 按记录 tuple_bytes 分配新链页写 chunk → 末步 `add_slot_with_logical_id` 落 locator；slot 已存在 → 既有幂等跳过（locator 已在，不重写链）。崩溃期运行时已分配的旧链页不被引用（locator 若已落位则 slot 存在、跳过；未落位则旧链 unreferenced）——收敛不依赖「重放页分配 == 运行期页分配」，与 R10 语义同型。恢复期分配新页有既有先例（Delete 墓碑经 `write_tuple_to_data_page` 尾页分配）。`build_pk_version_maps` keyless 桶、`rebuild_pk_indexes`/`extract_index_keys`、`derive_old_row_id` 的 tuple 消费点全部改为经装配入口取完整 tuple 字节（0xFF locator → 装配；内联 → 原路径）。

### D7 T02 方案定稿 = `rtsql check` 全页结构扫描命令（不采页校验和）

推荐理由（相对页校验和）：

1. **零格式演进**：页校验和需在每页存 4B——页内布局、页大小、加密记录布局（GCM 已占满认证域）任一改动都是全局格式迁移；check 命令对全部存量库文件立即可用（含本 change 前创建的库）。
2. **加密库已有页级完整性**：AES-GCM tag 对每页认证（损坏→`decryption failed`）；页校验和对其冗余。明文库的读时检测收益有限（未读字节的静默损坏仍需扫描才能定位）。
3. **语义级检出更深**：扫描不仅验证字节（tuple 逐行反序列化、slot 边界、链一致性），还验证结构（catalog 行可解析、数据链终止于 catalog tail、索引树可达无洞、溢出链 total 一致）——校验和只能发现「字节变了」，不能发现「指针断了」。
4. **写路径零成本**：无每页 CRC 计算与格式分支。

check 范围（只读）：文件头（open 既有校验）→ 页 census（0..page_count：全零页/0x01–0x04/未知类型）→ catalog（`scan_tables/scan_views/scan_columns` 解析错误、链环）→ 每表数据链（`data_chain_status` + 逐 slot bounds-checked 迭代 + tuple 反序列化 + 溢出链跟随校验）→ 每表三类索引树（`collect_all_pages` 严格模式：节点类型/洞）→ 交叉引用（数据链/树/溢出链可达页集合 ∪ catalog 固定页 vs 非零未引用页 = unreferenced 报告，信息级）。逐问题一行 `error:/warning:` 报告（含页号与语义），末行汇总；干净库 exit 0，发现问题 exit 1。不修复、不写盘。

### D8 check 的 panic 安全 = 命令内 bounds-checked 迭代

`get_slot_data` 对损坏 offset/length 会越界 panic（既有事实）。check 自带边界校验迭代（`offset+length ≤ 4096`、`free_space_offset ≤ 4096`、slot 区/数据区不重叠等）后再解析，**不修改** `get_slot_data` 及生产读路径（改动面隔离）。生产读路径对损坏元数据的 panic 面为本 change 范围外既有缺陷，作为 Issue 候选在交付时报告。

### D9 check CLI 语义（对齐 checkpoint/delete 先例）

`rtsql check <db>`：`lifecycle_db_target` 增 Check 臂（`:memory:` exit 2）+ 既有 `--readonly`×子命令 exit 2 守卫自动生效；`resolve_existing_db` 缺库 exit 1；经 `execute_command_inner` readonly 臂开库（共享锁 + 锁预算：冲突 exit 4；错钥 exit 5；pending WAL → open 失败 exit 1，错误文案已含 "open read-write once to recover" 指引）；`--format` 接受并忽略（checkpoint 先例）。退出码：0 = 无问题；1 = 发现问题或开库失败。文档同步：README 双语命令表 + SKILL.md Lifecycle 表 + Behavioral notes。

### D10 大记录承载面确认

WAL Update 记录含新旧两份完整 tuple（1MB 行 → ~2MB 帧），帧 len u32 与 `WalWriter` 批写无上限约束；checkpoint 重写与 buffer 逐页刷盘按页处理，无整帧驻留约束。CLI：主命令 SQL 参数（Linux ARG_MAX ~2MB）与 import CSV 字段均可承载 1MB 值；`dump` 输出 1MB SQL 字面量、restore 回读对称。JSON/table 渲染照常。不做分帧或压缩（Non-goal）。

### D11 catalog DEFAULT 段长字符串语义 = 与 tuple 表示对齐（2026-10-02 replan 并入）

Iteration 000 Review 发现：catalog 列行 DEFAULT 尾段（MS24 Iter000 引入，`u8 has_default | tag | payload`）有独立序列化器，String 恒 `[0x02][u16 len]`（`serialize_catalog_column_row` String 臂 `s.len() as u16`，catalog.rs），未随 T1 获得 0x08 长形式语义——>65535B DEFAULT 字面量长度前缀回绕，反序列化对 0x08 报 `unknown default value tag`。

**可达性分析（调查确证）**：该回绕在生产路径结构不可达——`insert_column_row` 经 `append_to_chain` 页内落位，catalog 列行容量 = 页内联上限 ~4074B（catalog 行溢出为本 change Non-goal），>65535B DEFAULT 字面量会先在链追加时页满失败，到不了 String 臂。因此本项定位**不是用户可见行为修复，而是表示一致性硬化**：消除两套格式间的语义分歧，避免 catalog 存储将来演进（如引入溢出）时静默继承回绕陷阱。

**选择**：序列化 >65535B String DEFAULT 走 `[0x08][u32 len LE][bytes]`（≤65535 逐字节不变，存量 catalog 文件零迁移）；反序列化增 0x08 臂（截断/坏 UTF-8 诚实拒绝）。复用 tuple.rs 的 `TAG_LONG_STRING` 常量语义，不引入新 tag。**替代方案否决**：(a) 仅登记 Issue 不改——用户裁定并入修复；(b) DEFAULT 段改用 tuple.rs 序列化器整段复用——DEFAULT 段是列行内嵌的 `tag|payload` 碎片而非完整元组，改载体是无谓的格式迁移；(c) 同时解除 catalog 行页内容量约束——违反已批准 Non-goal（列定义行保持页内），DEFAULT 字面量的真实长度边界（~4KB 量级、超限建表按既有页满错误拒绝）随 T8 文档化。

**测试见证层级**：序列化/反序列化为 `pub(crate)` 纯函数（既有行级往返单测先例 `column_row_with_default_roundtrip_all_variants`），单元级 RED→GREEN 直接可达，不依赖页容量。

## 实现顺序与依赖

Iteration 000（表示与存储原语 → 写读闭环）：T1 tag → T2 溢出页模块 → T3 写路径 → T4 读路径；e2e（tests/overflow_page_test.rs）。
Iteration 001（生命周期、恢复与全通道）：T5 恢复面 → T6 释放 → T7 通道/形态/基线 e2e → T8 文档；T12 catalog DEFAULT 段长字符串语义（2026-10-02 用户指令 replan 并入，D11；依赖 T1，可与其他任务并行）。依赖 000 accepted。
Iteration 002（check 命令，独立验收）：T9 lib 校验入口 → T10 CLI 子命令 + 测试 → T11 文档。依赖：仅复用 000/001 的溢出链结构（页 census 识别 0x04）；可与 001 并行但按序执行。

## 测试策略

- 新 `tests/overflow_page_test.rs`（000/001）：边界（4073/4074/4075、chunk 边界 4080±1、65535/65536 两 tag 切换点）、增长/缩回、多行混排、重开、GC 释放（`gc_table` lib 面）、drop/TRUNCATE 释放、崩溃恢复两态（`seed_pending_wal` 同型夹具，不打 close）、加密、`:memory:`、只读、1MB 基线、dump→restore/import 往返、CTAS/INSERT SELECT/UPSERT/REPLACE 通道。
- 新 `tests/check_command_test.rs`（002）：干净库通过、逐类损坏注入（数据页 payload 字节、slot 目录 offset/length、页头 page_type、链 next_page_id 断/环、catalog 行、索引树页、未知页类型、pending WAL、加密库错钥/对钥）、`--format` 忽略、`:memory:`/`--readonly` exit 2、缺库 exit 1。
- 校准：`tuple.rs::deserialize_unknown_tag_rejected` 字面量 0x08 → 0x7F（断言语义不变）；`slotted_page.rs::test_slotted_page_no_space`（5000B 仍需失败于「单页 add_slot」原语层——原语不变，测试保持）。
- 回归：受影响定向（storage/executor/recovery/gc/cli）→ fmt/clippy → M19 全量分批串行（`--lib`/`--bins` + 集成目标按名字典序 8–12 个/批）。

## 风险与已知边界

- 1MB 行的 UPDATE 产生 ~2MB WAL 帧——恢复内存峰值与 checkpoint 重写时长增大（验收基线内可控，不做优化）。
- 大事务多超页行：链页分配多次 `allocate_page`——同现状多次页分配，无新约束。
- unreferenced 页在频繁崩溃-恢复循环下累积（D5/D6 已述）——不损坏可达数据，check 可见；空间回收留待未来 VACUUM/GC 激活（Non-goal）。
- 生产读路径对损坏 slot 元数据的 panic 面既有存在（D8），本 change 不扩大也不修复（Issue 候选上报）。
- `String(u16)` max_len 声明与值长不校验的既有语义保持（>65535 值现可正确存储；DDL 仍不能声明 >65535 max_len）。
