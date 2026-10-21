# Iteration 003 / Cycle 000: 差分白名单缺陷清偿与登记收口（T03 + T04）

## Plan Context

- Status: ready（2026-10-21 Gate 2 自评 PASS；用户批准计划）
- Cycle Type: initial

**Cycle Scope**

- Change tasks: T03（①②③④ 四缺陷清偿）、T04（投影位外层作用域段登记收口——2026-10-21 用户指令并入，原话「把这些问题和iter3本来要做的规划在一起解决掉」）
- Acceptance gaps: None
- Repair items: None
- Inherited scope: `sql-semantics-conformance` R1-R4（本 Iteration 定稿新增）；`sql-semantic-binding` R4（本 Iteration 定稿新增）；R1-R3 既有交付面维持（002 `accepted` 现场原样）；proposal A4/A7；差分网 GREEN 保护基线（10/0，白名单 6 条目自本 Iteration 解冻——002 验证已完成）；既有全量 2046/0/2 基线
- Excluded scope: ⑤ 谓词×IN 子查询 AND 组合开放（维持拒绝）；差分子集扩容 / fuzzing / rusqlite 升级；`ORD-NULL-ORDERING` 已知语义差异条目；attach 隐式短名命中面扩面与视图/CTE 段登记再扩面（002 Review 期调查定论：五 FROM 面臂已登记、D6 兜底覆盖、无缺陷证据——不扩面）；执行器批量接口 / 类型化 API（MS31/MS32）；`condition_column_index` 等其余执行期按名点收口（I080 后续批次）

**Objective**

差分白名单 6 条目清偿至 1（仅剩 `ORD-NULL-ORDERING` 已知语义差异）：① 无键基表在 JOIN/SemiJoin 基座行完整可见、② TEXT 相关参数按真实值求值、④ `NOT IN` NULL 三值折叠、③ 聚合派生表外层列按 SELECT 清单名解析；遗留 L1 关闭：投影位外层段临时登记覆盖无别名派生表。五修复各自独立 RED→GREEN；既有 2046 基线与全部非缺陷输入行为零回归。

**Background**

Iteration 000 差分网分诊出 5 条 suspected 白名单条目（①×3〔Q3-03/Q3-08/Q6-03〕②×1〔Q5-05〕④×1〔Q6-04〕；③ 为拒绝面不入白名单，Q7-01 改非聚合形态并在 000 Act Response 报告）。2026-10-20 用户裁定四类缺陷以 T03/Iteration 003 在本 change 内清偿。2026-10-21 用户指令将 001-replan Review 遗留移交的「投影臂无别名派生外层段登记不完备」并入本 Iteration（T04）。

**Investigation Facts**

- Current Baseline: 工作区 = Iteration 002 `accepted` 现场（未提交：planner 五文件 + `binding.rs` + `pipeline.rs` + 三测试文件 + 本 change 目录）；全量 2046 passed / 0 failed / 2 ignored（002 Act，M19 分批 14 批实测）；差分网 10/0、白名单 6 条目；`cargo fmt --check` 零 diff。
- Current-State Evidence（2026-10-21 本会话实读，行号当前有效；Act 无需回读其他资料）:

  **① 无键基表 JOIN/SemiJoin 空集（Q3-03/Q3-08/Q6-03）**

  - `ScanExecutor::next`（`src/executor/scan.rs:55-99`）经 `table_meta.index_manager.scan_all()` 枚举行再按快照可见性读回。无键表（PK 列类型非 INT——行存储于数据页但不进入 PK B-Tree，planner 侧同款事实见 `query.rs:2221-2223` 注释「键列已知声明非 Int 时……该表全行无键」）索引为空 → 0 行。
  - 单表查询末端把顶层 `PhysicalPlan::Scan` 换成 `DataScan`（`query.rs:2236-2296`，OR / 谓词下推 / 无 WHERE 三臂；`match base_plan` 只匹配顶层），JOIN 内层 Scan 节点不做转换——缺陷只在 JOIN/SemiJoin 基座形态显形。
  - `TableMeta`（`table_manager.rs:87-112`）：`pk_index: usize`、`columns: Vec<(String, ColumnType)>`、`data_page_head`、`defaults`、`index_manager`。无键判据 = `columns[pk_index].1` 非 `ColumnType::Int`（`create_table_with_constraints` 要求 pk 命名列，`table_manager.rs:423-426`；无显式 PK 的建表走首列隐式 PK 通道，首列非 INT 即无键）。
  - `DataScanExecutor::new(table_meta, buffer_pool, snapshot, predicate, scan_cap, tx_manager)`（`data_scan.rs:116-138`）：数据页链走查（`data_page_head` 起、`next_page_id` 链），MVCC 快照可见性 + 替代去重（superseded-version map）+ 宽度感知读回 + 溢出两相装配——单表通道即依赖它对无键表正确返回行。
  - 差分白名单条目正文（`sql_differential_test.rs:176-190`）：Q3-03 自连接 0 行 vs SQLite 710 行、Q3-08 同 + TEXT 谓词 0 vs 500、Q6-03 SemiJoin 外层 0 vs 185。

  **② TEXT 相关参数求值 NULL（Q5-05）**

  - `ParameterExpression::evaluate_ref`（`predicate.rs:453-474`）：`Value::String(_) => ValueRef::Null`（:472，M36 T5 注释——MutexGuard 临时借不出 `&'a str`）。
  - `ValueRef`（`value_ref.rs:14-24`）：`Copy` 枚举、`Text(&'a str)` 纯借用、`size_of ≤ 24B` 测试约束——加 owned 变体破坏 Copy，属 I020 表示层范围，排除。
  - `Expression` trait（`predicate.rs:50-73`）：`evaluate(&self, row: &[Value])` 为 owned 向后兼容入口（默认实现：行转 refs → `evaluate_ref` → `to_value`）；`set_parameter_value(&self, ...)` 经 `Mutex<Value>` 内部可变性（`&self` 签名不可改，连锁全 planner）。**根因**：composite 子表达式默认经 `evaluate_ref` 递归，`ParameterExpression` 的 owned 入口从未被覆盖——参数真值在 owned 通道也不可达。
  - 谓词消费点走 owned 入口、行已是 owned `Vec<Value>`：`data_scan.rs:198`、`filter.rs:45`（均 `p.evaluate(&values)`）——在谓词层做 owned 分支不需要改任何消费点。
  - composite 实现清单：Expression 族 = `CaseExpression` / `CoalesceExpression` / `CastExpression` / `BinaryArithExpression` / `IntervalArithExpression`（predicate.rs）+ `FunctionExpression`（function.rs）；Predicate 族 = `ComparisonPredicate` / `ConstantPredicate` / `LogicalPredicate` / `LikePredicate` / `IsNullPredicate` / `NotPredicate`（predicate.rs）。叶子（Column/Constant）无子节点，无需改。
  - 注入通道不变：`inject_correlated_values`（`correlated.rs:14-`）经 `set_parameter_value` 逐计划注入；四执行器 `extract_param_values` 按外层行取值；子查询语句级缓存存 owned 全行集（含 NULL 键行）。
  - 差分白名单条目（`sql_differential_test.rs:191-196`）：Q5-05 TEXT 相关参数恒 NULL → 相关标量子查询外层 0 行 vs SQLite 106 行；INT 参数通道（Q5-04）正常。

  **④ NOT IN NULL 三值折叠缺失（Q6-04）**

  - `AntiJoinExecutor`（`anti_join.rs`）：`build_left_key` 对含 NULL 键返回 `None`（:137-141）；ScanLeft NOT IN 臂 `None =>` **输出**该行（:307-313，注释「NULL key means no match, so output」——三值语义应为 Unknown 折叠不返回）；`build_right_key` 对含 NULL 键返回 `None` → 行不入 hashmap（:113-117）——右集含 NULL 的全局折叠无载体。
  - 三处右集装配点：独立子查询 BuildRight（:193-200）、缓存命中重建（:228-240，缓存 `collected` 含 NULL 键行——重建时可重算标志）、未命中重建（:256-273）。
  - NOT EXISTS 臂（conditions 空，:286-293）与 SemiJoin（IN 通道 NULL 行为已正确，whitelist 明证）不在缺陷面。
  - 差分白名单条目（`sql_differential_test.rs:197-203`）：Q6-04 最小复现 orders(1,10)/(2,NULL)/(3,20)/(4,NULL) + users(10,20) → NOT IN 返回 ids 2,4（恰 NULL 行），SQLite 0 行。

  **③ 聚合派生表外层列引用拒绝**

  - 派生臂注册列源：`query.rs:482` `let columns = self.get_plan_output_columns(&subquery_plan);` → `register_table` + `register_binding_scope`。
  - `get_plan_output_columns` Aggregate 臂（`query.rs:87`）返回 `node.output_columns` = `node_output_columns`（`query.rs:2468-2472`）= **分组键规范名** ++ 聚合显示名——键位不带 SELECT 别名（`tag AS n` → "tag"）。
  - direct 形态判定（`query.rs:2439-2458`）：SELECT = 纯列键（序同 GROUP BY）++ 聚合项且无表达式项 → **无包装 Projection** → 聚合派生子查询计划顶 = Aggregate（可再被 Having/Sort/Limit 包裹）；非 direct 形态经 `wrap_projection` 落 `ProjectionNode`（columns = SELECT 名，`query.rs:164` Projection 臂直出）——非聚合与非 direct 聚合派生表现状正常的原因。
  - CTE 形态正常的原因：`CteDef.columns` 在定义期捕获（`resolve_cte_reference` `query.rs:1279-1283` 用 `def.columns.clone()`，非计划输出）——CTE/视图通道不在本缺陷面，不动。
  - 位置对齐事实：direct 判定强制 SELECT 键序 = GROUP BY 键序且聚合注册序 = SELECT 序（`query.rs:2461-2467` 注释）——`node_output_columns` 与 SELECT 清单**逐位对齐**，逐位改名安全。
  - 差分记录：③ 为拒绝面（`Column not found: n`）不入白名单；Q7-01 现用非聚合派生形态（000 Act Response 报告），修复后可加聚合派生差分查询。

  **T04 投影臂无别名派生段登记缺口**

  - `declared_qualifiers` Derived 臂（`subquery.rs:483-486`）无别名返回**空**；派生臂实际注册名 = `"derived"`（`query.rs:477-480` `unwrap_or_else(|| "derived".to_string())`，register_table / register_binding_scope 同名）。
  - 后果：投影位标量子查询的外层 FROM 为无别名派生表时，构建前临时登记（`query.rs:1478-1484` `declared_from_qualifiers` 循环）漏掉该段——外层引用 `derived.k` 的作用域身份判定落入缺口（001-replan Review 遗留移交，Gate 3 Minor 记录「书写限定符不可达」）。
  - Table 臂产出 `[别名, 基表名]` 并列（`subquery.rs:476-481`，A1 语义）与 FROM 臂注册一致，不在缺口面。

