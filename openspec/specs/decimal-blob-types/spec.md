# decimal-blob-types Specification

## Purpose

规定 DECIMAL 与 BLOB 的值布局、列声明、精确写入、同类型比较，以及旧库打开后的查询和写入。文件格式版本保持 1，目录行不增加字段。

## Requirements

### Requirement: DECIMAL 与 BLOB 的值布局

`Value` SHALL 具备 `Decimal { coefficient: i128, scale: u8 }` 和 `Blob(Vec<u8>)`。DECIMAL 的数值是系数乘以 10 的负 scale 次方。scale 的合法范围是 0 到 38。系数的十进制位数 SHALL NOT 超过 38。

tuple 中 DECIMAL 使用标签 `0x09`，负载为 `u8 scale` 加小端 `i128` 系数。BLOB 在长度不超过 65535 时使用标签 `0x0A` 和 `u16` 长度；更长时使用标签 `0x0B` 和 `u32` 长度。owned 与零拷贝反序列化 SHALL 读回同一值。未知标签仍按现有错误拒绝。

文件格式版本 SHALL 保持 1。`__tables` 与 `__columns` 的 G1、G2、G3 布局 SHALL NOT 增加字段。新标签只出现在类型字节和值字节里。打开旧库时 SHALL NOT 改写已有列的类型标签。

#### Scenario: 新标签往返

- **GIVEN** 系数 150、scale 2 的 DECIMAL，以及字节 `68 69` 的 BLOB
- **WHEN** 按各自列类型序列化再反序列化
- **THEN** 两个值与原来相同

#### Scenario: 旧标签仍然可读

- **GIVEN** 只含 Int、String、Float、Bool、Date、Timestamp 的既有 tuple
- **WHEN** 用当前解码读取
- **THEN** 值与加入新标签之前相同

### Requirement: 声明类型

`DECIMAL(p,s)` 与 `NUMERIC(p,s)` SHALL 成为精度 `p`、小数位 `s` 的列。`p` 为 1 到 38，`s` 为 0 到 `p`。`DECIMAL(p)` 与 `NUMERIC(p)` 的小数位为 0。未写精度、精度或小数位越界时 SHALL 拒绝建表，错误文本包含 `requires precision` 或 `scale must not exceed precision`，库中没有这张表。

`BLOB`、`BYTEA`、`BINARY`、`VARBINARY` 在不带长度时 SHALL 成为 BLOB 列。带长度时 SHALL 拒绝，错误文本包含 `does not take a length`，且不建表。

schema 与 dump 的 DDL SHALL 写出 `DECIMAL(p,s)` 和 `BLOB`。

#### Scenario: 合法声明可见

- **GIVEN** 空库
- **WHEN** 执行 `CREATE TABLE t (id INTEGER PRIMARY KEY, d DECIMAL(10,2), b BLOB)` 再查看 schema
- **THEN** 输出含 `DECIMAL(10,2)` 和 `BLOB`

#### Scenario: 缺少精度不建表

- **GIVEN** 空库
- **WHEN** 执行 `CREATE TABLE t (d DECIMAL)`
- **THEN** 语句失败，文本包含 `requires precision`
- **AND** 库中没有表 `t`

### Requirement: 精确写入与读回

写入 DECIMAL 列时，整数记号、数值记号和字符串 SHALL 按十进制原文解析。结果能按列的小数位精确缩放时写入缩放后的系数；需要舍入或精度放不下时 SHALL 拒绝，错误文本包含 `expects`，已有行不变。`Float` SHALL NOT 转成 DECIMAL。

`X'6869'` SHALL 写入字节 `68 69`。空 BLOB SHALL 能以 `Value::Blob` 的空字节经参数写入并读回。字符串写入 BLOB 列 SHALL 拒绝。

关闭数据库再打开后，已提交的 DECIMAL 与 BLOB SHALL 读回相同值。5000 字节的 BLOB SHALL 经既有行级溢出链存取，读回字节相同。

dump 再 restore 再 dump，含这两种列的文本 SHALL 相同。

#### Scenario: 精确值读回

- **GIVEN** 列 `d DECIMAL(10,2)` 和 `b BLOB`
- **WHEN** 写入数值 `1.50`、整数 `12`、字符串 `'1.50'` 和 `X'6869'`，再 SELECT
- **THEN** DECIMAL 读回分别为 1.50、12.00 和 1.50
- **AND** BLOB 读回字节 `68 69`

#### Scenario: 不能精确表示的小数被拒绝

- **GIVEN** 列 `d DECIMAL(10,2)` 为空
- **WHEN** 写入 `1.234`
- **THEN** 语句失败，文本包含 `expects`
- **AND** 表仍是空的

#### Scenario: 重开与超页字节不变

- **GIVEN** 已提交 1.50 和一个 5000 字节的 BLOB
- **WHEN** 关闭后再打开并查询
- **THEN** DECIMAL 仍是 1.50，BLOB 的 5000 字节相同

#### Scenario: dump 两代相同

- **GIVEN** 表含上述 DECIMAL 与非空 BLOB
- **WHEN** dump 到空库并 restore，再 dump
- **THEN** 第二次 dump 文本与第一次相同

### Requirement: 同类型比较

DECIMAL 与 DECIMAL SHALL 按数值比较。1.50 与 1.5 相等。BLOB 与 BLOB SHALL 按无符号字节序比较。任一侧是这两种类型而另一侧不是同一变体时，比较 SHALL 失败，错误文本包含 `Type mismatch`。字符串字面量 SHALL NOT 在比较时被解析成这两种类型。

两种类型的 `to_key` 与二级索引键 SHALL 返回 `None`。用 DECIMAL 或 BLOB 作主键时，等值过滤 SHALL 仍能读到该行。

`+` 作用于 DECIMAL SHALL 失败，文本包含 `Type mismatch`，行不变。

#### Scenario: 数值序与尺度

- **GIVEN** DECIMAL 列含 1.50 和 2.00
- **WHEN** 执行 `WHERE d > 1.5` 和 `ORDER BY d`
- **THEN** 过滤命中 2.00，排序先 1.50 后 2.00
- **AND** `WHERE d = 1.5` 命中 1.50

#### Scenario: 跨类型拒绝

- **GIVEN** DECIMAL 列 `d`
- **WHEN** 执行 `WHERE d = '1.50'`
- **THEN** 语句失败，文本包含 `Type mismatch`

#### Scenario: 主键等值可达

- **GIVEN** `d DECIMAL(10,2)` 是主键，且已写入 1.50
- **WHEN** 执行 `WHERE d = 1.50`
- **THEN** 该行返回

### Requirement: 旧库打开结果

`tests/fixtures/legacy/legacy_g1.db` SHALL 仍能打开。`SELECT * FROM users` SHALL 仍返回原来的三行，其中第一行是整数 1、字符串 alice、浮点 9.5。向该表插入一行整数主键后 SHALL 能读回，原有行仍在。打开和这次写入 SHALL NOT 因为本变更改版文件头或重写未涉及的目录代际。

#### Scenario: G1 样本打开查询并写入

- **GIVEN** 样本 `tests/fixtures/legacy/legacy_g1.db` 的副本
- **WHEN** 打开并执行 `SELECT * FROM users`，再插入 `(4, 'dave', 6.0)` 后重新查询
- **THEN** 原有 alice 行仍在，新行可读
- **AND** 打开没有因为格式版本或目录代际失败
