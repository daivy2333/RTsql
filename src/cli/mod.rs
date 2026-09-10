//! CLI —— one-shot 命令入口：参数解析、名称解析、三阶段执行、渲染、退出码

pub mod lifecycle;
pub mod render;
pub mod resolve;

use crate::database::Database;
use crate::network::protocol::Response;
use crate::parser::PlanBuilder;
use crate::pipeline::{execute_stage, parse_stage, plan_stage};
use crate::storage::StorageError;
use clap::{Parser, Subcommand, ValueEnum};
use render::{render, OutputKind, QueryPayload};
use std::future::Future;
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::pin::Pin;
use std::process::ExitCode;

/// 退出码分类：0 成功 / 1 一般错误 / 2 用法错误 / 3 SQL 错误 / 4 锁冲突 / 5 密钥错误。
///
/// InvalidKey 当前无产生路径（密钥 MS12 落地），仅枚举留位。
/// Signaled 携带信号编号，按 POSIX 映射 128+signum（SIGINT→130、SIGTERM→143），
/// 不输出 stderr 消息（130/143 自解释）。
pub enum ExitStatus {
    Success,
    General(String),
    Usage(String),
    Sql(String),
    Locked(String),
    InvalidKey(String),
    Signaled(i32),
}

impl ExitStatus {
    fn message(&self) -> Option<&str> {
        match self {
            ExitStatus::Success | ExitStatus::Signaled(_) => None,
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
            ExitStatus::Signaled(signum) => 128 + signum,
        };
        ExitCode::from(code as u8)
    }
}

/// rtsql 命令行入口：one-shot 主命令 + 生命周期子命令（MS10-T05）
#[derive(Parser)]
#[command(
    name = "rtsql",
    version,
    about = "One-shot SQL execution against an RTsql database"
)]
struct CliArgs {
    /// 数据库：裸名（集中存储）或含 `/` 的文件路径（主命令用）
    db: Option<String>,
    /// 要执行的单条 SQL 语句（主命令用）
    sql: Option<String>,
    /// 输出格式（默认：TTY 用 table，非 TTY 用 json）
    #[arg(short, long, value_enum, global = true)]
    format: Option<FormatArg>,
    #[command(subcommand)]
    command: Option<Command>,
}

/// 生命周期子命令。首个位置参数命中子命令名即分发（子命令优先）：
/// 裸名与子命令同名的数据库需以含 `/` 的路径形式经主命令打开。
#[derive(Subcommand)]
enum Command {
    /// 显式创建空数据库（唯一创建入口；目标已存在时报错）
    New {
        /// 裸名或含 `/` 的文件路径
        target: String,
    },
    /// 枚举集中存储区的数据库文件（不开库）
    List,
    /// 输出各用户表的 CREATE TABLE DDL
    Schema {
        /// 裸名或含 `/` 的文件路径
        db: String,
    },
    /// 导出 SQL 文本（CREATE TABLE + INSERT）
    Dump {
        /// 裸名或含 `/` 的文件路径
        db: String,
    },
    /// 从 dump SQL 文本恢复（`-` 读 stdin；目标须为空库）
    Restore {
        /// 裸名或含 `/` 的文件路径
        db: String,
        /// dump 文本文件路径，或 `-` 表示 stdin
        file: String,
    },
    /// 从 CSV 导入数据（目标表必须已存在；首行表头按列名匹配）
    Import {
        /// 裸名或含 `/` 的文件路径
        db: String,
        /// 目标表名
        table: String,
        /// CSV 文件路径
        file: String,
        /// CSV 格式开关（当前唯一支持格式，必须提供）
        #[arg(long)]
        csv: bool,
    },
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
    match &args.command {
        Some(Command::New { target }) => lifecycle::new_db(target).await,
        Some(Command::List) => lifecycle::list(args.format).await,
        Some(Command::Schema { db }) => lifecycle::schema(db).await,
        Some(Command::Dump { db }) => lifecycle::dump(db).await,
        Some(Command::Restore { db, file }) => lifecycle::restore(db, file).await,
        Some(Command::Import {
            db,
            table,
            file,
            csv,
        }) => lifecycle::import_csv(db, table, file, *csv, args.format).await,
        None => execute_main_command(args).await,
    }
}

