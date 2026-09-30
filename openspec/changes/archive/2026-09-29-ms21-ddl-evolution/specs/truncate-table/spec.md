## ADDED Requirements

### Requirement: TRUNCATE TABLE 物理清空保留定义

`TRUNCATE TABLE <t>`（与 `TRUNCATE <t>` 等价）SHALL 物理释放该表全部数据页（DROP TABLE 同型 free-list 通道）并重建空数据面：表定义（名称、列结构、DEFAULT、NOT NULL/UNIQUE 声明）、索引定义（含二级索引）SHALL 完整保留，索引 SHALL 重建为空；`TRUNCATE` 后查询 SHALL 返回空集、schema 查询面不变、新写入正常可用，结果 SHALL 跨重启持久。TRUNCATE SHALL NOT 产生墓碑或可回滚的行级痕迹（物理语义，DELETE 的 MVCC 语义不适用）；显式事务内 TRUNCATE SHALL 执行期点名拒绝（物理页释放不可回滚；CTAS/CREATE INDEX 事务内拒绝先例同型，拒绝先于执行器产生任何副作用）且零副作用。

多表 `TRUNCATE t1, t2` SHALL 计划期点名拒绝（首版单表）；表不存在、目标为视图或系统保留名 SHALL 点名拒绝；对空表 TRUNCATE SHALL 幂等成功。

#### Scenario: 清空后查询空集且 schema 不变

- **GIVEN** 表 `t(id INT PRIMARY KEY, v VARCHAR)` 含若干行，已建 `CREATE INDEX idx_v ON t(v)`（若在 T2 之后执行）
- **WHEN** `TRUNCATE TABLE t`
- **THEN** `SELECT COUNT(*) FROM t` 为 0；`SELECT * FROM t WHERE v = 'x'` 空集；`list`/`schema` 面表与列定义不变；`INSERT INTO t VALUES (1, 'x')` 后查询正常

#### Scenario: 物理释放与重开持久

- **GIVEN** TRUNCATE 后显式 close
- **WHEN** 重新打开数据库
- **THEN** 表存在且为空，行不复活（无墓碑恢复）；库文件体积反映页释放（不要求精确断言，允许运行期分配噪声）

#### Scenario: 显式事务内拒绝

- **GIVEN** CLI 会话已 `BEGIN`
- **WHEN** `TRUNCATE TABLE t`
- **THEN** 点名拒绝且零副作用（表数据不变），会话事务状态不变

#### Scenario: 多表与目标边界拒绝

- **WHEN** `TRUNCATE TABLE t1, t2` / `TRUNCATE nope` / `TRUNCATE some_view`
- **THEN** 均点名拒绝（多表不支持 / 表不存在 / 目标非基表）

#### Scenario: 空表幂等

- **GIVEN** 表 `t` 已空（或刚建）
- **WHEN** `TRUNCATE TABLE t`
- **THEN** 成功，无错误

### Requirement: 既有语义零回归

DELETE、DROP TABLE 既有语义与物理释放通道 SHALL 保持不变；TRUNCATE SHALL NOT 影响 MVCC 版本链、WAL 记录格式与恢复重放；全部既有测试 SHALL 零修改通过（语义校准白名单除外）。

#### Scenario: DELETE 与 DROP TABLE 零回归

- **WHEN** 运行完整测试套件（含 `drop_table_free_test`、墓碑可见性矩阵）
- **THEN** 全部通过；除本 change 新增见证外既有测试零修改；`cargo clippy -- -D warnings`、`cargo fmt --check`、`openspec validate` 通过
