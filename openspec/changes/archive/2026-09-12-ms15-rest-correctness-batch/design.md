# design — MS15-Rest 初版前正确性收口第二批（I034 / I037 / I039）

## D1: I034 — `get_plan_output_columns` scan 臂投影裁剪

**当前行为**：`get_plan_output_columns`（`src/parser/planner/query.rs:23-84`）仅 Filter/Sort 臂应用 `node.projection`；DataScan/Scan/IndexScanAll 臂返回 `node.columns` 全 schema。而扫描执行器自 MS10-T01 起按 `with_projection` 在行产出最后一步裁剪行——表头与行形状脱节。

**目标行为**：携带非空 `projection` 的 scan 节点臂按投影索引裁剪列名，与 Filter/Sort 臂既有模式逐字对齐（`projection.iter().map(|&i| columns[i].clone())`）。

**节点臂逐一裁定**（2026-09-12 调查实证）：

| 臂 | 现状 | 处置 | 依据 |
|---|---|---|---|
| DataScan | columns 恒全 schema，projection 索引指向全 schema（3 个构造点：no-WHERE `query.rs:578-587`、下推 `query.rs:572-581`、OR 臂 `query.rs:565-570` 恒空） | **应用投影（必需）** | 缺陷主面；projection 语义与 Filter/Sort 同构 |
| Scan | 恒 `projection: Vec::new()`（`query.rs:108/166` 唯二构造点） | 应用投影（恒等加固） | 对称性；行为零变化 |
| IndexScanAll | **无构造点**（grep 实证，plan 变体死路径） | 应用投影（恒等加固） | 死路径零行为变化；若将来接线语义已正确 |
| IndexScan | **不改** | 保持返回 `node.columns` | 构造时 columns 已收窄为投影后列名（`query.rs:527-533` `columns: projection_columns.clone()`），再应用其 projection（索引指向基 schema）会越界/双重裁剪 |

**消费面核查**（全部 2026-09-12 代码实证）：

- CLI 表头 `src/cli/mod.rs:330`（唯一 CLI 消费点）——修复目标。
- 聚合 `input_schema`（`query.rs:634`）——聚合/标量子查询/表达式路径 `projection_indices = None`（`query.rs:464-477`）→ scan projection 恒空 → 恒等，零影响。
- 派生表列注册（`query.rs:129`）——子查询为裸 DataScan 子集投影时，修复前登记全 schema（同族潜伏缺陷，登记列与行形状脱节），修复后登记投影列名 = 行形状，方向正确。
- plan cache——columns 每次执行时从 plan 现算（不入缓存），无缓存污染面。

**既有测试校准点**：`tests/cli_test.rs:289` 注释（"Act 校准：全表扫描子集单列投影的表头为全 schema，属既有单语句行为"）——修复后按新语义改写注释；断言面复核：`test_multi_statement_sequential_render` 断言 `SELECT id, name FROM users` 全投影（identity），不受影响。

## D2: I037 — UpdateExecutor 键位无键值的索引条目清理

**当前行为**：Step 7（`src/executor/update.rs:129-133`）无条件 `index_manager.update(&self.key, new_row_id)`。SET 键列为 NULL/非 Int 时，旧键条目指向键位已无键的新版本：INSERT 旧键被 `DuplicateKey` 误拒（`insert.rs:100-110` search 命中残留条目）、旧键点查经 IndexScan 返回无键行（索引信任无残差校验）、恢复重建（无键版本不入索引）后自愈——运行期/恢复两态不一致。

**目标行为**：SET 目标列为键列且新值不可键控时，Step 7 改为 `index_manager.delete(&self.key)`；其余形态保持 `update`。

**判定条件**（两者齐备才 delete）：

- `self.column_name == self.table_meta.pk_column`（`TableMeta.pk_column: String`，`src/storage/data/table_manager.rs:53`——区分"SET 键列"与"SET 非键列"，后者条目必须保持）；
- `self.new_value.to_key().is_none()`（`src/executor/value.rs:82-90`，仅 Int → Some；与插入侧/MS15-T01 可键控性权威定义同源）。

**一致性论证**：

- **WAL/恢复**：Update WAL 记录只含 old_tuple/new_tuple（`update.rs:113-122`），索引操作本就不入 WAL；恢复侧重放后从最终数据页重建索引（MS10-T02 R7/R8），键位 NULL 行不入索引——运行期 delete 与重建结果对齐（两态一致由构造保证）。
- **失败路径**：Step 6 写新版本先于 Step 7 索引操作；delete 失败时错误传播，单语句 DML auto-commit 包裹（MS06-T01）回滚版本写入，无半态。
- **键列 SET 为可键控值**：保持既有 `update(&self.key, ...)`——`to_key()==Some` 但新键 ≠ 旧键的形态（rekey）在本 change 范围外（proposal 默认假设 2），行为不变。
- **结构性后果（2026-09-12 Plan Review 确认）**：修复后运行期 SQL 无法再产生 old_tuple 无键的 Update WAL 记录（无键行不入索引 + `build_update` 要求键位等值）——MS10-T05 T8-R2 链是该形态唯一运行期生产者，`keyless_row_test::keyless_row_update_recovery_after_crash` 相应按 R1 语义校准（replan 契约，见 `iterations/001-update-key-index/001-replan.md`）；恢复侧 keyless 桶 old-lookup 子路径转为 legacy WAL 兼容面（重放代码未动、语义未变），主 spec `wal-recovery-replay-integrity`「无键行 Update 崩溃恢复语义正确」场景的运行期见证路径收窄——处置建议 docs-maintainer 收尾时登记（I 项候选：合成 WAL 见证或接受收窄）。keyless NEW_tuple 重放路径仍由本 Iteration 恢复见证（T4 恢复用例 + 校准后 T8-R2）。

