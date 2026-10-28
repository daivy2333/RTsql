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

## Act Response

- Status: reported
- Cycle: Iteration 003 / Cycle 000（initial）· Task T5、T6
- 基线: HEAD `8cc35a5`，分支 `changes/2026-10-28-ms42-module-boundaries`
- 产品代码零改动：`src/`、`tests/`、`Cargo.toml`、`Cargo.lock` 全未触碰（本 Cycle diff 只含 change 产物 `design.md` 与 `tasks.md`）

### 实际改动

本 Cycle 无产品代码与测试代码改动。写入面两个 change 产物：

1. `openspec/changes/2026-10-28-ms42-module-boundaries/design.md` —— 3 行**计数类**事实陈述修正（T6 显式授权）：

| 位置 | 原文 | 修正为 | 代码事实依据 |
|---|---|---|---|
| §6.1 现状出口清单 | 「35 个 `eprintln!`/`println!` 直写出口…wal/recovery 7」 | 「34 个…（排除 cfg(test)；文档注释中的形态引用不计调用点）：…wal/recovery 6」 | 全仓 `grep -oE '\b(eprintln!\|println!)' src/` 命中 **35** 次，其中 `src/wal/recovery.rs:169` 是文档注释里的形态引用（`/// 恢复告警的**报出**——沿用本文件既有的 `eprintln!` 形态（stderr）。`），非调用点；真实调用点 **34**：table_manager 12（2135–2715）、wal/recovery 6（184,190,200,1635,1646,1658）、profiling 5、btree/index_manager 4、network/server 3、executor/drop_index 2、data_page 1、database 1，合计 34。三个含 `#[cfg(test)]` 的文件（`table_manager.rs:2819`、`btree/index_manager.rs:909`、`data_page.rs:417`）其出口行全部落在 cfg(test) 之前，无需再扣 |
| §6.2 约定正文第 5 条 | 「35 个既有出口由 MS43/MS44 按域增量迁移」 | 「34 个既有出口…」 | 出口总数随 §6.1 同步，条款语义不变 |
| §5.2 所有权裁定 | 「…见 R79 §4.1（12 字段）」 | 「…见 R79 §4.1（字段数以 `pub struct Database` 的结构体字段为准：15 字段——buffer_pool、table_manager、transaction_manager、wal_writer、wal_buffer、plan_cache、schema_version、checkpoint_manager、isolation、views、mode、query_memory_limit、io_failpoint、implicit_transactions、path）」 | `src/database.rs:110-155` 的 `pub struct Database` 逐字段核对实有 **15** 个字段；「12」与结构体字段数、与 R79 §4.1 表行数三者互不相等 |

修正只改计数与事实陈述，未改契约语义、未新增条款（符合 Invariants「契约记录性成果」与 Non-goals「契约条款新增或语义改写」）。

2. `openspec/changes/2026-10-28-ms42-module-boundaries/tasks.md` —— 按实际完成同步状态：T5/T6 两行 `pending → done`、§4.1/§4.2 复选框 `[ ] → [x]`、当前批次状态行与 Iteration 003「执行依据（落地信息）」行的待执行表述改为已完成（Act SKILL Phase 4 第 6 步：initial Cycle 更新所属 Iteration 状态）。

### T5 五组合矩阵结果（Verification 1–11）

**组合构建门（5 条）**

| 组合 | 构造 | 命令 | 退出码 | 结果 |
|---|---|---|---|---|
| C0 默认 | 不加 feature 参数 | `cargo check --offline --locked --lib --bins --tests` | 0 | Finished dev profile，crate 告警 0（唯一输出为 cargo 自身 `config` 弃用提示） |
| C1 no-network | `--no-default-features --features cli,crypto` | `cargo check --offline --locked --no-default-features --features cli,crypto --lib --bins --tests` | 0 | 同上 |
| C2 no-crypto | `--no-default-features --features cli,network` | `cargo check --offline --locked --no-default-features --features cli,network --lib --bins --tests` | 0 | 同上 |
| C3 no-cli | `--no-default-features --features network,crypto` | `cargo check --offline --locked --no-default-features --features network,crypto --lib --tests` | 0 | 同上 |
| C4 core-only | `--no-default-features` | `cargo check --offline --locked --no-default-features --lib --tests` | 0 | 同上 |

**行为验证集（4 组合 × 2 条 = 8 条）**

| 组合 | 命令 | 退出码 | `test result:` 计数 |
|---|---|---|---|
| C1 | `cargo test --offline --locked --no-default-features --features cli,crypto --lib` | 0 | 431 passed; 0 failed; 0 ignored |
| C1 | `cargo test --offline --locked --no-default-features --features cli,crypto --test storage_test --test parser_test --test executor_test` | 0 | storage_test 38、parser_test 6、executor_test 21；0 failed |
| C2 | `cargo test --offline --locked --no-default-features --features cli,network --lib` | 0 | 419 passed; 0 failed; 0 ignored |
| C2 | `cargo test --offline --locked --no-default-features --features cli,network --test crypto_off_build_test --test storage_test --test parser_test` | 0 | crypto_off_build_test 3、storage_test 6、parser_test 21；0 failed（`crypto_off_build_test` 目标在 C2 内存在且运行，符合风险说明） |
| C3 | `cargo test --offline --locked --no-default-features --features network,crypto --lib` | 0 | 412 passed; 0 failed; 0 ignored |
| C3 | `cargo test --offline --locked --no-default-features --features network,crypto --test storage_test --test parser_test --test executor_test` | 0 | storage_test 39、parser_test 6、executor_test 21；0 failed |
| C4 | `cargo test --offline --locked --no-default-features --lib` | 0 | 390 passed; 0 failed; 0 ignored |
| C4 | `cargo test --offline --locked --no-default-features --test storage_test --test parser_test` | 0 | storage_test 6、parser_test 21；0 failed |

**质量门（3 条）**

| 门 | 命令 | 退出码 | 结果 |
|---|---|---|---|
| fmt | `cargo fmt --check` | 0 | 无输出 |
| clippy（默认组合） | `cargo clippy --offline --locked --all-targets -- -D warnings` | 0 | Finished dev profile，无告警 |
| clippy（core-only） | `cargo clippy --offline --locked --no-default-features --lib --tests -- -D warnings` | 0 | Finished dev profile，无告警（cfg 剥空与门控在关闭组合零告警） |

**T5 结论**：12 项验证（5 条构建门 + 4 组行为验证集 + 3 条质量门，共 **16 次 cargo 调用**）退出码全为 0（5 构建门 + 4 组合验证集 + 3 质量门）。C0 默认组合的行为验证即 T6 的 M19 分批全量，本轮只跑 C0 构建门与两条质量门。`--lib` 用例数 C0→C4 为 436 / 431 / 419 / 412 / 390，差额来自 cli / network / crypto 模块单测按组合剥除，按 Plan Context 风险说明不构成失败信号。

