# Iteration 000 / Cycle 000: 兼容样本与刻画断言——旧库与旧 dump 的支持范围被固定样本锁定

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T1
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `database-file-format-header` 域既有语义（文件头布局、格式错误分类、打开顺序守卫、既有语义零回归）；`cli-noninteractive-shell` 域既有语义（dump 渲染规则、restore 逐条 auto-commit 与 fail-fast 合同、退出码 1/3/4 语义）；`schema-persistence` 域既有语义（`__tables`/`__columns` 系统表与目录行布局）；`free-page-lifecycle` 域既有语义（打开时按持久目录重建空闲页集合、失败安全降级）；`drop-persistence-boundary` / `drop-table-physical-free` 域既有语义（MS33/MS35 的释放持久化判定与崩溃语义）。
- Excluded scope: 支持范围的规格文本与 README 表述（Iteration 001）；收尾全量回归（Iteration 001）；任何产品代码改动；旧库格式迁移工具；`restore` 的原子化/全量预检语义变更；`dump` 对非整数 `UNIQUE` 列的渲染规则调整；CI / 预编译发布 / crates.io 发布（I051–I053）；键编码格式变更（I024）；伴生文件加密（I060）；ISS22–ISS25 的修复；规格合并与全局状态同步（`openspec-docs-maintainer`）。

**Objective**

本 Cycle 完成后，仓库内存在一组**固定的旧格式样本**（旧代际库文件、旧代际库 + 未决写入日志、旧代际 dump 文本）与一组走真实入口的刻画断言，使下列事实成为可复跑的基线：

1. 旧代际库文件能**打开、查询、写入**，写入后既有已提交数据不丢失，完整性扫描零 error 级发现；
2. 旧代际库文件在当前打开路径（恢复 → 打开时安全重建）上**重建不放弃**，活表完整；
3. 旧代际库带未决写入日志时**恢复出已提交行**；
4. 旧代际 dump **能恢复到当前库**且行数据等价；
5. 三类范围外输入给出**明确、定位到语句的拒绝**（点名原因 + 第 k 条/共 n 条 + 失败退出码）。

可观察结果：`cargo test --test legacy_compat_test -- --test-threads=1` 全绿，且该命令在后续任何格式变更后仍能判定「旧数据是否仍被支持」。

**Investigation Facts**

- Starting point:
  - 仓库 `HEAD` = `0a08b14`（ahead of origin 9）。**工作区存在既有未提交改动**（MS34 + issue-fix change + MS35 的产品代码、测试与文档改动：`src/` 下 20 个文件、`tests/` 下 4 个测试文件、`.agents/` 与 `openspec/specs/` 文档改动）：**不得回滚、不得清理、不得提交**。本 change 在此现场之上增量开发。
  - 当前实现基线（MS35 已 accepted，其 change 尚未归档）：打开路径在恢复之后调用 `TableManager::rebuild_free_page_set`（`src/database.rs:436`）按持久目录重建可分配页集合；目录行的可选尾段（唯一根段 / 二级索引段 / 表类型字节 / 列默认值段）自 MS21、MS23、MS24 起逐步追加，读取路径按「尾段缺失即取默认值」兼容。
  - 变更前验证（本次 Plan 会话实际执行，2026-10-24）：用**真实旧构建**产出的 G1 代际样本在当前构建上跑通「打开 → 查询 → 写入 → 重开 → 完整性扫描」，以及「旧 dump 恢复」「三类范围外拒绝」。逐条证据见下方 Current-State Evidence。
  - 已知限制：`FORMAT_VERSION` 恒为 1（`src/storage/file_header.rs:22`），产品**不存在更旧的格式版本**；旧库的代际差异只在目录行布局（`design.md` D1）。

- Current-State Evidence（Plan 已核对、与实施直接相关的事实）:

  - **文件头版本契约**：`src/storage/file_header.rs:22` `pub const FORMAT_VERSION: u32 = 1;`；`:113` `decode` 校验头；`:123` `if version > FORMAT_VERSION` → `NewerFileVersion`；magic 不符或头不完整 → `NotADatabase`。`git log -S "FORMAT_VERSION: u32"` 在整个历史中只有引入点 `2eda010`（MS10-T03）与其后未改 ⇒ **格式版本从未变更，不存在更旧的格式版本**。
  - **目录行代际的分界提交**（`git log -S` 定位，均为真实历史提交）：
    - `4307a0e`（MS07，2026-08-26）：目录行初始布局 = `u16 名字长度 | 名字 | u32 数据页头 | u32 索引根 | u32 主键索引 | u16 主键列长度 | 主键列 | u32 列数 | u32 数据页尾指针`（= G1）。
    - `6e8dcf5`（MS23+MS24，2026-09-26）：追加**唯一根段**（`u32 计数 | 计数 × u32 根`）与**列默认值段**（`u8 有默认值 | 标签 | 载荷`）⇒ G2。
    - `2f83dd9`（MS21，2026-09-30）：追加**二级索引段**与**表类型字节** ⇒ G3（当前）。
    - G1 区间 = `4307a0e` … `d8165fc`（2026-09-24，MS23 落库前的最后一个提交）。
  - **兼容读分支（当前实现的事实）**：`src/storage/catalog.rs:1163` 注释与实现——唯一根段可选，缺失即为空；`:1245`——二级索引段与表类型字节可选，缺失即空 / 表类型取 `Table`；`:1465`——列默认值段可选，缺失即无默认值。当前行序列化在 `src/storage/catalog.rs:1047`（`serialize_catalog_row`）与 `:1298`（`serialize_catalog_column_row`）；**旧行是当前行的字节前缀**（新段一律追加在尾部）。
  - **既有单测（同一事实的单元级覆盖）**：`src/storage/catalog.rs:2162` `legacy_layout_row_deserializes_with_empty_unique_roots`、`:2205` `legacy_row_without_kind_section_reads_as_table`、`:2504` `no_default_row_output_is_byte_identical_to_legacy`、`:2526` `legacy_column_row_without_default_reads_none`。
  - **目录行写回路径（写入即升级的机制）**：`src/storage/catalog.rs:450` `update_table_tail` 走「反序列化 → 修改 → 重新序列化」整行重写；`:867` `update_field_in_chain` 在新行长度变化时走「追加新副本 + 删除旧副本」。⇒ 旧行被写回时**自动成为当前布局**（G3），不需要专门实现。
  - **打开路径（MS35 重建钩子）**：`src/database.rs` 的 `open_internal` 顺序为 storage → buffer pool → `TableManager::open_or_init` → views → WAL → 恢复 → `attach_index_catalog_contexts` → plan cache → checkpoint manager；`:436` 调用 `table_manager.rebuild_free_page_set(path)`（实现见 `src/storage/data/table_manager.rs:1610`，活页枚举 `:1626`，索引树遍历 `:1769`，索引根为 0 时返回空集）。重建输入是**持久目录**，而旧目录行经兼容读分支能解析出同样的字段 ⇒ 重建前提与目录行代际正交。
  - **恢复路径**：`src/database.rs:393-428` 的恢复流程 + `src/storage/data/table_manager.rs` 的目录重放；旧代际库的伴生写入日志格式与当前一致（头部 21 字节 + 记录），当前构建可直接消费。
  - **dump / restore 实现与拒绝面**：`src/cli/lifecycle.rs:986` `create_table_sql`（DDL 渲染：**仅当主键列声明类型为 INTEGER 时输出 `PRIMARY KEY`**；`UNIQUE` 无条件渲染）；`src/cli/lifecycle.rs:574` 起记录 restore 的既有合同（逐条 auto-commit、失败即停、第 k 条/共 n 条、退出码 3）；`src/parser/planner/ddl_dml.rs:1257` 计划期拒绝 `NON-INTEGER PRIMARY KEY`，`:1298` 与 `:1323` 计划期拒绝 `UNIQUE (INT columns only)`；非空约束违例在执行期报 `NOT NULL constraint violation: column '<名>'`。
  - **可观察面（测试入口）**：`rtsql::storage::{AsyncStorage, FileStorage, Page, PageId}` 为公开 API（`AsyncStorage::read_page` 可读原始页）；`rtsql::storage::page_format::{SlottedPage, SlottedPageRef, SlottedPageHeader}` 为公开 API（可解析目录页的槽与行字节）。既有 CLI 子命令：`new` / `list` / `schema` / `dump` / `restore` / `import` / `delete` / `checkpoint` / `check` / `stats` / `sample` / `profile`。
  - **既有测试与既有覆盖（不重复造）**：`tests/file_header_test.rs` 已覆盖 `NewerFileVersion`（`test_newer_version_rejected`）与无头文件拒绝（`test_headerless_legacy_file_rejected`）⇒ 本 change 不重复这两条，只在 RTM 中引用。

  - **本次会话实际运行所得的兼容事实（变更前证据，命令与结果如实记录）**：

    | 事实 | 实际观察 |
    |---|---|
    | 旧构建可得 | `git clone --no-hardlinks --shared <repo> /tmp/rtsql-legacy` → `git checkout d8165fc` → `cargo build --offline --bin rtsql` 成功（rustc 1.99.1），产物 `/tmp/rtsql-legacy/target/debug/rtsql` |
    | G1 样本字节确为旧布局 | 旧构建产出库文件的 `__tables` 页（文件偏移 64 起）行字节为 `05 00 "users" 02 00 00 00 03 00 00 00 00 00 00 00 02 00 "id" 03 00 00 00 02 00 00 00` 后即结束（无唯一根段 / 二级索引段 / 表类型字节）；`__columns` 行无默认值段 |
    | 旧库可打开可查询 | 当前构建对 G1 样本执行 `SELECT * FROM users` → `{"columns":["id","name","score"],"rows":[[1,"alice",9.5],[2,"bob",7.25],[3,"carol",8.0]]}`，退出码 0，stderr 无「重建放弃」消息 |
    | 旧库完整性扫描零 error | `rtsql check <旧库>` → `check ok: 4 pages scanned, no corruption found` |
    | 旧库可写入且读回 | 当前构建 `INSERT INTO users VALUES (4,'dave',6.0)` → `{"affected_rows":1}`；重开查询返回 4 行；`check` 仍为 `check ok` |
    | 写入使目录行升级 | 旧库导入 500 行后重读 `__tables` 页：新增一行为当前布局（尾段 `唯一根计数=0`、`二级索引计数=0`、`表类型字节=0x01`），旧行副本仅作死槽残留；数据行数 504 可查、`check ok: 21 pages scanned` |
    | 打开时重建在旧库上成立 | 旧库含一张已被删除的表（遗留空闲页）时打开成功、活表 2 行完整、`check ok: 6 pages scanned`；打开后写入成功且活表 3 行完整 |
    | 旧库带未决日志可恢复 | 旧构建导入中途被 `SIGKILL`（未 checkpoint）产出的库 + 旧日志，当前构建打开后恢复出 216 行，`check ok: 14 pages scanned` |
    | 旧 dump 可恢复 | 旧构建 `dump` 产出 `CREATE TABLE "users" ("id" INT PRIMARY KEY, "name" STRING NOT NULL, "score" FLOAT);` 等短类型名文本；当前构建 `restore` 退出码 0，读回 3 行与源库等价 |
    | 旧 dump 含日期/时间戳/布尔可恢复 | 旧 dump 含 `DATE '2026-01-02'`、`TIMESTAMP '2026-01-02 03:04:05'`、`TRUE` → restore 退出码 0，各列值与源库等价 |
    | 范围外拒绝显式（真实旧 dump） | 旧构建的 `UNIQUE`（非整数列）dump → `{"error":"statement 1 of 3 failed: Plan error: Unsupported constraint: UNIQUE (INT columns only); statement: CREATE TABLE \"legacy_uq\" ..."}`，退出码 3；旧构建的非整数 `PRIMARY KEY` dump → `Plan error: Unsupported constraint: NON-INTEGER PRIMARY KEY`，退出码 3；旧构建的 `NOT NULL` 含 NULL dump → `Execution error: NOT NULL constraint violation: column 's'; statement: INSERT INTO "nn" VALUES (1, NULL); previous statement(s) were committed`，退出码 3 |
    | 非整数主键的当前 dump 往返可达 | 当前构建对旧库（非整数主键表）`dump` 产出 `"id" TEXT`（不渲染 `PRIMARY KEY`）→ restore 退出码 0、行等价 |
    | 旧库加密形态可打开 | 旧构建 `--key pw123` 建的加密库（头 flags=0x1）→ 当前构建 `--key pw123` 打开并读回 1 行，退出码 0 |

