# Design — e2e 旅程验证面建设与缺陷清偿

调查基线：HEAD `44d04d9`（工作区含 2026-10-12 批未提交产物，无产品代码差异）；全量 1929 tests / 0 failures / 2 ignored（2026-10-06 采信）。ISS05 机制结论来自本会话一次性探针（`tests/zz_probe_scratch.rs`，全保真 Database 场景 + 孤立 IndexManager 场景双轨，跑毕即删；关键数字：30000 行回填 30000 条全在、search_all 漏发 476/重复发射 23420、`scan_all` 返回 190、`delete_exact` 对在树条目 KeyNotFound、全表 UPDATE 以 `Abort failed: Key not found` 失败）。

## D1：e2e 旅程套件形态（T1 / R1）

- **落位**：填充 `tests/e2e_test.rs`（现为空壳）。单文件多测试函数，一旅程一函数。
- **夹具**：复用 `cli_test.rs` 既有模式（`CARGO_BIN_EXE_rtsql` 子进程 + 独立 TempDir + `RTSQL_HOME` 指向 + 60s 兜底 kill），即 R31 runbook 配方；不新建夹具抽象层（AGENTS 验证纪律——不为验证新增判定层）。
- **断言面**：退出码、stdout/stderr 文本、磁盘状态（库文件存在性/`check` 输出/dump 内容）——AGENTS「端到端验证单独成类」定义。
- **已知缺陷交互标记**：命中 ISS05–ISS14 的断言点以 `#[ignore = "ISSxx 待 Tn 修复"]` 标注（项目先例：join_test T3-S11 ignore → 解除转 GREEN）；修复任务的首步是解除对应 ignore 并观察 RED。
- **稳定基线**：纳入常规 `cargo test`（全量走 M19 分批纪律）；旅程规模刻意受控（单旅程 ≤ 数十次子进程调用），预期套件总时长分钟级。
- **新暴露缺陷处置**（D-3）：套件建设期发现的新缺陷——与已知缺陷同面/相邻的小缺陷 Act 直接顺带修（Act Response 记录）；需新任务/新设计的报 Plan 转用户裁定；其余 Issue 候选只报告不落账。

## D2：等值下探正确算法（T2 / R2 机制 A+B 统一关闭）

**结构事实**（探针 + 源码闭合）：本 B-tree 的分裂以 `sorted[mid]` 为 middle_key，右半包含一切 `>= middle` 的键——**同值键 K 可散布在「分隔键 < K」的子树**（探针实证形态：分隔键 `(2,252) | (3,31)`，叶 252 尾部持 v=3 条目）。叶间全局有序且同值键在叶序上连续，但**内部节点分隔键不构成 K 的唯一定界**。

**正确下探集**（替代 `search_all_from_page_async` 现有「等值分隔键双侧收集 + 有等值即跳过路由」逻辑，index_manager.rs:192-257）：在内部节点上，记 `lo` = 首个 `sep >= K` 的下标（二分下界）、`hi` = 末个 `sep <= K` 的下标。需要访问的子树为：

```
children = { child(j) : j ∈ [max(0, lo-1), hi] } ∪ (lo == 0 ? { leftmost } : ∅)
```

依据：子树不变量为 `child(j) 键域 ⊆ [sep_j, sep_{j+1}]`（**闭区间**，两端均可取等——分裂把等于 middle 的键留在左半尾部）。单值分隔键（`lo == hi` 且相等）退化为该分隔键左右两子树（与现意图一致但无重复收集）；K 严格介于两分隔键之间时退化为单子树（与常规路由等价）；K 高于全部分隔键时为最右子树。每个子树恰好访问一次——机制 A（重复发射）与机制 B（漏访）同时关闭。

**同源消费面修正**：

- `BTree::delete_exact`（及非唯一键的一切单径下探原语）SHALL 采用同一子树集合——ISS05 事件记录的 `UPDATE … Key not found` 即其单径路由漏访边界子树；修后对在树条目必达。
- `IndexManager::search`（唯一索引点查）与 `insert` 路由**不动**：唯一键无跨界形态（分隔键相等于 K 即唯一定位）；插入落在路由子树内即保持叶内有序与全局叶序，无正确性问题。
- 同步版 `BTree::search_all`（btree.rs:1070-1131，首等值分隔键即 break 的漏行变体）：当前无生产调用者——**维持不动**，作为已知死代码记录（不清理无关代码；修复任务不扩散）。

