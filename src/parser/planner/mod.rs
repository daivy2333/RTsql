//! PlanBuilder - Convert AST to PhysicalPlan
//!
//! M4: SQL Parser and Physical Plan
//!
//! MS07-T03: split into `mod.rs` (struct + core + build_plan dispatcher) and
//! five sub-modules by responsibility:
//!   - `query`      — SELECT / FROM / JOIN / projection / PK-equality
//!   - `expression` — `build_expression` / `build_where` / `resolve_column_ref`
//!   - `aggregate`  — `build_having` + aggregate helpers
//!   - `subquery`   — IN/EXISTS subquery, correlated parameters
//!   - `ddl_dml`    — INSERT / UPDATE / DELETE / CREATE TABLE / DROP TABLE

mod aggregate;
mod binding;
mod ddl_dml;
mod expression;
mod query;
mod subquery;

use crate::executor::{ExplainNode, PhysicalPlan, Value};
use crate::parser::ast::unsupported_statement_name;
use crate::parser::error::PlanError;
use crate::storage::page_format::ColumnType;
use sqlparser::ast::Expr;
use sqlparser::ast::ObjectType;
use sqlparser::ast::Statement;
use sqlparser::ast::Value as SqlValue;
use std::collections::HashMap;

/// Transaction statement kinds recognized on the SQL surface (MS11-T02).
///
/// Shared by `build_plan` (non-session rejection, R4) and the CLI session
/// dispatcher (Iteration 001, R1/R3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxStatementKind {
    Begin,
    Commit,
    Rollback,
}

/// Classify a parsed statement as a transaction statement (design D2).
///
/// `Ok(None)` — not a transaction statement; `Ok(Some(kind))` — a clean
/// `BEGIN`/`COMMIT`/`ROLLBACK` (no modes/modifier/chain/savepoint); `Err` —
/// a boundary clause that is explicitly rejected, carrying the design D3
/// named message. Note sqlparser's `Commit`/`Rollback` `chain: bool` cannot
/// distinguish explicit `AND NO CHAIN` from absence, so both classify as the
/// clean statement.
pub(crate) fn classify_transaction_statement(
    stmt: &Statement,
) -> Result<Option<TxStatementKind>, PlanError> {
    match stmt {
        Statement::StartTransaction {
            modes, modifier, ..
        } => {
            if modes.is_empty() && modifier.is_none() {
                Ok(Some(TxStatementKind::Begin))
            } else {
                Err(PlanError::TransactionStatement(
                    "transaction modes in BEGIN/START TRANSACTION are not supported".to_string(),
                ))
            }
        }
        Statement::Commit { chain } => {
            if *chain {
                Err(PlanError::TransactionStatement(
                    "COMMIT AND CHAIN is not supported".to_string(),
                ))
            } else {
                Ok(Some(TxStatementKind::Commit))
            }
        }
        Statement::Rollback { chain, savepoint } => {
            if savepoint.is_some() {
                Err(PlanError::TransactionStatement(
                    "ROLLBACK TO SAVEPOINT is not supported".to_string(),
                ))
            } else if *chain {
                Err(PlanError::TransactionStatement(
                    "ROLLBACK AND CHAIN is not supported".to_string(),
                ))
            } else {
                Ok(Some(TxStatementKind::Rollback))
            }
        }
        Statement::SetTransaction { .. } => Err(PlanError::TransactionStatement(
            "SET TRANSACTION is not supported".to_string(),
        )),
        Statement::Savepoint { .. } => Err(PlanError::TransactionStatement(
            "SAVEPOINT is not supported".to_string(),
        )),
        Statement::ReleaseSavepoint { .. } => Err(PlanError::TransactionStatement(
            "RELEASE SAVEPOINT is not supported".to_string(),
        )),
        _ => Ok(None),
    }
}

/// MS20 T1: `ATTACH '<path>' AS <alias>` 的分类结果（会话注册面消费）。
///
/// 别名按标识符归一化规则小写处理（`table-name-resolution` 同型）；路径只接受
/// 单引号字符串字面量——`ATTACH` 的路径是打开期参数，表达式形态无法在计划期
/// 解析成文件位置（点名拒绝，不静默回落）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttachInfo {
    /// 数据库路径（已按 `resolve_db_path` 语义待解析的原始参数）
    pub(crate) path: String,
    /// 会话内别名（小写归一）
    pub(crate) alias: String,
}

/// Classify a parsed statement as an ATTACH statement (MS20 T1, design D2).
///
/// `Ok(None)` — not an ATTACH statement; `Ok(Some(info))` — a clean
/// `ATTACH [DATABASE] '<literal>' AS <alias>`; `Err` — a path form that is
/// explicitly rejected (anything other than a single-quoted string literal),
/// carrying the named message. Shared by `build_plan` (lib non-session
/// rejection, R3) and the CLI session dispatcher (R1/R2).
pub(crate) fn classify_attach_statement(stmt: &Statement) -> Result<Option<AttachInfo>, PlanError> {
    match stmt {
        Statement::AttachDatabase {
            schema_name,
            database_file_name,
            ..
        } => {
            let path = match database_file_name {
                Expr::Value(SqlValue::SingleQuotedString(path)) => path.clone(),
                _ => {
                    return Err(PlanError::AttachStatement(
                        "ATTACH database path must be a string literal".to_string(),
                    ))
                }
            };
            Ok(Some(AttachInfo {
                path,
                alias: schema_name.value.to_lowercase(),
            }))
        }
        _ => Ok(None),
    }
}

