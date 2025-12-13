//! Database connectivity and query execution.

mod connection;
pub mod metadata;
mod query;
mod session;
mod transaction;
pub mod types;

pub use connection::{create_pool, ConnectionManager, ConnectionPool, PooledConn};
pub use metadata::{
    ColumnInfo, DatabaseInfo, FunctionInfo, FunctionParameter, MetadataQueries, ProcedureInfo,
    ProcedureParameter, ServerInfo, TableInfo, TriggerInfo, ViewInfo,
};
pub use query::{ColumnInfo as QueryColumnInfo, QueryExecutor, QueryResult, ResultRow};
pub use session::{SessionInfo, SessionManager};
pub use transaction::TransactionManager;
pub use types::{SqlValue, TypeMapper};
