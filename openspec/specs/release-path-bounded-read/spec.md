# release-path-bounded-read Specification

## Purpose

释放路径（`DROP TABLE` / `TRUNCATE` / `DROP INDEX`）对页内容的读取契约：读取 SHALL 经由有边界检查的路径，损坏页 SHALL NOT 导致进程 abort。本域承载 B-4（释放路径用未检查的槽数据读取器扫描溢出定位符 ⇒ 越界切片 panic）。

## Requirements

### Requirement: 释放路径读取页内容时有边界检查

释放路径在扫描数据页的槽位以收集溢出链页时 SHALL 经由**有边界检查**的读取路径取得槽数据：越界的槽偏移或长度 SHALL NOT 造成越界切片。

#### Scenario: 损坏页上的删表不 panic

- **GIVEN** 一个数据库文件，其某张待删表的数据页槽位偏移越界（页内容已损坏）
- **WHEN** 执行 `DROP TABLE`
- **THEN** 语句 SHALL NOT panic，SHALL NOT 导致进程 abort
- **AND** 语句 SHALL 按释放路径既有的尽力而为语义结束（告警并跳过不可读的部分，返回成功或明确的错误）

#### Scenario: 损坏页上的截断不 panic

- **GIVEN** 一个数据库文件，其某张待截断表的数据页槽位偏移越界（页内容已损坏）
- **WHEN** 执行 `TRUNCATE`
- **THEN** 语句 SHALL NOT panic，SHALL NOT 导致进程 abort
- **AND** 语句 SHALL 按释放路径既有的尽力而为语义结束

#### Scenario: 损坏页上的删索引不 panic

- **GIVEN** 一个数据库文件，其某个待删二级索引的**索引树页**槽位偏移越界（页内容已损坏；`DROP INDEX` 的释放路径只扫描该索引自己的索引树页，不扫描该表的数据页）
- **WHEN** 执行 `DROP INDEX`
- **THEN** 语句 SHALL NOT panic，SHALL NOT 导致进程 abort
- **AND** 语句 SHALL 按释放路径既有的尽力而为语义结束（告警并跳过不可读的部分，返回成功或明确的错误）

#### Scenario: 未损坏的释放路径行为不变

- **GIVEN** 一个内容完好的数据库
- **WHEN** 执行 `DROP TABLE` / `TRUNCATE` / `DROP INDEX`
- **THEN** 释放的页集合 SHALL 与既有行为一致（未损坏页上的释放不遗漏、不重复）
- **AND** 既有释放断言 SHALL 保持通过
