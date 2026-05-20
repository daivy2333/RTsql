# 优化方向与技术债务

> 最后更新：2026-05-20（重新规划 - 嵌入式异步高性能最佳实践）
> 来源：嵌入式数据库异步高性能低功耗优化建议清单

---

## 🔴 Critical Issues（M9 必须修复）

### 1. PageGuard 使用 std::sync::Mutex（异步环境禁忌）

**问题**：
- `PageGuard` 使用 `std::sync::Mutex`（src/storage/page_frame.rs:26）
- 在异步环境中跨 `.await` 可能死锁
- 阻塞整个 Tokio 工作线程，导致性能严重下降

**示例**：
```rust
pub struct PageGuard {
    frame: Arc<Mutex<PageFrame>>,  // ❌ std::sync::Mutex
}
```

**修复方案**：
- 改用 `tokio::sync::Mutex`（异步安全）
- 或重构为不需要持锁跨 await 的设计

**影响范围**：
- `src/storage/page_frame.rs`
- `src/storage/buffer_pool.rs`

**优先级**：🔴 Critical（M9 必须修复）

---

### 2. page() 方法每次克隆 4KB（零拷贝缺失）

**问题**：
- `PageGuard::page()` 每次调用都克隆整个 Page（4KB）
- 频繁内存分配，性能开销巨大
- 违反零拷贝原则

**示例**：
```rust
pub fn page(&self) -> Page {
    self.frame.lock().unwrap().page.clone()  // ❌ 每次分配 4KB
}
```

**修复方案**：
- 改为返回引用：`pub fn page(&self) -> &[u8]`
- 或使用 `zerocopy` crate 映射固定长度字段
- 或手写安全代码直接指向页内切片

**影响范围**：
- `src/storage/page_frame.rs`
- `src/executor/*`（所有读页的 executor）

**优先级**：🔴 Critical（M9 必须修复）

---

### 3. BufferPool 持锁期间做 I/O（阻塞协程）

**问题**：
- `get_page()` 持写锁期间执行 `storage.read_page()`
- I/O 操作阻塞其他协程访问缓存
- 违反异步最佳实践

**示例**：
```rust
let mut pages = self.pages.write().await;  // 持写锁
let page = self.storage.read_page(page_id).await?;  // I/O 阻塞其他协程
```

**修复方案**：
- 使用两阶段锁：读锁检查 → 释放 → 异步 I/O → 写锁插入
- 或使用 `tokio::sync::Notify` 实现异步等待（无锁）
- 创建 `PageFuture`，未命中时挂起协程，唤醒后返回页

**影响范围**：
- `src/storage/buffer_pool.rs`

**优先级**：🔴 Critical（M9 必须修复）

---

## 🟡 Important Issues（M10-M11 修复）

### 4. 二进制协议替换（JSON → Binary） - 嵌入式核心需求

**问题**：
- 当前使用 JSON 序列化（serde_json）
- 文本格式开销大，不符合嵌入式高性能低功耗目标
- PostgreSQL 协议文本格式（format_code=0）效率低

**修复方案**：
- 实现 PostgreSQL 二进制格式（format_code=1）
- 内部数据传输使用紧凑二进制格式（已部分实现）
- 零拷贝解析（直接映射字节）

**影响范围**：
- `src/network/pg_messages.rs`（DataRow）
- `src/storage/page_format/tuple.rs`（已实现二进制，但可能优化）
- `src/executor/value.rs`

**优先级**：🟡 Important（M10 完成）

**注意**：我们的目标是异步高性能低功耗的嵌入式场景，二进制数据格式是核心需求！

---

### 5. MVCC 完整版本链遍历（M10）

**问题**：
- M7 仅验证最新版本可见性
- 无法访问历史版本（follow `next_version`）
- 长时间运行版本链过长

**修复方案**：
- 实现 `follow_version_chain()` 异步遍历
- 版本链 GC（清理已提交的旧版本）
- 后台协程定期清理

**影响范围**：
- `src/executor/index_scan.rs`
- `src/executor/scan.rs`
- `src/transaction/version_chain.rs`

**优先级**：🟡 Important（M10 完成）

---

### 6. WAL 后台协程（M11）

**问题**：
- WAL 写入可能阻塞查询协程
- 缺少后台 Checkpoint 协程
- 缺少 WAL 清理协程

**修复方案**：
- WAL 写入用 `tokio::spawn` 后台协程
- Checkpoint 用独立协程定期执行
- WAL 清理用后台协程

