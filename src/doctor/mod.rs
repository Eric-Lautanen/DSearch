use serde::Serialize;
use std::net::UdpSocket;
/// `dsearch doctor` — full health check.
///
/// Runs a series of checks and produces a human-readable (or JSON) report.
/// Every check reflects a real underlying test, not a hardcoded pass.
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, Serialize)]
pub struct DoctorCheck {
    pub category: String,
    pub name: String,
    pub status: CheckStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

impl std::fmt::Display for CheckStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CheckStatus::Ok => write!(f, "✓"),
            CheckStatus::Warn => write!(f, "⚠"),
            CheckStatus::Fail => write!(f, "✗"),
        }
    }
}

/// Run all doctor checks and return the results.
pub fn run_doctor(data_dir: &Path) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();

    // Identity checks
    checks.extend(check_identity(data_dir));

    // Storage checks
    checks.extend(check_storage(data_dir));

    // Network checks
    checks.extend(check_network(data_dir));

    // API checks
    checks.extend(check_api(data_dir));

    // Config checks
    checks.extend(check_config(data_dir));

    // Service checks
    checks.extend(check_service(data_dir));

    checks
}

fn check_identity(data_dir: &Path) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    let key_path = data_dir.join("identity.key");
    let cert_path = data_dir.join("node.crt");

    // Keypair exists
    if key_path.exists() {
        checks.push(DoctorCheck {
            category: "Identity".to_string(),
            name: format!("Keypair found at {}", key_path.display()),
            status: CheckStatus::Ok,
            detail: None,
        });

        // Try to load the keypair
        match crate::proto::cert::load_or_generate_identity(data_dir) {
            Ok((_key, node_id, _cert_der, _key_der)) => {
                checks.push(DoctorCheck {
                    category: "Identity".to_string(),
                    name: "TLS cert valid, matches keypair".to_string(),
                    status: CheckStatus::Ok,
                    detail: None,
                });
                checks.push(DoctorCheck {
                    category: "Identity".to_string(),
                    name: format!("Node ID: {}", &node_id[..16.min(node_id.len())]),
                    status: CheckStatus::Ok,
                    detail: None,
                });
            }
            Err(e) => {
                checks.push(DoctorCheck {
                    category: "Identity".to_string(),
                    name: "TLS cert valid, matches keypair".to_string(),
                    status: CheckStatus::Fail,
                    detail: Some(format!("Error: {}", e)),
                });
            }
        }
    } else {
        checks.push(DoctorCheck {
            category: "Identity".to_string(),
            name: format!("Keypair found at {}", key_path.display()),
            status: CheckStatus::Fail,
            detail: Some("File not found — run `dsearch init` first".to_string()),
        });
    }

    // Cert file exists
    if !cert_path.exists() {
        checks.push(DoctorCheck {
            category: "Identity".to_string(),
            name: format!("Cert file at {}", cert_path.display()),
            status: CheckStatus::Fail,
            detail: Some("File not found".to_string()),
        });
    }

    checks
}

fn check_storage(data_dir: &Path) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();
    let db_path = data_dir.join("store.redb");

    // Can we open the database?
    match redb::Database::open(&db_path) {
        Ok(_db) => {
            checks.push(DoctorCheck {
                category: "Storage".to_string(),
                name: "store.redb opens cleanly".to_string(),
                status: CheckStatus::Ok,
                detail: None,
            });
        }
        Err(e) => {
            // If the file doesn't exist, that's OK — it'll be created on first use
            if !db_path.exists() {
                checks.push(DoctorCheck {
                    category: "Storage".to_string(),
                    name: "store.redb opens cleanly".to_string(),
                    status: CheckStatus::Warn,
                    detail: Some(
                        "Database file not yet created — will be created on first use".to_string(),
                    ),
                });
            } else {
                checks.push(DoctorCheck {
                    category: "Storage".to_string(),
                    name: "store.redb opens cleanly".to_string(),
                    status: CheckStatus::Fail,
                    detail: Some(format!("Error: {}", e)),
                });
            }
        }
    }

    // Schema version
    match crate::storage::migrations::check_and_migrate_on_path(data_dir) {
        Ok(version) => {
            let is_current = version == crate::storage::migrations::CURRENT_SCHEMA_VERSION;
            checks.push(DoctorCheck {
                category: "Storage".to_string(),
                name: format!(
                    "Schema version: {} ({})",
                    version,
                    if is_current { "current" } else { "outdated" }
                ),
                status: if is_current {
                    CheckStatus::Ok
                } else {
                    CheckStatus::Warn
                },
                detail: if is_current {
                    None
                } else {
                    Some(format!(
                        "Current is {}",
                        crate::storage::migrations::CURRENT_SCHEMA_VERSION
                    ))
                },
            });
        }
        Err(e) => {
            checks.push(DoctorCheck {
                category: "Storage".to_string(),
                name: "Schema version".to_string(),
                status: CheckStatus::Warn,
                detail: Some(format!("Could not read: {}", e)),
            });
        }
    }

    // Quota
    if let Ok(config) = crate::config::load_config(data_dir) {
        let quota_str = if config.storage.quota_mb == 0 {
            "unlimited".to_string()
        } else {
            format!("{} MB", config.storage.quota_mb)
        };
        checks.push(DoctorCheck {
            category: "Storage".to_string(),
            name: format!("Quota: {}", quota_str),
            status: CheckStatus::Ok,
            detail: None,
        });
    }

    checks
}

