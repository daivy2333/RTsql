# MS42 design — 可组合的模块边界

事实基线：R79（`.agents/analysis/ms42-module-boundary-current-state.md`，revision 1f7dd6e；本分支基点 40e9ee8 仅多两笔 docs 提交）。本文只记本 change 的裁定与契约；事实面细节引用 R79 条目，不复制。

## 1. 目标分解与实现顺序

MS42 的三件事在本 change 的承载：

| Scope 条目 | 承载物 | 实现顺序 |
|---|---|---|
| CLI、网络、加密可选依赖 + 所选组合构建与行为验证 | features 表、cfg 门、测试面适配、组合矩阵验证（T1–T5） | 先行——可编译验证的真实边界 |
| 六层公共接口与状态归属 | 本文件 §5 契约（收尾作 project-model Mxx 候选移交） | 事实来自 R79，随实施结果在收尾核对（T6） |
| 模块向统一诊断报告信息的接口约定 | 本文件 §6 契约 | 纯约定，不落代码（§6.6） |

顺序理由：feature 组合是唯一能产生新验证证据的边界工作；两份契约是记录性成果，依赖实施后的代码事实做收尾核对，放在收尾 Iteration。

## 2. Feature 方案

### 2.1 features 表与依赖归属

```toml
[features]
default = ["cli", "network", "crypto"]
cli = ["dep:clap", "dep:clap_complete", "dep:csv"]
network = ["dep:tokio-util"]
crypto = ["dep:argon2", "dep:aes-gcm"]

[[bin]]
name = "rtsql"
path = "src/main.rs"
required-features = ["cli"]
```

clap、clap_complete、csv、tokio-util、argon2、aes-gcm 转为 `optional = true`。其余运行时依赖不可选，归属：tokio/async-trait/thiserror/serde/serde_json/dashmap/rand 跨边界（R79 §3.4）；crc32fast 属 wal、sqlparser 属 parser，其宿主模块本批不可选；`rand` 有三个非加密产品使用点（file_storage 盐、pg_protocol、cli sample），必须随核心保留。

### 2.2 门接缝清单

```text
src/lib.rs        pub mod network  ← #[cfg(feature = "network")]
src/lib.rs        pub mod cli      ← #[cfg(feature = "cli")]
src/main.rs       （内容不变；经 [[bin]] required-features 随 cli 门进出构建图）
src/storage/mod.rs    mod crypto   ← #[cfg(feature = "crypto")]
src/storage/file_header.rs  KNOWN_FLAGS_MASK = FLAG_ENCRYPTED（crypto 开）/ 0（crypto 关）
src/storage/file_storage.rs 加密分支（cipher 字段、initialize/validate 的密钥臂、
                             页读写加密臂、零页加密臂）按 feature 门控
tests/*.rs        104 个 network 锚定文件 + 44 个 bin 锚定文件（33 重叠）按 §2.3 规则加门
tests/crypto_off_build_test.rs   新增，文件级 #![cfg(not(feature = "crypto"))]
```

network 是叶子模块（src 内除自身零引用），门后无 src 侧其他改动；`tokio-util` 仅 `src/network/server.rs:10` 使用。CLI 侧无任何测试 import `rtsql::cli`，bin 经 `required-features` 门即可。

### 2.3 测试面适配规则（确定性，Act 按此枚举）

规划时实测（40e9ee8）：104 个文件含 `use rtsql::network::`（其中 97 个用 `network::protocol::{to_response, Response}` 断言词表，7 个为协议/服务专项或函数内 import——`executor_test.rs:497` 是唯一的函数内形态）；44 个文件含 `env!("CARGO_BIN_EXE_rtsql")`；二者重叠 33 个。

