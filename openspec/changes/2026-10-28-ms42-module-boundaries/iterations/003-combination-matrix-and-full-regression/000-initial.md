# Iteration 003 / Cycle 000: 组合矩阵与收尾全量

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T5、T6
- Acceptance gaps: None
- Repair items: None
- Inherited scope: MS42 需求 R5（组合共存与验证命令面）、R6（默认组合 M19 分批全量）、R7（六层公共接口与状态归属契约）、R8（诊断报告接口约定）；design §2.5 组合矩阵、§5/§6 两份契约、§7 验证设计；M19 全量分批约束；Iteration 000/001/002 的稳定结果（features 表与三个 feature 门控齐备、默认组合行为零变化、C0–C4 各组合在所属 Cycle 内已成立）
- Excluded scope: 任何产品代码改动（src/、tests/、Cargo.toml）、新增 feature 门控、CLI/网络/加密行为变更、依赖环消除、诊断接口落地实现（design §6.3）、第二次全量、非默认组合全量、新平台验证、MS43/MS44 范围、change 收尾合并与归档

**Objective**

五组合矩阵（C0–C4）构建门与行为验证集全绿；默认组合 `cargo fmt --check` 与 deny clippy、core-only 组合 deny clippy 三条质量门退出码 0；默认组合按 M19 分批全量一次，每批 0 failed 且覆盖对账 missing/extra/duplicate 均为 0；design §5/§6 两份边界契约逐项核对实施后的代码事实，计数类偏差按代码事实修正 design、语义类偏差按差异回报；change 结构自检完成。

**Investigation Facts**

