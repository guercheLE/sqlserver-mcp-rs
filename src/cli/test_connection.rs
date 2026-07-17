// SQL Server 2025 - master/msdb/sandbox combined catalog MCP server.

use sqlserver_mcp_catalog::auth::auth_manager::AuthManager;
use sqlserver_mcp_catalog::core::config_manager::load_config;
use sqlserver_mcp_catalog::services::sql_pool;

pub async fn run() -> anyhow::Result<()> {
    let config = load_config(serde_json::Map::new())?;
    let mut auth_manager = AuthManager::new(config.auth_method);
    let auth_method = auth_manager.resolve_tds_auth().await?;

    let (host, port) = config.host_and_port();
    let mut tiberius_config = tiberius::Config::new();
    tiberius_config.host(host);
    tiberius_config.port(port);
    tiberius_config.authentication(auth_method);
    if config.trust_server_cert {
        tiberius_config.trust_cert();
    }

    let pool = sql_pool::cached_pool(
        &format!("{host}:{port}"),
        tiberius_config,
        config.pool_max_size,
    )
    .await?;
    let mut conn = match pool.get().await {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("connection failed: {err}");
            std::process::exit(1);
        }
    };

    match conn.simple_query("SELECT 1").await {
        Ok(_) => {
            println!("connection OK");
            Ok(())
        }
        Err(err) => {
            anyhow::bail!("connection failed: {err}");
        }
    }
}
