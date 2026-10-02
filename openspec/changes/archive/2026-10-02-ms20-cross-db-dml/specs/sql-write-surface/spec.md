## ADDED Requirements

### Requirement: INSERT SELECT 物化后写入

INSERT INTO 既有表 SELECT 查询 SHALL 支持主库或附加库目标，源 SHALL 使用现有完整 SELECT 编译和查询路由能力（含 JOIN、子查询、派生表、视图、聚合、DISTINCT、CTE、集合操作及排序分页，仍受各能力已有拒绝面约束）。系统 SHALL 在任何目标插入前完整物化源为原生 Value 行，SHALL NOT 经 JSON 或 SQL 文本往返；目标表不创建或改结构。列清单 SHALL 按位置接收源列，允许子集和任意顺序，省略列按声明 DEFAULT 或 NULL 填充，显式源 NULL SHALL 保持 NULL。列数、未知列及重复目标列 SHALL 在源求值前拒绝（包括空源）。同库读写 SHALL 只消费物化前源行，不递归读取本语句新插入的行。

目标插入 SHALL 复用既有 INSERT/Upsert 执行器与类型、NOT NULL、PK、UNIQUE、二级索引语义；支持既有 ON CONFLICT/REPLACE 动作及拒绝边界。非显式事务语句 SHALL 只在目标文件开启一个隐式事务；任何源失败 SHALL 不写目标，插入中途失败 SHALL 回滚当前目标语句全部写入。空源 SHALL 返回 affected_rows=0；输出 SHALL 为受影响行数而非源行集。附加目标的显式事务限制按 cross-db-dml；主库目标在已有主库事务中 SHALL 复用该事务，按现有错误收口与回滚语义处理，不另开独立提交。

#### Scenario: 多库查询写入附加目标

- **GIVEN** 主库 t、附加库 a.u 与 b.v 已存在
- **WHEN** INSERT INTO b.v SELECT ... FROM t JOIN a.u ...
- **THEN** 仅 b.v 插入查询结果，源表不变，返回实际受影响行数，重开结果一致

#### Scenario: 写回主库与同表读写

- **GIVEN** 主库与附加库均有源行
- **WHEN** 分别把附加查询写主库、把目标表自身查询变换后写回同表
- **THEN** 路由准确，同表只消费写入前完整物化的行，调用有限结束

#### Scenario: 目标列映射与 DEFAULT

- **GIVEN** 目标有 DEFAULT 与可 NULL 列
- **WHEN** 指定重排列子集 INSERT SELECT，源含 NULL、整数、日期族及字符串值
- **THEN** 按位置正确填充，省略位用 DEFAULT/NULL，源 NULL 不被替换，类型门与 VALUES 相同

#### Scenario: 空源仍验证列结构

- **WHEN** 源产生零行，目标列清单合法或存在列数/未知列/重复列错误
- **THEN** 合法语句 affected_rows=0；非法语句计划期点名拒绝，目标不变

#### Scenario: 源失败与插入失败

- **GIVEN** 目标有既有数据与唯一索引
- **WHEN** 源求值失败，或物化行中后续行违反目标类型/NOT NULL/PK/UNIQUE
- **THEN** 源失败不写目标；隐式事务插入失败回滚本语句全部行及索引，已有数据保持

#### Scenario: SELECT 源冲突动作

- **WHEN** 对主库或附加目标执行 INSERT SELECT ON CONFLICT DO NOTHING/DO UPDATE，或 REPLACE INTO ... SELECT
- **THEN** 使用既有冲突仲裁、动作和计数语义；非法冲突目标或赋值形态继续拒绝

#### Scenario: 主库显式事务读写

- **GIVEN** BEGIN 内有主库自身写入
- **WHEN** INSERT INTO 主库表 SELECT 主库或附加库源，然后 ROLLBACK 或 COMMIT
- **THEN** 复用主库事务与既有查询可见性，ROLLBACK 撤销目标插入，COMMIT 保留，附加源不改变
