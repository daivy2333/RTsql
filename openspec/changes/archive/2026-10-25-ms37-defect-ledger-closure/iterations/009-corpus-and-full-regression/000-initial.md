# Iteration 009 / Cycle 000: 语料收口与收尾全量回归

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T13（ISS34）、T14（I064）、T15（I045）、T16（本 change 唯一一次全量回归）
- Acceptance gaps: None
- Repair items: None
- Inherited scope:
  - change `design.md` **D9**（标题对齐与 Purpose 是交接，不改本 change 的增量规格标题；WAL 边界只写现有承诺的界限）。**不得另选方向**。
  - change `design.md` **D10** 与 change `tasks.md`「收尾验证」（全量只在本 Iteration 跑一次，分批串行）。**不得另选跑法**。
  - `index-read-bounded-read` 增量继续逐字保留既有 Requirement 标题和既有 Scenario 标题。归档校验要求当前主规格里的每个 Scenario 标题在 `MODIFIED` 块里逐字出现。
- Excluded scope:
  - 产品代码、测试代码、磁盘格式。
  - 主规格库 `openspec/specs/` 的直接修改。T13 与 T15 只写交接文本，由文档维护在收尾合并时落位。
  - Issue 台账的登记、关闭、重开，以及 Runbook、全局任务、SNAPSHOT、项目记忆。
  - 未登记缺陷的修复，以及为它们新建 Iteration。
  - `cargo fmt --check` 与 `cargo clippy --all-targets` 的仓库级门（`design.md` D13）。

**Objective**

1. 交接文本写明 `index-read-bounded-read` 的 Requirement 标题和 4 个 Scenario 标题如何改成「未通过页级判据的索引树页」，且不改 SHALL、不改 Scenario 数量。
2. 两份 README 的性能对比段写清追加、提交确认、checkpoint 是三件不同的事；`sync_all` 或 SIGKILL 重开不能写成已经验证断电安全。`wal-auto-checkpoint` 的「提交先于 checkpoint」条款补上同一条边界，不新增断电承诺。
3. 13 份仍是归档占位的主 spec Purpose，在交接文本里各有一句能力定位和来源 change。`wal-recovery-replay-integrity` 已有正文，不覆盖。
4. 全量回归按下面的分批规则跑完一次，每一批 0 failed。

**Investigation Facts**

- Starting point:
  - Iteration 008 / Cycle 000 的 Act Response（2026-10-26，`reported`）与本 Plan Review（`accepted`）是起点。008 只改了 `create_table_sql` 的 `UNIQUE` 条件、两条 CLI 用例和两份 README 的存量兼容半句。`restore` 与 `sql_failure_message` 无 diff。定向回归一批 3 个目标后单独 `--lib`，均 `--test-threads=1`，0 failed。全量没有跑。
  - 工作区产品改动未提交。本 Cycle 从当前代码与当前规格实施，不回退 000–008。
- Current-State Evidence（2026-10-26 读取）：
  - 主规格 `openspec/specs/index-read-bounded-read/spec.md` 的 Requirement 标题仍是「损坏的索引页在读 / 写路径上给出明确的错误」。4 个 Scenario 标题仍是「损坏的索引页上的查询不 panic、不静默错读」「损坏的索引页上的插入不 panic」「损坏的索引页上的删除不 panic」「损坏的索引页上的更新不 panic」。正文已经写「未通过页级判据」。另有「未损坏路径的读 / 写行为零变化」，以及本 change 增量里的「叶页条目长度过短的索引树页给出明确的错误」。这两条标题不在 ISS34 的 4 条之内。
  - 本 change 的 `specs/index-read-bounded-read/spec.md` 逐字保留上述旧标题，并在文首说明改名不能写进 `MODIFIED` 块。
  - I045 点名的 14 份主 spec 里，13 份 `## Purpose` 仍是 `TBD - created by archiving change <id>. Update Purpose after archive.`。`wal-recovery-replay-integrity` 已有完整 Purpose，文中没有 TBD / TODO，来源写到 MS10-T02、MS10-T05、MS20-T02、`2026-10-03-recovery-tail-persistence`、`2026-10-03-practical-core-gaps`。
  - 13 份占位及其中的 change id：
    - `cli-noninteractive-shell` ← `2026-09-06-ms10-t01-cli-shell`
    - `database-file-format-header` ← `2026-09-08-ms10-t03-file-format-header`
    - `database-file-lock` ← `2026-09-06-ms10-t02-file-lock-graceful-shutdown`
    - `dml-transaction-lifecycle` ← `2026-08-25-fix-dml-tx-id-injection`
    - `drop-table-physical-free` ← `2026-08-26-ms07-t02-drop-table-physical-free`
    - `ms07-rest-tx-checkpoint-pushdown` ← `2026-08-30-ms07-rest-explicit-tx-checkpoint-pushdown`
    - `pipeline-stage-decomposition` ← `2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages`
    - `plancache-key-normalization` ← `2026-08-25-ms06-t02-plancache-dashmap`
    - `planner-module-decomposition` ← `2026-08-30-ms07-t03-planner-decomposition`
    - `schema-persistence` ← `2026-08-26-ms07-t01-schema-persistence`
    - `storage-io-optimization` ← `2026-09-05-ms08-t01-t02-pread-prefetch`
    - `wal-recovery-frame-parsing` ← `2026-09-06-ms10-t02-file-lock-graceful-shutdown`
    - `wal-writer-handle-reuse` ← `2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages`
  - 句式先例是 `openspec/specs/no-from-select/spec.md` 的 Purpose：一句能力定位，然后 `来源：` 加上 change。
  - `README.md`「Performance and resource comparison」与 `README.zh-CN.md`「性能与资源对比」都写了 2026-09-24 的默认配置对比，INSERT 100 行约 52x，并写明这是单机观察。两段都没有把追加、提交等待和 checkpoint 分开，也没有写断电未验证。
  - 提交等待的代码：`TransactionManager::commit` 先 `WALBuffer::append` 一条 `CommitTxn`，再 `append_commit_and_wait`。`WALBuffer::do_flush` 把批次交给 `WalWriter::write_batch`。`write_batch` 在全部 `write_all` 之后调用一次 `sync_all`，然后通知等待者。并发提交可以进同一批次。`WalWriter::fsync` 的注释写明组提交不走这个方法。checkpoint 是之后把已提交帧折进主文件的另一步；`wal-auto-checkpoint` 主规格写「WAL 已 fsync，提交先于 checkpoint」。
  - 主规格 `openspec/specs/wal-auto-checkpoint/spec.md` 的 Requirement「自动 checkpoint 失败传播」全文是：「自动 checkpoint 的任一步骤失败（I/O 错误等）SHALL 以错误响应报告触发该边界的提交语句，错误 SHALL 点名 checkpoint 失败；该语句的已提交数据 SHALL 不受影响（WAL 已 fsync，提交先于 checkpoint）。失败后水位仍超限，后续提交边界 SHALL 自然重试。」其下只有 Scenario「checkpoint 失败诚实报告且数据不丢」。本 change 还没有 `specs/wal-auto-checkpoint/spec.md`。
  - 集成测试文件数：`ls tests/*.rs | sort` 在 2026-10-26 得到 **148**。这只是计划时的参考。执行时必须重新列文件。前次全量 2231 passed / 0 failed / 2 ignored（143 目标 / 14 批）已经失效，不能当通过条件。
