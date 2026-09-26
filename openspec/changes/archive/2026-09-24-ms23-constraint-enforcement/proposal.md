# MS23 数据完整性约束执行面 — Proposal

## Why

建表 DDL 声明的约束一半从未被执行，属「DDL 静默接受、运行期永不生效」的静默错误结果类缺陷（MS15/MS16 同类纪律，正确性红线）：

- **NOT NULL / UNIQUE 只存不用**：约束经 `extract_column_constraints`（`src/parser/planner/ddl_dml.rs:369`）解析并持久化进 catalog（`__columns` 行 not_null/unique 字节），但 `TableMeta` 只承载 `(name, type)` 运行时形状（`src/storage/data/table_manager.rs:274` 注释明写「约束是 catalog 元数据，不进入内存 schema」），INSERT/UPDATE 执行器零消费（`src/executor/insert.rs`、`src/executor/update.rs` 无 not_null/unique 检查点）；`create_table_with_constraints` 注释自述 "The flags are metadata only: no INSERT or recovery path enforces them"（`table_manager.rs:222-226`）。
- **CHECK / FOREIGN KEY / 方言项静默丢弃**：`extract_column_constraints` 的 `_ => {}` 分支注释明写 "Null, ForeignKey, Check, DialectSpecific, etc. are ignored"（`ddl_dml.rs:390`）；表级约束仅消费 PRIMARY KEY，表级 CHECK/FOREIGN KEY/UNIQUE 同样静默忽略。

用户已批准 Explorer 缺口调查结论「这些问题存在且应当解决」，MS23 居初版后统一执行序首位（正确性红线先于一切新功能）。

## What Changes

- **NOT NULL 写入强制**：`TableMeta` 携带 per-column not_null 标志（从既有 catalog 持久化读回，无格式变化）；INSERT 在日期 coerce 后、任何写入前逐列校验，UPDATE 对 SET 目标列校验；违反 → 新 `StorageError::NullConstraintViolation { column }` 点名拒绝，零副作用。未声明 NOT NULL 的列（含未声明的 PK 列 NULL keyless 语义）行为不变。
- **约束诚实化拒绝**：列级 `CHECK` / `FOREIGN KEY` / `DialectSpecific`（如 AUTO_INCREMENT）从静默忽略改为建表计划期点名拒绝；表级 `CHECK` / `FOREIGN KEY` 同样点名拒绝（`Null`/`Comment` 等无语义期望选项维持忽略）。消除「建表成功但约束永不生效」的静默面。
- **UNIQUE 强制（INT 列）**：声明 UNIQUE 的 INT 列各建一个内部非 PK 唯一 B-Tree 索引（不经用户 CREATE INDEX 语句）；INSERT 重复 → `DuplicateKey` 同型拒绝（预检零副作用），UPDATE 维护唯一条目（碰撞预检/同值随行/改值 rekey/置 NULL 删条目四分支），DELETE 移除条目，事务回滚经既有 abort 索引修复通道修复唯一条目；唯一列 NULL 不参与唯一性（SQL 标准 + 既有 keyless 先例）。
- **UNIQUE 两态一致**：catalog 表行格式向后兼容追加唯一索引根页字段（`u32 unique_count | N × u32 roots`，旧行兼容读，checkpoint 位点 24B 先例）；干净重开消费持久化根；崩溃恢复（`redo_count > 0` 去信任重建通道）从最终数据页重建唯一索引，重建发现跨链重复 → 显式 `RedoFailed` 报错；`drop_table` 释放唯一索引页。
- **UNIQUE DDL 策略面**：非 INT 列（String/Float/Bool/Date/Timestamp）声明 UNIQUE → 建表点名拒绝（`Value::to_key` 仅 Int 产键，`src/executor/value.rs:92`；键编码扩展属 I024/MS21 域）；表级单列 `UNIQUE(col)` 映射为该列 UNIQUE 标志（等价列级声明）；表级多列组合 UNIQUE 点名拒绝（不支持组合唯一）。
- **存量兼容破坏（用户裁定接受）**：存量库若已有非 INT UNIQUE 列，其 dump 出的 DDL 在 restore 时会被新拒绝面挡住；项目未发布、存量风险低，文档记录该边界，不做 dump 端降级。
- **范围裁定（用户批准）**：T01+T02+T03 单 change 双 Iteration 交付；FOREIGN KEY 强制执行、用户级 CREATE/DROP INDEX、约束随 ALTER 演进、DEFAULT 应用（MS24-T01）不在本 change。

## Capabilities

### New Capabilities

- `sql-constraint-enforcement`: 定义建表约束的执行与诚实化语义——NOT NULL 写入强制、INT 列 UNIQUE 经非 PK 唯一索引强制与崩溃恢复两态一致、CHECK/FOREIGN KEY/方言项与不支持 UNIQUE 形态的计划期点名拒绝、既有无约束语义零回归。

### Modified Capabilities

无。`schema-persistence` 描述约束标志的持久化通道，本 change 不改变其语义（标志继续持久化、继续渲染）；`mvcc-tombstone-visibility` 的已知时序边界段不因唯一索引引入新变化（唯一索引与 PK 索引共享同一生命周期语义）。

## Impact

- 代码：
  - `src/executor/plan.rs` / `src/parser/planner/ddl_dml.rs` — 约束解析收紧与 UNIQUE DDL 策略
  - `src/storage/data/table_manager.rs` — TableMeta 约束标志与唯一索引承载、create/open/replace/drop 接线
  - `src/executor/insert.rs` / `update.rs` / `delete.rs` — 写路径约束校验与唯一索引维护
  - `src/transaction/manager.rs` — abort 索引修复扩展到唯一索引
  - `src/storage/catalog.rs` — 表行格式向后兼容追加
  - `src/storage/btree/index_manager.rs` — root 同步上下文泛化
  - `src/storage/error.rs` — `NullConstraintViolation` 变体（经既有 Sql 失败路径渲染，exit 3 同面）
  - `src/wal/recovery.rs` — 恢复重建唯一索引
- 测试：`tests/planner_test.rs` 拒绝矩阵、新增 `tests/constraint_enforcement_test.rs`（NOT NULL/UNIQUE e2e + 恢复两态）、`tests/cli_test.rs` 错误面与 dump/restore 往返
- 文档：README 约束语义与存量兼容边界说明（英文/中文同步）
- 编号记录：不新增 D/K；登记映射 MS23-T01/T02/T03；改进项无新增（约束缺口源自 Explorer 调查，未立 Ixx）
- 里程碑归属：MS23（tasks.md 权威）

## Rollback

按 Iteration 粒度回退：Iteration 000（NOT NULL + 诚实化）回退为恢复 `_ => {}` 忽略分支与 TableMeta 去标志；Iteration 001（UNIQUE）回退为移除唯一索引接线与 catalog 追加字段读取消费（旧库含追加字段的表行按兼容读降级为无唯一索引，天然向前兼容）。两 Iteration 均不改 WAL 记录格式与页格式，数据库文件可被回退版本打开（catalog 追加字段被旧反序列化器忽略）。
