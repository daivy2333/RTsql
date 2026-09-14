# design — MS16 正确性收口第二批

> 规划：openspec-plan 2026-09-12。Gate 1 已批准（2026-09-12：R1-R5 基线 + 三项集中决策；范围随后按用户指示扩展——探针新发现并入直接修复、不登记 improvement，原话见 proposal）。

## D1 路由判定规则（键列类型感知，I046 方向 B）

**当前行为**（`src/parser/planner/query.rs:505-609`）：

- 简单形态 `WHERE f = 5`（Float 键列）：`extract_pk_from_where`（query.rs:929）对 Int 字面量返回 `Some(key)` → `IndexScan` 点查空索引 → 静默空集（探针实证 `rows:[]` exit 0）。
- AND 形态 `WHERE f = 5 AND n = 1`：`extract_pk_from_where` 只匹配顶层 Eq 返回 None → 非 PK 臂 → `has_pk_equality` 真 + `has_non_keyable_pk_literal_leg` 假（Int 字面量可键控）→ `Filter(Scan)`（query.rs:560-568）索引遍历，无键行被丢。
- `WHERE s = 5`（String 键列 + Int 字面量）：同经 IndexScan/Filter(Scan)，空集——跨类型 equals false，结果巧合正确但路径不可靠。

**目标行为**：路由可靠性判定从「字面量可键控性」升级为「键列声明类型」：

- 键列声明类型为 **Int** ⇒ 索引路由可靠（配合 D3 写入强制，键列中不存在非 Int 可匹配值；存量越界行见 D5 兼容性）。
- 键列声明类型为 **非 Int**（Float/String/Bool）⇒ 该表所有行必为无键行（存储值类型 = 列声明类型，`to_key()` 仅 Int 有值），键位等值全形态（简单/AND/反向）统一分流到既有数据页臂：无 OR → 谓词下推 `DataScan(Some(predicate))`（query.rs:588-607）；含 OR → `Filter(DataScan)`（query.rs:569-587）。
- 行内求值正确性由 `Value::equals`（value.rs:118-137）保证：`equals(Int(5), Float(5.0))` 双向隐式转换为 true；Int vs String/Bool 为 false（结果不变场景）。

**两处判定门**（helper 形态建议 `pk_type_known_non_int(table) -> bool`，`matches!(primary_key_types.get(table), Some(ColumnType::Int))` 取反）：

1. extract 臂门：键列类型已知非 Int 时跳过 `extract_pk_from_where`（视同 None），流入非 PK 臂。
2. `Filter(Scan)` 臂门：`has_pk_eq && !pk_type_known_non_int && !has_non_keyable_pk_literal_leg`（query.rs:560 条件扩展）。

类型未知（未注册）→ 不分流，回退既有行为。实际只有 `pipeline.rs::register_table`（pipeline.rs:985-1009）为真表注册；派生表别名注册（query.rs:150，pk=""）的 primary_keys 条目使键位等值判定天然不匹配，类型条目惰性。子查询内单表 WHERE 经同一 `build_query` 递归，自动获得同一判定门（构造性覆盖）。

## D2 键列类型传递通道（加性）

`PlanBuilder`（`src/parser/planner/mod.rs:98-111`）新增字段 `primary_key_types: HashMap<String, ColumnType>`（`storage::page_format::ColumnType`，变体 Int/String(u16)/Float/Bool）+ 注册方法（如 `set_pk_column_type(name, ColumnType)`，lowercase 键与 `register_table` 一致）。**不改动既有 `register_table` 签名**（约 9 个调用点：pipeline 1 + query.rs 派生表 1 + 单测 7，保持零扰动）。

接线点 `pipeline.rs:996-1003`：`table_meta.columns: Vec<(String, ColumnType)>` 中按键列名（`table_meta.pk_column`，隐式首列主键由 TableMeta 供给）取类型后调用注册方法；pk_column 为空（派生别名）不设置。

选择理由：加性通道改动面最小、单测调用点零修改；注册遗漏时的降级行为（类型未知 = 既有路由）对 Int 键列表无正确性影响（主流形态），非 Int 键列表仍有 MS15-T01 字面量护栏兜底。备选「扩展 register_table 签名」被否：派生表注册点无类型信息、测试调用点批量扰动，收益仅单点注册。

plan cache 无影响：类型来自 plan 期 registration（与既有 primary_keys 同源同生命周期），MS06-T02 缓存键为 SQL canonical 文本，表类型在库生命周期内不变。

## D3 键列写入类型强制（根因收口）

