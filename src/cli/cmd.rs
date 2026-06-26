use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "dsearch", version, about = "Decentralized search network")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Data directory
    #[arg(long, global = true)]
    pub data_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Initialize node (first-run setup)
    Init {
        /// Node role
        #[arg(long)]
        role: Option<String>,
        /// Data directory
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },

    /// Node lifecycle
    Node {
        #[command(subcommand)]
        action: NodeAction,
    },

    /// Bootstrap peer management
    Bootstrap {
        #[command(subcommand)]
        action: BootstrapAction,
    },

    /// Peer management
    Peers {
        #[command(subcommand)]
        action: PeersAction,
    },

    /// Role management
    Role {
        #[command(subcommand)]
        action: RoleAction,
    },

    /// Search
    Search {
        /// Search query
        query: String,
        /// Schema filter
        #[arg(long)]
        schema: Option<String>,
        /// Result limit
        #[arg(long, default_value = "20")]
        limit: u32,
        /// Output format
        #[arg(long, default_value = "text")]
        output: String,
    },

    /// Record management
    Record {
        #[command(subcommand)]
        action: RecordAction,
    },

    /// Service management
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },

    /// Tray icon
    Tray {
        #[command(subcommand)]
        action: TrayAction,
    },

    /// Config management
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Identity management
    Identity {
        #[command(subcommand)]
        action: IdentityAction,
    },

    /// Diagnostics
    Doctor {
        /// Output format
        #[arg(long, default_value = "text")]
        output: String,
    },

    /// Log streaming
    Log {
        #[command(subcommand)]
        action: LogAction,
    },
}

#[derive(Subcommand)]
pub enum NodeAction {
    /// Start the node
    Start {
        /// Run headless (no UI)
        #[arg(long)]
        headless: bool,
        /// Node role(s), comma-separated
        #[arg(long)]
        role: Option<String>,
        /// QUIC listen port (default: 7744)
        #[arg(long)]
        port: Option<u16>,
    },
    /// Stop the node
    Stop,
    /// Show node status
    Status,
    /// Restart the node
    Restart,
}

#[derive(Subcommand)]
pub enum BootstrapAction {
    /// List bootstrap peers
    List,
    /// Add a bootstrap peer
    Add {
        #[arg(long)]
        id: String,
        #[arg(long)]
        addr: String,
        #[arg(long, default_value = "")]
        note: String,
    },
    /// Remove a bootstrap peer
    Remove {
        #[arg(long)]
        id: String,
    },
    /// Test bootstrap peer connectivity
    Test,
    /// Reset to defaults
    Reset,
}

#[derive(Subcommand)]
pub enum PeersAction {
    /// List known peers
    List {
        #[arg(long, default_value = "text")]
        output: String,
    },
    /// Add a peer
    Add {
        /// Peer multiaddr
        addr: String,
    },
    /// Ban a peer
    Ban {
        peer_id: String,
    },
    /// Unban a peer
    Unban {
        peer_id: String,
    },
}

#[derive(Subcommand)]
pub enum RoleAction {
    /// List available roles
    List,
    /// Set role
    Set {
        role: String,
    },
    /// Add a role
    Add {
        role: String,
    },
    /// Remove a role
    Remove {
        role: String,
    },
    /// Auto-detect best role
    Autodetect,
}

#[derive(Subcommand)]
pub enum RecordAction {
    /// Get a record by ID
    Get {
        id: String,
        #[arg(long, default_value = "text")]
        output: String,
    },
    /// List records
    List {
        #[arg(long)]
        schema: Option<String>,
        #[arg(long, default_value = "50")]
        limit: usize,
    },
    /// Pin a record
    Pin { id: String },
    /// Unpin a record
    Unpin { id: String },
    /// Delete a record
    Delete { id: String },
    /// Insert a record from a JSON file
    Insert {
        /// Path to JSON file containing the record
        file: PathBuf,
    },
    /// Run expiry sweep (remove expired records)
    Sweep,
    /// Announce a record
    Announce { id: String },
}

#[derive(Subcommand)]
pub enum ServiceAction {
    /// Install as OS service
    Install {
        #[arg(long)]
        headless: bool,
    },
    /// Enable start on boot
    Enable,
    /// Disable start on boot
    Disable,
    /// Show service status
    Status,
    /// Uninstall service
    Uninstall,
}

#[derive(Subcommand)]
pub enum TrayAction {
    /// Start tray icon
    Start,
    /// Stop tray icon
    Stop,
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Show current config
    Show,
    /// Set a config key
    Set {
        key: String,
        value: String,
    },
    /// Reset config to defaults
    Reset,
}

#[derive(Subcommand)]
pub enum IdentityAction {
    /// Show node identity
    Show,
    /// Export identity to file
    Export {
        path: PathBuf,
    },
    /// Import identity from file
    Import {
        path: PathBuf,
    },
}

#[derive(Subcommand)]
pub enum LogAction {
    /// Tail log output
    Tail {
        #[arg(long, default_value = "info")]
        level: String,
    },
}
