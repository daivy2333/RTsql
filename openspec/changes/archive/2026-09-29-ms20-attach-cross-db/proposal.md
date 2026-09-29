# MS20 一期：ATTACH 式跨库交互（attach 注册 + 命名空间跨查 + CTAS） — Proposal

## Why

RTsql 定位为嵌入式数据库，但当前一次执行只能打开单个 `.db` 文件（I059，2026-09-24 用户点名方向，归位 MS20-T01/T02）：两个及以上库文件的关联检索、跨库抽取/合并/物化完全不可达——agent 与脚本消费形态下「把数据库视作表」是高频需求（SQLite ATTACH 同型为先例；PostgreSQL 单连接锁死单库为反例）。

现状缺口（MS20 既定，Phase 2 调查复核行号）：

- planner/executor 无任何 attach 注册面与多库命名空间概念——`object_name_to_table_name`（MS15-Rest，spec `table-name-resolution`）将 `a.b` 两段名归一化为单表名消费，无别名分派。
- `CREATE TABLE AS SELECT` 无 planner/executor 臂（跨库物化场景随之缺席）。
- 跨文件 DML 为二期（MS20-T02），本 change 不做。

## What Changes

用户已裁定（2026-09-29，四项决策）：**范围=仅一期 T01**；**接入形态=SQL 语句**（`ATTACH '<path>' AS <alias>` / `DETACH <alias>`）；**附加库锁语义=独占锁打开**（复用 FileStorage try_lock，冲突 → `DatabaseLocked` exit 4，跨 attach 死锁结构不可能）；**一期写入边界=CTAS 仅写主库**，对附加库的一切写入/DDL 计划期点名拒绝。

- **T01 ATTACH 会话语句与注册面**（`sql-attach-session`，新）：`ATTACH '<path>' AS <alias>` 在当前 CLI 会话注册附加库（会话作用域、不持久化、会话结束随进程退出释放）。路径解析复用 `resolve_db_path` 语义（裸名 → `$RTSQL_HOME/db/<name>.db`、含 `/` 直开）。重复别名、路径不存在、锁冲突（含 attach 主库自身——flock 按 inode 拒绝，同 `DatabaseLocked` exit 4 同型）均点名拒绝且零副作用。lib 非会话路径（`Database::execute*` 直调）对 ATTACH 显式拒绝（session-only，MS11-T02 事务语句同型先例）。
- **T02 跨库命名空间与只读跨查**（`cross-db-namespace-query`，新）：两段限定名 `alias.table` 在全部表名消费点解析——`alias` 命中 attach 注册表 → 路由到对应附加库表，未命中 → 回落既有扁平名语义（按 `"alias.table"` 查主库 catalog，含点表名零回归，找不到仍 `TableNotFound`；假设 6 用户裁定 2026-09-29）；裸名恒解析到主库（既有语义零回归）。SELECT 全形态（含 JOIN/子查询/聚合/WHERE/ORDER BY）可跨主库与多附加库组合。三段名：首段命中别名 → 计划期点名拒绝；首段未命中 → 扁平名语义（假设 6 用户裁定 2026-09-29）。
- **T03 一期只读边界与 CTAS**（`create-table-as-select`，新）：对附加库的 INSERT/UPDATE/DELETE/CREATE/DROP/ALTER 计划期点名拒绝（能力名可操作，MS19 错误面先例）。`CREATE TABLE t AS SELECT ...` 在主库物化——列名与类型自 SELECT 输出形状推导、目标表已存在点名拒绝零副作用、物化结果跨重启可读；SELECT 源可含附加库引用（「数据库视作表」抽取/合并主场景）。

## 默认假设（Gate 1 审阅项；可逐项推翻）

