# SNAPSHOT

> 最后更新：2026-09-10（MS10-T05 + MS11-T01 实施与 docs sync 已 commit（b51985f / 4813374 / 046a76c）；push 待用户触发）
> 同步状态：current

## 项目身份

RTsql — 异步协程驱动的高性能嵌入式关系型数据库。以 Tokio 无栈协程为调度核心，实现轻量、便捷、高效的现代数据库系统。

## 技术栈

- **语言**: Rust 2021 edition
- **构建工具**: Cargo
- **异步运行时**: Tokio (rt-multi-thread, macros, sync, time, net, fs, io-util)
- **SQL 解析**: sqlparser-rs 0.44
- **序列化**: serde + serde_json
- **并发原语**: dashmap, lru, tokio-util
- **CLI**: clap 4 (derive)
- **CSV**: csv 1.4
- **校验**: crc32fast
- **测试框架**: criterion.rs (benchmark) + tempfile + rusqlite (对比)
- **格式化**: rustfmt
- **Lint**: clippy
- **随机数**: rand 0.8

## 关键特性

- **轻量**: 单库静态链接，无外部服务依赖
- **便捷**: API 简洁（open / execute / query），支持内存模式与持久化单文件
- **高效**: 基于协程的异步 I/O、MVCC 无锁读、零拷贝页访问、DashMap 缓冲池

## 主要模块边界

