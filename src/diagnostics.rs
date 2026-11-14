//! MS44：统一分级诊断门面（Iteration 000 / T1）、语句阶段接入
//!（Iteration 001 / T2）与等待归属（Iteration 002 / T3）。
//!
//! 三项配置分离：`Detail`（输出详细程度）、`ModuleSet`（观察类别）、
//! `Collect`（采集内容：事件 / 汇总 / 追踪）。耗时口径统一为包含耗时：
//! 语句总耗时 = 根阶段包含耗时，阶段独占耗时 = 自身包含耗时 − 直接子
//! 阶段包含耗时之和。语句管道经任务内阶段绑定（`with_statement_stages`）
//! 把 `parse` / `plan` / `execute` / `commit` 阶段挂进根作用域；本模块
//! 不依赖 database、pipeline、wal、storage 各层。
//!
//! 等待（Iteration 002）：等待类别复用 [`Category`]（Compute / Lock /
//! Queue / Io / Schedule）。等待归属「当时最内层 open 阶段」——阶段的
//! 等待累计存放在跨任务共享的等待槽里，刷盘任务只回写 Io 与 Schedule，
//! 不解释语义。

use std::collections::HashMap;
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

/// 事件观察类别。等待类别复用同一集合（Compute / Lock / Queue / Io /
/// Schedule），不引入第二套类别名。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Compute,
    Lock,
    Queue,
    Io,
    Schedule,
}

impl Category {
    /// 类别在等待累计里的稳定索引。
    fn index(self) -> usize {
        match self {
            Category::Compute => 0,
            Category::Lock => 1,
            Category::Queue => 2,
            Category::Io => 3,
            Category::Schedule => 4,
        }
    }
}

/// 等待类别累计：按 [`Category`] 索引的耗时与文件页数。
///
/// 四类等待（Lock / Queue / Io / Schedule）是阶段自身被包住的等待；
/// `Compute` 是阶段独占耗时减去自身等待之和，在阶段闭合时算出——内层
/// 等待不重复计入外层阶段（外层独占耗时已扣除子阶段包含耗时）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WaitTotals {
    /// 五类耗时的累计（索引与 [`Category`] 一致）。
    by_category: [Duration; 5],
    /// 读取的文件页数（Io 类别）。
    pages: u64,
}

impl WaitTotals {
    /// 某类别的累计耗时。
    pub fn get(&self, category: Category) -> Duration {
        self.by_category[category.index()]
    }

    /// 累计读取的文件页数（Io 类别）。
    pub fn pages(&self) -> u64 {
        self.pages
    }

    /// 四类等待的耗时之和（不含 Compute）。
    pub fn wait_total(&self) -> Duration {
        self.get(Category::Lock)
            + self.get(Category::Queue)
            + self.get(Category::Io)
            + self.get(Category::Schedule)
    }

    fn add(&mut self, category: Category, duration: Duration) {
        self.by_category[category.index()] += duration;
    }

    fn add_pages(&mut self, pages: u64) {
        self.pages += pages;
    }

    /// 累计另一组等待（同类别相加，页数相加）。
    fn merge(&mut self, other: &WaitTotals) {
        for index in 0..self.by_category.len() {
            self.by_category[index] += other.by_category[index];
        }
        self.pages += other.pages;
    }
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
///
/// 公开形状保持 Iteration 000 的四字段：等待累计只经 [`StageStat`] /
/// [`TraceSample`] 暴露，不进入本类型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageSample {
    pub module: Module,
    pub name: String,
    /// 本阶段包含耗时（含全部子阶段）。
    pub inclusive: Duration,
    pub children: Vec<StageSample>,
}

/// 内部阶段节点：公开的 [`StageSample`] 结构加该阶段的等待累计。
#[derive(Debug, Clone)]
struct StageNode {
    module: Module,
    name: String,
    /// 本阶段包含耗时（含全部子阶段）。
    inclusive: Duration,
    children: Vec<StageNode>,
    /// 本阶段自身的等待累计。
    waits: WaitTotals,
}