**2026-10-13 审计修正（Iteration 001 Cycle 000 阻塞之后）**：上面的闭区间是搜索侧的消费条件，不是当前插入路径已经保持的结构。`InternalNode::insert_separator`（node.rs:733）按键找插入位，页满重建时 `shift_slots_right_internal` 只按键排序；新的等值分隔键在稳定排序里排到整段等值键的末尾，不挨着被分裂的那个子页。后一次 `dup00` 分裂因此会插到已经含有 `dup01` 的边界叶右边。边界叶的上界分隔键仍是 `dup00`，叶内却有更大的键。

本会话对 Cycle 000 的失败夹具（5 个键 × 400 条乱序插入，探针测完即删）读到：`scan_all` 中 `dup01` 为 400，`search_all` 为 395。唯一一处上界破坏是根页 child[6]（页 32）：左右分隔键都是 `dup00`，叶内 42 条 `dup00` + 5 条 `dup01`；紧接着的 sep[7] 才是 `dup01`。下界没有破坏（没有子树的最小键小于前一个分隔键）。`equality_candidate_children` 从首个 `>= dup01` 的分隔键起算，不进入这片叶。继续放宽下探公式盖不住「更大的键落在更小分隔键左边」；要修的是分隔键的落位。

落位规则：新分隔键插在被分裂子页的紧右边。左半保留原页号，右半是新页。左半的键 `<=` 上推键，右半的键 `>=` 上推键，且右半原是旧子页键集的子集，所以仍被旧的下一分隔键封顶。等值分隔键不再按到达顺序挤到等值串末尾。内部节点分裂后，未插入的分隔键进入**仍持有被分裂子页**的那一半，同样按子页拼接，不用 `separator >= promoted` 选边（等值时该比较会把分隔键送进不含该子页的一半）。`update_parent_separator` 只改原槽位上的键和右子页，不拆掉再按键排序插回。`find_child_page_id_binary` 的等值向右路由保持不动。闭区间下探公式保持 Cycle 000 已落地的实现，不另改消费侧。

**验收判据**（沿 ISS05 台账）：等值扫描结果集与 DataScan 严格相等（双向），任意重复键规模；失败语句（含 UPDATE 探针）对索引零副作用。

## D3：叶子链保真与树遍历原语（T2 / R2 B1+B2）

- **B1**：`LeafNode::shift_slots_right`（node.rs:164-197）整页重建 SHALL 恢复 `next_leaf_page_id`（重建前读出、`insert_simple` 后经 `set_next_leaf_page_id` 或直接头写回）并 `reload_header()`——对齐 `shift_slots_right_internal`（:769-799）既有的 leftmost 保真先例。`LeafNode::split` 已 `reload_header` ✓。
- **B2**：`IndexManager::scan_all` 重写为**树遍历式**——从根起迭代下探（显式栈，内部页访问 leftmost + 全部分隔键子页、去重、叶页收集条目），镜像探针验证过的算法；**不依赖叶子链**（存量库断链数据因此天然兼容）、内部页感知（杜绝「内部页当叶读」的静默丢弃）、不以 page 0 为终止符（终止条件为栈空/已访问集）。消费面（update.rs:108 / delete.rs:119 / scan.rs:59 / table_manager.rs:128）零改动继承。
- **page-0 终止符脆弱性**：现行代码以 `PageId(0)` 为「无」哨兵，依赖「真实库中 catalog 先占 page 0」的分配顺序巧合。树遍历式 scan_all 消除该依赖；其余链上消费（如同步 `BTree::scan_all` 的下探段）维持现状（真实库分配顺序保证不触发，记录为已知边界，不改格式）。

## D4：DDL 语句边界有序落盘（T3 / R3 ISS06）

- **落盘时机**：DDL 语句成功路径上（执行器返回 AffectedRows 前）执行有序冲刷——用户视角「DDL 成功 = 表定义已持久」。不引入 WAL DDL 记录（MS18 设计事实「DDL 不写 WAL」维持）；K05 恢复硬失败语义不变；存量砖库不解救（D-2）。
- **有序性不变量**：`__tables` 行（或其更新副本所在页）为 **commit point，最后落盘**；其引用的一切页（数据页头、索引根页、`__columns` 列行页）先落盘。崩溃在冲刷中段时：引用页已至而表行未至 → 表定义不存在 → 该 DDL 视同未发生（WAL 若已有该表行记录则仍砖——但窗口从「DDL 后任意时刻」缩至「冲刷序列内部 µs 级」且**顺序保证不产生零列表/半表静默损坏**）；表行已至则引用页必已至 → 一致。
- **实现形态**：`BufferPool` 增两个原语——`flush_pages(&[PageId])`（按给定序冲刷指定脏页）与 `flush_all_except(&HashSet<PageId>)`（冲刷其余脏页；沿 M17 collect-then-write 约束，不持锁跨 await）。DDL 冲刷编排：`flush_all_except(commit_pages)` → `flush_pages(commit_pages)`。commit_pages 的识别在各 DDL 通道的 catalog 写入点完成（TableManager/Catalog 返回或记录新写入/更新的表行页号）。
- **接线面**：`create_table_with_constraints`（覆盖 DDL 执行器、CTAS 建表段、import 自动建表）、`register_secondary_index`（CREATE INDEX——commit point 为 catalog 新副本页）、ALTER ADD COLUMN（列行先、表行 count 更新最后）、`insert_view_row`（CREATE VIEW）。**DROP 类 DDL（DROP TABLE/INDEX/VIEW、TRUNCATE、ALTER DROP COLUMN）不接线**——其崩溃一致性维持 checkpoint-based 既有边界（语料既有语义，文档声明，T7）。
- **成本**：DDL 语句每条一次全池冲刷；DDL 低频，one-shot 单写者模型下无并发冲突；对照 CTAS 既有「建表→flush_all→插入」先例。