- Code and Critical Path: ① `src/executor/scan.rs`（枚举通道回退）+ `data_scan.rs`（复用走查，零改动）；② `src/executor/predicate.rs`（ParameterExpression evaluate 覆盖 + composite owned 分支 + contains_parameter 单源）+ `function.rs`（FunctionExpression 同款）；④ `src/executor/anti_join.rs`（三值折叠 + right_has_null_key 标志三装配点）；③ `src/parser/planner/query.rs`（派生臂注册列源 helper，聚合 direct 形态逐位映射 SELECT 名）；T04 `src/parser/planner/subquery.rs::declared_qualifiers`（Derived 无别名臂补 `derived`）。验证面：`tests/sql_differential_test.rs`（白名单 5 条目移除 + Q7 聚合派生新增）、各定向测试文件新增函数。

**Implementation Guidance**

建议顺序 T03-D1 → D4 → D2 → D3 → T04（D1/D4 纯执行器、互相独立；D2 触及表达式族面最宽放中段；D3 与 002 同文件 `query.rs` 放后；T04 最小收尾）：

1. **D1**：`ScanExecutor` 无键判据（`table_meta.columns` 取 `pk_index` 位类型非 `ColumnType::Int`，越界按无键处理）→ `next()` 首次执行时构造内部 `DataScanExecutor`（predicate `None`、scan_cap `None`、同 snapshot、同 table_meta/buffer_pool）驱动至耗尽并按本节点 `projection` 应用后收进 `results`——外部物化语义与既有 PK 路径同形。PK 表路径零改动。
2. **D4**：`right_has_null_key: bool` 字段；三装配点统一「行键含 NULL → 置位且不入 hashmap」；ScanLeft NOT IN 臂：左键 `None`（含 NULL）→ `continue`；`Some(key)` → `!right_has_null_key && !hashmap.contains_key(&key)` 才输出。NOT EXISTS / SemiJoin 零改动。
3. **D2**：`predicate.rs` 新增 `contains_parameter`（Expression/Predicate 两树递归，`ParameterExpression` 真）；`ParameterExpression::evaluate` 覆盖为 `self.value.lock().unwrap().clone()`；六 composite Expression + 六 Predicate 的 `evaluate()` 覆盖：`contains_parameter(自身)` 为真 → 子节点经 `evaluate()` 递归的 owned 求值，否则默认实现（refs 路径）——非参数路径逐字节保持。消费点（data_scan/filter 等）零改动。
4. **D3**：`query.rs` 派生臂列源改为 helper：解包 Limit/Sort/Having 后顶层为 Aggregate 时按 SELECT 清单逐位取名（键位 = 别名 ?? 列名；聚合位 = `node_output_columns` 对应显示名——direct 判定保证逐位对齐），否则维持 `get_plan_output_columns`。`get_plan_output_columns` 本身零改动（HAVING/ORDER 内部消费面不动）；CTE/视图捕获通道零改动。
5. **T04**：`declared_qualifiers` Derived 无别名臂返回 `vec!["derived".to_string()]`（与 `query.rs:477-480` 注册名一致；两处字面量以注释互指，来源单源）。
6. 每修复独立 RED → GREEN → 定向；五项全绿后白名单移除 5 条 + Q7 新增聚合派生查询 → 差分网全量 → 全量分批一次（M19）。

