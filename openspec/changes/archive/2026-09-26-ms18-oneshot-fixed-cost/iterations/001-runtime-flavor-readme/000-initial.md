# Iteration 001 / Cycle 000: runtime flavor 实测定形与 README 刷新

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T2, T3
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R2（runtime flavor 实测定形——proposal 级，无行为 spec）、R3（README 双语对比板块刷新）；design D5/D6（判据与口径已于 2026-09-26 随 change 计划获用户批准）；R26 runbook 测量纪律
- Excluded scope: T1（Iteration 000 已 accepted，不动 checkpoint 守卫面）；引擎级 criterion 重测（数字不受影响）；分配器/流式化/fsync 合并等 MS22 域；Server 面 runtime；WAL fsync 语义；README 对比板块以外的一切文档段（含备份边界段）

**Objective**

`current_thread` 与 `multi_thread` 两 flavor 各按 R26 口径完成三组实测（×3 取中位），按已批准判据（RSS ≥15% 降且 one-shot 时延与负载吞吐回归 ≤5% → 切换，否则保持）执行决策并留档；README 双语对比板块受影响数字刷新到 change 后真实状态（含 T1 效果与 flavor 决策），双语逐项一致。

**Background**

Iteration 000 已交付 checkpoint 零工作跳过（accepted 2026-09-26）：只读 one-shot 的 close 不再写位点/重写 WAL，实测时延 12.35 → 5.85 ms（Act 背靠背单轮）。本 Iteration 在该基线上定形 runtime flavor（I063 的 RSS 主体仍在）并刷新 README 对比板块（10.8 ms / 16.7 MiB 数字已失效）。

**Investigation Facts**

- Current Baseline: Iteration 000 最终 Act Response（本会话）——T1 交付：`CheckpointManager::checkpoint` 零工作守卫 + `BufferPool::has_dirty_pages`，1285 tests / 0 failures / 2 ignored（全量串行），Review Result: accepted；one-shot 时延改动后 5.85 ms/run（R26 步骤 5 单轮，release 构建）；README 对比板块现行值：Peak RSS ~16.7 MiB / One-shot `SELECT 1` 10.8 ms（2026-09-24 采集，已不代表 change 后状态）。
- Current-State Evidence:
  - flavor 切换面：`src/main.rs:3` `#[tokio::main]`（multi_thread 默认）→ 候选 `#[tokio::main(flavor = "current_thread")]`，单行 attribute 变更；A/B 无需 stash 机制——改 attribute → `cargo build --release` → 测量，按判据保留或还原。
  - current_thread 行为面（design D5 调查结论，实施时验证）：CLI 生产路径 open → 语句 → close（`src/cli/mod.rs:347`）；WALBuffer 后台 flush loop 为 `tokio::spawn` 任务（`src/database.rs:75`），current_thread 上同线程协作调度、功能不变；WAL/页 I/O 经 `spawn_blocking`（独立阻塞池，与 flavor 无关）；DataScan 预取 CLI 默认关闭；Server 不经 `main.rs`；`tokio::signal` current_thread 支持。
  - 测量口径（R26 runbook）：① one-shot 时延——预建 2000 行库（python 生成 workload.sql，2000 行 ≈103KB 安全线）50 次 `rtsql <db> "SELECT 1"` 循环均值；② 峰值 RSS——`/usr/bin/time -v` 分别测 one-shot（`SELECT 1`）与 2000 行加载负载（grep `Maximum resident`）；③ 负载吞吐——加载负载 Elapsed（grep `Elapsed`）；失败处理第 2 条（单实例检查）与第 7 条（绝对路径）适用。
  - README 目标面：`README.md` CLI-level footprint 表（Load time / Peak RSS / Main database file after close / One-shot `SELECT 1` latency / Binary size 五行，:284-292）+ 读数 prose 一句（:294 「one-shot invocations carry the fixed cost of an async runtime and a checkpointing close」——T1 后只读 close 不再 checkpoint，该句需按最终状态改写）；`README.zh-CN.md` 镜像行（:289 峰值内存 RSS、:291 单次 `SELECT 1` 时延，及对应 prose）。引擎级三行（INSERT/PK lookup/Full scan）与 Main file/Binary size 行不受本 change 影响——原值不动。
