//! PlanBuilder 绑定作用域表（I080 一期 / T02-C1）。
//!
//! FROM/JOIN 装配点按声明限定符登记的段（列清单 + Option 类型/可空）构成
//! 作用域表；关联参数分类（`qualified_column_expression` 优先级 1）优先经
//! 作用域身份查找，`inner_table_names` 字符串名单降级为未登记段兜底。
//! `binding_scope_boundary` 之前为外层作用域（累计段），之后为当前（内层）
//! 查询层的段——子查询 / FROM 派生 / 视图展开臂进入时置界，退出截断恢复，
//! 嵌套构建的内层段不外泄到外层分类面。

use super::PlanBuilder;
use crate::storage::page_format::ColumnType;

/// 一个已登记的作用域段——声明限定符（有别名即别名，否则 effective 表名）
/// 与该臂的列清单。类型/可空仅真实表可自既有加性通道取得；派生/CTE/视图段
/// 为 `None`（计划期未知，诚实记录，不猜测）。
///
/// `types`/`not_null` 本 Iteration（001/002）只登记不消费——消费方在后续批次
/// （类型/可空），R1 要求绑定结果携带全部四元信息，定点压制 dead_code。
/// `columns` 自 Iteration 002 起被消费——JOIN 组合行布局段自作用域表窗口
/// 派生（段列清单直取）。
#[derive(Debug, Clone)]
pub(crate) struct ScopeEntry {
    pub(crate) qualifier: String,
    pub(crate) columns: Vec<String>,
    #[allow(dead_code)]
    pub(crate) types: Option<Vec<ColumnType>>,
    #[allow(dead_code)]
    pub(crate) not_null: Option<Vec<bool>>,
}

/// 关联外层引用判定单源（001-replan Guidance ①）——分类（live 作用域命中，
/// `qualified_column_expression`）与收集（段名快照命中，`collect_outer_column_refs`）
/// 两消费点共用唯一公式，两侧判定不再各自维护基准。
///
/// 入参：`inner_hit` 书写限定符命中当前（内层）段；`outer_hit` 命中外层累计
/// 段；`in_declared_list` 命中 `inner_table_names` 声明名单。与旧分类条件
/// `!inner_hit && (outer_hit || !in_list)` 仅在「名单外 ∧ 内层命中」格不同
/// （无别名派生 `derived.k` 病理格），该格两侧可观察行为均为收集侧执行期
/// 错误（001-replan 探针基线见证），语义中立。
pub(crate) fn is_outer_reference(inner_hit: bool, outer_hit: bool, in_declared_list: bool) -> bool {
    !in_declared_list || (!inner_hit && outer_hit)
}

impl PlanBuilder {
    /// 登记一个作用域段（FROM/JOIN 装配点在既有注册调用旁同步调用）。
    pub(crate) fn register_binding_scope(
        &mut self,
        qualifier: String,
        columns: Vec<String>,
        types: Option<Vec<ColumnType>>,
        not_null: Option<Vec<bool>>,
    ) {
        self.binding_scopes.push(ScopeEntry {
            qualifier,
            columns,
            types,
            not_null,
        });
    }

    /// 真实表段的类型/可空取值——键经小写归一后自既有加性通道查；未注册
    /// （计划期不可达面）返回 `None`。
    pub(crate) fn table_binding_meta(
        &self,
        table_name: &str,
    ) -> (Option<Vec<ColumnType>>, Option<Vec<bool>>) {
        let key = table_name.to_lowercase();
        (
            self.table_column_types.get(&key).cloned(),
            self.table_not_null.get(&key).cloned(),
        )
    }

    /// 作用域段命中判定——书写限定符与段声明限定符原文大小写不敏感相等。
    /// 刻意不使用 `resolve_table_qualifier` 别名归一：外层别名驻留同一别名
    /// 注册表，归一会把外层限定符误命中内层段；分类等价性要求内层命中集
    /// ⊆ 名单判定放行集，而名单判定为原文相等。
    pub(crate) fn binding_scope_hit(&self, written: &str, start: usize, end: usize) -> bool {
        self.binding_scopes[start..end]
            .iter()
            .any(|e| e.qualifier.eq_ignore_ascii_case(written))
    }

    /// 截断作用域表至快照长度（子查询 / FROM 派生 / 视图展开臂退出时恢复
    /// 外层累计段）。
    pub(crate) fn restore_binding_scopes(&mut self, len: usize) {
        self.binding_scopes.truncate(len);
    }

    /// 作用域窗口——自 `entry` 起的段切片（声明序）。`entry` 之前的段不在
    /// 窗口内：外层作用域段、CTE 定义内部残留段、投影臂临时登记段。
    ///
    /// 窗口末段为本步 JOIN 右臂段（右臂段在 ON 编译前登记），其余段为左链
    /// 各臂段——`build_from_clause_with_projection` 据此派生组合行布局段与
    /// JOIN 侧别判定（Iteration 002 段键单源）。
    pub(crate) fn binding_scope_window(&self, entry: usize) -> &[ScopeEntry] {
        &self.binding_scopes[entry..]
    }
}

/// 作用域窗口的布局物化——段（声明限定符 + 列清单）转为组合行布局同形的
/// `(限定符, 列清单)` 列表。物化后与作用域表的后续增删解耦：子查询臂构建
/// 期的作用域截断不影响已物化副本（窗口快照纪律）。
pub(crate) fn scope_window_layout(window: &[ScopeEntry]) -> Vec<(String, Vec<String>)> {
    window
        .iter()
        .map(|e| (e.qualifier.clone(), e.columns.clone()))
        .collect()
}
