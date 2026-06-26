use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapPeer {
    pub id: String,
    pub addr: String,
    #[serde(default)]
    pub note: String,
}

/// Compiled-in default bootstrap peers.
/// Updated each release. Users with custom bootstrap.toml are never affected.
pub fn default_bootstrap_peers() -> Vec<BootstrapPeer> {
    vec![
        BootstrapPeer {
            id: "placeholder_bootstrap_1".to_string(),
            addr: "bootstrap1.dsearch.network:7744".to_string(),
            note: "official bootstrap 1".to_string(),
        },
        BootstrapPeer {
            id: "placeholder_bootstrap_2".to_string(),
            addr: "bootstrap2.dsearch.network:7744".to_string(),
            note: "official bootstrap 2".to_string(),
        },
        BootstrapPeer {
            id: "placeholder_bootstrap_3".to_string(),
            addr: "bootstrap3.dsearch.network:7744".to_string(),
            note: "official bootstrap 3".to_string(),
        },
    ]
}
