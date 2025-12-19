//! Integration tests for the MSSQL MCP Server.
//!
//! These tests support two modes:
//! 1. **Testcontainers** (default): Automatically spins up SQL Server containers
//! 2. **External server**: Connect to existing server via MSSQL_HOST env var
//!
//! ## Running with testcontainers (requires Docker):
//! ```bash
//! cargo test --test integration_tests -- --ignored --test-threads=1
//! ```
//!
//! ## Running against external server (e.g., CI service container):
//! ```bash
//! MSSQL_HOST=localhost MSSQL_PORT=1433 MSSQL_PASSWORD='yourPass' \
//!   cargo test --test integration_tests -- --ignored --test-threads=1
//! ```
//!
//! ## Testing against a specific SQL Server version:
//! ```bash
//! MSSQL_TEST_VERSION=2022-latest cargo test --test integration_tests -- --ignored
//! ```
//!
//! Note: SQL Server container requires ~2GB RAM and takes 30-60 seconds to start.

use serial_test::serial;
use std::time::Duration;
use testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt};
use testcontainers_modules::mssql_server::MssqlServer;
use tiberius::{Client, Config};
use tokio::net::TcpStream;
use tokio_util::compat::TokioAsyncWriteCompatExt;

/// Default SA password for testcontainers.
const DEFAULT_SA_PASSWORD: &str = "yourStrong(!)Password";

/// SQL Server versions supported for testing.
/// Version matrix (as of December 2025):
/// - 2025-latest: Current GA release (primary target)
/// - 2022-latest: Previous major version (supported)
/// - 2019-latest: Legacy (extended support only, mainstream ended Feb 2025)
/// - 2017-latest: Legacy (extended support ends Oct 2027)
pub mod versions {
    pub const SQL_SERVER_2025: &str = "2025-latest";
    pub const SQL_SERVER_2022: &str = "2022-latest";
    pub const SQL_SERVER_2019: &str = "2019-latest";
    pub const SQL_SERVER_2017: &str = "2017-latest";

    /// Default version for tests (current GA release)
    pub const DEFAULT: &str = SQL_SERVER_2025;
}

/// Get the SQL Server version to test against.
/// Reads from MSSQL_TEST_VERSION environment variable, defaults to 2025-latest.
fn get_test_version() -> String {
    std::env::var("MSSQL_TEST_VERSION").unwrap_or_else(|_| versions::DEFAULT.to_string())
}

/// Check if we should use an external server (vs testcontainers).
fn use_external_server() -> bool {
    std::env::var("MSSQL_HOST").is_ok()
}

/// Test database connection source.
#[allow(dead_code)] // Variants held for lifetime management (Drop trait)
enum TestDatabaseSource {
    /// External server configured via environment variables.
    External,
    /// Testcontainer-managed SQL Server (boxed to reduce enum size).
    Container(Box<ContainerAsync<MssqlServer>>),
}

/// Helper struct to manage the test database connection.
/// Supports both testcontainers and external servers.
struct TestDatabase {
    #[allow(dead_code)] // Held for lifetime management (Drop trait on Container)
    source: TestDatabaseSource,
    host: String,
    port: u16,
    password: String,
    version: String,
}

impl TestDatabase {
    /// Create a new test database connection.
    /// Uses external server if MSSQL_HOST is set, otherwise uses testcontainers.
    async fn new() -> Self {
        if use_external_server() {
            Self::from_external().await
        } else {
            Self::from_testcontainer(&get_test_version()).await
        }
    }

