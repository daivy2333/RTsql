# MS17-T02 缺陷清账：ISS01(+MAX 毒化) + ISS02 + ISS03 + I041 + I048

## Why

tasks.md MS17-T02（执行序第一棒，2026-09-23 用户裁定三 change 拆分后）定义初版分发前的预存缺陷清账，五项均有台账实证：

1. **ISS01**（`.claude/issues/ISS01-min-create-tx-id-zero-poisoning.md`）——`BufferPool::clear_all_visible` 的 `or_default()` 以 `min_create_tx_id=0` 毒化首建条目，INSERT 路径 `min(0, W)` 永久钉零，页级 all-invisible 快路径（`find_visible_version` / DataScan）对该页永久失效（保守失效：只损性能，语义正确）。
2. **ISS02**（`ISS02-in-subquery-join-plan-rejection.md`）——`get_subquery_first_column` 无 Join/NestedLoopJoin 形态臂，单列 `IN (SELECT … JOIN …)` 子查询被误报 `Subquery returns multiple columns`（错误文案与事实不符；本会话新鲜探针复现，exit 3）。
3. **ISS03**（`ISS03-scalar-subquery-header-shape-mismatch.md`）——`get_plan_output_columns` SubqueryEval 臂未计入执行器在 `result_column_index` 插入的标量列：CLI 表头 N 列对 N+1 值行，标量列名（alias）丢失，json `columns` 与 `rows` 宽度不一致。
4. **I041**（improvements）——`src/cli/resolve.rs` 两个 env 测试并发改写进程全局 `HOME`/`RTSQL_HOME`，全量 `cargo test` 约 1/6 假失败（偶发），污染 MS17 后续 change 的全量零回归验收门。
5. **I048**（improvements）——`import` 以 CLI 实参原文插值构造 `INSERT INTO {table}`（`src/cli/lifecycle.rs:496`），含引号字符的表名（历史带引号 restore 产物；现行通路可用转义 delimited ident 建出，如 `"a""b"` → catalog `a"b`）经 import 不可达（SQL 解析面报错）。

**Plan 调查新发现（2026-09-23，结构链读码实证）**：`BufferPool::set_all_visible` 对无条目页 `or_insert { all_visible: true, min_create_tx_id: u64::MAX }`（`buffer_pool.rs:382-385`）；该页随后任一写路径 `clear_all_visible` 仅 `and_modify(all_visible=false)`、min 保持 MAX——形成 `{min=MAX, all_visible=false}` 条目，`all_invisible_for(hw) = MAX > hw` 恒真，`find_visible_version` 直接 `Ok(None)`、DataScan 整页跳过：**该页全部行对所有后续快照不可见（静默漏行），方向与 ISS01 相反（快路径过 active）**，直至该页下一次 INSERT 经 `min(MAX, W)` 自愈。暴露面与 ISS01 相同：仅快照携带执行（RC 模式；RR 下 `statement_snapshot` 返回 `None`，M21 机制整体旁路——本会话 CLI RR 探针不复现与该界定一致）。与 ISS01 同函数族同故障域，修复（哨兵语义）一并闭合，范围扩展经本 proposal Gate 1 批准。

清账理由：初版分发不带已知静默错误结果与验收门噪声；I041 先行消除全量假失败源（MS17 验证边界原文要求「I041 清账后全量门稳定」）。

**Iteration 001 实施期发现（2026-09-23，夹具探针 + Plan Review 独立读码证实）**：RC 重启可见性回归——`close()`（checkpoint 截断 WAL）→ reopen `redo_count == 0` → 恢复观测不到任何事务 id → `advance_past(0)` no-op → 分配器归零；而 RC `statement_view` 以分配器当前值为高水位（`database.rs:77-81` 注释自证该前提即 RC 高水位健全性基础「every id ≤ the allocator's current value is committed, aborted, or active」）→ 重启前已提交行（create_tx > 高水位）对 RC 不可见，随新事务分配 id 渐进越过旧水位才逐步「复现」（间歇性丢行，难排查）。RR 无快照路径不受影响；暴露面为 RC lib API。机理链与 T5 哨兵修复正交（属事务/恢复域，T5 契约 Forbidden 禁止进入）。经用户裁定作为 Iteration 002 并入本 change 修复（用户决策 5）。

