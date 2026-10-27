# Iteration 003 / Cycle 000: 当前代目录行长度

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T5
- Acceptance gaps: None
- Repair items: None
- Inherited scope: Iteration 001 已接受的目录尾字节。`serialize_catalog_row` 在 kind 段和视图 SQL 之后追加一个 `u8`：`1` 为写出了 PRIMARY KEY，`0` 为没有。缺这个字节的旧行读成没写出 PRIMARY KEY。新写入的当前代行带这个字节。
- Excluded scope: T4 的负零键归一、T3 的较小构建侧与时间门、4.2 的全量回归。不改 `serialize_catalog_row` 和 `deserialize_catalog_row`。不改 `iterations/002-signed-zero-match/000-initial.md`，不把它的 Plan Context 改成非 ready。不改 `iterations/001-planner-selection/` 下的 Cycle。不读写加列变更目录。

**Objective**

`legacy_db_write_upgrades_touched_table_row_to_current_layout` 与 `legacy_g2_db_opens_queries_and_writes` 里，被写到的表目录行长度等于 `g1_tables_row_len + 10`，尾段以表类型字节 `0x01` 和 `explicit_primary_key` 字节 `0x00` 结束。未被写到的表仍停在原代际。缺尾字节的当前 kind 布局仍读成 `explicit_primary_key == false`。序列化仍写出这个字节。

**Background**

用户给出的失败：`legacy_db_write_upgrades_touched_table_row_to_current_layout` 在 `tests/legacy_compat_test.rs:441` 期望 40、得到 41；`legacy_g2_db_opens_queries_and_writes` 在 `tests/legacy_compat_test.rs:1086` 期望 41、得到 42。用户要求用本变更里、全量回归之前的一轮对齐这两条断言，不撤销已经批准的尾字节，检查标准不降低。本次没有重跑这两条测试。

**Investigation Facts**

- Starting point: 尾字节已经写在产品代码里。`iterations/002-signed-zero-match/000-initial.md` 的 Plan Context 为 ready，本 Cycle 不改那个文件。全量回归还在 Iteration 004，目录尚未展开。
- Current-State Evidence:
  - `serialize_catalog_row`（`src/storage/catalog.rs`）在 kind 字节和视图 SQL 之后执行 `buf.push(if row.explicit_primary_key { 1 } else { 0 })`。唯一根计数与二级索引计数都为 0、kind 为 Table、没有视图 SQL 时，数据页尾指针之后是 4 字节唯一根计数、4 字节二级索引计数、1 字节 kind、1 字节 `explicit_primary_key`，共 10 字节。
  - `deserialize_catalog_row` 在 kind 段消费成功后读下一个字节：值为 `1` 则 `explicit_primary_key` 为真，缺字节或值为 `0` 则为假。kind 段没有消费成功时也为假。缺字节不拒绝该行。
  - `Catalog::update_table_tail` 对命中的行做 `deserialize_catalog_row`，只改 `data_page_tail`，再 `serialize_catalog_row`。`TableManager::write_tuple` 在新数据页分配后调用 `update_table_tail`。两条测试都插入 120 行，使文件变大，从而走到这次重写。
  - 旧行没有尾字节时，反序列化得到 `false`，重写写出 `0`。重写后的长度比测试锁定的 `g1_tables_row_len + 9` 多 1。`users` 的名字 5 字节、主键列名 `id` 为 2 字节，G1 固定段是 31，加 9 为 40，与给出的期望一致，实际 41。`people` 的名字比 `users` 多 1 字节，G1 固定段是 32，加 9 为 41，与给出的期望一致，实际 42。
  - 长度断言只有这两处。`legacy_db_write_upgrades_touched_table_row_to_current_layout` 在长度断言之后把 `users[g1_len..]` 比成 `[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01]`。`legacy_g2_db_opens_queries_and_writes` 把 `people_row` 的同一段比成 `[0, 0, 0, 0, 0, 0, 0, 0, 0x01]`。只改 `+ 9` 而不改这两段字面量，下一条断言仍会因多出的 `0x00` 失败。全库没有第三条 `g1_tables_row_len + 9` 或同等的当前代绝对长度断言。
  - 同文件里未被写到的表不改：`audit` 与 `events` 仍断言等于 `g1_tables_row_len`；`notes` 仍断言等于 `g2_tables_row_len`。样本本身的代际断言 `legacy_generation_db_is_in_supported_range` 与 `legacy_g2_catalog_rows_are_in_g2_generation` 也不改。
  - `legacy_row_without_explicit_primary_key_byte_reads_false` 对当前布局 `pop()` 一次后断言 `explicit_primary_key` 为假且 kind 仍是 Table。`catalog_row_explicit_primary_key_roundtrip` 断言写出的 `true` 能读回。这两条不改。
