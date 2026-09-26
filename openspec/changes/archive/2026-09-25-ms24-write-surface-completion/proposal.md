# MS24 SQL 写面补全（子集 INSERT / DEFAULT / UPSERT / 写入类型门） — Proposal

## Why

SQL 写面存在三类已定位的功能与正确性缺口（MS18-MS27 路线中居统一执行序第二位，紧跟 MS23 约束执行面）：

- **子集 INSERT 与 DEFAULT 是「解析即丢弃」**：`map_insert_values`（`src/parser/planner/ddl_dml.rs:151`）要求列清单「恰为表列排列」，子集清单被计划期拒绝（数量不符即拒）；DEFAULT 值经 `extract_default_value`（`ddl_dml.rs:412`）解析进 `ColumnSchema.default_value`（`src/executor/plan.rs:227`）后被建表路径整体丢弃——`create_table_with_constraints` 入参为 4 元组不含 default（`src/storage/data/table_manager.rs:307`），`CatalogColumnRow` 无该字段不持久化（`src/storage/catalog.rs:71`），`create_table_sql` 注释明写「DEFAULT 不在 catalog 持久化面，不输出」（`src/cli/lifecycle.rs:576`）。两者都是 agent 生成 SQL 的高频写入形态。
- **UPSERT 子句静默忽略**：`build_plan` 的 `Statement::Insert` 分派（`src/parser/planner/mod.rs:163`）以 `..` 丢弃 `on`/`or`/`replace_into` 字段——`ON CONFLICT DO NOTHING/DO UPDATE` 按普通 INSERT 执行（冲突即 DuplicateKey 报错，子句从未生效）、`REPLACE INTO` 静默降级为普通 INSERT（replace 语义静默丢失）。静默忽略比诚实拒绝更危险（用户以为 upsert 生效），且 ON CONFLICT 是 SQLite/PG 写入惯用法。
- **非键列写入类型校验缺失（ISS04，R29）**：写入值与列声明类型全程无校验（String/Float 静默写入 INT 列，`serialize_tuple` 按值变体打 tag 不与 schema 交叉校验，`src/storage/page_format/tuple.rs:67`），属「静默写坏」正确性同类缺陷；2026-09-25 MS23 Review 登记（`.claude/issues/ISS04-non-key-column-write-type-validation-missing.md`）。

## What Changes