### T6 M19 分批全量（Verification 1–5）

**批计划（执行时实际枚举，未沿用规划基线）**

- `ls tests/*.rs | sort` → **N = 158** 个集成测试文件（`tests/` 下另有 `fixtures/`、`support/` 两个子目录，均无 `main.rs`，不构成 cargo 测试目标；目标名去重后 158 个，无重复）
- B = ceil(158/12) = **14**；B×12−N = 168−158 = **10** ⇒ 前 10 批各 11 个、后 4 批各 12 个（实测各批规模序列 `11 11 11 11 11 11 11 11 11 11 12 12 12 12`，合计 158）
- `--lib --bins` 单独成批 ⇒ **15 批 / 159 个测试目标 / 160 条 `test result:` 行**（`--lib --bins` 批产出 lib 与 bin 两条）

**执行形态**：批间严格串行——同一时刻只有一批在跑（三段后台任务串行接力，全程无并发 cargo 进程）；批内 `--test-threads=1`；全部命令带 `--offline --locked`；`Cargo.lock` 零变化；无一次无目标过滤的全量调用；无默认并行度跑任何全量批次；判定只看 cargo 退出码与原生 `test result:` 行；批次切分用 `ls tests/*.rs | sort` + `sed -n` 单步管道，**未新增脚本、封装命令、专用参数、判定层或任何 Hash/指纹/run-id/manifest 类身份证明机制**。

**逐批结果（执行序）**

| 序 | 目标清单 | 规模 | 退出码 | passed | failed | ignored | `test result:` 行数 |
|---|---|---|---|---|---|---|---|
| 0 | `--lib --bins`（lib + `rtsql` bin） | 1 目标 / 2 二进制 | 0 | 436（lib）+ 0（bin） | 0 | 0 | 2 |
| 1 | aggregate_extension_test, aggregate_output_shape_test, aggregate_test, alter_table_test, attach_session_test, btree_merge_test, btree_scale_test, btree_split_test, btree_test, busy_timeout_test, cache_perf_test | 11 | 0 | 149 | 0 | 0 | 11 |
| 2 | catalog_bounded_read_test, catalog_durability_test, chain_join_test, check_command_test, checkpoint_command_test, checkpoint_redo_reduction_test, checkpoint_skip_test, checkpoint_test, cli_test, composite_index_lifecycle_test, composite_index_runtime_test | 11 | 0 | 191 | 0 | **2**（cli_test 的 2 个长期 `#[ignore]` 用例） | 11 |
| 3 | concurrent_test, connection_limit_test, constraint_enforcement_test, cross_db_dml_test, cross_db_query_test, crypto_off_build_test, ctas_test, cte_test, data_page_bounded_read_test, database_file_lock_test, datetime_function_test | 11 | 0 | 143 | 0 | 0 | 11 |
| 4 | datetime_type_test, ddl_catalog_transactional_test, ddl_crash_recovery_test, ddl_layout_publication_test, ddl_rewrite_consistency_test, default_declaration_test, default_precheck_test, derived_scan_alias_test, dml_tombstone_abandon_window_test, dml_tx_id_test, drop_persistence_test | 11 | 0 | 110 | 0 | 0 | 11 |
| 5 | drop_table_free_test, e2e_test, encryption_test, executor_test, explain_test, explicit_tx_test, expression_e2e_test, file_header_test, file_storage_io_test, free_page_lifecycle_test, full_table_dml_test | 11 | 0 | 175 | 0 | 0 | 11 |
| 6 | gc_test, group_by_expr_test, hash_join_build_side_test, import_input_surface_test, index_key_encoding_evaluation_test, index_manager_test, index_page_ownership_stack_overflow_test, index_page_ownership_test, index_range_routing_test, index_read_bounded_read_test, index_write_atomicity_test | 11 | 0 | 65 | 0 | 0 | 11 |
| 7 | insert_column_list_test, insert_select_test, isolation_level_test, join_output_resolution_test, join_test, key_type_conformance_test, keyless_eq_routing_test, keyless_row_test, legacy_compat_test, limit_test, memory_db_test | 11 | 0 | 116 | 0 | 0 | 11 |
| 8 | mvcc_abort_test, mvcc_commit_test, mvcc_record_test, mvcc_tombstone_visibility_test, nested_loop_join_test, network_protocol_test, network_server_test, no_from_select_test, order_by_alias_sort_test, outer_join_test, overflow_page_test | 11 | 0 | 96 | 0 | 0 | 11 |
| 9 | param_binding_test, parser_test, pg_integration_test, pg_messages_test, pg_protocol_test, pipeline_test, plan_cache_test, plan_exec_test, planner_test, predicate_test, prefetch_test | 11 | 0 | 221 | 0 | 0 | 11 |
| 10 | prepared_statement_test, projection_expression_test, projection_test, pushdown_test, query_memory_test, readonly_session_test, recovery_bounded_read_test, recovery_e2e_test, recovery_error_surface_test, recovery_replay_convergence_test, recovery_tail_persistence_test | 11 | 0 | 101 | 0 | 0 | 11 |
| 11 | recovery_test, recovery_wal_convergence_test, release_durability_precondition_test, release_path_bounded_read_test, runtime_test, scalar_function_test, scalar_subquery_comparison_test, schema_persistence_test, secondary_index_equality_test, secondary_index_test, select_distinct_test, self_join_test | 12 | 0 | 126 | 0 | 0 | 12 |
| 12 | set_operations_test, sort_merge_join_test, sort_test, sql_differential_test, statement_cancellation_test, storage_test, string_index_runtime_test, string_index_sql_test, string_secondary_index_test, structural_property_test, subquery_alias_binding_test, subquery_test | 12 | 0 | 185 | 0 | 0 | 12 |
| 13 | subset_insert_test, sync_loader_test, table_alias_resolution_test, table_manager_test, truncate_test, tx_statement_test, typed_api_test, update_expression_test, update_index_maintenance_test, upsert_test, value_test, version_chain_test | 12 | 0 | 157 | 0 | 0 | 12 |
| 14 | view_test, visibility_test, wal_auto_checkpoint_test, wal_buffer_test, wal_handle_test, wal_integration_test, wal_record_test, wal_recovery_large_test, wal_writer_test, write_failure_state_test, write_path_slot_bounds_test, write_type_conformance_test | 12 | 0 | 97 | 0 | 0 | 12 |

**汇总**：15 批退出码全 0；**passed 2368 / failed 0 / ignored 2**；`test result:` 行数 **160**（158 文件目标 + lib/bin 两条），与批计划预期一致。逐批 passed 独立复算：文件批 14 批合计 1932，`--lib --bins` 批 436，合计 2368。

两个 0 用例目标的口径说明（非失败信号）：

