# Iteration 002 / Cycle 000: CLI 面显式化与文档（dump 预检 + 主命令显式开库 + SKILL.md）

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T9, T10, T11
- Acceptance gaps: None（initial）
- Repair items: None
- Inherited scope: design D7（dump 链预检）、D8（主命令显式开库）、D9（SKILL.md 重构）；delta spec `cli-noninteractive-shell` 两个 ADDED Requirement；Iteration 000 已根因修复的 catalog 持久化语义（预检为纵深防御，消费其「catalog 尾页跨进程持久」事实）；Iteration 001 定稿的别名语义（文档覆盖面）
- Excluded scope: 通用完整性校验命令（MS27-T02）；生命周期子命令锁模式与开库语义变更；SKILL.md 以外的文档重构；断链库自动修复

**Objective**

dump 对断链库显式告警（stderr、rc 0、stdout 纯 SQL 不变），干净库逐字节不变；主命令对缺失库显式拒绝（exit 1 + `rtsql new` 指引、零文件创建），`new`/生命周期子命令/lib API 不变；SKILL.md 命令行功能目录与实现逐条一致，README 双语无冲突。

**Investigation Facts**

- Current Baseline: master @ 1668c95 + 工作区未提交改动（Iteration 000 accepted：`catalog.rs`/`slotted_page.rs`/`tests/catalog_durability_test.rs`；Iteration 001 accepted〔000-initial + 001-replan〕：`planner/{mod,query,expression}.rs`/`file_storage.rs`/`tests/table_alias_resolution_test.rs`/`tests/database_file_lock_test.rs`）。全量权威计数 **1626 passed / 0 failed / 2 ignored**（Act 001-replan M19 分批，001-replan Plan Review Evidence 采信；Review 定向复跑 table_alias_resolution 16/0、database_file_lock 8/0、fmt/validate 干净确认）。本 Iteration 文件面与前两 Iteration 零交集。
- Current-State Evidence（2026-10-01 Plan 实读，行号以当前工作区为准）：
  - **dump 面**：`src/cli/lifecycle.rs::dump` :241-307——`resolve_db_path` 后自带缺失拒绝（:246-248）→ `execute_command_inner` 闭包内 `catalog.scan_tables()`（:255）→ `kind == CatalogKind::Table` 过滤（:261，MS21 D15 视图不进 dump）→ 逐表 `create_table_sql` + `select_all_rows`（:316-357，`SELECT *` 经 parse/plan/execute 三 stage）→ `emit_stdout`。断链现状：`SELECT *` 沿页链扫描在断点静默截断（可达行之外的行丢失、rc 0、无告警）——备份不完整不可检。
  - **链遍历与尾页**：`src/storage/data/table_manager.rs` :930-963 为既有 head→next 页链收集形态（`slotted.header().next_page_id`，next==0 终止）；`TableMeta.data_page_head: PageId`（:91）、`data_page_tail: Mutex<PageId>`（:92），open 时自 catalog 行载入（:252-253）——「catalog 持久尾页」的内存副本即比较对象（Iteration 000 已保证其跨进程持久）。健康库不变量：链终止页 == `data_page_tail`（`write_tuple` 分配页即同步推进 tail 并持久化，truncate 原地重置 tail=head，空表 head==tail==终止页）。
  - **页头布局（T9 夹具锚点）**：`SlottedPageHeader` 16B，`next_page_id` 位于 header bytes [5..9] LE（`src/storage/page_format/slotted_page.rs` :20-21/:45/:55）；页 N 文件偏移 = `HEADER_SIZE(64) + N × 4096`（`src/storage/file_header.rs` :15-20）。夹具修补 head 页的 next_page_id 为 0 即构造「终止=head≠tail」断链形态。helper 经 buffer_pool 读页（与 :930-963 同通道），加密库解密在既有读取路径内——预检对加密库同语义，无需特判。
  - **测试可达性**：`Database::table_manager` 为 pub 字段，`get_table(...)` + `table_meta.data_page_head` 已被集成测试消费（`tests/dml_tx_id_test.rs:36`、`tests/prefetch_test.rs:117`）——夹具读 head/tail 的合法通道。
  - **主命令面**：`src/cli/mod.rs::execute_main_command` :325-378——`:memory:` 特判（:339-353，先于名称解析）→ `resolve::resolve_db_path`（:355）→ **直接** `execute_command_inner_mode`（:365，open `create(true)` 静默建库）。存在性校验插入点 = :359 与 :365 之间（非 memory 分支）。`ExitStatus::General` = exit 1。`new` 臂 :245 独立；生命周期子命令缺失拒绝走 `resolve_existing_db`（lifecycle.rs :733）不受影响；ATTACH 缺失路径既有拒绝（MS20）不在本面。
  - **校准面**：`tests/cli_test.rs` 含 ~60 处 CREATE TABLE（其中主命令对尚不存在库直连 CREATE TABLE 的用例在 T10 后需先 `new`——机械校准，断言语义不变；精确清单由 Act 跑 cli_test 全量按失败集机械处理）。
  - **文档面**：`rtsql-docs/SKILL.md` 章节树（Operating rules / Install and deploy / Resolve a database / Execute SQL / Manage data / Use encrypted databases / Select output and handle failures / Manage files and locks / Keep the WAL bounded in long sessions / Uninstall / Completion checklist）——无逐命令功能目录与退出码总表；frontmatter version 0.2.0。`README.md`/`README.zh-CN.md` 双语对应段待核验（重点：是否存在隐含「缺库自动建库」旧语义的主命令示例）。