- Code and Critical Path:

```text
兼容样本的生产（一次性，离线，不在工作区内）
  git clone → git checkout d8165fc → cargo build --offline --bin rtsql
    └─ 旧构建 rtsql new / dump  → G1 库文件 + 旧 dump 文本（固定字节，提交进仓库）

测试运行时（真实入口）
  固定样本 tests/fixtures/legacy/*.db|*.sql
    └─ 复制到临时目录
        ├─ Database::open（真实文件库）
        │    ├─ FileStorage::open → 文件头校验（decode：版本 > FORMAT_VERSION → NewerFileVersion；magic 不符 → NotADatabase）
        │    ├─ TableManager::open_or_init → 扫描持久目录（目录行兼容读分支）
        │    ├─ 恢复（消费旧写入日志）
        │    └─ TableManager::rebuild_free_page_set（按持久目录重建可分配页集合；索引根为 0 → 空集）
        ├─ 查询 / 写入（写入 → update_table_tail → update_field_in_chain → 目录行按当前布局重写）
        ├─ check_integrity（只读；error 级发现数）
        └─ dump / restore（文本 DDL 渲染 → 计划期约束校验 → 执行）
```

- 动态边与状态变化:
  - 旧库打开 ⇒ 目录行按兼容读分支解析（可选尾段取默认值）⇒ `TableMeta` 的字段与新库一致 ⇒ 查询与写入路径与新库同型。
  - 写入某表 ⇒ 该表目录行被整行重写（当前布局）⇒ 同库其它表仍为旧代际 ⇒ 旧库仍可打开、查询、写入。
  - 打开 ⇒ 恢复消费写入日志 ⇒ 打开时重建按持久目录枚举活页 ⇒ 活页不进入可分配页集合；旧库的空闲页（已删除表遗留）仍可被分配。

**Implementation Guidance**

1. **先产出样本，再写断言**。样本是本 task 的前置：没有真实旧字节就没有可断言的对象。样本生产的完整命令见 Task Definitions 的 T1「First test」。
2. **样本必须是固定字节输入**：提交进仓库，测试运行时复制到临时目录再打开。**不得**在测试里伪造版本号、伪造行布局、按版本号分支；**不得**引入任何版本/哈希/修订比对。
3. **断言要落在可观察结果上**：行集与行数、错误文本的关键片段与语句序号、退出码语义、完整性 error 级发现数、目录行字节（仅「写入后目录行含当前布局尾段」这一条需要读原始页）。
4. **走真实入口**：用真实文件库 + `Database::open` / `close`；dump/restore 语义按 `rtsql` 的既有合同（逐条提交、失败即停、退出码）。不要在框架外另建判定层或判定脚本。
5. **样本规模保持小**：行数 ≪ 300，避开 ISS23 的栈溢出触发面（本次已验证普通 INSERT/import 500 行不触发该缺陷，其记录触发面为 `ALTER TABLE DROP COLUMN` 的完成路径，与本场景无关）。
6. **目录行升级的观察方法**：用 `FileStorage::open` + `AsyncStorage::read_page(PageId(0))` 读 `__tables` 所在页，用 `SlottedPageRef::new(page.data)` 取槽数据，沿页头 `next_page_id` 走目录链，找到表名对应的行字节，断言其尾段是当前布局（唯一根计数 0、二级索引计数 0、表类型字节 0x01）。若目录行被分页到后续页，沿链继续找。
7. **命名**：测试文件 `tests/legacy_compat_test.rs`；样本目录 `tests/fixtures/legacy/`；样本来源说明 `tests/fixtures/legacy/README.md`。用例名按 `tasks.md` 的 RTM 建议命名（可调整，须在 Act Response 记录实际名称）。

**Task Definitions**

### T1: 兼容样本产出与刻画断言

- Requirement/Scenario: `legacy-format-compat` R1（最旧代际库文件属于支持范围）、R2（旧库打开/查询/写入、写入后目录行升级、完整性零 error）、R3（打开时重建不放弃、旧库带未决日志恢复）、R4（旧 dump 恢复范围）、R5（三类范围外明确拒绝）；`database-file-format-header` 增补 requirement（唯一格式版本、版本是产品格式契约——由既有 `tests/file_header_test.rs` 覆盖，本 task 不新增用例）。
- Depends on: None
- Targets:
  - `tests/legacy_compat_test.rs`（新建）
  - `tests/fixtures/legacy/legacy_g1.db`（新建，固定字节样本）
  - `tests/fixtures/legacy/legacy_g1_pending_wal.db` + `tests/fixtures/legacy/legacy_g1_pending_wal.wal`（新建，固定字节样本对）
  - `tests/fixtures/legacy/legacy_dump.sql`、`legacy_dump_non_integer_pk.sql`、`legacy_dump_non_integer_unique.sql`、`legacy_dump_null_in_not_null.sql`（新建，固定文本样本）
  - `tests/fixtures/legacy/README.md`（新建：样本来源、旧构建命令、样本内容清单、再生步骤、已知限制）
- Current behavior: 旧代际库与旧 dump 的兼容行为当前已成立（见 Investigation Facts 的运行证据），但**没有任何仓库内样本与断言**固定它；README 只有一条负面边界描述（「旧 dump 在新库 restore 被拒」）。
- Required behavior:
  1. 仓库内存在上述固定字节样本；样本由更早版本的产品构建产出，来源与再生步骤记录在 `tests/fixtures/legacy/README.md`。
  2. `tests/legacy_compat_test.rs` 覆盖并断言以下用例（用例名可在不改变断言语义的前提下调整）：
     - `legacy_generation_db_is_in_supported_range`：打开 G1 样本，断言表名、列名、列类型与主键列可读（证明样本确实落在声明的支持范围内）。
     - `legacy_db_opens_queries_and_writes`：打开 → 查询 3 行 → 插入 1 行 → 关闭重开 → 4 行可查；`check_integrity` 的 error 级发现数为 0。
     - `legacy_db_write_upgrades_touched_table_row_to_current_layout`：触发目录行变更后，沿目录链找到该表行字节，断言其尾段为当前布局（唯一根计数 0、二级索引计数 0、表类型字节 0x01），并断言**同库另一张未被写到的表**的行字节仍为旧代际（在这些尾段之前即结束）。⇒ 因此 G1 样本 SHALL 含**两张表**（一张被写到、一张不被写到）。
     - `legacy_db_open_rebuild_keeps_live_table_intact`：打开含已删除表的旧库，断言打开成功、无「重建放弃」输出、活表行完整、error 级发现数为 0。
     - `legacy_db_with_pending_wal_recovers_committed_rows`：打开带未决日志的旧库，断言恢复后可查询且行数 ≥ 1、error 级发现数为 0。
     - `legacy_dump_restores_into_current_db`：恢复旧 dump 到新建空库，断言成功且行数据与样本预期一致；断言恢复出的库可再次导出并保持等价。
     - `legacy_dump_with_non_integer_primary_key_is_rejected`：断言恢复失败、错误文本含「NON-INTEGER PRIMARY KEY」与语句序号、退出语义为失败。
     - `legacy_dump_with_non_integer_unique_is_rejected`：断言恢复失败、错误文本含「UNIQUE (INT columns only)」与语句序号。
     - `legacy_dump_with_null_in_not_null_is_rejected`：断言恢复失败、错误文本含「NOT NULL constraint violation」与列名、并含语句序号。
  3. 断言只使用既有可观察面：行集/行数、错误文本与语句序号、完整性 error 级发现数、目录行原始字节。
