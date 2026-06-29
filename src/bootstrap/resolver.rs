use crate::bootstrap::defaults::{default_bootstrap_peers, BootstrapPeer};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapToml {
    #[serde(default = "default_true")]
    pub use_defaults: bool,
    #[serde(default)]
    pub peers: Vec<BootstrapPeer>,
}

fn default_true() -> bool {
    true
}

/// Resolve bootstrap peers: bootstrap.toml → DNS SRV → compiled defaults.
pub fn resolve_bootstrap_peers(data_dir: &std::path::Path) -> Vec<BootstrapPeer> {
    let mut peers = Vec::new();

    // 1. Read bootstrap.toml from data_dir
    let toml_path = data_dir.join("bootstrap.toml");
    if toml_path.exists() {
        if let Ok(contents) = std::fs::read_to_string(&toml_path) {
            if let Ok(config) = toml::from_str::<BootstrapToml>(&contents) {
                peers.extend(config.peers);
                if !config.use_defaults {
                    return peers;
                }
            }
        }
    }

    // 2. DNS SRV lookup for _dsearch._udp.dsearch.network
    let srv_peers = resolve_dns_srv("_dsearch._udp.dsearch.network");
    for p in &srv_peers {
        if !peers.iter().any(|ep| ep.id == p.id) {
            peers.push(p.clone());
        }
    }

    // 3. Compiled-in defaults
    let defaults = default_bootstrap_peers();
    for p in &defaults {
        if !peers.iter().any(|ep| ep.id == p.id) {
            peers.push(p.clone());
        }
    }

    peers
}

/// Resolve a DNS SRV record by querying DNS servers directly over UDP.
/// Sends a raw DNS query and parses the SRV response.
fn resolve_dns_srv(srv_name: &str) -> Vec<BootstrapPeer> {
    let mut peers = Vec::new();

    let dns_servers = ["8.8.8.8:53", "1.1.1.1:53", "9.9.9.9:53"];

    for dns_server in &dns_servers {
        match query_dns_srv(srv_name, dns_server) {
            Ok(records) => {
                peers.extend(records);
                break;
            }
            Err(_) => continue,
        }
    }

    peers
}

/// Send a DNS SRV query to a specific DNS server and parse the response.
fn query_dns_srv(name: &str, server: &str) -> Result<Vec<BootstrapPeer>, String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("bind: {}", e))?;
    socket
        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .map_err(|e| format!("set timeout: {}", e))?;

    let query = build_dns_query(name);
    socket
        .send_to(&query, server)
        .map_err(|e| format!("send: {}", e))?;

    let mut response = [0u8; 4096];
    let n = socket
        .recv(&mut response)
        .map_err(|e| format!("recv: {}", e))?;

    if n < 12 {
        return Err("DNS response too short".to_string());
    }

    parse_dns_srv_response(&response[..n])
}

/// Build a DNS query packet for SRV records.
fn build_dns_query(name: &str) -> Vec<u8> {
    let mut packet = Vec::with_capacity(64);

    // Header: ID=0x1234, flags=0x0100 (RD), QD=1
    packet.extend_from_slice(&[0x12, 0x34]); // Transaction ID
    packet.extend_from_slice(&[0x01, 0x00]); // Flags: standard query, RD=1
    packet.extend_from_slice(&[0x00, 0x01]); // QDCOUNT: 1
    packet.extend_from_slice(&[0x00, 0x00]); // ANCOUNT: 0
    packet.extend_from_slice(&[0x00, 0x00]); // NSCOUNT: 0
    packet.extend_from_slice(&[0x00, 0x00]); // ARCOUNT: 0

    // Question: encode name as DNS labels
    for label in name.split('.') {
        let label_bytes = label.as_bytes();
        packet.push(label_bytes.len() as u8);
        packet.extend_from_slice(label_bytes);
    }
    packet.push(0x00); // Root label

    // QTYPE: SRV = 33 (0x0021), QCLASS: IN = 1
    packet.extend_from_slice(&[0x00, 0x21]);
    packet.extend_from_slice(&[0x00, 0x01]);

    packet
}

/// Parse DNS response for SRV records.
fn parse_dns_srv_response(data: &[u8]) -> Result<Vec<BootstrapPeer>, String> {
    if data.len() < 12 {
        return Err("response too short".to_string());
    }

    let qdcount = u16::from_be_bytes([data[4], data[5]]) as usize;
    let ancount = u16::from_be_bytes([data[6], data[7]]) as usize;

    // Skip past the question section
    let mut offset = 12;
    for _ in 0..qdcount {
        offset = skip_dns_name(data, offset)?;
        offset += 4; // Skip QTYPE and QCLASS
    }

    let mut peers = Vec::new();

    for _ in 0..ancount {
        offset = skip_dns_name(data, offset)?;
        if offset + 10 > data.len() {
            break;
        }

        let rtype = u16::from_be_bytes([data[offset], data[offset + 1]]);
        let rdlength = u16::from_be_bytes([data[offset + 8], data[offset + 9]]) as usize;
        offset += 10;

        if offset + rdlength > data.len() {
            break;
        }

        if rtype == 33 && rdlength >= 6 {
            // SRV: priority(2) + weight(2) + port(2) + target(variable)
            let port = u16::from_be_bytes([data[offset + 4], data[offset + 5]]);

            let target = match read_dns_name(data, offset + 6, offset + rdlength) {
                Ok(name) => name,
                Err(_) => {
                    offset += rdlength;
                    continue;
                }
            };

            let addr = format!("{}:{}", target, port);
            let id = format!("srv:{}", addr);
            peers.push(BootstrapPeer {
                id,
                addr,
                note: "DNS SRV".to_string(),
            });
        }

        offset += rdlength;
    }

    Ok(peers)
}

