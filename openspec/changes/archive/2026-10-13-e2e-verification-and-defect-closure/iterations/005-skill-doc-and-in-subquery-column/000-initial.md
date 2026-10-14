# Iteration 005 / Cycle 000: 技能文档与 IN 子查询首列

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T8, T9（执行序 T8 → T9）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R8（`in-subquery-join-rejection` ADDED）；R7 里尚未落盘的 `rtsql-docs/SKILL.md`；design D11
- Excluded scope: README 双语（Iteration 004 已写入）；索引路由；`Sort`/`Limit`/`Projection` 的首列臂；语料合并；Issue 台账

**Objective**

`WHERE col IN (SELECT … WHERE <裸名主键> = …)` 按子查询 SELECT 清单的第一列返回匹配行，不再报 `Subquery returns multiple columns`。`rtsql-docs/SKILL.md` 写上与 README 相同的五条，并改掉超长 `DEFAULT` 的页满说法。

**Background**

Iteration 004 的审查发现 `rtsql-docs/SKILL.md` 没有 T7 的五条。用户 2026-10-14 决定：该遗留与 Iteration 003 的索引扫描首列误报一起放到下一轮。原话：「这个issue和当前cyc的遗留工作共同规划到下一个iter」。

**Investigation Facts**

- Current Baseline: Iteration 004 Act Response（2026-10-14）全量 1989 passed / 0 failed / 2 ignored。e2e 22 passed / 0 ignored。README 双语五条已在。`rtsql-docs/SKILL.md:192` 仍写超长 `DEFAULT` 按页满错误拒绝。
- Current-State Evidence:
  - `get_subquery_first_column`（`src/parser/planner/subquery.rs:633`）的匹配臂有 Scan、DataScan、Filter、Aggregate、SemiJoin、AntiJoin、Join、NestedLoopJoin、DerivedScan。其余落入 `:702`，报 `SubqueryReturnsMultipleColumns`。
  - 唯一调用点是 `IN` 右支（`subquery.rs:61`）。`EXISTS` 不调用它。
  - 裸名主键等值走 `IndexScan`（`query.rs:2095-2114`）。无 `ORDER BY` 且投影下标存在时，`columns` 是 SELECT 清单。`get_plan_output_columns` 对 `IndexScan` 直接返回 `node.columns`（`query.rs:88-94`），不再套 `projection`。
  - 二级索引等值走 `IndexScanAll`（`query.rs:2125-2142`）。`columns` 恒为基表全列，`projection` 决定输出。`get_plan_output_columns` 用 `projected_columns`（`query.rs:43-48` 与 `:95`）。
  - 限定名 `dept.id = 10` 不是简单主键等值，不进 `IndexScan`。JOIN 子查询仍由 `:688-689` 报 `InSubqueryJoinUnsupported`。
  - 带 `ORDER BY` 的内层计划根是 `Sort`，本函数没有 `Sort` 臂。这不是本轮要修的形态。
- Code and Critical Path: `IN` 谓词 → `try_build_where_subquery` → 内层 `build_query` 得到 `IndexScan` 或 `IndexScanAll` → `get_subquery_first_column` → 用返回的列名做 SemiJoin 条件。

**Implementation Guidance**

T8 只加两臂。`IndexScan` 取 `columns[0]`，表名用 `table_name`。`IndexScanAll` 先按 `projected_columns` 的规则得到输出列名，再取第一项，表名用 `table_name`。空清单维持多列错误。

T9 把 README 已写的五条放进技能文档的既有小节，不新开一套结构。第 192 行改成计划期点名拒绝，文案与 README 的 `DEFAULT value is too large for the catalog page capacity` 一致。

**Task Contracts**

### T8: IN 子查询索引扫描取首列（R8）

- Requirement/Scenario: R8 / 裸名主键等值、非首列输出、二级索引等值、限定名与 JOIN 保持
- Depends on: Iteration 004 accepted
- Targets: `src/parser/planner/subquery.rs::get_subquery_first_column`
- Current behavior: 内层 `WHERE id = 10` 这类裸名主键等值使右支成为 `IndexScan`，首列提取报 `Subquery returns multiple columns`。二级索引等值的 `IndexScanAll` 同样落入兜底臂。
- Required behavior: 上述两条返回 `get_plan_output_columns` 的第一列，查询按该列匹配。`IndexScanAll` 在投影非空时不得改用基表第一列。
- Preserve: Scan/DataScan/Filter/Aggregate/SemiJoin/AntiJoin/DerivedScan 臂不动。JOIN 仍报 `IN subquery with JOIN is not supported`。限定名主键等值仍不走 `IndexScan` 且结果不变。`EXISTS` 的调用关系不变。
- Forbidden: 改 `query.rs` 的索引路由；为 `Sort`、`Limit`、`Projection` 加臂；改执行器行裁剪。
- First test: 在 `tests/subquery_test.rs` 增加裸名主键等值用例，运行 `cargo test --test subquery_test <新测试名> -- --test-threads=1`，预期失败，错误含 `Subquery returns multiple columns`。
- GREEN condition: 该用例返回匹配行。同文件再覆盖非首列输出、二级索引等值、限定名对照、JOIN 拒绝文案不变。
- Verification: `cargo test --test subquery_test -- --test-threads=1` 0 failed。全量留到 T9 之后，本 Iteration 一次。
- Stop when: 补臂后，限定名主键等值或无 WHERE 的 `IN` 结果变了。出现这些情况时停，带回 Plan。

