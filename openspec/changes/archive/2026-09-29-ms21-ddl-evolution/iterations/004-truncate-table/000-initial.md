# Iteration 004 / Cycle 000-initial: TRUNCATE TABLE

## Plan Context

- Status: ready
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T4-a…b（`tasks.md` T4 全部；spec `truncate-table` R1–R2）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: 002 的二级索引登记面（三类索引根重置含 secondary：`update_secondary_index_root` + `TableMeta.secondary_indexes`）；MS18 checkpoint 零工作守卫（TRUNCATE 前的 checkpoint 调用——有未决 WAL/脏页时全量执行，零工作时无操作早退，两态皆安全）；drop_table 物理释放通道（`collect_data_pages` + `free_page` best-effort 循环）；本 change Iter003 的 ALTER 先例（DDL 直包执行器 + 执行后 `plan_cache.clear()` + in-tx 0A000 臂 + 穷尽补臂 + `flush_all()` 收尾持久）；design D11。
- Excluded scope: VIEW（005）；多表 TRUNCATE（AST 层不可解析，无运行时拒绝臂）；TRUNCATE 与活跃快照读者的细粒度 MVCC 语义；视图目标拒绝的真实化（005 落地后自然生效）；dump 面变化（无）

**Objective**

`TRUNCATE TABLE <t>` / `TRUNCATE <t>` 物理清空保留定义——数据页经 free-list 释放、head 原地重置、tail 回写、PK/唯一/二级三类索引重建为空树且根写回 catalog；表名/列结构/DEFAULT/NOT NULL/UNIQUE/二级索引登记完整保留；查询空集、schema 查询面不变、新写入正常可用、跨重启持久；显式事务内执行期 0A000 点名拒绝零副作用；空表幂等成功；DELETE/DROP TABLE 既有语义零回归。

**Investigation Facts**

- Current Baseline: revision 0fe6971 + Iter000–003 未提交实施（工作区 30 修改文件 + 5 新文件 + change 目录）；全量 1515 passed / 0 failed / 2 ignored（Iter003 Act Response 分批新鲜结论，Iter003 Plan Review 已采信，覆盖面未变化）；静态门（clippy -D warnings / fmt --check / openspec validate）全绿。
- Current-State Evidence:
  - **语句入口**：`Statement::Truncate { table_name: ObjectName, partitions: Option<Vec<Expr>>, table: bool }`（sqlparser 0.44 `ast/mod.rs:1706`）——单 `table_name` 字段（`TRUNCATE t1, t2` 多表形态**解析期即失败**，无运行时多表臂可写）；无 `cascade` 字段；`partitions`（Hive 特有）`Some` → 点名拒绝；`table` 仅为可选关键字标志（`TRUNCATE TABLE` / `TRUNCATE` 等价，皆接受）。现全仓无任何 `Truncate` 消费（grep 零匹配）→ planner catch-all `UnsupportedStatement`（`mod.rs::build_plan` `_ =>` 臂）。
  - **checkpoint-first（D11 步骤 2）**：`Database::checkpoint()`（`database.rs:270`，pub）——刷脏页 + WAL 截断到位点；此后 WAL 无 pre-TRUNCATE DML 待重放。MS18 零工作守卫：无未决 WAL 且无脏页时整体无操作早退——TRUNCATE 前调用两态皆安全（页释放不依赖 checkpoint 产物）。
  - **物理清除（D11 步骤 3）**：`TableManager::collect_data_pages(head)`（`table_manager.rs:1268`，私有，drop_table 同模块直接复用）+ `BufferPool::free_page`（`buffer_pool.rs:377`）best-effort 循环（drop_table 先例 `table_manager.rs:1270-1280`）；`FileStorage::free_page`（`file_storage.rs:285`）**立即零化磁盘页**（`write_page` 直写，不经缓冲池脏页追踪）；head 页原地重置（`SlottedPage::init`；建表初始化先例 `table_manager.rs:470` 区域）；`Catalog::update_table_tail(name, head)`（`catalog.rs:300`）回写。
  - **索引根重置（D11 步骤 4）**：PK 树 `Catalog::update_table_root`（`catalog.rs:329`）；唯一树逐 ordinal `update_unique_index_root`（`:357`）；二级树逐 ordinal `update_secondary_index_root`（`:396`）；内存换入先例 `replace_index_manager`（`table_manager.rs:971`）/ 三类整表换入 `replace_recovery_indexes`（`:1014`）；空树构造先例 `src/executor/create_index.rs`（新 `IndexManager` 根页分配）；旧树页释放 `IndexManager::collect_all_pages`（`index_manager.rs:405`，tolerant 变体 `:469`）。
  - **pipeline 接线点（Iter003 ALTER 先例同位）**：`plan_stage_routed` 的 `register_table` 通道（缺失表 42P01）；`extract_all_table_names` 需补 `Statement::Truncate` 目标臂（现 `pipeline.rs:1741` 区域 `_ => Vec::new()`）；`execute_stage_routed` DDL 直包臂（`:322` ALTER 先例）+ 执行后 `plan_cache.clear()`（DDL 不进 cache——`is_cacheable` 仅 Query，`:2047`）；`execute_stage_in_tx_routed` 执行期 0A000 臂（`:828` 区域）；`create_executor_from_plan` panic 臂 + 穷尽补臂（`plan.rs` 枚举、`plan_format.rs` render/describe、`query.rs` 空列臂、`correlated.rs` 叶臂）。
  - **计划期校验复用**：`reject_attached_write`（附加库目标点名拒绝，`build_alter_table` 先例）；`object_name_to_table_name` 归一化；D14 错误面（复用既有 PlanError 变体，不新增）。
  - **目标边界事实**：视图 005 未落地——「目标为视图/系统保留名」现阶段自然表现为非表 42P01（与 ALTER 系统保留名同型，见证用例锁定文案）；「目标非基表」的独立拒绝臂随 005 真实化，本 Iteration 不预建。
