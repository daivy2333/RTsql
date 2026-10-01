# Iteration 001 / Cycle 001: replan——限定名收口剩余面 + 锁冲突有界重试

## Plan Context

- Status: ready
- Cycle Type: replan

**Cycle Scope**

- Change tasks: T6/T8 剩余验收面（Gap 1/2）、T12（2026-10-01 用户裁定扩入）
- Acceptance gaps: ① `sql-table-alias-resolution` R2 点名 SHALL 在 GROUP BY 与聚合参数两通道未满足（父 Cycle Review Gap 1）；② R2 验收「NLJ 变体」缺测试见证（Gap 2）；③ 假 `DatabaseLocked`（平台 flock 释放可见性窗口，Gap 3）
- Repair items: None（replan 使用更新后的全局 task，不建 repair item）
- Inherited scope: 父 Cycle 已交付的别名实现（T5/T6/T7/T8 主体，工作区未提交、Review 审定成立）继续有效；`sql-table-alias-resolution`/`order-by-alias-sorting`/`join-output-shape-resolution` 三 delta spec、design D4-D6、proposal 假设 5/6/7 全部继承；design D10 与 delta spec `database-file-lock`（MODIFIED R1）为本 Cycle 新增授权
- Excluded scope: Iteration 002（T9/T10/T11）；别名已声明时真实名遮蔽语义（PG 对齐，父 Cycle Review Minor 7，用户未决）；锁重试窗口可调 API/env；窗口函数/LATERAL/递归 CTE；attach 语义变更

**Objective**

闭合 R2 在 GROUP BY 与聚合参数两通道的点名 SHALL（未知限定符 `TableNotFound` 42P01 携限定符，单表与 JOIN 同语义），补齐 NLJ 变体见证，并以有界退避重试消除平台 flock 释放可见性窗口导致的假 `DatabaseLocked`（拒绝语义不变）。

**Background**

父 Cycle `000-initial` 交付的别名实现经 Plan Review 审定：偏差 1-5 全部成立且正确、主体与声明一致（Review Findings 1-2），但存在两个验收缺口（Gap 1/2）与一个范围外缺陷（Gap 3）。用户 2026-10-01 裁定「直接给出下一轮 replan cycle，把 Issue 纳入解决」——Gap 3 的范围扩大超出 rework 边界（rework 只能完成既有 Acceptance），故创建本 replan Cycle；父 Cycle Review 终态 `replan-required`，其 Plan Context 与 Act Response 冻结不改写。

**Investigation Facts**