### T9: 技能文档补齐（R7 剩余）

- Requirement/Scenario: R7 的 `rtsql-docs/SKILL.md` 部分
- Depends on: T8（文档收口放在代码之后；五条陈述本身不依赖 T8）
- Targets: `rtsql-docs/SKILL.md` 的 Behavioral notes、Select output、Dump and restore、SQL capability surface 或 Completion checklist 中已有的对应小节；第 192 行
- Current behavior: 文件没有旅程套件命令、多语句 JSONL、`schema`/`dump` 长类型名、DDL 语句边界、二级索引与全表扫描一致。第 192 行写超长 `DEFAULT` 按页满错误拒绝。
- Required behavior: 文件写明这五条，并与 `README.md` 的对应段落一致。1. `cargo test --test e2e_test -- --test-threads=1` 是旅程套件。2. 多语句 `--format json` 是每条语句一行 JSON；成功的 DDL 输出 `{"affected_rows":0}`。3. `schema`/`dump` 渲染 `INTEGER`/`TEXT`/`BOOLEAN`。4. 成功的 `CREATE TABLE`/`CREATE INDEX`/`CREATE VIEW`/`ALTER TABLE ADD COLUMN`/`CREATE TABLE AS SELECT` 在进程被杀死后重开仍在；`DROP` 与加密库 WAL 明文窗口仍是边界。5. 二级索引等值查询与全表扫描行集一致。第 192 行改为计划期拒绝，并点名 `DEFAULT value is too large for the catalog page capacity`。
- Preserve: 不改产品代码，不改 README。
- Forbidden: 把 `Sort` 包住的 `IN` 子查询写成已经可用；把未实现的能力写进去。
- First test: 无产品代码。先读第 192 行和 Behavioral notes，再按下面五条改文件。
- GREEN condition: 五条都能在文件里定位，第 192 行不再出现 page-full。
- Verification: 人工。对照 `README.md` 的同主题段落，在 Act Response 里逐条写出技能文档中的落点小节。命令输出采信 Iteration 004 Act Response 的六组实测，不重跑。
- Stop when: README 与技能文档对同一行为的说法冲突。不要改 README 去迁就，带回 Plan。

**Invariants**

- 先看到主键等值 `IN` 用例失败，再加匹配臂。
- 技能文档只写 README 已经陈述、并且 Iteration 004 命令实测过的行为。

**Non-goals**

见 Excluded scope。

**Acceptance**

- R8：四个场景由 `subquery_test` 新用例覆盖。
- R7 剩余：技能文档五条与第 192 行。
- RTM：R8 → T8 → `subquery_test`（Covered）；R7 剩余 → T9 → 文件核对（Covered）。

**Verification**

`IN` 场景走库 API 测试，断言行集和错误文案。文档核对读文件。全量在 T9 完成后按名字典序分批一次，各批 0 failed。

**Gate 2 Readiness**

PASS——范围由用户 2026-10-14 决定。首列规则与 `get_plan_output_columns` 的现有两套契约对齐，无 TBD。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- 内层带 `ORDER BY` 时计划根是 `Sort`，本轮修完后仍会报多列。不要在测试里把这种形态写成成功。

## Act Response

- Status: reported（2026-10-14）

### 实际改动

**T8（R8）**——`src/parser/planner/subquery.rs::get_subquery_first_column` 增两臂（DerivedScan 臂之后、JOIN 臂与兜底之前）：

- `PhysicalPlan::IndexScan`：空 `columns` 维持 `SubqueryReturnsMultipleColumns`；否则取 `columns[0]`、`table` 用 `node.table_name`（`columns` 构造期已收窄为 SELECT 清单形状，契约同 `get_plan_output_columns` IndexScan 臂）。
- `PhysicalPlan::IndexScanAll`：`projected_columns(&node.columns, &node.projection)` 得输出列名后取首项（投影非空不回退基表第一列）；空清单维持多列错误。为支撑复用，`src/parser/planner/query.rs` 的 `projected_columns` 可见性 `fn` → `pub(crate)`（实现本体零变化，保证两消费方规则逐字节同源）。

