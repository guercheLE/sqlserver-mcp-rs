// SQL Server 2025 - master/msdb/sandbox combined catalog MCP server.
//
// `sqlAuth` (SQL Server Authentication: a SQL login validated by the engine
// itself — see docs/sqlserver-eda-openapi-pipeline/README.md's
// `securitySchemes` documentation). Adapted from mcpify's originally
// generated `strategies::basic` (the OpenAPI `http`/`basic` scheme this
// maps from), minus the HTTP `Authorization: Basic ...` header encoding
// that scheme normally implies — this project's transport
// (`services::api_client`) reads `username`/`password` directly to build a
// `tiberius::AuthMethod::sql_server(...)`, not an HTTP header.

use async_trait::async_trait;

use super::super::auth_strategy::{AuthConfig, AuthStrategy, Credentials};
use super::super::errors::AuthError;

#[derive(Debug, Default)]
pub struct SqlServerAuthStrategy;

#[async_trait]
impl AuthStrategy for SqlServerAuthStrategy {
    async fn authenticate(&self, config: &AuthConfig) -> anyhow::Result<Credentials> {
        let username = config
            .get("username")
            .ok_or_else(|| AuthError::MissingCredentials("username, password".to_string()))?;
        let password = config
            .get("password")
            .ok_or_else(|| AuthError::MissingCredentials("username, password".to_string()))?;

        let mut credentials = Credentials::new();
        credentials.insert("username".to_string(), username.clone());
        credentials.insert("password".to_string(), password.clone());
        Ok(credentials)
    }

    fn validate_credentials(&self, credentials: &Credentials) -> bool {
        credentials
            .get("username")
            .is_some_and(|username| !username.is_empty())
            && credentials
                .get("password")
                .is_some_and(|password| !password.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(username: &str, password: &str) -> AuthConfig {
        AuthConfig::from([
            ("username".to_string(), username.to_string()),
            ("password".to_string(), password.to_string()),
        ])
    }

    #[tokio::test]
    async fn carries_username_and_password_through_unchanged() {
        let strategy = SqlServerAuthStrategy;
        let credentials = strategy
            .authenticate(&config("sa", "s3cr3t"))
            .await
            .unwrap();
        assert_eq!(credentials.get("username").unwrap(), "sa");
        assert_eq!(credentials.get("password").unwrap(), "s3cr3t");
    }

    #[tokio::test]
    async fn rejects_a_config_missing_either_field() {
        let strategy = SqlServerAuthStrategy;
        assert!(strategy.authenticate(&AuthConfig::new()).await.is_err());
    }

    #[test]
    fn validates_credentials_carrying_both_fields() {
        let strategy = SqlServerAuthStrategy;
        assert!(strategy.validate_credentials(&config("sa", "s3cr3t")));
        assert!(!strategy.validate_credentials(&Credentials::new()));
    }
}
