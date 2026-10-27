# Design: MS37 缺陷台账收口（ISS25–ISS34）

本文件记录**长期选择及其理由**，随 change 归档。所有裁定依据本 change 的 goal 授权（用户已批准方向性裁定），但**每条裁定仍逐条核对了既有已验收合同**，并在下文给出「保持 / 修订 / 例外」的结论。

---

## D1. change 形态：单一 change，按故障域分 Iteration

**决定**：MS37 用**一个** change 承载 10 条缺陷，按**故障域**切分为 9 个 Iteration（000–008；2026-10-25 结构修正后由原 6 个拆出，详见 D14；**2026-10-25 二次结构修正后为 10 个 Iteration（000–009），详见 D15**）。

**理由**：

1. **裁定可统一**：ISS25 / ISS28 / ISS29 共享同一个「DDL 提交点」域，ISS30 / ISS31 共享同一个「写盘失败状态」域；拆成多个 change 会让这三处裁定分散在多个 change 的 `design.md` 中，模型容易走形。
2. **收尾开销小**：10 条缺陷的合并、Issue 台账的集中关闭、以及**唯一一次**全量回归预算，都只需要一个 change 的收尾动作。
3. **诊断边界清晰**：Iteration 的切分依据是**故障域 + 验证面**，不是缺陷条数——ISS26 与 ISS27 虽同属存储读面，但**根因位置与承载面不同**（前者是页级判据的覆盖面，后者是无界读点的分布面），且 ISS27 的各页型需要**各自**判定「损坏页在该页型的正确行为」，故分列不同 Iteration（ISS26 = Iteration 000；ISS27 = Iteration 001 / 003 / 004，见 D14；**二次结构修正后为 Iteration 001 / 004 / 005，见 D15**）。
4. **拆分信号**（触发拆分的条件，记录在 `tasks.md`）：若某一 Iteration 内出现**两个互不相干的页型组**需要各自的判据与夹具 ⇒ 该 Iteration 内部再按页型组切分为**连续 Iteration**（每个页型组各自一个 Iteration，各带唯一的 `000-initial.md`）；若某一缺陷的修复需要**新增磁盘格式**或**新增用户可见子命令** ⇒ 该缺陷移出本 change，另立 change。

**已排除的替代形态**：按缺陷逐条建 change（10 个 change，收尾与全量回归开销失控，裁定不统一）；按域拆成 5 个 change（DDL 域的裁定与写盘失败域的裁定会分处不同 change，MS37 的「零 open 残留」目标仍可达但收尾面放大）。

---

## D2. 【方向裁定一】DDL 提交点统一模型（ISS25 / ISS28 / ISS29）

### 2.1 事实基础（Plan 已逐行读码确认）

`ALTER TABLE … DROP COLUMN` 的现行时序（`src/storage/data/table_manager.rs` 的 `alter_drop_column` / `apply_column_drop`）：

1. 事务内改写数据面（逐行 DELETE + INSERT 为窄行）+ 重建索引；
2. **事务提交** ⇒ 窄行已持久（WAL）；
3. `apply_column_drop`：`rewrite_column_rows_drop`（改写 `__columns` 链）→ `update_table_column_count_ddl`（下调 `__tables` 行的 `column_count`）⇒ **`__tables` 行的写入是布局的发布点**（列数是活布局的唯一来源；`open_or_init` 按列数截断列行）。

⇒ **崩溃窗口在第 2 步与第 3 步之间**：窄行已持久而目录仍宣告 3 列 ⇒ 重开后按旧列序错位取值（ISS25 的崩溃-重开变体）。

### 2.2 为什么「持久化标记」与「移动提交点」两条候选都不成立

- **(a) 目录面持久化标记**：`__tables` 行需要新增「窄化在途」的可选尾段 ⇒ 改磁盘格式；且**标记本身仍不足以判定该窄化是否已提交**——崩溃后目录里只剩「有标记」，而「数据面窄化是否已提交」这一事实只存在于 WAL ⇒ 标记必须与提交状态**原子**绑定，单独存在时无法区分「已提交未发布」与「未提交」两种状态。
- **(b) 移动提交点**（把目录窄化移到数据面改写之前）：① 目录先行窄化而数据面未改写 ⇒ 立刻产生**镜像方向**的错位（3 列目录 + 3 值行 ⇒ 按 2 列读时若被删列非末列则取值错位）；② 目录面变更发生在事务内 ⇒ 语句失败/放弃后目录已被窄化 ⇒ 与既有域 `ddl-catalog-transactional` R1「被放弃或失败的 DDL 语句不留下目录面变更」**字面冲突**；③ 目录面写入不经 WAL ⇒ 事务回滚无法撤销已刷盘的目录变更。

### 2.3 决定：窄化意图随事务持久化 + 提交点即事务提交 + 失败补偿 + 每表令牌

**M1（意图与提交原子绑定）**：DROP COLUMN 的窄化意图（被删列的序号与新列数）作为**事务作用域的 WAL 记录**写入该语句的事务记录序列 ⇒ 它与该事务的提交记录**同一次持久化**落地 ⇒ 崩溃后「数据面窄化是否已提交」与「窄化意图」在**同一个持久载体**上原子一致 ⇒ WAL 恢复对每个**已提交**的窄化意图发布目录窄化（幂等）⇒ ISS25 的崩溃-重开变体闭合。

> 关键点：**磁盘格式不变**。意图只存在于 WAL 的事务记录序列中，不进入 `__tables` 行 ⇒ MS36 的 G1/G2/G3 兼容面**不受任何影响**；`database-file-format-header` 域**无需修订**。WAL 格式新增一种事务作用域记录（旧 WAL 不含该记录 ⇒ 旧库未决 WAL 的恢复行为不变 ⇒ `legacy-format-compat` 的 E5 / E11 **保持**）。

**M2（提交点即事务提交）**：DDL 语句的成功 / 失败由**事务提交**决定。目录窄化的**发布**（`__columns` 链改写 + `column_count` 下调）是提交之后的**幂等发布步骤**；发布失败**不回滚已提交的数据面**（回滚在物理上不可能——数据已提交），也**不把语句改判为失败**。

**M3（提交前失败 ⇒ 目录面补偿）**：**提交之前**失败（计划期拒绝、目录面写失败、语句被放弃、超时）的 DDL，其已发出的目录面变更必须**补偿**：把目录内容恢复为语句执行前的状态，并**保持脏标记**，使后续任一次成功刷盘持久化的是**回退后的内容** ⇒ 失败的 DDL 不留下目录面变更。

**M4（同表布局 DDL 令牌化）**：`NarrowingState` 的「单表一个布尔量」改为**带所有者令牌的每表布局 DDL 令牌**（CAS 获取）：同表并发的布局 DDL **显式拒绝**（错误点名该表）；只有令牌持有者能清除在途状态 ⇒ 重叠清除不再丢失读拒绝；被拒绝的第二条 DDL **不会启动改写** ⇒ 「豁免读按旧 schema 解码」不再发生。

