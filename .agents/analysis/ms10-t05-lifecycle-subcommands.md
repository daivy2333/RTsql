# MS10-T05 生命周期子命令实现上下文

> Snapshot: [SNAPSHOT](../docs/SNAPSHOT.md)
> Captured revision: `a5b0a5f`（master，2026-09-09，docs sync 提交；实施代码基线 = `8827700`）
> Observed branch: master；环境: Linux x86_64 WSL2
> Captured at: 2026-09-09
> See also: [usability-gap-cli-form.md](usability-gap-cli-form.md)（R18，MS10 规划依据）、[ms10-t03-file-format-header.md](ms10-t03-file-format-header.md)（R19，格式头）

## 目标与范围

为 MS10-T05（生命周期子命令 `new/list/schema/dump/restore/import --csv`）的 change 规划建立实现调查基线。回答：

1. 现有 clap 入口结构如何，加子命令的改动面与约束？
2. `Database` 公开 API 对 6 个子命令的支撑度与缺口？
3. schema 发现的数据源与访问路径？
4. dump/restore/import --csv 的数据面（SQL 能力、CSV 能力、类型系统）？
5. `resolve_db_path` 如何支撑 new/list？
6. 测试与退出码矩阵如何扩展？
7. 既有改进/契约对 T05 的约束？
8. 规划依据中已固定的设计空间是什么？

范围：`src/cli/`、`src/main.rs`、`src/database.rs`、`src/storage/catalog.rs`、`src/storage/data/table_manager.rs`、`src/storage/file_storage.rs`、`src/parser/planner/ddl_dml.rs`、`tests/cli_test.rs`、Cargo.toml。未运行测试；文中"基线"指 SNAPSHOT 记录的 2026-09-09 结果（671 pass）。

## 已确认事实、推断与未确认项

### 入口结构（问题 1）

- **F1 现行 `CliArgs` 是扁平位置参数结构**（`src/cli/mod.rs:63-77`）：`db: String` + `sql: String` + `--format`，无 subcommand 枚举。`run()`（mod.rs:88-95）→ `execute_command` → `execute_command_inner`。加子命令必须重构此结构。clap v4 允许 `#[command(subcommand)] Option<Cmd>` 与位置参数共存，首参匹配子命令名时按子命令解析、否则填充位置参数——此行为为**推断**（clap 通用模式），实现时以编译+测试验证。
- **F2 裸名库与子命令名冲突是真实边界**：若 F1 成立，裸名为 `list/new/schema/dump/restore/import` 的数据库经 one-shot 主命令打开时会被 clap 捕获为子命令 → 用法错误。规避路径（含 `/` 路径形式）与是否在错误文案中提示，属 Plan 决策。
- **F3 两阶段信号编排可直接复用**：`execute_command_inner(db_path, work_factory, signal_int, signal_term)`（mod.rs:162-196）对工作 future 工厂泛型化（HRTB `for<'a>`），open 与 work 各与信号竞争，所有路径收口 `close()`。schema/dump/restore/import 等开库子命令传入各自 work 闭包即可；`new` 同理。`list` 不开库，无信号需求。`open_error_status`（mod.rs:143-154）已集中处理锁冲突 exit 4 与一般 open 错误 exit 1。

### Database API 支撑度（问题 2）

- **F4 公开面**（`src/database.rs`）：`open(path)`、`create_table(name, columns, pk)`、`get_table(name) -> Arc<TableMeta>`、`execute_sql(sql) -> Response`、`begin/commit/rollback/execute_in_tx`（MS07-T04）、`checkpoint()`、`close()`（= checkpoint，database.rs:182-194）。字段 `table_manager: Arc<TableManager>` 公开（database.rs:17-25）。
- **F5 `new` 的语义基础已存在**：`Database::open` 对不存在路径静默建库（`file_storage.rs:41-47` `OpenOptions::create(true).truncate(false)`；契约测试 `tests/cli_test.rs:506` `test_new_database_created_silently`）。建库 = open 即完成（`TableManager::new` 按 `page_count()==0` 走 `Catalog::bootstrap`，bootstrap 内 `flush_all` 落盘 catalog 页，catalog.rs:119）。**推断**：`new` ≈ open + close()（close 补 checkpoint 并截断 WAL，最稳）；空库 open 后立即 drop 的落盘完整性未单独验证。
- **F6 `FileStorage::open` 不创建父目录**：std::fs OpenOptions 语义，父目录缺失 → Io 错误 → `open_error_status` → exit 1。既有契约由 `tests/cli_test.rs:1-7` 头注固化（"CLI 不建目录，父目录缺失按契约报错退出 1"，fixture 预建 `db/`）。`new` 是否为集中区建目录（`mkdir -p $RTSQL_HOME/db`）属 Plan 决策，与该契约冲突需显式裁决。
- **F7 开库即受既有守卫保护**：open 顺序 = 文件打开 → advisory 独占锁（`try_lock`，冲突 → `StorageError::DatabaseLocked`）→ 0 字节写 64B 头（无 fsync）/ 非 0 字节分类校验头（`NotADatabase`/`NewerFileVersion`/`IncompatibleHeader`）→ WAL → 恢复（file_storage.rs:41-90，MS10-T02/T03）。所有开库子命令自动继承锁 exit 4 与格式拒绝 exit 1 语义，无需新代码。

