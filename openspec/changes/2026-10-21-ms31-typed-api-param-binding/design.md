# Design: MS31 类型化库结果与参数绑定 API

## Current-State Evidence（2026-10-21 调查，均经实际代码核对）

### 入口与调用面

- `Database::execute_sql`（`src/database.rs:423`）纯委托 `pipeline::execute` → `execute_inner`（`src/pipeline.rs:1173`：cache get → parse → 单语句检查 → plan → execute_stage）；`Database::execute_in_tx`（`src/database.rs:529`）→ `pipeline::execute_in_tx`（`src/pipeline.rs:994`：parse → 单语句 → plan → execute_stage_in_tx，**无 cache get**，但 `plan_stage` 对 Query 语句仍 put）。
- 生产调用面仅三处：`network/handler.rs:28`（SqlHandler，JSON/PG 协议共用）、`cli/lifecycle.rs:837/888`（restore/import 的 DDL/INSERT 执行）、`storage/integrity.rs:645`（测试 helper `exec_ok`）。CLI one-shot 主路径（`src/cli/mod.rs:921-984`）**不调 execute_sql**——自行 `plan_stage_routed` + `get_plan_output_columns`（表头旁路，:949）+ `execute_stage[_in_tx]_routed` + Response 四臂匹配渲染。
- tests 81 个文件消费（典型断言：`matches!(resp, Response::Error { .. })`、`Response::QueryResult { rows }` 解构 + `serde_json::json!` 对比，见 `src/transaction/session.rs` 测试模块）。

### Response 词表与消费者

- `Response`（`src/network/protocol.rs:34-49`）：`QueryResult { rows: Vec<Vec<serde_json::Value>> }` / `AffectedRows { count }` / `Error { message, sqlstate: Option<String> }` / `Pong`；serde 可序列化（JSON 协议直出）。
- `render`（`src/cli/render.rs:24`）消费 `QueryPayload::Rows(Vec<Vec<serde_json::Value>>)` 四格式纯函数；PG 协议（`src/network/pg_protocol.rs:249-321`）消费 Response 四臂。两者**零改动即可保持字节等价**，前提是边界转换与现 `value_to_json` 逐字节同源。

### 错误漏斗

- parse 失败 → `Parse error: {msg}` + 42601（`execute_inner:1220`、`execute_in_tx:998`）；plan 失败 → `PlanFailure { message, sqlstate }`（`Plan error: ` 前缀单层，`src/pipeline.rs:133-157`）；执行失败 → `Execution error: {e}` sqlstate None（`execute_executor:1299`）；另有 `Abort failed: `/`Commit failed: `/多语句/只读 25006/附加库写 0A000 各漏斗点。SQLSTATE 分类源 `PlanError::sqlstate()`。文案与分类是 `error-reporting` 域既有契约，逐字节保持。

### 行收集与元数据

- `execute_executor`（`src/pipeline.rs:1284`）drain 执行器 → `value_to_json`（:1934，Float NaN/Inf → JSON Null 有损；Date/Timestamp → DA5 字符串）→ QueryResult。
- 列名单源 `PlanBuilder::get_plan_output_columns`（`src/parser/planner/query.rs:123`，全计划节点 walk）；`plan_output_names`（`src/pipeline.rs:1964`）为其 SubqueryEval 子集消费面。**计划节点不携带列类型**；MS30 绑定层 `ScopeEntry { qualifier, columns, types: Option<Vec<ColumnType>>, not_null }`（`src/parser/planner/binding.rs:22`）已穿线类型但当前 `#[allow(dead_code)]`。
- 值→类型矩阵现成：`ctas_value_type`（`src/pipeline.rs:965`，Value → StorageColumnType，Null → None）；`infer_ctas_column_types`（:930）带 `unwrap_or(String(255))` 兜底（建表需要，元数据不沿用该兜底）。

### 参数机制

- `ParameterExpression { param_name: String, value: Mutex<Value> }`（`src/executor/predicate.rs:427`）：owned `evaluate` 返真值（MS30 T03-D2 修复，复合表达式经 owned 路径递归）；`evaluate_ref` 对 String/Date/Timestamp 降级 Null（predicate.rs 既有已知限制——值借用通道无稳定 `&str` 借出；谓词/表达式位不受影响。「M37」为该文件注释沿用的旧编号，现役 project-model 无此条目）。
- 注入链：`Expression::set_parameter_value` / `inject_parameters`（trait 默认 no-op，复合臂实现）+ `inject_correlated_values`（`src/executor/correlated.rs:14`，计划树遍历注入）。
- 既有命名空间：相关参数 `"{table}.{column}"`（`subquery.rs:561`）、标量子查询占位 `__scalar_subquery_{index}`（`subquery.rs:262`）。
- **SQL 文本占位符现状不存在**：tokenizer 无方言门将 `?`（可带数字）tokenize 为 `Token::Placeholder`（sqlparser-0.44.0 `tokenizer.rs:1054`），`parse_value` 映射 `Value::Placeholder`（`parser/mod.rs:5912`）——GenericDialect 可达；现状 `value_from_sqlparser`（`src/parser/value.rs:28`）`_ => Err(PlanError::UnsupportedValue)` 计划期拒绝。