- Current Baseline: master @ 1668c95 + 工作区未提交改动（Iteration 000 durability：`catalog.rs`/`slotted_page.rs`/`tests/catalog_durability_test.rs`，Review accepted；Iteration 001 别名实现：`planner/{mod,query,expression}.rs`/`tests/table_alias_resolution_test.rs` 13 用例全绿）。全量权威计数 **1619 passed / 0 failed**（Act 001 M19 分批，父 Cycle Review Finding 10 采信）。`cargo fmt --check` / `cargo clippy --all-targets` 干净、`openspec validate 2026-09-30-catalog-durability-and-cli-surface --strict` exit 0（同源采信）。
- Current-State Evidence（2026-10-01 Plan 实读/实测，行号以当前工作区为准）：
  - **Gap 1 两通道**：`resolve_group_by_item` 两段臂 `src/parser/planner/query.rs:212-240`——JOIN 基座走 `resolve_layout_index(layout, Some(&parts[0].value), …)`，限定符不可达 miss → `NonAggregatedColumn`；单表基座 `column_indices.contains_key(&parts[1].value.to_lowercase())` 末段静默命中。`aggregate_arg_indices` 两段臂 `query.rs:2053-2085`——JOIN 基座 `resolve_layout_index` miss → `ColumnNotFound(列名)`；单表基座 `column_indices.get(&parts[1]…)` 末段静默。同文件 `:2131-2138` 主 ORDER BY 路径已有同型**限定符先点名**模式（`qualifier_in_scope` 不可达 → `PlanError::TableNotFound(qualifier)`）——两通道修复照此镜像。
  - `qualifier_in_scope(&self, qualifier) -> bool`（`query.rs`，父 Cycle 新增）：别名归一后命中 `self.tables` 或限定符本身是注册键（真实表名/CTE·视图 effective 名/attach 复合键）。单表基座「别名已声明、限定符写真实名」→ in scope（放行，与单表投影面 `SELECT people.name FROM people p` 成功语义一致，父 Cycle Review Finding 7 实证）。
  - Plan 探针 RED 现状（工作区构建实测）：`SELECT COUNT(x.id) FROM people` → exit 0；`SELECT id FROM people GROUP BY x.id` → exit 0；JOIN 基座 `COUNT(x.id)` → `Column not found: id`、`GROUP BY x.id` → `Non-aggregated column 'id'`。别名可达形态（`GROUP BY a.team_id` 自连接、单表 `GROUP BY p.id`）现状已成功，修复不得回归。
  - **Gap 3 锁窗口**：`src/storage/file_storage.rs:81`（`open_with_key` 的 `file.try_lock()`）与 `:135`（`open_readonly_with_key` 的 `try_lock_shared()`）——`TryLockError::WouldBlock` 当前立即 `DatabaseLocked(path)`。`TryLockError::Error(e)` 分支保持直传。fd 在 `try_lock` 失败后仍有效（不消费），可同 fd 重试。`FileStorage.file: Arc<fs::File>`，锁随 fd 生命周期（MS10-T02/MS26-T1 语义）。根因取证与影响面见 design D10（纯 std 探针 2/20000、strace 全进程树、默认并行度 6 轮 2 败）。
  - **测试入口**：`tests/table_alias_resolution_test.rs`（13 用例，seed 夹具 `people`/`team` 复用）；`tests/database_file_lock_test.rs`（现有 5 用例，锁矩阵与生命周期）；`tests/readonly_session_test.rs`（14 用例，默认并行度统计见证载体）；`tests/alter_table_test.rs`（同上）。
- Code and Critical Path: T6 补全为计划期纯变换（planner 层，镜像既有 ORDER BY 点名模式）；T12 为存储 open 路径时序变化（错误语义与返回值不变，仅延迟 ≤ ~8ms）。两者无代码耦合，可任意顺序执行（建议先 T6/T8 后 T12，验证收口一次做）。

**Implementation Guidance**

1. T6 两通道点名在**解析前**插入：两段限定符先 `qualifier_in_scope`，不可达即 `PlanError::TableNotFound(qualifier.clone())`（携原文），单表与 JOIN 两基座同语义；in-scope 限定符 + 列缺失维持既有错误变体（`NonAggregatedColumn`/`ColumnNotFound`）。不做归一化改写（段键仍按书写比较——JOIN 布局段键本就是声明限定符形态，别名命中天然工作）。
2. T12 重试循环：`WouldBlock` → `std::thread::sleep(Duration::from_millis(2))` → 同 fd 再 `try_lock`，连同首次共 5 次尝试（总窗口 ~8ms 量级）；`TryLockError::Error(e)` 不重试直传。具体间隔/次数可在「总窗口 ~8ms 量级」内调整（非实质选择）；两入口（读写/只读）同型。
3. 测试先行：T6 的 4 条 RED 用例（探针已证现状静默/误报）+ T12 的重试耗尽用例（确定性 RED——现立即拒绝，用例断言的错误不变但循环用例（300 轮 close→立即重开）修复前为统计性偶发，不作为 RED 判据，作为回归见证）。
4. 统计见证（T12，记入 Act Response 不落 Evidence）：修复前 RED 已由父 Cycle Review Finding 9 记录（默认并行度 6 轮 2 败）；修复后 `cargo test --test readonly_session_test` 与 `cargo test --test alter_table_test` 以默认并行度各连跑 ≥ 8 轮全绿。

**Task Contracts**

### T6（补全）: GROUP BY 与聚合参数两通道限定符点名

