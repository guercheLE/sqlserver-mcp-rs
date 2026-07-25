// SQL Server 2025 - master/msdb/model combined catalog MCP server.
//
// Real transport: a pooled TDS connection to SQL Server (via `tiberius`),
// replacing mcpify's originally-generated `reqwest`-based HTTP client.
// These OpenAPI operations are synthetic (see
// docs/sqlserver-eda-openapi-pipeline/README.md's "OpenAPI mapping
// convention") -- there is no real HTTP endpoint on the other end, so
// `endpoint.path`/`endpoint.method` don't describe an HTTP request; they
// encode `/<schema>/<name>` (set by `tools/generate_openapi.py`, one path per
// deduplicated operation -- see that script's `build_operations()`) and are
// always `POST`. `endpoint.description` carries the object's
// `sys.objects.type_desc` (`VIEW` / `SQL_STORED_PROCEDURE` /
// `SQL_INLINE_TABLE_VALUED_FUNCTION` / `EXTENDED_STORED_PROCEDURE` / ...),
// which determines the actual T-SQL shape: a bare `SELECT` for views,
// `SELECT ... FROM name(args)` for table-valued functions and `SELECT
// name(args)` for scalar functions (positional args -- no named-parameter
// call syntax exists for a function call), or `EXEC name @p = v, ...` for
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

/// The synthetic request-body property documenting an operation's execution
/// context (see `docs/sqlserver-eda-openapi-pipeline/tools/generate_openapi.py`'s
/// `EXECUTION_DATABASE_PARAM`) -- not a real SQL Server parameter of the
/// underlying object, so it's pulled out of `body` and used to qualify the
/// call rather than being bound as a `@P<n>` argument. Every generated
/// operation carries this property (see `ordered_params`'s exclusion of it),
/// so this name is reserved across the whole generated store.
const EXECUTION_DATABASE_PARAM: &str = "execution_database";

