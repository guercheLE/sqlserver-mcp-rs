// SQL Server 2025 - master/msdb/sandbox combined catalog MCP server.

use crate::core::config_schema::AuthMethod;
use crate::core::credential_storage::{load_credential, save_credential};

use super::auth_strategy::{AuthConfig, AuthStrategy, Credentials};
use super::errors::AuthError;
use super::strategies::azure_ad::AzureAdAuthStrategy;
use super::strategies::sql_server::SqlServerAuthStrategy;
use super::strategies::windows::WindowsAuthStrategy;

const CREDENTIAL_ACCOUNT: &str = "active-credentials";
const ENV_PREFIX: &str = "SQLSERVER";

/// Builds an `AuthConfig` straight from the `<PREFIX>_USERNAME`/`_PASSWORD`
/// (SQL Server/Windows auth) or `<PREFIX>_CLIENT_ID`/`_CLIENT_SECRET`/
/// `_TENANT_ID` (Azure AD) env vars documented in `.env.example`, if the
/// vars this `auth_method` needs are actually set. Returns `None` when the
/// required var(s) for this deployment's `auth_method` aren't present, so
/// callers fall back to the stored-credential lookup unchanged.
fn credentials_from_env(auth_method: AuthMethod) -> Option<AuthConfig> {
    let mut config = AuthConfig::new();
    match auth_method {
        AuthMethod::SqlServer | AuthMethod::Windows => {
            let username = std::env::var(format!("{ENV_PREFIX}_USERNAME")).ok()?;
            let password = std::env::var(format!("{ENV_PREFIX}_PASSWORD")).ok()?;
            config.insert("username".to_string(), username);
            config.insert("password".to_string(), password);
        }
        AuthMethod::AzureAd => {
            let client_id = std::env::var(format!("{ENV_PREFIX}_CLIENT_ID")).ok()?;
            let client_secret = std::env::var(format!("{ENV_PREFIX}_CLIENT_SECRET")).ok()?;
            config.insert("client_id".to_string(), client_id);
            config.insert("client_secret".to_string(), client_secret);
            if let Ok(tenant_id) = std::env::var(format!("{ENV_PREFIX}_TENANT_ID")) {
                config.insert("tenant_id".to_string(), tenant_id);
            }
        }
    }
    Some(config)
}

fn strategy_for(auth_method: AuthMethod) -> Box<dyn AuthStrategy> {
    match auth_method {
        AuthMethod::SqlServer => Box::new(SqlServerAuthStrategy),
        AuthMethod::Windows => Box::new(WindowsAuthStrategy),
        AuthMethod::AzureAd => Box::new(AzureAdAuthStrategy),
    }
}

/// Selects exactly one active auth strategy per deployment, chosen via the
/// `auth_method` config value — there is no runtime engine for resolving
/// multiple simultaneously-required schemes. These three methods
/// (`SqlServer`/`Windows`/`AzureAd`) are the complete set SQL Server's TDS
/// protocol accepts (see `core::config_schema::AuthMethod`'s doc comment)
/// — there is no per-request/HTTP-header credential relay concept here:
/// this server always connects to SQL Server with the one set of
/// operator-configured credentials resolved via `credentials()`'s config/
/// env/keychain cascade below, regardless of how a caller reaches *this*
/// MCP server (stdio or HTTP).
pub struct AuthManager {
    auth_method: AuthMethod,
    strategy: Box<dyn AuthStrategy>,
    cached_credentials: Option<Credentials>,
}

impl AuthManager {
    pub fn new(auth_method: AuthMethod) -> Self {
        Self {
            strategy: strategy_for(auth_method),
            auth_method,
            cached_credentials: None,
        }
    }

    pub async fn login(&mut self, config: &AuthConfig) -> anyhow::Result<Credentials> {
        let credentials = self.strategy.authenticate(config).await?;
        self.cached_credentials = Some(credentials.clone());
        save_credential(CREDENTIAL_ACCOUNT, &serde_json::to_string(&credentials)?)?;
        Ok(credentials)
    }

    /// Seeds in-memory credentials directly, bypassing OS-keychain/file
    /// storage entirely — for tests (which must never touch the real OS
    /// keychain as a side effect of running) and for callers that already
    /// hold validated credentials from elsewhere.
    pub fn set_credentials(&mut self, credentials: Credentials) {
        self.cached_credentials = Some(credentials);
    }

    pub async fn credentials(&mut self) -> anyhow::Result<Credentials> {
        if let Some(cached) = &self.cached_credentials
            && self.strategy.validate_credentials(cached)
        {
            return Ok(cached.clone());
        }

        if let Some(env_config) = credentials_from_env(self.auth_method)
            && let Ok(from_env) = self.strategy.authenticate(&env_config).await
            && self.strategy.validate_credentials(&from_env)
        {
            self.cached_credentials = Some(from_env.clone());
            return Ok(from_env);
        }

        if let Some(stored) = load_credential(CREDENTIAL_ACCOUNT)? {
            let parsed: Credentials = serde_json::from_str(&stored)?;
            if self.strategy.validate_credentials(&parsed) {
                self.cached_credentials = Some(parsed.clone());
                return Ok(parsed);
            }
            if let Ok(refreshed) = self.strategy.refresh_token(&parsed).await {
                self.cached_credentials = Some(refreshed.clone());
                save_credential(CREDENTIAL_ACCOUNT, &serde_json::to_string(&refreshed)?)?;
                return Ok(refreshed);
            }
        }

        Err(AuthError::NoActiveCredentials(format!("{:?}", self.auth_method)).into())
    }

