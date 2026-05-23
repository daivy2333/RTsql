// E2E tests - moved to pg_integration_test.rs (PostgreSQL protocol)
// and pipeline_test.rs (database-level integration tests)
//
// The project switched from JsonProtocol to PostgreSQL protocol in M8.
// Current integration tests:
// - pg_integration_test.rs: PostgreSQL protocol connection tests
// - pipeline_test.rs: SQL execution pipeline tests (CREATE, INSERT, SELECT, JOIN, etc.)
