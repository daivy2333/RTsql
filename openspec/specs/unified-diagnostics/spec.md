# unified-diagnostics Specification

## Purpose
规定会话内诊断的配置、阶段与等待口径、同一次语句的样本归属和输出上限。诊断只观察。SQL 结果、错误文本、SQLSTATE、取消后的行可见性，以及既有 stderr 告警，都不因打开诊断而改变。

## Requirements

### Requirement: 详细程度、观察类别和采集内容分开配置

会话诊断配置 SHALL 分开设置输出详细程度、观察类别和采集内容。采集内容 SHALL 至少能分别开关事件、性能统计和详细追踪。性能统计 SHALL 可以在事件关闭且详细追踪关闭时单独开启。

输出详细程度 SHALL NOT 决定是否采集。详细程度为关闭时，已经采集的摘要 SHALL 仍能在进程内读回，并且该次执行 SHALL NOT 因此写出新的诊断 stderr。

未修改过的会话配置 SHALL 三项采集都关闭，输出详细程度为 Summary。该默认下没有摘要可渲染，SHALL NOT 产生新的诊断 stderr。

严重级别 SHALL 只使用 Error、Warn、Info、Debug、Trace。阶段成功结束 SHALL 记为 Info，失败 SHALL 记为 Error，取消 SHALL 记为 Warn。

观察类别 SHALL 能限定为子集。性能统计打开但观察类别不含语句阶段时，摘要 SHALL NOT 出现解析、规划或执行阶段。

#### Scenario: 只开性能统计

- **GIVEN** 会话打开性能统计，关闭事件和详细追踪，观察类别包含语句阶段
- **WHEN** 执行一条成功的单语句查询
- **THEN** 进程内摘要含有该语句的阶段耗时，且不含逐条追踪样本
- **AND** 查询结果与诊断关闭时相同

#### Scenario: 详细程度关闭仍可在进程内读回

- **GIVEN** 性能统计打开，输出详细程度为关闭
- **WHEN** 执行一条成功语句
- **THEN** 进程内摘要仍有阶段耗时
- **AND** 该次执行不写出新的诊断 stderr

#### Scenario: 观察类别排除语句阶段

- **GIVEN** 性能统计打开，观察类别不包含语句阶段
- **WHEN** 执行一条成功查询
- **THEN** 摘要没有解析、规划、执行阶段
- **AND** 查询结果与诊断关闭时相同

### Requirement: 语句阶段与父子耗时

性能统计打开且观察语句阶段时，一次未命中计划缓存的语句 SHALL 分别记录解析、规划和执行。该语句实际提交时，提交 SHALL 记为执行阶段的子阶段。不写数据的查询 SHALL NOT 出现提交子阶段。计划缓存命中时，该次执行 SHALL NOT 记录解析和规划。

摘要里的语句总耗时 SHALL 等于该语句根阶段的包含耗时，SHALL NOT 把根阶段与其子阶段相加后当作总耗时。子阶段耗时 SHALL 单独列出。阶段独占耗时 SHALL 等于该阶段包含耗时减去其直接子阶段包含耗时之和；差为负时 SHALL 饱和为 0。该计算 SHALL NOT 假定提交是执行阶段的唯一直接子阶段。

#### Scenario: 固定包含耗时不相加

- **GIVEN** 语句根阶段包含耗时为 100，执行阶段包含耗时为 70，其提交子阶段包含耗时为 40
- **WHEN** 生成这条语句的摘要
- **THEN** 语句总耗时为 100
- **AND** 执行阶段的独占耗时为 30

#### Scenario: 插入的提交是执行的子阶段

- **GIVEN** 性能统计打开
- **WHEN** 成功执行一条会提交的插入
- **THEN** 摘要同时有执行阶段和提交子阶段
- **AND** 提交子阶段的包含耗时不大于执行阶段的包含耗时
- **AND** 该行在随后的查询中可见

#### Scenario: 查询没有提交子阶段

