# Iteration 002 / Cycle 000-initial: 表名解析归一化与 dump 保真（I039）

## Plan Context

- Status: ready
- Iteration: 002-table-name-normalization
- Cycle: 000-initial
- Cycle Type: initial
- Parent cycle: None

**Iteration Scope**

- Change tasks: T7, T8, T9
- Depends on: Iteration 001（accepted 2026-09-12，replan 链收口，见 `../001-update-key-index/001-replan.md` Plan Review；共享工作区与全量基线，无代码耦合）
- Stable baseline: 带引号与裸名拼写全语句等价（CREATE/DROP/SELECT/INSERT/UPDATE/DELETE）；dump→restore→dump DDL 表名恒等；裸名既有语义零回归（既有往返/生命周期/import 套件零修改通过）
- Verification boundary: T8/T9 全绿 + clippy/fmt 0 + `openspec validate --changes` PASS + CLI 探针
- Diagnostic boundary: `src/parser/ast.rs`（helper + 3 helper 体）+ 8 处内联替换点 + `tests/cli_test.rs` 追加用例
- Deferred tasks: None（Map 末 Iteration）

**Cycle Scope**

- Trigger: initial
- Acceptance gaps: None
- Repair items: None
- Inherited scope: change proposal 全部范围约束（默认假设 1：解析侧归一化方向）；design D3 归一化语义与边界
- Excluded scope: dump `quote_ident` 输出语义修改、历史带引号表名迁移、multipart/schema 限定名能力、I034/I037 面（已完成 Iteration）、性能

**Objective**

带引号标识符与裸名解析到同一表名（`CREATE TABLE "items"` 后 `SELECT/INSERT/UPDATE/DELETE/DROP` 经任一拼写命中）；`dump→restore→dump` 的 DDL 表名文本恒等（多代不膨胀）；`schema` 输出可重建同名表；裸名全链路与既有测试零回归。

**Background**

tasks MS15-T04 + improvements I039：MS10-T05 Iter001 Act Deviation 1 探针实证（2026-09-09）——引擎以 `ObjectName/Ident` 的 Display（含引号字符）为表名，dump 侧恒引号输出经 restore 后表名逐代膨胀（`"items"` → `"""items"""`）。用户 2026-09-12 批准方向 a（解析侧归一化，proposal 默认假设 1）。本 Iteration 为聚合 change 三域中的最后一域。

**Investigation Facts**

- Current Baseline: Iteration 001 最终 Act Response（001-replan）：全量 861 passed / 0 failed / 2 ignored（Plan Review 独立复跑采信）、clippy/fmt 0、validate PASS；Review Result `accepted`。工作区含 MS15-T01 + Iter000 + Iter001 未提交产物——Act 开始前 `git status`/`git diff` 基线检查；本 Iteration 代码面（ast.rs/pipeline.rs/subquery.rs/ddl_dml.rs）自规划调查后未被触碰（git status 复核 2026-09-12，覆盖范围未变化；query.rs 因 Iter000 实施行号偏移已复核刷新）。
- Current-State Evidence（2026-09-12 行号复核）:
  - Display 消费点共 **11 处**（grep `to_string().to_lowercase()` 过滤列名/别名面后实证；design D3 原「10 处」为计数勘误，已修订）：
    - helper 体 3 处：`src/parser/ast.rs:27`（`extract_table_name`，TableFactor 臂——`build_update` 等消费）、`src/parser/ast.rs:216-218`（`extract_name_from_object`，唯一调用点 `ddl_dml.rs:77`）、`src/parser/ast.rs:223`（`extract_join_table_name`，JOIN 右表）。
    - 内联 8 处：`src/parser/planner/query.rs:120`（SELECT 基表，Iter000 后行号）、`src/pipeline.rs:867`（INSERT 表名提取）、`src/pipeline.rs:958/971`（JOIN/子查询表注册提取）、`src/parser/planner/subquery.rs:136/140`（子查询表提取）、`src/parser/planner/ddl_dml.rs:305`（CREATE TABLE）、`src/parser/planner/ddl_dml.rs:348`（DROP TABLE）。
  - 类型事实：sqlparser 0.44 `pub struct ObjectName(pub Vec<Ident>)`（registry 源码实证）；`Ident.value: String` 为去引号内容（带引号时 `quote_style` 另存）——`CREATE TABLE "items"` 的 Display 为 `"items"`（含引号 7 字符），`.value` 为 `items`；`"""items"""` 的 `.value` 为 `"items"`（`""`→`"` 转义已由 sqlparser 解析）。列名消费已用 `.value`（`ast.rs:42/45/136-140`、`ddl_dml.rs` 列定义与 `assignment.id[0].value`）——表名对齐列名先例。
  - 表名存储/查找链：上述 11 处产出的名字符串 → `TableManager.get_table`/catalog 键（全链 lowercase）——helper 归一化后带引号与裸名自然汇聚同一键。
  - dump/schema 侧（不变面）：`create_table_sql`（`src/cli/lifecycle.rs:532-558`）以 `quote_ident`（`:560-562`，恒引号 + `"`→`""` 转义）渲染 catalog 表名；INSERT 行同理。归一化后 catalog 名不含引号字符（新库），输出即安全传输形态；`dump→restore→dump` 恒等由「restore 归一化去引号」保证。
  - 历史带引号表名（旧 restore 产物，catalog 名含引号字符）：dump 输出 `"""items"""` 形态 → restore 归一化得 `"items"`（转义语义）→ 恒等不继续膨胀；经值为 `"items"` 的拼写可达（如 `"""items"""`），经裸名不可达（design D3 边界，预发布可接受）。
  - 既有测试入口：`tests/cli_test.rs`（`fixture`/`run_cli` helper；`test_dump_restore_roundtrip_full_shape` `:1359` 锁定一代往返与 dump 文本含 `INSERT INTO "mixed"`——dump 侧不变故应零修改通过）、生命周期/import 套件。