- `src/database.rs` — Database 协调器（含 `close()` 显式落盘，MS07-T01；显式事务 API `begin/commit/rollback/execute_in_tx`，MS07-T04；`checkpoint_manager` 接线 + 公开 `checkpoint()` + `close()` 自动触发，MS07-T05）
- `src/pipeline.rs` — SQL 执行管道入口（含 DML 事务包裹，MS06-T01；用户事务执行路径 `execute_in_tx`/`execute_stage_in_tx`，MS07-T04；6 执行器构造点接线投影 `with_projection`，MS10-T01；`execute_inner`/`execute_in_tx` 多语句在 plan/cache put 前显式拒绝 `Response::Error`，替换 first() 静默截断，MS10-T04；`create_executor_from_plan` Projection 臂——`ProjectionExecutor` 逐行求值派生列，MS11-T01）
- `src/cli/` — CLI 非交互入口（one-shot 主命令 `rtsql <db> <sql>`：clap 参数化；`resolve_db_path` 名称解析——裸名→`$RTSQL_HOME/db/<name>.db`（默认 `~/.rtsql/`）、含 `/` 路径直开；`render` 四格式纯函数（table/json/csv/tsv，TTY 默认 table / 非 TTY 默认 json）；退出码 0/1/2/3/4/5；多语句 `;` 分片逐条执行（每条独立 auto-commit + 顺序渲染 + fail-fast 序号定位 `statement k of n failed`，T01 临时护栏退役），MS10-T04；锁冲突 `DatabaseLocked → exit 4` 映射 + 两阶段 select 优雅停机——open 阶段信号无 close 立即退、执行阶段信号经 `close()` checkpoint 后退，`Signaled(signum)` exit 130/143，`execute_command_inner` 信号 future 可注入，MS10-T02；生命周期子命令 `new/list/schema/dump/restore/import --csv`（`src/cli/lifecycle.rs`：`create_table_sql` DDL 生成器（PK/NOT NULL/UNIQUE 持久化约束渲染）/`sql_literal`/`csv_value` 纯函数 + dump SQL 文本流 / restore 空库前置 + 静默逐条循环（`-` stdin）+ fail-fast 复用 `sql_failure_status` / import 表头双向匹配 + 逐条 auto-commit + affected 输出；仅 `new` 创建父目录；裸名与子命令名冲突时子命令优先，MS10-T05）
- `src/parser/` — SQL 解析 + PlanBuilder；`planner/` 6 模块（mod/query/expression/aggregate/subquery/ddl_dml，`PlanBuilder` 三字段 pub(crate) + 公共 API 零变化，MS07-T03 落地）；query.rs 含 JOIN 表头臂 + `resolve_projection_indices` 投影解析 + 聚合 `input_schema` 统一（MS10-T01）；WHERE 新谓词七臂（IN/BETWEEN/LIKE/IS NULL/NOT 脱糖 + ESCAPE/TRY_CAST 显式拒绝）+ 值表达式 CASE/COALESCE/CAST 转换臂 + `contains_or` 七变体扩展（MS11-T01 Iter000）；SELECT 表达式项顶层 `Projection` 路由（AS 别名 / Display 列名 / 四拒绝面 / 聚合报错保持）+ `building_subquery` 子查询上下文抑制 + `ast.rs` 列提取七变体放行（COALESCE）（MS11-T01 Iter001）；`extract_insert_values` 负数字面量折叠（I040，MS11-T01）
- `src/executor/` — 25 个执行器（Scan / DataScan / IndexScan / IndexScanAll / Filter / Join / Aggregate / Sort / Limit / SemiJoin / AntiJoin / SubqueryEval / Correlated / Insert / Update / Delete / CreateTable / DropTable / DerivedScan / Projection / Having / Predicate / ValueRef / Result 等；InsertExecutor 持有 `Option<Arc<TableManager>>` 走 `write_tuple` 路径，MS07-T01；DataScan 支持 `predicate` 行内谓词过滤与 `scan_cap` 提前封顶，OR/Sort/Aggregate 路径保留原节点，MS07-T06；DataScan 支持后继页预取 `with_prefetch(true)` 显式启用、默认关闭，MS08-T02；扫描/Filter/Sort 执行器 `with_projection` 真投影——谓词与 MVCC 判定后按投影裁剪，`SELECT *` 恒等，MS10-T01；DataScan 替代集合去重——每行恰产出对当前快照的最新可见版本（已提交非墓碑替代者抑制），运行期与恢复后同源，MS10-T02 R6；InsertExecutor 键位不可键控行（NULL/非 Int）落库不入索引（文档化语义，SQLite NULL-PK 先例）+ CreateTableExecutor NOT NULL/UNIQUE 约束透传，MS10-T05；Predicate 三值求值内核 `Ternary`/`evaluate_ternary`（Unknown 折叠为行不匹配，既有形态行为逐字节不变）+ `LikePredicate`/`IsNullPredicate`/`NotPredicate` + `CaseExpression`/`CoalesceExpression`/`CastExpression` 值表达式（owned `evaluate`，`evaluate_ref` 对 String 结果显式报错），新 `projection.rs` `ProjectionExecutor` 对全形状输入行逐项求值派生列（MS11-T01））
- `src/storage/` — BufferPool（DashMap + Miss Semaphore + Per-Page Loading Locks）、AsyncStorage（含 `page_count()`，MS07-T01）、FileStorage（页读写 `FileExt::read_exact_at`/`write_all_at` 位置参数化，每页 1 syscall，MS08-T01；open 即 `try_lock` advisory 独占锁，冲突 → `StorageError::DatabaseLocked`，先于 WAL 打开与恢复，MS10-T02；64B 格式头——open 时初始化（0 字节新库）或分类校验，`NotADatabase`/`NewerFileVersion`/`IncompatibleHeader` 先于页解析与 WAL 触碰，页 I/O 偏移 +HEADER_SIZE 平移（`PageId::to_offset` 纯数学不变），MS10-T03）、DataPage、file_header（64B 布局编解码纯函数 + 私有 HeaderError，`KNOWN_FLAGS_MASK=0`——加密位拒绝至 MS12，MS10-T03）
- `src/storage/catalog.rs` — Catalog（系统表 `__tables` / `__columns` SlottedPage 管理 + 二进制行序列化 + 链式页表 + 保留名常量，MS07-T01）
- `src/storage/btree/` — B-Tree（IndexManager 含 `from_root` 路径 + `root_page_id` 访问器，MS07-T01；`collect_all_pages` 物理页枚举 pub async + visited 防环，MS07-T02；LeafNode、InternalNode、redistribution-first merge；`Key` 32 字节定长比较修尾部零键最小键搜索盲区，MS10-T02 G1；root 变更（分裂/收缩）经 catalog 上下文同步 `__tables` 行，MS10-T02 R5；`collect_all_pages_tolerant` 洞容忍收集（父指针枚举洞 id、不递归、不报错），MS10-T02 R8）
- `src/storage/data/` — TableManager（`async new(bp, storage) -> Result<Arc<Self>>` + `open_or_init` 重建 + 保留名检查 + 跨页 tail 同步，MS07-T01；`drop_table` 物理释放数据/索引页到 free-list + 私有 `collect_data_pages` 链遍历，MS07-T02；`replace_index_manager` 索引换入（恢复后重建），MS10-T02 R8；`create_table_with_constraints` 建库约束持久化通道（旧 `create_table` 签名零变化委托壳），MS10-T05）
- `src/storage/page_format/` — SlottedPage（6B logical_id slot）
- `src/storage/page_visibility.rs` — PageVisibilityInfo（页面级 MVCC 摘要）
- `src/transaction/` — TransactionId（AtomicU64）、TransactionManager（begin/commit/abort 唯一 WAL 源；`record_version` 按表聚合 + 多表回滚含墓碑，MS07-T04）、Snapshot、VersionChain（含 DELETED_TX_ID 墓碑守卫，MS06-T01）、RowLock
- `src/wal/` — WalWriter（含 `rewrite_truncate` 单临界区原地截断）/ Reader（带 LSN 读取；逐帧无歧义解析——歧义偏移先按新格式解析以 CRC 验证为判据、失败 seek 回帧首按旧格式，修复格式嗅探 derail，MS10-T02 T0）/ Buffer / Checkpoint（位点消费 + 九步重写截断，WAL 有界，MS07-T05）/ Recovery（位点过滤 redo + 全部失败显式 `Err`，K05，MS07-T05；位置寻址重放——redo 按记录 `row_id` 写入（slot 已存在跳过/稠密落位校验/未初始化页 init）+ Update 版本链重建 + Delete 墓碑重放，MS10-T02 T0b；`redo_count > 0` 时索引去信任——Update `old_row_id` 经磁盘版本多映射 max-rid 派生 + old_tuple 校验，重放后从最终数据页重建各表 PK 索引（链尾回溯 + 重复 PK 显式报错）经 `replace_index_manager` 换入 + catalog root 写回 + 旧树洞容忍释放，`redo_count == 0` 路径零变化，MS10-T02 R7/R8 D10；Update 重放无键回退——`PkVersionMaps` keyed/keyless 双桶（无键版本按 tuple 原始字节追踪，三处追加点），非 deindexed 分支保留 RedoFailed，MS10-T05）
- `src/network/` — Server（Semaphore 并发限流）、PgProtocol（write_buf 批写 + TCP_NODELAY）、JsonProtocol