/// PlanBuilder - Convert AST to PhysicalPlan
///
/// Stores table metadata (columns, primary keys) for validation and plan generation.
#[derive(Debug, Clone)]
pub struct PlanBuilder {
    /// Table name -> column names
    pub(crate) tables: HashMap<String, Vec<String>>,
    /// Table name -> primary key column name
    pub(crate) primary_keys: HashMap<String, String>,
    /// MS16 Iteration 000 (I046): Table name -> key column declared type (design D2 additive
    /// channel). Real tables are passed through at the pipeline registration point; derived table aliases
    /// (pk="") are not registered — unknown type = falls back to existing routing (only real
    /// tables can reach key-position equality, downgrade is unreachable).
    pub(crate) primary_key_types: HashMap<String, ColumnType>,
    /// Table names whose CREATE TABLE wrote `PRIMARY KEY`. Keys are lowercase,
    /// same as `primary_keys`. Stays empty unless `set_explicit_primary_key` runs.
    pub(crate) explicit_primary_keys: std::collections::HashSet<String>,
    /// MS24 Iteration 000 (D1): Table name -> per-column declared DEFAULT
    /// literals (index-aligned with the registered column list; `None` = no
    /// default). Additive channel mirroring `set_pk_column_type`; consumed by
    /// `map_insert_values` for subset-INSERT / DEFAULT-keyword fill.
    pub(crate) table_defaults: HashMap<String, Vec<Option<Value>>>,
    /// MS24 Iteration 001 (D4): Table name -> columns carrying a UNIQUE
    /// index (ascending column position, the same order the catalog persists
    /// unique roots in). Additive channel consumed by the `ON CONFLICT`
    /// conflict-target resolution — a target column is arbitrable only if it
    /// is an INT declared PK or listed here.
    pub(crate) table_unique_columns: HashMap<String, Vec<usize>>,
    /// MS21 Iter002 (T2-g/D6): Table name -> registered secondary indexes
    /// (`(index name, column name)`, creation order — the catalog section
    /// order). Additive channel consumed by the non-PK equality routing arm.
    pub(crate) table_secondary_indexes: HashMap<String, Vec<(String, String)>>,
    /// MS21 Iter002 (T2-c/D14): Table name -> per-column declared types
    /// (index-aligned with the registered column list). Additive channel
    /// consumed by the CREATE INDEX INT-only check.
    pub(crate) table_column_types: HashMap<String, Vec<ColumnType>>,
    /// MS21 Iter002 (T2-c/T2-f/D14): every registered secondary index name in
    /// the library (lowercase). Injected per-statement at the pipeline's plan
    /// stage for CREATE INDEX (duplicate-name check) and DROP INDEX
    /// (existence check) — plan-time rejection, zero side effects.
    pub(crate) existing_index_names: std::collections::HashSet<String>,
    /// MS21 Iter003 (T3-b/D9): table name -> live-row flag, injected
    /// per-statement at the pipeline's plan stage for ALTER TABLE ... ADD
    /// COLUMN — the input to the `NOT NULL`-without-DEFAULT rejection.
    pub(crate) table_has_live_rows: HashMap<String, bool>,
    /// Set of inner table names when building a subquery (for detecting outer references).
    /// None when building a top-level query.
    pub(crate) inner_table_names: Option<Vec<String>>,
    /// MS11-T01 Iter001: 正在构建子查询计划（WHERE IN/EXISTS、标量子查询、
    /// 派生表）。子查询计划形状是 SemiJoin/SubqueryEval/DerivedScan 机制的
    /// 消费面——SELECT 表达式项的顶层 Projection 路由在子查询上下文抑制，
    /// 子查询计划保持既有行为。
    pub(crate) building_subquery: bool,
    /// T5（change 2026-10-04-external-review-gaps）：FROM 派生表上下文标志
    /// ——仅 FROM 派生臂置位；其余子查询入口（IN/EXISTS、标量绑定、SELECT
    /// 投影位、视图展开）一律复位 false。集合操作守卫据此区分「FROM 派生
    /// 放行」与「表达式/视图子查询维持拒绝」。
    pub(crate) building_from_derived: bool,
    /// MS09-T02 (I015): NLJ 组合行列布局（design D5a 加性通道）。Some 时
    /// `build_expression` 两个列解析臂优先消费——列引用解析为组合行绝对
    /// 索引（前序表偏移累计 + 表内位置）。仅在 NLJ 分支编译 ON 期间置位
    ///（save/restore 严格配对），None 时既有路径行为逐字节不变。
    pub(crate) join_column_layout: Option<Vec<(String, Vec<String>)>>,
    /// MS20 T3：附加库引用的限定扁平名集合（`"o.u"`）——由 pipeline 的路由
    /// 分支注入。五写消费点据此点名拒绝（附加库一期只读）。加性通道，与
    /// `set_pk_column_type` 同型；无注册会话时为空，主库写面行为逐字节不变。
    pub(crate) attached_tables: std::collections::HashSet<String>,
    /// MS25 T5（design D5）：语句级 CTE 注册表——`WITH` 定义按声明序累积，
    /// 引用点克隆定义计划为既有 `DerivedScanNode`（无新执行器 / 节点）。
    /// `PlanBuilder` 语句级生命周期天然隔离，定义间的前向引用即「后定义可见
    /// 先定义」的注册序。
    pub(crate) cte_plans: Vec<CteDef>,
    /// MS25 T5：正在构建的 CTE 名称栈（定义计划构建期间置位）——命中即
    /// 自引用 / 成环，点名拒绝。push/pop 严格配对。
    pub(crate) pending_cte_names: Vec<String>,
    /// MS21 Iter005 (T5-b/D12)：语句级视图注册表——持久化视图的名称、定义
    /// SQL 与列名清单，由 pipeline 的 plan 阶段按语句注入（`Database.views`
    /// 的快照）。CREATE VIEW 重名检查、定义内既有视图引用合法性（T5-b）与
    /// FROM/JOIN 展开钩子（T5-c）共同消费。加性通道：无注入（空表）时主库
    /// 面行为逐字节不变。
    pub(crate) views: HashMap<String, ViewDef>,
    /// design D4：FROM / JOIN 处声明的表别名 → `self.tables` 真实键（小写归一）。
    /// 与 `tables` 键空间分离——别名不参与表名查找，`attached_tables` 复合键
    /// （`"o.dept_map"`）等既有注册形态不被污染。四个限定名消费点经
    /// `resolve_table_qualifier` 单点归一，自连接同表两个别名天然各持一条。
    pub(crate) table_aliases: HashMap<String, String>,
    /// I080 一期（T02-C1）：绑定作用域表——FROM/JOIN 装配点按声明限定符
    /// 登记的段（列清单 + Option 类型/可空）。关联参数分类优先经作用域身份
    /// 查找，`inner_table_names` 名单降级为未登记段兜底（D6）。
    /// `binding_scope_boundary` 之前为外层累计段，之后为当前查询层的段。
    pub(crate) binding_scopes: Vec<binding::ScopeEntry>,
    /// I080 一期（T02-C1）：当前分类层的段起始界——子查询 / FROM 派生 /
    /// 视图展开臂进入时置为当时长度、退出恢复；界后段即当前（内层）作用域。
    pub(crate) binding_scope_boundary: usize,
    /// I080 一期（T02-C1）：表名 -> per-column NOT NULL 标志（index-aligned
    /// 与注册列清单）。加性通道同 `table_column_types` 先例，作用域段登记时
    /// 经 `table_binding_meta` 消费。
    pub(crate) table_not_null: HashMap<String, Vec<bool>>,
}

