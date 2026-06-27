use eframe;
use tray_icon::{TrayIconBuilder, TrayIcon, menu::{Menu, MenuItem, PredefinedMenuItem}, Icon};
use tray_icon::menu::MenuEvent;

/// State for the system tray icon.
#[derive(Default)]
pub struct TrayState {
    tray_icon: Option<TrayIcon>,
    paused: bool,
    initialized: bool,
}

impl TrayState {
    /// Process tray events each frame.
    pub fn update(&mut self, _frame: &mut eframe::Frame) {
        if !self.initialized {
            self.initialized = true;
            self.create_tray();
        }

        // Process menu events
        if let Ok(event) = MenuEvent::receiver().try_recv() {
            match event.id().as_ref() {
                "open" => {}
                "pause" => {
                    self.paused = !self.paused;
                    if let Some(ref tray) = self.tray_icon {
                        let menu = build_tray_menu(self.paused);
                        tray.set_menu(Some(Box::new(menu)));
                    }
                }
                "quit" => {
                    std::process::exit(0);
                }
                _ => {}
            }
        }
    }

    fn create_tray(&mut self) {
        let menu = build_tray_menu(self.paused);
        let icon = load_tray_icon();

        let mut builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("DSearch — Decentralized Search");

        if let Some(icon) = icon {
            builder = builder.with_icon(icon);
        }

        match builder.build() {
            Ok(tray) => {
                self.tray_icon = Some(tray);
            }
            Err(e) => {
                tracing::warn!("Failed to create tray icon: {}", e);
            }
        }
    }

    /// Update the status dot color on the tray icon tooltip.
    pub fn set_status(&mut self, connected: bool) {
        if let Some(ref tray) = self.tray_icon {
            let status = if self.paused {
                "Paused"
            } else if connected {
                "Connected"
            } else {
                "Disconnected"
            };
            tray.set_tooltip(Some(format!("DSearch — {}", status))).ok();
        }
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }
}

fn build_tray_menu(paused: bool) -> Menu {
    let menu = Menu::new();
    let open = MenuItem::with_id("open", "Open DSearch", true, None);
    let pause_text = if paused { "Resume Node" } else { "Pause Node" };
    let pause = MenuItem::with_id("pause", pause_text, true, None);
    let quit = MenuItem::with_id("quit", "Quit", true, None);

    let separator = PredefinedMenuItem::separator();

    menu.append(&open).ok();
    menu.append(&separator).ok();
    menu.append(&pause).ok();
    menu.append(&PredefinedMenuItem::separator()).ok();
    menu.append(&quit).ok();

    menu
}

/// Load the tray icon from the assets directory.
/// Tries PNG files first, falls back to a generated green dot.
fn load_tray_icon() -> Option<Icon> {
    let png_paths = [
        "assets/linux/icon-32.png",
        "assets/linux/icon-16.png",
    ];

    for path in &png_paths {
        let path = std::path::Path::new(path);
        if path.exists() {
            if let Ok(img) = image::open(path) {
                let img = img.to_rgba8();
                let width = img.width();
                let height = img.height();
                let rgba = img.into_raw();
                return Icon::from_rgba(rgba, width, height).ok();
            }
        }
    }

    // Fallback: generate a simple 16x16 green dot icon
    let size = 16u32;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let cx = x as f32 - 7.5;
            let cy = y as f32 - 7.5;
            let dist = (cx * cx + cy * cy).sqrt();
            if dist < 5.0 {
                rgba.extend_from_slice(&[0x4C, 0xAF, 0x50, 0xFF]);
            } else {
                rgba.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
            }
        }
    }
    Icon::from_rgba(rgba, size, size).ok()
}