## 用户决策（2026-09-23 Gate 1 前裁定）

1. **MS17 拆分与顺序**：三个 change——缺陷清账（本 change）→ 最小加密（MS17-T01）→ 安装面+README（MS17-T03/T04）。理由：I041 假失败源先消除，加密 change 的全量零回归门才稳定；README 最后核对加密用法定稿。
2. **WAL/checkpoint 明文缺口（MS17-T01 范围预裁定）**：本轮仅加密主库文件；WAL 帧与 checkpoint 位点保持明文，记为 README 已知限制 + 登记 improvement（干净关闭会 checkpoint 后截断 WAL，明文暴露窗口仅限崩溃残留）。
3. **ISS02 处置方向**：最小诚实化——IN×JOIN 维持计划期拒绝，错误文案改为与事实相符（点名 JOIN 不支持）；不修 `get_subquery_first_column` 形态臂取列与 ON 关联参数注册（能力解锁登记 improvement）。
4. **规划粒度**：在 change 内规划多个 Iteration（本 change 全部 Iteration 进入 tasks.md 的 Iteration Plan，只展开 Iteration 000 目录与首个 Cycle）。
5. **范围扩展——Iteration 002（2026-09-23 Iteration 001 Review 后裁定）**：用户指令「我们额外添加一个 iter 用来解决这个问题，计划到当前 change」——RC 重启可见性回归（Iteration 001 实施期发现，见 Why 节）作为 Iteration 002（T7/T8）并入本 change 修复；Iteration 001 Plan Review 记录的 `Next Iteration: None` 由本裁定取代；水位持久化落点（checkpoint 位点文件 16B→24B）见 design D8。

## What Changes

