//! Authentication helpers for SQL Server connections.
//!
//! This module provides unified authentication handling for SQL Server
//! connections, supporting:
//! - SQL Server authentication (username/password)
//! - Windows authentication (SSPI/Kerberos)
//! - Azure AD authentication (service principal with client credentials)

use crate::config::{AuthConfig, DatabaseConfig, TdsVersionConfig};
use crate::error::ServerError;
use mssql_client::{Client, Config, Credentials, Ready, RetryPolicy, TdsVersion, TimeoutConfig};
use std::time::Duration;
use tracing::debug;

/// Convert our TdsVersionConfig to mssql_client's TdsVersion.
fn convert_tds_version(version: TdsVersionConfig) -> TdsVersion {
    match version {
        TdsVersionConfig::V7_3A => TdsVersion::V7_3A,
        TdsVersionConfig::V7_3B => TdsVersion::V7_3B,
        TdsVersionConfig::V7_4 => TdsVersion::V7_4,
        TdsVersionConfig::V8_0 => TdsVersion::V8_0,
    }
}

/// Type alias for a raw mssql-client connection in Ready state.
pub type RawConnection = Client<Ready>;

/// SQL Server resource URI for Azure AD token acquisition.
/// This is the standard resource URI for Azure SQL Database.
#[cfg(feature = "azure-auth")]
const AZURE_SQL_RESOURCE: &str = "https://database.windows.net/";

/// Build credentials based on AuthConfig.
///
/// For Azure AD, this acquires a fresh access token using the service principal credentials.
pub async fn build_credentials(auth: &AuthConfig) -> Result<Credentials, ServerError> {
    match auth {
        AuthConfig::SqlServer { username, password } => {
            Ok(Credentials::sql_server(username.clone(), password.clone()))
        }
        #[cfg(windows)]
        AuthConfig::Windows => {
            // Note: Integrated authentication requires the 'integrated-auth' feature
            // on mssql-client, which is platform-specific
            Err(ServerError::config(
                "Windows integrated authentication requires platform-specific setup. \
                 Consider using SQL Server or Azure AD authentication.",
            ))
        }
        AuthConfig::AzureAd {
            client_id,
            client_secret,
            tenant_id,
        } => {
            #[cfg(feature = "azure-auth")]
            {
                let token = acquire_azure_ad_token(client_id, client_secret, tenant_id).await?;
                Ok(Credentials::azure_token(token))
            }
            #[cfg(not(feature = "azure-auth"))]
            {
                // Silence unused variable warnings
                let _ = (client_id, client_secret, tenant_id);
                Err(ServerError::config(
                    "Azure AD authentication requires the 'azure-auth' feature. \
                     Rebuild with: cargo build --features azure-auth",
                ))
            }
        }
    }
}

/// Acquire an Azure AD access token for SQL Server using client credentials flow.
#[cfg(feature = "azure-auth")]
async fn acquire_azure_ad_token(
    client_id: &str,
    client_secret: &str,
    tenant_id: &str,
) -> Result<String, ServerError> {
    use azure_core::credentials::{Secret, TokenCredential};
    use azure_identity::ClientSecretCredential;

    debug!(
        "Acquiring Azure AD token for client_id: {}",
        &client_id[..8.min(client_id.len())]
    );

    // Create the client secret credential with the modern API
    let credential = ClientSecretCredential::new(
        tenant_id,
        client_id.to_string(),
        Secret::new(client_secret.to_string()),
        None, // Use default options
    )
    .map_err(|e| ServerError::auth(format!("Failed to create credential: {}", e)))?;

    // Request token for Azure SQL Database resource
    // The scope for Azure SQL is the resource URL with /.default suffix
    let token_response = credential
        .get_token(&[AZURE_SQL_RESOURCE], None)
        .await
        .map_err(|e| ServerError::auth(format!("Failed to acquire Azure AD token: {}", e)))?;

    debug!("Azure AD token acquired successfully");
    Ok(token_response.token.secret().to_string())
}

/// Create a mssql-client Config from DatabaseConfig.
///
/// This sets up the connection configuration including host, port, database,
/// encryption settings, authentication credentials, retry policy, and instance.
pub async fn create_config(db_config: &DatabaseConfig) -> Result<Config, ServerError> {
    let credentials = build_credentials(&db_config.auth).await?;

    // Build retry policy from configuration
    let retry_policy = RetryPolicy::new()
        .max_retries(db_config.retry.max_retries)
        .initial_backoff(Duration::from_millis(db_config.retry.initial_backoff_ms))
        .max_backoff(Duration::from_millis(db_config.retry.max_backoff_ms))
        .backoff_multiplier(db_config.retry.backoff_multiplier)
        .jitter(db_config.retry.jitter);

    // Build timeout config from our configuration
    let timeout_config = TimeoutConfig::new()
        .connect_timeout(db_config.timeouts.connect_timeout)
        .tls_timeout(db_config.timeouts.tls_timeout)
        .login_timeout(db_config.timeouts.login_timeout)
        .command_timeout(db_config.timeouts.command_timeout)
        .idle_timeout(db_config.timeouts.idle_timeout)
        .keepalive_interval(db_config.timeouts.keepalive_interval);

    let mut config = Config::new()
        .host(&db_config.host)
        .port(db_config.port)
        .credentials(credentials)
        .application_name(&db_config.application_name)
        .trust_server_certificate(db_config.trust_server_certificate)
        .encrypt(db_config.encrypt)
        .retry(retry_policy)
        .timeouts(timeout_config)
        .tds_version(convert_tds_version(db_config.tds_version));

    // Set database if specified
    if let Some(ref database) = db_config.database {
        config = config.database(database);
    }

    // Set named instance if specified
    if let Some(ref instance) = db_config.instance {
        config.instance = Some(instance.clone());
    }

    // Set MARS if enabled
    config.mars = db_config.mars;

    Ok(config)
}