- Code and Critical Path: `src/parser/planner/{mod.rs,ddl_dml.rs}`（`build_truncate` 分派 + 拒绝面）、`src/executor/plan.rs`/`plan_format.rs`（`PhysicalPlan::Truncate` 节点 + 渲染/描述穷尽臂）、`src/executor/truncate.rs`（新 `TruncateExecutor`）、`src/storage/data/table_manager.rs`（`truncate_table`：物理清除 + 三类根重置 + TableMeta 换入）、`src/pipeline.rs`（接线四臂 + `extract_all_table_names`）、`src/database.rs`（checkpoint 经执行器调用，签名零变化）；见证 `tests/truncate_test.rs`（新）。

**Implementation Guidance**

顺序 a→b：执行器与物理清除先行——执行序：计划期拒绝（build 层）→ in-tx 执行期拒绝（pipeline 臂，先于执行器任何副作用）→ `Database::checkpoint()`（截断 WAL，封死行级复活通道）→ 数据页尾页 `free_page` 释放 + head 原地重置 + `update_table_tail(head)` → PK/唯一/二级空树构造换入 + 三类 `update_*_root` 写回 catalog → 旧树 `collect_all_pages` best-effort 释放 → **`flush_all()` 收尾**（alter_drop_column 先例同型：语句返回即持久，T4-b 崩溃见证确定性）→ pipeline 臂 `plan_cache.clear()`。核心不变量：TRUNCATE 无行级痕迹（不写墓碑、不写行级 WAL 记录、不触碰版本链——checkpoint 后物理清除纯页级）。崩溃窗口分析写入 `truncate_table`/执行器 doc 注释，分析必须与恢复语义一致（`free_page` 立即零化 vs catalog 写回经缓冲池脏页——见 Risks；Iter003 F1 教训：窗口分析错记为「结构性消除/恰宽」会误导后续维护）。

