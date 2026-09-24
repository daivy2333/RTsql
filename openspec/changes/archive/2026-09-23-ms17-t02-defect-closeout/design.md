# MS17-T02 缺陷清账 — Design

> 调查基线：2026-09-23，工作区 = master `7364bc9` + MS13 实施 + 收尾 docs sync（未提交）。全部结论为本会话新鲜读码/探针取证，非沿引台账旧证。

## Current-State Evidence

### 快照与 M21 暴露面（ISS01/MAX 毒化的前提界定）

- `Database::statement_snapshot`（`src/database.rs:127-139`）：RR → `None`（历史无快照语义逐字节保持）；RC → `Some(Snapshot::statement_view(...))`。
- 快照消费点：`execute_stage`（pipeline.rs:203，auto-commit）与 `execute_stage_in_tx`（:306，会话事务）都经 `statement_snapshot`——RR 下恒 None。`find_visible_version` 三调用方（`index_scan.rs:82`/`index_scan_all.rs:84`/`scan.rs:65`）全部 `if let Some(snapshot)` 门控；DataScan 快路径（`data_scan.rs:404-414`）`zip(page_vis)` None 即 false。
- **结论**：M21 页级可见性机制（vis_map、双快路径、`set_all_visible`/`check_page_all_visible`）仅在 RC 模式激活；RR 默认路径（CLI 全部形态）vis_map 条目被写入但永不被消费。两个毒化缺陷的暴露面 = RC 模式 lib API。
- CLI RR 探针（本会话）：`SELECT * FROM t; DELETE WHERE id=1; SELECT WHERE id=2; SELECT * FROM t` 单进程多语句——点查与全扫均正确（快路径未激活，与界定一致）；显式事务探针同结果。

### ISS01（0 毒化，Confirmed，台账证据 + 本会话读码复认）

- `clear_all_visible`（`buffer_pool.rs:390-395`）：`.entry(page_id).and_modify(|i| i.all_visible = false).or_default()`——无条目页首建 `{min=0, all_visible=false}`。
- `update_visibility_on_insert`（:399-410）：`and_modify(min=min, all_visible=false).or_insert({min=W, false})`。
- INSERT 路径（`insert.rs:172-174`）与恢复镜像（`recovery.rs:182-183`）先 clear 后 update → `min(0, W)=0` 永久钉零 → `all_invisible_for(hw) = 0 > hw` 恒 false → 整页跳过快路径永不触发（保守失效）。

### MAX 毒化（本会话新发现，Confirmed 结构链）

1. DataScan 页扫毕（action JumpToPage/Done、snapshot Some、双 flag false）→ `check_page_all_visible` 通过 → `set_all_visible`（`buffer_pool.rs:378-386`）：**条目不存在时** `or_insert { all_visible: true, min_create_tx_id: u64::MAX }`（vis_map 进程内存态，重启后为空，or_insert 臂在重启后首次扫描即可达）。
2. 该页任一写路径 `clear_all_visible`：`delete.rs:77/79`（墓碑页+原行页）、`update.rs:169/171`（新旧两页）、`manager.rs:259`（commit 路径逐 version 页）、`data_page.rs:144`——条目已存在 → `and_modify` 只改 `all_visible=false`，**min 保持 MAX** → `{MAX, false}`。
3. 后续 RC 语句 `find_visible_version`：`all_visible=false` → `all_invisible_for(hw) = MAX > hw` 恒 true → `return Ok(None)`——**该页所有行对所有快照不可见**；DataScan 同谓词（`data_scan.rs:409-414`）整页跳过。直至该页下一次 INSERT 经 `min(MAX, W)=W` 自愈。
4. `set_all_visible` 自身的 MAX 在 `all_visible=true` 下安全（`find_visible_version` :292 `all_visible` 分支短路先于 :313 消费）——危险形态仅是「MAX + all_visible=false」组合。

### RC 重启可见性回归（Iteration 001 实施期新发现，Confirmed——夹具探针 + Plan Review 独立读码，2026-09-23）

