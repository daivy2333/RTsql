//! MS44：统一分级诊断门面（Iteration 000 / T1）与语句阶段接入
//!（Iteration 001 / T2）。
//!
//! 三项配置分离：`Detail`（输出详细程度）、`ModuleSet`（观察类别）、
//! `Collect`（采集内容：事件 / 汇总 / 追踪）。耗时口径统一为包含耗时：
//! 语句总耗时 = 根阶段包含耗时，阶段独占耗时 = 自身包含耗时 − 直接子
//! 阶段包含耗时之和。语句管道经任务内阶段绑定（`with_statement_stages`）
//! 把 `parse` / `plan` / `execute` / `commit` 阶段挂进根作用域；本模块
//! 不依赖 database、pipeline、wal、storage 各层。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::task_local;

/// SQL 文本在摘要中的保留长度（字符数）。
const SQL_TEXT_LIMIT: usize = 80;

// ---------- 配置 ----------

/// 输出详细程度：只作用于渲染，不改变采集。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Detail {
    /// 关闭渲染：摘要仍可从会话读回，渲染结果为空字符串。
    Off,
    /// 只渲染汇总数。
    #[default]
    Summary,
    /// 汇总数之外追加逐条事件行。
    Full,
}

/// 观察类别（子系统）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Module {
    Pipeline,
    Wal,
    Storage,
    Index,
}

/// 观察类别集合：`All`，或只含列出的子系统。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ModuleSet {
    #[default]
    All,
    Only(Vec<Module>),
}

impl ModuleSet {
    pub fn contains(&self, module: Module) -> bool {
        match self {
            ModuleSet::All => true,
            ModuleSet::Only(list) => list.contains(&module),
        }
    }
}

/// 采集内容开关。默认全关：采集全关时不产生摘要。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Collect {
    pub events: bool,
    pub stats: bool,
    pub trace: bool,
}

impl Collect {
    fn all_off(&self) -> bool {
        !self.events && !self.stats && !self.trace
    }
}

/// 一次会话的完整诊断配置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticsConfig {
    pub detail: Detail,
    pub modules: ModuleSet,
    pub collect: Collect,
    /// 单条摘要内事件保留上限：超出计入摘要的 `dropped`，保留最新。
    pub event_limit: usize,
    /// 会话内摘要保留条数上限：超出丢弃最旧。
    pub summary_limit: usize,
}

impl Default for DiagnosticsConfig {
    fn default() -> Self {
        Self {
            detail: Detail::Summary,
            modules: ModuleSet::All,
            collect: Collect::default(),
            event_limit: 64,
            summary_limit: 32,
        }
    }
}

// ---------- 事件与阶段 ----------

/// 事件严重级别。语句结束状态映射：成功 Info、失败 Error、未结束被丢弃 Warn。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warn,
    Error,
}

/// 事件观察类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Compute,
    Lock,
    Queue,
    Io,
    Schedule,
}

/// 公开事件记录。只有严重级别、类别、消息、数据四个字段——没有操作标识、
/// 运行标识或内容哈希字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub severity: Severity,
    pub category: Category,
    pub message: String,
    pub data: Option<String>,
}

/// 语句结束状态（输入口径；摘要内以 [`Severity`] 呈现）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeEnd {
    Success,
    Failed,
    Dropped,
}

fn severity_for(end: ScopeEnd) -> Severity {
    match end {
        ScopeEnd::Success => Severity::Info,
        ScopeEnd::Failed => Severity::Error,
        ScopeEnd::Dropped => Severity::Warn,
    }
}

/// 已经量好的阶段节点（包含耗时口径）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageSample {
    pub module: Module,
    pub name: String,
    /// 本阶段包含耗时（含全部子阶段）。
    pub inclusive: Duration,
    pub children: Vec<StageSample>,
}

impl StageSample {
    /// 直接子阶段包含耗时之和。
    fn children_inclusive(&self) -> Duration {
        self.children.iter().map(|child| child.inclusive).sum()
    }

    /// 独占耗时：包含耗时 − 直接子阶段包含耗时之和；子阶段之和超出时饱和为 0。
    fn exclusive(&self) -> Duration {
        self.inclusive.saturating_sub(self.children_inclusive())
    }
}

// ---------- 摘要 ----------

/// 单个阶段的汇总数（stats 口径）：按（类别，阶段名）聚合出现次数与耗时。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageStat {
    pub module: Module,
    pub name: String,
    pub count: u64,
    pub inclusive_total: Duration,
    pub exclusive_total: Duration,
}

/// 一条逐条追踪样本（trace 口径）：阶段单次出现的包含 / 独占耗时。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceSample {
    pub module: Module,
    pub name: String,
    pub inclusive: Duration,
    pub exclusive: Duration,
}

