# e2e 旅程验证面建设与缺陷清偿（ISS05–ISS14 全量收口）

## Why

用户 2026-10-13 指令：**①建立端到端测试验证数据库整体功能使用正常**（单元/特性矩阵测试不能保证的组合与旅程级正确性）；**②把 e2e 暴露的问题与已有 Issue（ISS05–ISS14，10 项 open）一起解决，直到验证功能完全正常**。多 Iteration 单 change。

现状依据：

1. **缺陷台账**：2026-10-12 外部评测 13 项反馈经 Explorer 核实全部成立（R32 分析），其中 10 项已落账 ISS05–ISS14（P0×2 / P1×3 / P2×5），P2-10/11 两项行为定性留用户裁定（本次已裁定，见下）。
2. **测试缺口**：现有 1929 tests 以特性矩阵为主（119 集成目标），缺「用户旅程」级组合验证——ISS05 类缺陷（多叶等值连串需规模才出现）、ISS06 类（建表→写→崩溃的多步状态转换）正是矩阵测试的结构盲区。`tests/e2e_test.rs` 现为空壳（注释指向他处）。
3. **Plan 调查修正 ISS05 机制记录**（本会话探针实证，临时探针已删除，证据链见 design D2/D3）：ISS05 台账记载「条目缺失产生行 Unconfirmed（回填漏行或分裂丢条目）」——**两者均不成立**。回填（`create_index.rs`）30000 行全部正确插入；正确根因是**等值下探逻辑漏洞**（机制 B：同值键跨越「小于 K 的分隔键」边界散布，search_all 遇等值分隔键即跳过常规路由，漏访边界子树；delete_exact 单径下探同源 → eval 的 `UPDATE … Key not found`）。调查同时发现两项**台账未登记的同面新缺陷**：
   - **B1 叶子链破坏**：`LeafNode::shift_slots_right`（node.rs:164-197）整页重建不恢复 `next_leaf_page_id`——任何非末位插入（乱序主键、二级索引回填）都把该叶的链指针清零（全页普查实证：~90% 叶子 next=0）。
   - **B2 异步 scan_all 结构性失效**：`IndexManager::scan_all`（index_manager.rs:339-373）把内部页当叶读（条目经长度校验静默丢弃）、只沿 leftmost 链走、以 page 0 为终止符——**任何 ≥2 层的树返回空/碎片**（全保真 30000 行实测返回 190）。消费面：无 WHERE 全表 UPDATE/DELETE（update.rs:108 / delete.rs:119）、`Scan` 执行器（scan.rs:59）、table_manager.rs:128——大表全表 DML 静默失真，属正确性红线（T6 既有测试表小未覆盖）。

### 用户决策（2026-10-13，Gate 1 集中裁定）

- **D-1 P2-10/11 维持现状并锁定**：多语句 `--format json` 输出为逐行 JSONL、DDL 输出 `affected_rows:0` 均维持现状，e2e 测试锁定 + 文档声明（T7），不改行为。
- **D-2 ISS06 走 DDL 语句边界落盘（预防）**：DDL 执行成功路径上将 catalog 有序落盘（表行最后落盘 = commit point），崩溃窗口从源头关闭；K05 恢复硬失败语义保持；存量已砖库不提供解救（v0.2.0 暴露面小，删库重建）；`check` 指引文案不改。
- **D-3 新缺陷处置「相邻顺带修 + 扩围报批」**：与已知缺陷同代码面/相邻的小缺陷（B1/B2 即此类——与 ISS05 同 btree 故障域）直接在本 change 修复；需要新任务/新设计或超出验证目标的报告用户批准后并入；其余登记 Issue 候选不落账。
- **D-4（2026-10-14）**：第 004 轮遗留的 `rtsql-docs/SKILL.md` 五条陈述，与第 003 轮的 `get_subquery_first_column` 索引扫描误报，一起放进下一轮，不在第 004 轮补。用户原话：「这个issue和当前cyc的遗留工作共同规划到下一个iter」。

## What Changes

工作面划分：

- **验证面（T1）**：`tests/e2e_test.rs` 填充为旅程级 e2e 套件（真实 CLI 入口、退出码/stdout/stderr/磁盘状态断言）；已知缺陷交互场景以 `#[ignore = "ISSxx"]` 标记，由对应修复任务解除；套件纳入稳定基线。
- **代码面（T2–T6）**：R2–R6 的行为修复，全部有 spec delta。
- **文档面（T7）**：README 双语 + rtsql-docs/SKILL.md 同步（e2e 套件说明、P2-10/11 锁定声明、类型名渲染规则、DDL 崩溃窗口关闭声明、已知边界）。

