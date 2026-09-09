## Purpose

记录有证据但尚未承诺实施的改进机会。条目使用 `Ixx` 编号，按 Phase 或主题分类。批准实施后创建 OpenSpec change 并标记 `promoted`。

## Requirements

### Requirement: 改进项可评估

改进项 SHALL 包含分类、问题、证据、影响、建议和状态。

#### Scenario: 发现未排期问题

- **WHEN** 已有证据表明存在改进机会但尚未批准实施
- **THEN** 使用递增 I 编号记录

#### Scenario: 批准实施

- **WHEN** 用户批准实施改进项
- **THEN** 创建 OpenSpec change 并把原条目标记 promoted

---

## Phase 1 基础设施（已完成）

**I001-I003 全部完成**（M41/M30/M38，详见 R08-013 + K14-16 + D09-10）

## Phase 2 存储引擎核心（已完成）

**I004-I008 全部完成**（M20/M19/M21/M36/M19，详见 K17-19, K22, D11-12, R10-013）

## Phase 3 并发控制

## I009: M40 RowLockTable DashMap

- **分类**: 性能 / 并发
- **问题**: `Arc<Mutex<HashMap>>` 行锁获取/释放串行化
- **方案**: `DashMap<RowId, Arc<Mutex<()>>>`
- **预期**: 行锁争抢 -5-10x
- **依赖**: M31（已完成）
- **状态**: planned（P3）
- **Legacy**: O009

## I010: M34 WAL fsync 合并

- **分类**: 性能 / WAL
- **问题**: 每事务提交单独 fsync，系统调用开销巨大
- **方案**: `tokio::time::interval` 定时器 + 累积多条记录一次 fsync
- **预期**: TPS 3-10x
- **依赖**: 无（M30 完成后可立即开始）
- **状态**: planned（P3）
- **Legacy**: O010

## I011: M32 WAL 写入背压

- **分类**: 性能 / WAL
- **问题**: WAL 无背压，高并发缓冲区膨胀
- **方案**: `Semaphore(WAL_MAX_PENDING)` 限制等待刷盘事务数
- **依赖**: M34（I010）
- **状态**: planned（P3）
- **Legacy**: O011

## I012: M42 消息传递重构

- **分类**: 架构 / 可维护性
- **问题**: 多个模块用 `Arc<Mutex<_>>` 共享状态，实为生产者-消费者模式
- **方案**: WAL→mpsc，提交→oneshot，Checkpoint→Notify，BufferPool→watch
- **依赖**: M32（I011）
- **状态**: planned（P3）
- **Legacy**: O012

## I013: M48 pread/pwrite 替代 seek+read

- **分类**: 性能 / 系统调用
- **问题**: 文件读写用 `seek()+read()/write()` 两次 syscall
- **方案**: `FileExt::read_at()` / `write_at()` 单次 syscall
- **预期**: syscall -50%
- **状态**: planned（P3，独立）
- **Legacy**: O013

## Phase 4 上层功能

## I014: M24 多隔离级别

- **分类**: 功能 / SQL 标准
- **问题**: 只有 Repeatable Read
- **方案**: Read Committed + Serializable（SSI）
- **依赖**: 无
- **状态**: planned（P4）
- **Legacy**: O014

## I015: M25 多 Join 算法

- **分类**: 功能 / 查询优化
- **问题**: 只有 Hash Join
- **方案**: NLJ + SMJ + 启发式选择
- **依赖**: 无
- **状态**: planned（P4）
- **Legacy**: O015

## I016: M26 代价模型 + Join 重排

- **分类**: 功能 / 优化器
- **问题**: 固定 join 顺序，无 cardinality/selectivity
- **方案**: `TableStatistics` + `CostEstimator` + DP/贪心重排
- **依赖**: M25（I015）
- **状态**: planned（P4）
- **Legacy**: O016

## I017: M27 关联子查询缓存

- **分类**: 性能 / 子查询
- **问题**: 每行外层重新执行子查询
- **方案**: `SubqueryCache` 参数值→结果集 LRU
- **依赖**: 无
- **状态**: planned（P4）
- **Legacy**: O017

## I018: M28 多层关联子查询

- **分类**: 功能 / 子查询
- **问题**: 显式拒绝多层嵌套
- **方案**: 递归遍历 + 多层注入
- **依赖**: M27（I017）
- **状态**: planned（P4）
- **Legacy**: O018

## I019: M29 PG Extended Query Protocol

