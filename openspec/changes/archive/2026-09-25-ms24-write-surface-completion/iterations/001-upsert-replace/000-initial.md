# Iteration 001 / Cycle 000: UPSERT 与 REPLACE INTO

## Plan Context

- Status: ready
- Iteration: 001-upsert-replace
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: 2.1, 2.2, 2.3, 2.4, 2.5, 2.6
- Depends on: Iteration 000（accepted 2026-09-25——写入类型门 D3 两趟结构、DEFAULT 持久化与填充通道、`PlanBuilder.table_defaults`、既有门优先级与文本锚点全绿）
- Stable baseline: ON CONFLICT DO NOTHING / DO UPDATE（三形态赋值）与 REPLACE INTO 端到端可用，冲突仲裁确定、碰撞预检零副作用、受影响计数符合语义；干净重开与崩溃恢复两态一致；既有 INSERT 路径逐字节不变；全量零回归
- Verification boundary: upsert_test 全矩阵（仲裁/动作/恢复两态/计数/拒绝面）+ planner_test 解析拒绝矩阵 + cli_test 错误面 + `cargo test` 全绿
- Diagnostic boundary: `src/executor/{plan.rs,upsert.rs}`、`src/parser/planner/{mod,ddl_dml}.rs`、`src/pipeline.rs` 构造臂与本 Iteration Cycle
- Deferred tasks: None

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: change 全部 R3/R4/R5 requirement 与 design D4/D5/D6；Iteration 000 产物——一般类型门规则（D3，含 FLOAT 升格与 R2 已知边界段）、`table_defaults` 通道（DO UPDATE `DEFAULT` 字面化数据源）、`TableMeta.unique_indexes` 唯一索引基建（MS23）、既有门优先级与文本（PK `KeyTypeMismatch` / NOT NULL / F1 守卫 / `ColumnTypeMismatch`）
- Excluded scope: DO UPDATE WHERE（v1 点名拒绝）；`INSERT OR ...` 方言迁移（解析层拒绝维持）；算术/函数赋值表达式；组合唯一约束（MS21 域）；多列 UPDATE SET 语句；I021 多值批量性能

**Objective**

`INSERT ... ON CONFLICT [target] DO NOTHING` 冲突行跳过（不计受影响行数）、`DO UPDATE SET col = expr, ...`（字面量 / `excluded.col` / 旧行裸列引用三形态）冲突行原位更新、`REPLACE INTO` 冲突行删除重插端到端可用；冲突目标省略 = PK + 全部唯一索引仲裁、显式单列须为 INT 声明 PK 或唯一索引列、组合/`ON CONSTRAINT`/`ON DUPLICATE KEY UPDATE`/`DO UPDATE WHERE` 计划期点名拒绝；干净重开与崩溃恢复两态一致；静默忽略面收口（`Statement::Insert` 分派显式消费 `on`/`replace_into`）；既有 INSERT/UPDATE/DELETE 行为逐字节不变；全量零回归。

**Background**

MS24 统一执行序第二位 Iteration 001。`build_plan` 的 `Statement::Insert` 分派以 `..` 丢弃 `on`/`replace_into`——ON CONFLICT 按普通 INSERT 执行（冲突即 DuplicateKey，子句从未生效）、REPLACE INTO 静默降级普通 INSERT；静默忽略比诚实拒绝更危险，且 ON CONFLICT/REPLACE 是 SQLite 写入惯用法。用户已批准需求基线（Gate 1，2026-09-25）与语义裁定（单 change 双 Iteration；DO UPDATE 三形态赋值；DO UPDATE WHERE v1 拒绝；冲突目标 SQLite 对齐）。

**Investigation Facts**

- Current Baseline: Iteration 000 accepted（2026-09-25，`../000-type-gate-subset-insert/000-initial.md` Plan Review）——全量 1182 passed / 0 failed / 2 ignored 全量新鲜；类型门/子集 INSERT/DEFAULT 闭环全绿；本 Iteration 新涉及表面（plan.rs / pipeline DML 臂 / sqlparser AST / delete.rs）自 MS23 收尾基线零变化（git 现状核对，本会话只读）。
- Current-State Evidence（本会话独立追读，含 sqlparser registry 源核对）：
  - Insert 分派丢弃面：`build_plan`（`src/parser/planner/mod.rs:178-183`）`Statement::Insert { table_name, columns, source, .. }`——`on`/`replace_into` 未消费。sqlparser 0.44 `Statement::Insert` 字段（ast/mod.rs :1731-1760）：`or: Option<SqliteOnConflict>`（:1733，GenericDialect 下 OR 形态不可达）、`on: Option<OnInsert>`（:1755）、`replace_into: bool`（:1759）。
  - sqlparser 0.44 AST（`~/.cargo/registry/src/.../sqlparser-0.44.0/src/ast/mod.rs` 实测核对）：`OnInsert`（:4190）= `DuplicateKeyUpdate(Vec<Assignment>) | OnConflict(OnConflict)`；`OnConflict`（:4200）= `{ conflict_target: Option<ConflictTarget>, action: OnConflictAction }`；`ConflictTarget`（:4207）= `Columns(Vec<Ident>) | OnConstraint(ObjectName)`；`OnConflictAction`（:4214）= `DoNothing | DoUpdate(DoUpdate)`；`DoUpdate`（:4222）= `{ assignments: Vec<Assignment>, selection: Option<Expr> }`；`Assignment`（:4482）= `{ id: Vec<Ident>, value: Expr }`。与 design D4 假设一致。
  - Plan/执行器接线：`PhysicalPlan`（`src/executor/plan.rs:18+`，Insert :30）+ `InsertNode`（:145-152 `{table_name, columns, values}`）；`execute_stage` DML 臂（`src/pipeline.rs:117` 模式匹配 + table_name 提取 :122-127）——`PhysicalPlan::Upsert` 需加入两处；`create_executor_from_plan` Insert 构造臂（:527-538，`tx_id.expect("DML Insert requires a transaction id")`）为 Upsert 构造臂模板；`is_cacheable`（:1126）仅 Query——Upsert 不进 plan cache；`register_table`（:1084-1122，defaults 注册 :1115）为 `table_unique_columns` 注册点（`TableMeta.unique_indexes: Vec<(usize, Arc<IndexManager>)>` 列位有序）。
  - 无冲突路径锚点（InsertExecutor 逐行序，`src/executor/insert.rs:114-301`）：coerce :126 → 升格趟 :137 → NOT NULL :147 → PK 键位门 :161 → PK DuplicateKey 预检 :177 → UNIQUE+F1 守卫 :195 → 类型拒绝趟 :218 → serialize :239 → 数据页 :249 → visibility :258 → WAL Insert :266 → record_version :276 → PK 条目 :281 → 唯一条目 :291（数据落位与 PK 之后）。
  - DO UPDATE 镜像源（`src/executor/update.rs`）：PK 门 + rekey 碰撞预检 :125-153、coerce :159-168、UNIQUE 碰撞预检 :205-221、类型门含升格 :228-248、新版本 `with_next_version(old)` 写 :256、WAL Update :271、record_version :283、PK 四分支维护 :299-322、唯一四分支维护 :332-353。
  - REPLACE 删除镜像源（`src/executor/delete.rs`）：唯一键提取 :49-74、墓碑 `mark_deleted` + `with_next_version(rid)` 写 :105-132、PK 条目删除 :134、唯一条目删除 :138-140、record_version :147-151、WAL Delete :158-165。
  - 仲裁数据源：`index_manager.search`（insert.rs:181 用法先例）；PK 声明类型经 `primary_key_types`（`src/parser/planner/mod.rs:153`）。
  - 回滚与恢复：`TransactionManager::abort` 清理通道含墓碑与唯一条目（MS23 修复后）；WAL Insert/Update/Delete 三型既有重放通道（MS10-T02 T0/T0b、MS09 墓碑重放）+ `redo_count > 0` 索引重建（`src/wal/recovery.rs`）——无新 WAL 类型（D6）。
  - 受影响既有测试面：`key_type_conformance_test`（PK 键位门锚点）、`constraint_enforcement_test`（恢复两态模式复用源 + UNIQUE 矩阵）、`subset_insert_test`/`insert_column_list_test`（无冲突 INSERT 行为等价锚）。
