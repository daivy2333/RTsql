# Iteration 000 / Cycle 000: 删库子命令与文件对备份边界

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1（= MS19-T01，含 I072 顺带项）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal R1（`cli-noninteractive-shell` ADDED「生命周期子命令 delete」8 场景）、design D1/D5、tasks.md Iteration 000 边界
- Excluded scope: 错误面改动（T2）、tx e2e（T3）、`install.sh` 语义、REPL、delete 以外的生命周期子命令行为变化

**Objective**

`rtsql delete <db>` 子命令全场景可用且既有命令零回归；文件对备份边界经实测并落 README 双语文档；实测结论与文档声明一致。

**Investigation Facts**

- Current Baseline: master @ bc1d18d，工作区干净，ahead of origin 3。1239 tests / 0 failures / 2 ignored（2026-09-26 全量新鲜，SNAPSHOT 仓库现场节；此后 Rust 代码零变化，采信为当前基线）。
- Current-State Evidence:
  - 子命令枚举 `Command`（`src/cli/mod.rs:89-155`）——新增变体落点；分派匹配 `execute_command`（`src/cli/mod.rs:197-233`）；生命周期实现惯例见 `new_db`（`src/cli/lifecycle.rs:16-44`，同步文件操作 + `execute_command_inner`）、`list`（`lifecycle.rs:47-100`，`emit_stdout` 输出行集）。
  - 存在性检查惯例 `resolve_existing_db`（`src/cli/lifecycle.rs:634-646`）：`resolve_db_path` + `exists()` → 不存在 `General("{path} does not exist")` exit 1。`resolve_db_path`（`src/cli/resolve.rs:30-35`）：含 `/` 直用，裸名 → `<base>/db/<name>.db`，不校验存在、不创建目录。
  - advisory 锁原语：`std::fs::File::try_lock()`（`src/storage/file_storage.rs:71-77`）——`TryLockError::WouldBlock → StorageError::DatabaseLocked`；锁对象是主库文件本身。CLI 锁冲突映射 `ExitStatus::Locked` exit 4、消息形状 `"database is locked: {path}"`（`src/cli/mod.rs:292-296`）。
  - 伴生文件命名：`db_path.with_extension("wal")`（`src/wal/writer.rs:27`）、`db_path.with_extension("checkpoint")`（`src/wal/checkpoint.rs:70`）——引擎唯一伴生文件族，无其他伴生。
  - 错误面 `ExitStatus`（`src/cli/mod.rs:27-63`）：Success/General(1)/Usage(2)/Sql(3)/Locked(4)/InvalidKey(5)；`emit_stderr`/`emit_stdout`（`mod.rs:527-539`）。
  - 测试夹具先例：`tests/cli_test.rs` 生命周期组与锁占用用例（锁冲突 exit 4，MS10-T02 落地）；`tests/tx_statement_test.rs:158` `run_cli`（`CARGO_BIN_EXE_rtsql` + TempDir CWD/RTSQL_HOME + 管道捕获）。
  - close 即 checkpoint（`src/database.rs:250-251`）——干净关闭后 WAL 截断，「只拷 `.db` 在 checkpoint 后等价」的机制依据；恢复按 WAL 重放（`src/wal/recovery.rs` 位置寻址 redo），DDL 无 WAL 记录（`src/cli/mod.rs:561` 注释）。
- Code and Critical Path: 本次仅触 CLI 应用层——`Command` 枚举 → `execute_command` 分派 → `lifecycle::delete_db`（新）→ 文件系统（锁探测 + remove_file + 报告）。不触 Database/存储/WAL 代码路径（`File::open`+`try_lock` 为标准库直用，不经 FileStorage）。

**Implementation Guidance**

按 design D1 执行，要点（细节以 D1 为准）：

- `delete_db` 为同步函数（不开库），`execute_command` 匹配臂直接调用。
- 顺序：定位 → 存在检查 → `File::open`（只读）+ `try_lock`（WouldBlock → `Locked` 零删除；Error → `General`）→ 持锁窗口内按 `.wal` → `.checkpoint` → 主文件顺序 `remove_file`（NotFound → `absent` 报告；其余错误 → 报告 + 整体 General）→ drop 释放锁。
- 报告逐文件走 `emit_stdout`：`deleted <path> (<n> bytes)` / `absent <path>`；dry-run 前缀 `would delete`，不探测锁不打开文件。
- `--dry-run` 用 `#[arg(long)]`；参数名 `db`（与 schema/dump 一致）。
- I072 实测按 design D5 矩阵；README 两语言各加一节，放在现有存储/持久化相关内容附近（位置非实质）。

