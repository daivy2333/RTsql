# release-path-bounded-read Delta

## MODIFIED Requirements

### Requirement: 释放路径读取页内容时有边界检查

释放路径在扫描数据页的槽位以收集溢出链页时 SHALL 经由**有边界检查**的读取路径取得槽数据：越界的槽偏移或长度 SHALL NOT 造成越界切片。释放路径对**索引树页**的**页级判据 SHALL 覆盖槽边界与条目长度两类内容**：索引树叶页的槽负载长度短于条目格式（`MAX_KEY_LEN + RowId::SIZE`）时 SHALL 判为未通过页级判据，并按本域的尽力而为语义（**计数 + 报出 + 跳过**）处理——该页被计入损坏页计数并被报出（报出内容 SHALL 点名该页），其不可读的部分 SHALL 被跳过，语句 SHALL NOT 因此 abort。

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

#### Scenario: 叶页条目长度过短的索引树页被计数、报出并跳过

- **GIVEN** 一个数据库文件，其某个待删索引的**索引树叶页**某个槽的负载长度短于条目格式（`MAX_KEY_LEN + RowId::SIZE`）；槽偏移仍落在页内
- **WHEN** 执行 `DROP INDEX`（或 `DROP TABLE` / `TRUNCATE` 释放该表的主键索引树）
- **THEN** 语句 SHALL NOT panic，SHALL NOT 导致进程 abort
- **AND** 该页 SHALL 被计入损坏页计数并被**报出**（报出内容 SHALL 点名该页）
- **AND** 该页的不可读部分 SHALL 被**跳过**（其子树不被释放）——「检测到损坏」与「忽略损坏」是两回事

#### Scenario: 未损坏的释放路径行为不变

- **GIVEN** 一个内容完好的数据库
- **WHEN** 执行 `DROP TABLE` / `TRUNCATE` / `DROP INDEX`
- **THEN** 释放的页集合 SHALL 与既有行为一致（未损坏页上的释放不遗漏、不重复）
- **AND** 既有释放断言 SHALL 保持通过
