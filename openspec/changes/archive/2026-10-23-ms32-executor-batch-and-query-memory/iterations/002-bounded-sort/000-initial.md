# Iteration 002 / Cycle 000: 有界排序

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: 3.1、3.2
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 的有界排序段落；design D3；`query-memory-budget`「LIMIT 直接约束排序时只驻留有界行」的结果正确性部分（内存上限本身在 Iteration 003）。
- Excluded scope: 相等键决胜、DISTINCT 下推、外部排序溢写、查询内存计费、批量生产路径、data_scan.rs:687 格式清理（Iteration 001 Review 记录的 Minor，另行轻量处理）。

**Objective**

`LIMIT` 直接包住 `Sort`、或中间只隔一层 `Projection` 时，排序只驻留 `offset + limit` 行，输出与全量排序再截取一致；`EXPLAIN` 文本不变；`DISTINCT`、聚合、集合运算等其他形状仍全量排序。

**Investigation Facts**

- Starting point: Iteration 001 最终 Act Response（2026-10-23，001-replan Cycle）+ accepted Review：join.rs 两阶段重构交付、join_test 净 +1 条、全量 M19 分批 2126 passed / 0 failed / 2 ignored、两目标文件 fmt/clippy 干净。本轮沿该现场，未重跑。
- Current-State Evidence:
  - `SortExecutor::initialize`（`src/executor/sort.rs:48`）先收齐全部输入行再 `sort_unstable_by`，`sorted_rows` 无上界。表达式键（输出别名键）先逐行 decorate 再排（sort.rs:69–106）；产出时按 `projection` 裁剪（sort.rs:109–112）。比较器 `compare_rows`/`compare_values` 含 NULL 在末尾与表达式键。
  - `SortNode`（`src/executor/plan.rs:534`）字段 input/order_by/table_name/columns/projection；`LimitNode`（plan.rs:547）input/limit/offset。
  - Limit 包装点两处（design D3 的「两处」）：普通 SELECT `src/parser/planner/query.rs` 约 3032–3068——input 为 `plan_with_distinct`，其下 ORDER BY 形状为 `Sort` 或 `Projection(Sort)`（deferred_wrap，表达式项，query.rs:2989–2996），DISTINCT 时为 `Distinct(Sort)` 或 `Distinct(Projection(Sort))`（query.rs:3008–3029）；集合运算尾 `query.rs` 约 1158–1171——input 为 identity-projection 的 `Sort`（query.rs:1144）。既有 `scan_cap` 下推只作用于直接包住的裸 `DataScan`（query.rs:3049–3059），与 Sort 形状无重叠。SubqueryEval 包在 Limit 之上（query.rs:3077 起），不影响判定。
  - `LimitExecutor::next`（`src/executor/limit.rs:30`）：`limit == 0` 不拉输入；先 skip offset 再取 limit。
  - EXPLAIN：`plan_format.rs` 渲染 `Sort (order_by=[...], columns=...)`（:279）与 `Limit (limit=..., offset=...)`（:285），不读 SortNode 新增字段即可保持文本不变；`plan_format.rs` 内嵌测试构造 `SortNode`（约 :429）需补 `bounded: None` 才编译。
  - pipeline 构造：`src/pipeline.rs:2308` Sort 臂 `SortExecutor::new(...).with_projection(...)`——bounded 传参点。
  - 测试现场：`tests/sort_test.rs`、`tests/limit_test.rs` 为 MockExecutor 单元风格；`tests/order_by_alias_sort_test.rs` 为 SQL 语义测试。Iteration 001 在 join_test.rs 加 `Database::execute_sql` 测试是单元风格文件承载 SQL 级测试的先例。
- Code and Critical Path: `Database::execute_sql` → `execute_stage` → `create_executor_from_plan`（Sort/Limit 臂）→ `SortExecutor`/`LimitExecutor`。修改面：`query.rs` 两处 Limit 包装、`plan.rs::SortNode`、`sort.rs::SortExecutor`、`pipeline.rs` Sort 臂、`plan_format.rs` 测试构造、`tests/sort_test.rs`。

**Implementation Guidance**

