## ADDED Requirements

### Requirement: JOIN 基座输出列序与请求一致

纯列清单（裸路径与 WHERE / ORDER BY 裁剪路径）的 JOIN 基座输出列序 SHALL 按 SELECT 清单序（对齐 Scan 基座既有语义）；重复的 SELECT 项 SHALL 重复产出对应列；裸名多命中 SHALL 维持计划期 `AmbiguousColumn` 点名、未知列 SHALL 维持 `ColumnNotFound`；`SELECT *` 与 carry-all 全组合承载形态（聚合 / 派生 / CTAS 输入、Sort 承载全列裁剪）SHALL 既有语义零变化；表达式项的顶层 Projection 路径（既有 SELECT 序）SHALL 零回归；输出表头与行 SHALL 同源自洽（CTAS 列名 / INSERT SELECT 映射 / DerivedScan 列清单随输出序）。

#### Scenario: 裸纯列清单 SELECT 序

- **GIVEN** `emp3(id, ename, dept_id)` JOIN `dept(id, dname)` ON `e.dept_id = d.id`
- **WHEN** `SELECT d.dname, e.ename FROM emp3 e JOIN dept d ON e.dept_id = d.id`
- **THEN** 表头与行均为 `[dname, ename]` 序（失败边界——修复前表序 `[ename, dname]`）

#### Scenario: WHERE / ORDER BY 裁剪路径 SELECT 序

- **GIVEN** 同上
- **WHEN** `SELECT d.dname, e.ename … WHERE e.id = 1`；`SELECT d.dname, e.ename … ORDER BY e.id`
- **THEN** Filter / Sort 裁剪序 = SELECT 清单序（失败边界——修复前裁剪按表序）

#### Scenario: 重复项与全组合形态

- **GIVEN** 同上
- **WHEN** `SELECT e.ename, e.ename …`；`SELECT *`；聚合 / CTAS 消费 JOIN 输出
- **THEN** 重复项产出两列；`SELECT *` 表序不变；CTAS 列名取输出表头（与 Scan 基座 CTAS 一致）

### Requirement: 多表链式 JOIN 装配正确性

单 FROM ≥3 表因子的左深等值链 SHALL 端到端可达——hash+hash 纯等值链与 hash+NLJ 混合链（第三步非等值 ON）SHALL 产出正确配对行；每步等值 ON 条件键 SHALL 按（真实表, 列）限定解析，链上同名列 SHALL 不串列；中间 JOIN 步 SHALL 承载链前缀全组合列且输出列索引为左输入行绝对偏移；仅 SELECT 部分表 SHALL 正常裁剪零 panic；两表单步全形态（含 `SELECT *`、WHERE、ORDER BY、聚合、测试直构 `JoinConfig` 裸名键）SHALL 逐字节零回归；真未知列的错误面 SHALL 不变。

#### Scenario: hash+hash 三表链

- **GIVEN** `emp(id, name, dept_id)` / `dept(id, label)` / `sal(id, dept_id, amount)`，ON 链 `e.dept_id = d.id` ∧ `s.dept_id = d.id`
- **WHEN** 三表 SELECT 等值链查询
- **THEN** 端到端可达且每步配对正确（失败边界——修复前 `Column not found` 计划失败）

#### Scenario: hash+NLJ 混合链

- **GIVEN** 同上，第三步 ON 为非等值谓词
- **WHEN** 三表 SELECT
- **THEN** 可达且谓词作用于全组合行（失败边界——修复前计划失败）

#### Scenario: 仅前两表 SELECT 不 panic

- **GIVEN** 同上三表 FROM
- **WHEN** `SELECT e.name, d.label FROM … 三表链`
- **THEN** 正常输出前两表列（失败边界——修复前 `join.rs` 键查表 `.expect` panic exit 101）

#### Scenario: 链上同名列不串列

- **GIVEN** `t1(id, …)` / `t2(id, …)` / `t3(x, …)` 链，ON 条件跨链引用 `t2.id`
- **WHEN** 等值链查询
- **THEN** 条件键按（真实表, 列）解析、配对正确（失败边界——修复前裸列名查表静默串列或缺列 panic）

#### Scenario: 两表单步零回归

- **GIVEN** 既有两表 JOIN 矩阵
- **WHEN** 全形态查询与直构 `JoinConfig` 执行
- **THEN** 行为逐字节零变化