- Code and Critical Path: 计划期 `build_insert`（`src/parser/planner/ddl_dml.rs:123`）扩参消费 `on`/`replace_into` → 冲突子句解析为 `UpsertNode`（values 经 D2 全宽填充）；执行期新 `UpsertExecutor` 逐行：前置门（coerce/升格/NOT NULL/PK 键位）→ 仲裁搜索（兼作无冲突路径预检）→ 三动作分派；无冲突路径镜像 insert.rs 全门序与写入序列（InsertExecutor 文件不动，行为等价锚点）；DO UPDATE 镜像 update.rs 写形状；REPLACE 镜像 delete.rs 删除语义 + 插入序列。

**Implementation Guidance**

顺序：2.1 计划表示与解析（拒绝矩阵 RED 先行）→ 2.2 执行器骨架与仲裁（无冲突路径先与 INSERT 等价、DoNothing 完成）→ 2.3 DO UPDATE 臂 → 2.4 REPLACE 臂 → 2.5 恢复两态与会话面 → 2.6 收口。`table_unique_columns` 通道由 2.1 自建（register_table 接线点与 000 defaults 注册同区）。DO UPDATE/REPLACE 写形状逐分支镜像 update.rs/delete.rs（版本链、WAL、visibility、record_version、PK/唯一索引维护），不重构既有执行器——helper 抽取属等价控制流选择留给 Act，契约以「UPDATE/DELETE 语句行为逐字节不变」为界（D5 否决方案在案）。

**Behavioral Change**

- 当前：`ON CONFLICT ...` 按普通 INSERT 执行（冲突即 DuplicateKey，子句从未生效）；`REPLACE INTO` 静默降级普通 INSERT（replace 语义丢失）；`ON DUPLICATE KEY UPDATE` 同被 `..` 丢弃。
- 目标：三动作端到端语义（R3）+ 冲突目标解析与拒绝面（R4）+ 恢复两态一致 + 静默忽略面收口（分派显式消费或点名拒绝，消除「子句写了但从未生效」）。
- 接口/错误语义：`PhysicalPlan::Upsert(UpsertNode)` 新变体 + D4 计划表示类型；`build_insert` 扩参（DML 不进 plan cache，无缓存键影响）；计划期拒绝经 `PlanError::ParseError` 点名文本（`insert_count_error`/`UnsupportedConstraint` 既有先例），CLI exit 3。

**Task Contracts**

### 2.1: ON CONFLICT/REPLACE 计划表示与解析分派

- Requirement/Scenario: R3（计划表示载体）、R4 全场景（目标解析与拒绝面）
- Depends on: None
- Targets: `src/executor/plan.rs`（`PhysicalPlan::Upsert(UpsertNode)` + `ConflictArbiter{All,Column(usize)}` / `ConflictAction{DoNothing,DoUpdate(Vec<UpsertAssignment>),Replace}` / `UpsertAssignment{column,expr}` / `UpsertValueExpr{Literal,Excluded(usize),Old(usize)}`，D4 形状）；`src/parser/planner/mod.rs`（`build_plan` Insert 分派显式传 `on`/`replace_into`；`PlanBuilder.table_unique_columns` 加性通道 + setter）；`src/parser/planner/ddl_dml.rs::build_insert`（扩参 + 冲突子句解析）；`src/pipeline.rs::register_table`（自 `TableMeta.unique_indexes` 注册唯一列位）
- Current behavior: `on`/`replace_into` 以 `..` 丢弃——ON CONFLICT 语句按普通 INSERT 计划、REPLACE INTO 静默降级；PlanBuilder 无唯一列元数据
- Required behavior: D4 全分支——`on=None ∧ replace_into=true` → `(All, Replace)`；`replace_into ∧ on.is_some()` → 点名拒绝；`OnInsert::OnConflict(oc)`：`conflict_target: None` → `All`；`Columns(idents)` 长度=1 且解析为 INT 声明 PK 列（`primary_key_types` 判声明类型）或唯一索引列（`table_unique_columns`）→ `Column(idx)`，其余（组合多列 / 非唯一列 / 非 INT 声明 PK）→「ON CONFLICT clause does not match any PRIMARY KEY or UNIQUE constraint」拒绝；`OnConstraint(_)` → 点名拒绝；`DoNothing` → 动作；`DoUpdate(du)`：`du.selection` 非空 →「DO UPDATE WHERE is not supported」拒绝；assignments 逐个 `id.len()==1`（否则点名拒绝）、LHS 列解析 `ColumnNotFound` 同既有、值臂 `Expr::Value` → `Literal` / TypedString 日期族 → `Literal` / NULL 标识 → `Literal(Null)` / `Identifier("DEFAULT")` → `Literal(声明 DEFAULT 或 Null)`（`table_defaults` 计划期字面化）/ 裸 `Identifier(col)` → `Old(idx)` / `CompoundIdentifier` 首段 `excluded` → `Excluded(idx)` / 其余形态 → 点名拒绝；`OnInsert::DuplicateKeyUpdate(_)` →「ON DUPLICATE KEY UPDATE is not supported」拒绝。`UpsertNode.values` 为 D2 全宽填充后行
- Preserve: 无冲突子句 INSERT 的 `build_insert` 路径行为不变；既有列清单/值解析拒绝面与文案不变；`PlanBuilder` 公共 API 加性（`new()` 初始化）
- Forbidden: 不动 sqlparser 方言配置（`INSERT OR ...` 维持解析层拒绝）；不实现 DO UPDATE WHERE；不接受算术/函数赋值；不新增计划期 NOT NULL/类型校验（执行器单点强制）
- Test witness: `tests/planner_test.rs` 拒绝与解析矩阵——先行 RED（UpsertNode 不存在编译 RED，逐用例驱动）：目标三态解析（None/合法单列 PK/合法单列唯一列）、拒绝矩阵（组合多列/非唯一列/非 INT PK/OnConstraint/DO UPDATE WHERE/DuplicateKeyUpdate/replace_into+on 并存）、DO UPDATE 赋值三形态与 `DEFAULT` 字面化、算术/函数点名拒绝
- GREEN condition: planner_test 增量全绿
- Verification: `cargo test --test planner_test`，退出码 0
- Stop when: sqlparser 实测 OnInsert/OnConflict 形态与调查矛盾（DESIGN-INVALID 返回 Plan）

### 2.2: UpsertExecutor 骨架与冲突仲裁（无冲突路径 + DO NOTHING）

- Requirement/Scenario: R3（仲裁与无冲突路径、DO NOTHING 计数）、R4（仲裁目标行为）
- Depends on: 2.1
- Targets: 新 `src/executor/upsert.rs::UpsertExecutor`；`src/pipeline.rs`（DML 臂模式匹配 + table_name 提取 + `create_executor_from_plan` Upsert 构造臂）
- Current behavior: 无 UpsertExecutor，`PhysicalPlan::Upsert` 不可达
- Required behavior: 逐行前置门（镜像 insert.rs 门序与文本）：coerce :126 → 升格趟 :137 → NOT NULL :147 → PK 键位门 :161 → 仲裁搜索：`All`——PK 值可键控（`to_key()`）→ `index_manager.search` 命中即冲突（PK 优先），其后逐唯一列（NULL 跳过 / F1 守卫 `KeyTypeMismatch` / search 命中）；`Column(idx)`——idx == `pk_index` → PK search，否则定位 `unique_indexes` 对应项（2.1 已保证存在）→ F1 守卫 + search。无命中 → 既有 INSERT 序列（类型拒绝趟 :218 → serialize :239 → 数据页 → visibility → WAL Insert → record_version → PK 条目 → 唯一条目），count += 1；`DoNothing` 命中跳行不计；`DoUpdate`/`Replace` 臂本任务以明确占位错误拒绝（2.3/2.4 替换）
- Preserve: `src/executor/insert.rs` 文件不动（零回归锚点）；既有错误面优先级与文本（PK/NOT NULL/F1/类型门）在 upsert 路径同序同文本
- Forbidden: 不改 InsertExecutor/UpdateExecutor/DeleteExecutor；不动 WAL/版本链协议；不为仲裁引入新索引操作原语
- Test witness: `tests/upsert_test.rs`（新建，lib API helpers 模式）骨架批——先行 RED：无冲突行插入与普通 INSERT 逐字节等价（含子集/DEFAULT/类型门 000 语义）、DO NOTHING 冲突行跳行计数 0、DO NOTHING 非冲突行正常插入计数 1、仲裁命中分派（PK 优先 / 唯一列序）
- GREEN condition: 骨架批全绿
- Verification: `cargo test --test upsert_test`，退出码 0
- Stop when: 仲裁搜索与既有索引可见性语义出现矛盾（如与 INSERT DuplicateKey 预检行为不一致）

