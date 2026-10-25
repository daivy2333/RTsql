# Tasks：MS35 空闲页跨重启复用

状态：`pending` / `in_progress` / `done` / `blocked`。完整状态以 change 产物（Cycle 的 `Act Response`）为准。

## 任务列表

| ID | 任务 | Iteration | 状态 |
|---|---|---|---|
| T1 | 先行 RED 用例：跨重启复用 + 重建不把活页判成空闲页（新增 `tests/free_page_lifecycle_test.rs`） | 000 | done |
| T2 | 存储层接缝与打开钩子：空闲集合注入接口（trait 默认实现）+ 读写打开路径在恢复之后的重建调用点 + 失败安全的空集合语义 | 000 | done |
| T3 | 活页集合重建实现：目录链（`__tables` / `__columns`）+ 表数据链 + 溢出链 + PK / 唯一 / 二级索引树 + 内存索引树并集 + 页数上界（使 T1 用例 GREEN） | 000 | done |
| T4 | 先行 RED 用例：释放未持久化前不可分配 + 释放持久化后可分配 + 崩溃后旧表内容完整 + 刷盘失败不提升 | 001 | done |
| T5 | 释放的持久化判定实现：待释放集合 + 释放完成编号 + checkpoint 第 2 步钩子（成功提升 / 失败丢弃）+ 六个释放点的完成标记（使 T4 用例 GREEN） | 001 | done |
| T6 | 既有同会话复用断言的分界更新（`tests/drop_table_free_test.rs` 5 例、`tests/overflow_page_test.rs` 1 例、lib 单测 `free_overflow_chain_returns_pages_for_reuse`）+ 端到端回归汇总（新回归文件补齐 `check_integrity` 与重开可读性断言） | 001 | done |
| T7 | 本 change 收尾全量回归（M19 分批串行） | 001 | done |
| T8 | 规格增量场景与验收边界对齐（replan：`free-page-lifecycle` R4 场景与 `drop-persistence-boundary` R1（MODIFIED）的崩溃点改到「释放之后、任何后续 DDL 之前」，DPB R1 前置条件补「此后没有其它 DDL 语句成功返回」；崩溃用例补齐 `other` 表覆盖并与修正后的场景对齐） | 001 | done |

### 任务执行依据要点

- **T1**：用例必须走真实入口（真实文件库 + 真实 `Database::open` / `close`），不得在框架外另建判定层；库文件体积用 `storage.page_count()` 之类的既有可观察量断言，不引入新指标。
- **T2**：接缝必须是 `AsyncStorage` 的**带默认实现的**方法，使 `MemoryStorage` 与测试替身不需要改动；只读打开路径不做重建（只读会话不分配页）。
- **T3**：遍历原语复用既有实现（`collect_data_pages` / `collect_overflow_chain_pages` / `IndexManager::collect_all_pages`）；遍历顺序与页数上界必须在遍历**之前**取快照；任一遍历失败即整次放弃（不注入任何页）。
- **T4**：崩溃用例按 `.agents/runbooks/in-process-crash-simulation.md`（R49）执行「丢弃句柄 + 不调用 close」；不得用真实断电或外部进程 kill。
- **T5**：checkpoint 钩子必须取「刷盘开始前的释放完成编号」，刷盘成功才提升、刷盘失败必须丢弃；**六个释放点**都要调用释放完成标记：`DROP TABLE`（`src/storage/data/table_manager.rs:1389+`）、`TRUNCATE`（`:1590+`）、`DROP INDEX`（`src/executor/drop_index.rs:62-63`）、GC 清理旧版本（`src/storage/data/table_manager.rs:170` → `src/storage/data_page.rs:249`）、`CREATE INDEX` 回填失败清理（`src/executor/create_index.rs:83`）、溢出链写入失败清理（`src/storage/page_format/overflow.rs:140`、`src/storage/data_page.rs:81`）。
- **T6**：既有断言的断言语义保留，只把「同会话立即复用」改成「释放持久化之后复用」（插入一次 `db.checkpoint().await` 作为分界）；不得弱化断言强度；lib 单测 `free_overflow_chain_returns_pages_for_reuse` 的断言同步改为「持久化后才可复用」。
- **T7**：执行依据见下方「收尾验证」，纪律逐条写死在 `iterations/001-durable-release-gate/000-initial.md` 的 T7 段。
- **T8**：只改规格增量文本与用例覆盖，**不改产品代码**；`free-page-lifecycle` R4 场景与 `drop-persistence-boundary` R1（MODIFIED）场景的崩溃点改到「`DROP TABLE` 成功返回之后、任何后续 DDL 语句成功返回之前」（DDL 的语句边界有序落盘会把后续 DDL 的提交点与其它脏页一并落盘，先前的目录删除因而随之落盘）；DPB R1（MODIFIED）的前置条件补「也没有其它 DDL 语句成功返回」；崩溃用例补 `other` 表覆盖（`other` 的行在重开后可查询），断言只增不减。

