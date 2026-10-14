# Iteration 000 / Cycle 000: e2e 旅程验证面

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1
- Acceptance gaps: None
- Repair items: None
- Inherited scope: R1（e2e 旅程验证面——proposal 需求基线）；用户决策 D-1（P2-10/11 锁定）、D-3（新缺陷处置）
- Excluded scope: ISS05–ISS14 全部产品代码修复（Iteration 001–004）；性能断言；验证判定层/身份证据机制（AGENTS 禁令）

**Objective**

`tests/e2e_test.rs` 成为旅程级端到端验证套件：真实 CLI 入口（子进程 + 磁盘 + stdin）、断言用户实际所见（退出码/stdout/stderr/磁盘状态）、覆盖 E1–E13 十三旅程；已知缺陷交互点以 `#[ignore = "ISSxx 待 Tn 修复"]` 标记；非缺陷面全绿并纳入稳定基线。

**Investigation Facts**

- Current Baseline: HEAD `44d04d9`，全量 1929 tests / 0 failures / 2 ignored（2026-10-06 采信，覆盖未变化可直接引用）；`tests/e2e_test.rs` 现为 6 行注释空壳（指向 pg_integration/pipeline_test），填充无迁移成本；本 Iteration 零产品代码改动。
- Current-State Evidence（夹具先例，Act 直接采用、不必回读 cli_test.rs 即可起步；细节以 `tests/cli_test.rs:1-130` 为准）：
  - 二进制路径 `env!("CARGO_BIN_EXE_rtsql")`；每次 spawn 以独立 TempDir 为 CWD 并 `env("RTSQL_HOME", dir)` + `env_remove("RTSQL_KEY")`；stdout/stderr 管道（非 TTY → 默认 JSON 格式）。
  - `spawn_cli(dir, args) -> Child`（不等待，信号/持锁用例用）；`spawn_cli_stdin(dir, args)`（stdin 管道，restore/import `-` 用例）；`run_cli(dir, args) -> CliOutput{code,stdout,stderr}`；`wait_cli` 以 60s deadline 轮询 try_wait、超时 kill+panic（one-shot 必须自退）；`fixture() -> TempDir` 预建 `db/` 子目录（CLI 不建目录）。
  - 子进程写 stdin 后 `drop(stdin)` 关写端使 read_to_string 见 EOF；子进程先退出时写端 EPIPE 属预期不视为夹具错误。
  - R31 runbook（`.agents/runbooks/cli-subprocess-witnessing.md`）约束：SQL 负载按 `MAX_ARG_STRLEN` 128 KiB 定尺、stdout 必须被消费（防管道阻塞）。
  - 锁/并发旅程形态参照 `tests/busy_timeout_test.rs` 与 `tests/database_file_lock_test.rs`（长持有进程 = stdin 管道喂慢输入的 import/restore，或待命写会话）；信号用例向存活进程 `kill(SIGKILL)`（`std::process::Child::kill`）。
- Code and Critical Path: 无产品代码路径；本 Cycle 全部改动在 `tests/e2e_test.rs`。已知缺陷交互点（→ ignore 标记映射）：E3b→ISS06/T3；E4 等值计数与全表 UPDATE affected→ISS05 与 B2/T2；E13 DEFAULT 超限文案→ISS14/T3、建表失败残留→ISS08/T3；E11 派生/视图别名与表达式→ISS07/T4、SemiJoin ORDER BY→ISS09/T5；E12 help 标记→ISS10/T6、前缀单层→ISS11/T6、类型名→ISS12/T6、env 非法值→ISS13/T6。P2-10/11（多语句 JSONL、DDL affected_rows:0）按 D-1 直接断言现状（无 ignore）。
- 行为基线依据：各旅程断言值来自语料域 Requirement（`cli-noninteractive-shell` 生命周期/格式/SQLSTATE、`database-encryption`、`readonly-open-mode`、`in-memory-database`、`sql-attach-session`/`cross-db-*`、`overflow-page-storage`、`cli-analytics-commands`、`wal-recovery-replay-integrity` 既有恢复语义等）与 R32 分析的缺陷形态记录（pre-fix RED 形态即 ignore 标记点的失败边界）。

**Implementation Guidance**

