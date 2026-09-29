# 产品可用性与 CLI 形态差距分析

- SNAPSHOT: `.claude/docs/SNAPSHOT.md`（2026-09-05，current；注意其"支持内存模式"描述已过期，见主题 5）
- 采集 revision: `709c85d`（master，2026-09-05；含 MS08-T01/T02 与文档 sync）
- 分支: master；环境: Linux x86_64 WSL2
- 分析日期: 2026-09-05（三轮：① 形态/SQL 面/交互缺口；② 文件模型/初始化/隔离；③ 用户形态决策细化——非交互 CLI + sudo 式密钥 + 集中存储 + 分析能力，主题 7）

## 目标与范围

回答四个问题，为后续 milestone/change 规划提供上下文：

1. 用户今天如何"使用"RTsql？产品形态现状如何？
2. 用户形态期望为 **CLI 数据库**（非 GUI/常驻服务优先）时，距"真正可用"差什么？
3. 距"好用"（日常交互体验）差什么？
4. 对现有 roadmap（MS08 剩余 + MS09）的权重影响与候选规划输入？

范围：入口/网络/解析/计划/执行 surfaces 的能力边界；不含性能微观分析（见 m19-datascan-path.md 与 MS08 bench 体系）。

## 主题 1：当前产品形态与使用方式

### 已确认事实

- **唯一二进制是硬编码 demo**（`src/main.rs:10-27`）：固定打开 CWD 下 `rtsql.db` → 尝试建 `test` 表（`.ok()` 吞错）→ 监听硬编码 `127.0.0.1:9876` → `Server::run()`。无 CLI 参数解析（Cargo.toml 无 clap 等依赖）、无环境变量、无 REPL、无 one-shot 执行、无内置客户端。
- **两条真实使用路径**：(a) 作为 lib crate 依赖，调 `Database::open/execute_sql/query`（`src/lib.rs` 公开 9 个模块 + 常用类型 re-export）；(b) 起 binary 后用外部 `psql` 连接。
- **PG 服务端**（`src/network/server.rs:33-69`）：Simple Query Protocol 3.0；TCP_NODELAY；`CancellationToken` shutdown（`server.rs:61`）但 **main.rs 未接信号处理**——Ctrl-C 直接终止进程，`Database::close()` 的 flush/checkpoint 不触发（数据安全由 WAL redo 兜底，恢复已验证，代价是重启 redo 变多）。
- **无鉴权**：`pg_protocol.rs` startup 无条件 `AuthenticationOk`（`user` 参数被忽略）。本地单用户可接受，但服务形态不可用。
- **结果渲染完整**（`pg_messages.rs:133-180`）：DataRow 文本格式正确（Int/Float `to_string`、Bool t/f、Null 长度 -1）——psql 显示兼容。
- **错误渲染单一**（`pg_protocol.rs:306-308`）：所有错误 → `severity=ERROR, code=58000`（internal_error），message 透传 Rust 错误链。
- JSON 协议（`protocol.rs:54+`）为 M8 前遗留，server 未接线，仅测试引用（`tests/e2e_test.rs:4` 注释确认切换历史）。

### 结论（问题 1）

RTsql 当前是"**嵌入式库 + 演示性 PG 服务**"，不是可分发的 CLI 工具。一个非开发者今天无法以任何合理方式使用它：没有可参数化的启动方式，没有交互 shell，"客户端"是借用 psql。

## 主题 2：SQL 语义广度（CLI 用户会撞到的面）

### 已确认事实

- **语句面 6 种**（`src/parser/planner/mod.rs:90-93` match 臂）：`CREATE TABLE / INSERT / UPDATE / DELETE / Query(SELECT) / DROP`，其余一律 `PlanError::UnsupportedStatement`（无上下文说明）。**没有**：
  - `BEGIN/COMMIT/ROLLBACK/START TRANSACTION` —— 显式事务只有 Rust API（`Database::begin/commit/rollback/execute_in_tx`，MS07-T04 落地）；**SQL 文本层无法开事务**
  - `ALTER TABLE`、`CREATE INDEX/DROP INDEX`（索引仅 PK 自动建）、`CREATE VIEW`、`TRUNCATE`、`EXPLAIN`