## 目录约定

- 源码: `src/`
- 集成测试: `tests/`（含新增 `tests/schema_persistence_test.rs` 8 测试，MS07-T01 落地；新增 `tests/drop_table_free_test.rs` 6 测试，MS07-T02 落地；新增 `tests/explicit_tx_test.rs` 8 测试，MS07-T04 落地；新增 `tests/checkpoint_redo_reduction_test.rs` 9 测试，MS07-T05 落地；新增 `tests/pushdown_test.rs` 15 测试，MS07-T06 落地；新增 `tests/file_storage_io_test.rs` 4 测试 + `tests/prefetch_test.rs` 3 测试，MS08-T01/T02 落地；新增 `tests/cli_test.rs` 12 测试 + `tests/projection_test.rs` 6 测试，MS10-T01 落地；新增 `tests/wal_recovery_large_test.rs` 7 测试 + `tests/btree_scale_test.rs` 5 测试 + `tests/database_file_lock_test.rs` 4 测试（cli_test 增至 17），MS10-T02 落地；新增 `tests/file_header_test.rs` 14 测试 + database_file_lock_test 增至 5 + cli_test 增至 21，MS10-T03 落地；cli_test 增至 25（多语句分片 4 新增 + 1 护栏用例重写），MS10-T04 落地；新增 `tests/keyless_row_test.rs` 4 测试 + cli_test 增至 48（生命周期子命令组 + 无键行落库/恢复/全形状往返），MS10-T05 落地；新增 `tests/expression_e2e_test.rs` 24 测试（predicate_test 23 / planner_test 36 / pushdown_test 16 追加，MS11-T01 Iter000 落地）；新增 `tests/projection_expression_test.rs` 16 测试 + cli_test 增至 54（派生列表头与渲染，MS11-T01 Iter001 落地））
- 单元测试: 文件内 `#[cfg(test)]`（含新增 `src/storage/catalog.rs` 10 单元测试）
- 基准测试: `benches/` (8 套: micro / concurrent / scale / sqlite_compare / single / precise_compare / data_scan / visibility)
- OpenSpec: `openspec/`
- 状态文档: `.claude/docs/`
- 模板: `.claude/docs/templates/`
- 分析: `.claude/analysis/`（按需）
- Runbook: `.claude/runbooks/`（按需）
- Incident: `.claude/incidents/`（按需）
- Legacy carrier: `.claude/legacy/`

## 支持平台

- 当前: Linux x86_64
- AI 平台: Claude Code、Codex、OpenCode

## 仓库现场

- **分支**: master
- **最新 revision**: 046a76c（docs sync：MS10-T05 实施 b51985f + MS11-T01 实施 4813374 + 平台 chore 73d4864；详见 commit log）
- **ahead of origin**: 11 commits（原 7 + MS10-T05 实施 + MS11-T01 实施 + docs sync + 平台 chore）
- **最新 tag**: M11
- **测试**: 769 tests pass, 0 failures, 2 ignored（2026-09-10 MS11-T01 收尾，Plan Review 独立复跑；基线 704 + expression_e2e 24 + projection_expression 16 + predicate/planner/pushdown/cli 追加 25；2 ignored 为信号标定设计项）
- **OpenSpec**: 20 capability specs validate PASS（2026-09-10 归档 ms11-t01 change 后；新增 sql-expression-evaluation，6 Requirement）

