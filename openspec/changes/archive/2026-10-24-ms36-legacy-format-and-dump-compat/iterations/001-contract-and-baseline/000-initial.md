# Iteration 001 / Cycle 000: 契约成文与收尾全量回归

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T2（规格增量定稿）、T3（README 存量兼容段对齐）、T4（本 change 收尾全量回归）
- Acceptance gaps: None（Iteration 000 的 A1–A9 已全部达成，其 Review Result = accepted）
- Repair items: None
- Inherited scope:
  - `legacy-format-compat` 域增量草案（5 req / 14 scen）已存在于 `specs/legacy-format-compat/spec.md`；`database-file-format-header` 的 ADDED 草案（1 req / 3 scen）已存在于 `specs/database-file-format-header/spec.md`——本 Cycle 在其上做**逐条对账与定稿**，不是从零起草。
  - Iteration 000 已验收的 9 个刻画用例、7 个固定字节样本（`tests/fixtures/legacy/`），以及「README 参考值与用例常量必须同步」的既有约定。
  - 既有 75 域规格（重点：`database-file-format-header`、`data-integrity-verification`、`cli-noninteractive-shell`、`schema-persistence`、`drop-persistence-boundary`）。
  - M19 分批全量回归纪律（`tasks.md` 收尾验证 10 条 + design D10 批切分算法）。
- Excluded scope:
  - 产品代码行为变更（本 Cycle 零 `src/` 改动）与用例增删改（零 `tests/` 改动）。
  - `restore` 原子化 / 全量预检、`dump` 对非 `INT` `UNIQUE` 的渲染规则（D9）、旧库迁移工具、CI 与发布设施、I024 / I060。
  - ISS22–ISS25（由后续承载 change 处理）。
  - 把增量规格合并进 `openspec/specs/`、更新 SNAPSHOT / 全局 tasks / M-R-I（`openspec-docs-maintainer` 收尾职责）。
  - 重新生成兼容样本（样本字节冻结；重新生成会改变恢复行数参考值）。

**Objective**

把 MS36 的支持范围从「测试里的断言」变成**可查证的合同文本**：规格增量 `legacy-format-compat` 含逐代能力矩阵且每个 Requirement / Scenario 子句都能指认证据来源；`database-file-format-header` 的 ADDED 保留「格式版本是产品格式契约、不引入身份机制」这一新增主张；中英双语 README 的「存量兼容」条目与规格逐条一致（含实测成立的迁移路径）。随后以 M19 分批串行纪律完成本 change 的收尾全量回归并对账，证明上述文本改动没有破坏既有 75 域的行为。

**Investigation Facts**

- Starting point:
  - revision `0a08b14`（ahead of origin 9）；工作区存在未提交现场（MS34 + issue-fix change + MS35 + 本 change 的 Iteration 000 改动）——不得回滚、不得清理、不得提交。
  - Iteration 000（`iterations/000-compat-samples-and-assertions/000-initial.md`）Review Result = accepted：F1（恢复行数门槛无判别力）与 F2（分配路径上的数据存活与完整性覆盖缺口）均已在该 Cycle 内闭合；遗留非阻塞 Minor finding 为 F3 / F4 / F5 / S1 / S2（见下）。
  - 沿用结论（来源：同一文件 Act Response「验证命令与实际结果」；被覆盖代码自该次运行后未变化）：`cargo test --offline --test legacy_compat_test -- --test-threads=1` → `9 passed; 0 failed; 0 ignored`；`rustfmt --edition 2021 --check tests/legacy_compat_test.rs` → 退出码 0；`tests/file_header_test.rs` → 14 passed；7 个样本文件字节数与 `tests/fixtures/legacy/README.md` 记载逐一一致。
  - 已知限制：样本字节冻结 ⇒ 恢复行数参考值 133 是「已提交字节的确定性属性」；重新生成样本会得到不同行数，届时必须同步 README 与 `RECOVERED_ROWS_REFERENCE`。
- Current-State Evidence:
  - 9 个刻画用例与其覆盖面（`tests/legacy_compat_test.rs`）：

    | 用例 | 锁定的事实 |
    |---|---|
    | `legacy_generation_db_is_in_supported_range` | G1 目录行形态（止于数据链尾指针）+ 表 / 列可读；判定不读文件版本标记 |
    | `legacy_db_opens_queries_and_writes` | 打开 / 查询 / 写入 / 重开读回；既有行未丢；完整性 0 error |
    | `legacy_db_write_upgrades_touched_table_row_to_current_layout` | 被写到的表目录行升级为当前布局；未被写到的 `audit` / `events` 保持旧代际；**会触发页分配的写入**之后既有已提交数据存活 + 完整性 0 error |
    | `legacy_db_open_rebuild_keeps_live_table_intact` | 打开时重建不放弃（stderr 无「重建放弃」）、活表完整、完整性 0 error |
    | `legacy_db_with_pending_wal_recovers_committed_rows` | 带未决写入日志打开后恢复出 133 行（`RECOVERED_ROWS_REFERENCE`）+ 种子行存活 + 完整性 0 error |
    | `legacy_dump_restores_into_current_db` | 旧 dump 恢复成功（退出码 0）+ 行数据等价（含日期 / 时间戳 / 布尔字面量）+ 再次导出等价 |
    | `legacy_dump_with_non_integer_primary_key_is_rejected` | 非整数 `PRIMARY KEY`：退出码 3、点名原因、失败语句序号 |
    | `legacy_dump_with_non_integer_unique_is_rejected` | 非 `INT` `UNIQUE`：退出码 3、点名原因、失败语句序号 |
    | `legacy_dump_with_null_in_not_null_is_rejected` | `NOT NULL` 含 NULL：退出码 3、点名原因与列名、失败语句序号 |

  - 规格增量逐条对账现状（14 个 Scenario，文件 `specs/legacy-format-compat/spec.md`）：

    | Scenario | 行 | 现状证据 | 判定 |
    |---|---|---|---|
    | 最旧代际库文件属于支持范围 | :19 | `legacy_generation_db_is_in_supported_range` | 已覆盖 |
    | 文件格式版本高于当前支持版本明确拒绝 | :26 | 既有 `tests/file_header_test.rs::test_newer_version_rejected`（`NewerFileVersion` + 「新版」消息）；子句「拒绝早于页解析 / 伴生文件打开 / 恢复」由既有域 `database-file-format-header` 的打开顺序守卫承载（本 change 未新增断言） | 已覆盖（引用既有域） |
    | 不含文件头的旧文件明确拒绝 | :33 | 既有 `test_headerless_legacy_file_rejected`（`NotADatabase`） | 已覆盖（引用既有域） |
    | 旧代际库打开并查询 | :43 | `legacy_db_opens_queries_and_writes` | 已覆盖 |
    | 旧代际库写入并读回 | :50 | 同上（重开后既有行 + 新行） | 已覆盖 |
    | 写入使被写到的表目录行升级为当前布局 | :57 | `legacy_db_write_upgrades_touched_table_row_to_current_layout`（含未被写到的表保持旧代际） | 已覆盖 |
    | 旧代际库完整性扫描零 error | :64 | A2 / A3 / A4 的 `error_count() == 0`；子句「扫描不修改文件内容」**无用例断言**（F5，属既有跨域行为） | 子句无断言 |
    | 旧代际库打开时重建不放弃且活表完整 | :77 | `legacy_db_open_rebuild_keeps_live_table_intact` | 已覆盖 |
    | 旧代际库带未决写入记录打开后恢复已提交行 | :85 | `legacy_db_with_pending_wal_recovers_committed_rows` | 已覆盖 |
    | 旧代际 dump 恢复到当前库 | :96 | `legacy_dump_restores_into_current_db`；子句「恢复出的库……可继续写入」**无用例断言**（S1） | 子句无断言 |
    | 旧代际 dump 的 DDL 渲染差异不影响恢复 | :103 | 同上 | 已覆盖 |
    | 非整数主键的旧 dump 明确拒绝 | :119 | `legacy_dump_with_non_integer_primary_key_is_rejected`；迁移路径子句：非整数 PK 路径已实测一次（见下） | 拒绝已覆盖；迁移路径有执行记录 |
    | 非整数唯一列的旧 dump 明确拒绝 | :126 | `legacy_dump_with_non_integer_unique_is_rejected`；迁移路径**未实测**（S2） | 拒绝已覆盖；迁移路径无证据 |
    | 非空列含空值的旧 dump 明确拒绝 | :133 | `legacy_dump_with_null_in_not_null_is_rejected`；迁移路径**未实测**（S2） | 拒绝已覆盖；迁移路径无证据 |

  - 逐代能力矩阵**尚未进入规格**：`specs/legacy-format-compat/spec.md:11`–`:17` 只按 G1 / G2 / G3 描述形态并声明三者都受支持，没有「代际 × 能力」矩阵，也未注明覆盖层级（G2 只有行级既有覆盖、G3 由当前构建自身写入）——F3。
  - 迁移路径的执行证据：非整数 `PRIMARY KEY` 路径已实测一次（规划期 Current-State Evidence：「当前 `dump` 对非整数 PK 表不渲染 `PRIMARY KEY`，restore 退出码 0、行等价」）；非 `INT` `UNIQUE` 与 `NOT NULL` 含 NULL 两条路径**从未执行**——S2。
  - README 现状（产品侧合同落点）：`README.zh-CN.md:277` 与 `README.md:277` 各有一条「存量兼容 / Legacy compatibility」条目，均只写「范围外的负面边界 + 迁移提示」，**没有**写明支持范围（旧库格式代际、旧 dump 支持范围、范围内各自能做什么），也没有写明「写入使被写到的表目录行升级为当前布局」与「格式版本是产品格式契约」这两条用户可见后果。
  - `database-file-format-header` 的 ADDED（`specs/database-file-format-header/spec.md`）：3 个 Scenario 中，「唯一的产品格式版本」是新增主张；另两个（高版本拒绝、无头文件拒绝）与既有域 `openspec/specs/database-file-format-header/spec.md` 的既有 Scenario 语义重复——F4。
  - 收尾全量回归的基线与现状：MS35 收尾全量 = **2183 passed / 0 failed / 2 ignored**，目标 = 134 个 `tests/*.rs` + `--lib --bins` = 135 目标 / 14 批（来源：MS35 change `iterations/001-durable-release-gate/000-initial.md` 的 T7 段）。当前 `tests/*.rs` = **135** 个（新增 `tests/legacy_compat_test.rs`）⇒ 本 change 收尾目标 = **136** / 13 批，预期总数 = 2183 + 9 = **2192 passed / 0 failed / 2 ignored**。
- Code and Critical Path:
  - 本 Cycle 的写入面仅限 change 产物与产品文档：`openspec/changes/2026-10-24-ms36-legacy-format-and-dump-compat/specs/legacy-format-compat/spec.md`、`.../specs/database-file-format-header/spec.md`、`openspec/changes/2026-10-24-ms36-legacy-format-and-dump-compat/tasks.md`（状态同步）、`README.md`、`README.zh-CN.md`、本 Cycle 文件。
  - `src/` 与 `tests/` **零改动**。
  - 迁移路径实测的入口：当前构建的真实 CLI（`target/debug/rtsql`）；输入为 `tests/fixtures/legacy/` 下的旧 dump 文本；实测在 `/tmp` 内进行（不修改仓库样本，不新增仓库脚本）。
  - 全量回归的入口：`cargo test --offline --test <f1> --test <f2> ... -- --test-threads=1`（每批一次独立调用）。

**Implementation Guidance**