**M5（窄化意图未发布时不得截断 WAL）**：WAL 是窄化意图的唯一持久载体 ⇒ checkpoint **SHALL NOT** 在窄化意图尚未发布时截断 WAL（须先发布在途窄化，或推迟截断）。

### 2.4 统一性论证

M1–M5 是**同一个模型**的五个面，而不是三条最小修补：

- ISS25（崩溃-重开）由 **M1** 闭合：发布不再依赖「进程继续运行」，而依赖 WAL 中已提交的意图 ⇒ 恢复路径可判定。
- ISS28（失败 DDL 的目录面变更落地）由 **M3** 闭合：失败不再是「目录已脏、等下次刷盘落地」，而是「目录已补偿、回退内容被刷盘」。
- ISS29（同表并发 DDL）由 **M4** 闭合：不再有两个改写共享一个布尔量。

三者共享的**不变量**是同一条：**布局窄化的发布点唯一，且该发布点可由已提交的持久事实判定**（WAL 意图 = 已提交；目录内容 = 已发布）。

### 2.5 既有合同逐条核对

| 既有合同 | 结论 | 依据 |
|---|---|---|
| `ddl-catalog-transactional` R1（被放弃或失败的 DDL 不留下目录面变更） | **保持** | M3 的补偿把目录恢复到语句执行前状态；M2 保证「发布失败」不被改判为「语句失败」⇒ 失败的语句确实不留目录面变更 |
| `ddl-catalog-transactional` R2（目录变更与数据面改写同生共死） | **保持** | 发布严格发生在事务提交之后（数据面改写已完成）；提交之前目录不窄化 |
| `ddl-catalog-transactional` R3（放弃 DDL 后重开库目录与数据面一致） | **保持** | 放弃 ⇒ 未提交 ⇒ WAL 无已提交意图 ⇒ 恢复不发布窄化 ⇒ 目录与行宽一致 |
| `ddl-rewrite-consistency` R1–R3（含改写窗口内的读拒绝） | **保持** | M4 强化在途拒绝（令牌化）；豁免读仍严格限于改写路径 |
| `legacy-format-compat` G1/G2/G3 支持范围 | **保持** | 意图只在 WAL；磁盘格式不变；`__tables` 行不新增尾段 |
| `legacy-format-compat` E5 / E11（旧库未决 WAL 恢复） | **保持** | 新 WAL 记录类型是**新增可选**；旧 WAL 不含该记录 ⇒ 恢复行为逐条不变 |
| `database-file-format-header`（格式版本契约） | **保持** | 无格式版本变化 |
| `mvcc-tombstone-visibility`、`drop-persistence-boundary`、`free-page-lifecycle`、`index-write-side-atomicity`、`index-page-ownership`、`release-durability-precondition` | **保持** | 本裁定不触及这些域的判定面；M3 依赖 RDP R1「刷盘失败把脏标记交还该页」这一既有语义 |

**已知代价**（记录在案）：M1 需要 WAL 格式新增一种事务作用域记录 ⇒ **新构建写出的 WAL 不能被旧构建读取**（前向兼容不属于 `legacy-format-compat` 的契约面，属已知限制，在新域的 Purpose 中写明）。

---

## D3. 【方向裁定二】ISS32：维持既有合同后关闭（部分恢复边界成文）

### 3.1 事实基础

`rtsql restore` 逐条执行 dump 的语句、每条自动提交；中途某条被拒绝面挡住时**停止**，已执行的前序语句**已生效**；当前实现给出**显式**拒绝（点名 + 退出码 3 + 「此前语句已提交」提示），**不**静默报告成功 ⇒ 既有域 `legacy-format-compat` 的「SHALL NOT 静默地部分恢复后报告成功」**已满足**。

### 3.2 决定

**维持既有合同，把边界写成规范条款后关闭**；**不**实现原子 restore，**不**实现全量预检。

**理由**：

1. 原子 restore / 全量预检是**功能语义扩展**（新增「多语句事务」能力与「预检阶段」），MS37 的 Non-goals 明确排除「修复不夹带能力扩展」⇒ 把它塞进缺陷收口 change 会突破里程碑边界。
2. 原子 restore 需要跨语句事务，而 CLI 的执行模型是逐条 auto-commit；引入它会牵动 `sql-write-surface` / `mvcc-tombstone-visibility` 的事务边界合同 ⇒ 风险面远大于缺陷本身。
3. 当前行为**已经是非静默的**（显式拒绝 + 非零退出码 + 已提交提示）⇒ 用户不会被误导为「全部恢复成功」。缺陷台账把它登记为「既有合同，不是新缺陷」，且 MS36 已把它记录为已知范围外边界。
4. 全局最优解因此是「**把边界从『已知范围外』升级为『规范条款』**」：边界一旦成为规范，用户与后续实现者都能依赖它；而把它留在「已知边界」里，等于让一个会影响数据完整性的边界长期不可依赖。

### 3.3 合同核对

| 既有合同 | 结论 |
|---|---|
| `cli-noninteractive-shell` 的 restore 条款（逐条 auto-commit、失败即停、失败前语句已生效） | **保持**（措辞强化：把「多表 dump 的部分恢复」写成具名 Scenario） |
| `legacy-format-compat` 的「SHALL NOT 静默地部分恢复后报告成功」 | **保持**（现状已满足；新增 Scenario 锁定其观察口径） |
| `sql-write-surface` / `mvcc-tombstone-visibility` 的事务边界 | **保持**（不引入跨语句事务） |

**例外说明**：本条是**修订「边界是否成文」**而非修订行为语义；用户可见行为（错误文案、退出码、已提交提示）**逐条不变**。

---

## D4. 【方向裁定三】ISS29：每表布局 DDL 令牌（拒绝并发，而非排队）

**决定**：在途状态由布尔量改为**带所有者令牌的每表 CAS 令牌**；同表并发布局 DDL **显式拒绝**（错误点名该表）；被拒绝者不启动改写。

**为何不排队**：「语句级排队」需要在执行器层引入 DDL 的调度与超时语义（新机制、新失败模式、新合同），而拒绝是**可观察、可测试、零新增机制**的形态；且 MS37 的 Non-goals 排除能力扩展。

**合同核对**：`ddl-rewrite-consistency` 的「改写窗口内的读拒绝」**保持**（令牌化只把它从「布尔量」变成「有所有者」⇒ 重叠清除不再丢失拒绝）。

---

## D5. 【方向裁定四】ISS33：`dump` 的 `UNIQUE` 渲染按 `PRIMARY KEY` 同型条件

**决定**：`create_table_sql` 的 `UNIQUE` 分支补上与 `PRIMARY KEY` 分支同型的条件——**仅当该列的声明类型为 INTEGER 时输出 `UNIQUE`** ⇒ 非整数 `UNIQUE` 列的表，其 dump 产物不再包含该约束 ⇒ 该类表**可 restore 往返**。

**理由**：与 `PRIMARY KEY` 分支**同型**（既有域 `cli-noninteractive-shell` 已把「DDL 渲染 SHALL 仅在主键列声明类型为 INTEGER 时输出 `PRIMARY KEY`」写成规范）⇒ 不引入新的不对称；产物文本本身即声明了「该约束不在产物内」⇒ **不是静默的部分恢复**。

