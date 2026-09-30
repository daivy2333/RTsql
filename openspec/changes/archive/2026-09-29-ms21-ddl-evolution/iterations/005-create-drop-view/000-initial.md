# Iteration 005 / Cycle 000-initial: CREATE/DROP VIEW

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T5-a…d（`tasks.md` T5 全部；spec `sql-views` R1–R3）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: MS25 的 DerivedScan 基座与别名应用（`DerivedScanNode` 构造先例 `query.rs:373`〔派生表〕/`:905`〔CTE〕、`get_plan_output_columns` DerivedScan 臂 `query.rs:55`、别名覆盖消费 `:1090`）；MS25 CTE 通道（`resolve_cte_reference` `query.rs:872`，FROM 钩子消费 `:337`/`:418`）；MS20 的 attach 路由（`register_table` `pipeline.rs:1994`，限定名路由 + 42P01 miss 面 `:2058` 区域）；MS24 的 `table_defaults`/`table_unique_columns` builder 通道注入先例（plan_stage 通道注入形态）；本 change Iter003 的 `insert_column_row`（列行链尾追加、链序不变量）与 DDL 直包臂 + in-tx 先例、Iter004 的穷尽补臂清单；design D12/D13/D14/D15。
- Excluded scope: 可更新视图/物化视图/CHECK OPTION；递归视图；视图权限；dump 携带视图（D15：dump 仍为 CREATE TABLE + INSERT）；运行期环检测（创建期验证结构性封死）；视图深度限制（见 Risks）

**Objective**

`CREATE VIEW <name> [(col, ...)] AS <SELECT>` 持久化只读视图（定义 SQL 文本 + 列名清单经 catalog 持久，跨重启/跨会话可查）；`SELECT ... FROM <view>` 在名称解析期展开为等价派生表（与内联 `FROM (<定义>) AS <视图名>` 逐字节一致，含组合面与错误面）；视图可引用基表与既有视图（嵌套展开），循环结构性不可能；显式列清单覆盖输出列名，缺省取定义输出表头；`DROP VIEW` 删除定义（跨重启持久），DML 穿透视图计划期只读拒绝，非基表目标（ALTER/TRUNCATE/CREATE INDEX/DROP TABLE）点名拒绝；定义变更后按新定义展开、旧计划不复活；既有面零回归。

**Investigation Facts**