**Task Contracts**

### T4-a: TRUNCATE 计划臂与执行器

- Requirement/Scenario: `truncate-table` R1 全部 5 场景（清空空集/schema 不变、物理释放与重开持久、in-tx 拒绝、目标边界拒绝、空表幂等）
- Depends on: None（002 的根重置 API 与登记面已在位）
- Targets: `src/parser/planner/mod.rs`（`Statement::Truncate` 分派臂）；`src/parser/planner/ddl_dml.rs::build_truncate`（`partitions` Some 点名拒绝、附加库 `reject_attached_write`、表存在防御臂——register_table 通道先行 42P01）；`src/executor/plan.rs`/`plan_format.rs`/`src/parser/planner/query.rs`/`correlated.rs`/`src/pipeline.rs`（`PhysicalPlan::Truncate` 节点 + 穷尽补臂 + DDL 直包臂〔执行后 `plan_cache.clear()`〕+ in-tx 0A000 臂 + `extract_all_table_names` Truncate 臂）；`src/executor/truncate.rs`（新 `TruncateExecutor`，持 `Arc<Database>` 同 `AlterTableExecutor` 形态）；`src/storage/data/table_manager.rs::truncate_table`（checkpoint 由执行器先行调用 → 尾页释放 + head 重置 + tail 回写 → 三类空树换入 + 根写回 → 旧树 best-effort 释放 → `flush_all()`）
- Current behavior: `TRUNCATE` 一律 catch-all `UnsupportedStatement`（RED 基线：`Unsupported statement type: TRUNCATE`）
- Required behavior: 清空后 `SELECT COUNT(*)` 为 0、二级索引列等值查询空集、`list`/`schema` 面表与列定义不变、新 INSERT 正常且新行可经 PK 点查与二级索引列查询可达；重开持久行不复活；显式事务内执行期 0A000 零副作用、会话事务状态不变；`TRUNCATE nope` 与系统保留名 42P01 点名拒绝、`partitions` 形态点名拒绝；空表 TRUNCATE 成功且二次 TRUNCATE 幂等成功
- Preserve: drop_table 物理释放通道与 free-list 语义零改动（新方法复用同型，不改 `drop_table` 本体）；DELETE 的 MVCC/墓碑语义零触碰；`Database::checkpoint()` pub 签名与九步/零工作两路径零变化；`update_table_tail`/`update_table_root`/`update_unique_index_root`/`update_secondary_index_root` 既有 API 逐字节复用；WAL 记录格式零变化（TRUNCATE 不引入新记录形态）
- Forbidden: 行级 DELETE 重写方案（物理语义，D11 已否决复活路径）；TRUNCATE 写行级 WAL；多表/视图局部支持；为清空引入身份型标记
- Test witness: `tests/truncate_test.rs` 新增 R1 五场景（行为面先 RED：现 catch-all）
- GREEN condition: 新用例绿 + `drop_table_free`/`checkpoint_skip`/`cli_test` schema 面既有矩阵零修改通过
- Verification: `cargo test --test truncate_test --test drop_table_free_test --test checkpoint_skip_test -- --test-threads=1`（0 failed）
- Stop when: checkpoint-first 与页释放的崩溃窗口出现 D11/Risks 分析之外的不可读形态——返回 Plan

### T4-b: 恢复语义见证与零回归