- Starting point: HEAD `163d1a9`（Iteration 002 accepted，Review 见 `../002-cli-optional/000-initial.md`）。三个 feature 门控至此齐备：`network`（Iteration 000）、`crypto`（Iteration 001）、`cli`（Iteration 002）；`Cargo.toml` 的 `[features]` 表与 `[[bin]] required-features = ["cli"]` 由 T1 就位。本 Cycle 不改产品代码——所有既有验证结论（各组合构建门与验证集、默认组合质量门）在覆盖未变化下直接沿用，但 T5 要求**逐一实跑**矩阵并留当次结果，T6 执行收尾全量。
- Current-State Evidence（本 Cycle 规划时逐点核实，基线 `163d1a9`；均为当次执行的静态核对结果）：
  - **组合矩阵命令面**（design §2.5 逐行，命令前缀统一 `cargo` + `--offline --locked`）：

    | 组合 | 构造 | 构建门 | 行为验证集 |
    |---|---|---|---|
    | C0 默认 | 不加 feature 参数 | `--lib --bins --tests` | 收尾 M19 分批全量 + fmt + deny clippy（归 T6） |
    | C1 no-network | `--no-default-features --features cli,crypto` | `--lib --bins --tests` | `--lib` + `--test storage_test --test parser_test --test executor_test` |
    | C2 no-crypto | `--no-default-features --features cli,network` | `--lib --bins --tests` | `--lib` + `--test crypto_off_build_test --test storage_test --test parser_test` |
    | C3 no-cli | `--no-default-features --features network,crypto` | `--lib --tests` | `--lib` + `--test storage_test --test parser_test --test executor_test` |
    | C4 core-only | `--no-default-features` | `--lib --tests` | `--lib` + `--test storage_test --test parser_test` |

    C1/C2 含 cli ⇒ 构建门含 `--bins`；C3/C4 无 cli ⇒ bin 目标被 `required-features` 跳过，构建门只含 `--lib --tests`。C0 的行为验证是全量，归 T6，T5 只跑 C0 构建门与质量门。
  - **定向验证并行度**：design §2.5 明定「定向单目标验证用默认并行度（M19 只约束全量）」；`cargo test <组合> --lib` 在四个非默认组合各跑一次，允许并行。bench 与 example 不进组合门（`harness = false` 的 bench 不被 `cargo test` 构建；`examples/bench_minimal.rs` 只用 `database::Database`）。
  - **质量门命令面**（design §7）：默认组合 `cargo fmt --check`、`cargo clippy --offline --locked --all-targets -- -D warnings`；core-only 组合 `cargo clippy --offline --locked --no-default-features --lib --tests -- -D warnings`（剥空与门控不得在关闭组合产生告警）。
  - **M19 批计划的规划时事实**（基线 `163d1a9`）：`ls tests/*.rs | sort` 实得 **N = 158** 个集成测试文件（`tests/` 下另有 `fixtures/`、`support/` 两个子目录，均无 `main.rs`，不构成 cargo 测试目标）。按 design §7 的等分规则 `B = ceil(N/12) = ceil(158/12) = 14`，`B×12−N = 10` ⇒ **前 10 批各 11 个文件、后 4 批各 12 个文件**（`10×11 + 4×12 = 158`），`--lib --bins` 单独成第 15 批 ⇒ **15 批 / 159 个测试目标**（158 文件目标 + `--lib --bins` 1 个）。**目标数与 `test result:` 行数不是同一数**：`--lib --bins` 一批产出 lib 与 bin 两条 ⇒ 预期 **160 条** `test result:` 行。执行时须以实际 `ls tests/*.rs | sort` 重新枚举并重算 B，基准必须是执行时实际文件集合，不得沿用 158。
  - **design §5 核对锚点**（§5.1 具名符号本轮已逐个核验存在，Act 只需补「可见性与归属层」判定）：入口层 `rtsql::cli::run()`（`src/cli/mod.rs`）、`Server::new`/`shutdown_token`（`src/network/server.rs:29,33`）、`network::protocol::to_response`（`src/network/protocol.rs:70`）、`PgProtocol`（`src/network/pg_protocol.rs:24`）、`JsonProtocol`（`src/network/protocol.rs:87`）；核心层 `Database` open 家族、`checkpoint()`（`src/database.rs:875`）、`PreparedStatement`（`src/database.rs:927`）、`set_query_memory_limit`（`src/database.rs:719`）、`parse_stage`/`plan_stage`/`plan_stage_routed`/`execute_stage`（`src/pipeline.rs:135,187,235,398`）、`AttachRoute`（`src/pipeline.rs:62`）、`RoutedTables`（`src/pipeline.rs`）、`infer_ctas_column_types`（`src/pipeline.rs:1120`）、`SqlOutcome`/`SqlError`/`ColumnMeta`（`src/result.rs:25,40,18`）、`with_profiling_scope`（`src/profiling.rs:60`）、`PhysicalPlan`（`src/executor/plan.rs:24`）；规划层 `parse_sql`（`src/parser/ast.rs:7`）、`PlanBuilder`（`src/parser/planner/mod.rs:155`）、`PlanError`（`src/parser/error.rs:7`）、`classify_attach_statement`/`classify_transaction_statement`/`AttachInfo`/`TxStatementKind`（`src/parser/planner/mod.rs:127,57,113,43`）；执行层 `Executor` trait（`src/executor/executor_trait.rs:11`）、`next_batch`（`src/executor/mod.rs:171`）、`query_budget`（`src/pipeline.rs:2`）、`plan_format`（`src/executor/explain.rs:8`）、`Value`/`ColumnType`/`datetime` 词表；存储层 `AsyncStorage`（`src/storage/async_storage.rs:6`）、`BufferPool`（`src/storage/buffer_pool.rs:20`）、`TableManager`/`TableMeta`（`src/storage/data/table_manager.rs:512,383`）、`IndexManager`（`src/storage/btree/index_manager.rs:146`）、`CatalogRow`/`CatalogColumnRow`/`CatalogKind`（`src/storage/catalog.rs:92,123,82`）、`RowId`（`src/storage/page_format/row_id.rs:5`）、`check_integrity`（`src/storage/integrity.rs:87`）、`StorageError`（`src/storage/error.rs:8`）、`WalWriter`/`WALBuffer`/`CheckpointManager`/`Recovery`（`src/wal/writer.rs:24`、`buffer.rs:19`、`checkpoint.rs:14`、`recovery.rs:22`）；事务层 `TransactionManager`（`src/transaction/manager.rs:59`）、`TransactionId`（`src/transaction/tx_id.rs`）、`Snapshot`（`src/transaction/snapshot.rs:20`）、`IsolationLevel`（`src/transaction/mod.rs:26`）、`VersionHeader`（`src/transaction/version_chain.rs:16`）、`TransactionSession`（`src/transaction/session.rs:19`）、`abort_by_id`/`discard_tx_version_tables`（`src/transaction/manager.rs:162,198`）。
  - **design §5.1 的计数陈述已核实成立**：「`network::protocol::to_response` …… 97 个测试文件用作断言词表」——实算含 `to_response` 且含 `network::protocol` 的测试文件恰 **97** 个；network 锚定文件共 104 个，97 + 7（协议/服务专项与函数内 import 形态）= 104，与 design §2.3 的规划值一致。
  - **design §5.2 的锚点已核实成立**：`AttachSession` 定义在 `src/cli/mod.rs:756`（design 引用的行号准确），其上 `src/cli/mod.rs:753` 的注释确含「刻意不放在 `Database`——attach 是 CLI 会话概念，lib 执行路径零变化」；`bump_schema_version` 单点在 `src/pipeline.rs:383`。
  - **design §6.1 的锚点已核实成立**：SQLSTATE 分类三源行号准确（`src/parser/error.rs:145` 为 `pub fn sqlstate`、`src/pipeline.rs:1673` 为 `fn storage_error_sqlstate`、`src/network/pg_messages.rs:266` 为 `pub fn map_error_to_sqlstate`）；「各层 thiserror 三个 error.rs」准确——`src/network/error.rs`、`src/storage/error.rs`、`src/transaction/error.rs` 用 `thiserror::Error`，`src/parser/error.rs` 不用；「产品接线 4 处」按**文件**口径准确——含 profiling API 调用的产品文件恰 4 个（`src/pipeline.rs`、`src/database.rs`、`src/executor/index_scan.rs`、`src/executor/index_scan_all.rs`）。
  - **design §5/§6 与代码事实的已知偏差（规划时已定位，2 处，均属计数类）**——T6 须逐项裁定并按 Task Definition 的规则处理：
    1. §6.1「**35** 个 `eprintln!`/`println!` 直写出口（排除 cfg(test)）」与分解「wal/recovery **7**」：按调用点实算为 **34** 个，差值全部落在 wal/recovery——实为 **6** 个调用点（`src/wal/recovery.rs:184,190,200,1635,1646,1658`）。R79 与 design 的 7 来自朴素 grep：`grep -c 'eprintln!' src/wal/recovery.rs` = 7，多出的一处是文档注释里的形态引用（`src/wal/recovery.rs:169`）。其余模块分解与实算一致（table_manager 12、profiling 5、btree/index_manager 4、network/server 3、executor/drop_index 2、data_page 1、database 1）。该文件在本 change 分支上零改动（`git log 40e9ee8..163d1a9 -- src/wal/recovery.rs` 为空），偏差是 R79 的计数口径问题，不是本 change 的行为改动。
    2. §5.2「具体字段清单与读写层见 R79 §4.1（**12 字段**）」：`pub struct Database`（`src/database.rs`）实有 **15** 个字段（`buffer_pool`、`table_manager`、`transaction_manager`、`wal_writer`、`wal_buffer`、`plan_cache`、`schema_version`、`checkpoint_manager`、`isolation`、`views`、`mode`、`query_memory_limit`、`io_failpoint`、`implicit_transactions`、`path`），R79 §4.1 表本身为 13 行（末行合并 `mode`/`isolation`/`path` 三项）。「12」与表行数、结构字段数三者互不相等。
  - **本 Cycle 的唯一潜在写入面**：T6 的契约核对若判定上述计数类偏差成立，按 Task Definition 修正 change `design.md` 的对应数字（§6.1 的出口总数与 wal/recovery 分解、§5.2 的字段数）。design.md 是 change 产物，随 change 归档；修正只改计数与事实陈述，不改契约语义、不新增条款。