- Code and Critical Path: 插入分配新数据页 → `write_tuple` → `update_table_tail` → 反序列化缺字节为假 → 序列化在 kind 后写 `0`。测试读回 `__tables` 链上该表最新行，用 `g1_tables_row_len` 作前缀，再检查剩余字节。

**Implementation Guidance**

先跑下面 First test 里的两条集成测试，确认失败点仍是长度断言、且实际值比 `g1_tables_row_len + 9` 大 1。同时跑两条 lib 测试，确认它们已经通过。然后只改 `tests/legacy_compat_test.rs` 里点名的期望和两处说明当前布局尾段的注释。不要改产品代码。

**Task Definitions**

### T5: 两条升级断言与当前代目录行长度一致

- Requirement/Scenario: 旧库文件的打开、查询与写入 / 写入使被写到的表目录行升级为当前布局
- Depends on: None
- Targets:
  - `tests/legacy_compat_test.rs::legacy_db_write_upgrades_touched_table_row_to_current_layout`
  - `tests/legacy_compat_test.rs::legacy_g2_db_opens_queries_and_writes`
  - `tests/legacy_compat_test.rs` 里 `g1_tables_row_len` 的文档注释，以及 `legacy_db_write_upgrades_touched_table_row_to_current_layout` 上方把当前布局尾段写成唯一根计数、二级索引计数和表类型字节的文档注释