**当前行为**：INSERT / UPDATE 不校验值类型与列类型（`build_insert` 无校验、序列化按值打 tag，tuple.rs:55-115）——`INSERT (5.0, 1)` 入 Int 键列成功落库为无键行（2026-09-12 探针实证）；`UPDATE SET id = 5.0` 走 I037 分支删旧键条目，行静默转无键。

**目标行为**：执行器前置校验（先于任何写入，与 INSERT 先查后写模式同构）：

- `InsertExecutor`（`src/executor/insert.rs:92-112`，键位值 `row_values[self.pk_index]`）：显式列序由既有 pk_index 解析覆盖，无需重复定位。校验先于 DuplicateKey 预检（非法类型无需访问索引）。
- `UpdateExecutor`（`src/executor/update.rs`）：`column_name == pk_column` 时校验 `new_value`；位于 Step 1（KeyNotFound 语义保持在前——目标行不存在仍报 `KeyNotFound`）之后、Step 6 首次写入之前；与 D4 碰撞预检同区。
- 判定：键列声明类型 Int（schema 中键列 `ColumnType::Int`）且值非 `Value::Int` / `Value::Null` → 拒绝。
- 错误：新加性变体 `StorageError::KeyTypeMismatch { column, expected, actual }`（thiserror，error.rs 既有加性先例 NotADatabase 等），Display 点名键列、期望 INT 与实际类型（如 `key column 'id' expects INT, got Float`）；`Response::Error` → CLI `sql_failure_status` → exit 3（cli/mod.rs:405 既有通用映射，无特判需求）。
- **只强制 Int 键列**（正确性最小面）：非 Int 键列经 D1 分流后对任意存储值类型结果正确（equals 语义），Float 键列接受 Int 值等宽松行为保持（spec 行为保持锚点）。非键列不做类型校验（无正确性影响，留类型系统域）。
- NULL 保持既有无键行语义；`UPDATE SET id = NULL` 继续走 I037 删旧键分支（update-index-maintenance R1 不受影响）。

**兼容性**：本变更后 Float/String/Bool-into-Int-键列 INSERT/UPDATE 显式报错（此前静默接受）。restore 重放历史越界 dump 文本将响亮失败（预发布产品可接受，存量越界行不迁移、经非键谓词仍可达）；import 不受影响（`csv_value` 按列声明类型转换，lifecycle.rs:502——`5.0` 入 INT 列在 import 侧本就报 `invalid INT value`）；dump 侧 `sql_literal` 不变。

**既有测试校准（Plan Review BH-1 裁定，2026-09-12）**：`tests/expression_e2e_test.rs::negative_number_literal_persists`（MS11-T01 R5/S1，I040 负数字面量）建表 `CREATE TABLE t (v INT, s STRING)`（首列 v 为隐式键列、声明 Int）并断言 `INSERT INTO t VALUES (-1.5, 'y')` 落库——与 R3 强制冲突（全量唯一确定性失败，Plan 兼容性预测「预计不存在」有误）。校准方案：负 Int 行（`(-1, 'x')`）保持原表原断言逐字节不动；负 Float 行移入 Float 键列表 `tf(f FLOAT, s STRING)`（`INSERT INTO tf VALUES (-1.5, 'y')`），重开断言两表各自行集——I040 的负 Int/负 Float 字面量折叠覆盖完整保留（负 Float 落 Float 键列同时补足非 Int 键列持久化覆盖）；校准在 `key-column-type-conformance` delta spec 按 update-index-maintenance R2「T8-R2 校准」先例记录。

## D4 rekey 索引维护（I047）

**当前行为**（`src/executor/update.rs:129-140`）：`SET id = 7 WHERE id = 5` 走 else 臂 `index_manager.update(&self.key, new_row_id)`——旧键条目残留指向新版本、新键 7 无条目（新键点查空集、旧键 INSERT 被 DuplicateKey 误拒、恢复重建后两态不一致）。

**目标行为**：Step 7 三分支化 + 写入前碰撞预检：

1. **前置块**（Step 1 之后、Step 6 首次写入之前；与 D3 类型校验同区）：
   - D3 类型校验（键列非 Int 值拒绝，NULL 放行）。
   - 碰撞预检：`column_name == pk_column` 且 `new_value.to_key() == Some(new_key)` 且 `new_key.as_bytes() != self.key` 时 `index_manager.search(new_key)` 命中 → `Err(StorageError::DuplicateKey)`（INSERT 同文同 variant，insert.rs:110 先例）。新键 == 旧键不预检（同键更新不是碰撞）。
