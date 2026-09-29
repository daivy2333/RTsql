# RTsql workspace crate 化与微内核数据库形态 — 探索分析

> Snapshot: [SNAPSHOT](../docs/SNAPSHOT.md)
> Captured revision: 7364bc9（master，其上有 MS13 + MS17-T02 实施 + docs sync 共 92 个未提交文件，均属已收口工作）
> Observed branch: master
> Captured at: 2026-09-24
> See also: [usability-gap-cli-form](usability-gap-cli-form.md)

## 目标与范围

**重心思想**：受 StarryOS/ArceOS 组件化操作系统模型启发，将 RTsql 从单 crate 纵向一体化数据库演进为「近似微内核」的组件化数据库——**workspace crate 划界 + trait 接缝 + cargo feature 组合**三层结构，实现「一个模块一个功能，需要什么就编译带什么 feature 的数据库」。

**定位**：本文档是长期方向（初版 MS17 交付之后）的调查输入，不承诺实施、不进入执行序。用户 2026-09-24 明确裁定：短期完成当前 change（`2026-09-23-ms17-initial-release`），crate 化为初版达成后的意愿方向。

**范围**：本会话新鲜读码的耦合事实（模块依赖图、真实环、词表熔接点、现成接缝）、目标形态草图、四刀迁移顺序推断、可能遇到的问题、外部先例调研（三种模块化模式的开源数据库/引擎，2026-09-24 web 查证），以及同日补充讨论的定位裁定与关联方向登记（I059，见「定位裁定与决策过滤器」「关联方向登记」两节）。

**不在范围**：任何实施计划、change、tasks——将来启动时由 Plan 以本文档为输入重新调查（代码届时必然已变）。

## 定位裁定与决策过滤器（2026-09-24 用户裁定）

**结论**：workspace crate 化是手段，不是身份——多个先例数据库（Materialize/GreptimeDB/RisingWave 等）均已 workspace 化，无人以「拆 crate」为身份。RTsql 的身份锁定为三分句：**异步 · 嵌入式 · CLI 数据库**。

**身份稀缺性依据**（定位主轴的事实支撑）：Rust 嵌入式数据库生态几乎全为同步内核——SQLite/sled/redb/fjall 在 tokio 服务中的通行做法是 `spawn_blocking` 包装；async-native 的完整 SQL 引擎近乎空白（2026-09-24 web 查证，仅 async-skipdb 类内存 KV），而 RTsql 自创立即为 Tokio 协程调度核心。该优势是同步对手结构上无法追赶的（追赶即重写 IO 层）。

**决策过滤器**（候选功能/里程碑逐一过表；三词不沾默认不做或退 improvements 排队）：

| 定位词 | 承诺 | 排除/降级 |
|---|---|---|
| **异步** | 协程调度核心不动摇；spawn_blocking 边界纪律（M09/M13）为受保护不变量；io_uring 后端（I028）是身份完成件而非普通优化 | 同步化捷径实现；「先同步后包一层」的功能 |
| **嵌入式** | 单文件格式稳定、零外部依赖、lib 嵌入面干净（词表 crate 即公共 API） | 守护进程化、集群/分布式叙事、外部服务依赖 |
| **CLI** | CLI 是产品面而非 demo：确定性行为、退出码契约、内省命令、agent SKILL 持续一等公民 | 只面向库开发者优化、CLI 沦为附属品 |

**历史回看验证**：MS10 CLI 全链路、MS13 分析薄命令、MS17-T04 agent SKILL 均为三词加分项；PG 协议降级为库能力、CI/Releases 裁剪均为不沾三词的正确裁剪——过滤器与既有决策史一致。

**对路线 B 的排序影响**：每刀准入判据从「边界值得墙」升级为「这道墙是否创造体现身份的公共接缝」——AsyncStorage 公共异步 API + io_uring 后端 crate（I028 复活为招牌 feature）与词表 crate（嵌入 API 面）优先级最高；net/crypto/cli feature 化次之（「编出你要的数据库」，组合即发行）；纯内部结构墙最低。**反面约束**：不打功能对标战争（窗口函数/JSON/向量赛道 SQLite/DuckDB 碾压，不追）；组合性必须挂性能承诺（对外性能声明前需新鲜 bench，I057 域；332x/5.6x 为 MS00 时代数据仅作历史参考）。

## 关联方向登记（I059，2026-09-24）

