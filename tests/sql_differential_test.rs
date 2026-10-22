//! rusqlite 差分测试网（MS30-T01 / I091）。
//!
//! RTsql（lib API，`:memory:`）与 SQLite（rusqlite in-memory，既有开发依赖）
//! 对同一固定种子数据集执行同一语义一致 SQL 子集，行集经归一化（BOOL→0/1
//! 折叠、数值内部按 f64 相等、行间稳定排序）后逐行比较。已知语义差异以
//! 白名单文档化（每条含查询 ID、分歧描述、原因分类、规避方式）；白名单外
//! 分歧使差分测试失败，不存在无条件跳过路径。
//!
//! 子集边界（双方语义一致的语句；踩线输入已排除，排除原因是双方类型或
//! 排序语义不同，不是 RTsql 能力缺失）：
//! - 严格类型门 vs SQLite type affinity：不向 INT/FLOAT 列插入字符串值；
//!   BOOL 列在 SQL 文本层用 TRUE/FALSE 字面量（SQLite 3.23+ 关键字，存 1/0）。
//! - 日期族以 TEXT 形态参与（DA5 `YYYY-MM-DD` 字典序=时间序，两引擎同为
//!   字符串语义）；不使用 DATE/TIMESTAMP 类型列（SQLite 无对应类型）。
//! - 排除裸除法（SQLite 除零返回 NULL、整数溢出转 Real，RTsql 报错）与
//!   无 ORDER BY 的 LIMIT（非确定行选择）。
//! - 排除显式 CAST 与跨族类型输入（CAST 方言差异面由既有回归锁定）。
//! - LIKE：RTsql 区分大小写、SQLite ASCII 不区分——种子数据与模式全部为
//!   小写 ASCII，两引擎判定一致；不引入混合大小写模式。
//! - 浮点聚合只用行序一致的形态：两引擎全表扫描均为插入序，SUM/AVG 逐项
//!   加法序相同、逐位一致；DISTINCT 聚合只用于整型列（求和精确），不对
//!   FLOAT 列做 AVG(DISTINCT)（两引擎去重后求和序可能不同）。
//! - ORDER BY 可空列：RTsql 把 NULL 排在 ASC 尾部，SQLite 排在头部——该
//!   已知分歧见白名单 `ORD-NULL-ORDERING`；无白名单的有序比较一律排除
//!   可空排序键。
//!
//! 等价变换（别名改写、派生表/CTE 包裹、谓词交换与重排）先在 RTsql 内部
//! 互比（变体 vs 原查询归一化一致），再对每个变体做跨引擎差分。

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rtsql::database::Database;
use rtsql::network::protocol::{to_response, Response};
use rusqlite::Connection;
use serde_json::Value as JsonVal;
use std::cmp::Ordering;

/// rusqlite 行值类型别名（与 `rtsql::Value` 区分）。
use rusqlite::types::Value as SqValue;

// ---------------------------------------------------------------------------
// 归一化
// ---------------------------------------------------------------------------

/// 归一化后的行值。
///
/// 排序全序：Null < Num < Text（`sort.rs` 与 SQLite 的 NULL 位置差异在
/// 无序比较中被行间排序消除）。数值统一 f64：本数据集全部整数结果
/// （计数、和）远小于 2^53，f64 表示精确；NaN/Inf 不可能出现
/// （RTsql `value_to_json` 将其映射为 Null，SQLite 不存储 NaN）。
#[derive(Debug, Clone, PartialEq)]
enum NormVal {
    Null,
    Num(f64),
    Text(String),
}

impl Eq for NormVal {}

