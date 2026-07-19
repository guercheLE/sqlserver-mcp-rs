// SQL Server 2025 - master/msdb/sandbox combined catalog MCP server.
//
// Real transport: a pooled TDS connection to SQL Server (via `tiberius`),
// replacing mcpify's originally-generated `reqwest`-based HTTP client.
// These OpenAPI operations are synthetic (see
// docs/sqlserver-eda-openapi-pipeline/README.md's "OpenAPI mapping
// convention") -- there is no real HTTP endpoint on the other end, so
// `endpoint.path`/`endpoint.method` don't describe an HTTP request; they
// encode `/<db>/<schema>/<name>` (set by `merge_openapi.py`) and are always
// `POST`. `endpoint.description` carries the object's `sys.objects.type_desc`
// (`VIEW` / `SQL_STORED_PROCEDURE` / `SQL_INLINE_TABLE_VALUED_FUNCTION` /
// `EXTENDED_STORED_PROCEDURE`), which determines the actual T-SQL shape:
// a bare `SELECT` for views, `SELECT ... FROM name(args)` for inline
// table-valued functions (positional args -- no named-parameter call syntax
// exists for a FROM-clause function call), or `EXEC name @p = v, ...` for
// stored procedures (named, so param order and optional/defaulted params
// don't matter).

use serde_json::{Map, Value};
use tiberius::ToSql;

use crate::auth::auth_manager::AuthManager;
use crate::core::config_schema::Config;
use crate::data::store::EndpointRecord;
use crate::services::sql_pool;
use crate::services::sql_type::{column_data_to_json, json_to_param};
use crate::validation::validator::resolved_schemas_for;

/// Parses `endpoint.path`'s `/<db>/<schema>/<name>` shape (set by
/// `docs/sqlserver-eda-openapi-pipeline/tools/merge_openapi.py`) into its
/// three parts -- the only place in this generated project that recovers
/// which database an operation targets, since mcpify's store doesn't carry
/// the source spec's `x-sql-database` vendor extension through.
fn parse_path(path: &str) -> anyhow::Result<(&str, &str, &str)> {
    let mut parts = path.trim_start_matches('/').splitn(3, '/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(db), Some(schema), Some(name))
            if !db.is_empty() && !schema.is_empty() && !name.is_empty() =>
        {
            Ok((
                validate_ident(db)?,
                validate_ident(schema)?,
                validate_ident(name)?,
            ))
        }
        _ => anyhow::bail!(
            "endpoint path '{path}' is not in the expected /<db>/<schema>/<name> shape"
        ),
    }
}

/// Rejects any identifier containing a character outside `[A-Za-z0-9_]` --
/// every identifier passed through here (`db`/`schema`/`name`, stored
/// procedure parameter names) comes from this project's own generated spec
/// (looked up from the local SQLite store by `operation_id`, never built
/// from a caller-supplied string), and every name in the curated object
/// set this pipeline targets is already plain alphanumeric/underscore --
/// see docs/sqlserver-eda-openapi-pipeline/sql/eda/allowlist.yaml. This is
/// defense-in-depth on top of `quote_ident`'s bracket-escaping, not a
/// response to any known injection path: T-SQL has no way to bind an
/// object/parameter *name* as a query parameter (only values), so an
/// identifier that ever did contain attacker-influenced text would bypass
/// the parameterized-value protection `services::sql_type::json_to_param`
/// provides for every argument *value*. Failing loudly here on an
/// unexpected character is preferable to silently trusting it.
fn validate_ident(ident: &str) -> anyhow::Result<&str> {
    if !ident.is_empty()
        && ident
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        Ok(ident)
    } else {
        anyhow::bail!("identifier '{ident}' contains characters outside [A-Za-z0-9_]")
    }
}

/// Brackets a SQL Server identifier, doubling any embedded `]` -- a second,
/// independent layer under `validate_ident`'s charset allow-list, kept
/// rather than dropped now that the allow-list exists, since a future
/// change loosening the allow-list (e.g. to admit `$`/`#`-prefixed names)
/// should not silently lose this escaping.
fn quote_ident(ident: &str) -> String {
    format!("[{}]", ident.replace(']', "]]"))
}