**Task Contracts**

### T03-D1: 无键基表 JOIN/SemiJoin 行可见性（①）

- Requirement/Scenario: `sql-semantics-conformance` R1 全部 3 场景
- Depends on: None（本 Iteration 内独立）
- Targets: `src/executor/scan.rs::ScanExecutor`
- Current behavior: 无键表（PK 列非 INT）作为 JOIN/SemiJoin 基座经 `index_manager.scan_all()` 枚举得 0 行（差分 Q3-03/Q3-08/Q6-03 白名单见证）
- Required behavior: 无键表基座行完整可见（数据页链走查，与单表 DataScan 通道同一可见性语义）；带 INT 主键表枚举路径与结果逐字节不变
- Preserve: `DataScanExecutor` 零改动（复用不改）；PK 表路径零改动；计划形状 / EXPLAIN 零变化（纯执行器内部）
- Forbidden: planner 路由改动（`query.rs` Scan→DataScan 转换面）；`DataScanExecutor` / `IndexManager` 语义改动；输出行序对 PK 表的任何变化
- First test: RED——差分白名单移除 Q3-03/Q3-08/Q6-03 三条目（现状差分失败：RTsql 0 行 vs SQLite 非空）；定向 `tests/join_test.rs` 新增函数（无键基表两表 JOIN 断言行数与值——现状断言失败）
- GREEN condition: 三条目移除后 `cargo test --test sql_differential_test` 全绿；新定向函数全绿；既有 JOIN/SemiJoin 套件零回归
- Verification: ① 差分网；② `join_test` + `subquery_test`（SemiJoin 形态）；③ 既有定向 JOIN 族套件
- Stop when: 无键判据在既有表形态外还有第三态（如 pk_index 越界的存量表）导致 PK/无键二分不成立；DataScan 走查复用需要改其公共签名

### T03-D2: TEXT 相关参数真实值求值（②）

- Requirement/Scenario: `sql-semantics-conformance` R2 全部 2 场景
- Depends on: None（本 Iteration 内独立）
- Targets: `src/executor/predicate.rs`（`ParameterExpression`、六 composite Expression、六 Predicate、`contains_parameter`）、`src/executor/function.rs::FunctionExpression::evaluate`
- Current behavior: 参数值为 String 时 `evaluate_ref` 返回 `ValueRef::Null`（`predicate.rs:472`）——含参树在 composite 递归中恒得 NULL，`evaluate` 默认实现经 `evaluate_ref` 同样不可达真值
- Required behavior: 含参数的表达式/谓词树求值得真实值（含 String/Date/Timestamp）；不含参数的树维持既有零拷贝路径与结果逐字节一致
- Preserve: `ValueRef` 形态（Copy / `Text(&'a str)`）零改动；`set_parameter_value` / 注入通道 / 语句级缓存零改动；谓词消费点（`data_scan.rs:198`、`filter.rs:45` 等）零改动；非参数路径行为与分配特性不变（`contains_parameter` 为假走默认实现）
- Forbidden: `ValueRef` 加 owned 变体或去 Copy（I020 表示层范围）；`Expression`/`Predicate` trait 公共签名改动；`unsafe` 借期扩展；per-row Box::leak 类无界内存策略
- First test: RED——差分白名单移除 Q5-05（现状差分失败：0 行 vs 106 行）；定向 `tests/scalar_subquery_comparison_test.rs` 新增函数（外层 TEXT 列相关标量子查询等值匹配断言非空——现状失败）
- GREEN condition: Q5-05 移除后差分全绿；新定向函数全绿；既有表达式/谓词/子查询套件零回归
- Verification: ① 差分网；② `scalar_subquery_comparison_test` + `subquery_test` + `expression` 族定向；③ 全量（非参数路径回归由全量 Gate 见证）
- Stop when: 某 composite 的 owned 递归与其 ref 语义存在可观察差异（溢出 / 类型门 / NULL 传播不一致）→ 返回 Plan

### T03-D4: NOT IN NULL 三值折叠（④）

- Requirement/Scenario: `sql-semantics-conformance` R3 全部 3 场景
- Depends on: None（本 Iteration 内独立）
- Targets: `src/executor/anti_join.rs::AntiJoinExecutor`
- Current behavior: 左键含 NULL 的行被输出（:307-313）；右集含 NULL 键时不匹配行被输出（右 NULL 键行不入 hashmap、无全局折叠载体）
- Required behavior: 左键 NULL 行不返回；右集含 NULL 键时全部不匹配行不返回；右集空集时全行返回；`IN`（SemiJoin）与 `NOT EXISTS` 通道行为不变
- Preserve: NOT EXISTS 臂（conditions 空）零改动；SemiJoin 零改动；缓存键与缓存内容结构不变（`collected` 仍含 NULL 键行，标志重建时重算）
- Forbidden: SemiJoin / SubqueryEval / ScalarSubqueryFilter 改动；相关子查询缓存机制改动
- First test: RED——差分白名单移除 Q6-04（现状差分失败：ids 2,4 vs 0 行）；定向 `tests/subquery_test.rs` 新增函数（Q6-04 最小复现断言 0 行 + 右集空集全行返回对照——现状失败）
- GREEN condition: Q6-04 移除后差分全绿；新定向函数全绿；既有 NOT IN / IN / NOT EXISTS 套件零回归
- Verification: ① 差分网；② `subquery_test` NOT IN 族；③ 全量
- Stop when: 三值折叠需触碰 `build_right_key`/`build_left_key` 的「NULL 不匹配 NULL」既有连接语义（JOIN 通道共用语义被牵连）→ 返回 Plan

### T03-D3: 聚合派生表输出列引用（③）

- Requirement/Scenario: `sql-semantics-conformance` R4 全部 2 场景
- Depends on: None（与 002 同文件 `query.rs`，顺序编辑避免冲突）
- Targets: `src/parser/planner/query.rs`（派生臂 :482 列源 + 新 helper）
- Current behavior: direct 形态聚合派生子查询的注册列 = 分组键规范名 ++ 聚合显示名——键位别名丢失，外层 `d.n` 计划期 `Column not found: n`
- Required behavior: 聚合派生表注册列 = SELECT 清单输出名（键位别名生效，聚合位维持显示名，逐位对齐）；非聚合与非 direct 聚合派生、CTE/视图通道、聚合查询自身通道（HAVING/ORDER/表头）零变化
- Preserve: `get_plan_output_columns` 各臂零改动；`AggregateNode` 结构与计划形状零改动（EXPLAIN 零校准）；CTE/视图定义期捕获通道零改动
- Forbidden: `get_plan_output_columns` Aggregate 臂改动（内部消费面风险）；`AggregateNode` 字段变更；直出/包装判定逻辑改动
- First test: RED——定向 `tests/derived_scan_alias_test.rs` 新增函数（`FROM (SELECT tag AS n, COUNT(*) AS c … GROUP BY tag) d` 外层 `d.n`/`d.c` 断言值——现状 `Column not found: n`）；GREEN 后差分 Q7 新增聚合派生查询（双引擎一致）
- GREEN condition: 新定向函数全绿；Q7 新增查询差分一致；既有聚合/派生/CTE/视图套件零回归；`explain_test` 零校准
- Verification: ① `derived_scan_alias_test` + `cte_test` + `view_test` + 聚合族定向；② 差分网（Q7 新增）；③ `explain_test`；④ 全量
- Stop when: SELECT 清单逐位取名在某个 direct 边界形态（如 `SELECT *` 聚合、键位表达式）与 `node_output_columns` 不对齐 → 返回 Plan

