//! Session state management for async query sessions and transactions.

use crate::database::QueryResult;
use crate::error::ServerError;
use chrono::{DateTime, Utc};
use mssql_client::CancelHandle;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Shared state wrapper type.
pub type SharedState = Arc<RwLock<SessionState>>;

/// Create a new shared state instance.
pub fn new_shared_state() -> SharedState {
    Arc::new(RwLock::new(SessionState::new()))
}

/// Session state for managing async queries, transactions, and server state.
#[derive(Debug, Default)]
pub struct SessionState {
    /// Active async query sessions.
    sessions: HashMap<String, QuerySession>,

    /// Active transactions.
    transactions: HashMap<String, TransactionState>,

    /// Cancel handles for running async queries.
    /// Maps session_id -> CancelHandle for native query cancellation.
    cancel_handles: HashMap<String, CancelHandle>,

    /// Server initialization status.
    initialized: bool,

    /// Current default timeout in seconds.
    default_timeout_seconds: u64,

    /// Current database name (for multi-database support).
    current_database: Option<String>,
}

/// Transaction isolation level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum IsolationLevel {
    ReadUncommitted,
    #[default]
    ReadCommitted,
    RepeatableRead,
    Serializable,
    Snapshot,
}

impl IsolationLevel {
    /// Get the full SQL statement to set this isolation level.
    ///
    /// This is consistent with mssql-client's IsolationLevel::as_sql() API.
    #[must_use]
    pub fn as_sql(&self) -> &'static str {
        match self {
            Self::ReadUncommitted => "SET TRANSACTION ISOLATION LEVEL READ UNCOMMITTED",
            Self::ReadCommitted => "SET TRANSACTION ISOLATION LEVEL READ COMMITTED",
            Self::RepeatableRead => "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ",
            Self::Serializable => "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE",
            Self::Snapshot => "SET TRANSACTION ISOLATION LEVEL SNAPSHOT",
        }
    }

    /// Get the isolation level name as used in SQL Server.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ReadUncommitted => "READ UNCOMMITTED",
            Self::ReadCommitted => "READ COMMITTED",
            Self::RepeatableRead => "REPEATABLE READ",
            Self::Serializable => "SERIALIZABLE",
            Self::Snapshot => "SNAPSHOT",
        }
    }
}

impl std::fmt::Display for IsolationLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Error returned when parsing an isolation level fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseIsolationLevelError(String);

impl std::fmt::Display for ParseIsolationLevelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid isolation level: '{}'", self.0)
    }
}

impl std::error::Error for ParseIsolationLevelError {}

impl std::str::FromStr for IsolationLevel {
    type Err = ParseIsolationLevelError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().replace(['-', '_'], " ").trim() {
            "read uncommitted" => Ok(IsolationLevel::ReadUncommitted),
            "read committed" => Ok(IsolationLevel::ReadCommitted),
            "repeatable read" => Ok(IsolationLevel::RepeatableRead),
            "serializable" => Ok(IsolationLevel::Serializable),
            "snapshot" => Ok(IsolationLevel::Snapshot),
            _ => Err(ParseIsolationLevelError(s.to_string())),
        }
    }
}

/// State of an active transaction.
#[derive(Debug, Clone)]
pub struct TransactionState {
    /// Unique transaction identifier.
    pub id: String,

    /// Optional transaction name.
    pub name: Option<String>,

    /// Transaction isolation level.
    pub isolation_level: IsolationLevel,

    /// When the transaction was started.
    pub started_at: DateTime<Utc>,

    /// Last activity timestamp.
    pub last_activity: DateTime<Utc>,

    /// Number of statements executed in this transaction.
    pub statement_count: u32,

    /// List of savepoints.
    pub savepoints: Vec<String>,

    /// Transaction status.
    pub status: TransactionStatus,
}

/// Status of a transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionStatus {
    Active,
    Committed,
    RolledBack,
}

impl std::fmt::Display for TransactionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransactionStatus::Active => write!(f, "active"),
            TransactionStatus::Committed => write!(f, "committed"),
            TransactionStatus::RolledBack => write!(f, "rolled_back"),
        }
    }
}

