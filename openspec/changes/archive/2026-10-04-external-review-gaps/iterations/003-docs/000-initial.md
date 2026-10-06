# Iteration 003 / Cycle 000: 文档同步批

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T8（文档同步——README.md / README.zh-CN.md / rtsql-docs/SKILL.md）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal R8 与场景 S30；design D8 七项清单；R1–R7 定稿行为全部来自 accepted Iteration——R1/T1（IndexScanAll 投影形状）、R2/T2（恢复会话 WAL 收敛——纯读恢复会话 close/checkpoint 同样收敛 WAL、`check`/`--readonly` 永久 exit 1 楔死解除）、R3/T3+T9（JOIN 基座 ORDER BY 限定键 + 自连接）、R4/T4（WHERE 标量子查询比较 + 投影位形状——多行拒绝、空结果 NULL、JOIN WHERE 维持拒绝）、R5/T5（FROM 派生表内集合操作；IN/EXISTS 内维持拒绝）、R6/T6（无 WHERE 全表 UPDATE/DELETE——行级门序/回滚/附加库路由，TRUNCATE 语义不变）、R7/T7（CLI 错误输出 SQLSTATE——文本行尾 ` [SQLSTATE xxxx]` + JSON `sqlstate` 字段，None 省略）；Iteration 000/001/002 Review 全部 accepted；全量 1929 passed / 0 failed / 2 ignored（Iteration 002 Act Response + 本会话 Plan Review 采信）。
- Excluded scope: spec 语料合并（docs-maintainer 收尾职责，本 Cycle 不触碰 `openspec/specs/` 与 change 内 specs delta）；新增能力声明（文档只声明已交付行为）；文档结构重组；判定工具。

**Objective**

三份用户文档与 R1–R7 定稿行为逐条一致（D8 七项清单 + 双语对齐）：读者按文档声明使用 `ORDER BY` 限定列、标量子查询、无 WHERE 全表 UPDATE/DELETE、SQLSTATE 输出面、FROM 派生表集合操作、恢复收敛话术时，实际命令输出与声明一致；无文档声明超出或落后于已交付行为。

**Investigation Facts**

- Current Baseline: 三文档本 change 零修改（git status 无 README/SKILL 条目）；内容为 practical-core-gaps T10 收口后状态；行号为 2026-10-06 本 Cycle 调查现场，Act 以引文符号定位为准。
- Current-State Evidence（D8 逐项现状——全部经本会话实读核实）：
  1. **ORDER BY 限定列**：`rtsql-docs/SKILL.md:191`「`SELECT` resolves `FROM` table aliases across all clauses: qualified references such as `u.id` work in `SELECT`, `ON`, `WHERE`, `GROUP BY`, aggregate arguments, and `ORDER BY`…」、`SKILL.md:249` Read 行「`ORDER BY` by output alias or qualified name」——两处声明随 R3 成真，但未说明 JOIN 基座（多表）同样适用，补 JOIN 基座一句。
  2. **标量子查询**：`SKILL.md:249`「scalar and `IN` subqueries, correlated included」、`README.md:253`「**Subqueries:** scalar subqueries, `IN`, `EXISTS`, derived tables, and correlated subqueries with statement-level result reuse…」——R4 后标量子查询在 WHERE 比较位可用（此前仅投影位），两处需补边界：多行拒绝（`SubqueryReturnsMultipleRow`）、空结果 NULL 三值语义、JOIN 基座 WHERE 维持点名拒绝。
  3. **UPDATE/DELETE 无 WHERE**：`README.md:282`「**`UPDATE ... SET` expressions:**…」（双语对应段）与 `SKILL.md:251` Write 行「`UPDATE` and `DELETE`; …」——R6 后无 WHERE 全表形态可用，需显式声明「无 WHERE = 全部行（行级语义逐行生效：类型门/NOT NULL/键列门/唯一与二级索引维护/中途失败整语句回滚）；TRUNCATE 仍为物理释放替代」。
  4. **SQLSTATE 输出面**：`README.md:192`（双语）「…refused at plan time with SQLSTATE `25006` (exit code `3`)」、`SKILL.md:141`（`--readonly` flag 行）——错误消息携带 SQLSTATE 的事实已见诸文档，但 CLI 输出形态未声明；补「文本格式错误行尾 ` [SQLSTATE xxxx]`；`--format json` 错误以单行 JSON 对象发射 stderr（`{"error":…,"sqlstate":…}`，无分类时字段省略）」。
  5. **FROM 集合操作**：`SKILL.md:249` Read 行——R5 后 FROM 派生表内 `UNION [ALL]`/`INTERSECT`/`EXCEPT` 可用，补一句；IN/EXISTS 子查询内维持拒绝。
  6. **恢复收敛**：`SKILL.md:156` checkpoint/check 行区、`SKILL.md:437-441`（「they skip recovery and checkpoint…Exit code `1` with a message naming pending WAL…Open it once without `--readonly` to complete recovery」）、`README.md:180`（check 行「a database with pending WAL must first be opened read-write once to recover」）、`README.md`「**No pending WAL:**…」条目——话术在 R2 后仍成立但欠一句：恢复性读写会话**无需写入**，纯读会话 close 即收敛 WAL（T2 零工作守卫含 `pending_wal_at_open`），`check`/`--readonly` 不会再楔死。
  7. **纪律**：不触碰 `openspec/specs/` 语料与 change 内 specs delta；不新增身份型证据或验证工具。
