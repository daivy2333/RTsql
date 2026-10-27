# Iteration 001 / Cycle 000: 创建、维护与恢复

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 规格 `sql-secondary-indexes` 中的建索引、条目与存活行、删列拒绝和唯一性边界。世代 1 的键字节和目录尾段沿用 000 已接受的实现。
- Excluded scope: 组合等值的 EXPLAIN、范围路由、`IndexRangeScan`、补扫、README、全量测试。这些属于 002 和 003。

**Objective**

SQL 可以创建规格允许的组合索引，以及新建的单列 INT 索引。回填、插入、更新、删除、UPSERT、REPLACE、回滚和未决 WAL 重建后，索引入口与当时存活行一致。删掉被任一索引列引用的列会失败，列和索引都还在。未被引用的列删掉后，其余索引的列号按删除位置减一，重开后仍然如此。查询行集与建索引前相同。已有的单列等值计划继续命中正确的键。

**Investigation Facts**

- Starting point: 000 的 Act Response 在 `1b1ca06`，Review Result 为 accepted。`Value::derive_secondary_index_key` 和目录尾段已经可用。打开表（`table_manager.rs` 装载二级索引处）、恢复重建和 `truncate_table` 仍把 `generation` 写成 0、`extra_column_indexes` 写成空。`register_secondary_index` 也固定写世代 0。验证沿用 000 Review 中记录的库测试和多列拒绝集成测试，覆盖的代码在该提交之后没有变化。
- Current-State Evidence:
  - `build_create_index` 在修饰检查之后、名字检查之前，于 `columns.len() != 1` 返回 `UnsupportedStatement("MULTI-COLUMN INDEX")`。通过后只构造单列 `CreateIndexNode { index_name, table_name, column_index, column_name }`。
  - 拒绝顺序今天是：修饰、多列、索引名是否存在且非限定、附加库写、表存在、裸列、类型、库级重名。类型门只允许 INT 和 `ColumnType::String`。
  - `CreateIndexExecutor::backfill_tree` 对单个 `column_index` 调用 `secondary_index_key`。`register_secondary_index` 把该列号写入目录和 `SecondaryIndexMeta`，世代固定 0。
  - 插入、更新、删除、UPSERT/REPLACE 的二级条目都按 `column_index` 一列调用 `secondary_index_key`。更新和 UPSERT 用 `delete_exact(key, row_id)` 再 `insert`。`upsert.rs` 有插入、冲突更新、冲突删除三条二级循环。
  - `extract_index_keys` 的主键和唯一分支用 `to_key`。二级分支按 `secondary_cols: &[usize]` 的单个列号调用 `secondary_index_key`。调用者是 `recovery.rs` 的重建，以及 `transaction/manager.rs` 的 `restore_tombstone_index_entries` 和 `read_prev_index_keys`。
  - `alter_drop_column` 只在 `column_index == removed` 时拒绝，文本是 `column '{}' is referenced by a secondary index and cannot be dropped (drop the index first)`。行重写的旧键和新键也只派生第一列。`narrowed_meta` 只按第一列过滤，并只把第一列在删除位置之后的序号减一；额外列原样复制。
  - `apply_column_drop` 和 `publish_column_narrowing` 在列数尚未是目标值时调用 `update_table_column_count_ddl`。该函数反序列化整行、只改 `column_count`、再序列化。ADD COLUMN（`table_manager.rs` 里把列数加一的那次调用）也用它。目录里的二级列号今天不会因为删列而改变。
  - `pipeline.rs` 把二级索引传给计划器的形式是 `(索引名, 第一列名)`。`extract_secondary_index_equality` 找到同名列后调用 `secondary_index_key`。这是今天单列等值进入 `IndexScanAll` 的唯一路径。
  - 直接断言多列被拒绝的测试有两处：`tests/secondary_index_test.rs` 的 `create_unique_and_composite_index_rejected`（`CREATE INDEX c ON t(age, id)`），以及 `tests/string_index_sql_test.rs` 的 `string_index_rejection_readonly_and_transaction`（`CREATE INDEX idx_multi ON t(name, age)`）。后者在拒绝之前已经 `CREATE INDEX idx_age`，并断言 `WHERE age = 25` 的行集不变且 EXPLAIN 点名 `idx_age`。
  - 崩溃模拟的现成形态是：DDL 后 `buffer_pool.flush_all()`，DML 后 `wal_buffer.shutdown()`，然后 `drop(db)` 且不 `close`，再 `Database::open`。见 `tests/update_index_maintenance_test.rs`。条目观察的现成形态是 `get_table` 后对 `secondary_indexes` 里的 `IndexManager` 调用 `search_all` / `scan_all`，行用 `execute_sql` 的 `SELECT`。见 `tests/string_secondary_index_test.rs`。