- Code and Critical Path: T2 触碰 `src/main.rs` 单行（或零触碰——判据不满足时还原）；T3 触碰 `README.md` / `README.zh-CN.md` 对比板块；design.md D5 回填数据。`cargo test` 不经 `main.rs`，flavor 决策不影响测试基建。

**Implementation Guidance**

执行顺序：① 按 R26 重建 2000 行预建库（干净起跑，先 rm 旧库文件）；② multi_thread（现状代码）三组实测 ×3 次（首跑弃用，取中位）；③ 改 `src/main.rs` attribute 为 current_thread → release 构建 → 同口径三组实测 ×3（同一预建库、同机、背靠背、R26 失败处理第 2 条确认单实例）；④ 套用判据（RSS 取 one-shot 形态峰值——I063 问题主体；负载形态 RSS 一并记录为信息性；回归 ≤5% 对 one-shot 时延与负载 Elapsed 两项分别判定）：满足 → 保留切换并进 ⑤；不满足 → 还原 attribute 为 multi_thread（`git diff src/main.rs` 确认还原干净）再进 ⑤；⑤ design.md D5 回填两组数据与决策结论；⑥ 全量串行回归（M19）；⑦ T3：按最终状态重测受影响数字（one-shot 时延、Peak RSS、Load time——若 flavor 切换后与既有记录口径可比则直接采用 ③/② 数据，否则补测），刷新 README 双语五行中受影响行 + :294（及 zh-CN 镜像）prose，附测量日期与语境注记。

**Task Contracts**

### T2: CLI runtime flavor 实测定形（R2）

- Requirement/Scenario: R2——三组实测定形 + 判据执行 + 数据留档 design.md D5；无行为 spec（既有 CLI 行为 spec 场景与全量测试为切换后零回归见证）
- Depends on: Iteration 000（accepted——实测基线基于零工作跳过落地后的 close 路径）
- Targets: `src/main.rs:3`（flavor attribute，按判据保留或还原）、`design.md` D5（数据与结论回填）、Act Response（测量记录）
- Current behavior: `#[tokio::main]` multi_thread 默认；one-shot RSS 16.7 MiB 量级（2026-09-24 口径）；时延 5.85 ms（T1 后单轮参照）
- Required behavior: 两 flavor 各得三组实测（one-shot 时延 / one-shot+负载 RSS / 负载 Elapsed，各 ×3 中位）；判据（RSS 以 one-shot 形态为准 ≥15% 降，且 one-shot 时延与负载 Elapsed 回归均 ≤5%）满足 → 切 current_thread 并全量零回归；不满足 → 保持 multi_thread（attribute 还原，diff 干净）；数据与结论无论哪支都留档 D5
- Preserve: 判据数值本身（用户批准 2026-09-26，不调门槛）；CLI 全部既有行为（渲染/退出码/信号停机/事务会话）；除 main.rs attribute 外零产品代码改动； benches 与测试基建不动
- Forbidden: 不引入 Builder 定制（worker 数等）；不动 Server 面；不改 benches；判据边界模糊（差异在噪声带内）时按保守分支处理（保持 multi_thread）并记录，不得为凑切换放松口径
- Test witness: 测量即见证——每次测量的命令与决定性输出（≤20 行/项）入 Act Response；最终状态全量串行 `cargo test -- --test-threads=1` 退出码 0（1285/0/2）
- GREEN condition: D5 回填完成（两 flavor × 三组 × 中位 + 判据套用结论 + 决策）；最终代码状态（切换或保持）经全量串行零回归确认；若切换，`git diff src/main.rs` 恰为单行 attribute 且全部既有 CLI e2e 零校准通过
- Verification: R26 步骤 1b（criterion 不适用本任务）、4、5 的 CLI 级子集——50 循环时延、`/usr/bin/time -v` RSS/Elapsed；判定用测量值本身（无需额外判定器）；人工核对项（README 行数值一致）所见即判定
- Stop when: 测量环境不可信（并发实例污染、数据异常抖动超出 ×3 中位可控范围）；判据两臂在测量误差内无法区分且保守分支依据不足（返回 Plan，不得自行改判据）

