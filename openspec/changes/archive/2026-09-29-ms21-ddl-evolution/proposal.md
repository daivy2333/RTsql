# MS21：DDL 演进与索引面（二级索引 + ALTER TABLE + TRUNCATE + VIEW + 索引一致性收口） — Proposal

## Why

- **DDL 面最大功能缺口**（R18 主题 2/3，I068 promoted 为 MS21）：`ALTER TABLE`、`CREATE INDEX`/`DROP INDEX` 全部不支持——列结构演进只能 dump→改 DDL→restore 重建整库；非 PK 查询列无索引，等值点查退化为全扫。`TRUNCATE`、`CREATE VIEW` 同为不支持（R18 差距清单），经用户 2026-09-29 集中裁定并入本 change。
- **正确性红线**（MS24 Review 遗留，tasks.md MS21-T03）：既有索引条目在删除与回滚路径两处不一致——① rekey UPDATE 回滚后旧键条目缺失、新键条目指向旧键版本，旧键等值点查漏行（MS24 001-replan Review Minor 2）；② `upsert.rs::delete_conflict_row` 的 `SlotNotFound` 容忍臂跳过索引条目删除，与 `delete.rs` 同形态不对齐（MS24 Issue 候选 1 / 父 Cycle Minor 4）。按「正确性先于新功能」编排纪律，阶段内最先执行。
- **单 change 多 Iteration 承载**（用户 2026-09-29 裁定）：MS21 全部范围入本 change，按依赖序分 Iteration 独立验收。

## What Changes

用户已裁定（2026-09-29，四项决策）：**拆分=单 change 多 Iteration**；**CREATE INDEX 列类型=仅 INT 列**；**ALTER TABLE=ADD 与 DROP 都入范围**（保留 split signal）；**CREATE VIEW 与 TRUNCATE 均并入**。

- **T1 索引条目维护一致性收口**（`update-index-maintenance`，MODIFIED）：① `TransactionManager::abort_cleanup_versions` A 趟对 rekey UPDATE 回滚的条目回退按 rekey 语义处理（现以 `find_key_by_row_id` 所得新键执行 `update(key, prev)`，而 prev 版本 tuple 携带旧键 → 回滚后新键条目指向旧键版本、旧键条目缺失、旧键等值点查漏行）；② `upsert.rs::delete_conflict_row` 的 `SlotNotFound` 容忍臂与 `delete.rs` 同形态对齐（无元组可读时仍按搜索键执行 `index_manager.delete`）。
- **T2 CREATE/DROP INDEX 二级索引**（`sql-secondary-indexes`，新）：单列、非唯一、INT 列二级索引——catalog 持久化登记、存量行全量索引化、INSERT/UPDATE/DELETE/UPSERT 全写路径条目维护（含回滚还原）、等值谓词查询路由可达（EXPLAIN 可观察）、干净重开与崩溃恢复两态一致（恢复重建通道扩展）、DROP INDEX 物理释放。非 INT 列 / UNIQUE / 组合 / 表达式 / 部分索引 / `IF [NOT] EXISTS` 计划期点名拒绝。
- **T3 ALTER TABLE ADD/DROP COLUMN**（`alter-table-column-evolution`，新）：尾部加列（存量行按 DEFAULT/NULL 读回填充，不强制全表重写）、删列（被索引引用拒绝；物理策略与 split signal 随 Phase 2 设计定稿）；约束与语句形态诚实化拒绝；dump/restore/import 面保真。
- **T4 TRUNCATE TABLE**（`truncate-table`，新）：物理释放数据页（DROP TABLE 同型通道），保留表定义、列结构、约束与索引定义（空索引重建）；显式事务内点名拒绝（物理释放不可回滚）；单表形态。
- **T5 CREATE/DROP VIEW**（`sql-views`，新）：非物化只读视图，定义持久化（catalog 演进）、查询期名称解析展开为派生表（复用 MS25 DerivedScan 通道）、视图可引用基表与既有视图（循环拒绝）、DML 与对视图的 DDL 点名拒绝；`OR REPLACE`/`MATERIALIZED`/附加库引用点名拒绝。
- **T6/T7 删除原语精确化与回滚 NULL-rekey 还原**（`update-index-maintenance`，MODIFIED 扩围；**2026-09-29 用户指令**「把这两个 issue 计划解决」，来源 Iter000 Act Response Issue 候选 1/2，design D16/D17）：① `LeafNode::delete` 歧义「插入位删除」精确化 + `DeleteExecutor` 未命中键零副作用（affected 0、不误删邻近条目、不误报 KeyNotFound）；② `abort_cleanup_versions` A 趟唯一臂对 SET 唯一列为 NULL 的回滚还原旧键条目（占用语义恢复）。经新 Iteration 001 承载（正确性红线，先于二级索引），原 001–004 顺延为 002–005。

