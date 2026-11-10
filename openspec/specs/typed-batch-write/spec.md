# typed-batch-write Specification

## Purpose

规定逐行插入占写入语句的测量，以及该比例达到 10% 时类型化批量写入入口的行为。未达到 10% 时不新增入口。本次测量 `total_ns = 332234286`，`execute_exclusive_ns = 9804119`，`9804119 * 10 < 332234286`。库没有 `Database::insert_batch`。

## Requirements

### Requirement: 一百条逐行插入的占比用诊断阶段测量

系统 SHALL 能对下列负载读出两个时间。文件数据库上已有表 `bench (id INT PRIMARY KEY, name STRING, value INT)`。诊断只打开性能统计，摘要上限至少 100。随后执行 100 条互不相同的单行插入，文本形状与 `benches/common/mod.rs` 的 `insert_rows` 相同：`INSERT INTO bench VALUES (i, 'user_i', i * 10)`，`i` 从 0 到 99。

这 100 条语句的语句总耗时之和是分母。每条语句执行阶段的独占时间之和是分子。提交是执行的子阶段，提交的包含时间 SHALL NOT 计入分子。语句总耗时 SHALL 使用该语句根阶段的包含耗时，SHALL NOT 把各阶段包含时间相加后代替。

测量 SHALL 打印 `total_ns` 与 `execute_exclusive_ns`。打印本身 SHALL NOT 判定该比例是否达到 10%。测量 SHALL NOT 改变这 100 行的插入结果。

#### Scenario: 一百条插入给出两个时间且行可见

- **GIVEN** 上述表已经建好，诊断在建表之后才打开
- **WHEN** 执行这 100 条插入
- **THEN** 得到 100 条成功摘要，每条都有解析、规划、执行和提交
- **AND** 每条的执行阶段独占时间加上提交的包含时间等于该执行阶段的包含时间
- **AND** 输出中有这 100 条的 `total_ns` 与 `execute_exclusive_ns`
- **AND** 随后查询 `bench` 得到 100 行

#### Scenario: 总耗时为零时测量无效

- **GIVEN** 上述负载
- **WHEN** 语句总耗时之和为 0，或任一插入缺少执行或提交阶段
- **THEN** 这次测量不能用来裁定入口
- **AND** 不得把该次结果记成低于 10%

### Requirement: 达到百分之十才提供批量入口

10% 是本变更是否交付入口的条件，不是运行时分支。库 SHALL NOT 在每次写入时读取基准或诊断占比来决定走哪条路径。测量满足 `execute_exclusive_ns * 10 >= total_ns` 并且本变更因此实现入口之后，库 SHALL 提供：

`Database::insert_batch(&self, table: &str, rows: &[Vec<Value>]) -> Result<SqlOutcome, SqlError>`

调用 SHALL 把 `table` 用 Rust `str::to_lowercase` 当成一个目录名，SHALL NOT 按 `.` 拆成附加库路由。检查顺序 SHALL 为：

1. 小写名在视图注册表中时，错误文本和 SQLSTATE 与 `execute_sql` 对该视图的 `INSERT` 相同，不打开隐式事务。空切片也适用。
2. 目录中没有该基表时，错误文本和 SQLSTATE 与 `execute_sql` 对这个不存在的表名做 `INSERT` 相同，不打开隐式事务。空切片也适用。
3. `rows` 为空时，返回 `Ok(SqlOutcome::AffectedRows(0))`，不调用 `begin`，表的行不变。只读会话也返回这个结果。
4. 任一行的长度与目录列数不同时，对第一行不匹配的行，错误文本和 SQLSTATE 与不带列清单、值个数相同的 `execute_sql` `INSERT` 相同，不打开隐式事务。
5. 其余情况构造不带列清单的 `Insert` 计划，经 `execute_stage` 执行。类型检查、`NOT NULL`、主键、`UNIQUE`、二级索引、只读拒绝、隐式事务的提交和中止与同一批值的多行 `INSERT` 相同。

