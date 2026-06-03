//! RTsql - Async coroutine-driven embedded relational database

use rtsql::database::Database;
use rtsql::network::Server;
use rtsql::storage::ColumnType;
use std::path::Path;
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let database = Arc::new(
        Database::open(Path::new("rtsql.db"))
            .await
            .expect("Failed to open database"),
    );

    database
        .create_table("test", vec![("id".to_string(), ColumnType::Int)], "id")
        .await
        .ok();

    let addr = "127.0.0.1:9876".parse().unwrap();
    let server = Server::new(addr, database, 64);

    println!("RTsql server listening on {}", addr);
    server.run().await.unwrap();
}