- Current Baseline: revision 0fe6971 + Iter000–004 未提交实施（工作区 30 修改文件 + 6 新文件 + change 目录）；全量 **1522 passed / 0 failed / 2 ignored**（Iter004 Act Response 分批新鲜结论，Iter004 Plan Review 采信 + 用户明示「测试可信」）；静态门全绿。
- Current-State Evidence:
  - **语句入口**：`Statement::CreateView { or_replace, materialized, name: ObjectName, columns: Vec<ViewColumnDef>, query: Box<Query>, options: CreateTableOptions, cluster_by, with_no_schema_binding, if_not_exists, temporary }`（sqlparser 0.44 `ast/mod.rs:1871`）；`ViewColumnDef { name: Ident, options: Option<Vec<SqlOption>> }`（`ast/ddl.rs:675`——无 data_type 字段，`options` Some → 点名拒绝）。`DROP VIEW` = `Statement::Drop { object_type: ObjectType::View, names, if_exists, .. }`（`ObjectType` `ast/mod.rs:4813`；现 Drop 臂 `mod.rs:376`：Table → build_drop_table、Index → build_drop_index、其余泛化拒绝——View 臂在此插入）。现全仓无 CreateView/DropView 消费 → catch-all `UnsupportedStatement`。
  - **catalog 承载**：`CatalogRow`（`catalog.rs:65` 区域）尾段现为 `unique_roots`（MS23）→ `secondary_indexes`（Iter002）；`serialize_catalog_row` `:764` / `deserialize_catalog_row` `:811`——kind 段追加其后、向后兼容读（缺段 → Table）。视图行字段约定：`data_page_head=0`/`data_page_tail=0`/`index_root_page_id=0`/`pk_column=""`/`pk_index=0`/`column_count=列数`。`scan_tables()` `:239` / `scan_columns(table)` `:248`；视图列行复用 `insert_column_row`（Iter003，链尾追加保链序）；删除复用既有表行/列行删除机制（`delete_table` 与 `delete_from_chain` matcher 同型）。
  - **open 载入**：`TableManager::open_or_init`（`table_manager.rs:224`）自 `scan_tables` 逐行建 TableMeta（`:225`）——kind=View 行在此跳过（防零 head 页构建）；视图注册表由 `Database` 持有（`Database` 字段区 `database.rs:17-28`，加 `views: RwLock<HashMap<String, ViewMeta>>` 形态），`Database::open` 序列中 table_manager 初始化后经新 Catalog 扫描 API（或 scan_tables 过滤 + 逐视图 scan_columns）载入。lifecycle `list`/`schema`/`dump` 三处 `catalog.scan_tables()` 消费点（`lifecycle.rs:212/:254/:416`）须过滤 kind=View（D15：dump 不携带视图；对无视图库逐字节不变）。
  - **计划臂**：`build_create_view`（新，`ddl_dml.rs`，与 `build_create_table`/`build_alter_table` 同位）：修饰拒绝矩阵（`or_replace`/`materialized`/`if_not_exists`/`temporary`/`options`/`cluster_by`/`with_no_schema_binding`/列 `options`）；重名 vs 基表（builder.tables）+ 既有视图（builder 视图通道，plan_stage 注入先例 `existing_index_names`/`table_has_live_rows` 同型）；定义引用验证——**创建期对定义 `Query` 直接 `build_query`**（sqlparser 已给 AST，无需重解析；CTAS `build_create_table_as_select` 消费 `get_plan_output_columns` 先例 `ddl_dml.rs:942-966`）：定义内表名先经 plan_stage 的 `register_table` 通道注入（缺失 → 42P01 自然上抛；列缺失 → 42703 自然上抛），附加库引用在 plan_stage 对定义表名 walk 后按 `route.is_registered` **点名拒绝**（spec R1：定义引用附加库拒绝——不可持久化会话限定名）；列清单长度 vs 定义输出宽度匹配（`get_plan_output_columns`）；SQL 文本超长前置点名拒绝（slot 上限，`add_slot` PageFull 前置检查，D12 ~3.5KB）。
  - **展开钩子**：FROM/JOIN 名字解析点 `query.rs:332`（`TableFactor::Table` 臂，CTE 查找 `:337` 先于基表）与 `:418`（JOIN 右侧，`resolve_cte_reference` 同型调用）——视图命中插入位：CTE 之后、基表 registry 之前（CTE 优先级语义不变）；命中 → `crate::parser::ast::parse_sql(定义文本)`（planner 侧入口，测试用先例 `ddl_dml.rs:1529`；pipeline 侧 re-export `pipeline.rs:13`）→ 顶层上下文 `build_query`（定义内 ORDER BY/LIMIT/聚合/JOIN/嵌套视图递归全可达，`:806-810` CTE 顶层上下文先例）→ 包 `DerivedScanNode`（alias=视图名、显式列清单覆盖 columns，`:373`/`:905` 构造先例、`:1090` 别名消费）。`register_table` miss 42P01（`pipeline.rs:2058` 区域）前插视图命中豁免（statement 级引用合法化——FROM 钩子承载实际展开）。
  - **DML 只读拒绝**：`build_insert`（`ddl_dml.rs:154`）/`build_update`（`:1378`）/`build_delete`（`:1465`）目标名消费 builder 视图通道 → 命中即计划期点名拒绝（`reject_attached_write` `:262-269` 同位形）；`DROP TABLE` 对视图名按表不存在拒绝（既有 `get_table` 通道自然成立，零代码）；`ALTER`/`TRUNCATE`/`CREATE INDEX` 目标为视图 → builder.tables miss → 42P01 自然成立（Iter003/004 已见证同型保留名行为）。
  - **IN 子查询缺口**：`get_subquery_first_column`（`subquery.rs:386`）现对 `DerivedScan` 兜底误报多列——补 DerivedScan 臂返回首列名（D12：CTE 同现状，随视图一并收口；若 `cte_test` 存在依赖旧误报文案的用例按校准白名单处理）。
  - **pipeline 接线**：`plan_stage_routed` 视图通道注入 + 定义表名 register/attach 判定；`execute_stage_routed` DDL 直包臂（Iter003/004 先例 `:322` 区域）+ 执行后 `plan_cache.clear()`（spec R3 旧计划不复活——`is_cacheable` 仅 Query `:2047`，引用视图的 SELECT 计划被缓存，CREATE/DROP VIEW 全清封死）；`execute_stage_in_tx_routed` **沿 CREATE TABLE 现状**（D13：立即生效、不随回滚撤销，`tx_statement_test.rs:348-365` 先例）——CreateView/DropView 臂直包执行器，**不加 0A000**；`create_executor_from_plan` panic 臂 + 穷尽补臂（`plan.rs` 枚举、`plan_format.rs`、`query.rs` 空列臂、`correlated.rs` 叶臂——Iter004 清单同款）。
  - **错误面**：D14——复用既有 PlanError 变体（能力拒绝 UnsupportedStatement 0A000、TableNotFound 42P01、ColumnNotFound 42703、重名/超长 InvalidQuery 形态），不新增变体。
- Code and Critical Path: `src/storage/catalog.rs`（kind 段编解码 + 视图行 API）、`src/storage/data/table_manager.rs`（open_or_init 跳过）、`src/database.rs`（视图注册表 + open 载入）、`src/cli/lifecycle.rs`（三处过滤）、`src/parser/planner/{mod.rs,ddl_dml.rs,query.rs,subquery.rs}`（分派/创建臂/展开钩子/IN 臂）、`src/executor/{plan.rs,plan_format.rs,create_view.rs?,drop_view.rs?}`、`src/pipeline.rs`（通道注入 + 四接线臂）；见证 `tests/view_test.rs`（新）。

**Implementation Guidance**

顺序 a→b→c→d：catalog 段与注册表先行（含 open_or_init 跳过与 lifecycle 过滤——无视图库零行为变化先锁定）；CREATE VIEW 随后（修饰/重名/引用验证矩阵 + 持久化 + 注册表更新）；展开与消费是核心（FROM/JOIN 钩子 + register_table 豁免 + DML 拒绝 + IN 臂——展开语义以「与内联派生表逐字节一致」为唯一裁判）；DROP VIEW 与组合面见证收尾。核心不变量：视图只读非物化（无数据页/索引树/行级痕迹）；名称唯一性跨基表与视图；CTE 通道优先级不变（先 CTE 后视图再基表）；定义变更后旧计划不复活（DDL 全清）。