impl PartialOrd for NormVal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for NormVal {
    fn cmp(&self, other: &Self) -> Ordering {
        fn rank(v: &NormVal) -> u8 {
            match v {
                NormVal::Null => 0,
                NormVal::Num(_) => 1,
                NormVal::Text(_) => 2,
            }
        }
        rank(self)
            .cmp(&rank(other))
            .then_with(|| match (self, other) {
                (NormVal::Num(a), NormVal::Num(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
                (NormVal::Text(a), NormVal::Text(b)) => a.cmp(b),
                _ => Ordering::Equal,
            })
    }
}

type NormRows = Vec<Vec<NormVal>>;

/// RTsql JSON 行值 → 归一化值。BOOL 折叠为 0/1（与 SQLite INTEGER 通道
/// 对齐，规格 R1 归一化规则，非语义放宽——折叠只发生在 Bool↔0/1 与数值
/// 内部，跨类分歧仍然失败）。
fn norm_rt(v: &JsonVal) -> NormVal {
    match v {
        JsonVal::Null => NormVal::Null,
        JsonVal::Bool(b) => NormVal::Num(if *b { 1.0 } else { 0.0 }),
        JsonVal::Number(n) => NormVal::Num(
            n.as_f64()
                .unwrap_or_else(|| panic!("non-finite number in result: {n}")),
        ),
        JsonVal::String(s) => NormVal::Text(s.clone()),
        other => panic!("unexpected JSON value in query result: {other}"),
    }
}

/// SQLite 行值 → 归一化值。
fn norm_sq(v: SqValue) -> NormVal {
    match v {
        SqValue::Null => NormVal::Null,
        SqValue::Integer(i) => NormVal::Num(i as f64),
        SqValue::Real(f) => NormVal::Num(f),
        SqValue::Text(s) => NormVal::Text(s),
        SqValue::Blob(b) => panic!("unexpected BLOB in differential result ({} bytes)", b.len()),
    }
}

/// 行间稳定排序（消除无 ORDER BY 查询的输出顺序差异；带 ORDER BY 的语义
/// 由 `order_and_aggregate_fixed_expectations` 的有序比较与单调性断言锁定）。
fn normalize(rows: impl Iterator<Item = Vec<NormVal>>) -> NormRows {
    let mut out: NormRows = rows.collect();
    out.sort();
    out
}

/// 失败消息用的行集摘要（前 5 行）。
fn summarize(rows: &NormRows) -> String {
    if rows.is_empty() {
        return "(0 rows)".to_string();
    }
    let shown = rows
        .iter()
        .take(5)
        .map(|r| format!("{r:?}"))
        .collect::<Vec<_>>()
        .join("; ");
    format!("{} rows; first: {shown}", rows.len())
}

// ---------------------------------------------------------------------------
// 白名单（规格 R1：每条含查询 ID、分歧描述、原因分类、规避方式）
// ---------------------------------------------------------------------------

/// 已知语义分歧白名单。原因分类只允许两种：
/// - `known-semantic-difference`：双方文档化语义不同，属预期差异；
/// - `suspected-product-defect`：疑似 RTsql 缺陷（按 proposal A4 分诊——
///   收口面内为 T02 修复候选，范围外为 Issue 候选报告）。
const CLASS_KNOWN: &str = "known-semantic-difference";
const CLASS_DEFECT: &str = "suspected-product-defect";

/// (查询 ID, 分歧描述, 原因分类, 规避方式)
///
/// 首跑分诊（2026-10-20，A4）：白名单中的 suspected-product-defect 条目是
/// 差分网抓到的静默错误结果/拒绝面的可执行记录。T03（2026-10-21）清偿后
/// 四类缺陷条目全部移除——归因与修复面记录如下：
/// ① `Q3-03/Q3-08/Q6-03`：无主键基表（items——首列 TEXT、无 INTEGER PRIMARY
///    KEY）作为 JOIN 子节点或 IN 子查询 SemiJoin 外层基表时返回空集。
///    `ScanExecutor::next` 以主键索引枚举行（`table_meta.index_manager
///    .scan_all()`，src/executor/scan.rs），无主键表无 PK 索引 → 0 行。
///    T03-D1：无键表（PK 位列类型非 Int）枚举通道回退数据页走查。
/// ② `Q5-05`：TEXT 相关参数求值为 NULL——
///    `ParameterExpression::evaluate_ref` 对 `Value::String` 返回
///    `ValueRef::Null`（src/executor/predicate.rs，M37 零拷贝 TODO），
///    相关子查询谓词 `s2.carrier = s.carrier` 恒 Unknown → 外层 0 行。
///    T03-D2：`ParameterExpression::evaluate` 覆盖为 owned 真值。
/// ③ 聚合派生表（`FROM (SELECT tag AS n, COUNT(*) … GROUP BY tag) d`）
///    外层列引用计划期拒绝（`Column not found: n`）；同内容 CTE 形态正常
///    （`get_plan_output_columns` 对 Aggregate 计划的列提取缺口）。T03-D3：
///    派生臂列源按 SELECT 清单逐位取名（键位取别名）。
/// ④ `Q6-04`：NOT IN 对 NULL 外层键不做三值折叠——AntiJoin 保留「无匹配」
///    的 NULL 键行。T03-D4：右集 NULL 键置位 + 左键 NULL 行三值折叠。
const KNOWN_DIVERGENCES: &[(&str, &str, &str, &str)] = &[
    (
        "ORD-NULL-ORDERING",
        "ORDER BY 可空列时 NULL 的排序位置：RTsql 将 NULL 排在 ASC 尾部/DESC 头部（sort.rs compare_values「NULL to end」），SQLite 将 NULL 排在 ASC 头部/DESC 尾部（NULL 为最小值）",
        CLASS_KNOWN,
        "需要 NULL 优先排序时用显式 IS NULL 分支或 COALESCE 表达；差分子集内可空排序键仅出现在本条目对应的观察形态，其余有序比较均排除可空排序键",
    ),
];

/// 白名单条目结构自检：ID/描述/分类/规避四字段齐全且分类值合法
/// （规格 R1 场景「白名单条目各带原因分类」）。
#[test]
fn whitelist_entries_are_fully_classified() {
    for (id, desc, class, workaround) in KNOWN_DIVERGENCES {
        assert!(!id.is_empty(), "whitelist entry has empty id");
        assert!(
            !desc.is_empty(),
            "[{id}] whitelist entry has empty description"
        );
        assert!(
            *class == CLASS_KNOWN || *class == CLASS_DEFECT,
            "[{id}] unknown classification: {class}"
        );
        assert!(
            !workaround.is_empty(),
            "[{id}] whitelist entry has empty workaround"
        );
    }
}

// ---------------------------------------------------------------------------
// 引擎构建（固定种子数据集，D-1）
// ---------------------------------------------------------------------------

struct Engines {
    rt: Database,
    sq: Connection,
}

/// 固定种子：任何一次运行生成同一数据集（失败输入可缩减、可重现）。
const SEED: u64 = 0x5EED_2026_1020;

fn opt_i(null: bool, v: i64) -> String {
    if null {
        "NULL".to_string()
    } else {
        v.to_string()
    }
}

fn opt_s(null: bool, v: &str) -> String {
    if null {
        "NULL".to_string()
    } else {
        format!("'{v}'")
    }
}

/// note 列：绝大多数 NULL；少量短值与越过存储边界的长值（~5,000B 越过
/// 4074B 内联阈值、65535/65536B 跨 tag 切换点、~70,000B），SQL 层透明。
fn note_literal(id: u32) -> String {
    match id {
        300..=304 => format!("'note5k_{id}_{}'", "n".repeat(4900)),
        305 => format!("'{}'", "n".repeat(65535)),
        306 => format!("'{}'", "n".repeat(65536)),
        307 => format!("'{}'", "n".repeat(70000)),
        308 => "'short_note_one'".to_string(),
        309 => "'short_note_two'".to_string(),
        _ => "NULL".to_string(),
    }
}

/// 固定种子数据集：4 表约 1020 行 + DDL。同一份 SQL 文本灌入两引擎
/// （DDL/INSERT 语法双方通用；BOOL 列 SQLite 经 NUMERIC affinity 存 1/0）。
fn seed_statements() -> Vec<String> {
    let mut rng = StdRng::seed_from_u64(SEED);
    let mut stmts = Vec::new();

    stmts.push(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, age INT, score FLOAT, active BOOL, joined TEXT)"
            .to_string(),
    );
    stmts.push(
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, user_id INT, amount INT, tag TEXT, note TEXT)"
            .to_string(),
    );
    stmts.push("CREATE TABLE items (k TEXT, a INT, b INT, label TEXT)".to_string());
    stmts.push(
        "CREATE TABLE shipments (id INTEGER PRIMARY KEY, ord_id INT, carrier TEXT, days INT)"
            .to_string(),
    );

