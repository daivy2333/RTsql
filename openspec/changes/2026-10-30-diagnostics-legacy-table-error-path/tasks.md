# 旧三段计时表在错误路径的输出收敛

来源：MS44 收尾 Review 的非阻塞 Minor finding，由用户裁定收敛。约束：M19、M20、M23。ISS42 不在本 change 承载。

Gate 1 为 PASS，批准来自本次 goal 指令「除了 iss42，其他的都顺便做掉吧」对 MS44 收尾遗留项的处置授权，不是跳过。本文件不更新全局里程碑状态。

## 当前批次状态

- 需求与 BDD：completed。场景草图在 `proposal.md`；规格 delta 在 `specs/unified-diagnostics/spec.md`。
- 000：实施完成——Act 已执行 T1、T2（Gate 3 与 Gate 4 通过；定向验证退出码 0、failed 0）。等待 `openspec-plan` Review 当前 Cycle。
- 001：收尾全量，只保留在下方 Iteration Plan，未展开目录。
- Persisted Evidence：none。
- 跳过项：无。

## 全部任务与归属

| Task | 所属轮次 | 依赖 | 交付与通过依据 | 状态 |
|---|---|---|---|---|
| T1 | 000 | 无 | 旧表只在走到执行阶段的返回路径输出；执行阶段之前不输出；表算法、错误文本与 SQLSTATE 不变 | completed |
| T2 | 000 | T1 | 权威规格「解析失败留下失败摘要且错误不变」的「含有解析阶段」子句由测试锁定 | completed |
| T3 | 001 | 000 accepted | M19 分批串行跑一次全量，每批退出码 0 且 failed 为 0 | pending |

## 1. 旧表输出时机收敛（Iteration 000）

- [x] 1.1 T1：按 `iterations/000-initial/000-initial.md` 的 T1 Task Definition 执行。先写 RED 断言，再改实现。本 change 的 delta 规格（`specs/unified-diagnostics/spec.md`）已随 change 产物就位，改实现时不重写规格正文。

当前行为：`RTSQL_PROFILING` 已设置时，`execute_inner` 在 `execute_staged` 返回后**无条件**调用 `print_timings`（`src/pipeline.rs:1456-1458`），因此解析失败、规划失败、多语句拒绝、占位符编号失败、参数个数不匹配、无语句这些执行阶段之前的失败路径也会打一张只含已完成阶段的旧表。

完成后：旧表只在走到执行阶段的返回路径输出——缓存命中后直接执行、以及解析与规划都成功后进入执行，两者都输出（执行阶段失败也输出）；执行阶段之前的六个早退都不输出。表头、列、行、`Total` 行算法、`total_start` 的计时窗口起点、`record_time` 的调用面、`RTSQL_PROFILING` 未设置时的行为全部不变。错误文本与 SQLSTATE 不变。诊断（新摘要）行为不变。

必须保持：`src/profiling.rs` 零改动（表渲染与 `record_time` 的 scope 外 panic 行为都不动，ISS42 不在承载面）。`record_time` 的调用点与 `profiling` 布尔参数的传递不变。`[recovery]` 等既有 stderr 告警原文不变。SQL 结果、SQLSTATE、取消后的行可见性不变。

禁止：改旧表的列、行或 `Total` 算法；把「所有错误都不输出」一起改掉；改诊断（新摘要）的任何行为；让 `record_time` 具备 scope 安全性或让 CLI 主命令接入旧表（ISS42）；新增操作标识字段、运行标识、内容哈希或其他身份证明机制；新增判定层、专用 CLI、脚本或审计器；改 `tests/recovery_bounded_read_test.rs` 的告警断言；改 `openspec/specs/` 权威正文；跑全量。

先行测试：`tests/diagnostics_statement_test.rs` 的 `legacy_table_absent_before_execute_stage`（RED）与 `legacy_table_present_on_execute_stage_failure`（守另一半，改前 GREEN）。RED 是前者失败——当前实现下执行阶段之前的失败路径仍打出这张表。GREEN 是该文件全部用例退出码 0、failed 为 0。

Verification：`cargo test --offline --test diagnostics_statement_test -- --test-threads=1`。该文件经 CLI 子进程操纵进程级 `RTSQL_PROFILING`，所以批内单线程。这不是全量。全量在 T3。

停止条件：为了对齐基点的打印点必须改变错误文本、SQLSTATE、诊断摘要内容或取消后的行可见性才能完成时，停止并回到计划。

## 2. 解析阶段断言补强（Iteration 000）

- [x] 2.2 T2：给 `tests/diagnostics_statement_test.rs` 的 `parse_error_keeps_error_text_and_summary_fails` 补上权威规格场景「解析失败留下失败摘要且错误不变」的「含有解析阶段」子句断言。只加断言，不改产品代码，不改该测试已有的 severity、错误文本与 SQLSTATE 断言。

当前行为：该测试只断言 severity 为 Error、错误文本逐字节相同、SQLSTATE 相同；规格写明的「含有解析阶段」没有断言锁定。

完成后：解析失败语句的摘要里有 `parse` 阶段断言就位。补断言前后都是 GREEN（实测：解析失败摘要有且仅有 `parse` 阶段）——这是锁定既有行为的断言补强，不是产品行为变更。

