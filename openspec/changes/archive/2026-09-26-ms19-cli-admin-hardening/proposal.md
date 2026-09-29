# MS19 CLI 管理面小收口（删库子命令 + 错误可操作化 + 会话边界测试加固） — Proposal

## Why

CLI 管理面存在三类已定位的缺口（MS18-MS27 路线统一执行序中紧随 MS23/MS24 之后的小项批）：

- **删库无一行式命令（I061，2026-09-24 用户方向）**：生命周期子命令只有 `new/list/schema/dump/restore/import`，SQL 层 `DROP TABLE` 只删表——删库只能文件级 `rm <name>.db <name>.wal <name>.checkpoint` 三件套。手工 rm 需要用户了解伴生文件布局，漏删伴生文件会残留脏现场；agent/脚本管理场景缺一行式命令，`list` 可枚举但不可回收。
- **错误信息不可操作（I070，R18 主题 3）**：`PlanError::UnsupportedStatement`/`UnsupportedExpression` 为无载荷变体（`src/parser/error.rs:13,17`），报错「Unsupported statement type」不点名不支持什么，排错需读源码或试错；PG 协议错误响应统一 58000（system error），协议面错误不可按类消费。
- **会话边界组合路径无回归锁定（I042，MS11-T02 Review finding）**：spec `sql-transaction-statements` R2 各拒绝场景均为独立调用、空闲会话；「会话事务活跃中遇边界子句」（如 `BEGIN; INSERT ...; SAVEPOINT sp1`）的组合路径行为正确（exit 3 + 点名文案 + 事务上下文后缀 + 重开无残留，Plan Review 二进制探针验证）但无 spec 场景与 e2e 用例——未来重构 `run_sql` 错误路径时该组合语义可能静默回归。
- **文件对备份边界无文档（I072，R18 主题 5 推论）**：文件对（`.db` + `.wal`）是移动/备份的最小单位——只拷主文件在 checkpoint 后等价、否则丢尾部事务；移动/重命名/拷贝的配对行为未实测，用户文档也未声明「备份首选 dump/restore 逻辑导出」。用户以文件拷贝做备份可能静默丢尾部事务且无提示。

## What Changes

- **`rtsql delete <db>` 删库子命令（I061）**：复用 `resolve_existing_db` 定位（裸名 → `$RTSQL_HOME/db/<name>.db`；含 `/` 路径直开），删除主文件与 `.wal`/`.checkpoint` 伴生文件（`db_path.with_extension` 命名，`src/wal/writer.rs:27`、`src/wal/checkpoint.rs:70`），逐文件报告释放结果。交互语义（用户裁定）：**无交互确认、直接删除**，保持 one-shot 一行式定位；提供 `--dry-run` 列出将删除的文件与大小、不执行任何删除。库打开中（advisory 独占锁被持有）SHALL 显式拒绝（exit 4 既有映射）且零删除；探测与删除在同一锁持有窗口内完成，消除检查-删除竞态。加密库为文件级操作无需密钥。delete 是文件级管理操作，SHALL NOT 做库内容校验（头部无效/零字节的文件同样可删）。
- **PlanError 特性名携带 + 错误分类（I070，用户裁定 PG 错误码分类明确纳入实施）**：`UnsupportedStatement`/`UnsupportedExpression` 携带能力名（语句种类/表达式种类），CLI 错误信息直接点名不支持的能力；PG 协议错误响应按错误类别区分错误码——能力面拒绝（不支持语句/表达式/约束/JOIN 类型/子查询位置等）映射 `0A000` feature_not_supported，解析错误 `42601`，表不存在 `42P01`，列不存在/歧义 `42703`/`42702`，其余未分类错误维持 `58000` 兜底；完整映射表随 design 定稿。CLI 既有退出码 0/1/2/3/4/5 与错误文案框架不变（追加能力名，既有测试断言按预期校准）。
- **会话边界组合场景 e2e（I042）**：`tests/tx_statement_test.rs` 增加组合场景——会话事务活跃中遇边界子句（`BEGIN; INSERT ...; SAVEPOINT sp1` 等形态）→ exit 3 + 点名文案 + 事务上下文后缀 + 重开无残留；纯测试加固，不改产品代码。
- **文件对备份边界实测与文档化（I072，随 T01 顺带）**：实测 `.db`+`.wal` 文件对在移动/拷贝/重命名下的配对行为矩阵（checkpoint 后等价、否则丢尾部事务）；README 双语声明备份边界——备份首选 dump/restore 逻辑导出，文件级拷贝的等价条件与风险明示。
- **列解析静默面收口（Iteration 002，2026-09-26 用户批准扩围）**：来源为 Iteration 001 Act Response Issue 候选 ①② 与 Plan 真二进制探针测绘——① 纯列路径未知投影列（裸名/限定名/混合已知+未知）静默回落 identity 投影返回全列全数据（`resolve_projection_indices` None→identity，`src/parser/planner/query.rs:1597`）；①-连带 纯非聚合查询 `col AS alias` 别名表头静默退化（别名被当基列名解析，表头回落全 schema）；② WHERE/值表达式路径列缺失被建模为 `ParseError`（42601）而非 `ColumnNotFound`（42703）（`src/parser/planner/expression.rs:283/340`，单表路径与多表路径建模不一致）；C `ORDER BY` 未知排序键静默不排序（计划期不校验，`src/executor/sort.rs:80` position None 跳过）。以上形态全部改为计划期显式点名拒绝（exit 3 / `ColumnNotFound` / PG 42703），别名形态改为正确生效。**无聚合 GROUP BY 整体被忽略（有效键与未知键均静默）不在本变更范围**——属未实现语义缺口需单独裁定（dedup vs 拒绝），作为 Issue 候选报告。

