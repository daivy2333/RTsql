# 旧三段计时表在错误路径的输出收敛

来源：MS44（统一分级诊断与性能归因）收尾 Review 遗留的非阻塞 Minor finding，由用户裁定处置。关联：M19、M20、M23；ISS42（不在本 change 承载）、ISS43（已在主分支修正）。

Gate 1 为 PASS。批准来自本次 goal 指令「除了 iss42，其他的都顺便做掉吧」对 MS44 收尾遗留项的处置授权，不是跳过。本文件不更新全局里程碑状态，不改 SNAPSHOT，不动已归档的 MS44 产物。

## Why

MS44 把旧的三段计时表（`RTSQL_PROFILING` 打开时输出 parse / plan / execute 耗时）从「只在成功路径输出」改成「每条返回路径都输出」，并在权威规格 `openspec/specs/unified-diagnostics/spec.md` 的 Requirement「既有计时出口和 stderr 告警保持」里写下该行为，新增场景「失败路径也输出旧的三段计时表」。

收尾 Review（`openspec/changes/archive/2026-10-28-ms44-unified-diagnostics/iterations/003-final-regression/000-initial.md`）把它记为非阻塞 Minor finding：需求 8 的原意是「保持」既有计时出口，而不是新增错误路径的输出。MS44 只把规格文字对齐到实现，没有改产品代码，也没有裁定这个行为该保留还是该收敛。

用户现已裁定：**收敛回变更前的行为**——旧表只在走到执行阶段的返回路径输出，执行阶段之前的失败路径不输出该表。

## What Changes

- 把旧三段计时表的输出时机改回与 MS44 变更前（基点 `e69c7e2`）一致：只有走到执行阶段的返回路径输出这张表。解析、规划等执行阶段之前的失败路径不输出该表；进入执行阶段后无论成功还是失败，仍输出该表；计划缓存命中后直接执行的路径仍输出该表。
- 同步权威规格 `unified-diagnostics` 的 Requirement「既有计时出口和 stderr 告警保持」：改写「每条返回路径都会输出这张表」那一段；场景「失败路径也输出旧的三段计时表」改写为「进入执行阶段后失败」的失败路径，并新增场景「执行阶段之前的失败路径不输出旧的三段计时表」。
- 补齐权威规格 Requirement「成功、失败和取消都结束记录」的场景「解析失败留下失败摘要且错误不变」中「含有解析阶段」这一子句的断言缺口（`tests/diagnostics_statement_test.rs` 的 `parse_error_keeps_error_text_and_summary_fails` 只断言了 severity、错误文本与 SQLSTATE）。

无 **BREAKING**。`RTSQL_PROFILING` 未设置时的行为、默认诊断配置下的行为、诊断（新摘要）行为、SQL 结果、错误文本、SQLSTATE、取消后的行可见性都不变。

## Capabilities

### New Capabilities

无。

### Modified Capabilities

- `unified-diagnostics`：Requirement「既有计时出口和 stderr 告警保持」改写旧表的输出时机，并改写、新增对应的失败路径场景。域路径 `openspec/specs/unified-diagnostics/`。其余 Requirement 不动。

## Impact

- 代码：`src/pipeline.rs` 的 `execute_inner` / `execute_staged`——旧表的打印点从「`execute_staged` 返回后无条件打印」改回「只有走到执行阶段的返回路径打印」。`src/profiling.rs` 不改：表头、列、`Total` 行算法、`is_profiling_enabled` 的判定、`record_time` 在旧 scope 外仍 panic 的既有行为全部保持。
- 不改：诊断（新摘要）的采集与输出、默认配置下的 stderr、SQL 文本、SQLSTATE、事务提交或回滚条件、磁盘格式、checkpoint 九步、WAL 刷盘单飞、`[recovery]` 等既有告警原文。不新增 crate 依赖，不新增 CLI 参数、SQL 语法或网络协议字段，不引入操作标识字段、运行标识、内容哈希或其他身份证明机制，不新增判定层、专用 CLI 或审计工具。
- 测试：`tests/diagnostics_statement_test.rs` 新增旧表输出时机的断言，并给既有 `parse_error_keeps_error_text_and_summary_fails` 补「含有解析阶段」断言。既有 `tests/recovery_bounded_read_test.rs` 对 `[recovery]` 的断言保持。收尾全量按 M19 分批串行，整个 change 只跑一次。
- 关联：MS44；M19；M20；M23。ISS42（`RTSQL_PROFILING` 对 CLI 主命令无效、scope 外 panic）待用户裁定，**本 change 不承载**，也不触碰 `record_time` 的 scope 安全性。

