# Iteration 001 / Cycle 001: 写出的整数主键才选择排序合并

## Plan Context

- Status: ready
- Cycle Type: replan

**Cycle Scope**

- Change tasks: T2
- Acceptance gaps: 父 Cycle 里 `equi_on_keeps_hash_join_shape` 与 `outer_join_routes_to_nested_loop_join` 的内连接变成了排序合并。修订后的完成条件见本 Cycle 的 Acceptance。不再要求「目录里登记了整数键列」就走排序合并。
- Repair items: None
- Inherited scope: Iteration 000 已交付的 `SortMergeJoinExecutor` 与 `PhysicalPlan::SortMergeJoin`。父 Cycle 已经写上的、两侧都写出整数 `PRIMARY KEY` 的语句测试和 `explain_compound_nodes` 的 `SortMergeJoin` 断言继续有效。纯等值内连接在两侧都是基表 `Scan`、且每个连接键都是该表写出的整数 `PRIMARY KEY` 时，发出排序合并节点。多重集合与同一输入上的哈希连接相同。`ORDER BY` 不参与选择，连接之上的 Sort 保留。
- Excluded scope: T4 的负零键归一、T3 的较小构建侧与时间门、4.2 的全量回归、I016 的代价模型与连接重排、I065、NaN 排序比较、`Value::Hash`、ISS39、ISS40、ISS36 及其他 DDL 缺陷。不改加列变更目录，不改 Iteration 000 的 accepted Review，不改 `.agents/memory/references.md`。不改 `create_table_sql` 的 `PRIMARY KEY` 渲染。不改变隐式首列仍写入 `pk_column` 的建表语义。

**Objective**

两侧都是基表扫描，且每个连接键都是该表写出的整数 `PRIMARY KEY` 时，`execute_sql` 发出排序合并节点，结果多重集合与哈希连接相同。没有写出 `PRIMARY KEY` 的整数首列仍走哈希。字符串主键不满足。外连接和非等值仍走嵌套循环。其余纯等值仍走哈希。

**Background**

父 Cycle `000-initial.md` 的 Act Response 为 `blocked`。它按当时的 Task Definition，用 `primary_keys` 列名和 `primary_key_types == Int` 选择排序合并。`CreateTableExecutor` 在 `CreateTableNode.primary_key` 为 `None` 时仍把第一列写入 `pk_column`，所以 `CREATE TABLE r (a INT)` 与 `CREATE TABLE r (rid INT, val VARCHAR)` 也被登记成整数主键。`equi_on_keeps_hash_join_shape` 与 `outer_join_routes_to_nested_loop_join` 因此退出码 101。已写出整数 `PRIMARY KEY` 的语句测试通过。执行器没有改。

用户随后收窄触发条件：只有写出 `PRIMARY KEY` 的整数主键才走排序合并。隐式首列整数键仍走哈希。上述两条锚点保持哈希或嵌套循环。这改变 T2 的完成条件，父 Cycle 的 Task Definition 约束不了新的判定，所以本 Cycle 是 replan，不要求 Act 在父 Cycle 上继续。

**Investigation Facts**

