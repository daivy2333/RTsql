# Tasks：MS36 历史库与 dump 兼容

状态：`pending` / `in_progress` / `done` / `blocked`。完整状态以 change 产物（Cycle 的 `Act Response`）为准。

## 任务列表

| ID | 任务 | Iteration | 状态 |
|---|---|---|---|
| T1 | 兼容样本产出与刻画断言：产出并固定旧库文件样本、旧库未决 WAL 样本、旧 dump 文本样本（含来源说明），新增 `tests/legacy_compat_test.rs` 断言打开、查询、写入、完整性、打开时重建、恢复，以及范围外的明确拒绝 | 000 | done |
| T2 | 规格增量：新建 `legacy-format-compat` 域（支持范围、逐代能力矩阵、范围外拒绝与迁移路径）+ 在 `database-file-format-header` 域增补「格式版本是产品格式契约」 | 001 | done |
| T3 | README 存量兼容段对齐：把支持范围、范围外拒绝形态与迁移路径写成与规格一致的表述 | 001 | done |
| T4 | 本 change 收尾全量回归（M19 分批串行） | 001 | done |

### 任务执行依据要点

- **T1**：样本是**固定字节输入**，由更早版本的产品构建产出后提交进仓库（`tests/fixtures/legacy/`），**运行期不得伪造版本号、不得伪造行布局、不得做版本/哈希/修订比对**。旧构建的可得性已验证：`git clone` 到隔离目录 → checkout `d8165fc` → `cargo build --offline --bin rtsql`（rustc 1.99.1 可构建）→ 用该二进制建库与 dump。样本来源与再生步骤写在 `tests/fixtures/legacy/README.md`。G1 库文件样本含**两张表**（一张被写到、一张不被写到），使「写入只升级被写到的表」可直接观察。
- **T1**：断言必须是**刻画断言**（当前行为即期望行为），预期写完即 GREEN；**不含任何产品代码改动**。若断言不成立，那是行为缺陷：按公共规则作为 Issue 候选上报并返回 Plan（需要新的执行依据时走 replan），不得在本 task 内顺手改产品代码。
- **T1**：用例走真实入口（真实文件库 + 真实 `Database::open` / `close` / 真实 `rtsql dump` / `restore` 语义），不得在框架外另建判定层；判定只看可观察结果（行集、错误文本、退出码语义、完整性发现数）。
- **T1**：样本规模保持小（行数 ≪ 300），避开 ISS23 的触发面；需要多页时用行宽控制而非行数。
- **T1**：范围外的三类拒绝断言必须包含**错误文本的关键片段**与**语句序号**，断言的是「明确拒绝且定位到语句」，不是「只要失败即可」。
- **T2**：只写规格增量文本，**不改产品代码**；增量文本必须与 T1 的实际断言结果逐条一致（能力矩阵逐格核对，每个子句都要能指认证据来源：无证据的子句改写或删除）。规格域用 `legacy-format-compat`（新域），`database-file-format-header` 用 ADDED 增补，不改写既有 requirement 正文。R5 的三条迁移路径必须**各实际执行一遍**（真实 CLI），规格里才可写「可执行」。
- **T3**：README 只改「存量兼容」相关段落，使其与规格域表述一致；不改其它章节。README 里的迁移路径必须可执行（用户按描述能完成迁移），只写 T2 已实测成立的路径。
- **T4**：执行依据见下方「收尾验证」，纪律逐条写死在 `iterations/001-contract-and-baseline/000-initial.md` 的 T4 段（本 Iteration 的 Cycle 文件由 Plan 在推进到该 Iteration 时展开）。

### 当前状态证据摘要（本次 Plan 实际运行所得）

