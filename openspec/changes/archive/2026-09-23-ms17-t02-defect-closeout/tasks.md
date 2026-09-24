# MS17-T02 缺陷清账 — Tasks

> 全局任务编号 T1–T8（T7/T8 为 2026-09-23 用户裁定追加的 Iteration 002——RC 重启可见性回归修复，proposal 用户决策 5）；Iteration 规划见文末 Iteration Plan。状态：`pending` / `in-progress` / `done` / `skipped`。

## T1: I041 resolve env 测试合并单测试

- 状态: done
- `src/cli/resolve.rs`：`test_bare_name_env_cases` + `test_db_dir_env_cases` 合并为单 `test_env_resolution_cases`（两组断言逐条保留、顺序执行；`EnvGuard` 原样）；文件头 doc 注释与实现恢复一致。
- 无产品代码变化。
- 验证: 合并测试通过；全量套件一次绿（结构性消除并行竞态，不重试增强）。

## T2: ISS02 IN×JOIN 诚实拒绝

- 状态: done
- `src/parser/error.rs`：新增 `PlanError::InSubqueryJoinUnsupported`，Display `IN subquery with JOIN is not supported`。
- `src/parser/planner/subquery.rs::get_subquery_first_column`：补 `Join | NestedLoopJoin` 显式拒绝臂；`_` fallback 与既有各形态臂逐字节保持。
- 测试见证（RED 先行）：现状单列 JOIN 子查询报 `Subquery returns multiple columns`；修后报新文案。lib 级用例（`tests/subquery_test.rs` 追加）：单列 JOIN 拒绝文案点名 JOIN / 多列+JOIN 同文案 / WHERE+JOIN 维持既有文案 / 非 JOIN 单列 IN 可达不变。
- 验证: 新用例全绿 + 既有 subquery/nested_loop_join 套件零修改。

## T3: ISS03 SubqueryEval 表头插列

- 状态: done
- `src/parser/planner/query.rs::get_plan_output_columns` SubqueryEval 臂：输入列名向量在 `min(result_column_index, len)` 处插入 `node.output_column`（镜像执行器 insert/push 语义）。执行器、计划构造、`SubqueryEvalNode` 字段零改动。
- 测试见证（RED 先行）：cli_test 新增 json 用例——现状 `columns` 4 元素对 `rows` 5 值；修后 `columns` 含别名且长度 == 行宽。含标量位于末列的 table 形态用例。
- 验证: 新用例全绿 + `tests/subquery_test.rs` 行值断言零修改 + I034 既有表头用例零修改。

## T4: I048 import 表名 quote_ident

- 状态: done
- `src/cli/lifecycle.rs::import_csv` INSERT 构造：表名实参经 `quote_ident` 包裹。实参比对（`get_table`）与 CSV 表头匹配逻辑零改动。
- 测试见证（RED 先行）：cli_test 新增用例——`CREATE TABLE "a""b"(i INT)` 建转义名表 → import 实参 `a"b`：现状 SQL 解析报错；修后落库成功且行可回读。裸名 import 既有用例零修改通过。
- 验证: 新用例全绿 + 既有 cli_test import/dump 组零修改。

## T5: ISS01 + MAX 毒化页级摘要哨兵修复

- 状态: done
- `src/storage/page_visibility.rs`：新增 `pub const MIN_CREATE_UNKNOWN: u64 = u64::MAX`；`all_invisible_for` 对 UNKNOWN 返回 false；doc 注释更新（哨兵语义 + MS17 收口注记）；既有 `Default`/`new` 语义不变。
- `src/storage/buffer_pool.rs::clear_all_visible`：`or_default()` 改 `or_insert(PageVisibilityInfo { min_create_tx_id: MIN_CREATE_UNKNOWN, all_visible: false })`。`set_all_visible`/`update_visibility_on_insert`/调用方（insert/delete/update/commit/recovery/data_page）零改动。
- 测试见证（RED 先行）：`page_visibility.rs` 单测——UNKNOWN 哨兵 `all_invisible_for` 为 false（现状 `u64::MAX` 输入为 true → RED）+ 真实值语义保持断言；`buffer_pool` 新测试模块——MAX 链 set→clear 后 `all_invisible_for(0)` 为 false（现状 true → RED）+ INSERT 序后 `min_create_tx_id == W`（现状 0 → RED，含 S1 THEN t<W/t>=W 断言）。集成测试（`isolation_level_test.rs` 追加，镜像既有基建）：RC restart→水位抬升（scratch 表 DML）→scan 置位→delete→点查与全扫可达（现状点查/全扫双空集 → RED）。
- 验证: 新单测/集成用例全绿 + `mvcc_tombstone_visibility_test`/`isolation_level_test`/`gc_test`/`version_chain_test` 零修改全绿。

## T6: 收尾全量验证

