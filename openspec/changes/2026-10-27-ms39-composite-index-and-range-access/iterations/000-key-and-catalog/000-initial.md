# Iteration 000 / Cycle 000: 键派生与目录世代

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1, T2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 规格 `sql-secondary-indexes` 中键编码、目录世代和哨兵端点。页格式保持 32 字节键、叶条目 38 字节、内部分隔 36 字节。
- Excluded scope: SQL 创建组合索引、DML 维护、范围执行器、EXPLAIN、README、全量测试。这些属于 001–003。

**Objective**

世代 1 的组合键和范围哨兵可以由单测直接观察。`__tables` 能往返组合列号和世代。世代 0 的单列记录不增加尾段。`CREATE INDEX` 多列语句仍然被拒绝。

**Investigation Facts**

- Starting point: 分支基点是 master `2b9a3d1`。MS38 已保留 32 字节定长键。`Value::secondary_index_key` 对 INT 使用 `i64::to_be_bytes()`，与 `Value::to_key` 相同。2026-10-27 运行 `cargo test --offline --lib executor::value::tests::secondary_index_key_preserves_int_bytes -- --test-threads=1`，结果 1 passed、0 failed。字符串编码是非零字节原样、`00` 写成 `00 FF`、结尾 `00 00`，总长超过 32 则 `None`。大端有符号整数的字节序不是数值序。
- Current-State Evidence:
  - `src/executor/value.rs::secondary_index_key` 与同文件的 `fixed32_string_key` 是单列派生。调用者有 `create_index.rs` 回填、`insert.rs`、`update.rs`、`delete.rs`、`upsert.rs`、`table_manager.rs`、`wal/recovery.rs::extract_index_keys` 的二级分支，以及 `query.rs::extract_secondary_index_equality`。本轮不改这些调用者。
  - `src/storage/page_format/key.rs::Key` 的 `Ord` 和 `Eq` 比较填零后的 32 字节。`len` 不参与比较。
  - `src/storage/catalog.rs` 的二级索引段是 `u16 name_len | name | u32 column_index | u32 root`。行在 kind 字节、可选视图 SQL 和 `explicit_primary_key` 之后结束。`deserialize_catalog_row` 对不完整尾段采取回退，不拒绝整行。
  - `src/storage/data/table_manager.rs::SecondaryIndexMeta` 只有 `name`、`column_index` 和 `index`。`register_secondary_index` 写入上述三段。
  - `src/parser/planner/ddl_dml.rs::build_create_index` 在 `columns.len() != 1` 时返回 `UnsupportedStatement("MULTI-COLUMN INDEX")`。集成测试 `string_index_rejection_readonly_and_transaction` 断言该文本。
- Code and Critical Path: 本轮的新路径止于纯函数和目录序列化。`serialize_catalog_row` / `deserialize_catalog_row` 是磁盘格式的唯一出入口。`append_secondary_index` 必须继续写出世代 0 的旧布局，否则现有建索引会改变字节。查询和写入仍走原来的单列函数。

**Implementation Guidance**

先写 T1 的测试并看到失败，再实现派生。T1 通过后再改目录结构。新字段使已有结构体字面量无法编译时，全部补成世代 0 和空的额外列号，不要在那些调用点改用世代 1。

哨兵只在一个函数里计算。后续范围执行器只消费它的返回值，不另写一套端点。

**Task Definitions**

### T1: 世代 1 键与范围哨兵可观察