| 事实 | 证据 |
|---|---|
| 格式版本只有 v1，从未变更 | `src/storage/file_header.rs:22` `FORMAT_VERSION = 1`；`git log -S "FORMAT_VERSION: u32"` 只有引入点 `2eda010`（MS10-T03）与其后未改 |
| 目录行代际的分界提交 | `6e8dcf5`（2026-09-26，MS23+MS24：加 unique_roots 段与列 DEFAULT 段）、`2f83dd9`（2026-09-30，MS21：加 secondary 段与 kind 字节）；G1 区间 = `4307a0e`(MS07) … `d8165fc`(2026-09-24) |
| 兼容读分支存在 | `src/storage/catalog.rs:1163`（unique_roots 段可选）、`:1245`（secondary/kind 段可选）、`:1465`（列 DEFAULT 段可选）；单测 `legacy_layout_row_deserializes_with_empty_unique_roots`、`legacy_row_without_kind_section_reads_as_table` |
| 当前行布局 = 旧行 + 追加尾段 | `src/storage/catalog.rs:1047` `serialize_catalog_row`、`:1298` `serialize_catalog_column_row` |
| 旧库打开/查询/写入成立 | 真实 G1 样本在当前构建下 `SELECT` 返回 3 行、`INSERT` 成功且重开可读、`check` 报 "check ok: 4 pages scanned, no corruption found" |
| 写入使被写到的表目录行升级 | 旧库导入 500 行后 `__tables` 目录行被重写为当前布局（字节层面出现 unique_roots=0、secondary=0、kind=1 尾段），旧行副本仅作死槽残留 |
| MS35 打开时重建在旧库上成立 | `src/database.rs:436` 调用 `table_manager.rebuild_free_page_set(path)`；真实旧库（含已释放页变体）打开无重建放弃消息、活表行完整、完整性零 error；`src/storage/data/table_manager.rs:1610`、`:1626`、`:1769` |
| 旧库带未决 WAL 可恢复 | 真实旧库 + 未决旧 WAL 在当前构建下恢复出 216 行，`check` 报 "check ok: 14 pages scanned, no corruption found" |
| 旧 dump 可恢复 | 真实旧 dump（`INT`/`STRING` 短类型名）restore 退出码 0，行数据等价；含 `DATE '…'`/`TIMESTAMP '…'`/`TRUE` 的旧 dump 同样恢复成功 |
| 范围外拒绝显式 | 非 INTEGER PK → "Plan error: Unsupported constraint: NON-INTEGER PRIMARY KEY"；非 INT UNIQUE → "Plan error: Unsupported constraint: UNIQUE (INT columns only)"；NOT NULL 含 NULL → "Execution error: NOT NULL constraint violation: column 's'"；三者均退出码 3 并带「第 k 条/共 n 条」 |
| 非 INTEGER PK 的当前 dump 往返可达 | 当前 `dump` 对非 INTEGER PK 表不渲染 `PRIMARY KEY`，restore 退出码 0、行等价 |
| 文件头范围外判定（既有覆盖） | `tests/file_header_test.rs`：`test_newer_version_rejected`、`test_headerless_legacy_file_rejected`（`NotADatabase`） |

### 已知外部约束（不在本 change 内修复）

| 约束 | 影响 | 处理 |
|---|---|---|
| ISS23（BTree 插入无限递归 → 栈溢出，`src/storage/btree/btree.rs:319-327`） | 兼容样本若使用大表规模会撞上 | 样本只用小表（行数 ≪ 300）；本次已验证 504 行的普通 INSERT/import 路径不触发该缺陷，其记录触发面为 `ALTER TABLE DROP COLUMN` 的完成路径，与本 change 场景无关 |
| ISS22 / ISS24 / ISS25（open） | 与存储层相邻 | 只作约束记录，不在本 change 内修改，也不作为本 change 的 Acceptance |
| restore 的逐条 auto-commit 与 fail-fast 语义（`src/cli/lifecycle.rs:574` 起的既有合同） | 多表 dump 中靠后语句被范围外拒绝时，靠前语句已生效 | 已知边界，明确记录；本 change 不改该语义（D4） |
| `dump` 对非 INT `UNIQUE` 列无条件渲染 `UNIQUE`（`src/cli/lifecycle.rs:986` `create_table_sql`） | 「旧库 → dump → 新库 restore」在该类表上被计划期拒绝 | 不是缺陷；可选改进登记于 `design.md` D9，不纳入本 change Acceptance |
| ISS25（提交点窗口位置性错读） | 曾建议与 MS36 相邻排期 | 由后续承载 change 处理，本 change 不涉及 |

## Requirements Traceability Matrix

`legacy-format-compat`（LFC，新域）与 `database-file-format-header`（DFH，增补一条 requirement）的 requirement 编号。

