# Iteration 001 / Cycle 000: 一次分批全量

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `proposal.md` 的维持合同，`design.md` 的 D5，`tasks.md` 里 Iteration 001 的验证边界。上一轮 `iterations/000-record-and-keep/000-initial.md` 的 Review Result 为 `accepted` 后，本轮才展开。M19 的分批和 `--test-threads=1` 继续有效。
- Excluded scope: 不改刷盘和驱逐，也不改恢复或文件锁。不重跑计时。不加 `--ignored`。不写新测试或判定脚本，也不做第二遍全量。不预建修复轮次。不建第二个变更。

**Objective**

在没有产品改动的树上，按下面的切分把现有测试跑完一次。每一批 cargo 退出码为 0，且该批每一行 `test result:` 的 failed 为 0。这次全量是整个目标里唯一的一次。

**Investigation Facts**

- Starting point: 上一轮 Act Response 没有修改产品符号。该轮验证时 HEAD 是 `3f95a874fd45ef55bf844a80ceb7e9aeac1357fc`，`git diff 0019bf127df5304f26e89927e336150fc6472b11 -- src tests Cargo.toml` 无输出。Review 又在 `c9ff98e0a24dab4b18abc1f4f88b3573f57d15bb` 上做了只读核对，产品路径仍无差异。计时和锁等待沿用上一轮 Act Response，不重跑。本 Cycle 的计划提交落在该 HEAD 之后，仍然不改 `src/`、`tests/` 和 `Cargo.toml`。Act 从包含本文件的提交所在工作区开始。
- Current-State Evidence:
  - 全量还没有在本变更里跑过。通过条件不是 runbook 里记载的历史 passed 总数。那些数字属于别的工作区。
  - 计划写下时，在本工作区执行 `ls tests/*.rs | sort | wc -l` 得到 169。按等分规则这是 15 个文件批（11 批各 11 个文件，4 批各 12 个文件）再加单独的 `--lib --bins`，共 16 批。169 只说明写下时公式适用。执行时重新数，不沿用 169。
  - 集成测试目标名是文件名去掉目录和 `.rs`。`tests/cli_test.rs` 对应 `--test cli_test`。
  - `--lib --bins` 一批会打出两行 `test result:`，一行属于库，一行属于二进制。文件批通常一行。通过条件看每一行的 failed，不要求行数等于 1。
  - `cli_test.rs` 里长期 `#[ignore]` 的校准用例不在普通 `cargo test` 里运行。本轮命令不带 `--ignored`，避免把上一轮已经沿用的计时再跑一遍。
- Code and Critical Path:
  - 本轮没有产品符号要改。入口是本工作区根目录的 `cargo test`。工作区是 `/home/daivy/projects/RTsql-changes/2026-11-17-ms52-torn-tree-and-cross-process`。不要在主仓库里跑。
  - 调用链停在测试进程。失败时读取失败目标自己的断言。缓冲池、恢复和文件锁没有本变更的改动，不在本轮里修。

**Implementation Guidance**

先取得当次文件清单并切批，对账通过后再启动 cargo。每一批等上一批进程退出后再启动。全量中途不要改文件清单的切法。

某一批第一次失败时，cargo 不会再跑该批里排在失败目标后面的文件。允许把这一整批再跑一次，用来区分偶发失败。复核通过则记下首跑失败，并继续下一批。复核仍失败则停止。不要只重跑失败的那一个目标，也不要把它算成另一次全量。

**Task Definitions**

### T3: 跑一次分批全量

- Requirement/Scenario: `proposal.md` 的维持合同，加上 `tasks.md` Iteration 001 的验证边界。全量不产生新的产品行为。
- Depends on: 上一轮 Review Result 为 `accepted`。T1 和 T2 已记入该轮 Act Response。
- Targets: 不修改源码。命令在上述工作区根目录执行。对象是当次 `ls tests/*.rs | sort` 的文件，以及单独的 `--lib` 和 `--bins`。
- Current behavior: 本变更还没有跑过这套分批全量。`src/`、`tests/` 和 `Cargo.toml` 相对 `0019bf127df5304f26e89927e336150fc6472b11` 没有差异。
- Required behavior: 开跑前 missing、extra 和 duplicate 都是 0。正常路径上每一批启动一次 cargo。某一批首跑失败时，只允许把该整批再启动一次；复核通过后，该批按通过计，并继续后面的批次。作为通过依据的那一次，cargo 退出码为 0，且该次输出里每一行 `test result:` 的 failed 为 0。文件批覆盖当次清单里的每个文件恰好一次。全部批次跑完后，再次 `ls tests/*.rs | sort` 与开跑前的清单一致。缺一批、缺少 `test result:` 行，或清单在中途变化，都不能声明全量通过。
- Preserve: `src/`、`tests/` 和 `Cargo.toml` 相对上一轮结束时的内容不变。每条 `cargo test` 带 `--test-threads=1`。批与批串行，同一时刻只有一批。普通套件不包含 `--ignored`。
- Forbidden: 无过滤的 `cargo test`。默认并行度。第二遍全量，含以后的返工再预算一次分批全量。为了让某批通过去改产品代码。把判定写成单独的脚本。`--ignored`。重跑上一轮的计时。在主仓库执行。
- First test: 不新写测试。没有产品代码变更，所以没有另一次 RED。第一批就是：

