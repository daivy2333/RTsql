## Purpose

记录 RTsql 现行的开发规范：用户发布、约束 agent 在本项目工作方式与倾向的规则与不变量。系统当前行为以 `openspec/specs/` 为权威，检索索引（含源码路径与 API 速查）见 `.agents/memory/references.md`，未承诺方向见 `.agents/memory/improvements.md`。条目使用 `Mxx` 编号，递增不重用。

## Requirements

### Requirement: 项目模型可验证

跨模块约束 SHALL 记录范围、不变量、证据和状态。

#### Scenario: 稳定约束登记

- **WHEN** 已验证某项约束影响多个模块或后续变更
- **THEN** 使用递增 M 编号记录分类、范围、不变量、证据和状态

---

<!-- arc: b14f741 --> 4 条已归档 (2026-11-18) — M06（依赖关系与调用链）、M11（API 路径索引）、M12（文件速查索引）、M14（数据库表与目录约定）为检索元数据，已迁往 `references.md` 的 source-index 条目（R83–R86）；原文见 `git show b14f741:.agents/memory/project-model.md`。

## M01: 整体系统架构与数据流

- **分类**: architecture
- **范围**: 整个数据库系统的分层与数据流
- **不变量**:
  - SQL 文本 → 解析（sqlparser）→ 计划（PlanBuilder，计划缓存只用于 SELECT）→ 物理计划 → 管道 → 火山执行器树 → 存储（BufferPool → 页守卫 → B-Tree → SlottedPage）
  - 分层依赖方向：入口 → 核心 → {规划, 执行} → {存储, 事务, 持久化}；规划 → 执行；新增代码 SHALL NOT 引入新的跨层环边（现存环与处置约束见 M21）
  - 物理计划节点集合以 `src/executor/plan.rs` 的枚举为准；计划输出行为见 `openspec/specs/explain-plan-output`
- **证据**: `src/database.rs`, `src/pipeline.rs`, `src/executor/plan.rs`, `src/parser/planner/`
- **状态**: active

## M02: 两层分离索引结构

- **分类**: architecture
- **范围**: B-Tree 索引层 + 数据页层
- **不变量**:
  - 索引页存 (key → row_id_pointer)，数据页存实际 row（独立管理）
  - SHALL NOT 采用 SQLite 聚簇索引模式
- **证据**: `src/storage/btree/`, `src/storage/data/table_manager.rs`（data_page_head）
- **状态**: active

## M03: 固定 32 字节 Key

- **分类**: architecture
- **范围**: B-Tree Key 编码
- **不变量**: B-Tree Key 固定 32 字节存储；放不下的完整值 SHALL NOT 截断、SHALL NOT 给列加长度上限（入树与扫描行为见 `openspec/specs/index-key-encoding-evaluation` 与 `openspec/specs/sql-secondary-indexes`）
- **证据**: `src/storage/btree/`
- **状态**: active

## M04: SlottedPage 页格式 + Logical Row ID

- **分类**: architecture
- **范围**: 数据页存储格式
- **不变量**:
  - Slot 6B = `{ logical_id: u16, offset: u16, length: u16 }`
  - Header `next_logical_id: u16`（递增分配，永不回收）
  - RowId.slot_id = logical_id（稳定跨 compact）
- **证据**: `src/storage/page_format/slotted_page.rs`（SlottedPageHeader）
- **状态**: active

## M05: 自定义二进制序列化（Tag + Value）

- **分类**: architecture
- **范围**: tuple 字节布局
- **不变量**:
  - Int = [Tag 0x01][8 bytes i64 LE]
  - String = [Tag 0x02][2 bytes len][N bytes UTF-8]
  - Null = [Tag 0x03]
  - Float = [Tag 0x04][8 bytes f64]
  - Bool = [Tag 0x05][1 byte]
  - Date = [Tag 0x06][4 bytes LE]
  - Timestamp = [Tag 0x07][8 bytes LE]
  - 长字符串 = [Tag 0x08][u32 len]；DECIMAL = [Tag 0x09][scale][i128 LE]；BLOB = [Tag 0x0A/0x0B]
  - 格式版本保持 1；`__tables` 与 `__columns` 不因新类型增加字段
- **证据**: `src/executor/value.rs`, `src/storage/page_format/tuple.rs`；类型语义见 `openspec/specs/decimal-blob-types`
- **状态**: active

## M08: AtomicPageId 无锁访问

- **分类**: architecture
- **范围**: B-Tree 根节点访问
- **不变量**: B-Tree 根页 ID 经 `AtomicU64::load(Acquire)` 异步无锁读取；async 路径 SHALL NOT 用 `std::sync::RwLock` 保护根页指针（避免跨 await 死锁）
- **证据**: `src/storage/btree/index_manager.rs`
- **状态**: active

