# table-name-resolution Specification（delta）

## ADDED Requirements

### Requirement: import 表名实参转义可达

`import` 子命令构造 `INSERT INTO <表名>` 时 SHALL 对表名实参经 `quote_ident` 转义包裹（与 dump、`select_all_rows`、分析命令同源），使含引号字符的 catalog 表名（经转义 delimited ident 建表可达，如 `CREATE TABLE "a""b"(…)` → catalog 名 `a"b`；以及历史带引号 restore 产物的完整带引号名）经 import 实参约定可达。表名实参与 catalog 的匹配语义 SHALL 保持逐字比对不变。裸名表（不含引号字符）的 import 行为 SHALL 逐字节保持。

#### Scenario: 含引号字符表名的 import 可达

- **GIVEN** 库中经 `CREATE TABLE "a""b"(i INT)` 建有 catalog 名为 `a"b` 的表，CSV 文件表头为 `i` 且含数据行
- **WHEN** `rtsql <db> import <db> 'a"b' <csv> --csv`
- **THEN** import 成功（affected 行数等于 CSV 数据行数），数据经 `SELECT * FROM "a""b"` 可回读（change 前该形态在 SQL 解析面报错）

#### Scenario: 裸名表 import 行为不变

- **GIVEN** 库中有裸名表 `items(i INT)` 与合法 CSV
- **WHEN** `rtsql <db> import <db> items <csv> --csv`
- **THEN** 行为与 change 前逐字节一致（构造出的 INSERT 语句解析后表名同为裸名）
