# RTsql

异步协程驱动的高性能嵌入式关系型数据库 — 以 Tokio 无栈协程为调度核心，实现轻量、便捷、高效的现代数据库系统。

[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](https://opensource.org/licenses/)
[![Tests](https://img.shields.io/badge/tests-~430%20pass-brightgreen.svg)]()

---

## 功能特性

| 类别 | 支持 |
|------|------|
| **SQL DML** | SELECT, INSERT, UPDATE, DELETE |
| **SQL DDL** | CREATE TABLE, DROP TABLE |
| **查询** | WHERE, JOIN (INNER), GROUP BY, HAVING, ORDER BY, LIMIT/OFFSET |
| **子查询** | WHERE IN/EXISTS (SemiJoin/AntiJoin), SELECT 标量 (SubqueryEval), FROM 派生表 (DerivedScan), 相关子查询 |
| **索引** | B-Tree 主键索引, 非唯一索引, split/merge 自动维护 |
| **事务** | MVCC 无锁读, 行级锁, 版本链 |
| **持久化** | WAL (Write-Ahead Logging) + Group Commit + 崩溃恢复 |
| **API** | `open`, `execute_sql`, `close` — 简洁嵌入式接口 |

---

## 设计理念

### 核心架构

RTsql 采用异步协程驱动的现代数据库架构：

| 架构层 | 传统数据库 | RTsql |
|--------|-----------|-------|
| I/O 模型 | 同步阻塞 I/O | Tokio 异步协程 |
| 并发模型 | 线程池 + 互斥锁 | MVCC 无锁读 + AtomicPageId |
| 页访问 | 每次克隆 4KB | 零拷贝 PageDataGuard |
| 缓冲池 | 单锁争用 | 两阶段锁（读锁检查→释放→I/O→写锁插入） |
| 索引访问 | RwLock\<BTree\> | AtomicPageId + async search |
| 索引维护 | 手动/无 | B-Tree split/merge 自动 |
| 崩溃恢复 | WAL | WAL + Group Commit + CRC32 |

### 技术亮点

1. **异步协程调度**
   - Tokio 多线程 scheduler，轻量级无栈协程
   - 消除 spawn_blocking 调度开销（~25µs → 0µs）
   - Async search 路径直接访问 BufferPool

2. **零拷贝页访问**
   - `PageDataGuard` 零拷贝读取，读操作无需克隆 4KB
   - BTree 读路径完全零拷贝（LeafNodeRef/InternalNodeRef）

3. **无锁索引设计**
   - `AtomicPageId` 替代 `RwLock<BTree>`，消除锁争用
   - 写操作保持 sync 路径（临时 BTree 实例）

4. **Redistribution-First B-Tree Merge**
   - 删除后先尝试从兄弟节点借 entries，不足时才合并
   - 递归 merge 传播 + root shrink，保持树结构合法
   - FileStorage free-list 复用释放的页

5. **WAL + Group Commit**
   - 写入操作批量刷盘，减少 fsync 开销
   - CRC32 校验 + LSN 序列号，保证数据完整性
   - RecoveryManager 崩溃恢复（redo committed + mark uncommitted）

---

## SQLite 全方位对比

### 速度对比

| 操作 | RTsql | SQLite | 对比 |
|------|-------|--------|------|
| **INSERT 100 rows** | 693µs | 232ms | **332x faster** ⚡ |
| **PK Lookup（单次）** | 0.66µs | 5.25µs | **8x faster** ⚡ |
| **PK Lookup（1000次）** | ~15µs/次 | ~20µs/次 | **1.3x faster** |
| **Full Scan 1K rows** | 327µs | 80µs | 4x slower |
| **DELETE 500 rows** | merge 自动维护 | 手动 VACUUM | 功能完整 |

**测试条件**：Release mode, 1000 行预热数据, criterion 精确测量。详细数据见 `.claude/docs/optimization.md`。

### 写入性能优势分析

RTsql INSERT 332x faster 的核心原因：
- **异步 I/O**：非阻塞写入，批量 Group Commit
- **MVCC 无锁写**：无需获取表级写锁
- **两阶段锁缓冲池**：I/O 期间不持锁，其他操作不受阻

### 并发性能

| 并发度 | RTsql（优化后） | 优化前 | 提升 |
|--------|----------------|--------|------|
| 1 线程 | ~99µs | ~170µs | 41% |
| 4 线程 | ~182µs | ~290µs | 37% |
| 8 线程 | ~283µs | ~520µs | 46% |
| 16 线程 | ~559µs | ~1.2ms | 54% |
| 32 线程 | ~1.2ms | ~3.2ms | 63% |

高并发场景下 RTsql 优势更明显，得益于 AtomicPageId 无锁读 + async search 路径。

### 资源消耗对比

| 维度 | RTsql | SQLite | 差异 |
|------|-------|--------|------|
| **数据文件（10K rows）** | 1.4 MB | 217 KB | 6.5x larger |
| **二进制大小** | 3.7 MB | 1.6 MB | 2.3x larger |
| **内存（启动）** | ~5 MB | ~1 MB | Tokio runtime 开销 |
| **测试覆盖** | ~430 tests | — | 全面 |

#### 文件大小差异分析

| 因素 | RTsql 开销 | 设计权衡 |
|------|-----------|---------|
| 两层分离索引（索引页+数据页） | ~3x | 灵活性：多索引、非唯一索引 |
| 固定 Key 32 bytes（vs SQLite varint） | ~10x per key | 简化实现、CPU 友好 |
| SlottedPage 页填充率 50-70% | ~1.4x | MVCC 版本链友好 |
| Tag byte 序列化 | ~1.2x | 类型安全 |

**总体权衡**：RTsql 选择**实现简洁性 + 架构灵活性**换取空间效率。后续可通过 varint 编码、页填充优化进一步缩小差距。

---

## 快速开始

### 安装

```bash
git clone https://github.com/daivy2333/RTsql.git
cd RTsql
cargo build --release
```

### 基本用法

```rust
use rtsql::Database;

#[tokio::main]
async fn main() {
    let db = Database::open("mydb.rtsql").await.unwrap();

    db.execute_sql("CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, age INTEGER)").await.unwrap();
    db.execute_sql("INSERT INTO users VALUES (1, 'Alice', 30)").await.unwrap();

    let result = db.execute_sql("SELECT * FROM users WHERE id = 1").await.unwrap();
    println!("{}", result);

    db.close().await.unwrap();
}
```

### 运行测试

```bash
cargo test                           # ~430 tests
cargo bench                          # 完整 benchmark 套件
cargo bench --bench sqlite_compare   # SQLite 对比
```

---

## 技术栈

| 类别 | 技术 | 版本 |
|------|------|------|
| 语言 | Rust | 1.75+ |
| 异步运行时 | Tokio | 1.x (multi-thread) |
| SQL 解析 | sqlparser-rs | 0.44 |
| 基准测试 | criterion.rs | 0.5 |
| SQLite 对比 | rusqlite | 0.31 |

---

## 架构概览

```
SQL Text → Parser (sqlparser) → PlanBuilder → PhysicalPlan (19 节点)
  → Pipeline → Volcano Executor Tree
    → Scan/Filter/Join/Aggregate/Having/Sort/Limit
    → SemiJoin/AntiJoin/SubqueryEval/DerivedScan
    → Insert/Update/Delete
  → Storage (BufferPool + BTree + SlottedPage + WAL)
```

完整架构决策记录见 `.claude/docs/architecture.md`（8 个 ADR）。

---

## 项目文档

| 文档 | 用途 |
|------|------|
| [architecture.md](.claude/docs/architecture.md) | 8 个架构决策记录 |
| [snapshot.md](.claude/docs/snapshot.md) | 项目状态快照 |
| [learned.md](.claude/docs/learned.md) | API 速查 + 踩坑经验 |
| [optimization.md](.claude/docs/optimization.md) | 性能基准 + 优化方向 |
| [archive.md](.claude/docs/archive.md) | 已归档历史条目 |

---

## 里程碑路线图

| 里程碑 | 内容 | 状态 |
|--------|------|------|
| M1-M12 | 核心 Storage/Executor/Parser/WAL/MVCC | ✅ |
| M13 | PageGuard 零拷贝 + BufferPool 两阶段锁 | ✅ |
| M14 | Async search + AtomicPageId 无锁读 | ✅ |
| M15 | 聚合函数 + GROUP BY + HAVING | ✅ |
| M16 | 子查询（独立+相关）+ 派生表 | ✅ |
| M17-Phase1 | 非唯一索引 + 批量删除 | ✅ |
| M17-Phase2 | B-Tree Split 机制 | ✅ |
| M17.5 | 代码清理 + SQLite 全面对比 | ✅ |
| M18-Phase1 | 架构 Warnings 清理 | ✅ |
| M18-Phase2 | Executor 层非唯一索引 | ✅ |
| M18-Phase3 | WAL + Group Commit + 崩溃恢复 | ✅ |
| **M18-Phase4** | **B-Tree Merge + free-list 页复用** | **✅** |

---

## 许可证

MIT OR Apache-2.0

## 致谢

受 SQLite（嵌入式标杆）、PostgreSQL（MVCC）、TiKV（异步协程）启发。
感谢 Rust 社区（Tokio, sqlparser-rs, criterion.rs）。
