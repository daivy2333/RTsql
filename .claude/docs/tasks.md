# 任务清单

> 最后更新：2026-05-20

## 进行中

- [ ] （无）

## 已完成

### M0: 项目骨架，引入 Tokio ✅

- [x] 初始化 Rust 项目（`cargo init`）
- [x] 添加 Tokio 依赖到 Cargo.toml
- [x] 配置 Tokio 多线程运行时
- [x] 创建基础模块结构
  - [x] src/storage/
  - [x] src/executor/
  - [x] src/transaction/
  - [x] src/parser/
  - [x] src/network/
- [x] 初始化 git 仓库
- [x] 编写第一个基础测试验证 Tokio 运行时工作

**完成日期**: 2026-05-20
**验证结果**: cargo build ✅, cargo test (3 passed) ✅, cargo clippy ✅

### M1: 文件/缓存层 ✅

- [x] 实现 `AsyncStorage` trait
- [x] 使用 `spawn_blocking` 读页
- [x] 实现异步 Buffer Pool
- [x] 实现 `get_page(page_id) -> PageGuard`
- [x] 测试页加载/淘汰逻辑
- [x] 实现 Clock 淘汰算法
- [x] 实现 PageGuard（引用计数 + mark_dirty）

**完成日期**: 2026-05-20
**验证结果**: cargo test (17 passed) ✅, cargo clippy (1 warning, acceptable) ✅, cargo fmt ✅

## 待办 - 开发路线图

### M2: B-Tree 索引与存储引擎 ✅

- [x] 实现 Key 结构（固定 32 bytes）
- [x] 实现 RowId 结构（page_id + slot_id）
- [x] 实现 Slotted Page 通用格式
- [x] 实现 LeafNode + InternalNode 结构
- [x] 实现 SyncPageLoader（block_on 包装 BufferPool）
- [x] 实现 BTree 核心逻辑（insert/search/delete/update）
- [x] 实现 IndexManager 异步 API（spawn_blocking 包装）
- [x] 测试索引操作正确性（53 测试通过）

**完成日期**: 2026-05-20
**验证结果**: cargo test (53 passed) ✅, cargo clippy (11 warnings, acceptable) ✅, cargo fmt ✅
**简化实现**: Split/Merge 未完整实现（推迟到后续优化），固定 Key 长度（32 bytes）

### M3: 事务与 MVCC ✅

- [x] 实现 TransactionId（AtomicU64 全局分配）
- [x] 实现 TransactionError 错误类型
- [x] 实现 Snapshot（可见性判断）
- [x] 实现 VersionHeader（版本链头部）
- [x] 实现 RowLockTable（行级写锁）
- [x] 实现 TransactionManager（begin/commit/abort）
- [x] 测试并发事务正确性（78 测试通过）

**完成日期**: 2026-05-20
**验证结果**: cargo test (78 passed) ✅, cargo clippy (warnings acceptable) ✅
**新增测试**: tx_id(2), snapshot(5), version_chain(5), row_lock(3), manager(6), concurrent(4)

### M4: SQL 解析与计划 ✅

- [x] 集成 sqlparser-rs (0.44)
- [x] 实现 PlanError 错误类型
- [x] 实现 Value 类型（Int/String/Null）
- [x] 实现 PhysicalPlan + 5 节点结构（Scan/IndexScan/Insert/Update/Delete）
- [x] 实现 AST 辅助函数（parse_sql/extract_select_body/extract_table_name/extract_columns）
- [x] 实现 PlanBuilder（AST → PhysicalPlan）
- [x] 测试解析正确性（14 测试通过）

**完成日期**: 2026-05-20
**验证结果**: cargo test ✅, cargo clippy ✅, cargo fmt ✅
**新增测试**: parser_test(6), planner_test(8)
**范围**: DML Only（INSERT/UPDATE/DELETE/SELECT），单表+主键查询

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

- **立即开始**: M5 里程碑 - 异步执行引擎