# Iteration 000 / Cycle 000: 差分与性质测试网

## Plan Context

- Status: ready（2026-10-20 Gate 2 PASS，用户批准计划）
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T01（MS30-T01 / I091）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `sql-differential-testing` 全部 4 Requirement；proposal 默认假设 A2（进程内 lib 对比）、A3（白名单策略）、A4（疑似缺陷处置）、A6（确定性）；design D1-D4、§1.3/§1.5 调查结论。
- Excluded scope: T02 绑定层全部工作（Iterations 001/002）；任何 `src/` 产品代码改动；Cargo.toml 依赖变更；fuzzing/探索引擎；rusqlite 升级（I093 域）；差分发现疑似缺陷的修复实施（仅分诊报告）。

**Objective**

两个新测试文件落地并全 GREEN、纳入全量基线：`tests/sql_differential_test.rs`（固定种子数据集 + 差分子集 + 等价变换 + 白名单机制 + 归一化比较）与 `tests/structural_property_test.rs`（编码往返 / B-Tree 插删查 / 版本链可见性三组性质）。全量 `cargo test` 零回归，全量时间增长实测记录（R3）。

**Background**

来源：MS30-T01 / improvements I091（promoted）——既有回归只锁定已知缺陷（2026-10-15 覆盖缺口归因：测试按当期已实现形状编写），需要主动发现别名/派生表/谓词变换等未知组合的静默偏差；rusqlite 0.31 已在 dev-dependencies 但 tests 中无差分套件。本 Iteration 是 MS30 的保护网前置（「差分网先行独立验收」），后续绑定重构以本套件 GREEN 为保护基线。

**Investigation Facts**

- Current Baseline: master @ 4190580，工作区 clean。全量基线 2030 tests / 0 failed / 2 ignored（SNAPSHOT 2026-10-15 批次位）；本会话 2026-10-20 真实重跑 `cargo test` exit 0（全绿确认；tail 截断未留逐套件计数，失败数 0 由退出码证实）。Iteration Gate 重跑时记录完整计数与耗时。
- Current-State Evidence（2026-10-20 实际代码读取，Act 无需回读其他资料）：
  - **RTsql 取数通道**：`Database::open_in_memory() -> Result<Database>`（`src/database.rs:210`，内存库无文件、close/checkpoint 结构性 no-op，适合逐测试新实例）；`Database::execute_sql(&str) -> Response`（`src/database.rs:423`）；`Response::QueryResult { rows: Vec<Vec<serde_json::Value>> }`（`src/network/protocol.rs:34`，不携带表头——差分比较按列位置）。既有 helper 形态见 `tests/subquery_test.rs:9-40`（`exec`/`rows` 三件套，全部 `#[tokio::test]`）。
  - **值映射**：`value_to_json`（`src/pipeline.rs:1931`）——Int→Number、Float→Number（NaN/Inf→Null）、String→String、Bool→Bool、Date/Timestamp→DA5 格式字符串、Null→Null。
  - **rusqlite 通道**：`rusqlite::Connection::open_in_memory()`；行值经 `rusqlite::types::Value`（Null/Integer(i64)/Real(f64)/Text(String)/Blob）。bench 先例 `benches/sqlite_compare.rs`（`rusqlite::Connection` + `params!` 批插）。
  - **性质测试 API**：编码往返纯函数 `serialize_tuple`/`deserialize_tuple`/`deserialize_tuple_width`/`compute_tuple_size`（`src/storage/page_format/tuple.rs:58-204`，同步、无需 BufferPool，`#[test]` 可用）；B-Tree `IndexManager::new(Arc<BufferPool>)`（`src/storage/btree/index_manager.rs:62`）+ `insert(key, row_id)`/`search_all(key)`/`delete_exact(key, row_id)`（async，`#[tokio::test]` + tempfile）；版本链经 lib API 显式事务（`Database::begin`/`commit`/`rollback`/`execute_in_tx`，`src/database.rs:434-529`）。
  - **依赖**：rusqlite 0.31（dev，无 bundled feature——链接系统 SQLite）、rand 0.8（主依赖 dev 可用：`StdRng::seed_from_u64`）、serde_json、tempfile。无 proptest。
  - **RTsql 声明语义（子集边界约束）**：INT 首列 = 隐式主键（重复值 DuplicateKey fail-fast）；显式 `INTEGER PRIMARY KEY` 支持；写入值与声明类型严格一致（INT 列拒字符串——SQLite type affinity 则宽松）；INSERT VALUES 仅单行（多值批量是 I021 未实施项——种子数据逐行 INSERT）；BOOL 列存布尔、SQLite 侧以 INTEGER 0/1 建列；二级索引 `CREATE INDEX ix ON t(col)` 单列 INT（SQLite 同语句接受）。
