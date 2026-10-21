# Design: MS30 SQL 语义绑定层一期（差分测试保护网 + JOIN/关联子查询收口）

- Change: 2026-10-20-ms30-semantic-binding-differential-net
- Date: 2026-10-20
- 调查基线：master @ 4190580，工作区 clean，全量基线 2030 tests / 0 failed / 2 ignored（SNAPSHOT 2026-10-15 AntiJoin 批次位，本会话采信；Iteration 000 落地时 Gate 全量复核为准）

## 1. Current-State Evidence（2026-10-20 实际代码读取）

### 1.1 名字解析的当前机制（I080 根因的实锤定位）

列引用解析收敛在 `src/parser/planner/expression.rs` 两个入口，均按「上下文标志优先级」逐级判定：

- `qualified_column_expression`（`expression.rs:258`，两段/三段限定列共用）优先级：
  1. `inner_table_names: Option<Vec<String>>`（`mod.rs:186`）Some 时，书写限定符不在内层名单 → 直接判为**相关外层引用**，产出 `ParameterExpression("qualifier.column")`（`expression.rs:264-272`）——字符串集成员资格判定，不查表/列是否真实存在；
  2. `join_column_layout: Option<Vec<(String, Vec<String>)>>`（`mod.rs:201`）Some 时按声明限定符段定位列（`expression.rs:281-299`）；
  3. 否则 `resolve_table_qualifier` 别名归一 → `self.tables` 字符串键查列位置（`expression.rs:305-318`）。
- `build_expression` 裸名臂（`expression.rs:337-378`）：布局 Some 时全段搜索（多命中 → `AmbiguousColumn`）；否则按当前 `table_name` 参数（调用方传入的「当前表」上下文）查列。

`inner_table_names` 的名单由 `extract_subquery_table_names`（`subquery.rs:381`）自子查询 FROM 的**声明限定符**（`declared_qualifiers`，`subquery.rs:404`——别名与基表名并列，2026-10-15 A1 豁免形态）静态提取；`collect_outer_column_refs`（`subquery.rs:449`）以同一字符串名单做相关参数分类。即 ISS16–ISS18 修复后，分类仍然依赖「提取名单与求值上下文一致」的约定，只是名单本身补齐了限定符——I080 所述「依赖字符串限定符与构建上下文」的现状：**分类正确性依赖两个独立提取器（`extract_subquery_table_names` 提取名单、`build_query` 实际注册表）对同一 FROM 子句产出一致的结论**，二者由约定耦合，无结构保证。

`join_column_layout` 的置位点共 6 处（save/restore 严格配对）：NLJ ON 编译（`query.rs:802-821`，RIGHT 交换臂布局按交换后输入序）、WHERE×JOIN（`query.rs:2004-2006`）、顶层 Projection 编译（`query.rs:1831`）、别名排序键编译（`query.rs:2818-2837`）、聚合 `extract_group_key`/HAVING 路径（`query.rs:1831` 同点复用）与 no-FROM 空布局（`query.rs:3055-3080`）。布局段键是**声明限定符**字符串，`qualifier_matches_declared`（`mod.rs:321`）做原文相等或别名归一化命中。

### 1.2 JOIN 侧别与条件键

Hash JOIN 路径：`extract_join_conditions`（`ddl_dml.rs:181`）+ `expr_join_side`（`ddl_dml.rs:263`）按书写限定符与声明限定符集匹配定侧，双腿同侧 `InvalidQuery` 点名；`resolve_column_ref`（`expression.rs:24`）产出 `ColumnRef { table: Option<String>, column }`。执行器侧 `condition_column_index`（`src/executor/join.rs:75`）在**执行期**按 `"table.column"` 字符串拼接查 `HashMap<String, usize>`（pipeline 装配点注册的限定键），裸名回退 `map.get(column)`——计划期已知的信息在执行期重新按名解析一次，是 I080 点名的「执行期按名定位点」。

### 1.3 库结果面（差分对比的取数通道）

`Database::execute_sql(&str) -> Response`（`database.rs:423`，委托 `pipeline::execute`）；`Response::QueryResult { rows: Vec<Vec<serde_json::Value>> }`（`src/network/protocol.rs:34`）——行值经 `value_to_json`（`pipeline.rs:1931`）：Int→Number、Float→Number（NaN/Inf→Null）、String/Bool→原样、Date/Timestamp→DA5 字符串、Null→Null。**QueryResult 不携带表头**（列名经 CLI/协议层渲染）——差分归一化在 serde_json::Value 层做即可，不依赖表头；但查询设计需保持输出列位置稳定。