**合同核对**：

| 既有合同 | 结论 |
|---|---|
| `sql-constraint-enforcement`「UNIQUE 形态与类型面诚实化」的**计划期拒绝**（非 INT 列 `UNIQUE` 在建表计划期点名拒绝） | **保持**（计划期行为不变） |
| `sql-constraint-enforcement` Scenario「存量非 INT UNIQUE 库的边界已知」（restore 被拒绝面挡住） | **修订**（`dump` 不再产出该形态 ⇒ 新产物可往返；**旧产物**仍被拒绝面挡住，输入面的拒绝**保持**） |
| `legacy-format-compat` 范围外面（dump 文本含非整数列 `UNIQUE` ⇒ 显式拒绝） | **保持**（输入面不变）；迁移路径说明**补写**（当前导出的形态即为迁移目标形态，与 `PRIMARY KEY` 的既有说明同型） |
| `cli-noninteractive-shell` 的 dump DDL 渲染条款 | **修订**（补 `UNIQUE` 的同型条件） |

---

## D6. ISS26：页级判据的两条相反处置（按页型分组）

ISS26 的处置方向**不是**统一的一条，而是两条**相反**的方向：

| 路径 | 处置 | 理由 |
|---|---|---|
| 读 / 写路径 | **明确错误**（点名损坏的页与该索引树） | 跳过 ⇒ 点查返回「查不到该行」⇒ 静默错读（比 abort 更隐蔽）；既有域 `index-read-bounded-read` 已确立该语义 |
| 释放路径 | **计数 + 报出 + 跳过** | 释放是物理尽力而为；既有域 `release-path-bounded-read` 已确立该语义（告警 + 跳过 + 返回） |

**落点**：`src/storage/integrity.rs` 的 `check_tree_page` **叶页分支**补上「槽负载长度 ≥ `MAX_KEY_LEN + RowId::SIZE`（= 38）」的条目长度判据（与**内部页分支**已有的 `MAX_KEY_LEN + 4` 判据同型）⇒ 读 / 写路径经既有 `ensure_index_page_intact` 闸门得到明确错误；释放路径经既有 `collect_all_pages` 得到计数 + 报出 + 跳过 ⇒ **两条相反处置由同一处判据扩展同时达成**。

**合同核对**：`index-read-bounded-read`（**修订**：判据覆盖面扩大 + 标题措辞按 ISS34 收口）、`release-path-bounded-read`（**修订**：叶条目长度纳入释放路径的页级判据）、`index-write-side-atomicity` / `index-page-ownership` / `free-page-lifecycle`（**保持**）。

---

## D7. ISS27：其余页型的有界读——按页型分组判定

ISS27 的 ~21 处无界读点分布在不同页型，**各页型的「损坏页正确行为」不同**，不得一次统一。逐组判定（实现细节在该 Iteration 的 Cycle 中定稿）：

| 页型组 | 无界读点（约） | 损坏页的正确行为 | 依据 |
|---|---|---|---|
| 数据页 | `src/storage/data_page.rs` 2 处、`src/storage/data/table_manager.rs` 3 处、`src/executor/data_scan.rs` 2 处、`src/executor/create_index.rs` 1 处、`src/transaction/manager.rs` 1 处 | **明确错误**（点名页与槽）；缓冲池的布尔型辅助（`check_page_all_visible`）**降级为「非全可见」**（走慢路径），错误由实际取行处抛出 | 行内容不可读 ⇒ 跳过 ⇒ 静默丢行；与 IRBR 的读路径语义同型 |
| 目录页 | `src/storage/catalog.rs` 5 处 | 打开 / 查询路径**明确错误**（点名页与该目录行所属的表）；释放 / 重建路径沿用既有的尽力而为与「重建放弃」语义 | 目录行不可读 ⇒ 跳过 ⇒ 该表静默消失 |
| 恢复面 | `src/wal/recovery.rs` 4 处（ISS27 的槽负载越界形态：重放前候选扫描 / 更新重放所需的旧行推导 / 重放后索引重建 / 未提交事务标记）+ 1 处**槽目录**读点（ISS35 的页头槽数越界形态：`redo_tuple_at_row_id` 的幂等探测；该形态在 `src/wal/**` 未被 Iteration 002 覆盖） | **按恢复类别分别处置**（不是单一方向）：**已提交记录重放类**（旧行推导 + 幂等探测）⇒ 越界槽 SHALL NOT 跳过该记录 ⇒ 以明确错误结束恢复与打开；**扫描类**（重放前候选扫描 / 重放后索引重建 / 未提交事务标记）⇒ 跳过该槽（页级形态 ⇒ 跳过该**页**）、**计入恢复告警并报出**、其余可恢复的记录继续重放，不 abort。细化与理由见 `tasks.md` Iteration 005 的方向裁定 | 恢复面的一处损坏不应否弃其余可恢复的数据（⇒ 扫描类跳过 + 报出）；但跳过**必须**被报出，且不可忠实重放的**已提交记录**不得被静默丢弃（⇒ 重放类报错，与 `wal-recovery-replay-integrity` K05 同向） |
| 缓冲池 | `src/storage/buffer_pool.rs` 3 处 | 随其宿主页型（数据页）判定；布尔型辅助降级为安全值 | 同上 |

**合同核对**：新增域 `page-read-bounded-read`（承载）；`release-path-bounded-read`（**修订**：释放 / 重建面的既有尽力而为语义不变）；`index-read-bounded-read`（**保持**，本组不含索引树叶条目长度）；`data-integrity-verification`（**保持**：完整性扫描的诊断分级不变）。

---

## D8. ISS30 / ISS31：写盘失败的状态面（一个模型，两个落点）

**决定**：

- **ISS30**：页帧引入「内容在途未落盘」状态；刷盘取走页内容时置位、写入结束（成功或失败）后清除；**驱逐路径遇到在途页 SHALL NOT 按「干净」移出池**（该页内容要么已落盘，要么仍可在池内取回）。写盘失败时该页**留在池内**并上抛错误 ⇒ 新内容不丢失。
- **ISS31**：写盘失败标记**不再永不复位**：失败状态按**页**记录（失败页集合），该页**后续一次成功写入**后即移出集合 ⇒ 释放提升判定由「库级一次性标志」改为「锚点页不在失败集合内」。

**为何不引入新的持久状态**：失败集合是**内存态**，不需要落盘 ⇒ 不改磁盘格式。

**合同核对**：`release-durability-precondition`（**修订**：R1「刷盘失败把脏标记交还该页」**保持**，R2 的提升判据表述**修订**为按页口径）；`free-page-lifecycle`（**保持**：释放集合语义不变）；`drop-persistence-boundary`（**保持**）。

---

## D9. 语料面（ISS34 / I045 / I064）

