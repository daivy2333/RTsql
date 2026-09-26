# Iteration 001 / Cycle 000: UNIQUE 强制端到端

## Plan Context

- Status: ready
- Iteration: 001-unique-enforcement
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 2.7, 2.8, 2.9
- Depends on: Iteration 000（accepted——`TableMeta.not_null` 标志通道、`StorageError::NullConstraintViolation`/`PlanError::UnsupportedConstraint` 错误面先例、`build_create_table` 表级约束遍历臂模式、`constraint_enforcement_test.rs` 测试文件骨架）
- Stable baseline: INT 列 UNIQUE 经专属非 PK 唯一索引在 INSERT/UPDATE/DELETE/回滚全路径强制且 NULL 豁免；干净重开与崩溃恢复两态一致，跨链重复显式报错；catalog 旧格式兼容；dump/restore 往返保持；PK 列声明的 UNIQUE 不建第二索引；全量零回归
- Verification boundary: constraint_enforcement_test（UNIQUE 全矩阵 + 恢复两态）+ planner_test DDL 策略矩阵 + catalog 单测 + cli_test 往返 + `cargo test` 全绿
- Diagnostic boundary: `src/storage/{catalog.rs,btree/index_manager.rs,data/table_manager.rs}`、`src/executor/{insert,update,delete}.rs`、`src/transaction/manager.rs`、`src/wal/recovery.rs` 与本 Cycle
- Deferred tasks: None（2.9 为本 Iteration 收口任务）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: MS23 全部 requirement 的 Iteration 001 部分（R3 INT 列 UNIQUE 强制、R4 UNIQUE 形态与类型面、R5 两态一致、R6 零回归中 2.9 承担的收口面）；用户四项裁定（tasks.md 头部记录：单 change 双 Iteration、UNIQUE 仅 INT 列、表级单列映射组合拒绝、存量非 INT UNIQUE dump→restore 破坏接受并文档化）
- Excluded scope: 用户级 CREATE/DROP INDEX（MS21-T01）；String 等 B-Tree 键编码扩展（I024 域）；FOREIGN KEY 强制；组合 UNIQUE 语义；约束随 ALTER 演进；DEFAULT 应用（MS24-T01）；CLI 管理面（MS19）；Iteration 000 Minor finding 的代码面处置（仅随 2.9 文档化）

**Objective**

建表声明的 INT 列 UNIQUE 在写入路径被专属唯一索引强制（INSERT 预检零副作用拒绝、UPDATE 四分支维护、DELETE 移除、回滚修复），NULL 豁免；索引在干净重开（catalog 持久化根）与崩溃恢复（数据页重建、跨链重复显式报错）两态下一致；非 INT 列 UNIQUE、组合 UNIQUE 建表点名拒绝，表级单列 UNIQUE(col) 等价映射列标志；PK 列声明的 UNIQUE 消费为 PK 既有唯一性、不建第二索引；全量既有语义零回归。

**Background**

MS23 数据完整性约束执行面第二期。Iteration 000（accepted）已收 NOT NULL 强制与 CHECK/FK/方言项诚实化。本期收 UNIQUE——唯一「DDL 接受、运行期零消费且需要存储结构演进」的约束：catalog 已持久化 per-column unique 标志（`CatalogColumnRow.unique`），但运行期无索引、无强制、恢复不重建。设计决策 D4-D10（design.md）：catalog 行尾随追加唯一根页、索引上下文槽位泛化、DDL 策略面、写路径四分支、abort 同型修复、恢复重建扩展、drop 释放。

**Investigation Facts**

- Current Baseline: 工作树未提交状态（master @ d8165fc + Iteration 000 七文件改动 + 既有文档批次）。全量 `cargo test` 1119 passed / 0 failed / 2 ignored（Iteration 000 Act Response 产生，注释-only 修复后受影响面 lib 298 + constraint_enforcement 8 复跑绿，Plan Review accepted 采信；此后代码面零变化）。Iteration 000 已落地：`TableMeta.not_null: Vec<bool>`（`table_manager.rs:53`，三构造点 :189/:286/:372 接线）、INSERT coerce 后逐列 NOT NULL 臂（`insert.rs:119-127`）、UPDATE Step 1 后 SET 目标列 NOT NULL 臂（`update.rs:89-106`）、`build_create_table` 表级约束拒绝遍历（`ddl_dml.rs:517-534`，`Unique` 两态落 `_ => {}` 不触碰）。
- Current-State Evidence:
  - **catalog 行格式**（`src/storage/catalog.rs`）：`CatalogRow` 字段 table_name/data_page_head/index_root_page_id/pk_index/pk_column/column_count/data_page_tail（:55-62 区）；`serialize_catalog_row`（:560）固定布局 + `debug_assert_eq!(buf.len(), total)`；`deserialize_catalog_row`（:578）顺序读、只做下界检查、**不拒绝尾随字节**——尾部追加 `u32 unique_count | N × u32 roots` 后旧反序列化器忽略之，向后兼容成立（checkpoint 位点 24B 兼容读同型先例）。`update_table_root`（:262）经 `update_field_in_chain`（:474，deserialize→mutate→serialize 闭包 + 页满 guard）往返天然保追加字段；新增 `update_unique_index_root` 仿此模式。`CatalogColumnRow.unique: bool` 已持久化（:66-73），`scan_columns`（:218）返回它。`matches_catalog_row_name`（:648 前缀匹配）不受追加影响。
  - **IndexManager**（`src/storage/btree/index_manager.rs`）：`catalog_ctx: Mutex<Option<(Arc<Catalog>, String)>>`（:30）；`from_root`（:61，无 catalog 构造）；`with_catalog_context`（:79 builder）/`set_catalog_context`（:88 post-construction）；`sync_root_to_catalog`（:95）仅调 `update_table_root`；`search`（:106）→ `Option<RowId>`；`insert`（:223）/`delete`（:245）/`update`（:306）；`find_key_by_row_id`（:325）＝进程内 `row_to_key` 反向映射（由本实例 insert 维护，from_root/重开实例为空——恢复后无未提交事务故 abort 修复无对象，PK 同型性质）；`collect_all_pages`（:335）/`collect_all_pages_tolerant`。update.rs Step 7 注释明示 delete-先于-insert 的次序约束（delete() 经 search 解析 row_to_key，先 insert 会使后续 delete 误清新行映射）。
  - **TableManager**（`src/storage/data/table_manager.rs`）：`TableMeta`（:50，含 Iteration 000 的 not_null）；`attach_index_catalog_contexts`（:149，现仅 PK 索引）；`open_or_init`（:162，scan_tables→scan_columns→TableMeta，not_null 自 `CatalogColumnRow` 读回——unique 标志同区可得）；`create_table_with_constraints`（:230 区，入参四元组含 unique 标志，`insert_table` 落 catalog 行）；`replace_index_manager`（:358，克隆 old 其余字段换 PK 索引）；`drop_table`（:400，PK `collect_all_pages` + 数据页 best-effort free）。
  - **写路径**：INSERT（`insert.rs:100` 区）序——coerce→NOT NULL→Int 键位类型→PK 重复预检（`index_manager.search`）→serialize→数据页写→页可见性→WAL→record_version→PK 索引插入；行值 `coerced` 长度 ≤ schema 列数（zip 截断），unique 列值按列序索引可得。UPDATE（`update.rs`）Step 1 索引定位→NOT NULL/MS16 校验区→coerce→读旧 tuple（`old_tuple_bytes` 在场）→Step 6 写入→Step 7 PK 三分支（:208-244：非键列 `update(old_key,new_rid)`；键位置 NULL `delete`；同值 `update`；rekey `delete(old)+insert(new)`）。DELETE（`delete.rs:44-120`）——PK `search` 得 rid→**不读行元组**→墓碑 slot 写入→PK `delete`→record_version→WAL；唯一列键值推导需在读墓碑前从 rid 处 slot 数据 `deserialize_tuple`（`page_format/tuple.rs`）提取。
  - **abort 修复**（`src/transaction/manager.rs:271` `abort_cleanup_versions`）：逐表逐 rid——读 header→`find_key_by_row_id`→有前驱 `update(key,prev)`/无前驱 `delete(key)`→`mark_aborted` 中性化；`tables: &HashMap<String, Arc<TableMeta>>` 在场，`meta.unique_indexes` 可达。
  - **恢复重建**（`src/wal/recovery.rs:837` `rebuild_pk_indexes`）五步形状：新索引实例（spawn_blocking `IndexManager::new`）→ 数据页链扫描（`extract_pk_key` 提取 PK 键，slot→`HashMap<RowId,(header,key)>`）→ 链尾回溯（不被指向 slot 起新→旧，首个「已提交 ∧ 非墓碑」版本建条目；跨链重复 PK → `RedoFailed` 点名表，:937）→ 批量 insert→`replace_index_manager` 换入→`update_table_root` 写回→旧树洞容忍释放。`redo_count == 0` 路径不经此函数（唯一索引自 catalog 根加载即 D4 消费）。
  - **键编码**：`Value::to_key`（`src/executor/value.rs:92`）仅 `Value::Int` 产 `Some(Key)`——INT 唯一列非 NULL 值必有键，NULL/其他类型 None；DDL 策略面保证 unique 标志仅落 INT 非 PK 列后，写路径 None 即 NULL。
  - **planner 与渲染**：`extract_column_constraints`（`ddl_dml.rs:369`）`Unique{is_primary:false}` → `ColumnConstraint::Unique` **无 PK 列判定**（:381 守卫仅排除 PRIMARY KEY 选项本身）；`extract_primary_key`（:430 区）返回 PK 列名（lowercase 归一先例 :455）；`build_create_table`（:472 区）持 AST `columns` + `convert_data_type` 类型 + Iteration 000 表级拒绝遍历，产 `CreateTableNode{columns: Vec<ColumnDef>}`；`ColumnDef::to_schema_column`（`src/executor/plan.rs`）折叠 `ColumnConstraint::Unique`→`ColumnSchema.unique`→catalog 持久化；`create_table_sql`（`src/cli/lifecycle.rs:576`）自 catalog 渲染 `UNIQUE`（:592-594）——表级单列映射为列标志后 dump/schema 渲染自动正确，lifecycle 产品代码零修改。lifecycle 纯函数测试 `ddl_generator_quotes_and_orders_constraints` 断言 `BOOL PRIMARY KEY NOT NULL UNIQUE` 渲染——即 dump 可产出「PK 列 + UNIQUE」DDL，restore 面须接受该形态（见下条设计澄清）。
  - **设计澄清（Plan 裁定，补 D6 第 3 点的不精确论据）**：`id INT PRIMARY KEY UNIQUE` / dump 产出的 `"pk" BOOL PRIMARY KEY NOT NULL UNIQUE`——`extract_column_constraints` 照样对 PK 列产出 `ColumnConstraint::Unique`（`is_primary:false` 守卫不检查该列是否 PK）。若不处理，2.3 会对 PK 列建第二个唯一索引（冗余且 rekey 双维护）。裁定：**声明在 PK 列上的 UNIQUE 消费为 PK 索引既有唯一性——不建第二唯一索引、不触发非 INT 拒绝、restore 自家 dump 恒可达**；非 PK 列 UNIQUE 非 INT 才点名拒绝。实现位置（planner 对 PK 列剥离 Unique 约束，或承载/执行面对 pk 列跳过）非实质，留 Act；`TableManager` 承载面跳过 pk 列为推荐位（同时覆盖直呼 `create_table_with_constraints` 的测试路径）。