2. **Step 7 三分支**：
   - 新值 `to_key() == None`（NULL）→ `index_manager.delete(&self.key)`（I037 语义逐字节保持）。
   - `new_key == self.key`（同键原值）→ 既有 `index_manager.update(&self.key, new_row_id)`（锚点用例保持）。
   - 新键可键控且 ≠ 旧键 → `index_manager.delete(old_key)` **随后** `index_manager.insert(new_key, new_row_id)`。顺序约束：先删后插——`delete` 内部 `search` 返回的 row_id 此时仍是旧版本 row_id，正确清理 `row_to_key` 旧映射（index_manager.rs:245-266）；若先 `update(old_key → new_row_id)` 再删，`delete` 的 search 会命中新 row_id 并清错反向映射。

**索引层事实**：`BTree::insert` 的 DuplicateKey 检查已被注释禁用（node.rs:127），索引层不拒重复——唯一性契约完全由执行器先查后写承担，故 rekey 必须自带碰撞预检。`IndexManager::insert/delete/update` 均经 `spawn_blocking` + root 同步（index_manager.rs:223-322），语义与既有调用一致。

**错误路径**：碰撞拒绝发生在任何写入之前，无隐式事务回滚依赖（pipeline DML 包裹的 abort 链路 pipeline.rs:141-175 不被触发）。delete 后 insert 仅剩 IO 错误窗口（与既有 I037 delete、既有 update 失败暴露面同类，见 Risks）。

**既有测试校准（Plan Review BH-3 裁定，2026-09-13）**：M10 时代直连执行器单测 `tests/gc_test.rs`（3 用例）、`tests/version_chain_test.rs`（2 用例）、`tests/plan_exec_test.rs::test_insert_update_scan_flow`（1 用例）以「SET 键列 = 另一 Int 值」建立版本链，并按**旧键** search/IndexScan 定位 rekey 后版本或寻址后续 UPDATE——依赖本节修复的缺陷行为（旧键条目残留指向新版本）。本节实施后该 6 用例确定性失败（Plan 独立复跑 + Act 双轮全量清单一致）。按 D3 BH-1 校准先例处置：测试主题（GC 清理与最新版本可达 / 版本链遍历与可见性 / 插改扫流）与断言语义保持，寻址与定位一律改用**行当前键**（rekey 后的新值键）；UpdateExecutor 全部测试用法排查证实受影响面恰为该 6 用例（`executor_test.rs` rekey 用例不依赖旧键寻址、修复后 GREEN；非键列更新用例不受影响）。校准在 `update-index-maintenance` delta spec 记录，执行见 tasks T9。

## D5 用户决策与范围记录

1. 探针残差（Int 键列 + Float 存储值）处置：**并入本 change 直接修复**（D3），不登记 improvement——用户 2026-09-12 原话：「既然在这里发现另外的缺陷，我们顺便就在同一个change计划然后修复了吧，不登记直接计划解决吧，继续」。
2. I047 碰撞语义：**写入前拒绝**（用户选定，2026-09-12）。
3. UPDATE/DELETE 键位等值对非 Int 键列：**保持 KeyNotFound**（用户选定，2026-09-12）——与 update-index-maintenance R1-S4 已验收语义一致（响亮报错非静默错误）；`build_update`/`build_delete`（ddl_dml.rs:359-437）不改动。
4. Gate 1：2026-09-12 批准 R1-R5 基线与范围。
5. 存量数据兼容：预发布（MS14 分发后置），既有越界行不迁移、restore 历史越界 dump 响亮失败——用户批准范围内记录。

## D6 测试策略

- **Iteration 000**：`tests/keyless_eq_routing_test.rs` 扩展 I046 场景（Float 键列 Int 字面量简单/AND/反向 + plan 形状断言 + String 键列 Int 字面量结果不变 + restart；夹具先例：exec_ok/query_rows/plan_of + wal_buffer.shutdown）；新建 `tests/key_type_conformance_test.rs`（D3 拒绝矩阵 + 零副作用 + NULL/非 Int 键列保持锚点 + import/restore 既有套件见证）。RED 先行：路由场景现经 IndexScan/Filter(Scan) 空集、强制场景现被接受，均为 RED；实现后 GREEN。
- **Iteration 001**：`tests/update_index_maintenance_test.rs` 扩展 rekey 场景（新键可达/旧键清理/碰撞写入前拒绝/恢复两态一致/同键锚点复跑）。RED：rekey 现状旧键残留、新键空集。
- 回归锚点：`pushdown_test.rs` 两 PK 形状用例、`keyless_row_test.rs`（含 T8-R2 校准）、`keyless_eq_routing_test.rs` 既有 8 用例、`update_index_maintenance_test.rs` 既有 5 用例——全部零修改通过。基线 867 tests / 0 failed / 2 ignored（2026-09-12，d8a244f 后代码面零变化，工作区仅 docs）。

