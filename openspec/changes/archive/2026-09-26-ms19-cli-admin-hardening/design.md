# MS19 CLI 管理面小收口 — Design

## D1: `rtsql delete <db>` 删库子命令

**决策**：新增 `Command::Delete { db: String, dry_run: bool }`（`#[arg(long)]`），实现在 `src/cli/lifecycle.rs` 新函数 `delete_db(db: &str, dry_run: bool) -> ExitStatus`（同步函数——不开库、纯文件操作，`execute_command` 匹配臂直接调用不加 `.await`）。

**锁探测与删除同窗口（消除 TOCTOU）**：复用引擎既有 advisory 锁原语 `std::fs::File::try_lock`（`src/storage/file_storage.rs:71` 同款）：

1. `resolve_db_path` 定位 + `exists()` 检查（`resolve_existing_db` 同语义：不存在 → `General("{path} does not exist")` exit 1）。
2. `File::open(db_path)`（只读）+ `file.try_lock()`：
   - `TryLockError::WouldBlock` → `ExitStatus::Locked("database is locked: {path}")`（与 `open_error_status` 消息形状一致）exit 4，**零删除**；
   - `TryLockError::Error(e)` → `General`（如权限/是目录）；
   - 成功 → **持有锁的窗口内**执行删除，完成后 drop 释放。持有独占锁期间不可能存在其他 RTsql 打开者，检查与删除之间无竞态。
3. 删除顺序：**`.wal` → `.checkpoint` → 主文件最后**。若中途崩溃，最坏状态是「主文件在而伴生已清」——主文件按最后 checkpoint 状态干净打开、可重试；反序（主文件先删）会在新库重建后残留可被恢复重放的旧 WAL，产生脏现场。
4. 逐文件报告（`emit_stdout`）：`deleted <path> (<n> bytes)` / `absent <path>`（伴生缺失跳过不算错误）；任何 `remove_file` 非 NotFound 错误 → 报告该文件 + 整体 `General` exit 1。主文件大小在删除前经 `metadata().len()` 读取。
5. `--dry-run`：同样的清单输出（`would delete <path> (<n> bytes)` / `absent <path>`），**不探测锁、不打开文件、不修改任何状态**——dry-run 是纯预览而非 preflight（对打开中的库 dry-run 成功而实删失败是预期差异）。

**替代方案与拒绝理由**：
- 经 `Database::open` / `FileStorage::open` 探测锁——拒绝：open 会初始化 0 字节文件头（对目标库产生变异）、校验头部（损坏库不可删，违背「文件级管理操作」语义）、触发 WAL 恢复（重且不必要）。
- TTY 确认 / `--force` 门槛——拒绝（用户裁定 2026-09-26）：one-shot 主消费形态是 agent/脚本，交互确认破坏一行式定位；删库由命令本身的明确性承担。
- 复用 `install.sh --purge-data`——拒绝：purge-data 是卸载场景的整目录清理，delete 是单库管理操作；边界在 README 文档化，无代码交互。

**已知边界（写入 spec）**：裸名 `delete` 的库与子命令冲突时子命令优先（既有文档化语义，主命令形态不可达）；不做库内容校验（头无效/零字节可删，不初始化不校验）；加密库删除与明文库无差别（不触及密钥通道）。

## D2: PlanError 携带能力名

**决策**：`UnsupportedStatement(String)` / `UnsupportedExpression(String)`——payload 为能力名；`Display` 追加 `: {name}`（`Unsupported statement type: MERGE`），保留既有文案框架。

- 构造点共 31 处（UnsupportedStatement 15：ast.rs ×9、planner/mod.rs ×2、query.rs ×1、ddl_dml.rs ×3；UnsupportedExpression 16：expression.rs ×10、aggregate.rs ×4、subquery.rs ×1、ddl_dml.rs ×1）。每处按被拒形态传字面量能力名。
- 顶层分派 catch-all（`planner/mod.rs:229`）经新辅助函数从 `stmt.to_string()` 首词提取语句关键字（sqlparser Display 以大写关键字开头，如 `MERGE INTO ...` → `MERGE`）；`mod.rs:226`（Drop 非 Table 臂）传 `format!("DROP {}", object_type)` 形态。辅助函数放 `ast.rs`（与既有 AST helper 同域）。
- 测试断言面已核实：`contains("Unsupported statement type")` 形态断言 7 处全部存活（追加后缀不破坏 contains）；其余 28 处为注释引用，不修改。

