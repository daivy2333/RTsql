# MS23 数据完整性约束执行面 — Design

> 调查基线：master @ d8165fc（2026-09-24，1101 tests pass 结论沿用，约束相关表面本轮零变化）。
> 用户裁定记录：单 change 双 Iteration；UNIQUE 仅 INT 列强制、非 INT 点名拒绝；表级单列 UNIQUE 映射、组合拒绝；存量非 INT UNIQUE 库 dump→restore 破坏接受并文档化。

## D1 TableMeta 携带约束标志

`TableMeta`（`src/storage/data/table_manager.rs:50`）新增 `pub not_null: Vec<bool>`（与 `columns` 按列序对齐的平行向量）。三个产品构造点同步接线：`create_table_with_constraints`（:280，入参已含 not_null）、`open_or_init`（:187，从 `CatalogColumnRow.not_null` 读回）、`replace_index_manager`（:361，继承旧值）。`Iteration 001` 再加 `unique_indexes: Vec<(usize, Arc<IndexManager>)>`（列序 → 该列专属唯一索引；PK 列永不入列）。

**推翻旧决策**：`table_manager.rs:274` 注释「TableMeta 只承载 (name, type)，约束是 catalog 元数据不进入内存 schema」自 MS10-T05 起成立，本 change 以强制执行需求推翻它（理由：执行器需要 O(1) 读取标志；经 catalog 回读每写一次不可接受）。

**替代方案（否决）**：把 `columns` 改为富结构体——消费面遍布全部执行器与扫描路径，爆炸半径大且无行为收益；每写经 catalog 查标志——I/O 与锁开销，且 catalog 读接口按表不按行。

## D2 NOT NULL 校验位置与错误面

- INSERT（`src/executor/insert.rs:100`）：日期 coerce（:112，既有）之后、MS16 键位类型预检（:124）之前，逐列 `not_null[i] && value.is_null()` → 新 `StorageError::NullConstraintViolation { column }`。此位置任何失败均零副作用（未触数据页/WAL/索引）。
- UPDATE（`src/executor/update.rs:76`）：Step 1 索引定位（KeyNotFound 优先，与 MS16 键位校验同区）之后、Step 6 写入之前，SET 目标列 `not_null` 且新值为 NULL → 同错误。planner 仅支持单列 SET（`ddl_dml.rs:554`），无需考虑多赋值。
- 错误变体跟随 `KeyTypeMismatch` 模式（thiserror 点名列名，`src/storage/error.rs:39`）；无专用 CLI 臂——经既有 Sql 失败路径渲染，exit 3 同面（仅 DatabaseLocked/InvalidKey 有专用臂，`src/cli/mod.rs:294`）。
- 会话语义复用既有机制：显式事务内语句失败 → `rollback_session` 自动回滚（`tx_statement_test.rs` 既有契约），本 change 不新增事务分支。

## D3 约束诚实化拒绝面

- 列级（`extract_column_constraints`，`ddl_dml.rs:369`）：`ColumnOption::Check` / `ForeignKey` / `DialectSpecific` → 新 `PlanError::UnsupportedConstraint(&'static str)` 点名特性；`Null` / `Comment` 维持忽略（无语义期望）。
- 表级（`build_create_table`，`ddl_dml.rs:472`）：新增对 `constraints` 的遍历（现仅 `extract_primary_key` 消费）——`TableConstraint::Check` / `ForeignKey` / Index 类变体 → 同错误。表级 `Unique` 留给 D6（Iteration 001）。
- 错误经建表计划期路径返回，表不创建（`CreateTableExecutor` 不会被触达）。

## D4 catalog 表行向后兼容追加

`serialize_catalog_row`（`src/storage/catalog.rs:560`）在既有固定布局尾部追加 `u32 unique_count | unique_count × u32 root`；`deserialize_catalog_row`（:578）读完既有字段后按剩余长度可选读取（不足 → 空）。依据：现反序列化器不拒绝尾随字节（顺序读、无总长校验），checkpoint 位点 24B 兼容读为先例（`wal/mod.rs read_site_file` 三分支）。`update_table_root` 的 deserialize→mutate→serialize 往返（:264-275）天然保追加字段。新增 `Catalog::update_unique_index_root(table, ordinal, root)`（同 `update_field_in_chain` 模式）。

**替代方案（否决）**：独立 `__indexes` 系统表——MS21 用户级 CREATE INDEX 的正域，本 change 引入属过度建设；行内嵌版本号字段——现格式无版本位，追加读已足够。

## D5 唯一索引 root 同步泛化

`IndexManager` catalog 上下文从 `Option<(Arc<Catalog>, String)>`（`src/storage/btree/index_manager.rs:53`）泛化为携带槽位描述（PK 表根 or 第 N 唯一根），`sync_root_to_catalog`（:95）按槽位分派 `update_table_root` / `update_unique_index_root`。`with_catalog_context`/`set_catalog_context` 签名相应调整；`TableManager::attach_index_catalog_contexts`（:146）同步为每表唯一索引逐个 attach。恢复期上下文仍不 attach（R-T0b-R5 载荷点不变：replay 后由 `attach_index_catalog_contexts` 统一接线）。

## D6 UNIQUE DDL 策略面（Iteration 001）

在 `build_create_table` 列定义后新增唯一策略处理：

