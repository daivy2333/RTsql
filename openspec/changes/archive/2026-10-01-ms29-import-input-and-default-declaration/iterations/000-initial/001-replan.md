# Iteration 000 / Cycle 001-replan: 修订语义收口（T2 两场景校准 + T8 重复列名守卫）

## Plan Context

- Status: ready
- Cycle Type: replan

**Cycle Scope**

- Change tasks: T2（剩余两场景，replan 修订语义）、T8（replan 增入）
- Acceptance gaps: T2「自动建表首列隐式主键（引擎语义沿用）」与「自动建表经保留名/重复列名守卫拒绝」两场景（父 Cycle Blocker Handoff；根因 = Plan 错误 ×2，见 Background）
- Repair items: None（replan 使用更新后的全局 task，不建 repair item）
- Inherited scope: proposal 默认假设 1-6、10；用户裁定 1/3/4；design D1-D3、D5、D9、D10 与 **D4（replan 修订版）**；**T1/T3 已完成且验证结论采信继承**（父 Cycle Act Response：T1 stdin 2 用例 GREEN、T3 memory 2 用例 GREEN、`cli_test import` 7/7 含 `test_import_missing_table_or_db` 校准臂；实现面 `import_csv` memory 路由/stdin 双分支/自动建表主体与 `lifecycle_db_target` 豁免已在工作区，本 Cycle 不重做不回退）。
- Excluded scope: Iteration 001 全部（T4-T7）；真无主键表引擎特性（结构性不可行/另立路线，design D4）；`ALTER TABLE ADD COLUMN` 同名冲突守卫（未调查，不扩围）。

**Background**

父 Cycle 000-initial 于 T2 两场景命中 Gate 5 阻塞。Plan Review 独立核实为 **Plan 错误 ×2**（非 Act 偏离）：①「普通建表无 PK 合法、零引擎改动」调查止于 planner 层，漏查 `CreateTableExecutor` 对未声明 PK 的首列默认臂；②「重复列名守卫自然生效」系把 CTAS 输出列重复检查错误泛化到普通建表通道。用户指令「工作阻塞，进行审计，给出解决」授权审计与解决；集中决策未获即时应答，按审计推荐执行并标记可推翻——主键语义 = 沿用引擎「未声明 PK → 首列隐式主键」（与 CTAS 同款）；重复列名 = planner 守卫本 change 内根因修复。`Review Result: replan-required` 已写入父 Cycle，本 Cycle 按修订后契约执行。

**Investigation Facts**

- Current Baseline: 分支 master @ 4e1944f；工作区含本 change 实施面（T1/T3 完成 + T2 主体）+ MS28 收尾未提交项（无交集）。`import_input_surface_test` 现状 **10 passed / 2 failed**（两失败用例 = 本 Cycle 校准对象）；`cli_test import` 7/7。全量权威基线 1657 / 0 failed / 2 ignored（MS28 accepted；本 Cycle GREEN 后全量分批复认）。
- Current-State Evidence（Plan 读码 + 独立直证，2026-10-01）：
  - **修正事实 1（主键）**：`src/executor/create_table.rs:74-83`——`match &node.primary_key { Some(pk) => pk.clone(), None => columns.first()… }`，注释「如果没有指定主键，使用第一列作为主键」；`create_table_with_constraints(&name, columns, &pk)` 以此 pk 建唯一索引。直证：`CREATE TABLE pkcheck (a INT, b STRING)` rc=0 → `rtsql schema` 渲染 `CREATE TABLE "pkcheck" ("a" INT PRIMARY KEY, "b" STRING);`。引擎不存在表级无主键；「无键行」= PK 值 NULL 的行不入索引（MS10-T05 行级语义）。dump→restore 往返自洽：dump 渲染首列 PRIMARY KEY，restore 的 CREATE TABLE 无声明 PK → 执行器重默认首列 → 同一形态。
  - **修正事实 2（重复列名）**：planner `build_create_table`（ddl_dml.rs:799-953）与 TableManager 均无重复列名检查（planner 列名经 `:859` `to_lowercase()` 归一）；直证：`CREATE TABLE dup ("a" INT, "a" INT)` rc=0 → schema 渲染双 `PRIMARY KEY`、`INSERT INTO dup VALUES (1,2)` rc=0；该形态 dump 后 restore 必触发 `MultiplePrimaryKey`（planner 表级 PK 提取多列即错）。CTAS 通道另有输出列重复检查（`build_create_table_as_select` 内，不受影响）。
  - **实施现场（父 Cycle 产出，本 Cycle 消费）**：`src/cli/lifecycle.rs::import_csv`——`get_table` Err 已改 `.ok()` 进入自动建表臂（records 预收集 → `csv_field_guess` → `infer_ctas_column_types`〔已 `pub(crate)`〕→ 冲突 General exit 1 → DDL `execute_sql`〔失败 Sql exit 3〕→ 恒等列映射）；两失败用例位于 `tests/import_input_surface_test.rs`（父 Cycle 按原契约所写：`auto_create_*_implicit_pk`/`auto_create_duplicate_column_names_rejected` 形态，断言对象为旧语义）。
