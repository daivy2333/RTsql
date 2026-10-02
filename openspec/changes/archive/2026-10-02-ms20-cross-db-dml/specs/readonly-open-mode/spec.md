## MODIFIED Requirements

### Requirement: 只读会话 ATTACH 附加库只读

只读会话中的 ATTACH SHALL 以只读模式打开附加库（共享锁、WAL 未决拒绝、现有锁预算与主库同型）。附加库 INSERT VALUES/INSERT SELECT/UPSERT/REPLACE、UPDATE、DELETE SHALL 在任何源求值或写入前按本域只读拒绝语义以 SQLSTATE 25006、CLI exit 3 点名拒绝。锁冲突与 WAL 未决 SHALL 使 ATTACH 失败，注册零副作用。只读跨库 SELECT 保持可用。

#### Scenario: 只读会话跨库只读分析

- **GIVEN** 两库 checkpoint-clean，附加库当前被写者持锁
- **WHEN** 只读会话 ATTACH 后执行 SELECT
- **THEN** 按预算等待；超时 DatabaseLocked，写者预算内关闭则注册成功并可读取

#### Scenario: 附加库 WAL 未决拒绝

- **GIVEN** 附加库有未决 WAL
- **WHEN** 在只读会话 ATTACH
- **THEN** 点名拒绝，注册表与附加库文件不改变

#### Scenario: 附加库写入拒绝

- **GIVEN** 只读会话已注册附加库
- **WHEN** 执行附加目标 VALUES、INSERT SELECT、UPSERT/REPLACE、UPDATE 或 DELETE（含 BEGIN 内）
- **THEN** SQLSTATE 25006、exit 3 点名拒绝，源不求值、数据与文件不变