## D5：建表 catalog 写序重排与失败清理（T3 / R3 ISS08）

- **写序**：`create_table_with_constraints` 的 catalog 持久段重排为「列行全部写入 → 表行最后写入」——表行存在性即建表 commit point（与 D4 有序落盘同一不变量，二者共享实现）。现行为表行先写、列行后写（eval 实证首列成功第二列页满 → 残留半表）。
- **失败清理**：列行写入中途失败 → 内存不注册（`tables.insert` 移至持久段成功后或失败即回退）+ best-effort 清理已写列行（复用 delete_table 的列行清理通道）+ 已分配数据页/索引页释放（沿 create_index 失败清理先例）。结果：失败零残留、同名可重试、无可用半表。
- **ALTER ADD COLUMN 同理**：列行成功 + 表行 count 更新成功方算生效；列行失败零可见变化（孤儿列行若残留 SHALL 不可见——`scan_columns` 按表行 `column_count` 或表存在性裁剪，Act 按实际枚举语义落实并以测试锁定）。
- **与 T2 的边界**：T3 触碰 table_manager/catalog 的写序与冲刷；T2 触碰 btree 下探原语——不同文件域，无交叉。

## D6：DEFAULT 超限计划期预检（T3 / R3 ISS14）

- `build_create_table`（ddl_dml.rs）在既有 `validate_default_value`（类型校验单点）旁增加**序列化尺寸预检**：DEFAULT 字面量经 catalog 列行序列化形态（`[has_default][tag][payload]`，String 载荷含 2B/u32 长度前缀）超出单页列行可用容量（与 MS27 T8 既有 ~4KB 文档边界同源，具体常数以 catalog 列行写入路径实际容量为准，Act 校准断言）时计划期拒绝，文案点名「DEFAULT 值过大超出 catalog 页容量」与列名；零副作用。
- 命中该预检后 ISS08 的执行期页满路径对该形态不再可达（预检先行拒绝）；执行期残余路径仍由 D5 兜底（防御纵深）。
- ALTER ADD COLUMN 同款预检（`build_alter_table` AddColumn 臂）。

## D7：building_from_derived 放行模式扩展（T4 / R4 ISS07）

- 沿 T5（change 2026-10-04-external-review-gaps D5）先例：`building_subquery && !building_from_derived` 为「维持抑制」判式。本次将该模式扩展到全部「顶层特性路由」门：
  - query.rs:1572/1585/1614/1626（SELECT 项嵌入分类——聚合路由与表达式项顶层 Projection 路由）与 :1677（同族门）；
  - query.rs:1365（DISTINCT——现派生上下文静默不去重，同族缺陷顺带修）。
- **视图通道**：视图展开（query.rs:1296-1303 区域）置 `building_from_derived = true`（与 FROM 派生臂同型上下文）——`sql-views` R3「CTE 通道展开与内联派生表逐字节一致」由矛盾转为成真。
- **维持抑制**：IN/EXISTS/标量表达式子查询上下文（subquery.rs:41/95 显式复位 `building_from_derived = false`）——SemiJoin/SubqueryEval/ScalarSubqueryFilter 的内层形状契约不变，错误面逐字节不变。
- CTE 通道（`consume_with` 顶层上下文构建）零改动——本就正确，作为对照面。
- 内层 Projection 节点经 `get_plan_output_columns` → DerivedScan 注册列清单自动携带别名/表达式输出名，外层解析零额外改动。

## D8：SemiJoin/AntiJoin 全宽承载（T5 / R5 ISS09）

