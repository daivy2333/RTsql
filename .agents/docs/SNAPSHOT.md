# SNAPSHOT

## 项目身份

RTsql（**v0.2.0**）— 异步协程驱动的高性能嵌入式关系型数据库。以 Tokio 无栈协程为调度核心，实现轻量、便捷、高效的现代数据库系统。

## 技术栈

- **语言**: Rust 2021 edition
- **构建工具**: Cargo
- **构建特性**: `[features]`——`default = ["cli", "network", "crypto"]`；`cli` / `network` / `crypto` 三个可选 feature 分别门控 CLI、network 模块与加密能力；`rtsql` 二进制经 `required-features = ["cli"]` 进出构建图
- **异步运行时**: Tokio (rt-multi-thread, macros, sync, time, net, fs, io-util)
- **SQL 解析**: sqlparser-rs 0.44
- **序列化**: serde + serde_json
- **并发原语**: dashmap, tokio-util（network feature 可选依赖）
- **CLI**: clap 4 (derive/env) + clap_complete 4（cli feature 可选依赖）
- **加密**: argon2 0.6 (Argon2id) + aes-gcm 0.11 (AES-256-GCM)（crypto feature 可选依赖）
- **CSV**: csv 1.4（cli feature 可选依赖）
- **校验**: crc32fast
- **测试框架**: criterion.rs (benchmark) + tempfile + rusqlite (对比)；dev-dependencies 另含 futures / which / libc
- **格式化**: rustfmt
- **Lint**: clippy
- **随机数**: rand 0.8

## 关键特性

- **轻量**：单库静态链接，无外部数据库服务依赖。
- **便捷**：one-shot CLI 与 12 个生命周期/分析子命令（new/list/schema/dump/restore/import/delete/checkpoint/check/stats/sample/profile），含隐藏 shell 补全与本机安装脚本；导入支持 stdin 通道与按 CSV 表头自动建表；锁等待预算可配置（`--busy-timeout` / `RTSQL_BUSY_TIMEOUT` / lib 三通道，默认 5000ms）。
- **错误面可操作**：不支持语句与表达式点名报错；PG 协议错误按类携带 SQLSTATE；CLI 错误输出渲染 SQLSTATE；恢复重放对「WAL 记录引用的表在 catalog 缺失」定向指因。
- **高效**：协程异步 I/O、MVCC 无锁读、零拷贝页访问、DashMap 缓冲池；one-shot 只读会话零工作时跳过 checkpoint。
- **统一分级诊断**：会话内诊断门面（详细程度、观察类别、采集内容三项独立配置）、语句阶段树、等待类别分类、事件与摘要双上限、跨模块比较指标；诊断只观察，不改变 SQL 结果、错误文本与 SQLSTATE。
- **读面代数完整**：OUTER JOIN 三态、DISTINCT、集合运算、非递归 CTE、聚合扩展与嵌套聚合表达式、别名排序、多表别名解析、标量子查询、派生表与集合操作组合、NULL 三值折叠、参数与真值求值。
- **差分与性质测试保护网**：rusqlite 跨引擎差分（语义一致子集双向对比、已知分歧白名单）与结构性质测试（编码往返、B-Tree 插删查、版本链可见性），判定全部位于 `cargo test` 集合内。
- **跨库交互**：ATTACH 会话注册与两段限定名路由、跨库 SELECT 全形态与写入、附加库 DDL/CTAS 边界。
- **嵌入式运行形态**：只读打开模式、`:memory:` 内存库、WAL 字节水位自动 checkpoint、锁等待预算与信号中断。
- **catalog 持久化正确性**：目录行等长原地覆写与变长跨页更新、DDL 语句边界的落盘顺序与提交点、ALTER DROP COLUMN 窄化意图的延迟发布。
- **派生基座投影收口**：FROM 子查询/CTE/视图三通道的外层投影裁剪与未知列点名。
- **DDL 演进与二级索引**：非唯一二级索引（单列/组合，INT 与字符串）、唯一索引、CREATE/DROP VIEW、ALTER TABLE ADD/DROP COLUMN、TRUNCATE、DELETE 未命中零副作用。
- **计划可观测与名字解析严格**：`EXPLAIN` 计划树输出与连接顺序行、JOIN 聚合参数解析与歧义点名、聚合输出形状。
- **查询代价与连接叶序**：`ANALYZE` 会话内统计、schema 版本失效、合格查询的左深代价重排、超过阈值保持书写叶序。
- **约束执行**：NOT NULL 与 INT 列 UNIQUE 强制执行；CHECK、FOREIGN KEY、非 INT UNIQUE、非 INTEGER 主键点名拒绝。
- **写面完整**：子集列清单 INSERT 与 DEFAULT、UPSERT 与 REPLACE、UPDATE 表达式与多列赋值、无 WHERE 全表 UPDATE/DELETE、声明期 DEFAULT 校验。
- **DECIMAL 与 BLOB**：可声明、写入、比较与读回；精度与小数位受限，跨类型比较失败；不进入二级索引键。
- **类型化 API 面与参数绑定**：公共入口返回类型化结果与 SQLSTATE；位置式占位符绑定原生值；可复用预编译语句与行游标按批消费。
- **执行资源契约**：执行器批量接口、等值哈希 JOIN 不整表驻留、有界排序、每语句查询内存预算与超限失败语义。
- **DROP 持久化边界与受控 I/O 失败**：页清零延后到再分配时、崩溃后 DROP 结果可查询、受控写盘失败开关下的两种恢复结果。
- **超页长值透明存储**：超过单页内联容量的行值经溢出页链存取，全写通道、加密与内存库同等可用，无人为长度上限。
- **取消与超时结果约定**：语句 future 不再被驱动是唯一取消事件；提交前取消不提交，提交已完成但结果未送达视为已提交；行可见性由取消域保证，键释放由存储域契约承载。
- **索引写侧原子性**：结构变更、根页发布与反向映射更新构成同一写单元，同一索引写侧串行化，取消或回滚后键全部释放。
- **DDL 目录事务化**：DROP COLUMN 的目录变更延后到提交点应用，被放弃或失败的 DDL 不留目录面变更。
- **空闲页跨重启复用**：释放在崩溃边界成立后才是可分配页；重开库按持久目录重建可分配页集合。
- **释放可分配判定的耐久性前提**：刷盘写失败页保持脏状态；锚点写真正持久化后释放才提升为可分配。
- **索引页归属唯一性**：删除合并兄弟节点时页只释放一次；插入路径沿内部节点下降必然终止。
- **释放路径有界读**：释放路径扫描槽位经有边界检查的读取路径，损坏页不 panic、不 abort。
- **改写窗口读一致性**：DROP COLUMN 改写期间读路径显式拒绝，不按旧列数取值。
- **DML 墓碑放弃窗口**：墓碑写序与 DDL 改写路径同形，被放弃语句的墓碑不抑制前驱版本。
- **索引读路径有界读**：索引树访问经页级判据，未通过时点名损坏页与索引树，不降级为「查不到」。
- **其余页型有界读**：数据页、目录页、恢复面与缓冲池的槽负载读取有边界检查。
- **页写入有界**：落位、删槽与原地改写经同一页级判据，越界时点名该页并回滚事务。
- **写盘失败状态**：被驱逐页内容已落盘或仍在池内可取回；写盘失败按页记录，并在该页后续一次成功写入后移出失败集合。
- **旧格式与 dump 兼容**：按目录行布局代际（G1/G2/G3）支持旧库打开、查询与写入；旧 dump 可恢复到当前库且行数据等价；范围外输入给出定位到语句的拒绝与迁移路径。
- **可选加密**：Argon2id 密钥派生与页级 AES-256-GCM；`--key` / `RTSQL_KEY` 双通道。
- **系统行为**：以 `openspec/specs/` 的行为规格为权威，本节只描述支持范围，不复制行为规格正文。

