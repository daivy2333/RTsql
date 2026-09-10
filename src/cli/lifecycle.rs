//! 生命周期子命令实现 —— new / list / schema / dump / restore / import。
//!
//! 开库子命令复用 `execute_command_inner` 两阶段信号编排（open → work → close），
//! 锁冲突与格式拒绝由 open 链既有语义自然继承。

use super::render::{render, QueryPayload};
use super::resolve;
use super::{emit_stdout, kind, sigint_future, sigterm_future, ExitStatus, FormatArg};
use crate::database::Database;
use crate::network::protocol::Response;
use crate::pipeline::{execute_stage, parse_stage, plan_stage};
use crate::storage::catalog::{CatalogColumnRow, CatalogRow};
use crate::storage::page_format::ColumnType;

/// `new <name|path>`：已存在拒绝 → 建父目录 → open 建库 + close checkpoint → 静默。
pub(super) async fn new_db(target: &str) -> ExitStatus {
    let db_path = match resolve::resolve_db_path(target) {
        Ok(path) => path,
        Err(e) => return ExitStatus::General(e),
    };
    if db_path.exists() {
        return ExitStatus::General(format!("{} already exists", db_path.display()));
    }
    if let Some(parent) = db_path.parent() {
        if !parent.as_os_str().is_empty() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                return ExitStatus::General(format!(
                    "failed to create directory {}: {}",
                    parent.display(),
                    e
                ));
            }
        }
    }
    // work 为空闭包：open 建库（0 字节写头 + catalog bootstrap）+ close 落盘即全部工作
    super::execute_command_inner(
        &db_path,
        move |_db| Box::pin(async { ExitStatus::Success }),
        sigint_future,
        sigterm_future,
    )
    .await
}

/// `list`：枚举集中区 `*.db` 常规文件（名称、大小），按名称排序渲染为行集；不开库。
pub(super) async fn list(format: Option<FormatArg>) -> ExitStatus {
    let dir = match resolve::db_dir() {
        Ok(dir) => dir,
        Err(e) => return ExitStatus::General(e),
    };

    let mut files: Vec<(String, u64)> = Vec::new();
    match std::fs::read_dir(&dir) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                if !file_type.is_file() {
                    continue;
                }
                let path = entry.path();
                if path.extension().is_none_or(|ext| ext != "db") {
                    continue;
                }
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                files.push((name, size));
            }
        }
        // 目录不存在：空行集（与空目录同语义）
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return ExitStatus::General(format!(
                "failed to read directory {}: {}",
                dir.display(),
                e
            ))
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let rows = files
        .into_iter()
        .map(|(name, size)| vec![serde_json::Value::from(name), serde_json::Value::from(size)])
        .collect();
    let text = render(
        kind(format),
        &["name".to_string(), "size_bytes".to_string()],
        &QueryPayload::Rows(rows),
    );
    match emit_stdout(&text) {
        Ok(()) => ExitStatus::Success,
        Err(e) => ExitStatus::General(e),
    }
}

/// `schema <db>`：逐用户表输出一行 CREATE TABLE DDL；空库无输出、静默 exit 0。
pub(super) async fn schema(db: &str) -> ExitStatus {
    let db_path = match resolve::resolve_db_path(db) {
        Ok(path) => path,
        Err(e) => return ExitStatus::General(e),
    };
    if !db_path.exists() {
        return ExitStatus::General(format!("{} does not exist", db_path.display()));
    }
    super::execute_command_inner(
        &db_path,
        move |db| {
            Box::pin(async move {
                // 系统表不可经 SQL 查询，DDL 数据只能走内部 catalog 读 API
                let catalog = db.table_manager.catalog();
                let tables = match catalog.scan_tables().await {
                    Ok(tables) => tables,
                    Err(e) => return ExitStatus::General(format!("failed to scan catalog: {}", e)),
                };
                for table in tables {
                    let mut columns = match catalog.scan_columns(&table.table_name).await {
                        Ok(columns) => columns,
                        Err(e) => {
                            return ExitStatus::General(format!("failed to scan catalog: {}", e))
                        }
                    };
                    columns.sort_by_key(|col| col.column_index);
                    let ddl = create_table_sql(&table, &columns);
                    if let Err(e) = emit_stdout(&ddl) {
                        return ExitStatus::General(e);
                    }
                }
                ExitStatus::Success
            })
        },
        sigint_future,
        sigterm_future,
    )
    .await
}

