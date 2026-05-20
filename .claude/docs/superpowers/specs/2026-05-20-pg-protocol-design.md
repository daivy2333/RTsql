# PostgreSQL 有线协议设计（Simple Query Protocol）

> 设计日期：2026-05-20
> 里程碑：M8（PostgreSQL 协议 + 性能优化）
> 状态：已批准

---

## 概述

### 目标

实现 PostgreSQL 3.0 有线协议的 Simple Query Protocol，兼容 psql 和 libpq 基础连接，替换当前 JsonProtocol。

### 范围

- **协议模式**：Simple Query Protocol（一条 SQL → 执行 → 返回结果）
- **认证**：无认证（AuthenticationOk 直接返回）
- **兼容工具**：psql 基础连接 + libpq 应用集成（基础功能）
- **数据类型**：Int（int4 OID 23）、Text（text OID 25）、Null（OID 0）

### 推迟功能

- Extended Query Protocol（parse/bind/execute）
- SSL/TLS negotiation
- 二进制格式（format_code = 1）
- 更多数据类型（Float/Bool/Date）
- CancelRequest 实际取消逻辑
- 详细错误字段（D/H/P/R）

---

## 架构设计

### 协议层架构

利用现有 Protocol trait 抽象，新增 `PgProtocol` 实现：

```
Protocol trait（parse_request + write_response）
    ├── JsonProtocol（当前，M6 实现）
    └── PgProtocol（M8 新增）
        ├── Startup 消息处理
        ├── Query 消息处理
        ├── Error 消息处理
        └── Terminate 消息处理
```

### 状态机设计

PgProtocol 使用状态机管理协议流程：

```rust
enum ProtocolState {
    Startup,      // 等待 StartupMessage
    Ready,        // ReadyForQuery 已发送，等待 Query
    Querying,     // 执行 SQL，发送 DataRow
    Error,        // 错误状态
}
```

状态流转：

```
Startup → (AuthenticationOk + ParameterStatus + BackendKeyData + ReadyForQuery)
  → Ready → (Query) → Querying → (Response 映射) → Ready
  → Error → (ErrorResponse + ReadyForQuery) → Ready
```

### 文件结构

```
src/network/
├── mod.rs           # 模块导出（新增 PgProtocol）
├── error.rs         # NetworkError（新增 SQLSTATE 映射函数）
├── protocol.rs      # Protocol trait + JsonProtocol（不变）
├── pg_protocol.rs   # 【新增】PgProtocol 实现
├── pg_messages.rs   # 【新增】Startup/Query/Error 消息序列化
├── connection.rs    # ConnectionHandler（不变）
├── handler.rs       # SqlHandler（不变）
└── server.rs        # Server（切换 Protocol：JsonProtocol → PgProtocol）
```

---

## Startup 消息处理

### 流程

```
客户端 → StartupMessage（protocol version 3.0 + user/database）
服务端 → AuthenticationOk（无认证）
服务端 → ParameterStatus（server_version="14.0", client_encoding="UTF8"）
服务端 → BackendKeyData（process_id + secret_key）
服务端 → ReadyForQuery（'I' = Idle）
```

### StartupMessage 解析

- 读取 Int32 length + Int32 protocol version（196608 = 3.0）
- 提取 user/database 参数（null-terminated strings）
- 验证协议版本（仅支持 3.0）

### 响应消息序列

1. **AuthenticationOk**：`Int8 'R' + Int32 8 + Int32 0`
2. **ParameterStatus**：`String name + String value`（server_version、client_encoding）
3. **BackendKeyData**：`Int8 'K' + Int32 12 + Int32 process_id + Int32 secret_key`
4. **ReadyForQuery**：`Int8 'Z' + Int32 5 + Int8 'I'`

---

## Query 消息处理

### 流程

```
客户端 → Query（Int8 'Q' + Int32 length + SQL string）
服务端 → RowDescription（列元数据）
服务端 → DataRow（×N，数据行）
服务端 → CommandComplete（command tag）
服务端 → ReadyForQuery（'I'）
```

### Query 消息解析

- 读取 Int8 'Q' + Int32 length
- 提取 SQL 字符串（null-terminated）
- 传递给现有 pipeline（parse → plan → execute → Response）

### Response 映射

| Response 变体 | PG 消息序列 |
|---------------|------------|
| `QueryResult { rows }` | RowDescription → DataRow(×N) → CommandComplete("SELECT N") |
| `AffectedRows { count }` | CommandComplete("INSERT/UPDATE/DELETE N") |
| `Error { message }` | ErrorResponse → ReadyForQuery('I') |
| `Pong` | CommandComplete("PING")（自定义） |

### RowDescription 结构

```
Int16 field_count
For each field:
  String name
  Int32 table_oid (0)
  Int16 column_attr (0)
  Int32 type_oid (Int: 23, Text: 25, Null: 0)
  Int16 type_size (Int: 4, Text: -1, Null: 0)
  Int32 type_modifier (-1)
  Int16 format_code (0 = text)
```

