//! Tests for PostgreSQL Simple Query Protocol state machine
//!
//! Task 9: PgProtocol state machine structure

use rtsql::network::PgProtocol;

#[test]
fn test_pg_protocol_initial_state() {
    let protocol = PgProtocol::new();
    assert_eq!(protocol.state(), "Startup");
    // process_id and secret_key should be random (at least one > 0)
    assert!(protocol.process_id() > 0 || protocol.secret_key() > 0);
}