| Requirement | Task | First Test | Status |
|---|---|---|---|
| LFC R1 旧库格式的支持范围按目录行布局代际定义 | T1, T2 | `tests/legacy_compat_test.rs::legacy_generation_db_is_in_supported_range`（刻画 G1 样本落在支持范围内）；版本超限与无头文件的拒绝引用既有域 `database-file-format-header` | Done |
| LFC R2 旧库文件的打开、查询与写入 | T1, T2 | `tests/legacy_compat_test.rs::legacy_db_opens_queries_and_writes`、`::legacy_db_write_upgrades_touched_table_row_to_current_layout`；「写入后仍能再次写入」由本 change 的真实 CLI 实测覆盖 | Done |
| LFC R3 旧库文件上的打开时重建与未决 WAL 恢复 | T1, T2 | `tests/legacy_compat_test.rs::legacy_db_open_rebuild_keeps_live_table_intact`、`::legacy_db_with_pending_wal_recovers_committed_rows`；活页不进入可分配页集合的既有断言见 `tests/free_page_lifecycle_test.rs::rebuild_never_marks_live_pages_free` | Done |
| LFC R4 旧 dump 的恢复范围 | T1, T2 | `tests/legacy_compat_test.rs::legacy_dump_restores_into_current_db`；「恢复出的库可继续写入」由本 change 的真实 CLI 实测（restore → INSERT → SELECT）覆盖 | Done |
| LFC R5 范围外的显式拒绝与迁移路径 | T1, T2, T3 | `tests/legacy_compat_test.rs::legacy_dump_with_non_integer_primary_key_is_rejected`、`:legacy_dump_with_non_integer_unique_is_rejected`、`:legacy_dump_with_null_in_not_null_is_rejected`；三条迁移路径各一次真实 CLI 实测（记录见本 Cycle 的 Act Response） | Done |
| DFH R 格式版本是产品格式契约 | T2, T3 | `tests/file_header_test.rs::test_newer_version_rejected`、`::test_headerless_legacy_file_rejected`（既有覆盖，本 change 不新增） | Done |

命名说明：矩阵中的测试名在 T1 实施后即为**实际用例名**（Iteration 000 的 accepted Review 已核对 9 个用例名与矩阵逐条一致）；Act 可在不改变断言语义的前提下调整命名，并在 Act Response 记录实际名称。

映射完整性：LFC R1–R5 与 DFH R 全部有 task 与测试映射，无 `Missing`；无 `Simplified`（无需求被裁剪）。

## Iteration Plan

### Iteration 000: 兼容样本与刻画断言——旧库与旧 dump 的支持范围被固定样本锁定

- Tasks: T1
- Depends on: None
- Stable result: 仓库内存在一组**固定的旧格式样本**（旧代际库文件、旧代际库 + 未决旧 WAL、旧代际 dump 文本），以及一组走真实入口的刻画断言，使下列事实成为可复跑的基线：旧代际库**能打开、能查询、能写入、能通过完整性扫描、能通过打开时安全重建、带未决 WAL 时能恢复出已提交行**；旧代际 dump **能恢复到当前库且行数据等价**；三类范围外输入 **给出明确、定位到语句的拒绝**（错误文本点名原因、含第 k 条/共 n 条、退出码 3）。⇒ 「旧库格式与旧 dump 的支持范围」从此是可断言的事实，而不是散落在 README 里的描述。
- Verification boundary: 本 Iteration 的独立完成判据为 T1 的定向用例通过，命令形如 `cargo test --test legacy_compat_test -- --test-threads=1`。**本 Iteration 不跑全量回归**：全量是本 change 的收尾验证，预算记在 Iteration 001（全 change 一次）。样本生产（旧构建 + 建库 + dump）是一次性的离线准备动作，不是验证命令，也不计入验证预算。
- Diagnostic boundary: 失败时按「样本来源与字节 → 打开路径（文件头校验 → 目录行读取 → WAL 恢复 → 打开时重建）→ 查询路径 → 写入路径（目录行重写与升级）→ dump/restore 路径 → 拒绝面」六段定位；最危险的失效模式是「重建把活页判成空闲页」与「目录行升级写坏目录链」，判读依据分别是「活表行在重开后仍完整」与「升级后目录行仍能解析出同样的表定义」。
- Non-goals: 规格文本（Iteration 001）；README 表述（Iteration 001）；收尾全量回归（Iteration 001）；任何产品代码改动；旧库格式迁移工具；CI/发布设施（I051–I053）；键编码格式变更（I024）；伴生文件加密（I060）；ISS22–ISS25 的修复。

### Iteration 001: 支持范围写成为合同与基线——兼容合同成立