    // users：240 行，唯一 name、可空 age/score/active、DA5 日期字符串。
    for id in 1..=240u32 {
        let name = format!("user_{id:04}");
        let age = opt_i(rng.gen_bool(0.05), rng.gen_range(20..=70));
        let score = if rng.gen_bool(0.08) {
            "NULL".to_string()
        } else {
            format!("{:.2}", rng.gen_range(0..10000) as f64 / 100.0)
        };
        let active = if rng.gen_bool(0.05) {
            "NULL".to_string()
        } else if rng.gen_bool(0.55) {
            "TRUE".to_string()
        } else {
            "FALSE".to_string()
        };
        let joined = format!(
            "'{:04}-{:02}-{:02}'",
            rng.gen_range(2020..=2024),
            rng.gen_range(1..=12),
            rng.gen_range(1..=28)
        );
        stmts.push(format!(
            "INSERT INTO users VALUES ({id}, '{name}', {age}, {score}, {active}, {joined})"
        ));
    }

    // orders：320 行，user_id 有 NULL 与重复、amount 重复簇（15% 落在 100 的倍数）、
    // tag 小基数集合、note 长值少量。
    for id in 1..=320u32 {
        let user_id = opt_i(rng.gen_bool(0.03), rng.gen_range(1..=240));
        let amount = if rng.gen_bool(0.15) {
            100 * rng.gen_range(1..=10)
        } else {
            rng.gen_range(1..=5000)
        };
        let tag = opt_s(
            rng.gen_bool(0.10),
            ["alpha", "beta", "gamma", "delta"][rng.gen_range(0..4)],
        );
        let note = note_literal(id);
        stmts.push(format!(
            "INSERT INTO orders VALUES ({id}, {user_id}, {amount}, {tag}, {note})"
        ));
    }

    // items：220 行，TEXT 首列（无隐式主键约束）、a/b 低基数重复（自连接形状）、
    // label 小基数集合。
    for i in 1..=220u32 {
        let k = format!("'item_{i:03}'");
        let a = opt_i(rng.gen_bool(0.05), rng.gen_range(1..=30));
        let b = opt_i(rng.gen_bool(0.05), rng.gen_range(1..=30));
        let label = opt_s(
            rng.gen_bool(0.10),
            ["red", "green", "blue"][rng.gen_range(0..3)],
        );
        stmts.push(format!("INSERT INTO items VALUES ({k}, {a}, {b}, {label})"));
    }

    // shipments：240 行，ord_id 指向 orders（含重复——多 shipment 同单）、
    // carrier/days 可空。
    for id in 1..=240u32 {
        let ord_id = rng.gen_range(1..=320);
        let carrier = opt_s(
            rng.gen_bool(0.05),
            ["fedex", "ups", "dhl"][rng.gen_range(0..3)],
        );
        let days = opt_i(rng.gen_bool(0.04), rng.gen_range(1..=14));
        stmts.push(format!(
            "INSERT INTO shipments VALUES ({id}, {ord_id}, {carrier}, {days})"
        ));
    }

    stmts
}

/// 构建两引擎并灌入同一份种子数据。
async fn build_engines() -> Engines {
    let stmts = seed_statements();

    let rt = Database::open_in_memory()
        .await
        .expect("open RTsql :memory:");
    for (i, stmt) in stmts.iter().enumerate() {
        match to_response(rt.execute_sql(stmt).await) {
            Response::QueryResult { .. } | Response::AffectedRows { .. } => {}
            Response::Error { message, .. } => {
                panic!("seed stmt #{i} failed on RTsql: {message}\n{stmt}")
            }
            other => panic!("seed stmt #{i}: unexpected response {other:?}"),
        }
    }

    let sq = Connection::open_in_memory().expect("open SQLite :memory:");
    sq.execute_batch(&stmts.join(";\n"))
        .expect("seed batch failed on SQLite");

    Engines { rt, sq }
}

// ---------------------------------------------------------------------------
// 差分通道
// ---------------------------------------------------------------------------