- Code and Critical Path: SQL `CREATE INDEX` 进入 `build_create_index`，执行器回填后 `register_secondary_index` 写目录。打开库时从目录行装入 `SecondaryIndexMeta`。之后的插入、更新、删除、UPSERT、删列重写和回滚都读这份内存元数据。未决 WAL 重开时，恢复按目录行新建空树，再用 `extract_index_keys` 回填。计划器只在单列等值上消费这份登记。

**Implementation Guidance**

先写 `composite_backfill_matches_live_rows` 并看到失败，再改计划器和回填。回填能对照条目之后，再改 DML、恢复和删列。同一轮里这些调用点都要改完，否则世代 1 的树会混进世代 0 的键。

世代只在计划器按下面的规则赋值一次。执行、恢复和回滚只读目录或内存元数据里的世代，不再各自选择编码。

已有 `CreateIndexNode` 字面量只有 `tests/string_secondary_index_test.rs` 一处，它建的是字符串索引。字段增加后填世代 0 和这一列，不要改成世代 1。`table_manager.rs` 里直接调用 `register_secondary_index` 的测试同样传世代 0 和空额外列。

**Task Definitions**

### T3: 组合索引和新建整数索引的条目与存活行一致

- Requirement/Scenario: CREATE INDEX 建立单列 INT 二级索引；组合二级索引的条目与存活行一致；唯一性边界保持在整数单列。场景 S01 的建索引与条目部分、S03 的旧字节保持、S06、S07、S09。单列等值 EXPLAIN 只维持已经存在的行为，组合与范围的计划形状属于 002。
- Depends on: 000 accepted
- Targets:
  - `src/parser/planner/ddl_dml.rs::build_create_index`
  - `src/executor/plan.rs::CreateIndexNode`，以及 `src/executor/create_index.rs` 的回填和登记
  - `src/storage/data/table_manager.rs` 的 `register_secondary_index`、打开时装载二级索引的循环、`alter_drop_column` 的拒绝和行重写、`narrowed_meta`、`truncate_table` 重建二级元数据、`apply_column_drop`、`publish_column_narrowing`
  - `src/storage/catalog.rs` 中删列发布所改的表行
  - `src/executor/insert.rs`、`update.rs`、`delete.rs`、`upsert.rs` 的二级键循环
  - `src/wal/recovery.rs::extract_index_keys` 和重建 `SecondaryIndexMeta` 的循环
  - `src/transaction/manager.rs` 里调用 `extract_index_keys` 的两处
  - `src/pipeline.rs` 传给计划器的二级索引登记，以及 `src/parser/planner/query.rs::extract_secondary_index_equality`
  - `tests/secondary_index_test.rs::create_unique_and_composite_index_rejected`
  - `tests/string_index_sql_test.rs::string_index_rejection_readonly_and_transaction`
