//! RTsql one-shot CLI 入口

#[tokio::main]
async fn main() -> std::process::ExitCode {
    rtsql::cli::run().await
}