- Starting point: 父 Cycle 的产品改动仍在工作区。`integer_pk_base_scan_join` 已接在纯等值内连接臂上。父 Cycle 的验证按它的 Act Response 沿用：`cargo test --test sort_merge_join_test` 在存量字符串主键改法之后退出码 0，12 passed；`explain_compound_nodes` 退出码 0；`equi_on_keeps_hash_join_shape` 与 `outer_join_routes_to_nested_loop_join` 退出码 101，实际内连接节点是 `SortMergeJoin`；`non_equi_on_routes_to_nested_loop_join` 以及四条既有结果用例退出码 0。本次重读了下面的计划期和建表路径，没有重跑这些测试，没有跑全量。`src/executor/join.rs`、`src/executor/value.rs`、`src/executor/value_ref.rs` 相对 HEAD 无未提交差异。`src/executor/sort_merge_join.rs` 与 `tests/sort_merge_join_test.rs` 是未跟踪文件。`src/pipeline.rs`、`src/storage/catalog.rs`、`src/storage/data/table_manager.rs` 另有与本判定无关的未提交改动，本 Cycle 只在这些文件里加显式主键位，不回退那些改动。
- Current-State Evidence:
  - 写出与否只存在于建表计划。`PlanBuilder::extract_primary_key`（`src/parser/planner/ddl_dml.rs`）收集列级 `ColumnOption::Unique { is_primary: true }` 和表级 `TableConstraint::Unique { is_primary: true }`。零个候选返回 `Ok(None)`。一个候选返回 `Ok(Some(列名))`，列名已小写。多于一个返回 `PlanError::MultiplePrimaryKey`。`build_create_table` 把这个 `Option` 放进 `CreateTableNode.primary_key`（`src/executor/plan.rs`）。`Some` 且该列的 `data_type != ColumnType::Int` 时，计划期拒绝，错误是 `PlanError::UnsupportedConstraint("NON-INTEGER PRIMARY KEY")`。`None` 不走这道拒绝。
  - `CreateTableExecutor`（`src/executor/create_table.rs`）在 `node.primary_key` 为 `Some` 时用该列名；为 `None` 时用第一列，没有列时用 `"id"`。然后调用 `TableManager::create_table_with_constraints(name, columns, pk)`。这个函数当前只有表名、列和 `pk: &str`，没有「是否写出」参数。`pk_column` 因此对两种表都是非空列名。
  - `CatalogRow`（`src/storage/catalog.rs`）和 `TableMeta`（`src/storage/data/table_manager.rs`）只保存 `pk_column` 与 `pk_index`。`serialize_catalog_row` 的最后一段是 kind 字节；kind 为 View 时其后还有 `u16 sql_len | sql_bytes`。没有显式主键字节。`deserialize_catalog_row` 在 kind 段缺失时把行读成 `CatalogKind::Table`。
  - `register_table_schema`（`src/pipeline.rs`）调用 `register_table(key, columns, &table_meta.pk_column)`。列名能在列清单里找到时再 `set_pk_column_type`。`PlanBuilder.primary_keys` 与 `primary_key_types` 没有第三项。`integer_pk_base_scan_join`（`src/parser/planner/query.rs`）为真的条件是：条件非空，左右都是 `PhysicalPlan::Scan`，两侧 `primary_key_types` 都是 `Some(ColumnType::Int)`，且每条条件的列名等于该侧 `primary_keys` 中的列名。限定列的表名必须等于该侧 `ScanNode.table_name`，裸名列按该侧扫描表检查。空字符串主键不成立。它不读 `CreateTableNode.primary_key`。
  - 因此 `CREATE TABLE r (a INT)` 的登记主键是 `a`、类型 `Int`；`CREATE TABLE r (rid INT, val VARCHAR)` 的登记主键是 `rid`、类型 `Int`。`tests/nested_loop_join_test.rs` 的 `equi_on_keeps_hash_join_shape` 用的就是前一种表，语句 `r.a = s.b`，断言 `PhysicalPlan::Join`。`tests/outer_join_test.rs` 的 `seed_left_joined` 建 `l(id INT PRIMARY KEY)` 与 `r(rid INT, val VARCHAR)`；同文件 `outer_join_routes_to_nested_loop_join` 末尾断言 `l.id = r.rid` 的内连接是 `PhysicalPlan::Join`。外连接三态在该测试前半段断言 `NestedLoopJoin`。父 Cycle 记录的失败点是这个内连接实际为 `SortMergeJoin`。
  - `tests/explain_test.rs` 的 `explain_compound_nodes` 建 `l(id INT PRIMARY KEY)` 与 `r(rid INT PRIMARY KEY)`，语句是 `l.id = r.rid`，并带 `WHERE`、`ORDER BY` 和 `LIMIT`。断言已经改成某一行去掉缩进后以 `SortMergeJoin` 开头且包含 `l`，排序行以 `Sort (` 开头。两侧都写出了整数 `PRIMARY KEY`，收窄之后这条断言仍然成立。
  - CTAS 在 `src/pipeline.rs` 用第一列当 `pk`，注释写明未声明主键时首列是隐式主键。这里没有 `PRIMARY KEY` 子句。`TableManager::create_table` 转调 `create_table_with_constraints`，同样没有写出位。
  - `create_table_sql`（`src/cli/lifecycle.rs`）对每个整数 `pk_column` 渲染 `PRIMARY KEY`，包括隐式首列。本 Cycle 不改这个函数。dump 再 restore 之后，恢复语句里写了 `PRIMARY KEY`，`extract_primary_key` 会返回 `Some`。那是现有 schema 文本的结果。
  - 重新组装 `TableMeta` 的生产路径会丢掉新字段，除非逐处复制：`narrowed_meta`、`open_or_init`、`create_table_with_constraints`、`alter_add_column`、`replace_index_manager`、`replace_recovery_indexes`、`register_secondary_index`、`take_secondary_index`。目录行的 update 先 `deserialize_catalog_row` 再 `serialize_catalog_row`，字段留在结构体上就会跟着走。`insert_view_row` 自己构造 `CatalogRow`。
  - 查询内存、`IN` 子查询拒绝文案、`EXPLAIN` 行文本与父 Cycle 已经验证过的语句测试相同。`IN` 拒绝文案仍是 `IN subquery with JOIN is not supported`。多列误报仍是 `Subquery returns multiple columns (IN subquery requires single column)`。上限 16 时 `execute_sql` 的 `sqlstate` 为 `Some("53200")`，`message` 为 `Execution error: query memory limit exceeded (limit 16 bytes)`。
