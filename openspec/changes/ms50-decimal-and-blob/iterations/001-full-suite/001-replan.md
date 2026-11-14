# Iteration 001 / Cycle 001: 中止夹具让出 BLOB 标签

## Plan Context

- Status: ready
- Cycle Type: replan

**Cycle Scope**

- Change tasks: T5
- Acceptance gaps: `test_abort_update_reverts_to_previous_version` 的 v1 负载首字节是 `0x0A`。中止后 `index_manager.search` 得到 `None`，断言右侧是 `Some(RowId { page_id: 2, slot_id: 0 })`。
- Repair items: None
- Inherited scope: 已落地的 DECIMAL 与 BLOB 声明、写入、比较和读回。值标签 `0x09`、`0x0A`、`0x0B`。格式版本 1。任何 `cargo test` 带 `--test-threads=1`。父 Cycle 已经跑过的那一次全量，包括各批退出码。
- Excluded scope: 第二次全量；重跑已经通过的批；补跑 B08 里没有运行的 10 个目标；修改值标签、`deserialize_tuple_width` 或 `abort_cleanup_versions`；修改产品代码；合并 master；给 DECIMAL 或 BLOB 增加行为。

**Objective**

这条中止夹具的前版本重新不能按表解码。中止后，手写键 `b"test_key"` 的索引查找结果回到 v1 的 `RowId`。标签和中止分支保持不变。这一次全量已经发生，本 Cycle 只证明这处修复，不声明第二次全量，也不声明那一次全量通过。

**Background**

父 Cycle 是 `iterations/001-full-suite/000-initial.md`。Plan Context 为 ready，Act Response 为 blocked，Review Result 为 `replan-required`。T4 按 M19 跑了 15 批。B00–B07 与 B09–B14 退出码 0，failed 为 0。B08 首跑和一次整批复核都是退出码 101。失败用例就是上面这一条。`mvcc_commit_test`、`mvcc_record_test`、`mvcc_tombstone_visibility_test`、`nested_loop_join_test`、`network_protocol_test`、`network_server_test`、`no_from_select_test`、`order_by_alias_sort_test`、`outer_join_test`、`overflow_page_test` 在 B08 里两次都没有运行，不算通过。

父 Cycle 禁止改产品代码和测试，也禁止第二遍全量。要让该断言通过，必须改夹具字节。这超出父 Cycle 的执行依据，所以剩余证明改成这一条定向测试。

**Investigation Facts**

- Starting point: 分支 `changes/ms50-decimal-and-blob`，HEAD `64352e33b5ec914e53ca631fcd965198308499e4`。审查时工作区干净。相对全量开跑时的 `354aed6d997a6a1333665ea8d281d9a165a31c13`，`src/` 和 `tests/` 没有 diff。B08 的失败观察仍然覆盖当前产品代码。本 Cycle 改代码前不再跑这条测试。
- Current-State Evidence:
  - `tests/mvcc_abort_test.rs` 相对 merge-base `f323d58a452ce34467d2e33fd6dda5cebe798b56` 没有 diff。失败发生在 `test_abort_update_reverts_to_previous_version`。表只有一个 `Int` 列 `id`。v1 负载是 `[0x0A, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]`。索引键是手写的 `b"test_key"`，不是列值编码。中止后的断言要求 `search(key) == Some(row_id_v1)`。失败时左侧是 `None`，右侧是 `Some(RowId { page_id: 2, slot_id: 0 })`。同一文件另两个测试在 B08 里已经通过：该目标的汇总是 `2 passed; 1 failed`。
  - `src/storage/page_format/tuple.rs` 的 `TAG_BLOB` 是 `0x0A`。`deserialize_tuple_width` 不按列类型核对标签。`0x0A` 后接小端 `u16` 长度 `0`，这 8 字节解码成空 `Value::Blob`，实际列数为 1，函数返回 `Ok`。变更前 `0x0A` 落入未知标签，函数返回错误。
  - `src/transaction/manager.rs` 的 `read_prev_index_keys` 在 `deserialize_tuple_width` 失败时返回 `Ok(None)`。注释写明 `mvcc_abort_test` 的手写键与 tuple 内容无关，不可解码时必须走原来的回退。`abort_cleanup_versions` 里，`derived` 为 `None` 且存在前版本时，调用 `index_manager.update(&key, prev)`。
  - 解码成功后，`src/wal/recovery.rs` 的 `extract_index_keys` 用 `Value::to_key`。`Value::Blob` 的 `to_key` 返回 `None`。于是 `derived` 是 `Some((None, …))`。对应分支是 `(Some(None), Some(_))`，只执行 `index_manager.delete(&key)`。查找因此得到 `None`。
  - 同文件 v2 的首字节是 `0x14`，不是值标签。中止清理读的是 v1。另两个测试的负载首字节是 `0x01`，走的是没有前版本的插入中止，B08 已通过。`tests/` 里只有这一处 `0x0A` 负载。
  - 真实行经 `serialize_tuple` 写入标签。`Int` 行的首字节是 `0x01`，不经过这条手写负载。