impl StageNode {
    /// 由公开的 [`StageSample`] 构造（公开形状没有等待字段，等待为零）。
    fn from_public(sample: StageSample) -> StageNode {
        StageNode {
            module: sample.module,
            name: sample.name,
            inclusive: sample.inclusive,
            children: sample
                .children
                .into_iter()
                .map(StageNode::from_public)
                .collect(),
            waits: WaitTotals::default(),
        }
    }

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
    /// 该阶段的等待类别累计（多次出现时按类别相加）。
    pub waits: WaitTotals,
}

/// 一条逐条追踪样本（trace 口径）：阶段单次出现的包含 / 独占耗时。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceSample {
    pub module: Module,
    pub name: String,
    pub inclusive: Duration,
    pub exclusive: Duration,
    /// 该次出现的等待类别累计。
    pub waits: WaitTotals,
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
    /// 语句开始时刻。
    pub started_at: Instant,
    /// 首行等待。没有交回行时为 `None`。
    pub first_row: Option<Duration>,
    /// 交回给调用方的结果行数。
    pub rows_delivered: u64,
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

/// 从一批保留摘要读出的比较指标。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentMetrics {
    /// 交回行数之和除以墙钟，单位是行每秒。没有摘要或墙钟为 0 时为 `None`。
    pub throughput_rows_per_sec: Option<f64>,
    /// 总耗时升序后的最近秩上界。没有摘要时为 `None`。
    pub slow_p95: Option<Duration>,
}

/// 用传入摘要计算吞吐和 P95。不读会话，也不保留第二套存储。
pub fn segment_metrics(summaries: &[Summary]) -> SegmentMetrics {
    if summaries.is_empty() {
        return SegmentMetrics {
            throughput_rows_per_sec: None,
            slow_p95: None,
        };
    }

    let mut t0 = summaries[0].started_at;
    let mut t1 = summaries[0].started_at + summaries[0].total;
    let mut rows_delivered = summaries[0].rows_delivered;
    for summary in &summaries[1..] {
        t0 = t0.min(summary.started_at);
        let ended = summary.started_at + summary.total;
        t1 = t1.max(ended);
        rows_delivered += summary.rows_delivered;
    }
    let wall = t1.saturating_duration_since(t0);
    let throughput_rows_per_sec = if wall.is_zero() {
        None
    } else {
        Some(rows_delivered as f64 / wall.as_secs_f64())
    };

    let mut totals: Vec<Duration> = summaries.iter().map(|summary| summary.total).collect();
    totals.sort();
    let rank = (95 * totals.len() + 99) / 100;
    let slow_p95 = totals.get(rank - 1).copied();

    SegmentMetrics {
        throughput_rows_per_sec,
        slow_p95,
    }
}

