# Design: 子查询别名限定符绑定修复与执行器错误前缀单层化

## 根因链（代码级 Confirmed，2026-10-15 Plan 调查）

四个 issue 共一条根因链 + 一个独立文案缺陷：

1. **别名丢弃**：`src/parser/planner/subquery.rs:376-396` `extract_subquery_table_names` 匹配 `TableFactor::Table { name, .. }` 与 join 臂同款模式，**`..` 丢弃了 `alias` 字段**。输出集只含基表名（`FROM c x` → `["c"]`）。消费方共五处：`try_build_where_subquery` IN 臂（:41）、EXISTS 臂（:101）、`build_scalar_subquery_binding`（:177）、`query.rs:1428` 投影位路径，以及分类本体。
2. **双重误判**：内层表达式构建期 `self.inner_table_names` 非空时，`qualified_column_expression`（`src/parser/planner/expression.rs:263-271`）把限定符 ∉ 内层集的任何 `t.col` 改写为 `ParameterExpression("{t}.{col}")`——`x.age` 因此变成幻影相关参数；计划期 `extract_correlated_params`/`collect_outer_column_refs`（`subquery.rs:404-542`）以同一缺别名集做一致误判 → SemiJoin/ScalarSubqueryFilter/SubqueryEval 全部携带幻影参数走「相关」路径。
3. **静默绑错**：三执行器的 `extract_param_values` 按**裸列名**查外层列索引表，未命中 `.unwrap_or(0)` 静默取外层首列（`semi_join.rs:151-166`、`scalar_subquery_filter.rs:86-102`、`subquery_eval.rs:160-175`）。幻影参数（`x.age` → outer_column=`age`）因此获得首列值或同名错列值。四种表象全部由此推出并逐一与实测吻合：
   - 双幻影参数同名同值 → 碰巧正确（repro 库 D1/P2/P3/P6/P7 全属此类）；
   - 外层无同名列 → 首列值注入，`= 'Lin'` 恒假 → 恒空集（ISS17 形态 1、ISS18、demo 相关 EXISTS 全拒）；
   - 注入首列值与字面量偶合 → 只放行部分外层行（ISS17 形态 2 的 `{1}`、EXPLAIN 无关）；
   - 相关经外层非 PK 列（`t.ref`）与幻影参数落在不同列 → 谓词恒真全放行（ISS17 形态 3 的 `{1,2,9}`）。
   - ISS16：WHERE 形态注入后子查询行数≠1 但最小形状下计数恒 2 ≠ 1 → 静默空集；select list 形态走 SubqueryEval，注入失败后子查询未收窄 → 2 行触发 `SubqueryReturnsMultipleRow`（v1）或未注入未报错但结果 NULL/塌缩（demo）。
4. **ISS19（独立）**：`src/storage/error.rs:111` `#[error("execution error: {0}")]` 自带小写前缀，`src/pipeline.rs:1300` `format!("Execution error: {}", e)` 再加大写前缀 → 双层。`tests/subquery_test.rs:632` 锁定了该形态。ISS11 修复的是 parse/plan 漏斗，本点为执行器 Result 上浆路径漏网。

## 修复设计与选择理由

### T1：声明限定符提取（核心修复）

`extract_subquery_table_names` 改为输出每个 FROM 项的**声明限定符**：`alias.name`（`Option<TableAlias>` 命中）否则基表名；JOIN 臂同规则；`TableFactor::Derived` 取 `alias.name`（与 `query.rs:386/430` FROM 注册路径同信息源，派生表在本引擎语义下必有别名，无别名按基表名规则不输出——以现场字段为准）。

- **理由**：消费方五处全部经这一函数，单点修复自动一致；`qualified_column_expression` 的后续解析臂（`resolve_table_qualifier` 别名→真实键归一）本就支持别名限定，分类修正后别名引用沿既有通道解析为内层列，无新增机制。
- **拒绝的替代**：(a) 在 `qualified_column_expression` 单独 consult FROM 注册表——分类与解析双源，不一致风险高；(b) 执行器侧按 `{alias}.{col}` 回查内层计划——把计划期错误推迟到运行期，违背「计划期点名」既定语义。
- 大小写：两消费方均已 `eq_ignore_ascii_case` 比较，声明限定符原样输出即可。
- 派生表：`has_outer_refs_outside` 多级相关检测同函数受益，派生表别名限定的外层引用分类同步修正（既有拒绝面保持：多级相关仍拒绝）。