**Task Contracts**

### T1: delete 子命令端到端可用 + 备份边界实测与文档化

- Requirement/Scenario: R1 全部 8 场景（`specs/cli-noninteractive-shell` ADDED delta）；I072 交付（实测矩阵 + README 双语，无 spec 场景）
- Depends on: None
- Targets: `src/cli/mod.rs::Command`（新变体 `Delete { db, dry_run }`）+ `execute_command` 匹配臂；`src/cli/lifecycle.rs::delete_db`（新函数）；`README.md` / `README.zh-CN.md`（备份与文件对段 + delete 用法行）；`tests/cli_test.rs`（delete 矩阵新用例）
- Current behavior: `rtsql delete` 不存在——`delete` 作为裸名走主命令（缺 `<sql>` 报 usage exit 2）；删库只能手工 `rm` 三件套。
- Required behavior: 见 R1 场景——裸名/路径删除并逐文件报告释放（exit 0）；`--dry-run` 只列出不删除；库打开中 `Locked` exit 4 零删除；库不存在 `General` exit 1 点名路径；伴生缺失跳过报告不算错误；头无效/零字节文件可删（不初始化不校验）；README 双语含备份边界声明（dump/restore 首选 + 等价条件 + 成对规则 + 与 purge-data 边界）且与实测矩阵一致。
- Preserve: 既有子命令（new/list/schema/dump/restore/import/stats/sample/profile/completions）与主命令行为、退出码、文案逐字节不变；锁消息形状与 `open_error_status` 一致（`database is locked: {path}`）；裸名与子命令冲突时子命令优先的既有语义。
- Forbidden: 不修改 Database/FileStorage/WAL/checkpoint 产品代码；不做库内容校验（不引入头解析）；不新增确认交互或 `--force`；不动错误面（T2 域）。
- Test witness: `tests/cli_test.rs` 新增 ≥7 用例覆盖 R1 场景（happy 裸名+报告、dry-run 文件保留、锁占用拒绝+零删除〔lib 持锁 + CLI 子进程，沿用既有锁用例模式〕、缺失拒绝、路径形态+伴生、corrupt 可删、absent 伴生跳过）；RED 建立后实现。锁占用用例前置：`Database::open`（lib API）持锁期间 spawn CLI delete。
- GREEN condition: `cargo test --test cli_test` 全绿（含既有用例零校准）。
- Verification: ① `cargo test --test cli_test delete` —— 全部新用例通过，退出码 0；② `cargo test -- --test-threads=1` 全量零回归（基线 1239+新增）；③ I072 矩阵人工实验（步骤见下），每项一句结论记入 Act Response；④ README 两语言各含备份段（人工检查：声明与矩阵结论一致、提及 dump/restore 首选与成对规则）。
  - 矩阵实验步骤（确定性）：预建库建表（干净 close）→ 循环 {spawn `rtsql <db> "INSERT ..."` 并在数百毫秒内 kill -9，检查 `<db>.wal` 大于阈值}得到未 checkpoint 尾部态 → **不打开原库**先快照两份副本（仅 `.db` / `.db`+`.wal` 成对）→ 打开原库确认行在（证明 WAL 持有该事务）→ 打开成对副本（行应在——重放）→ 打开仅主文件副本（行应缺——尾部丢失）→ 其余形态（干净后单拷/成对移动/只重命名主文件）按 D5 表逐一 CLI 验证。
- Stop when: `File::try_lock` 在目标平台行为与预期不符（如只读 fd 不可锁）；锁探测无法区分「被占用」与「其他错误」；README 现有结构无合理落点；矩阵结论与 D5 预期矛盾（需要回 Plan 修订文档声明）。

**Invariants**