### 已知外部约束（不在本 change 内修复）

| 约束 | 影响 | 处理 |
|---|---|---|
| ISS23（BTree 插入无限递归 → 栈溢出，`src/storage/btree/btree.rs:319-327`，open） | 本 change 的验证场景若使用大表规模会撞上 | 验证场景只用**小表**（行数 ≪ 300，不触发多行索引插入路径）；需要多页时用行宽/页容量控制而非行数 |
| ISS22 / ISS24 / ISS25（open） | 与本 change 的存储层相邻 | 只作约束记录，不在本 change 内修改，也不作为本 change 的 Acceptance |
| MS36（历史库与 dump 兼容） | 本 change 不改格式、不加伴生文件 | 无格式面影响；若后续引入持久空闲页结构，须由 MS36 承担兼容面 |

## Requirements Traceability Matrix

`free-page-lifecycle`（FPL）、`drop-table-physical-free`（DTPF）、`drop-persistence-boundary`（DPB）三个规格域的 requirement 编号。

| Requirement | Task | First Test | Status |
|---|---|---|---|
| FPL R1 释放的持久化判定 | T4, T5 | `tests/free_page_lifecycle_test.rs::released_page_not_allocatable_before_release_durable` | Done |
| FPL R2 重新打开时按持久目录重建空闲页集合 | T1, T2, T3 | `tests/free_page_lifecycle_test.rs::reopened_database_reuses_released_pages` | Done |
| FPL R3 重建的失败安全 | T1, T3 | `tests/free_page_lifecycle_test.rs::rebuild_failure_keeps_live_pages_intact` | Done |
| FPL R4 崩溃后未完成的释放不是可分配页 | T4, T5, T6 | `tests/free_page_lifecycle_test.rs::crash_with_pending_release_keeps_old_table_rows` | Done |
| DTPF R 同进程 free-list 复用（MODIFIED：复用时机受持久化判定约束） | T4, T5, T6 | `tests/free_page_lifecycle_test.rs::released_page_not_allocatable_before_release_durable` + `tests/drop_table_free_test.rs`（更新后的分界断言） | Done |
| DTPF R 跨重启 drop 安全（MODIFIED：重启时按持久目录重建空闲页集合） | T1, T2, T3, T6 | `tests/free_page_lifecycle_test.rs::reopened_database_reuses_released_pages` + `tests/drop_table_free_test.rs::test_cross_restart_after_drop_safe` | Done |
| DPB R checkpoint 前崩溃保留 DROP 前的已提交状态（MODIFIED：窗口归属移交 FPL 域） | T4, T5, T6 | `tests/free_page_lifecycle_test.rs::crash_with_pending_release_keeps_old_table_rows` + 既有 `tests/drop_persistence_test.rs`（11 个用例） | Done |

命名说明：矩阵中的测试名是**计划中的用例名**；Act 可在不改变断言语义的前提下调整命名，并在 Act Response 记录实际名称。

场景文本修正说明（replan / T8）：`free-page-lifecycle` R4 场景与 `drop-persistence-boundary` R1（MODIFIED）场景的**崩溃点**与**前置条件**已按 DDL 语句边界有序落盘的既有行为修正（`DROP TABLE` 之后、任何后续 DDL 成功返回之前崩溃；DPB R1 前置条件补「也没有其它 DDL 语句成功返回」）。**Requirement 正文与本矩阵的映射不变**（R1/R2/R3/R4、DTPF 两条、DPB R1 的 requirement 文本与对应测试均未改写），修正的是场景实例的可观察量表述。

