# pipeline-stage-decomposition Specification

## Purpose
TBD - created by archiving change 2026-08-26-ms06-t03-t04-wal-handle-pipeline-stages. Update Purpose after archive.
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

#### Scenario: profiling 输出三段耗时

- **WHEN** 开启 profiling 后执行一条 SQL
- **THEN** 输出包含 parse、plan、execute 三段各自的耗时
- **AND** profiling 关闭时零额外计时代价路径与现状等价

### Requirement: 三阶段独立 micro-bench

benches SHALL 提供对 parse、plan、execute 三阶段分别测量的基准入口，且可通过 cargo bench 运行。

#### Scenario: 三阶段 bench 可运行

- **WHEN** 运行新增的三阶段基准套件
- **THEN** 三个阶段各有独立测量入口并产出测量数据
- **AND** bench 编译与运行无警告错误

