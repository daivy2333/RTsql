# MS26 嵌入式运行形态 — Design

调查基线：change 创建时 HEAD 0fe6971 工作区（MS21 收尾待入库批次之上），全量 1541 tests / 0 failures / 2 ignored。

## Current-State Evidence（调查事实，Act 直接消费）

1. **锁面**：`FileStorage::open_with_key`（`src/storage/file_storage.rs:60`）以 `read+write+create` 打开、`file.try_lock()` 独占（:71）、锁先于头校验/WAL/恢复；冲突 → `StorageError::DatabaseLocked`。std `File::try_lock_shared` 与 `try_lock` 同批 stable（Rust 1.88），本项目已使用 `try_lock`。
2. **存储抽象**：`AsyncStorage` trait（`src/storage/async_storage.rs`）5 async + `page_count`；`Database::open_with_key` 已以 `Arc<dyn AsyncStorage>` 装配（`src/database.rs:74`）。`TableManager::new` 以 `page_count()==0` 判定 catalog bootstrap（M11）——内存存储天然走 bootstrap。
3. **Database 构造序**（`src/database.rs:68-177`）：storage → BufferPool(100) → TableManager + open_or_init → views 载入 → TransactionManager → WalWriter::open(path) → WALBuffer + flush loop → `set_wal_buffer` → RecoveryManager::full_recover → advance_past → attach_index_catalog_contexts → PlanCache → CheckpointManager。`close()` = `checkpoint()`；MS18 零工作守卫在 `CheckpointManager::checkpoint` 顶部（`write_count==0 && !has_dirty_pages` → `Ok(0)`）。
4. **WAL 可选性**：`TransactionManager.wal_buffer` 为 `Option<Arc<WALBuffer>>`，begin/commit 均 `if let Some`（`src/transaction/manager.rs:89,112`）；全部写执行器（insert/update/delete/upsert）与 `alter_table.rs` 以 `Option<Arc<WALBuffer>>` 接收、`if let Some` 追加——**写路径天然 WAL-可选**。`Database.wal_buffer`/`wal_writer`/`checkpoint_manager` 三字段为非 Option，消费方：pipeline 5 处 `Some(database.wal_buffer.clone())`（:676,:1206,:1221,:1235,:1247）、`alter_table.rs:93`、CLI shutdown（`cli/mod.rs:776`）、CheckpointManager 构造。**无 `database.wal_writer` 外部消费方**。
5. **提交边界**：生产提交点共 3 处——pipeline auto-commit DML（`pipeline.rs:463`，直呼 `transaction_manager.commit`）、CTAS 隐式插入（`pipeline.rs:705`，同型）、显式事务 `Database::commit`（CLI `COMMIT` 语句经 `TransactionSession::commit` → `Database::commit`，`session.rs:67`）。`commit_by_id` 无生产消费方（仅单测）。`Database::commit` 与 pipeline 两处直呼的 `transaction_manager.commit(tx, &buffer_pool)` 语义逐字节等价（前者是后者的委托壳）。
6. **无快照可见性**（只读跳过恢复的正确性前提）：`DataScanExecutor::superseder_suppresses`（`src/executor/data_scan.rs:335-361`）——无快照（RR 默认）时，带 `commit_tx_id` 的版本可见、无 commit 的不可见、墓碑按删除者活跃集判定（只读会话活跃集为空）。可见性判定**不依赖**恢复后的分配器水位。只读者持共享锁期间写者无法获独占锁（flock 互斥），库状态冻结，「最新已提交」恒定 → **只读会话不运行恢复是可见性安全的**。
7. **checkpoint 九步**（`src/wal/checkpoint.rs:150-201`）：零工作守卫 → LSN 捕获（= WAL 文件字节偏移）→ 水位闭包 → flush_all → fsync → 写位点（携水位）→ 追加 `WalRecord::Checkpoint` → `rewrite_truncate`（保留 [lsn..end)）→ 位点归零重写 → `reset_write_count`。干净关闭后 `.wal` **恰含一条 Checkpoint 标记记录**。
8. **WAL Reader**：`WalReader::open(wal_path)` + `read_all`（`src/wal/reader.rs:19,142`）——只读谓词可复用；`.wal` 缺失 → open 报错，调用方先行 `Path::exists` 分支。
9. **死接口**：`WalWriter` 的 `get_checkpoint_threshold`/`set_checkpoint_threshold`/`should_checkpoint`（`writer.rs:156-167`，默认阈值 1000 条记录）全仓库**零消费方**（src+tests）；`get_write_count`/`reset_write_count` 被 MS18 零工作守卫消费，保留。
10. **CLI 面**：主命令 `execute_main_command`（`cli/mod.rs:252`）→ `resolve_db_path` → `execute_command_inner`（open 与 run_sql 两阶段 select 信号竞争）→ `Database::open_with_key(RR, key)` → close。`open_error_status`（:315）：DatabaseLocked → exit 4、InvalidKey → exit 5、其余 General 1。`AttachSession`（:388-434）：`resolve_db_path` + 存在性预检 + `Database::open_with_key(RR, key)`——附加库一律读写独占打开。
11. **写计划形状**（只读拒绝面枚举）：`PhysicalPlan` 写变体 13 个——Insert / Upsert / Update / Delete / CreateTable / CreateTableAsSelect / CreateIndex / AlterTable / DropIndex / Truncate / CreateView / DropView / DropTable。其余变体均为读形状。plan cache 仅缓存 SELECT（写计划不入 cache）。
12. **SQLSTATE 通道**：`Response::Error` 已有可选 `sqlstate` 载荷（MS19 R2），PG/JSON 错误面按类分类。