- 顺序：T2 → T3 → T4。T2 定稿规格文本（T3 的措辞以 T2 为准），T4 最后跑（覆盖最终文本树）。
- T2 的对账方法：在增量文件内逐 Scenario 建立「子句 → 证据来源」对照；证据来源只能是三种：用例名 + 断言位置、既有测试文件 + 用例名、本 change 已记录的实际运行结果。**没有证据来源的子句不得留在规格里**——改写为可指认的表述，或删除。
- 能力矩阵每格必须带**覆盖层级**标注，且不得把「当前构建自身写入的代际」当成旧格式兼容证据：

  | 代际 | 打开 | 查询 | 写入 | 完整性 | 打开时重建 | dump | 恢复 |
  |---|---|---|---|---|---|---|---|
  | G1（最旧） | 文件级样本 | 文件级样本 | 文件级样本 | 文件级样本 | 文件级样本 | 文件级样本 | 文件级样本（4 份旧 dump 文本） |
  | G2 | 行级既有覆盖 | 行级既有覆盖 | 行级既有覆盖 | 行级既有覆盖 | 行级既有覆盖 | — | — |
  | G3（当前） | 当前构建原生路径（非兼容证据） | 同左 | 同左 | 同左 | 同左 | 同左 | 同左 |

  G2 / G3 各格的具体行级用例名由 Act 在对账时逐格填出（格式由上表结构决定）。
- 迁移路径实测：三条路径各自在 `/tmp` 内用真实 CLI 走一遍（编辑 dump 文本 → `restore` → 记录退出码与结果）；只有实测成立的路径才能以「可执行」写入规格与 README，未成立的按实测改写。
- T3 的措辞以 T2 的规格文本为准；中英双语两条目同步改写，不新增章节。
- T4 严格按 Task Definition 的纪律执行；判定只看 cargo 的退出码与 `test result:` 行。

**Task Definitions**

### T2: 规格增量定稿——逐代能力矩阵 + 逐条对账 + 迁移路径实测

- Requirement/Scenario: `legacy-format-compat` R1–R5 的全部 14 个 Scenario；`database-file-format-header` ADDED 的 3 个 Scenario
- Depends on: T1（Iteration 000，已 done）
- Targets:
  - `openspec/changes/2026-10-24-ms36-legacy-format-and-dump-compat/specs/legacy-format-compat/spec.md`
  - `openspec/changes/2026-10-24-ms36-legacy-format-and-dump-compat/specs/database-file-format-header/spec.md`
- Current behavior: 增量草案已存在（5 req / 14 scen + 1 req / 3 scen），但：(a) 逐代能力矩阵只在 `proposal.md`，未进规格，也未注明覆盖层级（F3）；(b) 两个子句无用例断言——R2 场景 4 的「扫描不修改文件内容」（F5）、R4 场景 1 的「恢复出的库……可继续写入」（S1）；(c) 三条迁移路径中两条未实测（S2）；(d) DFH ADDED 的两个 Scenario 与既有域重复（F4）。
- Required behavior:
  1. 规格内含**逐代能力矩阵**（G1 / G2 / G3 × 打开 / 查询 / 写入 / 完整性 / 打开时重建 / dump / 恢复），每格标注覆盖层级与证据来源；G3 的各格明确写为「当前构建原生路径」，不计入旧格式兼容证据。
  2. 每个 Requirement / Scenario 的**每个子句**都能指认证据来源（用例断言 / 既有测试文件 + 用例名 / 本 change 已记录的实际运行结果）；三者皆无的子句改写为可指认的表述或删除。
  3. R5 的三条迁移路径在 `/tmp` 内**各实际执行一遍**（真实 CLI：编辑 dump 文本 → `restore` → 记录退出码与结果），按实测结果确定规格中「可执行」的表述。
  4. DFH ADDED 保留「格式版本是产品格式契约、不引入身份机制」这一新增主张；与既有域重复的两个 Scenario 改为引用既有域表述，不作为本 change 的新主张重复声明。
- Preserve: 规格域只写验收过的行为，不用「应该 / 大概」式表述；`legacy-format-compat` 仍是新域（不并入既有域）；增量文件的 Purpose 与既有 75 域不冲突。
- Forbidden: 改 `openspec/specs/`（合并由 Maintainer 执行）；改 `src/` 或 `tests/`；改写 Iteration 000 的 Plan Context；把 F3 / F4 / F5 当成阻塞项；新增需求域。
- First test: 本 task 不含代码变更（文档分支，公共规则「轻量工作」）。先行产物 = **逐条对账表**：先读 `tests/legacy_compat_test.rs` 全文与 `tests/file_header_test.rs` 的相关用例名，再在增量文件内逐 Scenario 标注证据来源；**任何标不出证据来源的子句即为待改项**（当前已知待改项：F3 的覆盖层级、S1 的「可继续写入」、S2 的两条迁移路径、F4 的重复声明、F5 的如实标注）。
- GREEN condition: ① 矩阵在规格内且每格有覆盖层级标注；② 对账表中每个子句都有证据来源，无「待改项」残留；③ 三条迁移路径各有一次真实执行记录（命令 + 退出码 + 结果）；④ DFH ADDED 的重复声明已改写为引用；⑤ requirement / scenario 计数与对账表一致（若增删 Scenario，同步 `tasks.md` 的 RTM 与本 Cycle 的计数）。
- Verification: 人工逐条对账（最短步骤：读用例全文 → 逐子句标注 → 复述每格的证据；每步直接可观察结果 = 对账表无「待改项」残留）；迁移路径的真实执行结果记入 Act Response。失败含义：存在无证据来源的子句，或矩阵缺格 / 缺覆盖层级。
- Stop when: 某条迁移路径实测不可执行（→ 按实测改写规格表述并记为 Issue 候选或改进项，不扩范围）；需要改产品代码才能让某子句成立（→ 返回 Plan）。

### T3: README「存量兼容」段对齐（中英双语）

- Requirement/Scenario: `legacy-format-compat` R1–R5；`database-file-format-header` ADDED R1（对外可查的落点）
- Depends on: T2（措辞以 T2 定稿的规格文本为准）
- Targets: `README.md:277`（`### Constraints` 下的 `- **Legacy compatibility:**`）、`README.zh-CN.md:277`（`### 约束` 下的 `- **存量兼容：**`）
- Current behavior: 两条目只写「范围外的负面边界 + 迁移提示」，未写明支持范围（旧库格式代际、旧 dump 支持范围、范围内各自能做什么），未写明「写入使被写到的表目录行升级为当前布局」，未写明「格式版本是产品格式契约（唯一版本 v1；不做旧格式嗅探与迁移）」。
- Required behavior: 两条目与 T2 定稿的规格逐条一致，且包含：① 支持范围（代际定义与「按目录行布局代际识别」）；② 范围内能做什么（打开 / 查询 / 写入 / 完整性 / 打开时重建 / dump / 恢复，按矩阵可写明的部分）；③ 写入后的可见后果（被写到的表目录行升级为当前布局）；④ 范围外三类输入的拒绝形态（点名原因 + 失败语句序号 + 退出码）；⑤ **只写实测成立的**迁移路径；⑥ 格式版本契约（唯一版本 v1；旧格式兼容不靠版本号区分；高版本与无头文件明确拒绝）。
- Preserve: 只改这两个条目及其紧邻上下文；不新增章节；不动 README 其它章节；两条目语义等价（中英一致）。
- Forbidden: 改 `src/` / `tests/`；在 README 写规格之外的新承诺；删除既有的负面边界表述而不同步规格。
- First test: 文档分支，无先行测试。先行产物 = 与 T2 规格的**逐条对照清单**（README 段落 ↔ 规格 Requirement / Scenario ↔ 迁移路径实测结果）。
- GREEN condition: 两条目覆盖 Required behavior 的 ①–⑥；对照清单中每条规格要点都能在 README 文本中找到落点；中英两条目要点集合等价；README 的迁移路径与 T2 的实测记录一致。
- Verification: 人工逐条对照（最短步骤：读 T2 定稿规格 → 逐 Requirement / Scenario 在两条 README 条目中定位 → 复述落点；每步直接可观察结果 = 对照清单无空缺）。
- Stop when: README 需要承诺规格之外的语义才能说清（→ 返回 Plan）；迁移路径的实测结果与规格冲突（→ 返回 T2 修订后再来）。

### T4: 本 change 收尾全量回归（M19 分批串行）

- Requirement/Scenario: 收尾验证（`tasks.md`「收尾验证」段）；为 `legacy-format-compat` 全部 14 个 Scenario 提供回归护栏
- Depends on: T2、T3（回归覆盖最终文本树）
- Targets: `tests/*.rs`（当前 135 个文件）+ `--lib --bins`；本 task 只运行，不修改任何文件
- Current behavior: MS35 收尾全量 = 2183 passed / 0 failed / 2 ignored（134 个测试文件 + `--lib --bins` = 135 目标 / 14 批；来源：MS35 change `iterations/001-durable-release-gate/000-initial.md` T7 段）；当前树新增 `tests/legacy_compat_test.rs`（9 例）⇒ 预期 2192 / 0 / 2，136 目标 / 13 批。
- Required behavior: 逐条执行下列纪律并记录每批结果：
  1. `--lib --bins` 单独成批（批 1）。
  2. `tests/*.rs` 按**文件名字典序**分批，每批 **8–12** 个目标。
  3. 每批一次独立调用：`cargo test --offline --test <f1> --test <f2> ... -- --test-threads=1`。
  4. 批间**严格串行**：同一时刻只有一批在跑；上一批退出后才启动下一批。
  5. **禁止**一次无目标过滤的全量调用（例如 `cargo test --offline`）。
  6. **禁止**用默认并行度跑批（每批必须带 `-- --test-threads=1`）。
  7. 每批判定：进程退出码 = 0 **且** 该批 `test result:` 行的 failed 计数 = 0；两者任一不满足即为该批失败。
  8. 逐批记录 passed / failed / ignored 与目标清单。
  9. 覆盖对账：目标清单 = 收尾时 `tests/*.rs` 的全部文件 + `--lib --bins`；**missing 0 / extra 0 / duplicate 0**。
  10. 全量 Gate = 全部批次各自退出码 0 且 failed 计数 0；并与基线对账：合计 passed = 2192（= 2183 + 本 change 新增 9 例）、failed = 0、ignored = 2；差额必须逐项归因。

  批计划（当前 135 个文件；收尾时按实际文件清单重算，算法不变）：`B = ceil(135/12) = 12` 批文件（前 9 批各 11 个、后 3 批各 12 个 = 135），加 `--lib --bins` 一批 ⇒ **13 批 / 136 目标**；文件批次成员见「Risks and Notes」。

  判定层纪律：只看 cargo 的退出码与 `test result:` 行；计数可用现成的单步管道读出（管道只取数不判定）；**不新增**判定脚本、封装命令、专用参数或框架外判定层；不引入身份机制；不用 timer 阈值。
- Preserve: 不改 `src/` 与 `tests/`；不提交、不回滚既有现场；不并行跑批。
- Forbidden: 一次无目标过滤的全量调用；默认并行度跑批；用重复运行掩盖批失败（同一验证的重复仅限：计划写明的不稳定验证，或失败后判定是否偶发的一次复核，且须在记录中注明首跑失败）；为对账新建脚本。
- First test: 变更前 GREEN —— 本 change 的代码面自 Iteration 000 起未变，`tests/legacy_compat_test.rs` 的 9 例已 GREEN（沿用 Iteration 000 的 Act Response 结论：9 passed / 0 failed）；全量回归本身即收尾验证，无 RED 要求。
- GREEN condition: 13 批全部退出码 0 且 failed 计数 0；合计 2192 passed / 0 failed / 2 ignored；覆盖对账 missing 0 / extra 0 / duplicate 0；差额逐项归因（+9 = `tests/legacy_compat_test.rs` 的 9 例）。
- Verification: 每批 `cargo test` 的退出码与 `test result:` 行；计数用一步管道读出；失败含义 = 该批存在 failed 用例（保留该批，不重跑掩盖）。
- Stop when: 任一批失败（保留当前批次，不执行下游批次，按公共规则「三次失败」处理）；目标清单与实际文件清单不一致（先重算批计划）；出现需要改代码才能通过的失败（返回 Plan）。

