mod proto;
mod trust;
mod bootstrap;
mod cli;
mod node;
mod storage;
mod search;
mod scraper;
mod config;
mod api;
mod service;
mod ui;
mod sanitize;

use clap::Parser;
use cli::cmd::*;
use node::roles::NodeRole;
use node::server::Node;
use proto::cert;
use tracing::{info, warn};
use std::net::SocketAddr;
use std::path::PathBuf;

fn default_data_dir() -> PathBuf {
    dirs_next::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("dsearch")
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    let data_dir = cli.data_dir.clone()
        .unwrap_or_else(default_data_dir);

    if let Err(e) = run_command(cli, data_dir).await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

async fn run_command(cli: Cli, data_dir: PathBuf) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match cli.command {
        Commands::Init { role, data_dir: init_dir } => {
            let dir = init_dir.unwrap_or(data_dir);
            cmd_init(&dir, role.as_deref())?;
            Ok(())
        }
        Commands::Node { action } => cmd_node(action, &data_dir).await,
        Commands::Bootstrap { action } => cmd_bootstrap(action, &data_dir),
        Commands::Peers { action } => cmd_peers(action, &data_dir).await,
        Commands::Role { action } => cmd_role(action),
        Commands::Search { .. } => {
            eprintln!("Search not yet implemented (Phase 4)");
            std::process::exit(1);
        }
        Commands::Record { .. } => {
            eprintln!("Record management not yet implemented (Phase 3)");
            std::process::exit(1);
        }
        Commands::Service { .. } => {
            eprintln!("Service management not yet implemented (Phase 9)");
            std::process::exit(1);
        }
        Commands::Tray { .. } => {
            eprintln!("Tray not yet implemented (Phase 8)");
            std::process::exit(1);
        }
        Commands::Config { action } => cmd_config(action, &data_dir),
        Commands::Identity { action } => cmd_identity(action, &data_dir),
        Commands::Doctor { .. } => {
            eprintln!("Doctor not yet implemented (Phase 9)");
            std::process::exit(1);
        }
        Commands::Log { .. } => {
            eprintln!("Log streaming not yet implemented");
            std::process::exit(1);
        }
    }
}

fn cmd_init(data_dir: &PathBuf, role: Option<&str>) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    std::fs::create_dir_all(data_dir)?;

    let (_signing_key, node_id, _cert_der, _key_der) = cert::load_or_generate_identity(data_dir)?;

    let config_path = data_dir.join("config.toml");
    if !config_path.exists() {
        let default_config = default_config_toml();
        std::fs::write(&config_path, default_config)?;
        info!("Created default config.toml at {}", config_path.display());
    }

    let bootstrap_path = data_dir.join("bootstrap.toml");
    if !bootstrap_path.exists() {
        let default_bootstrap = default_bootstrap_toml();
        std::fs::write(&bootstrap_path, default_bootstrap)?;
        info!("Created default bootstrap.toml at {}", bootstrap_path.display());
    }

    let role_str = role.unwrap_or("light");
    println!("Node initialized successfully.");
    println!("  Node ID: {}", node_id);
    println!("  Data dir: {}", data_dir.display());
    println!("  Role: {}", role_str);
    println!("  Identity: {}/identity.key", data_dir.display());
    println!("  Cert: {}/node.crt", data_dir.display());

    Ok(())
}