- delete 全程不经 `Database`/`FileStorage`——不初始化头、不触发恢复、不触碰密钥。
- 持锁窗口覆盖全部删除动作（无检查-删除竞态窗口）。
- 删除顺序 `.wal` → `.checkpoint` → 主文件（崩溃中断时残留态可干净重开、可重试）。
- dry-run 零文件系统变更（无 open、无 metadata 以外的读、无锁）。

**Non-goals**

- T2/T3 全部内容；`install.sh` 行为；回收站/确认交互；目录形态目标（delete 目标是文件族，非目录）；Windows 兼容面。

**Acceptance**

- R1 的 8 个场景各自有通过断言（映射见 RTM），在 `cargo test --test cli_test` 下全绿。
- I072：README.md 与 README.zh-CN.md 均含备份边界段，内容与实测矩阵一致；矩阵 6 形态各有记录结论。
- 既有生命周期子命令与主命令零回归（全量测试零校准通过）。

**Verification**

- R1 场景 1（删除裸名库并报告）：`cargo test --test cli_test delete_happy_bare_name_reports_freed`——退出码 0、stdout 逐文件含主文件与 `.wal`、三路径均不存在、`list` 不再枚举。
- R1 场景 2（dry-run）：`delete_dry_run_keeps_files`——清单输出、文件仍在、库仍可开。
- R1 场景 3（打开中拒绝）：`delete_locked_rejects_without_deletion`——exit 4、文件原样。
- R1 场景 4（缺失）：`delete_missing_db_errors`——exit 1、点名路径。
- R1 场景 5（路径形态+伴生）：`delete_path_form_with_companions`——三文件删净并报告。
- R1 场景 6（corrupt 可删）：`delete_corrupt_file_deletable`——exit 0。
- R1 场景 7（absent 跳过）：`delete_absent_companions_skipped`——`absent` 行、exit 0。
- R1 场景 8（既有零回归）：全量 `cargo test -- --test-threads=1` 零校准。
- I072：上述人工矩阵步骤 + README 双语检查（人工，一句话结论入 Act Response）。

**Gate 2 Readiness**

见 change 交接报告的 Gate 2 结论；本文件在计划获批后由 Plan 将 Status 置 `ready`。

**Persisted Evidence**

- Mode: none

（全部验证为低成本可重跑命令/人工步骤，Act Response 承载结论即可；矩阵实验无一次性环境依赖——未 checkpoint 尾部态可用 kill -9 随时重建。）

**Risks and Notes**

- kill -9 定时实验存在重试循环（WAL 未及落盘时该事务不持久——以「打开原库行在」为准入检查，不满足则重试），非验收门，属实验前置。
- `File::try_lock` 对只读 fd 的可锁性：引擎自身以读写 fd 持锁，标准库 flock 语义对只读 fd 同样生效（BSD flock/LockFile 均可）；若目标平台表现异常，Act 按 Stop when 返回 Plan，不自行改用其他锁机制。
- 报告行文案（`deleted`/`would delete`/`absent` 前缀措辞）为非实质选择，Act 定稿后测试断言与文案一致即可。

## Act Response

- Status: reported

**Implemented**

T1（= MS19-T01）全部交付，Iteration 000 的唯一 task 完成；Iteration 完成与否待 Plan Review `accepted` 后由 Plan 判定。

产品代码（2 文件，无存储面改动）：

- `src/cli/lifecycle.rs`：新增 `pub(super) fn delete_db(db: &str, dry_run: bool) -> ExitStatus`（同步、不开库）——`resolve_existing_db` 定位与缺失拒绝（复用既有 exit 1 形态）→ dry-run 直接走纯预览 → `File::open`（只读）+ `try_lock`（`WouldBlock` → `Locked` exit 4 零删除；`Error` → `General`）→ **持锁窗口内**删除 → `drop` 释放锁。新增私有辅助 `delete_file_family(db_path, dry_run)`：按 `.wal` → `.checkpoint` → 主文件顺序逐文件 `metadata().len()` + `remove_file`，经 `emit_stdout` 逐行报告 `deleted <path> (<n> bytes)` / `absent <path>`（dry-run 为 `would delete`），首个非 NotFound 错误在报告后整体 `General` exit 1。新增 `use std::path::Path`。
- `src/cli/mod.rs`：`Command` 枚举新增 `Delete { db: String, #[arg(long)] dry_run: bool }`（置于 `Import` 与 `Stats` 之间）；`execute_command` 新增匹配臂 `Some(Command::Delete { db, dry_run }) => lifecycle::delete_db(db, *dry_run)`（不加 `.await`）。