**Task Contracts**

### T5-a: catalog kind 段与视图注册表

- Requirement/Scenario: R1-S3 跨重启持久的承载面；R2/R3 拒绝与一致性的数据面
- Depends on: None
- Targets: `src/storage/catalog.rs`（`CatalogRow.kind: CatalogKind`〔Table/View〕+ 视图 SQL 文本尾段编解码〔u8 kind + kind=View 携 `u16 len | 文本`，向后兼容读缺段 → Table〕+ `insert_view_row` + 视图扫描 API + 删除复用既有机制）；`src/storage/data/table_manager.rs::open_or_init`（kind=View 行跳过 TableMeta 构造）；`src/database.rs`（`Database` 视图注册表字段 + `open` 序列载入〔视图行 + 逐视图列行 `scan_columns`，列序 column_index 排序加固同 `:238` 先例〕）；`src/cli/lifecycle.rs`（`:212/:254/:416` 三处 scan_tables 消费过滤 kind=View）
- Current behavior: CatalogRow 无 kind；全表行一律建 TableMeta；视图概念不存在
- Required behavior: 视图行与列行持久往返（lib 单测）；kind 段向后兼容读（旧行 → Table 逐字节不变）；无视图库 lifecycle/dump 输出逐字节不变
- Preserve: 既有尾段（unique_roots、secondary_indexes）编解码零变化（视图段追加其后）；bootstrap 页 0/1 断言与 catalog 打开路径零变化；`create_table_with_constraints` 写面零变化
- Forbidden: 视图物化（数据页/索引树）；dump 携带视图；修改既有段格式
- Test witness: catalog 编解码往返 + 兼容读 lib 单测（纯增量）；e2e 面在 T5-b 到位后 RED
- GREEN condition: `cargo test --lib` 0 failed + `schema_persistence_test`/`cli_test` 零回归
- Verification: `cargo test --lib --test schema_persistence_test -- --test-threads=1`（0 failed）
- Stop when: 尾段无法向后兼容读（结构性）——返回 Plan

### T5-b: CREATE VIEW 计划臂与执行器

- Requirement/Scenario: R1 全部 7 场景（创建查询/列清单/持久/嵌套/循环拒绝/定义错误与重名/修饰拒绝）
- Depends on: T5-a
- Targets: `src/parser/planner/mod.rs`（`Statement::CreateView` 分派）；`src/parser/planner/ddl_dml.rs::build_create_view`（修饰拒绝矩阵；重名 vs 基表+视图；创建期定义 `build_query` 验证——引用缺失 42P01/42703 自然上抛、列清单长度 vs `get_plan_output_columns` 匹配、列 `options` 拒绝、SQL 文本超长前置点名拒绝）；`src/pipeline.rs`（plan_stage：定义表名 walk → attach 引用点名拒绝 + 基表引用 register_table 注入 + 视图通道注入；DDL 直包臂 + 执行后 `plan_cache.clear()`；in-tx 沿 CREATE TABLE 现状臂）；`src/executor/plan.rs`/`plan_format.rs`/`query.rs`/`correlated.rs`（`PhysicalPlan::CreateView` 节点 + 穷尽补臂）；`src/executor/`（新 CreateView 执行器：catalog 视图行 + 列行持久〔列行 ordinal 序复用 `insert_column_row`〕+ `Database.views` 注册）
- Current behavior: `CREATE VIEW` 落 catch-all `UnsupportedStatement`
- Required behavior: 创建成功后 `SELECT * FROM <view>` 返回定义行集、列名 = 显式清单或定义输出表头；重开可查；嵌套视图（定义引用既有视图）展开一致；定义引用不存在表/列点名拒绝零副作用；对基表名或既有视图名重名创建点名拒绝零副作用；`OR REPLACE`/`MATERIALIZED`/`IF NOT EXISTS`/`TEMP`/附加库引用/超长定义均点名拒绝
- Preserve: 基表数据与 schema 零变化；循环引用结构性封死（引用尚不存在的视图在创建期即拒）；D14 错误面
- Forbidden: 物化；`OR REPLACE` 语义；视图列类型声明；运行期环检测
- Test witness: `tests/view_test.rs` R1 场景（行为面先 RED：现报 UnsupportedStatement）；持久用例依赖 T5-a
- GREEN condition: 新用例绿 + `cte_test`/`derived_scan_alias_test`/`cli_test` 既有矩阵零修改通过
- Verification: `cargo test --test view_test --test cte_test --test derived_scan_alias_test -- --test-threads=1`（0 failed）
- Stop when: 创建期验证无法在单 builder 上下文闭合（结构性）——返回 Plan

### T5-c: 展开与消费

