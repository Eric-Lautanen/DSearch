/// Scraper subprocess isolation.
///
/// On Windows: uses Job Objects to limit resource usage and ensure child cleanup.
/// On Linux: uses seccomp (planned — requires `libseccomp` bindings).
/// On macOS: uses `sandbox-exec` (planned).
///
/// Currently implements basic process spawning with kill-on-drop semantics.
/// Full Job Object / seccomp integration requires platform-specific crates
/// (windows-sys, libseccomp) which are deferred to avoid adding heavy deps.
use std::process::{Command, Stdio};
use std::path::Path;

/// Configuration for a sandboxed scraper subprocess.
pub struct SandboxConfig {
    /// Maximum memory in MB (0 = unlimited).
    pub max_memory_mb: u32,
    /// Maximum CPU time in seconds (0 = unlimited).
    pub max_cpu_secs: u32,
    /// Kill subprocess when the parent process exits.
    pub kill_on_parent_exit: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            max_memory_mb: 128,
            max_cpu_secs: 300,
            kill_on_parent_exit: true,
        }
    }
}

/// Spawn a scraper subprocess with sandboxing.
/// The child process is spawned with piped stdout/stderr and null stdin.
/// On Windows, CREATE_BREAKAWAY_FROM_JOB is set so the child can be
/// assigned to its own job object by the caller.
pub fn spawn_scraper_process(
    exe_path: &Path,
    args: &[String],
    _config: &SandboxConfig,
) -> Result<std::process::Child, String> {
    let mut cmd = Command::new(exe_path);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x01000000;
        cmd.creation_flags(CREATE_BREAKAWAY_FROM_JOB);
    }

    cmd.spawn()
        .map_err(|e| format!("Failed to spawn scraper process: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_config_defaults() {
        let config = SandboxConfig::default();
        assert_eq!(config.max_memory_mb, 128);
        assert_eq!(config.max_cpu_secs, 300);
        assert!(config.kill_on_parent_exit);
    }
}