## Iteration Plan

### Iteration 000: 重新打开时按持久目录重建空闲页集合——释放的页跨重启可复用

- Tasks: T1, T2, T3
- Depends on: None
- Stable result: 重新打开数据库时，依据**持久目录**重建本次打开的可分配页集合：持久目录引用不到的页成为可分配页，重建覆盖目录链、表数据链、溢出链与三类索引树（并含恢复期重建的内存索引树页），且**任何仍被持久目录登记的页都不进入可分配页集合**。重建遇错则整次放弃（本次打开无可分配页，既有表照常可读）。⇒ 「反复打开并删除表之后，已释放空间仍可被后续分配使用」在跨重启维度成立。
- Verification boundary: 本 Iteration 的独立完成判据为 T1–T3 的定向用例通过，命令形如 `cargo test --test free_page_lifecycle_test -- --test-threads=1`。**本 Iteration 不跑全量回归**：全量是本 change 的收尾验证，预算记在 Iteration 001（全 change 一次）。
- Diagnostic boundary: 失败时按「目录链遍历 → 表数据链遍历 → 溢出链遍历 → 索引树遍历 → 内存索引树并集 → 页数上界 → 失败安全降级」七段定位；活页误判为空闲是最危险的失败模式，判读依据是「重建出的空闲集合是否恰好等于被释放的页集合」。
- Non-goals: 释放的持久化判定与会话内复用时机（Iteration 001）；尾部截断与整库整理；无键行旧版本 GC（I038）；文件格式与伴生文件变更；ISS22 / ISS23 / ISS24 / ISS25 的修复。

### Iteration 001: 释放的持久化判定——未完成释放永不可分配，关闭 MS33 遗留窗口

- Tasks: T4, T5, T6, T7, T8
- Depends on: Iteration 000
- Stable result: 被释放的页只有在其释放已被一次「在释放完成之后开始且成功完成」的 checkpoint 刷脏页持久化之后，才进入可分配集合；释放动作本身不写零页；刷盘失败时待释放项被丢弃（不提升）。⇒ MS33 R1 末句移交的窗口（「同会话复用释放页后在分配方 catalog 落盘前崩溃」）不再以「旧表数据被清零且不可恢复」的形式出现：崩溃后旧表已提交行仍可查询、未完成的释放所涉及的页不是可分配页、完整性无 error 级发现。崩溃点取在释放之后、任何后续 DDL 语句成功返回之前——`CREATE TABLE` 等 DDL 的语句边界有序落盘会把该语句的提交点与其它脏页一并落盘，先前的目录删除因而随之落盘（该形态下旧表在重开后不再存在，由 `drop-persistence-boundary` 域的崩溃模型界定）。同会话复用的时机从「释放后立即」改为「释放持久化后」（唯一的行为变更面，已在 `design.md` D2.6 声明）。
- Verification boundary: 独立完成判据为 T4–T6 的定向用例通过（`cargo test --test free_page_lifecycle_test -- --test-threads=1`、`cargo test --test drop_table_free_test -- --test-threads=1`、`cargo test --test overflow_page_test -- --test-threads=1`、`cargo test --test drop_persistence_test -- --test-threads=1`、`cargo test --lib free_overflow_chain_returns_pages_for_reuse -- --test-threads=1`）。
  **本 Iteration 承担本 change 的收尾验证：M19 分批全量回归（见下）**——按 M19 与 `.agents/runbooks/m19-batched-full-regression.md`（R48）执行一次，覆盖目标清单 = 变更前 `tests/` 全部测试文件 + `--lib --bins`。该收尾全量的执行依据是 **T7**（纪律逐条写死在 `iterations/001-durable-release-gate/000-initial.md` 的 T7「M19 分批串行全量回归纪律」第 1–9 条）。