- **表达式面**（`src/parser/planner/expression.rs`，fallback `_ => Err(PlanError::UnsupportedExpression)` 于 `expression.rs:75`）：`Identifier / CompoundIdentifier(两段) / Value / UnaryOp / BinaryOp / Nested`。WHERE 中**不支持** `IN / BETWEEN / LIKE / CASE / IS NULL / CAST / 标量函数`（`subquery.rs:207-310` 的 Between/InList/Case 命中仅在相关性分析 `has_outer_refs_outside`，非构建路径）。
- **聚合仅 5 个**：COUNT/SUM/AVG/MIN/MAX，单列参数（`planner/aggregate.rs:54-84`）。无标量函数（`Expr::Function` 仅聚合路径处理）。
- **类型系统 4 类**（`page_format/tuple.rs:24-33`）：`Int(i64) / String(u16 字节上限) / Float(f64) / Bool`；`Value` 含 `Null`（`executor/value.rs:46-57`）。无 DATE/TIMESTAMP/DECIMAL/BLOB/NULL 约束类型标注。
- **SELECT body 仅 `SetExpr::Select`**（`parser/ast.rs:17-19`）：无 UNION/INTERSECT/EXCEPT/VALUES。
- **多语句静默截断**：`parse_stage` 返回 `Vec<Statement>`（`pipeline.rs:42-48`），但执行只取 `statements.first()`（`pipeline.rs:237`、`341`）——**第二条起被静默丢弃，无任何报错**。对 CLI 脚本执行是直接脚枪。
- 方言：`GenericDialect`（`ast.rs:9`）。

### 推断

- JOIN 有实现（`tests/join_test.rs`、executor 集合含 Join/SemiJoin/AntiJoin），但 FROM 子句支持范围（ON 条件形式、多表链）未逐行确认。
- `ColumnConstraint`（lib.rs re-export）存在，PRIMARY KEY 用于建索引；NOT NULL/DEFAULT/UNIQUE 的支持度未逐条确认。

## 主题 3：距"好用"的差距清单（CLI 形态目标下）

按"没有它就不能用"→"有了它才顺手"排序：

| 层 | 缺口 | 现状锚点 | 实现面评估（推断） |
|---|---|---|---|
| 入口 | `rtsql <file.db>` 参数化启动；本地模式（不开 TCP）与 serve 模式选择 | `main.rs` 硬编码 | 薄：clap + 参数接线，改 `main.rs` |
| 前置 | **跨进程文件锁**（独占打开已被占用的 db 文件并报错） | `file_storage.rs` 无 flock，仅进程内 Mutex（`file_storage.rs:93/110`） | 小：打开后加 advisory lock（flock/fcntl），独占失败 → 明确错误 |
| 入口 | REPL（readline/多行输入/历史） | 无任何 stdin 读取代码 | 中：新 `cli/` 模块；执行后端复用 lib（不经网络） |
| 正确性 | 多语句脚本执行（分号分片或全量执行 + 明确报错） | `pipeline.rs:237` first() | 小：pipeline 层循环或 CLI 层预分片 |
| SQL | 事务语句 `BEGIN/COMMIT/ROLLBACK` | Rust API 已有（MS07-T04），SQL 层缺失 | 小-中：planner 加 3 个语句臂映射 `begin/commit/rollback`；pipeline 事务态接线 |
| SQL | `IS NULL / IN / LIKE / BETWEEN`（日常过滤四件套） | `expression.rs:75` fallback 报错 | 中：expression builder 每个独立可验收 |
| SQL | `CASE`、标量函数（`upper/length/abs/coalesce...`） | 无 | 中-大：函数注册机制是新面 |
| SQL | `ALTER TABLE ADD/DROP COLUMN`、`CREATE INDEX` | 无 | 中-大：触及 catalog/序列化/重建路径 |
| 交互 | 元命令（`.tables/.schema/.mode csv/.import/.dump/.quit`） | 无 | 小-中：REPL 层实现；`.dump` 需语句反序列化 |
| 交互 | 错误信息可操作化 | `UnsupportedStatement/Expression` 无上下文；PG 统一 58000 | 小：PlanError 携带特性名；错误码分类 |
| 交互 | 优雅停机（Ctrl-C → checkpoint/flush） | `CancellationToken` 已有，main 未接 | 小：`tokio::signal` 接线 |
| 兼容 | PG Extended Query（ prepared statement） | MS09-T03 planned | 对内置 CLI 形态**优先级下降**（REPL 不需要）；对 psql 生态工具仍有价值 |
| 数据 | 导入导出（CSV/SQL dump） | 无 | 中 |
| 类型 | DATE/TIMESTAMP/DECIMAL/BLOB | 无 | 大：改序列化格式，"好用"深水区，应靠后 |

