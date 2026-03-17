use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::http_helpers::origin_pool_key;
use super::probe::configure_http_client_builder;

const HTTP_POOL_CONNECT_TIMEOUT_SECONDS: u64 = 8;
const HTTP_POOL_TCP_KEEPALIVE_SECONDS: u64 = 45;
const HTTP_POOL_MAX_IDLE_PER_HOST: usize = 12;
const HTTP_POOL_MAX_ORIGINS: usize = 128;
const HTTP_POOL_IDLE_EVICTION_MILLIS: u64 = 10 * 60 * 1_000;
const HTTP_POOL_CLEANUP_INTERVAL_MILLIS: u64 = 60 * 1_000;

pub struct HttpClientLease {
    pub client: Arc<reqwest::Client>,
    pub reused_pool_client: bool,
}

struct PooledClient {
    client: Arc<reqwest::Client>,
    last_access_at_millis: AtomicU64,
}

impl PooledClient {
    fn new(client: Arc<reqwest::Client>, now_millis: u64) -> Self {
        Self {
            client,
            last_access_at_millis: AtomicU64::new(now_millis),
        }
    }

    fn touch(&self, now_millis: u64) {
        self.last_access_at_millis
            .store(now_millis, Ordering::Relaxed);
    }

    fn last_access_at_millis(&self) -> u64 {
        self.last_access_at_millis.load(Ordering::Relaxed)
    }
}

/// Global per-host HTTP client pool for reusing connections
pub struct HttpPool {
    clients: Arc<RwLock<BTreeMap<String, Arc<PooledClient>>>>,
    next_cleanup_at_millis: Arc<AtomicU64>,
}

fn build_client() -> Option<Arc<reqwest::Client>> {
    Some(Arc::new(
        configure_http_client_builder(reqwest::Client::builder())
            .tcp_nodelay(true)
            .connect_timeout(Duration::from_secs(HTTP_POOL_CONNECT_TIMEOUT_SECONDS))
            .tcp_keepalive(Duration::from_secs(HTTP_POOL_TCP_KEEPALIVE_SECONDS))
            .pool_max_idle_per_host(HTTP_POOL_MAX_IDLE_PER_HOST)
            .build()
            .ok()?,
    ))
}

impl HttpPool {
    pub fn new() -> Self {
        let now_millis = unix_epoch_millis();
        Self {
            clients: Arc::new(RwLock::new(BTreeMap::new())),
            next_cleanup_at_millis: Arc::new(AtomicU64::new(
                now_millis.saturating_add(HTTP_POOL_CLEANUP_INTERVAL_MILLIS),
            )),
        }
    }

    pub fn get_client(&self, url: &str) -> Option<HttpClientLease> {
        let host_key = origin_pool_key(url)?;
        let now_millis = unix_epoch_millis();
        if let Some(entry) = self.clients.read().ok()?.get(&host_key).cloned() {
            entry.touch(now_millis);
            self.maybe_cleanup(now_millis, Some(host_key.as_str()));
            return Some(HttpClientLease {
                client: Arc::clone(&entry.client),
                reused_pool_client: true,
            });
        }

        let client = build_client()?;
        let mut clients = self.clients.write().ok()?;
        cleanup_clients(&mut clients, now_millis, Some(host_key.as_str()));
        self.next_cleanup_at_millis.store(
            now_millis.saturating_add(HTTP_POOL_CLEANUP_INTERVAL_MILLIS),
            Ordering::Relaxed,
        );
        if let Some(existing) = clients.get(&host_key).cloned() {
            existing.touch(now_millis);
            return Some(HttpClientLease {
                client: Arc::clone(&existing.client),
                reused_pool_client: true,
            });
        }

        let entry = Arc::new(PooledClient::new(Arc::clone(&client), now_millis));
        clients.insert(host_key.clone(), entry);
        cleanup_clients(&mut clients, now_millis, Some(host_key.as_str()));
        Some(HttpClientLease {
            client,
            reused_pool_client: false,
        })
    }

