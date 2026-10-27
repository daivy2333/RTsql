# Iteration 002 / Cycle 000：SQL 声明与等值查询

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks：T6。
- Acceptance gaps：None。
- Repair items：None。
- Inherited scope：R2 的 SQL `CREATE INDEX` 对字符串列放行并回填；R3 的裸列字面量等值、EXPLAIN、投影和容量外扫描；S12 中 DROP INDEX 之后的查询回落；S13 的修饰、缺表、缺列、重名、只读和显式事务拒绝。整数二级索引的现有等值结果保持。
- Excluded scope：T7 的取消、超时、失败写入、干净重开、未决 WAL、旧库、内存库、加密和附加库组合；T8 的全量、用户文档和全局格式门。ISS36。字符串主键、唯一约束、范围路由、参数路由、限定列路由、组合谓词路由。条目宽度、文件头、`Value::to_key` 和 `secondary_index_key` 的编码。

**Objective**

用户可以用 SQL 在 TEXT 或 VARCHAR 列上创建非唯一二级索引。与列类型匹配、并且完整转义不超过 32 字节的裸列字面量等值走该索引；建索引前、建索引后和删索引后的结果行与列元数据一致。超过 32 字节的完整字符串仍可写入，查询走扫描，只返回真实相等的行。

**Investigation Facts**

- Starting point：001 的 Act Response 已实现 `Value::secondary_index_key` 和 `fixed32_string_key`。二级回填、INSERT、UPDATE、DELETE、UPSERT、`alter_drop_column` 和 `extract_index_keys` 的二级分支调用它，插入字节用 `as_bytes()`。`to_key` 仍只接受 Int。accepted Review 沿用 `string_secondary_index_test` 6 passed / 0 failed，以及 `secondary_index_test` 27 passed / 0 failed。该 Review 核对 `ddl_dml.rs` 的类型门仍只放行 Int，`extract_secondary_index_equality` 仍调用 `to_key`，`query.rs` 没有 001 的未暂存 diff。本轮展开前不重跑这两条已通过的测试。工作区另有 000 的评估原型和更早的未提交差异，本轮不得清理。
- 当前 SQL 行为：`build_create_index` 在修饰、表和列检查之后，只放行 `page_format::ColumnType::Int`。失败文本是 `Unsupported statement type: INDEX ON NON-INT COLUMN '{column}' (first version supports INT columns only)`，SQLSTATE `0A000`。`tests/secondary_index_test.rs::create_index_on_non_int_column_rejected` 对 `t(name VARCHAR)` 期望这条拒绝。`tests/string_secondary_index_test.rs` 有两处 `CREATE INDEX` 对字符串列的 `expect_err`，索引本身仍由 `CreateIndexExecutor` 建立。
- 类型映射：`convert_data_type` 把 TEXT、VARCHAR、CHAR、CLOB 都变成执行器 `ColumnType::String`。`ColumnDef::to_schema_column` 再把该类型存成 `String(255)`。`secondary_index_key` 不读取这个长度。BOOLEAN 与 FLOAT 分别是 `Bool` 和 `Float`。
- 等值入口：`extract_secondary_index_equality` 只匹配单个 `Eq`，并且两侧必须是裸 `Identifier` 和字面量。限定名、AND、OR、范围和参数占位返回 `None`。命中后调用 `value.to_key()`。`IndexScanAllNode.columns` 保持基表全列，`projection` 决定输出。`plan_format` 在 `index_name` 为 `Some` 时渲染 `index=<name>`。`ExplainExecutor` 只渲染计划，不执行内层。
- 执行与缓存：`pipeline` 的 `IndexScanAll` 臂把 `node.key.as_bytes()` 交给 `IndexScanAllExecutor`，再按登记名取二级树。CREATE INDEX 与 DROP INDEX 成功或失败后都 `bump_schema_version`，计划缓存被清空。下一条语句经 `register_table_schema` 重新读取 `secondary_indexes` 和逐列类型。
- 容量：`fixed32_string_key` 在原文字节数大于 30 时返回 `None`。30 个非零字节编码为 32 字节，入树。31 个非零字节不入树。001 已接受这条提前返回，本轮不改。叶条目 38 字节，槽 6 字节，页头 16 字节，页 4096 字节，一叶最多 92 条。200 条相同短字符串必然跨叶。
- 错误顺序：UNIQUE、多列、IF NOT EXISTS、表达式和部分索引在类型门之前拒绝。类型门在库级重名检查之前。显式事务内的 CREATE INDEX 由 `execute_stage_in_tx` 返回 `0A000` 和 `CREATE INDEX is not supported inside a transaction`。只读会话先返回 `25006` 和 `cannot execute CREATE INDEX in a read-only session`。这两条都发生在计划已经得到 `CreateIndex` 之后，所以字符串列只有先通过类型门，才会到达它们。

