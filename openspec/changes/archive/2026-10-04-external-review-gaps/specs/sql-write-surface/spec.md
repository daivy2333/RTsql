## ADDED Requirements

### Requirement: UPDATE 与 DELETE 的无 WHERE 全表形态

`UPDATE <table> SET …` 与 `DELETE FROM <table>` 省略 WHERE 子句时 SHALL 作用于表的全部可见行，行级语义与带 WHERE 的命中行完全同源：逐行版本写/墓碑（MVCC）、per 行写入类型一致门（FLOAT 列 Int 升格、日期族 coerce）、NOT NULL 强制、键列类型门与 rekey 碰撞预检、唯一与二级索引维护 SHALL 照常逐行生效；`affected_rows` SHALL 为实际处理的行数。语句执行中途任一行失败（碰撞预检、类型门、NOT NULL）SHALL 隐式事务回滚全部已写行零残留；显式事务内 SHALL 可用且可回滚（与 TRUNCATE 的 checkpoint-first 物理释放语义并存、互不替代）。带 WHERE 的形态 SHALL 零变化：主键等值照常、非主键等值一般谓词维持既有点名拒绝、子查询条件/赋值维持拒绝。TRUNCATE TABLE 既有语义 SHALL 零变化。

#### Scenario: 全表 DELETE

- **GIVEN** 表 t 有 N 行
- **WHEN** `DELETE FROM t`
- **THEN** affected_rows=N，逐行墓碑；重开一致；dump 无该表行（失败边界——修复前：`Missing required field: WHERE clause for DELETE` exit 3）

#### Scenario: 全表 UPDATE 表达式

- **GIVEN** 同上
- **WHEN** `UPDATE t SET v = v + 1`
- **THEN** 全部行 v+1（逐行旧行快照）；FLOAT 列升格 / NOT NULL / 键列类型门 per 行生效

#### Scenario: 逐行碰撞回滚

- **GIVEN** id INTEGER PRIMARY KEY 表，全体 rekey 形态（如 `SET id = id + 100`）且存在中途碰撞
- **WHEN** 无 WHERE UPDATE
- **THEN** 语句失败，全部已写行回滚零残留；显式事务内可手动 ROLLBACK

#### Scenario: 附加库目标

- **GIVEN** ATTACH 附加库
- **WHEN** 附加表无 WHERE UPDATE/DELETE
- **THEN** 经既有路由同语义执行，每文件独立提交（cross-db-dml 修订面）

#### Scenario: 既有边界零变化

- **GIVEN** 同库
- **WHEN** `TRUNCATE TABLE t`；带 WHERE 主键等值；带 WHERE 非主键一般谓词；WHERE 子查询形态
- **THEN** TRUNCATE 与主键等值语义零变化；一般谓词与子查询维持既有点名拒绝