## 主要模块边界

- `src/database.rs` — Database 协调器：open 家族单点（明文/加密 `open_with_key`、只读 `open_readonly[_with_key]`、`:memory:` `open_in_memory()`、锁等待预算 `open_with_lock_wait`/`open_readonly_with_lock_wait`——FileStorage 构造段专用线程 + oneshot 回传使锁等待期间 open future 让出、`open_with_isolation` RR/RC 默认 RR 逐字节等价 + RC `statement_snapshot` 语句视图）；恢复后事务 id `advance_past(max(WAL 最大 id, 位点水位))` 合并推进；`close()` 与公开 `checkpoint()` 在九步前持有会话闸门并暂停 WAL 刷盘循环；`checkpoint` 返回后恢复循环，`close` 在九步后 `shutdown` 并 join、不恢复循环。零工作守卫三条件（无 WAL 追加、无脏页、`!pending_wal_at_open`）跳过全量 checkpoint 保持不变。显式事务已 `begin` 且未 `commit` 就 `close` 时，库不代替调用方回滚，按默认打开重开后该行仍被查到，这不覆盖「刷盘后丢弃句柄、不调用 `close`」的既有结果。显式事务 API `begin/commit/rollback/execute_in_tx`；视图注册表 `ViewMeta`/`views`（open 载入 kind=View、CREATE/DROP VIEW 同步登记/移除）；schema 版本戳 `schema_version: Arc<AtomicU64>`（与 plan cache 失效同源递增、open 归零、克隆共享）+ 预编译语句面 `prepare`/`PreparedStatement`（`execute`/`execute_in_tx`，自持计划 + 版本戳快照 + 透明 re-plan）+ 查询内存上限 `query_memory_limit: Arc<AtomicU64>`（默认 0 不限制、`set_query_memory_limit` 只影响之后开始的语句、克隆共享）+ 测试失败开关 `io_failpoint`/`arm_io_failure`（类型化 `IoFailPoint::{DataWrite, WalFsync}` 一次性注入、系统调用前失败并自清除，主库 `FileStorage` 与 `WalWriter` 共享单元，`:memory:`/ATTACH/readonly 不波及、未设置时字节行为不变）+ 库属隐式事务回收面 `implicit_tx_guard`/`reclaim_abandoned_implicit_transactions`（`pub(crate)`——语句作用域守卫析构时登记被放弃的 tx id，回收在三个库自有同步点执行：下一条隐式写语句之前、`checkpoint()` 之前与自动 checkpoint 判定之前；回收走既有中止路径，表已不存在时先摘该事务的表级版本条目，中止失败保留登记项由下一触发点重试并点名错误）+ 查询行游标 `open_rows` / `open_rows_with_params` / `open_rows_in_tx` / `open_rows_in_tx_with_params` + 会话表统计 `session_stats: Arc<SessionStats>`（克隆共享，不落盘，关闭再打开为空）
- `src/table_stats.rs` — 会话统计与左深代价：`summarize` 按 `Value` 的相等累计行数、NULL 个数和不同值；`choose_left_deep_order` 在不超过 8 张基表时枚举合法左深叶序，书写顺序溢出或超过 8 张表时决定为不足以重排
- `src/pipeline.rs` — SQL 执行管道入口：parse/plan/execute 三阶段编排 + plan cache（CTAS/INSERT SELECT 等新计划不进 cache、执行后全清；DDL 直包臂执行后经 `bump_schema_version` 单点全清并递增 schema 版本戳）；`create_executor_from_plan` 全执行器构造工厂；DML 事务包裹 + 用户事务 `execute_in_tx`/`execute_stage_in_tx`；多语句计划期显式拒绝；参数执行通道 `execute_with_params`/`execute_in_tx_with_params`（占位符编号 → 个数校验 → 计划面编译 → `materialize_params` 注入/槽位物化 → 执行，含占位符语句不入不消费 plan cache）与预编译计划面 `plan_prepared`/`plan_stage_uncached`（不写缓存）；ATTACH 路由三通道（`plan_stage_routed`/`execute_stage_routed`/`execute_stage_in_tx_routed`，`AttachRoute`/`RoutedTables` 计划期判定、执行期按路由取附加库实例与事务/WAL/缓冲池，附加库扫描用其自身 transaction_manager）+ DML 根事务与四写执行器 `routed_db` 绑定目标库（abort 表键 metadata 本地名）；CTAS 混合臂 `execute_ctas_routed`（目标已存在守卫→物化→`infer_ctas_column_types`→`create_table_with_constraints`→`flush_all()`→隐式事务落行→失败 abort+尽力 `drop_table`/提交失败同收口；事务内 execute 期 0A000）；`InsertSelect` 臂（readonly/显式事务附加目标守卫前置→源完整物化原生 Value 行→按位置映射填全宽行归一既有 Insert/Upsert，单隐式事务失败回滚本语句全部行与索引、主库显式事务复用 tx_id）；直包臂六执行器（CREATE/DROP INDEX、ALTER TABLE、TRUNCATE、CREATE/DROP VIEW；前四者 in-tx 0A000 拒绝先于副作用，CREATE/DROP VIEW 沿 CREATE TABLE 现状）；CREATE INDEX/CREATE VIEW 专用计划臂与视图通道注入（`set_views`/`register_view_definition_tables` 递归注册 + 基表 get_table 42P01 上抛；`register_table` miss 前视图豁免）；`infer_ctas_column_types` `pub(crate)`（CLI import 自动建表复用）；快照参数全构造面穿线（查询/DML/子查询与 Semi/Anti/DerivedScan 重建臂）；错误前缀漏斗——`Execution error: ` 单层唯一赋予；语句级查询内存预算（`execute_stage_routed`/`execute_stage_in_tx_routed` 两叶子构造 `QueryBudget`、`create_executor_from_plan` 新 `budget` 参数全子树与子查询重建臂穿线；`storage_error_sqlstate`/`executor_creation_error` 两单源 helper 接漏斗与七转换臂——超限 `Execution error: query memory limit exceeded` + 53200，其余错误面逐字节不变；InsertSelect/CTAS/DerivedScan 源行「逐行入账、物化完成整段出账」）+ 库属隐式事务的守卫与回收接线（DML 臂与 CTAS 路径两处 `implicit_tx_guard` 作用域守卫，写语句开始前先触发回收）+ 查询游标 `open_rows`（计划成功后先只读拒绝，非查询臂在创建执行器前拒绝，一次拉取对根执行器调用一次 `next_batch`）+ `ANALYZE` 扫描当前快照可见行，`commit_table_stats` 只替换当前表后清缓存并递增 `schema_version`
- `src/diagnostics.rs` — 会话诊断门面：配置面 `DiagnosticsConfig`（`detail` / `modules` / `collect` / `event_limit` / `summary_limit`）、`Collect`（events / stats / trace 三项开关）、`Detail`（Off / Summary / Full，默认 Summary）、`Module`（Pipeline / Wal / Storage / Index）与 `ModuleSet`（All / Only）、`Severity`（Info / Warn / Error）、`Category`（Compute / Lock / Queue / Io / Schedule）；记录面 `Record` / `StageSample` / `StageStat` / `TraceSample` / `Summary` / `WaitTotals`；渲染 `render(summary, detail)`；会话槽 `Diagnostics`（配置下发、语句进入 `enter_statement`、摘要环形保留与读回）。采集接线为 task-local 语句阶段栈（`with_statement_stages` / `open_stage` / `close_stage` / `record_stage`）与等待槽（`WaitSlot` / `FlushWait` / `record_wait` / `record_page_read`，文件页读经 `PageReadTimer` 的 Drop 记账）；`StatementScope` 在语句结束时按成功 / 失败 / 取消三种结局收口。`Summary` 带语句开始时刻、首行等待和交回行数；`segment_metrics` 从保留摘要读出吞吐和 P95，`process_memory_peak_bytes` 在 Linux 上读 `ru_maxrss`。五个旧子指标经 `record_stage` 进入当前语句阶段树。诊断只观察，不改变事务结果、错误文本与 SQLSTATE
- `src/cli/` — CLI 非交互入口：one-shot 主命令（clap 参数化；`resolve_db_path` 裸名→`$RTSQL_HOME/db/<name>.db` 默认 `~/.rtsql/`、含 `/` 路径直开）；`render` 四格式纯函数（table/json/csv/tsv，TTY 默认 table）；退出码 0/1/2/3/4/5；多语句 `;` 分片逐条独立 auto-commit + fail-fast 序号定位 `statement k of n failed`；锁冲突 `DatabaseLocked → exit 4` + 两阶段 select 优雅停机（open 阶段信号立即退、执行阶段经 `close()` checkpoint 后退，`Signaled(signum)` 130/143）；生命周期子命令 new/list/schema/dump/restore/import/delete/checkpoint/check（`create_table_sql` DDL 生成器/`sql_literal`/`csv_value`/`csv_field_guess` 纯函数；`quote_ident` 表名包裹；restore 空库前置、`-` stdin；import 表头匹配/自动建表/逐条 auto-commit）+ 分析子命令 stats/sample/profile（共享 `resolve_existing_db`/`fetch_table_rows`/`select_all_rows`/`render_rows` 骨架）；SQL 事务语句会话接线（`TransactionSession` 三臂分派、错误路径统一回滚、活跃收尾回滚 + stderr 提示 exit 0）；`--key`/`RTSQL_KEY`（空值 exit 2、错钥 exit 5）与 `--busy-timeout`/env 穿线全部开库命令与 ATTACH；`RTSQL_QUERY_MEMORY_BYTES` 查询内存上限 env 通道（解析函数沿 checkpoint 水位通道同构——打开前非法值 exit 2 零库文件、主 mode open 后与 ATTACH 附加库两处应用）；ATTACH 会话面（`AttachSession` 所有权在 `execute_command_inner_mode`、全部退出路径统一 close、关闭失败原执行成功时 exit 1 点名）；table 渲染单元格 >64 字符截断为前 61 + `...`（json/csv/tsv 全值保真）；SQLSTATE 渲染（文本行尾 ` [SQLSTATE xxxx]`、JSON `sqlstate` 字段）；`ExitStatus::Sql` 携带 sqlstate 第二载荷 + `sql_failure_status` 五透传点
- `src/parser/` — SQL 解析 + PlanBuilder（planner 七模块 mod/query/expression/aggregate/subquery/ddl_dml/binding + ast）：占位符模块 `placeholder.rs`（AST 单遍按 SQL 文本序把裸 `?` 编号为 `__param_{i}` 并计数；非裸形态 `?N`/`$N`/`:name` 与 DDL 内占位符点名拒绝，标识位维持 sqlparser 既有 parse 拒绝面）；绑定作用域表（`binding.rs`——FROM/JOIN 臂段按声明限定符登记〔列清单 + Option 类型/可空〕、作用域窗口派生 JOIN 布局段与侧别判定〔段键/声明序唯一事实源〕、`is_outer_reference` 关联参数分类单源、`derived_output_columns` 聚合派生/CTE 列源）；JOIN 表头臂、`resolve_projection_indices` 投影解析、聚合 `input_schema` 统一；WHERE 谓词七臂脱糖（IN/BETWEEN/LIKE/IS NULL/NOT + ESCAPE/TRY_CAST 拒绝）+ 值表达式 CASE/COALESCE/CAST；SELECT 表达式项顶层 Projection 路由 + `building_subquery` 抑制 + `building_from_derived` 独立标志；事务语句三型分类 `classify_transaction_statement` 前置拒绝；标量函数注册表校验（OVER/DISTINCT/FILTER/命名参数/arity 点名）+ Trim/Ceil/Floor 独立变体；INSERT 列清单计划期校验与 `map_insert_values` 重排 + DEFAULT 关键字 + UPSERT 冲突子句解析（目标须 INT 声明 PK 或唯一索引列）+ INSERT SELECT 源编译与目标列映射；约束诚实化（CHECK/FK/方言项、组合/非 INT UNIQUE、非 INTEGER PK 点名拒绝）+ `validate_default_value` 声明期校验/归一 + 重复列名守卫；JOIN 执行器选择启发式（写出的整数 `PRIMARY KEY` 基表扫描等值内连接走排序合并，其余纯等值走 Hash，外连接与非等值走 NLJ）+ `join_column_layout` 组合列布局 + 表别名注册与 `resolve_table_qualifier` 归一化单点（attach 复合键直通）+ 限定列解析四消费点 + ORDER BY 限定键；外连接三态臂（LEFT/FULL 走 NLJ、RIGHT 计划期交换）+ WHERE×JOIN 解禁 + Distinct/SetOperation/CTE 注册表 + `OrderByColumn` 表达式键装配；聚合解析（直出形状安全判定、`aggregate_arg_indices` 两基座计划期解析、`contains_aggregate_expr` 提取-注册、`build_having` 完整表达式、规范名去歧义）；子查询面（`bind_where_scalar_subqueries` 深度优先改写占位、`extract_subquery_table_names` 声明限定符化〔别名优先 + 基表名并列保留〕、`get_subquery_first_column` 多形态臂、IN 子查询 JOIN 计划期拒绝、键列类型感知路由 `pk_type_known_non_int`）；CTAS/LIKE/CLONE 建表臂（输出列名重复拒绝、`CtasColumnTypeConflict` 58000）；ATTACH 表名分段路由消费（两段命中附加库/三段计划期拒绝）+ 附加 FROM 项隐式短名关联名；视图六件 DDL 计划臂（拒绝矩阵、创建期定义验证、`resolve_view_reference` 展开为 DerivedScan、`reject_view_write`）+ TRUNCATE/ALTER 计划臂 + 二级索引名名单注入；错误携带能力名（`UnsupportedStatement`/`UnsupportedExpression`）与 42703/42P01 分类、T6 排序键计划期校验 + 合格查询经 `decide_leaf_order` 调用 `choose_left_deep_order` 更换左深叶序，最外层列序保持书写装配
- `src/executor/` — 32 个执行器 + 计划渲染器（Scan/DataScan/IndexScan/IndexScanAll/IndexRangeScan/Filter/Join/NestedLoopJoin/SortMergeJoin/Aggregate/Having/Sort/Limit/SemiJoin/AntiJoin/SubqueryEval/ScalarSubqueryFilter/Correlated/Insert/Upsert/InsertSelect(无独立执行器,pipeline 归一)/Update/Delete/CreateTable/CTAS(pipeline 混合臂)/CreateIndex/DropIndex/AlterTable/Truncate/CreateView/DropView/DerivedScan/Projection/Distinct/SetOperation/Explain/ValueRef/Result/SingleRow + plan_format）；关键语义：火山 `next()` 逐行 + `with_projection` 真投影（`SELECT *` 恒等）；DataScan 谓词行内过滤/`scan_cap` 封顶/预取可选/替代去重（每行恰产出对当前快照最新可见版本）；Scan 无键表（PK 列非 INT）回退 DataScan 数据页走查（带主键表索引枚举零变化）；AntiJoin NOT IN 三值折叠（右集 NULL 键全局折叠、左键 NULL 行不返回、空集全返回）；ParameterExpression owned 求值真值（TEXT 相关参数）；DELETE 墓碑 slot 化 + `superseder_suppresses` 按删除者提交状态判定（aborted 不抑制/活跃不抑制/已提交抑制整链）；三值求值内核 `Ternary`（Unknown 折叠行不匹配）+ Like/IsNull/Not 谓词 + Case/Coalesce/Cast 表达式；标量函数单点注册表（D3 求值序：参数全求值→错误先传播→NULL 跳过校验；严格类型无隐式转换）；`datetime.rs` 日历数学（Hinnant 算法、DA5 定宽 parse-format、`trunc_ts`、Interval 算术、`coerce_datetime_write` 写入强制解析）+ 日期函数族 + ABS 溢出显式错误/ROUND 饱和对齐；约束执行（NOT NULL 前置零副作用、INT UNIQUE 专属唯一索引端到端 + 恢复两态、`KeyTypeMismatch` 键位类型门）；写入类型门 `ColumnTypeMismatch`（NULL 豁免、FLOAT 升格改写）；`UpsertExecutor` 仲裁三形态（DO NOTHING/DO UPDATE 三形态赋值镜像 UPDATE 写序列含 rekey 与碰撞预检/REPLACE 删后插）+ PK 重复预检；多列 UPDATE（同旧行快照求值、per-assignment 门序、Step 7 按 `pk_assigned` 三分支含 NULL 删旧键与 rekey）；无 WHERE 全表 UPDATE/DELETE（`IndexManager::scan_all` 枚举复用单行逻辑、逐行碰撞隐式事务回滚）；溢出两相装配（DataScan `YieldOverflow`、`find_visible_version` 出锁装配、内联零拷贝保持）；子查询语句级缓存（`HashMap<Vec<(String, Value)>, Vec<Vec<Value>>>`，查在 inject/执行前、存仅在成功 drain 后、错误与多行早退不入缓存；Semi/Anti 命中重建 right_hashmap）+ `extract_param_values` 四执行器 Result 化（未命中外层列点名 `correlated parameter column not found in outer row: {列名}` 58000）；NLJ 三态（LEFT/FULL 未匹配 NULL 补齐、组合行上 predicate 与 Filter 同源；组合行求值把左右行的值移入后再移回，SQL 结果与移入前一致）；Sort 消费表达式键（decorate-sort-undecorate）；Distinct 按输出值去重 NULL 折叠；SetOperation UNION[ALL]/INTERSECT/EXCEPT；Explain 不执行内层；自连接 `output_sides` 侧标志归属（双腿同侧 `InvalidQuery`）；六 DDL 执行器（CREATE INDEX TOCTOU 复检+回填、DROP INDEX 页释放、ALTER ADD/DROP 内隐式事务、TRUNCATE checkpoint-first 两步序、CREATE/DROP VIEW 注册表同步）；`query_budget.rs` 每语句预算（charge 超限回退/release 同字节出账/limit 0 短路 + `value_bytes` 口径与桶插入计费单源）与十算子驻留计费面（Join 右哈希经单源助手〔左行只计当前探测行〕、Sort 全量逐行 + 有界替换出入账、Distinct 幸存行 2×、Aggregate 分组键 + 状态差量、Semi/Anti 换表出账旧表、SetOperation 分支行入账/丢弃行出账、SubqueryEval/ScalarSubqueryFilter 缓存键值、NestedLoop 右行）；哈希 JOIN 两阶段 BuildRight → Output（`Executor::next_batch` 默认方法与逐行同序）；`SortMergeJoin` 执行写出整数主键的等值内连接；`SortNode.bounded` 有界排序；DDL 改写窗口读拒绝（`ALTER TABLE DROP COLUMN` 在途时读该表点名拒绝，不按旧列数对已窄化行位置取值）；Explain 在有连接顺序决定时于首行后插入 `JoinOrder estimated` 或 `JoinOrder written`
- `src/executor/ddl_commit_window.rs` — 「提交完成 → 目录应用」窗口的受控暂停点（测试支撑）：路径限定 + 一次消费 + 默认不生效；布防后该库下一次 `DROP COLUMN` 语句在提交点之后、目录窄化之前停住，测试放行后继续。句柄只携带放行信号与一个可观察布尔锚点，不携带任何运行身份
- `src/storage/add_column_count_flush_window.rs` — `ADD COLUMN` 列数刷盘返回成功之后、决定提交之前的受控暂停点（测试支撑）：按库路径一次消费、默认不生效。句柄只有放行信号和可观察锚点，不携带运行身份
- `src/storage/add_column_narrowing_wait.rs` — 同表 `ADD COLUMN` 在写目录之前的受控暂停点（测试支撑）：按库路径索引、默认不生效。句柄只有放行信号和进入次数，不携带运行身份
- `src/storage/` — BufferPool（DashMap + miss Semaphore 16 permits + per-page loading locks；锁序 miss_sem→loading_lock→pages.get→clock_hand.read→frame.lock；flush_all collect-then-write；clock 淘汰 RwLock 串行；`has_dirty_pages()` 只读探测；页级可见性 `MIN_CREATE_UNKNOWN = u64::MAX` 哨兵 + `clear_all_visible`/`set_all_visible`）；AsyncStorage（trait：`page_count()` 为必需方法；空闲页生命周期接缝 `inject_free_pages`/`complete_free_release`/`promote_free_releases`/`discard_free_releases` 为带默认实现的 trait 方法，内存后端不参与跨重启复用）；FileStorage（位置参数化页 I/O 每页 1 syscall、advisory 独占/共享锁 `try_lock_with_backoff` deadline 循环〔`LOCK_VISIBILITY_FLOOR` ≈8ms + 默认预算 5000ms、2ms 间隔〕、64B 格式头编解码〔`KNOWN_FLAGS_MASK=FLAG_ENCRYPTED`、32B 盐 + 12B KDF 参数〕、加密 4124B 页步长 `PageCipher`〔AAD=page_id、WAL/checkpoint 保持明文〕、`free_pages` 内存 free-list〔不落盘；释放进 `pending_frees` 待释放集合、释放完成并持久化后才提升进空闲列表、弹出时先写全零页、写零失败页号回退〕、`IoFailPoint` 类型化失败开关与 `write_page` 注入点〔任何字节落盘前〕、数据页读写两相装配〔>4074B 溢出分支：先链后 locator 末步落位、失败释放已分配页〕）；`StorageError` 家族：DatabaseLocked/NotADatabase/NewerFileVersion/IncompatibleHeader/KeyTypeMismatch/InvalidDateTime/NullConstraintViolation/ColumnTypeMismatch/ExecutionError/QueryMemoryExceeded（Display 无前缀，后者 SQLSTATE 53200 由 pipeline 漏斗分类）
- `src/storage/catalog.rs` — Catalog：系统表 `__tables`/`__columns` SlottedPage 管理 + 二进制行序列化 + 链式页表 + 保留名常量；列行 DEFAULT 段（`u8 has_default | tag | payload`，>65535B 走 `[0x08][u32 len]` 长形式）+ COL_TAG_DATE 0x05/TIMESTAMP 0x06；表行尾随段：唯一索引根 / 二级索引（`u16 name_len | name | col_index | root`；任一索引世代非 0 或有额外列时再追加世代与列序尾段，否则单列行长度不变）/ kind（0x01 Table / 0x02 View 携 sql）/ `explicit_primary_key` 字节向后兼容读；`update_slot_in_place` 等长原地更新 + `update_field_in_chain` 两趟（变长跨页追加 + 旧副本清理、失败携表名传播）+ `scan_tables` 同名去重（logical_id 最大者）+ `update_unique_index_root`/`update_secondary_index_root` ordinal 写回 + `insert_view_row`/`scan_views` + `catalog_column_row_fits` 容量预检
- `src/storage/free_page_rebuild.rs` — 空闲页重建的受控页读失败注入点（测试支撑）：路径限定 + 一次消费 + 默认不生效；布防后该库下一次空闲页重建在首次页读前中止，整次重建被放弃（打开仍成功、活表可读、本次打开无可分配页）。不布防时该模块对重建路径零影响
- `src/storage/btree/` — B-Tree：IndexManager（`from_root` 路径 + `root_page_id`、`search`/`search_all_from_page_async` 闭区间等值下探、`scan_range` 按 32 字节键字典序做范围行走、`delete_exact` 反向精确清理、`scan_all` 任意层树全量枚举〔内部页感知、不依赖叶链〕、`collect_all_pages[_tolerant]`）；LeafNode/InternalNode（redistribution-first merge、`insert_separator` 分隔键落位、`shift_slots_right` 叶链指针保真、`LeafNode::delete` 精确键验证）；Key 32 字节定长（比较修尾部零键盲区；字符串二级索引键在该宽度内派生，完整编码超出容量的字符串不截断，等值查询扫描完整值；新建单列 INT 二级索引与组合键的 INT 分量使用翻转符号位的 8 字节，升级前的单列 INT 索引仍用原大端字节）；root 变更经 `CatalogRootSlot`（Pk/Unique/Secondary 三槽位）同步 catalog；IndexManager 写侧**写单元**（结构变更 + 根页 id 发布 + 反向映射更新落在同一 `spawn_blocking` 闭包内，语句被取消不留半写状态；同一索引的结构变更经写侧闸门 `write_gate` 串行化，并发写者不交错改页、不用过期根覆盖已发布根；反向映射容器改同步 `RwLock`——写单元内同步访问且不跨 await 持锁，公开 `find_key_by_row_id` 签名不变；写侧暂停点夹具 `arm_write_pause` 仅测试装载、未装载零行为差异）；索引页归属唯一性（删除路径合并两个兄弟节点时被合并掉的节点页只释放一次，同一页号不进待释放集合两次；插入路径沿内部节点下降必终止——下降回到已访问过的节点页时插入返回错误，不耗尽栈）；索引树页访问与条目提取走有边界检查的读路径，页级判据不通过时返回点名损坏页与索引树的明确错误，不降级为「没有这个子树 / 条目」继续下降）
- `src/storage/data/` — TableManager：`async new` + `open_or_init`（按 column_count 裁剪孤儿列行、跳过 kind=View）+ 保留名检查 + 跨页 tail 同步；`create_table_with_constraints`（约束持久化通道 + 首数据页 0x03 初始化）；`TableMeta` 承载 not_null/unique_indexes/secondary_indexes/defaults（open 自 catalog 重建 `from_root`）；`drop_table`/`truncate_table` 物理释放（free-list 回落、三类索引空重建 catalog-first 根写回、溢出链随释放）；`gc_table`（主键索引链，以及数据页上无键链的已提交旧版本；链头、未提交版本和带键墓碑链不回收）；`data_chain_status` 只读链完整性（visited 防环、terminal 对照持久 tail）；`replace_index_manager`/`replace_recovery_indexes` 恢复换入；写路径 `heal_stale_tail` 自愈（内存 tail 落后链 terminal 时沿链前进双写回）；`ALTER TABLE DROP COLUMN` 的目录窄化延后到提交点（`alter_drop_column` 只做守卫检查、布局计算与事务内数据面改写，目录变更以 `PendingColumnDrop` 交回调用方；`apply_column_drop` 在提交后应用——列行先、计数后、再切活布局）；`rebuild_free_page_set`/`collect_live_pages`（重新打开时按持久目录重建本次打开的可分配页集合——目录链 + 每表数据页链 + 槽内 locator 指向的溢出链页 + PK/唯一/二级三类索引树页（持久目录登记的树与内存重建的树取并集），可分配集合 = 页数上界 − 活页集合；任一所有权遍历出错即放弃整次重建，打开仍成功、不注入任何页、绝不把活页判成空闲页）；`ALTER TABLE DROP COLUMN` 改写期间读该表点名拒绝（不按旧列数对已窄化行位置取值）；同表 `ADD COLUMN` 在窄化令牌期间等待且不改该表目录，正常路径发布安装取得令牌时捕获的窄化布局
- `src/storage/page_format/` — SlottedPage（6B logical_id slot、页头 next_logical_id 递增、`update_slot_in_place`、compacting、`add_slot_with_logical_id` 恢复面强制落位 max 饱和推进）；tuple（TAG 0x01–0x07 五类型 + DATE 0x06/TIMESTAMP 0x07 + `TAG_LONG_STRING 0x08` u32 长度前缀 + DECIMAL `0x09`（scale 与小端 i128）+ BLOB `0x0A`/`0x0B`、`deserialize_tuple_width` 宽度感知短行补 NULL）；`overflow.rs`（locator `[0xFF][total_len u32][first_page u32]` 编解码、溢出页 page_type 0x04、chunk ≤4080B 分解、链装配读/防环收集/释放原语）
- `src/storage/integrity.rs` — 只读结构完整性校验（`check_integrity` + `IntegrityFinding`/Error/Info 分级）：页 census 0x00–0x04 分类、catalog 解析、逐表数据链 walk（bounds-checked slot 迭代、terminal 对照 catalog tail 分级）、溢出链逐链校验、PK/唯一/二级三类索引树自实现 DFS（DAG re-visit 跳过）、census 溢出页 − 被引用集 = `leaked space` info；损坏输入零 panic，损坏的索引树页按 error 级点名页与索引树
- `src/storage/page_visibility.rs` — PageVisibilityInfo 页面级 MVCC 摘要（min_create_tx_id + all_visible 快路径）
- `src/transaction/` — TransactionId（AtomicU64 单调 + `advance_past` 水位推进）；TransactionManager（begin/commit/abort 唯一 WAL 源，`commit` 在 `CommitTxn` 与 `append_commit_and_wait` 返回之间持有会话闸门、`record_version` 按表聚合 + 多表回滚含墓碑、`abort_cleanup_versions` 两趟——A 趟回退/rekey 语义化回退〔`read_prev_index_keys` 派生键〕、B 趟墓碑还原 `restore_tombstone_index_entries`〔分桶 + 回溯取存活版本 + PK/唯一/二级还原 + NULL-rekey 补口〕）；`IsolationLevel` RR/RC；Snapshot（身份/高水位分离 + `statement_view`）；VersionChain（DELETED_TX_ID 墓碑守卫、`superseder_suppresses`）；RowLock；`TransactionSession`（会话状态机 begin/commit/rollback、双 begin/无事务终结错误即契约）；`abort_by_id`/`discard_tx_version_tables`（按 tx id 中止与摘除表级版本条目，供取消回收路径使用）
- `src/wal/` — WalWriter（`Arc<Mutex<File>>` 持久句柄、`rewrite_truncate` 原地截断、`bytes_appended`/`checkpoint_bytes_threshold` 原子水位）；Reader（LSN 逐帧、新旧格式嗅探以 CRC 为判据）；Buffer（同一时刻只有一个 `do_flush` 写盘；`append_commit_and_wait` 在该次 `write_batch` 返回后唤醒等待者，写盘失败仍返回 `Ok(())`；`shutdown` 刷出缓冲并 join，此后未达容量的 `append` 不再被定时器写走）；Checkpoint（`CheckpointSite` 24B 位点 lsn+timestamp+tx watermark、兼容读三分支、九步重写截断、零工作守卫三条件早退、`tx_watermark` 闭参两处位点写入）；Recovery（位点过滤 redo、失败显式 Err、位置寻址重放〔`row_id` 写入、`add_slot_with_logical_id` 强制落位、溢出链重分配 + locator 幂等、DELETE 墓碑重放〕、`redo_count > 0` 索引去信任——磁盘版本多映射派生 `old_row_id` + PK/唯一/二级三索引重建 + catalog 根写回 + `persist_replayed_tails` tail 写回、`mark_uncommitted_aborted` 未提交行中性化、`build_pk_version_maps` keyless 桶按 tuple 字节追踪、`redo_table_lookup_error` TableNotFound 定向指因、`RecoveryResult.wal_had_tx_records`）
- `src/network/` — Server（Semaphore 并发限流、`shutdown_token`）；PgProtocol（write_buf 批写 + TCP_NODELAY、Date/Timestamp OID 1082/1114 + DA5 文本编码、错误响应按类携带 SQLSTATE）；JsonProtocol

