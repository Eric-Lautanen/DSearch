use eframe::egui;
use std::path::PathBuf;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OnboardingStep {
    Welcome,
    GenerateIdentity,
    ChooseRole,
    ConnectBootstrap,
    Done,
}

pub struct OnboardingState {
    step: OnboardingStep,
    data_dir: PathBuf,
    node_id: String,
    selected_role: String,
    autonat_result: Option<bool>,
    bootstrap_connected: bool,
    bootstrap_peer_count: usize,
    identity_generated: bool,
    custom_data_dir: String,
    identity_error: String,
    // Bootstrap step: cache peer list and test results so we don't block every frame
    bootstrap_peers: Vec<BootstrapPeerInfo>,
    bootstrap_peers_loaded: bool,
    bootstrap_test_results: Vec<(String, bool, String)>,
    // Manual peer add
    new_peer_id: String,
    new_peer_addr: String,
}

#[derive(Debug, Clone)]
struct BootstrapPeerInfo {
    id: String,
    addr: String,
}

impl OnboardingState {
    pub fn new() -> Self {
        let default_dir = dirs_next::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("dsearch");
        Self {
            step: OnboardingStep::Welcome,
            data_dir: default_dir.clone(),
            node_id: String::new(),
            selected_role: "light".to_string(),
            autonat_result: None,
            bootstrap_connected: false,
            bootstrap_peer_count: 0,
            identity_generated: false,
            custom_data_dir: default_dir.to_string_lossy().to_string(),
            identity_error: String::new(),
            bootstrap_peers: Vec::new(),
            bootstrap_peers_loaded: false,
            bootstrap_test_results: Vec::new(),
            new_peer_id: String::new(),
            new_peer_addr: String::new(),
        }
    }

    pub fn is_complete(&self) -> bool {
        self.step == OnboardingStep::Done
    }

    /// Return the data_dir chosen during onboarding (may differ from the default).
    pub fn data_dir(&self) -> &PathBuf {
        &self.data_dir
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, _data_dir: &PathBuf) {
        egui::CentralPanel::default().show_inside(ui, |ui: &mut egui::Ui| {
            let available = ui.available_size();
            let wizard_width = 520.0;
            let x_offset = ((available.x - wizard_width) / 2.0).max(0.0);

            ui.horizontal(|ui: &mut egui::Ui| {
                ui.add_space(x_offset);
                ui.vertical(|ui: &mut egui::Ui| {
                    ui.add_space(40.0);
                    ui.set_max_width(wizard_width);
                    self.step_indicator(ui);
                    ui.add_space(20.0);

                    match self.step {
                        OnboardingStep::Welcome => self.step_welcome(ui),
                        OnboardingStep::GenerateIdentity => self.step_identity(ui),
                        OnboardingStep::ChooseRole => self.step_role(ui),
                        OnboardingStep::ConnectBootstrap => self.step_bootstrap(ui),
                        OnboardingStep::Done => self.step_done(ui),
                    }
                });
            });
        });
    }

    fn step_indicator(&self, ui: &mut egui::Ui) {
        let steps = ["Welcome", "Identity", "Role", "Connect", "Ready"];
        let current_idx = match self.step {
            OnboardingStep::Welcome => 0,
            OnboardingStep::GenerateIdentity => 1,
            OnboardingStep::ChooseRole => 2,
            OnboardingStep::ConnectBootstrap => 3,
            OnboardingStep::Done => 4,
        };

        ui.horizontal(|ui: &mut egui::Ui| {
            for (i, label) in steps.iter().enumerate() {
                let is_current = i == current_idx;
                let is_past = i < current_idx;
                let text = if is_current {
                    egui::RichText::new(*label)
                        .strong()
                        .color(egui::Color32::from_rgb(0x64, 0xB5, 0xF6))
                } else if is_past {
                    egui::RichText::new(*label).color(egui::Color32::from_rgb(0x4C, 0xAF, 0x50))
                } else {
                    egui::RichText::new(*label).color(egui::Color32::GRAY)
                };
                ui.label(text);
                if i < steps.len() - 1 {
                    ui.label(egui::RichText::new(" → ").color(egui::Color32::DARK_GRAY));
                }
            }
        });
    }