### T04: 投影位外层作用域段登记收口（L1 遗留）

- Requirement/Scenario: `sql-semantic-binding` R4（新增）全部 1 场景
- Depends on: None（本 Iteration 内独立）
- Targets: `src/parser/planner/subquery.rs::declared_qualifiers`
- Current behavior: Derived 无别名臂返回空——投影位临时登记漏段，外层无别名派生表的投影位相关引用落入登记缺口（001-replan Review 遗留移交）
- Required behavior: Derived 无别名臂返回 `vec!["derived"]`（与派生臂注册名 `query.rs:477-480` 单源一致）；该形态投影位相关引用经作用域身份解析为外层相关参数并正确注入求值；其余投影位形态行为不变
- Preserve: Table 臂 `[别名, 基表名]` 并列语义；`declared_from_qualifiers` / `extract_subquery_table_names` 委托关系；WHERE 侧三臂（外层快照取 live 作用域）零改动
- Forbidden: 无别名派生隐式名改为其他值（破坏注册名单源）；`declared_qualifiers` 对其余 TableFactor 变体的产出变化
- First test: RED——`tests/subquery_alias_binding_test.rs` 新增函数（外层 `FROM (SELECT k, v FROM t)` 无别名 + 投影位 `(SELECT max(x) FROM tt WHERE tt.y = derived.k)` 断言正确值——现状登记缺口致失败/错误）
- GREEN condition: 新函数全绿；既有 `subquery_alias_binding_test` 全部（含碰撞回归）零回归
- Verification: ① `subquery_alias_binding_test`；② `derived_scan_alias_test`；③ 全量
- Stop when: 该形态的正确求值暴露注入机制另有缺口（分类/收集正确但执行期取值失败）→ 返回 Plan

**Invariants**

- A7 零回归硬边界：既有 2046 基线零失败；全部非缺陷输入的行为、错误面（类型、文本、SQLSTATE）逐字节不变；EXPLAIN 计划形状零校准（五修复均不改计划节点）。
- 白名单单向收缩：移除 Q3-03/Q3-08/Q6-03/Q5-05/Q6-04 恰 5 条，`ORD-NULL-ORDERING` 保留；白名单机制与结构自检测试不变；Q7 新增聚合派生查询。
- ⑤ 谓词×IN 子查询 AND 组合维持既有计划期拒绝（proposal Non-goal）。
- 非参数表达式/谓词求值路径逐字节保持（D2 的 `contains_parameter` 为假分支 = 默认实现）。
- PK 表 Scan 枚举路径与输出逐字节保持（D1 的无键分支不可达）。
- 固定种子差分网确定性维持。

**Non-goals**

见 Excluded scope。

**Acceptance**

- `sql-semantics-conformance` R1（无键行可见）：白名单 ① 三条目移除后差分双引擎一致 + 定向 JOIN/SemiJoin 函数全绿。映射：T03-D1。
- R2（参数保真）：Q5-05 移除后差分一致 + TEXT 相关定向全绿 + 非参数路径全量零回归。映射：T03-D2。
- R3（NOT IN 折叠）：Q6-04 移除后差分一致 + 定向三态函数全绿 + IN/NOT EXISTS 零回归。映射：T03-D4。
- R4（聚合派生引用）：定向别名键函数全绿 + Q7 新增差分一致 + 聚合自身通道零回归。映射：T03-D3。
- `sql-semantic-binding` R4（登记完备）：T04 定向函数全绿 + 既有碰撞回归零回归。映射：T04。
- 全量：M19 分批一次零失败零校准。

**Verification**

| Scenario | 判定 | 通过依据 |
|---|---|---|
| R1 无键 JOIN/SemiJoin | 差分网（3 条目移除）+ `join_test`/`subquery_test` 新函数 | 全绿且双引擎一致 |
| R1 PK 表零回归 | 既有 JOIN 族定向 + 全量 | 零回归 |
| R2 参数保真 | 差分网（Q5-05 移除）+ `scalar_subquery_comparison_test` 新函数 | 全绿且双引擎一致 |
| R2 非参数路径 | 表达式/谓词族定向 + 全量 | 零回归 |
| R3 NOT IN 三态 | 差分网（Q6-04 移除）+ `subquery_test` 新函数（含空集对照） | 全绿且双引擎一致 |
| R3 IN/NOT EXISTS 不变 | 既有 NOT IN/IN/EXISTS 定向 | 零回归 |
| R4 聚合派生 | `derived_scan_alias_test` 新函数 + 差分 Q7 新增 + `explain_test` | 全绿、零校准 |
| binding R4 登记收口 | `subquery_alias_binding_test` 新函数 + 既有碰撞回归 | 全绿 |
| 全量 | M19 分批一次 | rc=0、零校准 |

**Gate 2 Readiness**

自评 PASS：RTM 无 Missing/Simplified（R1-R4 两域新增行全部映射 T03/T04，First Test 均为可执行 RED）；设计闭合（五修复的判据、装配点、复用面、边界均已定位 file:line；D2 的 owned 通道设计与 ValueRef Copy 约束、消费点 owned 入口事实已核对）；契约含完整执行边界与停止条件；Persisted Evidence `none`。待用户批准后转 `ready`。

**Persisted Evidence**

- Mode: none

Act Response 承载定向与全量命令及结果；无不可低成本重跑项。

**Risks and Notes**

- R1（条件性）：D1 无键判据依赖「无显式 PK 建表走首列隐式 PK、首列非 INT 即无键」的既有表示——若存量库存在 `pk_index` 越界形态，按无键处理（保守面）；`ColumnType` Int 变体名以 `src/storage/page_format` 实际定义为准（差异即 Stop when）。
- R2（边界记录）：D2 的 `contains_parameter` 每次求值 O(树规模)——表达式树小（个位到十位节点），相对行处理成本可忽略；非参数路径零额外成本。
- R3（条件性）：D4 折叠只作用于 NOT IN 臂——`build_*_key` 的「NULL 不匹配」为 JOIN/SemiJoin 共用语义，不动。
- R4（设计边界）：D3 逐位取名依赖 direct 判定的位置对齐保证（`query.rs:2461-2467` 注释）；非 direct 形态经 Projection 包装已正确，helper 只覆盖 direct 聚合顶。
- R5（非实质）：五修复合计新增定向测试函数 5 个 + 差分查询 1 条——全量时间增长在 R3 预算内（Iteration Gate 一次全量）。

**Act 实施后的风险消解**