Current-State Evidence 与关键路径：

| 符号 | 已读到的事实与本轮用途 |
|---|---|
| `build_create_index` | 唯一要放宽的声明门。成功后的 `CreateIndexNode` 只有名称、表、列号和列名，没有新字段。回填仍走 001 的 `backfill_tree`。 |
| `extract_secondary_index_equality` | 唯一要更换键来源的查询判定。列名匹配保持现有 `table_secondary_indexes` 查找。类型取 `table_column_types` 里与 `tables` 列清单同序的那一项。 |
| `Value::secondary_index_key` | Int 委托 `to_key`。String 走 fixed32。其他类型和 `Null` 返回 `None`。本轮调用它，不修改它。 |
| `IndexScanAllExecutor` / `pipeline` 的 IndexScanAll 臂 | 已按 `as_bytes()` 搜索登记树。本轮不改。 |
| `get_plan_output_columns` 与 `attach_column_metadata` | `SqlOutcome::Rows.columns` 带列名和声明类型。EXPLAIN 的列名是 `plan`。CLI JSON 的 `columns` 与这个列名清单同源。 |
| `Database::execute_sql` / `execute_with_params` / `execute_in_tx` / `open_readonly` | 库入口。只读和事务拒绝已经按计划形状分类。 |
| CLI `rtsql <裸名> <sql> --format json` | 一次进程一条语句。查询 JSON 是 `columns` 与 `rows`。CREATE INDEX 和 DROP INDEX 返回 `{"affected_rows":0}`。 |

**Implementation Guidance**

类型门的用户可见文本固定为：

`Unsupported statement type: INDEX ON UNSUPPORTED COLUMN '{column}' (only INT and string columns)`

路由得到的 `Key` 原样写入 `IndexScanAllNode.key`。不要改成 `full_data()`，也不要在计划期再编码一次。类型向量缺项或短于列清单时返回 `None`，不 panic，也不回退到 `to_key()`。`Database::execute_sql` 刚建出的字符串列如果拿不到 `String(_)`，停止并返回 Plan。

先写 `string_index_equality_matches_scan_multiset_and_columns` 并看到 RED，再改两个计划函数。之后补本 Cycle 其余测试，并改两处已经会失败的旧断言。

**Task Definitions**

### T6：SQL 声明与等值查询

