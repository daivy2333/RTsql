# MS21 DDL 演进与索引面 — Tasks

> 由 openspec-plan 维护；全部任务分配到 6 个逻辑 Iteration，只展开当前 Iteration 目录。2026-09-29 用户指令扩围：T6/T7（来源 Iter000 Act Response Issue 候选 1/2，design D16/D17）插入为 Iteration 001，原 001–004 顺延为 002–005。

## 任务清单

### T1 索引条目维护一致性收口（正确性红线）

- [x] T1-① `abort_cleanup_versions` A 趟 rekey 回退修复（design D1）：PK 臂与唯一索引臂改「删反查键 + 按 prev tuple 派生键条件回插」，非 rekey 快路径保持 `update` 逐字节不变；新增 rekey 回滚见证（PK/唯一列 × 运行期/干净重开/崩溃恢复两态）
- [x] T1-② `delete_conflict_row` SlotNotFound 容忍臂对齐（design D2）：仲裁命中 PK 键穿入，slot 不可读仍删条目；直构悬空条目夹具见证
- [x] T1-③ 既有回滚还原矩阵零回归（`explicit_tx_test` R6 六矩阵、`upsert_test`、`mvcc_tombstone_visibility_test`）

### T2 CREATE/DROP INDEX 二级索引（单列、非唯一、INT 列）

- [x] T2-a catalog 登记段（design D4）：`CatalogRow` 尾段 `sec_count | (name, col_index, root)*` 兼容读 + `update_secondary_index_root` + `CatalogRootSlot::Secondary` + `TableMeta.secondary_indexes` + open_or_init/attach contexts/replace_recovery 扩展
- [x] T2-b `IndexManager::delete_exact`（design D3）：包装 `BTree::delete_exact` + 反向映射精确清理；单测直构
- [x] T2-c CREATE INDEX 计划臂与执行器（design D5）：白名单臂 + 形态拒绝矩阵（UNIQUE/组合/表达式/部分/IF NOT EXISTS/非 INT 列/重名/表列缺失）+ 回填（链尾非墓碑）+ 登记时机 + 事务内拒绝
- [x] T2-d 写路径挂钩六处（design D7）：insert/update/delete/upsert 四执行器 + abort_cleanup A/B 趟 + 回滚还原见证
- [x] T2-e 恢复重建扩展（design D8）：rebuild 五步链 + 非唯一不判重 + 两态断言
- [x] T2-f DROP INDEX 执行器（design D4/D14）：登记移除 + 树页释放 + 回落扫描 + 不存在/IF EXISTS 拒绝
- [x] T2-g 查询路由（design D6）：`table_secondary_indexes` 通道 + 非 PK 臂简单等值 → `IndexScanAll` + EXPLAIN 形状 + 结果等价与零回归

### T3 ALTER TABLE ADD/DROP COLUMN

- [x] T3-a 宽度感知反序列化 + DEFAULT 覆盖层（design D9）：tuple 层新变体（实际值数）+ 全部消费点切换 + recovery extract 同源
- [x] T3-b ADD COLUMN 计划臂与执行器（design D9）：尾部加列 + 校验矩阵（NOT NULL/DEFAULT/约束拒绝/表非空判定）+ catalog `insert_column_row`/column_count + TableMeta 重建换入 + dump/restore 保真
- [x] T3-c DROP COLUMN 计划臂与执行器（design D10）：守卫矩阵（索引引用/最后一列/非基表/缺失）+ 内隐式事务逐行重写 + catalog 列行删除与 ordinal 重写 + 两态与崩溃恢复见证
- [x] T3-d ALTER 语句形态（design D13）：多操作拒绝、事务内拒绝、目标校验

### T4 TRUNCATE TABLE

- [x] T4-a 计划臂与执行器（design D11）：checkpoint-first 序 + head 重置/尾页释放/tail 回写 + 三类索引根重置 + 事务内/多表/非基表拒绝 + 幂等
- [x] T4-b 恢复语义见证：TRUNCATE 后崩溃重开行不复活（WAL 无待重放）+ 既有 DELETE/DROP TABLE 零回归

### T5 CREATE/DROP VIEW

- [x] T5-a catalog kind 尾段与视图注册表（design D12）：`CatalogRow` kind+SQL 文本尾段、`__columns` 复用列名、open_or_init 载入、内存注册表
- [x] T5-b CREATE VIEW 计划臂与执行器（design D12）：创建期 `build_query` 验证（引用存在性/循环结构性拒绝/列名清单/长度上限）+ 重名/OR REPLACE/MATERIALIZED/附加库引用拒绝
- [x] T5-c 展开与消费（design D12）：`register_table` 视图闸门 + planner FROM/JOIN 钩子 CTE 通道展开 + DML 只读拒绝 + `get_subquery_first_column` DerivedScan 臂 + plan cache 全清
- [x] T5-d DROP VIEW 执行器 + 组合面见证（JOIN/聚合/ORDER BY/嵌套/重启持久）

