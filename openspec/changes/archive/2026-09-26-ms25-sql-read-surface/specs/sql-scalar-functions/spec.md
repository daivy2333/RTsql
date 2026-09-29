## MODIFIED Requirements

### Requirement: 调用面与边界

标量函数 SHALL 在 WHERE 与 SELECT 双侧可用：WHERE 中的函数谓词无论走下推（无 OR、非简单 PK 等值）还是 Filter 包装（含 OR）路径，结果 SHALL 一致且正确。以下既有拒绝面 SHALL 保持：SELECT 列表中函数项与聚合混用报错；HAVING 中引用标量函数不支持。ORDER BY 引用表达式项的输出别名 SHALL 按输出列真实排序（语义详见 `order-by-alias-sorting` 能力）——原「排序列不可解析时所有行视为相等、保持输入序」的静默语义由本变更终止（校准段：原「既有语义锁定」场景的期望自本变更起翻转为真实排序，见下方同名场景）。`WHERE id = abs(-5)` 等主键列与函数比较 SHALL 走普通扫描路径并返回正确结果（SHALL NOT 误入索引点查）。

#### Scenario: WHERE 下推路径结果正确

- **GIVEN** 表 `t(id INT PRIMARY KEY, name VARCHAR)` 含 `name='AbC'` 与 `name='xyz'`
- **WHEN** `SELECT id FROM t WHERE upper(name) = 'ABC'`
- **THEN** 仅输出 `name='AbC'` 行的 `id`

#### Scenario: WHERE OR 组合结果正确

- **GIVEN** 同上种子
- **WHEN** `SELECT id FROM t WHERE upper(name) = 'ABC' OR id = 2`
- **THEN** 输出两行 `id`（函数谓词与 OR 组合正确）

#### Scenario: 聚合混用保持拒绝

- **GIVEN** 表 `t(name VARCHAR)` 存在
- **WHEN** `SELECT count(*), upper(name) FROM t`
- **THEN** 报 SQL 错误（与 change 前一致）

#### Scenario: HAVING 中标量函数保持不支持

- **GIVEN** 表 `t(id INT PRIMARY KEY)` 存在
- **WHEN** `SELECT id, count(*) FROM t GROUP BY id HAVING upper(id) = 'X'`
- **THEN** 报 SQL 错误（HAVING 不支持标量函数，与 change 前一致）

#### Scenario: ORDER BY 表达式别名静默保持输入序（既有语义锁定）

- **GIVEN** 表 `t(name VARCHAR)` 按 INSERT 顺序含 `name='b'`、`name='a'`
- **WHEN** `SELECT upper(name) AS u FROM t ORDER BY u`
- **THEN** 退出码 0，按输出列排序输出 `A`、`B`（SHALL NOT 保持输入序）——校准段：原「保持输入序」期望由 change `2026-09-26-ms25-sql-read-surface` 终止，场景名保留承载语料连续性，现行语义权威为 `order-by-alias-sorting`

#### Scenario: 主键列与函数比较走普通路径

- **GIVEN** 表 `t(id INT PRIMARY KEY)` 含 `id=5`
- **WHEN** `SELECT id FROM t WHERE id = abs(5)`
- **THEN** 输出 `id=5`（普通扫描路径，结果正确）