- **子集列清单 INSERT + DEFAULT 应用**：列清单从「恰为全列排列」放宽为「每项解析为互异已知列的子集」；省略列取声明 DEFAULT（无 DEFAULT 取 NULL），NOT NULL 列的 NULL 由既有执行器门零副作用拒绝（单点强制，不新增计划期重复路径）。DEFAULT 语义补全为完整闭环：CREATE TABLE 解析的 DEFAULT 字面量经 catalog 表行向后兼容追加持久化（MS23 unique_roots 先例）、重启后生效、dump DDL 渲染 DEFAULT 使 dump→restore 往返保真；`VALUES` 中的 `DEFAULT` 关键字等价于省略该列（sqlparser 0.44 无 `Expr::Default` 变体，解析为 `Identifier("DEFAULT")`，planner 识别）。
- **写入类型一致门（ISS04）**：INSERT/UPDATE 在任何写入前校验写入值变体与列声明类型一致——日期族 String 经既有 coerce 落类型后通过；FLOAT 列接受整数值无损升格（Int→Float，用户裁定）；其余跨类型以点名列名/期望/实际的错误拒绝，零副作用。一般类型门作为最后的写入前置校验（既有 PK `KeyTypeMismatch`、唯一列 F1 守卫、`NullConstraintViolation` 的触发优先级与错误文本保持不变）；单点覆盖 INSERT/UPDATE/dump/restore/import 全部写入通道。
- **UPSERT（ON CONFLICT，SQLite 子集对齐）**：`INSERT ... ON CONFLICT [target] DO NOTHING`——冲突行跳过（不计入 AffectedRows）；`DO UPDATE SET col = expr, ...`——冲突行原位更新，赋值表达式支持字面量、`excluded.col`（新行值）、裸列名（旧行值）三形态与多列赋值（用户裁定），算术/函数表达式计划期点名拒绝；`DO UPDATE WHERE` 本 change 点名拒绝（用户裁定范围的显式缺口，随 design 记录）。冲突目标：省略 = PK + 全部唯一索引仲裁（SQLite 语义，PK 先、唯一列按序）；显式单列须解析为 INT PK 或 INT 唯一列；组合多列目标与 `ON CONSTRAINT` 以 SQLite 语义「does not match any PRIMARY KEY or UNIQUE constraint」拒绝（引擎无组合唯一约束，该错误语义精确）；MySQL `ON DUPLICATE KEY UPDATE` 点名拒绝。冲突行定位复用 MS23 唯一索引基建；DO UPDATE 写形状镜像 UPDATE 语义（版本链、WAL Update、PK/唯一索引维护、rekey 碰撞预检）；恢复两态结构成立（WAL Insert/Update 记录均有既有重放通道）。
- **REPLACE INTO（SQLite 对齐）**：`REPLACE INTO`（GenericDialect 下解析为 `Statement::Insert { replace_into: true }`，当前同样被静默降级）映射为 replace 语义——冲突 → 删除冲突行（既有删除墓碑 + 索引清理通道）+ 插入新行，计 1 行。
- **静默忽略面收口**：`Statement::Insert` 分派不再以 `..` 丢弃 `on`/`replace_into`——被消费或被点名拒绝，消除「子句写了但从未生效」的静默面。`INSERT OR ...` 形态在 sqlparser GenericDialect 下不可达（仅 SQLiteDialect 解析，`src` sqlparser `parser/mod.rs:8335`），维持解析层拒绝并记录为已知边界（方言迁移不在本 change）。
- **回滚后墓碑行索引条目还原**（2026-09-26 Iteration 001 Plan Review 扩围，用户裁定并入本 change）：删除者事务 ROLLBACK 时，墓碑行在删除前最新存活版本的 PK 与唯一索引条目 SHALL 全部还原（`abort_cleanup_versions`）。修复前该条目在删除时已被清理、回滚时因墓碑 slot 无索引条目而无法定位而整体跳过，产生两类可观察错误结果——PK 等值点查漏行（行在全表扫描可见）与唯一值被释放后可再次插入（两条存活行共享同一唯一值，`UNIQUE` 约束静默失效）。本项同时消除 REPLACE 失败语句（先删后校验）留下的同类索引缺口。属既有缺陷（MS07-T04 建通道时只覆盖 INSERT / UPDATE 形态），本 change 首次以用户可见证据定位并纳入范围。
- **范围裁定（用户批准）**：MS24-T01/T02/T03 全部任务并入单一 change；DO UPDATE SET 支持字面量 + excluded + 旧行引用（用户裁定）；Float 升格（用户裁定）；冲突目标 SQLite 对齐（用户裁定「SQLite 有的我们都要有」授权，我据此采纳省略目标=全仲裁 + 显式单列 + 组合/ON CONSTRAINT 精确拒绝 + REPLACE INTO，OR 形态因方言不可达记录边界）。**2026-09-26 追加裁定**：Iteration 001 Plan Review 定位的既有缺陷（回滚后墓碑行索引条目不还原）经用户在「新建独立 change / replan 并入当前 change / 仅消除新触发面」三选项中裁定**并入当前 change**，按 `replan-required` 修订 Iteration 001 计划并创建 replan Cycle（该扩围改变验收边界与诊断边界，不作为 rework 处理）。

## Capabilities

### New Capabilities

- `sql-write-surface`: 定义 SQL 写入面的补全语义——子集列清单 INSERT 与 DEFAULT 的解析/持久化/应用闭环、写入值与列声明类型的强制一致门、UPSERT（ON CONFLICT DO NOTHING/DO UPDATE 与 REPLACE INTO）的冲突仲裁与原位更新语义、UPSERT 冲突目标与不支持形态的点名拒绝、既有写面语义零回归。