/// MS21 Iter005 (T5-b/D12)：一个已注册的持久化视图定义。列名清单 = 显式
/// 列清单（`CREATE VIEW v (a, b) AS ...`）或定义输出表头（创建期已定形，
/// 持久化于 `__columns` 行——列名-only 行）。
#[derive(Debug, Clone)]
pub(crate) struct ViewDef {
    pub(crate) sql: String,
    pub(crate) columns: Vec<String>,
}

/// MS25 T5：一个已注册的 CTE 定义（`WITH <名> [(列别名)] AS (<查询>)`）。
#[derive(Debug, Clone)]
pub(crate) struct CteDef {
    /// CTE 名（小写归一，与 `register_table` 键纪律一致）
    pub(crate) name: String,
    /// 输出列名（列别名列表优先，否则为定义计划的输出列）
    pub(crate) columns: Vec<String>,
    /// 定义计划（引用时克隆，来源为顶层上下文构建——定义内 ORDER BY / LIMIT /
    /// 集合操作 / 表达式项 / DISTINCT 经既有装配全量复用）
    pub(crate) plan: PhysicalPlan,
}

impl PlanBuilder {
    /// Create empty PlanBuilder
    pub fn new() -> Self {
        Self {
            tables: HashMap::new(),
            primary_keys: HashMap::new(),
            primary_key_types: HashMap::new(),
            explicit_primary_keys: std::collections::HashSet::new(),
            table_defaults: HashMap::new(),
            table_unique_columns: HashMap::new(),
            table_secondary_indexes: HashMap::new(),
            table_column_types: HashMap::new(),
            existing_index_names: std::collections::HashSet::new(),
            table_has_live_rows: HashMap::new(),
            inner_table_names: None,
            building_subquery: false,
            building_from_derived: false,
            join_column_layout: None,
            attached_tables: std::collections::HashSet::new(),
            cte_plans: Vec::new(),
            pending_cte_names: Vec::new(),
            views: HashMap::new(),
            table_aliases: HashMap::new(),
            binding_scopes: Vec::new(),
            binding_scope_boundary: 0,
            table_not_null: HashMap::new(),
        }
    }