- Current behavior: 被写到的 `users` 行长度为 `g1_tables_row_len + 10`，测试期望 `+ 9`（给出的结果是期望 40、得到 41）。被写到的 `people` 行同样多 1 字节（期望 41、得到 42）。两条尾段字面量都停在 kind 字节 `0x01`，没有末尾的 `0x00`。`audit`、`events` 仍是 G1 长度，`notes` 仍是 G2 长度。序列化已经在 kind 后写出 `explicit_primary_key`。
- Required behavior: `users.len()` 与 `people_row.len()` 都断言为各自的 `g1_tables_row_len + 10`。`users` 的尾段断言为 `[0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00]`。`people_row` 的尾段断言为 `[0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0x00]`。断言说明写明末字节是 `explicit_primary_key = 0`。`g1_tables_row_len` 的注释改为：返回值不含数据页尾指针之后的唯一根段、二级索引段、表类型字节和 `explicit_primary_key` 字节。该函数的计算不改。被写到的表的文档注释同样写上末字节 `0`。
- Preserve: `audit` 与 `events` 的长度仍等于 `g1_tables_row_len`。`notes` 的长度仍等于 `g2_tables_row_len`。两条测试里的查询、行数、完整性和文件增长断言不改。`legacy_row_without_explicit_primary_key_byte_reads_false` 仍为缺字节读成假。`catalog_row_explicit_primary_key_roundtrip` 仍为写出的真值能读回。`serialize_catalog_row` 仍在 kind 之后写出这个字节。
- Forbidden: 不改 `src/storage/catalog.rs`、`src/storage/data/table_manager.rs` 和其他产品代码。不删尾字节，也不让升级路径少写这 1 字节。不把末字节改成 `1`。不改未被写到的表和两份样本代际断言。不改 Iteration 002 的 Cycle 文件，不把它标成未 ready。不改 Iteration 001 的 Cycle。不读写加列变更目录。不新增测试，也不去改这两条以外的长度断言。
- First test: 修改前执行 `cargo test --test legacy_compat_test legacy_db_write_upgrades_touched_table_row_to_current_layout` 和 `cargo test --test legacy_compat_test legacy_g2_db_opens_queries_and_writes`。两条都必须失败，且失败点是长度断言：前者期望 40、得到 41，后者期望 41、得到 42。修改前再执行 `cargo test --lib legacy_row_without_explicit_primary_key_byte_reads_false` 和 `cargo test --lib catalog_row_explicit_primary_key_roundtrip`，两条都必须已经通过。任一条集成测试已经通过、失败点不是这个长度差、或任一条 lib 测试失败，都停止并返回计划。不要为了让 `+ 9` 通过而去改序列化。
- GREEN condition: 只改上述断言和两处注释之后，四条命令都退出码 0。两条集成测试的尾段断言也通过，末字节为 `0`。
- Verification: 四条命令逐条执行，定向测试可用默认并行度。通过条件是退出码 0。集成测试失败表示升级行的长度或尾段仍与当前代不一致，或查询、完整性、未写表代际被带坏。lib 测试失败表示缺字节不再读成假，或写出的真值不能读回。
- Stop when: 修改前的失败不是上述长度差；重写行的最后一个字节不是 `0`；不改产品代码就无法让这两条测试通过；需要改动的断言超出上面列出的两处长度和两处尾段。

**Invariants**

缺尾字节的旧行读成没写出 PRIMARY KEY。当前代序列化在 kind 之后写出 `explicit_primary_key`。未被写到的表保持原代际，不补这个字节。隐式首列仍写入 `pk_column`。连接选择规则不因本任务改变。

**Non-goals**

不实现 T4、T3 和全量回归。不改负零归一、构建侧和计划期分类。不改加列变更。不改两份旧样本文件。

**Acceptance**

- `legacy_db_write_upgrades_touched_table_row_to_current_layout` 中 `users` 的长度为 `g1_tables_row_len + 10`，尾段为 8 个 `0x00`、`0x01`、`0x00`。`audit` 与 `events` 仍等于 G1 长度。
- `legacy_g2_db_opens_queries_and_writes` 中 `people` 的长度为 `g1_tables_row_len + 10`，尾段同样以 `0x01, 0x00` 结束。`notes` 仍等于 G2 长度。
- `legacy_row_without_explicit_primary_key_byte_reads_false` 通过，缺字节仍为假。
- `catalog_row_explicit_primary_key_roundtrip` 通过，写出的真值仍能读回。
- diff 不包含 `serialize_catalog_row` 或 `deserialize_catalog_row` 的修改。

**Verification**

1. 修改前：`cargo test --test legacy_compat_test legacy_db_write_upgrades_touched_table_row_to_current_layout`。失败，期望 40、得到 41。
2. 修改前：`cargo test --test legacy_compat_test legacy_g2_db_opens_queries_and_writes`。失败，期望 41、得到 42。
3. 修改前：`cargo test --lib legacy_row_without_explicit_primary_key_byte_reads_false`。退出码 0。
4. 修改前：`cargo test --lib catalog_row_explicit_primary_key_roundtrip`。退出码 0。
5. 修改后重复这四条命令。四条都退出码 0。

这四条都走现有测试入口。前两条经 `Database::open` 与 `execute_sql` 写入旧样本，再读目录链字节。不新写判定程序，不跑全量。

**Gate 2 Readiness**

