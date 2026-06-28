use crate::bootstrap::defaults::{default_bootstrap_peers, BootstrapPeer};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapToml {
    #[serde(default = "default_true")]
    pub use_defaults: bool,
    #[serde(default)]
    pub peers: Vec<BootstrapPeer>,
}

fn default_true() -> bool {
    true
}

/// Resolve bootstrap peers: bootstrap.toml → DNS → compiled defaults.
pub fn resolve_bootstrap_peers(data_dir: &std::path::Path) -> Vec<BootstrapPeer> {
    let mut peers = Vec::new();

    // 1. Read bootstrap.toml from data_dir
    let toml_path = data_dir.join("bootstrap.toml");
    if toml_path.exists() {
        if let Ok(contents) = std::fs::read_to_string(&toml_path) {
            if let Ok(config) = toml::from_str::<BootstrapToml>(&contents) {
                peers.extend(config.peers);
                if !config.use_defaults {
                    return peers;
                }
            }
        }
    }

    // 2. DNS SRV lookup (placeholder for Phase 1)
    // _dsearch._udp.dsearch.network

    // 3. Compiled-in defaults
    let defaults = default_bootstrap_peers();
    for p in &defaults {
        if !peers.iter().any(|ep| ep.id == p.id) {
            peers.push(p.clone());
        }
    }

    peers
}

/// Write a bootstrap.toml with a specific peer entry.
pub fn write_bootstrap_peer(
    data_dir: &std::path::Path,
    id: &str,
    addr: &str,
    note: &str,
) -> Result<(), std::io::Error> {
    let toml_path = data_dir.join("bootstrap.toml");
    let mut config = if toml_path.exists() {
        let contents = std::fs::read_to_string(&toml_path)?;
        toml::from_str::<BootstrapToml>(&contents).unwrap_or_else(|_| BootstrapToml {
            use_defaults: true,
            peers: Vec::new(),
        })
    } else {
        BootstrapToml {
            use_defaults: true,
            peers: Vec::new(),
        }
    };

    config.peers.push(BootstrapPeer {
        id: id.to_string(),
        addr: addr.to_string(),
        note: note.to_string(),
    });

    let toml_str = toml::to_string_pretty(&config).unwrap_or_default();
    std::fs::write(&toml_path, toml_str)
}

/// Remove a bootstrap peer by id from bootstrap.toml.
pub fn remove_bootstrap_peer(data_dir: &std::path::Path, id: &str) -> Result<bool, std::io::Error> {
    let toml_path = data_dir.join("bootstrap.toml");
    if !toml_path.exists() {
        return Ok(false);
    }

    let contents = std::fs::read_to_string(&toml_path)?;
    let mut config = toml::from_str::<BootstrapToml>(&contents).unwrap_or_else(|_| BootstrapToml {
        use_defaults: true,
        peers: Vec::new(),
    });

    let before = config.peers.len();
    config.peers.retain(|p| p.id != id);
    if config.peers.len() == before {
        return Ok(false);
    }

    let toml_str = toml::to_string_pretty(&config).unwrap_or_default();
    std::fs::write(&toml_path, toml_str)?;
    Ok(true)
}