```text
cargo test --lib --bins -- --test-threads=1
```

- GREEN condition: 全部批次都满足 Required behavior，并且下面的产品 diff 仍无输出。
- Verification: 在工作区根目录按顺序做完这些事。判定只看 cargo 的退出码、`test result:` 行和产品 diff。
  1. `ls tests/*.rs | sort` 得到有序列表。`N` 是行数。`B = ceil(N / 12)`，整数运算写成 `(N + 11) / 12`。11 个文件的批数是 `B × 12 - N`。12 个文件的批数是 `N - 11 × B`。按字典序，前面的文件批各取 11 个文件，后面的各取 12 个。`--lib --bins` 不进入这 `B` 批，单独作为第一批。
  2. 开跑前核对：批次表里的文件与这次 `ls` 的结果一致，每个文件恰好一次。missing、extra 和 duplicate 都是 0。对账失败就停止，不要启动 cargo，这样唯一的一次全量不会花在漏目标的批次表上。不要另写对账脚本。
  3. 第一批运行上面的 `--lib --bins` 命令。随后按切好的顺序运行文件批。文件批的形式是 `cargo test --test <目标名> --test <目标名> ... -- --test-threads=1`。目标名是去掉目录和 `.rs` 后的名字。正常路径上每批启动一次。退出码是该 cargo 进程的退出码。`test result:` 行从同一次输出里抄出。输出很长时只保留这些行。可以用一步 `grep -E "^test result:"` 阅读同一次输出。管道的退出码不能代替 cargo 的退出码，也不要为了计数再启动一次 cargo。首跑失败时的那一次整批复核是唯一允许的额外启动。
  4. 全部批次结束后再执行一次 `ls tests/*.rs | sort`，并执行：

```text
git diff 0019bf127df5304f26e89927e336150fc6472b11 -- src tests Cargo.toml
```

- Stop when: 12 个文件的批数小于 0。此时停止并回到计划，不要改用别的切法。同一批第二次仍失败。失败要求修改缓冲池、恢复、文件锁或其他产品代码。清单在开跑后发生变化。以上任一情况都停止并报告已经跑到的批次和失败目标。不在本任务里开修复轮次，也不把剩余批次或整次分批再跑一遍。

**Invariants**

- 整个目标只有这一次分批全量。本 Cycle 用掉它。失败批次的一次整批复核属于这一次内部的偶发判别，不是第二次全量。以后的返工不得再排一次分批全量。
- 每条 `cargo test` 带 `--test-threads=1`。同一时刻只有一批。
- 不修改 `src/`、`tests/` 和 `Cargo.toml`。不新增行为规格，不改 `openspec/specs/`。

**Non-goals**

不因为全量失败去改刷盘和驱逐，也不改恢复或文件锁。不预建修复轮次。不重跑计时。不把历史 passed 总数对齐成通过条件。

**Acceptance**

- 当次文件清单按上面的等分规则切完，开跑前 missing、extra 和 duplicate 都是 0。
- 第一批是 `--lib --bins`，其后是字典序文件批。作为通过依据的那一次，退出码为 0，且每一行 `test result:` 的 failed 为 0。
- 若有一批首跑失败且整批复核通过，Act Response 同时记下首跑失败和复核通过。除此之外没有第二遍。
- 跑完后的 `ls tests/*.rs | sort` 与开跑前一致。产品 diff 无输出。
- Act Response 留下每一批的目标列表、cargo 退出码和 `test result:` 行。缺一批就不能声明通过。

**Verification**

- 维持合同的通过依据是产品 diff 无输出，以及本轮没有修改产品代码。
- 全量的通过依据是每一批 cargo 进程退出码 0，和同一次输出里每一行 `test result:` 的 failed 为 0。
- 这些命令就是真实的 `cargo test`。不另做判定脚本。

**Gate 2 Readiness**

PASS。无 BLOCKED。无 SKIPPED。计划由本次目标的批准离开 draft。

**Persisted Evidence**

- Mode: none

各批的目标列表、退出码和 `test result:` 行写入 Act Response。这些行决定全量是否通过。不另存日志目录。全量不能靠重跑来补记录，所以响应里必须留下当次那几行。

**Risks and Notes**

- 执行时文件数若不是 169，按当次 `ls` 重算。12 个文件的批数若小于 0，属于 Stop when，不要在本轮另选切法。
- 某一批第一次失败后，该批后面的文件还没跑。复核必须重跑整批。
- 历史 passed 总数不是本轮阈值。ignored 可以不是 0。通过只看 failed 为 0 和退出码 0。
- 全量失败时不改产品代码。空闲页重建放弃仍不登记，也不为本轮预建修复。

## Act Response

Act 执行后覆盖这一节。

## Plan Review

- Review Result: pending
