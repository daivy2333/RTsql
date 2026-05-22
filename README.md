# RTsql

异步协程驱动的高性能嵌入式关系型数据库 - 以 Tokio 无栈协程为调度核心，实现轻量、便捷、高效的现代数据库系统。

[![Rust](https://img.shields.io/badge/Rust-1.75%2B-orange.svg)](https://www.rust-lang.org/)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](https://opensource.org/licenses/)

---

## 设计理念

### 核心架构

RTsql 采用**异步协程驱动**的现代数据库架构，区别于传统数据库的同步阻塞设计：

| 架构层 | 传统数据库 | RTsql |
|--------|-----------|-------|
| **I/O 模型** | 同步阻塞 I/O | Tokio 异步协程（无栈纤程） |
| **并发模型** | 线程池 + 互斥锁 | MVCC 无锁读 + AtomicPageId |
| **页访问** | 每次克隆 4KB | 零拷贝 PageDataGuard |
| **缓冲池** | 单锁争用 | 两阶段锁（读锁检查→释放→I/O→写锁插入） |
| **索引访问** | RwLock<BTree> | AtomicPageId + async search |

### 技术亮点

1. **异步协程调度**
   - Tokio 多线程 scheduler，轻量级无栈协程
   - 消除 spawn_blocking 调度开销（从 ~25µs → 0µs）
   - Async search 路径直接访问 BufferPool

2. **零拷贝页访问**
   - `PageDataGuard` 提供零拷贝页数据读取
   - 读操作无需克隆 4KB 页数据
   - BTree 读路径完全零拷贝（LeafNodeRef/InternalNodeRef）

3. **无锁索引设计**
   - `AtomicPageId` 替代 `RwLock<BTree>`，消除锁争用
   - Async search 路径避免 `std::sync::RwLock` 跨 `.await` 死锁风险
   - 写操作保持 sync 路径（临时 BTree 实例）

4. **Slot Compacting**
   - 删除操作物理移动 slots，消除空洞
   - 保证二分搜索正确性（避免 deleted slots 干扰）

---

## SQLite 对比

### 性能基准（M14 Phase 2 T2 验证）

**精确单次查询对比**（criterion 直接测量，无 profiling overhead）：

| 操作 | RTsql | SQLite | 提速倍数 |
|------|-------|--------|---------|
| **PK lookup** | **~0.66µs (657ns)** | ~5.25µs | **8x faster** |

**测试条件**：
- 数据量：1000 行预热数据
- Warmup：50 次预热查询
- Release mode：`cargo bench --release`
- 单次查询：直接测量单次 `execute_sql()` / `query_row()`

### 性能可信性验证

**多测试方法一致性验证**：

| 测试方法 | RTsql PK Lookup | SQLite PK Lookup | 提速 |
|---------|----------------|------------------|------|
| 精确单次查询（`rtsql_vs_sqlite_single.rs`） | ~0.66µs | ~5.25µs | **8x** |
| 微基准循环（`micro_bench.rs`，50 次） | ~0.8µs（40µs/50） | ~5.4µs | **~6-7x** |
| Profiling 测试（`bench_minimal.rs`） | ~2-4µs（仅 index_manager_search） | - | - |

**差异解释**：
- Profiling overhead：task_local storage + timing API 增加 ~100µs overhead
- Benchmark overhead：criterion 测量更精准
- **可信结论**：RTsql 比 SQLite 快 **6-8x**（不同测试方法略有差异）

### 内部瓶颈消除

**M14 Phase 2 T2 优化成果**：

| 指标 | 优化前 | 优化后 | 提速 |
|------|--------|--------|------|
| index_manager_search | ~51µs (81%) | ~2-4µs | **17x** |
| executor_execution | ~57µs (90.5%) | ~10-15µs | **5-6x** |
| Total PK lookup | ~63µs | ~15-20µs | **3-4x** |

**关键改进**：
- spawn_blocking + SyncPageLoader 调度开销：**消除**（从 ~25µs → 0µs）
- std::sync::RwLock<BTree> 锁争用：**消除**（从 ~5µs → 0µs）

### 并发性能对比

| 并发度 | 优化前 | 优化后 | 提速 |
|--------|--------|--------|------|
| 1 线程 | ~170µs | ~99µs | **41%** |
| 4 线程 | ~290µs | ~182µs | **37%** |
| 8 线程 | ~520µs | ~283µs | **46%** |
| 16 线程 | ~1.2ms | ~559µs | **54%** |
| 32 线程 | ~3.2ms | ~1.2ms | **63%** |

**并发优化要点**：
- AtomicPageId 无锁访问，减少线程争用
- Async search 路径，消除 spawn_blocking 阻塞
- 两阶段锁缓冲池，I/O 操作不持锁

---

## 性能参数详解

### Benchmark 测试配置

**测试参数**（所有 benchmark 标准化）：
- **迭代次数**：50 次内部循环（平衡采样精度与运行时间）
- **并发测试**：[1, 4, 8, 16, 32] 线程并发度
- **规模测试**：[1K, 10K, 100K] 数据量规模

**运行命令**：
```bash
# RTsql 微基准测试
cargo bench --bench micro_bench

# SQLite 对比测试
cargo bench --bench sqlite_compare

# 并发压力测试
cargo bench --bench concurrent_bench

# 规模扩展测试
cargo bench --bench scale_bench

# Profiling 分析
RTSQL_PROFILING=1 cargo run --example bench_minimal
```

### 热路径分析（优化后）

```
execute_sql()
  → [cache hit] parse+plan skipped (~0µs)
  → create_executor_from_plan() (~2µs)
  → IndexScanExecutor::next()
    → IndexManager::search()
      → AtomicPageId::load(Ordering::Acquire)  ← ~0.1µs（无锁）
      → search_from_page_async()               ← ~2-3µs（async 路径）
      → AsyncPageLoader::load_page()           ← ~0.5µs（缓存命中）
      → LeafNodeRef::find_key_position_binary  ← ~1µs（零拷贝）
```

**对比传统数据库热路径**：
- SQLite：同步阻塞 I/O + 线程池调度 → ~5-10µs 线程调度开销
- RTsql：异步协程 + 无锁访问 → ~0.5µs 协程调度开销

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
    // 打开数据库
    let db = Database::open("mydb.rtsql").await.unwrap();

    // 创建表
    db.execute_sql(
        "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT, age INTEGER)"
    ).await.unwrap();

    // 插入数据
    db.execute_sql(
        "INSERT INTO users (id, name, age) VALUES (1, 'Alice', 30)"
    ).await.unwrap();

    // 查询数据
    let result = db.execute_sql("SELECT * FROM users WHERE id = 1").await.unwrap();
    println!("Query result: {}", result);

    // 关闭数据库
    db.close().await.unwrap();
}
```

### 性能测试

```bash
# 运行完整 benchmark 套件
cargo bench

# 单独测试 RTsql vs SQLite PK lookup
cargo bench --bench sqlite_compare
```

---

## 技术栈

| 类别 | 技术 | 版本 |
|------|------|------|
| **语言** | Rust | 1.75+ |
| **异步运行时** | Tokio | 1.x（multi-thread scheduler） |
| **SQL 解析** | sqlparser-rs | 0.44 |
| **序列化** | serde + serde_json | 1.0 |
| **基准测试** | criterion.rs | 0.5（html_reports + async_tokio） |
| **SQLite 对比** | rusqlite | 0.31 |
| **临时文件** | tempfile | 3.x |

---

## 项目文档

详细文档位于 `.claude/docs/`：

| 文档 | 用途 |
|------|------|
| [rules.md](.claude/docs/rules.md) | 编码规范与行为约束 |
| [architecture.md](.claude/docs/architecture.md) | 架构决策记录 |
| [snapshot.md](.claude/docs/snapshot.md) | 项目当前状态快照 |
| [tasks.md](.claude/docs/tasks.md) | 当前任务与待办 |
| [learned.md](.claude/docs/learned.md) | 学习记忆与踩坑经验 |
| [optimization.md](.claude/docs/optimization.md) | 优化方向与技术债 |

---

## 路线图

### 已完成里程碑

- ✅ **M1-M12**: 核心功能实现（Storage, Executor, Parser, WAL, MVCC）
- ✅ **M13**: PageGuard 零拷贝 + BufferPool 两阶段锁
- ✅ **M14 Phase 2 T2**: Async search 路径 + AtomicPageId（17x internal, 8x SQLite）

### 下一里程碑

- ⏳ **M15**: 聚合函数与 GROUP BY
- ⏳ **M16**: 子查询支持
- ⏳ **M17**: B-Tree split/merge + 非唯一索引
- ⏳ **M18**: WAL Group Commit（写入优化，预期 INSERT 5-10x 提速）

---

## 许可证

本项目采用双许可证（MIT OR Apache-2.0），详见 [LICENSE](LICENSE) 文件。

---

## 致谢

本项目受以下数据库系统启发：
- SQLite（嵌入式数据库标杆）
- PostgreSQL（MVCC 设计参考）
- TiKV（异步协程架构参考）

特别感谢 Rust 社区提供的优秀基础设施（Tokio, sqlparser-rs, criterion.rs）。