rusqlite 侧取数通道：`rusqlite::Connection`（in-memory，`sqlite_compare.rs` bench 先例），行值以 `rusqlite::types::Value` 枚举返回（Null/Integer/Real/Text/Blob）。

### 1.4 测试基建

- dev-dependencies：rusqlite 0.31（无 bundled feature 声明）、tempfile、rand 0.8（主依赖，dev 可用）、serde_json、libc、which、criterion。**无 proptest/quickcheck**——性质测试以固定种子 + `rand` + 手写断言落地，不新增依赖。
- 测试形态先例：集成测试 `#[tokio::test]` + `Database` + `Response` helpers（`tests/subquery_test.rs:9-40` 同款三件套，全部既有文件复用此形态）；执行器级单测内联 `#[cfg(test)]` + `StubRowExecutor`（semi_join/scalar_subquery_filter/subquery_eval/anti_join 四处先例）；全量零回归 Gate 先例：分批串行（M19 分批模式，历次 Iteration Gate 均用）。
- 全量基线 2030 tests / 0 failed / 2 ignored；运行时间秒级到分钟级（本 change 000 Gate 实测记录；Runbook R31 纪律：正常应在秒级到低分钟级）。
- B-Tree 公共 API：`IndexManager`（`src/storage/btree/index_manager.rs:62`——`insert(key, row_id)`/`search`/`search_all`/`delete_exact`，async、经 BufferPool）；编码往返：`serialize_tuple`/`deserialize_tuple`/`deserialize_tuple_width`/`compute_tuple_size`（`src/storage/page_format/tuple.rs:58-204`，纯函数、同步、无需 BufferPool）。
- CLI/协议渲染面已有 e2e 套件（`tests/e2e_test.rs` 27 例）——差分网不重复渲染面验证。

### 1.5 差分落点风险（设计输入）

- 类型系统差异：SQLite type affinity（TEXT 可存 INT）vs RTsql 严格类型门——差分数据集与查询子集必须避开双方类型语义不同的输入（如向 INT 列插字符串）。子集以「双方声明类型语义一致」为边界：INT/TEXT/REAL(FLOAT)/BOOL(以 INTEGER 0/1 语义参与时不比较渲染)/NULL、日期族以 TEXT 形态参与（SQLite 无 DATE 类型，比较按字符串字典序——RTsql DA5 字典序=时间序，字符串形态下与 SQLite 逐字可比）。
- 浮点：RTsql `value_to_json` 与 SQLite Real 经 serde_json/`rusqlite::types::Value::Real` 都是 f64；JSON Number 与 f64 直接比较，不做字符串化（排除 serde_json 序列化差异——`from_f64` 对 0.1 与 0.30000000000000004 等敏感形态的输出由数值相等判定绕过）。
- 排序：无 ORDER BY 的行序两引擎不保证一致——归一化统一做「行排序后比较」（每行序列化为可比较元组，行间排序，消除输出顺序差异；有 ORDER BY 的查询同样先排序后比，等价谓词/JOIN 默认无序面因此可比，ORDER BY 语义的严格验证靠固定断言用例——设计 D3）。
- LIMIT 无 ORDER BY：非确定行选择，子集排除该组合（LIMIT 恒配 ORDER BY）。
- 除零/溢出：SQLite 整数溢出转 Real、除零返回 NULL；RTsql 算术为报错或不同语义——子集排除裸除法（除法仅出现在分母恒非零的受控构造，或直接排除除法算子）。
- `group_concat`：SQLite 默认分隔符 `,` 与 RTsql 一致（已核对既有语义），在子集内。
- 字符串函数：`substr`/`trim`/`upper`/`lower`/`length` SQLite 语义与 RTsql MS11-T03 对齐（RTsql 以 SQLite 边缘为先例实现），在子集内；`round`/`abs` 数学函数同（MS13 对齐 SQLite 饱和语义）。
- RTsql 拒绝面（JOIN 形 IN 子查询、多级相关子查询、WHERE×JOIN 子查询、外连接 Hash 路径等）不进入差分子集——差分只比较**双方都接受**的语句；拒绝面由既有回归锁定。
- 相关子查询：RTsql 语义 =「外层引用按限定符分类、参数按名注入」；SQLite 相关子查询语义标准。别名/派生表/谓词变体组合正是历史缺陷集中面，全部在子集内。