- Requirement/Scenario: R1-S2 崩溃面 + R2 既有语义零回归
- Depends on: T4-a
- Targets: `tests/truncate_test.rs`（崩溃见证用例）
- Current behavior: 无 TRUNCATE 恢复见证（语句不可达）
- Required behavior: TRUNCATE → `flush_all()` + `wal_buffer.shutdown()` + drop 不 close（模拟语句后崩溃，ALTER 崩溃见证同型）→ 重开表存在且空、行不复活、二级索引空树可用（重开后 INSERT 新行可查）；既有 DELETE/DROP TABLE/墓碑可见性矩阵零回归
- Preserve: 恢复重放与 `redo_count == 0` 路径零变化（TRUNCATE 不引入新 WAL 记录形态，checkpoint 位点语义零触碰）
- Forbidden: 为见证新增专用工具或判定层
- Test witness: 崩溃见证用例（随 T4-a 分派骨架到位后转绿）；`mvcc_tombstone_visibility_test`/`wal_recovery_large_test` 零回归
- GREEN condition: 新用例绿 + 既有恢复矩阵零修改通过
- Verification: `cargo test --test truncate_test --test mvcc_tombstone_visibility_test --test wal_recovery_large_test -- --test-threads=1`（0 failed）+ 全量分批零回归
- Stop when: None

**Invariants**

- TRUNCATE 全程无行级痕迹：无墓碑、无行级 WAL、无版本链触碰；checkpoint 后物理清除纯页级。
- 表定义/列/DEFAULT/NOT NULL/UNIQUE/二级索引登记完整保留；仅数据面与三类索引内容清空（空索引非无索引）。
- 崩溃窗口全部为「DDL 部分状态」诚实化同类；doc 注释窗口分析与恢复语义一致（`free_page` 立即零化磁盘页、catalog/tail/根写回经缓冲池脏页至 `flush_all()` 才持久）。
- 拒绝面全部点名零副作用（D14，不新增 PlanError 变体）；显式事务内执行期 0A000（CTAS/CREATE INDEX/ALTER 先例同型）。

**Non-goals**

- 多表 TRUNCATE（AST 不可解析，无运行时臂）；TRUNCATE 与活跃快照读者的细粒度 MVCC 语义；视图目标拒绝的真实化（005）；dump/restore 面变化；AUTOINCREMENT/序列重置语义（本库无此概念）。

**Acceptance**

1. `truncate-table` R1–R2 全部场景有可观察见证（清空空集/schema 不变/新写入可用、物理释放与重开持久、in-tx 拒绝、目标边界拒绝、空表幂等、崩溃不复活）。
2. RTM：

| Requirement | Task | Test Witness | Status |
|---|---|---|---|
| truncate › R1 物理清空保留定义（5 场景） | T4-a/b | `truncate_test` 五场景 + 崩溃见证 | Covered |
| truncate › R2 既有语义零回归 | T4-b | 全量分批 0 failed + 静态门 | Covered |

**Verification**

- 各 Task 契约矩阵逐项 0 failed。
- 全量：M19 分批形态（批内 `--test-threads=1`、批间顺序）全部批次 0 failed（基线 1515 + 本轮新增）。
- 静态门：`cargo clippy --all-targets -- -D warnings`、`cargo fmt --check`、`openspec validate 2026-09-29-ms21-ddl-evolution` 全部通过。

**Gate 2 Readiness**

Gate 2 全部 PASS（2026-09-30 用户批准本 Iteration 计划——原话「批准」）。调查（AST 形状/checkpoint 接线/释放通道持久化语义/三类根重置 API/pipeline 接线全部 file:line 定位）、设计（D11 闭合 + flush 收尾序 + 窗口分析教训带入）、任务契约完备（2 契约依赖有序、拒绝面与 Preserve/Forbidden 明确）、验证最简直接（原生 cargo test 退出码）、Persisted Evidence 无必要——以上 PASS。无 BLOCKED、无 WAIVED 项。

**Persisted Evidence**

- Mode: none

**Risks and Notes**