- Code and Critical Path: 本 Cycle 不触碰产品代码路径。验证路径为 cargo 命令面：`cargo check`（组合构建门）、`cargo test`（组合验证集与全量批次）、`cargo fmt --check`、`cargo clippy`。唯一的文档路径是 `design.md` §5/§6 的计数陈述与实际代码事实的核对与必要修正。

**Implementation Guidance**

顺序：先 T5（矩阵 + 质量门），再 T6（全量 + 契约核对 + 结构自检）。T5 是低风险先行门——五组合构建门与验证集全绿是 T6 全量能够开始的前提；T6 的全量必须串行独占机器，不与任何其他 cargo 进程并行。

M19 全量执行形态（design §7 已定规则，Act 按此执行）：只在**默认组合**、**整个 change 一次**（本 Iteration 内执行）；非默认组合不承诺全量绿（crypto 域测试在 C2 预期失败，不入组合验证集）。批次规则：`ls tests/*.rs | sort` 取 N 个文件，`B = ceil(N/12)`，前 `B×12−N` 批各 11 个、其余各 12 个，`--lib --bins` 单独成一批。**SHALL NOT** 用一次无目标过滤的全量调用跑完（无论串行还是默认并行度），**SHALL NOT** 用默认并行度跑任何全量批次。批间严格串行（同一时刻只有一批在跑），批内 `--test-threads=1`。逐批记录退出码与 passed/failed/ignored，并做三项对账（missing / extra / duplicate 均为 0）。缺一批不得声明全量通过；失败批只整批复核一次（首跑失败记入验证记录，复核通过按通过处理）。

