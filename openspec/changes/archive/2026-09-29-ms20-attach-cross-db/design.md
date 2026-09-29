# Design — MS20 一期 ATTACH 式跨库交互

> 状态：closed（2026-09-29，Phase 2 调查闭合；三路并行调查证据见下文引用行号）

## 技术决策

### D1：AttachSession 注册面——CLI `run_sql` 本地持有

- 形态：`AttachSession { dbs: HashMap<String, Arc<Database>> }`（别名小写 → 实例），在 `run_sql` 循环前创建（`src/cli/mod.rs:382` `TransactionSession` 旁），循环结束后 best-effort `close()` 逐个释放（附加库一期只读无脏页，MS18 零工作守卫保证 close 无操作；进程退出时 flock 随 fd 释放兜底）。
- 不放进 `Database`：attach 是 CLI 会话概念，lib `Database::execute*` 路径无注册面、两段名按既有扁平名语义处理（零回归）。`Database` 零字段变化。
- 多实例可行性（调查确认）：全库无全局/静态状态；每实例成本 = 2 fd + 1 flush task + ≤400KiB 页缓存 + 独立 WAL/recovery。`Database` 字段全 `pub` 且 `Clone`，可直接被 pipeline 消费。
- 信号路径（`execute_command_inner` phase-2 只 close 主库，`mod.rs:355-367`）：附加库一期只读，不 close 无损（cancel-safety 注释 `mod.rs:325-327` 同型）；恢复安全由 WAL 机制保证。

### D2：ATTACH 分类与分派——MS11-T02 事务语句同型

- sqlparser 0.44 原生解析 `ATTACH [DATABASE] '<expr>' AS <ident>` → `Statement::AttachDatabase { schema_name, database_file_name: Expr, database: bool }`（sqlparser-0.44 `parser/mod.rs:476,667-677`、`ast/mod.rs:2035-2042`；GenericDialect 无门控，实测当前落到 catch-all `Unsupported statement type: ATTACH`）。
- `src/parser/planner/mod.rs` 新增 `classify_attach_statement(stmt) -> Result<Option<AttachInfo>, PlanError>`（`pub(crate)`，`classify_transaction_statement` 同型，`mod.rs:46-94` 先例）：提取别名（`Ident.value` 小写）与路径（`database_file_name` 仅接受单引号字符串字面量，其他表达式形态点名拒绝）；`build_plan` 前置拒绝（`mod.rs:204-213` 同型，session-only 文案、不进 plan cache）。
- CLI `run_sql` 三分派扩为四臂（`mod.rs:385-473`）：attach 臂在事务臂旁，不经 `TransactionSession`（与活跃事务正交，假设 11）。
- ATTACH 执行序：路径 `resolve_db_path` 复用（`src/cli/resolve.rs:30-35`）→ `exists()` 前置检查（拒绝语义同 `resolve_existing_db`，`lifecycle.rs:728-740`——`Database::open_with_key` 会静默创建 0 字节新库，`file_storage.rs:87-116`，必须先查）→ `Database::open_with_key(path, RepeatableRead, key)`（key 复用全局 `--key`/`RTSQL_KEY`，`mod.rs:341` 同参）→ 注册。
- 错误映射（`open_error_status` `mod.rs:304-320` 同型分类，但带语句上下文）：`DatabaseLocked` → exit 4；`InvalidKey` → exit 5（明文+key / 加密+无 key open 期即拒；加密+错误 key 在 open 的 catalog/recovery 读时解密失败，`file_storage.rs:197-202`，同属 open 期）；不存在/其他 → `sql_failure_status` 同型 exit 3。任一失败注册表零变化。

### D3：两段名路由——分段信息尚存处判定