- Preserve:
  - 产品代码零改动（`src/` 下任何文件都不得修改）。
  - 既有语义不动：restore 的逐条 auto-commit 与 fail-fast 合同、退出码 1/3/4 语义、dump 的 DDL 渲染规则（含「仅 INTEGER 主键渲染 `PRIMARY KEY`」）、文件头版本拒绝分类、打开顺序守卫。
  - 既有测试文件不改（`tests/file_header_test.rs` 等只读引用，不新增重复断言）。
  - 工作区既有未提交改动不回滚、不清理、不提交。
- Forbidden:
  - 在测试或样本中伪造文件版本号、伪造目录行布局、按版本号分支。
  - 任何形式的身份证明机制：版本/哈希/指纹比对、修订固定、运行标识、产物清单、时间顺序证明及其采集或审计工具。
  - 框架外的判定脚本、封装命令、专用参数或判定层。
  - 修改 `src/` 下任何产品代码；修改 `openspec/specs/`。
  - 把 ISS23–ISS25 的修复顺手纳入本 task。
  - 用默认并行度或大表规模跑断言。
- First test: **变更前 GREEN（刻画断言）**——本 task 不含产品代码改动，用例断言的是当前已验证成立的既有行为，预期写完即 GREEN。样本生产（先行步骤，必须先于断言落地）：

  ```bash
  # 1) 隔离克隆并构建旧版本（仓库之外，不触碰工作区现场）
  git clone --no-hardlinks --shared /home/daivy/projects/RTsql /tmp/rtsql-legacy
  cd /tmp/rtsql-legacy && git checkout d8165fc
  cargo build --offline --bin rtsql            # rustc 1.99.1 下可构建
  OLD=/tmp/rtsql-legacy/target/debug/rtsql

  # 2) G1 库文件样本（两张表：users 3 行 + audit 1 行；audit 用于观察「未被写到的表保持旧代际」）
  rm -rf /tmp/legacy-fixture && mkdir -p /tmp/legacy-fixture
  $OLD new /tmp/legacy-fixture/legacy.db
  $OLD /tmp/legacy-fixture/legacy.db "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score FLOAT);"
  $OLD /tmp/legacy-fixture/legacy.db "INSERT INTO users VALUES (1, 'alice', 9.5), (2, 'bob', 7.25), (3, 'carol', 8.0);"
  $OLD /tmp/legacy-fixture/legacy.db "CREATE TABLE audit (id INTEGER PRIMARY KEY, note TEXT);"
  $OLD /tmp/legacy-fixture/legacy.db "INSERT INTO audit VALUES (1, 'kept');"
  #    产物：/tmp/legacy-fixture/legacy.db（G1 目录行，两张表）

  # 3) G1 库 + 未决写入日志样本（导入中途 SIGKILL，不 checkpoint）
  #    小表规模：先建表，再用小 CSV 导入并在写入途中 kill -9，保留未决日志
  #    产物：/tmp/legacy-fixture/pending.db + /tmp/legacy-fixture/pending.wal

  # 4) 旧 dump 文本样本（由旧构建 dump 产出）
  $OLD dump /tmp/legacy-fixture/legacy.db > /tmp/legacy-fixture/legacy_dump.sql
  #    另用旧构建分别建「非整数主键表」「非整数 UNIQUE 表」「NOT NULL 含 NULL 表」并 dump，
  #    得到另三份旧 dump 文本样本
  ```

  样本落地后：`cargo test --test legacy_compat_test -- --test-threads=1` ⇒ 全部用例通过（期望退出码 0）。**若任一断言不成立**：说明当前行为与本 change 声明的支持范围不一致，按 Stop when 停止并回报，**不得**在本 task 内改产品代码。
- GREEN condition: `cargo test --test legacy_compat_test -- --test-threads=1` 退出码 0，全部用例通过；仓库内存在 7 个样本文件 + 1 份来源说明；`git status` 显示 `src/` 下无任何改动。
- Verification:
  - 自动：`cargo test --test legacy_compat_test -- --test-threads=1` ⇒ 退出码 0（行为类 scenario 走真实库 API 与真实 dump/restore 语义）。
  - 自动（样本确为旧布局）：对提交进仓库的样本文件本身做一次静态核对——`__tables` 行字节在 `u32 数据页尾指针` 之后即结束（无唯一根段 / 二级索引段 / 表类型字节），`__columns` 行无默认值段；该核对在测试内以断言实现（`legacy_generation_db_is_in_supported_range` 读原始页校验），不依赖人工比对。
  - 人工（仅当自动命令无法判定时）：确认 `tests/fixtures/legacy/README.md` 记录的来源命令与实际产出一致（旧构建 revision、样本内容清单、再生步骤齐全）。
  - 失败含义：任一用例失败即本 Cycle 未达成；不得以「重跑直到通过」结案。
- Stop when:
  - 任一刻画断言不成立（即当前行为与声明的支持范围冲突）→ 停止，作为行为缺陷上报并返回 Plan（需要新的执行依据时走 replan），**不在本 task 内改产品代码**。
  - 旧构建在本环境不可构建（工具链不兼容 / 依赖不可得）→ 停止并回报具体失败条件与已尝试的替代路径（此时「样本如何获得」成为需要用户决定的实质问题）。
  - 样本规模不得不触发 ISS23 的栈溢出路径 → 缩小样本而不是绕过缺陷。
  - 需要修改 `src/` 下任何产品代码才能让断言成立 → 停止并返回 Plan。

**Invariants**

- 产品代码零改动；本 change 不改变任何可观察行为。
- 版本是产品格式契约：不得引入任何运行身份匹配机制；样本是固定输入，运行期不做版本/哈希/修订比对。
- 支持范围内行为（打开/查询/写入/恢复）与范围外拒绝形态必须与 `design.md` D1–D6 的结论一致；测试断言即这些结论的可执行形式。
- 既有语义零回归：文件头拒绝分类、restore 逐条提交与 fail-fast、dump 渲染规则、打开顺序守卫、完整性扫描只读语义。
- 样本固定、可复现、与工作区现场无关（旧构建在仓库之外的隔离目录中构建）。
- 工作区既有未提交改动不回滚、不清理、不提交。

**Non-goals**

- 支持范围的规格文本与 README 表述（Iteration 001 的 T2/T3）。
- 收尾全量回归（Iteration 001 的 T4）。
- 旧库格式迁移/升级工具；把旧库「升级」为新格式的一次性转换命令。
- `restore` 的原子化或全量预检语义；多表 dump 部分恢复语义的改变。
- `dump` 对非整数 `UNIQUE` 列的渲染规则调整（`design.md` D9）。
- 加密旧库的专项覆盖（本次已顺带验证当前构建可打开旧构建的加密库，但不作为独立断言项）。
- CI / 预编译发布 / crates.io 发布（I051–I053）；键编码格式变更（I024）；伴生文件加密（I060）；ISS22–ISS25。

**Acceptance**

| # | Acceptance | Requirement | 判定 |
|---|---|---|---|
| A1 | 仓库内存在由更早版本产品构建产出的固定字节样本（旧库文件、旧库 + 未决日志、旧 dump 文本四份），来源与再生步骤记录在 `tests/fixtures/legacy/README.md` | LFC R1 | 样本文件存在且 README 含旧构建 revision 与产出命令 |
| A2 | 旧代际库能打开、查询、写入；写入后重开可读回新行；既有已提交行不丢失；完整性扫描 error 级发现数为 0 | LFC R2 | `legacy_db_opens_queries_and_writes` |
| A3 | 写入使被写到的表目录行升级为当前布局，同库其它表保持旧代际 | LFC R2 | `legacy_db_write_upgrades_touched_table_row_to_current_layout` |
| A4 | 旧代际库打开时重建不放弃，活表完整，error 级发现数为 0 | LFC R3 | `legacy_db_open_rebuild_keeps_live_table_intact` |
| A5 | 旧代际库带未决写入日志打开后恢复出已提交行，error 级发现数为 0 | LFC R3 | `legacy_db_with_pending_wal_recovers_committed_rows` |
| A6 | 旧 dump 恢复到当前库成功且行数据等价；含日期/时间戳/布尔字面量的旧 dump 同样可恢复 | LFC R4 | `legacy_dump_restores_into_current_db` |
| A7 | 三类范围外输入给出明确、定位到语句的拒绝（点名原因 + 第 k 条/共 n 条 + 失败退出码） | LFC R5 | 三个拒绝用例 |
| A8 | 文件头版本高于当前支持版本与不含文件头的文件按既有分类明确拒绝 | DFH 增补 requirement | 既有 `tests/file_header_test.rs` 覆盖（引用，不新增） |
| A9 | 产品代码零改动；样本与断言不引入任何身份证明机制或框架外判定层 | 设计不变量 | `git status` 显示 `src/` 无改动；代码自检 |

**Verification**