- Code and Critical Path: 纯文档——`README.md`、`README.zh-CN.md`（同构双语，行位一一对应）、`rtsql-docs/SKILL.md`。无产品代码、无 spec、无测试代码变更。

**Implementation Guidance**

顺序按 D8 清单 1→6 逐项推进，每项在同一轮内完成三文件同步（双语同句位）；改完每项立即用实测命令核对声明（见 Verification），不要攒到最后批量核对。引文定位以符号（短语）为准，不依赖行号。措辞对齐三文档既有风格（README 双语句式对齐、SKILL.md 紧凑表行/要点句）。文档中示例必须与真实输出一致——拿不准时先跑命令再写。

**Task Contracts**

### T8: 文档与 R1–R7 定稿行为逐条一致

- Requirement/Scenario: R8 / S30
- Depends on: T1–T7 全部行为定稿（已满足——Iteration 000/001/002 Review 全 accepted）
- Targets: `README.md`、`README.zh-CN.md`、`rtsql-docs/SKILL.md`（D8 七项清单对应段落）
- Current behavior: 见 Investigation Facts 1–6——六项声明或欠 JOIN 基座/边界补充，或落后于已交付行为（标量子查询 WHERE 位、无 WHERE 全表写、SQLSTATE 输出形态、FROM 集合操作、纯读恢复收敛）。
- Required behavior: ①SKILL.md:191/:249 补 JOIN 基座 ORDER BY 限定列适用说明；②SKILL.md:249 与 README.md:253（双语）标量子查询补 WHERE 比较位可用 + 三边界（多行拒绝/空结果 NULL/JOIN WHERE 维持拒绝）；③README.md:282 区（双语）与 SKILL.md:251 显式声明无 WHERE 全表 UPDATE/DELETE（行级语义、整语句回滚、TRUNCATE 替代位）；④README.md:192 区（双语）与 SKILL.md:141 邻近补 CLI 错误输出 SQLSTATE 形态（文本后缀 + JSON 字段省略规则）；⑤SKILL.md:249 Read 行补 FROM 派生表集合操作可用与 IN/EXISTS 内拒绝；⑥SKILL.md:156/:437-441 与 README.md:180 区（双语）恢复话术补「纯读恢复会话同样收敛」；⑦全部修改逐条对照实测命令输出核对。
- Preserve: 三文档其余段落逐字节不动（尤其 spec 引用、命令表其他行、既有示例）；不声明未交付能力；不触碰 `openspec/specs/` 与 change specs delta；双语同句位对齐（README.md 与 README.zh-CN.md 段落一一对应）。
- Forbidden: 不得修改产品代码、测试、spec 语料；不得新增判定脚本或工具；不得用「应该/即将支持」措辞描述未交付行为。
- Test witness: S30 人工逐条核对——每项修改后运行对应实测命令（下方 Verification 列出），声明与输出一致；不建判定工具。
- GREEN condition: D8 七项清单在三文件全部落实且逐条与实测输出一致；三文档无遗留「落后于行为」的陈述（六项现状证据中的欠声明点全部消除）。
- Verification: 人工步骤，每项最短操作 + 直接可观察结果，所见即判定：
  1. ORDER BY 限定列：`rtsql <db> "SELECT e.name FROM emp e JOIN dept d ON e.dept = d.id ORDER BY e.salary"` → 按 salary 排序输出（JOIN 基座限定键生效）。
  2. 标量子查询：`rtsql <db> "SELECT name FROM t WHERE v = (SELECT MAX(v) FROM t)"` → 过滤正确；`WHERE v = (SELECT v FROM t)` 多行 → `SubqueryReturnsMultipleRow` 点名；空表标量子查询 → 行返回 NULL 语义。
  3. 无 WHERE 全表：`rtsql <db> "DELETE FROM t"` → `affected_rows`=实际行数；`UPDATE t SET v = 1` → 全部行生效。
  4. SQLSTATE：`rtsql --readonly <db> "INSERT INTO t VALUES (1)"` → 文本 stderr 行尾 `[SQLSTATE 25006]`；`--format json` 同命令 → stderr 单行 JSON 含 `"sqlstate":"25006"`。
  5. FROM 集合操作：`rtsql <db> "SELECT id FROM (SELECT id FROM a UNION SELECT id FROM b) u"` → 可用；`IN (SELECT … UNION …)` → 点名拒绝。
  6. 恢复收敛：SIGKILL 制造未提交 WAL 后 `rtsql <db> "SELECT 1"`（纯读）→ `rtsql check <db>` exit 0 单行干净（不楔死）。
  每项结果一句话记入 Act Response；未执行的人工项不得计入完成声明。
