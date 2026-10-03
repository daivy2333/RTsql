# Iteration 000 / Cycle 000: 溢出表示与存储原语（写读闭环）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1、T2、T3、T4（tasks.md §1）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R1（超页元组经溢出页链存取）、R4 的写入失败清理面、R5（内联路径零回归）
- Excluded scope: 恢复重放（T5）、GC/drop/TRUNCATE 释放（T6）、全通道/加密/:memory:/只读 e2e 与 1MB 基线（T7）、文档（T8）、check 命令（T9–T11）——Iteration 001/002

**Objective**

>4074B 的 slot_data（22B VersionHeader + tuple 字节）经「主页 locator slot + 溢出页链」正确写入与读回：INSERT/SELECT 端到端、UPDATE 增长/缩回、多行混排、干净重开一致；≤4074B 既有路径与错误面逐字节不变。

**Investigation Facts**

- Current Baseline: master @ 81c8e6f，全量 1739 tests / 0 failures / 2 ignored（2026-10-02 MS20-T02 收尾轮，SNAPSHOT 采信）。本 Iteration 触碰面前无失效。
- Current-State Evidence（本 change Phase 2 精读 + 双代理映射，file:line 均已核实）:
  - `src/storage/page_format/tuple.rs:21-29` tag 域：0x01 Int / 0x02 String / 0x03 Null / 0x04 Float / 0x05 Bool / 0x06 Date / 0x07 Timestamp；**0x08 未用**（`deserialize_unknown_tag_rejected` 测试 `:780-784` 以 0x08 为未知 tag 字面量）。
  - `tuple.rs:88-97` String 序列化：`TAG_STRING [u16 len][bytes]`，`bytes.len() as u16`——>65535 静默回绕（本 Iteration T1 修复表示面）。三解码入口：`deserialize_tuple_width`（:178-294，宽度感知 + DEFAULT overlay :300-318）、`deserialize_value_refs`（:327-442，零拷贝）、`compute_tuple_size`（:50-64）。catalog DEFAULT 段复用同 tag 域（注释 :17-20）。
  - `src/storage/page_format/slotted_page.rs`：`Slot{logical_id u16, offset u16, length u16}` SIZE=6（:5-13）；`SlottedPageHeader` 16B：page_type@0、slot_count@1..3、free_space_offset@3..5、**next_page_id@5..9**、next_logical_id@9..11（:16-67）；页类型现行 0x01 Leaf/0x02 Internal/0x03 Data（:18 注释）；`add_slot_inner`（:225-274）容量检查与 `Err("No enough space in page")`；`get_slot_data` 返回 slot 数据切片（:195-199）；`SlottedPageRef` 只读零拷贝访问器（:78-135）。内联容量上限 = 4096−16−6 = **4074**。
  - `src/storage/data_page.rs:11-61` `write_tuple_to_data_page(buffer_pool, table_meta, version_header, tuple_bytes) -> Result<RowId>`：slot_data = vh.to_bytes()++tuple_bytes；tail 页 `guard.modify_page(|page| … slotted.add_slot(&slot_data))`；失败→`buffer_pool.storage().allocate_page()`→新页 init(0x03)→旧 tail `page.data[5..9]=next_id`→新页 add_slot 二次失败→`StorageError::PageFull`（:49-54，此时新页已挂链不回收）。页写模式 = `buffer_pool.get_page(id)` → guard `modify_page` 闭包。
  - `src/storage/data_page.rs:73-100` `read_tuple_from_data_page(buffer_pool, row_id, f: FnOnce(VersionHeader,&[u8])->Result<R>)`：`with_page_data` 页锁内调 f；**闭包内禁止 await/递归 BufferPool**（锁约束）。
  - `src/executor/data_scan.rs:427-503` DataScan 状态机：`with_page_data` 闭包内 `PageAction::{YieldValue(values), SkipSlot, JumpToPage(u64), Done}`；物化 `deserialize_row(tuple_bytes,&schema,&defaults)` 在闭包内（:500）；slot 消费即 `slot_index += 1`（:459）。
  - `src/storage/buffer_pool.rs:281-375` `find_visible_version(row_id, snapshot, f: FnOnce(&[u8])->Result<R>)`：内部枚举 `VisibilityResult::{Visible(R), NotVisible(Option<RowId>), NotFound}` + `f_opt.take()` 单次消费模式；all-visible 快路径（:306-331）与主循环（:333-364）均在页闭包内调 f。
  - VersionHeader 22B 内联于 slot 数据前缀（`data_page.rs:17`）；版本链遍历只消费 header。
  - 写通道上游：执行器经 `TableManager::write_tuple(&table_meta, version_header, tuple_bytes)`（`table_manager.rs:1553-1571`，tail 同步后委托 `write_tuple_to_data_page`）——本 Iteration 不改其签名。