用户提出跨数据库文件交互期望：多个 .db 文件经命令行关联检索（跨库 SELECT/JOIN/增删查改）、跨库取视图物化为新库（「数据库视作表」）。已登记 improvements I059——SQLite ATTACH 同型设计（表名命名空间「别名.表」+ 每文件独立快照 + try_lock 结构性消除交叉 attach 死锁），两期拆分：一期只读跨查 + CTAS 物化（CTAS 单独即通用 SQL 增益，`new` + attach + CTAS 组合覆盖物化全场景）；二期跨文件 DML（每文件独立提交、非原子 v1 语义文档化——SQLite 跨 attach 原子提交亦需 master journal 级复杂机制）。

与身份过滤器关系：三词全沾——异步（attach 文件走同一异步扫描路径）、嵌入式（无 daemon 多文件互查的正统设计，FDW/联邦式方案才违反身份）、CLI（一行命令跨库抽取/合并/分析，agent 场景强化）。与 crate 化关系：attach 注册表催生「多文件会话」抽象，对路线 B 顺风而非冲突；排期为初版（MS17）交付后的能力扩展，优先于路线 B 或并行皆可。

## 目标形态（推断，非承诺）

### 三层模型（对照 StarryOS/ArceOS）

ArceOS/StarryOS 的可组合性由三层构成，feature 只是最后一层开关：

```text
第 1 层  crate 边界    axruntime / axdriver / axfs / axnet / axtask ...
                       每子系统一个 crate，编译器强制依赖方向
第 2 层  trait 接缝    axdriver::BaseDriver / axfs::VfsOps / axtask::Scheduler
                       上层只认 trait，不认具体实现
第 3 层  cargo feature axdriver = ["net", "block"]
                       只选实现进最终镜像，不管拆墙
```

对 RTsql，「像 StarryOS 那样」的完整含义是三者齐备；单 crate 内撒 `#[cfg(feature)]` 只得到第三层。

### 微内核概念到数据库的映射

| 微内核概念 | 数据库对应物 | RTsql 现状 |
|---|---|---|
| 内核最小集 | 词表层（类型/协议）+ 执行骨架 | 熔在 executor/storage 内，需切出 |
| 内核态服务 | storage / wal / transaction | 模块存在但相互成环（见下） |
| 用户态服务 | parser / planner / executor | 模块存在，反向依赖内核态 |
| 系统服务进程 | cli / pg / json 协议 / 加密 | cli 与 network 内嵌，net 含错层的 Response |
| 能力开关 | cargo feature（可选件） | 无（全部默认编译进） |
| IPC 接缝 | trait（存储后端/函数注册/执行器） | 部分现成（AsyncStorage/REGISTRY/火山树） |

### 目标 crate 拓扑（草图）

```text
rtsql/
  crates/
    rtsql-types     词表层：Value/ColumnType/VersionHeader/Response 等
                    共享词表——零依赖或仅依赖外部基础库
    rtsql-storage   BufferPool/btree/page_format/catalog/data
                    （依赖 types；AsyncStorage trait 上移或本地化）
    rtsql-wal       WalWriter/WALBuffer/Checkpoint/Recovery
                    （依赖 types、storage trait 面）
    rtsql-parser    sqlparser 适配 + PlanBuilder（依赖 types、executor 计划类型）
    rtsql-executor  执行器树（依赖 types、storage trait 面）
    rtsql-core      database/pipeline/transaction/plan_cache 组装层
    rtsql-net       Server/PgProtocol/JsonProtocol（optional dep + feature）
    rtsql-crypto    KDF/AEAD（optional dep + feature，MS17-T01 产物）
    rtsql-cli       CLI 二进制（optional member）
```

feature 面只承载可选件与薄选项（`net`/`crypto`/`cli`/函数域），**不承载纵向切穿存储格式的伪功能**（`wal`/`mvcc`/`btree` 单独编译掉 = 换一个存储引擎，非裁剪）。

## 已确认事实（2026-09-24 本会话读码，revision 7364bc9 工作区）

### F1 模块依赖图（实测 `crate::` 引用，注释噪音已人工剔除）

```text
database  -> network pipeline plan_cache storage transaction wal
pipeline  -> database executor network parser profiling storage transaction
parser    -> executor
executor  -> database parser pipeline profiling storage transaction wal
storage   -> transaction wal
transaction -> database storage wal (network 仅 tests/session.rs:91)
wal       -> storage transaction
network   -> database executor
cli       -> database network parser pipeline storage transaction
```

### F2 模块级存在真实依赖环（crate 化的硬前置）