按 design D3：`SortNode` 增加 `bounded: Option<usize>`（`None` = 全量排序）。只在两处包装 Limit 时赋值，判定输入形状：`Sort`，或恰好一层 `Projection` 且其输入是 `Sort`（后者把值写进内层 SortNode）。值为 `offset.saturating_add(limit)`。`limit == 0` 时 LimitExecutor 不拉输入，不依赖该字段。`plan_format` 不打印 `bounded`。

有界保留用现有 `compare_rows`/`compare_values`（含 NULL 在末尾与表达式键 decorate）：驻留至多 K 行的「当前最优集」，输入耗尽后按同一比较排序输出；容器实现（堆或逐行淘汰）为非实质选择。`bounded: None` 路径不重排既有 collect-then-sort 结构。相等键不新增决胜规则。

**Task Definitions**

### 3.1: 有界排序只驻留 offset + limit 行

- Requirement/Scenario: query-memory-budget「LIMIT 直接约束排序时只驻留有界行」的结果部分；`executor-batch-interface` 不涉及。小上限下的内存见证留到 Iteration 003。
- Depends on: Iteration 001
- Targets: `src/executor/plan.rs::SortNode`；`src/executor/sort.rs::SortExecutor`；`src/parser/planner/query.rs`（两处 Limit 包装）；`src/pipeline.rs` Sort 臂；`src/executor/plan_format.rs`（测试构造补字段）；`tests/sort_test.rs`
- Current behavior: 任何 `ORDER BY ... LIMIT` 都全量收集输入行再排序，`sorted_rows` 无上界。
- Required behavior: `Limit(Sort)` 与 `Limit(Projection(Sort))`（含集合运算尾的 `Limit(Sort(SetOp))`）时，Sort 驻留行数 ≤ `offset + limit`，输出行序列与全量排序再由 Limit 截取的结果一致。`Limit(Distinct(...))` 及 Sort 不被 Limit 直接约束的形状行为不变。`EXPLAIN` 输出逐字节不变。
- Preserve: `LimitExecutor` 语义、`scan_cap` 下推、`bounded: None` 的既有路径、DISTINCT/聚合输出、表达式排序键与输出别名键排序、NULL 在末尾。
- Forbidden: 相等键决胜规则；DISTINCT 下推或把 Distinct 形态纳入 bounded；外部排序/溢写；修改 `plan_format` 渲染逻辑（只允许测试构造补字段）。
- First test: 在 `tests/sort_test.rs` 增两条 `Database::execute_sql` 测试并先于产品代码运行，预期改前即 GREEN（本任务按 TDD 重构分支执行：改前 GREEN、改后保持 GREEN；有界驻留由代码结构核对，同 Iteration 001 删 `left_rows` 先例）：`order_by_unique_key_limit_offset_matches_full_sort_prefix`——唯一主键乱序插入 N 行（N > k + m），`SELECT id FROM t ORDER BY id LIMIT k OFFSET m` 输出 = 升序前缀切片；`select_distinct_order_by_limit_returns_distinct_prefix`——值集 `[1,1,2,2,3,3,4]`，`SELECT DISTINCT v FROM t ORDER BY v LIMIT 2` 输出 `[1,2]`（重复值横跨截断界，锁 Distinct 形态不启用有界——若误对 Distinct 之下的 Sort 设界，输出会缺值）。命令：`cargo test --test sort_test -- --test-threads=1`。
- GREEN condition: 上述两测试与 `sort_test`、`order_by_alias_sort_test`、`limit_test` 全部通过；`SortNode` 有 `bounded` 字段且仅两处 Limit 包装点赋值；`sort.rs` bounded 路径驻留行数有 K 上界（结构核对）；`cargo test --lib plan_format` 通过（EXPLAIN 文本不变）。
- Verification: `cargo test --test sort_test --test order_by_alias_sort_test --test limit_test -- --test-threads=1` 退出码 0，加 `cargo test --lib plan_format -- --test-threads=1`。
- Stop when: 既有测试因相等键次序变化失败（回到计划，不在实现里加决胜）；或 deferred_wrap 的 `Projection(Sort)` 实际形状与本判定不符。

### 3.2: 全量分批回归