PASS。无 BLOCKED，无 SKIPPED。2026-10-26 用户要求把这两条长度断言纳入本变更，插在全量之前，不撤销已批准的尾字节，检查标准不降低。调查确认多出的 1 字节是 `serialize_catalog_row` 写出的 `explicit_primary_key`，升级路径对缺字节的行写出 `0`。全库没有第三条当前代长度断言。任务、增量规格、design 的 D7 与本 Cycle 一致。RTM 无 Missing。Persisted Evidence 为 none。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- 给出的 40/41 与 41/42 来自用户陈述，本 Cycle 编写时没有重跑。Act 的修改前命令若与这个长度差不一致，按 First test 停止。
- 尾段字面量不是单独的第三条长度断言，但它们把当前代尾段钉在 9 字节。必须和长度断言一起改，否则长度改完后下一条断言失败。
- `g1_tables_row_len` 只计算 G1 固定段。注释要改，函数体不改。

## Act Response

- Status: reported

**实际改动**

只改了 `tests/legacy_compat_test.rs`。

- `g1_tables_row_len` 的注释改为：返回值不含数据页尾指针之后的唯一根段、二级索引段、表类型字节和 `explicit_primary_key` 字节。函数体未改。
- `legacy_db_write_upgrades_touched_table_row_to_current_layout` 的文档注释写上末字节 `explicit_primary_key` 为 0。`users` 的长度改为 `g1_tables_row_len + 10`，尾段改为 8 个 `0x00`、`0x01`、`0x00`。断言说明写明末字节是 `explicit_primary_key = 0`。
- `legacy_g2_db_opens_queries_and_writes` 里 `people_row` 的长度改为 `g1_tables_row_len + 10`，尾段改为 `[0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0x00]`。断言说明同样写明末字节是 `explicit_primary_key = 0`。
- `audit`、`events`、`notes` 的长度断言未改。`serialize_catalog_row` 和 `deserialize_catalog_row` 未改。

**与计划的偏差**

无。

**自检**

Gate 3 通过。两条集成测试在改断言前都停在长度断言，实际值比 `g1_tables_row_len + 9` 大 1。改完后长度和尾段都通过，末字节为 `0`。缺字节仍读成假，写出的真值仍能读回。本轮 diff 只有上述注释和断言。`src/storage/catalog.rs` 相对 HEAD 的未提交差异是本轮之前就在的，本轮没有改这个文件。

无 Critical 或 Important。遗留 Minor：无。

**验证**

| 命令 | 时机 | 退出码 | 结果 |
|---|---|---|---|
| `cargo test --test legacy_compat_test legacy_db_write_upgrades_touched_table_row_to_current_layout` | 修改前 | 101 | 0 passed，1 failed。`tests/legacy_compat_test.rs:441`，left 41，right 40 |
| `cargo test --test legacy_compat_test legacy_g2_db_opens_queries_and_writes` | 修改前 | 101 | 0 passed，1 failed。`tests/legacy_compat_test.rs:1086`，left 42，right 41 |
| `cargo test --lib legacy_row_without_explicit_primary_key_byte_reads_false` | 修改前 | 0 | 1 passed，0 failed |
| `cargo test --lib catalog_row_explicit_primary_key_roundtrip` | 修改前 | 0 | 1 passed，0 failed |
| `cargo test --test legacy_compat_test legacy_db_write_upgrades_touched_table_row_to_current_layout` | 修改后 | 0 | 1 passed，0 failed |
| `cargo test --test legacy_compat_test legacy_g2_db_opens_queries_and_writes` | 修改后 | 0 | 1 passed，0 failed |
| `cargo test --lib legacy_row_without_explicit_primary_key_byte_reads_false` | 修改后 | 0 | 1 passed，0 failed |
| `cargo test --lib catalog_row_explicit_primary_key_roundtrip` | 修改后 | 0 | 1 passed，0 failed |

四条命令逐条执行。未跑全量。

**Persisted Evidence**

None required。

