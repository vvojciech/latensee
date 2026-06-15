use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, RwLock};

use hickory_resolver::Resolver;
use hickory_resolver::name_server::TokioConnectionProvider;
use tokio_util::sync::CancellationToken;

use super::state::TraceState;

/// Cache for reverse DNS results. Stores both positive and negative lookups.
pub struct DnsCache {
    entries: HashMap<IpAddr, Option<String>>,
}

impl DnsCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Insert a lookup result. `hostname` is None for negative (failed) lookups.
    pub fn insert(&mut self, addr: IpAddr, hostname: Option<String>) {
        self.entries.insert(addr, hostname);
    }

    /// Returns None if not cached, Some(None) if cached negative, Some(Some(&hostname)) if cached positive.
    pub fn get(&self, addr: &IpAddr) -> Option<Option<&String>> {
        self.entries.get(addr).map(|v| v.as_ref())
    }
}

/// Async DNS resolver with caching.
pub struct DnsResolver {
    resolver: Resolver<TokioConnectionProvider>,
    cache: DnsCache,
}

impl DnsResolver {
    pub async fn new() -> Result<Self, anyhow::Error> {
        let resolver = Resolver::builder_tokio()?.build();
        Ok(Self {
            resolver,
            cache: DnsCache::new(),
        })
    }

    /// Resolve an IP to a hostname. Checks cache first, then performs reverse lookup.
    /// Caches both positive and negative results.
    pub async fn resolve(&mut self, addr: IpAddr) -> Option<String> {
        if let Some(cached) = self.cache.get(&addr) {
            return cached.cloned();
        }

        let hostname = match self.resolver.reverse_lookup(addr).await {
            Ok(lookup) => lookup
                .iter()
                .next()
                .map(|name| name.to_string().trim_end_matches('.').to_string()),
            Err(_) => None,
        };

        self.cache.insert(addr, hostname.clone());
        hostname
    }

    /// Check cache without performing a lookup.
    /// Returns None if not cached, Some(None) if cached negative, Some(Some(&hostname)) if cached positive.
    pub fn cached(&self, addr: &IpAddr) -> Option<Option<&String>> {
        self.cache.get(addr)
    }
}

/// Background task that resolves hostnames for hops in TraceState.
pub async fn run_dns_resolver(
    state: Arc<RwLock<TraceState>>,
    mut resolver: DnsResolver,
    no_dns: bool,
    cancel: CancellationToken,
) {
    if no_dns {
        return;
    }

    loop {
        // Collect addresses that need resolution
        let addrs_to_resolve: Vec<(usize, IpAddr)> = {
            let state = state.read().unwrap();
            state
                .hops
                .iter()
                .enumerate()
                .filter_map(|(i, hop)| {
                    if let (Some(addr), None) = (&hop.addr, &hop.hostname) {
                        // Only resolve if not already cached (including negative cache)
                        if resolver.cached(addr).is_none() {
                            Some((i, *addr))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect()
        };

        // Resolve and write back
        for (idx, addr) in addrs_to_resolve {
            if cancel.is_cancelled() {
                return;
            }

            let hostname = resolver.resolve(addr).await;

            let mut state = state.write().unwrap();
            if let Some(hop) = state.hops.get_mut(idx) {
                if hop.addr == Some(addr) {
                    hop.hostname = hostname;
                }
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
            _ = cancel.cancelled() => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn dns_cache_stores_and_retrieves_positive_result() {
        let mut cache = DnsCache::new();
        let addr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        cache.insert(addr, Some("localhost".to_string()));

        let result = cache.get(&addr);
        assert!(matches!(result, Some(Some(name)) if name == "localhost"));
    }

    #[test]
    fn dns_cache_stores_and_retrieves_negative_result() {
        let mut cache = DnsCache::new();
        let addr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        cache.insert(addr, None);

        let result = cache.get(&addr);
        assert!(matches!(result, Some(None)));
    }

    #[test]
    fn dns_cache_returns_none_for_uncached_address() {
        let cache = DnsCache::new();
        let addr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));

        let result = cache.get(&addr);
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn resolver_resolves_loopback() {
        let mut resolver = DnsResolver::new().await.expect("failed to create resolver");
        let addr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        let result = resolver.resolve(addr).await;
        assert!(
            result.is_some(),
            "expected 127.0.0.1 to resolve to a hostname"
        );
        assert!(
            !result.as_ref().unwrap().is_empty(),
            "expected non-empty hostname"
        );
    }

    #[tokio::test]
    async fn resolver_caches_result_after_lookup() {
        let mut resolver = DnsResolver::new().await.expect("failed to create resolver");
        let addr = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        // Not cached before first resolve
        assert!(resolver.cached(&addr).is_none());

        let _ = resolver.resolve(addr).await;

        // Cached after resolve
        let cached = resolver.cached(&addr);
        assert!(cached.is_some(), "expected result to be cached after resolve");
    }
}