## 2. 目标行为

### 2.1 T01 差分与性质测试网（Iteration 000）

新增 `tests/sql_differential_test.rs`（差分 + 等价变换）与 `tests/structural_property_test.rs`（结构性质），全部为普通 `#[tokio::test]`/`#[test]` 集成测试：

- **固定种子数据集**：`rand::rngs::StdRng::seed_from_u64` 派生固定数据（3-4 表、每表 ~200-400 行，覆盖 NULL、重复值、同名列、自连接形状、日期族字符串、长字符串边界 ~4KB 与 >65535B 各少量）；同构 DDL+INSERT 序列灌入 RTsql（lib API）与 SQLite（rusqlite in-memory）。
- **差分子集 Q1-Q7**（查询清单内嵌于测试文件、每条一句）：投影/别名（含 `SELECT t.col AS a`、`*`）、WHERE 谓词组合（比较/AND/OR/NOT/BETWEEN/IN list/LIKE/IS NULL）、JOIN（INNER 等值含自连接别名、≥3 表链式）、聚合（GROUP BY 列/表达式键、HAVING、COUNT/SUM/AVG/MIN/MAX、COUNT(DISTINCT)、group_concat）、标量子查询（WHERE 比较 + 投影位、相关与非相关）、IN/EXISTS 子查询（相关与非相关、内层别名形态）、派生表/CTE 包裹、ORDER BY（含 LIMIT+ORDER BY）、DISTINCT、集合操作（UNION/UNION ALL/INTERSECT/EXCEPT）。
- **等价变换**：每条基查询生成 2-3 个变体（别名改写/派生表包裹/谓词交换与重排/CTE 改写），先在 **RTsql 内部**互比（变体 vs 原查询归一化一致），再与 SQLite 差分。
- **归一化**：测试内普通函数——每行映射为可比较元组（Null/Int/Real/Text/Bool 五类、日期族归 Text）、行间稳定排序、逐行逐列数值相等判定（Bool 与 Int 0/1 语义在数据集构造期即消歧：BOOL 列的值在 SQLite 建表用 INTEGER 0/1、RTsql 用 BOOL，比较时 Bool(b)==Integer(0|1) 归一化折叠）。
- **白名单**：`KNOWN_DIVERGENCES: &[(&str, &str, &str)]`（查询 ID、描述、原因分类）常量；差分循环对命中白名单的查询只断言「当前形态已被白名单覆盖」（记录性断言），白名单外分歧直接 panic 带双方行集摘要。初始白名单预计为空或极少——子集边界本身已排除已知差异；差分网首次运行发现的真实差异按 A4 处置（收口面内修复候选 / 范围外 Issue 候选）。
- **性质测试**：
  - 编码往返：固定种子值矩阵（各类型 + NULL + 长字符串 3 切换点）`serialize_tuple`→`deserialize_tuple` 逐项相等；`compute_tuple_size` == 实际写入长度；`deserialize_tuple_width` 宽度一致（`#[test]` 纯同步）。
  - B-Tree：经 `IndexManager` 插入固定种子 (key, row_id) 序列（含重复键）→ `search_all` 每键返回集 == 逻辑模型（BTreeMap 多重集）→ 交错删除（`delete_exact`）→ 再枚举一致（`#[tokio::test]`，tempdir + BufferPool）。
  - 版本链可见性：固定种子写入/更新/删除/回滚序列（lib API 显式事务）→ 快照读行集 == 串行模型（`#[tokio::test]`）。

### 2.2 T02 绑定阶段基建与收口（Iteration 001+）

**绑定结果结构**（`src/parser/planner/` 内新模块 `binding.rs`，无新执行器/无 PhysicalPlan 词表变化）：

```rust
/// 一次查询构建作用域内解析成功的列引用绑定。
pub(crate) struct BoundColumn {
    /// 关系段键（声明限定符——与 join_column_layout 段键同键空间）
    pub(crate) relation: String,
    /// 关系内列位置（组合行布局段内偏移；单表 = 表内位置）
    pub(crate) position: usize,
}
```