## 需求

1. 旧三段计时表只在走到执行阶段的返回路径输出。执行阶段之前的失败路径（解析失败、规划失败、多语句拒绝、占位符编号失败、参数个数不匹配、无语句可执行）不输出该表。进入执行阶段后无论成功还是失败都输出该表；计划缓存命中后直接执行的路径也输出该表。
2. 表头、列、行的内容与排序、`Total` 行算法不变。`Total` 的计时窗口不变。`RTSQL_PROFILING` 未设置时不输出该表。
3. 该收敛只改这张表的输出时机。语句返回的错误文本与 SQLSTATE 不变；诊断（新摘要）行为不变：失败语句仍产生失败摘要，取消仍记为取消，行可见性不变。
4. 权威规格场景「解析失败留下失败摘要且错误不变」的「含有解析阶段」子句由测试锁定。

## 不做

- 不让旧表在所有错误路径都不输出（执行阶段的错误路径仍输出该表）。
- 不改旧表的列、行、`Total` 算法、计时窗口或 `record_time` 的调用面。
- 不改诊断（新摘要）的任何行为，不改默认配置下的 stderr，不改既有告警原文。
- 不修复 ISS42：不让 `record_time` 具备 scope 安全性，不让 CLI 主命令接入旧表，不动 `src/profiling.rs`。
- 不修改已归档的 MS44 产物（归档载体不可变）。
- 不新增操作标识字段、运行标识、内容哈希或其他身份证明机制；不新增判定层、专用 CLI、脚本或审计工具。
- 不把「错误路径不输出旧表」扩大成「错误路径不产生任何 stderr 输出」。
- 不预建「修复尚未发现的缺陷」的 Iteration。

## 场景草图

| # | 类别 | 前置 | 动作 | 可观察结果 | 失败边界 |
|---|---|---|---|---|---|
| S1 | Sad | `RTSQL_PROFILING` 已设置，会话配置保持默认 | 经语句管道执行一条在执行阶段之前失败的语句（规划期失败） | stderr 不含旧的三段计时表；错误文本与退出码不变 | 提前失败仍打出这张表 |
| S2 | Sad | `RTSQL_PROFILING` 已设置，会话配置保持默认 | 经语句管道执行一条进入执行阶段后失败的语句 | stderr 仍含这张表；错误文本与退出码不变 | 把「所有错误都不输出」做成过度收敛 |
| S3 | Happy | `RTSQL_PROFILING` 已设置，会话配置保持默认 | 经语句管道执行成功的语句 | stderr 仍含 parse、plan、execute 与 `Total` 行 | 成功路径的表消失 |
| S4 | Happy | `RTSQL_PROFILING` 未设置 | 经语句管道执行成功的语句 | stderr 不含这张表 | 开关失效 |
| S5 | Sad | 性能统计打开 | 执行无法解析的语句 | 摘要状态为失败且含解析阶段；错误文本与 SQLSTATE 不变 | 补的断言失败或与旧表收敛冲突 |
| S6 | 兼容 | 诊断配置默认 | 既有恢复路径 | stderr 仍含 `[recovery]` 原文 | 借收敛之名改写告警 |

## 回滚

