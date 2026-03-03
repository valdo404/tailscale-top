use crate::api::{self, Device, TailscaleClient};
use std::collections::HashMap;
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortMode {
    Name,
    TxDesc,
    RxDesc,
}

#[derive(Debug, Clone)]
pub struct NodeInfo {
    pub name: String,
    pub ip: String,
    pub os: String,
    pub online: bool,
    pub has_webclient: bool,
    pub tx_bytes: Option<u64>,
    pub rx_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: String,
    pub event_type: String,
    pub node_name: String,
    pub node_ip: String,
    pub message: String,
}

pub struct App {
    pub nodes: Vec<NodeInfo>,
    pub sort_mode: SortMode,
    pub tailnet_name: String,
    pub total_nodes: usize,
    pub online_nodes: usize,
    pub refresh_interval_secs: u64,
    pub loading: bool,
    pub error: Option<String>,
    pub log_entries: Vec<LogEntry>,
    previous_online: HashMap<String, bool>,
    client: TailscaleClient,
}

impl App {
    pub fn new(api_key: String, refresh_interval_secs: u64) -> Self {
        Self {
            nodes: Vec::new(),
            sort_mode: SortMode::Name,
            tailnet_name: String::new(),
            total_nodes: 0,
            online_nodes: 0,
            refresh_interval_secs,
            loading: true,
            error: None,
            log_entries: Vec::new(),
            previous_online: HashMap::new(),
            client: TailscaleClient::new(api_key),
        }
    }

    fn now_str() -> String {
        chrono::Local::now().format("%H:%M:%S").to_string()
    }

    fn add_log(&mut self, event_type: &str, node_name: &str, node_ip: &str, message: String) {
        self.log_entries.push(LogEntry {
            timestamp: Self::now_str(),
            event_type: event_type.to_string(),
            node_name: node_name.to_string(),
            node_ip: node_ip.to_string(),
            message,
        });
        if self.log_entries.len() > 500 {
            self.log_entries.drain(..self.log_entries.len() - 500);
        }
    }

    pub async fn refresh(&mut self) {
        match self.client.fetch_devices().await {
            Ok(devices) => {
                self.error = None;
                self.build_nodes(&devices).await;
            }
            Err(e) => {
                let msg = format!("API error: {e}");
                self.add_log("error", "", "", msg.clone());
                self.error = Some(msg);
            }
        }
        self.loading = false;
    }

    async fn build_nodes(&mut self, devices: &[Device]) {
        let local_status = api::parse_local_status();

        // Extract tailnet name from first device FQDN
        if let Some(first) = devices.first() {
            let parts: Vec<&str> = first.name.splitn(3, '.').collect();
            if parts.len() >= 2 {
                self.tailnet_name = parts[1..].join(".");
                if self.tailnet_name.ends_with('.') {
                    self.tailnet_name.pop();
                }
            }
        }

        // Build initial nodes from API + local status
        let mut nodes: Vec<NodeInfo> = devices
            .iter()
            .map(|d| {
                let ip = d.ipv4().to_string();
                let local = local_status.get(&ip);

                NodeInfo {
                    name: d.short_name().to_string(),
                    ip: ip.clone(),
                    os: d.short_os().to_string(),
                    online: local.is_some_and(|l| l.online),
                    has_webclient: false,
                    tx_bytes: local.map(|l| l.tx_bytes),
                    rx_bytes: local.map(|l| l.rx_bytes),
                }
            })
            .collect();

        // Fetch remote metrics from online nodes in parallel (port 5252)
        let handles: Vec<(usize, JoinHandle<_>)> = nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.online)
            .map(|(i, n)| {
                let ip = n.ip.clone();
                (i, tokio::spawn(TailscaleClient::fetch_node_metrics(ip)))
            })
            .collect();

        for (i, handle) in handles {
            if let Some(traffic) = handle.await.ok().flatten() {
                nodes[i].has_webclient = true;
                if traffic.tx_bytes > 0 || traffic.rx_bytes > 0 {
                    nodes[i].tx_bytes = Some(traffic.tx_bytes);
                    nodes[i].rx_bytes = Some(traffic.rx_bytes);
                }
            }
        }

        // Detect connection/disconnection events
        for node in &nodes {
            let was_online = self.previous_online.get(&node.ip).copied();
            match (was_online, node.online) {
                (Some(false), true) => {
                    self.add_log("connect", &node.name, &node.ip, "came online".into());
                }
                (Some(true), false) => {
                    self.add_log("disconnect", &node.name, &node.ip, "went offline".into());
                }
                (None, true) => {
                    self.add_log("discovered", &node.name, &node.ip, "online".into());
                }
                (None, false) => {
                    self.add_log("discovered", &node.name, &node.ip, "offline".into());
                }
                _ => {}
            }
        }

        // Update previous state
        self.previous_online = nodes
            .iter()
            .map(|n| (n.ip.clone(), n.online))
            .collect();

        self.nodes = nodes;
        self.total_nodes = self.nodes.len();
        self.online_nodes = self.nodes.iter().filter(|n| n.online).count();
        self.sort_nodes();
    }

    pub fn set_sort_mode(&mut self, mode: SortMode) {
        self.sort_mode = mode;
        self.sort_nodes();
    }

    fn sort_nodes(&mut self) {
        match self.sort_mode {
            SortMode::Name => {
                self.nodes
                    .sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
            }
            SortMode::TxDesc => {
                self.nodes
                    .sort_by(|a, b| b.tx_bytes.unwrap_or(0).cmp(&a.tx_bytes.unwrap_or(0)));
            }
            SortMode::RxDesc => {
                self.nodes
                    .sort_by(|a, b| b.rx_bytes.unwrap_or(0).cmp(&a.rx_bytes.unwrap_or(0)));
            }
        }
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}