| Acceptance | 最简直接判定 | 通过依据 |
|---|---|---|
| A1 | 读 `tests/fixtures/legacy/README.md` + 列出样本文件 | README 含旧构建 revision（`d8165fc`）、产出命令、样本内容清单、再生步骤；7 个样本文件在库内 |
| A2 | `cargo test --test legacy_compat_test legacy_db_opens_queries_and_writes -- --test-threads=1` | 退出码 0；用例断言 3 行 → 插入 → 重开 4 行 → error 发现数 0 |
| A3 | `cargo test --test legacy_compat_test legacy_db_write_upgrades_touched_table_row_to_current_layout -- --test-threads=1` | 退出码 0；目录行尾段为当前布局、其它表行仍为旧代际 |
| A4 | `cargo test --test legacy_compat_test legacy_db_open_rebuild_keeps_live_table_intact -- --test-threads=1` | 退出码 0；无重建放弃输出、活表行完整、error 发现数 0 |
| A5 | `cargo test --test legacy_compat_test legacy_db_with_pending_wal_recovers_committed_rows -- --test-threads=1` | 退出码 0；恢复出行数 ≥ 1、error 发现数 0 |
| A6 | `cargo test --test legacy_compat_test legacy_dump_restores_into_current_db -- --test-threads=1` | 退出码 0；恢复行与样本预期一致 |
| A7 | `cargo test --test legacy_compat_test legacy_dump_with_ -- --test-threads=1` | 退出码 0；三个用例分别断言错误文本关键片段与语句序号 |
| A8 | `cargo test --test file_header_test -- --test-threads=1` | 退出码 0（既有覆盖，回归确认） |
| A9 | `git status --short` | `src/` 下无改动；`tests/legacy_compat_test.rs` 与 `tests/fixtures/legacy/` 为本 Cycle 新增 |

**Gate 2 Readiness**

PASS —— 调查已完成（实现事实与运行证据齐备）、设计已闭合（支持范围与拒绝形态已定，无 TBD）、任务与 Iteration Map 已规划、RTM 无 `Missing` 且无 `Simplified`、`Persisted Evidence` 模式已定为 `none`、Gate 1 的用户批准已按 goal 指令取得。无 `BLOCKED`、无 `SKIPPED`。

**Persisted Evidence**

- Mode: none

理由：验证结论（用例通过计数、样本字节事实）由 Act Response 承载即可；样本本身是仓库内的固定输入（提交物，不是 Evidence）；旧构建可低成本重建，测试运行不依赖旧工具链；没有「不可低成本重跑」或「只有 Evidence 能支撑的 Acceptance」。不设 `required` 项。

**Risks and Notes**

- **样本生产的工具链依赖**：旧构建在当前工具链（rustc 1.99.1）下可构建（本次已验证）。若后续环境变化导致不可构建，样本已是仓库内固定字节，**测试不依赖旧构建**；只有「重新生成样本」才需要旧工具链。
- **未决日志样本的行数依赖**：该样本由「导入中途 SIGKILL」产生，恢复后的行数取决于被杀时机。因此断言只要求「恢复出行数 ≥ 1 + 完整性零 error」，**不硬编码行数**；精确行数记录在 `tests/fixtures/legacy/README.md` 供人工参考。若样本被重新生成，只需更新 README 的参考值。
- **样本表数量**：本次探针用的是单表样本；计划产出的 G1 样本含**两张表**（`users` 被写到、`audit` 不被写到），以便 A3「未被写到的表保持旧代际」可直接观察。样本内容因此比探针多一张表，其余内容一致。
- **ISS23 边界**：本次实测普通 INSERT/import 500 行不触发该缺陷（其记录触发面为 `ALTER TABLE DROP COLUMN` 的完成路径），但样本仍按「行数 ≪ 300」取小规模，避免依赖这条未经规格固定的观察。
- **目录行升级的断言依赖原始页读取**：用到 `AsyncStorage::read_page` 与 `SlottedPageRef`（均为公开 API）。若读页路径在实现中变化（例如目录页不再固定在页 0），断言须沿目录链定位而不是写死页号。
- **非实质未知项（留给 Act 的局部选择，不阻塞）**：样本文件名、用例命名、断言的具体辅助函数拆分；旧 dump 文本样本是否合并为一份文件；目录链遍历的辅助写法。
- **范围外观察（不在本 Cycle 处理）**：当前 `dump` 对非整数 `UNIQUE` 列无条件渲染 `UNIQUE`，导致「旧库 → dump → 新库 restore」在该类表上被计划期拒绝。这是既有行为（不是缺陷），可选改进见 `design.md` D9。
- **Runbook 候选**：R48 的批切分算法在某些文件数下会产出不足 8 个目标的末批；本 change 的收尾全量改用等分规则（`design.md` D10）。该差异作为 Runbook 候选上报，由 `openspec-experience-recorder` 决定是否更新 R48；本 change 不改 Runbook。

## Act Response

- Status: reported

### T1 状态

`done` —— Gate 3 与 Gate 4 均通过；Task Definition 的 Verification 全部满足。本 task 为**表征型**（刻画断言），无产品代码改动，**首次运行即 GREEN**（未人为制造 RED；无 RED→GREEN 环节）。Plan Review（Findings F1/F2）要求的两项断言修复已在**当前 Cycle 内**完成并验证；本 Response 是包含原 T1 实施与本轮修复的**最新完整当前状态**（整体覆盖，不追加逐轮历史）。

### 本轮（Plan Review 要求）修复摘要

| 项 | 状态 | 内容 |
|---|---|---|
| **F1**（阻塞 · Plan 错误）：A5「恢复出已提交行」的断言无法区分「未决日志被重放」与「未被重放」 | **已修复** | 门槛由「恢复行数 ≥ 1」改为「**恢复行数 == 参考值 133**」，并补「恢复前的种子行不得因重放丢失」断言；判别力已实测（见下） |
| **F2**（覆盖缺口 · 行为当前正确）：会触发页分配的写入之后，未断言既有已提交数据仍存活、完整性仍为零 error | **已修复** | A3 内增「写入确实分配新页」的文件大小断言 + 「重开 → 既有行与新增行均可读 → `check_integrity` 零 error」断言 |

### 实际改动（文件与符号）

| 文件 | 变更 | 说明 |
|---|---|---|
| `tests/legacy_compat_test.rs` | 新建 + **本轮精准修改**（719 → 838 行） | 9 个用例 + 14 个辅助项（13 个函数 + 1 个结构体）；本轮新增常量 `RECOVERED_ROWS_REFERENCE`（`:601`），改写 A5 断言（`:624`），扩写 A3 断言（`:407`–`:539`），文件头断言面清单补一项 |
| `tests/fixtures/legacy/legacy_g1.db` | 新建 | 固定字节样本：旧代际库文件（**字节未变**） |
| `tests/fixtures/legacy/legacy_g1_pending_wal.db` | 新建 | 固定字节样本：旧代际库（**字节未变**） |
| `tests/fixtures/legacy/legacy_g1_pending_wal.wal` | 新建 | 固定字节样本：未决写入日志（与上库同名；**字节未变**） |
| `tests/fixtures/legacy/legacy_dump.sql` | 新建 | 固定文本样本：旧 dump（短类型名 + 日期/时间戳/布尔字面量；**字节未变**） |
| `tests/fixtures/legacy/legacy_dump_non_integer_pk.sql` | 新建 | 固定文本样本：范围外（非整数主键；**字节未变**） |
| `tests/fixtures/legacy/legacy_dump_non_integer_unique.sql` | 新建 | 固定文本样本：范围外（非整数 UNIQUE；**字节未变**） |
| `tests/fixtures/legacy/legacy_dump_null_in_not_null.sql` | 新建 | 固定文本样本：范围外（NOT NULL 含 NULL；**字节未变**） |
| `tests/fixtures/legacy/README.md` | 新建 + **本轮仅改参考值文本**（8495 → 9052 字节） | 样本来源、旧构建命令、内容清单、再生步骤、已知限制；本轮改两处「恢复行数参考值」的**表述**（参考值数字 133 本身未变） |
| `openspec/changes/2026-10-24-ms36-legacy-format-and-dump-compat/tasks.md` | 精准修改 | T1 状态 `in_progress`→`done`；「最终状态与 Iteration Map 终态」的 change 状态行与 Iteration 000 行 |

`tests/legacy_compat_test.rs` 的用例（Plan Context 的 Required behavior 逐条对应，用例名与 Plan 建议一致，本轮**未增删用例**）：

`legacy_generation_db_is_in_supported_range`、`legacy_db_opens_queries_and_writes`、`legacy_db_write_upgrades_touched_table_row_to_current_layout`、`legacy_db_open_rebuild_keeps_live_table_intact`、`legacy_db_with_pending_wal_recovers_committed_rows`、`legacy_dump_restores_into_current_db`、`legacy_dump_with_non_integer_primary_key_is_rejected`、`legacy_dump_with_non_integer_unique_is_rejected`、`legacy_dump_with_null_in_not_null_is_rejected`。

辅助符号（13 个函数 + 结构体 `CliOut` + 本轮新增常量）：`fixture_path`、`copy_fixture`、`CliOut` / `run_cli`、`cli_error_message`、`cli_rows`、`db_rows`、`read_chain_rows`、`tables_row_name`、`g1_tables_row_len`、`find_table_row`、`newest_table_row`、`column_row_names`、`g1_columns_row_len`、`RECOVERED_ROWS_REFERENCE`。

`src/` 下**零改动**（`git status --short -- src/` 与实施前基线逐行一致：20 个 `M` + 1 个 `?? src/storage/free_page_rebuild.rs`，全部为 MS34/MS35 预存在改动）。既有测试文件未改。

### F1：门槛的选择与取舍

**采用形态 ③（Plan Review 的可接受形态之一）**：断言「恢复行数 == README 记录的参考值（133）」，并在同一用例补「恢复前的种子行（`id = 0`）仍存在」。

修改后的断言措辞（`tests/legacy_compat_test.rs:624`）：

```rust
    assert_eq!(
        rows.len(),
        RECOVERED_ROWS_REFERENCE,
        "the pending write log must replay into recovered rows \
         (expected {RECOVERED_ROWS_REFERENCE} recovered rows, got {}); \
         without the pending log only the 1 seeded row is present",
        rows.len()
    );
    // 恢复前的既有已提交行（种子行）不得因重放而丢失。
    assert!(
        rows.iter().any(|r| r[0].as_i64() == Some(0)),
        "the committed seed row must survive the replay of the pending write log"
    );
```