- Code and Critical Path: 写 = executor → write_tuple → write_tuple_to_data_page（改点）；读 = read_tuple_from_data_page / DataScan / find_visible_version（改点）。溢出页为独立页（不入数据页 tail 链），经 BufferPool get/allocate/free 处理，加密（4124B 记录、AAD=page_id）与 `:memory:` 后端由存储层自动承接。

**Implementation Guidance**

格式契约（design D1–D4，Act 直接按此实现）：

- locator（溢出 slot 的 tuple 段，9B）：`[0xFF][total_len u32 LE][first_overflow_page u32 LE]`；slot.length = 22+9 = 31。0xFF 为 marker，不与 tuple tag 域（0x01–0x08）冲突；内联 payload 首字节恒 ∈ tag 域。
- 长字符串 tag：`TAG_LONG_STRING = 0x08`，payload `[u32 len LE][bytes]`；仅 `s.len() > 65535` 时使用，≤65535 维持 TAG_STRING。
- 溢出页：`SlottedPageHeader` 复用布局，`page_type = 0x04`、`next_page_id` = 链后继（0 终止）、`free_space_offset` = 本页 payload 字节数、其余字段 0；payload 自偏移 16 起，满页 4080B，末页余量。
- 写序（T3）：slot_data.len() > 4074 → 依 chunk 顺序 `allocate_page` + 写溢出页 → **最后**在数据页落 locator slot（tail 页满则按既有新页分配挂链逻辑落 locator）；任一步失败 → 释放本次已分配的全部链页后返回原错误。≤4074 路径代码逐字节不动（预检查置于溢出分支判定，不触碰既有分支）。
- 读序（T4）：三消费点统一模式——页闭包内识别 `tuple_bytes[0] == 0xFF` → 解出 (total_len, first_page) 出闭包 → 顺序 get_page 装配（校验各页 0x04、payload_len、累计 = total_len）→ 以装配缓冲继续原逻辑；内联路径零拷贝保持。`find_visible_version` 的 all-visible 快路径同型处理。建议装配/分解/释放原语集中在 `src/storage/page_format/overflow.rs`（T2），三消费点只做模式接线。

**Task Contracts**

### T1: 长字符串增量 tag 0x08

- Requirement/Scenario: R1（S1/S11 表示面）；S2 页边界两值形态
- Depends on: None
- Targets: `src/storage/page_format/tuple.rs::{TAG_LONG_STRING, compute_tuple_size, serialize_tuple, deserialize_tuple_width, deserialize_value_refs}`
- Current behavior: String 一律 `[0x02][u16 len]`，>65535 值长度回绕写坏；0x08 被三解码入口拒绝。
- Required behavior: 序列化 >65535B String 用 `[0x08][u32 len]`（≤65535 逐字节不变）；三解码入口 + `compute_tuple_size` 接受两 tag；`as u16` 回绕消除（用显式长度分支，超长值不再静默截断）。
- Preserve: ≤65535 序列化字节与既有完全一致；非 String 值路径零变化；catalog DEFAULT 段同 tag 域自动受益，不单独改。
- Forbidden: 不改 ColumnType::String(u16) 声明面；不新增 max_len 写入校验（既有语义）。
- Test witness: `tuple.rs` 单测新模块（RED：长串往返 + 65535/65536 切换点 + 回绕消除断言）；校准 `deserialize_unknown_tag_rejected` 字面量 0x08→0x7F（断言语义不变）。命令 `cargo test --lib tuple`。
- GREEN condition: 新单测全绿 + 既有 tuple 单测零回归。
- Verification: `cargo test --lib tuple -- --test-threads=1` 0 failed。
- Stop when: 发现 tag 域或 DEFAULT 段消费点与本契约冲突（返回 Plan）。

