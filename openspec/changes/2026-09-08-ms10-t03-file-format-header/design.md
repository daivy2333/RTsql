# design: MS10-T03 文件 magic/格式版本头

> 调查依据：R19 `.claude/analysis/ms10-t03-file-format-header.md`（revision `268fa4f`）+ 本会话补查（`async_storage.rs` page_count 契约、tests 无主库裸 I/O）。用户决策 2026-09-08：exit 1 复用 / 旧文件统一拒绝 / 仅主库加头 / 不加 CRC。

## D1: 头布局——64 字节定长，字段自描述

```
offset 0..8    magic            b"RTSQLDB\0"
offset 8..12   format_version   u32 LE = 1
offset 12..16  flags            u32 LE（bit0 = encrypted，MS12 预留；其余位必须为 0）
offset 16..20  page_size        u32 LE = 4096
offset 20..52  salt_reserved    32B（MS12-T01 Argon2id 盐落点，当前必须全 0）
offset 52..64  reserved         12B（当前必须全 0）
```

- 64B 为 2 的幂，留出字段演进空间；盐区现在划出而非届时扩头，避免"版本 2 才有加密"的布局抖动。
- **为什么不用 4KB 超级页**（R19 方案 B）：与固定保留页 `TABLES_PAGE_ID=0`/`COLUMNS_PAGE_ID=1`（catalog.rs:35-37）冲突，需重编目录页号、改 bootstrap 断言与"首分配页"语义，浪费 4KB，且页 0 会被 MS12 页级 transform 波及需额外豁免。
- **为什么不用页 0 内嵌头**：页 0 是活跃 catalog 页，混格式脆弱。
- **拒绝方案**：sidecar 头文件破坏"持久化单文件"关键特性（SNAPSHOT）。

## D2: 头逻辑独立模块 `src/storage/file_header.rs`

纯函数 + 常量：`HEADER_SIZE = 64`、`FORMAT_VERSION = 1`、`FLAG_ENCRYPTED = 1`、`KNOWN_FLAGS_MASK`、magic 常量；`FileHeader { version, flags, page_size }` 结构与 `encode(&self) -> [u8; 64]` / `decode(&[u8; 64]) -> Result<FileHeader, HeaderError>`；`HeaderError` 为模块内私有分类（BadMagic/Unsupported/Incompatible），由 FileStorage 映射为 `StorageError` 变体（模块不依赖 error.rs 以外的东西，可独立单测）。

- 替代方案：内联进 file_storage.rs——open 已承担锁/长度校验，再塞编解码混职责；独立模块让"布局"有唯一权威落点，MS12-T01 只扩展此模块。

## D3: 偏移平移局部化在 FileStorage，`to_offset` 保持纯数学

`PageId::to_offset` 维持 `id * page_size`（`src/storage/page_id.rs:9-11`）。头偏移只在 FileStorage 的 3 个调用点合成：

- `read_page_blocking`（file_storage.rs:69）：`offset = HEADER_SIZE + page_id.to_offset(page_size)`
- `write_page_blocking`（file_storage.rs:81）：同上
- `allocate_page`（file_storage.rs:109）：`set_len(HEADER_SIZE + (page_id+1) * page_size)`

**收益**：`tests/storage_test.rs:10-17` 两个 `to_offset` 单测零修改；`PageId` 对非存储消费者保持"无文件概念"的纯语义。**风险面**：偏移遗漏只会发生在 file_storage.rs 单文件内，`file_storage_io_test` 的越界 EOF 与串页守卫 + 全量回归可暴露。

## D4: open 校验顺序与分支

`FileStorage::open` 在现有 try_lock（file_storage.rs:31-37）之后、`file_len % page_size` 检查（:43-48）处替换为：

```
锁（不变）→ metadata
├─ file_len == 0                  → encode 头，write_all_at 到 offset 0（无 fsync，D7）；page_count = 0
├─ file_len < 64                  → NotADatabase(path)
├─ 读 64B 头，decode
│   ├─ magic 不符或 version == 0  → NotADatabase(path)
│   ├─ version > FORMAT_VERSION   → NewerFileVersion(found)
│   ├─ flags & !KNOWN_FLAGS_MASK  → IncompatibleHeader("unknown feature flags …")（含加密位——前向防护，不把密文页当明文解析）
│   ├─ page_size != 4096          → IncompatibleHeader("page size …")
│   └─ (file_len − 64) % 4096 ≠ 0 → PageSizeMismatch { expected: 4096, actual: (file_len − 64) % 4096 }
└─ page_count = (file_len − 64) / 4096
```