- **环 1**：`database -> pipeline -> executor -> database`——5+ 个执行器直接持有 `Database`（`src/executor/{drop_table,create_table,subquery_eval,semi_join,join_related_config}.rs` 的 `use crate::database::Database`）。
- **环 2**：`storage <-> transaction`——数据页格式嵌入事务类型：`src/storage/data_page.rs:5`、`src/storage/data/table_manager.rs:14`、`src/storage/buffer_pool.rs:12` 引 `transaction::VersionHeader`/`Snapshot`；`src/storage/error.rs:5` 引 `TransactionError`；反向 `transaction -> storage` 亦存在。
- **含义**：Rust crate 不允许循环依赖。上述环是 crate 划界前必须断开的结构债，断开方式即「词表下沉」（VersionHeader/Snapshot/Response 等移入零依赖词表层，`Database` 句柄改 trait 或移出执行器构造面）。**断环跨第一刀与第三刀（环 2 词表侧第一刀断、环 1 第三刀断），构成整个迁移的主要工程量。**

### F3 词表熔接点（枚举变体无法 cfg，决定 feature 边界）

- `Value` 枚举：`src/executor/value.rs:50`——Date/Timestamp 等类型变体熔在全库共享值枚举（MS13 全链路实施证明类型变更穿 executor/predicate/tuple/catalog 五层）。**Rust 不支持干净地 cfg 枚举变体**，故 `datetime` 类型域不可作为 feature，只能 gate 函数/命令薄皮。
- `PhysicalPlan` 枚举：`src/executor/plan.rs:18`，约 20 变体（M01 一致）——planner 与 executor 的共享计划词表，crate 化时面临「词表化（进 types）还是 trait 化」的单点架构决策。
- `ColumnType`：`src/storage/page_format/tuple.rs:30`——类型词表与磁盘格式绑定。
- `VersionHeader`：定义在 `src/transaction/`，被 storage 数据页逐字节嵌入（F2 环 2 根因）——**存储格式与事务语义的物理熔接**，也是「mvcc 不是功能」的代码证据。

### F4 现成接缝（可复用的第 2 层）

- `AsyncStorage` trait（`src/storage/async_storage.rs`）——存储后端唯一接缝，`FileStorage` 只是实现之一；与 SurrealDB 的 kv-* 后端 feature 模式同构（见先例调研模式 1）。
- `function.rs` REGISTRY 元数据注册表（3 处核心符号）——标量函数域的 registry 接缝。
- Executor 火山树——执行器天然可插拔组合。
- `IsolationLevel` 枚举（`src/transaction/`）——行为开关已枚举化。

### F5 可见性改造面量化

`pub(crate)` 共 99 处（`grep -rn "pub(crate)" src/ | wc -l`）——crate 切分时每处需重新裁定跨 crate 可见性。规模适中，非阻塞量级。

### F6 Response 错层（第一刀的直接对象）

`Response` 定义在 `src/network/protocol.rs:34`，而 `database.rs`、`pipeline.rs`、`transaction/session.rs`、`cli/` 全部依赖 `crate::network`——核心反向依赖网络模块。`net` feature 化前必须先把它下沉。

### F7 MS17-T01 加密设计 feature-ready

change `2026-09-23-ms17-initial-release` 的加密实现（crypto 独立模块 + transform 封死 FileStorage + `KNOWN_FLAGS_MASK` 语义）可后置为 `crypto` feature 且改造成本接近零——把 crypto 模块与 FileStorage transform 包进 `#[cfg(feature = "crypto")]` 即可；mask=0 的「加密位拒绝」行为恰为 feature 关闭态，行为语料库无需改写叙事。

### F8 无 CI

improvements I051 登记「无 CI workflow」。feature/crate 化的回归矩阵（`--no-default-features` 与关键组合）没有自动执行面（后果展开见边界节第 3 条）。

## 先例调研（外部事实，2026-09-24 web 查证；非本项目代码结论）

市面上的「微内核/组件化数据库」分三种模式，与本项目路线的对应关系：

### 模式 1：编译期模块化（路线 A 的先例）

- **SQLite**：编译期 `SQLITE_ENABLE_*` 宏裁剪（FTS5/RTREE/JSON 等）——编译期可配置数据库的元老。
- **SurrealDB**：`kv-mem`/`kv-rocksdb`/`kv-surrealkv`/`kv-tikv`/`kv-indxdb` 一组 cargo feature 选存储后端，`cargo build --features surrealdb/kv-rocksdb` 即按需构建（查证：docs.rs feature flags、官方博客 Introducing Surreal<Any>）。**与 RTsql 的 AsyncStorage 接缝 + crypto/net feature 设想同构**。
- **DuckDB**：扩展系统（httpfs/parquet/json/icu 等），支持静态/动态链接与自定义扩展集构建。