- 建议顺序：夹具 → E1（全生命周期，最广）→ E2/E8/E9/E10（独立通道）→ E5/E6/E7（进程交互/加密——加密旅程命令数刻意少：Argon2id 每次开库派生密钥，控制时长）→ E3/E3b/E4（崩溃与索引，pre-fix 全 ignore 但对照断言绿）→ E11/E12/E13（读面与文案，部分 ignore）。
- 崩溃旅程定尺：E3b 用 stdin 喂 2000–10000 行 CSV 的 import（debug 逐行 auto-commit ≈4ms/行），启动后 1–2s kill——确保表已建、≥1 行已提交、未 checkpoint；对照 E3 用已存在表 + 中途 kill（对照面本就绿）。
- 大值旅程字符串在测试内代码构造（不依赖 fixture 文件）；`MAX_ARG_STRLEN` 限内经 stdin/dump 文件承载大 SQL。
- ignore 标记统一格式 `#[ignore = "ISSxx 待 T<n> 修复"]`，与 tasks.md 映射一致——后续任务的验收入口即「解除 ignore + 观察 RED」。
- 新暴露缺陷按 D-3 分流：同面小缺陷顺带修（Act Response 记录）、需新任务/设计报 Plan、其余 Issue 候选只报告。

**Task Contracts**

### T1: e2e 旅程套件建成且非缺陷面全绿

- Requirement/Scenario: R1（E1–E13，proposal 场景草图全表）；D-1（E10 锁定）
- Depends on: None
- Targets: `tests/e2e_test.rs`（新内容；现为空壳）
- Current behavior: 空壳；无旅程级验证面。
- Required behavior: ①夹具（上述 Investigation Facts 形态）；②E1 全生命周期（new → 建表[约束+DEFAULT+重复列名拒绝] → 写矩阵[子集 INSERT/UPSERT 三形态/UPDATE 表达式多列/全表 UPDATE+DELETE/带 WHERE] → 读矩阵[JOIN 三型/聚合/CTE/视图/集合/DISTINCT/别名排序/派生表/IN 子查询] → checkpoint → dump → restore 新库 → 数据逐行对照 → delete 逐文件报告）；③E2 导入（文件+stdin `-`+自动建表+`:memory:`+表头冲突拒绝）；④E3 已存在表崩溃恢复对照（kill → 重开 → count 一致 → check 干净）+ E3b 自动建表即崩溃（pre-fix 砖化，整段 ignore）；⑤E4 索引（≥30000 行重复键 + CREATE INDEX → 等值 COUNT/DISTINCT vs 建索引前基线[ignore]、无 WHERE UPDATE affected[ignore]、重开一致、DROP INDEX 恢复）；⑥E5 并发（写持锁 + 短预算等待/超时 exit 4 + 只读并发 + delete 占用拒绝）；⑦E6 跨库（ATTACH→跨查→跨写→CTAS→单库独立）；⑧E7 加密（--key 全流程+错钥 exit 5+check+dump/restore 带钥）；⑨E8 大值（~1MB 写读+dump/restore+check）；⑩E9 分析（stats/sample/profile × 四格式）；⑪E10 输出锁定（多语句 JSONL、DDL affected_rows:0、四格式渲染——直接断言现状）；⑫E11 派生/视图读面（内层别名/表达式三通道[派生+视图 ignore、CTE 绿] + SemiJoin ORDER BY[ignore]）；⑬E12 文案（help[ignore]、前缀[ignore]、类型名[ignore]、env[ignore]）；⑭E13 错误面（未知列/表/不支持语句 exit 3+SQLSTATE+零副作用 + DEFAULT 超限[ignore] + 建表失败残留[ignore]）。
- Preserve: 全部 119 个既有集成目标与 lib 单测零改动；不新增夹具抽象层/判定层；断言面限于退出码/输出文本/磁盘状态。
- Forbidden: 修改任何 `src/` 产品代码；性能/时延断言；身份证据机制（hash/指纹/run-id 类）；依赖并行度或执行顺序的用例形态（M19：单用例自带独立 TempDir 现场）。
- First test: 旅程逐个落地——非缺陷点先观察实际输出再断言（变更前 GREEN 锁定形态）；缺陷交互点观察 pre-fix RED 形态后标 `#[ignore]`（见证记录进 Act Response）。命令：`cargo test --test e2e_test`。
- GREEN condition: 单目标全绿（ignore 项除外）；每个 ignore 标注与 ISSxx/Tn 映射一致。
- Verification: `cargo test --test e2e_test -- --test-threads=1`（M19 允许单目标任意并行度，串行取稳定）；`cargo test --test cli_test -- --test-threads=1` 与 `cargo test --test check_command_test -- --test-threads=1` 抽样回归（零产品改动，确认无环境干扰）；套件时长记录（超分钟级需在 Act Response 报告并评估基线成本）。
- Stop when: 发现需新设计/新任务的缺陷（报 Plan 转用户裁定）；发现与 R32 记录不一致的缺陷形态（记录并按 D-3 分流）；夹具无法以现有模式表达某旅程（如需要网络层——本项目无 serve 面，不应发生）。

