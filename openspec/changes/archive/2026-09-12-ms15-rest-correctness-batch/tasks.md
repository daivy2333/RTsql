# tasks — MS15-Rest 初版前正确性收口第二批（I034 / I037 / I039）

> 规划：openspec-plan 2026-09-12；用户指示将 MS15 剩余工作（T02/T03/T04）聚合为一个 change。Gate 1 已批准（2026-09-12 用户原话"批准"：三 delta spec 需求与范围 + 三项默认假设 ① I039 解析侧归一化 ② I037 邻接 rekey 形态排除 ③ 工作区不提交，无推翻项）。Gate 2 已批准（同上，Plan Context 000-initial 转 ready）。
> 设计：design.md D1-D5。
> 状态：Iteration 000 accepted；Iteration 001 经 000-initial（Gate 6 阻塞）→ 001-replan（用户批准 T8-R2 校准）accepted 收口（2026-09-12，Plan Review 独立复跑 861/0/2）。Iteration 002 经 000-initial（T7/T8 完成，T9 于转义名 dump 探针 Gate 6 阻塞）→ Plan Review rework-required → 001-rework（T9-R1：`select_all_rows` 经 `quote_ident` + 转义名 dump 往返测试）accepted 收口（2026-09-12，Plan Review 独立复跑 867/0/2 + P5 独立复跑）。**change 实施侧完成，待用户 commit + docs-maintainer 收尾。** Plan Review 遗留登记项（范围外，转 docs-maintainer）：I037 邻接 rekey 形态（proposal 默认假设 2）、跨进程同键 UPDATE→DELETE 变更丢失（独立复现确认）、import 实参插值边界观察。

## Task List

| Task | 状态 | 目标 | 关键产出 | Iteration |
|---|---|---|---|---|
| T1 | **completed**（2026-09-12） | RED 测试见证：R1 CLI 表头缺陷形态 | `tests/cli_test.rs` 追加：裸 DataScan 子集投影表头（json `columns`=["name"] 与 `rows` 字段数一致）、下推 DataScan 子集投影表头、聚合/表达式路径表头零回归锚点；修复前观察 RED | 000 |
| T2 | **completed**（2026-09-12） | R1 实现（design D1） | `src/parser/planner/query.rs` `get_plan_output_columns`：DataScan 臂应用 `node.projection`（必需），Scan/IndexScanAll 臂恒等加固，IndexScan 臂保持；`tests/cli_test.rs:289` 注释按新语义校准；T1 转 GREEN | 000 |
| T3 | **completed**（2026-09-12） | R1 回归收尾 | 聚合 `input_schema`（query.rs:634）与派生表列注册（query.rs:129）消费面复核；全量 `cargo test`（基线 853 只增不减）、clippy/fmt 0、`openspec validate --changes` PASS | 000 |
| T4 | **completed**（2026-09-12，000-initial） | RED 测试见证：R2 键位无键值索引清理 | 新增 `tests/update_index_maintenance_test.rs` 5 用例；RED 形态与契约逐字一致（INSERT DuplicateKey 误拒 / 点查残留 / 两态面） | 001 |
| T5 | **completed**（2026-09-12，000-initial；实施完成，新套件 GREEN） | R2 实现（design D2） | `src/executor/update.rs` Step 7 分支化 + Risks 探针实测（serialize 不拒 String 入 Int 列，delete 分支与恢复一致）；GREEN condition 的 T8-R2 阻塞经 replan 移交 T6 | 001 |
| T6 | **completed**（2026-09-12，001-replan） | R2 回归收尾（含 T8-R2 语义校准，2026-09-12 replan） | `tests/keyless_row_test.rs::keyless_row_update_recovery_after_crash` 校准（第二次 UPDATE 断言 `Key not found` + 恢复面 v=0 行恰 1 + 重开旧键 INSERT 成功，注释按新语义改写，其余 3 用例与产品代码零修改）；keyless_row_test 4/4、全量 861 passed / 0 failed / 2 ignored、clippy/fmt/validate 干净；Act Response `reported` 待 Plan Review | 001 |
| T7 | **completed**（2026-09-12，002 000-initial） | RED 测试见证：R3 表名归一化 | `tests/cli_test.rs` 追加 5 用例（带引号建表裸名互访 / 五语句等价互访 / 裸名 UPDATE-DELETE 命中带引号建表 / dump→restore→dump 恒等 / schema 重建同名）；修复前 5 failed 形态逐字一致（互访报表不存在 / 二代 `"""mixed"""` 膨胀）；见证形态两处等价调整见 Act Response Deviations | 002 |
| T8 | **completed**（2026-09-12，002 000-initial） | R3 实现（design D3） | `src/parser/ast.rs` 新增 `object_name_to_table_name` helper（`Ident.value` + lowercase + `.` 连接）+ 改写 3 个既有 helper 函数体（签名不变）+ 替换 8 处内联消费（query.rs:120、pipeline.rs:867/958/971、subquery.rs:136/140、ddl_dml.rs:305/348，共 11 处）；`lifecycle.rs` 零修改（Forbidden 保持）；T7 全转 GREEN（cli_test 64 + parser_test 6 + planner_test 36） | 002 |
| T9 | **completed**（2026-09-12，002 001-rework；000-initial 曾 Gate 6 阻塞，经 Plan Review rework-required 转 T9-R1 收口） | R3 回归与 change 全量收尾 | 000-initial：全量 866/0/2、探针 P1-P4 全过、P5 转义名 dump 失败（`select_all_rows` 裸插值）→ Blocker Handoff；001-rework（T9-R1）：`select_all_rows` 经 `quote_ident` 包裹 + 头注释按归一化语义改写、新增 `test_escaped_name_dump_restore_identity`（RED→GREEN）；全量 **867 passed / 0 failed / 2 ignored**、clippy/fmt 0、validate PASS、探针 P4/P5 复跑 PASS（Plan Review 独立复跑采信） | 002 |