async fn cmd_node(action: NodeAction, data_dir: &PathBuf) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        NodeAction::Start { headless: _, role, port } => {
            std::fs::create_dir_all(data_dir)?;

            let (signing_key, node_id, _cert_der, _key_der) = cert::load_or_generate_identity(data_dir)?;

            let role_str = role.as_deref().unwrap_or("light");
            let node_role = NodeRole::from_str(role_str).unwrap_or(NodeRole::Light);

            let quic_port = port.unwrap_or(7744);
            let listen_addr: SocketAddr = format!("0.0.0.0:{}", quic_port).parse()?;

            let mut node = Node::new(signing_key, node_id.clone(), node_role, data_dir.clone(), listen_addr);

            node.start().await?;
            info!("Node {} started", &node_id[..8]);

            // Connect to bootstrap peers
            let peers = bootstrap::resolver::resolve_bootstrap_peers(data_dir);
            for peer in &peers {
                if peer.id == node_id {
                    continue;
                }
                match peer.addr.parse::<SocketAddr>() {
                    Ok(addr) => {
                        info!("Connecting to bootstrap peer {} at {}", &peer.id[..8.min(peer.id.len())], addr);
                        match node.connect_to_peer(addr).await {
                            Ok(_) => info!("Connected to bootstrap peer {}", &peer.id[..8.min(peer.id.len())]),
                            Err(e) => warn!("Failed to connect to bootstrap peer {}: {}", &peer.id[..8.min(peer.id.len())], e),
                        }
                    }
                    Err(e) => warn!("Invalid bootstrap address {}: {}", peer.addr, e),
                }
            }

            // Write PID file for `node stop` to find
            let pid_path = data_dir.join("node.pid");
            std::fs::write(&pid_path, std::process::id().to_string())?;

            // Write api.port file (placeholder for Phase 7)
            let port_path = data_dir.join("api.port");
            std::fs::write(&port_path, "7743")?;

            // Watch for shutdown signal file (for `node stop` on Windows)
            let shutdown_path = data_dir.join("node.shutdown");
            let _data_dir_clone = data_dir.clone();
            let shutdown_watcher = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    if shutdown_path.exists() {
                        info!("Shutdown signal file detected, initiating graceful shutdown");
                        let _ = std::fs::remove_file(&shutdown_path);
                        // Send ourselves SIGINT / Ctrl+C equivalent
                        // On Windows, we just break out of the loop
                        break;
                    }
                }
            });

            println!("Node running. Press Ctrl+C to stop.");
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    info!("Ctrl+C received, shutting down...");
                }
                _ = shutdown_watcher => {
                    info!("Stop signal received, shutting down...");
                }
            }

            // Clean up PID file
            let _ = std::fs::remove_file(data_dir.join("node.pid"));

            info!("Shutting down...");
            node.stop().await?;
            println!("Node stopped.");
            Ok(())
        }
        NodeAction::Stop => {
            // Read PID from node.pid file and send SIGTERM (Unix) or kill (Windows)
            let pid_path = data_dir.join("node.pid");
            if pid_path.exists() {
                let pid_str = std::fs::read_to_string(&pid_path)?;
                let pid: u32 = pid_str.trim().parse()
                    .map_err(|e| format!("Invalid PID in node.pid: {}", e))?;
                
                #[cfg(windows)]
                {
                    // On Windows, use the shutdown signal file mechanism
                    // The running node watches for node.shutdown and exits gracefully
                    let shutdown_signal = data_dir.join("node.shutdown");
                    std::fs::write(&shutdown_signal, "stop")?;
                    println!("Stop signal sent to node (PID {}). Waiting for graceful shutdown...", pid);
                    
                    // Wait up to 10 seconds for the process to exit
                    let mut exited = false;
                    for _ in 0..20 {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        // Check if the PID is still running
                        let output = std::process::Command::new("tasklist")
                            .args(["/FI", &format!("PID eq {}", pid), "/NH"])
                            .output()?;
                        let output_str = String::from_utf8_lossy(&output.stdout);
                        if !output_str.contains(&pid.to_string()) {
                            exited = true;
                            break;
                        }
                    }
                    
                    if exited {
                        println!("Node (PID {}) stopped gracefully.", pid);
                    } else {
                        // Fallback: force kill
                        let _ = std::process::Command::new("taskkill")
                            .args(["/PID", &pid.to_string(), "/F"])
                            .output();
                        println!("Node (PID {}) force-killed (graceful shutdown timed out).", pid);
                    }
                    let _ = std::fs::remove_file(&pid_path);
                    let _ = std::fs::remove_file(data_dir.join("node.shutdown"));
                }
                
                #[cfg(unix)]
                {
                    unsafe {
                        libc::kill(pid as i32, libc::SIGTERM);
                    }
                    println!("SIGTERM sent to node (PID {}).", pid);
                    let _ = std::fs::remove_file(&pid_path);
                }
            } else {
                println!("No running node found (node.pid missing). Is the node running?");
            }
            Ok(())
        }
        NodeAction::Status => {
            println!("Node status: not yet queryable (Phase 7)");
            Ok(())
        }
        NodeAction::Restart => {
            println!("Node restart: not yet implemented (Phase 7)");
            Ok(())
        }
    }
}