- Tasks: T2, T3, T4
- Depends on: Iteration 000
- Stable result: 「支持的旧库格式与旧 dump 范围」成为**正式合同**：规格域 `legacy-format-compat` 记录逐代能力矩阵（打开/查询/写入/完整性/打开时重建/dump/恢复）、范围外三类拒绝的形态与迁移路径，以及「写入使被写到的表目录行升级为当前布局」这一格式后果；`database-file-format-header` 域记录「格式版本是产品格式契约」——唯一版本为 v1、旧库的差别在目录行代际、产品不做旧格式嗅探与迁移；README 存量兼容段与上述合同逐条一致。⇒ 后续任何格式变更（I024 等）都有基线可对照：改动是否破坏已有数据，可按本支持范围判定。
- Verification boundary: 独立完成判据为 T2/T3 的文本与 T1 的实际断言逐条一致（能力矩阵逐格核对，迁移路径可执行），以及 **T4 收尾全量：M19 分批全量回归（见下）**。收尾全量的执行依据是 **T4**（纪律逐条写死在 `iterations/001-contract-and-baseline/000-initial.md` 的 T4「M19 分批串行全量回归纪律」第 1–10 条）。
- Diagnostic boundary: 失败时按「能力矩阵逐格 → 拒绝面逐条 → 迁移路径可执行性 → 合同与 README 表述一致性」四段定位；合同文本与实现事实矛盾时，以实现事实为准并修文本（文本不构成行为变更）。
- Non-goals: 恢复的原子化/全量预检语义变更；产品代码行为变更；`dump` 对非 INT `UNIQUE` 的渲染规则调整（`design.md` D9）；旧库格式迁移工具；CI/发布设施；I024；I060；ISS22–ISS25。

## 收尾验证：M19 分批全量回归（记在最后一个 Iteration）

**时机**：本 change 的最后一个 Iteration（001）收尾时执行一次（全 change 一次预算）。中途如需追加全量，须在本 Iteration Plan 记录理由。

**执行纪律（SHALL）**：

1. 测试目标按**确定性顺序**分组（`--lib --bins` 一批；`tests/*.rs` 按文件名字典序），每组一次独立的 `cargo test --test <f1> ... --test <fN> -- --test-threads=1` 调用。
2. 批间**顺序执行**，同一时刻只有一批在跑。
3. **SHALL NOT** 用一次无目标过滤的全量调用跑完（无论串行还是默认并行度）。
4. **SHALL NOT** 用默认并行度跑任何全量批次（每批必须带 `--test-threads=1`）。
5. 批大小 **8–12 个目标**。
6. 全量 Gate 等价于「**全部批次各自 0 failed**」；缺一批不得声明全量通过。
7. 结论必须记录**每批的 passed/failed 计数**，并做覆盖对账：目标清单 = 收尾时 `tests/` 全部测试文件 + `--lib --bins`（分批不重不漏）。
8. 判定只看 cargo 自己的退出码与原生输出；计数可用一步通用管道读出（`… 2>&1 | grep -E "^test result:"`），管道只做计数、不做判定；**不得**新增判定脚本、封装命令或判定层。
9. 单次目标验证（`cargo test --test <name>`）不受本纪律约束，可用默认并行度。
10. 批次内出现 failed 时**不得**重跑该批直到通过；重跑只允许用于判定是否偶发错误的一次复核，复核通过按通过处理并在验证记录注明首跑失败。

**批计划生成规则（本次明确，见 `design.md` D10）**：目标清单 = 收尾时 `tests/*.rs` 全部文件 + `--lib --bins`（本次为 135 + 1 = 136 个目标）。文件批数 `B = ceil(文件数 / 12)`；文件按字典序切片，前 `B × 12 − 文件数` 批各取 11 个目标，其余各取 12 个（保证每批落在 8–12 且不重不漏）；`--lib --bins` 单独成第 `B+1` 批。本次 135 个文件 → B = 12 → 9 批 × 11 + 3 批 × 12，加 `--lib --bins` 一批，合计 **13 批 / 136 目标**。

**覆盖对账基准**：变更前 `tests/` 文件清单 + `--lib --bins`（MS35 收尾时为 134 个文件 + `--lib --bins` = 135 目标），加本 change 新增的 `tests/legacy_compat_test.rs`；当前树为 **135 个文件 + `--lib --bins` = 136 目标**，收尾时按实际文件列表重算；预期 passed 总数 = 基线 2183 + 本 change 新增用例数（9）= **2192**，failed = 0，ignored = 2。

## 平衡审计

