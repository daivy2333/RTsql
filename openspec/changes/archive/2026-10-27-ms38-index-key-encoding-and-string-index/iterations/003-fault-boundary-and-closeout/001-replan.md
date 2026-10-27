# Iteration 003 / Cycle 001：收尾验收边界

## Plan Context

- Status: ready
- Cycle Type: replan

**Cycle Scope**

- Change tasks: T8
- Acceptance gaps: 父 Cycle 的 A13 把全局 fmt/clippy 失败定为 blocked，T8 因此不能标完成。用户已把这两条命令移出本变更的完成条件。本 Cycle 要关上的是任务状态和这项决定之间的差距。
- Repair items: None
- Inherited scope: T7 的 A1–A11。T8 已经跑完的 13 批全量、两份 README、OpenSpec 检查，以及本变更新编辑上的格式和 lint。R1–R6 的行为要求不变。fixed32、32 字节键、`secondary_index_key`、002 的类型门和等值路由继续有效。
- Excluded scope: 产品代码。重跑 T7、13 批全量、README、OpenSpec、全局 fmt 和全局 clippy。修改 ISS36 或把它标成已关闭。新 Iteration。000、001、002 的 Cycle 文件。父 Cycle `000-initial.md` 的 Plan Context、Act Response 和 Blocker Handoff。全局里程碑、SNAPSHOT、主规格、`.agents/docs/tasks.md`。

**Objective**

Act Response 为 reported，并写上用户决定。`tasks.md` 的 T8 为 completed。全局门失败仍只留在父 Cycle，不作为本变更的完成条件。

**Background**

父 Cycle 是 `iterations/003-fault-boundary-and-closeout/000-initial.md`。它的 Act Response 为 blocked。当时的 Plan Review 保持 pending，因为是否把全局门移出完成条件要先由用户决定。

用户原话：「这个后续解决，直接收尾吧」。

这句的含义是：ISS36 保持 open，以后另行处置。不在本变更里修全局 fmt/clippy。不把 ISS36 收成本变更的新 Iteration。本变更按已完成的字符串索引工作收尾。全局门的失败保留为已记录的既有缺陷，不再阻塞本变更的验收和收尾。

批准来自这句原话，不是 Gate 跳过。

**Investigation Facts**

- Starting point：父 Cycle 的 Plan Context 保持 ready，本规划不改写它。Act Response 为 blocked。写入本文件之前，该 Cycle 的 Review Result 为 pending，Next Cycle 为 None。000、001、002 的 Review Result 都已是 accepted，本 Cycle 不改那三份文件。
- 下列命令结果沿用父 Cycle 的 Act Response。父 Cycle 的 Plan Review 已核对过覆盖范围。相关文件的修改时间早于该 Cycle 文件（17:24），之后没有再改。本 Cycle 不重跑。
  - `cargo test --offline --test string_index_runtime_test`：12 passed / 0 failed。
  - `cargo test --offline --test index_key_encoding_evaluation_test`：7 passed / 0 failed。T7 没有改产品代码，所以没有加跑 `string_secondary_index_test` 和 `string_index_sql_test`。
  - 全量：`ls tests/*.rs` 去掉目录和 `.rs`，加上 `lib` 和 `bins`，`LC_ALL=C` 排序。`N=156`，`B=13`，每批 12 个目标。`string_index_runtime_test` 在第 11 批。13 批都已运行，每批退出码 0，failed 为 0。`bins` 和 `network_server_test` 是 0 passed / 0 failed，目标都跑过。没有复核，因为第一次 failed 就是 0。
  - `cargo clippy --offline --all-targets`（不加 `-D warnings`）退出码 0。本变更新文件的 warning 为 0。`ddl_dml.rs` 只剩第 1482 行那一条，不在本变更编辑上。
  - `openspec validate 2026-10-27-ms38-index-key-encoding-and-string-index --strict --no-interactive` 输出含 `is valid`，退出码 0。
  - `openspec status --change 2026-10-27-ms38-index-key-encoding-and-string-index` 为 4/4，退出码 0。