判定只看 cargo 退出码与原生 `test result:` 行。**不得**新增脚本、封装命令、专用参数、判定层，或任何 Hash/指纹/revision pin/run-id/session id/manifest 类身份证明机制（公共规则 › 行为约束）；计数若需过滤，只用现成通用工具的单步管道（如 `grep -E "^test result:"`），管道只计数不判定。

**Task Definitions**

### T5: 五组合矩阵收口 + 双组合质量门

- Requirement/Scenario: R5（组合共存与验证命令面；Scenario「组合矩阵收口」与「全关组合构建与单元测试」）；R1 默认组合等价不变量在矩阵面上的持续成立
- Depends on: Iteration 002 accepted
- Targets: 无产品代码改动。验证目标面：`src/`、`tests/`、`Cargo.toml` 的五组合构建图与测试目标集；质量门目标面为全仓 Rust 源
- Current behavior: 三个 feature 门控已落地（C3/C4 在 Iteration 002 内已各自通过其构建门与验证集）；五组合矩阵尚未在同一个 Iteration 内逐一实跑并留当次结果；core-only 组合的 deny clippy 门尚未执行过（Iteration 002 的 Plan Context 明确把 core-only clippy 归 T5）
- Required behavior: C0–C4 五组合的构建门退出码全为 0；C1–C4 四组合的行为验证集全部通过；默认组合 `cargo fmt --check` 与 deny clippy、core-only 组合 deny clippy 三条质量门退出码全为 0；任一组合失败即本任务失败
- Preserve: 产品代码零改动（不得为让矩阵通过而修改 `src/`、`tests/`、`Cargo.toml`）；默认组合行为等价；`Cargo.lock` 零变化（全部命令带 `--locked`）
- Forbidden: 修改任何产品代码或测试；新增/删除 feature；调整 `Cargo.toml`；跳过任一组合或其验证集；用默认并行度跑任何全量（全量归 T6）
- First test: 起点观察（变更前 GREEN）——`cargo check --offline --locked --lib --bins --tests`（C0 构建门）退出码 0 且无 crate 告警，作为矩阵执行的起点 GREEN
- GREEN condition: 5 条组合构建门 + 4 组合行为验证集 + 3 条质量门共 12 条命令退出码全为 0；每条组合的 `--lib` 用例数与 `test result:` 计数记入 Act Response
- Verification:
  1. C0 构建门：`cargo check --offline --locked --lib --bins --tests` 退出码 0。
  2. C1 构建门：`cargo check --offline --locked --no-default-features --features cli,crypto --lib --bins --tests` 退出码 0。
  3. C2 构建门：`cargo check --offline --locked --no-default-features --features cli,network --lib --bins --tests` 退出码 0。
  4. C3 构建门：`cargo check --offline --locked --no-default-features --features network,crypto --lib --tests` 退出码 0。
  5. C4 构建门：`cargo check --offline --locked --no-default-features --lib --tests` 退出码 0。
  6. C1 行为验证：`cargo test --offline --locked --no-default-features --features cli,crypto --lib` 退出码 0；`cargo test --offline --locked --no-default-features --features cli,crypto --test storage_test --test parser_test --test executor_test` 退出码 0。
  7. C2 行为验证：`cargo test --offline --locked --no-default-features --features cli,network --lib` 退出码 0；`cargo test --offline --locked --no-default-features --features cli,network --test crypto_off_build_test --test storage_test --test parser_test` 退出码 0。
  8. C3 行为验证：`cargo test --offline --locked --no-default-features --features network,crypto --lib` 退出码 0；`cargo test --offline --locked --no-default-features --features network,crypto --test storage_test --test parser_test --test executor_test` 退出码 0。
  9. C4 行为验证：`cargo test --offline --locked --no-default-features --lib` 退出码 0；`cargo test --offline --locked --no-default-features --test storage_test --test parser_test` 退出码 0。
  10. 质量门：`cargo fmt --check` 退出码 0；`cargo clippy --offline --locked --all-targets -- -D warnings` 退出码 0；`cargo clippy --offline --locked --no-default-features --lib --tests -- -D warnings` 退出码 0。
  11. 逐组合结果表（组合、构造、命令、退出码、`test result:` 的 passed/failed/ignored 计数）记入 Act Response；构建门失败的错误首行记入 Act Response。
