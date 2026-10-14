# wal-recovery-replay-integrity Specification

## Purpose

WAL 崩溃恢复重放的完整性与收敛语义：redo 按记录位置寻址且幂等（logical_id 强制落位、页链与内存 tail 重建）、Update/Delete 重放复现运行期全部副作用（版本链重建、无键行回退推导 old_row_id、墓碑 + 索引清理）、恢复收尾把重放推进的 `data_page_tail` 写回 catalog 使恢复 → close → 重开两态一致、写路径对存量陈旧 tail 自愈防链断、redo 目标表 catalog 缺失时定向指因（点名表名与「上次关闭未经 checkpoint」可能原因，打开拒绝不变）；K05 显式报错语义贯穿。来源：MS10-T02 落地；MS10-T05（无键行回退）、MS20-T02（logical_id 强制落位）、change `2026-10-03-recovery-tail-persistence`（tail 写回 + 自愈）、change `2026-10-03-practical-core-gaps`（恢复错误面指因）历次修订。

## Requirements

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

### Requirement: 重放保持 DML 语义

Update 与 Delete 的重放 SHALL 复现运行期全部副作用：Update 新版本 SHALL 写入记录携带的位置并重建 `next_version → old_row_id` 版本链。old_row_id SHALL 与运行期同源推导：`old_tuple` 键位值可键控（Int）时由 PK 推导（重放形态下经磁盘版本多映射，沿用 max rid < 记录 row_id + old_tuple 逐字节校验语义）；键位值不可键控（NULL / 非 Int）时 SHALL 经无键行回退推导——恢复预扫描与重放按 tuple 原始字节为无键行建立候选集并沿重放追加分派，old_row_id 从该候选集以同一 max-rid + 逐字节校验语义派生，SHALL NOT 因键位值不可键控而 RedoFailed。Delete SHALL 同时重放数据页墓碑（`mark_deleted`）与索引清理。恢复后查询语义（更新可见新值、已删行不可见、行数为已提交终态）SHALL 与原运行一致。

#### Scenario: Update/Delete 混合负载恢复语义正确

- **GIVEN** 含 checkpoint 中位点与驱逐规模的 INSERT/UPDATE/DELETE 混合负载 WAL，崩溃（drop 不 close）后重开
- **WHEN** 查询被更新行、被删除行与总行数
- **THEN** 被更新行返回新值、被删除行不可见、总行数精确等于已提交终态（此前：追加式重放产生重复新版本且丢失墓碑与版本链）

#### Scenario: 无键行 Update 崩溃恢复语义正确

- **GIVEN** 表含键位（PK 列，未声明时为第一列）值为 NULL 或非 Int 的行，行经 INSERT 与 UPDATE 后进程崩溃（drop 不 close）
- **WHEN** `Database::open` 执行崩溃恢复
- **THEN** 恢复成功（无 RedoFailed），被更新无键行返回新值，总行数精确等于已提交终态，可键控行恢复语义不受影响

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

### Requirement: 恢复重放目标表缺失的错误面诚实

恢复重放（`redo_record` 的 Insert/Update/Delete 三臂）遇「WAL 已提交记录引用的表在 catalog 中不存在」时，错误信息 SHALL 点名该表名、陈述「WAL 记录引用的表在 catalog 中缺失」的事实，并陈述最可能原因——数据库上次关闭未经 checkpoint（如 lib `Database` 句柄未调用 `close()` 即 drop，未持久化的 DDL 随 catalog 脏页丢失）。打开 SHALL 维持失败（既有 K05 显式报错语义不变）：恢复 SHALL NOT 跳过、静默丢弃或自动清理不可重放的已提交记录。

其他 redo 失败类别（页 I/O、索引操作、PK 派生失败等）的既有错误文案 SHALL 零变化；打开失败的退出路径与错误传播方式 SHALL 零变化（本 Requirement 只改错误面指因，不改恢复行为与打开结果）。

#### Scenario: 句柄 drop 不 close 后重开的错误指因

- **GIVEN** lib 打开数据库，`CREATE TABLE` + 提交 `INSERT` 后句柄直接 drop（未 `close()`）——catalog 脏页丢失、WAL 留有已提交记录
- **WHEN** 重新 open
- **THEN** open 失败，错误信息含表名、catalog 缺失事实与「上次关闭未经 checkpoint（如句柄未 close 即 drop）」的可能原因（失败边界——修复前：`WAL redo failed: table 't' lookup failed during redo: table not found: t`，将 DDL 未持久化误诊为表缺失）

#### Scenario: 正常关闭不触发定向错误面

- **GIVEN** 同上序列但句柄经 `close()` 正常落盘
- **WHEN** 重新 open 并查询该表
- **THEN** open 成功、行可见（定向错误面只覆盖无 checkpoint 关闭形态，正常恢复路径不误伤）

### Requirement: 恢复会话 WAL 收敛（含零 redo 形态）

