//! Query result caching with configurable TTL.
//!
//! Provides in-memory caching for query results to reduce database load
//! and improve response times for repeated queries.

use crate::database::QueryResult;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Cache entry containing a query result and metadata.
#[derive(Debug, Clone)]
pub struct CacheEntry {
    /// The cached query result.
    pub result: QueryResult,

    /// When the entry was created.
    pub created_at: Instant,

    /// Time-to-live for this entry.
    pub ttl: Duration,

    /// Number of times this entry has been accessed.
    pub hit_count: u64,

    /// Size estimate in bytes.
    pub size_bytes: usize,
}

impl CacheEntry {
    /// Create a new cache entry.
    pub fn new(result: QueryResult, ttl: Duration) -> Self {
        let size_bytes = estimate_result_size(&result);
        Self {
            result,
            created_at: Instant::now(),
            ttl,
            hit_count: 0,
            size_bytes,
        }
    }

    /// Check if the entry has expired.
    pub fn is_expired(&self) -> bool {
        self.created_at.elapsed() > self.ttl
    }

    /// Get the age of this entry.
    pub fn age(&self) -> Duration {
        self.created_at.elapsed()
    }

    /// Record a cache hit.
    pub fn record_hit(&mut self) {
        self.hit_count += 1;
    }
}

/// Query cache key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    /// Normalized query string.
    query: String,

    /// Maximum rows limit.
    max_rows: usize,

    /// Current database context (if any).
    database: Option<String>,
}

impl Hash for CacheKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.query.hash(state);
        self.max_rows.hash(state);
        self.database.hash(state);
    }
}

impl CacheKey {
    /// Create a new cache key.
    pub fn new(query: &str, max_rows: usize, database: Option<String>) -> Self {
        Self {
            query: normalize_query(query),
            max_rows,
            database,
        }
    }
}

/// Cache statistics.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// Total number of cache hits.
    pub hits: u64,

    /// Total number of cache misses.
    pub misses: u64,

    /// Total number of entries in cache.
    pub entry_count: usize,

    /// Total estimated size in bytes.
    pub total_size_bytes: usize,

    /// Number of evictions.
    pub evictions: u64,
}

impl CacheStats {
    /// Calculate hit rate as a percentage.
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            (self.hits as f64 / total as f64) * 100.0
        }
    }
}

/// Query result cache.
pub struct QueryCache {
    /// Cache entries.
    entries: RwLock<HashMap<CacheKey, CacheEntry>>,

    /// Default TTL for entries.
    default_ttl: Duration,

    /// Maximum cache size in bytes.
    max_size_bytes: usize,

    /// Maximum number of entries.
    max_entries: usize,

    /// Whether caching is enabled.
    enabled: bool,

    /// Cache statistics.
    stats: RwLock<CacheStats>,
}