1. 文件同时含 network import 与 bin 宏 → 文件头 `#![cfg(all(feature = "network", feature = "cli"))]`。
2. 仅 network import → `#![cfg(feature = "network")]`；仅 bin 宏 → `#![cfg(feature = "cli")]`。
3. `executor_test.rs` 的函数内 `use rtsql::network::Response` → 该测试函数加 `#[cfg(feature = "network")]`，文件其余用例不门（文件级门会误伤非 network 用例）。
4. 门只加文件头/函数属性，不改任何断言与用例本体。
5. 执行时以实际 grep 重新枚举（规则同上），计数与规划值差异记入 Act Response；不因计数漂移改变规则。

`#![cfg(...)]` 内层属性在条目与宏展开前剥除整个文件，因此 `env!("CARGO_BIN_EXE_rtsql")` 在 bin 缺席的组合不会展开；integration test 与被测包共享 feature cfg 集，`cfg(feature = …)` 在 tests/ 有效。

### 2.4 备选方案对比

| 决策点 | 选定 | 备选 | 弃用理由 |
|---|---|---|---|
| network 测试锚定 | 文件级 cfg 门 | 迁移 97 文件断言到 `SqlOutcome` 锚定 | 97 文件大机械改写在默认组合引入不必要回归面；锚定迁移属独立重构，无本批需求依赖它 |
| crypto 关闭形态 | cfg 门控加密分支 | 保留 `PageCipher` 空类型桩使签名恒定 | 桩是无行为死代码；具体形态（分支门控的粒度）属非实质实现选择，留给 Act |
| 死依赖 lru/anyhow | 移除 | 保留或留注释 | 见 §4 |
| 层边界表达 | 契约记录（§5） | `pub`→`pub(crate)` 收窄 | tests/ 以深路径 pub 锚定（database:: 124 文件、network:: 97、storage:: ~60），收窄等比例破坏测试编译面；收紧属独立重构 |
| 依赖环 | 保留并记录处置 | 本 change 解环 | R25 估返工以周计；MS42 明示「先在模块内验证边界，有实际收益再拆 crate」；feature 组合不需要解环 |

### 2.5 组合矩阵（所选组合）

| 组合 | 构造 | 构建门（均为 `cargo check --offline` 前缀） | 行为验证 |
|---|---|---|---|
| C0 默认 | `--features default`（即不加参数） | `--lib --bins --tests` | 收尾 M19 分批全量 + fmt + deny clippy |
| C1 no-network | `--no-default-features --features cli,crypto` | `--lib --bins --tests` | `--lib` + `--test storage_test --test parser_test --test executor_test` |
| C2 no-crypto | `--no-default-features --features cli,network` | `--lib --bins --tests` | `--lib` + `--test crypto_off_build_test --test storage_test --test parser_test` |
| C3 no-cli | `--no-default-features --features network,crypto` | `--lib --tests` | `--lib` + `--test storage_test --test parser_test --test executor_test` |
| C4 core-only | `--no-default-features` | `--lib --tests` | `--lib` + `--test storage_test --test parser_test` |

选定理由：三个单关组合各自隔离一个 feature 的关闭语义（诊断边界独立）；全关组合覆盖任意两 feature 同时缺失的构建面，故不再验证更多排列。`storage_test`/`parser_test`/`executor_test` 属「无 network import、无 bin 宏、无加密正例」的常可用目标池（157 − 115 = 42 个文件，规划时实测），覆盖存储、规划、执行三域。

定向单目标验证用默认并行度（M19 只约束全量）。`cargo test <组合> --lib` 在四个非默认组合各跑一次（单元测试目标，允许并行）。benches 与 examples 不进组合门（`cargo test` 不构建 harness=false 的 bench；`examples/bench_minimal.rs` 只用 `database::Database`，不受门影响）。

### 2.6 兼容面

install.sh 与 build-riscv64-musl.sh 不传 feature flag，走默认组合，行为不变。`Cargo.lock` 在 T1 首次构建时随依赖增删更新（该次命令不带 `--locked`），其后验证命令可恢复 `--locked`。