**替代方案**：`&'static str` payload——拒绝：顶层 catch-all 的名字来自运行时语句首词，需 `String`。不改 payload 改独立 variant——拒绝：spec R2 要求两变体本身携带。

## D3: PG 协议错误码分类

**决策**：分类函数 + 类型化失败通道 + Response 载荷扩展，三层闭合。

1. **映射表**：`PlanError::sqlstate(&self) -> &'static str`（`src/parser/error.rs`）：
   - `ParseError` → `42601`（syntax_error）
   - `UnsupportedValue` / `UnsupportedStatement` / `UnsupportedExpression` / `UnsupportedJoinType` / `UnsupportedConstraint` / `UnsupportedSubqueryPosition` / `InSubqueryJoinUnsupported` / `TransactionStatement` → `0A000`（feature_not_supported）
   - `TableNotFound` → `42P01`；`ColumnNotFound` / `GroupByColumnNotFound` → `42703`；`AmbiguousColumn` → `42702`
   - 其余全部 → `58000` 兜底
2. **通道**：`Response::Error { message: String, sqlstate: Option<String> }`，serde 属性 `skip_serializing_if = "Option::is_none"`——JSON 协议输出在 None 时**逐字节不变**（网络协议兼容零破坏）；None 经 PG 层回落 58000。现有 23 处 `Response::Error` 构造点全部补 `sqlstate: None`（引擎内部错误维持兜底），仅类型化漏斗点携带真实分类。
3. **漏斗**：`plan_stage` 返回类型改为 `Result<PhysicalPlan, PlanFailure>`；新 `PlanFailure { message: String, sqlstate: &'static str }`（pipeline.rs）实现 `Display`（→ message，调用方文本零变化）与 `From<PlanError>`（`message = format!("Plan error: {e}")`、`sqlstate = e.sqlstate()`——保持既有 "Plan error: " 前缀框架）。调用方适配：`run_sql`（`&e` → `&e.to_string()`）、`execute_inner` plan 臂（携带 `Some(sqlstate)`）、`execute_in_tx` / `fetch_table_rows`（Display 机械适配）。`parse_stage` 签名不变（返回 String），`execute_inner` parse 臂分类为 `42601`（含 "Empty SQL" 退化形态——解析类失败统一归 syntax_error）。
4. **PG 出口**：`pg_protocol.rs:306` 由硬编码 `"58000"` 改为 `sqlstate.as_deref().unwrap_or("58000")`；`pg_messages::error_response(severity, code, message)` 签名不变。

**替代方案**：新增 `Response::SqlError` variant——拒绝：JSON 协议消费方需感知新 variant，兼容面大于可选字段；消息内嵌机器码前缀由 PG 层剥离——拒绝：改变用户可见文本（违背 R4）；typed 错误直达 handler——拒绝：改 SqlHandler 契约，影响面大于 PlanFailure 通道。

**兼容性**：`pg_messages_test.rs:158` 断言的 58000 是 `NetworkError` 映射（IO/协议写错误），维持不变（未分类兜底语义的一部分）。

## D4: 会话边界组合 e2e（I042）

`tests/tx_statement_test.rs` 沿既有 `run_cli` 夹具（`CARGO_BIN_EXE_rtsql` + TempDir CWD/RTSQL_HOME + 管道捕获）新增两用例，锁定既有行为（组合路径由 `run_sql` Err 分支 + `sql_failure_status` 事务后缀 + 收尾回滚组合产生，已由 MS11-T02 Plan Review 探针验证）：

- `BEGIN; INSERT INTO t VALUES (1); SAVEPOINT sp1` → exit 3、stderr 含 `SAVEPOINT` 与 `not committed` 后缀、重开查询无残留；
- `BEGIN; INSERT INTO t VALUES (1); SET TRANSACTION ISOLATION LEVEL READ COMMITTED` → 同构断言。

纯测试加固，产品代码零改动。

## D5: 文件对备份边界（I072）

**实测矩阵**（Act 执行，结论与关键输出记入 Act Response；命令可低成本重跑，不建 Evidence 目录）：