## M09: 异步协程驱动调度核心

- **分类**: runtime
- **范围**: 整个 I/O 与执行模型
- **不变量**:
  - Tokio 多线程 scheduler 作为无栈协程运行时
  - 阻塞 I/O 通过 `spawn_blocking` 隔离（如 BTree 写操作）
  - async 路径不持 `std::sync::Mutex` 跨 `.await`
- **证据**: `Cargo.toml`（tokio features）, `src/storage/btree/btree.rs`
- **状态**: active

## M10: MVCC 可见性语义

- **分类**: domain
- **范围**: 事务隔离
- **不变量**:
  - 隔离级别：Repeatable Read（默认，`Database::open` 行为与历史逐字节等价）+ Read Committed（`Database::open_with_isolation` 语句级已提交视图）；Serializable 非目标
  - 读路径先查页级 `PageVisibilityInfo` 摘要（min_create_tx_id + all_visible），再回退到逐行 VersionHeader 检查
  - 不可见版本沿 `VersionHeader.next_version` 链查找
  - 事务 ID 全局 AtomicU64 单调递增分配，无锁
  - 纯内存优化，崩溃后自动降级为逐行检查（正确性不受影响）
  - 可见性与墓碑行为面见 `openspec/specs/mvcc-tombstone-visibility` 与 `openspec/specs/transaction-isolation-levels`
- **证据**: `src/storage/page_visibility.rs`, `src/transaction/snapshot.rs`, `src/transaction/mod.rs`（IsolationLevel）
- **状态**: active

## M13: 异步执行原则

- **分类**: runtime
- **范围**: async 代码风格
- **不变量**:
  - async 路径不持 `std::sync::Mutex` 跨 `.await`
  - 闭包内禁止 `.await`（FnOnce 非 async 编译期强制）
  - BufferPool 闭包内禁止递归调用其他 BufferPool 方法
  - BTree 写操作通过 `BTree::from_root()` + `spawn_blocking` 隔离
- **证据**: `src/storage/buffer_pool.rs`, `src/storage/btree/btree.rs`
- **状态**: active

## M15: 命名规范

- **分类**: domain
- **范围**: 代码风格
- **不变量**:
  - 模块名：snake_case（buffer_pool、slotted_page）
  - 类型名：PascalCase（PageGuard、WalRecord）
  - 常量：SCREAMING_SNAKE_CASE（MAX_RETRY_COUNT）
  - 布尔值：is_/has_/can_/should_ 前缀
  - 集合：复数形式（pages、slots）
- **证据**: `src/` 现网代码一致性（抽样）
- **状态**: active

## M17: BufferPool 并发模型（DashMap + Miss Semaphore + Per-Page Loading Locks）

- **分类**: architecture
- **范围**: 缓存并发与页加载路径
- **不变量**:
  - `pages: DashMap<PageId, Arc<Mutex<PageFrame>>>`——cache hit 路径 lock-free
  - miss Semaphore（16 permits）约束并发页加载总量，防突发 IO 风暴
  - per-page `loading_locks: DashMap<PageId, Arc<tokio::sync::Mutex<()>>>`——同页并发 miss 仅一次 read_page（miss Sem 只限总量，不保证同页 double-check 正确性）
  - 锁顺序约定：`miss_sem.acquire() → loading_lock.lock() → pages.get() → clock_hand.read() → frame.lock()`；同序获取无环，异序可死锁
  - `flush_all` 采用 collect-then-write（不得持分片读锁跨 await 写页）
  - 淘汰保持 clock_hand `RwLock<Vec<PageId>>` 串行淘汰；公开 API 签名不变
- **证据**: `src/storage/buffer_pool.rs`（pages/vis_map/loading_locks 字段与 get_page）
- **状态**: active

## M18: 恢复路径索引去信任与重建不变量

- **分类**: domain
- **范围**: WAL 恢复 × B-Tree 索引一致性
- **不变量**:
  - BufferPool 页驱逐按 LRU 而非树拓扑刷盘——checkpoint 后的运行期修改使磁盘 B-Tree 同时含洞（已刷盘父页指向未刷盘子页）与孤儿页（已刷盘但不可达），任何 catalog root 同步策略都无法修复
  - `redo_count > 0`（不洁关闭）时恢复路径 SHALL 零消费磁盘索引树：`old_row_id` 由数据页自建的磁盘版本多映射按 `max{rid < record.row_id}` 派生，重放后从最终数据页重建各表 PK 索引（链尾回溯 + 重复 PK 显式报错）并换入
  - `redo_count == 0`（checkpoint-clean 关闭）时磁盘树可信
  - 数据页不受此影响——WAL 位置寻址重放以数据页为权威
  - 触碰驱逐/刷盘路径的性能改动实施前 SHALL 对照本条