/// `dump <db>`：逐用户表输出 DDL 行 + 全行 INSERT 语句流（SQL 文本导出）；
/// 空库无输出、静默 exit 0。catalog 表序即输出序。
pub(super) async fn dump(db: &str) -> ExitStatus {
    let db_path = match resolve::resolve_db_path(db) {
        Ok(path) => path,
        Err(e) => return ExitStatus::General(e),
    };
    if !db_path.exists() {
        return ExitStatus::General(format!("{} does not exist", db_path.display()));
    }
    super::execute_command_inner(
        &db_path,
        move |db| {
            Box::pin(async move {
                let catalog = db.table_manager.catalog();
                let tables = match catalog.scan_tables().await {
                    Ok(tables) => tables,
                    Err(e) => return ExitStatus::General(format!("failed to scan catalog: {}", e)),
                };
                for table in tables {
                    let mut columns = match catalog.scan_columns(&table.table_name).await {
                        Ok(columns) => columns,
                        Err(e) => {
                            return ExitStatus::General(format!("failed to scan catalog: {}", e))
                        }
                    };
                    columns.sort_by_key(|col| col.column_index);
                    if let Err(e) = emit_stdout(&create_table_sql(&table, &columns)) {
                        return ExitStatus::General(e);
                    }
                    let rows = match select_all_rows(db, &table.table_name).await {
                        Ok(rows) => rows,
                        Err(status) => return status,
                    };
                    for row in rows {
                        let values: Vec<String> = row.iter().map(sql_literal).collect();
                        let insert = format!(
                            "INSERT INTO {} VALUES ({});",
                            quote_ident(&table.table_name),
                            values.join(", ")
                        );
                        if let Err(e) = emit_stdout(&insert) {
                            return ExitStatus::General(e);
                        }
                    }
                }
                ExitStatus::Success
            })
        },
        sigint_future,
        sigterm_future,
    )
    .await
}

/// `SELECT * FROM <t>` 经 pipeline 三 stage 取行集。表名用 catalog 原名而非
/// quote_ident：引擎以 ObjectName 的 Display 形式为表名（pipeline/planner 各
/// 提取点直接 to_string 查表），裸名建表的目录名不含引号、带引号建表的目录名
/// 本身含引号——原名写入 SQL 经 sqlparser Display 往返后与目录名恒等，两种
/// 来源都正确解析（quote_ident 反而会给裸名表附加引号致查表失败）。SELECT *
/// 恒等投影保证行形状 = schema 列序（MS10-T01 R6，projection_test 锁定）。
async fn select_all_rows(
    db: &Database,
    table: &str,
) -> Result<Vec<Vec<serde_json::Value>>, ExitStatus> {
    let sql = format!("SELECT * FROM {}", table);
    let statements = match parse_stage(&sql).await {
        Ok(statements) => statements,
        Err(e) => {
            return Err(ExitStatus::General(format!(
                "failed to dump table {}: {}",
                table, e
            )))
        }
    };
    let Some(stmt) = statements.first() else {
        return Err(ExitStatus::General(format!(
            "failed to dump table {}: empty statement list",
            table
        )));
    };
    let stmt_text = stmt.to_string();
    let plan = match plan_stage(db, &stmt_text, stmt, false).await {
        Ok(plan) => plan,
        Err(e) => {
            return Err(ExitStatus::General(format!(
                "failed to dump table {}: {}",
                table, e
            )))
        }
    };
    match execute_stage(db, plan, false).await {
        Response::QueryResult { rows } => Ok(rows),
        Response::Error { message } => Err(ExitStatus::General(format!(
            "failed to dump table {}: {}",
            table, message
        ))),
        other => Err(ExitStatus::General(format!(
            "unexpected response while dumping table {}: {:?}",
            table, other
        ))),
    }
}