### T3: README 双语对比板块刷新（R3）

- Requirement/Scenario: R3——受影响数字重测更新，双语一致；文档交付物（MS19 备份边界先例，无行为 spec）
- Depends on: T2（数字反映 flavor 决策后的最终状态）
- Targets: `README.md` CLI-level footprint 表受影响行 + :294 读数 prose；`README.zh-CN.md` 镜像行 + 对应 prose
- Current behavior: Peak RSS ~16.7 MiB、One-shot `SELECT 1` 10.8 ms（2026-09-24 采集）；prose 归因「async runtime and a checkpointing close」——T1 后只读 close 的 checkpoint 开销已消失
- Required behavior: 受影响行刷为 change 后实测值（One-shot latency、Peak RSS、Load time——Load time 仅在 flavor 切换且与既有口径可比时更新，否则注明采集日期保持原值并说明）；prose 按最终状态改写（checkpointing close 归因句）；两文件逐项一致；附测量日期（2026-09-26）与语境注记（close 零工作跳过 + flavor 状态）；引擎级三行、Main file after close、Binary size 行原值不动
- Preserve: 对比表结构与列（Metric | RTsql | sqlite3）；「单机观察」限定与 re-run 提示 prose；备份边界段（R30）与其余一切 README 段落零触碰；sqlite3 对比列数字不动
- Forbidden: 不更新引擎级 criterion 数字；不新增/删除表格行与章节；不改对比板块以外的任何 README 内容；不为未重测的数字虚构新值
- Test witness: 无自动化测试（文档面）——见证 = `git diff README.md README.zh-CN.md` 仅落在受影响行 + prose + 注记；测量命令与输出入 Act Response
- GREEN condition: 双语 diff 一致且范围限于对比板块；受影响行数值与 D5/Act 记录的最终测量值一致；人工复核（所见即判定：行值更新、双语一致、引擎行原样）通过
- Verification: `git diff` 人工核对（最短步骤：逐行比对两文件受影响区）；若 Load time 行因口径问题保持原值，核对注记存在且表述准确
- Stop when: 最终测量值与 README 现有口径不可比（如负载行数/口径漂移）且注记方案仍无法诚实表述（返回 Plan 裁定表述方式）

**Invariants**

- flavor 决策严格按用户批准判据执行，不因实施便利调整门槛；保守分支（保持现状）是判据不满足时的合法且预期结果。
- T1 交付面（checkpoint 零工作守卫）在本 Iteration 零触碰。
- 全量验证始终 `cargo test -- --test-threads=1`（M19）。
- README 引擎级数字与 sqlite3 列原样保留——本 change 未触碰引擎执行路径。

**Non-goals**

- 引擎级 criterion 重测、分配器、流式化、WAL fsync 合并（MS22 域）。
- Server 面 runtime、REPL、`install.sh`。
- README 对比板块以外文档（备份边界段、安装段等）。

**Acceptance**

- R2：两 flavor × 三组实测数据齐备且留档 D5；判据套用结论明确（切换或保持）；最终状态全量串行 1285+/0/2 零回归；若切换，既有 CLI e2e 零校准。
- R3：README 双语对比板块受影响行 = 最终实测值，prose 与注记诚实反映 change 后状态，双语一致，范围限于对比板块。
- Scenario→Task→Test 链路：RTM R2/R3 行（测量记录 + diff 核对，无 spec 场景）。

**Verification**

- T2-V1 三组实测执行：两 flavor 各 50 循环时延 + `/usr/bin/time -v` RSS（one-shot/负载）+ 负载 Elapsed，×3 取中位；输出入 Act Response。
- T2-V2 判据套用：中位数代入批准判据，结论唯一（切换/保持），留档 D5。
- T2-V3 最终状态回归：`cargo test -- --test-threads=1` 退出码 0。
- T3-V1 README diff：`git diff README.md README.zh-CN.md` 范围与一致性人工核对（所见即判定）。
- 判定均为测量值自身、cargo 退出码与 diff 直读，无身份型证据工程。