- Code and Critical Path: 11 处表名字符串产出点 → `validate_table`/`self.tables` 注册 → `TableManager` 查找/catalog；`build_plan` 前置的 `TxStatementKind` 分类与 plan cache 键（SQL 文本规范化）不触表名字符串面。

**Implementation Guidance**

实现顺序：T7 先写 RED（互访 + 多代恒等 + schema 保真），确认 RED 形态后 T8 实施 helper + 替换，T9 回归收尾。T8 形态建议（非实质细节可就地调整）：`ast.rs` 新增

```rust
/// MS15-Rest (I039): resolve a table name from its ObjectName by the
/// identifiers' unquoted values (quote_style-insensitive), lowercased —
/// quoted and bare spellings resolve to the same table. Multi-part names
/// join with '.', matching the previous Display shape.
pub fn object_name_to_table_name(name: &ObjectName) -> String {
    name.0.iter().map(|id| id.value.to_lowercase()).collect::<Vec<_>>().join(".")
}
```

3 个既有 helper（`extract_table_name`/`extract_name_from_object`/`extract_join_table_name`）函数体改调该 helper（签名不变，调用点零改动）；8 处内联 `X.to_string().to_lowercase()` 改 `object_name_to_table_name(X)`（`ddl_dml.rs:348` 的 `names[0]` 同型）。注意 `Ident` 类型导入（ast.rs 已有 sqlparser 引用面）。dump 侧 `lifecycle.rs` 零修改。

**Behavioral Change**

- 当前：`CREATE TABLE "items"` 存表名 `"items"`（含引号字符）——`SELECT * FROM items` 报表不存在；dump 该表输出 `"""items"""` 逐代膨胀。
- 目标：两拼写归一 `items` 全语句等价；dump→restore→dump DDL 表名文本恒等。
- 接口：`extract_table_name`/`extract_name_from_object`/`extract_join_table_name` 签名不变、返回值语义收窄（去引号）；新增 pub helper（crate 内使用）。
- 错误语义：无新增错误路径；带引号建表后的互访从「表不存在」错误变为成功（缺陷修复本体）；历史带引号表名经裸名访问从「命中」变为「不存在」（design D3 边界——预发布无兼容承诺，该形态仅来自旧 restore 产物）。

**Task Contracts**

### T7: RED 测试见证——R3 缺陷形态与既有往返锚点

