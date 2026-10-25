# M19 分批全量回归执行配方

- Status: active
- Last validated: 2026-10-25
- Environment: Linux x86_64（WSL2 6.18.40.1）、cargo 1.99.1 / rustc 1.99.1、RTsql `tests/` 集成测试套件（2026-10-23 实例 130 个测试目标；2026-10-24 实例 134 个测试目标 = 133 个 `tests/*.rs` + `--lib`/`--bins`；2026-10-25 MS35 实例 135 个目标 = 134 个文件 + `--lib`/`--bins`；2026-10-25 缺陷修复实例 143 个目标 = 142 个文件 + `--lib`/`--bins`）
- Source: change `2026-10-23-ms33-drop-persistence-failure` Iteration 000 / Cycle 000 Act Response（`openspec/changes/archive/2026-10-23-ms33-drop-persistence-failure/iterations/000-crash-before-checkpoint/000-initial.md`，2026-10-23）；change `2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog` Iteration 001 / Cycle 000 Act Response（`openspec/changes/archive/2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog/iterations/001-ddl-catalog-transactional/000-initial.md`，2026-10-24，13 批 / 134 目标 / 2171 passed / 0 failed / 2 ignored）；change `2026-10-24-ms35-free-page-reuse-across-restart` Iteration 001（`openspec/changes/archive/2026-10-24-ms35-free-page-reuse-across-restart/iterations/001-durable-release-gate/000-initial.md`，2026-10-24，14 批 / 135 目标 / 2183-0-2）；change `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` Iteration 007（`.../iterations/007-legacy-compat-evidence/000-initial.md`，2026-10-25，14 批 / 143 目标 / 2231-0-2）；change `2026-10-24-ms36-legacy-format-and-dump-compat` Iteration 001（`.../iterations/001-contract-and-baseline/000-initial.md`，2026-10-25，13 批 / 136 目标 / 2192-0-2，**等分批次规则**的来源）

## 适用范围

- 需要给出「全量回归通过」结论时（公共规则 M19 强制分批：一次无目标过滤的 `cargo test` 不作为全量结论）。
- 验证某次改动没有破坏既有行为，需要每批的 passed/failed 计数作为交付证据。
- 不适用：单次定向验证（`cargo test --test <name>` 可用默认并行度，M19 明确不约束）；单测或 bench 跑太久/假死的诊断（见 `.agents/runbooks/test-bench-diagnosis.md`）；基准 baseline 对比（见 `.agents/runbooks/ms08-bench-comparison.md`）。

## 前置条件

- 工作区可编译；`cargo test` 能构建本项目的测试目标（`tests/*.rs` + `--lib`/`--bins`；2026-10-23 实例 130 个、2026-10-24 实例 133 个 `tests/*.rs`）。
- 机器可承受串行执行十余批（2026-10-23 实例 14 批、2026-10-24 实例 13 批）；分批的目的是避免一次调用构建链接全部测试二进制并长时间独占机器。
- 全程只跑一批：批内 `--test-threads=1`，批与批顺序执行，不要并发启动多批。

## 操作步骤

1. 先跑库与二进制目标一批：

```bash
cargo test --lib --bins -- --test-threads=1
```

2. 再把 `tests/*.rs` 按**文件名字典序**排序，每批 8–12 个文件（2026-10-23 实例每批 10 个；确定性切分算法见「批计划与对账」节），每批一次独立调用：

```bash
ls tests/*.rs | sort
# 第 N 批（N 从 1 起）：
cargo test --test <f1> --test <f2> ... --test <fk> -- --test-threads=1
```

3. 每批记录退出码与 passed/failed/ignored 计数。计数可从 cargo 原生输出用一步管道读出，管道只做计数、不做判定：

```bash
cargo test --test <f1> ... -- --test-threads=1 2>&1 | grep -E "^test result:"
```

4. 全量结论等价于「每一批各自 0 failed」。缺一批不能声明全量通过；逐批计数要保留在交付记录里。

## 验证