- Code and Critical Path: 测试在 `setup` 之后直接调用 `write_tuple_to_data_page`、`index_manager.update` 和 `TransactionManager::abort`。不经过 `UpdateExecutor`。中止进入 `abort_cleanup_versions`，再读 v1 的 `read_prev_index_keys`。本 Cycle 只改 v1 那 8 字节的首字节，使 `deserialize_tuple_width` 重新返回错误，从而回到 `update(&key, prev)`。

**Implementation Guidance**

只改 `tuple_bytes_v1` 的第一个字节和紧挨着的那句注释。其余 7 字节保持 `0x00`。不要改 v2、索引键、断言、同文件其他测试或产品代码。注释不再把这 8 字节写成整数 10，写明首字节不是值标签，前版本因此不可解码。

**Task Definitions**

### T5: 中止夹具的前版本重新不可解码

- Requirement/Scenario: `tasks.md` 的 Iteration 001。这一次全量已经记录 B08 失败。本任务只闭合该失败用例的标签碰撞。
- Depends on: T4 的 15 批已经启动，并且 B08 的两次结果已经写入父 Cycle。
- Targets: `tests/mvcc_abort_test.rs` 的 `test_abort_update_reverts_to_previous_version` 中 `tuple_bytes_v1`。
- Current behavior: v1 首字节 `0x0A` 被解码成空 BLOB。中止删除 `b"test_key"`。`search` 返回 `None`。
- Required behavior: v1 首字节是 `0x0C`。`deserialize_tuple_width` 对这 8 字节返回错误。中止后 `search(b"test_key")` 等于 v1 的 `RowId`。断言文本和比较对象不变。
- Preserve: `TAG_DECIMAL = 0x09`、`TAG_BLOB = 0x0A`、`TAG_LONG_BLOB = 0x0B`。`abort_cleanup_versions` 的五个匹配分支。`deserialize_tuple_width` 的自描述解码。同文件另两个测试的字节和断言。父 Cycle 记录的各批结果。
- Forbidden: 修改 `src/`；把首字节改成 `0x01` 到 `0x0B` 中的任何一个；补跑 B08 的 10 个未运行目标；重跑已通过的批；第二遍全量；默认并行度；合并 master。
- First test: 不新写测试。RED 沿用父 Cycle 的 B08：同一断言两次失败，左侧 `None`，右侧 `Some(RowId { page_id: 2, slot_id: 0 })`。产品代码自该观察之后没有变化，改前不再运行 `cargo test --test mvcc_abort_test test_abort_update_reverts_to_previous_version -- --test-threads=1`。
- GREEN condition: 改完首字节后，上面这条命令退出码 0，且该进程 `test result:` 汇总行的 failed 是 0。
- Verification: 只运行这一条命令。退出码非 0 或 failed 非 0 表示夹具仍未回到回退路径，或断言变成了别的失败。
- Stop when: 要通过就必须修改值标签、`deserialize_tuple_width` 或 `abort_cleanup_versions`。或者定向命令的失败不再是「左侧 `None`、右侧为 v1 的 `RowId`」这一条。两种情况都停止并回到计划，不改产品代码，也不再开全量。

**Invariants**

- 值标签和中止清理的分支与本 Cycle 开始时相同。
- 这条命令带 `--test-threads=1`。
- 不把父 Cycle 的全量改记成通过，也不把未运行的 10 个目标记成通过。

**Non-goals**

第二次全量、补跑未运行目标、修改产品代码、调整标签或中止语义。

**Acceptance**