- Code and Critical Path: DDL 文本→`build_create_table`（策略面 2.4：类型门 + 表级单列映射 + PK 列消费裁定）→`CreateTableNode`→`CreateTableExecutor`→`to_schema_column`（unique 折叠）→`create_table_with_constraints`（2.3：分配唯一索引 + catalog 行携带 roots）→catalog 持久化（2.1 新布局）。打开：`open_or_init`（2.3：自 catalog 行 roots `from_root` 重建唯一索引；INT 非 PK 门）→`attach_index_catalog_contexts`（2.2：逐唯一索引 attach）。运行期写：INSERT 预检+条目（2.5）、UPDATE 四分支（2.6）、DELETE 条目移除 + abort 同型修复（2.7）——全部经 `TableMeta.unique_indexes`。崩溃恢复：`rebuild_pk_indexes` 扩展（2.8）重建唯一索引并写回根。根运行期变更：`sync_root_to_catalog` 槽位分派（2.2）。drop：`drop_table` 逐唯一索引释放（2.3）。

**Implementation Guidance**

按任务序执行：2.1（catalog 格式，纯存储层先行）→2.2（root 同步泛化，依赖 2.1 的 `update_unique_index_root`）→2.3（TableMeta 承载与生命周期，依赖 2.1/2.2）→2.4（DDL 策略面，可独立，但映射产物的消费依赖 2.3）→2.5/2.6/2.7（写路径三执行器 + abort，依赖 2.3）→2.8（恢复重建，依赖 2.1/2.3）→2.9（收口 + 文档）。2.4 与 2.1-2.3 无代码依赖可并行，但其 e2e 见证（表级单列映射后强制生效）依赖 2.3。设计细节以 design.md D4-D10 为准（本文件与 design 冲突时以 Task Contract 为准；D6 第 3 点按上方设计澄清执行）。

**Behavioral Change**

- 当前：INT 列 UNIQUE 声明建表成功、catalog 持久化 unique 标志、dump 渲染 UNIQUE，但重复值照常落库、重开/恢复后无索引、DELETE/UPDATE 不维护任何唯一条目；非 INT 列与组合 UNIQUE 同样静默接受。
- 目标：INT 非 PK 列 UNIQUE 经专属唯一索引全路径强制（`DuplicateKey` 同型拒绝、NULL 豁免、回滚可重插）；非 INT 列 UNIQUE 与表级组合 UNIQUE 建表点名拒绝；表级单列 `UNIQUE(col)` 等价列级；PK 列 UNIQUE 不建第二索引；干净重开自 catalog 根、崩溃恢复自数据页重建两态一致；catalog 旧行兼容打开（行为＝无唯一索引；含旧库非 INT unique 标志列——跳过建索引，运行期不强制）。
- 接口/状态/错误语义：`CatalogRow` 尾随追加唯一根页字段（向后兼容读）；`IndexManager` catalog 上下文槽位泛化（`with_catalog_context`/`set_catalog_context` 签名调整）；`TableMeta` 增 `unique_indexes: Vec<(usize, Arc<IndexManager>)>`；复用既有 `StorageError::DuplicateKey`；恢复跨链重复 → 既有 `WalError::RedoFailed` 点名表列。WAL 记录格式、页格式、checkpoint 位点零变化。

**Task Contracts**

### 2.1: catalog 表行尾随追加唯一索引根页（向后兼容）

- Requirement/Scenario: R5 S4（旧格式兼容打开的格式前提）、R5 S1/S2（两态的持久化根载体）
- Depends on: None
- Targets: `src/storage/catalog.rs::serialize_catalog_row`、`::deserialize_catalog_row`、`CatalogRow`、`Catalog::update_unique_index_root`（新增）
- Current behavior: 固定布局 `u16 name_len | name | u32 head | u32 idx_root | u32 pk_index | u16 pk_len | pk | u32 column_count | u32 tail`；反序列化忽略尾随字节；无唯一根页字段
- Required behavior: serialize 尾部追加 `u32 unique_count | unique_count × u32 roots`；deserialize 读完既有字段后按剩余长度可选读（不足 → 空 Vec）；`CatalogRow` 增唯一根页承载字段（形态非实质，如 `unique_roots: Vec<u32>`）；新增 `Catalog::update_unique_index_root(&self, table: &str, ordinal: usize, root: u32)`——仿 `update_table_root` 的 `update_field_in_chain` 闭包模式，只改 ordinal 位、保其余字段
- Required changes: 序列化/反序列化对称扩展 + 新 catalog 方法
- Preserve: 旧布局字节被新反序列化器读为空唯一根（兼容）；`matches_catalog_row_name` 前缀匹配不受影响；`update_table_root` 往返保追加字段（deserialize→mutate→serialize 天然成立）；既有 catalog 单测零回归
- Forbidden: 不引入版本号字段；不改既有字段布局与顺序；不动 `__columns` 行格式
- Test witness: `src/storage/catalog.rs` `#[cfg(test)]` 区（`create_table_with_constraints_persists_flags` 同区）新增单测——(a) 新格式往返（含 ≥2 根）；(b) 手工构造旧布局字节 → deserialize → 唯一根为空；(c) `update_table_root` 后追加字段保留；(d) `update_unique_index_root` 只改目标 ordinal。先观察 RED（字段/方法不存在）
- GREEN condition: 单测绿 + `cargo test --lib` 零回归
- Verification: `cargo test --lib`；退出码 0
- Stop when: 发现 `deserialize_catalog_row` 存在总长校验会拒绝尾随字节（与调查矛盾——返回 Plan）

### 2.2: IndexManager catalog 上下文槽位泛化