- Requirement/Scenario: R3（delta spec `specs/table-name-resolution/spec.md`）R1 S「带引号建表后裸名访问命中」、S「带引号与裸名拼写等价互访」、S「UPDATE/DELETE 表名解析归一化」、R2 S「dump-restore-dump 表名恒等」、S「schema 输出可重建同名表」
- Depends on: None
- Targets: `tests/cli_test.rs`（追加测试，沿用 `fixture`/`run_cli` helper；MS15-Rest Iteration 002 节）
- Current behavior: `CREATE TABLE "items"` 后 `INSERT INTO items` / `SELECT ... FROM items` 报表不存在；dump→restore→dump 二代 DDL 表名 `"""items"""` 膨胀；schema 输出的 DDL 重建出带引号名表
- Required behavior: 测试断言目标行为（互访成功 / 多代 dump 表名行文本恒等 / schema 重建同名）；修复前 RED
- Required changes: 仅新增测试；不修改产品代码与既有测试
- Preserve: 既有断言与意图零修改（`test_dump_restore_roundtrip_full_shape` 等既有用例不得触碰）
- Forbidden: 修改 `src/`；修改既有测试
- Test witness: `cargo test --test cli_test`，新增用例 RED（互访报表不存在 / 二代表名膨胀——断言失败形态与 delta spec THEN 的修复前描述一致）
- GREEN condition: T8 后全部转 GREEN
- Verification: `cargo test --test cli_test <new_test_names>`，退出码 0
- Stop when: RED 形态与缺陷记录不符（如互访已成功——基线与 I039 记录矛盾，返回 Plan）

### T8: R3 实现——表名解析归一化（design D3）

- Requirement/Scenario: R3 全部场景
- Depends on: T7
- Targets: `src/parser/ast.rs`（新增 `object_name_to_table_name` + `extract_table_name:27`/`extract_name_from_object:216-218`/`extract_join_table_name:223` 三函数体）；8 处内联替换：`query.rs:120`、`pipeline.rs:867/958/971`、`subquery.rs:136/140`、`ddl_dml.rs:305/348`
- Current behavior: 11 处以 Display（含引号字符）+ lowercase 为表名
- Required behavior: 11 处统一经 helper（`Ident.value` + lowercase + `.` 连接）；函数签名与调用点零变化；`lifecycle.rs`/dump 侧零修改
- Required changes: 见 Targets；无其他行改动
- Preserve: 列名/别名消费面（已用 `.value`，不动）；plan cache 键、`TxStatementKind` 分类、`validate_table` 语义；表名 lowercase 行为；multipart `.` 连接形态
- Forbidden: 修改 `lifecycle.rs`、catalog/存储层、WAL/恢复、plan 路由（I036/I046 域）
- Test witness: T7 用例转 GREEN；`tests/keyless_row_test.rs`/`keyless_eq_routing_test.rs`/`cli_test.rs` 既有用例零修改通过
- GREEN condition: `cargo test --test cli_test` 全绿（含既有 59+新增）
- Verification: 同上 + `cargo test --test parser_test --test planner_test`（解析/规划回归）
- Stop when: 替换后任一既有路径出现表名不匹配（除 delta spec R3 预期变化面外）——实质，返回 Plan

### T9: R3 回归与 change 全量收尾

- Requirement/Scenario: R3 S「裸名全链路与既有测试零回归」+ change 级收尾
- Depends on: T8
- Targets: 全量验证命令 + CLI 探针（无代码修改）
- Current behavior: —
- Required behavior: 全量 861+新增 全绿；clippy/fmt 0；validate PASS；三缺陷形态探针复核（I034 CLI 表头 / I037 键位清理 / I039 引号归一）
- Required changes: 无代码修改
- Preserve: —
- Forbidden: 为凑绿修改既有测试
- Test witness: `cargo test --no-fail-fast`（全量）、`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate --changes`；CLI 探针：`CREATE TABLE "items"` → `SELECT * FROM items` 与 `SELECT * FROM "items"` 均成功、dump→restore→dump 表名行恒等
- GREEN condition: 全部退出码 0；基线 861 只增不减
- Verification: 同上
- Stop when: 全量出现无法归因于本变更面的失败（返回 Plan；已知 I041 偶发 env 竞态除外——重跑即绿按 I041 记录处置并注明）

**Invariants**

- 表名 lowercase 不变；plan cache 键语义不变；catalog/存储格式不变；dump `quote_ident` 输出不变；恢复重放语义不变；列名/别名解析面不变。

**Non-goals**

- dump `quote_ident` 语义；历史带引号表名迁移与可达性恢复；multipart/schema 限定名；rekey/I034/I037 面；性能。

**Acceptance**

R3 delta spec 场景全部满足且既有套件零回归。RTM（Iteration 002 范围）：