**影响范围**：
- `src/storage/wal.rs`（待实现）
- `src/transaction/manager.rs`

**优先级**：🟡 Important（M11 完成）

---

## 🟢 Performance Optimizations（M13 可选）

### 7. 大查询并行化

**方案**：
- 全表扫描按页切分
- 用 `tokio::spawn` 启动多个子协程并行处理
- 结果通过 `mpsc` 或 `FuturesUnordered` 收集
- 聚合操作两阶段：局部聚合子协程 + 全局聚合父协程

**优先级**：🟢 Low（M13 可选）

---

### 8. 锁精简化

**方案**：
- 用 `AtomicU64` 管理事务时间戳（避免锁）
- 索引修改才用 `RwLock`，短暂持有
- 用 `tokio::sync::Semaphore` 限制并发读取页数

**优先级**：🟢 Low（M13 可选）

---

### 9. 语义缓存

**方案**：
- SQL 逻辑计划规范化作为键
- 结果存为 `Arc<ResultSet>`，带 MVCC 版本号
- 多个相同查询共享结果
- 版本变更自动失效

**优先级**：🟢 Low（M13 可选）

---

### 10. io_uring 替换

**方案**：
- 替换 `spawn_blocking` + 同步 I/O
- 使用 `tokio-uring` 或 `io-uring` crate
- 真正的异步磁盘读写

**优先级**：🟢 Low（M13 可选，Linux 5.1+）

---

## 📋 优化优先级总结

| 优先级 | 问题 | 里程碑 | 说明 |
|--------|------|--------|------|
| 🔴 Critical | std::sync::Mutex 跨 await | M9 | 死锁风险 |
| 🔴 Critical | 零拷贝缺失（每次克隆 4KB） | M9 | 性能开销 |
| 🔴 Critical | BufferPool 持锁期间 I/O | M9 | 阻塞协程 |
| 🟡 Important | 二进制协议替换 | M10 | 嵌入式核心需求 |
| 🟡 Important | MVCC 完整版本链 | M10 | 事务完整性 |
| 🟡 Important | WAL 后台协程 | M11 | 可靠性 |
| 🟢 Low | 大查询并行化 | M13 | 性能优化 |
| 🟢 Low | 锁精简化 | M13 | 性能优化 |
| 🟢 Low | 语义缓存 | M13 | 扩展功能 |
| 🟢 Low | io_uring | M13 | 可选优化 |

---

## ⚠️ 常见陷阱提醒

```
❌ 绝对不要在 .await 期间持有 std::sync::Mutex（会死锁整个工作线程）
❌ CPU 密集操作（排序、哈希）必须用 spawn_blocking 隔离
❌ 页缓存淘汰不要用复杂锁竞争（LRU-simple 足够）
❌ 目前先不要引入 unsafe 做零拷贝（等火焰图确认瓶颈）
✅ 先改改动小、收益大的（异步页缓存 + 零拷贝）
```

---

## 已完成的优化（M1-M8）

### M1-M7 已解决的问题

- [x] **网络层实现**（M6）：Protocol trait + Server + Graceful shutdown
- [x] **数据存储层**（M7）：TableManager + Tuple 序列化 + Executor 真实执行
- [x] **MVCC 基础**（M7）：Snapshot 可见性 + VersionHeader（单版本）
- [x] **PostgreSQL 协议**（M8）：Simple Query Protocol + PgProtocol 状态机

### M1-M2 已解决的技术债务

- [x] **ScanExecutor NotImplemented**（M7）：已实现全表扫描
- [x] **RowId 测试占位**（M7）：已实现真实 RowId 分配
- [x] **事务未整合**（M7）：Executor 已持有 Transaction
- [x] **仅索引层执行**（M7）：已实现数据存储层

---

## 下一步行动

**M9 优化重点**（Critical Issues）：
1. ✅ 修复 `PageGuard` std::sync::Mutex → tokio::sync::Mutex
2. ✅ 实现 `page()` 零拷贝（返回引用而非克隆）
3. ✅ 修复 `BufferPool` 持锁期间 I/O → 异步等待

**M10 优化重点**（Important Issues）：
1. ✅ 实现二进制协议（format_code=1）- **嵌入式核心需求**
2. ✅ 完整版本链遍历 + GC

**M11 优化重点**：
1. ✅ WAL 后台协程 + Checkpoint