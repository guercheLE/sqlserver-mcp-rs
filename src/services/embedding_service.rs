// SQL Server 2025 - master/msdb/sandbox combined catalog MCP server.
//
// Single source of truth for embedding computation. bin/populate_embeddings.rs
// (indexing time) and tools/search_tool.rs (live query time) both call
// `embed()` from here, so the two are structurally guaranteed to share the
// same model and vector space.
//
// mcpify's originally-generated `all-mpnet-base-v2` at its native 768
// dimensions -- also mcpify's own generator hard-code for the
// `semantic_endpoints` vec0 table's column width (`mcpify/src/db/schema.rs`,
// not something this project's source controls), so this module's model
// and every `mcp_store*.db.zst` file mcpify produces always agree on
// dimension with no extra resize step needed between a `mcpify sync` and
// `populate_embeddings`.
//
// A smaller 384-dim model (`all-MiniLM-L6-v2`) was tried at one point to
// fit crates.io's 10 MiB package-size limit, back when this project's
// stores were committed uncompressed -- with 4 SQL Server versions each
// embedding ~700-800 operations, 768-dim float32 vectors alone pushed the
// packaged crate right to that limit. Since then the stores were switched
// to zstd-compressed (level 19) `.db.zst` embeds (see src/data/store.rs),
// and re-measuring `cargo package` at 768-dim against the current catalog
// came back at 8.5 MiB compressed -- comfortably under the limit again
// (embedding vectors themselves are close to incompressible, so the
// compression gain comes from the surrounding relational/schema data, not
// the vectors) -- so the switch back to the full 768-dim model for better
// `search` tool relevance was safe to make permanent.

use std::sync::{Mutex, OnceLock};

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

fn model() -> &'static Mutex<TextEmbedding> {
    static MODEL: OnceLock<Mutex<TextEmbedding>> = OnceLock::new();
    MODEL.get_or_init(|| {
        // Downloads on first use and caches locally afterward (no network
        // needed once cached). `.expect()` matches this project's other
        // unrecoverable-startup-error handling: nothing useful can happen
        // if the model can't be fetched/loaded.
        Mutex::new(
            TextEmbedding::try_new(TextInitOptions::new(EmbeddingModel::AllMpnetBaseV2))
                .expect("failed to load the all-mpnet-base-v2 embedding model"),
        )
    })
}

/// Computes a 768-dim embedding vector for `text`, mean-pooled and
/// normalized (fastembed's default behavior for this model, replicating
/// the sentence-transformers reference implementation).
pub fn embed(text: &str) -> anyhow::Result<Vec<f32>> {
    let model = model();
    let mut model = model.lock().unwrap();
    let mut embeddings = model.embed(vec![text], None)?;
    embeddings
        .pop()
        .ok_or_else(|| anyhow::anyhow!("embedding model returned no output for the given text"))
}