/// 主命令臂：db/sql 任一缺失 → 手动 usage（exit 2）；合法输入走既有全链路。
async fn execute_main_command(args: &CliArgs) -> ExitStatus {
    let (db, sql) = match (&args.db, &args.sql) {
        (Some(db), Some(sql)) => (db, sql),
        _ => {
            return ExitStatus::Usage(
                "missing <db> and/or <sql>; usage: rtsql <db> <sql> (or `rtsql --help`)"
                    .to_string(),
            )
        }
    };
    let db_path = match resolve::resolve_db_path(db) {
        Ok(path) => path,
        Err(e) => return ExitStatus::General(e),
    };
    // sql/format 按值捕获：work future 拥有它们，满足 for<'a> 的 HRTB 工厂约束
    let sql = sql.clone();
    let format = args.format;
    execute_command_inner(
        &db_path,
        move |db| Box::pin(async move { run_sql(db, &sql, format).await }),
        sigint_future,
        sigterm_future,
    )
    .await
}

/// 信号 future（SIGINT/SIGTERM）；每个 select 阶段生成新实例。
type SignalFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

/// 阶段 2 的工作负载 future（工厂参数注入，供结构测试替换为确定性源）。
type WorkFuture<'a> = Pin<Box<dyn Future<Output = ExitStatus> + Send + 'a>>;

/// SIGINT future；ctrl_c 安装失败时保持 pending（默认处置仍在，不伪造信号）。
fn sigint_future() -> SignalFuture {
    Box::pin(async {
        if tokio::signal::ctrl_c().await.is_err() {
            std::future::pending::<()>().await;
        }
    })
}

/// SIGTERM future；安装失败或流关闭同样保持 pending，不伪造信号。
fn sigterm_future() -> SignalFuture {
    Box::pin(async {
        let received =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(mut stream) => stream.recv().await.is_some(),
                Err(_) => false,
            };
        if !received {
            std::future::pending::<()>().await;
        }
    })
}

fn open_error_status(db_path: &Path, e: StorageError) -> ExitStatus {
    match e {
        StorageError::DatabaseLocked(_) => {
            ExitStatus::Locked(format!("database is locked: {}", db_path.display()))
        }
        other => ExitStatus::General(format!(
            "failed to open database {}: {}",
            db_path.display(),
            other
        )),
    }
}

/// 两阶段编排（D4）：阶段 1 open、阶段 2 run_sql 各自与信号 future 竞争，
/// 信号 → `Signaled(128+signum)`；已打开时信号退出前仍执行 `close()` checkpoint。
///
/// 取消安全：打开阶段取消与 kill -9 mid-recovery 等价（恢复从不截断 WAL，位点
/// 重放收敛）；执行阶段取消只丢弃未提交 DML（恢复期 uncommitted 清理）。
/// `close()` 期间不新增 select（快路径；二次 Ctrl-C 不强杀，kill -9 兜底）。
async fn execute_command_inner(
    db_path: &Path,
    work: impl for<'a> FnOnce(&'a Database) -> WorkFuture<'a> + Send,
    signal_int: impl Fn() -> SignalFuture + Send,
    signal_term: impl Fn() -> SignalFuture + Send,
) -> ExitStatus {
    let db = tokio::select! {
        opened = Database::open(db_path) => match opened {
            Ok(db) => db,
            Err(e) => return open_error_status(db_path, e),
        },
        _ = signal_int() => return ExitStatus::Signaled(2),
        _ = signal_term() => return ExitStatus::Signaled(15),
    };

    let status = tokio::select! {
        result = work(&db) => result,
        _ = signal_int() => ExitStatus::Signaled(2),
        _ = signal_term() => ExitStatus::Signaled(15),
    };

    match db.close().await {
        Ok(()) => status,
        // 原始错误（如 SQL 错）比 close 失败更相关；数据已由 WAL 兜底。
        // 信号退出保持 Signaled（信号语义主导），close 失败仅 stderr 提示。
        Err(e) => match status {
            ExitStatus::Success => ExitStatus::General(format!("close failed: {}", e)),
            ExitStatus::Signaled(signum) => {
                emit_stderr(&format!("close failed: {}", e));
                ExitStatus::Signaled(signum)
            }
            other => other,
        },
    }
}

async fn run_sql(db: &Database, sql: &str, format: Option<FormatArg>) -> ExitStatus {
    let statements = match parse_stage(sql).await {
        Ok(statements) => statements,
        Err(e) => return ExitStatus::Sql(e),
    };

    // MS10-T04：分片逐条执行。每条语句独立 plan/execute（auto-commit，逐条生效），
    // 结果顺序渲染写 stdout；缓存键用该语句自身的 canonical 文本而非完整串
    // （完整串键会让逐条 SELECT 互相覆盖同一键）。
    let total = statements.len();
    for (index, stmt) in statements.iter().enumerate() {
        let statement_text = stmt.to_string();
        let plan = match plan_stage(db, &statement_text, stmt, false).await {
            Ok(plan) => plan,
            Err(e) => return sql_failure_status(index + 1, total, &e, &statement_text),
        };
        let columns = PlanBuilder::new().get_plan_output_columns(&plan);

        match execute_stage(db, plan, false).await {
            Response::QueryResult { rows } => {
                if let Err(e) =
                    emit_stdout(&render(kind(format), &columns, &QueryPayload::Rows(rows)))
                {
                    return ExitStatus::General(e);
                }
            }
            Response::AffectedRows { count } => {
                if let Err(e) =
                    emit_stdout(&render(kind(format), &[], &QueryPayload::Affected(count)))
                {
                    return ExitStatus::General(e);
                }
            }
            Response::Error { message } => {
                return sql_failure_status(index + 1, total, &message, &statement_text);
            }
            Response::Pong => {}
        }
    }

    ExitStatus::Success
}