### schema 数据源（问题 3）

- **F8 系统表不可经 SQL 查询**：`__tables`/`__columns` 不在 `TableManager.tables` map（`open_or_init` 只重建用户表，table_manager.rs:159-208），create/drop 有保留名守卫（`ReservedTableName`）。`SELECT * FROM __tables` → `TableNotFound`。**schema 子命令必须走内部 API**：`db.table_manager.catalog()`（table_manager.rs:136）→ `Catalog::scan_tables() -> Vec<CatalogRow>`（catalog.rs:205-211）与 `scan_columns(table_name) -> Vec<CatalogColumnRow>`（catalog.rs:214-220）。两者 pub，CLI 同 crate 可达。
- **F9 catalog 行结构**（catalog.rs:49-69）：`CatalogRow { table_name, data_page_head, index_root_page_id, pk_index, pk_column, column_count, data_page_tail }`；`CatalogColumnRow { table_name, column_index, column_name, column_type(存储面), not_null, unique }`。schema/dump 所需的表名/列序/类型/PK/约束均在内。

### dump/restore/import 数据面（问题 4）

- **F10 双 `ColumnType` 体系与 255 固定转换**：SQL 面 `executor::value::ColumnType`（`Int/String/Float/Bool`，String 无长度，value.rs:10-19）与存储面 `storage::page_format::ColumnType`（`String(u16)`，tuple.rs:24-33）。转换点 `executor/plan.rs:195-201`：SQL `String` → 存储 `String(255)` 固定默认。DDL 词法面 `convert_data_type`（ddl_dml.rs:133-169）把 INT 族/STRING 族（Varchar/Char/Text/Clob 等）/FLOAT 族/BOOL 族各归一类，未知类型 fallback 到 String。**含义**：CLI 侧建表/restore 产生的 String 列恒为 255；dump DDL 用 `STRING` 关键字可保真往返。仅 lib 直调 `Database::create_table`（接收存储面 ColumnType）可造出非 255 长度列，其长度信息 dump DDL **无法表达**（restore 后变 255）——dump 保真边界。
- **F11 两枚举均无 `Display`/SQL 名映射**：schema 列类型渲染、dump DDL 类型渲染需新写小映射函数（存储面 → SQL 关键字），并处理 `String(u16)` 长度信息的展示口径（Plan 决策：显示 255 原值还是忽略）。
- **F12 约束持久化面**：catalog 每列只持久化 `not_null`/`unique` 布尔（CatalogColumnRow），**无 default 字段**——`extract_column_constraints`（ddl_dml.rs:170-198）虽收集 `DefaultValue`，推断 DEFAULT 不入 catalog、重启丢失；dump 无法还原 DEFAULT。此为**推断**（未追 catalog 写入全链），Plan 应确认后写入保真声明。
- **F13 dump 的行导出通道现成**：`SELECT * FROM <t>` 经 pipeline → `Response::QueryResult { rows: Vec<Vec<serde_json::Value>> }`（network/protocol.rs，R18 已核）。NULL → `Value::Null`。INSERT 文本生成需单引号加倍转义（无现成 SQL literal 转义函数，需新写）。SQL 语句面共 6 种（CREATE TABLE/INSERT/UPDATE/DELETE/SELECT/DROP，planner/mod.rs:90-93），dump 产物（DDL+INSERT）不超此面。
- **F14 restore 的执行通道现成**：dump SQL 文本走 `parse_stage` → 逐条 `plan_stage`/`execute_stage` 循环（`run_sql`，mod.rs:198-239；MS10-T04 分片逐条 auto-commit + fail-fast）。两点 Plan 决策：restore 应静默或摘要（`run_sql` 现对每条渲染 Affected 输出，不合适直接复用）；restore 目标非空库时 `create_table` 报 `DuplicateTable` fail-fast 中止（是否要求空库/是否自动 drop 需裁决）。DDL 无 WAL 记录（mod.rs:308 注释，design D7），restore 完成必须 `close()`——`execute_command_inner` 已保证。
- **F15 import --csv 无 CSV 解析依赖**：Cargo.toml:8-22 无 csv crate；R18 主题 7 仅定位命令名。三个 Plan 决策：(a) 引入 csv crate（RFC4180 引号/转义边界多，不建议手写）——新增依赖需按 Scope Control 论证；(b) 目标表约定：预存在表（R18 的 agent 工作流是 schema 先行 + 写 SQL）还是自动建表；(c) CSV 文本 → 类型化 Value 转换规则（按目标表 schema 驱动转换 / NULL 表示约定）与批量事务性（逐条 auto-commit vs `execute_in_tx` 单事务，MS07-T04 API 已有）。
- **F16 `list` 是纯文件系统操作**：枚举 `$RTSQL_HOME/db/*.db`。`resolve_db_path`（resolve.rs:12-26）未暴露基目录推导（base 逻辑内联），需提取 `db_dir()` 类 helper（refactor 面）。是否开库校验（触碰锁/格式头）vs 仅 stat 枚举属 Plan 决策——仅枚举快且无副作用，但不反映文件有效性。渲染可复用 `render()`（`render.rs:24-37`，`QueryPayload::Rows`），json 输出 agent 友好。