## 同步状态

- `current` — 文档与代码一致（MS10-T05 / MS11-T01 实施与 docs sync 已 commit（2026-09-10）；push 待用户触发）

## 权威文档

- 公共规则: `CLAUDE.md`
- 项目模型: `openspec/specs/project-model/spec.md` (Mxx)
- 决策: `openspec/specs/decisions/spec.md` (Dxx)
- 知识: `openspec/specs/knowledge/spec.md` (Kxx)
- 参考: `openspec/specs/references/spec.md` (Rxx)
- 改进: `openspec/specs/improvements/spec.md` (Ixx)
- 任务与路线: `.claude/docs/tasks.md`
- 变更: `openspec/changes/`（当前无活跃 change；归档目录含 MS06-T01 + MS06-T02 + MS06-T03-T04 + MS07-T01 + MS07-T02 + MS07-T03 + ms07-rest + ms08-t01-t02 + ms10-t01-cli-shell + ms10-t02-file-lock-graceful-shutdown + ms10-t03-file-format-header + ms10-t04-multi-statement-execution + ms10-t05-lifecycle-subcommands + ms11-t01-sql-expressions carrier）
- Legacy migration carrier: `.claude/legacy/2026-08-25-openspec-init-migration/`
- 新增能力 spec:
  - `openspec/specs/dml-transaction-lifecycle/spec.md`（MS06-T01 落地）
  - `openspec/specs/plancache-key-normalization/spec.md`（MS06-T02 落地）
  - `openspec/specs/wal-writer-handle-reuse/spec.md`（MS06-T03 落地）
  - `openspec/specs/pipeline-stage-decomposition/spec.md`（MS06-T04 落地）
  - `openspec/specs/schema-persistence/spec.md`（MS07-T01 落地，7 个 Requirement）
- `openspec/specs/drop-table-physical-free/spec.md`（MS07-T02 落地，7 个 Requirement）
- `openspec/specs/planner-module-decomposition/spec.md`（MS07-T03 落地，5 个 Requirement）
- `openspec/specs/ms07-rest-tx-checkpoint-pushdown/spec.md`（MS07-T04/T05/T06 落地，3 个 Requirement：R1 显式事务 / R2 Checkpoint / R3 谓词-LIMIT 下推）
- `openspec/specs/storage-io-optimization/spec.md`（MS08-T01/T02 落地，3 个 Requirement：R1 页 I/O 位置参数化 / R2 零接口零格式变更 / R3 DataScan 预取可选能力默认关闭）
- `openspec/specs/cli-noninteractive-shell/spec.md`（MS10-T01 落地，6 个 Requirement：R1 参数化入口与主命令 / R2 名称解析 / R3 列名表头 / R4 输出格式四态 / R5 多语句护栏 / R6 扫描执行器真投影；MS10-T02 修改：R1 增锁冲突 exit 4 场景 + 新增 Requirement 优雅停机 4 场景；MS10-T04 修改：R1 多语句语义修正 + R5 护栏退役替换为 Requirement「多语句分片逐条执行」6 场景；MS10-T05 修改：R1 子命令分发扩展（六生命周期子命令 + 裸名冲突子命令优先）+ 新增 Requirement 生命周期子命令 new（4 场景）/ list（3）/ schema（4）/ dump 与 restore（7）/ import --csv（9），现 11 个 Requirement）
- `openspec/specs/database-file-lock/spec.md`（MS10-T02 落地，2 个 Requirement：R1 独占锁语义 / R2 生命周期与释放）
- `openspec/specs/wal-recovery-frame-parsing/spec.md`（MS10-T02 落地，2 个 Requirement）
- `openspec/specs/wal-recovery-replay-integrity/spec.md`（MS10-T02 落地，2 个 Requirement；MS10-T05 修改：「重放保持 DML 语义」old_row_id 推导扩展无键行回退（keyless 桶，SHALL NOT RedoFailed）+ 新增场景「无键行 Update 崩溃恢复语义正确」）
- `openspec/specs/database-file-format-header/spec.md`（MS10-T03 落地，4 个 Requirement：R1 头布局与生命周期 / R2 格式错误显式拒绝 / R3 打开顺序守卫 / R4 既有语义零回归）
- `openspec/specs/sql-expression-evaluation/spec.md`（MS11-T01 落地，6 个 Requirement：R1 谓词四件套 / R2 NULL 三值语义 / R3 CASE-COALESCE-CAST / R4 SELECT 派生列 / R5 INSERT 负数字面量 / R6 既有语义零回归）
