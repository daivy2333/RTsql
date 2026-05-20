# 外部参考资料

> 收录依赖文档链接、重要概念、常见解决方案

## 核心依赖

| 依赖 | 文档链接 | 关键概念 |
|------|----------|----------|
| Tokio | https://tokio.rs/tokio/tutorial | 异步运行时，多线程调度器，spawn_blocking，io_uring |
| sqlparser-rs | https://github.com/ballista-compute/sqlparser-rs | SQL 解析器，支持 PostgreSQL 语法 |
| zerocopy | https://docs.rs/zerocopy/latest/zerocopy/ | 零拷贝数据结构，Slotted Page 访问 |
| jemalloc | https://github.com/jemalloc/jemalloc | 内存分配器，减少碎片 |
| mimalloc | https://github.com/microsoft/mimalloc | 内存分配器，高性能 |
| sqllogictest | https://github.com/ballista-compute/sqllogictest | SQL 逻辑测试框架 |
| proptest | https://docs.rs/proptest/latest/proptest/ | 属性测试框架，覆盖边界场景 |

## 领域知识笔记

### 异步协程与数据库

- **协程调度优势**: 用户态无栈协程消除线程上下文切换，I/O 吞吐最大化
- **海量连接**: 数千连接复用少量工作线程，每个连接仅占用极少量内存（无独立栈）
- **锁等待**: 通过 `tokio::sync` 实现，不阻塞物理线程
- **CPU隔离**: 重操作通过 `spawn_blocking` 移至阻塞线程池

### MVCC（多版本并发控制）

- **读无锁**: 读取历史版本，不阻塞写操作
- **写冲突**: 通过异步锁挂起协程等待
- **快照读**: 基于 `AtomicU64` 分配事务ID

### 存储引擎

- **Slotted Page**: 4KB 页，紧凑行存储
- **Buffer Pool**: 异步页加载/淘汰，`get_page() -> Future`
- **WAL**: Write-Ahead Logging，保证持久性

### 索引

- **B-Tree**: 同步内核，外部 async 包装
- **LSM-Tree**: 可选方案，适合写入密集场景

## 性能优化方向

- **io_uring**: 零拷贝真异步磁盘读写（M7 阶段）
- **页缓存策略**: 调优缓存大小、淘汰算法
- **协程调度**: 调优工作线程数、任务优先级

## 测试策略

- **sqllogictest**: 验证 SQL 兼容性
- **proptest**: 覆盖边界场景（空输入、超大输入、边界值）
- **并发测试**: 验证 MVCC 正确性，多事务并发场景