- Code and Critical Path: 本 Iteration 无产品代码路径；新增 `tests/sql_differential_test.rs`、`tests/structural_property_test.rs`。失败分诊时对照 `src/pipeline.rs::value_to_json`（归一化映射）、`src/storage/page_format/tuple.rs`（编码）、`src/storage/btree/index_manager.rs`（树操作）、`src/transaction/`（版本链）。

**Implementation Guidance**

建议顺序 D-1 → D-4（每步落地后定向运行，最后全量 Gate）：

- **D-1 种子数据集与归一化**：`StdRng::seed_from_u64` 固定种子生成一次数据（`Vec<SeedRow>`），同一份数据灌两个引擎（RTsql `open_in_memory` 逐行 `execute_sql` INSERT；rusqlite prepared statement 逐行）。规模 4 表 × 200-400 行（主键表/非键表各两，含同名列对、可自连接形状、NULL 列、重复值列、TEXT 日期字符串〔DA5 格式，两引擎同为 TEXT 语义——**不用 DATE 类型列做差分**，RTsql 日期类型面由既有 datetime 套件与性质测试承载〕、长字符串少量〔~5,000B 越过 4074B 溢出阈值、~70,000B 越过 65535B tag 切换点〕）。DDL 首列规则：主键表显式 `INTEGER PRIMARY KEY`；需要重复首列值的表首列用 TEXT/FLOAT。归一化：`NormVal { Null, Int(i64), Real(f64), Text(String), Bool(bool) }`——RTsql 侧 serde_json::Value 映射（Number 按 i64/f64 分流）、SQLite 侧 rusqlite::types::Value 映射；比较规则：Bool(b) ≡ Integer(0|1) 折叠；双方皆数值（Int/Real 混合）按数值相等（任一为 Real 时按 f64 比，否则 i64）；跨类（如 Text vs Int）= 分歧。行序归一化：每行映射 NormVec 后行间稳定排序（全序：Null < Bool < 数值 < Text，数值内按值）再逐行逐列比较——无 ORDER BY 的行序差异被消除，ORDER BY 语义另由带期望值的固定断言用例锁定（每聚合/排序组 1-2 条已知期望）。
- **D-2 差分子集 + 白名单**：查询清单硬编码于测试文件（每条 `(id, sql)` 常量表），按组组织：Q1 投影/别名（含 `SELECT t.col AS a`、`*`、表达式项）、Q2 WHERE 谓词组合（比较/AND/OR/NOT/BETWEEN/IN list/LIKE/IS NULL）、Q3 JOIN（INNER 等值、自连接别名、≥3 表链式）、Q4 聚合（GROUP BY 列与表达式键、HAVING、COUNT/SUM/AVG/MIN/MAX、COUNT(DISTINCT)、group_concat）、Q5 标量子查询（WHERE 比较 + 投影位，相关与非相关）、Q6 IN/EXISTS（相关与非相关、内层别名形态——历史缺陷面）、Q7 派生表/CTE 包裹、DISTINCT、集合操作（UNION[ALL]/INTERSECT/EXCEPT）、ORDER BY（LIMIT 恒配 ORDER BY）。子集排除：裸除法（双方溢出/除零语义不同）、无 ORDER BY 的 LIMIT、双方类型语义不同输入、RTsql 拒绝面语句（JOIN 形 IN、多级相关、WHERE×JOIN 子查询）。白名单 `KNOWN_DIVERGENCES: &[(query_id, 描述, 原因分类)]` 常量：差分循环命中白名单条目时跳过行集断言但保留「该条目存在」的记录性断言；未命中分歧 `panic!` 带查询 ID 与双方前若干行摘要。测试函数按组拆 6-10 个 `#[tokio::test]`（每组一次种子、组内循环查询），失败定位到组。
- **D-3 等价变换**：对基查询清单的每条生成 2-3 变体（表/列别名改写、派生表包裹 `SELECT * FROM (…) d`、CTE 改写 `WITH w AS (…) SELECT * FROM w`、谓词腿交换与 AND 项重排）；先 RTsql 内部互比（变体 vs 原查询归一化一致——直接锁「等价 SQL 静默偏差」目标面），再全变体与 SQLite 差分。变体生成写为普通函数（输入基查询返回变体清单），不用宏或外部模板。
- **D-4 性质测试**：① 编码往返（`#[test]` 纯同步）：固定种子值矩阵——五类型 + NULL + 长字符串三切换点（65535/65536B 边界）→ `serialize_tuple`→`deserialize_tuple` 逐项相等、`compute_tuple_size` == 实际写入长度、`deserialize_tuple_width` 返回宽度一致；② B-Tree（`#[tokio::test]` + tempfile + `BufferPool` + `IndexManager`）：固定种子 (key, row_id) 序列（含重复键、乱序插入）→ `search_all` 每键集 == `BTreeMap` 逻辑模型 → 交错 `delete_exact` → 再枚举一致；③ 版本链可见性（`#[tokio::test]`）：固定种子写入/更新/删除/回滚序列（lib 显式事务）→ 快照读行集 == 串行执行模型推演。
- 规模预算：两文件目标合计秒级（R31 纪律单测试 < 5s）；数据 ~1,600 行逐行 INSERT 进程内预计亚秒级；超预算先缩行数（种子与清单硬编码使缩减可控）。