**Invariants**

- AGENTS 验证纪律：端到端断言用户实际所见；不建判定层；不为证据新增工具。
- M19：全量验证分批纪律（本 Iteration 不跑全量）；单用例独立现场。
- 既有测试与语料零改动（本 Iteration 无行为变化）。

**Non-goals**

ISS05–ISS14 修复（Iteration 001–004）；e2e 套件对网络协议面的覆盖（无该面）；文档（T7）。

**Acceptance**

- R1 Covered：E1–E13 全部旅程落地，非缺陷断言绿、缺陷点 ignore 映射正确（R1 → T1 → `tests/e2e_test.rs` 各旅程测试函数）。
- RTM（change 级）：

| Requirement | Task | First Test | Status |
|---|---|---|---|
| R1 e2e 旅程验证面 | T1 | `tests/e2e_test.rs` E1–E13 旅程函数 | Covered |
| R2 索引与树结构正确性 | T2 | `secondary_index_equality_test` S1–S6 + e2e E4 解除 ignore | Covered（Iteration 001） |
| R3 DDL 持久化与原子性 | T3 | 崩溃恢复矩阵 S7–S9 + e2e E3b/E13 解除 ignore | Covered（Iteration 002） |
| R4 派生表/视图内层列 | T4 | derived_scan_alias/view 扩展 S10–S11 + e2e E11 解除 ignore | Covered（Iteration 003） |
| R5 SemiJoin 排序键 | T5 | subquery_test 扩展 S12 + e2e E11 解除 ignore | Covered（Iteration 003） |
| R6 CLI 文案四点 | T6 | cli_test 扩展 S13 + e2e E12 解除 ignore | Covered（Iteration 004） |
| R7 文档同步 | T7 | 三文档逐条人工核对 | Covered（Iteration 004） |

**Verification**

E1–E13 为行为类 scenario 且全部跨特性/经外部接口——一律真实入口（子进程 CLI）；断言即判定（退出码/输出/磁盘状态直接断言，无判定层）。单目标运行 + 两套件抽样回归如 Task Contract 所列。

**Gate 2 Readiness**

PASS——契约自包含（夹具形态、旅程清单、ignore 映射、验收与停止条件齐备）；无 TBD；Persisted Evidence = none（Act Response 承载）；用户批准计划后本文件转 ready。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- 套件时长：崩溃/加密/导入旅程偏慢（kill 等待 + Argon2id 派生），若单目标超长（>3–5 分钟）Act Response 报告实际时长并评估是否拆分慢速旅程为独立目标（属验证基建编排，非行为变化，Act 可直接决定并记录）。
- 新缺陷暴露的不确定性（D-3 已定分流原则）；pre-fix RED 形态以 R32 + 本会话探针记录为准，形态不符时按 D-3 记录分流而非硬造 ignore。
- WSL2 下 flock/信号时序属已知环境面（R31 既有配方已消化），不新增特殊处理。

## Act Response

- Status: reported

**实际改动**

- `tests/e2e_test.rs`（唯一改动面，+1356/-7；空壳注释整体替换）：夹具（cli_test/R31 模式：`CARGO_BIN_EXE_rtsql` + TempDir/RTSQL_HOME + 60s 兜底）+ E1–E13 十三旅程共 22 个测试函数。
- 旅程落地：E1 全生命周期（new→建表[NOT NULL/DEFAULT/重复列名拒绝]→写矩阵[子集 INSERT/UPSERT 三形态/UPDATE 表达式多列/全表 UPDATE+DELETE/带 WHERE]→读矩阵[JOIN 三型/聚合/CTE/视图/集合/DISTINCT/别名排序/派生表/IN 子查询]→checkpoint→dump→restore→逐行对照→delete）；E2 导入（文件/stdin `-`/自动建表/`:memory:`/表头冲突零副作用）；E3 崩溃对照（中途 SIGKILL→恢复区间+行唯一+重开幂等+check 单行+收敛零工作）；E3b 自动建表即崩溃（ignore）；E4 索引（绿面基线/CREATE INDEX/DROP INDEX 回落 + ignore 正确性面）；E5 并发锁（预算内等待成功/超时 exit 4/只读冲突 exit 4/delete 占用拒绝）；E6 跨库（ATTACH/跨查/跨写/CTAS/两库独立）；E7 加密（全流程/错钥 exit 5/明文不泄/check/dump 带钥往返/env 通道）；E8 大值（1MB 经 restore stdin 写读逐字节/REPLACE 增长 2MB/dump 往返/check 单行）；E9 分析（stats/sample/profile × 四格式）；E10 输出锁定（多语句 JSONL 四行/DDL `affected_rows:0`/四格式形态，D-1）；E11 读面（CTE 绿对照 + ISS07/ISS09 ignore 面）；E12 文案（错误面 SQLSTATE 绿 + ISS10–13 ignore 面）；E13 错误面（未知列/表/不支持语句 exit 3+SQLSTATE+零副作用绿 + ISS14/ISS08 ignore 面）。
- ignore 标记 10 项，映射与 tasks.md 一致：`ISS06 待 T3 修复`（E3b）、`ISS05 待 T2 修复`（E4 正确性面）、`ISS07 待 T4 修复`/`ISS09 待 T5 修复`（E11）、`ISS10–13 待 T6 修复`（E12 ×4）、`ISS14 待 T3 修复`/`ISS08 待 T3 修复`（E13）。

