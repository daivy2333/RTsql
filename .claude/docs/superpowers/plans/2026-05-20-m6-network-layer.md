# M6 网络层实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 TCP 服务器 + JSON 协议 + 端到端测试，验证网络层与 M4/M5 集成

**Architecture:** Protocol trait 抽象 + JsonProtocol 实现 + ConnectionHandler 每连接一协程 + SqlHandler mock executor

**Tech Stack:** Tokio (TcpListener/spawn), tokio-util (CancellationToken), serde/serde_json, async_trait

---

## 前置依赖

**设计规范**: `.claude/docs/superpowers/specs/2026-05-20-m6-network-layer-design.md`

**相关里程碑**: M4 (Parser), M5 (Executor)

**新增依赖**:
```toml
tokio-util = { version = "0.7", features = ["sync"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

---

## Task 1: 添加依赖

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: 添加 tokio-util, serde, serde_json 到 Cargo.toml**

打开 `Cargo.toml`，在 `[dependencies]` 部分添加：

```toml
tokio-util = { version = "0.7", features = ["sync"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

- [ ] **Step 2: 运行 cargo build 验证依赖下载**

```bash
cargo build
```

Expected: 成功下载依赖，无编译错误

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "feat(m6): add tokio-util, serde, serde_json dependencies"
```

---

## Task 2: NetworkError 错误类型

**Files:**
- Create: `src/network/mod.rs`
- Create: `src/network/error.rs`
- Modify: `src/lib.rs`

- [ ] **Step 1: 创建 src/network/mod.rs 模块导出**

```rust
pub mod error;
pub mod protocol;
pub mod connection;
pub mod handler;
pub mod server;

pub use error::NetworkError;
pub use protocol::{Protocol, JsonProtocol, Request, Response};
pub use connection::ConnectionHandler;
pub use handler::SqlHandler;
pub use server::Server;
```

- [ ] **Step 2: 创建 src/network/error.rs NetworkError enum**

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Protocol parse error: {0}")]
    ProtocolParse(String),

    #[error("Protocol write error: {0}")]
    ProtocolWrite(String),

    #[error("SQL parse error: {0}")]
    SqlParse(String),

    #[error("Execution error: {0}")]
    Execution(String),
}
```

- [ ] **Step 3: 在 src/lib.rs 中导出 network 模块**

打开 `src/lib.rs`，在现有模块导出后添加：

```rust
pub mod network;
```

- [ ] **Step 4: 运行 cargo build 验证编译**

```bash
cargo build
```

Expected: 成功编译，无错误

- [ ] **Step 5: Commit**

```bash
git add src/network/mod.rs src/network/error.rs src/lib.rs
git commit -m "feat(m6): add NetworkError and network module skeleton"
```

---

## Task 3: Protocol trait + Request/Response

**Files:**
- Create: `src/network/protocol.rs`

- [ ] **Step 1: 在 protocol.rs 中定义 Protocol trait + Request/Response enum**

```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::network::error::NetworkError;

/// 协议抽象 trait
#[async_trait]
pub trait Protocol: Send + Sync {
    async fn parse_request(&mut self, stream: &mut TcpStream) -> Result<Option<Request>, NetworkError>;
    async fn write_response(&mut self, stream: &mut TcpStream, response: &Response) -> Result<(), NetworkError>;
}

/// 请求类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    Query { sql: String },
    Insert { sql: String },
    Update { sql: String },
    Delete { sql: String },
    Ping,
}

/// 响应类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    QueryResult { row_ids: Vec<(u64, u16)> },
    AffectedRows { count: u64 },
    Error { message: String },
    Pong,
}

impl Request {
    pub fn sql(&self) -> Option<&str> {
        match self {
            Request::Query { sql } => Some(sql),
            Request::Insert { sql } => Some(sql),
            Request::Update { sql } => Some(sql),
            Request::Delete { sql } => Some(sql),
            Request::Ping => None,
        }
    }
}
```

- [ ] **Step 2: 运行 cargo build 验证编译**

```bash
cargo build
```

Expected: 成功编译，无错误

- [ ] **Step 3: Commit**

```bash
git add src/network/protocol.rs
git commit -m "feat(m6): define Protocol trait and Request/Response enums"
```

---

## Task 4: JsonProtocol 实现

**Files:**
- Modify: `src/network/protocol.rs`

- [ ] **Step 1: 在 protocol.rs 中添加 JsonProtocol 结构体**

在 `protocol.rs` 文件末尾添加：

```rust
/// JSON 协议实现
pub struct JsonProtocol {
    buffer: Vec<u8>,
}

impl JsonProtocol {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(4096),
        }
    }
}

impl Default for JsonProtocol {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 2: 实现 Protocol trait for JsonProtocol**

继续在 `protocol.rs` 添加：

```rust
#[async_trait]
impl Protocol for JsonProtocol {
    async fn parse_request(&mut self, stream: &mut TcpStream) -> Result<Option<Request>, NetworkError> {
        self.buffer.clear();

        // 读取直到遇到换行符
        let mut byte = [0u8; 1];
        loop {
            match stream.read(&mut byte).await {
                Ok(0) => return Ok(None), // 连接关闭
                Ok(_) => {
                    if byte[0] == b'\n' {
                        break;
                    }
                    self.buffer.push(byte[0]);
                }
                Err(e) => return Err(NetworkError::Io(e)),
            }
        }

        // 解析 JSON
        let request: Request = serde_json::from_slice(&self.buffer)
            .map_err(|e| NetworkError::ProtocolParse(e.to_string()))?;

        Ok(Some(request))
    }

    async fn write_response(&mut self, stream: &mut TcpStream, response: &Response) -> Result<(), NetworkError> {
        let json = serde_json::to_vec(response)
            .map_err(|e| NetworkError::ProtocolWrite(e.to_string()))?;

        stream.write_all(&json).await?;
        stream.write_all(&[b'\n']).await?;
        stream.flush().await?;

        Ok(())
    }
}
```

- [ ] **Step 3: 运行 cargo build 验证编译**

```bash
cargo build
```

Expected: 成功编译，无错误

- [ ] **Step 4: Commit**

```bash
git add src/network/protocol.rs
git commit -m "feat(m6): implement JsonProtocol with newline-delimited framing"
```

---

## Task 5: JsonProtocol 单元测试

**Files:**
- Create: `tests/network/protocol_test.rs`

- [ ] **Step 1: 创建 tests/network 目录**

```bash
mkdir -p tests/network
```

- [ ] **Step 2: 写 protocol_test.rs 测试 Request 序列化/反序列化**

```rust
use rtsql::network::{Request, Response, JsonProtocol};

#[test]
fn test_request_serialize_query() {
    let req = Request::Query { sql: "SELECT * FROM users".to_string() };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("Query"));
    assert!(json.contains("SELECT * FROM users"));
}

#[test]
fn test_request_deserialize_insert() {
    let json = r#"{"Insert":{"sql":"INSERT INTO users VALUES (1)"}}"#;
    let req: Request = serde_json::from_str(json).unwrap();
    assert_eq!(req.sql(), Some("INSERT INTO users VALUES (1)"));
}

#[test]
fn test_response_serialize_affected_rows() {
    let resp = Response::AffectedRows { count: 5 };
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("AffectedRows"));
    assert!(json.contains("5"));
}