**T9（R7 剩余）**——`rtsql-docs/SKILL.md` 六处精准编辑：

1. `Install and deploy`（"Build without installing" 之后）：e2e 旅程套件命令与定位（逐句对齐 README.md 对应段）。
2. `Select output and handle failures`：多语句 `--format json` 为 JSONL（每语句一行独立 JSON 对象）、成功 DDL 输出 `{"affected_rows":0}`。
3. 长类型名三处：`schema` 命令表条目、`Create and inspect` 段（schema 渲染标准名）、`Dump and restore` 段（导出 DDL 同名渲染，dump→restore 往返复现声明）。
4. `Behavioral notes`：DDL 语句边界落盘（`__tables` 行最后落盘为 commit point；被杀后重开 schema 仍在、WAL 重放恢复行）；剩余崩溃窗口 = DROP 类（checkpoint-based 页释放）+ 加密库 WAL/checkpoint 明文 sidecar，建议 `check` 验证或 dump/restore 备份。
5. `Evolve schema`：`CREATE INDEX` 子弹补二级索引等值行集与全表扫描严格一致（任意规模、含重启与崩溃恢复）。
6. 原第 192 行：`DEFAULT` 超限由「页满错误」改为计划期点名拒绝，文案 `DEFAULT type mismatch: column '<name>': DEFAULT value is too large for the catalog page capacity`（与 README 及 `ddl_dml.rs:1174/1460` 实际消息一致），零副作用表述保留。

测试：`tests/subquery_test.rs` +5（r8_in_subquery_pk_equality_index_scan_first_column / r8_in_subquery_index_scan_non_first_column_output / r8_in_subquery_secondary_index_equality_projected_first_column / r8_in_subquery_qualified_pk_equality_unchanged / r8_in_subquery_join_rejection_unchanged），含 T8 机制说明节首注释。

### 与计划的偏差

1. `projected_columns` 提升为 `pub(crate)` 复用（Implementation Guidance 要求"按 projected_columns 的规则"——直接复用函数本体保证规则同源，避免内联副本漂移）。非实质。
2. e2e 套件陈述落点选 `Install and deploy`（源码检出验证语境），不在 Task Contract 列举的小节名单内；该名单以"或"连接且 Required behavior 只要求可定位+与 README 一致，此为非实质落点选择。
3. 长类型名陈述在 schema 命令表、`Create and inspect`、`Dump and restore` 三处落点（对齐 README 的分布风格），超出单点最低要求，均为同义短句。

### 自检发现与处理

- 新测试首版 `exec(db,` 缺借用致 E0308 编译错（测试代码笔误），修正后 RED 见证有效。已修复。
- `cargo fmt` 重排新增 import（rustfmt 排序规则）。已修复，fmt --check 干净。
- 既有 `r5_not_in_anti_join_order_by_non_output_column` 文档注释中"缺 IndexScan 臂为范围外缺陷"陈述随 T8 修复过期——更新为指向本 T8 用例矩阵的历史注记。非行为变化。
- clippy：`secondary_index_equality_test` 1 条 match→if-let 建议为 Iteration 001 存量告警（Iteration 004 Act 已记录"存量 1 条不变"），不在本 Cycle 范围，未处理。

遗留 Minor：无。

### 验证

- T8 先行测试 RED：`cargo test --test subquery_test r8_in_subquery_pk_equality_index_scan_first_column -- --test-threads=1` → FAILED，错误 `Plan error: Subquery returns multiple columns (IN subquery requires single column)`（与契约预测逐字一致，pre-fix 见证）。
- T8 GREEN：`cargo test --test subquery_test -- --test-threads=1` → **43 passed / 0 failed / 0 ignored**（基线 38 + 本 Cycle 5）。
- T9 人工核对：六处落点逐条读回，与 `README.md` 对应段（:176/:236/:263/:285/:329 及 DDL 边界段 :109）同义；`grep page-full rtsql-docs/SKILL.md` 零匹配（rc=1）。
- Cycle 全量（按名字典序分批一次）：批次 1–6 共 120 集成目标 + lib，各批 0 failed；e2e 套件串行 `cargo test --test e2e_test -- --test-threads=1` → **22 passed / 0 failed / 0 ignored**（70.68s）；lib **398 passed / 0 failed**。全量合计 1994 tests / 0 failed / 2 ignored（1989 基线 + 本 Cycle 净增 5；2 ignored 为 cli_test 存量）。
- `cargo fmt --check` 干净；`cargo clippy --all-targets` 无新增告警（存量 1 条如上）。

