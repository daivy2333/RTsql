//! CLI —— one-shot 命令入口：参数解析、名称解析、三阶段执行、渲染、退出码

pub mod lifecycle;
pub mod render;
pub mod resolve;

use crate::database::Database;
use crate::network::protocol::Response;
use crate::parser::planner::{
    classify_attach_statement, classify_transaction_statement, AttachInfo, TxStatementKind,
};
use crate::parser::PlanBuilder;
use crate::pipeline::{
    execute_stage_in_tx_routed, execute_stage_routed, parse_stage, plan_stage_routed, AttachRoute,
    RoutedTables,
};
use crate::storage::StorageError;
use crate::transaction::{IsolationLevel, TransactionSession};
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};
use render::{render, OutputKind, QueryPayload};
use std::collections::HashMap;
use std::future::Future;
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::pin::Pin;
use std::process::ExitCode;
use std::sync::Arc;

/// MS26 T4：`:memory:` 内存库实参（`in-memory-database` R1）——精确串相等，
/// 不做大小写/引号变体归一；主命令特判，生命周期子命令点名拒绝。
const MEMORY_DB: &str = ":memory:";

/// 退出码分类：0 成功 / 1 一般错误 / 2 用法错误 / 3 SQL 错误 / 4 锁冲突 / 5 密钥错误。
///
/// InvalidKey 由加密库的密钥错误面产生（MS17-T01：错误密钥 / 加密无钥 /
/// 明文带钥）。Signaled 携带信号编号，按 POSIX 映射 128+signum（SIGINT→130、
/// SIGTERM→143），不输出 stderr 消息（130/143 自解释）。
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
    /// 加密库密钥；也可经 RTSQL_KEY 环境变量提供（显式 flag 优先）
    #[arg(long, value_name = "KEY", global = true, env = "RTSQL_KEY")]
    key: Option<String>,
    /// MS26 T1：只读打开主库（advisory 共享锁，多读单写）。写语句在计划期
    /// 以 SQLSTATE 25006 拒绝。仅主命令有效——生命周期子命令维持读写打开
    /// （proposal 假设 10）。
    #[arg(long)]
    readonly: bool,
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
    /// 删除数据库及其伴生文件（文件级操作，不校验库内容）
    Delete {
        /// 裸名或含 `/` 的文件路径
        db: String,
        /// 只列出将删除的文件与大小，不执行删除
        #[arg(long)]
        dry_run: bool,
    },
    /// 输出每列统计摘要（行数/null率/distinct/min/max/分位数）
    Stats {
        /// 裸名或含 `/` 的文件路径
        db: String,
        /// 目标表名
        table: String,
    },
    /// 随机抽样 N 行（reservoir sampling）
    Sample {
        /// 裸名或含 `/` 的文件路径
        db: String,
        /// 目标表名
        table: String,
        /// 抽样行数（默认 10）
        n: Option<usize>,
    },
    /// 输出每列画像（类型/min/max/String 列 top-k 高频值）
    Profile {
        /// 裸名或含 `/` 的文件路径
        db: String,
        /// 目标表名
        table: String,
        /// top-k 上限调整（默认 5，上限 20）
        #[arg(long)]
        top: Option<usize>,
    },
    #[command(hide = true)]
    Completions { shell: CompletionShell },
}

#[derive(ValueEnum, Clone, Copy)]
enum CompletionShell {
    Bash,
    Zsh,
    Fish,
}