| 形态 | 预期 |
|---|---|
| 干净关闭后只拷 `.db` | 打开成功、数据完整（close 即 checkpoint，WAL 已截断） |
| 有未 checkpoint 写入时只拷 `.db` | 副本丢失尾部事务（静默不完整） |
| 同状态下拷 `.db` + `.wal` 成对 | 打开副本经 WAL 重放，数据完整 |
| 成对移动/重命名（同名不同目录） | 配对保持、完整 |
| 只重命名主文件（`.wal` 不随） | `with_extension` 配对按文件名派生，WAL 失配、尾部事务丢失 |

**文档化**：README.md 与 README.zh-CN.md 增「备份与文件对」段——备份首选 `dump`/`restore` 逻辑导出；文件级拷贝的等价条件（干净关闭/checkpoint 后）与成对规则（`.db`+`.wal` 同名配对、重命名需同改、`.checkpoint` 同属文件族）；`rtsql delete` 与 `install.sh --purge-data` 的边界（单库管理 vs 卸载清目录）。

## 实现顺序与依赖

Iteration 000（T1：delete + I072）独立于 Iteration 001（T2+T3：错误面 + 会话 e2e），无共享文件面（前者只触 CLI/README，后者触 parser/pipeline/network）。Iteration 内 T2 先于 T3 无硬依赖，但 T3 的错误文案断言依赖 D2 落地后的最终文案形状，故 T2 先行。

## D6: 纯列路径投影严格解析与别名表头生效（Iteration 002 / T4）

**现状与根因**（Plan 真二进制探针 + 代码调查，2026-09-26）：`resolve_projection_indices`（`src/parser/planner/query.rs:1597`）对任一未匹配基列的投影名返回 `None` → identity 投影。注释把「别名/表达式/聚合」与「未知列」混为同一兜底桶，导致：未知裸列/限定列（含混合已知+未知）静默返回全列全数据；`col AS alias`（纯非聚合查询）的别名被当基列名解析失败 → 静默 identity，表头退化全 schema。`projection_columns` 来源 `extract_columns`（ast.rs）——限定名取列尾段、`ExprWithAlias` 取别名，故 `t2.id` 可解析而 `t2.nosuch`/`n` 不可解析。

**决策（两部分）**：

1. **未知列显式拒绝**：纯列路径（`has_aggregates == false && subquery_evals.is_empty() && !has_expression_items` 且 `base_plan` 为 `Scan`）下，`projection_columns` 各项（`*` 通配除外）SHALL 全部可解析到基 schema 列；任一不可解析 → `PlanError::ColumnNotFound(该项名)` 计划期拒绝（exit 3，PG 42703）。`resolve_projection_indices` 保留 `None` 语义仅用于 `*`/空清单；未知列不再走 identity 回落。
2. **别名路由（仅纯非聚合查询）**：角色分类循环后、路由裁决前，若 `!has_aggregates && !building_subquery`，把 `ExprWithAlias(plain column)` 项从 `Column` 角色改标 `Expression`（`has_expression_items = true`）——使这类查询走既有顶层 `ProjectionExecutor` 通路（与 `id + 1 AS d` 同路径）：表头 = 别名、行 = 投影列，未知列经 `build_expression` 得 `ColumnNotFound`。**聚合查询不改分类**——聚合路径已正确处理别名（`agg_output_columns.push(alias)`，探针 `SELECT name AS n, COUNT(*) FROM t2 GROUP BY name` → `["n","count_star"]`），改分类会破坏混合投影校验。

**替代方案与拒绝理由**：
- 在纯列路径内把别名映射回源列、仅修行投影——拒绝：表头仍无法表达别名（`projected_columns` 用基列名），需要新增 per-item 命名管道，改动面大于复用 Projection 通路。
- 全量改标 `ExprWithAlias(plain)` 为 Expression（含聚合查询）——拒绝：破坏聚合混合投影的角色校验（表达式项 SHALL 解析到 GROUP BY 键，别名 ≠ 键名时误拒既有可用形态）。

## D7: 值表达式列缺失重分类 ColumnNotFound（Iteration 002 / T5）

**现状**：`build_expression` 单表路径（`src/parser/planner/expression.rs:283` 裸名、`:340` 两段限定名）对列缺失构造 `PlanError::ParseError("Column '{}' not found in table '{}'")`；同函数多表/NLJ 布局路径（`:271`/`:324`）已是 `ColumnNotFound`——同一错误两类建模，PG 分类 42601（syntax_error）而非 42703（undefined_column）。grep 全量确认无任何测试断言 `not found in table` 文案。

