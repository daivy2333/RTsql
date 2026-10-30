# 自动建表路径的注释与「未声明主键 → 首列隐式主键」既定语义相反

- Status: open（修复已交付于主分支提交 `5035869`，2026-10-30；关闭待用户指令——公共规则与 Recorder 规范把 Issue 的关闭限定为「按用户指令执行」，本次 goal 授权覆盖的是 M23 的「登记与承载」环节，不含缺陷台账的状态迁移）
- Filed: 2026-10-30
- Source: Act Response 的 Issue 候选 2 与该 Cycle 的 Plan Review「Issue 候选判定（只报告，不登记）」段，`openspec/changes/2026-10-28-ms44-unified-diagnostics/iterations/001-statement-phases/000-initial.md`（Act Response Status `reported`，Review Result `accepted`；该 change 尚未归档，位于 worktree `/home/daivy/projects/RTsql-changes/2026-10-28-ms44-unified-diagnostics`，主仓 `openspec/changes/` 下无此目录）
- Environment: WSL2（Linux 6.18.40.1-microsoft-standard-WSL2 x64）；被测二进制为 change worktree 的 `target/debug/rtsql`（构建产物时间 2026-10-30 18:43，核验时工作树 HEAD `17ad6b1`）；`src/executor/create_table.rs`、`src/cli/lifecycle.rs`、`src/parser/planner/ddl_dml.rs`、`src/storage/data/table_manager.rs` 与 master `3686a05` 无差异（`git diff master -- <上述路径>` 为空），该二进制对 master 的同构行为有效；主仓 master 无构建产物，未在 master 实跑

## 缺陷描述

- 预期（规格面，且实现与之逐项一致）：`openspec/specs/sql-constraint-enforcement/spec.md:215` 的 Requirement 写明「未显式声明主键的『未声明主键 → 首列隐式主键』既有引擎语义 SHALL 零变化（INTEGER 首列重复 DuplicateKey 强制、非 INTEGER 首列行落非键行重复合法——文档化行为保持）」，其 Scenario「隐式主键语义零变化」（同文件 `:224-228`）逐字给出该形态。同向表述另见 `openspec/specs/create-table-as-select/spec.md:13`（CTAS 新表沿用同款语义，重复首列 Int 值以 `DuplicateKey` 点名失败）与 `openspec/specs/cli-noninteractive-shell/spec.md:549` 及其 Scenario「自动建表首列隐式主键（引擎语义沿用）」（同文件 `:632-637`：INT 首列形态于首个重复值处以 DuplicateKey fail-fast 拒绝 exit 3，FLOAT/STRING 首列重复值全部导入成功）。`README.zh-CN.md:275` 同口径。
- 实际（代码注释面）：`src/cli/lifecycle.rs:825-827` 的注释写「建表 DDL（无主键——首列重复值合法，无键行语义；保留名/重复列名等常规建表守卫自然生效）」。该注释位于 `rtsql import` 自动建表路径内，其后 11 行即 `db.execute_sql(&ddl)`（`:838`）。
- 实际（运行观察，2026-10-30）：注释所描述的路径本身即与注释相反——自动建表 + INT 首列 + 首列重复值，以 DuplicateKey fail-fast 拒绝，退出码 3，报「import row 2 of 2 failed: Execution error: Duplicate key」；即该形态下「首列重复值合法，无键行语义」不成立。同一路径下 FLOAT/STRING 首列的重复值确实合法（非键行语义），注释对这半边成立。
- 位置：
  - 缺陷所在：`src/cli/lifecycle.rs:825-827`（三行注释）。
  - 被注释描述的真实行为：`src/executor/create_table.rs:73-83`——`CREATE TABLE` 无显式主键时「如果没有指定主键，使用第一列作为主键」，并以 `node.primary_key.is_some()`（`:92`）把「显式与否」写入目录（`src/storage/catalog.rs:118` 的 `explicit_primary_key`）；`src/executor/insert.rs:155-187` 按该键位做重复预检，命中即 `StorageError::DuplicateKey`（`:185`）。
  - 代码内另一处正确表述：`src/pipeline.rs:985`（CTAS 路径「建表：首列沿用『未声明主键时首列为隐式主键』既有语义」）。
