# proposal: MS10-T05 生命周期子命令（new/list/schema/dump/restore/import --csv）

## Why

MS10 主轨 T01-T04 落地了 one-shot 主命令、文件锁、优雅停机、格式头与多语句分片执行；`rtsql <db> <sql>` 全链路可用。但生命周期面仍是空白（R18 主题 7，tasks.md MS10-T05）：

1. **无显式创建/发现入口**：建库只能靠主命令静默创建（`test_new_database_created_silently` 契约）；agent 写 SQL 前没有 `schema` 发现步骤（R18：没有它 agent 无法写第一条 SQL）；集中存储区无法枚举（`list`）。
2. **无数据进出通道**：`dump/restore`（逻辑导出）与 `import --csv` 缺失——.db/.wal 文件对不可直接拷贝备份（checkpoint 后才等价，R18 主题 5 推论），CSV 数据无法入库。分析能力（MS11+）与 agent 工作流都以"schema 可发现 + 数据可导入"为前提。

**用户决策（2026-09-09，本会话 Gate 1）**：

1. **import --csv 目标表**：表必须已存在；CSV 首行表头按列名匹配（顺序无关）；空字段→NULL（非 String 列）/空串（String 列）；类型按表 schema 转换，失败 fail-fast。
2. **dump 产物形态**：SQL 文本（CREATE TABLE + INSERT 语句流，stdout，sqlite `.dump` 同款）；restore 直接执行该文本完成往返。
3. **目录创建**：仅 `new` 创建缺失父目录（裸名建 `$RTSQL_HOME/db/`、路径建父目录）；主命令与其余子命令维持「不建目录」契约不变。
4. **不存在库文件**：开库子命令（schema/dump/restore/import）对不存在的库文件报错 exit 1（`new` 是唯一显式创建入口）；主命令静默建库契约不变（既有测试锁定）。

**默认假设（本提案标注，Gate 1 一并批准）**：

- `schema` 输出 DDL 文本（每表一行 CREATE TABLE，含 PK/NOT NULL/UNIQUE 持久化约束），与 dump 共用 DDL 生成器；DEFAULT 约束不在 catalog 持久化面（catalog.rs `serialize_catalog_column_row` 无 default 字段），不出现在输出中。
- `list` 输出行集（列 `name`/`size_bytes`，按名称排序），复用 `render()`，支持 `--format` 与 TTY/非 TTY 默认；只枚举文件不开库（不校验内容有效性）。
- `new` 对已存在文件（含 0 字节）报错 exit 1；成功静默 exit 0。
- `restore` 要求目标库存在且无用户表（否则 exit 1）；支持 `-` 读 stdin（`dump | restore -` 管道）；成功静默 exit 0；执行期错误 exit 3 带语句序号（复用 MS10-T04 fail-fast 语义）。
- `import` 逐条 auto-commit（与 MS10-T04 已批准的逐条生效语义同构）；成功输出受影响行数（沿用 Affected 渲染语义）。
- CSV 解析引入 `csv` crate（RFC4180 引号/转义/换行边界由标准实现承载；手写解析是更大且更劣的"新增代码"）。
- 裸名与子命令名冲突（clap 实证：首参命中子命令名即分发）：子命令优先，此类库用含 `/` 路径形式打开；help 与文档注明。
- 退出码归类（不新增退出码）：Usage 2（子命令参数缺失/非法）/ General 1（文件系统与存在性错误、库不存在、目标表不存在、CSV 结构与类型转换错误）/ Sql 3（restore/import 的 SQL 执行失败）/ Locked 4（所有开库子命令）/ Signaled 128+n（信号停机语义与主命令一致）。

## What Changes

- **入口重构（`src/cli/mod.rs`）**：`CliArgs` 从扁平位置参数改为 `db: Option<String>` + `sql: Option<String>` + 全局 `--format` + `#[command(subcommand)] command: Option<Command>`（clap 探针实证：可选位置参数与子命令共存，`mydb "SQL"` 主命令形态零变化，`list`/`new foo`/`schema mydb` 正确分发）。主命令缺参由手动 usage 处理承接（exit 2，既有 `test_usage_error_exit_2` 仅断言退出码）。
- **resolve 扩展（`src/cli/resolve.rs`）**：提取 `db_dir()`/`rtsql_home()` helper（`list` 与 `new` 复用；`resolve_db_path` 语义不变）。
- **新增子命令（`src/cli/`）**：
  - `new <name|path>`：存在性检查 → mkdir 父目录 → open（建库）→ close（checkpoint）；复用两阶段信号编排（work 为空闭包）。
  - `list`：枚举集中区 `*.db`（名称、大小），不开库。
  - `schema <db>`：经 `catalog().scan_tables()/scan_columns()` 生成 DDL 文本输出（系统表不可 SQL 查询，必须走内部 API）。
  - `dump <db>`：DDL 生成器 + 逐表 `SELECT *` → INSERT 文本流（字面量转义：单引号加倍；NULL→NULL；Bool→TRUE/FALSE）。
  - `restore <db> <file|->`：读入 SQL 文本 → 复用逐条执行循环（静默）+ 空库前置检查。
  - `import <db> <table> <file> --csv`：csv crate 解析 → 表头按名匹配 → schema 驱动类型转换 → 逐条 INSERT。