/// 一条语句的诊断摘要。没有操作标识、运行标识或内容哈希字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    /// 截断到 80 个字符的 SQL 文本。
    pub sql: String,
    pub severity: Severity,
    /// 语句总耗时（根阶段包含耗时）。
    pub total: Duration,
    /// 阶段汇总数；`collect.stats` 为假时为空。
    pub stages: Vec<StageStat>,
    /// 逐条追踪样本；`collect.trace` 为假时为空。
    pub samples: Vec<TraceSample>,
    /// 逐条事件；`collect.events` 为假时为空。
    pub events: Vec<Record>,
    /// 超出 `event_limit` 丢弃的事件数。
    pub dropped: u64,
}

/// 渲染一条摘要：`Off` 为空字符串，`Summary` 只有汇总数，`Full` 追加事件行；
/// 非空结果以 `diagnostics ` 开头。只做字符串拼装，不写 stderr。
pub fn render(summary: &Summary, detail: Detail) -> String {
    if detail == Detail::Off {
        return String::new();
    }
    let mut out = format!(
        "diagnostics sql={} severity={:?} total={:?}",
        summary.sql, summary.severity, summary.total
    );
    for stat in &summary.stages {
        out.push_str(&format!(
            "\n  stage {} module={:?} count={} inclusive={:?} exclusive={:?}",
            stat.name, stat.module, stat.count, stat.inclusive_total, stat.exclusive_total
        ));
    }
    if detail == Detail::Full {
        for event in &summary.events {
            out.push_str(&format!(
                "\n  event {:?}/{:?} {}",
                event.severity, event.category, event.message
            ));
        }
    }
    out
}

// ---------- 会话 ----------

struct SessionState {
    config: DiagnosticsConfig,
    summaries: Vec<Summary>,
}

/// 会话诊断门面。`Database` 以 `Arc` 持有并随 `Clone` 共享。
pub struct Diagnostics {
    state: Mutex<SessionState>,
}

impl Diagnostics {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(SessionState {
                config: DiagnosticsConfig::default(),
                summaries: Vec::new(),
            }),
        }
    }

    /// 下发新配置，只影响之后形成的摘要。
    pub fn configure(&self, config: DiagnosticsConfig) {
        self.state.lock().unwrap().config = config;
    }

    /// 已记录的摘要快照（最新在末尾）。
    pub fn summaries(&self) -> Vec<Summary> {
        self.state.lock().unwrap().summaries.clone()
    }

    /// 会话采集是否打开（events / stats / trace 任一开）——语句入口的绑定
    /// 判定；采集全关时整条语句零诊断成本。
    pub(crate) fn collecting(&self) -> bool {
        !self.state.lock().unwrap().config.collect.all_off()
    }

    /// 按当前会话 `Detail` 渲染一条摘要（语句 Result 确定后的渲染出口消费）。
    pub(crate) fn render_with_session_detail(&self, summary: &Summary) -> String {
        let detail = self.state.lock().unwrap().config.detail;
        render(summary, detail)
    }

    /// 开一条语句作用域：结束时以真实经过时间构造根阶段，走
    /// [`Diagnostics::record_pre_measured`] 背后同一套计算。正常结束用
    /// [`StatementScope::end_ok`] / [`StatementScope::end_err`]；未结束就
    /// 析构按 `Dropped` 记录。
    pub fn statement(self: &Arc<Self>, sql: &str) -> StatementScope {
        StatementScope {
            session: Arc::clone(self),
            inner: Some(ScopeInner {
                sql: sql.to_string(),
                start: Instant::now(),
            }),
            stages: None,
        }
    }

    /// 语句管道的根作用域入口（MS44 Iteration 001 / T2）：当前任务存在
    /// 阶段绑定时开根并接上阶段树；无绑定（采集全关或非语句任务）返回
    /// `None`——默认配置零成本。
    pub(crate) fn enter_statement(self: &Arc<Self>, sql: &str) -> Option<StatementScope> {
        let stages = STATEMENT_STAGES.try_with(|stack| Arc::clone(stack)).ok();
        stages.map(|stages| StatementScope {
            session: Arc::clone(self),
            inner: Some(ScopeInner {
                sql: sql.to_string(),
                start: Instant::now(),
            }),
            stages: Some(stages),
        })
    }

    /// 公开函数：把一组已经量好的包含耗时立即记成一条摘要。
    ///
    /// `root` 是语句根阶段：包含耗时即语句总耗时，只有 `children` 参与阶段
    /// 汇总与追踪（根自身不占阶段名、不受类别过滤）。`events` 按当前
    /// `event_limit` 截断，超出计入 `dropped`。
    pub fn record_pre_measured(
        &self,
        sql: &str,
        end: ScopeEnd,
        root: StageSample,
        events: Vec<Record>,
    ) {
        self.record_statement(sql, end, root, events);
    }

    /// 唯一的摘要形成路径：语句作用域结束与预量入口共用同一套计算。返回
    /// 本次形成的摘要（采集全关时不形成，返回 `None`）。
    fn record_statement(
        &self,
        sql: &str,
        end: ScopeEnd,
        root: StageSample,
        mut events: Vec<Record>,
    ) -> Option<Summary> {
        let mut state = self.state.lock().unwrap();
        let config = state.config.clone();

        // 采集全关：不分配摘要。
        if config.collect.all_off() {
            return None;
        }

        let mut dropped = 0;
        if events.len() > config.event_limit {
            let overflow = events.len() - config.event_limit;
            dropped = overflow as u64;
            events.drain(0..overflow);
        }

        let mut stages = Vec::new();
        if config.collect.stats {
            collect_stats(&root.children, &config.modules, &mut stages);
        }
        let mut samples = Vec::new();
        if config.collect.trace {
            collect_samples(&root.children, &config.modules, &mut samples);
        }
        let events = if config.collect.events {
            events
        } else {
            Vec::new()
        };

        let summary = Summary {
            sql: sql.chars().take(SQL_TEXT_LIMIT).collect(),
            severity: severity_for(end),
            total: root.inclusive,
            stages,
            samples,
            events,
            dropped,
        };
        state.summaries.push(summary.clone());

        // 摘要上限：只留最新的。
        if state.summaries.len() > config.summary_limit {
            let overflow = state.summaries.len() - config.summary_limit;
            state.summaries.drain(0..overflow);
        }
        Some(summary)
    }
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self::new()
    }
}