- Diagnostic boundary: 失败时按「待释放集合入队 → 释放完成编号推进 → checkpoint 第 2 步钩子取样 → 刷盘成功提升 / 刷盘失败丢弃 → 六个释放点的完成标记」五段定位；「页被提升得过早」与「页永远不被提升」是两个相反的失效方向，分别用「崩溃后旧表行缺失」与「库文件持续增长」判读。
- Non-goals: `TRUNCATE` 自身的既有崩溃语义（`truncate-table` 域记录的「部分清空」形态）；MS33 R2–R4 与 MS34 三个新增域的语义；尾部截断与整库整理；I038；ISS22–ISS25。

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

**覆盖对账基准**：变更前的 `tests/` 文件清单 + `--lib --bins`（收尾时按实际文件列表重算，并与本 change 新增的 `tests/free_page_lifecycle_test.rs` 一并纳入）。

## 平衡审计

| Iteration | 聚合/拆分判定 | 结论 |
|---|---|---|
| 000 | 内聚结果 = 「重新打开时按持久目录重建空闲页集合」；同在存储/页管理层、同一定位边界（活页集合枚举）、同一验证入口（新增 `tests/free_page_lifecycle_test.rs`）。T1/T2/T3 分属「建立先行测试与可观察能力 → 接入缝与打开钩子 → 实现重建」，三者单独不构成稳定结果，合并为一个 Iteration 成立。 | 不拆分 |
| 001 | 内聚结果 = 「释放的持久化判定与会话内复用时机」；落在释放/checkpoint 交互层，与 Iteration 000 的「打开时重建」是不同故障域（释放时机 vs 活页枚举）、不同定位边界（待释放集合 vs 遍历段）、不同验证入口。单独成立且可独立验收。 | 不拆分 |
| change 整体 | 重建与持久化判定同属「空闲页生命周期」这一条不变量链（N1：释放在崩溃边界上成立才可分配）的两半：前者提供跨重启的空闲集合来源，后者提供会话内的释放时机判定。拆成两个 change 会让中间态出现「跨重启可复用但窗口未关」或「窗口已关但跨重启不可用」的半成品语义，两者共用一次收尾全量回归预算。 | 一个 change、两个 Iteration |

**首个 Iteration 承载范围检查**：Iteration 000 只承载「打开时重建」这一半成果，不因编号为 `000` 而承载整个 change（释放的持久化判定在 Iteration 001）✓。

**行为回退面声明**：Iteration 001 会把「同会话立即复用」改为「释放持久化后复用」。这是本 change 唯一的行为回退面，已在 `design.md` D2.6 与 `drop-table-physical-free` 的 MODIFIED 增量中显式声明，并由 T6 的既有用例断言分界更新覆盖。

## 最终状态与 Iteration Map 终态

- **change 状态**：全部任务 done；两个 Iteration 均已 `accepted`；**无后继 Iteration**（`Next Iteration: None`）；change 进入收尾。
- **最终 Review 依据**：`iterations/001-durable-release-gate/002-replan.md` 的 `Plan Review`（Review Result: `accepted`）——T8 与 A8–A11 全部满足，父 Cycle 的 GAP-1 / GAP-2 已闭合，三份规格增量合并后不新增任何与实现相反的断言；`design.md` D4 的不可达窗口分句已在该 Review 中更正。
- **收尾交接**（`openspec-docs-maintainer` 执行，本次 Plan Review 不执行）：合并三份规格增量进 `openspec/specs/`（新建 `free-page-lifecycle` 域；MODIFIED `drop-persistence-boundary` R1 与 `drop-table-physical-free` 两条）、同步全局 tasks / SNAPSHOT / M / R / I、归档 change。**本 change 另有一项规格文本候选已移交**（既有域 `drop-table-physical-free` 的基线 requirement 与实现的偏差，含两处 MS33 遗留残留），由随后的承载 change 处理，详见 `002-replan.md` 的 Issue 候选 1。

| Iteration | Tasks | 依赖 | Review Result | 终态 |
|---|---|---|---|---|
| 000 重新打开时按持久目录重建空闲页集合 | T1, T2, T3 | None | `accepted` | 完成（`iterations/000-reopen-free-page-reconstruction/000-initial.md`） |
| 001 释放的持久化判定 | T4, T5, T6, T7, T8 | Iteration 000 | `accepted` | 完成（`iterations/001-durable-release-gate/002-replan.md`） |
| — | — | — | — | **Next Iteration: None**（Iteration Map 无后继 Iteration） |