**Invariants**

- `src/` 与 `tests/` 零改动；`git status --short -- src/` 保持 20 个 `M` + 1 个 `??`（MS34 / MS35 既有现场）。
- 兼容样本字节冻结（7 个样本文件）；`tests/fixtures/legacy/README.md` 的参考值 133 与 `tests/legacy_compat_test.rs` 的 `RECOVERED_ROWS_REFERENCE` 必须同步。
- 无身份证明机制（无 hash / 指纹 / revision pin / run-id / manifest / 时间顺序证明）；判定全部落在 `cargo test` 或真实 CLI 入口内；无框架外判定层；无 timer 阈值。
- 既有 75 域规格与 `openspec/specs/` 内容零改动（合并由 Maintainer 执行）。
- 本 Cycle 的写入面仅限：本 change 的 `specs/*.md`、`tasks.md`、`README.md`、`README.zh-CN.md`、本 Cycle 文件。

**Non-goals**

- 产品代码行为变更；用例增删改。
- `restore` 原子化 / 全量预检；`dump` 对非 `INT` `UNIQUE` 的渲染规则（D9）；旧库迁移工具；CI / 发布设施；I024 / I060。
- ISS22–ISS25（由后续承载 change 处理）。
- 把增量规格合并进 `openspec/specs/`、更新 SNAPSHOT / 全局 tasks / M-R-I（Maintainer 收尾职责）。
- 重新生成兼容样本。
- 处理 F4 的规格合并期去重（合并期由 Maintainer 按既有域对照处理；本 Cycle 只把 ADDED 内的重复声明改写为引用）。

**Acceptance**

| # | 验收条件 | requirement / scenario | design | task | 代码位置 | 测试 |
|---|---|---|---|---|---|---|
| A1 | 规格增量含逐代能力矩阵，每格有覆盖层级与证据来源；14 个 Scenario 的每个子句可指认证据来源 | LFC R1–R5；DFH ADDED R1 | D3 / D6 | T2 | `specs/legacy-format-compat/spec.md`、`specs/database-file-format-header/spec.md` | 逐条对账表（文本对账） |
| A2 | 三条迁移路径各有一次真实执行记录，规格中的「可执行」表述与实测一致 | LFC R5 的三个 Scenario | D6 | T2 | 同上 | 真实 CLI 执行记录（命令 + 退出码 + 结果） |
| A3 | 中英双语 README「存量兼容」条目与规格逐条一致（支持范围 / 范围内能力 / 写入后果 / 三类拒绝形态 / 已验证的迁移路径 / 版本契约） | LFC R1–R5；DFH ADDED R1 | D3 | T3 | `README.md:277`、`README.zh-CN.md:277` | 与规格的逐条对照清单 |
| A4 | 收尾全量按 M19 分批串行纪律通过：13 批 / 136 目标各自退出码 0 且 failed = 0；合计 2192 passed / 0 failed / 2 ignored；覆盖对账 missing 0 / extra 0 / duplicate 0 | 收尾验证；LFC 全部 Scenario 的回归护栏 | D10 | T4 | `tests/*.rs`（135）+ `--lib --bins` | 逐批 `cargo test` 的 `test result:` 行 |
| A5 | 产品代码与用例零改动；无身份机制；无框架外判定层 | design 不变量；AGENTS「行为约束」 | D6 / D10 | T2 / T3 / T4 | `src/`、`tests/` | `git status --short -- src/ tests/` |

**Verification**

| 验收 | 最简直接判定 | 通过依据 | 失败含义 |
|---|---|---|---|
| A1 | 逐 Scenario 复述对账表：每条子句 → 证据来源（用例名 + 断言位置 / 既有测试文件 + 用例名 / 已记录运行结果）；矩阵逐格复述覆盖层级 | 对账表无「待改项」残留；矩阵每格有层级标注 | 存在无证据子句，或矩阵缺格 / 缺覆盖层级 |
| A2 | 三条迁移路径的真实 CLI 执行记录（命令、退出码、结果）；规格对应段落的措辞与之一致 | 记录中的退出码与结果支持规格措辞 | 规格写了实测未成立的路径 |
| A3 | 逐 Requirement / Scenario 在两条 README 条目中定位落点；中英各复述一次要点集合 | 对照清单无空缺；两要点集合等价 | README 缺项或与规格冲突 |
| A4 | 逐批 `cargo test --offline --test <targets> -- --test-threads=1` 的退出码与 `test result:` 行；13 批 × 目标清单 | 全部 0 failed；合计 2192 / 0 / 2；对账 0 / 0 / 0 | 任一批有 failed |
| A5 | `git status --short -- src/ tests/`；`grep` 检索身份机制关键词与判定脚本 | `src/` / `tests/` 相对进入本 Cycle 时无新增改动 | 出现越界改动 |

**Gate 2 Readiness**

**PASS** —— 需求与范围已由 change 的 `proposal.md` / `design.md` / `tasks.md` 固定；实现事实已由 Iteration 000 的 accepted Review 与 Act Response 确认（9 个用例、7 个样本、README 参考值）；三个 Task Definition 均给出 Target / Current / Required / Preserve / Forbidden / First test / GREEN / Verification / Stop when；Verification 以真实入口为主，判定全部落在 `cargo test` 或真实 CLI 内；无阻塞未知项。

- BLOCKED：无。
- SKIPPED：无（Gate 2 检查项逐项通过）。
- 非实质未知项（留给 Act 的局部选择）：能力矩阵中 G2 各格的具体行级用例名（Act 在对账时逐格填出，格式由矩阵结构决定）；迁移路径实测所用的 `/tmp` 工作目录名与编辑后的 dump 文件名。

**Persisted Evidence**

- Mode: none

理由：三项验收（A1 / A2 / A3）的判定结果是对账表与文本对照，可由 Act Response 的表格承载；A4 的判定结果是逐批 `test result:` 行的计数与目标清单，同样可由 Act Response 承载（A2 的三条迁移路径执行记录只需命令、退出码与结果各一行）。这些结果都可低成本重跑或直接复述，不满足 `required` 的门槛（缺少它不会阻止任何决定）。

**Risks and Notes**

- F3 / F4 / F5 的处置已写死：F3 由 T2 的矩阵覆盖层级消解；F4 由 T2 把重复声明改写为引用既有域（合并期去重由 Maintainer 执行）；F5（`扫描不修改文件内容` 无字节级断言）属既有跨域行为，T2 对账时如实标注为「既有域已规定、无新增断言」，**不得**写成已验收的新断言。
- S1（`恢复出的库可继续写入`）与 S2（两条迁移路径未实测）是本 Cycle 的主要输入；T2 未处理完之前不得标记 A1 / A2 达成。
- 批计划（收尾时按实际 `tests/*.rs` 清单重算，算法为 `B = ceil(N/12)`、前 `B*12-N` 批各 11 个、其余各 12 个）：
  - 批 1：`--lib --bins`
  - 批 2：aggregate_extension_test / aggregate_output_shape_test / aggregate_test / alter_table_test / attach_session_test / btree_merge_test / btree_scale_test / btree_split_test / btree_test / busy_timeout_test / cache_perf_test
  - 批 3：catalog_durability_test / chain_join_test / check_command_test / checkpoint_command_test / checkpoint_redo_reduction_test / checkpoint_skip_test / checkpoint_test / cli_test / concurrent_test / connection_limit_test / constraint_enforcement_test
  - 批 4：cross_db_dml_test / cross_db_query_test / ctas_test / cte_test / database_file_lock_test / datetime_function_test / datetime_type_test / ddl_catalog_transactional_test / ddl_crash_recovery_test / default_declaration_test / default_precheck_test
  - 批 5：derived_scan_alias_test / dml_tx_id_test / drop_persistence_test / drop_table_free_test / e2e_test / encryption_test / executor_test / explain_test / explicit_tx_test / expression_e2e_test / file_header_test
  - 批 6：file_storage_io_test / free_page_lifecycle_test / full_table_dml_test / gc_test / group_by_expr_test / import_input_surface_test / index_manager_test / index_write_atomicity_test / insert_column_list_test / insert_select_test / isolation_level_test
  - 批 7：join_output_resolution_test / join_test / key_type_conformance_test / keyless_eq_routing_test / keyless_row_test / legacy_compat_test / limit_test / memory_db_test / mvcc_abort_test / mvcc_commit_test / mvcc_record_test
  - 批 8：mvcc_tombstone_visibility_test / nested_loop_join_test / network_protocol_test / network_server_test / no_from_select_test / order_by_alias_sort_test / outer_join_test / overflow_page_test / param_binding_test / parser_test / pg_integration_test
  - 批 9：pg_messages_test / pg_protocol_test / pipeline_test / plan_cache_test / plan_exec_test / planner_test / predicate_test / prefetch_test / prepared_statement_test / projection_expression_test / projection_test
  - 批 10：pushdown_test / query_memory_test / readonly_session_test / recovery_e2e_test / recovery_error_surface_test / recovery_replay_convergence_test / recovery_tail_persistence_test / recovery_test / recovery_wal_convergence_test / runtime_test / scalar_function_test
  - 批 11：scalar_subquery_comparison_test / schema_persistence_test / secondary_index_equality_test / secondary_index_test / select_distinct_test / self_join_test / set_operations_test / sort_test / sql_differential_test / statement_cancellation_test / storage_test / structural_property_test
  - 批 12：subquery_alias_binding_test / subquery_test / subset_insert_test / sync_loader_test / table_alias_resolution_test / table_manager_test / truncate_test / tx_statement_test / typed_api_test / update_expression_test / update_index_maintenance_test / upsert_test
  - 批 13：value_test / version_chain_test / view_test / visibility_test / wal_auto_checkpoint_test / wal_buffer_test / wal_handle_test / wal_integration_test / wal_record_test / wal_recovery_large_test / wal_writer_test / write_type_conformance_test
- 基线：MS35 收尾全量 2183 / 0 / 2（134 个测试文件 + `--lib --bins` = 135 目标 / 14 批）；本 change 预期 2192 / 0 / 2（135 个测试文件 + `--lib --bins` = 136 目标 / 13 批）；差额 +9 = `tests/legacy_compat_test.rs` 的 9 例。若收尾时实际文件清单变化，以实际清单重算并对差额逐项归因。
- 双语 README：`README.md` 与 `README.zh-CN.md` 各有一条对应条目，本 Cycle 两条同改；不新增章节（结构性双语对等调整不在本 Cycle 范围）。
- 规格增量文本的最终合并由 `openspec-docs-maintainer` 在 change 收尾时执行；本 Cycle 不触碰 `openspec/specs/`。

## Act Response

- Status: reported

### 任务完成清单

| 任务 | 状态 | 涉及文件与符号 |
|---|---|---|
| T2 规格增量定稿 | done | `openspec/changes/2026-10-24-ms36-legacy-format-and-dump-compat/specs/legacy-format-compat/spec.md`（Requirement 1 新增「逐代能力矩阵」+ E1–E11 证据索引；R1 S2、R2 S4、R4 S1、R5 三个 Scenario 的子句改写）；`.../specs/database-file-format-header/spec.md`（S2/S3 改写为引用既有域；S1 补既有域锚点） |
| T3 README 存量兼容段对齐 | done | `README.md:277`（`- **Legacy compatibility:**` 整条改写）；`README.zh-CN.md:277`（`- **存量兼容：**` 整条改写）。两文件各 1 行变更，无新增章节、无新增段落 |
| T4 收尾全量回归（M19 分批串行） | done | 只运行、不修改文件：`tests/*.rs`（135 个）+ `--lib --bins` = 136 目标 / 13 批 |

change 状态同步：`tasks.md` 的 T2/T3/T4 → `done`；RTM 六行 Status → `Done`（并补记实测证据来源）；Iteration Map 与「最终状态」段同步为「Act 已完成，Review Result 待 Plan 填写」。

### T2：逐代能力矩阵定稿结果