- 根因（Confirmed，文档层）：注释把「DDL 文本未显式声明 PRIMARY KEY」误写成「表无主键、首列重复值合法、无键行语义」。引擎的隐式首列键登记自 2026-05-20 起存在（`9409bfa`，`CreateTableExecutor`），注释写于 2026-10-02（`24ae134`）——两者从未一致，属写注释时的口径混淆，不是行为演进后遗留的旧描述。
- 候选定性（本条判定的核心，也是 Act 与 Plan Review 留待 Recorder 判定的问题）：Act Response 与 Plan Review 把候选 2 表述为「行为与 `src/cli/lifecycle.rs` 的注释矛盾」，未指明矛盾在哪一侧。经核，该行为是规格明文规定的既有引擎语义（三个规格域 + README），实现与规格逐项一致，实测行为与规格 Scenario 逐字对应（INT 首列重复 → DuplicateKey fail-fast exit 3；FLOAT/STRING 首列重复 → 合法）。**行为侧不构成缺陷**；成立的只有注释与规格/实现相反这一项。本条登记的是该注释缺陷，不是「首列隐式唯一键」这一行为。
- 边界：
  - 只在首列声明类型为 INTEGER 时触发明示的 DuplicateKey；FLOAT/STRING 首列重复值合法（非键行语义）——注释对这半边成立。
  - 注释不产生运行时影响：引擎行为由 `src/executor/create_table.rs` 与 `src/executor/insert.rs` 决定，与注释文本无关。
  - NULL 键位保持无键行语义、不入索引、不做重复检查（`src/executor/insert.rs:171-187`），与本条无关。
  - 未核：库通道（`Database::execute_sql`）直接建表后的同等形态未单独实跑；`rtsql dump` 渲染与 restore 往返未在本条核。

## 影响

- 当前（文档层，非行为）：`src/cli/lifecycle.rs` 的注释与规格、实现、实测行为三者相反，且位于「自动建表」这条被规格 Scenario 明确约束的路径的现场。
- 风险：后续变更读到该注释，可能把「INTEGER 首列重复 → DuplicateKey」当成缺陷并按注释修正（例如让无显式主键的表不再登记首列键），该改动会同时违反 `sql-constraint-enforcement`、`create-table-as-select`、`cli-noninteractive-shell` 三域的 SHALL 条款，并把 `rtsql import` 的既有 fail-fast 语义改成静默接受重复值。
- 已发生的用户影响：None。本条未观察到故障；DuplicateKey 拒绝本身是规格要求的正确行为，不是用户可见损失。
- 不受影响：引擎实际行为、导入与建表的既有语义、规格库内容、README。

## 事件记录

None。文档层缺陷，未引发故障；未观察到用户、数据或交付影响。

## 处置

- 处置指针：改 `src/cli/lifecycle.rs:825-827` 的注释文本，使其与规格一致（表述「DDL 不显式声明主键；表沿用引擎『未声明主键 → 首列隐式主键』语义——INTEGER 首列重复值以 DuplicateKey fail-fast 拒绝，FLOAT/STRING 首列重复值合法（非键行语义）」）。
- 该修改不改变系统可观察行为，按公共规则 › 轻量工作属直接在主分支执行的注释修改，不建 change、不开分支；用户授权时执行即可。
- 明确不得做的事：不得以本条为据修改引擎的隐式首列主键语义——那会违反三个规格域的 SHALL 条款；本条的存在正是为防止这类误修。
- 2026-10-30 登记。Status 为 open。任务列表尚未指向本条。
- 2026-10-30 修复已交付：主分支提交 `5035869`（`fix: 修正 import 自动建表注释与首列隐式主键语义相反`，2026-10-30 19:14）把 `src/cli/lifecycle.rs` 的该段注释改写为与三域规格一致——「建表 DDL（不显式声明主键——新表沿用引擎『未声明主键 → 首列隐式主键』既有语义：INT 首列重复值以 DuplicateKey fail-fast 拒绝，FLOAT/STRING 首列重复值合法（非键行语义）；保留名/重复列名等常规建表守卫自然生效），失败走既有 SQL 失败通道（exit 3）且零副作用」。处置指针：fixed → `5035869`（`src/cli/lifecycle.rs`，注释文字 5 insertions / 3 deletions；可执行逻辑、参数与控制流未改，可观察行为未变）。该提交按公共规则 › 轻量工作直接在主分支执行，未建 change。
- 关闭待用户指令：Status 仍为 open。关闭与重开只按用户指令执行（Recorder 规范与公共规则「Issue 的建立、关闭与重开由 Recorder 按用户指令执行」）；本次 goal 原文为「继续实施，把这个change做完，要求遵守各种规范」，M23 明确覆盖的是「登记 Issue」环节，缺陷台账的状态迁移不在其内，故本流程不代做关闭。用户确认后按既有格式记为 `closed（2026-10-30 fixed → 5035869）`，并在本段追加关闭原因与指针。