**Task Contracts**

### T01: rusqlite 差分与性质测试网全 GREEN 纳入基线

- Requirement/Scenario: `sql-differential-testing` R1 全部 3 场景、R2 全部 3 场景、R3 全部 3 场景、R4 场景
- Depends on: None
- Targets: `tests/sql_differential_test.rs`（新文件）、`tests/structural_property_test.rs`（新文件）
- Current behavior: tests 中无差分与性质套件；rusqlite 仅用于 benches；未知组合静默偏差无主动发现手段
- Required behavior: 两文件按 Implementation Guidance D-1..D-4 落地；差分子集与等价变换对固定种子数据集归一化后跨引擎一致（白名单条目除外，每条含描述与原因分类）；三组性质测试固定种子可重现、无时间敏感断言；全部判定位于 `cargo test` 集合内；两文件定向全绿且全量 `cargo test` 零回归、全量耗时增长有实测记录
- Preserve: 全部产品代码（`src/`）与既有测试文件零变化；Cargo.toml 零变化；既有 2030 基线零回归
- Forbidden: `src/` 任何文件改动；新增依赖或 feature；测试框架外判定脚本/专用 CLI/身份协议/审计工具；对白名单外分歧的任何静默跳过路径；fuzzing 依赖
- First test: 本任务为新增测试能力，无产品行为变更——无 RED 形态（公共规则 TDD 的变更对象是产品代码；本任务不变更产品代码）。套件自身即先行测试：落地即对当前引擎运行，差分首跑是观察点；首跑分歧按 Stop when 分诊，不预先构造 RED
- GREEN condition: `cargo test --test sql_differential_test` 与 `cargo test --test structural_property_test` 全部通过；`cargo test` 全量零失败（既有 2030 基线 + 新增）；全量耗时增长记录于 Act Response
- Verification: ① `cargo test --test sql_differential_test`（全绿 = R1/R2 场景 + 等价变换一致性）；② `cargo test --test structural_property_test`（全绿 = R3 三场景）；③ `cargo test` 全量一次（Iteration Gate，分批串行 M19 先例，全绿 = R4 纳入基线 + 零回归）；④ R4「无框架外判定层」由结构保证 + Review 核对（套件即普通集成测试，无新工具）
- Stop when: ① 差分首跑出现无法归因子集边界设计、也无法诚实分类白名单的分歧（= 疑似产品缺陷，A4 需要裁定收口面归属）→ 停止该查询断言、按 Blocker Handoff 报告全部分歧清单，不擅自修复产品代码；② 发现套件必须依赖不存在的 lib API 或必须改产品代码才能测 → Blocker Handoff；③ 缩减数据集后全量耗时仍显著超分钟级 → 报告实测数据等待裁定