- Current behavior: 多列 `CREATE INDEX` 在计划期被拒绝，文本含 `MULTI-COLUMN INDEX`。新建单列 INT 索引的键是 `secondary_index_key` 的大端字节。目录和内存元数据没有把世代 1 或额外列号写进去，打开时也不读。等值路由对登记的第一列调用 `secondary_index_key`。删列只看第一列。
- Required behavior:
  - 拒绝顺序：既有修饰、索引名缺失或限定名、附加库写、表不存在、列不存在或列位置上的表达式、列类型不支持、重复列、全 INT 且列数大于 4、最后才是索引重名。修饰仍在最前，所以 `CREATE UNIQUE INDEX`、`CONCURRENTLY`、`IF NOT EXISTS`、`USING`、`INCLUDE`、`NULLS DISTINCT`、带 WHERE 的部分索引继续用现在的文本拒绝，即使列数不是 1。
  - 重复列：计划期失败，错误文本含 `duplicate column`，表上不出现新索引。用 `InvalidQuery`，显示文本含 `duplicate column name`。
  - 全 INT 且列数大于 4：`UnsupportedStatement("MULTI-COLUMN INDEX EXCEEDS 32-BYTE KEY")`。错误文本含这串字。表和已有索引无变化。含字符串的组合不受这条约束，列数可以大于 4；放不下的行不入树，语句仍然成功。
  - 两列及以内的 INT 组合、INT 与字符串的组合、新建的单列 INT，都创建成功。
  - 世代：新建单列 INT 是 1，额外列为空。新建或已有的单列字符串是 0，额外列为空。两列及以上、列类型都在 INT 和字符串之内的索引是 1；`column_index` 是第一列，`extra_column_indexes` 是其后的列号，顺序与索引定义一致。升级前已经落盘、没有尾段的单列 INT 索引打开后仍是世代 0。
  - 回填、插入、更新、删除、UPSERT、REPLACE、删列造成的行重写、回滚和恢复重建，都对每个二级索引用 `Value::derive_secondary_index_key`，列类型和值按该索引的完整列序，世代用该索引记录的世代。返回 `None` 的行不入这个索引，不截断。主键和唯一索引仍用 `to_key`。
  - 更新和替换仍用 `delete_exact(旧键, 旧 row id)` 再按新键 `insert`。只改变组合中的一列时，旧键只失去这一行，共享旧键的另一行还在；新键含被更新的行。旧键和新键有一个是 `None` 时，只删除或只插入存在的那一侧。
  - `register_secondary_index` 把计划器给出的世代和额外列号写入目录和内存。打开表、恢复重建、`truncate_table` 换上的新树，都复制来源里的世代和额外列号，不再写死 0。`truncate_table` 只换空树和根页，不改世代。
  - `extract_index_keys` 的二级结果与 `table_meta.secondary_indexes` 一一对应，每个位置用该索引的世代和全部列。主键、唯一列的 `to_key` 结果保持今天的位置含义。
  - 删列：`column_index` 或任一额外列号等于被删位置时，语句失败，文本仍是 `column '{}' is referenced by a secondary index and cannot be dropped (drop the index first)`。列定义和索引都还在，不做行重写。未被引用的列删除成功后，每个存活索引里大于被删位置的列号减一，第一列和额外列都减。`narrowed_meta` 与目录行使用同一规则。
  - 目录列号和 `column_count` 在同一次表行序列化里改完。`publish_column_narrowing` 发现列数已经是目标值时直接返回，不得再次减列号。ADD COLUMN 仍只增加 `column_count`，不减二级列号。
  - 单列等值路由：只选择没有额外列的二级索引。探测键用该索引的世代调用 `derive_secondary_index_key`。世代 0 的结果必须与今天的 `secondary_index_key` 相同。世代 1 的新建单列 INT 索引，`WHERE` 等值的 EXPLAIN 仍含 `IndexScanAll` 和该索引名，行集与建索引前相同。组合索引不进入这条路由；只约束它的某一列时，计划保持扫描，行集仍与扫描相同。不实现组合整键等值和范围计划。
- Preserve: 单列字符串索引的键字节、升级前单列 INT 索引的等值字节、主键和唯一索引的 `to_key`。`CREATE UNIQUE INDEX` 的拒绝文本。只读会话和显式事务里的 `CREATE INDEX` 仍按现在的 sqlstate 拒绝。页类型、32 字节键、叶条目 38 字节、内部分隔 36 字节。
- Forbidden: 修改 `Value::to_key`、`Key` 的比较、页格式。不要把旧的单列 INT 索引改成世代 1，也不要在打开时重写已有索引页。不要实现 `IndexRangeScan`、组合等值 EXPLAIN、补扫或 README。不要为了跨列数的键相等去改编码。
- First test: 新增 `tests/composite_index_lifecycle_test.rs` 的 `composite_backfill_matches_live_rows`。表含 INT 列和字符串列，行里有重复组合、NULL，以及拼接后超过 32 字节的字符串。`CREATE INDEX` 建在这些列上。实现前运行 `cargo test --offline --test composite_index_lifecycle_test composite_backfill_matches_live_rows -- --test-threads=1`，预期失败：多列语句被拒绝。
- GREEN condition: 下面 Verification 里的命令都是 passed、failed 为 0。
- Verification: 见本 Cycle 的 Verification。库测试失败表示条目、世代或列号与本任务不一致。集成测试失败表示 SQL 边界或已有单列等值被改坏。
- Stop when: 不修改 `Key::cmp` 或页格式就无法让同一列清单下的两个不同值得到不同键；或者不把组合索引进现有单列等值路由就无法让 `SELECT` 行集正确。这两种情况都返回计划。

**Invariants**

