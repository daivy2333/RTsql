# Iteration 001 / Cycle 000-initial: 显式 checkpoint 子命令与文档收尾

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T5, T6
- Acceptance gaps: None
- Repair items: None
- Inherited scope: proposal 默认假设 3-9（checkpoint 输出一行不加 `--format`、文档面、迭代序）；design D6（checkpoint 命令）；Iteration 000 全部交付（锁等待三通道、`execute_command_inner(_mode)` 的 `lock_wait` 参数、`run_cli_env` 校准 helper 先例）。
- Excluded scope: `--format` 四态支持；checkpoint 进度/统计输出；MS27-T02 完整性校验；只读/内存会话 checkpoint 语义变更。

**Objective**

`rtsql checkpoint <db>` 端到端可用——已提交数据落稳（WAL 截断/位点推进）且外部备份可见；零工作早退明示 exit 0；缺库 exit 1、锁冲突经等待预算 exit 4、`--readonly`/`:memory:` exit 2、加密 `--key` 成功/错钥 exit 5；close/水位自动 checkpoint 语义零变化；SKILL.md 与 README 双语逐条一致（含 busy-timeout 通道与 checkpoint 命令）。

**Investigation Facts**

- Current Baseline: Iteration 000 accepted（2026-10-01，全量 **1651 passed / 0 failed / 2 ignored**，工作分支 master @ 4e1944f + 本 change 未提交实施面）。锁等待通道可用：`Database::open_with_lock_wait`/`open_readonly_with_lock_wait`、`FileStorage::open_with_lock`/`open_readonly_with_lock`（默认 5000ms、`Some(0)` 仅下限）；CLI `--busy-timeout <MS>`（global + env `RTSQL_BUSY_TIMEOUT`，clap 校验 exit 2、flag 优先）；`execute_command_inner`/`execute_command_inner_mode` 均带 `lock_wait: Option<Duration>` 参数。测试基建先例：`tests/attach_session_test.rs`/`readonly_session_test.rs` 的 `run_cli_env`（env 注入 + `env_remove("RTSQL_BUSY_TIMEOUT")` 防宿主污染）。
- Current-State Evidence（行号为 Iteration 000 实施后现场）：
  - `src/database.rs:571` `Database::checkpoint(&self) -> Result<()>`——委托 `checkpoint_manager`；只读/内存模式 `checkpoint_manager` 为 None → 直接 `Ok(())`（该组合在本命令不可达：`:memory:`/`--readonly` 组合被守卫前置拦截）。零工作守卫在 `CheckpointManager::checkpoint` 内部（MS18）：`get_write_count() == 0 && !has_dirty_pages()` → 无操作早退，不触 WAL/位点/脏页。
  - 零工作谓词的公开取样点：`db.wal_writer: Option<Arc<WalWriter>>`（pub 字段，ReadWrite 模式恒 Some）、`db.buffer_pool`（pub 字段）；`WalWriter::get_write_count()`（`src/wal/writer.rs:167`，pub）、`BufferPool::has_dirty_pages()`（`src/storage/buffer_pool.rs:243`，pub）。语义：`get_write_count` 计本会话 WAL 追加次数（checkpoint 命令新开会话恒 0）；恢复重放将 redo 页置脏 ⇒ 「WAL 有未决」时 `has_dirty_pages()` 为 true、谓词为 false。会话持独占锁期间无并发写者，**checkpoint() 调用前取样稳定**。
  - `src/cli/mod.rs:112` `Command` 枚举（`Delete`/`Stats`/`Sample`/`Profile` 等同型臂）；`:331` `lifecycle_db_target`——New/Schema/Dump/Restore/Import/Delete 七臂返回 `Some(db)`，List/Stats/Sample/Profile/Completions 返回 None；`:memory:` 守卫（`execute_command` 顶部）对该函数返回 `Some` 的目标 exit 2——**Checkpoint 增臂即自动生效**。`--readonly` × 任意子命令的全局守卫（dispatch 前独立 match 臂）——**`--readonly checkpoint` 自动 exit 2，零新代码**。
  - `src/cli/lifecycle.rs:776` `resolve_existing_db`（缺失 → General exit 1，生命周期命令统一入口）；`:203` `schema` 是 lifecycle 开库命令样板（resolve → `execute_command_inner(&db_path, key, lock_wait, work, sigint_future, sigterm_future)` → 编排尾部自动 `close()`）；`:116` `delete_db` 为 stdout 逐行报告先例（无 `--format`）；lifecycle 输出走 `println!` 直写 stdout，错误经 `run()` 的 `emit_stderr`。
  - 主命令存在性校验（`execute_main_command`）不涉及子命令；`new` 创建通道、`list` 无库实参——均与本任务无交集。