## D7 INSERT 列清单映射修复（BH-2，Plan Review 裁定并入，2026-09-12）

**当前行为**：`build_insert`（ddl_dml.rs:70-88）把列清单装入 `InsertNode.columns`（plan.rs:137-144）但**全链路无消费点**——`InsertExecutor` 构造只收 `values` + table_meta，值按表列序位置解释。探针三形态（2026-09-12，d8a244f+T2/T4 工作区）：乱序清单静默错位（`INSERT INTO p (v, id) VALUES (1, 5.0)` → 落库 `[1, 5.0]`，即 id=1、v=5.0）；部分清单触发 `compute_tuple_size` 断言 panic（tuple.rs:38，exit 101）；未知列静默接受（affected 1）。既有测试列清单均按表列序书写，缺陷潜伏。

**目标行为**（plan 期单点修复，`build_insert`）：

- 列清单非空：SHALL 恰为表列集合的一个排列（每项为已知列、无重复、数量与表列数相等），否则 `PlanError` 拒绝（exit 3，计划期、零副作用）；满足时把每行 values 按清单→表列映射重排后装入 `InsertNode.values`。
- 列清单为空：每行 values 长度 SHALL 等于表列数，否则计划期拒绝（消除无清单形态的同源 panic）。
- 重排后键位值即用户赋给键列的值——D3 键位类型校验（执行器侧）天然作用于正确位置，R3「显式列序」场景由此可见证（`(v, id) VALUES (1, 5.0)` → 重排后 id 收 5.0 → `KeyTypeMismatch`）。
- 执行器与存储层零改动；`InsertNode.columns` 字段保留（plan 数据完整性），消费面即重排逻辑本身。

**选择理由**：`build_insert` 已有 `self.tables` 注册列名（表列序，pipeline 注册顺序保证）；plan 期校验使三类缺陷（错位/panic/未知列）统一在执行前响亮拒绝。拒绝而非 NULL 填充（SQL partial INSERT 语义）：引擎无 DEFAULT，列数不足的填充属能力扩展，非本 change 目标（Non-goal：partial INSERT 支持）。备选「执行器侧重排」被否：执行器无列清单输入，需扩构造面；plan 期零执行成本且与既有计划期拒绝面（ESCAPE/TRY_CAST 等）一致。

**测试见证**：新建 `tests/insert_column_list_test.rs`——乱序清单正确落位（修复前错位 RED）、乱序清单键位越界拒绝（R3 场景）、部分清单计划期拒绝（修复前 panic RED）、未知列拒绝（修复前 accepted RED）、无清单数量不符拒绝、清单与表列序一致锚点（既有行为 GREEN）。

## Risks and Notes

- rekey 先删后插后 insert 的 IO 失败窗口（索引缺旧键条目直至重开重建）——与既有 I037 delete、既有 update 失败暴露同类，不新增风险类别；WAL/版本链不受影响（索引不进 WAL）。
- 既有测试若隐式依赖越界键位写入（Float-into-Int 键列），RED 阶段即暴露 → 按实质基线发现返回 Plan（预计不存在：既有套件数据均合规）。**→ Plan Review 已证伪（BH-1）**：`negative_number_literal_persists` 即此类依赖，全量唯一确定性失败；按 D3 校准方案处置（见 D3 末段）。
- 既有测试若隐式依赖 rekey 缺陷行为（旧键条目残留指向新版本），全量阶段即暴露 → 按实质基线发现返回 Plan（预计不存在：Iteration 001 调查将全量影响面锁定为 `update_index_maintenance_test` + `keyless_row_test`）。**→ Plan Review 已证伪（BH-3，2026-09-13）**：M10 时代直连执行器测试 gc_test ×3 / version_chain_test ×2 / plan_exec_test ×1 即此类依赖，全量 6 处确定性失败；按 D4 校准方案处置（见 D4 末段）。
- 已知 flaky：I041（`cli::resolve` env 竞态，全量偶发假失败、复跑即绿）——Plan Review 独立复跑撞到一次；与本次 diff 无关，属既有登记项。
- INSERT 列清单无消费点三形态（错位/panic/未知列接受）已由 BH-2 裁定并入修复，见 D7。
- 类型未知降级（未注册类型不分流）——只有 pipeline 真表注册路径可触达键位等值，降级不可达；派生别名 pk="" 惰性已确认（query.rs:150）。
- `equals(Int, Float)` 精度边界（`as f64` 转换）：与既有非键谓词下推求值语义一致，不新增语义面。