- Stop when: 任一组合构建门或验证集失败、任一质量门非 0 退出、任一命令需要 `--locked` 之外的其他依赖解析改写（出现 lockfile 冲突）、出现需要修改产品代码才能通过的告警或错误——立即停止并按实际错误回报（不得改产品代码迁就矩阵）

### T6: 默认组合 M19 分批全量 + §5/§6 契约核对 + change 结构自检

- Requirement/Scenario: R6（默认组合 M19 分批全量一次通过；Scenario「默认组合全量回归」的 passed/failed 计数与三项对账）；R7（六层公共接口与状态归属契约与代码事实一致，文档性验证）；R8（诊断报告接口约定与现状出口清单一致，文档性验证）；R1 默认组合行为等价的收尾级再验证（测试全集面）
- Depends on: T5
- Targets: `tests/*.rs`（执行时实际文件集合）、`src/`（契约核对的只读对象）、`openspec/changes/2026-10-28-ms42-module-boundaries/design.md`（§5/§6 计数类偏差的修正面）、`openspec/changes/2026-10-28-ms42-module-boundaries/tasks.md` 与 `openspec/changes/2026-10-28-ms42-module-boundaries/specs/build-feature-combinations/spec.md`（结构自检的只读对象）
- Current behavior: 默认组合的测试全集从未在本 change 的最终形态下整体跑过（Iteration 000/001/002 均按 M19 不跑全量，全量预算留在本轮）；design §5/§6 的契约文本成型于 R79（基线 revision 1f7dd6e），其中两处计数陈述与实施后的代码事实不一致（见 Investigation Facts 的已知偏差两项）
- Required behavior: 默认组合全量按 M19 分批执行且每批 0 failed；覆盖对账 missing/extra/duplicate 均为 0；design §5 的六层接口与状态归属、§6 的诊断接口约定逐项核对完成，计数类偏差按代码事实修正 `design.md`，语义类偏差按差异回报；change 结构自检通过
- Preserve: 不得因全量结果去修改产品代码或调整判据（基线失效时差额必须由实际结果说明）；判定只看 cargo 退出码与原生 `test result:` 行；`design.md` 的契约语义与条款结构不因计数修正而改动
- Forbidden: 用一次无目标过滤的全量调用跑完全量；用默认并行度跑任何全量批次；并发启动多批；把规划时的 N=158 当作执行基准而不重新枚举；为对上历史基线数字而调整判据或剔除失败用例；新增脚本、封装命令、专用参数、判定层或任何 Hash/指纹/revision pin/run-id/manifest 类身份证明机制；保存完整测试套件输出或日志目录
- First test: 起点观察——生成批计划（`ls tests/*.rs | sort` 取实际 N，重算 B 与各批规模），并先执行第 1 批 `cargo test --offline --locked --lib --bins -- --test-threads=1`，确认串行单批形态成立、退出码 0、产出 lib 与 bin 两条 `test result:` 行
- GREEN condition: 全部批次（文件批 + `--lib --bins` 批）各自退出码 0 且 failed = 0；三项对账 missing/extra/duplicate 全为 0；design §5/§6 逐项核对表完成且计数类偏差已按代码事实修正；结构自检通过
- Verification:
  1. 批计划：以执行时实际 `ls tests/*.rs | sort` 为基准重算 N 与 `B = ceil(N/12)`，前 `B×12−N` 批各 11 个、其余各 12 个，`--lib --bins` 单独成第 `B+1` 批；批表逐批写明批号、目标清单、退出码、passed/failed/ignored。
  2. 逐批执行：每批一次独立调用 `cargo test --offline --locked --test <f1> ... -- --test-threads=1`（`--lib --bins` 批用 `cargo test --offline --locked --lib --bins -- --test-threads=1`）；批间严格串行，同一时刻只有一批在跑。
  3. 计数与判定：只看 cargo 退出码与原生 `test result:` 行；`--lib --bins` 批产出 lib 与 bin 两条 `test result:` 行，须与「测试目标数」口径分别记录。
  4. 覆盖对账：以执行时实际文件集合为基准逐项比对批次表的目标清单，出 missing / extra / duplicate 三项，三项均须为 0。
  5. 全量结论：等价于「全部批次各自 0 failed」；缺一批不得声明全量通过；失败批只整批复核一次，首跑失败记入验证记录。
  6. 契约核对（文档性验证，无自动测试）：逐项核对 design §5.1（六层构成与各层公共接口：符号存在、可见性、归属层）、§5.2（状态归属：Database 协调器所有权、`plan_cache` 独占 pipeline、`schema_version` 单点递增、`wal_buffer` 三层持有入口、`views` 注册表、`io_failpoint` 打开期定型、会话级后台对象归属、ATTACH 会话状态归 CLI）、§5.3（允许方向、环 1a/1b/1c/2 的边与位置、词表归属遗留）、§6.1（现状出口清单与分解、计时设施、错误通道三源与 thiserror 三个 error.rs）、§6.2（约定正文六条与现状不冲突）、§6.3（不落接口代码的理由）。核对表逐项写「一致 / 偏差 + 证据」；计数类偏差（已知两项：§6.1 出口总数 35→34 与 wal/recovery 7→6、§5.2 的 12 字段→15）以代码事实为准修正 `design.md` 对应数字；语义类偏差按差异回报（不改契约条款、不扩大本 Cycle 范围）。
  7. change 结构自检：`tasks.md` 的任务状态与实际完成一致；`specs/build-feature-combinations/spec.md` 与 `design.md` 同已实现行为一致；`iterations/` 下 Iteration 与 Cycle 文件齐全；各 Cycle 的 `Review Result` 与流程状态一致。