- **ISS34**：`index-read-bounded-read` 的 Requirement 标题与 4 个 Scenario 标题的措辞残余（正文已收窄为「未通过页级判据」，标题仍写「损坏的索引页」）⇒ 标题与正文及已实现行为一致。**排在存储 Iteration 之后**，因为 ISS26 会扩展该域的判据覆盖面与 Scenario 集合 ⇒ 先扩内容、再一次性对齐措辞，避免二次返工。
  **承载形态 = 交接项**：OpenSpec 的 `MODIFIED` 块在归档时被校验「当前规格中的每个 Scenario 标题必须逐字出现在该块内」，否则归档拒绝该增量 ⇒ **既有 Scenario 的标题改名无法由增量表达**；Requirement 标题虽可用 `RENAMED` 表达，但改名后仍与既有 Scenario 标题并存 ⇒ 本 change 的增量**只承载行为变化**（ISS26），并逐字保留既有 Requirement 与 Scenario 标题；ISS34 的标题对齐与 I045 的 Purpose 补写**合并为同一批文档交接项**，交 `openspec-docs-maintainer` 在合并增量时同批应用（沿用 MS25「归档当轮由 Maintainer 补写真实 Purpose」的既有做法）。
- **I045**：14 处主 spec 的 Purpose 补写。Purpose 属**文档面**（不含行为要求）⇒ 按 AGENTS.md 的「轻量工作」规则**不建 change**；本 change 以**交接项**形式交 `openspec-docs-maintainer`，在收尾合并时随增量一并落位。
- **I064**：WAL 提交确认与 checkpoint 边界的文档化 ⇒ README 的性能对比段落补「追加 / 提交确认 / checkpoint 各自承诺」的区分，并明确「仅有 `sync_all` 调用或进程 SIGKILL 测试时，不宣称已验证完整断电安全」。同时在承载 WAL 提交语义的既有域补一条边界 Scenario（**保持**既有承诺，不扩大）。

---

## D10. 验证预算与 M19 分批串行纪律

- **全量回归在整个 change 只跑一次**，安排在**最后一个 Iteration（008；2026-10-25 二次结构修正后为 009，见 D15）**的收尾验证。
- 分批方式：测试目标按**字典序**分组，每组 8–12 个目标，每批一次独立的 `cargo test --test <目标> … -- --test-threads=1`；批间**顺序执行**，同一时刻只有一批在跑。
- **SHALL NOT** 用一次无目标过滤的全量调用跑完；**SHALL NOT** 用默认并行度跑任何全量批次。
- 全量 Gate 等价于「全部批次各自 0 failed」；缺一批不得声明全量通过。
- 前次全量终态（2231 / 0 / 2，143 目标 / 14 批）**在本次改动前已失效**，不得作为依据。
- 其余 Iteration 只跑与改动面相关的定向回归（受影响范围的测试目标 + `cargo test --lib -- --test-threads=1`）。

---

## D11. 禁止事项（贯穿全 change）

- **不得**为构建、测试、鉴定测试、Evidence 或运行归属新增 Hash / 指纹 / revision pin / run-id / session ID / peer pin / artifact manifest / 日志 Hash 链 / TIME_ORDER 时间证明及其采集、审计、鉴定工具与专用测试。
- **不得**在项目测试框架之外建立判定层（脚本 / 封装命令 / 专用参数）。
- **不得**改磁盘格式（保 MS36 的 G1 / G2 / G3 兼容面）；WAL 的新增记录类型是**新增可选**，不改变旧记录的解析。
- **不得**引入跨语句事务、原子 restore、全量预检、非整数列 `UNIQUE` 的计划期支持、索引名注入 `IndexManager`。
- **不得**引入 ≥ 300 行的表规模构造用例（ISS23 约束）。

---

## D12. ISS35：页头槽数越界的读路径 abort——承载形态与三个待判问题的裁定

**问题**：`SlottedPageRef::get_slot`（`src/storage/page_format/slotted_page.rs:96-110`）与可写变体 `SlottedPage::get_slot`（`:183-201`）用 `let slot_start = Page::PAGE_SIZE - (index + 1) * Slot::SIZE` 直接算槽目录项位置，**无下溢保护**；读路径的迭代上界取自页头的 `slot_count` ⇒ 页头 `slot_count ≥ 683` 时该减法下溢 ⇒ 切片越界 panic ⇒ **进程 abort**。台账：`.agents/issues/ISS35-slot-count-underflow-aborts-read-path.md`（根因 `Confirmed（读码确认）`）。

**与 ISS27 的关系**：同域不同机制 —— ISS27 是槽**负载**（offset / length）越界，ISS35 是槽**目录**定位下溢（页头 `slot_count` 越界）⇒ **不合并**、独立承载、独立关闭。

**承载形态 = 独立 Iteration 002**（T17，`iterations/002-slot-count-underflow/000-initial.md`，Cycle Type = `initial`），**排在 T4（Iteration 003；**2026-10-25 二次结构修正后为 Iteration 004，见 D15**）之前**以冻结判据形状。**理由（执行正确性）**：T4 / T5 的读点同样经由 `get_slot` 取得 `&Slot`；若 ISS35 的修复改变 `get_slot` 的形状（返回 `Option` / `Result` 或新增界限），T4 / T5 必须写在最终形状上，否则返工 ⇒ 把 ISS35 排在**紧邻 T4 之前**的独立 Iteration（002），既保证执行顺序，又不与其它页型组共用一个 Iteration。**（原承载形态为「Iteration 001 内的本地后继 Cycle，Cycle Type = `replan`」；2026-10-25 结构修正见 D14；**2026-10-25 二次结构修正见 D15 —— 写路径组插入为 Iteration 003，目录页组顺延为 Iteration 004；「ISS35 排在 T4 之前」这一实质结论不变。**）**

**台账三个待判问题的裁定**：

| 问题 | 裁定 | 理由 |
|---|---|---|
| ① 修复位置（`get_slot` 内部 vs. 各调用点） | **页格式层提供唯一的容量界限 + 页级判据，由读点在取槽之前消费**：`MAX_PAGE_SLOTS`（值 680）从 `src/storage/btree/node.rs:11-14` **上移**到 `src/storage/page_format/slotted_page.rs`（索引树侧改为引用，**取值与判据不变 ⇒ 索引树路径零回归**）；页格式层另给出一个**页级判据**（页头声称的槽数是否超出该页的槽目录容量），阈值与 `integrity::validate_slots` 的 `slot count exceeds the page capacity` violation **同源**（`slot_count ≥ 681`，严于 abort 阈值 683 ⇒ 同时覆盖「槽目录压到页头」与「定位下溢」两种形态）。**不改 `get_slot` 的 `None` 语义** | 原语层把越界索引折叠成 `None` 会与「槽不存在」**不可区分** ⇒ 在尚未纳入判据的读点（目录页组 / 恢复面组）上会把 abort 变成**静默跳过**（违反「跳过 = 静默错读」）。判据必须在**取槽之前**问（取槽即可能 abort） |
| ② 损坏页在该页型的正确行为 | 数据页组 ⇒ 明确的 `StorageError::CorruptedDataPage`（点名该页与页头声称的槽数）；目录页组 ⇒ 明确错误 ⇒ **该库 SHALL NOT 打开**（沿用 D7 / Cycle 000 的裁定）；恢复面组 ⇒ 按 T5 已定的两类语义（已提交记录重放类 ⇒ 明确错误；扫描类 ⇒ 跳过 + 计入告警并报出） | 与 D7 的按页型分组判定同向；各页型的既有损坏语义不在本裁定中被统一 |
| ③ 是否把可写变体与 `add_slot_inner` 的同形空间检查纳入同一 Iteration | **不纳入** | ISS35 台账明确「本条只主张读路径」；写路径的同形无界访问需要独立方向 ⇒ 作为 **Issue 候选**上报（只报告不登记）：`src/storage/data_page.rs:237` 的 `page.data[slot_offset..slot_offset + VersionHeader::SIZE]`（负载偏移越界 ⇒ panic）与 `SlottedPage::get_slot` 的槽目录下溢；`src/wal/recovery.rs:1519` 从损坏页的槽取 `logical_id` 后调用 `update_version_header_in_data_page` ⇒ **恢复面可达**该 panic（属 T5 的范围） |

