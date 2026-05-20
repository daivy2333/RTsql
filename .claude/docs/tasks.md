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

### M5: 异步执行引擎 ✅

- [x] 实现 ExecResult enum（RowId/AffectedRows/NotImplemented）
- [x] 实现 Executor trait（async fn next()）
- [x] 实现 ScanExecutor（返回 NotImplemented）
- [x] 实现 IndexScanExecutor（主键索引查找）
- [x] 实现 InsertExecutor（批量插入）
- [x] 实现 UpdateExecutor（更新 RowId）
- [x] 实现 DeleteExecutor（删除）
- [x] 单元测试（tests/executor_test.rs）
- [x] 集成测试（tests/plan_exec_test.rs）

**完成日期**: 2026-05-20
**验证结果**: cargo test (115 passed) ✅, cargo clippy ✅, cargo fmt ✅
**新增测试**: executor_test(7), plan_exec_test(4)
**范围**: 仅索引层执行，返回 RowId（数据层推迟 M6）
**新增文件**: result.rs, executor_trait.rs, scan.rs, index_scan.rs, insert.rs, update.rs, delete.rs

### M6: 网络层 ✅

- [x] 添加依赖（tokio-util, serde, serde_json）
- [x] 实现 NetworkError 错误类型
- [x] 实现 Protocol trait + Request/Response
- [x] 实现 JsonProtocol（newline-delimited framing）
- [x] 实现 SqlHandler（mock executor）
- [x] 实现 ConnectionHandler（每连接一协程）
- [x] 实现 Server（TcpListener + graceful shutdown）
- [x] 单元测试（tests/network_protocol_test.rs）
- [x] 集成测试（tests/network_server_test.rs）

**完成日期**: 2026-05-20
**验证结果**: cargo test (124 passed) ✅, cargo clippy ✅, cargo fmt ✅
**新增测试**: network_protocol_test(5), network_server_test(4)
**范围**: 仅网络层，mock executor（数据存储层推迟后续里程碑）
**新增文件**: error.rs, protocol.rs, connection.rs, handler.rs, server.rs
**协议**: JSON 协议（后续升级 PostgreSQL）

### M7: 全流程集成 + 数据存储层

- [ ] 实现数据存储层（TableManager、Row 数据存储）
- [ ] 整合真实 Executor + Storage + Transaction
- [ ] 替换 mock executor 为真实 executor
- [ ] 端到端测试（真实 SQL 执行）

**异步相关重点**: 实现数据存储层，整合全流程

### M8: PostgreSQL 协议 + 性能优化

- [ ] 实现 PostgreSQL 有线协议（兼容 psql 等工具）
- [ ] 替换 `io_uring`（可选）
- [ ] 调优协程调度策略
- [ ] 调优页缓存策略
- [ ] 性能基准测试

**异步相关重点**: PostgreSQL 协议、io_uring、性能调优

## 阻塞项

- （无）

## 下一步

- **立即开始**: M7 里程碑 - 数据存储层 + 全流程集成