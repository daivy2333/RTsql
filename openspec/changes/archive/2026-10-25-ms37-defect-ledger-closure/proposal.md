# Proposal: MS37 缺陷台账收口（ISS25–ISS34）

## Why

`.agents/issues/` 台账中有 **10 条 `open` 缺陷**（ISS25–ISS34），它们分属三个故障域，且**全部处在「已验收合同与实际行为之间有明确落差」的位置**：

- **存储读面（ISS26 / ISS27）**：索引树叶页的条目长度不在页级判据内（短条目被静默跳过），其余页型（数据页 / 目录页 / 恢复面 / 缓冲池 / executor 与事务调用点）仍有约 21 处无界槽读点 ⇒ 损坏页可触发越界切片 panic，或退化为「查不到该行」的静默错读。
- **DDL 提交点（ISS25 / ISS28 / ISS29）**：`ALTER TABLE … DROP COLUMN` 的「数据面改写已提交、目录窄化未发布」窗口在崩溃-重开后按旧列序错位取值；失败的 DDL 语句其目录面变更会在后续刷盘时落地；同表并发 DDL 的在途状态是一个布尔量 ⇒ 重叠清除与豁免读按旧 schema 解码。
- **写盘失败状态（ISS30 / ISS31）**：并发驱逐与写盘失败交错时，被驱逐页的新内容既未落盘也不在池内；`last_failed_write` 是库级一次性、永不复位的标志 ⇒ 一次写盘失败之后该会话内的释放提升永久停滞。

另有两条**合同面**缺口（ISS32 多表 dump 部分恢复的边界未成文；ISS33 `dump` 对非整数 `UNIQUE` 列无条件渲染 ⇒ 该类表 dump 不可 restore）与一条**语料面**缺口（ISS34：`index-read-bounded-read` 域的 Requirement 标题与 4 个 Scenario 标题措辞残余）。

MS37 的 Outcome 要求这 10 条缺陷**全部处置并关闭**（零 `open` 残留）。

## What Changes

**一个 change，按故障域分 Iteration**（理由见 `design.md` D1）。六个 Iteration：

| Iteration | 标题 | 承载缺陷 | 落点 |
|---|---|---|---|
| 000 | 索引树叶条目长度纳入页级判据 | ISS26 | `src/storage/integrity.rs` 的 `check_tree_page` 叶页分支 |
| 001 | 其余页型的有界读（按页型分组） | ISS27 | 数据页 / 目录页 / 恢复面 / 缓冲池 / executor 与事务调用点 |
| 002 | DDL 提交点统一模型 | ISS25 + ISS28 + ISS29 | `src/storage/data/table_manager.rs`、`src/storage/catalog.rs`、WAL 与恢复面 |
| 003 | 写盘失败的状态面 | ISS30 + ISS31 | `src/storage/buffer_pool.rs`、`src/storage/file_storage.rs` |
| 004 | dump / restore 的合同面 | ISS32 + ISS33 | `src/cli/lifecycle.rs`、dump 渲染规则与 restore 边界 |
| 005 | 语料收口与收尾全量回归 | ISS34 + I045 + I064 | 规格语料、README，以及本 change **唯一一次** M19 分批全量回归 |

**（结构修正，2026-10-25）：上表是本 change **最初**的六 Iteration 切分，已被 `design.md` D14（第一次结构修正）与 D15（第二次结构修正，写路径组插入）取代 —— 当前共 **10 个 Iteration（000–009）**：000 索引树叶条目长度（ISS26）｜001 数据页组（T3）｜002 页头槽数越界的读路径 abort（T17 / ISS35）｜**003 写路径的槽界限（T18 / T19 · ISS37 + ISS38）**｜004 目录页组（T4）｜005 恢复面组（T5）｜006 DDL 提交点统一模型（T6–T8）｜007 写盘失败状态面（T9 / T10）｜008 dump / restore 合同面（T11 / T12）｜009 语料收口与收尾全量回归（T13–T16）。权威的 Iteration Map 见本 change 的 `tasks.md`。）**

**四项方向性裁定**（由本 change 的 goal 授权裁定，理由与兼容面分析见 `design.md`）：

1. **DDL 提交点统一模型**（ISS25 / ISS28 / ISS29 共享）：窄化意图作为**事务作用域的 WAL 记录**与数据面改写**同一次持久化**落地；提交点即事务提交；提交前失败走**目录面补偿**；同表布局 DDL 由**带所有者令牌的每表 CAS** 串行化。**磁盘格式不变** ⇒ MS36 的 G1/G2/G3 兼容面不受影响。
2. **ISS28**：失败的 DDL 语句的目录面变更必须**补偿**（恢复为语句执行前的目录内容并保持脏标记）⇒ 既有域 `ddl-catalog-transactional` R1 的字面**保持**，无需修订。
3. **ISS29**：在途状态由「单表一个布尔量」改为**带所有者令牌的每表布局 DDL 令牌**；同表并发布局 DDL **显式拒绝**（点名该表）。
4. **ISS32**：**维持既有合同后关闭**——原子 restore / 全量预检属功能语义扩展（MS37 Non-goals 明确排除），当前行为已是显式拒绝（点名 + 退出码 3 + 「此前语句已提交」提示）⇒ 把该边界**写成规范条款**并以真实入口回归用例锁定。

## Impact

- **受影响的行为域**：新增 `page-read-bounded-read`、`ddl-layout-publication`、`write-failure-state` 三个域；修改 `index-read-bounded-read`、`release-path-bounded-read`、`cli-noninteractive-shell`、`sql-constraint-enforcement`、`legacy-format-compat` 五个域。
- **兼容面**：`legacy-format-compat` 的 **G1 / G2 / G3 支持范围不变**（磁盘格式不变）；`ddl-catalog-transactional` R1/R2/R3 **保持**；`mvcc-tombstone-visibility`、`drop-persistence-boundary`、`release-durability-precondition`、`index-write-side-atomicity`、`index-page-ownership`、`ddl-rewrite-consistency`、`free-page-lifecycle` **保持**。**唯一被修订的既有合同**是 `sql-constraint-enforcement` 的「存量非 INT UNIQUE 库的边界已知」Scenario（ISS33 使 `dump` 不再输出该 `UNIQUE`，故该边界由「restore 被拒绝面挡住」转为「产物不含该约束、可往返」）。
- **测试**：新增损坏注入夹具（叶条目长度）与既有夹具形态（按文件偏移改写页字节）复用；全量回归按 **M19 分批串行**纪律在 Iteration 009（原编号 008；结构修正见 `design.md` D14 / D15）执行**一次**。
- **不做**：磁盘格式变更、新增用户可见子命令、原子 restore、非整数列的 UNIQUE 计划期支持放宽、索引名注入 `IndexManager`、任何身份证明机制（hash / 指纹 / revision pin / run-id / manifest / 时间顺序证明）。