规格 `legacy-format-compat` Requirement 1 内新增「逐代能力矩阵」（3 代际 × 8 能力），每格标覆盖层级 + 证据键，证据索引 E1–E11 逐条给出用例名 / 断言位置：

| 代际 | 打开 | 查询 | 写入 | 完整性 | 打开时重建 | dump | 恢复 |
|---|---|---|---|---|---|---|---|
| G1（最旧） | 文件级样本 E1 | 文件级样本 E1 | 文件级样本 E2 | 文件级样本 E1 | 文件级样本 E3 | 文件级样本 E4 | 文件级样本 E5 |
| G2 | 行级既有覆盖 E6 | 行级既有覆盖 E6 | 无 G2 专属断言 E7 | 无 G2 专属断言 E7 | 无 G2 专属断言 E7 | 无 G2 专属断言 E7 | 无 G2 专属断言 E7 |
| G3（当前） | 当前构建原生路径·非兼容证据 E8 | 同左 E8 | 同左 E8 | 同左 E8 | 同左 E8 | E9 | E10 |

- **G1**：文件级样本 = `tests/fixtures/legacy/` 的固定字节样本 + `tests/legacy_compat_test.rs` 的 9 个走真实入口的用例。dump 列（E4）另有一次真实 CLI 实测（当前构建导出 G1 旧库，退出码 0）；恢复列（E5）= 未决写入日志样本恢复出参考行数 133。
- **G2**：E6 = `src/storage/catalog.rs::legacy_layout_row_deserializes_with_empty_unique_roots`、`::legacy_row_without_kind_section_reads_as_table`、`::legacy_column_row_without_default_reads_none`、`src/storage/data/table_manager.rs::legacy_row_with_int_unique_flag_opens_without_unique_index`。**E7 如实标注**：仓库内没有按 G2 形态（唯一根段存在、二级索引段与表类型字节缺失）构造的用例，也没有 G2 布局的文件级样本；G2 与 G1 只差「唯一根段」这一当前布局的原生段，其余行为路径与 G1 相同，由 G1 文件级样本覆盖。
- **G3**：明确写为「当前构建原生路径 · 非兼容证据」，不计入旧格式兼容证据。

**逐条对账表**（14 个 Scenario × 每个子句 → 证据来源；无「待改项」残留）：

| Scenario | 子句 | 证据来源 | 处置 |
|---|---|---|---|
| R1 S1 最旧代际库文件属于支持范围 | 目录行缺全部可选尾段、按缺省语义解析 | `legacy_generation_db_is_in_supported_range`（`__tables` / `__columns` 行长度 == G1 固定段长度断言） | 已覆盖 |
| R1 S1 | 判定不依赖运行时写入的文件版本标记 | 同上（判定只读目录行字节）+ `tests/fixtures/legacy/README.md`（样本是固定字节输入） | 已覆盖 |
| R1 S2 高版本拒绝 | 按既有分类拒绝（`NewerFileVersion`）+ 消息 | 既有域 `database-file-format-header` + `tests/file_header_test.rs::test_newer_version_rejected` | 引用既有域 |
| R1 S2 | 拒绝早于页解析 / 伴生文件打开 / 恢复 | 既有域「格式错误显式拒绝」+「打开顺序守卫」 | **改写为引用**（本域不新增断言） |
| R1 S3 无头文件拒绝 | `NotADatabase`，不嗅探不迁移 | 既有域 + `tests/file_header_test.rs::test_headerless_legacy_file_rejected` | 引用既有域 |
| R2 S1 打开并查询 | 打开成功、查询返回全部已提交行 | `legacy_db_opens_queries_and_writes`（3 行 alice/bob/carol 逐值断言） | 已覆盖 |
| R2 S1 | 打开过程无降级 / 放弃 / 迁移提示 | `legacy_db_open_rebuild_keeps_live_table_intact`（真实 CLI stderr 无「重建放弃」） | 已覆盖 |
| R2 S2 写入并读回 | 新行与既有行都可查、数量内容一致 | `legacy_db_opens_queries_and_writes`（重开 4 行、第 4 行 dave） | 已覆盖 |
| R2 S2 | 既有已提交数据未丢失 | 同上 + `legacy_db_write_upgrades_touched_table_row_to_current_layout`（重开后 id 1..=123 全部可读） | 已覆盖 |
| R2 S3 写入升级目录行 | 被写到的表目录行重写为当前布局 | `legacy_db_write_upgrades_touched_table_row_to_current_layout`（行尾 9 字节 == `00×8 01`） | 已覆盖 |
| R2 S3 | 未被写到的表保持原代际、库仍可开可查 | 同上（audit/events 行长度 == G1 固定段长度；重开后两表仍可查） | 已覆盖 |
| R2 S4 完整性零 error | 扫描完成、error 级发现数 0 | 三个用例的 `check_integrity(...).error_count() == 0` | 已覆盖 |
| R2 S4 | **扫描不修改文件内容（F5）** | 既有域 `data-integrity-verification`「校验面与既有边界协调」+ `tests/check_command_test.rs::test_check_clean_db_single_line_file_unchanged`（校验前后库文件全字节不变） | **改写**：由既有域规定，并注明本域对旧代际库未新增字节级断言 |
| R3 S1 打开时重建 | 打开成功、无「重建放弃」提示 | `legacy_db_open_rebuild_keeps_live_table_intact`（真实 CLI stderr 不含 `rebuild abandoned`） | 已覆盖 |
| R3 S1 | 活表行完整、其页不进入可分配页集合 | 同上（users 3 行 / events 1 行 / `discarded` 仍不存在）+ 既有断言 `tests/free_page_lifecycle_test.rs::rebuild_never_marks_live_pages_free` | 已覆盖（既有断言） |
| R3 S1 | 完整性 0 error | 同上 | 已覆盖 |
| R3 S2 未决记录恢复 | 恢复出已提交行 | `legacy_db_with_pending_wal_recovers_committed_rows`（恢复行数 == 参考值 133，门槛有判别力） | 已覆盖 |
| R3 S2 | 恢复后可查询、完整性 0 error | 同上 | 已覆盖 |
| R4 S1 旧 dump 恢复 | 恢复成功（退出码 0）、表与行等价 | `legacy_dump_restores_into_current_db`（users/audit/events 行集逐行断言） | 已覆盖 |
| R4 S1 | **恢复出的库使用当前格式、可继续写入（S1）** | 「可再次导出且等价」由该用例断言（dump → restore → dump 逐字节相等）；「使用当前格式」与「**可继续写入**」**无用例断言** → 本 change 真实 CLI 实测记录（见下） | **以实测补齐** |
| R4 S2 DDL 渲染差异 | 恢复成功、各类型列值等价 | `legacy_dump_restores_into_current_db`（DATE / TIMESTAMP / BOOL 逐值断言） | 已覆盖 |
| R5 S1 非整数主键 | 失败、点名原因 + 语句序号 | `legacy_dump_with_non_integer_primary_key_is_rejected`（`NON-INTEGER PRIMARY KEY`、`statement 1 of 2 failed`、`statement: CREATE TABLE`、退出码 3） | 已覆盖 |
| R5 S1 | 迁移路径 | 规划期只测过一次 → 本 change 实测 path 1 | **按实测改写措辞** |
| R5 S2 非整数 UNIQUE | 失败、点名原因 + 语句序号 | `legacy_dump_with_non_integer_unique_is_rejected`（`UNIQUE (INT columns only)`、`statement 1 of 2 failed`、退出码 3） | 已覆盖 |
| R5 S2 | 迁移路径（**S2：从未执行**） | 本 change 实测 path 2a / 2b / 2c | **按实测改写措辞**（2b 不成立） |
| R5 S3 NOT NULL 含 NULL | 失败、点名违例 + 列名 + 语句序号 | `legacy_dump_with_null_in_not_null_is_rejected`（`NOT NULL constraint violation`、`column 's'`、`statement 3 of 3 failed`、退出码 3） | 已覆盖 |
| R5 S3 | 迁移路径（**S2：从未执行**） | 本 change 实测 path 3a / 3b | 按实测改写（两条均成立） |
| DFH S1 唯一的产品格式版本 | 版本为 1 | `tests/file_header_test.rs::test_new_database_starts_with_header_and_reopens`（断言头版本字段 == `FORMAT_VERSION`） | 已覆盖 |
| DFH S1 | 只声明布局、不承担运行身份语义 | 该 Requirement 自身即主张；产品运行期无版本比对 / 指纹 / 修订固定 / 运行标识 | 已覆盖（产品行为） |
| DFH S1 | 不声明更旧版本、不按版本号嗅探迁移 | `test_version_zero_rejected`（version=0 → `NotADatabase`）+ `test_headerless_legacy_file_rejected`（不做嗅探与迁移） | 已覆盖 |
| DFH S2 高版本拒绝 | —— | 与既有域「格式错误显式拒绝 / 文件由新版创建」重复 | **改写为引用既有域**（F4 闭合） |
| DFH S3 无头文件拒绝 | —— | 与既有域「旧无头文件统一拒绝」重复 | **改写为引用既有域**（F4 闭合） |

**R5 三条迁移路径逐条实测**（真实 CLI `target/debug/rtsql`，输入为 `tests/fixtures/legacy/` 的旧 dump 文本副本，全部在 `/tmp/ms36-migration-paths/` 内进行，仓库样本只读未改）：

> **命令列的工作目录与路径形式（2026-10-25 补记）**：命令列中的 `./` 路径相对**工作目录** `/tmp/ms36-migration-paths/`；`rtsql` 指当前构建的 CLI（仓库内 `target/debug/rtsql`）；仓库样本用**绝对路径**引用（`/home/daivy/projects/RTsql/tests/fixtures/legacy/…`），工作目录内的输入是该样本的副本。**裸名形式不可用**：CLI 只把**含 `/`** 的参数按路径直开，裸名一律解析到 `$RTSQL_HOME/db/<name>.db`（未设 `RTSQL_HOME` 时为 `$HOME/.rtsql/db/<name>.db`）——按裸名字面执行会得到 `does not exist` 并以退出码 1 结束。故命令列一律用路径形式。

| 路径 | 编辑 | 命令 | 退出码 | 结果 | 结论 |
|---|---|---|---|---|---|
| 1 非整数 `PRIMARY KEY` | `sed 's/ PRIMARY KEY//'` | `rtsql new ./p1.db` → `rtsql restore ./p1.db ./p1_edited.sql` | 0 / **0** | `SELECT * FROM nipk;` → `{"columns":["id","v"],"rows":[["k1","x"]]}`；再 `dump` → `CREATE TABLE "nipk" ("id" TEXT, "v" TEXT);`（当前格式、无 `PRIMARY KEY`） | **成立** |
| 2a 非整数 `UNIQUE`（去声明） | `sed 's/ UNIQUE//'` | `rtsql new ./p2a.db` → `rtsql restore ./p2a.db ./p2a.sql` | 0 / **0** | `SELECT * FROM niuq;` → `{"columns":["id","email"],"rows":[[1,"a@b.c"]]}` | **成立** |
| 2b 非整数 `UNIQUE`（只改列类型） | `sed 's/"email" STRING UNIQUE/"email" INT UNIQUE/'` | `rtsql restore ./p2b.db ./p2b.sql` | **3** | `{"error":"statement 2 of 2 failed: Execution error: key column 'email' expects INT, got String; statement: INSERT INTO \"niuq\" VALUES (1, 'a@b.c'); previous statement(s) were committed"}` | **不成立**（只改列类型不足） |
| 2c 非整数 `UNIQUE`（改列类型 + 转换行值） | `sed -e 's/"email" STRING UNIQUE/"email" INT UNIQUE/' -e "s/VALUES (1, 'a@b.c')/VALUES (1, 42)/"` | `rtsql restore ./p2c.db ./p2c.sql` | **0** | `{"columns":["id","email"],"rows":[[1,42]]}` | **成立** |
| 3a `NOT NULL` 含 NULL（回填） | `sed "s/VALUES (2, NULL)/VALUES (2, 'backfilled')/"` | `rtsql restore ./p3a.db ./p3a.sql` | **0** | `{"columns":["id","s"],"rows":[[1,"ok"],[2,"backfilled"]]}` | **成立** |
| 3b `NOT NULL` 含 NULL（清理该行） | `grep -v "VALUES (2, NULL)"` | `rtsql restore ./p3b.db ./p3b.sql` | **0** | `{"columns":["id","s"],"rows":[[1,"ok"]]}` | **成立** |