- Requirement/Scenario：R2 的字符串建索引与 S13 的拒绝；R3 的等值、EXPLAIN、投影、前缀、容量外扫描和非路由谓词；S12 的删索引后结果与列元数据。
- Depends on：None。001 已 accepted，布局和二级派生已在本 Cycle 的 Starting point。
- Targets：`src/parser/planner/ddl_dml.rs::build_create_index`；`src/parser/planner/query.rs::extract_secondary_index_equality`；`tests/secondary_index_test.rs::create_index_on_non_int_column_rejected`；`tests/string_secondary_index_test.rs` 的两处 SQL 拒绝断言和文件头里「仍拒绝字符串列」那句；新文件 `tests/string_index_sql_test.rs`。
- Current behavior：VARCHAR 或 TEXT 的 `CREATE INDEX` 在计划期被拒绝。字符串等值走扫描。整数裸列等值仍走 `IndexScanAll`。超容量字符串可以写入行，但 SQL 建不出字符串索引。
- Required behavior：
  - VARCHAR 与 TEXT 的单列非唯一 `CREATE INDEX` 成功。建索引前记录的等值结果、列名和 `column_type`，在建索引后和 `DROP INDEX` 后保持一致。建索引后的 EXPLAIN 含 `IndexScanAll` 和 `index=<索引名>`。删索引后的 EXPLAIN 含 `DataScan`，且不含 `IndexScanAll`。
  - `name = '字面量'` 与 `'字面量' = name` 都走索引，行多重集相同。样本包含重复值、空串、`你好`、共享前缀的 `pre-a` 与 `pre-b`，以及 001 已使用的 `'ab\0'`。查询一个值不得返回另一个值。
  - `SELECT name, id`、`SELECT name`、`SELECT *` 的列名、声明类型和值与建索引前一致。`COUNT(*)` 的列名清单和计数值也一致，EXPLAIN 仍能看到该索引。`name VARCHAR DEFAULT 'def'` 且插入时省略该列的行，等值查询能返回它，并走索引。`SELECT id, name FROM (SELECT id, name FROM t WHERE name = 'dup') d` 的结果与直接查询一致，EXPLAIN 含 `IndexScanAll`。
  - 30 个非零字节走索引。31 个非零字节的两条只在末字节不同的字符串走 `DataScan`，各自只返回自己的行。把一行从 30 字节改为 31 字节，再改回去之后，两侧查询都与全表中的真实值一致，计划随当前字面量在索引和扫描之间切换。
  - 同一可容纳字符串写入 200 行。EXPLAIN 走该索引，`COUNT(*)` 为 200，行多重集与建索引前相同。
  - `name = NULL`、`name = 1`、`name > 'a'`、`name = 'dup' AND id = 1`、`name = 'dup' OR name = 'pre-a'`、`t.name = 'dup'`，以及 `execute_with_params` 的 `name = ?`，在索引存在时仍不出现 `IndexScanAll`，结果与建索引前相同，也不产生新的计划错误。
  - BOOLEAN 与 FLOAT 的建索引失败，消息等于上面的固定文本并带列名，SQLSTATE 为 `0A000`。失败后原表的整数索引仍可创建，等值结果不变。字符串列上的 `CREATE UNIQUE INDEX` 和多列索引仍是原来的 `Unsupported statement type`。重复的字符串索引名仍包含 `already exists`。
  - 只读库上对字符串列建索引返回 `25006`，消息含 `read-only session`。只读拒绝没有留下索引。显式事务里的字符串建索引返回 `0A000`，消息含 `inside a transaction`。回滚后自动提交的建索引可以成功。
  - 同一张表上的 INT 二级索引，`age = 25` 与 `25 = age` 仍走 `IndexScanAll`，行与建索引前一致。
  - CLI 对同一个库依次完成建表、插入、建索引、EXPLAIN、等值查询、删索引和再次查询。查询 JSON 的 `columns` 与 `rows` 在建前、建后和删后一致。建索引与删索引的 stdout 是 `{"affected_rows":0}`，退出码为 0。建索引后的 EXPLAIN 含索引名；删索引后不含 `IndexScanAll`。
- Preserve：`secondary_index_key`、`to_key`、叶 38 字节、内部 36 字节、页类型 0x01/0x02、`FORMAT_VERSION`。`IndexScanAll.columns` 仍是基表全列。pipeline 仍传 `as_bytes()`。CREATE/DROP INDEX 仍清空计划缓存。PK 与唯一索引仍走 `to_key`。`string_secondary_index_test` 仍用 `CreateIndexExecutor` 建索引，仍是 6 个测试。`secondary_index_test` 仍是 27 个测试。
- Forbidden：修改 `value.rs`、`node.rs`、`key.rs`、`integrity.rs`、`file_header.rs`、`index_scan_all.rs`、`pipeline.rs` 和 `create_index.rs`。放行 UNIQUE、多列、表达式、部分索引、参数、限定列、AND、OR 或范围。截断长字符串，或拒绝长字符串的行写入。把评估原型编进 `src/`。清理无关工作区差异。运行全量、Criterion、全局 `cargo fmt --check` 或全局 clippy。处理 ISS36。创建 003 的目录。
- First test：新增 `string_index_equality_matches_scan_multiset_and_columns`。表含 VARCHAR 列、TEXT 列和 INT 列。建索引前保存等值行、列名和类型。第一条失败点是 VARCHAR 的 `CREATE INDEX` 返回 `Unsupported statement type`。这是运行期 RED，不是编译失败。
- GREEN condition：该测试以及 Verification 列出的新测试全部通过。30 字节走 `IndexScanAll`，31 字节走 `DataScan`。200 行的 `COUNT(*)` 为 200。BOOLEAN 与 FLOAT 被拒绝后，整数索引的查询行数不变。`secondary_index_test` 为 27 passed / 0 failed。`string_secondary_index_test` 为 6 passed / 0 failed。
- Verification：改计划门之前，只运行第一条命令。预期失败点是 VARCHAR 的 `CREATE INDEX` 返回 `Unsupported statement type`。改完计划门和旧断言后，运行后三条，每条都是 0 failed。不跑无目标过滤的全量。