**Gate 2 Readiness**

PASS——R2/R3 为 proposal 级 requirement（无 Missing；RTM Covered）；判据与口径已随 change 计划获用户批准（2026-09-26，D5/D6）；测量面（R26 runbook、main.rs 单行切换、README 目标行）全部实证定位；任务契约自包含；验证直接判定；无实质未知项（current_thread 行为风险已由 D5 分析覆盖并以实测与全量回归兜底）。

**Persisted Evidence**

- Mode: none

测量命令可在任意 checkout 低成本重跑（R26 全程只读测量、产物落 /tmp 与 target/）；记录以 Act Response ≤20 行决定性输出承载。

**Risks and Notes**

- 测量噪声：×3 取中位 + 首跑弃用为噪声对策；同机背靠背（两 flavor 交替窗口尽量接近）；R26 失败处理第 2 条（单实例确认）执行。
- current_thread 下多语句负载吞吐可能劣化（解析+执行+I/O 串行化）——判据的吞吐臂直接覆盖，劣化超 5% 即保持现状。
- Load time 行的口径可比性：若 flavor 切换改变 Elapsed，新值与旧值（3.2 s，2026-09-24）采集条件相同（2000 行、同机），可直接更新；若 T2 判据保持 multi_thread，Load time 不变、行值不动（仅必要时注记 T1 不影响写入路径）。
- README :294 prose 的改写幅度保持最小——只修正「checkpointing close」归因与（若切换）runtime 表述，不重写段落。
- 非实质选择留给 Act：循环脚本形态（shell for 循环按 R26 原文）、注记具体措辞、tempdir 路径。

## Act Response

- Status: reported

**Implemented**

T2 + T3 全量交付。**最终代码状态 = 保持 multi_thread（判据保守分支），产品代码零变更**（`git diff src/main.rs` 为空）；本轮变更面全部为文档与测量记录：

- T2（R2）：① 按 R26 重建 2000 行预建库（`/tmp/workload.sql` 102,768B，先 `rm` 旧库三件套；setup 加载确认 `SELECT COUNT(*)` = 2000）；② multi_thread（现状 release 构建）三组实测 ×3；③ `src/main.rs:3` attribute 临时切换 `#[tokio::main(flavor = "current_thread")]` → release 重建 → 同口径 ×3；④ 判据套用——RSS 臂不满足（见 Verification 3）→ 按用户批准判据保守分支**保持 multi_thread**，attribute 还原并重建；⑤ `design.md` D5 回填两组数据、判定值与决策结论（含 I063 假设证伪说明）。
- T3（R3）：`README.md` / `README.zh-CN.md` 对比板块——Peak RSS 行 `~16.7 → ~16.4 MiB`、One-shot latency 行 `10.8 → 6.5 ms`（均为最终态实测值）；表格后新增注记一句（2026-09-26 重测 + 零工作关闭跳过语境 + flavor 评估保留默认多线程 + Load time 行保留 2026-09-24 值及原因）；「解读 / Reading the numbers」prose 最小改写——删除「关闭时 checkpoint 的固定开销」归因、改为「只读关闭不再叠加 checkpoint 开销（零工作关闭整体跳过）」。Load time、Main file after close、Binary size 三行与引擎级三行、sqlite3 对比列零触碰。

**Deviations from Plan**

1. **「×3 首跑弃用取中位」的判定值计算口径**：计划未写明弃用首跑后剩两值时中位的取法；本轮按「后两次的中位 = 两值均值」计算（如 mt 时延 (6.56+6.34)/2 = 6.45），三次原始值全部留档 D5 可复核。非实质测量解释差异。
2. **T2-④ 无代码变更与计划一致，但 main.rs 的中间状态**：attribute 切换 → 还原在 git diff 上自净（还原后 `git diff src/main.rs` 0 行）。计划预期分支（「保持 = 无代码变更」），非偏差，记录最终态以免误读。
3. **README Peak RSS 行取负载形态 RSS（16,810 KB ≈ 16.4 MiB）而非判据 RSS**：计划 T3 所指 Peak RSS 行语境即负载进程（CLI 表格引言「loading 2,000 rows plus SELECT COUNT(*)」），与判据的 one-shot 形态 RSS（判据专用）是两个口径；两口径数据均在 D5 留档，行值按行语境取负载形态。非实质口径澄清。

