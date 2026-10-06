## ADDED Requirements

### Requirement: CLI 错误输出携带 SQLSTATE

CLI 错误输出 SHALL 在 `Response::Error` 携带 `sqlstate` 时渲染该代码：文本格式在错误行尾附加 ` [SQLSTATE xxxx]`，`--format json` 的错误对象 SHALL 包含 `sqlstate` 字段（`sqlstate` 缺失的错误——Usage/General 类——SHALL 省略该字段与后缀）。既有错误文案、退出码矩阵、非错误输出的序列化 SHALL 零变化；PG 协议错误响应的 SQLSTATE 携带（error-reporting R2）SHALL 零变化。

#### Scenario: 只读拒绝文本携带 SQLSTATE

- **GIVEN** `--readonly` 会话
- **WHEN** `INSERT` 被计划期拒绝
- **THEN** 错误行含 `[SQLSTATE 25006]`，文案与 exit 3 不变（失败边界——修复前：纯文字无代码）

#### Scenario: JSON 格式携带字段

- **GIVEN** 同上
- **WHEN** `--format json`
- **THEN** 错误 JSON 含 `"sqlstate":"25006"`；无 sqlstate 的错误（如缺库 General）省略字段

#### Scenario: PG 面与非错误输出零变化

- **GIVEN** serve/PG 客户端与正常查询
- **WHEN** 既有 pg_integration 断言与正常结果渲染
- **THEN** 零回归