读写打开完成恢复后，若打开时 WAL 含任一非 Checkpoint 记录（无论 redo 是否实际发生），该会话的 `close()` 与显式 `checkpoint()` SHALL 执行完整 checkpoint 并截断 WAL——「打开时 WAL 曾有事务记录」SHALL 视同未决工作参与零工作判定。收敛后 `rtsql check` 与 `--readonly` 打开 SHALL 正常（文档承诺「open read-write once to recover, then check」对纯读恢复会话成立）；`rtsql checkpoint` 在该状态 SHALL 报告实际执行（`wal truncated; checkpoint site advanced`）。WAL 从未含事务记录的形态（纯读会话、干净库）SHALL 保持 MS18 零工作跳过语义（位点与 WAL 字节不变）。已提交数据崩溃恢复（redo > 0）的既有收敛路径 SHALL 零变化；恢复重放的数据正确性语义（未提交丢弃、已提交完好）SHALL 零变化。

#### Scenario: 未提交事务崩溃后纯读会话收敛

- **GIVEN** 已提交数据完好的库 + 显式事务写入中途 SIGKILL（WAL 含未提交记录、数据页无该行）
- **WHEN** 纯 SELECT 的读写打开（触发恢复，数据正确）→ close → `rtsql check`
- **THEN** check exit 0 单行干净（失败边界——修复前：永久 exit 1 "pending WAL"，与 checkpoint 命令 "no pending work" 构成死循环）

#### Scenario: checkpoint 命令报告实际执行

- **GIVEN** 同上恢复后的库（会话未再写入）
- **WHEN** `rtsql checkpoint <db>`
- **THEN** 输出 `wal truncated; checkpoint site advanced`（失败边界——修复前：`no pending work; wal untouched`）

#### Scenario: 零工作跳过保持

- **GIVEN** WAL 从未含事务记录的干净库
- **WHEN** 纯读会话 close；显式 checkpoint
- **THEN** 位点文件与 WAL 字节不变（MS18 语义保持，checkpoint_skip_test S1/S2/S6 语义）

#### Scenario: marker-only WAL 收敛

- **GIVEN** WAL 仅含空事务标记记录（BEGIN/COMMIT 无数据写入）的崩溃残留
- **WHEN** 纯读读写打开 → check
- **THEN** 收敛 exit 0（修复前：同样楔死于 pending WAL 拒绝）

#### Scenario: redo 收敛路径零变化

- **GIVEN** 含已提交写入的崩溃库
- **WHEN** 读写打开（恢复重放）→ close → check
- **THEN** 行为与修复前一致（checkpoint_skip_test S5、check_command_test 既有用例零回归）

### Requirement: DDL 语句边界 catalog 有序落盘

DDL 语句（CREATE TABLE、CREATE INDEX、CREATE VIEW、ALTER TABLE ADD COLUMN，含 import 自动建表与 CTAS 建表段）执行成功返回前，其 catalog 效果 SHALL 有序落盘：被引用页（数据页头、索引根页、`__columns` 列行页）先落盘，`__tables` 行（或其更新副本所在页）最后落盘（commit point）。此后未 checkpoint 崩溃（SIGKILL），读写打开 SHALL 恢复可达——WAL 重放能找到表定义，已提交数据完好，`check`/`schema`/`checkpoint` 正常（`check` 的「open read-write once to recover」指引在该形态下可达）。恢复重放的硬失败语义（K05：catalog 缺表即拒绝打开、不跳过记录）SHALL 保持不变；建表前/建表冲刷中段崩溃的既有语义（表定义未持久 = DDL 视同未发生）SHALL 不变。DROP 类 DDL（DROP TABLE/INDEX/VIEW、TRUNCATE、ALTER DROP COLUMN）的崩溃窗口 SHALL 维持既有 checkpoint-based 边界（不因本要求改变）。

#### Scenario: import 自动建表后未 checkpoint 崩溃可恢复

- **GIVEN** `rtsql new` + `import --csv`（自动建表 + 已提交行、WAL 未达自动水位、无 checkpoint）+ SIGKILL
- **WHEN** 读写打开（或 `check`/`schema`/`checkpoint`）
- **THEN** 打开成功、表定义与已提交数据完好、`check` exit 0 干净（失败边界——修复前：全命令 exit 1 整库砖化，`check` 提示的恢复路径即失败操作，死锁闭环）

#### Scenario: DDL 各形态建后即崩恢复

- **GIVEN** 分别执行 CREATE TABLE（含约束/DEFAULT）、CREATE INDEX（含回填）、CREATE VIEW、ALTER ADD COLUMN、CTAS、import 自动建表后立即 SIGKILL（无 checkpoint/close）
- **WHEN** 读写重开
- **THEN** 表/索引/视图/列定义全部生效，数据可达，`check` 干净

#### Scenario: 小数据量不砖化（池驱逐不可依赖）

- **GIVEN** 小数据量（catalog 页不被缓冲池驱逐刷盘）+ WAL 已有该表已提交行 + 未 checkpoint + SIGKILL
- **WHEN** 任意开库命令
- **THEN** 恢复可达（失败边界——修复前：依赖池驱逐偶然刷盘才不砖化；确定性砖化窗口）

#### Scenario: 既有恢复与零工作语义零回归

- **GIVEN** 表已存在的库同型崩溃；干净库（WAL 从无事务记录）纯读会话
- **WHEN** 恢复打开 / close / 显式 checkpoint
- **THEN** 既有恢复行为与 MS18 零工作跳过语义逐字节不变