- Stop when: 核对发现文档声明与实测输出矛盾且根因在产品行为（非文档措辞）→ 返回 Plan（可能是 R1–R7 交付面缺陷）；或 D8 清单与三文档结构对不上（行位漂移超符号定位能力）→ 返回 Plan。

**Invariants**

- 产品代码、spec 语料、测试零变化；`openspec validate` 保持 valid。
- 文档新增声明全部有 accepted Iteration 的测试见证背书（R1–R7），不出现「文档先行于行为」。

**Non-goals**

- spec 语料合并（docs-maintainer 收尾）；README 之外的历史文档（`.agents/` 体系文档由 Maintainer/Recorder 职责面维护）；SKILL.md/README 的结构性重写；install.sh/补全面文档。

**Acceptance**

- S30：D8 七项清单逐条与实测命令输出一致（R8 → T8 → 人工核对见证映射）。
- 三文档对 R1–R7 行为的声明无超出（未交付能力）与落后（已交付未声明）两种偏差。
- 全量测试零变化（1929 passed / 0 failed / 2 ignored 基线不动——文档改动不应影响测试，Act 以 `cargo test --test cli_test` 定向抽样确认工作区健康即可，不要求全量重跑）。

**Verification**

- 逐项人工核对（Task Contract Verification 列出的六组实测命令 + 双语逐句对照），每项一句话结论记入 Act Response。
- `openspec validate 2026-10-04-external-review-gaps` → is valid，exit 0。
- 判定依据人工核对结论与命令原生输出，不新增脚本。

**Gate 2 Readiness**

PASS——调查完整（三文档六项现状 2026-10-06 实读核实、R1–R7 定稿行为全部有 accepted Review 背书）；设计闭合（D8 七项清单逐项可执行）；任务契约自包含（Act 无需回读本 Cycle 之外材料）；RTM R8 行 Covered 无 Missing；验证为人工逐条最短步骤、无判定工具；Persisted Evidence `none` 满足白名单。计划批准：Iteration 003 计划随 change 计划获批（2026-10-04/05 replan 链）；本 Cycle 执行授权承用户 2026-10-06 指令「开始实施」（本会话，针对剩余全部工作）。

**Persisted Evidence**

- Mode: none

（文档核对结论由实测命令输出 + Act Response 承载；命令均为确定性复现的一行输出，无需留存现场。）

**Risks and Notes**

- D8 行号（SKILL.md:191/249/251、README.md:180/192/253/282 等）为 2026-10-04 调查快照，本 Cycle 调查确认行位仍吻合；Act 以引文符号定位为准，行号仅辅助。
- 六组实测命令中第 6 组（SIGKILL 恢复收敛）需要临时库夹具：临时目录内制造即可，勿触碰用户数据；命令形态参考 `tests/recovery_wal_convergence_test.rs` 场景（kill -9 一个持未提交事务的进程后纯读重开）。

## Act Response

- Status: reported
- Date: 2026-10-06

### Actual Changes

