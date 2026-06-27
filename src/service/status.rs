/// Service status — check registration and running state.
use std::path::Path;

/// Check if the service is registered with the OS.
pub fn is_service_registered(data_dir: &Path) -> bool {
    crate::service::install::is_service_registered(data_dir)
}

/// Check if the service is currently running.
pub fn is_service_running(data_dir: &Path) -> bool {
    crate::service::install::is_service_running(data_dir)
}
