# sql-expression-evaluation Specification

## Purpose
SQL 表达式求值能力。WHERE 谓词四件套（`[NOT] IN` / `[NOT] BETWEEN` / `[NOT] LIKE` / `IS [NOT] NULL` / `NOT`）按 SQL 三值 NULL 语义求值（True/False/Unknown，行选择将 Unknown 折叠为不匹配），新谓词与既有 Filter/DataScan 下推路径共享同一 `Predicate` 求值器；值表达式 CASE（searched/simple）、COALESCE、CAST 既作谓词操作数又作非聚合查询的 SELECT 投影项（派生列，`AS` 别名或表达式 Display 文本命名）；INSERT VALUES 接受负数字面量。全部变更 additive：既有比较/AND/OR 行选择结果、错误文案与纯列查询计划不变。来源：MS11-T01（change `2026-09-10-ms11-t01-sql-expressions`，2026-09-10 归档）。

## Requirements

### Requirement: 谓词表达式四件套

非聚合查询的 WHERE 子句 SHALL 支持 `[NOT] IN (值列表)`、`[NOT] BETWEEN <low> AND <high>`、`[NOT] LIKE <模式>`、`IS [NOT] NULL` 与 `NOT <谓词>`。新谓词 SHALL 与 Filter 执行器共享同一 `Predicate` 求值器，并 SHALL 在满足既有下推门槛（不含 OR、非 PK 等值形态）时装入 DataScan 行内过滤；不满足时 SHALL 保留 Filter 节点，两种路径产出等价。`ESCAPE` 子句、ILIKE/RLIKE/SIMILAR TO SHALL 显式拒绝。LIKE 任一操作数非 String 时 SHALL 产生执行期类型错误。

#### Scenario: IN 匹配与否定

- **GIVEN** 表 t 含整数列 id，行值 (1,2,3)
- **WHEN** `SELECT id FROM t WHERE id IN (1, 3)`；随后 `WHERE id NOT IN (1, 3)`
- **THEN** 前者返回 id 1 和 3 两行，后者返回 id 2 一行

#### Scenario: BETWEEN 含端点

- **GIVEN** 表 t 含行值 id (1,2,3,4)
- **WHEN** `WHERE id BETWEEN 2 AND 3`；随后 `WHERE id NOT BETWEEN 2 AND 3`
- **THEN** 前者返回 2、3；后者返回 1、4

#### Scenario: LIKE 通配符与否定

- **GIVEN** 表 t 含字符串列 name，行值 ('Alice','Bob','Carol')
- **WHEN** `WHERE name LIKE 'A%'`；随后 `WHERE name LIKE '_ob'`；随后 `WHERE name NOT LIKE '%o%'`
- **THEN** 依次返回 Alice；Bob；Alice（Alice 不含 `o`；Bob/Carol 含 `o` 被 LIKE 命中而排除）

#### Scenario: IS NULL 与 IS NOT NULL

- **GIVEN** 表 t 含可空列 v，行值 (1, NULL, 3)
- **WHEN** `WHERE v IS NULL`；随后 `WHERE v IS NOT NULL`
- **THEN** 前者返回 NULL 行；后者返回 1、3 两行

#### Scenario: NOT 复合谓词

- **GIVEN** 表 t 含列 a、b
- **WHEN** `WHERE NOT (a = 1 OR b = 2)`
- **THEN** 返回既非 a=1 也非 b=2 的行

#### Scenario: 下推与 Filter 等价

- **GIVEN** 同一表与同一新谓词（如 `v BETWEEN 2 AND 3`）
- **WHEN** 分别以 DataScan 装入谓词（无 OR 路径）与保留 Filter（与 PK 等值 AND 组合路径）执行
- **THEN** 两者返回相同行集；plan 断言确认谓词分别位于 `DataScanNode.predicate` 与 `FilterNode.predicate`

#### Scenario: 不支持形态显式拒绝

- **GIVEN** `WHERE name LIKE 'A%' ESCAPE '\'`、`WHERE name ILIKE 'a%'`
- **WHEN** 构建计划
- **THEN** 报计划错误（exit 3），文案指明不支持的具体形态，既有错误文案不变

### Requirement: NULL 三值语义

谓词求值 SHALL 在内部采用 True/False/Unknown 三值：比较与新算子的操作数遇 NULL 产生 Unknown；AND/OR SHALL 按 SQL 三值表组合（Unknown AND True = Unknown，Unknown OR True = True 等）；`NOT Unknown = Unknown`；最终行选择 SHALL 将 Unknown 折叠为不匹配（排除该行）。无 NOT 的既有比较/AND/OR 形态的行选择结果 SHALL 与本 spec 生效前一致。