取舍理由：

- **判别力最强**：133 是**已提交样本字节的属性**（确定）——同一份字节恢复出的行集恒定；因此「未重放」（只剩 1 行种子）与「完整重放」（133 行）被门槛明确区分，且**部分重放**也会失败。形态 ①（`> 种子行数`）与形态 ②（`含 id > 0`）只区分「有/无重放」，对部分重放无判别力。
- **代价**：断言与样本再生耦合——重新生成样本（旧构建被杀时机不同）会改变恢复行数，届时须同步更新 README 的参考值与用例常量（`RECOVERED_ROWS_REFERENCE`）。该耦合已在 README 的「参考值」段与「已知限制」段写明，并在常量注释处指向 README（参考值的权威位置在 README，用例只是可执行判定）。
- **不选 ①/② 的理由**：两者都会把「恢复行数」这一事实留在门槛之外（只要求「多于 1 行」或「存在某一行 id > 0」），无法区分完整重放与部分重放；本 change 的目的正是把「旧库带未决日志能恢复出已提交行」固定为可复跑的基线。
- **未引入任何身份机制**：门槛比对的是**行数**这一内容属性，不是版本 / 哈希 / 修订 / 运行标识（I092 不适用）。

### F1：判别力的实证

**(a) 未决日志缺席时，修复后的门槛确实失败**（Plan Review 的复现方法：只复制 `.db`，不带 `.wal`）。

真实 CLI 入口（当前构建）：

```
$ cp tests/fixtures/legacy/legacy_g1_pending_wal.db /tmp/ms36-f1-nofwal/   # 不带 .wal
$ ./target/debug/rtsql /tmp/ms36-f1-nofwal/legacy_g1_pending_wal.db "SELECT * FROM t;"
{"columns":["id","v"],"rows":[[0,"seed"]]}
select exit=0
$ ./target/debug/rtsql check /tmp/ms36-f1-nofwal/legacy_g1_pending_wal.db
check ok: 7 pages scanned, no corruption found
check exit=0
```

⇒ 未决日志缺席时只有 **1 行**、完整性仍为 `check ok` ⇒ **旧门槛（`rows.len() >= 1`）在此情形下仍会通过**（Plan Review F1 的判断在当前构建上复现成立）。

同一门槛在测试框架内的实测失败（临时探针用例，与 A5 使用**完全相同**的门槛；探针只用于取证，用完即删，不在交付物内）：

```
$ cargo test --offline --test legacy_compat_test tmp_probe_a5_threshold_without_pending_wal -- --test-threads=1
running 1 test
test tmp_probe_a5_threshold_without_pending_wal ... FAILED

---- tmp_probe_a5_threshold_without_pending_wal stdout ----
thread 'tmp_probe_a5_threshold_without_pending_wal' panicked at tests/legacy_compat_test.rs:658:5:
assertion `left == right` failed: the pending write log must replay into recovered rows (expected 133 recovered rows, got 1); without the pending log only the 1 seeded row is present
  left: 1
 right: 133

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 9 filtered out; finished in 0.00s
```

**(b) 带未决日志时通过**（同一构建、同一门槛）：

```
$ ./target/debug/rtsql /tmp/ms36-f1-withwal/legacy_g1_pending_wal.db "SELECT COUNT(*) FROM t;"
{"columns":["count_star"],"rows":[[133]]}
count exit=0
$ ./target/debug/rtsql check /tmp/ms36-f1-withwal/legacy_g1_pending_wal.db
check ok: 10 pages scanned, no corruption found
check exit=0
```

用例侧：A5 用例在最终全文件运行中通过（`test legacy_db_with_pending_wal_recovers_committed_rows ... ok`）。

⇒ 修复后的 A5 门槛**可区分「重放」与「未重放」**：未决日志缺席 → 断言失败（`left: 1, right: 133`）；未决日志在场 → 断言成立。

### F2：新增断言的实际内容与判别力

修改后的 A3（`tests/legacy_compat_test.rs:407`–`:539`）在既有的「目录行升级 / 其它表保持旧代际」断言之外，新增两组断言：

**(1) 这次写入确实分配了新页**（把断言锚定在「会分配页」的路径上）：

```rust
    let size_after_write = std::fs::metadata(&db).expect("stat legacy db").len();
    assert!(
        size_after_write > size_before_write,
        "the bulk write must allocate a new page \
         (db file grew from {size_before_write} to {size_after_write} bytes)"
    );
```

**(2) 重开 → 行可读 → 完整性零 error**：

```rust
        assert_eq!(users_rows.len(), 123, "the pre-existing rows and the newly written rows must all be readable");
        assert_eq!(ids, (1..=123).collect::<Vec<i64>>(), "every written id must be readable exactly once after the reopen");
        assert_eq!(name_of(1), "alice", "the first committed row must survive");
        assert_eq!(name_of(2), "bob", "the second committed row must survive");
        assert_eq!(name_of(3), "carol", "the third committed row must survive");
        assert_eq!(name_of(123), "user123", "the last written row must be readable");
        assert_eq!(audit_rows, vec![vec![serde_json::json!(1), serde_json::json!("kept")]], "the untouched table's committed row must survive the allocating write");
        assert_eq!(events_rows.len(), 1, "the second untouched table's committed row must survive the allocating write");
        assert_eq!(report.error_count(), 0, "integrity findings after the allocating write: {:?}", report.findings);
```

判别力说明（实际观测值，来自临时探针，取证后删除）：

```
PROBE file size before write = 41024, after write = 45120
PROBE users rows = 123, first = Some([Number(1), String("alice"), Number(9.5)]), last = Some([Number(123), String("user123"), Number(1.5)]);
       audit rows = [[Number(1), String("kept")]];
       events rows = [[Number(1), String("2026-01-02"), String("2026-01-02 03:04:05"), Bool(true)]];
       integrity total_pages = 11, error_count = 0, info_count = 0
```

- **确实在「会分配页」的路径上**：库文件由 41024 字节（64 字节文件头 + 10 页 × 4096）增长到 45120 字节（64 + 11 页 × 4096）——**恰好分配了一个新页**；完整性扫描报告 `total_pages = 11`。对照：A2 的单行插入**不分配**（不触发文件增长），因此本路径是 Plan Review 指出的唯一分配路径。
- **断言不是恒真**：把期望行数故意改错一位后重跑，断言失败并报出真实行集（`left: 123, right: 122`），证明断言绑定的是**重开后真实读回的行集**，不是空断言：

```
thread 'tmp_probe_f2_allocating_write_path' panicked at tests/legacy_compat_test.rs:725:5:
assertion `left == right` failed: deliberately wrong expectation to show the assertion is bound to the real row set
  left: 123
 right: 122
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 10 filtered out; finished in 0.05s
```

- 覆盖了 Review 指出的两个缺口：R2「写入 SHALL NOT 降低或丢失该库中既有的已提交数据」（`users` 的 3 行既有行 + 未被写到的 `audit` / `events` 的既有行在分配页的写入之后仍在）与 R3「其任何页都不进入可分配页集合」（若活页被误判为可分配页并被覆写，重开后既有行会丢失或完整性出现 error 级发现——两者都被断言锁住）。

### 兼容样本的落地证据（原实施，事实未变）

| 项 | 事实 |
|---|---|
| 样本路径 | `tests/fixtures/legacy/`（7 个样本文件 + `README.md`） |
| 来源 revision | `d8165fc634fa650d839811b1d6b0165e204cad01`（2026-09-24，MS23 落库前的最后一次提交） |
| 来源命令 | `git clone --no-hardlinks --shared <repo> /tmp/rtsql-legacy` → `git checkout d8165fc` → `cargo build --offline --bin rtsql`（rustc 1.99.1，构建成功，产物 `/tmp/rtsql-legacy/target/debug/rtsql`）；旧构建用 `new` / SQL 语句 / `import` / `DROP TABLE` / `dump` 产出样本 |
| 固定方式 | 作为**固定输入**提交进仓库；测试运行期复制到临时目录再打开/恢复。运行期**不做**任何版本/哈希/修订比对——样本只是固定输入 |
| 字节规模 | `legacy_g1.db` 41024；`legacy_g1_pending_wal.db` 28736；`legacy_g1_pending_wal.wal` 15093；`legacy_dump.sql` 497；`legacy_dump_non_integer_pk.sql` 98；`legacy_dump_non_integer_unique.sql` 107；`legacy_dump_null_in_not_null.sql` 133；`README.md` 9052；目录合计 112K |
| 是否在工作区内生成 | **否**。旧构建的克隆、编译与样本生产全部在仓库之外（`/tmp/rtsql-legacy`、`/tmp/legacy-fixture`）；工作区内只有复制进去的样本文件与 README |
| 清理证明 | 原实施已清理 `/tmp/rtsql-legacy`、`/tmp/legacy-fixture` 与各探针目录并复查无残留；本轮的临时探针用例（仅测试文件内的两个 `#[tokio::test]` 函数）已从 `tests/legacy_compat_test.rs` 删除，最终文件为 9 个用例 ⇒ **测试不依赖旧构建产物、临时目录或任何探针残留** |

样本内容（旧构建真实产出，非运行时伪造）：