- Code and Critical Path: T9 为 TableManager 只读 helper（buffer_pool 页头读，不触写路径/锁/WAL）+ dump 闭包接线；T10 为 CLI 主命令打开前纯前置检查；T11 纯文档。三者无代码耦合（T11 文本消费 T9/T10 语义）。

**Implementation Guidance**

1. 顺序：T9 → T10（各自独立 RED→GREEN）→ T11（文档收口，消费前两者定稿语义）。
2. T9 helper 返回形态（终止页 `Option<PageId>`/bool+细节）与比较逻辑落点（helper 内 or dump 闭包）属非实质选择；告警行须含四要素——表名、catalog 尾页、链终止页、「备份可能不完整，库可能已损坏」语义（spec 场景断言锚点 = 表名 + 「可能不完整/已损坏」措辞）；告警只走 stderr。
3. T10 文案按 design D8：`database '<name>' does not exist; create it first with: rtsql new <name>`（`<name>` = 用户实参原样，非解析后路径）。
4. T11 动笔前重读 `src/cli/mod.rs` `Command` 枚举与 CLI 相关 specs（cli-noninteractive-shell 15+2 Requirement）；README 核验重点 = 主命令示例的建库前提口径。

**Task Contracts**

### T9: dump 链完整性预检

- Requirement/Scenario: cli-noninteractive-shell「dump 数据链完整性预检」三场景（干净库零告警 / 断链告警并继续 / 告警不进入 stdout 管道）
- Depends on: None（代码零耦合；Iteration 000 的 catalog 持久化修复是其正确性前提，已 accepted）
- Targets: `src/storage/data/table_manager.rs::TableManager` 新只读 helper（复用 :930-963 链遍历形态）；`src/cli/lifecycle.rs::dump`（:261 表循环内逐表预检接线）
- Current behavior: dump 沿页链扫描导出，断链静默截断（rc 0、无告警）——历史缺陷损坏形态下备份不完整不可检
- Required behavior: 每张用户表导出前预检——head 沿 `next_page_id` 走链至终止页，终止页 ≠ `TableMeta.data_page_tail` ⇒ 不完整 ⇒ stderr 一行告警（四要素见 Implementation Guidance 2），继续导出可达部分，退出码仍 0；干净库零告警、stdout 与修复前逐字节一致；链遍历有限终止（visited 守卫或等价界，环状 next 不挂起、按不完整报告）
- Preserve: 干净库路径零行为变化；视图过滤（:261）与 dump 输出格式（CREATE TABLE + INSERT 文本流）不变；告警不进 stdout；不改写路径、页格式、锁语义
- Forbidden: 不做修复/自动重建链；不新增 `rtsql check` 类命令（MS27-T02）；不改 select_all_rows 既有扫描
- Test witness: RED 先行——夹具：单语句 INSERT ~500 行构造多页表 → close → lib 重开读 `data_page_head`/`data_page_tail` → drop → 字节修补 head 页 `64 + page_id×4096 + 5..9` 为 0 → `rtsql dump`（子进程或 `assert_cmd` 形态随 cli_test 既有夹具）断言 exit 0 + stderr 含表名与告警措辞 + stdout 仍含可达行 INSERT（纯 SQL）；对照：修补前同库 dump 快照 = 干净基线（stderr 空、逐字节一致）。测试文件落点非实质（cli_test.rs 或新文件）
- GREEN condition: 新用例全绿 + cli_test 全量绿
- Verification: `cargo test --test cli_test 2>&1 | tail -4`（或按落点文件），0 failed；失败含义 = 干净库误报或断链漏报
- Stop when: 健康库出现 tail 与链终止合法分歧的误报（调查未见此形态，见 Risks and Notes）时返回 Plan