- Requirement/Scenario: R5 S1/S2（唯一索引根的运行期变更持久化通道）
- Depends on: 2.1（`update_unique_index_root`）
- Targets: `src/storage/btree/index_manager.rs::IndexManager`（`catalog_ctx` 字段、`with_catalog_context`、`set_catalog_context`、`sync_root_to_catalog`）、`src/storage/data/table_manager.rs::attach_index_catalog_contexts`
- Current behavior: `catalog_ctx: Mutex<Option<(Arc<Catalog>, String)>>`；`sync_root_to_catalog` 一律 `update_table_root`；`attach_index_catalog_contexts` 仅 attach PK 索引
- Required behavior: 上下文携带槽位描述（PK 表根 / 第 N 唯一根；形态非实质，如枚举 `CatalogRootSlot`），`sync_root_to_catalog` 按槽位分派 `update_table_root` / `update_unique_index_root(table, ordinal, root)`；`with_catalog_context`/`set_catalog_context` 签名相应调整；`attach_index_catalog_contexts` 逐表对 `unique_indexes` 逐个 attach
- Required changes: 上下文类型 + 分派 + attach 扩展
- Preserve: 恢复期不 attach 语义不变（replay 时 root 变更不落 catalog，重放后由 `attach_index_catalog_contexts` 统一接线——R-T0b-R5）；无上下文构造（测试）no-op 不变；PK 索引行为逐字节不变
- Forbidden: 不改 B-Tree 分裂/收缩逻辑；不改 root 的 AtomicU64 读路径
- Test witness: 既有 root 同步测试保持绿（root 变更落 catalog 行为不变）；唯一索引根写回的行为见证随 2.3/2.5 的 e2e 落地（本任务以编译 + 既有套件零回归为界）
- GREEN condition: `cargo test --lib` 零回归
- Verification: `cargo test --lib`；退出码 0
- Stop when: 槽位泛化需要改动 `sync_root_to_catalog` 以外的调用方契约（返回 Plan）

### 2.3: TableMeta 唯一索引承载与生命周期

- Requirement/Scenario: R5 S1（干净重开自根恢复）、S5（drop_table 释放）、R3 全部（运行期强制的前提承载）
- Depends on: 2.1、2.2
- Targets: `src/storage/data/table_manager.rs::TableMeta`、`::create_table_with_constraints`、`::open_or_init`、`::replace_index_manager`、`::drop_table`、`::attach_index_catalog_contexts`
- Current behavior: `TableMeta` 无唯一索引承载；create/open/replace/drop 均 PK 索引单树
- Required behavior: `TableMeta` 增 `unique_indexes: Vec<(usize, Arc<IndexManager>)>`（列序号 → 该列专属唯一索引；PK 列永不入列）。create：对「INT ∧ unique ∧ 非 PK 列」各建 `IndexManager::new` 并 attach 上下文（2.2 槽位），catalog 行携带按列升序的根页；open_or_init：自 catalog 行唯一根 + `CatalogColumnRow.unique` 标志重建（`from_root`），仅「INT ∧ unique ∧ 非 PK」列——**非 INT unique 标志列（旧库可达）静默跳过**（行为＝无唯一索引、运行期不强制，与 R5-S4 兼容边界一致）；replace_index_manager：PK 换新、`unique_indexes` 继承（并预留恢复期整表 swap 的扩展位，形态非实质）；drop_table：逐唯一索引 `collect_all_pages` best-effort 释放（对齐 PK 的 warn+放弃先例）
- Required changes: 结构体字段 + create/open/replace/drop 四接线 + attach 扩展
- Preserve: `create_table` 委托壳行为不变（flags false → 无唯一索引）；`TableMeta` 其余字段与既有消费点零变化；drop 的 best-effort 语义（失败 warn 不阻塞）不变；无 unique 列的表 `unique_indexes` 恒空、全路径行为逐字节不变
- Forbidden: 不在本任务加写路径校验（2.5-2.7）；不对非 INT unique 列报错（兼容边界，错误面属 2.4 的新建表路径）
- Test witness: table_manager/catalog 测试区新增——(a) create 含 INT UNIQUE 列 → `unique_indexes` 非空且列序号正确；(b) 新 TableManager open_or_init 等价路径重建（根页一致）；(c) 手工插入 catalog 行含非 INT unique 标志（或直呼 `create_table_with_constraints` 构造）→ open 后该列不在 `unique_indexes`；(d) drop 含唯一索引表后 free 无 warn 报错、同进程新建表可写（完整 e2e 见证在 2.9）。先观察 RED（字段不存在编译失败即见证）
- GREEN condition: 单测绿 + `cargo test --lib` 零回归
- Verification: `cargo test --lib`；退出码 0
- Stop when: `open_or_init` 处 catalog 行根页数与 unique 列数不一致（数据异常——返回 Plan 定策略）

### 2.4: UNIQUE DDL 策略面（INT 门 + 表级单列映射 + 组合拒绝 + PK 列消费裁定）

- Requirement/Scenario: R4 S1（非 INT 拒绝）、S2（表级单列等价 + schema 渲染）、S3（组合拒绝）、R3 S1-S8 的 DDL 前提
- Depends on: None（e2e 见证依赖 2.3）
- Targets: `src/parser/planner/ddl_dml.rs::build_create_table`
- Current behavior: 列级 `Unique{is_primary:false}` 无类型门（非 INT 照样建表）；表级 `Unique{is_primary:false}` 落 Iteration 000 遍历的 `_ => {}` 静默忽略
- Required behavior: 在 `extract_primary_key` 之后、`CreateTableNode` 构造之前新增唯一策略处理——(a) 列级：ColumnDef 带 `ColumnConstraint::Unique`（或 AST `Unique{is_primary:false}` 选项）且该列**非 PK 列**且 `convert_data_type` 结果非 `ColumnType::Int` → `PlanError::UnsupportedConstraint` 点名「UNIQUE 仅支持 INT 列」；(b) PK 列声明的 UNIQUE → 消费为 PK 既有唯一性：不拒绝、最终 `ColumnSchema` 不携带会引发第二索引的 Unique 承载（实现位置非实质——推荐 `TableManager` 承载面跳过 pk 列〔2.3 已述〕+ planner 不做剥离，两者其一即可达成「不建第二索引」；若两处都做需保证幂等）；(c) 表级 `Unique{is_primary:false, columns}`：len > 1 → 点名拒绝组合 UNIQUE；len == 1 → 目标列按 (a)/(b) 同规则（PK 列忽略、非 INT 拒绝、INT 映射为该列 `ColumnConstraint::Unique` 推入 ColumnDef——`to_schema_column` 折叠后 catalog 持久化与 dump 渲染自动正确）
- Required changes: `build_create_table` 唯一策略臂
- Preserve: Iteration 000 全部拒绝面（CHECK/FK/方言/Index/FulltextOrSpatial）与 `Null`/`Comment` 忽略不变；表级 PK（`Unique{is_primary:true}`）经 `extract_primary_key` 消费不变；既有 planner_test 43 用例零回归；列名归一化对齐 `extract_primary_key` 的 lowercase 先例
- Forbidden: 不实现用户级 CREATE/DROP INDEX；不实现 String 等类型键编码；不动 `extract_column_constraints` 的 Iteration 000 拒绝臂
- Test witness: `tests/planner_test.rs` 矩阵——`name STRING UNIQUE`/`f FLOAT UNIQUE`/`b BOOL UNIQUE`/`d DATE UNIQUE`/`ts TIMESTAMP UNIQUE` ×5 拒绝（文本点名仅 INT）；表级 `UNIQUE(code)` INT 单列建表成功且 plan 中该列携带 Unique；表级 `UNIQUE(a,b)` 组合拒绝点名；`id INT PRIMARY KEY UNIQUE` 建表成功不拒绝；表级 PK 既有用例保持。先观察 RED（当前全部静默成功）
- GREEN condition: 新矩阵绿 + planner_test 全绿
- Verification: `cargo test --test planner_test`；退出码 0
- Stop when: sqlparser 0.44 `TableConstraint::Unique` 的 columns 形态与预期不符（按实际枚举调整，记录偏差后继续）；或发现表级单列映射与列级约束叠加产生重复 `Unique` 约束的形态（去重策略返回 Plan）

### 2.5: INSERT 唯一强制（预检零副作用 + 落位后条目）

- Requirement/Scenario: R3 S1（重复拒绝零副作用）、S2（不同值成功）、S5（多 NULL）、S8（多唯一列独立）
- Depends on: 2.3（承载）、2.4（INT 门保证键必可得）
- Targets: `src/executor/insert.rs::InsertExecutor::next`
- Current behavior: unique 标志零消费——重复值照常落库
- Required behavior: 每行在 PK 重复预检（现 :140 区）之后、serialize 之前，逐 `(col_idx, uindex)`：`row_values[col_idx]` 为 NULL → 跳过；`to_key()` → `uindex.search(key)` 命中 `Some` → `Err(StorageError::DuplicateKey)`（既有变体，预检阶段未触任何写入）；数据落位与 PK 索引插入（现 :194-199）之后，逐非 NULL 唯一列 `uindex.insert(key, row_id)`（条目在数据落位后插入——镜像 PK 顺序，数据写失败不留索引条目）
- Required changes: insert.rs 预检臂 + 条目插入臂
- Preserve: coerce→NOT NULL→键位类型→PK 重复的既有顺序与错误文案；多行 VALUES 逐行语义；无 unique 列的表路径逐字节不变
- Forbidden: 不改 PK 预检/插入位置；不加唯一列类型二次校验（2.4 已保证）
- Test witness: `tests/constraint_enforcement_test.rs` 追加——(a) 重复值 DuplicateKey 拒绝 + 零副作用（行数/索引不变、随后可插不同值）；(b) 不同值成功；(c) 多行 NULL 全成功；(d) 两 INT UNIQUE 列各自独立拒绝。先观察 RED（当前重复值落库成功）
- GREEN condition: 四组用例绿
- Verification: `cargo test --test constraint_enforcement_test`；退出码 0
- Stop when: 无（契约面清晰）