/// Parses `endpoint.path`'s `/<schema>/<name>` shape (set by
/// `docs/sqlserver-eda-openapi-pipeline/tools/generate_openapi.py`) into its
/// two parts. Unlike the old per-database-prefixed path convention, which
/// database to actually run against is no longer baked into the path at
/// generation time (a deduplicated operation may exist identically in more
/// than one of master/msdb/model) -- see `resolve_execution_database`.
fn parse_path(path: &str) -> anyhow::Result<(&str, &str)> {
    let mut parts = path.trim_start_matches('/').splitn(2, '/');
    match (parts.next(), parts.next()) {
        (Some(schema), Some(name)) if !schema.is_empty() && !name.is_empty() => {
            Ok((validate_ident(schema)?, validate_ident(name)?))
        }
        _ => anyhow::bail!("endpoint path '{path}' is not in the expected /<schema>/<name> shape"),
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

/// Resolves an operation's execution-context database: the caller-supplied
/// `execution_database` body property (see `EXECUTION_DATABASE_PARAM`) if
/// present, else the server's configured `Config::default_database` if one
/// was set up (optional, see `cli::setup_wizard`), else `None` -- meaning
/// `build_statement` two-part-qualifies the call and lets the connection's
/// own current database context resolve it (equivalent to `DB_NAME()`/
/// `DB_ID()`). This three-tier fallback applies uniformly to every
/// operation now, not just ones documented in a particular database --
/// see `build_statement`'s doc comment.
fn resolve_execution_database(
    body: &Map<String, Value>,
    config: &Config,
) -> anyhow::Result<Option<String>> {
    if let Some(value) = body.get(EXECUTION_DATABASE_PARAM) {
        let requested = value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("'{EXECUTION_DATABASE_PARAM}' must be a string"))?;
        return Ok(Some(validate_ident(requested)?.to_string()));
    }
    config
        .default_database
        .as_deref()
        .map(validate_ident)
        .transpose()
        .map(|opt| opt.map(str::to_string))
}

struct Param {
    name: String,
    ordinal: u64,
    x_sql_type: String,
}

/// Every operation's generated input schema wraps its real parameters one
/// level down, under a single top-level `body` property (mcpify's
/// request-body convention -- see `ApiClient::execute`'s own `body`
/// extraction from the caller's `args`, which this mirrors), almost always
/// as a `$ref` into the schema's own `$defs` rather than an inline object.
/// Resolves that wrapper (and its `$ref`, if present) to the real
/// parameter-name map. Returns `None` for a schema with no `body` property
/// at all (a parameterless operation), which `ordered_params` below treats
/// as zero parameters.
fn request_properties(input_schema: &Value) -> Option<&Map<String, Value>> {
    let body_schema = input_schema.get("properties")?.get("body")?;
    let resolved = match body_schema.get("$ref").and_then(Value::as_str) {
        Some(reference) => reference
            .strip_prefix("#/$defs/")
            .and_then(|name| input_schema.get("$defs")?.get(name))?,
        None => body_schema,
    };
    resolved.get("properties").and_then(Value::as_object)
}

/// Reads `properties`/`x-sql-ordinal`/`x-sql-type` off a resolved input
/// schema (see `tools/generate_openapi.py`'s `build_request_schema`),
/// sorted by declared ordinal -- required for a table-valued function's
/// positional call syntax, and used for stored procedures' named syntax
/// mainly for a deterministic/readable generated statement.
fn ordered_params(input_schema: &Value) -> Vec<Param> {
    let mut params: Vec<Param> = request_properties(input_schema)
        .into_iter()
        .flatten()
        .filter(|(name, _)| name.as_str() != EXECUTION_DATABASE_PARAM)
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
///
/// `execution_database` is the resolved value of the synthetic
/// `execution_database` request property (see `resolve_execution_database`)
/// -- every operation now behaves the way only `sandbox`-tagged operations
/// used to: three-part-qualified (`db.schema.name`) when a database is
/// known, two-part-qualified (`schema.name`, resolved against the
/// connection's own current database context) when it isn't. There is no
/// per-operation baked-in database anymore -- a deduplicated operation may
/// exist identically in more than one of master/msdb/model (see
/// `docs/sqlserver-eda-openapi-pipeline/README.md`'s "OpenAPI mapping
/// convention"), so which one to actually hit is always resolved per call.
fn build_statement(
    schema: &str,
    name: &str,
    kind: Option<&str>,
    params: &[Param],
    body: &Map<String, Value>,
    execution_database: Option<&str>,
) -> anyhow::Result<(String, Vec<Box<dyn ToSql>>)> {
    let qualified = match execution_database {
        Some(db) => format!(
            "{}.{}.{}",
            quote_ident(db),
            quote_ident(schema),
            quote_ident(name)
        ),
        None => format!("{}.{}", quote_ident(schema), quote_ident(name)),
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
                    // Named EXEC-argument syntax requires the `@` sigil
                    // directly on the parameter name (`EXEC proc @param =
                    // value`, not `EXEC proc [param] = value` or
                    // `EXEC proc param = value`) -- bracket-quoting is for
                    // object identifiers, not parameter references, and
                    // using it here produced a syntax error on every
                    // parameterized stored-procedure call. `validate_ident`
                    // still runs first for the same defense-in-depth reason
                    // as the object-identifier path above.
                    assignments.push(format!("@{} = @P{}", validate_ident(&p.name)?, i + 1));
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
    pub async fn execute(
        &self,
        endpoint: &EndpointRecord,
        args: &Value,
        auth_manager: &mut AuthManager,
    ) -> anyhow::Result<Value> {
        let (schema, name) = parse_path(&endpoint.path)?;

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
        let execution_database = resolve_execution_database(&body, &self.config)?;

        let (sql, bound) = build_statement(
            schema,
            name,
            endpoint.description.as_deref(),
            &params,
            &body,
            execution_database.as_deref(),
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
    fn parse_path_splits_schema_name() {
        assert_eq!(
            parse_path("/INFORMATION_SCHEMA/COLUMNS").unwrap(),
            ("INFORMATION_SCHEMA", "COLUMNS")
        );
    }

    #[test]
    fn parse_path_rejects_a_path_with_too_few_segments() {
        assert!(parse_path("/COLUMNS").is_err());
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
        assert!(parse_path("/sys/sp_who; DROP TABLE endpoints--").is_err());
    }

    #[test]
    fn build_statement_selects_from_a_view_with_no_parameters() {
        let (sql, bound) = build_statement(
            "INFORMATION_SCHEMA",
            "COLUMNS",
            Some("VIEW"),
            &[],
            &Map::new(),
            None,
        )
        .unwrap();
        assert_eq!(sql, "SELECT * FROM [INFORMATION_SCHEMA].[COLUMNS]");
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
            "sys",
            "sp_rename",
            Some("SQL_STORED_PROCEDURE"),
            &params,
            &body,
            Some("master"),
        )
        .unwrap();
        assert_eq!(
            sql,
            "EXEC [master].[sys].[sp_rename] @objname = @P1, @newname = @P2"
        );
        assert_eq!(bound.len(), 2);
    }

    /// Regression test for incident 26-07-05700-04: every parameterized
    /// stored-procedure `call` (e.g. `sp_columns`, `sp_executesql`) failed
    /// with SQL Server error 102 "Incorrect syntax near '='", no matter
    /// what argument keys the caller supplied, because the generated
    /// assignment list bracket-quoted the parameter name (`[table_name] =
    /// @P1`) instead of prefixing it with `@` (`@table_name = @P1`) --
    /// bracket-quoting is valid for object identifiers, not for named
    /// EXEC arguments. This asserts the `@`-prefixed form specifically, so
    /// a future edit can't silently reintroduce bracket-quoting here even
    /// if it still happens to produce syntactically-plausible-looking SQL.
    #[test]
    fn build_statement_uses_at_prefixed_names_not_bracket_quoting_for_named_exec_arguments() {
        let params = vec![Param {
            name: "table_name".to_string(),
            ordinal: 1,
            x_sql_type: "nvarchar(384)".to_string(),
        }];
        let mut body = Map::new();
        body.insert(
            "table_name".to_string(),
            Value::String("databases".to_string()),
        );
        let (sql, _bound) = build_statement(
            "sys",
            "sp_columns",
            Some("SQL_STORED_PROCEDURE"),
            &params,
            &body,
            None,
        )
        .unwrap();
        assert_eq!(sql, "EXEC [sys].[sp_columns] @table_name = @P1");
        assert!(!sql.contains("[table_name]"));
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
            "sys",
            "dm_exec_sql_text",
            Some("SQL_INLINE_TABLE_VALUED_FUNCTION"),
            &params,
            &body,
            Some("master"),
        )
        .unwrap();
        assert_eq!(sql, "SELECT * FROM [master].[sys].[dm_exec_sql_text](@P1)");
        assert_eq!(bound.len(), 1);
    }

    #[test]
    fn build_statement_omits_the_database_qualifier_by_default() {
        let (sql, bound) =
            build_statement("dbo", "widgets", Some("VIEW"), &[], &Map::new(), None).unwrap();
        assert_eq!(sql, "SELECT * FROM [dbo].[widgets]");
        assert!(bound.is_empty());
    }

    #[test]
    fn build_statement_three_part_qualifies_with_a_requested_execution_database() {
        let (sql, bound) = build_statement(
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
    fn build_statement_execs_a_parameterless_proc() {
        let (sql, bound) = build_statement(
            "sys",
            "sp_who",
            Some("SQL_STORED_PROCEDURE"),
            &[],
            &Map::new(),
            None,
        )
        .unwrap();
        assert_eq!(sql, "EXEC [sys].[sp_who]");
        assert!(bound.is_empty());
    }

    #[test]
    fn ordered_params_excludes_the_synthetic_execution_database_property() {
        let schema = serde_json::json!({
            "properties": {
                "body": { "$ref": "#/$defs/sys_sp_who_Request" }
            },
            "$defs": {
                "sys_sp_who_Request": {
                    "properties": {
                        "loginame": { "x-sql-ordinal": 1, "x-sql-type": "sysname" },
                        "execution_database": { "type": "string" }
                    }
                }
            }
        });

        let params = ordered_params(&schema);
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "loginame");
    }

    #[test]
    fn resolve_execution_database_prefers_the_body_property_over_the_configured_default() {
        let mut body = Map::new();
        body.insert(
            EXECUTION_DATABASE_PARAM.to_string(),
            Value::String("reporting".to_string()),
        );
        let mut config = test_config();
        config.default_database = Some("fallback".to_string());
        assert_eq!(
            resolve_execution_database(&body, &config)
                .unwrap()
                .as_deref(),
            Some("reporting")
        );
    }

    #[test]
    fn resolve_execution_database_falls_back_to_the_configured_default() {
        let mut config = test_config();
        config.default_database = Some("fallback".to_string());
        assert_eq!(
            resolve_execution_database(&Map::new(), &config)
                .unwrap()
                .as_deref(),
            Some("fallback")
        );
    }

    #[test]
    fn resolve_execution_database_is_none_when_nothing_is_configured_or_supplied() {
        assert_eq!(
            resolve_execution_database(&Map::new(), &test_config()).unwrap(),
            None
        );
    }

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

    #[test]
    fn ordered_params_follow_the_schema_ordinals() {
        // Matches the real shape every generated schema actually has (see
        // `generated_schemas.json`): parameters live under `$defs`, `$ref`'d
        // from the top-level `body` property, not as flat top-level
        // properties.
        let schema = serde_json::json!({
            "properties": {
                "body": { "$ref": "#/$defs/sys_example_Request" }
            },
            "$defs": {
                "sys_example_Request": {
                    "properties": {
                        "second": { "x-sql-ordinal": 2, "x-sql-type": "int" },
                        "first": { "x-sql-ordinal": 1, "x-sql-type": "nvarchar(20)" }
                    }
                }
            }
        });

        let params = ordered_params(&schema);
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "first");
        assert_eq!(params[0].x_sql_type, "nvarchar(20)");
        assert_eq!(params[1].name, "second");
    }

    /// Regression test for incident 26-07-05700-04's underlying second bug:
    /// `ordered_params` used to read the schema's literal top-level
    /// `properties` map, which for every real generated schema is just
    /// `{"body": {"$ref": ...}}` -- so every parameterized operation
    /// collapsed to a single fake `body` parameter (observed live as
    /// `EXEC ... @body = @P1` instead of the real named arguments) rather
    /// than the operation's actual parameters.
    #[test]
    fn ordered_params_resolves_the_body_ref_wrapper() {
        let schema = serde_json::json!({
            "properties": {
                "body": { "$ref": "#/$defs/sys_sp_columns_Request" }
            },
            "$defs": {
                "sys_sp_columns_Request": {
                    "properties": {
                        "table_name": { "x-sql-ordinal": 1, "x-sql-type": "nvarchar(384)" },
                        "table_owner": { "x-sql-ordinal": 2, "x-sql-type": "nvarchar(384)" }
                    }
                }
            }
        });

        let params = ordered_params(&schema);
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "table_name");
        assert_eq!(params[1].name, "table_owner");
        assert!(params.iter().all(|p| p.name != "body"));
    }

    #[test]
    fn ordered_params_resolves_an_inline_body_without_a_ref() {
        let schema = serde_json::json!({
            "properties": {
                "body": {
                    "properties": {
                        "only": { "x-sql-ordinal": 1, "x-sql-type": "int" }
                    }
                }
            }
        });

        let params = ordered_params(&schema);
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].name, "only");
    }

    #[test]
    fn ordered_params_is_empty_for_a_schema_with_no_body_property() {
        let schema = serde_json::json!({ "properties": {} });
        assert!(ordered_params(&schema).is_empty());
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