- Code and Critical Path: T2 校准 = 修订两测试断言（产品代码零改动预期——隐式主键即现行为）；T8 = `build_create_table` 装配完成点（T4 声明门同位置，Iteration 001 将在其上加 DEFAULT 校验循环，互不冲突）加重复列名检查 → 自动建表重复表头经 `execute_sql` 计划期拒绝 → import 既有 `Response::Error` → `ExitStatus::Sql` 通道 exit 3。

**Implementation Guidance**

- T2 两用例校准按 delta spec 修订场景逐条对齐（断言对象变化记录于 Act Response Deviations，父 Cycle 用例不删除不弱化——断言意图由修订断言承接）；INT 首列重复用例需断言：exit 3、stderr 含 Duplicate 语义、失败前已提交行重开可见、`rtsql schema` 渲染首列 `PRIMARY KEY`。
- T8 错误用复用 `PlanError::InvalidQuery`（CTAS 输出列重复同面，不新增变体/分类表行）；文案点名列名（措辞非实质）。检查置于 column_defs 装配后、PK 提取前后皆可（建议紧邻 T4 未来落点，单一遍历）。
- 顺序 T8 → T2 校准（守卫先行，重复表头用例随后转绿）；最后跑受影响面 + 全量分批。

**Task Contracts**

### T8: planner 重复列名计划期守卫

- Requirement/Scenario: sql-write-surface ADDED「CREATE TABLE 重复列名计划期拒绝」全部场景；cli-noninteractive-shell import MODIFIED「自动建表经保留名/重复列名守卫拒绝」
- Depends on: None
- Targets: `src/parser/planner/ddl_dml.rs::build_create_table`（column_defs 装配后单点）
- Current behavior: `CREATE TABLE dup ("a" INT, "a" INT)`（含归一后同名 `A`/`a`）rc=0，静默建成且 dump→restore 必坏（MultiplePrimaryKey）
- Required behavior: column_defs 装配完成后检测归一后同名重复，首个重复 `Err(PlanError::InvalidQuery)`（点名列名）；拒绝零副作用；CTAS 输出列重复既有拒绝与唯一列名建表零变化
- Preserve: planner 列名小写归一语义；表级 PK/UNIQUE 策略面；既有建表全部用例逐字节
- Forbidden: 不动 `build_create_table_as_select`；不动 `extract_primary_key`；不给 `ALTER TABLE ADD COLUMN` 加同名守卫（范围外）；不新增 PlanError 变体
- Test witness: `tests/planner_test.rs` 增 3 例（同名重复拒绝点名列名 / 引号大小写归一后同名拒绝 / 唯一列名零回归抽样）——**RED**：守卫缺失时 `CREATE TABLE dup ("a" INT, "a" INT)` 计划成功（现状直证在案）
- GREEN condition: 3 例全绿 + `import_input_surface_test` 重复表头用例转绿（exit 3 + 零副作用）+ planner_test 建表组与 CTAS 组零回归
- Verification: `cargo test --test planner_test`；`cargo test --test import_input_surface_test`
- Stop when: 守卫与既有表级 PK 提取/UNIQUE 策略面出现执行序冲突（契约失效）——返回 Plan

### T2-R: 自动建表两场景按修订语义校准