    /// Register table metadata
    pub fn register_table(&mut self, name: &str, columns: Vec<String>, pk: &str) {
        let name_lower = name.to_lowercase();
        self.tables.insert(name_lower.clone(), columns);
        self.primary_keys.insert(name_lower, pk.to_lowercase());
    }

    /// MS16 Iteration 000 (I046): Register the key column's declared type (design D2 additive channel —
    /// `register_table` signature unchanged; the key is lowercase, consistent with `register_table`).
    pub fn set_pk_column_type(&mut self, name: &str, ct: ColumnType) {
        self.primary_key_types.insert(name.to_lowercase(), ct);
    }

    /// Record that `name`'s CREATE TABLE wrote `PRIMARY KEY`.
    /// The key is lowercase, consistent with `register_table`.
    /// `register_table` does not insert this set.
    pub fn set_explicit_primary_key(&mut self, name: &str) {
        self.explicit_primary_keys.insert(name.to_lowercase());
    }

    /// design D4：登记 FROM / JOIN 处声明的表别名 → `self.tables` 真实键。
    /// 键同 `register_table` 纪律小写归一；同名别名后写覆盖先写（SQL 中同一
    /// FROM 作用域内别名不重复，重复只可能来自子查询重建，不构成可达语义）。
    pub(crate) fn register_table_alias(&mut self, alias: &str, target_key: &str) {
        self.table_aliases
            .insert(alias.to_lowercase(), target_key.to_lowercase());
    }

    /// design D4：限定名单点归一——命中别名映射返回其真实键，否则 `None`
    /// 交回调用方按原样使用限定符（真实表名、CTE/视图 effective 名、派生表
    /// 别名、attach 复合键均走此分支）。四个消费点据此在 `self.tables`、
    /// NLJ 布局与 JOIN 输出列上用同一套真实名比较。
    pub(crate) fn resolve_table_qualifier<'a>(&'a self, qualifier: &'a str) -> Option<&'a str> {
        self.table_aliases.get(qualifier).map(|s| s.as_str())
    }

    /// Iteration 002 T7（R9）：两段附加 FROM 项（`o.u`）无显式别名时，以尾段
    /// 登记隐式关联名（`u` → `o.u`）——短名限定列经 `resolve_table_qualifier`
    /// 归一化后沿既有通道解析。登记条件：flat 名在 `attached_tables`（字面
    /// 含点主库表名不带该标记，不登记）且恰分两段。显式别名路径不调用本
    /// 方法，登记不发生（显式别名优先）。
    pub(crate) fn register_implicit_attached_short_name(&mut self, flat_name: &str) {
        let name = flat_name.to_lowercase();
        if self.attached_tables.contains(&name) {
            let parts: Vec<&str> = name.split('.').collect();
            if parts.len() == 2 {
                self.register_table_alias(parts[1], &name);
            }
        }
    }

    /// Iteration 002 T7（R9）：书写限定符与 FROM 臂声明限定符的匹配——原文
    /// 相等，或书写限定符经 `resolve_table_qualifier` 归一后命中该臂（隐式
    /// 短名 `u` ↔ 声明限定符 `o.u`）。显式别名的声明限定符即别名文本，原文
    /// 相等分支先行命中，既有行为零变化。
    pub(crate) fn qualifier_matches_declared(&self, written: &str, declared: &str) -> bool {
        declared.eq_ignore_ascii_case(written)
            || self
                .resolve_table_qualifier(written)
                .is_some_and(|key| key.eq_ignore_ascii_case(declared))
    }

    /// MS24 Iteration 000 (D1): Register the table's per-column declared
    /// DEFAULT literals (additive channel — `register_table` signature
    /// unchanged; the key is lowercase, consistent with `register_table`).
    /// The vector is index-aligned with the registered column list.
    pub fn set_table_defaults(&mut self, name: &str, defaults: Vec<Option<Value>>) {
        self.table_defaults.insert(name.to_lowercase(), defaults);
    }

    /// MS24 Iteration 001 (D4): Register the columns carrying a UNIQUE index
    /// (additive channel — `register_table` signature unchanged; the key is
    /// lowercase, consistent with `register_table`). The vector holds column
    /// positions in ascending order.
    pub fn set_table_unique_columns(&mut self, name: &str, columns: Vec<usize>) {
        self.table_unique_columns
            .insert(name.to_lowercase(), columns);
    }

    /// MS21 Iter002 (T2-g/D6): Register a table's secondary indexes
    /// (`(index name, column name)` pairs, creation order — additive channel,
    /// key lowercase like `register_table`).
    pub fn set_table_secondary_indexes(&mut self, name: &str, indexes: Vec<(String, String)>) {
        self.table_secondary_indexes
            .insert(name.to_lowercase(), indexes);
    }