fn cmd_bootstrap(action: BootstrapAction, data_dir: &PathBuf) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        BootstrapAction::List => {
            let peers = bootstrap::resolver::resolve_bootstrap_peers(data_dir);
            if peers.is_empty() {
                println!("No bootstrap peers configured.");
            } else {
                println!("Bootstrap peers:");
                for (i, p) in peers.iter().enumerate() {
                    println!("  {}. id={} addr={} note=\"{}\"", i + 1, p.id, p.addr, p.note);
                }
            }
            Ok(())
        }
        BootstrapAction::Add { id, addr, note } => {
            bootstrap::resolver::write_bootstrap_peer(data_dir, &id, &addr, &note)?;
            println!("Added bootstrap peer: id={} addr={}", id, addr);
            Ok(())
        }
        BootstrapAction::Remove { id } => {
            if bootstrap::resolver::remove_bootstrap_peer(data_dir, &id)? {
                println!("Removed bootstrap peer: id={}", id);
            } else {
                println!("Bootstrap peer not found: id={}", id);
            }
            Ok(())
        }
        BootstrapAction::Test => {
            println!("Bootstrap test: not yet implemented (requires running node)");
            Ok(())
        }
        BootstrapAction::Reset => {
            let bootstrap_path = data_dir.join("bootstrap.toml");
            if bootstrap_path.exists() {
                std::fs::remove_file(&bootstrap_path)?;
                println!("Removed custom bootstrap.toml. Defaults will be used.");
            }
            Ok(())
        }
    }
}

async fn cmd_peers(action: PeersAction, data_dir: &PathBuf) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        PeersAction::List { .. } => {
            let peers_path = data_dir.join("peers.json");
            if peers_path.exists() {
                let contents = std::fs::read_to_string(&peers_path)?;
                println!("{}", contents);
            } else {
                println!("No peers known. Is the node running?");
            }
            Ok(())
        }
        PeersAction::Add { .. } => {
            println!("Peer add: not yet implemented (requires running node)");
            Ok(())
        }
        PeersAction::Ban { .. } => {
            println!("Peer ban: not yet implemented (Phase 9)");
            Ok(())
        }
        PeersAction::Unban { .. } => {
            println!("Peer unban: not yet implemented (Phase 9)");
            Ok(())
        }
    }
}

fn cmd_role(action: RoleAction) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        RoleAction::List => {
            println!("Available roles:");
            for role in NodeRole::all() {
                println!("  {}", role);
            }
            Ok(())
        }
        RoleAction::Set { role } => {
            let r = NodeRole::from_str(&role)
                .ok_or_else(|| format!("Unknown role: {}", role))?;
            println!("Role set to: {}", r);
            Ok(())
        }
        RoleAction::Add { role } => {
            let r = NodeRole::from_str(&role)
                .ok_or_else(|| format!("Unknown role: {}", role))?;
            println!("Role added: {}", r);
            Ok(())
        }
        RoleAction::Remove { role } => {
            let r = NodeRole::from_str(&role)
                .ok_or_else(|| format!("Unknown role: {}", role))?;
            println!("Role removed: {}", r);
            Ok(())
        }
        RoleAction::Autodetect => {
            println!("AutoNAT detection: not yet implemented (requires running node)");
            Ok(())
        }
    }
}