- Requirement/Scenario: R1-S1/S4 展开一致性 + R2-S1 DML 拒绝 + R3 全部（解析一致性/组合面/零回归）
- Depends on: T5-a、T5-b
- Targets: `src/pipeline.rs::register_table`（miss 42P01 前插视图命中豁免——statement 级引用合法化）；`src/parser/planner/query.rs`（builder 视图通道 + FROM 钩子 `:332` 臂与 JOIN `:418` 臂：CTE 查找后、基表前查视图 → `parse_sql(定义)` → `build_query` 递归展开 → `DerivedScanNode` 包裹〔alias=视图名、显式列清单覆盖 columns，`:373`/`:905` 先例〕）；`src/parser/planner/ddl_dml.rs`（`build_insert:154`/`build_update:1378`/`build_delete:1465` 目标消费视图通道 → 只读点名拒绝）；`src/parser/planner/subquery.rs::get_subquery_first_column`（`:386` 补 DerivedScan 臂返回首列名）
- Current behavior: 视图名 FROM 引用落 42P01；DML 目标为视图名落 register_table 42P01；IN 子查询对 DerivedScan 基座误报多列
- Required behavior: `SELECT ... FROM <view>` 与内联派生表结果/表头/错误面逐字节一致（含 WHERE/JOIN/聚合/ORDER BY/子查询组合）；视图引用视图递归展开；`INSERT/UPDATE/DELETE` 穿透视图计划期只读点名拒绝、基表零变化；IN 子查询引用视图可达（首列名正确）
- Preserve: CTE 优先级不变（同名 CTE 遮蔽视图，resolve_cte_reference 先位）；派生表既有组合语义零特殊化；`DROP VIEW` 后引用点名拒绝、同名重建后按新定义展开（T5-b plan cache 全清承载）
- Forbidden: 视图专用优化路径；展开结果的特例语义；可更新视图
- Test witness: `tests/view_test.rs` 组合面 + DML 拒绝 + IN 子查询用例；`cte_test`/`derived_scan_alias_test` 零回归（若 IN 臂校准触发白名单按公共规则记录）
- GREEN condition: 新用例绿 + 既有矩阵零修改通过
- Verification: `cargo test --test view_test --test cte_test --test derived_scan_alias_test --test join_output_resolution_test -- --test-threads=1`（0 failed）
- Stop when: 展开递归与既有 CTE/派生表通道语义冲突（结构性）——返回 Plan

### T5-d: DROP VIEW 执行器与组合面见证

- Requirement/Scenario: R2-S2/S3（DROP VIEW 删除与不存在拒绝/非基表目标拒绝）+ R3-S1 定义变更 + 崩溃持久
- Depends on: T5-b、T5-c
- Targets: `src/parser/planner/mod.rs`（Drop 臂 `ObjectType::View` → `build_drop_view`：`IF EXISTS` 点名拒绝、视图不存在点名拒绝）；`src/executor/`（新 DropView 执行器：catalog 表行 + 列行删除〔复用既有删除机制〕+ `Database.views` 移除）；`src/pipeline.rs`（DDL 直包臂 + `plan_cache.clear()`；in-tx 沿 DROP TABLE 现状臂）；`tests/view_test.rs`（R2/R3 见证 + 崩溃重开持久 + JOIN/聚合/ORDER BY/嵌套组合面）
- Current behavior: `DROP VIEW` 落 ObjectType 泛化拒绝；`DROP TABLE <view>` 现状即按表不存在拒绝（自然成立，见证锁定）
- Required behavior: DROP VIEW 删除定义（重开持久）；删除后查询点名拒绝、基表数据不受影响；再删/删不存在点名拒绝；`TRUNCATE/CREATE INDEX/ALTER/DROP TABLE` 对视图名点名拒绝（前两者 42P01 自然成立 + 见证锁定）；同会话 DROP → 以新定义重建 → 按新定义展开；显式事务内 CREATE/DROP VIEW 沿 CREATE/DROP TABLE 现状（立即生效不随回滚撤销）
- Preserve: 基表数据零变化；`DROP TABLE` 对视图名既有 42P01 文案零变化；D13 事务边界表零变化
- Forbidden: `DROP VIEW IF EXISTS` 支持；级联删除引用视图
- Test witness: `tests/view_test.rs` R2/R3 场景；`tx_statement_test` 零回归
- GREEN condition: 新用例绿 + `tx_statement_test` 零修改通过
- Verification: `cargo test --test view_test --test tx_statement_test -- --test-threads=1`（0 failed）+ 全量分批零回归
- Stop when: None

**Invariants**

- 视图只读、非物化：无数据页、无索引树、无行级痕迹；DML/非基表 DDL 目标全部计划期或执行期入口点名拒绝。
- 名称唯一性跨基表与视图；展开语义与内联派生表逐字节一致（结果、表头、错误面）；CTE > 视图 > 基表的解析优先级不变。
- 定义变更后旧计划不复活（引用视图的 SELECT 计划缓存 + CREATE/DROP VIEW 全清）；DROP 后引用点名拒绝。
- 视图定义只引用主库基表/既有视图（附加库引用创建期点名拒绝）；循环引用结构性不可能。
- lifecycle/list/schema/dump/import 对无视图库逐字节不变；视图不进 dump（D15）。
- in-tx 沿 CREATE/DROP TABLE 现状（D13）；D14 错误面不新增变体。

**Non-goals**

- 可更新视图/物化视图/CHECK OPTION；递归视图；视图深度限制；权限/所有权；dump 携带视图；`OR REPLACE`/`IF EXISTS`/`TEMP` 支持；运行期环检测。

**Acceptance**

