use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::http_helpers::origin_pool_key;
use super::probe::configure_http_client_builder;

const HTTP_POOL_CONNECT_TIMEOUT_SECONDS: u64 = 8;
const HTTP_POOL_TCP_KEEPALIVE_SECONDS: u64 = 45;
const HTTP_POOL_MAX_IDLE_PER_HOST: usize = 12;

pub struct HttpClientLease {
    pub client: Arc<reqwest::Client>,
    pub reused_pool_client: bool,
}

/// Global per-host HTTP client pool for reusing connections
pub struct HttpPool {
    clients: Arc<Mutex<BTreeMap<String, Arc<reqwest::Client>>>>,
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
        Self {
            clients: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn get_client(&self, url: &str) -> Option<HttpClientLease> {
        let host_key = origin_pool_key(url)?;
        if let Some(client) = self.clients.lock().ok()?.get(&host_key).cloned() {
            return Some(HttpClientLease {
                client,
                reused_pool_client: true,
            });
        }

        let client = build_client()?;
        let mut clients = self.clients.lock().ok()?;
        if let Some(existing) = clients.get(&host_key).cloned() {
            return Some(HttpClientLease {
                client: existing,
                reused_pool_client: true,
            });
        }

        clients.insert(host_key, Arc::clone(&client));
        Some(HttpClientLease {
            client,
            reused_pool_client: false,
        })
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
        }
    }
}