#[test]
fn test_response_deserialize_error() {
    let json = r#"{"Error":{"message":"table not found"}}"#;
    let resp: Response = serde_json::from_str(json).unwrap();
    match resp {
        Response::Error { message } => assert_eq!(message, "table not found"),
        _ => panic!("Expected Error response"),
    }
}

#[test]
fn test_ping_pong_roundtrip() {
    let req = Request::Ping;
    let json = serde_json::to_string(&req).unwrap();
    let parsed: Request = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, Request::Ping);

    let resp = Response::Pong;
    let json = serde_json::to_string(&resp).unwrap();
    let parsed: Response = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, Response::Pong);
}
```

- [ ] **Step 3: 运行测试验证序列化正确性**

```bash
cargo test --test protocol_test
```

Expected: 5 tests passed

- [ ] **Step 4: Commit**

```bash
git add tests/network/protocol_test.rs
git commit -m "test(m6): add JsonProtocol serialization/deserialization tests"
```

---

## Task 6: SqlHandler 实现

**Files:**
- Create: `src/network/handler.rs`

- [ ] **Step 1: 创建 handler.rs SqlHandler 结构体**

```rust
use crate::network::protocol::{Request, Response};
use crate::executor::ExecResult;
use crate::network::error::NetworkError;

/// SQL 处理器（M6 简化：mock executor）
pub struct SqlHandler {
    // M6 无持久化状态
}

