use crate::network::protocol::{Request, Response};

/// SQL handler (M6 simplified: mock executor)
pub struct SqlHandler {
    // M6: No persistent state
}

impl SqlHandler {
    pub fn new() -> Self {
        Self {}
    }

    pub fn execute(&mut self, request: Request) -> Response {
        match request {
            Request::Query { sql: _ } => {
                // M6 mock: return fixed RowId
                Response::QueryResult {
                    row_ids: vec![(0, 1)], // mock: page_id=0, slot_id=1
                }
            }
            Request::Insert { sql: _ } => {
                // M6 mock: return fixed AffectedRows
                Response::AffectedRows { count: 1 }
            }
            Request::Update { sql: _ } => Response::AffectedRows { count: 1 },
            Request::Delete { sql: _ } => Response::AffectedRows { count: 1 },
            Request::Ping => Response::Pong,
        }
    }
}

impl Default for SqlHandler {
    fn default() -> Self {
        Self::new()
    }
}