**Pre-fix RED 见证（10/10，全部经 `--ignored` 实跑确认）**

- E3b：exit 1 `WAL redo failed: table 'autot' referenced by committed WAL records is missing from the catalog`（与 ISS06 台账机制段逐字吻合）。
- E4：等值 COUNT 经索引 235 ≠ DataScan 基线 150（虚发形态；10000 行规模阈值实测不触发、15000 触发，套件取 15000 承载 ignore 面、30000 承载绿面）。
- E11：派生/视图内层别名 42703 `Column not found`；SemiJoin ORDER BY 42703。
- E12：`--help` 含 `MS28 T1：`/`MS26 T1：`/`proposal 假设 10`；`Parse error: Parse error:` 双层前缀；dump `INT/STRING` 短名；env 非法值报错只提 flag。
- E13：`Page full` 不点名 DEFAULT；失败后 schema 可见半表 `CREATE TABLE "t" ("id" INT PRIMARY KEY)`。

**夹具修复（自检发现 → 已修复）**

- `wait_cli` 管道死锁：子进程输出 >64KB 管道缓冲（E8 读回 ~1MB JSON）而父进程退出前不读 → 死锁 60s 超时。修复：stdout/stderr 独立读线程并发消费（R31「stdout 必须被消费」约束的夹具级落地）。修复后 E8 全绿。
- `e13_oversized_default_named_rejection` 假阳性：pre-fix 报错的消息体虽不点名 DEFAULT，但语句回显段含 `DEFAULT` 字样致断言误过。修复：按 `; statement:` 分割取语义消息段断言。修复后 pre-fix RED 成立。
- `e12_busy_timeout_env_named`：exit 2 为 clap 纯文本 stderr（非 JSON 载荷），原断言对 stderr 做 JSON 解析。修复：原始文本断言。

**断言校准（观察实际输出后锁定，非产品缺陷）**

- E1：`ON CONFLICT DO UPDATE SET name = excluded.name` 只赋值名列（age 保持 18）；派生表 `age > 26` 此时两命中；one-shot 写入经 close 收敛 → checkpoint 恒报 `no pending work`（`wal truncated` 截断面由 E3 崩溃现场承载）；restore 要求空库前置需先 `new`。
- E8：`||` 未支持（0A000 点名），增长通道改用已注册 `REPLACE`（1MB→2MB）。
- E6：ATTACH 多语句输出为 JSONL，结果断言取末行（新增 `last_json` 断言便利）。
- E3：SIGKILL 终止 `status.code()` 为 `None`（Unix 信号终止无退出码）。
- E4 结构调整：Rust 无块级 ignore → 拆绿面/正确性面两个测试，各自自带现场（M19）。

**与计划的偏差**：无实质偏离。计划未预见者均为夹具/断言层问题，已在契约内处理并记录。

**Gate 自检**

- Gate 3：spec 合规（R1 全旅程落地、ignore 映射正确、D-1 锁定断言就位、无 src/ 改动、无性能断言、无判定层/身份证据）；代码质量（diff 仅 `tests/e2e_test.rs`、零编译警告、测试不因错误原因通过——两处假阳性已修复并复验 RED）。
- Gate 4：契约验证全部真实运行通过（摘录见下）。

**验证命令与实际结果**

