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

### M8: PostgreSQL 协议 + 性能优化

- [x] 实现 PostgreSQL 有线协议（Simple Query Protocol）
- [x] 实现 pg_messages 消息序列化层
- [x] 实现 PgProtocol 状态机
- [x] Server 切换到 PgProtocol
- [x] 集成测试（pg_integration_test.rs）
- [ ] psql 真实连接测试（环境限制：psql 未安装）

**完成日期**: 2026-05-20
**验证结果**: cargo test (159 passed) ✅, cargo clippy ✅, cargo fmt ✅
**新增文件**: pg_messages.rs, pg_protocol.rs, pg_integration_test.rs, pg_messages_test.rs, pg_protocol_test.rs
**推迟功能**: Extended Protocol, SSL/TLS, 二进制格式

**注意**: psql 真实连接测试需要安装 PostgreSQL 客户端工具

## 待办 - 开发路线图（重新规划）

> 嵌入式数据库核心功能优先级调整（2026-05-20）

### M9: SQL 基础能力完善 🔴 高优先级

**目标**: 让用户能通过 SQL 正常使用数据库

- [ ] DDL: CREATE TABLE/DROP TABLE（扩展 Parser + PlanBuilder）
- [ ] 复杂 WHERE 表达式计算（ExpressionEvaluator，支持 `AND/OR/>/<`）
- [ ] ORDER BY 排序
- [ ] LIMIT/OFFSET 分页
- [ ] 聚合函数（COUNT/SUM/AVG，可选）

**阻塞问题**: 当前用户必须通过 TableManager API 创建表，无法用 SQL

---

### M10: MVCC 完整性 🟡 中优先级

**目标**: 完整的多版本并发控制

- [ ] 完整版本链遍历（follow `next_version` 找第一个可见版本）
- [ ] 版本链 GC（清理已提交的旧版本，防止版本链过长）
- [ ] Read Committed 隔离级别（除现有 Repeatable Read）
- [ ] 事务回滚（Abort 时清理未提交版本）

**当前状态**: M7 仅验证最新版本可见性，无法访问历史版本

---

### M11: WAL 持久化 🔴 高优先级

**目标**: 嵌入式数据库崩溃恢复能力

- [ ] WAL（Write-Ahead Logging）写入流程
- [ ] WAL 重放恢复（启动时重做未完成事务）
- [ ] Checkpoint 机制（定期刷盘 + 截断 WAL）
- [ ] 原子性保障（事务提交前 WAL 必须持久化）

**必要性**: 嵌入式数据库崩溃恢复必需，持久化保障

---

### M12: JOIN 多表 🟢 低优先级

**目标**: 多表查询能力

- [ ] INNER JOIN 实现（两表连接）
- [ ] LEFT/RIGHT JOIN（可选）
- [ ] 多表 WHERE 条件
- [ ] JoinExecutor 实现

**理由**: 嵌入式场景可能单表为主，但 JOIN 是 SQL 标准功能

---

### M13: 性能优化与完善

**目标**: 高性能嵌入式数据库

- [ ] io_uring 替换（可选，Linux 5.1+）
- [ ] 协程调度优化（Tokio 配置调优）
- [ ] 性能基准测试（sysbench/sqllogictest）
- [ ] 连接池（嵌入式场景可选）
- [ ] 内存分配器优化（jemalloc/mimalloc）

---

### 推迟/可选功能

| 功能 | 状态 | 说明 |
|------|------|------|
| PostgreSQL Extended Protocol | 推迟 | 嵌入式数据库可能不需要 prepared statement |
| SSL/TLS | 推迟 | 嵌入式场景通常本地访问 |
| 二进制格式（format_code=1） | 推迟 | 文本格式足够 |
| psql 真实连接测试 | 可选 | PostgreSQL 协议层可能分离/删除 |

---

## 阻塞项

- **当前阻塞**: 用户无法通过 SQL 创建表（必须用 TableManager API）
- **影响范围**: 所有需要测试 SQL 功能的场景

---

## 下一步行动

**立即开始**: M9 SQL 基础能力完善
- 优先实现 CREATE TABLE DDL
- 然后实现 WHERE 表达式计算

**里程碑顺序**: M9 → M10 → M11 → M12 → M13
