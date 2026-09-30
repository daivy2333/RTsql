# Iteration 001 / Cycle 000-initial: `:memory:` 库形态（内存存储后端 + CLI 特判）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T3, T4
- Acceptance gaps: None
- Repair items: None
- Inherited scope: MS26 三任务路线（本 Iteration 只做 T3/T4）；proposal 假设 6 与 design D1/D5/D6；Iteration 000 落定的 `OpenMode` + 三字段 Option 化 + `open_internal` 收敛构造链基线（`iterations/000-readonly-open-mode/000-initial.md` Act Response + accepted Plan Review，2026-09-30）；用户四项决策中与锁/只读相关的边界沿用（本 Iteration 无新决策项）。
- Excluded scope: WAL 字节水位（Iteration 002）；内存库持久化/加密/跨会话共享；stats/sample/profile 对 `:memory:` 的点名拒绝（spec R3 SHALL 列表为六生命周期子命令，其余维持既有 resolve_existing_db 拒绝行为）；`--readonly`/`:memory:` 文档面（随 change 收尾同步）。

**Objective**

`:memory:` 内存库形态端到端可用：lib `Database::open_in_memory()` 经新 `MemoryStorage`（`AsyncStorage` 实现）承载全部页数据，零文件（`.db`/`.wal`/`.checkpoint` 均不产生）；会话内全形态（CRUD/DDL/事务/CTAS/ATTACH/多语句分片）与文件库同语义；`close()`/`checkpoint()` 结构性 no-op；进程退出即弃；CLI 主命令 `:memory:` 特判（`--key`/`--readonly` 组合与生命周期子命令实参点名拒绝 exit 2）；文件库路径零回归。

**Investigation Facts**

- Current Baseline: Iteration 000 收尾工作区（未提交，HEAD 9e0d7e1 之上），全量 1554 tests / 0 failures / 2 ignored（2026-09-30 000-initial.md Act Response M19 分批，Plan Review accepted 采信）。`OpenMode { ReadWrite, ReadOnly }` + `Database::mode`/`is_readonly()` + `wal_writer`/`wal_buffer`/`checkpoint_manager` 三字段 Option 化 + 私有 `open_internal(mode, path, isolation, key)` 收敛构造链已落地（`src/database.rs`）。
- Current-State Evidence（本 Cycle Plan 补查，文件行号对应当前工作区）:
  - 存储抽象：`src/storage/async_storage.rs:5-23`——`AsyncStorage` trait：5 个 async 方法（`read_page`/`write_page`/`allocate_page`/`free_page`/`sync`）+ `page_size()` 默认实现 + `page_count()`。`Database::open_internal` 以 `Arc<dyn AsyncStorage>` 装配。
  - `page_count()` 生产消费方唯一：`src/storage/data/table_manager.rs:156`——`TableManager::new` 以 `page_count() == 0` 判定 `Catalog::bootstrap`（分配 catalog 页 0/1）vs `Catalog::open`。MemoryStorage 新实例 `page_count()==0` → 天然走 bootstrap；除此之外无生产语义约束（`file_storage.rs:529` 一处单测断言除外）。
  - 构造链对 Memory 的现成裁剪：`open_internal` 中 WAL writer/buffer、恢复 + `advance_past` + `attach_index_catalog_contexts`、CheckpointManager 三段全部由 `mode == OpenMode::ReadWrite` 门控——新增 `OpenMode::Memory` 变体后自动落入 None/跳过臂；storage 构造 match（ReadWrite → `FileStorage::open_with_key` / ReadOnly → `open_readonly_with_key` + WAL 谓词）需新增 Memory 臂；`close()`/`checkpoint()` 的 `let Some(manager) = &self.checkpoint_manager else { return Ok(()) }` 对 Memory 结构性 no-op；`is_readonly()` 判 `mode == ReadOnly` → Memory 为假（可写，design D1）。
  - 写路径 WAL-可选：`TransactionManager.wal_buffer: Option<Arc<WALBuffer>>`，begin/commit 均 `if let Some`；全部写执行器与 `alter_table.rs` 以 `Option<Arc<WALBuffer>>` 接收——Memory 模式 None 时事务提交、回滚还原、版本可见性全部由页 + 事务管理器承载（Iteration 000 调查事实 4，覆盖未变化）。
  - CLI 主命令：`execute_main_command`（`src/cli/mod.rs:252` 起）——`(db, sql)` 提取后 `resolve::resolve_db_path(db)`；`resolve.rs:30-35` 含 `/` 直开、否则 `db_dir()?.join(format!("{}.db", arg))`——`:memory:` 现状按裸名解析为 `$RTSQL_HOME/:memory:.db`（今天即静默落盘真实文件）。T4 特判插在 resolve 之前。`--key` 经 `CliArgs.key`（`env = "RTSQL_KEY"`）——flag 与 env 双通道收敛于同一字段，`args.key.is_some()` 即覆盖两通道。
  - 生命周期子命令：`execute_command`（`src/cli/mod.rs:215` 起）match `&args.command`——db 目标子命令 `New{target}`/`Schema{db}`/`Dump{db}`/`Restore{db}`/`Import{db}`/`Delete{db}`；`--readonly` 组合守卫臂（`Some(_) if args.readonly` → Usage）已存在于同一 match 顶部（Iteration 000 偏差 6），`:memory:` 实参守卫可同位叠加。stats/sample/profile 亦带 db 实参但不在 spec R3 SHALL 列表（见 Non-goals）。
  - 回归面：`tests/readonly_session_test.rs`（13 例）+ `tests/database_file_lock_test.rs`（全矩阵）+ `tests/cli_test.rs`（生命周期/主命令矩阵）为 T3/T4 直接回归见证；`tests/attach_session_test.rs`/`cross_db_query_test.rs`/`ctas_test.rs` 为 R4 零特殊化见证。