impl SqlHandler {
    pub fn new() -> Self {
        Self {}
    }

    pub fn execute(&mut self, request: Request) -> Response {
        match request {
            Request::Query { sql } => {
                // M6 mock: 返回固定的 RowId
                Response::QueryResult {
                    row_ids: vec![(0, 1)], // mock: page_id=0, slot_id=1
                }
            }
            Request::Insert { sql } => {
                // M6 mock: 返回固定的 AffectedRows
                Response::AffectedRows { count: 1 }
            }
            Request::Update { sql } => {
                Response::AffectedRows { count: 1 }
            }
            Request::Delete { sql } => {
                Response::AffectedRows { count: 1 }
            }
            Request::Ping => Response::Pong,
        }
    }
}

impl Default for SqlHandler {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 2: 运行 cargo build 验证编译**

```bash
cargo build
```

Expected: 成功编译

- [ ] **Step 3: Commit**

```bash
git add src/network/handler.rs
git commit -m "feat(m6): implement SqlHandler with mock executor"
```

---

## Task 7: ConnectionHandler 实现

**Files:**
- Create: `src/network/connection.rs`

- [ ] **Step 1: 创建 connection.rs ConnectionHandler 结构体**

```rust
use crate::network::protocol::{Protocol, Request, Response};
use crate::network::handler::SqlHandler;
use crate::network::error::NetworkError;
use tokio::net::TcpStream;

/// 连接处理器，每连接一协程
pub struct ConnectionHandler<P: Protocol> {
    protocol: P,
    handler: SqlHandler,
}

impl<P: Protocol> ConnectionHandler<P> {
    pub fn new(protocol: P, handler: SqlHandler) -> Self {
        Self { protocol, handler }
    }

    /// 处理连接生命周期
    pub async fn handle(&mut self, stream: TcpStream) -> Result<(), NetworkError> {
        let mut stream = stream;

        loop {
            // 1. 解析请求
            let request = self.protocol.parse_request(&mut stream).await?;

            match request {
                Some(req) => {
                    // 2. 执行 SQL
                    let response = self.handler.execute(req);

                    // 3. 写回响应
                    self.protocol.write_response(&mut stream, &response).await?;
                }
                None => {
                    // 连接关闭
                    break;
                }
            }
        }

        Ok(())
    }
}
```

- [ ] **Step 2: 运行 cargo build 验证编译**

```bash
cargo build
```

Expected: 成功编译

- [ ] **Step 3: Commit**

```bash
git add src/network/connection.rs
git commit -m "feat(m6): implement ConnectionHandler for per-connection coroutine"
```

---

## Task 8: Server 实现

**Files:**
- Create: `src/network/server.rs`

- [ ] **Step 1: 创建 server.rs Server 结构体**

```rust
use crate::network::connection::ConnectionHandler;
use crate::network::protocol::JsonProtocol;
use crate::network::handler::SqlHandler;
use crate::network::error::NetworkError;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use std::net::SocketAddr;

/// TCP 服务器
pub struct Server {
    addr: SocketAddr,
    shutdown: CancellationToken,
}

impl Server {
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            addr,
            shutdown: CancellationToken::new(),
        }
    }

    /// 获取 shutdown token（用于外部触发停止）
    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    /// 启动服务器
    pub async fn run(self) -> Result<(), NetworkError> {
        let listener = TcpListener::bind(self.addr).await?;
        println!("Server listening on {}", self.addr);

        loop {
            tokio::select! {
                // Accept 新连接
                result = listener.accept() => {
                    let (stream, peer_addr) = result?;

                    // Spawn 协程处理连接
                    let handler = ConnectionHandler::new(
                        JsonProtocol::new(),
                        SqlHandler::new(),
                    );

                    tokio::spawn(async move {
                        if let Err(e) = handler.handle(stream).await {
                            eprintln!("Connection error from {}: {}", peer_addr, e);
                        }
                    });
                }

                // Shutdown signal
                _ = self.shutdown.cancelled() => {
                    println!("Server shutting down");
                    break;
                }
            }
        }

        Ok(())
    }
}
```

- [ ] **Step 2: 运行 cargo build 验证编译**

```bash
cargo build
```

Expected: 成功编译

- [ ] **Step 3: Commit**

```bash
git add src/network/server.rs
git commit -m "feat(m6): implement Server with TcpListener and graceful shutdown"
```

---

## Task 9: Server 集成测试

**Files:**
- Create: `tests/network/server_test.rs`

- [ ] **Step 1: 写 server_test.rs 测试 Server 启动 + 客户端连接**

```rust
use rtsql::network::{Server, JsonProtocol, Request, Response};
use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;
use std::net::SocketAddr;

async fn start_test_server(port: u16) -> CancellationToken {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let server = Server::new(addr);
    let shutdown = server.shutdown_token();

    tokio::spawn(async move {
        server.run().await.unwrap();
    });

    // 等待服务器启动
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    shutdown
}

async fn send_request(port: u16, request: &Request) -> Response {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect(addr).await.unwrap();

    // 发送 JSON 请求（带换行符）
    let json = serde_json::to_string(request).unwrap();
    stream.write_all(json.as_bytes()).await.unwrap();
    stream.write_all(&[b'\n']).await.unwrap();

    // 读取响应
    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read(&mut byte).await.unwrap();
        if byte[0] == b'\n' {
            break;
        }
        buffer.push(byte[0]);
    }

    let response: Response = serde_json::from_slice(&buffer).unwrap();
    response
}

#[tokio::test]
async fn test_server_ping_pong() {
    let shutdown = start_test_server(9001).await;

    let response = send_request(9001, &Request::Ping).await;
    assert_eq!(response, Response::Pong);

    shutdown.cancel();
}

#[tokio::test]
async fn test_server_query_flow() {
    let shutdown = start_test_server(9002).await;

    let request = Request::Query {
        sql: "SELECT * FROM users".to_string(),
    };
    let response = send_request(9002, &request).await;

    match response {
        Response::QueryResult { row_ids } => {
            // M6 mock: 返回固定的 RowId
            assert_eq!(row_ids, vec![(0, 1)]);
        }
        _ => panic!("Expected QueryResult"),
    }

    shutdown.cancel();
}

#[tokio::test]
async fn test_server_insert_flow() {
    let shutdown = start_test_server(9003).await;

    let request = Request::Insert {
        sql: "INSERT INTO users VALUES (1, 'Alice')".to_string(),
    };
    let response = send_request(9003, &request).await;

    match response {
        Response::AffectedRows { count } => {
            // M6 mock: 返回固定的 AffectedRows
            assert_eq!(count, 1);
        }
        _ => panic!("Expected AffectedRows"),
    }

    shutdown.cancel();
}

#[tokio::test]
async fn test_server_multiple_requests() {
    let shutdown = start_test_server(9004).await;

    // 连接保持打开，发送多个请求
    let addr = SocketAddr::from(([127, 0, 0, 1], 9004));
    let mut stream = TcpStream::connect(addr).await.unwrap();

    // 发送 Ping
    let json = serde_json::to_string(&Request::Ping).unwrap();
    stream.write_all(json.as_bytes()).await.unwrap();
    stream.write_all(&[b'\n']).await.unwrap();

    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        stream.read(&mut byte).await.unwrap();
        if byte[0] == b'\n' {
            break;
        }
        buffer.push(byte[0]);
    }
    let resp1: Response = serde_json::from_slice(&buffer).unwrap();
    assert_eq!(resp1, Response::Pong);

    // 发送 Query
    buffer.clear();
    let json = serde_json::to_string(&Request::Query { sql: "SELECT 1".to_string() }).unwrap();
    stream.write_all(json.as_bytes()).await.unwrap();
    stream.write_all(&[b'\n']).await.unwrap();

    loop {
        stream.read(&mut byte).await.unwrap();
        if byte[0] == b'\n' {
            break;
        }
        buffer.push(byte[0]);
    }
    let resp2: Response = serde_json::from_slice(&buffer).unwrap();
    match resp2 {
        Response::QueryResult { row_ids } => assert_eq!(row_ids, vec![(0, 1)]),
        _ => panic!("Expected QueryResult"),
    }

    shutdown.cancel();
}
```

- [ ] **Step 2: 运行测试验证 Server 正常工作**

```bash
cargo test --test server_test
```

Expected: 4 tests passed

- [ ] **Step 3: Commit**

```bash
git add tests/network/server_test.rs
git commit -m "test(m6): add Server integration tests for query/insert/ping flows"
```

---

## Task 10: 运行全部测试 + Clippy + Fmt

**Files:**
- None (验证阶段)

- [ ] **Step 1: 运行 cargo test 验证所有测试通过**

```bash
cargo test
```

Expected: All tests passed (including existing M0-M5 tests)

- [ ] **Step 2: 运行 cargo clippy 检查代码质量**

```bash
cargo clippy
```

Expected: No errors, warnings acceptable

- [ ] **Step 3: 运行 cargo fmt 格式化代码**

```bash
cargo fmt
```

Expected: 格式化完成

- [ ] **Step 4: 最终 Commit（如果有格式化变更）**

```bash
git add -A
git commit -m "style(m6): apply cargo fmt formatting"
```

---

## Task 11: 更新项目文档

**Files:**
- Modify: `.claude/docs/snapshot.md`
- Modify: `.claude/docs/tasks.md`
- Modify: `.claude/docs/learned.md`

- [ ] **Step 1: 更新 snapshot.md 标记 M6 完成**

打开 `.claude/docs/snapshot.md`，更新：
- 当前状态：M6 完成（网络层已实现）
- 最近提交：添加 M6 相关 commit
- 最近修改：添加 src/network/*, tests/network/*

- [ ] **Step 2: 更新 tasks.md 标记 M6 为已完成**

打开 `.claude/docs/tasks.md`，将 M6 从待办移到已完成：
```markdown
### M6: 网络层 ✅

- [x] 添加依赖（tokio-util, serde, serde_json）
- [x] 实现 NetworkError 错误类型
- [x] 实现 Protocol trait + Request/Response
- [x] 实现 JsonProtocol
- [x] 实现 SqlHandler（mock executor）
- [x] 实现 ConnectionHandler
- [x] 实现 Server（TcpListener + shutdown）
- [x] 单元测试（protocol_test）
- [x] 集成测试（server_test）

**完成日期**: 2026-05-20
**验证结果**: cargo test (passed) ✅, cargo clippy ✅, cargo fmt ✅
**新增测试**: protocol_test(5), server_test(4)
**范围**: 仅网络层，mock executor
```

- [ ] **Step 3: 更新 learned.md 记录网络层知识**

打开 `.claude/docs/learned.md`，添加：
```markdown
| tokio::net::TcpListener | `TcpListener::bind(addr).await` | TCP 监听 | 2026-05-20 |
| tokio::spawn | `tokio::spawn(async move { handler.handle(stream) })` | 每连接一协程 | 2026-05-20 |
| CancellationToken | `tokio_util::sync::CancellationToken` | Graceful shutdown | 2026-05-20 |
| tokio::select! | `tokio::select! { accept => ..., shutdown => ... }` | 多事件监听 | 2026-05-20 |
| Protocol trait | `#[async_trait] trait Protocol { async fn parse_request/write_response }` | 协议抽象 | 2026-05-20 |
| JSON 帧协议 | 消息以 `\n` 结尾，serde_json 序列化 | 简单帧协议 | 2026-05-20 |
```

- [ ] **Step 4: Commit**

```bash
git add .claude/docs/snapshot.md .claude/docs/tasks.md .claude/docs/learned.md
git commit -m "docs: mark M6 complete, update project status"
```

---

## Self-Review Checklist

**1. Spec Coverage:**

| Spec Section | Task | Status |
|--------------|------|--------|
| Protocol trait | Task 3, 4 | ✅ |
| JsonProtocol | Task 4, 5 | ✅ |
| ConnectionHandler | Task 7 | ✅ |
| SqlHandler | Task 6 | ✅ |
| Server | Task 8, 9 | ✅ |
| NetworkError | Task 2 | ✅ |
| 测试策略 | Task 5, 9 | ✅ |
| 依赖变更 | Task 1 | ✅ |
| 文档更新 | Task 11 | ✅ |

**2. Placeholder Scan:**

- ✅ 无 "TBD/TODO/implement later"
- ✅ 无 "add appropriate error handling"
- ✅ 无 "write tests for the above"
- ✅ 无 "similar to Task N"
- ✅ 所有代码步骤包含完整代码
- ✅ 所有类型定义在 Task 3-8 中完成

**3. Type Consistency:**

- ✅ `Request` 定义在 Task 3，使用在 Task 5, 6, 9
- ✅ `Response` 定义在 Task 3，使用在 Task 5, 6, 9
- ✅ `NetworkError` 定义在 Task 2，使用在 Task 3, 7, 8
- ✅ `Protocol` trait 定义在 Task 3，实现 in Task 4
- ✅ `JsonProtocol` 定义在 Task 4，使用在 Task 8
- ✅ `SqlHandler::execute` 返回 `Response`，定义在 Task 6
- ✅ `ConnectionHandler::handle` 返回 `Result<(), NetworkError>`，定义在 Task 7

---

## 验证检查点

**Gate 5 验证**（每个 Task 完成后）:

| Task | 验证命令 | Expected Output |
|------|----------|-----------------|
| Task 1 | `cargo build` | 成功下载依赖 |
| Task 2 | `cargo build` | 成功编译 |
| Task 3 | `cargo build` | 成功编译 |
| Task 4 | `cargo build` | 成功编译 |
| Task 5 | `cargo test --test protocol_test` | 5 passed |
| Task 6 | `cargo build` | 成功编译 |
| Task 7 | `cargo build` | 成功编译 |
| Task 8 | `cargo build` | 成功编译 |
| Task 9 | `cargo test --test server_test` | 4 passed |
| Task 10 | `cargo test` | All passed |
| Task 10 | `cargo clippy` | No errors |
| Task 10 | `cargo fmt` | Formatted |

---

## 执行顺序

```
Task 1 (依赖) → Task 2 (NetworkError) → Task 3 (Protocol trait) → Task 4 (JsonProtocol)
  ↓
Task 5 (测试 JsonProtocol) → Task 6 (SqlHandler) → Task 7 (ConnectionHandler) → Task 8 (Server)
  ↓
Task 9 (测试 Server) → Task 10 (全量验证) → Task 11 (文档更新)
```

---

**Plan complete. Ready for execution.**