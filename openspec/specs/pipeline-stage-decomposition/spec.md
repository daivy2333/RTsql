# pipeline-stage-decomposition Specification

## Purpose
SQL 执行拆成 parse、plan、execute 三个阶段函数，完整流程由这三段按顺序组成。来源：`2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages`

## Requirements

### Requirement: 三阶段函数拆分

pipeline SHALL 将 SQL 执行分解为 parse（parse_sql）、plan（cache 查找 + 表注册 + build_plan + cache 写入）、execute（executor 创建 + 运行，含 DML 事务包裹）三个独立阶段函数；完整流程 SHALL 由三阶段顺序组合而成。

#### Scenario: 正常查询经过三阶段

- **WHEN** 执行一条合法 SELECT（cache miss）
- **THEN** parse 阶段产出语句、plan 阶段产出 PhysicalPlan、execute 阶段产出结果 Response
- **AND** 结果与拆分前一致

#### Scenario: cache hit 跳过 parse 与 plan

- **WHEN** 相同 SQL 第二次执行且命中 plan cache
- **THEN** parse 与 plan 阶段不执行，直接进入 execute 阶段
- **AND** 返回正确结果

#### Scenario: parse 阶段错误终止

- **WHEN** SQL 文本无法解析
- **THEN** 在 parse 阶段终止并返回既有格式的 Parse error Response
- **AND** plan 与 execute 阶段不执行

#### Scenario: plan 阶段错误终止

- **WHEN** 语句解析成功但无法构建计划（如表不存在）
- **THEN** 在 plan 阶段终止并返回既有格式的 Plan error Response
- **AND** execute 阶段不执行

#### Scenario: DML 事务包裹保持

- **WHEN** 执行 INSERT / UPDATE / DELETE
- **THEN** DML 仍运行在真实事务内（begin → 执行 → commit，失败 abort）
- **AND** 写后 `create_tx_id != 0`（MS06-T01 语义不回退）

#### Scenario: DDL 缓存失效保持

- **WHEN** 执行 CREATE TABLE 或 DROP TABLE 成功
- **THEN** plan cache 被清空，行为与现状一致

### Requirement: 阶段级单测覆盖

parse、plan、execute 三阶段 SHALL 各自具备不依赖完整 pipeline 流程即可独立调用并断言的单元测试。

#### Scenario: 各阶段独立可测

- **WHEN** 分别运行三阶段的单元测试
- **THEN** parse 单测仅断言解析产物、plan 单测仅断言计划构建（给定已注册表）、execute 单测仅断言计划执行产物
- **AND** 任一阶段单测失败时可定位到唯一阶段

### Requirement: 三段顶层计时观测

profiling 启用时 SHALL 输出 parse / plan / execute 三段顶层耗时；现有 profiling 开关机制（默认关闭）不变；子指标能力保留；输出计时名称分组允许调整。

主命令 `rtsql <db> "<sql>"` 在环境变量 `RTSQL_PROFILING` 已设置时 SHALL NOT 以进程中止结束。计划含索引扫描的语句 SHALL 把查询结果交给调用方，或者返回该语句在未设置该变量时会返回的 SQL 错误。错误文本与 SQLSTATE SHALL 与未设置该变量时相同。

同一条件下，主命令中进入执行阶段的语句 SHALL 向 stderr 输出一张旧的三段计时表。这张表与 import 路径已有的表是同一输出：表头含 `Time (µs)`，并含 parse、plan、execute 三行与 `Total` 行。一次主命令调用中有多条语句进入执行阶段时，每条语句 SHALL 各输出一张表；后一张表 SHALL NOT 包含只有前一条语句才会记下的子指标。未进入执行阶段的路径 SHALL NOT 输出该表，包括解析失败、规划失败，以及 BEGIN、COMMIT、ROLLBACK 和 ATTACH。进入执行阶段之后，无论语句成功还是返回 SQL 错误，SHALL 输出该表。

未设置 `RTSQL_PROFILING` 时，主命令 SHALL 保持现有行为：不输出该表，查询结果、错误文本、SQLSTATE 与退出码不变。

import 路径在该变量已设置时 SHALL 仍按既有规则输出这张表。统一诊断门面的观察输出 SHALL 保持；本要求不改写它的输出口径。主命令在该变量已设置时 SHALL NOT 因此写出以 `diagnostics ` 开头的诊断输出。

#### Scenario: profiling 输出三段耗时

- **WHEN** 开启 profiling 后执行一条 SQL
- **THEN** 输出包含 parse、plan、execute 三段各自的耗时
- **AND** profiling 关闭时零额外计时代价路径与现状等价

#### Scenario: 主命令主键等值扫描返回结果并输出三段表