- `Database::open`（`database.rs:79-86`）：`advance_past(max(WAL 观测 committed/aborted/uncommitted))`——分配器水位的唯一恢复来源是 WAL 重放观测。
- checkpoint 九步（`checkpoint.rs:94-133`）：lsn 捕获 → flush 脏页 → fsync WAL → 写位点 (lsn, ts) → 追加 Checkpoint 帧 → `rewrite_truncate(lsn)` 保留 [lsn..end) → 位点置 (0, ts2)。干净 close 后残余 WAL 仅含 Checkpoint 帧（无 Begin/Commit）→ 三集皆空 → max=0 → **分配器从零起步**。
- `statement_snapshot` RC 臂以 `current_tx_id()`（分配器当前值）为高水位；`database.rs:77-81` 注释自证健全性前提「every id ≤ the allocator's current value is committed, aborted, or active」——checkpoint 截断使该前提失效。重启前已提交行 create_tx > 高水位 → RC 不可见，随新分配渐进「复现」。
- 位点文件：伴生 `<db>.checkpoint` 16B（lsn u64 LE + timestamp u64 LE）；`read_site_file`（<16B → None，恢复端与 CheckpointManager 共享语义）；`write_checkpoint_site` truncate+write+sync_all。
- 调用链收口：`close()` → `Database::checkpoint()`（`database.rs:234-240`，CLI 优雅停机同源）→ `CheckpointManager::checkpoint()`——生产唯一接线点；`RecoveryResult` 唯一消费方 `database.rs` open。`tests/checkpoint_test.rs` 3 处直接构造 CheckpointManager（:15/:50/:75，签名适配面）。
- 实证：`tests/isolation_level_test.rs` T5 夹具首跑前置断言 `left: 0, right: 3`（close+reopen RC SELECT 0 行）；RR 同形 reopen 有行（`checkpoint_redo_reduction_test` 对照）。

## Decisions

### D1 ISS01 + MAX 毒化：哨兵未知语义（Design A）

- `page_visibility.rs` 新增 `pub const MIN_CREATE_UNKNOWN: u64 = u64::MAX;`；`all_invisible_for` 改为 `self.min_create_tx_id != MIN_CREATE_UNKNOWN && self.min_create_tx_id > snapshot_tx_id`（未知 → false → 回落逐行检查，与 Default 安全语义同向）。
- `clear_all_visible` 首建改 `.or_insert(PageVisibilityInfo { min_create_tx_id: MIN_CREATE_UNKNOWN, all_visible: false })`。
- 闭合推演：INSERT 路径 clear（首建 `{UNKNOWN,false}`）→ update `min(UNKNOWN, W)=W` ✓；set_all_visible `{true, UNKNOWN}` 写后清除 → `{UNKNOWN, false}` → `all_invisible_for`=false 回落逐行 ✓（MAX 毒化闭合）；commit 路径对无条目页孤立 clear → `{UNKNOWN,false}` 保守 ✓；恢复镜像同 INSERT ✓。`set_all_visible` 的 `or_insert` MAX 原样保留（语义归一为 UNKNOWN）。
- **拒绝的替代方案**：(b) `check_page_all_visible` 顺带计算真实 min 并传给 `set_all_visible`——更侵入（签名变化、调用链改动），且不解决 insert 路径 0 毒化（仍需调序）；(c) 仅调换 insert.rs clear/update 顺序——只修 0 毒化，MAX 毒化原样。Design A 两处小改同域闭合两个缺陷。
- 语义注记：修复后 `min_create_tx_id = 0` 只可能来自真实 tx_id 0（aborted 标记位，MS06-T01 后 DML 不产生）——`all_invisible_for(0 > hw)` 恒 false，保守无害。

### D2 ISS02：Join 形态显式拒绝臂 + 新错误变体

- `error.rs` 新增 `PlanError::InSubqueryJoinUnsupported`，Display `IN subquery with JOIN is not supported`（英文风格与 `UnsupportedJoinType => "Only INNER JOIN is supported"` 一致）。
- `get_subquery_first_column`（`subquery.rs:386-440`）补 `PhysicalPlan::Join(_) | PhysicalPlan::NestedLoopJoin(_) => Err(PlanError::InSubqueryJoinUnsupported)`；`_` fallback 逐字节保持。
- 探针边界（本会话）：单列 JOIN 子查询 → 误报多列（本 change 修复对象）；多列+JOIN → 同文案（真多列时文案碰巧相符，修后统一为主因 JOIN 文案）；`WHERE + JOIN` 子查询 → 既有 `Unsupported statement type`（相符，不动）；ORDER BY 限定名子查询 → 更早的 `ORDER BY only supports column names`（相符，不动）。
- 事实注记：多列无 JOIN IN 子查询静默按首列比较（`IN (SELECT r.a, s.b FROM r, s)` → 按首列比较 exit 0）——既有语义，本 change 不裁决；`get_subquery_first_column` 返回值仅作 SemiJoin/AntiJoin 元数据（非首列 IN 子查询行值经子查询计划自身投影，探针验证结果正确），改臂不触碰值比较路径。