测试（1 文件）：`tests/cli_test.rs` 末尾新增 MS19 Iteration 000 段，7 个用例一一对应 R1 场景 1-7（场景 8 由既有用例集 + 全量零回归承担）：`test_delete_happy_bare_name_reports_freed` / `test_delete_dry_run_keeps_files` / `test_delete_locked_rejects_without_deletion` / `test_delete_missing_db_errors` / `test_delete_path_form_with_companions` / `test_delete_corrupt_file_deletable` / `test_delete_absent_companions_skipped`。

文档（2 文件）：`README.md` / `README.zh-CN.md` 各增「备份边界与数据库文件族」小节（首选 dump/restore、文件族构成、7 形态实测表、成对规则、delete 与 `install.sh --purge-data` 边界）、「删除数据库」小节（用法、文件级语义、exit 4/1、`absent` 不算错误），CLI 命令面表各增 `rtsql delete <db> [--dry-run]` 行。

**Deviations from Plan**

1. **用例命名带 `test_` 前缀**：RTM 列出的是 `delete_happy_bare_name_reports_freed` 等裸名，实际函数名为 `test_delete_*`（本文件 83 个用例的既有约定）。Plan Context Verification 里的 `cargo test --test cli_test delete_...` 是子串过滤，仍全部命中，两种写法等价。
2. **报告行顺序为伴生先、主文件后**（D1 强制的删除顺序），spec 场景只要求「逐文件报告」未规定顺序。
3. **I072 矩阵实测 7 形态**（Acceptance 要求 ≥6）：D5 表 5 行 + 「干净关闭后成对拷贝」+ 「干净关闭后拷 `.db`+`.checkpoint`」。
4. **形态「未 checkpoint 只拷 `.db`」的实测结论比 D5 预期更细**：主文件已被后台 flush 写入 32 页（131136 字节）却只有 1 行可见——被 flush 的行携带被杀事务的 id，无 WAL 恢复时按未提交处理。文档按实测写「静默不完整（938 行中的 1 行）」而非「丢失尾部 N 行」，未沿用 D5 的模糊表述。
5. **kill -9 构造尾部态的两处实测约束**（Plan Context Risks 已预告重试循环，此处定稿）：① 单条 argv 的 SQL 文本受 Linux `MAX_ARG_STRLEN`（128 KiB）限制，568 KB 脚本直接 exit 126、根本没执行、WAL 零增长；改为 82 KB / 3000 语句。② debug 构建下 3000 语句全量约 6.2 s，2 s 时 kill 命中执行阶段（WAL 98406 字节），一次成功。
6. **`cargo fmt --check` 的既有偏差未清理**：`src/executor/delete.rs`、`src/storage/data/table_manager.rs`、`src/wal/recovery.rs`、`tests/cli_test.rs:3346-3673`、`tests/constraint_enforcement_test.rs` 存在非本次引入的格式偏差（均为 MS23/MS24 批次已提交内容）。本次新增代码自身 fmt-clean；按 Surgical Changes 不动无关代码。
7. **副本引用必须含 `/`**：实验脚本首轮把副本写成裸名（`form1.db`），被解析为 `$RTSQL_HOME/db/form1.db` 并由主命令**新建空库**（主命令对不存在的库即创建），一度误读为「拷贝丢数据」。改为绝对路径后各形态结论正常。记录此点是因为它同时解释了「主命令对不存在路径建库」的既有行为。

**Blocker Handoff**

None.

**Blocker Resolution**

None.

**Self-Review**

Gate 3（spec compliance）：R1 的 8 场景逐条核对全部实现并有断言（场景 8 零回归见 Verification ②）；契约的 Preserve 项（既有 10 个子命令与主命令、退出码、文案逐字节不变、锁消息形状 `database is locked: {path}`、裸名/子命令冲突子命令优先）经全量零校准通过；Forbidden 项（不触 Database/FileStorage/WAL/checkpoint、不引头解析校验、无确认交互/`--force`、不动错误面）经 diff 核对成立——`delete_db` 只用 `std::fs`（`File::open`/`try_lock`/`metadata`/`remove_file`）与既有 `resolve_existing_db`/`emit_stdout`。Invariants 四条成立：不经 Database/FileStorage（不初始化头、不触发恢复、不触密钥）；持锁窗口覆盖全部删除动作；删除顺序伴生先主文件后；dry-run 只做 `metadata` 读、无 open 无锁。