- Code and Critical Path: T3 变更面 = 新 `src/storage/memory_storage.rs`（`MemoryStorage` 实现 AsyncStorage）+ `src/storage/mod.rs` 导出 + `src/database.rs`（`OpenMode::Memory` 变体 + `open_in_memory()` 入口 + `open_internal` storage match Memory 臂）。T4 变更面 = `src/cli/mod.rs`（`execute_main_command` `:memory:` 特判 + `execute_command` 生命周期实参守卫 + inner 编排内存变体）+ `tests/memory_db_test.rs`（新）。数据流：CLI `:memory:` → `execute_main_command` 特判 → inner 内存变体（`open_in_memory()`，无文件锁，两阶段 select 形态保持）→ `run_sql`（`;` 分片既有）→ `close()`（no-op）。错误路径：`--key`/`--readonly` 组合 → exit 2 Usage；生命周期子命令 `:memory:` 实参 → exit 2；ATTACH 附加库沿 MS20/MS26 既有错误面（`attach_database` readonly 参数取 `db.is_readonly()`=false → 附加库读写独占打开）。

**Implementation Guidance**

- 执行顺序：T3 先行（lib 面可独立验证），T4 依赖 T3 的 `open_in_memory()`。
- MemoryStorage 结构按 design D5：`pages: Mutex<HashMap<u64, Box<[u8; PAGE_SIZE]>>>` + `free_pages: Mutex<Vec<u64>>`；read/write 为页拷贝；allocate 先 free list 后新 id；sync no-op；无格式头（`page_count()==0` → catalog bootstrap）。
- `page_count()` 语义建议镜像 FileStorage 文件长度（单调已分配计数，free 不收缩）——唯一生产消费方是 bootstrap 判定 `== 0`，两选一均满足；非实质，Act 定并在 Response 记录。
- Memory 模式传入 `open_internal` 的 path 参数不被消费（WAL/恢复/checkpoint 构造全被 `mode == ReadWrite` 门挡住）——占位值（如 `Path::new(":memory:")`）非实质。
- 缺页读（`read_page` 未分配 id）的错误变体非实质（FileStorage 越界读为 Io 类；正常路径不可达——BufferPool 只驱逐已分配页）。
- `:memory:` 检测用精确串相等（`db == ":memory:"`），不做大小写/引号变体归一（spec 实参形态即 `:memory:`）。

**Task Contracts**

### T3: `MemoryStorage` 与 `Database::open_in_memory()`（lib 内存库面）