## 默认假设（Gate 1 审阅项；可逐项推翻）

1. **索引名库级唯一**：`DROP INDEX <name>` 不带表名限定（sqlparser 0.44 `DropIndex` 形态以 Phase 2 复核为准）；同名重复 `CREATE INDEX` 点名拒绝。
2. **二级索引首版仅等值路由**：被索引列 `WHERE col = <Int 字面量>` 路由到该索引（与 PK 键位等值路由同型，含 MS16 类型门语义——不可键控字面量回落行内求值）；范围扫描、`ORDER BY` 利用索引序、覆盖索引不在范围。非唯一索引重复键多行全部命中（B-Tree 重复键承载方案随 Phase 2 设计定稿）。
3. **PK 列建二级索引不 specially-case**：允许（冗余但行为等价），不做冗余检测。
4. **二级索引列必须声明 INT**：索引列写入非 Int 值已被 MS24 写入类型门拒绝（`ColumnTypeMismatch`），无需新增守卫。
5. **ADD COLUMN 形态**：仅尾部追加；支持类型声明与 `DEFAULT`/`NOT NULL`；存量行读回按 DEFAULT（无 DEFAULT → NULL）填充；`NOT NULL` 无 DEFAULT 且表非空 → 点名拒绝；ADD 中声明 `PRIMARY KEY`/`UNIQUE`/`CHECK`/FK → 点名拒绝（唯一索引演进不在范围）。
6. **DROP COLUMN 边界**：被 PK/唯一/二级索引引用的列点名拒绝；表至少保留一列；物理策略（惰性截断 vs 全行重写）随 Phase 2 调查定稿。**split signal 保留**：若 DROP 需全行重写且改造成本超预期，DROP 拆出独立 change，本 change 先收 ADD。
7. **ALTER 语句形态**：单语句单操作；`ALTER TABLE ... ADD COLUMN ..., DROP COLUMN ...` 多操作点名拒绝；`ALTER TABLE RENAME`/改列类型不在范围。
8. **TRUNCATE 语义**：与 DROP TABLE 同型物理边界——不产生墓碑、不可回滚，显式事务内点名拒绝；多表 `TRUNCATE t1, t2` 点名拒绝（首版单表）；`TRUNCATE TABLE t` 与 `TRUNCATE t` 两形态等价。
9. **视图语义**：只读（DML 计划期点名拒绝）；展开为派生表语义（MS25 DerivedScan 同型，视图内 ORDER BY/LIMIT/聚合按派生表既有语义）；列名取显式列清单，缺省取查询输出表头；视图可引用基表与既有视图，循环引用计划期点名拒绝；`CREATE OR REPLACE`/`MATERIALIZED`/视图定义引用附加库（`alias.table`）点名拒绝（attach 为会话作用域，持久化视图引用会悬空）。
10. **`IF [NOT] EXISTS` 子句**：CREATE INDEX / DROP INDEX / DROP VIEW 首版一律不支持，点名拒绝（诚实化，避免静默吞并）；后续按需放开。
11. **DDL 显式事务边界（Phase 2 修订，design D13，Gate 2 待批）**：Phase 2 调查发现 CREATE INDEX（回填一致性）与 ALTER TABLE（DROP 的重写事务一致性）的数据面语义依赖无并发未提交写窗口，而现状是显式事务内 DDL 立即执行、窗口不成立——修订为：**CREATE INDEX / DROP INDEX / ALTER TABLE 在显式事务内计划期点名拒绝（0A000，CTAS 先例同型）**；CREATE VIEW / DROP VIEW 为 catalog-only，沿既有 CREATE TABLE 现状（立即生效、不随回滚撤销，`tx_statement_test.rs:348-365` 既有语义）；TRUNCATE 维持拒绝（假设 8）。
12. **plan cache**：T2–T5 全部 DDL 沿用既有 DDL 失效面（执行后失效或不进 cache，与 CREATE/DROP TABLE 同型）；引用视图/索引的查询计划在对应 DDL 执行后不得复活旧计划。
13. **视图持久化**：catalog 演进向后兼容读（对齐 MS23 唯一索引根段 / MS24 DEFAULT 段先例——系统表新增表或既有行尾随扩展段，Phase 2 设计定稿）；视图数量与嵌套深度不设人为上限（循环拒绝已封死自引用）。