- Requirement/Scenario: cli-noninteractive-shell import MODIFIED「自动建表首列隐式主键（引擎语义沿用）」（修订后）
- Depends on: T8（重复表头场景消费守卫）
- Targets: `tests/import_input_surface_test.rs` 两失败用例（断言校准；产品代码零改动预期）
- Current behavior: `auto_create_*implicit_pk*` 用例断言「首列重复值全部导入」而实际 DuplicateKey fail-fast（exit 3）；`auto_create_duplicate_column_names_rejected` 断言计划期拒绝而守卫缺失（T8 后转绿，断言形态按实现核对——exit 3 + stderr 点名 + 零副作用）
- Required behavior: INT 首列重复 → 首个重复值 DuplicateKey fail-fast（exit 3、stderr 含 Duplicate 语义、失败前已提交行重开可见、`rtsql schema` 渲染首列 `PRIMARY KEY`）；FLOAT/STRING 首列重复 → 全部导入 exit 0（非键行）；重复表头 → exit 3 点名列名零副作用（T8 供给）
- Preserve: 父 Cycle 已绿十用例零改动；既有 import 面零回归
- Forbidden: 不删用例；不为转绿弱化断言；不改 `import_csv` 产品代码（若发现需产品改动即返回 Plan——修订语义下不应需要）
- Test witness: 两用例校准前后断言 diff 记录于 Act Response；GREEN = `cargo test --test import_input_surface_test` **12 passed / 0 failed**
- GREEN condition: 矩阵全绿 + `cli_test` import 组零回归
- Verification: `cargo test --test import_input_surface_test`；`cargo test --test cli_test`
- Stop when: 修订语义下仍需产品代码改动才能自洽（说明设计仍有缺口）——返回 Plan

**Invariants**

- 父 Cycle Invariants 全部继续有效（import auto-commit/fail-fast、既有表路径逐字节、import 不建库文件、CTAS 语义不受影响、锁/通道守卫零变化）。
- 引擎建表语义零改动：`CreateTableExecutor` 首列默认臂、`extract_primary_key`、CTAS 通道原样。
- `Review Result: replan-required` 的父 Cycle 产物冻结不改写；本 Cycle 只新增。

**Non-goals**

- Iteration 001（T4-T7）；真无主键表；`ALTER TABLE ADD COLUMN` 同名守卫；import 攒批；Bool/Date 猜测；`:memory:` 其他子命令豁免。

**Acceptance**

1. 重复列名守卫：普通 `CREATE TABLE` 同名/归一后同名计划期拒绝点名列名零副作用；唯一列名与 CTAS 零回归（sql-write-surface ADDED；T8）。
2. 自动建表首列隐式主键：INT 首列重复 DuplicateKey fail-fast（部分导入保留 + schema 渲染 PRIMARY KEY）、FLOAT/STRING 首列重复合法；重复表头 exit 3 零副作用（import MODIFIED 修订场景；T2-R + T8）。
3. 父 Cycle 已交付面零回退：`import_input_surface_test` 12/12、`cli_test import` 7/7、T1/T3 场景全绿（T1/T2-R/T3/T8）。
4. 全量分批零回归（基线 1657 不降）（T8/T2-R）。

**Verification**

- `cargo test --test planner_test`（1）、`cargo test --test import_input_surface_test`（2/3）、`cargo test --test cli_test`（3）、全量分批 M19（4）。
- 校准断言 diff 与 RED 形态记录于 Act Response；人工步骤：无。

**Gate 2 Readiness**

其余检查项 PASS（修订 requirement 已入 delta spec、调查闭合〔两修正事实带直证〕、任务可执行、追踪完整、验证充分、无身份型证据工程、Evidence 模式 none）。**2026-10-01 用户指令「工作阻塞，进行审计，给出解决」授权阻塞处置；两项 replan 裁定（隐式主键沿用 / planner 守卫本 change 修复）经审计推荐执行、已在 proposal「Replan 裁定」节标记可推翻——Gate 2 以该授权与审计结论通过。**

**Persisted Evidence**

- Mode: none

（Act Response 记录命令、决定性输出与退出码即足。）

**Risks and Notes**

- 全量套件若存在依赖「重复列名可建」的既有用例（预期无——该形态表不可 dump 往返，无合法消费方），T8 会使其 RED：就地按新语义校准并在 Response 记录（非实质）；无法校准的实质冲突返回 Plan。
- `auto_create_duplicate_column_names_rejected` 现有断言形态（父 Cycle 按原契约所写）可能与 T8 实际错误文案/退出码细节有差——按实现核对校准，断言意图（计划期拒绝 + 零副作用）不变。
- INT 首列重复用例的「部分导入保留」断言依赖逐条 auto-commit 既有语义（MS10 以来不变），无需新机制。
- 非实质留 Act：守卫检查的遍历写法、错误文案措辞、测试函数命名。

## Act Response

- Status: reported

**Implemented**