- Stop when: 任一批 failed（按 Verification 5 处理，仍失败即停止并回报，不重跑整轮全量）；批次无法串行执行（机器余量不足）；对账出现非 0 项且无法归因；契约核对出现需要改动契约语义或扩大 change 范围的偏差（按差异回报，返回 Plan）；自检发现 tasks/specs/design 与实现不一致且修正需要改写已冻结 Cycle 产物（停止并回报）

**Invariants**

- 产品代码零改动：src/、tests/、Cargo.toml 在本 Cycle 内一字不动；`Cargo.lock` 零变化。
- 默认组合行为逐字节等价（CLI 全部子命令、退出码、输出；库公共 API；磁盘格式）。
- 验证只证明目标行为：判定看 cargo 退出码与原生 `test result:` 行；无判定层、无身份证明机制（公共规则 › 行为约束）。
- M19：全量只在默认组合、整个 change 一次；批间串行、批内 `--test-threads=1`；缺一批不得声明全量通过。
- 不引入跨层依赖环边；新增代码（本 Cycle 为零）不引入 `eprintln!`/`println!` 直写出口。
- 契约记录性成果：design §5/§6 的语义不因收尾核对而改写，只允许计数类事实陈述的修正。

**Non-goals**

第二次全量、非默认组合全量、新平台验证、依赖环消除、诊断接口落地实现、契约条款新增或语义改写、产品代码与测试代码改动、change 收尾合并与归档、MS43/MS44 范围。