- Requirement/Scenario: sql-table-alias-resolution R2（点名 SHALL + 覆盖清单的 GROUP BY 与聚合参数通道）；未知限定符点名场景的单表/JOIN 同语义扩展
- Depends on: None（别名注册 T5 已在父 Cycle 交付并审定）
- Targets: `src/parser/planner/query.rs::resolve_group_by_item` 两段臂（:212-240）、`src/parser/planner/query.rs::aggregate_arg_indices` 组装闭包两段分支（:2053-2085）
- Current behavior: 未知限定符（归一后不可达）在单表基座经末段静默解析成功；JOIN 基座报 `NonAggregatedColumn(列名)`/`ColumnNotFound(列名)`（误报列缺失，错误不携带限定符）
- Required behavior: 两段限定符先经 `qualifier_in_scope` 检查，不可达 → `PlanError::TableNotFound(qualifier)`（42P01，携限定符原文），单表与 JOIN 同语义；in-scope 限定符 + 列缺失维持既有错误面不变；别名可达形态（`GROUP BY a.x`/`COUNT(a.x)`，单表 + JOIN）与裸名/别名/位置/表达式文本四级解析序零变化
- Preserve: attach 复合键三段通道；`aggregate_arg_indices` 的 `None` 与非列参数臂；GROUP BY 输出列名/表头职责；MS13 解析序
- Forbidden: 不改相关子查询（correlated）限定符语义；不解禁表达式项；不动 ORDER BY 既有点名路径
- Test witness: `tests/table_alias_resolution_test.rs` 新增（RED 先行）——①`SELECT id FROM people GROUP BY x.id` 拒绝且消息含 `x`（RED：现状 exit 0）；②`SELECT COUNT(x.id) FROM people` 拒绝且含 `x`（RED：现状 exit 0）；③JOIN 基座 `… GROUP BY x.id` 拒绝且含 `x`（RED：现状 `Non-aggregated column 'id'`）；④JOIN 基座 `SELECT COUNT(x.id) FROM people p JOIN team t ON p.id = t.id` 拒绝且含 `x`（RED：现状 `Column not found: id`）；⑤别名可达回归：`SELECT p.id, COUNT(*) FROM people p GROUP BY p.id`（单表）与自连接 `SELECT a.team_id, COUNT(*) FROM people a JOIN people b ON a.id = b.id GROUP BY a.team_id` 成功（现状已绿，防回归护栏）
- GREEN condition: `cargo test --test table_alias_resolution_test` 全绿
- Verification: `cargo test --test table_alias_resolution_test 2>&1 | tail -4`，0 failed；失败含义 = 两通道仍有遗漏消费点或 in-scope 判定误伤既有形态
- Stop when: 点名需改动相关子查询语义、或四通道之外出现第五消费点、或 in-scope 判定与别名遮蔽产生新的语义分歧时返回 Plan

### T8（收口）: NLJ 变体见证与读面回归

- Requirement/Scenario: sql-table-alias-resolution R2（Cycle Acceptance 六缺陷清单的 NLJ 变体项）；R5 零回归
- Depends on: T6（补全）
- Targets: `tests/table_alias_resolution_test.rs`（新增 LEFT JOIN 用例）；既有读面测试族回归
- Current behavior: LEFT JOIN + 别名行为正确（父 Cycle Review Finding 4 探针实证：`[["ann","red"],["bob",null]]`、`WHERE t.id IS NULL` → `[["bob"]]`）但无用例见证
- Required behavior: 新增用例见证历史缺陷 NLJ 变体形态（`SELECT p.name, t.team_name FROM people p LEFT JOIN team t ON t.id = p.team_id` 含未匹配 NULL 补齐断言 + `WHERE t.id IS NULL` 反连接形态）；T8 测试族（order_by_alias_sort / join_output_resolution / aggregate_output_shape / derived_scan_alias / cte / no_from_select / subquery / outer_join / nested_loop_join）全绿
- Preserve: 白名单外零校准（父 Cycle Act 已证零校准，本 Cycle 新增用例不得改写既有断言）
- Forbidden: 不为本 Iteration 能力回改既有断言
- Test witness: 新用例（对现实现即 GREEN——见证性质，非 RED；缺陷的 RED 已由父 Cycle Investigation Facts 实测记录）+ 测试族回归输出
- GREEN condition: 新用例 + 测试族全绿；全量分批（M19）0 failed
- Verification: M19 分批合计行 + `openspec validate 2026-09-30-catalog-durability-and-cli-surface --strict`
- Stop when: 回归暴露真实语义回归（超出历史宽松形态）时返回 Plan

