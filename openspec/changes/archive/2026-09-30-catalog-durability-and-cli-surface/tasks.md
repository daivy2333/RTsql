# catalog-durability-and-cli-surface — Tasks

## 任务清单

| Task | 目标 | Iteration | Requirement 映射 |
|---|---|---|---|
| T1 | `SlottedPage` 等长原地更新原语：`update_slot_in_place(logical_id, data)`（按 logical_id 定位 slot、等长校验、覆写数据区、header 不动不回写）+ 单测（等长覆写字节级断言、不等长拒绝、logical_id 缺失拒绝、header 字段不变断言） | 000 | catalog-update-persistence R1 |
| T2 | `update_field_in_chain` 重构：等长分支走原地写回；变长分支走链上跨页追加（`append_to_chain` 复用）+ 同名旧副本清理 + `scan_tables` 同名去重（logical_id 最大者）；删除页满静默 `Ok(())` 分支，全部失败 `Err` 携表名传播 + catalog 单测（≥200 次更新循环零泄漏零副本增长、重开持久化、窒息页原地更新成功、CREATE INDEX 增段跨页持久、双副本确定性读） | 000 | catalog-update-persistence R1/R2/R3 |
| T3 | 跨进程 durability 回归 e2e：`tests/catalog_durability_test.rs`——lib 形态 15 轮 `open → INSERT 1000 行（单语句）→ close` 断言行数单调、终值 15000、dump 行数一致、首/中/尾点查命中；CLI 形态抽样 12 轮（`rtsql <db>` 独立进程）；单进程 import 大批量对照（本就正确零回归）。修复前同测试见证 RED（行数冻结复现） | 000 | catalog-update-persistence R4/R6 |
| T4 | 索引根持久化专项（同文件）：UNIQUE 列大量插入促唯一树根分裂 → close/reopen → 重插同值仍拒绝；二级索引同型分裂 → 重开等值查询仍路由 IndexScanAll 且结果与扫描等价；`DROP INDEX` 可达 | 000 | catalog-update-persistence R5 |
| T5 | FROM 别名注册：`PlanBuilder.table_aliases` 映射 + `resolve_table_qualifier` 归一化 + 四注册点（FROM 基表臂 / JOIN 右臂 / CTE·视图 effective 名臂 / attach 复合键豁免）+ 单测（自连接两别名、别名×attach 键共存） | 001 | sql-table-alias-resolution R1 |
| T6 | 限定列解析统一：四消费点接归一化（`resolve_column_ref`/`resolve_qualified_column`、`qualified_column_expression` 无布局臂与 NLJ 布局臂、JOIN 输出过滤与 `resolve_join_projection_indices`、布局表名构造）；未知限定符 `TableNotFound(qualifier)` 点名；单表终止静默丢弃限定符；三段限定仅 attach 复合键可达（本地点名拒绝）+ `tests/table_alias_resolution_test.rs`（矩阵：×SELECT/ON/WHERE/GROUP BY/聚合参数 × 单表/JOIN/CTE/视图基座 + 拒绝面） | 001 | sql-table-alias-resolution R2/R3/R4 |
| T7 | ORDER BY 限定键：`extract_column_name` 两段扩展 + 两条构建路径（query.rs:682 扫描臂 / 1999 聚合·JOIN 臂）排序键解析经归一化 + Sort 装配到输入列索引 + `tests/order_by_alias_sort_test.rs` 扩展（限定键排序/DESC/组合/未知限定键拒绝） | 001 | sql-table-alias-resolution R2（ORDER BY 面）、order-by-alias-sorting R2/R3（MODIFIED） |
| T8 | 既有读面回归与校准收口：order_by_alias_sort / join_output_resolution / aggregate_output_shape / derived_scan_alias / cte / no_from_select / subquery 全绿；依赖静默丢弃限定符的存量用例白名单校准（调查预期：别名已声明的形态经映射天然兼容，预期零或近零校准）+ `openspec validate` | 001 | sql-table-alias-resolution R5、join-output-shape-resolution R3（MODIFIED） |
| T12 | 锁冲突有界退避重试（design D10，2026-10-01 用户裁定 replan 扩入）：`FileStorage::open_with_key`/`open_readonly_with_key` 的 `WouldBlock` 分支同 fd 重试（2ms × 4 次重试，总窗口 ~8ms）后仍 `DatabaseLocked`（错误面不变）+ `tests/database_file_lock_test.rs` 增重试耗尽拒绝与 close→立即重开循环用例 + 默认并行度套件循环复跑统计见证（修复前 RED 已由 Plan 复现：6 轮 2 败）| 001 | database-file-lock R1（MODIFIED） |
| T9 | dump 链完整性预检：`TableManager` 只读 helper（head→next 收集，终止页 ≠ catalog tail ⇒ 不完整）+ `dump_db` 接线（stderr 一行告警 + rc 0 + 继续导出）+ 测试（干净库零告警逐字节一致；页级字节修补夹具构造断链库 → 告警 + 管道纯 SQL） | 002 | cli-noninteractive-shell「dump 数据链完整性预检」（ADDED） |
| T10 | 主命令显式开库：`execute_command` 主命令臂 `resolve_db_path` 后存在性校验（缺失 → General exit 1 + `rtsql new` 指引，零文件创建）；`new` 豁免；生命周期子命令与 lib API 不变 + cli_test 机械校准（存量直接 CREATE TABLE 的新库用例先 `new`，断言语义不变）+ 拒绝矩阵用例 | 002 | cli-noninteractive-shell「主命令打开不存在库显式拒绝」（ADDED） |
| T11 | `rtsql-docs/SKILL.md` 命令行功能目录重构：主命令全 flag（`--format`/`--key`/`--readonly`/`:memory:`/多语句）、10 生命周期子命令逐条一行示例、env 三通道、退出码表（0/1/2/3/4/5/130/143）、本 change 新语义（缺库拒绝/dump 告警/别名支持）；README 双语对应段核验一致；真值来源 = `src/cli/mod.rs` `Command` + specs | 002 | 文档任务（无 spec 映射，验收 = 文档与实现逐条一致） |

