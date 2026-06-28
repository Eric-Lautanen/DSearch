use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

mod bootstrap_panel;
mod onboarding;
mod search;
mod settings;
mod tray;

pub use onboarding::OnboardingState;
pub use tray::TrayState;

/// Which panel is currently active in the main UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Panel {
    Search,
    Settings,
}

/// The main application state for the egui UI.
pub struct DsearchApp {
    data_dir: PathBuf,
    node_id: Arc<Mutex<String>>,
    api_port: Arc<Mutex<Option<u16>>>,

    // Onboarding
    onboarding: Option<OnboardingState>,

    // Main UI state
    panel: Panel,
    search: search::SearchPanel,
    settings: settings::SettingsPanel,

    // Status bar
    status: StatusBar,

    // Tray
    tray: TrayState,
}

/// Status bar data — refreshed from the API each frame.
#[derive(Debug, Clone, Default)]
pub struct StatusBar {
    pub role: String,
    pub peers: usize,
    pub records: usize,
    pub tier2_size_mb: f64,
    pub bandwidth_mbps: f64,
}

/// Shared API helper — used by sub-panels to call the local API.
pub struct ApiHelper {
    pub data_dir: PathBuf,
}

impl ApiHelper {
    pub fn api_get(&self, path: &str) -> Option<String> {
        let port_path = self.data_dir.join("api.port");
        let contents = std::fs::read_to_string(port_path).ok()?;
        let port: u16 = contents.trim().parse().ok()?;
        crate::cli::api_client::api_get(port, path).ok()
    }
}

impl DsearchApp {
    pub fn new(data_dir: PathBuf) -> Self {
        let needs_onboarding = !data_dir.join("identity.key").exists();

        Self {
            data_dir,
            node_id: Arc::new(Mutex::new(String::new())),
            api_port: Arc::new(Mutex::new(None)),
            onboarding: if needs_onboarding {
                Some(OnboardingState::new())
            } else {
                None
            },
            panel: Panel::Search,
            search: search::SearchPanel::default(),
            settings: settings::SettingsPanel::default(),
            status: StatusBar::default(),
            tray: TrayState::default(),
        }
    }

    fn api(&self) -> ApiHelper {
        ApiHelper {
            data_dir: self.data_dir.clone(),
        }
    }

    fn refresh_status(&mut self) {
        let api = self.api();
        if let Some(body) = api.api_get("/node") {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                self.status.role = v
                    .get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("light")
                    .to_string();
                self.status.peers = v.get("peers").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                self.status.records =
                    v.get("records").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            }
        }
        if let Some(body) = api.api_get("/storage") {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                let size_bytes = v.get("size_bytes").and_then(|v| v.as_u64()).unwrap_or(0);
                self.status.tier2_size_mb = size_bytes as f64 / (1024.0 * 1024.0);
            }
        }
        if let Some(body) = api.api_get("/health") {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                if let Some(id) = v.get("node_id").and_then(|v| v.as_str()) {
                    *self.node_id.lock().unwrap() = id.to_string();
                }
            }
        }
        *self.api_port.lock().unwrap() = {
            let port_path = self.data_dir.join("api.port");
            std::fs::read_to_string(port_path)
                .ok()
                .and_then(|s| s.trim().parse().ok())
        };
    }
}

impl eframe::App for DsearchApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
        self.refresh_status();
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // If onboarding is active, show the wizard instead of the main UI
        if let Some(ref mut onboarding) = self.onboarding {
            onboarding.ui(ui, &self.data_dir);
            if onboarding.is_complete() {
                self.onboarding = None;
            }
            return;
        }

        // Top nav bar
        egui::Panel::top("nav_bar").show_inside(ui, |ui: &mut egui::Ui| {
            ui.horizontal(|ui: &mut egui::Ui| {
                ui.selectable_value(&mut self.panel, Panel::Search, "🔍 Search");
                ui.selectable_value(&mut self.panel, Panel::Settings, "⚙ Settings");
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui: &mut egui::Ui| {
                        let node_id = self.node_id.lock().unwrap();
                        if !node_id.is_empty() {
                            ui.label(
                                egui::RichText::new(format!(
                                    "Node: {}…",
                                    &node_id[..8.min(node_id.len())]
                                ))
                                .small()
                                .color(egui::Color32::GRAY),
                            );
                        }
                    },
                );
            });
        });

        // Status bar at the bottom
        egui::Panel::bottom("status_bar").show_inside(ui, |ui: &mut egui::Ui| {
            ui.horizontal(|ui: &mut egui::Ui| {
                let dot_color = if self.status.peers > 0 {
                    egui::Color32::from_rgb(0x4C, 0xAF, 0x50)
                } else {
                    egui::Color32::from_rgb(0xF4, 0x43, 0x36)
                };
                ui.painter().circle_filled(
                    ui.available_rect_before_wrap().left_center() + egui::vec2(6.0, 0.0),
                    5.0,
                    dot_color,
                );
                ui.add_space(14.0);
                ui.label(egui::RichText::new(format!("Role: {}", self.status.role)).small());
                ui.separator();
                ui.label(egui::RichText::new(format!("Peers: {}", self.status.peers)).small());
                ui.separator();
                ui.label(egui::RichText::new(format!("Records: {}", self.status.records)).small());
                ui.separator();
                ui.label(
                    egui::RichText::new(format!("Tier 2: {:.1} MB", self.status.tier2_size_mb))
                        .small(),
                );
                ui.separator();
                ui.label(
                    egui::RichText::new(format!(
                        "Bandwidth: {:.0} Mbps",
                        self.status.bandwidth_mbps
                    ))
                    .small(),
                );
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui: &mut egui::Ui| {
                        ui.label(
                            egui::RichText::new(format!("📁 {}", self.data_dir.display()))
                                .small()
                                .color(egui::Color32::from_rgb(0x64, 0xB5, 0xF6)),
                        );
                    },
                );
            });
        });

        // Central content — use the ApiHelper to avoid double-&mut-self
        let api = self.api();
        egui::CentralPanel::default().show_inside(ui, |ui: &mut egui::Ui| match self.panel {
            Panel::Search => self.search.ui(ui, &api),
            Panel::Settings => self.settings.ui(ui, &api),
        });

        // Tray icon management
        self.tray.update(frame);
    }
}

/// Launch the egui UI. This blocks until the window is closed.
pub fn run_ui(data_dir: PathBuf) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1024.0, 700.0])
            .with_min_inner_size([640.0, 400.0])
            .with_app_id("dsearch"),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "DSearch",
        options,
        Box::new(|_cc| Ok(Box::new(DsearchApp::new(data_dir)))),
    )
    .map_err(|e| format!("eframe error: {}", e).into())
}