### 2.6: UPDATE 唯一维护四分支

- Requirement/Scenario: R3 S3（改值碰撞拒绝）、S4（非唯一列更新随行）
- Depends on: 2.3、2.5（条目语义）
- Targets: `src/executor/update.rs::UpdateExecutor::next`
- Current behavior: 唯一条目零维护——更新后唯一索引与数据页失配
- Required behavior: 对每 `(col_idx, uindex)`（旧值自 `old_tuple_bytes` 反序列化取列值；SET 目标列为该列时新值取 `new_value`，否则同旧值）：(1) SET 目标非该列或 SET 同值 → 数据写入后（Step 7 区）`uindex.update(u_key, new_row_id)`（NULL 跳过）；(2) SET 该列新值非 NULL → 任何写入前 `uindex.search(new_key)` 碰撞预检（命中 `DuplicateKey`，原行保持）→ 数据写入后 `uindex.delete(old_key)` + `uindex.insert(new_key, new_row_id)`（**delete 先于 insert**——delete 经 search 解析 row_to_key 反向映射，次序约束与 PK rekey 同型）；(3) SET 该列 NULL → 数据写入后 `uindex.delete(old_key)`（NULL 不入索引，镜像 I037 PK 分支）。碰撞预检位置与 NOT NULL/MS16 校验同层（任何写入前）
- Required changes: update.rs 唯一维护区（碰撞预检 + Step 7 扩展）
- Preserve: Step 7 PK 三分支行为与顺序注释语义；KeyNotFound 优先；NOT NULL/键位校验既有顺序；单列 SET 限制（planner 层不变）
- Forbidden: 不改 PK 分支；不实现多列 SET；不加级联类行为
- Test witness: constraint_enforcement_test 追加——(a) SET 唯一列为已有值 → DuplicateKey + 原值保持；(b) SET 非唯一列 → 唯一条目随行（更新后按唯一值查询可达新版本、唯一性检查继续正确）；(c) SET 唯一列新值成功且旧值不再命中；(d) SET 唯一列 NULL 成功且同值可重插；(e) 改值后唯一性保持（RTM 2.6 矩阵）。先观察 RED
- GREEN condition: 五组用例绿 + 既有 update 面零回归
- Verification: `cargo test --test constraint_enforcement_test`；退出码 0
- Stop when: `old_tuple_bytes` 反序列化缺列值提取通路（需新增 helper 时属非实质可自行加；若需改 tuple 格式则返回 Plan）

### 2.7: DELETE 条目移除与回滚唯一修复

- Requirement/Scenario: R3 S6（DELETE 后同值可重插）、S7（事务回滚后同值可重插）
- Depends on: 2.3、2.5（条目语义）
- Targets: `src/executor/delete.rs::DeleteExecutor::next`、`src/transaction/manager.rs::abort_cleanup_versions`
- Current behavior: DELETE 只删 PK 条目；abort 修复只修 PK 索引——残留唯一条目使后续同值插入假阳性 DuplicateKey
- Required behavior: DELETE——PK `search` 得 rid 后、写墓碑前，从 rid 处 slot 数据反序列化行元组提取唯一列值（`deserialize_tuple` + 列序；NULL 跳过），在删 PK 条目（现 :91）同区逐唯一列 `uindex.delete(u_key)`；SlotNotFound 容忍路径（无 tuple 可读）与 PK 同型——跳过唯一删除。abort——`abort_cleanup_versions` 对每表 `meta.unique_indexes` 逐唯一索引执行与 PK 同型修复（`uindex.find_key_by_row_id(rid)` → 有前驱 `update(key, prev)` / 无前驱 `delete(key)`）；`mark_aborted` 中性化逻辑不变
- Required changes: delete.rs 元组读取 + 唯一删除臂；manager.rs 逐唯一索引修复循环
- Preserve: 墓碑 slot 化与 record_version/WAL 语义零变化；abort 的 PK 修复顺序与 mark_aborted 位置不变；进程内 row_to_key 反向映射性质（PK 同型——回滚场景条目均经同实例插入可命中；恢复后实例无未提交事务故无修复对象）
- Forbidden: 不改 WAL Delete 记录格式（唯一条目移除不新增 WAL 记录——恢复重建自数据页，2.8）；不重设计 rekey-abort 边缘语义（继承 PK 既有形状，design D8 明示同型继承已知边界）
- Test witness: constraint_enforcement_test 追加——(a) DELETE 唯一行后同值重插成功（S6）；(b) CLI 会话显式 `ROLLBACK` 后同值重插成功；(c) CLI 会话事务内语句失败自动回滚后同值重插成功（S7——经 `rollback_session` → `abort_cleanup_versions`）；(d) DELETE 后唯一索引无残留（同值重插不被拒）。(b)(c) 为 cli_test 用例（会话路径），(a)(d) 为 lib e2e。先观察 RED（当前回滚后重插假阳性拒绝）
- GREEN condition: 四组用例绿
- Verification: `cargo test --test constraint_enforcement_test --test cli_test`；退出码 0
- Stop when: DELETE 读元组发现墓碑前版本不可达（版本链形态异常——返回 Plan）

### 2.8: 恢复重建唯一索引

- Requirement/Scenario: R5 S1（干净重开——2.3 承载，此处回归见证）、S2（崩溃恢复后强制保持）、S3（跨链重复显式报错）、S4（旧格式兼容）
- Depends on: 2.1、2.3
- Targets: `src/wal/recovery.rs::rebuild_pk_indexes`、`src/storage/data/table_manager.rs::replace_index_manager`（或其 swap 扩展）
- Current behavior: 恢复只重建 PK 索引；唯一索引在 redo_count>0 后丢失
- Required behavior: 五步形状逐表扩展——(1) 除 PK 外为每个唯一列建新 `IndexManager`；(2) 页扫描时同一次 `deserialize_tuple` 兼提 PK 键与各唯一列键（NULL 跳过；现 `extract_pk_key` 扩展或多键提取 helper）；(3) 链尾回溯循环内逐唯一列维护 `HashMap<键, RowId>`，同一唯一列跨链重复存活值 → `WalError::RedoFailed` 显式点名表与列（镜像 PK :937 文案风格）；(4) 批量 insert 后 PK 与唯一索引一并 swap（`replace_index_manager` 扩展或新方法，形态非实质）→ catalog 根写回（`update_table_root` 既有 + 逐唯一 `update_unique_index_root`）→(5) 旧树（含旧唯一树）洞容忍释放。`redo_count == 0` 路径零变化（唯一索引自 catalog 根加载）
- Required changes: recovery.rs 重建扩展 + table_manager swap 面
- Preserve: PK 重建五步语义、`RedoFailed` 错误面、洞容忍释放 warn+放弃；恢复期不 attach catalog 上下文（root 变更不落 catalog，重放后统一接线）；唯一根页写回仅发生于重建完成后（2.2 槽位上下文在 attach 前不生效）
- Forbidden: 不改 WAL 记录格式与重放逻辑（redo 臂零变化）；不为唯一重建引入新错误变体（复用 `RedoFailed`）
- Test witness: constraint_enforcement_test 追加（复用 `mvcc_tombstone_visibility_test`/`isolation_level_test` 恢复测试模式——WAL 保留 + 非正常关闭重开）——(a) 崩溃恢复后 INSERT 重复值仍 DuplicateKey（S2）；(b) 干净关闭重开后强制保持（S1 回归见证）；(c) 跨链重复注入（直改数据页构造两条链同值——最低成本形态，或恢复测试夹具允许的等价注入）→ 恢复以点名表列的显式错误失败（S3）；(d) 旧格式 catalog 行（2.1 的手工字节 fixture 或旧版文件）打开成功、行为＝无唯一索引（S4）。先观察 RED（当前恢复后重复值落库成功）
- GREEN condition: 四组用例绿 + 既有恢复套件零回归
- Verification: `cargo test --test constraint_enforcement_test`；退出码 0
- Stop when: 唯一列键提取需改 tuple 反序列化公共 API 签名（影响多消费面——返回 Plan；局部 helper 非实质可自行加）

### 2.9: 收口验证与文档