impl From<CompletionShell> for clap_complete::Shell {
    fn from(shell: CompletionShell) -> Self {
        match shell {
            CompletionShell::Bash => Self::Bash,
            CompletionShell::Zsh => Self::Zsh,
            CompletionShell::Fish => Self::Fish,
        }
    }
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
    // 空密钥防呆（开库前拒绝）：误设空环境变量不得静默降级为无钥打开
    if args.key.as_deref() == Some("") {
        return ExitStatus::Usage("key must not be empty".to_string());
    }
    // MS26 T5/T6（`wal-auto-checkpoint` R2）：非法水位配置在**打开前**点名
    // 拒绝 exit 2（零副作用），与 `RTSQL_KEY` 空值同一纪律与通道。校验只对
    // 开库路径生效——未开库命令（list/new/delete/completions）不消费 WAL
    // 通道，故不施加该校验（Task Contract Forbidden）。
    if let Err(message) = checkpoint_bytes_from_env() {
        return ExitStatus::Usage(message);
    }
    // MS26 T4（`in-memory-database` R3）：六生命周期子命令对 `:memory:` 点名拒绝
    // exit 2——守卫在名称解析与任何文件创建之前（`new` 是唯一会创建父目录的
    // 入口，故必须早于分派）。stats/sample/profile 不在此列：它们沿既有
    // `resolve_existing_db` 的 not-found 拒绝面（Cycle Non-goals）。
    if let Some(target) = args.command.as_ref().and_then(lifecycle_db_target) {
        if target == MEMORY_DB {
            return ExitStatus::Usage(format!(
                "{MEMORY_DB} is only supported by the main command (rtsql {MEMORY_DB} <sql>)"
            ));
        }
    }
    match &args.command {
        // MS26 T1：`--readonly` 只属主命令——生命周期子命令维持读写打开
        // （proposal 假设 10），显式拒绝而非静默忽略。
        Some(_) if args.readonly => ExitStatus::Usage(
            "--readonly is only supported by the main command (rtsql <db> <sql>)".to_string(),
        ),
        Some(Command::New { target }) => lifecycle::new_db(target, args.key.as_deref()).await,
        Some(Command::List) => lifecycle::list(args.format).await,
        Some(Command::Schema { db }) => lifecycle::schema(db, args.key.as_deref()).await,
        Some(Command::Dump { db }) => lifecycle::dump(db, args.key.as_deref()).await,
        Some(Command::Restore { db, file }) => {
            lifecycle::restore(db, file, args.key.as_deref()).await
        }
        Some(Command::Import {
            db,
            table,
            file,
            csv,
        }) => lifecycle::import_csv(db, table, file, *csv, args.format, args.key.as_deref()).await,
        Some(Command::Delete { db, dry_run }) => lifecycle::delete_db(db, *dry_run),
        Some(Command::Stats { db, table }) => {
            lifecycle::stats(db, table, args.format, args.key.as_deref()).await
        }
        Some(Command::Sample { db, table, n }) => {
            lifecycle::sample(db, table, *n, args.format, args.key.as_deref()).await
        }
        Some(Command::Profile { db, table, top }) => {
            lifecycle::profile(db, table, *top, args.format, args.key.as_deref()).await
        }
        Some(Command::Completions { shell }) => {
            let mut command = CliArgs::command();
            let mut stdout = std::io::stdout();
            clap_complete::generate(
                clap_complete::Shell::from(*shell),
                &mut command,
                "rtsql",
                &mut stdout,
            );
            ExitStatus::Success
        }
        None => execute_main_command(args).await,
    }
}

/// WAL 字节水位的环境变量名（MS26 T5/T6，`wal-auto-checkpoint` R2）。
const ENV_WAL_CHECKPOINT_BYTES: &str = "RTSQL_WAL_CHECKPOINT_BYTES";

/// 解析 `RTSQL_WAL_CHECKPOINT_BYTES`（`wal-auto-checkpoint` R2）。
///
/// `Ok(None)` = 未设置，保持 `WalWriter` 的 16 MiB 默认值；`Ok(Some(n))` =
/// 合法字节数（`0` 表示禁用自动 checkpoint）；`Err(msg)` = 非法值，文案点名
/// 变量名与回显非法值，由调用方映射为 `Usage`（exit 2）。
///
/// 解析用 `u64`：非数字、负数与溢出三种非法形态同归此一支。
fn checkpoint_bytes_from_env() -> Result<Option<u64>, String> {
    match std::env::var(ENV_WAL_CHECKPOINT_BYTES) {
        Err(_) => Ok(None),
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(bytes) => Ok(Some(bytes)),
            Err(_) => Err(format!(
                "{ENV_WAL_CHECKPOINT_BYTES} must be a non-negative byte count (0 disables auto checkpoint), got {raw:?}"
            )),
        },
    }
}