    /// Connect to an external SQL Server configured via environment variables.
    async fn from_external() -> Self {
        let host = std::env::var("MSSQL_HOST").expect("MSSQL_HOST must be set");
        let port = std::env::var("MSSQL_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(1433);
        let password =
            std::env::var("MSSQL_PASSWORD").unwrap_or_else(|_| DEFAULT_SA_PASSWORD.to_string());
        let version =
            std::env::var("MSSQL_TEST_VERSION").unwrap_or_else(|_| "external".to_string());

        eprintln!(
            "Using external SQL Server at {}:{} (version: {})",
            host, port, version
        );

        Self {
            source: TestDatabaseSource::External,
            host,
            port,
            password,
            version,
        }
    }

    /// Start a new SQL Server test container with a specific version.
    async fn from_testcontainer(version: &str) -> Self {
        eprintln!(
            "Starting SQL Server {} container via testcontainers...",
            version
        );

        let container = MssqlServer::default()
            .with_accept_eula()
            .with_tag(version)
            .start()
            .await
            .unwrap_or_else(|e| panic!("Failed to start SQL Server {} container: {}", version, e));

        let host = container.get_host().await.expect("Failed to get host");
        let port = container
            .get_host_port_ipv4(1433)
            .await
            .expect("Failed to get port");

        // Wait a bit for SQL Server to fully initialize
        tokio::time::sleep(Duration::from_secs(5)).await;

        eprintln!(
            "SQL Server {} container ready at {}:{}",
            version, host, port
        );

        Self {
            source: TestDatabaseSource::Container(Box::new(container)),
            host: host.to_string(),
            port,
            password: DEFAULT_SA_PASSWORD.to_string(),
            version: version.to_string(),
        }
    }

    /// Get the SQL Server version this connection is using.
    #[allow(dead_code)]
    fn version(&self) -> &str {
        &self.version
    }

    /// Create a tiberius client connected to the test database.
    async fn connect(&self) -> Client<tokio_util::compat::Compat<TcpStream>> {
        let mut config = Config::new();
        config.host(&self.host);
        config.port(self.port);
        config.authentication(tiberius::AuthMethod::sql_server("sa", &self.password));
        config.trust_cert();

        let tcp = TcpStream::connect(format!("{}:{}", self.host, self.port))
            .await
            .expect("Failed to connect to TCP");
        tcp.set_nodelay(true).expect("Failed to set TCP_NODELAY");

        Client::connect(config, tcp.compat_write())
            .await
            .expect("Failed to connect to SQL Server")
    }
}

// Type alias for backward compatibility in tests
type TestContainer = TestDatabase;

// =============================================================================
// Connection Tests
// =============================================================================

mod connection_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires Docker"]
    #[serial]
    async fn test_database_connection() {
        let container = TestContainer::new().await;
        let mut client = container.connect().await;

        // Simple connectivity test
        let stream = client
            .simple_query("SELECT 1 AS test")
            .await
            .expect("Query failed");

        let results = stream.into_results().await.expect("Failed to get results");
        assert!(!results.is_empty(), "Expected at least one result set");

        let first_set = &results[0];
        assert_eq!(first_set.len(), 1, "Expected one row");

        let value: i32 = first_set[0].get(0).expect("Failed to get column value");
        assert_eq!(value, 1);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    #[serial]
    async fn test_database_version() {
        let container = TestContainer::new().await;
        let mut client = container.connect().await;

        let stream = client
            .simple_query("SELECT @@VERSION")
            .await
            .expect("Query failed");

        let results = stream.into_results().await.expect("Failed to get results");
        let version: &str = results[0][0].get(0).expect("Failed to get version");

        assert!(
            version.contains("Microsoft SQL Server"),
            "Version should mention SQL Server"
        );
    }
}

// =============================================================================
// Query Tests
// =============================================================================

mod query_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires Docker"]
    #[serial]
    async fn test_simple_query() {
        let container = TestContainer::new().await;
        let mut client = container.connect().await;

        // Test basic SELECT with multiple columns
        let stream = client
            .simple_query("SELECT 1 AS num, 'hello' AS text, GETDATE() AS ts")
            .await
            .expect("Query failed");

        let results = stream.into_results().await.expect("Failed to get results");
        assert_eq!(results[0].len(), 1);

        let num: i32 = results[0][0].get(0).expect("Failed to get num");
        let text: &str = results[0][0].get(1).expect("Failed to get text");

        assert_eq!(num, 1);
        assert_eq!(text, "hello");
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    #[serial]
    async fn test_create_and_query_table() {
        let container = TestContainer::new().await;
        let mut client = container.connect().await;

        // Create a test table
        client
            .simple_query(
                "CREATE TABLE #test_table (
                    id INT PRIMARY KEY,
                    name NVARCHAR(100),
                    value DECIMAL(10,2)
                )",
            )
            .await
            .expect("Create table failed")
            .into_results()
            .await
            .expect("Failed to get results");

        // Insert test data
        client
            .simple_query(
                "INSERT INTO #test_table (id, name, value) VALUES
                (1, N'Alice', 100.50),
                (2, N'Bob', 200.75),
                (3, N'Charlie', 300.00)",
            )
            .await
            .expect("Insert failed")
            .into_results()
            .await
            .expect("Failed to get results");

        // Query the data
        let stream = client
            .simple_query("SELECT id, name, value FROM #test_table ORDER BY id")
            .await
            .expect("Select failed");

        let results = stream.into_results().await.expect("Failed to get results");
        assert_eq!(results[0].len(), 3, "Expected 3 rows");

        let first_name: &str = results[0][0].get(1).expect("Failed to get name");
        assert_eq!(first_name, "Alice");
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    #[serial]
    async fn test_null_handling() {
        let container = TestContainer::new().await;
        let mut client = container.connect().await;

        let stream = client
            .simple_query("SELECT NULL AS null_col, 'not null' AS text_col")
            .await
            .expect("Query failed");

        let results = stream.into_results().await.expect("Failed to get results");

        let null_val: Option<&str> = results[0][0].get(0);
        let text_val: Option<&str> = results[0][0].get(1);

        assert!(null_val.is_none(), "Expected NULL");
        assert_eq!(text_val, Some("not null"));
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    #[serial]
    async fn test_data_types() {
        let container = TestContainer::new().await;
        let mut client = container.connect().await;

        let stream = client
            .simple_query(
                "SELECT
                    CAST(42 AS INT) AS int_val,
                    CAST(1.2345 AS FLOAT) AS float_val,
                    CAST(123.45 AS DECIMAL(10,2)) AS decimal_val,
                    CAST('2024-01-15' AS DATE) AS date_val,
                    CAST(1 AS BIT) AS bit_val,
                    N'Unicode: ' AS nvarchar_val",
            )
            .await
            .expect("Query failed");

        let results = stream.into_results().await.expect("Failed to get results");
        assert_eq!(results[0].len(), 1);

        // Verify int
        let int_val: i32 = results[0][0].get(0).expect("Failed to get int");
        assert_eq!(int_val, 42);

        // Verify float
        let float_val: f64 = results[0][0].get(1).expect("Failed to get float");
        assert!((float_val - 1.2345).abs() < 0.0001);

        // Verify bit
        let bit_val: bool = results[0][0].get(4).expect("Failed to get bit");
        assert!(bit_val);
    }
}

// =============================================================================
// Transaction Tests
// =============================================================================

mod transaction_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires Docker"]
    #[serial]
    async fn test_transaction_commit() {
        let container = TestContainer::new().await;
        let mut client = container.connect().await;

        // Create table
        client
            .simple_query("CREATE TABLE #tx_test (id INT PRIMARY KEY, value INT)")
            .await
            .expect("Create table failed")
            .into_results()
            .await
            .expect("Failed");

        // Begin transaction
        client
            .simple_query("BEGIN TRANSACTION")
            .await
            .expect("Begin failed")
            .into_results()
            .await
            .expect("Failed");

        // Insert data
        client
            .simple_query("INSERT INTO #tx_test VALUES (1, 100)")
            .await
            .expect("Insert failed")
            .into_results()
            .await
            .expect("Failed");

        // Commit
        client
            .simple_query("COMMIT TRANSACTION")
            .await
            .expect("Commit failed")
            .into_results()
            .await
            .expect("Failed");

        // Verify data persisted
        let stream = client
            .simple_query("SELECT value FROM #tx_test WHERE id = 1")
            .await
            .expect("Select failed");

        let results = stream.into_results().await.expect("Failed");
        assert_eq!(results[0].len(), 1);

        let value: i32 = results[0][0].get(0).expect("Failed");
        assert_eq!(value, 100);
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    #[serial]
    async fn test_transaction_rollback() {
        let container = TestContainer::new().await;
        let mut client = container.connect().await;

        // Create table with initial data
        client
            .simple_query(
                "CREATE TABLE #rollback_test (id INT PRIMARY KEY, value INT);
                 INSERT INTO #rollback_test VALUES (1, 100);",
            )
            .await
            .expect("Setup failed")
            .into_results()
            .await
            .expect("Failed");

        // Begin transaction
        client
            .simple_query("BEGIN TRANSACTION")
            .await
            .expect("Begin failed")
            .into_results()
            .await
            .expect("Failed");

        // Update data
        client
            .simple_query("UPDATE #rollback_test SET value = 999 WHERE id = 1")
            .await
            .expect("Update failed")
            .into_results()
            .await
            .expect("Failed");

        // Rollback
        client
            .simple_query("ROLLBACK TRANSACTION")
            .await
            .expect("Rollback failed")
            .into_results()
            .await
            .expect("Failed");

        // Verify data was rolled back
        let stream = client
            .simple_query("SELECT value FROM #rollback_test WHERE id = 1")
            .await
            .expect("Select failed");

        let results = stream.into_results().await.expect("Failed");
        let value: i32 = results[0][0].get(0).expect("Failed");
        assert_eq!(value, 100, "Value should be unchanged after rollback");
    }
}

// =============================================================================
// Metadata Tests
// =============================================================================

mod metadata_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires Docker"]
    #[serial]
    async fn test_list_databases() {
        let container = TestContainer::new().await;
        let mut client = container.connect().await;

        let stream = client
            .simple_query("SELECT name FROM sys.databases ORDER BY name")
            .await
            .expect("Query failed");

        let results = stream.into_results().await.expect("Failed");
        assert!(!results[0].is_empty(), "Should have at least one database");

        // Check for system databases
        let db_names: Vec<&str> = results[0]
            .iter()
            .filter_map(|row| row.get::<&str, _>(0))
            .collect();

        assert!(db_names.contains(&"master"), "Should have master database");
        assert!(db_names.contains(&"tempdb"), "Should have tempdb database");
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    #[serial]
    async fn test_list_tables() {
        let container = TestContainer::new().await;
        let mut client = container.connect().await;

        let stream = client
            .simple_query(
                "SELECT TABLE_SCHEMA, TABLE_NAME
                 FROM INFORMATION_SCHEMA.TABLES
                 WHERE TABLE_TYPE = 'BASE TABLE'
                 ORDER BY TABLE_SCHEMA, TABLE_NAME",
            )
            .await
            .expect("Query failed");

        let results = stream.into_results().await.expect("Failed");
        // master database has some system tables
        assert!(
            !results[0].is_empty(),
            "Should have at least some system tables"
        );
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    #[serial]
    async fn test_column_metadata() {
        let container = TestContainer::new().await;
        let mut client = container.connect().await;

        // Create a test table
        client
            .simple_query(
                "CREATE TABLE #meta_test (
                    id INT NOT NULL PRIMARY KEY,
                    name NVARCHAR(100) NULL,
                    created_at DATETIME2 DEFAULT GETDATE()
                )",
            )
            .await
            .expect("Create failed")
            .into_results()
            .await
            .expect("Failed");

        // Query column metadata
        let stream = client
            .simple_query(
                "SELECT
                    COLUMN_NAME,
                    DATA_TYPE,
                    IS_NULLABLE,
                    CHARACTER_MAXIMUM_LENGTH
                 FROM tempdb.INFORMATION_SCHEMA.COLUMNS
                 WHERE TABLE_NAME LIKE '#meta_test%'
                 ORDER BY ORDINAL_POSITION",
            )
            .await
            .expect("Query failed");

        let results = stream.into_results().await.expect("Failed");
        assert_eq!(results[0].len(), 3, "Should have 3 columns");

        let first_col_name: &str = results[0][0].get(0).expect("Failed");
        assert_eq!(first_col_name, "id");
    }
}

// =============================================================================
// Error Handling Tests
// =============================================================================

mod error_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires Docker"]
    #[serial]
    async fn test_invalid_query_syntax() {
        let container = TestContainer::new().await;
        let mut client = container.connect().await;

        let result = client.simple_query("SELEC invalid syntax").await;

        assert!(result.is_err(), "Invalid SQL should return error");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("Incorrect syntax")
                || err.to_string().contains("could not find")
                || err.to_string().contains("error"),
            "Error should mention syntax issue: {}",
            err
        );
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    #[serial]
    async fn test_table_not_found() {
        let container = TestContainer::new().await;
        let mut client = container.connect().await;

        let result = client
            .simple_query("SELECT * FROM nonexistent_table_xyz")
            .await;

        assert!(result.is_err(), "Query on non-existent table should fail");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("Invalid object name")
                || err.to_string().contains("does not exist"),
            "Error should mention invalid object: {}",
            err
        );
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    #[serial]
    async fn test_constraint_violation() {
        let container = TestContainer::new().await;
        let mut client = container.connect().await;

        // Create table with primary key
        client
            .simple_query("CREATE TABLE #pk_test (id INT PRIMARY KEY)")
            .await
            .expect("Create failed")
            .into_results()
            .await
            .expect("Failed");

        // Insert first row
        client
            .simple_query("INSERT INTO #pk_test VALUES (1)")
            .await
            .expect("First insert failed")
            .into_results()
            .await
            .expect("Failed");

        // Try to insert duplicate key
        let result = client.simple_query("INSERT INTO #pk_test VALUES (1)").await;

        assert!(result.is_err(), "Duplicate key insert should fail");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("PRIMARY KEY")
                || err.to_string().contains("duplicate")
                || err.to_string().contains("Violation"),
            "Error should mention primary key violation: {}",
            err
        );
    }
}