- **多表拒绝的措辞校准（收尾项）**：`TRUNCATE t1, t2` 在 sqlparser 0.44 解析期即失败（单 `table_name` 字段），错误为解析器语法错误而非能力名点名；spec 场景「多表不支持」的见证以「错误返回」判定，「计划期点名拒绝」的字面措辞校准（计划期 → 解析期）留收尾合并裁定——与 Iter003 F2（DROP 键引用守卫执行器阶段）同批校准。
- **崩溃窗口（D11 步骤 2 后至 `flush_all()` 前）**：checkpoint 已截断 WAL → 无重放复活通道。`FileStorage::free_page` **立即零化磁盘页**（`write_page` 直写），而 head 重置与 catalog tail/根写回为缓冲池脏页（`flush_all()` 前不持久）——释放开始后、flush 前崩溃：重开 = head 页（未重置落盘）残余行可见 + 已释放页行消失 + 旧索引根（悬空条目对零化页，点查走 SlotNotFound 容忍同型）——即 D11 已文档化的「部分清除」诚实化状态；flush 后崩溃 → 完整 TRUNCATE 态。执行器 `flush_all()` 收尾把窗口压缩到语句执行期间。
- **复用页可见性摘要残留**：freed 页从缓冲池移除但 `vis_map` 条目可能残留；页 id 重用后由既有 clear_all_visible-on-write 语义覆盖（MS17-T02 哨兵语义）——TRUNCATE 后 INSERT 见证覆盖。
- **checkpoint 零工作守卫交互**：无未决 WAL 且无脏页时 `checkpoint()` 无操作早退——不影响后续物理清除；`flush_all()` 收尾独立保证持久。
- 非实质选择留给 Act：`TruncateNode` 字段组织、`truncate_table` 方法签名形状、空树构造的复用形态、拒绝文案措辞（点名能力）、用例命名与断言组织。

## Act Response

- Status: reported

**Actual Changes**

- `src/executor/plan.rs` — `PhysicalPlan::Truncate(TruncateNode)` 变体 + `TruncateNode { table_name }` 节点（置于 DDL 变体组，DropIndex 之后）。
- `src/parser/planner/mod.rs` — `build_plan` 新增 `Statement::Truncate` 分派臂 → `build_truncate`。
- `src/parser/planner/ddl_dml.rs` — `build_truncate`：`partitions`（Hive `PARTITION` 清单）点名拒绝 → `reject_attached_write` → `tables` 注册表存在性防御臂（pipeline 通道先行 42P01）→ `TruncateNode`。
- `src/executor/truncate.rs`（新）— `TruncateExecutor`（持 `Arc<Database>`，AlterTableExecutor 同型）：步骤 1 `Database::checkpoint()`（checkpoint-first，D11 步骤 2）→ 步骤 2 `table_manager.truncate_table`；doc 注释携带崩溃窗口分析（与 D11/Risks 一致）。
- `src/storage/data/table_manager.rs` — 新 `truncate_table`（reserved-name 防御臂 → `collect_data_pages` 尾页 `free_page` 释放、head 页 `SlottedPage::init(0x03)` 原地重置 → in-memory tail 先回 head（`replace_recovery_indexes` 换入继承）→ `update_table_tail` 回写 → PK/唯一/二级空树构造 + 三类 `update_*_root` catalog-first 根写回（失败路径新树 best-effort 释放后传播，零副作用）→ `replace_recovery_indexes` 三类整表换入 → 旧树 `collect_all_pages` best-effort 释放（drop_table 同策略）→ `flush_all()` 收尾）；私有 helper `new_empty_tree`（`IndexManager::new` spawn_blocking + `with_catalog_context`）。doc 注释携带五步序与崩溃窗口分析。`drop_table`/`collect_data_pages` 本体零改动（纯新增 + import 行）。
- `src/pipeline.rs` — 四接线臂：`execute_stage_routed` Truncate 直包执行器 + 执行后 `plan_cache.clear()`；`execute_stage_in_tx_routed` 执行期 0A000 点名拒绝（先于执行器副作用）；`create_executor_from_plan` panic 臂扩展；`extract_all_table_names` 新增 `Statement::Truncate` 目标臂（register_table 通道承载 42P01/附加库标记）。
- 穷尽补臂三处：`src/executor/plan_format.rs`（render 叶臂 + `describe` "Truncate {table}"）、`src/parser/planner/query.rs`（`get_plan_output_columns` DDL 空 Vec 臂）、`src/executor/correlated.rs`（叶臂）。
- `src/executor/mod.rs` — `mod truncate` + `TruncateExecutor`/`TruncateNode` 导出。
- `tests/truncate_test.rs`（新，7 用例）— R1-S1 清空空集/二级索引空集/schema 保留/新写入 PK+二级可达、R1-S1+ 裸 `TRUNCATE` 等价、R1-S2 close 重开持久不复活、R1-S3 in-tx 0A000 零副作用会话态不变、R1-S4 多表（解析期失败）/缺失/保留名/`PARTITION` 拒绝 + 零副作用、R1-S5 空表幂等、R2 崩溃见证（flush_all + WAL shutdown + drop 不 close → 重开行不复活、空树可用）。