- `cargo test --test e2e_test -- --test-threads=1` → 12 passed / 0 failed / 10 ignored（37.49s；套件时长在分钟级内，无需拆分慢速旅程）。
- `cargo test --test e2e_test -- --test-threads=1 --ignored` → 10/10 FAILED（pre-fix RED 见证，见上）。
- `cargo test --test check_command_test -- --test-threads=1` → 11 passed / 0 failed（2.78s）。
- `cargo test --test cli_test -- --test-threads=1` → 120 passed / 0 failed / 2 ignored（113.22s）。
- `git status --short -- tests/` → 仅 `M tests/e2e_test.rs`。

**Persisted Evidence**

- None required（模式 none；Act Response 承载全部见证；RED 形态均可在本 Cycle 解除 ignore 低成本复现）。

**Experience Candidates**

- None。（`wait_cli` 并发消费模式为 R31 既有约束的夹具级落地，不超出 R31 范围，不构成新 Runbook；本轮未暴露 change 范围外新缺陷——`||` 未支持为 0A000 既有拒绝面、import records 预收集为 MS29 文档化设计，均非缺陷。）

**未解决问题**

- 无。10 项 ignore 留待 Iteration 001–004 对应任务解除（解除即 RED→GREEN 验收入口）。

## Plan Review

- Review Result: accepted

**Findings**

独立检查：通读 `tests/e2e_test.rs` 全文（1357 行 / 22 测试函数）；`git diff --stat` 范围核对（代码面仅 `tests/e2e_test.rs` +1356/-7，其余为本 change 文档——契约「无 src/ 改动」成立）；`#[test]`=22、`#[ignore]`=10 与 Act 声明一致；ignore 标注 10 项与 tasks.md 映射逐条一致（ISS06/T3、ISS05/T2、ISS07/T4、ISS09/T5、ISS10–13/T6×4、ISS14/T3、ISS08/T3）；E1–E13 旅程与契约清单逐项对上；D-1 锁定断言就位（E10 四行 JSONL + DDL `affected_rows:0`、E1 建表行）；夹具为 R31/cli_test 模式（并发读线程消费 stdout/stderr，防 64KB 管道缓冲死锁——修复正确）；断言为实际输出直接断言（`json`/`last_json`/`assert_rows` 为解析便利非判定层）；每测试独立 TempDir（M19 合规）；无性能断言、无身份证据机制。Act 四组验证命令（e2e 12/0/10、`--ignored` 10/10 RED、check_command 11/0、cli_test 120/0/2）覆盖未变化，采信。

非阻塞 Minor（记录不返工）：

1. **E5 起跑竞态低概率偶发面**：`hold_lock` 线程与 CLI 子进程同时起步，理论上子进程先获锁则 `elapsed >= 800ms` 断言失败（无就绪屏障）。实测未触发；busy_timeout_test 同型先例。如复现，后续触碰时以就绪信号或失败重试一次加固。
2. **e12_help 标记清单严于 ISS10 点名范围**（额外含 `ISS`/`Iteration`/`T1：`）——对 T6 只增不减，无回归风险；T6 实施时全部用户可见 doc comment 须满足该更严集合。
3. **E4 正确性面规模阈值 15000**（Act 实测校准：10000 不触发）——数据确定性（`i%100+1`）使树形状确定、与机器无关，无 flake 面。

偏差分类（Act 偏离，均已记录且属契约内处理）：三处夹具修复（管道并发读、两处假阳性断言）与断言校准（E1 UPSERT 只赋值 SET 列、E3 `status.code()==None`、E6 JSONL 取末行、E8 `||` 0A000 既有拒绝面改 `REPLACE`、checkpoint 零工作=one-shot close 已收敛语义且 wal-truncated 面由 E3 承载）——逐条对照语料语义成立，无产品缺陷遮蔽。

**Acceptance Gaps**

无。R1/T1 验收满足：十三旅程落地、非缺陷面全绿、ignore 映射正确、D-1 锁定就位、抽样回归通过、套件纳入稳定基线（单目标 37.49s，分钟级内无需拆分）。

**Evidence**

`tests/e2e_test.rs` 全文读码；`git diff --stat` + `git status --short -- tests/`；`grep -c` 计数（22/10）；Act Response 四组验证命令与结果（采信，覆盖未变化）。

**Follow-up Decision**

Acceptance 全满足，发现均为非阻塞 Minor——**accepted**。按 Iteration Map 展开 Iteration 001（索引与树结构正确性 / T2）。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

`iterations/001-index-tree-correctness/000-initial.md`（已展开，Plan Context ready）