### 测试与退出码（问题 6）

- **F17 CLI 测试模式**（`tests/cli_test.rs`，914 行 25 测试）：真二进制 `env!("CARGO_BIN_EXE_rtsql")` + `run_cli/spawn_cli/wait_cli` helper（cli_test.rs:26-79，60s 超时 kill）；TempDir fixture 预建 `db/` 并把 `RTSQL_HOME` 指向 TempDir（fixture()，cli_test.rs:81-86）；管道非 TTY → 默认 JSON。信号用例 `#[tokio::test]` 直接 spawn 发信号。lib 单测模式：`mod.rs` tests（信号结构测试，mod.rs:287-371）、resolve.rs tests（EnvGuard 顺序执行防 env 并行污染，resolve.rs:28-59）。子命令测试沿用同文件同模式即可。
- **F18 退出码矩阵无需扩展**：`ExitStatus` 6 类（mod.rs:24-60）已覆盖子命令可见错误类——Success 0 / General 1 / Usage 2（clap 用法错自行 exit 2）/ Sql 3 / Locked 4 / InvalidKey 5（留位）/ Signaled 128+n。子命令错误归入既有类（General/Usage 为主），密钥类属 MS12。

### 既有约束与规划依据（问题 7、8）

- **F19 I034（planned）与 T05 的交叠**：裸 DataScan 子集投影经 `get_plan_output_columns` 返回全 schema 表头（improvements spec.md:266-273）。T05 中任何"SELECT 子集列再渲染"路径（如 schema/list 若走 SQL 查询渲染）会踩同一缺口；走内部 Catalog API（F8）则不受影响。I034 修复独立于 T05。
- **F20 I035（no-FROM SELECT 缺失）不影响 T05**：dump/restore/import 不需要常量查询。
- **F21 R18 已固定的设计空间**（usability-gap-cli-form.md:155-160，主题 7 用户决策）：`rtsql new <name|path>`、`list`（集中区枚举 *.db）、`schema <db>`（定位为"agent 写 SQL 前的发现步骤"）、`dump/restore`（**逻辑导出，规避 .db/.wal 文件对配对问题**——只拷 .db 在 checkpoint 后等价、否则丢尾事务，R18 主题 5 推论）、`import --csv`、`key ...`（MS12）。dump/restore 采纳逻辑导出是既定方向，产物形态（SQL 文本 vs CSV 集）未定。
- **F22 归档 ms10-t01 change 无子命令结构预留**：其 design.md 仅含投影设计（D 系列指向扫描执行器），子命令结构留给 T05 自行设计。

## 调用链或数据流

主命令现行链（子命令各臂在此基础上分叉）：

```
run() [mod.rs:88]
  → CliArgs::parse (clap)                      // F1 重构点
  → execute_command → resolve_db_path [resolve.rs:12]   // F16 helper 提取点
  → execute_command_inner [mod.rs:162]         // F3 复用点
      select { Database::open, sig_int, sig_term }      // F7 守卫链
      select { work(&db),  sig_int, sig_term }          // ← 子命令差异全部在此臂
      db.close()                                        // F14 restore/new 依赖
```