**Deviations from Plan**

- 无实质偏差。计划留 Act 的非实质选择：`new_empty_tree` helper 形态、catalog 根写回失败路径的新树清理（对齐 create_index 失败释放先例）、in-memory tail 先于换入重置的实现位置。
- 流程注记：首次全量 Gate 后台运行误带 `tail -4` 管道——聚合输出丢失且管道退出码（恒 0）不能证明 cargo 退出码，该次不构成有效证据，随即原样重跑（无管道）取得下述有效结论；非验证重复（首次未产生可判定证据）。

**Self-Review Findings**

- 已修复：`new_empty_tree` 失败路径初版为冗余 match（`Err(e) => return Err(e)`），Gate 3 收敛为 `?`；`cargo fmt` 校准 1 处 eprintln 折行。无遗留 Critical/Important。
- 遗留 Minor：无。（多表拒绝措辞校准〔计划期 → 解析期〕为 Plan Risks 已登记的收尾项，测试按「错误返回 = 拒绝」见证，不在本 Cycle 处理。）

**Verification**

- RED 见证：实现前 `cargo test --test truncate_test -- --test-threads=1` → 7 failed / 0 passed（全部 `Unsupported statement type: TRUNCATE` catch-all，与契约 Current behavior 一致）。
- T4-a 契约命令：`cargo test --test truncate_test --test drop_table_free_test --test checkpoint_skip_test -- --test-threads=1` → truncate 7 + drop_table_free 6 + checkpoint_skip 6 = 19 passed / 0 failed（exit 0）。
- T4-b 契约命令：`cargo test --test truncate_test --test mvcc_tombstone_visibility_test --test wal_recovery_large_test -- --test-threads=1` → 7 + 11 + 7 = 25 passed / 0 failed（exit 0）。
- 全量 Gate：`cargo test -- --test-threads=1`（无管道）→ **1522 passed / 0 failed / 2 ignored**（基线 1515 + 本轮 7），exit 0。
- 静态门：`cargo clippy --all-targets -- -D warnings` 无告警；`cargo fmt --check` 干净；`openspec validate 2026-09-29-ms21-ddl-evolution` → "is valid"。

**Persisted Evidence**

- None required（Plan Mode: none；验证结论已按上记录入，均可低成本重跑）。

**Experience Candidates**

- None。

**Unresolved Issues**

- 无。

## Plan Review

- Review Result: accepted
- Date: 2026-09-30

**Findings**

独立 diff 审读（`truncate.rs`/`truncate_table`/`new_empty_tree`/`build_truncate`/`mod.rs` 分派/pipeline 四接线/穷尽臂经编译完整性背书/`tests/truncate_test.rs`）与 Act Response 逐项核对。