1. **I041 测试基建（T1）**——`src/cli/resolve.rs` 将 `test_bare_name_env_cases` 与 `test_db_dir_env_cases` 合并为单个 `#[test]` 顺序执行（文件头 doc 注释本就宣称「涉及 env 的用例集中在单个 #[test] 内顺序执行」，修正实现与注释一致）。结构性消除并行竞态；无产品代码变化。
2. **ISS02 IN×JOIN 诚实拒绝（T2）**——`PlanError` 新增 `InSubqueryJoinUnsupported`（文案 `IN subquery with JOIN is not supported`，沿用既有英文错误风格）；`get_subquery_first_column`（`src/parser/planner/subquery.rs`）补 `Join`/`NestedLoopJoin` 显式拒绝臂；`_` fallback 臂保持不变（多列误报文案对真多列形态语义相符）。多列+JOIN 形态同得 JOIN 文案（主因优先）。关联 ON 注册与 Join 形态取列能力解锁登记 improvement 候选，不在本 change。
3. **ISS03 标量子查询表头形状（T3）**——`get_plan_output_columns` SubqueryEval 臂（`src/parser/planner/query.rs:114`）按 `node.output_column` + `node.result_column_index` 在输入列名向量对应位置插入标量列名（`insert(min(idx, len))`，与执行器 `row.insert/push` 语义镜像；`SubqueryEvalNode` 两字段已 `pub`）。不改执行器行产出；CLI json `columns` 数与 `rows` 每行值数一致，table/csv/tsv 表头不再错位。
4. **I048 import 表名转义可达（T4）**——`import_csv` 的 INSERT 构造（`src/cli/lifecycle.rs:496`）表名经既有 `quote_ident` 包裹（与 dump/`select_all_rows`/分析命令同源）。实参比对（`get_table` 逐字）与 SQL 解析（转义引号往返 → 归一化不去引号）双面核实可达；裸名行为不变。
5. **ISS01 + MAX 毒化一并修复（T5）**——`src/storage/page_visibility.rs` 引入 `MIN_CREATE_UNKNOWN = u64::MAX` 哨兵语义：`all_invisible_for` 对 UNKNOWN 返回 false（未知 → 回落逐行检查）；`BufferPool::clear_all_visible` 首建条目改 `or_insert { min_create_tx_id: MIN_CREATE_UNKNOWN, all_visible: false }`。INSERT 路径 `min(UNKNOWN, W) = W` 恢复真实最小值（0 毒化消除）；`set_all_visible` 的 `or_insert` MAX 与哨兵语义归一（all_visible 短路先于 all_invisible_for 消费，写后清除回落逐行——MAX 毒化闭合）。不重排 insert.rs 调用序、不改 `check_page_all_visible`、不改任何调用方。
6. **收尾全量验证（T6）**——全量 `cargo test --no-fail-fast` 零回归（基线 1050，I041 清账后门稳定）+ clippy `--all-targets -D warnings` 0 + fmt 0 + `openspec validate` PASS + change 结构自检。
7. **RC 重启可见性高水位修复（T7，Iteration 002——用户决策 5 追加）**——checkpoint 位点文件扩展 16B→24B：第三字段 `tx watermark u64 LE`（在 LSN 捕获之后读取的分配器当前值，两次位点写入均携带）；`read_site_file` 兼容读（≥24B 携带水位 / 16B..24B 旧格式无水位 / <16B None 既有）；`RecoveryResult` 增 `checkpoint_tx_watermark: Option<u64>`；`Database::open` 以 max(WAL 观测最大 id, 位点水位) 推进分配器；`Database::checkpoint` 传分配器读数闭包。修复后干净关闭重开，已提交行对 RC 立即可见（不再依赖新 DML 抬水位）；`checkpoint_test.rs` 直接调用点签名机械适配（断言集不变）；Iteration 001 T5 e2e 夹具的 scratch 抬水位 workaround 移除（其 doc 注记同步改写）。
8. **追加收尾全量验证（T8，Iteration 002）**——T7 合入后全量 `cargo test --no-fail-fast` 零回归（基线 1061 + T7 新增）+ clippy/fmt/validate + change 结构自检刷新（000/001 accepted、002 本 Response）。

Delta specs：

- 新增 `in-subquery-join-rejection`：IN 子查询 JOIN 形态诚实拒绝 + 既有 IN 面零回归。
- 修改 `mvcc-tombstone-visibility`：新增 Requirement「页级可见性摘要无毒化（哨兵语义）」——首建条目哨兵 / all_visible 置位后写清除不整页误判 / RC 端到端可达 / 零回归。
- 修改 `cli-noninteractive-shell`：新增 Requirement「标量子查询输出列的表头形状」。
- 修改 `table-name-resolution`：新增 Requirement「import 表名实参转义可达」。
- 修改 `transaction-isolation-levels`（Iteration 002 追加）：新增 Requirement「RC 重启后可见性高水位健全（checkpoint 水位持久化）」——干净重开立即可见 / 位点往返与旧格式兼容 / id 不复用 / 既有 checkpoint·恢复语义零回归。

## BDD 场景草图（缺口扫描结论）

覆盖面按 delta spec 场景落定；要点：