```sh
cargo test --offline --test string_index_sql_test string_index_equality_matches_scan_multiset_and_columns -- --test-threads=1
cargo test --offline --test string_index_sql_test -- --test-threads=1
cargo test --offline --test secondary_index_test -- --test-threads=1
cargo test --offline --test string_secondary_index_test -- --test-threads=1
```

新文件最终为 8 passed / 0 failed。既有两个文件的通过数保持 27 和 6。

- Stop when：可容纳字符串必须修改 `secondary_index_key`、条目宽度或 `Key` 才能被 SQL 找到；31 字节字符串必须被拒绝写入或截断才能让扫描断言通过；TEXT 或 VARCHAR 的存储类型不是 `String(_)`；只读或事务拒绝只有改掉 `25006` 或 `0A000` 的现有文案才能通过；为了让限定列、参数或范围走索引才能通过测试。返回 Plan。

**Invariants**

- 叶条目 38 字节，内部分隔条目 36 字节，页类型 0x01 与 0x02，`FORMAT_VERSION` 保持 1。
- 计划期使用 `secondary_index_key` 的结果。编码长度超过 32 字节的字符串不进入 `IndexScanAll`。
- NULL 不入二级索引。`name = NULL` 保持建索引前的结果。
- `Value::to_key` 仍只接受 Int。整数二级索引的查找字节不变。
- 新测试各自使用临时目录。CLI 测试预建 `RTSQL_HOME/db`，实参用裸库名，`stdin` 设为 null，移除 `RTSQL_KEY`，并读完 stdout。
- 全量仍只在 T8 执行一次，按 M19 分批，每批 `--test-threads=1`。本 Cycle 不消耗这次预算。

**Non-goals**

取消、超时、崩溃恢复、旧库、内存库、加密库、附加库、用户文档、全局 fmt 与 clippy、全量测试、ISS36，以及范围、参数、限定列和组合谓词的新路由。没有需要人工操作的验证项。

**Acceptance**

- A1 / R2 / T6：VARCHAR 与 TEXT 建索引成功。等值行、列名和声明类型在建前与建后一致。EXPLAIN 点名该索引。
- A2 / R3 / T6：左右两种等值、重复值、空串、UTF-8、尾部零字节和共享前缀只返回真实相等的行。
- A3 / R3 / T6：单列、换序多列、星号、`COUNT(*)`、DEFAULT 省略列和派生查询的结果与建索引前一致。派生查询和聚合的 EXPLAIN 能看到索引。
- A4 / R3 / T6：30 字节走索引。两条 31 字节长字符串走扫描，互不混入。跨过 32 字节边界的更新之后，两侧查询与表中的值一致。
- A5 / R3 / T6：200 行相同短字符串的等值结果与建索引前相同，计划是 `IndexScanAll`，计数为 200。
- A6 / S12 / T6：DROP INDEX 后结果和列元数据与建索引前相同，EXPLAIN 回到 `DataScan`。
- A7 / R3 / T6：NULL、类型不匹配、范围、AND、OR、限定列和参数不走索引，结果不变。
- A8 / S13 / T6：BOOLEAN、FLOAT、UNIQUE、多列和重名仍被拒绝。只读为 `25006`，显式事务为 `0A000`。拒绝之后可以再成功创建索引。INT 等值仍走索引。
- A9 / T6：CLI 的建索引、查询、EXPLAIN 和删索引与 A1、A6 的可观察结果一致，退出码为 0。
- A10 / T6：`secondary_index_test` 27 passed / 0 failed，`string_secondary_index_test` 6 passed / 0 failed。

