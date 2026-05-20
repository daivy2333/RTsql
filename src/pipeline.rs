use crate::database::Database;
use crate::executor::{
    CreateTableExecutor, DeleteExecutor, DropTableExecutor, ExecResult, Executor, FilterExecutor,
    IndexScanExecutor, InsertExecutor, PhysicalPlan, ScanExecutor, UpdateExecutor, Value,
};
use crate::network::protocol::Response;
use crate::parser::{parse_sql, PlanBuilder};
use crate::storage::Result;
use sqlparser::ast::{Query, SetExpr, Statement, TableFactor, TableWithJoins};
use std::sync::Arc;

pub async fn execute(database: &Database, sql: &str) -> Response {
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
                return execute_executor(executor).await;
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
                return execute_executor(executor).await;
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
                    node.values,
                    0,
                )) as Box<dyn Executor + Send>)
            }

            PhysicalPlan::Update(node) => {
                let table_meta = database.table_manager.get_table(&node.table_name).await?;
                Ok(Box::new(UpdateExecutor::new(
                    table_meta,
                    database.buffer_pool.clone(),
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
                let input = create_executor_from_plan(*node.input, database).await?;
                // TODO: Task 3 - Implement SortExecutor
                Ok(input)
            }

            PhysicalPlan::Limit(node) => {
                let input = create_executor_from_plan(*node.input, database).await?;
                // TODO: Task 5 - Implement LimitExecutor
                Ok(input)
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

fn extract_table_name(stmt: &Statement) -> Option<String> {
    match stmt {
        Statement::Query(query) => extract_query_table_name(query),
        Statement::Insert { table_name, .. } => Some(table_name.to_string().to_lowercase()),
        Statement::Update { table, .. } => extract_from_table_with_joins(table),
        Statement::Delete { from, .. } => {
            let tables = match from {
                sqlparser::ast::FromTable::WithFromKeyword(t) => t.clone(),
                sqlparser::ast::FromTable::WithoutKeyword(t) => t.clone(),
            };
            tables.first().and_then(extract_from_table_with_joins)
        }
        _ => None,
    }
}

fn extract_query_table_name(query: &Query) -> Option<String> {
    match query.body.as_ref() {
        SetExpr::Select(select) => {
            let from = &select.from;
            from.first().and_then(extract_from_table_with_joins)
        }
        _ => None,
    }
}

fn extract_from_table_with_joins(twj: &TableWithJoins) -> Option<String> {
    match &twj.relation {
        TableFactor::Table { name, .. } => Some(name.to_string().to_lowercase()),
        _ => None,
    }
}

async fn register_table(
    database: &Database,
    builder: &mut PlanBuilder,
    stmt: &Statement,
) -> std::result::Result<(), String> {
    let table_name = match extract_table_name(stmt) {
        Some(name) => name,
        None => return Ok(()),
    };

    match database.table_manager.get_table(&table_name).await {
        Ok(table_meta) => {
            let columns: Vec<String> = table_meta
                .columns
                .iter()
                .map(|(name, _)| name.clone())
                .collect();
            builder.register_table(&table_meta.name, columns, &table_meta.pk_column);
            Ok(())
        }
        Err(e) => Err(format!("Table '{}' not found: {}", table_name, e)),
    }
}
