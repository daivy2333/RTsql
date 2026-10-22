use crate::database::Database;
use crate::network::protocol::{to_response, Request, Response};
use std::sync::Arc;

pub struct SqlHandler {
    database: Arc<Database>,
}

impl SqlHandler {
    pub fn new(database: Arc<Database>) -> Self {
        Self { database }
    }

    pub async fn execute(&self, request: Request) -> Response {
        let sql = match request.sql() {
            Some(sql) => sql.to_string(),
            None => {
                return match request {
                    Request::Ping => Response::Pong,
                    _ => Response::Error {
                        message: "Unknown request".to_string(),
                        sqlstate: None,
                    },
                };
            }
        };

        // MS31-T01：库入口返回类型化结果——协议面在此边界转换（Ping 在
        // 调用库入口前本地处理，不进类型化结果）。
        to_response(self.database.execute_sql(&sql).await)
    }
}
