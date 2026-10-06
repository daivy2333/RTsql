use crate::database::Database;
use crate::executor::{
    AggregateExecutor, AggregateNode, AlterTableExecutor, AntiJoinExecutor, CreateIndexExecutor,
    CreateTableAsSelectNode, CreateTableExecutor, CreateViewExecutor, DataScanExecutor,
    DeleteExecutor, DerivedScanExecutor, DistinctExecutor, DropIndexExecutor, DropTableExecutor,
    DropViewExecutor, ExecResult, Executor, ExplainExecutor, FilterExecutor, HavingExecutor,
    IndexScanAllExecutor, IndexScanExecutor, InsertExecutor, InsertNode, InsertSelectNode,
    JoinConfig, JoinExecutor, JoinRelatedConfig, LimitExecutor, NestedLoopJoinExecutor,
    PhysicalPlan, ProjectionExecutor, ScalarSubqueryFilterExecutor, ScanExecutor,
    SemiJoinExecutorV2, SetOperationExecutor, SingleRowExecutor, SortExecutor,
    SubqueryEvalExecutor, TruncateExecutor, UpdateExecutor, UpsertExecutor, UpsertNode, Value,
};
use crate::network::protocol::Response;
use crate::parser::planner::ViewDef;
use crate::parser::{parse_sql, PlanBuilder, PlanError};
use crate::profiling::{
    init_profiling, is_profiling_enabled, print_timings, record_time, with_profiling_scope,
};
use crate::storage::page_format::ColumnType as StorageColumnType;
use crate::storage::{Result, StorageError};
use crate::transaction::Snapshot;
use sqlparser::ast::{Expr, ObjectType, Query, SetExpr, Statement, TableFactor, TableWithJoins};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::time::Instant;

/// create_executor_from_plan 返回类型别名
///
/// 用于解决 type_complexity warning，简化 async 函数返回类型签名
type CreateExecutorFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Box<dyn Executor + Send>>> + Send + 'a>,
>;

/// MS20 T2（`cross-db-namespace-query`）：语句级路由结果——**计划期实际判定
/// 为附加库引用**的限定扁平名 → 附加库实例。
///
/// 执行面据此取库，而不是重新从计划节点的 `table_name` 推导：计划节点只
/// 带扁平名（`"o.u"`），对扁平串 split 会与主库字面表名 `"o.u"` 混淆
/// （R1/S4 的零回归守护约束）。键即 `register_table` 路由分支写入的键，
/// 计划与执行两侧对同一 key 的判定天然一致。
pub type RoutedTables = HashMap<String, RoutedTable>;

/// MS20 T2：一条已路由的附加库引用。`local_name` 是该库 catalog 内的真实表名
///（`"o.u"` → `"u"`）——catalog 以本地名索引，限定名只在会话路由层存在。
#[derive(Clone)]
pub struct RoutedTable {
    pub db: Arc<Database>,
    pub local_name: String,
}

/// MS20 T2：会话级附加库别名注册表（别名小写 → 打开的附加库实例）。
///
/// 由 CLI `AttachSession` 构造后随语句穿线；lib 缺省入口（`execute`/
/// `execute_stage`）不带路由，行为逐字节零变化。MS20-T02 起：别名表参与
/// 读面与写面路由（附加 DML 目标归附加库），DDL/CTAS 附加目标维持拒绝。
#[derive(Clone, Default)]
pub struct AttachRoute {
    dbs: HashMap<String, Arc<Database>>,
}

impl AttachRoute {
    pub fn new(dbs: HashMap<String, Arc<Database>>) -> Self {
        Self { dbs }
    }

    /// 首段是否命中已注册别名（别名键为小写，调用方传入的段已小写归一）。
    pub fn is_registered(&self, alias: &str) -> bool {
        self.dbs.contains_key(alias)
    }

    pub fn db_for_alias(&self, alias: &str) -> Option<&Arc<Database>> {
        self.dbs.get(alias)
    }
}

/// MS20 T2：表名提取的保留形态——既有扁平名（全部消费点的键，逐字节不变）
/// 加拍平前的分段（路由判定的唯一依据）。
#[derive(Clone)]
struct TableRef {
    /// `object_name_to_table_name` 结果——PlanBuilder / 计划节点 / 错误文案键
    flat: String,
    /// 逐段小写标识符；`len() == 1` 即裸名或字面含点表名
    parts: Vec<String>,
}

impl TableRef {
    fn new(name: &sqlparser::ast::ObjectName) -> Self {
        let parts = crate::parser::ast::object_name_parts(name);
        Self {
            flat: parts.join("."),
            parts,
        }
    }
}

pub async fn execute(database: &Database, sql: &str) -> Response {
    let profiling = is_profiling_enabled();

    if profiling {
        // Wrap entire execution in profiling scope for task-local storage
        return with_profiling_scope(execute_inner(database, sql)).await;
    }

    execute_inner(database, sql).await
}

/// Pipeline 阶段 1：SQL 文本 → AST。
///
/// `parse_sql` 失败时返回 `Err("Parse error: {e}")`；解析后语句集为空时返回
/// `Err("Empty SQL")`。编排器将 Err 文本包成 `Response::Error`。
pub async fn parse_stage(sql: &str) -> std::result::Result<Vec<Statement>, String> {
    let statements = parse_sql(sql).map_err(|e| format!("Parse error: {}", e))?;
    if statements.is_empty() {
        return Err("Empty SQL".to_string());
    }
    Ok(statements)
}

/// 解析阶段失败的 SQLSTATE（MS19-T02：语法错误统一 42601，含 "Empty SQL"
/// 退化形态——`parse_stage` 保持返回 `String`，分类在漏斗点赋予）。
const PARSE_SQLSTATE: &str = "42601";

/// Pipeline 阶段 2 的类型化失败（MS19-T02）：`Display` 与既有 `String` 错误
/// 逐字节等价（`Plan error: ` 前缀在此引入），额外携带 PG SQLSTATE 分类。
#[derive(Debug, Clone)]
pub struct PlanFailure {
    pub message: String,
    pub sqlstate: &'static str,
}

impl fmt::Display for PlanFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for PlanFailure {}

impl From<PlanError> for PlanFailure {
    fn from(e: PlanError) -> Self {
        Self {
            message: format!("Plan error: {}", e),
            sqlstate: e.sqlstate(),
        }
    }
}

/// Pipeline 阶段 2：AST → PhysicalPlan。
///
/// DDL 变体走 `PlanBuilder::new().build_plan`（不注册表，不做 table_metadata_lookup）。
/// 其余变体走 `register_table` → `build_plan` → 对可缓存语句（仅 SELECT）`put`。
///
/// `profiling: true` 时，`table_metadata_lookup` 子指标在该阶段内记录。
pub async fn plan_stage(
    database: &Database,
    sql: &str,
    stmt: &Statement,
    profiling: bool,
) -> std::result::Result<PhysicalPlan, PlanFailure> {
    plan_stage_routed(
        database,
        sql,
        stmt,
        profiling,
        None,
        &mut RoutedTables::new(),
    )
    .await
}

/// MS20 T2：带跨库路由上下文的 `plan_stage`（既有签名为委托壳，lib 路径恒
/// `route = None` → 行为逐字节不变）。
///
/// `route` 决定哪些限定名解析到附加库；`routed` 输出本语句实际路由的
/// 限定名 → 附加库，供同语句的执行面取库。含附加库引用的语句跳过 plan
/// cache 写——cache key 只含 SQL 文本，不含 attach 身份。
pub async fn plan_stage_routed(
    database: &Database,
    sql: &str,
    stmt: &Statement,
    profiling: bool,
    route: Option<&AttachRoute>,
    routed: &mut RoutedTables,
) -> std::result::Result<PhysicalPlan, PlanFailure> {
    match stmt {
        // MS20 T4: CTAS 两侧都要判定——写目标沿用 DDL 标记（附加库目标计划期
        // 拒绝），物化源经 `register_table` 走跨库路由（可读附加库）。`CTAS`
        // 不是 `Statement::Query`，不进 plan cache（DDL/DML 既有纪律）。
        Statement::CreateTable { query: Some(_), .. } => {
            let table_lookup_start = if profiling {
                Some(Instant::now())
            } else {
                None
            };
            let mut plan_builder = PlanBuilder::new();
            if let Some(route) = route {
                mark_attached_ddl_targets(&mut plan_builder, stmt, route);
            }
            register_table(database, &mut plan_builder, stmt, route, routed).await?;
            if let Some(start) = table_lookup_start {
                record_time("table_metadata_lookup", start.elapsed());
            }
            plan_builder.build_plan(stmt).map_err(PlanFailure::from)
        }
        Statement::CreateTable { .. }
        | Statement::Drop {
            object_type: ObjectType::Table,
            ..
        } => {
            let mut plan_builder = PlanBuilder::new();
            // MS20 T3: DDL 走此分支（不经 `register_table`），写目标的附加库
            // 引用判定在此独立完成——只需「首段命中别名」，不查 catalog。
            // MS21 Iter002：CREATE INDEX / DROP INDEX 不落此分支（前者需要
            // register_table 的表 schema 通道，后者无表名可标记）——附加库
            // 目标各自在其计划臂点名拒绝。
            if let Some(route) = route {
                mark_attached_ddl_targets(&mut plan_builder, stmt, route);
            }
            plan_builder.build_plan(stmt).map_err(PlanFailure::from)
        }
        // MS21 Iter005 (T5-b/D12)：CREATE VIEW 专用臂——视图通道注入（重名
        // 检查与 T5-c 展开钩子消费）+ 定义表名视图感知注册（附加库引用点名
        // 拒绝、既有视图引用合法）。DDL 不进 plan cache。
        Statement::CreateView { .. } => {
            let table_lookup_start = if profiling {
                Some(Instant::now())
            } else {
                None
            };
            let mut plan_builder = PlanBuilder::new();
            plan_builder.set_views(current_view_defs(database).await);
            register_view_definition_tables(database, &mut plan_builder, stmt, route).await?;
            if let Some(start) = table_lookup_start {
                record_time("table_metadata_lookup", start.elapsed());
            }
            plan_builder.build_plan(stmt).map_err(PlanFailure::from)
        }
        _ => {
            let table_lookup_start = if profiling {
                Some(Instant::now())
            } else {
                None
            };
            let mut plan_builder = PlanBuilder::new();
            // MS21 Iter005 (T5-c/D12)：语句级视图通道注入——FROM/JOIN 展开钩子
            // 与 register_table 视图豁免的共同消费面（`Database.views` 快照；
            // 无视图库为空表 → 全路径逐字节不变）。
            plan_builder.set_views(current_view_defs(database).await);
            // MS21 Iter002 (T2-c/T2-f/D14)：CREATE INDEX / DROP INDEX 的计划期
            // 名单——全库二级索引名一次性注入（重名 / 不存在检查）。
            if matches!(stmt, Statement::CreateIndex { .. } | Statement::Drop { .. }) {
                let names: HashSet<String> = database
                    .table_manager
                    .all_secondary_index_names()
                    .await
                    .into_iter()
                    .collect();
                plan_builder.set_existing_index_names(names);
            }
            // MS21 Iter003 (T3-b/D9)：ALTER TABLE 目标表的行存在性注入
            // （NOT NULL 无 DEFAULT × 非空的计划期判定输入）。
            if let Statement::AlterTable { name, .. } = stmt {
                let table_name = crate::parser::ast::object_name_to_table_name(name);
                let has_rows = database
                    .table_manager
                    .table_has_live_rows(&table_name)
                    .await
                    .unwrap_or(false);
                plan_builder.set_table_has_live_rows(&table_name, has_rows);
            }
            register_table(database, &mut plan_builder, stmt, route, routed).await?;
            if let Some(start) = table_lookup_start {
                record_time("table_metadata_lookup", start.elapsed());
            }
            let plan = plan_builder.build_plan(stmt).map_err(PlanFailure::from)?;
            // MS20 T2: attach 身份不在 cache key 内——含附加库引用的语句不得
            // 进缓存，否则同文本语句在「有/无该别名」的会话间互相污染。
            if is_cacheable(stmt) && routed.is_empty() {
                database.plan_cache.put(sql.to_string(), plan.clone());
            }
            Ok(plan)
        }
    }
}

/// Pipeline 阶段 3：PhysicalPlan → Response。
///
/// 路由：
/// - DDL（CreateTable / DropTable）：直包对应 Executor，执行后 `plan_cache.clear()`
/// - DML（Insert / Update / Delete）：`begin → prefetch abort meta → create_executor(tx_id) → execute → commit/abort`
/// - 其余（Query 路径）：`create_executor(None) → execute`
///
/// `profiling: true` 时，`executor_creation` 与 `executor_execution` 子指标在该阶段内记录。
pub async fn execute_stage(database: &Database, plan: PhysicalPlan, profiling: bool) -> Response {
    execute_stage_routed(database, plan, profiling, &RoutedTables::new()).await
}