### T12: 锁冲突有界退避重试

- Requirement/Scenario: database-file-lock R1（MODIFIED，本 change delta spec）——窗口耗尽拒绝 + close→立即重开可达两场景
- Depends on: None
- Targets: `src/storage/file_storage.rs::open_with_key` 的 `WouldBlock` 分支（:81 附近）、`src/storage/file_storage.rs::open_readonly_with_key` 的 `WouldBlock` 分支（:135 附近）
- Current behavior: `WouldBlock` 立即 `DatabaseLocked(path)`——平台 flock 释放可见性窗口（design D10）下，同进程 close→立即重开偶发假 `DatabaseLocked`
- Required behavior: `WouldBlock` 后同 fd 有界重试（`std::thread::sleep(2ms)` × 4 次重试，连同首次 5 次尝试，总窗口 ~8ms 量级）；耗尽仍 `WouldBlock` → `DatabaseLocked(path)` 语义不变；`TryLockError::Error(e)` 不重试直传；两入口（读写/只读共享锁）同型
- Preserve: 锁的 advisory 语义、单写者模型、`DatabaseLocked` 错误面与 CLI exit 4；`readonly-open-mode` 的锁矩阵（活跃持有者照旧拒绝，仅时延 +窗口量级）；锁先于 WAL 打开与恢复的顺序；不加 env/config/API 通道
- Forbidden: 不动锁获取时机（仍先于 WAL/恢复）；不为重试新增判定层、日志或身份型证据工程；不改 WAL/checkpoint 文件面的任何锁行为（其本就无锁）
- Test witness: `tests/database_file_lock_test.rs` 新增——①重试耗尽拒绝：测试内以原生 `std::fs::File` + `try_lock` 稳定持有路径锁，`Database::open` 断言 `DatabaseLocked`（确定性；现实现即绿——本用例为语义保持见证）；②同型只读打开变体；③close→立即重开循环 ≥ 300 轮全成功（修复前统计性偶发 2/20000 量级，作回归见证）；④既有锁矩阵 5 用例零修改全绿。统计见证（RED/GREEN 记入 Act Response）：修复前默认并行度 `readonly_session_test` 6 轮 2 败（父 Cycle Review Finding 9）；修复后同套件 + `alter_table_test` 各 ≥ 8 轮全绿
- GREEN condition: `cargo test --test database_file_lock_test` 全绿 + 统计见证轮数达标
- Verification: `cargo test --test database_file_lock_test 2>&1 | tail -4` + 统计循环输出；失败含义 = 重试未吸收窗口或真冲突误放行
- Stop when: 重试导致既有锁矩阵用例出现反向偶发（该拒的放行）或 D10 语义之外的可见行为变化时返回 Plan

**Invariants**

- 计划期纯变换（T6）：不触存储/执行器/WAL；计划形状对真实表名与别名书写等价。
- T12 只改 open 路径 `WouldBlock` 分支时序：错误变体、错误文案、SQLSTATE/退出码、锁获取顺序全部不变。
- attach 两段表引用 attach-first、三段复合键通道、相关子查询限定符语义不变。
- 表头命名职责（末段）不变；不新增判定层或身份型证据工程。

**Non-goals**

见 Cycle Scope Excluded scope。另：锁重试窗口的可配置化、Windows/macOS 平台的窗口特性差异评估（现证仅 Linux/WSL2，重试为通用无害缓解）。

**Acceptance**

- R2（补全）：未知限定符在 GROUP BY 与聚合参数两通道、单表与 JOIN 两基座，以 `TableNotFound` 42P01 携限定符点名拒绝；别名可达形态零回归（T6）。
- R2（见证）：NLJ 变体（LEFT JOIN + 别名）有测试见证（T8）。
- R-lock（新）：活跃持有者跨窗口后仍被 `DatabaseLocked` 拒绝；同进程 close→立即重开 ≥ 300 轮零假拒绝；默认并行度套件统计见证达标（T12）。
- R5：T8 测试族 + 全量分批（M19）0 failed + `openspec validate --strict` 通过（T8/T12 收口）。