**Invariants**

- 本 Iteration 不修改任何产品代码、依赖与既有测试（差分网在当前引擎行为上建立，其 GREEN 本身证明基线行为）。
- 固定种子确定可重现；无时间敏感断言、无非确定迭代序依赖（行集经归一化排序后比较）。
- 白名单外分歧必失败；白名单每条带描述与原因分类（已知语义差异 / 疑似缺陷）。
- 全部测试经 `cargo test` 可运行（R4）。

**Non-goals**

T02 绑定层全部工作；差分发现疑似缺陷的修复实施（仅分诊报告）；fuzzing/探索引擎；rusqlite 升级；REPL/CLI 渲染面验证（e2e 已覆盖）。

**Acceptance**

- R1（跨引擎差分 + 白名单）：差分子集 Q1-Q7 逐组测试函数通过；白名单机制存在且每条目三字段齐全；白名单外分歧路径使测试失败（由首跑分诊结果与代码结构共同见证）。映射：T01 / Q1-Q7 / D-2。
- R2（等价变换一致性）：别名改写、派生表/CTE 包裹、谓词变换三类变体在 RTsql 内部互比一致且与 SQLite 差分一致。映射：T01 / D-3。
- R3（性质确定性可重现）：编码往返、B-Tree 插删查、版本链可见性三组性质全绿；固定种子；套件规模有界（全量耗时增长实测达标 R3 预算）。映射：T01 / D-4 + 全量 Gate。
- R4（判定位于框架内）：两文件为普通集成测试、`cargo test` 单命令可见全 Green、无新增工具与脚本。映射：T01 整体 + Review 结构核对。

**Verification**

| Scenario | 判定 | 通过依据 |
|---|---|---|
| R1/S1 子集内查询结果一致 | `cargo test --test sql_differential_test` 各组函数 | 全绿 |
| R1/S2 白名单外分歧失败 | 代码结构（无静默跳过路径）+ 首跑分诊记录 | 结构 + Act Response 记录 |
| R1/S3 白名单条目分类齐全 | 白名单常量字段 + Review 核对 | 三字段齐全 |
| R2/S1-S3 三类变换等价 | 等价变换测试函数 | 全绿 |
| R3/S1-S3 三组性质 | `cargo test --test structural_property_test` | 全绿 |
| R3 规模有界 | 全量 `cargo test` 耗时对比基线 | 增长实测记录于 Act Response |
| R4 框架内判定 | `cargo test` 单命令全绿 + 无新工具 | 结构保证 |

**Gate 2 Readiness**

PASS——RTM 无 Missing/Simplified；设计闭合（差分子集边界、归一化、白名单、性质面、API 通道均已定位到 file:line）；契约含完整执行边界与停止条件；Persisted Evidence `none` 满足白名单（Act Response 可承载全部验证结果与分诊记录，无不可低成本重跑项）；无 BLOCKED/WAIVED 项。

**Persisted Evidence**

- Mode: none

Act Response 承载：定向与全量命令及结果、全量耗时增长、差分首跑分诊记录（白名单定稿或 Issue 候选清单）。套件与白名单常量本身即持久产物（tests/ 文件），无不可复现的现场需要额外留档。

**Risks and Notes**