| Requirement | Scenario | Design | Task | Iteration | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| R1 | 带引号建表后裸名访问命中 | D3 | T7/T8 | 002 | 11 处替换点 | `cli_test.rs` 新增互访用例 | None | Covered |
| R1 | 带引号与裸名拼写等价互访 | D3 | T7/T8 | 002 | 同上 | 同上（SELECT/INSERT/UPDATE/DELETE/DROP） | None | Covered |
| R1 | 引号转义按标识符语义解析 | D3 | T8/T9 | 002 | helper（`Ident.value`） | 探针 + 单测（如适用） | None | Covered |
| R1 | UPDATE/DELETE 表名解析归一化 | D3 | T7/T8 | 002 | `ast.rs:27`/`ddl_dml.rs:348` 等 | `cli_test.rs` 新增 | None | Covered |
| R2 | dump-restore-dump 表名恒等 | D3 | T7/T8 | 002 | 11 处替换点（restore 面经 CREATE 归一化） | `cli_test.rs` 新增多代用例 | None | Covered |
| R2 | schema 输出可重建同名表 | D3 | T7/T8 | 002 | schema 命令（经 helper 链） | `cli_test.rs` 新增 | None | Covered |
| R3 | 裸名全链路与既有测试零回归 | D3 | T8/T9 | 002 | 全部（裸名 `Ident.value`==Display 去引号同值） | 既有往返/生命周期/import 套件 + 全量 | None | Covered |

**Verification**

- `cargo test --no-fail-fast`（全量，基线 861 只增不减）、`cargo clippy --all-targets -- -D warnings`（0）、`cargo fmt --check`（0）、`openspec validate --changes`（PASS）。
- CLI 探针（决定性输出记入 Act Response，≤20 行）：引号建表互访、`dump a` → restore → `dump b` 的 CREATE TABLE 行 diff 为空、`schema` 重建同名。
- Persisted Evidence 为 none（全量可低成本重跑）。

**Gate 2 Readiness**

- 无 Missing requirement：PASS（RTM 全 Covered，R3 delta spec 8 场景均有 task/代码/测试映射）
- Simplified requirement 已批准：PASS（无 Simplified 项）
- 调查完整：PASS（11 处替换点 2026-09-12 行号复核 + ObjectName/Ident 类型实证 + dump 链与历史名边界论证；代码面覆盖范围自调查后未变化——git status 复核）
- 设计闭合：PASS（D3 归一化语义、边界、dump 不变面明确）
- 任务可执行：PASS（T7-T9 均有位置/行为/见证/停止条件）
- 分轮合理：PASS（单域：解析 helper + 机械替换 + CLI 测试；诊断边界单一）
- 追踪完整：PASS（RTM 链路闭合）
- 验证充分：PASS（RED→GREEN + 既有套件零修改 + 全量收尾 + 探针）
- 无身份型证据工程：PASS
- 无实质未知项/TBD：PASS（历史带引号表名可达性已定性为 design D3 边界，非未知项）
- OpenSpec 产物一致：PASS（proposal/design D3（已修订 11 处计数）/tasks/delta spec/cycle 交叉一致）
- Persisted Evidence 模式明确：PASS（none）
- 用户批准计划：PASS（2026-09-12 用户"批准"覆盖本 change Iteration Map 与 T7-T9 范围（Gate 1/2 change 级）；本 Cycle 契约为该范围内实现细节的展开，无范围/验收变化——先例：000-initial 与 001 000-initial 同款）

**Persisted Evidence**

- Mode: none

Budget: 不适用。

**Risks and Notes**

- 带引号 DDL 在引擎内的既存形态（测试/夹具）：若既有测试或夹具含带引号建表并依赖 Display 名（grep 排查 `CREATE TABLE "` 用例），其表名语义随 R3 变化——预期无（MS10-T05 夹具规避了引号名），若发现按「既有测试按语义校准」先例处理并记 Act Response；主 spec `cli-noninteractive-shell` 的 schema/dump 场景均用裸名，无冲突面。
- 全量已知偶发 I041——失败先重跑再判定。
- 工作区未提交产物累积（MS15-T01 + 本 change 三 Iteration）——本 Iteration 收尾后 change 实施侧完成，建议用户 commit 后由 docs-maintainer 收尾（I034/I037/I039 转 promoted、遗留 I 项候选登记、SNAPSHOT/tasks 同步）。

## Act Response

- Status: blocked

**Implemented**