### DML 计划节点值表示

- `InsertNode.values` / `UpsertNode.values`：`Vec<Vec<Value>>` 计划期全物化（`src/executor/plan.rs:188/239`）；`UpsertValueExpr = Literal/Excluded/Old`（:208）。
- `UpdateAssignment { column, value: UpdateValue }`，`UpdateValue = Literal(Value) | Expression(ExpressionRef)`（:278-303）；SET 右值 `Expr::Value(v)` → Literal（`ddl_dml.rs:2005`）。
- **DML WHERE → Key 计划期物化**：`build_update` 经 `extract_pk_from_where`（`ddl_dml.rs:1979`）从 WHERE 字面量提取主键——`WHERE a = ?` 是计划期值依赖，参数执行期才可得。
- INSERT VALUES 逐值 `value_from_sqlparser`（`ddl_dml.rs:780`）；UPSERT SET 右值（:587）。

### PlanCache 与失效

- `PlanCache`（`src/plan_cache.rs`）：DashMap<归一化 SQL, PhysicalPlan>、max 100、全 `&self`；只有 `Statement::Query` 进缓存（`is_cacheable`，`pipeline.rs:2756`）；DDL 八臂 + CTAS 执行后 `clear()`（`execute_stage_routed:340/359/370/381/400/407`）。`PhysicalPlan: Clone`。

## 设计决策

### D1 公共结果与错误类型（新模块 `src/result.rs`）

- `pub enum SqlOutcome { Rows { columns: Vec<ColumnMeta>, rows: Vec<Vec<Value>> }, AffectedRows(u64) }`
- `pub struct ColumnMeta { pub name: String, pub column_type: Option<ColumnType> }`（`ColumnType` = `storage::ColumnType`；`None` = 类型不可得，诚实表示不虚构）
- `pub struct SqlError { pub message: String, pub sqlstate: Option<String> }`——message 与现 `Response::Error.message` 逐字节同源（前缀漏斗位置与文案不动，仅返回形态从 Response 改为 Err）
- `pub fn value_to_json(Value) -> serde_json::Value` 自 pipeline 迁入本模块（逻辑逐字节不变），CLI 与 network 边界共用；pipeline 不再持有该函数。
- 模块不依赖 `network`（核心/协议解耦的结构性落点）；serde_json 依赖保持（转换 helper 属边界共用工具，非网络词表）。
- DDL 语义：沿现 `AffectedRows(0)` 形态；`Pong` 保留为协议本地概念（SqlHandler 在调用库之前处理 Ping）。

### D2 签名替换（用户裁定 A2）

- `Database::execute_sql(&self, sql: &str) -> Result<SqlOutcome, SqlError>`；`Database::execute_in_tx(&self, sql: &str, tx: &Transaction) -> Result<SqlOutcome, SqlError>`。不保留旧签名兼容层。
- pipeline 全链同步 Result 化：`execute` / `execute_inner` / `execute_in_tx` / `execute_stage[_routed]` / `execute_stage_in_tx[_routed]` / `execute_executor` / `readonly_rejection` / `attached_write_in_tx_rejection` / CTAS / InsertSelect 各漏斗点由 `Response::Error` 改 `Err(SqlError)`（文案、SQLSTATE、触发点零变化）。

### D3 列元数据装配

- **names**：`get_plan_output_columns` 单源（与 CLI 表头现状同源，无新通道）。
- **types**：新函数 `plan_output_types(database, plan) -> Vec<Option<ColumnType>>`（pipeline，与 names 同臂结构 walk）：扫描族臂（Scan/DataScan/IndexScan/IndexScanAll/ScalarSubqueryFilter/SemiJoin/AntiJoin 携 `table_name + columns/projection`）与 Join/NestedLoopJoin（`output_columns` 逐 (table, column)）查 `TableMeta` 声明类型（projection 裁剪跟随 names walk）；Sort/Filter/Limit/Having/Projection/Distinct/SetOperation 递归输入；Aggregate/DerivedScan/SubqueryEval/表达式列 → `None`。
- **执行后兜底**：`None` 槽位对结果行跑 `ctas_value_type` 矩阵推导（与 CTAS 同矩阵、**不沿用** `String(255)` 兜底）；无行/全 NULL/推导冲突保持 `None`。空结果 `SELECT a FROM t` 经声明类型臂仍报 INT（A4 语义）。