- 全局门的失败原文留在父 Cycle 的 Blocker Handoff，这里只保留判定所需的结果，不搬整段输出。
  - `cargo fmt --check` 退出码 1。点名文件：`src/storage/data/table_manager.rs`、`tests/hash_join_build_side_test.rs`、`tests/sort_merge_join_test.rs`。
  - `cargo clippy --offline --all-targets -- -D warnings` 退出码 101。`could not compile rtsql (lib) due to 20 previous errors`。点名文件：`src/pipeline.rs`、`src/cli/lifecycle.rs`、`src/executor/alter_table.rs`、`src/executor/executor_trait.rs`、`src/executor/sort_merge_join.rs`、`src/network/protocol.rs`、`src/parser/planner/ddl_dml.rs`、`src/storage/async_storage.rs`、`src/storage/buffer_pool.rs`、`src/storage/data/table_manager.rs`、`src/transaction/manager.rs`。deny 在 lib 停止，测试目标没有被这条命令 lint。
- `.agents/issues/ISS36-fmt-clippy-gates-unsatisfiable.md` 的 Status 为 open。本 Cycle 不改这个文件。
- Current-State Evidence：本规划读了工作区和点名位置，没有跑测试。
  - `tests/*.rs` 仍是 154 个。加上 `lib` 和 `bins` 为 156，与父 Cycle 的批表一致。
  - 未暂存 diff 只有 `README.md`、`README.zh-CN.md`、`benches/index_key_encoding_bench.rs`、`src/parser/planner/query.rs`、`tests/index_key_encoding_evaluation_test.rs`、`tests/support/index_key_encoding.rs`。`tests/string_index_runtime_test.rs` 是未跟踪新文件。
  - `query.rs` 的未暂存 diff 只调整了 `PhysicalPlan` 和 `ProjectionItem` 的 import 换行。评估文件的未暂存 diff 是 `matches!`、`sort_by_key`、`std::slice::from_ref`、`is_multiple_of`，并删除 `assert!(-1 < 1)`。
  - `insert.rs`、`update.rs`、`delete.rs`、`upsert.rs`、`create_index.rs`、`recovery.rs`、`transaction/manager.rs`、`pipeline.rs`、`value.rs`、`index_manager.rs`、`ddl_dml.rs`、`table_manager.rs` 没有未暂存 diff。
  - `table_manager.rs` 第 1358 行和第 1464 行调用 `secondary_index_key`。第 732 行是 `create_table_with_constraints(...).await`。父 Cycle 审查记录的 rustfmt 差异是这一处换行，不在 `secondary_index_key` 上。
  - `ddl_dml.rs` 第 1482 行是 ADD COLUMN 默认值分支里的 `if default_value.is_none()`。`build_create_index` 的类型门在第 1953–1969 行。
  - `README.md` 与 `README.zh-CN.md` 第 263 行已经写明：单列非唯一 `CREATE INDEX` 接受 INTEGER 和字符串声明（TEXT、VARCHAR、CHAR、CLOB）；完整键编码放得进 32 字节时走索引；更长的字符串整值写入并扫描该完整字面量；其他列进入溢出页时回落不变；不截断；不给列加长度上限；NULL 不入索引；重复字符串允许；PRIMARY KEY 与 UNIQUE 仍只接受整数。
  - `tests/string_index_runtime_test.rs` 含 T7 点名的 12 个测试。断言正文在父 Cycle 审查时已核对，该文件的修改时间早于那次审查。
  - 本变更的 delta specs 没有把全局门写成行为要求。R1–R6 不因这次边界变化而裁剪，需求追踪表保持 Covered，没有 Simplified。
- Code and Critical Path：本 Cycle 不进入产品调用链。写入位置只有两处。
  - 本文件的 `## Act Response`。
  - `openspec/changes/2026-10-27-ms38-index-key-encoding-and-string-index/tasks.md` 任务表中 T8 的状态单元格。

**Implementation Guidance**

先读任务表的 T8 行，确认状态单元格是 `blocked`。然后填写本文件的 Act Response，状态写 `reported`。最后只把该单元格改成 `completed`。

不要改任务表的其他行。003 的批次状态已经写明全局门失败不再是完成条件，并且让读者到任务表看 T8 状态。那一条保持不动。

**Task Definitions**

### T8: 把用户决定写入收尾状态