## 目录约定

- 源码: `src/`
- 集成测试: `tests/`（171 个 `tests/*.rs`；分批串行执行约束见 project-model M19）
- 单元测试: 各文件内 `#[cfg(test)]`
- 基准测试: `benches/`（12 套：micro / concurrent / cache / scale / index_key_encoding / sqlite_compare / data_scan / wal_group_commit / tx_id / visibility / buffer_pool_concurrency / pipeline_stages）
- OpenSpec: `openspec/`（行为规格见 `openspec/specs/`）
- 安装脚本: `install.sh`
- 交叉构建脚本: `build-riscv64-musl.sh`（RISC-V 64 musl 固定 target；产物输出 `dist/`，Git 忽略）
- 用户文档: `README.md`、`README.zh-CN.md`、`rtsql-docs/SKILL.md`
- 状态文档: `.agents/docs/`
- 分析: `.agents/analysis/`（按需）
- Runbook: `.agents/runbooks/`（按需）
- Issue: `.agents/issues/`（按需）
- Legacy carrier: `.agents/legacy/`

## 支持平台

- 当前开发与验证: Linux x86_64
- 安装脚本目标: Linux / macOS
- AI 平台: Claude Code、Codex、OpenCode

## 仓库现场

- **分支**: master
- **提交基线**: d428e27b6baebf30d9dde1ecd79601c76f1c208d
- **ahead of origin**: 39
- **工作区**: 干净。`openspec/changes/` 下无活跃 change。最新归档 `openspec/changes/archive/2026-11-24-ms54-dirty-page-batch-io/`。
- **最新 tag**: 无

## 权威文档

- 公共规则：`AGENTS.md`。
- 任务与路线：`.agents/docs/tasks.md`。
- 项目规范：`.agents/memory/project-model.md`（Mxx）。
- 检索元数据：`.agents/memory/references.md`（Rxx）。
- 未承诺方向：`.agents/memory/improvements.md`（Ixx）。
- 行为规格：`openspec/specs/`。
- 变更：`openspec/changes/`（当前无活跃 change；已完成 change 在 `openspec/changes/archive/`）。
- 历史迁移 carrier：`.agents/legacy/2026-08-25-openspec-init-migration/`。
