# 外部参考资料

> 最后更新：2026-05-22

## 核心依赖

| 依赖 | 文档链接 | 关键概念 |
|------|----------|----------|
| Tokio | https://tokio.rs/tokio/tutorial | 异步运行时，多线程调度器，spawn_blocking |
| sqlparser-rs | https://github.com/ballista-compute/sqlparser-rs | SQL 解析器，支持 PostgreSQL 语法 |
| criterion.rs | https://bheisler.github.io/criterion.rs/book/ | Rust 基准测试框架，html_reports，async_tokio |
| rusqlite | https://docs.rs/rusqlite/latest/rusqlite/ | SQLite 绑定，用于对比测试 |
| tempfile | https://docs.rs/tempfile/latest/tempfile/ | 临时文件/目录，测试与 benchmark |
| serde_json | https://docs.rs/serde_json/latest/serde_json/ | JSON 序列化，协议层 |
| tokio-util | https://docs.rs/tokio-util/latest/tokio_util/ | CancellationToken，graceful shutdown |

## WAL 参考

| 主题 | 链接 | 说明 |
|------|------|------|
| WAL 基础 | https://www.sqlite.org/wal.html | SQLite WAL 文档 |
| ARIES 恢复 | https://cs.stanford.edu/people/csilv-22/aries.pdf | 学术经典 WAL 恢复算法 |
| PostgreSQL WAL | https://www.postgresql.org/docs/current/wal-internals.html | PG WAL 内部实现 |
| Checkpoint | https://www.sqlite.org/fileformat2.html#walformat | SQLite checkpoint 格式 |

## 领域知识

### 异步协程与数据库

- **协程调度**: 用户态无栈协程消除线程上下文切换，I/O 吞吐最大化
- **海量连接**: 数千连接复用少量工作线程
- **锁等待**: tokio::sync 实现，不阻塞物理线程
- **CPU 隔离**: spawn_blocking 移至阻塞线程池

### MVCC

- **读无锁**: 读取历史版本，不阻塞写
- **写冲突**: 异步锁挂起协程等待
- **快照读**: AtomicU64 分配事务 ID

### 存储引擎

- **Slotted Page**: 4KB 页，紧凑行存储
- **Buffer Pool**: 异步页加载/淘汰，两阶段锁
- **零拷贝**: PageDataGuard + SlottedPageRef

### 索引

- **B-Tree**: 同步内核，spawn_blocking 包装
- **哈希连接**: BuildRight → ScanLeft → Output

## 性能优化方向

| 方向 | 优先级 | 说明 |
|------|--------|------|
| io_uring | 低 | 零拷贝真异步磁盘读写 |
| jemalloc/mimalloc | 低 | 内存分配器优化 |
| 大查询并行化 | 低 | 全表扫描按页切分 |
| B-Tree split/merge | 高 | 索引完整性 |

## 测试策略

| 策略 | 工具 | 说明 |
|------|------|------|
| 单元测试 | cargo test | 83 lib tests |
| 集成测试 | tests/*.rs | 74 tests |
| 基准测试 | criterion.rs | 4 套 benchmark |
| SQLite 对比 | rusqlite | 性能对比参考 |