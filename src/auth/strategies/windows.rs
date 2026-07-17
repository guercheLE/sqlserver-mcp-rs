// SQL Server 2025 - master/msdb/sandbox combined catalog MCP server.
//
// `windowsAuth` (Windows Authentication / Integrated Security — see
// docs/sqlserver-eda-openapi-pipeline/README.md's `securitySchemes`
// documentation). Not discovered by mcpify's own OpenAPI-auth-scheme
// classifier (it only recognizes `http`/`basic`/`bearer` and `oauth2`
// shapes; `windowsAuth`'s `http`/`negotiate` scheme isn't one of them —
// see `core::config_schema::AuthMethod`'s doc comment), so this strategy
// was added by hand.
//
// `tiberius::AuthMethod::windows` is only compiled when the `winauth`
// feature is enabled *and* the target OS is Windows (native SSPI) — see
// `tiberius`'s `client::auth` module. On Linux/macOS (this pipeline's
// primary target — see docs/sqlserver-eda-openapi-pipeline/docker-compose.yml,
// which runs SQL Server in Linux containers), this strategy still resolves
// credentials the same way (env vars / OS keychain / prompt), but
// `services::api_client` cannot actually hand them to `tiberius` on those
// platforms: there is no non-Windows NTLM/SSPI implementation without also
// enabling `integrated-auth-gssapi` (Kerberos via `libgssapi`, a system
// library this project doesn't currently link) — see
// `auth::auth_manager::AuthManager::resolve_tds_auth`, which surfaces that
// as a clear runtime error rather than silently falling back to another
// auth mode.

use async_trait::async_trait;

use super::super::auth_strategy::{AuthConfig, AuthStrategy, Credentials};
use super::super::errors::AuthError;

#[derive(Debug, Default)]
pub struct WindowsAuthStrategy;

#[async_trait]
impl AuthStrategy for WindowsAuthStrategy {
    async fn authenticate(&self, config: &AuthConfig) -> anyhow::Result<Credentials> {
        let username = config.get("username").ok_or_else(|| {
            AuthError::MissingCredentials("username (optionally DOMAIN\\user), password".to_string())
        })?;
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

    #[tokio::test]
    async fn carries_a_domain_qualified_username_through_unchanged() {
        let strategy = WindowsAuthStrategy;
        let config = AuthConfig::from([
            ("username".to_string(), "CORP\\alice".to_string()),
            ("password".to_string(), "s3cr3t".to_string()),
        ]);
        let credentials = strategy.authenticate(&config).await.unwrap();
        assert_eq!(credentials.get("username").unwrap(), "CORP\\alice");
    }

    #[tokio::test]
    async fn rejects_a_config_missing_either_field() {
        let strategy = WindowsAuthStrategy;
        assert!(strategy.authenticate(&AuthConfig::new()).await.is_err());
    }
}