改动在 change 分支的 `src/pipeline.rs` 与 `tests/diagnostics_statement_test.rs`，加上本 change 的规格 delta。回滚是还原或放弃该分支。主分支在合并前不被改写。无格式迁移，无新的磁盘字段。旧表的输出时机回到 MS44 变更后的状态时，SQL 结果、错误文本与 SQLSTATE 与改前相同，因此不需要数据回放。

## 默认假设

下列假设闭合场景缺口。它们不扩大已批准范围，不引入身份证明机制，不改变事务结果。

1. 「对齐变更前」以 MS44 变更的基点 `e69c7e2`（master 上该变更的 merge-base）的代码为准，逐条返回路径核对，不按「错误」二字泛化。依据：用户指令要求精确对齐，且该基点是变更前的权威代码状态。
2. 旧表的「表」指 `src/profiling.rs::print_timings` 打到 stderr 的那张三段计时表（表头含 `Time (µs)`、行含 parse / plan / execute 与子指标、末行是 `Total`）。判定用表头特征串，不引入新的行标识。依据：既有测试 `legacy_profiling_table_follows_env_var` 已用 `Time (µs)` 区分该表与新诊断输出，沿用同一判据。
3. 失败路径经真实 CLI 入口观察：`rtsql import` 的自动建表 DDL 与逐行 INSERT 都经 `Database::execute_sql` 进入语句管道，其 stderr 是产品的真实输出面。库 API 入口的 stderr 在 Rust 测试框架内不可直接观察（`eprintln!` 被测试框架捕获），因此失败路径的判定走 CLI 子进程。依据：公共规则 › 验证要求断言用户实际所见，且判定不建在项目测试框架之外。
4. 执行阶段之前的失败用规划期失败代表（重复列名的 CSV → 自动建表 DDL 在规划期被拒），执行阶段的失败用键冲突代表（预置同键行 → 逐行 INSERT 在执行期被拒）。两者都是 `rtsql import` 的真实用户操作。依据：不新增失败注入点，失败面沿用既有产品守卫。
5. 解析失败路径不单独建 CLI 判定：CLI 生成的 SQL 文本不经过用户可控的解析失败面。解析失败与其他执行阶段之前的早退（多语句、占位符、参数个数、无语句）由实现与基点逐条对齐保证，规格用一条覆盖「执行阶段之前」的规则表述。依据：不为覆盖一个不可观察面新增注入点或判定层。
6. 场景 S5 的补断言在当前实现下已成立（实测：解析失败摘要有且仅有 `parse` 阶段），属于锁定既有行为的断言强化，不是产品行为变更。依据：MS44 收尾 Review 已判定该子句「实现满足，测试未锁定」。
7. 场景「失败路径也输出旧的三段计时表」改写而不是删除：OpenSpec 集成（`openspec validate --strict`）要求 MODIFIED 块保留当前规格里已有的全部场景标题，删除会判 ERROR 并阻断 `openspec archive`。因此保留该标题，把它的 GIVEN / WHEN 收窄到「已触达执行阶段的失败路径」，并新增互补的场景「执行阶段之前的失败路径不输出旧的三段计时表」，两个场景合起来覆盖失败路径的两半。依据：用户授权是「删除或改写」，改写同样落在授权内；保留标题也满足集成约束，保持仓库 `openspec validate --all --strict` 的干净基线（本次规划前为 90 passed / 1 failed，唯一的 failed 就是本 change）。

## 后续 change

不创建。旧表输出时机的收敛、规格 delta 与断言补强在本 change 内完成。ISS42 待用户裁定，不在本 change 内承载，也不写入本提案。

## 实施期缺陷

本 change 实施期新发现的实质缺陷按 M23 由当前 change 追加 Iteration 就地承载；范围外的实质缺陷按 M20 报告为 Issue 候选，不在本 change 内扩范围修复。本计划不预建那一轮。

## 未自行批准的缺口

无。把「错误路径」泛化成「所有错误都不输出」、改动旧表的列与算法、触碰 ISS42 的 `record_time` scope 安全性，以及为判定新增身份机制或专用工具，都落在「不做」里，本计划不采用。
