use std::fs;
/// Service management — install/enable/disable/status/uninstall.
///
/// Platform-specific registration:
/// - Linux: systemd user unit
/// - macOS: launchd plist
/// - Windows: Windows Service via sc.exe
use std::path::{Path, PathBuf};

/// Install the service for the current platform.
pub fn service_install(data_dir: &Path, headless: bool) -> Result<String, String> {
    let exe =
        std::env::current_exe().map_err(|e| format!("Cannot determine executable path: {}", e))?;
    let exe_str = exe.to_str().ok_or("Executable path is not valid UTF-8")?;

    if cfg!(target_os = "linux") {
        install_systemd(data_dir, exe_str, headless)
    } else if cfg!(target_os = "macos") {
        install_launchd(data_dir, exe_str, headless)
    } else if cfg!(target_os = "windows") {
        install_windows_service(data_dir, exe_str, headless)
    } else {
        Err("Service installation not supported on this platform".to_string())
    }
}

/// Enable the service (start on boot).
pub fn service_enable(_data_dir: &Path) -> Result<String, String> {
    if cfg!(target_os = "linux") {
        let output = std::process::Command::new("systemctl")
            .args(["--user", "enable", "dsearch"])
            .output()
            .map_err(|e| format!("Failed to run systemctl: {}", e))?;
        if output.status.success() {
            Ok("Service enabled (will start on boot)".to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("systemctl enable failed: {}", stderr))
        }
    } else if cfg!(target_os = "macos") {
        let output = std::process::Command::new("launchctl")
            .args(["load", "-w"])
            .arg(launchd_plist_path().to_str().unwrap_or(""))
            .output()
            .map_err(|e| format!("Failed to run launchctl: {}", e))?;
        if output.status.success() {
            Ok("Service enabled (will start on boot)".to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("launchctl load failed: {}", stderr))
        }
    } else if cfg!(target_os = "windows") {
        let output = std::process::Command::new("sc")
            .args(["config", "DSearch", "start=auto"])
            .output()
            .map_err(|e| format!("Failed to run sc: {}", e))?;
        if output.status.success() {
            Ok("Service enabled (will start on boot)".to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("sc config failed: {}", stderr))
        }
    } else {
        Err("Service enable not supported on this platform".to_string())
    }
}

/// Disable the service (do not start on boot).
pub fn service_disable(_data_dir: &Path) -> Result<String, String> {
    if cfg!(target_os = "linux") {
        let output = std::process::Command::new("systemctl")
            .args(["--user", "disable", "dsearch"])
            .output()
            .map_err(|e| format!("Failed to run systemctl: {}", e))?;
        if output.status.success() {
            Ok("Service disabled (will not start on boot)".to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("systemctl disable failed: {}", stderr))
        }
    } else if cfg!(target_os = "macos") {
        let output = std::process::Command::new("launchctl")
            .args(["unload", "-w"])
            .arg(launchd_plist_path().to_str().unwrap_or(""))
            .output()
            .map_err(|e| format!("Failed to run launchctl: {}", e))?;
        if output.status.success() {
            Ok("Service disabled (will not start on boot)".to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("launchctl unload failed: {}", stderr))
        }
    } else if cfg!(target_os = "windows") {
        let output = std::process::Command::new("sc")
            .args(["config", "DSearch", "start=demand"])
            .output()
            .map_err(|e| format!("Failed to run sc: {}", e))?;
        if output.status.success() {
            Ok("Service disabled (will not start on boot)".to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("sc config failed: {}", stderr))
        }
    } else {
        Err("Service disable not supported on this platform".to_string())
    }
}

/// Get service status (registered + running).
pub fn service_status(data_dir: &Path) -> Result<String, String> {
    let registered = is_service_registered(data_dir);
    let running = is_service_running(data_dir);

    let reg_str = if registered {
        "registered"
    } else {
        "not registered"
    };
    let run_str = if running { "running" } else { "stopped" };

    Ok(format!("Service: {} | {}", reg_str, run_str))
}

/// Uninstall the service.
pub fn service_uninstall(_data_dir: &Path) -> Result<String, String> {
    if cfg!(target_os = "linux") {
        // Stop first
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "stop", "dsearch"])
            .output();
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "disable", "dsearch"])
            .output();

        let unit_path = systemd_unit_path();
        if unit_path.exists() {
            fs::remove_file(&unit_path)
                .map_err(|e| format!("Failed to remove unit file: {}", e))?;
        }
        let _ = std::process::Command::new("systemctl")
            .args(["--user", "daemon-reload"])
            .output();
        Ok("Service uninstalled".to_string())
    } else if cfg!(target_os = "macos") {
        let _ = std::process::Command::new("launchctl")
            .args(["unload"])
            .arg(launchd_plist_path().to_str().unwrap_or(""))
            .output();
        let plist_path = launchd_plist_path();
        if plist_path.exists() {
            fs::remove_file(&plist_path).map_err(|e| format!("Failed to remove plist: {}", e))?;
        }
        Ok("Service uninstalled".to_string())
    } else if cfg!(target_os = "windows") {
        let _ = std::process::Command::new("net")
            .args(["stop", "DSearch"])
            .output();
        let output = std::process::Command::new("sc")
            .args(["delete", "DSearch"])
            .output()
            .map_err(|e| format!("Failed to run sc: {}", e))?;
        if output.status.success() {
            Ok("Service uninstalled".to_string())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("sc delete failed: {}", stderr))
        }
    } else {
        Err("Service uninstall not supported on this platform".to_string())
    }
}

// ---- Platform-specific install implementations ----

