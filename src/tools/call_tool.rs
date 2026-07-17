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