## Iteration Plan

### Iteration 000: catalog 持久化正确性（P0 静默丢数据收口）

- Tasks: T1, T2, T3, T4
- Depends on: None
- 交付（2026-09-30 Act Response `reported` + Plan Review `accepted`）：T1 `update_slot_in_place` 等长原地更新原语（header 不触，窒息页可写）；T2 `update_field_in_chain` 两趟重构（等长原地 / 变长 `append_to_chain`+`delete_named_copies_except` 精确清理 + `scan_tables` 同名去重，静默 `Ok(())` 分支删除，失败携表名传播）+ 计划外修复 `append_to_chain` 旧 `free_page` 跨页追加丢行缺陷；T3 `tests/catalog_durability_test.rs` 15 轮 lib + 12 轮 CLI durability（修复前 RED：第 5 轮冻结 4822/5000）+ 五面一致；T4 UNIQUE/二级索引两段会话重开保持。新增 14 例（lib 10 + 集成 4），全量 1604 / 0 failed。两条 Minor（collapse tie-break 跨页回绕、`db/` 孤儿已清理）见 Cycle Plan Review。逐项见 `iterations/000-catalog-durability/000-initial.md`。
- Stable baseline: catalog 等长更新原地持久化（窒息页恢复更新能力、零泄漏零副本增长）；变长更新跨页重定位且同名恰一行；多轮跨进程写入行数单调、五面（count/扫描/MAX/点查/dump）一致；UNIQUE 与二级索引根分裂重开保持强制与路由；全量零回归。修复前 T3 同测试可见证 RED（行数冻结）。
- Verification boundary: `tests/catalog_durability_test.rs` 全绿（含 RED→GREEN 记录）+ catalog/page_format 单测 + 受影响边界（schema_persistence / checkpoint_redo_reduction / checkpoint_skip / drop_table_free / truncate / secondary_index / constraint_enforcement / upsert 回滚面）+ 全量分批（M19）。
- Diagnostic boundary: `src/storage/page_format/slotted_page.rs`、`src/storage/catalog.rs`（`update_field_in_chain`/`append_to_chain`/`scan_tables`）、`src/storage/data/table_manager.rs`（调用方错误传播面）。
- Non-goals: 页格式变更、catalog 紧凑化/VACUUM、既有损坏库修复工具、WAL/checkpoint 链路改动。