需求基线：

- **R1 e2e 旅程验证面**（T1，测试基建，无 spec delta）：以真实 CLI 入口覆盖用户旅程——全生命周期（建库→DDL→写读矩阵→checkpoint→dump/restore→delete）、导入、崩溃恢复、索引、并发锁、跨库、加密、`:memory:`、错误面、大值、分析子命令、输出面锁定、派生/视图读面、文案面。断言用户实际所见（退出码、输出、磁盘状态）；已知缺陷交互点标 `#[ignore]` 随修复解除；非缺陷面套件全绿后纳入稳定基线（M19 分批纪律内）。
- **R2 二级索引与树结构正确性**（域 `sql-secondary-indexes` MODIFIED ×2）：非唯一二级索引等值查询（IndexScanAll）结果集 SHALL 与 DataScan 严格相等（双向：不重复、不遗漏），任意重复键规模（多叶等值连串、≥2 层树）成立；同值键跨分隔键边界散布形态 SHALL 全部可达（范围下探）；树遍历原语（`IndexManager::scan_all`）SHALL 对任意层树返回全部条目（内部页感知、不依赖叶子链、无 page-0 终止符脆弱性）；叶子链指针 SHALL 在任何叶内重排（shift 重建）后保真；无 WHERE 全表 UPDATE/DELETE 在多层树表上 SHALL 触达全部行（affected_rows = 实际行数）；失败语句（含 KeyNotFound 路径）对索引 SHALL 零副作用；唯一键索引、范围/复合谓词回落、PK 点查既有面零回归。
- **R3 DDL 语句边界持久化与建表原子性**（域 `wal-recovery-replay-integrity` ADDED + `sql-write-surface` ADDED）：DDL 语句（CREATE TABLE/INDEX/VIEW、ALTER ADD COLUMN、import 自动建表、CTAS 建表段）成功返回前，其 catalog 效果 SHALL 有序落盘——被引用页（数据页头、索引根、`__columns` 列行）先落盘，`__tables` 行（或其更新副本）最后落盘为 commit point；此后未 checkpoint 崩溃，读写重开 SHALL 恢复可达（WAL 重放找到表定义），`check`/`schema`/`checkpoint` 正常；建表 catalog 写入中途失败 SHALL 零残留（内存不注册、无可用半表、孤儿清理、同名可重试）；DEFAULT 字面量序列化超出 catalog 列行页内容量 SHALL 计划期点名拒绝（点名 DEFAULT 与约束语义），零副作用。DROP 类 DDL 崩溃窗口维持既有边界（checkpoint-based，文档声明）。
- **R4 派生表与视图内层输出列外层可见**（域 `join-output-shape-resolution` ADDED + `sql-views` MODIFIED）：FROM 派生表与视图通道的内层 SELECT 别名列与表达式（计算）列 SHALL 对外层可引用（内层顶层 Projection 路由在 FROM 派生上下文放行，沿 T5 `building_from_derived` 先例）；三通道（派生表/视图/CTE）行为一致（`sql-views` R3「逐字节一致」声明成真）；内层 DISTINCT 在派生上下文同族放行；IN/EXISTS/标量表达式子查询上下文维持既有抑制（错误面不变）；裸列名形态与 CTE 通道零回归。
- **R5 SemiJoin/AntiJoin 基座全宽承载与排序键**（域 `join-output-shape-resolution` ADDED）：WHERE 带 IN/EXISTS 子查询时，SemiJoin/AntiJoin 基座 SHALL 承载左输入全宽行（对齐 design D10 不变量），排序键计划期校验与 Sort/Projection 裁剪序 SHALL 与无子查询基座一致——`ORDER BY <非输出列>` 可用且语义一致；字面量 IN 谓词下推路径、投影形状、EXISTS 形态、相关子查询形态零回归。
- **R6 CLI 文案与错误面四点**（域 `cli-noninteractive-shell` MODIFIED ×2 + ADDED + `error-reporting` ADDED）：`--help` 文本 SHALL 无内部任务编号/提案标记（用户视角描述）；错误消息前缀 SHALL 单层（`Parse error:` 不再嵌套，断言校准）；schema/dump DDL 列类型 SHALL 渲染标准长名（INTEGER/TEXT/FLOAT/BOOLEAN/DATE/TIMESTAMP——dump→restore 往返保持）；`RTSQL_BUSY_TIMEOUT` 非法值报错 SHALL 点名环境变量（沿 `RTSQL_WAL_CHECKPOINT_BYTES` 代码侧校验先例，flag 优先级与退出码不变）；多语句 JSONL 与 DDL `affected_rows:0` 以文档+测试锁定（D-1）。
- **R7 文档同步**（文档面 T7，无 spec delta）：README 双语 + rtsql-docs/SKILL.md——e2e 套件定位与运行方式、P2-10/11 输出形态声明、类型名渲染规则、DDL 崩溃窗口关闭与剩余边界（drop 类 DDL、加密 WAL 明文窗口）、索引正确性声明成真。2026-10-14 起，SKILL.md 未落盘的部分由 T9 完成。
- **R8 IN 子查询索引扫描取首列**（域 `in-subquery-join-rejection` ADDED，T8）：`IN` 右支为 `IndexScan` 或 `IndexScanAll` 时返回该计划的第一个输出列，不再误报 `Subquery returns multiple columns`。