### T6 删除原语精确化（DELETE 未命中键零副作用）— 正确性红线，2026-09-29 用户指令扩围

- [x] T6-a `LeafNode::delete` 精确键验证（design D16）：删除前验证命中位键与目标键相等（对齐 `LeafNode::update` 既有验证形态），缺失键返回 `KeyNotFound`；歧义「插入位删除」原语对所有调用者消除
- [x] T6-b `DeleteExecutor` 未命中早退（design D16）：`search` 未命中返回 `AffectedRows(0)`，不触碰墓碑/唯一/PK 条目/WAL；命中路径（含 SlotNotFound 容忍臂）逐字节不变
- [x] T6-c 见证与零回归：未命中两形态（中间缺失键误删邻近 / 大于全部键 KeyNotFound 误报）RED→GREEN + 既有 delete/upsert/恢复矩阵零回归

### T7 回滚还原补口——SET 唯一列为 NULL 的旧键条目还原 — 正确性红线，2026-09-29 用户指令扩围

- [x] T7-a A 趟唯一臂 NULL-rekey 还原（design D17）：唯一反查 None + prev 存在时经 `read_prev_index_keys` 派生 prev 唯一键并 `insert` 还原（与 D1「prev 键位 NULL 仅删除」臂镜像对称）；单语句形态
- [x] T7-b 见证与零回归：值→NULL 回滚后旧值占用恢复（同值 INSERT DuplicateKey）RED→GREEN + 重开两态一致 + 既有矩阵零回归

## Iteration Plan

### Iteration 000: 索引条目维护一致性收口

- Tasks: T1-①②③
- Depends on: None
- Stable baseline: rekey UPDATE 回滚后 PK 与唯一索引条目与存活版本一致（旧键点查可达、新键无残留、双键 INSERT 可用、重开两态一致）；UPSERT 冲突删除 slot 不可读态与 DELETE 对齐；既有回滚矩阵零回归
- Verification boundary: 新增 rekey 回滚与夹具见证 + `explicit_tx`/`upsert`/`mvcc_tombstone_visibility`/`update_index_maintenance` 既有矩阵 + 全量串行零回归
- Diagnostic boundary: `src/transaction/manager.rs`、`src/executor/upsert.rs`（对照 `src/executor/delete.rs`）、`tests/{explicit_tx,update_index_maintenance,upsert,mvcc_abort}_test.rs`
- Non-goals: 二级索引（001）；写侧 rekey 执行器改动（Forbidden）

### Iteration 001: 删除原语精确化与回滚 NULL-rekey 还原（2026-09-29 用户指令扩围）

- Tasks: T6-a…c、T7-a…b
- Depends on: 000（复用其 `read_prev_index_keys` 派生机制与 upsert 在位守卫先例；T6 修复的歧义原语正是 000 容忍臂守卫所防御的缺陷）
- Stable baseline: DELETE 未命中键零副作用（affected 0、邻近条目不误删、无 KeyNotFound 误报）且命中路径逐字节不变；SET 唯一列为 NULL 的回滚还原旧值占用，重开两态一致；既有矩阵零回归
- Verification boundary: 新增用例（`update_index_maintenance_test`/`explicit_tx_test` 扩展）+ `upsert`/`mvcc_tombstone_visibility`/`mvcc_abort`/`keyless_row` 既有矩阵 + 全量分批零回归
- Diagnostic boundary: `src/storage/btree/node.rs`、`src/executor/delete.rs`、`src/transaction/manager.rs`
- Non-goals: 多语句同行链回滚处理序（pre-existing Minor）；二级索引重复键 `delete_exact`（002 的 T2-b）；DELETE 非 PK 等值形态（计划期拒绝现状）

### Iteration 002: CREATE/DROP INDEX 二级索引端到端