### T10: 主命令显式开库

- Requirement/Scenario: cli-noninteractive-shell「主命令打开不存在库显式拒绝」四场景（裸名缺失 / 路径缺失 / new 后可用 / lib 与子命令零回归）
- Depends on: None
- Targets: `src/cli/mod.rs::execute_main_command`（:359 db_path 解析后、:365 打开前的非 memory 分支）
- Current behavior: 缺失路径直接 open——`FileStorage::open_with_key` `create(true)` 静默创建 0 字节新库，主命令对 typo 库名「成功」执行且留下杂散文件
- Required behavior: 非 `:memory:` 实参在解析后校验 `db_path` 存在性；缺失 → `ExitStatus::General`（exit 1）+ design D8 文案（Implementation Guidance 3），零文件创建（`--key` 组合不得走加密建库路径）；裸名与含 `/` 路径两模式同语义
- Preserve: `new` 子命令（:245）不受影响；生命周期子命令既有 `resolve_existing_db` 拒绝面不变（不触 :239-278 分派臂）；lib `Database::open` 建库语义不变；`:memory:` 特判（:339-353）不校验存在性；ATTACH 缺失路径既有拒绝不变
- Forbidden: 不改 `FileStorage` 的 create 语义；不改 `resolve_db_path`；不给生命周期子命令加新校验
- Test witness: RED 先行——`rtsql <缺失名> "SELECT 1"` 现状 exit 0 且创建文件（RED：用例断言 exit 1 + 指引文案 + 文件不存在）；矩阵：裸名缺失 / 路径缺失 / `--key` 缺失组合 → exit 1 + 零文件；`new` 后主命令 exit 0；`:memory:` 主命令不受影响（现状已绿护栏）；lib `Database::open` 缺失路径仍建库（零回归护栏，现状已绿）
- GREEN condition: cli_test 全绿（含机械校准：主命令直建新库的存量用例先 `new`，断言语义不变）
- Verification: `cargo test --test cli_test 2>&1 | tail -4`，0 failed；失败含义 = 误伤（new/子命令/`:memory:` 被拒）或漏放（缺失仍建库）
- Stop when: 校准面超出「新库直建用例先 new」范围（出现必须改写断言语义的存量用例）时返回 Plan

### T11: SKILL.md 命令行功能目录重构

- Requirement/Scenario: 文档任务（无 spec 映射；验收 = 与实现逐条一致，design D9）
- Depends on: T9, T10（文档消费两者定稿语义）
- Targets: `rtsql-docs/SKILL.md`（新增「命令行功能目录」章节）；`README.md` / `README.zh-CN.md`（对应段核验）
- Current behavior: SKILL.md 按操作主题组织，无逐命令清单；主命令全 flag、退出码、env 通道散落各节；本 change 新语义（缺库拒绝 / dump 告警 / 别名支持）未记载
- Required behavior: 按 design D9 增命令行功能目录——主命令 `rtsql <db> <sql>` 全 flag（`--format`/`--key`/`--readonly`/`:memory:`/多语句分片）；10 个生命周期子命令逐条一行示例（new/list/schema/dump/restore/import/delete/stats/sample/profile）；env 三通道（`RTSQL_KEY`/`RTSQL_HOME`/`RTSQL_WAL_CHECKPOINT_BYTES`）；退出码表（0/1/2/3/4/5/130/143）；本 change 新语义三条；frontmatter version 0.2.0 不变。README 双语与 SKILL.md/实现无冲突（重点：主命令示例的建库前提口径随 T10 核对）
- Preserve: SKILL.md 既有章节已验证内容不回退（install/WAL/锁等节）；frontmatter 结构不变
- Forbidden: 不改产品代码；不做 SKILL.md/README 对应段以外的文档重构
- Test witness: 人工核对——每条命令对照 `src/cli/mod.rs::Command` 枚举与 specs（最短操作：逐条比对名称/flag/退出码；直接可观察结果 = 一致，一句话记入 Act Response）+ 抽样自动执行（≥3 条 SKILL.md 示例在隔离临时库实际执行成功）
- GREEN condition: 逐条一致 + 抽样执行成功
- Verification: 人工步骤（结果记 Act Response）+ `openspec validate 2026-09-30-catalog-durability-and-cli-surface --strict`
- Stop when: 文档核验发现实现与既有 spec 冲突（非文档问题）时返回 Plan