- `crypto_off_build_test`（批 3）在**默认组合**下为 0 用例——该文件是 `#![cfg(not(feature = "crypto"))]`，只在 crypto 关闭组合内存在断言（C2 组合内实有 3 个用例并通过）。
- `network_server_test`（批 8）为 0 用例——该文件用例体自 M8 提交 `1e4e3fc feat(server): switch to PgProtocol` 起整块注释（文件内注释明示「暂时跳过所有测试，等待新的 pg_integration_test」）；本 change 只在文件首行加了 `#![cfg(feature = "network")]`（diff 为证），0 用例是改动前既有状态，非本 change 引入的回归。

**三项覆盖对账（Verification 4）**

基准 = 执行时实际 `ls tests/*.rs | sort` 的 158 个文件目标；批表目标清单 = 执行时同一切分规则逐批产出的清单（14 个文件批）。

| 项 | 口径 | 结果 |
|---|---|---|
| missing | 实际文件集中未被任何批次覆盖的目标 | **0** |
| extra | 批次清单中不存在于实际文件集的目标 | **0** |
| duplicate | 批次清单中重复出现的目标 | **0** |

三项差集内容为空；批计划目标数 158 = 实际文件集 158。

**全量结论**：全部 15 批各自退出码 0 且 failed = 0，三项对账 0/0/0，无缺批，全量通过。M19 遵守情况：只在默认组合、整个 change 一次（本 Iteration 内）、批间串行、批内 `--test-threads=1`、无无过滤全量调用、无默认并行度全量批次、无失败批（故无需整批复核）。

### design §5 / §6 契约核对（Verification 6）

**§5.1 层构成与公共接口** —— 逐项核对符号存在、可见性、归属层：

| 层 | 核对项 | 结论 |
|---|---|---|
| 入口 | `rtsql::cli::run()`（`src/cli/mod.rs:242` `pub async fn run() -> ExitCode`；`src/lib.rs:4` `#[cfg(feature = "cli")] pub mod cli;`） | 一致（符号存在、可见性 `pub`、归属入口层） |
| 入口 | `Server::new` / `shutdown_token`（`src/network/server.rs:20,29`，`pub fn`） | 一致 |
| 入口 | `network::protocol::to_response`（`src/network/protocol.rs:70`，`pub fn`） | 一致；「97 个测试文件用作断言词表」独立复核 = **97**（`grep -l "network::protocol" tests/*.rs` 恰 97，且与含 `to_response` 的文件集同一集合） |
| 入口 | `PgProtocol`（`src/network/pg_protocol.rs:24`）/ `JsonProtocol`（`src/network/protocol.rs:87`） | 一致 |
| 入口 | 「入口层不被任何层引用（network、cli 均为叶子）」 | 一致：`grep -rn "crate::network\|crate::cli" src/` 在 `src/network/`、`src/cli/` 之外命中 **0** |
| 核心 | `Database` open 家族（open / open_with_isolation / open_with_key / open_with_lock_wait / open_readonly / open_readonly_with_key / open_readonly_with_lock_wait / open_in_memory，`src/database.rs:199-286`）、显式事务 API（`begin`:562、`commit`:573、`execute_in_tx`:764、`execute_in_tx_with_params`:775）、`checkpoint()`:875、`prepare()`:793 与 `pub struct PreparedStatement`:927、`set_query_memory_limit`:719 | 一致（全部 `pub`，归属核心层） |
| 核心 | pipeline 族 `parse_stage`:135 / `plan_stage`:187 / `plan_stage_routed`:235 / `execute_stage`:398 / `execute_stage_in_tx`:1262 / `execute_stage_in_tx_routed`:1272、`AttachRoute`:62、`RoutedTables`:46、`infer_ctas_column_types`:1120（`pub(crate)`，CLI 复用：`src/cli/lifecycle.rs:12` 导入并在 808 行调用） | 一致 |
| 核心 | `SqlOutcome`（`src/result.rs:25`）/ `SqlError`:40 / `ColumnMeta`:18，lib 根重导出 `pub use result::{ColumnMeta, SqlError, SqlOutcome};` | 一致 |
| 核心 | `with_profiling_scope`（`src/profiling.rs:60`）/ `is_profiling_enabled`:55；`PhysicalPlan`（`src/executor/plan.rs:24`），lib 根重导出 `pub use executor::{..., PhysicalPlan, ...}` | 一致 |
| 规划 | `parse_sql`（`src/parser/ast.rs:7`）、`PlanBuilder`（`src/parser/planner/mod.rs:155`）、`PlanError`（`src/parser/error.rs:7`）、`classify_attach_statement`:127 / `classify_transaction_statement`:57 / `AttachInfo`:113 / `TxStatementKind`:43（均为 `pub(crate)`，CLI 消费）、「二级索引名名单注入面」（`PlanBuilder.existing_index_names` + `set_existing_index_names`） | 一致 |
| 执行 | `Executor` trait（`src/executor/executor_trait.rs:11`，`pub use executor_trait::Executor;`）、`next_batch`（trait 方法，executor_trait.rs:22）、`query_budget` 预算面（`pub use query_budget::QueryBudget;` ⇒ `rtsql::executor::QueryBudget` 公开）、「32+ 执行器」实测 `pub use …Executor` **36** 个 | 一致 |
| 执行 | `Value` / `ColumnType` / `datetime` 词表 | 偏差 F1（见「自检发现」）；词表归属与消费面（存储 / pg 协议消费 `datetime`）本身一致 |
| 执行 | 「`plan_format`（EXPLAIN）」列入「对外公共接口（其他层实际消费）」列 | 偏差 F2（见「自检发现」）——契约语义不受影响 |
| 存储 | `AsyncStorage`（`src/storage/async_storage.rs:6`）与「`Arc<dyn AsyncStorage>` 持有点 database.rs:332」 | 一致（`src/database.rs:332` 逐字命中） |
| 存储 | `BufferPool`:20、`TableManager`:512 / `TableMeta`:383、`IndexManager`:146、`CatalogRow`:92 / `CatalogColumnRow`:123 / `CatalogKind`:82、`RowId`:5、`check_integrity`:87、`StorageError`:8、`WalWriter`:24 / `WALBuffer`:19 / `CheckpointManager`:14 / wal `RecoveryManager`:22（design 记作「Recovery」，指 `src/wal/recovery.rs` 模块与其 `RecoveryManager` 类型，符号存在） | 一致 |
| 事务 | `TransactionManager`:59（begin/commit/abort/abort_by_id/discard_tx_version_tables 唯一 WAL 事务记录源）、`TransactionId`:3、`Snapshot`:20 / `IsolationLevel`:26、`VersionHeader`:16、`TransactionSession`:19、`abort_by_id`:162 / `discard_tx_version_tables`:198 | 一致 |