**Blocker Handoff**

None.

**Blocker Resolution**

None.

**Self-Review**

Gate 3（spec compliance）：

- R2：两 flavor × 三组实测齐备（原始 3 值/组全留档 D5）；判据按用户批准数值执行——RSS 臂（one-shot 形态）+2.1% 不满足 ≥15% 降 → 保持 multi_thread（契约明文「保守分支是判据不满足时的合法且预期结果」，未放松口径）；数据与结论回填 design.md D5；最终态全量串行零回归（1285/0/2）。若切换分支的 CLI e2e 零校准条款因未切换而不适用。
- R3：双语 diff 仅落在受影响两行 + 注记 + prose（git diff 人工核对，见 Verification 6）；受影响行数值与 D5/本轮实测一致（6.45→6.5 ms、16,810 KB→16.4 MiB）；Load time 行按计划保持原值且注记说明；引擎级三行、Main file、Binary size、sqlite3 列、表格结构、对比板块以外全部内容零触碰；附测量日期 2026-09-26 与语境注记（零工作关闭跳过 + flavor 状态）。
- Preserve/Forbidden：判据数值未调整；CLI 全部既有行为未触碰（产品代码零变更）；无 Builder 定制；Server 面/benches/测试基建未动；T1 交付面零触碰（`src/wal/checkpoint.rs`、`src/storage/buffer_pool.rs` 本轮零 diff）；README 备份边界段与其余段落零触碰；无身份型证据机制（`/usr/bin/time -v`、`date +%s%N` 原生输出直读，无判定器）。

Gate 3（code quality）：最终 diff 无计划外修改（文档三处：两 README + design.md D5 + 本 Response）；无产品代码、无新增警告面、无死代码问题；测量未因错误原因通过（每组原始三值留档可复核，弃首跑 + 取中位按计划执行）；命名与措辞贴合既有 README 风格。

自检发现并已处理：无 Critical、无 Important。一处测量观察如实记录——multi_thread 负载 Elapsed 三值离散较大（5.00/3.61/4.01 s，R26 历史 3.21 s），属单机噪声；判定用中位缓解，且该值不进入 README（Load time 行保持原值），不影响任何结论。

**Verification Evidence**

1. multi_thread（现状，最终态）三组实测（命令：R26 步骤 5 的 50 循环 + `/usr/bin/time -v`；同机背靠背，×3 首跑弃用）：
```
[mt] A 时延: 6.54*/6.56/6.34 ms/run        → 判定 6.45
[mt] B one-shot RSS: 7080*/6560/6584 KB   → 判定 6572
[mt] C 负载: Elapsed 5.00*/3.61/4.01 s → 判定 3.81；RSS 16824*/16804/16816 KB → 16810
```
2. current_thread（attribute 切换 + release 重建 47.22s）同口径：
```
[ct] A 时延: 2.02*/2.07/1.99 ms/run       → 判定 2.03
[ct] B one-shot RSS: 6584*/6584/6840 KB   → 判定 6712
[ct] C 负载: Elapsed 6.43*/4.35/3.39 s → 判定 3.87；RSS 16312*/16248/16548 KB → 16398
```
3. 判据套用（批准判据：one-shot RSS 降 ≥15% **且** 时延/负载回归 ≤5% → 切换）：RSS 6712 vs 6572 KB = **+2.1%，FAIL**；时延 -68%（满足）；负载 +1.6%（满足）。合取不成立 → **保持 multi_thread**（`git diff src/main.rs` 还原后 0 行，退出码 0；最终态 release 重建完成）。
4. 全量串行（`cargo test -- --test-threads=1`，M19 纪律，最终态），退出码 0：
```
passed=1285 failed=0 ignored=2
```
与 Iteration 000 accepted 基线一致（本 Iteration 产品代码零变更，1285 为零回归见证）。
5. 单实例与工具前置（R26 失败处理 2）：`ps` 检查无残留 rtsql/测量实例；`/usr/bin/time`、python3 就绪；workload 102,768B ≤ 105KB 安全线（无 E2BIG）；预建库 `SELECT COUNT(*)` = 2000。
6. T3-V1 README diff（`git diff README.md README.zh-CN.md` 人工核对，所见即判定）：每文件恰 2 行数值替换（Peak RSS 16.7→16.4 MiB、One-shot latency 10.8→6.5 ms）+ 1 句新增注记 + prose 归因改写；双语逐项一致；Load time/Main file/Binary/引擎级/sqlite3 列原样。
7. OpenSpec：`openspec validate 2026-09-26-ms18-oneshot-fixed-cost --strict` → `Change ... is valid`，退出码 0。