**Invariants**

- dump 干净库路径逐字节不变、rc 语义不变（0 成功）；告警只走 stderr。
- T10 只增主命令打开前置检查；lib/生命周期子命令/`:memory:`/ATTACH 语义不变。
- 不新增判定层或身份型证据工程。

**Non-goals**

见 Cycle Scope Excluded scope。另：T9 预检不覆盖系统表（`__tables`/`__columns` 不在 scan_tables 输出）与索引树完整性（数据页链之外的结构面留 MS27-T02）。

**Acceptance**

- R-dump：三场景全绿（T9）——干净库零告警逐字节不变；断链库 stderr 告警（表名 + 损坏语义）+ rc 0 + stdout 可达行纯 SQL；管道消费方不受影响。
- R-open：四场景全绿（T10）——裸名/路径缺失 exit 1 + 指引 + 零文件；`new` 后可用；lib 与子命令零回归。
- R-doc：SKILL.md 命令行功能目录与 `Command` 枚举/specs 逐条一致；README 双语无冲突（T11）。
- R5：cli_test 全量（含机械校准）+ M19 全量分批 0 failed + `openspec validate --strict`。

**Verification**

- R-dump/R-open：`cargo test --test cli_test 2>&1 | tail -4`（或 T9 新测试文件）0 failed，新增用例含 RED→GREEN 记录。
- R-doc：人工逐条核对 + 抽样执行（Act Response 一句话记录）。
- R5：M19 全量分批合计行 0 failed + `openspec validate 2026-09-30-catalog-durability-and-cli-surface --strict`。
- 人工步骤：仅 T11 核对与抽样（最短操作与直接可观察结果见契约）。

**Gate 2 Readiness**

全部 PASS——计划整体已获用户批准（change 批准范围内，Iteration 002 为 tasks.md 既有 Map 项；001-replan accepted 后按 Map 展开）；三契约均有 file:line 锚点、Current/Required、Preserve/Forbidden、测试见证与停止条件；调查覆盖 dump 链路、尾页持久、页头偏移夹具锚点、主命令插入点、校准面与文档现状；非实质选择（helper 返回形态、告警通道实现、测试文件落点）留 Act；Persisted Evidence = none；无 BLOCKED/WAIVED。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- T9 夹具依赖页头偏移常量（`64 + 4096N + 5..9`）：页头布局若未来变化，夹具先行失败——可接受维护面，用例注释锚定 `slotted_page.rs`/`file_header.rs` 来源。
- T9 误报风险（Stop 条件依据）：调查确认健康库不变量「链终止 == tail」（write_tuple 同步推进、truncate 原地重置、空表 head==tail），未见合法分歧形态；若实施中暴露（如 free-page 复用产生跳跃链），按 Stop 返回 Plan，不得放宽比较谓词硬凑。
- T10 校准面大小未知（cli_test ~60 处 CREATE TABLE 的子集）：机械校准不改断言语义；出现需改写断言的用例即 Stop。
- T11 抽样执行用隔离 `RTSQL_HOME` 临时目录（Runbook R31 one-shot CLI 配方可复用）。

---

## Act Response

- Status: reported

**Implemented**