**§5.2 状态归属**

| 项 | 结论 |
|---|---|
| 「会话级可变状态统一归 `Database` 协调器所有（`Arc` 共享、克隆共享全部可变态）」 | 一致；字段数陈述按代码事实已修正为 15（见「实际改动」） |
| 「`plan_cache` 读写独占 pipeline」 | 一致（`plan_cache` 定义于 `src/plan_cache.rs`，读写经 pipeline） |
| 「`schema_version` 单点递增在 pipeline `bump_schema_version`，database 5 处消费」 | 一致：`bump_schema_version` 单点定义于 `src/pipeline.rs:383`（全仓唯一定义）；database 侧 5 处消费点逐点命中（796 读、804 写、993 读、998 读、1004 写） |
| 「`wal_buffer` 三层持有入口（执行器写路径 + TransactionManager 内部槽 + CLI shutdown）」 | 一致：执行器写路径（`executor/{delete,update,insert,upsert,alter_table}.rs` 持有 `Option<Arc<WALBuffer>>`）、TransactionManager 内部槽、CLI shutdown（`src/cli/mod.rs:1185`）三层均在；归属仍为 Database |
| 「`views` 注册表归 Database；planner 持有注入快照副本（非共享 Arc）」 | 一致：`Database.views: Arc<RwLock<HashMap<String, ViewMeta>>>`；planner 侧为 `pub(crate) views: HashMap<String, ViewDef>` + `set_views`（普通 HashMap，非共享 Arc） |
| 「`io_failpoint` 打开期定型，主库 FileStorage 与 WalWriter 共享同一单元；ATTACH 附加库各自独立」 | 一致：`src/database.rs:330` 构造单例后分别分发 `open_with_lock_io_failpoint` / `open_with_io_failpoint`（同一 `IoFailPointSlot`） |
| 「会话级后台对象归属其宿主管理器；生命周期随 `Database` 实例」 | 一致（`WALBuffer` 后台 flush / `WalWriter` 句柄 / `TransactionManager` 内部登记 / `BufferPool` 并发结构 / `IndexManager` 写侧闸门均以 `Arc` 挂在 Database 持有链上） |
| 「ATTACH 会话状态归入口层 CLI（`AttachSession`，cli/mod.rs:756，注释明确『刻意不放在 Database』）」 | 一致：`src/cli/mod.rs:756` 正是 `struct AttachSession`，其上 753 行注释逐字含「刻意不放在 `Database`」 |

**§5.3 依赖方向与环清单**

| 项 | 结论 |
|---|---|
| 允许方向（入口 → 核心 → {规划, 执行} → {存储, 事务, 持久化}；规划 → 执行；result/profiling 零环跨切面） | 一致（`src/profiling.rs` 无跨层依赖：task_local + 环境变量 + 计时表） |
| 环 1a「database→pipeline（execute 族，database.rs:537-1013）→executor（工厂）→database（13 文件持具体类型）」 | 一致：`src/database.rs:537` 与 `:1013` 均为 `crate::pipeline::` 调用；executor 侧持具体 `Database` 类型的文件实测 **13** 个（create_table / create_index / create_view / drop_table / drop_view / drop_index / alter_table / truncate + subquery_eval / scalar_subquery_filter / semi_join / anti_join / join_related_config），与「DDL 执行器族 + 子查询/Join 配置族」描述吻合 |
| 环 1b「executor↔pipeline（`RoutedTables` 12 + `create_executor_from_plan` 5：子查询重建臂 5 文件）」 | 一致：`RoutedTables` 在 `executor/` 内实测 **12** 处（subquery_eval 4、scalar_subquery_filter 3、semi_join 2、anti_join 2、join_related_config 1）；`create_executor_from_plan`（定义于 `src/pipeline.rs:2216`）的外部调用方恰 **5** 个文件（executor 侧 4 个子查询重建臂 + `transaction/snapshot.rs`） |
| 环 1c「database↔transaction（transaction/session.rs:9 产品代码引 `Database`）」 | 一致：`src/transaction/session.rs:9` 逐字为 `use crate::database::Database;` |
| 环 2「storage↔transaction（`VersionHeader` 嵌入页格式 5 文件 6 处；反向 `page_format`/`RowId`/`ColumnType`）」 | 一致：storage 侧引入 `crate::transaction::VersionHeader` 的文件为 data/table_manager.rs、integrity.rs、data_page.rs、buffer_pool.rs、page_format/overflow.rs 共 **5** 个；反向边由 `transaction/manager.rs:1-2`（`crate::storage::page_format::SlottedPageRef` / `ColumnType`）与 `transaction/version_chain.rs:1`（`crate::storage::RowId`）构成 |
| 「本 change 起新增代码不得引入新的跨层环边」 | 一致：本 change 分支 `git diff 40e9ee8..HEAD -- src/` 新增/改动的 `use crate::` 行 **0** 条；src 侧改动仅 `src/lib.rs`（2 行 cfg 门）与 storage 三文件的 cfg 门控 |
| 词表归属遗留（`Value` 在 executor/value.rs、`ColumnType` 在 storage/page_format/tuple.rs、`PhysicalPlan` 在 executor/plan.rs、`datetime` 在 executor/datetime.rs） | 偏差 F3（见「自检发现」）：前三项定位准确，但 `ColumnType` 存在第二处同名定义 |

**§6.1 现状出口清单**

| 项 | 结论 |
|---|---|
| 直写出口总数与分解 | **偏差成立，已修正**：真实调用点 **34**（原述 35），分解 table_manager 12 / wal/recovery **6**（原述 7）/ profiling 5 / btree/index_manager 4 / network/server 3 / executor/drop_index 2 / data_page 1 / database 1 —— 其余模块分解与实算完全一致。差额全部落在 wal/recovery，多出的一处是 `src/wal/recovery.rs:169` 的文档注释形态引用（R79 与 design 的 7 来自朴素 `grep -c`，未剔除注释）。该文件在本 change 分支零改动（`git log 40e9ee8..HEAD -- src/wal/recovery.rs` 为空），偏差是 R79 的计数口径问题，不是本 change 的行为改动 |
| 既有计时设施「`profiling.rs`（task_local + `RTSQL_PROFILING` + `with_profiling_scope`）」 | 一致（`src/profiling.rs:10` `task_local!`、`:56` `std::env::var("RTSQL_PROFILING")`、`:60` `with_profiling_scope`） |
| 「产品接线 4 处（pipeline 三段、database 1、两个索引算子）」 | 一致：**4 个产品文件**含 profiling 调用（`src/pipeline.rs`、`src/database.rs`、`src/executor/index_scan.rs`、`src/executor/index_scan_all.rs`），括号内枚举的 6 个调用点逐点命中（pipeline.rs:120/124/1416、database.rs:965、index_scan.rs:69、index_scan_all.rs:86）——「4 处」与括号内 6 个调用点是文件口径与调用点口径两个并存口径，均成立 |
| 「库公共面单漏斗 `SqlError { message, sqlstate }`」 | 一致（`src/result.rs:40-43` 两字段逐字命中） |
| 「SQLSTATE 分类三源（parser/error.rs:145、pipeline.rs:1673-1686、network/pg_messages.rs:266）」 | 一致（三处行号逐字命中：`pub fn sqlstate` 145、`fn storage_error_sqlstate` 1673、`pub fn map_error_to_sqlstate` 266） |
| 「各层 thiserror 三个 error.rs」 | 一致（`src/network/error.rs`、`src/storage/error.rs`、`src/transaction/error.rs` 用 `thiserror::Error`；`src/parser/error.rs` 不用） |