- R1（无键判据的保守面）：判据实现为 `columns.get(pk_index)` 匹配 `ColumnType::Int`——`pk_index` 越界与列类型非 Int 两种形态均归无键（保守面：多走数据页通道，不漏行）；`ColumnType` 的 Int 变体名以 `src/storage/page_format` 实际定义为准，实读一致，Stop when 未触发。既有 Int PK 表走 PK 路径逐字节不变。
- R2（`contains_parameter` 的每次求值开销）：未引入 `contains_parameter`（见 Deviations 1——composite 本已 owned 递归，引入反而改变非参数路径）。参数求值成本 = 一次 Mutex 锁 + 一次 `Value` 克隆，仅作用于含参树；非参数路径零额外成本。
- R3（D4 折叠的作用域）：成立——折叠只在 AntiJoin 的 NOT IN 臂生效；`build_left_key`/`build_right_key` 零改动，JOIN 与 SemiJoin（IN 通道）未触碰。
- R4（D3 逐位取名的对齐假设）：helper 内置护栏——位数与聚合输出列数不一致、通配符项、列名不可解析三种非对齐形态一律回落 `get_plan_output_columns`；非 direct 形态顶层为 Projection（列已是 SELECT 清单名）亦回落。对齐假设失效时不会产生错位映射。
- R5（新增测试规模与全量预算）：实际新增定向测试函数 11 个（逐修复覆盖三态/对照形态，契约要求）+ 差分查询 1 条；全量 2057/0/2 在 Iteration Gate 预算内完成。

## Act Response

- Status: reported

### 实际改动

五个修复各自独立 RED→GREEN；白名单 6 → 1（仅剩 `ORD-NULL-ORDERING` 已知语义差异）。

**① D1 无键基表行可见性（`src/executor/scan.rs` + `src/pipeline.rs`）**

- `ScanExecutor` 新增 `tx_manager: Option<Arc<TransactionManager>>` 字段与 `with_tx_manager` 构造器（既有 `new()` 签名不变，测试/bench 调用点零改动）。
- 新增 `ScanExecutor::is_keyless(table_meta)`：PK 位（`table_meta.columns[table_meta.pk_index]`）列类型非 `ColumnType::Int` 或位置越界 → 无键表。
- 新增 `ScanExecutor::collect_keyless_rows()`：构造内部 `DataScanExecutor`（predicate `None`、scan_cap `None`、同 snapshot、同 table_meta/buffer_pool、同 tx_manager），`with_projection` 后驱动至耗尽，产出行收进 `results`——外部物化语义与既有 PK 路径同形。
- `next()` 首次执行分流：无键表走数据页通道，PK 表路径逐字节不变。
- `src/pipeline.rs` 的 `PhysicalPlan::Scan` 臂追加 `.with_tx_manager(Some(db.transaction_manager.clone()))`（`routed_db` 取库，与 DataScan 节点臂同源）。

**④ D4 NOT IN NULL 三值折叠（`src/executor/anti_join.rs`）**

- 新增 `right_has_null_key: bool` 字段。
- 三处右集装配点统一「行键含 NULL → 置位且不入 hashmap」：独立子查询 BuildRight、相关缓存命中重建、相关缓存未命中重建。
- ScanLeft 的 NOT IN 臂：右集含 NULL 键 → `continue`（任一 `x NOT IN (right set)` 为 UNKNOWN → 无行可返回）；右集空集 → 全行返回；左键含 NULL 且右集非空 → `continue`。
- NOT EXISTS 臂、SemiJoin（IN 通道）、`build_left_key`/`build_right_key` 的「NULL 不匹配 NULL」连接语义、缓存键与缓存内容结构零改动。

**② D2 TEXT 相关参数 owned 求值（`src/executor/predicate.rs`）**

- `ParameterExpression` 覆盖 `evaluate`（返回 `self.value.lock().unwrap().clone()`）——参数真值在 owned 通道可达。
- `ValueRef` 形态、`set_parameter_value`、注入通道、语句级缓存、谓词消费点零改动。

**③ D3 聚合派生表注册列（`src/parser/planner/query.rs`）**

- 新增 `PlanBuilder::derived_output_columns(query, plan)`：解包 Limit/Sort/Having 后顶层为 Aggregate（direct 形态）时按 SELECT 清单逐位取名——键位取别名（无别名取列名），聚合位取 `AggregateNode::output_columns` 的显示名；位数与聚合输出列数不一致、通配符项、列名不可解析等一切非对齐形态回落 `get_plan_output_columns`。
- 派生臂（`query.rs` 派生表列源）与 CTE 定义期捕获列源改用该 helper。`get_plan_output_columns` 各臂、`AggregateNode` 结构、直出/包装判定零改动。

**T04 无别名派生段登记收口（`src/parser/planner/subquery.rs`）**

- `declared_qualifiers` 的 Derived 无别名臂产出 `vec!["derived"]`（与派生臂实际注册名同源，两处字面量以注释互指）。Table 臂 `[别名, 基表名]` 并列语义、`declared_from_qualifiers`/`extract_subquery_table_names` 委托关系、WHERE 侧三臂的外层作用域来源（live 作用域）零改动。

**测试与白名单**

- `tests/join_test.rs` +1 函数（无键基表两表 JOIN 行数与值）。
- `tests/subquery_test.rs` +3 函数（NOT IN 三态：NULL 外键 / 右集含 NULL / 右集空集），并修正既有 `test_where_not_in_subquery_with_null` 断言（3 → 2，见 Deviation 4）。
- `tests/scalar_subquery_comparison_test.rs` +2 函数（TEXT 相关参数：AVG 形与 COUNT 形）。
- `tests/derived_scan_alias_test.rs` +3 函数（聚合派生别名键 / 无别名键 / 同内容 CTE）。
- `tests/subquery_alias_binding_test.rs` +2 函数（T04 形态 A 契约形状 + 形态 B 子查询自身 FROM 含无别名派生）。
- `tests/sql_differential_test.rs`：移除 5 条 suspected 条目（Q3-03/Q3-08/Q6-03/Q5-05/Q6-04），白名单注释同步改写为「已清偿 + 归因与修复面」；Q7 追加聚合派生查询（Q7-17）。

### Deviations from Plan