- Requirement/Scenario: in-memory-database R1（lib 打开与全形态）、R2（checkpoint/WAL 无操作 + 事务语义保持）、R4（ATTACH/CTAS 零特殊化 + 文件库零回归）
- Depends on: None（OpenMode/Option 字段基线由 Iteration 000 已交付）
- Targets: 新 `src/storage/memory_storage.rs::MemoryStorage`（`AsyncStorage` 全实现）；`src/storage/mod.rs`（导出）；`src/database.rs::OpenMode`（增 `Memory` 变体）、`Database::open_in_memory()`（新 pub 入口，固定 `IsolationLevel::RepeatableRead`）、`open_internal` storage match 增 Memory 臂
- Current behavior: `:memory:` 无任何承载——`OpenMode` 仅两变体；lib 无内存库入口；`rtsql :memory:` 落盘真实文件（T4 修）。
- Required behavior: ① `MemoryStorage` 全部 5 async 方法 + `page_count` 可用：页读写为内存映射拷贝、allocate 消费 free list 优先、sync no-op、新实例 `page_count()==0`；② `Database::open_in_memory()` 可用——mode=Memory、`wal_writer`/`wal_buffer`/`checkpoint_manager` 全 None、不运行恢复、catalog 走 bootstrap 路径、视图注册表空载入；③ 会话内 CRUD/DDL（含 CREATE INDEX/VIEW/ALTER/TRUNCATE）、显式事务 begin/commit/rollback（回滚后行消失、唯一键占用还原、可重插）、CTAS、ATTACH 文件库（附加库读写独占打开，`db.is_readonly()` 为假）、约束拒绝面与文件库同型；④ `close()`/`checkpoint()` 对 Memory 无任何文件 I/O（结构性：checkpoint_manager 为 None）；⑤ `OpenMode::Memory` 不改变 ReadWrite/ReadOnly 任何既有行为（match 穷尽由编译器保证，编译期暴露遗漏消费点）。
- Preserve: `AsyncStorage` trait 定义零变化（`async_storage.rs` 不改）；`FileStorage` 与既有两模式行为逐字节不变（readonly_session_test/database_file_lock_test 全绿）；`open_readonly[_with_key]`/`open_with_key` 公有签名不变；`readonly_rejection`/`is_readonly()` 语义不变（Memory 不被误拒写）；pipeline 提交边界不触碰（Iteration 002 范围）。
- Forbidden: 不为内存库引入 WAL/checkpoint/恢复任何形态；不实现持久化/加密/跨会话共享；不修改 `resolve_db_path`（T4 在其上游特判）；不动 Iteration 002 水位面（WalWriter 计数器/阈值）。
- Test witness: 新 `tests/memory_db_test.rs` T3 部分先 RED：`open_in_memory` 建表/插入/查询/更新/删除/建索引/建视图全链路；显式事务 BEGIN 写入 ROLLBACK 后行消失且可重插、唯一键占用还原；`checkpoint()`/`close()` 调用成功；drop 后重开为空库。零文件断言：会话前后进程 CWD 与 `RTSQL_HOME`（测试自设 TempDir 时）无新文件。
- GREEN condition: memory_db_test T3 用例全绿 + readonly_session_test/database_file_lock_test/attach_session_test/cross_db_query_test/ctas_test 零回归。
- Verification: `cargo test --test memory_db_test`（退出码 0）；`cargo test --test readonly_session_test --test database_file_lock_test --test attach_session_test --test cross_db_query_test --test ctas_test -- --test-threads=1`（退出码 0）；全量分批在 T4 完成后统一执行（M19 纪律）。
- Stop when: MemoryStorage 与 BufferPool 交互出现文件库不存在的语义分叉（如驱逐写回丢失）；`OpenMode` 扩展暴露 Iteration 000 调查未覆盖的 `mode` 消费点；内存库事务回滚与文件库语义不一致。

### T4: CLI `:memory:` 特判与拒绝面

- Requirement/Scenario: in-memory-database R1（CLI 主命令特判 + 多语句管道 + 进程退出即弃）、R3（生命周期子命令与 `--key` 拒绝面）、R4（ATTACH/CTAS 组合）
- Depends on: T3
- Targets: `src/cli/mod.rs`——`execute_main_command`（`:memory:` 特判：组合拒绝 + 内存打开路径）、`execute_command`（生命周期子命令 `:memory:` 实参守卫）、`execute_command_inner_mode` 或其内存等价变体（`open_in_memory` 打开，保留两阶段 select 形态）；`tests/memory_db_test.rs`（扩展 T4 用例）
- Current behavior: `rtsql :memory: <sql>` 经 `resolve_db_path` 解析为 `$RTSQL_HOME/:memory:.db` 真实文件（静默落盘）；生命周期子命令对 `:memory:` 同样按文件名处理；`--key` 组合尝试打开加密文件。
- Required behavior: ① 主命令 db 实参精确等于 `:memory:` → 不经 `resolve_db_path`，走 `open_in_memory()`（无文件锁、无 resolve）；② `:memory:` + `--key`/`RTSQL_KEY` → 打开前 Usage 点名拒绝 exit 2（无持久化，加密无意义）；③ `:memory:` + `--readonly` → Usage 点名拒绝 exit 2（Plan 裁定：只读模式面向文件库的共享锁语义，只读内存会话无意义——诚实拒绝优于静默忽略，与 Iteration 000 偏差 6 同纪律）；④ 生命周期子命令（new/schema/dump/restore/import/delete）实参为 `:memory:` → 点名拒绝 exit 2、零文件产生（守卫位于路径解析前）；⑤ 多语句 `;` 分片在内存会话按 MS10-T04 既有形态执行；进程退出后无任何文件、重开为空库。
- Preserve: `resolve_db_path` 与全部生命周期子命令对普通路径的行为逐字节不变（cli_test 既有全矩阵零回归）；stats/sample/profile 对 `:memory:` 维持既有 resolve_existing_db 拒绝行为（不在 spec R3 SHALL 列表，不改）；`--readonly` 主命令文件库行为不变（readonly_session_test 零回归）；ATTACH 在内存会话走读写附加库既有路径（`attach_database` readonly 参数传 `db.is_readonly()`=false，零特殊分支）。
- Forbidden: 不为内存库新增退出码映射（复用 Usage exit 2 与既有通道）；不实现 `:memory:` 持久化/`list` 特判（list 无 db 实参，不受影响）；不动 `src/cli/resolve.rs`；不给 ATTACH/CTAS 加内存主库分支。
- Test witness: T4 部分先 RED——CLI e2e：`rtsql :memory: "CREATE TABLE t(id INT PRIMARY KEY, v INT); INSERT INTO t VALUES (1,10),(2,20); SELECT sum(v) FROM t"` 输出 `30` 且 `RTSQL_HOME` 目录与 CWD 零新文件；`:memory:` + `--key pw` exit 2；`:memory:` + `--readonly` exit 2；`:memory:` 作 new/schema/dump/restore/import/delete 实参各自 exit 2 点名拒绝且零文件；重开 `:memory:` 为空库；lib 内存会话 `ATTACH 'file.db' AS f` + `SELECT * FROM f.t` + `CREATE TABLE copy AS SELECT * FROM f.t` 会话内可查。
- GREEN condition: T4 用例全绿 + cli_test/readonly_session_test/attach_session_test/cross_db_query_test/ctas_test 零回归 + 全量分批（M19）GREEN。
- Verification: `cargo test --test memory_db_test --test cli_test -- --test-threads=1`（退出码 0）；`cargo test --test readonly_session_test --test attach_session_test --test cross_db_query_test --test ctas_test -- --test-threads=1`；全量按目标名字典序分批（每批 10 个、批内 `--test-threads=1`、批间顺序执行），各批 0 failed。
- Stop when: `:memory:` 特判与生命周期子命令守卫出现绕过路径（某子命令实参形态未覆盖）；内存会话 ATTACH 附加库打开引入计划外分支；exit 码映射需要新增通道。

