# 任务清单

> 最后更新：2026-05-20

## 进行中

- [ ] （无）

## 已完成

### M0: 项目骨架，引入 Tokio ✅

- [x] 初始化 Rust 项目
- [x] 添加 Tokio 依赖
- [x] 创建基础模块结构
- [x] 验证 Tokio 运行时

**完成日期**: 2026-05-20
**验证结果**: cargo test (3 passed) ✅

### M1: 文件/缓存层 ✅

- [x] AsyncStorage trait + FileStorage
- [x] BufferPool + Clock 淘汰 + PageGuard

**完成日期**: 2026-05-20
**验证结果**: cargo test (17 passed) ✅

### M2: B-Tree 索引与存储引擎 ✅

- [x] Key/RowId/SlottedPage 格式
- [x] LeafNode + InternalNode
- [x] BTree 核心逻辑 + IndexManager async API

**完成日期**: 2026-05-20
**验证结果**: cargo test (53 passed) ✅

### M3: 事务与 MVCC ✅

- [x] TransactionId + TransactionManager
- [x] Snapshot（Repeatable Read 可见性）
- [x] VersionHeader（22B 版本链）
- [x] RowLockTable（异步行锁）

**完成日期**: 2026-05-20
**验证结果**: cargo test (78 passed) ✅

### M4: SQL 解析与计划 ✅

- [x] sqlparser-rs 集成
- [x] PlanBuilder（AST → PhysicalPlan）
- [x] 5 节点 PhysicalPlan

**完成日期**: 2026-05-20
**验证结果**: cargo test (92 passed) ✅

### M5: 异步执行引擎 ✅

- [x] Executor trait（async fn next）
- [x] ExecResult + 5 Executor 实现
- [x] 单元测试 + 集成测试

**完成日期**: 2026-05-20
**验证结果**: cargo test (115 passed) ✅

### M6: 网络层 ✅

- [x] Protocol trait + JsonProtocol
- [x] Server + ConnectionHandler + SqlHandler (mock)
- [x] Graceful shutdown

**完成日期**: 2026-05-20
**验证结果**: cargo test (124 passed) ✅

### M7: 全流程集成 + 数据存储层 ✅

- [x] 实现 ColumnType + tuple 序列化（serialize/deserialize_tuple）
- [x] 实现 TableManager（表元数据注册、create_table/get_table）
- [x] 扩展 ExecResult（Row 变体）+ Response（rows 字段）
- [x] 实现 data_page 读写（write/read_tuple_to_data_page）
- [x] 实现真实 InsertExecutor（数据页写入 + 索引更新）
- [x] 实现 BTree::scan_all + IndexManager::scan_all
- [x] 重写 IndexScanExecutor（读 Tuple + MVCC 可见性过滤）
- [x] 重写 ScanExecutor（全表扫描）
- [x] 重写 UpdateExecutor（版本链创建）
- [x] 更新 DeleteExecutor（tx_id 字段）
- [x] MVCC 可见性集成（Snapshot.is_visible/is_visible_self）
- [x] 创建 Database 协调器结构（BufferPool+TableManager+TxManager）
- [x] 创建 SQL 执行管道（pipeline.rs）
- [x] 替换 mock SqlHandler → 真实 pipeline（async execute）
- [x] 端到端 TCP 测试（7 tests：insert/select/update/delete/ping/error）
- [x] 更新所有现有测试

**完成日期**: 2026-05-20
**验证结果**: cargo test (157 passed) ✅, cargo clippy ✅, cargo fmt ✅
**新增文件**: database.rs, pipeline.rs, tuple.rs, table_manager.rs, data_page.rs, e2e_test.rs
**新增测试**: 34 个（tuple:6 + table_mgr:6 + data_page:5 + executor:+4 MVCC + e2e:7 + table_manager:6）
**MVCC 范围**: M7 仅验证最新版本可见性，完整版本链遍历推迟到 M8

## 待办 - 开发路线图

### M8: PostgreSQL 协议 + 性能优化

- [ ] 实现 PostgreSQL 有线协议（兼容 psql 等工具）
- [ ] 实现完整版本链遍历（follow next_version）
- [ ] 实现 WAL（Write-Ahead Logging）
- [ ] 实现版本链 GC（清理旧版本）
- [ ] 复杂 WHERE 表达式计算
- [ ] JOIN 多表支持
- [ ] 替换 `io_uring`（可选）
- [ ] 调优协程调度策略
- [ ] 性能基准测试

**异步相关重点**: PostgreSQL 协议、io_uring、性能调优

## 阻塞项

- （无）

## 下一步

- **立即开始**: M8 里程碑 - PostgreSQL 协议 + 性能优化