#### Scenario: NOT IN 含 NULL 排除全部

- **GIVEN** 表 t 含可空列 v，行值 (1, 2)
- **WHEN** `WHERE v NOT IN (2, NULL)`
- **THEN** 返回 0 行（SQL 标准：比较遇 NULL 为 Unknown，NOT Unknown 仍 Unknown）

#### Scenario: 既有比较 NULL 行为不变

- **GIVEN** 表 t 含可空列 v，行值 (NULL, 2)
- **WHEN** `WHERE v = 2`；随后 `WHERE v > 0`
- **THEN** 两者均只返回 v=2 的行（与现状一致，Unknown 折叠为排除）

#### Scenario: OR 组合中 True 胜出

- **GIVEN** 表 t 含可空列 a、普通列 b，行 (a=NULL, b=1)
- **WHEN** `WHERE a = 1 OR b = 1`
- **THEN** 该行命中（Unknown OR True = True）

#### Scenario: IS NULL 不受三值影响

- **GIVEN** 可空列 v 行值 (NULL, 1)
- **WHEN** `WHERE NOT (v IS NULL)`
- **THEN** 返回 v=1 的行（IS NULL 产出 True/False，NOT 正常取反）

### Requirement: 值表达式 CASE / COALESCE / CAST

SQL SHALL 支持 searched CASE（`CASE WHEN <cond> THEN <r> ... [ELSE <e>] END`）与 simple CASE（`CASE <operand> WHEN <v> THEN <r> ... [ELSE <e>] END`），缺省 ELSE 产出 NULL；SHALL 支持 `COALESCE(<expr>, ...)`（≥1 参数，返回首个非 NULL 值，全 NULL 产出 NULL）；SHALL 支持 `CAST(<expr> AS <类型>)`，目标类型限 Int/Float/String/Bool：数值↔字符串按值解析/格式化，Float→Int 截断向零，非法转换产生执行期错误，CAST NULL 产出 NULL。以上值表达式 SHALL 既可作谓词操作数（WHERE）也可作 SELECT 投影项。TRY_CAST SHALL 显式拒绝。

#### Scenario: searched CASE 与缺省 ELSE

- **GIVEN** 表 t 含整数列 score，行值 (95, 40)
- **WHEN** `SELECT CASE WHEN score >= 60 THEN 'pass' END FROM t`
- **THEN** 依次产出 'pass' 与 NULL

#### Scenario: simple CASE 的 operand 为 NULL

- **GIVEN** 表 t 含可空列 v，行值 (NULL, 1)
- **WHEN** `SELECT CASE v WHEN 1 THEN 'one' ELSE 'other' END FROM t`
- **THEN** NULL 行产出 'other'（operand NULL 与 1 比较为 Unknown，不命中），v=1 行产出 'one'

#### Scenario: COALESCE 逐参数取首个非 NULL

- **GIVEN** 表 t 含可空列 a、b，行 (NULL, 'x')、('y', 'z') 与 (NULL, NULL)
- **WHEN** `SELECT COALESCE(a, b, 'fallback') FROM t`
- **THEN** 依次产出 'x'、'y'、'fallback'（全 NULL 行落到末位兜底参数）

#### Scenario: CAST 数值与字符串

- **GIVEN** 表 t 含字符串列 s，行值 ('42', 'abc')；含浮点列 f，行值 (1.7)
- **WHEN** `SELECT CAST(s AS INT) FROM t`；`SELECT CAST(f AS INT) FROM t`；`SELECT CAST(42 AS STRING) FROM t`
- **THEN** 依次产出 42；执行期报错（'abc' 不可解析为 Int）；1（截断向零）；'42'

#### Scenario: 谓词操作数中使用值表达式

- **GIVEN** 表 t 含浮点列 f，行值 (59.5, 60.5)
- **WHEN** `WHERE CAST(f AS INT) >= 60`
- **THEN** 仅返回 60.5 行（CAST(59.5) = 59）

### Requirement: SELECT 派生列（投影表达式）