## 3. crypto 关闭组合的错误语义裁定

| 情形 | 行为 | 依据 |
|---|---|---|
| 打开 `FLAG_ENCRYPTED` 置位的库（任何密钥参数） | `KNOWN_FLAGS_MASK=0` → decode 在密钥分支之前以 `UnknownFlags(1)` 拒绝 → 既有 `IncompatibleHeader`（消息含 `unknown feature flags`） | 复用现成 unknown-flags 前向防护（`file_header.rs:126-129`、`header_rejection`），零新错误路径 |
| 明文库携带密钥打开（`Some(key)`） | 锁获取之后返回 `StorageError::InvalidKey`，消息点名本构建未包含加密能力（区别于 crypto 开启组合的 `database is not encrypted`） | 复用既有密钥错误通道与 CLI 退出码 5；`database-encryption` 域的明密互斥合同在关闭组合退化为「一律拒绝携带密钥」 |
| 0 字节路径携带密钥新建 | 同上 `InvalidKey`，且发生在任何格式头写入之前（可能残留 0 字节空文件，与既有打开失败路径同类） | 拒绝先于 `initialize_header` 写头 |
| 锁序 | 拒绝点位于 `initialize_header`/`validate_header` 内（构造器在持锁后调用），锁冲突仍优先于密钥错误 | 既有顺序守卫（database-encryption R2） |

`FLAG_ENCRYPTED` 常量在两种组合都保留（格式位定义，非加密能力）；`file_header.rs` 单测中的加密正例（roundtrip、KDF 参数校验）与 `file_storage.rs` 单测中的加密用例加 `#[cfg(feature = "crypto")]` 函数级门。`rand` 不随 crypto 可选（§2.1）。

## 4. 依赖处置裁定（I093 构建配置部分）

- `lru`、`anyhow`：src/tests/benches/examples 全仓零引用（R79 §3.4 + 本次复测）。移除。理由：MS42 要求「依据实际调用……形成所选组合的构建验证」，死依赖无法归属任何 feature，保留会使依赖归属边界结论失真；移除零行为影响（无可达代码路径）。属 MS42/I093 范围内的构建配置修正，不是无对象替换。
- `futures`：仅 `tests/concurrent_test.rs` 使用（4 处 `join_all`）。移入 `[dev-dependencies]`，等价零行为差。
- 不做：无理由的版本升级、其他依赖的可选化（§2.1 归属表）。
- 附带移交：SNAPSHOT「技术栈/并发原语」提到 lru、benches 计数过时——属 docs-maintainer 日常修正，收尾时作为移交信息报告，不在本 change 修改。

## 5. 六层公共接口与状态归属契约

（收尾核对后作为 project-model Mxx 候选移交 docs-maintainer；MS43 直接依赖本节。）

### 5.1 层构成与公共接口（按实际调用，R79 §1.2）