/// MS20 T2：带路由上下文的 `execute_stage`（既有签名为委托壳，lib 路径恒空
/// 路由表 → 行为逐字节不变）。扫描臂按计划期判定的 `RoutedTables` 取库。
pub async fn execute_stage_routed(
    database: &Database,
    plan: PhysicalPlan,
    profiling: bool,
    routed: &RoutedTables,
) -> Response {
    // MS26 T2：只读拒绝面——6 个执行入口（`execute` / `execute_stage` /
    // `execute_stage_routed` / `execute_in_tx` / `execute_stage_in_tx` /
    // `execute_stage_in_tx_routed`）的委托链都收敛到本函数与
    // `execute_stage_in_tx_routed` 两个叶子，故在此与该叶子各挂一次即
    // 覆盖全部入口。
    if let Some(rejection) = readonly_rejection(database, &plan) {
        return rejection;
    }
    match &plan {
        PhysicalPlan::CreateTable(_) | PhysicalPlan::DropTable(_) => {
            let executor: Box<dyn Executor + Send> = match &plan {
                PhysicalPlan::CreateTable(_) => Box::new(CreateTableExecutor::new(
                    plan.clone(),
                    Arc::new(database.clone()),
                )),
                PhysicalPlan::DropTable(_) => Box::new(DropTableExecutor::new(
                    plan.clone(),
                    Arc::new(database.clone()),
                )),
                _ => unreachable!("DDL match already filtered"),
            };
            let response = execute_executor(executor).await;
            // 时序保持：DDL 执行后清缓存（与原 execute_inner 行为等价——成功后与失败后都清）。
            database.plan_cache.clear();
            response
        }
        // MS21 Iter002 (T2-c/T2-f)：CREATE/DROP INDEX 直包对应执行器（DDL 臂
        // 同型；执行后清缓存——引用该索引的 SELECT 计划若曾被缓存，全清封死
        // 复活窗口）。
        PhysicalPlan::CreateIndex(_) | PhysicalPlan::DropIndex(_) => {
            let executor: Box<dyn Executor + Send> = match &plan {
                PhysicalPlan::CreateIndex(_) => Box::new(CreateIndexExecutor::new(
                    plan.clone(),
                    Arc::new(database.clone()),
                )),
                PhysicalPlan::DropIndex(_) => Box::new(DropIndexExecutor::new(
                    plan.clone(),
                    Arc::new(database.clone()),
                )),
                _ => unreachable!("DDL match already filtered"),
            };
            let response = execute_executor(executor).await;
            database.plan_cache.clear();
            response
        }
        // MS21 Iter003 (T3-b/c)：ALTER TABLE 直包执行器（DDL 臂同型；执行后
        // 清缓存——引用该表的 SELECT 计划若曾被缓存，全清封死复活窗口）。
        PhysicalPlan::AlterTable(_) => {
            let executor: Box<dyn Executor + Send> = Box::new(AlterTableExecutor::new(
                plan.clone(),
                Arc::new(database.clone()),
            ));
            let response = execute_executor(executor).await;
            database.plan_cache.clear();
            response
        }
        // MS21 Iter004 (T4-a)：TRUNCATE 直包执行器（DDL 臂同型；执行后清缓存
        // ——引用该表的 SELECT 计划若曾被缓存，全清封死复活窗口）。
        PhysicalPlan::Truncate(_) => {
            let executor: Box<dyn Executor + Send> = Box::new(TruncateExecutor::new(
                plan.clone(),
                Arc::new(database.clone()),
            ));
            let response = execute_executor(executor).await;
            database.plan_cache.clear();
            response
        }
        // MS21 Iter005 (T5-b/T5-d)：CREATE/DROP VIEW 直包执行器（DDL 臂同型
        //；执行后清缓存——引用视图的 SELECT 计划内嵌展开子树，全清封死定义
        // 变更后旧计划复活窗口，spec R3）。
        PhysicalPlan::CreateView(_) | PhysicalPlan::DropView(_) => {
            let executor: Box<dyn Executor + Send> = match &plan {
                PhysicalPlan::CreateView(_) => Box::new(CreateViewExecutor::new(
                    plan.clone(),
                    Arc::new(database.clone()),
                )),
                PhysicalPlan::DropView(_) => Box::new(DropViewExecutor::new(
                    plan.clone(),
                    Arc::new(database.clone()),
                )),
                _ => unreachable!("DDL match already filtered"),
            };
            let response = execute_executor(executor).await;
            database.plan_cache.clear();
            response
        }
        PhysicalPlan::CreateTableAsSelect(node) => {
            // MS20 T4：CTAS 是混合面（SELECT 物化 + 建表 + 落行），没有单一
            // executor —— 走 `execute_ctas_routed`。缓存清理与 DDL 臂同款。
            let response = execute_ctas_routed(database, node, profiling, routed).await;
            database.plan_cache.clear();
            response
        }
        PhysicalPlan::InsertSelect(node) => {
            // MS20-T02 Iter001（design D4）：只读守卫已在函数顶部以 INSERT
            // 形态拒绝（先于源求值）。源**完整物化**成功后才经递归进入 DML
            // 臂——begin/abort_tables/abort/commit 与执行器接线不在此复制；
            // 同表写回只消费物化前行（物化先于 begin，结构性保证）。
            let snapshot = database.statement_snapshot(None).await;
            let rows =
                match materialize_insert_select_source(database, &node.source, snapshot, routed)
                    .await
                {
                    Ok(rows) => rows,
                    Err(message) => {
                        return Response::Error {
                            message,
                            sqlstate: None,
                        }
                    }
                };
            let normalized = match normalize_insert_select(node, rows) {
                Ok(plan) => plan,
                Err(message) => {
                    return Response::Error {
                        message,
                        sqlstate: None,
                    }
                }
            };
            Box::pin(execute_stage_routed(
                database, normalized, profiling, routed,
            ))
            .await
        }
        PhysicalPlan::Insert(_)
        | PhysicalPlan::Upsert(_)
        | PhysicalPlan::Update(_)
        | PhysicalPlan::Delete(_) => {
            // 目标名取自有 String（`plan` 随后被移入执行器构造，借自 `plan`
            // 的 `&str` 不能跨越该 move）。
            let target_name = match &plan {
                PhysicalPlan::Insert(n) => n.table_name.clone(),
                PhysicalPlan::Upsert(n) => n.table_name.clone(),
                PhysicalPlan::Update(n) => n.table_name.clone(),
                PhysicalPlan::Delete(n) => n.table_name.clone(),
                _ => unreachable!("is_dml guarantees a DML plan"),
            };
            // MS20-T02（design D1/D2）：DML 目标经计划期路由表选定目标库；
            // 非显式根在 begin 前同时检查实际目标只读（主库只读已在上方拦截，
            // 此处覆盖目标库判定）。abort 表键用 catalog 本地名——与
            // record_version / WAL 记录的表名一致。
            let (target_db, local_name) = routed_db(routed, database, &target_name);
            if let Some(rejection) = readonly_rejection(target_db, &plan) {
                return rejection;
            }

            // DML must run inside a real transaction (MS06-T01 spec) — the
            // transaction, buffer pool and WAL all belong to the target db.
            let tx = target_db.transaction_manager.begin().await;
            let tx_id = tx.id();

            let abort_tables = target_db
                .table_manager
                .get_table(local_name)
                .await
                .ok()
                .map(|tm| HashMap::from([(local_name.to_string(), tm)]));

            let executor_creation_start = if profiling {
                Some(Instant::now())
            } else {
                None
            };
            let executor =
                match create_executor_from_plan(plan, database, Some(tx_id), None, routed).await {
                    Ok(e) => e,
                    Err(e) => {
                        if let Some(abort_tables) = &abort_tables {
                            let _ = target_db
                                .transaction_manager
                                .abort(tx, &target_db.buffer_pool, abort_tables)
                                .await;
                        }
                        return Response::Error {
                            message: e.to_string(),
                            sqlstate: None,
                        };
                    }
                };
            if let Some(start) = executor_creation_start {
                record_time("executor_creation", start.elapsed());
            }

            let exec_start = if profiling {
                Some(Instant::now())
            } else {
                None
            };
            let response = execute_executor(executor).await;
            if let Some(start) = exec_start {
                record_time("executor_execution", start.elapsed());
            }

            match &response {
                Response::Error { .. } => {
                    if let Some(abort_tables) = &abort_tables {
                        if let Err(abort_err) = target_db
                            .transaction_manager
                            .abort(tx, &target_db.buffer_pool, abort_tables)
                            .await
                        {
                            return Response::Error {
                                message: format!("Abort failed: {}", abort_err),
                                sqlstate: None,
                            };
                        }
                    }
                }
                _ => {
                    // MS26 T5：收敛到目标库的 `Database::commit`（水位检查单点
                    // ——每个提交库按自身阈值检查，design D3）。既有文案形态
                    // 不变——内层错误已点名 `checkpoint failed`
                    // （`wal-auto-checkpoint` R4）；本臂不做破坏性清理。
                    if let Err(commit_err) = target_db.commit(tx).await {
                        return Response::Error {
                            message: format!("Commit failed: {}", commit_err),
                            sqlstate: None,
                        };
                    }
                }
            }
            response
        }
        _ => {
            // Query path (and any non-DML/non-DDL).
            // MS09 Iter000 (D4): under Read Committed, the statement
            // evaluates against a snapshot taken at statement start; the
            // default RepeatableRead path stays `None` (byte-for-byte
            // unchanged).
            let statement_snapshot = database.statement_snapshot(None).await;
            let executor_creation_start = if profiling {
                Some(Instant::now())
            } else {
                None
            };
            let executor =
                match create_executor_from_plan(plan, database, None, statement_snapshot, routed)
                    .await
                {
                    Ok(e) => e,
                    Err(e) => {
                        return Response::Error {
                            message: e.to_string(),
                            sqlstate: None,
                        };
                    }
                };
            if let Some(start) = executor_creation_start {
                record_time("executor_creation", start.elapsed());
            }

            let exec_start = if profiling {
                Some(Instant::now())
            } else {
                None
            };
            let response = execute_executor(executor).await;
            if let Some(start) = exec_start {
                record_time("executor_execution", start.elapsed());
            }
            response
        }
    }
}

/// MS26 T2：只读会话的写计划拒绝（`readonly-open-mode` R4，design D3）——
///
/// 计划形状是语句语义的权威判据：13 个写变体在只读会话中于执行入口顶部
/// （解析后、任何写入生效前）点名拒绝，SQLSTATE `25006`
/// （read_only_sql_transaction）。`is_readonly()` 为假时零成本直通。
pub(crate) fn readonly_rejection(db: &Database, plan: &PhysicalPlan) -> Option<Response> {
    if !db.is_readonly() {
        return None;
    }
    let kind = match plan {
        PhysicalPlan::Insert(_) => "INSERT",
        PhysicalPlan::Upsert(_) => "UPSERT",
        // MS20-T02 Iter001：INSERT SELECT 是 INSERT 形态，同一只读拒绝面
        //（新变体显式加臂——守卫先于源求值与目标写入）。
        PhysicalPlan::InsertSelect(_) => "INSERT",
        PhysicalPlan::Update(_) => "UPDATE",
        PhysicalPlan::Delete(_) => "DELETE",
        PhysicalPlan::CreateTable(_) => "CREATE TABLE",
        PhysicalPlan::CreateTableAsSelect(_) => "CREATE TABLE AS SELECT",
        PhysicalPlan::CreateIndex(_) => "CREATE INDEX",
        PhysicalPlan::AlterTable(_) => "ALTER TABLE",
        PhysicalPlan::DropIndex(_) => "DROP INDEX",
        PhysicalPlan::Truncate(_) => "TRUNCATE",
        PhysicalPlan::CreateView(_) => "CREATE VIEW",
        PhysicalPlan::DropView(_) => "DROP VIEW",
        PhysicalPlan::DropTable(_) => "DROP TABLE",
        // 防御臂：新增写变体未纳入枚举时不得误拒读形状（由契约测试在
        // Plan Review 侧核对枚举完备性）。
        _ => return None,
    };
    Some(Response::Error {
        message: format!("cannot execute {kind} in a read-only session"),
        sqlstate: Some(READ_ONLY_SQLSTATE.to_string()),
    })
}

/// 只读写拒绝的 SQLSTATE 分类（PG `read_only_sql_transaction`，MS19 R2 通道）。
const READ_ONLY_SQLSTATE: &str = "25006";

/// MS20-T02（design D2）：显式事务内附加库写入拒绝——写节点的**目标**命中
/// 本语句路由表即点名拒绝（SQLSTATE `0A000`），发生于执行器构造 / 源求值 /
/// begin 之前。只判目标：主库目标 + 附加源的语句不受影响（`routed` 非空不
/// 误拒主库写）；lib 路径路由表恒空，零变化。
fn attached_write_in_tx_rejection(plan: &PhysicalPlan, routed: &RoutedTables) -> Option<Response> {
    let target = match plan {
        PhysicalPlan::Insert(n) => &n.table_name,
        PhysicalPlan::Upsert(n) => &n.table_name,
        // MS20-T02 Iter001：INSERT SELECT 同为写面——目标命中附加路由即拒绝，
        // 先于源求值（守卫清单显式加臂，新变体不得逃逸）。
        PhysicalPlan::InsertSelect(n) => &n.table_name,
        PhysicalPlan::Update(n) => &n.table_name,
        PhysicalPlan::Delete(n) => &n.table_name,
        _ => return None,
    };
    if routed.contains_key(target) {
        return Some(Response::Error {
            message: format!(
                "WRITE TO ATTACHED DATABASE is not supported inside a transaction: '{target}'"
            ),
            sqlstate: Some("0A000".to_string()),
        });
    }
    None
}

/// MS10-T04：单结果执行路径（网络 / 显式事务）的多语句显式拒绝——
/// 两条路径共用同一文案；拒绝必须发生在 plan_stage / cache put 之前。
fn multi_statement_rejected(count: usize) -> Response {
    Response::Error {
        message: format!(
            "only a single statement is supported on this path (got {count} statements); use the CLI (rtsql) to run multi-statement scripts"
        ),
        sqlstate: None,
    }
}

