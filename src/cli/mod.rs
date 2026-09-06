//! CLI —— one-shot 命令入口：参数解析、名称解析、三阶段执行、渲染、退出码

pub mod render;
pub mod resolve;

use crate::database::Database;
use crate::network::protocol::Response;
use crate::parser::PlanBuilder;
use crate::pipeline::{execute_stage, parse_stage, plan_stage};
use clap::{Parser, ValueEnum};
use render::{render, OutputKind, QueryPayload};
use std::io::{IsTerminal, Write};
use std::process::ExitCode;

/// 退出码分类：0 成功 / 1 一般错误 / 2 用法错误 / 3 SQL 错误 / 4 锁冲突 / 5 密钥错误。
///
/// Locked 与 InvalidKey 当前无产生路径（跨进程文件锁 T02、密钥 MS12 落地），仅枚举留位。
pub enum ExitStatus {
    Success,
    General(String),
    Usage(String),
    Sql(String),
    Locked(String),
    InvalidKey(String),
}

impl ExitStatus {
    fn message(&self) -> Option<&str> {
        match self {
            ExitStatus::Success => None,
            ExitStatus::General(m)
            | ExitStatus::Usage(m)
            | ExitStatus::Sql(m)
            | ExitStatus::Locked(m)
            | ExitStatus::InvalidKey(m) => Some(m),
        }
    }
}

impl From<&ExitStatus> for ExitCode {
    fn from(status: &ExitStatus) -> Self {
        let code = match status {
            ExitStatus::Success => 0,
            ExitStatus::General(_) => 1,
            ExitStatus::Usage(_) => 2,
            ExitStatus::Sql(_) => 3,
            ExitStatus::Locked(_) => 4,
            ExitStatus::InvalidKey(_) => 5,
        };
        ExitCode::from(code)
    }
}

/// rtsql 一次性 SQL 执行命令
#[derive(Parser)]
#[command(
    name = "rtsql",
    version,
    about = "One-shot SQL execution against an RTsql database"
)]
struct CliArgs {
    /// 数据库：裸名（集中存储）或含 `/` 的文件路径
    db: String,
    /// 要执行的单条 SQL 语句
    sql: String,
    /// 输出格式（默认：TTY 用 table，非 TTY 用 json）
    #[arg(short, long, value_enum)]
    format: Option<FormatArg>,
}

#[derive(ValueEnum, Clone, Copy)]
enum FormatArg {
    Table,
    Json,
    Csv,
    Tsv,
}

/// CLI 主入口：参数解析（clap 用法错误自行退出 2）→ 执行 → 渲染/报错 → 退出码。
pub async fn run() -> ExitCode {
    let args = CliArgs::parse();
    let status = execute_command(&args).await;
    if let Some(message) = status.message() {
        emit_stderr(message);
    }
    ExitCode::from(&status)
}

async fn execute_command(args: &CliArgs) -> ExitStatus {
    let db_path = match resolve::resolve_db_path(&args.db) {
        Ok(path) => path,
        Err(e) => return ExitStatus::General(e),
    };

    let db = match Database::open(&db_path).await {
        Ok(db) => db,
        Err(e) => {
            return ExitStatus::General(format!(
                "failed to open database {}: {}",
                db_path.display(),
                e
            ));
        }
    };

    let status = run_sql(&db, &args.sql, args.format).await;

    match db.close().await {
        Ok(()) => status,
        // 原始错误（如 SQL 错）比 close 失败更相关；数据已由 WAL 兜底
        Err(e) => match status {
            ExitStatus::Success => ExitStatus::General(format!("close failed: {}", e)),
            other => other,
        },
    }
}

async fn run_sql(db: &Database, sql: &str, format: Option<FormatArg>) -> ExitStatus {
    let statements = match parse_stage(sql).await {
        Ok(statements) => statements,
        Err(e) => return ExitStatus::Sql(e),
    };
    if statements.len() > 1 {
        return ExitStatus::Sql(format!(
            "one statement at a time: got {} statements; `;` splitting is not supported yet (lands with MS10-T04)",
            statements.len()
        ));
    }

    let plan = match plan_stage(db, sql, &statements[0], false).await {
        Ok(plan) => plan,
        Err(e) => return ExitStatus::Sql(e),
    };
    let columns = PlanBuilder::new().get_plan_output_columns(&plan);

    match execute_stage(db, plan, false).await {
        Response::QueryResult { rows } => emit(kind(format), &columns, &QueryPayload::Rows(rows)),
        Response::AffectedRows { count } => emit(kind(format), &[], &QueryPayload::Affected(count)),
        Response::Error { message } => ExitStatus::Sql(message),
        Response::Pong => ExitStatus::Success,
    }
}

fn emit(kind: OutputKind, columns: &[String], payload: &QueryPayload) -> ExitStatus {
    match emit_stdout(&render(kind, columns, payload)) {
        Ok(()) => ExitStatus::Success,
        Err(e) => ExitStatus::General(e),
    }
}

/// TTY 默认 table，非 TTY 默认 json；显式 `--format` 覆盖。
fn kind(format: Option<FormatArg>) -> OutputKind {
    match format {
        Some(FormatArg::Table) => OutputKind::Table,
        Some(FormatArg::Json) => OutputKind::Json,
        Some(FormatArg::Csv) => OutputKind::Csv,
        Some(FormatArg::Tsv) => OutputKind::Tsv,
        None => {
            if std::io::stdout().is_terminal() {
                OutputKind::Table
            } else {
                OutputKind::Json
            }
        }
    }
}

fn emit_stdout(text: &str) -> Result<(), String> {
    let mut stdout = std::io::stdout();
    stdout
        .write_all(text.as_bytes())
        .and_then(|_| stdout.write_all(b"\n"))
        .and_then(|_| stdout.flush())
        .map_err(|e| format!("write to stdout failed: {}", e))
}

fn emit_stderr(message: &str) {
    let mut stderr = std::io::stderr();
    let _ = writeln!(stderr, "{}", message);
}