- 状态: done
- 全量 `cargo test --no-fail-fast`：**1061 passed / 0 failed / 2 ignored**（基线 1056 + 新增 5，零回归）；`cargo clippy --all-targets -- -D warnings` 0（仅环境级 cargo config 弃用提示）；`cargo fmt --check` 0；`openspec validate` changes 1 passed + specs 36 passed。
- change 结构自检：tasks 状态与实际完成一致；specs/design 与已实现行为一致（delta spec S1-S4 与 design D1 逐点核对）；Iteration 与 Cycle 文件齐全；`Review Result` 与流程状态一致（000 accepted / 001 pending 本 Response）。
- 验证: 各命令决定性输出 + 退出码记入 Act Response。

## T7: RC 重启可见性高水位修复（checkpoint 位点水位，Iteration 002）

- 状态: done
- `src/wal/checkpoint.rs`：位点 16B→24B（第三字段 `tx watermark u64 LE`）——`write_checkpoint_site` 携带水位；`read_site_file` 兼容读（≥24B 携带水位 / 16B..24B 旧格式无水位 / <16B None 既有）；`CheckpointManager::checkpoint` 增参 `tx_watermark: impl Fn() -> u64`，**LSN 捕获之后**调用，步骤 5 与步骤 8 两次位点写入均携带（design D8 健全性推演）。
- `src/wal/recovery.rs`：`RecoveryResult` 增 `checkpoint_tx_watermark: Option<u64>`，`full_recover` 从位点直填；redo/分类逻辑零改动。
- `src/database.rs`：`open` 以 `max(WAL 观测最大 id, watermark.unwrap_or(0))` 推进分配器；`checkpoint` 传 `|| self.transaction_manager.current_tx_id()`。
- 测试见证（RED 先行）：`tests/isolation_level_test.rs` 新 e2e——干净 close → 重开 RC → `SELECT * FROM t` 立即返回三行（现状 0 行 RED）；位点单测（24B 往返 / 16B 旧格式无水位 / <16B None）；`checkpoint_test.rs` 直接调用点签名机械适配（断言集不变，预授权适配面）；Iteration 001 T5 e2e 夹具移除 scratch 抬水位 workaround（doc 注记同步改写为修复后语义）。
- 验证: 新用例全绿 + `checkpoint_redo_reduction_test`/`recovery_test`/`recovery_e2e_test` 零修改 + 全量零回归（T8）。

## T8: 追加收尾全量验证（Iteration 002）

- 状态: done
- 全量 `cargo test --no-fail-fast`（基线 1061 + T7 新增，零回归）+ clippy `--all-targets -D warnings` 0 + fmt 0 + `openspec validate` PASS；结构自检刷新（000/001 accepted、002 本 Response；specs 含 transaction-isolation-levels delta）。
- 验证: 各命令决定性输出 + 退出码记入 Act Response。

---

## Iteration Plan

### Iteration 000: surface-defects（测试基建 + planner/CLI 表面缺陷批）

- Tasks: T1, T2, T3, T4
- Depends on: None
- Stable baseline: 全量假失败源消除（I041）；IN×JOIN 拒绝文案与事实相符且无新增静默面（ISS02）；标量子查询表头与行形状一致（ISS03）；含引号名 import 可达（I048）。后续 Iteration 可在其稳定的全量门上工作。
- Verification boundary: 四任务新增测试全绿 + 既有 subquery/cli 投影/import 套件零修改 + 本 Iteration 末全量一次绿。
- Diagnostic boundary: `src/cli/resolve.rs`、`src/parser/{error,planner/subquery,planner/query}.rs`、`src/cli/lifecycle.rs`、`tests/{subquery_test,cli_test}.rs`。
- Non-goals: 页级可见性摘要（后续 Iteration）；IN×JOIN 能力解锁与多列 IN 语义裁决（improvement 候选）；执行器行产出改动。
- 平衡审计: 四任务变更面零重叠、各自独立 RED→GREEN 与用户可见行为变化，聚合为「初版前用户可见面清账」单一成果；无过碎（每任务独立验收），无需拆分。

### Iteration 001: visibility-summary-closeout（ISS01+MAX 毒化修复 + change 收尾）

- Tasks: T5, T6
- Depends on: Iteration 000（全量门稳定）
- Stable baseline: RC 模式页级摘要无毒化（0 毒化与 MAX 毒化闭合，快路径恢复可用且保守回落正确）；全 change 收口（全量/clippy/fmt/validate/结构自检）。
- Verification boundary: `page_visibility`/`buffer_pool` 新单测 + RC 集成用例全绿 + 既有可见性/隔离套件零修改 + T6 全量验证记录。
- Diagnostic boundary: `src/storage/{page_visibility,buffer_pool}.rs`、`tests/isolation_level_test.rs`（基建镜像）；T6 为 change 级验证。
- Non-goals: 页级快路径性能量化（improvement 域）；MVCC 可见性语义变化（本 Iteration 只修摘要信息质量，行级语义零触碰）；ISS 台账落账（Recorder 流程）。
- 平衡审计: ISS01 与 MAX 毒化同函数族同故障域（vis_map 条目生命周期），合并为单一「摘要无毒化」成果；T6 收尾任务并入形成 change 级验证闭环，单独拆出会产生无独立验收价值的薄 Iteration。