### T2: 溢出页格式与链原语模块

- Requirement/Scenario: R1（S1 存储格式）、R4（S「写入失败不泄漏」的原语基础）
- Depends on: T1
- Targets: 新 `src/storage/page_format/overflow.rs`（含单测）；`src/storage/page_format/mod.rs` 导出
- Current behavior: 无溢出机制；>4074B 必 PageFull。
- Required behavior: 提供（a）locator 编解码（0xFF/total_len u32/first_page u32）；（b）chunk 分解纯函数（total → 页数与各页 payload_len）；（c）pool 级写链 `write_overflow_chain(buffer_pool, tuple_bytes) -> Result<PageId /*first*/>`（顺序 allocate+写 0x04 页）；（d）装配读 `read_overflow_chain(buffer_pool, first_page, total_len) -> Result<Vec<u8>>`（校验页型/长度/累计）；（e）链收集 `collect_overflow_pages(buffer_pool, first_page) -> Result<Vec<PageId>>` 与释放 `free_overflow_chain(buffer_pool, first_page)`。
- Preserve: 不修改 SlottedPage 既有方法；0x04 页不进数据页 tail 链。
- Forbidden: 不接线执行器/写路径（T3/T4 职责）；不在原语内做 GC 策略。
- Test witness: 新模块单测（RED）：编解码往返、chunk 边界（4080±1、total=1、total 恰 N 页）、写后读装配等价、链收集/释放幂等、页头字段正确。
- GREEN condition: 模块单测全绿；`cargo build` 通过。
- Verification: `cargo test --lib overflow -- --test-threads=1` 0 failed。
- Stop when: BufferPool 页写模式与 `guard.modify_page` 假设不符（返回 Plan）。

### T3: 写路径溢出分支

- Requirement/Scenario: R1（S1/S2/S3 写面）、R4（写入失败不泄漏）、R5（≤4074 零回归）
- Depends on: T2
- Targets: `src/storage/data_page.rs::write_tuple_to_data_page`
- Current behavior: slot_data 任意长度先试 tail add_slot，失败分配新数据页挂链再试，>4074 二次失败 → PageFull 且新空页残留链上。
- Required behavior: slot_data.len() > 4074 → 直走溢出：`write_overflow_chain(tuple_bytes)` → 数据页按既有逻辑落 31B locator slot（tail 满则新页挂链——既有代码路径复用）；链写或落位任一步失败 → `free_overflow_chain` 清理本次已分配页后冒原错误（无 locator 落位即无 dangling）。≤4074 路径不触碰（既有残留形态对该区间本就不可达）。
- Preserve: `write_tuple`/`TableManager` 签名；WAL 记录内容（Insert/Update 仍携完整 tuple 字节——上游构造，本任务不改）。
- Forbidden: 不改 SlottedPage；不实现 GC/drop/truncate 释放（T6）。
- Test witness: 新 `tests/overflow_page_test.rs`（RED）：>4074 写入成功 + 读回（读经 T4 前可先以 locator 原语直读作过渡见证？**否**——T3/T4 同 Iteration 顺序执行，e2e 见证在 T4 后统一建立；T3 见证 = `write_tuple` 返回 RowId + 数据页 slot 存在（lib 直查）+ `cargo test --lib data_page`）。
- GREEN condition: data_page 单测（新增溢出分支/失败清理注入）全绿。
- Verification: `cargo test --lib data_page -- --test-threads=1` 0 failed。
- Stop when: 事务/WAL 写序假设与 executor 实际调用序冲突（返回 Plan）。

### T4: 读路径两相装配