- Requirement/Scenario: 范围谓词路由到二级索引扫描；CREATE INDEX 的编码条款。场景 S02、S04、S05 的端点部分。
- Depends on: None
- Targets: `src/executor/value.rs`。新增 `derive_secondary_index_key(columns, values, generation)` 和 `secondary_range_bounds(index_columns, prefix_bounds)`。保留 `secondary_index_key` 的现有字节。
- Current behavior: 只有单列派生。INT 键是大端 8 字节。没有组合拼接，也没有范围端点。
- Required behavior:
  - `generation == 0` 且恰好一列时，结果字节等于 `secondary_index_key`。列数不是 1 时返回 `None`。
  - `generation == 1`：任一 NULL 或不支持类型返回 `None`。INT 分量是 `to_be_bytes()` 后首字节异或 `0x80` 的 8 字节。字符串分量是现有转义，不含填零。按列序拼接，总长大于 32 返回 `None`，否则 `Key::new`。
  - 有符号顺序：`i64::MIN < -1 < 0 < 1 < i64::MAX` 的世代 1 单列键按此比较。
  - 组合 `(1, i64::MAX)` 的键小于 `(2, i64::MIN)`。两个能放下的不同组合不得得到同一个 `Key`。
  - `secondary_range_bounds` 的前缀除最后一列外必须是等值，最后一列可以是等值或带包含标记的上下界。否则返回 `None`。前缀长于索引列、类型不匹配、NULL 或字符串放不下时返回 `None`。
  - 设前缀编码为 P，长度为 n。包含下界是 P 补零，比较为大于等于。不包含下界在 n < 32 时是 P 补 `FF`，n = 32 时是这 32 字节，比较为大于。包含上界对称使用补 `FF` 或 32 字节本身，比较为小于等于。不包含上界是 P 补零，比较为小于。
  - 用这些端点判断键时：单列 `> 0` 排除 0 和全部负数，包含 1。单列 `< 0` 只包含负数。组合索引 `(a, b)` 上，`(1, i64::MAX)` 属于 `a = 1`，不属于 `a > 1`。`(2, i64::MIN)` 属于 `a > 1`，不属于 `a = 1 AND b > 2`。`(1, i64::MAX)` 属于 `a = 1 AND b > 2`。
  - 四列 INT 的整键长度为 32。`d > 4` 且前三列等值时，不包含下界就是这 32 字节本身，大于该键的下一个整数组合被包含，等于该键的组合被排除。
- Preserve: `secondary_index_key_preserves_int_bytes` 继续通过。`Key` 的比较方式、页类型和条目长度不变。
- Forbidden: 修改 `to_key`、计划器、执行器、DML、恢复和 README。不得截断，不得引入变长页格式。
- First test: 在 `src/executor/value.rs` 的测试模块新增 `ordered_int_key_matches_signed_order`、`composite_key_orders_prefix_before_next_int`、`composite_key_is_none_when_encoding_exceeds_32`、`legacy_single_column_derivation_matches_secondary_index_key`、`range_sentinel_composite_prefix_excludes_next_leading_value`、`range_sentinel_four_int_columns_use_exact_key_when_full`。实现前运行 `cargo test --offline --lib executor::value::tests::ordered_int_key_matches_signed_order`，预期失败（编译失败或断言失败）。同时运行 `secondary_index_key_preserves_int_bytes`，预期 1 passed、0 failed。
- GREEN condition: 上述新测试和 `secondary_index_key_preserves_int_bytes` 都是 passed、failed 为 0。命令：`cargo test --offline --lib executor::value::tests`。
- Verification: 同一命令。失败表示键序、超长或哨兵与本任务不一致。
- Stop when: 要满足测试就必须修改 `Key::cmp`、条目长度或主键 `to_key`。这是计划错误，停止并返回计划，不要改这些对象。

### T2: 目录尾段保存世代和其余列号

- Requirement/Scenario: CREATE INDEX 的持久化条款，以及旧整数索引字节保持。场景 S03 的目录部分。
- Depends on: T1
- Targets: `src/storage/catalog.rs` 的 `CatalogSecondaryIndex`、`serialize_catalog_row`、`deserialize_catalog_row`；`src/storage/data/table_manager.rs` 的 `SecondaryIndexMeta` 和所有必须为了编译而填写新字段的构造点，包括 `register_secondary_index` 和 `narrowed_meta`。
- Current behavior: 二级索引只有一个列号。序列化在 `explicit_primary_key` 结束。内存元数据没有世代。
- Required behavior:
  - `CatalogSecondaryIndex` 增加 `generation: u8` 和 `extra_column_indexes: Vec<u32>`。`column_index` 仍是第一列。
  - `SecondaryIndexMeta` 增加 `generation: u8` 和 `extra_column_indexes: Vec<usize>`。`column_index` 仍是第一列。
  - 全部索引都是世代 0 且没有额外列时，序列化字节与增加字段之前的布局相同，行以 `explicit_primary_key` 结束，不写尾段。
  - 任一索引世代非 0 或有额外列时，在 `explicit_primary_key` 之后写 `u32 count`，再对每个索引写 `u8 generation | u16 extra_count | extra_count 个 u32`。`count` 等于索引个数。
  - 反序列化时尾段缺失、`count` 不符或被截断：每个索引为世代 0，列只有原来的 `column_index`，不返回错误。尾段完整时使用其中的世代，并把额外列号接在第一列之后。
  - 现有 `register_secondary_index` 写出的记录保持世代 0、无额外列。`narrowed_meta` 复制新字段；本轮仍只按第一列过滤，不改变删列结果。