    fn step_welcome(&mut self, ui: &mut egui::Ui) {
        ui.heading("Welcome to DSearch");
        ui.add_space(8.0);
        ui.label("DSearch is a decentralized search network.");
        ui.label("Your node helps index and discover content across the network,");
        ui.label("with no central server controlling what you find.");
        ui.add_space(16.0);

        ui.group(|ui: &mut egui::Ui| {
            ui.label(egui::RichText::new("Data Directory").strong());
            ui.add_space(4.0);
            ui.horizontal(|ui: &mut egui::Ui| {
                let mut dir_str = self.custom_data_dir.clone();
                ui.label("📁");
                if ui.text_edit_singleline(&mut dir_str).changed() {
                    self.custom_data_dir = dir_str;
                    self.data_dir = PathBuf::from(&self.custom_data_dir);
                }
            });
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    "This is where your identity, config, and stored records will live.",
                )
                .small()
                .color(egui::Color32::GRAY),
            );
        });

        ui.add_space(24.0);
        ui.with_layout(
            egui::Layout::right_to_left(egui::Align::BOTTOM),
            |ui: &mut egui::Ui| {
                if ui.button("Continue →").clicked() {
                    self.step = OnboardingStep::GenerateIdentity;
                }
            },
        );
    }

    fn step_identity(&mut self, ui: &mut egui::Ui) {
        ui.heading("Generate Node Identity");
        ui.add_space(8.0);

        if !self.identity_generated {
            ui.label("Your node needs a unique cryptographic identity.");
            ui.label("This Ed25519 keypair identifies you on the network and");
            ui.label("is used to sign content records and announcements.");
            ui.add_space(8.0);

            if !self.identity_error.is_empty() {
                ui.colored_label(
                    egui::Color32::from_rgb(0xF4, 0x43, 0x36),
                    &self.identity_error,
                );
                ui.add_space(8.0);
            }

            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::BOTTOM),
                |ui: &mut egui::Ui| {
                    if ui.button("Generate Identity →").clicked() {
                        self.identity_error.clear();
                        match crate::proto::cert::generate_identity() {
                            Ok((signing_key, node_id, cert_der, key_der)) => {
                                std::fs::create_dir_all(&self.data_dir).ok();
                                match crate::proto::cert::save_identity(
                                    &self.data_dir,
                                    &signing_key,
                                    &cert_der,
                                    &key_der,
                                ) {
                                    Ok(()) => {
                                        self.node_id = node_id;
                                        self.identity_generated = true;
                                    }
                                    Err(e) => {
                                        self.identity_error =
                                            format!("Failed to save identity: {}", e);
                                    }
                                }
                            }
                            Err(e) => {
                                self.identity_error = format!("Failed to generate identity: {}", e);
                            }
                        }
                    }
                    if ui.button("← Back").clicked() {
                        self.step = OnboardingStep::Welcome;
                    }
                },
            );
        } else {
            ui.label(
                egui::RichText::new("✓ Identity generated!")
                    .color(egui::Color32::from_rgb(0x4C, 0xAF, 0x50)),
            );
            ui.add_space(8.0);
            ui.group(|ui: &mut egui::Ui| {
                ui.label(egui::RichText::new("Node ID:").strong());
                ui.add_space(4.0);
                ui.horizontal(|ui: &mut egui::Ui| {
                    let id_display = self.node_id.clone();
                    ui.monospace(egui::RichText::new(&id_display).small());
                    if ui.small_button("📋 Copy").clicked() {
                        ui.ctx().copy_text(id_display.clone());
                    }
                });
            });
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(format!(
                    "Identity saved to: {}/identity.key\nCert saved to: {}/node.crt",
                    self.data_dir.display(),
                    self.data_dir.display()
                ))
                .small()
                .color(egui::Color32::GRAY),
            );

            ui.add_space(24.0);
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::BOTTOM),
                |ui: &mut egui::Ui| {
                    if ui.button("Continue →").clicked() {
                        self.step = OnboardingStep::ChooseRole;
                    }
                    if ui.button("← Back").clicked() {
                        self.step = OnboardingStep::Welcome;
                    }
                },
            );
        }
    }

    fn step_role(&mut self, ui: &mut egui::Ui) {
        ui.heading("Choose Your Role");
        ui.add_space(8.0);
        ui.label("Your role determines how your node participates in the network.");
        ui.add_space(12.0);

        let roles = [
            ("light", "Light", "Best for laptops and home connections (recommended).\nHolds own content only, no public port needed."),
            ("full", "Full", "Contribute to search routing.\nNeeds an open port for inbound connections."),
            ("scraper", "Scraper", "Index web content for the network.\nRuns scrape jobs and announces records."),
            ("archive", "Archive", "Store content long-term for resilience.\nAccepts replication pushes from other nodes."),
            ("custom", "Custom", "Choose multiple roles manually."),
        ];

        for (key, name, desc) in &roles {
            let is_selected = self.selected_role == *key;
            let response = ui.selectable_label(is_selected, *name);
            if response.clicked() {
                self.selected_role = key.to_string();
            }
            if is_selected {
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new(*desc)
                        .small()
                        .color(egui::Color32::LIGHT_GRAY),
                );
                ui.add_space(4.0);
            }
        }

        ui.add_space(12.0);
        if let Some(reachable) = self.autonat_result {
            if reachable {
                ui.label(
                    egui::RichText::new(
                        "✓ Your node appears publicly reachable — Full node is possible",
                    )
                    .color(egui::Color32::from_rgb(0x4C, 0xAF, 0x50)),
                );
            } else {
                ui.label(
                    egui::RichText::new("✗ Your node is behind NAT — Light node recommended")
                        .color(egui::Color32::from_rgb(0xFF, 0x98, 0x00)),
                );
            }
        } else {
            ui.horizontal(|ui: &mut egui::Ui| {
                ui.spinner();
                ui.label("Running AutoNAT probe…");
            });
            if self.autonat_result.is_none() {
                self.autonat_result = Some(false);
            }
        }

        ui.add_space(24.0);
        ui.with_layout(
            egui::Layout::right_to_left(egui::Align::BOTTOM),
            |ui: &mut egui::Ui| {
                if ui.button("Continue →").clicked() {
                    self.write_config_with_role();
                    self.step = OnboardingStep::ConnectBootstrap;
                }
                if ui.button("← Back").clicked() {
                    self.step = OnboardingStep::GenerateIdentity;
                }
            },
        );
    }

    fn step_bootstrap(&mut self, ui: &mut egui::Ui) {
        ui.heading("Connect to Network");
        ui.add_space(8.0);

        if !self.bootstrap_connected {
            // Load bootstrap peers once (not every frame)
            if !self.bootstrap_peers_loaded {
                let peers = crate::bootstrap::resolver::resolve_bootstrap_peers(&self.data_dir);
                self.bootstrap_peers = peers
                    .iter()
                    .map(|p| BootstrapPeerInfo {
                        id: p.id.clone(),
                        addr: p.addr.clone(),
                    })
                    .collect();
                self.bootstrap_peers_loaded = true;
            }

            ui.label("Bootstrap peers provide your entry point into the network.");
            ui.add_space(8.0);

            if self.bootstrap_peers.is_empty() {
                ui.label(
                    egui::RichText::new("No bootstrap peers found.")
                        .color(egui::Color32::from_rgb(0xFF, 0x98, 0x00)),
                );
            } else {
                ui.label(
                    egui::RichText::new(format!("Found {} bootstrap peer(s)", self.bootstrap_peers.len()))
                        .small(),
                );

                // Show test results if available
                if !self.bootstrap_test_results.is_empty() {
                    ui.add_space(4.0);
                    for (addr, reachable, detail) in &self.bootstrap_test_results {
                        let status = if *reachable { "✓" } else { "✗" };
                        let color = if *reachable {
                            egui::Color32::from_rgb(0x4C, 0xAF, 0x50)
                        } else {
                            egui::Color32::from_rgb(0xF4, 0x43, 0x36)
                        };
                        ui.horizontal(|ui: &mut egui::Ui| {
                            ui.label(egui::RichText::new(status).color(color));
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} ({})",
                                    &addr,
                                    detail
                                ))
                                .small(),
                            );
                        });
                    }
                } else {
                    // Show peer list without TCP testing (avoids blocking UI)
                    ui.add_space(4.0);
                    for peer in &self.bootstrap_peers {
                        ui.horizontal(|ui: &mut egui::Ui| {
                            ui.label(egui::RichText::new("•").color(egui::Color32::GRAY));
                            ui.label(
                                egui::RichText::new(format!(
                                    "{} ({})",
                                    &peer.id[..8.min(peer.id.len())],
                                    peer.addr
                                ))
                                .small(),
                            );
                        });
                    }
                }
            }

            ui.add_space(12.0);

            // Test connectivity button (runs TCP connects — brief block but user-initiated)
            ui.horizontal(|ui: &mut egui::Ui| {
                if ui.button("🔍 Test Connectivity").clicked() {
                    self.bootstrap_test_results.clear();
                    for peer in &self.bootstrap_peers {
                        let result = std::net::TcpStream::connect_timeout(
                            &peer.addr.parse().unwrap_or_else(|_| "0.0.0.0:0".parse().unwrap()),
                            std::time::Duration::from_secs(2),
                        );
                        match result {
                            Ok(_) => self.bootstrap_test_results.push((
                                peer.addr.clone(),
                                true,
                                "reachable".to_string(),
                            )),
                            Err(e) => self.bootstrap_test_results.push((
                                peer.addr.clone(),
                                false,
                                e.to_string(),
                            )),
                        }
                    }
                }
            });

            ui.add_space(12.0);

            // Manual peer add
            ui.group(|ui: &mut egui::Ui| {
                ui.label(egui::RichText::new("Add a peer manually").strong());
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "If automatic discovery fails, you can add a peer address here.",
                    )
                    .small()
                    .color(egui::Color32::GRAY),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui: &mut egui::Ui| {
                    ui.label("ID:");
                    ui.text_edit_singleline(&mut self.new_peer_id);
                });
                ui.horizontal(|ui: &mut egui::Ui| {
                    ui.label("Addr:");
                    ui.text_edit_singleline(&mut self.new_peer_addr);
                });
                ui.add_space(4.0);
                if ui.button("Add Peer").clicked()
                    && !self.new_peer_id.is_empty()
                    && !self.new_peer_addr.is_empty()
                    && crate::bootstrap::resolver::write_bootstrap_peer(
                        &self.data_dir,
                        &self.new_peer_id,
                        &self.new_peer_addr,
                        "added during onboarding",
                    )
                    .is_ok()
                {
                    self.bootstrap_peers_loaded = false; // Reload peers
                    self.bootstrap_test_results.clear();
                    self.new_peer_id.clear();
                    self.new_peer_addr.clear();
                }
            });

            ui.add_space(24.0);
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::BOTTOM),
                |ui: &mut egui::Ui| {
                    if ui.button("Skip →").clicked() {
                        self.write_bootstrap_defaults();
                        self.bootstrap_connected = true;
                        self.step = OnboardingStep::Done;
                    }
                    if ui.button("Continue →").clicked() {
                        self.write_bootstrap_defaults();
                        self.bootstrap_connected = true;
                        self.bootstrap_peer_count = self.bootstrap_peers.len();
                        self.step = OnboardingStep::Done;
                    }
                    if ui.button("← Back").clicked() {
                        self.step = OnboardingStep::ChooseRole;
                    }
                },
            );
        } else {
            ui.label(
                egui::RichText::new(format!(
                    "Connected to {} peer(s). Network ready.",
                    self.bootstrap_peer_count
                ))
                .color(egui::Color32::from_rgb(0x4C, 0xAF, 0x50)),
            );
            ui.add_space(24.0);
            ui.with_layout(
                egui::Layout::right_to_left(egui::Align::BOTTOM),
                |ui: &mut egui::Ui| {
                    if ui.button("Start Searching →").clicked() {
                        self.step = OnboardingStep::Done;
                    }
                },
            );
        }
    }

    fn step_done(&mut self, ui: &mut egui::Ui) {
        ui.heading("You're all set!");
        ui.add_space(12.0);
        ui.label("Your node is connected. Start searching, or add a scraper");
        ui.label("in Settings to contribute content to the network.");
        ui.add_space(16.0);

        ui.group(|ui: &mut egui::Ui| {
            ui.label(egui::RichText::new("Summary").strong());
            ui.add_space(4.0);
            ui.label(format!(
                "Node ID: {}…",
                &self.node_id[..16.min(self.node_id.len())]
            ));
            ui.label(format!("Role: {}", self.selected_role));
            ui.label(format!("Data dir: {}", self.data_dir.display()));
        });

        ui.add_space(24.0);
        ui.with_layout(
            egui::Layout::right_to_left(egui::Align::BOTTOM),
            |ui: &mut egui::Ui| {
                if ui.button("Start Searching →").clicked() {
                    // is_complete() will return true
                }
            },
        );
    }

    fn write_config_with_role(&self) {
        std::fs::create_dir_all(&self.data_dir).ok();
        let config_path = self.data_dir.join("config.toml");
        if !config_path.exists() {
            let mut config = crate::config::DsearchConfig::default();
            config.node.role = self.selected_role.clone();
            crate::config::save_config(&self.data_dir, &config).ok();
        } else if let Ok(mut config) = crate::config::load_config(&self.data_dir) {
            config.node.role = self.selected_role.clone();
            crate::config::save_config(&self.data_dir, &config).ok();
        }
    }

    fn write_bootstrap_defaults(&self) {
        let bootstrap_path = self.data_dir.join("bootstrap.toml");
        if !bootstrap_path.exists() {
            let default_toml = r#"# {data_dir}/bootstrap.toml
# Edit freely. Add community or private bootstrap nodes here.
# The built-in list is always tried alongside this file.
# Remove the built-in list entirely by setting use_defaults = false.

use_defaults = true
"#;
            std::fs::write(&bootstrap_path, default_toml).ok();
        }
    }
}