    /// MS21 Iter002 (T2-c/D14): Register the table's per-column declared
    /// types (index-aligned with the registered column list — additive
    /// channel, key lowercase like `register_table`).
    pub fn set_table_column_types(&mut self, name: &str, types: Vec<ColumnType>) {
        self.table_column_types.insert(name.to_lowercase(), types);
    }

    /// I080 一期（T02-C1）: Register the table's per-column NOT NULL flags
    /// (index-aligned with the registered column list — additive channel,
    /// key lowercase like `set_table_column_types`).
    pub fn set_table_not_null(&mut self, name: &str, not_null: Vec<bool>) {
        self.table_not_null.insert(name.to_lowercase(), not_null);
    }

    /// MS21 Iter002 (T2-c/T2-f/D14): Inject the library-wide secondary index
    /// name set (plan_stage does the async gather before `build_plan`).
    pub fn set_existing_index_names(&mut self, names: std::collections::HashSet<String>) {
        self.existing_index_names = names;
    }

    /// MS21 Iter003 (T3-b/D9): inject the target table's live-row flag
    /// (plan_stage does the async scan before `build_plan`).
    pub fn set_table_has_live_rows(&mut self, name: &str, has_rows: bool) {
        self.table_has_live_rows
            .insert(name.to_lowercase(), has_rows);
    }

    /// MS21 Iter005 (T5-b/D12)：注入语句级视图注册表（`Database.views` 的
    /// 快照；plan 阶段做 async 读取后一次性置换）。键为小写归一视图名。
    pub(crate) fn set_views(&mut self, views: HashMap<String, ViewDef>) {
        self.views = views;
    }

    /// MS20 T3：标记一个限定名为附加库引用（pipeline 路由分支调用）。
    pub fn mark_attached_table(&mut self, name: &str) {
        self.attached_tables.insert(name.to_lowercase());
    }

    /// MS20 T3：写目标若为附加库引用则点名拒绝（附加库一期只读）。错误经
    /// MS19 `UnsupportedStatement` 通道携带能力名，SQLSTATE 分类与既有
    /// 能力面拒绝一致（0A000）。
    pub(crate) fn reject_attached_write(&self, table_name: &str) -> Result<(), PlanError> {
        if self.attached_tables.contains(&table_name.to_lowercase()) {
            return Err(PlanError::UnsupportedStatement(
                "WRITE TO ATTACHED DATABASE (attached databases are read-only)".into(),
            ));
        }
        Ok(())
    }

    /// MS21 Iter005 (T5-c/D12)：写目标若为视图名则计划期点名拒绝（视图
    /// 只读、非物化——无行可写）。D14：`UnsupportedStatement` 能力名通道
    /// （0A000），与 `reject_attached_write` 同位形。
    pub(crate) fn reject_view_write(&self, table_name: &str) -> Result<(), PlanError> {
        if self.views.contains_key(&table_name.to_lowercase()) {
            return Err(PlanError::UnsupportedStatement(
                "WRITE TO VIEW (views are read-only)".into(),
            ));
        }
        Ok(())
    }

