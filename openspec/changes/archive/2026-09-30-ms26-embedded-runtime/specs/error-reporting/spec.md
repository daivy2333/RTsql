# error-reporting Delta — MS26 T1 只读拒绝 25006 分类

## MODIFIED Requirements

### Requirement: PG 协议错误码分类

PG 协议错误响应 SHALL 按错误类别携带区分的 SQLSTATE 错误码，SHALL NOT 再对全部错误统一返回 58000。分类 SHALL 至少覆盖：计划期能力面拒绝（不支持语句/表达式/约束/JOIN 类型/子查询位置等）→ `0A000`（feature_not_supported）；解析错误 → `42601`（syntax_error）；表不存在 → `42P01`（undefined_table）；列不存在 → `42703`（undefined_column）；列名歧义 → `42702`（ambiguous_column）；只读状态下的写操作拒绝（`readonly-open-mode` 域）→ `25006`（read_only_sql_transaction）；其余未分类错误 SHALL 维持 `58000` 兜底。完整变体映射表随 design 定稿并保持可枚举。

#### Scenario: 能力拒绝返回 0A000

- **GIVEN** 经 PG 协议连接的客户端
- **WHEN** 发送不被支持的语句（如 `MERGE`）
- **THEN** 错误响应携带 SQLSTATE `0A000`（feature_not_supported）且消息点名能力

#### Scenario: 解析错误返回 42601

- **GIVEN** 经 PG 协议连接的客户端
- **WHEN** 发送语法错误的语句
- **THEN** 错误响应携带 SQLSTATE `42601`（syntax_error）

#### Scenario: 名字解析错误按类返回

- **WHEN** 分别发送引用不存在表与不存在列的语句
- **THEN** 错误响应分别携带 `42P01` / `42703`

#### Scenario: 未分类错误维持兜底

- **WHEN** 触发未纳入分类表的内部错误
- **THEN** 错误响应维持 `58000`，不因分类引入新的错误形态

#### Scenario: 只读拒绝返回 25006

- **GIVEN** 只读会话
- **WHEN** 执行任意 DML/DDL 写语句
- **THEN** 错误响应 SQLSTATE 为 `25006`，错误文案点名只读状态
