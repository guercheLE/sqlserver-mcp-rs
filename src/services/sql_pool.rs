// SQL Server - master/msdb/sandbox combined catalog MCP server.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A `tiberius::Config` pointed at a reserved documentation-only
    /// address (RFC 5737 TEST-NET-1) -- never routable, so nothing this
    /// test does can ever reach a real server. Safe to use because `bb8`'s
    /// `Pool::builder()` defaults `min_idle` to `None` (see bb8's own
    /// `Builder::default`/`PoolInner::start_connections`/`wanted`), so
    /// `build()` doesn't eagerly establish any connection when the
    /// builder never calls `.min_idle(..)` -- exactly what `cached_pool`
    /// does -- meaning `Pool::builder().max_size(n).build(manager).await`
    /// itself never touches the network here.
    fn unroutable_config() -> tiberius::Config {
        let mut config = tiberius::Config::new();
        config.host("192.0.2.1");
        config.port(1433);
        config
    }

    #[tokio::test]
    async fn cached_pool_serves_a_cache_hit_on_the_same_key_without_erroring() {
        // First call: cache miss, builds and inserts a new pool. Second
        // call, same key: cache hit, clones the cached entry instead of
        // building again -- exercises both branches of `cached_pool`
        // without a live SQL Server (`build()` doesn't eagerly connect;
        // see `unroutable_config`'s doc comment).
        let key = "cached_pool_serves_a_cache_hit_on_the_same_key_without_erroring";
        cached_pool(key, unroutable_config(), 5).await.unwrap();
        cached_pool(key, unroutable_config(), 5).await.unwrap();
    }

    #[tokio::test]
    async fn cached_pool_builds_a_separate_pool_for_a_distinct_key() {
        cached_pool(
            "cached_pool_builds_a_separate_pool_for_a_distinct_key_a",
            unroutable_config(),
            5,
        )
        .await
        .unwrap();
        cached_pool(
            "cached_pool_builds_a_separate_pool_for_a_distinct_key_b",
            unroutable_config(),
            5,
        )
        .await
        .unwrap();
    }
}