### Modified Capabilities

- `insert-column-list-mapping`: 列清单从「恰为表列集合的一个排列」放宽为「表列的子集排列」，缺省列按 `sql-write-surface` 的 DEFAULT/NULL 语义填充为全宽行（原「部分清单计划期拒绝」场景的 panic 消除意图由全宽填充承接）；未知列/重复列/无清单行长度不符的计划期拒绝与无 panic 保证保持（变更内携带该 capability 的 MODIFIED delta）。
- `sql-constraint-enforcement`: R3「INT 列 UNIQUE 强制」的事务回滚子句去歧义——回滚条目修复按占用关系分形态表述（回滚的 INSERT/UPDATE 不留占用、回滚的 DELETE/REPLACE 由复现行重新占用），并新增「DELETE / REPLACE 回滚后同值被复现行占用」场景；唯一性强制、错误面与既有场景逐条保持（变更内携带该 capability 的 MODIFIED delta）。
- `mvcc-tombstone-visibility`: 新增「回滚后墓碑行的索引条目还原（PK 与唯一）」Requirement——既有 R3 只覆盖扫描路径（「行为与该删除未发生一致（扫描路径）」），本 Requirement 补齐索引路径的两态一致（变更内携带该 capability 的 ADDED delta）。

说明：`schema-persistence` 的持久化通道不变（DEFAULT 按同一追加模式扩展字段）；`key-column-type-conformance` / `sql-constraint-enforcement` 的既有错误面优先级与文本不变（一般类型门在其后触发）。

## Impact

- 代码：
  - `src/parser/planner/mod.rs` — Statement::Insert 分派消费 `on`/`replace_into`
  - `src/parser/planner/ddl_dml.rs` — build_insert 冲突子句、map_insert_values 子集/DEFAULT 填充、DEFAULT 关键字臂
  - `src/executor/plan.rs` — UpsertNode / 冲突策略与赋值表达式载体、CreateTableNode DEFAULT 透传
  - `src/executor/insert.rs` — 一般类型门、子集行、冲突仲裁与 DO NOTHING/DO UPDATE 分派
  - `src/executor/update.rs` — 一般类型门（assigned 列）
  - 新 `src/executor/upsert.rs`（或 insert.rs 内分派——随 design 定稿）— DO UPDATE 原位更新与 REPLACE 路径
  - `src/storage/data/table_manager.rs` — TableMeta 携带 per-column DEFAULT、create/open 接线
  - `src/storage/catalog.rs` — CatalogColumnRow 追加 default 字段（向后兼容读）
  - `src/cli/lifecycle.rs` — create_table_sql 渲染 DEFAULT（dump 保真）
  - `src/parser/error.rs` / `src/storage/error.rs` — 点名拒绝错误面
  - `src/transaction/manager.rs`（2026-09-26 扩围）— `abort_cleanup_versions` 墓碑行的 PK/唯一索引条目还原（版本链回溯 + 存活版本元组取键 + 与索引条目移除分趟处理）
  - `src/wal/recovery.rs` / `src/wal/mod.rs`（2026-09-26 扩围）— `extract_index_keys` 提为 `pub(crate)` 供事务层复用（可见性变更，行为零变化）
- 测试：新增 `tests/upsert_test.rs`、`tests/subset_insert_test.rs`（或并入既有文件）；`tests/insert_column_list_test.rs` 子集拒绝用例按新语义校准（预期内）；cli_test 增量（DEFAULT 渲染 / upsert 错误面）；`tests/explicit_tx_test.rs` 增量（回滚索引还原矩阵，2026-09-26 扩围）。
- 已知边界（用户裁定/调查确认）：DO UPDATE WHERE v1 拒绝；`INSERT OR ...` 方言不可达维持解析层拒绝；非 INT 声明 PK 列的键位类型错配（如 STRING PK 收 Int 值落无键行）由一般类型门顺带覆盖写入面，键控语义（I024 键编码域）不变。
