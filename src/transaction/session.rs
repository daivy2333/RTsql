//! Session-scoped explicit transaction state (MS11-T02 design D1).
//!
//! The CLI one-shot call creates one `TransactionSession` per `run_sql`
//! invocation; transaction statements (BEGIN/COMMIT/ROLLBACK) drive it and
//! ordinary statements execute through [`Database::execute_in_tx`] with the
//! session's transaction. Errors are plain messages carrying the design D3
//! contract text — the message string is the contract.

use crate::database::Database;
use crate::transaction::Transaction;

/// Session-scoped explicit transaction state (MS11-T02 design D1).
///
/// One instance per statement loop (CLI `run_sql` in Iteration 001);
/// transaction statements drive it and ordinary statements execute through
/// [`Database::execute_in_tx`] with [`TransactionSession::tx`]. Methods
/// return plain messages carrying the design D3 contract text — the message
/// string is the contract, not a typed error.
pub struct TransactionSession {
    tx: Option<Transaction>,
}

impl TransactionSession {
    pub fn new() -> Self {
        Self { tx: None }
    }

    /// Whether a transaction is currently open in this session.
    pub fn is_active(&self) -> bool {
        self.tx.is_some()
    }

    /// Id of the open transaction, or `None` when idle.
    pub fn tx_id(&self) -> Option<u64> {
        self.tx.as_ref().map(Transaction::id)
    }

    /// Borrow the open transaction for in-transaction statement execution
    /// (`Database::execute_in_tx`).
    pub fn tx(&self) -> Option<&Transaction> {
        self.tx.as_ref()
    }

    /// Open the session transaction.
    ///
    /// Errors with `transaction already active` when a transaction is already
    /// open (no nesting); the open transaction is left untouched.
    pub async fn begin(&mut self, db: &Database) -> Result<(), String> {
        if self.tx.is_some() {
            return Err("transaction already active".to_string());
        }
        match db.begin().await {
            Ok(tx) => {
                self.tx = Some(tx);
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        }
    }

    /// Commit and close the session transaction.
    ///
    /// Errors with `no active transaction` when idle.
    pub async fn commit(&mut self, db: &Database) -> Result<(), String> {
        match self.tx.take() {
            None => Err("no active transaction".to_string()),
            Some(tx) => db.commit(tx).await.map_err(|e| e.to_string()),
        }
    }

    /// Roll back and close the session transaction.
    ///
    /// Errors with `no active transaction` when idle.
    pub async fn rollback(&mut self, db: &Database) -> Result<(), String> {
        match self.tx.take() {
            None => Err("no active transaction".to_string()),
            Some(tx) => db.rollback(tx).await.map_err(|e| e.to_string()),
        }
    }
}

impl Default for TransactionSession {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::protocol::Response;

    async fn open_db() -> (Database, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(&dir.path().join("session_test.db"))
            .await
            .unwrap();
        (db, dir)
    }

    fn expect_ok(resp: &Response, what: &str) {
        assert!(
            !matches!(resp, Response::Error { .. }),
            "{what} failed: {:?}",
            resp
        );
    }

    fn query_rows(resp: Response, what: &str) -> Vec<Vec<serde_json::Value>> {
        match resp {
            Response::QueryResult { rows } => rows,
            other => panic!("{what}: expected QueryResult, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn begin_commit_roundtrip_persists_writes() {
        let (db, _dir) = open_db().await;
        expect_ok(&db.execute_sql("CREATE TABLE t (id INT)").await, "create t");

        let mut session = TransactionSession::new();
        assert!(!session.is_active());
        assert_eq!(session.tx_id(), None);

        session.begin(&db).await.unwrap();
        assert!(session.is_active());
        assert!(session.tx_id().is_some());

        expect_ok(
            &db.execute_in_tx("INSERT INTO t VALUES (1)", session.tx().unwrap())
                .await,
            "in-session insert",
        );
        session.commit(&db).await.unwrap();
        assert!(!session.is_active());
        assert_eq!(session.tx_id(), None);

        let rows = query_rows(
            db.execute_sql("SELECT id FROM t").await,
            "post-commit select",
        );
        assert_eq!(rows.len(), 1, "committed write must be visible");
        assert_eq!(rows[0][0], serde_json::json!(1));
    }

    #[tokio::test]
    async fn begin_rollback_discards_writes() {
        let (db, _dir) = open_db().await;
        expect_ok(&db.execute_sql("CREATE TABLE t (id INT)").await, "create t");

        let mut session = TransactionSession::new();
        session.begin(&db).await.unwrap();
        expect_ok(
            &db.execute_in_tx("INSERT INTO t VALUES (1)", session.tx().unwrap())
                .await,
            "in-session insert",
        );
        session.rollback(&db).await.unwrap();
        assert!(!session.is_active());

        let rows = query_rows(
            db.execute_sql("SELECT id FROM t").await,
            "post-rollback select",
        );
        assert_eq!(rows.len(), 0, "rolled-back write must be invisible");
    }

    #[tokio::test]
    async fn begin_twice_errors_and_keeps_first_transaction() {
        let (db, _dir) = open_db().await;
        let mut session = TransactionSession::new();
        session.begin(&db).await.unwrap();
        let tx_id = session.tx_id().unwrap();

        let err = session.begin(&db).await.unwrap_err();
        assert_eq!(err, "transaction already active");
        assert!(session.is_active(), "original transaction must stay open");
        assert_eq!(session.tx_id(), Some(tx_id));
    }

    #[tokio::test]
    async fn commit_and_rollback_without_begin_error() {
        let (db, _dir) = open_db().await;
        let mut session = TransactionSession::new();

        let err = session.commit(&db).await.unwrap_err();
        assert_eq!(err, "no active transaction");
        let err = session.rollback(&db).await.unwrap_err();
        assert_eq!(err, "no active transaction");
    }

    #[tokio::test]
    async fn session_tx_id_consistent_with_database_allocation() {
        let (db, _dir) = open_db().await;
        let pre = db.begin().await.unwrap();
        let mut session = TransactionSession::new();
        session.begin(&db).await.unwrap();

        let session_id = session.tx_id().unwrap();
        assert!(
            session_id > pre.id(),
            "session tx_id must come from the same allocator as db.begin()"
        );

        // Terminate both transactions so the manager state stays clean.
        session.rollback(&db).await.unwrap();
        db.rollback(pre).await.unwrap();
    }
}
