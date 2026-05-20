# M0 里程碑设计文档：项目骨架与 Tokio 运行时

> 创建日期：2026-05-20
> 状态：Approved

---

## 概述

M0 里程碑目标：初始化 RTsql 项目骨架，引入 Tokio 异步运行时，验证运行时配置正确。

---

## 项目目录结构

```
RTsql/
├── Cargo.toml              # Rust 项目配置，含 Tokio 依赖
├── src/
│   ├── main.rs             # 数据库服务器入口
│   ├── lib.rs              # 库入口，导出模块公共接口
│   ├── storage/
│   │   └── mod.rs          # 存储引擎模块（占位符）
│   ├── executor/
│   │   └── mod.rs          # 执行引擎模块（占位符）
│   ├── transaction/
│   │   └── mod.rs          # 事务管理模块（占位符）
│   ├── parser/
│   │   └── mod.rs          # SQL 解析模块（占位符）
│   └── network/
│   │   └── mod.rs          # 网络层模块（占位符）
└── tests/
    └── runtime_test.rs     # 运行时功能验证测试
```

---

## Cargo.toml 配置

```toml
[package]
name = "rtsql"
version = "0.1.0"
edition = "2021"
description = "Async coroutine-driven embedded relational database"
license = "MIT OR Apache-2.0"

[dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync", "time", "net", "fs"] }

[dev-dependencies]
# 测试相关依赖暂无，后续里程碑添加

[profile.release]
opt-level = 3
lto = true

[profile.dev]
opt-level = 0
```

**Tokio features 选择性启用**：
- `rt-multi-thread`: 多线程调度器
- `macros`: `#[tokio::main]` 和 `#[tokio::test]`
- `sync`: 异步锁和通道（M3 使用）
- `time`: 异步定时器
- `net`: TCP/UDP 异步网络（M6 使用）
- `fs`: 异步文件操作（M1 使用）

---

## main.rs 入口

```rust
//! RTsql - Async coroutine-driven embedded relational database

use rtsql::storage;
use rtsql::executor;
use rtsql::transaction;
use rtsql::parser;
use rtsql::network;

#[tokio::main]
async fn main() {
    println!("RTsql database server starting...");

    // TODO: Initialize storage engine (M1)
    // TODO: Initialize execution engine (M5)
    // TODO: Start network server (M6)

    println!("RTsql ready. (M0 skeleton completed)");
}
```

---

## lib.rs 库入口

```rust
//! RTsql library - Async embedded database components

pub mod storage;
pub mod executor;
pub mod transaction;
pub mod parser;
pub mod network;
```

---

## 各模块 mod.rs（占位符）

**storage/mod.rs**:
```rust
//! Storage engine - Buffer pool, page cache, file I/O
//!
//! M1: Implement AsyncStorage trait and buffer pool
```

**executor/mod.rs**:
```rust
//! Execution engine - Physical plan execution, async iterator
//!
//! M5: Implement async fn next() -> Result<Option<Row>>
```

**transaction/mod.rs**:
```rust
//! Transaction management - MVCC, concurrency control
//!
//! M3: Implement transaction ID allocation and MVCC snapshot read
```

**parser/mod.rs**:
```rust
//! SQL parser - Parse SQL to internal representation
//!
//! M4: Integrate sqlparser-rs
```

**network/mod.rs**:
```rust
//! Network layer - TCP server, protocol implementation
//!
//! M6: Implement tokio::net::TcpListener and connection handling
```

---

## 运行时验证测试

```rust
//! Runtime functionality test
//!
//! Verify Tokio runtime is properly configured:
//! - async function execution
//! - tokio::spawn task scheduling
//! - multi-thread scheduler behavior

#[tokio::test]
async fn test_async_execution() {
    let result = async_compute(42).await;
    assert_eq!(result, 84);
}

async fn async_compute(n: u32) -> u32 {
    n * 2
}

#[tokio::test]
async fn test_spawn_task() {
    let handle = tokio::spawn(async {
        100
    });

    let result = handle.await.expect("task should complete");
    assert_eq!(result, 100);
}

#[tokio::test]
async fn test_multi_thread_spawn() {
    use std::sync::Arc;
    use tokio::sync::Mutex;

    let counter = Arc::new(Mutex::new(0));
    let mut handles = vec![];

    for _ in 0..10 {
        let counter_clone = counter.clone();
        handles.push(tokio::spawn(async move {
            let mut guard = counter_clone.lock().await;
            *guard += 1;
        }));
    }

    for handle in handles {
        handle.await.expect("task should complete");
    }

    let final_count = *counter.lock().await;
    assert_eq!(final_count, 10);
}
```

---

## Git 初始化

`.gitignore`:
```
/target
/Cargo.lock
**/*.rs.bk
*.pdb
```

初始化提交：
```bash
git init
git add Cargo.toml src/ tests/ .gitignore CLAUDE.md .claude/
git commit -m "M0: Initialize project skeleton with Tokio runtime"
```

---

## 成功标准

M0 完成需满足：
1. `cargo build` 编译成功（无 error/warning）
2. `cargo test` 三个运行时测试全部通过
3. `cargo clippy` 无 warning
4. Git 仓库初始化完成，首次提交已创建
5. 项目目录结构符合设计

---

## 边界情况

M0 无实际功能实现，错误处理在后续里程碑定义。

M0 的边界情况：
- Cargo.toml Tokio features 缺失 → 编译错误，需补充
- 模块目录未创建 → lib.rs `mod` 声明失败，需创建目录
- 测试未标记 `#[tokio::test]` → 运行时未初始化，测试失败

---

## 设计决策

| 决策 | 选择 | 原因 |
|------|------|------|
| 程序入口 | 数据库服务器 | 架构定义的目标 |
| 模块组织 | lib.rs 导出公共接口 | 保持内部实现私有，便于测试和扩展 |
| Tokio 配置 | 多线程 scheduler | 自动 worker threads，适合生产数据库 |
| M0 测试内容 | 运行时功能验证 | 验证 async 执行、spawn 调度、运行时初始化 |