**Verification**

判定看 `SqlOutcome` 的列名、`column_type` 和行，EXPLAIN 文本里的 `IndexScanAll`、`index=<名称>` 或 `DataScan`，错误消息与 SQLSTATE，以及 CLI 的退出码和 JSON。行比较按多重集，含 `id` 的结果按 `id` 排序后比较。

`tests/string_index_sql_test.rs` 的测试名：

- `string_index_equality_matches_scan_multiset_and_columns`
- `string_index_projection_aggregate_default_and_derived`
- `string_index_prefix_nul_capacity_and_cross_boundary_update`
- `string_index_multileaf_duplicates_match_scan`
- `string_index_drop_restores_scan_metadata`
- `string_index_unrouted_predicates_stay_scan`
- `string_index_rejection_readonly_and_transaction`
- `cli_string_index_create_query_explain_and_drop`

第一个测试同时覆盖 VARCHAR、TEXT，以及同一库中 INT 列 `age = 25` 的既有路由。容量测试里的 31 字节样本是 30 个 `a` 再加一个不同的末字节。多叶测试插入 200 行，不使用 30000 行的整数回归。

CLI 每个进程只执行一条 SQL。`RTSQL_HOME` 指向临时目录根，并事先创建 `db/`。命令形如 `rtsql <裸名> <sql> --format json`。

001 的库级测试和 `secondary_index_test` 在本轮改动前的 6 passed 与 27 passed 继续有效，来源是 001 的 Act Response 与 accepted Review。改动旧断言之后必须重跑这两个文件。未改 `value.rs` 时不重跑 `secondary_index_key_preserves_int_bytes`、`string_secondary_key_preserves_full_value_within_32` 和 `string_key_uses_existing_tree_entry_width`。不重跑 Criterion。不跑全量。

**RTM**

| Requirement | Task | First Test | Status |
|---|---|---|---|
| R2 SQL 字符串建索引 | T6 | `string_index_equality_matches_scan_multiset_and_columns` | Covered |
| R3 等值、EXPLAIN、容量与投影 | T6 | `string_index_equality_matches_scan_multiset_and_columns` | Covered |
| S12 删索引后的结果与列元数据 | T6 | `string_index_drop_restores_scan_metadata` | Covered |
| S13 拒绝、只读和事务 | T6 | `string_index_rejection_readonly_and_transaction` | Covered |

R4 的取消和超时、R5 的重开与其余运行形态属于 T7。R6 的全量属于 T8。没有 Simplified。

**Gate 2 Readiness**

PASS。没有 BLOCKED，没有 SKIPPED。类型门、错误文本、路由的 `None` 与扫描语义、容量边界和验证入口已经闭合。批准来自本次执行 goal，不是跳过 Gate 2。

**Persisted Evidence**

- Mode: none

测试命令、通过计数、EXPLAIN 中的节点名和 CLI 退出码写入 Act Response 即可。这些测试可以重跑，没有不可再生的现场。不创建 evidence 目录。

**Risks and Notes**

- 计划期若把 `full_data()` 放进节点，pipeline 会把它当作 32 字节查找键。空串碰巧仍是全零，其他含终止符 `00 00` 的键会在尾部零被截断后互相撞上。发现这种失败时改回 `secondary_index_key` 返回的 `Key`，不改 `Key::deserialize`。
- `fixed32_string_key` 对长于 30 字节的原文提前返回 `None`。这是 001 已接受的等价行为。30 字节非零仍入树，31 字节非零仍扫描。
- `register_table` 不重新小写化列名。列类型按下标读取，位置与 `set_table_column_types` 的向量一致。不要为了路由去改登记函数。
- 工作区里的评估原型和更早差异都保留。ISS36 不在本轮处理。
- 本轮调查没有发现需要另建 Iteration 的范围外实质缺陷。没有新登记的 Issue。

## Act Response

- Status: reported

**Implemented**