- Requirement/Scenario: R1（S1/S3 读面）、R5（内联零拷贝与既有行为零回归）
- Depends on: T3
- Targets: `src/storage/data_page.rs::read_tuple_from_data_page`；`src/executor/data_scan.rs`（PageAction 扩展）；`src/storage/buffer_pool.rs::find_visible_version`
- Current behavior: 三消费点在页闭包内直接消费 `slot_data[22..]`；对 locator 形态会把 9B locator 当 tuple 解析（错）。
- Required behavior: 三点统一两相模式——闭包内 `tuple_bytes[0]==0xFF` → 携 (vh, total_len, first_page) 出闭包 → `read_overflow_chain` 装配 → 继续原消费（f/反序列化/状态机 yield）。DataScan 增 `PageAction::YieldOverflow` 变体（slot 已消费，出闭包装配后按 YieldValue 后续处理）；`find_visible_version` 增 `VisibilityResult::VisibleOverflow` 变体（all-visible 快路径同型）。内联路径逐字节保持零拷贝。
- Preserve: 三 API 对外签名与返回类型；内联行行为；页锁约束（闭包内无 await——装配全部在闭包外）。
- Forbidden: 不改 get_slot_data；不改版本链遍历逻辑（header 内联不变）。
- Test witness: `tests/overflow_page_test.rs`（RED→GREEN）：4073/4074/4075 边界三值、chunk 边界（4080 倍数±1）、UPDATE 增长为超页/超页缩回页内、超页与内联行混排、SELECT 全表扫描 + 主键点查（find_visible_version 面）、干净重开一致。
- GREEN condition: overflow_page_test 上述用例全绿 + 既有 executor/storage 定向回归 0 failed。
- Verification: `cargo test --test overflow_page_test -- --test-threads=1` 0 failed；`cargo test --lib -- --test-threads=1` 0 failed。
- Stop when: 装配引入的跨页 await 与页锁/guard 生命周期冲突无法在闭包外解决（返回 Plan）。

**Invariants**

- ≤4074B slot_data 的落库字节、错误面、性能路径逐字节不变（既有全量测试零回归）。
- VersionHeader 恒内联于数据页 slot；版本链遍历（可见性/superseded/abort/commit 传播）不感知溢出。
- 溢出页永不进入数据页 tail 链（`data_chain_status`/dump 预检零影响）。
- locator 落位是溢出写序的最后一步（崩溃窗口无 dangling）。
- 页锁闭包内不得 await/递归 BufferPool（装配在闭包外）。

**Non-goals**

本 Cycle 不处理：恢复重放溢出（T5）、GC/drop/TRUNCATE 释放（T6）、加密/:memory:/只读 e2e 与 1MB 基线、全通道（T7）、文档（T8）、check（T9–T11）、生产读路径 panic 面修复（Issue 候选）。

**Acceptance**

R1（写读闭环面）：S1 写读一致、S2 页边界两侧可达、S3 增长缩回；R4 写入失败不泄漏面（原语 + 写路径清理）；R5 内联零回归（既有单测全绿）。映射：T1→表示、T2→原语、T3→写、T4→读；见证 = overflow_page_test + tuple/overflow/data_page 单测 + `--lib` 回归。

**Verification**

- `cargo test --lib -- --test-threads=1`（tuple/overflow/data_page/buffer_pool 单测 + 全 lib 回归）0 failed。
- `cargo test --test overflow_page_test -- --test-threads=1` 0 failed。
- `cargo fmt --check` 与 `cargo clippy --tests` 无新告警。
- 受影响定向回归：`cargo test --test projection_test --test pushdown_test --test gc_test -- --test-threads=1`（DataScan 消费面）0 failed。

**Gate 2 Readiness**

PASS（2026-10-02 用户批准完整计划，选项「批准完整计划」；同批批准 T02 方案「仅 rtsql check」——design D7 定稿）。检查项无 BLOCKED/WAIVED：无 Missing/Simplified requirement；调查、设计、任务契约、分轮、追踪、验证逐项满足；Persisted Evidence = none；无非实质选择外的待决项。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- 65535/65536 切换点与 4074 阈值相互独立（表示面 vs 页容量面），测试矩阵需两轴覆盖（已在 T1/T4 见证列出）。
- `find_visible_version` 的 all-visible 快路径是易漏点（两处闭包消费 f），契约已点名。
- 1MB 级 UPDATE 的 ~2MB WAL 帧在 T5/T7 验证（本 Cycle 不触碰恢复面）。
- 非实质选择（overflow.rs 内部函数拆分、命名、单测组织）由 Act 自定。

## Act Response

- Status: reported

**实际改动**

产品代码（5 修改 + 2 新增；`git diff --stat` 361 insertions / 27 deletions）：