### Iteration 002: restart-watermark（RC 重启水位修复 + 追加收尾，2026-09-23 用户裁定追加）

- Tasks: T7, T8
- Depends on: Iteration 001（accepted；全量 1061 稳定门）
- Stable baseline: 干净关闭重开后 RC 高水位健全（已提交行立即可见、重启后 id 不复用、位点文件向后兼容读取）；change 二次收口（全量/clippy/fmt/validate/结构自检刷新）。
- Verification boundary: 位点单测 + RC 重开 e2e（RED 先行）全绿 + checkpoint/恢复既有套件零修改（`checkpoint_test.rs` 预授权签名适配点除外，断言集不变）+ T8 全量验证记录。
- Diagnostic boundary: `src/wal/checkpoint.rs`（位点读写 + checkpoint 水位捕获）、`src/wal/recovery.rs::full_recover`、`src/database.rs::{open,checkpoint}`、`tests/{isolation_level_test,checkpoint_test}.rs`。
- Non-goals: WAL 帧格式与主库文件头（水位走位点伴生文件，零触碰）；数据页派生方案（design D8 拒绝）；加密域（MS17-T01）；ISS 台账落账（Recorder 流程）。
- 平衡审计: T7 单一故障域（重启水位健全性）自包含修复——位点写入/读取/恢复消费三点一线，与 000/001 变更面零重叠（wal/checkpoint + database vs storage/page_visibility）；T8 change 级二次收口并入（Iteration 001 T5/T6 同构先例）。不与 000/001 合并：001 已 accepted 冻结，且本缺陷在 001 Review 时为范围外候选，需独立 Iteration 隔离验收边界与诊断面。

## Requirements Traceability Matrix

| Requirement (delta spec) | Scenario 代表 | Design | Task | Iter | Code Surface | Test Witness | Simplification | Status |
|---|---|---|---|---|---|---|---|---|
| in-subquery-join-rejection R1 诚实拒绝 | 单列/多列 JOIN 点名 JOIN；WHERE+JOIN 既有 | D2 | T2 | 000 | `subquery.rs::get_subquery_first_column`、`error.rs` | subquery_test 新增 4 用例（RED：误报多列文案） | None | Covered |
| in-subquery-join-rejection R2 零回归 | 非 JOIN 可达/套件零修改 | D2 | T2 | 000 | 同上 | subquery_test 既有 28 用例零修改 | None | Covered |
| cli-noninteractive-shell 新增 R 表头形状 | 标量位置携带别名/末列/零回归 | D3 | T3 | 000 | `query.rs::get_plan_output_columns` SubqueryEval 臂 | cli_test 新增 json/table 用例（RED：4 列对 5 值） | None | Covered |
| table-name-resolution 新增 R import 转义可达 | 转义名 import/裸名不变 | D4 | T4 | 000 | `lifecycle.rs::import_csv` | cli_test 新增用例（RED：解析报错） | None | Covered |
| （测试基建，无行为 requirement）I041 | env 用例合并串行 | D5 | T1 | 000 | `src/cli/resolve.rs` tests | 合并测试 + 全量一次绿（结构性） | None | Covered |
| mvcc-tombstone-visibility 新增 R 摘要无毒化 | 首建真实 min/写后不误判/RC 端到端/零回归 | D1 | T5 | 001 | `page_visibility.rs`、`buffer_pool.rs::clear_all_visible` | page_visibility/buffer_pool 单测（RED：哨兵误判/毒化 0）+ RC 集成用例 | None | Covered |
| transaction-isolation-levels 新增 R 重启高水位健全 | 干净重开立即可见/位点往返兼容/id 不复用/零回归 | D8 | T7 | 002 | `checkpoint.rs`（位点读写+水位捕获）、`recovery.rs::full_recover`、`database.rs::{open,checkpoint}` | e2e（RED：重开 RC 0 行）+ 位点单测（24B 往返/16B 兼容/<16B None）+ checkpoint 套件（预授权适配点断言不变） | None | Covered |
| 全部域零回归（追加） | 全量零修改 | D7/D8 | T8 | 002 | — | 全量 `--no-fail-fast` + clippy/fmt/validate | None | Covered |
| 全部域零回归 | 全量零修改 | D7 | T6 | 001 | — | 全量 `--no-fail-fast` + clippy/fmt/validate | None | Covered |