/// Skip a DNS name in a packet, handling compression pointers.
fn skip_dns_name(data: &[u8], mut offset: usize) -> Result<usize, String> {
    loop {
        if offset >= data.len() {
            return Err("name extends past packet".to_string());
        }
        let len = data[offset];
        if len == 0 {
            return Ok(offset + 1);
        }
        if (len & 0xC0) == 0xC0 {
            return Ok(offset + 2);
        }
        offset += 1 + len as usize;
    }
}
/// Read a DNS name from a packet, following compression pointers.
/// `end` is the nominal end of the RDATA, but compression pointers
/// may reference data anywhere in the full packet, so we use
/// `data.len()` as the actual boundary for pointer targets.
fn read_dns_name(data: &[u8], mut offset: usize, _end: usize) -> Result<String, String> {
    let mut name = String::new();
    let mut jumped = false;
    let mut visited = 0usize;
    let data_len = data.len();

    loop {
        if offset >= data_len {
            break;
        }
        let len = data[offset];
        if len == 0 {
            break;
        }
        if (len & 0xC0) == 0xC0 {
            if offset + 1 >= data_len {
                break;
            }
            let ptr = (((len & 0x3F) as usize) << 8) | data[offset + 1] as usize;
            if ptr >= data_len || visited > 20 {
                break;
            }
            offset = ptr;
            jumped = true;
            visited += 1;
            continue;
        }
        if !name.is_empty() {
            name.push('.');
        }
        let label_end = offset + 1 + len as usize;
        if label_end > data_len {
            break;
        }
        let label = String::from_utf8_lossy(&data[offset + 1..label_end]);
        name.push_str(&label);
        offset = label_end;

        if jumped {
            visited += 1;
            if visited > 20 {
                break;
            }
        }
    }

    Ok(name)
}

/// Write a bootstrap.toml with a specific peer entry.
pub fn write_bootstrap_peer(
    data_dir: &std::path::Path,
    id: &str,
    addr: &str,
    note: &str,
) -> Result<(), std::io::Error> {
    let toml_path = data_dir.join("bootstrap.toml");
    let mut config = if toml_path.exists() {
        let contents = std::fs::read_to_string(&toml_path)?;
        toml::from_str::<BootstrapToml>(&contents).unwrap_or_else(|_| BootstrapToml {
            use_defaults: true,
            peers: Vec::new(),
        })
    } else {
        BootstrapToml {
            use_defaults: true,
            peers: Vec::new(),
        }
    };

    config.peers.push(BootstrapPeer {
        id: id.to_string(),
        addr: addr.to_string(),
        note: note.to_string(),
    });

    let toml_str = toml::to_string_pretty(&config).unwrap_or_default();
    std::fs::write(&toml_path, toml_str)
}

/// Remove a bootstrap peer by id from bootstrap.toml.
pub fn remove_bootstrap_peer(data_dir: &std::path::Path, id: &str) -> Result<bool, std::io::Error> {
    let toml_path = data_dir.join("bootstrap.toml");
    if !toml_path.exists() {
        return Ok(false);
    }

    let contents = std::fs::read_to_string(&toml_path)?;
    let mut config = toml::from_str::<BootstrapToml>(&contents).unwrap_or_else(|_| BootstrapToml {
        use_defaults: true,
        peers: Vec::new(),
    });

    let before = config.peers.len();
    config.peers.retain(|p| p.id != id);
    if config.peers.len() == before {
        return Ok(false);
    }

    let toml_str = toml::to_string_pretty(&config).unwrap_or_default();
    std::fs::write(&toml_path, toml_str)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_dns_query() {
        let query = build_dns_query("_dsearch._udp.dsearch.network");
        assert_eq!(query[0], 0x12);
        assert_eq!(query[1], 0x34);
        assert!(query.len() > 12);
    }

    #[test]
    fn test_skip_dns_name() {
        let data = b"\x03www\x07example\x03com\x00\x00\x01\x00\x01";
        let offset = skip_dns_name(data, 0).unwrap();
        assert_eq!(offset, 17);
    }

    #[test]
    fn test_read_dns_name() {
        let data = b"\x03www\x07example\x03com\x00\x00\x01\x00\x01";
        let name = read_dns_name(data, 0, data.len()).unwrap();
        assert_eq!(name, "www.example.com");
    }

    #[test]
    fn test_read_dns_name_compression() {
        // Build a DNS packet where the name at offset 0 is "www" + compression pointer
        // to offset 6 where "example.com\0" starts
        let mut data = Vec::new();
        // Offset 0: label "www" (3 bytes + 1 length)
        data.push(0x03);
        data.extend_from_slice(b"www");
        // Offset 4: compression pointer to offset 6
        data.push(0xC0);
        data.push(0x06);
        // Offset 6: "example.com\0"
        data.push(0x07);
        data.extend_from_slice(b"example");
        data.push(0x03);
        data.extend_from_slice(b"com");
        data.push(0x00);
        // Padding
        data.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);

        let name = read_dns_name(&data, 0, data.len()).unwrap();
        assert_eq!(name, "www.example.com");
    }

    #[test]
    fn test_query_dns_srv_invalid_server() {
        let result = query_dns_srv("_dsearch._udp.dsearch.network", "0.0.0.0:1");
        assert!(result.is_err());
    }
}