`PlanBuilder::build_create_index` 对 `ColumnType::Int` 和 `ColumnType::String(_)` 放行。缺类型以及其他列类型返回 `INDEX ON UNSUPPORTED COLUMN '{column}' (only INT and string columns)`。`PlanBuilder::extract_secondary_index_equality` 按列清单下标读取声明类型，调用 `Value::secondary_index_key`，把返回的 `Key` 原样放进 `IndexScanAllNode.key`。类型向量缺项或短于列清单时返回 `None`，不回退到 `to_key`。

改动文件：

- `src/parser/planner/ddl_dml.rs`
- `src/parser/planner/query.rs`
- `tests/string_index_sql_test.rs`（新文件，8 个测试）
- `tests/secondary_index_test.rs`：`create_index_on_non_int_column_rejected` 改为拒绝 BOOLEAN 列
- `tests/string_secondary_index_test.rs`：两处 SQL 拒绝改为建索引成功后删除，观察用的索引仍由 `CreateIndexExecutor` 建立

未改 `value.rs`、`node.rs`、`key.rs`、`integrity.rs`、`file_header.rs`、`index_scan_all.rs`、`pipeline.rs`、`create_index.rs`。未执行 T7、T8，未创建下一轮。

**Deviations from Plan**

- CLI 主命令对不存在的库直接拒绝，文案要求先执行 `rtsql new`。测试仍预建 `RTSQL_HOME/db`，并在 SQL 序列前增加一次 `rtsql new sidx`。建表、插入、建索引、EXPLAIN、等值查询、删索引和再次查询仍是每进程一条 SQL。
- `execute_sql` 看到的 BOOLEAN 与 FLOAT 拒绝消息是既有 `Plan error: ` 前缀加上规定的类型门文本。测试断言完整消息相等。`pipeline.rs` 未改。

**Self-review**

Gate 3 没有遗留的 Critical 或 Important 问题。先行测试先在 VARCHAR 的 `CREATE INDEX` 上失败，失败消息是旧的 `INDEX ON NON-INT COLUMN`。计划门改完后，库测试与 CLI 测试覆盖了本轮 Acceptance。没有 Minor 遗留。

**Verification**

改计划门之前：

```sh
cargo test --offline --test string_index_sql_test string_index_equality_matches_scan_multiset_and_columns -- --test-threads=1
```

结果：0 passed / 1 failed。失败点是 `CREATE INDEX idx_name ON t(name)`，消息为 `Plan error: Unsupported statement type: INDEX ON NON-INT COLUMN 'name' (first version supports INT columns only)`。VARCHAR 与 TEXT 的声明类型已是 `String(_)`。

改完计划门、新测试和旧断言后：

```sh
cargo test --offline --test string_index_sql_test -- --test-threads=1
```

首跑 7 passed / 1 failed。失败点是 CLI 主命令：`database 'sidx' does not exist`。补上 `rtsql new` 后重跑，8 passed / 0 failed。

```sh
cargo test --offline --test secondary_index_test -- --test-threads=1
```

27 passed / 0 failed。

```sh
cargo test --offline --test string_secondary_index_test -- --test-threads=1
```

6 passed / 0 failed。

未跑全量，未跑全局 fmt 或 clippy。

**Persisted Evidence**

None required。

**Experience Candidates**

None。

**Unresolved**

Review Result 仍为 pending。没有当前轮范围外的实质缺陷需要作为 Issue 候选报告。T7、T8 未执行。

## Plan Review

- Review Result: accepted

**Findings**

阻塞 Acceptance：0。非阻塞：2。

1. CLI 准备步骤（Act 偏离，不阻塞 A9）。`cli_string_index_create_query_explain_and_drop` 在 SQL 序列前执行一次 `rtsql new sidx`。计划写的准备是预先创建 `RTSQL_HOME/db`，后续命令形如 `rtsql <裸名> <sql> --format json`。当前 CLI 对尚未 `new` 的库直接拒绝。测试仍预建 `db/`，去掉 `RTSQL_KEY`，`stdin` 为 null，并读完 stdout。建表、插入、建索引前查询、建索引、EXPLAIN、建索引后查询、删索引、删后 EXPLAIN 和删后查询仍是每进程一条 SQL。建索引与删索引的 stdout 是 `{"affected_rows":0}`，退出码为 0。查询 JSON 的 `columns` 与 `rows` 在建前、建后和删后一致。产品代码没有为这条偏差改动。
2. 类型门消息前缀（现状，不阻塞 A8）。`execute_sql` 返回的 BOOLEAN 与 FLOAT 拒绝消息是 `Plan error: ` 加上计划固定的 `Unsupported statement type: INDEX ON UNSUPPORTED COLUMN '{column}' (only INT and string columns)`。前缀来自 `pipeline.rs` 的 `From<PlanError> for PlanFailure`。该文件本轮未改，mtime 为 2026-10-26 20:25。测试对完整消息做相等断言，SQLSTATE 为 `0A000`，列名在正文里。类型门正文与计划固定文本一致。