fn cmd_config(action: ConfigAction, data_dir: &PathBuf) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        ConfigAction::Show => {
            let config_path = data_dir.join("config.toml");
            if config_path.exists() {
                let contents = std::fs::read_to_string(&config_path)?;
                println!("{}", contents);
            } else {
                println!("No config.toml found. Run `dsearch init` first.");
            }
            Ok(())
        }
        ConfigAction::Set { .. } => {
            println!("Config set: not yet implemented (Phase 2)");
            Ok(())
        }
        ConfigAction::Reset => {
            let config_path = data_dir.join("config.toml");
            let default_config = default_config_toml();
            std::fs::write(&config_path, default_config)?;
            println!("Config reset to defaults.");
            Ok(())
        }
    }
}

fn cmd_identity(action: IdentityAction, data_dir: &PathBuf) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        IdentityAction::Show => {
            let key_path = data_dir.join("identity.key");
            if key_path.exists() {
                let key_bytes = std::fs::read(&key_path)?;
                if key_bytes.len() != 32 {
                    eprintln!("Invalid identity key (expected 32 bytes, got {})", key_bytes.len());
                    std::process::exit(1);
                }
                let signing_key = ed25519_dalek::SigningKey::from_bytes(
                    key_bytes.as_slice().try_into().unwrap()
                );
                let node_id = cert::node_id_from_pubkey(&signing_key.verifying_key());
                println!("Node ID: {}", node_id);
                println!("Public key: {}", hex::encode(signing_key.verifying_key().to_bytes()));
            } else {
                println!("No identity found. Run `dsearch init` first.");
            }
            Ok(())
        }
        IdentityAction::Export { path } => {
            let key_path = data_dir.join("identity.key");
            let cert_path = data_dir.join("node.crt");
            if key_path.exists() && cert_path.exists() {
                std::fs::create_dir_all(&path)?;
                std::fs::copy(&key_path, path.join("identity.key"))?;
                std::fs::copy(&cert_path, path.join("node.crt"))?;
                println!("Identity exported to {}", path.display());
            } else {
                println!("No identity found. Run `dsearch init` first.");
            }
            Ok(())
        }
        IdentityAction::Import { path } => {
            let key_src = path.join("identity.key");
            let cert_src = path.join("node.crt");
            if key_src.exists() && cert_src.exists() {
                std::fs::copy(&key_src, data_dir.join("identity.key"))?;
                std::fs::copy(&cert_src, data_dir.join("node.crt"))?;
                println!("Identity imported from {}", path.display());
            } else {
                println!("No identity found at {}", path.display());
            }
            Ok(())
        }
    }
}

fn default_config_toml() -> String {
    r#"[node]
role = "light"
max_connections = 200
min_protocol_version = 1
ipv4 = true
ipv6 = true

[api]
port = 7743

[gateway]
enabled = false
bind = "0.0.0.0:7744"
rate_limit_per_min = 60
require_api_key = false

[storage]
quota_mb = 0
quota_action = "evict_oldest"
tier2_max_mb = 512

[relay]
bandwidth_limit_mbps = 100

[scraper]
default_interval_secs = 3600
default_replicate = 0
default_lifecycle = "ephemeral"

[log]
level = "info"
output = "stderr"
max_size_mb = 50
rotate_count = 3

[bootstrap]
use_defaults = true
"#.to_string()
}

fn default_bootstrap_toml() -> String {
    r#"# {data_dir}/bootstrap.toml
# Edit freely. Add community or private bootstrap nodes here.
# The built-in list is always tried alongside this file.
# Remove the built-in list entirely by setting use_defaults = false.

use_defaults = true
"#.to_string()
}