| 层 | 模块 | 对外公共接口（其他层实际消费） |
|---|---|---|
| 入口 | `cli/`、`network/`、`main.rs` | CLI：进程退出码与四格式输出合同（经二进制锚定）；lib 面 `rtsql::cli::run()` 仅供 bin 装配。network：`Server::new/shutdown_token`、`network::protocol::to_response`（CLI/Server 边界转换词表，97 个测试文件用作断言词表）、PgProtocol/JsonProtocol。入口层消费核心与各层 pub 面，不被任何层引用（network、cli 均为叶子） |
| 核心 | `database.rs`、`pipeline.rs`、`plan_cache.rs`、`result.rs`、`profiling.rs` | `Database` open 家族与显式事务 API、`checkpoint()`、`prepare`/`PreparedStatement`、`set_query_memory_limit`；pipeline 的 `parse_stage`/`plan_stage[_routed]`/`execute_stage[_in_tx[_with_params]]` 族与 `AttachRoute`/`RoutedTables`、`infer_ctas_column_types`（pub(crate)，CLI 复用）；`SqlOutcome`/`SqlError`/`ColumnMeta`（result，lib 根重导出）；profiling 的 `with_profiling_scope`/`is_profiling_enabled`；`PhysicalPlan`（定义在执行层、lib 根重导出，核心-规划-执行共同词表） |
| 规划 | `parser/`（含 planner 七模块） | `parse_sql`、`PlanBuilder`、`PlanError`（含 SQLSTATE 分类）、`classify_attach_statement`/`classify_transaction_statement`/`AttachInfo`/`TxStatementKind`（CLI 消费）、二级索引名名单注入面 |
| 执行 | `executor/` | `Value` 值词表（存储侧消费面按引用文件计：3 个产品文件——catalog.rs、data/table_manager.rs、page_format/tuple.rs，另有 1 个测试文件 btree/index_manager.rs；result/network/cli 消费）、`ColumnType` 与 `datetime` 编码（存储/pg 协议消费）、`Executor` trait 与 `next_batch`、`query_budget` 预算面、32+ 执行器与 `ExplainExecutor`（EXPLAIN 计划树渲染，由 core 层 pipeline 消费；渲染模块 `plan_format` 为执行层私有，不属对外公共面） |
| 存储（含持久化） | `storage/`、`wal/` | `AsyncStorage` trait（`Arc<dyn AsyncStorage>` 持有点 database.rs:332——存储后端替换接缝）、`BufferPool`、`TableManager`/`TableMeta`、`IndexManager`、catalog 词表（`CatalogRow`/`CatalogColumnRow`/`CatalogKind`）、页格式词表（`ColumnType`、`RowId`、tuple 编码）、`check_integrity`、`StorageError`；wal 的 `WalWriter`/`WALBuffer`/`CheckpointManager`/Recovery |
| 事务 | `transaction/` | `TransactionManager`（begin/commit/abort 唯一 WAL 事务记录源）、`TransactionId`、`Snapshot`/`IsolationLevel`、`VersionHeader`（数据页格式逐字节消费）、`TransactionSession`、`abort_by_id`/`discard_tx_version_tables`（取消回收面） |

### 5.2 状态归属（R79 §4 + 裁定）

所有权裁定：**会话级可变状态统一归 `Database` 协调器所有**（`Arc` 共享、克隆共享全部可变态），具体字段清单与读写层见 R79 §4.1（字段数以 `pub struct Database` 的结构体字段为准：15 字段——buffer_pool、table_manager、transaction_manager、wal_writer、wal_buffer、plan_cache、schema_version、checkpoint_manager、isolation、views、mode、query_memory_limit、io_failpoint、implicit_transactions、path）。要点裁定：

- `plan_cache` 读写独占 pipeline；`schema_version` 单点递增在 pipeline `bump_schema_version`，database 5 处消费（预编译重计划）；克隆共享、open 归零、不持久化。
- `wal_buffer` 是唯一「三层持有入口」的字段（执行器写路径直接追加 + TransactionManager 内部槽 + CLI shutdown）——归属仍为 Database；MS43 改造此对象的协作时以本清单为起点。
- `views` 注册表归 Database；planner 持有的是注入快照副本（非共享 Arc）。
- `io_failpoint` 打开期定型，主库 FileStorage 与 WalWriter 共享同一单元；ATTACH 附加库各自独立。
- 会话级后台对象（`WALBuffer` 后台 flush/Notify、`WalWriter` 句柄与水位、`TransactionManager` 内部登记、`BufferPool` 并发结构、IndexManager 写侧闸门）归属其宿主管理器；生命周期随 `Database` 实例。
- **ATTACH 会话状态归入口层 CLI**（`AttachSession`，cli/mod.rs:756，注释明确「刻意不放在 Database」）——现存跨层状态归属先例：会话概念归入口，逐语句路由归核心。

### 5.3 依赖方向规则与环清单