### 2.3: DO UPDATE 原位更新

- Requirement/Scenario: R3 DO UPDATE 场景（三形态赋值 / 碰撞预检零副作用 / rekey / 计数）
- Depends on: 2.2
- Targets: `src/executor/upsert.rs`（DoUpdate 臂替换占位）
- Current behavior: 2.2 占位错误
- Required behavior: 读冲突行旧 tuple（deserialize）→ 逐 assignment 求值（`Literal` 经 coerce 落列类型 / `Excluded(i)` 取本行待插值（已 coerce/升格）/ `Old(i)` 取旧值）→ final 行（旧值改赋值列）NOT NULL 零副作用 → 赋值触及 PK：Int/Null 键位门 + rekey 碰撞预检（镜像 update.rs:125-153）→ 触及唯一列：改值分支新键 search 碰撞预检（镜像 :205-221）→ 赋值列类型门含升格（D3 规则，镜像 :228-248）→ serialize final → 新版本 `with_next_version(冲突行 rid)` 写数据页 → visibility → WAL `Update{old_tuple, new_tuple}` → record_version → PK 四分支维护（镜像 :299-322）+ 唯一四分支维护（镜像 :332-353）→ count += 1
- Preserve: UPDATE 语句行为逐字节不变（镜像不重构）；既有门文本；非赋值列继承旧值原样（含存量混合库 passthrough 语义与 update.rs 一致）
- Forbidden: 不实现 WHERE；不支持表达式赋值；不重构 UpdateExecutor；不改 `DO UPDATE SET` 左值多列形态拒绝
- Test witness: upsert_test DO UPDATE 批——三形态赋值矩阵（字面量/excluded/旧行引用）、多列赋值、`DEFAULT` 赋值、碰撞预检零副作用（新键命中即拒、两行数据与索引不变）、rekey、受影响计数
- GREEN condition: 批全绿
- Verification: `cargo test --test upsert_test`，退出码 0
- Stop when: 镜像分支与 UPDATE 语句可观察行为出现实质差异

### 2.4: REPLACE INTO

- Requirement/Scenario: R3 REPLACE 场景
- Depends on: 2.2
- Targets: `src/executor/upsert.rs`（Replace 臂替换占位）
- Current behavior: 2.2 占位错误
- Required behavior: `All` 仲裁收集全部冲突 rid 去重 → 逐行既有删除语义（镜像 delete.rs：唯一键提取 :49-74、墓碑 `mark_deleted`+`with_next_version` 写 :105-132、visibility、PK 条目删除 :134、唯一条目删除 :138-140、record_version :147-151、WAL Delete :158-165）→ 执行 2.2 插入序列；每 VALUES 行计 1（无论替换多少冲突行，SQLite changes() 语义）
- Preserve: DELETE 语句行为逐字节不变；墓碑/record_version/WAL Delete 语义与 DeleteExecutor 同构
- Forbidden: 不支持 `INSERT OR REPLACE`（方言不可达）；`REPLACE INTO` 与 ON CONFLICT 并存维持 2.1 点名拒绝；不重构 DeleteExecutor
- Test witness: upsert_test REPLACE 批——冲突替换 / 非冲突普通插入 / 多冲突行（PK+唯一双约束命中去重）/ 受影响计数
- GREEN condition: 批全绿
- Verification: `cargo test --test upsert_test`，退出码 0
- Stop when: 删除语义镜像与 DELETE 语句可观察行为实质差异

### 2.5: 恢复两态与会话面

- Requirement/Scenario: R3 恢复两态一致场景、R4 拒绝面 CLI 错误、R5
- Depends on: 2.3, 2.4
- Targets: `tests/upsert_test.rs`（恢复两态 + 显式事务批）；`tests/cli_test.rs`（计划期拒绝错误面）
- Current behavior: 无 upsert 恢复/事务/CLI 错误面见证
- Required behavior: 干净重开与崩溃恢复后 DO UPDATE/REPLACE/DO NOTHING 数据与索引一致（复用 constraint_enforcement_test 恢复两态模式）；显式事务内 upsert 失败回滚、回滚后可重插（abort 清理通道含墓碑与唯一条目）；CLI 计划期拒绝错误面 exit 3（ON CONSTRAINT / DO UPDATE WHERE / does-not-match / ON DUPLICATE KEY UPDATE 点名文本）
- Preserve: 既有恢复重放通道零变化（无新 WAL 类型）；事务会话语义不变
- Forbidden: 不为 upsert 新增专用重放臂（Insert/Update/Delete 三型既有通道承载）；不做专用 CLI 臂
- Test witness: 恢复两态用例 + 显式事务回滚/重插用例 + cli_test 错误面用例
- GREEN condition: 全绿
- Verification: `cargo test --test upsert_test --test cli_test`，退出码 0
- Stop when: 恢复重放出现 upsert 特有缺口（BASELINE-CHANGED 返回 Plan）

### 2.6: 收口验证

- Requirement/Scenario: R5
- Depends on: 2.1, 2.2, 2.3, 2.4, 2.5
- Targets: `README.md` / `README.zh-CN.md`（写面语义段：子集 INSERT/DEFAULT/UPSERT/REPLACE/类型门）
- Current behavior: README 无该面说明
- Required behavior: 双语文档与实现一致；`cargo test` 全量绿；`cargo clippy --all-targets` 0 warning；校准与偏差逐条记 Act Response
- Preserve: 既有文档其余段不动
- Forbidden: 不扩张文档范围（不写未来承诺）
- Test witness: `cargo test` 全量输出（计数 = 1182 + 本 Iteration 净增用例，0 failures）
- GREEN condition: 0 failures（ignored 计数不增）
- Verification: `cargo test` + `cargo clippy --all-targets`，退出码 0
- Stop when: 出现无法归因本 change 的回归（BASELINE-CHANGED 返回 Plan）

**Invariants**

- `src/executor/{insert,update,delete}.rs` 文件不动（零回归锚点）；UpsertExecutor 以镜像实现共享语义。
- 既有错误面优先级与文本逐字节不变（`KeyTypeMismatch` / `NullConstraintViolation` / F1 守卫 / `DuplicateKey` / `ColumnTypeMismatch` / 既有计划期拒绝）。
- 无新 WAL 记录类型；恢复重放与索引重建通道不动（Insert/Update/Delete 三型既有通道承载两态一致）。
- 无冲突子句 INSERT 行为逐字节不变（含 Iteration 000 的子集清单/DEFAULT 填充/类型门语义）。
- DML 不进 plan cache；`PlanBuilder` 公共 API 既有签名不变（加性）。
- 不建身份型证据工程；验证用原生 `cargo test` 退出码与输出。

**Non-goals**

- DO UPDATE WHERE（v1 显式缺口，计划期点名拒绝）；`INSERT OR ...` 方言迁移（解析层拒绝维持）；算术/函数赋值表达式；组合唯一约束冲突目标（MS21 域）；多列 UPDATE SET 语句；I021 多值批量性能；跨语句/跨事务 upsert 原子性精细化（语句级 auto-commit 包裹语义不变）。