- `legacy_g1.db`：活表 `users`（3 行）/ `audit`（1 行）/ `events`（1 行，含 DATE / TIMESTAMP / BOOL）+ 由**旧构建**删掉的 `discarded` 表（遗留空闲页）。目录行全部为 G1 布局（`__tables` 行止于数据页尾指针，`__columns` 行无 DEFAULT 段）。
- `legacy_g1_pending_wal.db` + `.wal`：旧构建 `import` 中途 `SIGKILL`（未 checkpoint）产出的未决写入日志；当前构建恢复出 **133 行**（id 0..132）。本轮起**该数字由用例直接断言**（原为 README 的人工参考值）。
- `legacy_dump.sql`：旧构建 `dump` 产出，`CREATE TABLE "users" ("id" INT PRIMARY KEY, "name" STRING NOT NULL, "score" FLOAT);` 等短类型名 + `DATE '2026-01-02'` / `TIMESTAMP '2026-01-02 03:04:05'` / `TRUE` 字面量。

### 验证命令与实际结果

**本轮最终定向验证（修改后、格式化后）**：

```
$ cargo test --offline --test legacy_compat_test -- --test-threads=1
running 9 tests
test legacy_db_open_rebuild_keeps_live_table_intact ... ok
test legacy_db_opens_queries_and_writes ... ok
test legacy_db_with_pending_wal_recovers_committed_rows ... ok
test legacy_db_write_upgrades_touched_table_row_to_current_layout ... ok
test legacy_dump_restores_into_current_db ... ok
test legacy_dump_with_non_integer_primary_key_is_rejected ... ok
test legacy_dump_with_non_integer_unique_is_rejected ... ok
test legacy_dump_with_null_in_not_null_is_rejected ... ok
test legacy_generation_db_is_in_supported_range ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.27s
```

该命令覆盖本文件全部 9 个用例，因而**逐条覆盖 A1–A9 的判定用例**（A5 与 A3 的断言本轮已改写，其结论由本次运行给出；其余用例的断言未变，覆盖范围未变化）。

**本轮的取证性运行（预期失败，用于证明判别力；探针已删除，不在交付物内）**：

| 命令 | 实际 `test result:` 行 |
|---|---|
| `cargo test --offline --test legacy_compat_test tmp_probe_a5_threshold_without_pending_wal -- --test-threads=1`（未决日志缺席，门槛不变） | `test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 9 filtered out; finished in 0.00s`（断言失败：`left: 1, right: 133`） |
| `cargo test --offline --test legacy_compat_test tmp_probe_f2_allocating_write_path -- --test-threads=1 --nocapture`（打印实际数值 + 故意写错一位的期望） | `test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 10 filtered out; finished in 0.05s`（断言失败：`left: 123, right: 122`；探针输出见 F2 段） |

**格式与静态检查**：

```
$ rustfmt --edition 2021 --check tests/legacy_compat_test.rs
（无输出，退出码 0）
```

**样本字节核对（与本轮改动前的副本逐字节比较）**：

```
$ for f in legacy_g1.db legacy_g1_pending_wal.db legacy_g1_pending_wal.wal legacy_dump.sql \
    legacy_dump_non_integer_pk.sql legacy_dump_non_integer_unique.sql legacy_dump_null_in_not_null.sql; do \
    cmp tests/fixtures/legacy/$f /tmp/ms36-fixture-baseline/$f && echo "identical: $f"; done
identical: legacy_g1.db
identical: legacy_g1_pending_wal.db
identical: legacy_g1_pending_wal.wal
identical: legacy_dump.sql
identical: legacy_dump_non_integer_pk.sql
identical: legacy_dump_non_integer_unique.sql
identical: legacy_dump_null_in_not_null.sql
```

⇒ **7 个样本字节全部未变**；README 只改了参考值**文本**（参考值数字 133 本身未变，文件 8495 → 9052 字节）。

**产品代码零改动（A9）**：

```
$ git status --short -- src/
 M src/database.rs …（20 个 M）
?? src/storage/free_page_rebuild.rs
```

与实施前基线逐行一致（全部为 MS34/MS35 既有现场），本 Cycle 未新增任何 `src/` 改动。

**规格增量未变（requirement / scenario 数量核对）**：

| 域 | Requirement 数 | Scenario 数 | 本轮改动 |
|---|---|---|---|
| `legacy-format-compat`（change delta） | 5（R1–R5） | 14（R1:3、R2:4、R3:2、R4:2、R5:3） | 无 |
| `database-file-format-header`（change delta，ADDED） | 1 | 3 | 无 |
| **合计** | **6** | **17** | **无（未增删）** |

`openspec/changes/2026-10-24-ms36-legacy-format-and-dump-compat/specs/` 两个增量文件本轮**未修改**；`openspec/specs/` 未触碰。

**本 Iteration 不跑全量回归**：高开销验证按 change 预算，收尾全量属 Iteration 001 的 T4（M19 分批串行），本 Cycle 未占用该预算。

### Gate 3 逐项结论

**Spec compliance** — 通过：

- Plan Review 的 F1/F2 修复要求逐条落实：F1 的门槛可区分「重放 / 未重放」（已实测）；F2 在 A3 内补齐「重开 + 行可读 + 完整性」断言，且锚定在**确实分配页**的路径上（文件增长 41024 → 45120）。
- 断言语义只**加强**、未削弱：原 A5 的「恢复出行数 ≥ 1 + 完整性零 error」两条断言都保留（前者收紧为 `== 133`，后者不变），并新增种子行存活断言；A3 原有的目录行布局断言全部保留。
- requirement / scenario 数量未变（6 / 17），规格增量文件与 `openspec/specs/` 未动。
- Preserve 项全部成立：产品代码零改动；restore 逐条 auto-commit 与 fail-fast、退出码 1/3/4 语义、dump 渲染规则、文件头拒绝分类、打开顺序守卫均未触碰；既有测试文件未改；工作区既有未提交改动未回滚、未清理、未提交。
- Forbidden 项全部成立：未伪造版本号/行布局、未按版本号分支；判定全部在 `cargo test` 内（真实库 API + `CARGO_BIN_EXE_rtsql` 子进程），无框架外脚本、封装命令或专用参数；未改 `src/` 与 `openspec/specs/`；未纳入 ISS23–ISS25 的修复；断言规模小（样本行数 ≤ 4，用例最大单表写入 120 行）。
- **无身份证明机制**：新增的「恢复行数 == 参考值」与「库文件大小增长」都是**内容属性**（恢复出的行集、页分配的结果），不是运行身份；全文无版本 / 哈希 / 指纹 / 修订 / 运行标识比对。
- Gate 证据覆盖 Task Definition 的通过条件：7 个样本文件 + 1 份来源说明存在于库内且字节未变；`src/` 零改动；9/9 用例通过。

**Code quality** — 通过：

- 无新增警告（最终编译输出只有 cargo 自身的 `config` 弃用提示，与本文件无关）。
- `rustfmt --edition 2021 --check tests/legacy_compat_test.rs` → **0 差异**（仓库其余文件的 fmt 差异为 MS34/MS35 预存在，未触碰）。
- 无 Critical / Important 问题；遗留 Minor 问题：见下表第 4–6 项（均为 Plan Review 已判定不阻塞、或范围外的既有行为）。

### Gate 4 逐项结论

通过 —— Task Definition 的 Verification 全部满足：9/9 用例通过（当次真实运行的实际 `test result:` 行见上：9 passed / 0 failed）、7 个样本文件字节未变 + `README.md` 参考值文本已同步、`git status` 显示 `src/` 零改动、F1 判别力与 F2 判别力均有实测输出。完成声明基于本次真实运行；A8（`tests/file_header_test.rs` 的既有覆盖，14 passed）沿用**本 Cycle 上一轮 Act Response** 的结论——该文件未被修改、覆盖范围未变化，按公共规则「验证」直接沿用，未重跑。

### 自检发现、已修复内容与遗留 Minor 问题

| # | 发现 | 处置 | 状态 |
|---|---|---|---|
| 1 | **F1（Plan Review，Plan 错误）**：A5 的「≥ 1 行 + 零 error」在未决日志未被重放时同样成立，断言无判别力 | 门槛改为「恢复行数 == 参考值 133」+ 种子行存活断言；判别力实测（未决日志缺席 → `left: 1, right: 133` 失败） | 已修复 |
| 2 | **F2（Plan Review，Plan 遗漏）**：唯一会分配页的写入（A3 的 120 行插入）只断言了目录行布局，未锁定既有数据存活与完整性 | A3 补「文件大小增长（确实分配页）」+「重开 → 行可读 → 完整性零 error」断言 | 已修复 |
| 3 | 新增代码未完全 rustfmt 对齐（2 处：`Database::open(...).await.expect(...)` 换行、`assert_eq!(name_of(123), ...)` 折行） | `rustfmt --edition 2021 tests/legacy_compat_test.rs`（只格式化本文件），复查 0 差异 | 已修复 |
| 4 | 遗留 Minor（F3，Plan Review 判定不阻塞）：R1 正文「G1/G2/G3 全部受支持」中 G2 只有行级既有覆盖 | 不在本 Cycle 处理——由 Iteration 001 的规格文本（T2）以「覆盖层级」一句话消解（Plan Review Follow-up Decision） | 遗留（Plan 已记） |
| 5 | 遗留 Minor（F4，Plan Review 判定不阻塞）：`database-file-format-header` 的 ADDED requirement 与既有 requirement 的两个场景语义重复 | 不在本 Cycle 处理——交规格合并时按既有域对照处理，保留「版本是产品格式契约、不引入身份机制」这一主张 | 遗留（Plan 已记） |
| 6 | 遗留 Minor（F5，Plan Review 判定不阻塞）：R2 场景子句「扫描不修改文件内容」未在本用例内做字节级断言 | 不在本 Cycle 处理——既有跨域行为（`data-integrity-verification` 域已规定只读），非本 change 新增主张 | 遗留（Plan 已记） |
| 7 | 范围外观察（沿用原实施，非本次引入）：多表 dump 在后段语句被拒时**部分恢复**（既有 restore 契约，design D4）；`dump` 无条件渲染非整数 `UNIQUE` 使该类旧表的 dump 不可恢复（既有行为，design D9） | 不实施，记为可选改进 | 遗留（Plan 已记，范围外） |