### D4 边界转换（输出零变化的结构保证）

- `network::protocol` 提供 `pub fn to_response(Result<SqlOutcome, SqlError>) -> Response`（`Ok(Rows)` → QueryResult〔value_to_json〕、`Ok(AffectedRows)` → AffectedRows、`Err` → Error 原样）。
- `SqlHandler::execute`（handler.rs）边界转换后返回 Response——PG/JSON 协议代码零改动。
- CLI 主路径：`execute_stage*` 返回 SqlOutcome 后，QueryOutcome →（value_to_json）→ `QueryPayload::Rows` → render（render.rs 零改动）；`sql_failure_status` 从 SqlError 解构 (message, sqlstate)，文案与退出码不变。
- tests 81 文件：经 `to_response` 保持既有 Response 断言形态（每文件 helper 内一行替换，断言不动）——A2 批准的迁移形态。

### D5 占位符计划面（可达集 = 调查定稿，A6）

- 命名空间 `__param_{index}`（0 基，按 AST 出现序）——与 `{table}.{column}`、`__scalar_subquery_{i}` 无碰撞。仅接受裸 `?`；`?N`（Placeholder("?N")）与 `$N` 点名拒绝（一套位置式方言）。
- 计数：AST 级 `count_placeholders(&Statement) -> usize`（parse 后即可校验；个数不符在 plan 前点名「期望 N 实际 M」）。`execute_sql` 内部等价 `params = &[]`——含占位符语句无参执行报同一个数错误。
- **表达式位**（SELECT/WHERE 谓词、投影项、UPDATE SET 右值）：`Expr::Value(Placeholder("?"))` → `ParameterExpression::new("__param_{i}")`（PlanBuilder 计数器分配）。UPDATE SET 的 `Expr::Value` Literal 臂遇 `?` 改路由 `UpdateValue::Expression(ParameterExpression)`（表达式臂执行器对旧行快照求值，参数 owned evaluate 真值）。
- **DML 值位**（INSERT VALUES / UPSERT 待插值与 DO UPDATE SET Literal 臂）：`InsertNode`/`UpsertNode` 新增 `param_slots: Vec<ValueParamSlot>` 字段（`ValueParamSlot { row: usize, col: usize, param_index: usize }`；Upsert 的 DO UPDATE 赋值以独立槽位形态记录），值槽计划期置 `Value::Null` 占位；pipeline DML 臂构造执行器前按参数物化 `values[r][c] = params[i]` / 赋值右值物化——类型门/NOT NULL/键列门/索引维护全部沿用执行器既有语义（占位 Null 豁免计划期检查，执行期门在真实值上跑，与字面量语句同源）。
- **不可达位点名拒绝**：DML WHERE 定位键位（`WHERE a = ?`——Key 为计划期物化，`extract_pk_from_where` 无值不可提取，`ddl_dml.rs:1976-1986`）、标识位、DDL 语句内占位符。delta spec 场景已同步修订（见 specs/sql-parameter-binding）。

### D6 参数执行通道

- `Database::execute_with_params(&self, sql: &str, params: &[Value]) -> Result<SqlOutcome, SqlError>`；`Database::execute_in_tx_with_params(&self, sql, params, tx)`。
- 流程：parse → `count_placeholders` 校验 → plan（占位符已编译为 ParameterExpression / param_slots）→ SELECT 树经 `inject_correlated_values` 同款 walk 注入 `__param_{i}` → DML 臂槽位物化 → execute_stage[_in_tx]。plan 为本次新建，注入直接作用于该计划（无需 clone）。
- 值借用通道限制面（`evaluate_ref` 降级，见调查结论）：谓词/UPDATE 表达式/投影项均走 owned evaluate 真值通道（MS30 T03-D2 修复面）；顶层投影位 `SELECT ?` 的求值通道在 Act 先行测试中锁定（TEXT 参数真值断言，若触发 evaluate_ref 降级则该位置点名拒绝并记录）。

### D7 可复用预编译语句（A5/A7）