**Acceptance**

- R5（组合共存与验证命令面）：五组合构建门退出码全 0（↔ design §2.5 矩阵表；delta spec「组合矩阵收口」「全关组合构建与单元测试」）；C1–C4 验证集全通过；默认组合 fmt + deny clippy、core-only 组合 deny clippy 退出码 0（↔ design §7 质量门）。
- R6（默认组合 M19 分批全量一次通过）：每批退出码 0 且 failed = 0；missing / extra / duplicate 三项对账均为 0（↔ delta spec「默认组合全量回归」；M19 分批约束）。
- R7（六层公共接口与状态归属契约与代码事实一致）：design §5 的逐项核对表完成且无未处置的语义偏差；计数类偏差已按代码事实修正 design（↔ design §5.1/§5.2/§5.3；文档性验证，无自动测试）。
- R8（诊断报告接口约定与现状出口清单一致）：design §6 的逐项核对表完成；出口清单计数与实际调用点一致（↔ design §6.1/§6.2；文档性验证，无自动测试）。
- R1 的收尾级再验证：默认组合分批全量覆盖测试全集面，构成「默认组合行为等价」的收尾证据（首次测试仍为 T1 的改前/改后等价基线）。
- 映射：proposal 需求 5–8；design §2.5 / §5 / §6 / §7；tasks.md T5、T6 交付依据；change `tasks.md` RTM 的 R5/R6/R7/R8 四行（如实标注 R7/R8 为文档性验证、无自动测试）。

**Verification**

逐 Scenario 最简直接判定（真实入口：cargo 命令面）：