- **GIVEN** 性能统计打开
- **WHEN** 执行一条不写数据的成功查询
- **THEN** 摘要有执行阶段
- **AND** 没有提交子阶段

#### Scenario: 缓存命中不记录未发生的解析

- **GIVEN** 同一条可缓存语句已经成功执行过，性能统计打开
- **WHEN** 再执行同一条语句
- **THEN** 摘要不为这次执行记录解析和规划耗时
- **AND** 执行阶段仍有记录

### Requirement: 等待类别

详细追踪打开且观察相应类别时，提交等待 SHALL 分别给出 Lock、Queue、Io 和 Schedule 的耗时。Schedule SHALL 允许为 0。执行阶段中从文件读取数据页 SHALL 记为 Io。缓冲池已经持有的页 SHALL NOT 仅因被访问就记为 Io。没有被这些等待包住的阶段时间 SHALL 记为 Compute。解析阶段 SHALL 有 Compute，且 SHALL NOT 有 Io。

同一条语句摘要内，内层等待的耗时 SHALL NOT 再加进它的外层等待。同一次刷盘释放的每个提交，其摘要 SHALL 包含该次刷盘的整段 Io 耗时。这些按语句列出的 Io 耗时 SHALL NOT 被规定为可以相加后当作进程级 I/O 总量。

只开性能统计且详细追踪关闭时，等待 SHALL 以类别汇总数出现，SHALL NOT 以逐条追踪样本出现。

#### Scenario: 提交等待带出锁、队列和 I/O

- **GIVEN** 详细追踪打开
- **WHEN** 成功提交一条插入
- **THEN** 该语句摘要的提交子阶段给出 Lock、Queue、Io 和 Schedule
- **AND** 插入的行在随后的查询中可见

#### Scenario: 解析阶段记为计算

- **GIVEN** 详细追踪打开
- **WHEN** 成功执行一条未命中缓存的语句
- **THEN** 解析阶段的等待含有 Compute
- **AND** 解析阶段没有 Io

#### Scenario: 文件页读取记为 I/O

- **GIVEN** 详细追踪打开且观察存储类别，文件库的缓冲池尚未持有目标数据页
- **WHEN** 执行一条要读取该页的查询
- **THEN** 该语句摘要含有 Io
- **AND** 查询结果与诊断关闭时相同

#### Scenario: 只开统计时不保留逐条样本

- **GIVEN** 只开性能统计，详细追踪关闭
- **WHEN** 成功提交一条插入
- **THEN** 摘要含有提交子阶段的等待类别汇总数
- **AND** 不含逐条追踪样本

### Requirement: 同一次语句的异步样本

刷盘任务上为某次提交采集的等待样本 SHALL 进入该次语句的摘要，SHALL NOT 进入同时执行的另一条语句的摘要。

摘要 SHALL 携带该语句自身的 SQL 文本。该文本超出保留长度时 SHALL 被截断。公开摘要 SHALL NOT 包含操作标识、运行标识或内容哈希字段。测试或证据的通过条件 SHALL NOT 使用这类字段。

#### Scenario: 并发提交的摘要不互相包含

- **GIVEN** 同一会话上两条 SQL 文本不同的插入并发提交，详细追踪打开
- **WHEN** 两条都返回成功
- **THEN** 每条摘要只含有自己的 SQL 文本
- **AND** 摘要没有操作标识、运行标识或内容哈希字段
- **AND** 两行都可见

### Requirement: 成功、失败和取消都结束记录

语句成功、返回错误，或语句 future 在完成前被放弃时，该语句的诊断记录 SHALL 结束。成功 SHALL 记为成功，返回错误 SHALL 记为失败，future 被放弃 SHALL 记为取消。

返回的错误文本和 SQLSTATE SHALL 与诊断关闭时相同。提交前被放弃的隐式写入，在同一会话的下一条写语句开始前 SHALL NOT 可见。诊断摘要 SHALL NOT 代替该行可见性断言。

#### Scenario: 解析失败留下失败摘要且错误不变