### D3 ISS03：SubqueryEval 表头插列（镜像执行器）

- `query.rs:114` 臂改为：取输入列名向量 → `let idx = node.result_column_index.min(columns.len()); columns.insert(idx, node.output_column.clone())`——镜像 `subquery_eval.rs:187-192` 的 `if idx <= row.len() { insert } else { push }`（`min` 使 idx > len 时落到 len 位 = 追加语义）。
- 字段就绪：`SubqueryEvalNode`（`executor/plan.rs:436-447`）`output_column`/`result_column_index` 均 `pub`。构造点 `query.rs:1198-1212`（result_column_index = proj_idx − 前置标量子查询数，右到左包装）不变。
- 消费面：CLI `run_sql`（`cli/mod.rs:363`）唯一表头来源；lib `Response::Rows` 无列元数据；`subquery_test.rs` 只断言行值（20 用例零校准）；cli_test 无标量子查询用例（新增 RED e2e）。其上 Projection 包装（表达式项形态，`query.rs:1216-1223`）的 Projection 臂返回自身 columns，不受影响。

### D4 I048：import 表名 quote_ident 包裹

- `lifecycle.rs:496` `format!("INSERT INTO {} VALUES ({});", table, …)` → `quote_ident(&table)`（`lifecycle.rs:594-596`，与 dump `:192`/`select_all_rows` `:220`/分析命令 `:666` 同源）。
- 可达性核实链：`get_table(&table)` 实参逐字比对（:424 附近）不动；SQL 侧 `quote_ident("a\"b")` → `"a""b"` → sqlparser 解析 value = `a"b` → `object_name_to_table_name`（`ast.rs:239-245`，lowercase + join，**不去引号**）= catalog 名 ✓。裸名 `items` → `"items"` → value `items` ✓ 与现状等价。历史「完整带引号名」（catalog 名含首尾引号，如 `"items"`）无现行建名通路（I039 归一化后 `CREATE TABLE "items"` 落 catalog `items`），修复代码路径同样覆盖（实参带引号 → 双重转义往返），e2e 以可直达的转义引号名（`a"b`）作见证。
- 现行可复现缺陷形态：`CREATE TABLE "a""b"(i INT)` → catalog `a"b` → `import … a"b` → 现状 `INSERT INTO a"b …` SQL 解析报错；修后可达。

### D5 I041：env 测试合并单测试

- `resolve.rs:80-127` 两个 `#[test]` 合并为单 `test_env_resolution_cases`：两组断言逐条保留、顺序执行；`EnvGuard` 机制原样。文件头 doc 注释（:41-42「涉及 env 的用例集中在单个 #[test] 内顺序执行」）与实现恢复一致。
- 竞态消除为结构性（单测试无线程并行），非概率性缓解；全量门一次性通过即为验证（不重试增强）。

### D6 Iteration 划分

- Iteration 000 `surface-defects`（T1-T4）：测试基建 + planner/CLI 表面小缺陷批——四个任务互不依赖、各自 RED→GREEN、同属「用户可见面/验收门清账」，聚合为单一可验证成果（先例：MS15-Rest 批处理）。I041 在此先行，后续 Iteration 与 T6 的全量门不再受假失败污染。
- Iteration 001 `visibility-summary-closeout`（T5-T6）：ISS01+MAX 毒化（同函数族同故障域，MS09 Iter000 曾三轮拉锯的高危域，独立隔离便于排障）+ change 收尾全量门。
- 平衡审计：000 四任务各自独立验收、变更面零重叠（resolve.rs / subquery.rs+error.rs / query.rs / lifecycle.rs），无过碎（每任务有独立 RED 见证与用户可见行为变化）；001 两任务形成「高危域修复 + change 级验证闭环」内聚成果。拆三分会产生无独立验收价值的薄 Iteration。

### D7 验证策略

- 逐任务最简直接判定：T1 合并测试通过 + 全量一次绿；T2 lib 级 `execute_sql` 错误文案断言（plan 期拒绝，无需 e2e）；T3 cli_test json `columns`/`rows` 宽度断言（RED：4 列对 5 值）；T4 cli_test 建转义名表 + import + 行数断言（RED：解析报错）；T5 `page_visibility` 单测（哨兵语义）+ RC 集成测试（镜像 `isolation_level_test.rs` 基建：scan→delete→点查可达 + 单测级条目状态断言）。
- T6 全量 `--no-fail-fast` + clippy/fmt/validate，一次通过；不建身份型证据、不建判定层；Persisted Evidence 全部 `none`（Act Response 承载，输出 ≤20 行/项）。

