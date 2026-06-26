/// Config migration framework.
///
/// `config_version` in the `[meta]` table of config.toml tracks the
/// config file's own schema version, independent of the wire protocol
/// version. On startup, if `config_version < CURRENT_CONFIG_VERSION`,
/// migration functions run in order. If `config_version` is from a
/// future version, the node refuses to open (prevents silent data
/// corruption on downgrade).

use super::CURRENT_CONFIG_VERSION;

/// Run all pending config migrations.
/// Returns Ok(()) if all succeed, or Err with a description of the failure.
pub fn run_migrations(current_version: u32) -> Result<(), String> {
    if current_version > CURRENT_CONFIG_VERSION {
        return Err(format!(
            "config_version {} is from a future version (current: {}). \
             Downgrading is not supported.",
            current_version, CURRENT_CONFIG_VERSION
        ));
    }

    // No migrations yet — version 1 is the initial schema.
    // Future migrations go here:
    // if current_version < 2 { migrate_v1_to_v2()?; }
    // if current_version < 3 { migrate_v2_to_v3()?; }

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