### Persisted Evidence

None required（Plan Context Mode: none）。

### Experience Candidates

None。

### 未解决问题

无。Plan Context「Risks and Notes」提示的形态（内层带 `ORDER BY` 时计划根为 `Sort`、仍报多列）维持既有边界，新测试未将该形态写成成功，符合 Excluded scope。

## Plan Review

- Review Result: accepted（2026-10-14，Plan 独立审计）

**Findings**

独立阅读实际代码与断言，核对 Act Response 声明与代码一致（验证结论采信来源与新鲜度见 Evidence）：

1.〔Act 偏离·非实质〕`projected_columns`（`src/parser/planner/query.rs:43`）可见性 `fn` → `pub(crate)`：函数本体为纯裁剪 helper（projection 空恒等 / 非空按下标取列），实现零变化；`get_plan_output_columns` 的 Scan/DataScan/DerivedScan/IndexScanAll 四消费臂（`:56-58`/`:95`）与 T8 新臂共用同一函数，「规则逐字节同源」声明与代码一致。import 位于 `subquery.rs:8`。在 Implementation Guidance「按 projected_columns 的规则」 latitude 内，不阻塞。
2.〔Act 偏离·非实质〕SKILL.md e2e 陈述落点选 `Install and deploy`（`:107`），不在 Task Contract 列举的小节名单内；该名单以「或」连接且 Required behavior 只要求可定位 + 与 README 一致，落点有效。
3.〔非阻塞观察〕spec delta 场景 2 的夹具为 `dept`/`region`，实际测试 `r8_in_subquery_index_scan_non_first_column_output` 用自建 `item`/`sku` 表——语义同型（IndexScan 收窄后取 `columns[0]` 而非基表第一列），Requirement 行为描述与测试一致，无需动作。
4.〔非阻塞·存量〕clippy 1 条（`secondary_index_equality_test` match→if-let）为 Iteration 001 存量告警，两轮 Act 均记录不变，不在本 Cycle 范围。

**Acceptance Gaps**

无。R8 四场景由 `tests/subquery_test.rs` 五个新用例覆盖（S1 裸名主键等值 / S2 非首列输出 / S3 二级索引投影首列 + 对照 / S4 限定名不变 / S5 JOIN 拒绝文案不变，`:998-1113`）；R7 剩余五条 + 第 192 行改写全部在 SKILL.md 定位（`:107`/`:401`/`:154`/`:276`/`:311`/`:197`/`:292`/`:196`）。

**Evidence**

- 代码：`subquery.rs:703-729`（IndexScan/IndexScanAll 两臂，空清单维持 `SubqueryReturnsMultipleColumns`，JOIN 臂 `:689-691` 与兜底 `:730` 未动）；`query.rs:43-51`（`pub(crate) projected_columns` 本体不变）。
- 测试：`tests/subquery_test.rs:998-1113` 五个 r8 用例与 spec delta（`specs/in-subquery-join-rejection/spec.md`）场景一一对应；既有 r5 用例注释更新（`:947-952`）与 Act Self-Review 声明一致。
- 文档：SKILL.md 六处落点逐一读回；`grep -i "page.full\|页满" rtsql-docs/SKILL.md` 零匹配（rc=1）；DEFAULT 文案与 `ddl_dml.rs:1174/1460` 实际消息逐字一致；README ↔ SKILL.md 五条对应段（README `:329`/`:236`/`:109`/`:285`/`:176-177` vs SKILL `:107`/`:401`/`:197`/`:196`/`:154+276+311`）无冲突。
- 采信 Act Response 验证结论（mtimes 核实：全部代码文件 2026-10-14 15:33–15:40 早于 Cycle 文件 15:50，结论对当前代码状态有效，未重跑）：subquery_test 43 passed / 0 failed；e2e 22 passed / 0 failed / 0 ignored；全量 1994 tests / 0 failed / 2 ignored；fmt/clippy 无新增。

**Follow-up Decision**

Acceptance 全部满足，无阻塞 finding，偏差均为契约 latitude 内非实质选择 → accepted。Iteration 005 完成；change 全部六个 Iteration 均 accepted，无剩余 Iteration。

**Iteration Plan Update**

None。

**Next Cycle**

None。

**Next Iteration**

None（005 为 Map 中最后一个 Iteration；change 待收尾——7 域 spec delta 合并、SNAPSHOT/tasks 刷新、归档归 openspec-docs-maintainer，ISS05–ISS14 台账关闭归 openspec-experience-recorder）。