- Tasks: T2-a…g
- Depends on: 001（T6 精确化原语使「二级索引写路径禁用歧义原语」的地基成立）；000（A/B 趟回滚清理形态是二级索引回滚还原的地基）
- Stable baseline: 单列 INT 二级索引建/删/维护/恢复/路由端到端可用：建索引回填与重启/崩溃两态一致；四执行器与回滚路径条目一致；等值查询路由 `IndexScanAll` 且结果等价；DROP INDEX 回落扫描；拒绝矩阵全点名
- Verification boundary: 新增 `tests/secondary_index_test.rs`（或等价分组）+ `EXPLAIN` 形状断言 + 恢复两态 + 全量串行零回归
- Diagnostic boundary: `src/storage/{catalog.rs,data/table_manager.rs,btree/}`、`src/wal/recovery.rs`、`src/executor/{insert,update,delete,upsert,create_index?,drop_index?}.rs`、`src/parser/planner/{mod,ddl_dml,query}.rs`、`src/pipeline.rs`、`src/transaction/manager.rs`
- Non-goals: 范围扫描/索引序/覆盖索引；组合与表达式索引；dump 携带索引；TRUNCATE 联动（004 承接）

### Iteration 003: ALTER TABLE ADD/DROP COLUMN

- Tasks: T3-a…d
- Depends on: None 硬依赖（建议序在 002 后——DROP 守卫消费二级索引登记；宽度感知层先行使 002 的索引读路径同享）
- Stable baseline: 尾部加列存量行按 DEFAULT/NULL 读回、新行全宽、dump→restore 保真；任意位置删列经内隐式事务重写后 schema/数据/索引一致、崩溃恢复全有或全无；拒绝矩阵全点名
- Verification boundary: 新增 `tests/alter_table_test.rs` + 宽度感知层在既有扫描/恢复套件零回归 + dump/restore 往返 + 全量串行零回归
- Diagnostic boundary: `src/storage/page_format/tuple.rs`、`src/storage/{catalog.rs,data/table_manager.rs}`、`src/executor/`（新 alter 执行器与扫描消费点）、`src/parser/planner/ddl_dml.rs`、`src/pipeline.rs`、`src/wal/recovery.rs`
- Non-goals: RENAME/改列类型；在线并发语义；ADD 的 NOT NULL 回填

### Iteration 004: TRUNCATE TABLE

- Tasks: T4-a…b
- Depends on: 002（三类索引根重置含二级）
- Stable baseline: TRUNCATE 物理清空保留全部定义与索引结构（空索引）、跨重启持久、行不复活；拒绝矩阵与幂等成立
- Verification boundary: 新增 `tests/truncate_test.rs` + 崩溃重开不复活见证 + `drop_table_free` 页释放模式断言 + 全量串行零回归
- Diagnostic boundary: `src/storage/data/table_manager.rs`、`src/executor/`（新 truncate 执行器）、`src/parser/planner/ddl_dml.rs`、`src/pipeline.rs`、`src/database.rs`（checkpoint 接线）
- Non-goals: 多表 TRUNCATE；TRUNCATE 与活跃快照读者的细粒度 MVCC 语义

### Iteration 005: CREATE/DROP VIEW

- Tasks: T5-a…d
- Depends on: None 硬依赖（建议序殿后——独立查询面，复用 003 后的稳定基线）
- Stable baseline: 视图创建/查询/嵌套/持久化端到端可用，结果与内联派生表一致；DML 只读拒绝、循环结构性不可能、定义变更后解析一致、IN 子查询 DerivedScan 臂补齐
- Verification boundary: 新增 `tests/view_test.rs` + 拒绝矩阵 + 持久化重开 + `cte_test`/`derived_scan_alias_test` 零回归 + 全量串行零回归
- Diagnostic boundary: `src/storage/catalog.rs`、`src/database.rs`（视图注册表）、`src/parser/planner/{mod,query,subquery}.rs`、`src/parser/planner/ddl_dml.rs`、`src/pipeline.rs`、`src/executor/`（新 view 执行器）
- Non-goals: 可更新视图/物化视图/CHECK OPTION；dump 携带视图；运行期环检测（创建期验证结构性封死）

## 与 OpenSpec 同步

- 全部任务经本 change 承载；`MS21-T01/T02/T03` 对应本 change T2/T3/T1，TRUNCATE 与 VIEW 为用户 2026-09-29 扩围裁定，T6/T7 为同日二次扩围（Iter000 Act Response Issue 候选 1/2，用户指令「把这两个 issue 计划解决」并入，同属索引一致性正确性主题；tasks.md MS21 条目届时由 docs-maintainer 收尾同步）
- 两个 Issue 候选经本扩围直接进入实施计划，不再作为独立缺陷台账条目强制登记；如需 `.agents/issues/` 台账记录由用户另行指令 `openspec-experience-recorder`
- 完成后经 `openspec-docs-maintainer` 收尾合并增量规格（4 新域 + `update-index-maintenance` MODIFIED/ADDED）并归档
