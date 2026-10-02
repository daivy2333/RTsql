# 跨文件 DML 二期

## Why

路线 `.agents/docs/tasks.md` 的 MS20-T02 尚未实施。现有 ATTACH 会话支持跨库 SELECT 和主库 CTAS，但附加库 INSERT/UPDATE/DELETE 被计划期拒绝。本变更完成附加库写入及每文件独立提交的行为边界。

## What Changes

用户于 2026-10-02 批准三项推荐选择，原话：「批准，记得遵守各类规范」；同日审计完整计划后再次回复「批准」。本文件为获批需求基线，首轮执行契约已获批。

- R1：普通可写 CLI 会话通过已注册别名向附加库执行既有 INSERT VALUES（含子集列、DEFAULT、UPSERT/REPLACE）、UPDATE、DELETE。沿用各语句已有单库能力与拒绝边界。
- R2：每条 DML 在目标文件独立提交。后续语句失败不撤销此前已提交的其他文件数据；当前失败语句沿用单库回滚语义。不承诺跨文件原子性或协调快照。
- R3：本期在显式 BEGIN 会话内拒绝附加库写入；主库显式事务和跨库读取保持现有语义。拒绝经现有 CLI fail-fast/会话回滚路径处理。
- R4：只读会话继续拒绝全部写入；附加库 DDL 与 CTAS 附加目标继续拒绝；未命中别名的扁平名回落、裸名指向主库、别名归一化及不缓存附加库语句保持。
- R5：写入后的关闭/重开与崩溃恢复结果保持目标库单库保证。锁等待沿用已实现预算，超时 exit 4；信号取消沿用现有优雅停机，不引入跨文件补偿。
- R6：文档明确独立提交、显式事务限制与支持语句面。
- R7：支持 INSERT SELECT，目标为主库或附加库既有表，源可读取主库和其他附加库；沿用现有 SELECT 能力和目标列约束。
- R8：UPDATE/DELETE 已支持的子查询形态允许跨库读取；不扩展原本拒绝的语法。调查确认当前两种 DML 均无子查询执行路径，保持主键等值及字面量赋值限制，并以拒绝测试锁定该边界；这不裁剪已批准的「既有形态」范围。
- R9（2026-10-02 用户指令追加，Iteration 002）：两段附加 FROM 项（`alias.table`）在无显式别名时，其尾段 SHALL 作为该项的隐式关联名注册，使 `SELECT u.tag FROM t JOIN o.u …` 类短名限定列引用计划期可达（标准 SQL 隐式别名语义，SQLite 同型）；显式别名优先，纯主库表与字面含点表名行为不变。
- R10（2026-10-02 用户指令追加，Iteration 002）：崩溃恢复 redo 的落位 SHALL 以 WAL 记录携带的 logical_id 为准收敛——运行期「语句部分写入回滚 → 后续写入 → 崩溃」序列（页内逻辑 id 已被中止事务消耗）SHALL NOT 因重放分配序列与运行期失配而 RedoFailed 拒绝打开；无法物理落位（页满/IO）仍显式报错。

## 场景草图与待决策

| 场景 | 前置状态 | 动作 | 可观察结果及失败边界 |
|---|---|---|---|
| S1 附加库增删改 | 主库与附加库各有同名表 | 对 alias.table INSERT/UPDATE/DELETE | 只改变附加库；主库不变；重开后结果一致 |
| S2 写面组合 | 附加库有 DEFAULT/NOT NULL/UNIQUE/索引 | 子集 INSERT、UPSERT/REPLACE、rekey UPDATE、DELETE | 同单库语义；约束失败不遗留当前语句修改 |
| S3 多文件先成功后失败 | 主库与两个附加库可写 | 依次成功写两个文件，下一条约束失败 | 前两条提交保留，第三条无新增副作用，CLI fail-fast |
| S4 显式事务边界 | BEGIN 后主库已有未提交修改 | 尝试写附加库 | 点名拒绝；附加库不变；主库由既有错误收口回滚 |
| S5 只读与 DDL | 只读会话或附加目标 DDL | 写附加库/CREATE/DROP/ALTER/CTAS 附加目标 | 既有拒绝面与零副作用保持 |
| S6 名字与空结果 | 同名表、未知别名/缺表、空表 | 路由写入或 UPDATE/DELETE 未命中 | 路由准确；缺表点名；DELETE 未命中 affected=0，UPDATE 未命中沿既有 KeyNotFound；扁平名回落不变 |
| S7 超时与取消 | 附加文件被锁或执行阶段收到信号 | ATTACH 等待/写入中取消 | 预算耗尽 exit 4；130/143 停机；已提交数据保留，未提交按单库保证处理 |
| S8 多库数据作为写入输入 | 主库/附加库提供源行 | INSERT SELECT | 按目标列映射一次写入目标库；源不改变；失败回滚当前目标语句 |
| S9 UPDATE/DELETE 语法边界 | 目标存在 | 子查询赋值或主键子查询条件 | 继续拒绝当前不支持形态，不静默忽略输入 |
| S10 SELECT 列结构 | 有固定 schema 的目标，源可为空 | 重排列子集/错误宽度/未知或重复列 | 合法映射用 DEFAULT/NULL 补位；非法结构在源求值前拒绝，空源也验证 |
| S11 SELECT 源失败及写入中途失败 | 目标有存量与唯一约束 | 源求值错误，或后续源行违反类型/NOT NULL/唯一约束 | 源失败零写入；隐式事务写入失败还原本语句全部目标行与索引 |
| S12 同表 SELECT 与空源 | 目标也是源或源无行 | 物化后写回 | 只读取写入前源行，有限结束；空源 affected_rows=0 |
| S13 SELECT 冲突与主库事务 | 有冲突行或主库 BEGIN | SELECT ON CONFLICT/REPLACE；主库目标 COMMIT/ROLLBACK | 复用既有仲裁计数；主库目标复用事务，附加源不改变 |
| S14 checkpoint/close 错误 | 已有成功提交，随后存储关闭失败 | 返回退出状态并重开 | 已提交保持；成功→exit1，已有错误/信号保留且关闭错误点名，所有已注册库均尝试关闭 |
| S15 恢复两态与组合 | 加密/明文附加或内存主库有附加目标 | 目标写入后干净重开或 WAL 恢复 | 数据、PK、唯一/二级索引一致；模式/key/budget 既有边界保持 |
| S16 附加表短名限定引用 | 附加库有表 u，主库有表 t | `SELECT u.tag FROM t JOIN o.u ON t.id = u.id`（及单表/WHERE/GROUP BY/ORDER BY 面） | 短名限定列解析到附加 FROM 项，计划期可达；显式别名与全限定形态零变化；主库字面含点表名不误登记 |
| S17 中止后重插的崩溃恢复 | 语句部分写入（前行已落槽位消耗逻辑 id）后回滚，随后重插并崩溃（WAL 未 checkpoint） | 重开执行恢复 | 恢复成功无 RedoFailed；行/PK/唯一/二级索引与运行期提交终态一致；无 abort 与 abort 后不重插对照序列行为不变；恢复重跑幂等 |