**读点覆盖与关闭条件**：T17 覆盖**数据页组**（T3 的 11 个读点 + 可写变体的 `update_version_header_in_data_page` 取槽点）；目录页组在 T4、恢复面组在 T5 应用**同一判据** ⇒ **ISS35 台账条目的关闭条件 = 本 change 内三组全部完成**（关闭由 Recorder 按用户指令执行，Plan 不改台账）。

**边界（不并入本裁定）**：① `get_slot` 的 `None` 语义与签名不变 ⇒ 原语仍是「按页头 `slot_count` 迭代槽」的隐患点 ⇒ 页格式层写入规则：**任何按页头槽数迭代槽的读点必须先问页级判据**；② T17 **不跑全量**（change 预算在 Iteration 008 / T16；**2026-10-25 二次结构修正后为 Iteration 009 / T16，见 D15**）；③ T17 之外的既有基线条件（全仓 fmt / clippy）见 D13。

**合同核对**：`page-read-bounded-read`（**修订**：新增 Scenario「页头声称的槽数超出槽目录容量时读路径不 abort」）；`index-read-bounded-read`（**保持**：索引树侧判据与取值不变）；`release-path-bounded-read`（**保持**）；`data-integrity-verification`（**保持**：完整性扫描面已由 `validate_slots` 有界，不受影响）。

---

## D13. ISS36：项目门（`cargo fmt --check` / `cargo clippy --all-targets -- -D warnings`）在 HEAD 即不可满足——**范围外排除项**

**问题**：README.md / README.zh-CN.md 第 324–325 行列出的两个项目门，在**当前 HEAD（`e9cb9d3`）上即已失败**，与本 change 的任何改动无关。台账：`.agents/issues/ISS36-fmt-clippy-gates-unsatisfiable.md`。

**根因基线事实（2026-10-25 在 HEAD 独立核实）**：

- 项目**无** `rust-toolchain.toml` ⇒ 工具链由环境决定（仓库内另装 `nightly` 与 `nightly-2026-02-25` 两个本地工具链）。
- `cargo fmt --check` 在 HEAD 报 **50 处 diff，分布在 9 个既有文件**（是「9 个文件」而非「50 个文件」——此前口述有误，以本数字为准）。
- `cargo clippy --all-targets -- -D warnings` 在 HEAD 即失败：**17 条 lib 错误**（deny 模式下 lib 即中止，test 目标未被 lint）；非 deny 模式为 **32 条 warning**。
- 台账记录的「第二次测量 = fmt 59 处 / 12 文件、deny-clippy 18 条」是**实施中途**的快照（当时工作区含本 change 的未格式化改动）；本 change 的各 Cycle 只对本 Cycle 触碰的文件运行 `rustfmt` ⇒ 当前工作区已回到上述 HEAD 基线（9 个文件 / 17 条 lib）。

**裁定：不在本 change 实施。** 理由：

1. 修复形态之一是**仓库级的格式重排**——跨 12 个文件，且其中数个正是本 change 修改的文件（`src/storage/data/table_manager.rs`、`src/storage/integrity.rs` 等）⇒ 把他人的既有格式差异与本 change 的行为改动混在同一批 diff 中，违反 **Surgical Changes**，且会让本 change 的 diff 失去可审计性。
2. 修复形态之二是**新增工具链固定文件**（`rust-toolchain.toml`）⇒ 改变**整个仓库的构建工具链** ⇒ 在途各 Cycle 已取得的编译与验证基线（`cargo test --lib` 420/0、定向回归 354/0 等）全部失效，须在**独立的 change** 中连同**全量回归**一并处理。
3. 两条路径都超出本 change 的范围边界（MS37 的缺陷台账收口），且与本 change 的需求、Acceptance、任务无因果关系。

**对本 change 的处置**：各 Cycle 只核对**本 Cycle 触碰的文件**在 `cargo fmt --check` / `cargo clippy --all-targets` 下**零新增 finding**（既有基线差异不动、不重排）；本 change **不**声明「项目门通过」。

**对收尾措辞的影响（须如实表述）**：本 change 收尾时，**项目门在 HEAD 即不可满足，属既有基线条件** —— 不得写成「项目门通过」或「fmt / clippy 干净」；收尾报告只能表述为「本 change 触碰的文件零新增 finding；仓库级门在 HEAD 即为红，属既有基线条件（ISS36 台账，范围外）」。

**Issue 台账**：ISS36 由 Recorder 建档并保持 `open`；本 change **不**为其写实现计划、任务或 Iteration；台账的关闭由 Recorder 按用户指令在独立 change 完成后执行。

---

## D14. 【结构修正，2026-10-25】Iteration 001 拆为四个各自承载单一任务的 Iteration（001 / 002 / 003 / 004）

**问题**：修正前，本 change 的 Iteration 001 同时承载 **4 个任务**（T3 数据页组 / T17 ISS35 / T4 目录页组 / T5 恢复面组），用同一目录下的 **3 个 cycle 文件**展开，其中 `002-catalog-bounded-read.md` 的 `Cycle Type` 写作 `initial` 却不是该 Iteration 的首个 cycle；`001-slot-count-underflow.md` 写作 `replan`，但其语义并非「执行修订后的计划」而是「执行一个新增任务」。

**偏离的规则（诊断依据）**：

1. 公共规则（AGENTS.md › Iteration 与 Cycle 线程）：「Rework Cycle 使用 `001-rework.md` 等**本地编号**完成既有 Acceptance，不修改 Iteration Map；Replan Cycle 使用同一目录的**后继编号**执行修订后的计划」⇒ 同目录的后继编号是 rework / replan 的**专用**机制，不是为新任务开第二个 cycle 的通道。
2. 公共规则：「当前 Iteration 只有在 Review Result 为 `accepted` 后才能完成并展开 Map 中的下一 Iteration」⇒ 继续推进的合法机制是**展开下一个 Iteration**。
3. 公共规则：「Plan 在 change `tasks.md` 中把全部任务分配到工作量适中、可独立验证和诊断的 Iteration，**只展开当前 Iteration 目录和当前 Cycle**」⇒ 一个 Iteration 目录内同时存在 3 个 cycle 文件、且其中两个是 `initial` 类型，与「只展开当前 Cycle」相悖。
4. 项目先例（`openspec/changes/archive/`，**64** 个已归档 change）：全部**后继** cycle 文件名（共 **29** 个）的后缀**全部**是 `replan`（**19** 个）或 `rework`（**10** 个）——**无一个**是为新任务而开的第二个 cycle；每个 Iteration 只有一个承载任务的 `000-initial.md`。最接近的先例 `2026-10-24-open-defect-batch-storage-ddl-legacy-evidence` 把 5 个故障域拆成 **8 个 Iteration**。