- **GIVEN** 性能统计打开
- **WHEN** 执行一条无法解析的语句
- **THEN** 摘要状态为失败，且含有解析阶段
- **AND** 返回的错误文本和 SQLSTATE 与诊断关闭时相同

#### Scenario: 放弃 future 后摘要为取消且未提交行不可见

- **GIVEN** 性能统计打开
- **WHEN** 一条隐式插入的 future 在提交前被放弃，随后同一会话执行下一条写语句
- **THEN** 被放弃语句的摘要状态为取消
- **AND** 被放弃插入的行不可见

### Requirement: 输出限量与慢输出

每条语句保留的事件和追踪样本 SHALL 有上限。默认上限 SHALL 为 64。超出后 SHALL 停止追加事件和追踪样本，SHALL 保留超出条数，性能统计汇总 SHALL 继续更新。

会话保留的已结束语句摘要 SHALL 有上限。默认 SHALL 最多保留 32 条，超出后丢弃最旧的。

语句执行过程中的诊断投递 SHALL NOT 写 stderr。写到 stderr 的诊断文本 SHALL 发生在该语句的 SQL 结果已经确定之后。慢的 stderr SHALL 可以推迟调用返回，SHALL NOT 改变该 SQL 结果。查询返回的行 SHALL NOT 包含阶段名称或等待类别。

#### Scenario: 超出上限后仍保留统计

- **GIVEN** 单条语句的事件上限为 1，详细追踪打开
- **WHEN** 执行一条会产生多个阶段记录的成功语句
- **THEN** 摘要的超出条数大于 0
- **AND** 阶段耗时汇总仍在
- **AND** SQL 结果与诊断关闭时相同

#### Scenario: 会话只保留最近的摘要

- **GIVEN** 会话摘要上限为 1，性能统计打开
- **WHEN** 连续成功执行两条语句
- **THEN** 进程内只读得到后一条语句的摘要

#### Scenario: SQL 结果不包含诊断文本

- **GIVEN** 输出详细程度为完整，详细追踪打开
- **WHEN** 执行一条成功查询
- **THEN** 返回行与诊断关闭时相同
- **AND** 行内容不包含阶段名称或等待类别

### Requirement: 三种开销被实测且不设数值门槛

SHALL 在同一条已有的单行查询负载上分别测量诊断关闭、只开性能统计、详细追踪三种配置的耗时。三次的 SQL 结果 SHALL 相同。本次 SHALL NOT 把三者耗时差的上限写成通过条件，也 SHALL NOT 把未测得的零开销写成通过条件。

#### Scenario: 三种配置结果相同且都有耗时记录

- **WHEN** 对同一条查询依次使用诊断关闭、只开性能统计和详细追踪
- **THEN** 三次查询结果相同
- **AND** 三次耗时都被记录
- **AND** 通过条件不包含三者差值的上限

### Requirement: 既有计时出口和 stderr 告警保持

`RTSQL_PROFILING` 未设置时，既有三段计时 SHALL 保持关闭。该变量已设置且会话诊断配置保持默认时，stderr SHALL 仍包含 parse、plan、execute 三段耗时，子指标能力 SHALL 保持。

经语句管道执行的语句在该变量已设置时，SHALL 只在执行阶段被触达的返回路径输出这张表。执行阶段被触达指两种情形：计划缓存命中后直接执行，以及解析与规划都成功后进入执行。解析失败、规划失败、多语句拒绝、占位符编号失败、参数个数不匹配、无语句可执行等执行阶段之前的失败路径 SHALL NOT 输出这张表。进入执行阶段后，无论执行成功还是失败，SHALL 输出这张表。

该收敛 SHALL 只改变这张表的输出时机。语句返回的错误文本与 SQLSTATE SHALL 与收敛前相同；诊断摘要、取消语义和取消后的行可见性 SHALL NOT 受影响。

诊断配置为默认时，现有用户可见 stderr 告警 SHALL 保持原文。恢复路径上已有的 `[recovery]` 告警 SHALL 仍出现在产生它的失败条件下。本变更 SHALL NOT 删除或改写这些告警来充当诊断输出。

