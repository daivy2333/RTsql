## ADDED Requirements

### Requirement: 恢复重放收敛 data_page_tail 持久一致

崩溃恢复存在重放（redo_count > 0）时，恢复收尾 SHALL 把每个发生重放写入的表的 `data_page_tail` 经既有 catalog 更新通道（`update_table_tail`）持久化，使「恢复 → close → 重开」后 catalog 持久 tail 与数据链 terminal 一致。tail 写回失败 SHALL 显式报错（K05 语义不变）。重放重复执行（crash-during-recovery 后重开）SHALL 幂等收敛。无重放（redo_count == 0 或无 WAL）的打开路径 SHALL 零变化（不触碰 tail 持久化）。

#### Scenario: 恢复后 tail 两态一致

- **GIVEN** checkpoint 后写入含跨页增长（页满自动分配新页）的已提交事务，进程崩溃（drop 不 close，WAL 未 checkpoint）
- **WHEN** `Database::open` 执行崩溃恢复、close 后再次重开
- **THEN** catalog 持久 tail 与数据链 terminal 一致；`rtsql check` exit 0 且无 info 级 finding；`rtsql dump` 无链完整性告警

#### Scenario: 重复恢复幂等

- **GIVEN** 恢复完成后未产生新写入、未 checkpoint，进程再次崩溃后重开
- **WHEN** 恢复再次执行
- **THEN** tail 写回幂等，行集与首次恢复完全一致

#### Scenario: 无重放零变化

- **GIVEN** 干净库（无 WAL 文件或无可重放记录）
- **WHEN** 打开并执行读写
- **THEN** 打开路径不触碰 tail 持久化，行为与 IO 面不变

### Requirement: 写路径陈旧 tail 自愈

运行期写路径以内存 `data_page_tail` 页为写入目标前 SHALL 检测其陈旧形态（该页 `next` 指针非零，即持久 tail 落后于链 terminal——本修复前已恢复的存量库可达此形态）。检出时 SHALL 沿链前进至 terminal（自愈）、同步持久化修正后的 tail，再执行写入；SHALL NOT 覆写链中既有 `next` 指针导致已恢复页行不可达。健康库（tail 页 `next == 0`）SHALL 零额外常态 IO。

#### Scenario: 陈旧 tail 页满时已恢复页行不丢失

- **GIVEN** 存量库持久 tail 落后链 terminal（如本修复前恢复产生的形态）
- **WHEN** 持续写入直至 tail 页满、触发新页分配
- **THEN** 新页挂链于 terminal，陈旧 tail 与 terminal 之间的已恢复页行全部可达，tail 字段同步修正并持久化；重开后行集一致

#### Scenario: 健康库零额外常态 IO

- **GIVEN** 健康库（持久 tail 与链 terminal 一致）
- **WHEN** 常规写入
- **THEN** 写路径无因自愈引入的额外页访问（tail 页 `next == 0` 即写，不触发行走查）

### Requirement: 观测面与既有恢复语义零回归

`rtsql check` 对「持久 tail 落后 terminal」的 info 分级 SHALL 保留（存量库指示器）；新建恢复库 SHALL 干净通过（无 info）。`rtsql dump` 链完整性预检语义 SHALL 不变。既有恢复套件行为——重放位置寻址与幂等、DML 语义重放、K05 显式报错、索引重建、溢出行恢复——SHALL 零回归。

#### Scenario: 存量库观测面保持

- **GIVEN** 存量陈旧 tail 库（WAL 已清、不再触发恢复重放）
- **WHEN** 执行 `rtsql check` 与 `rtsql dump`
- **THEN** check 仍以 info 报告 tail 落后且 exit 0（分级不移除）；dump 保持既有链预检告警语义；首次写入触发自愈后两者转干净

#### Scenario: 既有恢复面零回归

- **GIVEN** 既有恢复测试面（驱逐规模大 WAL、无键行 Update、中止后重插收敛、超页溢出行恢复）
- **WHEN** 运行恢复套件与定向回归
- **THEN** 全部通过，K05 错误面与索引/释放行为不变