- **T8 守卫（本 Cycle 产品代码唯一改动）**：`src/parser/planner/ddl_dml.rs::build_create_table`——column_defs 装配（`collect::<Result<…>>()`）之后、`extract_primary_key` 之前插入单遍重复检测：`column_defs[..index]` 前缀比对（列名在装配点已经 `to_lowercase()` 归一，同名与大小写变体同面），首个重复即 `Err(PlanError::InvalidQuery("duplicate column name '{name}'"))` 点名归一后列名。
- **T8 测试见证**：`tests/planner_test.rs` +3 例，RED 先行（实现前直证守卫缺失时 `CREATE TABLE dup ("a" INT, "a" STRING)` 计划成功、产出双 `"a"` 列 CreateTableNode）：
  - `test_create_table_duplicate_column_rejected`（同名重复点名拒绝）
  - `test_create_table_duplicate_column_case_normalized_rejected`（`"A"`/`"a"` 归一后同名拒绝）
  - `test_create_table_unique_column_names_still_accepted`（唯一列名零回归抽样）
- **T2-R 两用例校准**（`tests/import_input_surface_test.rs`，产品代码零改动——契约预期成立）：
  - 隐式主键用例按修订语义反转断言并扩 STRING 对照：INT 首列（`id,v: 1,a / 1,b`）→ exit 3、stderr 含 `Duplicate`、失败前已提交行重开可见（SELECT 恰 `[[1,"a"]]`）、`rtsql schema` 渲染 `"id" INT PRIMARY KEY`；STRING 首列（`name,v: a,1 / a,2` → 表 `s`）→ 全部导入 exit 0、affected_rows 2。
  - 重复表头用例补 stderr 点名断言（`duplicate column name`）；exit 3 + schema 零残留（零副作用）断言保持。

**Deviations from Plan**

1. 用例改名 `auto_create_no_pk_first_column_duplicates` → `auto_create_first_column_implicit_pk_semantics`：旧名在修订语义下失实（引擎「未声明 PK → 首列隐式主键」存在）。属计划「非实质留 Act：测试函数命名」授权，见证意图由修订断言承接，用例未删除未弱化。
2. FLOAT/STRING 对照子场景并入 INT 用例同函数承载（两份 fixture 同 TempDir、表名分立）：计划 Required behavior 含两形态而 GREEN 条件限定 12/12——拆成独立用例将破坏既定 GREEN 计数；同函数双形态见证效力等同。
3. 计划外修复三处继承面遗留（均属本 change T1/T3 实施面，非本 Cycle 契约目标；Gate 3 自检发现，按 Plan Context 风险注记「就地校准 + Response 记录」处置）：
   - `tests/memory_db_test.rs::t4_cli_lifecycle_subcommands_reject_memory_target` 遗留断言 import 拒绝 `:memory:` exit 2，与父 Cycle 已交付的 T3 豁免语义直接冲突——父 Cycle 因全量分批未执行而未暴露，本 Cycle 首次全量分批批 5 RED。按 delta spec「`:memory:` import 免建表可达」将 import 移出拒绝矩阵（其余五子命令拒绝不变），注释记明豁免去向；其余子命令拒绝场景仍由 `import_input_surface_test::memory_import_key_rejected_and_others_still_guarded` 覆盖，见证链完整。
   - `src/cli/lifecycle.rs` 3 处 fmt 排版（父 Cycle 产出未格式化）：`cargo fmt` 清零（diff 仅限该文件）。
   - clippy `too_many_arguments`（`import_csv` 8 形参）：`#[allow(clippy::too_many_arguments)]` 注记豁免——参数均为分派臂直达语义，收拢结构体反而加间接层。

**Self-Review**

- Spec compliance：T8 守卫单点、复用 `InvalidQuery`、点名归一后列名；`build_create_table_as_select`、`extract_primary_key`、ALTER 通道均未触碰，未新增 PlanError 变体（契约 Forbidden 全部遵守）。T2-R 断言与 delta spec 修订场景逐条对齐（INT fail-fast / 部分保留 / schema PRIMARY KEY / STRING 合法 / 重复表头 exit 3 点名零副作用）。Invariants 全部成立：既有表导入路径逐字节、import 不建库文件、CTAS 语义不受影响、锁/通道守卫零变化、父 Cycle 已绿十用例零改动。
- Code quality：全量分批零新警告；fmt/clippy 清零后复核通过；无身份型证据机制与判定层；新增测试保持「单用例自带独立 TempDir」形态（M19）。
- 遗留 Minor：无。

**Verification Evidence**