**§6.2 约定正文六条 —— 与现状不冲突**

| 条 | 结论 |
|---|---|
| 1 统一门面（约定名 `rtsql::diagnostics`） | 一致——`src/` 内 `diagnostics` 命中 **0**，门面尚未落地，与 §6.3 理由自洽；本 change 未新增任何直写出口（src diff 只含 cfg 门） |
| 2 数据模型 `DiagnosticRecord` | 一致（无实现，无冲突） |
| 3 交付 API `diagnostics::report(record)` / `diagnostics::scope(name)` | 一致（无实现，无冲突） |
| 4 职责边界（模块自产事件、门面独占配置/输出） | 一致（无实现，无冲突） |
| 5 迁移原则 | 一致（条款语义未变；数量随 §6.1 由 35 更正为 34） |
| 6 与错误通道的关系（错误走 `Result`，SQLSTATE 三源零变化） | 一致（本 change 对 SQLSTATE 三源零改动，三处行号在实施后仍逐字命中） |

**§6.3 不落该接口代码的理由** —— 一致：`src/` 内无 `diagnostics` 模块、无消费者；MS43/MS44 未开工，门面骨架无可观察行为可先行测试，符合「验证只证明目标行为」与最小必要实现。

### change 结构自检（Verification 7）

| 自检项 | 结论 |
|---|---|
| `tasks.md` 的任务状态与实际完成一致 | 一致（已同步：T5/T6 → done，§4.1/§4.2 复选框 → `[x]`，当前批次状态行与 Iteration 003 落地信息行改为已完成；T1–T4 的 done 与其各自 Cycle 的 `Act Response: reported` + `Review Result: accepted` 一致） |
| `specs/build-feature-combinations/spec.md` 与 `design.md` 同已实现行为一致 | 一致——spec 五条 Requirement（默认组合等价 / network / crypto / cli / 组合共存与验证命令面）与已实现行为逐条相符：默认组合 features = `cli,network,crypto`；`[[bin]] required-features = ["cli"]` 生效（C3/C4 构建门不含 `--bins`）；network/crypto/cli 三个门点到位；C2 内 `crypto_off_build_test` 3 用例通过（加密头拒绝 / 携带密钥拒绝 / 明文全功能）；spec「默认组合全量回归」场景的 passed/failed 计数与三项对账由本 Cycle 的 15 批结果承载 |
| `iterations/` 下 Iteration 与 Cycle 文件齐全 | 一致（000/001/002/003 四个 Iteration 目录，各含 `000-initial.md`，无缺件） |
| 各 Cycle 的 `Review Result` 与流程状态一致 | 一致（000/001/002 的 `Review Result: accepted` 与其 `Act Response: reported` 配对；003 的 Plan Context `Status: ready`、本 Act Response `Status: reported`、`Review Result` 仍为 pending，待 Plan Review 填写——符合「Plan 写完 Review 和后继产物后最后更新」） |
| `evidence/` 目录 | 不存在（Persisted Evidence = `none`，未创建占位目录） |

### 与计划的偏差

无实质偏差。两处非实质差异记录：

1. **`--lib --bins` 批的执行序**：Cycle First test 要求「先执行第 1 批 `cargo test --offline --locked --lib --bins -- --test-threads=1`」以确认串行单批形态，change `tasks.md` §4.2 亦写「`--lib --bins` 单独成第一批」；而 Cycle Verification 1 的批表编号把它放在 `B+1` = 第 15 批。本 Cycle 按 First test 先执行该批（表中序号 0），批内容、目标数与对账口径不变；批表位次仍为 B+1 = 15。
2. **组合 `--lib` 用例数**：C0 436 / C1 431 / C2 419 / C3 412 / C4 390，差额来自 cli / network / crypto 模块单测按组合剥除，按 Plan Context 风险说明不构成对账门槛，也不是失败信号。

### 自检发现

- **Gate 3（T5 task 级 review）**：diff 无产品代码与测试代码改动（`git status --short` 只命中 change 产物的 `design.md` 与 `tasks.md`）；12 条矩阵命令退出码全 0，无 crate 告警、无新增 `#[allow]`、无新增直写出口；测试不会因错误原因通过——C1/C2/C3/C4 的门控有效性由各组合 `--lib` 用例数递减（436/431/419/412/390）与「C2 内 `crypto_off_build_test` 目标存在且 3 用例通过」双面确认。
- **Gate 3（T6 task 级 review）**：批计划由执行时实际文件集合重算（N=158），未沿用规划基线值作判据；每批的退出码与计数均取自 cargo 原生 `test result:` 行；`design.md` 修正只改计数事实陈述，未改契约语义、未新增条款；判定层与身份证明机制为零（无新增脚本/封装命令/专用参数，无 Hash/指纹/run-id/manifest 链）。
- **契约核对发现**：3 处偏差，均为文档陈述层面、无行为影响，其中 1 处为计数类（已按授权修正 design），2 处为语义/完整性类（按 Task Definition 规则只回报不改条款）：
  - **F1（Minor，未修正）**：`design §5.1` 执行层行「`Value` 值词表（**存储 27 处**、result/network/cli 消费）」中的「存储 27 处」在当前代码事实下不可复现。storage/ 侧各口径实测互不相等且无一为 27：引用文件 **4** 个（table_manager / btree/index_manager / catalog / page_format/tuple）、`use …Value` import 语句 **3** 条、类型位点 **9** 处、含 `cfg(test)` 的行 **143**、剔除 `cfg(test)` 后行 **62**、`Value` token 出现 **159** 次；而 `src/result.rs` 的 `Value` 行数恰为 **27**——该数字与 result 侧精确吻合、与其余 storage 口径无一吻合，疑为 R79 中 storage 与 result 的消费面数字互换。**未替换数字**：原计数所依据的统计口径无法从代码事实唯一确定，替换为任一其他数字都属猜测而非「以代码事实为准」，故按差异回报交 Plan Review 裁定。契约语义（`Value` 是跨层共用词表，storage/result/network/cli 共同消费）不受影响。
  - **F2（Minor，未修正）**：`design §5.1` 执行层行把 `plan_format` 列入「对外公共接口（其他层实际消费）」列，但 `src/executor/mod.rs:39` 声明为私有 `mod plan_format;`，仅被同层 `src/executor/explain.rs:8` 消费（无跨层消费者）；EXPLAIN 的对外公共面是 `pub use explain::ExplainExecutor`（`src/executor/mod.rs:76`）。属可见性标注偏差，契约语义（执行层对外提供 EXPLAIN 计划树渲染）成立且不受影响。
  - **F3（Minor，未修正）**：`design §5.3` 词表归属遗留列举「`ColumnType` 在 `storage/page_format/tuple.rs`」，但代码另有一处同名定义 `src/executor/value.rs:10`；lib 根重导出的是 executor 版（`rtsql::ColumnType`，`src/lib.rs:19`），storage 版为 `rtsql::storage::ColumnType`（`src/storage/mod.rs:45`）。该条列举不完整（两个同名类型分属两层），补全属「契约条款新增」，超出本 Cycle 授权与 Non-goals，故只回报交 Plan 裁定。
