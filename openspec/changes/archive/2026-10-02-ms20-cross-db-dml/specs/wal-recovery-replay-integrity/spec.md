## MODIFIED Requirements

### Requirement: 重放位置寻址且幂等

崩溃恢复的 redo SHALL 按记录携带的原始位置（`row_id`，含页与 logical_id）重放数据变更，而非从 tail 盲目追加：目标槽位已存在时 SHALL 跳过；不存在时 SHALL 以记录携带的 logical_id 精确落位——页内分配计数器不足以自然产出该 id 时 SHALL 按记录 id 落位并推进计数器（页间切换按记录序列重建页链 next 指针与内存 tail）。运行期「语句部分写入回滚 → 后续写入 → 崩溃」序列（页内逻辑 id 已被中止事务消耗）SHALL NOT 因重放分配序列与运行期失配而 RedoFailed 拒绝打开；恢复结果 SHALL 与原运行提交状态一致（行集、PK/唯一/二级索引、版本链）。在任何驱逐/刷盘状态下，重放结果 SHALL 与原运行提交状态一致；重放重复执行（crash-during-recovery 后重开）SHALL 收敛到同一状态。重放无法物理落位（页满、IO 错误）时 SHALL 仍显式报错（K05 语义不变）。

#### Scenario: 驱逐规模大 WAL 恢复数据精确

- **GIVEN** 数据规模超过 BufferPool 容量（>100 页）且 WAL 未 checkpoint（如 1 万行、50 行/显式事务、drop 不 close）
- **WHEN** `Database::open` 执行崩溃恢复
- **THEN** `COUNT(*)` 精确等于已提交行数（无重复、无丢失；此前：行数虚增、头部区域重复、原链尾部丢失且损坏经 checkpoint 持久化）

#### Scenario: 恢复后索引与数据一致

- **GIVEN** 同上场景恢复完成
- **WHEN** 对已存在 PK 重复 INSERT、对新 PK INSERT
- **THEN** 已存在 PK 被索引拒绝（DuplicateKey）、新 PK 成功——PK 索引内容与数据页一致（此前：重放行不在索引中）

#### Scenario: 恢复重跑幂等

- **GIVEN** 恢复完成后无新写入、未 checkpoint，进程再次崩溃后重开
- **WHEN** 恢复再次执行
- **THEN** 行数与内容与首次恢复完全一致

#### Scenario: 中止后重插的崩溃恢复收敛

- **GIVEN** 表含 NOT NULL/UNIQUE 约束与二级索引，checkpoint 后依次提交写入（含 UPSERT 与主键 rekey），随后多行语句前行已写、后行约束失败回滚（页内逻辑 id 已被中止事务消耗），重插同键行并提交，崩溃（WAL 未 checkpoint、drop 不 close）
- **WHEN** `Database::open` 执行崩溃恢复
- **THEN** 恢复成功（无 RedoFailed）；行集、PK 点查、唯一占用与二级索引查询与运行期提交终态一致；无 abort 与 abort 后不重插两个对照序列行为不变