**Invariants**

- 文件库全部既有路径（lib 三入口 + CLI 全命令）行为逐字节不变——`OpenMode::Memory` 是纯加性变体。
- 内存库零文件：主文件、`.wal`、`.checkpoint` 均不产生（结构性：无文件路径消费；close/checkpoint no-op）。
- 内存库 `is_readonly()` 为假——只读拒绝面与 ATTACH 只读传递均不触发；写路径经 `Option<WALBuffer>` None 天然降级，无新守卫。
- 会话内语义与文件库同型：约束、错误面（SQLSTATE 分类）、plan cache、事务回滚还原、CTAS/ATTACH 组合。
- exit 码映射不重排：General 1 / Usage 2 / SQL 失败 3 / 锁 4 / 密钥 5。

**Non-goals**

- WAL 字节水位（Iteration 002 T5/T6）；内存库持久化、加密、跨会话共享、`list` 特判；stats/sample/profile 的 `:memory:` 点名拒绝；`--readonly`/`:memory:` 文档面（随 change 收尾同步）；锁等待语义。

**Acceptance**

- in-memory-database R1：lib `open_in_memory()` 全 CRUD/DDL/事务可用且与文件库同语义；CLI `:memory:` 多语句计算管道输出正确、进程退出即弃、零文件。（T3/T4 测试）
- in-memory-database R2：不运行恢复；`checkpoint()`/`close()` 无文件 I/O；事务回滚语义（行消失、唯一键还原、可重插）与文件库一致。（T3 测试）
- in-memory-database R3：六生命周期子命令与 `--key`/`--readonly` 组合对 `:memory:` 点名拒绝 exit 2、零文件。（T4 测试）
- in-memory-database R4：内存会话 ATTACH 文件库跨查 + CTAS 物化进内存主库会话内可查；attach_session/cross_db/ctas 既有测试零修改通过。（T4 测试）

**Verification**

- R1/R2/R4 → `cargo test --test memory_db_test`（退出码 0；场景级断言含零文件与回滚矩阵）。
- R3 → `cargo test --test memory_db_test`（拒绝面用例）+ `cargo test --test cli_test -- --test-threads=1`（生命周期零回归）。
- 受影响边界 → `cargo test --test readonly_session_test --test attach_session_test --test cross_db_query_test --test ctas_test -- --test-threads=1`。
- 全量 → M19 分批纪律（目标名字典序、每批 10 个、批内 `--test-threads=1`、批间顺序执行），各批 0 failed；结论注明分批方式与各批计数。

**Gate 2 Readiness**