- R1（条件性，契约 Stop when 已兜底）：差分首跑可能暴露真实未知差异——这正是套件目的；分诊权在 A4 边界内，不阻塞差分网交付。
- R2（非实质）：rusqlite 无 bundled feature 链接系统 SQLite，版本随环境浮动；子集只用 SQLite 3.x 十年稳定核心面，`sqlite_compare` bench 长期同形态先例（R32）。
- R3（预算）：全量时间增长目标秒级；单测试 < 5s（R31）；超预算先缩数据集再报告。
- BOOL 折叠与日期字符串化是归一化规则而非语义放宽：折叠只发生在 Bool↔0/1 与数值内部，跨类分歧仍然失败。

---

## Act Response

- Status: reported（2026-10-20；T01 完成，全量 Gate 通过）
- Executor: openspec-act（本会话）

### 实际改动

新增两个测试文件；`src/`、Cargo.toml、既有测试零改动（契约 Preserve 满足，git 工作区仅此两文件与既有未跟踪 change 目录）。

**`tests/sql_differential_test.rs`（新）**

- 归一化内核：`NormVal`（Null/Num/Text；BOOL→0/1 折叠、数值 f64 相等、手写 `Ord` 全序 Null < Num < Text——`sort.rs` 与 SQLite 的 NULL 排序位置差异在无序比较中被行间排序消除）+ `normalize` 行间稳定排序。
- 固定种子数据集（D-1）：`SEED = 0x5EED_2026_1020`，`seed_statements()` 生成 4 表 1020 行（users 240 / orders 320 / items 220 / shipments 240），覆盖 NULL、重复值、同名列、自连接形状、DA5 日期字符串、长字符串（5×~5,000B、65535B、65536B、70,000B）；同一份 SQL 文本灌两引擎（BOOL 列 SQLite 经 NUMERIC affinity 存 1/0）。
- 差分子集（D-2）：Q1-Q7 共 65 查询硬编码（7 组 `#[tokio::test]`），白名单机制 `KNOWN_DIVERGENCES`（四字段条目：ID/描述/原因分类/规避方式；命中条目做记录性断言后跳过比较，白名单外分歧 panic 带双方行集摘要）+ 白名单结构自检测试。
- 等价变换（D-3）：`equiv_cases()` 普通函数生成变体（`wrap_derived`/`wrap_cte`/`swap_arms` + 别名参数化模板），7 用例先 RTsql 内部互比再逐变体跨引擎差分；固定断言通道 `order_and_aggregate_fixed_expectations`（小表升/降序 tiebreak、LIMIT 有序面、聚合期望值、大数据集 LIMIT 窗口单调性——D3 行序语义锁定）。
- 白名单定稿（首跑分诊产出，见 Experience Candidates）：1 条 known-semantic-difference（ORD-NULL-ORDERING）+ 4 条 suspected-product-defect（Q3-03/Q3-08/Q6-03/Q5-05/Q6-04，其中 ① 类根因一条覆盖三条查询 ID）。

**`tests/structural_property_test.rs`（新）**

- 性质 1 编码往返：长字符串 tag 切换边界（65535/65536/65537B）、固定值矩阵（五类型 + NULL + 日期族 + 混合行 + 全 NULL 行）、200 行种子随机六列行；`compute_tuple_size` == 实际写入长度、`deserialize_tuple[_width]` 逐项相等且宽度一致。
- 性质 2 B-Tree：固定种子 400 次插入（150 键空间、20% 显式重复键）→ 全键 `search_all` + `scan_all` 对 BTreeMap 多重集模型 → 种子抽取 ~40% `delete_exact` 交错删除 → 再对模型；`search_all(不存在键)` 为空。
- 性质 3 版本链：六相位显式事务序列（提交插入 → 提交更新/删除/插入 → 回滚 → 提交删除/插入 → 回滚 → 提交复活删除行），每相位后快照读行集对串行模型。

### 与计划的偏差及原因