- Requirement/Scenario: 有界排序的结果不变回归面
- Depends on: 3.1
- Targets: 无产品代码
- Current behavior: Iteration 001 结束时 2126 passed / 0 failed / 2 ignored。本轮净 +2 条测试。
- Required behavior: 按项目模型 M19：`--lib --bins` 一批；`tests/*.rs` 按文件名字典序每 10 个一批，每批 `--test-threads=1`，0 failed，不漏批。
- Preserve: 不新增测试框架外的判定脚本。
- Forbidden: 一次无过滤的 `cargo test`。
- First test: 无。消费 3.1 已通过的测试。
- GREEN condition: 每一批 failed 为 0；总数与 2126 + 2 对账。
- Verification: 各批 passed/failed 计数写入 Act Response。
- Stop when: 某一批失败。范围内先修；范围外回到计划。

**Invariants**

- `bounded: None` 的排序输出与 Iteration 001 结束时逐字节一致。
- DISTINCT、聚合、集合运算不被 Limit 直接约束的 Sort 不设 `bounded`。
- `EXPLAIN` 文本逐字节不变。
- 相等键次序不新增决胜规则（既有 unstable 语义）。

**Non-goals**

相等键决胜、DISTINCT 下推、外部排序溢写、查询内存计费（Iteration 003）、`next_batch` 批量、data_scan.rs:687 格式清理。

**Acceptance**

- `sort_test`、`order_by_alias_sort_test`、`limit_test` 通过，含两条新测试（唯一键前缀等价 + DISTINCT 不受界）。
- `Limit(Sort)` 与 `Limit(Projection(Sort))` 的 Sort 驻留 ≤ `offset + limit` 行（结构核对）。
- `EXPLAIN` 文本不变（`plan_format` 单测 + 既有 explain 集成测试）。
- 3.2 各批 0 failed，总数对账 2126 + 2。

**Verification**

- 3.1 两条新测试与三文件定向命令为直接判定；有界驻留由 `sort.rs` 结构核对，不写内存采样。
- EXPLAIN 不变由 `cargo test --lib plan_format` 与全量批内既有 explain 测试承载。
- 3.2 沿用 Iteration 000/001 的 M19 分批方式。

**Gate 2 Readiness**

PASS。任务 3.1、3.2 属 2026-10-23 用户原话「批准」的四轮计划，机制为 design D3 既定内容，本轮无新增范围。无 BLOCKED，无 WAIVED。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

两条新测试在改前即 GREEN——它们锁定「输出等价」与「Distinct 形态不越界」，有界驻留本身由代码结构核对（与 Iteration 001 删 `left_rows` 同款）。相等键在截断界上的保留集差异属既有 unstable 语义；若既有测试因此失败，按 Stop when 回到计划。

## Act Response

- Status: reported

### 实际改动

**Task 3.1 有界排序只驻留 offset + limit 行**

- `src/executor/plan.rs`：`SortNode` 新增 `bounded: Option<usize>`（`None` = 全量排序；文档注明仅两处 Limit 包装点赋值、渲染不输出）。
- `src/executor/sort.rs`：`SortExecutor` 新增 `bounded` 字段与 `with_bounded` builder；`initialize` 分派 `None`（既有 collect-then-sort 结构保留）与 `Some(k)`——有界驻留：`(keys, row)` 保留集至多 k 行，新行严格小于当前最差行才替换（相等键不新增决胜，保留先到行），输入耗尽后同一比较器排序 + 同一投影裁剪；新增 `decorate_row_keys` 单行表达式键求值 helper（求值顺序与错误消息同全量路径）。
- `src/parser/planner/query.rs`：新增自由函数 `mark_sort_bounded`（`Sort` 直设、`Projection(Sort)` 写内层 SortNode，其余形状不动）；两处 Limit 包装点〔集合运算尾 `plan_with_order`、普通 SELECT `plan_with_distinct` 之上〕以 `offset.saturating_add(limit)` 调用；两处既有 `SortNode` 构造补 `bounded: None`；集合运算尾绑定改 `let mut`。
- `src/pipeline.rs`：Sort 臂接线 `.with_bounded(node.bounded)`。
- `src/executor/plan_format.rs`：仅内嵌测试构造补 `bounded: None`；渲染逻辑零改动（EXPLAIN 文本不变）。
- `tests/sort_test.rs`：新增 `query_rows` helper 与两条 `Database::execute_sql` 测试——`order_by_unique_key_limit_offset_matches_full_sort_prefix`（唯一键前缀等价）、`select_distinct_order_by_limit_returns_distinct_prefix`（Distinct 形态不越界）。