PASS——2026-09-30 用户批准 change 计划（Gate 1/Gate 2 经 AskUserQuestion 明确批准，覆盖 Iteration Map 与 design D1/D5/D6；见 000-initial.md Gate 2 Readiness 同源记录）；本 Cycle 的 Current Baseline 引用 Iteration 000 accepted Review 采信的 1554/0/2 全量结论，`:memory:` 新表面（`AsyncStorage` trait、`TableManager` bootstrap 判定、CLI 主命令/子命令臂、`resolve_db_path` 现状）已于本 Cycle 补查（见 Investigation Facts），无新增实质未知项。无 BLOCKED/WAIVED 项。

**Persisted Evidence**

- Mode: none

（验证结论以 Act Response 命令 + 决定性输出 + 退出码承载；零文件见证直接在测试断言内完成。）

**Risks and Notes**

- MemoryStorage 的 `page_count()` 语义（单调 vs 活跃页）非实质——唯一生产消费方是 bootstrap 判定 `== 0`；Act 定并在 Response 记录。
- 内存库缓冲池驱逐写回即 map 写入——无文件库的「脏页 vs 磁盘」二态，语义只强不弱；若 Act 发现与驱逐/刷脏相关的隐藏假设（如 sync 时序依赖），记录即可，不构成契约语义。
- `execute_command_inner_mode` 的内存变体形态（参数扩展 vs 独立函数）非实质，保持两阶段 select 结构即可。
- 文档债：`--readonly`（Iteration 000 Remaining Issue）与 `:memory:` 的 README/rtsql-docs 面随 change 收尾同步，本 Iteration 不处理。
- stats/sample/profile 对 `:memory:` 实参维持既有 not-found 拒绝（exit 1、零文件）——超出 spec R3 SHALL 列表，不扩围。

## Act Response

- Status: reported

**Implemented**

T3（`MemoryStorage` + `Database::open_in_memory`）与 T4（CLI `:memory:` 特判 + 拒绝面）均已实施。产品代码 4 个文件面：

- **新 `src/storage/memory_storage.rs`**（`MemoryStorage`）——`pages: Mutex<HashMap<u64, Box<[u8; PAGE_SIZE]>>>` + `free_pages: Mutex<Vec<u64>>` + `next_page_id: AtomicU64` 实现 `AsyncStorage` 全 5 方法：读/写为页拷贝（`read_page_sync` / `write_page_sync` 承担拷贝，`async` 包装内无 await 点，不跨 await 持锁）；`allocate_page` 先消费 free list、空时 `fetch_add` 取新号，两条路径都写零页；`free_page` 回收即写零页并入 free list；`sync()` no-op；`page_count()` 为单调已分配计数。`Default` 实现为 `clippy::new_without_default` 所需。`AsyncStorage` trait 与 `FileStorage` 零改动。
- **`src/storage/mod.rs`**——`mod memory_storage;` + `pub use memory_storage::MemoryStorage;`。
- **`src/database.rs`**——`OpenMode` 增 `Memory` 变体；新增 `pub async fn open_in_memory()`（固定 `RepeatableRead`，`path` 传占位 `Path::new(":memory:")`）；`open_internal` storage match 增 Memory 臂。既有 `mode == ReadWrite` 三处门控（WAL writer/buffer、恢复 + `advance_past` + `attach_index_catalog_contexts`、CheckpointManager）使 Memory 自动落入 `None`/跳过臂——未新增任何模式分支。`is_readonly()` 仍判 `== ReadOnly`，Memory 为假。
- **`src/cli/mod.rs`**——`const MEMORY_DB: &str = ":memory:"`；`execute_main_command` 在 `resolve_db_path` 之前特判（`--key`/`RTSQL_KEY`、`--readonly` 组合各一条点名 Usage exit 2；否则 `Path::new(MEMORY_DB)` 直入编排）；`execute_command` match 之前加 `lifecycle_db_target` 守卫（六生命周期子命令实参点名拒绝 exit 2，早于 `new` 建父目录）；`execute_command_inner_mode` 增 `in_memory: bool` 参数，open future 三臂（memory / readonly / 既有读写），两阶段 select 与 `open_error_status` 单点不变；`execute_command_inner` 委托壳传 `(false, false)`。
- **新 `tests/memory_db_test.rs`**（15 例：T3 9 + T4 6）。

**Deviations from Plan**

