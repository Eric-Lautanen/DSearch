use super::ApiHelper;
use eframe::egui;

#[derive(Default)]
pub struct BootstrapPanel {
    new_peer_id: String,
    new_peer_addr: String,
    new_peer_note: String,
    test_results: Vec<(String, bool, String)>,
}

impl BootstrapPanel {
    pub fn ui(&mut self, ui: &mut egui::Ui, api: &ApiHelper) {
        ui.heading("Bootstrap Peers");
        ui.add_space(8.0);

        if let Some(body) = api.api_get("/bootstrap") {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(peers) = v.get("peers").and_then(|p| p.as_array()) {
                    if peers.is_empty() {
                        ui.label(
                            egui::RichText::new("No bootstrap peers configured.")
                                .color(egui::Color32::GRAY),
                        );
                    } else {
                        ui.group(|ui: &mut egui::Ui| {
                            ui.label(
                                egui::RichText::new(format!("Configured Peers ({})", peers.len()))
                                    .strong(),
                            );
                            ui.add_space(4.0);

                            egui::Grid::new("bootstrap_peers_grid").striped(true).show(
                                ui,
                                |ui: &mut egui::Ui| {
                                    ui.label(egui::RichText::new("ID").strong());
                                    ui.label(egui::RichText::new("Address").strong());
                                    ui.label(egui::RichText::new("Source").strong());
                                    ui.label(egui::RichText::new("Note").strong());
                                    ui.end_row();

                                    for (i, p) in peers.iter().enumerate() {
                                        let id =
                                            p.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                                        let addr =
                                            p.get("addr").and_then(|v| v.as_str()).unwrap_or("?");
                                        let note =
                                            p.get("note").and_then(|v| v.as_str()).unwrap_or("");
                                        let source = if i < 3 { "built-in" } else { "user" };

                                        ui.label(
                                            egui::RichText::new(format!(
                                                "{}…",
                                                &id[..8.min(id.len())]
                                            ))
                                            .small(),
                                        );
                                        ui.label(egui::RichText::new(addr).small());
                                        ui.label(egui::RichText::new(source).small().color(
                                            match source {
                                                "built-in" => {
                                                    egui::Color32::from_rgb(0x4C, 0xAF, 0x50)
                                                }
                                                "dns" => egui::Color32::from_rgb(0x64, 0xB5, 0xF6),
                                                _ => egui::Color32::from_rgb(0xFF, 0x98, 0x00),
                                            },
                                        ));
                                        ui.label(
                                            egui::RichText::new(note)
                                                .small()
                                                .color(egui::Color32::GRAY),
                                        );
                                        ui.end_row();
                                    }
                                },
                            );
                        });
                    }
                }
            }
        }

        ui.add_space(8.0);

        ui.horizontal(|ui: &mut egui::Ui| {
            if ui.button("🔍 Test All").clicked() {
                self.test_results.clear();
                let peers = crate::bootstrap::resolver::resolve_bootstrap_peers(&api.data_dir);
                for peer in &peers {
                    let addr = peer.addr.clone();
                    let result = std::net::TcpStream::connect_timeout(
                        &addr
                            .parse()
                            .unwrap_or_else(|_| "0.0.0.0:0".parse().unwrap()),
                        std::time::Duration::from_secs(3),
                    );
                    match result {
                        Ok(_) => self
                            .test_results
                            .push((addr, true, "reachable".to_string())),
                        Err(e) => self.test_results.push((addr, false, e.to_string())),
                    }
                }
            }
        });

        if !self.test_results.is_empty() {
            ui.add_space(4.0);
            ui.group(|ui: &mut egui::Ui| {
                ui.label(egui::RichText::new("Test Results").strong());
                ui.add_space(4.0);
                for (addr, reachable, detail) in &self.test_results {
                    ui.horizontal(|ui: &mut egui::Ui| {
                        if *reachable {
                            ui.label(
                                egui::RichText::new("✓")
                                    .color(egui::Color32::from_rgb(0x4C, 0xAF, 0x50)),
                            );
                        } else {
                            ui.label(
                                egui::RichText::new("✗")
                                    .color(egui::Color32::from_rgb(0xF4, 0x43, 0x36)),
                            );
                        }
                        ui.label(addr);
                        ui.label(
                            egui::RichText::new(detail)
                                .small()
                                .color(egui::Color32::GRAY),
                        );
                    });
                }
            });
        }

        ui.add_space(8.0);

        ui.group(|ui: &mut egui::Ui| {
            ui.label(egui::RichText::new("Add Peer Manually").strong());
            ui.add_space(4.0);
            ui.horizontal(|ui: &mut egui::Ui| {
                ui.label("ID:");
                ui.text_edit_singleline(&mut self.new_peer_id);
            });
            ui.horizontal(|ui: &mut egui::Ui| {
                ui.label("Addr:");
                ui.text_edit_singleline(&mut self.new_peer_addr);
            });
            ui.horizontal(|ui: &mut egui::Ui| {
                ui.label("Note:");
                ui.text_edit_singleline(&mut self.new_peer_note);
            });
            ui.add_space(4.0);
            if ui.button("Add Peer").clicked()
                && !self.new_peer_id.is_empty()
                && !self.new_peer_addr.is_empty()
            {
                match crate::bootstrap::resolver::write_bootstrap_peer(
                    &api.data_dir,
                    &self.new_peer_id,
                    &self.new_peer_addr,
                    &self.new_peer_note,
                ) {
                    Ok(()) => {
                        self.new_peer_id.clear();
                        self.new_peer_addr.clear();
                        self.new_peer_note.clear();
                    }
                    Err(e) => {
                        tracing::warn!("Failed to add bootstrap peer: {}", e);
                    }
                }
            }
        });

        ui.add_space(8.0);

        ui.group(|ui: &mut egui::Ui| {
            ui.label(egui::RichText::new("Default Peers").strong());
            ui.add_space(4.0);
            if let Ok(config) = crate::config::load_config(&api.data_dir) {
                let use_defaults = config.bootstrap.use_defaults;
                ui.horizontal(|ui: &mut egui::Ui| {
                    ui.label(if use_defaults { "✓ Using default bootstrap peers" } else { "✗ Default peers disabled (private network mode)" });
                });
                if !use_defaults {
                    ui.label(egui::RichText::new("⚠ Private network mode — your node won't connect to the public DSearch network.")
                        .small().color(egui::Color32::from_rgb(0xFF, 0x98, 0x00)));
                }
            }
        });
    }
}