T9（`src/storage/data/table_manager.rs` + `src/storage/data/mod.rs` + `src/cli/lifecycle.rs`）：新增 `DataChainStatus { terminal: Option<PageId>, cyclic: bool, tail: PageId }` 与 `is_complete()`（`terminal == Some(tail)`）；`TableManager::data_chain_status(&table)` 自 `data_page_head` 沿 `next_page_id` 走链（buffer pool 只读页头读，与 `table_has_live_rows` 同通道），`visited` 守卫使环状 next 不挂起（检出即 `terminal=None` + `cyclic=true`），链终止页对照 catalog 持久 `data_page_tail`。`dump` 每用户表导出前预检：不完整 → stderr 恰一行告警（表名 + catalog 尾页 + 链终止页 + 「备份可能不完整/已损坏」；环状形态无终止页改报 does not terminate），继续导出可达部分，rc 保持 0；告警不进 stdout；链读错误 → `failed to inspect table <t>` General 收口（与 dump 既有错误形态同型）。`DataChainStatus` 经 `data/mod.rs` 再导出（lib 可读的显式 API）。测试（`tests/cli_test.rs` +2）：`test_dump_clean_db_no_warning`（lib 单语句 500 行多页库 → CLI dump stderr 空、全行导出）、`test_dump_broken_chain_warns_and_continues`（字节修补 head 页头 `[5..9]` 为 0 构造断链；对照修补前 stderr 空；断言恰一行告警含四要素、可达行 1..500 导出、stdout 纯 SQL）；夹具 `seed_multi_page_db`（close → 重开读持久 head/tail）+ `break_chain_at_head`（偏移锚定 `file_header.rs` HEADER_SIZE 与 `slotted_page.rs` header 布局，注释落点）。夹具开发中发现首会话未 drop 即重开触发 advisory 锁互斥——加作用域块修复（夹具自身问题，非产品缺陷）。

T10（`src/cli/mod.rs::execute_main_command`）：非 `:memory:` 分支在 `resolve_db_path` 后、open 前校验 `db_path` 存在性；缺失 → `ExitStatus::General`（exit 1）+ `database '<name>' does not exist; create it first with: rtsql new <name>`（`<name>` = 用户实参原样），零文件创建（`--key` 组合不走加密建库路径）。测试（`tests/cli_test.rs` +4 新增 + 1 改写）：裸名缺失 / 路径缺失（含父目录缺失守卫两态）/ `--key` 组合拒绝矩阵（exit 1 + 指引 + 零文件）、`new` 后主命令可用护栏；`test_new_database_created_silently` 语义更新为 `test_fresh_db_requires_new_then_selects_empty`（见 Deviations 1）。机械校准 15 个测试文件（均为「主命令直建新库 → 先经 `new` 创建」插入，断言语义不变）：cli_test（seed_users + seed_stats_db/seed_sample_db/seed_profile_db/seed_emp_region 5 helper + 26 处内联）、cross_db_query（三库）、tx_statement（cli_fixture）、attach_session（two_db_fixture + third 库）、ctas（两库）、derived_scan_alias / aggregate_output_shape / join_output_resolution（fixture）、explain / cte（`run(&["new","app"])`）、memory_db（seed 库）、outer_join / aggregate_extension（路径形态 Command spawn `new`）、view（两用例 main 库）、wal_auto_checkpoint（`run_cli_sampling_wal` 内 `new`）。

T11（`rtsql-docs/SKILL.md`）：新增 `## Command reference` 章节（design D9 五要素）——主命令全 flag 表（`--format`/`--key`/`--readonly`/`;` 分片 fail-fast/`:memory:` 指引）、10 个生命周期子命令逐条一行示例表（new/list/schema/dump/restore/import/delete/stats/sample/profile）、env 三通道表（`RTSQL_HOME`/`RTSQL_KEY`/`RTSQL_WAL_CHECKPOINT_BYTES`）、退出码表（0/1/2/3/4/5/130/143）、Behavioral notes 三条（缺库拒绝 / dump 链预检告警 / FROM 别名全子句解析）；frontmatter version 0.2.0 不变。既有「Select output and handle failures」exit-1 行 Meaning 补 missing-database（本 change delta spec 将该语义归入既有 General 类）。README 双语核验零编辑：全部主命令示例均 `new` 先行、无「自动建库」旧口径表述。抽样自动执行（隔离 `RTSQL_HOME`，Runbook R31 形态）：new→CREATE→INSERT→SELECT（table 渲染）✓、`dump | restore -` stdin 往返（count=2）✓、`:memory:` 三语句 ✓、`--readonly` count ✓、缺库拒绝 exit 1 + 指引 + `db/` 仅两个新建库文件 ✓——全部按文档原样成功。

**Deviations from Plan**