## Iteration Plan

### Iteration 000: CLI 表头与投影行形状一致（I034）

- Tasks: T1, T2, T3
- Depends on: None
- Stable baseline: 裸 DataScan / 下推 DataScan 子集投影的 CLI 表头等于投影列名，json `columns`/`rows` 字段数一致；聚合与表达式路径表头零回归；全量回归零修改
- Verification boundary: T2/T3 全绿 + clippy/fmt 0 + validate PASS
- Diagnostic boundary: `src/parser/planner/query.rs`（`get_plan_output_columns`）+ `tests/cli_test.rs` 追加用例
- Non-goals: I037/I039、IndexScan 臂改动、plan 路由、渲染格式族

### Iteration 001: UPDATE 键位无键值索引条目清理（I037）

- Tasks: T4, T5, T6
- Depends on: Iteration 000（共享全量基线与工作区，无代码依赖）
- Stable baseline: 键位置 NULL 后旧键 INSERT 成功、点查空集、恢复两态一致；非键列与原值更新行为不变；全量回归通过（T8-R2 按 R1 语义校准除外——2026-09-12 replan 修订，spec R2 校准条款）
- Verification boundary: T5/T6 全绿 + clippy/fmt 0 + validate PASS
- Diagnostic boundary: `src/executor/update.rs`（Step 7）+ `tests/update_index_maintenance_test.rs`
- Non-goals: rekey 可键控形态（proposal 默认假设 2）、WAL/恢复语义、插入侧去重逻辑

### Iteration 002: 表名解析归一化与 dump 保真（I039）

- Tasks: T7, T8, T9
- Depends on: Iteration 001（共享全量基线与工作区，无代码依赖）
- Stable baseline: 带引号与裸名拼写全语句等价；dump→restore→dump DDL 恒等；裸名既有语义零回归
- Verification boundary: T8/T9 全绿 + clippy/fmt 0 + validate PASS + CLI 探针
- Diagnostic boundary: `src/parser/ast.rs`（helper）+ 10 处替换点 + `tests/cli_test.rs` 追加用例
- Non-goals: dump `quote_ident` 输出语义、历史带引号表名迁移、multipart/schema 限定名能力

### 平衡审计

三个 Iteration 各服务一个独立可验收成果（CLI 渲染正确性 / UPDATE 索引维护正确性 / 表名解析语义），故障域互异（渲染消费面 / 执行器索引步 / 解析归一化），验证与诊断边界各自清晰。不合并：聚合为单 Iteration 会使诊断边界横跨三域、失败归因模糊；不再拆分：单项改动面均 ≤2 代码文件 + 1 测试文件，工作量适中。依赖为工作区串行（避免全量基线混叠），无代码耦合——单项失败不阻塞其余 Iteration 的独立实施与回滚。

## 相关依据

- improvements I034（MS10-T04 Plan Review finding 5）、I037（MS10-T05 001-rework Plan Review Finding 5）、I039（MS10-T05 Iter001 Act Deviation 1）
- tasks MS15-T02/T03/T04（缺陷依据对应如上）；用户 2026-09-12 聚合指示
- 行级调查证据：design.md D1-D3（2026-09-12 本会话，工作树基线 f9e1e1f + MS15-T01 未提交实施）