- Code and Critical Path: SQL 建表时，`extract_primary_key` 的 `Option` 进入 `CreateTableNode.primary_key`。执行器把 `is_some()` 交给 `create_table_with_constraints` 的新参数，写入 `TableMeta.explicit_primary_key` 和 `CatalogRow.explicit_primary_key`。打开库时 `open_or_init` 把目录位抄回 `TableMeta`。`register_table_schema` 在该位为真时调用 `PlanBuilder::set_explicit_primary_key`。查询时 `integer_pk_base_scan_join` 在原有四条之外还要求左右扫描表都在 `explicit_primary_keys` 里。缺一则仍构造 `PhysicalPlan::Join`。

**Implementation Guidance**

显式位是 `bool`。名字用 `explicit_primary_key`。`true` 只表示建表 SQL 写出了 `PRIMARY KEY`。

`create_table_with_constraints` 增加最后一个参数 `explicit_primary_key: bool`。`CreateTableExecutor` 传入 `node.primary_key.is_some()`，并且继续在 `None` 时把第一列写入 `pk`。`TableManager::create_table`、CTAS 路径，以及其余现有调用，都传入 `false`。不要从 `pk` 非空推断这个参数。

`CatalogRow` 与 `TableMeta` 增加同名字段。序列化在 kind 段之后追加一个 `u8`：View 行先写完 SQL 再追加。`1` 为真，`0` 为假。反序列化时，kind 与 View 的 SQL 都消费完之后若还有一个字节且值为 `0` 或 `1`，用该字节；没有这个字节时为 `false`。不要因为缺字节拒绝打开旧库。`insert_view_row` 写 `false`。上面列出的 `TableMeta` 重组从旧值复制该字段。

`PlanBuilder` 增加 `explicit_primary_keys: HashSet<String>`，键与 `primary_keys` 一样小写。`register_table` 的签名不变，不在里面插入这个集合。新增 `set_explicit_primary_key`。`register_table_schema` 只在 `table_meta.explicit_primary_key` 为真时调用它。`PlanBuilder::new` 把集合建成空的。

`integer_pk_base_scan_join` 保留现有四条，再要求左右 `ScanNode.table_name` 都在 `explicit_primary_keys` 里。两侧都要在。`l` 写出了主键而 `r.rid` 没有写出时，整步仍是哈希。不要用 `pk_type_known_non_int` 的否定。`build_from_clause_with_projection` 上方注释里的「已声明整数主键」改成写出的整数 `PRIMARY KEY`。

`src/storage/catalog.rs` 里 `legacy_row_without_kind_section_reads_as_table` 现在 `pop()` 掉的是 kind 字节。尾字节加上之后，一次 `pop()` 只会去掉显式主键字节。这个测试要截掉显式主键字节和 kind 字节，读回仍是 `CatalogKind::Table`、无 SQL，且 `explicit_primary_key` 为 `false`。`view_row_with_truncated_sql_section_tolerates_as_table` 要截进 SQL 正文，而不是只截掉新的尾字节；截断后仍读成 Table。另加一行往返：`explicit_primary_key == true` 的表行写回后仍为 `true`。缺尾字节的当前 kind 布局读回为 `false`，kind 仍是 Table。

**Task Definitions**

### T2: 写出的整数主键基表扫描发出排序合并