struct Param {
    name: String,
    ordinal: u64,
    x_sql_type: String,
}

/// Reads `properties`/`x-sql-ordinal`/`x-sql-type` off a resolved input
/// schema (see `tools/generate_openapi.py`'s `build_request_schema`),
/// sorted by declared ordinal -- required for a table-valued function's
/// positional call syntax, and used for stored procedures' named syntax
/// mainly for a deterministic/readable generated statement.
fn ordered_params(input_schema: &Value) -> Vec<Param> {
    let mut params: Vec<Param> = input_schema
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .map(|(name, schema)| Param {
            name: name.clone(),
            ordinal: schema
                .get("x-sql-ordinal")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            x_sql_type: schema
                .get("x-sql-type")
                .and_then(Value::as_str)
                .unwrap_or("nvarchar(max)")
                .to_string(),
        })
        .collect();
    params.sort_by_key(|p| p.ordinal);
    params
}

/// Builds the T-SQL text (with `@P1`/`@P2`/... placeholders) and the
/// matching positionally-ordered bound parameters for one operation call.
fn build_statement(
    db: &str,
    schema: &str,
    name: &str,
    kind: Option<&str>,
    params: &[Param],
    body: &Map<String, Value>,
    database_override: Option<&str>,
) -> anyhow::Result<(String, Vec<Box<dyn ToSql>>)> {
    // "sandbox" isn't a real database name a production instance is
    // guaranteed to have -- it's the EDA pipeline's placeholder (see
    // docs/sqlserver-eda-openapi-pipeline/README.md) for whatever database
    // the TDS connection is already in (`sql_pool`/this fn's caller never
    // sets an initial `database` on `tiberius::Config`, so that's always
    // the login's server-configured default database, i.e. `db_name()`).
    // Qualifying with a literal `[sandbox].` prefix would send every
    // sandbox-database call at the wrong (nonexistent, or coincidentally
    // named) database in any real deployment, so this two-part-qualifies
    // instead by default and lets the connection's own current-database
    // context resolve it -- unless the caller passed a top-level
    // `database` argument (see `ApiClient::execute`), in which case that
    // name replaces "sandbox" and the call goes out three-part qualified
    // against the requested database instead. `master`/`msdb` are real,
    // always-present system database names on every SQL Server instance,
    // so those always stay three-part qualified and never take the
    // override -- there's no ambiguity to resolve for them.
    let qualified = match (db, database_override) {
        ("sandbox", Some(requested_db)) => format!(
            "{}.{}.{}",
            quote_ident(requested_db),
            quote_ident(schema),
            quote_ident(name)
        ),
        ("sandbox", None) => format!("{}.{}", quote_ident(schema), quote_ident(name)),
        (_, _) => format!(
            "{}.{}.{}",
            quote_ident(db),
            quote_ident(schema),
            quote_ident(name)
        ),
    };

    let mut bound: Vec<Box<dyn ToSql>> = Vec::with_capacity(params.len());
    for param in params {
        let value = body.get(&param.name).cloned().unwrap_or(Value::Null);
        bound.push(json_to_param(&value, &param.x_sql_type)?);
    }

    let sql = match kind {
        Some("VIEW") => format!("SELECT * FROM {qualified}"),
        Some(k) if k.ends_with("_FUNCTION") || k.contains("TABLE_VALUED_FUNCTION") => {
            let placeholders = (1..=bound.len())
                .map(|i| format!("@P{i}"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("SELECT * FROM {qualified}({placeholders})")
        }
        // Stored procedures (SQL_STORED_PROCEDURE/EXTENDED_STORED_PROCEDURE)
        // and the `kind.is_none()` fallback (an older store built before
        // `description` carried this classification) both take the EXEC
        // path -- named parameter syntax, so a proc with only optional
        // parameters and no arguments supplied still works (`bound` empty
        // produces a bare `EXEC name`).
        _ => {
            if bound.is_empty() {
                format!("EXEC {qualified}")
            } else {
                let mut assignments = Vec::with_capacity(params.len());
                for (i, p) in params.iter().enumerate() {
                    assignments.push(format!(
                        "{} = @P{}",
                        quote_ident(validate_ident(&p.name)?),
                        i + 1
                    ));
                }
                format!("EXEC {qualified} {}", assignments.join(", "))
            }
        }
    };

    Ok((sql, bound))
}

/// Maps a `tiberius` error to this project's synthetic 400/403/500 split
/// (see `docs/sqlserver-eda-openapi-pipeline/README.md`'s "OpenAPI mapping
/// convention" -- SQL Server severity 11-16 statement errors are `400`,
/// severity-14 permission errors are `403`, severity 17-25 engine/fatal
/// errors are `500`), rather than surfacing the raw driver error
/// undifferentiated. Returned as a JSON value matching
/// `components.schemas.SqlServerError` shape rather than a bare string, so
/// callers get the same structured error either way they might have gotten
/// it from a real synchronous SQL Server error.
fn classify_tiberius_error(err: tiberius::error::Error) -> anyhow::Error {
    use tiberius::error::Error as TError;
    match &err {
        TError::Server(token) => {
            let (number, state, class, message, procedure, line) = (
                token.code(),
                token.state(),
                token.class(),
                token.message(),
                token.procedure(),
                token.line(),
            );
            let category = if class >= 17 {
                "fatal/resource error (500-equivalent)"
            } else if class == 14 {
                "permission denied (403-equivalent)"
            } else {
                "statement/user error (400-equivalent)"
            };
            anyhow::anyhow!(
                "SQL Server error {number} (severity {class}, state {state}) in '{procedure}' line {line}: {message} [{category}]"
            )
        }
        other => anyhow::anyhow!("SQL Server connection/protocol error (500-equivalent): {other}"),
    }
}

pub struct ApiClient {
    config: Config,
}

impl ApiClient {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Executes `endpoint` against the configured SQL Server instance.
    /// `auth_manager` resolves to a `tiberius::AuthMethod` (see
    /// `auth::auth_manager::AuthManager::resolve_tds_auth`) from the
    /// server's own configured credentials -- there is no per-request
    /// credential override; SQL Server auth is always this server's own
    /// configured identity, not something a caller supplies per call.
    ///
    /// A top-level `database` string in `args` (a sibling of `body`, e.g.
    /// `{"database": "reporting", "body": {...}}`) lets a caller target a
    /// specific database for a `sandbox`-tagged operation, overriding the
    /// connection's own current database (see `build_statement`'s doc
    /// comment for why "sandbox" needs this at all). It's not part of the
    /// generated/documented input schema -- mcpify's schema wrapper
    /// doesn't set `additionalProperties: false`, so an extra key here
    /// passes `validate_input` unnoticed -- and it's a no-op for
    /// `master`/`msdb` operations, which are always sent against their
    /// real, literal database regardless of this argument.
    pub async fn execute(
        &self,
        endpoint: &EndpointRecord,
        args: &Value,
        auth_manager: &mut AuthManager,
    ) -> anyhow::Result<Value> {
        let (db, schema, name) = parse_path(&endpoint.path)?;

        let (input_schema, _output_schema) =
            resolved_schemas_for(&self.config.api_version, &endpoint.operation_id).ok_or_else(
                || {
                    anyhow::anyhow!(
                        "no resolved schema found for operation '{}' under api_version '{}'",
                        endpoint.operation_id,
                        self.config.api_version
                    )
                },
            )?;
        let params = ordered_params(input_schema);

        let empty = Map::new();
        let args_map = args.as_object().unwrap_or(&empty);
        let body = args_map
            .get("body")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let database_override = args_map
            .get("database")
            .and_then(Value::as_str)
            .map(validate_ident)
            .transpose()?;

        let (sql, bound) = build_statement(
            db,
            schema,
            name,
            endpoint.description.as_deref(),
            &params,
            &body,
            database_override,
        )?;

        let auth_method = auth_manager.resolve_tds_auth().await?;

        let (host, port) = self.config.host_and_port();
        let mut tiberius_config = tiberius::Config::new();
        tiberius_config.host(host);
        tiberius_config.port(port);
        tiberius_config.authentication(auth_method);
        if self.config.trust_server_cert {
            // Curated system-catalog objects, not user data -- trusting the
            // server cert matches this project's primary use case (a local
            // Docker/dev instance with a self-signed cert; see
            // docs/sqlserver-eda-openapi-pipeline/docker-compose.yml)
            // rather than requiring every operator to supply a CA bundle
            // before the first query works. Set `trust_server_cert: false`
            // for a production instance with a real CA-signed cert.
            tiberius_config.trust_cert();
        }

        let pool_key = format!("{host}:{port}");
        let pool =
            sql_pool::cached_pool(&pool_key, tiberius_config, self.config.pool_max_size).await?;
        let mut conn = pool.get().await.map_err(|err| {
            anyhow::anyhow!("failed to obtain a pooled SQL Server connection: {err}")
        })?;

        let bound_refs: Vec<&dyn ToSql> = bound.iter().map(|param| param.as_ref()).collect();
        let stream = conn
            .query(&sql, &bound_refs)
            .await
            .map_err(classify_tiberius_error)?;
        let rows = stream
            .into_first_result()
            .await
            .map_err(classify_tiberius_error)?;

        let json_rows: Vec<Value> = rows
            .iter()
            .map(|row| {
                let mut obj = Map::new();
                for (column, data) in row.cells() {
                    obj.insert(column.name().to_string(), column_data_to_json(data));
                }
                Value::Object(obj)
            })
            .collect();
        Ok(Value::Array(json_rows))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_path_splits_db_schema_name() {
        assert_eq!(
            parse_path("/master/INFORMATION_SCHEMA/COLUMNS").unwrap(),
            ("master", "INFORMATION_SCHEMA", "COLUMNS")
        );
    }

    #[test]
    fn parse_path_rejects_a_path_with_too_few_segments() {
        assert!(parse_path("/master/COLUMNS").is_err());
    }

    #[test]
    fn quote_ident_doubles_embedded_closing_brackets() {
        assert_eq!(quote_ident("weird]name"), "[weird]]name]");
    }

    #[test]
    fn validate_ident_accepts_alphanumeric_and_underscore() {
        assert!(validate_ident("sp_who2").is_ok());
        assert!(validate_ident("INFORMATION_SCHEMA").is_ok());
    }

    #[test]
    fn validate_ident_rejects_anything_else() {
        assert!(validate_ident("").is_err());
        assert!(validate_ident("robert'; drop table endpoints;--").is_err());
        assert!(validate_ident("weird]name").is_err());
        assert!(validate_ident("has space").is_err());
    }

    #[test]
    fn parse_path_rejects_a_path_with_an_unsafe_segment() {
        assert!(parse_path("/master/sys/sp_who; DROP TABLE endpoints--").is_err());
    }

    #[test]
    fn build_statement_selects_from_a_view_with_no_parameters() {
        let (sql, bound) = build_statement(
            "master",
            "INFORMATION_SCHEMA",
            "COLUMNS",
            Some("VIEW"),
            &[],
            &Map::new(),
            None,
        )
        .unwrap();
        assert_eq!(sql, "SELECT * FROM [master].[INFORMATION_SCHEMA].[COLUMNS]");
        assert!(bound.is_empty());
    }

    #[test]
    fn build_statement_execs_a_proc_with_named_parameters() {
        let params = vec![
            Param {
                name: "objname".to_string(),
                ordinal: 1,
                x_sql_type: "nvarchar(1035)".to_string(),
            },
            Param {
                name: "newname".to_string(),
                ordinal: 2,
                x_sql_type: "sysname".to_string(),
            },
        ];
        let mut body = Map::new();
        body.insert("objname".to_string(), Value::String("t1".to_string()));
        body.insert("newname".to_string(), Value::String("t2".to_string()));
        let (sql, bound) = build_statement(
            "master",
            "sys",
            "sp_rename",
            Some("SQL_STORED_PROCEDURE"),
            &params,
            &body,
            None,
        )
        .unwrap();
        assert_eq!(
            sql,
            "EXEC [master].[sys].[sp_rename] [objname] = @P1, [newname] = @P2"
        );
        assert_eq!(bound.len(), 2);
    }

    #[test]
    fn build_statement_selects_from_a_function_with_positional_placeholders() {
        let params = vec![Param {
            name: "session_id".to_string(),
            ordinal: 1,
            x_sql_type: "smallint".to_string(),
        }];
        let mut body = Map::new();
        body.insert("session_id".to_string(), Value::from(52));
        let (sql, bound) = build_statement(
            "master",
            "sys",
            "dm_exec_sql_text",
            Some("SQL_INLINE_TABLE_VALUED_FUNCTION"),
            &params,
            &body,
            None,
        )
        .unwrap();
        assert_eq!(sql, "SELECT * FROM [master].[sys].[dm_exec_sql_text](@P1)");
        assert_eq!(bound.len(), 1);
    }

    #[test]
    fn build_statement_omits_the_database_qualifier_for_sandbox_by_default() {
        let (sql, bound) = build_statement(
            "sandbox",
            "dbo",
            "widgets",
            Some("VIEW"),
            &[],
            &Map::new(),
            None,
        )
        .unwrap();
        assert_eq!(sql, "SELECT * FROM [dbo].[widgets]");
        assert!(bound.is_empty());
    }

    #[test]
    fn build_statement_replaces_sandbox_with_a_requested_database_override() {
        let (sql, bound) = build_statement(
            "sandbox",
            "dbo",
            "widgets",
            Some("VIEW"),
            &[],
            &Map::new(),
            Some("reporting"),
        )
        .unwrap();
        assert_eq!(sql, "SELECT * FROM [reporting].[dbo].[widgets]");
        assert!(bound.is_empty());
    }

    #[test]
    fn build_statement_ignores_the_database_override_for_master_and_msdb() {
        let (sql, _bound) = build_statement(
            "master",
            "sys",
            "sp_who",
            Some("SQL_STORED_PROCEDURE"),
            &[],
            &Map::new(),
            Some("reporting"),
        )
        .unwrap();
        assert_eq!(sql, "EXEC [master].[sys].[sp_who]");
    }

    #[test]
    fn build_statement_execs_a_parameterless_proc() {
        let (sql, bound) = build_statement(
            "master",
            "sys",
            "sp_who",
            Some("SQL_STORED_PROCEDURE"),
            &[],
            &Map::new(),
            None,
        )
        .unwrap();
        assert_eq!(sql, "EXEC [master].[sys].[sp_who]");
        assert!(bound.is_empty());
    }

    #[test]
    fn ordered_params_follow_the_schema_ordinals() {
        let schema = serde_json::json!({
            "properties": {
                "second": { "x-sql-ordinal": 2, "x-sql-type": "int" },
                "first": { "x-sql-ordinal": 1, "x-sql-type": "nvarchar(20)" }
            }
        });

        let params = ordered_params(&schema);
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "first");
        assert_eq!(params[0].x_sql_type, "nvarchar(20)");
        assert_eq!(params[1].name, "second");
    }

    #[test]
    fn build_statement_rejects_an_unsafe_parameter_name() {
        let params = vec![Param {
            name: "unsafe name".to_string(),
            ordinal: 1,
            x_sql_type: "int".to_string(),
        }];

        assert!(
            build_statement(
                "master",
                "sys",
                "sp_example",
                Some("SQL_STORED_PROCEDURE"),
                &params,
                &Map::new(),
                None,
            )
            .is_err()
        );
    }

    #[test]
    fn protocol_errors_are_classified_as_server_failures() {
        let error =
            classify_tiberius_error(tiberius::error::Error::Protocol("invalid packet".into()));
        assert_eq!(
            error.to_string(),
            "SQL Server connection/protocol error (500-equivalent): Protocol error: invalid packet"
        );
    }
}
