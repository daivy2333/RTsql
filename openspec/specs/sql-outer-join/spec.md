# sql-outer-join Specification

## Purpose
约束 OUTER JOIN 三态的计划期可达与结果语义：LEFT/RIGHT/FULL 的匹配组合、未匹配侧 NULL 补齐与 ON 谓词三值 NULL 语义（Unknown 视为不匹配），非等值 ON 经 NLJ 通道承载，外连接结果与 WHERE/ORDER BY/聚合的组合语义明确，既有 INNER 面与 NATURAL/CROSS APPLY 等拒绝面零回归。来源：MS25（change `2026-09-26-ms25-sql-read-surface`，SQLite 语义对齐）。

## Requirements

### Requirement: LEFT OUTER JOIN 可达与语义

`FROM 左表 LEFT [OUTER] JOIN 右表 ON <谓词>` SHALL 计划期接受并端到端可达：ON 谓词为真的行对按 INNER 语义组合输出；左行无匹配右行时 SHALL 输出该左行并以 NULL 补齐右表列。ON 谓词的 NULL 求值 SHALL 按 SQL 三值语义处理（Unknown 视为不匹配，走补 NULL 分支）。实现路径（等值 ON 经 Hash 通道扩展或统一经 NLJ 通道）随 change design 定稿，结果语义 SHALL 与本 Requirement 一致。

#### Scenario: LEFT JOIN 匹配行组合

- **GIVEN** 表 `l(id INT PRIMARY KEY)` 含 id=1,2,3；表 `r(rid INT, val VARCHAR)` 含 (1,'a'),(2,'b')
- **WHEN** `SELECT l.id, r.val FROM l LEFT JOIN r ON l.id = r.rid`
- **THEN** 输出 3 行：`(1,'a')`、`(2,'b')`、`(3,NULL)`

#### Scenario: LEFT JOIN NULL 连接键不匹配

- **GIVEN** 表 `ln(id INT, k INT PRIMARY KEY)` 含 (1,1),(NULL,2)；表 `rn(rid INT, val VARCHAR)` 含 (1,'a')（夹具注记：主键列 `k` 为行可见性锚——JOIN 输入经 `ScanNode` 索引遍历，有主键表中键值为 NULL 的行不入索引不可见（MS10-T05 键位语义），NULL 落在非键位连接列 `id` 上使「NULL 连接键不匹配」可被见证；无主键表全行经数据页可达，既有 INNER 面同一边界）
- **WHEN** `SELECT ln.id, rn.val FROM ln LEFT JOIN rn ON ln.id = rn.rid`
- **THEN** 输出 2 行：`(1,'a')` 与 `(NULL,NULL)`（NULL 连接键不匹配任何行，含 NULL 连接键的右行）

#### Scenario: LEFT JOIN 非等值 ON 可达

- **GIVEN** 表 `l(v INT)` 含 (1),(5)；表 `r(rid INT, val VARCHAR)` 含 (3,'x')
- **WHEN** `SELECT l.v, r.val FROM l LEFT JOIN r ON r.rid > l.v`
- **THEN** id=1 行匹配 (3,'x')，id=5 行无匹配输出 `(5,NULL)`（经 NLJ 通道）

### Requirement: RIGHT 与 FULL OUTER JOIN 可达与语义

`RIGHT [OUTER] JOIN` SHALL 输出所有匹配组合，并为无匹配右行以 NULL 补齐左表列；`FULL [OUTER] JOIN` SHALL 同时为无匹配左行补 NULL 右列与无匹配右行补 NULL 左列。三态的谓词求值、NULL 键语义与 LEFT 一致。RIGHT/FULL 的交付以 change design 的 NLJ 改造成本调查结论为准；调查裁定超预期拆出时，本 Requirement 相应收窄并经用户批准（split signal 记录于 design）。

#### Scenario: RIGHT JOIN 未匹配右行补 NULL（未匹配左行丢弃）

- **GIVEN** 表 `l(id INT, name VARCHAR)` 含 (1,'a'),(2,'b')；表 `r(rid INT, val VARCHAR)` 含 (1,'x'),(9,'z')
- **WHEN** `SELECT l.id, r.val FROM l RIGHT JOIN r ON l.id = r.rid`
- **THEN** 输出 2 行：`(1,'x')`（匹配组合）与 `(NULL,'z')`（右行 9 无匹配，左列补 NULL）；左行 (2,'b') 无匹配且 RIGHT JOIN 不保存左侧行——SHALL NOT 出现（标准 SQL/SQLite 语义；本场景 THEN 原误写 3 行（含左行 (2,'·')），系起草错误——与 Requirement 正「仅为无匹配右行补齐」及 SQLite 对齐基准矛盾，Plan Review 2026-09-27 勘误，实现按 LEFT 交换重写承载）