- Requirement/Scenario: 「整数主键基表扫描走排序合并」的三个场景，加上重新打开后仍走排序合并；「纯等值 JOIN Hash 路径保持」里的无写出主键、非主键列、字符串主键、连接结果再连接，加上重新打开后隐式首列仍走哈希；「外连接与非等值仍走嵌套循环」；「IN 子查询继续拒绝连接形状」；「排序合并节点可被计划文本识别」的语句文本；「排序合并连接遵守查询内存上限」的语句入口
- Depends on: T1
- Targets: `src/executor/create_table.rs` 的 `CreateTableExecutor`；`src/storage/data/table_manager.rs` 的 `create_table_with_constraints`、`create_table`、`open_or_init`、`narrowed_meta`、`alter_add_column`、`replace_index_manager`、`replace_recovery_indexes`、`register_secondary_index`、`take_secondary_index`；`src/storage/catalog.rs` 的 `CatalogRow`、`serialize_catalog_row`、`deserialize_catalog_row`、`insert_view_row`；`src/pipeline.rs` 的 `register_table_schema` 与 CTAS 建表调用；`src/parser/planner/mod.rs` 的 `PlanBuilder`；`src/parser/planner/query.rs` 的 `integer_pk_base_scan_join`；`tests/sort_merge_join_test.rs`；`src/storage/catalog.rs` 的目录行测试
- Current behavior: 纯等值内连接在两侧都是基表扫描、条件非空、列名等于 `primary_keys`、类型为已登记 `Int` 时构造 `PhysicalPlan::SortMergeJoin`。`CREATE TABLE r (a INT)` 与 `CREATE TABLE s (b INT)` 的 `r.a = s.b` 满足这些条件。`l(id INT PRIMARY KEY)` 与 `r(rid INT, val VARCHAR)` 的 `l.id = r.rid` 也满足。写出整数 `PRIMARY KEY` 的两侧等值同样满足。外连接和非等值是嵌套循环。目录行没有「是否写出 PRIMARY KEY」的字段。
- Required behavior: 下面五条同时成立时，该连接节点是 `PhysicalPlan::SortMergeJoin`，`EXPLAIN` 去掉缩进后有一行以 `SortMergeJoin` 开头，并含等值条件。任一条件不成立时，该步仍是 `PhysicalPlan::Join`，对应行以 `Join [` 开头。
  - 左右子计划都是 `PhysicalPlan::Scan`。
  - 条件列表非空，每条左列等于左表 `primary_keys` 里的列名，右列等于右表的列名。限定列的表名等于该侧 `ScanNode.table_name`。裸名列按该侧扫描表检查。
  - 两侧 `primary_key_types` 都是 `Some(ColumnType::Int)`。
  - 两侧表都在 `PlanBuilder.explicit_primary_keys` 里。这个集合只来自 `TableMeta.explicit_primary_key == true`。该布尔只在 `CreateTableNode.primary_key.is_some()` 时为真，也就是 `extract_primary_key` 返回了一个列名。`CREATE TABLE r (a INT)`、`CREATE TABLE r (rid INT, val VARCHAR)`、CTAS、`create_table` 和 `create_table_with_constraints` 的既有调用都是 `false`。缺尾字节的旧目录行读回也是 `false`。
  - 关闭数据库再打开后，上面的布尔仍是建表时写入的值。写出整数 `PRIMARY KEY` 的两表等值在重新打开后仍是排序合并。没有写出 `PRIMARY KEY` 的 `r(a INT)` 与 `s(b INT)` 在重新打开后仍是哈希。
  - 语句结果：`customers(id INT PRIMARY KEY)` 与 `orders(id INT PRIMARY KEY)`，`customers.id = orders.id`。插入 customers 的 1、2 和 orders 的 2、3。无 `ORDER BY` 的结果多重集合是一行 `(2, 2)`。
  - 自连接：`customers a JOIN customers b ON a.id = b.id`。插入 1 和 2。结果多重集合是 id 1 与 id 2。`EXPLAIN` 条件文本是 `customers.id = customers.id`。
  - `ORDER BY customers.id` 的同一连接：连接节点仍是排序合并，其上仍有 Sort。Sort 行去掉缩进后以 `Sort (` 开头，出现在 `SortMergeJoin` 行之前，缩进更浅。结果次序是 `ORDER BY` 的次序。
  - 语句内存：两张单列整数主键表各一行，`set_query_memory_limit(16)` 后执行该连接。`execute_sql` 返回错误，`sqlstate` 是 `Some("53200")`，`message` 是 `Execution error: query memory limit exceeded (limit 16 bytes)`。没有成功结果行。这两张表的主键都是写出的 `PRIMARY KEY`。
  - `orders(id INT PRIMARY KEY, customer_id INT, amount INT)` 与 `customers(id INT PRIMARY KEY, name STRING)`，`orders.customer_id = customers.id`，计划仍是哈希。同一测试里再执行 `customers.id = orders.id AND customers.id = orders.customer_id`，因为 `customer_id` 不是 orders 的主键，整步仍是哈希。
  - 两侧存量字符串主键的等值内连接仍是哈希。继续用 `create_table_with_constraints` 建表，最后一个参数为 `false`。不要改回 `CREATE TABLE ... TEXT PRIMARY KEY`。
  - 三张写出 `id INT PRIMARY KEY` 的基表按主键连成三表内连接。`EXPLAIN` 有一行 `SortMergeJoin`，也有一行 `Join [`。第二步不是排序合并。
  - 外层 `IN` 使用上述写出的整数主键连接作为子查询。语句失败，错误含 `IN subquery with JOIN is not supported`，并且不含 `Subquery returns multiple columns`。
  - `equi_on_keeps_hash_join_shape` 的计划保持 `PhysicalPlan::Join`。`outer_join_routes_to_nested_loop_join` 的外连接保持嵌套循环，其中 `l.id = r.rid` 的内连接保持 `PhysicalPlan::Join`。`non_equi_on_routes_to_nested_loop_join` 保持嵌套循环。