impl QueryCache {
    /// Create a new query cache.
    pub fn new(default_ttl: Duration, max_size_mb: usize, max_entries: usize, enabled: bool) -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
            default_ttl,
            max_size_bytes: max_size_mb * 1024 * 1024,
            max_entries,
            enabled,
            stats: RwLock::new(CacheStats::default()),
        }
    }

    /// Check if caching is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Get a cached result.
    pub async fn get(&self, key: &CacheKey) -> Option<QueryResult> {
        if !self.enabled {
            return None;
        }

        let mut entries = self.entries.write().await;

        if let Some(entry) = entries.get_mut(key) {
            if entry.is_expired() {
                // Remove expired entry
                entries.remove(key);
                let mut stats = self.stats.write().await;
                stats.misses += 1;
                return None;
            }

            entry.record_hit();
            let result = entry.result.clone();

            let mut stats = self.stats.write().await;
            stats.hits += 1;

            Some(result)
        } else {
            let mut stats = self.stats.write().await;
            stats.misses += 1;
            None
        }
    }

    /// Insert a result into the cache.
    pub async fn insert(&self, key: CacheKey, result: QueryResult) {
        self.insert_with_ttl(key, result, self.default_ttl).await;
    }

    /// Insert a result with a specific TTL.
    pub async fn insert_with_ttl(&self, key: CacheKey, result: QueryResult, ttl: Duration) {
        if !self.enabled {
            return;
        }

        let entry = CacheEntry::new(result, ttl);
        let entry_size = entry.size_bytes;

        let mut entries = self.entries.write().await;

        // Check if we need to evict entries
        let current_size: usize = entries.values().map(|e| e.size_bytes).sum();
        let mut eviction_needed =
            entries.len() >= self.max_entries || current_size + entry_size > self.max_size_bytes;

        // Evict expired and LRU entries if needed
        if eviction_needed {
            let evictions = self.evict_entries(&mut entries, entry_size).await;
            let mut stats = self.stats.write().await;
            stats.evictions += evictions as u64;

            // Re-check after eviction
            let new_size: usize = entries.values().map(|e| e.size_bytes).sum();
            eviction_needed =
                entries.len() >= self.max_entries || new_size + entry_size > self.max_size_bytes;
        }

        // Only insert if we have space
        if !eviction_needed {
            entries.insert(key, entry);
            let mut stats = self.stats.write().await;
            stats.entry_count = entries.len();
            stats.total_size_bytes = entries.values().map(|e| e.size_bytes).sum();
        }
    }

    /// Evict entries to make room.
    async fn evict_entries(&self, entries: &mut HashMap<CacheKey, CacheEntry>, needed_bytes: usize) -> usize {
        let mut evicted = 0;

        // First, remove all expired entries
        let expired_keys: Vec<CacheKey> = entries
            .iter()
            .filter(|(_, e)| e.is_expired())
            .map(|(k, _)| k.clone())
            .collect();

        for key in expired_keys {
            entries.remove(&key);
            evicted += 1;
        }

        // Check if we need more space
        let current_size: usize = entries.values().map(|e| e.size_bytes).sum();
        if entries.len() < self.max_entries && current_size + needed_bytes <= self.max_size_bytes {
            return evicted;
        }

        // Remove least recently used (lowest hit count) entries
        let mut entries_by_hits: Vec<(CacheKey, u64)> = entries
            .iter()
            .map(|(k, e)| (k.clone(), e.hit_count))
            .collect();
        entries_by_hits.sort_by_key(|(_, hits)| *hits);

        let target_size = self.max_size_bytes - needed_bytes;
        let mut current_total: usize = current_size;

        for (key, _) in entries_by_hits {
            if entries.len() < self.max_entries && current_total <= target_size {
                break;
            }

            if let Some(entry) = entries.remove(&key) {
                current_total = current_total.saturating_sub(entry.size_bytes);
                evicted += 1;
            }
        }

        evicted
    }

    /// Clear all entries from the cache.
    pub async fn clear(&self) {
        let mut entries = self.entries.write().await;
        entries.clear();

        let mut stats = self.stats.write().await;
        stats.entry_count = 0;
        stats.total_size_bytes = 0;
    }

    /// Invalidate entries matching a pattern.
    pub async fn invalidate(&self, pattern: &str) {
        let mut entries = self.entries.write().await;
        entries.retain(|key, _| !key.query.contains(pattern));

        let mut stats = self.stats.write().await;
        stats.entry_count = entries.len();
        stats.total_size_bytes = entries.values().map(|e| e.size_bytes).sum();
    }

    /// Get cache statistics.
    pub async fn stats(&self) -> CacheStats {
        let entries = self.entries.read().await;
        let mut stats = self.stats.read().await.clone();
        stats.entry_count = entries.len();
        stats.total_size_bytes = entries.values().map(|e| e.size_bytes).sum();
        stats
    }

    /// Clean up expired entries.
    pub async fn cleanup(&self) {
        let mut entries = self.entries.write().await;
        let before = entries.len();
        entries.retain(|_, entry| !entry.is_expired());
        let evicted = before - entries.len();

        if evicted > 0 {
            let mut stats = self.stats.write().await;
            stats.evictions += evicted as u64;
            stats.entry_count = entries.len();
            stats.total_size_bytes = entries.values().map(|e| e.size_bytes).sum();
        }
    }
}

/// Shared cache wrapper.
pub type SharedCache = Arc<QueryCache>;

