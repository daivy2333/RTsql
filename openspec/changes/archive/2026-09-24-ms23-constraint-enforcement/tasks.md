# MS23 数据完整性约束执行面 — Tasks

> 里程碑：MS23（tasks.md 权威；统一执行序首位）。
> 用户裁定：T01+T02+T03 单 change 双 Iteration；UNIQUE 仅 INT 列强制（非 INT 点名拒绝）；表级单列 UNIQUE 映射、组合拒绝；存量非 INT UNIQUE 库 dump→restore 破坏接受并文档化。
> 验证边界：直接目标测试 → 受影响边界（恢复两态 / CLI 会话 / dump-restore）→ 全量 `cargo test`；逐场景最简直接判定，不建身份型证据工程。

## 1. Iteration 000 — 约束诚实化与 NOT NULL 强制（MS23-T02 + T03）

- [x] 1.1 列级约束诚实化：`extract_column_constraints`（`src/parser/planner/ddl_dml.rs:369`）对 `Check`/`ForeignKey`/`DialectSpecific` 返回点名 `PlanError::UnsupportedConstraint`；`Null`/`Comment` 维持忽略；`tests/planner_test.rs` 拒绝矩阵（CHECK/FK/方言项 × 拒绝 + NOT NULL/DEFAULT/PK 不受影响）。
- [x] 1.2 表级约束诚实化：`build_create_table`（`ddl_dml.rs:472`）新增表约束遍历，`TableConstraint::Check`/`ForeignKey`/Index 类变体点名拒绝；表级 `Unique` 不触碰（Iteration 001 D6）；planner_test 表级拒绝用例。
- [x] 1.3 TableMeta 约束标志：`TableMeta` 加 `pub not_null: Vec<bool>`，`create_table_with_constraints`（:280）/`open_or_init`（:187，自 `CatalogColumnRow.not_null` 读回）/`replace_index_manager`（:361）三构造点接线；catalog 单测补恢复读回断言。
- [x] 1.4 INSERT NOT NULL 强制：`InsertExecutor` coerce 后、键位类型预检前逐列校验，违反 → `StorageError::NullConstraintViolation { column }`（`src/storage/error.rs` 新变体）；`tests/constraint_enforcement_test.rs` 新建——拒绝点名、零副作用（行数/索引不变）、非 NULL 成功、PK+NOT NULL 组合拒绝、未声明列 keyless 零回归。
- [x] 1.5 UPDATE NOT NULL 强制：`UpdateExecutor` Step 1 后、写前对 SET 目标列校验同错误；constraint_enforcement_test 更新拒绝与原值保持用例。
- [x] 1.6 错误面与会话接线验证：cli_test——auto-commit INSERT 违反 exit 3 + 点名文本；显式事务内违反自动回滚后续可写；确认无专用 CLI 臂需求（Sql 失败路径渲染）。
- [x] 1.7 全量回归：`cargo test` 全绿；既有用例如需按新契约校准，逐条在 Act Response 记录理由。

## 2. Iteration 001 — UNIQUE 强制端到端（MS23-T01）

- [x] 2.1 catalog 表行追加演化：`serialize/deserialize_catalog_row` 尾部 `u32 unique_count | N × u32 roots` 兼容读写（旧行 → 空）；`Catalog::update_unique_index_root`；catalog 单测（新格式往返 / 旧行兼容 / root 写回保字段）。
- [x] 2.2 root 同步泛化：`IndexManager` catalog 上下文加槽位描述（PK 表根 / 第 N 唯一根），`sync_root_to_catalog` 分派；`attach_index_catalog_contexts` 逐唯一索引接线。
- [x] 2.3 TableMeta 唯一索引承载与生命周期：`unique_indexes: Vec<(usize, Arc<IndexManager>)>`；create 分配 / open_or_init 自 catalog 根 `from_root` / replace 扩展 swap / `drop_table` 逐唯一索引 best-effort 释放。
- [x] 2.4 UNIQUE DDL 策略面：列级 UNIQUE 非 INT 类型点名拒绝；表级单列 `UNIQUE(col)` 映射列标志（schema 渲染随之正确）；表级组合 UNIQUE 点名拒绝；planner_test 矩阵。
- [x] 2.5 INSERT 唯一强制：预检（NULL 跳过、命中 `DuplicateKey`）+ 数据落位后条目插入（D7 顺序）；constraint_enforcement_test——重复拒绝零副作用、不同值成功、多 NULL、多唯一列独立。
- [x] 2.6 UPDATE 唯一维护四分支：非唯一列随行 / 同值随行 / 改值碰撞预检 + rekey / 置 NULL 删条目；constraint_enforcement_test 对应矩阵（含改值后唯一性保持）。
- [x] 2.7 DELETE 与回滚修复：`DeleteExecutor` 逐唯一列删条目；`abort_cleanup_versions` 逐唯一索引同型修复；用例——DELETE 后重插、显式回滚后重插、事务内语句失败自动回滚后重插。
- [x] 2.8 恢复重建唯一索引：`rebuild_pk_indexes` 扩展（唯一列新索引 + 链尾回溯提取 + 跨链重复 `RedoFailed` 点名表列 + swap + 根写回 + 旧树释放）；两态用例——干净重开强制保持 / 崩溃恢复后强制保持 / 旧格式 catalog 行兼容打开。
- [x] 2.9 收口验证：dump→restore INT UNIQUE 往返强制保持；drop_table 页释放后同进程新建写入正常；`cargo test` 全绿；README（双语）约束语义与存量兼容边界说明。