### 模式 2：crate workspace 化（路线 B 的先例）

- **Materialize**：Cargo workspace，`mz-repr`/`mz-expr`/`mz-storage`/`mz-compute` 等 100+ `mz-*` crate——词表层（repr/expr）独立是其一等公民。
- **GreptimeDB**：workspace 多 crate（`common-*`/frontend/datanode/flownode/metric-engine）。
- **RisingWave**：workspace 多 crate 流数据库（查证：仓库 Cargo.toml）。
- **Databend**：workspace（`databend-common-*`/query/meta）。
- **InfluxDB 3.0 (IOx)**：workspace（`influxdb3_*` 核心 crate 组）。
- **DataFusion**：可组合查询引擎库——`datafusion-expr`/`physical-plan`/`optimizer` 等 crate + `TableProvider` trait 接缝，嵌入方换数据源即可获得完整 SQL 能力。
- **GlueSQL**：「粘性 SQL 库」——执行层与存储完全解耦，存储实现 `Store`/`StoreMut` trait 即得完整 SQL（sled/memory/CSV/JSON 等后端独立 crate）（查证：gluesql.org、GitHub）。**是「trait 接缝换后端」的最纯粹范本**。
- **Neon**：workspace（pageserver/safekeeper/proxy/compute）——控制面组件化。

### 模式 3：运行时插件微内核（进程内/跨进程）

- **PostgreSQL**：扩展架构——Table Access Method（PG12+）/Index AM/FDW/钩子/动态加载，运行时组合能力。
- **FoundationDB**：角色分解（proxy/resolver/sequencer/storage server/tLog）+ 消息通信 + 确定性仿真测试——分布式内核式解耦的工程范本。
- **DBOS**（研究/创业项目）：反向命题「数据库即操作系统」，Postgres 作为内核承载 OS 抽象。

### 关键告诫：matklad《Large Rust Workspaces》（2021-08）

rust-analyzer 作者 matklad 的经验总结（2021-08）：大型 workspace 的 crate 墙有真实成本——feature unification 复杂度、冷构建时间、版本级联升级；其建议倾向「内聚逻辑保留大 crate，workspace 只为必要边界服务」，反对为拆而拆。RTsql 启动路线 B 时以「边界值得墙」为每刀的准入判据，不追求 crate 数量。

## 推断（四刀迁移顺序草图——每一刀独立验收、全量 GREEN→GREEN）

```text
第一刀：词表下沉            Response 出 network（F6）；VersionHeader/Snapshot
  最便宜、其余刀的前置        等事务词表出 transaction 进 types 层
                             -- 断环 2 的词表侧；F3 三枚举的归属判定

第二刀：存储域 crate 化      rtsql-types + rtsql-storage + rtsql-wal
  边界最清晰                  -- 断环 2 的所有权侧；AsyncStorage 上移 types
                             -- 磁盘格式/行为零变化为硬锚

第三刀：语言域 crate 化      parser/executor 拆出；Database 句柄出执行器
  最难（断环 1）              构造面（trait 化或内核态服务化）
                             -- PhysicalPlan 词表化 vs trait 化的架构决策点

第四刀：组装层 + 可选件      rtsql-core 收口；net/crypto/cli 转 optional
  StarryOS 形态达成           deps + features；组合矩阵定型
```

顺序结构性理由：crate 墙是编译器执法，划错改造成本远高于模块内重构——先用模块内接缝验证边界正确，再立墙。

## 边界与失败路径（可能遇到的问题）