1. **会话作用域**：attach 注册仅存于当前 `rtsql` 调用进程内；one-shot 多语句（`;` 分片）同次调用内可见；不写 catalog、不产生伴生文件。
2. **plan cache**：含附加库引用的语句计划不进 plan cache（attach 状态不在 cache key 内，避免跨语句陈旧命中）；ATTACH/DETACH 语句本身不进 plan cache（同事务语句先例）。
3. **加密附加库**：附加库经与主库相同的密钥通道打开（全局 `--key`/`RTSQL_KEY`）；密钥与附加库不匹配时的打开失败行为以 Phase 2 对 `open_with_key` 实际行为的调查结论为准并在 design 定稿。
4. **隔离语义**：附加库以主库相同隔离级别打开；跨库查询不引入跨文件快照协调——每库各自 MVCC 视图，单语句内逐库一致（跨库一致性不在一期范围）。
5. **DETACH 取消（用户豁免，2026-09-29）**：原默认假设「一期包含 DETACH」经讨论后由用户裁定取消（原话：「同意你的判断，给出调用技能豁免」）。理由：sqlparser 0.44 无 DETACH 关键字/AST 变体且 Statement 枚举不可扩展，实现必须引入文本级预解析（破坏「无自研分片器」不变量）或语义挪用 `DEALLOCATE`/`DISCARD`（违背错误面诚实纪律）；而 one-shot 形态下注册面随进程退出释放，DETACH 无真实消费场景。**接受的风险/缺口**：one-shot 内无法中途释放附加库锁或换挂别名（理论场景、无消费方）。**重评触发条件**：REPL（I067）或 serve/PG 长会话形态（I071）落地时重评——届时 sqlparser 版本与自有元命令通道可能已改变结论。
6. **两段名未命中别名回落扁平名（用户裁定 2026-09-29）**：`foo.bar` 中 `foo` 命中注册表 → 附加库路由；未命中 → 按既有扁平名 `"foo.bar"` 查主库 catalog（既有含点表名语义零回归），找不到仍 `TableNotFound`（42P01）。三段名：首段命中别名 → 点名拒绝；未命中 → 扁平名语义。
7. **CTAS 隐式主键与失败语义（用户裁定 2026-09-29）**：CTAS 表沿用「首列隐式主键」既有 fallback；物化时首列 Int 重复值 → `DuplicateKey` 点名失败；失败时清理残留（回滚插入事务 + best-effort drop 已建表）保零副作用。
8. **自附加拒绝复用锁机制（用户裁定 2026-09-29）**：不引入 canonicalize 路径比较；attach 主库自身（含 symlink/hardlink 同 inode）由 flock 拒绝，报 `DatabaseLocked`（exit 4）。

9. **命名空间冲突**：附加别名不得与已注册别名重复；附加库的表与主库表同名不冲突（裸名恒主库，两段名首段命中别名才路由）；`alias.table` 中 alias 恰好等于主库某表名不影响。
10. **CTAS 形状**：列名取 SELECT 输出表头；类型自输出值推导的规则与边界（如全 NULL 列）在 Phase 2 调查后于 design 定稿；CTAS 不支持约束声明（PK/UNIQUE/NOT NULL/DEFAULT 均不带入），如需约束走既有 CREATE TABLE + INSERT 路径。
11. **ATTACH 在事务会话内**：CLI 会话存在活跃事务时执行 ATTACH 允许——attach 与事务正交（注册面不碰主库数据；附加库引用在活跃事务中的 DML 本已被只读边界拒绝）。

## 范围外（Non-goals）

- DETACH（用户豁免，见假设 5；REPL/serve 长会话形态落地时重评）。

- 跨文件 DML（二期 MS20-T02）：含跨文件 INSERT/UPDATE/DELETE、独立提交语义与边界文档化。
- 跨 attach 原子提交（master journal 级机制）；每文件独立 RC 快照之外的隔离精细化。
- 只读共享锁多读单写（MS26-T01 域）；`:memory:` 后端（MS26-T03 域）。
- crate 化「多文件会话」抽象（I074 排除域）。
- 视图（CREATE VIEW）、跨库视图物化的持久化形式（CTAS 为一次性物化，非活视图）。

## Impact

- **受影响代码面**（Phase 2 调查复核）：`src/cli/` 会话分派（`run_sql` 三臂扩展）、`src/database.rs` attach 注册面与多库协调、`src/parser/`（ATTACH/DETACH 语句识别与分类、`object_name_to_table_name` 两段名扩展、CTAS planner 臂、只读边界拒绝）、`src/pipeline.rs` 执行路由、`src/executor/`（CTAS 执行器或复用 create+insert 组合）。
- **受影响 specs**：新增 `sql-attach-session`、`cross-db-namespace-query`、`create-table-as-select` 三域；可能 MODIFIED `cli-noninteractive-shell`（会话语句分派面）与 `table-name-resolution`（两段名消费点扩展）——以 Phase 2 调查定稿。
- **测试**：新增跨库交互测试文件（attach 注册/拒绝矩阵、限定名解析、只读跨查各形态、CTAS 物化与重启、锁冲突）；全量零回归（当前基线 1408 tests）。

## 验证边界

- ATTACH/DETACH 拒绝矩阵（重复别名/自 attach/路径缺失/锁冲突/lib 非会话路径）。
- 限定名解析矩阵（两段名命中/未命中别名/三段拒绝/裸名回归）。
- 只读跨查 e2e（JOIN/子查询/聚合/ORDER BY 跨库组合）。
- 只读边界拒绝矩阵（附加库 DML/DDL 全点名）。
- CTAS e2e（物化形状/已存在拒绝/跨重启读回/源含附加库）。
- 全量 `cargo test -- --test-threads=1` 零回归。