Gate 3（code quality）：无计划外修改；错误与边界路径（`WouldBlock`/`Error`/NotFound/其他 IO）均有显式臂且不掩盖；锁对象生命周期显式 `drop` 且各提前返回路径隐式释放；无死代码（`delete_file_family` 单一调用点）、无重复实现、无新增 warning（`cargo clippy --all-targets` exit 0）；无身份型证据机制或判定层（测试经真二进制 `CARGO_BIN_EXE_rtsql` 判定退出码与原生输出）；命名与局部结构沿用本文件既有风格（`emit_stdout` 逐行、括号内联格式串）。

自检发现（均已处理或非阻塞）：

- Minor 1（保留）：`resolve_existing_db` 的存在性检查与实际删除之间仍有 TOCTOU 窗口（检查后主文件被外部移除 → 报 `absent <main>` 且 exit 0）。报告诚实、非静默，且与既有生命周期命令同形，不修。
- Minor 2（保留，行为已验证）：目录形态目标下伴生报 `absent`、主文件报 `failed to delete ...: Is a directory (os error 21)`、exit 1、目录原样保留。非 goales 明示「delete 目标是文件族，非目录」，不新增用例。
- Minor 3（**留给 Review**）：全量串行首跑 `tests/cli_test.rs` 出现 1 项失败，未捕获用例名（该次输出经管道过滤后未保留失败段）。按公共规则允许的一次偶发复核：`cargo test --test cli_test -- --test-threads=1` 与 `cargo test -- --test-threads=1` 均通过（见 Verification ①②），按通过处理。既有时间敏感用例族（`SIGNAL_DELAY_MS = 200` 的信号用例、`calibration_recovery_time`）为可能来源但**未验证**，不在本 change 范围，未作 Issue 候选。
- 未发现 Critical / Important 问题。

**Verification Evidence**

① 目标测试（新用例，GREEN）：`cargo test --test cli_test delete`

```
running 8 tests
test test_delete_absent_companions_skipped ... ok
test test_delete_corrupt_file_deletable ... ok
test test_delete_missing_db_errors ... ok
test test_delete_path_form_with_companions ... ok
test test_delete_dry_run_keeps_files ... ok
test test_delete_locked_rejects_without_deletion ... ok
test test_delete_happy_bare_name_reports_freed ... ok
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 100 filtered out
```

（第 8 条 `test_update_delete_bare_names_on_quoted_created_table` 为既有用例被子串命中。）

② 全量零回归（场景 8）：`cargo test -- --test-threads=1`（串行，退出码 0；81 个测试二进制）

```
passed=1246 failed=0 ignored=2
```

基线 1239 + 本次新增 7 = 1246。**首跑失败记录**：同命令首跑 `tests/cli_test.rs` 报 105 passed / 1 failed（96.40s），复核通过（见 Self-Review Minor 3）。

③ 静态检查与规格校验：

- `cargo clippy --all-targets` → exit 0（仅 `~/.cargo/config` 弃用提示，非本项目告警）
- `cargo fmt --check` → 本次新增代码无差异（差异文件清单见 Deviations 6，均为既有）
- `openspec validate 2026-09-26-ms19-cli-admin-hardening --strict` → `Change '2026-09-26-ms19-cli-admin-hardening' is valid`，exit 0

④ CLI 实际运行（真实二进制，`/tmp/opencode/ms19-cli-demo`）：

```
$ rtsql delete                          → exit=2（clap usage）
$ rtsql delete nosuch                   → exit=1（"/…/db/nosuch.db does not exist"）
$ rtsql delete demo --dry-run           → exit=0，3 行 would delete 清单，文件全在
$ rtsql delete demo                     → exit=0，deleted .wal(21B)/.checkpoint(24B)/.db(16448B)，db/ 清空
$ rtsql delete demo（再次）             → exit=1（"does not exist"）
$ rtsql --help | grep delete            → 列出 delete 子命令
```

