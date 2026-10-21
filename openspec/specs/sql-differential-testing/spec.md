# sql-differential-testing Specification

## Purpose

以 rusqlite（既有开发依赖）为参照引擎的跨引擎差分与结构性质测试网：对语义一致 SQL 子集双向对比、等价变换一致性验证、B-Tree/版本链/编码往返性质测试（固定种子）。差分网是规划器与语义改动的静默偏差拦截保护基线；判定全部位于项目测试框架内。来源：MS30-T01 / I091（change `2026-10-20-ms30-semantic-binding-differential-net` Iteration 000）；首跑分诊出的 5 条疑似缺陷经 T03 清偿（2026-10-21，白名单余 1 条已知语义差异）。

## Requirements

### Requirement: 语义一致子集的跨引擎差分对比

对双方语义一致的 SQL 子集，RTsql 与 SQLite（rusqlite，既有开发依赖）对同一查询集的执行结果经归一化后 SHALL 逐行一致；已知语义差异 SHALL 以白名单文档化——每条含分歧描述、原因分类（已知语义差异 / 疑似缺陷）与规避方式；白名单外分歧 SHALL 使差分测试失败，SHALL NOT 静默跳过。

#### Scenario: 子集内查询结果一致

- **GIVEN** 差分子集内一条查询与固定种子数据集，在 RTsql 与 SQLite 同构建库
- **WHEN** 两引擎分别执行该查询
- **THEN** 归一化（排序稳定性、NULL 表示、数值形态）后的行集一致

#### Scenario: 白名单外分歧使测试失败

- **GIVEN** 一条 RTsql 与 SQLite 结果分歧且不在白名单的查询
- **WHEN** 差分套件运行
- **THEN** 该查询的差分断言失败；开发期分诊为缺陷或补录白名单，无无条件跳过路径

#### Scenario: 白名单条目各带原因分类

- **GIVEN** 白名单中任一已知分歧条目
- **WHEN** 查看该条目
- **THEN** 记录分歧描述、原因分类与规避方式

### Requirement: 等价变换一致性

别名改写、派生表包裹、等价谓词变换产生的查询变体与原查询在 RTsql 上 SHALL 产出归一化后一致的结果；变换变体与 SQLite 的差分 SHALL 同样满足跨引擎一致性。

#### Scenario: 别名改写等价

- **GIVEN** 子集内一条查询与其仅改变表/列别名的变体
- **WHEN** 两查询在同一 RTsql 库上执行
- **THEN** 归一化行集一致

#### Scenario: 派生表包裹等价

- **GIVEN** 一条基表查询与其派生表/CTE 包裹形态
- **WHEN** 两查询执行
- **THEN** 归一化行集一致

#### Scenario: 谓词变换等价

- **GIVEN** 一条查询与其语义等价的谓词形态变体（如比较腿交换、AND 项重排）
- **WHEN** 两查询执行
- **THEN** 归一化行集一致

### Requirement: 结构性质测试确定性可重现

B-Tree、版本链与编码往返的性质测试 SHALL 在项目测试框架内以固定种子生成输入、可重现运行，SHALL NOT 依赖时间敏感断言或非确定迭代序；套件规模 SHALL 有界（固定查询集与种子），纳入全量回归基线后不使全量运行时间失控。

#### Scenario: B-Tree 插删查不变量

- **GIVEN** 固定种子生成的键/行序列
- **WHEN** 依序插入、更新、删除后全量枚举
- **THEN** 枚举结果与排序去重的逻辑模型一致

#### Scenario: 编码往返无损

- **GIVEN** 固定种子生成的各类型值（含边界：NULL、长字符串、日期族）
- **WHEN** 序列化再反序列化
- **THEN** 值与类型逐项相等

#### Scenario: 版本链可见性不变量

- **GIVEN** 固定种子生成的写入/更新/删除/回滚序列
- **WHEN** 依序执行后读取
- **THEN** 可见行集与串行执行模型一致

### Requirement: 差分判定位于项目测试框架内

差分与性质套件 SHALL 全部位于 `cargo test` 可运行集合内；SHALL NOT 新增测试框架外判定脚本、专用 CLI、身份协议或审计工具；结果归一化 SHALL 为测试代码内的普通比较逻辑，实际输出需要过滤时使用现成通用工具的单步管道。

#### Scenario: 全部判定经 cargo test 运行

- **GIVEN** 本域全部差分与性质测试
- **WHEN** `cargo test` 全量运行
- **THEN** 套件纳入全量基线，无框架外判定层或专用工具