1. **断环成本被低估**（F2）：两个环贯穿执行器构造面与数据页格式。`Database` 出执行器意味着 DDL/子查询执行器需要新的内核服务接口——这可能反过来重塑 pipeline 的执行器装配协议，牵动 `M06` 调用链模型的全部下游。
2. **`PhysicalPlan` 归属是单点架构决策**（F3）：词表化（进 types，全 crate 可见）保留简单但把执行细节锁进公共词表；trait 化解锁组合但重写 planner→executor 全部装配。决策错配的返工以「周」计。
3. **测试矩阵 × 无 CI**（F8）：crate/feature 组合（`--no-default-features`、关键组合）的缺陷只在对应组合被构建和运行时暴露；组合无人跑，缺陷无人发现。**路线 B 启动的前置条件应包含 I051（CI）落地或等价的本地组合门纪律。**
4. **spec 语料库条件化**：37 个行为 spec 描述「这一个数据库」。crate 化是纯结构重构（行为零变化，spec 不改），但可选件 feature 化（net/crypto/cli）会让「零回归论证」多出「feature 开/关两态」维度。
5. **可见性重设计**（F5）：99 处 `pub(crate)` 逐一裁定；漏改的表现形式是编译错误（安全）而非行为回归（可控）。
6. **过度拆分**（matklad 告诫）：crate 数量不是目标；每刀以「边界值得墙」准入，必要时停留在两刀后的形态也是合法终态。
7. **时机风险**：初版（MS17）交付前启动会搅浑其验证面——已由用户裁定规避（初版后启动，2026-09-24）。

## 测试、验证入口与影响面

- **基线**：1065 tests pass / 0 failed / 2 ignored（MS17-T02 收口，maintainer 记录，SNAPSHOT current）；clippy/fmt/validate 全 0/PASS。
- **路线 B 的验证范式**：纯结构重构按 TDD 重构纪律执行——每刀前观察全量 GREEN，刀后保持同值 GREEN；磁盘格式/行为零变化是每刀的硬锚（格式相关套件 `file_header_test`/`recovery_*`/`checkpoint_*` 零修改为刀验收的一部分）。
- **影响面**：`src/` 全部 11 个模块 + `Cargo.toml`（workspace 化）+ 99 处 pub(crate)；不影响 `openspec/specs/` 行为语料库正文（结构重构不产生行为 delta）。
- **本会话未运行**：除 grep 事实采集与既有基线采信外，本分析未运行任何构建/测试（纯读码调查）。

## 关键文件

| 事实 | 位置 |
|---|---|
| Response 定义（F6 错层） | [src/network/protocol.rs:34](/home/daivy/projects/RTsql/src/network/protocol.rs#L34) |
| Value 枚举（F3） | [src/executor/value.rs:50](/home/daivy/projects/RTsql/src/executor/value.rs#L50) |
| PhysicalPlan 枚举（F3） | [src/executor/plan.rs:18](/home/daivy/projects/RTsql/src/executor/plan.rs#L18) |
| ColumnType 与磁盘格式绑定（F3） | [src/storage/page_format/tuple.rs:30](/home/daivy/projects/RTsql/src/storage/page_format/tuple.rs#L30) |
| storage←transaction 环（F2 环 2） | [data_page.rs:5](/home/daivy/projects/RTsql/src/storage/data_page.rs#L5)、[buffer_pool.rs:12](/home/daivy/projects/RTsql/src/storage/buffer_pool.rs#L12)、[error.rs:5](/home/daivy/projects/RTsql/src/storage/error.rs#L5) |
| executor→database 环（F2 环 1） | [drop_table.rs](/home/daivy/projects/RTsql/src/executor/drop_table.rs)、[create_table.rs](/home/daivy/projects/RTsql/src/executor/create_table.rs)、[subquery_eval.rs](/home/daivy/projects/RTsql/src/executor/subquery_eval.rs)、[semi_join.rs](/home/daivy/projects/RTsql/src/executor/semi_join.rs)、[join_related_config.rs](/home/daivy/projects/RTsql/src/executor/join_related_config.rs) 各 `use crate::database::Database` |
| AsyncStorage 接缝（F4） | [src/storage/async_storage.rs](/home/daivy/projects/RTsql/src/storage/async_storage.rs) |
| 函数注册表接缝（F4） | [src/executor/function.rs](/home/daivy/projects/RTsql/src/executor/function.rs) |
| 依赖图谱与环的完整清单 | 本文档 F1/F2 节 |

## 未确认项

1. `PhysicalPlan` 词表化 vs trait 化的取舍——需第三刀启动时的专项设计调查（含对 planner 装配协议的实测影响面）。
2. `Database` 出执行器后的内核服务接口形态（trait 注入 vs 服务定位 vs 构造期闭包）——同上，属第三刀设计域。
3. feature 组合矩阵的最小充分集（若启动路线 B）——依赖 CI（I051）落地时的并行预算。
4. 各先例项目（Materialize/GreptimeDB 等）的 crate 划分细节仅经 web 摘要核实，未逐一读其仓库；引用时以「模式存在性」为准，不作为结构细节依据。