/// Create a mssql-client Config with a custom application name suffix.
///
/// This is useful for distinguishing different connection types in SQL Server logs.
pub async fn create_config_with_suffix(
    db_config: &DatabaseConfig,
    suffix: &str,
) -> Result<Config, ServerError> {
    let credentials = build_credentials(&db_config.auth).await?;

    let app_name = format!("{}-{}", db_config.application_name, suffix);

    // Build retry policy from configuration
    let retry_policy = RetryPolicy::new()
        .max_retries(db_config.retry.max_retries)
        .initial_backoff(Duration::from_millis(db_config.retry.initial_backoff_ms))
        .max_backoff(Duration::from_millis(db_config.retry.max_backoff_ms))
        .backoff_multiplier(db_config.retry.backoff_multiplier)
        .jitter(db_config.retry.jitter);

    // Build timeout config from our configuration
    let timeout_config = TimeoutConfig::new()
        .connect_timeout(db_config.timeouts.connect_timeout)
        .tls_timeout(db_config.timeouts.tls_timeout)
        .login_timeout(db_config.timeouts.login_timeout)
        .command_timeout(db_config.timeouts.command_timeout)
        .idle_timeout(db_config.timeouts.idle_timeout)
        .keepalive_interval(db_config.timeouts.keepalive_interval);

    let mut config = Config::new()
        .host(&db_config.host)
        .port(db_config.port)
        .credentials(credentials)
        .application_name(&app_name)
        .trust_server_certificate(db_config.trust_server_certificate)
        .encrypt(db_config.encrypt)
        .retry(retry_policy)
        .timeouts(timeout_config)
        .tds_version(convert_tds_version(db_config.tds_version));

    // Set database if specified
    if let Some(ref database) = db_config.database {
        config = config.database(database);
    }

    // Set named instance if specified
    if let Some(ref instance) = db_config.instance {
        config.instance = Some(instance.clone());
    }

    // Set MARS if enabled
    config.mars = db_config.mars;

    Ok(config)
}

/// Create a raw connection to SQL Server.
///
/// This is a convenience function that handles the full connection flow:
/// 1. Creates configuration with authentication
/// 2. Establishes connection to SQL Server
///
/// # Arguments
/// * `db_config` - Database configuration including auth settings
/// * `app_name_suffix` - Optional suffix for application name (e.g., "session", "txn")
pub async fn create_connection(
    db_config: &DatabaseConfig,
    app_name_suffix: Option<&str>,
) -> Result<RawConnection, ServerError> {
    // Create config with optional suffix
    let config = match app_name_suffix {
        Some(suffix) => create_config_with_suffix(db_config, suffix).await?,
        None => create_config(db_config).await?,
    };

    // Establish connection
    let address = format!("{}:{}", db_config.host, db_config.port);
    debug!("Creating connection to {}", address);

    let client = Client::connect(config)
        .await
        .map_err(|e| ServerError::connection(format!("Failed to connect to SQL Server: {}", e)))?;

    debug!("Connection established successfully");
    Ok(client)
}

/// Truncate a string for logging purposes.
///
/// This is a shared utility for safe logging of potentially long strings.
pub fn truncate_for_log(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{PoolConfig, RetryConfig, TimeoutsConfig};

    fn test_db_config() -> DatabaseConfig {
        DatabaseConfig {
            host: "localhost".to_string(),
            port: 1433,
            instance: None,
            database: Some("master".to_string()),
            auth: AuthConfig::SqlServer {
                username: "sa".to_string(),
                password: "test".to_string(),
            },
            pool: PoolConfig::default(),
            timeouts: TimeoutsConfig::default(),
            encrypt: false,
            trust_server_certificate: true,
            application_name: "test".to_string(),
            mars: false,
            retry: RetryConfig::default(),
            tds_version: TdsVersionConfig::default(),
        }
    }

    #[tokio::test]
    async fn test_build_credentials() {
        let auth = AuthConfig::SqlServer {
            username: "sa".to_string(),
            password: "test".to_string(),
        };
        let creds = build_credentials(&auth).await;
        assert!(creds.is_ok());
    }

    #[tokio::test]
    async fn test_create_config() {
        let db_config = test_db_config();
        let config = create_config(&db_config).await;
        assert!(config.is_ok());
    }

    #[tokio::test]
    async fn test_create_config_with_suffix() {
        let db_config = test_db_config();
        let config = create_config_with_suffix(&db_config, "session").await;
        assert!(config.is_ok());
    }

    #[test]
    fn test_truncate_for_log() {
        assert_eq!(truncate_for_log("short", 10), "short");
        assert_eq!(
            truncate_for_log("this is a long string", 10),
            "this is a ..."
        );
        assert_eq!(truncate_for_log("exactly10!", 10), "exactly10!");
    }
}
