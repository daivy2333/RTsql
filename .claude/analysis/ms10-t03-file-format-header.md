# MS10-T03 文件格式头：打开链路、页寻址约束与放置方案

> Snapshot: [SNAPSHOT](../docs/SNAPSHOT.md)
> Captured revision: `268fa4f`（master，工作树 clean；运行实验使用 `target/debug/rtsql`，构建于 2026-09-08 19:13，回溯行号与该 revision 源码一致）
> Observed branch: master
> Captured at: 2026-09-08

## 目标与范围

为 MS10-T03（文件 magic/格式版本头，FileStorage open 校验）的 Plan 提供实现调查输入，回答八个问题：

1. 格式头校验挂在打开链路哪个位置？
2. 磁盘布局对头放置有什么约束？哪些方案可行？
3. 0 字节新库契约与无头文件如何分类？
4. 当前打开非 RTsql 文件的行为基线是什么？
5. CLI 错误面如何映射格式错误？
6. MS12-T01 对头字段有什么需求？
7. 哪些测试断言裸布局，636 基线中哪些会动？
8. 范围边界（.wal/.checkpoint、free-list）在哪里？

调查限定主库文件。运行时实验在 /tmp 完成，未修改仓库。

## 已确认事实、推断与未确认项

### 已确认事实

- 打开顺序（`src/database.rs:28-93`）：`FileStorage::open` → `BufferPool::new(100)` → `TableManager::new` → `open_or_init` → `TransactionManager::new` → `WalWriter::open` → `WALBuffer` → `RecoveryManager::full_recover` → `attach_index_catalog_contexts` → `CheckpointManager::new`。`FileStorage::open` 是唯一文件级入口，先于一切页解析与 WAL。
- `FileStorage::open`（`src/storage/file_storage.rs:20-58`）：create-or-open 读写模式；`try_lock` 独占锁，冲突报 `StorageError::DatabaseLocked`，先于 WAL 打开与恢复（MS10-T02）；随后仅做 `file_len % 4096 != 0 → PageSizeMismatch`（actual = 余数）与 `page_count = file_len / 4096`。对文件内容零校验。
- 页寻址唯一偏移源：`PageId::to_offset = page_id * page_size`（`src/storage/page_id.rs:9-11`）。生产代码仅 3 个调用点，全在 file_storage.rs——读页 :69、写页 :81、`allocate_page` 扩容 :109。
- 保留页固定：`TABLES_PAGE_ID = 0`、`COLUMNS_PAGE_ID = 1`（`src/storage/catalog.rs:35-37`）。`Catalog::bootstrap` 在空文件上按序分配页 0、页 1，分配结果不是期望值时返回 `Internal` 错误（catalog.rs:98-137）；第一个用户数据页是页 2。`Catalog::open` 直接触碰页 0 和页 1，且注释明示不校验页类型（catalog.rs:126-133）——"文件不是 catalog"的判定留给调用方，现状由后续解析失败兜底。
- 新库契约：0 字节文件 = 全新数据库。catalog bootstrap 完成后文件变为 8192 字节（实测）。`FileStorage::open` 在约 30 个测试文件、多个 `#[cfg(test)]` 单测和 bench 中以 NamedTempFile 或 tempdir 空文件直接调用，全部依赖这一契约。
- 伴生文件两个：WAL = `db_path.with_extension("wal")`（替换扩展名，`foo.db → foo.wal`；`src/wal/writer.rs:27`，recovery 另有 5 处同公式）；checkpoint 位点 = `db_path.with_extension("checkpoint")`（`src/wal/checkpoint.rs:52`），固定 16 字节。WAL 文件本身无 magic 头，LSN 即字节偏移。
- CLI 错误映射（`src/cli/mod.rs:143-154`）：`open_error_status` 只特判 `DatabaseLocked → ExitStatus::Locked`（exit 4）；其余 `StorageError` 一律 `ExitStatus::General`（exit 1），消息 `failed to open database {path}: {err}`。退出码表 0/1/2/3/4/5 无格式错误专码（`src/cli/mod.rs:19-32`）。
- MS12-T01 依赖（`tasks.md` MS12 节）：文件头加密 flag 区分明/密库；Argon2id 盐存文件头；页级 AES-GCM transform 加在 FileStorage 读写路径。头必须位于页空间之外，才能不被页级加解密 transform 覆盖。

### 运行时实验（sad path 基线，2026-09-08 实测）