- Requirement/Scenario: R4 S4（dump/restore 往返保持）、S5（存量边界文档化）、R5 S5（drop 释放 e2e）、R6（全量零回归）
- Depends on: 2.1-2.8
- Targets: `tests/cli_test.rs`、`tests/constraint_enforcement_test.rs`、`README.md`、`README.zh-CN.md`
- Current behavior: README 无约束语义说明；dump/restore 对 UNIQUE 无往返见证
- Required behavior: (a) cli_test——INT UNIQUE 表 dump→新库 restore 成功且唯一强制在新库生效（R4-S4 往返恒等）；`schema` 子命令输出渲染 UNIQUE（表级单列映射后）；`"pk" BOOL PRIMARY KEY NOT NULL UNIQUE` 自家 dump 形态 restore 可达（PK 列消费裁定）；(b) constraint_enforcement_test——drop_table 页释放后同进程新建表写入正常且文件页数无异常增长（R5-S5）；(c) `cargo test` 全绿（0 failures，ignored=2 或逐条归因）；(d) README 双语新增约束语义段：NOT NULL 强制、INT 列 UNIQUE（NULL 豁免）、CHECK/FK/方言项与非 INT/组合 UNIQUE 的计划期拒绝面、存量兼容边界两条——非 INT UNIQUE 列 dump→restore 破坏（proposal 已接受的裁定）+ 存量「NOT NULL 列含 NULL」数据 dump→restore 被拒（Iteration 000 Act Response Remaining Issues 归档于此）
- Required changes: 测试收口 + README 双语文档
- Preserve: 既有 README 结构与既有章节内容；`rtsql-docs/SKILL.md` 不在本任务（如需另行裁定）
- Forbidden: 不为过测试弱化断言；不扩写范围外文档（安装、性能等）
- Test witness: (a)(b) 新用例 RED→GREEN 或既有机制直接满足的见证；(c) `cargo test` 统计行；(d) 文档无自动判定——以内容覆盖上述五点为 Act Response 声明项
- GREEN condition: 全部测试绿 + README 双语段落齐备
- Verification: `cargo test`；退出码 0；统计行写入 Act Response
- Stop when: dump→restore 往返出现非预期失败且无法归因本 change 契约（返回 Plan）

**Invariants**

- PK 索引既有语义（DuplicateKey/KeyTypeMismatch/无键行落库不入索引/rekey 三分支）逐字节不变。
- 无 unique 列表的全部 DML/DDL/恢复行为逐字节不变；`unique_indexes` 恒空路径零开销语义。
- WAL 记录格式、数据页/索引页格式、checkpoint 位点格式零变化（catalog 表行是唯一格式演进面，向后兼容）。
- 既有错误文案零变化；唯一强制复用既有 `DuplicateKey`/`RedoFailed`/`UnsupportedConstraint` 变体。
- CLI exit code 映射（0/1/2/3/4/5）不变。
- 恢复期 replay 不 attach catalog 上下文的语义不变（R-T0b-R5）。
- `redo_count == 0` 恢复路径行为不变（唯一索引自 catalog 根加载）。
- 旧格式 catalog 行与非 INT unique 标志旧库可打开、DML 可用（行为＝无唯一索引）。

**Non-goals**

- 用户级 CREATE/DROP INDEX 语句（MS21-T01——复用本期唯一索引基建，建议序在其后）
- String/Float/Bool/Date/Timestamp 键编码扩展（I024/MS21 域）
- FOREIGN KEY/CHECK 强制（永久 Non-goal，Iteration 000 已诚实化）
- 组合 UNIQUE 语义；约束随 ALTER 演进（MS21）；DEFAULT 应用（MS24-T01）
- rekey-abort 的 PK 既有边缘语义重设计（同型继承，design D8）
- 部分索引/表达式索引

**Acceptance**

- R3：INT 列 UNIQUE 经专属唯一索引在 INSERT（预检零副作用）/UPDATE（四分支）/DELETE/回滚全路径强制且 NULL 豁免、多唯一列独立（constraint_enforcement_test S1-S8 八场景 + cli_test S7 会话面）。
- R4：非 INT UNIQUE ×5、组合 UNIQUE 点名拒绝；表级单列 UNIQUE(col) 等价映射且 schema 渲染正确；dump/restore INT UNIQUE 往返保持；PK 列 UNIQUE 不建第二索引且自家 dump 形态 restore 可达（planner_test 矩阵 + cli_test）。
- R5：干净重开自 catalog 根、崩溃恢复自数据页重建两态一致，跨链重复显式报错点名表列；旧格式兼容打开；drop_table 释放唯一索引页（constraint_enforcement_test 两态 + catalog 单测）。
- R6：`cargo test` 全绿（0 failures，ignored=2 或逐条归因记录），无未归因修改；README 双语兼容边界说明齐备。
- 映射：R3→D5/D6/D7/D8→2.2/2.3/2.4/2.5/2.6/2.7；R4→D6→2.4/2.9；R5→D4/D5/D9/D10→2.1/2.3/2.8/2.9；R6→2.9（tasks.md RTM）。

**Verification**

| Scenario | 判定 |
|---|---|
| R4 DDL 策略矩阵 | `cargo test --test planner_test` 退出码 0，新矩阵绿 |
| R3 写路径全矩阵 | `cargo test --test constraint_enforcement_test` 退出码 0（INSERT/UPDATE/DELETE/回滚组） |
| R3 S7 会话回滚 | `cargo test --test cli_test` 退出码 0，回滚重插用例绿 |
| R5 两态 + catalog | `cargo test --lib` 退出码 0（catalog 单测）+ constraint_enforcement_test 两态组绿 |
| R4-S4/R5-S5 收口 | `cargo test --test cli_test` 往通用例绿 + drop 释放用例绿 |
| R6 回归 | `cargo test` 退出码 0，统计 `0 failures`（ignored 与基线一致或逐条归因） |

全部为测试框架原生退出码与统计输出判定；无人工步骤（README 为内容声明项，非验证判定对象）；无 Evidence 目录要求。

**Gate 2 Readiness**

- 无 Missing requirement：PASS——RTM 六行全 Covered（tasks.md），无 Simplified。
- 调查完整：PASS——Current-State Evidence 列 catalog/IndexManager/TableManager/三执行器/abort/恢复/键编码/planner 全消费面与行号（本文件）；Iteration 000 既有面（TableMeta/insert/update NOT NULL 臂）自 accepted Review 采信，只补查本期新表面。
- 设计闭合：PASS——design.md D4-D10 定案格式演进、槽位泛化、策略面、写路径、abort、恢复、drop；D6 第 3 点论据不精确处已由 Plan 澄清裁定（PK 列 UNIQUE 消费为 PK 唯一性，写入 2.3/2.4 契约），无影响契约语义的 TBD。
- 任务可执行：PASS——九个 Task Contract 均有 Targets/Current/Required/Preserve/Forbidden/见证/停止条件。
- 分轮合理：PASS——Iteration Plan 平衡审计记录于 tasks.md（唯一索引单一垂直切片，中途切分留 enforcement 半开状态不可交付）；单 Iteration 承载单一成果。
- 追踪完整：PASS——RTM requirement×scenario×design×task×code×test 全链接。
- 验证充分：PASS——覆盖 R3/R4/R5 全部 scenario（含 sad path：拒绝矩阵、碰撞预检、跨链重复、旧格式兼容、回滚重插）与 R6 回归；全部最简直接判定（测试框架原生退出码）。
- 无身份型证据工程：PASS——无哈希/run-id/判定层；验证用既有 cargo test。
- 无实质未知项：PASS——sqlparser 变体名核对类非实质项写入 2.4 停止条件；ColumnDef 可变性、swap 方法形态、元组提取 helper 形态均非实质留 Act。
- OpenSpec 一致：PASS——proposal/design/tasks/specs/cycle 相互一致；`openspec validate --strict` 于交付前执行。
- Persisted Evidence：PASS——Mode none（下方声明），全部验证 Act Response 可承载。
- 用户批准计划：**PASS——用户于 2026-09-24 规划交付后明确答复「批准」（原话，Iteration 000 Gate 2 同源记录），批准对象为含双 Iteration 全部任务与用户四项裁定的 change 级计划；本 Cycle 为该已批计划中 Iteration 001 的展开。**

**Persisted Evidence**

- Mode: none

Act Response 记录命令、决定性输出（每项 ≤20 行）、退出码即满足全部验收判定；无不可复现依赖、无一次性环境、无需要留场的现场。Budget：不适用。

**Risks and Notes**

- sqlparser 0.44 `TableConstraint::Unique` 的 columns 字段形态与 `ColumnDef` 约束可变性以实际代码为准；臂匹配与字段访问差异属非实质调整，记录于 Act Response Deviations。
- 非 INT unique 标志旧库打开为静默跳过（行为＝无唯一索引）：与 R5-S4「行为为无唯一索引」及 R4-S5「restore 被拒」的分工是——**直接打开旧文件可用**，dump 后 **restore 新建**才被 2.4 拒绝面挡住；README 文档须写清这一区分。
- UPDATE 多唯一列 × SET PK 列组合：PK rekey 改变 row_id 时全部唯一条目须随行 update（四分支的分支 1 覆盖，Act 实现时勿漏该组合的矩阵用例）。
- 唯一索引 abort 修复依赖进程内 row_to_key（PK 同型性质）；恢复后无未提交事务故无对象，不构成缺口。
- DELETE 的 SlotNotFound 容忍路径下无元组可读——唯一条目无法删除（与 PK 同型容忍，既有测试夹具形态，非生产路径）。
- 2.8 跨链重复注入用例需直改数据页（恢复测试夹具允许范围）；若夹具不支持，最低成本形态与 Act 商定后记录，不为此扩大夹具面。
- Iteration 000 遗留 Minor（存量 NOT NULL 列含 NULL 数据的 dump→restore 拒绝）已归档至 2.9 README 文档面，本期无代码处置。

