# 任务清单

> 最后更新：2026-05-20

## 进行中

- [ ] （无）

## 待办 - 开发路线图

### M0: 项目骨架，引入 Tokio

- [ ] 初始化 Rust 项目（`cargo init`）
- [ ] 添加 Tokio 依赖到 Cargo.toml
- [ ] 配置 Tokio 多线程运行时
- [ ] 创建基础模块结构
  - [ ] src/storage/
  - [ ] src/executor/
  - [ ] src/transaction/
  - [ ] src/parser/
  - [ ] src/network/
- [ ] 初始化 git 仓库
- [ ] 编写第一个基础测试验证 Tokio 运行时工作

**异步相关重点**: 确定异步运行时配置

### M1: 文件/缓存层

- [ ] 实现 `AsyncStorage` trait
- [ ] 使用 `spawn_blocking` 读页
- [ ] 实现异步 Buffer Pool
- [ ] 实现 `get_page(page_id) -> PageFuture`
- [ ] 测试页加载/淘汰逻辑

**异步相关重点**: 实现 `AsyncStorage` trait，使用 `spawn_blocking` 读页

### M2: B-Tree 索引与存储引擎

- [ ] 实现同步 B-Tree 索引内核
- [ ] 通过 `spawn_blocking` 暴露为 async API
- [ ] 实现 Slotted Page 行存储格式
- [ ] 测试索引操作正确性

**异步相关重点**: 索引同步，通过 `spawn_blocking` 暴露为 async API

### M3: 事务与 MVCC

- [ ] 实现全局事务 ID 分配（`AtomicU64`）
- [ ] 实现 MVCC 快照读（无锁）
- [ ] 实现异步读写锁（`tokio::sync::RwLock`）
- [ ] 测试并发事务正确性

**异步相关重点**: 用异步锁实现提交等待，快照读无锁

### M4: SQL 解析与计划

- [ ] 集成 sqlparser-rs
- [ ] 实现同步解析
- [ ] 生成物理计划（包含 async 节点）
- [ ] 测试解析正确性

**异步相关重点**: 同步解析，生成物理计划（包含 async 节点）

### M5: 异步执行引擎

- [ ] 实现 `async fn next() -> Result<Option<Row>>` 迭代器
- [ ] 整合存储异步接口
- [ ] 支持流式返回结果
- [ ] 测试执行引擎

**异步相关重点**: 实现 `async fn next()` 迭代器，整合存储异步接口

### M6: 全流程集成 + 网络层

- [ ] 实现 TCP 服务器（`tokio::net::TcpListener`）
- [ ] 每个连接一个协程处理
- [ ] 实现 PostgreSQL 有线协议或自定义协议
- [ ] 端到端测试

**异步相关重点**: 实现 TCP 服务器，每个连接一个协程

### M7: 性能深度优化

- [ ] 替换 `io_uring`（可选）
- [ ] 调优协程调度策略
- [ ] 调优页缓存策略
- [ ] 性能基准测试

**异步相关重点**: 替换 `io_uring`，调优协程调度、页缓存策略

## 阻塞项

- （无）

## 下一步

- **立即开始**: M0 里程碑 - 初始化项目骨架