use crate::database::Database;
use crate::network::protocol::{Request, Response};
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
                    },
                };
            }
        };

        self.database.execute_sql(&sql).await
    }
}