impl TransactionState {
    /// Create a new transaction.
    pub fn new(name: Option<String>, isolation_level: IsolationLevel) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            isolation_level,
            started_at: now,
            last_activity: now,
            statement_count: 0,
            savepoints: Vec::new(),
            status: TransactionStatus::Active,
        }
    }

    /// Record a statement execution.
    pub fn record_statement(&mut self) {
        self.statement_count += 1;
        self.last_activity = Utc::now();
    }

    /// Add a savepoint.
    pub fn add_savepoint(&mut self, name: String) {
        if !self.savepoints.contains(&name) {
            self.savepoints.push(name);
        }
        self.last_activity = Utc::now();
    }

    /// Mark transaction as committed.
    pub fn commit(&mut self) {
        self.status = TransactionStatus::Committed;
        self.last_activity = Utc::now();
    }

    /// Mark transaction as rolled back.
    pub fn rollback(&mut self) {
        self.status = TransactionStatus::RolledBack;
        self.last_activity = Utc::now();
    }

    /// Check if transaction is still active.
    pub fn is_active(&self) -> bool {
        self.status == TransactionStatus::Active
    }

    /// Get transaction age in seconds.
    pub fn age_seconds(&self) -> i64 {
        (Utc::now() - self.started_at).num_seconds()
    }
}

/// Summary of a transaction for listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionSummary {
    pub id: String,
    pub name: Option<String>,
    pub isolation_level: String,
    pub status: String,
    pub started_at: String,
    pub statement_count: u32,
    pub age_seconds: i64,
}

impl From<&TransactionState> for TransactionSummary {
    fn from(tx: &TransactionState) -> Self {
        Self {
            id: tx.id.clone(),
            name: tx.name.clone(),
            isolation_level: tx.isolation_level.to_string(),
            status: tx.status.to_string(),
            started_at: tx.started_at.to_rfc3339(),
            statement_count: tx.statement_count,
            age_seconds: tx.age_seconds(),
        }
    }
}

/// Status of an async query session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    /// Query is currently running.
    Running,

    /// Query completed successfully.
    Completed,

    /// Query failed with an error.
    Failed,

    /// Query was cancelled.
    Cancelled,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Running => write!(f, "running"),
            SessionStatus::Completed => write!(f, "completed"),
            SessionStatus::Failed => write!(f, "failed"),
            SessionStatus::Cancelled => write!(f, "cancelled"),
        }
    }
}

/// An async query session.
#[derive(Debug, Clone)]
pub struct QuerySession {
    /// Unique session identifier.
    pub id: String,

    /// The query being executed.
    pub query: String,

    /// Session status.
    pub status: SessionStatus,

    /// Query result (if completed).
    pub result: Option<QueryResult>,

    /// Error message (if failed).
    pub error: Option<String>,

    /// When the session was created.
    pub created_at: DateTime<Utc>,

    /// When the session was last updated.
    pub updated_at: DateTime<Utc>,

    /// Progress percentage (0-100).
    pub progress: u8,
}

impl QuerySession {
    /// Create a new running session.
    pub fn new(query: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4().to_string(),
            query,
            status: SessionStatus::Running,
            result: None,
            error: None,
            created_at: now,
            updated_at: now,
            progress: 0,
        }
    }

    /// Mark the session as completed with a result.
    pub fn complete(&mut self, result: QueryResult) {
        self.status = SessionStatus::Completed;
        self.result = Some(result);
        self.updated_at = Utc::now();
        self.progress = 100;
    }

    /// Mark the session as failed with an error.
    pub fn fail(&mut self, error: String) {
        self.status = SessionStatus::Failed;
        self.error = Some(error);
        self.updated_at = Utc::now();
    }

    /// Mark the session as cancelled.
    pub fn cancel(&mut self) {
        self.status = SessionStatus::Cancelled;
        self.updated_at = Utc::now();
    }

    /// Update progress.
    pub fn set_progress(&mut self, progress: u8) {
        self.progress = progress.min(100);
        self.updated_at = Utc::now();
    }

    /// Check if the session is still running.
    pub fn is_running(&self) -> bool {
        self.status == SessionStatus::Running
    }

    /// Get session age in seconds.
    pub fn age_seconds(&self) -> i64 {
        (Utc::now() - self.created_at).num_seconds()
    }
}