- T7 completed：`tests/cli_test.rs` 追加 5 用例（MS15-Rest Iteration 002 节）——带引号建表裸名互访、五语句带引号/裸名等价互访、裸名 UPDATE/DELETE 命中带引号建表、dump→restore→dump 两代 CREATE 行恒等、schema DDL 重建同名表。修复前观察 RED：5 failed / 0 passed，形态与缺陷记录逐字一致（`Table '"items"' not found` / 二代 `CREATE TABLE """mixed"""` 膨胀 / schema 路径裸名报表不存在）。
- T8 completed：11 处表名消费点全部经 `object_name_to_table_name` 归一化（`Ident.value` + lowercase + `.` 连接）；`extract_table_name`/`extract_name_from_object`/`extract_join_table_name` 签名与调用点零变化；T7 用例全转 GREEN（cli_test 64 passed 含既有 59 零修改、parser_test 6、planner_test 36）。
- T9 部分完成：全量/clippy/fmt/validate/探针 P1-P4 全过（见 Verification Evidence）；P5 转义名 dump 探针失败 → Gate 6 阻塞（见 Blocker Handoff）。

**Changed Files and Symbols**

- `src/parser/ast.rs`：新增 pub `object_name_to_table_name(&ObjectName) -> String`；`extract_table_name`/`extract_name_from_object`/`extract_join_table_name` 函数体改调该 helper（签名不变）。
- `src/parser/planner/query.rs:120`（基表提取）、`src/parser/planner/subquery.rs:136/140`（子查询表名）、`src/parser/planner/ddl_dml.rs:305`（build_create_table）/`:348`（build_drop_table）、`src/pipeline.rs:867`（extract_all_table_names Insert 臂）/`:958/971`（主表与 JOIN 臂）——8 处内联 `X.to_string().to_lowercase()` 替换。
- `tests/cli_test.rs`：新增 5 测试（Iteration 002 节）。
- `cargo fmt` 重排 pipeline.rs:867 与 cli_test.rs 两处本次新增超长行（语义不变）。

**Deviations from Plan**

1. T7 等价互访用例 UPDATE/DELETE 语句形态调整（先例：本 change Iter000 COALESCE 等价调整）：delta spec S2/S4 示例语句 `UPDATE … SET id = 2 WHERE id = 1` + 同键 `DELETE … WHERE id = 2` 组合踏入 proposal 默认假设 2 明确排除的 I037 邻接 rekey 缺陷区——探针实证：同键 rekey 后新键不可达（`SELECT WHERE id=2` 空集）、同键跨进程 UPDATE→DELETE 两步变更丢失（见 Remaining Issues 2）。改用非键列 SET（`SET n = 99/101`）+ 删除未被更新触及的键 / 先 DELETE 后 UPDATE 序（探针 Z2 验证形态），锁定同一被测属性（拼写命中同一表）；RED 形态不受影响（修复前互访报表不存在先于行级语义）。
2. schema 用例 b 库断言修正：Plan Context 未指明 b 库数据态——b 只应用 DDL 未插数据，行集断言应为空集；Act 首次运行发现测试自身疏漏即改（a 断言 [[10]]、b 断言 []，两库均以裸名命中证明同名重建）。
3. fmt 对本次新增两处超长行的机械重排（pipeline.rs:867 match 臂、cli_test.rs 断言），契约内等价。

**Blocker Handoff**

- 发现位置：T9 / CLI 探针 P5（RTM R1「引号转义按标识符语义解析」见证，Gate 5 验证）。
- Plan 预期：Plan Context Investigation Facts/Risks 断言——「历史带引号表名（catalog 名含引号字符）：dump 输出 `"""items"""` 形态 → restore 归一化得 `"items"` → 恒等不继续膨胀；经值为 `"items"` 的拼写可达（如 `"""items"""`）」，即 dump 对该形态可用且多代恒等。
- 实际：`CREATE TABLE """items""" (id INT)` 成功（catalog 名 = `"items"`，值语义符合 spec R1-S3 表名断言）；随后 `dump` 报错 `failed to dump table "items": Table 'items' not found: table not found: items`。根因：`select_all_rows`（`src/cli/lifecycle.rs:216`）以 `format!("SELECT * FROM {}", table)` 裸插值构造行扫描 SQL——归一化后 `SELECT * FROM "items"` 解析为 `items` ≠ catalog 名 `"items"`。改动前 Display 名使裸插值凑巧可解析（dump 可用、仅输出膨胀）；本 change 使其变为报错——dump 对含引号字符表名（含历史 restore 产物）功能回归，spec R1-S3「多代 dump/restore 对该名 SHALL 恒等不继续膨胀」不可满足（其前提是 dump 可用）。
- 影响：R1-S3 acceptance gap；Plan Context 调查事实「归一化后 catalog 名不含引号字符（新库）」对转义形态不成立；T8 契约 Forbidden「修改 lifecycle.rs」与修复面冲突。
- 修复方向（供 Plan 裁定，Act 未实施）：`select_all_rows` 行扫描 SQL 改用既有 `quote_ident(table)` 包裹——与 dump CREATE/INSERT 输出面同一 helper（`src/cli/lifecycle.rs:560-562`），一行改动，dump 输出语义不变面保持；需 Plan 修订当前 Cycle 契约（Forbidden 解禁一处）或调整 S3 范围。
- 任务状态：T7 completed、T8 completed、T9 blocked（其余验证项全过）。工作区未提交（沿用默认假设 3）；已通过 Gate 3/4 与本 Cycle 其余 Gate 5 验证。
- Evidence：None required（单命令可复现：`rtsql e 'CREATE TABLE """items""" (id INT)'` 后 `rtsql dump e`）。
- 恢复条件：用户裁定后经 openspec-plan Review 当前 Cycle（修订契约/范围），Act 于当前 Cycle 继续。

