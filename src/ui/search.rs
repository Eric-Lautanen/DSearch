use super::ApiHelper;
use eframe::egui;

#[derive(Default)]
pub struct SearchPanel {
    query: String,
    results: Vec<SearchResult>,
    searching: bool,
    last_query: String,
    error: String,
}

#[derive(Debug, Clone)]
struct SearchResult {
    id: String,
    schema: String,
    source_url: String,
    body_snippet: String,
    title: String,
}

impl SearchPanel {
    pub fn ui(&mut self, ui: &mut egui::Ui, api: &ApiHelper) {
        ui.horizontal(|ui: &mut egui::Ui| {
            let search_icon = egui::RichText::new("🔍").size(20.0);
            ui.label(search_icon);
            let response = ui.add(
                egui::TextEdit::singleline(&mut self.query)
                    .hint_text("Search the network…")
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Body),
            );
            let enter_pressed = ui.input(|i: &egui::InputState| i.key_pressed(egui::Key::Enter))
                && !self.query.is_empty();
            if ui.button("Search").clicked() || enter_pressed {
                self.run_search(api);
            }
            let _ = response;
        });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(8.0);

        if self.searching {
            ui.vertical_centered(|ui: &mut egui::Ui| {
                ui.add_space(40.0);
                ui.spinner();
                ui.add_space(8.0);
                ui.label("Searching…");
            });
        } else if !self.error.is_empty() {
            ui.vertical_centered(|ui: &mut egui::Ui| {
                ui.add_space(60.0);
                ui.label(egui::RichText::new("⚠").size(48.0));
                ui.add_space(8.0);
                ui.heading("Search failed");
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(&self.error)
                        .color(egui::Color32::from_rgb(0xF4, 0x43, 0x36)),
                );
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "Make sure the node is running and the API is reachable.",
                    )
                    .color(egui::Color32::GRAY)
                    .small(),
                );
            });
        } else if self.results.is_empty() && self.last_query.is_empty() {
            ui.vertical_centered(|ui: &mut egui::Ui| {
                ui.add_space(60.0);
                ui.label(egui::RichText::new("🔍").size(48.0));
                ui.add_space(8.0);
                ui.heading("Start searching");
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "Your node is connected. Try searching for something,\n\
                     or add a scraper in Settings to contribute content to the network.",
                    )
                    .color(egui::Color32::GRAY),
                );
            });
        } else if self.results.is_empty() && !self.last_query.is_empty() {
            ui.vertical_centered(|ui: &mut egui::Ui| {
                ui.add_space(60.0);
                ui.label(egui::RichText::new("😕").size(48.0));
                ui.add_space(8.0);
                ui.heading("No results found");
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "No content matched your query. Try different keywords,\n\
                     or check that scrapers are running in Settings.",
                    )
                    .color(egui::Color32::GRAY),
                );
            });
        } else {
            egui::ScrollArea::vertical().show(ui, |ui: &mut egui::Ui| {
                for result in &self.results {
                    ui.group(|ui: &mut egui::Ui| {
                        ui.horizontal(|ui: &mut egui::Ui| {
                            ui.label(
                                egui::RichText::new(&result.schema)
                                    .small()
                                    .color(egui::Color32::from_rgb(0x64, 0xB5, 0xF6)),
                            );
                            ui.label(egui::RichText::new("•").small().color(egui::Color32::GRAY));
                            ui.label(
                                egui::RichText::new(&result.source_url)
                                    .small()
                                    .color(egui::Color32::GRAY),
                            );
                        });
                        ui.add_space(2.0);
                        ui.label(egui::RichText::new(&result.title).strong());
                        ui.add_space(2.0);
                        ui.label(
                            egui::RichText::new(&result.body_snippet)
                                .small()
                                .color(egui::Color32::LIGHT_GRAY),
                        );
                        ui.add_space(2.0);
                        ui.horizontal(|ui: &mut egui::Ui| {
                            ui.label(
                                egui::RichText::new(format!(
                                    "ID: {}…",
                                    &result.id[..12.min(result.id.len())]
                                ))
                                .small()
                                .color(egui::Color32::DARK_GRAY),
                            );
                            if ui.small_button("📋").on_hover_text("Copy ID").clicked() {
                                ui.ctx().copy_text(result.id.clone());
                            }
                        });
                    });
                    ui.add_space(4.0);
                }
            });
        }
    }

    fn run_search(&mut self, api: &ApiHelper) {
        if self.query.is_empty() {
            return;
        }
        self.searching = true;
        self.error.clear();
        self.last_query = self.query.clone();

        let query = self.query.clone();
        let path = format!("/search?q={}&limit=20", query.replace(' ', "+"));

        match api.api_get_result(&path) {
            Ok(body) => {
                self.results.clear();
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body) {
                    if let Some(results) = v.get("results").and_then(|r| r.as_array()) {
                        for r in results {
                            let id = r
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let schema = r
                                .get("schema")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let source_url = r
                                .get("source_url")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let body_text = r.get("body").and_then(|v| v.as_str()).unwrap_or("");
                            let snippet: String = body_text.chars().take(200).collect();
                            let title = r
                                .get("tags")
                                .and_then(|t| t.as_array())
                                .and_then(|a| a.first())
                                .and_then(|v| v.as_str())
                                .unwrap_or(&id[..16.min(id.len())])
                                .to_string();

                            self.results.push(SearchResult {
                                id,
                                schema,
                                source_url,
                                body_snippet: snippet,
                                title,
                            });
                        }
                    }
                }
            }
            Err(e) => {
                self.results.clear();
                self.error = e;
            }
        }

        self.searching = false;
    }
}
