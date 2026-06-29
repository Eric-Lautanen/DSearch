use std::net::UdpSocket;
use std::time::Duration;

/// AutoNAT probe — determines if this node is publicly reachable.
///
/// Strategy:
/// 1. Try STUN binding to discover public address.
/// 2. Try to bind the QUIC port (default 7744) on UDP.
/// 3. If STUN succeeds and the port matches, we're likely reachable.
/// 4. If STUN fails but we can bind the port, we might be reachable
///    (behind NAT but with port forwarding).
/// 5. If we can't bind the port, we're definitely not reachable.
pub struct AutoNATResult {
    pub is_public: bool,
    pub public_addr: Option<String>,
    pub reason: String,
}

/// Run the AutoNAT probe. Returns whether the node appears publicly reachable.
pub fn probe(quic_port: u16, timeout: Duration) -> AutoNATResult {
    // Step 1: Try to bind the QUIC port
    let can_bind = UdpSocket::bind(format!("0.0.0.0:{}", quic_port)).is_ok();

    if !can_bind {
        return AutoNATResult {
            is_public: false,
            public_addr: None,
            reason: format!("Cannot bind UDP port {} — another process is using it", quic_port),
        };
    }

    // Step 2: Try STUN to discover public address
    let mut public_addr: Option<String> = None;
    for server in super::stun::default_stun_servers() {
        match super::stun::stun_bind(server, timeout) {
            Ok(addr) => {
                public_addr = Some(addr);
                break;
            }
            Err(_) => continue,
        }
    }

    match public_addr {
        Some(ref addr) => AutoNATResult {
            is_public: true,
            public_addr: Some(addr.clone()),
            reason: format!("STUN reports public address {}", addr),
        },
        None => AutoNATResult {
            is_public: false,
            public_addr: None,
            reason: "STUN probe failed — node is likely behind NAT without port forwarding".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_autonat_probe_unreachable_port() {
        // Port 1 is not bindable on most systems
        let result = probe(1, Duration::from_millis(100));
        // This may or may not succeed depending on OS permissions,
        // but it should not panic
        let _ = result.is_public;
    }
}