## 场景草图

### e2e 旅程（R1，T1）

| 场景 | 前置状态 | 动作 | 可观察结果及失败边界 |
|---|---|---|---|
| E1 全生命周期 | 空 RTSQL_HOME | new → 建表（约束+DEFAULT+重复列名拒绝）→ 写矩阵（子集 INSERT/UPSERT 三形态/UPDATE 表达式多列/全表 UPDATE-DELETE/带 WHERE）→ 读矩阵（JOIN 三型/聚合/CTE/视图/集合/DISTINCT/别名排序/派生表/IN 子查询）→ checkpoint → dump → restore 新库 → 数据逐行对照 → delete | 每步 exit 0、最终数据一致、delete 逐文件报告 |
| E2 导入 | 空 | CSV 文件 + stdin `-` + 目标表缺失自动建表 + `:memory:` import + 表头冲突拒绝 | 行数与类型正确；冲突 exit 1 零副作用 |
| E3 崩溃恢复 | 已存在表的库写入中途 SIGKILL | 读写重开 → count 一致 → check 干净 | 恢复可达、已提交数据完好 |
| E3b 自动建表即崩溃（ISS06） | import 自动建表 + 少量行 + SIGKILL（未 checkpoint） | 任意开库命令 | 修复前：全命令 exit 1 砖化 → `#[ignore]`，T3 解除后恢复可达 |
| E4 索引旅程（ISS05/B1/B2） | ≥30000 行重复键表 + CREATE INDEX | 等值 COUNT/CISTINCT vs 建索引前基线；无 WHERE UPDATE affected；重开一致性；DROP INDEX | 修复前：计数虚高/漏行、affected 失真 → `#[ignore]`，T2 解除后严格相等 |
| E5 并发锁 | 写会话持锁 | 并发写（短 busy-timeout 等待/超时 exit 4）、只读并发 SELECT、delete 占用拒绝 | 锁矩阵行为与退出码符合语料 |
| E6 跨库 | 两库 | ATTACH → 跨查 → 跨写 → CTAS 物化 → close | 行数一致、单库独立可用 |
| E7 加密 | `--key` 建库 | 全流程 DML/查询/错钥 exit 5/check/dump+restore 带钥 | 加密面行为符合语料 |
| E8 大值 | - | ~1MB 字符串写读 + dump/restore + check | 往返无损、check 干净 |
| E9 分析子命令 | 有数据 | stats/sample/profile × 四格式 | 输出契约符合语料 |
| E10 输出面锁定 | - | 多语句 `;` + `--format` 四态 + DDL affected_rows:0 | JSONL 逐对象、affected_rows:0（D-1 锁定） |
| E11 派生/视图读面（ISS07/ISS09） | people/dept | 内层别名/表达式项三通道引用 + SemiJoin ORDER BY 非输出列 | 修复前：42703/58000 → `#[ignore]`，T4/T5 解除后一致 |
| E12 文案面（ISS10–13） | - | `--help`、错误 SQL 前缀、dump 类型名、`RTSQL_BUSY_TIMEOUT=abc` | 修复前：内部标记/双层前缀/短名/只提 flag → `#[ignore]`，T6 解除 |
| E13 错误面 | - | 未知列/表/不支持语句/DEFAULT 超限（ISS14）/建表失败残留（ISS08） | 退出码+SQLSTATE+零副作用；ISS08/14 部分 `#[ignore]` 随 T3 解除 |

