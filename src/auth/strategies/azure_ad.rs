// SQL Server 2025 - master/msdb/sandbox combined catalog MCP server.
//
// `azureADAuth` (Azure Active Directory / Microsoft Entra ID — see
// docs/sqlserver-eda-openapi-pipeline/README.md's `securitySchemes`
// documentation; only SQL Server 2022/2025 accept it). Adapted from
// mcpify's originally generated `strategies::oauth2` (the OpenAPI `oauth2`
// scheme this maps from): the token exchange itself is the same standard
// OAuth2 mechanics, just the client-credentials grant (service-to-service —
// an MCP server has no interactive browser/redirect available at tool-call
// time, unlike `oauth2.rs`'s authorization-code flow) against Azure AD's
// token endpoint, requesting the `https://database.windows.net/.default`
// scope the spec's `azureADAuth.flows` declares.
//
// `services::api_client` passes the resulting `access_token` straight to
// `tiberius::AuthMethod::AADToken(token)` — tiberius only *consumes* a
// pre-obtained AAD token, it doesn't perform the OAuth exchange itself,
// which is exactly what this strategy is for.
//
// Token lifetime is typically ~1 hour; this strategy refreshes via a fresh
// client-credentials exchange (no separate refresh token exists for this
// grant type) once `validate_credentials` reports the cached token
// expired, the same expiry-driven reuse `AuthManager::credentials` already
// does for every strategy — a long-running server process re-authenticates
// automatically on the next tool call after expiry, not via a background
// timer.

use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::Deserialize;

use super::super::auth_strategy::{AuthConfig, AuthStrategy, Credentials};
use super::super::errors::AuthError;

const DEFAULT_SCOPE: &str = "https://database.windows.net/.default";

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: Option<u64>,
}

#[derive(Debug, Default)]
pub struct AzureAdAuthStrategy;

impl AzureAdAuthStrategy {
    fn token_url(config: &AuthConfig) -> anyhow::Result<String> {
        if let Some(url) = config.get("token_url") {
            return Ok(url.clone());
        }
        let tenant_id = config.get("tenant_id").ok_or_else(|| {
            AuthError::MissingCredentials(
                "tenant_id (or token_url), client_id, client_secret".to_string(),
            )
        })?;
        Ok(format!(
            "https://login.microsoftonline.com/{tenant_id}/oauth2/v2.0/token"
        ))
    }

    fn from_token_response(
        data: TokenResponse,
        config: &AuthConfig,
        token_url: &str,
    ) -> Credentials {
        let mut credentials = Credentials::new();
        credentials.insert("access_token".to_string(), data.access_token);
        if let Some(expires_in) = data.expires_in {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            credentials.insert(
                "expires_at".to_string(),
                (now_ms + expires_in * 1000).to_string(),
            );
        }
        for key in ["client_id", "client_secret", "tenant_id", "scope"] {
            if let Some(value) = config.get(key) {
                credentials.insert(key.to_string(), value.clone());
            }
        }
        credentials.insert("token_url".to_string(), token_url.to_string());
        credentials
    }
}

#[async_trait]
impl AuthStrategy for AzureAdAuthStrategy {
    async fn authenticate(&self, config: &AuthConfig) -> anyhow::Result<Credentials> {
        let client_id = config.get("client_id").ok_or_else(|| {
            AuthError::MissingCredentials("client_id, client_secret, tenant_id".to_string())
        })?;
        let client_secret = config.get("client_secret").ok_or_else(|| {
            AuthError::MissingCredentials("client_id, client_secret, tenant_id".to_string())
        })?;
        let token_url = Self::token_url(config)?;
        let scope = config
            .get("scope")
            .map(String::as_str)
            .unwrap_or(DEFAULT_SCOPE);

        let params = [
            ("grant_type", "client_credentials"),
            ("client_id", client_id.as_str()),
            ("client_secret", client_secret.as_str()),
            ("scope", scope),
        ];
        let response = reqwest::Client::new()
            .post(&token_url)
            .form(&params)
            .send()
            .await?
            .error_for_status()?
            .json::<TokenResponse>()
            .await?;

        Ok(Self::from_token_response(response, config, &token_url))
    }

    /// Client-credentials grants have no separate refresh token — a
    /// "refresh" is just re-running the same exchange with the credentials
    /// already on hand.
    async fn refresh_token(&self, credentials: &Credentials) -> anyhow::Result<Credentials> {
        self.authenticate(credentials).await
    }

    fn validate_credentials(&self, credentials: &Credentials) -> bool {
        if !credentials.contains_key("access_token") {
            return false;
        }
        let Some(expires_at) = credentials.get("expires_at") else {
            return true;
        };
        let Ok(expires_at) = expires_at.parse::<u64>() else {
            return false;
        };
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        now_ms < expires_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_url_defaults_to_the_tenant_specific_v2_endpoint() {
        let config = AuthConfig::from([("tenant_id".to_string(), "abc-123".to_string())]);
        assert_eq!(
            AzureAdAuthStrategy::token_url(&config).unwrap(),
            "https://login.microsoftonline.com/abc-123/oauth2/v2.0/token"
        );
    }

    #[test]
    fn token_url_prefers_an_explicit_override() {
        let config = AuthConfig::from([(
            "token_url".to_string(),
            "https://example.com/token".to_string(),
        )]);
        assert_eq!(
            AzureAdAuthStrategy::token_url(&config).unwrap(),
            "https://example.com/token"
        );
    }

    #[test]
    fn token_url_errors_without_either_tenant_id_or_token_url() {
        assert!(AzureAdAuthStrategy::token_url(&AuthConfig::new()).is_err());
    }

    #[test]
    fn validates_a_token_with_no_expiry_as_always_valid() {
        let strategy = AzureAdAuthStrategy;
        let credentials = Credentials::from([("access_token".to_string(), "abc".to_string())]);
        assert!(strategy.validate_credentials(&credentials));
    }

    #[test]
    fn rejects_an_expired_token() {
        let strategy = AzureAdAuthStrategy;
        let credentials = Credentials::from([
            ("access_token".to_string(), "abc".to_string()),
            ("expires_at".to_string(), "1".to_string()),
        ]);
        assert!(!strategy.validate_credentials(&credentials));
    }

    #[test]
    fn rejects_credentials_with_no_access_token() {
        let strategy = AzureAdAuthStrategy;
        assert!(!strategy.validate_credentials(&Credentials::new()));
    }
}