## Act Response

- Status: reported

**Implemented**

Iteration 001 全部 9 个 task 完成（2.1-2.9），除 2.2（重构，变更前 GREEN 保持）外全部按契约 RED→GREEN；同 Iteration Plan Review 的 Follow-up Decision F1 修复已在当前 Cycle 完成（见下方 F1 修复段；本 Response 曾置 pending 消费 Review，修复后覆盖为本完整快照）。INT 列 UNIQUE 经专属非 PK 唯一索引端到端强制：catalog 表行尾随追加 `u32 unique_count | N × u32 roots` 向后兼容段（旧行读为空）；`IndexManager` catalog 上下文泛化为 `CatalogRootSlot`（PK 表根 / 第 N 唯一根）并按槽位分派持久化；`TableMeta.unique_indexes: Vec<(usize, Arc<IndexManager>)>` 经 create（INT ∧ unique ∧ 非 PK 列逐列建树）/ open_or_init（自 catalog 根 `from_root`，空 roots 旧行静默无索引）/ replace（继承）三构造点接线，drop 逐唯一树 best-effort 释放，恢复后 `attach_index_catalog_contexts` 逐唯一索引 attach；DDL 策略面——非 INT 列级与表级单列 UNIQUE、组合 UNIQUE 计划期点名拒绝，表级单列 `UNIQUE(col)` 映射列标志，PK 列声明 UNIQUE 消费为 PK 既有唯一性（承载面跳过，不建第二索引，restore 自家 dump 恒可达）；INSERT 预检（NULL 豁免、`DuplicateKey` 零副作用拒绝）+ 数据落位后条目；UPDATE 四分支（随行 / 同值 / 改值预检 + delete-先于-insert rekey / 置 NULL 删条目）；DELETE 写墓碑前提取唯一键、删 PK 条目同区移除；`abort_cleanup_versions` 逐唯一索引同型修复；崩溃恢复 `rebuild_pk_indexes` 扩展——同一次 tuple 反序列化兼提 PK 与唯一键、逐唯一列跨链重复 `RedoFailed` 点名表列、整表 swap（新 `replace_recovery_indexes`）、逐唯一根写回 catalog、旧树洞容忍释放。NULL 全路径豁免；无 unique 列的表全路径逐字节不变。

F1 修复（Plan Review Follow-up Decision 1-4）：闭合「2.4 INT 门只约束列声明类型、不约束值运行时类型」的前提缺口——INSERT 唯一预检循环原位（NULL 豁免后、`to_key()` None 处）与 UPDATE 碰撞预检区（任何写入前）对「UNIQUE 列新值非 NULL 且 `to_key()` 为 None」返回 `StorageError::KeyTypeMismatch { column, expected: "INT", actual: key_value_type_name(..) }`（镜像 update.rs MS16 PK 先例，零副作用拒绝）；UPDATE 唯一维护区 else 臂 `new_key.unwrap()` panic 路径删除（if-let 防御形态，可达态行为逐字节不变）；两条守卫仅对 `unique_indexes` 非空列生效（循环遍历 unique_indexes 天然满足），旧格式非 INT unique 标志列（无索引）与无唯一列表路径不变。测试见证：`tests/constraint_enforcement_test.rs` 追加 `insert_non_int_into_int_unique_column_rejected_key_type_mismatch` / `update_set_int_unique_column_to_non_int_rejected_keeps_old_value`（RED——静默成功与 update.rs:298 panic——后 GREEN）。

**Changed Files and Symbols**

产品（12 文件；F2 勘误——`src/cli/lifecycle.rs` 此前漏报）：
- `src/storage/catalog.rs` — `CatalogRow.unique_roots: Vec<u32>`；`serialize_catalog_row`/`deserialize_catalog_row` 尾随段对称扩展（兼容读：段缺失/不完整 → 空）；`Catalog::update_unique_index_root(table, ordinal, root)`（仿 `update_table_root` 闭包模式，ordinal 越界 Internal 错误）。
- `src/storage/btree/index_manager.rs` — 新 `CatalogRootSlot` 枚举（`PrimaryKey{table}` / `Unique{table, ordinal}`）；`catalog_ctx`、`with_catalog_context`、`set_catalog_context`、`sync_root_to_catalog` 槽位泛化与分派。
- `src/storage/btree/mod.rs`、`src/storage/mod.rs` — `CatalogRootSlot` 再导出。
- `src/storage/data/table_manager.rs` — `TableMeta.unique_indexes`；`create_table_with_constraints`（qualifying 列建树 + `unique_roots` 入 catalog 行 + 过时「metadata only」注释更新）；`open_or_init`（空 roots 旧行静默跳过；非空 roots 与 qualifying 数不匹配 Internal 错误；`from_root` 重建）；`replace_index_manager` 继承；新 `replace_recovery_indexes`（恢复整表 swap，返回旧 PK + 旧唯一树）；`drop_table` 逐唯一树 collect+free；`attach_index_catalog_contexts` 逐唯一索引 attach `Unique{ordinal}` 槽位。
- `src/parser/planner/ddl_dml.rs` — `build_create_table` 新增 UNIQUE 策略面：列级非 PK 非 INT 门、表级组合拒绝、表级单列映射（幂等去重）、PK 列消费裁定（`column_defs` 改 `mut` 以支持映射）。
- `src/executor/insert.rs` — PK 预检后唯一预检臂（NULL 豁免、`DuplicateKey`；F1 修复追加非 NULL 无键值 `KeyTypeMismatch` 守卫）+ PK 索引插入后逐唯一条目臂。
- `src/executor/update.rs` — Step 2 后唯一列旧值快照（无唯一列零开销）、Step 3 后碰撞预检（任何写入前；F1 修复追加非 NULL 无键新值 `KeyTypeMismatch` 守卫）、Step 7 后四分支维护区（F1 修复移除 else 臂 `unwrap`，if-let 防御形态）。
- `src/executor/delete.rs` — `unique_keys_of_row`（写墓碑前提取，SlotNotFound 容忍同型跳过）+ 删 PK 条目同区逐唯一删除。
- `src/transaction/manager.rs` — `abort_cleanup_versions` 逐唯一索引 `find_key_by_row_id` 同型修复（PK 修复与 `mark_aborted` 之间）。
- `src/wal/recovery.rs` — `extract_index_keys`（一次反序列化兼提 PK + 唯一键）、`RebuildScanPage`/`RebuildSlots` 类型扩展、`rebuild_pk_indexes` 五步全扩展（逐表 scan_columns 定 qualifying 列 → 建唯一新树 → 扫描 → 链尾回溯逐唯一列判重 `RedoFailed` 点名表列 → 批量插入 + 整表 swap + PK/唯一根写回 + 旧树释放）。
- `src/cli/lifecycle.rs` — （F2 勘误补报）测试模块 `CatalogRow` 夹具补 `unique_roots` 字段——结构体字面量穷举性编译必需行，语义为空。

测试（3 文件 + README ×2）：
- `tests/planner_test.rs` — +5（非 INT 列级 UNIQUE ×5 类型矩阵、表级单列映射、组合拒绝、PK 列 UNIQUE 接受、表级单列非 INT 拒绝）。
- `tests/constraint_enforcement_test.rs` — +16（INSERT UNIQUE 四组、UPDATE 四分支五组 + PK-rekey 组合、DELETE 重插、崩溃恢复/干净重开/跨链重复注入、drop 页释放 + F1 修复见证 2：`insert_non_int_into_int_unique_column_rejected_key_type_mismatch` / `update_set_int_unique_column_to_non_int_rejected_keeps_old_value`；勘误——原报 +13 实为 +14，修复前文件 22 用例自洽）。
- `tests/cli_test.rs` — +4（F2 勘误：原报 +3；显式 ROLLBACK 重插、事务内唯一冲突自动回滚重插、INT UNIQUE dump→restore 往返强制、PK-BOOL-UNIQUE 自家 dump 形态 restore）+ 1 处既有用例校准（见 Deviations 6）。
- `README.md` / `README.zh-CN.md` — 新增「Constraints / 约束」段（NOT NULL 强制、INT 列 UNIQUE 与 NULL 豁免、计划期拒绝面、两条存量兼容边界）。

**Deviations from Plan**