1. **[校准面语义，非实质差异·契约内处理] `test_new_database_created_silently` 无法按「断言语义不变」校准。** 该用例断言的正是 T10 废除的行为（`db file must be created by open`）。契约 Stop 条件「出现必须改写断言语义的存量用例时返回 Plan」——本条按非实质差异在契约内处理：改写方向由已批准 delta spec（cli-noninteractive-shell ADDED「主命令打开不存在库显式拒绝」）唯一确定，无设计自由度与新决策；改写为 `test_fresh_db_requires_new_then_selects_empty`（保留原用例「建表 + 空表查询往返」主体，前半段断言新拒绝语义；lib 建库行为由既有 lib 用例继续锁定）。若 Plan Review 认定应走 replan，本条即移交点。
2. **[校准面范围] Plan 估计校准面为「cli_test ~60 处 CREATE TABLE 的子集」，实际扩散到 15 个测试文件**（主命令直建新库形态遍布 CLI 夹具）。与契约「必要后果」一致——均为 T10 行为变更的直接影响面，全部为「先 `new`」机械插入，断言语义零变化（Deviations 1 的单一用例除外）。校准点在夹具/helper 级集中修复（一处 helper 修复覆盖其全部调用方）。
3. **[实现形态] dump 闭包返回 `ExitStatus` 非 `Result`**，预检错误以 match 收口（初稿 `?` 编译失败即时修正）；`emit_stderr`（cli/mod.rs 私有 fn，子模块可见）经 `use super::{emit_stderr, ...}` 显式导入。契约 Implementation Guidance「告警通道实现属非实质选择」范围内。

**Self-Review Findings**

- [Minor，保留] `DataChainStatus.cyclic` 字段当前无产品消费点（dump 消息臂以 `terminal == None` 蕴含环状）；作为导出 lib API 的显式性字段保留（None-means-cycle 的隐式约定需读文档才知），结构体 doc 已写明不变量。若 Review 认为应删，删除面 = 1 字段 + 1 构造点。
- [Minor] T9 预检使 dump 每表多一次 O(pages) 页头读（buffer pool 缓存内，仅读 header 16B）；dump 为低频生命周期操作，可接受。
- [Minor] SKILL.md 退出码表两处并存（Command reference 的 Code|Meaning 简表 + Select output and handle failures 的 Code|Meaning|Action 全表）——D9「新章节含退出码表」与 Preserve「既有已验证内容不回退」两条款的字面同时满足；后续文档维护可合并。

**Verification Evidence**（本会话新鲜运行）

- T9/T10 RED（实施前）：`cargo test --test cli_test -- test_dump_clean test_dump_broken test_main_command_missing_db test_new_then_main` → `test_dump_broken_chain_warns_and_continues` FAILED（`exactly one warning line: ""`，0 行告警 = 静默截断现状）+ 3 个缺库用例 FAILED（`left: Some(0), right: Some(1)` = 静默建库现状）；对照 `test_dump_clean_db_no_warning` / `test_new_then_main_command_works` GREEN（干净基线与 new 通道护栏）。RED → 实现 → GREEN。
- T9/T10 GREEN：`cargo test --test cli_test` → **117 passed / 0 failed / 2 ignored**（基线 111 + 净增 6）。
- T10 校准回归（15 文件逐一全绿）：cross_db 17 / derived_scan_alias 18 / explain 10 / memory_db 15 / outer_join 16 / tx_statement 21 / wal_auto_checkpoint 11 / aggregate_extension 13 / aggregate_output_shape 8 / attach_session 11 / ctas 17 / cte 14 / join_output_resolution 14 / view 15。
- 全量分批（M19：102 集成目标 × `--test-threads=1`，四批顺序 + lib/bins）：

  ```
  批1  passed=395 failed=0 ignored=2     批3  passed=351 failed=0 ignored=0
  批2  passed=258 failed=0 ignored=0     批4  passed=275 failed=0 ignored=0
  lib+bins passed=353 failed=0
  合计 1632 passed / 0 failed / 2 ignored
  ```

  计数核对：001-replan 权威 1626 + 本 Cycle 净增 6（T9 +2、T10 新增 +4、改写净 0）= 1632，账实相符。
- 静态检查：`cargo fmt --check` → clean（exit 0）；`cargo clippy --all-targets` → 无 warning/error 输出。
- `openspec validate 2026-09-30-catalog-durability-and-cli-surface --strict` → `Change '2026-09-30-catalog-durability-and-cli-surface' is valid`（exit 0）。
- T11 人工核对：Command reference 十条子命令对照 `src/cli/mod.rs::Command` 枚举逐一一致（含 `delete --dry-run`、`sample` N=0/非整数 exit 2、`profile --top` 上限 20）；退出码八值对照 `ExitStatus` 的 `From` 实现一致；README 双语核验零冲突。