- Code and Critical Path:
  - T13 / T15 不改规格库。交接文件是本 change 的 `corpus-handoff.md`。
  - T14 改两份 README 的性能对比段，并新增本 change 的 `specs/wal-auto-checkpoint/spec.md`。不改 `src/wal/**`，不改主规格。
  - T16 的入口是 `cargo test` 的分批调用。判定是每一批的退出码和该批输出里的 `test result:` 行。

**Implementation Guidance**

1. 顺序固定：T13 → T14 → T15 → T16。全量放在最后，并且整个 change 只跑这一次。
2. T13 与 T15 写同一份交接文件。T14 的 README 句子和边界 Scenario 用下面 Task Definition 里的原文，不要改承诺强弱。
3. 文档任务没有可先失败的产品测试。改前核对当前标题、占位句和 README 段落仍是上面写的那样；对不上就停止。

**Task Definitions**

### T13: IRBR 五个标题的交接文本可逐字套用（ISS34）

- Requirement/Scenario: IRBR 的 Requirement 与 4 个 Scenario 标题措辞（ISS34）
- Depends on: None
- Targets:
  - 新建 `openspec/changes/2026-10-25-ms37-defect-ledger-closure/corpus-handoff.md`
  - 只读：`openspec/specs/index-read-bounded-read/spec.md` 与本 change 的 `specs/index-read-bounded-read/spec.md`
- Current behavior:
  - 主规格 Requirement 标题和 4 个 Scenario 标题仍写「损坏的索引页」。正文写「未通过页级判据」。
  - 本 change 的增量保留旧标题。
- Required behavior:
  1. 交接文件列出五对逐字替换，旧串必须与主规格当前标题一致：
     - Requirement：`损坏的索引页在读 / 写路径上给出明确的错误` → `未通过页级判据的索引树页在读 / 写路径上给出明确的错误`
     - Scenario：`损坏的索引页上的查询不 panic、不静默错读` → `未通过页级判据的索引树页上的查询不 panic、不静默错读`
     - Scenario：`损坏的索引页上的插入不 panic` → `未通过页级判据的索引树页上的插入不 panic`
     - Scenario：`损坏的索引页上的删除不 panic` → `未通过页级判据的索引树页上的删除不 panic`
     - Scenario：`损坏的索引页上的更新不 panic` → `未通过页级判据的索引树页上的更新不 panic`
  2. 写明落位顺序：合并时先按增量套用（增量里仍是旧标题，才能通过「当前标题必须出现在 `MODIFIED` 块」的校验），再在合并后的主规格上做这五处替换。先改主规格标题会使归档拒绝该增量。
  3. 写明不改：SHALL 正文、GIVEN / WHEN / THEN、Scenario 数量、Requirement 数量；「未损坏路径的读 / 写行为零变化」和「叶页条目长度过短的索引树页给出明确的错误」两个标题不动。
- Preserve:
  - 本 change 增量文件里的旧标题，一个字都不改。
  - 主规格库文件，本 Cycle 不写。
- Forbidden:
  - 把改名写进 `specs/index-read-bounded-read/spec.md` 的 `MODIFIED` 标题。
  - 改 GIVEN 里的「页内容已损坏」。那是注入页的描述，不是这 4 个标题。
  - 调用文档维护去当场改主规格。本任务只留下交接文本。
- First test:
  - 改交接文件之前，读主规格，确认上面五个旧标题都在，且增量文件仍使用旧标题。这是文档核对，没有产品 RED。