**被移动的路径**：

| 旧路径 | 新路径 | 任务 | Cycle Type |
|---|---|---|---|
| `iterations/001-bounded-read-other-page-types/000-initial.md` | `iterations/001-data-page-bounded-read/000-initial.md` | T3 | `initial`（不变） |
| `iterations/001-bounded-read-other-page-types/001-slot-count-underflow.md` | `iterations/002-slot-count-underflow/000-initial.md` | T17 | `replan` → **`initial`** |
| `iterations/001-bounded-read-other-page-types/002-catalog-bounded-read.md` | `iterations/003-catalog-bounded-read/000-initial.md` | T4 | `initial`（不变；文件名改为 `000-initial.md`） |
| （未展开） | `iterations/004-recovery-bounded-read/000-initial.md` | T5 | **本次不创建**；按正常流程在 T4 被接受后由 Plan 展开 |

**（2026-10-25 二次结构修正见 D15：写路径组插入为 Iteration 003 ⇒ 上表 T4 行的「新路径」再次顺延为 `iterations/004-catalog-bounded-read/000-initial.md`（Iteration 004）；T5 行相应顺延为 `iterations/005-recovery-bounded-read/000-initial.md`（Iteration 005，**仍未创建**）。）**

**修正后的结构**（Iteration Map 见 `tasks.md`）：

| Iteration | 标题 | Tasks | 依赖 | 状态 |
|---|---|---|---|---|
| 000 | 索引树叶条目长度纳入页级判据 | T1, T2 | None | accepted |
| 001 | 数据页组的有界读（ISS27 · T3） | T3 | 000 | accepted |
| 002 | 页头槽数越界的读路径 abort（ISS35 · T17） | T17 | 001 | in progress（Act Response `reported`，待 Plan Review） |
| 003 | 目录页组的有界读（ISS27 · T4） | T4 | 001, 002 | planned（Plan Context `ready`） |
| 004 | 恢复面组的有界读（ISS27 · T5） | T5 | 003 | planned（**尚未展开**） |
| 005 | DDL 提交点统一模型 | T6, T7, T8 | 004 | planned（原 002 顺延） |
| 006 | 写盘失败的状态面 | T9, T10 | 005 | planned（原 003 顺延） |
| 007 | dump / restore 的合同面 | T11, T12 | 006 | planned（原 004 顺延） |
| 008 | 语料收口与收尾全量回归 | T13, T14, T15, T16 | 007 | planned（原 005 顺延） |

**（2026-10-25 二次结构修正见 D15：写路径组（T18 / T19）插入为 Iteration 003 ⇒ 上表 003→004、004→005、005→006、006→007、007→008、008→009 依次顺延，当前共 10 个 Iteration（000–009）；权威的当前 Iteration Map 见 `tasks.md`。）**

**T17 的 `Cycle Type` 由 `replan` 改为 `initial` 的理由**：`replan` 的语义是「执行**修订后**的计划」—— 即该 Iteration 的既有计划被修订后才执行本 Cycle。T17 是一个**新增任务**，承载它的 Iteration 002 的计划**就是**以本 Cycle 为首个 Cycle 建立的，不存在「先有计划、再修订」的先前计划 ⇒ 该 Cycle 是新 Iteration 的**首个** cycle ⇒ `Cycle Type: initial`。原判定「ISS35 必须先于 T4 / T5 落地以冻结 `get_slot` 的最终形状」**不变**——它由**迭代顺序**（002 紧邻 003 之前）保证，不再依赖同目录内的 cycle 序号。

**修正的边界（硬约束）**：

- **本次修正不改变任何 Task Definition / Acceptance / 验证内容**：各 cycle 文件的 Plan Context 正文（目标、调查事实、Task Definitions、Acceptance、验证命令、Gate 2 结论、Persisted Evidence、风险）与已写入的 Act Response / Plan Review **逐字保留**；只改动文件所在目录与文件名、文件头部的「所属 Iteration」标识、以及指向其它 cycle / Iteration 的路径与序号引用。
- **不修改产品代码**（`src/`、`tests/` 一律不动）、**不运行任何测试**、**不修改**全局 `.agents/docs/tasks.md` / `SNAPSHOT` / 项目记忆 / references、**不修改** Issue 台账（ISS35 的「处置指针」字段归 Recorder 所有）、**不创建** Iteration 004 的 cycle 文件、**不执行 git commit**、不归档 change。
- 各页型组的**方向裁定**（D7 的分组判定、T4 的「目录页损坏 ⇒ 该库 SHALL NOT 打开」、T5 的两类恢复语义）与 ISS35 的三个待判问题裁定（D12）**全部不变**，只改承载面。

**交叉引用同步范围**：`tasks.md`（任务表归属列与状态备注、任务执行依据要点、RTM、Iteration Plan 四节拆分、平衡审计表、Iteration Map 表、Next Iteration）、`design.md`（D1 的 Iteration 数量与拆分信号措辞、D10 的收尾全量 Iteration 号、D12 的承载形态）、`proposal.md`（收尾全量的 Iteration 号）、以及 4 个 cycle 文件正文中指向其它 cycle / Iteration 的序号与路径。

---

## D15. 写路径的槽界限（ISS37 + ISS38）：承载形态、依赖顺序与判定原则

**问题**：Iteration 002（T17 / ISS35）的 Act 阶段把「写路径的同形无界访问」作为 Issue 候选上报（`iterations/002-slot-count-underflow/000-initial.md` 的 Act Response「Experience Candidates」第 2 项的第 1 / 2 条）。Recorder 已按用户指令独立核实并建档为 **ISS37**（写路径的页头槽数下溢）与 **ISS38**（写路径的槽负载偏移无界写）⇒ 依 goal 指令「中途发现并登记的 issue 直接在那个 change 分配 iter 进行解决」，本 change 必须为二者分配承载 Iteration，并写明其与既有决策的依赖顺序、错误形态与失败语义。

### D15.1 承载形态

**承载形态 = 独立 Iteration 003「写路径的槽界限（ISS37 + ISS38）」**，唯一 Cycle `iterations/003-write-path-slot-bounds/000-initial.md`（Cycle Type = `initial`，Plan Context 状态 `ready`，Gate 2 全部 PASS），承载两个任务：