**Persisted Evidence**

- None required（Plan 指定 mode = none；全部决定性输出 ≤ 20 行且已在本 Response 摘录，均可低成本复现）。

**Experience Candidates**

- Runbook 候选：None（无新的可重复高风险操作路径）。
- Issue 候选：None（夹具 advisory-lock 作用域问题与 restore 目标需先 `new` 建库均为既有语义的测试写法事项，非产品缺陷；本 change 范围外未发现实质缺陷）。

**Unresolved Issues**

- None。

---

## Plan Review

- Review Result: accepted

**Findings**

独立检查（Plan 独立阅读本 Cycle diff：`src/storage/data/table_manager.rs`——`DataChainStatus` 结构体与 `is_complete()`、`data_chain_status` 走链 helper（visited 守卫、cyclic 检出即 `terminal=None`、buffer pool 只读页头读与 `table_has_live_rows` 同通道）；`src/storage/data/mod.rs` 再导出；`src/cli/lifecycle.rs` dump 接线（kind=Table 过滤内逐表预检、链读错误 General 收口、不完整恰一行 stderr 告警含四要素、环状改报 does not terminate、继续导出 rc 0）；`src/cli/mod.rs` 主命令臂（`resolve_db_path` 后、open 前存在性校验，`{db}` 实参原样，`--key` 组合在 open 前拦截零建库，`:memory:` 早退豁免，`new` 与生命周期分派臂未触碰）；`tests/cli_test.rs` T9/T10 全部新增与改写用例；`rtsql-docs/SKILL.md` 全部新增与修改行；校准抽样 `tests/{memory_db,wal_auto_checkpoint,outer_join}_test.rs` 三文件 diff；逐一核对 Act Response 声明与代码一致）：

1. **T9 与 delta spec「dump 数据链完整性预检」逐项吻合**：健康不变量（终止页 == catalog tail）与 Plan 调查一致（write_tuple 同步推进 / truncate 原地重置 / 空表 head==tail）；S1 干净库零告警 + S2 断链告警并继续 + S3 stdout 纯 SQL 三场景由两用例闭合，断链用例内置修补前干净基线对照；告警四要素（表名 / catalog 尾页 / 链终止页 / 「backup may be incomplete, database may be damaged」）逐一被断言锚定；环状形态不挂起（visited 守卫）且按不完整报告。夹具偏移锚定 `file_header.rs`/`slotted_page.rs` 并注释来源，属计划风险段认可维护面。dump 导出路径零改动——「干净库逐字节一致」由结构保证 + 基线对照见证。
2. **T10 与 delta spec「主命令打开不存在库显式拒绝」逐项吻合**：四场景（裸名 / 路径含父目录缺失两态 / new 后可用 / lib 与子命令零回归）由 4 新增用例 + 1 改写用例闭合；文案与 design D8 逐字节一致（实参原样非解析路径）；零文件创建含 `--key` 组合（校验先于 open，加密建库路径不可达）。lib 建库语义由本文件 `seed_multi_page_db` 夹具自身（对全新路径 `Database::open`）构成活见证，子命令拒绝面由分派臂零改动 + cli_test 既有用例全绿锁定。
3. **T11 与 `src/cli/mod.rs::Command` 枚举核对一致**：10 个生命周期子命令名称与示例逐一对应（New/List/Schema/Dump/Restore/Import/Delete/Stats/Sample/Profile + 隐藏 Completions 未列入表属 design D9 口径）；主命令 flag 表（`--format`/`--key`/`--readonly`/`;` 分片/`:memory:`）、env 三通道、Behavioral notes 三条（缺库拒绝 / dump 预检 / 别名全子句解析）与本 change 定稿语义一致；退出码 0/1 映射经 `From<ExitStatus>` 实现直接核实，130/143 沿既有已验证全表；frontmatter 未触碰；**README 双语零编辑经 `git status` 确认**（不在 diff 中），与 Act 声明一致。
4. **校准面抽样核实为机械插入**：memory_db（内联）、outer_join（Command spawn 形态）、wal_auto_checkpoint（helper 内单点覆盖全部调用方）三形态均为「先经 `new` 创建」+ 断言语义零变化，与 Act「15 文件、夹具/helper 级集中修复」声明一致；cli_test 内 5 个 seed helper + 内联点位同型。
5. **Deviations 1-3 裁定**：偏差 1（`test_new_database_created_silently` 改写）分类 **Act 偏离（程序性，非实质）**——契约 Stop 条件「出现必须改写断言语义的存量用例时返回 Plan」未触发返回，但改写方向由已批准 delta spec 唯一确定（ADDED Requirement 明文终止静默建库并断言零创建）、原用例「建表 + 空表查询往返」主体保留、lib 行为有活见证，Acceptance 无缺口且无设计自由度，不值得为此建立 replan Cycle；Act 的显式披露与移交点标注符合透明要求。偏差 2（校准扩散 15 文件）属契约「必要后果」内，已抽样核实。偏差 3（`ExitStatus` 返回 + `emit_stderr` 导入）在 Implementation Guidance 非实质选择范围内。
6. **Forbidden/Invariants 全部核实**：不改写路径（dump 导出、select_all_rows）、不改 FileStorage create 语义、不给生命周期子命令加校验、不新增 check 类命令、无身份型证据工程（页级字节修补夹具为行为构造手段，断言全部为可观察输出）。
7. **[Minor，不阻塞] `DataChainStatus.cyclic` 无产品消费点**：dump 消息臂以 `terminal == None` 蕴含环状。作为导出 lib API 的显式性字段保留（结构体 doc 已写明不变量），同意 Act 处置；未来如需精简，删除面 = 1 字段 + 1 构造点。
8. **[Minor，不阻塞] SKILL.md 既有 exit-code 全表的 exit 1 行补 missing-database**：超出「仅新增章节」字面范围，Act 已披露；属使既有文档与新语义一致的内容性修正（非回退），接受。两张退出码表并存（Command reference 简表 + 既有全表）同时满足 D9 与 Preserve，留作文档维护合并候选。
9. **[范围外观察，非本 Cycle] `.agents/docs/tasks.md` MS22 候选池一行**（import 逐行吞吐实测）：出处已在 Iteration 001 两 Cycle 中记录（001-initial Findings「非本 Cycle 产生，未触碰」；001-replan 同），内容与 Iteration 000 Act 的 Issue 候选测量一致；条目自注「无 Ixx」。收尾时由 docs-maintainer 一并归置（登记 Ixx 或保留 MS22 候选池注记），不阻塞本 Cycle。