1. `sql-views` R1–R3 全部场景有可观察见证（创建/查询/列清单/持久/嵌套/循环与定义错误拒绝/修饰拒绝、DML 只读拒绝/DROP VIEW/非基表目标、定义变更一致/组合面/全量零回归）。
2. RTM：

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| sql-views › R1 CREATE VIEW 持久化只读视图（7 场景） | T5-a/b/c | `view_test` R1 组 + catalog lib 单测 | Covered |
| sql-views › R2 视图只读与 DML/DDL 拒绝（3 场景） | T5-c/d | `view_test` R2 组 | Covered |
| sql-views › R3 视图解析与计划一致性（3 场景） | T5-c/d | `view_test` R3 组 + 全量分批 + 静态门 | Covered |

**Verification**

- 各 Task 契约矩阵逐项 0 failed。
- 全量：M19 分批形态全部批次 0 failed（基线 1522 + 本轮新增）。
- 静态门：`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate 2026-09-29-ms21-ddl-evolution` 全部通过。

**Gate 2 Readiness**

Gate 2 全部 PASS（2026-09-30 用户批准本 Iteration 计划——原话「批准」）。调查（AST 形状/catalog 尾段/展开钩子/DML 目标面/IN 缺口/pipeline 接线/lifecycle 消费点全部 file:line 定位）、设计（D12 闭合——创建期验证结构性封死循环、CTE 通道展开、plan cache 全清；D13/D14/D15 边界带入）、任务契约完备（4 契约依赖有序、拒绝面与 Preserve/Forbidden 明确）、验证最简直接、Persisted Evidence 无必要——以上 PASS。无 BLOCKED、无 WAIVED 项。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- **展开递归深度**：创建期验证封死循环（引用尚不存在的视图即拒），嵌套深度无上限——超深视图链的计划期递归（`build_query` 递归展开）理论上可栈溢出；spec 未设深度限制，接受为文档化边界（非目标）。
- **IN 子查询臂的行为面变化**：`get_subquery_first_column` 补 DerivedScan 臂同时改变 CTE 基座 IN 子查询的既有兜底（误报多列 → 正确首列名，D12 记载「随视图一并收口」）——若 `cte_test` 有锁定旧误报的用例，按校准白名单处理并在 Act Response 记录。
- **attach 引用检测时机**：创建期按 plan_stage 的 route 判定定义表名——「ATTACH 会话中创建、他会话使用」不可复现，故 spec R1 规定创建期即点名拒绝（可持久对象不得携带会话限定名）。
- **register_table 豁免与路由顺序**：两段限定名先 attach 路由（视图命中豁免只对 flat 名——视图是主库对象）；三段深限定拒绝不变。
- **超长定义上限的具体字节值**：slot 容量减去行头/尾段的预留为非实质实现细节，Act 以「超长 → 前置点名拒绝零副作用」为契约，边界值不进断言（超长用例以明确超限文本构造）。
- 非实质选择留给 Act：`ViewMeta` 形状与注册表并发原语、视图扫描 API 形态（scan_views vs scan_tables 过滤）、执行器文件组织（create_view.rs/drop_view.rs 合分）、拒绝文案措辞（点名能力）、用例命名与断言组织。

## Act Response

- Status: reported

### 实际改动

**T5-a catalog kind 段与视图注册表**

- `src/storage/catalog.rs`：`CatalogKind`（Table/View）+ `CatalogRow.kind`/`view_sql` 字段；尾段编解码——secondary 段后追加 `u8 kind`（kind=View 携 `u16 len | text`），向后兼容读（缺段/不完整/未知/无效 utf8 → Table，同既有「不完整尾随段容忍」模式）；`insert_view_row`（零页指针 + 空 PK + column_count）+ `scan_views`；lib 单测 4 个（View 行往返 / 旧格式截断兼容 / 截断 SQL 段容忍 / insert_view_row 持久往返 + 基表共存 + delete_table 复用）。
- `src/storage/data/table_manager.rs::open_or_init`：kind=View 行跳过 TableMeta 构造（防零 head 页接线）。
- `src/wal/recovery.rs`：`build_pk_version_maps` / `rebuild_pk_indexes` / `mark_uncommitted_aborted` 三处 scan_tables 消费点同谓词跳过 View 行（见 Deviations 1）。
- `src/database.rs`：`ViewMeta { sql, columns }` + `Database.views: Arc<RwLock<HashMap<String, ViewMeta>>>`；`open_with_key` 在 open_or_init 后经 scan_tables 过滤 kind=View + 逐视图 scan_columns（column_index 排序加固）载入。
- `src/cli/lifecycle.rs`：schema / dump 两处 Table-only 过滤（D15：视图不进输出）；restore 空库守卫只对 Table 行拒绝（视图行不构成非空）；测试夹具 `table()` 补 kind 字段。
- `src/storage/mod.rs`：`CatalogKind` re-export。

**T5-b CREATE VIEW 计划臂与执行器**