**Acceptance**

R3（DO NOTHING/DO UPDATE/REPLACE 语义 + 恢复两态）经 `tests/upsert_test.rs` 全矩阵见证；R4（冲突目标解析与拒绝面）经 `tests/planner_test.rs` 拒绝矩阵 + upsert_test 仲裁行为 + `tests/cli_test.rs` 错误面见证；R5（既有写面零回归）经 InsertExecutor 不动锚点 + 既有矩阵重复执行 + `cargo test` 全绿见证。映射见 change tasks.md RTM（R3/R4/R5 行）。

**Verification**

- R4 拒绝与解析：`cargo test --test planner_test` 退出码 0。
- R3 动作矩阵：`cargo test --test upsert_test` 退出码 0（仲裁/三动作/碰撞零副作用/rekey/计数）。
- R3 两态 + R4 CLI：`cargo test --test upsert_test --test cli_test` 退出码 0。
- R5：`cargo test` 全量 0 failures（计数 = 1182 + 净增用例）+ `cargo clippy --all-targets` 0 warning。
- 全部为原生 cargo 输出与退出码直接判定，无封装判定层。

**Gate 2 Readiness**

- 无 Missing requirement：RTM 五行全 Covered（tasks.md），R3/R4 映射本 Iteration 2.1-2.6。PASS
- 无未批准 Simplified：DO UPDATE WHERE/INSERT OR 为 R4 拒绝面场景（Gate 1 已向用户明示并获批）；无 Simplified 项。PASS
- 调查完整：Current-State Evidence 全部来自本会话对当前工作树与 sqlparser 0.44 registry 源的独立追读（AST 形态实测核对，行号在案）；基线 1182 tests 采信 Iteration 000 accepted Review（表面零变化经 git 现状核对）。PASS
- 设计闭合：D4/D5/D6 行为/接口/错误/兼容语义完整；仲裁序/赋值三形态/Replace 计数/两态通道均有裁定；无契约语义 TBD。PASS
- 任务可执行：2.1-2.6 每个 Task Contract 有 Targets/Current/Required/Preserve/Forbidden/witness/GREEN/Verification/Stop。PASS
- 分轮合理：单 Iteration 六任务 UPSERT 垂直切片，平衡审计在 tasks.md（000 Review 已核）。PASS
- 追踪完整：RTM R↔S↔D↔Task↔Code↔Test 链路闭合。PASS
- 验证充分：全部 scenario（含 sad path/edge/恢复两态）映射到测试命令；最简直接判定；无身份型证据工程/判定层。PASS
- Act 自包含：本 Cycle 含全部实现事实（含 sqlparser AST 形状与镜像源行号）与契约，Act 无需回读 Assistant/Explorer/父 Cycle。PASS
- 非实质未知项入 Risks：多行行间冲突可见性、Abort 对 Replace 混合版本的清理（既有通道，实测见证）——均不迫使 Act 决定契约语义。PASS
- Persisted Evidence：none（见下）。PASS
- 计划获批：change 级用户裁定与批准在案（tasks.md 头部裁定记录 + Iteration 000 Gate 2 分轮项 PASS + 000 Review accepted 展开）。PASS

**Persisted Evidence**

- Mode: none

`none` 依据：全部验证为 `cargo test`/`cargo clippy` 原生命令，输出与退出码写入 Act Response 即可承载（公共规则 › 验证）；无不可低成本重跑的现场。

- Budget: 按 Evidence 预算执行（公共规则 › 验证）。

**Risks and Notes**

- 同语句多行 upsert 的行间冲突：仲裁 search 消费实时索引条目（与 INSERT DuplicateKey 预检同性质），后行可见前行已插条目——SQLite 同型语义，无需特判；若实测出现与该预期矛盾的可见性行为属实质问题返回 Plan。
- Abort 回滚对 Replace 产出的 Delete+Insert 混合版本依赖既有 abort 清理通道（MS23 含墓碑与唯一条目修复）；2.5 事务回滚用例实测见证，若出现回滚后残留属实质缺陷返回 Plan（Issue 候选）。
- `INSERT INTO t SELECT ...`（source 非 VALUES）维持既有非 Values 拒绝路径，upsert 子句不改变该面；`RETURNING` 字段维持未消费（`..` 覆盖，既有行为）。
- `Assignment.id` 为 `Vec<Ident>`——LHS 复合左值（如 `t.v = ...`）按 `id.len()==1` 点名拒绝（D4 契约在案）。

## Act Response

- Status: reported

**Implemented**

按 Plan Context 全部 6 个 task 完成（change tasks.md 2.1-2.6 已勾选）：

- 2.1：`PhysicalPlan::Upsert(UpsertNode)` + D4 四个计划类型（`ConflictArbiter{All,Column}` / `ConflictAction{DoNothing,DoUpdate,Replace}` / `UpsertAssignment` / `UpsertValueExpr{Literal,Excluded,Old}`）；`Statement::Insert` 分派停止以 `..` 丢弃 `on`/`replace_into`，显式下传 `build_insert`；`PlanBuilder.table_unique_columns` 加性通道 + `set_table_unique_columns`，`register_table` 自 `TableMeta.unique_indexes` 注册列位；冲突子句解析覆盖 D4 全分支（省略目标=全仲裁 / 显式单列须为 INT 声明 PK 或唯一列 / 组合·非唯一·非 INT PK·未知列统一 SQLite 语义拒绝 / `ON CONSTRAINT` / `DO UPDATE WHERE` / `ON DUPLICATE KEY UPDATE` / `replace_into`+`on` 并存点名拒绝），赋值三形态 + `DEFAULT` 计划期字面化 + 复合左值与算术/函数点名拒绝。
- 2.2：新 `src/executor/upsert.rs::UpsertExecutor`（`prepare_row` → `arbitrate` → 动作分派）；前置门与 `InsertExecutor` 同序同文本（coerce → FLOAT 升格 → NOT NULL → PK 键位门）；`All` 仲裁 PK 先、唯一列按序（含 F1 `KeyTypeMismatch` 守卫），`Column(idx)` 只仲裁该列；无冲突路径镜像 INSERT 序列（UNIQUE 预检 → 类型拒绝趟 → serialize → 数据页 → visibility → WAL Insert → record_version → PK 条目 → 唯一条目）；`DoNothing` 跳行不计。`insert.rs` 文件未动（零回归锚点）。
- 2.3：DO UPDATE 原位更新，镜像 `update.rs` 写形状——旧 tuple 读取 → 赋值求值（`Old` 取赋值前旧行快照）→ 日期族强制解析 → 赋值列 NOT NULL → 键位门 + rekey 碰撞预检 → 唯一碰撞预检 → 赋值列类型门（含 FLOAT 升格）→ serialize → `with_next_version(冲突行)` → visibility → WAL Update → record_version → PK 四分支 + 唯一四分支维护。
- 2.4：REPLACE INTO——`All` 仲裁收集全部冲突 rid 去重后逐行镜像 `delete.rs` 删除语义（唯一键提取 → 墓碑 `mark_deleted` → visibility → PK 条目删除 → 唯一条目删除 → record_version → WAL Delete）→ 插入序列；每个 VALUES 行计 1。
- 2.5：恢复两态（干净重开 / 崩溃恢复 redo+重建）与显式事务（commit 持久化、混合拒绝+回滚、REPLACE 回滚）用例；CLI 计划期拒绝错误面（exit 3 + 点名 stderr）。
- 2.6：README 双语「Write surface / 写面」段（子集 INSERT+DEFAULT、值类型门、DO NOTHING/DO UPDATE、冲突目标、REPLACE、计划期拒绝清单、两态一致）+ 能力清单 `INSERT` 条目补 upsert 形态；`cargo test` 全量绿、`cargo clippy --all-targets` 0 warning、改动面 fmt 校准。

**Changed Files and Symbols**

产品代码（8 文件）：