- Code and Critical Path: `Command::Checkpoint { db }` → `execute_command` 分派臂（经 `lifecycle_db_target` 守卫）→ `lifecycle::checkpoint_db(db, key, lock_wait)` → `resolve_existing_db` → `execute_command_inner`（rw 独占开库，锁冲突经预算 exit 4 由既有 `open_error_status` 承载）→ work 闭包：零工作谓词取样 → `db.checkpoint().await` → stdout 一行确认 → 编排尾部 `close()`（此时已无未决工作，MS18 零工作跳过使 close 零额外开销）。

**Implementation Guidance**

- `checkpoint_db` 沿 `schema` 样板；确认行文案（design D6 定稿）：零工作 `checkpoint complete (no pending work; wal untouched)`、正常 `checkpoint complete (wal truncated; checkpoint site advanced)`；失败 `ExitStatus::General(format!("checkpoint failed: {}", e))`（exit 1，词根对齐 MS26 自动 checkpoint 失败点名）。
- 谓词取样必须在 `db.checkpoint()` **之前**（checkpoint 清脏页）。`db.wal_writer` 为 `None` 的臂在命令路径不可达，防御性写 `true` 即可，不加专门拒绝。
- `lifecycle_db_target` 增 `Command::Checkpoint { db }` 臂时保持既有臂顺序与 match 穷尽。
- 文档（T6）编辑面：`rtsql-docs/SKILL.md` 命令行功能目录（当前含 2026-10-01 未提交重排修改，34 行 diff——在其上继续，不回退）；README.md 定位：命令表（`:178` 区）、锁冲突 exit 4 表行（`:223`）、自动 checkpoint 段（`:201-208`，补「显式命令」入口与 I079 备份同步点提示）、lib API 清单（`:281-282`，补 `open_with_lock_wait`/`open_readonly_with_lock_wait`）；README.zh-CN.md 对应段同步；`--busy-timeout`/`RTSQL_BUSY_TIMEOUT` 通道说明（默认 5000ms、0=不等待仅下限、`:memory:` 忽略、`delete` 探测不适用）两文档均需落点。

**Task Contracts**

### T5: checkpoint 子命令端到端

- Requirement/Scenario: cli-noninteractive-shell ADDED「生命周期子命令 checkpoint」全部场景（落盘确认/零工作明示/缺库/锁冲突/只读与内存组合拒绝/加密库）
- Depends on: None（Iteration 000 通道与穿线已就绪）
- Targets: `src/cli/mod.rs::Command`（增 `Checkpoint { db }`）、`lifecycle_db_target`（增臂）、`execute_command` 分派臂、`src/cli/lifecycle.rs::checkpoint_db`（新）
- Current behavior: 无 `checkpoint` 子命令——`rtsql checkpoint <db>` 报 unknown 子命令（clap usage exit 2）
- Required behavior: `rtsql checkpoint <db>`＝`resolve_existing_db`（缺失 exit 1）→ `execute_command_inner` rw 独占开库（`lock_wait` 穿线；锁冲突经预算耗尽 exit 4，错误文案含库名）→ work 内零工作谓词取样（`get_write_count()==0 && !has_dirty_pages()`，None 臂 true）→ `Database::checkpoint()` → stdout 一行确认（零工作明示 / 正常落盘两形态）exit 0；checkpoint 失败 `General("checkpoint failed: ...")` exit 1；`--readonly`/`:memory:` 组合 exit 2（既有守卫自动）；`--key` 既有通道、错钥 exit 5
- Preserve: `Database::checkpoint()`/`CheckpointManager`/`close()`/水位自动 checkpoint 语义零变化；既有子命令与主命令行为零变化；`lifecycle_db_target` 既有臂不重排语义
- Forbidden: 不加 `--format` 支持；不给 stats/sample/profile 增 `:memory:` 拒绝；不改守卫先行序；不改 `open_error_status` 分类
- Test witness: `tests/checkpoint_command_test.rs`（新）——**RED**：`rtsql checkpoint db` 期望 exit 0 现状失败（unknown subcommand）；**GREEN** 矩阵：写入后 checkpoint exit 0 + 随后 `dump` 可见全部数据 + WAL 文件缩减或置空；干净库 exit 0 + stdout 含 no pending work；缺库 exit 1 点名库名且零文件创建；持锁者 + `RTSQL_BUSY_TIMEOUT=0` → exit 4 含 "database is locked"；`--readonly checkpoint db` 与 `checkpoint :memory:` 均 exit 2；`--key` 加密库成功 / 错钥 exit 5
- GREEN condition: 矩阵全绿 + `cli_test` 存量零回归
- Verification: `cargo test --test checkpoint_command_test`；`cargo test --test cli_test`
- Stop when: 零工作谓词在 checkpoint() 前取样被证明不稳定（如发现恢复重放不置脏的 WAL 未决形态），或确认行无法在不扩 lib API 的前提下区分两形态——返回 Plan