## Requirements Traceability Matrix

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R1 NOT NULL 写入强制 | S1-S5 | D1/D2 | 1.3/1.4/1.5/1.6 | 000 | `table_manager.rs::TableMeta`、`insert.rs::next`、`update.rs::next`、`error.rs::NullConstraintViolation` | `constraint_enforcement_test`（5 场景）+ cli_test 会话/退出码 | None | Covered |
| R2 约束诚实化拒绝 | S1-S5 | D3 | 1.1/1.2 | 000 | `ddl_dml.rs::extract_column_constraints`、`::build_create_table` | `planner_test` 拒绝矩阵 | None | Covered |
| R3 INT 列 UNIQUE 强制 | S1-S8 | D5/D6/D7/D8 | 2.2/2.3/2.4/2.5/2.6/2.7 | 001 | `insert.rs`/`update.rs`/`delete.rs` 唯一维护、`manager.rs::abort_cleanup_versions`、`index_manager.rs` 上下文 | `constraint_enforcement_test`（8 场景） | None | Covered |
| R4 UNIQUE 形态与类型面诚实化 | S1-S5 | D6 | 2.4/2.9 | 001 | `ddl_dml.rs::build_create_table` 策略面、`cli/lifecycle.rs` schema 渲染（映射后） | planner_test 矩阵 + cli_test dump/restore 往返 | None | Covered |
| R5 两态一致 | S1-S5 | D4/D5/D9/D10 | 2.1/2.3/2.8 | 001 | `catalog.rs` 追加演化、`recovery.rs::rebuild_pk_indexes`、`table_manager.rs` drop/open | constraint_enforcement_test 恢复两态 + catalog 单测 | None | Covered |
| R6 既有语义零回归 | S1-S3 | 全部不变量面 | 1.7/2.9 | 000+001 | 全量 | `cargo test` 全绿 | None | Covered |

> 注：R 编号对应 `specs/sql-constraint-enforcement/spec.md` 中 Requirement 出现顺序（NOT NULL 强制 / 诚实化拒绝 / INT 列 UNIQUE 强制 / UNIQUE 形态与类型面 / 两态一致 / 零回归）。

## Iteration Plan

### Iteration 000: 约束诚实化与 NOT NULL 强制

- Tasks: 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 1.7
- Depends on: None
- Stable baseline: 建表声明的 NOT NULL 在 INSERT/UPDATE 被零副作用强制并点名拒绝；CHECK/FK/方言项建表点名拒绝且不留半成品表；TableMeta 携带 not_null 标志且恢复路径读回一致；全量测试零回归
- Verification boundary: planner_test 拒绝矩阵 + constraint_enforcement_test（NOT NULL 面）+ cli_test 错误面/会话用例 + `cargo test` 全绿
- Diagnostic boundary: `src/parser/planner/ddl_dml.rs`、`src/executor/{insert,update}.rs`、`src/storage/{data/table_manager.rs,error.rs}` 与本 Iteration Cycle
- Non-goals: UNIQUE 一切强制与策略面（Iteration 001）；DEFAULT 应用；FOREIGN KEY 强制
- 平衡审计: 五个任务共享「约束解析收紧 + TableMeta 标志 + 写路径校验」单一故障域，合成一个可独立验收的正确性成果（诚实化 + NOT NULL）；单独拆出 1.1/1.2 无法独立构成验收成果，拆分过碎；并入 UNIQUE 则超出单 Iteration 稳定基线承载（索引/catalog/恢复深水面）。规模适中、验证与诊断边界一致，单 Iteration 合理。

### Iteration 001: UNIQUE 强制端到端

- Tasks: 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 2.7, 2.8, 2.9
- Depends on: Iteration 000（TableMeta 标志通道与错误面先例；1.3 的构造点接线模式被 2.3 复用）
- Stable baseline: INT 列 UNIQUE 经专属非 PK 唯一索引在 INSERT/UPDATE/DELETE/回滚全路径强制且 NULL 豁免；干净重开与崩溃恢复两态一致，跨链重复显式报错；catalog 旧格式兼容；dump/restore 往返保持；全量零回归
- Verification boundary: constraint_enforcement_test（UNIQUE 全矩阵 + 恢复两态）+ planner_test DDL 策略矩阵 + catalog 单测 + cli_test 往返 + `cargo test` 全绿
- Diagnostic boundary: `src/storage/{catalog.rs,btree/index_manager.rs,data/table_manager.rs}`、`src/executor/{insert,update,delete}.rs`、`src/transaction/manager.rs`、`src/wal/recovery.rs` 与本 Iteration Cycle
- Non-goals: 用户级 CREATE/DROP INDEX（MS21-T01）；String 等 B-Tree 键编码扩展（I024 域）；FOREIGN KEY 强制；组合 UNIQUE；约束随 ALTER 演进
- 平衡审计: 九个任务构成唯一索引的单一垂直切片（catalog 演化 → 索引基建 → DDL 策略 → 三执行器 + 回滚维护 → 恢复两态 → 收口）；中途切分会留下 enforcement 半开状态（如 INSERT 强制但恢复不重建 = 恢复后静默漏判，属本 change 要消灭的缺陷形态），无法作为稳定基线交付，故不拆分。工作量与 MS07-T01/T02 量级单 Iteration 先例相当，验证边界统一为「UNIQUE 端到端 + 两态一致」，单 Iteration 合理。
