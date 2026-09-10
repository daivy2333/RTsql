# design: MS10-T05 生命周期子命令（new/list/schema/dump/restore/import --csv）

> 调查依据：R20 `.claude/analysis/ms10-t05-lifecycle-subcommands.md`（revision `a5b0a5f`）+ 本会话补充验证（clap 探针、Bool 字面量通道、csv 网络可达、基线 cli_test 25/0/2）。

## 当前行为 vs 目标行为

- 当前：`rtsql` 只有 one-shot 主命令（`CliArgs` 扁平位置参数 `db`/`sql`/`--format`，mod.rs:63-77）；`rtsql list` 等调用被解析为"库名 list + 缺 SQL 参数" → exit 2；建库仅靠主命令静默创建；无 schema 发现、无数据导入导出。
- 目标：`rtsql` 分发 6 个生命周期子命令（`new`/`list`/`schema`/`dump`/`restore`/`import --csv`）；主命令合法输入行为零变化（含静默建库契约）；开库子命令要求库已存在（`new` 是唯一显式创建入口）；dump/restore 以 SQL 文本完成逻辑导出往返；import 以 CSV→表 schema 转换逐条入库。

## 关键决策

### D1 入口重构：`Option` 位置参数 + `Option<Subcommand>`（clap 探针实证）

`CliArgs` 改为 `db: Option<String>` + `sql: Option<String>` + `--format`（`global = true`，子命令可继承）+ `#[command(subcommand)] command: Option<Command>`。`Command` 枚举：`New { target }`、`List`、`Schema { db }`、`Dump { db }`、`Restore { db, file }`、`Import { db, table, file, --csv }`。

clap 4 derive 实证（`/tmp/clap-probe` 探针，clap 4 同源版本）：可选位置参数与子命令共存时，`list`/`new foo`/`schema mydb` 正确分发子命令，`mydb "SQL"` 照常填充位置参数；而**必需**位置参数（`db: String`）会堵死子命令分发（先报缺 `<DB>`）。主命令缺参由手动分支承接（`ExitStatus::Usage`，exit 2）——既有 `test_usage_error_exit_2` 仅断言退出码，兼容。

裸名冲突实证：首参命中子命令名即分发（`list "SELECT 1"` → 子命令臂报多余参数 exit 2）。子命令优先为平台行为，此类库以含 `/` 路径形式打开（`resolve_db_path` 语义不变）；help 文案注明。

### D2 退出码归类（零新增枚举）

| 类 | 产生路径 |
|---|---|
| Usage 2 | 子命令参数缺失/非法（clap 自动）；主命令缺参（手动）；`import` 未提供 `--csv` |
| General 1 | 库文件不存在（schema/dump/restore/import 前置检查）；`new` 目标已存在；目标表不存在；CSV 结构/表头不匹配/类型转换失败；IO 错误；open 其他错误；`list` 基目录无法解析 |
| Sql 3 | restore/import 中 SQL 语句执行失败（fail-fast，含定位） |
| Locked 4 | schema/dump/restore/import 经 `Database::open` 的锁冲突（`open_error_status` 复用）。`new` 因存在性检查先行，锁冲突无稳定观察路径，不入契约 |
| Signaled 128+n | 两阶段信号编排沿用（所有开库子命令） |

### D3 存在性前置检查在编排之前

schema/dump/restore/import 在调用 `execute_command_inner` 前对 resolve 后路径做 `Path::exists` 检查 → `General("<path> does not exist")`；`new` 检查已存在 → `General("... already exists")`。TOCTOU 窗口（检查与 open 之间被第三方创建/锁定）单用户 CLI 可接受，锁兜底在 `FileStorage::open` 内部。

### D4 `new` 复用信号编排（work 为空闭包）

存在性检查 → `create_dir_all` 父目录 → `execute_command_inner(path, |_db| Success, sigint, sigterm)`：open 建库（0 字节写 64B 头 + Catalog::bootstrap，file_storage.rs/catalog.rs 现有链）+ close checkpoint 截断 WAL。风险注记：open 后信号中断可留半初始化文件，重试 `new` 得 already-exists（手工删除即可；不引入事务性文件创建）。

### D5 `list` 纯文件系统枚举

`resolve.rs` 提取 `rtsql_home()` / `db_dir()`（推导逻辑自 `resolve_db_path` 内联段收敛，`resolve_db_path` 行为不变，既有 2 单测零修改）。`list` = `db_dir()` → `read_dir`（目录不存在 → 空行集）→ 过滤扩展名 `.db` 的常规文件 → `(name, size_bytes)` 按名称排序 → `render(kind, ["name","size_bytes"], Rows)` → stdout。不开库、不校验内容。

