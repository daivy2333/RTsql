# M6 网络层设计规范

> 最后更新：2026-05-20
> 设计阶段：brainstorming → implementation

---

## 1. 概述

M6 里程碑范围：**仅网络层**（不含数据存储层）。

### 交付范围

- TCP 服务器 + 每连接一协程
- Protocol trait 抽象 + JsonProtocol 实现
- SqlHandler（调用 parser → executor）
- 端到端集成测试（索引层执行）

### 推迟内容

- 数据存储层（TableManager、Row 数据）→ 后续里程碑
- PostgreSQL 有线协议 → 后续里程碑
- 会话状态管理 → 后续里程碑
- 事务整合 → 后续里程碑

---

## 2. 架构概览

```
┌─────────────────────────────────────────────────────────────┐
│                      Network Layer (M6)                     │
├─────────────────────────────────────────────────────────────┤
│  Server          TcpListener + connection loop + shutdown   │
│    ↓ spawn(tokio::spawn)                                    │
│  Connection      协议解析 + handler 调用 + response 写回     │
│    ↓ Protocol trait                                         │
│  Protocol        JsonProtocol (Request/Response 序列化)     │
│    ↓ SqlHandler                                              │
│  Handler         parse_sql → PhysicalPlan → Executor       │
├─────────────────────────────────────────────────────────────┤
│                  Parser Layer (M4)                          │
│                  Execution Layer (M5)                       │
│                  Transaction Layer (M3)                     │
│                  Storage Layer (M2)                         │
└─────────────────────────────────────────────────────────────┘
```

### 核心流程

1. Server 监听 TCP 端口，accept 连接后 `tokio::spawn(ConnectionHandler)`
2. ConnectionHandler 读取字节流，通过 Protocol trait 解析为 Request
3. SqlHandler 处理 Request（parse → execute），返回 ExecResult
4. ConnectionHandler 通过 Protocol trait 序列化 Response，写回客户端

---

## 3. 文件结构

```
src/network/
├── mod.rs        # pub use server::Server; pub use protocol::JsonProtocol; ...
├── error.rs      # NetworkError enum
├── protocol.rs   # Protocol trait + JsonProtocol + Request/Response enum
├── connection.rs # ConnectionHandler struct + async fn handle()
├── handler.rs    # SqlHandler struct + fn execute(sql) -> Response
└── server.rs     # Server struct + async fn run() + shutdown signal
```

---

## 4. 组件设计

### 4.1 Protocol trait

```rust
// src/network/protocol.rs

use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use tokio::net::TcpStream;
use crate::network::error::NetworkError;

/// 协议抽象 trait，为后续升级 PG 协议预留接口
#[async_trait]
pub trait Protocol: Send + Sync {
    /// 从字节流解析请求
    async fn parse_request(&mut self, stream: &mut TcpStream) -> Result<Option<Request>, NetworkError>;
    
    /// 序列化响应到字节流
    async fn write_response(&mut self, stream: &mut TcpStream, response: &Response) -> Result<(), NetworkError>;
}

/// JSON 协议实现
pub struct JsonProtocol {
    buffer: Vec<u8>,
}

impl JsonProtocol {
    pub fn new() -> Self {
        Self { buffer: Vec::with_capacity(4096) }
    }
}

/// 请求类型（对应现有 PhysicalPlan 能力）
#[derive(Serialize, Deserialize)]
pub enum Request {
    Query { sql: String },
    Insert { sql: String },
    Update { sql: String },
    Delete { sql: String },
    Ping,
}

/// 响应类型
#[derive(Serialize, Deserialize)]
pub enum Response {
    QueryResult { row_ids: Vec<(u64, u16)> },
    AffectedRows { count: u64 },
    Error { message: String },
    Pong,
}
```

#### JsonProtocol 实现要点