| 输入文件 | 行为 | 退出码 | 根因 |
|---|---|---|---|
| 4096B 随机数据 | `IO error: failed to fill whole buffer` | 1 | catalog `open` 触碰页 1（偏移 4096..8192）越过 EOF → `read_exact_at` UnexpectedEof |
| 57B 文本 | `Page size mismatch: expected 4096, got 57` | 1 | `file_len % 4096` 校验命中，消息对非 DB 文件有误导 |
| 8192B 随机数据 | **panic → abort（SIGABRT）** | 134 | 见下方崩溃链 |
| 0 字节 | 当作新库正常打开（后续 exit 3 是 `SELECT 1` 缺 FROM 的 SQL 错） | 3 | catalog bootstrap 分配页 0/1，文件变 8192B |

8192B 崩溃链：`Database::open`（database.rs:36）→ `open_or_init`（table_manager.rs:160）→ `Catalog::scan_tables`（catalog.rs:210）→ `scan_chain`（catalog.rs:392）→ `with_page_data`（buffer_pool.rs:153）。垃圾页数据使 SlottedPage 无校验切片越界 panic（`src/storage/page_format/slotted_page.rs:128`），随后 `PageGuard::drop` 中 `unwrap` PoisonError 二次 panic（`src/storage/page_frame.rs:101`），panic in destructor → 非 unwinding abort。大小合法、页 0/1 齐全的垃圾文件是当前最危险的打开场景，进程直接崩死而非报错。

### 推断（待 Plan 确认）

- 头放置推荐方案 A——文件前缀头 + 偏移平移：头 H 字节置于文件头，页 N 位于 `H + N*4096`。`to_offset` 是唯一收敛点，页号语义、catalog 常量、BufferPool、B-Tree、恢复路径全部不动。
- 方案 B——超级页（页 0 作头）不可取：与 `TABLES_PAGE_ID=0` 冲突，需重编目录页号、改 bootstrap 断言与"首分配页"语义（storage_test.rs:110-117 断言首分配页为 0），浪费 4KB，且页 0 会被 MS12 页级 transform 波及，需额外豁免。
- 方案 A 改动面：`page_id.rs`（偏移公式加头常量；注意 `to_offset(page_size)` 签名不变、头偏移为独立常量）、`file_storage.rs`（长度校验改 `(len-H) % 4096`、`allocate_page` set_len、空文件写头）、`tests/drop_table_free_test.rs:18-19`（`metadata.len()/4096` 裸除法 helper 需扣头）。
- 头字段候选（Plan 定稿）：magic 8B + format version u32 LE + flags u32（bit0 = 加密，MS12 预留）+ page_size u32（自描述）+ 32B 盐预留区。是否加头 CRC 需 Acceptance 证明必要性——CLAUDE.md 默认禁止新增校验和。
- 旧无头文件直接拒绝而非迁移（路线图"趁零用户落"前提，SNAPSHOT 记录零存量用户）；"文件由新版创建"对应 version 高于当前支持的报错文案。

### 未确认项

- 头精确布局、字节序、是否记录 page_size 并交叉校验——Plan 决策。
- 格式错误退出码：复用 exit 1（General）还是新增分类——Plan 与用户决策。路线图退出码表未含格式专码。
- "非 RTsql 文件"与"文件由新版创建"是否拆成两条消息——Plan。
- `.wal` / `.checkpoint` 是否同批加头——任务文本限定"FileStorage open 校验"，倾向不做；若 MS12 加密需要 WAL 覆盖再议。
- 空文件写头的时机（open 即写 vs 首次落盘前）——Plan。

## 调用链或数据流

打开链（T03 插入点加粗）：

```
CLI execute_command_inner (cli/mod.rs:168, 阶段 1 与信号竞争)
└─ Database::open (database.rs:28)
   ├─ FileStorage::open (file_storage.rs:20)
   │   ├─ create-or-open 读写句柄
   │   ├─ try_lock 独占锁 → DatabaseLocked (exit 4)   ← 已有第 1 道闸
   │   ├─ ★ 格式头校验/初始化插入点（锁之后、metadata 检查处）
   │   └─ file_len % 4096 校验 → PageSizeMismatch      ← 现有第 2 道闸，将被头校验吸收
   ├─ BufferPool::new(100)
   ├─ TableManager::new + open_or_init（catalog bootstrap 或 open，页 0/1）
   ├─ WalWriter::open → <db>.wal（创建于 open，MS10-T02 排序保证晚于锁）
   ├─ RecoveryManager::full_recover（读 .wal + .checkpoint 位点）
   └─ CheckpointManager::new（close() 时写位点、重写截断 WAL）
```