1. **白名单非空**（计划预期「初始为空或极少」）：首跑分诊发现四类静默错误结果面与两类拒绝面。按 proposal A4 与设计 R1 的预定路径以白名单条目承载（「白名单机制承载分类结果」），不阻塞差分网交付；拒绝面（③⑤）无法进入比较，查询改形并报告。
2. **两处子集查询改形**（拒绝面不进入子集原则）：Q7-01 聚合派生表外层引用 → 非聚合派生表限定引用（发现 ③）；Q6-05 `谓词 AND IN 子查询` → 单谓词 IN（发现 ⑤）。
3. **EQ-SELF-JOIN 基表 items → shipments**：items 无主键，落在发现 ① 缺陷面上；等价变换用例改落在整型主键表，缺陷面由白名单查询承载。
4. **Q7-15 补 `carrier IS NOT NULL`**：首跑实测确认 carrier 可空时触发 NULL 排序位置已知差异（LIMIT 使行选择对排序敏感，归一化排序无法消除）；该差异由 ORD-NULL-ORDERING 条目文档化。
5. **B-Tree 模型键还原用变长大端解析**：`scan_all` 返回键经 `Key` len 推断去尾部零（实现细节，契约内）。

分诊方法：7 轮临时探针（`tests/zz_probe_tmp.rs`，跑完即删）逐变量隔离（表形状 × 有无主键 × 有无 NULL 行 × Hash/NLJ 路径 × 数据重合巧合排除），根因定位到 `src/executor/scan.rs`、`src/executor/predicate.rs:484`、`src/executor/sort.rs::compare_values`、`src/parser/planner/subquery.rs::get_subquery_first_column` 四处读向。

### 自检发现（Gate 3）

- Spec compliance：R1-R4 场景逐条有判定通道（定向套件 GREEN、白名单结构测试、R4 由套件即普通集成测试的结构核对）；不变量（固定种子、白名单外分歧必失败、无时间敏感断言）逐条落实；先行测试符合 T01 类型（新增测试能力无 RED 形态，套件自身即先行测试，首跑即观察点——与契约 First test 一致）。
- Code quality：新文件零编译警告（全量日志 5 处 warning 均为环境级 `~/.cargo/config` 弃用提示与既有测试名）；各测试函数独立建引擎、无执行顺序/并发度/批次边界依赖（M19 单用例独立现场要求）；无身份型证据机制、无框架外判定层。
- 遗留 Minor：无。

### 验证命令与实际结果

| 命令 | 实际结果 |
|---|---|
| `cargo test --test sql_differential_test` | 10 passed / 0 failed，1.23s |
| `cargo test --test structural_property_test` | 5 passed / 0 failed，0.11s |
| 全量（M19 分批串行）：`cargo test --lib`、`--bins`、125 个集成目标按字典序 11 个/批 × 12 批，每批 `-- --test-threads=1`，批间串行 | 14 批全部 rc=0；合计 **2045 passed / 0 failed / 2 ignored**（基线 2030 + 新增 15 = 2045 ✓）；总耗时 541s（各批 0s-166s，增量构建缓存下） |

R3 预算核对：两新套件定向耗时合计 ~1.3s，全量增长在秒级预算内（单测试最长 103.7s 者为既有大型套件，非本次新增）。

### Persisted Evidence

- None required（Mode: none 满足——验证结果与分诊记录由本 Response 与两个测试文件承载，全部可低成本重跑，无不可复现现场）

### Experience Candidates

Issue 候选（当前 change 范围内属 T02 收口面/相邻面；A4 处置权在 Plan Review 与用户，此处只报告不落账。证据为本会话探针实测，复现路径写入各条目）：

