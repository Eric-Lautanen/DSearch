use super::CURRENT_CONFIG_VERSION;

pub fn run_migrations(current_version: u32) -> Result<(), String> {
    if current_version > CURRENT_CONFIG_VERSION {
        return Err(format!(
            "config_version {} is from a future version (current: {}). \
             Downgrading is not supported.",
            current_version, CURRENT_CONFIG_VERSION
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_migrations_needed_for_current() {
        assert!(run_migrations(CURRENT_CONFIG_VERSION).is_ok());
    }

    #[test]
    fn future_version_rejected() {
        assert!(run_migrations(CURRENT_CONFIG_VERSION + 1).is_err());
    }

    #[test]
    fn version_zero_ok() {
        assert!(run_migrations(0).is_ok());
    }
}