**Blocker Resolution**

None

**Self-Review**

- Spec review：11 处替换点与 Task Contract 逐一对应；helper 形态与 Plan 建议一致；签名/调用点零变化；Forbidden 面（lifecycle.rs、catalog/存储层、WAL/恢复、plan 路由）未触碰；既有 `.value` 列名/别名消费面零修改；T7 RED→GREEN 证明目标行为；`test_dump_restore_roundtrip_full_shape` 等既有用例零修改通过（带引号 SELECT 比对经归一化仍可达同名表）。
- Code review：全量 diff 无计划外修改；无新增警告（clippy 0）；无死代码/重复实现；无身份型证据机制。
- 已修复发现：Deviation 1（rekey 组合依赖）、Deviation 2（schema b 库断言）——均为测试自身问题，修复后重跑受影响验证。
- 遗留 Minor：F1 helper 无独立单元测试（覆盖经 cli_test e2e 与探针，RTM 对应行「如适用」）；F2 等价用例语句形态与 delta spec 示例字面不同（Deviation 1，待 Plan Review 追认；如 Plan 追认，建议随收尾对 spec S2/S4 示例语句做同款校准）。

**Verification Evidence**

| 验证项 | 命令或操作 | 输出摘录 | 覆盖范围 | 结论 |
|---|---|---|---|---|
| 全量测试 | `cargo test --no-fail-fast` | 866 passed / 0 failed / 2 ignored（68 套件 0 FAILED；I041 `test_db_dir_env_cases` 偶发失败，重跑 3 passed 即绿，按 I041 记录处置） | 全仓（基线 861 + 5 新增） | PASS |
| 目标套件 | `cargo test --test cli_test` / `--test parser_test` / `--test planner_test` | 64 / 6 / 36 passed | R3 面 + 解析规划回归 | PASS |
| 静态检查 | `cargo clippy --all-targets -- -D warnings`；`cargo fmt --check` | 0 warning；0 diff（exit 0） | 全仓 | PASS |
| OpenSpec | `openspec validate --changes` | Totals: 1 passed, 0 failed | change 产物 | PASS |
| 探针 P1（I034） | `SELECT name FROM s` | `{"columns":["name"],"rows":[["Alice"]]}` 字段数一致 | 表头/行形状 | PASS |
| 探针 P2（I037） | 键→NULL 后旧键 INSERT + 点查 | INSERT affected 1（无误拒）；点查 `[[7,2]]` | 键位索引清理 | PASS |
| 探针 P3（I039） | 带引号建表后 `SELECT * FROM items` 与 `SELECT * FROM "items"` | 均 `[[1,10]]` | 引号互访 | PASS |
| 探针 P4（I039） | `dump a` → restore → `dump b` CREATE 行 diff | 空 diff（`CREATE LINES IDENTICAL`） | 裸名多代 dump 恒等 | PASS |
| 探针 P5（R1-S3） | `CREATE TABLE """items""" (id INT)` 后 `dump` | `Table 'items' not found`（详见 Blocker Handoff） | 转义名 dump | FAIL → 阻塞 |

**Persisted Evidence**

None required（Mode: none；全部验证可低成本重跑，探针单命令可复现）。

**Experience Candidates**

None（Remaining Issues 2 的预存缺陷为 improvement 候选，非 Runbook/Incident；未发生满足 Runbook/Incident 白名单的运维过程或故障）。

**Remaining Issues**