- Requirement/Scenario: R5、R6 的行为已经由父 Cycle 的 T7 和 13 批全量覆盖。本任务不新增行为。它只让 T8 的完成状态和用户决定一致。
- Depends on: 父 Cycle 中 T7 已完成。13 批全量、两份 README 和 OpenSpec 检查已记录。这些都不重跑。
- Targets:
  - 本文件的 `## Act Response`
  - `openspec/changes/2026-10-27-ms38-index-key-encoding-and-string-index/tasks.md` 任务表的 T8 状态单元格
- Current behavior: 父 Cycle 的 Act Response 为 blocked。任务表的 T8 为 blocked。ISS36 为 open。字符串二级索引的产品行为已经落地。全局 fmt 退出码 1，deny clippy 退出码 101，原文在父 Cycle 的 Blocker Handoff。
- Required behavior:
  - 本文件 Act Response 的 Status 为 `reported`。
  - Implemented 写明没有改产品代码，也没有重跑 T7、13 批全量、OpenSpec、全局 fmt 或全局 clippy。
  - 写入用户原话「这个后续解决，直接收尾吧」。写明 ISS36 保持 open，不在本变更修那两条全局命令，不把 ISS36 收成新 Iteration，失败原文留在父 Cycle 的 Blocker Handoff，不再阻塞本变更验收。
  - 不写全局门通过。不写 ISS36 已关闭。
  - 任务表 T8 的状态单元格从 `blocked` 改为 `completed`。同一行的交付说明可以保留。
  - Experience Candidates 写 None。Persisted Evidence 写 None required。
- Preserve: 父 Cycle 的 Plan Context、Act Response 和 Blocker Handoff。000、001、002 的 Cycle 文件。ISS36 文件及其 open 状态。R1–R6 和需求追踪表。两份 README。产品代码。已记录的 13 批计数。`tasks.md` 里除 T8 状态单元格以外的段落。
- Forbidden: 修改 `src/`、`tests/`、`benches/` 和两份 README。运行 cargo、rustfmt、clippy 或 openspec。修改 `.agents/issues/ISS36-fmt-clippy-gates-unsatisfiable.md`。修改 `openspec/specs/`、`proposal.md`、`design.md`、`.agents/docs/SNAPSHOT.md` 和 `.agents/docs/tasks.md`。创建 Iteration 004 或另一个 Cycle。改写 `000-initial.md`。把 T8 写成 `completed` 以外的新状态。宣称两条全局命令退出码为 0。
- First test: 不建立产品测试。动手前只读任务表的 T8 行，状态单元格应为 `blocked`。本文件的 Act Response 此时尚未由 Act 填写。纯文档无从建立先行测试，不适用「没有先行测试不变更」。不要运行 `cargo test`。若 T8 已经不是 `blocked`，停止并返回 Plan。
- GREEN condition: T8 状态单元格为 `completed`。本文件 Act Response 的 Status 为 `reported`，并且含有原话「这个后续解决，直接收尾吧」以及「ISS36 保持 open」。本次新改的路径只有本文件和 `tasks.md`。工作区里原来就有的产品 diff 不算本次改动。
- Verification: 读 T8 行和本文件 Act Response。通过是指状态、原话和 ISS36 仍为 open 的表述都在，而且没有本次新改的产品文件。失败是指 T8 仍为 blocked，Response 不是 reported，或写了全局门通过，或写了 ISS36 已关闭。
- Stop when: 要把 T8 标成 completed 就必须改产品代码、重跑全量、改父 Cycle、改 ISS36 或改主规格。遇到这些情况就返回 Plan，不要做下去。

**Invariants**

- 键仍是 32 字节。叶条目 38 字节，内部分隔条目 36 字节。页类型仍是 `0x01` / `0x02`。
- `Value::to_key` 仍只接受整数。二级字符串只经 `secondary_index_key`。
- 非零原文超过 30 字节，或转义后超过 32 字节，不入树。查询走完整字面量扫描。
- 取消、附加库索引 DDL、只读、内存和加密记录步长保持父 Cycle 已经验收的行为。
- 全量预算已在父 Cycle 用掉。本 Cycle 不再跑第二次全量。
- ISS36 继续 open。

**Non-goals**

产品修复。全局格式化或全局 lint 清理。关闭 ISS36。新 Iteration。主规格合并。重跑已经通过的命令。

**Acceptance**

