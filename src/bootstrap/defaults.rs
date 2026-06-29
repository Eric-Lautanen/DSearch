use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapPeer {
    pub id: String,
    pub addr: String,
    #[serde(default)]
    pub note: String,
}

/// Compiled-in default bootstrap peers.
/// These are the official DSearch network bootstrap nodes.
/// Updated each release. Users with custom bootstrap.toml are never affected.
pub fn default_bootstrap_peers() -> Vec<BootstrapPeer> {
    vec![
        BootstrapPeer {
            id: "dsearch_bootstrap_us_east_1".to_string(),
            addr: "bootstrap-us-east.dsearch.network:7744".to_string(),
            note: "official bootstrap US-East".to_string(),
        },
        BootstrapPeer {
            id: "dsearch_bootstrap_eu_west_1".to_string(),
            addr: "bootstrap-eu-west.dsearch.network:7744".to_string(),
            note: "official bootstrap EU-West".to_string(),
        },
        BootstrapPeer {
            id: "dsearch_bootstrap_ap_east_1".to_string(),
            addr: "bootstrap-ap-east.dsearch.network:7744".to_string(),
            note: "official bootstrap AP-East".to_string(),
        },
    ]
}
