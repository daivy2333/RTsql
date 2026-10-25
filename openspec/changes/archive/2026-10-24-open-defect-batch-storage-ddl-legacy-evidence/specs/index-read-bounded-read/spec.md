# index-read-bounded-read Specification

## Purpose

索引树（`src/storage/btree/node.rs` 的访问器族）在**读 / 写热路径**上对页内容的读取契约：读取 SHALL 经由有边界检查的路径，**未通过页级判据**的索引树页 SHALL NOT 导致进程 abort，且 SHALL 给出**明确的错误**（点名损坏的页与该索引树）而不是被静默跳过成「查不到该行」。本域承载 B-4 同根因层的读 / 写路径面（释放路径的同根因面已由 `release-path-bounded-read` 域承载，其处置方向与本域相反：释放路径为尽力而为的「告警 + 跳过」，读 / 写路径为「明确错误」）。

## ADDED Requirements

### Requirement: 损坏的索引页在读 / 写路径上给出明确的错误

索引树访问器族与索引树的下降 / 条目提取路径 SHALL 经由**有边界检查**的读取路径取得槽数据：越界的槽偏移或长度 SHALL NOT 造成越界切片、SHALL NOT 导致进程 abort。读取到**未通过页级判据**的索引树页时，系统 SHALL 返回**明确的错误**（错误须点名损坏的页与该索引树），SHALL NOT 把该页当作「没有这个子树」或「没有这个条目」继续（跳过 ⇒ 点查返回「查不到该行」⇒ 静默错读；任选一个子结点继续 ⇒ 读到别的行），SHALL NOT 把该错误降级为告警后继续下降。未损坏路径的读 / 写行为 SHALL 与修复前一致。

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

#### Scenario: 未损坏路径的读 / 写行为零变化

- **GIVEN** 一个内容完好的数据库
- **WHEN** 执行 `SELECT` / `INSERT` / `DELETE` / `UPDATE`
- **THEN** 点查命中数、行集合与写入结果 SHALL 与既有行为一致（键序、子结点选择、命中数不变）
- **AND** 既有索引相关用例 SHALL 保持通过