1. **D2 未实施 `contains_parameter` + 12 个 composite 覆盖（Plan 的根因描述与代码事实不符）**：Plan Context 称「composite 子表达式默认经 `evaluate_ref` 递归」。实读相反——六个 composite Expression（`CaseExpression` / `CoalesceExpression` / `CastExpression` / `BinaryArithExpression` / `IntervalArithExpression` / `FunctionExpression`）与六个 Predicate 各自覆盖 `evaluate`，其子节点一律经 owned `evaluate()` 递归；`evaluate_ref` 的递归消费点为零（`predicate.rs` 内仅默认实现自调 `evaluate_ref`，`function.rs` 内的 `evaluate_ref` 调用只出现在一条既有单测的断言里）。根因只在叶子 `ParameterExpression` 未覆盖 owned 入口。按 Plan 指引补 12 个 composite 覆盖反而会把非参数路径从「已 owned 递归」改为「refs 路径 + 逐层行→ValueRef 物化」，与 Invariant「非参数表达式/谓词求值路径逐字节保持」「非参数路径行为与分配特性不变」相悖；且 Predicate trait 无 `evaluate_ref`、无默认 `evaluate`，Plan 的「六 Predicate 否则默认实现（refs 路径）」不可实现。故只覆盖 `ParameterExpression::evaluate`——含参树在任一 composite 形态下均可达（composite 逐层 owned 递归到底）。Plan 的 R2 风险（`contains_parameter` 每次求值 O(树规模)）随该实现一并消失（非参数路径零额外成本，不受影响）。
2. **D4 追加「右集空集」例外（Plan 指引的「左键 None → continue」无条件执行）**：契约 Required behavior 明确「右集空集时全行返回」，SQL 语义同（`x NOT IN (∅)` 恒为 TRUE，`NULL` 左键亦然——空集判定不产生 UNKNOWN）。无条件 `continue` 会使该形态与契约条款 3 及 SQLite 分歧，故空集分支先行返回全行。
3. **D1 追加 `with_tx_manager` 与 pipeline 接线（Plan 指引未提 tx_manager）**：契约 Required behavior 要求无键回退通道「与单表 DataScan 通道同一可见性语义」，而单表 DataScan 节点臂传 `Some(db.transaction_manager.clone())`（I033 墓碑删除者提交态抑制）。不传时 `tx_manager=None` 等价空活跃集，在无快照（RepeatableRead 默认）下会把并发未提交删除误判为已删而漏行。故按 DataScan 通道同源接线；`new()` 签名不变，既有调用点零改动。
4. **D4 修正既有测试断言（`tests/subquery_test.rs::test_where_not_in_subquery_with_null`，3 → 2）**：该断言与其注释固化了缺陷行为（注释原文「NULL on left side of NOT IN -> row passes」），与差分白名单 Q6-04 的归因一致（SQLite 同行形返回 2 行）。D4 修复后该断言必然失败，故按正确三值语义改写并在测试内注明原值与归因。
5. **D3 同时改 CTE 定义期捕获列源（Plan 的「CTE 形态正常」前提经实测不成立）**：Plan 调查称「CTE 形态正常的原因：`CteDef.columns` 在定义期捕获（非计划输出）——CTE/视图通道不在本缺陷面」。实读：捕获本身取自 `get_plan_output_columns`（`query.rs` CTE 定义期列源），与派生臂同一缺口；RED 实测同内容 CTE 形态同样计划期拒绝 `Column not found: n`。helper 为单一列源实现，两处调用点共用 → CTE 通道的聚合形态由「计划期拒绝」变为可用。其余 CTE 形态（`get_plan_output_columns` 返回值不变的所有非 direct 聚合形态）行为零变化。Plan 的「CTE/视图通道零变化」在聚合 CTE 形态上不成立，属缺陷形态的纠正而非语义演进。
6. **T04 的可观察落点与 Plan 的 First test 形状不同（Plan 的形状修复前已 GREEN）**：Plan 的 First test 形状（外层 FROM 为无别名派生表 + 投影位子查询以 `derived.k` 限定引用外层列）修复前即 GREEN——`is_outer_reference` 的 D6 兜底（`!in_declared_list`）恰好覆盖该形态。真正的 RED 形态是「无别名派生表作为子查询自身 FROM 的基座 + 子查询内以 `derived.y` 限定引用它」：修复前该引用被误判为外层相关参数，执行期报 `correlated parameter column not found in outer row: y`（与 001-replan 探针基线记载的 `derived.k` 病理形状错误面同源）。新增测试覆盖两形态（形态 A 契约形状为回归护栏，形态 B 为 RED→GREEN 主体）。登记源修正为共享单源后，可观察落点落在 WHERE 侧臂（`extract_subquery_table_names` 同一来源）：四个病理形态由执行期错误变为正确结果。Plan 的「WHERE 侧三臂零改动」就其括注所指的外层作用域来源（live 作用域）成立且未改；内层表名单（`in_declared_list`）随登记源补全而变——这正是该缺陷族（001-replan Gate 3 记为 Minor 并移交「登记完备化」）的清偿面。
7. **`cargo fmt` 校准**：本 Act 新增代码有两处 rustfmt 偏差（超宽单行 / match 表达式形态），已 `cargo fmt` 收敛（全仓 `cargo fmt --check` 零 diff）。格式化在全量 Gate 期间发生，行为零差异；受影响面的定向套件在格式化后重跑全绿（见验证节末「后置变更定向复核」）。

### Gate 3 自检

- **Spec compliance**：
  - R1（无键行可见）：无键判据 = PK 位列类型非 Int（含越界），与 Plan 调查的「无显式 PK 走首列隐式 PK、首列非 INT 即无键」表示一致；回退通道复用 `DataScanExecutor`（零改动复用），外部物化语义与 PK 路径同形（先物化后逐行产出）。差分三条目移除后双引擎一致 + 定向 JOIN 函数全绿。✓
  - R2（参数保真）：`ParameterExpression::evaluate` 覆盖后 owned 通道取真值；差分 Q5-05 移除后双引擎一致（106 行）+ 两个 TEXT 相关定向函数全绿；非参数路径未触碰（Deviation 1）+ 全量 2057/0/2 零回归。`ValueRef` 形态、trait 公共签名、注入通道、缓存零改动。✓
  - R3（NOT IN 折叠）：左键 NULL 行不返回、右集含 NULL 键时无行可返回、右集空集全行返回三态均有定向断言；IN 通道与 NOT EXISTS 臂零改动（`anti_join.rs` 既有单测 `missing_outer_column_reports_error_naming_column` 与 `subquery_test` 的 IN/NOT EXISTS 族全绿）。✓
  - R4（聚合派生引用）：direct 形态键位别名生效、聚合位维持显示名、逐位对齐（位数不符即回落）；非 direct 形态经 Projection 包装维持既有列源；聚合查询自身通道（HAVING/ORDER/表头）零改动——`explain_test` 10/0 零校准。✓
  - binding R4（登记完备）：无别名派生的隐式注册名 `derived` 在作用域登记中可达，登记名与实际注册名单源一致；既有碰撞回归 23 项零回归（`subquery_alias_binding_test` 25/0）。✓
  - A7 硬边界：既有 2046 基线零失败（全量 2057 = 2046 + 本 Cycle 新增 11 个定向函数）；非缺陷输入行为零变化；EXPLAIN 计划形状零校准；白名单恰移除 5 条、机制与结构自检测试不变。✓
- **Code quality**：改动面 = 5 个产品文件（`scan.rs` / `pipeline.rs` / `anti_join.rs` / `predicate.rs` / `query.rs` / `subquery.rs` 六处，其中 `pipeline.rs` 仅一行接线）；存储 / 网络 / `PhysicalPlan` 词表零改动；`cargo clippy --lib` 无新增警告；`cargo fmt --check` 全仓零 diff；无新增身份型证据机制、判定层或框架外工具；无 `unsafe`、无 per-row 泄漏。
- **单源核对**：无键判据单点（`is_keyless`）、右集 NULL 键标志单点（三装配点同源置位）、聚合派生列源单点（`derived_output_columns` 两调用点共用）、无别名派生隐式名两处字面量互指（`query.rs` 派生臂 / `subquery.rs::declared_qualifiers`）。
- 已修复：Deviation 7（rustfmt 校准）。
- 遗留 Minor：无。
- Experience Candidates：None。