### D6 DDL 生成器（纯函数 + lib 单测；schema 与 dump 共用）

输入 `CatalogRow` + 按 `column_index` 升序的 `Vec<CatalogColumnRow>`，输出单行 DDL：
`CREATE TABLE "name" ("col" TYPE [PRIMARY KEY] [NOT NULL] [UNIQUE], ...)`。

- 类型映射：Int→`INT`、Float→`FLOAT`、Bool→`BOOL`、`String(_)`→`STRING`。往返依据：planner `convert_data_type`（ddl_dml.rs:133-169）四族归一 + `STRING` 关键字有 cli_test 先例；CLI 建库 String 恒为存储长度 255（`plan.rs:200` 固定转换），`STRING` 关键字 restore 后恒等；非 255 长度仅 lib 直建可出现，DDL 不表达（proposal 边界）。
- 标识符恒双引号包裹（内部 `"` 加倍），规避关键字冲突与大小写问题；catalog 存名即输出名。
- 约束序 PRIMARY KEY → NOT NULL → UNIQUE（`extract_column_constraints` 可解析，ddl_dml.rs:170-198）；DEFAULT 不在 catalog 持久化面（catalog.rs:652-672 序列化无 default 字段），不输出。

### D7 dump：DDL + INSERT 文本流

每表（catalog `scan_tables` 顺序）：D6 DDL 行 → `parse_stage("SELECT * FROM \"t\"")` → `plan_stage(db, stmt_text, stmt, false)` → `execute_stage` → `Response::QueryResult.rows` → 每行一条 `INSERT INTO "t" VALUES (lit, ...);`（列序 = scan_columns 升序，值序即 schema 序）。

字面量纯函数（lib 单测）：JSON Number→十进制（i64 优先，f64 走 f64 Display，往返经 SQL Number 解析 value.rs:11-20）；Bool→`TRUE`/`FALSE`（INSERT 通道已验证：`value.rs:26` `SqlValue::Boolean → Value::Bool`）；String→单引号加倍；Null→`NULL`。Float 非有限值已被 `value_to_json` 转 Null（pipeline.rs:716-726），且 SQL 文本无法表达 NaN——CLI 可达数据无此形态，边界记 Risks。

### D8 restore：静默逐条执行 + 空库前置

存在性检查（D3）→ `execute_command_inner` open → work 内：`catalog().scan_tables()` 非空 → `General` 拒绝（restore 要求空库）；否则读文件（`-` → stdin `read_to_string`）→ `parse_stage` 全串 → 逐条 `plan_stage(db, stmt.to_string(), stmt, false)` + `execute_stage`（不渲染）→ 失败复用 `sql_failure_status`（exit 3，序号 + 语句文本 + 前序已生效注记）。DDL 持久化依赖 close（编排保证）。顺序注记：先空库检查后读文件（避免对拒绝目标白读大文件）。

### D9 import：csv crate + schema 驱动转换 + 逐条 INSERT

新增依赖 `csv = "1"`（crates.io 可达已验证 2026-09-09；RFC4180 引号/转义/跨行由标准实现承载，手写解析为更大且更劣的新增代码）。流程（work 内）：`db.get_table(table)` 取 `TableMeta.columns`（表不存在 → General）→ CSV 首行表头按名匹配：表全部列必须在表头中、表头不得含表外列（违规 → General，含具体列名）→ 每数据行按 header→schema index 重排 → 逐字段类型转换（纯函数，lib 单测）：
- Int 列：`i64::from_str`；Float 列：`f64::from_str`；Bool 列：`true/false` 大小写不敏感；String 列：原样。
- 空字段（空串）→ `Null`（非 String 列）/ `String("")`（String 列）。
- 转换失败 → `General`，文案含数据行号（1-based，不含表头）、列名与原值。

构建 `INSERT INTO "t" VALUES (...)`（全列 schema 序——不使用列清单形式，规避 planner INSERT 列清单支持度未知项，零 planner 依赖）→ `db.execute_sql` 逐条（auto-commit，逐条生效）→ SQL 失败 → `Sql`（文案 `import row {k} of {n} failed: {error}`，fail-fast）→ 全部成功 → `emit_stdout(render(kind, [], Affected(total)))`。未提供 `--csv` flag → `Usage`（当前唯一支持格式）。

### D10 实现组织（非实质，Act 定）

入口与分发在 `src/cli/mod.rs`；子命令实现建议 `src/cli/` 内新子模块（单文件 `lifecycle.rs` 或按命令拆分均可）；D6/D7/D9 纯函数必须可被 lib 单测直接覆盖（`#[cfg(test)]` 或 pub(crate)）。