- `src/storage/page_format/tuple.rs`（T1）：`TAG_LONG_STRING = 0x08` 常量 + `STRING_U16_MAX` 阈值；`compute_tuple_size`/`serialize_tuple` String 臂 >65535 走 `[0x08][u32 len LE][bytes]`（≤65535 逐字节不变，`as u16` 回绕消除）；`deserialize_tuple_width`/`deserialize_value_refs` 各增 TAG_LONG_STRING 解码臂（截断/坏 UTF-8 诚实拒绝）；模块格式文档更新；`deserialize_unknown_tag_rejected` 校准 0x08→0x7F（断言语义不变）；新测试 3 例（长串三入口往返 / 65535-65536 切换点含字节布局断言 / 混排元组）。
- `src/storage/page_format/overflow.rs`（T2，新增）：locator 编解码（`LOCATOR_TAG 0xFF`/`LOCATOR_SIZE 9`/`encode_locator`/`decode_locator`/`is_overflow_ref`）、`chunk_plan` 纯函数（4080 满页 + 余量末页）、页级 `build_overflow_page`/`parse_overflow_page`（page_type 0x04、free_space_offset=payload 长度、next_page_id 链后继；越界/错页型点名拒绝）、pool 级 `write_overflow_chain`（预分配全链页 id→逐页写→失败释放本次已分配页）、`read_overflow_chain`（页型/长度/累计=total_len 校验 + 页数上界防环）、`collect_overflow_pages`（visited 防环）、`free_overflow_chain`；`INLINE_SLOT_MAX = 4096−16−6 = 4074` 常量。单测 10 例含分配失败注入（`FailAfterNAllocs` mock）验证清理后页可回收。
- `src/storage/page_format/mod.rs`：`pub mod overflow` 导出。
- `src/storage/data_page.rs`（T3/T4）：`write_tuple_to_data_page` 顶部溢出判定（slot_data > 4074 → `write_overflow_chain` → 31B locator slot_data 置换；`overflow_first_page` 记录）；落位失败臂 best-effort `free_overflow_chain` 后冒原 `PageFull`；≤4074 既有路径逐字节未动。`read_tuple_from_data_page` 两相装配（闭包内识别 locator → `SlotRead::Overflow{vh,total_len,first_page}` 出闭包 → `read_overflow_chain` 装配 → 闭包外消费 f；内联零拷贝路径保持）。
- `src/executor/data_scan.rs`（T4）：`PageAction::YieldOverflow { total_len, first_page }` 变体；页闭包内在可见性判定后识别 locator；出闭包装配 `read_overflow_chain` + `deserialize_row` 归一为 `YieldValue` 后续处理（filter→projection→cap 复用）；终态 match 增结构性不可达臂（规范化后穷尽性）。
- `src/storage/buffer_pool.rs`（T4）：`find_visible_version` 增 `VisibilityResult::VisibleOverflow`；all-visible 快路径增 `FastPath::{Done,Overflow}` 两相（契约点名的易漏点）；主循环可见臂识别 locator 出页锁装配后消费 f；两处装配均在页锁外。
- `tests/overflow_page_test.rs`（新增 9 例）：写侧（locator/链页直查见证、4073/4074 内联边界、4075 溢出边界）+ 读侧（SQL 边界三值、chunk 4080±1、UPDATE 增长/缩回、内联超页混排、PK 点查 find_visible_version 面、clean reopen 一致）。

**与计划的偏差**

1. 计划 Investigation Facts 称「catalog DEFAULT 段复用同 tag 域自动受益，不单独改」——实际 catalog 有独立内联 DEFAULT 序列化器（`catalog.rs:1226-1258`，String 仍 `s.len() as u16`），不会自动获得 0x08 语义。按契约 Preserve 条款未改动；>65535 DEFAULT 字面量在 catalog 段仍回绕（既有边界，本 change 前后一致）。见「未解决问题」。
2. `write_overflow_chain` 空输入返回 0，与 locator「first_page=0 = 无链」哨兵同值；生产不可达（catalog 引导恒占页 0–1，真实链不可能从页 0 起），已在模块文档写明；单测装配采用生产形态（TableManager 引导）。
3. 测试侧发现：`wal_buffer.shutdown()` ≠ `close()`（前者为崩溃模拟、不 checkpoint，DDL 不入 WAL）——初次 clean-reopen 用例误用 shutdown 致重开时 redo 报 `table not found`（DDL 未持久化），按测试意图（干净重开而非崩溃恢复两态，后者属 Iteration 001 T5）改为 `db.close()`。产品行为无变化。
4. 非实质自定（契约授权 Act 自定）：overflow.rs 内部函数拆分（`write_chain_pages` 提取）、清理实现置于 `write_overflow_chain` 内部（对外契约不变）、单测组织。