- RED 见证（T8 实现前）：`cargo test --test planner_test test_create_table_duplicate` → 2 failed（panic：`Expected duplicate column rejection, got CreateTable(… "a","a" …)`）。
- T8 GREEN：`cargo test --test planner_test test_create_table` → `17 passed; 0 failed`；全文件 93/0。
- T2-R GREEN：`cargo test --test import_input_surface_test` → `12 passed; 0 failed`。
- 受影响面：`cli_test` import 组 `7 passed; 0 failed`；`memory_db_test` 修复后 `15 passed; 0 failed`。
- 全量分批（M19 纪律）：9 批、字典序分组、每批单次 `cargo test … --test … -- --test-threads=1`、批间顺序执行；各批 passed 计数——批1（lib 354 / bins 0 / aggregate_extension 13 / aggregate_output_shape 8 / aggregate 19 / alter_table 16 / attach_session 11 / btree_merge 10 / btree_scale 5 / btree_split 12 / btree 10 / busy_timeout 18）= 476；批2（cache_perf 1 … ctas 17，cli_test 117 含 2 ignored）= 216；批3 = 164；批4 = 117（import_input_surface 12/12 在批内复核）；批5 = 84（memory_db 15 为修复后重跑结论，keyless_eq 14 / keyless_row 4 / limit 5 / mvcc_abort 3 / mvcc_commit 4 / mvcc_record 5 / mvcc_tombstone 11 / nlj 9 / net_proto 5 / net_server 0 / no_from 9）；批6 = 202；批7 = 152；批8 = 179；批9 = 82。**合计 1672 passed / 0 failed / 2 ignored**，与「MS28 基线 1657 + import_input_surface_test 新文件 12（父 Cycle 交付）+ T8 新增 3」严格吻合，无丢失无意外增。
- lifecycle.rs 在全量后变更（fmt + allow 属性，非行为面）→ 受影响边界定向复跑：import_input_surface `12/0`、cli_test import 组 `7/0`、memory_db `15/0`；其余批次结论覆盖面未变化，采信保留。
- 格式与静态检查：`cargo fmt -- --check` clean；`cargo clippy --lib` 0 warnings。
- `openspec validate 2026-10-01-ms29-import-input-and-default-declaration` → `Change … is valid`。

**Persisted Evidence**

- None required（Plan 指定 mode = none；未创建 `evidence/` 目录，Act Response 记录命令与决定性输出即足）。

**Experience Candidates**

- None（memory_db 校准属本 change 范围内遗留断言冲突，非范围外实质缺陷；fmt/clippy 收口为常规自检修复，无可复用高风险操作路径）。

**Remaining Issues**

- 无。

## Plan Review

- Review Result: accepted

**Findings**

独立审计（读码 + 只读 git diff 逐文件核对 Act 声明，2026-10-01）：

1. **T8 守卫核实成立**：`build_create_table`（ddl_dml.rs）column_defs 装配（:856-868）后、`extract_primary_key`（:882）前插入单遍前缀比对（:870-879），归一后同名 `Err(PlanError::InvalidQuery("duplicate column name '{}'" ))` 点名列名；该文件 git diff 仅 +11 行恰为守卫本体；CTAS 臂于 :847 提前返回不受影响，`build_create_table_as_select`/`extract_primary_key` 零触碰——契约 Forbidden 全部遵守。
2. **测试见证核实成立**：`planner_test.rs` +3 例（:1018 同名拒绝点名 / :1035 `"A"`/`"a"` 归一后同名 / :1052 唯一列名零回归）与契约 Test witness 逐条对应；`import_input_surface_test.rs` 12 测试函数齐备，两校准用例断言与修订语义逐条吻合——INT 首列（exit 3 + stderr `Duplicate` + 失败前已提交行 `[[1,"a"]]` 重开可见 + schema 渲染 `"id" INT PRIMARY KEY`）、STRING 首列（全导入 exit 0 + affected 2）、重复表头（exit 3 + stderr `duplicate column name` + schema 零残留）。
3. **Deviations 4 项均核实为非实质（分类并入本节）**：
   - 用例改名 `auto_create_no_pk_first_column_duplicates` → `auto_create_first_column_implicit_pk_semantics`——计划授权范围（非实质留 Act：测试函数命名），见证意图由修订断言承接（Act 偏离，已授权）。
   - FLOAT/STRING 对照并入 INT 用例同函数——GREEN 计数 12/12 约束下的等价承载（Act 偏离，授权范围内）；**但实际交付仅含 STRING 对照，FLOAT 形态无直接执行见证**——见 Minor 1。
   - `memory_db_test.rs::t4` import 移出拒绝矩阵——继承面遗留断言与父 Cycle 已交付 T3 豁免语义的范围内冲突，校准与 delta spec「`:memory:` 其他生命周期子命令维持拒绝」场景一致（其余 5 子命令保持，注释记明豁免去向，其接受面由 `memory_import_stdin_and_file`/`memory_import_key_rejected_and_others_still_guarded` 承接，见证链完整）；归因为 Plan 遗漏（父 Cycle 校准清单未含该文件，Gate 4 全量分批因阻塞未执行而未暴露）。
   - lifecycle.rs fmt 3 处 + `#[allow(clippy::too_many_arguments)]`——常规自检收口，豁免带理由注释，非行为面（Act 偏离，非实质）。
