# join-output-shape-resolution Delta — 投影严格解析纳入表别名

## MODIFIED Requirements

### Requirement: JOIN 基座投影严格解析与别名列可达

JOIN 查询的 SELECT 投影 SHALL 逐项解析：未知列 SHALL 以 `ColumnNotFound` 计划期点名拒绝（现状零宽行静默产出的缺陷终止）；纯列引用的别名项（`t.c AS alias`）SHALL 可达且表头为别名（MS19 T4b 机制对 JOIN 基座的扩展）；限定列引用的限定符 SHALL 先经表别名归一化（`sql-table-alias-resolution` 域）再匹配组合形状的真实表名——归一化后不可达的限定符 SHALL 点名拒绝而非计入列缺失；任意表达式项 × JOIN 维持既有显式拒绝。纯列清单的严格校验与过滤（命中计数、歧义点名）在归一化后的限定符上执行，判定规则不变。

#### Scenario: 未知列点名拒绝

- **GIVEN** 同上种子
- **WHEN** `SELECT nosuch FROM p JOIN q ON p.g < q.h`
- **THEN** 计划期 `ColumnNotFound` 点名拒绝（现状 RED：零列零宽行静默产出）

#### Scenario: 纯列引用别名项可达

- **GIVEN** 同上种子
- **WHEN** `SELECT p.g AS pg FROM p JOIN q ON p.g < q.h`
- **THEN** 输出 pg 列含各组合行的 p.g 值，表头为 `pg`

#### Scenario: 别名限定列可达

- **GIVEN** `FROM people a JOIN people b ON a.id = b.id`（两别名指向同表）
- **WHEN** `SELECT a.name FROM …`（历史缺陷形态：`Column not found: name`）
- **THEN** 返回左别名侧 name 列，表头 `name`

#### Scenario: 表达式项 × JOIN 维持拒绝

- **GIVEN** 同上种子
- **WHEN** `SELECT upper(p.x) FROM p JOIN q ON p.g < q.h`
- **THEN** 计划期点名拒绝（既有「表达式项 × JOIN」拒绝面不变）