- `src/parser/planner/mod.rs`：`ViewDef { sql, columns }` + `PlanBuilder.views` 语句级通道 + `set_views`（pub(crate)）；`Statement::CreateView` 分派 → `build_create_view`。
- `src/parser/planner/ddl_dml.rs`：`MAX_VIEW_SQL_BYTES = 3000`（slot 容量边界，契约=「超长前置点名拒绝」，边界值不进断言）；`build_create_view`——修饰拒绝矩阵（OR REPLACE / MATERIALIZED / IF NOT EXISTS / TEMP / CLUSTER BY / NO SCHEMA BINDING / WITH-OPTIONS）、限定名拒绝、系统保留名拒绝、重名跨基表与视图、列选项防御臂、创建期定义 `build_query` 验证（42P01/42703 自然上抛；循环结构性封死）、输出列重复拒绝、列清单长度/重复校验、定义文本 `Query::to_string()` 规范化 + 超长前置拒绝。
- `src/executor/plan.rs`：`PhysicalPlan::CreateView` + `CreateViewNode { view_name, sql, columns }`。
- `src/executor/create_view.rs`（新）：TOCTOU 复检（表集合 + 视图注册表双向）→ 列名行（类型占位 String(255)，列名-only 行——全部类型消费面已跳过 View 行）→ `insert_view_row` 持久化 → 注册表登记；失败零副作用。
- `src/pipeline.rs`：`extract_all_table_names` 增 CreateView 定义查询臂（CTAS 物化源语义同型）；`plan_stage_routed` 增 CreateView 专用臂（视图通道注入 + `register_view_definition_tables`）；`execute_stage_routed` / `execute_stage_in_tx_routed` DDL 直包臂（in-tx 沿 CREATE TABLE 现状，D13）+ 执行后 `plan_cache.clear()`（spec R3）。
- 穷尽补臂：`plan_format.rs`（leaf + describe `CreateView v (columns=[…])`）、`correlated.rs`、`query.rs::get_plan_output_columns`、`create_executor_from_plan` panic 臂。

**T5-c 展开与消费**

- `src/parser/planner/query.rs`：`resolve_view_reference`——命中视图通道 → 定义文本 `parse_sql` 重解析 → 派生表同型上下文（`building_subquery` save/restore，内联等价唯一裁判）`build_query` 递归展开 → `DerivedScanNode` 包裹（alias=引用别名或视图名、columns=持久化清单）→ 列通道注册；FROM 臂与 JOIN 右臂钩子插在 CTE 查找之后、基表 `validate_table` 之前（CTE > 视图 > 基表优先级不变量）。
- `src/pipeline.rs`：generic 臂语句级视图通道注入（无视图库空表 → 逐字节不变）；`register_table` miss 42P01 前插视图命中豁免 + `register_view_base_tables_recursive`（递归注册视图定义的基表面，展开钩子的列解析依赖；DAG 结构性无需环检测）；`current_view_defs` 映射助手。
- `src/parser/planner/mod.rs`：`reject_view_write`（`UnsupportedStatement("WRITE TO VIEW (views are read-only)")`，0A000，与 reject_attached_write 同位形）。
- `src/parser/planner/ddl_dml.rs`：`build_insert`/`build_update`/`build_delete` 目标视图只读拒绝（先于 validate_table，避免误导性 42P01）；`build_create_table` 增视图名重名守卫（执行期基表重名检查不覆盖 kind=View 行，D12 命名空间契约）。
- `src/parser/planner/subquery.rs`：`get_subquery_first_column` 补 `DerivedScan` 臂返回首列名（CTE 基座误报多列随之收口，D12；cte_test 零回归证实无用例依赖旧误报）。

**T5-d DROP VIEW 执行器与组合面见证**

- `src/parser/planner/mod.rs`：Drop 臂 `ObjectType::View` → `build_drop_view`；`ddl_dml.rs::build_drop_view`（IF EXISTS 点名拒绝、单名、限定名拒绝、存在性经视图通道、不存在 `TableNotFound` 42P01）。
- `src/executor/plan.rs`：`PhysicalPlan::DropView` + `DropViewNode`；`src/executor/drop_view.rs`（新）：TOCTOU 复检 → `catalog.delete_table` 复用双链删除（视图行 + 列名行）→ 注册表移除。
- `src/pipeline.rs`：CREATE/DROP VIEW 共用 DDL 直包臂（normal + in-tx）+ panic 臂；穷尽补臂（plan_format/correlated/query）。
- `tests/view_test.rs`（新，15 测试）：R1-S1…S7 全场景（含 CLI 表头形态、CLI ATTACH 引用拒绝 e2e）、R2-S1…S3、R3-S1…S2。

### 与计划的偏差及原因

1. **T5-a 契约补充 `src/wal/recovery.rs` 三处消费点**（契约 targets 未枚举）：open_or_init 跳过 View 行的直接机械后果——recovery 三处 scan_tables 消费点逐行 `get_table`，视图行会以 `TableNotFound → RedoFailed` 使含视图库的崩溃恢复失败。同 D12「视图行对表域不可见」不变量的必要补全，T5-d 崩溃持久见证依赖。
2. **CREATE VIEW 计划臂的注册通道**：契约描述「定义内表名先经 plan_stage 的 register_table 通道注入」——实际未走 register_table（其附加库臂是路由而非拒绝、且视图名会被 42P01），改为视图感知的 `register_view_definition_tables`（附加引用点名拒绝 + 既有视图引用递归注册 + 基表注入）。行为面与契约一致，实现位等价移动（Phase 3 契约内等价局部实现）。
3. **定义展开上下文**：契约写「顶层上下文 build_query（CTE 顶层先例）」，实现按内联等价唯一裁判取派生表臂先例（`building_subquery` save/restore 置 true）——展开语义与 `FROM (<定义>) AS <视图名>` 逐字节一致，含 WITH 定义在子查询上下文中的拒绝面与内联形态一致。
4. **`view_test` R1-S7 列选项断言移除**：`CREATE VIEW vv (a INT)` 在 sqlparser parse 层即拒绝（ViewColumnDef 无类型字段），planner 的 `VIEW COLUMN OPTIONS` 臂为直呼构造防御臂、SQL 不可达；parse 错误即观察面，契约拒绝矩阵保持。
5. **`planner_test` 1 处校准（白名单）**：`unsupported_statement_names_drop_object_type` 主体 `DROP VIEW v` → `DROP ROLE someone`——spec `sql-views` R2 将 DROP VIEW 转正为支持语句，原测试锁定的 catch-all 旧行为随之终止；测试意图（非 Table DROP 点名对象类型）不变。
6. **R3-S2 组合面两处对照调整**（测试面事实，非产品行为）：JOIN 右侧内联对照不可表达（JOIN 右臂对 `TableFactor::Derived` 是既有 `DERIVED TABLE` 拒绝面，视图引用为普通表名语法可达）→ 改语义结果断言；标量子查询 WHERE 值面为既有拒绝（基表同款 `Unsupported expression type: EXPRESSION`）→ IN 形态替代。视图对既有拒绝面零特殊化继承（IN+AND 组合同）。