**决策**：两处改构造 `PlanError::ColumnNotFound(column_name)`——文案随之变为 `Column not found: {col}`（与 `:271`/`:324` 既有形态一致，表名信息不再携带；SQLite 同类文案 `no such column: x` 亦不含表名），PG 分类自动经既有映射得 42703（`error.rs:143`，映射表与 27 变体单测零修改）。`Table '{}' not found` ParseError（`:277`/`:334`，表达式内表缺失）不在范围——FROM 解析已先行拦截，仅畸形形态可达。

## D8: ORDER BY 排序键计划期校验（Iteration 002 / T6）

**现状**：`query.rs:1126-1137` 构造 `OrderByColumn` 不做 schema 校验；`SortExecutor::compare_rows`（`src/executor/sort.rs:80`）按 `position()` 找键，`None` 静默跳过该键（探针：`ORDER BY nosuch` 与 `ORDER BY nosuch, name DESC` 均不排序不报错）。

**决策**：在 Sort 节点构造点（`:1156` 前）校验每个 order_by 键的**可解析性 = 键与 `sort_columns`（`:1145-1149` 已算出的 Sort 真实可见列集，与 SortExecutor 同源）或顶层投影项输出名（`select.projection` 中任一 `ExprWithAlias` 的别名，大小写不敏感）之一匹配**；两者均不匹配 → `PlanError::ColumnNotFound(key)` 计划期拒绝。输出别名键（匹配投影输出名）不拒绝也不解析为排序列——SortNode 照常携带，`SortExecutor` 既有 None 跳过臂使其保持输入序（2026-09-26 Review 修订：字面「仅 sort_columns」版本与已验收语料 `sql-scalar-functions` R5 锁定的「ORDER BY 引用表达式项别名静默保持输入序、SHALL NOT 改变」冲突，全量唯一失败项 `order_by_expression_alias_keeps_input_order` 即该形态；收窄依据是本变更 delta R4「新增拒绝面 SHALL NOT 影响白名单外既有可达形态」）。不新增执行器臂、不改 `SortExecutor`、不实现按别名排序。既有可达形态零回归：`sort_columns` 即 D10 设计下 Sort 的真实输入列集——基础扫描链含未投影基础列（`SELECT id FROM t ORDER BY name` 可达保持）、表达式查询基础列可达（探针 `SELECT id + 1 AS d FROM t ORDER BY name DESC` 正确排序保持）、聚合查询按投影列名（`ORDER BY count_star` 可达保持）、输出别名键保持输入序（`ORDER BY u` / `ORDER BY n` 与变更前一致）。

## D9: 无聚合 GROUP BY 整体被忽略（记录，不实施）

探针证实 `SELECT name FROM t2 GROUP BY name`（无聚合）不分组不去重——GROUP BY 在无聚合时整体不进入任何分支（有效键与未知键同样静默）。带聚合时未知键经非聚合列校验已有错误。此为**未实现语义缺口**（标准语义：按键去重的分组投影），非列解析错误建模；修复需先裁定语义（实现去重 vs 计划期点名拒绝），独立于本变更。按 Act 候选与本次调查报告为 Issue 候选，由 Recorder 落账、后续路线裁定。

## 风险

- D3 的 23 处 `Response::Error` 构造点为机械修改（`sqlstate: None`），遗漏会编译失败（穷尽性），无静默风险。
- D2 顶层 catch-all 首词提取依赖 sqlparser Display 行为（大写关键字开头）；对罕见 variant 若首词不理想，能力名可读性下降但不失真（非实质，Act 调整个别字面量）。
- delete 锁探测在 NFS 等不支持 flock 的文件系统上行为与 FileStorage 一致（同原语），不引入新的平台差异面。
- D6 别名路由的条件改标（仅 `!has_aggregates && !building_subquery`）若误扩到聚合查询会破坏混合投影校验——契约以探针锁定形态（`name AS n, COUNT(*) GROUP BY name` 零回归）作 GREEN 条件。
- D6/D8 使原 exit 0 静默形态变为 exit 3——属用户批准的行为变更（proposal 扩围段），README 无需同步（无用户文档承诺过静默行为）。例外（2026-09-26 Review 修订）：D8 的输出别名键（`ORDER BY u` / `ORDER BY n`）被语料 `sql-scalar-functions` R5 锁定为既有静默语义，维持静默不变、不纳入拒绝面。