**遗留 Minor 问题：无未解决项**（第 4–7 项均为 Plan Review 已判定不阻塞、或范围外既有行为，非本次引入的缺陷）。

### Deviations from Plan

1. **A5 门槛偏离 Plan Context 的「恢复出行数 ≥ 1 / 不硬编码行数」**（Plan Context `:226`、Risks `:245`）：改为「恢复行数 == 参考值 133」。该偏离**由 Plan Review 的 F1 修复要求（Follow-up Decision 第 ① 条，列明三种可接受形态）明确授权**，属 Review 要求的修复，不是 Act 的自由发挥。
2. **新增断言面「库文件大小」**：Plan Context 的断言面清单（目录行原始字节、行集与行数、错误文本与语句序号、退出码、完整性 error 发现数）没有这一项。F2 要求把断言锁定在「会触发页分配的写入」路径上，必须有一个**可观察**证据证明该路径确实分配了页；库文件大小是既有可观察面（非新增产品接口、非身份机制），故加入并在文件头注释中登记。
3. **README 参考值文本**随形态 ③ 同步：只改「恢复行数参考值」的表述（参考值数字 133 未变、样本字节未变），属 Plan Review 明确允许的同步动作。
4. **本轮追加的「种子行存活」断言**（A5）：F1 的修复要求只规定门槛形态，未规定是否可加断言；该断言直接支撑 R3 的「恢复出已提交行」（种子行是重放前就已提交的行），且不改变断言语义方向（只加强），故一并实施。
5. （沿用原实施，未变）G1 样本的表数、A3 的触发手段、A4 的 stderr 观察方式、`legacy_dump_null_in_not_null.sql` 的最小编辑——见上一轮记录，结论未变。
6. **本轮未重跑 A8 的 `tests/file_header_test.rs`**：该文件未被修改、覆盖范围未变化，按公共规则「验证」沿用上一轮结论（14 passed），未重复运行已通过的验证。

### Persisted Evidence

`None required` —— 模式为 `none`（Plan 未设 `required` 项），且无需保存：

- F1 / F2 的判别力证据是**当次运行的原始输出**，已逐字记入本 Response（失败断言的完整消息与 `test result:` 行、探针打印的实际数值），未丢失决定性结构；
- 复现成本极低：未决日志缺席的情形只需两条 CLI 命令（`cp` 只带 `.db` → `rtsql … "SELECT * FROM t;"`），可由任何人即时复现；两个临时探针用例是一次性取证代码，已从交付物删除（最终文件 9 个用例，测试通过）；
- 样本本身是仓库内的固定输入（提交物，不是 Evidence）；旧构建可低成本重建，测试运行不依赖旧工具链；
- 不存在「不可低成本重跑」或「只有 Evidence 能支撑的 Acceptance」。

**未创建** `evidence/` 目录。

### Experience Candidates

1. **Runbook 候选**（可承载，沿用上一轮并补充本次经验）：「从旧 revision 产出固定兼容样本」的可复现操作路径 —— 隔离克隆到仓库之外 → `cargo build --offline --bin rtsql` → 用旧构建建库 / `import` 中途 `kill -9` 产未决日志 / `DROP TABLE` 产遗留空闲页 / `dump` 产旧文本 → **只取样本本体入库，伴生 `.wal` / `.checkpoint` 不入库** → 记录恢复行数参考值。已在本 Cycle 端到端验证成功且可重复。**本轮新增的可复用经验**：固定样本的**派生量**（恢复行数、页分配结果）必须由断言直接锁定并与来源说明同步更新——只写成文档里的参考值时，「未重放」与「重放」在断言上不可区分（本 change 的 F1 即为此）。证据：本 Response 的 F1/F2 段 + `tests/fixtures/legacy/README.md`。交 `openspec-experience-recorder` 判断是否新建 Runbook。
2. **Runbook 候选**（承接 Plan 已记的 R48 差异）：M19 分批全量回归的批切分算法在某些文件数下会产出不足 8 个目标的末批；本 change 的收尾全量改用等分规则（design D10）。本 Cycle 未跑全量、未产生新的运行证据，故仅作为候选上报，由 Recorder 决定是否更新 R48。
3. **Issue 候选**：**None** —— 本轮修复未发现当前 change 范围外的实质缺陷（Plan Review 判定的 F1/F2 是**覆盖缺口与断言门槛问题**，不是产品行为缺陷；F1 的行为侧事实——未决日志确实被重放、恢复出 133 行——由本轮实测再次确认成立）。

### 未解决项与停止条件

- 未解决项：无。F1 / F2 已在当前 Cycle 内修复并验证；F3 / F4 / F5 为 Plan Review 判定**不阻塞**的 Minor，按其 Follow-up Decision 不在本 Cycle 处理（分别交 Iteration 001 的 T2 与规格合并处理）。
- 停止边界：本 Act 完成**不构成**对 Recorder / Plan / Maintainer / Archivist 的授权；change 未归档，全局 tasks / SNAPSHOT / 项目记忆 / `openspec/specs/` 未同步，分支未清理，未创建下一 Cycle 或下一 Iteration，`Plan Review` 未填写（`Review Result` 仍为 `pending`）。下一步等待 Plan 复审当前 Cycle。

## Plan Review

- Review Result: accepted

**Review 范围与方法**

只读当前 Cycle、实际代码与 diff、Act Response；按公共规则「验证」沿用 Act 已报告且覆盖范围未失效的结论，只补跑 Act 未覆盖且本次判定必需的检查（本轮：F1 判别力的「未决日志缺席」场景独立复现、样本字节与工作区现场静态核对、规格增量逐条对账）。偏差分类按 Plan 遗漏 / Plan 错误 / Act 偏离 / 现状变化 / 新证据五类；阻塞项与 Minor finding 分列。

**上一轮阻塞项的关闭核对（F1 / F2）**

**F1（Plan 错误 · 阻塞 Acceptance）——已关闭；门槛 ③ 的取舍裁定为可接受**

- 修复后的门槛（`tests/legacy_compat_test.rs:601` 常量 `RECOVERED_ROWS_REFERENCE = 133`；`:624` 行数断言；`:633` 种子行存活断言）：恢复行数 **== 133**。
- 判别力（本次 Review 独立复现，真实 CLI 入口、当前构建）：把 `legacy_g1_pending_wal.db` **单独**复制到 `/tmp`（不带 `.wal`）→ `SELECT COUNT(*) FROM t` → `{"columns":["count_star"],"rows":[[1]]}`，退出码 0；`SELECT * FROM t` → `{"columns":["id","v"],"rows":[[0,"seed"]]}`；`rtsql check` → `check ok: 7 pages scanned, no corruption found`。带 `.wal` 时同一组命令 → `[[133]]`，`check ok: 10 pages scanned`。⇒「未重放」与「完整重放」在门槛上取值不同（1 vs 133）⇒ **断言具备判别力**（Act 另以同一门槛的临时探针在测试框架内实测失败 `left: 1, right: 133`）。
- 前提核对：「133 是已提交字节的确定性属性」成立——`.db` 与 `.wal` 均为仓库内固定输入，恢复是按序重放已提交帧的确定性过程；本次 Review 独立复现得同一数值（Act 本轮、第一轮 Review、Act 上一轮三次独立观察均为 133）。
- 取舍裁定（形态 ③ vs ①/② vs 与再生成无关的不变量）：**采纳形态 ③**。①（行数 > 种子行数）与 ②（存在 `id > 0` 的行）只区分「有/无重放」，对**部分重放**无判别力；与再生成无关的等价门槛只能由测试自行解析未决写入日志的二进制格式得出（等于在测试里复制一份格式真相），代价高于一处常量同步。形态 ③ 的代价是「重新生成样本时须同步 README 参考值与用例常量」——该耦合已写在 `tests/fixtures/legacy/README.md` 的「各样本的预期结论」段与「已知限制」段，以及常量文档注释（`:596`–`:601`），不构成隐藏前提。
- 结论：**F1 已关闭**，当前 Cycle 无需修复。

**F2（Plan 遗漏 · 覆盖缺口；行为当前成立）——已关闭**

- 新增断言（`tests/legacy_compat_test.rs:407`–`:539`）：写入前文件大小在**打开之后**取（`:407`–`:411`，排除打开时重建改写文件的干扰）→ 120 行插入 → 断言文件增长（`:427`–`:432`）→ 重开 → `users` 123 行（`:470`）、id 集合恰为 `1..=123`（`:480`）、既有行 `alice`/`bob`/`carol`（`:496`–`:498`）与新行 `user123`（`:500`）可读、未被写到的 `audit` / `events` 既有行存活（`:513`、`:522`）、`check_integrity().error_count() == 0`（`:531`）。
- 「确实分配了页」的证据是否可靠：`size_after_write > size_before_write` 在本用例中是**场景前提**而非行为断言——未分配新页时库文件不会增长；实测增长恰为一页（41024 → 45120 B = 10 → 11 页，与完整性报告 `total_pages = 11` 一致）。其它可能的增长来源均落在测量窗口之外（打开时重建发生在测量之前；checkpoint 把已分配页写回原位不产生新页）⇒ 该断言不会因错误原因通过。**行为锁**由读回行集与完整性断言承担。
- `users_rows.len() == 123` 与 id 集合 `1..=123` 是否过拟合样本：否——123 = 样本既有 3 行（id 1..3）+ 本用例自己写入的 120 行（id 4..123），期望值由**用例自身的写入**推出而非样本的隐藏属性；它锁定「不丢行、不重复、无空洞」，并与内容断言（`alice`/`bob`/`carol`/`user123`）互为佐证。
- 结论：**F2 已关闭**。