## 主题 4：可用性地基与规划输入

### 地基现状（支撑"可用"判断）

- **崩溃安全已验证**：WAL + group commit + recovery + checkpoint（位点消费 + 重写截断）共 20+ 测试（`recovery_e2e_test.rs`、`checkpoint_redo_reduction_test.rs` 9、`wal_*` 等）。
- **性能**：MS08-T01/T02 完成；`before-MS08-T01/T02` criterion 基线体系已建立（R17 Runbook）。剩余项对 CLI 单机用户的可见收益排序（推断）：T06 fsync 合并（写吞吐）> T03 writev > T04 RowLock/T05 Varint。
- **已知热点**：`BufferPool::evict_one` 锁范围跨脏页写回（I 候选待授权登记）。
- **MS09 现有内容**：Read Committed（T01）、NLJ/Hash 切换（T02）、PG Extended（T03）、子查询缓存（T04）。

### 用户形态决策对 roadmap 的影响（规划输入，非决定）

用户明确"CLI 数据库而非 UI/服务端优先"，且确认产品方向为**双层补齐：缺陷层（引擎剩余）+ 应用层（怎么用）**。据此：

1. **候选新 MS："CLI 非交互命令面"（应用层主轨，2026-09-05 第三轮口径修订）**——用户明确**非交互优先（面向 agent/脚本），REPL 降为后续可选**：参数化入口与名称解析 + `rtsql <db> <sql>` 主命令 + 生命周期子命令 + 多语句执行修复 + 跨进程文件锁 + 优雅停机 + 输出格式（table/json/csv）+ 退出码分类 + 错误信息。详见主题 7。
2. **SQL 事务语句 + 过滤表达式四件套**应提前（CLI 可用性直接前提）；SQL 事务复用 MS07-T04 API，实现面小，收益大。
3. **MS09-T03（PG Extended）优先级下降**：非交互 CLI 不依赖协议扩展；psql 兼容可延后。server 代码停止投入，保留为库能力（将来可选 `rtsql serve` 子命令复活）。
4. **MS08 剩余项**保持但让位于 CLI MS 之后执行；T06 fsync 合并对写吞吐最有感，可并行评估。
5. 类型系统扩展（DATE/DECIMAL/BLOB）是独立深水区（格式变更），且是分析能力（日期函数）的底座，排在函数库之前评估。
6. **多用户隔离**：用户已决策不做多用户；代之以**单密钥机制**（主题 7）。数据库级/用户级权限体系属服务形态需求，不建。

### 现有测试与验证入口

- 交互面：`pg_protocol_test.rs` / `pg_integration_test.rs` / `network_server_test.rs`（协议层绿）；**无任何 CLI/REPL 测试**（不存在对应模块）。
- 语句面：`parser_test.rs` 29、`executor_test.rs` 39、`plan_exec_test.rs` 等。
- 回归总入口：`cargo test --all`（585 passed，2026-09-05）。

## 主题 5：文件模型与初始化（CLI 形态的地基事实）

### 已确认事实