### 缺陷修复（R2–R6，T2–T6）

| 场景 | 前置状态 | 动作 | 可观察结果及失败边界 |
|---|---|---|---|
| S1 多叶等值连串严格相等 | t1：30000 行 n∈1..100 重复键 + idx(n) | `SELECT COUNT(*), COUNT(DISTINCT id) FROM t WHERE n=k` 逐 k 对照建索引前 DataScan 基线 | 双向严格相等（修复前：+22944 虚发 / 476 漏发） |
| S2 边界子树可达 | 同上 | 对漏发条目 `UPDATE t SET n=k' WHERE id=<rid>` | 成功且旧条目删除（修复前：`Key not found`） |
| S3 失败语句索引零副作用 | 同上 | 构造失败 UPDATE（碰撞）后索引计数不变 | 计数不变（修复前：abort 重插改变索引内容） |
| S4 多层树全表 DML | ≥3 层 PK 树表（乱序键） | 无 WHERE UPDATE/DELETE | affected_rows = 行数、全部行触达（修复前：affected≈0 或 KeyNotFound 失败） |
| S5 scan_all 全量 | 同上 | 经 Scan 执行器/内部通道全量枚举 | 全部条目返回（修复前：碎片） |
| S6 叶子链保真 | 乱序插入后的索引树 | 逐叶检查 next 指针 | 链完整（修复前：~90% 断链） |
| S7 DDL 边界崩溃恢复 | import 自动建表 + 未 checkpoint + SIGKILL | 读写重开 → count → check | 恢复可达、数据完好、check exit 0（修复前：全命令 exit 1） |
| S8 DDL 各形态崩溃 | CREATE TABLE/INDEX/VIEW、ALTER ADD、CTAS、import 各自成功后立即崩溃 | 重开 | 全部可达（表/索引/视图/列生效） |
| S9 建表失败零残留 | 超大 DEFAULT（>4KB） | CREATE TABLE → schema → INSERT → 同名重建 | 计划期点名 DEFAULT 拒绝（修复前：`Page full` + 残留半表 + 重建被拒） |
| S10 派生表内层别名/表达式 | people 3 行 | `SELECT v FROM (SELECT age AS v …) t`；`SELECT tier FROM (SELECT CASE…END AS tier …) t`；视图同型；CTE 对照 | 全部正常（修复前：42703/58000） |
| S11 内层 DISTINCT | 同上 | `FROM (SELECT DISTINCT …)` | 真实去重（修复前：静默不去重） |
| S12 SemiJoin 排序 | people/dept | `SELECT name FROM people WHERE id IN (SELECT did FROM dept) ORDER BY id` | 按 id 排序输出 name（修复前：42703） |
| S13 文案四点 | - | `--help`；`schema t.db`（非法 SQL）；dump INTEGER 表；`RTSQL_BUSY_TIMEOUT=abc` | 无内部标记；单层前缀；`INTEGER` 渲染；报错点名 env（各修复前形态对照） |

## Impact

- **代码面**：`src/storage/btree/{index_manager,btree,node}.rs`（T2）、`src/storage/data/table_manager.rs` + `src/storage/buffer_pool.rs` + `src/storage/catalog.rs` + 各 DDL 执行器接线（T3）、`src/parser/planner/{query,subquery}.rs` + `src/executor/{semi_join,anti_join}.rs` + 消费面（T4/T5）、`src/cli/{mod,lifecycle}.rs` + `src/pipeline.rs` + `src/parser/error.rs`（T6）。
- **测试面**：新 e2e 套件 + secondary_index/full_table_dml/新崩溃恢复矩阵/derived_scan_alias/view/subquery/cli_test 扩展。
- **不兼容面**：dump DDL 类型名渲染变化（INT→INTEGER 等，往返可达不受影响，既有 dump 断言校准）；错误前缀形态变化（既有断言校准）。均属文案面。
- **涉及语料域**：`sql-secondary-indexes`、`sql-write-surface`、`wal-recovery-replay-integrity`、`join-output-shape-resolution`、`sql-views`、`cli-noninteractive-shell`、`error-reporting`（7 域 delta）。