- **分类**: 功能 / 协议
- **问题**: 只有 Simple Query
- **方案**: Parse/Bind/Describe/Execute + Prepared Statement
- **依赖**: M38（已完成）
- **状态**: planned（P4）
- **Legacy**: O019

## I020: M37 clone 消除 Arc/Cow

- **分类**: 性能 / 分配
- **问题**: `Value::clone()` 在聚合/排序/JOIN 中反复调用
- **方案**: `Value::Text` 内部 `Arc<str>` + `Cow<'_, str>` 延迟分配
- **依赖**: M20（已完成）
- **状态**: planned（P4）
- **Legacy**: O020

## I021: M39 INSERT 批量执行

- **分类**: 性能 / 写入
- **问题**: 多值 INSERT 逐行执行
- **方案**: `bulk_insert(keys)` + `append_batch(records)`
- **依赖**: M20（已完成）
- **状态**: planned（P4）
- **Legacy**: O021

## I022: M44 表定义持久化

- **分类**: 功能 / 持久化
- **问题**: `TableManager` 纯内存，重启丢失
- **方案**: Schema Page（系统表 `__tables` / `__columns`）
- **依赖**: 无
- **状态**: planned（P4）
- **Legacy**: O022, K05

## Phase 5 高级优化

## I023: M22 预取 Prefetch

- **分类**: 性能 / I/O
- **问题**: 顺序扫描逐页读，I/O 延迟未重叠
- **方案**: `Prefetcher` 双缓冲 + 异步预取下一页
- **预期**: 大表 ~15-25%
- **依赖**: M19（已完成）+ M31（已完成）
- **状态**: planned（P5）
- **Legacy**: O023, D12 下游

## I024: M23 Varint Key 编码

- **分类**: 性能 / 存储
- **问题**: 固定 32B Key，INT PK 浪费 ~28B
- **方案**: `Key` 内部 `Vec<u8>` 变长编码
- **预期**: 索引空间 ~70% 缩减
- **依赖**: 无
- **状态**: planned（P5）
- **Legacy**: O024, D02 successor

## I025: M33 B+Tree 节点级锁

- **分类**: 并发 / 锁
- **问题**: 每次操作 lock 整棵树
- **方案**: Semaphore 限流 + latch coupling（crabbing protocol）
- **依赖**: M23（I024）
- **状态**: planned（P5）
- **Legacy**: O025

## I026: M35 脏页 writev 批量写回

- **分类**: 性能 / Checkpoint
- **问题**: 逐页 `write_at()` + 单独 `fsync()`
- **方案**: 连续页合并写 + `writev()` 向量化
- **预期**: Checkpoint 5-10x
- **依赖**: M31（已完成）+ M48（I013）
- **状态**: planned（P5）
- **Legacy**: O026, D12 下游

## I027: M43 并行扫描

- **分类**: 性能 / 并行
- **问题**: 全表扫描单线程
- **方案**: 按页范围分区 + `mpsc` 汇聚
- **依赖**: M19（已完成）+ M22（I023）
- **状态**: planned（P5）
- **Legacy**: O027

## I028: M45 io_uring 批量提交

- **分类**: 性能 / I/O
- **问题**: `tokio::fs` 底层 `spawn_blocking`，每次 I/O 一次 syscall
- **方案**: `tokio-uring` 批量提交（IOSQE_IO_LINK）
- **预期**: I/O 延迟 -30-50%
- **依赖**: Linux 5.1+
- **状态**: planned（P5）
- **Legacy**: O028, K36

## 长期方向（未规划具体里程碑）

## I029: M46 瘦内部节点

- **分类**: 性能 / 存储
- **问题**: B+Tree 内部节点只存 separator keys，不存完整 Key
- **难度**: 高，需重构 B+Tree 分裂/合并逻辑
- **依赖**: M23（I024）
- **状态**: long-term
- **Legacy**: O029

## I030: M47 合并 Tag byte

- **分类**: 性能 / 序列化
- **问题**: Slot Tag byte 合并进 VersionHeader，省 1 byte/slot
- **难度**: 低，但影响序列化格式兼容性
- **状态**: long-term
- **Legacy**: O030

## I031: B-Tree 撕裂树运行期根修（结构感知刷盘/驱逐改造）