## 证据

- `openspec/changes/2026-10-28-ms44-unified-diagnostics/iterations/001-statement-phases/000-initial.md`：Act Response 的「Experience Candidates」Issue 候选 2（记录 `CREATE TABLE u (a INT, b INT)` 后 `INSERT (1,1)` 成功、`INSERT (1,2)` 报 `Execution error: Duplicate key`、`INSERT (2,1)` 成功，并指出与 `src/cli/lifecycle.rs` 注释矛盾，问「是注释失效还是行为缺陷由 Recorder 判定」）；同文件 Plan Review 的「Issue 候选判定（只报告，不登记）」段（Review Result `accepted`，判为变更范围外、不在本 change 承载，建议 Recorder 登记）。同文件 Plan Review 的偏差 1 归类亦记录了 Plan 对 CLI 入口的事实错误，与本条无关。
- 规格条款位置：`openspec/specs/sql-constraint-enforcement/spec.md:215`（Requirement 正文）与 `:224-228`（Scenario「隐式主键语义零变化」）；`openspec/specs/create-table-as-select/spec.md:13`；`openspec/specs/cli-noninteractive-shell/spec.md:549`（Requirement 正文）与 `:632-637`（Scenario「自动建表首列隐式主键（引擎语义沿用）」）；`README.zh-CN.md:275`。
- 代码事实（master `3686a05` 与 worktree 同构）：`src/cli/lifecycle.rs:825-827`（缺陷注释）、`src/cli/lifecycle.rs:838`（`db.execute_sql(&ddl)`）、`src/executor/create_table.rs:73-93`、`src/executor/insert.rs:155-187`、`src/storage/catalog.rs:118`、`src/pipeline.rs:985`。
- 运行观察（2026-10-30，Linux x86_64，worktree 二进制 `target/debug/rtsql`，隔离的 `RTSQL_HOME` 与临时库）：
  - `rtsql import <db> t dup_int.csv --csv`（表头 `id,val`，数据行 `1,10` / `1,20`）→ 退出码 3，stderr「import row 2 of 2 failed: Execution error: Duplicate key」。
  - `rtsql import <db> t2 dup_str.csv --csv`（表头 `k,val`，数据行 `x,10` / `x,20`）→ 退出码 0，导入 2 行。
  - `rtsql import <db> t3 dup_float.csv --csv`（表头 `k,val`，数据行 `1.5,10` / `1.5,20`）→ 退出码 0，导入 2 行。
  - `rtsql schema <db>` → `CREATE TABLE "t" ("id" INTEGER PRIMARY KEY, "val" INTEGER);`、`CREATE TABLE "t2" ("k" TEXT, "val" INTEGER);`——与规格「`rtsql schema` 渲染首列 `PRIMARY KEY`」一致。
  - 主命令形态：`CREATE TABLE u(a INT, b INT)` 后 `INSERT INTO u(a,b) VALUES (1,1)` 成功、`VALUES (1,2)` 报 `Execution error: Duplicate key`（退出码 3）、`VALUES (2,1)` 成功；`CREATE TABLE s(a TEXT, b INT)` 后 `('x',1)` 与 `('x',2)` 均成功。
- 历史：`git log -S "如果没有指定主键，使用第一列作为主键" -- src/executor/create_table.rs` → `9409bfa`（2026-05-20）；`git log -L 825,827:src/cli/lifecycle.rs` → 注释由 `24ae134`（2026-10-02）引入。
- 修复交付核对（2026-10-30，只读 diff）：`git show 5035869 -- src/cli/lifecycle.rs` 显示改动只覆盖该三行注释（5 insertions / 3 deletions，新文本见「处置」段），无可执行代码变化；`git log` 显示该提交在 master 上（其后为 `88b8087`）。注释文本与 `sql-constraint-enforcement` / `create-table-as-select` / `cli-noninteractive-shell` 三域的表述逐项一致。改动不含可观察行为，未做运行验证（判定对象是注释文本本身，已由 diff 阅读确认）；主仓 master 无构建产物，本次未在 master 上编译或运行。
- 未知项：注释写下时（2026-10-02）该形态的 DuplicateKey 强制是否已生效未在本条核——只确认隐式首列键的登记自 2026-05-20 起存在；库通道直建表的同等形态未单独实跑；`rtsql dump` / restore 往返未核。