    /// Build PhysicalPlan from Statement
    pub fn build_plan(&mut self, stmt: &Statement) -> Result<PhysicalPlan, PlanError> {
        // MS11-T02: transaction statements have no executor semantics on any
        // statement path — reject precisely before planning (boundary clauses
        // propagate their D3 message; clean statements get the session-only
        // message). They never reach the plan cache either way.
        if classify_transaction_statement(stmt)?.is_some() {
            return Err(PlanError::TransactionStatement(
                "transaction statements (BEGIN/COMMIT/ROLLBACK) are only supported in an rtsql CLI session".to_string(),
            ));
        }
        // MS20 T1: ATTACH is a CLI-session registration statement — the lib
        // execution paths have no attach registry, so reject it here (before
        // planning, hence before the plan cache) exactly like transaction
        // statements above. The CLI session dispatcher classifies it first and
        // consumes it without ever reaching this arm.
        if classify_attach_statement(stmt)?.is_some() {
            return Err(PlanError::AttachStatement(
                "ATTACH is only supported in an rtsql CLI session".to_string(),
            ));
        }
        match stmt {
            Statement::Query(query) => self.build_query(query),
            Statement::Insert {
                table_name,
                columns,
                source,
                on,
                replace_into,
                ..
            } => self.build_insert(table_name, columns, source, on, *replace_into),
            Statement::Update {
                table,
                assignments,
                selection,
                ..
            } => self.build_update(table, assignments, selection),
            Statement::Delete {
                from, selection, ..
            } => self.build_delete(from, selection),
            Statement::CreateTable {
                name,
                columns,
                constraints,
                query,
                like,
                clone,
                ..
            } => self.build_create_table(
                name,
                columns,
                constraints,
                query.as_deref(),
                like.as_ref(),
                clone.as_ref(),
            ),
            Statement::Drop {
                object_type,
                if_exists,
                names,
                ..
            } => {
                if *object_type == ObjectType::Table {
                    self.build_drop_table(names, if_exists)
                } else if *object_type == ObjectType::Index {
                    // MS21 Iter002 (T2-f)：DROP INDEX 分派（此前落非 Table
                    // 泛化拒绝臂）。
                    self.build_drop_index(names, if_exists)
                } else if *object_type == ObjectType::View {
                    // MS21 Iter005 (T5-d)：DROP VIEW 分派（此前落非 Table
                    // 泛化拒绝臂）。
                    self.build_drop_view(names, if_exists)
                } else {
                    Err(PlanError::UnsupportedStatement(format!(
                        "DROP {}",
                        object_type
                    )))
                }
            }
            Statement::CreateIndex { .. } => {
                // MS21 Iter002 (T2-c)：CREATE INDEX 分派（此前落 catch-all
                // UnsupportedStatement）。
                self.build_create_index(stmt)
            }
            Statement::AlterTable { .. } => {
                // MS21 Iter003 (T3-b)：ALTER TABLE 分派（此前落 catch-all
                // UnsupportedStatement）。
                self.build_alter_table(stmt)
            }
            Statement::Truncate { .. } => {
                // MS21 Iter004 (T4-a)：TRUNCATE 分派（此前落 catch-all
                // UnsupportedStatement）。
                self.build_truncate(stmt)
            }
            Statement::CreateView { .. } => {
                // MS21 Iter005 (T5-b)：CREATE VIEW 分派（此前落 catch-all
                // UnsupportedStatement）。
                self.build_create_view(stmt)
            }
            Statement::Explain {
                analyze,
                verbose,
                format,
                statement,
                ..
            } => {
                // MS25 T6: `EXPLAIN <SELECT>` 计划期构造被观测计划并包
                // `Explain` 包装节点（执行期零副作用）。修饰面与被观测语句族
                // 逐项点名拒绝（`describe_alias` 的 `DESC` / `DESCRIBE` 与
                // EXPLAIN 同语义，不拒绝）。
                if *analyze {
                    return Err(PlanError::UnsupportedStatement("EXPLAIN ANALYZE".into()));
                }
                if *verbose {
                    return Err(PlanError::UnsupportedStatement("EXPLAIN VERBOSE".into()));
                }
                if format.is_some() {
                    return Err(PlanError::UnsupportedStatement("EXPLAIN FORMAT".into()));
                }
                match statement.as_ref() {
                    Statement::Query(query) => {
                        let inner = self.build_query(query)?;
                        Ok(PhysicalPlan::Explain(ExplainNode {
                            input: Box::new(inner),
                        }))
                    }
                    _ => Err(PlanError::UnsupportedStatement(
                        "EXPLAIN of non-SELECT statements".into(),
                    )),
                }
            }
            _ => Err(PlanError::UnsupportedStatement(unsupported_statement_name(
                stmt,
            ))),
        }
    }

    /// Validate table exists
    fn validate_table(&self, table_name: &str) -> Result<(), PlanError> {
        let name_lower = table_name.to_lowercase();
        if self.tables.contains_key(&name_lower) {
            Ok(())
        } else {
            Err(PlanError::ParseError(format!(
                "Table '{}' does not exist",
                table_name
            )))
        }
    }
}