/// 深度优先聚合计入 `out`。被观察类别排除的阶段自身不记录，其子阶段仍按
/// 各自类别独立判定。
fn collect_stats(nodes: &[StageSample], modules: &ModuleSet, out: &mut Vec<StageStat>) {
    for node in nodes {
        if modules.contains(node.module) {
            let exclusive = node.exclusive();
            if let Some(stat) = out
                .iter_mut()
                .find(|stat| stat.module == node.module && stat.name == node.name)
            {
                stat.count += 1;
                stat.inclusive_total += node.inclusive;
                stat.exclusive_total += exclusive;
            } else {
                out.push(StageStat {
                    module: node.module,
                    name: node.name.clone(),
                    count: 1,
                    inclusive_total: node.inclusive,
                    exclusive_total: exclusive,
                });
            }
        }
        collect_stats(&node.children, modules, out);
    }
}

/// 逐条样本，遍历口径与 [`collect_stats`] 相同。
fn collect_samples(nodes: &[StageSample], modules: &ModuleSet, out: &mut Vec<TraceSample>) {
    for node in nodes {
        if modules.contains(node.module) {
            out.push(TraceSample {
                module: node.module,
                name: node.name.clone(),
                inclusive: node.inclusive,
                exclusive: node.exclusive(),
            });
        }
        collect_samples(&node.children, modules, out);
    }
}

// ---------- 任务内阶段绑定（MS44 Iteration 001 / T2）----------

task_local! {
    /// 当前语句的阶段树（`pipeline::execute_with_params` 的绑定点建立，
    /// `profiling.rs` 的 `PROFILING_DATA` 同款机制）。采集全关的语句不建立
    /// 绑定——默认配置零成本。
    static STATEMENT_STAGES: Arc<Mutex<StageStack>>;
}

/// 语句阶段树：已完结阶段按父子挂接；open 栈承载当前嵌套阶段（隐式 DML
/// 提交期间最内 open 阶段是 `execute`，`Database::commit` 把 `commit` 挂其下）。
#[derive(Default)]
struct StageStack {
    /// 根的已完成子阶段（无 open 阶段时新完结阶段挂到这里）。
    root_children: Vec<StageSample>,
    /// open 阶段栈（最内在尾）。阶段耗时由调用方的单次测量在 close 时给出，
    /// 栈内不持有计时起点。
    open: Vec<OpenStage>,
}

struct OpenStage {
    module: Module,
    name: String,
    children: Vec<StageSample>,
}

impl StageStack {
    /// 挂一个已完结阶段到当前最内 open 阶段（无 open 时挂到根）。
    fn push_completed(&mut self, module: Module, name: &str, inclusive: Duration) {
        let sample = StageSample {
            module,
            name: name.to_string(),
            inclusive,
            children: Vec::new(),
        };
        match self.open.last_mut() {
            Some(innermost) => innermost.children.push(sample),
            None => self.root_children.push(sample),
        }
    }

    fn open_stage(&mut self, module: Module, name: &str) {
        self.open.push(OpenStage {
            module,
            name: name.to_string(),
            children: Vec::new(),
        });
    }