- 核心约束（调查发现）：`object_name_to_table_name`（`src/parser/ast.rs:317-323`）把多段名拍平为 `"o.u"` 后，**与引号单 Ident 含点（`"o.u"`）不可区分**——路由判定必须在 `ObjectName` 分段尚存处（`name.0.len() >= 2 && 首段 ∈ 注册表`），不得在扁平串上 split。
- 路由规则（spec 已定稿）：`len >= 2 && 首段命中注册表` → 附加库路由；否则扁平名回落（既有含点表名零回归）；`len >= 3 && 首段命中` → 计划期点名拒绝。`len == 1` 恒走既有路径（CTE 遮蔽/主库——CTE 名单段，`query.rs:825`，无路由交互）。
- 读面接线：`extract_all_table_names`（`src/pipeline.rs:1069-1242`）提取层保留分段（新增携带 `ObjectName` 或 (alias, table) 的提取结果），`register_table`（`pipeline.rs:1244-1295`）按路由规则向主库或对应附加库的 `table_manager.get_table` 取 schema；附加库表以限定扁平名 `"o.u"` 为 key 注册进 `PlanBuilder.tables`（schema 来自附加库 catalog），plan 扫描节点 `table_name` 同 key。
- 写面拒绝：planner 计划期需知道哪些名字是附加库引用——pipeline 路由判定时把「附加库引用名集合」经加性字段传入 `PlanBuilder`（MS16 `set_pk_column_type` 通道先例）；`ddl_dml.rs` 五个写消费点（INSERT:162 / UPDATE:924 / DELETE:1012 / CREATE:788 / DROP:906）遇集合成员 → 计划期点名拒绝（只读边界）。`CREATE TABLE o.t AS ...` 同臂拒绝（CTAS 仅主库）。
- 三段列引用：`resolve_column_ref`（`expression.rs:24-81`，现仅收两段 `t.col`）扩三段臂：前两段 join 命中已注册表引用名（含表别名）→ 列取第三段；未命中维持 `UnsupportedExpression("COLUMN REFERENCE")` 既有拒绝。
- 执行面路由：pipeline 持有 `name → Arc<Database>` 路由表，`create_executor_from_plan`（`pipeline.rs:518-898`）各扫描臂（Scan:536 / DataScan:544 / IndexScan:559 / IndexScanAll 及 SemiJoin:770 / AntiJoin:814 / SubqueryEval:831 重建路径）按 `node.table_name` 查路由表取对应库的 `table_manager`/`buffer_pool`/`transaction_manager`；未命中 → 主库（lib 路径零变化）。

### D4：plan cache 绕过

- 规则：语句引用的两段名首段命中注册表 → 该语句跳过 plan cache 读与写（写点 `plan_stage` `pipeline.rs:111-113` `is_cacheable`；CLI 会话读路径同规）。理由：cache key 仅归一化 SQL 文本（`plan_cache.rs:88-125`），无 attach 身份——同文本 `SELECT * FROM o.u` 在「无 alias o」与「有 alias o」会话语义不同。
- 实现：pipeline 入口加性参数携带路由上下文（lib 路径传 `None`，既有签名调用点适配为委托壳，MS17 `open_with_key` 委托壳先例）。

### D5：CTAS 执行序与崩溃一致性

调查发现的两条硬约束：

1. **catalog 无 WAL**（DDL 不落 WAL，schema 仅靠脏页 flush 持久化），而 WAL redo 按表名查 catalog（`src/wal/recovery.rs:274-275,580-581`），缺表 → `RedoFailed` → **库变砖**。故 CTAS 必须在写任何数据 WAL 记录前落盘 catalog。
2. **执行期无类型信息**：`Response` 只有 JSON 行（`protocol.rs:34-49`），类型推导须在 `ExecResult::Row(Vec<Value>)` 层完成（DerivedScan 物化先例 `pipeline.rs:836-850`）。

执行序（`execute_stage` 新增 CTAS 混合臂）：

