use crate::database::Database;
use crate::executor::{
    CreateTableExecutor, DeleteExecutor, DropTableExecutor, ExecResult, Executor, FilterExecutor,
    IndexScanExecutor, InsertExecutor, JoinExecutor, LimitExecutor, PhysicalPlan, ScanExecutor, SortExecutor,
    UpdateExecutor, Value,
};
use crate::network::protocol::Response;
use crate::parser::{parse_sql, PlanBuilder};
use crate::storage::Result;
use sqlparser::ast::{Query, SetExpr, Statement, TableFactor, TableWithJoins};
use std::collections::HashMap;
use std::sync::Arc;

/// Returns true if the statement is cacheable (SELECT queries only).
fn is_cacheable(stmt: &Statement) -> bool {
    matches!(stmt, Statement::Query(_))
}

pub async fn execute(database: &Database, sql: &str) -> Response {
    // Check plan cache first
    let cached_plan = {
        let mut cache = database.plan_cache.lock().unwrap();
        cache.get(sql).cloned()
    };
    if let Some(plan) = cached_plan {
        // Cache hit — skip parse + plan, go straight to execution
        let executor = match create_executor_from_plan(plan, database).await {
            Ok(e) => e,
            Err(e) => {
                return Response::Error {
                    message: e.to_string(),
                }
            }
        };
        return execute_executor(executor).await;
    }

    let statements = match parse_sql(sql) {
        Ok(s) => s,
        Err(e) => {
            return Response::Error {
                message: format!("Parse error: {}", e),
            }
        }
    };

    if statements.is_empty() {
        return Response::Error {
            message: "Empty SQL".to_string(),
        };
    }

    // Handle the first statement (single-statement execution)
    if let Some(stmt) = statements.first() {
        match stmt {
            // DDL: CREATE TABLE - no need to register table first
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

                // DDL invalidates all cached plans
                database.plan_cache.lock().unwrap().clear();

                return response;
            }

            // DDL: DROP TABLE - no need to register table first
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

                // DDL invalidates all cached plans
                database.plan_cache.lock().unwrap().clear();

                return response;
            }

            // Query, Insert, Update, Delete - need table metadata
            _ => {
                let mut plan_builder = PlanBuilder::new();
                if let Err(e) = register_table(database, &mut plan_builder, stmt).await {
                    return Response::Error { message: e };
                }

                let plan = match plan_builder.build_plan(stmt) {
                    Ok(p) => p,
                    Err(e) => {
                        return Response::Error {
                            message: format!("Plan error: {}", e),
                        }
                    }
                };

                // Store in cache if this is a cacheable statement (SELECT only)
                if is_cacheable(stmt) {
                    let mut cache = database.plan_cache.lock().unwrap();
                    cache.put(sql.to_string(), plan.clone());
                }

                let executor = match create_executor_from_plan(plan, database).await {
                    Ok(e) => e,
                    Err(e) => {
                        return Response::Error {
                            message: e.to_string(),
                        }
                    }
                };
                return execute_executor(executor).await;
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
fn create_executor_from_plan(
    plan: PhysicalPlan,
    database: &Database,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<Box<dyn Executor + Send>>> + Send + '_>,
> {
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

            PhysicalPlan::Insert(node) => {
                let table_meta = database.table_manager.get_table(&node.table_name).await?;
                Ok(Box::new(InsertExecutor::new(
                    table_meta,
                    database.buffer_pool.clone(),
                    database.transaction_manager.clone(),
                    node.values,
                    0,
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
                    0,
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Delete(node) => {
                let table_meta = database.table_manager.get_table(&node.table_name).await?;
                let index_manager = table_meta.index_manager.clone();
                Ok(Box::new(DeleteExecutor::new(
                    index_manager,
                    node.key.as_bytes().to_vec(),
                    0,
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::CreateTable(_) | PhysicalPlan::DropTable(_) => {
                panic!("DDL should be handled separately in execute()")
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
                let (left_column_indices, left_table_name) = extract_column_indices(&join_node.left)?;
                let (right_column_indices, right_table_name) = extract_column_indices(&join_node.right)?;

                // Build left executor recursively
                let left_executor = create_executor_from_plan(*join_node.left, database).await?;

                // Build right executor recursively
                let right_executor = create_executor_from_plan(*join_node.right, database).await?;

                Ok(Box::new(JoinExecutor::new(
                    left_executor,
                    right_executor,
                    join_node.conditions.clone(),
                    join_node.output_columns.clone(),
                    left_column_indices,
                    right_column_indices,
                    left_table_name,
                    right_table_name,
                )) as Box<dyn Executor + Send>)
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

fn extract_column_indices(
    plan: &PhysicalPlan,
) -> Result<(HashMap<String, usize>, String)> {
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
        _ => Err(crate::storage::StorageError::ExecutionError(
            "Expected Scan or Join".into(),
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

/// Extract all table names from a query (including JOIN tables)
fn extract_all_query_table_names(query: &Query) -> Vec<String> {
    match query.body.as_ref() {
        SetExpr::Select(select) => {
            select
                .from
                .iter()
                .flat_map(extract_all_from_table_with_joins_item)
                .collect()
        }
        _ => Vec::new(),
    }
}

/// Extract all table names from a TableWithJoins (including JOIN tables)
fn extract_all_from_table_with_joins_item(twj: &TableWithJoins) -> Vec<String> {
    let mut tables = Vec::new();

    // Main table
    if let TableFactor::Table { name, .. } = &twj.relation {
        tables.push(name.to_string().to_lowercase());
    }

    // JOIN tables
    for join in &twj.joins {
        if let TableFactor::Table { name, .. } = &join.relation {
            tables.push(name.to_string().to_lowercase());
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