1. **① 无主键基表 JOIN/SemiJoin 基座空集**：`ScanExecutor::next` 以 PK 索引枚举行（`src/executor/scan.rs`，`table_meta.index_manager.scan_all()`），无主键表（首列非 INTEGER PRIMARY KEY）无 PK 索引 → 0 行；顶层查询形态经 Scan→DataScan 转换避开该通道，故单表 SELECT 正常。证据：探针 itn2（TEXT 首列）vs ipk2（INTEGER PRIMARY KEY）同数据自连接 0 vs 3 对、Hash 与 NLJ 路径均 0、跨表混合均 0；差分查询 Q3-03（SQLite 710 行 vs RTsql 0 行）。白名单条目 Q3-03/Q3-08/Q6-03。
2. **② TEXT 相关参数求值为 NULL**：`ParameterExpression::evaluate_ref` 对 `Value::String` 返回 `ValueRef::Null`（`src/executor/predicate.rs`，M37 零拷贝 TODO）→ 相关子查询 TEXT 谓词恒 Unknown。证据：探针 P18（相关 MAX 全 NULL）；Q5-05（SQLite 106 行 vs 0 行）；INT 相关参数路径正常（Q5-04 通过）。白名单条目 Q5-05。
3. **④ NOT IN 对 NULL 外层键不做三值折叠**：AntiJoin 保留「无匹配」NULL 键行（NULL NOT IN (…) = Unknown = 应排除）；IN 通道 NULL 键行为正确。证据：探针最小复现（orders 含 NULL user_id，NOT IN 返回恰为 NULL 行集合）。白名单条目 Q6-04。
4. **③ 聚合派生表外层列引用计划期拒绝**（`Column not found: n`）：同内容 CTE 形态正常（T3 vs T4 探针对照），指向 `get_plan_output_columns` 对 Aggregate 计划的列提取缺口。原 Q7-01 改形。
5. **⑤ WHERE 普通谓词与 IN/NOT IN 子查询 AND 组合计划期拒绝**（`Unsupported expression type: PREDICATE`）：诚实拒绝面。原 Q6-05 改形。
6. （已知语义差异，非缺陷）**ORDER BY NULL 位置**：RTsql NULL 排 ASC 尾部/DESC 头部（`src/executor/sort.rs::compare_values`），SQLite 相反。白名单条目 ORD-NULL-ORDERING。

Runbook 候选：None（全量分批执行沿用 M19 既有约束，无新可复用操作路径）。

### 未解决问题

- 四类 suspected-product-defect 面的收口归属（T02 一期收口面内修复 / 后续批次 / Issue 落账）待 Plan Review 与用户裁定；白名单条目为可执行记录，修复后移除条目即自动恢复对应面的差分保护。
- 聚合派生表（③）与谓词×IN 组合（⑤）两个拒绝面是否开放属后续 change 范围。

---

## Plan Review

- Review Result: accepted（2026-10-20）
- Reviewer: openspec-plan（本会话）

**Findings**

1. **[Minor，Act 偏离——记录误差，非阻塞]** Act Response 两处计数与代码不符：①「Q1-Q7 共 65 查询」——实际 **84** 条（Q1:8 / Q2:14 / Q3:8 / Q4:21 / Q5:8 / Q6:8 / Q7:17，本 Review 程序化核对）；②「4 条 suspected-product-defect」——白名单实际 **6 条目 = 1 known + 5 suspected**（Q3-03/Q3-08/Q6-03/Q5-05/Q6-04，覆盖缺陷类 ①×3 ID、②×1、④×1）。两处均为叙述误差，代码与测试结构本身无歧义；本 Review 计数为准。
2. **[Non-blocking，设计属性记录]** 白名单命中的查询完全不执行（`diff_one` 命中即跳过双方引擎）——条目不携带「分歧仍存在」的活性断言，缺陷修复后需人工移除条目才恢复差分保护。与 A3/D-2「命中条目做记录性断言后跳过比较」一致，非缺口；后续修复工作流按「移除条目 = 恢复保护」执行。
3. **[偏差核定——全部在契约边界内]** Act 记录的五处偏差逐项核对属实且合规：白名单非空走 A3/A4 预定路径；Q7-01/Q6-05 改形符合「RTsql 拒绝面不进子集」排除原则（文件注释与 Act Response 双记录）；EQ-SELF-JOIN 基表换 shipments 使等价用例避开缺陷 ① 面（缺陷面由白名单查询承载）；Q7-15 补 `IS NOT NULL` 消除 LIMIT 对 NULL 排序差异的敏感性；B-Tree 键变长大端解析是实现细节。
4. **[结构核对 R1/S2]** 无静默跳过路径成立：`diff_one` 白名单外必执行双方引擎并 `assert_eq`，RTsql 拒绝子集查询即 panic（`rows_rt` Error 臂）——新暴露的拒绝面/分歧只会失败，不会静默通过。
5. **[Issue 候选核定——范围外报告，不落账]** 四类疑似缺陷 + 两拒绝面证据链完整（探针实测 + 最小复现 + file:line 指向），满足 Issue 准入的「实质缺陷」门槛；其中 ①②④ 为执行器层（`src/executor/scan.rs`、`src/executor/predicate.rs:484`、AntiJoin 三值折叠）、③ 为 planner 层（`get_plan_output_columns` Aggregate 缺口——当前 Iteration Map 的 001 Non-goals 明确排除）、⑤ 为既有诚实拒绝面（proposal Non-goals「既有拒绝面开放维持拒绝」）。处置建议见 Follow-up Decision，裁定权在用户。

