# index-read-bounded-read Delta

> **标题对齐说明（ISS34）**：OpenSpec 的 `MODIFIED` 块在归档时会被校验「当前规格中的每个 Scenario 标题必须逐字出现在该块内」，否则归档拒绝该增量。因此**既有 Scenario 的标题改名无法由增量表达**（Requirement 标题虽可用 `RENAMED` 表达，但改名后仍会与既有 Scenario 标题并存）。本增量只承载**行为变化**（ISS26：条目长度纳入页级判据），并逐字保留既有 Requirement 与 Scenario 标题；**ISS34 的标题措辞对齐**（Requirement 标题 + 4 个 Scenario 标题）以**文档交接项**交 `openspec-docs-maintainer` 在合并时一并完成（见 change `tasks.md` 的 T13）。

## MODIFIED Requirements

### Requirement: 损坏的索引页在读 / 写路径上给出明确的错误

索引树访问器族与索引树的下降 / 条目提取路径 SHALL 经由**有边界检查**的读取路径取得槽数据：越界的槽偏移或长度 SHALL NOT 造成越界切片、SHALL NOT 导致进程 abort。**页级判据 SHALL 覆盖槽边界与条目长度两类内容**：索引树**叶页**的槽负载长度短于条目格式（`MAX_KEY_LEN + RowId::SIZE`）时 SHALL 判为未通过页级判据。读取到**未通过页级判据**的索引树页时，系统 SHALL 返回**明确的错误**（错误须点名损坏的页与该索引树），SHALL NOT 把该页当作「没有这个子树」或「没有这个条目」继续（跳过 ⇒ 点查返回「查不到该行」⇒ 静默错读；任选一个子结点继续 ⇒ 读到别的行），SHALL NOT 把该错误降级为告警后继续下降。未损坏路径的读 / 写行为 SHALL 与修复前一致。

#### Scenario: 损坏的索引页上的查询不 panic、不静默错读

- **GIVEN** 一个数据库文件，其某张表的**索引树页**槽目录项 offset 字段越界（页内容已损坏；该页是这条查询实际扫描的索引树页）
- **WHEN** 执行点查（`WHERE id = …`）与全表扫描
- **THEN** 语句 SHALL NOT panic，SHALL NOT 导致进程 abort
- **AND** 语句 SHALL 返回**明确的错误**（点名损坏的页与该索引树）
- **AND** 语句 SHALL NOT 返回「查不到该行」而全表扫描命中（那会是静默错读）

#### Scenario: 损坏的索引页上的插入不 panic

- **GIVEN** 同上形态（损坏点落在该语句实际扫描的索引树页）
- **WHEN** 执行 `INSERT`
- **THEN** 语句 SHALL NOT panic，SHALL NOT 导致进程 abort，且 SHALL 返回明确的错误（点名损坏的页与该索引树）
- **AND** 该失败 SHALL NOT 留下半写状态（损坏在读取阶段被发现）

#### Scenario: 损坏的索引页上的删除不 panic

- **GIVEN** 同上形态
- **WHEN** 执行 `DELETE`
- **THEN** 语句 SHALL NOT panic，SHALL NOT 导致进程 abort，且 SHALL 返回明确的错误（点名损坏的页与该索引树）

#### Scenario: 损坏的索引页上的更新不 panic

- **GIVEN** 同上形态
- **WHEN** 执行 `UPDATE`
- **THEN** 语句 SHALL NOT panic，SHALL NOT 导致进程 abort，且 SHALL 返回明确的错误（点名损坏的页与该索引树）

#### Scenario: 叶页条目长度过短的索引树页给出明确的错误

- **GIVEN** 一个数据库文件，其某张表的**索引树叶页**某个槽的负载长度短于条目格式（`MAX_KEY_LEN + RowId::SIZE`）；槽偏移仍落在页内（页内容已损坏，但未越界）
- **WHEN** 执行读该索引树的查询（点查或整棵索引树扫描）
- **THEN** 语句 SHALL 返回**明确的错误**（点名损坏的页与该索引树），SHALL NOT panic、SHALL NOT 导致进程 abort
- **AND** 语句 SHALL NOT 把该条目当作「没有这个条目」而静默跳过（那会让点查返回「查不到该行」⇒ 静默错读）

#### Scenario: 未损坏路径的读 / 写行为零变化

- **GIVEN** 一个内容完好的数据库
- **WHEN** 执行 `SELECT` / `INSERT` / `DELETE` / `UPDATE`
- **THEN** 点查命中数、行集合与写入结果 SHALL 与既有行为一致（键序、子结点选择、命中数不变）
- **AND** 既有索引相关用例 SHALL 保持通过