- **文件对模型**（`src/database.rs:28-53`）：`Database::open(path)` 打开/创建恰好两个文件——主文件 `<path>`（`FileStorage::open`，create+truncate(false)，4KB 页对齐校验）+ WAL `<path>` 扩展名替换为 `.wal`（`wal/writer.rs:27` `with_extension("wal")`：`rtsql.db` → `rtsql.wal`；无扩展名路径则追加）。
- **主文件自包含**：页 0 = `__tables`、页 1 = `__columns`（保留 catalog 页，MS07-T01），其后为数据页（SlottedPage `next_page_id` 链）、B-Tree 索引页、free-list 空闲页。schema/表/索引/数据全在一个文件。
- **初始化流程**（`table_manager.rs:115-126` → `catalog.rs:92-124`）：`storage.page_count() == 0` → `Catalog::bootstrap`（按序分配页 0/1，初始化为空 SlottedPage page_type 0x03，`flush_all` 落盘）→ 空库；否则 `Catalog::open`（绑定已有页 0/1）→ `open_or_init` 扫 `__tables`/`__columns` 重建内存 TableMeta（含 `IndexManager::from_root`）。**新库初始化 = 打开即完成，无显式 init 命令**。
- **关闭语义**（`database.rs:169-183`）：`close()` = 全量 checkpoint（脏页 flush + 写位点 + WAL 重写截断）。`database.rs:170` 注释明确警告：**drop 后不 close 直接重开 → 脏 catalog 页未落盘 → 重开见空 schema**。进程被 kill 的数据安全由 WAL redo 兜底（恢复已验证），代价是重开 redo 变多。
- **无内存模式**：全仓无 `:memory:` 处理；SNAPSHOT 的"支持内存模式"描述已过期（`Database::open` 永远落盘）。
- **无跨进程文件锁**：`file_storage.rs` 仅进程内 `Mutex`（free_pages），无 flock/fcntl。**两个进程（或同进程两个实例）同时打开同一文件 = 两个独立 BufferPool 写同一组页，结构上必然损坏**。同进程顺序重开的危险已有官方注释；跨进程并发无任何防护。

### 对 CLI 形态的推论

- `rtsql <file>` 打开不存在路径 = 静默创建空库（现状行为，sqlite3 同款）——CLI 需决定是否提示"已创建新数据库"。
- CLI 停机必须 `close()`（信号接线），既为 checkpoint 也为 catalog 完整性。
- 独占文件锁是 CLI 化的前置项，不是增强项。
- 文件对（.db + .wal）是移动/备份的最小单位；只拷贝 .db 不拷贝 .wal 在 checkpoint 后等价、否则丢尾部事务——CLI 的备份/导出元命令应以逻辑导出（.dump）为首选。

## 主题 6：多数据库与多用户隔离现状与设计空间

### 已确认事实

- **多数据库隔离 = 文件边界**：无 `CREATE DATABASE`/`USE`/跨库查询；catalog 只有表级命名空间。两个库 = 两个独立文件对，唯一共享的是代码与页格式。
- **同进程多库**：可同时 `open` 多个 `Database` 实例（各自 BufferPool/WAL），互不可见、互不感知——不同文件安全，**同一文件不安全**（见主题 5 锁缺失）。
- **多用户隔离 = 无**：PG 层无条件 `AuthenticationOk`（`pg_protocol.rs` startup），无用户/角色/权限概念；事务隔离（`transaction/snapshot.rs:11-53`）是**语句/事务级 MVCC 快照隔离**（经典 SI 规则：已提交 + create_tx ≤ 快照位 + 不在活跃集），与"用户"无关。每条 `execute_sql` = 一个隐式事务（单语句快照）；显式事务 API（MS07-T04）提供事务级快照。无可配置隔离级别（MS09-T01 计划 Read Committed）。

### 设计空间（CLI 形态下的建议边界，供规划）

| 层 | 现状 | CLI 阶段建议 | 服务形态（将来可选） |
|---|---|---|---|
| 多库隔离 | 文件边界 | 维持文件边界；`rtsql <file>` 路径即库；REPL 可加 `.open`/`.databases` 元命令 | 同左 |
| 跨进程互斥 | 无 | **advisory 独占锁（前置项）**：占用时报"database is locked"+ 持有者信息 | 服务进程单写者天然解决 |
| 多用户 | 无 | **OS 文件权限即用户隔离**（chmod/属主），CLI 不建用户体系 | 届时在 serve 模式加认证（PG 协议已有 startup 挂点） |
| 事务隔离 | 隐式单语句 + 显式 API（SI） | SQL 级 `BEGIN/COMMIT/ROLLBACK` 接线已有 API | Read Committed（MS09-T01） |

结论：CLI 形态下"多用户隔离"的正确答案是**不做数据库内用户体系**——与 sqlite 一致，隔离靠文件系统；多用户需求自然映射到"多文件 + OS 权限"。数据库内权限体系只有服务形态才需要，且 PG 协议 startup 已是现成挂点。

## 主题 7：非交互 CLI 与密钥/分析能力设计空间（用户决策 2026-09-05 第三轮）