/// Create a new shared cache.
pub fn new_shared_cache(default_ttl: Duration, max_size_mb: usize, max_entries: usize, enabled: bool) -> SharedCache {
    Arc::new(QueryCache::new(default_ttl, max_size_mb, max_entries, enabled))
}

/// Normalize a query string for caching.
///
/// Removes extra whitespace and normalizes case for better cache hit rates.
fn normalize_query(query: &str) -> String {
    // Normalize whitespace
    query
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase()
}

/// Estimate the size of a query result in bytes.
fn estimate_result_size(result: &QueryResult) -> usize {
    let mut size = 0;

    // Column metadata
    for col in &result.columns {
        size += col.name.len();
        size += col.sql_type.len();
        size += 8; // bools and padding
    }

    // Row data (rough estimate)
    for row in &result.rows {
        for (key, value) in &row.columns {
            size += key.len();
            size += estimate_value_size(value);
        }
    }

    // Base overhead
    size += 64;

    size
}

/// Estimate the size of a SQL value in bytes.
fn estimate_value_size(value: &crate::database::types::SqlValue) -> usize {
    use crate::database::types::SqlValue;

    match value {
        SqlValue::Null => 1,
        SqlValue::Bool(_) => 1,
        SqlValue::I8(_) => 1,
        SqlValue::I16(_) => 2,
        SqlValue::I32(_) => 4,
        SqlValue::I64(_) => 8,
        SqlValue::F32(_) => 4,
        SqlValue::F64(_) => 8,
        SqlValue::String(s) => s.len(),
        SqlValue::Bytes(b) => b.len(),
        SqlValue::Decimal(d) => d.to_string().len(),
        SqlValue::Uuid(_) => 16,
        SqlValue::DateTime(_) => 32,
        SqlValue::DateTimeUtc(_) => 32,
        SqlValue::Date(_) => 16,
        SqlValue::Time(_) => 16,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_query() {
        assert_eq!(
            normalize_query("SELECT  *   FROM\n\tUsers"),
            "SELECT * FROM USERS"
        );
        assert_eq!(
            normalize_query("select id from  users where active = 1"),
            "SELECT ID FROM USERS WHERE ACTIVE = 1"
        );
    }

    #[test]
    fn test_cache_key() {
        let key1 = CacheKey::new("SELECT * FROM Users", 100, None);
        let key2 = CacheKey::new("SELECT  *  FROM  Users", 100, None);
        assert_eq!(key1, key2);

        let key3 = CacheKey::new("SELECT * FROM Users", 200, None);
        assert_ne!(key1, key3);
    }

    #[tokio::test]
    async fn test_cache_operations() {
        let cache = new_shared_cache(Duration::from_secs(60), 10, 100, true);

        let key = CacheKey::new("SELECT 1", 100, None);
        let result = QueryResult::empty();

        // Insert
        cache.insert(key.clone(), result.clone()).await;

        // Get
        let cached = cache.get(&key).await;
        assert!(cached.is_some());

        // Stats
        let stats = cache.stats().await;
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 0);
        assert_eq!(stats.entry_count, 1);
    }

    #[tokio::test]
    async fn test_cache_expiration() {
        let cache = new_shared_cache(Duration::from_millis(10), 10, 100, true);

        let key = CacheKey::new("SELECT 1", 100, None);
        let result = QueryResult::empty();

        cache.insert(key.clone(), result).await;

        // Should hit immediately
        assert!(cache.get(&key).await.is_some());

        // Wait for expiration
        tokio::time::sleep(Duration::from_millis(20)).await;

        // Should miss after expiration
        assert!(cache.get(&key).await.is_none());
    }

    #[tokio::test]
    async fn test_cache_disabled() {
        let cache = new_shared_cache(Duration::from_secs(60), 10, 100, false);

        let key = CacheKey::new("SELECT 1", 100, None);
        let result = QueryResult::empty();

        cache.insert(key.clone(), result).await;

        // Should always miss when disabled
        assert!(cache.get(&key).await.is_none());
    }

    #[test]
    fn test_cache_stats_hit_rate() {
        let mut stats = CacheStats::default();
        assert_eq!(stats.hit_rate(), 0.0);

        stats.hits = 80;
        stats.misses = 20;
        assert!((stats.hit_rate() - 80.0).abs() < 0.01);
    }
}