### Iteration 001: 表别名解析统一（P1 限定名家族收口）

- Tasks: T5, T6, T7, T8, T12（T12 为 2026-10-01 replan 扩入——锁冲突有界重试，见 design D10；T5-T8 主体已由 000-initial Cycle 实现并经 Plan Review 审定，剩余 Gap 与 T12 由 001-replan Cycle 承接）
- Depends on: None（与 000 无代码耦合；顺序体现正确性红线优先）
- 交付（2026-10-01 双 Cycle 收口：000-initial Act `reported` + Review `replan-required`〔Gap 1/2/3〕→ 001-replan Act `reported` + Plan Review `accepted`）：T5/T6/T7/T8 主体（别名注册 + 四消费点归一化 + 声明限定符向量 + ORDER BY 限定键）+ replan 承接 T6 补全（GROUP BY 与聚合参数两通道限定符点名，含无聚合路径 `validate_group_by_qualifiers`）+ T8 NLJ 见证 + T12 锁冲突有界退避重试（`try_lock_with_backoff`，~8ms 窗口，错误面不变）。全量 1626 / 0 failed；详见 `iterations/001-table-alias-resolution/{000-initial,001-replan}.md`。
- Stable baseline: FROM 别名全子句面解析可用（自连接两别名 × SELECT/ON/WHERE/GROUP BY/聚合参数/ORDER BY × 单表/JOIN/CTE/视图基座）；未知限定符与本地三段限定计划期点名——**含 GROUP BY 与聚合参数两通道**（000-initial Review Gap 1）；ORDER BY 限定键真实排序；既有读面零回归（白名单外零校准）；锁冲突拒绝语义不变且平台释放窗口下 close→立即重开可达（T12）。
- Verification boundary: `tests/table_alias_resolution_test.rs` 全绿 + order_by_alias_sort 扩展 + 既有读面测试族回归（T8 清单）+ `tests/database_file_lock_test.rs` 全绿 + 全量分批（M19）+ 默认并行度 readonly_session/alter_table 套件循环复跑统计见证（T12）。
- Diagnostic boundary: `src/parser/planner/query.rs`（FROM/JOIN 构建、投影过滤、ORDER BY、GROUP BY 与聚合参数解析臂）、`src/parser/planner/expression.rs`（两解析体与 build_expression 臂）、`src/parser/ast.rs`（qualified 提取）、`src/storage/file_storage.rs`（两处 try_lock 分支）。
- Non-goals: 窗口函数/LATERAL/标量子查询投影/递归 CTE；attach 两段表引用语义变更；表达式项 × JOIN 解禁；锁重试窗口可调 API/env 通道。

### Iteration 002: CLI 面显式化与文档（P2 + P3 + SKILL.md）