### 用户决策记录

- **非交互优先**：整个数据库通过 CLI 操作且均为非交互式（方便 agent 与脚本）；REPL 不是核心（可作后续可选）。
- **存储位置**：指定路径则生成在该处；不指定则集中固定位置保存。
- **密钥机制**：不做多用户；单密钥，sudo 式——一个终端首次访问某 db 需以命令行方式（非交互）提供密码，之后该终端不再询问。
- **分析能力**：希望补上数据分析能力，同样以 CLI 命令行形态。

### 已确认事实（支撑设计）

- **JSON 渲染已存在**：`pipeline.rs:684 value_to_json` + `Response::QueryResult { rows: Vec<Vec<serde_json::Value>> }`（`network/protocol.rs:35`）——CLI `--format json` 可直接复用，近乎免费；csv/table 为新增渲染层。
- **无 `Database::query` 独立 API**：库面就是 `Database::open/create_table/execute_sql/begin/commit/rollback/execute_in_tx/checkpoint/close`（`database.rs`）——CLI 是这些 API 的薄封装。
- **无加密依赖**：Cargo.toml/lock 无 aes/argon2/ring 等（rand_chacha 仅来自 rand）。加密 = 新依赖 + 密码学安全关键路径。
- **GROUP BY 仅列名**（`planner/query.rs:426` `Vec<String>`），HAVING/ORDER/LIMIT/5 聚合已有——分析能力的 SQL 底子存在，缺表达式与函数层。
- **多语句截断**（主题 2）：one-shot 单语句调用天然绕开；CLI 若支持 `;` 分片需自行循环并逐条报错。

### 命令面设计空间（建议，供规划）

- **主命令**：`rtsql <db> <sql>`——one-shot 执行，输出到 stdout，退出码分类（0 成功 / 2 用法错 / 3 SQL 错 / 4 连接锁冲突 / 5 密钥错误）。
- **生命周期子命令**：`rtsql new <name|path>`、`rtsql list`（集中区枚举 *.db）、`rtsql schema <db>`（agent 写 SQL 前的发现步骤）、`rtsql dump/restore`（逻辑导出，规避 WAL 配对问题）、`rtsql import --csv`、`rtsql key ...`。
- **名称解析**：含 `/` 的参数 = 路径原样；裸名 = `$RTSQL_HOME/db/<name>.db`（默认 `~/.rtsql/`）——即用户要的"指定地方 / 不指定集中固定位置"。
- **输出**：TTY 默认表格；非 TTY 默认 json（agent/管道友好）；`--format table|json|csv|tsv` 显式覆盖。json 复用 `value_to_json`。

### 密钥机制设计空间

**关键判断：密码必须落在加密上才有意义。** 文件是明文时，任何"打开前验证密码"都只是礼貌性门锁——拿到文件的人绕过 CLI 直接读。所以推荐 **SQLCipher 模型的整库加密**：

- KDF：Argon2id（密码 + 文件头随机盐）→ 派生密钥；页加密 AES-256-GCM（或 file-level stream cipher）；文件头存盐 + KDF 参数 + 校验器。
- 实现面：新依赖（argon2 + aes-gcm 或 ring）+ `FileStorage` 读写路径包一层加解密 transform + 密码学面 BDD（错误密码/损坏密文/格式迁移）。**触及安全关键路径与磁盘格式，应独立 change，不与 CLI 壳混做。**
- **sudo 式"终端记住"的实现选项**：
  1. **环境变量 `RTSQL_KEY`（推荐主路径）**：shell 会话 `export` 一次，该终端所有后续命令继承——与"那个终端不再询问"语义精确吻合，且零跨进程协调、agent 友好（`env RTSQL_KEY=... rtsql ...`）。
  2. **TTL 密钥缓存**（推荐辅路径）：首次 `--key` 后将派生密钥写入 `~/.rtsql/keys/<db-id>`（0600），带过期时间（sudo timestamp 模式）；过期后重新要 key。缓存派生密钥而非明文密码。
  3. key agent 常驻进程（ssh-agent 模式）：跨终端共享，但引入进程管理复杂度——非必要不做。