fn install_systemd(data_dir: &Path, exe_path: &str, headless: bool) -> Result<String, String> {
    let unit_dir = systemd_unit_dir();
    fs::create_dir_all(&unit_dir)
        .map_err(|e| format!("Failed to create systemd unit dir: {}", e))?;

    let data_dir_str = data_dir
        .to_str()
        .ok_or("Data dir path is not valid UTF-8")?;
    let node_cmd = if headless {
        format!(
            "{} node start --headless --data-dir {}",
            exe_path, data_dir_str
        )
    } else {
        format!("{} node start --data-dir {}", exe_path, data_dir_str)
    };

    let unit_content = format!(
        "[Unit]\n\
         Description=DSearch Node\n\
         After=network.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={}\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        node_cmd
    );

    let unit_path = systemd_unit_path();
    fs::write(&unit_path, &unit_content)
        .map_err(|e| format!("Failed to write unit file: {}", e))?;

    // Reload systemd
    let _ = std::process::Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .output();

    // Write marker file so we can detect registration
    write_service_marker(data_dir, "systemd")?;

    Ok(format!(
        "Service installed (systemd user unit at {})",
        unit_path.display()
    ))
}

fn install_launchd(data_dir: &Path, exe_path: &str, headless: bool) -> Result<String, String> {
    let plist_dir = launchd_plist_dir();
    fs::create_dir_all(&plist_dir)
        .map_err(|e| format!("Failed to create launchd plist dir: {}", e))?;

    let data_dir_str = data_dir
        .to_str()
        .ok_or("Data dir path is not valid UTF-8")?;
    let node_cmd = if headless {
        vec![
            exe_path.to_string(),
            "node".to_string(),
            "start".to_string(),
            "--headless".to_string(),
            "--data-dir".to_string(),
            data_dir_str.to_string(),
        ]
    } else {
        vec![
            exe_path.to_string(),
            "node".to_string(),
            "start".to_string(),
            "--data-dir".to_string(),
            data_dir_str.to_string(),
        ]
    };

    let args_xml: String = node_cmd
        .iter()
        .map(|a| format!("<string>{}</string>", a))
        .collect::<Vec<_>>()
        .join("\n        ");

    let plist_content = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
           <key>Label</key>\n\
           <string>com.dsearch.node</string>\n\
           <key>ProgramArguments</key>\n\
           <array>\n\
         {args_xml}\n\
           </array>\n\
           <key>RunAtLoad</key>\n\
           <true/>\n\
           <key>KeepAlive</key>\n\
           <true/>\n\
         </dict>\n\
         </plist>\n"
    );

    let plist_path = launchd_plist_path();
    fs::write(&plist_path, &plist_content).map_err(|e| format!("Failed to write plist: {}", e))?;

    // Load the plist
    let _ = std::process::Command::new("launchctl")
        .args(["load", "-w"])
        .arg(plist_path.to_str().unwrap_or(""))
        .output();

    write_service_marker(data_dir, "launchd")?;

    Ok(format!(
        "Service installed (launchd plist at {})",
        plist_path.display()
    ))
}

fn install_windows_service(
    data_dir: &Path,
    exe_path: &str,
    headless: bool,
) -> Result<String, String> {
    let data_dir_str = data_dir
        .to_str()
        .ok_or("Data dir path is not valid UTF-8")?;

    // Create the service using sc.exe
    let bin_path = if headless {
        format!(
            "{} node start --headless --data-dir {}",
            exe_path, data_dir_str
        )
    } else {
        format!("{} node start --data-dir {}", exe_path, data_dir_str)
    };

    let output = std::process::Command::new("sc")
        .args([
            "create",
            "DSearch",
            "binPath=",
            &bin_path,
            "start=demand",
            "DisplayName=DSearch Node",
        ])
        .output()
        .map_err(|e| format!("Failed to run sc: {}", e))?;

    if output.status.success() {
        write_service_marker(data_dir, "windows_service")?;
        Ok("Service installed (Windows Service: DSearch)".to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Check if service already exists
        if stderr.contains("already exists") || stderr.contains("ERROR_SERVICE_EXISTS") {
            write_service_marker(data_dir, "windows_service")?;
            Ok("Service already exists (Windows Service: DSearch)".to_string())
        } else {
            Err(format!("sc create failed: {}", stderr))
        }
    }
}

// ---- Helper functions ----

fn systemd_unit_dir() -> PathBuf {
    dirs_next::home_dir()
        .unwrap_or_default()
        .join(".config")
        .join("systemd")
        .join("user")
}

fn systemd_unit_path() -> PathBuf {
    systemd_unit_dir().join("dsearch.service")
}

fn launchd_plist_dir() -> PathBuf {
    dirs_next::home_dir()
        .unwrap_or_default()
        .join("Library")
        .join("LaunchAgents")
}

fn launchd_plist_path() -> PathBuf {
    launchd_plist_dir().join("com.dsearch.node.plist")
}

fn write_service_marker(data_dir: &Path, service_type: &str) -> Result<(), String> {
    let marker_path = data_dir.join("service.registered");
    fs::write(&marker_path, service_type)
        .map_err(|e| format!("Failed to write service marker: {}", e))
}

/// Check if the service is registered (marker file exists).
pub fn is_service_registered(data_dir: &Path) -> bool {
    data_dir.join("service.registered").exists()
}

/// Check if the service is currently running (API port file exists and API responds).
pub fn is_service_running(data_dir: &Path) -> bool {
    let port_path = data_dir.join("api.port");
    if !port_path.exists() {
        return false;
    }
    if let Ok(port_str) = fs::read_to_string(&port_path) {
        if let Ok(port) = port_str.trim().parse::<u16>() {
            return crate::cli::api_client::api_get(port, "/health").is_ok();
        }
    }
    false
}
