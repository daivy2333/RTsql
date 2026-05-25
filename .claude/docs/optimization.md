# 优化方向与技术债

> 最后更新：2026-05-25（精简合并，未规划条目已规划为 M43-M48）

## 优化索引（M19-M48）

| # | 里程碑 | 问题 | 方案 | 状态 |
|---|--------|------|------|------|
| 1 | M19 | 全表扫描走 B+tree 双读 | DataScan 直读数据页 | 📋 P2 |
| 2 | M20 | 每行 Vec 堆分配 | SlottedPageRef 零拷贝 | 📋 P2 |
| 3 | M21 | 逐行 VersionHeader 可见性检查 | 页面级 MVCC 快速跳过 | 📋 P2 |
| 4 | M22 | 顺序扫描无预读 | Prefetch 双缓冲 | 📋 P5 |
| 5 | M23 | 固定 32B Key 空间膨胀 | Varint Key 编码 | 📋 P5 |
| 6 | M24 | 只有 Repeatable Read | 多隔离级别 | 📋 P4 |
| 7 | M25 | 只有 Hash Join | NLJ + SMJ + 代价选择 | 📋 P4 |
| 8 | M26 | 无代价模型/Join 重排 | 统计信息+代价估算+重排序 | 📋 P4 |
| 9 | M27 | 关联子查询无缓存 | 参数化缓存+物化 | 📋 P4 |
| 10 | M28 | 不支持多层关联子查询 | 递归参数注入 | 📋 P4 |
| 11 | M29 | PG 只有 Simple Query | Extended Query Protocol | 📋 P4 |
| 12 | M30 | 连接并发无上限 | Semaphore 限流 | 📋 P1 |
| 13 | M31 | BufferPool Mutex 阻塞并发读 | DashMap + Semaphore | 📋 P3 |
| 14 | M32 | WAL 写入无背压 | Semaphore 刷盘限流 | 📋 P3 |
| 15 | M33 | B+Tree 操作全局锁 | Semaphore + 节点级锁 | 📋 P5 |
| 16 | M34 | 事务提交单独 fsync | Group Commit 定时合并 | 📋 P3 |
| 17 | M35 | 脏页逐页写回 | writev 向量化写 | 📋 P5 |
| 18 | M36 | 热路径堆分配 | ValueRef 借用枚举 | 📋 P2 |
| 19 | M37 | 热路径 clone | Arc/Cow 替代 String | 📋 P4 |
| 20 | M38 | 网络逐行 write | BufWriter + TCP_NODELAY | 📋 P1 |
| 21 | M39 | INSERT 逐行执行 | B+Tree bulk insert | 📋 P4 |
| 22 | M40 | RowLockTable 全局锁 | DashMap 分片 | 📋 P3 |
| 23 | M41 | 事务 ID Mutex | AtomicU64 无锁 | 📋 P1 |
| 24 | M42 | 共享状态 Mutex | mpsc/oneshot/Notify/watch | 📋 P3 |
| 25 | M43 | 单线程扫描 | 分区并行扫描 | 📋 P5 |
| 26 | M44 | 表定义纯内存 | Schema Page 持久化 | 📋 P4 |
| 27 | M45 | tokio::fs spawn_blocking 开销 | io_uring 批量提交 | 📋 P5 |
| 28 | M46 | 两层索引空间利用率低 | 瘦内部节点（只存 separator keys） | 💡 长期 |
| 29 | M47 | Slot Tag byte 重复标记 | 合并进 VersionHeader | 💡 长期 |
| 30 | M48 | seek+read 两次 syscall | pread/pwrite 单次调用 | 📋 P3 |