/// MS20-T02 Iter001（design D4）：把 `InsertSelect` 源完整物化为原生 Value
/// 行——CTAS 同款消费模式（不经 JSON/SQL 文本往返，不流式）。任何错误都在
/// 目标 begin / 写入之前返回。
async fn materialize_insert_select_source(
    database: &Database,
    source: &PhysicalPlan,
    snapshot: Option<Snapshot>,
    routed: &RoutedTables,
) -> std::result::Result<Vec<Vec<Value>>, String> {
    let mut select =
        match create_executor_from_plan(source.clone(), database, None, snapshot, routed).await {
            Ok(executor) => executor,
            Err(e) => return Err(e.to_string()),
        };
    let mut rows: Vec<Vec<Value>> = Vec::new();
    loop {
        match select.next().await {
            Ok(Some(ExecResult::Row(values))) => rows.push(values),
            // 源计划恒为 SELECT（`build_insert_select` 产出），只有行输出；
            // RowId / AffectedRows 是写面信号，视为物化结束。
            Ok(Some(_)) | Ok(None) => break,
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(rows)
}

/// MS20-T02 Iter001（design D4）：物化行按预编译位置拼装全宽目标行，并把
/// 计划归一为既有 `Insert`/`Upsert`——类型/NOT NULL/PK/UNIQUE/二级索引/
/// 冲突动作语义全部沿用既有写面，不复制提交/回滚逻辑。运行期行宽不符点名
/// 错误，不静默 zip 截断。
fn normalize_insert_select(
    node: &InsertSelectNode,
    rows: Vec<Vec<Value>>,
) -> std::result::Result<PhysicalPlan, String> {
    let mut values: Vec<Vec<Value>> = Vec::with_capacity(rows.len());
    for row in rows {
        if row.len() != node.source_width {
            return Err(format!(
                "INSERT ... SELECT source row width {} does not match planned width {}",
                row.len(),
                node.source_width
            ));
        }
        let mut full = node.fill.clone();
        for (requested, &target) in node.target_positions.iter().enumerate() {
            full[target] = row[requested].clone();
        }
        values.push(full);
    }
    match &node.upsert {
        Some((arbiter, action)) => Ok(PhysicalPlan::Upsert(UpsertNode {
            table_name: node.table_name.clone(),
            values,
            arbiter: arbiter.clone(),
            action: action.clone(),
        })),
        None => Ok(PhysicalPlan::Insert(InsertNode {
            table_name: node.table_name.clone(),
            columns: node.columns.clone(),
            values,
        })),
    }
}

/// MS20 T4：CTAS 物化执行（`execute_stage_routed` 混合臂的实现体）。
///
/// 顺序固定（design D5）：物化 SELECT → 类型推导 → 建表 → `flush_all` →
/// 隐式事务落行。`flush_all` 必须在建表之后、任何数据 WAL 记录之前：catalog
/// 不写 WAL，redo 按表名查 catalog 缺表即 `RedoFailed` 变砖；先 flush 让
/// 「残留 schema 完整的空表」成为崩溃后唯一可达形态。
///
/// 零副作用：物化/类型推导/建表三处失败都在写入之前（既有表零变化）；落行失败
/// 走 abort + 尽力 drop_table，同名可重试。
async fn execute_ctas_routed(
    database: &Database,
    node: &CreateTableAsSelectNode,
    profiling: bool,
    routed: &RoutedTables,
) -> Response {
    // 0. 目标已存在 → 与建表通道同型（`DuplicateTable`）拒绝。提前于物化，
    //    免去为注定失败的语句跑完整个源结果集；错误面与「物化后建表被拒」
    //    一致，既有表零变化。
    if database.table_manager.table_exists(&node.table_name) {
        return Response::Error {
            message: StorageError::DuplicateTable(node.table_name.clone()).to_string(),
            sqlstate: None,
        };
    }

    // 1. 物化源 SELECT（语句级快照，与查询路径同款；源可指向附加库）。
    let statement_snapshot = database.statement_snapshot(None).await;
    let create_start = if profiling {
        Some(Instant::now())
    } else {
        None
    };
    let mut select = match create_executor_from_plan(
        *node.select.clone(),
        database,
        None,
        statement_snapshot,
        routed,
    )
    .await
    {
        Ok(executor) => executor,
        Err(e) => {
            return Response::Error {
                message: e.to_string(),
                sqlstate: None,
            }
        }
    };
    if let Some(start) = create_start {
        record_time("executor_creation", start.elapsed());
    }

    let exec_start = if profiling {
        Some(Instant::now())
    } else {
        None
    };
    let mut rows: Vec<Vec<Value>> = Vec::new();
    loop {
        match select.next().await {
            Ok(Some(ExecResult::Row(values))) => rows.push(values),
            // 源计划恒为 SELECT（`build_create_table_as_select` 产出），只有行
            // 输出；RowId / AffectedRows 是写面信号，视为物化结束。
            Ok(Some(_)) | Ok(None) => break,
            Err(e) => {
                return Response::Error {
                    message: e.to_string(),
                    sqlstate: None,
                }
            }
        }
    }
    if let Some(start) = exec_start {
        record_time("executor_execution", start.elapsed());
    }

    // 2. 类型推导（数据依赖）——不可行混合在此点名拒绝，catalog 仍零副作用。
    let schema = match infer_ctas_column_types(&node.columns, &rows) {
        Ok(schema) => schema,
        Err(failure) => {
            return Response::Error {
                message: failure.to_string(),
                sqlstate: Some(failure.sqlstate().to_string()),
            }
        }
    };

    // 3. 建表：首列沿用「未声明主键时首列为隐式主键」既有语义。
    let pk = node
        .columns
        .first()
        .cloned()
        .unwrap_or_else(|| "id".to_string());
    let columns: Vec<(String, StorageColumnType, bool, bool, Option<Value>)> = node
        .columns
        .iter()
        .cloned()
        .zip(schema)
        .map(|(name, column_type)| (name, column_type, false, false, None))
        .collect();
    if let Err(e) = database
        .table_manager
        .create_table_with_constraints(&node.table_name, columns, &pk)
        .await
    {
        return Response::Error {
            message: e.to_string(),
            sqlstate: None,
        };
    }

    // 5. catalog 落盘（先于任何数据 WAL 记录——见函数文档的崩溃一致性约束）。
    if let Err(e) = database.buffer_pool.flush_all().await {
        let _ = database.table_manager.drop_table(&node.table_name).await;
        return Response::Error {
            message: format!("Flush failed after CTAS create: {e}"),
            sqlstate: None,
        };
    }

    // 6. 隐式事务落行。
    let tx = database.transaction_manager.begin().await;
    let tx_id = tx.id();
    let table_meta = match database.table_manager.get_table(&node.table_name).await {
        Ok(table_meta) => table_meta,
        Err(e) => {
            let _ = database
                .transaction_manager
                .abort(tx, &database.buffer_pool, &HashMap::new())
                .await;
            let _ = database.table_manager.drop_table(&node.table_name).await;
            return Response::Error {
                message: e.to_string(),
                sqlstate: None,
            };
        }
    };
    let count = rows.len() as u64;
    let mut insert = InsertExecutor::with_table_manager(
        table_meta.clone(),
        Some(database.table_manager.clone()),
        database.buffer_pool.clone(),
        database.transaction_manager.clone(),
        rows,
        tx_id,
        database.wal_buffer.clone(),
    );
    let mut insert_failure: Option<String> = None;
    loop {
        match insert.next().await {
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(e) => {
                insert_failure = Some(e.to_string());
                break;
            }
        }
    }

    // 7. 收口：失败 → abort + 尽力 drop_table（零副作用：同名可重试）。
    if let Some(message) = insert_failure {
        let abort_tables = HashMap::from([(node.table_name.clone(), table_meta)]);
        let _ = database
            .transaction_manager
            .abort(tx, &database.buffer_pool, &abort_tables)
            .await;
        let _ = database.table_manager.drop_table(&node.table_name).await;
        return Response::Error {
            message,
            sqlstate: None,
        };
    }
    // 8. 收口：事务提交失败 → 尽力 drop_table（零副作用：同名可重试）。
    //    MS26 T5：水位检查**不在**本段内——它失败时事务已提交（WAL 已
    //    fsync、版本已标 committed），沿用上面的 drop_table 会删掉已提交
    //    数据的表，违反 `wal-auto-checkpoint` R4。故拆两段：事务提交本身
    //    失败才清理，提交成功后的 checkpoint 失败只点名传播（表存活）。
    if let Err(e) = database
        .transaction_manager
        .commit(tx, &database.buffer_pool)
        .await
    {
        let _ = database.table_manager.drop_table(&node.table_name).await;
        return Response::Error {
            message: format!("Commit failed: {e}"),
            sqlstate: None,
        };
    }
    if let Err(e) = database.maybe_auto_checkpoint().await {
        return Response::Error {
            message: e.to_string(),
            sqlstate: None,
        };
    }
    Response::AffectedRows { count }
}

/// MS20 T4：CTAS 输出列类型推导（design D5 step 3）。
///
/// 全 NULL → `STRING(255)`（沿用 DDL 路径 VARCHAR 默认长度，见
/// `ColumnDef::to_schema_column`）；同型保留；`Int∪Float` → `FLOAT`
/// （无损提升，与 MS24 FLOAT 列写面同语义）；其余混合点名拒绝。
///
/// MS29 T2：`pub(crate)` 供 CLI import 免建表导入复用同一推导矩阵
/// （逻辑零改动）。
pub(crate) fn infer_ctas_column_types(
    columns: &[String],
    rows: &[Vec<Value>],
) -> std::result::Result<Vec<StorageColumnType>, PlanError> {
    let mut inferred: Vec<StorageColumnType> = Vec::with_capacity(columns.len());
    for (index, name) in columns.iter().enumerate() {
        let mut current: Option<StorageColumnType> = None;
        for row in rows {
            // 行宽不足的槽位按 NULL 参与推导（缺列不制造类型冲突）。
            let Some(observed) = row.get(index).and_then(ctas_value_type) else {
                continue;
            };
            current = Some(match (current, observed) {
                (None, column_type) => column_type,
                (Some(previous), column_type) if previous == column_type => previous,
                // Int∪Float → FLOAT（无损提升，与 MS24 FLOAT 列写面同语义）
                (Some(StorageColumnType::Int), StorageColumnType::Float)
                | (Some(StorageColumnType::Float), StorageColumnType::Int) => {
                    StorageColumnType::Float
                }
                (Some(previous), column_type) => {
                    return Err(PlanError::CtasColumnTypeConflict(format!(
                        "{name} ({}, {})",
                        ctas_type_name(&previous),
                        ctas_type_name(&column_type)
                    )));
                }
            });
        }
        inferred.push(current.unwrap_or(StorageColumnType::String(255)));
    }
    Ok(inferred)
}

/// MS20 T4：单个 `Value` 的存储列类型；`NULL` 返回 `None`（参与任意类型）。
fn ctas_value_type(value: &Value) -> Option<StorageColumnType> {
    match value {
        Value::Null => None,
        Value::Int(_) => Some(StorageColumnType::Int),
        Value::Float(_) => Some(StorageColumnType::Float),
        Value::Bool(_) => Some(StorageColumnType::Bool),
        Value::String(_) => Some(StorageColumnType::String(255)),
        Value::Date(_) => Some(StorageColumnType::Date),
        Value::Timestamp(_) => Some(StorageColumnType::Timestamp),
    }
}

/// MS20 T4：类型点名文案（与 `insert.rs::column_type_name` 同款大写形态）。
fn ctas_type_name(column_type: &StorageColumnType) -> &'static str {
    match column_type {
        StorageColumnType::Int => "INT",
        StorageColumnType::String(_) => "STRING",
        StorageColumnType::Float => "FLOAT",
        StorageColumnType::Bool => "BOOL",
        StorageColumnType::Date => "DATE",
        StorageColumnType::Timestamp => "TIMESTAMP",
    }
}

/// Execute one SQL statement inside an existing user transaction (MS07-T04).
///
/// Reuses the same parse/plan stages as the implicit pipeline; execution
/// goes through `execute_stage_in_tx`, which threads the user's `tx_id`
/// into DML executors and skips the implicit begin/commit/abort wrapping.
pub async fn execute_in_tx(database: &Database, sql: &str, tx_id: u64) -> Response {
    let statements = match parse_stage(sql).await {
        Ok(s) => s,
        Err(message) => {
            return Response::Error {
                message,
                sqlstate: Some(PARSE_SQLSTATE.to_string()),
            }
        }
    };
    if statements.len() > 1 {
        return multi_statement_rejected(statements.len());
    }
    let stmt = match statements.first() {
        Some(s) => s,
        None => {
            return Response::Error {
                message: "No statement executed".to_string(),
                sqlstate: None,
            }
        }
    };
    let plan = match plan_stage(database, sql, stmt, false).await {
        Ok(p) => p,
        Err(failure) => {
            return Response::Error {
                message: failure.to_string(),
                sqlstate: Some(failure.sqlstate.to_string()),
            }
        }
    };
    execute_stage_in_tx(database, plan, tx_id).await
}

/// Stage-3 execution for a user transaction (MS07-T04).
///
/// DML nodes consume the caller's `tx_id` (`create_executor_from_plan`
/// contract from MS06-T01) and run without any implicit transaction
/// wrapping. Query and other nodes receive `Some(tx_id)` as well but ignore
/// it for visibility (scans stay `snapshot: None`, semantics unchanged).
/// DDL executes immediately and clears the plan cache, mirroring the
/// implicit path. A failed statement returns an error response and leaves
/// the transaction alive for the caller to commit or roll back.
pub async fn execute_stage_in_tx(database: &Database, plan: PhysicalPlan, tx_id: u64) -> Response {
    execute_stage_in_tx_routed(database, plan, tx_id, &RoutedTables::new()).await
}

/// MS20 T2：带路由上下文的 `execute_stage_in_tx`（显式事务内跨库 SELECT 走此
/// 路径——主库部分用会话事务，附加库部分各用自身快照，design 假设 4）。
pub async fn execute_stage_in_tx_routed(
    database: &Database,
    plan: PhysicalPlan,
    tx_id: u64,
    routed: &RoutedTables,
) -> Response {
    // MS26 T2：只读拒绝面（与 `execute_stage_routed` 同点，理由见该处）。
    // 优先级高于既有的「显式事务内不支持」0A000 拒绝——只读会话的写语句
    // 先按只读状态点名，事务保持活跃（沿既有语句级失败语义）。
    if let Some(rejection) = readonly_rejection(database, &plan) {
        return rejection;
    }
    // MS20-T02（design D2）：显式事务内附加库写入点名拒绝（0A000），先于
    // 执行器构造 / 源求值 / begin——会话事务（主库）保持活跃，由 CLI 失败
    // 收口回滚。
    if let Some(rejection) = attached_write_in_tx_rejection(&plan, routed) {
        return rejection;
    }
    match &plan {
        PhysicalPlan::CreateTable(_)
        | PhysicalPlan::DropTable(_)
        // MS21 Iter005 (T5-b/T5-d/D13)：CREATE/DROP VIEW 沿 CREATE TABLE
        // 现状——catalog-only，立即生效、不随回滚撤销。
        | PhysicalPlan::CreateView(_)
        | PhysicalPlan::DropView(_) => {
            let executor: Box<dyn Executor + Send> = match &plan {
                PhysicalPlan::CreateTable(_) => Box::new(CreateTableExecutor::new(
                    plan.clone(),
                    Arc::new(database.clone()),
                )),
                PhysicalPlan::DropTable(_) => Box::new(DropTableExecutor::new(
                    plan.clone(),
                    Arc::new(database.clone()),
                )),
                PhysicalPlan::CreateView(_) => Box::new(CreateViewExecutor::new(
                    plan.clone(),
                    Arc::new(database.clone()),
                )),
                PhysicalPlan::DropView(_) => Box::new(DropViewExecutor::new(
                    plan.clone(),
                    Arc::new(database.clone()),
                )),
                _ => unreachable!("DDL match already filtered"),
            };
            let response = execute_executor(executor).await;
            database.plan_cache.clear();
            response
        }
        PhysicalPlan::CreateTableAsSelect(_) => {
            // MS20 T4：CTAS 的物化面要求「建表 + 一次独立隐式事务」整体原子，
            // 显式事务内的会话态（含用户已执行的写）不能被中途 abort /
            // drop_table 破坏 —— 点名拒绝。拒绝点在 execute 之前、会话事务
            // 之外，CLI 与 lib 两条在途路径共用同一文案。
            Response::Error {
                message: "CREATE TABLE AS SELECT is not supported inside a transaction".to_string(),
                sqlstate: Some("0A000".to_string()),
            }
        }
        // MS21 Iter002 (D13/T2-c)：显式事务内 CREATE/DROP INDEX 点名拒绝
        // （0A000，CTAS 先例同型——回填与登记一致性依赖无并发未提交写窗口）。
        PhysicalPlan::CreateIndex(_) => Response::Error {
            message: "CREATE INDEX is not supported inside a transaction".to_string(),
            sqlstate: Some("0A000".to_string()),
        },
        PhysicalPlan::DropIndex(_) => Response::Error {
            message: "DROP INDEX is not supported inside a transaction".to_string(),
            sqlstate: Some("0A000".to_string()),
        },
        // MS21 Iter003 (T3-b/D13)：显式事务内 ALTER 执行期点名拒绝（0A000，
        // D13——ADD/DROP 的重写一致性依赖无并发未提交写窗口）。
        PhysicalPlan::AlterTable(_) => Response::Error {
            message: "ALTER TABLE is not supported inside a transaction".to_string(),
            sqlstate: Some("0A000".to_string()),
        },
        // MS21 Iter004 (T4-a/D13)：显式事务内 TRUNCATE 执行期点名拒绝（0A000，
        // 物理页释放不可回滚；拒绝先于执行器产生任何副作用）。
        PhysicalPlan::Truncate(_) => Response::Error {
            message: "TRUNCATE TABLE is not supported inside a transaction".to_string(),
            sqlstate: Some("0A000".to_string()),
        },
        PhysicalPlan::InsertSelect(node) => {
            // MS20-T02 Iter001（design D4）：显式事务内**主库目标**的 INSERT
            // SELECT——只读（25006）与附加目标（0A000）拒绝已在本函数顶部
            // 完成（先于源求值）。物化沿用会话事务的语句级快照（自身未提交
            // 写可见），插入复用该 tx_id，不另开提交点。
            let snapshot = database.statement_snapshot(Some(tx_id)).await;
            let rows =
                match materialize_insert_select_source(database, &node.source, snapshot, routed)
                    .await
                {
                    Ok(rows) => rows,
                    Err(message) => {
                        return Response::Error {
                            message,
                            sqlstate: None,
                        }
                    }
                };
            let normalized = match normalize_insert_select(node, rows) {
                Ok(plan) => plan,
                Err(message) => {
                    return Response::Error {
                        message,
                        sqlstate: None,
                    }
                }
            };
            // 归一后的 Insert/Upsert 落 `_` 通用臂：会话事务保持活跃，提交/
            // 回滚由调用方收口。
            Box::pin(execute_stage_in_tx_routed(database, normalized, tx_id, routed)).await
        }
        _ => {
            // MS09 Iter000 (D4): in-transaction statements take their
            // statement-start snapshot with the transaction's own id as the
            // reader (self writes stay visible via `is_visible_self`); the
            // default RepeatableRead path stays `None`.
            let statement_snapshot = database.statement_snapshot(Some(tx_id)).await;
            match create_executor_from_plan(plan, database, Some(tx_id), statement_snapshot, routed)
                .await
            {
                Ok(executor) => execute_executor(executor).await,
                Err(e) => Response::Error {
                    message: e.to_string(),
                    sqlstate: None,
                },
            }
        }
    }
}

async fn execute_inner(database: &Database, sql: &str) -> Response {
    let profiling = is_profiling_enabled();

    if profiling {
        init_profiling();
    }

    let total_start = if profiling {
        Some(Instant::now())
    } else {
        None
    };

    // Step 1: cache lookup (cache_hit_check 子指标保持)
    let cache_hit_check_start = if profiling {
        Some(Instant::now())
    } else {
        None
    };
    let cached_plan = database.plan_cache.get(sql);
    if let Some(start) = cache_hit_check_start {
        record_time("cache_hit_check", start.elapsed());
    }

    if let Some(plan) = cached_plan {
        // Cache hit — skip parse + plan, directly execute.
        let response = execute_stage(database, plan, profiling).await;
        if let Some(start) = total_start {
            print_timings(start.elapsed());
        }
        return response;
    }

    // Cache miss — three-stage pipeline.

    // Stage 1: parse
    let parse_start = if profiling {
        Some(Instant::now())
    } else {
        None
    };
    let statements = match parse_stage(sql).await {
        Ok(s) => s,
        Err(message) => {
            if let Some(start) = parse_start {
                record_time("parse", start.elapsed());
            }
            return Response::Error {
                message,
                sqlstate: Some(PARSE_SQLSTATE.to_string()),
            };
        }
    };
    if let Some(start) = parse_start {
        record_time("parse", start.elapsed());
    }
    // MS10-T04：单结果执行路径仅支持单语句——多语句从静默截断改为显式拒绝，
    // 拒绝发生在 plan_stage / cache put 之前（完整串键污染随之消失）。
    if statements.len() > 1 {
        return multi_statement_rejected(statements.len());
    }
    let stmt = match statements.first() {
        Some(s) => s,
        None => {
            return Response::Error {
                message: "No statement executed".to_string(),
                sqlstate: None,
            };
        }
    };

    // Stage 2: plan
    let plan_start = if profiling {
        Some(Instant::now())
    } else {
        None
    };
    let plan = match plan_stage(database, sql, stmt, profiling).await {
        Ok(p) => p,
        Err(failure) => {
            if let Some(start) = plan_start {
                record_time("plan", start.elapsed());
            }
            return Response::Error {
                message: failure.to_string(),
                sqlstate: Some(failure.sqlstate.to_string()),
            };
        }
    };
    if let Some(start) = plan_start {
        record_time("plan", start.elapsed());
    }

    // Stage 3: execute
    let execute_start = if profiling {
        Some(Instant::now())
    } else {
        None
    };
    let response = execute_stage(database, plan, profiling).await;
    if let Some(start) = execute_start {
        record_time("execute", start.elapsed());
    }

    if let Some(start) = total_start {
        print_timings(start.elapsed());
    }
    response
}

/// Execute an executor and return the response
async fn execute_executor(mut executor: Box<dyn Executor + Send>) -> Response {
    let mut rows: Vec<Vec<serde_json::Value>> = Vec::new();
    let mut affected_rows: Option<u64> = None;

    loop {
        match executor.next().await {
            Ok(Some(ExecResult::Row(values))) => {
                rows.push(values.into_iter().map(value_to_json).collect());
            }
            Ok(Some(ExecResult::AffectedRows(count))) => {
                affected_rows = Some(count);
            }
            Ok(Some(ExecResult::RowId(_))) => {}
            Ok(None) => break,
            Err(e) => {
                return Response::Error {
                    message: format!("Execution error: {}", e),
                    sqlstate: None,
                }
            }
        }
    }

    if let Some(count) = affected_rows {
        Response::AffectedRows { count }
    } else {
        Response::QueryResult { rows }
    }
}

/// MS20 T2：计划节点的 `table_name` 若在本语句的路由表内则取该附加库实例，
/// 否则取主库。键来自计划期判定（限定扁平名），不重新解析分段。
fn routed_db<'a>(
    routed: &'a RoutedTables,
    database: &'a Database,
    table_name: &'a str,
) -> (&'a Database, &'a str) {
    match routed.get(table_name) {
        Some(entry) => (entry.db.as_ref(), entry.local_name.as_str()),
        None => (database, table_name),
    }
}