    /// 关最内 open 阶段，以测得的包含耗时挂到父级（无 open 时防御性 no-op）。
    fn close_stage(&mut self, name: &str, inclusive: Duration) {
        let Some(open) = self.open.pop() else {
            return;
        };
        debug_assert_eq!(
            open.name, name,
            "close_stage must match the innermost open stage"
        );
        let sample = StageSample {
            module: open.module,
            name: open.name,
            inclusive,
            children: open.children,
        };
        match self.open.last_mut() {
            Some(parent) => parent.children.push(sample),
            None => self.root_children.push(sample),
        }
    }

    /// 根作用域结束：取走根的已完成子阶段；仍 open 的阶段没有耗时，弃置。
    fn finish(&mut self) -> Vec<StageSample> {
        self.open.clear();
        std::mem::take(&mut self.root_children)
    }
}

/// 为当前任务建立语句阶段绑定（`execute_with_params` 的绑定点）。
pub(crate) fn with_statement_stages<F>(f: F) -> impl std::future::Future<Output = F::Output>
where
    F: std::future::Future,
{
    STATEMENT_STAGES.scope(Arc::new(Mutex::new(StageStack::default())), f)
}

/// 阶段计时起点：绑定存在时 `Some`（`Database::commit` 等深层代码在绑定外
/// 得到 `None`，计时零成本）。
pub(crate) fn stage_instant() -> Option<Instant> {
    STATEMENT_STAGES.try_with(|_| Instant::now()).ok()
}

/// 把一个已完结阶段挂到当前最内 open 阶段（无 open 时挂到根）。无绑定的
/// 任务（显式 COMMIT 的 session/CLI 调用、checkpoint 触发点）为 no-op。
pub(crate) fn record_stage(module: Module, name: &str, inclusive: Duration) {
    let _ = STATEMENT_STAGES.try_with(|stack| {
        stack
            .lock()
            .unwrap()
            .push_completed(module, name, inclusive);
    });
}

/// 开一个 open 阶段（`execute`）：耗时由调用方在同一次测量后经
/// `close_stage` 给出——每个阶段只测一次。无绑定的任务为 no-op。
pub(crate) fn open_stage(module: Module, name: &str) {
    let _ = STATEMENT_STAGES.try_with(|stack| {
        stack.lock().unwrap().open_stage(module, name);
    });
}

/// 关最内 open 阶段并以测得的包含耗时挂到父级。无绑定的任务为 no-op。
pub(crate) fn close_stage(name: &str, inclusive: Duration) {
    let _ = STATEMENT_STAGES.try_with(|stack| {
        stack.lock().unwrap().close_stage(name, inclusive);
    });
}

// ---------- 语句作用域 ----------

struct ScopeInner {
    sql: String,
    start: Instant,
}

/// 语句作用域：结束时走 [`Diagnostics`] 的同一套计算形成摘要。
pub struct StatementScope {
    session: Arc<Diagnostics>,
    inner: Option<ScopeInner>,
    /// 任务内阶段树的句柄（语句管道经 [`Diagnostics::enter_statement`] 接上；
    /// 直接经 [`Diagnostics::statement`] 开的作用域无阶段）。作用域析构在
    /// 绑定作用域之外也能发生（语句 future 被放弃时），故持有句柄而不是回读
    /// task-local。
    stages: Option<Arc<Mutex<StageStack>>>,
}

impl StatementScope {
    /// 成功结束：摘要严重级别 Info。返回本次形成的摘要（采集全关时 `None`）。
    pub fn end_ok(self) -> Option<Summary> {
        self.finish(ScopeEnd::Success)
    }

    /// 失败结束：摘要严重级别 Error。返回本次形成的摘要（采集全关时 `None`）。
    pub fn end_err(self) -> Option<Summary> {
        self.finish(ScopeEnd::Failed)
    }

    fn finish(mut self, end: ScopeEnd) -> Option<Summary> {
        match self.inner.take() {
            Some(inner) => Self::record(&self.session, inner, end, self.stages.take()),
            None => None,
        }
    }

    fn record(
        session: &Diagnostics,
        inner: ScopeInner,
        end: ScopeEnd,
        stages: Option<Arc<Mutex<StageStack>>>,
    ) -> Option<Summary> {
        let children = match stages {
            Some(stack) => stack.lock().unwrap().finish(),
            None => Vec::new(),
        };
        let root = StageSample {
            // 语句根是容器：不参与类别过滤，也不占阶段名。
            module: Module::Pipeline,
            name: "statement".to_string(),
            inclusive: inner.start.elapsed(),
            children,
        };
        session.record_statement(&inner.sql, end, root, vec![])
    }
}

impl Drop for StatementScope {
    fn drop(&mut self) {
        // 未结束就析构：按 Dropped 记录（严重级别 Warn）。
        if let Some(inner) = self.inner.take() {
            Self::record(&self.session, inner, ScopeEnd::Dropped, self.stages.take());
        }
    }
}
