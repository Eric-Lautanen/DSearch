use super::bootstrap_panel;
use super::ApiHelper;
use eframe::egui;

#[derive(Default, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Identity,
    Gateway,
    Scrapers,
    #[default]
    Bootstrap,
    SearchProviders,
}

#[derive(Default)]
pub struct SettingsPanel {
    pub tab: SettingsTab,
    new_key_nickname: String,
    new_key_secret: Option<String>,
    new_job_name: String,
    new_job_target: String,
    new_job_source: String,
    new_job_refresh: String,
    new_job_lifecycle: String,
    pub bootstrap: bootstrap_panel::BootstrapPanel,
    new_provider_name: String,
    new_provider_target: String,
    add_provider_error: String,
    add_job_error: String,
    api_error: String,
}

impl SettingsPanel {
    pub fn ui(&mut self, ui: &mut egui::Ui, api: &ApiHelper) {
        ui.horizontal(|ui: &mut egui::Ui| {
            ui.selectable_value(&mut self.tab, SettingsTab::General, "General");
            ui.selectable_value(&mut self.tab, SettingsTab::Identity, "Identity");
            ui.selectable_value(&mut self.tab, SettingsTab::Gateway, "Gateway");
            ui.selectable_value(&mut self.tab, SettingsTab::Scrapers, "Scrapers");
            ui.selectable_value(&mut self.tab, SettingsTab::Bootstrap, "Bootstrap");
            ui.selectable_value(
                &mut self.tab,
                SettingsTab::SearchProviders,
                "Search Providers",
            );
        });
        ui.separator();
        ui.add_space(8.0);

        // Show API error banner if present
        if !self.api_error.is_empty() {
            ui.colored_label(
                egui::Color32::from_rgb(0xF4, 0x43, 0x36),
                egui::RichText::new(format!("⚠ {}", &self.api_error)).small(),
            );
            ui.add_space(4.0);
        }

        egui::ScrollArea::vertical().show(ui, |ui: &mut egui::Ui| match self.tab {
            SettingsTab::General => self.tab_general(ui, api),
            SettingsTab::Identity => self.tab_identity(ui, api),
            SettingsTab::Gateway => self.tab_gateway(ui, api),
            SettingsTab::Scrapers => self.tab_scrapers(ui, api),
            SettingsTab::Bootstrap => self.bootstrap.ui(ui, api),
            SettingsTab::SearchProviders => self.tab_search_providers(ui, api),
        });
    }