- Preserve: 隐式首列仍然写入 `pk_column`，索引、唯一性和 `ON CONFLICT` 的既有仲裁不改。嵌套循环的算法与外连接结果。哈希连接仍先构建右输入。`SortMergeJoinExecutor` 的比较与乘积。`Value::Hash`。NaN 的排序比较。`IN` 子查询对哈希连接和嵌套循环的既有拒绝文案。`create_table_sql` 的现有渲染。每个新测试使用自己的 `TempDir`。`pipeline.rs`、`catalog.rs`、`table_manager.rs` 里已有的其他未提交改动保持不动。
- Forbidden: 不改 `src/executor/join.rs`、`src/executor/sort_merge_join.rs`、`src/executor/value.rs`、`src/executor/value_ref.rs`。不把 `primary_keys` 非空或 `pk_column` 非空当成写出的 `PRIMARY KEY`。不把 SQL 改写成躲开排序合并。不把 `equi_on_keeps_hash_join_shape` 或 `outer_join_routes_to_nested_loop_join` 的断言改成排序合并。不改 `explain_compound_nodes` 的 SQL。不改加列变更目录，不改 Iteration 000 的 Review。不跑全量，不跑 criterion。不实现 T4 的负零归一。不改 `Value::Hash`，不改 NaN 排序。
- First test: 改判定前先跑 `cargo test --test nested_loop_join_test equi_on_keeps_hash_join_shape`。预期退出码 101，实际节点是 `SortMergeJoin`。若这次已经是 `PhysicalPlan::Join`，停止并返回计划。看到这个失败之后，再增加 `implicit_first_column_integer_key_stays_hash_after_reopen`：`CREATE TABLE r (a INT)` 与 `CREATE TABLE s (b INT)`，关闭后用同一路径重新打开，`EXPLAIN SELECT r.a FROM r JOIN s ON r.a = s.b` 的连接行应以 `Join [` 开头。修改判定前再跑 `cargo test --test sort_merge_join_test implicit_first_column_integer_key_stays_hash_after_reopen`，预期失败，计划行以 `SortMergeJoin` 开头。两条失败都看到之后，再改显式主键位和 `integer_pk_base_scan_join`。
- GREEN condition: 上面第二条命令退出码 0。随后 Verification 里其余命令按顺序退出码 0，且 failed 为 0。
- Verification: 见本 Cycle 的 Verification。任一命令退出码非 0，或 failed 不为 0，本任务未完成。
- Stop when: 为了通过测试需要改执行器算法、`Value::Hash` 或 NaN 排序；需要改掉隐式首列写入 `pk_column` 的行为；需要改 `create_table_sql` 才能让锚点保持哈希；`equi_on_keeps_hash_join_shape` 或 `outer_join_routes_to_nested_loop_join` 的内连接仍是排序合并；重新打开后写出位丢失，写出的整数主键变成哈希，或隐式首列变成排序合并。这些情况返回计划。

**Invariants**

纯等值 SQL 只有在五条条件都成立时才离开哈希。外连接和非等值仍是嵌套循环。连接键相等仍与 `Value` 的 `PartialEq` 一致。无 `ORDER BY` 的排序合并结果不保证行序。打开数据库的新测试各自使用自己的 `TempDir`。旧目录行缺尾字节时，连接选择保持哈希，不把缺字节当成写出了 `PRIMARY KEY`。

**Non-goals**

负零与正零的哈希查找、较小构建侧、全量回归、NaN 排序、聚合与半连接和反连接的分桶、DDL 缺陷、schema 文本里何时打印 `PRIMARY KEY`。

**Acceptance**