- GREEN condition:
  - 交接文件里的五对新旧串与上面一致，并写了落位顺序。增量文件的标题 diff 为空。主规格 diff 为空。
- Verification:
  - 读交接文件和两份规格的标题。不跑测试。
- Stop when: 主规格标题已经不是上面的旧串；或必须改 SHALL / GIVEN 才能让标题和正文一致。

### T14: README 与 wal-auto-checkpoint 写明三件承诺的边界（I064）

- Requirement/Scenario: I064 的文档边界（追加 / 提交确认 / checkpoint；`sync_all` 与 SIGKILL 不是断电验证）
- Depends on: T13
- Targets:
  - `README.md` 的「Performance and resource comparison」
  - `README.zh-CN.md` 的「性能与资源对比」
  - 新建 `openspec/changes/2026-10-25-ms37-defect-ledger-closure/specs/wal-auto-checkpoint/spec.md`
- Current behavior:
  - 两份 README 都有默认配置下的 52x INSERT 对比，没有三件承诺的区分，也没有断电未验证这句。
  - `wal-auto-checkpoint` 已写提交的 `sync_all` 先于 checkpoint，没有「这不等于断电验证」的 Scenario。
- Required behavior:
  1. 在英文段「These are single-machine observations, not benchmark guarantees.」之后、表格之前，插入下面这段，不改表格数字：

     `Appending a frame, a commit wait returning, and a checkpoint are three different promises. The commit wait returns after that batch's sync_all. Concurrent commits can share one batch, so the per-statement INSERT row is not a count of one fsync per statement. Both engines are on their default journal and sync settings. Seeing sync_all run, or killing the process with SIGKILL and reopening, does not show that power loss has been tested.`

     代码标识 `sync_all` 与 `SIGKILL` 用反引号，与 README 现有写法一致。
  2. 在中文段「以下为单机现场观察，不是基准保证。」之后、表格之前，插入：

     `追加一帧、提交等待返回、以及 checkpoint，是三件不同的事。提交等待在该批次的 sync_all 返回之后才返回。并发提交可以共用一个批次，所以上表逐条 INSERT 的数字不是每条语句一次 fsync 的计数。两边都使用各自的默认日志和同步配置。只看到 sync_all 被调用，或者用 SIGKILL 杀掉进程再打开，都不能说明已经验证过断电安全。`

     `sync_all` 与 `SIGKILL` 同样用反引号。
  3. 新建增量规格，只含一个 `MODIFIED` Requirement，标题「自动 checkpoint 失败传播」。Requirement 正文与现有 Scenario「checkpoint 失败诚实报告且数据不丢」从主规格逐字抄入，然后追加：

     `#### Scenario: 提交确认不包含 checkpoint，也不构成断电验证`

     - **GIVEN** 事务提交已经从 `WALBuffer::append_commit_and_wait` 返回；该返回发生在对应批次 `WalWriter::write_batch` 的 `sync_all` 返回之后
     - **WHEN** 对照本要求里「已提交数据不受影响（WAL 已 fsync，提交先于 checkpoint）」
     - **THEN** 该承诺是：这次 `sync_all` 已经返回，且 checkpoint 尚未被这次提交确认所包含
     - **AND** 只观察到 `sync_all` 被调用，或只用进程 SIGKILL 后重开，都不足以把结果写成已验证的完整断电安全
     - **AND** 本场景不新增断电后仍可恢复的承诺，也不改变 checkpoint 失败后已提交行经重开可查的既有条款
- Preserve:
  - 表格中的时间、倍数和机器描述。
  - 「自动 checkpoint 失败传播」的原 SHALL 句和原 Scenario。
  - `drop-persistence-boundary` 里已有的「不是真实断电」。那是 DROP 的崩溃模型，本任务不改它。
- Forbidden:
  - 改 `src/wal/**`、`src/transaction/**` 或任何产品代码。
  - 写「已经验证断电安全」或任何更强的耐久承诺。
  - 改主规格 `openspec/specs/wal-auto-checkpoint/spec.md`。
  - 为这句文档新增测试、夹具或命令。
- First test:
  - 改之前读两份 README 的该段和主规格该 Requirement，确认还没有上面的段落和 Scenario。文档核对，没有产品 RED。
- GREEN condition:
  - 两段插入句与上面一致，表格数字未改。
  - 增量规格里原 Requirement 正文和原 Scenario 与主规格逐字相同，新 Scenario 只有上面这一条。
- Verification:
  - 读这三处文本。不跑测试。
- Stop when: 必须改产品代码才能让句子成立；或现有条款已经写了完整断电安全，需要先改承诺而不是加边界。

### T15: 13 份占位 Purpose 的交接句，已有 Purpose 不覆盖（I045）

- Requirement/Scenario: I045 的 14 处 Purpose。其中 13 处仍是占位，1 处已经写完。
- Depends on: T14
- Targets:
  - `openspec/changes/2026-10-25-ms37-defect-ledger-closure/corpus-handoff.md`（在 T13 的文件上追加）
  - 只读：上列 14 份 `openspec/specs/<domain>/spec.md` 的 `## Purpose`
- Current behavior:
  - 13 份 Purpose 是归档占位句，句内带有来源 change id。
  - `wal-recovery-replay-integrity` 已有 Purpose，没有 TBD / TODO。
