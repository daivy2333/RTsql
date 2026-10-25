# Tasks：ISS20 索引写侧原子性 + ISS21 DDL 目录事务化

状态：`pending` / `in_progress` / `done` / `blocked`。完整状态以 change 产物（Cycle 的 `Act Response`）为准。

## 任务列表

| ID | 任务 | Iteration | 状态 |
|---|---|---|---|
| T1 | 建立确定性触发 ISS20 窗口的能力（写侧暂停点夹具）+ 先行 RED 用例 | 000 | done |
| T2 | 索引写单元原子化与写侧串行化（使 T1 用例 GREEN） | 000 | done |
| T3 | 取消/回滚后的键全释放与索引一致性端到端回归 | 000 | done |
| T4 | ISS21 先行 RED 用例（放弃 DDL → 回收 → 重开 → 目录列数与行宽一致） | 001 | done |
| T5 | DDL 目录窄化延后到提交点之后（使 T4 用例 GREEN） | 001 | done |
| T6 | DDL 目录面回归对齐（正常完成路径 + 既有 MS34 回收用例断言补齐）+ 本 change 收尾全量回归（M19 分批串行） | 001 | done |

## Requirements Traceability Matrix

`index-write-side-atomicity`（IWS）与 `ddl-catalog-transactional`（DCT）两个规格域的 requirement 编号。

| Requirement | Task | First Test | Status |
|---|---|---|---|
| IWS R1 索引写单元不可被取消分割 | T1, T2 | `tests/index_write_atomicity_test.rs::cancelled_write_keeps_index_self_consistent` | Covered |
| IWS R2 同一索引的结构写侧串行化 | T2, T3 | `tests/index_write_atomicity_test.rs::cancelled_write_keeps_index_self_consistent`（写单元原子性）+ `::concurrent_writers_do_not_fork_the_tree` | Covered |
| IWS R3 取消或回滚释放被取消语句占用的全部键 | T1, T2, T3 | `tests/index_write_atomicity_test.rs::cancelled_write_keeps_index_self_consistent`（键复用断言）+ `tests/statement_cancellation_test.rs`（显式/隐式路径、RR/RC 扩展断言） | Covered |
| IWS R4 取消或回滚之后索引结构保持一致 | T2, T3 | `tests/index_write_atomicity_test.rs::cancelled_write_keeps_index_self_consistent`（完整性断言）+ `::concurrent_writers_do_not_fork_the_tree` | Covered |
| DCT R1 被放弃或失败的 DDL 语句不留下目录面变更 | T4, T5 | `tests/ddl_catalog_transactional_test.rs::abandoned_drop_column_leaves_catalog_unchanged` | Covered |
| DCT R2 目录变更与数据面改写同生共死 | T5, T6 | `tests/ddl_catalog_transactional_test.rs::completed_drop_column_aligns_catalog_and_rows` | Covered |
| DCT R3 放弃 DDL 之后重开库目录与数据面一致 | T4, T5, T6 | `tests/ddl_catalog_transactional_test.rs::abandoned_drop_column_leaves_catalog_unchanged`（含 close→reopen 读回） | Covered |

命名说明：矩阵中的测试名是**计划中的用例名**；Act 可在不改变断言语义的前提下调整命名，并在 Act Response 记录实际名称。

## Iteration Plan

### Iteration 000: 索引写侧原子化，取消/回滚后键全释放且索引结构自洽

- Tasks: T1, T2, T3
- Depends on: None
- Stable result: 主键/唯一索引的结构变更、根页 id 发布与反向映射更新构成不可分割且串行的写单元；被取消语句写入的全部键在回滚后释放；取消/回滚之后索引结构自洽（不 abort、完整性无 Error）。该结果是 `statement-cancellation-outcome` 规格 R2「已知边界」所指向的存储域契约的已验收行为。
- Verification boundary: 本 Iteration 的独立完成判据为 T1/T2/T3 的定向用例（新增回归文件 + 扩展 `tests/statement_cancellation_test.rs`）全部通过，命令形如 `cargo test --test index_write_atomicity_test -- --test-threads=1` 与 `cargo test --test statement_cancellation_test -- --test-threads=1`。**本 Iteration 不跑全量回归**：全量是本 change 的收尾验证，预算记在 Iteration 001（一次）。
- Diagnostic boundary: 失败时按「写单元边界 → 暂停点位置 → 回滚分桶 → 完整性检查报告」四段定位；结构损坏（栈溢出 / SIGABRT）由 gdb 栈与 `child_page_id` 帧判读（ISS20 现场已有同型样例）。
- Non-goals: DDL 目录面（Iteration 001）；空闲页跨重启复用（MS35）；历史库与 dump 兼容（MS36）；分级诊断系统（I096）。

### Iteration 001: DDL 目录变更的事务化——被放弃/失败的 DDL 不留下目录面变更

- Tasks: T4, T5, T6
- Depends on: Iteration 000
- Stable result: `ALTER TABLE DROP COLUMN` 的目录窄化只在语句确定完成（提交点）之后生效；被放弃或失败的 DDL 不改变目录的列定义与列数；放弃后回收并重开库，目录列数与数据面行宽一致。
- Verification boundary: 独立完成判据为 T4/T5/T6 的定向用例通过（`cargo test --test ddl_catalog_transactional_test -- --test-threads=1` 与 `cargo test --test statement_cancellation_test -- --test-threads=1`）。
  **本 Iteration 承担本 change 的收尾验证：M19 分批全量回归（见下）**——按 M19 与 `.agents/runbooks/m19-batched-full-regression.md`（R48）执行一次，覆盖目标清单 = 变更前 `tests/` 全部测试文件 + `--lib --bins`。该收尾全量的执行依据是 **T6**（纪律逐条写死在 `iterations/001-ddl-catalog-transactional/000-initial.md` 的 T6「M19 分批串行全量回归纪律」第 1–10 条）。