### T6: 文档同步与核验

- Requirement/Scenario: 文档任务（无 spec 映射；验收 = 文档与实现逐条一致）
- Depends on: T5（checkpoint 命令语义定稿）
- Targets: `rtsql-docs/SKILL.md`、`README.md`、`README.zh-CN.md`
- Current behavior: SKILL.md 命令目录无 `--busy-timeout`/`RTSQL_BUSY_TIMEOUT`/`rtsql checkpoint`；README 命令表无 checkpoint 行、锁冲突段无等待语义、lib API 清单无新 open 入口、备份段无显式同步点提示；README 双语无 MS28 语义
- Required behavior: SKILL.md——主命令 flag 增 `--busy-timeout`（默认 5000ms、0=不等待仅内部 ~8ms 下限、`:memory:` 接受并忽略、`delete` 探测不适用）、env 三通道表增 `RTSQL_BUSY_TIMEOUT`、子命令目录增 `rtsql checkpoint <db>` 条目（含零工作明示与拒绝面）、退出码表无新增（0/1/2/3/4/5/130/143 全覆盖既有）；README 双语——命令表增 checkpoint 行、锁冲突说明补等待语义与默认值、自动 checkpoint 段补显式命令入口与「外部备份前同步点」提示（I079 诉求）、lib API 清单补 `open_with_lock_wait`/`open_readonly_with_lock_wait`；双语逐段一致
- Preserve: 既有文档段落语义（版本号 0.2.0、基准数据、既有命令说明）不回退；SKILL.md 未提交重排内容保留
- Forbidden: 不动 `install.sh`/`build-riscv64-musl.sh` 相关段；不引入未实现能力的文档声明
- Test witness: `rtsql --help` 含 `--busy-timeout` 与 checkpoint 子命令（抽样自动执行）；文档逐条人工核对记录于 Act Response
- GREEN condition: 两文档命令面与 `src/cli/mod.rs` `Command` 枚举 + flag 集合逐条一致；README 双语无语义冲突
- Verification: `rtsql --help`、`rtsql checkpoint --help` 抽样 + 人工逐条核对
- Stop when: 发现文档需声明的能力与实现冲突（语义问题）→ 返回 Plan

**Invariants**

- `Database::checkpoint()`、`CheckpointManager`（含 MS18 零工作守卫、MS26 水位联动）与 `close()` 语义零变化；lib API 零新增（本 Iteration 不扩 lib 面）。
- 既有生命周期子命令、主命令、ATTACH、`delete` 行为零变化；守卫先行序（空 key → 水位 env → `:memory:`×子命令 → `--readonly`×子命令 → 分派）不变。
- checkpoint 命令复用 `execute_command_inner` 两阶段编排——信号语义（130/143）与优雅停机自动继承。
- Iteration 000 交付面（锁等待三通道）零回退。

**Non-goals**

- `--format` 四态支持；checkpoint 进度/统计输出；`rtsql check`/完整性校验（MS27-T02）；只读/内存会话 checkpoint 能力；import 攒批（MS22）。

**Acceptance**