- Required behavior:
  1. 交接文件为 13 份占位各写一条替换。每条包含：域目录名、当前占位全文、新 Purpose。新 Purpose 是一句该域已经写在 Requirements 里的能力定位，加上 `来源：` 和占位句里的那个 change id。句式对齐 `no-from-select`。不新增该域没有的行为，不写 SHALL 清单。
  2. 同一文件记下 `wal-recovery-replay-integrity`：当前 Purpose 保留，本交接不提供替换句。
  3. 写明落位时机与 T13 相同：收尾合并时写入主规格，本 Cycle 不写 `openspec/specs/`。
- Preserve:
  - `wal-recovery-replay-integrity` 的现有 Purpose 原文。
  - 各域 Requirements 正文。
- Forbidden:
  - 覆盖已经写好的 `wal-recovery-replay-integrity` Purpose。
  - 在 Purpose 里加入新的行为承诺、TBD 或 TODO。
  - 修改主规格库。
- First test:
  - 写之前读 14 份 Purpose，确认 13 份仍是占位、1 份已有正文。文档核对，没有产品 RED。
- GREEN condition:
  - 交接文件有 13 条替换，每条都能在对应占位句上整段替换，并带有该占位句里的 change id。
  - 交接文件写明第 14 份不替换。主规格 diff 为空。
- Verification:
  - 读交接文件，并对照 14 份 Purpose。不跑测试。
- Stop when: 某份占位句的 change id 与上面列表不一致；或一句 Purpose 必须发明该域 Requirements 里没有的行为才能写完。

### T16: 本 change 唯一一次全量回归，按分批串行跑完

- Requirement/Scenario: change `tasks.md`「收尾验证」与 `design.md` D10
- Depends on: T15
- Targets:
  - 无产品文件。结果写入本 Cycle 的 Act Response。
- Current behavior:
  - 008 及更早的 Iteration 都没有跑全量。前次 2231 / 0 / 2 的终态已经失效。
  - 计划时 `tests/*.rs` 有 148 个文件。执行时以当时的 `ls` 为准。
- Required behavior:
  1. 先重新生成清单：`ls tests/*.rs | sort`。目标名是文件名去掉目录和 `.rs`。按这个顺序切批，不重不漏。
  2. 文件批用等分规则，保证每批落在 8–12：
     - `N` = 清单条数
     - `B = ceil(N / 12)`
     - 按字典序，前 `B × 12 − N` 批各 11 个目标，其余各 12 个
     - 校验：`11 × (B × 12 − N) + 12 × (N − B × 12 + B) = N`，且每批是 11 或 12
     - 计划时 N = 148 ⇒ B = 13，前 8 批各 11，后 5 批各 12。执行时 N 若不是 148，按同一公式重算，不要把 148 写死。
  3. `--lib` 与 `--bins` 单独成一批，不塞进文件批：`cargo test --lib --bins -- --test-threads=1`。这一批放在文件批之前。文件批的命令是 `cargo test --test <目标> … -- --test-threads=1`。
  4. 批内必须带 `--test-threads=1`。批与批串行：上一批的退出码已经记下，才启动下一批。同一时刻只有一批在跑。
  5. 不得用一次不带目标过滤的 `cargo test` 跑完全量。不得去掉 `--test-threads=1` 用默认并行度跑任何一批。不得同时启动两批。
  6. 每一批记录：批号、目标名、退出码、该批输出中每条 `test result:` 的 passed / failed / ignored。`--lib --bins` 一批会有两条 `test result:`（lib 一条，bin 一条），两条都要记下。
  7. 通过条件：全部批次都已执行，每一批退出码 0，每一条 `test result:` 的 failed 为 0。缺一批不能写成全量通过。覆盖核对：每个 `tests/*.rs` 的目标名恰好出现一次；多出来的、缺的、重复的都是 0。
  8. 某批失败时，只把这一批再跑一次，用来区分是不是偶发失败。复核通过则按通过记录，并写明首跑失败。cargo 会在该批第一个失败目标之后停下，所以复核必须重跑整批。复核仍失败则停止，不启动后续批，也不再跑一整轮。
- Preserve:
  - 本 change 只此一次全量。定向失败不得改用无过滤全量重试。
  - 2231 / 0 / 2 不作为通过条件，也不为了凑这个数字去改测试。
- Forbidden:
  - `cargo test`、`cargo test --workspace`、`cargo test --all-targets` 这类无过滤全量。
  - 省略 `--test-threads=1`。
  - 并行启动多批，或在一批还没结束时启动下一批。
  - 为清点结果新写脚本、封装命令或判定层。计数只用 cargo 自己的退出码和输出；需要过滤时只用一步 `grep -E '^test result:'`。
  - 跑 `--doc`。本规则的全量是 `--lib` / `--bins` 加 `tests/*.rs`。
  - 创建 `evidence/` 目录。
  - 因全量失败去改产品代码。失败名写入 Act Response 后停止。
- First test:
  - 无产品变更。第一批就是 `--lib --bins` 那条命令。
- GREEN condition:
  - 文件批加 `--lib --bins` 批全部跑完，每批 0 failed，覆盖核对三项为 0。
- Verification:
  - 见上面的命令和通过条件。结果写入 Act Response 的批次表。
- Stop when: 某一批复核后仍有 failed；或必须改产品代码、放宽分批规则才能得到 0 failed。

**Invariants**