- 保留已通过的 T7 定向结果和 T8 的 13 批全量、两份 README、OpenSpec 检查。不要求重跑。来源是父 Cycle 的 Act Response：`string_index_runtime_test` 为 12 passed / 0 failed；13 批每批退出码 0、failed 为 0；两份 README 第 263 行已写明字符串类型和超容量扫描；`openspec validate` 含 `is valid`，`openspec status` 为 4/4。
- 全局门的失败原文保留在父 Cycle `iterations/003-fault-boundary-and-closeout/000-initial.md` 的 Blocker Handoff。`cargo fmt --check` 退出码 1，`cargo clippy --offline --all-targets -- -D warnings` 退出码 101。本变更不修这两条命令，不清 ISS36 基线，不把 ISS36 标成已关闭。
- 本 Cycle 没有产品代码任务。Act 把用户原话「这个后续解决，直接收尾吧」写入本 Cycle 的 Act Response，状态为 `reported`，并把 `tasks.md` 里 T8 从 `blocked` 改为 `completed`。不要求 Act 改代码或重跑全量。

**Verification**

前两条沿用父 Cycle 已记录的命令结果，用读文件确认它们还在，不重跑。第三条读本文件 Act Response 和任务表的 T8 状态。没有身份证明字段，也没有单独的判定脚本。

**Gate 2 Readiness**

PASS。没有 BLOCKED。没有 SKIPPED。已跑过的 T7、13 批全量、两份 README 和 OpenSpec 检查继续作为证据，不重跑，也不算跳过。批准来自用户原话「这个后续解决，直接收尾吧」，不是跳过。

**Persisted Evidence**

- Mode: none

验收就是 Act Response 里的用户原话，以及 `tasks.md` 里 T8 的状态。这两处本身就是记录。再读一次文件就能复核，没有即将消失的现场。缺少 evidence 目录不会改变能否收尾的判断。因此不设 required，也不创建 evidence 目录。

**Risks and Notes**

- 工作区里有大量更早的未提交 diff。判断本次有没有改产品代码时，只看本次写入有没有碰到 `src/`、`tests/`、`benches/` 和两份 README。不要求把工作区清干净。
- 父 Cycle 的 Unresolved 写了「本变更新文件没有被这两条命令点名」。全局命令点了 `table_manager.rs` 和 `ddl_dml.rs`，点名位置不在本变更的新编辑上。父 Cycle 已把这个表述宽度记为非阻塞。本 Cycle 不改那句。
- `explicit_tx_timeout_keeps_string_index_until_caller_ends_tx` 的提交半程使用空表 `t3`。父 Cycle 已记为非阻塞。本 Cycle 不改测试。
- R1–R6 没有 Simplified。delta specs 不改。

## Act Response

- Status: reported

**Implemented**

没有改产品代码。没有重跑 T7、13 批全量、OpenSpec、全局 fmt 或全局 clippy。

用户原话：「这个后续解决，直接收尾吧」。

ISS36 保持 open。不在本变更修那两条全局命令，不把 ISS36 收成新 Iteration。失败原文留在父 Cycle `iterations/003-fault-boundary-and-closeout/000-initial.md` 的 Blocker Handoff，不再阻塞本变更验收。

本次写入：

- 本文件的 Act Response
- `openspec/changes/2026-10-27-ms38-index-key-encoding-and-string-index/tasks.md` 任务表中 T8 的状态单元格，从 `blocked` 改为 `completed`。同一行的交付说明保留。

父 Cycle、000/001/002 的 Cycle、ISS36、proposal、design、主规格、SNAPSHOT 和全局 tasks 保持原样。没有创建新 Cycle 或新 Iteration。

**Deviations from Plan**

None。动手前任务表 T8 的状态单元格仍是 `blocked`，本文件的 Act Response 尚未填写。

**Self-review**

Gate 3：本任务不新增行为。R5、R6 仍由父 Cycle 已记录的 T7 和 13 批全量覆盖。先行测试是读任务表，T8 为 `blocked`。纯文档无从建立产品先行测试。没有产品 diff。没有遗留的 Critical 或 Important 问题。

Gate 5 未命中。没有改产品代码、重跑全量、改父 Cycle、改 ISS36 或改主规格。

**Verification**

沿用父 Cycle Act Response，本 Cycle 不重跑：`string_index_runtime_test` 为 12 passed / 0 failed；13 批每批退出码 0、failed 为 0；两份 README 第 263 行已写明字符串类型和超容量扫描；`openspec validate` 含 `is valid`，`openspec status` 为 4/4。