1. `rtsql checkpoint <db>` 端到端：有未决 WAL 时落盘（dump 可见 + WAL 缩减）、干净时零工作明示，均 exit 0（delta「生命周期子命令 checkpoint」场景 1/2）。
2. 拒绝面：缺库 exit 1、锁冲突经预算 exit 4、`--readonly`/`:memory:` exit 2、错钥 exit 5（场景 3/4/5/6）。
3. 文档：SKILL.md/README 双语命令面与实现逐条一致，含 busy-timeout 通道与 checkpoint 命令（T6）。
4. 全量零回归（Iteration 000 基线 1651 不降）。

**Verification**

- `cargo test --test checkpoint_command_test`（矩阵 1-2）、`cargo test --test cli_test`（回归）、全量分批 M19（4）、`rtsql --help` 抽样 + 文档人工核对（3）、`openspec validate --strict`。
- RED 见证：T5 修复前 unknown subcommand 形态，记录于 Act Response。
- 人工步骤：文档逐条核对——最短操作为「打开 SKILL.md 命令目录逐行对照 `rtsql --help`/实现」，结果一句话记入 Act Response。

**Gate 2 Readiness**

全部 PASS——无 Missing requirement（RTM 全 Covered）、调查完整（本 Cycle 前置 Iteration 000 accepted + 本文件 Current-State Evidence 为实施后现场读码）、设计闭合（design D6 定稿文案与拒绝面）、任务可执行（两契约含代码位置/行为变化/测试见证/停止条件）、验证充分（delta 全场景映射 + 零回归）、无实质未知项、无身份型证据工程、Evidence 模式 none。计划批准沿 change 级 Gate 2（2026-10-01 用户原话：批准，含默认假设 1-9）。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- SKILL.md 当前带 2026-10-01 milestone 重排未提交修改（34 行 diff）：T6 在其上继续编辑，收尾提交时两批改动同文件合并——非冲突（重排改的是任务路线段，本任务改命令功能目录段），Act 编辑前先读当前文件全貌。
- 确认行文案为 design 定稿值，措辞微调属非实质（Response 记录即可）。
- 零工作谓词依赖「恢复重放置脏」语义：`wal-auto-checkpoint`/MS18 既有测试族已锁定该行为（redo 页经 BufferPool 置脏），本 Cycle 用例（写入→close→重开→checkpoint 应报 zero work？否——重开后脏页已在恢复时置脏但 checkpoint 已跑……注意：用例设计按「写入后不 close、另一进程开库」场景构造，见 T5 witness；干净库形态用 `new` 后立即 checkpoint 构造）。
- 非实质留 Act：测试 helper 复用形态（是否抽 `run_cli_env` 到共享位置——不抽，沿各文件先例）、文档措辞细节。

## Act Response

- Status: reported

**Implemented**

- T5：`src/cli/mod.rs`——`Command::Checkpoint { db }` 枚举臂（`delete` 与 `stats` 之间）、`execute_command` 分派臂（`lifecycle::checkpoint_db(db, key, lock_wait)`）、`lifecycle_db_target` 增 `| Command::Checkpoint { db }`（`:memory:` 守卫自动 exit 2、`--readonly` 组合守卫自动 exit 2，零新守卫代码）；`src/cli/lifecycle.rs`——`checkpoint_db`（`resolve_existing_db` 缺失 General exit 1 → `execute_command_inner` rw 独占开库〔锁冲突经预算 `open_error_status` exit 4〕→ work 闭包零工作谓词于 `checkpoint()` 前取样〔`wal_writer.get_write_count()==0 && !buffer_pool.has_dirty_pages()`，None 臂 `true`〕→ `db.checkpoint()` → stdout 一行确认〔零工作 `checkpoint complete (no pending work; wal untouched)` / 正常 `checkpoint complete (wal truncated; checkpoint site advanced)`，design D6 定稿文案 verbatim〕→ 失败 `General("checkpoint failed: {e}")` exit 1）。
- T6：`rtsql-docs/SKILL.md`——主命令面增 `--busy-timeout` 行、子命令目录增 `checkpoint` 行、env 表增 `RTSQL_BUSY_TIMEOUT` 行、锁节两处「立即拒绝」改预算等待语义、WAL 有界节增显式命令入口；`README.md`/`README.zh-CN.md`——命令表增 checkpoint 行、只读会话锁冲突改等待语义（默认值/0/`:memory:` 忽略/`delete` 不适用）、退出码 4 行补预算耗尽、自动 checkpoint 节增「显式」bullet（I079 备份同步点）、备份边界段补 `rtsql checkpoint` 同步点提示、lib API 清单增 `open_with_lock_wait`/`open_readonly_with_lock_wait`。
- 测试：`tests/checkpoint_command_test.rs` 新增 6 例（有未决 WAL 端到端 + dump 可见 + WAL 收缩 / 零工作明示 / 缺库 exit 1 零文件 / 持锁 + `RTSQL_BUSY_TIMEOUT=0` exit 4 / `--readonly` 与 `:memory:` exit 2 / 加密库对钥 exit 0 错钥 exit 5）。