- 本 change 的增量规格继续保留 IRBR 的旧标题。
- 不新增断电安全承诺，不改 WAL、checkpoint、提交的产品行为。
- 全量只跑一次，并且遵守分批串行。
- 不引入身份证明、判定层、新产品参数或配置。
- 不修改主规格库、Issue 台账和全局任务。

**Non-goals**

- 产品代码与测试代码。
- 登记或关闭 ISS32、ISS33、ISS34 以及其他 Issue。
- 把未登记的缺陷写进本 Iteration 的任务。
- 仓库级 `cargo fmt --check` 与 `cargo clippy --all-targets`。
- GIVEN 措辞、SHALL 条款、Scenario 数量的改写。

**Acceptance**

| # | 验收条件 | 需求 / 场景 | 设计 | 任务 | 代码位置 | 测试 |
|---|---|---|---|---|---|---|
| A45 | 交接文件有五对标题，新标题把「损坏的索引页」换成「未通过页级判据的索引树页」；增量标题未改；主规格未改 | ISS34 标题对齐 | D9 | T13 | `corpus-handoff.md` | 读五对标题和两份规格 |
| A46 | 两份 README 插入规定段落且表格数字未改；`wal-auto-checkpoint` 增量逐字保留原 Requirement 并只追加规定的边界 Scenario | I064 | D9 | T14 | 两份 README；change `specs/wal-auto-checkpoint/spec.md` | 读这三处 |
| A47 | 交接文件有 13 条 Purpose 替换，每条带占位句中的 change id；`wal-recovery-replay-integrity` 标明不替换；主规格未改 | I045 | D9 | T15 | `corpus-handoff.md` | 对照 14 份 Purpose |
| A48 | 全量按字典序等分，每批 8–12，批内 `--test-threads=1`，`--lib --bins` 单独成批，批间串行，无过滤全量和默认并行度都没有用；全部批次 0 failed，覆盖核对三项为 0 | D10 收尾验证 | D10 | T16 | 无产品改动 | 各批 `cargo test` 的退出码与 `test result:` |

**Verification**

- T13、T14、T15 是读文本。没有运行时行为，不设产品测试。
- T16 是本 change 唯一一次全量。规则：
  - 目标名按 `ls tests/*.rs | sort` 的字典序。
  - 每批 8–12 个目标。文件批用等分规则（前若干批 11 个，其余 12 个）。
  - 每一批的命令带 `--test-threads=1`。
  - 批间串行，同一时刻只有一批。
  - 不得一次无过滤全量。
  - 不得用默认并行度跑任何一批。
  - `--lib --bins` 单独一批，放在文件批之前。
- 通过条件：A45–A47 的文本与 Task Definition 一致；A48 的每一批 0 failed。

**Gate 2 Readiness**

- Step 5：无 TBD。D9 / D10 已裁定。13 份 Purpose 的句子由该域现有 Requirements 抽出，不新造行为。标题、README 段落和边界 Scenario 已给出原文。全量的切批、命令、通过条件和失败停点已写明。Act 只读本文件即可做 T13 的标题核对。无身份证明、无判定层。⇒ 全部 PASS。
- 用户批准：goal「继续把当前 ms 做完，记得遵守各种规范，尤其是多批次测试规范」构成 Gate 2 的批准。批准不降低判据。

**Cycle RTM**

| Requirement / Scenario | Task | First Test | Status |
|---|---|---|---|
| ISS34 五个标题的交接 | T13 | 读主规格五个旧标题 | Covered |
| I064 README 与提交 / checkpoint 边界 | T14 | 读性能对比段与「自动 checkpoint 失败传播」 | Covered |
| I045 的 14 份 Purpose | T15 | 读 14 份 Purpose | Covered |
| 收尾全量回归 | T16 | `cargo test --lib --bins -- --test-threads=1` 起的分批 | Covered |

**Persisted Evidence**

- Mode: none

批次表和交接核对写入 Act Response。命令可以重跑，但本 change 的全量预算只有这一次，不为留档再跑第二轮。不建 `evidence/` 目录。

**Risks and Notes**

- 执行时 `tests/*.rs` 的数量若不是 148，按公式重算。不要为了凑 148 去增删文件。
- Issue 正文还提到 GIVEN 措辞。本 Cycle 按 D9 和 T13 只交接标题。GIVEN 不改。
- `wal-recovery-frame-parsing` 与 `database-file-lock` 的占位句指向同一个 change id。按占位句原样引用，不要改成另一个 id。
- 跳过项：不跑第二次全量；不改产品代码；不改主规格库；不登记 Issue。

**后续任务边界**

- 授权边界：无。本 Iteration 是 Map 的最后一项。
- 能力边界：无。全量由 Act 在本机串行执行。
- 停止边界：Act 不创建下一 Iteration，不调用文档维护，不归档。
- 不在本 Cycle：主规格落位、台账关闭、仓库级 fmt / clippy。

## Act Response

**状态**: `reported`
**Cycle**: Iteration 009 / Cycle 000（`initial`）　**Task**: T13、T14、T15、T16
**Act 执行日期**: 2026-10-26

Plan Context 保持 `ready`。本文件没有 Plan Review。没有下一 Cycle，没有下一 Iteration。

### 接手时的实际状态

- Plan Context `ready`，Review Result 仍是 `pending`，没有后继 Cycle，文件里原先没有 Act Response。
- 从当前工作区实施，没有回退 Iteration 000–008。

### 文档核对