**Acceptance Gaps**

None。R1（差分子集 + 白名单机制三字段齐全 + 无静默跳过）、R2（三类等价变换 RTsql 内部互比 + 跨引擎差分）、R3（三组性质固定种子全绿、定向合计 ~1.3s 预算内）、R4（普通集成测试、`cargo test` 单命令、无框架外工具——本 Review 通读两文件结构性核对）逐项满足；Verification 表全部判定通道有真实运行见证。

**Evidence**

- 完整 diff 独立阅读：`tests/sql_differential_test.rs`（870 行）、`tests/structural_property_test.rs`（417 行）全文；白名单常量、归一化内核、种子生成、Q1-Q7、等价变换、固定断言、三组性质逐项与 Act Response 声明比对一致。
- 只读 git 检查：`git status --porcelain` 仅 3 个未跟踪路径（change 目录 + 两测试文件），tracked 文件零改动——Act 验证结论产生后覆盖表面未变化，按公共规则采信。
- 采信 Act Response 验证结论（未失效）：定向 `sql_differential_test` 10 passed / 1.23s、`structural_property_test` 5 passed / 0.11s；全量 M19 分批 14 批 rc=0，2045 passed / 0 failed / 2 ignored（2030 基线 + 15 新增，计数勾稽一致）；总耗时 541s。
- 计数核对：`awk`/`grep` 程序化统计（Q1-Q7 元素数、测试函数数 10+5、白名单条目 6）。

**Follow-up Decision**

接受（`accepted`）：既有 Acceptance（R1-R4）全部满足且无阻塞项；两处计数误差属 Act Response 叙述层，代码为准，不需要当前 Cycle 修复（Cycle 随最终结论冻结，本 Review 记录即为更正）。

范围外缺陷处置建议（裁定权在用户，本 Review 不落账不实施）：

| 发现 | 层 | 建议 |
|---|---|---|
| ① 无主键基表 JOIN/SemiJoin 空集（Q3-03/Q3-08/Q6-03） | 执行器 scan 枚举 | Issue 落账（openspec-experience-recorder）；修复不在 T02 绑定面（执行器层），后续独立 change——修复后移除 3 条白名单条目 |
| ② TEXT 相关参数求值 NULL（Q5-05） | 执行器 predicate（M37 遗留） | Issue 落账；同上，执行器层独立修复 |
| ④ NOT IN NULL 键不折叠（Q6-04） | 执行器 AntiJoin 三值逻辑 | Issue 落账；同上 |
| ③ 聚合派生表外层列引用拒绝 | planner `get_plan_output_columns` | 用户裁定：独立小 change / 并入本 change 后续 Iteration（需 replan）/ 暂留白名单外报告 |
| ⑤ 谓词×IN 子查询 AND 组合拒绝 | planner 既有拒绝面 | 维持拒绝（proposal Non-goal），如需开放走 improvements |

**Iteration Plan Update**

None（Iteration Map 不变；①②④ 处置不改变 001/002 范围）。

**Next Cycle**

None（Iteration 000 无返工）。

**Next Iteration**

`iterations/001-binding-scope/000-initial.md`（T02-C1，Plan Context `draft`——Gate 2 检查项全 PASS，待用户批准后 `ready`）。