- **无 Critical / Important 问题**，未触发 Gate 5，无 Blocker Handoff。
- **未发现当前 change 范围外的实质缺陷**（两个 0 用例测试目标经核实为改动前既有状态，非缺陷；无失败测试、无质量门告警）。

### 验证命令与实际结果

| # | 命令 | 退出码 | 实际结果 |
|---|---|---|---|
| 1 | `cargo check --offline --locked --lib --bins --tests`（C0 构建门 / T5 First test 起点 GREEN） | 0 | Finished dev profile，crate 告警 0 |
| 2 | `cargo check --offline --locked --no-default-features --features cli,crypto --lib --bins --tests` | 0 | 同上 |
| 3 | `cargo check --offline --locked --no-default-features --features cli,network --lib --bins --tests` | 0 | 同上 |
| 4 | `cargo check --offline --locked --no-default-features --features network,crypto --lib --tests` | 0 | 同上 |
| 5 | `cargo check --offline --locked --no-default-features --lib --tests` | 0 | 同上 |
| 6 | `cargo test --offline --locked --no-default-features --features cli,crypto --lib` | 0 | 431 passed; 0 failed; 0 ignored |
| 7 | `cargo test --offline --locked --no-default-features --features cli,crypto --test storage_test --test parser_test --test executor_test` | 0 | 38 / 6 / 21 passed；0 failed |
| 8 | `cargo test --offline --locked --no-default-features --features cli,network --lib` | 0 | 419 passed; 0 failed; 0 ignored |
| 9 | `cargo test --offline --locked --no-default-features --features cli,network --test crypto_off_build_test --test storage_test --test parser_test` | 0 | 3 / 6 / 21 passed；0 failed |
| 10 | `cargo test --offline --locked --no-default-features --features network,crypto --lib` | 0 | 412 passed; 0 failed; 0 ignored |
| 11 | `cargo test --offline --locked --no-default-features --features network,crypto --test storage_test --test parser_test --test executor_test` | 0 | 39 / 6 / 21 passed；0 failed |
| 12 | `cargo test --offline --locked --no-default-features --lib` | 0 | 390 passed; 0 failed; 0 ignored |
| 13 | `cargo test --offline --locked --no-default-features --test storage_test --test parser_test` | 0 | 6 / 21 passed；0 failed |
| 14 | `cargo fmt --check` | 0 | 无输出 |
| 15 | `cargo clippy --offline --locked --all-targets -- -D warnings` | 0 | Finished dev profile，无告警 |
| 16 | `cargo clippy --offline --locked --no-default-features --lib --tests -- -D warnings` | 0 | Finished dev profile，无告警 |
| 17 | `cargo test --offline --locked --lib --bins -- --test-threads=1`（T6 First test / 批 0） | 0 | lib 436 passed; bin 0 passed；0 failed；2 条 `test result:` 行 |
| 18 | 批 1–14 的 `cargo test --offline --locked --test <f…> -- --test-threads=1`（每批一次独立调用，批间串行） | 各批 0 | 见「M19 分批全量 · 逐批结果」表；合计 1932 passed / 0 failed / 2 ignored |

**Gate 4 判定**：T5 Verification 1–11 与 T6 Verification 1–7 全部满足，完成声明均基于本次真实运行（无沿用结论、无重复运行已通过验证、无失败批复核）。

### Persisted Evidence

None required（Plan Context Persisted Evidence = `none`；cargo 命令可低成本复现，不满足 required 白名单；未创建 `evidence/` 目录，未保存完整日志目录、源码副本或测试套件输出）。

### Experience Candidates

None —— 本 Cycle 未发现可重复或高风险的操作路径缺口；全量与矩阵结论均由 cargo 原生命令面承载；未发现当前 change 范围外的实质缺陷（F1/F2/F3 均为本 change 自有 design.md 的陈述精度问题，落在 change 范围内，已在本 Response 处置）。预期 RED 与普通测试通过不构成候选。

### 未解决问题

1. **F1**：`design §5.1`「`Value` 值词表（存储 27 处…）」的计数与当前代码事实不可复现（实测口径见上），未替换数字，待 Plan Review 裁定是否按 R79 的原始统计口径补正。
2. **F2**：`design §5.1` 执行层行 `plan_format` 的可见性标注偏差（私有模块 vs 对外公共接口列），待 Plan Review 决定是否补注。
3. **F3**：`design §5.3` 词表归属遗留未列出 `executor/value.rs:10` 的同名 `ColumnType`，待 Plan Review 决定是否补全。

三项均为文档陈述精度问题，不影响任何 Acceptance、不阻塞 Iteration 003 的完成判定。

## Plan Review

- Review Result: accepted

**Findings**

独立阅读本 Cycle 完整 diff（`8cc35a5..ed13665`）与实际代码，核对 Act Response 声明与代码事实：无阻塞 finding。Act 遗留的三条 Minor 经独立复算确认 design 文本与代码事实不符，已在 Plan 写入面按代码事实修正 design.md；另记一条来自前序 Iteration 002 Review 的非阻塞 Minor。

