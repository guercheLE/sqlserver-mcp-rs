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