- 键仍是 32 字节。超长和 NULL 不入树，不截断。
- 同一列清单内，两个都能编码的不同值组合得到的 `Key` 不相等。跨列数时，尾随全零分量可以与更短的键相等；这不在本轮修改。
- 世代 0 单列索引的目录行仍然没有世代尾段。
- 主键和唯一索引的字节不变。

**Non-goals**

组合等值和范围的 EXPLAIN、`IndexRangeScan`、超长行补扫、索引选择、README、全量回归。

**Acceptance**

- 新建组合索引和新建单列 INT 索引后，`scan_all` 的键集合等于对这些存活行调用 `derive_secondary_index_key` 得到的键；NULL 和超长行不在树里，`SELECT` 仍返回它们。
- 改组合中的一列、删除、事务回滚，以及不 `close` 的重开之后，条目与当时存活行一致。
- 新建单列 INT 索引的树使用世代 1 字节。`WHERE` 单列等值的行集与建索引前相同，EXPLAIN 仍点名该索引。单列字符串索引仍是世代 0。
- `CREATE INDEX c ON t(age, id)` 和 `CREATE INDEX idx_multi ON t(name, age)` 成功。`CREATE UNIQUE INDEX` 仍被拒绝。五列全 INT 的文本含 `MULTI-COLUMN INDEX EXCEEDS 32-BYTE KEY`。重复列的文本含 `duplicate column`。
- 被组合索引任一列引用的 `DROP COLUMN` 失败，文本含 `referenced by a secondary index`，列和索引都还在。删掉前面未被引用的列后，重开读到的列号已经减一，世代仍是 1。
- 不跑全量。

**Verification**

这些测试都从 `Database::execute_sql` 进入。条目用该索引的 `search_all` 或 `scan_all` 断言，行用 `SELECT` 的结果断言。

1. `cargo test --offline --test composite_index_lifecycle_test -- --test-threads=1`
   - `composite_backfill_matches_live_rows`
   - `new_single_column_int_index_uses_ordered_key`：负数的树内键等于世代 1 派生，不等于原来的大端 `to_key`；等值查询行集与建索引前相同，EXPLAIN 含 `IndexScanAll` 和索引名。
   - `composite_update_keeps_the_other_duplicate`
   - `composite_rollback_removes_uncommitted_entry`
   - `composite_recovery_matches_live_rows`：`flush_all`、WAL `shutdown`、`drop` 且不 `close`、再 `open`。
   - `drop_column_sees_every_index_column`
   - `create_index_rejects_duplicate_column_and_five_ints`
2. `cargo test --offline --test secondary_index_test create_unique_and_composite_index_rejected -- --test-threads=1`
3. `cargo test --offline --test string_index_sql_test string_index_rejection_readonly_and_transaction -- --test-threads=1`

第 2 条保持测试函数名。其中 `CREATE INDEX c ON t(age, id)` 改为成功，`CREATE UNIQUE INDEX` 仍失败。第 3 条保持测试函数名。其中 `idx_multi` 改为成功；`idx_age` 的等值行集和 EXPLAIN 断言保持原样。失败表示本任务的条目、世代、列号或既有单列等值与要求不一致。

**Gate 2 Readiness**

PASS。计划审查的 11 项均通过，RTM 无 Missing，没有未批准的裁剪。批准来自本次目标对已写 Iteration Plan 的授权，不是跳过。

**Persisted Evidence**

- Mode: none

测试可以重跑。命令、结果和改动的符号写入 Act Response 即可。

**Risks and Notes**

- 000 留下的打开、恢复和 `truncate_table` 写死世代 0。本轮只要创建世代 1 的索引，这三处就必须改成复制真实世代，否则后续写入会把两种编码放进同一棵树。
- 组合索引如果仍以第一列名进入 `extract_secondary_index_equality`，`WHERE` 该列会用单列键去查组合树，行会变少。本轮把组合索引排除出这条路由，是为了保住行集，不是把 002 的组合等值计划提前做完。
- 同一列清单内的编码可以区分不同值。不要为了单列 `1` 与 `(1, i64::MIN)` 相等去改 `Key` 或整数分量。

## Act Response

- Status: 等待实施

**Implemented**

尚未实施。

**Verification Evidence**

无。

**Persisted Evidence**

None required。

## Plan Review

- Review Result: pending

**Findings**

尚未审查实施结果。

**Follow-up Decision**

等待 Act。

**Next Cycle**

None

**Next Iteration**

None