- `tuple_bytes_v1` 的首字节是 `0x0C`，其后 7 字节仍是 `0x00`。
- 定向命令退出码 0，汇总行 failed 为 0。
- `src/` 没有改动。`TAG_BLOB` 仍是 `0x0A`。
- 父 Cycle 的批结果保持原记录。本 Cycle 不声明全量通过。

**Verification**

改后的唯一命令：

```text
cargo test --test mvcc_abort_test test_abort_update_reverts_to_previous_version -- --test-threads=1
```

通过条件是该 cargo 进程退出码 0，且输出中 `test result:` 汇总行的 failed 为 0。这不是全量，也不代替 B08。

**Gate 2 Readiness**

PASS。无 BLOCKED，无 SKIPPED。

计划批准来自目标原文「那就开始规划和实施这个ms，要求遵守各种规范，尤其是多批次串行测试规范，本次最多进行一次全量」，以及本次审查授权里「若要修复：创建后继 Cycle 或 Iteration，执行依据必须是定向测试，不能再包含一遍全量」。不是跳过。检查项已对照父 Cycle、`tasks.md` 的修订后 Iteration 001 和 `design.md` 的「中止夹具不占用值标签」。

**Persisted Evidence**

- Mode: none

定向命令的退出码和 failed 计数写入 Act Response。

**Risks and Notes**

- 首字节如果改成 `0x01`，负载会解码成整数，派生键与 `b"test_key"` 不相等，清理会删掉手写键，断言仍然是 `None`。所以指定 `0x0C`，不留字节选择。
- `0x03` 会解码成 NULL，同样落入只删除的分支。`0x0C` 避开全部已分配值标签。
- B08 未运行的 10 个目标与这条夹具没有同一个 `0x0A` 负载。本 Cycle 不把它们补进验证。

## Act Response

- Status: reported

**实际改动**

- T5 完成。`tests/mvcc_abort_test.rs` 的 `test_abort_update_reverts_to_previous_version` 中，`tuple_bytes_v1` 首字节从 `0x0A` 改为 `0x0C`，其后 7 字节仍是 `0x00`。同行注释不再把这 8 字节写成整数 10，写明首字节不是值标签，前版本因此不可解码。
- 断言文本、比较对象、v2 的 `0x14`、同文件另两个测试和 `src/` 未改。`TAG_BLOB` 仍是 `0x0A`。
- `tasks.md` 只更新本轮状态：T5 改为完成，5.1 勾选。

**偏差**

无。

**自检**

Gate 3：RED 沿用父 Cycle 的 B08，改前没有重跑。diff 只有 v1 首字节和同行注释，以及本 Response 与 `tasks.md` 的状态。断言仍比较 `search` 结果和 `Some(row_id_v1)`，文本仍是 `v1 should still be visible after tx2 abort - index should point back to v1`。GREEN 的 failed 为 0，证明中止后索引指回 v1。没有改值标签、解码或中止分支。没有新增身份证明机制或判定层。无未修复的 Critical 或 Important。

Gate 4：定向命令退出码 0，汇总行 failed 为 0。Gate 5：未命中。

全量：`SKIPPED: 本 Cycle 排除第二次全量。父 Cycle 的那一次全量保持原记录，不声明通过。B08 未运行的 10 个目标不补跑。`

**验证**

RED：沿用父 Cycle `iterations/001-full-suite/000-initial.md` 的 B08。同一断言两次失败，left 是 `None`，right 是 `Some(RowId { page_id: 2, slot_id: 0 })`。改前 `git diff 64352e33b5ec914e53ca631fcd965198308499e4 -- src tests/mvcc_abort_test.rs` 为空。产品代码自该观察之后没有变化，没有重跑这条测试。

GREEN：

- `cargo test --test mvcc_abort_test test_abort_update_reverts_to_previous_version -- --test-threads=1`：退出码 0。`test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out`。failed 是 0。只跑了这一次。该文件另两个测试在 2 filtered out 里，没有运行。

全量未跑。

**Persisted Evidence**

None required。

**Experience Candidates**

None

**未解决**

本 Cycle 的 Acceptance 没有剩余项。父 Cycle 的 T4 仍是 blocked：这一次全量不声明通过，B08 未运行的 10 个目标不算通过。本 Cycle 不处理这两项。