### D11 约束持久化通道（2026-09-09 Review 修订，001-rework 契约）

D6 原假设「catalog 持久化面含约束、schema 直接可读」经 Act Blocker 证伪于写入侧：catalog 序列化/读取两端就绪（catalog.rs:652-672），但 SQL 建库链在 `CreateTableExecutor`（create_table.rs:46-54 `to_tuple()` 压平）→ `TableManager::create_table`（table_manager.rs:209-214 无约束参数，:282-293 硬编码 false）丢弃 NOT NULL/UNIQUE。修订决策：

1. **运行时语义 = 仅元数据持久化**：写入真实值不引入 INSERT/恢复强制（当前引擎约束解析后即弃、无运行时语义；spec R-schema 只要求输出真实持久化约束；强制属新能力，不做）。
2. **API 兼容 = 签名零变化**：`Database::create_table`（76 处测试/bench 调用）与 `TableManager::create_table`（5 处直调）签名不动、委托新方法并显式 false（行为逐字一致）；新增 `create_table_with_constraints` 供 SQL 路径透传约束。
3. **恢复语义**：restore 经 SQL 建表走同一 executor 路径，约束自然持久化，无需额外处理。

本修订触碰面：`src/executor/create_table.rs`（约束提取与传参）、`src/storage/data/table_manager.rs`（新方法 + 旧签名委托）。上文「storage 层零修改」Invariant 按此收窄为「storage 建库约束通道之外零修改」。规划依据（R20-F9）的教训已由父 Cycle Review 以 PLAN-OMISSION 记录。

## Change Surface

| Task | Requirement | File/Symbol | Current | Planned |
|---|---|---|---|---|
| T1 | R1 分发/冲突/零回归 | `src/cli/mod.rs::CliArgs/run/execute_command` | 扁平位置参数 | Option 参数 + `Command` 枚举 + 手动 usage |
| T2 | R-list/R-new 前置 | `src/cli/resolve.rs` | base 推导内联 | `rtsql_home()`/`db_dir()` helper |
| T3 | R-new | `src/cli/`（新子命令） | 无 | new 全流程（D4） |
| T4 | R-list | `src/cli/` | 无 | list 枚举渲染（D5） |
| T5 | R-schema | `src/cli/` + DDL 生成器 | 无 | DDL 纯函数 + schema 命令（D6） |
| T6 | R-dump-restore (dump) | `src/cli/` + 字面量纯函数 | 无 | dump 流（D7） |
| T7 | R-dump-restore (restore) | `src/cli/` | 无 | restore 流（D8） |
| T8 | R-import | `Cargo.toml` + `src/cli/` | 无 csv 依赖 | csv + import 流（D9） |
| T9 | 验证边界 | 全仓只读 | 671 基线 | 全量门 |

## Invariants

- 主命令 `rtsql <db> "<sql>"` 合法输入行为零变化：渲染、退出码、静默建库、close checkpoint、信号语义（cli_test 既有 25 用例零修改全绿为硬约束）。
- `resolve_db_path` 对外语义不变；`render.rs`、`ExitStatus` 枚举与既有映射不变；pipeline 三 stage 与 `execute_sql`/`execute_in_tx` 签名不变；planner/pipeline/storage 层零修改。
- 既有 671 测试基线零回归；除新增用例外既有测试断言零修改。

## Non-goals

见 proposal Out of Scope：MS11/MS12/MS13 内容；非 255 String 长度保真；NaN/Inf dump 语义；流式 dump/restore；planner 列清单 INSERT；退出码扩展；REPL/serve。

## Risks and Notes

- csv crate 需网络拉取（本地缓存无；crates.io 可达已验证）。若 Act 时网络不可用 → Blocker Handoff，不手写解析。
- clap `global = true` 的 `--format`：接受位置拓宽（`rtsql --format json list` 也合法），行为只增不减，不破坏既有场景。
- `new` 信号中断留半初始化文件（D4 风险注记）；restore/import 大文件全文读入内存（proposal Impact 注记）。
- dump 的 Float 精度往返依赖 f64 Display→f64 parse（Rust 保证最短往返表示）；JSON Number 含 u64 大整数时 `as_i64` 失败情形不可达（Value::Int 恒 i64）。
- `list` 的 `size_bytes` 对非 RTsql 内容文件照实列出（不校验 magic）——R-list 场景锁定该语义。
- 非实质未知项：子命令实现的文件组织（D10）、`emit_stdout` 分段写还是整缓冲（stdout 内容不变）、错误文案的微调（保留各 spec 场景锁定的关键词：`already exists`、`does not exist`、`database is locked`、序号模板）。