1. **`MemoryStorage` 增 `next_page_id: AtomicU64`**：design D5 的字段集未含它，但缺它既无法在 free list 空时分配新页号，也无法给出 Plan 要求镜像 `FileStorage` 的单调计数（`FileStorage` 侧等价物是 `file_len`）。加性、无行为分歧。
2. **`page_count()` 取单调已分配计数**（Plan Risks 第 1 条交由 Act 定）：非「活跃页数」。唯一生产消费方是 `TableManager::new`（`src/storage/data/table_manager.rs:156`）的 `== 0` bootstrap 判定；活跃页数会让「曾分配后全部释放」的存储被误判为空库而重新 bootstrap。已 grep 确认生产消费方唯一（另有一处 `file_storage.rs:529` 单测断言）。
3. **缺页读用既有 `StorageError::InvalidPageId`**（Plan 标注非实质）：未新增错误面变体，`error-reporting` 分类表不受影响。
4. **`allocate_page` 复用路径显式写零页**（实现指引未明写）：与 `FileStorage::free_page`「回收即写零页」同型，保证「allocate → 首读先于首写读到全零空页」的既有依赖在内存后端同样成立。
5. **CLI 内存臂取「参数扩展」形态**（Plan Risks 第 3 条交由 Act 定）：`execute_command_inner_mode` 加一个 `bool`，而非另立函数体——避免复制两阶段 select 编排。
6. **生命周期守卫置于 `--readonly` 组合守卫之前**（在 match 之前）：`rtsql --readonly schema :memory:` 因此报 `:memory:` 消息而非 `--readonly` 消息，两者同为 exit 2，语义等价。
7. **测试断言三处修正**（均为测试侧，非产品代码）：① 视图用例改用 `SELECT * FROM v`——派生表 / CTE / 视图三通道一致丢弃外层投影（既有行为，见 Issue 候选），不把该行为固化为新要求；② ATTACH 夹具用裸名 `'seed'`（`resolve_db_path` 对无 `/` 实参补 `.db`，写 `'seed.db'` 会变 `seed.db.db`）并预建 `db/`（CLI 不建父目录）；③ 零文件断言由「目录本应为空」改为前后集合比较——工作区存在 9 月 5 日遗留 artifact（见 Remaining Issues），不把工作区整洁度当前提。
8. **零文件见证分工**（Plan 测试见证的落地细化）：lib 侧断言「无新增 `:memory:` 命名文件 + 三 WAL/checkpoint 字段为 `None`」；CLI 侧以 CWD 与 `RTSQL_HOME` 同指独立 TempDir 断言目录零条目。lib 路径不经 `RTSQL_HOME`，而进程 CWD 无法在并行测试中安全改写，故不取 lib 侧目录快照。

**Blocker Handoff**

None.

**Blocker Resolution**

None.

**Self-Review**

- **Critical/Important：0 项未解决。**
- 自检发现并已修复 4 项（全部为测试侧或夹具，不涉及产品语义）：
  1. 视图用例断言 `SELECT n FROM v` 返回单列 → 实际两列。核实为**既有**行为（文件库实跑 `SELECT n FROM v`、`SELECT n FROM (SELECT id,n FROM t) AS d`、`WITH c AS (…) SELECT n FROM c` 三者均返回 `[id,n]`），与内存实现无关；改用 `SELECT *` 并转为 Issue 候选上报。
  2. `t3_creates_no_memory_named_files` 的「前置目录无 `:memory:` 文件」前提失败（工作区有 9 月 5 日遗留 `:memory:.wal`）→ 改为前后集合比较，零文件不变量只约束本会话。
  3. `t3_view_alter_truncate` 的 `INSERT INTO t VALUES (9, 90)` 报 `expects 3 values, got 2`——加列后列数变化，改 `(9, 90, 99)`；该报错同时正面见证 ADD COLUMN 在内存会话生效。
  4. T4 ATTACH 用例两处夹具错误（缺 `db/` 预建、`'seed.db'` 补后缀）→ 修正后 GREEN。
- Gate 3 逐项核对：需求/场景完整（`in-memory-database` R1/R2/R3/R4 全部有测试见证）；不变量未破坏（文件库三入口 + CLI 全命令逐字节不变，由 5 个受影响边界文件 58 例 + `cli_test` 111 例 + 全量零回归三重见证；`is_readonly()` 语义不变；退出码映射未重排）；Forbidden 项核对（未引入 WAL/checkpoint/恢复任何形态、未改 `resolve_db_path`、未改 `AsyncStorage` trait、未碰 Iteration 002 的 WalWriter 计数器/阈值面、未给 ATTACH/CTAS 加内存分支、stats/sample/profile 未扩围）。
- 遗留 Minor：2 项（见 Remaining Issues），均非本次引入，不阻塞。

**Verification Evidence**

定向（退出码为判定）：

1. `cargo test --test memory_db_test` → **15 passed / 0 failed**（T3 9 + T4 6），exit 0。
2. `cargo test --test memory_db_test --test cli_test -- --test-threads=1` → 15 passed + 111 passed / 0 failed / 2 ignored，exit 0。
3. T4 受影响边界：`cargo test --test readonly_session_test --test attach_session_test --test cross_db_query_test --test ctas_test -- --test-threads=1` → 11 / 17 / 17 / 13 passed，0 failed，exit 0。
4. `cargo build` → exit 0；`cargo clippy --lib --bins` → 0 warning，exit 0；`cargo fmt --check` → clean，exit 0。
5. `openspec validate 2026-09-30-ms26-embedded-runtime --strict` → `Change '2026-09-30-ms26-embedded-runtime' is valid`，exit 0。