    /// Resolves this deployment's active `tiberius::AuthMethod` from the
    /// config/env/keychain cascade (`credentials()`).
    pub async fn resolve_tds_auth(&mut self) -> anyhow::Result<tiberius::AuthMethod> {
        let credentials = self.credentials().await?;
        match self.auth_method {
            AuthMethod::SqlServer => {
                let username = credentials
                    .get("username")
                    .ok_or_else(|| AuthError::MissingCredentials("username".to_string()))?;
                let password = credentials
                    .get("password")
                    .ok_or_else(|| AuthError::MissingCredentials("password".to_string()))?;
                Ok(tiberius::AuthMethod::sql_server(username, password))
            }
            AuthMethod::Windows => {
                let username = credentials
                    .get("username")
                    .ok_or_else(|| AuthError::MissingCredentials("username".to_string()))?;
                let password = credentials
                    .get("password")
                    .ok_or_else(|| AuthError::MissingCredentials("password".to_string()))?;
                windows_auth_method(username, password)
            }
            AuthMethod::AzureAd => {
                let token = credentials
                    .get("access_token")
                    .ok_or_else(|| AuthError::MissingCredentials("access_token".to_string()))?;
                Ok(tiberius::AuthMethod::AADToken(token.clone()))
            }
        }
    }
}

/// `tiberius::AuthMethod::windows` only exists (`#[cfg(all(windows, feature
/// = "winauth"))]`) when actually compiling for Windows — it's a native
/// SSPI binding, not a portable NTLM implementation. On any other target
/// (this pipeline's primary one: Linux containers, see
/// docs/sqlserver-eda-openapi-pipeline/docker-compose.yml) there is no
/// non-Windows NTLM/SSPI path in this project's current dependencies
/// (`integrated-auth-gssapi` would add Kerberos via a system `libgssapi`
/// this project doesn't link), so this surfaces as a clear runtime error
/// instead of a compile failure or a silent wrong-auth fallback.
#[cfg(windows)]
fn windows_auth_method(username: &str, password: &str) -> anyhow::Result<tiberius::AuthMethod> {
    Ok(tiberius::AuthMethod::windows(username, password))
}

#[cfg(not(windows))]
fn windows_auth_method(_username: &str, _password: &str) -> anyhow::Result<tiberius::AuthMethod> {
    anyhow::bail!(
        "Windows Authentication requires tiberius's native SSPI binding, which is only compiled \
         on Windows targets; this server is running on a non-Windows target with no \
         non-Windows NTLM/Kerberos implementation linked in. Use `sql_server` or `azure_ad` \
         auth instead, or add Kerberos support (tiberius's `integrated-auth-gssapi` feature, \
         which requires the system `libgssapi` library) if Windows Authentication against a \
         non-Windows client is required."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn seeded_credentials_win_over_stored_ones() {
        let mut manager = AuthManager::new(AuthMethod::SqlServer);
        let mut credentials = Credentials::new();
        credentials.insert("username".to_string(), "sa".to_string());
        credentials.insert("password".to_string(), "s3cr3t".to_string());
        manager.set_credentials(credentials.clone());

        let resolved = manager.credentials().await.unwrap();
        assert_eq!(resolved.get("username").map(String::as_str), Some("sa"));
    }

    #[tokio::test]
    async fn resolve_tds_auth_builds_a_sql_server_auth_method() {
        let mut manager = AuthManager::new(AuthMethod::SqlServer);
        let mut credentials = Credentials::new();
        credentials.insert("username".to_string(), "sa".to_string());
        credentials.insert("password".to_string(), "s3cr3t".to_string());
        manager.set_credentials(credentials);

        let auth = manager.resolve_tds_auth().await.unwrap();
        assert!(matches!(auth, tiberius::AuthMethod::SqlServer(_)));
    }

    #[tokio::test]
    async fn resolve_tds_auth_builds_an_azure_ad_auth_method() {
        let mut manager = AuthManager::new(AuthMethod::AzureAd);
        let mut credentials = Credentials::new();
        credentials.insert("access_token".to_string(), "eyJ...".to_string());
        manager.set_credentials(credentials);

        let auth = manager.resolve_tds_auth().await.unwrap();
        assert!(matches!(auth, tiberius::AuthMethod::AADToken(_)));
    }

    #[tokio::test]
    async fn resolve_tds_auth_errors_without_any_credentials() {
        let mut manager = AuthManager::new(AuthMethod::SqlServer);
        assert!(manager.resolve_tds_auth().await.is_err());
    }
}