**Task 3.2 全量分批回归**：无产品代码改动。

### 与计划的偏差

1. 先行测试夹具调整：DISTINCT 测试初版 `CREATE TABLE t (v INT)` 触发首列隐式主键（`Duplicate key`），改前首跑 7 passed / 1 failed；夹具改为显式 `id INT PRIMARY KEY, v INT` 七行 (id, v) 后改前 GREEN。测试策略不变（仍锁定输出等价与 Distinct 不越界），属测试编写修正。
2. 等价局部实现：sort.rs `None` 路径 decorate 内联循环改为调用共享 `decorate_row_keys`（与有界路径单源，求值顺序与错误消息逐字节相同）；collect-then-sort 结构未重排，计划禁止项无违反。

### 自检发现

- 无本次改动引入的 clippy 警告：触碰文件仅 pipeline.rs:2664 一条既有警告被点名（doc 注释空行，位于本次唯一 diff 块 2317 之外）；`ddl_dml.rs:1482`、`sql_differential_test.rs:244` 等其余警告均在未触碰文件。
- 遗留 Minor：无新增。Iteration 001 Review 记录的 `data_scan.rs:687` 格式清理按本轮排除范围未处理。

### 验证命令与实际结果

- 改前先行测试：`cargo test --test sort_test -- --test-threads=1`——首跑 7 passed / 1 failed（夹具隐式主键 `Duplicate key`，见偏差 1），夹具修正后 8 passed / 0 failed（两新测试改前 GREEN 成立）。
- 改后定向：`cargo test --test sort_test --test order_by_alias_sort_test --test limit_test -- --test-threads=1`——8 / 7 / 5 passed，0 failed；`cargo test --lib plan_format -- --test-threads=1`——2 passed / 0 failed（EXPLAIN 文本不变）。
- fmt/clippy：`rustfmt --edition 2021 --check` 六个触碰文件全净；`cargo clippy --lib` / `--tests` 无新增警告。
- Task 3.2 全量回归（M19）：批 00 `--lib --bins` + 批 01–13（128 个测试文件按文件名字典序每 10 个一批，末批 8 个），共 14 批，每批 `--test-threads=1`，全部 exit 0、0 failed；聚合 **2128 passed / 0 failed / 2 ignored** = Iteration 001 终态 2126 + 本轮净增 2，对账吻合。完整日志 `/tmp/ms32-iter002-full-regression.log`（会话临时文件，不落仓库）。
- 有界驻留结构核对（Acceptance 规定不以内存采样判定）：`sort.rs` `Some(k)` 臂 `retained` 仅在 `len < k` 时 push、替换不增长度——驻留恒 ≤ K；淘汰比较与终排同用 `compare_rows`（键装饰对齐规则同一）。

### Persisted Evidence

None required（Mode: none；无满足白名单的保存情形）。

### Experience Candidates

None。

### 未解决问题

None。

## Plan Review

- Review Result: accepted

**Findings**

独立阅读六个触碰文件的 diff（plan.rs、plan_format.rs、pipeline.rs、query.rs、sort.rs、sort_test.rs），与 Act 声明一致，`git status` 核对工作区仅含声明范围。

有界路径正确性独立验证：驻留不变式成立——`retained` 只在 `len < k` 时 push、替换按 `retained[worst_idx] = ...` 原位进行（sort.rs 有界臂），任意时刻驻留 ≤ K；保留集维持「已处理行的 K 最小集」（新行严格小于当前最差才替换，相等保留先到行——截断界相等键差异属既有 unstable 语义，与全量排序再截取在同一比较器下等价）。比较器参数对齐核对无误：`max_by` 取最后最大元 = 当前最差；`compare_rows(&row, worst_row, &row_keys, worst_keys)` 参数序正确；`decorate_row_keys` 只为表达式键压栈，与 `compare_rows` 的 `key_idx` 推进规则对齐。`k == 0` 防御守卫与设计注记一致。有界路径表达式键求值错误发生在收集中途而非收齐后，错误文案与传播路径同源，公共入口不返回部分行。

