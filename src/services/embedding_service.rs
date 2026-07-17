// SQL Server 2025 - master/msdb/sandbox combined catalog MCP server.
//
// Single source of truth for embedding computation. bin/populate_embeddings.rs
// (indexing time) and tools/search_tool.rs (live query time) both call
// `embed()` from here, so the two are structurally guaranteed to share the
// same model and vector space.
//
// `all-MiniLM-L6-v2` at its native 384 dimensions -- switched from mcpify's
// originally-generated `all-mpnet-base-v2` (768-dim) purely to fit
// crates.io's 10 MiB package-size limit: with 4 SQL Server versions each
// embedding ~700-800 operations, 768-dim float32 vectors alone pushed the
// packaged crate to ~10.0 MiB compressed (measured via `cargo package`),
// right at the limit. Embedding vectors are close to incompressible
// (unlike the relational/schema data, which compresses ~20x), so halving
// the dimension was the single most effective lever -- it brought the
// package to a comfortable ~7 MiB. The tradeoff is somewhat coarser
// `search` tool relevance than the larger model would give; acceptable
// here since queries are short technical phrases ("rename a table",
// "current session's SQL text"), not long natural-language passages.
//
// IMPORTANT: mcpify's own generator hard-codes the `semantic_endpoints`
// vec0 table's column as `FLOAT[768]` (`mcpify/src/db/schema.rs`, not
// something this project's source controls) -- every `mcp_store*.db` file
// mcpify produces is born with a 768-dim column regardless of what this
// module computes. Docs/sqlserver-eda-openapi-pipeline/scripts/
// regenerate_mcp_server.sh's final step recreates that table at `FLOAT[384]`
// in each store after every `mcpify sync`, before `populate_embeddings`
// runs -- skipping that step means `populate_embeddings` will fail with a
// sqlite-vec dimension-mismatch error (384-dim vectors don't fit a
// 768-dim column), not silently write wrong data.

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
            TextEmbedding::try_new(TextInitOptions::new(EmbeddingModel::AllMiniLML6V2))
                .expect("failed to load the all-MiniLM-L6-v2 embedding model"),
        )
    })
}

/// Computes a 384-dim embedding vector for `text`, mean-pooled and
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
