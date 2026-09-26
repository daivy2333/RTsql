# MS24 SQL 写面补全（子集 INSERT / DEFAULT / UPSERT / 写入类型门） — Tasks

> 里程碑：MS24（tasks.md 权威；统一执行序第二位，紧随 MS23）。
> 用户裁定：MS24-T01/T02/T03 单 change 双 Iteration；DO UPDATE SET = 字面量 + `excluded.col` + 旧行裸列引用（算术/函数点名拒绝）；DO UPDATE WHERE v1 点名拒绝；FLOAT 列整数值无损升格；冲突目标 SQLite 对齐（省略=全仲裁、显式单列、组合/ON CONSTRAINT 精确拒绝、REPLACE INTO 实现）。**2026-09-26 追加裁定**：Iteration 001 Plan Review 的两项阻塞发现与既有缺陷「回滚后墓碑行索引条目不还原」并入本 change，按 `replan-required` 修订 Iteration 001 计划（新增 2.7-2.10）并在同目录创建 replan Cycle `001-replan.md`。
> 验证边界：直接目标测试 → 受影响边界（恢复两态 / 回滚两态 / CLI / dump-restore-import）→ 全量 `cargo test`；逐场景最简直接判定，不建身份型证据工程。

## 1. Iteration 000 — 写入类型门与子集 INSERT/DEFAULT（MS24-T03 + T01）

- [x] 1.1 类型门错误面：`StorageError::ColumnTypeMismatch { column, expected, actual }` 新变体（`src/storage/error.rs`，`KeyTypeMismatch` 同型 thiserror 点名模式）。
- [x] 1.2 INSERT 一般类型门：`InsertExecutor`（`src/executor/insert.rs`）在既有 UNIQUE 预检（:169）之后、serialize 之前逐列校验（NULL 豁免、日期族 coerce 后一致、FLOAT 列 Int 值就地升格、其余跨类型拒绝）；既有 PK/唯一/NOT NULL 门触发优先级与文本不变；新 `tests/write_type_conformance_test.rs` 矩阵（非键 INT 收 String/非键 STRING 收 Int/Float 升格/日期族 String/NULL 豁免/键列既有文本优先/零副作用/全通道——dump→restore 与 import 不误报）。
- [x] 1.3 UPDATE 一般类型门：`UpdateExecutor`（`src/executor/update.rs`）UNIQUE 碰撞预检（:190）后、serialize 前对赋值列校验（同规则含升格改写 `new_value`）；write_type_conformance_test UPDATE 臂矩阵。
- [x] 1.4 DEFAULT 持久化通道：`CatalogColumnRow.default_value: Option<Value>` + 列行序列化尾部追加（`u8 has_default | tag | payload`，可选读兼容旧行，`src/storage/catalog.rs`）+ `TableMeta.defaults: Vec<Option<Value>>`（`create_table_with_constraints` 入参 4→5 元组、`create_table` 委托壳、`open_or_init` 读回，`src/storage/data/table_manager.rs`）；catalog 单测（往返 / 旧格式行兼容 / DEFAULT NULL 保真）。
- [x] 1.5 PlanBuilder defaults 通道与子集填充：`table_defaults` + `set_table_defaults`（`src/parser/planner/mod.rs`，注册点 `pipeline.rs::register_table` 接线）；`extract_insert_values` 行值 `InsertValue` 化（DEFAULT 关键字识别）；`map_insert_values` 子集放宽 + 省略位/DEFAULT 位填充——**含无清单分支按表列位对 `DefaultKeyword` 等价填充**（design D2，`INSERT INTO t VALUES (1, DEFAULT)` 形态）；planner_test（子集成功/未知列/重复列/DEFAULT 关键字解析臂——含清单与无清单两形态）。
- [x] 1.6 子集 INSERT 端到端与 dump/schema 渲染：新 `tests/subset_insert_test.rs`（省略列取 DEFAULT/NULL/NOT NULL 拒绝零副作用/全列零回归）+ 重启用例（重开后再子集 INSERT 取 DEFAULT）+ `create_table_sql` DEFAULT 渲染（`src/cli/lifecycle.rs`，dump→restore 往返保真 + `rtsql schema` 含 DEFAULT，cli_test 增量）。
- [x] 1.7 校准与全量回归：`tests/insert_column_list_test.rs::partial_list_rejected_at_plan_time_no_panic` 按子集新语义重写（未知列/重复列/无 panic 防回归意图新用例承接）；`create_table_with_constraints` 5 元组波及的测试构造点机械适配；`cargo test` 全绿，校准逐条记 Act Response。