- `Database` 新增 `schema_version: Arc<AtomicU64>`；pipeline 全部 `plan_cache.clear()` 站点收敛为单一 helper `bump_schema_version(db)`（clear + fetch_add 同点），版本戳与缓存失效同源。
- `Database::prepare(&self, sql: &str) -> Result<PreparedStatement, SqlError>`：parse + 计数 + plan 一次；`PreparedStatement { db: Database, sql: String, plan: PhysicalPlan, param_count: usize, schema_version: u64 }`（Database 为 Clone 的 Arc 集，持引用不延长文件生命周期语义——close 由调用者编排，与现 Arc<Database> 消费者同型）。
- `PreparedStatement::execute(&self, params: &[Value]) -> Result<SqlOutcome, SqlError>`：版本戳不匹配 → **透明 re-plan**（重跑 parse+plan，更新自持计划与版本戳，结果与直接执行一致）；匹配 → `plan.clone()` → 注入/物化 → execute_stage。**每次执行操作计划私有副本**——`&self` 并发安全（A7），无共享可变状态。
- `PreparedStatement::execute_in_tx(&self, params, tx)` 同款走 execute_stage_in_tx。
- prepare 语句面与 execute_sql 一致（单语句、无 attach 路由上下文——lib 面既有形态）；prepared 不读写 plan_cache（自持计划；SELECT「计划不经每次执行重建」由自持保证）；DDL 可 prepare，执行触发版本 bump → 后续执行透明 re-plan（幂等）。

### D8 事务与隔离

- 参数化执行覆盖隐式（execute_with_params）与显式事务（execute_in_tx_with_params / PreparedStatement::execute_in_tx），DML 消费 tx_id 语义与现 `execute_in_tx` 一致；`statement_snapshot` 穿线不变；不引入新隔离语义。

## 影响范围与责任边界

**修改**：`src/result.rs`（新）、`src/database.rs`（签名 + schema_version + prepare 家族）、`src/pipeline.rs`（Result 化 + 元数据装配 + 占位符编译/注入/物化 + bump helper + 参数通道）、`src/network/protocol.rs`（to_response + value_to_json 迁入消费）、`src/network/handler.rs`、`src/cli/mod.rs`、`src/cli/lifecycle.rs`、`src/storage/integrity.rs`（测试 helper）、`src/executor/plan.rs`（InsertNode/UpsertNode 增 param_slots 字段）、`src/executor/correlated.rs`（如需 DML 臂扩展）、`src/parser/planner/expression.rs`（Placeholder 臂）、`src/parser/planner/ddl_dml.rs`（值位/SET 臂/键位拒绝）、`src/parser/ast.rs` 或新 walk（count_placeholders）、tests 81 文件迁移 + 新增测试文件。

**保持（零改动或零行为变化）**：`render.rs`、`pg_protocol.rs`、JSON 协议序列化、执行器 `next()` 协议与全部执行器实现（Insert/Upsert 经槽位物化后消费既有形态）、plan cache key 语义与 DML/DDL 缓存纪律、错误文案与 SQLSTATE 分类、CLI 退出码、`value_to_json` 转换逻辑（仅移位）。

**禁止**：PhysicalPlan 节点类型集变更（仅对既有节点加字段）、执行器接口重构、plan cache key 语义重构、多套占位符方言、字符串替换式绑定。

## 实现顺序与依赖

1. **Iteration 000（T01）**：公共类型与边界转换 → 签名替换与全调用面迁移 → 列类型装配 → 全量回归。签名替换是原子面（编译完整性迫使 src + tests 同轮迁移），为最大体量轮。
2. **Iteration 001（T02 参数面）**：计数与拒绝面 → 计划面占位符编译 → execute_with_params 通道与错误矩阵 → 全量回归。依赖 000 的类型化结果形态。
3. **Iteration 002（T02 预编译）**：schema 版本 → prepare/PreparedStatement 与失效/并发/事务矩阵 → 全量回归。依赖 001 的参数通道。

## 风险与已知边界

- **R1 值借用通道限制面**：顶层投影位参数若触发 `evaluate_ref` String 降级，Act 先行测试暴露后按 D6 处理（该位点名拒绝）；谓词/UPDATE 表达式位已由 owned 通道覆盖（MS30 T03-D2）。
- **R2 迁移体量**：81 测试文件机械迁移，`to_response` helper 保持断言不动；全量分批回归（M19）验证。
- **R3 类型装配臂覆盖**：`plan_output_types` 与 names walk 臂结构对称但 Aggregate/派生/子查询列为 `None`（诚实边界，delta spec 场景已锁定）；值推导冲突（同列 Int∪Text）保持 `None` 而非 CTAS 的报错语义（元数据非建表，无冲突面）。
- **R4 DML WHERE 键位拒绝**为一期诚实边界（计划期值依赖）；后续批次若需求出现，需执行期键求值重构（独立设计），本 change 不预留半成品。