/// MS20 T2：按路由取附加库（或主库）的表元数据——catalog 以本地名索引，故
/// 命中路由时以 `local_name` 取表。
///
/// 限定扁平名留在计划节点（`node.table_name`）与列引用上，不改写
/// `TableMeta.name`：写执行器的目标资源经 `routed_db` 取目标库，WAL/版本
/// 记录使用目标 catalog 本地名（MS20-T02）。
async fn load_table_meta(
    routed: &RoutedTables,
    database: &Database,
    table_name: &str,
) -> crate::storage::Result<std::sync::Arc<crate::storage::data::TableMeta>> {
    let (db, local_name) = routed_db(routed, database, table_name);
    db.table_manager.get_table(local_name).await
}

/// Create executor from physical plan (recursive for Filter)
///
/// `tx_id` is `Some(real_tx_id)` for DML nodes (Insert/Update/Delete) and `None`
/// for SELECT-side nodes (Scan/Filter/Join/Aggregate/...). Callers must wrap DML
/// in a real `Transaction` from `TransactionManager::begin()`; see `execute_inner`.
///
/// `snapshot` is the statement-start Read Committed snapshot (MS09 Iter000,
/// D4) — `None` under the default RepeatableRead path, which keeps every scan
/// construction byte-for-byte unchanged. It threads to all scan nodes and to
/// subquery/derived-table rebuilds.
///
/// `routed` 是本语句计划期判定的跨库路由表（MS20 T2）：键为限定扁平名
/// （`"o.u"`），值是该限定名所属的附加库实例。扫描臂据此取 schema、缓冲池与
/// 事务管理器——**附加库必须用自己的 `transaction_manager`**：tx-id 空间按库
/// 独立，串用主库实例会错误抑制墓碑（design D6）。写执行器臂同样经
/// `routed_db` 取目标库资源（MS20-T02）。
pub(crate) fn create_executor_from_plan<'a>(
    plan: PhysicalPlan,
    database: &'a Database,
    tx_id: Option<u64>,
    snapshot: Option<Snapshot>,
    routed: &'a RoutedTables,
) -> CreateExecutorFuture<'a> {
    Box::pin(async move {
        match plan {
            PhysicalPlan::Filter(node) => {
                // Recursively create input executor
                let input = create_executor_from_plan(
                    *node.input,
                    database,
                    tx_id,
                    snapshot.clone(),
                    routed,
                )
                .await?;
                Ok(Box::new(
                    FilterExecutor::new(input, node.predicate).with_projection(node.projection),
                ) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Scan(node) => {
                let db = routed_db(routed, database, &node.table_name).0;
                let table_meta = load_table_meta(routed, database, &node.table_name).await?;
                Ok(Box::new(
                    ScanExecutor::new(table_meta, db.buffer_pool.clone(), snapshot)
                        .with_projection(node.projection),
                ) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::DataScan(node) => {
                let db = routed_db(routed, database, &node.table_name).0;
                let table_meta = load_table_meta(routed, database, &node.table_name).await?;
                Ok(Box::new(
                    DataScanExecutor::new(
                        table_meta,
                        db.buffer_pool.clone(),
                        snapshot,
                        node.predicate,
                        node.scan_cap,
                        Some(db.transaction_manager.clone()),
                    )
                    .with_projection(node.projection),
                ) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::IndexScan(node) => {
                let db = routed_db(routed, database, &node.table_name).0;
                let table_meta = load_table_meta(routed, database, &node.table_name).await?;
                Ok(Box::new(
                    IndexScanExecutor::new(
                        table_meta,
                        db.buffer_pool.clone(),
                        node.key.as_bytes().to_vec(),
                        snapshot,
                    )
                    .with_projection(node.projection),
                ) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::IndexScanAll(node) => {
                let db = routed_db(routed, database, &node.table_name).0;
                let table_meta = load_table_meta(routed, database, &node.table_name).await?;
                // MS21 Iter002 (T2-g/D6)：二级索引路由命中时按登记名取该树；
                // None（既有路径）保持 PK 树搜索，构造逐字节不变。
                let mut executor = IndexScanAllExecutor::new(
                    table_meta.clone(),
                    db.buffer_pool.clone(),
                    node.key.as_bytes().to_vec(),
                    snapshot,
                );
                if let Some(index_name) = &node.index_name {
                    let index = table_meta
                        .secondary_indexes
                        .iter()
                        .find(|s| &s.name == index_name)
                        .map(|s| s.index.clone())
                        .ok_or_else(|| {
                            StorageError::Internal(format!(
                                "plan routes to unregistered secondary index '{}'",
                                index_name
                            ))
                        })?;
                    executor = executor.with_index_manager(index);
                }
                Ok(Box::new(executor.with_projection(node.projection)) as Box<dyn Executor + Send>)
            }

            // MS20-T02（design D1）：四个写构造臂经既有路由取目标库——附加库
            // 目标使用其自身 TableManager/BufferPool/TransactionManager/WAL 与
            // catalog 本地表名；主库目标沿 `routed_db` 未命中回落，逐字节不变。
            PhysicalPlan::Insert(node) => {
                let db = routed_db(routed, database, &node.table_name).0;
                let table_meta = load_table_meta(routed, database, &node.table_name).await?;
                Ok(Box::new(InsertExecutor::with_table_manager(
                    table_meta,
                    Some(db.table_manager.clone()),
                    db.buffer_pool.clone(),
                    db.transaction_manager.clone(),
                    node.values,
                    tx_id.expect("DML Insert requires a transaction id"),
                    db.wal_buffer.clone(),
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Upsert(node) => {
                let db = routed_db(routed, database, &node.table_name).0;
                let table_meta = load_table_meta(routed, database, &node.table_name).await?;
                Ok(Box::new(UpsertExecutor::with_table_manager(
                    table_meta,
                    Some(db.table_manager.clone()),
                    db.buffer_pool.clone(),
                    db.transaction_manager.clone(),
                    node.arbiter.clone(),
                    node.action.clone(),
                    node.values,
                    tx_id.expect("DML Upsert requires a transaction id"),
                    db.wal_buffer.clone(),
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Update(node) => {
                let db = routed_db(routed, database, &node.table_name).0;
                let table_meta = load_table_meta(routed, database, &node.table_name).await?;
                Ok(Box::new(UpdateExecutor::new(
                    table_meta,
                    db.buffer_pool.clone(),
                    db.transaction_manager.clone(),
                    node.key.as_bytes().to_vec(),
                    node.assignments,
                    tx_id.expect("DML Update requires a transaction id"),
                    db.wal_buffer.clone(),
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Delete(node) => {
                let db = routed_db(routed, database, &node.table_name).0;
                let table_meta = load_table_meta(routed, database, &node.table_name).await?;
                Ok(Box::new(DeleteExecutor::new(
                    table_meta,
                    db.buffer_pool.clone(),
                    db.transaction_manager.clone(),
                    node.key.as_bytes().to_vec(),
                    tx_id.expect("DML Delete requires a transaction id"),
                    db.wal_buffer.clone(),
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::CreateTable(_) | PhysicalPlan::DropTable(_) => {
                panic!("DDL should be handled separately in execute()")
            }

            // MS21 Iter002 (T2-c/T2-f)：索引 DDL 由 execute_stage 的 DDL 直包
            // 臂执行，无单一执行器构造臂。MS21 Iter003：ALTER TABLE 同型。
            // MS21 Iter004：TRUNCATE 同型。MS21 Iter005：CREATE/DROP VIEW 同型。
            PhysicalPlan::CreateIndex(_)
            | PhysicalPlan::DropIndex(_)
            | PhysicalPlan::AlterTable(_)
            | PhysicalPlan::Truncate(_)
            | PhysicalPlan::CreateView(_)
            | PhysicalPlan::DropView(_) => {
                panic!("DDL should be handled separately in execute()")
            }

            // MS20 T4: CTAS 是混合面，由 `execute_ctas_routed` 物化，不存在
            // 单一 executor。
            PhysicalPlan::CreateTableAsSelect(_) => {
                panic!("CTAS should be handled by execute_ctas_routed()")
            }

            // MS20-T02 Iter001：INSERT SELECT 由两个 execute 叶子先物化归一为
            // Insert/Upsert 再进入写路径，本函数只见归一后的计划。
            PhysicalPlan::InsertSelect(_) => {
                panic!("INSERT SELECT should be materialized by the execute leaves")
            }

            PhysicalPlan::Aggregate(node) => {
                let AggregateNode {
                    input,
                    group_by,
                    group_key_exprs,
                    aggregates,
                    aggregate_arg_indices,
                    output_columns,
                    table_name: _,
                    column_indices,
                } = node;
                let input_executor =
                    create_executor_from_plan(*input, database, tx_id, snapshot.clone(), routed)
                        .await?;
                Ok(Box::new(AggregateExecutor::new(
                    input_executor,
                    group_by,
                    group_key_exprs,
                    aggregates,
                    aggregate_arg_indices,
                    output_columns,
                    column_indices,
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Having(node) => {
                let input = create_executor_from_plan(
                    *node.input,
                    database,
                    tx_id,
                    snapshot.clone(),
                    routed,
                )
                .await?;
                Ok(Box::new(HavingExecutor::new(input, node.predicate))
                    as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Sort(node) => {
                // Recursively create input executor
                let input = create_executor_from_plan(
                    *node.input,
                    database,
                    tx_id,
                    snapshot.clone(),
                    routed,
                )
                .await?;
                Ok(Box::new(
                    SortExecutor::new(input, node.order_by, node.columns)
                        .with_projection(node.projection),
                ) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Limit(node) => {
                // Recursively create input executor
                let input = create_executor_from_plan(
                    *node.input,
                    database,
                    tx_id,
                    snapshot.clone(),
                    routed,
                )
                .await?;
                Ok(Box::new(LimitExecutor::new(input, node.limit, node.offset))
                    as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Join(join_node) => {
                // Build column index mappings from ScanNodes (must do before moving)
                // T9 replan：链首表名不再参与归属（output_sides 侧标志取代），
                // 名字结果弃置；对左输入的调用保留——嵌套链形态既有错误传播面。
                let (mut left_column_indices, _) = extract_column_indices(&join_node.left)?;
                let (mut right_column_indices, right_table_name) =
                    extract_column_indices(&join_node.right)?;
                // T11（change 2026-10-04-external-review-gaps）：注册
                // `(真实表, 列)` 限定键——链上同名列条件键按限定解析不串列
                //（执行器限定优先、裸名回退；限定键唯一命中才注册）。
                register_join_qualified_keys(&mut left_column_indices, &join_node.left);
                register_join_qualified_keys(&mut right_column_indices, &join_node.right);

                // Build left executor recursively
                let left_executor = create_executor_from_plan(
                    *join_node.left,
                    database,
                    tx_id,
                    snapshot.clone(),
                    routed,
                )
                .await?;

                // Build right executor recursively
                let right_executor = create_executor_from_plan(
                    *join_node.right,
                    database,
                    tx_id,
                    snapshot.clone(),
                    routed,
                )
                .await?;

                Ok(Box::new(JoinExecutor::new(JoinConfig {
                    left_executor,
                    right_executor,
                    conditions: join_node.conditions.clone(),
                    output_columns: join_node.output_columns.clone(),
                    output_sides: join_node.output_sides.clone(),
                    left_column_indices,
                    right_column_indices,
                    right_table_name,
                })) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::NestedLoopJoin(node) => {
                // T9 replan：链首表名不再参与归属（output_sides 侧标志取代）。
                // 保留对左输入的列索引解析调用——嵌套链形态既有错误传播面，
                // 结果弃置。
                extract_column_indices(&node.left)?;

                // Build left and right executors recursively
                let left_executor = create_executor_from_plan(
                    *node.left,
                    database,
                    tx_id,
                    snapshot.clone(),
                    routed,
                )
                .await?;
                let right_executor = create_executor_from_plan(
                    *node.right,
                    database,
                    tx_id,
                    snapshot.clone(),
                    routed,
                )
                .await?;

                Ok(Box::new(NestedLoopJoinExecutor::new(
                    left_executor,
                    right_executor,
                    node.predicate.clone(),
                    node.output_columns.clone(),
                    node.output_sides.clone(),
                    node.join_type,
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::SemiJoin(node) => {
                // Build column index mapping for right table (like JoinExecutor does)
                let (right_column_indices, _) = extract_column_indices(&node.right)?;

                // Build column index mapping for left table (need all columns for condition lookup)
                let (left_column_indices, _) = extract_column_indices(&node.left)?;

                // For correlated subqueries, clone the right plan before consuming
                let right_plan = if !node.correlated_params.is_empty() {
                    Some((*node.right).clone())
                } else {
                    None
                };

                // Build left and right executors recursively
                let left_executor = create_executor_from_plan(
                    *node.left,
                    database,
                    tx_id,
                    snapshot.clone(),
                    routed,
                )
                .await?;
                let right_executor = create_executor_from_plan(
                    *node.right,
                    database,
                    tx_id,
                    snapshot.clone(),
                    routed,
                )
                .await?;

                Ok(Box::new(SemiJoinExecutorV2::new(JoinRelatedConfig {
                    left: left_executor,
                    right: right_executor,
                    conditions: node.conditions,
                    output_columns: node.output_columns,
                    correlated_params: node.correlated_params,
                    left_column_indices,
                    right_column_indices,
                    right_plan,
                    database: Some(Arc::new(database.clone())),
                    snapshot,
                    routed_tables: Arc::new(routed.clone()),
                })) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::AntiJoin(node) => {
                // Build column index mapping for right table (like SemiJoin does)
                let (right_column_indices, _) = extract_column_indices(&node.right)?;

                // Build column index mapping for left table (need all columns for condition lookup)
                let (left_column_indices, _) = extract_column_indices(&node.left)?;

                // Save right plan clone for correlated per-row rebuild before consuming node.right
                let right_plan = if !node.correlated_params.is_empty() {
                    Some((*node.right).clone())
                } else {
                    None
                };
                // Build left and right executors recursively
                let left_executor = create_executor_from_plan(
                    *node.left,
                    database,
                    tx_id,
                    snapshot.clone(),
                    routed,
                )
                .await?;
                let right_executor = create_executor_from_plan(
                    *node.right,
                    database,
                    tx_id,
                    snapshot.clone(),
                    routed,
                )
                .await?;

                Ok(Box::new(AntiJoinExecutor::new(JoinRelatedConfig {
                    left: left_executor,
                    right: right_executor,
                    conditions: node.conditions,
                    output_columns: node.output_columns,
                    correlated_params: node.correlated_params,
                    left_column_indices,
                    right_column_indices,
                    right_plan,
                    database: Some(Arc::new(database.clone())),
                    snapshot,
                    routed_tables: Arc::new(routed.clone()),
                })) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::SubqueryEval(node) => {
                // T4（change 2026-10-04-external-review-gaps）：外层列位映射
                // 按输入链的收窄后行形状解析（相关参数按行取值；SELECT 投影
                // ∪ 相关参数列补充）。扫描类/链式形状列名序可得，其余形状
                // 回落既有 name→pos 映射（恒等行，行为与引入前一致）。
                let outer_column_indices = match plan_output_names(&node.input) {
                    Some(names) => names
                        .into_iter()
                        .enumerate()
                        .map(|(idx, col)| (col.to_lowercase(), idx))
                        .collect(),
                    None => extract_column_indices(&node.input)?.0,
                };
                let input_executor = create_executor_from_plan(
                    *node.input,
                    database,
                    tx_id,
                    snapshot.clone(),
                    routed,
                )
                .await?;
                Ok(Box::new(SubqueryEvalExecutor::with_output_width(
                    input_executor,
                    *node.subquery,
                    node.output_column.clone(),
                    node.result_column_index,
                    node.correlated_params.clone(),
                    outer_column_indices,
                    Arc::new(database.clone()),
                    snapshot,
                    Arc::new(routed.clone()),
                    node.output_width,
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::ScalarSubqueryFilter(node) => {
                // T4（change 2026-10-04-external-review-gaps）：外层列位映射按
                // 输入链全形状解析（谓词与相关参数在裁剪前的全行上求值）。
                let (outer_column_indices, _) = extract_column_indices(&node.input)?;
                let input_executor = create_executor_from_plan(
                    *node.input,
                    database,
                    tx_id,
                    snapshot.clone(),
                    routed,
                )
                .await?;
                Ok(Box::new(ScalarSubqueryFilterExecutor::new(
                    input_executor,
                    node.predicate.clone(),
                    node.bindings.clone(),
                    outer_column_indices,
                    Arc::new(database.clone()),
                    snapshot,
                    Arc::new(routed.clone()),
                    node.projection.clone(),
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::DerivedScan(node) => {
                // Materialize subquery results into memory
                let mut subquery_executor = create_executor_from_plan(
                    *node.subquery,
                    database,
                    tx_id,
                    snapshot.clone(),
                    routed,
                )
                .await?;
                let mut rows = Vec::new();
                loop {
                    match subquery_executor.next().await? {
                        Some(ExecResult::Row(row)) => rows.push(row),
                        Some(_) => {} // skip non-row results
                        None => break,
                    }
                }
                Ok(
                    Box::new(DerivedScanExecutor::new(rows, node.projection.clone()))
                        as Box<dyn Executor + Send>,
                )
            }

            PhysicalPlan::Projection(node) => {
                // MS11-T01 Iter001: 逐项求值走 owned `Expression::evaluate`
                // 路径（新值表达式的 evaluate_ref 对 String 结果报错，禁用）
                let input = create_executor_from_plan(
                    *node.input,
                    database,
                    tx_id,
                    snapshot.clone(),
                    routed,
                )
                .await?;
                let items = node.items.into_iter().map(|item| item.expr).collect();
                Ok(Box::new(ProjectionExecutor::new(input, items)) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Distinct(node) => {
                // MS25 T2: materialize the (already output-shaped) input row
                // stream, then hand the rows to the de-duplicating executor
                // (same construction-time materialization shape as DerivedScan).
                let input = create_executor_from_plan(
                    *node.input,
                    database,
                    tx_id,
                    snapshot.clone(),
                    routed,
                )
                .await?;
                Ok(Box::new(DistinctExecutor::new(input)) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::SetOperation(node) => {
                // MS25 T3: both branches are full SELECT plans — build each
                // executor and let the set-operation executor materialize them.
                let left = create_executor_from_plan(
                    *node.left,
                    database,
                    tx_id,
                    snapshot.clone(),
                    routed,
                )
                .await?;
                let right = create_executor_from_plan(
                    *node.right,
                    database,
                    tx_id,
                    snapshot.clone(),
                    routed,
                )
                .await?;
                Ok(
                    Box::new(SetOperationExecutor::new(left, right, node.op, node.all))
                        as Box<dyn Executor + Send>,
                )
            }

            PhysicalPlan::SingleRow => {
                // MS13 T9: no-FORM 虚拟单行输入（恰产出一行空行）
                Ok(Box::new(SingleRowExecutor::new()) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Explain(node) => {
                // MS25 T6: EXPLAIN 是只读观测面——渲染被观测计划，**不构造
                // 内层执行器**（派生表急切物化与任何执行器副作用都不触发）。
                Ok(Box::new(ExplainExecutor::new(&node.input)) as Box<dyn Executor + Send>)
            }
        }
    })
}

pub(crate) fn value_to_json(value: Value) -> serde_json::Value {
    match value {
        Value::Int(n) => serde_json::Value::Number(n.into()),
        Value::String(s) => serde_json::Value::String(s),
        Value::Null => serde_json::Value::Null,
        Value::Float(f) => {
            // serde_json doesn't support special float values (NaN, Inf)
            // so we need to handle them carefully
            if f.is_finite() {
                serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null)
            } else {
                // NaN, Infinity -> Null (not representable in JSON)
                serde_json::Value::Null
            }
        }
        Value::Bool(b) => serde_json::Value::Bool(b),
        // MS13: 日期族按 DA5 字符串形态渲染
        Value::Date(d) => serde_json::Value::String(crate::executor::datetime::format_date(d)),
        Value::Timestamp(t) => {
            serde_json::Value::String(crate::executor::datetime::format_timestamp(t))
        }
    }
}

/// T4（change 2026-10-04-external-review-gaps）：计划的输出列名序（仅覆盖
/// SubqueryEval 消费面可达的形状——扫描类/聚合/排序/链式标量节点；其余形状
/// 返回 `None` 交回调用方既有 name→pos 映射）。投影裁剪语义与
/// `get_plan_output_columns` 同款：空投影 = 恒等，非空 = 按索引裁剪。
fn plan_output_names(plan: &PhysicalPlan) -> Option<Vec<String>> {
    match plan {
        PhysicalPlan::Scan(n) => Some(if n.projection.is_empty() {
            n.columns.clone()
        } else {
            n.projection.iter().map(|&i| n.columns[i].clone()).collect()
        }),
        PhysicalPlan::DataScan(n) => Some(if n.projection.is_empty() {
            n.columns.clone()
        } else {
            n.projection.iter().map(|&i| n.columns[i].clone()).collect()
        }),
        PhysicalPlan::DerivedScan(n) => Some(if n.projection.is_empty() {
            n.columns.clone()
        } else {
            n.projection.iter().map(|&i| n.columns[i].clone()).collect()
        }),
        PhysicalPlan::IndexScan(n) => Some(n.columns.clone()),
        PhysicalPlan::IndexScanAll(n) => Some(if n.projection.is_empty() {
            n.columns.clone()
        } else {
            n.projection.iter().map(|&i| n.columns[i].clone()).collect()
        }),
        PhysicalPlan::ScalarSubqueryFilter(n) => Some(n.columns.clone()),
        PhysicalPlan::Aggregate(n) => Some(n.output_columns.clone()),
        PhysicalPlan::Sort(n) => Some(if n.projection.is_empty() {
            n.columns.clone()
        } else {
            n.projection.iter().map(|&i| n.columns[i].clone()).collect()
        }),
        PhysicalPlan::Limit(n) => plan_output_names(&n.input),
        PhysicalPlan::SubqueryEval(n) => {
            let mut names = plan_output_names(&n.input)?;
            if n.output_width > 0 {
                names.truncate(n.output_width);
            }
            let idx = n.result_column_index.min(names.len());
            names.insert(idx, n.output_column.clone());
            Some(names)
        }
        _ => None,
    }
}

/// T11（change 2026-10-04-external-review-gaps）：为 Join 条件键查表注册
/// `(真实表, 列)` 限定键——限定键在该输入内唯一命中时才注册（自连接同表
/// 两侧限定键歧义时不注册，执行器回落裸名查表既有语义）。基位映射沿用
/// `extract_column_indices` 的裸名语义，本函数只做加性补充（`or_insert`，
/// 不覆盖既有条目）。
fn register_join_qualified_keys(map: &mut HashMap<String, usize>, plan: &PhysicalPlan) {
    let mut pairs: Vec<(String, String, usize)> = Vec::new();
    collect_join_key_pairs(plan, &mut pairs);
    let mut candidates: HashMap<String, usize> = HashMap::new();
    let mut ambiguous: HashSet<String> = HashSet::new();
    for (table, column, pos) in pairs {
        let key = format!("{}.{}", table.to_lowercase(), column.to_lowercase());
        if candidates.remove(&key).is_some() {
            ambiguous.insert(key);
        } else {
            candidates.insert(key, pos);
        }
    }
    for (key, pos) in candidates {
        if ambiguous.contains(&key) {
            continue;
        }
        map.entry(key).or_insert(pos);
    }
}

/// 收集条件键限定对 `(真实表, 列, 输出行绝对位)`——覆盖链输入可达的形状
///（Scan / DerivedScan / IndexScan / Join / NLJ / Semi / Anti 的输出列）；
/// 其余形状不产生限定键（执行器裸名回退）。
fn collect_join_key_pairs(plan: &PhysicalPlan, out: &mut Vec<(String, String, usize)>) {
    match plan {
        PhysicalPlan::Scan(n) => {
            for (i, col) in n.columns.iter().enumerate() {
                out.push((n.table_name.clone(), col.clone(), i));
            }
        }
        PhysicalPlan::DerivedScan(n) => {
            for (i, col) in n.columns.iter().enumerate() {
                out.push((n.alias.clone(), col.clone(), i));
            }
        }
        PhysicalPlan::IndexScan(n) => {
            for (i, col) in n.columns.iter().enumerate() {
                out.push((n.table_name.clone(), col.clone(), i));
            }
        }
        PhysicalPlan::Join(n) => collect_output_column_pairs(&n.output_columns, out),
        PhysicalPlan::NestedLoopJoin(n) => collect_output_column_pairs(&n.output_columns, out),
        PhysicalPlan::SemiJoin(n) => collect_output_column_pairs(&n.output_columns, out),
        PhysicalPlan::AntiJoin(n) => collect_output_column_pairs(&n.output_columns, out),
        _ => {}
    }
}

fn collect_output_column_pairs(
    columns: &[crate::executor::OutputColumn],
    out: &mut Vec<(String, String, usize)>,
) {
    for (i, col) in columns.iter().enumerate() {
        if let Some(table) = &col.table {
            out.push((table.clone(), col.column.clone(), i));
        }
    }
}

fn extract_column_indices(plan: &PhysicalPlan) -> Result<(HashMap<String, usize>, String)> {
    match plan {
        PhysicalPlan::Scan(scan_node) => {
            let indices: HashMap<String, usize> = scan_node
                .columns
                .iter()
                .enumerate()
                .map(|(idx, col)| (col.to_lowercase(), idx))
                .collect();
            Ok((indices, scan_node.table_name.clone()))
        }
        PhysicalPlan::DataScan(scan_node) => {
            // M19: same column index layout as Scan.
            let indices: HashMap<String, usize> = scan_node
                .columns
                .iter()
                .enumerate()
                .map(|(idx, col)| (col.to_lowercase(), idx))
                .collect();
            Ok((indices, scan_node.table_name.clone()))
        }
        PhysicalPlan::Join(join_node) => {
            // For nested joins, use output_columns to build indices
            let indices: HashMap<String, usize> = join_node
                .output_columns
                .iter()
                .enumerate()
                .map(|(idx, col)| (col.column.to_lowercase(), idx))
                .collect();
            // Use first condition's left table as "table name"
            let table_name = join_node
                .conditions
                .first()
                .map(|c| c.left_column.table.clone().unwrap_or_default())
                .unwrap_or_default();
            Ok((indices, table_name))
        }
        PhysicalPlan::NestedLoopJoin(node) => {
            // MS09-T02: same output_columns mapping as the Join arm; NLJ has
            // no equi conditions, so the chain-head table name comes from the
            // first output column's table_alias.
            let indices: HashMap<String, usize> = node
                .output_columns
                .iter()
                .enumerate()
                .map(|(idx, col)| (col.column.to_lowercase(), idx))
                .collect();
            let table_name = node
                .output_columns
                .first()
                .map(|c| c.table_alias.clone())
                .unwrap_or_default();
            Ok((indices, table_name))
        }
        PhysicalPlan::SemiJoin(semi_join_node) => {
            // SemiJoin outputs only left table columns
            let indices: HashMap<String, usize> = semi_join_node
                .output_columns
                .iter()
                .enumerate()
                .map(|(idx, col)| (col.column.to_lowercase(), idx))
                .collect();
            // Use first condition's left table as "table name"
            let table_name = semi_join_node
                .conditions
                .first()
                .map(|c| c.left_column.table.clone().unwrap_or_default())
                .unwrap_or_default();
            Ok((indices, table_name))
        }
        PhysicalPlan::AntiJoin(anti_join_node) => {
            // AntiJoin outputs only left table columns
            let indices: HashMap<String, usize> = anti_join_node
                .output_columns
                .iter()
                .enumerate()
                .map(|(idx, col)| (col.column.to_lowercase(), idx))
                .collect();
            // Use first condition's left table as "table name"
            let table_name = anti_join_node
                .conditions
                .first()
                .map(|c| c.left_column.table.clone().unwrap_or_default())
                .unwrap_or_default();
            Ok((indices, table_name))
        }
        PhysicalPlan::Filter(filter_node) => {
            // For Filter, extract from input
            extract_column_indices(&filter_node.input)
        }
        PhysicalPlan::Aggregate(agg_node) => {
            // For Aggregate, use output_columns
            let indices: HashMap<String, usize> = agg_node
                .output_columns
                .iter()
                .enumerate()
                .map(|(idx, col)| (col.to_lowercase(), idx))
                .collect();
            Ok((indices, agg_node.table_name.clone()))
        }
        PhysicalPlan::SubqueryEval(node) => {
            // T4（change 2026-10-04-external-review-gaps）：扫描类/链式形状的
            // 输出列名序可得——按 output_width 截断 + 插列后的最终输出重建
            // 列位映射；其余形状回落既有 name→pos 映射 + 插列（恒等行）。
            if let Some(mut names) = plan_output_names(&node.input) {
                if node.output_width > 0 {
                    names.truncate(node.output_width);
                }
                let idx = node.result_column_index.min(names.len());
                names.insert(idx, node.output_column.clone());
                let indices: HashMap<String, usize> = names
                    .into_iter()
                    .enumerate()
                    .map(|(idx, col)| (col.to_lowercase(), idx))
                    .collect();
                let table_name = extract_column_indices(&node.input)?.1;
                return Ok((indices, table_name));
            }
            let (mut indices, table_name) = extract_column_indices(&node.input)?;
            let next_idx = indices.len();
            indices.insert(node.output_column.to_lowercase(), next_idx);
            Ok((indices, table_name))
        }
        PhysicalPlan::ScalarSubqueryFilter(node) => {
            // T4: `columns` 即裁剪后实际输出形状（IndexScan「已收窄」契约
            // 同型——表头与执行器行的唯一真源）。
            let indices: HashMap<String, usize> = node
                .columns
                .iter()
                .enumerate()
                .map(|(idx, col)| (col.to_lowercase(), idx))
                .collect();
            Ok((indices, node.table_name.clone()))
        }
        PhysicalPlan::Sort(sort_node) => {
            // T4: ORDER BY + 标量子查询项形态下 SubqueryEval 落在 Sort 之上
            // ——`columns` 是其输入行的真实列序，`projection` 非空时行已
            // 按其收窄（列名随投影裁剪，与 get_plan_output_columns 的 Sort
            // 臂同款）。
            let names: Vec<String> = if sort_node.projection.is_empty() {
                sort_node.columns.clone()
            } else {
                sort_node
                    .projection
                    .iter()
                    .map(|&i| sort_node.columns[i].clone())
                    .collect()
            };
            let indices: HashMap<String, usize> = names
                .into_iter()
                .enumerate()
                .map(|(idx, col)| (col.to_lowercase(), idx))
                .collect();
            Ok((indices, sort_node.table_name.clone()))
        }
        PhysicalPlan::Limit(limit_node) => {
            // T4: Limit 不改列——透传输入映射（SubqueryEval 可经 LIMIT 之上
            // 包裹到达）。
            extract_column_indices(&limit_node.input)
        }
        PhysicalPlan::DerivedScan(node) => {
            let indices: HashMap<String, usize> = node
                .columns
                .iter()
                .enumerate()
                .map(|(idx, col)| (col.to_lowercase(), idx))
                .collect();
            Ok((indices, String::new()))
        }
        PhysicalPlan::IndexScan(node) => {
            let indices: HashMap<String, usize> = node
                .columns
                .iter()
                .enumerate()
                .map(|(idx, col)| (col.to_lowercase(), idx))
                .collect();
            Ok((indices, node.table_name.clone()))
        }
        PhysicalPlan::IndexScanAll(node) => {
            // T1（change 2026-10-04-external-review-gaps D1）：`columns` 是
            // 基表全列，输出形状由 `projection` 决定——先裁剪再建立输出位
            // 映射（对齐 IndexScan 臂「已收窄」口径；执行器行即该形状）。
            let projected: Vec<String> = if node.projection.is_empty() {
                node.columns.clone()
            } else {
                node.projection
                    .iter()
                    .map(|&i| node.columns[i].clone())
                    .collect()
            };
            let indices: HashMap<String, usize> = projected
                .iter()
                .enumerate()
                .map(|(idx, col)| (col.to_lowercase(), idx))
                .collect();
            Ok((indices, node.table_name.clone()))
        }
        // MS13 T9: no-FORM 输入节点不可达此处（无列消费方）；空臂防御。
        PhysicalPlan::SingleRow => Ok((HashMap::new(), String::new())),
        _ => Err(crate::storage::StorageError::ExecutionError(
            "Expected Scan, Join, SemiJoin, AntiJoin, Filter, Aggregate, SubqueryEval, ScalarSubqueryFilter, Sort, Limit, DerivedScan, IndexScan, or IndexScanAll".into(),
        )),
    }
}

/// Extract all table names from a statement (including JOIN tables)
fn extract_all_table_names(stmt: &Statement) -> Vec<TableRef> {
    match stmt {
        Statement::Query(query) => extract_all_query_table_names(query),
        // MS25 T6: `EXPLAIN <SELECT>` 的被观测语句贡献其表名（`EXPLAIN` 走
        // plan_stage 的非 DDL 分支，目录注册为其内层语句服务）。
        Statement::Explain { statement, .. } => extract_all_table_names(statement),
        // MS20-T02 Iter001 (design D4)：INSERT 保留写目标，SELECT 源贡献其
        // 完整表面（CTE 感知提取）——源表经 `register_table` 注册 schema/
        // 路由/视图豁免。VALUES 源提取结果为空，行为与此前逐字节一致。
        Statement::Insert {
            table_name, source, ..
        } => {
            let mut tables = vec![TableRef::new(table_name)];
            if let Some(q) = source {
                tables.extend(extract_all_query_table_names(q));
            }
            tables
        }
        Statement::Update { table, .. } => extract_all_from_table_with_joins_item(table),
        // MS20 T4: CTAS 的物化源是完整 SELECT（含 CTEs / JOIN / 子查询），
        // 目录注册为该 SELECT 服务；写目标名不参与提取（目标由 DDL 标记与
        // 写库面检查负责，MS20 T3 既有语义）。
        Statement::CreateTable { query: Some(q), .. } => extract_all_query_table_names(q),
        Statement::Delete { from, .. } => {
            let tables = match from {
                sqlparser::ast::FromTable::WithFromKeyword(t) => t.clone(),
                sqlparser::ast::FromTable::WithoutKeyword(t) => t.clone(),
            };
            tables
                .iter()
                .flat_map(extract_all_from_table_with_joins_item)
                .collect()
        }
        // MS21 Iter002 (T2-c)：CREATE INDEX 的目标表经 register_table 注册
        // （builder 需要其 schema 通道做列存在性 / INT 声明检查）。
        Statement::CreateIndex { table_name, .. } => vec![TableRef::new(table_name)],
        // MS21 Iter003 (T3-b)：ALTER TABLE 的目标表经 register_table 注册
        // （缺失表 42P01；builder 需要其 schema 通道做列校验）。
        Statement::AlterTable { name, .. } => vec![TableRef::new(name)],
        // MS21 Iter004 (T4-a)：TRUNCATE 的目标表经 register_table 注册
        // （缺失表/系统保留名 42P01、附加库引用标记只读拒绝）。
        Statement::Truncate { table_name, .. } => vec![TableRef::new(table_name)],
        // MS21 Iter005 (T5-b/D12)：CREATE VIEW 的定义查询贡献其表名（CTAS
        // 物化源语义同型——CTE 名已按可见性过滤）。引用合法性由视图感知
        // 注册消费（附加库引用拒绝、既有视图引用合法、基表注入 schema）。
        Statement::CreateView { query, .. } => extract_all_query_table_names(query),
        _ => Vec::new(),
    }
}

/// Extract all table names from a query (including JOIN tables, derived subqueries, and WHERE/SELECT subqueries)
///
/// MS25 T5: `WITH`-defined names are statement-scoped and shadow same-named base
/// tables, so they never reach the catalog (the planner's CTE registry resolves
/// them). Definition bodies contribute their own tables, each filtered by the
/// names visible at that point — a name defined *later* still resolves to the
/// base table inside an earlier definition.
fn extract_all_query_table_names(query: &Query) -> Vec<TableRef> {
    let Some(with) = &query.with else {
        return extract_query_body_table_names(query);
    };
    let mut tables = Vec::new();
    let mut visible: Vec<String> = Vec::new();
    for cte in &with.cte_tables {
        let mut definition_tables = extract_all_query_table_names(&cte.query);
        definition_tables.retain(|t| !visible.contains(&t.flat));
        tables.extend(definition_tables);
        visible.push(cte.alias.name.value.to_lowercase());
    }
    let mut body_tables = extract_query_body_table_names(query);
    body_tables.retain(|t| !visible.contains(&t.flat));
    tables.extend(body_tables);
    tables
}

/// Table names of a query body (dispatching over its set expression).
fn extract_query_body_table_names(query: &Query) -> Vec<TableRef> {
    match query.body.as_ref() {
        SetExpr::Select(select) => extract_select_table_names(select),
        // MS25 T3: compound bodies contribute both branches' tables (each
        // branch is a full SELECT, so the Select arm handles it as usual).
        SetExpr::SetOperation { left, right, .. } => {
            let mut tables = extract_all_table_names_from_set_expr(left);
            tables.extend(extract_all_table_names_from_set_expr(right));
            tables
        }
        SetExpr::Query(inner) => extract_all_query_table_names(inner),
        _ => Vec::new(),
    }
}

/// MS25 T3: 分支表名提取——复合分支本身可能是嵌套复合。
fn extract_all_table_names_from_set_expr(expr: &SetExpr) -> Vec<TableRef> {
    match expr {
        SetExpr::Select(select) => extract_select_table_names(select),
        SetExpr::SetOperation { left, right, .. } => {
            let mut tables = extract_all_table_names_from_set_expr(left);
            tables.extend(extract_all_table_names_from_set_expr(right));
            tables
        }
        SetExpr::Query(inner) => extract_all_query_table_names(inner),
        _ => Vec::new(),
    }
}

/// Table names of a single SELECT: FROM/JOIN plus WHERE and projection
/// subqueries.
fn extract_select_table_names(select: &sqlparser::ast::Select) -> Vec<TableRef> {
    let from_tables: Vec<TableRef> = select
        .from
        .iter()
        .flat_map(extract_all_from_table_with_joins_item)
        .collect();

    // Extract tables from WHERE clause subqueries
    let where_tables = select
        .selection
        .as_ref()
        .map(extract_subquery_tables_from_expr)
        .unwrap_or_default();

    // Extract tables from SELECT projection subqueries
    let projection_tables: Vec<TableRef> = select
        .projection
        .iter()
        .flat_map(|item| match item {
            sqlparser::ast::SelectItem::UnnamedExpr(expr)
            | sqlparser::ast::SelectItem::ExprWithAlias { expr, .. } => {
                extract_subquery_tables_from_expr(expr)
            }
            _ => Vec::new(),
        })
        .collect();

    // Combine FROM, WHERE, and projection tables
    from_tables
        .into_iter()
        .chain(where_tables)
        .chain(projection_tables)
        .collect()
}

/// Extract table names from subqueries inside an expression
fn extract_subquery_tables_from_expr(expr: &Expr) -> Vec<TableRef> {
    match expr {
        Expr::InSubquery { subquery, .. } | Expr::Exists { subquery, .. } => {
            extract_all_query_table_names(subquery)
        }
        Expr::Subquery(subquery) => extract_all_query_table_names(subquery),
        // Recurse into nested expressions
        Expr::BinaryOp { left, right, .. } => extract_subquery_tables_from_expr(left)
            .into_iter()
            .chain(extract_subquery_tables_from_expr(right))
            .collect(),
        Expr::UnaryOp { expr, .. } => extract_subquery_tables_from_expr(expr),
        Expr::Nested(expr) => extract_subquery_tables_from_expr(expr),
        Expr::Between {
            expr, low, high, ..
        } => extract_subquery_tables_from_expr(expr)
            .into_iter()
            .chain(extract_subquery_tables_from_expr(low))
            .chain(extract_subquery_tables_from_expr(high))
            .collect(),
        Expr::InList { expr, .. } => extract_subquery_tables_from_expr(expr),
        Expr::Case { .. } => Vec::new(), // Case expressions don't typically contain subqueries
        _ => Vec::new(),
    }
}

/// Extract all table names from a TableWithJoins (including JOIN tables and derived subqueries)
fn extract_all_from_table_with_joins_item(twj: &TableWithJoins) -> Vec<TableRef> {
    let mut tables = Vec::new();

    // Main table
    match &twj.relation {
        TableFactor::Table { name, .. } => {
            tables.push(TableRef::new(name));
        }
        TableFactor::Derived { subquery, .. } => {
            // Recursively extract table names from the derived subquery
            tables.extend(extract_all_query_table_names(subquery));
        }
        _ => {}
    }

    // JOIN tables
    for join in &twj.joins {
        match &join.relation {
            TableFactor::Table { name, .. } => {
                tables.push(TableRef::new(name));
            }
            TableFactor::Derived { subquery, .. } => {
                tables.extend(extract_all_query_table_names(subquery));
            }
            _ => {}
        }
    }

    tables
}

/// MS20 T2：把一条表 schema 灌进 `PlanBuilder`——列/键类型/DEFAULT/唯一列
/// 四条加性通道，`register_table` 主库路径与附加库路由分支共用（MS16 D2 /
/// MS24 D1+D4 的既有行为逐字节不变）。
fn register_table_schema(
    builder: &mut PlanBuilder,
    key: &str,
    table_meta: &crate::storage::data::TableMeta,
) {
    let columns: Vec<String> = table_meta
        .columns
        .iter()
        .map(|(name, _)| name.clone())
        .collect();
    builder.register_table(key, columns, &table_meta.pk_column);
    // MS16 Iteration 000 (I046, design D2): 传递键列声明类型，键位
    // 等值路由按类型分流。键列不在列清单（pk_column 为空）不注册
    // ——类型未知 = 既有路由。
    if let Some((_, pk_type)) = table_meta
        .columns
        .iter()
        .find(|(name, _)| name == &table_meta.pk_column)
    {
        builder.set_pk_column_type(key, pk_type.clone());
    }
    // MS24 Iteration 000 (D1): 传递 per-column 声明 DEFAULT，
    // 子集 INSERT / DEFAULT 关键字填充在计划期消费。
    builder.set_table_defaults(key, table_meta.defaults.clone());
    // MS24 Iteration 001 (D4): 传递承载唯一索引的列位（升序），
    // ON CONFLICT 冲突目标只仲裁 INT 声明 PK 或这些列。
    let unique_columns: Vec<usize> = table_meta
        .unique_indexes
        .iter()
        .map(|(col_pos, _)| *col_pos)
        .collect();
    builder.set_table_unique_columns(key, unique_columns);
    // MS21 Iter002 (T2-c/D14)：传递逐列声明类型，CREATE INDEX 的 INT-only
    // 门在计划期消费。
    builder.set_table_column_types(
        key,
        table_meta
            .columns
            .iter()
            .map(|(_, ct)| ct.clone())
            .collect(),
    );
    // MS21 Iter002 (T2-g/D6)：传递登记的二级索引（索引名, 列名），非 PK
    // 等值路由在计划期消费。
    builder.set_table_secondary_indexes(
        key,
        table_meta
            .secondary_indexes
            .iter()
            .map(|s| (s.name.clone(), table_meta.columns[s.column_index].0.clone()))
            .collect(),
    );
}

/// MS20 T3：DDL 写目标（CREATE TABLE / DROP TABLE）的附加库引用标记。这两个
/// 语句走 `plan_stage` 的 DDL 分支，不经 `register_table`，判定只依赖「首段
/// 命中注册别名」——不查 catalog，因此 `CREATE TABLE o.newt`（表尚不存在）
/// 同样被点名拒绝。
fn mark_attached_ddl_targets(builder: &mut PlanBuilder, stmt: &Statement, route: &AttachRoute) {
    let names: Vec<&sqlparser::ast::ObjectName> = match stmt {
        Statement::CreateTable { name, .. } => vec![name],
        Statement::Drop { names, .. } => names.iter().collect(),
        _ => Vec::new(),
    };
    for name in names {
        let parts = crate::parser::ast::object_name_parts(name);
        if parts.len() >= 2 && route.is_registered(&parts[0]) {
            builder.mark_attached_table(&parts.join("."));
        }
    }
}

/// MS21 Iter005 (T5-b/D12)：`Database.views` → builder 视图通道快照
/// （`ViewMeta` → planner 层 `ViewDef` 的纯映射）。
async fn current_view_defs(database: &Database) -> std::collections::HashMap<String, ViewDef> {
    let views = database.views.read().await;
    views
        .iter()
        .map(|(name, meta)| {
            (
                name.clone(),
                ViewDef {
                    sql: meta.sql.clone(),
                    columns: meta.columns.clone(),
                },
            )
        })
        .collect()
}

/// MS21 Iter005 (T5-b/D12)：CREATE VIEW 定义表名的视图感知注册——
/// 附加库引用计划期点名拒绝（可持久对象不得携带会话限定名，spec R1）；
/// 既有视图引用合法（递归注册其定义的基表面）；基表引用注入
/// register_table 通道（缺失 42P01 自然上抛，为定义验证的列解析服务）。
async fn register_view_definition_tables(
    database: &Database,
    builder: &mut PlanBuilder,
    stmt: &Statement,
    route: Option<&AttachRoute>,
) -> std::result::Result<(), PlanFailure> {
    for table_ref in extract_all_table_names(stmt) {
        if let Some(route) = route {
            if table_ref.parts.len() >= 2 && route.is_registered(&table_ref.parts[0]) {
                return Err(PlanFailure {
                    message: format!(
                        "CREATE VIEW definition references attached database table '{}' (persistable objects cannot reference session-scoped attached databases)",
                        table_ref.flat
                    ),
                    sqlstate: PlanError::UnsupportedStatement(String::new()).sqlstate(),
                });
            }
        }
        if builder.views.contains_key(&table_ref.flat) {
            register_view_base_tables_recursive(database, builder, &table_ref.flat).await?;
            continue;
        }
        match database.table_manager.get_table(&table_ref.flat).await {
            Ok(table_meta) => register_table_schema(builder, &table_meta.name, &table_meta),
            Err(e) => {
                let message = format!("Table '{}' not found: {}", table_ref.flat, e);
                let sqlstate = PlanError::TableNotFound(table_ref.flat).sqlstate();
                return Err(PlanFailure { message, sqlstate });
            }
        }
    }
    Ok(())
}

/// MS21 Iter005 (T5-c/D12)：递归注册一个视图定义所引用的基表面（嵌套视图
/// 深入其定义）。展开钩子在 build_query 期消费这些 schema 通道做列解析；
/// 循环引用结构性不可能（创建期「引用尚不存在的视图即拒」→ 引用图按
/// 创建序成 DAG），无需环检测。
async fn register_view_base_tables_recursive(
    database: &Database,
    builder: &mut PlanBuilder,
    view_name: &str,
) -> std::result::Result<(), PlanFailure> {
    let Some(def) = builder.views.get(view_name) else {
        return Ok(());
    };
    let sql = def.sql.clone();
    let statements = parse_sql(&sql).map_err(PlanFailure::from)?;
    let refs = match statements.first() {
        Some(Statement::Query(q)) => extract_all_query_table_names(q),
        _ => Vec::new(),
    };
    for table_ref in refs {
        if builder.views.contains_key(&table_ref.flat) {
            Box::pin(register_view_base_tables_recursive(
                database,
                builder,
                &table_ref.flat,
            ))
            .await?;
            continue;
        }
        match database.table_manager.get_table(&table_ref.flat).await {
            Ok(table_meta) => register_table_schema(builder, &table_meta.name, &table_meta),
            Err(e) => {
                let message = format!("Table '{}' not found: {}", table_ref.flat, e);
                let sqlstate = PlanError::TableNotFound(table_ref.flat).sqlstate();
                return Err(PlanFailure { message, sqlstate });
            }
        }
    }
    Ok(())
}

async fn register_table(
    database: &Database,
    builder: &mut PlanBuilder,
    stmt: &Statement,
    route: Option<&AttachRoute>,
    routed: &mut RoutedTables,
) -> std::result::Result<(), PlanFailure> {
    let table_names = extract_all_table_names(stmt);
    if table_names.is_empty() {
        return Ok(());
    }

    for table_ref in table_names {
        // MS20 T2：路由判定只在 `ObjectName` 分段尚存处进行。三段及以上且首段
        // 命中别名 → 点名拒绝（无 schema 层级概念）。
        let attached = match (route, table_ref.parts.len()) {
            (Some(route), len) if len >= 2 && route.is_registered(&table_ref.parts[0]) => {
                if len >= 3 {
                    return Err(PlanFailure::from(PlanError::UnsupportedStatement(
                        "QUALIFIED NAME DEEPER THAN alias.table (no schema hierarchy)".into(),
                    )));
                }
                route.db_for_alias(&table_ref.parts[0])
            }
            _ => None,
        };

        if let Some(attached_db) = attached {
            // 附加库表不存在 → 按「该限定名在附加库中不存在」点名，SQLSTATE 与
            // 主库 `TableNotFound` 同类（42P01）；不静默回落主库同名表。
            let table_meta = attached_db
                .table_manager
                .get_table(&table_ref.parts[1])
                .await
                .map_err(|_| {
                    let message = format!(
                        "Table '{}' not found in attached database '{}'",
                        table_ref.flat, table_ref.parts[0]
                    );
                    PlanFailure {
                        message,
                        sqlstate: PlanError::TableNotFound(table_ref.flat.clone()).sqlstate(),
                    }
                })?;
            register_table_schema(builder, &table_ref.flat, &table_meta);
            // MS20 T3：写消费点据此点名拒绝（附加库只读）。
            builder.mark_attached_table(&table_ref.flat);
            routed.insert(
                table_ref.flat.clone(),
                RoutedTable {
                    db: attached_db.clone(),
                    local_name: table_ref.parts[1].clone(),
                },
            );
            continue;
        }

        match database.table_manager.get_table(&table_ref.flat).await {
            Ok(table_meta) => {
                register_table_schema(builder, &table_meta.name, &table_meta);
            }
            // MS21 Iter005 (T5-c/D12)：视图名豁免——statement 级引用合法化
            // 并递归注册其定义的基表面（42P01 判定推迟到 FROM/JOIN 展开钩子
            // 与定义基表注册；视图是主库对象，两段限定名路由先于此分支处理
            // ——豁免只对 flat 名）。
            Err(_) if builder.views.contains_key(&table_ref.flat) => {
                register_view_base_tables_recursive(database, builder, &table_ref.flat).await?;
            }
            // 表缺失文案保持不变，SQLSTATE 取 `PlanError` 映射表的同名分类。
            Err(e) => {
                let message = format!("Table '{}' not found: {}", table_ref.flat, e);
                let sqlstate = PlanError::TableNotFound(table_ref.flat).sqlstate();
                return Err(PlanFailure { message, sqlstate });
            }
        }
    }

    Ok(())
}

/// Check if a statement is cacheable
/// Only SELECT queries are cacheable, DDL and DML statements are not
fn is_cacheable(stmt: &Statement) -> bool {
    matches!(stmt, Statement::Query(_))
}

#[cfg(test)]
mod tests {
    //! Per-stage unit tests for MS06-T04: verify each stage can be invoked
    //! independently of the full pipeline orchestrator and produces the
    //! expected error / output / cache side-effect.

    use super::*;
    use crate::executor::PhysicalPlan;
    use tempfile::tempdir;

    async fn open_db_with_table(create_sql: &str) -> (Database, tempfile::TempDir) {
        let dir = tempdir().unwrap();
        let db = Database::open(&dir.path().join("test.db")).await.unwrap();
        let resp = db.execute_sql(create_sql).await;
        assert!(
            !matches!(resp, Response::Error { .. }),
            "setup CREATE TABLE failed: {:?}",
            resp
        );
        (db, dir)
    }

    // ---- parse_stage ----

    #[tokio::test]
    async fn parse_stage_valid_sql_yields_statements() {
        let stmts = parse_stage("SELECT 1").await.expect("parse should succeed");
        assert_eq!(stmts.len(), 1);
    }

    #[tokio::test]
    async fn parse_stage_invalid_sql_returns_parse_error() {
        // Unmatched parenthesis is unambiguously rejected by sqlparser.
        let err = parse_stage("SELECT (1 FROM t")
            .await
            .expect_err("parse should fail");
        assert!(
            err.starts_with("Parse error:"),
            "expected 'Parse error:' prefix, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn parse_stage_empty_sql_returns_empty_error() {
        let err = parse_stage("").await.expect_err("empty should fail");
        assert_eq!(err, "Empty SQL");

        let err_ws = parse_stage("   \n\t  ")
            .await
            .expect_err("whitespace should fail");
        assert_eq!(err_ws, "Empty SQL");
    }

    // ---- plan_stage ----

    #[tokio::test]
    async fn plan_stage_select_on_known_table_yields_scan_plan() {
        let (db, _dir) =
            open_db_with_table("CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;
        let stmts = parse_stage("SELECT * FROM t").await.unwrap();
        let stmt = stmts.first().unwrap();
        let plan = plan_stage(&db, "SELECT * FROM t", stmt, false)
            .await
            .expect("plan should succeed for known table");
        // SELECT on a simple table is built as either Scan or DataScan depending
        // on planner routing — both are valid "scan-class" plans.
        assert!(
            matches!(plan, PhysicalPlan::Scan(_) | PhysicalPlan::DataScan(_)),
            "expected Scan / DataScan plan, got: {:?}",
            std::mem::discriminant(&plan)
        );
    }

    #[tokio::test]
    async fn plan_stage_unknown_table_returns_not_found_error() {
        let (db, _dir) = open_db_with_table("CREATE TABLE t (id INT PRIMARY KEY)").await;
        let stmts = parse_stage("SELECT * FROM missing").await.unwrap();
        let stmt = stmts.first().unwrap();
        let err = plan_stage(&db, "SELECT * FROM missing", stmt, false)
            .await
            .expect_err("plan should fail for unknown table");
        assert!(
            err.to_string().contains("not found"),
            "expected 'not found' in error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn plan_stage_select_writes_to_cache_but_dml_does_not() {
        let (db, _dir) =
            open_db_with_table("CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;
        let select_sql = "SELECT * FROM t";
        let stmts = parse_stage(select_sql).await.unwrap();
        let stmt = stmts.first().unwrap();
        let _ = plan_stage(&db, select_sql, stmt, false).await.unwrap();
        assert_eq!(
            db.plan_cache_len(),
            1,
            "SELECT plan should be cached after plan_stage"
        );

        // INSERT must not be cached: invoke plan_stage with a fresh DML.
        let insert_sql = "INSERT INTO t (id, name) VALUES (1, 'a')";
        let stmts = parse_stage(insert_sql).await.unwrap();
        let stmt = stmts.first().unwrap();
        let _ = plan_stage(&db, insert_sql, stmt, false).await.unwrap();
        assert_eq!(
            db.plan_cache_len(),
            1,
            "DML plan must not enlarge the plan cache"
        );
    }

    /// MS20 T2: 含附加库引用的语句不进 plan cache——cache key 只含 SQL 文本，
    /// 不含 attach 身份；同文本语句在「有/无该别名」的会话间会互相污染。
    #[tokio::test]
    async fn plan_stage_with_attached_reference_skips_plan_cache() {
        let (db, _dir) =
            open_db_with_table("CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;
        let attached_dir = tempdir().unwrap();
        let attached = Database::open(&attached_dir.path().join("attached.db"))
            .await
            .unwrap();
        let resp = attached
            .execute_sql("CREATE TABLE u (id INT PRIMARY KEY)")
            .await;
        assert!(
            !matches!(resp, Response::Error { .. }),
            "setup failed: {resp:?}"
        );

        let route = AttachRoute::new(HashMap::from([(
            "o".to_string(),
            std::sync::Arc::new(attached),
        )]));
        let select_sql = "SELECT * FROM o.u";
        let stmts = parse_stage(select_sql).await.unwrap();
        let stmt = stmts.first().unwrap();

        // 命中附加库：路由表非空 -> 跳过 cache 写
        let mut routed = RoutedTables::new();
        let plan = plan_stage_routed(&db, select_sql, stmt, false, Some(&route), &mut routed)
            .await
            .unwrap();
        assert_eq!(
            routed.len(),
            1,
            "the statement must route to the attached db"
        );
        assert_eq!(
            db.plan_cache_len(),
            0,
            "attached-ref plan must not be cached"
        );

        // 同一 SQL 文本在无注册表会话：按既有路径写 cache
        db.plan_cache.clear();
        let mut no_route = RoutedTables::new();
        let _plan = plan_stage_routed(&db, select_sql, stmt, false, None, &mut no_route).await;
        assert_eq!(
            db.plan_cache_len(),
            0,
            "unrouted statement fails at catalog lookup before caching"
        );
        let planned_table = match &plan {
            PhysicalPlan::Scan(node) => node.table_name.clone(),
            PhysicalPlan::DataScan(node) => node.table_name.clone(),
            other => panic!("unexpected plan shape: {other:?}"),
        };
        assert_eq!(
            planned_table, "o.u",
            "the routed table must be planned under its qualified flat name"
        );

        // 对照组：不含附加库引用的 SELECT 照常进 cache
        let plain_sql = "SELECT * FROM t";
        let stmts = parse_stage(plain_sql).await.unwrap();
        let stmt = stmts.first().unwrap();
        let mut plain = RoutedTables::new();
        plan_stage_routed(&db, plain_sql, stmt, false, Some(&route), &mut plain)
            .await
            .unwrap();
        assert!(plain.is_empty(), "main-db table must not route");
        assert_eq!(
            db.plan_cache_len(),
            1,
            "plain SELECT must still be cached in an attach session"
        );
    }

    // ---- execute_stage ----

    #[tokio::test]
    async fn execute_stage_simple_query_plan_returns_rows() {
        let (db, _dir) =
            open_db_with_table("CREATE TABLE t (id INT PRIMARY KEY, name VARCHAR)").await;
        db.execute_sql("INSERT INTO t (id, name) VALUES (1, 'a')")
            .await;
        let stmts = parse_stage("SELECT * FROM t").await.unwrap();
        let stmt = stmts.first().unwrap();
        let plan = plan_stage(&db, "SELECT * FROM t", stmt, false)
            .await
            .unwrap();

        // Drain cache to ensure execute_stage alone, not the orchestrator, is tested.
        db.plan_cache.clear();
        assert_eq!(db.plan_cache_len(), 0);

        let resp = execute_stage(&db, plan, false).await;
        match resp {
            Response::QueryResult { rows } => assert_eq!(rows.len(), 1),
            other => panic!("Expected QueryResult, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn execute_stage_ddl_plan_clears_cache_on_success() {
        let (db, _dir) = open_db_with_table("CREATE TABLE t (id INT PRIMARY KEY)").await;
        // Prime the cache with a SELECT plan so we can observe clearing.
        db.execute_sql("SELECT * FROM t").await;
        assert!(
            db.plan_cache_len() > 0,
            "Cache should have an entry after SELECT"
        );

        // Build a DDL plan directly and route through execute_stage.
        let stmts = parse_stage("CREATE TABLE t2 (id INT PRIMARY KEY)")
            .await
            .unwrap();
        let stmt = stmts.first().unwrap();
        let plan = plan_stage(&db, "CREATE TABLE t2 (id INT PRIMARY KEY)", stmt, false)
            .await
            .unwrap();
        let resp = execute_stage(&db, plan, false).await;
        assert!(
            !matches!(resp, Response::Error { .. }),
            "DDL execution should succeed: {:?}",
            resp
        );
        assert_eq!(
            db.plan_cache_len(),
            0,
            "DDL execute_stage should clear the plan cache"
        );
    }

    // ---- multi-statement explicit rejection (MS10-T04 S6) ----

    /// S6：网络路径（`execute` → `execute_inner`）遇多语句必须显式报错，
    /// 不得静默截断执行第一条；拒绝发生在 plan_stage / cache put 之前
    /// （cache 长度为 0 证明无键污染、无执行）。
    #[tokio::test]
    async fn execute_sql_multi_statement_returns_error() {
        let (db, _dir) = open_db_with_table("CREATE TABLE t (id INT PRIMARY KEY)").await;
        let resp = db.execute_sql("SELECT 1; SELECT 2").await;
        match resp {
            Response::Error { message, .. } => {
                assert!(
                    message.contains("single statement"),
                    "error must state the single-statement policy: {}",
                    message
                );
            }
            other => panic!("expected Response::Error, got {:?}", other),
        }
        assert_eq!(
            db.plan_cache_len(),
            0,
            "rejection must happen before plan_stage puts a cache entry"
        );
    }

    /// S6：显式事务路径遇多语句必须显式报错，且事务保持存活——
    /// 随后单语句 `execute_in_tx` 正常执行、commit 成功。
    #[tokio::test]
    async fn execute_in_tx_multi_statement_returns_error() {
        let (db, _dir) = open_db_with_table("CREATE TABLE t (id INT PRIMARY KEY)").await;
        let tx = db.begin().await.unwrap();
        let resp = db
            .execute_in_tx("INSERT INTO t VALUES (1); INSERT INTO t VALUES (2)", &tx)
            .await;
        match resp {
            Response::Error { message, .. } => {
                assert!(
                    message.contains("single statement"),
                    "error must state the single-statement policy: {}",
                    message
                );
            }
            other => panic!("expected Response::Error, got {:?}", other),
        }

        // 事务未终结：单语句继续可用并成功提交
        let resp = db.execute_in_tx("INSERT INTO t VALUES (1)", &tx).await;
        assert!(
            !matches!(resp, Response::Error { .. }),
            "tx must remain usable after multi-statement rejection: {:?}",
            resp
        );
        db.commit(tx).await.unwrap();
    }
}
