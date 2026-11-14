## Why

列类型今天是 Int、String、Float、Bool、Date、Timestamp。`DECIMAL` 和 `BLOB` 在 `convert_data_type` 的兜底臂里变成 String，精确小数和原始字节没有自己的值。MS50 要求这两种类型可以声明、写入、比较和读回，并且旧库打开有明确结果。结果游标用另一组查询验收，不在本变更里。

## What Changes

- 新增 `Decimal` 与 `Blob` 值，以及带精度和小数位的 DECIMAL 列、不带长度的 BLOB 列。
- 可以建表、写入、按数值或字节序比较、查询读回、关闭再打开后读回。
- 超页 BLOB 走已经存在的行级溢出链，不新增页类型。
- 文件格式版本保持 1。目录行仍是 G1、G2、G3。旧库照现在的方式打开、查询和写入。

## 已锁定口径

批准来源是用户原话：「那就开始规划和实施这个ms，要求遵守各种规范，尤其是多批次串行测试规范，本次最多进行一次全量」。本次目标派发把这句话当作 Gate 1 与 Gate 2 的批准，不是跳过。

下面几项按全局最优解选定。被替代的方案在 design.md。

1. 两种类型放在同一个变更。它们的验收都是声明、写入、比较、读回和旧库打开。
2. 不提高 `FORMAT_VERSION`，不加目录行字段。新类型是现有类型字节里的新标签，和 Date、Timestamp 一样。
3. DECIMAL 用 `i128` 系数和 `u8` 小数位。精度 1 到 38。不引入新的数值依赖。比较按十进制位，不做会溢出的系数对齐乘法。
4. 未写精度的 `DECIMAL` / `NUMERIC` 拒绝。`DECIMAL(p)` 的小数位是 0。`BLOB`、`BYTEA`、`BINARY`、`VARBINARY` 不接受长度参数。
5. 写入只接受精确表示。需要四舍五入的小数拒绝，不改已有行。`Float` 不转成 DECIMAL。
6. 比较严格同类型，和 Date 一样。跨类型是 `Type mismatch`。两种类型都不能做索引键，`to_key` 返回 `None`。
7. 不做 DECIMAL 算术。`d + 1` 是类型错误。
8. 十六进制字面量 `X'6869'` 是 BLOB。空 BLOB 用参数 `Value::Blob` 的空字节写入。

## 场景草图

失败边界：旧库打不开或已有行变了；写入把近似值收成别的数；比较把 DECIMAL 当成 Float；为了新标签去改 G1/G2/G3 或格式版本。

| 场景 | 前置 | 动作 | 可观察结果 |
|---|---|---|---|
| 声明 | 空库 | `CREATE TABLE` 含 `DECIMAL(10,2)` 和 `BLOB` | schema 文本含这两种类型；未写精度的 DECIMAL 拒绝且不建表 |
| 精确写入 | 已建表 | 写入 `1.50`、整数 `12`、字符串 `'1.50'`、`X'6869'` | 读回分别是 1.50、12.00、1.50 和字节 68 69 |
| 拒绝近似 | 列是 `DECIMAL(10,2)` | 写入 `1.234` | 类型错误，行数不变 |
| 比较 | 列中有 1.50 和 2.00 | `WHERE d > 1.5` 与 `ORDER BY d` | 按数值序；`d = '1.50'` 报 Type mismatch |
| 相等尺度 | 值 1.50 与字面量 1.5 | 等值比较 | 成立 |
| 主键读回 | DECIMAL 列作主键 | 按该值等值查询 | 行可达，不依赖新的索引键编码 |
| 关闭再开 | 已写入上述行 | 关闭后 `Database::open` 再查询 | 值相同 |
| 超页 | 参数写入 5000 字节的 BLOB | 查询并再次打开 | 字节相同；使用既有溢出链 |
| 旧库 | `tests/fixtures/legacy/legacy_g1.db` | 打开、查询 `users`、插入一行整数后再查询 | 原有 alice 行仍在；新行可读；打开本身成功 |
| dump 往返 | 含两种列和值的库 | dump 到空库再 restore，再 dump | 两代 dump 文本相同 |
| 不做算术 | 已有 DECIMAL 列 | `SELECT d + 1` | 类型错误，行不变 |

## Capabilities

### New Capabilities

- `decimal-blob-types`：两种类型的声明、存储、写入、比较、读回，以及旧库打开结果。

### Modified Capabilities

无。`legacy-format-compat` 的代际定义不变。`database-file-format-header` 的版本契约不变。`datetime-type-system` 不变。

## Impact

- 值与两个 `ColumnType`、tuple 标签、catalog 列类型标签、计划期类型映射、写入门、比较、CAST、JSON、schema/dump/import、PG 类型 OID 的穷尽匹配。
- 不改结果游标，不改查询预算里既有类型的计费口径。BLOB 驻留按字节长度计，DECIMAL 按固定 16 字节计。
- 约束：M19。本里程碑唯一一次全量分批在 Iteration 001。M21 的词表增加变体，不新增环。
- 关联：I069。与 I065 的变更无依赖。

## 回滚

去掉两个变体和标签。已写入新标签的库，旧构建会在未知标签上报现有的解码错误；回滚不迁移这些文件。未使用新类型的库字节不变。

## Gate 1

Gate 1：PASS。需求、场景和范围由上述用户原话批准，不是跳过。本变更只覆盖 MS50 里两种值类型。页格式代际不改，因此兼容声明写在本变更，不进入流式变更。
