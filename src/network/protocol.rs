use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::network::error::NetworkError;
use crate::result::{value_to_json, SqlError, SqlOutcome};

/// Protocol abstraction trait
#[async_trait]
pub trait Protocol: Send + Sync {
    async fn parse_request(
        &mut self,
        stream: &mut TcpStream,
    ) -> Result<Option<Request>, NetworkError>;
    async fn write_response(
        &mut self,
        stream: &mut TcpStream,
        response: &Response,
    ) -> Result<(), NetworkError>;
}

/// Request types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Request {
    Query { sql: String },
    Insert { sql: String },
    Update { sql: String },
    Delete { sql: String },
    Ping,
}

/// Response types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Response {
    QueryResult {
        rows: Vec<Vec<serde_json::Value>>,
    },
    AffectedRows {
        count: u64,
    },
    /// `sqlstate` 为 PG 协议 SQLSTATE 分类（MS19-T02）；`None` 时字段不参与
    /// JSON 序列化，输出与本变更前逐字节一致，PG 侧回落 58000。
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        sqlstate: Option<String>,
    },
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

/// 类型化执行结果 → 协议响应（MS31-T01 边界转换）。
///
/// 四臂映射：`Ok(Rows)` → `QueryResult`（逐值 `value_to_json`）、
/// `Ok(AffectedRows)` → `AffectedRows`、`Err` → `Error` 原样透传。
/// `Pong` 不进类型化结果——Ping 由 `SqlHandler` 在调用库入口前本地处理。
pub fn to_response(outcome: std::result::Result<SqlOutcome, SqlError>) -> Response {
    match outcome {
        Ok(SqlOutcome::Rows { rows, .. }) => Response::QueryResult {
            rows: rows
                .into_iter()
                .map(|row| row.into_iter().map(value_to_json).collect())
                .collect(),
        },
        Ok(SqlOutcome::AffectedRows(count)) => Response::AffectedRows { count },
        Err(error) => Response::Error {
            message: error.message,
            sqlstate: error.sqlstate,
        },
    }
}

/// JSON protocol implementation
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

#[async_trait]
impl Protocol for JsonProtocol {
    async fn parse_request(
        &mut self,
        stream: &mut TcpStream,
    ) -> Result<Option<Request>, NetworkError> {
        self.buffer.clear();

        // Read until newline
        let mut byte = [0u8; 1];
        loop {
            match stream.read(&mut byte).await {
                Ok(0) => return Ok(None), // Connection closed
                Ok(_) => {
                    if byte[0] == b'\n' {
                        break;
                    }
                    self.buffer.push(byte[0]);
                }
                Err(e) => return Err(NetworkError::Io(e)),
            }
        }

        // Parse JSON
        let request: Request = serde_json::from_slice(&self.buffer)
            .map_err(|e| NetworkError::ProtocolParse(e.to_string()))?;

        Ok(Some(request))
    }

    async fn write_response(
        &mut self,
        stream: &mut TcpStream,
        response: &Response,
    ) -> Result<(), NetworkError> {
        let json =
            serde_json::to_vec(response).map_err(|e| NetworkError::ProtocolWrite(e.to_string()))?;

        stream.write_all(&json).await?;
        stream.write_all(b"\n").await?;
        stream.flush().await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::Value;
    use crate::result::{ColumnMeta, SqlError, SqlOutcome};

    /// R3 场景 4 通道：None 时 JSON 输出逐字节不变（既有协议兼容见证）。
    #[test]
    fn response_error_json_omits_none_sqlstate() {
        let json = serde_json::to_string(&Response::Error {
            message: "boom".to_string(),
            sqlstate: None,
        })
        .unwrap();
        assert_eq!(json, r#"{"Error":{"message":"boom"}}"#);
    }

    /// R3 场景 4：携带分类时字段出现。
    #[test]
    fn response_error_json_includes_sqlstate_when_present() {
        let json = serde_json::to_string(&Response::Error {
            message: "boom".to_string(),
            sqlstate: Some("0A000".to_string()),
        })
        .unwrap();
        assert_eq!(json, r#"{"Error":{"message":"boom","sqlstate":"0A000"}}"#);
    }

    /// T1：边界转换映射矩阵——Ok(Rows) → QueryResult（逐值 `value_to_json`
    /// 转换，Float NaN → Null、Date → DA5 字符串）。
    #[test]
    fn to_response_maps_rows_with_value_conversion() {
        let outcome = Ok(SqlOutcome::Rows {
            columns: vec![ColumnMeta {
                name: "a".to_string(),
                column_type: None,
            }],
            rows: vec![
                vec![Value::Int(1), Value::Float(f64::NAN)],
                vec![Value::Date(0), Value::Null],
            ],
        });
        match to_response(outcome) {
            Response::QueryResult { rows } => {
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0][0], serde_json::json!(1));
                assert_eq!(rows[0][1], serde_json::Value::Null);
                assert_eq!(rows[1][0], serde_json::json!("0001-01-01"));
                assert_eq!(rows[1][1], serde_json::Value::Null);
            }
            other => panic!("expected QueryResult, got {other:?}"),
        }
    }

    /// T1：Ok(AffectedRows) → AffectedRows 透传。
    #[test]
    fn to_response_maps_affected_rows() {
        match to_response(Ok(SqlOutcome::AffectedRows(3))) {
            Response::AffectedRows { count } => assert_eq!(count, 3),
            other => panic!("expected AffectedRows, got {other:?}"),
        }
    }

    /// T1：Err(SqlError) → Error{message, sqlstate} 逐字段相等。
    #[test]
    fn to_response_maps_error_payload_field_by_field() {
        let outcome = Err(SqlError {
            message: "boom".to_string(),
            sqlstate: Some("42703".to_string()),
        });
        match to_response(outcome) {
            Response::Error { message, sqlstate } => {
                assert_eq!(message, "boom");
                assert_eq!(sqlstate, Some("42703".to_string()));
            }
            other => panic!("expected Error, got {other:?}"),
        }
    }
}