非聚合查询的 SELECT 列表 SHALL 支持本 spec 定义的值表达式项（CASE/COALESCE/CAST 及既有列引用/字面量），执行时对每行求值并按项输出；SHALL 支持 `AS <别名>` 命名派生列，无别名时列名 SHALL 为表达式 Display 文本。派生列 SHALL 与既有列裁剪投影（`with_projection` 语义：谓词与 MVCC 判定后按投影产出）共存，`SELECT *` 行为不变，CLI 四格式渲染无需改动（产出为既有 `Value` 变体）。聚合查询中的非聚合表达式项 SHALL 保持现有报错。MS13（change `2026-09-23-ms13-analytics-functions`）引入的算术表达式节点经共享编译通路在 SELECT 投影项与 WHERE 比较腿双侧可达：算术比较腿按逐行求值结果参与谓词判定（严格数值面、NULL 传播，与投影项同一节点语义）。

#### Scenario: 派生列输出与命名

- **GIVEN** 表 t 含列 name、score
- **WHEN** `SELECT name, CASE WHEN score >= 60 THEN 'Y' ELSE 'N' END AS passed FROM t`
- **THEN** 输出两列，表头 `["name", "passed"]`，逐行求值；去掉 AS 别名后表头第二列为 CASE 表达式 Display 文本

#### Scenario: 派生列与普通列、字面量混合

- **GIVEN** 表 t 含列 id
- **WHEN** `SELECT id, COALESCE(NULL, id) FROM t`
- **THEN** 每行输出 id 与 id 值；表头 `["id", "COALESCE(NULL, id)"]`

#### Scenario: SELECT * 不变与聚合查询报错保持

- **GIVEN** 既有 `SELECT * FROM t` 与聚合查询 `SELECT COUNT(*), CASE WHEN 1 = 1 THEN 'x' END FROM t`
- **WHEN** 分别执行
- **THEN** 前者行为与现状完全一致；后者保持聚合路径现有报错（非聚合项不可与聚合混用）

#### Scenario: CLI 渲染兼容

- **GIVEN** 派生列查询在 TTY 与非 TTY 环境
- **WHEN** 分别以默认格式与 `--format csv` 执行
- **THEN** table/json/csv/tsv 输出与同形状既有查询渲染规则一致（派生列值均为既有 Value 变体）

#### Scenario: WHERE 算术比较腿（MS13 共享编译通路）

- **GIVEN** 表 t 含列 id，行 id=1、id=2
- **WHEN** `SELECT id FROM t WHERE id + 1 > 2 ORDER BY id`
- **THEN** 仅输出 id=2 行（算术腿逐行求值后参与比较，exit 0；接受记录：Iteration 000 Review F1）

### Requirement: INSERT 负数字面量

INSERT 的 VALUES SHALL 接受带一元负号的数字字面量（`-<number>`），折叠为对应负值；非数字字面量的一元负号 SHALL 保持现有拒绝。

#### Scenario: 负数入库与恢复往返

- **GIVEN** 表 t 含整数列 v 与字符串列 s
- **WHEN** `INSERT INTO t VALUES (-1, 'x')` 后查询；对含该行的库执行 dump → restore 往返
- **THEN** 查询返回 -1；restore 后数据一致（dump 文本的负数可重新导入）

#### Scenario: 非字面量取负保持拒绝

- **GIVEN** 表 t 含整数列 v
- **WHEN** `INSERT INTO t VALUES (-v)`（列引用取负）
- **THEN** 报 `UnsupportedValue` 类计划错误（现状不变）

### Requirement: 既有语义零回归

本 spec 的全部变更 SHALL 为 additive：既有全量测试零修改通过；plan cache 仅缓存 `Statement::Query` 的现状不变；既有错误文案不变、新增错误为 additive；PhysicalPlan 节点集合除按 design D4 新增 `Projection`（第 20 种）外 SHALL 保持 19 种不变；JOIN ON 仅接受等值列条件的现状不变。

#### Scenario: 全量回归零修改

- **GIVEN** 本 spec 生效前的工作区（704 pass / 0 failed / 2 ignored 基线）
- **WHEN** 实施完成后运行全量 `cargo test`
- **THEN** 既有测试文件零修改全部通过，新增测试全部通过，clippy/fmt 零告警

#### Scenario: 新谓词不破坏索引路由

- **GIVEN** 表 t 以整数列为 PK，行含 NULL 键位行（MS10-T05 无键行语义）
- **WHEN** `WHERE id IN (1, 2)` 执行
- **THEN** 无键行不因该谓词形态被静默丢弃（IN 不匹配 PK 等值路由形态，走通用过滤路径）