/// 引擎 JSON 值 → SQL 字面量（dump 生成 INSERT 用）。与 planner 字面量解析
/// 往返：Number 先 i64 后 f64（f64 Display 为最短往返表示）；Bool→TRUE/FALSE；
/// String 单引号加倍；Null→NULL。引擎值只产生以上形状（value_to_json 已把
/// 非有限 Float 转 Null），其余 JSON 形状不可达，防御性归为 NULL。
fn sql_literal(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "NULL".to_string(),
        serde_json::Value::Bool(b) => {
            if *b {
                "TRUE".to_string()
            } else {
                "FALSE".to_string()
            }
        }
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => i.to_string(),
            None => n.to_string(),
        },
        serde_json::Value::String(s) => format!("'{}'", s.replace('\'', "''")),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => "NULL".to_string(),
    }
}

/// `restore <db> <file|->`：目标须为空库；读 dump SQL 文本逐条静默执行
/// （每条独立 auto-commit，逐条生效），成功静默 exit 0。执行期任一语句失败
/// 立即停止：exit 3 序号定位，失败前语句已生效。
pub(super) async fn restore(db: &str, file: &str) -> ExitStatus {
    let db_path = match resolve::resolve_db_path(db) {
        Ok(path) => path,
        Err(e) => return ExitStatus::General(e),
    };
    if !db_path.exists() {
        return ExitStatus::General(format!("{} does not exist", db_path.display()));
    }
    let file = file.to_string();
    // 闭包内错误文案用副本；原 db_path 供编排借用
    let db_path_in_work = db_path.clone();
    super::execute_command_inner(
        &db_path,
        move |db| {
            Box::pin(async move {
                // 空库前置（D8）：先空库检查后读文件（避免对拒绝目标白读大文件）
                let catalog = db.table_manager.catalog();
                match catalog.scan_tables().await {
                    Ok(tables) if !tables.is_empty() => {
                        return ExitStatus::General(format!(
                            "{} is not empty; restore requires an empty database",
                            db_path_in_work.display()
                        ));
                    }
                    Ok(_) => {}
                    Err(e) => return ExitStatus::General(format!("failed to scan catalog: {}", e)),
                }
                let sql_text = match read_restore_input(&file) {
                    Ok(text) => text,
                    Err(e) => return ExitStatus::General(e),
                };
                let statements = match parse_stage(&sql_text).await {
                    Ok(statements) => statements,
                    // parse 全串解析、零执行，错误文本自带行列定位（与主命令同语义）
                    Err(e) => return ExitStatus::Sql(e),
                };
                let total = statements.len();
                for (index, stmt) in statements.iter().enumerate() {
                    let statement_text = stmt.to_string();
                    let plan = match plan_stage(db, &statement_text, stmt, false).await {
                        Ok(plan) => plan,
                        Err(e) => {
                            return super::sql_failure_status(
                                index + 1,
                                total,
                                &e,
                                &statement_text,
                                false,
                            )
                        }
                    };
                    match execute_stage(db, plan, false).await {
                        Response::Error { message } => {
                            return super::sql_failure_status(
                                index + 1,
                                total,
                                &message,
                                &statement_text,
                                false,
                            )
                        }
                        // restore 不渲染：dump 文本面只有 CREATE/INSERT，
                        // 其余响应静默忽略
                        Response::AffectedRows { .. } | Response::QueryResult { .. } => {}
                        Response::Pong => {}
                    }
                }
                ExitStatus::Success
            })
        },
        sigint_future,
        sigterm_future,
    )
    .await
}