⑤ 契约中两项无自动化用例的声明，直接验证：

- 加密库免密钥删除（`/tmp/opencode/ms19-cli-demo2`）：`rtsql --key pw123 new sec` 建库写入后，`rtsql delete sec --dry-run` 与 `rtsql delete sec`（**不带 `--key`**）均 exit 0，三文件删净 —— R1「SHALL NOT 要求密钥」成立。
- 目录形态目标：`mkdir db/dirty.db` 后 `rtsql delete dirty` → 伴生 `absent`、主文件 `failed to delete …: Is a directory (os error 21)`、exit 1、目录保留。

⑥ I072 实测矩阵（`/tmp/opencode/ms19-i072`，`RTSQL_HOME` 指向该目录；真值 = 原库 kill -9 后经 WAL 重放为 **938 行**）：

| 形态 | 实测 | 结论 |
|---|---|---|
| 干净关闭后只拷 `.db` | 1 行 / 1 行 | 完整（close 即 checkpoint，WAL 已截断 21 B） |
| 干净关闭后成对拷 `.db`+`.wal` | 1 行 / 1 行 | 完整 |
| 干净关闭后拷 `.db`+`.checkpoint` | 1 行 / 1 行 | 完整（位点文件非数据必需） |
| 未 checkpoint 态只拷 `.db` | **1 行 / 938** | 静默不完整（打开无报错） |
| 未 checkpoint 态成对拷 `.db`+`.wal` | 938 行 | 完整（打开时 WAL 重放） |
| 成对移动到另一目录（同名） | 938 行 | 完整（伴生名由主文件名派生，同基名即可） |
| 只重命名主文件（`.wal` 留原处） | **1 行 / 938** | 尾部丢失，且 98406 B 孤立 `.wal` 被静默忽略 |

矩阵 7 形态结论与 README 双语声明逐条一致（README 表与上表同数据）。

**Persisted Evidence**

None required（Plan 设定 Mode: none，予以维持）。尾部态可用 `SIGKILL` 随时重建、矩阵各形态为一次性文件拷贝，全部结论已落在本节表格；实验目录 `/tmp/opencode/ms19-i072` 是一次性现场，无保留价值，不建 `evidence/` 目录。

**Experience Candidates**

- Runbook 候选（证据：本节 ⑥ 与 Deviations 4/5）：「文件对备份边界实测」的可复现操作路径 —— ① 单条 argv 受 Linux `MAX_ARG_STRLEN` 128 KiB 限制，大 SQL 脚本需降到 82 KB / 3000 语句（debug 全量约 6.2 s）；② kill 时刻取全量耗时约 1/3（2 s）稳定命中执行阶段，以「`.wal` 显著大于阈值 + 原库 open 后行数为准入检查」；③ 副本必须用含 `/` 的绝对路径，否则裸名被解析进集中存储区并被主命令**新建空库**，会伪装成「拷贝丢数据」；④ 必须先快照全部副本再打开原库（打开即 checkpoint 消费尾部）。已端到端验证成功、可重复且步骤非显然，符合 Runbook 准入。
- Issue 候选：None（本 change 范围外无已证据的实质缺陷；首跑那 1 项未捕获失败见 Minor 3，未复现故不落账）。

**Remaining Issues**

- Critical / Important：无。
- Minor：见 Self-Review 1-3（1、2 为保留不修的既有形态，3 为留给 Review 的首跑失败记录）。

## Plan Review

- Review Result: accepted

**Findings**

独立审查（git diff 全量阅读 + 代码与 Act 声明逐条核对 + 定向复跑），非阻塞结论如下；无阻塞 Acceptance 的发现。