async fn rows_rt(db: &Database, id: &str, sql: &str) -> NormRows {
    match to_response(db.execute_sql(sql).await) {
        Response::QueryResult { rows } => {
            normalize(rows.into_iter().map(|r| r.iter().map(norm_rt).collect()))
        }
        Response::Error { message, .. } => panic!(
            "[{id}] RTsql rejected subset query (subset boundary or product gap, triage per A4): {message}\nSQL: {sql}"
        ),
        other => panic!("[{id}] expected QueryResult, got {other:?}\nSQL: {sql}"),
    }
}

fn rows_sq(conn: &Connection, id: &str, sql: &str) -> NormRows {
    let mut stmt = conn
        .prepare(sql)
        .unwrap_or_else(|e| panic!("[{id}] SQLite prepare failed: {e}\nSQL: {sql}"));
    let ncols = stmt.column_count();
    let mut rows = stmt
        .query([])
        .unwrap_or_else(|e| panic!("[{id}] SQLite query failed: {e}"));
    let mut out = Vec::new();
    while let Some(row) = rows
        .next()
        .unwrap_or_else(|e| panic!("[{id}] SQLite row iteration failed: {e}"))
    {
        let mut nv = Vec::with_capacity(ncols);
        for i in 0..ncols {
            let v: SqValue = row
                .get(i)
                .unwrap_or_else(|e| panic!("[{id}] SQLite get column {i} failed: {e}"));
            nv.push(norm_sq(v));
        }
        out.push(nv);
    }
    normalize(out.into_iter())
}

/// 差分单查询：命中白名单 → 记录性断言（条目存在且四字段齐全）后跳过
/// 行集比较；未命中 → 归一化行集必须一致，分歧 panic 带双方摘要。
async fn diff_one(eng: &Engines, id: &str, sql: &str) {
    if let Some((_, desc, class, workaround)) =
        KNOWN_DIVERGENCES.iter().find(|(qid, ..)| *qid == id)
    {
        assert!(
            !desc.is_empty() && !class.is_empty() && !workaround.is_empty(),
            "[{id}] whitelist entry incomplete"
        );
        println!("[{id}] whitelisted ({class}): {desc}");
        return;
    }
    let rt = rows_rt(&eng.rt, id, sql).await;
    let sq = rows_sq(&eng.sq, id, sql);
    assert_eq!(
        rt,
        sq,
        "[{id}] differential divergence\nSQL: {sql}\nRTsql:  {}\nSQLite: {}",
        summarize(&rt),
        summarize(&sq)
    );
}

/// 按组循环差分（查询 ID = `Q{n}-{序号}`）。
async fn diff_group(eng: &Engines, group: u8, queries: &[&str]) {
    for (i, sql) in queries.iter().enumerate() {
        diff_one(eng, &format!("Q{group}-{:02}", i + 1), sql).await;
    }
}

// ---------------------------------------------------------------------------
// 差分子集（D-2：Q1-Q7，硬编码清单）
// ---------------------------------------------------------------------------

/// Q1 投影/别名。
const Q1: &[&str] = &[
    "SELECT id, name FROM users",
    "SELECT name AS n, age FROM users WHERE age IS NOT NULL",
    "SELECT u.name, u.age FROM users u WHERE u.id <= 50",
    "SELECT * FROM shipments WHERE days < 5",
    "SELECT id, score FROM users WHERE score IS NOT NULL",
    "SELECT name, age + 1 FROM users WHERE id <= 20",
    "SELECT t.k, t.a FROM items t WHERE t.b = 5",
    "SELECT id, joined FROM users WHERE joined >= '2022-06-01' AND joined < '2023-01-01'",
];

/// Q2 WHERE 谓词组合。
const Q2: &[&str] = &[
    "SELECT id FROM users WHERE age > 60",
    "SELECT id FROM orders WHERE amount >= 4000 AND tag IS NOT NULL",
    "SELECT id FROM orders WHERE amount < 100 OR amount > 4900",
    "SELECT id FROM orders WHERE NOT (amount BETWEEN 100 AND 4900)",
    "SELECT id FROM orders WHERE tag IN ('alpha', 'gamma')",
    "SELECT id FROM orders WHERE amount IN (100, 200, 300, 400, 500)",
    "SELECT id FROM users WHERE name LIKE 'user_01%'",
    "SELECT id FROM users WHERE name LIKE 'user_02__'",
    "SELECT id FROM orders WHERE user_id IS NULL",
    "SELECT id FROM users WHERE active = TRUE",
    "SELECT id FROM users WHERE joined >= '2022-06-01' AND joined < '2023-01-01'",
    "SELECT id FROM shipments WHERE days BETWEEN 3 AND 7 AND carrier IS NOT NULL",
    "SELECT k FROM items WHERE a = 7 AND b IS NOT NULL",
    "SELECT id FROM users WHERE NOT (age > 60 AND active = FALSE)",
];

/// Q3 JOIN（INNER 等值、自连接别名、≥3 表链式；复合 ON 条件）。
const Q3: &[&str] = &[
    "SELECT users.name, orders.amount FROM users JOIN orders ON users.id = orders.user_id WHERE orders.tag = 'alpha'",
    "SELECT u.name, o.amount FROM users u JOIN orders o ON u.id = o.user_id WHERE o.amount > 4500",
    "SELECT i1.k, i2.k FROM items i1 JOIN items i2 ON i1.a = i2.b WHERE i1.k < i2.k",
    "SELECT users.name, shipments.carrier FROM users JOIN orders ON users.id = orders.user_id JOIN shipments ON shipments.ord_id = orders.id WHERE shipments.days <= 3",
    "SELECT o.id, s.id FROM orders o JOIN shipments s ON s.ord_id = o.id WHERE o.amount BETWEEN 1000 AND 1100",
    "SELECT * FROM shipments JOIN orders ON shipments.ord_id = orders.id WHERE orders.id = 5",
    "SELECT u.id, o.id FROM users u JOIN orders o ON u.id = o.user_id AND o.amount > 4800",
    "SELECT i1.k, i2.k, i1.a FROM items i1 JOIN items i2 ON i1.a = i2.b WHERE i1.label = 'red' AND i2.label IS NOT NULL",
];