## 设计决策

### D1 打开模式：构造链扩展，字段 Option 化

- 新 `pub(crate) enum OpenMode { ReadWrite, ReadOnly, Memory }`，`Database` 增 `mode` 字段 + `pub fn is_readonly(&self) -> bool`（ReadOnly 为真；Memory 为假——内存库可写）。
- `wal_writer`/`wal_buffer`/`checkpoint_manager` 三字段 Option 化：ReadWrite 全 Some（现状）；**ReadOnly 与 Memory 全 None**。6 处 pipeline/alter_table 消费点 `Some(database.wal_buffer.clone())` → `database.wal_buffer.clone()`；CLI shutdown `if let Some`。理由：两模式零写入，WAL/checkpoint 通道结构性不存在，杜绝「守卫遗漏导致落盘」类缺陷；且两模式共享同一形态，避免差异分叉。
- 新入口：`open_readonly(path)` / `open_readonly_with_key(path, key)` / `open_in_memory()`，委托私有 `open_internal(mode, path, isolation, key)`（既有 `open_with_key` 为 ReadWrite 壳，**既有公有签名逐字节不变**）。ReadOnly/Memory 跳过 WalWriter/WALBuffer/恢复/CheckpointManager 构造；ReadOnly 仍走 views 载入与 `open_or_init`（只读 catalog）。
- 替代方案（否决）：每模式独立构造函数树（重复 views/catalog 装配代码）；options struct（签名 churn 违背假设 11）。

### D2 只读存储：`FileStorage::open_readonly`

- 新构造：`OpenOptions read(true)`（**无 write、无 create**——缺失文件 NotFound 诚实拒绝）→ `try_lock_shared()`（WouldBlock → `DatabaseLocked`，语义与独占同型）→ 头校验/密钥/KDF/页长度校验复用既有内部分解（`open_with_key` 头校验段提取共享 helper，明文/加密两态行为逐字节不变）。
- `allocate_page`/`write_page`/`free_page` 在只读实例上返回显式错误（`StorageError::Io`-类或新变体随实现定，文案点名 read-only）——结构性兜底，正常路径不可达（恢复跳过 + 写拒绝在更早层）。`sync` no-op。
- WAL 未决谓词（在存储打开后、任何页写入前）：`Path::exists(wal)` 为假 → 通过；否则 `WalReader::read_all` 全记录必须皆为 `Checkpoint` 变体；否则新 `StorageError::WalPendingRecovery(path)`（Display 文案点名「存在未决 WAL，需先以读写模式打开完成恢复」）。CLI 经 `open_error_status` 其他臂 → General exit 1。
- 替代方案（否决）：内存恢复不落盘——驱逐刷脏（`evict_one` 对脏页写盘）使「保证不落盘」需钉住全部重放页或改造驱逐策略，内存无界且改动面大（用户裁定拒绝，2026-09-30）。

### D3 只读拒绝面：pipeline 计划形状守卫

- pipeline 新自由函数 `readonly_rejection(db: &Database, plan: &PhysicalPlan) -> Option<Response>`：`db.is_readonly()` 且 plan ∈ 13 写变体 → `Response::Error { message: "cannot execute <语句类> in a read-only session", sqlstate: Some("25006") }`。挂在 6 个执行入口顶部（`execute` / `execute_stage` / `execute_stage_routed` / `execute_in_tx` / `execute_stage_in_tx` / `execute_stage_in_tx_routed`）——计划期（解析后）、任何写入生效前。
- SELECT/ATTACH/DETACH/事务语句不经此守卫（计划形状为读或语句级分派在 pipeline 之外）。显式事务内写语句经 `execute_stage_in_tx` 同样被拒（事务保持活跃，同既有语句级失败语义）。
- 理由：计划形状是语句语义的权威判据，单点枚举穷尽 13 变体；planner 层守卫需侵入 6 模块且与 attach 路由交织。否决：CLI 层文本分类（lib 消费者不受保护、破坏诚实化纪律）。

### D4 自动 checkpoint：单点挂 `Database::commit` + WalWriter 字节计数