**S1「恢复出的库可继续写入」与另两处子句的实测补齐**（真实入口、同一纪律；路径形式同上）：

| 子句 | 命令 | 退出码 | 结果 |
|---|---|---|---|
| R4 S1 恢复出的库可继续写入 | `rtsql new ./s1.db` → `rtsql restore ./s1.db ./legacy_dump.sql` → `rtsql ./s1.db "INSERT INTO users VALUES (4, 'dave', 6.0);"` → `rtsql ./s1.db "SELECT * FROM users;"` → `rtsql ./s1.db "INSERT INTO audit VALUES (2, 'after-restore');"` → `rtsql ./s1.db "SELECT COUNT(*) FROM audit;"` | 0 / **0** / **0** / **0** / **0** / **0** | `{"affected_rows":1}`；users 4 行（alice/bob/carol/dave）；`{"columns":["count_star"],"rows":[[2]]}`；`dump` 渲染 `CREATE TABLE "users" ("id" INTEGER PRIMARY KEY, "name" TEXT NOT NULL, "score" FLOAT);`（当前格式长类型名） |
| E4 当前构建导出 G1 旧库 | `cp /home/daivy/projects/RTsql/tests/fixtures/legacy/legacy_g1.db ./g1_dump_probe.db` → `rtsql dump ./g1_dump_probe.db` → `rtsql ./g1_dump_probe.db "SELECT COUNT(*) FROM users;"` | **0** / 0 | stderr 空；导出 3 表 + 全部行（长类型名渲染）；`{"columns":["count_star"],"rows":[[3]]}` |
| E2 写入后仍能再次写入 | 同一 G1 旧库 `./g1_dump_probe.db`：`rtsql ./g1_dump_probe.db "INSERT INTO users VALUES (4,'dave',6.0);"` → 再 `rtsql ./g1_dump_probe.db "INSERT INTO users VALUES (5,'erin',5.5);"` → `rtsql ./g1_dump_probe.db "SELECT COUNT(*) FROM users;"` → `rtsql ./g1_dump_probe.db "INSERT INTO audit VALUES (2,'second-round');"` → `rtsql ./g1_dump_probe.db "SELECT * FROM audit;"` | **0** / **0** / 0 / **0** / 0 | `{"affected_rows":1}` ×2；users 5 行；audit 2 行 |

**规格措辞改动前后对照（按实测改写）**：

| 位置 | 改动前 | 改动后 | 依据 |
|---|---|---|---|
| LFC R2 S4（`AND` 子句） | 「扫描不修改文件内容」 | 「该扫描的只读性质（不修改库文件内容）由既有域 `data-integrity-verification` 的『校验面与既有边界协调』规定（既有用例 `tests/check_command_test.rs::test_check_clean_db_single_line_file_unchanged` 断言校验前后库文件全字节不变）；本域对旧代际库**未新增**字节级断言」 | F5：无旧代际库的字节级断言，不得写成已验收的新断言 |
| LFC R4 S1（`AND` 子句） | 「恢复出的库使用当前格式，可继续写入与再次导出」 | 「恢复出的库使用当前格式（导出按当前格式渲染长类型名），可继续写入，并可再次导出且导出内容与恢复出的库等价」 | S1：补入「使用当前格式」的可观察形态与「导出内容等价」的断言形态 |
| LFC R5 S1（`AND` 子句） | 「迁移路径有文档：按当前导出的渲染规则去掉 `PRIMARY KEY` 声明后恢复（当前导出对非整数主键表本就不渲染主键，往返可达）」 | 「迁移路径：去掉 dump 中的 `PRIMARY KEY` 声明后恢复——该形态与当前导出的渲染一致（当前导出对非整数主键表不渲染 `PRIMARY KEY`，见 `tests/cli_test.rs::test_dump_pk_rendering_matches_enforced_semantics`），恢复成功且行数据等价」 | path 1 实测成立；补上既有断言锚点 |
| LFC R5 S2（`AND` 子句） | 「迁移路径有文档：把被约束列改为整数类型或去掉 `UNIQUE` 声明、重建该表后再恢复」 | 「迁移路径：去掉 `UNIQUE` 声明后恢复；或把被约束列改为整数类型**并同步转换该列的行值**后恢复——只改列类型而不转换行值仍被键列类型门拒绝（`key column '<列名>' expects INT, got String`，退出码 3）」 | path 2a/2c 成立、**2b 不成立** → 原措辞不可执行 |
| LFC R5 S3（`AND` 子句） | 「迁移路径有文档：清理或回填该行后再恢复」 | 「迁移路径：回填该行的 NULL 后恢复，或清理（删除）该行后恢复」 | path 3a/3b 实测成立 |
| DFH S2 / S3 | 各自独立声明高版本 / 无头文件拒绝 | 「（引用既有域）」并指明既有域 requirement / scenario 名与既有用例名 | F4：与既有域重复，不作为本 change 的新主张 |
| DFH S1（`THEN` / `AND`） | 无证据锚点 | 补 `test_new_database_starts_with_header_and_reopens` / `test_version_zero_rejected` / `test_headerless_legacy_file_rejected` 锚点 | 逐子句可指认 |

计数核对：LFC **5 requirement / 14 scenario**、DFH **1 requirement / 3 scenario**，与对账表一致（未增删 Scenario，`tasks.md` 的 RTM 计数无需变更）。

### T3：中英双语存量兼容条目对齐前后对照

对齐后的要点集合（中英等价，各覆盖 Required behavior ①–⑥）：① 支持范围按**系统表目录行的布局代际**（G1 最旧 / G3 当前）划分，不按文件格式版本；② 范围内能做：打开 / 查询 / 写入 / 完整性扫描 / 打开时重建 / dump、带未决写入日志的恢复、旧 dump 恢复到当前库（行数据等价、恢复出的库用当前格式、可继续写入、可再次导出）；③ 写入后果：**被写到的**表目录行升级为当前布局，未被写到的表保持原代际；④ 范围外三类拒绝形态（点名原因 + 失败语句序号 + 退出码 3）；⑤ 只写实测成立的迁移路径；⑥ 版本契约（唯一版本 `v1`、旧布局按代际而非版本号区分、高版本 `NewerFileVersion` 拒绝、无文件头 `NotADatabase` 拒绝、不做格式嗅探与猜测性迁移）。

**英文（`README.md:277`）改动前 → 改动后**：

- 改动前：`- **Legacy compatibility:** databases created by older versions that contain non-`INT` `UNIQUE` columns open normally (those columns are not enforced); dumping and restoring such a table into a new database is rejected by the plan-time rules above. Legacy tables declared with a non-integer `PRIMARY KEY` also open, read, and dump normally, and the dump DDL renders without `PRIMARY KEY` so `dump` → `restore` round-trips them; dumps produced by older versions that embed a non-integer `... PRIMARY KEY` DDL fail restore on a new database (the declaration was never enforced). Rows written before `NOT NULL` enforcement that contain `NULL` in a `NOT NULL` column also fail on restore. Re-create the schema (moving non-`INT` `UNIQUE` columns to `INT` or dropping the declaration) and clean or backfill such rows before restoring.`
- 改动后（现文）：`- **Legacy compatibility:** the supported range of old databases and old dumps is defined by **the catalog-row layout generation** (`G1` = oldest: `__tables` rows end at the data-chain tail pointer; `G3` = current: the rows also carry the unique-roots, secondary-index and table-kind sections), not by the file format version — the version is a single product format contract (`v1`) that only declares the layout the product writes. Inside the range a legacy database opens, queries, takes writes, passes an integrity scan, rebuilds on open and can be dumped; a legacy database with a pending write log recovers its committed rows; an old dump restores into a new database with equivalent rows, and the restored database uses the current format, keeps accepting writes and can be dumped again. A write has one visible consequence: the catalog row of the **written** table is rewritten to the current layout, while tables that were not written keep their own generation. Databases created by older versions that contain non-`INT` `UNIQUE` columns open normally (those columns are not enforced), and legacy tables declared with a non-integer `PRIMARY KEY` also open, read and dump normally — their dump DDL renders without `PRIMARY KEY`, so `dump` → `restore` round-trips them. Outside the range the input is refused explicitly and located to the statement: a non-integer `PRIMARY KEY` (`NON-INTEGER PRIMARY KEY`), a `UNIQUE` on a non-`INT` column (`UNIQUE (INT columns only)` — the plan-time rules above refuse that table itself) and a `NULL` in a `NOT NULL` column (`NOT NULL constraint violation: column '<name>'`) each fail the restore with exit code 3, naming the cause and the failing statement number — a dump produced by an older version that embeds a non-integer `... PRIMARY KEY` DDL fails for that reason, and rows written before `NOT NULL` enforcement that hold `NULL` in a `NOT NULL` column fail for that reason too. Migration paths: drop the `PRIMARY KEY` declaration from the dump and restore again; drop the `UNIQUE` declaration and restore again, or change the constrained column to an integer type **and** convert that column's row values at the same time; backfill the `NULL` and restore again, or delete that row and restore again. A file whose format version is newer than the supported one is refused (`NewerFileVersion`, exit code 1) and a file without a file header is refused (`NotADatabase`, exit code 1) — the product does not sniff old formats and does not guess a migration.`

**中文（`README.zh-CN.md:277`）改动前 → 改动后**：

- 改动前：`- **存量兼容：** 旧版本创建的、含非 `INT` `UNIQUE` 列的数据库可正常打开（这些列不被强制）；但把这类表 dump 后 restore 进新库会被上述计划期规则拒绝。 以非整数 `PRIMARY KEY` 声明的存量表同样可正常打开、读取与 dump，且 dump DDL 不渲染 `PRIMARY KEY`（`dump` → `restore` 往返可达）；旧版本生成的、内嵌非整数 `... PRIMARY KEY` DDL 的 dump 在新库 restore 失败（该声明从未被强制）。在 `NOT NULL` 强制落地前写入的、「NOT NULL 列含 NULL」的存量行同样会在 restore 时被拒。请先调整 schema（把非 `INT` `UNIQUE` 改为 `INT` 或去掉声明）并清理或回填此类数据，再执行 restore。`
- 改动后（现文）：`- **存量兼容：** 旧库与旧 dump 的支持范围按**系统表目录行的布局代际**划分（`G1` 最旧：`__tables` 行止于数据链尾指针；`G3` 当前：行上还带唯一根段、二级索引段与表类型字节），**不按文件格式版本划分**——版本号是产品格式契约，唯一版本 `v1`，只声明产品写出的布局。范围内：旧库可打开、可查询、可写入、可通过完整性扫描、打开时可重建、可导出 dump；带未决写入日志的旧库能恢复出已提交行；旧 dump 可恢复到新库且行数据等价，恢复出的库使用当前格式、可继续写入、可再次导出。写入有一处可见后果：**被写到的**表的目录行被重写为当前布局，未被写到的表保持各自原有代际。旧版本创建的、含非 `INT` `UNIQUE` 列的数据库可正常打开（这些列不被强制）；以非整数 `PRIMARY KEY` 声明的存量表同样可正常打开、读取与 dump，且 dump DDL 不渲染 `PRIMARY KEY`（`dump` → `restore` 往返可达）。范围外：输入被明确拒绝并定位到语句——非整数 `PRIMARY KEY`（`NON-INTEGER PRIMARY KEY`）、非 `INT` 列的 `UNIQUE`（`UNIQUE (INT columns only)`——表本身按上述计划期规则拒绝）、`NOT NULL` 列含 `NULL`（`NOT NULL constraint violation: column '<列名>'`）三类都以退出码 3 结束 restore，点名拒绝原因并给出失败语句序号；旧版本生成的、内嵌非整数 `... PRIMARY KEY` DDL 的 dump 因前者失败（该声明从未被强制），在 `NOT NULL` 强制落地前写入的、「NOT NULL 列含 NULL」的存量行因后者失败。迁移路径：去掉 dump 中的 `PRIMARY KEY` 声明后重新 restore；去掉 `UNIQUE` 声明后重新 restore，或把被约束列改为整数类型**并同步转换该列的行值**后重新 restore；回填该行的 `NULL` 后重新 restore，或删除该行后重新 restore。格式版本高于当前支持版本的文件被拒绝（`NewerFileVersion`，退出码 1），不含文件头的文件被拒绝（`NotADatabase`，退出码 1）——产品不做旧格式嗅探，也不做猜测性迁移。`