impl Default for PlanBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_builder_new() {
        let builder = PlanBuilder::new();
        assert!(builder.tables.is_empty());
        assert!(builder.primary_keys.is_empty());
    }

    #[test]
    fn test_register_table() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into(), "name".into()], "id");

        assert!(builder.tables.contains_key("users"));
        assert!(builder.primary_keys.contains_key("users"));
        assert_eq!(builder.primary_keys.get("users"), Some(&"id".to_string()));
    }

    #[test]
    fn test_validate_table() {
        let mut builder = PlanBuilder::new();
        builder.register_table("users", vec!["id".into()], "id");

        assert!(builder.validate_table("users").is_ok());
        assert!(builder.validate_table("nonexistent").is_err());
    }

    /// design D4：自连接两个别名各自归一到同一基表键；未注册的限定符与真实
    /// 表名逐字回传（既有点名文案与 `TableNotFound` 语义不变）。
    #[test]
    fn test_resolve_table_qualifier_normalizes_aliases() {
        let mut builder = PlanBuilder::new();
        builder.register_table("people", vec!["id".into(), "name".into()], "id");
        builder.register_table_alias("p", "people");
        builder.register_table_alias("q", "people");

        assert_eq!(builder.resolve_table_qualifier("p"), Some("people"));
        assert_eq!(builder.resolve_table_qualifier("q"), Some("people"));
        assert_eq!(builder.resolve_table_qualifier("people"), None);
        assert_eq!(builder.resolve_table_qualifier("nope"), None);
    }

    /// attach 复合键（`"o.dept_map"`）已按限定名注册，归一化不得改写它。
    #[test]
    fn test_resolve_table_qualifier_keeps_attach_composite_key() {
        let mut builder = PlanBuilder::new();
        builder.register_table("o.dept_map", vec!["emp".into()], "emp");
        builder.register_table_alias("p", "people");

        assert_eq!(builder.resolve_table_qualifier("o.dept_map"), None);
    }

    /// 别名注册与限定符查询同用标识符小写归一。
    #[test]
    fn test_register_table_alias_is_case_insensitive() {
        let mut builder = PlanBuilder::new();
        builder.register_table_alias("P", "people");

        assert_eq!(builder.resolve_table_qualifier("p"), Some("people"));
    }

    /// T7（R9）：两段附加 FROM 项无显式别名 → 尾段隐式关联名登记。
    #[test]
    fn test_implicit_short_name_registered_for_attached_two_segment() {
        let mut builder = PlanBuilder::new();
        builder.mark_attached_table("o.u");
        builder.register_implicit_attached_short_name("o.u");

        assert_eq!(builder.resolve_table_qualifier("u"), Some("o.u"));
    }

    /// T7（R9）：登记条件要求附加标记——字面含点主库表名（未 mark）不登记，
    /// 短名维持 `TableNotFound` 既有点名面。
    #[test]
    fn test_implicit_short_name_requires_attached_marker() {
        let mut builder = PlanBuilder::new();
        builder.register_implicit_attached_short_name("o.u");

        assert_eq!(builder.resolve_table_qualifier("u"), None);
    }

    /// T7（R9）：显式别名登记不被隐式登记覆盖（显式别名优先——调用点仅在
    /// 无显式别名时调用隐式登记；此处固定同作用域内二者并存时显式条目不变）。
    #[test]
    fn test_implicit_short_name_keeps_explicit_alias() {
        let mut builder = PlanBuilder::new();
        builder.mark_attached_table("o.u");
        builder.register_table_alias("x", "o.u");
        builder.register_implicit_attached_short_name("o.u");

        assert_eq!(builder.resolve_table_qualifier("x"), Some("o.u"));
        assert_eq!(builder.resolve_table_qualifier("u"), Some("o.u"));
    }

    /// T7（R9）：匹配 helper——原文相等与归一化命中两分支，未登记限定符
    /// 不命中。
    #[test]
    fn test_qualifier_matches_declared_branches() {
        let mut builder = PlanBuilder::new();
        builder.mark_attached_table("o.u");
        builder.register_table_alias("x", "o.u");
        builder.register_implicit_attached_short_name("o.u");

        // 原文相等（显式别名 / 全限定）。
        assert!(builder.qualifier_matches_declared("x", "x"));
        assert!(builder.qualifier_matches_declared("o.u", "o.u"));
        // 归一化命中（隐式短名）。
        assert!(builder.qualifier_matches_declared("u", "o.u"));
        // 未登记限定符不命中。
        assert!(!builder.qualifier_matches_declared("z", "o.u"));
    }
}

#[cfg(test)]
mod attach_statement_tests {
    use super::*;
    use crate::parser::parse_sql;

    /// MS20 T1：ATTACH 分类器与 `build_plan` 前置拒绝（`sql-attach-session`
    /// R1/R3 的库内面）。CLI 会话臂在 `src/cli/mod.rs` 消费本分类器。
    fn classify(sql: &str) -> Result<Option<AttachInfo>, PlanError> {
        let statements = parse_sql(sql).expect("parse must succeed");
        assert_eq!(statements.len(), 1);
        classify_attach_statement(&statements[0])
    }

    #[test]
    fn clean_attach_statements_extract_path_and_alias() {
        for (sql, path, alias) in [
            ("ATTACH 'other.db' AS o", "other.db", "o"),
            ("ATTACH DATABASE 'other.db' AS o", "other.db", "o"),
            ("ATTACH '/abs/other.db' AS o", "/abs/other.db", "o"),
        ] {
            match classify(sql).unwrap() {
                Some(info) => {
                    assert_eq!(info.path, path, "{sql}");
                    assert_eq!(info.alias, alias, "{sql}");
                }
                None => panic!("{sql}: expected Some(AttachInfo)"),
            }
        }
    }

    /// 别名按标识符归一化规则小写处理（`table-name-resolution` 同型）。
    #[test]
    fn attach_alias_is_lowercased() {
        match classify("ATTACH 'other.db' AS OtherDB").unwrap() {
            Some(info) => assert_eq!(info.alias, "otherdb"),
            None => panic!("expected Some(AttachInfo)"),
        }
    }

    /// 路径仅接受单引号字符串字面量；其他表达式形态点名拒绝。
    #[test]
    fn non_literal_path_rejected_with_named_message() {
        for (sql, message) in [
            (
                "ATTACH other AS o",
                "ATTACH database path must be a string literal",
            ),
            (
                "ATTACH 'a' || 'b' AS o",
                "ATTACH database path must be a string literal",
            ),
        ] {
            match classify(sql) {
                Err(PlanError::AttachStatement(msg)) => assert_eq!(msg, message, "{sql}"),
                other => panic!("{sql}: expected AttachStatement, got {:?}", other),
            }
        }
    }