1. 列级 `ColumnConstraint::Unique`：声明类型非 `ColumnType::Int` → `UnsupportedConstraint("UNIQUE (INT columns only)")` 同族点名错误。
2. 表级 `TableConstraint::Unique { is_primary: false, columns }`：单列 → 映射为该列 unique 标志（等价列级声明；列名经既有 lowercase 归一）；多列 → 点名拒绝组合 UNIQUE。
3. PK 列声明的 UNIQUE（`UNIQUE` 与 `PRIMARY KEY` 同列）：PK 索引已是唯一索引，不重复建——planner 对 PK 列不产出 Unique 约束（`ddl_dml.rs:381` `is_primary: false` 守卫既有），表级 `PRIMARY KEY` 同理，无需新分支。

## D7 写路径唯一性检查与条目维护

INSERT（`insert.rs`）每行顺序：日期 coerce → NOT NULL（D2）→ Int 键位类型 → PK 重复预检（既有 :140）→ **逐唯一列预检**（值 NULL 跳过；`to_key()` 仅 Int 产键所以 INT 列必有键）→ serialize → 数据页写入 → WAL → record_version → PK 索引插入（既有）→ **逐唯一列条目插入**。预检在任何写入前（零副作用拒绝）；条目在数据落位后插入（镜像 PK 顺序，数据写失败不留索引条目）。

UPDATE（`update.rs`）Step 7 扩展四分支（SET 目标列为某唯一列 `u`，其余唯一列一律「同值随行」`update(old_key, new_row_id)`）：

1. SET 非 `u`：`u` 值不变，`u` 条目随行 `update(u_key, new_row_id)`；
2. SET `u` 同值：同上；
3. SET `u` 新值（非 NULL）：先 `search(新键)` 碰撞预检（命中即 `DuplicateKey`，任何写入前）→ 数据写入后 `delete(旧键)` + `insert(新键)`（镜像 PK rekey 顺序，`update.rs:218-224` 先例）；
4. SET `u` 为 NULL：`delete(旧键)`（NULL 不入索引，镜像 I037 PK 分支）。

DELETE（`delete.rs:91`）：删 PK 条目处同步逐唯一列 `delete(唯一键)`（NULL 值行无条目，跳过）。

## D8 回滚唯一索引修复

`TransactionManager::abort_cleanup_versions`（`src/transaction/manager.rs:271`）既有 PK 修复模式（`find_key_by_row_id` → 有前驱 `update` 指向前驱 / 无前驱 `delete`）扩展到每表全部唯一索引：逐唯一索引执行同型修复。依据：不修复则回滚 INSERT 残留唯一条目，后续同值插入假阳性 DuplicateKey，违反 R3「回滚后可重插」。已知既有边界（未提交删除回滚后 PK 点查时序边界，`mvcc-tombstone-visibility` spec 已知边界段）由唯一索引同型继承，本 change 不修（预存边界，范围外）。

## D9 恢复重建唯一索引

`rebuild_pk_indexes`（`src/wal/recovery.rs:837`）扩展：每表除 PK 新索引外，为每个唯一列建新 `IndexManager`；既有链尾回溯循环（:917-952，首个「已提交 ∧ 非墓碑」版本）逐唯一列反序列化 tuple 提取列值（`deserialize_tuple` + 列序取值；NULL 跳过），唯一值 `Vec<u8>` 键 → `HashMap<键, RowId>`，跨链重复 → `RedoFailed` 显式点名表列（镜像 "duplicate PK across chains" :937）。写完条目后：PK 与唯一索引一并 `replace`（`TableManager` 新增或扩展 swap 方法）→ catalog 根写回（PK `update_table_root` 既有 + 逐唯一 `update_unique_index_root`）→ 旧树释放（洞容忍，:984 先例）。`redo_count == 0` 路径零变化（唯一索引自 catalog 根加载，D4）。

## D10 drop_table 与检查点

`drop_table`（`table_manager.rs:391`）在 PK `collect_all_pages` 释放处逐唯一索引 `collect_all_pages` 同型 best-effort 释放。checkpoint 无参与（唯一索引页与 PK 索引页同为普通页，走既有 eviction/flush）。

## D11 测试布点

- 计划期拒绝矩阵 → `tests/planner_test.rs`（D3/D6，build_plan 层）。
- NOT NULL / UNIQUE 执行与两态 e2e → 新增 `tests/constraint_enforcement_test.rs`（lib API 直构 + Database 打开面；恢复两态用既有恢复测试模式——`mvcc_tombstone_visibility_test.rs` / `isolation_level_test.rs` 先例）。
- CLI 错误面（exit 3 + 点名文本）与 dump/restore 往返、会话回滚重插 → `tests/cli_test.rs` 增量。
- catalog 兼容读 / root 写回单测 → `src/storage/catalog.rs` `#[cfg(test)]`（`create_table_with_constraints_persists_flags` 同区）。
- 既有用例预期零破坏（无现有用例向 NOT NULL 列插 NULL 或向 UNIQUE 列插重复值——已核对三处 DDL 声明点均为 schema 渲染断言）；如 Act 发现例外，按新契约校准并在 Response 记录。

## Acceptance 映射

R1（NOT NULL）→ D1/D2 → Iteration 000 任务 1.3-1.6；R2（诚实化）→ D3 → 1.1-1.2；R3（UNIQUE 强制）→ D5/D6/D7/D8 → Iteration 001 任务 2.2-2.7；R4（两态一致）→ D4/D5/D8/D9/D10 → 2.1/2.5/2.8/2.9；R5（零回归）→ 全部设计的不变量面 → 1.7/2.9。逐条 RTM 见 tasks.md。
