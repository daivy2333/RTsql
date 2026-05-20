use crate::database::Database;
use crate::executor::{
    DeleteExecutor, ExecResult, Executor, IndexScanExecutor, InsertExecutor, PhysicalPlan,
    ScanExecutor, UpdateExecutor, Value,
};
use crate::network::protocol::Response;
use crate::parser::{parse_sql, PlanBuilder};
use sqlparser::ast::{Query, SetExpr, Statement, TableFactor, TableWithJoins};

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

    let mut plan_builder = PlanBuilder::new();
    if let Err(e) = register_table(database, &mut plan_builder, &statements[0]).await {
        return Response::Error {
            message: format!("Plan error: {}", e),
        };
    }

    let plan = match plan_builder.build_plan(&statements[0]) {
        Ok(p) => p,
        Err(e) => {
            return Response::Error {
                message: format!("Plan error: {}", e),
            }
        }
    };

    let mut executor: Box<dyn Executor + Send> = match plan {
        PhysicalPlan::Scan(node) => {
            match database.table_manager.get_table(&node.table_name).await {
                Ok(table_meta) => Box::new(ScanExecutor::new(
                    table_meta,
                    database.buffer_pool.clone(),
                    None,
                )),
                Err(e) => {
                    return Response::Error {
                        message: e.to_string(),
                    }
                }
            }
        }
        PhysicalPlan::IndexScan(node) => {
            match database.table_manager.get_table(&node.table_name).await {
                Ok(table_meta) => Box::new(IndexScanExecutor::new(
                    table_meta,
                    database.buffer_pool.clone(),
                    node.key.as_bytes().to_vec(),
                    None,
                )),
                Err(e) => {
                    return Response::Error {
                        message: e.to_string(),
                    }
                }
            }
        }
        PhysicalPlan::Insert(node) => {
            match database.table_manager.get_table(&node.table_name).await {
                Ok(table_meta) => Box::new(InsertExecutor::new(
                    table_meta,
                    database.buffer_pool.clone(),
                    node.values,
                    0,
                )),
                Err(e) => {
                    return Response::Error {
                        message: e.to_string(),
                    }
                }
            }
        }
        PhysicalPlan::Update(node) => {
            match database.table_manager.get_table(&node.table_name).await {
                Ok(table_meta) => Box::new(UpdateExecutor::new(
                    table_meta,
                    database.buffer_pool.clone(),
                    node.key.as_bytes().to_vec(),
                    node.column,
                    node.new_value,
                    0,
                )),
                Err(e) => {
                    return Response::Error {
                        message: e.to_string(),
                    }
                }
            }
        }
        PhysicalPlan::Delete(node) => {
            match database.table_manager.get_table(&node.table_name).await {
                Ok(table_meta) => {
                    let index_manager = table_meta.index_manager.clone();
                    Box::new(DeleteExecutor::new(
                        index_manager,
                        node.key.as_bytes().to_vec(),
                        0,
                    ))
                }
                Err(e) => {
                    return Response::Error {
                        message: e.to_string(),
                    }
                }
            }
        }
        PhysicalPlan::CreateTable(_) | PhysicalPlan::DropTable(_) => {
            // TODO: Task 6/7 will implement these executors
            return Response::Error {
                message: "DDL executor not yet implemented".to_string(),
            };
        }
    };

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

fn value_to_json(value: Value) -> serde_json::Value {
    match value {
        Value::Int(n) => serde_json::Value::Number(n.into()),
        Value::String(s) => serde_json::Value::String(s),
        Value::Null => serde_json::Value::Null,
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
) -> Result<(), String> {
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
        Err(_) => Ok(()),
    }
}