## D3: I039 — 表名解析归一化（去引号）

**方向裁定**：解析侧归一化（I039 记录方案之一，proposal 默认假设 1）。备选"DDL 生成侧条件引号"被拒：引擎仍把带引号 Display 存成表名，已含引号的表名逐代仍膨胀，且 CREATE/查询/DDL 生成两侧语义分裂。

**实现形态**：`src/parser/ast.rs` 新增唯一 helper：

```rust
pub fn object_name_to_table_name(name: &ObjectName) -> String {
    name.0.iter().map(|id| id.value.to_lowercase()).collect::<Vec<_>>().join(".")
}
```

`Ident.value`（sqlparser 0.44 类型名，`ObjectName(pub Vec<Ident>)`）为去引号内容，列名消费已用 `.value`（`ast.rs:42/45/136-140`、`ddl_dml.rs` 列定义）——表名对齐列名先例。multipart 名以 `.` 连接，与 Display 拼接形态一致（引擎无 schema 概念，行为面不变）。

**替换点（11 处 Display 消费，2026-09-12 Plan Review 行号复核；原 10 处为计数勘误）**：`ast.rs:27`（extract_table_name）/`ast.rs:216-218`（extract_name_from_object，调用点 `ddl_dml.rs:77`）/`ast.rs:223`（extract_join_table_name）改函数体；`query.rs:120`（Iter000 后现行行号）、`pipeline.rs:867/958/971`、`subquery.rs:136/140`、`ddl_dml.rs:305`（CREATE）/`ddl_dml.rs:348`（DROP）共 8 处内联改调 helper。

**dump/schema 侧不变**：`create_table_sql`/`quote_ident`（`lifecycle.rs:532-562`）保持恒引号 + 转义输出——归一化后目录名不含引号字符，输出即安全传输形态；`dump→restore→dump` 恒等（restore 归一化去引号，二代 dump 与一代同文本）。

**边界语义**：

- **历史带引号表名**（旧 restore 产物，目录名含引号字符）：dump 输出 `"""items"""` 形态 → restore 归一化得 `"items"`（转义按标识符语义）→ 恒等不继续膨胀；该表经值为 `"items"` 的拼写（如 `"""items"""`）可达，经裸名不可达。预发布无兼容承诺，不迁移。
- **plan cache**：键为 SQL 文本规范化，解析语义变化按文本自然分键，无污染面。
- **catalog/存储**：`__tables` 行内表名即目录键，无格式变更。

## D4: Iteration 结构与测试策略

三个逻辑 Iteration 对应三个故障域，独立验证边界（平衡审计见 tasks.md）：

- **Iteration 000（I034）**：RED 先行——`tests/cli_test.rs` 追加裸 DataScan / 下推 DataScan 表头断言（json `columns`/`rows` 字段数一致）观察 RED → D1 实现 GREEN → 聚合 `input_schema`/派生表面回归复核 + 全量。
- **Iteration 001（I037）**：RED 先行——新 `tests/update_index_maintenance_test.rs`（INSERT 误拒消除 / 点查空集 / 崩溃恢复两态 / 非键列保持 / 原值更新保持）观察 RED → D2 实现 GREEN → `keyless_row_test.rs` 恢复套件零回归 + 全量。
- **Iteration 002（I039）**：RED 先行——`tests/cli_test.rs` 追加引号建表互访 / dump-restore-dump 恒等 / schema 保真用例观察 RED → D3 实现 GREEN → 既有往返套件零回归 + 全量收尾（clippy/fmt/validate/CLI 探针）。

测试入口沿用既有模式：CLI e2e 用 `run_cli`/`fixture` helper（`tests/cli_test.rs`），lib 行为用 `Database::execute_sql`（`tests/keyless_row_test.rs` 模式）。

## D5: 基线与风险

- **基线**：当前工作树 = f9e1e1f + MS15-T01 实施与 docs sync（未提交），853 tests pass / 0 failed / 2 ignored（2026-09-12 Plan Review 独立复跑）。Act 开始前建议用户先 commit MS15-T01（默认假设 3）；若先实施，Act 以实施时点 `git status`/`git diff` 做基线检查并在 Response 记录。
- **R1 风险**：派生表列注册行为变化（登记从全 schema 收窄为投影列）——外层查询引用被投影掉列名的写法从"错误结果/静默 Null"变为"列不存在报错"，方向正确；全量回归复核，若有既有用例锁定旧行为按"行形状校准"清单处理（R6 既有场景先例）。
- **R2 风险**：`SET 键列 = 'x'`（String 值入 Int 键列）的类型校验时序（serialize_tuple 是否先拒绝）为非实质未知项——delete 条件下索引状态与恢复重建一致（无键行不入索引），不阻塞；Act 在 Response 记录实测行为。
- **R3 风险**：历史带引号表名可达性收缩（design D3 边界语义）——预发布可接受；若 Act 发现既有测试/夹具依赖带引号表名（grep 排查 `\"` 建表用例），按校准清单处理。
- **三次失败防线**：三项修复面相互独立（query.rs 列名提取 / update.rs 索引步 / ast.rs 解析 helper），单 Iteration 失败不跨域传播。