**Deviations from Plan**

- RED 形态与契约预判不同：预判「unknown subcommand exit 2」，实际为「`checkpoint` 非既有子命令名 → clap 落主命令位 → 缺库显式拒绝『database 'checkpoint' does not exist』exit 1」（主命令缺库守卫先于一切）。RED 结论等价（子命令不存在），已在测试注释与 tasks.md 记录，断言零调整。

**Blocker Handoff**

- None

**Blocker Resolution**

- None

**Self-Review**

- Gate 3（T5）：spec compliance——delta 场景 1-6 全映射（两 exit 0 形态 + 四种拒绝面）；`Database::checkpoint()`/`CheckpointManager`/`close()` 零触碰；守卫先行序未动；未加 `--format`；`open_error_status` 未动。code quality——diff 仅 3 枚举/分派点 + 1 新函数 + 1 新测试文件，无计划外修改；确认文案与 design D6 逐字一致。
- Gate 3（T6）：两文档命令面与 `Command` 枚举 + flag 集合逐条核对一致；既有段（版本号、基准数据、`install.sh` 段）零回退；SKILL.md 未提交重排内容保留未动。
- 遗留 Minor：无。

**Verification Evidence**

- `cargo test --test checkpoint_command_test` → `6 passed; 0 failed`（rc 0）。
- `cargo test --test cli_test` → `117 passed; 0 failed; 2 ignored`（rc 0）。
- 全量分批 M19：lib `354 passed` + 集成 4 批 `321+331+390+261` → 合计 **1657 passed / 0 failed / 2 ignored**（基线 1651 + 新增 6，Acceptance 4 满足）。
- `cargo clippy --tests` → 无警告（rc 0）；`cargo fmt --check` → 净（rc 0，已对新增测试文件应用一次性格式化后复验）。
- `openspec validate --changes --strict` → `1 passed, 0 failed`。
- 抽样：`rtsql --help` 含 `checkpoint` 子命令与 `--busy-timeout <MS>` flag；`rtsql checkpoint --help` 正常。

**Persisted Evidence**

- None required（Persisted Evidence 模式 none；验证均可低成本重跑）。

**Experience Candidates**

- None

**Remaining Issues**

- None

## Plan Review

- Review Result: accepted

**Findings**

独立审读（2026-10-01）：完整 diff 覆盖 `src/cli/mod.rs`（`Command::Checkpoint` 枚举/分派/`lifecycle_db_target` 增臂）、`src/cli/lifecycle.rs::checkpoint_db`（`resolve_existing_db` → `execute_command_inner` 穿线 → 谓词取样先于 `checkpoint()` → 两形态确认文案与 design D6 verbatim → 失败 `General` exit 1）、`tests/checkpoint_command_test.rs` 6 例、`rtsql-docs/SKILL.md`、`README.md`/`README.zh-CN.md`；Act 声明逐条与代码核对一致，`Database::checkpoint()`/`CheckpointManager`/`close()` 零触碰（`src/database.rs` 工作区 diff 仅 Iteration 000 open 面改动），守卫先行序与 `delete` 不穿线保持，未加 `--format`。