// =============================================================================
// Performance/Bulk Tests
// =============================================================================

mod performance_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires Docker"]
    #[serial]
    async fn test_bulk_insert() {
        let container = TestContainer::new().await;
        let mut client = container.connect().await;

        // Create table
        client
            .simple_query("CREATE TABLE #bulk_test (id INT, value VARCHAR(100))")
            .await
            .expect("Create failed")
            .into_results()
            .await
            .expect("Failed");

        // Build bulk insert statement
        let mut values = Vec::new();
        for i in 0..100 {
            values.push(format!("({}, 'value_{}')", i, i));
        }
        let insert_sql = format!("INSERT INTO #bulk_test VALUES {}", values.join(","));

        client
            .simple_query(&insert_sql)
            .await
            .expect("Bulk insert failed")
            .into_results()
            .await
            .expect("Failed");

        // Verify count
        let stream = client
            .simple_query("SELECT COUNT(*) FROM #bulk_test")
            .await
            .expect("Count failed");

        let results = stream.into_results().await.expect("Failed");
        let count: i32 = results[0][0].get(0).expect("Failed");
        assert_eq!(count, 100, "Should have inserted 100 rows");
    }

    #[tokio::test]
    #[ignore = "requires Docker"]
    #[serial]
    async fn test_large_result_set() {
        let container = TestContainer::new().await;
        let mut client = container.connect().await;

        // Generate a large result set using recursive CTE
        let stream = client
            .simple_query(
                "WITH nums AS (
                    SELECT 1 AS n
                    UNION ALL
                    SELECT n + 1 FROM nums WHERE n < 500
                )
                SELECT n FROM nums
                OPTION (MAXRECURSION 500)",
            )
            .await
            .expect("Query failed");

        let results = stream.into_results().await.expect("Failed");
        assert_eq!(results[0].len(), 500, "Should have 500 rows");
    }
}