#### Scenario: 旧开关仍输出三段耗时

- **GIVEN** `RTSQL_PROFILING` 已设置，会话诊断配置保持默认
- **WHEN** 执行一条未命中计划缓存的语句
- **THEN** stderr 含有 parse、plan 和 execute

#### Scenario: 恢复告警原文仍在

- **GIVEN** 诊断配置为默认
- **WHEN** 走到会写出 `[recovery]` 告警的既有恢复失败条件
- **THEN** stderr 仍包含 `[recovery]`

#### Scenario: 失败路径也输出旧的三段计时表

- **GIVEN** `RTSQL_PROFILING` 已设置，会话诊断配置保持默认，该语句已触达执行阶段
- **WHEN** 经语句管道执行一条进入执行阶段后失败的语句
- **THEN** stderr 仍含一张三段计时表
- **AND** 返回的错误文本与 SQLSTATE 不变

#### Scenario: 执行阶段之前的失败路径不输出旧的三段计时表

- **GIVEN** `RTSQL_PROFILING` 已设置，会话诊断配置保持默认，该语句尚未触达执行阶段
- **WHEN** 经语句管道执行一条在执行阶段之前失败的语句
- **THEN** stderr 不含这张三段计时表
- **AND** 返回的错误文本与 SQLSTATE 不变

### Requirement: 诊断不改变事务结果

同一语句在诊断关闭和打开时，成功行、错误文本、SQLSTATE 和已提交可见性 SHALL 相同。诊断 SHALL NOT 提交、回滚或取消调用方没有要求的事务。

#### Scenario: 打开诊断不改变已提交行

- **WHEN** 同一插入在诊断关闭和详细追踪打开时各成功执行一次
- **THEN** 两次都成功
- **AND** 重新打开后两次插入的行都在

### Requirement: 五个旧子指标进入当前语句阶段树

性能统计打开时，`table_metadata_lookup`、`executor_creation`、`executor_execution`、`index_manager_search`、`index_manager_search_all` SHALL 用各调用点现有的一次测量，经 `record_stage` 进入当前语句的阶段树，并出现在该语句摘要中。前三个的观察类别 SHALL 为 Pipeline，后两个 SHALL 为 Index。

这五段 SHALL NOT 通过 `record_pre_measured` 另起语句摘要，也 SHALL NOT 为此再开一层 open 阶段。`RTSQL_PROFILING` 已设置时，同一次测量 SHALL 仍写入旧三段计时表。采集全关且该变量未设置时，这五段 SHALL NOT 取测量。

`table_metadata_lookup` SHALL 在 plan 开着时记入，并作为 plan 的直接子阶段。`executor_creation`、`executor_execution`，以及执行期间发生的索引查找，SHALL 在 execute 开着时记入，并作为 execute 的直接子阶段。索引查找的区间可以落在 `executor_execution` 之内；两者的包含耗时之和 MAY 大于 execute 的包含耗时。

一次不走索引扫描执行器的成功插入，execute 的直接子阶段 SHALL 是 `executor_creation`、`executor_execution` 和 commit。这三者的区间不重叠，因此 execute 的独占耗时加上这三者的包含耗时 SHALL 等于 execute 的包含耗时。

文件页读 SHALL 继续只经 `record_page_read` 计入当时最内层 open 阶段。本需求 SHALL NOT 新增缓冲池计数或行数资源计数器。解析、规划、执行、提交的 open/close 与等待归属 SHALL 保持不变。观察类别不含 Index 时，摘要 SHALL NOT 出现两个索引阶段名。观察类别不含 Pipeline 时，摘要 SHALL NOT 出现前三个阶段名。

既有 stderr 告警 SHALL 保持原文。SQL 结果、错误文本和 SQLSTATE SHALL NOT 因这五段上报而改变。

#### Scenario: 插入带上三个流水线子指标

