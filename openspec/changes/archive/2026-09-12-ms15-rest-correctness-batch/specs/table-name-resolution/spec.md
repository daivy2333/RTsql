# table-name-resolution Specification

## Purpose

约束引擎对语句中表名的解析语义：表名 SHALL 取标识符的去引号值（`Identifier.value`），带引号与裸名拼写等价；dump/schema 产出的 DDL 在 restore 后表名保真。来源：MS15-Rest（change `2026-09-12-ms15-rest-correctness-batch`，improvements I039）。

## ADDED Requirements

### Requirement: 标识符表名解析归一化

引擎对 SQL 语句中表名的解析 SHALL 使用标识符的去引号值（sqlparser `Identifier.value`，小写化），SHALL NOT 使用含引号字符的 Display 形式。同一表 SHALL 可经带引号与裸名两种拼写等价访问，适用语句 SHALL 覆盖 CREATE TABLE、DROP TABLE、SELECT（含 JOIN 与子查询表引用）、INSERT、UPDATE、DELETE。带引号拼写中的引号转义（`""` → `"`）SHALL 按标识符语义解析。

#### Scenario: 带引号建表后裸名访问命中

- **GIVEN** 任意可建库环境
- **WHEN** `CREATE TABLE "items" (id INT PRIMARY KEY, n INT)` 后执行 `INSERT INTO items VALUES (1, 10)` 与 `SELECT n FROM items WHERE id = 1`
- **THEN** 全部成功，返回 `(10)`（修复前 INSERT/SELECT 因表名 `"items"` ≠ `items` 报表不存在）

#### Scenario: 带引号与裸名拼写等价互访

- **GIVEN** `CREATE TABLE items (id INT PRIMARY KEY, n INT)` 已执行且含行 `(1, 10)`
- **WHEN** 分别以 `SELECT * FROM "items"`、`INSERT INTO "items" VALUES (5, 50)`、`UPDATE "items" SET n = 99 WHERE id = 1`、`DELETE FROM "items" WHERE id = 5`、`DROP TABLE "items"` 访问
- **THEN** 各语句命中同一表 `items`，行为与裸名拼写一致（UPDATE SET 非键列、DELETE 删未触及键——键列 rekey 与同键跨进程 UPDATE→DELETE 组合的行级语义不属本 scenario 见证面）

#### Scenario: 引号转义按标识符语义解析

- **GIVEN** `CREATE TABLE """items""" (id INT)` 形式的 DDL（标识符内容为 `"items"`）
- **WHEN** 执行后查询 catalog 与 dump 输出
- **THEN** 表名为内容 `"items"`（含引号字符），多代 dump/restore 对该名 SHALL 恒等不继续膨胀

#### Scenario: UPDATE/DELETE 表名解析归一化

- **GIVEN** `CREATE TABLE "logs" (id INT PRIMARY KEY, n INT)` 已执行且含行 `(1, 100)`、`(2, 200)`
- **WHEN** `UPDATE logs SET n = 1000 WHERE id = 1` 与 `DELETE FROM logs WHERE id = 2`
- **THEN** 均命中表 `logs` 正常执行（修复前报表不存在；UPDATE SET 非键列——键列 rekey 行级语义不属本 scenario 见证面）

### Requirement: dump 与 schema 表名保真

`dump` 与 `schema` 输出的 `CREATE TABLE` DDL 经 `restore`（或主命令执行）重建后 SHALL 得到同名表；对不含引号字符的表名，`dump → restore → dump` 的 DDL 表名拼写 SHALL 恒等（多代不膨胀）。

#### Scenario: dump-restore-dump 表名恒等

- **GIVEN** 库 `a` 含裸名建表 `mixed` 与若干行
- **WHEN** `dump a`（第 1 代）→ `restore` 到空库 `b` → `dump b`（第 2 代）
- **THEN** 两代 dump 中该表的 `CREATE TABLE` 行文本一致（修复前第 2 代表名为 `"""mixed"""` 引号膨胀）；库 `b` 中 `SELECT` 经 `mixed` 与 `"mixed"` 拼写均可访问

#### Scenario: schema 输出可重建同名表

- **GIVEN** 库 `a` 含带引号建表 `"items"`（归一化后目录名为 `items`）
- **WHEN** `schema a` 输出 DDL 在另一空库执行
- **THEN** 重建出同名表（`items`），与源库目录名一致

### Requirement: 既有裸名语义零回归

裸名建表/访问全链路 SHALL 与本 change 前一致；既有 dump/restore 往返（`test_dump_restore_roundtrip_full_shape`）、生命周期子命令、import --csv 套件 SHALL 零修改通过；既有测试套件 SHALL 除本 change 新增见证外零修改。

#### Scenario: 裸名全链路与既有测试零回归

- **WHEN** 运行完整测试套件与静态检查（`cargo test`、`cargo clippy -- -D warnings`、`cargo fmt --check`、`openspec validate`）
- **THEN** 全部通过，既有测试文件除本 change 新增见证外零修改