### 验证（全部真实运行）

- **RED（修复前，逐项实跑）**：
  - D1：`cargo test --test join_test test_join_keyless_base_tables_visible` → FAILED，断言 `left: 0, right: 2`。
  - D4：`cargo test --test subquery_test t03_d4` → 2 FAILED——最小复现返回 `[[2],[4]]`（应 0 行）；右集含 NULL 键返回 2 行（应 0 行）。
  - D2：`cargo test --test scalar_subquery_comparison_test t03_d2` → 2 FAILED，均 `left: []`。
  - D3：`cargo test --test derived_scan_alias_test t03_d3` → 2 FAILED，均 `Plan error: Column not found: n`（派生形态与同内容 CTE 形态）。
  - T04：`cargo test --test subquery_alias_binding_test t04` → 1 FAILED，`Execution error: correlated parameter column not found in outer row: y`（形态 B；形态 A 修复前后均 GREEN，见 Deviation 6）。
- **GREEN 定向（最终代码，逐目标计数）**：`sql_differential_test` 10 / `subquery_test` 46 / `scalar_subquery_comparison_test` 16 / `table_alias_resolution_test` 19 / `cte_test` 14 / `view_test` 19 / `self_join_test` 6 / `join_output_resolution_test` 19 / `explain_test` 10 / `derived_scan_alias_test` 26 / `join_test` 8 / `subquery_alias_binding_test` 25 —— 合计 218 passed / 0 failed。
- **差分网**：`sql_differential_test` 10/0；白名单 5 条 suspected 条目移除后全绿（Q3-03 710 行、Q3-08 500 行、Q6-03 185 行、Q5-05 106 行、Q6-04 0 行，与 SQLite 一致），Q7-17 聚合派生新增双引擎一致；`ORD-NULL-ORDERING` 保留；白名单结构自检测试不变。
- **EXPLAIN 计划形状**：`explain_test` 10/0，零校准。
- **全量（M19 分批一次）**：14 批 = `--lib` + 125 个集成目标按字典序每批 ≤10 目标，每批独立 `cargo test … -- --test-threads=1`、批间顺序执行、无无过滤全量调用、无默认并行；全部批次 rc=0，**合计 2057 passed / 0 failed / 2 ignored**（002 基线 2046/0/2 + 本 Cycle 新增 11 个定向函数，2 个 ignored 为既有忽略项）。目标覆盖经 ran-vs-want 清单 diff 核对：ran 125 = want 125、唯一目标 125——不重不漏。分批计数：

  | 批 | 目标（组） | passed/failed |
  |---|---|---|
  | 1 | lib | 407/0 |
  | 2 | aggregate_extension_test…busy_timeout_test（10） | 148/0 |
  | 3 | cache_perf_test…concurrent_test（10） | 182/0（2 ignored） |
  | 4 | connection_limit_test…ddl_crash_recovery_test（10） | 147/0 |
  | 5 | default_declaration_test…explicit_tx_test（10） | 161/0 |
  | 6 | expression_e2e_test…insert_select_test（10） | 100/0 |
  | 7 | isolation_level_test…mvcc_commit_test（10） | 89/0 |
  | 8 | mvcc_record_test…parser_test（10） | 95/0 |
  | 9 | pg_integration_test…projection_expression_test（10） | 203/0 |
  | 10 | projection_test…runtime_test（10） | 64/0 |
  | 11 | scalar_function_test…sql_differential_test（10） | 141/0 |
  | 12 | storage_test…tx_statement_test（10） | 158/0 |
  | 13 | update_expression_test…wal_handle_test（10） | 122/0 |
  | 14 | wal_integration_test…write_type_conformance_test（5） | 40/0 |

- **后置变更定向复核**：全量 Gate 运行期间仅发生 `cargo fmt` 收敛与两处注释订正（`query.rs` / `subquery.rs`）——`git diff -w` 语义审计确认无行为改动；受影响的 12 个定向套件在格式化后重跑 → 218 passed / 0 failed（计数与格式化前逐目标一致）。
- **构建 / 静态检查**：`cargo clippy --lib` 无新增警告；`cargo fmt --check` 全仓零 diff。

### Persisted Evidence

None required（计划为 `none`；全部验证可低成本重跑——差分网与定向套件为分钟级，全量按 M19 分批可重跑；临时诊断探针 `tests/zz_t04_probe.rs` / `tests/zz_t04_probe2.rs` 已删除，其关键输出摘录于 Gate 3 与 Deviation 6）。

### 未解决问题

- 白名单余 `ORD-NULL-ORDERING`（已知语义差异，契约 Non-goal，不在本 Iteration 清偿面）。
- Deviation 5（CTE 通道聚合形态由计划期拒绝变为可用）与 Deviation 6（WHERE 侧四个病理形态由执行期错误变为正确结果）为可观察行为变化面，请 Review 重点审计这两处。
- ⑤ 谓词 × IN 子查询 AND 组合维持既有计划期拒绝（契约 Non-goal）。
- 执行器批量接口 / 类型化 API（MS31 / MS32）与 `condition_column_index` 等其余执行期按名点收口（I080 后续批次）不在本 Iteration 范围。

---

## Plan Review

- Review Result: accepted
- Reviewer: openspec-plan（2026-10-21 本会话）

**Findings**