- **GIVEN** 性能统计打开，且未设置 `RTSQL_PROFILING`
- **WHEN** 成功插入一行，随后查询该行
- **THEN** 该插入的摘要含有 `table_metadata_lookup`、`executor_creation`、`executor_execution` 和 commit
- **AND** execute 的独占耗时加上这三个子阶段的包含耗时等于 execute 的包含耗时
- **AND** 随后的查询看见这一行

#### Scenario: 主键等值记入索引查找

- **GIVEN** 表中已有目标行，性能统计打开
- **WHEN** 执行 `SELECT * FROM t WHERE id = 1`
- **THEN** 摘要含有 `index_manager_search`，且不含 `index_manager_search_all`
- **AND** 返回的行与诊断关闭时相同

#### Scenario: 二级索引等值记入 search_all

- **GIVEN** 表上已有 `CREATE INDEX idx_b ON t(b)`，且存在 `b = 10` 的行，性能统计打开
- **WHEN** 执行 `SELECT * FROM t WHERE b = 10`
- **THEN** 摘要含有 `index_manager_search_all`
- **AND** 返回的行与诊断关闭时相同

#### Scenario: 页读仍记在当时开着的阶段

- **GIVEN** 只读打开的文件库尚未把目标数据页放进缓冲池，性能统计打开
- **WHEN** 执行要读取该页的主键等值查询
- **THEN** 摘要的文件页数至少为 1，解析阶段的 Io 为 0
- **AND** 摘要同时含有 `index_manager_search`
- **AND** 查询结果与诊断关闭时相同

#### Scenario: 排除 Index 时不出现索引子指标

- **GIVEN** 性能统计打开，观察类别只有 Pipeline
- **WHEN** 执行一条主键等值查询
- **THEN** 摘要没有 `index_manager_search` 和 `index_manager_search_all`
- **AND** 摘要仍有 execute
- **AND** 查询结果与诊断关闭时相同

#### Scenario: 排除 Pipeline 时不出现三个流水线子指标

- **GIVEN** 性能统计打开，观察类别只有 Index
- **WHEN** 成功插入一行
- **THEN** 摘要没有 `table_metadata_lookup`、`executor_creation` 和 `executor_execution`
- **AND** 插入的行在随后的查询中可见

#### Scenario: 旧表仍包含索引子指标

- **GIVEN** `RTSQL_PROFILING` 已设置
- **WHEN** 经 CLI 执行 `SELECT * FROM t WHERE id = 1`
- **THEN** stderr 的旧三段计时表含有 `index_manager_search`
- **AND** 查询结果仍含目标行

### Requirement: 比较指标从保留摘要读出

比较指标 SHALL 从会话已经保留的语句摘要读出，SHALL NOT 进入 SQL 结果列，SHALL NOT 改变错误文本或 SQLSTATE，SHALL NOT 写入诊断渲染文本。

每条摘要 SHALL 带有语句开始时刻和成功交回的结果行数。首行等待 SHALL 是从该语句开始到第一行被放进成功的 `SqlOutcome::Rows` 的时长；没有成功交回至少一行时，首行等待 SHALL 不存在。插入、空结果、返回错误，以及语句 future 在完成前被放弃，SHALL 没有首行等待，交回行数 SHALL 为 0。语句总耗时 SHALL 仍是根阶段包含耗时。INSERT SELECT 源、CTAS 源和派生表物化中的行 SHALL NOT 计入交回行数，也 SHALL NOT 产生首行等待。

吞吐和较慢请求 SHALL 只使用当前保留的那些摘要，SHALL NOT 另建场景对象。没有保留摘要时，这两项 SHALL 都不给出数值。

墙钟 SHALL 是这些摘要中最早的语句开始到最晚的语句结束。结束时刻 SHALL 等于该语句的开始时刻加上语句总耗时。墙钟 SHALL 用饱和减法，且 SHALL NOT 为负。

吞吐 SHALL 等于这些摘要的交回行数之和除以墙钟秒数，单位是行每秒。墙钟为 0 时 SHALL NOT 给出吞吐。交回行数之和为 0 且墙钟大于 0 时，吞吐 SHALL 为 0。

