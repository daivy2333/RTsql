# Delta: error-reporting

## MODIFIED Requirements

### Requirement: 错误消息前缀单层

SQL 文本解析类错误的 CLI 输出 SHALL 恰含一层类别前缀（`Parse error: …`），SHALL NOT 出现同类前缀嵌套（`Parse error: Parse error: …`）或跨类嵌套（`Plan error: Parse error: …`）。前缀由单一赋予点产生；SQLSTATE 分类（42601）与退出码 SHALL 零变化。

本次追加执行器通道：执行错误的 CLI 输出 SHALL 恰含一层 `Execution error:` 前缀——`StorageError::ExecutionError` 的 Display 文案 SHALL NOT 自带类别前缀，单层前缀由执行漏斗（pipeline `Response::Error` 构造点）唯一赋予。修复前：变体自带小写 `execution error: ` 前缀与漏斗大写 `Execution error: ` 前缀叠加为 `Execution error: execution error: …`（ISS19，ISS11 修复 parse/plan 漏斗后的执行器漏网点）。

#### Scenario: 非法 SQL 单层前缀

- **WHEN** `rtsql demo "schema t.db"`（非法 SQL 文本）
- **THEN** 错误输出恰一层 `Parse error:` 前缀（失败边界——修复前：`Parse error: Parse error: sql parser error: …` 双层嵌套）

#### Scenario: 计划阶段解析类错误单层

- **WHEN** 计划阶段产生解析类错误（经 `PlanError::ParseError` 通道）
- **THEN** 输出恰一层前缀（失败边界——修复前：`Plan error: Parse error: …` 跨类嵌套形态）

#### Scenario: 执行器错误单层前缀

- **WHEN** 执行器路径产生错误（如 `SELECT a.id, (SELECT x.name FROM t x WHERE x.id = a.id) AS who FROM a` 未收窄标量多行场景）
- **THEN** 输出恰一层 `Execution error:` 前缀（失败边界——修复前：`Execution error: execution error: Subquery returns multiple rows …` 双层叠加）

#### Scenario: 既有错误面零回归

- **WHEN** 既有错误分类用例（42601/42P01/42703/0A000 等）与退出码矩阵
- **THEN** 分类、文案主体与退出码不变（仅前缀层数收敛；锁双层形态的既有断言随白名单校准并逐条记录）