/// MS26 T4：六个 db 目标生命周期子命令的库实参（`:memory:` 拒绝面，
/// `in-memory-database` R3）。`list` 无库实参；`stats`/`sample`/`profile`
/// 刻意不在此列（见 `execute_command` 的守卫注释）。
fn lifecycle_db_target(command: &Command) -> Option<&str> {
    match command {
        Command::New { target } => Some(target.as_str()),
        Command::Schema { db }
        | Command::Dump { db }
        | Command::Restore { db, .. }
        | Command::Import { db, .. }
        | Command::Delete { db, .. } => Some(db.as_str()),
        Command::List
        | Command::Stats { .. }
        | Command::Sample { .. }
        | Command::Profile { .. }
        | Command::Completions { .. } => None,
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
    // MS26 T4（`in-memory-database` R1/R3）：`:memory:` 特判在名称解析之前——
    // 该实参不解析为文件、不取文件锁、不产生任何伴生文件；`--key`（无持久化，
    // 加密无意义）与 `--readonly`（只读语义面向文件库的共享锁）组合点名拒绝
    // exit 2。守卫早于 open，故拒绝路径零副作用。
    let memory = db == MEMORY_DB;
    if memory {
        if args.key.is_some() {
            return ExitStatus::Usage(format!(
                "{MEMORY_DB} does not support --key / RTSQL_KEY (an in-memory database is never encrypted)"
            ));
        }
        if args.readonly {
            return ExitStatus::Usage(format!(
                "{MEMORY_DB} does not support --readonly (read-only sessions open a file-backed database)"
            ));
        }
    }
    let db_path = if memory {
        Path::new(MEMORY_DB).to_path_buf()
    } else {
        match resolve::resolve_db_path(db) {
            Ok(path) => path,
            Err(e) => return ExitStatus::General(e),
        }
    };
    // sql/format/key 按值捕获：work future 拥有它们，满足 for<'a> 的 HRTB 工厂约束
    let sql = sql.clone();
    let format = args.format;
    let key = args.key.clone();
    let readonly = args.readonly;
    execute_command_inner_mode(
        &db_path,
        args.key.as_deref(),
        readonly,
        memory,
        move |db| {
            let key = key.clone();
            Box::pin(async move { run_sql(db, &sql, format, key.as_deref()).await })
        },
        sigint_future,
        sigterm_future,
    )
    .await
}

/// 信号 future（SIGINT/SIGTERM）；每个 select 阶段生成新实例。
type SignalFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

/// 阶段 2 的工作负载 future（工厂参数注入，供结构测试替换为确定性源）。
type WorkFuture<'a> = Pin<Box<dyn Future<Output = ExitStatus> + Send + 'a>>;

/// SIGINT future；处理器在**构造期**同步安装，`recv()` 等待信号。
///
/// MS25 T11：注册早于第一次 open 轮询——信号分支的首次轮询原本晚于
/// `Database::open_with_key` 的同步前缀（文件锁获取），该窗口内的 SIGINT 按
/// 默认处置终止进程（130 来自信号默认行为而非映射）。安装失败时保持 pending
/// （默认处置仍在，不伪造信号）。
fn sigint_future() -> SignalFuture {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()) {
        Ok(mut stream) => Box::pin(async move {
            stream.recv().await;
        }),
        Err(_) => Box::pin(std::future::pending::<()>()),
    }
}

/// SIGTERM future；同 `sigint_future` 的构造期安装（安装失败或流关闭保持
/// pending，不伪造信号）。
fn sigterm_future() -> SignalFuture {
    match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(mut stream) => Box::pin(async move {
            stream.recv().await;
        }),
        Err(_) => Box::pin(std::future::pending::<()>()),
    }
}