    #[test]
    fn non_attach_statements_classify_to_none() {
        for sql in [
            "SELECT 1",
            "INSERT INTO t VALUES (1)",
            "BEGIN",
            "CREATE TABLE t (id INT)",
            "DROP TABLE t",
        ] {
            assert_eq!(classify(sql).unwrap(), None, "{sql}");
        }
    }

    /// R3：lib 非会话路径在 `build_plan` 前置拒绝，先于 plan cache。
    #[test]
    fn build_plan_rejects_attach_with_session_message() {
        for sql in ["ATTACH 'other.db' AS o", "ATTACH DATABASE 'other.db' AS o"] {
            let statements = parse_sql(sql).unwrap();
            let mut builder = PlanBuilder::new();
            match builder.build_plan(&statements[0]) {
                Err(PlanError::AttachStatement(msg)) => assert_eq!(
                    msg, "ATTACH is only supported in an rtsql CLI session",
                    "{sql}"
                ),
                other => panic!("{sql}: expected AttachStatement, got {:?}", other),
            }
        }
    }

    /// 非字面量路径在任何执行面上都非法：分类器先于 session-only 拒绝，
    /// `build_plan` 原样透出该点名消息（不谎报为「仅会话可用」）。
    #[test]
    fn build_plan_reports_path_form_rejection_for_non_literal_path() {
        let statements = parse_sql("ATTACH other AS o").unwrap();
        let mut builder = PlanBuilder::new();
        match builder.build_plan(&statements[0]) {
            Err(PlanError::AttachStatement(msg)) => {
                assert_eq!(msg, "ATTACH database path must be a string literal")
            }
            other => panic!("expected AttachStatement, got {:?}", other),
        }
    }
}

#[cfg(test)]
mod tx_statement_tests {
    use super::*;
    use crate::parser::parse_sql;

    fn classify(sql: &str) -> Result<Option<TxStatementKind>, PlanError> {
        let statements = parse_sql(sql).expect("parse must succeed");
        assert_eq!(statements.len(), 1);
        classify_transaction_statement(&statements[0])
    }

    #[test]
    fn clean_transaction_statements_classify_to_kind() {
        for (sql, expected) in [
            ("BEGIN", TxStatementKind::Begin),
            ("BEGIN TRANSACTION", TxStatementKind::Begin),
            ("START TRANSACTION", TxStatementKind::Begin),
            ("COMMIT", TxStatementKind::Commit),
            ("ROLLBACK", TxStatementKind::Rollback),
        ] {
            assert_eq!(classify(sql).unwrap(), Some(expected), "{sql}");
        }
    }

    #[test]
    fn boundary_clauses_rejected_with_named_message() {
        for (sql, message) in [
            (
                "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
                "SET TRANSACTION is not supported",
            ),
            ("SAVEPOINT sp1", "SAVEPOINT is not supported"),
            (
                "RELEASE SAVEPOINT sp1",
                "RELEASE SAVEPOINT is not supported",
            ),
            (
                "BEGIN ISOLATION LEVEL SERIALIZABLE",
                "transaction modes in BEGIN/START TRANSACTION are not supported",
            ),
            (
                "START TRANSACTION READ ONLY",
                "transaction modes in BEGIN/START TRANSACTION are not supported",
            ),
            ("COMMIT AND CHAIN", "COMMIT AND CHAIN is not supported"),
            ("ROLLBACK AND CHAIN", "ROLLBACK AND CHAIN is not supported"),
            (
                "ROLLBACK TO SAVEPOINT sp1",
                "ROLLBACK TO SAVEPOINT is not supported",
            ),
        ] {
            match classify(sql) {
                Err(PlanError::TransactionStatement(msg)) => {
                    assert_eq!(msg, message, "{sql}");
                }
                other => panic!("{sql}: expected TransactionStatement, got {:?}", other),
            }
        }
    }

    #[test]
    fn non_transaction_statements_classify_to_none() {
        for sql in [
            "SELECT 1",
            "INSERT INTO t VALUES (1)",
            "UPDATE t SET id = 1",
            "DELETE FROM t",
            "CREATE TABLE t (id INT)",
            "DROP TABLE t",
        ] {
            assert_eq!(classify(sql).unwrap(), None, "{sql}");
        }
    }

    #[test]
    fn build_plan_rejects_transaction_statements_with_session_message() {
        for sql in ["BEGIN", "COMMIT", "ROLLBACK"] {
            let statements = parse_sql(sql).unwrap();
            let mut builder = PlanBuilder::new();
            match builder.build_plan(&statements[0]) {
                Err(PlanError::TransactionStatement(msg)) => assert_eq!(
                    msg,
                    "transaction statements (BEGIN/COMMIT/ROLLBACK) are only supported in an rtsql CLI session",
                    "{sql}"
                ),
                other => panic!("{sql}: expected TransactionStatement, got {:?}", other),
            }
        }
    }
}