一期不引入独立 IR 遍：绑定结果以**声明限定符 → 列 → BoundColumn 的作用域表**形式在 `PlanBuilder` 内承载（加性字段 `bound_columns: Vec<ScopeEntry>`，`ScopeEntry { qualifier, columns: Vec<String> }`——即「带身份的布局」），由 FROM/JOIN 装配点在注册表登记处同步登记（声明限定符 + 列清单 + 每列可空/类型信息自 `table_column_types` 取）。收口消费点：

- **C1 关联子查询参数分类**：`qualified_column_expression` 优先级 1 的判定从「字符串名单成员资格」改为「作用域身份查找」，判定公式单源于 `binding.rs::is_outer_reference(inner_hit, outer_hit, in_declared_list)`——内层作用域段（`[boundary, len)`）命中 → 内层引用；内层未命中且外层段（`[0, boundary)`）命中 → **外层相关参数**；名单兜底未收录限定符（attach 复合键等未登记面）→ 既有名单判定。**碰撞形状语义（2026-10-20 用户裁定，对齐 SQLite）**：内层 FROM 以别名声明同表 + 外层同名未别名声明 + 子查询内基名限定引用 → 别名遮蔽基名，解析为外层相关参数；无外层同名的基名引用（A1 宽松形状，2026-10-15 豁免）维持内层解析。`extract_subquery_table_names`/`inner_table_names` 字符串名单保留为兜底。**收集侧对齐（001-replan，2026-10-20）**：`collect_outer_column_refs` 的收录判定改用同一 `is_outer_reference` 谓词——输入为四收集臂在作用域截断前捕获的段名快照（内层段名 + 外层段名），消除「分类产出参数、收集名单判定不收录」的未绑定参数脱节（父 Cycle 000-initial Plan Review Finding 1：碰撞形状静默 NULL 回归）；投影位标量臂收集自构建前移至构建后（快照可得性要求；「构建失败 + 预收集本应先行拒绝」的双无效输入错误优先级变化为已接受边界）。错误面：碰撞形状外逐字节保持（差分网 + 既有基线零回归佐证）。
- **C2 JOIN 组合行布局解析**：`join_column_layout` 的段键空间与绑定作用域表统一——布局结构从 `Vec<(String, Vec<String>)>` 改为携带同一 `ScopeEntry` 身份的段（布局构造点从作用域表派生，`build_from_clause_with_projection` 与 NLJ/WHERE/Projection/ORDER BY 五个消费点不改调用形），消除「布局段键与 FROM 注册表各自构造、约定一致」的耦合。`expr_join_side`/`extract_join_conditions` 的侧别判定改查作用域表（等价判定，错误面不变）。

**明确不改**：`condition_column_index` 执行期按名查表（MS25 装配的限定键 map，改它牵动执行器接口——超出「不改执行器接口」边界，留二期）；`resolve_column_ref`/`ColumnRef` 计划节点载荷（Hash JOIN 条件键通道）；`get_plan_output_columns` 表头通道；视图/CTAS/INSERT SELECT 各 DDL 面。这些面经差分网获得保护但不在一期重构。

**收口面调查结论（Phase 2 定稿）**：一期收口 = C1（分类判定单点化）+ C2（布局与作用域表统一）。二者是历史缺陷（ISS16-18 分类面、ISS05/07/09 布局面）的直接结构根因，且改动面收敛在 planner 内部两个函数族；执行器侧定位点（join.rs:75）与其余按名点受差分网保护、留待 I080 后续批次。

## 3. 关键技术选择