| Iteration | 聚合/拆分判定 | 结论 |
|---|---|---|
| 000 | 内聚结果 = 「旧库与旧 dump 的支持范围被固定样本锁定」；同一验证入口（新增 `tests/legacy_compat_test.rs` 走真实库 API 与真实 dump/restore 语义）、同一定位边界（样本字节 → 打开/恢复/重建 → 查询/写入 → dump/restore → 拒绝面）。样本生产与断言编写分属同一结果的两半，单独任一半都不构成稳定结果（T1 合并成立）。 | 不拆分 |
| 001 | 内聚结果 = 「支持范围写成为合同与基线」；落在规格与用户文档层，与 Iteration 000 的行为断言是不同故障域（合同表述 vs 样本行为）、不同定位边界（能力矩阵逐格 vs 打开/写入路径）、不同验证方式（逐条对照断言 + 收尾全量）。单独成立且可独立验收。 | 不拆分 |
| change 整体 | 000 提供「被断言的事实」，001 把同一事实写成为「可查证的合同」，两者是同一不变量的两个形态（事实 → 合同），不是两个独立成果；拆成两个 change 会出现「有合同无断言」或「有断言无合同」的半成品状态，且两者共用一次收尾全量回归预算。 | 一个 change、两个 Iteration |

**首个 Iteration 承载范围检查**：Iteration 000 只承载「样本与刻画断言」，不承载合同文本与收尾全量（均在 Iteration 001）✓。

**行为回退面声明**：本 change **无行为回退面**——产品代码零改动，新增用例是对既有行为的刻画断言，预期一写即 GREEN。唯一的行为相关产出是「把既有行为写成合同」，合同文本与实现事实冲突时以实现事实为准（Iteration 001 诊断边界已写明）。

## 最终状态与 Iteration Map 终态

- **change 状态**：两个 Iteration 均实施完成且 Plan Review 已执行，**两轮 Review Result 均为 `accepted`**。Iteration 000 的阻塞项 F1（A5「恢复出已提交行」的断言无判别力）与 F2（会分配页的写入路径缺数据存活与完整性断言）已在该 Iteration 内修复并经独立复核闭合（F1 门槛改为「恢复行数 == 参考值 133」，实测未重放时为 1 行 ⇒ 断言失败；F2 补齐文件增长断言与重开后的行可读 / 完整性断言）；其遗留非阻塞 Minor finding F3 / F4 / F5 / S1 / S2 由 Iteration 001 的 T2 / T3 消化。Iteration 001 的 Plan Review（`iterations/001-contract-and-baseline/000-initial.md`）独立复核了变更面、T2 的矩阵与逐条对账、T3 的双语对齐、T4 的分批与覆盖对账，并在 `/tmp` 用真实 CLI 复现 6 条迁移路径的退出码与结果（含 2b 的 `key column 'email' expects INT, got String` 拒绝）；结论 `accepted`，遗留非阻塞 Minor finding M1–M4 与 Acceptance Gap AG1（G2 文件级证据缺失）/ AG2（三处一次性实测子句无常态化用例），二者作为 Issue 候选 ISS26 / ISS27 交由后续承载 change 处理（只报告不登记）。详见两个 Cycle 的 Plan Review 与 Act Response。
- **当前 Iteration**：001（支持范围写成为合同与基线）**已完成**（Review Result = `accepted`，`Next Iteration: None`）；**无当前 Cycle**，无待执行 Cycle，无返工项。Act 已完成：T2 规格增量定稿（逐代能力矩阵 + 逐条对账 + 三条迁移路径实测）、T3 中英双语 README 对齐、T4 收尾全量 13 批 / 136 目标 / **2192 passed / 0 failed / 2 ignored**（覆盖对账 missing 0 / extra 0 / duplicate 0）。
- **后继 Iteration**：None（001 是本 change 的最后一个 Iteration）。
- **收尾交接**（`openspec-docs-maintainer` 执行，Plan 不执行）：合并规格增量进 `openspec/specs/`（新建 `legacy-format-compat` 域；`database-file-format-header` 增补一条 requirement）、同步全局 tasks / SNAPSHOT / M / R / I、归档 change。

| Iteration | Tasks | 依赖 | Review Result | 终态 |
|---|---|---|---|---|
| 000 兼容样本与刻画断言 | T1 | None | `accepted` | 实施完成 + Plan Review accepted（F1 / F2 已在该 Iteration 内闭合） |
| 001 支持范围写成为合同与基线 | T2, T3, T4 | Iteration 000 | `accepted` | 实施完成 + Plan Review accepted（T2 规格定稿、T3 双语 README 对齐、T4 收尾全量 13 批 / 136 目标 / 2192 passed / 0 failed / 2 ignored，覆盖对账 0 / 0 / 0；遗留 Minor finding M1–M4 与 Acceptance Gap AG1 / AG2 不阻塞，已作 Issue 候选 ISS26 / ISS27 移交） |
| — | — | — | — | — |