- Tasks: T9, T10, T11
- Depends on: Iteration 000（dump 预检针对的损坏形态由 000 根因修复，预检为纵深防御）、Iteration 001（文档覆盖别名语义）
- Stable baseline: dump 对断链库告警（stderr、rc 0、stdout 纯 SQL）且干净库逐字节不变；主命令缺库显式拒绝（exit 1 + 指引、零文件创建）且 `new`/生命周期子命令/lib API 不变；SKILL.md 命令行功能目录与实现逐条一致，README 双语无冲突。
- Verification boundary: 新增 dump 告警与缺库拒绝用例全绿 + cli_test 全量（含机械校准）+ SKILL.md 每条命令人工核对与抽样自动执行 + 全量分批（M19）。
- Diagnostic boundary: `src/storage/data/table_manager.rs`（helper）、`src/cli/lifecycle.rs`（dump_db）、`src/cli/mod.rs`（主命令臂）、`rtsql-docs/SKILL.md`、`README.md`/`README.zh-CN.md`。
- Non-goals: 通用完整性校验命令（MS27-T02）；生命周期子命令锁模式与开库语义变更；SKILL.md 以外的文档重构。

## 平衡审计

- 000（存储层四任务，单测试文件 + 单测内聚）：单一故障域（catalog 持久化），独立验证边界（durability e2e + 既有存储测试族），中等工作量——不拆。
- 001（planner 层四任务）：同一解析模型（别名归一化）贯穿四消费点，拆开会产生半成品解析面（注册了别名但部分子句仍拒绝），必须整体交付——不拆；与 000 不同故障域、不同诊断面——不合。
- 002（CLI 两项 + 文档）：两项均为 CLI 行为显式化、共享 cli_test 验证边界，文档任务消费两者语义，合并为收尾批（MS26 Iteration 003 收尾批先例）——不拆。

## Requirements Traceability Matrix

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| catalog-update-persistence R1 | T1, T2 | slotted_page/catalog 单测（等长覆写、窒息页更新、200 次循环零泄漏） | Covered |
| catalog-update-persistence R2 | T2 | catalog 单测（页满非等长 Err、tail 推进失败零副作用） | Covered |
| catalog-update-persistence R3 | T2 | catalog 单测（CREATE INDEX 跨页持久、双副本确定性读） | Covered |
| catalog-update-persistence R4 | T3 | `catalog_durability_test.rs`（15 轮 lib + 12 轮 CLI，RED→GREEN） | Covered |
| catalog-update-persistence R5 | T4 | `catalog_durability_test.rs`（UNIQUE/二级分裂重开矩阵） | Covered |
| catalog-update-persistence R6 | T3, T4 | 全量分批（M19）0 failures | Covered |
| sql-table-alias-resolution R1 | T5 | alias 单测 + `table_alias_resolution_test.rs` R1 组 | Covered |
| sql-table-alias-resolution R2 | T6, T7（replan 契约承接剩余面） | `table_alias_resolution_test.rs` R2 组 + order_by_alias_sort 扩展 + GROUP BY/聚合参数两通道点名与可达回归用例（replan 补 witness） | Covered |
| sql-table-alias-resolution R3 | T6 | 单表别名/未知限定符用例组 | Covered |
| sql-table-alias-resolution R4 | T6 | 三段本地拒绝 + ATTACH 三段零回归用例 | Covered |
| sql-table-alias-resolution R5 | T8 | T8 测试族清单全绿 | Covered |
| cli-noninteractive-shell ADDED（dump 预检） | T9 | 断链告警 + 干净库逐字节用例 | Covered |
| cli-noninteractive-shell ADDED（主命令显式开库） | T10 | 拒绝矩阵 + new 通道 + lib/子命令零回归用例 | Covered |
| order-by-alias-sorting R2/R3（MODIFIED） | T7, T8 | 限定键排序用例 + 既有排序回归 | Covered |
| database-file-lock R1（MODIFIED，replan 扩入） | T12 | `tests/database_file_lock_test.rs` 重试耗尽拒绝 + close→重开循环用例 + 默认并行度套件循环统计见证 | Covered |
| join-output-shape-resolution R3（MODIFIED） | T6, T8 | 别名限定投影用例 + 既有 JOIN 投影回归 | Covered |
| 文档一致性（T11，无 spec 映射） | T11 | SKILL.md 命令逐条人工核对 + 抽样自动执行 | Covered |
