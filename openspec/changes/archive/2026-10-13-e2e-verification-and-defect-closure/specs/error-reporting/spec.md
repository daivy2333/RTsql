## ADDED Requirements

### Requirement: 错误消息前缀单层

SQL 文本解析类错误的 CLI 输出 SHALL 恰含一层类别前缀（`Parse error: …`），SHALL NOT 出现同类前缀嵌套（`Parse error: Parse error: …`）或跨类嵌套（`Plan error: Parse error: …`）。前缀由单一赋予点产生；SQLSTATE 分类（42601）与退出码 SHALL 零变化。

#### Scenario: 非法 SQL 单层前缀

- **WHEN** `rtsql demo "schema t.db"`（非法 SQL 文本）
- **THEN** 错误输出恰一层 `Parse error:` 前缀（失败边界——修复前：`Parse error: Parse error: sql parser error: …` 双层嵌套）

#### Scenario: 计划阶段解析类错误单层

- **WHEN** 计划阶段产生解析类错误（经 `PlanError::ParseError` 通道）
- **THEN** 输出恰一层前缀（失败边界——修复前：`Plan error: Parse error: …` 跨类嵌套形态）

#### Scenario: 既有错误面零回归

- **WHEN** 既有错误分类用例（42601/42P01/42703/0A000 等）与退出码矩阵
- **THEN** 分类、文案主体与退出码不变（仅前缀层数收敛；锁双层形态的既有断言随白名单校准并逐条记录）