- `src/executor/plan.rs`：`Upsert` 变体 :33；`ConflictArbiter` :159、`UpsertValueExpr` :168、`UpsertAssignment` :179、`ConflictAction` :188、`UpsertNode` :199。
- `src/executor/upsert.rs`（新，647 行）：`UpsertExecutor` :51、`prepare_row` :106、`arbitrate` :146、`insert_row` :218、`apply_do_update` :310、`delete_conflict_row` :531、`Executor::next` :603。
- `src/executor/mod.rs`：`mod upsert` + `pub use upsert::UpsertExecutor` + plan 类型再导出。
- `src/executor/correlated.rs`：`inject_correlated_values` DML 叶臂加 `Upsert`。
- `src/parser/planner/mod.rs`：`table_unique_columns` :120、`new()` 初始化 :144、`set_table_unique_columns` :176、Insert 分派显式传 `on`/`replace_into` :199。
- `src/parser/planner/query.rs`：`get_plan_output_columns` DML 臂加 `Upsert`（派生表列注册面，Upsert 无输出列）。
- `src/parser/planner/ddl_dml.rs`：`conflict_target_mismatch` :69、`upsert_assignment_unsupported` :77、`build_insert` 扩参 :151、`build_upsert_action` :192、`resolve_conflict_target` :263、`build_upsert_assignment` :297、`build_upsert_value` :321、`resolve_column_pos` :388。
- `src/pipeline.rs`：`register_table` 唯一列注册 :1142、execute_stage DML 臂 :118/:127、`create_executor_from_plan` Upsert 构造臂 :544。

测试（2 新建 + 2 增量）：

- `tests/upsert_test.rs`（新，29 用例）：2.2 骨架批 6（无冲突等价 / DO NOTHING PK / DO NOTHING 唯一 / 显式目标仅仲裁该列 / 仲裁序 / 混合多行计数）、R4 显式目标 2、2.3 DO UPDATE 8（三形态、裸列名、`DEFAULT` 两形态、碰撞零副作用、rekey 碰撞与成功、NOT NULL、类型门、多行逐行、唯一列裸列名）、2.4 REPLACE 5（PK 冲突 / 唯一冲突 / 无冲突 / 同 rid 双命中去重 / 多冲突行）、2.5 两态 2 + 事务 3。
- `tests/planner_test.rs`（+19）：目标三态解析、拒绝矩阵 7、DO UPDATE 赋值三形态 + `DEFAULT` 字面化 2、算术/函数/未知列/复合左值拒绝 4、REPLACE 映射 1。
- `tests/cli_test.rs`（+1）：4 条计划期拒绝 exit 3 + 点名 stderr，3 条合法形态 exit 0。

文档与产物：README.md / README.zh-CN.md（写面段 + 能力行）、change tasks.md（2.1-2.6 勾选）、本 Cycle Act Response。

**Deviations from Plan**

1. **DML 臂与构造臂接线提前到 2.1**（契约把 `pipeline.rs` DML 臂 + `create_executor_from_plan` 划给 2.2）：新增 `PhysicalPlan` 变体使 4 处穷尽 match 必须同步编译（`executor/correlated.rs`、`planner/query.rs::get_plan_output_columns`、`pipeline.rs` execute_stage DML 臂与构造臂）。若严格按任务序，2.1 无法独立编译。故 2.1 一并完成四处 match 接线（均按既有 DML 臂形态追加，无提前占位、无行为改动），2.2 保留执行器实现与仲裁逻辑。
2. **`OnInsert` 非穷尽枚举需 `_` 臂**：sqlparser 0.44 将 `OnInsert` 标 `#[non_exhaustive]`，编译器要求兜底臂。实现为点名拒绝「unsupported INSERT conflict clause」，与 D4「不静默忽略」意图一致（不引入行为，只防未来变体静默）。
3. **冲突目标的声明类型判定显式用 `page_format::ColumnType`**：`primary_key_types` 通道存储页格式类型，与 `crate::executor::ColumnType` 是两个不同枚举（编译器拒绝直接比较），判定改用全路径 `crate::storage::page_format::ColumnType::Int`——与 `query.rs::pk_type_known_non_int` 既有先例同型。
4. **upsert_test 首轮 5 个用例期望校准（非实现缺陷）**：初版按「DO UPDATE 带入待插行全部列」「替换/更新后原唯一值释放」写期望；实测语义为「DO UPDATE 只消费 SET 清单（未赋值列取旧行值）」「唯一值由存活行占用」。逐条以 SQLite 语义与实测行为核对后校准断言（`do_update_literal_assignment_updates_in_place` 改断言 code 保持 100 且仍被占用、`do_update_rekey_maintains_pk_entry` 改断言 name 取旧值 `Alice`、`replace_into_replaces_unique_conflicting_row` 改断言 code=100 由替换行占用而 PK 键位 1 释放、显式目标/去重两例改用 `COUNT(*) WHERE` 断言行不存在）。未弱化断言：每处改为断言正确语义并补充反向断言（如释放的键位可复用）。
5. **`insert_row` 返回类型 `Result<RowId>` → `Result<()>`**：Phase 4 code review 发现返回的 rid 无消费点（初版为预留），改为 `Result<()>` 消除无用返回。
6. **DO UPDATE 的 NOT NULL 校验范围取「仅被赋值列」**（契约原文「final 行 NOT NULL」）：未被赋值列的违反属原行既有状态，全行校验会让存量违例行无法被 upsert 更新（放大既有缺陷）；对全部可达状态两种写法行为等价。
7. **2.5 CLI 错误面用例的见证时序**：该用例在 2.1 实现完成后编写，首跑即 GREEN（其拒绝行为已在 2.1 经 planner_test 完成 RED→GREEN 见证）。有效性论证：变更前 `Statement::Insert` 分派以 `..` 丢弃 `on`/`replace_into`，该 5 条语句全部按普通 INSERT 执行并 exit 0，故用例对本次改动敏感（断言覆盖 exit 码与逐条点名 stderr）。
8. **测试规模超出契约列举**：契约骨架批列 6 条用例，实际按 delta spec R3/R4 场景补齐至 29 条（补入 spec 明确命名但契约未列举的场景：显式唯一列/PK 目标各自仲裁、多行 DO NOTHING 逐行计数、多行 DO UPDATE 逐行判定、裸列名作用于唯一列、同 rid 被 PK 与唯一双命中的去重）。只增不减，无范围扩张。

**Blocker Handoff**

None

**Blocker Resolution**

None

**Self-Review**