允许方向：入口 → 核心 → {规划, 执行} → {存储, 事务, 持久化}；规划 → 执行（计划词表供给）；result/profiling 为零环跨切面（result 依赖 executor/storage 词表，profiling 零依赖）。

现存环（保留，处置为拆 crate 前置约束；**本 change 起新增代码不得引入新的跨层环边**）：

| 环 | 边与位置 | 拆分前置 |
|---|---|---|
| 环 1a | database→pipeline（execute 族，database.rs:537-1013）→executor（工厂）→database（13 文件持具体类型：DDL 执行器族 + 子查询/Join 配置族） | executor 对 `Database` 的依赖面收敛为trait 或参数包 |
| 环 1b | executor↔pipeline（`RoutedTables` 12 + `create_executor_from_plan` 5：子查询重建臂 5 文件） | 子查询重建臂的装配协议专项设计（R25 遗留 `PhysicalPlan` 表示与装配边界） |
| 环 1c | database↔transaction（transaction/session.rs:9 产品代码引 `Database`） | `TransactionSession` 执行臂注入方式 |
| 环 2 | storage↔transaction（`VersionHeader` 嵌入页格式 5 文件 6 处；反向 `page_format`/`RowId`/`ColumnType`） | `Value`/`ColumnType` 词表归属下沉（R25 第一刀） |

词表归属遗留决策（R25 遗留、本 change 只更新消费面数字不改归属）：`Value` 在 executor/value.rs（存储逐字节依赖）、`ColumnType` 在 storage/page_format/tuple.rs 与 executor/value.rs（两个同名类型分属存储与执行两层；lib 根重导出执行层版 `rtsql::ColumnType`，存储层版为 `rtsql::storage::ColumnType`）、`PhysicalPlan` 在 executor/plan.rs、`datetime` 在 executor/datetime.rs。归属调整属 crate 拆分 change 的专项设计。

## 6. 诊断报告接口约定（面向 MS43 衔接、MS44 实现）

### 6.1 现状出口清单（迁移基底）

- 34 个 `eprintln!`/`println!` 直写出口（排除 cfg(test)；文档注释中的形态引用不计调用点）：table_manager 12（DROP/TRUNCATE 释放 best-effort 告警）、wal/recovery 6、profiling 5（计时表）、btree/index_manager 4、network/server 3、executor/drop_index 2、data_page 1、database 1。
- 既有计时设施：`profiling.rs`（task_local + `RTSQL_PROFILING` + `with_profiling_scope`），产品接线 4 处（pipeline 三段、database 1、两个索引算子）。
- 错误通道：库公共面单漏斗 `SqlError { message, sqlstate }`；SQLSTATE 分类三源（parser/error.rs:145、pipeline.rs:1673-1686、network/pg_messages.rs:266）；各层 thiserror 三个 error.rs。

### 6.2 约定正文

1. **统一门面**：后续统一诊断经单一门面模块（约定名 `rtsql::diagnostics`）收发；各模块 SHALL NOT 再新增 `eprintln!`/`println!` 直写出口（自本 change 起对新代码生效）。
2. **数据模型**：`DiagnosticRecord { severity（分级枚举，名称 MS44 定）、category（模块/阶段标识）、message、data（可选键值：耗时、资源计数） }`；事件记录与性能统计两类；阶段计时沿用 `with_profiling_scope` 的 scope 语义（阶段 + 算子两级）。
3. **交付 API（草案签名）**：`diagnostics::report(record)`（同步投递、门面决定丢弃/缓冲/输出，模块不得自持输出）与 `diagnostics::scope(name) -> ScopeGuard`。MS44 实现时可收敛参数与返回形态，但不得把输出决策移回模块。
4. **职责边界**：模块产生自身事件与统计（模块自己持有阶段语义）；门面独占配置（详细程度/观察模块/采集内容三分离，对齐 I096）、跨异步上下文关联与输出限量。诊断只观察，不改变事务结果、不替代行为断言。
5. **迁移原则**：34 个既有出口由 MS43/MS44 按域增量迁移；「用户可见 stderr 告警」的有无变化属行为变化，须由承载 change 的 spec 增量明示，不得借诊断名义静默吸收。profiling.rs 并入门面作计时提供方，4 个接线点语义保持。
6. **与错误通道的关系**：错误继续走 `Result` 返回值（错误不是诊断事件）；SQLSTATE 三源分类本 change 零变化。