- 核查 A（Act 声明成立）：本 Cycle 产品代码零改动。`git diff --name-only 8cc35a5..ed13665` 恰命中 3 个 change 产物（本 Cycle 文件、`design.md`、`tasks.md`）；`git diff --name-only 8cc35a5..ed13665 -- src tests Cargo.toml Cargo.lock` 命中 **0**；工作区干净（`git status --porcelain --untracked-files=all` 为空，无脚本、夹具或临时产物残留）。
- 核查 B（design 三处计数修正成立，Act 复算与代码事实相符）：
  - §6.1 出口总数 35 → **34**：全仓 `grep -roE '\b(eprintln!|println!)' src/` 命中 35 次，其中 `src/wal/recovery.rs:169` 是文档注释里的形态引用（`/// 恢复告警的**报出**——沿用本文件既有的 `eprintln!` 形态（stderr）。`），非调用点；真实调用点 34。逐文件分解独立复算一致：table_manager 12、wal/recovery **6**（184、190、200、1635、1646、1658）、profiling 5、btree/index_manager 4、network/server 3、executor/drop_index 2、data_page 1、database 1，合计 34。三个含 `#[cfg(test)]` 的文件（`table_manager.rs:2819`、`btree/index_manager.rs:909`、`data_page.rs:417`）其全部出口行分别落在 2135–2715 / 809–884 / 408，均在 cfg(test) 段之前，无需再扣。
  - §6.2 第 5 条随 §6.1 由 35 更正为 34，条款语义与六条结构未变。
  - §5.2「12 字段」→ **15 字段**：`pub struct Database`（`src/database.rs:110-155`）逐字段核对实有 15 个字段，Act 列出的字段名与结构体字段逐项一致、顺序一致；「12」与结构体字段数、R79 §4.1 表 13 行三者互不相等成立。
  - 三处修正只改计数与事实陈述，未改契约语义、未新增条款——与本 Cycle Invariants「契约记录性成果」及 Non-goals「契约条款新增或语义改写」一致。
- 核查 C（tasks.md 状态同步成立）：T5、T6 两行 `pending → done`、§4.1/§4.2 复选框 `[ ] → [x]`、当前批次状态行与 Iteration 003「执行依据（落地信息）」行改为已完成，与本 Cycle Act Response `Status: reported` 及实际完成范围一致；T1–T4 的 `done` 与各自 Cycle 的 `Act Response: reported` + `Review Result: accepted` 一致。
- F1（**已处置**·原 Act Minor·偏差成立）：`design §5.1` 执行层行「`Value` 值词表（存储 27 处…）」不可复现。独立穷举 storage 侧全部口径，无一为 27：引用 `Value` 的文件 4 个（其中 `btree/index_manager.rs:1256` 的 `use crate::Value;` 位于 `#[cfg(test)] mod tests` 内，非产品消费面）、`use …Value` import 3 条、匹配 `\bValue\b` 的行 143（剔除注释行 139）、token 159、`crate::executor::Value|crate::Value` 路径引用 31 行、`catalog.rs` 单文件 26 行 / 26 token / 25 处 `Value::`；而 `src/result.rs` 匹配 `\bValue\b` 的行恰为 **27**，在 R79 基线 revision `1f7dd6e` 上同样为 27（storage 侧 143/159 不变）。Act 的假设（R79 中 storage 与 result 的消费面数字互换）与代码事实相容，但原始统计口径无法从代码事实唯一确定，替换为任一其他数字都属猜测——故不替换数字，改为按代码事实给出可复现口径：**存储侧消费面按引用文件计 = 3 个产品文件（catalog.rs、data/table_manager.rs、page_format/tuple.rs）+ 1 个测试文件（btree/index_manager.rs）**，并保留 result/network/cli 消费面的原语义。契约语义（`Value` 是跨层共用词表，存储/result/network/cli 共同消费）不变。
- F2（**已处置**·原 Act Minor·偏差成立）：`plan_format` 不是「对外公共接口（其他层实际消费）」。独立核对：`src/executor/mod.rs:39` 声明为私有 `mod plan_format;`；全仓唯一消费点是同层 `src/executor/explain.rs:8`（`use crate::executor::plan_format::render_plan_tree;`），无跨层消费者；EXPLAIN 的对外公共面是 `pub use explain::ExplainExecutor;`（`src/executor/mod.rs:76`），由 core 层 `src/pipeline.rs:8`（导入）、`:2929`（构造）实际消费。design §5.1 执行层行据此更正为 `ExplainExecutor`（EXPLAIN 计划树渲染，由 core 层 pipeline 消费；渲染模块 `plan_format` 为执行层私有，不属对外公共面）——契约语义（执行层对外提供 EXPLAIN 计划树渲染）成立且未变。
- F3（**已处置**·原 Act Minor·偏差成立）：`ColumnType` 有两处同名定义——`src/storage/page_format/tuple.rs:41` 与 `src/executor/value.rs:10`；lib 根重导出执行层版（`src/lib.rs:19` 的 `rtsql::ColumnType`），存储层版为 `rtsql::storage::ColumnType`（`src/storage/mod.rs:45`）。design §5.3 的词表归属遗留列举补全为「`ColumnType` 在 storage/page_format/tuple.rs 与 executor/value.rs（两个同名类型分属存储与执行两层；lib 根重导出执行层版 …，存储层版为 …）」。归属决策本身未动（R25 词表归属下沉仍属 crate 拆分 change 的专项设计），本次只补全位置列举，未新增归属裁定或依赖方向条款。
- F4（记录·非阻塞·**Plan 错误**，归 Iteration 002 Review F2）：该 Review 的静态复数记「`src/cli/**` 25 处」，独立复算为 **24**（`src/cli/lifecycle.rs` 5、`render.rs` 13、`mod.rs` 4、`resolve.rs` 2），其派生基数 437 应为 436。差额 1 不影响任何 Acceptance——组合间 `--lib` 用例数差额按本 Cycle Plan Context 风险说明不是对账门槛，且本次 Act 记录的五组合计数经独立静态分解完全自洽（见 Evidence）。仅记录，不返工。

偏差分类：本 Cycle 无 Plan 遗漏、无 Plan 错误、无 Act 偏离、无现状变化。F1/F2/F3 属 **新证据**（Act 的契约核对新暴露的计数口径与可见性事实，Plan 独立复算后落为 design 修正）；F4 属 **Plan 错误**（Iteration 002 Plan Review 的静态计数，偏差 1，非阻塞）。F1–F3 均落在本 change 范围内（change 自有 design.md 的陈述精度），不构成 Issue 候选。

**Acceptance Gaps**

None。五条 Acceptance 逐条锚定：

- R5（组合共存与验证命令面）：5 条组合构建门 + 4 组合行为验证集 + 3 条质量门，共 16 次 cargo 调用，Act 记录退出码全 0（12 项判定）；本次 Review 未重跑（覆盖未变化，直接沿用，来源见 Evidence）。
- R6（默认组合 M19 分批全量）：15 批各自退出码 0 且 failed = 0；missing / extra / duplicate 三项对账经 Review 独立重算为 0/0/0（见 Evidence）。
- R7（六层公共接口与状态归属契约与代码事实一致）：§5.1/§5.2/§5.3 逐项核对表完成；两处计数类偏差（§6.1 出口 35→34 与 wal/recovery 7→6、§5.2 12→15 字段）已由 Act 按代码事实修正，三条陈述精度偏差（F1/F2/F3）已由本 Review 按代码事实修正——§5 与代码事实的偏差至此全部处置，无未处置偏差。
- R8（诊断报告接口约定与现状出口清单一致）：§6.1 出口清单与分解、计时设施、错误通道三源与 thiserror 三个 error.rs 逐项核对完成；§6.2 六条与现状不冲突（`src/` 内 `diagnostics` 零命中，门面未落地与 §6.3 自洽）；§6.3 理由成立。
- R1 收尾级再验证：默认组合分批全量覆盖测试全集面（15 批 / 159 个测试目标 / 160 条 `test result:` 行），构成「默认组合行为等价」的收尾证据。