全量（M19 分批纪律：目标名字典序、每批 10 个、批内 `--test-threads=1`、批间顺序执行；逐批记录 cargo 真实退出码与计数）：

- 批 0 `cargo test --lib --bins -- --test-threads=1` → 340 passed / 0 failed，exit 0。
- 批 1–99（99 个测试目标，每批 10 个）→ 每批 `cargo_exit=0`、0 failed；`memory_db_test` 所在批 15 passed，`cli_test` 所在批 111 passed / 2 ignored。
- 合计 **1569 passed / 0 failed / 2 ignored**（Iteration 000 基线 1554 + 本 Cycle 新增 15；`FAILED BATCHES: none`）。

**Persisted Evidence**

None required——Plan 指定 Mode: none；零文件见证在测试断言内完成，验证结论由上列命令 + 计数 + 退出码承载，无独立证据文件。

**Experience Candidates**

**Issue 候选（范围外实质缺陷，有证据；只报告不落账）**——派生表 / CTE / 视图通道丢弃外层投影清单：

- 现象：`SELECT n FROM (SELECT id, n FROM t) AS d`、`WITH c AS (SELECT id, n FROM t) SELECT n FROM c`、`SELECT n FROM v`（v 为 `CREATE VIEW v AS SELECT id, n FROM t`）三者均返回两列 `["id","n"]` 而非一列。
- 证据：文件库实跑（与内存实现无关），三种形态输出均为 `{"columns":["id","n"],"rows":[[1,10],[2,20]]}`；基表查询 `SELECT n FROM t` 正常返回单列，故问题限于派生表/CTE/视图展开路径。
- 归属：MS25 派生表/CTE 域（`sql-views` R1 的「等价派生表」表述以同型方式成立，故视图 spec 自身未被违反）。本次 Cycle 范围外，登记与否由用户指令 `openspec-experience-recorder` 决定。

**Remaining Issues**

- **文档面欠账（延续 Iteration 000）**：`--readonly` 与 `:memory:` 尚未进入 `README.md` / `README.zh-CN.md` / `rtsql-docs/SKILL.md` 的命令面文档。本 Cycle Non-goals 明确不处理，随 change 收尾同步。
- **非阻塞 Minor 1（归属 Iteration 000，非本次引入）**：`cargo clippy --tests` 有 5 条告警——`tests/readonly_session_test.rs:84/211/291` 三条 `empty line after outer attribute`、`tests/executor_test.rs:838/863` 两条 `unused variable: table_manager`（Iteration 000 Deviation 3 机械改写 `Database { .. }` 字面量后的残留绑定）。本次未修改（计划范围外），是否并入本 Cycle 由 Plan Review 裁定。
- **非阻塞观察（工作区既有，非本次产生）**：仓库根存在 9 月 5 日遗留的 0 字节 `:memory:.wal`（Iteration 000 修复 `executor_test` 的 `WalWriter::open(":memory:")` 副作用之前的产物）。未删除——非本次改动产生的孤儿，删除需用户授权。
- **按 Non-goals 未扩围**：`stats` / `sample` / `profile` 对 `:memory:` 维持既有 `resolve_existing_db` not-found 拒绝（exit 1、零文件），不在 spec R3 SHALL 列表。

## Plan Review

- Review Result: accepted

**Findings**

独立检查（强制项：独立阅读实际代码与 diff，核对 Act Response 声明与代码一致；不重跑已采信验证）：