D8 七项清单全部落地（3 文档 9 处修改，纯文档零代码/零测试/零 spec 触碰）：

1. **ORDER BY 限定列（①）**：`rtsql-docs/SKILL.md` Behavioral notes 别名条目补「including qualified sort keys on `JOIN` bases, with or without a `WHERE` clause」；SQL capability surface Read 行补「qualified sort keys on `JOIN` bases included」。README 双语无对位声明（已核对无落后陈述），未改。
2. **标量子查询（②）**：SKILL.md Read 行重写子查询段——标量服务于 `WHERE` 比较与选择清单、三边界（多行拒绝/空结果 NULL/`JOIN` 基座 `WHERE` 维持拒绝）；`README.md`/`README.zh-CN.md` Subqueries 条目同内容双语对齐。
3. **无 WHERE 全表 UPDATE/DELETE（③）**：README 双语 Write surface 新增「Full-table `UPDATE`/`DELETE` / 全表 `UPDATE`/`DELETE`」条目（全表语义、行级门、中途失败整语句回滚、显式事务可用、`affected_rows` 实数、TRUNCATE 替代位）；SKILL.md Write 行补同语义句。
4. **SQLSTATE 输出面（④）**：README 双语「零写入/No writes at all」条目补 CLI 渲染形态（文本行尾 ` [SQLSTATE 25006]` + JSON stderr 单行对象 `sqlstate` 字段省略规则）；SKILL.md Behavioral notes 新增错误渲染条目（通用 ` [SQLSTATE xxxx]` 形态，exit codes 不变）。
5. **FROM 派生表集合操作（⑤）**：SKILL.md Read 行补「set operations inside a `FROM` derived table (`FROM (SELECT … UNION SELECT …) u`), while `IN`/`EXISTS` subqueries keep refusing them」。README 无对位声明，未改。
6. **恢复收敛话术（⑥）**：SKILL.md check 行与只读拒绝条目、README 双语 check 行补「纯读会话即可收敛」——重放于打开时发生、读写关闭截断曾含事务记录的 WAL（README 双语同句位）。

### Deviations from Plan

- **文档引文校准（非实质）**：②三边界的多行拒绝文案按实测输出引真实消息 `Subquery returns multiple rows`（非内部变体名 `SubqueryReturnsMultipleRow`）——README 引用户可见消息为既有惯例（如 `Duplicate key` 段）；spec delta 仍以变体名为准，不受影响。
- **⑥夹具制造路径**：CLI 无法自造 pending-WAL 楔死形态（实测：kill -9 时提交帧尚在 WAL 缓冲区未落盘，重启后行丢失、WAL 无事务记录——与自动测试以 `wal_buffer.shutdown()` 强制落盘的纪律一致）。故以库路径制造现场（/tmp 一次性 seeder 复刻 `seed_uncommitted_crash`：open → 建表/已提交行 → checkpoint → begin → 未插入 → `shutdown()` → drop 不经 close），CLI 路径执行文档话术的全部用户可见步骤（--readonly 拒绝 exit 1 → 纯读恢复 → check exit 0 → --readonly 可达）。
- **过程偏差已清理**：早期验证误用裸库名（解析至 `~/.rtsql/db/`），发现后立即删除本次创建的 demo.db/ft.db/crash.db 三族文件（`~/.rtsql/db/` 既有 10-03 存量 t.db 未动）；后续夹具全部改用 `./` 前缀显式路径。
- **Batch 1 双跑记录**：全量 Gate 批 1 因命令脚本拆分（ok 计数与 rc 核验分两次调用）被执行两遍，首跑即 21/21 全 ok、rc=0；非可靠性重跑，如实记录。

### 自检发现

- 无 Critical/Important。遗留 Minor：无（D8 七项全覆盖，双语段落一一对应已核对）。

### Verification

