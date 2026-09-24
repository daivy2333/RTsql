# SNAPSHOT

> 最后更新：2026-09-24（maintainer RISC-V musl 交叉构建收尾：change `2026-09-24-riscv64-musl-build-artifacts` 归档——单 Iteration 单 Cycle Plan Review accepted；specs 39→40（新增 riscv64-musl-build，4 Requirement）；新增仓库根 `build-riscv64-musl.sh`（固定 musl target 静态交叉构建）与 `.gitignore` `/dist/`；本轮统一提交一并纳入该 change 实施/收尾与范围外工作流（install.sh bashrc PATH、docs/SKILL.md→rtsql-docs/ 迁移、docs 路径勘误），提交基线 145bba4。前次 2026-09-24：MS17 初版收尾，1101 tests）
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
- **CLI**: clap 4 (derive/env) + clap_complete 4
- **加密**: argon2 0.6 (Argon2id) + aes-gcm 0.11 (AES-256-GCM)
- **CSV**: csv 1.4
- **校验**: crc32fast
- **测试框架**: criterion.rs (benchmark) + tempfile + rusqlite (对比)
- **格式化**: rustfmt
- **Lint**: clippy
- **随机数**: rand 0.8

## 关键特性

- **轻量**: 单库静态链接，无外部数据库服务依赖
- **便捷**: one-shot CLI、9 个可见生命周期/分析子命令、隐藏 shell 补全生成与本机安装脚本
- **高效**: 基于协程的异步 I/O、MVCC 无锁读、零拷贝页访问、DashMap 缓冲池
- **可选加密**: Argon2id 密钥派生 + 页级 AES-256-GCM；`--key` / `RTSQL_KEY` 双通道

## 主要模块边界