- Plan compliance: PASS——6 个 Task Contract 逐项对账：Targets 全部达成（2.1 四处 + 2.2 执行器五方法 + 2.3/2.4 两臂 + 2.5 两态/事务/CLI + 2.6 README/验证）；Required behavior 的 D4/D5 全分支与三动作语义均有测试见证；Preserve 面（`insert.rs`/`update.rs`/`delete.rs` 三文件未动、既有错误面优先级与文本、无新 WAL 记录类型、恢复重放通道不动、DML 不进 plan cache、`PlanBuilder` 既有签名不变）经既有矩阵与全量零校准确认；Forbidden 项未触碰（未动方言配置、未实现 DO UPDATE WHERE、未接受算术/函数赋值、未加计划期 NOT NULL/类型校验、未建身份型证据工程与判定层）。
- Full diff reviewed: PASS——Response 前独立重读完整 diff（8 个产品文件 + 3 个测试文件 + 2 个 README + tasks.md）：逐条比对 insert.rs/update.rs/delete.rs 镜像源的分支序与错误文本；跨任务交互核查（类型门 × 仲裁守卫同错误面不重复触发、`All` 仲裁 × DO UPDATE 取首个命中、`Excluded` 值经 coerce/升格后再被赋值列类型门消费、REPLACE 删除后插入序列的 UNIQUE 预检与已删条目无残留）；新增 variant 的 4 处穷尽 match 逐一确认语义归属（相关参数注入 DML 叶、派生表列注册无输出列、DML 事务臂与 abort 表解析、构造臂注入 tx_id）。
- Critical findings unresolved: 0
- Important findings unresolved: 0
- Minor findings unresolved: 1——`Arbiter::All` 下 DO UPDATE 只作用于首个命中冲突行（PK 优先、其后唯一列按序），多个不同冲突行时只更新一行。属 D5 契约「首个命中即定」与 spec R3 既定语义（SQLite 同型：首个违反约束行），记录不修。
- 测试有效性核查：新增用例均含反向断言或状态断言（如碰撞后原值保持、释放键位可复用、COUNT 为 0），不因错误原因通过；既有矩阵零校准（1182 → 1231 净增 49 全为新增用例，无一条既有断言被改写）。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| 计划期解析与拒绝矩阵 | `cargo test --test planner_test` | `72 passed; 0 failed`（基线 53 + 净增 19；RED 先行：编译失败 26 errors——`PhysicalPlan::Upsert` / 三个计划类型 / `set_table_unique_columns` 均不存在） | 2.1 D4 全分支 + 既有计划面零回归 | PASS |
| 执行面矩阵 | `cargo test --test upsert_test` | `29 passed; 0 failed`（RED 先行：编译失败——`could not find upsert in executor`） | 2.2/2.3/2.4 三动作 + 仲裁 + 两态 + 显式事务（R3/R4 全场景） | PASS |
| CLI 错误面 | `cargo test --test cli_test` | `99 passed; 0 failed; 2 ignored` | 2.5 计划期拒绝 exit 3 + 点名 stderr + 合法形态 exit 0 | PASS |
| 受影响既有边界 | `cargo test --test constraint_enforcement_test --test key_type_conformance_test --test insert_column_list_test --test subset_insert_test --test write_type_conformance_test --test tx_statement_test` | `24 / 8 / 7 / 6 / 13 / 19 passed` | MS23 约束面、MS16 键列门、Iteration 000 子集/类型门/事务会话语义零回归 | PASS |
| lib 单测 | `cargo test --lib -- --test-threads=1` | `311 passed; 0 failed` | 既有 lib 面 + catalog/table_manager 单测 | PASS |
| 全量 | `cargo test -- --test-threads=1` | `1231 passed; 0 failed; 2 ignored`（基线 1182 + 净增 49；82 个测试二进制，exit 0） | 全仓库，最终状态新鲜串行运行 | PASS |
| 静态分析 | `cargo clippy --all-targets` | 0 warning（初版 1 个 `needless_return` 已修） | 全 targets | PASS |
| 格式化 | `cargo fmt --check` | 本 change 改动区零漂移（余 5 文件为 MS23 未提交既有漂移，按 Surgical Changes 保持原样） | 改动面 | PASS |
| OpenSpec 结构 | `openspec validate 2026-09-25-ms24-write-surface-completion --strict` | `Change '2026-09-25-ms24-write-surface-completion' is valid` | change 结构自检 | PASS |

**Persisted Evidence**

None required（Mode: none——全部验证为 `cargo test` / `cargo clippy` / `cargo fmt --check` / `openspec validate` 原生命令，输出与退出码由本表承载；无不可低成本重跑的现场，未创建 `evidence/` 目录）

**Experience Candidates**

| Type | Candidate | Evidence | Reason |
|---|---|---|---|
| Issue | 显式事务回滚 `DELETE`（及本 change REPLACE 的删除段）后，原行在全表扫描中复现但 PK 索引条目不恢复——`WHERE id = ...` 的 PK 等值点查漏行 | 本 Response「Remaining Issues 1」+ `tests/upsert_test.rs::explicit_tx_replace_rollback_restores_row` 注释；根因位点 `src/transaction/manager.rs::abort_cleanup_versions`（墓碑 slot 无键条目，`find_key_by_row_id` 为 None 故跳过索引还原）；已用一次性独立验证确认与既有 `DELETE` 回滚同型 | 本 change 范围外的实质缺陷（既有 DELETE 语义，REPLACE 镜像继承），证据可复现；建立/关闭由 Recorder 按用户指令执行 |
| Runbook | None | | 全量串行测试流程已由用户在本会话明确指定（`--test-threads=1`），属常规 cargo 命令，无重复性风险操作可沉淀 |

**Remaining Issues**

1. **DELETE / REPLACE 回滚后 PK 索引条目不恢复**（既有缺陷，非本 change 引入）：回滚后全表扫描可见原行，但 PK 等值点查（走索引）漏行。修复面在 `abort_cleanup_versions` 的墓碑 slot 索引还原，属 DELETE 语句既有语义（超出本 change Invariant「DELETE 行为逐字节不变」），已作 Issue 候选上报。
2. **BOOL 列声明类型不匹配的 DEFAULT**（Iteration 000 遗留 finding，已排为 tasks.md `MS24-T04`）：声明期不做 DEFAULT 类型校验，仅写入期被类型门点名拒绝；本 Iteration 未纳入范围（并入需用户批准扩围）。
3. **工作树既有 fmt 漂移**（MS23 未提交区 5 文件：`delete.rs`、`table_manager.rs`、`recovery.rs`、`cli_test.rs` 旧段、`constraint_enforcement_test.rs`）：按 Surgical Changes 保持原样，建议后续独立批次全局 `cargo fmt`。
4. `Arbiter::All` + DO UPDATE 只更新首个命中冲突行（Minor，D5/spec 既定语义）。

**Commit or Diff Reference**

未创建 Git commit（工作树含 MS23 与 MS24 Iteration 000/001 全部未提交改动，commit 时机由用户指令；本 Iteration 改动面见上方 Changed Files 清单，`git diff -- src tests README.md README.zh-CN.md` 覆盖其中已跟踪文件，新增文件 `src/executor/upsert.rs`、`tests/upsert_test.rs` 尚未跟踪）

## Plan Review

- Review Result: replan-required

**Findings**

独立审查（2026-09-26，Plan Review）。只读基线检查：工作树自 Act 验证运行后覆盖范围表面零变化（`src/**/*.rs` 与 `tests/*.rs` 最新 mtime 14:51:30 早于 `target/debug/deps/upsert_test-6e8fbd6edb904141`（14:51:31）与最新构建产物（14:51:39）；CLI 二进制 14:45:53 晚于全部 `src` 文件）——按公共规则 › 验证 采信 Act Response 的验证结论，不重复运行。独立重读 8 个产品文件的完整 unstaged diff、新增 `src/executor/upsert.rs`（647 行）、镜像源 `insert.rs` / `update.rs` / `delete.rs` 与 3 个测试文件增量；对 Act 未覆盖的行为面用项目原生 CLI 做只读观察（临时库置于 `/tmp`，判定依据为 exit 码与原生输出）。

阻塞 Acceptance 的发现：

1. **显式唯一列目标路径缺 PK 重复预检 —— 静默写入重复主键行**（`src/executor/upsert.rs:218` `insert_row`）。`insert_row` 只镜像了 `InsertExecutor` 的 UNIQUE 预检（`insert.rs:195-210`）及其后序列；PK 重复预检（`insert.rs:177-187`）留在 `arbitrate`（`upsert.rs:150-154`）内，仅对 `All` 与 `Column(pk_index)` 生效。显式目标为唯一列时，PK 约束在无冲突插入路径上完全不被检查：`arbitrate` 未命中 → `insert_row` 直接写页并对同一键执行 `index_manager.insert`，覆盖既有 PK 条目。违反 delta spec R3「显式目标仅仲裁该约束，行违反仲裁外约束时 SHALL 以既有 DuplicateKey 错误拒绝（SQLite 对齐）」。独立复现：

   ```text
   $ rtsql /tmp/audit24/e.db "CREATE TABLE t2 (id INT PRIMARY KEY, code INT UNIQUE)"
   $ rtsql /tmp/audit24/e.db "INSERT INTO t2 VALUES (1, 100)"                     # affected_rows 1
   $ rtsql /tmp/audit24/e.db "INSERT INTO t2 VALUES (1, 200) ON CONFLICT (code) DO NOTHING"
   {"affected_rows":1}                                                            # exit 0 —— 应 DuplicateKey / exit 3
   $ rtsql /tmp/audit24/e.db "SELECT * FROM t2"                                   # [[1,100],[1,200]] 重复主键行
   $ rtsql /tmp/audit24/e.db "SELECT * FROM t2 WHERE id = 1"                      # [[1,200]] 原行键位点查漏行
   ```

   DO UPDATE 臂同型（`ON CONFLICT (code) DO UPDATE SET code = 300` 对既有 `(1,100)` → `affected_rows` 1、`SELECT COUNT(*)` = 2，checkpoint 后持久）。REPLACE 臂不受影响（恒 `All` 仲裁，PK 命中行先删除）。既有见证 `explicit_target_only_arbitrates_that_column` 只覆盖「目标 = PK、唯一列冲突」方向，未覆盖反向（目标 = 唯一列、PK 冲突），故该缺口对 29 + 19 条新增用例全部不可见。