#### Scenario: FULL JOIN 两侧未匹配补齐

- **GIVEN** 表 `l(id INT)` 含 (1),(2)；表 `r(rid INT, val VARCHAR)` 含 (1,'x'),(9,'z')
- **WHEN** `SELECT l.id, r.val FROM l FULL JOIN r ON l.id = r.rid`
- **THEN** 输出 3 行：`(1,'x')`、`(2,NULL)`、`(NULL,'z')`

### Requirement: 外连接组合面

外连接结果行流 SHALL 与既有执行节点组合：外层 WHERE 谓词（含引用补 NULL 列的过滤）、ORDER BY / LIMIT、聚合与 GROUP BY 均按既有语义作用于补 NULL 后的行。外连接形态出现在 IN 子查询中 SHALL 维持既有 `InSubqueryJoinUnsupported` 计划期点名拒绝（MS17-T02 语义不变）。

#### Scenario: WHERE 过滤作用于补 NULL 行

- **GIVEN** 同 LEFT JOIN 匹配行场景种子
- **WHEN** `SELECT l.id FROM l LEFT JOIN r ON l.id = r.rid WHERE r.val = 'a'`
- **THEN** 仅输出 id=1（补 NULL 行被 WHERE 滤除，标准外连接行为）

#### Scenario: ORDER BY 作用于外连接结果

- **GIVEN** 同 LEFT JOIN 匹配行场景种子（右表排序列与左表列名不冲突）
- **WHEN** `SELECT l.id, r.val FROM l LEFT JOIN r ON l.id = r.rid ORDER BY val DESC`
- **THEN** 按 val 排序输出（裸列名键解析到 JOIN 输出形状，NULL 排序位置随既有比较语义，行集为 3 行）

#### Scenario: IN 子查询中 JOIN 形态维持拒绝

- **GIVEN** 表 `l(id INT)`、`r(rid INT)` 存在
- **WHEN** `SELECT id FROM l WHERE id IN (SELECT rid FROM r LEFT JOIN l ON true)`
- **THEN** 计划期点名拒绝（`InSubqueryJoinUnsupported`；拒绝语义与变更前一致——变更前该形态经 planner JOIN 臂拒绝为 `UnsupportedJoinType`，本能力落地后外连接计划可达、由子查询 JOIN 诚实化拒绝臂接管，文案详见 `in-subquery-join-rejection` 修订）

### Requirement: 既有 JOIN 语义零回归

本能力 SHALL NOT 改变既有行为：INNER JOIN（纯等值 Hash 路径与非等值 NLJ 路径）无 WHERE 路径的结果与计划形状逐字节不变；`UnsupportedJoinType` 对其余 JOIN 形态（NATURAL、CROSS APPLY 等）的点名拒绝保持；INNER 与 USING 组合的现状行为不触碰；既有测试零修改通过，**校准白名单**（replan 裁定 2026-09-27，方案 (a)——D2 统一解禁的必要后果）：① `tests/nested_loop_join_test.rs::where_over_non_equi_join_stays_rejected`（锁定 WHERE×JOIN 拒绝面的 T13 见证）改写为本能力的可用性见证（前提退役：其锁定的拒绝面由 D2 解除）；② `tests/subquery_test.rs::test_in_subquery_where_join_keeps_existing_rejection` 期望文案校准为 `InSubqueryJoinUnsupported`（拒绝保留，文案统一，详见 `in-subquery-join-rejection` 修订）。

#### Scenario: INNER JOIN 零回归

- **WHEN** 执行既有 INNER JOIN 形态（等值与非等值、含 `tests/nested_loop_join_test.rs` 既有矩阵）
- **THEN** 结果与计划形状与本变更前一致

#### Scenario: 其余 JOIN 形态拒绝保持

- **WHEN** 执行 `NATURAL LEFT JOIN`、`CROSS APPLY` 等未支持的外连接/非 INNER 形态（注：`NATURAL JOIN`/`JOIN USING` 的 INNER 形态现状为 `MissingOnClause` 拒绝——`Inner(_)` 臂先于形态分类，维持不变）
- **THEN** 计划期 `UnsupportedJoinType` 点名拒绝（与变更前一致）