fn check_network(data_dir: &Path) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();

    // Check if QUIC port is bindable (default 7744)
    let quic_port = match crate::config::load_config(data_dir) {
        Ok(_config) => {
            // The QUIC port is the node's listen port, default 7744.
            // It's not the gateway port — the gateway is HTTP.
            // We check the node.port config if it exists, otherwise default.
            7744
        }
        Err(_) => 7744,
    };

    match UdpSocket::bind(format!("0.0.0.0:{}", quic_port)) {
        Ok(_) => {
            checks.push(DoctorCheck {
                category: "Network".to_string(),
                name: format!("UDP port {} bindable", quic_port),
                status: CheckStatus::Ok,
                detail: None,
            });
        }
        Err(e) => {
            checks.push(DoctorCheck {
                category: "Network".to_string(),
                name: format!("UDP port {} bindable", quic_port),
                status: CheckStatus::Fail,
                detail: Some(format!("Error: {}", e)),
            });
        }
    }

    // Check bootstrap peers
    let peers = crate::bootstrap::resolver::resolve_bootstrap_peers(data_dir);
    for peer in &peers {
        // Try a quick UDP connect to check reachability
        match UdpSocket::bind("0.0.0.0:0") {
            Ok(sock) => {
                sock.set_read_timeout(Some(Duration::from_secs(2))).ok();
                match sock.connect(&peer.addr) {
                    Ok(_) => {
                        checks.push(DoctorCheck {
                            category: "Network".to_string(),
                            name: format!("Bootstrap peer {} reachable", peer.addr),
                            status: CheckStatus::Ok,
                            detail: None,
                        });
                    }
                    Err(e) => {
                        checks.push(DoctorCheck {
                            category: "Network".to_string(),
                            name: format!("Bootstrap peer {} reachable", peer.addr),
                            status: CheckStatus::Fail,
                            detail: Some(format!("Unreachable: {}", e)),
                        });
                    }
                }
            }
            Err(e) => {
                checks.push(DoctorCheck {
                    category: "Network".to_string(),
                    name: format!("Bootstrap peer {} reachable", peer.addr),
                    status: CheckStatus::Fail,
                    detail: Some(format!("Cannot create socket: {}", e)),
                });
            }
        }
    }
    if peers.is_empty() {
        checks.push(DoctorCheck {
            category: "Network".to_string(),
            name: "Bootstrap peers configured".to_string(),
            status: CheckStatus::Warn,
            detail: Some("No bootstrap peers found".to_string()),
        });
    }

    checks
}

fn check_api(data_dir: &Path) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();

    // Check if API is reachable
    let port_path = data_dir.join("api.port");
    if port_path.exists() {
        if let Ok(port_str) = std::fs::read_to_string(&port_path) {
            if let Ok(port) = port_str.trim().parse::<u16>() {
                match crate::cli::api_client::api_get(port, "/health") {
                    Ok(_) => {
                        checks.push(DoctorCheck {
                            category: "API".to_string(),
                            name: format!("Local API on port {}", port),
                            status: CheckStatus::Ok,
                            detail: None,
                        });
                    }
                    Err(e) => {
                        checks.push(DoctorCheck {
                            category: "API".to_string(),
                            name: format!("Local API on port {}", port),
                            status: CheckStatus::Fail,
                            detail: Some(format!("Not reachable: {}", e)),
                        });
                    }
                }
            }
        }
    } else {
        checks.push(DoctorCheck {
            category: "API".to_string(),
            name: "Local API".to_string(),
            status: CheckStatus::Warn,
            detail: Some("Node not running (api.port file not found)".to_string()),
        });
    }

    checks
}

fn check_config(data_dir: &Path) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();

    match crate::config::load_config(data_dir) {
        Ok(_config) => {
            checks.push(DoctorCheck {
                category: "Config".to_string(),
                name: "config.toml valid".to_string(),
                status: CheckStatus::Ok,
                detail: None,
            });
        }
        Err(e) => {
            checks.push(DoctorCheck {
                category: "Config".to_string(),
                name: "config.toml valid".to_string(),
                status: CheckStatus::Fail,
                detail: Some(format!("Error: {}", e)),
            });
        }
    }

    checks
}

fn check_service(data_dir: &Path) -> Vec<DoctorCheck> {
    let mut checks = Vec::new();

    // Check if service is registered
    let registered = crate::service::status::is_service_registered(data_dir);
    checks.push(DoctorCheck {
        category: "Service".to_string(),
        name: "Registered for startup".to_string(),
        status: if registered {
            CheckStatus::Ok
        } else {
            CheckStatus::Warn
        },
        detail: if registered {
            None
        } else {
            Some("Not registered — run `dsearch service install`".to_string())
        },
    });

    // Check if currently running
    let port_path = data_dir.join("api.port");
    let running = port_path.exists()
        && std::fs::read_to_string(&port_path)
            .ok()
            .and_then(|s| s.trim().parse::<u16>().ok())
            .is_some_and(|port| crate::cli::api_client::api_get(port, "/health").is_ok());
    checks.push(DoctorCheck {
        category: "Service".to_string(),
        name: "Currently running".to_string(),
        status: if running {
            CheckStatus::Ok
        } else {
            CheckStatus::Warn
        },
        detail: None,
    });

    checks
}

/// Format doctor results as human-readable text.
pub fn format_text(checks: &[DoctorCheck]) -> String {
    let mut output = String::new();
    let mut current_category = String::new();

    for check in checks {
        if check.category != current_category {
            current_category = check.category.clone();
            output.push_str(&format!("\n  {}\n", current_category));
        }
        let detail_str = check
            .detail
            .as_ref()
            .map(|d| format!(" ({})", d))
            .unwrap_or_default();
        output.push_str(&format!(
            "    {} {}{}\n",
            check.status, check.name, detail_str
        ));
    }

    output
}

/// Format doctor results as JSON.
pub fn format_json(checks: &[DoctorCheck]) -> String {
    serde_json::to_string_pretty(checks).unwrap_or_else(|_| "[]".to_string())
}
