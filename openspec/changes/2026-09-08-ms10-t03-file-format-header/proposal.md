# proposal: MS10-T03 文件 magic/格式版本头

## Why

`FileStorage::open`（`src/storage/file_storage.rs:20-58`）当前对文件内容零校验：只有 try_lock 独占锁和 `file_len % 4096` 整除检查。任何大小合法的非 RTsql 文件都会被 catalog / B-Tree 当作页数据解析，产生三类不可判读的失败（2026-09-08 实测，R19 分析）：

1. **8192B 垃圾文件 → SIGABRT（exit 134）**：SlottedPage 对垃圾页数据做无校验切片 panic（`src/storage/page_format/slotted_page.rs:128`），随后 `PageGuard::drop` 中 PoisonError unwrap 二次 panic，panic-in-destructor 非 unwinding abort。进程崩死而非报错。
2. **4096B 垃圾文件 → 误导性 IO 错误**：catalog 固定触碰页 0/1，页 1 越过 EOF → `IO error: failed to fill whole buffer`（exit 1）。
3. **小文件 → 误导性 `Page size mismatch`**：57B 文本文件报 `expected 4096, got 57`，用户无从判断文件根本不是数据库。

同时 MS12-T01（整库加密）以本任务为硬前置：加密 flag 区分明/密库、Argon2id 盐存文件头、页级 AES-GCM transform 不得覆盖头——头必须先落，且位于页空间之外。路线图标注"趁零用户落"：现在引入头的兼容成本最低（SNAPSHOT 记录零存量用户），晚做需迁移存量无头文件。

**用户决策（2026-09-08，本会话）**：

1. **退出码**：格式错误复用 exit 1（General），stderr 给出具体原因；退出码表（0/1/2/3/4/5）不扩面。
2. **旧无头文件**：统一拒绝（"不是 RTsql 数据库"），不做页 0/1 嗅探迁移——嗅探保留把任意垃圾文件误判为旧库的风险，正是当前 panic 场景的根源。
3. **范围**：仅主库文件加头；`.wal` / `.checkpoint` 伴生文件不加（WAL 帧已有逐帧 CRC，位点损坏已有安全退化全量重放）。
4. **头校验和**：不加 CRC。magic + version + 页整除校验已覆盖"认错文件 / 新版文件 / 截断损坏"三类目标场景；不引入 CLAUDE.md 默认禁止的校验和。

## What Changes

- **新增 `src/storage/file_header.rs`**：64 字节文件头布局——magic `"RTSQLDB\0"`（8B）+ format_version u32 LE（=1）+ flags u32 LE（bit0 = encrypted，MS12 预留）+ page_size u32 LE（=4096）+ 32B 盐预留区（MS12 Argon2id）+ 12B 保留；encode/decode 纯函数 + `HEADER_SIZE`/`FORMAT_VERSION` 常量 + 单元测试。
- **`FileStorage::open` 接线（锁之后）**：0 字节文件 → 写头（新库语义保持，page_count 仍为 0）；`< 64B` / magic 不符 / version=0 → `NotADatabase`；`version > 1` → `NewerFileVersion`（"文件由新版创建"）；未知 flags 位（含加密位，当前版本不支持）或 page_size ≠ 4096 → `IncompatibleHeader`；`(len − 64) % 4096 ≠ 0` → 既有 `PageSizeMismatch`（语义更新为扣除头后的页整除校验）。任何拒绝都发生在 catalog/WAL/recovery 之前。
- **页偏移平移局部化**：`PageId::to_offset` 保持纯数学 `id * page_size`（`tests/storage_test.rs:10-17` 两个偏移单测零修改）；偏移加头只在 FileStorage 读页/写页/分配页 3 个调用点进行。
- **错误变体 additive（`src/storage/error.rs`）**：`NotADatabase(String)`、`NewerFileVersion(u32)`、`IncompatibleHeader(String)`，均携带路径或字段详情；CLI `open_error_status` 零改动（General/exit 1 分支承载，用户决策 1）。
- **测试**：新增 `tests/file_header_test.rs`（头 roundtrip + 拒绝矩阵，垃圾文件 panic 场景以 RED 起步）；`tests/cli_test.rs` 增补 e2e（exit 1 + 文案、锁优先于头校验）；`tests/drop_table_free_test.rs` 的 `page_count` helper 扣除头；全量 636+ 回归。

## Out of Scope（本 change 不做）

- `.wal` / `.checkpoint` 伴生文件加头（用户决策 3）。
- 旧无头文件的嗅探与自动迁移（用户决策 2）。
- 头部 CRC（用户决策 4）；可变页大小支持（只记录并校验 4096）。
- MS12 加密实现本身（头 flags 位与盐区仅预留布局）。
- free-list 持久化（R19 相邻发现：重启后 free-list 丢失、释放页泄漏——独立改进候选）。
- Windows / 非 Unix 平台（`FileExt` 限 Unix，MS13 non-goal）。
- REPL、网络协议路径、lib API 签名变化。

## Impact

- **修改**：`src/storage/file_storage.rs`（open 校验/初始化 + 3 处偏移加头）、`src/storage/error.rs`（+3 additive 变体）、`src/storage/mod.rs`（导出 file_header）、`src/storage/async_storage.rs`（`page_count` 文档措辞：empty file → header-only file）、`tests/drop_table_free_test.rs`（helper 扣头）。
- **新增**：`src/storage/file_header.rs`（约 80 行 + 单测）、`tests/file_header_test.rs`（约 10 用例）、`tests/cli_test.rs` 增补约 4 用例。
- **行为变化**：① 8192B 垃圾文件打开 → exit 1 干净报错（此前 SIGABRT 134）；② 新库文件自带 64B 头（文件最小 64B，此前 0B；bootstrap 后最小 8256B，此前 8192B）；③ 无头/新版/未知 flag 文件显式拒绝且不触碰任何页数据与 WAL；④ 0 字节文件仍是"新库"。
- **兼容性**：`Database::open` / `FileStorage::open` 签名不变；新错误变体 additive（Act 复核无 `StorageError` 穷尽 match 消费方）；catalog 保留页 0/1、页号语义、BufferPool / B-Tree / WAL 恢复路径零变化；约 30 个测试文件的空文件打开透明获得头，`AsyncStorage::page_count()==0 → bootstrap` 契约保持。
- **风险**：头写入不做 fsync——崩溃残留半头 → 下次 open 按 `NotADatabase` 拒绝；新库此刻尚无数据，删除重建即可，无数据损失。头偏移平移若遗漏调用点会被全量回归与 file_storage_io_test 的 EOF/串页守卫暴露。