/// Q4 聚合（GROUP BY 列键、HAVING、五聚合、DISTINCT 聚合、group_concat
/// 单行组形态、空集聚合）。
const Q4: &[&str] = &[
    "SELECT COUNT(*) FROM orders",
    "SELECT COUNT(note), COUNT(tag) FROM orders",
    "SELECT SUM(amount) FROM orders",
    "SELECT AVG(score) FROM users",
    "SELECT SUM(score) FROM users",
    "SELECT MIN(joined), MAX(joined) FROM users",
    "SELECT MIN(age), MAX(age) FROM users",
    "SELECT user_id, COUNT(*) FROM orders WHERE user_id IS NOT NULL GROUP BY user_id",
    "SELECT tag, COUNT(*), SUM(amount) FROM orders GROUP BY tag",
    "SELECT label, COUNT(a) FROM items GROUP BY label",
    "SELECT tag, COUNT(*) FROM orders GROUP BY tag HAVING COUNT(*) > 50",
    "SELECT amount, COUNT(*) FROM orders WHERE amount <= 1000 GROUP BY amount HAVING COUNT(*) >= 2",
    "SELECT COUNT(DISTINCT user_id) FROM orders",
    "SELECT COUNT(DISTINCT tag) FROM orders",
    "SELECT COUNT(DISTINCT amount) FROM orders",
    "SELECT AVG(DISTINCT age) FROM users",
    "SELECT SUM(DISTINCT amount) FROM orders WHERE amount <= 200",
    "SELECT id, group_concat(tag) FROM orders WHERE id <= 40 GROUP BY id",
    "SELECT AVG(amount) FROM orders WHERE amount < 50",
    "SELECT user_id, MIN(amount), MAX(amount), AVG(amount) FROM orders GROUP BY user_id",
    "SELECT SUM(amount), AVG(amount), COUNT(*) FROM orders WHERE amount > 9999",
];

/// Q5 标量子查询（WHERE 比较 + 投影位，相关与非相关；单表——JOIN 基座
/// 标量子查询是 RTsql 既有拒绝面，不进入子集）。
const Q5: &[&str] = &[
    "SELECT name FROM users WHERE age = (SELECT MAX(age) FROM users)",
    "SELECT id FROM orders WHERE amount > (SELECT AVG(amount) FROM orders)",
    "SELECT name FROM users u WHERE u.age > (SELECT AVG(age) FROM users)",
    "SELECT o.id FROM orders o WHERE o.amount > (SELECT AVG(o2.amount) FROM orders o2 WHERE o2.user_id = o.user_id)",
    "SELECT s.id FROM shipments s WHERE s.days < (SELECT AVG(s2.days) FROM shipments s2 WHERE s2.carrier = s.carrier)",
    "SELECT id, (SELECT MAX(amount) FROM orders) FROM users WHERE id <= 10",
    "SELECT u.id, (SELECT COUNT(*) FROM orders o WHERE o.user_id = u.id) FROM users u WHERE u.id <= 30",
    "SELECT name FROM users WHERE score > (SELECT AVG(score) FROM users WHERE score IS NOT NULL)",
    // 2026-10-20 replan（001-replan）碰撞形状：内层别名声明同表 + 基名限定
    // 引用 → 外层相关参数（别名遮蔽基名，对齐 SQLite——用户裁定）。
    "SELECT name FROM users WHERE age = (SELECT MAX(age) FROM users u WHERE u.id <= users.id)",
    "SELECT id, (SELECT MAX(age) FROM users u WHERE u.id <= users.id) FROM users WHERE id <= 20",
];

/// Q6 IN/EXISTS 子查询（相关与非相关、内层别名形态——历史缺陷面；
/// 内层单表——JOIN 形 IN 子查询是既有拒绝面）。
///
/// 首跑分诊更正：原 Q6-05 为 `carrier = 'dhl' AND ord_id IN (…)`——WHERE
/// 中普通谓词与 IN 子查询的 AND 组合是既有计划期拒绝面（`Unsupported
/// expression type: PREDICATE`），拒绝面不进入子集；改用单谓词 IN 形态，
/// 该组合缺口在 Act Response 报告。
const Q6: &[&str] = &[
    "SELECT id FROM orders WHERE user_id IN (SELECT id FROM users WHERE active = TRUE)",
    "SELECT id FROM users WHERE id IN (SELECT user_id FROM orders WHERE amount > 4900)",
    "SELECT k FROM items WHERE a IN (SELECT b FROM items WHERE label = 'red')",
    "SELECT id FROM orders WHERE user_id NOT IN (SELECT id FROM users)",
    "SELECT id FROM shipments WHERE ord_id IN (SELECT id FROM orders WHERE tag IS NOT NULL)",
    "SELECT id FROM users WHERE EXISTS (SELECT 1 FROM orders o WHERE o.user_id = users.id AND o.amount > 4800)",
    "SELECT id FROM users u WHERE EXISTS (SELECT 1 FROM orders o WHERE o.user_id = u.id)",
    "SELECT id FROM users WHERE NOT EXISTS (SELECT 1 FROM orders o WHERE o.user_id = users.id)",
    // 2026-10-20 replan（001-replan）碰撞形状（EXISTS 面）。
    "SELECT id FROM users WHERE EXISTS (SELECT 1 FROM users u WHERE u.age = users.age)",
];

