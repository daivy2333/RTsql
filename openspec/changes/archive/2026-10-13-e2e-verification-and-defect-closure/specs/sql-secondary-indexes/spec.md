## MODIFIED Requirements

### Requirement: 二级索引等值查询路由可达

非 PK 等值谓词经二级索引路由（`IndexScanAll`）SHALL 产出**与全表扫描（DataScan）严格相等的结果集——双向：不重复、不遗漏**，任意重复键规模成立（多叶等值连串、≥2 层树）。同值键跨越「小于查找键的分隔键」边界散布的树形态下，全部命中条目 SHALL 可达（范围下探：首 ≥K 分隔键的前一子树至末 ≤K 分隔键的子树）。失败语句（含 UPDATE 旧键删除路径）SHALL 对索引零副作用（报错后索引条目集不变）。范围谓词、复合谓词（不路由 IndexScanAll 的形态）与唯一键索引既有结果 SHALL 零变化。

#### Scenario: 多叶等值连串双向严格相等

- **GIVEN** 表 t1（id INTEGER PRIMARY KEY, n INTEGER）30000 行 n∈1..100 重复键分布，`CREATE INDEX i1 ON t1(n)` 前 DataScan 基线逐 k 记录
- **WHEN** 建索引后逐 k `SELECT COUNT(*), COUNT(DISTINCT id) FROM t1 WHERE n = k`
- **THEN** COUNT(*) 与 COUNT(DISTINCT id) 均等于基线（失败边界——修复前：COUNT 虚高 [实测 +22944 总量] 且 DISTINCT 漏行 [实测 476 条漏发]）

#### Scenario: 边界子树条目的 UPDATE 探针可达

- **GIVEN** 同上建索引后，取基线中存在但索引扫描曾漏发的行 id
- **WHEN** `UPDATE t1 SET n = 101 WHERE id = <该行>`
- **THEN** 成功执行且旧键条目删除（失败边界——修复前：`Execution error: Key not found`，且失败后索引计数被 abort 路径改变——零副作用被违反）

#### Scenario: 失败语句索引零副作用

- **GIVEN** 同上建索引后的表，记录某键 k 的 COUNT/DISTINCT 基线
- **WHEN** 构造一条执行失败的 UPDATE（如与唯一约束/键碰撞冲突）后重新计数
- **THEN** `WHERE n = k` 的 COUNT 与 DISTINCT 与失败前一致（失败边界——修复前：abort 重插改变索引内容）

#### Scenario: 既有回落与唯一索引零回归

- **GIVEN** 同上
- **WHEN** 范围谓词 `WHERE n > k`、复合谓词 `WHERE n = k AND id <= x`、唯一键索引点查
- **THEN** 全部行为与修复前一致

### Requirement: 二级索引写路径条目维护

（既有 Requirement 主体保持——四执行器与回滚还原、恢复两态一致；本次追加树遍历与叶链保真面）追加：树遍历原语 `IndexManager::scan_all` SHALL 对任意层树返回全部条目（内部页感知、不依赖叶子链指针、不以 page 0 为终止条件）；叶子链指针（`next_leaf_page_id`）SHALL 在任何叶内条目重排（非末位插入触发的整页重建）后保持不变。同值键写路径（INSERT/UPDATE/DELETE 的 `insert`/`delete_exact`）SHALL 对跨界散布形态的全部条目正确维护（删除可达、重插定位正确）。

#### Scenario: 多层树全量枚举完整

- **GIVEN** 30000 行表 + 二级索引（树 ≥2 层）
- **WHEN** 经内部全量枚举通道（无 WHERE 全表 UPDATE/DELETE、Scan 执行器）枚举
- **THEN** 枚举条目数 = 表行数（失败边界——修复前：`scan_all` 对内部根树返回空/碎片 [实测 190/30000]，全表 DML affected_rows 失真或整语句失败）

#### Scenario: 叶子链在乱序插入后保真

- **GIVEN** 以非升序键值持续插入形成的索引树（多次触发非末位插入与分裂）
- **WHEN** 逐叶检查 `next_leaf_page_id`
- **THEN** 除最右叶外全部非零且构成完整链（失败边界——修复前：~90% 叶子链指针被 `shift_slots_right` 重建清零）

#### Scenario: 全表 DML 多层树行数正确

- **GIVEN** ≥3 层 PK 树的大表（乱序主键分布）
- **WHEN** `UPDATE t SET …`（无 WHERE）/ `DELETE FROM t`（无 WHERE）
- **THEN** `affected_rows` = 实际行数且全部行被触达（失败边界——修复前：affected≈0 或 `Abort failed: Key not found` 整语句失败）