全局门的失败原文留在父 Cycle 的 Blocker Handoff。`cargo fmt --check` 退出码 1，`cargo clippy --offline --all-targets -- -D warnings` 退出码 101。

本 Cycle 的核对是读文件：Act Response 的 Status 为 `reported`，含原话「这个后续解决，直接收尾吧」以及「ISS36 保持 open」。任务表 T8 的状态单元格为 `completed`。本次新改路径只有本文件和 `tasks.md`。

**Persisted Evidence**

None required。

**Experience Candidates**

None。

**Unresolved**

ISS36 保持 open，以后另行处置。全局 fmt 与 deny clippy 的失败原文仍在父 Cycle 的 Blocker Handoff，不再阻塞本变更验收。

## Plan Review

- Review Result: accepted

**Findings**

修订后的验收已经满足。本 Cycle 的 Act Response 与 `tasks.md` 的 T8 行一致，没有阻塞项。

Act Response 的 Status 是 `reported`。Implemented 写明没有改产品代码，也没有重跑 T7、13 批全量、OpenSpec、全局 fmt 或全局 clippy。正文有用户原话「这个后续解决，直接收尾吧」，并写明 ISS36 保持 open，不在本变更修那两条全局命令，不把 ISS36 收成新 Iteration，失败原文留在父 Cycle 的 Blocker Handoff。没有写全局门通过，也没有写 ISS36 已关闭。Experience Candidates 是 None。Persisted Evidence 是 None required。

`tasks.md` 任务表里 T8 的状态单元格是 `completed`。同一行仍写着全局 fmt/clippy 失败留在 003/000 的 Blocker Handoff，不再作为完成条件。T1–T7 仍是 `completed`。Iteration 003 仍是 T7 与 T8，并写明这是最后一轮。

父 Cycle `000-initial.md` 的 Act Response 仍是 `blocked`，Review Result 仍是 `replan-required`。它的 Blocker Handoff 仍记录 `cargo fmt --check` 退出码 1，以及 `cargo clippy --offline --all-targets -- -D warnings` 退出码 101。000、001、002 的 Cycle 没有在这次写入中被改。ISS36 的 Status 仍是 open。

没有 Act 偏离，也没有需要新执行依据的缺口。

非阻塞：`tasks.md` 第 12 行仍称 003 为当前可执行轮次，第 94 行仍有「本次规划不调用 Act」。这两句是规划写入时留下的。执行依据要求 Act 不要改 T8 状态单元格以外的段落。它们不改变 T8 已完成这一结果。

**Evidence**

- 本审查读了本文件的 Act Response 和 `tasks.md` 全文。T8 状态单元格为 `completed`。
- 沿用父 Cycle 的 Act Response，覆盖未变，不重跑：`string_index_runtime_test` 为 12 passed / 0 failed；13 批每批退出码 0、failed 为 0；`openspec validate` 含 `is valid`，`openspec status` 为 4/4。父 Cycle 另记录 `index_key_encoding_evaluation_test` 为 7 passed / 0 failed，以及不加 `-D warnings` 的 clippy 退出码 0。
- 另读：两份 README 第 263 行仍写明字符串声明、32 字节内走索引、超长整值扫描、溢出页回落、不截断、NULL 不入索引、重复字符串允许，以及 PRIMARY KEY 与 UNIQUE 仍只接受整数。ISS36 的 Status 为 open，文件时间是 2026-10-25 22:27:37。`tests/*.rs` 为 154 个。`tests/string_index_runtime_test.rs` 里仍有 T7 的 12 个测试。
- 文件时间：父 Cycle 为 17:45:16。此后只有本文件（17:48:12）和 `tasks.md`（17:48:15）被写入。`string_index_runtime_test.rs` 为 16:53:23，两份 README 不晚于 16:57:16，`ddl_dml.rs` 为 16:03:34，`table_manager.rs` 为 15:26:08。这些时间早于父 Cycle 所沿用的结论，覆盖没有变化。

**Follow-up Decision**

修订后的三条验收都有对应记录。当前 Cycle 不需要再改。不创建 rework 或 replan。Iteration 003 是本变更最后一轮，没有后继 Iteration。

**Next Cycle**

None

**Next Iteration**

None