**自检发现与已修复**

- Gate 3 修复：locator 落位失败臂原用 `?` 传播清理错误，会以清理错误替换原 `PageFull`——改为 best-effort `let _ = free_overflow_chain` 后冒原错误（契约「冒原错误」），修复后重跑 lib + overflow_page_test 全绿。
- clippy 新告警 8 条（新测试 `&[x.clone()]` → `std::slice::from_ref`、doc 注释 `>` 开头被判 blockquote）全部修复；`cargo fmt` 格式化后 `--check` 干净。
- `SlottedPageHeader` 字面量构造遇私有 `_padding` → 改 `new()` + 公有字段赋值。

**验证**

| 命令 | 结果 | 退出码 |
|---|---|---|
| `cargo test --lib tuple -- --test-threads=1` | T1 RED（`TAG_LONG_STRING` 不存在编译失败）→ GREEN **29 passed; 0 failed** | 0 |
| `cargo test --lib overflow -- --test-threads=1` | T2 RED（10 FAILED，todo!() 桩）→ GREEN **12 passed; 0 failed**（10 新 + 2 既有同名匹配） | 0 |
| `cargo test --test overflow_page_test -- --test-threads=1` | T3 RED（2/3 写用例 PageFull）→ T4 RED（6 读用例 locator 误解析）→ GREEN **9 passed; 0 failed** | 0 |
| `cargo test --lib -- --test-threads=1` | **383 passed; 0 failed; 0 ignored**（基线 371 + 净增 12，零回归） | 0 |
| `cargo test --test projection_test --test pushdown_test --test gc_test -- --test-threads=1` | **6 + 16 + 3 passed; 0 failed**（DataScan 消费面定向回归） | 0 |
| `cargo fmt --check` / `cargo clippy --tests` | fmt 干净；clippy 仅剩既有环境告警（`~/.cargo/config` 弃用提示，非代码） | 0 |

Acceptance 映射：R1 写读闭环面（S1 写读一致 / S2 页边界两侧可达 / S3 增长缩回）由 overflow_page_test 9 例见证；R4 写入失败不泄漏面由 T2 失败注入 + T3 清理见证；R5 内联零回归由 4073/4074 边界用例 + lib 383 全绿见证。加密/`:memory:`/只读/1MB 基线/恢复两态/释放矩阵属 Iteration 001（T5–T7），本 Cycle 未触及。

**Persisted Evidence**

- None required（Plan 模式 none；验证均可低成本重跑，上表即决定性输出）。

**Experience Candidates**

- None。

**未解决问题（非阻塞，供审计与后续裁定）**

1. catalog DEFAULT 段独立序列化器对 >65535B String DEFAULT 字面量仍 `as u16` 回绕（计划假设失准处，偏差 1）。DDL 侧现实不可达性高、既有边界未扩大；是否登记 Issue 由用户裁定（本 Act 不落账）。
2. `BufferPool::write_commit_tx_id`（提交路径）对溢出行经 `read_tuple_from_data_page` 完整装配链 payload 后传入 `update_version_header_in_data_page` 的 `_tuple_bytes`（被忽略）——行为正确但提交热路径 O(row) 读链；优化超出本 Cycle 范围（三 API 契约为「Preserve」项），留作性能批候选观察。

## Plan Review

- Review Result: accepted

**Findings**

独立检查方式：完整阅读工作区 diff（5 修改文件）与 2 个新增文件全文，逐条核对 Act Response 声明与代码；Act 的验证结论（lib 383 / overflow_page_test 9 / 定向回归 / fmt-clippy）产生于本会话、其后覆盖面未变化，直接采信（来源：000 Act Response 验证表）；`git diff --stat` 与 Act 引用差 1 行（362/27 vs 361/27），核对内容与描述的最终态（含 Gate 3 修复）逐文件一致，判为统计引用时点的非实质出入，不构成重跑触发。