### T2：绑参失败显式报错

三处 `extract_param_values` 未命中从 `unwrap_or(0)` 改为返回 `Err(StorageError::ExecutionError("correlated parameter column not found in outer row: {column}"))`（签名改 `Result` 并上浆）。

- **理由**：消除「静默错结果」的最后退路；T1 后正常 SQL 不可触达（幻影参数消失），本项仅面向未来漏分类，把错误面从数据错误转为显式报错。沿用 58000 兜底 SQLSTATE，不新增分类（A2）。
- **拒绝的替代**：保留回落并记 warning——无日志通道，静默性不变；绑定失败跳过该行——语义更错。

### T3：单层前缀

`StorageError::ExecutionError` 的 thiserror 文案改为 `"{0}"`；`pipeline.rs:1300` 保持唯一前缀点。`tests/subquery_test.rs:632` 锚点随文案校准（单行断言更新）。

- **理由**：与 ISS11 既定「漏斗点单层赋予」同手法；其他 `StorageError` 变体前缀（IO error 等）不经该漏斗叠加，零影响。

### 影响面与兼容性

- 计划形状：修复后别名内层的 `EXISTS/IN` 走非相关快路径（`correlated_params` 空），`EXPLAIN` 的 `correlated_params=N` 从虚高值回落为真实值——属错误面纠正，非行为回归；e2e 若有断言锁定幻影相关形状需一并校准（T4 先行测试暴露）。
- 缓存：`correlated-subquery-cache` 语义（按参数值序缓存）在幻影参数消失后缓存键集合变化，既有缓存 requirement 的零回归场景由全量测试承载。
- 错误面：T2 新增一条不可经正常 SQL 触达的报错路径；T3 收窄一条文案。退出码不变。

## 实现顺序与依赖

T1（核心修复 + 形状覆盖矩阵先行测试 RED）→ T2（加固，依赖 T1 后幻影参数不可经 SQL 产生、单测直构造执行器）→ T3（文案，独立，可与 T2 并行）→ T4（e2e 见证，依赖 T1–T3 定稿）。全部落地后全量 `cargo test` 一次（Iteration Gate）。

## 测试覆盖缺口归因与同类面审计（2026-10-15，用户指令并入本 change）

**归因**：缺陷潜伏非执行器之过，是形状覆盖缺口。既有 57 个子查询用例的内层表**零别名**——全部以基名限定（`dept.region`）或内层非限定引用编写，缺陷所在的「内层别名限定 → 幻影参数」路径从未被任何用例进入；相关用例外层全部单键列/PK 关联，幻影参数与真实参数同名同值，`unwrap_or(0)` 回落恰好注入正确值（「碰巧正确」区）；AntiJoin 面无别名覆盖；`subquery_test.rs:632` 把 ISS19 双层文案锁定为期望形态，测试在主动保护 bug。机制：测试按当期实现形状补写，无「声明限定符 × 算子 × 相关性 × 内层形态 × 索引」矩阵思维。

**同类面审计结论**（同日基线探针，均判别式期望、非碰巧正确）：

| 新形状 | 预修复实测 | 期望 | 判定 |
|---|---|---|---|
| `NOT EXISTS (SELECT 1 FROM b x WHERE x.aid = a.id)` | {1,2} 全放行 | {}（全覆盖） | 同根因（AntiJoin 臂） |
| `NOT IN (SELECT x.aid FROM b x WHERE x.aid = a.id)` | {1,2} 全放行 | {} | 同根因 |
| `EXISTS (SELECT 1 FROM b x JOIN d y ON ... WHERE x.aid=1 AND y.v='x')` | 恒空集 | {1} | 同根因（JOIN 臂别名） |
| `EXISTS (SELECT 1 FROM (SELECT id FROM b) u WHERE u.id = 1)` | {1} 截断 | {1,2} | 同根因（Derived 别名，`extract` 不产出 Derived 限定符） |
| `IN` 别名无索引的 Literal 谓词形状 | 判别式实测正确 | — | 碰巧正确区，仍入矩阵钉住 |

全库 grep 确认锁定 bug 的断言仅 `:632` 一处；e2e 无子查询 journey。以上四个新形状由 T1 的同一函数修复自动覆盖（JOIN 臂与 Derived 别名进入声明限定符集），无需额外产品代码变更；修复载体是 T1 先行测试矩阵与 T4 e2e 形状的维度扩充（已在任务契约列明）。