- **对齐 D10 不变量**（与其他基座一致）：SemiJoin/AntiJoin 的 `output_columns` SHALL 为左输入基表全列（`subquery.rs:63-64/110-111` 的 `build_output_columns_for_table(table_name, projection_columns)` 改传全列），`build_output_row` 直通左行不裁剪；SELECT 清单裁剪与排序由下游承载——**沿 T3（JOIN 基座 ORDER BY）的 `sort_projection` 臂先例**（ORDER BY 需要非输出列时承载全宽、Sort 后裁剪），Act 以该先例实现为模板。
- **消费面**：`get_plan_output_columns` SemiJoin/AntiJoin 臂、`resolve_projection_indices`、排序键计划期校验（query.rs:2703-2730 的可解析集）随之按全宽基座语义对齐；字面量 IN（谓词下推、行本就全宽）与 EXISTS/相关形态零回归。
- **风险**：投影/排序消费面改动波及既有 subquery/cli 用例形状断言——允许按新语义白名单校准（先例：T6/T7 的 cli_test 校准），校准点逐条记录。

## D9：文案批四点方案（T6 / R6 ISS10–13）

- **ISS10**：cli/mod.rs:107-109（busy_timeout）与 :112-114（readonly）doc comment 改写为用户视角描述；内部编号/提案注记移入普通 `//` 注释（不可见）。纯文案，帮助文本无快照测试——新增断言用例锁定不含 `MS2`/`proposal` 字样。
- **ISS11**：前缀单层原则——`parse_stage`（pipeline.rs:113）与 `PlanError::ParseError` Display（error.rs:74）两层只保留一层。定稿：**parse_stage 去前缀**（返回裸 `sqlparser` 错误文本），由错误分类漏斗点统一赋予单层 `Parse error:` 前缀与 42601（该漏斗已存在——`PARSE_SQLSTATE` 注释所述机制）；`PlanError::ParseError` Display 保持（计划阶段产出的解析类错误唯一前缀来源）。pipeline.rs:2761 既有断言与一切锁双层形态的测试同步校准。修复后全错误面恰一层 `Parse error:`。
- **ISS12**：`column_type_sql`（lifecycle.rs:1025-1034）映射改标准长名：`INT→INTEGER`、`STRING→TEXT`、`BOOL→BOOLEAN`、`FLOAT/DATE/TIMESTAMP` 不变。dump→restore 往返（解析器接受长名）与多代 dump/restore 幂等保持；既有 dump 文本断言校准。
- **ISS13**：`busy_timeout` 去掉 clap `env` 绑定，改代码侧读取（沿 `RTSQL_WAL_CHECKPOINT_BYTES` 先例，cli/mod.rs:376+ 形态）：flag 显式优先；env 存在且非法 → 开库前 exit 2 且文案点名 `RTSQL_BUSY_TIMEOUT` 与实际值；`RTSQL_KEY`/`RTSQL_WAL_CHECKPOINT_BYTES` 同族面核对（如有同病一并顺带——同类缺陷同面修复，Act 记录）。

## D10：P2-10/11 锁定（D-1 决策落地）

- 多语句 `--format json` = **JSONL**（逐语句一 JSON 对象、fail-fast 序号定位）——e2e 断言锁定 + 文档声明（T7）；DDL `affected_rows:0` 同。均不改行为。
- 语料承载：`cli-noninteractive-shell` ADDED「多语句 JSONL 与 DDL affected_rows 输出形态」（文档化现状行为，区别于缺陷修复类 Requirement）。

## D11：IN 子查询索引扫描取首列（T8）

`get_subquery_first_column` 只被 `IN` 右支调用（`subquery.rs:61`）。`EXISTS` 不调用它。裸名主键等值把内层计划建成 `IndexScan`（`query.rs:2105-2114`，`columns` 在无 `ORDER BY` 时已是 SELECT 清单）。二级索引等值建成 `IndexScanAll`（`query.rs:2137-2142`，`columns` 恒为基表全列，`projection` 决定输出）。两节点都没有匹配臂，落入 `:702` 的 `_`，报 `Subquery returns multiple columns`。

取列与 `get_plan_output_columns` 对齐：`IndexScan` 用 `columns[0]`（该函数对这个节点不再套投影，避免二次裁剪）；`IndexScanAll` 用 `projected_columns(columns, projection)` 的第一项。空列清单仍报多列。不改索引路由，不给 `Sort`/`Limit`/`Projection` 补臂。

## 已知边界与 Non-goals

- DROP 类 DDL 崩溃窗口（D4）；加密库 WAL/checkpoint 明文滞留（语料既有设计，R32 已取得脏窗口实证样本，不在本 change 范围——Issue 候选随收尾报告）；同步 `BTree::search_all` 死代码不清理；存量砖库/断链索引文件不做迁移修复（预防式修复，新会话起正确）。
- e2e 套件不覆盖性能断言（时延/吞吐归 MS22 域）；不新建验证判定层/身份证据机制（AGENTS 禁令）。