成功时受影响行数 SHALL 等于 `rows` 的长度。显式 `Value::Null` SHALL 落成 NULL，SHALL NOT 改写成该列的声明默认值。入口 SHALL NOT 接受列清单、`ON CONFLICT` 或显式事务句柄。`begin` 之后的 `insert_batch` SHALL 自己提交，随后对该显式事务 `rollback` SHALL NOT 撤掉这批行。

任一行失败时，本批已写入的行 SHALL 全部不可见，对应索引条目 SHALL NOT 留下。错误文本和 SQLSTATE SHALL 与同值多行 `INSERT` 相同。入口 SHALL NOT 提供部分成功。

本入口 SHALL NOT 新增诊断阶段名，也 SHALL NOT 改变 `execute_sql` 的插入结果、错误文本或 SQLSTATE。

#### Scenario: 多行与索引和逐行插入一致

- **GIVEN** 测量已经达到 10%，表有二级索引
- **WHEN** `insert_batch` 写入多行全宽值
- **THEN** 受影响行数等于行数
- **AND** 查询结果和经该二级索引的查找与同值多行 `INSERT` 一致

#### Scenario: 类型与约束拒绝和逐行插入相同

- **GIVEN** 测量已经达到 10%，表上有 `NOT NULL` 列和 `UNIQUE` 列
- **WHEN** 分别用 `insert_batch` 写入类型不符的行、违反 `NOT NULL` 的行，以及违反 `UNIQUE` 的行
- **THEN** 每一次的错误文本和 SQLSTATE 都与同值的 `execute_sql` 插入相同
- **AND** 每一次调用都没有留下新行

#### Scenario: 后一行失败则整批不可见

- **GIVEN** 测量已经达到 10%，批次的后一行与前面的主键冲突
- **WHEN** 调用 `insert_batch`
- **THEN** 返回的错误文本和 SQLSTATE 与同值多行 `INSERT` 相同
- **AND** 本批每一行都不可见
- **AND** 二级索引没有本批的新条目

#### Scenario: 缺表、视图和行宽在写入前拒绝

- **GIVEN** 测量已经达到 10%
- **WHEN** 表名不存在、表名是视图，或某一行宽度不符，包括空切片打到缺表或视图上
- **THEN** 错误文本和 SQLSTATE 与对应的 `execute_sql` 插入相同
- **AND** 没有新的已提交行

#### Scenario: 空批次不写行

- **GIVEN** 测量已经达到 10%，目标是已存在的基表
- **WHEN** `rows` 为空，包括只读会话
- **THEN** 返回受影响行数 0
- **AND** 表的行数不变

#### Scenario: 显式 NULL 不套用默认值

- **GIVEN** 测量已经达到 10%，某列声明了 DEFAULT
- **WHEN** `insert_batch` 在该列传入 `Value::Null`
- **THEN** 读回的值是 NULL
- **AND** 与不带列清单、该位置写 NULL 的 `INSERT` 相同

#### Scenario: 不加入显式事务

- **GIVEN** 测量已经达到 10%，调用方已经 `begin`
- **WHEN** `insert_batch` 成功，然后 `rollback` 那个显式事务
- **THEN** `insert_batch` 写入的行仍然可见

### Requirement: 低于百分之十时不新增入口

当 `execute_exclusive_ns * 10 < total_ns` 时，库 SHALL NOT 新增批量写入入口。`execute_sql` 的插入行为 SHALL 保持测量前的行为。变更产物 SHALL 记下两个纳秒数和「不建入口」。

#### Scenario: 未达标不增加方法

- **GIVEN** 测量的执行阶段独占时间之和乘以 10 小于语句总耗时之和
- **WHEN** 本次实现结束
- **THEN** 产品代码中没有 `insert_batch`
- **AND** 变更产物含有两个纳秒数和「不建入口」
- **AND** 原有单行 `INSERT` 仍然成功并提交