- `WalWriter` 增 `bytes_appended: AtomicU64`（`write_record`/`write_batch` 内 spawn_blocking 返回实际写入字节数后累加）与 `checkpoint_bytes_threshold: AtomicU64`（默认 `DEFAULT_WAL_CHECKPOINT_BYTES = 16 MiB`；0 = 禁用）+ `set_checkpoint_bytes_threshold(&self, u64)` pub API。字节归零与既有 `reset_write_count` 同位（checkpoint 步骤 9）。
- pipeline :463 与 :705 的 `transaction_manager.commit(tx, &buffer_pool)` 直呼改为等价委托壳 `database.commit(tx)`（语义逐字节不变，D5 调查事实 5），使提交边界单点收敛于 `Database::commit`。
- `Database::commit` 尾部追加：`mode==ReadWrite` ∧ 阈值>0 ∧ `bytes_appended >= threshold` ∧ `active_transactions().is_empty()` → 执行 `self.checkpoint()`；checkpoint 失败将错误返回给该提交语句（文案点名 checkpoint failed；提交本身已 WAL fsync，数据不受影响——spec 语义）。活跃事务非空则顺延（不阻塞不失败）。
- 死接口删除：`should_checkpoint`/`get_checkpoint_threshold`/`set_checkpoint_threshold`/`checkpoint_threshold` 字段整体移除（调查事实 9 零消费方），杜绝双触发机制并存。
- 理由：单点挂接覆盖 3 类边界（显式 COMMIT、CLI COMMIT、auto-commit DML/CTAS）；否决 TransactionManager 回调槽（新增异步闭包机器，循环 Arc 风险）与 per-commit `metadata()` syscall（每语句一次文件元数据调用，热路径开销）。

### D5 `:memory:`：MemoryStorage + 构造裁剪

- 新 `src/storage/memory_storage.rs`：`MemoryStorage { pages: Mutex<HashMap<u64, Box<[u8; PAGE_SIZE]>>>, free_pages: Mutex<Vec<u64>> }` 实现 `AsyncStorage`——read/write 页拷贝、allocate 先 free list、page_count 为已分配页数、sync no-op。无格式头（`page_count()==0` → TableManager catalog bootstrap，调查事实 2）。
- `open_in_memory()`：D1 构造序裁剪（无 WAL/恢复/checkpoint）+ `mode=Memory`。事务提交路径经 `TransactionManager` 的 `Option` wal_buffer 天然降级（调查事实 4），版本可见性/回滚还原全部来自页与事务管理器，与会话内文件库同语义。
- `Database::checkpoint()`/`close()`：`mode != ReadWrite` → `Ok(())` 结构性 no-op（不进 MS18 守卫——memory 会话有脏页，守卫会放行九步并触碰位点路径）。
- CLI：`execute_main_command` 对 `db == ":memory:"` 特判——`--key`/`RTSQL_KEY` 存在 → 点名拒绝（exit 2）；否则 `execute_command_inner` 的内存变体（open_in_memory，无文件锁/信号 open 阶段仍保持 select 形态）。生命周期子命令臂（new/schema/dump/restore/import/delete）在路径解析前对 `:memory:` 点名拒绝（exit 2）。
- ATTACH/CTAS：零特殊分支（内存主库为普通 `Database` 实例，路由面复用 MS20 通道）；附加库按会话模式打开（D6）。

### D6 只读 ATTACH 传递

`AttachSession::open` 增会话模式参数：readonly 会话 → `Database::open_readonly_with_key(path, key)`（共享锁 + WAL 未决谓词同主库）；读写/内存会话 → 现状 `open_with_key`。存在性预检保留（错误文案一致）。附加库 `DatabaseLocked`/`WalPendingRecovery` → ATTACH 失败零副作用（MS20 收口语义不变）。

### D7 CLI `--readonly`

`CliArgs` 主命令增 `--readonly` flag；`execute_main_command` 分支：readonly → `open_readonly_with_key`，否则现状。DML/DDL 拒绝经 pipeline Response（sqlstate 25006）→ `sql_failure_status` exit 3（既有 SQL 失败通道，零新映射）。

## Iteration 顺序与依赖

000 只读模式（引入 OpenMode + 字段 Option 化 + 锁/谓词/拒绝面）→ 001 `:memory:`（复用 000 的模式与 Option 字段形态，只增存储后端与 CLI 特判）→ 002 WAL 水位（独立提交路径面，Option 化后的 `Database::commit` 单点挂接）。000 的字段 Option 化是 001 的直接前置；002 与 000/001 无语义耦合但依赖同一基线。

## 风险与边界

- `try_lock_shared` 平台可用性与 `try_lock` 同源（Linux/macOS 目标平台）；ENOLCK 等错误按普通 IO 传播（spec 已锁定）。
- Option 化触及 pipeline 5 处构造点——逐字节等价改写，全量回归兜底。
- 只读会话的 plan cache、可见性快路径（all_visible）行为与读写会话同源，无快照语义冻结态恒正确（调查事实 6）。
- DDL 不写 WAL → 不计入字节水位（水位语义即「WAL 累计追加」；DDL 脏页由 BufferPool 容量与 close checkpoint 兜底）。