较慢请求 SHALL 是这些摘要的语句总耗时的 P95。将总耗时按升序排成 `t_1 ≤ … ≤ t_N`，下标从 1 计。`N = 0` 时 SHALL NOT 给出 P95。否则秩 `r = ceil(95 × N / 100)`，P95 SHALL 等于 `t_r`。SHALL NOT 插值，也 SHALL NOT 另设最小样本数。

内存峰值 SHALL 是进程已有的峰值。Linux 上 SHALL 读取 `getrusage(RUSAGE_SELF)` 的 `ru_maxrss`，并按 KiB 乘以 1024 换算成字节。读取失败时 SHALL NOT 给出数值。非 Linux SHALL NOT 给出数值。该峰值 SHALL NOT 读取查询预算的入账，也 SHALL NOT 读取某一种分配器的统计。查询内存上限为 0 时，查询预算的入账 SHALL 仍立即返回且不计字节。

#### Scenario: 非空查询有首行等待

- **GIVEN** 表中至少有一行，性能统计打开
- **WHEN** 查询这些行
- **THEN** 摘要的首行等待存在，且不大于语句总耗时
- **AND** 交回行数等于返回的行数
- **AND** 返回的行与诊断关闭时相同

#### Scenario: 插入和空结果没有首行等待

- **GIVEN** 性能统计打开
- **WHEN** 成功插入一行，再查询一个不存在的键
- **THEN** 两条摘要都没有首行等待
- **AND** 插入的交回行数为 0，空查询的交回行数为 0
- **AND** 插入的受影响行数与诊断关闭时相同

#### Scenario: 出错没有首行等待且错误不变

- **GIVEN** 同一条无法解析的语句
- **WHEN** 在诊断关闭和只开性能统计时各执行一次
- **THEN** 两次的错误文本和 SQLSTATE 相同
- **AND** 只开性能统计的那条摘要没有首行等待

#### Scenario: 被放弃的语句没有首行等待

- **GIVEN** 性能统计打开，一条隐式插入的 future 在提交前被放弃
- **WHEN** 同一会话随后插入原先被放弃的键
- **THEN** 被放弃语句的摘要没有首行等待，交回行数为 0
- **AND** 该键的再次插入成功

#### Scenario: 段落只含保留下来的摘要

- **GIVEN** 会话摘要上限为 1，性能统计打开
- **WHEN** 连续成功执行两条都会交回行的查询
- **THEN** 吞吐和 P95 只根据后一条摘要计算
- **AND** 两条查询各自的结果行都与诊断关闭时相同

#### Scenario: P95 取最近秩的上界

- **GIVEN** 一组只带语句总耗时的摘要，没有执行 SQL
- **WHEN** 计算 P95
- **THEN** 没有摘要时不给出 P95
- **AND** 一条摘要时 P95 等于该总耗时
- **AND** 两条摘要时 P95 等于较大的总耗时
- **AND** 二十条摘要时 P95 等于升序后的第 19 条

#### Scenario: 墙钟为零或行数为零

- **GIVEN** 一组构造好的摘要
- **WHEN** 计算吞吐
- **THEN** 墙钟为 0 时不给出吞吐
- **AND** 交回行数之和为 0 且墙钟大于 0 时，吞吐为 0

#### Scenario: 进程峰值不读查询预算

- **GIVEN** Linux 上查询内存上限为 0
- **WHEN** 读取进程内存峰值，并执行一条已知结果的查询
- **THEN** 峰值等于当时 `ru_maxrss` 的 KiB 数乘以 1024
- **AND** 查询返回原来的行，且不报查询预算超限

#### Scenario: 比较指标不进入 SQL 结果

- **GIVEN** 性能统计打开
- **WHEN** 执行一条成功查询
- **THEN** 返回行与诊断关闭时相同
- **AND** 行内容不包含首行等待、吞吐、P95 或内存峰值这些读出