1. SELECT 部分走 `register_table` + `build_plan`（`plan_stage` 的 DDL 裸臂 `pipeline.rs:96-98` 需为 CTAS 开注册通道）；输出列名取 `get_plan_output_columns`。
2. 执行 SELECT 全物化 `Vec<Vec<Value>>`（与 Response 物化同量级，无新增内存架构）。
3. 逐列类型推导（全行扫描）：全 NULL → `String(255)`；Int∪Float → `Float`（写入门升格先例 `insert.rs:137-143`）；全同型 → 该型；其他混合 → 建表前点名拒绝（`CTAS column type conflict`，零副作用）。
4. `create_table_with_constraints(name, cols, pk=首列名)`（`table_manager.rs:315-320`；首列隐式主键沿用 `create_table.rs:74-83` 既有 fallback）。
5. `buffer_pool.flush_all()`（`buffer_pool.rs:216`）落 catalog 页——崩溃一致性边界：此后崩溃残留空表（schema 完整、无 WAL 数据记录），redo 不变砖。
6. 隐式事务逐行插入（`InsertExecutor::with_table_manager` 路径 `insert.rs:82-109`，全部既有校验门自然生效：`DuplicateKey`/类型门等）。
7. 失败 → abort 事务 + best-effort `drop_table` 清理（零副作用 spec 承诺）。
8. plan cache 全清（DDL 同型，`pipeline.rs:143,347`）。

显式事务内 CTAS：一期点名拒绝（DDL 不入事务 + 回滚清理语义复杂化；spec 已含拒绝场景）。

### D6：隔离与快照

CLI 恒 RR（`mod.rs:341`）→ `statement_snapshot` 返回 `None`（`database.rs:149-151`），附加库同型 `None`，无跨库快照协调问题。DataScan 的 `tx_manager`（tombstone suppression active-set 捕获，`data_scan.rs:97-105`）必须按路由表取**附加库**实例——tx-id 空间按库独立（各从 0 分配，`manager.rs:57-74`），串用会错误抑制。RC 跨库不存在（attach 是 CLI-only，lib RC 路径无 registry）。

### D7：表头与渲染

`get_plan_output_columns`（`query.rs:51-140`）与渲染层零变化——附加库列表头为裸列名（不带库前缀），CLI `mod.rs:428` 消费不变。

### D8：self-attach 与锁生命周期

不引入 canonicalize（全库零先例）：flock 按 inode 拒绝同文件（含 symlink/hardlink；同进程二次 open+try_lock 亦失败，`database_file_lock_test.rs:35-48` 已锁定），报 `DatabaseLocked` exit 4。附加库锁从 ATTACH 持有至进程退出。

## 替代方案记录

| 决策点 | 备选 | 否决理由 |
|---|---|---|
| DETACH | 预解析拦截 / 借 `DEALLOCATE` | 破坏无自研分片器不变量 / 语义挪用不诚实；one-shot 无消费场景（用户豁免 2026-09-29） |
| 两段名未命中 | 硬拒绝 | break 既有含点扁平表名（零回归红线） |
| 扁平串 split 路由 | `register_table` 对扁平名 split_once | 拍平后与引号单 Ident 含点不可区分，会错误路由既有表 `"o.u"` |
| self-attach 专属文案 | canonicalize 比较 | 全库零先例；flock 已按 inode 覆盖（含 symlink/hardlink） |
| CTAS 全 NULL 列 | 拒绝 / Int | String(255) 最诚实可用（NULL 写入任意类型合法；后续 INSERT 受写入门约束） |
| CTAS 注册面进 `Database` | lib 也支持 ATTACH | attach 是 CLI 会话概念；lib 零变化保零回归 |

## 风险与非实质未知项

## 全 change RTM

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| sql-attach-session R1–R3 | T1 | `tests/attach_session_test.rs` + planner 单元测试 | Covered |
| cross-db-namespace-query R1（两段名路由） | T2 | `tests/cross_db_query_test.rs` 路由矩阵 | Covered |
| cross-db-namespace-query R2（只读跨查组合） | T2 | `tests/cross_db_query_test.rs` 组合场景 | Covered |
| cross-db-namespace-query R3（只读边界） | T3 | `tests/cross_db_query_test.rs` 写面拒绝矩阵 | Covered |
| create-table-as-select R1 | T4 | `tests/ctas_test.rs` 物化/推导/失败零副作用矩阵 | Covered |

## 风险与非实质未知项

- CTAS 输出列名重复（`SELECT a, a`）：建表前列名去重校验，重复 → 点名拒绝（实现细节，随 Act 按既有拒绝模式处理）。
- `PlanBuilder` 附加库引用集合的具体字段形态、路由表类型签名：等价实现选择，留给 Act。
- `run_sql`/`plan_stage` 接线参数的具体传递方式（新参数 vs 上下文结构体）：等价实现选择。