/// Q7 派生表/CTE/DISTINCT/集合操作/ORDER BY+LIMIT（可空列不进入有序比较）。
///
/// 首跑分诊更正：原 Q7-01 为聚合派生表外层引用（`FROM (SELECT tag AS n,
/// COUNT(*) … GROUP BY tag) d`），RTsql 计划期拒绝（`Column not found: n`）——
/// 拒绝面不进入差分子集，改用非聚合派生表的限定外层引用形态。
/// T03-D3 修复后该形态进入差分子集（追加为 Q7-17）。
const Q7: &[&str] = &[
    "SELECT d.id, d.amount FROM (SELECT id, amount FROM orders WHERE tag = 'alpha') d",
    "SELECT * FROM (SELECT id, amount FROM orders WHERE amount > 4900) t",
    "WITH big AS (SELECT id, amount FROM orders WHERE amount > 4800) SELECT id, amount FROM big",
    "WITH s AS (SELECT carrier, COUNT(*) AS c FROM shipments GROUP BY carrier) SELECT carrier, c FROM s WHERE c > 40",
    "SELECT DISTINCT tag FROM orders",
    "SELECT DISTINCT user_id, tag FROM orders WHERE id <= 100",
    "SELECT id FROM users WHERE age > 65 UNION SELECT id FROM users WHERE score > 99.0",
    "SELECT carrier FROM shipments WHERE days IS NOT NULL UNION ALL SELECT carrier FROM shipments WHERE days = 1",
    "SELECT user_id FROM orders WHERE user_id IS NOT NULL INTERSECT SELECT id FROM users WHERE age > 60",
    "SELECT id FROM users WHERE age > 65 EXCEPT SELECT id FROM users WHERE active = FALSE",
    "SELECT tag FROM orders WHERE id <= 50 INTERSECT SELECT tag FROM orders WHERE id > 30",
    "SELECT id, amount FROM orders ORDER BY amount DESC, id LIMIT 10",
    "SELECT name FROM users ORDER BY name LIMIT 5",
    "SELECT id, score FROM users WHERE score IS NOT NULL ORDER BY score DESC, id LIMIT 8",
    "SELECT carrier, days FROM shipments WHERE days IS NOT NULL AND carrier IS NOT NULL ORDER BY days, carrier, id LIMIT 12",
    "SELECT tag, COUNT(*) AS c FROM orders GROUP BY tag ORDER BY c DESC, tag",
    "SELECT DISTINCT label FROM items ORDER BY label",
    // T03-D3：聚合派生表的外层限定列引用（键位别名可见）。
    "SELECT d.n, d.c FROM (SELECT tag AS n, COUNT(*) AS c FROM orders GROUP BY tag) d",
];

#[tokio::test]
async fn q1_projection_and_alias() {
    let eng = build_engines().await;
    diff_group(&eng, 1, Q1).await;
}

#[tokio::test]
async fn q2_where_predicates() {
    let eng = build_engines().await;
    diff_group(&eng, 2, Q2).await;
}

#[tokio::test]
async fn q3_inner_joins() {
    let eng = build_engines().await;
    diff_group(&eng, 3, Q3).await;
}

#[tokio::test]
async fn q4_aggregates() {
    let eng = build_engines().await;
    diff_group(&eng, 4, Q4).await;
}

#[tokio::test]
async fn q5_scalar_subqueries() {
    let eng = build_engines().await;
    diff_group(&eng, 5, Q5).await;
}

#[tokio::test]
async fn q6_in_exists_subqueries() {
    let eng = build_engines().await;
    diff_group(&eng, 6, Q6).await;
}

#[tokio::test]
async fn q7_derived_cte_sets_order() {
    let eng = build_engines().await;
    diff_group(&eng, 7, Q7).await;
}

// ---------------------------------------------------------------------------
// 等价变换（D-3：普通函数生成变体，RTsql 内部互比 + 逐变体跨引擎差分）
// ---------------------------------------------------------------------------

struct EquivCase {
    id: &'static str,
    base: String,
    variants: Vec<String>,
}

/// 派生表包裹。
fn wrap_derived(sql: &str) -> String {
    format!("SELECT * FROM ({sql}) d")
}

/// CTE 包裹。
fn wrap_cte(sql: &str) -> String {
    format!("WITH w AS ({sql}) SELECT * FROM w")
}

/// 交换谓词两臂（子串必须唯一；占位符 \u{0} 不可能出现在 SQL 文本中）。
fn swap_arms(sql: &str, a: &str, b: &str) -> String {
    sql.replacen(a, "\u{0}TMP\u{0}", 1)
        .replacen(b, a, 1)
        .replace("\u{0}TMP\u{0}", b)
}

/// 双表 JOIN 基查询模板（表别名参数化）。
fn join_users_orders(ua: &str, oa: &str) -> String {
    format!(
        "SELECT {ua}.name, {oa}.amount FROM users {ua} JOIN orders {oa} \
         ON {ua}.id = {oa}.user_id WHERE {oa}.amount > 4500 AND {oa}.tag = 'alpha'"
    )
}

/// 自连接基查询模板（基表带整型主键——无主键表在 JOIN 基座返回空集，见
/// 白名单注释 ①，等价变换用例不落在该缺陷面上）。
fn self_join_shipments(sa: &str, sb: &str) -> String {
    format!(
        "SELECT {sa}.id, {sb}.id FROM shipments {sa} JOIN shipments {sb} \
         ON {sa}.ord_id = {sb}.ord_id WHERE {sa}.carrier = 'dhl' AND {sb}.days IS NOT NULL"
    )
}