## 2. Iteration 001 — UPSERT 与 REPLACE INTO（MS24-T02）

- [x] 2.1 计划表示与分派：`PhysicalPlan::Upsert(UpsertNode)` + `ConflictArbiter/ConflictAction/UpsertAssignment/UpsertValueExpr`（`src/executor/plan.rs`，design D4）；`build_plan` Insert 分派显式传 `on`/`replace_into`；`build_insert` 冲突子句解析——目标形态（省略 `All` / 显式单列须解析为 **INT 声明 PK 列**（`primary_key_types` 通道）**或唯一索引列**（新增 `PlanBuilder.table_unique_columns` 通道，`register_table` 自 `TableMeta.unique_indexes` 注册）；其余——组合多列、非唯一列、非 INT 声明 PK——SQLite 语义「does not match」拒绝 / `ON CONSTRAINT` 点名拒绝）、`DO NOTHING`、`DO UPDATE`（assignments 三形态 + `DEFAULT` 字面化、WHERE 点名拒绝）、`DuplicateKeyUpdate` 点名拒绝、`replace_into` → Replace（与 on 并存点名拒绝）；planner_test 拒绝与解析矩阵。
- [x] 2.2 UpsertExecutor 骨架与仲裁搜索：新 `src/executor/upsert.rs` + pipeline 构造臂；逐行 coerce → NOT NULL → PK 键位门 → 仲裁搜索（All：PK 先唯一列按序，F1 守卫；Column：对应索引）；无冲突路径 = 既有 INSERT 序列（InsertExecutor 保持不动，行为等价锚点）；`DoNothing` 跳行不计；受影响计数语义。
- [x] 2.3 DO UPDATE 原位更新：冲突行旧值读取 → 赋值求值（Literal coerce / `Excluded` / `Old`）→ final 行 NOT NULL + 键位门 + rekey 碰撞预检 + 唯一碰撞预检 + 赋值列类型门（含升格）→ 新版本写（`with_next_version`）→ WAL Update → record_version → PK 四分支 + 唯一四分支维护（design D5.5，镜像 update.rs 语义）；`tests/upsert_test.rs`（三形态赋值矩阵 / 碰撞零副作用 / rekey / 计数）。
- [x] 2.4 REPLACE INTO：仲裁全部约束、冲突行去重逐行既有删除语义（墓碑 + WAL Delete + PK/唯一条目清理，design D5.6）→ 插入序列；upsert_test（冲突替换 / 非冲突普通插入 / 多冲突行 / 计数）。
- [x] 2.5 恢复两态与会话面：干净重开与崩溃恢复后 DO UPDATE/REPLACE/DO NOTHING 数据索引一致（upsert_test 恢复两态，复用 constraint_enforcement_test 恢复模式）；显式事务内 upsert 失败回滚与回滚后可重插；cli_test 错误面（exit 3 点名文本）。
- [x] 2.6 收口验证：README（双语）写面语义（子集 INSERT/DEFAULT/UPSERT/REPLACE/类型门）说明；`cargo test` 全绿；校准与偏差逐条记 Act Response。

> 2026-09-26 replan 增补（父 Cycle `000-initial` Plan Review 驱动，承载于 `iterations/001-upsert-replace/001-replan.md`）：2.7-2.10 关闭父 Cycle 的两项阻塞发现并修复其根因缺陷「回滚后墓碑行索引条目不还原」。

- [x] 2.7 `insert_row` PK 重复预检（父 Cycle 阻塞发现 1，契约内补齐）：`src/executor/upsert.rs::insert_row` 在 UNIQUE 预检之前补 PK 重复预检，镜像 `insert.rs:177-187`（`to_key()` 为 `Some` 且 `index_manager.search` 命中即 `DuplicateKey`，serialize 与任何写入之前）；`tests/upsert_test.rs` 补两例（显式唯一列目标 + PK 冲突的 DO NOTHING 臂与 DO UPDATE 臂，含原行点查反向断言），RED 先行。
- [x] 2.8 README 双语写面段复核（父 Cycle 阻塞发现 2）：回滚索引还原（2.9）落地后按实测复核「被拒绝的写入零副作用」表述是否在三个动作上成立；成立则保持原文，不成立则按实测收口措辞（不新增未来承诺），并同步 `README.md` / `README.zh-CN.md`。
- [x] 2.9 回滚后墓碑行索引条目还原（扩围，design D9）：`src/transaction/manager.rs::abort_cleanup_versions` 两趟处理（A 趟既有回退/移除、B 趟墓碑还原）+ 版本链回溯（跳过本 tx 创建的版本）+ 复用 `wal/recovery.rs::extract_index_keys`（提 `pub(crate)`）取存活版本的 PK/唯一键并 `insert` 还原；`tests/explicit_tx_test.rs` 增量 6 场景（DELETE 回滚点查可达 / 唯一值仍占用 / REPLACE 回滚原行完整复现 / 同事务 update→delete 回滚 / 失败 REPLACE 语句回滚无残留 / 回滚后干净重开两态），RED 先行。
- [x] 2.10 replan 收口验证：`cargo test --test upsert_test --test explicit_tx_test --test constraint_enforcement_test --test mvcc_tombstone_visibility_test` + `cargo clippy --all-targets` + `cargo fmt --check`（改动面）+ 全量 `cargo test`；校准与偏差逐条记 Act Response；change 结构自检（tasks 状态、specs/design 与实现一致、Iteration/Cycle 齐全、`Review Result` 与流程状态一致）。