对照要点：既有的负面边界表述全部保留（非 `INT` `UNIQUE` 列不被强制且表本身按计划期规则拒绝、非整数主键存量表可开可读可 dump 且 DDL 不渲染 `PRIMARY KEY`、旧 dump 内嵌非整数主键 DDL 恢复失败、NOT NULL 含 NULL 的存量行恢复失败），并把原有的迁移提示按实测改精确（「改为整数类型**并同步转换该列的行值**」）。两文件各只改 1 行（`git diff --stat` = 2 files changed, 2 insertions(+), 2 deletions(-)），未新增章节或段落，中英要点集合逐条等价。

### T4：收尾全量回归分批执行记录（M19 分批串行）

批计划：目标清单 = 收尾时 `ls tests/*.rs | sort` 的 **135** 个文件 + `--lib --bins` = **136 目标**；`--lib --bins` 单独成批；文件批按文件名字典序切分，每批 8–12 个目标；每批一次独立调用 `cargo test --offline --test <f1> ... -- --test-threads=1`；批间严格串行（同一时刻只有一批在跑）；判定只看 cargo 退出码与 `test result:` 行的 failed 计数；计数用一步管道 `grep -E "^test result: ok|^test result: FAILED"` 读出（只取数不判定），cargo 退出码用 `PIPESTATUS[0]` 读取。未使用任何无目标过滤的全量调用，未使用默认并行度，未新增判定脚本 / 封装命令 / 专用参数。

| 批 | 目标 | 退出码 | passed | failed | ignored |
|---|---|---|---|---|---|
| 1 | `--lib --bins`（lib 420 + bins 0） | 0 | 420 | 0 | 0 |
| 2 | aggregate_extension_test, aggregate_output_shape_test, aggregate_test, alter_table_test, attach_session_test, btree_merge_test, btree_scale_test, btree_split_test, btree_test, busy_timeout_test, cache_perf_test | 0 | 149 | 0 | 0 |
| 3 | catalog_durability_test, chain_join_test, check_command_test, checkpoint_command_test, checkpoint_redo_reduction_test, checkpoint_skip_test, checkpoint_test, cli_test, concurrent_test, connection_limit_test, constraint_enforcement_test | 0 | 210 | 0 | 2 |
| 4 | cross_db_dml_test, cross_db_query_test, ctas_test, cte_test, database_file_lock_test, datetime_function_test, datetime_type_test, ddl_catalog_transactional_test, ddl_crash_recovery_test, default_declaration_test, default_precheck_test | 0 | 140 | 0 | 0 |
| 5 | derived_scan_alias_test, dml_tx_id_test, drop_persistence_test, drop_table_free_test, e2e_test, encryption_test, executor_test, explain_test, explicit_tx_test, expression_e2e_test, file_header_test | 0 | 191 | 0 | 0 |
| 6 | file_storage_io_test, free_page_lifecycle_test, full_table_dml_test, gc_test, group_by_expr_test, import_input_surface_test, index_manager_test, index_write_atomicity_test, insert_column_list_test, insert_select_test, isolation_level_test | 0 | 88 | 0 | 0 |
| 7 | join_output_resolution_test, join_test, key_type_conformance_test, keyless_eq_routing_test, keyless_row_test, legacy_compat_test, limit_test, memory_db_test, mvcc_abort_test, mvcc_commit_test, mvcc_record_test | 0 | 95 | 0 | 0 |
| 8 | mvcc_tombstone_visibility_test, nested_loop_join_test, network_protocol_test, network_server_test, no_from_select_test, order_by_alias_sort_test, outer_join_test, overflow_page_test, param_binding_test, parser_test, pg_integration_test | 0 | 124 | 0 | 0 |
| 9 | pg_messages_test, pg_protocol_test, pipeline_test, plan_cache_test, plan_exec_test, planner_test, predicate_test, prefetch_test, prepared_statement_test, projection_expression_test, projection_test | 0 | 217 | 0 | 0 |
| 10 | pushdown_test, query_memory_test, readonly_session_test, recovery_e2e_test, recovery_error_surface_test, recovery_replay_convergence_test, recovery_tail_persistence_test, recovery_test, recovery_wal_convergence_test, runtime_test, scalar_function_test, scalar_subquery_comparison_test | 0 | 118 | 0 | 0 |
| 11 | schema_persistence_test, secondary_index_equality_test, secondary_index_test, select_distinct_test, self_join_test, set_operations_test, sort_test, sql_differential_test, statement_cancellation_test, storage_test, structural_property_test, subquery_alias_binding_test | 0 | 154 | 0 | 0 |
| 12 | subquery_test, subset_insert_test, sync_loader_test, table_alias_resolution_test, table_manager_test, truncate_test, tx_statement_test, typed_api_test, update_expression_test, update_index_maintenance_test, upsert_test, value_test | 0 | 200 | 0 | 0 |
| 13 | version_chain_test, view_test, visibility_test, wal_auto_checkpoint_test, wal_buffer_test, wal_handle_test, wal_integration_test, wal_record_test, wal_recovery_large_test, wal_writer_test, write_type_conformance_test | 0 | 86 | 0 | 0 |
| **合计** | **13 批 / 136 目标（135 个测试文件 + `--lib --bins`）** | **全部 0** | **2192** | **0** | **2** |

2 个 ignored 来自批 3 的 `cli_test.rs`（长期 `#[ignore]` 的校准 / 诊断用例），与本 change 无关。

**覆盖对账**（以实际 `tests/*.rs` 文件集合为基准，`comm` / `uniq` 一步通用管道）：

- 文件集合 135 = 批次内文件目标 135 = 批次内唯一文件目标 135
- missing（文件集合里有、任何批次里没有）= **0**
- extra（批次里有、文件集合里没有）= **0**
- duplicate（出现在一个以上批次）= **0**
- 加上单独成批的 `--lib --bins` ⇒ 目标 136 = 135 + 1，批次 13

**与基线的总数对账**：基线 = MS35 收尾全量 **2183 passed / 0 failed / 2 ignored**（14 批 / 135 目标，来源 MS35 change 的 T7 段）；本 change 终态 **2192 passed / 0 failed / 2 ignored**（13 批 / 136 目标）。差额 **+9 passed**，逐项归因 = 本 change 新增 `tests/legacy_compat_test.rs` 的 **9** 个用例（批 7 内 `legacy_compat_test` 目标显示 `9 passed`）；failed 增量 0、ignored 增量 0（2 个 ignored 仍是 `cli_test.rs` 的长期用例）。lib 目标 420 与基线一致。**无未归因差额**。

### Gate 3 / Gate 4 结论

| 任务 | Gate 3（Task Review） | Gate 4（Verified Completion） |
|---|---|---|
| T2 | **PASS**：矩阵在规格内、每格有覆盖层级 + 证据来源（E1–E11）；14 个 Scenario 的每个子句都能指认证据来源，对账表无「待改项」残留；三条迁移路径各有一次真实执行记录（命令 + 退出码 + 结果）；DFH ADDED 的两个重复 Scenario 已改写为引用；requirement / scenario 计数（5+1 / 14+3）与对账表一致。代码面零改动、无身份机制、无框架外判定层、无 timer 阈值 | **PASS**：① 矩阵在规格内且每格有覆盖层级标注 ✓；② 对账表无待改项残留 ✓；③ 三条迁移路径各有真实执行记录 ✓；④ DFH 重复声明已改写为引用 ✓；⑤ 计数一致 ✓ |
| T3 | **PASS**：两条目覆盖 Required behavior ①–⑥；逐条对照清单无空缺；中英要点集合逐条等价；README 的迁移路径与 T2 实测记录一致；diff 只含两个条目各 1 行，无新增章节 / 段落，未动其它章节 | **PASS**：①–⑥ 全覆盖、对照清单无空缺、中英等价、迁移路径与实测一致 ✓ |
| T4 | **PASS**：13 批各自退出码 0 且 failed = 0；批大小 11/11/11/11/11/11/11/11/12/12/12/12/11 均在 8–12；批间严格串行；未用无目标过滤的全量调用，未用默认并行度 | **PASS**：13 批 / 136 目标全部 0 failed；合计 2192 / 0 / 2 与预期完全一致；覆盖对账 missing 0 / extra 0 / duplicate 0；差额 +9 逐项归因 ✓ |

Gate 5：未触发（无阻塞项，无同一验证点连续失败）。

### 自检发现与处置

| # | 发现 | 处置 |
|---|---|---|
| 1 | R5 场景 2 的迁移路径草案不可执行：草案写「把被约束列改为整数类型**或**去掉 `UNIQUE` 声明」，实测只改列类型而不转换该列行值仍被键列类型门拒绝（退出码 3） | **已修复**：规格与 README 改为「去掉 `UNIQUE` 声明」或「改为整数类型**并同步转换该列的行值**」两条，并写明 2b 的拒绝形态 |
| 2 | R4 场景 1 的「恢复出的库……可继续写入」与 R2 requirement 的「写入后仍能……写入」两处子句在仓库内无用例断言（S1）；Plan 又禁止改 `tests/` | **已修复**：以真实 CLI 实测补齐（restore → INSERT → SELECT；G1 旧库连续两次写入），写入规格证据索引 E2 / E4 与 `tasks.md` 的 RTM |
| 3 | Plan Context 的矩阵模板在 G2 的写入 / 完整性 / 打开时重建 / dump / 恢复五格标注「行级既有覆盖」，与仓库内实际证据不符（E6 的四个行级用例只覆盖目录行的兼容读解析，既无 G2 形态的用例，也无 G2 文件级样本） | **已修复**：五格改标「无 G2 专属断言」，并在 E7 说明 G2 与 G1 只差「唯一根段」这一原生段、其余路径由 G1 文件级样本覆盖 |
| 4 | F5（扫描不修改文件内容）若平铺直叙会被读成旧代际库已验收的新断言 | **已修复**：改写为引用既有域 `data-integrity-verification` 的既有 Requirement + 既有字节级用例，并显式声明本域未新增字节级断言 |

遗留 Minor 问题：**无**（无未解决的 Critical / Important；Plan Review 遗留的 F3 / F4 / F5 / S1 / S2 五项均已在 T2 闭合）。

已记录的**事实性缺口（非缺陷）**：G2 的五格无专属断言（见上）。补齐它需要新增 G2 布局的文件级样本与用例，属产品 / 用例改动，超出本 change「零 `src/`、零 `tests/` 改动」的边界，本 Cycle 不做。

### 验证命令与实际结果