- 成功判据：每一批的 cargo 退出码为 0，且各批 `test result:` 行的 failed 计数为 0。判定只看 cargo 自己的退出码与原生输出，不新增判定脚本、封装命令或判定层。
- 2026-10-23 实例（`acfd274` + MS33 的 D1 改动，130 个测试目标）：14 批，合计 **2151 passed / 0 failed / 2 ignored**。2 个 ignored 是 `cli_test.rs` 里长期标 `#[ignore]` 的校准/诊断用例（`calibration_recovery_time`、`diagnostic_wal_parse`），与本次改动无关。与 MS32 收尾基线 2144/0/2 相比正好多出本次新增的 7 个用例。
- 2026-10-24 实例（`2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog` 收尾全量，134 个测试目标）：13 批，合计 **2171 passed / 0 failed / 2 ignored**（2 个 ignored 同为 `cli_test.rs` 的长期 `#[ignore]` 用例）。与 MS34 收尾基线 2163/0/2 相比 **+8** = 新增 `index_write_atomicity_test.rs` 5 个用例 + 新增 `ddl_catalog_transactional_test.rs` 3 个用例（`statement_cancellation_test.rs` 前后均 8 个用例，本次只增断言）。批计划与对账见下节。
- **2026-10-25 实例（MS35 收尾全量，135 个目标 / 14 批）**：合计 **2183 passed / 0 failed / 2 ignored**（2 个 ignored 同为 `cli_test.rs` 的长期 `#[ignore]` 用例）。目标清单 = 134 个 `tests/*.rs` 文件 + `--lib --bins` ⇒ **135 个目标 / 14 批**（13 个文件批次 + 1 个 lib/bins 批次；按旧算法为 12 批 × 11 + 1 批 × **2**）。与 MS34 收尾基线 2171/0/2 相比 **+12** = 新增 `tests/free_page_lifecycle_test.rs` 的 8 例 + 4 例增量（改名与断言更新不改变用例数）。**注意**：本实例的末批只有 2 个目标，**超出 8–12 区间** ⇒ 用等分规则重排会得到 13 批（10 批 × 11 + 2 批 × 12），但**已执行的批次表不追溯重排**；事实按原样记录。
- **2026-10-25 实例（缺陷修复 change 收尾全量，143 个目标 / 14 批）**：合计 **2231 passed / 0 failed / 2 ignored**。目标清单 = 执行时实际的 142 个 `tests/*.rs` 文件 + `--lib --bins` ⇒ **143 个目标 / 14 批**（B00 = `--lib --bins`；B01–B12 各 11 个目标、B13 10 个 ⇒ 每批落在 8–12 区间内）。**覆盖对账 missing 0 / extra 0 / duplicate 0**（142 个文件各出现恰好一次，执行前后 `ls tests/*.rs | sort` 的 `diff` 一致）。基线失效后的差额归因：`新增文件用例数 + 既有文件增量` = 32 + 7 = **39** = 2231 − 2192（2192/0/2 是 MS36 基线，对当时的工作区有效、对本实例已失效）。
- 批次规模经验值 8–12 个目标/批（M19）。测试目标数变化后按同一分组规则重算批数。

## 批计划与对账（2026-10-24 实例补充）

### 批计划生成

1. 目标清单 = `ls tests/*.rs | sort` 的**文件名字典序**（2026-10-24 实例 133 个文件）；`--lib --bins` **单独成一批**（该批一次调用覆盖 lib 与 bin 两个测试目标）。
2. 切分算法（确定性、可复算）：**剩余 ≤ 12 则取剩余，否则取 11** ⇒ 每批 8–12 个目标（2026-10-24 实例：11×11 + 12，共 12 个文件批次 + 1 个 lib/bins 批次 = **13 批**；2026-10-23 实例用的是每批 10 个的变体，同样落在 8–12 区间内）。
3. **等分批次规则（2026-10-25 补充，MS36 的 D10 定形）**——上一步的切分算法在某些文件数下会产出**越界**的末批（文件数落到特定余数时，末批可能不足 8 个目标）。要保证**每批都落在 8–12**且**不重不漏**，改用下面的等分规则：