- 使用 `tokio::io::AsyncReadExt::read` 读取字节流
- JSON 消息格式：每条消息以换行符 `\n` 结尾（简单帧协议）
- 反序列化：`serde_json::from_slice(&buffer)`
- 序列化：`serde_json::to_vec(response) + "\n"`

---

### 4.2 ConnectionHandler

```rust
// src/network/connection.rs

use crate::network::protocol::{Protocol, Request, Response};
use crate::network::handler::SqlHandler;
use crate::network::error::NetworkError;
use tokio::net::TcpStream;

pub struct ConnectionHandler<P: Protocol> {
    protocol: P,
    handler: SqlHandler,
}

impl<P: Protocol> ConnectionHandler<P> {
    pub fn new(protocol: P, handler: SqlHandler) -> Self {
        Self { protocol, handler }
    }
    
    pub async fn handle(&mut self, stream: TcpStream) -> Result<(), NetworkError> {
        let mut stream = stream;
        
        loop {
            let request = self.protocol.parse_request(&mut stream).await?;
            
            match request {
                Some(req) => {
                    let response = self.handler.execute(req);
                    self.protocol.write_response(&mut stream, &response).await?;
                }
                None => break,
            }
        }
        
        Ok(())
    }
}
```

---

### 4.3 SqlHandler

```rust
// src/network/handler.rs

use crate::network::protocol::{Request, Response};
use crate::parser::{parse_sql, PlanBuilder};
use crate::executor::{PhysicalPlan, ExecResult};
use crate::network::error::NetworkError;

pub struct SqlHandler {
    // M6 简化：无持久化状态
}

impl SqlHandler {
    pub fn new() -> Self {
        Self {}
    }
    
    pub fn execute(&mut self, request: Request) -> Response {
        match request {
            Request::Query { sql } | Request::Insert { sql } | 
            Request::Update { sql } | Request::Delete { sql } => {
                match self.process_sql(&sql) {
                    Ok(result) => self.result_to_response(result),
                    Err(e) => Response::Error { message: e.to_string() },
                }
            }
            Request::Ping => Response::Pong,
        }
    }
    
    fn process_sql(&mut self, sql: &str) -> Result<ExecResult, NetworkError> {
        // 1. 解析 SQL
        let statements = parse_sql(sql)
            .map_err(|e| NetworkError::ProtocolParse(e.to_string()))?;
        
        if statements.len() != 1 {
            return Err(NetworkError::ProtocolParse("Only single statement supported".into()));
        }
        
        // 2. 生成计划（M6 简化：mock PlanBuilder）
        // TODO: 整合真实 PlanBuilder（需要元数据管理）
        let plan = self.mock_plan(&statements[0])?;
        
        // 3. 执行计划（M6 简化：mock executor）
        // TODO: 整合真实 executor + storage
        self.mock_execute(&plan)
    }
    
    fn result_to_response(&self, result: ExecResult) -> Response {
        match result {
            ExecResult::RowId(rid) => Response::QueryResult {
                row_ids: vec![(rid.page_id(), rid.slot_id())]
            },
            ExecResult::AffectedRows(n) => Response::AffectedRows { count: n },
            ExecResult::NotImplemented => Response::Error {
                message: "Operation not implemented".into()
            },
        }
    }
    
    // M6 mock 函数（后续里程碑替换为真实实现）
    fn mock_plan(&self, stmt: &Statement) -> Result<PhysicalPlan, NetworkError> {
        // TODO: 返回 mock PhysicalPlan
        unimplemented!("mock_plan placeholder")
    }
    
    fn mock_execute(&self, plan: &PhysicalPlan) -> Result<ExecResult, NetworkError> {
        // TODO: 返回 mock ExecResult
        unimplemented!("mock_execute placeholder")
    }
}
```

#### M6 简化说明

- `mock_plan` / `mock_execute` 是占位符，用于测试网络层
- 后续里程碑整合真实 PlanBuilder + Executor + Storage

---

### 4.4 Server

