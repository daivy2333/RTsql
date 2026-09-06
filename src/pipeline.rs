use crate::database::Database;
use crate::executor::{
    AggregateExecutor, AggregateNode, AntiJoinExecutor, CreateTableExecutor, DataScanExecutor,
    DeleteExecutor, DerivedScanExecutor, DropTableExecutor, ExecResult, Executor, FilterExecutor,
    HavingExecutor, IndexScanAllExecutor, IndexScanExecutor, InsertExecutor, JoinConfig,
    JoinExecutor, JoinRelatedConfig, LimitExecutor, PhysicalPlan, ScanExecutor, SemiJoinExecutorV2,
    SortExecutor, SubqueryEvalExecutor, UpdateExecutor, Value,
};
use crate::network::protocol::Response;
use crate::parser::{parse_sql, PlanBuilder};
use crate::profiling::{
    init_profiling, is_profiling_enabled, print_timings, record_time, with_profiling_scope,
};
use crate::storage::Result;
use sqlparser::ast::{Expr, Query, SetExpr, Statement, TableFactor, TableWithJoins};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// create_executor_from_plan 返回类型别名
///
/// 用于解决 type_complexity warning，简化 async 函数返回类型签名
type CreateExecutorFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Box<dyn Executor + Send>>> + Send + 'a>,
>;

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
) -> std::result::Result<PhysicalPlan, String> {
    match stmt {
        Statement::CreateTable { .. } | Statement::Drop { .. } => PlanBuilder::new()
            .build_plan(stmt)
            .map_err(|e| format!("Plan error: {}", e)),
        _ => {
            let table_lookup_start = if profiling {
                Some(Instant::now())
            } else {
                None
            };
            let mut plan_builder = PlanBuilder::new();
            register_table(database, &mut plan_builder, stmt).await?;
            if let Some(start) = table_lookup_start {
                record_time("table_metadata_lookup", start.elapsed());
            }
            let plan = plan_builder
                .build_plan(stmt)
                .map_err(|e| format!("Plan error: {}", e))?;
            if is_cacheable(stmt) {
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
        PhysicalPlan::Insert(_) | PhysicalPlan::Update(_) | PhysicalPlan::Delete(_) => {
            // DML must run inside a real transaction (MS06-T01 spec).
            let tx = database.transaction_manager.begin().await;
            let tx_id = tx.id();

            let table_name = match &plan {
                PhysicalPlan::Insert(n) => &n.table_name,
                PhysicalPlan::Update(n) => &n.table_name,
                PhysicalPlan::Delete(n) => &n.table_name,
                _ => unreachable!("is_dml guarantees a DML plan"),
            };
            let abort_tables = database
                .table_manager
                .get_table(table_name)
                .await
                .ok()
                .map(|tm| HashMap::from([(table_name.clone(), tm)]));

            let executor_creation_start = if profiling {
                Some(Instant::now())
            } else {
                None
            };
            let executor = match create_executor_from_plan(plan, database, Some(tx_id)).await {
                Ok(e) => e,
                Err(e) => {
                    if let Some(abort_tables) = &abort_tables {
                        let _ = database
                            .transaction_manager
                            .abort(tx, &database.buffer_pool, abort_tables)
                            .await;
                    }
                    return Response::Error {
                        message: e.to_string(),
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
                        if let Err(abort_err) = database
                            .transaction_manager
                            .abort(tx, &database.buffer_pool, abort_tables)
                            .await
                        {
                            return Response::Error {
                                message: format!("Abort failed: {}", abort_err),
                            };
                        }
                    }
                }
                _ => {
                    if let Err(commit_err) = database
                        .transaction_manager
                        .commit(tx, &database.buffer_pool)
                        .await
                    {
                        return Response::Error {
                            message: format!("Commit failed: {}", commit_err),
                        };
                    }
                }
            }
            response
        }
        _ => {
            // Query path (and any non-DML/non-DDL).
            let executor_creation_start = if profiling {
                Some(Instant::now())
            } else {
                None
            };
            let executor = match create_executor_from_plan(plan, database, None).await {
                Ok(e) => e,
                Err(e) => {
                    return Response::Error {
                        message: e.to_string(),
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

/// Execute one SQL statement inside an existing user transaction (MS07-T04).
///
/// Reuses the same parse/plan stages as the implicit pipeline; execution
/// goes through `execute_stage_in_tx`, which threads the user's `tx_id`
/// into DML executors and skips the implicit begin/commit/abort wrapping.
pub async fn execute_in_tx(database: &Database, sql: &str, tx_id: u64) -> Response {
    let statements = match parse_stage(sql).await {
        Ok(s) => s,
        Err(message) => return Response::Error { message },
    };
    let stmt = match statements.first() {
        Some(s) => s,
        None => {
            return Response::Error {
                message: "No statement executed".to_string(),
            }
        }
    };
    let plan = match plan_stage(database, sql, stmt, false).await {
        Ok(p) => p,
        Err(message) => return Response::Error { message },
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
            database.plan_cache.clear();
            response
        }
        _ => match create_executor_from_plan(plan, database, Some(tx_id)).await {
            Ok(executor) => execute_executor(executor).await,
            Err(e) => Response::Error {
                message: e.to_string(),
            },
        },
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
            return Response::Error { message };
        }
    };
    if let Some(start) = parse_start {
        record_time("parse", start.elapsed());
    }
    let stmt = match statements.first() {
        Some(s) => s,
        None => {
            return Response::Error {
                message: "No statement executed".to_string(),
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
        Err(message) => {
            if let Some(start) = plan_start {
                record_time("plan", start.elapsed());
            }
            return Response::Error { message };
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

/// Create executor from physical plan (recursive for Filter)
///
/// `tx_id` is `Some(real_tx_id)` for DML nodes (Insert/Update/Delete) and `None`
/// for SELECT-side nodes (Scan/Filter/Join/Aggregate/...). Callers must wrap DML
/// in a real `Transaction` from `TransactionManager::begin()`; see `execute_inner`.
pub(crate) fn create_executor_from_plan(
    plan: PhysicalPlan,
    database: &Database,
    tx_id: Option<u64>,
) -> CreateExecutorFuture<'_> {
    Box::pin(async move {
        match plan {
            PhysicalPlan::Filter(node) => {
                // Recursively create input executor
                let input = create_executor_from_plan(*node.input, database, tx_id).await?;
                Ok(Box::new(
                    FilterExecutor::new(input, node.predicate).with_projection(node.projection),
                ) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Scan(node) => {
                let table_meta = database.table_manager.get_table(&node.table_name).await?;
                Ok(Box::new(
                    ScanExecutor::new(table_meta, database.buffer_pool.clone(), None)
                        .with_projection(node.projection),
                ) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::DataScan(node) => {
                let table_meta = database.table_manager.get_table(&node.table_name).await?;
                Ok(Box::new(
                    DataScanExecutor::new(
                        table_meta,
                        database.buffer_pool.clone(),
                        None,
                        node.predicate,
                        node.scan_cap,
                    )
                    .with_projection(node.projection),
                ) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::IndexScan(node) => {
                let table_meta = database.table_manager.get_table(&node.table_name).await?;
                Ok(Box::new(
                    IndexScanExecutor::new(
                        table_meta,
                        database.buffer_pool.clone(),
                        node.key.as_bytes().to_vec(),
                        None,
                    )
                    .with_projection(node.projection),
                ) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::IndexScanAll(node) => {
                let table_meta = database.table_manager.get_table(&node.table_name).await?;
                Ok(Box::new(
                    IndexScanAllExecutor::new(
                        table_meta,
                        database.buffer_pool.clone(),
                        node.key.as_bytes().to_vec(),
                        None,
                    )
                    .with_projection(node.projection),
                ) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Insert(node) => {
                let table_meta = database.table_manager.get_table(&node.table_name).await?;
                Ok(Box::new(InsertExecutor::with_table_manager(
                    table_meta,
                    Some(database.table_manager.clone()),
                    database.buffer_pool.clone(),
                    database.transaction_manager.clone(),
                    node.values,
                    tx_id.expect("DML Insert requires a transaction id"),
                    Some(database.wal_buffer.clone()),
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Update(node) => {
                let table_meta = database.table_manager.get_table(&node.table_name).await?;
                Ok(Box::new(UpdateExecutor::new(
                    table_meta,
                    database.buffer_pool.clone(),
                    database.transaction_manager.clone(),
                    node.key.as_bytes().to_vec(),
                    node.column,
                    node.new_value,
                    tx_id.expect("DML Update requires a transaction id"),
                    Some(database.wal_buffer.clone()),
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Delete(node) => {
                let table_meta = database.table_manager.get_table(&node.table_name).await?;
                let index_manager = table_meta.index_manager.clone();
                Ok(Box::new(DeleteExecutor::new(
                    index_manager,
                    database.buffer_pool.clone(),
                    database.transaction_manager.clone(),
                    table_meta.name.clone(),
                    node.key.as_bytes().to_vec(),
                    tx_id.expect("DML Delete requires a transaction id"),
                    Some(database.wal_buffer.clone()),
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::CreateTable(_) | PhysicalPlan::DropTable(_) => {
                panic!("DDL should be handled separately in execute()")
            }

            PhysicalPlan::Aggregate(node) => {
                let AggregateNode {
                    input,
                    group_by,
                    aggregates,
                    output_columns,
                    table_name: _,
                    column_indices,
                } = node;
                let input_executor = create_executor_from_plan(*input, database, tx_id).await?;
                Ok(Box::new(AggregateExecutor::new(
                    input_executor,
                    group_by,
                    aggregates,
                    output_columns,
                    column_indices,
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Having(node) => {
                let input = create_executor_from_plan(*node.input, database, tx_id).await?;
                Ok(Box::new(HavingExecutor::new(input, node.predicate))
                    as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Sort(node) => {
                // Recursively create input executor
                let input = create_executor_from_plan(*node.input, database, tx_id).await?;
                Ok(Box::new(
                    SortExecutor::new(input, node.order_by, node.columns)
                        .with_projection(node.projection),
                ) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Limit(node) => {
                // Recursively create input executor
                let input = create_executor_from_plan(*node.input, database, tx_id).await?;
                Ok(Box::new(LimitExecutor::new(input, node.limit, node.offset))
                    as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Join(join_node) => {
                // Build column index mappings from ScanNodes (must do before moving)
                let (left_column_indices, left_table_name) =
                    extract_column_indices(&join_node.left)?;
                let (right_column_indices, right_table_name) =
                    extract_column_indices(&join_node.right)?;

                // Build left executor recursively
                let left_executor =
                    create_executor_from_plan(*join_node.left, database, tx_id).await?;

                // Build right executor recursively
                let right_executor =
                    create_executor_from_plan(*join_node.right, database, tx_id).await?;

                Ok(Box::new(JoinExecutor::new(JoinConfig {
                    left_executor,
                    right_executor,
                    conditions: join_node.conditions.clone(),
                    output_columns: join_node.output_columns.clone(),
                    left_column_indices,
                    right_column_indices,
                    left_table_name,
                    right_table_name,
                })) as Box<dyn Executor + Send>)
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
                let left_executor = create_executor_from_plan(*node.left, database, tx_id).await?;
                let right_executor =
                    create_executor_from_plan(*node.right, database, tx_id).await?;

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
                let left_executor = create_executor_from_plan(*node.left, database, tx_id).await?;
                let right_executor =
                    create_executor_from_plan(*node.right, database, tx_id).await?;

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
                })) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::SubqueryEval(node) => {
                let (outer_column_indices, _) = extract_column_indices(&node.input)?;
                let input_executor =
                    create_executor_from_plan(*node.input, database, tx_id).await?;
                Ok(Box::new(SubqueryEvalExecutor::new(
                    input_executor,
                    *node.subquery,
                    node.output_column.clone(),
                    node.result_column_index,
                    node.correlated_params.clone(),
                    outer_column_indices,
                    Arc::new(database.clone()),
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::DerivedScan(node) => {
                // Materialize subquery results into memory
                let mut subquery_executor =
                    create_executor_from_plan(*node.subquery, database, tx_id).await?;
                let mut rows = Vec::new();
                loop {
                    match subquery_executor.next().await? {
                        Some(ExecResult::Row(row)) => rows.push(row),
                        Some(_) => {} // skip non-row results
                        None => break,
                    }
                }
                Ok(Box::new(DerivedScanExecutor::new(rows)) as Box<dyn Executor + Send>)
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
            // Recurse into input, then add the scalar result column
            let (mut indices, table_name) = extract_column_indices(&node.input)?;
            let next_idx = indices.len();
            indices.insert(node.output_column.to_lowercase(), next_idx);
            Ok((indices, table_name))
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
            let indices: HashMap<String, usize> = node
                .columns
                .iter()
                .enumerate()
                .map(|(idx, col)| (col.to_lowercase(), idx))
                .collect();
            Ok((indices, node.table_name.clone()))
        }
        _ => Err(crate::storage::StorageError::ExecutionError(
            "Expected Scan, Join, SemiJoin, AntiJoin, Filter, Aggregate, SubqueryEval, DerivedScan, IndexScan, or IndexScanAll".into(),
        )),
    }
}

/// Extract all table names from a statement (including JOIN tables)
fn extract_all_table_names(stmt: &Statement) -> Vec<String> {
    match stmt {
        Statement::Query(query) => extract_all_query_table_names(query),
        Statement::Insert { table_name, .. } => vec![table_name.to_string().to_lowercase()],
        Statement::Update { table, .. } => extract_all_from_table_with_joins_item(table),
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
        _ => Vec::new(),
    }
}

/// Extract all table names from a query (including JOIN tables, derived subqueries, and WHERE/SELECT subqueries)
fn extract_all_query_table_names(query: &Query) -> Vec<String> {
    match query.body.as_ref() {
        SetExpr::Select(select) => {
            let from_tables: Vec<String> = select
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
            let projection_tables: Vec<String> = select
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
        _ => Vec::new(),
    }
}

/// Extract table names from subqueries inside an expression
fn extract_subquery_tables_from_expr(expr: &Expr) -> Vec<String> {
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
fn extract_all_from_table_with_joins_item(twj: &TableWithJoins) -> Vec<String> {
    let mut tables = Vec::new();

    // Main table
    match &twj.relation {
        TableFactor::Table { name, .. } => {
            tables.push(name.to_string().to_lowercase());
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
                tables.push(name.to_string().to_lowercase());
            }
            TableFactor::Derived { subquery, .. } => {
                tables.extend(extract_all_query_table_names(subquery));
            }
            _ => {}
        }
    }

    tables
}

async fn register_table(
    database: &Database,
    builder: &mut PlanBuilder,
    stmt: &Statement,
) -> std::result::Result<(), String> {
    let table_names = extract_all_table_names(stmt);
    if table_names.is_empty() {
        return Ok(());
    }

    for table_name in table_names {
        match database.table_manager.get_table(&table_name).await {
            Ok(table_meta) => {
                let columns: Vec<String> = table_meta
                    .columns
                    .iter()
                    .map(|(name, _)| name.clone())
                    .collect();
                builder.register_table(&table_meta.name, columns, &table_meta.pk_column);
            }
            Err(e) => return Err(format!("Table '{}' not found: {}", table_name, e)),
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
            err.contains("not found"),
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
}