T13、T14、T15 没有产品 RED。改之前读过的文本与 Plan Context 一致：主规格和本变更增量里的五个旧标题都还在；两份 README 的性能对比段还没有三件承诺那一段；「自动 checkpoint 失败传播」还没有边界 Scenario；13 份 Purpose 仍是占位句，change id 与计划列表一致；`wal-recovery-replay-integrity` 已有正文，没有 TBD，也没有 TODO。

### 实际改动

| 位置 | 改动 |
|---|---|
| `corpus-handoff.md` | 新建。五对标题的旧串、新串和落位顺序；13 条 Purpose 替换，每条带占位句里的 change id；`wal-recovery-replay-integrity` 标明不替换 |
| `README.md` | 在「These are single-machine observations, not benchmark guarantees.」之后、表格之前插入规定的英文段。`sync_all` 与 `SIGKILL` 使用反引号。表格数字未改 |
| `README.zh-CN.md` | 在「以下为单机现场观察，不是基准保证。」之后、表格之前插入规定的中文段。反引号同样只加在 `sync_all` 与 `SIGKILL` 上。表格数字未改 |
| `specs/wal-auto-checkpoint/spec.md` | 新建。一个 `MODIFIED` Requirement「自动 checkpoint 失败传播」。原 Requirement 正文和 Scenario「checkpoint 失败诚实报告且数据不丢」与主规格逐字相同，只追加规定的边界 Scenario |

产品代码、测试代码、主规格库都没有改。`specs/index-read-bounded-read/spec.md` 的标题没有改。相对 HEAD 的 README diff 里还有 Iteration 008 留下的存量兼容半句；本 Cycle 没有动那一段。

### 与计划的偏差

无。

### 自检

Gate 3：T13 的五对新旧串与 Task Definition 一致，并写了「先套用增量、再替换合并后的主规格」。SHALL、GIVEN / WHEN / THEN、Scenario 数量、Requirement 数量，以及另外两个标题，都写明不动。T14 没有把 `sync_all` 或 SIGKILL 写成已验证断电安全。增量规格的原条款与主规格逐字相同。T15 的 13 条都是该域 Requirements 里已有的能力，没有新的行为承诺，没有 TBD / TODO。T16 没有为失败去改产品代码。没有身份证明或判定层。计划范围内没有未解决的 Critical 或 Important。

Gate 4：T13–T15 对照文本通过。T16 见下方批次表，每一批退出码 0，每一条汇总行的 failed 为 0。

Gate 5：未触发。

遗留 Minor：无。

### 验证

T13、T14、T15 只读文本，没有跑测试。

T16 先执行 `ls tests/*.rs | sort`。`N = 148`，`B = ceil(148 / 12) = 13`，前 `13 × 12 − 148 = 8` 批各 11 个目标，后 5 批各 12 个。核对 `11 × 8 + 12 × 5 = 148`。`--lib --bins` 单独成第 00 批，放在文件批之前。每一批都带 `--test-threads=1`。上一批的退出码已经记下，才启动下一批。没有无过滤全量，没有默认并行度，没有同时开两批，没有跑 `--doc`，没有建 `evidence/`。

规定的 `grep -E '^test result:'` 在第 00 批还打出两行单元测试名：`result::tests::value_to_json_preserves_conversion_semantics` 和 `result::tests::value_to_json_renders_datetime_family_as_da5_string`。这两行是 `test result::...`，不是汇总。下表只记 `test result: ok.` 汇总。数字顺序与该批目标顺序一致，写成 passed / failed / ignored。