## Requirements Traceability Matrix

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R1 子集 INSERT 与 DEFAULT | S1-S8 | D1/D2/D7 | 1.4/1.5/1.6/1.7 | 000 | `catalog.rs` 列行序列化、`table_manager.rs::TableMeta.defaults`、`planner/mod.rs::table_defaults`、`ddl_dml.rs::map_insert_values`、`lifecycle.rs::create_table_sql` | `subset_insert_test`（8 场景）+ catalog 单测 + cli_test 往返 | None | Covered |
| R2 写入值类型一致门 | S1-S6 | D3 | 1.1/1.2/1.3/1.7 | 000 | `error.rs::ColumnTypeMismatch`、`insert.rs::next`、`update.rs::next` | `write_type_conformance_test`（6 场景） | None | Covered |
| R3 UPSERT 语义 | S1-S6 | D4/D5/D6 | 2.1/2.2/2.3/2.4/2.5/2.6 + **2.7** | 001 | `plan.rs::UpsertNode`、`upsert.rs::UpsertExecutor`、`planner/mod.rs` Insert 分派、`ddl_dml.rs::build_insert` | `upsert_test`（6 场景 + replan 补 2 例：显式目标与仲裁外 PK 约束） | None | Covered |
| R4 冲突目标与拒绝面 | S1-S4 | D4/D5 | 2.1/2.2 | 001 | `ddl_dml.rs::build_insert` 目标解析、`upsert.rs` 仲裁 | `planner_test` 拒绝矩阵 + `upsert_test` | None | Covered |
| R5 既有写面语义零回归 | S1-S2 | D2/D3/D5 不变量面 | 1.2/1.3/1.7/2.2/2.6 + 2.8/2.10 | 000+001 | 全量（InsertExecutor 不动锚点 / 既有错误面优先级 / 文档与实现一致） | `cargo test` 全绿 + 既有矩阵重复执行 + README 写面段复核 | None | Covered |
| R6 回滚后墓碑行索引条目还原（`mvcc-tombstone-visibility` ADDED） | S1-S7 | D9 | 2.9/2.10 | 001 replan | `transaction/manager.rs::abort_cleanup_versions`、`wal/recovery.rs::extract_index_keys`（可见性） | `explicit_tx_test`（6 场景）+ `upsert_test` REPLACE 回滚用例更新 + `constraint_enforcement_test` / `mvcc_tombstone_visibility_test` 零回归 | None | Covered |
| R3' `sql-constraint-enforcement` R3 回滚子句去歧义（MODIFIED） | 9 场景（既有 8 零校准 + 新增 DELETE/REPLACE 回滚占用场景） | D9 | 2.9/2.10 | 001 replan | 同 R6 | 同 R6 | None | Covered |

> 注：R 编号对应各 capability delta 中 Requirement 出现顺序——R1-R5 对应 `specs/sql-write-surface/spec.md`（子集 INSERT 与 DEFAULT / 类型一致门 / UPSERT 语义 / 冲突目标与拒绝面 / 零回归），R6 对应 `specs/mvcc-tombstone-visibility/spec.md` 的 ADDED Requirement，R3' 对应 `specs/sql-constraint-enforcement/spec.md` 的 MODIFIED R3。

## Iteration Plan

### Iteration 000: 写入类型门与子集 INSERT/DEFAULT

