## MODIFIED Requirements

### Requirement: 既有计时出口和 stderr 告警保持

`RTSQL_PROFILING` 未设置时，既有三段计时 SHALL 保持关闭。该变量已设置且会话诊断配置保持默认时，stderr SHALL 仍包含 parse、plan、execute 三段耗时，子指标能力 SHALL 保持。

经语句管道执行的语句在该变量已设置时，SHALL 只在执行阶段被触达的返回路径输出这张表。执行阶段被触达指两种情形：计划缓存命中后直接执行，以及解析与规划都成功后进入执行。解析失败、规划失败、多语句拒绝、占位符编号失败、参数个数不匹配、无语句可执行等执行阶段之前的失败路径 SHALL NOT 输出这张表。进入执行阶段后，无论执行成功还是失败，SHALL 输出这张表。

该收敛 SHALL 只改变这张表的输出时机。语句返回的错误文本与 SQLSTATE SHALL 与收敛前相同；诊断摘要、取消语义和取消后的行可见性 SHALL NOT 受影响。

诊断配置为默认时，现有用户可见 stderr 告警 SHALL 保持原文。恢复路径上已有的 `[recovery]` 告警 SHALL 仍出现在产生它的失败条件下。本变更 SHALL NOT 删除或改写这些告警来充当诊断输出。

#### Scenario: 旧开关仍输出三段耗时

- **GIVEN** `RTSQL_PROFILING` 已设置，会话诊断配置保持默认
- **WHEN** 执行一条未命中计划缓存的语句
- **THEN** stderr 含有 parse、plan 和 execute

#### Scenario: 恢复告警原文仍在

- **GIVEN** 诊断配置为默认
- **WHEN** 走到会写出 `[recovery]` 告警的既有恢复失败条件
- **THEN** stderr 仍包含 `[recovery]`

#### Scenario: 失败路径也输出旧的三段计时表

- **GIVEN** `RTSQL_PROFILING` 已设置，会话诊断配置保持默认，该语句已触达执行阶段
- **WHEN** 经语句管道执行一条进入执行阶段后失败的语句
- **THEN** stderr 仍含一张三段计时表
- **AND** 返回的错误文本与 SQLSTATE 不变

#### Scenario: 执行阶段之前的失败路径不输出旧的三段计时表

- **GIVEN** `RTSQL_PROFILING` 已设置，会话诊断配置保持默认，该语句尚未触达执行阶段
- **WHEN** 经语句管道执行一条在执行阶段之前失败的语句
- **THEN** stderr 不含这张三段计时表
- **AND** 返回的错误文本与 SQLSTATE 不变
