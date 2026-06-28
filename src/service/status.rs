use std::path::Path;

pub fn is_service_registered(data_dir: &Path) -> bool {
    crate::service::install::is_service_registered(data_dir)
}
