## ADDED Requirements

### Requirement: UPDATE SET 值表达式

`UPDATE ... SET col = <右值>` 的右值 SHALL 接受完整值表达式：列引用（`SET v = v + 1`，逐行对命中行求值）、算术运算、`CAST`、标量函数、`CASE`/`COALESCE`。多列赋值 SHALL 全部基于同一旧行快照求值（标准 SQL 语义，赋值左至右互不影响）。求值结果 SHALL 经既有写入边界强制：写入值类型一致门（类型不符计划点名拒绝零副作用、FLOAT 列 Int 结果升格、日期族 coerce 后一致）、NOT NULL 强制、键列类型门（Int 键列只接受 Int/NULL）与既有 rekey 路径（删旧键插新键、碰撞预检零副作用）SHALL 照常生效。SET 右值中的聚合调用 SHALL 计划期拒绝；子查询 SHALL 维持既有 UPDATE 拒绝边界。`ON CONFLICT DO UPDATE SET` 三形态与 `REPLACE INTO` 既有语义 SHALL 零变化。

#### Scenario: 列引用表达式按旧行求值

- **GIVEN** 表 `t(id INTEGER PRIMARY KEY, v INTEGER)`，行 (1, 10)
- **WHEN** `UPDATE t SET v = v + 1 WHERE id = 1`
- **THEN** v 更新为 11，仅命中行变更（失败边界——修复前：`Unsupported value type`）

#### Scenario: 常量表达式与函数右值

- **GIVEN** 同上表
- **WHEN** `SET v = 1 + 2`；`SET name = upper('ada')`；`SET v = CAST('42' AS INTEGER)`
- **THEN** 全部生效（常量表达式逐行一致）

#### Scenario: 多列同旧行快照

- **GIVEN** 行 (1, 10, 'x')（列 a、b）
- **WHEN** `UPDATE t SET a = a + 1, b = a WHERE id = 1`（b 取 a 的旧值）
- **THEN** a = 11 且 b = 10（全部右值基于同一旧行，赋值顺序不影响）

#### Scenario: 类型门与约束照常

- **GIVEN** FLOAT 列、NOT NULL 列、DATE 列的表
- **WHEN** 表达式结果跨类型（Int 列得 String）；表达式求值为 NULL 落 NOT NULL 列；DATE 列 `SET d = '2026-01-01'`
- **THEN** 跨类型计划期/执行期点名拒绝零副作用；NOT NULL 零副作用拒绝；日期 coerce 后写入成功；FLOAT 列 Int 结果无损升格

#### Scenario: 键列表达式 rekey

- **GIVEN** `id INTEGER PRIMARY KEY`
- **WHEN** `UPDATE t SET id = id + 100 WHERE id = 1`；以及更新结果与新键碰撞的形态
- **THEN** rekey 语义（删旧键插新键）生效；碰撞预检零副作用拒绝；键列表达式结果非 Int/NULL 时 `KeyTypeMismatch`

#### Scenario: 拒绝面保持

- **GIVEN** 同上表
- **WHEN** `SET v = MAX(v)`；`SET v = (SELECT MAX(v) FROM t)`；`ON CONFLICT DO UPDATE SET` 与 `REPLACE INTO` 既有形态回归
- **THEN** 前两者计划期拒绝；后两者语义零回归