fn open_error_status(db_path: &Path, e: StorageError) -> ExitStatus {
    match e {
        StorageError::DatabaseLocked(_) => {
            ExitStatus::Locked(format!("database is locked: {}", db_path.display()))
        }
        // detail 保留打开面语境（加密无钥 / 明文带钥 / 页认证失败），
        // 前缀沿用 StorageError Display 语义（exit 5 消息含 "invalid key"）
        StorageError::InvalidKey(detail) => {
            ExitStatus::InvalidKey(format!("invalid key: {detail}"))
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
/// 读写两阶段编排（D4）——既有全部调用方的委托壳（`readonly = false`）。
async fn execute_command_inner(
    db_path: &Path,
    key: Option<&str>,
    work: impl for<'a> FnOnce(&'a Database) -> WorkFuture<'a> + Send,
    signal_int: impl Fn() -> SignalFuture + Send,
    signal_term: impl Fn() -> SignalFuture + Send,
) -> ExitStatus {
    execute_command_inner_mode(db_path, key, false, false, work, signal_int, signal_term).await
}

/// MS26 T1/T4：带打开模式的两阶段编排——`readonly` 为真时经
/// `Database::open_readonly_with_key` 取 advisory 共享锁打开（缺失文件拒绝、
/// WAL 未决拒绝）；`in_memory` 为真时经 `Database::open_in_memory` 打开（无文件、
/// 无锁，`db_path` 只用于错误文案）；否则走既有 `open_with_key`（行为逐字节
/// 不变）。
async fn execute_command_inner_mode(
    db_path: &Path,
    key: Option<&str>,
    readonly: bool,
    in_memory: bool,
    work: impl for<'a> FnOnce(&'a Database) -> WorkFuture<'a> + Send,
    signal_int: impl Fn() -> SignalFuture + Send,
    signal_term: impl Fn() -> SignalFuture + Send,
) -> ExitStatus {
    // MS25 T11：阶段 1 的信号 future 在 select 之前构造——处理器安装发生在
    // 构造期（`signal()` 同步注册），因而早于 `Database::open_with_key` 的第一次
    // 轮询及其同步前缀（文件锁获取）。阶段 2 仍按工厂生成新实例。
    let phase1_sigint = signal_int();
    let phase1_sigterm = signal_term();
    let open = async {
        if in_memory {
            Database::open_in_memory().await
        } else if readonly {
            Database::open_readonly_with_key(db_path, key).await
        } else {
            Database::open_with_key(db_path, IsolationLevel::RepeatableRead, key).await
        }
    };
    let db = tokio::select! {
        opened = open => match opened {
            Ok(db) => db,
            Err(e) => return open_error_status(db_path, e),
        },
        _ = phase1_sigint => return ExitStatus::Signaled(2),
        _ = phase1_sigterm => return ExitStatus::Signaled(15),
    };

    // MS26 T5/T6（`wal-auto-checkpoint` R2）：水位 env 通道的单点应用位——
    // open 成功后、work 执行前，覆盖主命令与全部开库生命周期子命令。只读 /
    // 内存会话无 WAL 通道，设置器为 no-op。合法性已在 `execute_command`
    // 顶部（打开前）判定，此处只剩应用。
    if let Ok(Some(bytes)) = checkpoint_bytes_from_env() {
        db.set_checkpoint_bytes_threshold(bytes);
    }

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

/// MS20 T1（`sql-attach-session`，design D1）：附加库注册表——会话作用域，
/// 不写 catalog、不产生伴生状态文件，随会话进程退出释放。
///
/// 别名（小写，与 `register_table` 键纪律一致）→ 打开的附加库实例。一期只读：
/// 本 Iteration 只建立注册面与锁生命周期，查询路由由 Iteration 001 消费。
/// 刻意不放在 `Database`——attach 是 CLI 会话概念，lib 执行路径零变化。
#[derive(Default)]
struct AttachSession {
    dbs: HashMap<String, Arc<Database>>,
}

impl AttachSession {
    fn contains(&self, alias: &str) -> bool {
        self.dbs.contains_key(alias)
    }

    fn register(&mut self, alias: String, db: Database) {
        self.dbs.insert(alias, Arc::new(db));
    }
}

/// ATTACH 失败分类：文案与退出码语义类分离——文案由调用方统一套多语句上下文
/// （`sql_failure_message`），语义类决定 `ExitStatus` 变体（锁 4 / 密钥 5 /
/// 其余 3）。分类与 `open_error_status` 同型。
enum AttachFailure {
    Sql(String),
    Locked(String),
    InvalidKey(String),
}

/// ATTACH 执行面：路径解析 → 存在性前置 → 独占锁打开 → 注册。任一失败零副作用
/// （注册表不变）。重复别名先于打开判定，避免同别名二次持锁。
///
/// MS26 T2（`readonly-open-mode` R5）：`readonly` 为真时附加库随会话只读打开
/// （共享锁 + WAL 未决谓词，同主库语义）；模式直接取自主库实例的打开模式，
/// 会话内不另存一份状态。
async fn attach_database(
    session: &mut AttachSession,
    info: &AttachInfo,
    key: Option<&str>,
    readonly: bool,
) -> Result<(), AttachFailure> {
    if session.contains(&info.alias) {
        return Err(AttachFailure::Sql(format!(
            "attach alias '{}' is already registered in this session",
            info.alias
        )));
    }
    let path = resolve::resolve_db_path(&info.path).map_err(AttachFailure::Sql)?;
    // `Database::open_with_key` 对不存在的路径会静默建 0 字节新库，故 ATTACH
    // 必须先查——拒绝语义与 `resolve_existing_db` 同型。
    if !path.exists() {
        return Err(AttachFailure::Sql(format!(
            "{} does not exist",
            path.display()
        )));
    }
    // 附加库以主库同源参数打开（RR + 同一密钥通道）：加密面共用全局 --key。
    let db = if readonly {
        Database::open_readonly_with_key(&path, key).await
    } else {
        Database::open_with_key(&path, IsolationLevel::RepeatableRead, key).await
    }
    .map_err(|e| attach_open_failure(&path, e))?;
    session.register(info.alias.clone(), db);
    Ok(())
}

/// 打开失败 → 分类（`open_error_status` 三臂同型；General 归 SQL 失败类）。
fn attach_open_failure(path: &Path, e: StorageError) -> AttachFailure {
    match e {
        StorageError::DatabaseLocked(_) => {
            AttachFailure::Locked(format!("database is locked: {}", path.display()))
        }
        StorageError::InvalidKey(detail) => {
            AttachFailure::InvalidKey(format!("invalid key: {detail}"))
        }
        other => AttachFailure::Sql(format!(
            "failed to open attached database {}: {}",
            path.display(),
            other
        )),
    }
}

async fn run_sql(
    db: &Database,
    sql: &str,
    format: Option<FormatArg>,
    key: Option<&str>,
) -> ExitStatus {
    let statements = match parse_stage(sql).await {
        Ok(statements) => statements,
        Err(e) => return ExitStatus::Sql(e),
    };

    // MS10-T04：分片逐条执行。每条语句独立 plan/execute，结果顺序渲染写
    // stdout；缓存键用该语句自身的 canonical 文本而非完整串（完整串键会让
    // 逐条 SELECT 互相覆盖同一键）。
    // MS11-T02：事务语句（BEGIN/COMMIT/ROLLBACK）驱动本调用的会话事务态；
    // 事务开启后的普通语句经 execute_stage_in_tx 执行（不再逐条 auto-commit），
    // 收尾与失败路径显式回滚会话事务（design D5）。
    let mut session = TransactionSession::new();
    // MS20 T1：附加库注册表与 `TransactionSession` 同生命周期但正交
    // （假设 11——注册面不碰主库数据，活跃事务内亦允许 attach）。
    let mut attach_session = AttachSession::default();

    let total = statements.len();
    for (index, stmt) in statements.iter().enumerate() {
        let statement_text = stmt.to_string();
        // MS20 T1：ATTACH 会话注册臂——不经 `TransactionSession`，与 MS11-T02
        // 三分派互斥（非 ATTACH 语句恒 `Ok(None)`），既有分派逐字节不变。
        match classify_attach_statement(stmt) {
            Ok(Some(info)) => {
                match attach_database(&mut attach_session, &info, key, db.is_readonly()).await {
                    Ok(()) => {
                        // D4 同构：注册语句无行集，渲染 Affected(0)
                        if let Err(e) =
                            emit_stdout(&render(kind(format), &[], &QueryPayload::Affected(0)))
                        {
                            rollback_session(db, &mut session).await;
                            return ExitStatus::General(e);
                        }
                        continue;
                    }
                    Err(failure) => {
                        // 失败路径与既有三分派同型：先回滚活跃会话事务（ATTACH 不改
                        // 主库数据，但保持失败收口单一），再按语义类映射退出码。
                        let in_tx = rollback_session(db, &mut session).await;
                        let k = index + 1;
                        return match failure {
                            AttachFailure::Sql(text) => {
                                sql_failure_status(k, total, &text, &statement_text, in_tx)
                            }
                            AttachFailure::Locked(text) => ExitStatus::Locked(sql_failure_message(
                                k,
                                total,
                                &text,
                                &statement_text,
                                in_tx,
                            )),
                            AttachFailure::InvalidKey(text) => ExitStatus::InvalidKey(
                                sql_failure_message(k, total, &text, &statement_text, in_tx),
                            ),
                        };
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                return sql_failure_status(
                    index + 1,
                    total,
                    &e.to_string(),
                    &statement_text,
                    rollback_session(db, &mut session).await,
                );
            }
        }

        match classify_transaction_statement(stmt) {
            Ok(Some(tx_kind)) => {
                let outcome = match tx_kind {
                    TxStatementKind::Begin => session.begin(db).await,
                    TxStatementKind::Commit => session.commit(db).await,
                    TxStatementKind::Rollback => session.rollback(db).await,
                };
                match outcome {
                    // D4：事务语句无行集，直接渲染 Affected(0)（与 DDL 同构形状）
                    Ok(()) => {
                        if let Err(e) =
                            emit_stdout(&render(kind(format), &[], &QueryPayload::Affected(0)))
                        {
                            rollback_session(db, &mut session).await;
                            return ExitStatus::General(e);
                        }
                    }
                    Err(message) => {
                        return sql_failure_status(
                            index + 1,
                            total,
                            &message,
                            &statement_text,
                            rollback_session(db, &mut session).await,
                        );
                    }
                }
            }
            Ok(None) => {
                // MS20 T2：跨库路由上下文随语句穿线——计划期判定哪些限定名
                // 解析到附加库，执行期据此取库。注册表为空时与既有路径等价。
                let route = AttachRoute::new(attach_session.dbs.clone());
                let mut routed = RoutedTables::new();
                let plan = match plan_stage_routed(
                    db,
                    &statement_text,
                    stmt,
                    false,
                    Some(&route),
                    &mut routed,
                )
                .await
                {
                    Ok(plan) => plan,
                    Err(e) => {
                        return sql_failure_status(
                            index + 1,
                            total,
                            &e.to_string(),
                            &statement_text,
                            rollback_session(db, &mut session).await,
                        );
                    }
                };
                let columns = PlanBuilder::new().get_plan_output_columns(&plan);

                let response = match session.tx_id() {
                    Some(tx_id) => execute_stage_in_tx_routed(db, plan, tx_id, &routed).await,
                    None => execute_stage_routed(db, plan, false, &routed).await,
                };
                match response {
                    Response::QueryResult { rows } => {
                        if let Err(e) =
                            emit_stdout(&render(kind(format), &columns, &QueryPayload::Rows(rows)))
                        {
                            rollback_session(db, &mut session).await;
                            return ExitStatus::General(e);
                        }
                    }
                    Response::AffectedRows { count } => {
                        if let Err(e) =
                            emit_stdout(&render(kind(format), &[], &QueryPayload::Affected(count)))
                        {
                            rollback_session(db, &mut session).await;
                            return ExitStatus::General(e);
                        }
                    }
                    Response::Error { message, .. } => {
                        return sql_failure_status(
                            index + 1,
                            total,
                            &message,
                            &statement_text,
                            rollback_session(db, &mut session).await,
                        );
                    }
                    Response::Pong => {}
                }
            }
            Err(e) => {
                return sql_failure_status(
                    index + 1,
                    total,
                    &e.to_string(),
                    &statement_text,
                    rollback_session(db, &mut session).await,
                );
            }
        }
    }

    // D5：调用收尾不允许残留未提交会话事务——显式回滚 + stderr 提示，退出码仍为 0。
    if session.is_active() {
        match session.rollback(db).await {
            Ok(()) => emit_stderr("uncommitted transaction was rolled back at exit"),
            Err(e) => emit_stderr(&format!("uncommitted transaction rollback failed: {}", e)),
        }
    }
    // MS20 T1（design D1）：附加库 best-effort close——一期只读无脏页，MS18 零工作
    // 守卫使 close 无操作；结果忽略（进程退出时 flock 随 fd 释放兜底）。fail-fast
    // 中途退出走同一兜底，无需在此展开。
    for attached in attach_session.dbs.values() {
        let _ = attached.close().await;
    }
    ExitStatus::Success
}

/// D5 错误路径收尾：会话事务仍开启时先显式回滚再返回（回滚自身失败仅
/// stderr 记录，不掩盖原始错误）；返回是否处于事务上下文，供
/// `sql_failure_status` 区分前序语句生效状态后缀。
async fn rollback_session(db: &Database, session: &mut TransactionSession) -> bool {
    if !session.is_active() {
        return false;
    }
    if let Err(e) = session.rollback(db).await {
        emit_stderr(&format!("session rollback failed: {}", e));
    }
    true
}

/// 逐条执行的失败定位（D3）：语句序号 + 失败语句文本（≤200 字符截断），
/// k > 1 时注明前序语句生效状态——auto-commit 上下文为已生效（已提交）；
/// 会话事务上下文（`in_transaction`，返回前已显式回滚）为未提交并已随
/// 事务回滚。parse 错误不套本模板——parse_stage 全串解析、零执行，文本
/// 自带行列定位。
fn sql_failure_status(
    k: usize,
    n: usize,
    error: &str,
    statement_text: &str,
    in_transaction: bool,
) -> ExitStatus {
    ExitStatus::Sql(sql_failure_message(
        k,
        n,
        error,
        statement_text,
        in_transaction,
    ))
}

/// 逐条执行的失败文案（`sql_failure_status` 与 ATTACH 的锁/密钥失败类共用同一
/// 构造点——退出码语义不同，消息形状逐字节一致）。
fn sql_failure_message(
    k: usize,
    n: usize,
    error: &str,
    statement_text: &str,
    in_transaction: bool,
) -> String {
    let mut display = statement_text.to_string();
    if display.chars().count() > 200 {
        display = format!("{}...", display.chars().take(200).collect::<String>());
    }
    let mut message = format!("statement {k} of {n} failed: {error}; statement: {display}");
    if k > 1 {
        if in_transaction {
            message.push_str(
                "; previous statement(s) were not committed (rolled back with the transaction)",
            );
        } else {
            message.push_str("; previous statement(s) were committed");
        }
    }
    message
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
            db.wal_buffer.as_ref().unwrap().shutdown().await;
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
            None,
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