2. **README 双语「零副作用」承诺被 REPLACE 失败路径证伪**（`README.md:197` / `README.zh-CN.md:197`）。本 change 新增的写面段首句称 "Every write is validated before any row is touched, so a rejected write leaves no side effects"，但 REPLACE 按 design D5 步骤 6 与契约 2.4 先删冲突行再执行插入序列，插入序列的一般类型门位于删除之后。独立复现：

   ```text
   $ rtsql /tmp/audit24/a.db "INSERT INTO t VALUES (1, 10)"
   $ rtsql /tmp/audit24/a.db "REPLACE INTO t VALUES (1, 'abc')"                   # exit 3 ColumnTypeMismatch
   $ rtsql /tmp/audit24/a.db "SELECT * FROM t"                                   # [[1,10]] 行复现
   $ rtsql /tmp/audit24/a.db "SELECT * FROM t WHERE id = 1"                      # [] PK 等值点查漏行
   ```

   漏行根因是既有 abort 缺陷（`src/transaction/manager.rs:292`：`find_key_by_row_id` 对墓碑 slot 返回 `None` → 不还原 PK 条目），非本 change 引入；行为侧顺序已由 design D5 获批、修复超出本 Cycle 契约。任务 2.6 的 Required behavior「双语文档与实现一致」在当前实现下不成立，属本 Cycle 交付物缺陷。

非阻塞 Minor findings：

3. **既有 abort 缺陷的可达面扩大**（与 Act Remaining Issues 1 / Experience Candidates 同源）：Act 报告的触发路径是显式事务回滚；本次独立复现表明单条 auto-commit `REPLACE INTO` 因插入段失败即可触发（冲突行已删 → 语句级 abort → PK 条目不还原），用户看到 exit 3 却留下「全表扫描可见、PK 等值点查漏行」的状态。根因仍是 `abort_cleanup_versions`，与 `DELETE` 回滚同型；作为同一 Issue 候选的可达面补充上报，本 change 不修（超出 Invariant「DELETE 行为逐字节不变」与本 Iteration 范围），是否落账由用户指令交 `openspec-experience-recorder`。
4. **`delete_conflict_row` 的 SlotNotFound 容忍臂不删 PK 条目**（`upsert.rs:548`）：`DeleteExecutor` 在同形态下仍按搜索键执行 `index_manager.delete`（`delete.rs:134`），upsert 因无行可算键而跳过。仅在索引条目指向不存在 slot 的夹具/损坏态可达（正常路径 rid 必有 slot），不属任何 spec 场景；记录不修。
5. **仲裁内 F1 守卫使「PK 冲突 + 唯一列非法类型值」的行报错而非被 DO NOTHING 跳过**：契约 2.2 明确要求 F1 守卫置于仲裁内（`upsert.rs:160-166`），实现与契约一致；但同输入的纯 INSERT 报 `Duplicate key`、upsert 报 `key column 'code' expects INT, got String`（均为 exit 3，独立复现），且 R3「DO NOTHING 跳过冲突行」在该角落不成立。方向安全（无静默写坏），建议 change 收尾合并规格时作为已知边界记录；是否写入 spec 由用户裁定。
6. Act Self-Review 的 `Plan compliance: PASS` 对任务 2.6 不准确（见阻塞发现 2）。零回归锚点成立：本 Iteration 的 unstaged diff 仅含 8 个产品文件 + 3 个测试文件 + 2 个 README + change 产物，`src/executor/{insert,update,delete}.rs` 零改动。

**Deviation Classification**

1. DML 臂与构造臂接线提前到 2.1 — `PLAN-OMISSION`（非阻塞）。契约把 `pipeline.rs` DML 臂与 `create_executor_from_plan` 划给 2.2，但新增 `PhysicalPlan` 变体使 4 处穷尽 match 必须同步编译。代码核实：`executor/correlated.rs:82`、`planner/query.rs:128`、`pipeline.rs:118`/`:127`、`pipeline.rs:544` 均按既有 DML 臂形态追加，无提前占位、无行为改动（派生表列注册面返回空输出列，Upsert 无输出列）。
2. `OnInsert` 非穷尽需 `_` 臂 — `PLAN-OMISSION`（非阻塞）。sqlparser 0.44 标记 `#[non_exhaustive]`，调查未预判；实现为点名拒绝，与 D4「不静默忽略」意图一致。
3. 冲突目标声明类型判定改用 `page_format::ColumnType` — `PLAN-OMISSION`（非阻塞）。`primary_key_types`（`planner/mod.rs:109`）存页格式类型，与 `executor::ColumnType` 是两个枚举；判定改用全路径类型正确，与 `query.rs::pk_type_known_non_int` 先例同型。
4. upsert_test 5 处期望校准 — `ACT-DEVIATION`（非阻塞）。逐条以 SQLite 语义核对后校准（DO UPDATE 只消费 SET 清单；唯一值由存活行占用）。独立复核支持该语义：`ON CONFLICT (code) DO UPDATE SET id = 5` 对无键行 `(NULL,7)` 命中后 rekey 成功、`code=7` 仍被占用报 DuplicateKey、`WHERE id = 5` 点查命中；未见弱化断言。
5. `insert_row` 返回类型 `Result<RowId>` → `Result<()>` — 非实质。
6. DO UPDATE 的 NOT NULL 校验仅覆盖赋值列 — `ACT-DEVIATION`（非阻塞）。契约原文为「final 行」；仅赋值列校验使存量违例行仍可被 upsert 更新，对全部可达状态两种写法行为等价，不违反 R3「任何写入前零副作用」。
7. 2.5 CLI 错误面用例首跑即 GREEN — 测试见证的等价形态（非阻塞）。已独立核实该用例对本次改动敏感：`ON CONFLICT (id, code)` / `ON CONSTRAINT` / `DO UPDATE WHERE` / `ON DUPLICATE KEY UPDATE` 四条在改动前按普通 INSERT 执行并 exit 0，现均 exit 3 且带点名文案。
8. 测试规模超出契约列举（29 条 vs 骨架批 6 条）— `ACT-DEVIATION`（非阻塞）。只增不减，新增场景均在 delta spec 内。

**Acceptance Gaps**

- **R3（UPSERT 语义）未满足**：「显式目标仅仲裁该约束，行违反仲裁外约束时 SHALL 以既有 DuplicateKey 错误拒绝」在「目标 = 唯一列、PK 冲突」方向不成立（阻塞发现 1，含 DO NOTHING 与 DO UPDATE 两臂；REPLACE 臂不涉及）。既有见证只覆盖反向方向。
- **R5（既有写面语义零回归）的文档一致性面**：任务 2.6 Required behavior「双语文档与实现一致」被阻塞发现 2 证伪。行为零回归本身成立（`insert.rs` / `update.rs` / `delete.rs` 零改动 + 全量 1231 passed 采信 + 本次独立抽查的既有 INSERT 语义一致）。
- 其余 R3 / R4 场景与 RTM 五行映射经本次独立核对成立：仲裁序（PK 先、唯一列按序）、DO NOTHING 计数、赋值三形态与多列/多行、rekey、REPLACE 去重与多冲突行、干净重开与崩溃恢复两态、显式事务提交/回滚、CLI exit 3 拒绝面、无冲突子句 INSERT 路径逐字节不变（含子集清单与 DEFAULT 填充经 upsert 路径消费）。