- **F1〔非阻塞 Minor · 边缘〕三类根写回部分失败的悬空根**：`truncate_table` 步骤 3 的 catalog 根写回逐项进行（PK → 唯一 → 二级）；若中途失败（如 PK 根已写、唯一根写失败），失败路径 best-effort 释放**全部新树**后传播——已写入 catalog 的根（如 PK）随即指向已释放零化页。in-memory meta 未换入保持一致（代码注释该 claim 准确），会话内无恙；但 close 后 flush 使悬空根持久 → 重开 PK 索引 `from_root` 于零化页。仅 IO 失败可达，属「DDL 部分状态」诚实化同类；doc 注释未陈述该残余。处置：记录为权威；doc 注释补记建议随后续触碰该文件的顺带修正（非阻塞）。
- **F2〔核实通过〕head 重置页类型语义**：`SlottedPage::init(page, 0x03)` 产出完整 Data 页头（`SlottedPageHeader::new` 归零 slot 结构）；建表路径依赖 `allocate_page` 零化页（`page_type == 0` 由 `data_page.rs:25-26` 懒初始化特判）——两态并存被既有读取路径覆盖，0x03 为 catalog 系统页同款 Data 类型，无消费点歧义。TRUNCATE 后 INSERT/点查/二级查询见证覆盖。
- **F3〔已登记项确认〕多表拒绝措辞校准**：Plan Risks 收尾项维持（`TRUNCATE t1, t2` 解析期失败、测试按「错误返回 = 拒绝」见证），与 Iter003 F2 同批收尾校准。
- **F4〔采信说明〕验证结论**：RED（7 failed / 0 passed，catch-all 与契约 Current behavior 一致）→ 目标矩阵 19/0 + 25/0 → 全量串行 **1522 passed / 0 failed / 2 ignored**（= 基线 1515 + 本轮 7，计数自洽）→ 静态门全绿。git 只读核对：工作区文件集与 Act 声明一致（新增 `src/executor/truncate.rs`/`tests/truncate_test.rs`，无意外面），覆盖面未变化；叠加用户明示「测试可信」，采信不重跑。Act 流程注记（首次全量带管道退出码不构成证据、原样重跑）符合公共规则 › 验证纪律。
- **F5〔偏差分类〕无实质偏差**：声明的非实质选择（`new_empty_tree` helper、失败路径新树清理、in-memory tail 先行）均在 Plan 授权范围内；truncate_table 五步序与契约执行序逐步吻合；`drop_table`/`collect_data_pages` 本体零改动（契约 Preserve）达成。

**Evidence**

- 代码面：`truncate.rs` 两步执行序（checkpoint → truncate_table）与 D11 逐步对应；`truncate_table` 防御臂 → 尾页释放（`skip(1)` 保 head）→ head 原地重置 → in-memory tail + `update_table_tail` → 三类空树 catalog-first 根写回（失败释放新树传播）→ `replace_recovery_indexes` 换入（head/tail 继承核实 `:1044` 区域）→ 旧树 best-effort 释放 → `flush_all()` 收尾；`build_truncate` 拒绝面（partitions/attached/existence）与 D14 一致；pipeline 直包臂 + `plan_cache.clear()`、in-tx 0A000 臂先于副作用、`extract_all_table_names:1763`。
- 测试面：`truncate_test.rs` 7 例与场景映射头注逐项对应（清空/二级空集/schema 保留/新写入双通道可达、裸 TRUNCATE、重开持久、in-tx 0A000 零副作用、边界拒绝矩阵含零副作用断言、空表幂等、崩溃不复活 + 空树可用）。
- 验证面：见 F4 采信说明。

**Acceptance Gaps**

None——R1 五场景 + R2 零回归全部有见证，RTM Covered，既有矩阵零回归，全量与静态门新鲜。

**Follow-up Decision**

接受（accepted）：F1 为 IO 失败边缘的诚实化残余（正确分析已记录，doc 补记非阻塞）；F3 为 Plan 阶段已登记的收尾校准项。无需当前 Cycle 修复，无 rework/replan。

**Next Cycle**

None

**Next Iteration**

`openspec/changes/2026-09-29-ms21-ddl-evolution/iterations/005-create-drop-view/000-initial.md`（已展开，Status: draft——Gate 2 待用户批准后转 ready 交 Act；本 change 最后一个 Iteration）