| 目的 | 命令 | 实际结果 |
|---|---|---|
| 变更前 GREEN（沿用） | `cargo test --offline --test legacy_compat_test -- --test-threads=1`（Iteration 000 的 Act Response） | 9 passed / 0 failed；本 Cycle 批 7 复跑同为 9 passed |
| 变更前 GREEN（沿用） | `cargo test --offline --test file_header_test -- --test-threads=1`（Iteration 000） | 14 passed；本 Cycle 批 5 复跑同为 14 passed |
| 规格增量合法性 | `openspec validate 2026-10-24-ms36-legacy-format-and-dump-compat --strict` | `Change '2026-10-24-ms36-legacy-format-and-dump-compat' is valid`（退出码 0） |
| 迁移路径实测 | 见「R5 三条迁移路径逐条实测」表（path 1 / 2a / 2b / 2c / 3a / 3b） | 退出码 0 / 0 / **3** / 0 / 0 / 0（仅 2b 不成立） |
| S1 / E2 / E4 实测 | 见上两表 | 退出码全部 0，行集与预期一致 |
| 收尾全量 | 13 批 `cargo test --offline --test <targets> -- --test-threads=1` | 13 批退出码全 0，合计 2192 passed / 0 failed / 2 ignored |
| 覆盖对账 | `comm -23 / -13` + `uniq -d`（一步通用管道） | missing 0 / extra 0 / duplicate 0 |
| 代码面零改动 | `git status --short -- src/ tests/` | `src/` = 20 个 `M` + 1 个 `??`（`src/storage/free_page_rebuild.rs`），与进入本 Cycle 时一致；`tests/` 集合未变（本 Cycle 未新增 / 修改任何测试文件） |
| 样本字节冻结 | `stat -c '%s %n' tests/fixtures/legacy/*` | 41024 / 28736 / 15093 / 497 / 98 / 107 / 133，与 `tests/fixtures/legacy/README.md` 的记载逐一一致；`tests/legacy_compat_test.rs:601` 的 `RECOVERED_ROWS_REFERENCE = 133` 与 README 的 133 同步 |

### Deviations from Plan

1. **能力矩阵 G2 行五格改标「无 G2 专属断言」**（Plan Context 的 Implementation Guidance 模板在这五格写了「行级既有覆盖」）。实测：E6 的四个行级兼容读用例只覆盖目录行的兼容读解析（缺唯一根段 / 缺表类型字节 / 缺默认值段），没有任何用例按 G2 形态构造，也不覆盖写入 / 完整性 / 打开时重建 / dump / 恢复；仓库内也没有 G2 布局的文件级样本。按 AGENTS「不得把不成立的行为写成已验收」改标，并在 E7 说明 G2 与 G1 的差别与覆盖来源。批数 / 目标数 / 结构不受影响。
2. **T2 增做了 3 次真实 CLI 实测以补齐 Plan 未指派的子句**：S1（恢复出的库可继续写入）、E4（当前构建导出 G1 旧库）、E2（写入后仍能再次写入）。这三处子句在仓库内无用例断言，T2 的 Required behavior #2 要求每个子句都能指认「本 change 已记录的实际运行结果」，而本 change 禁止改 `tests/`（不能加用例），因此只能以真实 CLI 实测补齐。全部在 `/tmp/ms36-migration-paths/` 内进行，仓库样本只读未改。
3. **R5 场景 2 的迁移路径措辞按实测改写**（见自检发现 #1）：原草案的「改为整数类型」单独使用不可执行。
4. **批计划切点后移一位**：Plan Context 的逐批清单把第 100 个文件 `scalar_subquery_comparison_test` 放在批 11；执行时它已作为批 10 的第 12 个目标执行（批 10 因此为 12 个目标，仍在 8–12 内）。为避免该目标重复入批、且不重跑已通过的批 10，批 11 / 12 / 13 的切点整体后移一位（批 11 = 文件 101–112、批 12 = 文件 113–124、批 13 = 文件 125–135）。**批次总数（13）、目标总数（136）、批大小区间（8–12）、覆盖不重不漏均不受影响**；Plan 记录的文件集合与实际文件集合完全一致（135 个文件同名同序）。
5. **批 10 的调用前缀带了一条格式错误的 cargo 命令**（`cargo test --offline cargo test --offline`）：cargo 的参数解析直接以 `error: unexpected argument 'test' found` 拒绝，未构建、未运行任何测试（该行 stdout 为空；已用 `-- --list` 复核确认），批 10 本身由同一条 shell 行的第二个命令执行、结果不受影响。全程**没有**发生任何无目标过滤的全量调用。

### Persisted Evidence

**None required** —— 依 Plan Context 的 `Persisted Evidence: none` 理由：A1 / A2 / A3 的判定结果是对账表与文本对照，A4 的判定结果是逐批 `test result:` 行的计数与目标清单，均可由本 Response 的表格承载；三条迁移路径与三次补齐实测只需命令、退出码与结果各一行。这些结果都可低成本重跑或直接复述，缺少它们不会阻止任何决定。未创建 `evidence/` 目录。

### Experience Candidates

- **Runbook 候选（更新 R48 的切分算法）**：M19 分批全量回归配方 `.agents/runbooks/m19-batched-full-regression.md` 记的「剩余 ≤ 12 则取剩余，否则取 11」算法在文件数落到某些余数时产出超出 8–12 的末批；本 change 采用 `design.md` D10 的等分规则（`B = ceil(N/12)`，前 `B×12−N` 批各 11 个、其余各 12 个）并实测通过（135 个文件 → 13 批 / 136 目标 / 2192 passed / 0 failed / 2 ignored，覆盖对账 0/0/0）。证据：本 Response 的 T4 分批表与对账结果。建议由 `openspec-experience-recorder` 决定是否把 R48 的切分算法更新为等分规则（本 change 不改 Runbook）。
- **Issue 候选**：None —— 本 change 范围内未发现实质缺陷；范围外的 ISS22–ISS25 已在 `tasks.md` 记录，不重复登记。

### 未解决问题与停止条件

- 未解决项：无阻断项。唯一已记录的事实性缺口是 G2 的五格无专属断言（需新增样本与用例，属后续 change 的范围）。
- 停止条件：当前 Cycle 的 T2 / T3 / T4 全部完成并通过 Gate 3 / Gate 4；无待办任务，无下游授权需求。本 Response 不创建下一 Cycle / 下一 Iteration，不填写 `Plan Review`，不维护全局状态，不归档 change。

## Plan Review

- Review Result: accepted

### 独立检查结果

本轮 Review 只读当前 Cycle、change 产物、仓库代码与 diff；Act 已报告且覆盖范围未变化的验证结论沿用（本 Review 未重跑全量回归，符合 AGENTS「不重跑全量」）。

**变更面**：`git status --short -- src/` = 20 个 `M` + 1 个 `??`（`src/storage/free_page_rebuild.rs`），与 Cycle 不变量一致；`tests/` 的最近修改时间全部早于本 Cycle 的规格写入时间（`tests/legacy_compat_test.rs` 21:49、`tests/fixtures/legacy/*` 21:30–21:48，而规格 22:07、README 22:09、tasks 22:22），本 Cycle 未新增或修改任何 `src/` / `tests/` 文件。样本字节 `41024 / 28736 / 15093 / 497 / 98 / 107 / 133` 与 `tests/fixtures/legacy/README.md` 记载逐一一致，`tests/legacy_compat_test.rs:601` 的 `RECOVERED_ROWS_REFERENCE = 133` 同步。change 目录仅 7 个文件，无脚本、无 Evidence 目录（Persisted Evidence = none 成立）。

**T2（规格定稿与迁移路径）**：

- 逐代能力矩阵已入规格（`specs/legacy-format-compat/spec.md:21`–`:27`），3 代际 × 8 能力格每格带覆盖层级与证据键，证据索引 E1–E11 逐条给出可指认的用例名与断言位置。矩阵对 G3 明确写「当前构建原生路径 · 非兼容证据」，不计入旧格式兼容证据。
- 逐条对账表覆盖 14 个 Scenario 的每个子句，**抽查复核**引用的锚点全部存在：`tests/file_header_test.rs::{test_new_database_starts_with_header_and_reopens, test_version_zero_rejected, test_newer_version_rejected, test_headerless_legacy_file_rejected}`、`tests/check_command_test.rs::test_check_clean_db_single_line_file_unchanged`、`tests/cli_test.rs::test_dump_pk_rendering_matches_enforced_semantics`、`tests/recovery_test.rs::{test_recover_commit_abort_marks, test_recover_from_checkpoint}`、`tests/free_page_lifecycle_test.rs::rebuild_never_marks_live_pages_free`、`src/storage/catalog.rs::catalog_row_with_unique_roots_roundtrip`；F5 改写引用的既有域 `data-integrity-verification`「校验面与既有边界协调」确实存在且规定「完整性校验 SHALL 为只读操作，SHALL NOT 修改库文件内容」。`tests/legacy_compat_test.rs` 中被引用的断言逐条核对一致（目录行尾 9 字节 == `00×8 01`、未被写到的表保持 G1 行长、错误文本 `NON-INTEGER PRIMARY KEY` / `statement 1 of 2 failed` / `UNIQUE (INT columns only)` / `NOT NULL constraint violation` / `column 's'` / `statement 3 of 3 failed`、stderr 不含 `rebuild abandoned`、恢复行数门槛 == 133）。
- **迁移路径在 `/tmp` 用真实 CLI（`target/debug/rtsql`）独立复现**（仓库样本只读复制，复现后已清理）：路径 1（去 `PRIMARY KEY`）→ 退出 0，行 `[["k1","x"]]`；路径 2a（去 `UNIQUE`）→ 退出 0，行 `[[1,"a@b.c"]]`；**路径 2b（只改列类型）→ 退出 3，stderr `key column 'email' expects INT, got String`**；路径 2c（改类型 + 转换行值）→ 退出 0，行 `[[1,42]]`；路径 3a（回填 NULL）→ 退出 0；路径 3b（删除该行）→ 退出 0。**规格 R5 S2 的改写措辞与实测一致**：只写「去掉 `UNIQUE` 声明」与「改为整数类型**并同步转换行值**」两条可执行路径，并把 2b 的拒绝形态（键列类型门、退出码 3）明写为不可行——没有把不成立的路径写成可用。
- 另独立复核了两条一次性实测子句：旧 dump restore → INSERT → SELECT（退出码全 0，users 4 行）成立；当前构建导出 G1 旧库成立（`dump` 退出码 0、stderr 空、渲染当前长类型名、`SELECT COUNT(*)` = 3）。

**T3（双语 README）**：两文件各 1 行改写（`git diff --stat` = 2 files changed / 2 insertions / 2 deletions），未新增章节或段落。逐条比对中英要点集合等价：① 支持范围按目录行布局代际而非文件版本划分；② 范围内能力（打开 / 查询 / 写入 / 完整性 / 打开时重建 / dump、未决写入日志恢复、旧 dump 恢复且行数据等价、恢复出的库用当前格式且可继续写入与再次导出）；③ 写入后果（被写到的表目录行升级为当前布局，未被写到的表保持原代际）；④ 范围外三类拒绝形态（点名原因 + 失败语句序号 + 退出码 3）；⑤ 只写实测成立的三条迁移路径（与本次复现逐条一致）；⑥ 版本契约（唯一 `v1`、高版本 `NewerFileVersion` 退出码 1、无头文件 `NotADatabase` 退出码 1、不做嗅探与猜测性迁移）。既有的负面边界表述在中英两侧均保留。

**T4（全量回归记录）**：按本 Review 从 Act 的分批表重建 12 个文件批的成员并与仓库实际 `tests/*.rs` 清单对账——批大小 `11,11,11,11,11,11,11,11,12,12,12,11` 全部落在 8–12；拼接结果与实际文件清单**逐行相等**（missing 0 / extra 0 / duplicate 0，字典序逐批保持）；加单独成批的 `--lib --bins` 得 13 批 / 136 目标。逐批 passed 相加 = 2192，ignored 仅批 3 的 2 个，failed 全 0；与基线 2183 / 0 / 2 的差额 +9 全部归因于 `tests/legacy_compat_test.rs` 的 9 例（批 7 内该目标 9 passed），无未归因差额。