/// Summary of a session for listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub status: String,
    pub query_preview: String,
    pub created_at: String,
    pub progress: u8,
}

impl From<&QuerySession> for SessionSummary {
    fn from(session: &QuerySession) -> Self {
        Self {
            id: session.id.clone(),
            status: session.status.to_string(),
            query_preview: truncate(&session.query, 100),
            created_at: session.created_at.to_rfc3339(),
            progress: session.progress,
        }
    }
}

impl SessionState {
    /// Create a new session state.
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            transactions: HashMap::new(),
            cancel_handles: HashMap::new(),
            initialized: false,
            default_timeout_seconds: 30,
            current_database: None,
        }
    }

    /// Mark the server as initialized.
    pub fn mark_initialized(&mut self) {
        self.initialized = true;
    }

    /// Check if the server is initialized.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Get the default timeout.
    pub fn default_timeout(&self) -> u64 {
        self.default_timeout_seconds
    }

    /// Set the default timeout.
    pub fn set_default_timeout(&mut self, seconds: u64) {
        self.default_timeout_seconds = seconds;
    }

    /// Create a new session and return its ID.
    pub fn create_session(
        &mut self,
        query: String,
        max_sessions: usize,
    ) -> Result<String, ServerError> {
        // Check if we've hit the session limit
        let running_count = self.sessions.values().filter(|s| s.is_running()).count();
        if running_count >= max_sessions {
            return Err(ServerError::Session(format!(
                "Maximum concurrent sessions ({}) reached",
                max_sessions
            )));
        }

        let session = QuerySession::new(query);
        let id = session.id.clone();
        self.sessions.insert(id.clone(), session);
        Ok(id)
    }

    /// Get a session by ID.
    pub fn get_session(&self, id: &str) -> Option<&QuerySession> {
        self.sessions.get(id)
    }

    /// Get a mutable session by ID.
    pub fn get_session_mut(&mut self, id: &str) -> Option<&mut QuerySession> {
        self.sessions.get_mut(id)
    }

    /// List all sessions.
    pub fn list_sessions(&self) -> Vec<SessionSummary> {
        self.sessions.values().map(SessionSummary::from).collect()
    }

    /// List sessions by status.
    pub fn list_sessions_by_status(&self, status: SessionStatus) -> Vec<SessionSummary> {
        self.sessions
            .values()
            .filter(|s| s.status == status)
            .map(SessionSummary::from)
            .collect()
    }

    /// Remove a session.
    pub fn remove_session(&mut self, id: &str) -> Option<QuerySession> {
        self.sessions.remove(id)
    }

    /// Clean up old sessions.
    pub fn cleanup_sessions(&mut self, max_age_seconds: i64) {
        self.sessions.retain(|_, session| {
            // Keep running sessions
            if session.is_running() {
                return true;
            }
            // Remove old completed/failed/cancelled sessions
            session.age_seconds() < max_age_seconds
        });
    }

    /// Get count of running sessions.
    pub fn running_session_count(&self) -> usize {
        self.sessions.values().filter(|s| s.is_running()).count()
    }

    /// Get total session count.
    pub fn total_session_count(&self) -> usize {
        self.sessions.len()
    }

    // =========================================================================
    // Cancel Handle Management
    // =========================================================================

    /// Store a cancel handle for a session.
    ///
    /// This associates a CancelHandle with a session ID, allowing native
    /// SQL Server query cancellation via Attention packets.
    pub fn store_cancel_handle(&mut self, session_id: &str, handle: CancelHandle) {
        self.cancel_handles.insert(session_id.to_string(), handle);
    }

    /// Get a cancel handle for a session.
    ///
    /// Returns the CancelHandle if one exists for the session.
    pub fn get_cancel_handle(&self, session_id: &str) -> Option<&CancelHandle> {
        self.cancel_handles.get(session_id)
    }

    /// Remove a cancel handle for a session.
    ///
    /// Called when a session completes or is cancelled to clean up resources.
    pub fn remove_cancel_handle(&mut self, session_id: &str) -> Option<CancelHandle> {
        self.cancel_handles.remove(session_id)
    }

    /// Check if a session has a cancel handle.
    pub fn has_cancel_handle(&self, session_id: &str) -> bool {
        self.cancel_handles.contains_key(session_id)
    }

    /// Clean up cancel handles for non-running sessions.
    ///
    /// This removes cancel handles for sessions that are no longer running.
    pub fn cleanup_cancel_handles(&mut self) {
        let running_session_ids: std::collections::HashSet<_> = self
            .sessions
            .iter()
            .filter(|(_, s)| s.is_running())
            .map(|(id, _)| id.clone())
            .collect();

        self.cancel_handles
            .retain(|id, _| running_session_ids.contains(id));
    }

    // =========================================================================
    // Transaction Management
    // =========================================================================

    /// Create a new transaction and return its ID.
    pub fn create_transaction(
        &mut self,
        name: Option<String>,
        isolation_level: IsolationLevel,
        max_transactions: usize,
    ) -> Result<String, ServerError> {
        // Check if we've hit the transaction limit
        let active_count = self.transactions.values().filter(|t| t.is_active()).count();
        if active_count >= max_transactions {
            return Err(ServerError::Session(format!(
                "Maximum concurrent transactions ({}) reached",
                max_transactions
            )));
        }

        let tx = TransactionState::new(name, isolation_level);
        let id = tx.id.clone();
        self.transactions.insert(id.clone(), tx);
        Ok(id)
    }

    /// Get a transaction by ID.
    pub fn get_transaction(&self, id: &str) -> Option<&TransactionState> {
        self.transactions.get(id)
    }

    /// Get a mutable transaction by ID.
    pub fn get_transaction_mut(&mut self, id: &str) -> Option<&mut TransactionState> {
        self.transactions.get_mut(id)
    }

    /// List all transactions.
    pub fn list_transactions(&self) -> Vec<TransactionSummary> {
        self.transactions
            .values()
            .map(TransactionSummary::from)
            .collect()
    }

    /// List active transactions.
    pub fn list_active_transactions(&self) -> Vec<TransactionSummary> {
        self.transactions
            .values()
            .filter(|t| t.is_active())
            .map(TransactionSummary::from)
            .collect()
    }

    /// Remove a transaction.
    pub fn remove_transaction(&mut self, id: &str) -> Option<TransactionState> {
        self.transactions.remove(id)
    }

    /// Clean up old completed transactions.
    pub fn cleanup_transactions(&mut self, max_age_seconds: i64) {
        self.transactions.retain(|_, tx| {
            // Keep active transactions
            if tx.is_active() {
                return true;
            }
            // Remove old completed/rolled back transactions
            tx.age_seconds() < max_age_seconds
        });
    }

    /// Get count of active transactions.
    pub fn active_transaction_count(&self) -> usize {
        self.transactions.values().filter(|t| t.is_active()).count()
    }

    /// Get total transaction count.
    pub fn total_transaction_count(&self) -> usize {
        self.transactions.len()
    }

    // =========================================================================
    // Database Management
    // =========================================================================

    /// Set the current database.
    pub fn set_current_database(&mut self, database: Option<String>) {
        self.current_database = database;
    }

    /// Get the current database.
    pub fn current_database(&self) -> Option<&str> {
        self.current_database.as_deref()
    }
}

