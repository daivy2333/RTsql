# 任务与逻辑迭代

（计划于 2026-10-13 经用户批准——Gate 1 与 Gate 2 合并批准，Iteration 000 Plan Context 已转 ready。）需求基线与用户决策（D-1/D-2/D-3，2026-10-13）见 proposal；实现调查与设计决策见 design.md D1–D10。复选框表示实施状态。ISS05 机制记录的调查修正（回填无辜、下探漏洞 + B1/B2 新缺陷）见 proposal Why 与 design D2/D3——ISS05 台账正文的机制段以本 change 调查为准（台账更新归 experience-recorder 按用户指令）。

**工作面划分**：T1 为**验证面**（测试基建）；T2–T6 为**代码面**（行为修复，各有 spec delta）；T7 为**文档面**（README.md / README.zh-CN.md / rtsql-docs/SKILL.md）。

## 1. e2e 旅程验证面

- [x] 1.1 **T1〔验证〕** e2e 旅程套件——填充 `tests/e2e_test.rs`：夹具（R31/cli_test 模式：`CARGO_BIN_EXE_rtsql` 子进程 + TempDir/RTSQL_HOME + 60s 兜底）+ E1–E13 旅程（proposal 场景草图；design D1）。命中 ISS05–ISS14 的断言点标 `#[ignore = "ISSxx 待 Tn 修复"]`；非缺陷面全绿；P2-10/11 按 D-1 锁定现状断言；套件纳入稳定基线。依赖 None。验收：单目标 `cargo test --test e2e_test` 全绿（ignore 项除外）+ 既有 119 集成目标零回归 + 套件建设期新暴露缺陷按 D-3 分流（相邻顺带修入对应任务、扩围报批、其余 Issue 候选报告）。（Act 2026-10-13 完成：22 测试函数 12 passed / 0 failed / 10 ignored〔37.49s〕+ `--ignored` 10/10 pre-fix RED 见证；夹具三处自检修复〔管道并发读防死锁、两处假阳性断言〕；check_command/cli_test 抽样回归零失败；Plan Review accepted，三项非阻塞 Minor 记录在案）

## 2. 索引与树结构正确性（ISS05 + B1/B2，P0）

- [x] 2.1 **T2〔代码〕** 等值下探 + 分隔键落位 + 树遍历原语 + 叶链保真——① `IndexManager::search_all_from_page_async` 按 design D2 闭区间下探（Cycle 000 已接入，本轮不改消费公式）；② `BTree::delete_exact` 同集合下探（Cycle 000 已接入）；③ `LeafNode::shift_slots_right` 恢复叶链（Cycle 000 已完成）；④ `IndexManager::scan_all` 树遍历（Cycle 000 已完成，页号 0 是合法子页）；⑤ `InternalNode::insert_separator` 把新分隔键拼在被分裂子页的紧右边，内部节点分裂后的待插入分隔键进入仍持有该子页的一半，`update_parent_separator` 原槽改键不重排（design D2 审计修正）；`search`/`insert` 的 `find_child_page_id_binary` 不动；同步 `BTree::search_all` 死代码不动；⑥ 失败语句索引零副作用回归。R2（`sql-secondary-indexes` MODIFIED ×2），S1–S6 + E4/E13 相关点。依赖 T1（解除 E4 ignore）。验收：新 `tests/secondary_index_equality_test.rs`（30000 行多叶等值连串 COUNT/DISTINCT vs DataScan 基线双向严格相等 + 边界子树 UPDATE 探针 + 失败语句零副作用 + 唯一键/范围/复合谓词零回归）+ btree 层单测（shift 保真、scan_all 多层树全量、delete_exact 全条目可达、等值分隔键落在被分裂子页右侧且子树键不超过下一分隔键）+ `full_table_dml_test` 大表多层树用例（affected_rows=行数）+ scan.rs/table_manager 消费面回归 + e2e E4 解除 ignore RED→GREEN。

## 3. DDL 持久化与原子性（ISS06 + ISS08 + ISS14，P0/P1）