| Scenario | 判定 |
|---|---|
| 组合矩阵收口（R5） | T5 Verification 1–10（5 条构建门 + 4 组合验证集 + 3 条质量门，各自退出码 0） |
| 全关组合构建与单元测试（R5） | T5 Verification 5 + 9（C4 构建门 + C4 `--lib` 与核心目标） |
| 默认组合全量回归（R6 / R1 收尾级） | T6 Verification 1–5（批表逐批退出码 0、failed = 0；三项对账 0/0/0） |
| 六层公共接口与状态归属契约（R7） | T6 Verification 6（design §5 逐项核对表，文档性，无自动测试） |
| 诊断报告接口约定（R8） | T6 Verification 6（design §6 逐项核对表，文档性，无自动测试） |
| change 结构自检 | T6 Verification 7（tasks/specs/design 与实现一致、Iteration/Cycle 齐全、Review Result 与流程状态一致） |

**Gate 2 Readiness**

PASS（2026-10-28）。无 BLOCKED、无 SKIPPED。批准来自 goal 指令（「要求遵守各种规范尤其是多批次串行测试规范，中途发现的issue全都登记后纳入当前change计划新iter解决，开始吧继续当前change的工作」）对 MS42 范围的授权（Iteration 002 accepted 后按既有 Map 展开本 Iteration）。Step 5 审查计划全部通过：RTM 的 R5/R6/R7/R8 四行无 Missing（R7/R8 如实标注为文档性验证）；设计闭合（矩阵命令面、批次规则、契约核对清单与已知偏差均已具体化，无 TBD）；与 change 的 tasks、specs、design 一致；Persisted Evidence 模式明确为 `none`；无需要留给 Act 决定的接口或错误语义选择。

**Persisted Evidence**

- Mode: none

全量结论由 Act Response 的逐批表格（批号、目标清单、退出码、passed/failed/ignored）与三项对账结果承载；组合矩阵由逐组合命令与计数承载；契约核对由 design 的修正结果与 Act Response 的核对表承载。cargo 命令可低成本复现，不满足 required 白名单（无需保留不可低成本重跑的结果、结果不会因环境消失而失效、Act Response 足以承载结论）。禁止保存完整日志目录、源码副本或完整测试套件输出。

**Risks and Notes**

- 批计划按执行时实际文件集合重算；规划时基线（`163d1a9`）为 N = 158 → B = 14 → 10 批 × 11 + 4 批 × 12，`--lib --bins` 单独第 15 批 ⇒ 15 批 / 159 个测试目标 / 预期 160 条 `test result:` 行（`--lib --bins` 批产出两条）。目标数与行数是两个口径，两者各自成立，不互相替代。
- 组合 `--lib` 用例数在 C0–C4 间递减（cli / network / crypto 模块单测按组合剥除），差额不是失败信号，不构成对账门槛；各组合实际计数逐条记入 Act Response 即可。
- C2 的 `crypto_off_build_test` 只在 crypto 关闭时编译（`#![cfg(not(feature = "crypto"))]`），在 C2 组合内应存在并通过；若该目标缺失或未运行，属 T5 失败。
- 非默认组合不承诺全量绿（crypto 域测试在 C2 预期失败，不入组合验证集）——本 Cycle 不在非默认组合跑全量。
- core-only 组合的 deny clippy 是本轮新增的门面：cfg 剥空与门控不得在关闭组合产生告警；若出现告警，属 T5 失败（不得加 `#[allow]` 消警）。
- 全量预算只在本轮一次；失败批只整批复核一次（公共规则：验证通过一次即为通过，禁止重试直至通过；禁止为增强信心重复运行已通过的验证）。
- 全量执行期间不得并行启动其他 cargo 进程；机器余量不足时按公共规则记录事实并回报，不静默改验证面。
- 契约核对属文档性验证（无自动测试），其结论是「design 文本与代码事实的一致性判定」，不是行为断言；R7/R8 在 RTM 中如实标注。
- design §5/§6 的两处已知计数偏差已在 Investigation Facts 定位并给出证据，T6 按 Task Definition 的规则处理：计数类以代码事实为准修正 design，语义类按差异回报。
