use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_sqlserver-mcp"))
        .args(args)
        .env("SQLSERVER_URL", "localhost")
        .env("SQLSERVER_AUTH_METHOD", "sql_server")
        .output()
        .unwrap()
}

#[test]
fn version_prints_the_installed_package_version() {
    let output = run(&["version"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        env!("CARGO_PKG_VERSION")
    );
}

#[test]
fn versions_marks_the_default_and_active_catalog() {
    let output = Command::new(env!("CARGO_BIN_EXE_sqlserver-mcp"))
        .arg("versions")
        .env("SQLSERVER_URL", "localhost")
        .env("SQLSERVER_AUTH_METHOD", "sql_server")
        .env("SQLSERVER_API_VERSION", "2022")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "2025 (default)\n2022 (active)\n2019\n2017\n"
    );
}

#[test]
fn config_prints_the_resolved_non_secret_configuration() {
    let output = run(&["config"]);
    assert!(output.status.success());
    let config: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(config["url"], "localhost");
    assert_eq!(config["auth_method"], "sql_server");
    assert_eq!(config["api_version"], "2025");
}

#[test]
fn search_rejects_an_empty_measured_workload_before_loading_the_model() {
    let output = run(&["search", "test query", "--profile-iterations", "0"]);
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap().trim(),
        "--profile-iterations must be at least 1"
    );
}

#[test]
fn profiling_controls_do_not_appear_in_public_help() {
    let output = run(&["search", "--help"]);
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(!help.contains("profile-warmups"));
    assert!(!help.contains("profile-iterations"));
}

/// A real, parameterless (aside from the synthetic `execution_database`)
/// operation in the default 2025 catalog -- `get` only ever reads the
/// local embedded store, never a live SQL Server connection, so this is
/// a genuine end-to-end success case, not just an error-path check.
#[test]
fn get_prints_the_seeded_schema_for_a_real_operation() {
    let output = run(&["get", "dbo_autoadmin_fetch_system_flags"]);
    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["operation_id"], "dbo_autoadmin_fetch_system_flags");
}

#[test]
fn get_reports_an_unknown_operation_id() {
    let output = run(&["get", "not_a_real_operation"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("unknown operationId 'not_a_real_operation'"));
}

#[test]
fn call_reports_an_unknown_operation_id_before_touching_the_network() {
    // cli::call::run looks the endpoint up in the local store itself,
    // before ever calling call_operation/ApiClient -- reachable with no
    // live SQL Server.
    let output = run(&["call", "not_a_real_operation"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("unknown operationId 'not_a_real_operation'"));
}

/// `search` only ever touches the local embedded store + the (already
/// locally cached) embedding model -- a real end-to-end success case
/// reachable with no live SQL Server, unlike `call`.
#[test]
fn search_returns_results_for_a_real_query() {
    let output = run(&["search", "list active SQL Server connections"]);
    assert!(output.status.success());
    let results: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(results.as_array().unwrap().len() > 0);
}

#[test]
fn search_reports_profiling_timing_to_stderr_when_warmups_are_requested() {
    let output = run(&[
        "search",
        "list active SQL Server connections",
        "--profile-warmups",
        "1",
    ]);
    assert!(output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("profile workload"));
}

#[test]
fn call_rejects_invalid_input_before_touching_the_network() {
    // dbo_sp_MailItemResultSets requires mailitem_id/profile_id/
    // conversation_handle/service_contract_name/message_type_name -- an
    // explicit empty `body` fails schema validation inside call_operation
    // before any TDS connection is attempted. (The default `--args '{}'`
    // has no `body` key at all, which the top-level schema treats as
    // simply absent/optional rather than invalid -- it only starts
    // checking `body`'s own required fields once `body` is present.)
    let output = run(&[
        "call",
        "dbo_sp_MailItemResultSets",
        "--args",
        "{\"body\":{}}",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("invalid input"));
}