- **分类**: 正确性 / 存储引擎
- **问题**: 页驱逐按 LRU 而非树拓扑刷盘——checkpoint 后的运行期修改使磁盘 B-Tree 含洞（父页指向未刷盘子页）与孤儿页（裸读实测 `scan_all` 184/10000 + `InvalidPageType` 洞页）。恢复侧已由 `redo_count > 0` 时索引去信任 + 重放后重建消解（MS10-T02 R7/R8），但运行期检查点间的磁盘树仍处于撕裂状态；MS12 整库加密若引入页级 transform 将放大对磁盘树一致性的依赖
- **候选方案**: 结构感知刷盘（子树后序）/ checkpoint 树快照 / no-steal 驱逐——均为 BufferPool 驱逐策略重设计，MS10-T02 design D10 拒绝并入
- **量化支撑**: D10 恢复重建代价实测 40k 行→8.86s、160k 行→40.7s（100 页池随机访存主导；撕裂树运行期根修可同时压缩该恢复代价）
- **状态**: planned（与 MS08-T03 脏页 writev 同域，实施前先量化）

## I032: `BufferPool::mark_tx_aborted` 空实现补全

- **分类**: 正确性 / 事务恢复
- **问题**: `mark_tx_aborted` 为 no-op（`buffer_pool.rs:369-371`）——`RecoveryManager::full_recover` 的 mark-uncommitted-aborted 步骤实际空转；未提交行仅靠 header `commit_tx_id=None` 的不可见性兜底，aborted 行物理滞留数据页
- **影响**: 当前语义自洽（未提交行不可见、重建谓词按 committed 排除），但未提交事务的页空间不可回收，且未来依赖「aborted 标记」的机制（如空间回收、更细的可见性）将踩空
- **方案**: 恢复期对 uncommitted 事务的行打显式 aborted 标记（header 扩展或墓碑化），或明确文档化「无标记」模型
- **状态**: planned（小改动，随下次触碰 transaction/recovery 面顺带评估）

## I033: update→delete 行旧版本在无快照扫描重现

- **分类**: 正确性 / 执行器
- **问题**: 版本链 T→A（update）→ tombstone（delete）中，墓碑替代者不抑制前驱（`superseder_suppresses` 对 `is_deleted` 恒 false，MS10-T02 R6 精确保留既有语义）→ 无快照扫描产出已删除行的旧版本 T
- **影响**: 语义为「既有行为未扩大」（R6 前同样重现）；MS10-T02 验收夹具 UPDATE/DELETE 域不相交故未触发；未来混合负载计数会虚高
- **方案**: 抑制谓词区分「已提交墓碑」（应抑制整条链）与「未提交墓碑」（不抑制、回溯前驱）——需对照 WAL committed 集合或 header 编码扩展
- **状态**: planned（与 MS09-T01 隔离级别工作同域，届时一并处理）

## I034: 裸 DataScan 子集投影的 CLI 表头返回全 schema

- **分类**: 正确性 / CLI 渲染
- **问题**: 无 WHERE 的裸 DataScan 子集投影，CLI 表头经 `get_plan_output_columns`（plan 节点 columns 元数据）返回全 schema 而行已按 projection 裁剪——`SELECT name FROM t` 表头 `["id","name"]`、行 `[["Alice"]]`；带 WHERE 的 IndexScan 路径表头正确（`["name"]`）。spec `cli-noninteractive-shell` R6 S1「表头 ["name"]」的 bare-DataScan 分支自 MS10-T01 起未满足
- **证据**: MS10-T04 Plan Review finding 5 独立探针复现（2026-09-09，revision `8827700`）；`tests/projection_test.rs`（MS10-T01）只断言 lib 行形状，未覆盖 CLI 表头
- **影响**: json 输出 `columns` 与 `rows` 字段数不一致，机器消费需二次裁剪；表格输出表头错位
- **方案**: `get_plan_output_columns` 对裸 DataScan 节点按 `projection` 索引裁剪表头；修正后同步校准 `tests/cli_test.rs::test_multi_statement_sequential_render` 的注释登记
- **状态**: planned

## I035: no-FROM SELECT 不受支持

- **分类**: 功能 / SQL 能力边界
- **问题**: `SELECT 1`（无 FROM 子句）在 plan 阶段报 `Plan error: Missing required field: FROM clause`（exit 3）——MS10-T04 S5 分号边界见证原拟 SQL 因此不可达，修订为含 FROM 等价 SQL（用户批准，见归档 change Blocker Handoff/Resolution）；`tests/cli_test.rs:481-482` 既有注释同证
- **证据**: MS10-T04 Act Blocker Handoff + Plan Review 独立复现（2026-09-09，revision `5c42ec8`）
- **影响**: 常量表达式查询、`SELECT current_setting()` 类无表探测不可达；agent/脚本日常探测用法受挫
- **方案**: planner 增加 no-FROM SELECT 臂（单行虚拟输入），属引擎能力扩展，需独立 change
- **状态**: planned
