use crate::database::Database;
use crate::executor::{
    AggregateExecutor, AggregateNode, AntiJoinExecutor, CreateTableExecutor, DeleteExecutor,
    DerivedScanExecutor, DropTableExecutor, ExecResult, Executor, FilterExecutor, HavingExecutor,
    IndexScanExecutor, IndexScanAllExecutor, InsertExecutor, JoinConfig, JoinExecutor, JoinRelatedConfig, LimitExecutor, PhysicalPlan, ScanExecutor,
    SemiJoinExecutorV2, SortExecutor, SubqueryEvalExecutor, UpdateExecutor, Value,
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
use std::time::{Duration, Instant};

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

    // Check plan cache first
    let cached_plan = {
        if profiling {
            let t0 = Instant::now();
            let result = {
                let mut cache = database.plan_cache.lock().unwrap();
                cache.get(sql).cloned()
            };
            record_time("cache_hit_check", t0.elapsed());
            result
        } else {
            let mut cache = database.plan_cache.lock().unwrap();
            cache.get(sql).cloned()
        }
    };

    if let Some(plan) = cached_plan {
        // Cache hit — skip parse + plan
        if profiling {
            record_time("parse_and_plan", Duration::ZERO);
        }

        let executor_start = if profiling {
            Some(Instant::now())
        } else {
            None
        };
        let executor = match create_executor_from_plan(plan, database).await {
            Ok(e) => e,
            Err(e) => {
                return Response::Error {
                    message: e.to_string(),
                }
            }
        };
        if profiling {
            record_time("executor_creation", executor_start.unwrap().elapsed());
        }

        let exec_start = if profiling {
            Some(Instant::now())
        } else {
            None
        };
        let response = execute_executor(executor).await;
        if profiling {
            record_time("executor_execution", exec_start.unwrap().elapsed());
            print_timings(total_start.unwrap().elapsed());
        }
        return response;
    }

    // Cache miss — parse and plan
    let parse_start = if profiling {
        Some(Instant::now())
    } else {
        None
    };
    let statements = match parse_sql(sql) {
        Ok(s) => s,
        Err(e) => {
            return Response::Error {
                message: format!("Parse error: {}", e),
            }
        }
    };
    if profiling {
        record_time("parse_and_plan", parse_start.unwrap().elapsed());
    }

    if statements.is_empty() {
        return Response::Error {
            message: "Empty SQL".to_string(),
        };
    }

    // Handle the first statement
    if let Some(stmt) = statements.first() {
        match stmt {
            // DDL: CREATE TABLE
            Statement::CreateTable { .. } => {
                let plan = match PlanBuilder::new().build_plan(stmt) {
                    Ok(p) => p,
                    Err(e) => {
                        return Response::Error {
                            message: format!("Plan error: {}", e),
                        }
                    }
                };

                let executor: Box<dyn Executor + Send> =
                    Box::new(CreateTableExecutor::new(plan, Arc::new(database.clone())));
                let response = execute_executor(executor).await;

                database.plan_cache.lock().unwrap().clear();

                if profiling {
                    print_timings(total_start.unwrap().elapsed());
                }

                return response;
            }

            // DDL: DROP TABLE
            Statement::Drop { .. } => {
                let plan = match PlanBuilder::new().build_plan(stmt) {
                    Ok(p) => p,
                    Err(e) => {
                        return Response::Error {
                            message: format!("Plan error: {}", e),
                        }
                    }
                };

                let executor: Box<dyn Executor + Send> =
                    Box::new(DropTableExecutor::new(plan, Arc::new(database.clone())));
                let response = execute_executor(executor).await;

                database.plan_cache.lock().unwrap().clear();

                if profiling {
                    print_timings(total_start.unwrap().elapsed());
                }

                return response;
            }

            // Query, Insert, Update, Delete
            _ => {
                let table_lookup_start = if profiling {
                    Some(Instant::now())
                } else {
                    None
                };
                let mut plan_builder = PlanBuilder::new();
                if let Err(e) = register_table(database, &mut plan_builder, stmt).await {
                    return Response::Error { message: e };
                }
                if profiling {
                    record_time(
                        "table_metadata_lookup",
                        table_lookup_start.unwrap().elapsed(),
                    );
                }

                let plan = match plan_builder.build_plan(stmt) {
                    Ok(p) => p,
                    Err(e) => {
                        return Response::Error {
                            message: format!("Plan error: {}", e),
                        }
                    }
                };

                if is_cacheable(stmt) {
                    let mut cache = database.plan_cache.lock().unwrap();
                    cache.put(sql.to_string(), plan.clone());
                }

                let executor_start = if profiling {
                    Some(Instant::now())
                } else {
                    None
                };
                let executor = match create_executor_from_plan(plan, database).await {
                    Ok(e) => e,
                    Err(e) => {
                        return Response::Error {
                            message: e.to_string(),
                        }
                    }
                };
                if profiling {
                    record_time("executor_creation", executor_start.unwrap().elapsed());
                }

                let exec_start = if profiling {
                    Some(Instant::now())
                } else {
                    None
                };
                let response = execute_executor(executor).await;
                if profiling {
                    record_time("executor_execution", exec_start.unwrap().elapsed());
                    print_timings(total_start.unwrap().elapsed());
                }
                return response;
            }
        }
    }

    Response::Error {
        message: "No statement executed".to_string(),
    }
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
pub(crate) fn create_executor_from_plan(
    plan: PhysicalPlan,
    database: &Database,
) -> CreateExecutorFuture<'_> {
    Box::pin(async move {
        match plan {
            PhysicalPlan::Filter(node) => {
                // Recursively create input executor
                let input = create_executor_from_plan(*node.input, database).await?;
                Ok(Box::new(FilterExecutor::new(input, node.predicate))
                    as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Scan(node) => {
                let table_meta = database.table_manager.get_table(&node.table_name).await?;
                Ok(Box::new(ScanExecutor::new(
                    table_meta,
                    database.buffer_pool.clone(),
                    None,
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::IndexScan(node) => {
                let table_meta = database.table_manager.get_table(&node.table_name).await?;
                Ok(Box::new(IndexScanExecutor::new(
                    table_meta,
                    database.buffer_pool.clone(),
                    node.key.as_bytes().to_vec(),
                    None,
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::IndexScanAll(node) => {
                let table_meta = database.table_manager.get_table(&node.table_name).await?;
                Ok(Box::new(IndexScanAllExecutor::new(
                    table_meta,
                    database.buffer_pool.clone(),
                    node.key.as_bytes().to_vec(),
                    None,
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Insert(node) => {
                let table_meta = database.table_manager.get_table(&node.table_name).await?;
                Ok(Box::new(InsertExecutor::new(
                    table_meta,
                    database.buffer_pool.clone(),
                    database.transaction_manager.clone(),
                    node.values,
                    0, // placeholder, will be set by execute_inner
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
                    0, // placeholder, will be set by execute_inner
                    Some(database.wal_buffer.clone()),
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Delete(node) => {
                let table_meta = database.table_manager.get_table(&node.table_name).await?;
                let index_manager = table_meta.index_manager.clone();
                Ok(Box::new(DeleteExecutor::new(
                    index_manager,
                    table_meta.name.clone(),
                    node.key.as_bytes().to_vec(),
                    0, // placeholder, will be set by execute_inner
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
                let input_executor = create_executor_from_plan(*input, database).await?;
                Ok(Box::new(AggregateExecutor::new(
                    input_executor,
                    group_by,
                    aggregates,
                    output_columns,
                    column_indices,
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Having(node) => {
                let input = create_executor_from_plan(*node.input, database).await?;
                Ok(Box::new(HavingExecutor::new(input, node.predicate))
                    as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Sort(node) => {
                // Recursively create input executor
                let input = create_executor_from_plan(*node.input, database).await?;
                Ok(
                    Box::new(SortExecutor::new(input, node.order_by, node.columns))
                        as Box<dyn Executor + Send>,
                )
            }

            PhysicalPlan::Limit(node) => {
                // Recursively create input executor
                let input = create_executor_from_plan(*node.input, database).await?;
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
                let left_executor = create_executor_from_plan(*join_node.left, database).await?;

                // Build right executor recursively
                let right_executor = create_executor_from_plan(*join_node.right, database).await?;

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
                let left_executor = create_executor_from_plan(*node.left, database).await?;
                let right_executor = create_executor_from_plan(*node.right, database).await?;

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
                let left_executor = create_executor_from_plan(*node.left, database).await?;
                let right_executor = create_executor_from_plan(*node.right, database).await?;

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
                let input_executor = create_executor_from_plan(*node.input, database).await?;
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
                    create_executor_from_plan(*node.subquery, database).await?;
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

fn value_to_json(value: Value) -> serde_json::Value {
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