/// 等价变换用例：每条基查询 2-3 个变体。
fn equiv_cases() -> Vec<EquivCase> {
    let join_base = join_users_orders("u", "o");
    let self_base = self_join_shipments("s1", "s2");
    let single_base =
        "SELECT id, amount FROM orders WHERE amount > 4800 AND tag IS NOT NULL".to_string();
    let group_base =
        "SELECT user_id, COUNT(*) FROM orders WHERE user_id IS NOT NULL GROUP BY user_id"
            .to_string();
    let having_base =
        "SELECT tag, COUNT(*) AS c FROM orders GROUP BY tag HAVING COUNT(*) > 30".to_string();
    let bool_base = "SELECT id FROM users WHERE age > 60 AND active = TRUE".to_string();

    vec![
        EquivCase {
            id: "EQ-JOIN-ALIAS",
            base: join_base.clone(),
            variants: vec![
                join_users_orders("usr", "ord"),
                swap_arms(&join_base, "o.amount > 4500", "o.tag = 'alpha'"),
            ],
        },
        EquivCase {
            id: "EQ-JOIN-WRAP",
            base: join_base,
            variants: vec![wrap_derived(&join_users_orders("u", "o")), wrap_cte(&join_users_orders("u", "o"))],
        },
        EquivCase {
            id: "EQ-SELF-JOIN",
            base: self_base.clone(),
            variants: vec![
                self_join_shipments("x", "y"),
                swap_arms(&self_base, "s1.carrier = 'dhl'", "s2.days IS NOT NULL"),
            ],
        },
        EquivCase {
            id: "EQ-SINGLE-PRED",
            base: single_base.clone(),
            variants: vec![
                swap_arms(&single_base, "amount > 4800", "tag IS NOT NULL"),
                wrap_derived(&single_base),
                wrap_cte(&single_base),
            ],
        },
        EquivCase {
            id: "EQ-GROUP",
            base: group_base.clone(),
            variants: vec![
                "SELECT user_id AS uid, COUNT(*) AS cnt FROM orders WHERE user_id IS NOT NULL GROUP BY user_id".to_string(),
                wrap_derived(&group_base),
            ],
        },
        EquivCase {
            id: "EQ-HAVING",
            base: having_base.clone(),
            variants: vec![
                "SELECT o.tag, COUNT(*) AS c FROM orders o GROUP BY o.tag HAVING COUNT(*) > 30"
                    .to_string(),
                wrap_derived(&having_base),
            ],
        },
        EquivCase {
            id: "EQ-BOOL-PRED",
            base: bool_base.clone(),
            variants: vec![swap_arms(&bool_base, "age > 60", "active = TRUE"), wrap_cte(&bool_base)],
        },
    ]
}