| 批 | 退出码 | 目标 | 汇总（passed / failed / ignored） |
|---|---|---|---|
| 00 | 0 | `--lib`，然后 `--bins` | lib 421 / 0 / 0；bin 0 / 0 / 0 |
| 01 | 0 | `aggregate_extension_test`，`aggregate_output_shape_test`，`aggregate_test`，`alter_table_test`，`attach_session_test`，`btree_merge_test`，`btree_scale_test`，`btree_split_test`，`btree_test`，`busy_timeout_test`，`cache_perf_test` | 39 / 0 / 0，8 / 0 / 0，19 / 0 / 0，16 / 0 / 0，11 / 0 / 0，10 / 0 / 0，5 / 0 / 0，12 / 0 / 0，10 / 0 / 0，18 / 0 / 0，1 / 0 / 0 |
| 02 | 0 | `catalog_bounded_read_test`，`catalog_durability_test`，`chain_join_test`，`check_command_test`，`checkpoint_command_test`，`checkpoint_redo_reduction_test`，`checkpoint_skip_test`，`checkpoint_test`，`cli_test`，`concurrent_test`，`connection_limit_test` | 5 / 0 / 0，4 / 0 / 0，9 / 0 / 0，11 / 0 / 0，6 / 0 / 0，9 / 0 / 0，6 / 0 / 0，2 / 0 / 0，126 / 0 / 2，10 / 0 / 0，3 / 0 / 0 |
| 03 | 0 | `constraint_enforcement_test`，`cross_db_dml_test`，`cross_db_query_test`，`ctas_test`，`cte_test`，`data_page_bounded_read_test`，`database_file_lock_test`，`datetime_function_test`，`datetime_type_test`，`ddl_catalog_transactional_test`，`ddl_crash_recovery_test` | 26 / 0 / 0，17 / 0 / 0，21 / 0 / 0，17 / 0 / 0，14 / 0 / 0，8 / 0 / 0，8 / 0 / 0，19 / 0 / 0，16 / 0 / 0，3 / 0 / 0，6 / 0 / 0 |
| 04 | 0 | `ddl_layout_publication_test`，`ddl_rewrite_consistency_test`，`default_declaration_test`，`default_precheck_test`，`derived_scan_alias_test`，`dml_tombstone_abandon_window_test`，`dml_tx_id_test`，`drop_persistence_test`，`drop_table_free_test`，`e2e_test`，`encryption_test` | 9 / 0 / 0，9 / 0 / 0，14 / 0 / 0，5 / 0 / 0，26 / 0 / 0，3 / 0 / 0，6 / 0 / 0，11 / 0 / 0，6 / 0 / 0，27 / 0 / 0，7 / 0 / 0 |
| 05 | 0 | `executor_test`，`explain_test`，`explicit_tx_test`，`expression_e2e_test`，`file_header_test`，`file_storage_io_test`，`free_page_lifecycle_test`，`full_table_dml_test`，`gc_test`，`group_by_expr_test`，`import_input_surface_test` | 39 / 0 / 0，10 / 0 / 0，21 / 0 / 0，24 / 0 / 0，14 / 0 / 0，4 / 0 / 0，12 / 0 / 0，11 / 0 / 0，3 / 0 / 0，12 / 0 / 0，12 / 0 / 0 |
| 06 | 0 | `index_manager_test`，`index_page_ownership_stack_overflow_test`，`index_page_ownership_test`，`index_read_bounded_read_test`，`index_write_atomicity_test`，`insert_column_list_test`，`insert_select_test`，`isolation_level_test`，`join_output_resolution_test`，`join_test`，`key_type_conformance_test` | 3 / 0 / 0，1 / 0 / 0，5 / 0 / 0，7 / 0 / 0，5 / 0 / 0，7 / 0 / 0，10 / 0 / 0，9 / 0 / 0，19 / 0 / 0，9 / 0 / 0，8 / 0 / 0 |
| 07 | 0 | `keyless_eq_routing_test`，`keyless_row_test`，`legacy_compat_test`，`limit_test`，`memory_db_test`，`mvcc_abort_test`，`mvcc_commit_test`，`mvcc_record_test`，`mvcc_tombstone_visibility_test`，`nested_loop_join_test`，`network_protocol_test` | 14 / 0 / 0，4 / 0 / 0，16 / 0 / 0，5 / 0 / 0，15 / 0 / 0，3 / 0 / 0，4 / 0 / 0，5 / 0 / 0，11 / 0 / 0，9 / 0 / 0，5 / 0 / 0 |
| 08 | 0 | `network_server_test`，`no_from_select_test`，`order_by_alias_sort_test`，`outer_join_test`，`overflow_page_test`，`param_binding_test`，`parser_test`，`pg_integration_test`，`pg_messages_test`，`pg_protocol_test`，`pipeline_test` | 0 / 0 / 0，9 / 0 / 0，7 / 0 / 0，16 / 0 / 0，27 / 0 / 0，28 / 0 / 0，6 / 0 / 0，6 / 0 / 0，9 / 0 / 0，11 / 0 / 0，17 / 0 / 0 |
| 09 | 0 | `plan_cache_test`，`plan_exec_test`，`planner_test`，`predicate_test`，`prefetch_test`，`prepared_statement_test`，`projection_expression_test`，`projection_test`，`pushdown_test`，`query_memory_test`，`readonly_session_test`，`recovery_bounded_read_test` | 7 / 0 / 0，4 / 0 / 0，107 / 0 / 0，23 / 0 / 0，3 / 0 / 0，14 / 0 / 0，16 / 0 / 0，6 / 0 / 0，16 / 0 / 0，12 / 0 / 0，15 / 0 / 0，6 / 0 / 0 |
| 10 | 0 | `recovery_e2e_test`，`recovery_error_surface_test`，`recovery_replay_convergence_test`，`recovery_tail_persistence_test`，`recovery_test`，`recovery_wal_convergence_test`，`release_durability_precondition_test`，`release_path_bounded_read_test`，`runtime_test`，`scalar_function_test`，`scalar_subquery_comparison_test`，`schema_persistence_test` | 6 / 0 / 0，3 / 0 / 0，3 / 0 / 0，4 / 0 / 0，3 / 0 / 0，5 / 0 / 0，2 / 0 / 0，9 / 0 / 0，3 / 0 / 0，32 / 0 / 0，16 / 0 / 0，8 / 0 / 0 |
| 11 | 0 | `secondary_index_equality_test`，`secondary_index_test`，`select_distinct_test`，`self_join_test`，`set_operations_test`，`sort_test`，`sql_differential_test`，`statement_cancellation_test`，`storage_test`，`structural_property_test`，`subquery_alias_binding_test`，`subquery_test` | 3 / 0 / 0，27 / 0 / 0，12 / 0 / 0，6 / 0 / 0，21 / 0 / 0，8 / 0 / 0，10 / 0 / 0，8 / 0 / 0，21 / 0 / 0，5 / 0 / 0，25 / 0 / 0，46 / 0 / 0 |
| 12 | 0 | `subset_insert_test`，`sync_loader_test`，`table_alias_resolution_test`，`table_manager_test`，`truncate_test`，`tx_statement_test`，`typed_api_test`，`update_expression_test`，`update_index_maintenance_test`，`upsert_test`，`value_test`，`version_chain_test` | 6 / 0 / 0，2 / 0 / 0，19 / 0 / 0，6 / 0 / 0，7 / 0 / 0，21 / 0 / 0，17 / 0 / 0，14 / 0 / 0，11 / 0 / 0，32 / 0 / 0，19 / 0 / 0，3 / 0 / 0 |
| 13 | 0 | `view_test`，`visibility_test`，`wal_auto_checkpoint_test`，`wal_buffer_test`，`wal_handle_test`，`wal_integration_test`，`wal_record_test`，`wal_recovery_large_test`，`wal_writer_test`，`write_failure_state_test`，`write_path_slot_bounds_test`，`write_type_conformance_test` | 19 / 0 / 0，5 / 0 / 0，11 / 0 / 0，4 / 0 / 0，4 / 0 / 0，2 / 0 / 0，14 / 0 / 0，7 / 0 / 0，4 / 0 / 0，3 / 0 / 0，11 / 0 / 0，13 / 0 / 0 |