**Verification**

- R2：`cargo test --test table_alias_resolution_test 2>&1 | tail -4`（新增 4 条 RED→GREEN + 护栏绿）。
- R-lock：`cargo test --test database_file_lock_test 2>&1 | tail -4`；统计循环两条命令各 ≥ 8 轮的 per-round 结果行。
- R5：M19 全量分批合计行 0 failed + `openspec validate 2026-09-30-catalog-durability-and-cli-surface --strict`。
- 人工步骤：无。

**Gate 2 Readiness**

全部 PASS——计划与扩围获用户批准（2026-10-01「直接给出下一轮 replan cyc，把 Issue 纳入解决」）；三契约均有 file:line 锚点、Current/Required 行为、测试见证与停止条件；Gap 根因调查完整（别名缺口经代码读读 + 探针实证；锁窗口经纯 std 探针 + strace 全进程树 + 并行套件复现三角取证）；design D10 与 delta spec 已闭合语义；RTM 已更新无 Missing；非实质选择（重试间隔/次数微调、用例命名）已留 Act；Persisted Evidence = none；无 BLOCKED/WAIVED。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- T12 统计见证是概率性的（窗口 2/20000 量级）：修复后 ≥ 8 轮全绿不能证明窗口消除，只证明「不再以测试可见频率发生」；确定性判据是重试机制本身（代码审读 + 耗尽用例）。该限度记入 Act Response，不作过度声明。
- `std::thread::sleep` 在 tokio 上下文（`Database::open` 为 async）阻塞调用线程 ≤ ~8ms：open 路径现有实现本就含同步 open/头校验/KDF（MS10-T03/MS17-T01），无新增异步语义破坏；M19 全量套件总时延增量可忽略。
- T6 单表基座「别名已声明、GROUP BY 写真实名」（`FROM people p GROUP BY people.id`）修复后 in-scope 放行（与单表投影面一致）；JOIN 基座同形态维持既有拒绝（`NonAggregatedColumn`）——两基座在该遮蔽形态上的差异属父 Cycle Review Minor 7 记录的未决语义，不在本 Cycle 收口。

## Act Response

- Status: reported

**Implemented**

T6（补全，`src/parser/planner/query.rs`）：`resolve_group_by_item` CompoundIdentifier 两段臂与 `aggregate_arg_indices` 组装闭包两段分支均在解析前插入 `qualifier_in_scope` 检查——归一后不可达即 `PlanError::TableNotFound(限定符原文)`（42P01），单表与 JOIN 两基座同语义，镜像既有 ORDER BY 点名模式；in-scope 限定符 + 列缺失维持既有 `NonAggregatedColumn`/`ColumnNotFound` 错误面。`resolve_group_by_item` 布局臂内过时注释（「不新增错误面」）同步修正；不做归一化改写（段键按书写比较，契约 Preserve 保持）。`tests/table_alias_resolution_test.rs` 13 → 16：`unknown_qualifier_in_group_by_and_aggregate_args_is_named`（4 拒绝形态）+ `in_scope_alias_group_by_and_aggregate_args_reachable`（别名可达护栏 3 形态）。

T8（收口，`tests/table_alias_resolution_test.rs`）：新增 `left_join_alias_projection_and_null_padding`——`LEFT JOIN team t` 别名限定投影 + 未匹配行 NULL 补齐 + `WHERE t.id IS NULL` 反连接形态（对现实现即 GREEN，见证性质；缺陷 RED 已由父 Cycle Review Finding 4 记录）。T8 测试族 9 套件回归全绿。