- **证据**: `src/wal/recovery.rs::rebuild_pk_indexes`；行为面见 `openspec/specs/wal-recovery-replay-integrity`
- **状态**: active

## M19: 全量测试分批执行约束

- **分类**: runtime
- **范围**: 测试执行与验证命令
- **不变量**:
  - 全量回归验证 SHALL 分批执行：测试目标（`--lib`/`--bins` 与各集成测试文件）按确定性顺序分组，每组一次独立的 `cargo test … --test … -- --test-threads=1` 调用，批间顺序执行、同一时刻只有一批在跑；SHALL NOT 用一次无目标过滤的全量调用跑完（无论串行还是默认并行度），SHALL NOT 用默认并行度跑任何全量批次
  - 原因：测试规模已过千，无过滤的全量调用要一次性构建链接全部测试二进制并长时间独占机器，已超出目标开发机的承载能力；批内保持 `--test-threads=1` 则延续隔离理由——并行执行时大量用例同时持有库文件、跑大 WAL 构建与子进程信号用例，内存与进程余量不足
  - 批大小按机器能力调整（经验值 8–12 个目标/批）；分组顺序固定（如目标名字典序），保证覆盖可核对、不重不漏
  - 单次目标验证（`--test <name>`、单独 `--lib` 等非分批运行）同样使用 `--test-threads=1` 串行执行；SHALL NOT 用默认并行度跑任何测试调用（含单元测试与定向运行）
  - 新增测试 SHALL 保持「单用例自带独立 TempDir 现场」形态，不依赖并行度，也不得引入对执行顺序、并发度或批次边界的假设
  - 验证记录中的全量结论 SHALL 注明分批执行方式并保留各批的 passed/failed 计数；全量 Gate 等价于「全部批次各自 0 failed」，缺一批不得声明全量通过
- **证据**: 2026-09-29 用户指令（全量单次串行负担超过本机能力，改为分批；豁免由 Act 直接修订本条）；溯源 2026-09-26 用户指令（禁并行跑全量）；执行配方见 `.agents/runbooks/m19-batched-full-regression.md`（R48）
- **状态**: active

## M20: 范围外缺陷的承载闭环约束

- **分类**: domain
- **范围**: 实施与探索中的缺陷发现、报告与处置
- **不变量**:
  - 实施或探索中发现的、当前 change 范围外的实质缺陷，SHALL 走完整承载闭环——登记 Issue（openspec-experience-recorder）→ 建 change（openspec-plan）→ 实施修复（openspec-act）→ 收尾合并规格（openspec-docs-maintainer）；SHALL NOT 只做等级分类（严重度、优先级）后搁置
  - 等级分类只决定承载顺序与排期，不构成处置结论；缺陷不因等级低而免于承载
  - 承载落点按缺陷所在层确定：落在存储/索引层的由存储域后续 change 承载，落在 DDL 目录面的由 DDL 目录事务化主题承载；与哪一里程碑相邻排期由 milestone-planner 统一裁定
  - 发现方只报告与移交，不在当前 change 内扩范围修复；当前 change 的验收面只覆盖本 change 范围内的行为
  - 适用边界：本条为人工/常规流程的基线；autopilot goal 驱动自动调度下同一批实施产出的实质缺陷改由当前 change 追加 Iteration 就地承载，见 M23
- **证据**: 2026-10-24 用户指令「对于实施当中发现的所有issue候选，都直接寻找好的承载方式流程进行修复而不是等级」；MS34 实施期间产生的三个 Issue 候选（索引写侧原子性与取消后一致性、DDL 目录事务化）按本条进入承载闭环而非等级分类
- **状态**: active

## M21: 六层公共接口与状态归属契约