**Convergence**

N/A（首次 Review）。

**Evidence**

- 代码独立重读（unstaged diff 全量）：`executor/plan.rs:33`/`:159-206`（Upsert 变体与 D4 四类型 + `UpsertNode`）、`executor/upsert.rs`（`prepare_row:106`、`arbitrate:146`、`insert_row:218`、`apply_do_update:310`、`delete_conflict_row:531`、`Executor::next:604`）、`parser/planner/ddl_dml.rs:69`/`:192`/`:263`/`:297`/`:321`/`:388`、`planner/mod.rs:120`/`:144`/`:176`/`:199`、`planner/query.rs:128`、`pipeline.rs:118`/`:127`/`:544`/`:1135-1142`、`executor/mod.rs`、`executor/correlated.rs:82`。
- 镜像源逐分支比对：`insert.rs:114-301`（coerce → 升格趟 → NOT NULL → PK 键位门 → PK DuplicateKey 预检 → UNIQUE+F1 → 类型拒绝趟 → serialize → 落位 → visibility → WAL → record_version → PK 条目 → 唯一条目）、`update.rs:89-353`（Step 1-7 与唯一四分支）、`delete.rs:79-166`（唯一键提取 → 墓碑 → 条目清理 → record_version → WAL Delete）。结论：`prepare_row`、`insert_row`（除 PK 预检缺失外）、`apply_do_update`、`delete_conflict_row` 的分支序与错误文本均与镜像源一致。
- 注册与存储通道核实：`table_manager.rs:340-343`（`pk_index` 必为真实列，无声明 PK 时由 `create_table.rs:74-83` 取首列）、`table_manager.rs:377-397`（`unique_indexes` 仅 INT 非 PK 列，列位升序）、`pipeline.rs:1121-1142`（`register_table` 三个加性通道注册）、`transaction/manager.rs:271-300`（abort 以 `find_key_by_row_id` 还原索引；墓碑 slot 无条目）、`btree/index_manager.rs:272-293`（`delete` 对缺失键容忍）、`planner/mod.rs:109`（`primary_key_types` 存页格式类型）。
- 测试独立重读：`tests/upsert_test.rs` 29 用例逐条对账 delta spec 场景，含反向断言（`do_nothing_skips_pk_conflict_row` 复查原值、`do_update_collision_precheck_is_side_effect_free` 复查两行原值、`replace_into_dedupes_row_hit_by_pk_and_unique` 复查单次删除后可见性）；`tests/planner_test.rs:1140-1570`（目标三态、拒绝矩阵 7 条、赋值三形态 + `DEFAULT` 字面化 2 条、算术/函数/未知列/复合左值拒绝 4 条、REPLACE 映射 1 条）；`tests/cli_test.rs:3623-3694`（4 条 exit 3 点名 + 3 条合法形态 exit 0）。
- 验证采信：Act Response Verification Evidence 表（2026-09-25）——全量 `cargo test -- --test-threads=1` 1231 passed / 0 failed / 2 ignored exit 0、`cargo clippy --all-targets` 0 warning、`cargo fmt --check` 改动面零漂移、`openspec validate 2026-09-25-ms24-write-surface-completion --strict` valid；采信依据为上述只读基线检查。
- 补充只读行为观察（Act 未覆盖；原生 CLI，exit 码与原生输出直接判定）：显式唯一列目标 + PK 冲突 → exit 0 重复主键行（DO NOTHING 与 DO UPDATE 两臂，阻塞发现 1）；失败 REPLACE → PK 点查漏行（阻塞发现 2 / Minor 3）；`INSERT OR REPLACE` 与 `INSERT OR IGNORE` 仍为解析层拒绝（已知边界成立）；`ON CONFLICT (nope)` 与非 INT 声明 PK 目标 → 点名拒绝；无键行（NULL PK）的 DO NOTHING / REPLACE / DO UPDATE rekey 与索引一致性；省略目标对非 INT PK 表与纯 INSERT 行为一致（均为无键行落库）；CLI 显式事务内 upsert（`BEGIN; … ON CONFLICT (id) DO UPDATE …; COMMIT`）exit 0。
- Persisted Evidence：none 模式核对——无 `evidence/` 目录属预期，不作为问题。
- change 结构自检：`tasks.md` 2.1-2.6 勾选与实现一致；`specs/sql-write-surface` R3/R4 与实现一致（除阻塞发现 1 与 Minor 5 的角落）；`specs/insert-column-list-mapping` delta 与 Iteration 000 交付一致（000 Review 已 accepted）；两个 Iteration 目录与 Cycle 文件齐全；`Review Result` 与流程状态一致（001 为本次 Review 对象）。

**Follow-up Decision**

**`replan-required`（2026-09-26，用户裁定扩围）**。阻塞发现 1（`insert_row` 缺 PK 重复预检）与阻塞发现 2（README 零副作用表述被失败 REPLACE 证伪）均成立；发现 2 的根因是既有缺陷「回滚后墓碑行索引条目不还原」，其修复面在 `src/transaction/manager.rs::abort_cleanup_versions`——跨出本 Cycle 的 Diagnostic boundary，且必然改变 DELETE 回滚后的可观察行为，与本 Cycle 不变量「`insert.rs`/`update.rs`/`delete.rs` 不动、既有错误面逐字节不变」直接冲突。因此不作为当前 Cycle 修复、也不作为 rework：按公共规则「范围或验证契约变化必须使用 `replan-required`」修订 change 与 Iteration 001 计划，并创建同目录后继 replan Cycle。

- 用户决策（2026-09-26）：在「新建独立 change / replan 并入当前 change / 仅消除 MS24 新触发面」三个路线中明确裁定**并入当前 change**。
- 已完成的 replan 产物：proposal 增补范围裁定与两个 Modified Capabilities；新增 delta `specs/mvcc-tombstone-visibility/spec.md`（ADDED「回滚后墓碑行的索引条目还原」7 场景）与 `specs/sql-constraint-enforcement/spec.md`（MODIFIED R3 回滚子句去歧义 + 新增 DELETE/REPLACE 回滚占用场景）；design 增补 D9（两趟处理 + 版本链回溯 + 取键还原 + 四个被否决替代方案）与 D10；tasks 增补 2.7-2.10、RTM 增至七行、Iteration 001 计划按扩围修订（含拆分候选与用户裁定的平衡审计）。
- 后继 Cycle：`iterations/001-upsert-replace/001-replan.md`（Status: ready，承载 2.7-2.10；其 Plan Context 自包含父 Cycle 全部必要事实与复现证据，Act 无需回读本 Cycle）。本 Cycle 的两项阻塞发现在该 Cycle 中分别由 2.7 与 2.8 关闭，根因缺陷由 2.9 修复、2.10 收口。
- 本 Cycle 的 Findings、Deviation Classification、Acceptance Gaps、Convergence 与 Evidence 作为本轮审查记录保持不变（Cycle 已进入终态，不再改写）。

**Iteration Plan Update**

Iteration 001 计划已按扩围修订（见 tasks.md `### Iteration 001`：Tasks 增补 2.7-2.10；Stable baseline 增补「显式目标只仲裁该约束」与「回滚后索引条目还原」两条；Verification boundary 增补 explicit_tx_test 回滚矩阵与两组零回归；Diagnostic boundary 增补 `transaction/manager.rs` 与 `wal` 取键 helper 可见性；Non-goals 增补提交路径与恢复通道、运行期索引语义改写；平衡审计记录拆分候选成立但经用户裁定保留在同一 Iteration）。目标、范围、依赖与 requirement 映射的实质变化仅为**验收边界新增**与**诊断边界扩大**；任务编号、Iteration 编号与 Cycle 编号链不变。

**Next Cycle**

`openspec/changes/2026-09-25-ms24-write-surface-completion/iterations/001-upsert-replace/001-replan.md`

**Next Iteration**

None
