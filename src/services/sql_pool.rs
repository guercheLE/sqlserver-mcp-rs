// SQL Server 2025 - master/msdb/sandbox combined catalog MCP server.
//
// A process-wide cache of `bb8` connection pools, one per distinct
// `tiberius::Config` (in practice: one, since `Config` is loaded once at
// startup and reused for the process's lifetime) — mirrors
// `data::store::cached_store_connection`'s process-wide-cache-by-key shape,
// the existing convention in this codebase for "expensive resource, built
// once, shared across tool calls" rather than introducing a different
// pattern for this one case.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use bb8::Pool;
use bb8_tiberius::ConnectionManager;

pub type SqlPool = Pool<ConnectionManager>;

/// Returns the cached pool for `cache_key`, building one via `config` on
/// first use. `cache_key` is caller-supplied rather than derived from
/// `config` here, since two `Config`s that are meaningfully different for
/// pooling purposes (e.g. different resolved AAD tokens) don't necessarily
/// differ in a way that's cheap to hash/compare.
pub async fn cached_pool(
    cache_key: &str,
    config: tiberius::Config,
    max_size: u32,
) -> anyhow::Result<SqlPool> {
    static POOLS: OnceLock<Mutex<HashMap<String, SqlPool>>> = OnceLock::new();
    let pools = POOLS.get_or_init(|| Mutex::new(HashMap::new()));

    if let Some(pool) = pools.lock().unwrap().get(cache_key) {
        return Ok(pool.clone());
    }

    let manager = ConnectionManager::new(config);
    let pool = Pool::builder().max_size(max_size).build(manager).await?;

    // `bb8::Pool` is a cheap `Arc`-backed handle, so the pool built above
    // and the one returned to the caller after this insert are the same
    // underlying pool either way — a lost race here (two callers both
    // missing the cache and both building) just means one extra pool gets
    // built and then discarded once its `Arc` refcount drops, not a
    // correctness issue.
    let mut pools = pools.lock().unwrap();
    Ok(pools.entry(cache_key.to_string()).or_insert(pool).clone())
}