- `src/database.rs` — Database 协调器（含 `close()` 显式落盘，MS07-T01；显式事务 API `begin/commit/rollback/execute_in_tx`，MS07-T04；`checkpoint_manager` 接线 + 公开 `checkpoint()` + `close()` 自动触发，MS07-T05；`open_with_isolation`——`IsolationLevel::{RepeatableRead, ReadCommitted}` 打开面参数（默认 RR 逐字节等价）+ `statement_snapshot` RC 语句级已提交视图构造，MS09-T01；MS17-T02——open 恢复后 `advance_past(max(WAL 观测最大 id, 位点水位.unwrap_or(0)))` 合并推进 + `checkpoint()` 传 `|| transaction_manager.current_tx_id()` 水位闭包（pub 签名不变，CLI 优雅停机同源）；MS17-T01——`open_with_key(path, isolation, key)` 核心入口，既有 open/open_with_isolation 委托壳，明文/加密文件统一经 FileStorage）
- `src/pipeline.rs` — SQL 执行管道入口（含 DML 事务包裹，MS06-T01；用户事务执行路径 `execute_in_tx`/`execute_stage_in_tx`，MS07-T04；6 执行器构造点接线投影 `with_projection`，MS10-T01；`execute_inner`/`execute_in_tx` 多语句在 plan/cache put 前显式拒绝 `Response::Error`，替换 first() 静默截断，MS10-T04；`create_executor_from_plan` Projection 臂——`ProjectionExecutor` 逐行求值派生列，MS11-T01；MS09——快照参数全构造面穿线（查询/DML/子查询与 Semi/Anti/DerivedScan 重建臂均接 `Option<Snapshot>`，RC 语句视图经 `statement_snapshot` 下发）+ `NestedLoopJoin` 构造臂，MS09-T01/T02；MS13——`value_to_json` Date/Timestamp 两臂（DA5 字符串形态，Iter000）+ AggregateNode 构造点接线 `group_key_exprs`（Iter001）+ `create_executor_from_plan` SingleRow 臂与 `extract_column_indices` 防御空臂（Iter002））
- `src/cli/` — CLI 非交互入口（one-shot 主命令 `rtsql <db> <sql>`：clap 参数化；`resolve_db_path` 名称解析——裸名→`$RTSQL_HOME/db/<name>.db`（默认 `~/.rtsql/`）、含 `/` 路径直开；`render` 四格式纯函数（table/json/csv/tsv，TTY 默认 table / 非 TTY 默认 json）；退出码 0/1/2/3/4/5；多语句 `;` 分片逐条执行（每条独立 auto-commit + 顺序渲染 + fail-fast 序号定位 `statement k of n failed`，T01 临时护栏退役），MS10-T04；锁冲突 `DatabaseLocked → exit 4` 映射 + 两阶段 select 优雅停机——open 阶段信号无 close 立即退、执行阶段信号经 `close()` checkpoint 后退，`Signaled(signum)` exit 130/143，`execute_command_inner` 信号 future 可注入，MS10-T02；生命周期子命令 `new/list/schema/dump/restore/import --csv`（`src/cli/lifecycle.rs`：`create_table_sql` DDL 生成器（PK/NOT NULL/UNIQUE 持久化约束渲染）/`sql_literal`/`csv_value` 纯函数 + dump SQL 文本流 / restore 空库前置 + 静默逐条循环（`-` stdin）+ fail-fast 复用 `sql_failure_status` / import 表头双向匹配 + 逐条 auto-commit + affected 输出；仅 `new` 创建父目录；裸名与子命令名冲突时子命令优先，MS10-T05；SQL 事务语句会话接线——`run_sql` 每调用新建 `TransactionSession`，循环内分类三臂分派（事务语句成功渲染 `Affected(0)` 直连不经 executor；错误路径统一 `rollback_session` 显式回滚 + `sql_failure_status` 事务上下文后缀二选一；循环正常结束仍活跃 → 回滚 + stderr 提示、exit 0），事务内普通语句走 `execute_stage_in_tx`，MS11-T02；MS15-Rest——`select_all_rows` 行扫描 SQL 经 `quote_ident` 包裹（归一化后转义名 catalog dump 可用、多代 dump/restore 恒等））；MS13——`Command` 加 `stats/sample/profile` 三子命令（`--format` 四态沿用；stats 每列 [column,type,row_count,null_rate,distinct,min,max,p50,p90,p99]——null 率二位小数/空表 100、distinct 精确计数、min/max 全可比类型（Date/Timestamp DA5 字典序=时间序）、分位数仅数值列最近邻秩 + p50 偶数双值平均；sample reservoir sampling（N 默认 10，0/非整数 exit 2，M≤N 全行）；profile top-k 仅 String 列（k 默认 5、1..=20 越界 exit 2，频次降序并列字典序稳定）；三命令共享 `resolve_existing_db`/`fetch_table_rows`/`select_all_rows`/`render_rows` 骨架 + `execute_command_inner` 优雅停机，表缺失经 `sql_failure_status` exit 3、库缺失 General exit 1，`select_all_rows` dump General 语义原样保留；主命令 no-FORM SELECT 可达 `SELECT 1+1` 单行）；MS17-T02——`import_csv` INSERT 构造表名经 `quote_ident` 包裹（含引号字符 catalog 表名 import 可达、裸名逐字节不变，I048）+ `resolve.rs` env 用例合并单 `test_env_resolution_cases`（I041 结构性消除并行竞态，测试基建无产品代码变化）；MS17——全局 `--key`（env `RTSQL_KEY`，空值 exit 2，密钥错误 exit 5）穿线全部开库命令，隐藏 `completions <bash|zsh|fish>` 三值生成子命令；根目录 `install.sh` 提供 prefix/补全/PATH 提示与程序/数据两模式卸载）
- `src/parser/` — SQL 解析 + PlanBuilder；`planner/` 6 模块（mod/query/expression/aggregate/subquery/ddl_dml，`PlanBuilder` 三字段 pub(crate) + 公共 API 零变化，MS07-T03 落地）；query.rs 含 JOIN 表头臂 + `resolve_projection_indices` 投影解析 + 聚合 `input_schema` 统一（MS10-T01）；WHERE 新谓词七臂（IN/BETWEEN/LIKE/IS NULL/NOT 脱糖 + ESCAPE/TRY_CAST 显式拒绝）+ 值表达式 CASE/COALESCE/CAST 转换臂 + `contains_or` 七变体扩展（MS11-T01 Iter000）；SELECT 表达式项顶层 `Projection` 路由（AS 别名 / Display 列名 / 四拒绝面 / 聚合报错保持）+ `building_subquery` 子查询上下文抑制 + `ast.rs` 列提取七变体放行（COALESCE）（MS11-T01 Iter001）；`extract_insert_values` 负数字面量折叠（I040，MS11-T01）；`TxStatementKind`/`classify_transaction_statement`（mod.rs 事务语句三型 + 六类边界子句精确分类，`build_plan` 前置拒绝——lib 非会话路径 session-only/点名文案且不进 plan cache，MS11-T02）；标量函数臂（MS11-T03）——`Expr::Function` 注册名查表（OVER/DISTINCT/FILTER/NULL treatment/ORDER BY/命名参数/通配符/arity 点名拒绝后构造 `FunctionExpression`，未注册名维持既有文案）+ `Expr::Trim`/`Expr::Ceil`/`Expr::Floor` 独立 sqlparser 变体接线（仅纯形态，规格化/TO DateTimeField 形态点名拒绝）；`ast.rs` extract 两门放行注册名与独立变体，未知名维持既有 `UnsupportedStatement`；MS15-T01 键位等值路由修复（I036）——`has_non_keyable_pk_literal_leg` 分类 helper（Eq 腿字面量经 `value_from_sqlparser`→`to_key()` 判可键控性，AND 递归/OR 保守 false）+ `has_pk_eq` 分支条件收窄，不可键控字面量腿（String/Float/Bool/NULL）禁用索引路由、落入既有 OR 臂/谓词下推臂，无键行键位等值可达（形态 2 Int 字面量 + Float 键列残差登记 I046 待独立 change）；MS15-Rest——`get_plan_output_columns` 新增 `projected_columns`（DataScan 臂必需应用 projection、Scan/IndexScanAll 恒等加固、IndexScan 保持构造期收窄，I034 CLI 表头与行形状一致）+ 表名解析归一化 `object_name_to_table_name`（`ast.rs`，`Ident.value` 去引号 + lowercase + `.` 连接）统一 11 处表名消费点，带引号与裸名拼写全语句等价（I039，spec table-name-resolution）；MS16——键列类型感知路由（I046 方向 B）：`PlanBuilder` 加性 `primary_key_types` + `set_pk_column_type`（pipeline 注册点接线），query.rs `pk_type_known_non_int` 两判定门（`extract_pk_from_where` 门 1 + `has_pk_eq` 分支收窄门 2），键列声明非 Int 时键位等值形态（简单/AND/反向）统一回退谓词下推 DataScan/Filter(DataScan) 行内求值（spec planner-key-equality-routing）；`build_insert`（ddl_dml.rs）INSERT 列清单 plan 期校验（恰为表列排列：未知/重复/数量不符拒绝；无清单行长度校验）+ `map_insert_values` 按清单重排（共享 `insert_count_error` 文案），错位/panic/未知列三形态消除（spec insert-column-list-mapping）；MS09——JOIN 执行器选择启发式（spec join-executor-selection）：`is_pure_equi_join_on`/`is_structural_column_ref` 结构探测（ddl_dml.rs 自由函数，AND 分解镜像 `extract_join_conditions` 递归形状、零语义解析）分流——纯等值 ON 保持既有 Hash 臂原样、否则 NLJ 分支；`PlanBuilder` 加性 `join_column_layout` + expression.rs 两列解析臂布局覆盖消费（非限定名全表搜索 0→ColumnNotFound / >1→AmbiguousColumn / 1→偏移+位置；限定名表偏移+表内位置），`build_from_clause_with_projection` 布局 save/restore 严格配对内编译 ON 于组合行绝对索引（左 0..n / 右 n..n+m）；`get_plan_output_columns` NLJ 臂 + `:345`/`:424`/`:507` 匹配点扩展（NLJ → "join_result"、SELECT 表达式项 + JOIN 与 WHERE + JOIN 拒绝对新节点同语义生效）；MS13——`convert_data_type` Result 化（Date/Datetime/Timestamp(None) 显式映射、Tz/Time/Interval 点名拒绝、String 兜底保持）+ TypedString 四入口 plan 期解析（build_expression/build_where/extract_insert_values/UPDATE SET）+ `ast.rs` 放行清单扩展（TypedString/BinaryOp/Interval）+ `BinaryArithExpression` 编译臂（SELECT 投影项与 WHERE 比较腿双侧可达，Iter000）+ `Expr::Interval` 双入口臂与数值分流前探测、sqlparser 吞比较 `unswallow_interval_comparison` 解缠绕（Iter001）+ GROUP BY 四级解析序 `resolve_group_by_item`（列名→别名→表达式文本→位置）+ 混合投影解锁与条件 Projection 包装（直出/包装分形输出列名，Iter001）+ `build_no_from_select` no-FROM 分支（九项拒绝面点名 + 空布局覆盖列引用编译，置于子查询检测前）+ `get_plan_output_columns` SingleRow 臂（Iter002）；MS17-T02——`PlanError::InSubqueryJoinUnsupported`（`get_subquery_first_column` 补 `Join`/`NestedLoopJoin` 显式拒绝臂，IN 子查询 JOIN 形态计划期点名拒绝、不误报多列，spec in-subquery-join-rejection）+ `get_plan_output_columns` SubqueryEval 臂按 `result_column_index` 插入标量列名（CLI 表头与行形状一致，ISS03）
- `src/executor/` — 27 个执行器（Scan / DataScan / IndexScan / IndexScanAll / Filter / Join / NestedLoopJoin / Aggregate / Sort / Limit / SemiJoin / AntiJoin / SubqueryEval / Correlated / Insert / Update / Delete / CreateTable / DropTable / DerivedScan / Projection / Having / Predicate / ValueRef / Result / SingleRow 等；InsertExecutor 持有 `Option<Arc<TableManager>>` 走 `write_tuple` 路径，MS07-T01；DataScan 支持 `predicate` 行内谓词过滤与 `scan_cap` 提前封顶，OR/Sort/Aggregate 路径保留原节点，MS07-T06；DataScan 支持后继页预取 `with_prefetch(true)` 显式启用、默认关闭，MS08-T02；扫描/Filter/Sort 执行器 `with_projection` 真投影——谓词与 MVCC 判定后按投影裁剪，`SELECT *` 恒等，MS10-T01；DataScan 替代集合去重——每行恰产出对当前快照的最新可见版本（已提交非墓碑替代者抑制），运行期与恢复后同源，MS10-T02 R6；InsertExecutor 键位不可键控行（NULL/非 Int）落库不入索引（文档化语义，SQLite NULL-PK 先例）+ CreateTableExecutor NOT NULL/UNIQUE 约束透传，MS10-T05；Predicate 三值求值内核 `Ternary`/`evaluate_ternary`（Unknown 折叠为行不匹配，既有形态行为逐字节不变）+ `LikePredicate`/`IsNullPredicate`/`NotPredicate` + `CaseExpression`/`CoalesceExpression`/`CastExpression` 值表达式（owned `evaluate`，`evaluate_ref` 对 String 结果显式报错），新 `projection.rs` `ProjectionExecutor` 对全形状输入行逐项求值派生列（MS11-T01）；新 `function.rs` 标量函数单点注册表——`REGISTRY` 元数据（十函数 name+arity）同模块驱动 planner 校验入口 `is_scalar_function`/`check_scalar_function` 与 `FunctionExpression` 分派求值（D3 顺序：全部参数求值→错误先传播→任一 NULL→NULL 且跳过类型校验；`evaluate_ref` 物化 owned 后仅 Copy 变体回借；严格类型无隐式转换；string 六件含 substr SQLite 边缘/trim 仅 U+0020，math 四件 abs 同型/round half-away-from-zero+负 digits 整数位/floor-ceil Float，MS11-T03）；UpdateExecutor Step 7 键位分支——SET 键列为不可键控值改 `index_manager.delete` 旧键条目清理（运行期/恢复两态一致，I037，spec update-index-maintenance，MS15-Rest）；MS16——`InsertExecutor` Int 键列键位类型预检（先于 DuplicateKey 预检）+ `UpdateExecutor` 前置块键列类型校验与碰撞预检（新键 `search` 命中即 `DuplicateKey`，任何写入前零副作用）+ Step 7 三分支（NULL 删旧键〔I037 原样〕/ 同键 update〔原样〕/ rekey 先 `delete(old)` 后 `insert(new)`），Int 键列只接受 Int/NULL（`KeyTypeMismatch`，spec key-column-type-conformance；rekey spec update-index-maintenance）；MS09——`NestedLoopJoinExecutor`（右输入物化一次、左输入流式逐行 × 右行逐组合，`left_row ++ right_row` 组合行上 `predicate.evaluate` 与 FilterExecutor 同源 fold 语义，`build_output_row` 与 join.rs 同型）；`DeleteExecutor` 墓碑 slot 化——DELETE 写独立墓碑版本（create_tx=删除者、next→被删 rid），索引移除保持；`DataScanExecutor` `superseder_suppresses` 重写——抑制按删除者提交状态（aborted 不抑制 / 活跃不抑制 / 已提交抑制整链）+ 快照可见性 `is_visible ∨ is_visible_self`（RC 语句视图）；`SubqueryEvalExecutor`/`SemiJoinExecutorV2`/`AntiJoinExecutor` 关联臂语句级缓存 `HashMap<Vec<(String, Value)>, Vec<Vec<Value>>>`（查在 inject/执行前、存仅在成功 drain 后，错误与多行早退不入缓存；Semi/Anti 命中从行集重建 right_hashmap + right_has_rows 保 NULL 键行语义）（spec mvcc-tombstone-visibility / correlated-subquery-cache）；MS13——新 `datetime.rs` 日历数学模块（Hinnant civil 算法 / DA5 定宽 parse-format / `trunc_ts` / `IntervalParts` 解析与 `add_months_ymd` 同日锚定截月末 / datediff 对称朝零截断 / `coerce_datetime_write` 写入边界强制解析）+ `Value`/`ValueRef` Date/Timestamp 变体全套方法臂（同型比较/Hash/Display/to_key→None/lt_agg；跨族比较谓词层同变体守卫）+ `compare_values` 显式臂 + `IntervalArithExpression`（NULL 传播/月先微秒后/Sub 求值期取负）+ `StorageError::InvalidDateTime` + InsertExecutor/UpdateExecutor 写入 coerce（先于 MS16 键位预检，零副作用拒绝）+ `predicate.rs` CAST 日期族矩阵（String↔日期解析格式化/Timestamp↔Date 截断零点/跨族拒绝）+ `function.rs` REGISTRY +10 日期函数族（NOW(0,0) 零参 carve-out；eval_scalar 臂 D3 求值序/严格类型/date_trunc 单位大小写不敏感；ABS `checked_abs` 溢出显式错误/ROUND ±308 SQLite 对齐饱和（I043）；错误载体 ValueError→String 保既有文案）+ `AggregateNode.group_key_exprs` 求值化分桶（`extract_group_key` 逐行求值表达式键，混合投影解锁、未分组项 NonAggregatedColumn 点名）+ 新 `single_row.rs::SingleRowExecutor`（no-FORM 虚拟单行恰产一行空行）（spec datetime-type-system / datetime-functions / group-by-expression / no-from-select / cli-analytics-commands / sql-scalar-functions 修改））
- `src/storage/` — BufferPool（DashMap + Miss Semaphore + Per-Page Loading Locks）、AsyncStorage（含 `page_count()`，MS07-T01）、FileStorage（页读写 `FileExt::read_exact_at`/`write_all_at` 位置参数化，每页 1 syscall，MS08-T01；open 即 `try_lock` advisory 独占锁，冲突 → `StorageError::DatabaseLocked`，先于 WAL 打开与恢复，MS10-T02；64B 格式头——open 时初始化（0 字节新库）或分类校验，`NotADatabase`/`NewerFileVersion`/`IncompatibleHeader` 先于页解析与 WAL 触碰，页 I/O 偏移 +HEADER_SIZE 平移（`PageId::to_offset` 纯数学不变），MS10-T03；`StorageError::KeyTypeMismatch` 键列写入类型强制错误面，MS16；`StorageError::InvalidDateTime` 日期族写入边界强制解析错误面，MS13；MS09——`BufferPool::mark_tx_aborted` no-op 移除（I032 配套）+ `find_visible_version` 页级 all-invisible 快路径与 `check_page_all_visible` 消费 `snapshot.high_water()`（002-rework 修正）；MS17-T02——`page_visibility.rs` `MIN_CREATE_UNKNOWN = u64::MAX` 哨兵（`all_invisible_for` 对 UNKNOWN 回落 false）+ `BufferPool::clear_all_visible` `or_insert` 哨兵首建（ISS01 0 毒化与 set_all_visible MAX 毒化双向闭合，RC 快路径保守回落正确））、DataPage、file_header（64B 布局编解码纯函数 + 私有 HeaderError；`KNOWN_FLAGS_MASK=FLAG_ENCRYPTED`，头携带 32B 盐与 12B KDF 参数，MS10-T03 + MS17-T01）、`crypto.rs`（Argon2id + `PageCipher`，4096B 页 ↔ 12B nonce + ciphertext + 16B tag，AAD=page_id）、FileStorage `open_with_key` 与 4124B 加密页步长（WAL/checkpoint 保持明文，MS17-T01）
- `src/storage/catalog.rs` — Catalog（系统表 `__tables` / `__columns` SlottedPage 管理 + 二进制行序列化 + 链式页表 + 保留名常量，MS07-T01；COL_TAG_DATE 0x05/COL_TAG_TIMESTAMP 0x06 序列化往返，MS13）
- `src/storage/btree/` — B-Tree（IndexManager 含 `from_root` 路径 + `root_page_id` 访问器，MS07-T01；`collect_all_pages` 物理页枚举 pub async + visited 防环，MS07-T02；LeafNode、InternalNode、redistribution-first merge；`Key` 32 字节定长比较修尾部零键最小键搜索盲区，MS10-T02 G1；root 变更（分裂/收缩）经 catalog 上下文同步 `__tables` 行，MS10-T02 R5；`collect_all_pages_tolerant` 洞容忍收集（父指针枚举洞 id、不递归、不报错），MS10-T02 R8）
- `src/storage/data/` — TableManager（`async new(bp, storage) -> Result<Arc<Self>>` + `open_or_init` 重建 + 保留名检查 + 跨页 tail 同步，MS07-T01；`drop_table` 物理释放数据/索引页到 free-list + 私有 `collect_data_pages` 链遍历，MS07-T02；`replace_index_manager` 索引换入（恢复后重建），MS10-T02 R8；`create_table_with_constraints` 建库约束持久化通道（旧 `create_table` 签名零变化委托壳），MS10-T05）
- `src/storage/page_format/` — SlottedPage（6B logical_id slot）；tuple.rs TAG_DATE 0x06（4B LE）/TAG_TIMESTAMP 0x07（8B LE）compute/serialize/deserialize×2 臂 + storage ColumnType Date/Timestamp（MS13，损坏/截断显式拒绝同既有五类型）
- `src/storage/page_visibility.rs` — PageVisibilityInfo（页面级 MVCC 摘要）
- `src/transaction/` — TransactionId（AtomicU64）、TransactionManager（begin/commit/abort 唯一 WAL 源；`record_version` 按表聚合 + 多表回滚含墓碑，MS07-T04）、Snapshot、VersionChain（含 DELETED_TX_ID 墓碑守卫，MS06-T01）、RowLock、`TransactionSession`（`session.rs` 会话基元：`Option<Transaction>` 状态机 begin/commit/rollback/is_active/tx_id/tx，双 begin/无事务终结错误文案即契约，MS11-T02）；MS09-T01——`IsolationLevel` 枚举（mod.rs）；`Snapshot` 自身身份/高水位分离（`high_water` 字段 + `statement_view` 构造器，`is_visible` 规则 2 改用 high_water，`new` 构造 RR 面行为不变，D10）+ `TransactionId::advance_past` 恢复后分配器水位推进（tx_id.rs，消除重启 id 复用）；`abort_cleanup_versions` 墓碑化改 `VersionHeader::mark_aborted` 中性化（含 DELETE 墓碑 slot 情形）；`VersionChain` `superseder_suppresses` 按删除者提交状态判定（aborted create_tx=0 不抑制 / 活跃不抑制 / 已提交抑制整链）
- `src/wal/` — WalWriter（含 `rewrite_truncate` 单临界区原地截断）/ Reader（带 LSN 读取；逐帧无歧义解析——歧义偏移先按新格式解析以 CRC 验证为判据、失败 seek 回帧首按旧格式，修复格式嗅探 derail，MS10-T02 T0）/ Buffer / Checkpoint（位点消费 + 九步重写截断，WAL 有界，MS07-T05）/ Recovery（位点过滤 redo + 全部失败显式 `Err`，K05，MS07-T05；位置寻址重放——redo 按记录 `row_id` 写入（slot 已存在跳过/稠密落位校验/未初始化页 init）+ Update 版本链重建 + Delete 墓碑重放，MS10-T02 T0b；`redo_count > 0` 时索引去信任——Update `old_row_id` 经磁盘版本多映射 max-rid 派生 + old_tuple 校验，重放后从最终数据页重建各表 PK 索引（链尾回溯 + 重复 PK 显式报错）经 `replace_index_manager` 换入 + catalog root 写回 + 旧树洞容忍释放，`redo_count == 0` 路径零变化，MS10-T02 R7/R8 D10；Update 重放无键回退——`PkVersionMaps` keyed/keyless 双桶（无键版本按 tuple 原始字节追踪，三处追加点），非 deindexed 分支保留 RedoFailed，MS10-T05；MS09——Delete redo 臂写独立墓碑 slot（与运行期 DeleteExecutor 两态一致）+ `mark_uncommitted_aborted` 页链迭代标记（I032：恢复期未提交行显式中性化不复活）；MS17-T02——`CheckpointSite` 24B 位点（lsn + timestamp + tx watermark，wal/mod.rs 导出）+ `read_site_file` 兼容读三分支（≥24B 携带水位 / 16B..24B 旧格式无水位 / <16B None 既有）+ `CheckpointManager::checkpoint` 增 `tx_watermark` 闭参、步骤 1b 于 LSN 捕获后读取（D8 健全性：位点前缀 Begin id ≤ 水位、水位后分配 id 其 Begin 落重放尾部）、步骤 5/8 两次位点写入均携带 + `RecoveryResult.checkpoint_tx_watermark`（`full_recover` 直填含空 records 早退；代际失效位点仅作废 lsn 过滤、水位仍消费——过度推进仅跳号安全））
- `src/network/` — Server（Semaphore 并发限流）、PgProtocol（write_buf 批写 + TCP_NODELAY；Date/Timestamp 列 OID 1082/1114 + DA5 文本编码，MS13）、JsonProtocol

