//! Authentication helpers for SQL Server connections.
//!
//! This module provides unified authentication handling for SQL Server
//! connections, supporting:
//! - SQL Server authentication (username/password)
//! - Windows authentication (SSPI/Kerberos)
//! - Azure AD authentication (service principal with client credentials)

use crate::config::{AuthConfig, DatabaseConfig};
use crate::error::McpError;
use tiberius::{AuthMethod, Client, Config, EncryptionLevel};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};
use tracing::debug;

/// Type alias for a raw tiberius connection.
pub type RawConnection = Client<Compat<TcpStream>>;

/// SQL Server resource URI for Azure AD token acquisition.
/// This is the standard resource URI for Azure SQL Database.
/// Reserved for future Azure AD token refresh implementation.
#[allow(dead_code)]
const AZURE_SQL_RESOURCE: &str = "https://database.windows.net/";

/// Configure tiberius authentication method based on AuthConfig.
///
/// For Azure AD, this acquires a fresh access token using the service principal credentials.
pub async fn configure_auth(config: &Config, auth: &AuthConfig) -> Result<Config, McpError> {
    let mut config = config.clone();

    match auth {
        AuthConfig::SqlServer { username, password } => {
            config.authentication(AuthMethod::sql_server(username, password));
            Ok(config)
        }
        #[cfg(windows)]
        AuthConfig::Windows => {
            config.authentication(AuthMethod::Integrated);
            Ok(config)
        }
        AuthConfig::AzureAd {
            client_id,
            client_secret,
            tenant_id,
        } => {
            #[cfg(feature = "azure-auth")]
            {
                let token = acquire_azure_ad_token(client_id, client_secret, tenant_id).await?;
                config.authentication(AuthMethod::aad_token(token));
                Ok(config)
            }
            #[cfg(not(feature = "azure-auth"))]
            {
                // Silence unused variable warnings
                let _ = (client_id, client_secret, tenant_id);
                Err(McpError::config(
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
) -> Result<String, McpError> {
    use azure_core::auth::TokenCredential;
    use azure_identity::ClientSecretCredential;

    debug!(
        "Acquiring Azure AD token for client_id: {}",
        &client_id[..8.min(client_id.len())]
    );

    // Authority host for Azure AD
    let authority_host: azure_core::Url =
        format!("https://login.microsoftonline.com/{}", tenant_id)
            .parse()
            .map_err(|e| McpError::Authentication(format!("Invalid tenant ID URL: {}", e)))?;

    // Create HTTP client for token requests
    let http_client = azure_core::new_http_client();

    // Create the client secret credential
    let credential = ClientSecretCredential::new(
        http_client,
        authority_host,
        tenant_id.to_string(),
        client_id.to_string(),
        client_secret.to_string(),
    );

    // Request token for Azure SQL Database resource
    // The scope for Azure SQL is the resource URL with /.default suffix
    let token_response = credential
        .get_token(&[AZURE_SQL_RESOURCE])
        .await
        .map_err(|e| {
            McpError::Authentication(format!("Failed to acquire Azure AD token: {}", e))
        })?;

    debug!("Azure AD token acquired successfully");
    Ok(token_response.token.secret().to_string())
}

/// Create a tiberius Config from DatabaseConfig.
///
/// This sets up the basic connection configuration (host, port, database, encryption)
/// but does NOT configure authentication - use `configure_auth` for that.
pub fn create_base_config(db_config: &DatabaseConfig) -> Config {
    let mut config = Config::new();

    // Set host and port
    config.host(&db_config.host);
    config.port(db_config.port);

    // Set database if specified
    if let Some(ref database) = db_config.database {
        config.database(database);
    }

    // Configure encryption
    if db_config.encrypt {
        config.encryption(EncryptionLevel::Required);
    } else {
        config.encryption(EncryptionLevel::Off);
    }

    // Trust server certificate if requested
    if db_config.trust_server_certificate {
        config.trust_cert();
    }

    // Set application name
    config.application_name(&db_config.application_name);

    config
}

/// Create a tiberius Config with a custom application name suffix.
///
/// This is useful for distinguishing different connection types in SQL Server logs.
pub fn create_base_config_with_suffix(db_config: &DatabaseConfig, suffix: &str) -> Config {
    let mut config = create_base_config(db_config);
    config.application_name(format!("{}-{}", db_config.application_name, suffix));
    config
}

/// Create a raw connection to SQL Server.
///
/// This is a convenience function that handles the full connection flow:
/// 1. Creates base configuration
/// 2. Configures authentication (including Azure AD token acquisition if needed)
/// 3. Establishes TCP connection
/// 4. Performs TDS handshake
///
/// # Arguments
/// * `db_config` - Database configuration including auth settings
/// * `app_name_suffix` - Optional suffix for application name (e.g., "session", "txn")
pub async fn create_connection(
    db_config: &DatabaseConfig,
    app_name_suffix: Option<&str>,
) -> Result<RawConnection, McpError> {
    // Create base config with optional suffix
    let base_config = match app_name_suffix {
        Some(suffix) => create_base_config_with_suffix(db_config, suffix),
        None => create_base_config(db_config),
    };

    // Configure authentication (may acquire Azure AD token)
    let config = configure_auth(&base_config, &db_config.auth).await?;

    // Establish TCP connection
    let address = format!("{}:{}", db_config.host, db_config.port);
    debug!("Creating connection to {}", address);

    let tcp = TcpStream::connect(&address)
        .await
        .map_err(|e| McpError::connection(format!("Failed to connect to {}: {}", address, e)))?;

    tcp.set_nodelay(true)
        .map_err(|e| McpError::connection(format!("Failed to set TCP_NODELAY: {}", e)))?;

    // Perform TDS handshake
    let client = Client::connect(config, tcp.compat_write())
        .await
        .map_err(|e| McpError::connection(format!("Failed to connect to SQL Server: {}", e)))?;

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
    use crate::config::PoolConfig;

    fn test_db_config() -> DatabaseConfig {
        DatabaseConfig {
            host: "localhost".to_string(),
            port: 1433,
            database: Some("master".to_string()),
            auth: AuthConfig::SqlServer {
                username: "sa".to_string(),
                password: "test".to_string(),
            },
            pool: PoolConfig::default(),
            encrypt: false,
            trust_server_certificate: true,
            application_name: "test".to_string(),
        }
    }

    #[test]
    fn test_create_base_config() {
        let db_config = test_db_config();
        let _config = create_base_config(&db_config);
        // Config doesn't expose getters, so we just verify it doesn't panic
    }

    #[test]
    fn test_create_base_config_with_suffix() {
        let db_config = test_db_config();
        let _config = create_base_config_with_suffix(&db_config, "session");
        // Config doesn't expose getters, so we just verify it doesn't panic
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