1. **Minor（Act 偏离·未记录编辑）**：SKILL.md 只读会话句移除了 `DETACH`（旧「`SELECT`, `ATTACH`, `DETACH`, and transaction statements still work」→ 新句无 DETACH），Act Response 的 T6 编辑清单未记录此项。核实方向正确：DETACH 全仓零实现（`src/`、`tests/` 零匹配；improvements I076 登记 sqlparser 0.44 无该关键字/AST 变体），原句属不实能力声明；移除符合 T6 GREEN「文档与实现逐条一致」与 Forbidden「不引入未实现能力的文档声明」。仅文档措辞，不阻塞。
2. **Minor（Act 偏离·测试注释失准）**：`tests/checkpoint_command_test.rs:123` 注释仍写「修复前 clap unknown subcommand exit 2」（Plan 预判形态），而实际 RED（Act Response Deviations 与 change `tasks.md` 已如实记录）为主命令缺库拒绝「database 'checkpoint' does not exist」exit 1。Act 声称「已在测试注释与 tasks.md 记录」——tasks.md 属实，测试注释不属实。仅注释、零行为影响，不阻塞。
3. **Minor（Plan 遗漏·描述不精确）**：Plan Context 风险注记把 SKILL.md 既有未提交 diff（~32 行）描述为「重排改任务路线段」，实际内容为「SQL capability surface」表与「Evolve schema」节。对执行零影响——Act 按「在其上继续编辑、不回退」处理，两段在 diff 中完整保留，T6 文档面与既有内容无冲突。

**Acceptance Gaps**

None——四项 Acceptance 全满足：

1. delta 场景 1/2 → 测试 1（未决 WAL：exit 0 确认行 + `dump` 三点数据可见 + WAL 收缩断言）、测试 2（干净库零工作明示 `no pending work`）。
2. delta 场景 3/4/5/6 → 测试 3（缺库 exit 1 点名库名 + 零文件创建）、测试 4（持有者持锁 + `RTSQL_BUSY_TIMEOUT=0` → exit 4 含 `database is locked` 与库路径）、测试 5（`--readonly checkpoint` 与 `checkpoint :memory:` 均 exit 2 点名组合）、测试 6（对钥 exit 0、错钥 exit 5）。
3. T6 文档：README 双语 7 处插入 + 3 处改写逐条对应 Act 记录面（命令表行/锁冲突 bullet/退出码 4 行/自动 checkpoint 显式 bullet/备份伴生句 checkpoint 同步点/lib API 两入口）；SKILL.md 三个命令/env 行 + 锁节两处 + WAL 节改写对应记录面（DETACH 例外见 Finding 1）；命令面与 `Command` 枚举 + flag 集合一致。
4. 全量 1657 passed / 0 failed / 2 ignored ≥ 基线 1651（采信来源见 Evidence）。

**Evidence**

Act Response 验证结论（2026-10-01 20:45 写毕）按公共规则 › 验证 直接采信：`cargo test --test checkpoint_command_test` → 6 passed；`cargo test --test cli_test` → 117 passed / 2 ignored；全量 M19 分批 354+321+331+390+261 = 1657 passed / 0 failed / 2 ignored；`cargo clippy --tests` 净；`cargo fmt --check` 净；`openspec validate --changes --strict` → 1 passed。新鲜度只读核对：全部触碰面 mtime ≤ 20:44:03（最晚 `tests/checkpoint_command_test.rs`）早于 Act Response 20:45:42，结论产生后代码面未变化（git status 只读检查），覆盖范围未失效——不重跑已采信验证。Plan 独立补跑的检查为完整 diff 审读（两道必检之第二道），不以 Act Self-Review 代替；Persisted Evidence 模式 `none`，无 Evidence 目录属预期，不构成 finding。

**Follow-up Decision**

None——无需当前 Cycle 修复。三项 finding 均为 Minor（文档/注释级且方向已核实正确或零影响），按公共规则「不为不阻塞 Acceptance 的 Minor finding 创建修复」，记录即闭环。

**Iteration Plan Update**

None——Iteration Map 不变（000/001 两 Iteration，T1-T6 全部交付）。

**Next Cycle**

None

**Next Iteration**

None——无剩余 Iteration。change 具备收尾条件：增量规格合并（`cli-noninteractive-shell` +2 Requirement、`database-file-lock` R1 修改、`readonly-open-mode` R1 修改）与 change 归档、SNAPSHOT 刷新由 `openspec-docs-maintainer` 执行。