## 目录约定

- 源码: `src/`
- 集成测试: `tests/`（含新增 `tests/schema_persistence_test.rs` 8 测试，MS07-T01 落地；新增 `tests/drop_table_free_test.rs` 6 测试，MS07-T02 落地；新增 `tests/explicit_tx_test.rs` 8 测试，MS07-T04 落地；新增 `tests/checkpoint_redo_reduction_test.rs` 9 测试，MS07-T05 落地；新增 `tests/pushdown_test.rs` 15 测试，MS07-T06 落地；新增 `tests/file_storage_io_test.rs` 4 测试 + `tests/prefetch_test.rs` 3 测试，MS08-T01/T02 落地；新增 `tests/cli_test.rs` 12 测试 + `tests/projection_test.rs` 6 测试，MS10-T01 落地；新增 `tests/wal_recovery_large_test.rs` 7 测试 + `tests/btree_scale_test.rs` 5 测试 + `tests/database_file_lock_test.rs` 4 测试（cli_test 增至 17），MS10-T02 落地；新增 `tests/file_header_test.rs` 14 测试 + database_file_lock_test 增至 5 + cli_test 增至 21，MS10-T03 落地；cli_test 增至 25（多语句分片 4 新增 + 1 护栏用例重写），MS10-T04 落地；新增 `tests/keyless_row_test.rs` 4 测试 + cli_test 增至 48（生命周期子命令组 + 无键行落库/恢复/全形状往返），MS10-T05 落地；新增 `tests/expression_e2e_test.rs` 24 测试（predicate_test 23 / planner_test 36 / pushdown_test 16 追加，MS11-T01 Iter000 落地）；新增 `tests/projection_expression_test.rs` 16 测试 + cli_test 增至 54（派生列表头与渲染，MS11-T01 Iter001 落地）；新增 `tests/tx_statement_test.rs` 19 测试（3 lib 非会话路径拒绝 + 16 CLI 会话 e2e，MS11-T02 落地）；新增 `tests/scalar_function_test.rs` 28 测试 + cli_test 增至 56（函数表头 2 用例，MS11-T03 落地）；新增 `tests/keyless_eq_routing_test.rs` 8 测试（键位等值无键行可达 + plan 形状 + restart，MS15-T01 落地）；新增 `tests/update_index_maintenance_test.rs` 5 测试（I037 键位索引清理 + 恢复面，MS15-Rest 落地）；cli_test 增至 65（I034 表头 3 用例 + I039 表名归一化 5 用例 + 转义名 dump 往返 1 用例，MS15-Rest 落地）；MS16 落地——`tests/keyless_eq_routing_test.rs` 扩展至 14（路由类型门 6 用例）、新增 `tests/key_type_conformance_test.rs` 8（R3 拒绝矩阵 + 显式列序 R3-S8）、新增 `tests/insert_column_list_test.rs` 7（R6 六场景 + 重复列 SHALL）、`tests/update_index_maintenance_test.rs` 扩展至 9（I037 5 + I047 rekey 4）、`tests/expression_e2e_test.rs` BH-1 校准 1 处（负 Float 行移 Float 键列表 `tf`）、BH-3 校准——`tests/gc_test.rs` 3 / `tests/version_chain_test.rs` 2 / `tests/plan_exec_test.rs` 1 按行当前键寻址）；MS09 落地——新增 `tests/mvcc_tombstone_visibility_test.rs` 11（I033 探针序列 + 未提交删除形态 + 回滚恢复 + restart 两态 + I032 不复活）、新增 `tests/isolation_level_test.rs` 7（RC 脏读排除 / 语句间可见·消失 / 自身写可见 / auto-commit 等价 / RR 零回归）、新增 `tests/nested_loop_join_test.rs` 9（非等值语义 / 混合腿 / NULL 排除 / 空输入 / plan 形状 / Hash 保持 / ORDER BY / 注入臂直构见证 / WHERE 拒绝）、`tests/subquery_test.rs` 扩展至 28（缓存等价见证 8 用例）；MS13 落地——新增 `tests/datetime_type_test.rs` 16 + `tests/datetime_function_test.rs` 19 + `tests/group_by_expr_test.rs` 12 + `tests/no_from_select_test.rs` 9（8 CLI e2e + 1 lib 一致性）+ cli_test 增至 78（stats 5/sample 4/profile 4）+ scalar_function_test 增至 32（I043 3 函数/I044 1 函数）+ datetime.rs 36 组与 single_row.rs/lifecycle 纯函数 src 单测；校准 3 处（expression_e2e cast_unknown 示例 DATE→TIME〔datetime-type-system R7 校准段〕、projection_expression 与 scalar_function 聚合×表达式通道 → NonAggregatedColumn〔group-by-expression R2 校准段〕）；另 src 内单测补足（快照高水位 / advance_past / NLJ 执行器等，全量 1050）；MS17-T02 落地——表面批（subquery_test +4 JOIN 拒绝与零回归 / cli_test +3 ISS03 表头 json+table 与 I048 import）、哨兵批（page_visibility 单测 +2、buffer_pool 新测试模块 +2、isolation_level_test +1 RC 端到端；T7 后该夹具 scratch workaround 移除）、水位批（isolation_level_test +1 RC 重开 e2e、checkpoint.rs 位点单测 +3：24B 往返 / 16B 旧格式 / 8B 短写）、checkpoint_test/recovery_test 签名机械适配（断言集零变化），全量 1065
- 单元测试: 文件内 `#[cfg(test)]`（含新增 `src/storage/catalog.rs` 10 单元测试）
- 基准测试: `benches/` (8 套: micro / concurrent / scale / sqlite_compare / single / precise_compare / data_scan / visibility)
- OpenSpec: `openspec/`
- 安装脚本: `install.sh`
- 交叉构建脚本: `build-riscv64-musl.sh`（RISC-V 64 musl 固定 target；产物输出 `dist/`，Git 忽略）
- 用户文档: `README.md`、`README.zh-CN.md`、`rtsql-docs/SKILL.md`
- 状态文档: `.claude/docs/`
- 分析: `.claude/analysis/`（按需）
- Runbook: `.claude/runbooks/`（按需）
- Incident: `.claude/incidents/`（按需）
- Legacy carrier: `.claude/legacy/`