T12（`src/storage/file_storage.rs`）：新增 `try_lock_with_backoff(file, path, lock)` 私有 helper——`WouldBlock` 同 fd 退避重试（`std::thread::sleep(2ms)` × 4 次重试，连同首次 5 次尝试，总窗口 ~8ms 量级），耗尽仍 `DatabaseLocked(path)`（错误面不变）；`TryLockError::Error(e)` 不重试直传 `StorageError::Io(e)`。`open_with_key`（独占 `try_lock`）与 `open_readonly_with_key`（共享 `try_lock_shared`）两入口接线，锁获取时机（先于 WAL/恢复）不变；`open_readonly` doc「不等待」表述同步更新。`tests/database_file_lock_test.rs` 5 → 8：重试耗尽拒绝（读写）、只读同型变体、close→立即重开 300 轮循环回归见证；既有 5 用例零修改。

**Deviations from Plan**

1. **[机制级，Plan 遗漏] 无聚合查询的 GROUP BY 不经 `resolve_group_by_item`。** 契约见证 ① `SELECT id FROM people GROUP BY x.id` 的 RED 形态实为「`has_aggregates == false` 时 GROUP BY 整体静默被忽略」（探针实证：单表/JOIN × 无聚合 GROUP BY 四形态全部 exit 0，含别名可达形态），Plan Investigation Facts 假设该语句经 `resolve_group_by_item` 两段臂路由，实际该函数仅在 `has_aggregates` 为真时调用（`query.rs` 聚合构建门）。契约内处理：新增 `validate_group_by_qualifiers` helper（镜像 `validate_projection_qualifiers` 模式），仅在无聚合 else 臂对两段限定名做同款作用域点名——Acceptance「`SELECT id FROM people GROUP BY x.id` 拒绝且含 x」达成，in-scope 形态与裸名/别名/位置/表达式文本四级解析序零变化，无聚合 GROUP BY 的既有静默求值语义本身不变（仅收口未知限定符静默）。计划期纯变换，不触存储/执行器/WAL。
2. **[测试形式] 见证 ③ 采用与契约 RED 记录一致的含聚合 JOIN 形态**（`SELECT p.id, COUNT(*) … GROUP BY x.id`，RED = `Non-aggregated column 'id'`），并追加无聚合 JOIN 形态覆盖（经 deviation 1 通道拒绝）；护栏组顺带覆盖单表聚合参数 `COUNT(p.id)` 可达。
3. **[测试形式] T8 见证用例不含 ORDER BY。** 实测 `ORDER BY p.id`（投影外键）在 JOIN 基座被既有排序键校验拒绝（`ColumnNotFound: id`，INNER 同型探针实证）——属语料 `order-by-alias-sorting` R3「未知键拒绝」既有语义（JOIN 基座排序键校验集 = 投影输出列），非本 Cycle 引入、非别名域回归；行序断言按基表扫描序（与本文件既有用例约定一致）。

**Self-Review Findings**

- [Important，已修复] T8 见证初版自加 `ORDER BY p.id` 触发 deviation 3 的既有拒绝——测试写法问题（对现实现即 GREEN 的见证用例不应叠加超出契约形态的断言），已按契约见证形态修正，实现代码无需改动。
- 遗留 Minor（接受，非缺陷）：单表基座「别名已声明、GROUP BY 写真实名」经 `qualifier_in_scope` 放行（Plan Risks and Notes 已预记，与单表投影面一致）；JOIN 基座同形态维持既有拒绝——父 Cycle Review Minor 7 未决语义，不在本 Cycle 收口。

**Verification Evidence**