/// restore 输入读取：`-` 读 stdin，其余按文件路径读。
fn read_restore_input(file: &str) -> Result<String, String> {
    if file == "-" {
        std::io::read_to_string(std::io::stdin())
            .map_err(|e| format!("failed to read stdin: {}", e))
    } else {
        std::fs::read_to_string(file).map_err(|e| format!("failed to read {}: {}", file, e))
    }
}

/// `import <db> <table> <file> --csv`：CSV 导入已存在的表。首行表头按列名
/// 与表列匹配（顺序无关），逐行按 schema 重排 + 类型转换，逐条 INSERT
/// （auto-commit）；成功输出受影响行数。
pub(super) async fn import_csv(
    db: &str,
    table: &str,
    file: &str,
    csv_flag: bool,
    format: Option<FormatArg>,
) -> ExitStatus {
    // 当前唯一支持格式必须显式声明；先于一切 IO
    if !csv_flag {
        return ExitStatus::Usage(
            "rtsql import requires --csv (the only supported import format)".to_string(),
        );
    }
    let db_path = match resolve::resolve_db_path(db) {
        Ok(path) => path,
        Err(e) => return ExitStatus::General(e),
    };
    if !db_path.exists() {
        return ExitStatus::General(format!("{} does not exist", db_path.display()));
    }
    let table = table.to_string();
    let file = file.to_string();
    let db_path_in_work = db_path.clone();
    super::execute_command_inner(
        &db_path,
        move |db| {
            Box::pin(async move {
                let meta = match db.get_table(&table).await {
                    Ok(meta) => meta,
                    Err(_) => {
                        return ExitStatus::General(format!(
                            "table {} does not exist in {}",
                            table,
                            db_path_in_work.display()
                        ))
                    }
                };
                let mut reader = match csv::Reader::from_path(&file) {
                    Ok(reader) => reader,
                    Err(e) => {
                        return ExitStatus::General(format!("failed to read {}: {}", file, e))
                    }
                };
                let headers = match reader.headers() {
                    Ok(headers) => headers.clone(),
                    Err(e) => {
                        return ExitStatus::General(format!(
                            "failed to read CSV header from {}: {}",
                            file, e
                        ))
                    }
                };
                let schema = meta.columns.clone();

                // 表头匹配：表全部列必须在表头中（缺失 → General 含列名）
                let mut col_index: Vec<usize> = Vec::with_capacity(schema.len());
                for (name, _) in &schema {
                    match headers.iter().position(|h| h == name) {
                        Some(index) => col_index.push(index),
                        None => {
                            return ExitStatus::General(format!(
                                "CSV header is missing column \"{}\" of table {}",
                                name, table
                            ))
                        }
                    }
                }
                // 表头不得含表外列（未知 → General 含列名）
                for header in headers.iter() {
                    if !schema.iter().any(|(name, _)| name == header) {
                        return ExitStatus::General(format!(
                            "CSV header contains unknown column \"{}\" of table {}",
                            header, table
                        ));
                    }
                }

                // 先收齐数据行（n 需已知，row k of n 定位）；RFC4180 引号/转义/
                // 跨行由 csv crate 承载，记录长度不一致按解析错误拒绝
                let records: Vec<csv::StringRecord> =
                    match reader.records().collect::<Result<Vec<_>, _>>() {
                        Ok(records) => records,
                        Err(e) => {
                            return ExitStatus::General(format!("failed to parse {}: {}", file, e))
                        }
                    };

                let total = records.len();
                let mut imported: u64 = 0;
                for (k, record) in records.iter().enumerate() {
                    let row_no = k + 1;
                    let mut values: Vec<String> = Vec::with_capacity(schema.len());
                    for (i, (_, col_type)) in schema.iter().enumerate() {
                        let field = record.get(col_index[i]).unwrap_or("");
                        match csv_value(field, col_type) {
                            Ok(value) => values.push(sql_literal(&value)),
                            Err(reason) => {
                                return ExitStatus::General(format!(
                                    "row {}: column \"{}\": {}",
                                    row_no, schema[i].0, reason
                                ))
                            }
                        }
                    }
                    // 表名/值序：表名用实参原文（与 get_table 比对同形）；全列
                    // schema 序，不用列清单形式（planner INSERT 仅支持全列 VALUES）
                    let insert = format!("INSERT INTO {} VALUES ({});", table, values.join(", "));
                    match db.execute_sql(&insert).await {
                        Response::AffectedRows { count } => imported += count,
                        Response::Error { message } => {
                            return ExitStatus::Sql(format!(
                                "import row {} of {} failed: {}",
                                row_no, total, message
                            ))
                        }
                        other => {
                            return ExitStatus::Sql(format!(
                                "import row {} of {} failed: unexpected response {:?}",
                                row_no, total, other
                            ))
                        }
                    }
                }

                let text = render(kind(format), &[], &QueryPayload::Affected(imported));
                match emit_stdout(&text) {
                    Ok(()) => ExitStatus::Success,
                    Err(e) => ExitStatus::General(e),
                }
            })
        },
        sigint_future,
        sigterm_future,
    )
    .await
}