1. **2.6 碰撞预检位置**：契约写「与 NOT NULL/MS16 校验同层」，但预检输入需要旧值（Step 2 才读旧 tuple），实现放在 Step 3 改值后、任何写入（Step 6）之前。满足契约实质约束「任何写入前」（零副作用成立），为该检查可计算的最早位置；分类为契约内部张力的实现裁定，非实质偏差。
2. **2.3 空 unique_roots 语义细分**：契约停止条件写「根页数与 unique 列数不一致（数据异常——返回 Plan 定策略）」。实现按契约 Invariants/R5-S4（「旧格式 catalog 行可打开、行为＝无唯一索引」）细分为：空 roots（旧格式行）→ 静默无唯一索引；非空 roots 但数目与 qualifying 列不匹配 → `StorageError::Internal`（新代码行不可能产生，防御面）。分类：按契约其余文本裁定，请 Plan Review 核验。
3. **2.4 表级映射与列级 UNIQUE 叠加**：契约停止条件「产生重复 Unique 约束的形态（去重策略返回 Plan）」——实现为幂等跳过（已携带 Unique 不重复推入），语义无歧义，未触发 Plan 上升。
4. **2.4 表级 UNIQUE 指向未知列** → `PlanError::ColumnNotFound`（契约未点名；仿表级 PK 未知列经 create_table `ColumnNotFound` 的既有形态）。
5. **2.8(d) 旧格式兼容见证**由 2.3 的 `legacy_row_with_int_unique_flag_opens_without_unique_index` 承载（catalog.insert_table 手工旧行 + open_or_init 断言），未在 constraint_enforcement_test 重复 Database 级夹具。
6. **既有用例校准 ×1**：`cli_test::test_schema_unique_roundtrip`（MS15-Rest）夹具 `name STRING UNIQUE` → `code INT UNIQUE`。归因：非 INT UNIQUE 按新契约（R4-S1）计划期点名拒绝，原夹具依赖旧的静默接受行为；schema 渲染保真断言等价迁移（`"code" INT UNIQUE`）。属本 change 契约校准，非回归。
7. **2.8 测试夹具机制**：崩溃两用例 DDL 后 `db.checkpoint()`（wal_recovery_large_test 夹具先例——catalog 页无 WAL 记录，靠 checkpoint 落盘）；跨链注入后 `buffer_pool.flush_all()`（直写槽无 WAL 记录，不落盘即丢失）。夹具机制细节，非契约偏差。
8. **F1 修复（Plan Review Follow-up Decision 1-4）**：UPDATE 维护区 else 臂以 if-let 防御形态移除 `unwrap`（契约第 1 点要求删除 panic 路径；预检守卫使该臂不可达态退化为 NULL 同型「无条目」语义，可达态行为逐字节不变）；两条守卫仅遍历 `unique_indexes` 生效，旧格式非 INT unique 标志列（无索引）与无唯一列表路径不变（R5-S4/R6 保持）；守卫位于 PK 重复预检之后（与既有错误优先序一致）。
9. （F2 勘误，Plan Review 指出）产品文件清单漏报 `src/cli/lifecycle.rs`（11→12）、cli_test 计数 +3 实为 +4（93→97）、constraint_enforcement_test 计数 +13 实为 +14——本快照已按勘误更正，代码无需修复。

**Blocker Handoff**

None required.

**Blocker Resolution**

（无——本 Cycle 未进入 blocked 状态。）

**Self-Review**

- Plan compliance: 9/9 task 按契约完成；Plan Review Follow-up Decision 1-4 全部按契约执行（守卫位置——INSERT 唯一预检循环原位 / UPDATE 碰撞预检区任何写入前；错误变体与 MS16 先例同型同字段；NULL 豁免保持；见证两用例 RED→GREEN）；写路径顺序（INSERT 预检在 serialize 前/条目在 PK 插入后、UPDATE delete-先于-insert、DELETE 提取在墓碑前）与契约逐条一致；Forbidden 项（不动 PK 分支、不加用户级 CREATE INDEX、不实现 FK/CHECK 强制、不改 WAL/页/checkpoint 格式、不引入新错误变体、不触碰未提交文档批次）在修复轮复核无违反；Invariants（无 unique 列表全路径逐字节不变、PK 语义不变、恢复 replay 不 attach 语义不变、redo_count==0 路径不变、exit 映射不变）修复轮复核成立。
- Full diff reviewed: 是——修复轮对 19 文件全量 diff（src 14 + tests 3 + README ×2）重审：F1 两处守卫与两条见证为本轮唯一新增行为面，其余为 Iteration 000/001 既有已审内容；工作树中既有文档批次（tasks/improvements/references/analysis 归档 + archive carrier + change 目录）未被触碰、未暂存。
- 已修复发现：update.rs 预检循环误用外层 `col_idx`（E0614 编译期即修复）；recovery.rs slots map clippy type_complexity（类型别名 `RebuildSlots` 消除）；delete.rs 内联类型路径改导入（风格统一）；跨链注入测试 advisory 锁未释放（`meta` Arc 存活）→ 作用域收敛修复；F1 panic 路径（Plan Review 发现，当前 Cycle 修复——见 Implemented F1 段与 Verification F1 行）。修复后受影响面复跑绿（末次：constraint 24 + 全量 1152）。
- Critical findings unresolved: 无。
- Important findings unresolved: 无。
- Minor findings unresolved: 1 项——见 Remaining Issues。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| 2.1 RED | `cargo test --lib storage::catalog` | `error[E0609]: no field unique_roots` / `E0599` | catalog 行格式 | RED 见证 ✓ |
| 2.1 GREEN | `cargo test --lib` | `302 passed; 0 failed` | +4 catalog 单测（往返/旧行兼容/root 写回保字段/ordinal 定位） | PASS |
| 2.2 GREEN | `cargo test --lib` | `302 passed; 0 failed`（重构后） | 槽位泛化，root 同步行为不变 | PASS |
| 2.3 RED | `cargo test --lib table_manager` | `error[E0609]: no field unique_indexes` ×5 | TableMeta 承载面 | RED 见证 ✓ |
| 2.3 GREEN | `cargo test --lib` | `306 passed; 0 failed` | +4（create 建树/重开重建/非 INT 跳过/旧行兼容） | PASS |
| 2.4 RED | `cargo test --test planner_test` | `4 failed`（非 INT ×5/组合/映射/表级非 INT 静默成功） | DDL 策略面 | RED 见证 ✓ |
| 2.4 GREEN | `cargo test --test planner_test` | `48 passed; 0 failed`（43 既有 + 5 新增） | 策略矩阵 + 既有零回归 | PASS |
| 2.5 RED | `cargo test --test constraint_enforcement_test` | `2 failed`（重复值落库成功） | INSERT 既有行为 | RED 见证 ✓ |
| 2.5 GREEN | 同上 + `cargo test --lib` | `12 passed` / `306 passed` | INSERT 矩阵（拒绝零副作用/不同值/多 NULL/双列独立） | PASS |
| 2.6 RED | 同上 | `3 failed`（占用不随更新迁移） | UPDATE 既有行为 | RED 见证 ✓ |
| 2.6 GREEN | 同上 + `cargo test --lib` | `17 passed` / `306 passed` | 四分支 + PK-rekey 组合 + 分支 1 回归见证 | PASS |
| 2.7 RED | constraint + cli_test | delete 重插 `FAILED` + cli 回滚 ×2 `FAILED`（残留条目假阳性拒绝） | DELETE/回滚既有行为 | RED 见证 ✓ |
| 2.7 GREEN | `constraint 18 passed` + `cargo test --lib 306` + `cli_test 95 passed` | 三路径重插全部成功 | DELETE/显式回滚/自动回滚 + abort 唯一修复 | PASS |
| 2.8 RED | `cargo test --test constraint_enforcement_test` | `2 failed`（恢复后强制丢失 / 跨链重复重开未报错） | 恢复重建既有行为 | RED 见证 ✓ |
| 2.8 GREEN | 同上 + recovery/checkpoint_redo/mvcc/isolation 四套件 | `21 passed`；`3/9/11/9 passed` 全绿 | 崩溃两态 + 跨链 RedoFailed 点名 + 恢复套件零回归 | PASS |
| 2.9 | `cargo test --test cli_test` + constraint drop 用例 | `97 passed; 2 ignored`；drop 释放 `1 passed` | dump→restore 强制往返 + PK-UNIQUE 形态 restore + drop 页复用（既有机制直接满足，RED 不适用） | PASS |
| 2.4 校准 | `cli_test::test_schema_unique_roundtrip` | `passed`（INT 夹具） | 非 INT UNIQUE 新拒绝面下的渲染保真 | CALIBRATED |
| 2.9 全量 | `cargo test` | `passed=1150 failed=0 ignored=2`（基线 1119 + 净增 31 精确吻合） | 全仓库 | PASS |
| OpenSpec | `openspec validate 2026-09-24-ms23-constraint-enforcement` | `Change ... is valid` | change 结构 | PASS |
| 静态检查 | `cargo build` + `cargo clippy` | 无新增 warning（type_complexity 已消除） | 全部改动文件 | PASS |
| F1 RED（INSERT） | `cargo test --test constraint_enforcement_test insert_non_int` | `Expected Error containing "key column 'code' expects INT, got String", got AffectedRows { count: 1 }`（修复前静默落库无条目） | INSERT 唯一预检修复前行为 | RED 见证 ✓ |
| F1 RED（UPDATE） | 同轮 `update_set_int_unique_column_to_non_int_rejected_keeps_old_value` | `panicked at src/executor/update.rs:298:39: called Option::unwrap() on a None value`（22 既有用例同轮全绿） | UPDATE 维护区修复前 panic 路径 | RED 见证 ✓ |
| F1 GREEN | `cargo test --test constraint_enforcement_test` | `24 passed; 0 failed`（22 既有 + 2 新增） | 两守卫 + unwrap 移除 + 约束矩阵零回归 | PASS |
| 修复轮静态检查 | `cargo clippy --all-targets` | 退出码 0，无代码 warning | 全部改动文件 | PASS |
| F1 后全量 | `cargo test` | `passed=1152 failed=0 ignored=2`（基线 1150 + 净增 2 精确吻合），退出码 0 | 全仓库 | PASS |
| OpenSpec（修复轮） | `openspec validate 2026-09-24-ms23-constraint-enforcement --strict` | `Change ... is valid` | change 结构 | PASS |