- **diff 一致性（Plan 独立阅读）**：本次变更面恰为 Act 声明的 5 文件（`src/cli/mod.rs`、`src/cli/lifecycle.rs`、`tests/cli_test.rs`、`README.md`、`README.zh-CN.md`）+ change 目录；`.claude→.agents` 迁移与 AGENTS.md/CLAUDE.md 变更为会话开始前已暂存的既有现场，不属本 Cycle。五文件 diff 全为**纯新增**（无既有行修改），「既有用例零校准」由 diff 结构直接证实。
- **代码与 D1 契约吻合**：`delete_db` 顺序（定位/存在检查 → dry-run 前置分支 → `File::open` 只读 + `try_lock` → 持锁窗口内 `delete_file_family` → `drop`）、删除顺序（`.wal`→`.checkpoint`→主文件）、`WouldBlock → Locked("database is locked: {path}")` 零删除、`absent` 跳过、失败收集后报告完整再整体 General——逐条与 Plan Context Implementation Guidance 及 Invariants 一致；不经 Database/FileStorage、无内容校验、无确认交互（Forbidden 成立）。
- **测试映射**：7 新用例与 R1 场景 1-7 一一对应，断言直接判定退出码/原生输出/文件系统终态；S3 用例（测试进程只读 fd 持锁 → exit 4 → 释放后成功）同时实证了 Plan Risks 中「只读 fd 可 flock」假设。RTM 见证名带 `test_` 前缀为 Act 偏差 1（子串过滤等价）。
- **偏差分类**：Act 偏差 1-7 逐条审阅——1/2 为 Plan 遗漏的局部约定（命名前缀、报告顺序 spec 未规定），3/4 为实测更完整（7 形态 ≥6、结论比 D5 预期更细且更诚实），5 为实验约束定稿（Plan Risks 已预告），6 为 Surgical Changes 正确应用，7 为有价值的过程教训——全部非实质、有记录、不阻塞。
- **Minor 1/2（保留不修）同意**：`resolve_existing_db` 后的 TOCTOU 残窗（检查后被外部移除 → `absent` + exit 0）报告诚实、与既有生命周期命令同形；目录形态目标非目标场景且行为已验证。
- **Minor 3（首跑 1 失败）定向审计**：Plan 独立复跑 `cargo test --test cli_test -- --test-threads=1` → **106 passed / 0 failed / 2 ignored**（97.73s），疑点未复现；Act 的一次复核符合公共规则 › 验证 的偶发复核条款，按通过处理。不满足 Issue 准入（未复现、无证据、不指向实质缺陷），不落账。
- **测试二进制计数观察（非本 change 缺陷）**：Act 记 81，SNAPSHOT「仓库现场」段记 82（MS24 收尾时）；Plan 独立清点 `tests/*.rs` 79 文件 + lib 单测 + doctest = 81，与 Act 一致。属 SNAPSHOT 散文记录与实际计数的既有差异，随 Maintainer 收尾同步自然校正，无需动作。
- **Runbook 候选合格**：「文件对备份边界实测」步骤可重复、非显然、有证据（Act Verification ⑥ 与 Deviations 4/5）——维持候选状态，Recorder 按用户指令建立产物。

**Acceptance Gaps**

None.（R1 场景 1-8 逐一有独立核对或采信注明的通过证据；I072 双语 README 与 7 形态矩阵一致；全量 1246/0/2。）

**Evidence**

- 独立 diff 阅读：`git diff HEAD -- src/cli/{mod,lifecycle}.rs tests/cli_test.rs README.md README.zh-CN.md`（本会话原文见对话记录；纯新增结构）。
- 定向复跑：`cargo test --test cli_test -- --test-threads=1` → `test result: ok. 106 passed; 0 failed; 2 ignored`（97.73s，退出码 0）。
- 采信 Act 未失效结论（来源：Act Response Verification ①②③⑤⑥，其覆盖范围与当前代码一致）：新用例 GREEN、全量 `cargo test -- --test-threads=1` 1246/0/2、clippy exit 0、加密库免密钥删除、目录形态行为、I072 七形态矩阵。
- 文本形态探针（Plan 实测，供 Iteration 001 基线）：`Plan error: Unsupported statement type`（MERGE 形态）、`Parse error: Parse error: sql parser error: …`（既有双前缀 quirk）——T2 的「追加能力名、框架不变」基线即此。

**Follow-up Decision**

`accepted`：T1 全部 Acceptance 满足，偏差与 Minor 均非阻塞，无需当前 Cycle 修复或 rework。Iteration 000 完成。

**Iteration Plan Update**

None（Map 保持不变）。

**Next Cycle**

None。

**Next Iteration**

`iterations/001-error-surface-session-e2e/000-initial.md`（已展开，Plan Context `draft`，待其 Gate 2）