```text
文件数 = N（`ls tests/*.rs | sort` 的实际输出条数）
文件批数 B = ceil(N / 12)
文件按字典序切片：前 (B × 12 − N) 批各取 11 个目标，其余各取 12 个
`--lib --bins` 单独成第 B+1 批
```

   - 校验：`11 × (B×12 − N) + 12 × (N − B×12 + B) = N` ⇒ 不重不漏；每批规模 ∈ {11, 12} ⇒ 落在 8–12。
   - 实例：N = 135 ⇒ B = 12，9 批各 11 + 3 批各 12 = 135 ⇒ **13 批**（MS36）；N = 134 ⇒ B = 12，10 批各 11 + 2 批各 12 = 134 ⇒ **13 批**；N = 142 ⇒ B = 12，2 批各 11 + 10 批各 12 = 142 ⇒ **13 批**。
   - **按批数反推**（等价表述）：目标数 ÷ 批数得到的每批规模必须落在 **8–12**；落不进去时调整批数，而不是让末批越界。
   - 用「剩余 ≤ 12 取剩余，否则取 11」算法且文件数使末批越界时：**要么**改用等分规则，**要么**在记录里明确写出该末批的实际规模（不写成「每批 8–12」）。2026-10-25 的 MS35 实例即属后者：134 个文件按旧算法切成 12 批 × 11 + 1 批 × **2** ⇒ 末批 2 个目标超出 8–12 区间，该事实已写入该实例的批次表。
4. 每批一次**独立**调用：`cargo test --test <f1> ... --test <fk> -- --test-threads=1`；批间**严格串行**，同一时刻只有一批在跑（公共规则 M19）。
5. 逐批记录退出码与 passed / failed / ignored 计数；计数只用一步通用管道 `grep -E "^test result:"` 读出（只计数、不判定）。

### 覆盖对账（目标不重不漏）

- 以**实际** `tests/*.rs` 文件集合（`ls tests/*.rs`）为基准，与批次表里的目标清单逐项比对，出三项：
  - **missing**：在文件集合里、但不在任何批次中的目标（须为 0）；
  - **extra**：在批次里、但不在文件集合中的目标（须为 0）；
  - **duplicate**：出现在一个以上批次中的目标（须为 0）。
- 2026-10-24 实例：133 个文件各出现恰好一次 ⇒ missing 0 / extra 0 / duplicate 0。2026-10-25 缺陷修复实例（142 个文件）⇒ missing **0** / extra **0** / duplicate **0**（执行前后 `ls tests/*.rs | sort` 的 `diff` 一致）。
- 对账基准必须是**执行时实际**的文件集合（不是计划时的清单）：计划时的清单只作参考，执行时重新生成后再对账。

### 总数对账（与基线的差额逐项归因）

- **目标数 vs `test result:` 行数不是同一数**：2026-10-24 实例为「134 个测试目标 / 135 条 `test result:` 行」——133 个文件目标各一条 + lib 一条 + bin 一条（`--lib --bins` 一批产出两条）。两数各自成立，不是笔误。2026-10-25 缺陷修复实例同型：**143 个目标 / 144 条 `test result:` 行**（142 个文件各一条 + lib + bin）。⇒ 收尾记录里**两个口径都要写明**。
- 差额公式：`本次终态 − 基线终态 = 本次新增用例数`。2026-10-24 实例：2171 − 2163 = **+8** = 新增测试文件的用例数（5 + 3）+ 既有文件的用例数增量（0）。2026-10-25 缺陷修复实例：2231 − 2192 = **+39** = 新增 7 个测试文件的用例数（5 + 1 + 9 + 7 + 2 + 3 + 5 = 32）+ 既有 `legacy_compat_test` 的增量（9 → 16 = **+7**）⇒ 32 + 7 = 39。
- **基线失效的处理**：目标数或覆盖面变化后，**旧基线不得作为通过判据**；差额必须由**实际结果**说明（逐文件统计新增文件的用例数 + 既有文件的增量），**不得**为了对上基线而调整判据。
- 逐项核对方法：按 `#[test]` / `#[tokio::test]` 独立计数新增文件与既有文件的用例数，核对差额是否恰好等于增量；lib 目标数应与基线相同（2026-10-24 实例 lib = 420 = 基线 420）。
- 差额对不上时的排查顺序：① 批次是否缺一批（M19：缺一批不得声明全量通过）；② 是否有目标被计入两个批次；③ 基线数字是否含 ignored 的计数口径差异；④ 目标清单是否按**执行时实际**的 `tests/` 文件集合重新生成。

### 批次表记录形态

每批一行：批号、目标列表、cargo 退出码、passed / failed / ignored 三项计数。收尾交付时保留整张批次表与三项对账结果，作为「全量通过」结论的证据。

## 失败处理

- **某批 failed**：先单独复核该用例，再整批复核一次（失败后判定是否偶发错误的一次复核；复核通过按通过处理，并在验证记录注明首跑失败）。
- **cargo 在首个失败目标后中止该批的后续目标**：批内目标按给定顺序执行，第一个失败目标之后的测试二进制**不会运行**。复核时必须重跑整批，否则会把「未执行」误记成「通过」。2026-10-23 的 B04 首跑即此形态：e2e_test 失败后 encryption_test / executor_test / explain_test 未执行，整批复核才补齐这 56 个用例。
- **某批跑很久**：见 `.agents/runbooks/test-bench-diagnosis.md`（死锁/空转/setup 过重的区分与 `ps aux` 看 CPU% 的做法）。
- **失败测试与本次改动无关**：不改其他模块去迁就全量。按当前 change 的纪律把失败测试名写进 Act Response 并停止，由用户或 Plan 决定后续。
- 判定失败归因不明时不要重复运行整轮全量；按公共规则只对疑点做定向审计。

## 回滚

不适用（验证执行方法，无产品状态变更）。分批过程不修改仓库文件；测试自身产生的库文件与临时目录随用例结束清理。

## 证据

- `openspec/changes/archive/2026-10-23-ms33-drop-persistence-failure/iterations/000-crash-before-checkpoint/000-initial.md`（Act Response「验证命令与实际结果」的 1.4 分批明细表与 B04 首跑失败/复核记录；14 批 / 130 目标 / 2151/0/2）
- `openspec/changes/archive/2026-10-24-ms34-issue-fix-index-atomicity-and-ddl-catalog/iterations/001-ddl-catalog-transactional/000-initial.md`（Act Response 的 T6 收尾全量批次表：13 批 / 134 目标 / 2171/0/2，与基线 2163/0/2 的 +8 差额归因；Plan Review 的 F9 覆盖对账与算术自洽核对）
- `openspec/changes/archive/2026-10-24-ms35-free-page-reuse-across-restart/iterations/001-durable-release-gate/000-initial.md`（Act Response 的 T7 收尾全量批次表：14 批 / 135 目标 / 2183/0/2；与基线 2171/0/2 的 +12 差额归因；末批 2 个目标的越界事实）
- `openspec/changes/archive/2026-10-24-ms36-legacy-format-and-dump-compat/iterations/001-contract-and-baseline/000-initial.md`（Act Response 的 T4 收尾全量批次表：13 批 / 136 目标 / 2192/0/2；**`design.md` D10 的等分批次规则**——`B = ceil(文件数 / 12)`、前 `B×12 − 文件数` 批各 11 个、其余各 12 个）
- `openspec/changes/archive/2026-10-24-open-defect-batch-storage-ddl-legacy-evidence/iterations/007-legacy-compat-evidence/000-initial.md`（Act Response 的 T21 收尾全量批次表：14 批 / 143 目标 / 2231/0/2；12 条纪律逐条成立；覆盖对账 missing 0 / extra 0 / duplicate 0；目标数 143 与 `test result:` 行数 144 的口径说明；基线 2192/0/2 已失效与差额归因 32 + 7 = 39；Plan Review 的 F8 独立复算与对账）
- 公共规则 M19：`.agents/memory/project-model.md` 的「M19: 全量测试分批执行约束」
- 相关配方：`.agents/runbooks/test-bench-diagnosis.md`（慢/假死诊断）、`.agents/runbooks/in-process-reopen-testing.md`（进程内 close→reopen 测试夹具）