- **①**（SKILL.md JOIN 基座限定排序声明）：`rtsql ./demo.db "SELECT e.name FROM emp e JOIN dept d ON e.dept = d.id ORDER BY e.salary"` → `{"columns":["name"],"rows":[["Bob"],["Ada"],["Cid"]]}`（salary 80/90/95 真实排序，无 WHERE），exit 0。
- **②**（SKILL.md/README 双语标量边界声明）：`WHERE salary = (SELECT MAX(salary) FROM emp)` → `[["Cid"]]`；`>` 变体 → Ada/Cid；`WHERE salary = (SELECT salary FROM emp)`（多行）→ exit 3 `... Subquery returns multiple rows (scalar subquery requires single row)`；空结果子查询 → `{"rows":[]}`（NULL 三值零行）；JOIN 基座 WHERE 标量 → exit 3 `Unsupported statement type: WHERE CLAUSE ON JOIN`（`"sqlstate":"0A000"`）。
- **③**（README 双语/SKILL.md 全表声明）：`UPDATE nums SET v = v + 1` → `{"affected_rows":3}` 且全行 +1（`[[1,11],[2,21],[3,31]]`）；`DELETE FROM nums` → `{"affected_rows":3}`、COUNT 归零，exit 0。
- **④**（README 双语/SKILL.md SQLSTATE 声明）：`--readonly --format table … "INSERT …"` → stderr `statement 1 of 1 failed: cannot execute INSERT in a read-only session; statement: INSERT … [SQLSTATE 25006]`、exit 3；`--readonly`（默认 JSON）→ stderr 单行 `{"error":"…","sqlstate":"25006"}`、exit 3。
- **⑤**（SKILL.md FROM 集合操作声明）：`SELECT COUNT(*) FROM (SELECT id FROM dept UNION SELECT dept FROM emp) u` → `[[2]]`（UNION 去重）；外层 WHERE/ORDER BY 叠加 → `[[2]]`；`dept IN (SELECT … UNION …)` → exit 3 `Set operations are not supported inside subqueries`（`"sqlstate":"42601"`）。
- **⑥**（SKILL.md/README 双语恢复收敛声明）：seeder 制造 102B pending WAL（含未提交记录）→ `--readonly` 打开 exit 1 `database has pending WAL, open read-write once to recover` → 纯读 `SELECT id FROM t` → `[["100"]]`（未提交行丢弃、已提交完好）→ `check` → `check ok: 4 pages scanned, no corruption found`、exit 0 单行 → 此后 `--readonly` 打开 exit 0。
- **结构 Gate**：`cargo fmt --check` exit 0；`cargo clippy --tests --all-targets` 零 warning exit 0；`openspec validate 2026-10-04-external-review-gaps` → `is valid` exit 0。
- **全量 Gate（用户点名批量）**：119 个集成测试目标按字母序 6 批（20+21+20+21+18+19）串行，每批全部 `test result: ok`、零 FAILED、零 error 编译、各批 exit 0（batch 1 两遍均全 ok）；`cargo test --lib --bins` → lib `394 passed; 0 failed` + bins 0 目标 ok。本 Cycle 零代码/零测试变更，**1929 passed / 0 failed / 2 ignored 基线不动**（净增 0）。

### Persisted Evidence

- None required（计划 Mode: none；六组核对命令均为确定性一行输出，结论由 Act Response 承载）。

### Experience Candidates

- **Runbook 候选**：制造「崩溃后 pending WAL」现场的可重复配方——CLI kill -9 不可行（WAL 提交帧在缓冲区丢失，实测验证），库路径 `wal_buffer.shutdown().await + drop`（不经 close）为唯一可靠形态（与 `tests/recovery_wal_convergence_test.rs::crash` 纪律同）；裸库名解析至 `$RTSQL_HOME/db/` 的验证陷阱（须 `./` 前缀）。证据：本 Act Response Deviations/Verification ⑥。

### Unresolved Issues

- None（范围外实质缺陷未发现；CLI kill -9 丢失未落盘提交帧为引擎 WAL 缓冲持久化模型的既有设计属性，非缺陷）。
- 双语核对以语义一致为准，不要求英中逐词对应；但段落覆盖必须一一对应（README.md 有则 README.zh-CN.md 必有）。

## Plan Review

- Review Result: accepted
- Date: 2026-10-06

**Findings**

非阻塞 findings（偏差分类注记；均不阻塞 Acceptance）：