## 范围与决策记录（用户批准）

- MS19-T01/T02/T03 全部任务并入单一 change，两个 Iteration 分域验收（用户裁定 2026-09-26）：Iteration 000 = delete 子命令 + I072 备份边界；Iteration 001 = 错误面 + 会话边界 e2e。
- delete 交互语义：直接删除 + `--dry-run`（用户裁定 2026-09-26）；无 TTY 确认、无 `--force` 门槛。
- PG 协议错误码分类明确纳入实施（用户裁定 2026-09-26），不再仅评估。
- 命令命名沿用已批准路线：`rtsql delete <db>`。
- 与 `install.sh --purge-data` 的关系：purge-data 清空整个数据目录属卸载场景，delete 仅单库——边界在 README 文档化，无代码交互。
- **Iteration 002 扩围（用户裁定 2026-09-26）**：Issue 候选 ①② 连同 Plan 探针发现的同族静默面（别名表头退化、ORDER BY 未知键）以「全收口」范围并入本变更第三轮 Iteration；无聚合 GROUP BY 忽略（语义缺口）不并入，报 Issue 候选。范围选项（全收口 vs 最小）经用户选择确认。

## Capabilities

### New Capabilities

- `error-reporting`: 定义错误可操作化语义——`UnsupportedStatement`/`UnsupportedExpression` 的能力名携带与 CLI 点名文案、PG 协议错误码按类别分类映射、既有退出码与错误语义零回归。

### Modified Capabilities

- `cli-noninteractive-shell`: 新增「生命周期子命令 delete」Requirement——定位、伴生文件清理、释放报告、`--dry-run`、锁占用拒绝、缺失拒绝、路径形态与无密钥要求（变更内携带该 capability 的 ADDED delta）。
- `sql-transaction-statements`: R2「边界子句显式拒绝」补充活跃会话组合场景——会话事务开启后遇边界子句的拒绝语义、收尾回滚与重开无残留（变更内携带该 capability 的 MODIFIED delta；行为本身已存在，本变更以 spec 场景与 e2e 锁定）。

说明：备份边界文档化不设行为 spec——交付物是 README 双语声明 + 实测矩阵记录（Act Response/Evidence），不属于引擎行为面。

## Impact

- 代码：
  - `src/cli/lifecycle.rs` — delete 子命令编排（锁探测、文件删除、释放报告、dry-run）
  - `src/cli/resolve.rs` — `resolve_existing_db` 复用与缺失错误形态（随调查确认）
  - clap 子命令声明处（`src/cli/mod.rs` 或 `src/main.rs`，随调查确认）— `delete` 注册
  - 锁探测原语复用点（`src/storage/file_storage.rs` FileExt try_lock，形态随调查定稿）
  - `src/parser/error.rs` — PlanError 两变体携带能力名 + 错误分类载体
  - planner 构造点（`src/parser/planner/` 各模块）— Unsupported* 构造点携带能力名（Phase 2 清点）
  - `src/network/pg_protocol.rs` — 错误码分类映射（现状统一 58000 处）
  - `README.md` / `README.zh-CN.md` — delete 子命令说明 + 备份边界段
- 测试：
  - `tests/cli_test.rs` — delete 子命令矩阵（happy/dry-run/缺失/锁占用/伴生缺失/路径形态/释放报告）
  - `tests/planner_test.rs` / `tests/cli_test.rs` — 错误面能力名断言与既有文案校准
  - `tests/tx_statement_test.rs` — 组合场景 e2e（I042 锁定）
  - I072 实测矩阵 — 验证记录（Act Response；若矩阵结论依赖一次性环境则按 Evidence 白名单评估）
- 已知边界：
  - 裸名 `delete` 的数据库与子命令名冲突时子命令优先（既有文档化语义）——该库经主命令形态不可达，经路径形态可达。
  - delete 不做库内容校验（文件级管理操作，损坏/空文件同样可删）。
  - 文件级拷贝备份在未 checkpoint 时丢尾部事务——README 明示，行为本身不变（I072 为文档化而非行为变更）。