1. Blocker Handoff（lifecycle.rs `select_all_rows` 裸插值 × spec R1-S3）——本 Response 唯一阻塞项。
2. 预存引擎缺陷候选（本 change 范围外，建议 docs-maintainer 登记 I 项）：**跨进程同键 UPDATE→DELETE 变更丢失**——`INSERT (1,10)` → 进程 A `UPDATE SET n=99 WHERE id=1`（affected 1）→ 进程 B `DELETE WHERE id=1`（affected 1）→ 进程 C 扫描见原始 `[[1,10]]`、点查 `id=1` 空集（两视图不一致，两步变更从扫描面消失）；对照：异键 update→delete（Z1）、delete→update（Z2）均正常，全裸名序列同样复现（预存缺陷，与本 change 表名面无关，亦未被既有 861 测试覆盖）。缺陷域猜测（供后续调查）：版本链 / checkpoint / recovery 的同键组合面。
3. Minor F1/F2（见 Self-Review）。
4. 工作区未提交产物继续累积（MS15-T01 + 本 change Iter 000/001/002）——commit 决定权在用户（默认假设 3）。

**Commit or Diff Reference**

未提交。本 Cycle diff = 上述 6 文件；工作树含 MS15-T01 与 Iter 000/001 未提交产物，对照基线 f9e1e1f。

## Plan Review

- Review Result: rework-required

**Findings**

独立检查（diff 逐文件核对、lifecycle.rs 现场、探针独立复现，2026-09-12）与 Act Response 交叉核对：

1. **Blocker 成立（阻塞 Acceptance，本 Review 唯一阻塞项）**。Plan 独立复现 P5：`CREATE TABLE """items""" (id INT)` + `INSERT INTO """items""" VALUES (1)` 后 `dump` 正常输出 DDL 行 `CREATE TABLE """items""" ("id" INT PRIMARY KEY);`，随即报 `failed to dump table "items": Table 'items' not found: table not found: items`（exit 1）；裸名对照组 dump 正常。根因与 Act 判定一致：`select_all_rows`（`src/cli/lifecycle.rs:207`）裸插值构造行扫描 SQL，R3 归一化后 `SELECT * FROM "items"` 解析为 `items` ≠ catalog 名 `"items"`。函数头注释（`lifecycle.rs:199-203`）记录的裸插值理由（「quote_ident 反而会给裸名表附加引号致查表失败」）是归一化前 Display 语义下的旧事实，归一化后两种拼写汇聚同名、理由失效——Plan 调查遗漏该注释与转义形态 catalog 名的组合。影响：R1-S3「多代 dump/restore 恒等」前提「dump 可用」不成立；相对 change 前（dump 可用、仅输出膨胀）构成功能回归，必须修复后本 Cycle 方可 accepted。
2. **Deviation 1（T7 等价互访用例语句形态调整）：追认，非阻塞**。原 delta spec S2/S4 示例语句 `SET id = 2`（同键 rekey）踏入 proposal 默认假设 2 明确排除的 I037 邻接缺陷区——示例语句是 Plan 起草缺陷，Act 改用非键列 SET + 删未触及键的形态锁定同一被测属性（拼写命中同一表），处置正确。本 Review 已同步校准 delta spec S2/S4 示例语句（见 Evidence），F2 就此关闭。
3. **Deviation 2（schema 用例 b 库空集断言）：追认，非阻塞**。Plan Context 未指明 b 库数据态，Act 就地修正测试自身疏漏并记录，属 Act 可处理的非实质局部差异。
4. **Deviation 3（fmt 机械重排两处超长行）：非问题**，契约内等价。
5. **F1（helper 无独立单元测试）：Minor，不阻塞**。RTM 对应行为「探针 + 单测（如适用）」，helper 经 cli_test e2e 全语句面覆盖，不强制补单测。
6. **Remaining Issues 2（跨进程同键 UPDATE→DELETE 变更丢失）：NEW-EVIDENCE，确认成立，非阻塞本 Iteration**。Plan 独立复现：`INSERT (1,10)` → 进程 A `UPDATE SET n=99 WHERE id=1`（affected 1）→ 进程 B `DELETE WHERE id=1`（affected 1）→ 进程 C 扫描 `[[1,10]]`、点查 `id=1` 空集；对照 Z1（异键 update→delete）、Z2（delete→update）均正常（`[[1,99]]`）。与 Act 报告逐字一致。属预存引擎缺陷（版本链/checkpoint/恢复的同键组合面），在本 change 范围外——收尾时由 docs-maintanner 登记 I 项（附本探针复现），与 proposal 默认假设 2 的 I037 邻接形态同批。
7. **import 边界观察（非阻塞）**：`import` 的 `INSERT INTO {实参}`（`lifecycle.rs:467`）用 CLI 实参原文插值——历史带引号表名（catalog 名含引号字符）经 import 实参约定不可达（实参比对与解析文本双重转义矛盾）。无 requirement 覆盖 import 对该形态的面（R2 仅 dump/schema），归入 design D3「历史带引号表名可达性收缩」边界，收尾登记时一并记录，不扩大修复面。
8. **工作区与验证采信**：`git status`/`git diff` 复核与 Act Response「Changed Files」一致（Iter 002 面：ast.rs/query.rs/subquery.rs/ddl_dml.rs/pipeline.rs/cli_test.rs，无计划外文件；11 处替换点与 T8 契约逐字一致）；T7/T8 的 866/0/2、clippy/fmt/validate 与探针 P1-P4 结论产自同一工作树、覆盖范围未变化，本 Review 采信（来源：本文件 Act Response Verification Evidence）；Plan 补跑的检查为 P5 复现、Remaining Issues 2 复现 + Z1/Z2 对照、裸名 dump 对照（Findings 1/6）。

