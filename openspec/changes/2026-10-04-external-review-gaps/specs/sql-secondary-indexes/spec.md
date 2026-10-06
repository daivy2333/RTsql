## ADDED Requirements

### Requirement: IndexScanAll 列元数据与输出形状契约

二级索引等值路由节点（IndexScanAll）的列元数据 SHALL 遵循统一契约：节点 `columns` 恒为基表全列名清单，真实输出形状由 `projection`（基表列号）唯一决定。任意 SELECT 投影组合——单列（含后位列与索引列自身）、乱序多列、通配——SHALL 产出与请求一致且表头与行逐位对齐的结果，SHALL NOT panic；`EXPLAIN` 渲染 SHALL 遵循同一契约（计划树显示真实输出形状）。以该扫描为输入的聚合/分组、派生表、CTAS、视图、INSERT SELECT 的列解析 SHALL 按基表列号语义正确映射。PK 等值路由（IndexScan，「columns 已收窄」契约）与非索引回落路径 SHALL 零变化。

#### Scenario: 后位单列投影不 panic 且表头正确

- **GIVEN** 表 `q6(id INTEGER PRIMARY KEY, name TEXT, dept_id INTEGER, salary INTEGER)` 与 `CREATE INDEX idx_q6_sal ON q6 (salary)`，行 (1,'Ada',1,90)
- **WHEN** `SELECT name FROM q6 WHERE salary = 90`
- **THEN** 表头 `["name"]`、行 `[["Ada"]]`（失败边界——修复前：`index out of bounds` panic，exit 101）

#### Scenario: 乱序多列投影表头与行对齐

- **GIVEN** 同上
- **WHEN** `SELECT name, id FROM q6 WHERE salary = 90`
- **THEN** 表头 `["name","id"]`、行 `[["Ada",1]]` 逐位对齐（失败边界——修复前：静默错位，表头 `["id","name"]`）

#### Scenario: EXPLAIN 渲染遵循同一契约

- **GIVEN** 同上
- **WHEN** `EXPLAIN SELECT name FROM q6 WHERE salary = 90`
- **THEN** 计划树文本正常输出且含 IndexScanAll 与真实输出形状（失败边界——修复前：`plan_format.rs` panic）

#### Scenario: PK 路由与范围谓词零回归

- **GIVEN** 同上
- **WHEN** `SELECT name FROM q6 WHERE id = 1`；`SELECT name FROM q6 WHERE salary > 80`；`SELECT * FROM q6 WHERE salary = 90`
- **THEN** 全部行为与修复前一致（既有契约与回落路径不破坏）

#### Scenario: 二级索引扫描上的聚合映射正确

- **GIVEN** 同上
- **WHEN** `SELECT COUNT(*), MAX(id) FROM q6 WHERE salary = 90`
- **THEN** 聚合结果正确（聚合输入 schema 按基表列号映射，不再受错位元数据影响）