- T6 RED→GREEN：`cargo test --test table_alias_resolution_test` 实施前 `unknown_qualifier_in_group_by_and_aggregate_args_is_named` FAILED（`Expected Error, got QueryResult { rows: [[1],[2],[3]] }`——用例 ① exit 0 形态，与 Plan 探针一致）；实施后 `16 passed; 0 failed`（exit 0）。
- T8 测试族（9 套件）：aggregate_output_shape 8 / cte 14 / derived_scan_alias 18 / join_output_resolution 14 / nested_loop_join 9 / no_from_select 9 / order_by_alias_sort 7 / outer_join 16 / subquery 32——合计 **127 passed / 0 failed**，白名单外零校准。
- T12：`cargo test --test database_file_lock_test` → `8 passed; 0 failed`（exit 0）；重试耗尽两用例对修复后实现确定拒绝，300 轮循环零假拒绝。
- T12 统计见证（默认并行度，各 10 轮 ≥ 要求的 8 轮）：`readonly_session_test` 10 轮全绿（14 passed × 10，1.14–1.27s/轮）；`alter_table_test` 10 轮全绿（16 passed × 10，0.08–0.10s/轮）。修复前 RED 已由父 Cycle Review Finding 9 记录（6 轮 2 败）；本见证只证明「不再以测试可见频率发生」，确定性判据是重试机制本身与耗尽用例（Plan Risks and Notes 限度记入，不作过度声明）。
- 全量分批（M19：102 集成目标 × `--test-threads=1`，四批顺序执行 + lib/bins）：

  ```
  批1  passed=389 failed=0 ignored=2     批3  passed=351 failed=0 ignored=0
  批2  passed=258 failed=0 ignored=0     批4  passed=275 failed=0 ignored=0
  lib passed=353 failed=0     bins passed=0
  合计 1626 passed / 0 failed / 2 ignored
  ```

  计数说明：父 Cycle Review Minor 6 已记录台账 ±1 歧义（声明新增 16 vs 批次差 15）；本 Response 以 **0 failed** 为判据（达成），总数按本轮实测 1626 记。
- 静态检查：`cargo fmt -- --check` → clean（exit 0）；`cargo clippy --all-targets` → 0 告警。
- `openspec validate 2026-09-30-catalog-durability-and-cli-surface --strict` → `Change … is valid`（exit 0）。
- delta spec 核对：`database-file-lock` R1 MODIFIED 五场景（持有跨窗口拒绝 / 同进程不豁免 / close→重开 300 轮 / `Error` 不掩盖）与 `sql-table-alias-resolution` R2「GROUP BY 与聚合参数通道、SHALL NOT 静默求值」措辞逐条比对实现一致。

**Persisted Evidence**

- None required（Plan 指定 mode = none；决定性输出均 ≤20 行且已在本 Response 摘录，全部可低成本复现；统计见证按 Plan 记入本 Response 不落 Evidence）。

**Experience Candidates**

- Runbook 候选：None（无新的可重复高风险操作路径）。
- Issue 候选：None（deviation 3 的 JOIN 基座投影外排序键面属语料既有语义非缺陷；本 Cycle 范围外未发现实质缺陷）。

**Unresolved**

None。

**工作区说明（非本 Cycle 产物）**

Iteration 000（`catalog.rs`/`slotted_page.rs`/`tests/catalog_durability_test.rs`）与父 Cycle 001-initial 产物保持未提交状态（Review accepted / replan-required 冻结）；`.agents/docs/tasks.md` 的 MS22 候选池实测条目为非本 Cycle 改动，未触碰。

## Plan Review

- Review Result: accepted

**Findings**

独立检查（Plan 独立阅读本 Cycle diff：`src/parser/planner/query.rs` 三处 T6 面——`resolve_group_by_item` 两段臂 :220-222、`aggregate_arg_indices` 两段分支 :2097-2124、`validate_group_by_qualifiers` :777-791 及其无聚合 else 臂调用点 :2163；`src/storage/file_storage.rs`——`try_lock_with_backoff` :67-92 与 `open_with_key`/`open_readonly_with_key` 两接线点；`tests/table_alias_resolution_test.rs` 16 用例、`tests/database_file_lock_test.rs` 8 用例；逐一核对 Act Response 声明与代码一致）：