- **GIVEN** `RTSQL_PROFILING` 已设置，表含整数主键且至少有一行
- **WHEN** 主命令执行该主键列的简单等值 `SELECT`
- **THEN** 退出码为 0，stdout 含该行
- **AND** stderr 含一张三段计时表，其中有 parse、plan、execute、`Total`，以及 `index_manager_search`
- **AND** stderr 不含 `diagnostics ` 前缀

#### Scenario: 主命令二级索引等值扫描返回结果并输出三段表

- **GIVEN** `RTSQL_PROFILING` 已设置，表的非主键列上有二级索引，且该列至少有一行匹配值
- **WHEN** 主命令执行该列的等值 `SELECT`
- **THEN** 退出码为 0，stdout 含该行
- **AND** stderr 的三段计时表含 parse、plan、execute、`Total`，以及 `index_manager_search_all`

#### Scenario: 主命令无索引点查的语句也输出三段表

- **GIVEN** `RTSQL_PROFILING` 已设置，表中已有数据
- **WHEN** 主命令执行一条会进入执行阶段、且计划不做索引点查的语句
- **THEN** 退出码为 0，查询结果与未设置该变量时的结果一致
- **AND** stderr 含一张有 parse、plan、execute 与 `Total` 的三段计时表

#### Scenario: 主命令未设置变量时不输出三段表

- **GIVEN** `RTSQL_PROFILING` 未设置，表含整数主键且至少有一行
- **WHEN** 主命令执行该主键列的简单等值 `SELECT`
- **THEN** 退出码为 0，stdout 含该行
- **AND** stderr 不含 `Time (µs)`

#### Scenario: 主命令解析失败不输出三段表

- **GIVEN** `RTSQL_PROFILING` 已设置
- **WHEN** 主命令执行无法解析的 SQL 文本
- **THEN** 退出码为 3，stderr 含解析错误文本且不带 SQLSTATE
- **AND** stderr 不含 `Time (µs)`

#### Scenario: 主命令规划失败不输出三段表

- **GIVEN** `RTSQL_PROFILING` 已设置
- **WHEN** 主命令查询一张不存在的表
- **THEN** 退出码为 3，stderr 含 `Table not found`，SQLSTATE 为 `42P01`
- **AND** stderr 不含 `Time (µs)`

#### Scenario: 主命令执行失败仍返回既有错误并输出三段表

- **GIVEN** `RTSQL_PROFILING` 已设置，整数主键表中该主键已经存在
- **WHEN** 主命令再次插入同一主键
- **THEN** 退出码为 3，stderr 含 `Execution error: Duplicate key`，且该错误对象没有 sqlstate
- **AND** stderr 含一张三段计时表

#### Scenario: 主命令显式事务内的索引扫描返回结果并只输出一张表

- **GIVEN** `RTSQL_PROFILING` 已设置，表含整数主键且至少有一行
- **WHEN** 主命令一次执行 `BEGIN`、该主键的简单等值 `SELECT` 和 `COMMIT`
- **THEN** 退出码为 0，stdout 含该行
- **AND** stderr 恰好含一张三段计时表，其中有 parse、plan、execute

#### Scenario: 主命令多条语句各自输出三段表且子指标不串

- **GIVEN** `RTSQL_PROFILING` 已设置，表含整数主键且至少有一行
- **WHEN** 主命令先执行该主键的简单等值 `SELECT`，再执行 `SELECT COUNT(*)` 统计该表
- **THEN** 退出码为 0，stderr 含两张三段计时表，两张都有 parse、plan、execute
- **AND** 后一张表不含 `index_manager_search`

#### Scenario: 主命令只执行 ATTACH 时不输出三段表

- **GIVEN** `RTSQL_PROFILING` 已设置，主库和另一个已创建的库都在
- **WHEN** 主命令只执行一条把另一个库附上来的 `ATTACH`
- **THEN** 退出码为 0
- **AND** stderr 不含 `Time (µs)`

#### Scenario: 导入路径仍输出三段表

- **GIVEN** `RTSQL_PROFILING` 已设置，会话诊断配置保持默认
- **WHEN** `rtsql import` 成功导入
- **THEN** stderr 含 parse、plan、execute
- **AND** 这次导入的查询结果与错误语义保持原样

### Requirement: 三阶段独立 micro-bench

benches SHALL 提供对 parse、plan、execute 三阶段分别测量的基准入口，且可通过 cargo bench 运行。

#### Scenario: 三阶段 bench 可运行

- **WHEN** 运行新增的三阶段基准套件
- **THEN** 三个阶段各有独立测量入口并产出测量数据
- **AND** bench 编译与运行无警告错误