    fn tab_general(&mut self, ui: &mut egui::Ui, api: &ApiHelper) {
        ui.heading("General Settings");
        ui.add_space(8.0);

        ui.group(|ui: &mut egui::Ui| {
            ui.label(egui::RichText::new("Data Directory").strong());
            ui.add_space(4.0);
            let data_dir_str = api.data_dir.to_string_lossy().to_string();
            ui.horizontal(|ui: &mut egui::Ui| {
                ui.label("📁");
                ui.label(&data_dir_str);
                if ui.small_button("Open").clicked() {
                    #[cfg(windows)]
                    std::process::Command::new("explorer")
                        .arg(&api.data_dir)
                        .spawn()
                        .ok();
                    #[cfg(unix)]
                    std::process::Command::new("xdg-open")
                        .arg(&api.data_dir)
                        .spawn()
                        .ok();
                }
            });
        });

        ui.add_space(8.0);

        match api.api_get_result("/config") {
            Ok(body) => {
                self.api_error.clear();
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                    ui.group(|ui: &mut egui::Ui| {
                        ui.label(egui::RichText::new("Node Configuration").strong());
                        ui.add_space(4.0);

                        let config_keys = [
                            ("node.role", "Role"),
                            ("node.max_connections", "Max Connections"),
                            ("node.min_protocol_version", "Min Protocol Version"),
                            ("node.ipv4", "IPv4"),
                            ("node.ipv6", "IPv6"),
                            ("api.port", "API Port"),
                            ("storage.quota_mb", "Storage Quota (MB)"),
                            ("storage.quota_action", "Quota Action"),
                            ("storage.tier2_max_mb", "Tier 2 Max (MB)"),
                            ("log.level", "Log Level"),
                            ("log.output", "Log Output"),
                            ("bootstrap.use_defaults", "Use Default Peers"),
                        ];

                        for (key, label) in &config_keys {
                            let parts: Vec<&str> = key.split('.').collect();
                            if parts.len() == 2 {
                                if let Some(section) = v.get(parts[0]) {
                                    if let Some(val) = section.get(parts[1]) {
                                        ui.horizontal(|ui: &mut egui::Ui| {
                                            ui.label(egui::RichText::new(*label).strong());
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui: &mut egui::Ui| {
                                                    ui.label(
                                                        egui::RichText::new(val.to_string()).color(
                                                            egui::Color32::from_rgb(0x64, 0xB5, 0xF6),
                                                        ),
                                                    );
                                                },
                                            );
                                        });
                                    }
                                }
                            }
                        }
                    });
                }
            }
            Err(e) => {
                self.api_error = e;
            }
        }
    }

    fn tab_identity(&mut self, ui: &mut egui::Ui, api: &ApiHelper) {
        ui.heading("Identity");
        ui.add_space(8.0);

        match api.api_get_result("/identity") {
            Ok(body) => {
                self.api_error.clear();
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                    ui.group(|ui: &mut egui::Ui| {
                        let node_id = v
                            .get("node_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        ui.label(egui::RichText::new("Node ID").strong());
                        ui.add_space(4.0);
                        ui.horizontal(|ui: &mut egui::Ui| {
                            ui.monospace(egui::RichText::new(node_id).small());
                            if ui.small_button("📋 Copy").clicked() {
                                ui.ctx().copy_text(node_id.to_string());
                            }
                        });
                        ui.add_space(4.0);
                        let has_identity = v
                            .get("has_identity")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        if has_identity {
                            ui.label(
                                egui::RichText::new("✓ Identity key found")
                                    .color(egui::Color32::from_rgb(0x4C, 0xAF, 0x50)),
                            );
                        } else {
                            ui.label(
                                egui::RichText::new("✗ No identity key found")
                                    .color(egui::Color32::from_rgb(0xF4, 0x43, 0x36)),
                            );
                        }
                    });
                }
            }
            Err(e) => {
                self.api_error = e;
            }
        }

        ui.add_space(8.0);
        ui.group(|ui: &mut egui::Ui| {
            ui.label(egui::RichText::new("Identity Files").strong());
            ui.add_space(4.0);
            let key_path = api.data_dir.join("identity.key");
            let cert_path = api.data_dir.join("node.crt");
            ui.label(format!("🔑 {}", key_path.display()));
            ui.label(format!("📜 {}", cert_path.display()));
        });
    }

    fn tab_gateway(&mut self, ui: &mut egui::Ui, api: &ApiHelper) {
        ui.heading("Gateway API");
        ui.add_space(8.0);

        if let Ok(body) = api.api_get_result("/config") {
            self.api_error.clear();
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(gw) = v.get("gateway") {
                    ui.group(|ui: &mut egui::Ui| {
                        ui.label(egui::RichText::new("Gateway Configuration").strong());
                        ui.add_space(4.0);
                        let enabled = gw.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
                        let bind = gw
                            .get("bind")
                            .and_then(|v| v.as_str())
                            .unwrap_or("0.0.0.0:7744");
                        let rate_limit = gw
                            .get("rate_limit_per_min")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(60);
                        let require_key = gw
                            .get("require_api_key")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);

                        ui.horizontal(|ui: &mut egui::Ui| {
                            ui.label("Enabled:");
                            ui.label(
                                egui::RichText::new(if enabled { "Yes" } else { "No" }).color(
                                    if enabled {
                                        egui::Color32::from_rgb(0x4C, 0xAF, 0x50)
                                    } else {
                                        egui::Color32::GRAY
                                    },
                                ),
                            );
                        });
                        ui.horizontal(|ui: &mut egui::Ui| {
                            ui.label("Bind:");
                            ui.label(bind);
                        });
                        ui.horizontal(|ui: &mut egui::Ui| {
                            ui.label("Rate Limit:");
                            ui.label(format!("{}/min", rate_limit));
                        });
                        ui.horizontal(|ui: &mut egui::Ui| {
                            ui.label("Require API Key:");
                            ui.label(if require_key { "Yes" } else { "No" });
                        });
                    });
                }
            }
        } else if let Err(e) = api.api_get_result("/config") {
            self.api_error = e;
        }

        ui.add_space(8.0);

        ui.group(|ui: &mut egui::Ui| {
            ui.label(egui::RichText::new("API Keys").strong());
            ui.add_space(4.0);

            ui.horizontal(|ui: &mut egui::Ui| {
                ui.label("Nickname:");
                ui.text_edit_singleline(&mut self.new_key_nickname);
                if ui.button("Create Key").clicked() && !self.new_key_nickname.is_empty() {
                    let key_store = crate::api::gateway_keys::GatewayKeyStore::new(api.data_dir.clone());
                    match key_store.create_key(&self.new_key_nickname) {
                        Ok((secret, info)) => {
                            self.new_key_secret = Some(format!(
                                "Nickname: {}\nSecret: {}\n\nSave this secret — it won't be shown again!",
                                info.nickname, secret
                            ));
                        }
                        Err(e) => {
                            self.new_key_secret = Some(format!("Error: {}", e));
                        }
                    }
                    self.new_key_nickname.clear();
                }
            });

            if let Some(ref secret) = self.new_key_secret {
                ui.add_space(4.0);
                ui.colored_label(egui::Color32::from_rgb(0xFF, 0x98, 0x00), secret);
            }

            ui.add_space(8.0);

            let key_store = crate::api::gateway_keys::GatewayKeyStore::new(api.data_dir.clone());
            if let Ok(keys) = key_store.list_keys() {
                if keys.is_empty() {
                    ui.label(egui::RichText::new("No API keys configured.").color(egui::Color32::GRAY));
                } else {
                    for k in &keys {
                        ui.horizontal(|ui: &mut egui::Ui| {
                            ui.label(egui::RichText::new(&k.nickname).strong());
                            ui.label(egui::RichText::new(format!("created: {}", k.created_at)).small().color(egui::Color32::GRAY));
                            ui.label(egui::RichText::new(format!("requests: {}", k.request_count)).small().color(egui::Color32::GRAY));
                            if ui.small_button("🗑 Revoke").clicked() {
                                key_store.revoke_key(&k.nickname).ok();
                            }
                        });
                    }
                }
            }
        });
    }

    fn tab_scrapers(&mut self, ui: &mut egui::Ui, api: &ApiHelper) {
        ui.heading("Scraper Jobs");
        ui.add_space(8.0);

        if let Some(body) = api.api_get("/scraper") {
            self.api_error.clear();
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(jobs) = v.get("jobs").and_then(|j| j.as_array()) {
                    if jobs.is_empty() {
                        ui.label(
                            egui::RichText::new("No scraper jobs configured.")
                                .color(egui::Color32::GRAY),
                        );
                    } else {
                        for j in jobs {
                            ui.group(|ui: &mut egui::Ui| {
                                let name = j.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                                let source =
                                    j.get("source").and_then(|v| v.as_str()).unwrap_or("?");
                                let target =
                                    j.get("target").and_then(|v| v.as_str()).unwrap_or("?");
                                let refresh =
                                    j.get("refresh").and_then(|v| v.as_str()).unwrap_or("?");
                                let lifecycle =
                                    j.get("lifecycle").and_then(|v| v.as_str()).unwrap_or("?");

                                ui.horizontal(|ui: &mut egui::Ui| {
                                    ui.label(egui::RichText::new(name).strong());
                                    ui.label(
                                        egui::RichText::new(format!(
                                            "{} • {} • {}",
                                            source, refresh, lifecycle
                                        ))
                                        .small()
                                        .color(egui::Color32::GRAY),
                                    );
                                });
                                ui.label(
                                    egui::RichText::new(target)
                                        .small()
                                        .color(egui::Color32::from_rgb(0x64, 0xB5, 0xF6)),
                                );
                            });
                            ui.add_space(4.0);
                        }
                    }
                }
            }
        } else {
            // API not reachable — try loading from config directly
            if let Ok(config) = crate::config::load_config(&api.data_dir) {
                if config.scraper.jobs.is_empty() {
                    ui.label(
                        egui::RichText::new("No scraper jobs configured.")
                            .color(egui::Color32::GRAY),
                    );
                } else {
                    for j in &config.scraper.jobs {
                        ui.group(|ui: &mut egui::Ui| {
                            ui.horizontal(|ui: &mut egui::Ui| {
                                ui.label(egui::RichText::new(&j.name).strong());
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} • {} • {}",
                                        j.source, j.refresh, j.lifecycle
                                    ))
                                    .small()
                                    .color(egui::Color32::GRAY),
                                );
                            });
                            ui.label(
                                egui::RichText::new(&j.target)
                                    .small()
                                    .color(egui::Color32::from_rgb(0x64, 0xB5, 0xF6)),
                            );
                        });
                        ui.add_space(4.0);
                    }
                }
            } else {
                self.api_error = "Cannot load scraper jobs: API unreachable and config file invalid.".to_string();
            }
        }

        ui.add_space(8.0);

        ui.group(|ui: &mut egui::Ui| {
            ui.label(egui::RichText::new("New Scraper Job").strong());
            ui.add_space(4.0);

            ui.horizontal(|ui: &mut egui::Ui| {
                ui.label("Name:");
                ui.text_edit_singleline(&mut self.new_job_name);
            });
            ui.horizontal(|ui: &mut egui::Ui| {
                ui.label("Target:");
                ui.text_edit_singleline(&mut self.new_job_target);
            });

            egui::ComboBox::from_label("Source").show_ui(ui, |ui: &mut egui::Ui| {
                ui.selectable_value(&mut self.new_job_source, "url".to_string(), "URL");
                ui.selectable_value(&mut self.new_job_source, "feed".to_string(), "Feed");
                ui.selectable_value(&mut self.new_job_source, "api".to_string(), "API");
                ui.selectable_value(&mut self.new_job_source, "keyword".to_string(), "Keyword");
            });

            egui::ComboBox::from_label("Refresh").show_ui(ui, |ui: &mut egui::Ui| {
                ui.selectable_value(&mut self.new_job_refresh, "once".to_string(), "Once");
                ui.selectable_value(
                    &mut self.new_job_refresh,
                    "interval".to_string(),
                    "Interval",
                );
                ui.selectable_value(
                    &mut self.new_job_refresh,
                    "on-change".to_string(),
                    "On Change",
                );
            });

            egui::ComboBox::from_label("Lifecycle").show_ui(ui, |ui: &mut egui::Ui| {
                ui.selectable_value(
                    &mut self.new_job_lifecycle,
                    "ephemeral".to_string(),
                    "Ephemeral",
                );
                ui.selectable_value(&mut self.new_job_lifecycle, "pinned".to_string(), "Pinned");
            });

            if !self.add_job_error.is_empty() {
                ui.colored_label(
                    egui::Color32::from_rgb(0xF4, 0x43, 0x36),
                    egui::RichText::new(&self.add_job_error).small(),
                );
            }

            ui.add_space(4.0);
            if ui.button("Add Job").clicked()
                && !self.new_job_name.is_empty()
                && !self.new_job_target.is_empty()
            {
                let source = if self.new_job_source.is_empty() {
                    "url"
                } else {
                    &self.new_job_source
                };
                let refresh = if self.new_job_refresh.is_empty() {
                    "once"
                } else {
                    &self.new_job_refresh
                };
                let lifecycle = if self.new_job_lifecycle.is_empty() {
                    "ephemeral"
                } else {
                    &self.new_job_lifecycle
                };

                match crate::config::load_config(&api.data_dir) {
                    Ok(mut config) => {
                        let job = crate::model::ScrapeJob {
                            name: self.new_job_name.clone(),
                            source: crate::model::ScrapeSource::from_str(source)
                                .unwrap_or(crate::model::ScrapeSource::Url),
                            target: self.new_job_target.clone(),
                            transform: None,
                            refresh: crate::model::RefreshPolicy::from_str(refresh)
                                .unwrap_or(crate::model::RefreshPolicy::Once),
                            interval_secs: 3600,
                            lifecycle: crate::model::Lifecycle::from_str(lifecycle)
                                .unwrap_or(crate::model::Lifecycle::Ephemeral),
                            ttl_secs: 3600,
                            max_results: None,
                        };
                        config.scraper.jobs.push(job);
                        match crate::config::save_config(&api.data_dir, &config) {
                            Ok(()) => {
                                self.add_job_error.clear();
                                self.new_job_name.clear();
                                self.new_job_target.clear();
                            }
                            Err(e) => {
                                self.add_job_error = format!("Failed to save config: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        self.add_job_error = format!("Failed to load config: {}", e);
                    }
                }
            }
        });
    }

    fn tab_search_providers(&mut self, ui: &mut egui::Ui, api: &ApiHelper) {
        ui.heading("Search Providers");
        ui.add_space(8.0);

        ui.label("Search providers resolve keyword queries into URLs for scraping.");
        ui.add_space(8.0);

        let providers_path = api.data_dir.join("search_providers.toml");
        if providers_path.exists() {
            if let Ok(contents) = std::fs::read_to_string(&providers_path) {
                ui.group(|ui: &mut egui::Ui| {
                    ui.label(egui::RichText::new("Current Providers").strong());
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .show(ui, |ui: &mut egui::Ui| {
                            ui.monospace(egui::RichText::new(&contents).small());
                        });
                });
            }
        } else {
            ui.label(
                egui::RichText::new(
                    "No search_providers.toml found. Default provider (DuckDuckGo) will be used.",
                )
                .color(egui::Color32::GRAY),
            );
        }

        ui.add_space(8.0);

        ui.group(|ui: &mut egui::Ui| {
            ui.label(egui::RichText::new("Add Provider").strong());
            ui.add_space(4.0);
            ui.horizontal(|ui: &mut egui::Ui| {
                ui.label("Name:");
                ui.text_edit_singleline(&mut self.new_provider_name);
            });
            ui.horizontal(|ui: &mut egui::Ui| {
                ui.label("Target:");
                ui.text_edit_singleline(&mut self.new_provider_target);
            });

            if !self.add_provider_error.is_empty() {
                ui.colored_label(
                    egui::Color32::from_rgb(0xF4, 0x43, 0x36),
                    egui::RichText::new(&self.add_provider_error).small(),
                );
            }

            if ui.button("Add").clicked() && !self.new_provider_name.is_empty() {
                match crate::scraper::discovery::providers::add_provider(
                    &api.data_dir,
                    &self.new_provider_name,
                    &self.new_provider_target,
                ) {
                    Ok(()) => {
                        self.add_provider_error.clear();
                        self.new_provider_name.clear();
                        self.new_provider_target.clear();
                    }
                    Err(e) => {
                        self.add_provider_error = e;
                    }
                }
            }
        });
    }
}