- 非交互输入面：`--key <arg>` / `--password-file <path>` / `RTSQL_KEY`——**绝不交互提示**（agent 场景挂起 = 事故）。
- 密钥与锁的关系：锁是并发正确性（前置），密钥是机密性（独立 change）；`database is locked` 与 `invalid key` 是不同错误类、不同退出码。

### 数据分析能力评估（"你觉得呢"的回答）

**值得做，且 CLI 形态正好是 agent 分析的正确接口；但分析能力的主体是 SQL 函数/表达式层，CLI 只加薄命令。**

- 已有底子：5 聚合 + GROUP BY（列名）+ HAVING + ORDER + LIMIT + 谓词下推。
- agent 分析的真实需求排序（推断，基于"agent 写 SQL"的工作方式）：
  1. **schema 发现**（`rtsql schema`）——没有它 agent 无法写第一条 SQL；catalog 数据已就绪，纯薄层。
  2. **表达式四件套 + CASE/COALESCE**（主题 2 缺口）——过滤与派生列的日常件。
  3. **标量函数库**（string: upper/length/substr/replace；math: round/abs；类型转换 CAST）——按需增量，每个可独立验收。
  4. **日期/时间类型与函数**（date_trunc、interval）——分析最高频维度，但依赖类型系统扩展（格式变更），单列深水区。
  5. **CLI 薄命令**：`rtsql stats <db> <table>`（行数/空值率/distinct/min/max/分位数——底层即聚合 SQL）、`rtsql sample`、`rtsql profile`。全部可用 SQL 实现，不动引擎。
  6. 窗口函数（OVER/PARTITION BY）：分析力大跃迁但执行器面改动大——远期，不进第一轮。
- 结论：分析能力 = "函数/表达式补齐（引擎层，渐进）+ stats 类薄命令（CLI 层，便宜）"，与主题 2 缺口清单同源，不产生独立大 MS；日期类型是其唯一需要专门规划的深水区。



## 主题 8：安装与分发（2026-09-05 第四轮补充）

### 已确认事实（实测）

- **单二进制、极轻**（本机实测，rev `709c85d`）：`cargo build --release` 32 秒；二进制 4.2MB，strip 后 **3.7MB**（`[profile.release] lto=true` 已配置生效）。
- **运行时零第三方依赖**：`ldd` 仅 libc/libm/libgcc_s（glibc 动态链接）。musl 全静态可行（本机未装 musl target，未实测）。
- **平台边界**：`std::os::unix::fs::FileExt`（MS08-T01）限 Unix——Linux x86_64 现状；macOS 理论可编译（同为 Unix）但未验证；**Windows 需重写页 I/O 层**（无 FileExt），当前不可行。
- **仓库无分发设施**：无 CI workflow、无打包配置、无 completions/man。`version 0.1.0`，双许可 MIT OR Apache-2.0（利于打包渠道）。
- **无文件格式标识**：`FileStorage::open` 仅校验 4KB 对齐（`file_storage.rs:32-34`），**无 magic/格式版本头**——跨版本打开不兼容旧文件时无法给出可操作错误（SQLite 有 16 字节 magic 头）。

### 分发渠道建议（按优先级）

1. **GitHub Releases 预编译产物**（主渠道）：tar.gz 内含二进制 + shell completions + man page；CI 矩阵 x86_64-linux-gnu / x86_64-linux-musl（全静态）/ aarch64-linux / macOS aarch64+x86_64（`cargo-zigbuild` 或 GitHub runner 原生）。需要先建 CI——目前不存在。
2. **`cargo install rtsql`**（crates.io）：Rust 用户零成本；需补全 `[package]` 元数据（description 已有）。
3. **Homebrew tap**：Releases 稳定后再做（macOS/Linux）。
4. deb/AUR/install 脚本：按需求再议，不预建。

### 安装形态要点（CLI 数据库的特殊优势）

- **无服务/守护进程注册**——安装 = 放一个二进制，卸载 = 删它；数据目录 `~/.rtsql/` 与程序生命周期完全解耦（升级/卸载不动数据）。这是相对 C/S 数据库的安装故事核心卖点，应在 README 显式表达。
- **升级 = 换二进制**；因此**文件格式版本策略**成为分发期需求：建议在文件头加 magic + format_version（一次性格式变更，趁用户量小的现在做），open 时不兼容即报"文件由新版创建"——否则未来升级会静默踩旧文件。
- 包内 completions/man 由 clap 构建期生成（依赖 CLI 壳 MS 先落地）。



