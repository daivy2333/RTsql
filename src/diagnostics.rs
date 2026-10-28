//! MS44 Iteration 000（T1）：统一分级诊断门面。
//!
//! 三项配置分离：`Detail`（输出详细程度）、`ModuleSet`（观察类别）、
//! `Collect`（采集内容：事件 / 汇总 / 追踪）。耗时口径统一为包含耗时：
//! 语句总耗时 = 根阶段包含耗时，阶段独占耗时 = 自身包含耗时 − 直接子
//! 阶段包含耗时之和。本轮只交付门面与预量入口，不接语句管道；本模块
//! 不依赖 database、pipeline、wal、storage 各层。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
        }
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

    /// 唯一的摘要形成路径：语句作用域结束与预量入口共用同一套计算。
    fn record_statement(
        &self,
        sql: &str,
        end: ScopeEnd,
        root: StageSample,
        mut events: Vec<Record>,
    ) {
        let mut state = self.state.lock().unwrap();
        let config = state.config.clone();

        // 采集全关：不分配摘要。
        if config.collect.all_off() {
            return;
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

        state.summaries.push(Summary {
            sql: sql.chars().take(SQL_TEXT_LIMIT).collect(),
            severity: severity_for(end),
            total: root.inclusive,
            stages,
            samples,
            events,
            dropped,
        });

        // 摘要上限：只留最新的。
        if state.summaries.len() > config.summary_limit {
            let overflow = state.summaries.len() - config.summary_limit;
            state.summaries.drain(0..overflow);
        }
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

// ---------- 语句作用域 ----------

struct ScopeInner {
    sql: String,
    start: Instant,
}

/// 语句作用域：结束时走 [`Diagnostics`] 的同一套计算形成摘要。
pub struct StatementScope {
    session: Arc<Diagnostics>,
    inner: Option<ScopeInner>,
}

impl StatementScope {
    /// 成功结束：摘要严重级别 Info。
    pub fn end_ok(self) {
        self.finish(ScopeEnd::Success);
    }

    /// 失败结束：摘要严重级别 Error。
    pub fn end_err(self) {
        self.finish(ScopeEnd::Failed);
    }

    fn finish(mut self, end: ScopeEnd) {
        if let Some(inner) = self.inner.take() {
            Self::record(&self.session, inner, end);
        }
    }

    fn record(session: &Diagnostics, inner: ScopeInner, end: ScopeEnd) {
        let root = StageSample {
            // 语句根是容器：不参与类别过滤，也不占阶段名。
            module: Module::Pipeline,
            name: "statement".to_string(),
            inclusive: inner.start.elapsed(),
            children: vec![],
        };
        session.record_statement(&inner.sql, end, root, vec![]);
    }
}

impl Drop for StatementScope {
    fn drop(&mut self) {
        // 未结束就析构：按 Dropped 记录（严重级别 Warn）。
        if let Some(inner) = self.inner.take() {
            Self::record(&self.session, inner, ScopeEnd::Dropped);
        }
    }
}