新鲜性注记：最终全量 1152 结论产生于 F1 修复（insert.rs/update.rs 守卫 + 测试见证 2 条）之后，覆盖全部改动表面；clippy 与 `openspec validate --strict` 同轮新鲜。1150 历史结论的采信链条见上行 2.9 全量行及其后受影响面复跑记录。

**Persisted Evidence**

None required（Plan Mode: none；无白名单情形，全部验证由本 Response 承载；未创建 evidence/ 目录）。

**Experience Candidates**

None.

**Remaining Issues**

1. （Minor，PK 同型既有边界，保持 Iteration 001 原记录）DELETE 的 SlotNotFound 容忍路径下唯一条目无法移除、恢复后实例 `row_to_key` 反向映射为空故 abort 修复无对象——均为 PK 同型既有边界（契约 Preserve/D8 明示继承），非本 change 缺口；若未来出现生产路径触达再行评估。
2. （Minor，已按 2.9(d) 文档化收口）存量兼容两条边界——非 INT UNIQUE 列 dump→restore 被拒、NOT NULL 强制前写入的 NULL 行 restore 被拒——README 双语「约束/Constraints」段已覆盖，无代码处置。
3. （范围外既有缺陷，Plan Review F4 定性，待 User 指令）非键列 INSERT/UPDATE 值与列声明类型全程无类型校验（String/Float 写入 INT 列静默落库，读回才报 invalid value）——早于本 change 存在；本 change 已收口其在唯一强制面的边缘（F1 panic 与唯一预检静默跳过均改为 `KeyTypeMismatch` 拒绝），其余面修复属独立类型校验面，建议由 Recorder 按用户指令登记 Issue。

**Commit or Diff Reference**

未提交（用户未指令 commit）；工作树含 Iteration 000 + 001 全部代码/测试改动 + F1 修复 + change 产物 + README 双语更新；既有文档批次（tasks/improvements/references/analysis/archive carrier）原样保留。

## Plan Review

- Review Result: accepted

**Findings**（当前 Cycle 修复轮重新 Review——按 Follow-up Decision 完成后覆盖前版 Review 为最新完整反馈）

独立检查（非 Self-Review 复述）：对 F1 修复三处代码位点、两条新见证与本轮 diff 范围逐一核对——

1. INSERT 守卫（insert.rs 唯一预检循环原位）：NULL 豁免分支保持在前；`let Some(key) = value.to_key() else { return Err(StorageError::KeyTypeMismatch { column, expected: "INT", actual: key_value_type_name(..) }) }` 取代原静默跳过；位置在 PK 预检之后、serialize 与一切写入之前，零副作用成立；列名自 `table_meta.columns[*col_idx]` 取 schema 列序，与索引绑定列一致。
2. UPDATE 守卫（碰撞预检区，任何写入前）：同值/NULL 跳过保持在前（自碰撞误报规避不变）；`let Some(new_key) = new_v.to_key() else { return Err(KeyTypeMismatch…) }` 位于 Step 4 serialize 之前——修复前该形态先写损坏值（Step 4-6）再 panic，现在任何写入前类型化拒绝，原行保持。
3. 维护区 else 臂 `new_key.unwrap()` 已移除，改为与特性其余位点一致的 `if let Some` 防御形态；可达态独立复核——else 臂仅在「SET 目标为该唯一列 ∧ 新值非 NULL ∧ 异于旧值」时触达，而 UPDATE 守卫已保证该形态下 `to_key` 必为 Some，if-let 为纯防御（不可达 None 退化为不写条目，语义等同 NULL 分支且不会发生）；非 SET 目标唯一列恒 `old_v == new_v` 走分支 1，不受影响。
4. 守卫作用域：两条守卫均在 `unique_indexes` 非空循环内生效——旧格式库非 INT unique 标志列（无索引）与无唯一列表路径逐字节不变（R5-S4/R6 保持）；NULL 豁免语义不变。
5. 错误面：`KeyTypeMismatch` Display（`"key column '{column}' expects {expected}, got {actual}"`，error.rs:54）与两条见证断言文本（"…expects INT, got String" / "…got Float"）吻合；复用既有变体，无新错误变体（Invariant 保持）；契约 2.5「不加类型二次校验」前提已按 Follow-up Decision 修订。
6. 本轮 diff 范围核对：产品面净变化仅 insert.rs（37→+43）与 update.rs（74→+85）两文件（diffstat 对比前版 Review 时点），测试面仅 constraint_enforcement_test 两条追加（+2，总数 24 自洽）；其余文件未触碰。
7. 两条见证独立阅读：INSERT 用例断言拒绝文本 + 零副作用（行数 0、随后合法值可插）；UPDATE 用例断言拒绝文本 + 原行保持（SELECT 核对 code=100）+ 唯一条目归属保持（他人同值仍拒）。RED 证据与前版 Review F1 诊断精确互证——UPDATE RED 输出 `panicked at src/executor/update.rs:298:39: called Option::unwrap() on a None value`（即前版指认的 unwrap 位点），INSERT RED 为静默落库 `AffectedRows(1)`。
8. 前版 Findings 1-10（2.1-2.9 契约、Forbidden/Invariants）全部维持有效；F2 勘误已按请求落实（lifecycle.rs 入清单、cli_test +3→+4、constraint +13→+14），并顺带修正前版 Review 自身的「21 函数」计数笔误（修复前实为 22，现 24）——双方计数以 24 为准，以本行澄清。
9. F4（非键列类型校验缺失，范围外既有缺陷）维持 Issue 候选定性，Act 已在 Remaining Issues #3 如实承载、待 User 指令；本 change 唯一强制面的边缘已由本轮守卫收口。

**Deviation Classification**

- Deviation 8（维护区 if-let 防御形态）→ 非实质：即 Follow-up Decision 第 1 点「删除 panic 路径」的实现形态；可达态行为由预检守卫决定，if-let 仅为不可达态防御，与特性其余位点风格一致。
- 前版 Deviation 1-7 分类全部维持（非实质/非阻塞/合法校准）；F2 → ACT-DEVIATION 已按勘误闭合；F1 → PLAN-INVALID 已按 Follow-up Decision 修复闭合。
- 无新增 ACT-DEVIATION、无 BASELINE-CHANGED、无 NEW-EVIDENCE。

**Acceptance Gaps**

None——前版唯一 gap（R3 UPDATE 面可达 panic）已闭合：守卫使非 Int 值在任何写入前类型化拒绝（见证 2 条 RED→GREEN + 代码位点核对），维护区无 panic 路径；R6 全量 1152/0/2（1150 + 净增 2 精确吻合）与 clippy / `openspec validate --strict` 同轮新鲜。R1-R6 全部 Acceptance 满足。

**Convergence**

closed——F1 gap 一次修复轮内收敛（Review 指认 → 按 Follow-up Decision 契约修复 → 独立核对 + RED 证据互证），无重复返工。

**Evidence**

- 独立阅读：insert.rs 守卫区、update.rs 预检守卫区与维护区、`KeyTypeMismatch` Display（error.rs:54）、两条新见证全文、diffstat 范围对比；Act Response F1 修复段与 RED/GREEN 验证行。
- 采信 Act 修复轮验证结论（公共规则 › 验证）：constraint 24 / 全量 1152/0/2 / `cargo clippy --all-targets` / `openspec validate --strict` 均产生于当前工作树状态（只读基线检查：本 Review 全程只读、未修改任何覆盖表面，工作树与结论产生时点一致）；未重跑。
- 本次 Review 只读命令（sed/grep/git diff --stat）退出码均为 0。

**Follow-up Decision**

None——既有 Acceptance 全部满足，无当前 Cycle 修复项。Iteration 001 完成。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

None（Iteration 001 为本 change 最后一个 Iteration，已获 accepted——change 全部 Iteration 完成，进入收口：由 openspec-docs-maintainer 合并 `sql-constraint-enforcement` 增量规格、同步 tasks MS23 状态与 SNAPSHOT；commit 与 archive 由用户指令）
