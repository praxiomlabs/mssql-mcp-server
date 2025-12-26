//! Database connectivity and query execution.

mod auth;
mod connection;
pub mod metadata;
mod query;
mod session;
mod transaction;
pub mod types;

pub use auth::{create_connection, truncate_for_log, RawConnection};
pub use connection::{create_pool, pool_status, ConnectionPool, PoolStatus, PooledConn};
pub use metadata::{
    ColumnInfo, DatabaseInfo, FunctionInfo, FunctionParameter, MetadataQueries, ProcedureInfo,
    ProcedureParameter, ServerInfo, TableInfo, TriggerInfo, ViewInfo,
};
pub use query::{ColumnInfo as QueryColumnInfo, QueryExecutor, QueryResult, ResultRow};
pub use session::{SessionInfo, SessionManager};
pub use transaction::TransactionManager;
pub use types::{SqlValue, TypeMapper};