- **分类**: architecture
- **范围**: 入口 / 核心 / 规划 / 执行 / 存储 / 事务六层的公共接口面、状态归属与依赖方向
- **不变量**:
  - 六层划分：入口（`cli/`、`network/`、`main.rs`——CLI 进程退出码与四格式输出合同；lib 面 `rtsql::cli::run()` 供 bin 装配；network 的 `Server::new`/`shutdown_token`、`network::protocol::to_response`、PgProtocol/JsonProtocol）、核心（`database.rs`、`pipeline.rs`、`plan_cache.rs`、`result.rs`、`profiling.rs`）、规划（`parser/` 七模块）、执行（`executor/`）、存储（`storage/`、`wal/`）、事务（`transaction/`）；各层对外公共接口面与词表归属的逐项清单见归档 change `design.md` §5
  - 入口层消费核心与各层 pub 面，不被任何层引用（network、cli 均为叶子）
  - 会话级可变状态统一归 `Database` 协调器所有（`Arc` 共享，克隆共享全部可变态）
  - `plan_cache` 读写独占 pipeline；`schema_version` 单点递增在 pipeline `bump_schema_version`；克隆共享、open 归零、不持久化
  - `wal_buffer` 是唯一「三层持有入口」的字段，归属仍为 Database
  - `views` 注册表归 Database，planner 持有注入快照副本（非共享 Arc）；`io_failpoint` 打开期定型，主库 FileStorage 与 WalWriter 共享同一单元，ATTACH 附加库各自独立
  - 会话级后台对象（WALBuffer 后台 flush/Notify、WalWriter 句柄与水位、TransactionManager 内部登记、BufferPool 并发结构、IndexManager 写侧闸门）归属其宿主管理器，生命周期随 `Database` 实例
  - ATTACH 会话状态归入口层 CLI（`AttachSession`）
  - 允许依赖方向：入口 → 核心 → {规划, 执行} → {存储, 事务, 持久化}；规划 → 执行；result 与 profiling 为零环跨切面
  - 现存依赖环保留，处置为拆 crate 前置约束；新增代码不得引入新的跨层环边
  - 词表归属遗留（`Value`、`ColumnType` 同名双定义、`PhysicalPlan`、`datetime`）的调整属 crate 拆分 change 的专项设计
- **证据**: `openspec/changes/archive/2026-10-28-ms42-module-boundaries/design.md` §5（六层接口、状态归属裁定、依赖方向与环清单）；实施后代码事实的逐项核对见该 change 的 `iterations/003-combination-matrix-and-full-regression/000-initial.md` Plan Review
- **状态**: active

## M22: 模块向统一诊断报告信息的接口约定

- **分类**: architecture
- **范围**: 诊断信息的产生、采集与交付边界
- **不变量**:
  - 统一门面：诊断经单一门面模块（`rtsql::diagnostics`）收发；各模块不得新增 `eprintln!`/`println!` 直写出口
  - 数据模型：`DiagnosticRecord { severity, category, message, data }`；分事件记录与性能统计两类；阶段计时沿用 `with_profiling_scope` 的 scope 语义
  - 交付 API：`diagnostics::report(record)`（同步投递，门面决定丢弃/缓冲/输出）与 `diagnostics::scope(name) -> ScopeGuard`；输出决策不得移回模块
  - 职责边界：模块产生自身事件与统计（模块自己持有阶段语义）；门面独占配置（详细程度/观察模块/采集内容三分离）、跨异步上下文关联与输出限量；诊断只观察，不改变事务结果、不替代行为断言
  - 「用户可见 stderr 告警」的有无变化属行为变化，SHALL 由承载 change 的 spec 增量明示，不得借诊断名义静默吸收
  - 错误继续走 `Result` 返回值（错误不是诊断事件）；SQLSTATE 三源分类不变
- **证据**: `openspec/changes/archive/2026-10-28-ms42-module-boundaries/design.md` §6（诊断报告接口约定与现状出口清单）；行为面见 `openspec/specs/unified-diagnostics`
- **状态**: active

## M23: 自动调度实施期缺陷的就地承载约束

- **分类**: domain
- **范围**: autopilot goal 驱动派发实施 change 期间新发现实质缺陷的登记与承载
- **不变量**:
  - 适用边界与 M20 互补：人工/常规流程的缺陷承载按 M20 闭环；本条只覆盖 openspec-autopilot 接收 goal 指令实施某个 change 时、该 change 实施中新发现的实质缺陷（Act/Plan 记录的 Issue 候选；实质缺陷界定沿用公共规则 › Plan 调查，与 M20 同一口径）
  - goal 注入的授权（公共规则 › 阶段边界）覆盖「登记 Issue」的用户指令环节：此类缺陷由 openspec-experience-recorder 直接登记，不等用户单独指令
  - 登记后不为该缺陷另建 change、不推回路线重排；由 openspec-plan 在当前 change 的 Iteration Map 追加一轮新 Iteration 承载修复——发现后追加，不预建；autopilot 按正常状态路由推进该 Iteration 至 Review accepted，随后进入正常收尾
  - 缺陷超出该承载方式（需新需求裁定、修改已批准 Acceptance 或扩大 goal 范围）时，子代理停止并回报用户决定，不自动扩权
- **证据**: 2026-10-28 用户 goal 原话「开始实施，把这个change做完，要求遵守各种规范，尤其是多批次串行测试规范，对于实施当中发现的issue，直接登记后在这个change规划新iter进行解决，另外把这个issue在自动调度时候的规范写到M文档」
- **状态**: active

<!-- arc: ARC-202609241843b --> 2 条已归档 (2026-09-24) → openspec/changes/archive/2026-09-24-ARC-202609241843b/proposal.md