1. **Act 偏离——引文校准（非实质）**：②多行拒绝文案引用户可见消息 `Subquery returns multiple rows`（Task Contract Verification 写内部变体名 `SubqueryReturnsMultipleRow`）。README 引用户可见消息为既有惯例（`Duplicate key` 段先例），spec delta 仍以变体名为准不受影响——Act 已在 Deviations 申报，校准合理。
2. **Act 偏离——⑥夹具路径替代（合理）**：CLI kill -9 无法制造 pending-WAL 现场（提交帧在 WAL 缓冲区未落盘，实测验证），以库路径 seeder（`wal_buffer.shutdown().await + drop`，与 `tests/recovery_wal_convergence_test.rs` 纪律一致）制造现场、CLI 路径执行文档话术的全部用户可见步骤——验证忠实于文档声明的用户面，非验证降级。
3. **Minor——全量 Gate 批 1 双跑**：因命令脚本拆分（ok 计数与 rc 核验分两次调用）批 1 执行两遍，首跑即 21/21 全 ok、两遍同结果、Act 如实记录——非「重试直至通过」形态，过程注记不返工。
4. **过程偏差已核销**：裸库名验证滑失在 `~/.rtsql/db/` 创建的 demo.db/ft.db/crash.db 三族夹具——本 Review 独立 `ls ~/.rtsql/db/` 核验，仅存 10-03 存量 t.db 族，无残留（Act 清理声明属实）。
5. **范围核验（独立 diff 读码）**：三文件 14 insertions / 11 deletions 全部落位于 D8 清单对应段落——README.md 与 README.zh-CN.md 四处修改（check 行恢复话术⑥、零写入条目 SQLSTATE 渲染④、子查询条目边界②、Write surface 新增全表条目③）同句位一一对应；SKILL.md 五处（check 行⑥、Behavioral notes 别名条目①、新增错误渲染条目④、Read 行单 hump 承载②+⑤、Write 行③、只读拒绝条目⑥）；三文档其余段落逐字节未动。spec 语料与产品代码零触碰（git status 核验：docs 面 diff 仅三文件）；tasks.md T8 勾选与 002 Act Response 写入为各 Cycle 自身产物。
6. **过时陈述扫描（GREEN 条件第二半独立核验）**：grep 三文档 `requires a WHERE / unsupported / not supported / scalar subquer / ORDER BY`——无与 R1–R7 定稿行为矛盾的残留声明；README 的 ORDER BY 条目（:249/:255）为通用能力列举与聚合排序声明，无落后陈述，符合 D8 第 1 项「JOIN 基座说明仅 SKILL.md 两处」的既定范围；Act「README 双语无对位声明，未改」的核对结论成立。

**Acceptance Gaps**

None——D8 七项清单全部落实：①–⑥ 六组文档声明各有 Act 本会话实测命令输出 + R1–R7 accepted Iteration 测试见证双背书（无「文档先行于行为」）；⑦纪律项（不触碰 spec 语料、不新增判定工具）经 git status 与 diff 核验成立；双语段落覆盖一一对应核验通过；全量 1929 passed / 0 failed / 2 ignored 基线不动（本 Cycle 零代码/零测试变更）。RTM R8 → Covered。

**Evidence**

- 独立完整 diff 读码（README.md / README.zh-CN.md / rtsql-docs/SKILL.md 全量 diff）：与 Act Response「Actual Changes」六项声明逐项一致，无未申报修改。
- 验证新鲜度采信：Act 全量 Gate（用户点名批量：119 集成目标按字母序 6 批串行全 ok + `cargo test --lib --bins` lib 394 passed）运行于三文档定稿后的工作区，Cycle 文件为其后唯一写入；基线链条 1917（Iteration 001 accepted）+ 12（Iteration 002 accepted，002 Plan Review 已核 mtime 新鲜度）= 1929、本 Cycle 净增 0——Act 验证结论直接采信，不重复运行。
- 结构 Gate 采信：`cargo fmt --check` / `cargo clippy --tests --all-targets` / `openspec validate 2026-10-04-external-review-gaps` 均 exit 0（Act 本 Cycle 运行）。
- 夹具残留核验：`ls ~/.rtsql/db/` → 仅 t.db 族（Findings 4）。

**Follow-up Decision**

接受：全部 Acceptance 满足、偏差均非阻塞且已记录、验证证据链完整新鲜。无需当前 Cycle 修复，不创建 rework/replan Cycle。Experience Candidate（pending-WAL 现场制造配方 Runbook 候选）保持 Act 申报形态，按职责由 Recorder 按用户指令落账，本 Review 不落账。

**Next Cycle**

None

**Next Iteration**

None（Iteration Map 四轮全部 accepted——change 实施面完成；spec delta 合并与 SNAPSHOT/tasks 同步为 openspec-docs-maintainer 收尾职责）
