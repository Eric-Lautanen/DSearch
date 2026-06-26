use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::model::ScrapeJob;

pub mod migrations;

/// Current config schema version.
/// Incremented when config.toml's structure changes.
/// NOT the same as the wire protocol version.
pub const CURRENT_CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DsearchConfig {
    #[serde(default)]
    pub node: NodeConfig,
    #[serde(default)]
    pub api: ApiConfig,
    #[serde(default)]
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub relay: RelayConfig,
    #[serde(default)]
    pub scraper: ScraperConfig,
    #[serde(default)]
    pub log: LogConfig,
    #[serde(default)]
    pub bootstrap: BootstrapConfig,
}

impl Default for DsearchConfig {
    fn default() -> Self {
        Self {
            node: NodeConfig::default(),
            api: ApiConfig::default(),
            gateway: GatewayConfig::default(),
            storage: StorageConfig::default(),
            relay: RelayConfig::default(),
            scraper: ScraperConfig::default(),
            log: LogConfig::default(),
            bootstrap: BootstrapConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    #[serde(default = "default_role")]
    pub role: String,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
    #[serde(default = "default_min_protocol_version")]
    pub min_protocol_version: u8,
    #[serde(default = "default_true")]
    pub ipv4: bool,
    #[serde(default = "default_true")]
    pub ipv6: bool,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            role: default_role(),
            max_connections: default_max_connections(),
            min_protocol_version: default_min_protocol_version(),
            ipv4: true,
            ipv6: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    #[serde(default = "default_api_port")]
    pub port: u16,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self { port: default_api_port() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_gateway_bind")]
    pub bind: String,
    #[serde(default = "default_rate_limit")]
    pub rate_limit_per_min: u32,
    #[serde(default)]
    pub require_api_key: bool,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bind: default_gateway_bind(),
            rate_limit_per_min: default_rate_limit(),
            require_api_key: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    #[serde(default)]
    pub quota_mb: u32,
    #[serde(default = "default_quota_action")]
    pub quota_action: String,
    #[serde(default = "default_tier2_max_mb")]
    pub tier2_max_mb: u32,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            quota_mb: 0,
            quota_action: default_quota_action(),
            tier2_max_mb: default_tier2_max_mb(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayConfig {
    #[serde(default = "default_bandwidth_limit")]
    pub bandwidth_limit_mbps: u32,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self { bandwidth_limit_mbps: default_bandwidth_limit() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScraperConfig {
    #[serde(default = "default_interval_secs")]
    pub default_interval_secs: u64,
    #[serde(default)]
    pub default_replicate: u32,
    #[serde(default = "default_lifecycle_str")]
    pub default_lifecycle: String,
    #[serde(default)]
    pub jobs: Vec<ScrapeJob>,
}

impl Default for ScraperConfig {
    fn default() -> Self {
        Self {
            default_interval_secs: default_interval_secs(),
            default_replicate: 0,
            default_lifecycle: default_lifecycle_str(),
            jobs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_log_output")]
    pub output: String,
    #[serde(default = "default_log_file")]
    pub file: String,
    #[serde(default = "default_max_size_mb")]
    pub max_size_mb: u32,
    #[serde(default = "default_rotate_count")]
    pub rotate_count: u32,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            output: default_log_output(),
            file: default_log_file(),
            max_size_mb: default_max_size_mb(),
            rotate_count: default_rotate_count(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapConfig {
    #[serde(default = "default_true")]
    pub use_defaults: bool,
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        Self { use_defaults: true }
    }
}

// Default value functions
fn default_role() -> String { "light".to_string() }
fn default_max_connections() -> u32 { 200 }
fn default_min_protocol_version() -> u8 { 1 }
fn default_api_port() -> u16 { 7743 }
fn default_gateway_bind() -> String { "0.0.0.0:7744".to_string() }
fn default_rate_limit() -> u32 { 60 }
fn default_quota_action() -> String { "evict_oldest".to_string() }
fn default_tier2_max_mb() -> u32 { 512 }
fn default_bandwidth_limit() -> u32 { 100 }
fn default_interval_secs() -> u64 { 3600 }
fn default_lifecycle_str() -> String { "ephemeral".to_string() }
fn default_log_level() -> String { "info".to_string() }
fn default_log_output() -> String { "stderr".to_string() }
fn default_log_file() -> String { "{data_dir}/dsearch.log".to_string() }
fn default_max_size_mb() -> u32 { 50 }
fn default_rotate_count() -> u32 { 3 }
fn default_true() -> bool { true }

/// Load config from data_dir/config.toml, or return defaults if missing.
pub fn load_config(data_dir: &Path) -> Result<DsearchConfig, String> {
    let config_path = data_dir.join("config.toml");
    if config_path.exists() {
        let contents = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read config.toml: {}", e))?;
        let config: DsearchConfig = toml::from_str(&contents)
            .map_err(|e| format!("Failed to parse config.toml: {}", e))?;

        // Check for future config_version
        if let Some(version) = extract_config_version(&contents) {
            if version > CURRENT_CONFIG_VERSION {
                return Err(format!(
                    "config_version {} is from a future version (current: {}). \
                     Downgrading is not supported — your data may be corrupted.",
                    version, CURRENT_CONFIG_VERSION
                ));
            }
        }

        Ok(config)
    } else {
        Ok(DsearchConfig::default())
    }
}

/// Extract config_version from raw TOML string (if present).
fn extract_config_version(toml_str: &str) -> Option<u32> {
    let mut in_meta = false;
    for line in toml_str.lines() {
        let trimmed = line.trim();
        if trimmed == "[meta]" {
            in_meta = true;
            continue;
        }
        if trimmed.starts_with('[') && trimmed != "[meta]" {
            in_meta = false;
            continue;
        }
        if in_meta && trimmed.starts_with("config_version") {
            if let Some(eq_pos) = trimmed.find('=') {
                let val = trimmed[eq_pos + 1..].trim();
                return val.parse::<u32>().ok();
            }
        }
    }
    None
}

/// Save config to data_dir/config.toml.
pub fn save_config(data_dir: &Path, config: &DsearchConfig) -> Result<(), String> {
    let config_path = data_dir.join("config.toml");
    let toml_str = toml::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    std::fs::write(&config_path, toml_str)
        .map_err(|e| format!("Failed to write config.toml: {}", e))?;
    Ok(())
}

/// Set a config key by dot-separated path (e.g. "node.role", "api.port").
/// Returns an error if the key is unknown.
pub fn set_config_value(config: &mut DsearchConfig, key: &str, value: &str) -> Result<(), String> {
    match key {
        "node.role" => {
            config.node.role = value.to_string();
        }
        "node.max_connections" => {
            config.node.max_connections = value.parse()
                .map_err(|_| format!("Invalid value for {}: expected u32", key))?;
        }
        "node.min_protocol_version" => {
            config.node.min_protocol_version = value.parse()
                .map_err(|_| format!("Invalid value for {}: expected u8", key))?;
        }
        "node.ipv4" => {
            config.node.ipv4 = value.parse()
                .map_err(|_| format!("Invalid value for {}: expected bool", key))?;
        }
        "node.ipv6" => {
            config.node.ipv6 = value.parse()
                .map_err(|_| format!("Invalid value for {}: expected bool", key))?;
        }
        "api.port" => {
            config.api.port = value.parse()
                .map_err(|_| format!("Invalid value for {}: expected u16", key))?;
        }
        "gateway.enabled" => {
            config.gateway.enabled = value.parse()
                .map_err(|_| format!("Invalid value for {}: expected bool", key))?;
        }
        "gateway.bind" => {
            config.gateway.bind = value.to_string();
        }
        "gateway.rate_limit_per_min" => {
            config.gateway.rate_limit_per_min = value.parse()
                .map_err(|_| format!("Invalid value for {}: expected u32", key))?;
        }
        "gateway.require_api_key" => {
            config.gateway.require_api_key = value.parse()
                .map_err(|_| format!("Invalid value for {}: expected bool", key))?;
        }
        "storage.quota_mb" => {
            config.storage.quota_mb = value.parse()
                .map_err(|_| format!("Invalid value for {}: expected u32", key))?;
        }
        "storage.quota_action" => {
            config.storage.quota_action = value.to_string();
        }
        "storage.tier2_max_mb" => {
            config.storage.tier2_max_mb = value.parse()
                .map_err(|_| format!("Invalid value for {}: expected u32", key))?;
        }
        "relay.bandwidth_limit_mbps" => {
            config.relay.bandwidth_limit_mbps = value.parse()
                .map_err(|_| format!("Invalid value for {}: expected u32", key))?;
        }
        "scraper.default_interval_secs" => {
            config.scraper.default_interval_secs = value.parse()
                .map_err(|_| format!("Invalid value for {}: expected u64", key))?;
        }
        "scraper.default_replicate" => {
            config.scraper.default_replicate = value.parse()
                .map_err(|_| format!("Invalid value for {}: expected u32", key))?;
        }
        "scraper.default_lifecycle" => {
            config.scraper.default_lifecycle = value.to_string();
        }
        "log.level" => {
            config.log.level = value.to_string();
        }
        "log.output" => {
            config.log.output = value.to_string();
        }
        "log.file" => {
            config.log.file = value.to_string();
        }
        "log.max_size_mb" => {
            config.log.max_size_mb = value.parse()
                .map_err(|_| format!("Invalid value for {}: expected u32", key))?;
        }
        "log.rotate_count" => {
            config.log.rotate_count = value.parse()
                .map_err(|_| format!("Invalid value for {}: expected u32", key))?;
        }
        "bootstrap.use_defaults" => {
            config.bootstrap.use_defaults = value.parse()
                .map_err(|_| format!("Invalid value for {}: expected bool", key))?;
        }
        _ => return Err(format!("Unknown config key: {}", key)),
    }
    Ok(())
}

/// Generate the default config.toml content as a string.
pub fn default_config_toml() -> String {
    let config = DsearchConfig::default();
    let mut toml_str = toml::to_string_pretty(&config).unwrap_or_default();
    toml_str.push_str(&format!("\n[meta]\nconfig_version = {}\n", CURRENT_CONFIG_VERSION));
    toml_str
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ScrapeSource, RefreshPolicy, Lifecycle};

    #[test]
    fn default_config_roundtrip() {
        let config = DsearchConfig::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: DsearchConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.node.role, "light");
        assert_eq!(parsed.api.port, 7743);
        assert_eq!(parsed.gateway.enabled, false);
        assert_eq!(parsed.storage.quota_mb, 0);
        assert_eq!(parsed.scraper.default_interval_secs, 3600);
        assert_eq!(parsed.log.level, "info");
        assert_eq!(parsed.bootstrap.use_defaults, true);
    }

    #[test]
    fn load_missing_config_returns_defaults() {
        let dir = std::env::temp_dir().join("dsearch_test_missing_config_2");
        let _ = std::fs::remove_dir_all(&dir);
        let config = load_config(&dir).unwrap();
        assert_eq!(config.node.role, "light");
    }

    #[test]
    fn set_config_value_works() {
        let mut config = DsearchConfig::default();
        set_config_value(&mut config, "node.role", "full").unwrap();
        assert_eq!(config.node.role, "full");
        set_config_value(&mut config, "api.port", "8888").unwrap();
        assert_eq!(config.api.port, 8888);
        set_config_value(&mut config, "gateway.enabled", "true").unwrap();
        assert_eq!(config.gateway.enabled, true);
    }

    #[test]
    fn set_config_unknown_key_fails() {
        let mut config = DsearchConfig::default();
        assert!(set_config_value(&mut config, "nonexistent.key", "val").is_err());
    }

    #[test]
    fn future_config_version_rejected() {
        let toml_str = r#"
[node]
role = "light"

[meta]
config_version = 999
"#;
        let version = extract_config_version(toml_str);
        assert_eq!(version, Some(999));
    }

    #[test]
    fn extract_config_version_missing() {
        let toml_str = r#"
[node]
role = "light"
"#;
        assert_eq!(extract_config_version(toml_str), None);
    }

    #[test]
    fn config_with_scraper_jobs() {
        let toml_str = r#"
[node]
role = "scraper"

[scraper]
default_interval_secs = 900
default_lifecycle = "ephemeral"

[[scraper.jobs]]
name = "local-weather"
source = "api"
target = "https://api.weather.example/v1/current"
transform = "weather_v1"
refresh = "interval"
interval_secs = 900
lifecycle = "ephemeral"
ttl_secs = 3600

[[scraper.jobs]]
name = "saved-article"
source = "url"
target = "https://example.com/article"
refresh = "once"
lifecycle = "pinned"
ttl_secs = 0
"#;
        let config: DsearchConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.node.role, "scraper");
        assert_eq!(config.scraper.jobs.len(), 2);
        assert_eq!(config.scraper.jobs[0].name, "local-weather");
        assert_eq!(config.scraper.jobs[0].source, ScrapeSource::Api);
        assert_eq!(config.scraper.jobs[0].refresh, RefreshPolicy::Interval);
        assert_eq!(config.scraper.jobs[0].lifecycle, Lifecycle::Ephemeral);
        assert_eq!(config.scraper.jobs[1].name, "saved-article");
        assert_eq!(config.scraper.jobs[1].source, ScrapeSource::Url);
        assert_eq!(config.scraper.jobs[1].refresh, RefreshPolicy::Once);
        assert_eq!(config.scraper.jobs[1].lifecycle, Lifecycle::Pinned);
    }
}