## 已批准的集中决策

1. 批准 R1–R6 及显式事务中拒绝附加库写入。
2. 包含 INSERT SELECT，源可来自主库或其他附加库。
3. 允许已经支持的 UPDATE/DELETE 子查询形态跨库读取，保持现有语法边界。
4. 2026-10-02 用户指令追加 Iteration 002 收口两个实施期发现的 Issue 候选（原话：「额外给出一个iter，我们顺便把issue解决了」；范围选择「两个都修」）——R9 附加 FROM 项隐式短名限定引用可达（Iteration 001 Issue 候选）、R10 恢复重放按 WAL logical_id 收敛（Iteration 000 Issue 候选：abort 后槽位/逻辑 id 消耗 × 崩溃恢复 redo 失败）。

## Non-goals

跨文件原子提交、跨文件协调快照、附加库 DDL、附加目标 CTAS、DETACH、REPL/serve、多文件会话公共 API 重构、存储格式变化、性能优化。

## 工作状态与边界

- Phase 1 CLARIFY：completed；场景扫描、需求基线、集中决策完成。
- Gate 1：PASS；2026-10-02 用户批准上述三项。
- Phase 2 INVESTIGATE AND PLAN：completed；调查、设计、任务分轮、RTM 与当前执行契约已建立。
- Gate 2：PASS；2026-10-02 用户批准完整计划，原话：「批准」。当前 Context 为 ready；尚无 Act 实施授权。
- Persisted Evidence：none；不创建 Evidence。
- SKIPPED：产品代码修改、全局状态同步、实施、归档，均不属于本次 Plan 授权。

## Impact 与回退

关联路线 MS20-T02 与用户方向 I059（其已批准范围以 tasks 与一期 proposal 为准）；当前约束复用 M13/M17/M18/M19。长期选择保存在本变更 design D1–D5，不恢复退役 D/K 全局文档；openspec/config.yaml 中旧 D/K 路由与当前 AGENTS 不一致时按公共规则执行。

影响面：CLI AttachSession/退出编排；pipeline 目标事务与执行器构造、INSERT 源表注册/物化；planner DML 与 SELECT 源构建；新增计划节点的消费面；跨库/只读及新写面测试；README 双语与仓库 rtsql-docs；Iteration 002 追加 planner FROM/JOIN 别名注册点与 WAL 恢复 redo 落位（`src/parser/planner/query.rs`、`src/storage/page_format/slotted_page.rs`、`src/wal/recovery.rs`）。增量规格覆盖 cross-db-dml（新增）、cross-db-namespace-query、readonly-open-mode、sql-write-surface、sql-table-alias-resolution（修改）、wal-recovery-replay-integrity（修改）。

无磁盘格式和依赖迁移；回退代码后新增 SQL 再次被拒绝，已提交数据仍使用既有格式。跨文件此前已提交语句不因后续失败或二进制回退撤销。Iteration 002 的恢复收敛不改变运行期页分配行为与磁盘格式（新 SlottedPage 原语仅恢复面消费）；回退二进制后 R10 场景回到既有 RedoFailed 形态。

当前上下文来源：AGENTS.md、2026-10-01 tasks/SNAPSHOT、project-model（含 M19 分批全量测试约束），以及已归档一期 proposal 和现行规格；本期实际代码调查与 133 项定向基线已完成，详见 design 与当前 Cycle。