- [x] 3.1 **T3〔代码〕** DDL 有序落盘 + 建表写序重排 + DEFAULT 预检——① `BufferPool::flush_pages`/`flush_all_except` 两原语（M17 collect-then-write 约束）；② `create_table_with_constraints` catalog 持久段重排「列行先、表行最后 = commit point」+ 失败清理（内存不注册/孤儿列行清理/已分配页释放）+ ALTER ADD COLUMN 同理（design D5）；③ DDL 通道有序冲刷接线（create/CREATE INDEX 注册/CREATE VIEW/ALTER ADD；CTAS 与 import 经 create 路径继承；DROP 类不接线，design D4）；④ `build_create_table`/`build_alter_table` DEFAULT 序列化尺寸预检点名（design D6）。R3，S7–S9 + E3b/E13 相关点。依赖 T1（解除 E3b/S9 ignore）。验收：新崩溃恢复矩阵测试（自动建表 import + SIGKILL → 重开可达 + check 干净；CREATE TABLE/INDEX/VIEW/ALTER/CTAS 各形态建后即崩恢复；对照：已存在表崩溃恢复既有面零回归）+ DEFAULT 预检矩阵（各类型边界值/点名文案/零副作用/同名可重试）+ 建表失败零残留四步（报错/schema 无表/INSERT 拒绝/重建成功）+ checkpoint_skip/check_command/catalog_durability/recovery_* 既有套件零回归 + e2e E3b/S9 解除 ignore RED→GREEN。（Act 2026-10-13 完成：三则 e2e ignore 解除 pre-fix RED 见证→全 GREEN；新增 `tests/ddl_crash_recovery_test.rs` 6 例〔create/index/view/alter/ctas/对照〕+ `tests/default_precheck_test.rs` 5 例〔超限点名/边界放行/全类型/ALTER 同款〕；catalog `insert_table` 写序重排〔列行先、表行最后、新页链入时 prev tail 并入 commit set〕+ `flush_commit_point` 编排；`append_secondary_index`/`update_table_column_count_ddl` 接线、DROP 类不接线；`open_or_init` 按 `column_count` 裁剪孤儿列行；`catalog_column_row_fits` 空页探针预检〔无硬编码常数〕；三处既有用例按新语义校准〔checkpoint_skip S4 更名、wal_auto_checkpoint pure_select、recovery_error_surface S22 存量砖形手工夹具〕，逐条记录；M19 全量 13 批 1566 passed / 0 failed + lib 398 + bins 0；详见 Cycle 000 Act Response）

## 4. planner 读面（ISS07 + ISS09，P1）

- [x] 4.1 **T4〔代码〕** 派生表/视图内层输出列外层可见——`building_from_derived` 放行模式扩展到 query.rs:1365（DISTINCT）/1572/1585/1614/1626/1677 五门 + 视图展开路径置位（design D7）；IN/EXISTS/标量上下文维持抑制（错误面逐字节不变）；CTE 通道零改动对照。R4，S10–S11 + E11 相关点。依赖 T1。验收：`derived_scan_alias_test` + `view_test` 扩展（别名/表达式/DISTINCT × 派生/视图/CTE 三通道矩阵 + IN/EXISTS 内维持拒绝）+ cte/subquery/set_operations 既有套件零回归 + e2e E11（ISS07 部分）解除 ignore RED→GREEN。（Act 2026-10-13 完成：视图展开 `resolve_view_reference` 与 FROM 派生同型置位 + 分类门统一 `inner_suppressed = building_subquery && !building_from_derived`〔DISTINCT 门 + 五道分类门 + `alias_reroutable`〕；IN/EXISTS/标量三入口复位逐字节保持〔query.rs 标量投影位 + subquery.rs 三入口，save/restore 严格配对〕；derived_scan_alias_test +5〔S1 别名/S2 表达式/S3 内层 DISTINCT 去重/S4 三通道逐字节一致/S5 表达式子查询拒绝面逐字节〕+ view_test +4〔别名/表达式/DISTINCT/嵌套视图〕；cte/subquery/set_operations 单目标零回归；e2e `e11_inner_columns_outer_visible` 解除 ignore GREEN——先行 RED 由 Iteration 000 `--ignored` 10/10 见证采信）
- [x] 4.2 **T5〔代码〕** SemiJoin/AntiJoin 全宽承载与排序键——`output_columns` 改基表全列 + `build_output_row` 直通 + 下游裁剪沿 T3 `sort_projection` 先例；`get_plan_output_columns`/`resolve_projection_indices`/排序键校验消费面对齐（design D8）；字面量 IN/EXISTS/相关形态零回归。R5，S12 + E11 相关点。依赖 T1（与 T4 同 Iteration 无执行依赖，后置避免 query.rs 双触同段）。验收：`subquery_test` 扩展（ORDER BY 非输出列 × IN/EXISTS/相关 + 投影形状回归 + 形状断言白名单校准逐条记录）+ cli_test 表头对照 + e2e E11（ISS09 部分）解除 ignore RED→GREEN。（Act 2026-10-13 完成：`build_full_output_columns_for_table` 全列装配〔原 `build_output_columns_for_table` 契约内等价改名、`projection_columns` 参数退役签名保持〕+ `SemiJoinNode`/`AntiJoinNode`/`JoinRelatedConfig` 增 `projection: Vec<usize>` 裁剪位〔空 = 恒等全宽，`apply_projection` 共享原语〕+ 两执行器 `build_output_row` 左行直通后裁剪 + 无 ORDER BY 时节点 `projection` 裁剪〔query.rs WHERE 子查询臂〕/有 ORDER BY 时 `sort_projection` 增 SemiJoin/AntiJoin 臂（索引按基表 schema 序解析）+ `get_plan_output_columns`/pipeline `plan_output_names` 两表头消费面同款对齐 + correlated.rs 参数注入只遍历子节点、节点 `projection` 天然保留；subquery_test +6〔S1 非输出列 ASC/DESC 真实排序/S2 与无子查询·字面量 IN 三形态同序/S3 无 ORDER BY 投影形状零全宽泄漏 + 多列投影/S4 EXISTS 基座/S5 NOT IN AntiJoin 基座/S6 相关 IN〕+ cli_test +1 表头对照〔无 ORDER BY/ORDER BY 非输出列/多列投影三形态 columns 与 rows 逐位对齐〕；无 ORDER BY 形状断言零校准——输出恒 SELECT 清单，校准白名单为空；e2e `e11_semijoin_order_by_non_output_column` 解除 ignore GREEN——先行 RED 同上采信）