### 自检发现、已修复内容和遗留 Minor 问题

- 已修复：视图定义基表未预注册导致展开后列解析 42P01（TDD 揪出）→ `register_view_base_tables_recursive`；`build_create_table` 缺视图名重名守卫（D12 命名空间）→ 计划期补齐；clippy 两处（`set_views` 可见性、`ViewDef.name`/`ViewMeta.name` 死字段）→ 收紧/删除。
- 遗留 Minor：CREATE TABLE 对既有视图名的 TOCTOU 复检仅在计划期（CreateViewExecutor 有执行期复检、反向无）——单进程 plan+execute 背靠背窗口，与既有 CREATE TABLE 面一致，不单独加锁。
- 未解决问题：None。

### 验证

- T5-a：`cargo test --lib --test schema_persistence_test --test cli_test -- --test-threads=1` → 340 + 8 + 111 passed / 0 failed（2 ignored，基线同）。
- T5-b/c/d 契约批：`cargo test --test cte_test --test derived_scan_alias_test --test join_output_resolution_test --test tx_statement_test -- --test-threads=1` → 14 + 7 + 14 + 21 passed / 0 failed。
- RED 见证：实现前 `cargo test --test view_test` → 9 failed / 0 passed（catch-all `UnsupportedStatement`），与契约 Current behavior 一致。
- view_test 终态：`cargo test --test view_test` → 15 passed / 0 failed。
- 静态门：`cargo fmt --check` exit 0；`cargo clippy --all-targets -- -D warnings` exit 0。
- 全量 Gate：`cargo test -- --test-threads=1` → **1541 passed / 0 failed / 2 ignored**（基线 1522 + view_test 15 + catalog lib 4），exit 0。
- `openspec validate 2026-09-29-ms21-ddl-evolution` → valid。
- 注：第一次全量串行运行期间工作区发生了 `ViewMeta.name` 死字段删除（跑后源码变化），按验证规则该次证据不覆盖当前源码，已对当前源码重跑全量（上行为最终证据）；此次重跑属源码变化后的重验，非重复运行增强信心。

### Persisted Evidence

- None required（Mode: none；全部验证可低成本重跑，无白名单情形）。

### Experience Candidates

- None（涉及的既有能力边界——标量子查询 WHERE 值面、IN+AND 组合、限定名排序键、JOIN 右侧派生表——均为语料既有点名拒绝面，非缺陷）。

## Plan Review

- Review Result: accepted
- Date: 2026-09-30

**Findings**

独立代码审读（catalog 尾段编解码与兼容读、`insert_view_row`/`scan_views`、`open_or_init` 跳过、recovery 三消费点、database 注册表载入、lifecycle 三过滤、`build_create_view`/`build_drop_view` 全拒绝矩阵、`resolve_view_reference` 与 FROM/JOIN 钩子位、DML 三臂拒绝位、IN 子查询 DerivedScan 臂、`register_table` 豁免与路由序、pipeline 三接线 + plan cache 全清、穷尽补臂、两新执行器）与 Act Response 逐项核对。