**Evidence**

独立阅读的本轮 diff 只覆盖 Act 点名的五处。`ddl_dml.rs`、`query.rs`、`secondary_index_test.rs` 的未暂存差异是类型门、`secondary_index_key` 路由和 BOOLEAN 拒绝断言。`tests/string_index_sql_test.rs` 是新文件。`tests/string_secondary_index_test.rs` 未跟踪；文件头写明 SQL 接受字符串列，两处 `CREATE INDEX` 成功后 `DROP INDEX`，观察用的索引仍由 `CreateIndexExecutor` 建立，测试函数 6 个。

`PlanBuilder::build_create_index` 在修饰、多列和表达式拒绝之后，对 `ColumnType::Int` 与 `ColumnType::String(_)` 放行。缺类型与其余类型返回规定正文。`PlanBuilder::extract_secondary_index_equality` 按列清单下标读取声明类型，调用 `Value::secondary_index_key`，把返回的 `Key` 原样写入 `IndexScanAllNode.key`。类型向量缺项或短于列清单时返回 `None`。限定名、AND、OR、范围和占位符在取键前返回 `None`。`extract_pk_from_where` 仍调用 `to_key`。

八个测试名与计划清单一致，断言覆盖 A1–A9：VARCHAR 与 TEXT 建索引、两侧字面量、重复值、空串、`你好`、`ab\0`、共享前缀、同一表上的 INT 等值、投影、`COUNT(*)`、DEFAULT、派生查询、30 字节 `IndexScanAll`、31 字节 `DataScan`、跨边界 UPDATE、200 行、DROP 后的列元数据与 `DataScan`、未路由谓词、BOOLEAN、FLOAT、UNIQUE、多列、重名、只读 `25006`、事务 `0A000` 和 CLI JSON。`secondary_index_test` 有 27 个 `#[tokio::test]`。

验证沿用本 Cycle Act Response。改门前 `string_index_equality_matches_scan_multiset_and_columns` 为 0 passed / 1 failed，失败消息是旧的 `INDEX ON NON-INT COLUMN`。补上 `rtsql new` 后 `string_index_sql_test` 为 8 passed / 0 failed，`secondary_index_test` 为 27 passed / 0 failed，`string_secondary_index_test` 为 6 passed / 0 failed。代码、diff 与这些断言一致。覆盖文件在该结论之后没有再改：审查时 mtime 仍是 16:03–16:12，没有进程正在写这些文件。未跑全量、全局 fmt 或 clippy。Persisted Evidence 为 none。

001 与更早差异保留。`src/executor/value.rs`、`create_index.rs`、insert、update、delete、upsert、`index_manager.rs`、`table_manager.rs`、`recovery.rs` 的 mtime 在 15:26 之前。`pipeline.rs`、`index_scan_all.rs`、`integrity.rs`、`file_header.rs` 更早。本轮没有清理它们。

**Follow-up Decision**

A1–A10 有对应代码路径和测试断言，沿用的验证与当前 diff 一致。两处非阻塞差异留在本 Review。当前 Cycle 不再修改。Iteration Plan 保持不变。

`tasks.md` 里的 Iteration 003 仍是后续路线。本次派发写明：「若结论为 accepted，不要在本次运行里展开 Iteration 003。」因此不创建 `iterations/003-*`。003 要等后续规划展开后才能交给 Act。

**Next Cycle**

None

**Next Iteration**

Iteration 003：故障边界、运行形态与收尾（T7、T8）。目录未创建。