- 产品代码 4 个文件面全部独立读毕：`memory_storage.rs`（页拷贝/free list 优先 + 复用与回收双路写零/`sync` no-op/`page_count` 单调计数；两 `Mutex` 锁域独立不嵌套，`pop` 返回 owned 后再取页表锁，无死锁面）；`database.rs`（`OpenMode::Memory` 纯加性变体、storage match 第三臂 `Arc::new(MemoryStorage::new())`、`open_in_memory` 占位路径不被消费、`is_readonly()` 判 `== ReadOnly` Memory 为假）；`cli/mod.rs`（`MEMORY_DB` 常量、特判先于 `resolve_db_path`、`--key`/`--readonly` 组合各一条点名 Usage exit 2、`lifecycle_db_target` 精确覆盖 New/Schema/Dump/Restore/Import/Delete 六臂且 List/Stats/Sample/Profile/Completions 返回 `None`——Preserve 项「stats/sample/profile 不扩围」守住、`execute_command_inner` 委托壳 `(false, false)`、inner_mode 三臂 open future 保持两阶段 select 结构）。
- `AsyncStorage` trait 与 `FileStorage` 零改动（diff 确认）；`resolve_db_path` 零改动；pipeline 提交边界与 WalWriter 水位面零触碰（Forbidden 项全守住）；ATTACH 零特殊分支（readonly 参数取 `db.is_readonly()`=false 走既有读写打开）。
- 测试见证核对：`memory_db_test.rs` 15 例（T3 9 + T4 6）与契约 witness 一一对应——CRUD 链路/二级索引等值路由/视图+ALTER+TRUNCATE/事务回滚唯一键还原/无 WAL-checkpoint 通道断言（三字段 `is_none()`）/重开即弃/约束拒绝面/CTAS 独立物化/零文件见证（lib 侧 `:memory:` 前缀集合比较 + CLI 侧 TempDir 目录零条目）/多语句管道/--key 双通道/--readonly 组合/六子命令矩阵/ATTACH+CTAS。
- 偏差分类：偏差 1（`next_page_id` 加性）= 实现必要字段，无行为分歧；偏差 2（单调 page_count）= Plan 预留选择的正确裁定（活跃页计数会使「曾分配后全释放」误重 bootstrap）；偏差 3/4/5 = 非实质；偏差 6（守卫次序）= 两臂同为 exit 2、`:memory:` 文案更具体，等价；偏差 7（测试断言三处修正）= 测试侧校准，其中视图用例的断言失败直接暴露了本 change 范围外的既有静默缺陷（见 Issue 候选）——由 RED 到 Issue 候选的处置正确；偏差 8（零文件见证分工）= 并行测试下 CWD 不可安全改写的合理落地。
- 偏差 7 的派生表/CTE/视图投影丢弃 = Act 独立发现的既有行为，与内存实现无关（文件库实跑证据）；归属 MS25 派生表/CTE/视图展开域，不阻塞本 Iteration Acceptance（in-memory-database R1–R4 全部满足）。

非阻塞 Minor finding（处置均已按 2026-09-30 用户指令归入 Iteration 003 收尾批，不在本 Cycle 修复）：

1. `cargo clippy --tests` 5 条告警（readonly_session_test.rs:84/211/291 空行、executor_test.rs:838/863 Iteration 000 偏差 3 残留未用绑定）→ Iteration 003 T10。
2. `--readonly`/`:memory:` 文档面欠账 → Iteration 003 T7。
3. 仓库根 0 字节 `:memory:.wal`（2026-09-05 旧 executor_test 夹具副作用产物，git 已跟踪；该副作用已由 Iteration 000 偏差 3 消除）→ 属本次改动产生的孤儿，按 Surgical Changes 清理义务归入 Iteration 003 T10（随收尾批次提交删除）。
4. `--readonly`×生命周期子命令组合拒绝缺自动化用例（Iteration 000 Minor 1 延续）→ Iteration 003 T9 + readonly-open-mode delta spec R1 场景增补。

**Issue 候选处置**（2026-09-30 用户指令）：派生表/CTE/视图通道丢弃外层投影（范围外实质缺陷）不落账 `openspec-experience-recorder`，在本 change 内解决——登记为 Iteration 003 T8（delta spec `join-output-shape-resolution` ADDED R5 已随本 Review 写入 change specs），执行序先于 Iteration 002（静默错误结果收口优先于新特性，正确性红线纪律）。

**Acceptance Gaps**

None——in-memory-database R1–R4 全部场景有代码与测试见证对应。

**Evidence**

- 本 Review 新增检查：上述独立代码审读（`memory_storage.rs` 全文、`database.rs`/`cli/mod.rs`/`storage/mod.rs` 本 Iteration 变更面、`memory_db_test.rs` 全文、`lifecycle_db_target` 子命令覆盖矩阵定点核对）。
- 采信 Act Response 验证结论（2026-09-30，用户裁定「测试可信」）：定向 `memory_db_test` 15 passed、cli_test 111 passed / 2 ignored；受影响边界 4 文件 58 例 0 failed；build/clippy(--lib --bins)/fmt clean；全量 M19 分批 **1569 passed / 0 failed / 2 ignored**（1554 + 15）；`openspec validate --strict` 通过。覆盖面未失效，不重跑。

**Follow-up Decision**

既有 Acceptance 全部满足、无阻塞 finding → `accepted`。四条 Minor 与 Issue 候选的处置经 2026-09-30 用户指令归入 Iteration 003（T7/T8/T9/T10），不在本 Cycle 返工。

**Iteration Plan Update**

None——Map 增补 Iteration 003（新 Iteration，非本 Iteration 范围修订）；执行序调整为 000 → 001 → 003 → 002。

**Next Cycle**

None。

**Next Iteration**

`iterations/003-derived-projection-closeout/000-initial.md`（派生基座投影收口 + 变更收尾批，T8/T9/T10/T7）——已按 2026-09-30 用户指令展开，Plan Context 状态 `ready`；执行序先于 Iteration 002。