- Diagnostic boundary: 失败时按「目录改写时点 → 提交点 → 内存布局切换 → 重开库读回」四段定位；目录与数据面的宽度差异用重开库后的目录列数与行宽直接比对。
- Non-goals: MS21 D10 的既有崩溃边界（提交与目录应用之间的窗口）；ADD COLUMN 路径的既有提交点写法（`update_table_column_count_ddl`）不改语义；并发读窗口的性质不变。

## 收尾验证：M19 分批全量回归（记在最后一个 Iteration）

**时机**：本 change 的最后一个 Iteration（001）收尾时执行一次（全 change 一次预算）。中途如需追加全量，须在本 Iteration Plan 记录理由。

**执行纪律（SHALL）**：

1. 测试目标按**确定性顺序**分组（`--lib --bins` 一批；`tests/*.rs` 按文件名字典序），每组一次独立的 `cargo test --test <f1> ... --test <fN> -- --test-threads=1` 调用。
2. 批间**顺序执行**，同一时刻只有一批在跑。
3. **SHALL NOT** 用一次无目标过滤的全量调用跑完（无论串行还是默认并行度）。
4. **SHALL NOT** 用默认并行度跑任何全量批次（每批必须带 `--test-threads=1`）。
5. 批大小 **8–12 个目标**（按机器能力调整）。
6. 全量 Gate 等价于「**全部批次各自 0 failed**」；缺一批不得声明全量通过。
7. 结论必须记录**每批的 passed/failed 计数**，并做覆盖对账：目标清单 = 变更前 `tests/` 全部测试文件 + `--lib --bins`（分批不重不漏）。
8. 判定只看 cargo 自己的退出码与原生输出；计数可用一步通用管道读出（`… 2>&1 | grep -E "^test result:"`），管道只做计数、不做判定；**不得**新增判定脚本、封装命令或判定层。
9. 单次目标验证（`cargo test --test <name>`）不受本纪律约束，可用默认并行度。

**覆盖对账基准**：变更前的 `tests/` 文件清单 + `--lib --bins`（收尾时按实际文件列表重算，并与本 change 新增的测试文件一并纳入）。

## 平衡审计

| Iteration | 聚合/拆分判定 | 结论 |
|---|---|---|
| 000 | 内聚结果 = 「索引写侧写单元的原子性与串行化 + 取消后的键释放与结构自洽」；同一存储/索引层、同一定位边界（索引写路径与回滚分桶）、同一验证入口（新增回归文件 + 既有取消用例）。T1/T2/T3 分属「建立触发能力 → 实施修复 → 端到端回归」，三者单独不构成稳定结果，合并为一个 Iteration 成立。 | 不拆分 |
| 001 | 内聚结果 = 「DDL 目录变更的事务化」；落在 DDL/目录层，与 Iteration 000 的存储/索引层是不同故障域、不同定位边界（目录改写时点 vs 索引写单元）、不同验证入口。单独成立且可独立验收。 | 不拆分 |
| change 整体 | ISS20 与 ISS21 根因不同层、验证方式不同、故障域不同；合并成一个 change 是因为两者同源于 M20 的同一批 Issue 候选、同属一次缺陷闭环交付，且共用一次收尾全量回归预算。 | 一个 change、两个 Iteration |

**首个 Iteration 承载范围检查**：Iteration 000 只承载 ISS20 的存储/索引面成果，不因编号为 `000` 而承载整个 change（ISS21 在 Iteration 001）✓。

## 最终状态与 Iteration Map 终态（Plan Review 收尾，2026-10-24）

- **两个 Iteration 全部 `accepted`**：
  - Iteration 000（索引写侧原子性，承载 ISS20）—— `iterations/000-index-write-side-atomicity/000-initial.md`，Review Result `accepted`。
  - Iteration 001（DDL 目录事务化，承载 ISS21）—— `iterations/001-ddl-catalog-transactional/000-initial.md`，Review Result `accepted`（本次 Review 填写）。
- **Iteration Map 终态**：`000 accepted → 001 accepted → 无后续 Iteration`。Map 中的全部 Iteration 均已 accepted，**无 rework / replan Cycle**，change 的实现工作全部完成（无剩余 Iteration）。
- 任务 T1–T6 全部 `done`；RTM 全部 `Covered`（无 `Missing`、无 `Simplified`）。
- 收尾全量终态（M19 分批串行，13 批，全部 exit=0）：**2171 passed / 0 failed / 2 ignored**；覆盖目标 **134**（`--lib --bins` + `tests/*.rs` 133 个文件）、`test result:` 行 **135**；覆盖对账 missing 0 / extra 0 / duplicate 0；与基线 2163/0/2 的差额 **+8** = `index_write_atomicity_test.rs` 5 个用例 + `ddl_catalog_transactional_test.rs` 3 个用例（`statement_cancellation_test.rs` 只增断言、用例数 8 → 8）。
- 待合并的规格增量（合并与全局状态同步、Issue 关闭、change 归档属 `openspec-docs-maintainer` / `openspec-experience-recorder`，不在本 change 的 Plan 职责内）：`specs/index-write-side-atomicity/spec.md`、`specs/ddl-catalog-transactional/spec.md`。交接清单见 `iterations/001-ddl-catalog-transactional/000-initial.md` 的 `Plan Review` → 收尾交接说明。
- 设计记录的长期修订（本次 Review 就地完成）：`design.md` D5 的「崩溃窗口」段表述更正（原文「不会静默错读」与读码事实矛盾；窗口本身仍是本 change 的 Non-goal，作为 Issue 候选 D 交由后续承载）。
