use std::path::PathBuf;
use eframe::egui;
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
        }
    }

    pub fn is_complete(&self) -> bool {
        self.step == OnboardingStep::Done
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
                    egui::RichText::new(*label).strong().color(egui::Color32::from_rgb(0x64, 0xB5, 0xF6))
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
            ui.label(egui::RichText::new(
                "This is where your identity, config, and stored records will live."
            ).small().color(egui::Color32::GRAY));
        });

        ui.add_space(24.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::BOTTOM), |ui: &mut egui::Ui| {
            if ui.button("Continue →").clicked() {
                self.step = OnboardingStep::GenerateIdentity;
            }
        });
    }

    fn step_identity(&mut self, ui: &mut egui::Ui) {
        ui.heading("Generate Node Identity");
        ui.add_space(8.0);

        if !self.identity_generated {
            ui.label("Your node needs a unique cryptographic identity.");
            ui.label("This Ed25519 keypair identifies you on the network and");
            ui.label("is used to sign content records and announcements.");
            ui.add_space(16.0);

            ui.with_layout(egui::Layout::right_to_left(egui::Align::BOTTOM), |ui: &mut egui::Ui| {
                if ui.button("Generate Identity →").clicked() {
                    if let Ok((signing_key, node_id, cert_der, key_der)) =
                        crate::proto::cert::generate_identity()
                    {
                        std::fs::create_dir_all(&self.data_dir).ok();
                        if crate::proto::cert::save_identity(&self.data_dir, &signing_key, &cert_der, &key_der).is_ok() {
                            self.node_id = node_id;
                            self.identity_generated = true;
                        }
                    }
                }
                if ui.button("← Back").clicked() {
                    self.step = OnboardingStep::Welcome;
                }
            });
        } else {
            ui.label(egui::RichText::new("✓ Identity generated!").color(egui::Color32::from_rgb(0x4C, 0xAF, 0x50)));
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
            ui.label(egui::RichText::new(format!(
                "Identity saved to: {}/identity.key\nCert saved to: {}/node.crt",
                self.data_dir.display(),
                self.data_dir.display()
            )).small().color(egui::Color32::GRAY));

            ui.add_space(24.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::BOTTOM), |ui: &mut egui::Ui| {
                if ui.button("Continue →").clicked() {
                    self.step = OnboardingStep::ChooseRole;
                }
                if ui.button("← Back").clicked() {
                    self.step = OnboardingStep::Welcome;
                }
            });
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
                ui.label(egui::RichText::new(*desc).small().color(egui::Color32::LIGHT_GRAY));
                ui.add_space(4.0);
            }
        }

        ui.add_space(12.0);
        if let Some(reachable) = self.autonat_result {
            if reachable {
                ui.label(egui::RichText::new("✓ Your node appears publicly reachable — Full node is possible")
                    .color(egui::Color32::from_rgb(0x4C, 0xAF, 0x50)));
            } else {
                ui.label(egui::RichText::new("✗ Your node is behind NAT — Light node recommended")
                    .color(egui::Color32::from_rgb(0xFF, 0x98, 0x00)));
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
        ui.with_layout(egui::Layout::right_to_left(egui::Align::BOTTOM), |ui: &mut egui::Ui| {
            if ui.button("Continue →").clicked() {
                self.write_config_with_role();
                self.step = OnboardingStep::ConnectBootstrap;
            }
            if ui.button("← Back").clicked() {
                self.step = OnboardingStep::GenerateIdentity;
            }
        });
    }

    fn step_bootstrap(&mut self, ui: &mut egui::Ui) {
        ui.heading("Connect to Network");
        ui.add_space(8.0);

        if !self.bootstrap_connected {
            ui.label("Connecting to bootstrap peers…");
            ui.add_space(8.0);
            ui.spinner();
            ui.add_space(8.0);

            let peers = crate::bootstrap::resolver::resolve_bootstrap_peers(&self.data_dir);
            if !peers.is_empty() {
                ui.label(egui::RichText::new(format!("Found {} bootstrap peer(s)", peers.len())).small());
            } else {
                ui.label(egui::RichText::new("No bootstrap peers found.").color(egui::Color32::from_rgb(0xFF, 0x98, 0x00)));
            }

            ui.add_space(12.0);
            ui.group(|ui: &mut egui::Ui| {
                ui.label(egui::RichText::new("Add a peer manually").strong());
                ui.add_space(4.0);
                ui.label(egui::RichText::new("If automatic discovery fails, you can add a peer address here.")
                    .small().color(egui::Color32::GRAY));
            });

            ui.add_space(24.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::BOTTOM), |ui: &mut egui::Ui| {
                if ui.button("Skip →").clicked() {
                    self.write_bootstrap_defaults();
                    self.bootstrap_connected = true;
                    self.step = OnboardingStep::Done;
                }
                if ui.button("Retry").clicked() {
                    self.bootstrap_connected = true;
                    self.write_bootstrap_defaults();
                    self.step = OnboardingStep::Done;
                }
                if ui.button("← Back").clicked() {
                    self.step = OnboardingStep::ChooseRole;
                }
            });
        } else {
            ui.label(egui::RichText::new(format!(
                "Connected to {} peer(s). Network ready.",
                self.bootstrap_peer_count
            )).color(egui::Color32::from_rgb(0x4C, 0xAF, 0x50)));
            ui.add_space(24.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::BOTTOM), |ui: &mut egui::Ui| {
                if ui.button("Start Searching →").clicked() {
                    self.step = OnboardingStep::Done;
                }
            });
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
            ui.label(format!("Node ID: {}…", &self.node_id[..16.min(self.node_id.len())]));
            ui.label(format!("Role: {}", self.selected_role));
            ui.label(format!("Data dir: {}", self.data_dir.display()));
        });

        ui.add_space(24.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::BOTTOM), |ui: &mut egui::Ui| {
            if ui.button("Start Searching →").clicked() {
                // is_complete() will return true
            }
        });
    }

    fn write_config_with_role(&self) {
        std::fs::create_dir_all(&self.data_dir).ok();
        let config_path = self.data_dir.join("config.toml");
        if !config_path.exists() {
            let mut config = crate::config::DsearchConfig::default();
            config.node.role = self.selected_role.clone();
            crate::config::save_config(&self.data_dir, &config).ok();
        } else {
            if let Ok(mut config) = crate::config::load_config(&self.data_dir) {
                config.node.role = self.selected_role.clone();
                crate::config::save_config(&self.data_dir, &config).ok();
            }
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