- `integer_pk_base_scans_select_sort_merge`：写出整数主键的基表等值，`EXPLAIN` 含 `SortMergeJoin`，结果多重集合是 `(2, 2)`。
- `written_integer_primary_key_stays_sort_merge_after_reopen`：同样两表关闭再打开后，`EXPLAIN` 仍含 `SortMergeJoin`。
- `implicit_first_column_integer_key_stays_hash_after_reopen`：`r(a INT)` 与 `s(b INT)` 关闭再打开后，连接行以 `Join [` 开头。
- `self_join_integer_pk_selects_sort_merge`：自连接的 `EXPLAIN` 含 `SortMergeJoin` 和 `customers.id = customers.id`，结果多重集合是 id 1 与 id 2。
- `order_by_keeps_sort_merge_and_sort`：`EXPLAIN` 同时有 `Sort (` 与 `SortMergeJoin`，结果次序由 `ORDER BY` 决定。
- `explain_integer_pk_join_names_sort_merge`：`EXPLAIN` 含 `SortMergeJoin` 和等值条件。
- `sort_merge_join_statement_memory_limit_53200`：上限 16 时语句失败，SQLSTATE `53200`，消息与 Task Definition 中的句子一致，没有成功结果行。
- `non_pk_equijoin_stays_hash`：`orders.customer_id = customers.id` 以及多一条非主键腿的语句，`EXPLAIN` 行以 `Join [` 开头。
- `string_pk_equijoin_stays_hash`：存量字符串主键的等值内连接以 `Join [` 开头。
- `third_table_join_stays_hash`：三表链的 `EXPLAIN` 同时有 `SortMergeJoin` 和 `Join [`。
- `in_subquery_integer_pk_join_rejected`：错误含 `IN subquery with JOIN is not supported`，不含多列误报。
- `explain_compound_nodes`：`l.id = r.rid` 的计划行以 `SortMergeJoin` 开头，`Sort (` 仍在，`Limit` 与 `Filter` 仍在。
- `equi_on_keeps_hash_join_shape` 仍是哈希。`non_equi_on_routes_to_nested_loop_join` 仍是嵌套循环。`outer_join_routes_to_nested_loop_join` 的外连接仍是嵌套循环，内连接断言仍是 `PhysicalPlan::Join`。
- 目录行：`explicit_primary_key` 为真的行往返后仍为真。没有尾字节的行读回为假。`legacy_row_without_kind_section_reads_as_table` 仍把无 kind 段读成 Table。`view_row_with_truncated_sql_section_tolerates_as_table` 仍把截断的 View SQL 读成 Table。
- `test_pipeline_join_with_null_keys` 仍是 1 行。`execute_sql_returns_err_not_partial_rows_on_join_where_overflow` 仍整句失败并含溢出文案。`exists_inner_join_both_arms_aliased` 的内层结果仍是 id 1。`join_with_expression_rejected` 的错误消息仍含 `expression` 或 `Unsupported`。
- 上述既有用例若只因为无 `ORDER BY` 的行序改变而失败，把该断言改成多重集合比较。不改 SQL。

**Verification**

下列命令按写出的顺序逐条执行。每条都是一次独立的 `cargo test`。不合并成一次无过滤的 `cargo test`。定向测试使用默认并行度。不跑全量，不跑 criterion。除第 1 条和第 2 条按 First test 预期失败外，其余每条退出码 0 且 failed 为 0。

1. 修改前：`cargo test --test nested_loop_join_test equi_on_keeps_hash_join_shape`。预期退出码 101，节点是 `SortMergeJoin`。
2. 修改前：`cargo test --test sort_merge_join_test implicit_first_column_integer_key_stays_hash_after_reopen`。预期失败，连接行以 `SortMergeJoin` 开头。
3. 修改后：`cargo test --test sort_merge_join_test`。
4. `cargo test --lib catalog_row_explicit_primary_key_roundtrip`。
5. `cargo test --lib legacy_row_without_explicit_primary_key_byte_reads_false`。
6. `cargo test --lib legacy_row_without_kind_section_reads_as_table`。
7. `cargo test --lib view_row_with_truncated_sql_section_tolerates_as_table`。
8. `cargo test --test explain_test explain_compound_nodes`。
9. `cargo test --test nested_loop_join_test equi_on_keeps_hash_join_shape`。
10. `cargo test --test nested_loop_join_test non_equi_on_routes_to_nested_loop_join`。
11. `cargo test --test pipeline_test test_pipeline_join_with_null_keys`。
12. `cargo test --test join_test execute_sql_returns_err_not_partial_rows_on_join_where_overflow`。
13. `cargo test --test subquery_alias_binding_test exists_inner_join_both_arms_aliased`。
14. `cargo test --test projection_expression_test join_with_expression_rejected`。
15. `cargo test --test outer_join_test outer_join_routes_to_nested_loop_join`。

第 11 到第 14 条若只因行序失败，按 Acceptance 改成多重集合后重跑该条一次。第 9 条或第 15 条若仍是排序合并，停止。

**Gate 2 Readiness**

PASS。无 BLOCKED，无 SKIPPED。2026-10-26 用户批准：「批准在目录行末尾追加 `explicit_primary_key` 字节。缺这个字节的旧行读成没写出 PRIMARY KEY。按已有重订计划实施。」该批准只覆盖这个尾字节和本 Cycle 已经写明的方案，不扩大范围。本次重读后，任务、规格、设计与本 Cycle 的执行依据一致。RTM 没有 Missing。Persisted Evidence 为 none。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- 整数主键在 SQL 里不能重复，所以本 Cycle 的语句测试看的是匹配键的多重集合。
- `create_table_sql` 仍会给每个整数 `pk_column` 印出 `PRIMARY KEY`。dump 再 restore 之后，那张表会变成写出的主键。本 Cycle 不改这条渲染。
- 缺尾字节的旧库，包括升级前已经写出 `PRIMARY KEY` 的表，读回都是 `false`，连接保持哈希。新的 `CREATE TABLE ... PRIMARY KEY` 会写上 `true`。无法从旧字节分辨当年有没有写出关键字。
- `test_pipeline_join_with_null_keys` 会改走排序合并。它插入 NULL。若主键拒绝这次插入，该测试在修改前就已经失败；那时沿用修改前的失败并停止，不把主键约束改掉。
- 扫描执行器若也向同一预算入账，上限 16 仍应得到 `53200` 和同一句消息。若错误变成别的 SQLSTATE 或另一句消息，停止。
- 负零漏匹配还没有用这对键跑过。本 Cycle 不修。
- 第 4 条和第 5 条是新增的 `catalog.rs` 测试名。实现时用这两个名字，让 Verification 里的命令能滤到它们。