| 任务 | 承载的缺陷 | 范围（根因实现点） |
|---|---|---|
| T18 | ISS37：写路径的页头槽数下溢 | `SlottedPage::add_slot_inner`（`src/storage/page_format/slotted_page.rs:294-299`）与 `SlottedPage::free_space`（`:405-409`）对页头 `slot_count` 的**同形无保护减法** `Page::PAGE_SIZE - slot_count * Slot::SIZE`（`slot_count ≥ 683` 时整数下溢）＋ `SlottedPage::delete_slot` 的同形减法（`:340-367`）＋ 既有「`Err(_)` ⇒ 换新页继续」分支对损坏页的**跳过**行为 |
| T19 | ISS38：写路径的槽负载偏移无界写 | `update_version_header_in_data_page`（`src/storage/data_page.rs:266-273`，写长 `VersionHeader::SIZE = 22`，不校验该槽自身的 `length`）＋ `SlottedPage::update_slot_in_place`（`src/storage/page_format/slotted_page.rs:384`；ISS38 台账记其 `copy_from_slice` 在 `:397-399`） |

**两个同族兄弟点纳入同一 Iteration 的理由**（二者在台账中都是「本条不主张」的相邻面，由本裁定纳入）：

- `delete_slot` / `delete_slot_by_logical_id` 的同形减法 → 归 **T18**：与 ISS37 **同形、同文件、同原语族**（同一表达式 `PAGE_SIZE - n * Slot::SIZE`），且有生产触点（`src/storage/data_page.rs:327`、`src/storage/catalog.rs:824` / `:987`）⇒ 排除它会留下一个**同规则的已知 abort 原语**，使「写路径的槽界限」名义闭合而实际未闭合。
- `update_slot_in_place` → 归 **T19**：与 ISS38 **同族**（同一「直接消费槽目录项的 `offset` / `length` 而不判界」的机制），且是目录行**唯一**的生产原地改写路径（`src/storage/catalog.rs:924`）⇒ 排除它会使写侧的负载判界**不完整**。

**T18 / T19 不拆为两个 Iteration 的理由**：同一批页格式层写入原语、同一条「判据唯一 ＋ 错误同名同形」形态、同一套夹具（`tests/write_path_slot_bounds_test.rs`）、同一验证面（定向回归 20 目标，M19 两批各 10、批内 `--test-threads=1`、批间串行 ＋ `cargo test --lib`）；且 T19 依赖 T18 落定的**共享判界原语** ⇒ 拆开会重复引入判据（与 001 / 002 / 004 / 005 各页型组同理由，见 `tasks.md` 平衡审计表「合并检查」）。

### D15.2 依赖顺序（两条硬依赖）

1. **写路径必须排在 T5（恢复面组）之前**：恢复重放经 `src/wal/recovery.rs:1519` 到达 `src/storage/data_page.rs:272` 的无界写（ISS38 台账「生产触达路径」表）⇒ 写路径未收口时，T5 的「恢复面有界」保证是**空**的。本 Iteration 对 `src/wal/**` **零 diff**，但经 `update_version_header_in_data_page` **自动**覆盖 `recovery.rs:1519` ⇒ T5 不得重复实现同一有界写。
2. **写路径必须排在 T4（目录页组）之前**：写路径触及 `src/storage/catalog.rs:707`（目录页链追加 `add_slot`）与 `src/storage/catalog.rs:924`（`update_slot_in_place` 的生产调用点），与 T4 **同改目录页写入点** ⇒ 两个 Iteration 不应相邻改动同一批点。

⇒ **Iteration 顺序**：002（T17）→ **003（写路径 · T18 / T19）** → 004（目录页组 · T4）→ 005（恢复面组 · T5）→ 006（DDL）→ 007（写盘失败）→ 008（dump-restore）→ 009（语料 ＋ 收尾全量）。原 Iteration 003（目录页组）**顺延为 Iteration 004**，其 Cycle 目录 `iterations/003-catalog-bounded-read/` 已重命名为 `iterations/004-catalog-bounded-read/`（内容除「所属 Iteration 标识」与「路径 / 顺序引用」外逐字保留）。**权威的当前 Iteration Map 见 `tasks.md`**。

### D15.3 与既有决策的一致性

**与 D12（读路径冻结方向）的关系**：D12 冻结了四件事——页级判据**前置**（读点在取槽之前问页级判据）、`MAX_PAGE_SLOTS` **唯一定义**（上移到页格式层，索引树侧取值与判据不变）、**命名明确**的 `CorruptedDataPage` 错误（点名该页）、**不改 `get_slot` 的 `None` 语义**。写侧**复用前两项与第三项**：

- 写侧**复用同一页级判据**：Iteration 002 冻结的页级判据（`slot_count_exceeds_capacity()`，`src/storage/page_format/slotted_page.rs:89-91`，全项目唯一实现，阈值 `slot_count > MAX_PAGE_SLOTS`）由页格式层写入原语在**消费页头 `slot_count` 之前**询问 ⇒ 不另造阈值、不另造判据；
- 写侧**复用同一槽负载判界**：T3 冻结的槽负载判界**上提为读侧 / 写侧共用的唯一实现**（索引树侧既有取值与判据不变）；
- 写侧**复用同一错误形态**：`StorageError::CorruptedDataPage { page_id, detail }`（**同名同形**），detail 点名该页（负载形态时同时点名该槽；槽负载越界形态额外点名该槽的字节区间）⇒ 不新增错误变体。

**但写路径的失败语义与读路径不同**：读路径 ＝ 查询失败 ⇒ **错误返回**；写路径 ＝ 写入失败 ⇒ **语句失败并回滚事务**，且页损坏**不得**换页继续（既有「`Err(_)` ⇒ 换新页继续」分支必须区分「页满 / 页损坏」）、**不得**折叠成「行 / 槽不存在」、**不得**静默跳过 ⇒ 该差异写成独立 Requirement（见 D15.5）。D12 的「不改 `get_slot` 的 `None` 语义」裁定**不适用于写侧**——写侧走的是写入原语自身的错误通道，不是 `get_slot` 的 `None` ⇒ 两处裁定互不冲突。

**与既有域 `write-failure-state`（ISS30 / ISS31 的写盘失败状态面）不冲突**：两者是**不同事实**，触发条件、恢复动作、状态位置均不同：

| | 写侧页损坏（本 D15 · T18 / T19） | 写盘失败状态面（`write-failure-state` · ISS30 / ISS31） |
|---|---|---|
| 触发条件 | 页内容**已损坏**（页头 `slot_count` 越界 / 槽目录项 `offset` / `length` 越界）——写入**前**即可判定 | 刷盘 / 落盘失败——**持久化**面失败 |
| 可否重试 | 否（确定性、永久条件） | 是（暂态；失败标记按页记录并可复位） |
| 恢复动作 | 语句失败 ⇒ 会话事务回滚（既有 `rollback_session`） | 页内容状态与释放提升的可判定性（既有 `last_failed_write` 语义） |
| 承载面 | 本 Iteration 003 | Iteration 007 |

⇒ 两者**不共用状态、不互相遮蔽、不互相表述**：delta spec 的「写入失败使语句失败并回滚事务」Scenario 明确要求该错误 **SHALL NOT** 被表述为刷盘 / 落盘失败。

### D15.4 与台账的对应