/// 逐条执行的失败定位（D3）：语句序号 + 失败语句文本（≤200 字符截断），
/// k > 1 时注明前序语句已生效（逐条 auto-commit 的可观察事实）。
/// parse 错误不套本模板——parse_stage 全串解析、零执行，文本自带行列定位。
fn sql_failure_status(k: usize, n: usize, error: &str, statement_text: &str) -> ExitStatus {
    let mut display = statement_text.to_string();
    if display.chars().count() > 200 {
        display = format!("{}...", display.chars().take(200).collect::<String>());
    }
    let mut message = format!("statement {k} of {n} failed: {error}; statement: {display}");
    if k > 1 {
        message.push_str("; previous statement(s) were committed");
    }
    ExitStatus::Sql(message)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::page_format::ColumnType;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::sync::Notify;

    /// D5-⑤ 执行+close 阶段信号（确定性结构测试）：工作负载 future 先放行
    /// 信号再永久 pending；SIGINT 信号 future 等 `Notify` 许可（由工作负载在
    /// 阶段 2 内存储），故阶段 1 必然 open 胜出、信号确定落在执行阶段。
    /// 断言：close() 已执行（WAL 截断 <1KB）+ 返回 `Signaled(2)`。
    #[tokio::test]
    async fn phase2_signal_runs_close_and_returns_signaled() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("sig.db");
        let wal_path = dir.path().join("sig.wal");

        // 前置：建库写入 >1KB WAL 后 drop 不 close（无 checkpoint）。
        // DDL 无 WAL 记录（design D7 持久化模型）：先 checkpoint 落盘 schema，
        // 再写一批不落盘的数据让 WAL 重新增长，供重开恢复重放。
        {
            let db = Database::open(&db_path).await.unwrap();
            db.create_table("t", vec![("id".to_string(), ColumnType::Int)], "id")
                .await
                .unwrap();
            db.checkpoint().await.unwrap();
            for i in 0..300 {
                match db
                    .execute_sql(&format!("INSERT INTO t VALUES ({})", i))
                    .await
                {
                    Response::AffectedRows { .. } => {}
                    other => panic!("insert {} failed: {:?}", i, other),
                }
            }
            // 冲刷缓冲并停掉后台 flush loop：恢复期间不允许同进程并发写者
            db.wal_buffer.shutdown().await;
            drop(db);
        }
        let wal_before = std::fs::metadata(&wal_path).unwrap().len();
        assert!(
            wal_before >= 1024,
            "precondition: WAL should exceed 1KB, got {wal_before}"
        );

        let release = Arc::new(Notify::new());
        let work_release = release.clone();

        let status = execute_command_inner(
            &db_path,
            move |_db| {
                let work_release = work_release.clone();
                Box::pin(async move {
                    // 已进入阶段 2：放行 SIGINT 信号 future
                    work_release.notify_one();
                    std::future::pending::<ExitStatus>().await
                })
            },
            move || {
                let wait = release.clone();
                Box::pin(async move { wait.notified().await })
            },
            || Box::pin(std::future::pending::<()>()),
        )
        .await;

        let signum = match status {
            ExitStatus::Signaled(signum) => signum,
            ExitStatus::Success => panic!("expected Signaled(2), got Success"),
            ExitStatus::General(m) => panic!("expected Signaled(2), got General: {m}"),
            ExitStatus::Usage(m) => panic!("expected Signaled(2), got Usage: {m}"),
            ExitStatus::Sql(m) => panic!("expected Signaled(2), got Sql: {m}"),
            ExitStatus::Locked(m) => panic!("expected Signaled(2), got Locked: {m}"),
            ExitStatus::InvalidKey(m) => panic!("expected Signaled(2), got InvalidKey: {m}"),
        };
        assert_eq!(signum, 2, "SIGINT must map to signum 2");

        let wal_after = std::fs::metadata(&wal_path).unwrap().len();
        assert!(
            wal_after < 1024,
            "signal during phase 2 must run close() (WAL truncated), got {wal_after} bytes"
        );
    }
}