### 6.3 本 change 不落该接口代码的理由

无消费者（MS43/MS44 未开工）：门面骨架是死代码，且无可观察行为可先行测试，违反「验证只证明目标行为」与最小必要实现。契约经本 design 归档并在收尾作为 Mxx 候选移交，MS43/MS44 的 planning 可直接引用本节，无需重做约定。

## 7. 验证设计

- 逐任务：各 Task Definition 的 First test / GREEN / Verification（见 Cycle 与 tasks.md）。
- 组合验证集：§2.5 矩阵。定向目标集是常可用池中的域名代表（存储 `storage_test`、规划 `parser_test`、执行 `executor_test`）；组合验证不重跑全量。
- **全量预算（整个 change 一次）**：Iteration 003、默认组合、M19 分批串行 + MS36 D10 等分批次规则（`B = ceil(文件数/12)`，前 `B×12−N` 批各 11、其余各 12，`--lib --bins` 单独成批）；逐批记录 passed/failed/ignored 与三项对账。非默认组合不承诺全量绿（crypto 域测试在 C2 预期失败，不入组合验证集）。
- 质量门：默认组合 `cargo fmt --check` 与 `cargo clippy --offline --all-targets -- -D warnings`；core-only 组合补 `cargo clippy --offline --no-default-features --lib --tests -- -D warnings`（剥空与门控不得在关闭组合产生告警）。沿用 MS41 命令面。
- 基线沿用：MS41（Linux 本机质量检查、分批全量）结论在覆盖未变化下直接沿用（goal 约束 3）；本 change 各任务以定向命令建立当次 GREEN，不重跑改前全量。
- 无身份证明机制：判定只看 cargo 退出码与原生 `test result:` 行；不新增脚本、封装命令或判定层。

## 8. Issue 承载机制（goal 指令优先于 M20 另建 change 默认）

实施中发现的实质缺陷：Act 只报告（Act Response 的 Issue 候选）→ 调度器派发 recorder 登记 Issue → 调度器转交后，由 Plan 在本 change 的 Iteration Map **新增 Iteration** 承载解决（每个新增 Iteration 保持独立验证与诊断边界）。不预设空 Iteration；收尾前 Map 中的全部 Iteration（含新增）accepted 才能收尾。范围外但与模块边界无关的缺陷同样走此通道（goal 原文「中途发现的issue全都登记后纳入当前change计划新iter解决」）。M20 的「发现方只报告不扩范围修复」仍适用：登记与承载由调度器/Plan/Act 分工完成，Act 不在当前 Cycle 顺手修。

## 9. 风险与非实质开放点

- crypto 门控实现形态（分支门控粒度、cipher 字段处理）留给 Act；边界是「关闭组合无加密代码路径编译进产物，明文路径与默认组合逐字节一致」。
- `KNOWN_FLAGS_MASK=0` 时 decode 的加密分支不可达：Act 可选择重构该分支或保留 cfg 门死分支，不影响行为。
- 测试文件计数（104/44/33）以执行时 grep 为准（§2.3 规则 5）。
- `cargo test <组合> --lib` 时长未知（首次执行记录）；若单组合 --lib 超出可接受时长，按公共规则记录事实并回报，不静默改验证面。
- Cargo.lock 随 T1 更新；如出现与 lockfile 相关的离线解析失败，按实际错误回报，不做依赖版本变更。