- Tasks: 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 1.7
- Depends on: None
- Stable baseline: 写入值与列声明类型一致门在 INSERT/UPDATE 全写面生效（含升格与既有错误面优先级不变），dump/restore/import 零误报；子集列清单 INSERT 端到端可用且省略列 DEFAULT/NULL 语义正确；DEFAULT 经 catalog 持久化跨重启生效且 dump/schema 渲染保真；全量测试零回归（校准逐条记录）
- Verification boundary: write_type_conformance_test 矩阵 + subset_insert_test + planner_test 子集/解析矩阵 + catalog 单测 + cli_test 往返与渲染 + `cargo test` 全绿
- Diagnostic boundary: `src/executor/{insert,update}.rs`、`src/storage/{error.rs,catalog.rs,data/table_manager.rs}`、`src/parser/planner/{mod,ddl_dml}.rs`、`src/cli/lifecycle.rs` 与本 Iteration Cycle
- Non-goals: UPSERT/REPLACE 一切语义（Iteration 001）；多列 UPDATE SET；表达式赋值
- 平衡审计: 七个任务共享「写入前置校验 + 列元数据通道（TableMeta/PlanBuilder/catalog）」单一故障域——类型门与子集填充消费同一批新通道（`TableMeta.defaults`、`table_defaults`），拆开则类型门先落地、子集填充需二次触碰同批构造点与测试夹具（返工面大于聚合面）；合并后构成一个可独立验收成果「写面基础收口」，中途状态（门生效但子集不可用）亦为合法稳定基线。规模中-大与 MS23 Iteration 000 量级相当，单 Iteration 合理。

### Iteration 001: UPSERT 与 REPLACE INTO（2026-09-26 replan 修订）

- Tasks: 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 2.7, 2.8, 2.9, 2.10
- Depends on: Iteration 000（accepted 2026-09-25——写入类型门 D3 两趟结构、DEFAULT 持久化与填充通道、`PlanBuilder.table_defaults`、既有门优先级与文本锚点全绿）
- Stable baseline: ON CONFLICT DO NOTHING / DO UPDATE（三形态赋值）与 REPLACE INTO 端到端可用，冲突仲裁确定、碰撞预检零副作用、受影响计数符合语义；显式冲突目标只仲裁该约束、违反仲裁外约束（PK 或唯一）以既有 DuplicateKey 拒绝；干净重开与崩溃恢复两态一致；删除者事务回滚后被删行的 PK 与唯一索引条目完整还原（索引状态与「删除未发生」一致，含同事务 update→delete、REPLACE 与失败语句三形态）；既有 INSERT 路径逐字节不变；全量零回归
- Verification boundary: upsert_test 全矩阵（仲裁/动作/恢复两态/回滚两态/计数/拒绝面）+ planner_test 解析拒绝矩阵 + cli_test 错误面 + explicit_tx_test 回滚还原矩阵 + constraint_enforcement_test / mvcc_tombstone_visibility_test 零回归 + `cargo test` 全绿
- Diagnostic boundary: `src/executor/{plan.rs,upsert.rs}`、`src/parser/planner/{mod,ddl_dml}.rs`、`src/pipeline.rs` 构造臂、`src/transaction/manager.rs::abort_cleanup_versions`（含 `wal/recovery.rs` 取键 helper 的可见性）与本 Iteration Cycle
- Non-goals: DO UPDATE WHERE（显式缺口，v1 点名拒绝）；`INSERT OR ...` 方言迁移（解析层拒绝维持）；算术/函数赋值表达式；组合唯一约束（MS21 域）；多列 UPDATE SET 语句；提交路径与恢复重放通道的索引策略变更（仅复用既有取键 helper）；删除时延迟移除索引条目等运行期索引语义改写
- 平衡审计（2026-09-26 修订）: 原六任务为 UPSERT 单一垂直切片（计划表示 → 执行器骨架/仲裁 → 两个动作 → 两态与收口），量级与 MS23 Iteration 001 相当。replan 增补的四任务中，2.7 属父 Cycle 阻塞发现的契约内补齐（同一 `insert_row` 方法、与 2.2 同一故障域，不构成独立成果）；2.9 是唯一新增的独立成果（既有缺陷修复，跨 `transaction` 层与 `sql-write-surface` / `sql-constraint-enforcement` / `mvcc-tombstone-visibility` 三个 capability，与 UPSERT 切片不同故障域、不同诊断边界），2.8 与 2.10 为其收口与复核。因缺陷根因与 UPSERT 无耦合且修复面独立，**拆分候选成立**；用户 2026-09-26 明确裁定并入当前 change（选项二），故保留在同一 Iteration 并以 replan Cycle 承接——若实施中 2.9 出现实质设计变更（超出 D9 的两趟 + 回溯 + 取键还原），按 Gate 6 返回 Plan 重新评估拆分为独立 change。