### D8 RC 重启水位：checkpoint 位点 16B→24B（Iteration 002，用户决策 5 追加）

- **位点扩展**：`.checkpoint` 第三字段 `tx watermark u64 LE`；两次位点写入（步骤 5 截断前 lsn 位点 + 步骤 8 截断后 lsn=0 位点）均携带。
- **水位捕获时机（健全性关键）**：水位 MUST 在 LSN 捕获**之后**读取——`CheckpointManager::checkpoint` 增参 `tx_watermark: impl Fn() -> u64`，于步骤 1（LSN 捕获）之后立即调用；`Database::checkpoint` 传 `|| self.transaction_manager.current_tx_id()`。推演：位点前缀内（offset < lsn）落盘 Begin 的事务，其 id 分配先于 Begin 写入、Begin 写入先于 LSN 捕获，故 id ≤ 捕获后读取的水位；水位读取后才分配 id 的事务，其 Begin 必落在 offset ≥ lsn（WAL 追加只写）→ 重放尾部观测覆盖。因此任意崩溃点 max(WAL 观测, 位点水位) ≥ 一切已分配历史 id。若水位经参数在调用前捕获（LSN 之前），则「wm 读取与 LSN 捕获之间分配且 Begin 落前缀」的 id 双双漏观测——禁止该形态。
- **崩溃窗口演化**：截断前崩溃（位点 lsn>0）→ 前缀 id 不观测但水位覆盖 ✓；截断后（位点 0）→ 残余 WAL 无 id，水位携带 ✓；位点撕裂写（落盘 16..23B）→ 按旧格式解析无水位 → 行为不差于现状（该窗口缺陷残留，保守；位点本就 truncate+write+sync_all）。
- **消费侧**：`RecoveryResult` 增 `checkpoint_tx_watermark: Option<u64>`（`full_recover` 从位点直填）；`Database::open` `advance_past(max(max_tx_id, watermark.unwrap_or(0)))`。兼容读：`read_site_file` ≥24B 携带水位 / 16B..24B 旧格式无水位 / <16B None（既有）。
- **不触碰面**：WAL 帧 `WalRecord::Checkpoint { lsn, timestamp }` 格式不变；主库文件头（12B 预留/盐槽）零触碰——留给 MS17-T01 加密域。位点是明文伴生文件，与用户决策 2「checkpoint 位点保持明文」一致。
- **测试适配面**：`checkpoint_test.rs` 3 处直接构造/调用点机械适配新签名（断言集不变）；`checkpoint_redo_reduction_test`/CLI 全走 `Database::checkpoint` 不受影响。Iteration 001 T5 e2e 夹具移除 scratch 抬水位 workaround（doc 注记同步改写——预存缺陷已修复）。
- **拒绝的替代方案**：(e) 恢复后数据页派生最大 id——GC 已移除历史版本可使派生值低于真实 max → id 复用（恰破坏 advance_past 存在目的）；且干净重开全量走页破坏 checkpoint 快重开价值。(f) 主库文件头 12B 预留位存水位——头部为 MS17-T01 加密域（flag/盐槽），创建期一次写语义变为每 checkpoint 重写，与加密 change 碰撞且无收益；位点伴生文件即为此类信息既存的家。

## Risks and Notes

- MAX 毒化为运行时结构链实证（五点读码互证 + RC 暴露面界定），RR 探针不复现与界定一致；Act 以 RED 单测/RC 集成测试提供运行时见证（test-first）。若 RED 无法构造（结构链有未预见断裂），按 Gate 6 阻塞返回 Plan——这是 T5 的 Stop when。
- `min(MAX, W)` 自愈窗口：MAX 毒化页在下一次 INSERT 后恢复；修复前 RC 用户如遇「重启→扫描→删改→点查空」现象即本缺陷，README 已知限制不涉及（RC 为 lib API 特性，README 面向 CLI）。
- 台账修订：MAX 毒化发现与 ISS01 修复结果由 Act Response Experience Candidates 报告，Recorder 按用户指令更新 `ISS01` 台账（含处置记录追加）。
- 多列 IN 首列静默语义、IN×JOIN 能力解锁两项 improvement 候选随 Act Response 报告，落账走 Recorder/docs-maintainer 流程。