change 结构自检：`tasks.md` 与 Iteration 000 时相同——Task List 为无状态字段的表格（非 checkbox 清单），按精准修改纪律 Act 不新增状态字段，T2/T3 交付状态由本 Response 与 RTM 承载；R2/R3 为 proposal 级（无行为 spec），delta spec `ms07-rest-tx-checkpoint-pushdown` 本 Iteration 零触碰；design D5 已回填、D6 已按最终态执行；`Review Result: pending` 与流程状态一致。

**Persisted Evidence**

None required —— 计划设定 `Mode: none`。全部测量为 R26 只读流程（产物落 /tmp 与 target/，可低成本重跑），记录以本 Response ≤20 行决定性输出承载；flavor 切换为可逆单行变更且已还原，无一次性环境、无不可复现结论。

**Experience Candidates**

- 无新增 Runbook 候选：本轮测量全程按既有 R26 runbook 执行，无新操作路径（Iteration 000 已登记的「git stash 对照基线」候选仍待用户裁定）。
- Issue 候选：无。未发现 change 范围外实质缺陷。
- 供用户裁定的事实（非候选）：I063 的「worker 线程是 one-shot RSS 大头」假设被实测证伪（两 flavor one-shot RSS ≈6.5 MB 几乎相同），而 current_thread 的 one-shot 时延收益显著（6.45→2.03 ms，-68%）——数据已留档 design.md D5；若需按此更新 improvements I063 或调整方向，属 Maintainer/用户决策，本 Act 不落账。

**Remaining Issues**

1. Minor（范围外发现，留档不处理）：current_thread 的 one-shot 时延收益（-68%）因批准判据 RSS 臂不满足而未采纳——属判据主锚（RSS）与实测收益（时延）错位；将来若以时延为主判据重启 I063 评估，数据在 D5。本 Cycle 按契约执行保守分支，不改判据、不扩范围。
2. Minor（测量基建）：负载 Elapsed 单机噪声较大（3.39–6.43 s 跨 3 值），5% 回归阈值在该噪声带下区分度有限；本轮结论（+1.6%，且 RSS 臂独立否决）不受影响。将来 flavor/性能对比可考虑增加重复次数或固定负载口径。
3. 无 Critical、无 Important。Gate 5 未触发（无 Blocker，无三次失败）。

## Plan Review

- Review Result: accepted

**Findings**

独立检查与 Act 声明逐项一致，未发现 Plan 遗漏、Plan 错误、Act 偏离或基线变化。逐项核实：

