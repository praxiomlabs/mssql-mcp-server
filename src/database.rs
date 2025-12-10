//! Database connectivity and query execution.

mod connection;
pub mod metadata;
mod query;
pub mod types;

pub use connection::{create_pool, ConnectionManager, ConnectionPool, PooledConn};
pub use metadata::{
    ColumnInfo, DatabaseInfo, MetadataQueries, ProcedureInfo, ProcedureParameter, ServerInfo,
    TableInfo, ViewInfo,
};
pub use query::{ColumnInfo as QueryColumnInfo, QueryExecutor, QueryResult, ResultRow};
pub use types::{SqlValue, TypeMapper};