    fn maybe_cleanup(&self, now_millis: u64, protected_host: Option<&str>) {
        let next_cleanup_at_millis = self.next_cleanup_at_millis.load(Ordering::Relaxed);
        if now_millis < next_cleanup_at_millis {
            return;
        }

        if self
            .next_cleanup_at_millis
            .compare_exchange(
                next_cleanup_at_millis,
                now_millis.saturating_add(HTTP_POOL_CLEANUP_INTERVAL_MILLIS),
                Ordering::AcqRel,
                Ordering::Relaxed,
            )
            .is_err()
        {
            return;
        }

        if let Ok(mut clients) = self.clients.write() {
            cleanup_clients(&mut clients, now_millis, protected_host);
        }
    }
}

impl Default for HttpPool {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for HttpPool {
    fn clone(&self) -> Self {
        Self {
            clients: Arc::clone(&self.clients),
            next_cleanup_at_millis: Arc::clone(&self.next_cleanup_at_millis),
        }
    }
}

fn cleanup_clients(
    clients: &mut BTreeMap<String, Arc<PooledClient>>,
    now_millis: u64,
    protected_host: Option<&str>,
) {
    let stale_before = now_millis.saturating_sub(HTTP_POOL_IDLE_EVICTION_MILLIS);
    let stale_keys: Vec<String> = clients
        .iter()
        .filter(|(host, entry)| {
            Some(host.as_str()) != protected_host
                && entry.last_access_at_millis() <= stale_before
        })
        .map(|(host, _)| host.clone())
        .collect();

    for key in stale_keys {
        clients.remove(&key);
    }

    while clients.len() > HTTP_POOL_MAX_ORIGINS {
        let Some(oldest_host) = clients
            .iter()
            .filter(|(host, _)| Some(host.as_str()) != protected_host)
            .min_by_key(|(_, entry)| entry.last_access_at_millis())
            .map(|(host, _)| host.clone())
        else {
            break;
        };

        clients.remove(&oldest_host);
    }
}

fn unix_epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{cleanup_clients, PooledClient, HTTP_POOL_IDLE_EVICTION_MILLIS, HTTP_POOL_MAX_ORIGINS};
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn pooled_client(now_millis: u64) -> Arc<PooledClient> {
        Arc::new(PooledClient::new(Arc::new(reqwest::Client::new()), now_millis))
    }

    #[test]
    fn cleanup_evicts_idle_origins() {
        let now_millis = 1_000_000_u64;
        let mut clients = BTreeMap::from([
            (
                "https://stale.example".to_string(),
                pooled_client(now_millis.saturating_sub(HTTP_POOL_IDLE_EVICTION_MILLIS + 1)),
            ),
            ("https://fresh.example".to_string(), pooled_client(now_millis)),
        ]);

        cleanup_clients(&mut clients, now_millis, None);

        assert!(!clients.contains_key("https://stale.example"));
        assert!(clients.contains_key("https://fresh.example"));
    }

    #[test]
    fn cleanup_keeps_protected_origin_even_if_idle() {
        let now_millis = 1_000_000_u64;
        let mut clients = BTreeMap::from([(
            "https://protected.example".to_string(),
            pooled_client(now_millis.saturating_sub(HTTP_POOL_IDLE_EVICTION_MILLIS + 1)),
        )]);

        cleanup_clients(&mut clients, now_millis, Some("https://protected.example"));

        assert!(clients.contains_key("https://protected.example"));
    }

    #[test]
    fn cleanup_trims_oldest_hosts_when_pool_grows_too_large() {
        let now_millis = 1_000_000_u64;
        let client = Arc::new(reqwest::Client::new());
        let mut clients = BTreeMap::new();

        for index in 0..=HTTP_POOL_MAX_ORIGINS {
            clients.insert(
                format!("https://host-{index}.example"),
                Arc::new(PooledClient::new(
                    Arc::clone(&client),
                    now_millis.saturating_add(index as u64),
                )),
            );
        }

        cleanup_clients(
            &mut clients,
            now_millis.saturating_add(HTTP_POOL_MAX_ORIGINS as u64),
            None,
        );

        assert_eq!(clients.len(), HTTP_POOL_MAX_ORIGINS);
        assert!(!clients.contains_key("https://host-0.example"));
        assert!(clients.contains_key(&format!("https://host-{HTTP_POOL_MAX_ORIGINS}.example")));
    }
}