- **最终代码态 = 保持 multi_thread（保守分支）**：`git diff --stat src/main.rs` 为空、当前内容 `#[tokio::main]`；T1 交付面零触碰（checkpoint.rs/buffer_pool.rs/database.rs 合计 110 insertions/1 deletion，与 Iteration 000 accepted 时逐行审阅的状态一致）；`tests/checkpoint_skip_test.rs` 349 行未变；无新增 untracked 文件。
- **D5 回填与 Act Response 数据完全一致**，判定算术复核全对：mt 时延 (6.56+6.34)/2 = 6.45 ms、ct (2.07+1.99)/2 = 2.03 ms（-68%）；RSS (6560+6584)/2 = 6572 vs (6584+6840)/2 = 6712 KB（+2.1% < 15% 降 → FAIL）；负载 (3.61+4.01)/2 = 3.81 vs (4.35+3.39)/2 = 3.87 s（+1.6% ≤ 5%）；合取不成立 → 保持。判据数值未调整，保守分支按契约执行。
- **README 双语 diff 有界且一致**（逐行阅读）：每文件恰 2 行数值替换（Peak RSS 16.7→16.4 MiB、One-shot latency 10.8→6.5 ms）+ 1 句注记 + 读数 prose 最小改写；Load time/Main file/Binary size/引擎级三行/sqlite3 列/表格结构/对比板块以外内容零触碰；双语逐项一致；注记与 prose 表述准确（2026-09-26 重测、零工作关闭跳过语境、flavor 评估保留默认多线程、Load time 保留原因）。
- **Deviation 3（Peak RSS 行取负载形态 16.4 MiB）核实为行语境正确选择**：该行测量基准本就是负载进程（表格引言「loading 2,000 rows plus SELECT COUNT(*)」，旧值 16.7 MiB 亦来自 R26 负载测量 17124 KB），新值 16,810 KB ≈ 16.4 MiB 同基准可比；判据用的 one-shot 形态 RSS（6572/6712 KB）单独留档 D5，两口径未混淆。时延行同理（50 循环 one-shot 口径前后一致）。Deviation 1（两值中位 = 均值）计算口径已注明且三原始值全留档可复核。

Minor finding（不阻塞）：

1. **判据主锚与实测收益错位（Act Remaining Issue 1，非缺陷）**：current_thread 的 one-shot 时延收益 -68%（6.45→2.03 ms）因批准判据 RSS 臂不满足（+2.1%）未采纳。Act 按契约执行保守分支正确（未放松口径、未扩范围）。**这是用户决策点而非返工项**：若以时延为主判据重启 I063 评估，数据已在 D5 留档；I063 的「worker 线程是 RSS 大头」假设被实测证伪，其台账更新属 Maintainer/用户决策。
2. **负载 Elapsed 噪声带（Act Remaining Issue 2）**：单值跨 3.39–6.43 s，5% 阈值区分度有限；本轮结论不受影响（RSS 臂独立否决）。留档备将来测量设计参考。

**Acceptance Gaps**

无。R2：两 flavor × 三组实测齐备且 D5 留档、判据按批准值执行（结论：保持 multi_thread）、最终态全量串行 1285/0/2 零回归（产品代码零变更，与 Iteration 000 accepted 基线同值）。R3：README 双语受影响行 = 最终实测值、注记与 prose 诚实反映 change 后状态、双语一致、范围限于对比板块。

**Evidence**

- 采信 Act Response Verification Evidence 1–7（本会话新鲜运行，覆盖未失效）：两 flavor 三组实测原始值、判据套用、全量串行 1285/0/2、单实例前置检查、`openspec validate --strict` 通过。
- Plan 独立补充：`git diff src/main.rs` 为空 + 当前文件内容直读；README 双语 diff 逐行阅读；design.md D5 回填段全文阅读与算术复核；T1 交付面 diff --stat 与 Iteration 000 审阅状态一致（110/1）；`tests/checkpoint_skip_test.rs` 行数 349 未变。

**Follow-up Decision**

接受（accepted）：Acceptance 全部满足，无阻塞 finding。Minor 1 为用户决策点（I063 后续方向），不构成本 change 返工项；Minor 2 留档。Iteration Map（000/001）全部完成——无剩余 Iteration。

**Iteration Plan Update**

None。

**Next Cycle**

None。

**Next Iteration**

None——Iteration Map 已全部 accepted，change 两 Iteration 均交付；后续为收尾（规格 merge、SNAPSHOT/tasks 同步、归档），属 `openspec-docs-maintainer` 职责，待用户调用。
