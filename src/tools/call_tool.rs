// SQL Server 2025 - master/msdb/sandbox combined catalog MCP server.

use crate::auth::auth_manager::AuthManager;
use crate::core::config_schema::Config;
use crate::data::store::EndpointRecord;
use crate::services::api_client::ApiClient;
use crate::validation::validator::{validate_input, validate_output};

/// Validates arguments against the input schema, resolves the configured
/// SQL Server credentials, executes the live TDS call, and checks the
/// response against the output schema before returning it.
///
/// An output-schema mismatch is only logged, not raised: `resultset.sql`'s
/// best-effort column introspection (see
/// docs/sqlserver-eda-openapi-pipeline/README.md's "Known limitations")
/// can be wrong about a conditional-result-set object's actual shape, and
/// rejecting an otherwise-successful call over a documentation gap would
/// deny the caller real data it already has in hand. Input validation
/// still hard-fails: those arguments are under the caller's control, not
/// the database's.
///
/// Takes an already-looked-up `EndpointRecord` rather than a `Connection`
/// and an `operation_id` to look up itself: `rusqlite::Connection` isn't
/// `Sync`, so a `&Connection` held across this function's `.await` points
/// (the TDS call) would make the caller's future non-`Send` — a hard
/// requirement for `#[tool]` methods. Callers look the endpoint up (a
/// synchronous, `Connection`-scoped step) before calling this function,
/// not inside it.
pub async fn call_operation(
    endpoint: &EndpointRecord,
    config: &Config,
    auth_manager: &mut AuthManager,
    operation_id: &str,
    args: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    validate_input(&config.api_version, operation_id, &args)?;

    let client = ApiClient::new(config.clone());
    let response = client.execute(endpoint, &args, auth_manager).await?;

    if let Err(err) = validate_output(&config.api_version, operation_id, &response) {
        tracing::warn!(
            operation_id,
            error = %err,
            "response did not match the documented schema; returning it as-is"
        );
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> Config {
        Config {
            url: "localhost".to_string(),
            auth_method: crate::core::config_schema::AuthMethod::SqlServer,
            sql_port: 1433,
            pool_max_size: 10,
            trust_server_cert: true,
            api_version: "2025".to_string(),
            log_level: "info".to_string(),
            rate_limit: 100,
            timeout_ms: 30_000,
            cache_size: 500,
            retry_attempts: 3,
            transport: crate::core::config_schema::Transport::Stdio,
            host: "127.0.0.1".to_string(),
            port: 3000,
            cors_allow: None,
            default_database: None,
        }
    }

    fn dummy_endpoint(operation_id: &str) -> EndpointRecord {
        // Never read before `validate_input` returns its `Err` below, so
        // these placeholder values are never exercised.
        EndpointRecord {
            operation_id: operation_id.to_string(),
            path: "/dbo/sp_MailItemResultSets".to_string(),
            method: "POST".to_string(),
            summary: None,
            description: None,
            input_schema: serde_json::Value::Null,
            output_schema: serde_json::Value::Null,
            auth_scheme_ref: None,
        }
    }

    /// `dbo_sp_MailItemResultSets` (a real 2025-catalog operation) requires
    /// `mailitem_id`/`profile_id`/`conversation_handle`/
    /// `service_contract_name`/`message_type_name` -- an empty body fails
    /// schema validation, which `call_operation` checks *before* building
    /// an `ApiClient` or attempting any TDS connection, so this is
    /// reachable without live SQL Server infra.
    #[tokio::test]
    async fn call_operation_rejects_invalid_input_before_touching_the_network() {
        let operation_id = "dbo_sp_MailItemResultSets";
        let endpoint = dummy_endpoint(operation_id);
        let config = test_config();
        let mut auth_manager = AuthManager::new(config.auth_method);

        let result = call_operation(
            &endpoint,
            &config,
            &mut auth_manager,
            operation_id,
            serde_json::json!({"body": {}}),
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid input"));
    }
}