## 支持平台

- 当前开发与验证: Linux x86_64
- 安装脚本目标: Linux / macOS
- AI 平台: Claude Code、Codex、OpenCode

## 仓库现场

- **分支**: master
- **提交基线**: 145bba4（此前已提交状态，origin/master 同点；本文件所在提交统一纳入 RISC-V musl 交叉构建 change 实施/收尾与范围外工作流）
- **ahead of origin**: 1 commit（本文件所在提交）
- **工作区**: 本文件所在统一提交覆盖 RISC-V 交叉构建 change 实施/收尾与范围外工作流（install.sh bashrc PATH、SKILL.md 迁移、docs 路径勘误）；提交完成后工作区 clean（`dist/` 产物 Git 忽略）
- **最新 tag**: M11
- **测试**: 1101 tests pass, 0 failures, 2 ignored（Rust 产品代码零变化，沿用 2026-09-24 MS17 初版收口结论）
- **OpenSpec**: 40 specs（2026-09-24 新增 riscv64-musl-build；change 已归档；无活跃 change。注：新版 openspec CLI `validate --all --strict` 将 14 个 MS06–MS10 时代 spec 的占位 Purpose 标记为失败——既有状态，非本轮引入）

## 同步状态

- `current` — 文档与代码一致；RISC-V musl 交叉构建 change 实施与收尾由本文件所在统一提交承载；40 specs 已合并，change 已归档，tasks/SNAPSHOT 已同步