## 关键文件索引

| 文件 | 事实 |
|---|---|
| `src/main.rs` | 硬编码 demo 入口（rtsql.db / :9876 / 建 test 表） |
| `src/network/server.rs` | TcpListener + Semaphore + CancellationToken（main 未接信号） |
| `src/network/pg_protocol.rs` | Simple Protocol 3.0；无条件 AuthenticationOk；错误统一 58000 |
| `src/network/pg_messages.rs` | DataRow 文本渲染（含 Null/-1、Bool t/f） |
| `src/network/protocol.rs:34-39` | `Response` 枚举（QueryResult JSON rows / AffectedRows / Error / Pong） |
| `src/pipeline.rs:42-48/233-241/329-343` | parse_stage 多语句 → first() 截断 |
| `src/parser/planner/mod.rs:90-93` | 6 种语句臂 + UnsupportedStatement |
| `src/parser/planner/expression.rs:75` | 表达式 fallback 报错（无 IN/LIKE/BETWEEN/CASE/IS NULL/CAST/函数） |
| `src/parser/planner/aggregate.rs:54-84` | 5 聚合函数 |
| `src/parser/ast.rs` | GenericDialect；SELECT body 仅 Select |
| `src/storage/page_format/tuple.rs:24-33` | 4 列类型 |
| `src/database.rs:28-53/169-183` | open 文件对 + 恢复；close()=checkpoint；重开空 schema 警告 |
| `src/storage/file_storage.rs:20-33/93/110` | 主文件打开/页对齐校验；无跨进程锁（仅进程内 Mutex） |
| `src/wal/writer.rs:27` | WAL 路径 = `with_extension("wal")` |
| `src/storage/data/table_manager.rs:115-126` | bootstrap/open 判定（page_count==0） |
| `src/storage/catalog.rs:92-124` | 页 0/1 保留页分配与 flush |
| `src/transaction/snapshot.rs:11-53` | 快照隔离规则（经典 SI） |
| `src/pipeline.rs:684` | `value_to_json`——CLI json 输出可复用 |
| `src/parser/planner/query.rs:426` | GROUP BY 仅列名（`Vec<String>`） |
| `Cargo.toml` | release LTO 已配置；无 CI/打包设施；双许可；version 0.1.0 |
| `target/release/rtsql`（实测） | 4.2MB（strip 后 3.7MB）；ldd 仅 libc/libm/libgcc_s |

## 未确认项

1. JOIN 的 FROM 范围（ON 形式、多表链、OUTER）——`join_test.rs` 存在但未逐行读 planner/query.rs。
2. `ColumnConstraint` 具体（NOT NULL/DEFAULT/UNIQUE）支持度。
3. ORDER BY/HAVING/GROUP BY 的表达式支持范围（是否同 expression.rs 一致受限）。
4. Ctrl-C 下实际恢复代价（推断 WAL redo 兜底可靠，未实测 kill 场景）。
5. `.wal` 与主文件的移动/重命名配对行为（`with_extension` 替换语义下，拷贝主文件不带 WAL 的恢复语义）未实测——建议 CLI 备份方案以逻辑导出规避。
6. 同进程两个 `Database` 实例打开同一文件的实际损坏形态为结构推断（两个独立 BufferPool + WAL writer），未复现实验——锁前置项的必要性不依赖该实验。
7. 页级加密的性能开销（每页 AES-GCM 加解密 + Argon2id 打开时 KDF）未测量——加密 change 需按 MS08 纪律先建基线；WSL2 下每页加解密约在 µs 量级（推断），但 Argon2id 参数选择是打开延迟 ↔ 暴力破解抵抗的权衡，需专门决策。
8. musl 全静态构建与 macOS 构建未实测（本机无对应 target/平台）；macOS 下 `FileExt` 可用但整个测试套件未在该平台跑过。
9. 文件 magic/格式版本头的引入时机（建议趁用户量小尽早）与旧文件迁移策略未决策。

## See also

- [m19-datascan-path.md](m19-datascan-path.md)（DataScan 链遍历与性能）
- [m21-page-visibility-incomplete.md](m21-page-visibility-incomplete.md)（页面级 MVCC）