`None` 路径逐行对照：原 collect-then-sort 结构未重排，唯一差异是 decorate 内联循环改为共享 `decorate_row_keys`（求值顺序与错误消息逐字节相同）——Act 偏差 2 属实且等价。

规划面核对：`mark_sort_bounded` 形状判定与 D3 精确一致（`Sort` 直设、`Projection(Sort)` 写内层、`Distinct(...)` 与其他形状不动）；两处 Limit 包装点（集合运算尾 query.rs:1186、普通 SELECT query.rs:3087）以 `offset.saturating_add(limit)` 调用；scan_cap 臂重绑定 `DataScan` 后才 mark，形状无重叠。`grep -rn bounded src/` 确认 `Some` 赋值单源（仅 `mark_sort_bounded` 内两处），其余全为 `None` 默认或传递。`limit == 0` 时 bound = offset 但 LimitExecutor 不拉输入，字段不被消费，与 D3 注记一致。

EXPLAIN：plan_format.rs 渲染逻辑零改动（diff 仅测试构造补 `bounded: None`），文本不变由 diff 与 Act 的 `--lib plan_format` 2/0 双面证实。

测试核对：两条新测试与 Task Contract 名称和语义逐字一致；DISTINCT 测试用显式 `id` 主键夹具使 `v` 可重复，重复值横跨截断界——若误对 Distinct 之下 Sort 设界输出会缺值，能真实捕获越界；唯一键测试乱序插入 N=10、k+m=5 < N，判定确定。偏差 1（夹具首版触发首列隐式主键，修正后改前 GREEN）属测试编写修正，已如实记录，策略不变。

沿用 Act 验证结论（覆盖未失效，工作区与 Act 声明一致）：改前先行测试 8/0（夹具修正后）；改后定向 8/7/5 全 0 failed；`--lib plan_format` 2/0；fmt 六触碰文件净；clippy 无新增警告（pipeline.rs:2664 为既有警告，位于本轮 diff 块外）；全量 M19 14 批 2128 passed / 0 failed / 2 ignored = 2126 + 2 对账吻合。

Minor（不阻塞，无需修复）：有界路径对每行调用 `decorate_row_keys`，无表达式键时也分配空 `Vec`——与全量路径的 `Option` 短路相比多一次空分配，行为相等，属微开销，不作修复要求。

`data_scan.rs:687` 格式遗留按本轮排除范围未处理，仍待用户授权轻量修复（Iteration 001 Review 已记录）。

**Acceptance Gaps**

无。四项 Acceptance 全部满足：三文件加两新测试通过（沿用 Act，8/7/5）；`Limit(Sort)` 与 `Limit(Projection(Sort))` 驻留 ≤ K（结构独立核对）；EXPLAIN 文本不变（diff + plan_format 单测）；3.2 各批 0 failed 且 2128 = 2126 + 2 对账。

**Evidence**

- `git diff` 六文件全量阅读；`grep -rn "bounded" src/`（Some 赋值单源）；`grep -n "mark_sort_bounded" src/parser/planner/query.rs`（恰两处调用）
- `src/executor/sort.rs` 有界臂（驻留不变式）、`decorate_row_keys`、None 路径对照
- `src/parser/planner/query.rs:57–66`（形状判定）、`:1181–1188`、`:3081–3089`（两包装点）
- Act Response 的定向、fmt/clippy 与全量验证结论（沿用，来源已注明）

**Follow-up Decision**

accepted。两项偏差均非实质且已记录；有界驻留、形状判定与输出等价均有独立证据；无阻塞项。无当前 Cycle 修复要求。

**Iteration Plan Update**

无。四轮 Map 不变。

**Next Cycle**

None

**Next Iteration**

`iterations/003-query-memory-budget/000-initial.md`（按既有 Map 展开，Status: ready，Gate 2 沿 2026-10-23 用户批准的四轮计划与 design D4/D5）