- **Happy**：合并后 env 解析测试全绿；非 JOIN 单列 IN 子查询照常可达；`SELECT id, (SELECT …) AS alias FROM t` 表头在标量位置携带 alias、json columns 数 == rows 宽度；含引号字符表名（`"a""b"` → `a"b`）import 落库成功；RC 模式下 scan→delete→其余行点查可达。
- **Sad**：`IN (SELECT … JOIN …)` 报点名 JOIN 的计划期拒绝（exit 3，文案与事实相符）；多列+JOIN 同文案；毒化修复后 `all_invisible_for` 对未知哨兵返回 false（回落逐行，不整页误判）。
- **Edge**：标量子查询位于首列/末列（result_column_index 0 与 N）；执行器 `row.push` 兜底分支（idx > len）表头同步追加；`r.a` 限定名 ORDER BY 子查询维持更早的既有拒绝（文案本就相符）；`clear_all_visible` 在无条目页被 commit 路径单独调用（`{UNKNOWN, false}` 保守回落）。
- **兼容**：既有 1050 测试零修改通过（除 resolve.rs 两测试合并为新测试——同文件内重构，断言集逐条保留）；RR 默认路径零变化（M21 本就旁路）；RC 既有套件（isolation_level/mvcc_tombstone_visibility）零修改。
- **追加（Iteration 002 范围）**——Happy：干净 close→checkpoint 截断→重开 RC，`SELECT` 立即返回全部已提交行；重开后新 INSERT 的 id 高于水位、新旧行同语句共存。Sad：位点撕裂写（落盘 16..23B）按旧格式解析无水位，行为不差于现状（该窗口缺陷残留，保守）。Edge：checkpoint 进行期间并发分配 id 的崩溃窗口——水位在 LSN 捕获之后读取，位点前缀内 Begin 落盘的 id 必 ≤ 水位或位于重放尾部，max(WAL, 水位) 覆盖。兼容：16B 旧位点/无位点文件路径行为不变；WAL 帧格式与主库文件头零触碰；RR 与 `redo_from` 过滤语义不变。

## Out of Scope / Non-goals

- ISS02 的 IN×JOIN 能力解锁（Join 形态臂取列 + `extract_correlated_params` ON 遍历注册关联参数）——登记 improvement 候选。
- 多列无 JOIN IN 子查询的「静默取首列」语义裁决——本会话探针实证（`IN (SELECT r.a, s.b FROM r, s)` 静默按首列比较 exit 0）；登记 improvement 候选（显式拒绝 or 行值 IN），不在本 change 裁决。
- MS17-T01 最小加密 / MS17-T03 安装面 / MS17-T04 README（后续 change）。
- WAL/checkpoint 加密（MS17-T01 范围预裁定：本轮仅主库文件）。
- WAL 帧格式与主库文件头格式变更（Iteration 002 水位走 `.checkpoint` 位点伴生文件，WAL 帧与主库头零触碰）。
- 数据页派生水位方案（design D8 拒绝——GC 移除历史版本可使派生值低于真实 max，破坏 id 不复用）。
- 页级快路径性能量化（原 MS08 实测域纪律；本 change 只恢复快路径可用性，不做 bench 设施——正式加密/性能 bench 已登记 I057）。
- 非 Int 键列、键位路由、更新索引维护等已收口域（MS15/MS16 specs 锁定，零触碰）。

## 默认假设（用户未显式裁定，按合理默认补齐，可否决）

- **DA1** 新错误文案为英文 `IN subquery with JOIN is not supported`（既有 PlanError 文案全英文，风格一致；Exit 3 既有 `Plan error:` 前缀不变）。
- **DA2** ISS03 表头插列位置 `min(result_column_index, columns.len())`，与执行器 `row.insert`（idx ≤ len）/`row.push`（idx > len）双分支镜像。
- **DA3** 哨兵常量命名 `MIN_CREATE_UNKNOWN`，置于 `page_visibility.rs` 与 `PageVisibilityInfo` 同址；`Default` 保持 `{0, false}` 不变（无人再经 `or_default` 建条目）。
- **DA4** I041 合并后测试名 `test_env_resolution_cases`；两组断言逐条保留，Guard 机制原样。
- **DA5** RC 端到端见证用 `open_with_isolation(ReadCommitted)` lib API（镜像 `tests/isolation_level_test.rs` 基建）；不新增 CLI 隔离级别旗标。