## Act Response

- Status: reported

两侧都是基表扫描，且每个连接键都是该表写出的整数 `PRIMARY KEY` 时，`execute_sql` 发出排序合并。没有写出 `PRIMARY KEY` 的整数首列，关闭再打开后仍是哈希。

### 实际改动

- `CatalogRow.explicit_primary_key`、`serialize_catalog_row`、`deserialize_catalog_row`：kind 段和视图 SQL 之后追加一个 `u8`。`1` 为真，`0` 为假。没有这个字节时读成 `false`，不拒绝打开。`insert_view_row` 写 `false`。
- `TableMeta.explicit_primary_key`。`create_table_with_constraints` 增加最后一个参数。`CreateTableExecutor` 传入 `node.primary_key.is_some()`；`None` 时仍把第一列写入 `pk`。`TableManager::create_table`、CTAS，以及 `table_manager`、`cli_test`、`constraint_enforcement_test`、`datetime_type_test`、`keyless_eq_routing_test`、`sort_merge_join_test` 里的既有调用传入 `false`。
- `open_or_init`、`narrowed_meta`、`alter_add_column`、`replace_index_manager`、`replace_recovery_indexes`、`register_secondary_index`、`take_secondary_index` 从旧值复制该字段。
- `PlanBuilder.explicit_primary_keys` 与 `set_explicit_primary_key`。`register_table` 的签名不变。`register_table_schema` 只在 `table_meta.explicit_primary_key` 为真时调用它。
- `integer_pk_base_scan_join` 保留原有四条，再要求左右 `ScanNode.table_name` 都在 `explicit_primary_keys` 里。
- `src/cli/lifecycle.rs` 的测试夹具 `table()` 补上该字段，值为 `false`。`create_table_sql` 没有改。
- 新增 `implicit_first_column_integer_key_stays_hash_after_reopen`、`written_integer_primary_key_stays_sort_merge_after_reopen`、`catalog_row_explicit_primary_key_roundtrip`、`legacy_row_without_explicit_primary_key_byte_reads_false`。`legacy_row_without_kind_section_reads_as_table` 同时截掉尾字节和 kind 字节。`view_row_with_truncated_sql_section_tolerates_as_table` 截进 SQL 正文。

### 与计划的偏差

`build_from_clause_with_projection` 里另一处仍写着「已声明整数主键」的注释，以及 `integer_pk_base_scan_join` 文档里的「四个条件」，改成写出的整数 `PRIMARY KEY` 和五个条件。判定按 Task Definition，没有另起触发条件。

### 验证

修改判定前：

1. `cargo test --test nested_loop_join_test equi_on_keeps_hash_join_shape`：退出码 101。0 passed，1 failed。实际节点是 `SortMergeJoin`。
2. `cargo test --test sort_merge_join_test implicit_first_column_integer_key_stays_hash_after_reopen`：退出码 101。0 passed，1 failed。计划行以 `SortMergeJoin` 开头。

修改后，下列命令按顺序各执行一次。退出码都是 0，failed 都是 0。

3. `cargo test --test sort_merge_join_test`：14 passed。
4. `cargo test --lib catalog_row_explicit_primary_key_roundtrip`：1 passed。
5. `cargo test --lib legacy_row_without_explicit_primary_key_byte_reads_false`：1 passed。
6. `cargo test --lib legacy_row_without_kind_section_reads_as_table`：1 passed。
7. `cargo test --lib view_row_with_truncated_sql_section_tolerates_as_table`：1 passed。
8. `cargo test --test explain_test explain_compound_nodes`：1 passed。
9. `cargo test --test nested_loop_join_test equi_on_keeps_hash_join_shape`：1 passed。
10. `cargo test --test nested_loop_join_test non_equi_on_routes_to_nested_loop_join`：1 passed。
11. `cargo test --test pipeline_test test_pipeline_join_with_null_keys`：1 passed。
12. `cargo test --test join_test execute_sql_returns_err_not_partial_rows_on_join_where_overflow`：1 passed。
13. `cargo test --test subquery_alias_binding_test exists_inner_join_both_arms_aliased`：1 passed。
14. `cargo test --test projection_expression_test join_with_expression_rejected`：1 passed。
15. `cargo test --test outer_join_test outer_join_routes_to_nested_loop_join`：1 passed。