/// CSV 字段 → 引擎 JSON 值（import 类型转换纯函数）。空字段：非 String 列 →
/// NULL；String 列 → 空串。Bool 大小写不敏感。错误信息含原值，行列上下文
/// 由调用方补充。
fn csv_value(field: &str, col_type: &ColumnType) -> Result<serde_json::Value, String> {
    if field.is_empty() {
        return Ok(match col_type {
            ColumnType::String(_) => serde_json::Value::String(String::new()),
            _ => serde_json::Value::Null,
        });
    }
    match col_type {
        ColumnType::Int => field
            .parse::<i64>()
            .map(serde_json::Value::from)
            .map_err(|_| format!("invalid INT value '{}'", field)),
        ColumnType::Float => field
            .parse::<f64>()
            .map(serde_json::Value::from)
            .map_err(|_| format!("invalid FLOAT value '{}'", field)),
        ColumnType::Bool => match field.to_ascii_lowercase().as_str() {
            "true" => Ok(serde_json::Value::Bool(true)),
            "false" => Ok(serde_json::Value::Bool(false)),
            _ => Err(format!("invalid BOOL value '{}'", field)),
        },
        ColumnType::String(_) => Ok(serde_json::Value::String(field.to_string())),
    }
}

/// 生成一表的单行 CREATE TABLE DDL（含结尾分号；schema 与 dump 共用）。
///
/// 列序由调用方保证为 column_index 升序。类型映射 Int→INT / Float→FLOAT /
/// Bool→BOOL / `String(_)`→STRING（planner `convert_data_type` 四族归一，restore
/// 后恒等；存储长度不表达）。标识符恒双引号（内部 `"` 加倍）。约束序
/// PRIMARY KEY → NOT NULL → UNIQUE。DEFAULT 不在 catalog 持久化面，不输出。
pub(crate) fn create_table_sql(table: &CatalogRow, columns: &[CatalogColumnRow]) -> String {
    let cols: Vec<String> = columns
        .iter()
        .map(|col| {
            let mut part = format!(
                "{} {}",
                quote_ident(&col.column_name),
                column_type_sql(&col.column_type)
            );
            if col.column_name == table.pk_column {
                part.push_str(" PRIMARY KEY");
            }
            if col.not_null {
                part.push_str(" NOT NULL");
            }
            if col.unique {
                part.push_str(" UNIQUE");
            }
            part
        })
        .collect();
    format!(
        "CREATE TABLE {} ({});",
        quote_ident(&table.table_name),
        cols.join(", ")
    )
}

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn column_type_sql(column_type: &ColumnType) -> &'static str {
    match column_type {
        ColumnType::Int => "INT",
        ColumnType::Float => "FLOAT",
        ColumnType::Bool => "BOOL",
        ColumnType::String(_) => "STRING",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::catalog::{CatalogColumnRow, CatalogRow};

    fn table(name: &str, pk: &str) -> CatalogRow {
        CatalogRow {
            table_name: name.to_string(),
            data_page_head: 2,
            index_root_page_id: 0,
            pk_index: 0,
            pk_column: pk.to_string(),
            column_count: 2,
            data_page_tail: 2,
        }
    }

    fn column(
        index: u32,
        name: &str,
        t: ColumnType,
        not_null: bool,
        unique: bool,
    ) -> CatalogColumnRow {
        CatalogColumnRow {
            table_name: "t".to_string(),
            column_index: index,
            column_name: name.to_string(),
            column_type: t,
            not_null,
            unique,
        }
    }

    #[test]
    fn ddl_generator_renders_types_and_constraints() {
        let t = table("users", "id");
        let columns = vec![
            column(0, "id", ColumnType::Int, false, false),
            column(1, "name", ColumnType::String(255), true, false),
        ];

        let ddl = create_table_sql(&t, &columns);
        assert!(
            ddl.contains("CREATE TABLE \"users\""),
            "table ident must be quoted: {ddl}"
        );
        assert!(
            ddl.contains("\"id\" INT PRIMARY KEY"),
            "PK must be an inline column constraint: {ddl}"
        );
        assert!(
            ddl.contains("\"name\" STRING NOT NULL"),
            "NOT NULL must be rendered: {ddl}"
        );
        assert!(
            !ddl.contains("255"),
            "storage-side String length must not leak into DDL: {ddl}"
        );
    }

    #[test]
    fn ddl_generator_quotes_and_orders_constraints() {
        let t = table("we\"ird", "pk");
        let columns = vec![column(0, "pk", ColumnType::Bool, true, true)];

        let ddl = create_table_sql(&t, &columns);
        assert_eq!(
            ddl,
            "CREATE TABLE \"we\"\"ird\" (\"pk\" BOOL PRIMARY KEY NOT NULL UNIQUE);"
        );
    }

    #[test]
    fn sql_literal_escaping() {
        use serde_json::json;
        assert_eq!(sql_literal(&json!(null)), "NULL");
        assert_eq!(sql_literal(&json!(true)), "TRUE");
        assert_eq!(sql_literal(&json!(false)), "FALSE");
        assert_eq!(sql_literal(&json!(42)), "42");
        assert_eq!(sql_literal(&json!(-7)), "-7");
        assert_eq!(sql_literal(&json!(3.5)), "3.5");
        assert_eq!(sql_literal(&json!(-0.25)), "-0.25");
        assert_eq!(sql_literal(&json!("it's")), "'it''s'");
        assert_eq!(sql_literal(&json!("")), "''");
    }

    #[test]
    fn csv_value_conversion() {
        use serde_json::json;
        assert_eq!(csv_value("42", &ColumnType::Int).unwrap(), json!(42));
        assert_eq!(csv_value("-7", &ColumnType::Int).unwrap(), json!(-7));
        assert!(csv_value("1.5", &ColumnType::Int).is_err());
        assert_eq!(csv_value("3.5", &ColumnType::Float).unwrap(), json!(3.5));
        assert!(csv_value("1,5", &ColumnType::Float).is_err());
        assert_eq!(csv_value("TRUE", &ColumnType::Bool).unwrap(), json!(true));
        assert_eq!(csv_value("False", &ColumnType::Bool).unwrap(), json!(false));
        assert!(csv_value("yes", &ColumnType::Bool).is_err());
        assert_eq!(
            csv_value("it's", &ColumnType::String(255)).unwrap(),
            json!("it's")
        );
        // 空字段：非 String 列 → NULL；String 列 → 空串
        assert_eq!(
            csv_value("", &ColumnType::Int).unwrap(),
            serde_json::Value::Null
        );
        assert_eq!(
            csv_value("", &ColumnType::Bool).unwrap(),
            serde_json::Value::Null
        );
        assert_eq!(csv_value("", &ColumnType::String(255)).unwrap(), json!(""));
        // 非法值错误信息含原值
        let err = csv_value("abc", &ColumnType::Int).unwrap_err();
        assert!(
            err.contains("abc"),
            "error must contain original value: {err}"
        );
    }
}