1. **T6/T8/T12 全部与 Act 声明一致，契约逐项闭合**。三处限定符点名均为解析前 `qualifier_in_scope` 检查 → `TableNotFound(qualifier)`（42P01 携原文），单表与 JOIN 两基座同语义；in-scope 限定符 + 列缺失维持既有 `NonAggregatedColumn`/`ColumnNotFound` 错误面（代码核实）；归一化改写未做（段键按书写比较，契约 Preserve 保持）。T12 重试循环与 design D10 逐项吻合（首次 + 4 次重试、2ms 间隔、`Error(e)` 直传 `Io`、锁获取仍先于头校验/WAL/恢复、零新 API/env 通道）。
2. **偏差 1（机制级，Plan 遗漏）核实为真实且处理正确**：无聚合查询的 GROUP BY 确不经 `resolve_group_by_item`（聚合构建门在外，Plan 探针与 Act RED 一致）；Act 以 `validate_group_by_qualifiers` 镜像 `validate_projection_qualifiers` 模式在 else 臂补齐，仅收口两段限定名的未知限定符静默，无聚合 GROUP BY 既有静默求值语义不变——契约见证 ① 的 Acceptance 达成且无语义扩大。分类 Plan 遗漏，非 Act 偏离。
3. **偏差 2/3（测试形式）核实覆盖等价**：见证 ③ 含聚合 JOIN 形态与契约 RED 记录一致，追加的无聚合 JOIN 形态经 deviation 1 通道拒绝（:370-374）；T8 见证不含 ORDER BY 的理由成立（JOIN 基座投影外排序键拒绝属语料 `order-by-alias-sorting` R3 既有语义，非本 Cycle 回归）。
4. **Forbidden/Invariants 全部核实**：相关子查询限定符臂未触碰（`qualifier_in_scope` 与父 Cycle 已审定的投影面同一 helper 同一模式）；ORDER BY 主路径点名块（:2178-2182）为父 Cycle 原样；锁矩阵既有 5 用例零修改；测试无身份型证据工程（纯行为断言 + 固定轮数循环）。
5. **验收面全部闭合**：R2 补全（见证 ①-⑤ 齐备，RED 由 Act 记录、GREEN 本 Review 定向复跑确认）、R2 NLJ 见证（`left_join_alias_projection_and_null_padding`，对现实现 GREEN 的见证性质与父 Cycle Finding 4 探针一致）、R-lock（重试耗尽两用例确定性 + 300 轮循环 + 统计见证限度如实记录未过度声明）、R5（测试族 + 全量 + validate）。
6. **[Minor，不阻塞] `validate_group_by_qualifiers` 仅处理 `GroupByExpr::Expressions`**：`All`（ROLLUP 等方言形态）不在校验面——该形态在无聚合路径本就被整体静默忽略，既有语义不变，且不在 delta spec 覆盖清单内；记录备查。
7. **[Minor，不阻塞] 无聚合路径三段限定 GROUP BY 仍随整体静默忽略**：与裸名未知列同型（既有静默语义），R4 三段拒绝面在投影/表达式通道已收口，GROUP BY 通道不扩围——父 Cycle Minor 7 同族的未决语义面，不在本 change 收口。

**Acceptance Gaps**

None——replan Cycle 三项 Acceptance（R2 补全 / R2 NLJ 见证 / R-lock）与 R5 全部满足。

**Evidence**

- 完整 diff 独立审读（Findings 1-4）；`git diff --stat` 确认产品代码改动面 = planner `query.rs` + `file_storage.rs` + 两测试文件（planner `mod.rs`/`expression.rs` 为父 Cycle 已审定产物，未再变化）。
- Act Response「Verification Evidence」（本会话新鲜运行，采信：全量 M19 分批 1626/0、T8 测试族 127/0、统计见证 10+10 轮、fmt/clippy/validate）。
- Plan 定向补跑（replan 进入触发的受影响范围审计，2026-10-01）：`cargo test --test table_alias_resolution_test` → `16 passed; 0 failed`；`cargo test --test database_file_lock_test` → `8 passed; 0 failed`；`cargo fmt -- --check` → clean；`openspec validate 2026-09-30-catalog-durability-and-cli-surface --strict` → `is valid`（均 exit 0）。

**Follow-up Decision**

`accepted`：三项 Acceptance 全部闭合、无阻塞 finding、两条 Minor 均为不扩围的既有语义记录。父 Cycle 的三个 Gap（GROUP BY/聚合参数两通道点名、NLJ 见证、假 `DatabaseLocked`）在本 Cycle 全部解决；Act 报告的 Issue 候选 None 与父 Cycle Review 的处置（随 T12 在 change 内解决不落账）一致。Iteration 001 完成。

**Iteration Plan Update**

None（Iteration Plan 未变化）。

**Next Cycle**

None

**Next Iteration**

`iterations/002-cli-surface/000-initial.md`（已展开：T9/T10/T11）