/// 进程驻留峰值。Linux 上是 `ru_maxrss * 1024`；调用失败、值为负或其他操作系统为 `None`。
pub fn process_memory_peak_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        linux_ru_maxrss_bytes()
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
fn linux_ru_maxrss_bytes() -> Option<u64> {
    use std::os::raw::{c_int, c_long};

    #[repr(C)]
    #[allow(dead_code)]
    struct Timeval {
        tv_sec: c_long,
        tv_usec: c_long,
    }

    #[repr(C)]
    #[allow(dead_code)]
    struct Rusage {
        ru_utime: Timeval,
        ru_stime: Timeval,
        ru_maxrss: c_long,
        ru_ixrss: c_long,
        ru_idrss: c_long,
        ru_isrss: c_long,
        ru_minflt: c_long,
        ru_majflt: c_long,
        ru_nswap: c_long,
        ru_inblock: c_long,
        ru_oublock: c_long,
        ru_msgsnd: c_long,
        ru_msgrcv: c_long,
        ru_nsignals: c_long,
        ru_nvcsw: c_long,
        ru_nivcsw: c_long,
    }

    unsafe extern "C" {
        fn getrusage(who: c_int, usage: *mut Rusage) -> c_int;
    }

    const RUSAGE_SELF: c_int = 0;
    let mut usage = unsafe { std::mem::zeroed::<Rusage>() };
    let rc = unsafe { getrusage(RUSAGE_SELF, &mut usage) };
    if rc != 0 || usage.ru_maxrss < 0 {
        return None;
    }
    Some((usage.ru_maxrss as u64) * 1024)
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
        let start = Instant::now();
        if let Some(stages) = &stages {
            stages.lock().unwrap().started_at = Some(start);
        }
        stages.map(|stages| StatementScope {
            session: Arc::clone(self),
            inner: Some(ScopeInner {
                sql: sql.to_string(),
                start,
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
        let recorded_at = Instant::now();
        let started_at = recorded_at
            .checked_sub(root.inclusive)
            .unwrap_or(recorded_at);
        self.record_statement(
            sql,
            end,
            StageNode::from_public(root),
            events,
            started_at,
            None,
            0,
        );
    }

    /// 唯一的摘要形成路径：语句作用域结束与预量入口共用同一套计算。返回
    /// 本次形成的摘要（采集全关时不形成，返回 `None`）。
    fn record_statement(
        &self,
        sql: &str,
        end: ScopeEnd,
        root: StageNode,
        mut events: Vec<Record>,
        started_at: Instant,
        first_row: Option<Duration>,
        rows_delivered: u64,
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
            started_at,
            first_row,
            rows_delivered,
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
fn collect_stats(nodes: &[StageNode], modules: &ModuleSet, out: &mut Vec<StageStat>) {
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
                stat.waits.merge(&node.waits);
            } else {
                out.push(StageStat {
                    module: node.module,
                    name: node.name.clone(),
                    count: 1,
                    inclusive_total: node.inclusive,
                    exclusive_total: exclusive,
                    waits: node.waits,
                });
            }
        }
        collect_stats(&node.children, modules, out);
    }
}

/// 逐条样本，遍历口径与 [`collect_stats`] 相同。
fn collect_samples(nodes: &[StageNode], modules: &ModuleSet, out: &mut Vec<TraceSample>) {
    for node in nodes {
        if modules.contains(node.module) {
            out.push(TraceSample {
                module: node.module,
                name: node.name.clone(),
                inclusive: node.inclusive,
                exclusive: node.exclusive(),
                waits: node.waits,
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
    root_children: Vec<StageNode>,
    /// open 阶段栈（最内在尾）。阶段耗时由调用方的单次测量在 close 时给出，
    /// 栈内不持有计时起点。
    open: Vec<OpenStage>,
    /// 本语句的等待槽（跨任务共享；刷盘任务只回写 Io 与 Schedule）。
    waits: WaitSlot,
    /// 下一个 open 阶段的 id（0 保留给语句根）。
    next_stage_id: u64,
    /// 语句开始时刻。`enter_statement` 写入，供首行等待计算。
    started_at: Option<Instant>,
    /// 第一行进入将返回的结果时，从 `started_at` 起的时长。
    first_row: Option<Duration>,
    /// 非空结果行集的交回行数。空结果、受影响行数和错误为 0。
    rows_delivered: u64,
}

struct OpenStage {
    /// 阶段 id——等待按 id 归属，跨任务回写按 id 落账。
    id: u64,
    module: Module,
    name: String,
    children: Vec<StageNode>,
}

/// 语句根的阶段 id：没有 open 阶段时发生的等待记入语句根，不进入阶段
/// 汇总数。
const ROOT_STAGE_ID: u64 = 0;

impl StageStack {
    /// 挂一个已完结阶段到当前最内 open 阶段（无 open 时挂到根）。
    fn push_completed(&mut self, module: Module, name: &str, inclusive: Duration) {
        let node = StageNode {
            module,
            name: name.to_string(),
            inclusive,
            children: Vec::new(),
            waits: WaitTotals::default(),
        };
        match self.open.last_mut() {
            Some(innermost) => innermost.children.push(node),
            None => self.root_children.push(node),
        }
    }

    fn open_stage(&mut self, module: Module, name: &str) {
        self.next_stage_id += 1;
        self.open.push(OpenStage {
            id: self.next_stage_id,
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
        let node = StageNode {
            module: open.module,
            name: open.name,
            inclusive,
            children: open.children,
            waits: WaitTotals::default(),
        };
        // 等待累计：语句任务本地测得的与刷盘任务回写的都落在该阶段的槽里，
        // 阶段闭合时一次取走。
        let mut waits = self.waits.take_stage(open.id);
        // Compute = 独占耗时 − 自身等待之和。内层等待不重复计入外层阶段：
        // 外层独占耗时已扣除子阶段包含耗时。
        let compute = node.exclusive().saturating_sub(waits.wait_total());
        waits.add(Category::Compute, compute);
        let node = StageNode { waits, ..node };
        match self.open.last_mut() {
            Some(parent) => parent.children.push(node),
            None => self.root_children.push(node),
        }
    }

    /// 根作用域结束：取走根的已完成子阶段；仍 open 的阶段没有耗时，弃置。
    fn finish(&mut self) -> Vec<StageNode> {
        self.open.clear();
        std::mem::take(&mut self.root_children)
    }
}

// ---------- 等待槽（MS44 Iteration 002 / T3）----------

/// 等待槽：一条语句的等待累计容器，跨任务共享。
///
/// 语句任务在登记等待者时把句柄交给刷盘任务；刷盘任务只回写 Io 与
/// Schedule，不解释语义——句柄的语义由本模块解释。
#[derive(Clone, Default)]
pub(crate) struct WaitSlot {
    shared: Arc<Mutex<SlotShared>>,
}

#[derive(Default)]
struct SlotShared {
    /// 按阶段 id 的等待累计（id 0 = 语句根）。
    stages: HashMap<u64, StageWait>,
}

/// 一个阶段的等待累计。
#[derive(Default, Clone, Copy)]
struct StageWait {
    /// 阶段累计（`Compute` 在阶段闭合时按独占耗时算出）。
    totals: WaitTotals,
    /// 最近一次刷盘回写的 Io 与 Schedule（Queue 计算用）。
    last_flush: Option<FlushWait>,
}

/// 一次刷盘回写给等待者的 Io 与 Schedule。
#[derive(Clone, Copy)]
pub(crate) struct FlushWait {
    pub(crate) io: Duration,
    pub(crate) schedule: Duration,
}

impl WaitSlot {
    fn bucket_mut(shared: &mut SlotShared, stage_id: Option<u64>) -> &mut StageWait {
        shared
            .stages
            .entry(stage_id.unwrap_or(ROOT_STAGE_ID))
            .or_default()
    }

    /// 记一次等待（语句任务内测得）。
    fn add(&self, stage_id: Option<u64>, category: Category, duration: Duration) {
        let mut shared = self.shared.lock().unwrap();
        Self::bucket_mut(&mut shared, stage_id)
            .totals
            .add(category, duration);
    }

    /// 记一次文件页读取（Io + 页数加 1）。
    fn add_page(&self, stage_id: Option<u64>, duration: Duration) {
        let mut shared = self.shared.lock().unwrap();
        let bucket = Self::bucket_mut(&mut shared, stage_id);
        bucket.totals.add(Category::Io, duration);
        bucket.totals.add_pages(1);
    }

    /// 跨任务回写：一次刷盘的 Io 与 Schedule——只写给被这次刷盘释放的
    /// 等待者所属的语句。
    pub(crate) fn record_flush(&self, stage_id: Option<u64>, io: Duration, schedule: Duration) {
        let mut shared = self.shared.lock().unwrap();
        let bucket = Self::bucket_mut(&mut shared, stage_id);
        bucket.totals.add(Category::Io, io);
        bucket.totals.add(Category::Schedule, schedule);
        bucket.last_flush = Some(FlushWait { io, schedule });
    }

    /// 读回最近一次刷盘的 Io 与 Schedule（语句侧算 Queue 用）。
    pub(crate) fn last_flush(&self, stage_id: Option<u64>) -> Option<FlushWait> {
        let shared = self.shared.lock().unwrap();
        shared
            .stages
            .get(&stage_id.unwrap_or(ROOT_STAGE_ID))
            .and_then(|bucket| bucket.last_flush)
    }

    /// 取走某阶段的等待累计（阶段闭合时）。
    fn take_stage(&self, stage_id: u64) -> WaitTotals {
        let mut shared = self.shared.lock().unwrap();
        shared
            .stages
            .remove(&stage_id)
            .map(|bucket| bucket.totals)
            .unwrap_or_default()
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

/// 等待计时起点：任务内有语句绑定时 `Some`（绑定外 `None`——不计时）。
pub(crate) fn wait_timing_start() -> Option<Instant> {
    STATEMENT_STAGES.try_with(|_| Instant::now()).ok()
}

/// 记一次等待：归属「当时最内层 open 阶段」；没有 open 阶段时记入语句根
/// （不进入阶段汇总数）。无绑定的任务为 no-op。
pub(crate) fn record_wait(category: Category, duration: Duration) {
    let _ = STATEMENT_STAGES.try_with(|stack| {
        let stack = stack.lock().unwrap();
        let stage_id = stack.open.last().map(|stage| stage.id);
        stack.waits.add(stage_id, category, duration);
    });
}

/// 第一行 `ExecResult::Row` 放进将返回的结果时调用。
/// 无阶段绑定，或绑定上没有开始时刻时，不记。
pub(crate) fn note_first_delivered_row() {
    let _ = STATEMENT_STAGES.try_with(|stack| {
        let mut stack = stack.lock().unwrap();
        if stack.first_row.is_some() {
            return;
        }
        if let Some(started_at) = stack.started_at {
            stack.first_row = Some(started_at.elapsed());
        }
    });
}

/// 执行器结束时落定交回行数。`rows` 为 0 时清掉首行时长。
pub(crate) fn finish_delivered_rows(rows: u64) {
    let _ = STATEMENT_STAGES.try_with(|stack| {
        let mut stack = stack.lock().unwrap();
        if rows == 0 {
            stack.first_row = None;
            stack.rows_delivered = 0;
        } else {
            stack.rows_delivered = rows;
        }
    });
}

/// 记一次文件页读取：Io 耗时 + 页数加 1。无绑定的任务为 no-op。
pub(crate) fn record_page_read(duration: Duration) {
    let _ = STATEMENT_STAGES.try_with(|stack| {
        let stack = stack.lock().unwrap();
        let stage_id = stack.open.last().map(|stage| stage.id);
        stack.waits.add_page(stage_id, duration);
    });
}

/// 当前语句的等待槽与最内层 open 阶段 id。
///
/// 无任务内绑定（采集全关或非语句任务）时返回 `None`；有绑定但没有 open
/// 阶段时阶段 id 为 `None`——这类等待记入语句根。跨任务的等待者登记用它
/// 取得句柄（登记发生在语句任务内，句柄随等待者交给刷盘任务）。
pub(crate) fn current_wait_slot() -> Option<(WaitSlot, Option<u64>)> {
    STATEMENT_STAGES
        .try_with(|stack| {
            let stack = stack.lock().unwrap();
            let stage_id = stack.open.last().map(|stage| stage.id);
            (stack.waits.clone(), stage_id)
        })
        .ok()
}

/// 文件页读取计时守卫：析构时把整次调用的耗时记为 Io 并把页数加 1。
///
/// 守卫覆盖整个读取调用（含 `crypto` 分支与阻塞读），不需要复制读取函数体。
pub(crate) struct PageReadTimer {
    start: Option<Instant>,
}

impl PageReadTimer {
    /// 开始计时：没有语句绑定时不做任何事（默认配置零成本）。
    pub(crate) fn start() -> Self {
        PageReadTimer {
            start: wait_timing_start(),
        }
    }
}

impl Drop for PageReadTimer {
    fn drop(&mut self) {
        if let Some(start) = self.start.take() {
            record_page_read(start.elapsed());
        }
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
        let (children, first_row, rows_delivered) = match stages {
            Some(stack) => {
                let mut stack = stack.lock().unwrap();
                let children = stack.finish();
                if end == ScopeEnd::Success {
                    (children, stack.first_row, stack.rows_delivered)
                } else {
                    (children, None, 0)
                }
            }
            None => (Vec::new(), None, 0),
        };
        let root = StageNode {
            // 语句根是容器：不参与类别过滤，也不占阶段名。
            module: Module::Pipeline,
            name: "statement".to_string(),
            inclusive: inner.start.elapsed(),
            children,
            waits: WaitTotals::default(),
        };
        session.record_statement(
            &inner.sql,
            end,
            root,
            vec![],
            inner.start,
            first_row,
            rows_delivered,
        )
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