- **偏差 ⑤ 独立确认**：在本机 cargo 1.99.1 上复现 `cargo test --offline cargo test --offline --test <name> -- --test-threads=1`，cargo 在参数解析阶段即以 `error: unexpected argument 'test' found` 拒绝（退出码 1），未构建、未运行任何测试；全程**未发生**无目标过滤的全量调用（Act 记录的 13 批成员与退出码与本 Review 重建一致）。
- **偏差 ④ 影响判定**：切点后移一位使实执行的尺寸分布为 `8×11 + 3×12 + 1×11`（共 135 个文件目标），与 D10 等分规则给出的尺寸多重集 `{11×9, 12×3}` 相同，仅排列位置不同。「每批 8–12」「字典序」「不重不漏」「批间串行」「批内 `--test-threads=1`」四条纪律均未受影响。

**规格增量可合并性**：逐条核对 `legacy-format-compat`（5 Requirement / 14 Scenario）与 `database-file-format-header` 的 ADDED（1 Requirement / 3 Scenario）——每个陈述的行为都有用例断言、既有测试锚点或可复现的真实 CLI 记录支撑；DFH 的 S2 / S3 已改写为对既有域 `database-file-format-header`「格式错误显式拒绝 / 文件由新版创建」「旧无头文件统一拒绝」的引用，与既有域语义重复的部分已消除且无冲突；与既有 75 域逐面比对无冲突——`sql-constraint-enforcement`（非 INT 列 `UNIQUE` 计划期点名拒绝）、`key-column-type-conformance`（键列类型门）、`cli-noninteractive-shell`（restore 逐条 auto-commit + 失败退出码 3 + 定位到语句）、`data-integrity-verification`（校验只读）、`database-file-format-header`（格式拒绝分类与退出码 1）均被新域以引用或「沿用既有语义」方式承接，无重复声明、无相反陈述。`openspec validate --strict` → `valid`。

**身份机制与判定层**：change 产物全文检索 hash / 指纹 / 修订固定 / 运行标识 / 产物清单等关键词，命中项全部是「禁止 / SHALL NOT」表述，无任何身份机制实现；判定全部落在 `cargo test` 与真实 CLI 入口内，计数只用 `grep` / `comm` / `uniq` 一步通用管道，未新增判定脚本、封装命令、专用参数或框架外判定层；无 timer 阈值。

### Findings

均为**非阻塞 Minor finding**，不构成 Acceptance 阻塞，不要求当前 Cycle 修复。

| # | 分类 | 发现 | 处置 |
|---|---|---|---|
| M1 | Act 记录保真 | Act Response 的命令列记的是**裸名**形式（`rtsql new p1.db` / `rtsql dump g1_dump_probe.db`）。CLI 的裸名解析为 `$RTSQL_HOME/db/<name>.db`（`src/cli/resolve.rs:30`），按原文复跑会以「does not exist」退出 1；实际运行须用路径形式（`./` 或绝对路径）。**所记录的退出码与结果本身经本 Review 复现全部成立**（6 条迁移路径 + E4 的 dump/SELECT + S1 的 restore→INSERT→SELECT），失配的只是记录里的命令写法 | 记录层修正（不阻塞）；建议由 Recorder 在归档摘要或后续变更中以路径形式记录这些命令 |
| M2 | Plan 错误（已被 Act 纠正） | D7 与 Plan Context 的矩阵模板把 G2 的五格写成「行级既有覆盖」，与仓库证据不符 | Act 已改标「无 G2 专属断言」并在 E7 披露；**改写方向正确**，本轮接受 |
| M3 | 措辞落差 | 矩阵 G2 的「打开 / 查询」两格标「行级既有覆盖（E6）」，而 E7 声明「没有按 G2 形态构造的用例」——E6 的四个用例覆盖的是「可选段缺失」的兼容读分支，非 G2 形态本身 | 不阻塞；事实已由 E7 披露。建议后续承载 change 补齐 G2 文件级样本后，把这两格一并升格为文件级 |
| M4 | 归类建议 | DFH R1 的「产品 SHALL NOT 为同一次运行引入版本比对、指纹匹配、修订固定、运行标识或产物清单一类的身份机制」是**对后续变更的约束**，不是可观察行为 | 不阻塞；合并期可由 Maintainer 决定保留在规格（作为 SHALL NOT 约束）或只保留 design.md / proposal 中的可观察部分 |

### Acceptance Gaps（均不阻塞，记录待承载）

- **AG1（G2 文件级证据缺失）**：规格声明「G1、G2、G3 SHALL 全部属于受支持范围」，但 G2 无任何文件级样本或用例；矩阵已如实标注覆盖层级并披露「无 G2 专属断言」。**裁定**：不使规格矩阵不成立——规格没有把未验收行为写成已验收（覆盖层级逐格标注、E7 显式披露），且 G2 的支持性由已验收的兼容读机制（E6 的「可选段缺失即取默认值」）与已验收的写升级路径（G1 文件级证据）共同支撑；把 G2 排除出支持范围同样缺少证据。因此 AG1 属**证据深度缺口**，需新增 G2 形态样本与用例（产品 / 用例改动，超出本 Cycle「零 `src/`、零 `tests/`」边界）才能闭合，不属本 Iteration 既有 Acceptance 的必要条件。
- **AG2（三处子句只有一次性实测、无常态化用例）**：`E2`（旧库写入后仍能再次写入）、`E4`（当前构建导出 G1 旧库）、`R4 S1`（恢复出的库可继续写入）三处子句在仓库内没有常态化用例锁定，既有测试只覆盖**当前格式**的 restore→写与 dump 路径（`tests/default_declaration_test.rs`、`tests/cli_test.rs::test_dump_pk_rendering_matches_enforced_semantics`），不覆盖**旧格式**样本。**裁定**：三处子句的等级是「**已验收（一次性真实入口实测，证据随 change 归档保存）**」，不是「常态化锁定」——AGENTS 禁止的是判定层与身份机制，并未禁止以真实运行作为完成声明；本次三条子句的实测已由本 Review 独立复现，规格文本为真。但存在**规格与实现漂移**风险：后续改动若破坏这三条行为，仓库内不会有用例报警。

### 对 T2 证据策略与 G2 缺口的裁定

- **T2 证据策略（可接受）**：Cycle 的 Excluded scope 冻结了 `tests/`，Act 因此改用三次真实 CLI 运行闭合 S1 / E2 / E4，是既有执行依据内的唯一可行路径（Plan 的 T2 Required behavior #2 明确允许「本 change 已记录的实际运行结果」作为证据来源）。三次运行均走真实入口、只读仓库样本、记录命令与退出码，未引入判定层或身份机制，符合公共规则。代价是证据不可常态化——已作为 AG2 记录并给出承载建议。
- **G2 无专属断言（不构成阻塞）**：判定依据是「规格是否把未验收的一代写成已验收」。当前规格不构成该情形：矩阵逐格标注覆盖层级，E7 明确写出「没有按 G2 形态构造的用例，仓库内也没有 G2 布局的文件级样本」，且把「支持范围」与「证据强度」在文本上分开声明。**处置**：保留现状（不阻塞本 Iteration），以 AG1 交由后续承载 change 补齐；补齐后应同步把矩阵 G2 行整体升格为文件级并复核 README 是否需要显式点名 G2（中英两侧目前一致地只点名 G1 与 G3 两个代际端点）。

### 收敛判断

Iteration 001 的 A1–A5 全部达成：A1 矩阵与逐条对账无待改项残留；A2 三条迁移路径各有真实执行记录且规格措辞与之一致（本 Review 已独立复现 6 条路径的退出码与结果）；A3 双语 README 六项要点齐备且中英等价；A4 分批纪律与对账自洽（13 批 / 136 目标 / 2192 / 0 / 2 / 对账 0-0-0）；A5 产品代码与用例零改动、无身份机制、无框架外判定层。Act 的 5 条 Deviations 全部属**如实披露的范围内处置**，其中偏差 ③（2b 路径不成立后改写规格与 README）是本 change 最有价值的产出之一——它把一条不可执行的合同降级为真实边界。

**Iteration Plan 是否保持不变**：是。范围、依赖、验证边界与诊断边界均未变化，无 replan 需求。

### Issue 候选（只报告，不登记）

- **ISS26（候选）G2 形态文件级证据缺失**：规格声明 G2 属受支持范围，但仓库无 G2 布局的文件级样本与用例，矩阵八格中五格标注「无 G2 专属断言」、两格仅有行级兼容读覆盖。**性质**：证据缺口（非产品缺陷）。**承载建议**：由后续承载 change 补齐——按 `tests/fixtures/legacy/README.md` 已记载的旧构建复现流程（隔离目录 checkout + `cargo build --offline --bin rtsql`），在 G2 区间提交 `6e8dcf5`（2026-09-26，MS23+MS24，含 unique_roots 段、不含 secondary/kind）产出 G2 库文件样本，提交进 `tests/fixtures/legacy/`，并在 `tests/legacy_compat_test.rs` 增补 G2 形态的打开 / 查询 / 写入 / 完整性 / 打开时重建 / dump / 恢复断言；同步把规格矩阵 G2 行升格并复核 README 是否显式点名 G2。
- **ISS27（候选）三处一次性实测子句无常态化用例**：AG2 所列三条子句（E2 / E4 / R4 S1）目前只由一次性真实 CLI 运行支撑。**性质**：证据缺口（非产品缺陷）。**承载建议**：与 ISS26 合并进同一承载 change（同一文件、同一夹具族、同一验证入口），在 `tests/legacy_compat_test.rs` 增补三条常态化用例：旧 dump restore → INSERT → SELECT、旧库连续两次写入、当前构建导出 G1 旧库并核对渲染形态。

登记（建立 / 关闭 / 重开）由 `openspec-experience-recorder` 按用户指令执行；本 Review 只报告。

### Runbook 候选

- **R48 切分算法更新为等分规则**（Act 已提出，本 Review 复核后支持）：R48 记的「剩余 ≤ 12 则取剩余，否则取 11」在 134 个文件时产出 13 个目标的末批，超出 8–12；本 change 采用的等分规则（`B = ceil(N/12)`，尺寸多重集 `{11×(B×12−N), 12×(N−B×11)}`）实测满足 8–12 与不重不漏。建议由 Recorder 更新 R48。

### Follow-up Decision

无需当前 Cycle 修复，无返工项；本 Iteration 判定为完成。

- Next Cycle: None
- Next Iteration: None

### 收尾交接说明（`openspec-docs-maintainer` 执行；Plan 不执行）

1. **合并 2 份规格增量进 `openspec/specs/`**：新建 `legacy-format-compat` 域（5 Requirement / 14 Scenario，含逐代能力矩阵与 E1–E11 证据索引）；`database-file-format-header` 增补 1 条 Requirement（格式版本是产品格式契约，含 3 个 Scenario，其中 S2 / S3 为对既有域的引用）。合并期按 M4 决定「身份机制禁令」条款的归属；合并后域数 75 → 76。
2. **同步全局状态**：`tasks.md` 的 MS36 → completed 并记入两 Iteration 终态（000 / 001 均 accepted、全量终态 2192 / 0 / 2、136 目标 / 13 批）；`SNAPSHOT.md` 刷新（`tests/` 133 → 135 个文件 / 2171 → 2192 tests、行为规格 75 → 76 域、仓库现场段落同步 HEAD `0a08b14` ahead 9 与本 change 的未提交现场）；`project-model.md` / `references.md` / `improvements.md` 按需登记（本 change 无新增 M/R/I；R48 切分算法更新由 Recorder 处理）。
3. **归档 change**：`openspec/changes/2026-10-24-ms36-legacy-format-and-dump-compat/` 按正常收尾归档（终态 Review Result = `accepted`，两 Iteration 全部 accepted）。
4. **ISS26 / ISS27** 由 Recorder 按用户指令建立并确定承载 change（建议与 MS36 相邻排期、合并为一个「兼容合同证据补齐」change）。