- Preserve: 现有单列建索引、等值查询和 catalog 往返的行含义。多列 `CREATE INDEX` 仍被拒绝。
- Forbidden: 在本轮让 SQL 接受多列索引，或把新建 INT 索引写成世代 1。不得重写打开时读到的旧页。
- First test: 在 `src/storage/catalog.rs` 的测试中新增 `legacy_secondary_index_row_has_no_generation_tail` 和 `composite_secondary_index_row_roundtrips_generation_and_columns`。实现前 `legacy_secondary_index_row_has_no_generation_tail` 失败。`catalog_row_with_secondary_indexes_roundtrip` 在只补齐新字段、尚未改语义时期望仍通过。
- GREEN condition: 两个新测试和 `catalog_row_with_secondary_indexes_roundtrip` 都是 passed、failed 为 0。`cargo test --offline --test string_index_sql_test string_index_rejection_readonly_and_transaction` 通过，且失败文本仍含 `MULTI-COLUMN INDEX`。
- Verification: `cargo test --offline --lib catalog_row_with_secondary_indexes_roundtrip legacy_secondary_index_row_has_no_generation_tail composite_secondary_index_row_roundtrips_generation_and_columns`，以及上面的集成测试。库测试失败表示尾段规则错误。集成测试失败表示本轮越过了 SQL 边界。
- Stop when: 不改变 `build_create_index` 就无法保持多列拒绝，或旧单列记录必须写尾段才能通过新测试。返回计划。

**Invariants**

- `Key` 固定 32 字节，比较填零后的全长。
- 主键和唯一索引继续使用 `Value::to_key` 的大端字节。
- 叶条目 38 字节，内部分隔 36 字节，页类型 0x01/0x02。
- 世代 0 的单列二级索引磁盘记录不增长。
- 超长编码返回 `None`，不截断。

**Non-goals**

SQL 组合索引、范围扫描执行器、DML 改用世代 1、恢复重建、README、全量回归、表达式索引和部分索引。

**Acceptance**

- T1 的六个新测试和 `secondary_index_key_preserves_int_bytes` 通过，键序与哨兵符合 Task Definition。
- T2 的两个新测试和既有 catalog 往返通过。多列 `CREATE INDEX` 仍含 `MULTI-COLUMN INDEX`。
- `git diff` 不含 `src/parser/`、`src/executor/create_index.rs`、`src/executor/index_scan_all.rs`、DML 执行器和 `README.md`。

**Verification**

T1 与 T2 的命令见各任务。它们直接断言键字节、键顺序、序列化长度和 SQL 错误文本。不使用额外的判定程序。不跑全量。全量预算在 Iteration 003。

**Gate 2 Readiness**

PASS。计划审查的 11 项均通过，RTM 无 Missing，没有未批准的裁剪。批准来自本次目标授权，不是跳过。

**Persisted Evidence**

- Mode: none

测试可以重跑。命令、结果和改动的符号写入 Act Response 即可。

**Risks and Notes**

- 结构体增加字段后，所有字面量都要补世代 0 和空额外列。这是编译所必需的，不改变这些调用点的键。
- 哨兵的 32 字节边界由 `range_sentinel_four_int_columns_use_exact_key_when_full` 锁定。不要在测试之外再解释一套端点。
- 本轮不处理实施时才发现的缺陷。若出现任务范围外的实质缺陷，停止并报告，不在本轮扩大修改。

## Act Response

实施尚未开始。本段由 Act 覆盖。

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