**Deviation Classification**

- Blocker（P5 转义名 dump 回归）：PLAN-OMISSION——调查遗漏 `select_all_rows` 旧注释语义与转义形态 catalog 名的组合。阻塞，转入 rework。
- Deviation 1：PLAN-INVALID——Plan 起草的 spec 示例语句与 change 自身排除项（默认假设 2）冲突；Act 修正正确。非阻塞。
- Deviation 2：ACT-DEVIATION——测试自身疏漏就地修正。非阻塞。
- Deviation 3：None——机械重排。
- Remaining Issues 2：NEW-EVIDENCE——预存引擎缺陷，范围外。非阻塞。

**Acceptance Gaps**

- R1-S3「引号转义按标识符语义解析」THEN 的「多代 dump/restore 对该名 SHALL 恒等不继续膨胀」不可满足：dump 对转义名 catalog 报错（Finding 1 探针证据）。其余场景（R1 互访/S2/S4、R2 裸名恒等与 schema 同名、R3 零回归）已由 T7/T8 见证与全量覆盖。

**Convergence**

N/A（本 Iteration 首次 Review，无父 Cycle 比较项）

**Evidence**

- 代码与 diff：`git diff src/parser/ast.rs`（helper + 3 函数体，签名与调用点零变化）；`src/parser/planner/query.rs`（基表提取 + I034 臂 + MS15-T01 内容）、`subquery.rs`/`ddl_dml.rs`/`pipeline.rs`（8 处内联替换）与 T8 契约逐字一致；`src/cli/lifecycle.rs:199-207`（select_all_rows 裸插值 + 失效注释）、`:178-181`（dump INSERT 输出已用 quote_ident）、`:560-562`（quote_ident 定义）。
- 探针（2026-09-12 Plan 独立运行，`target/debug/rtsql`，工作树未变）：P5 复现（Finding 1）；Remaining Issues 2 复现 + Z1/Z2 对照（Finding 6）；裸名 dump 对照正常。
- 采信结论：全量 866/0/2、clippy/fmt/validate、探针 P1-P4（来源：本文件 Act Response，同一工作树覆盖未变化）。

**Follow-up Decision**

Blocker 修复需要新的执行契约：T9 契约为「无代码修改」、T8 契约 Forbidden「修改 lifecycle.rs」，而修复面正是 `lifecycle.rs::select_all_rows`（一行 SQL 构造 + 失效注释改写）加一条持久测试见证（cli_test 转义名 dump 往返用例，Gate 3 TDD 见证）。既有 Task Contract 不足以约束该修复——按公共规则「需要新执行契约时才创建 rework Cycle」与 iteration-planning Review 分类第 2 行，判 `rework-required`：创建同 Iteration 后继 Cycle `001-rework.md`，repair item T9-R1 承载修复与 T9 收尾验证复跑。Acceptance（R3 delta spec SHALL 面）与 Iteration Map 不变，故非 replan。修复方向采纳 Act 建议：`select_all_rows` 经既有 `quote_ident` 包裹——与 dump 输出面同一 helper，输出语义不变，归一化后对全部 catalog 名类可解析。

**Iteration Plan Update**

None

**Next Cycle**

`iterations/002-table-name-normalization/001-rework.md`（repair item T9-R1：select_all_rows 经 quote_ident + 转义名 dump 往返测试 + T9 收尾验证复跑）

**Next Iteration**

None（Iteration 002 未 accepted；Map 末 Iteration）