| # | 选择 | 替代方案 | 理由 |
|---|---|---|---|
| D1 | 差分经 lib API（execute_sql）进程内对比 | CLI 子进程对比 | 被测对象是 SQL 语义；渲染面已有 e2e 套件；进程内毫秒级×数百查询可控（CLI 子进程每查询 ~100ms×300 = 30s+ 不可接受） |
| D2 | 数据集固定种子、查询清单硬编码于测试文件 | 运行期生成 SQL/表结构 | 可重现、失败可缩减（I091 明示「缩减失败输入」——固定清单即最小失败输入）；无探索引擎依赖 |
| D3 | 归一化 = 行级排序 + 类型元组比较；ORDER BY 语义验证靠带期望值的固定断言用例 | 全部查询带期望值 | 无序行集的跨引擎比较必须消除顺序；固定断言锁 ORDER BY 语义（ handful 条），差分锁等价性（全量组合） |
| D4 | 性质测试用 rand 固定种子 + 手写断言，不新增 proptest | 引入 proptest | I091「复用既有开发依赖」；rand 0.8 已在依赖树；性质的生成空间用手工矩阵+种子序列已可覆盖目标不变量 |
| D5 | 绑定作用域表在 PlanBuilder 内加性承载，不做独立 IR/遍 | 独立 BindingIR AST 遍 | I080 明示「不一次重写全部规划器」；一期消费面 2 个，IR 遍的收益在消费面扩展后才兑现；加性字段方案零回归风险最小 |
| D6 | C1 判定单点化 + 名单兜底保留 | 直接删除 inner_table_names 机制 | 附加库复合键/CTE/派生表段的一期登记边界需逐面调查；兜底保证未登记段行为逐字节不变，收口不扩面 |
| D7 | 白名单常量内嵌测试文件 | 独立 fixture 文件 | 单一权威位置；`cargo test` 单命令可见；避免 fixture 加载层 |
| D8 | Iteration 000 单独验收后才展开绑定 Iteration | 全 change 一次验收 | tasks.md MS30 明示「差分网先行独立验收」；绑定重构需要保护网已 GREEN 的稳定基线 |

## 4. 数据迁移 / 并发 / 兼容

无数据迁移。并发面零触碰（差分网为纯测试新增；绑定层是 planner 同步代码）。兼容性：绑定层不改任何用户可见接口——`PlanBuilder` 公共 API、`PhysicalPlan` 词表、错误面全部保持；`ScopeEntry` 为 `pub(crate)` 加性字段。

## 5. 实现顺序与依赖

```
Iteration 000（T01）：D-1 种子数据集与归一化 helper → D-2 差分子集+白名单 → D-3 等价变换 → D-4 性质测试 → 全量 Gate
Iteration 001（T02-C1）：绑定作用域表基建 + 关联参数分类单点化（先行测试：差分网别名/子查询面 + 既有 2029 基线）
Iteration 002（T02-C2）：布局与作用域表统一（先行测试：差分网 JOIN 面 + 既有基线 + EXPLAIN 计划形状锁定）
```

C1 先于 C2：分类判定是 2026-10-15 修复的直接面（最新缺陷记忆），且 C2 的段键统一依赖 C1 建立的作用域表基建。每个 Iteration 独立可验收（各自全量 Gate + 差分网 GREEN）。

## 6. 验证设计（最简直接判定）

- 差分/性质：`cargo test --test sql_differential_test`、`--test structural_property_test` 直接退出码判定（无判定层）。
- 绑定层：先行测试 = 差分网相关面 + 既有子查询/JOIN/别名矩阵套件（`subquery_alias_binding_test` 22、`subquery_test` 43、`join_output_resolution_test`、`self_join_test`、`chain_join_test`、`nested_loop_join_test`、`table_alias_resolution_test`、`scalar_subquery_comparison_test` 14、`cte_test`、`view_test`）全绿；全量 `cargo test` 零回归（分批串行，每 Iteration 一次预算）；EXPLAIN 计划形状用例（既有 explain_test）零校准——锁 D5「词表不变」。
- 错误面零回归：既有错误断言（planner_test 87、cli_test 错误面用例）零校准通过即证明（A7）。

## 7. 风险与未知项

- **R1**（条件性）：差分网首次运行可能暴露真实未知差异（这正是其目的）。处置：Iteration 000 Act 期分诊——可归因子集边界误判（数据/查询设计踩到双方语义差异）则修数据集并记录；疑似产品缺陷 → A4 处置（收口面内 = T02 修复候选，范围外 = Issue 候选报告）。不阻塞差分网本身交付（白名单机制承载分类结果）。
- **R2**（非实质未知项）：rusqlite 0.31 无 bundled feature 时链接系统 SQLite——版本随环境。差分子集只用稳定核心 SQL（SQLite 3.x 十年稳定面），环境敏感性低；`sqlite_compare` bench 已长期同形态运行（R32 先例）。
- **R3**（条件性）：全量回归时间增长（新增 2 测试文件）。预算：单文件秒级（数据集 ~千行级 × 查询 ~百条，lib in-process）；000 Gate 实测记录增长量，超出分钟级显著增长时缩减数据集规模（D2 硬编码使缩减可控）。
- **R4**：`ScopeEntry` 登记遗漏面（attach 复合键、视图展开段、CTE 段）——D6 兜底机制保证未登记段回落既有名单判定，行为不变；登记完备性由差分网与既有套件共同见证。