覆盖核对：文件批目标 148，清单 148。缺 0，多 0，重复 0。`network_server_test` 和 bin 的汇总是 0 passed / 0 failed，目标都跑过，failed 仍是 0。没有失败批，所以没有复核重跑。前次 2231 / 0 / 2 没有当作通过条件。

### Persisted Evidence

None required。

### Experience Candidates

None。

### Issue 候选

None。

### 未解决

无阻塞项。Iteration 009 未标 accepted，等待 Plan Review。

## Plan Review

- Review Result: accepted

**Findings**

对照交接文件、两份 README、`specs/wal-auto-checkpoint/spec.md`、主规格和当前 `tests/*.rs` 清单。A45–A48 均满足。没有阻塞 Acceptance 的偏差，也没有 Minor finding。

- A45：`corpus-handoff.md` 的五对标题与 Task Definition 逐字相同，并写了先套用增量、再替换合并后主规格。另外两个标题、SHALL、GIVEN / WHEN / THEN、Scenario 数量和 Requirement 数量都写明不动。本变更 `specs/index-read-bounded-read/spec.md` 的五个标题仍是旧串。`openspec/specs/` 相对 HEAD 无 diff。
- A46：两份 README 在规定句子之后、表格之前插入了规定段落，`sync_all` 与 `SIGKILL` 使用反引号。表格数字不在本次 diff 里。相对 HEAD 的 README diff 另有 Iteration 008 的存量兼容半句；性能对比段以外的那一处不是本 Cycle 的改动。增量规格只含一个 `MODIFIED` Requirement「自动 checkpoint 失败传播」。原 Requirement 正文和 Scenario「checkpoint 失败诚实报告且数据不丢」与主规格 `openspec/specs/wal-auto-checkpoint/spec.md` 第 58、62–64 行逐字相同，只追加了规定的边界 Scenario。没有写成已验证断电安全。
- A47：交接文件有 13 条替换。占位全文和 change id 与 13 份主规格的当前 `## Purpose` 一致。新 Purpose 复述各域 Requirements 里已有的能力，没有新的行为承诺，没有 TBD，也没有 TODO。`wal-recovery-replay-integrity` 标明不替换；其主规格 Purpose 已有正文，文中没有 TBD，也没有 TODO。
- A48：见 Evidence。分批、串行、`--test-threads=1`、`--lib --bins` 单独成批、覆盖核对和 failed 均为 0，与本 Cycle 已写的规则一致。没有因此放宽通过条件。

**Evidence**

来源是本 Cycle 的 Act Response 批次表，以及同一次运行留在终端里的 cargo 输出（2026-10-26 16:04:32 启动，16:16:28 结束，退出码 0）。本 Review 没有重跑全量。`src/` 与 `tests/` 在该次结束之后没有更新的文件，覆盖范围未失效。

- `N = 148`，`B = 13`，前 8 批各 11 个目标，后 5 批各 12 个。`11 × 8 + 12 × 5 = 148`。
- 第 00 批是 `cargo test --lib --bins -- --test-threads=1`，退出码 0。汇总两条：lib `421 passed; 0 failed; 0 ignored`，bin `0 passed; 0 failed; 0 ignored`。`grep` 另打出两行 `test result::tests::...`，那是测试名，不是汇总；Act 表未把它们计入汇总，与输出一致。
- 文件批 01–13 的目标名与当前 `ls tests/*.rs | sort` 去掉目录和 `.rs` 后的 148 个名字逐名相同，顺序是字典序。缺 0，多 0，重复 0。每一批都经 `cargo test --test <目标> … -- --test-threads=1`，上一批记下退出码后才启动下一批。14 个退出码都是 0。汇总行共 150 条，failed 都是 0。`cli_test` 为 `126 passed; 0 failed; 2 ignored`，与表中 `126 / 0 / 2` 一致。
- `network_server_test` 的汇总是 `0 passed; 0 failed; 0 ignored; 0 filtered out`。该文件正文整段被注释，目标跑过，failed 仍是 0。
- 没有无目标过滤的 `cargo test`，没有去掉 `--test-threads=1` 的批次，没有同时开两批，没有 `--doc`，没有 `evidence/` 目录。没有失败批，所以没有复核重跑。前次 2231 / 0 / 2 没有当作通过条件。

**Follow-up Decision**

A45–A48 已由当前文本和这一次全量满足。没有需要当前 Cycle 修复的缺口，也不需要新的执行依据。009 是 Map 的最后一项，不创建后继 Iteration。Issue 候选：无。

**Next Cycle**

None

**Next Iteration**

None