## 5. 文案批与文档同步

- [x] 5.1 **T6〔代码〕** ISS10–13 四点——help 用户视角改写（cli/mod.rs:107-114）；错误前缀单层（parse_stage 去前缀 + 漏斗点单层赋予，pipeline.rs:113/error.rs:74，断言校准）；`column_type_sql` 标准长名（lifecycle.rs:1025-1034 + dump 断言校准 + 往返/多代幂等）；busy-timeout env 代码侧校验点名 `RTSQL_BUSY_TIMEOUT`（沿 WAL 先例，flag 优先/退出码不变，RTSQL_KEY 同族核对）。R6，S13 + E12 相关点。依赖 T1（解除 E12 ignore）。验收：cli_test 扩展（help 无内部标记/单层前缀各路径/dump 类型名矩阵/env 非法值 × 三 env 族）+ 既有前缀/ dump 断言校准 + e2e E12 解除 ignore RED→GREEN。（Act 2026-10-14 完成：四则 e2e pre-fix RED 见证〔解除 ignore 前 `--ignored` 4/4 failed〕→解除 ignore→全 GREEN〔套件 22 passed / 0 failed / **0 ignored**，change 全部 ignore 清零〕；cli_test +3 新用例〔help 三形态无标记/文本+计划级单层前缀/busy-timeout env 三态〕+ 9 处渲染断言校准 + profile/stats 类型名 2 处；pipeline 单测校准 1 处〔parse_stage 裸文本断言化〕；lifecycle 单测校准 3 则；full_table_dml 双层前缀断言 2 处；ctas_test schema DDL 断言 5 处；import_input 4 处〔其中类型推导冲突文案 `(INT, STRING)` 为 Debug 渲染路径非 column_type_sql 面，恢复原名断言〕；RTSQL_KEY 同族核对结论「非误报成 flag，不动」；fmt/clippy 干净〔存量 1 条 Iter001 告警不变〕；定向 8 套件零回归；详见 Cycle 000 Act Response）
- [x] 5.2 **T7〔文档〕** 文档同步——README.md / README.zh-CN.md / rtsql-docs/SKILL.md：e2e 套件定位与运行方式、P2-10/11 输出形态声明（D-1）、类型名渲染规则、DDL 崩溃窗口关闭与剩余边界（drop 类、加密 WAL 明文窗口）、索引正确性声明成真。R7。依赖 T2–T6 行为定稿。验收：逐条人工核对（对照实测命令输出）。（Act 2026-10-14：README 双语五条已落盘，六组命令人工实测与实现回报一致。SKILL.md 五条与第 192 行页满 DEFAULT 句未落盘。2026-10-14 用户决定把该遗留与索引扫描首列误报一起放入下一轮，本任务不再补 SKILL.md，见 T9）

## 6. 技能文档补齐与 IN 子查询首列