1. **[核定——通过] D1 无键回退（R1）**：`is_keyless`（`scan.rs:69-74`）= PK 位非 `ColumnType::Int` 或越界（`.get` 兜底）✓；`collect_keyless_rows` 复用 `DataScanExecutor`（predicate/scan_cap None、同 snapshot/table_meta/buffer_pool/tx_manager，`with_projection` 后物化进 `results`——外部形状与 PK 路径同形）✓；`next()` 分流下 PK 路径仅重排缩进零语义变化 ✓；`pipeline.rs` Scan 臂 `with_tx_manager(Some(db.transaction_manager.clone()))` 与 DataScan 臂同源（Deviation 3 成立——不传则 I033 墓碑抑制判空活跃集，并发未提交删除会漏行，属可见性语义必要接线）✓；`new()` 签名不变、`data_scan.rs` 零改动 ✓。
2. **[核定——通过] D4 三值折叠（R3）**：`right_has_null_key` 于三装配点统一「行键含 NULL → 置位且不入 hashmap」（独立 BuildRight / 缓存命中重建 / 缓存未命中重建；相关延迟分支初始化 false）✓；ScanLeft NOT IN 臂四态核对——右集含 NULL 键 → `continue`（无行可返回）；右集空集 → 全行返回（含左键 NULL——`x NOT IN (∅)` 恒 TRUE，Deviation 2 成立且为契约 Required behavior 第 3 款明文）；左键 NULL 且右集非空 → `continue`；非 NULL 键未命中 → 输出 ✓；NOT EXISTS 臂、SemiJoin、`build_*_key` 零改动 ✓。
3. **[核定——通过；偏差分类 Plan 错误] D2/Deviation 1（composite 前提）**：独立验证成立——`Predicate` trait 无默认 `evaluate`（`predicate.rs:30` 抽象方法，六谓词各自实现）；六个 composite Expression（:535/:581/:689/:777/:841）与 `FunctionExpression`（`function.rs:103`）均覆盖 owned `evaluate` 且子节点经 `evaluate()` 递归（抽查 `CastExpression::evaluate` :689-693 直调 `self.expr.evaluate(row)`，其 `evaluate_ref` 反而经 owned 转换委托）。Plan Context 的根因描述（「composite 默认经 `evaluate_ref` 递归」）系只读 trait 默认实现未读各 impl 的调查错误；最小修复（仅 `ParameterExpression::evaluate` 覆盖 +12 行）正确，按 Plan 指引补 12 个 composite 覆盖会把非参数路径从 owned 递归改为 refs 物化、违反本 Cycle Invariant——Act 偏离方向正确且必要。
4. **[核定——通过] D3 helper（R4）**：`derived_output_columns`（`query.rs:68-118`）解包 Limit/Sort/Having → 非 Aggregate / `extract_select_body` 失败 / 通配符项 / 列名不可解析 / 位数不符一律回落 `get_plan_output_columns`（保守面完整）；键位别名 ?? `expr_to_column_name`、聚合位取 `agg.output_columns[key_count + agg_index]`（direct 判定保证位置对齐）；`extract_select_body` 为既有 helper（`ast.rs:25`）✓；两调用点（派生臂 + CTE 定义期捕获）✓；`get_plan_output_columns` 各臂与 `AggregateNode` 结构零改动 ✓。
5. **[核定——通过；偏差分类 Plan 遗漏] D4/Deviation 4（既有断言 3→2）**：原注释逐字固化缺陷推理（「In practice: NULL on left side of NOT IN -> row passes」）；新注释逐行归因（Alice/Carol/Dave 命中排除、Bob/Eve 返回、Frank NULL→UNKNOWN 不返回），与 Q6-04 差分归因及 SQLite 同行形一致——该断言属缺陷冻结面，改写是修复的必要结果，非断言校准违规。
6. **[核定——通过；偏差分类 Plan 错误] T04/Deviation 6（RED 形态与落点）**：Plan 的 First test 形态修复前经 D6 兜底（`!in_declared_list`）已 GREEN——调查不足；真实 RED 形态 B（无别名派生作为子查询自身 FROM 基座 + `derived.y` 引用被误判外层参数 → 执行期错误）与 001-replan 探针记载的病理族同源。落点经共享单源（`declared_qualifiers` 同时供 `extract_subquery_table_names`）使 `in_declared_list` 补全——四个病理形态由执行期错误变为正确结果，正是 001-replan Gate 3 移交「登记完备化」的清偿面；「WHERE 侧外层作用域来源（live 作用域）零改动」的契约括注意图保持成立。两形态测试（A 护栏 + B 主体）覆盖正确。
7. **[核定——通过] Deviation 5（CTE 列源共用）**：Act RED 实测推翻 Plan「CTE 形态正常」前提（该前提源自 000 分诊记录，未实测）；helper 单源两调用点共用使聚合 CTE 形态由计划期拒绝变为可用——缺陷形态纠正，其余 CTE 形态列源返回值不变（helper 回落分支）。
8. **[核定——通过] Preserve/Forbidden 面**：存储 / 网络 / `data_scan.rs` / `plan.rs`（PhysicalPlan 词表）不在 git status ✓；`ValueRef` 形态、`Expression`/`Predicate` trait 公共签名、注入通道、语句级缓存零改动 ✓；白名单恰 1 条（`ORD-NULL-ORDERING` + `CLASS_KNOWN`），移除的 5 条以归因注释保留修复面记录，结构自检测试不变 ✓；Q7-17 新增 ✓；11 个新定向函数 + 1 处缺陷冻结断言改写为全部测试面变化。
9. **[Minor——记录] 注释标签笔误**：`subquery.rs:490` 注释「T03-D4 无别名派生的隐式注册名」应为 T04（D4 为 NOT IN 项）——纯注释标签，非阻塞。
10. **[Minor——记录] 白名单归因注释保留已证伪的 000 期观察**：③ 段「同内容 CTE 形态正常」为分诊期记录，Deviation 5 的 RED 已证伪（CTE 同样拒绝）；注释以「首跑分诊记录」框架呈现，历史现场与当前事实的边界可辨——纯记录性，非阻塞。

**Acceptance Gaps**

None——六项 Acceptance 全部满足：`sql-semantics-conformance` R1（白名单 ① 三条目移除后差分双引擎一致〔710/500/185 行〕+ 定向 keyless JOIN 函数）、R2（Q5-05 移除后一致〔106 行〕+ 两个 TEXT 相关定向 + 非参数路径全量零回归）、R3（Q6-04 移除后一致〔0 行〕+ 三态定向 + IN/NOT EXISTS 零回归）、R4（定向别名键函数 + Q7-17 差分一致 + `explain_test` 零校准）；`sql-semantic-binding` R4（T04 两形态函数 + 既有碰撞回归 25/0）；A7（全量 2057/0/2 = 2046 + 新增 11，采信 Act M19 分批实测——报告后覆盖表面未变化，本 Review 全程只读 git 操作核对工作区一致）。

**Evidence**

- 完整 diff 独立阅读：`scan.rs`（108 行）、`anti_join.rs`（73 行）、`predicate.rs`（+12）、`pipeline.rs`（Scan 臂 +5）、`query.rs`/`subquery.rs` 增量段（D3 helper 全文、`declared_qualifiers` Derived 臂、CTE 列源调用点）。
- 关键前提独立验证：composite owned 覆盖 grep 实证（`fn evaluate` 14 处覆盖点 / `Predicate` trait :30 无默认）；`extract_select_body` 既有（`ast.rs:25`）；白名单区域实读（恰 1 条 + 归因注释 + 自检测试在场）；缺陷冻结断言改写前后对照（`subquery_test.rs` diff）。
- 采信 Act Response 验证结论（未失效）：逐项 RED 实跑记录、GREEN 定向 218/0、差分网 10/0（移除条目后行数与 SQLite 一致）、`explain_test` 10/0、全量分批 14 批 2057/0/2、clippy/fmt。

**Follow-up Decision**

接受：六项 Acceptance 全部满足、无阻塞 finding。七条 Deviations 中——Deviation 1/5/6 的 Plan 前提错误均经独立验证确认（composite 结构、CTE 列源、D6 兜底覆盖面），Act 的偏离方向由代码事实与契约不变量唯一确定且全部核实通过；Deviation 2/3 为契约条款与可见性语义的必要落实；Deviation 4 为缺陷冻结面的必要改写；Deviation 7 格式化无争议。两条 Minor（Findings 9/10）为注释层记录，不构成返工依据。Iteration 003 为 Map 最后一项，T01-T04 全部完成，change 范围内无剩余工作——按用户 2026-10-21 指令（「接收的话就归档收尾」）进入收尾。

**Iteration Plan Update**

None

**Next Cycle**

None

**Next Iteration**

None（Map 全部 Iteration 完成；change 进入收尾归档）