**本轮新增的 Minor finding（均不阻塞）**

**S1（Minor）：`legacy-format-compat` R4 场景「恢复出的库使用当前格式，可继续写入与再次导出」中的「可继续写入」子句无用例断言**——`legacy_dump_restores_into_current_db` 断言了恢复成功、行等价与「再次导出保持等价」，但从未向恢复出的库写入。归属：Iteration 001 的 T2（规格文本须与断言逐条一致：该子句要么改写为可指认的表述，要么由 T2 实测补证）。

**S2（Minor）：R5「产品文档 SHALL 给出可执行的迁移路径」中两条路径尚无执行证据**——非整数 `PRIMARY KEY` 的迁移路径在规划期已实际执行（`tasks.md` Current-State Evidence：「当前 `dump` 对非整数 PK 表不渲染 `PRIMARY KEY`，restore 退出码 0、行等价」）；非 `INT` `UNIQUE`（改列类型或去掉声明）与 `NOT NULL` 含 `NULL`（清理或回填该行）两条路径**未被执行过**。归属：Iteration 001 的 T2（执行三条路径并按实测结果改写规格表述）+ T3（README 只写已验证的路径）。

**F3 / F4 / F5（Minor，沿用上一轮判定，仍不阻塞）**——F3（R1 正文未注明 G2 的行级覆盖层级、G3 由当前构建自身写入）：交 Iteration 001 的 T2 以「覆盖层级」一句话消解；F4（DFH ADDED 的两个场景与既有域「文件由新版创建」「旧无头文件统一拒绝」语义重复）：交规格合并时按既有域对照处理，保留「版本是产品格式契约、不引入身份机制」这一主张；F5（R2 场景子句「扫描不修改文件内容」未做字节级断言）：属既有跨域行为（`data-integrity-verification` 域已规定只读），非本 change 新增主张。

**已独立核对且无发现的问题（结论留痕）**

- **样本字节未变**：7 个样本文件的字节数与 `tests/fixtures/legacy/README.md` 记载逐一一致（`legacy_g1.db` 41024、`legacy_g1_pending_wal.db` 28736、`legacy_g1_pending_wal.wal` 15093、4 份 dump 文本 497 / 98 / 107 / 133）；文件 mtime 显示 7 个样本停留在原实施时刻、仅 `README.md` 为本轮修改时刻 ⇒ 本轮未重写样本字节。README 的参考值 133 与用例常量 `RECOVERED_ROWS_REFERENCE` 一致。
- **产品代码零改动**：`git status --short -- src/` 为 20 个 `M` + 1 个 `??`（`src/storage/free_page_rebuild.rs`），与第一轮 Review 独立观测到的 MS34 / MS35 既有现场逐项一致，本 Cycle 未新增任何 `src/` 改动。
- **无身份证明机制**：`tests/legacy_compat_test.rs` 全文检索 `sha|hash|md5|checksum|fingerprint|revision|run_id|manifest` 仅命中 `std::collections::HashSet` 与文档注释里的样本来源 revision；断言比较的是行数、行内容、目录行字节、错误文本、退出码、完整性发现数、库文件大小等**行为 / 内容属性**。
- **无判定层、无 timer 阈值**：全部判定在 `cargo test` 内（真实库 API + `CARGO_BIN_EXE_rtsql` 子进程）；文件内无 `Instant` / `Duration` / `elapsed` / 超时阈值；工作区无新增脚本、封装命令或专用参数。
- **无探针残留**：交付文件为 9 个 `#[tokio::test]` 用例，无 `tmp_probe` / `#[ignore]` 残留；Act 的临时探针已删除。
- **规格增量与既有 75 域无冲突**：`legacy-format-compat` 为新域（`openspec/specs/` 下无同名域，现有 75 域）；`database-file-format-header` 的 ADDED 不改写既有 requirement 正文。

**Acceptance Gaps**

| # | Acceptance | 判定 | 缺口 |
|---|---|---|---|
| A1 | 固定字节样本 + 来源与再生步骤 | 达成 | — |
| A2 | 旧库打开/查询/写入、重开读回、既有数据不丢、完整性零 error | 达成 | — |
| A3 | 写入使被写到的表目录行升级、未被写到的表保持旧代际 | 达成 | —（F2 已补齐分配路径上的数据存活与完整性锁定） |
| A4 | 打开时重建不放弃、活表完整、完整性零 error | 达成 | — |
| A5 | 带未决写入日志打开后恢复出已提交行、完整性零 error | **达成** | —（F1 门槛已具判别力：未重放 → 1 行 ≠ 133） |
| A6 | 旧 dump 恢复成功且行数据等价（含日期/时间戳/布尔字面量） | 达成 | 「恢复出的库可继续写入」子句无用例断言（S1 → Iteration 001 的 T2） |
| A7 | 三类范围外输入给出明确、定位到语句的拒绝 | 达成 | 三条迁移路径中两条尚无执行证据（S2 → Iteration 001 的 T2 / T3） |
| A8 | 文件头高版本与无头文件按既有分类拒绝 | 达成（既有覆盖引用） | — |
| A9 | 产品代码零改动、无身份机制、无框架外判定层 | 达成 | — |

无阻塞 Acceptance 的缺口。S1 / S2 属 Iteration 001 的 T2 / T3 输入，不阻塞 Iteration 000 的完成。

**Evidence**

本次 Review 实际运行（只读 + `/tmp` 内的定向复核；仓库内未产生任何文件改动）：

| 命令 / 操作 | 观察结果 |
|---|---|
| `git status --short -- src/` | 20 个 `M` + 1 个 `?? src/storage/free_page_rebuild.rs`（MS34 / MS35 既有现场，与上一轮 Review 观测一致） |
| `git status --short`（全量） | 无新增脚本 / 判定层文件；`tests/legacy_compat_test.rs` 与 `tests/fixtures/` 为本 Cycle 新增；其余为既有未提交现场 |
| `ls -l tests/fixtures/legacy/` | 7 个样本文件字节数与 README 记载逐一一致；`README.md` 9052 B（本轮仅改参考值文本） |
| 复制 `.db`（不带 `.wal`）→ `rtsql <abs> "SELECT COUNT(*) FROM t;"` | `{"columns":["count_star"],"rows":[[1]]}`，退出码 0；`SELECT * FROM t` → `[[0,"seed"]]`；`rtsql check` → `check ok: 7 pages scanned, no corruption found` ⇒ **F1 门槛在未重放时取值 1 ≠ 133，断言失败** |
| 复制 `.db` + `.wal` → 同组命令 | `SELECT COUNT(*) FROM t` → `[[133]]`，退出码 0；`rtsql check` → `check ok: 10 pages scanned, no corruption found` ⇒ 门槛成立 |
| `grep -c "^#\[tokio::test\]" tests/legacy_compat_test.rs` | 9（用例名与 Plan Context 的 Required behavior 逐条对应；无探针残留） |
| `grep -nE "Instant|Duration|elapsed|sleep|timeout" tests/legacy_compat_test.rs` | 无命中（无 timer 阈值） |
| `grep -niE "sha\|hash\|md5\|checksum\|fingerprint\|revision\|run_id\|manifest" tests/legacy_compat_test.rs` | 仅命中 `std::collections::HashSet` 与文档注释中的样本来源 revision（无版本 / 哈希比对） |
| `ls openspec/specs/ \| wc -l`；`ls -d openspec/specs/legacy-format-compat` | 75 域；新域名未占用 |

沿用（来源：本 Cycle 的 Act Response「验证命令与实际结果」；被覆盖的代码自该次运行后未变化）：`cargo test --offline --test legacy_compat_test -- --test-threads=1` → `9 passed; 0 failed; 0 ignored`；`rustfmt --edition 2021 --check tests/legacy_compat_test.rs` → 退出码 0；`tests/file_header_test.rs` 14 passed（A8 的既有覆盖，未重跑）。

**Follow-up Decision**

**接受 Iteration 000，不创建 rework Cycle，不做当前 Cycle 修复。**

- F1 与 F2 均已在**当前 Cycle 内**闭合，闭合方式落在既有执行依据内（同一批固定样本、同一测试文件、零产品代码改动）；两项都不需要新的执行依据、新的 Current-State Evidence 或新的 Task Definition ⇒ 不满足 rework / replan 的触发条件（公共规则 › Iteration 与 Cycle 线程）。
- 遗留 Minor finding 的归属已明确，不作为返工理由：F3 → Iteration 001 的 T2；F4 → 规格合并时按既有域对照处理；F5 → 属既有跨域行为；S1 / S2 → Iteration 001 的 T2（逐条对账 + 迁移路径实测）与 T3（只写已验证的路径）。
- 本次 Review 未产生新的 Issue 候选（范围内外的实质缺陷均无）；Act 上报的三项 Runbook 候选维持由 `openspec-experience-recorder` 裁决，本 Review 不代为登记。
- 下一步：展开 Iteration 001（`iterations/001-contract-and-baseline/000-initial.md`，状态 `ready`），承载 T2 / T3 / T4。

**Iteration Plan Update**

无。`tasks.md` 的 Iteration Plan 保持不变：Iteration 001（`001-contract-and-baseline`，T2 / T3 / T4）已按本 Review 展开其 Cycle 文件 `iterations/001-contract-and-baseline/000-initial.md`；M19 分批串行纪律（10 条）与本 change 的批计划（135 文件 + `--lib --bins` = 136 目标 / 13 批）逐条写死在该 Cycle 的 T4 段。

**Next Cycle**

None（Iteration 000 只有 Cycle 000；不创建 rework / replan Cycle）

**Next Iteration**

`openspec/changes/2026-10-24-ms36-legacy-format-and-dump-compat/iterations/001-contract-and-baseline/000-initial.md`（Iteration 001 / Cycle 000，状态 `ready`；承载 T2 / T3 / T4）