第 11 到第 14 条没有因行序失败，没有改断言。

### Persisted Evidence

None required。

### Experience Candidates

None。

### Issue 候选

None。

### 未解决问题

无。

## Plan Review

- Review Result: accepted

**Findings**

无阻塞项，无 Minor。

`integer_pk_base_scan_join` 保留原有四条，再要求左右 `ScanNode.table_name` 都在 `explicit_primary_keys` 里。这个集合只在 `TableMeta.explicit_primary_key` 为真时，由 `register_table_schema` 调用 `set_explicit_primary_key` 写入。`CreateTableExecutor` 传入 `node.primary_key.is_some()`；`None` 时仍把第一列写入 `pk`。`TableManager::create_table`、CTAS，以及 `table_manager`、`cli_test`、`constraint_enforcement_test`、`datetime_type_test`、`keyless_eq_routing_test`、`sort_merge_join_test` 里的既有 `create_table_with_constraints` 调用传入 `false`。

目录行在 kind 段和视图 SQL 之后追加一个 `u8`。反序列化只在 kind 段消费成功后读取该字节：`1` 为真，缺字节或 `0` 为假，不因此拒绝打开。`open_or_init`、`narrowed_meta`、`alter_add_column`、`replace_index_manager`、`replace_recovery_indexes`、`register_secondary_index`、`take_secondary_index` 从旧值复制该字段。`insert_view_row` 写 `false`。

Act 记录的注释改写与代码一致。`build_from_clause_with_projection` 和 `integer_pk_base_scan_join` 的说明已写成写出的整数 `PRIMARY KEY` 和五个条件。判定没有另起触发条件。

`equi_on_keeps_hash_join_shape` 仍断言 `PhysicalPlan::Join`。`outer_join_routes_to_nested_loop_join` 的外连接仍断言嵌套循环，其中 `l.id = r.rid` 的内连接仍断言 `PhysicalPlan::Join`。`explain_compound_nodes` 的 SQL 仍是 `l.id = r.rid`，并带 `WHERE`、`ORDER BY` 和 `LIMIT`，计划行断言 `SortMergeJoin`。`create_table_sql` 没有改。`src/executor/join.rs`、`src/executor/value.rs`、`src/executor/value_ref.rs` 相对 HEAD 没有未提交差异。没有改加列变更目录。

**Evidence**

验证沿用本 Cycle Act Response，没有重跑。修改判定前第 1、2 条退出码 101，各 1 failed。修改后第 3 到第 15 条退出码 0、failed 0；其中 `cargo test --test sort_merge_join_test` 为 14 passed。`tests/sort_merge_join_test.rs` 有 14 个 `tokio::test`，与 14 passed 一致。

这些源文件的修改时间早于本文件里 Act Response 的写入，其后没有新的 git 提交。声明与当前代码不矛盾。工作区 `target/` 里能看到的测试二进制早于这些源文件，这次审查不把那些旧二进制当作本轮运行证据。

代码位置：

- `src/parser/planner/query.rs` 的 `integer_pk_base_scan_join`：两侧都是 `Scan`，条件非空，两侧主键类型为已登记整数，两侧表名都在 `explicit_primary_keys`，且每条条件列是该侧扫描表的主键。别名经 `resolve_qualified_column` 归一成真实表名后再比较。
- `src/executor/create_table.rs`：`primary_key` 为 `None` 时主键列仍是第一列，显式位为 `false`。
- `src/storage/catalog.rs` 的 `deserialize_catalog_row`：缺尾字节读成 `false`。`legacy_row_without_explicit_primary_key_byte_reads_false` 去掉当前布局的尾字节后断言为假，kind 仍是 Table。`catalog_row_explicit_primary_key_roundtrip` 断言为真的行往返后仍为真。`legacy_row_without_kind_section_reads_as_table` 同时去掉尾字节和 kind 字节。`view_row_with_truncated_sql_section_tolerates_as_table` 用 `truncate(len - 2)` 截进 SQL 正文。
- `implicit_first_column_integer_key_stays_hash_after_reopen` 与 `written_integer_primary_key_stays_sort_merge_after_reopen` 都关闭后用同一路径重新打开。

Persisted Evidence 为 none。未要求证据目录。

**Follow-up Decision**

三条验收都在当前代码里成立，既有 Acceptance 没有缺口，所以接受。Iteration Plan 不改。

`tasks.md` 的 Map 里还有 Iteration 002 和 Iteration 003。这次审查只结束本 Cycle，不展开下一 Iteration，也不写后继 Cycle。

**Next Cycle**

None

**Next Iteration**

None