错误映射链：`FileStorage::open` 的 `StorageError` → `open_error_status`（cli/mod.rs:143）→ `Locked→exit 4` / `General→exit 1`，stderr 输出（cli/mod.rs:91-94）。

## 边界与失败路径

- 锁与头的顺序：锁在前（file_storage.rs:31-37）。头校验必须放在锁之后——第二个打开者应看到 `DatabaseLocked` 而非格式错误；`database_file_lock_test.rs` 的顺序语义需为新闸保留（坏文件 + 锁被占 → 仍报锁）。
- 截断文件：`(len-H) % 4096 != 0` 的报错需要比现在更可判读；`PageSizeMismatch` 变体可复用或被新变体替代。
- version 高于支持 → "文件由新版创建"（路线图原文文案方向）；magic 不符 → "不是 RTsql 数据库"。两条路径都发生在任何页解析之前，8192B 垃圾文件的 panic → abort 场景随之消除。
- 相邻发现（不属 T03）：`free_pages: Mutex<Vec<u64>>` 纯内存（file_storage.rs:16,56），open 时置空、无任何重建路径；drop_table 释放的页在重启后泄漏（文件只增不小）。方案 B 的超级页本可顺带持久化 free-list，但那是独立改进，不应进入 T03 范围。

## 测试、验证入口与影响面

基线：636 tests pass / 0 failed / 2 ignored（2026-09-08，MS10-T02 后）。

直接受影响（方案 A 预估）：

- `tests/storage_test.rs:10-17`——`to_offset(4096) == 20480`、`to_offset(0) == 0` 两个偏移数学单测，按新语义更新（TDD：先改期望观察 RED）。
- `tests/storage_test.rs:110-117`——首分配页 = 0、page_count 递增语义：方案 A 不变，应保持绿色。
- `tests/drop_table_free_test.rs:18-19`——`page_count(path) = metadata.len()/4096` 裸除法 helper，需扣头。
- `tests/file_storage_io_test.rs:65-76`——越界读 UnexpectedEof 语义：偏移平移后需保持（文件含头 1 页时读 PageId(3) 仍 EOF）。
- `tests/database_file_lock_test.rs`——锁优先顺序守卫，建议补"锁被占 + 坏文件 → DatabaseLocked"场景。

新增测试入口（Acceptance 候选）：

- 新建 → 立即重开（头 roundtrip、空库含头）。
- magic 不符 / version 过高 / version 过低（如未来降级）各报对应错误。
- 截断文件（`(len-H) % 4096 != 0`）报错可判读。
- 8192B 垃圾文件干净报错不 panic（先 RED 复现 exit 134，再 GREEN）。
- CLI e2e：格式错误 exit 码与 stderr 文案（`target/debug/rtsql ./bad.db "SELECT 1"`）。

验证命令：`cargo test`（全量）、`cargo clippy -- -D warnings`、`cargo fmt --check`、`openspec validate`。实验入口：`target/debug/rtsql <path> <sql>`。

## 关键文件

| 文件 | 角色 |
|---|---|
| `src/storage/file_storage.rs` | 头校验/初始化宿主；open、读写页、allocate/free、锁 |
| `src/storage/page_id.rs` | `to_offset` 唯一偏移公式 |
| `src/storage/page.rs` | `PAGE_SIZE=4096`、`Page::from_bytes` |
| `src/storage/error.rs` | `DatabaseLocked`/`PageSizeMismatch` 等变体，格式错误变体落点 |
| `src/storage/catalog.rs:35-137` | 保留页 0/1 常量与 bootstrap/open |
| `src/database.rs:28-93` | 打开链编排，头校验次序锚点 |
| `src/cli/mod.rs:19-32,143-154` | 退出码表与 open 错误映射 |
| `src/wal/writer.rs:26-38` | `.wal` 命名（with_extension）与无头追加格式 |
| `src/wal/checkpoint.rs:47-67` | `.checkpoint` 16B 位点文件 |
| `tests/storage_test.rs` / `tests/drop_table_free_test.rs` / `tests/file_storage_io_test.rs` | 裸布局断言集中地 |
| `tests/database_file_lock_test.rs` | 锁顺序语义守卫 |