**Evidence**

代码与 diff 独立核对（静态，本次当次执行；**本 Review 未执行任何 cargo 命令，未重跑 M19 全量或任何已通过且覆盖未变化的验证**）：

- diff 范围与零改动面（见 Findings 核查 A、C）。
- §6.1 出口计数与分解：全仓 `grep -roE '\b(eprintln!|println!)' src/` = 35 命中，逐文件分布 12/7/5/4/3/2/1/1；`src/wal/recovery.rs` 命中行 169、184、190、200、1635、1646、1658，其中 169 为文档注释 ⇒ 真实调用点 6，其余模块调用点全部在 cfg(test) 段之前 ⇒ 合计 34。
- §5.2 字段数：`pub struct Database` 结构体逐字段核对 = 15，字段名与 design 修正后列举逐项一致。
- M19 批计划独立重算（不复用 Act 的切分结果）：`ls tests/*.rs | sort` ⇒ **N = 158**；`tests/fixtures/`、`tests/support/` 无 `main.rs`，不构成 cargo 测试目标；目标名去重后仍 158，无重复。`B = ceil(158/12) = 14`，`B×12−N = 10` ⇒ 前 10 批各 11、后 4 批各 12（实测规模序列 `11 11 11 11 11 11 11 11 11 11 12 12 12 12`，合计 158），`--lib --bins` 单独成第 15 批 ⇒ 15 批 / 159 个测试目标 / 160 条 `test result:` 行。
- 三项对账独立重算：以执行时实际文件集合为基准按上述规则重切 14 个文件批，与 Act 批表**逐批逐目标比对**：14/14 批、目标清单**零差异**（0 处 mismatch），消耗文件恰为 158 ⇒ missing = 0、extra = 0、duplicate = 0 独立成立。
- 逐批计数自洽性：文件批 passed 合计 149+191+143+110+175+65+116+96+221+101+126+185+157+97 = **1932**；`--lib --bins` 批 436（lib）+ 0（bin）⇒ 合计 **2368**，failed 0，ignored 2（`cli_test` 的两个长期 `#[ignore]`，独立核对 `tests/cli_test.rs` 恰 2 处 `#[ignore]`）；`test result:` 行数 2 + 158 = **160**。与 Act 声明逐值一致。
- 组合 `--lib` 用例数独立静态分解：C0 436，剥除面 cli **24**（`src/cli/**` 静态用例数）、network **5**（`src/network/**`）、crypto **17**（`src/storage/crypto.rs` 8 + `file_header.rs` 3 + `file_storage.rs` 6，均带 `#[cfg(feature = "crypto")]` 门）⇒ C1 = 436 − 5 = 431、C2 = 436 − 17 = 419、C3 = 436 − 24 = 412、C4 = 436 − 24 − 5 − 17 = 390，与 Act 记录五个数字**逐值相同**。
- 零用例目标与既有状态：`tests/crypto_off_build_test.rs:8` 为 `#![cfg(not(feature = "crypto"))]`，默认组合下 0 用例属预期（C2 组合内 3 个 `#[tokio::test]` 用例存在）；`tests/network_server_test.rs` 用例体自 M8 起整块注释，本 change 该文件 diff 仅 +1 行 `#![cfg(feature = "network")]`，0 用例为改动前既有状态，非本 change 引入。
- 契约锚点抽查（§5.1/§5.2/§6.1）：`grep -l "network::protocol" tests/*.rs` = **97**，与含 `to_response` 的文件集同一集合；`src/executor/mod.rs` 中 `pub use …Executor` = **36**（满足「32+ 执行器」）；profiling 产品调用面为 **4** 个文件（`pipeline.rs`、`database.rs`、`executor/index_scan.rs`、`executor/index_scan_all.rs`）；`schema_version` 在 database 侧 5 处消费（796 读、804 写、993 读、998 读、1004 写）；持具体 `Database` 类型的 executor 文件 **13**；`RoutedTables` 在 executor 内 **12** 处；`create_executor_from_plan` 外部调用方 **5** 个文件（executor 侧 4 个子查询重建臂 + `transaction/snapshot.rs`，定义点在 `src/pipeline.rs`）；`grep -rn "crate::network\|crate::cli" src/` 在 `src/network/`、`src/cli/` 之外命中 **0**（入口层叶子性成立）。
- 判定层与身份证明机制残留扫描（本次当次执行）：change 分支 diff（`40e9ee8..HEAD`）中无新增脚本、封装命令或专用参数（新增非 `.rs`/非 openspec 文件仅 `Cargo.toml`、`Cargo.lock`）；无 Hash/指纹/run-id/session-id/manifest/checksum 类机制（`grep` 命中项全部是 `Cargo.lock` 的依赖校验和与无关的 `HashMap`/`HashSet` 业务用法）；批次切分只用 `ls tests/*.rs | sort` + `sed -n` 单步通用管道；工作区无未跟踪残留。
- 验证结论沿用来源：组合矩阵、质量门与 M19 全量结论产生于本 Cycle 的当次真实运行（Act Response 验证命令表 #1–#18），产生后覆盖范围未失效（HEAD 仍为 `ed13665`，工作区干净，本 Cycle 产品代码零改动），按公共规则 › 验证直接沿用，**未重跑**；本 Review 补跑的是 Act 未覆盖的独立静态复算（上述计数、切分与对账）。

**Follow-up Decision**

既有 Acceptance 全部满足且无阻塞项，Iteration 003 完成，本 change 的 Iteration Map 内无剩余 Iteration（T1/T2 → 000，T3 → 001，T4 → 002，T5/T6 → 003），不再创建后继 Cycle 或 Iteration。F1–F3 已在 Plan 写入面按代码事实修正 `design.md`（只改计数口径与事实陈述，未改契约语义、未新增条款、未新增归属裁定），修正后 §5/§6 与代码事实的偏差全部处置；F4 只记录不返工。Act Response 的 Runbook/Issue 候选均为 None，本次 Review 亦未发现本 change 范围外的实质缺陷，无 Issue 候选。收尾（规格合并、归档、状态同步）由 `openspec-docs-maintainer` 按职责执行，不在本 Review 范围内。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

None
