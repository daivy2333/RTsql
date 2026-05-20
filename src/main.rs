//! RTsql - Async coroutine-driven embedded relational database

#![allow(unused_imports)] // M0: Imports validate module exports work

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