- **spec 修改（`cli-noninteractive-shell`）**：R1 文字扩展子命令分发与裸名冲突语义；新增 5 个 Requirement（new/list/schema/dump-restore/import）。
- **测试**：`tests/cli_test.rs` 追加子命令集成测试组（沿用真二进制 + TempDir 模式）；`resolve.rs` 单测扩展目录 helper；lib 单测覆盖 DDL 生成器、SQL 字面量转义、CSV 类型转换纯函数。

## Out of Scope（本 change 不做）

> **修订（2026-09-09，Iteration 000 Review 后）**：原「planner/pipeline/storage 层任何修改」经实施审计收窄——NOT NULL/UNIQUE 在建库链被丢弃（catalog 写入硬编码 false）导致 R-schema S1 不可满足，001-rework 契约将 storage 触碰面收窄放行至**建库约束通道**（`src/executor/create_table.rs` 约束透传 + `src/storage/data/table_manager.rs` 新增 `create_table_with_constraints`，既有签名零变化）；「storage 层其余路径 / planner / pipeline 层零修改」与「lib API 签名变化」仍然成立。约束运行时强制（INSERT/恢复校验）仍不做。
> **修订二（2026-09-09，Iteration 001 Review 后，用户裁定方向 A「无键值落库不入索引」）**：实施触碰面收窄放行至**无键行语义通道**——引擎对键位（PK 列，未声明时为第一列）值不可键控（NULL/非 Int）的行原为整行静默丢弃（`src/executor/insert.rs:96-99`），阻塞 R-import S3/S4 且构成静默数据丢失；修复放行 `src/executor/insert.rs`（无键落库不入索引）+ `src/wal/recovery.rs`（Update 重放无键回退，否则无键行 UPDATE 后崩溃将 RedoFailed 致库不可打开）+ `wal-recovery-replay-integrity` spec MODIFIED delta。可键控行路径、WAL 磁盘格式、lib API 签名、planner/pipeline/CLI 层限制仍然成立。键控面扩展（String/Float 键控）、DDL 级 PK 类型拒绝、隐式主键重设计、GC 无键链覆盖均不做（improvement 候选）。

- SQL 级事务语句、表达式四件套、标量函数（MS11）；`key` 子命令与加密（MS12）；REPL；`serve`；stats/sample/profile 薄命令（MS13）。
- dump 的非 255 String 长度保真（CLI 侧建库恒为 255，仅 lib 直建可出现，R20 F10 边界记录）；NaN/Inf Float 值的 dump 语义（跟随 `value_to_json` 现状）。
- planner/pipeline/storage 层任何修改（INSERT 不引入列清单形式，import 在 CLI 侧重排值序规避）；lib API 签名变化。
- 退出码枚举扩展（5 密钥留位不变）。

## Impact

- **修改**：`src/cli/mod.rs`（入口重构 + 子命令分发与实现）、`src/cli/resolve.rs`（目录 helper）、`Cargo.toml`（+csv）、`openspec/specs/cli-noninteractive-shell/spec.md`。
- **新增**：`src/cli/` 子命令实现（预计 mod.rs 内模块化组织，是否拆文件由 Act 按非实质选择处理）；`tests/cli_test.rs` 子命令测试组；DDL 生成/转义/CSV 转换的 lib 单测。
- **行为变化**：① `rtsql list/new/schema/dump/restore/import` 从 usage 错误 → 可用子命令；② 主命令对合法输入行为零变化（含静默建库）；③ 裸名命中子命令名的主命令调用从"缺 SQL 参数" → 子命令分发（文档化边界）。
- **兼容性**：`resolve_db_path`、`render`、`ExitStatus`、pipeline stage 函数、lib 公开 API 全部不变；既有 671 测试基线零回归（cli_test 25 用例零修改为约束）。
- **风险**：clap 可选位置参数丢失自动"缺参"报错，需手动 usage 处理对齐 exit 2；`--format` 转 global 属性后参数位置宽容度变化（行为只增不减）；restore/import 的大文件内存占用（全文读入，本 change 不做流式——数据量级为嵌入式单机场景）。