/// 等价变换一致性（规格 R2）：先 RTsql 内部互比（变体 vs 原查询），再对
/// 基查询与每个变体做跨引擎差分。
#[tokio::test]
async fn equivalence_transforms_consistent() {
    let eng = build_engines().await;
    for case in equiv_cases() {
        let base_rt = rows_rt(&eng.rt, case.id, &case.base).await;
        let base_sq = rows_sq(&eng.sq, case.id, &case.base);
        assert_eq!(
            base_rt,
            base_sq,
            "[{}] base query diverges\nSQL: {}\nRTsql:  {}\nSQLite: {}",
            case.id,
            case.base,
            summarize(&base_rt),
            summarize(&base_sq)
        );
        for (vi, variant) in case.variants.iter().enumerate() {
            let vid = format!("{}-v{}", case.id, vi + 1);
            let vrt = rows_rt(&eng.rt, &vid, variant).await;
            assert_eq!(
                vrt, base_rt,
                "[{vid}] equivalent variant diverges from base on RTsql\nSQL: {variant}\nbase:  {}\nvariant: {}",
                summarize(&base_rt),
                summarize(&vrt)
            );
            let vsq = rows_sq(&eng.sq, &vid, variant);
            assert_eq!(
                vrt,
                vsq,
                "[{vid}] variant diverges across engines\nSQL: {variant}\nRTsql:  {}\nSQLite: {}",
                summarize(&vrt),
                summarize(&vsq)
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 固定断言（D-3：ORDER BY 语义与聚合的有序/期望值锁定，小表手工可算）
// ---------------------------------------------------------------------------

/// 小表 `ob`：无 NULL 数值列（NULL 排序位置属白名单条目 ORD-NULL-ORDERING，
/// 不进入固定断言）。
const OB_DDL: &str = "CREATE TABLE ob (id INTEGER PRIMARY KEY, v INT)";
const OB_ROWS: &[(&str, &str)] = &[
    ("INSERT INTO ob VALUES (1, 30)", "ob-1"),
    ("INSERT INTO ob VALUES (2, 10)", "ob-2"),
    ("INSERT INTO ob VALUES (3, 30)", "ob-3"),
    ("INSERT INTO ob VALUES (4, 20)", "ob-4"),
    ("INSERT INTO ob VALUES (5, 10)", "ob-5"),
];

/// 有序执行：不做行间排序归一化，逐行逐位比较（ORDER BY 语义专用通道）。
async fn rows_rt_ordered(db: &Database, id: &str, sql: &str) -> NormRows {
    match to_response(db.execute_sql(sql).await) {
        Response::QueryResult { rows } => rows
            .into_iter()
            .map(|r| r.iter().map(norm_rt).collect())
            .collect(),
        Response::Error { message, .. } => panic!("[{id}] RTsql rejected: {message}\nSQL: {sql}"),
        other => panic!("[{id}] expected QueryResult, got {other:?}"),
    }
}

fn rows_sq_ordered(conn: &Connection, id: &str, sql: &str) -> NormRows {
    let mut stmt = conn
        .prepare(sql)
        .unwrap_or_else(|e| panic!("[{id}] SQLite prepare failed: {e}\nSQL: {sql}"));
    let ncols = stmt.column_count();
    let mut rows = stmt.query([]).unwrap();
    let mut out = Vec::new();
    while let Some(row) = rows.next().unwrap() {
        let mut nv = Vec::with_capacity(ncols);
        for i in 0..ncols {
            let v: SqValue = row.get(i).unwrap();
            nv.push(norm_sq(v));
        }
        out.push(nv);
    }
    out
}

/// 断言 RTsql 有序结果 == 期望值，且与 SQLite 有序结果逐位一致。
async fn assert_ordered(eng: &Engines, id: &str, sql: &str, expected: NormRows) {
    let rt = rows_rt_ordered(&eng.rt, id, sql).await;
    assert_eq!(
        rt,
        expected,
        "[{id}] ORDER BY expectation failed\nSQL: {sql}\nRTsql:  {}\nexpected: {expected:?}",
        summarize(&rt)
    );
    let sq = rows_sq_ordered(&eng.sq, id, sql);
    assert_eq!(
        rt, sq,
        "[{id}] ordered result diverges across engines\nSQL: {sql}"
    );
}

/// ORDER BY 全序/字典序/LIMIT 与聚合期望值（手工可算小表；规格 R2 场景
/// 之外的固定断言通道，锁定行序语义）。
#[tokio::test]
async fn order_and_aggregate_fixed_expectations() {
    let rt = Database::open_in_memory()
        .await
        .expect("open RTsql :memory:");
    let sq = Connection::open_in_memory().expect("open SQLite :memory:");
    for (sql, id) in std::iter::once((OB_DDL, "ob-ddl")).chain(OB_ROWS.iter().copied()) {
        match to_response(rt.execute_sql(sql).await) {
            Response::QueryResult { .. } | Response::AffectedRows { .. } => {}
            Response::Error { message, .. } => panic!("[{id}] RTsql failed: {message}"),
            other => panic!("[{id}] unexpected response {other:?}"),
        }
        sq.execute(sql, [])
            .unwrap_or_else(|e| panic!("[{id}] SQLite failed: {e}"));
    }
    let eng = Engines { rt, sq };

    let n = |x: f64| vec![NormVal::Num(x)];
    let nv = |a: f64, b: f64| vec![NormVal::Num(a), NormVal::Num(b)];

    // 升序 + id tiebreak。
    assert_ordered(
        &eng,
        "OB-ASC",
        "SELECT id FROM ob ORDER BY v ASC, id",
        vec![n(2.0), n(5.0), n(4.0), n(1.0), n(3.0)],
    )
    .await;
    // 降序 + 降序 tiebreak。
    assert_ordered(
        &eng,
        "OB-DESC",
        "SELECT id FROM ob ORDER BY v DESC, id DESC",
        vec![n(3.0), n(1.0), n(4.0), n(5.0), n(2.0)],
    )
    .await;
    // LIMIT 截断有序面。
    assert_ordered(
        &eng,
        "OB-LIMIT",
        "SELECT id FROM ob ORDER BY v ASC, id LIMIT 3",
        vec![n(2.0), n(5.0), n(4.0)],
    )
    .await;
    // 聚合期望值（COUNT/SUM/AVG/MIN/MAX 单行五列输出，跨引擎逐位一致）。
    assert_ordered(
        &eng,
        "OB-AGG",
        "SELECT COUNT(*), SUM(v), AVG(v), MIN(v), MAX(v) FROM ob",
        vec![vec![n(5.0), n(100.0), n(20.0), n(10.0), n(30.0)]
            .into_iter()
            .flatten()
            .collect()],
    )
    .await;
    // GROUP BY + ORDER BY 聚合键。
    assert_ordered(
        &eng,
        "OB-GROUP",
        "SELECT v, COUNT(*) FROM ob GROUP BY v ORDER BY v",
        vec![nv(10.0, 2.0), nv(20.0, 1.0), nv(30.0, 2.0)],
    )
    .await;

    // 大数据集上的单调性：LIMIT+ORDER BY 的窗口满足排序键约束（不需要
    // 手算种子期望值即可锁排序语义）。
    let eng_big = build_engines().await;
    let top = rows_rt_ordered(
        &eng_big.rt,
        "OB-TOP",
        "SELECT amount, id FROM orders ORDER BY amount DESC, id LIMIT 10",
    )
    .await;
    assert_eq!(top.len(), 10, "[OB-TOP] expected 10 rows");
    for w in top.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        let amount_ok = match (&a[0], &b[0]) {
            (NormVal::Num(x), NormVal::Num(y)) => x >= y,
            _ => panic!("[OB-TOP] non-numeric amount in ordered result"),
        };
        assert!(
            amount_ok,
            "[OB-TOP] amounts not non-increasing: {a:?} then {b:?}"
        );
        if let (NormVal::Num(x), NormVal::Num(y)) = (&a[0], &b[0]) {
            if x == y {
                if let (NormVal::Num(i1), NormVal::Num(i2)) = (&a[1], &b[1]) {
                    assert!(
                        i1 < i2,
                        "[OB-TOP] id tiebreak not ascending within equal amounts: {a:?} then {b:?}"
                    );
                }
            }
        }
    }
}