**Experience Candidates**

None。

**Issue 候选**

None。

**未解决问题**

无。

## Plan Review

- Review Result: accepted

**Findings**

无阻塞项，无 Minor，无 Issue 候选。

`tests/legacy_compat_test.rs` 相对 HEAD 的 diff 只有四处：`g1_tables_row_len` 的注释，`legacy_db_write_upgrades_touched_table_row_to_current_layout` 的文档注释，以及这两条测试的长度和尾段断言。`users.len()` 为 `g1_tables_row_len + 10`，尾段为 8 个 `0x00`、`0x01`、`0x00`。`people_row.len()` 为 `g1_tables_row_len + 10`，尾段为 `[0, 0, 0, 0, 0, 0, 0, 0, 0x01, 0x00]`。两处断言说明都写明末字节是 `explicit_primary_key = 0`。`audit` 与 `events` 仍等于 `g1_tables_row_len`，`notes` 仍等于 `g2_tables_row_len`。`g1_tables_row_len` 的函数体未改，计算停在数据页尾指针。

`serialize_catalog_row` 在 kind 之后写入 `explicit_primary_key`：真为 `1`，假为 `0`。`deserialize_catalog_row` 在 kind 段消费成功后，下一字节为 `1` 才读成真；缺字节或 `0` 读成假。`update_table_tail` 只改 `data_page_tail`，再调用 `serialize_catalog_row`。`legacy_row_without_explicit_primary_key_byte_reads_false` 仍 `pop` 一次后断言假，且 kind 为 Table。`catalog_row_explicit_primary_key_roundtrip` 仍把写出的真值读回。

`src/storage/catalog.rs` 相对 HEAD 含有上述尾字节，那是本轮之前的差异。该文件最后写入时间是 20:25:15，`table_manager.rs` 是 20:26:31，都早于测试文件 21:14:38 和本 Cycle 文件 21:16:19。本轮 diff 没有这两份产品代码。Iteration 001、002 的 Cycle 文件时间戳也早于 21:14:38。

**Evidence**

沿用本 Cycle Act Response 的验证表，未重跑。测试文件、`catalog.rs` 和 `table_manager.rs` 在该结论写入之后没有再改，声明与当前代码一致。

修改前：

- `cargo test --test legacy_compat_test legacy_db_write_upgrades_touched_table_row_to_current_layout`：退出码 101，1 failed。`tests/legacy_compat_test.rs:441`，left 41，right 40。
- `cargo test --test legacy_compat_test legacy_g2_db_opens_queries_and_writes`：退出码 101，1 failed。`tests/legacy_compat_test.rs:1086`，left 42，right 41。
- `cargo test --lib legacy_row_without_explicit_primary_key_byte_reads_false`：退出码 0，1 passed，0 failed。
- `cargo test --lib catalog_row_explicit_primary_key_roundtrip`：退出码 0，1 passed，0 failed。

修改后这四条都是退出码 0，1 passed，0 failed。

代码位置：`tests/legacy_compat_test.rs` 的 `g1_tables_row_len` 注释（164–165 行）、升级测试文档注释（399–401 行）、`users` 断言（442–455 行）、`audit`/`events`（457–463 行）、`people_row` 断言（1089–1102 行）、`notes`（1103–1108 行）。`src/storage/catalog.rs` 的 `serialize_catalog_row`（1288 行）、`deserialize_catalog_row`（1451–1455 行）、`update_table_tail`（549–563 行），以及两个 lib 测试（2410–2427 行）。

**Follow-up Decision**

五条既有 Acceptance 都有当前代码和沿用的命令结果。无 Acceptance gap。Iteration Plan 保持不变。本审查不创建后继 Cycle，也不展开 Iteration 004。`tasks.md` 里下一项仍是 Iteration 004（较小构建侧与全量回归），目录未创建。

**Next Cycle**

None

**Next Iteration**

None
