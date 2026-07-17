// SQL Server 2025 - master/msdb/sandbox combined catalog MCP server.
//
// Recreates each store's `semantic_endpoints` vec0 table at
// `EMBEDDING_DIM` dimensions. Needed because mcpify's own generator
// hard-codes that table as `FLOAT[768]` (`mcpify/src/db/schema.rs`, not
// something this project's source controls) every time it writes a fresh
// `mcp_store*.db` -- so every `mcpify sync` re-creates a 768-dim column
// regardless of what `services::embedding_service` actually computes.
// This binary patches the column width back to match, on the *source*
// files at the repo root (`mcp_store.db`/`mcp_store_v2017.db`/...) --
// deliberately not through `data::store::resolve_store_path`, which
// extracts whatever bytes are already `include_bytes!`-embedded in this
// very binary from its *last* build, not the source files a fresh
// `mcpify sync` just wrote.
//
// Run once after every `mcpify sync`, before `populate_embeddings`:
// dropping and recreating the table empties it, so a stale 768-dim vector
// row is never left mismatched against the new column width (sqlite-vec
// would reject inserting a 384-dim vector into that stale row's column
// anyway, but starting from an empty table makes that impossible by
// construction rather than by relying on the insert failing correctly).
// See docs/sqlserver-eda-openapi-pipeline/scripts/regenerate_mcp_server.sh,
// which calls this automatically in the right order.

use std::path::Path;

use sql_server_2025_master_msdb_sandbox_combined_catalog::data::store::{
    VERSION_STORE_FILES, open_store_read_write,
};

/// Must match `services::embedding_service`'s active model's native
/// output dimension -- fastembed's `EmbeddingModel::AllMiniLML6V2` is
/// 384-dim (see that module's doc comment for why this model was chosen).
const EMBEDDING_DIM: u32 = 384;

fn resize_one(path: &Path) -> anyhow::Result<()> {
    let conn = open_store_read_write(path)?;
    conn.execute("DROP TABLE IF EXISTS semantic_endpoints", [])?;
    conn.execute(
        &format!(
            "CREATE VIRTUAL TABLE semantic_endpoints USING vec0(
                operation_id TEXT PRIMARY KEY,
                embedding FLOAT[{EMBEDDING_DIM}]
            )"
        ),
        [],
    )?;
    conn.execute_batch("VACUUM")?;
    Ok(())
}

fn main() -> anyhow::Result<()> {
    for (version, file) in VERSION_STORE_FILES {
        let path = Path::new(file);
        if !path.exists() {
            anyhow::bail!("'{file}' (version {version}) not found -- run from the repo root");
        }
        resize_one(path)?;
        println!("resized '{file}' (version {version}) to FLOAT[{EMBEDDING_DIM}]");
    }
    println!("done -- now run populate_embeddings to refill semantic_endpoints");
    Ok(())
}