- **F1〔核实通过 · 偏差 1 = Plan 遗漏〕recovery 三消费点补跳过**：`open_or_init` 跳过 kind=View 行的必要机械后果——`build_pk_version_maps`（`recovery.rs:317`）/ `rebuild_pk_indexes`（`:941`）/ `mark_uncommitted_aborted`（`:1314`）逐行 `get_table`，视图行将 `TableNotFound → RedoFailed` 使含视图库崩溃恢复失败。契约 targets 未枚举属 Plan 遗漏；Act 补全正确，且为 T5-d 崩溃持久见证的前置。非阻塞。
- **F2〔核实通过 · 偏差 2 = Plan 契约描述不精确〕定义表名注册通道**：契约写「经 register_table 通道注入」，但 register_table 的 attach 臂是路由而非拒绝、视图名会先落 42P01——所描述机制不可行。Act 改专用 `register_view_definition_tables`（`pipeline.rs:2076`：attach 引用点名拒绝 0A000 → 既有视图递归注册 → 基表 register 42P01 自然上抛），行为面与契约意图逐项一致，属等价实现位移动。非行为缺口，非阻塞。
- **F3〔核实通过 · 偏差 3〕展开上下文**：`resolve_view_reference`（`query.rs:940`）以 `building_subquery` save/restore 取派生表臂先例——「与内联派生表逐字节一致」唯一裁判的直接落实，含 WITH 定义在子查询上下文拒绝面同形。非阻塞。
- **F4〔核实通过 · 偏差 4/5〕测试面两处**：R1-S7 列选项断言移除——sqlparser parse 层即拒（`ViewColumnDef` 无类型字段），planner `VIEW COLUMN OPTIONS` 臂为直呼构造防御臂，SQL 可达面的拒绝行为仍可观察；`planner_test` 校准主体 `DROP VIEW v` → `DROP ROLE someone`——DROP VIEW 转正使原主体失效，白名单正当、意图（非 Table DROP 点名对象类型）保持且有注释。均非阻塞。
- **F5〔核实通过 · 偏差 6 事实主张〕JOIN 右侧内联对照不可表达**：`extract_join_table_name`（`ast.rs:352-356`）对非 `TableFactor::Table` 一律 `"DERIVED TABLE"` 拒绝——Act 的对照弱化理由成立；视图引用本身在 JOIN 右臂可达（`query.rs:435` 钩子），测试以语义结果断言覆盖；标量子查询 WHERE 值面为基表同款既有拒绝，IN 形态满足 R3-S2「子查询引用视图」。零特殊化继承成立。
- **F6〔采信说明〕验证结论**：RED（view_test 9 failed / 0 passed，catch-all 与契约 Current behavior 一致）→ view_test 15/0 → 契约批 359/0 与 56/0 → 全量串行 **1541 passed / 0 failed / 2 ignored**（计数自洽 = 基线 1522 + view_test 15 + catalog lib 4）→ 静态门 + `openspec validate` 全绿。git 只读核对：工作区新增 `src/executor/{create_view,drop_view}.rs`、`tests/view_test.rs`，`planner_test` 校准 1 处，视图相关修改面与 Act 声明一致、无未报告的意外面；叠加用户明示「测试可信」，采信不重跑。Act 流程注记（首次全量期间源码变化 → 该证据作废并重跑重验）符合公共规则 › 验证纪律。
- **F7〔遗留 Minor 确认〕CREATE TABLE × 既有视图名 TOCTOU 复检仅计划期**：反向无执行期复检（CreateViewExecutor 有）——单进程 plan+execute 背靠背窗口，与既有 CREATE TABLE 面一致，Act 已如实记录。不构成缺陷台账准入，非阻塞。

**Evidence**

- 承载面：catalog kind 尾段追加于 secondary 段后（`serialize_catalog_row:876-884`），兼容读三分支（缺段/截断/无效 utf8 → Table，`deserialize_catalog_row:1028-1045`，legacy 行残余字节安全回落）；`insert_view_row` 零页指针/空 PK/`column_count` 列数；视图行对表域不可见不变量在 `open_or_init:236`、recovery 三点、lifecycle `:217/:261/:422`（restore 守卫仅数 Table 行）四处闭合；database 载入 column_index 排序加固（`database.rs:96`）。
- 计划与展开面：修饰拒绝矩阵逐项在 `build_create_view`（`ddl_dml.rs:1234-1387`，含输出列空/重复、清单长度/重复、`MAX_VIEW_SQL_BYTES=3000` 前置）；FROM 钩子 `query.rs:344` 与 JOIN 右臂 `:435` 均在 CTE 未命中后、`validate_table` 前（CTE > 视图 > 基表不变量）；DML 三臂 `reject_view_write` 先于 `validate_table`（`:173/:1602/:1694`）；`build_create_table` 视图名守卫 `:815`；IN 臂 `subquery.rs` DerivedScan 首列名；豁免只对 flat 名且居 attach 路由后（`pipeline.rs:2215`）。
- 执行面：普通与 in-tx 两直包臂均 `plan_cache.clear()`（`pipeline.rs:367-382/:842-870`，in-tx 沿 CREATE TABLE 现状无 0A000，D13）；panic 臂 `:1258-1265`；两执行器 TOCTOU 复检 + 持久化先于注册登记（失败零副作用）；穷尽臂 plan_format/correlated/get_plan_output_columns 齐全。
- 测试面：`view_test.rs` 15 例与 R1-S1…S7 / R2-S1…S3 / R3-S1…S3 场景映射头注逐项对应（含 CLI 表头形态、CLI attach 引用拒绝 e2e、先建缓存再 DROP 的旧计划复活封死见证、多处零副作用断言、重开两态）。
- 验证面：见 F6 采信说明。

**Acceptance Gaps**

None——`sql-views` R1（7 场景）/ R2（3 场景）/ R3（3 场景）全部有可观察见证，RTM Covered，既有矩阵零回归（`cte_test`/`derived_scan_alias_test`/`tx_statement_test` 等），全量与静态门新鲜，计数自洽。

**Follow-up Decision**

接受（accepted）：六项偏差全部核实为非阻塞（Plan 遗漏两处均由 Act 正确补全，测试面两处为白名单事实）；遗留 Minor F7 维持 Act 记录，无需当前 Cycle 修复，无 rework/replan。本 Iteration 完成，本 change 六个 Iteration 全部 accepted，进入收尾（增量规格合并、SNAPSHOT/tasks 同步、归档由 `openspec-docs-maintainer` 承担）。

**Next Cycle**

None

**Next Iteration**

None（本 change 最后一个 Iteration；无剩余 Iteration）