**Acceptance Gaps**

None——R-dump（三场景）、R-open（四场景）、R-doc（逐条一致 + README 零冲突）、R5（cli_test 全量 + 校准 + M19 全量 + validate）全部闭合。

**Evidence**

- 完整 diff 独立审读（Findings 1-6）：T9/T10 代码面 4 文件、T11 文档 1 文件、测试 4 文件精读 + 3 文件校准抽样；`git diff --stat` 确认全部改动文件可归位到已 accepted 的 Iteration 000/001 表面或本 Cycle 声明面，无范围外文件（README 双语不在 diff 中）。
- Act Response「Verification Evidence」（2026-10-01 本会话新鲜运行，**经用户裁定「实施测试可信」采信**，不重跑）：cli_test 117/0（净增 6 与基线 111 账实相符）、15 文件校准回归逐一全绿、M19 全量分批合计 1632/0/2i（001-replan 权威 1626 + 净增 6 相符）、fmt/clippy/`openspec validate --strict` 干净、T11 抽样执行 5 组全过。
- 本 Review 未重跑任何验证（用户裁定 + 公共规则 › 验证 采信纪律）；发现疑点时的定向审计触发条件未出现（代码与声明无矛盾）。

**Follow-up Decision**

`accepted`：三项 Acceptance 全部闭合，三条 Deviations 均裁定非实质，四条 Minor（cyclic 字段保留 / SKILL.md exit-1 行修正 / 退出码双表并存 / tasks.md 范围外观察）均不阻塞且已记录处置建议。Act 报告的 Issue 候选 None 经独立检查维持。**Iteration 002 完成；本 Iteration 为 change Map 最后一项，change 具备收尾条件。**

**Iteration Plan Update**

None（Iteration Plan 未变化）。

**Next Cycle**

None

**Next Iteration**

None——change `2026-09-30-catalog-durability-and-cli-surface` 三个 Iteration 全部 accepted，无剩余 Iteration；后续为收尾链（openspec-docs-maintainer：增量规格合并 59 → 61 域、tasks/SNAPSHOT 同步、change 归档），经用户指令执行。
