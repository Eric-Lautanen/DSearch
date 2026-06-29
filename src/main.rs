mod api;
mod bootstrap;
mod cli;
mod config;
mod doctor;
mod model;
mod node;
mod proto;
mod sanitize;
mod scraper;
mod search;
mod service;
mod storage;
mod trust;
mod ui;

use clap::Parser;
use cli::api_client;
use cli::cmd::*;
use node::roles::NodeRole;
use node::server::Node;
use proto::cert;
use std::net::SocketAddr;
use std::net::TcpStream;
use std::path::Path;
use std::path::PathBuf;
use storage::Store;
use tracing::{info, warn};

fn default_data_dir() -> PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("dsearch")
}

#[tokio::main]
async fn main() {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("failed to install rustls crypto provider");

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let data_dir = cli.data_dir.clone().unwrap_or_else(default_data_dir);

    if let Err(e) = run_command(cli, data_dir).await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

async fn run_command(
    cli: Cli,
    data_dir: PathBuf,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match cli.command {
        Commands::Init {
            role,
            data_dir: init_dir,
        } => {
            let dir = init_dir.unwrap_or(data_dir);
            cmd_init(&dir, role.as_deref())?;
            Ok(())
        }
        Commands::Node { action } => cmd_node(action, &data_dir).await,
        Commands::Bootstrap { action } => cmd_bootstrap(action, &data_dir),
        Commands::Peers { action } => cmd_peers(action, &data_dir).await,
        Commands::Role { action } => cmd_role(action, &data_dir),
        Commands::Search {
            query,
            schema,
            limit,
            output,
        } => cmd_search(&query, schema, limit, &output, &data_dir),
        Commands::Record { action } => cmd_record(action, &data_dir),
        Commands::Service { action } => cmd_service(action, &data_dir),
        Commands::Tray { action } => cmd_tray(action, &data_dir),
        Commands::Config { action } => cmd_config(action, &data_dir),
        Commands::Identity { action } => cmd_identity(action, &data_dir),
        Commands::Scraper { action } => cmd_scraper(action, &data_dir).await,
        Commands::Gateway { action } => cmd_gateway(action, &data_dir),
        Commands::Doctor { output } => cmd_doctor(&data_dir, &output),
        Commands::Log { action } => cmd_log(action, &data_dir),
    }
}

fn cmd_init(
    data_dir: &PathBuf,
    role: Option<&str>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    std::fs::create_dir_all(data_dir)?;

    let (_signing_key, node_id, _cert_der, _key_der) = cert::load_or_generate_identity(data_dir)?;

    let config_path = data_dir.join("config.toml");
    if !config_path.exists() {
        // Build default config with the specified role baked in
        let mut config = config::DsearchConfig::default();
        if let Some(role_str) = role {
            config.node.role = role_str.to_string();
        }
        // Use default_config_toml which appends [meta] config_version,
        // but we need the role set — so write via save_config then append meta
        let toml_str = toml::to_string_pretty(&config)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;
        let full_config = format!(
            "{}\n[meta]\nconfig_version = {}\n",
            toml_str.trim_end(),
            config::CURRENT_CONFIG_VERSION
        );
        std::fs::write(&config_path, full_config)?;
        info!("Created default config.toml at {}", config_path.display());
    } else if let Some(role_str) = role {
        // Config already exists — just update the role
        if let Ok(mut config) = config::load_config(data_dir) {
            config.node.role = role_str.to_string();
            // Re-save preserving meta section
            let toml_str = toml::to_string_pretty(&config)
                .map_err(|e| format!("Failed to serialize config: {}", e))?;
            let full_config = format!(
                "{}\n[meta]\nconfig_version = {}\n",
                toml_str.trim_end(),
                config::CURRENT_CONFIG_VERSION
            );
            std::fs::write(&config_path, full_config)?;
        }
    }

    let bootstrap_path = data_dir.join("bootstrap.toml");
    if !bootstrap_path.exists() {
        let default_bootstrap = default_bootstrap_toml();
        std::fs::write(&bootstrap_path, default_bootstrap)?;
        info!(
            "Created default bootstrap.toml at {}",
            bootstrap_path.display()
        );
    }

    let providers_path = data_dir.join("search_providers.toml");
    if !providers_path.exists() {
        let default_providers = default_search_providers_toml();
        std::fs::write(&providers_path, default_providers)?;
        info!(
            "Created default search_providers.toml at {}",
            providers_path.display()
        );
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

async fn cmd_node(
    action: NodeAction,
    data_dir: &PathBuf,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        NodeAction::Start {
            headless,
            role,
            port,
        } => {
            std::fs::create_dir_all(data_dir)?;

            if let Err(e) = config::load_config(data_dir) {
                eprintln!("Config error: {}", e);
                std::process::exit(1);
            }

            let (signing_key, node_id, _cert_der, _key_der) =
                cert::load_or_generate_identity(data_dir)?;

            let role_str = role.as_deref().unwrap_or("light");
            let node_role = NodeRole::from_str(role_str).unwrap_or(NodeRole::Light);

            let quic_port = port.unwrap_or(7744);
            let listen_addr: SocketAddr = format!("0.0.0.0:{}", quic_port).parse()?;

            // Open store for API server
            let db = storage::open_store(data_dir)?;
            let cfg = config::load_config(data_dir)
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
            let mut store = Store::new(db, cfg.storage.clone());
            store.set_signing_key(std::sync::Arc::new(signing_key.clone()));
            let store = std::sync::Arc::new(store);

            let mut node = Node::new(
                signing_key,
                node_id.clone(),
                node_role,
                data_dir.clone(),
                listen_addr,
            );
            node.store = Some(store.clone());

            node.start().await?;
            info!("Node {} started", &node_id[..8]);

            // Start the local HTTP API server with port auto-increment
            let api_port = crate::api::local::start_api_server(
                data_dir.clone(),
                cfg.api.port,
                node_id.clone(),
                cfg.clone(),
                store.clone(),
            )
            .await?;
            info!("Local API started on port {}", api_port);

            // Start the background expiry sweeper
            let _sweeper = store.start_expiry_sweeper(std::time::Duration::from_secs(300));

            // Start the periodic scraper job scheduler
            let _scheduler = crate::scraper::scheduler::start_scheduler(
                store.clone(),
                cfg.clone(),
                data_dir.clone(),
            );

            // Start the gateway API if enabled
            if cfg.gateway.enabled {
                crate::api::gateway::start_gateway_server(
                    data_dir.clone(),
                    cfg.clone(),
                    store.clone(),
                    node_id.clone(),
                )
                .await?;
            }

            // Connect to bootstrap peers
            let peers = bootstrap::resolver::resolve_bootstrap_peers(data_dir);
            for peer in &peers {
                if peer.id == node_id {
                    continue;
                }
                match peer.addr.parse::<SocketAddr>() {
                    Ok(addr) => {
                        info!(
                            "Connecting to bootstrap peer {} at {}",
                            &peer.id[..8.min(peer.id.len())],
                            addr
                        );
                        match node.connect_to_peer(addr).await {
                            Ok(_) => info!(
                                "Connected to bootstrap peer {}",
                                &peer.id[..8.min(peer.id.len())]
                            ),
                            Err(e) => warn!(
                                "Failed to connect to bootstrap peer {}: {}",
                                &peer.id[..8.min(peer.id.len())],
                                e
                            ),
                        }
                    }
                    Err(e) => warn!("Invalid bootstrap address {}: {}", peer.addr, e),
                }
            }

            // Write PID file for `node stop` to find
            let pid_path = data_dir.join("node.pid");
            std::fs::write(&pid_path, std::process::id().to_string())?;

            if headless {
                // Headless mode: wait for Ctrl+C or shutdown signal
                let shutdown_path = data_dir.join("node.shutdown");
                let shutdown_watcher = tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        if shutdown_path.exists() {
                            info!("Shutdown signal file detected, initiating graceful shutdown");
                            let _ = std::fs::remove_file(&shutdown_path);
                            break;
                        }
                    }
                });

                println!("Node running (headless). Press Ctrl+C to stop.");
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {
                        info!("Ctrl+C received, shutting down...");
                    }
                    _ = shutdown_watcher => {
                        info!("Stop signal received, shutting down...");
                    }
                }
            } else {
                // UI mode: launch the egui UI
                // The node is already running in the tokio runtime.
                // eframe::run_native blocks, so we launch it from a new thread
                // while the tokio runtime continues on this thread.
                let ui_data_dir = data_dir.clone();
                let shutdown_path = data_dir.join("node.shutdown");

                // Spawn a task to watch for shutdown signals
                let shutdown_watcher = tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        if shutdown_path.exists() {
                            info!("Shutdown signal file detected, initiating graceful shutdown");
                            let _ = std::fs::remove_file(&shutdown_path);
                            break;
                        }
                    }
                });

                // Run the UI on the main thread (eframe requires the main thread on some platforms)
                // We need to exit the tokio context first, then run eframe.
                // The simplest approach: spawn the node work in background and run eframe here.
                // Since we're already inside #[tokio::main], we use a separate thread for eframe.
                std::mem::drop(shutdown_watcher);

                // Launch the UI — this blocks until the window closes
                ui::run_ui(ui_data_dir)?;
            }

            // Clean up PID file
            let _ = std::fs::remove_file(data_dir.join("node.pid"));

            info!("Shutting down...");
            node.stop().await?;
            println!("Node stopped.");
            Ok(())
        }
        NodeAction::Stop => {
            let pid_path = data_dir.join("node.pid");
            if pid_path.exists() {
                let pid_str = std::fs::read_to_string(&pid_path)?;
                let pid: u32 = pid_str
                    .trim()
                    .parse()
                    .map_err(|e| format!("Invalid PID in node.pid: {}", e))?;

                #[cfg(windows)]
                {
                    let shutdown_signal = data_dir.join("node.shutdown");
                    std::fs::write(&shutdown_signal, "stop")?;
                    println!(
                        "Stop signal sent to node (PID {}). Waiting for graceful shutdown...",
                        pid
                    );

                    let mut exited = false;
                    for _ in 0..20 {
                        std::thread::sleep(std::time::Duration::from_millis(500));
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
                        let _ = std::process::Command::new("taskkill")
                            .args(["/PID", &pid.to_string(), "/F"])
                            .output();
                        println!(
                            "Node (PID {}) force-killed (graceful shutdown timed out).",
                            pid
                        );
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
            // Try to reach the API
            if let Some(port) = api_client::api_is_reachable(data_dir) {
                match api_client::api_get(port, "/node") {
                    Ok(body) => println!("{}", body),
                    Err(e) => println!("Node API error: {}", e),
                }
            } else {
                println!("Node is not running (API not reachable).");
            }
            Ok(())
        }
        NodeAction::Restart => {
            // Stop the node first
            let pid_path = data_dir.join("node.pid");
            if pid_path.exists() {
                let pid_str = std::fs::read_to_string(&pid_path)?;
                let pid: u32 = pid_str
                    .trim()
                    .parse()
                    .map_err(|e| format!("Invalid PID in node.pid: {}", e))?;

                #[cfg(windows)]
                {
                    let shutdown_signal = data_dir.join("node.shutdown");
                    std::fs::write(&shutdown_signal, "restart")?;
                    println!("Stop signal sent to node (PID {}). Waiting...", pid);

                    let mut exited = false;
                    for _ in 0..20 {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        let output = std::process::Command::new("tasklist")
                            .args(["/FI", &format!("PID eq {}", pid), "/NH"])
                            .output()?;
                        let output_str = String::from_utf8_lossy(&output.stdout);
                        if !output_str.contains(&pid.to_string()) {
                            exited = true;
                            break;
                        }
                    }

                    if !exited {
                        let _ = std::process::Command::new("taskkill")
                            .args(["/PID", &pid.to_string(), "/F"])
                            .output();
                        println!("Node force-killed (graceful shutdown timed out).");
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

                println!("Node stopped. Start it again with `dsearch node start`.");
            } else {
                println!("No running node found. Use `dsearch node start` to start one.");
            }
            Ok(())
        }
    }
}

fn cmd_bootstrap(
    action: BootstrapAction,
    data_dir: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Try API first for list
    if let Some(port) = api_client::api_is_reachable(data_dir) {
        if let BootstrapAction::List = action {
            match api_client::api_get(port, "/bootstrap") {
                Ok(body) => {
                    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    if let Some(peers) = v.get("peers").and_then(|p| p.as_array()) {
                        if peers.is_empty() {
                            println!("No bootstrap peers configured.");
                        } else {
                            println!("Bootstrap peers ({}):", peers.len());
                            for (i, p) in peers.iter().enumerate() {
                                let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                                let addr = p.get("addr").and_then(|v| v.as_str()).unwrap_or("?");
                                let note = p.get("note").and_then(|v| v.as_str()).unwrap_or("");
                                println!("  {}. id={} addr={} note=\"{}\"", i + 1, id, addr, note);
                            }
                        }
                    }
                }
                Err(e) => eprintln!("API error: {}", e),
            }
            return Ok(());
        }
    }

    match action {
        BootstrapAction::List => {
            let peers = bootstrap::resolver::resolve_bootstrap_peers(data_dir);
            if peers.is_empty() {
                println!("No bootstrap peers configured.");
            } else {
                println!("Bootstrap peers:");
                for (i, p) in peers.iter().enumerate() {
                    println!(
                        "  {}. id={} addr={} note=\"{}\"",
                        i + 1,
                        p.id,
                        p.addr,
                        p.note
                    );
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
            let peers = bootstrap::resolver::resolve_bootstrap_peers(data_dir);
            if peers.is_empty() {
                println!("No bootstrap peers configured. Add peers with `dsearch bootstrap add`.");
                return Ok(());
            }

            println!(
                "Testing connectivity to {} bootstrap peer(s)...",
                peers.len()
            );
            let mut reachable = 0;
            let mut failed = 0;

            for peer in &peers {
                match peer.addr.parse::<SocketAddr>() {
                    Ok(addr) => {
                        let start = std::time::Instant::now();
                        match TcpStream::connect_timeout(&addr, std::time::Duration::from_secs(5)) {
                            Ok(_) => {
                                let elapsed = start.elapsed();
                                println!(
                                    "  ✓ {} ({}) — {}ms",
                                    &peer.id[..8.min(peer.id.len())],
                                    peer.addr,
                                    elapsed.as_millis()
                                );
                                reachable += 1;
                            }
                            Err(e) => {
                                println!(
                                    "  ✗ {} ({}) — {}",
                                    &peer.id[..8.min(peer.id.len())],
                                    peer.addr,
                                    e
                                );
                                failed += 1;
                            }
                        }
                    }
                    Err(e) => {
                        println!(
                            "  ✗ {} ({}) — invalid address: {}",
                            &peer.id[..8.min(peer.id.len())],
                            peer.addr,
                            e
                        );
                        failed += 1;
                    }
                }
            }

            println!(
                "\nResults: {}/{} reachable, {}/{} failed",
                reachable,
                peers.len(),
                failed,
                peers.len()
            );
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

async fn cmd_peers(
    action: PeersAction,
    data_dir: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        PeersAction::List { output } => {
            if let Some(port) = api_client::api_is_reachable(data_dir) {
                match api_client::api_get(port, "/peers") {
                    Ok(body) => {
                        if output == "json" {
                            println!("{}", body);
                        } else {
                            let v: serde_json::Value =
                                serde_json::from_str(&body).unwrap_or_default();
                            if let Some(peers) = v.get("peers").and_then(|p| p.as_array()) {
                                if peers.is_empty() {
                                    println!("No peers known.");
                                } else {
                                    println!("Peers ({}):", peers.len());
                                    for p in peers {
                                        let addr =
                                            p.get("addr").and_then(|v| v.as_str()).unwrap_or("?");
                                        println!("  {}", addr);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => eprintln!("API error: {}", e),
                }
            } else {
                println!("No peers known. Is the node running?");
            }
            Ok(())
        }
        PeersAction::Add { addr } => {
            // Try API first
            if let Some(port) = api_client::api_is_reachable(data_dir) {
                let body = serde_json::json!({"addr": addr}).to_string();
                if let Ok(resp) = api_client::api_post(port, "/peers/add", &body) {
                    let v: serde_json::Value = serde_json::from_str(&resp).unwrap_or_default();
                    if v.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                        println!("Peer {} added.", addr);
                    } else {
                        eprintln!("Failed to add peer: {}", addr);
                    }
                    return Ok(());
                }
            }
            println!("Peer add: node is not running (start the node first)");
            Ok(())
        }
        PeersAction::Ban { peer_id } => {
            // Try API first
            if let Some(port) = api_client::api_is_reachable(data_dir) {
                let body = serde_json::json!({"peer_id": peer_id}).to_string();
                if let Ok(resp) = api_client::api_post(port, "/peers/ban", &body) {
                    let v: serde_json::Value = serde_json::from_str(&resp).unwrap_or_default();
                    if v.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                        println!("Peer {} banned.", peer_id);
                    } else {
                        eprintln!("Failed to ban peer: {}", peer_id);
                    }
                    return Ok(());
                }
            }
            println!("Peer ban: node is not running (start the node first)");
            Ok(())
        }
        PeersAction::Unban { peer_id } => {
            // Try API first
            if let Some(port) = api_client::api_is_reachable(data_dir) {
                let body = serde_json::json!({"peer_id": peer_id}).to_string();
                if let Ok(resp) = api_client::api_post(port, "/peers/unban", &body) {
                    let v: serde_json::Value = serde_json::from_str(&resp).unwrap_or_default();
                    if v.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                        println!("Peer {} unbanned.", peer_id);
                    } else {
                        eprintln!("Failed to unban peer: {}", peer_id);
                    }
                    return Ok(());
                }
            }
            println!("Peer unban: node is not running (start the node first)");
            Ok(())
        }
    }
}

fn cmd_role(
    action: RoleAction,
    data_dir: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        RoleAction::List => {
            println!("Available roles:");
            for role in NodeRole::all() {
                println!("  {}", role);
            }
            Ok(())
        }
        RoleAction::Set { role } => {
            let r = NodeRole::from_str(&role).ok_or_else(|| format!("Unknown role: {}", role))?;
            std::fs::create_dir_all(data_dir)?;
            let mut cfg = config::load_config(data_dir)
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
            cfg.node.role = role.clone();
            config::save_config(data_dir, &cfg)
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
            println!("Role set to: {} (saved to config.toml)", r);
            Ok(())
        }
        RoleAction::Add { role } => {
            let r = NodeRole::from_str(&role).ok_or_else(|| format!("Unknown role: {}", role))?;
            std::fs::create_dir_all(data_dir)?;
            let mut cfg = config::load_config(data_dir)
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
            // Append role (comma-separated in the role string)
            let mut roles: Vec<String> = cfg
                .node
                .role
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            let role_str = r.to_string();
            if !roles.contains(&role_str) {
                roles.push(role_str.clone());
                cfg.node.role = roles.join(",");
                config::save_config(data_dir, &cfg)
                    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
                println!("Role added: {} (saved to config.toml)", r);
            } else {
                println!("Role {} already present", r);
            }
            Ok(())
        }
        RoleAction::Remove { role } => {
            let r = NodeRole::from_str(&role).ok_or_else(|| format!("Unknown role: {}", role))?;
            std::fs::create_dir_all(data_dir)?;
            let mut cfg = config::load_config(data_dir)
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
            let role_str = r.to_string();
            let mut roles: Vec<String> = cfg
                .node
                .role
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty() && s != &role_str)
                .collect();
            if roles.is_empty() {
                roles.push("light".to_string());
            }
            cfg.node.role = roles.join(",");
            config::save_config(data_dir, &cfg)
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
            println!("Role removed: {} (saved to config.toml)", r);
            Ok(())
        }
        RoleAction::Autodetect => {
            let result = crate::node::autonat::probe(7744, std::time::Duration::from_secs(5));
            if result.is_public {
                println!("✓ Node appears publicly reachable.");
                if let Some(ref addr) = result.public_addr {
                    println!("  Public address: {}", addr);
                }
                println!("  Recommendation: full or archive role");
            } else {
                println!("✗ Node is not publicly reachable.");
                println!("  Reason: {}", result.reason);
                println!("  Recommendation: light role (no inbound connections needed)");
            }
            Ok(())
        }
    }
}

fn cmd_config(
    action: ConfigAction,
    data_dir: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        ConfigAction::Show => {
            // Try API first
            if let Some(port) = api_client::api_is_reachable(data_dir) {
                if let Ok(body) = api_client::api_get(port, "/config") {
                    let v: serde_json::Value =
                        serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
                    println!("{}", serde_json::to_string_pretty(&v)?);
                    return Ok(());
                }
            }
            let config_path = data_dir.join("config.toml");
            if config_path.exists() {
                let contents = std::fs::read_to_string(&config_path)?;
                println!("{}", contents);
            } else {
                let defaults = config::default_config_toml();
                println!("{}", defaults);
            }
            Ok(())
        }
        ConfigAction::Set { key, value } => {
            // Try API first
            if let Some(port) = api_client::api_is_reachable(data_dir) {
                let body = serde_json::json!({"key": key, "value": value}).to_string();
                if api_client::api_post(port, "/config/set", &body).is_ok() {
                    println!("Set {} = {}", key, value);
                    return Ok(());
                }
            }
            std::fs::create_dir_all(data_dir)?;
            let mut cfg = config::load_config(data_dir)
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
            config::set_config_value(&mut cfg, &key, &value)
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
            config::save_config(data_dir, &cfg)
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
            println!("Set {} = {}", key, value);
            Ok(())
        }
        ConfigAction::Reset => {
            std::fs::create_dir_all(data_dir)?;
            let default_config = config::default_config_toml();
            let config_path = data_dir.join("config.toml");
            std::fs::write(&config_path, default_config)?;
            println!("Config reset to defaults.");
            Ok(())
        }
    }
}

fn cmd_identity(
    action: IdentityAction,
    data_dir: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        IdentityAction::Show => {
            // Try API first
            if let Some(port) = api_client::api_is_reachable(data_dir) {
                if let Ok(body) = api_client::api_get(port, "/identity") {
                    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    if let Some(node_id) = v.get("node_id").and_then(|v| v.as_str()) {
                        println!("Node ID: {}", node_id);
                    }
                    return Ok(());
                }
            }
            let key_path = data_dir.join("identity.key");
            if key_path.exists() {
                let key_bytes = std::fs::read(&key_path)?;
                if key_bytes.len() != 32 {
                    eprintln!(
                        "Invalid identity key (expected 32 bytes, got {})",
                        key_bytes.len()
                    );
                    std::process::exit(1);
                }
                let signing_key =
                    ed25519_dalek::SigningKey::from_bytes(key_bytes.as_slice().try_into().unwrap());
                let node_id = cert::node_id_from_pubkey(&signing_key.verifying_key());
                println!("Node ID: {}", node_id);
                println!(
                    "Public key: {}",
                    hex::encode(signing_key.verifying_key().to_bytes())
                );
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

fn cmd_record(
    action: RecordAction,
    data_dir: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Try API first for all record operations (node may have DB locked)
    if let Some(port) = api_client::api_is_reachable(data_dir) {
        match &action {
            RecordAction::Get { id, output } => {
                let path = format!("/record/{}", id);
                match api_client::api_get(port, &path) {
                    Ok(body) => {
                        if output == "json" {
                            println!("{}", body);
                        } else {
                            let v: serde_json::Value =
                                serde_json::from_str(&body).unwrap_or_default();
                            print_record_text(&v);
                        }
                        return Ok(());
                    }
                    Err(e) if e.contains("not found") => {
                        eprintln!("Record not found: {}", id);
                        std::process::exit(1);
                    }
                    Err(_) => {} // Fall through to direct access
                }
            }
            RecordAction::List {
                schema,
                limit,
                output,
            } => {
                let mut path = format!("/records?limit={}", limit);
                if let Some(ref s) = schema {
                    path.push_str(&format!("&schema={}", s));
                }
                if let Ok(body) = api_client::api_get(port, &path) {
                    if output == "json" {
                        println!("{}", body);
                    } else {
                        let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                        if let Some(records) = v.get("records").and_then(|r| r.as_array()) {
                            if records.is_empty() {
                                println!("No records found.");
                            } else {
                                println!("Records ({}):", records.len());
                                for r in records {
                                    let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                                    let schema =
                                        r.get("schema").and_then(|v| v.as_str()).unwrap_or("?");
                                    let created =
                                        r.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0);
                                    println!("  {}  schema={}  created={}", id, schema, created);
                                }
                            }
                        }
                    }
                    return Ok(());
                }
            }
            RecordAction::Insert { file } => {
                let json_str = std::fs::read_to_string(file)
                    .map_err(|e| format!("read {}: {}", file.display(), e))?;
                match api_client::api_post(port, "/record/insert", &json_str) {
                    Ok(body) => {
                        let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                        let action = v
                            .get("action")
                            .and_then(|v| v.as_str())
                            .unwrap_or("inserted");
                        let id = v.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                        println!("Record {} {}.", id, action);
                        return Ok(());
                    }
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            RecordAction::Pin { id } => {
                let body = serde_json::json!({"id": id}).to_string();
                match api_client::api_post(port, "/record/pin", &body) {
                    Ok(_) => {
                        println!("Record {} pinned.", id);
                        return Ok(());
                    }
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            RecordAction::Unpin { id } => {
                let body = serde_json::json!({"id": id}).to_string();
                match api_client::api_post(port, "/record/unpin", &body) {
                    Ok(_) => {
                        println!("Record {} unpinned.", id);
                        return Ok(());
                    }
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            RecordAction::Delete { id } => {
                let body = serde_json::json!({"id": id}).to_string();
                match api_client::api_post(port, "/record/delete", &body) {
                    Ok(_) => {
                        println!("Record {} deleted.", id);
                        return Ok(());
                    }
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            RecordAction::Announce { id } => {
                let body = serde_json::json!({"id": id}).to_string();
                match api_client::api_post(port, "/record/announce", &body) {
                    Ok(_) => {
                        println!("Record {} announced.", id);
                        return Ok(());
                    }
                    Err(e) => {
                        eprintln!("Error: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            RecordAction::Sweep => match api_client::api_post(port, "/record/sweep", "{}") {
                Ok(body) => {
                    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    let rec = v
                        .get("records_removed")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let ann = v
                        .get("announcements_removed")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    println!(
                        "Expiry sweep complete: removed {} records, {} announcements.",
                        rec, ann
                    );
                    return Ok(());
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            },
        }
    }

    // Direct store access fallback (only when API is not reachable)
    let db = storage::open_store(data_dir)?;
    let config = config::load_config(data_dir)
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
    let store = Store::new(db, config.storage);

    match action {
        RecordAction::Get { id, output } => {
            match store.get_record(&id)? {
                Some(record) => {
                    if output == "json" {
                        println!("{}", serde_json::to_string_pretty(&record)?);
                    } else {
                        println!("ID:          {}", record.id);
                        println!("Source URL:  {}", record.source_url);
                        println!("Source Hash: {}", record.source_hash);
                        println!("Schema:      {}", record.schema);
                        println!("Tags:        {}", record.tags.join(", "));
                        println!("Created At:  {}", record.created_at);
                        println!("Expires At:  {}", record.expires_at);
                        println!(
                            "Lifecycle:   {}",
                            if store.is_pinned(&id)? {
                                "pinned"
                            } else {
                                "ephemeral"
                            }
                        );
                        println!("Body:");
                        println!("{}", record.body);
                    }
                }
                None => {
                    eprintln!("Record not found: {}", id);
                    std::process::exit(1);
                }
            }
            Ok(())
        }
        RecordAction::List {
            schema,
            limit,
            output,
        } => {
            let records = store.list_records(schema.as_deref(), limit)?;
            if records.is_empty() {
                println!("No records found.");
            } else if output == "json" {
                println!("{}", serde_json::to_string_pretty(&records)?);
            } else {
                println!("Records ({}):", records.len());
                for r in &records {
                    let pinned = if store.is_pinned(&r.id)? {
                        " [pinned]"
                    } else {
                        ""
                    };
                    println!(
                        "  {}  schema={}  tags=[{}]  created={}  expires={}{}",
                        r.id,
                        r.schema,
                        r.tags.join(","),
                        r.created_at,
                        r.expires_at,
                        pinned
                    );
                }
            }
            Ok(())
        }
        RecordAction::Pin { id } => {
            if store.pin_record(&id)? {
                println!("Record {} pinned.", id);
            } else {
                eprintln!("Record not found: {}", id);
                std::process::exit(1);
            }
            Ok(())
        }
        RecordAction::Unpin { id } => {
            if store.unpin_record(&id)? {
                println!("Record {} unpinned.", id);
            } else {
                eprintln!("Record {} was not pinned.", id);
            }
            Ok(())
        }
        RecordAction::Delete { id } => {
            if store.delete_record(&id)? {
                println!("Record {} deleted.", id);
            } else {
                eprintln!("Record not found: {}", id);
                std::process::exit(1);
            }
            Ok(())
        }
        RecordAction::Insert { file } => {
            let json_str = std::fs::read_to_string(&file)
                .map_err(|e| format!("read {}: {}", file.display(), e))?;
            let record: crate::model::ContentRecord =
                serde_json::from_str(&json_str).map_err(|e| format!("parse record JSON: {}", e))?;
            let mut record = crate::sanitize::sanitize_record(&record)
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
            let result = store.insert_record(&mut record)?;
            match result {
                storage::records::InsertResult::Inserted => {
                    println!("Record {} inserted.", record.id)
                }
                storage::records::InsertResult::ReplacedNewer => println!(
                    "Record {} replaced older record with same source_hash.",
                    record.id
                ),
                storage::records::InsertResult::SkippedOlder => println!(
                    "Record {} skipped (older than existing with same source_hash).",
                    record.id
                ),
            }
            Ok(())
        }
        RecordAction::Sweep => {
            let (records, announcements) = store.sweep_once()?;
            println!(
                "Expiry sweep complete: removed {} records, {} announcements.",
                records, announcements
            );
            Ok(())
        }
        RecordAction::Announce { id } => match store.get_record(&id)? {
            Some(record) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let mut ann = crate::model::Announcement {
                    record_id: record.id.clone(),
                    source_hash: record.source_hash.clone(),
                    schema: record.schema.clone(),
                    tags: record.tags.clone(),
                    holder_addr: "127.0.0.1:7744".to_string(),
                    expires_at: if record.expires_at == 0 {
                        now + 86400
                    } else {
                        record.expires_at
                    },
                    sig: "".to_string(),
                };
                store.insert_announcement(&mut ann)?;
                println!("Record {} announced.", id);
                Ok(())
            }
            None => {
                eprintln!("Record not found: {}", id);
                std::process::exit(1);
            }
        },
    }
}
fn print_record_text(v: &serde_json::Value) {
    println!(
        "ID:          {}",
        v.get("id").and_then(|v| v.as_str()).unwrap_or("?")
    );
    println!(
        "Source URL:  {}",
        v.get("source_url").and_then(|v| v.as_str()).unwrap_or("?")
    );
    println!(
        "Source Hash: {}",
        v.get("source_hash").and_then(|v| v.as_str()).unwrap_or("?")
    );
    println!(
        "Schema:      {}",
        v.get("schema").and_then(|v| v.as_str()).unwrap_or("?")
    );
    println!(
        "Tags:        {}",
        v.get("tags")
            .and_then(|v| v.as_array())
            .map(|a| a
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", "))
            .unwrap_or_default()
    );
    println!(
        "Created At:  {}",
        v.get("created_at").and_then(|v| v.as_u64()).unwrap_or(0)
    );
    println!(
        "Expires At:  {}",
        v.get("expires_at").and_then(|v| v.as_u64()).unwrap_or(0)
    );
    println!("Body:");
    println!("{}", v.get("body").and_then(|v| v.as_str()).unwrap_or(""));
}

fn cmd_search(
    query: &str,
    schema: Option<String>,
    limit: u32,
    output: &str,
    data_dir: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Try API first
    if let Some(port) = api_client::api_is_reachable(data_dir) {
        let mut path = format!("/search?q={}", url_encode(query));
        if let Some(ref s) = schema {
            path.push_str(&format!("&schema={}", url_encode(s)));
        }
        path.push_str(&format!("&limit={}", limit));
        if let Ok(body) = api_client::api_get(port, &path) {
            if output == "json" {
                println!("{}", body);
            } else {
                let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                if let Some(results) = v.get("results").and_then(|r| r.as_array()) {
                    if results.is_empty() {
                        println!("No results found.");
                    } else {
                        println!("Search results ({}):", results.len());
                        for r in results {
                            let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                            let schema = r.get("schema").and_then(|v| v.as_str()).unwrap_or("?");
                            let source =
                                r.get("source_url").and_then(|v| v.as_str()).unwrap_or("?");
                            let body = r.get("body").and_then(|v| v.as_str()).unwrap_or("");
                            let snippet: String = body.chars().take(120).collect();
                            println!("  {}  schema={}  source={}", id, schema, source);
                            println!("    {}", snippet);
                        }
                    }
                }
            }
            return Ok(());
        }
    }

    // Direct store access fallback
    let db = storage::open_store(data_dir)?;
    let config = config::load_config(data_dir)
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
    let store = Store::new(db, config.storage);

    let effective_query = match schema {
        Some(s) => format!("schema:{} {}", s, query),
        None => query.to_string(),
    };

    let results = store.search_records(&effective_query, limit as usize)?;

    if output == "json" {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else if results.is_empty() {
        println!("No results found.");
    } else {
        println!("Search results ({}):", results.len());
        for r in &results {
            let tags = r.tags.join(",");
            println!(
                "  {}  schema={}  tags=[{}]  created={}  source={}",
                r.id, r.schema, tags, r.created_at, r.source_url
            );
            let snippet: String = r.body.chars().take(120).collect();
            println!("    {}", snippet);
        }
    }
    Ok(())
}

/// RFC 3986 compliant URL encoding for query parameters.
/// Encodes all characters except unreserved set: A-Z a-z 0-9 - _ . ~
fn url_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            b' ' => {
                result.push('+');
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}

async fn cmd_scraper(
    action: cli::cmd::ScraperAction,
    data_dir: &PathBuf,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        cli::cmd::ScraperAction::Add {
            name,
            source,
            target,
            refresh,
            interval_secs,
            lifecycle,
            ttl_secs,
        } => {
            std::fs::create_dir_all(data_dir)?;
            let config = config::load_config(data_dir)
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;

            let job = crate::model::ScrapeJob {
                name: name.clone(),
                source: crate::model::ScrapeSource::from_str(&source)
                    .ok_or_else(|| format!("Unknown source type: {}", source))?,
                target,
                transform: None,
                refresh: crate::model::RefreshPolicy::from_str(&refresh)
                    .ok_or_else(|| format!("Unknown refresh policy: {}", refresh))?,
                interval_secs,
                lifecycle: crate::model::Lifecycle::from_str(&lifecycle)
                    .ok_or_else(|| format!("Unknown lifecycle: {}", lifecycle))?,
                ttl_secs,
                max_results: None,
            };

            let mut cfg = config;
            cfg.scraper.jobs.push(job);
            config::save_config(data_dir, &cfg)
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;

            println!("Scraper job '{}' added.", name);
            Ok(())
        }
        cli::cmd::ScraperAction::List => {
            // Try API first
            if let Some(port) = api_client::api_is_reachable(data_dir) {
                if let Ok(body) = api_client::api_get(port, "/scraper") {
                    let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
                    if let Some(jobs) = v.get("jobs").and_then(|j| j.as_array()) {
                        if jobs.is_empty() {
                            println!("No scraper jobs configured.");
                        } else {
                            println!("Scraper jobs ({}):", jobs.len());
                            for j in jobs {
                                let name = j.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                                let source =
                                    j.get("source").and_then(|v| v.as_str()).unwrap_or("?");
                                let target =
                                    j.get("target").and_then(|v| v.as_str()).unwrap_or("?");
                                let refresh =
                                    j.get("refresh").and_then(|v| v.as_str()).unwrap_or("?");
                                let lifecycle =
                                    j.get("lifecycle").and_then(|v| v.as_str()).unwrap_or("?");
                                println!(
                                    "  {}  source={}  target={}  refresh={}  lifecycle={}",
                                    name, source, target, refresh, lifecycle
                                );
                            }
                        }
                    }
                    return Ok(());
                }
            }
            let config = config::load_config(data_dir)
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
            if config.scraper.jobs.is_empty() {
                println!("No scraper jobs configured.");
            } else {
                println!("Scraper jobs ({}):", config.scraper.jobs.len());
                for job in &config.scraper.jobs {
                    println!(
                        "  {}  source={}  target={}  refresh={}  lifecycle={}",
                        job.name, job.source, job.target, job.refresh, job.lifecycle
                    );
                }
            }
            Ok(())
        }
        cli::cmd::ScraperAction::Run { name } => {
            // Try API first
            if let Some(port) = api_client::api_is_reachable(data_dir) {
                let body = serde_json::json!({"name": name}).to_string();
                if let Ok(resp) = api_client::api_post(port, "/scraper/run", &body) {
                    let v: serde_json::Value = serde_json::from_str(&resp).unwrap_or_default();
                    if v.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                        let record_id = v.get("record_id").and_then(|v| v.as_str()).unwrap_or("?");
                        println!("Job '{}' completed: record {}", name, record_id);
                    } else {
                        eprintln!("Job '{}' failed", name);
                    }
                    return Ok(());
                }
            }
            let db = storage::open_store(data_dir)?;
            let config = config::load_config(data_dir)
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })?;
            let store = Store::new(db, config.storage);

            let job = config
                .scraper
                .jobs
                .iter()
                .find(|j| j.name == name)
                .ok_or_else(|| format!("Scraper job '{}' not found", name))?;

            let lifecycle_str = job.lifecycle.as_str();
            let result = crate::scraper::job::run_url_job(
                &store,
                &job.name,
                &job.target,
                lifecycle_str,
                job.ttl_secs,
            )
            .await?;

            if result.inserted {
                println!(
                    "Job '{}' completed: record {} inserted.",
                    result.job_name, result.record_id
                );
            } else if result.replaced {
                println!(
                    "Job '{}' completed: record {} replaced (dedup).",
                    result.job_name, result.record_id
                );
            } else {
                println!(
                    "Job '{}' completed: record {} skipped (older).",
                    result.job_name, result.record_id
                );
            }
            Ok(())
        }
        cli::cmd::ScraperAction::ProviderAdd { name, endpoint } => {
            match crate::scraper::discovery::providers::add_provider(data_dir, &name, &endpoint) {
                Ok(()) => println!("Provider '{}' added with endpoint {}", name, endpoint),
                Err(e) => eprintln!("Error: {}", e),
            }
            Ok(())
        }
        cli::cmd::ScraperAction::ProviderRemove { name } => {
            match crate::scraper::discovery::providers::remove_provider(data_dir, &name) {
                Ok(true) => println!("Provider '{}' removed.", name),
                Ok(false) => println!("Provider '{}' not found.", name),
                Err(e) => eprintln!("Error: {}", e),
            }
            Ok(())
        }
    }
}

fn cmd_gateway(
    action: GatewayAction,
    data_dir: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        GatewayAction::Create { nickname } => {
            let nickname = nickname.unwrap_or_else(crate::api::gateway_keys::generate_nickname);
            let key_store = crate::api::gateway_keys::GatewayKeyStore::new(data_dir.to_path_buf());
            match key_store.create_key(&nickname) {
                Ok((secret, info)) => {
                    println!("API key created:");
                    println!("  Nickname:    {}", info.nickname);
                    println!(
                        "  Secret:      {}  (save this — it won't be shown again)",
                        secret
                    );
                    println!("  Created at:  {}", info.created_at);
                }
                Err(e) => {
                    eprintln!("Error creating key: {}", e);
                    std::process::exit(1);
                }
            }
            Ok(())
        }
        GatewayAction::List => {
            let key_store = crate::api::gateway_keys::GatewayKeyStore::new(data_dir.to_path_buf());
            match key_store.list_keys() {
                Ok(keys) => {
                    if keys.is_empty() {
                        println!("No gateway API keys.");
                    } else {
                        println!("Gateway API keys ({}):", keys.len());
                        for k in &keys {
                            println!(
                                "  {}  created={}  last_used={}  requests={}",
                                k.nickname, k.created_at, k.last_used, k.request_count
                            );
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Error listing keys: {}", e);
                    std::process::exit(1);
                }
            }
            Ok(())
        }
        GatewayAction::Revoke { nickname } => {
            let key_store = crate::api::gateway_keys::GatewayKeyStore::new(data_dir.to_path_buf());
            match key_store.revoke_key(&nickname) {
                Ok(true) => println!("Key '{}' revoked.", nickname),
                Ok(false) => {
                    eprintln!("Key '{}' not found.", nickname);
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("Error revoking key: {}", e);
                    std::process::exit(1);
                }
            }
            Ok(())
        }
    }
}

fn default_bootstrap_toml() -> String {
    r#"# {data_dir}/bootstrap.toml
# Edit freely. Add community or private bootstrap nodes here.
# The built-in list is always tried alongside this file.
# Remove the built-in list entirely by setting use_defaults = false.

use_defaults = true
"#
    .to_string()
}

fn default_search_providers_toml() -> String {
    r#"# {data_dir}/search_providers.toml
# Configure keyword discovery search providers.
# Each provider defines a remote API endpoint that can be queried
# for content matching specific keywords.

[[providers]]
name = "default"
enabled = false
endpoint = "https://search.dsearch.network/v1"
"#
    .to_string()
}

fn cmd_tray(
    action: TrayAction,
    data_dir: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        TrayAction::Start => {
            // Launch the UI with a hidden window — only the tray icon is visible.
            // The eframe viewport builder supports `with_visible(false)`.
            ui::run_ui_tray(data_dir.to_path_buf())?;
            Ok(())
        }
        TrayAction::Stop => {
            // Send shutdown signal to the running node
            let shutdown_path = data_dir.join("node.shutdown");
            std::fs::write(&shutdown_path, "stop")?;
            println!("Stop signal sent to tray/node.");
            Ok(())
        }
    }
}

fn cmd_log(
    action: LogAction,
    data_dir: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        LogAction::Tail { level } => {
            let log_path = data_dir.join("dsearch.log");
            if !log_path.exists() {
                println!(
                    "No log file found at {}. Is the node running?",
                    log_path.display()
                );
                return Ok(());
            }

            let level_filter = match level.as_str() {
                "trace" => 0,
                "debug" => 1,
                "info" => 2,
                "warn" => 3,
                "error" => 4,
                _ => 2,
            };

            // Read existing content and print the last 50 lines
            let contents = std::fs::read_to_string(&log_path)?;
            let lines: Vec<&str> = contents.lines().collect();
            let start = lines.len().saturating_sub(50);
            for line in &lines[start..] {
                if log_line_matches_level(line, level_filter) {
                    println!("{}", line);
                }
            }

            // Tail new lines (poll every 500ms for up to 5 minutes)
            let mut offset = contents.len();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
            loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                if std::time::Instant::now() > deadline {
                    break;
                }
                let contents = match std::fs::read_to_string(&log_path) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                if contents.len() > offset {
                    let new_content = &contents[offset..];
                    for line in new_content.lines() {
                        if log_line_matches_level(line, level_filter) {
                            println!("{}", line);
                        }
                    }
                    offset = contents.len();
                }
            }
            Ok(())
        }
    }
}

fn log_line_matches_level(line: &str, min_level: usize) -> bool {
    let line_level = if line.contains("TRACE") {
        0
    } else if line.contains("DEBUG") {
        1
    } else if line.contains("INFO") {
        2
    } else if line.contains("WARN") {
        3
    } else if line.contains("ERROR") {
        4
    } else {
        2 // Default to info level for unrecognized lines
    };
    line_level >= min_level
}

fn cmd_service(
    action: ServiceAction,
    data_dir: &Path,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match action {
        ServiceAction::Install { headless } => {
            let msg = service::service_install(data_dir, headless)?;
            println!("{}", msg);
            Ok(())
        }
        ServiceAction::Enable => {
            let msg = service::service_enable(data_dir)?;
            println!("{}", msg);
            Ok(())
        }
        ServiceAction::Disable => {
            let msg = service::service_disable(data_dir)?;
            println!("{}", msg);
            Ok(())
        }
        ServiceAction::Status => {
            let msg = service::service_status(data_dir)?;
            println!("{}", msg);
            Ok(())
        }
        ServiceAction::Uninstall => {
            let msg = service::service_uninstall(data_dir)?;
            println!("{}", msg);
            Ok(())
        }
    }
}

fn cmd_doctor(
    data_dir: &Path,
    output: &str,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let checks = doctor::run_doctor(data_dir);
    if output == "json" {
        println!("{}", doctor::format_json(&checks));
    } else {
        println!("dsearch doctor\n");
        print!("{}", doctor::format_text(&checks));
    }

    // Exit with non-zero if any check failed
    let has_fail = checks.iter().any(|c| c.status == doctor::CheckStatus::Fail);
    if has_fail {
        std::process::exit(1);
    }
    Ok(())
}