```rust
// src/network/server.rs

use crate::network::connection::ConnectionHandler;
use crate::network::protocol::JsonProtocol;
use crate::network::handler::SqlHandler;
use crate::network::error::NetworkError;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use std::net::SocketAddr;

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
    
    pub fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }
    
    pub async fn run(self) -> Result<(), NetworkError> {
        let listener = TcpListener::bind(self.addr).await?;
        
        loop {
            tokio::select! {
                result = listener.accept() => {
                    let (stream, _addr) = result?;
                    
                    let handler = ConnectionHandler::new(
                        JsonProtocol::new(),
                        SqlHandler::new()
                    );
                    
                    tokio::spawn(async move {
                        if let Err(e) = handler.handle(stream).await {
                            eprintln!("Connection error: {}", e);
                        }
                    });
                }
                _ = self.shutdown.cancelled() => break,
            }
        }
        
        Ok(())
    }
}
```

---

### 4.5 NetworkError

```rust
// src/network/error.rs

use thiserror::Error;
use crate::parser::PlanError;

#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Protocol parse error: {0}")]
    ProtocolParse(String),
    
    #[error("SQL parse error: {0}")]
    SqlParse(#[from] PlanError),
    
    #[error("Execution error: {0}")]
    Execution(String),
}
```

---

## 5. 测试策略

### 5.1 单元测试

```rust
// tests/network/protocol_test.rs

#[tokio::test]
async fn test_json_protocol_roundtrip() {
    // 测试 Request → JSON → Response 流程
}
```

### 5.2 集成测试

```rust
// tests/network/server_test.rs

#[tokio::test]
async fn test_server_query_flow() {
    // 启动 Server → 客户端发送 Query → 验证 QueryResult
}

#[tokio::test]
async fn test_server_insert_flow() {
    // 启动 Server → 客户端发送 Insert → 验证 AffectedRows
}

#[tokio::test]
async fn test_error_handling() {
    // 启动 Server → 客户端发送无效 SQL → 验证 Response::Error
}

#[tokio::test]
async fn test_ping_pong() {
    // 启动 Server → 客户端发送 Ping → 验证 Pong
}
```

#### 测试客户端

使用 `tokio::net::TcpStream` 作为测试客户端，序列化 JSON 消息发送。

---

## 6. 依赖变更

```toml
# Cargo.toml 新增
tokio-util = { version = "0.7", features = ["sync"] }  # CancellationToken
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

---

## 7. 集成策略

### M6 集成范围

- 网络层调用 M4 Parser（parse_sql）
- 网络层调用 M5 Executor（mock 实现）
- **不整合** M2 Storage + M3 Transaction（推迟后续里程碑）

### 后续里程碑集成

- 整合真实 PlanBuilder（需要元数据管理）
- 整合真实 Executor + IndexManager + Storage
- 整合 TransactionManager（多语句事务）

---

## 8. 成功标准

| 标准 | 验证方式 |
|------|----------|
| TCP 服务器正常启动 | cargo run → 客户端连接成功 |
| 每连接一协程 | 多客户端并发测试 |
| JSON 协议正确解析 | 单元测试 roundtrip |
| 索引层执行集成 | 集成测试 Query/Insert 流程 |
| 错误优雅处理 | 集成测试 Error 响应 |
| Graceful shutdown | Ctrl+C → Server 停止 |

---

## 9. 风险与缓解

| 风险 | 缓解措施 |
|------|----------|
| mock executor 不真实 | 后续里程碑替换真实实现 |
| JSON 效率低 | 标记为优化点，后续升级 PG 协议 |
| 无会话状态限制多 | 后续里程碑扩展会话管理 |

---

## 10. 优化点记录

以下内容推迟到后续里程碑或优化阶段：

- JSON 协议效率优化 → 升级 PG 协议或 TLV 二进制
- 会话状态管理 → 多语句事务支持
- 连接池管理 → 客户端连接复用
- SSL/TLS 加密 → 安全连接支持