- 锁先于一切（用户决策 + 既有 MS10-T02 语义），坏文件被占用时报锁不报格式。
- version 取值域：合法仅 `1..=FORMAT_VERSION`；`0` 视为未初始化/损坏归 `NotADatabase`；`> 1` 归"新版创建"。
- salt/reserved 区当前校验全 0（与 KNOWN_FLAGS_MASK 同理，防止未来字段被旧二进制误读）；MS12 启用盐时同步放宽该校验（届时 replan 面，不属本 change）。

## D5: 错误变体 additive（error.rs）

```rust
NotADatabase(String)            // "not an RTsql database: {path}"
NewerFileVersion(String)        // 携带路径与 found 版本号："database file was created by a newer version of RTsql (file version {found}): {path}"——携带 path 的 String 形态，对齐 DatabaseLocked 先例
IncompatibleHeader(String)      // "incompatible database header: {detail} ({path})"
```

- 均为 additive 变体；Act 需复核全仓无 `StorageError` 的穷尽 match（R19 结论：CLI `open_error_status` 只 match `DatabaseLocked` + 通配，无消费方需改）。
- CLI 零改动：`open_error_status` 的 `other => General` 分支（cli/mod.rs:148-153）承载，消息走 Display，退出码 1（用户决策 1）。

## D6: 0 字节 = 新库契约与 page_count 语义

`AsyncStorage::page_count` 文档（async_storage.rs:16-21）承诺"freshly-opened empty file returns 0"，是 `TableManager::new` bootstrap/open 分支依据。加头后：0 字节 → open 即写头 → 文件 64B、`page_count()==0` 不变 → bootstrap 照常分配页 0/1 → 文件 8256B。唯一代码动作：`async_storage.rs` 文档措辞 "empty file" → "file containing only the format header"。约 30 处测试空文件打开透明获得头，无需逐个适配。

## D7: 头写入用 `write_all_at`，不做 fsync

与 MS08-T01 pwrite 纪律一致（单 syscall、无共享偏移）。open 是同步上下文（database.rs:30 在 runtime block_on 内直接调用），`write_all_at` 直接可用。不做 `sync_data`：崩溃残留半头 → 下次 open 按 `NotADatabase` 拒绝；此刻新库无任何数据，删除重建零损失。头完整但页未落盘的场景由 catalog bootstrap 的 flush_all 兜底（既有行为）。

## D8: 测试策略

- **RED 见证**：`tests/file_header_test.rs::garbage_8k_file_opens_with_clean_error`——当前实现下 8192B 随机文件打开 panic（tokio::test 捕获为 panic 而非 Err），测试 RED；实现后 GREEN。其余拒绝场景（新版/flag/截断/过小/旧无头）当前实现同样无法满足断言（无头概念），天然 RED。
- **拒绝矩阵**：file_header_test 覆盖 9 场景（roundtrip、垃圾 4k/8k、<64B、version=99、version=0、加密 flag、page_size≠4096、截断、无头旧库页 0/1）。构造辅助：`write_header_at(path, FileHeader)` 测试夹具。
- **CLI e2e**（cli_test.rs 增补）：垃圾文件 exit 1 + stderr 文案；锁优先（持锁进程 + 垃圾文件 → exit 4）；拒绝不产生伴生文件。
- **回归**：全量 `cargo test`（636 基线 + 新增）；`drop_table_free_test.rs:18-19` helper 改 `(len − 64) / 4096`；`file_storage_io_test` 越界 EOF 断言预期自然保持（1 页文件实际 64+4096 字节，读 PageId(3) 偏移 64+12288 超界）。
- **验证命令**：`cargo test`、`cargo clippy -- -D warnings`、`cargo fmt --check`、`openspec validate`；CLI e2e 用 `target/debug/rtsql`。