### DataRow 结构

```
Int16 column_count
For each column:
  Int32 length (-1 = NULL, N = data bytes)
  Byte[N] data (text format)
```

### 数据类型映射

| executor::Value | PG Type OID | Format |
|-----------------|------------|--------|
| `Int(i64)` | 23 (int4) | 文本格式（i64 → ASCII） |
| `String(String)` | 25 (text) | UTF-8 字符串 |
| `Null` | 0 (unknown) | NULL 标记（length = -1） |

---

## Error Handling

### ErrorResponse 消息结构

```
Int8 'E'
Int32 length
Fields (each: Int8 field_type + String value):
  'S' Severity (ERROR/FATAL/PANIC)
  'V' Severity (non-localized)
  'C' Code (SQLSTATE, 5 chars)
  'M' Message (错误详情)
  ...
NUL terminator (Int8 '\0')
```

### 错误类型映射

| 错误类型 | SQLSTATE Code | Severity |
|----------|--------------|----------|
| ParseError | "42000" (syntax error) | ERROR |
| PlanError | "42P01" (undefined table) | ERROR |
| StorageError | "58000" (system error) | ERROR |
| TransactionError | "40001" (serialization failure) | ERROR |
| NetworkError | "08006" (connection failure) | FATAL |

---

## Terminate 和 Cancel 消息

### Terminate 消息

```
客户端 → Terminate（Int8 'X' + Int32 4）
服务端 → 关闭连接（无响应）
```

### CancelRequest 消息

```
客户端 → CancelRequest（Int8 'F' + Int32 8 + Int32 process_id + Int32 secret_key）
服务端 → ErrorResponse（"57014" Query cancelled）→ ReadyForQuery('I')
```

**推迟**：CancelRequest 实际取消逻辑（需要异步任务取消机制）

---

## 测试策略

### 单元测试

| 测试层级 | 测试内容 |
|----------|----------|
| **消息序列化** | Startup/Query/Error 消息字节格式正确性 |
| **协议状态流转** | Startup → Ready → Query → Ready 流程 |
| **数据类型映射** | Value → PG OID → DataRow 格式正确性 |
| **错误处理** | ErrorResponse SQLSTATE 映射正确性 |

### 集成测试

| 测试场景 | 测试方法 |
|----------|----------|
| **psql 连接** | 使用 psql 命令行工具连接 Server |
| **SELECT 查询** | psql 执行 SELECT，验证返回格式 |
| **INSERT/UPDATE** | psql 执行 DML，验证 CommandComplete |
| **错误处理** | psql 执行语法错误 SQL，验证 ErrorResponse |

---

## 实现计划

### Phase 1：消息序列化层（pg_messages.rs）

- Startup 消息序列化（AuthenticationOk、ParameterStatus、BackendKeyData、ReadyForQuery）
- Query 消息序列化（RowDescription、DataRow、CommandComplete）
- ErrorResponse 消息序列化（SQLSTATE 映射）

### Phase 2：协议状态机（pg_protocol.rs）

- PgProtocol 结构（state、process_id、secret_key）
- ProtocolState 状态机实现
- parse_request 实现（Startup/Query/Terminate 解析）
- write_response 实现（Response → PG 消息序列）

### Phase 3：Server 切换

- Server 切换 Protocol：JsonProtocol → PgProtocol
- 验证 psql 连接基础功能

### Phase 4：测试覆盖

- 单元测试（消息序列化 + 状态流转）
- 集成测试（psql 真实连接）

---

## 关键设计决策

1. **Simple Query Protocol**：符合 YAGNI 原则，满足 psql 基础连接需求
2. **无认证**：嵌入式数据库常见做法，简化实现
3. **文本格式**：数据返回使用文本格式（format_code = 0），便于调试
4. **渐进式实现**：分阶段实现 + 测试，降低风险
5. **推迟复杂功能**：Extended Protocol、SSL/TLS、二进制格式推迟后续 milestone

---

## 风险和约束

### 实现风险

- **协议细节**：PostgreSQL 协议字节格式复杂，需精确实现
- **psql 兼容性**：psql 可能发送未预期的消息类型（需测试验证）
- **数据类型限制**：仅支持 3 种数据类型（Int/Text/Null），复杂查询受限

### 已知约束

- **无 SSL/TLS**：psql 可能要求 SSL negotiation（需处理 SSLRequest 拒绝）
- **无 Extended Protocol**：prepared statement 不支持（libpq 高级功能受限）
- **无实际取消**：CancelRequest 仅返回错误，不实际中断执行

---

## 后续扩展路径

M8 完成后，后续 milestone 可扩展：

1. **Extended Query Protocol**：支持 prepared statement 和参数化查询
2. **SSL/TLS**：使用 rustls 或 openssl 实现 SSL negotiation
3. **二进制格式**：支持 format_code = 1，提升性能
4. **更多数据类型**：Float、Bool、Date、JSON 等
5. **CancelRequest 实际取消**：异步任务取消机制