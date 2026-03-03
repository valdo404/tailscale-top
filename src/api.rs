use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize)]
pub struct DevicesResponse {
    pub devices: Vec<Device>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    #[allow(dead_code)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub hostname: String,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub addresses: Vec<String>,
    #[serde(default)]
    #[allow(dead_code)]
    pub last_seen: String,
    #[serde(default)]
    #[allow(dead_code)]
    pub client_connectivity: Option<ClientConnectivity>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct ClientConnectivity {
    #[serde(default)]
    pub mapping_varies_by_dest_ip: Option<bool>,
}

impl Device {
    pub fn short_name(&self) -> &str {
        // Prefer hostname, but fall back to FQDN name if hostname is empty/localhost
        let h = self.hostname.split('.').next().unwrap_or(&self.hostname);
        if h.is_empty() || h == "localhost" {
            self.name.split('.').next().unwrap_or(&self.name)
        } else {
            h
        }
    }

    pub fn ipv4(&self) -> &str {
        self.addresses
            .iter()
            .find(|a| a.contains('.'))
            .map(|s| s.as_str())
            .unwrap_or("-")
    }

    pub fn short_os(&self) -> &str {
        match self.os.to_lowercase().as_str() {
            s if s.contains("macos") => "macOS",
            s if s.contains("ios") => "iOS",
            s if s.contains("android") => "Android",
            s if s.contains("windows") => "Windows",
            s if s.contains("linux") => "Linux",
            _ => &self.os,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PeerTraffic {
    pub tx_bytes: u64,
    pub rx_bytes: u64,
}

/// Combined local status info for a peer (from `tailscale status --json`).
#[derive(Debug, Clone, Default)]
pub struct LocalPeerInfo {
    pub online: bool,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
}

pub struct TailscaleClient {
    client: Client,
    api_key: String,
}

impl TailscaleClient {
    pub fn new(api_key: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self { client, api_key }
    }

    /// Fetch all devices from the Tailscale API.
    pub async fn fetch_devices(&self) -> Result<Vec<Device>, reqwest::Error> {
        let resp: DevicesResponse = self
            .client
            .get("https://api.tailscale.com/api/v2/tailnet/-/devices")
            .bearer_auth(&self.api_key)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(resp.devices)
    }

    /// Fetch Prometheus metrics from a node's metrics endpoint.
    /// Requires `tailscale set --webclient` on the target node and ACL for port 5252.
    /// Returns parsed tx/rx totals from Prometheus counters, or None if unreachable.
    pub async fn fetch_node_metrics(ip: String) -> Option<PeerTraffic> {
        let client = Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .ok()?;

        let url = format!("http://{ip}:5252/metrics");
        let body = client.get(&url).send().await.ok()?.text().await.ok()?;
        Some(parse_prometheus_metrics(&body))
    }
}

/// Parse Prometheus-format metrics text for tx/rx byte counters.
/// Looks for `tailscaled_inbound_bytes_total` and `tailscaled_outbound_bytes_total`.
fn parse_prometheus_metrics(body: &str) -> PeerTraffic {
    let mut tx: u64 = 0;
    let mut rx: u64 = 0;

    for line in body.lines() {
        if line.starts_with('#') {
            continue;
        }
        // tailscaled_inbound_bytes_total{} 12345
        if line.starts_with("tailscaled_inbound_bytes_total") {
            if let Some(val) = line.split_whitespace().last() {
                rx += val.parse::<f64>().unwrap_or(0.0) as u64;
            }
        }
        // tailscaled_outbound_bytes_total{} 12345
        if line.starts_with("tailscaled_outbound_bytes_total") {
            if let Some(val) = line.split_whitespace().last() {
                tx += val.parse::<f64>().unwrap_or(0.0) as u64;
            }
        }
    }

    PeerTraffic { tx_bytes: tx, rx_bytes: rx }
}

/// Parse `tailscale status --json` to get online status and tx/rx per peer.
/// Returns a map of tailscale IP → LocalPeerInfo.
/// This is a single call that combines both online status and traffic data.
pub fn parse_local_status() -> HashMap<String, LocalPeerInfo> {
    let mut map = HashMap::new();

    let output = Command::new("tailscale")
        .args(["status", "--json"])
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o.stdout,
        _ => return map,
    };

    let json: serde_json::Value = match serde_json::from_slice(&output) {
        Ok(v) => v,
        Err(_) => return map,
    };

    // Self node is always online
    if let Some(self_node) = json.get("Self") {
        if let Some(ip) = extract_first_ip(self_node) {
            map.insert(
                ip,
                LocalPeerInfo {
                    online: true,
                    tx_bytes: 0,
                    rx_bytes: 0,
                },
            );
        }
    }

    if let Some(peers) = json.get("Peer").and_then(|p| p.as_object()) {
        for (_key, peer) in peers {
            let Some(ip) = extract_first_ip(peer) else {
                continue;
            };

            let online = peer.get("Online").and_then(|v| v.as_bool()).unwrap_or(false);
            let tx = peer.get("TxBytes").and_then(|v| v.as_u64()).unwrap_or(0);
            let rx = peer.get("RxBytes").and_then(|v| v.as_u64()).unwrap_or(0);

            map.insert(
                ip,
                LocalPeerInfo {
                    online,
                    tx_bytes: tx,
                    rx_bytes: rx,
                },
            );
        }
    }

    map
}

fn extract_first_ip(node: &serde_json::Value) -> Option<String> {
    node.get("TailscaleIPs")
        .and_then(|ips| ips.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}