## 范围外（Non-goals）

- 变长 Key 编码（I024 方向）与全类型/String 列二级索引（Key 32B 定长与 String(255) 容量是格式演进，另议）。
- 部分索引（WHERE）、表达式索引、组合（多列）索引、`UNIQUE` 二级索引（唯一性约束走建表声明，MS23 语义）。
- 范围扫描与 ORDER BY 的索引序利用、覆盖索引、代价模型（I016）。
- 可更新视图（DML 穿透视图）、物化视图、触发器、CHECK OPTION。
- `ALTER TABLE RENAME`/改列类型/改表名；在线 schema 变更的并发语义精细化。
- ALTER/TRUNCATE/VIEW 与 ATTACH 附加库的组合（附加库只读边界外的一切 DDL 已由 MS20 拒绝；视图引用附加库见假设 9）。
- split signal：DROP COLUMN 全行重写过大时拆出（见假设 6）。

## Impact

- **受影响代码面**（Phase 2 调查复核；T6/T7 扩围补充）：`src/storage/catalog.rs`（索引与视图元数据登记、列演进）、`src/storage/data/table_manager.rs`（create/drop/truncate 索引通道、列演进数据面、读回填充）、`src/storage/btree/`（二级索引管理、重复键承载；T6：`node.rs` 叶删除精确化）、`src/wal/recovery.rs`（恢复重建扩展二级索引）、`src/parser/planner/`（ddl_dml.rs 四类 DDL 臂 + query.rs 视图展开与索引路由）、`src/pipeline.rs`（执行器接线）、`src/executor/`（新 DDL 执行器 + insert/update/delete/upsert 二级索引维护钩子；T6：`delete.rs` 未命中早退）、`src/transaction/manager.rs`（T1-①、T7）、`src/cli/lifecycle.rs`（dump/schema 渲染新对象）。
- **受影响 specs**：新增 `sql-secondary-indexes`、`alter-table-column-evolution`、`truncate-table`、`sql-views` 四域；MODIFIED `update-index-maintenance`（T1 收口 + T6/T7 扩围 Requirements）；`cli-noninteractive-shell`/`schema-persistence` 是否需要 MODIFIED 以 Phase 2 调查定稿。
- **测试**：新增二级索引 / ALTER / TRUNCATE / VIEW 测试文件 + 扩展 `update_index_maintenance_test`/`explicit_tx_test`；全量零回归（change 创建时基线 1458 tests / 0 failures / 2 ignored；Iter000 后 1464 / 0 / 2i）。

## 验证边界

- T1：rekey 回滚两态矩阵（PK + 唯一列，运行期/重开/崩溃恢复）+ upsert 冲突删除夹具态对齐 + MS24 既有回滚还原矩阵零回归。✅ Iter000 accepted（2026-09-29）
- T6：DELETE 未命中两形态（中间缺失键误删邻近 / 大于全部键 KeyNotFound 误报）RED→GREEN + 命中路径与容忍臂零回归。
- T7：SET 唯一列为 NULL 的回滚占用恢复 RED→GREEN + 重开两态一致 + 既有回滚矩阵零回归。
- T2：建/删索引 e2e + 等值路由计划形状 + 重复键多行命中 + 写路径四执行器条目维护 + 回滚还原 + 干净重开与崩溃恢复两态 + 拒绝矩阵。
- T3：加列矩阵（DEFAULT/NOT NULL/约束拒绝）+ 存量行读回填充 + 删列边界矩阵 + dump→restore 往返 + 重启持久。
- T4：清空后查询/schema/持久 + 事务内拒绝 + 多表拒绝 + 幂等。
- T5：创建/查询/嵌套/循环拒绝/持久 + DML 拒绝面 + 定义变更后解析一致 + 拒绝矩阵。
- 全量 `cargo test -- --test-threads=1` 零回归；`cargo clippy -- -D warnings`、`cargo fmt --check`、`openspec validate` 通过。