## 权威文档

- 公共规则: `CLAUDE.md`
- 项目模型: `openspec/specs/project-model/spec.md` (Mxx)
- 决策: `openspec/specs/decisions/spec.md` (Dxx)
- 知识: `openspec/specs/knowledge/spec.md` (Kxx)
- 参考: `openspec/specs/references/spec.md` (Rxx)
- 改进: `openspec/specs/improvements/spec.md` (Ixx)
- 任务与路线: `.claude/docs/tasks.md`
- 变更: `openspec/changes/`（无活跃 change；归档目录含 MS06–MS17 已完成 carriers，最新为 `archive/2026-09-23-ms17-initial-release/` 与 `archive/2026-09-24-riscv64-musl-build-artifacts/`）
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
- `openspec/specs/cli-noninteractive-shell/spec.md`（MS10-T01 起持续扩展；MS17-T02 新增标量子查询表头形状；MS17-T01 增全局 `--key`/exit 5，MS17-T03 新增隐藏 `completions <bash|zsh|fish>` Requirement；现 14 个 Requirement）
- `openspec/specs/database-file-lock/spec.md`（MS10-T02 落地，2 个 Requirement：R1 独占锁语义 / R2 生命周期与释放）
- `openspec/specs/wal-recovery-frame-parsing/spec.md`（MS10-T02 落地，2 个 Requirement）
- `openspec/specs/wal-recovery-replay-integrity/spec.md`（MS10-T02 落地，2 个 Requirement；MS10-T05 修改：「重放保持 DML 语义」old_row_id 推导扩展无键行回退（keyless 桶，SHALL NOT RedoFailed）+ 新增场景「无键行 Update 崩溃恢复语义正确」）
- `openspec/specs/database-file-format-header/spec.md`（MS10-T03 起 4 个 Requirement；MS17-T01 扩展 64B 头的加密位/盐/KDF 参数、加密页偏移与 24B checkpoint 位点校准）
- `openspec/specs/database-encryption/spec.md`（MS17-T01，4 个 Requirement：页记录格式 / Argon2id 与打开拒绝面 / `--key`·env 与 exit 5 / 明文零回归及伴生文件限制）
- `openspec/specs/install-script/spec.md`（MS17-T03，2 个 Requirement：一键编译安装与补全 / 程序与数据两模式卸载）
- `openspec/specs/riscv64-musl-build/spec.md`（MS17 后续分发扩展，4 个 Requirement：固定目标的交叉构建入口 / 构建前置检查与锁定依赖 / 版本化分发产物 / 宿主机验证边界）
- `openspec/specs/sql-expression-evaluation/spec.md`（MS11-T01 落地，6 个 Requirement：R1 谓词四件套 / R2 NULL 三值语义 / R3 CASE-COALESCE-CAST / R4 SELECT 派生列 / R5 INSERT 负数字面量 / R6 既有语义零回归）
- `openspec/specs/sql-transaction-statements/spec.md`（MS11-T02 落地，5 个 Requirement：R1 事务语句 CLI 会话往返 / R2 边界子句显式拒绝（含 `AND NO CHAIN` 按裸语句同义）/ R3 会话状态边界 / R4 非会话路径显式拒绝 / R5 既有语义零回归）
- `openspec/specs/sql-scalar-functions/spec.md`（MS11-T03 落地，6 个 Requirement：R1 注册与分派机制 / R2 string 函数六件 / R3 math 函数四件 / R4 NULL 语义与嵌套参数 / R5 调用面与边界 / R6 既有语义零回归；MS13 修改——R1 大小写变体 SQL 层 e2e 见证 + `now` 等零参注册函数 carve-out、R3 abs i64::MIN 溢出显式错误 + round 极端 digits SQLite 对齐饱和（I043），仍 6 个 Requirement）
- `openspec/specs/planner-key-equality-routing/spec.md`（MS15-T01 落地；MS16 修改——R2 收窄至 Int 键列 + 新增「键列类型感知路由」，现 4 个 Requirement：R1 键位等值过滤对无键行可达 / R2 可键控字面量路由保持（Int 键列）/ R3 键列类型感知路由 / R4 既有语义零回归）
- `openspec/specs/update-index-maintenance/spec.md`（MS15-Rest 落地，2 个 Requirement：R1 键位置更新为无键值后旧键索引条目清理 / R2 既有 UPDATE 语义零回归；MS16 修改——新增「键位 rekey 后索引条目一致」（含 BH-3 校准段），现 3 个 Requirement）
- `openspec/specs/key-column-type-conformance/spec.md`（MS16 落地，1 个 Requirement「Int 键列写入类型强制」7 场景，含 BH-1 校准段）
- `openspec/specs/insert-column-list-mapping/spec.md`（MS16 落地，1 个 Requirement「INSERT 显式列清单映射与校验」6 场景）
- `openspec/specs/table-name-resolution/spec.md`（MS15-Rest 落地，3 个 Requirement：R1 标识符表名解析归一化 / R2 dump 与 schema 表名保真 / R3 既有裸名语义零回归；MS17-T02 修改——新增「import 表名实参转义可达」（I048），现 4 个 Requirement）
- `openspec/specs/transaction-isolation-levels/spec.md`（MS09-T01 落地，3 个 Requirement：R1 隔离级别经 lib API 配置（`open_with_isolation`，默认 RR 逐字节等价）/ R2 Read Committed 语句级已提交视图 / R3 既有默认路径零回归；MS17-T02 修改——新增「RC 重启后可见性高水位健全（checkpoint 水位持久化）」（位点 24B 携带 tx watermark + max 合并推进 + 旧格式兼容读），现 4 个 Requirement）
- `openspec/specs/mvcc-tombstone-visibility/spec.md`（MS09-T01 落地，5 个 Requirement：R1 墓碑自描述且不影响前驱版本 / R2 已提交墓碑抑制整条版本链 / R3 未提交与已回滚墓碑不抑制、评估回溯前驱 / R4 恢复后墓碑语义一致（含 I032 不复活）/ R5 既有 MVCC 语义零回归；附已知边界段——未提交删除/回滚后 PK 点查预存时序边界；MS17-T02 修改——新增「页级可见性摘要无毒化（哨兵语义）」（ISS01+MAX 毒化，`MIN_CREATE_UNKNOWN`），现 6 个 Requirement）
- `openspec/specs/join-executor-selection/spec.md`（MS09-T02 落地，5 个 Requirement：R1 纯等值 JOIN Hash 路径保持 / R2 非等值 JOIN 经 NLJ 可达 / R3 NLJ 结果语义与三值 NULL 处理 / R4 启发式选择为计划期判定 / R5 既有 JOIN 面零回归）
- `openspec/specs/correlated-subquery-cache/spec.md`（MS09-T04 落地，5 个 Requirement：R1 相同关联参数值复用子查询结果 / R2 不同参数值独立求值（NULL 键成分）/ R3 缓存结果与直执行逐字节等价（错误不缓存）/ R4 缓存生命周期为单次语句执行 / R5 既有子查询语义零回归）
- `openspec/specs/datetime-type-system/spec.md`（MS13-T01 落地，7 个 Requirement：R1 DATE/TIMESTAMP 值与存储格式 / R2 DDL 显式类型映射 / R3 类型字面量与写入边界强制解析 / R4 比较与键控边界 / R5 CAST 矩阵扩展 / R6 渲染与导入导出面 / R7 既有语义零回归，含 BH-1 型校准段——cast_unknown 示例 DATE→TIME）
- `openspec/specs/datetime-functions/spec.md`（MS13-T02 引擎侧落地，4 个 Requirement：R1 日期函数族注册与语义 / R2 INTERVAL 表达式算术（含 WHERE 腿解缠绕通路）/ R3 datediff 语义 / R4 既有标量函数零回归）
- `openspec/specs/group-by-expression/spec.md`（MS13-T02 落地，2 个 Requirement：R1 GROUP BY 别名与表达式引用（列名→别名→表达式文本→位置四级解析序）/ R2 匹配失败与既有约束保持，含校准段——聚合×表达式混用拒绝通道改 NonAggregatedColumn）
- `openspec/specs/no-from-select/spec.md`（MS13-T03/I035 落地，3 个 Requirement：R1 常量表达式单行可达 / R2 拒绝面显式 / R3 既有 FROM 形态零回归与算术表达式项解锁）
- `openspec/specs/cli-analytics-commands/spec.md`（MS13-T02 CLI 侧落地，4 个 Requirement：R1 stats 输出契约 / R2 sample 输出契约 / R3 profile 输出契约 / R4 格式四态与错误面）
- `openspec/specs/in-subquery-join-rejection/spec.md`（MS17-T02 落地，2 个 Requirement：R1 IN 子查询 JOIN 形态诚实拒绝（ISS02 最小诚实化——`InSubqueryJoinUnsupported` 计划期点名，不误报多列）/ R2 既有 IN 子查询语义零回归）