1. 〔Minor，不阻塞｜Plan 遗漏〕**CREATE INDEX 存量回填是计划读点清单之外的运行期 tuple 消费点**：`src/executor/create_index.rs:118-163` 在页闭包内对 `slot_data[22..]` 直接 `deserialize_row`，超页行 slot 的 tuple 段为 9B locator → 未知 tag 0xFF 点名报错（响亮失败，无坏数据）。不在 Iteration 000 Acceptance 内（R1–R5 完整状态属 Iteration 001 稳定基线），但违反 R1「溢出对 SQL 层 SHALL 透明」，必须在 001 收口。处置：已写入 Iteration 001 T5 契约（与恢复面同型的两相装配）+ T7 回填见证。
2. 〔Minor，不阻塞｜Plan 错误，Act 已按 Preserve 正确处置〕catalog DEFAULT 段为独立序列化器（`catalog.rs`），不自动获得 0x08 语义；>65535B String DEFAULT 字面量在 catalog 段仍回绕（既有边界，未扩大）。Act 偏差 1 如实上报，契约 Preserve 条款阻断了越权修改——处置正确。是否登记 Issue 由用户裁定（Act 不落账，本 Review 同样只报告）。
3. 〔观察项，非缺陷〕`BufferPool::write_commit_tx_id` 提交热路径对溢出行经完整链装配后传入被忽略的 `_tuple_bytes`——行为正确、O(row) 读链。留作 MS22 性能批候选观察（000 Act 未解决问题 2，维持）。
4. 〔已接受的偏差〕`write_overflow_chain` 空输入返回 0 与「first_page=0 = 无链」哨兵同值——生产不可达（catalog 引导恒占页 0–1），模块文档已写明，单测采用生产形态。接受。
5. 〔核实记录〕关键不变量逐项验证：slot 消费先于溢出判定（`data_scan.rs:463` vs `:506`，无跳行/死循环）；全部装配在页锁闭包外；locator 恒为写序最后一步；溢出页不入数据 tail 链；≤4074 路径与 ≤65535 序列化逐字节未动；`deserialize_tuple`/`deserialize_row` 均委托 `deserialize_tuple_width`（tag 覆盖完整）。其余 slot 字节直接消费点排查完毕：`build_superseded_map`/`check_page_all_visible`/`mark_uncommitted_aborted`/GC·重写收集器均 header-only 安全；回滚路径（`manager.rs:526/:603`）经 `read_tuple_from_data_page` 已装配安全；`btree/node.rs` 为 32B 键格式不相干；catalog 行按 Non-goal 保持页内。

**Acceptance Gaps**

None——R1 写读闭环面、R4 写入失败不泄漏面、R5 内联零回归由 Act 验证表与代码审读共同证实。

**Evidence**

- 代码：`src/storage/page_format/tuple.rs`（T1 三解码入口 + 0x7F 校准 + 3 新测试）、`overflow.rs`（T2 原语 + 失败注入 mock + 10 单测）、`src/storage/data_page.rs`（T3 溢出分支与 best-effort 清理、T4 `SlotRead` 两相）、`src/executor/data_scan.rs`（`YieldOverflow` 归一化）、`src/storage/buffer_pool.rs`（`VisibleOverflow` + FastPath 两相）、`tests/overflow_page_test.rs`（9 例与 T3/T4 见证清单一一对应）。
- 验证采信：000 Act Response 验证表（lib 383 passed / overflow_page_test 9 passed / projection+pushdown+gc 定向绿 / fmt-clippy 干净，退出码 0）；两道不可默认检查满足——GREEN 真实运行有记录，本 Review 独立阅读完整 diff 核对声明与代码一致。

**Follow-up Decision**

接受（accepted）：无 Acceptance gap；全部 finding 为非阻塞 Minor，且 finding 1 已转化为 Iteration 001 T5/T7 契约的必要内容，无需当前 Cycle 返工。Act 留存的两个未解决问题维持「报告不落账」，提交用户裁定。

**Next Cycle**

None

**Next Iteration**

`iterations/001-lifecycle-recovery-channels/000-initial.md`（已展开，Status: ready）