4. **Minor 1（记录不返工）**：Acceptance 2 与验证边界均含「FLOAT 首列重复合法」，校准用例仅见证 STRING 形态。独立查证：`Value::to_key`（`src/executor/value.rs:98`）`Float` 与 `String` 同处 `None` 臂——FLOAT 首列与 STRING 首列走完全相同的非键行路径（不入索引 → DuplicateKey 结构性不可达），设计 D4 明文同款；行为由代码结构 + STRING 形态执行见证共同保证。不构成 Acceptance gap。
5. **Minor 2（记录不返工）**：Act Response Deviations 2 描述「FLOAT/STRING 对照子场景并入」与实际交付（仅 STRING）存在措辞出入——同 Minor 1 根因，随后续 change 文档化语义时自然覆盖，无需当前 Cycle 修复。
6. **spec 语序笔误已校准**：父 Cycle Remaining Issues 的 delta spec `:memory:` import 语序（`rtsql :memory: import …` → `rtsql import :memory: …`）已在 replan 中修正（spec 场景与正文均核对无误）。

**Acceptance Gaps**

无——四项 Acceptance 全部满足（详见 Evidence）。全量对账严格吻合：1672 = MS28 基线 1657 + `import_input_surface_test` 新文件 12（父 Cycle 交付）+ T8 新增 3。

**Evidence**

- 独立读码：`src/parser/planner/ddl_dml.rs:856-882`（守卫位置与 CTAS 提前返回）、`src/executor/value.rs:92-103`（to_key Float/String 同臂）、`src/cli/mod.rs:348-363`（lifecycle_db_target Import 豁免）与 `:279-282`/`:383-397`（memory 判定与 `--key` 拒绝）、`src/pipeline.rs:780`（`pub(crate) infer_ctas_column_types`）、`src/cli/lifecycle.rs:593-595`（clippy 豁免注释）。
- 独立 diff 核对（只读 git）：ddl_dml.rs +11（守卫本体）、planner_test.rs +56（3 例）、import_input_surface_test.rs（12 测试函数）、memory_db_test.rs +6/-2（矩阵校准 + 注释）、cli_test.rs import 校准 hunk（表缺失臂 → 自动建表语义 exit 0 + affected 1；库缺失臂不变）。
- Act Response 验证结论采信（来源：001-replan Act Response › Verification Evidence，2026-10-01 本工作区分批执行；覆盖未变化——结论后仅 lifecycle.rs fmt/属性非行为面变更且已定向复跑三文件）：RED 见证（planner_test 2 failed 守卫缺失形态）、planner_test 93/0、import_input_surface_test 12/12、cli_test import 组 7/7、memory_db_test 15/15、全量分批 9 批 1672/0/2、fmt/clippy 清零、`openspec validate` 通过。无重跑触发情形（无矛盾、覆盖面未变化、无失败、无第三次失败）。

**Follow-up Decision**

接受（accepted）：T8 守卫与 T2-R 校准按修订契约交付且独立核实一致；4 项 Deviations 均非实质且已记录；2 项 Minor finding 不阻塞 Acceptance（FLOAT 形态由 to_key 同臂结构保证），按 iteration-planning 分类「只出不阻塞 Acceptance 的 Minor finding → accepted，记录不返工」。Iteration 000 完成。

**Next Cycle**

None

**Next Iteration**

`openspec/changes/2026-10-01-ms29-import-input-and-default-declaration/iterations/001-default-declaration/000-initial.md`（已展开，Status: ready——T4/T5/T6/T7，行号锚点按 T8 守卫合入后工作区复核刷新）