- [x] 6.1 **T8〔代码〕** `get_subquery_first_column` 为 `IndexScan`/`IndexScanAll` 补臂——首列与 `get_plan_output_columns` 一致（design D11）。R8。依赖 Iteration 004 accepted。验收：`subquery_test` 新增裸名主键等值、非首列输出、二级索引等值、限定名对照、JOIN 拒绝保持。（Act 2026-10-14 完成：先行 RED 见证〔`Subquery returns multiple columns (IN subquery requires single column)`〕→两臂 GREEN；`IndexScan` 直取已收窄 `columns[0]`、`IndexScanAll` 按 `projected_columns` 规则裁剪后取首列〔query.rs 可见性 fn→pub(crate) 复用，语义逐字节同源〕；subquery_test +5〔S1 裸名主键等值/S2 非首列输出 item 表/S3 二级索引投影首列+基表首列对照/S4 限定名不变/S5 JOIN 拒绝文案〕；既有 r5 用例头部过期缺陷说明指向 T8 修正；fmt/clippy 干净〔存量 Iter001 告警 1 条不变〕；详见 Cycle 000 Act Response）
- [x] 6.2 **T9〔文档〕** 把 T7 未写入的五条补进 `rtsql-docs/SKILL.md`，并修正第 192 行超长 DEFAULT 句。R7 的剩余部分。依赖 T7 的 README 已落盘；与 T8 无代码依赖，放在 T8 之后以便文档收口时可以提到已修复的 IN 行为。验收：五条能在文件中定位，第 192 行不再写页满错误。（Act 2026-10-14 完成：五条落点——e2e 旅程套件命令〔Install and deploy〕、多语句 JSONL+DDL `{"affected_rows":0}`〔Select output and handle failures〕、`schema`/`dump` 标准长类型名〔schema 命令表+Create and inspect+Dump and restore〕、DDL 语句边界落盘与剩余崩溃窗口〔Behavioral notes〕、二级索引等值行集与全表扫描一致〔Evolve schema〕；原 192 行改为计划期点名 `DEFAULT value is too large for the catalog page capacity`，grep 全文无 page-full 残留；逐条与 README.md 对应段同义核对）

## Iteration Plan

### Iteration 000: e2e 旅程验证面

- Tasks: T1
- Depends on: None
- Stable baseline: 旅程级 e2e 套件存在且非缺陷面全绿（ignore 项承载已知缺陷标记）；为后续每个修复任务提供旅程级验证入口（解除 ignore 即 RED 见证）。
- Verification boundary: 单目标 `cargo test --test e2e_test` + 既有集成目标抽样回归（cli_test/check_command/recovery 类）——本 Iteration 不做全量（测试基建零产品改动，全量预算留给首个代码 Iteration 001）。
- Diagnostic boundary: `tests/e2e_test.rs`（新）+ `tests/cli_test.rs` 夹具模式参照。
- Non-goals: 任何产品代码修复（ISS05–ISS14 全部留给后续 Iteration）；性能断言。
- 平衡审计: 单任务体量中大（夹具 + 13 旅程 + ignore 标记矩阵）但产物单一内聚（「验证面存在且可用」一个可验收结果），拆分无独立验收价值；作为独立 Iteration 使后续修复任务的验收面（解除 ignore）稳定可用。

### Iteration 001: 索引与树结构正确性（P0）

- Tasks: T2
- Depends on: Iteration 000（E4 ignore 解除入口；基线 1929 + e2e 套件）
- Stable baseline: 二级索引等值查询与 DataScan 双向严格相等（任意规模）；多层树全表 DML/全量枚举正确；叶链保真；失败语句索引零副作用——为后续 Iteration 提供干净索引基线。
- Verification boundary: 新 secondary_index_equality_test + btree 单测 + full_table_dml 大表用例 + 消费面定向回归（secondary_index/update_index_maintenance/btree_*/scan 类）+ 全量（M19 分批，本 Iteration 预算一次）。
- Diagnostic boundary: `src/storage/btree/{index_manager,btree,node}.rs`（含 `insert_separator` / `insert_into_page` / `update_parent_separator`）、`src/executor/{index_scan_all,update,delete,scan}.rs` 消费面、对应测试。
- Non-goals: DDL 持久化（Iter 002）；planner 读面（Iter 003）；同步 search_all 死代码清理；存量文件迁移修复。
- 平衡审计: 单任务但为三缺陷（机制 A/B + B1/B2）一体的 btree 正确性主题，共享同一组原语改动与同一验证边界（严格相等 + 全量枚举），拆分会产生同文件双触；体量中-大但契约单一。

### Iteration 002: DDL 持久化与原子性（P0/P1）