各子命令数据流（Plan 输入）：

- `new`：resolve → open（建库）→ close。
- `list`：resolve base dir → read_dir(`*.db`) → render()。不开库。
- `schema`：open → `table_manager.catalog().scan_tables()/scan_columns()` [catalog.rs:205/214] → 类型名映射（F11）→ render() → close。
- `dump`：open → scan_tables → 逐表 `SELECT * FROM t`（run_sql 管线或直接三 stage）→ DDL+INSERT 文本生成（转义 F13）→ close。
- `restore`：open → dump 产物经 parse/plan/execute 循环（F14）→ close。
- `import --csv`：open → 读 CSV（F15 依赖决策）→ 类型转换 → INSERT（事务性决策）→ close。

## 边界与失败路径

- **锁冲突**：所有开库子命令经 F7 守卫链 → `DatabaseLocked` → exit 4（`open_error_status` 复用）。
- **格式头拒绝**：旧损坏/异版文件 → exit 1（T03 拒绝矩阵自动生效于 schema/dump/restore/import）。
- **父目录缺失**：现行契约 exit 1（F6）；`new` 是否放宽为建目录待 Plan 裁决。
- **裸名=子命令名冲突**：F2，需 Plan 定文案与文档口径。
- **restore 非空库 / import 目标表不存在**：fail-fast 语义与前置条件检查待 Plan 定（F14/F15）。
- **dump 保真边界**：String 非 255 长度不可表达（F10）；DEFAULT 约束不在 catalog 持久化面（F12 推断，待确认）；转义面 = 单引号加倍（F13）。
- **长任务信号**：dump/restore/import 大数据量下应复用 F3 两阶段编排（信号 → close → 130/143），避免新增裸跑路径。

## 测试、验证入口与影响面

- 既有基线：`cargo test --all` 671 pass / 2 ignored（SNAPSHOT，2026-09-09）；`tests/cli_test.rs` 25 测试；`clippy -D warnings` / `fmt` / `openspec validate` 全 0/PASS。
- 本任务影响面：`src/cli/mod.rs`（入口重构 + 子命令臂）、`src/cli/resolve.rs`（目录 helper）、`src/main.rs`（6 行转发，无需动）、Cargo.toml（若引入 csv）。**纯 CLI 层 + 渲染层改动；database.rs/pipeline.rs/存储层预期零修改**（F3/F4/F7/F8 全部复用现成 API）。
- 新测试入口：cli_test.rs 追加子命令集成测试（参数/退出码/渲染/冲突/dump-restore 往返/CSV 边界）；resolve.rs 单测扩展目录 helper；lib 单测覆盖类型名映射与 SQL literal 转义纯函数。
- 验证状态：本文档调查期间**未运行任何测试**；F17 基线引自 SNAPSHOT 记录。

## 关键文件

| 文件 | 事实锚点 |
|---|---|
| `src/cli/mod.rs:63-77` | `CliArgs` 扁平结构（F1 重构点） |
| `src/cli/mod.rs:162-196` | 两阶段信号编排（F3 复用点） |
| `src/cli/mod.rs:198-254` | `run_sql` 逐条循环 + fail-fast 模板（F14 restore 通道） |
| `src/cli/mod.rs:24-60` | `ExitStatus` 退出码矩阵（F18） |
| `src/cli/resolve.rs:12-26` | 名称解析，base 内联不建目录（F16） |
| `src/cli/render.rs:24-37` | `render()` 四格式纯函数（F16 复用） |
| `src/database.rs:17-25/28-93/182-194` | 公开字段/API、open 建库链、close=checkpoint（F4/F5） |
| `src/storage/file_storage.rs:41-90` | create 语义、锁→头校验顺序（F6/F7） |
| `src/storage/catalog.rs:49-69/205-220` | CatalogRow/CatalogColumnRow + scan API（F8/F9） |
| `src/storage/data/table_manager.rs:136/159-208` | `catalog()` 访问器、系统表不在用户 map（F8） |
| `src/executor/plan.rs:195-201` | SQL→存储类型转换，String→String(255)（F10） |
| `src/parser/planner/ddl_dml.rs:133-198` | 类型归一与约束提取（F10/F12） |
| `Cargo.toml:8-22` | 无 csv 依赖（F15） |
| `tests/cli_test.rs:26-86` | 真二进制测试模式（F17） |
