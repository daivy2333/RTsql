# 优化方向与技术债

> 最后更新：2026-05-24（新增 M24-M29 规划）

## 高优先级（已规划为 M19-M23）

### 1. Index-to-Data 双读 — DataScan 路径 [M19]

**问题**：全表扫描走 B+tree → RowId → 数据页，每行双读。SQLite 只有数据页。

**当前影响**：Full Scan 1K rows 327µs vs SQLite 80µs（4x slower）

**方案**：新增 DataScanExecutor，直接按 page_id 顺序遍历数据页

**状态**：📋 已规划（M19）

---

### 2. 每行堆分配 — 零拷贝读取 [M20]

**问题**：`SlottedPage::get` 每行 `Vec::from` 堆分配，全表扫描 N 行 = N 次分配。

**当前影响**：I/O 密集场景 ~20-30% 性能损失

**方案**：返回 `&[u8]` 切片引用消除堆分配

**状态**：📋 已规划（M20）

---

### 3. 每行 VersionHeader — 页面级 MVCC [M21]

**问题**：每行 16 字节 VersionHeader，全表扫描逐行检查可见性。

**当前影响**：纯开销，~10-15% 性能损失

**方案**：页面级可见性标记，整页可跳过时跳过

**状态**：📋 已规划（M21）

---

### 4. 无预读 — 预取 Prefetch [M22]

**问题**：顺序扫描无 read-ahead，每次只读一页。

**当前影响**：大表 I/O 延迟未重叠，~15-25% 性能损失

**方案**：异步预读下一页，双缓冲

**状态**：📋 已规划（M22）

---

### 5. 固定 Key 32B — Varint Key 编码 [M23]

**问题**：B-Tree Key 固定 32 bytes，INT PRIMARY KEY 浪费 ~28 bytes。

**当前影响**：索引空间 ~10x 膨胀，文件 6.5x larger than SQLite

**方案**：varint 编码（1-9 bytes），大幅缩减索引页

**状态**：📋 已规划（M23）

---

## 中优先级（已规划为 M24-M29）

### 6. 隔离级别单一 [M24]

**问题**：只实现了 Repeatable Read（快照隔离），无 Read Committed / Serializable。

**当前影响**：无法满足不同业务场景的隔离需求；Read Committed 可减少写冲突回滚，Serializable 可防止写偏序。

**方案**：
- Read Committed：每条语句重新获取 snapshot
- Serializable：SSI（Serializable Snapshot Isolation）+ predicate locking

**状态**：📋 已规划（M24）

---

### 7. Join 算法单一 [M25]

**问题**：只有 Hash Join，无 Nested Loop Join / Sort-Merge Join。

**当前影响**：小表 join 或有序数据场景效率低；Hash Join 需要 build 整个 hash table，对小数据集不划算。

**方案**：
- Nested Loop Join：适用于小表驱动大表
- Sort-Merge Join：适用于已排序数据
- 代价模型自动选择 Join 算法

**状态**：📋 已规划（M25）

---

### 8. 无 Join 重排序 / 代价模型 [M26]

**问题**：Planner 固定 join 顺序，无 cardinality/selectivity 估算，无代价模型。

**当前影响**：多表 join 可能选到最差执行顺序；优化器无法做基于代价的决策。

**方案**：
- 统计信息收集（行数、NDV、直方图）
- 代价估算模型（CPU + I/O 代价）
- Join 重排序（动态规划 / 贪心）

**状态**：📋 已规划（M26）

---

### 9. 关联子查询无缓存 [M27]

**问题**：关联子查询每行外层都重新执行，无物化缓存。N 行外层 = N 次子查询执行。

**当前影响**：关联子查询性能随外层行数线性下降；同样参数重复执行浪费。

**方案**：
- 参数化缓存：相同关联参数值命中缓存
- 子查询物化：将子查询结果物化为临时表

**状态**：📋 已规划（M27）

---

### 10. 不支持多层关联子查询 [M28]

**问题**：代码显式拒绝多层嵌套关联子查询，复杂查询直接报错。

**当前影响**：无法执行 `WHERE EXISTS (SELECT ... WHERE col = (SELECT ...))` 等嵌套结构。

**方案**：
- 递归注入外层参数到多层子查询
- 逐层解析关联列引用

**状态**：📋 已规划（M28）

---

### 11. PG 协议只支持 Simple Query [M29]

**问题**：无 Extended Query Protocol (Parse/Bind/Describe/Execute)。

**当前影响**：预编译语句不可用；二进制传输不可用；每次查询都要完整解析。

**方案**：
- 实现 Parse → Bind → Describe → Execute 消息流
- 支持 prepared statement 缓存
- 二进制格式 DataRow 传输

**状态**：📋 已规划（M29）

---

## 中优先级（未来优化）

### 12. 全表扫描并行化

**问题**：单线程扫描，大表无法利用多核。

**方案**：分区并行扫描（每线程扫一段 page 范围）。

**状态**：💡 待规划

---

### 13. 表定义持久化

**问题**：表元信息仅在内存，重启丢失。

**方案**：Schema Page 持久化到文件头。

**状态**：💡 待规划

---

### 14. io_uring 异步磁盘 I/O

**问题**：当前基于 tokio::fs，系统调用开销大。

**方案**：io_uring 批量提交 I/O 请求。

**状态**：💡 待规划

---

## 低优先级（技术债）

### 15. 两层索引结构

**问题**：B+tree 每层都是完整页（含 keys + pointers），内部节点空间利用率低。

**状态**：💡 长期考虑

---

### 16. Tag byte 开销

**问题**：每个 slot 有 Tag byte 用于标记删除/有效性，可合并到 VersionHeader。

**状态**：💡 长期考虑