- Tasks: T3
- Depends on: Iteration 001（独立故障域，顺序编排；e2e E3b ignore 解除入口）
- Stable baseline: DDL 成功即表定义持久（崩溃可达恢复）；建表失败零残留可重试；DEFAULT 超限计划期点名拒绝。
- Verification boundary: 新崩溃恢复矩阵 + DEFAULT 预检矩阵 + 建表失败零残留 + checkpoint/check/catalog_durability/recovery 定向回归 + 全量（本 Iteration 预算一次）。
- Diagnostic boundary: `src/storage/data/table_manager.rs`、`src/storage/buffer_pool.rs`、`src/storage/catalog.rs`、DDL 执行器接线、`src/parser/planner/ddl_dml.rs` 预检、对应测试。
- Non-goals: DROP 类 DDL 崩溃窗口（既有边界）；存量砖库解救；WAL DDL 记录。
- 平衡审计: 写序重排 + 有序冲刷 + 预检为同一结构不变量（表行 = commit point）的三面落地，共享 create_table_with_constraints 单点与同一崩溃测试边界；合并为单任务单 Iteration 形成「DDL 语句边界持久化与原子性」一个可验收结果。

### Iteration 003: planner 读面（P1）

- Tasks: T4, T5（执行序 T4 → T5）
- Depends on: Iteration 002（顺序编排；e2e E11 ignore 解除入口）
- Stable baseline: 派生表/视图内层别名与表达式列三通道一致可用（含内层 DISTINCT）；SemiJoin/AntiJoin 排序键语义与无子查询基座一致。
- Verification boundary: derived_scan_alias/view/subquery 扩展 + cte/set_operations/join_output_resolution/cli_test 定向回归 + 全量（本 Iteration 预算一次）。
- Diagnostic boundary: `src/parser/planner/{query,subquery}.rs`、`src/executor/{semi_join,anti_join}.rs`、消费面（get_plan_output_columns/resolve_projection_indices/排序键校验）、对应测试。
- Non-goals: IN/EXISTS/标量内集合操作与表达式项（维持抑制）；JOIN 基座 WHERE 子查询解锁。
- 平衡审计: 两任务同属 planner 读面收口、共享 derived/subquery 测试边界与 E11 验收入口；代码面相邻但契约独立、执行序避免 query.rs 同段双触；合为一轮「读面组合能力清零」。

### Iteration 004: 文案批与文档同步（收尾）

- Tasks: T6, T7
- Depends on: Iteration 003（T7 依赖 T2–T6 行为定稿；e2e E12 ignore 解除入口）
- Stable baseline: CLI 文案四点定稿；README 双语五条已同步；e2e 套件 0 ignore。SKILL.md 与 IN 子查询索引扫描首列留给 Iteration 005（2026-10-14 用户决定）。
- Verification boundary: cli_test 扩展与断言校准 + 文档逐条人工核对 + 全量（本 Iteration 预算一次，作为 change 收尾终验）。
- Diagnostic boundary: `src/cli/{mod,lifecycle}.rs`、`src/pipeline.rs`、`src/parser/error.rs`、三文档。
- Non-goals: 语料合并（docs-maintainer 收尾）；Issue 台账更新（recorder 按用户指令）。
- 平衡审计: 四点文案均为小改动面、共享 CLI 输出断言边界；T7 文档面依赖全部行为定稿故同 Iteration 收尾；合为「文案定稿 + 文档同步 + 终验」单一收尾结果。

### Iteration 005: 技能文档补齐与 IN 子查询首列

- Tasks: T8, T9（执行序 T8 → T9）
- Depends on: Iteration 004 accepted
- Stable baseline: 裸名主键等值的 `IN` 子查询按 SELECT 第一列匹配；`rtsql-docs/SKILL.md` 与 README 的五条陈述一致。
- Verification boundary: `subquery_test` 扩展 + 定向零回归 + SKILL.md 人工核对。全量一次，放在 T9 之后。
- Diagnostic boundary: `src/parser/planner/subquery.rs::get_subquery_first_column`；`rtsql-docs/SKILL.md`。
- Non-goals: 给 `Sort`/`Limit`/`Projection` 补首列臂；改索引路由；改 README；语料合并；Issue 台账。
- 平衡审计: 两任务故障域不同，按用户 2026-10-14 的决定放在同一轮。T8 先改代码，T9 只改技能文档。诊断边界分开，文档失败不回头改规划器。

## 执行序与依赖说明

000 → 001 → 002 → 003 → 004 → 005。每个代码 Iteration 的全量验证按 M19 分批执行（一次预算）；005 的全量在 T9 之后。
