# MS15-Rest: 初版前正确性收口第二批（I034 / I037 / I039 聚合 change）

## Why

tasks MS15-T02/T03/T04 + improvements I034/I037/I039：MS14 分发收口达成初版前，清零剩余 3 项用户可见正确性缺陷。路线图原计划每项独立 change（单项失败不阻塞其余）；用户 2026-09-12 指示将 MS15 剩余工作聚合为**一个 change**——三项分属不同故障域（CLI 渲染 / update 执行器索引维护 / 引擎表名解析语义），以三个逻辑 Iteration 保持独立验证边界与"单项失败不阻塞其余"的属性，仅共享 change 载体。

三项缺陷均为既有实证（improvements 登记 + 归档 change 记录），本会话调查补充了行级代码证据：

- **I034（CLI 表头）**：`get_plan_output_columns`（`src/parser/planner/query.rs:23-84`）仅对 Filter/Sort 臂应用 `node.projection`；DataScan/Scan/IndexScanAll 臂返回全 schema 列名，而扫描执行器已按投影裁剪行——`SELECT name FROM t`（裸 DataScan）CLI 表头 `["id","name"]`、行 `[["Alice"]]`；json 输出 `columns` 与 `rows` 字段数不一致。spec `cli-noninteractive-shell` R6 S1「表头 ["name"]」的 bare-DataScan 分支自 MS10-T01 起未满足（MS10-T04 Plan Review finding 5 实证）。
- **I037（UPDATE 键位索引）**：`UpdateExecutor`（`src/executor/update.rs:129-133`）对新值不问可键控性，无条件 `index_manager.update(&self.key, new_row_id)`——`UPDATE SET <键列> = NULL` 后旧键条目指向键位已为 NULL 的版本：对旧键值 INSERT 被 DuplicateKey 误拒（无行实际持有）、旧键值点查返回键位为 NULL 的行（索引信任无残差校验）、崩溃恢复后重建索引自然清除（两态不一致）。
- **I039（表名引号膨胀）**：全仓 10 处表名消费用 `ObjectName/Identifier` 的 Display（含引号字符）：`ast.rs:27/217/223`、`query.rs:102`、`pipeline.rs:867/958/971`、`subquery.rs:136/140`、`ddl_dml.rs:305/348`。dump 侧 `create_table_sql`/`quote_ident`（`lifecycle.rs:532-562`）恒加引号转义，restore 把带引号 Display 存成字面表名 → `dump→restore→dump` 逐代膨胀（`"items"` → `"""items"""`）。

## What Changes

- **R1（I034）**：修改 spec `cli-noninteractive-shell`「扫描执行器真投影」——CLI 表头 SHALL 经 `get_plan_output_columns` 与行形状一致（覆盖裸 DataScan / 下推 DataScan 路径与 json 字段数一致面）；实现：`get_plan_output_columns` 对携带投影的 scan 节点臂（DataScan 必需；Scan/IndexScanAll 对称加固）应用 `node.projection`，与 Filter/Sort 臂既有模式一致；IndexScan 臂不变（构造时已收窄 columns）。
- **R2（I037）**：新 capability spec `update-index-maintenance`（2 Requirement）——UPDATE 将键列置为不可键控值（`to_key()==None`）SHALL 删除旧键索引条目（`IndexManager::delete`）而非更新指向；旧行为新行（MVCC 链）与恢复侧索引重建语义对齐；既有 UPDATE 语义零回归。
- **R3（I039）**：新 capability spec `table-name-resolution`（3 Requirement）——引擎表名解析 SHALL 取标识符去引号值（`Identifier.value`，列名同款先例），带引号与裸名拼写等价（CREATE/DROP/SELECT/INSERT/UPDATE/DELETE 全语句面）；dump/schema 输出 DDL 的 restore 结果表名保真、`dump→restore→dump` 文本恒等（多代不膨胀）；裸名既有语义零回归。实现：`ast.rs` 新增归一化 helper，替换 10 处 Display 消费点；dump 侧 `quote_ident` 不变。

不改变：`extract_pk_from_where`/`IndexScan` 路由、无键行存储语义（MS10-T05 001-rework）、plan cache 键、恢复重放语义（keyless 桶）、CLI 渲染格式族、dump 的 `quote_ident` 转义输出。

## Scenario Sketch

见三个 delta spec（R1: `specs/cli-noninteractive-shell/spec.md`；R2: `specs/update-index-maintenance/spec.md`；R3: `specs/table-name-resolution/spec.md`）。关键形态：

- R1 前置：含数据表 + 子集投影 SELECT；触发裸 DataScan / 下推 DataScan 查询；观察 CLI 表头、行形状与 json `columns`/`rows` 字段数。失败边界：修复前表头字段数 > 行字段数（错位）。
- R2 前置：键列表 + 键行；触发 `UPDATE SET <键列> = NULL`；观察旧键 INSERT（误拒 → 成功）、旧键点查（返回无键行 → 空集）、崩溃重开两态。失败边界：非键列 SET 与键列原值更新行为不变。
- R3 前置：带引号/裸名建表；触发同表任意拼写访问与 dump→restore→dump 链；观察表名归一与 dump 文本恒等。失败边界：修复前多代引号逐代膨胀。

## 默认假设（Gate 1 已批准，2026-09-12）

以下决策曾提交用户（AskUserQuestion）未获回答，按保守默认执行并登记；2026-09-12 用户审计批准本 change 时对三项默认一并批准（原话"批准"），无推翻项：

1. **I039 方向 = A 解析侧归一化**（I039 记录的两方案之一、推荐方向）：引擎侧根治，dump/schema 输出不变；代价是历史带引号表名（旧 restore 产物）变为不可达——预发布无兼容承诺，design D3 记录。
2. **I037 邻接形态排除**：调查新发现 `UPDATE SET 键列 = 另一个可键控 Int`（如 5→7）同病（旧键条目残留、新键不可达、INSERT 旧键误拒、恢复后两态不一致）。按 Scope Control 默认不扩用户点名范围，建议 docs-maintainer 登记新 I 项（修复方向：删旧键 + 插新键 + 新键撞已有行 DuplicateKey 拒绝，与恢复侧重复 PK 显式报错一致）；Gate 1 审计若批准并入则扩 R2。
3. **工作区不提交**：MS15-T01 实施与 docs sync 保持未提交，本 change 计划基线 = 当前工作树（853 tests pass / 0 failed / 2 ignored，2026-09-12 Plan Review 独立复跑）。Act 开始前建议用户先 commit MS15-T01（历次先例每 change 独立 commit），commit 决定权保留用户。

## Out of Scope

- I037 邻接形态（键列 SET 为另一可键控 Int，见默认假设 2）。
- I046（形态 2 键列类型感知路由，用户已裁定独立后续 change）；I035 no-FROM SELECT；I038 GC 无键链盲区；I032/I041/I042/I043/I044/I045。
- IndexScanAll 修复面（plan 变体当前无构造点，grep 实证；仅对称加固其 `get_plan_output_columns` 臂）。
- dump 侧 `quote_ident` 语义（恒引号 + 转义输出保持——归一化后目录名不含引号字符，输出即安全传输形态）。
- 性能优化；plan 路由改动（I036/I046 域）。