| 台账 | 路径 | 状态 | 承载 |
|---|---|---|---|
| ISS37（写路径的槽目录容量下溢） | `.agents/issues/ISS37-write-path-slot-count-underflow.md` | `open`（根因 `Confirmed（读码确认，未构造复现）`） | Iteration 003 / T18 |
| ISS38（写路径的槽负载偏移无界写） | `.agents/issues/ISS38-write-path-slot-payload-offset-unbounded.md` | `open`（根因 `Confirmed（读码确认，未构造复现）`） | Iteration 003 / T19 |

**「处置指针」字段归 Recorder 所有** ⇒ 本 change **不改台账**；承载分配记录在 `tasks.md`（任务表 T18 / T19 ＋ Iteration 003 计划）与本 D15，由上层转交 Recorder 回填。两条台账的**关闭条件**相同：T18 / T19 的 Acceptance（A12–A15）全部满足且 Plan Review 判定 `accepted`。

### D15.5 规格承载

**为何新建 `page-write-bounded-write` 规格域，而非扩展读侧的 `page-read-bounded-read` 域**：读侧与写侧是**并列**的判定，而不是同一条判定的两半——读侧对「槽负载短于待读长度」的槽的既有逐点语义**不受影响**（读侧按各页型的既有语义处置，不因此报错），写侧对「短于待写长度」的槽**报错**；两者的 Requirement / Scenario 约束的是**不同类别**的可观察行为（读 ＝ 查询失败并返回错误；写 ＝ 语句失败并回滚事务）⇒ 混在同一域内，收尾合并时无法判定「哪一条 Requirement 约束哪一类行为」。⇒ 新建独立域，读侧域 `page-read-bounded-read` 的正文**不改**。

该域的 Scenario 与要点（`specs/page-write-bounded-write/spec.md`）：

1. **「页头声称的槽数超出槽目录容量时写路径不 abort 且不换页继续」**（T18）——不触发整数下溢 / 越界切片 / 越界写 / panic / 进程 abort；以**点名该页与页头声称的槽数**的明确错误结束；该错误 **SHALL NOT** 被当作「该页已满」而**换新页继续**（换页会跳过损坏页继续落盘 ⇒ 损坏被静默掩盖），**SHALL NOT** 使写入被静默跳过；槽目录容量的界限全项目**只有一处定义**，索引树侧的既有判据与取值不变。
2. **「槽负载越界或过短时写路径不越界写且不静默改写相邻字节」**（T19）——不产生任何越界写，不覆盖该槽自身负载之外的相邻字节；以**点名该页与该槽**（槽负载越界形态含该槽的字节区间；载荷不足形态点名该槽的负载长度）的明确错误结束；**读侧对「负载过短」的既有逐点语义保持不变**。
3. **「写入失败使语句失败并回滚事务」**（T18 / T19）——命中损坏页的写入语句以**明确的错误**失败且错误点名该页；该失败使会话事务**失败 / 回滚**（不留下「已生效的一半」）；该错误 **SHALL NOT** 被表述为刷盘 / 落盘失败（见 D15.3）。
4. **「未损坏路径的行为零变化」**（对照）——既有写入路径（INSERT / UPDATE / DELETE / 目录追加 / 目录行改写 / 恢复重放的落位与版本头改写）的可观察行为与既有行为**逐项一致**（既有错误文案逐字不变；「页已满 ⇒ 换新页继续」的既有路径不变）；索引树侧的既有判据与取值不变。

### D15.6 对 ISS35 的关系

ISS37 / ISS38 与 ISS35（`.agents/issues/ISS35-slot-count-underflow-aborts-read-path.md`）是**读侧 / 写侧的同族，但机制独立** ⇒ 不合并、独立承载、独立关闭：

| | ISS35（已由 Iteration 002 / T17 承载，`accepted`） | ISS37 / ISS38（本 D15 的 Iteration 003 承载） |
|---|---|---|
| 侧 | **读**路径 | **写**路径 |
| 触发字段 / 形态 | 页头 `slot_count` 越界 ⇒ **槽目录定位**下溢 | 页头 `slot_count` 越界被无保护地参与**减法**（ISS37）；槽目录项 `offset` / `length` 越界（ISS38） |
| 检测谓词 | 取槽**之前**问页级判据（12 个取槽点） | 写入原语**内部**先问同一页级判据 ＋ 槽负载判界 |
| 承载面 | 数据页组的取槽点 | 页格式层的写入原语（落位 / 删槽 / 原地改写） |

ISS35 台账明确「本条只主张读路径」，并把 `add_slot_inner` / `free_space` 列为「相邻面（本条不主张）」⇒ **本 D15 就是该相邻面的承载裁定**。

### D15.7 被 D15 取代的既有表述（失效指针校正）

本 D15 生效后，下列**仍指向旧迭代号 / 旧路径**的表述作废。对应位置按「**追加标注**」处理（**冻结正文不改写**）；本表是这些标注的汇总，权威的当前结构以 `tasks.md` 的 Iteration Map 为准。

| 位置 | 旧值 | 现值 |
|---|---|---|
| `design.md` D1（Iteration 数量） | 9 个 Iteration（000–008） | **10 个 Iteration（000–009）** |
| `design.md` D1（ISS27 的分列） | ISS27 ＝ Iteration 001 / 003 / 004 | ISS27 ＝ Iteration **001 / 004 / 005** |
| `design.md` D10（收尾全量的 Iteration 号） | 008 | **009** |
| `design.md` D12（T4 的 Iteration 号） | Iteration 003 | Iteration **004** |
| `design.md` D12（全量预算的 Iteration 号） | Iteration 008 | Iteration **009** |
| `design.md` D14（被移动的路径表 · T4 行） | `iterations/003-catalog-bounded-read/000-initial.md` | `iterations/004-catalog-bounded-read/000-initial.md` |
| `design.md` D14（修正后的结构表） | 003 ＝ 目录页组 … 008 ＝ 语料 | **003 ＝ 写路径组**；其后 003→004、004→005、005→006、006→007、007→008、008→009 依次顺延 |
| `iterations/002-slot-count-underflow/000-initial.md`（Plan Context「停止边界」） | 展开 Iteration 003（T4，`iterations/003-catalog-bounded-read/000-initial.md`） | 实际后继 ＝ **Iteration 003（写路径组）** → Iteration **004**（T4，`iterations/004-catalog-bounded-read/000-initial.md`） |
| `iterations/000-…` / `iterations/001-…` 的 Plan Context 与 Plan Review 中的 Iteration 号 | 页型组 001 / 003 / 004；后续组 005 / 006 / 007 / 008 | 页型组 001 / **004** / **005**；后续组 **006** / **007** / **008** / **009** |
| `iterations/002-…` 的 Plan Context「Excluded scope」/ RTM /「后续处理事项」中的 Iteration 号 | T4 → 003、T5 → 004、后续组 005 / 006 / 007 / 008、全量 → 008 | T4 → **004**、T5 → **005**、后续组 **006 / 007 / 008 / 009**、全量 → **009** |
| `tasks.md`（读点计数的适用域） | ISS27 域的 Iteration 001 / 003 / 004 | ISS27 域的 Iteration **001 / 004 / 005** |
| `proposal.md`（收尾全量的 Iteration 号） | Iteration 008 | Iteration **009** |