/// Truncate a string for preview.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_lifecycle() {
        let mut session = QuerySession::new("SELECT 1".to_string());
        assert!(session.is_running());
        assert_eq!(session.progress, 0);

        session.set_progress(50);
        assert_eq!(session.progress, 50);

        session.complete(crate::database::QueryResult::empty());
        assert!(!session.is_running());
        assert_eq!(session.status, SessionStatus::Completed);
        assert_eq!(session.progress, 100);
    }

    #[test]
    fn test_session_state() {
        let mut state = SessionState::new();
        assert!(!state.is_initialized());

        state.mark_initialized();
        assert!(state.is_initialized());

        // Create sessions
        let id1 = state.create_session("SELECT 1".to_string(), 10).unwrap();
        let _id2 = state.create_session("SELECT 2".to_string(), 10).unwrap();

        assert_eq!(state.total_session_count(), 2);
        assert_eq!(state.running_session_count(), 2);

        // Complete one session
        state
            .get_session_mut(&id1)
            .unwrap()
            .complete(crate::database::QueryResult::empty());
        assert_eq!(state.running_session_count(), 1);

        // List sessions
        let sessions = state.list_sessions();
        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn test_session_limit() {
        let mut state = SessionState::new();

        // Create max sessions
        for i in 0..3 {
            state.create_session(format!("SELECT {}", i), 3).unwrap();
        }

        // Should fail to create another
        let result = state.create_session("SELECT 4".to_string(), 3);
        assert!(result.is_err());
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("this is too long", 10), "this is to...");
    }

    #[test]
    fn test_transaction_lifecycle() {
        let mut tx =
            TransactionState::new(Some("test_tx".to_string()), IsolationLevel::ReadCommitted);
        assert!(tx.is_active());
        assert_eq!(tx.statement_count, 0);

        tx.record_statement();
        assert_eq!(tx.statement_count, 1);

        tx.add_savepoint("sp1".to_string());
        assert_eq!(tx.savepoints.len(), 1);

        tx.commit();
        assert!(!tx.is_active());
        assert_eq!(tx.status, TransactionStatus::Committed);
    }

    #[test]
    fn test_transaction_state() {
        let mut state = SessionState::new();

        // Create transactions
        let id1 = state
            .create_transaction(Some("tx1".to_string()), IsolationLevel::ReadCommitted, 10)
            .unwrap();
        let id2 = state
            .create_transaction(None, IsolationLevel::Serializable, 10)
            .unwrap();

        assert_eq!(state.total_transaction_count(), 2);
        assert_eq!(state.active_transaction_count(), 2);

        // Commit one transaction
        state.get_transaction_mut(&id1).unwrap().commit();
        assert_eq!(state.active_transaction_count(), 1);

        // List transactions
        let txns = state.list_transactions();
        assert_eq!(txns.len(), 2);

        let active = state.list_active_transactions();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, id2);
    }

    #[test]
    fn test_transaction_limit() {
        let mut state = SessionState::new();

        // Create max transactions
        for i in 0..3 {
            state
                .create_transaction(Some(format!("tx{}", i)), IsolationLevel::ReadCommitted, 3)
                .unwrap();
        }

        // Should fail to create another
        let result = state.create_transaction(None, IsolationLevel::ReadCommitted, 3);
        assert!(result.is_err());
    }

    #[test]
    fn test_isolation_level_parsing() {
        assert_eq!(
            "read_uncommitted".parse(),
            Ok(IsolationLevel::ReadUncommitted)
        );
        assert_eq!("READ-COMMITTED".parse(), Ok(IsolationLevel::ReadCommitted));
        assert_eq!(
            "repeatable read".parse(),
            Ok(IsolationLevel::RepeatableRead)
        );
        assert_eq!("SERIALIZABLE".parse(), Ok(IsolationLevel::Serializable));
        assert_eq!("snapshot".parse(), Ok(IsolationLevel::Snapshot));
        assert!("invalid".parse::<IsolationLevel>().is_err());
    }

    #[test]
    fn test_database_switching() {
        let mut state = SessionState::new();
        assert!(state.current_database().is_none());

        state.set_current_database(Some("TestDB".to_string()));
        assert_eq!(state.current_database(), Some("TestDB"));

        state.set_current_database(None);
        assert!(state.current_database().is_none());
    }
}