必须保持：产品代码零改动。规格正文不改（权威规格该场景已经写明「含有解析阶段」）。

禁止：改产品代码以让断言成立；改该测试已有的错误文本、SQLSTATE、severity 断言；改规格正文；把断言写成对摘要字段的相等比较以外的强耦合形式（只断言阶段存在与阶段名）。

先行测试：本任务没有 RED——断言在改前就成立，属 characterization。变更前 GREEN：`cargo test --offline --test diagnostics_statement_test -- --test-threads=1` 中该用例通过。

Verification：`cargo test --offline --test diagnostics_statement_test -- --test-threads=1`，退出码 0 且 failed 为 0。

停止条件：补上的断言在改前不成立，或与 T1 的改动冲突时，停止并回到计划，不自行改规格正文。

## 3. 收尾全量（Iteration 001）

- [ ] 3.1 T3：T2 通过后，按下面的分批方式跑一次全量。这是整个 change 的唯一一次全量。不改产品代码。失败时停止，不在本轮里修。

分批是通过条件的一部分：

1. 执行时枚举，不把规划时的文件数写死。集成测试文件来自 `ls tests/*.rs | sort`。
2. 第一批单独执行：`cargo test --offline --lib --bins -- --test-threads=1`。
3. 其余文件按该排序每 12 个一批，最后一批可以少于 12。文件名去掉 `.rs` 得到 `--test` 的名字。每批一条命令：`cargo test --offline --test <名> --test <名> ... -- --test-threads=1`。
4. 批间一条命令结束后再开始下一条。禁止一次不带 `--lib` / `--bins` / `--test` 过滤的 `cargo test`。禁止全量批次使用默认并行度。
5. 对账：全部 `--test` 名字的集合等于枚举集合，无缺失、无多余、无重复；`--lib` 和 `--bins` 只出现在第一批。
6. 每一批把退出码和 failed 计数写入 Act Response。退出码 0 且 failed 为 0 才算该批通过。任一批未跑，不得声明全量通过。
7. 某一批失败时，只把该批再跑一次。第二次仍失败则本任务失败，不改并行度，不为了凑通过而重跑其余已通过的批。

## Iteration Plan

### Iteration 000：旧表输出时机收敛

- Tasks：T1、T2
- Depends on：None
- Stable result：旧三段计时表只在走到执行阶段的返回路径输出，执行阶段之前的失败路径不输出；表算法、错误文本、SQLSTATE 与诊断行为不变；解析失败摘要含解析阶段的断言就位。
- Verification boundary：`cargo test --offline --test diagnostics_statement_test -- --test-threads=1`，退出码 0 且 failed 为 0。不跑全量。全量预算在 001，整个 change 一次。
- Diagnostic boundary：`src/pipeline.rs` 的旧表打印点与 `src/profiling.rs` 的表渲染；`tests/diagnostics_statement_test.rs` 的 CLI 子进程断言。诊断摘要内容变化、错误文本变化、SQLSTATE 变化不属于「移动打印点」能解释的失败。
- Non-goals：旧表的列与算法、诊断（新摘要）行为、ISS42、默认配置下的 stderr、既有告警原文、全量回归。

### Iteration 001：收尾全量

- Tasks：T3
- Depends on：000 accepted
- Stable result：默认构建上按 M19 分批串行的一次全量，每批退出码 0 且 failed 为 0，目标对账无缺无重。
- Verification boundary：本节 T3 的分批方式。只此一次。失败批只整批复核一次。
- Diagnostic boundary：按失败测试所在的批和目标文件定位。不把全量失败当成修改分批规则的理由。
- Non-goals：新行为、补跑已通过的批、无过滤全量、默认并行度全量。

## 平衡审计

000 交付的是同一个测试文件、同一套 stderr 判据、同一故障域下的一组结果：旧表在执行阶段之前不输出、在执行阶段失败时照旧输出、成功路径照旧输出，外加同文件内的一处断言补强。拆成两轮会得到两个都只改一处断言的半成品轮，且第二轮的断言补强本身不产生独立可验收结果，因此合并为一轮。001 只有全量回归，与 MS44 的收尾轮同样式，把行为失败与整库回归分在不同排障范围里。两个 Iteration 都不是按文件数、代码行数或固定任务数切的。没有预建「修复尚未发现的缺陷」的 Iteration。

## Requirements Traceability Matrix

| Requirement | Task | First Test | Status |
|---|---|---|---|
| 既有计时出口和 stderr 告警保持（旧表只在走到执行阶段的返回路径输出） | T1 | `legacy_table_absent_before_execute_stage` | Covered |
| 既有计时出口和 stderr 告警保持（执行阶段失败仍输出该表，成功路径与开关语义不变） | T1 | `legacy_table_present_on_execute_stage_failure` | Covered |
| 成功、失败和取消都结束记录（「解析失败留下失败摘要且错误不变」的「含有解析阶段」子句） | T2 | `parse_error_keeps_error_text_and_summary_fails` | Covered |
| 收尾全量（M19 分批预算） | T3 | M19 分批命令 | Covered |
